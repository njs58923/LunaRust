//! Opt-in retained-heap experiments: manual full collection and the idle policy.
use crate::{
    components::{parse_events, Channel, ComponentPort},
    idle_heap::{IdleHeapReclaimer, ReclaimOutcome},
    mesh::MeshCommand,
    Engine,
};
use deno_core::v8;
use serde_json::Value;
use std::{
    collections::HashMap,
    path::PathBuf,
    process::Command,
    sync::{mpsc, Arc},
    thread,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, Default)]
struct Heap {
    used: usize,
    total: usize,
    physical: usize,
    external: usize,
}

impl Heap {
    fn read(engine: &mut Engine) -> Self {
        let mut stats = v8::HeapStatistics::default();
        engine.rt.v8_isolate().get_heap_statistics(&mut stats);
        Self {
            used: stats.used_heap_size(),
            total: stats.total_heap_size(),
            physical: stats.total_physical_size(),
            external: stats.external_memory(),
        }
    }

    fn add(&mut self, other: Self) {
        self.used += other.used;
        self.total += other.total;
        self.physical += other.physical;
        self.external += other.external;
    }
}

// This guard protects script evaluation and callbacks in the experiment. V8's
// native GC cannot be preempted like JavaScript; its duration is measured below.
struct Guard(mpsc::Sender<()>, Option<thread::JoinHandle<()>>);
impl Guard {
    fn new(engine: &mut Engine) -> Self {
        let handle = engine.execution_handle();
        let (done, receiver) = mpsc::channel();
        let join = thread::spawn(move || {
            if matches!(
                receiver.recv_timeout(Duration::from_secs(15)),
                Err(mpsc::RecvTimeoutError::Timeout)
            ) {
                handle.terminate_execution();
            }
        });
        Self(done, Some(join))
    }
}
impl Drop for Guard {
    fn drop(&mut self) {
        let _ = self.0.send(());
        if let Some(join) = self.1.take() {
            let _ = join.join();
        }
    }
}

fn drain(engine: &mut Engine, next_node: &mut i32) -> (usize, usize, usize) {
    drain_with_activity(engine, next_node).0
}

fn drain_with_activity(engine: &mut Engine, next_node: &mut i32) -> ((usize, usize, usize), bool) {
    let (_, commands) = engine.drain_mesh_commands();
    let uploads = commands
        .iter()
        .filter(|c| matches!(c, MeshCommand::Upload(..)))
        .count();
    let disposals = commands.len() - uploads;
    let creations = engine.drain_element_creation_queue();
    let created = creations.len();
    for (request, _) in creations {
        engine.push_element_creation_result(request, *next_node);
        *next_node += 1;
    }
    let logs = engine.drain_logs();
    assert!(
        logs.iter().all(|(level, _)| level != "error"),
        "Facade errors: {logs:?}"
    );
    let activity = uploads
        + disposals
        + created
        + logs.len()
        + engine.drain_attr_updates().len()
        + engine.drain_transform_position_updates().len()
        + engine.drain_transform_rotation_updates().len()
        + engine.drain_transform_scale_updates().len()
        + engine.drain_hierarchy_append_queue().len()
        + engine.drain_remove_element_queue().len()
        > 0;
    ((uploads, disposals, created), activity)
}

fn settle(engine: &mut Engine, next_node: &mut i32) {
    for _ in 0..32 {
        engine.fire_raf(32.0);
        let (uploads, disposals, created) = drain(engine, next_node);
        assert_eq!((uploads, disposals), (0, 0));
        assert!(!engine.has_pending_animation_frames());
        if created == 0
            && engine
                .next_timer_deadline()
                .is_none_or(|d| d > Instant::now())
        {
            return;
        }
    }
    panic!("Facade did not settle immediate creation callbacks");
}

