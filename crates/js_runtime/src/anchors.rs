//! Space-owned anchor commands and bounded native pose inboxes.
use deno_core::{op2, OpState};
use serde::Deserialize;
use serde::Serialize;
use std::{
    collections::{HashMap, VecDeque},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Instant,
};

pub const MAX_LISTENERS: usize = 16;
pub const MAX_TARGETS: usize = 16;
pub const HISTORY_FRAMES: usize = 64;
#[derive(Clone, Debug, Deserialize)]
pub struct ListenTarget {
    pub key: Option<String>,
    pub node: Option<i32>,
}
#[derive(Clone)]
pub struct Subscription {
    pub id: u32,
    pub targets: Vec<ListenTarget>,
    pub input: bool,
    pub history: bool,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Sample {
    pub index: usize,
    pub available: bool,
    pub position: [f32; 3],
    pub rotation: [f32; 4],
    pub scale: [f32; 3],
    pub error: Option<String>,
    pub input: Option<ControllerInput>,
}
#[derive(Clone, Debug, Serialize)]
pub struct ControllerInput {
    pub trigger: f32,
    pub grip: f32,
    pub primary: bool,
    pub secondary: bool,
}
#[derive(Clone)]
pub struct CapturedFrame {
    pub frame_id: u64,
    pub captured: Instant,
    pub samples: Vec<Sample>,
    pub discontinuous: bool,
}
struct Inbox {
    subscription: Subscription,
    frames: VecDeque<CapturedFrame>,
    lost: bool,
    availability: Option<Vec<bool>>,
}
#[derive(Default)]
struct ListenerState {
    next: u32,
    inboxes: HashMap<u32, Inbox>,
}
/// Pose data bypasses the ordinary command backlog. The host never waits for JS.
#[derive(Default)]
pub struct AnchorPort {
    state: Mutex<ListenerState>,
    pub input_ready: AtomicBool,
    pub tick_ready: AtomicBool,
    pub active: AtomicBool,
}
impl AnchorPort {
    pub fn subscriptions(&self) -> Vec<Subscription> {
        self.state
            .lock()
            .unwrap()
            .inboxes
            .values()
            .map(|s| s.subscription.clone())
            .collect()
    }
    pub fn publish(&self, id: u32, mut frame: CapturedFrame) -> bool {
        let mut state = self.state.lock().unwrap();
        let Some(inbox) = state.inboxes.get_mut(&id) else {
            return false;
        };
        let availability: Vec<_> = frame.samples.iter().map(|s| s.available).collect();
        frame.discontinuous |= inbox.availability.as_ref() != Some(&availability);
        inbox.availability = Some(availability);
        // Revoked access discards already queued poses as well as this pose.
        if frame.samples.iter().any(|s| s.error.is_some()) {
            inbox.frames.clear();
            frame.discontinuous = true;
        }
        if !inbox.subscription.history {
            frame.discontinuous |= inbox.frames.iter().any(|f| f.discontinuous);
            inbox.frames.clear();
        } else if inbox.frames.len() == HISTORY_FRAMES {
            inbox.frames.pop_front();
            inbox.lost = true;
        }
        inbox.frames.push_back(frame);
        if !inbox.subscription.input {
            self.tick_ready.store(true, Ordering::Release);
        }
        inbox.subscription.input && !self.input_ready.swap(true, Ordering::AcqRel)
    }
    fn take(&self, input_only: bool) -> Vec<(u32, CapturedFrame)> {
        let mut state = self.state.lock().unwrap();
        self.input_ready.store(false, Ordering::Release);
        let mut ready = Vec::new();
        for inbox in state.inboxes.values_mut() {
            if input_only && !inbox.subscription.input {
                continue;
            }
            if inbox.lost {
                if let Some(first) = inbox.frames.front_mut() {
                    first.discontinuous = true;
                }
                inbox.lost = false;
            }
            ready.extend(inbox.frames.drain(..).map(|f| (inbox.subscription.id, f)));
        }
        ready.sort_by_key(|(id, f)| (f.frame_id, *id));
        ready
    }
}
#[op2]
pub fn op_anchor_listen(
    state: &mut OpState,
    #[serde] targets: Vec<ListenTarget>,
    input: bool,
    history: bool,
) -> Result<u32, anyhow::Error> {
    anyhow::ensure!(
        !targets.is_empty() && targets.len() <= MAX_TARGETS,
        "Listen requires 1..16 targets"
    );
    for target in &targets {
        anyhow::ensure!(
            matches!((&target.key, target.node), (Some(key), None) if valid_key(key) && !key.starts_with("__"))
                || matches!((&target.key, target.node), (None, Some(node)) if node >= 0),
            "Expected an anchor key or resolved element"
        );
    }
    let port = state.borrow::<Arc<AnchorPort>>();
    let mut inner = port.state.lock().unwrap();
    anyhow::ensure!(
        inner.inboxes.len() < MAX_LISTENERS,
        "Too many anchor listeners"
    );
    inner.next = inner
        .next
        .checked_add(1)
        .ok_or_else(|| anyhow::anyhow!("Listener ID exhausted"))?;
    let id = inner.next;
    inner.inboxes.insert(
        id,
        Inbox {
            subscription: Subscription {
                id,
                targets,
                input,
                history,
            },
            frames: VecDeque::new(),
            lost: false,
            availability: None,
        },
    );
    port.active.store(true, Ordering::Release);
    Ok(id)
}
#[op2(fast)]
pub fn op_anchor_unlisten(state: &mut OpState, id: u32) {
    let port = state.borrow::<Arc<AnchorPort>>();
    let mut inner = port.state.lock().unwrap();
    inner.inboxes.remove(&id);
    port.active
        .store(!inner.inboxes.is_empty(), Ordering::Release);
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Delivery {
    id: u32,
    frame_id: u64,
    timestamp: f64,
    discontinuous: bool,
    samples: Vec<Sample>,
}
#[op2]
#[serde]
pub fn op_anchor_samples(state: &mut OpState, input_only: bool) -> Vec<Delivery> {
    let origin = state.borrow::<super::PerfState>().start;
    state
        .borrow::<Arc<AnchorPort>>()
        .take(input_only)
        .into_iter()
        .map(|(id, f)| Delivery {
            id,
            frame_id: f.frame_id,
            timestamp: f.captured.saturating_duration_since(origin).as_secs_f64() * 1000.,
            discontinuous: f.discontinuous,
            samples: f.samples,
        })
        .collect()
}

pub const MAX_ANCHORS: usize = 64;
#[derive(Debug, Clone)]
pub enum Source {
    Native(String),
    Element(i32),
}
#[derive(Debug)]
pub struct Request {
    pub id: u32,
    pub key: String,
    pub source: Option<Source>,
    pub guard: Option<u32>,
}
#[derive(Serialize)]
pub struct Reply {
    pub id: u32,
    pub error: Option<String>,
}
#[derive(Default)]
pub struct AnchorQueue {
    next: u32,
    pub requests: Vec<Request>,
    pub replies: Vec<Reply>,
}

pub fn valid_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= 128
        && key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-.:".contains(&b))
}

#[op2(fast)]
pub fn op_anchor_request(
    state: &mut OpState,
    #[string] key: &str,
    #[string] native: &str,
    #[smi] node: i32,
    remove: bool,
    expected: u32,
) -> Result<u32, anyhow::Error> {
    anyhow::ensure!(valid_key(key), "Invalid anchor key");
    let queue = state.borrow_mut::<AnchorQueue>();
    anyhow::ensure!(
        queue.requests.len() < MAX_ANCHORS,
        "Anchor request queue full"
    );
    let source = if remove {
        None
    } else if !native.is_empty() {
        anyhow::ensure!(
            native.starts_with("__") && valid_key(native),
            "Invalid native reference"
        );
        Some(Source::Native(native.into()))
    } else {
        anyhow::ensure!(node >= 0, "Anchor element must be resolved");
        Some(Source::Element(node))
    };
    queue.next = queue
        .next
        .checked_add(1)
        .ok_or_else(|| anyhow::anyhow!("Anchor request ID exhausted"))?;
    let id = queue.next;
    queue.requests.push(Request {
        id,
        key: key.into(),
        source,
        guard: (expected != 0).then_some(expected),
    });
    Ok(id)
}
#[op2]
#[serde]
pub fn op_anchor_poll(state: &mut OpState) -> Vec<Reply> {
    std::mem::take(&mut state.borrow_mut::<AnchorQueue>().replies)
}
