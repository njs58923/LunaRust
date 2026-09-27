//! One bounded opportunity to reclaim a mesh builder's temporary V8 allocations.
//! The host must call collection only on the owning worker, after publishing its
//! output, with no queued commands or animation frames and enough timer slack.
use crate::Engine;
use deno_core::v8;
use std::{
    sync::{Mutex, TryLockError},
    time::{Duration, Instant},
};

const QUIET_PERIOD: Duration = Duration::from_millis(500);
const RETRY_DELAY: Duration = Duration::from_millis(50);
const OPPORTUNITY_WINDOW: Duration = Duration::from_secs(5);
const COOLDOWN: Duration = Duration::from_secs(30);
const MIN_USED_HEAP: usize = 3 * 1024 * 1024;

#[derive(Clone, Copy)]
struct Opportunity {
    deadline: Instant,
    expires: Instant,
}

#[derive(Default)]
pub struct IdleHeapReclaimer {
    pending: Option<Opportunity>,
    next_eligible: Option<Instant>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ReclaimOutcome {
    NotDue,
    Deferred,
    Expired,
    SkippedSmallHeap,
    Collected {
        before_used: usize,
        after_used: usize,
        duration: Duration,
    },
}

impl IdleHeapReclaimer {
    /// Only an actual mesh upload arms maintenance; ordinary timers never do.
    pub fn on_mesh_upload(&mut self, now: Instant) {
        if self.pending.is_some() {
            self.note_activity(now);
        } else if self.next_eligible.is_none_or(|next| now >= next) {
            self.pending = Some(Opportunity {
                deadline: now + QUIET_PERIOD,
                expires: now + OPPORTUNITY_WINDOW,
            });
        }
    }

    /// Restart the quiet period without extending the original attempt window.
    pub fn note_activity(&mut self, now: Instant) {
        if let Some(pending) = &mut self.pending {
            if now >= pending.expires {
                self.finish_attempt(now);
            } else {
                pending.deadline = (now + QUIET_PERIOD).min(pending.expires);
            }
        }
    }

    /// The host can block indefinitely when this is None. No periodic idle poll
    /// is needed once an opportunity was collected, skipped, cancelled or expired.
    pub fn next_deadline(&self) -> Option<Instant> {
        self.pending.map(|pending| pending.deadline)
    }

    /// Queue activity, an imminent timer or another worker's GC may defer a due
    /// opportunity. Retries stop after five seconds, even if the worker stays busy.
    pub fn defer(&mut self, now: Instant) {
        if let Some(pending) = &mut self.pending {
            if now >= pending.expires {
                self.finish_attempt(now);
            } else if now >= pending.deadline {
                pending.deadline = (now + RETRY_DELAY).min(pending.expires);
            }
        }
    }

    /// Animation is not an idle mesh workload. Cancelling does not count as a
    /// collection attempt; a later static upload may open a new opportunity.
    pub fn cancel(&mut self) {
        self.pending = None;
    }

    /// Attempt collection on the owning isolate thread. This never executes a
    /// script or pumps V8 tasks: FinalizationRegistry cleanup remains queued by
    /// V8, and live callbacks/resources remain reachable. Native GC is not
    /// interruptible; callers must leave their JS watchdog disarmed here.
    pub fn collect_if_due(&mut self, engine: &mut Engine, now: Instant) -> ReclaimOutcome {
        if let Some(outcome) = self.check_due(now) {
            return outcome;
        }
        // Prevent many newly mounted isolates from collecting concurrently. A
        // contended worker yields to its command loop instead of waiting here.
        static COLLECTION: Mutex<()> = Mutex::new(());
        let _permit = match COLLECTION.try_lock() {
            Ok(guard) => guard,
            Err(TryLockError::Poisoned(error)) => error.into_inner(),
            Err(TryLockError::WouldBlock) => {
                self.defer(now);
                return ReclaimOutcome::Deferred;
            }
        };
        let isolate = engine.rt.v8_isolate();
        let mut heap = v8::HeapStatistics::default();
        isolate.get_heap_statistics(&mut heap);
        let before_used = heap.used_heap_size();
        // Consume the opportunity even for a small heap. Waiting or new timers
        // alone must not turn this into recurring forced collection.
        if !self.begin_collection(now, before_used) {
            return ReclaimOutcome::SkippedSmallHeap;
        }
        let started = Instant::now();
        isolate.low_memory_notification();
        let duration = started.elapsed();
        isolate.get_heap_statistics(&mut heap);
        ReclaimOutcome::Collected {
            before_used,
            after_used: heap.used_heap_size(),
            duration,
        }
    }

    fn check_due(&mut self, now: Instant) -> Option<ReclaimOutcome> {
        match self.pending {
            None => Some(ReclaimOutcome::NotDue),
            Some(pending) if now >= pending.expires => {
                self.finish_attempt(now);
                Some(ReclaimOutcome::Expired)
            }
            Some(pending) if now < pending.deadline => Some(ReclaimOutcome::NotDue),
            Some(_) => None,
        }
    }