fn measure(
    source: &str,
    mut recipe: Value,
    use_policy: bool,
) -> (Heap, Heap, Heap, Heap, f64, f64, Option<ReclaimOutcome>) {
    // All V8 access, including the optional full collection, stays on this worker.
    let mut engine = Engine::new();
    let mut guard = Some(Guard::new(&mut engine));
    engine
        .eval("globalThis.hiperspace.dimention = new HSMLRootElement(0);")
        .unwrap();
    engine.update_attr_snapshot(
        [
            (0, HashMap::new()),
            (1, [("id".into(), "obra".into())].into()),
            (
                2,
                [
                    ("id".into(), "toque".into()),
                    ("color".into(), "transparent".into()),
                ]
                .into(),
            ),
        ]
        .into(),
    );
    engine
        .update_tag_snapshot([(0, "space".into()), (1, "group".into()), (2, "box".into())].into());
    engine.update_hierarchy_snapshot(
        [(0, -1), (1, 0), (2, 0)].into(),
        [(0, vec![1, 2]), (1, vec![]), (2, vec![])].into(),
    );
    let parent = Arc::new(ComponentPort::default());
    let child = Arc::new(ComponentPort::default());
    let channel = Channel::new(
        &parent,
        &child,
        1,
        "http://localhost:2053".into(),
        recipe.clone(),
        parse_events("armada,cambio").unwrap(),
    )
    .unwrap();
    engine.configure_component_port(child);
    let baseline = Heap::read(&mut engine);
    engine
        .eval_cached("http://localhost:2053/objetos/pared.js", source)
        .unwrap();
    let mut next_node = 3;
    engine.fire_raf(16.0);
    let (uploads, disposals, _) = drain(&mut engine, &mut next_node);
    assert_eq!((uploads, disposals), (1, 0));
    let mut policy = IdleHeapReclaimer::default();
    if use_policy {
        policy.on_mesh_upload(Instant::now());
    }
    assert!(parent
        .drain()
        .iter()
        .any(|event| event.kind == "component:armada"));
    settle(&mut engine, &mut next_node);
    if use_policy {
        policy.note_activity(Instant::now());
    }
    let settled = Heap::read(&mut engine);
    let (idle, collected, gc_ms, outcome) = if use_policy {
        loop {
            let due = policy
                .next_deadline()
                .expect("Real static facade lost its reclaim opportunity");
            let timer = engine.next_timer_deadline();
            let wake = timer.map_or(due, |timer| timer.min(due));
            thread::sleep(wake.saturating_duration_since(Instant::now()));
            let now = Instant::now();
            // Like the worker, service actual timers first and never collect
            // with a timer due in the next 20 ms. No fabricated clock advance.
            if timer.is_some_and(|timer| timer <= now) {
                engine.fire_raf(500.0);
                let (commands, activity) = drain_with_activity(&mut engine, &mut next_node);
                assert_eq!(commands, (0, 0, 0));
                assert!(!engine.has_pending_animation_frames());
                if activity {
                    policy.note_activity(Instant::now());
                }
                continue;
            }
            if now < due {
                continue;
            }
            if timer.is_some_and(|timer| timer <= now + Duration::from_millis(20)) {
                policy.defer(now);
                continue;
            }
            let before = Heap::read(&mut engine);
            // Native collection runs with the JS watchdog disarmed, as in the
            // worker. Re-arm the guard before servicing any further callbacks.
            drop(guard.take());
            let result = policy.collect_if_due(&mut engine, now);
            guard = Some(Guard::new(&mut engine));
            assert_eq!(engine.next_timer_deadline(), timer);
            match result {
                ReclaimOutcome::NotDue | ReclaimOutcome::Deferred => continue,
                ReclaimOutcome::Collected { duration, .. } => {
                    assert_eq!(policy.next_deadline(), None);
                    break (
                        before,
                        Heap::read(&mut engine),
                        duration.as_secs_f64() * 1000.0,
                        Some(result),
                    );
                }
                ReclaimOutcome::SkippedSmallHeap => {
                    assert_eq!(policy.next_deadline(), None);
                    break (before, Heap::read(&mut engine), 0.0, Some(result));
                }
                ReclaimOutcome::Expired => {
                    panic!("Real facade never became quiet enough for collection")
                }
            }
        }
    } else {
        // Real facades keep _vr.js's 150 ms service timer. Give automatic GC the
        // same low-allocation activity before requesting one explicit collection.
        let idle_end = Instant::now() + Duration::from_millis(350);
        while Instant::now() < idle_end {
            let wake = engine
                .next_timer_deadline()
                .unwrap_or(idle_end)
                .min(idle_end);
            thread::sleep(wake.saturating_duration_since(Instant::now()));
            if Instant::now() >= idle_end {
                break;
            }
            engine.fire_raf(500.0);
            assert_eq!(drain(&mut engine, &mut next_node), (0, 0, 0));
            assert!(!engine.has_pending_animation_frames());
        }
        let idle = Heap::read(&mut engine);
        let deadline = engine.next_timer_deadline();
        drop(guard.take());
        let gc_started = Instant::now();
        engine.rt.v8_isolate().low_memory_notification();
        let gc_ms = gc_started.elapsed().as_secs_f64() * 1000.0;
        let collected = Heap::read(&mut engine);
        guard = Some(Guard::new(&mut engine));
        assert_eq!(
            engine.next_timer_deadline(),
            deadline,
            "GC must preserve timer scheduling"
        );
        (idle, collected, gc_ms, None)
    };
    parent.send(1, "informar".into(), "null").unwrap();
    engine.fire_raf(1000.0);
    assert_eq!(drain(&mut engine, &mut next_node), (0, 0, 0));
    assert!(
        parent
            .drain()
            .iter()
            .any(|event| event.kind == "component:armada"),
        "informar listener must survive collection"
    );
    recipe["material"] = Value::String(
        if recipe["material"] == "hormigon" {
            "ladrillo"
        } else {
            "hormigon"
        }
        .into(),
    );
    channel.update_props(recipe);
    let redraw_started = Instant::now();
    engine.fire_raf(1200.0);
    let (uploads, disposals, _) = drain(&mut engine, &mut next_node);
    let redraw_ms = redraw_started.elapsed().as_secs_f64() * 1000.0;
    assert_eq!(
        (uploads, disposals),
        (1, 1),
        "props listener must rebuild after collection"
    );
    assert!(parent
        .drain()
        .iter()
        .any(|event| event.kind == "component:armada"));
    drop(guard);
    (
        baseline, settled, idle, collected, gc_ms, redraw_ms, outcome,
    )
}

fn street_fixture() -> (Arc<String>, Vec<Value>) {
    let root = std::env::var_os("LUNA_STREET_SOURCE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../server_expo"));
    let output = Command::new("bun")
        .current_dir(&root)
        .args([
            "-e",
            r#"
        const {construirCalle} = await import('./src/calle.ts');
        const text = construirCalle('http://localhost:2053');
        const config = JSON.parse(/globalThis\.CALLE = (.*);/.exec(text)[1]);
        console.log(JSON.stringify(config.fachadas.map(f =>
            ({...f.pared, modo:'mesh', caras:1, tocable:false}))));
    "#,
        ])
        .output()
        .expect("Bun is required for the real facade recipes");
    assert!(
        output.status.success(),
        "Street generation failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let recipes: Vec<Value> = serde_json::from_slice(&output.stdout).unwrap();
    assert!(!recipes.is_empty());
    let source = ["_base.js", "_vr.js", "_obra.js", "pared.js"]
        .map(|file| std::fs::read_to_string(root.join("public/objetos").join(file)).unwrap())
        .join("\n;\n");
    let source = Arc::new(format!("(function(){{\n{source}\n}})();"));
    (source, recipes)
}

#[test]
#[ignore = "real facade retained V8 heap/GC experiment; requires server_expo and Bun"]
fn benchmark_real_street_retained_heap() {
    let (source, recipes) = street_fixture();
    let count = recipes.len();
    let mut totals = [Heap::default(); 4];
    let mut gc_times = Vec::new();
    for (index, recipe) in recipes.into_iter().enumerate() {
        let source = source.clone();
        let (baseline, settled, idle, collected, gc_ms, redraw_ms, _) =
            thread::spawn(move || measure(&source, recipe, false))
                .join()
                .expect("Facade heap worker failed");
        for (total, value) in totals.iter_mut().zip([baseline, settled, idle, collected]) {
            total.add(value);
        }
        gc_times.push(gc_ms);
        eprintln!("street_heap index={index} settled_used={} idle_used={} collected_used={} idle_external={} collected_external={} gc_ms={gc_ms:.3} redraw_ms={redraw_ms:.3}",
            settled.used, idle.used, collected.used, idle.external, collected.external);
    }
    gc_times.sort_by(f64::total_cmp);
    eprintln!("street_heap facades={count} baseline={:?} settled={:?} idle350ms={:?} collected={:?} gc_sum_ms={:.3} gc_median_ms={:.3} gc_max_ms={:.3}",
        totals[0], totals[1], totals[2], totals[3], gc_times.iter().sum::<f64>(), gc_times[count / 2], gc_times[count - 1]);
}

#[test]
#[ignore = "real facade idle heap policy benchmark; requires server_expo and Bun"]
fn benchmark_real_street_idle_heap_policy() {
    let (source, recipes) = street_fixture();
    let count = recipes.len();
    let mut totals = [Heap::default(); 4];
    let mut gc_times = Vec::new();
    let mut skipped = 0;
    for (index, recipe) in recipes.into_iter().enumerate() {
        let source = source.clone();
        let (baseline, settled, idle, after, gc_ms, redraw_ms, outcome) =
            thread::spawn(move || measure(&source, recipe, true))
                .join()
                .expect("Facade idle policy worker failed");
        for (total, value) in totals.iter_mut().zip([baseline, settled, idle, after]) {
            total.add(value);
        }
        match outcome {
            Some(ReclaimOutcome::Collected { .. }) => gc_times.push(gc_ms),
            Some(ReclaimOutcome::SkippedSmallHeap) => skipped += 1,
            other => panic!("Unexpected real facade collection outcome: {other:?}"),
        }
        eprintln!("street_idle_heap index={index} outcome={outcome:?} settled_used={} idle_used={} after_used={} idle_physical={} after_physical={} idle_external={} after_external={} gc_ms={gc_ms:.3} redraw_ms={redraw_ms:.3}",
            settled.used, idle.used, after.used, idle.physical, after.physical, idle.external, after.external);
    }
    assert_eq!(gc_times.len() + skipped, count);
    gc_times.sort_by(f64::total_cmp);
    eprintln!("street_idle_heap facades={count} collected={} skipped_small={skipped} baseline={:?} settled={:?} before_policy={:?} after_policy={:?} gc_sum_ms={:.3} gc_median_ms={:.3} gc_max_ms={:.3}",
        gc_times.len(), totals[0], totals[1], totals[2], totals[3], gc_times.iter().sum::<f64>(),
        gc_times.get(gc_times.len() / 2).copied().unwrap_or_default(), gc_times.last().copied().unwrap_or_default());
}