    fn finish_attempt(&mut self, now: Instant) {
        self.pending = None;
        self.next_eligible = Some(now + COOLDOWN);
    }

    fn begin_collection(&mut self, now: Instant, used_heap: usize) -> bool {
        self.finish_attempt(now);
        used_heap >= MIN_USED_HEAP
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_mesh_uploads_arm_and_activity_debounces_the_opportunity() {
        let now = Instant::now();
        let mut policy = IdleHeapReclaimer::default();
        policy.note_activity(now);
        policy.defer(now);
        assert_eq!(policy.next_deadline(), None);
        policy.on_mesh_upload(now);
        assert_eq!(policy.next_deadline(), Some(now + QUIET_PERIOD));
        assert_eq!(policy.check_due(now), Some(ReclaimOutcome::NotDue));
        policy.note_activity(now + Duration::from_millis(400));
        assert_eq!(
            policy.next_deadline(),
            Some(now + Duration::from_millis(900))
        );
        assert_eq!(
            policy.check_due(now + QUIET_PERIOD),
            Some(ReclaimOutcome::NotDue)
        );
        assert_eq!(policy.check_due(now + Duration::from_millis(900)), None);
    }

    #[test]
    fn retries_end_and_new_uploads_cannot_extend_the_five_second_window() {
        let now = Instant::now();
        let mut policy = IdleHeapReclaimer::default();
        policy.on_mesh_upload(now);
        policy.defer(now); // A premature retry cannot delay the quiet period.
        assert_eq!(policy.next_deadline(), Some(now + QUIET_PERIOD));
        policy.defer(now + QUIET_PERIOD);
        assert_eq!(
            policy.next_deadline(),
            Some(now + QUIET_PERIOD + RETRY_DELAY)
        );
        policy.on_mesh_upload(now + Duration::from_millis(4800));
        assert_eq!(policy.next_deadline(), Some(now + OPPORTUNITY_WINDOW));
        assert_eq!(
            policy.check_due(now + OPPORTUNITY_WINDOW),
            Some(ReclaimOutcome::Expired)
        );
        assert_eq!(policy.next_deadline(), None);
        policy.on_mesh_upload(now + OPPORTUNITY_WINDOW + Duration::from_secs(1));
        assert_eq!(policy.next_deadline(), None);
    }

    #[test]
    fn busy_retries_also_expire_without_running_collection() {
        let now = Instant::now();
        let mut policy = IdleHeapReclaimer::default();
        policy.on_mesh_upload(now);
        for retry in 10..100 {
            policy.defer(now + Duration::from_millis(retry * 50));
            assert!(policy.next_deadline().is_some());
        }
        policy.defer(now + OPPORTUNITY_WINDOW);
        assert_eq!(policy.next_deadline(), None);
        assert_eq!(
            policy.check_due(now + OPPORTUNITY_WINDOW),
            Some(ReclaimOutcome::NotDue)
        );
    }

    #[test]
    fn finishing_an_attempt_requires_a_new_upload_after_cooldown() {
        let now = Instant::now();
        let attempted = now + QUIET_PERIOD;
        let mut policy = IdleHeapReclaimer::default();
        policy.on_mesh_upload(now);
        assert_eq!(policy.check_due(attempted), None);
        policy.finish_attempt(attempted);
        policy.on_mesh_upload(attempted + COOLDOWN - Duration::from_nanos(1));
        assert_eq!(policy.next_deadline(), None);
        policy.note_activity(attempted + COOLDOWN);
        assert_eq!(policy.next_deadline(), None);
        policy.on_mesh_upload(attempted + COOLDOWN);
        assert_eq!(
            policy.next_deadline(),
            Some(attempted + COOLDOWN + QUIET_PERIOD)
        );
    }

    #[test]
    fn cancelling_animation_allows_a_later_static_upload() {
        let now = Instant::now();
        let mut policy = IdleHeapReclaimer::default();
        policy.on_mesh_upload(now);
        policy.cancel();
        assert_eq!(policy.next_deadline(), None);
        policy.on_mesh_upload(now + Duration::from_millis(1));
        assert_eq!(
            policy.next_deadline(),
            Some(now + Duration::from_millis(1) + QUIET_PERIOD)
        );
    }

    #[test]
    fn heap_threshold_skips_small_isolates_and_consumes_both_attempts() {
        let now = Instant::now();
        for (used, expected) in [(MIN_USED_HEAP - 1, false), (MIN_USED_HEAP, true)] {
            let mut policy = IdleHeapReclaimer::default();
            policy.on_mesh_upload(now);
            assert_eq!(policy.begin_collection(now + QUIET_PERIOD, used), expected);
            assert_eq!(policy.next_deadline(), None);
            policy.on_mesh_upload(now + QUIET_PERIOD + Duration::from_secs(1));
            assert_eq!(
                policy.next_deadline(),
                None,
                "small heaps must not retry every upload"
            );
        }
    }
}
