//! Opt-in integration benchmark of the real street facade scripts.
//!
//! PowerShell: `$env:LUNA_STREET_SOURCE_DIR = '../server_expo'`, then
//! `cargo test -p luna --lib benchmark_real_street_facades -- --ignored --nocapture`.
//! Requires Bun to generate the exact facade recipes, but starts no HTTP server,
//! window, or GPU. Measures isolate/bootstrap/script/mesh CPU wall time and binary
//! upload sizes, not renderer frame time or V8's retained heap.

use crate::js::{
    spawn_space_worker, stop_space_worker, JsTickData, JsWorkerCommand, JsWorkerEvent,
    SpaceScriptWorker, SpaceSnapshots,
};
use js_runtime::{
    components::{parse_events, Channel, ComponentPort},
    mesh::MeshCommand,
};
use serde_json::Value;
use std::{
    collections::HashMap,
    path::PathBuf,
    process::Command,
    sync::{mpsc, Arc},
    time::{Duration, Instant},
};

struct Facade {
    worker: SpaceScriptWorker,
    channel: Arc<Channel>,
    next_node: i32,
}

impl Drop for Facade {
    fn drop(&mut self) {
        stop_space_worker(&mut self.worker);
    }
}

fn receive(worker: &SpaceScriptWorker) -> JsWorkerEvent {
    match worker.event_rx.recv_timeout(Duration::from_secs(15)) {
        Ok(JsWorkerEvent::WorkerError(error)) => panic!("Facade worker failed: {error}"),
        Ok(event) => event,
        Err(error) => panic!("Facade worker did not respond: {error}"),
    }
}

fn tick(worker: &mut SpaceScriptWorker, timestamp: f64) -> JsTickData {
    worker
        .try_send(JsWorkerCommand::Tick {
            elapsed_ms: timestamp,
        })
        .unwrap();
    let JsWorkerEvent::TickData(data) = receive(worker) else {
        panic!("Expected facade tick output");
    };
    assert!(
        data.logs.iter().all(|(level, _)| level != "error"),
        "Facade script errors: {:?}",
        data.logs
    );
    data
}

fn facade_snapshot() -> SpaceSnapshots {
    // The public space in pared.hsml: the model is created by the real script.
    SpaceSnapshots {
        attr_snap: [
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
        tag_snap: [(0, "space".into()), (1, "group".into()), (2, "box".into())].into(),
        parents: [(0, -1), (1, 0), (2, 0)].into(),
        children: [(0, vec![1, 2]), (1, vec![]), (2, vec![])].into(),
        positions: HashMap::new(),
        rotations: HashMap::new(),
        scales: HashMap::new(),
        global_positions: HashMap::new(),
    }
}

impl Facade {
    fn new(parent: &Arc<ComponentPort>, index: usize, props: Value) -> Self {
        let mut worker = spawn_space_worker(700_000 + index as u32).unwrap();
        worker
            .try_send(JsWorkerCommand::UpdateSnapshots(facade_snapshot()))
            .unwrap();
        assert!(matches!(receive(&worker), JsWorkerEvent::SnapshotApplied));
        let channel = Channel::new(
            parent,
            &worker.component_port,
            index as i32 + 1,
            "http://localhost:2053".into(),
            props,
            parse_events("armada,cambio").unwrap(),
        )
        .unwrap();
        Self {
            worker,
            channel,
            next_node: 3,
        }
    }

    fn eval(&mut self, source: &str) {
        self.worker
            .try_send(JsWorkerCommand::EvalScript {
                url: "http://localhost:2053/objetos/pared.js".into(),
                code: source.into(),
            })
            .unwrap();
        match receive(&self.worker) {
            JsWorkerEvent::EvalResult { error: None, .. } => {}
            JsWorkerEvent::EvalResult { error, .. } => {
                panic!("Real facade script failed: {error:?}")
            }
            _ => panic!("Expected facade evaluation result"),
        }
    }

    fn settle(&mut self, data: &JsTickData) {
        // Resolve asynchronous createElement handles as the host does. No Bevy
        // entities are spawned: this benchmark stops at the renderer boundary.
        if !data.creation_queue.is_empty() {
            let results = data
                .creation_queue
                .iter()
                .map(|(request, _)| {
                    let node = self.next_node;
                    self.next_node += 1;
                    (*request, node)
                })
                .collect();
            self.worker
                .try_send(JsWorkerCommand::PushElementCreationResults(results))
                .unwrap();
        }
        let settled = tick(&mut self.worker, 32.0);
        assert!(settled.creation_queue.is_empty());
        assert!(settled.mesh_commands.1.is_empty());
        assert!(!settled.needs_continuous_ticks);
        assert!(settled.next_timer_deadline.is_none());
        self.worker.component_port.take_wake();
    }
}

#[derive(Default)]
struct MeshTotals {
    uploads: usize,
    disposals: usize,
    vertices: usize,
    indices: usize,
    bytes: usize,
}

fn mesh_totals(data: &JsTickData) -> MeshTotals {
    let mut totals = MeshTotals::default();
    for command in &data.mesh_commands.1 {
        match command {
            MeshCommand::Upload(_, upload) => {
                let mesh = &upload.data;
                totals.uploads += 1;
                totals.vertices += mesh.positions.len();
                totals.indices += mesh.indices.len();
                totals.bytes += mesh.positions.len() * 12
                    + mesh.normals.len() * 12
                    + mesh.uvs.len() * 8
                    + mesh.colors.len() * 16
                    + mesh.indices.len() * 4;
            }
            MeshCommand::Dispose(_) => totals.disposals += 1,
        }
    }
    totals
}

#[test]
#[ignore = "real street CPU benchmark; requires LUNA_STREET_SOURCE_DIR and Bun, no GPU/FPS"]
fn benchmark_real_street_facades() {
    let source_dir = PathBuf::from(
        std::env::var_os("LUNA_STREET_SOURCE_DIR")
            .expect("Set LUNA_STREET_SOURCE_DIR to the server_expo directory"),
    );
    let source_dir = source_dir
        .canonicalize()
        .expect("Street source directory does not exist");
    let output = Command::new("bun")
        .current_dir(&source_dir)
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
        .expect("Bun is required to generate the real street recipes");
    assert!(
        output.status.success(),
        "Street generation failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let recipes: Vec<Value> =
        serde_json::from_slice(&output.stdout).expect("Expected facade recipe JSON");
    assert!(!recipes.is_empty());
    // This is the same concatenation order used by server_expo's script route.
    let source = ["_base.js", "_vr.js", "_obra.js", "pared.js"]
        .map(|file| {
            std::fs::read_to_string(source_dir.join("public/objetos").join(file))
                .unwrap_or_else(|error| panic!("Cannot read {file}: {error}"))
        })
        .join("\n;\n");
    let parent = Arc::new(ComponentPort::default());
    let mut facades = Vec::new();
    let mut totals = MeshTotals::default();
    let mut eval_ms = Vec::new();
    let mount_start = Instant::now();
    for (index, recipe) in recipes.iter().enumerate() {
        let mut facade = Facade::new(&parent, index, recipe.clone());
        let started = Instant::now();
        facade.eval(&source);
        eval_ms.push(started.elapsed().as_secs_f64() * 1000.0);
        let data = tick(&mut facade.worker, 16.0);
        let geometry = mesh_totals(&data);
        assert_eq!(
            geometry.uploads, 1,
            "Facade {index} did not generate one mesh"
        );
        assert_eq!(geometry.disposals, 0);
        let events = parent.drain();
        let armed = events
            .iter()
            .find(|e| e.kind == "component:armada")
            .expect("Real facade did not emit armada");
        assert_eq!(armed.detail["modo"], "mesh");
        assert_eq!(
            armed.detail["vertices"].as_u64(),
            Some(geometry.vertices as u64)
        );
        totals.uploads += geometry.uploads;
        totals.vertices += geometry.vertices;
        totals.indices += geometry.indices;
        totals.bytes += geometry.bytes;
        facade.settle(&data);
        facades.push(facade);
    }
    let mount_ms = mount_start.elapsed().as_secs_f64() * 1000.0;
    // All real isolates remain alive together, as they do after street loading.
    // No host ticks are sent in this interval. Check for unsolicited output and
    // then ask once to confirm their scripts did not schedule RAF or timers.
    std::thread::sleep(Duration::from_millis(200));
    let idle_check = Instant::now();
    for facade in &mut facades {
        assert!(matches!(
            facade.worker.event_rx.try_recv(),
            Err(mpsc::TryRecvError::Empty)
        ));
        let idle = tick(&mut facade.worker, 1000.0);
        assert!(!idle.needs_continuous_ticks);
        assert!(idle.next_timer_deadline.is_none());
        assert!(idle.mesh_commands.1.is_empty());
        assert!(idle.creation_queue.is_empty());
        assert!(idle.attr_updates.is_empty());
    }
    let idle_probe_ms = idle_check.elapsed().as_secs_f64() * 1000.0;
    let facade = &mut facades[0];
    // The real informar handler still responds after dormancy without a redraw.
    parent.send(1, "informar".into(), "null").unwrap();
    assert!(facade.worker.component_port.take_wake());
    let informed = tick(&mut facade.worker, 1100.0);
    assert!(informed.mesh_commands.1.is_empty());
    assert!(parent
        .drain()
        .iter()
        .any(|event| event.kind == "component:armada"));
    // A property update must awaken its original listeners and replace only its
    // own geometry; the other 24 facade isolates stay dormant.
    let mut changed = recipes[0].clone();
    changed["material"] = Value::String(
        if changed["material"] == "hormigon" {
            "ladrillo"
        } else {
            "hormigon"
        }
        .into(),
    );
    facade.channel.update_props(changed);
    assert!(facade.worker.component_port.take_wake());
    let update_start = Instant::now();
    let updated = tick(&mut facade.worker, 1200.0);
    let update_ms = update_start.elapsed().as_secs_f64() * 1000.0;
    let update_geometry = mesh_totals(&updated);
    assert_eq!(update_geometry.uploads, 1);
    assert_eq!(update_geometry.disposals, 1);
    assert!(parent
        .drain()
        .iter()
        .any(|event| event.kind == "component:armada"));
    facade.settle(&updated);
    for facade in &facades[1..] {
        assert!(!facade.worker.component_port.take_wake());
        assert!(matches!(
            facade.worker.event_rx.try_recv(),
            Err(mpsc::TryRecvError::Empty)
        ));
    }
    eval_ms.sort_by(f64::total_cmp);
    println!("facades={} source_bytes={} uploads={} vertices={} indices={} upload_bytes={} mount_ms={:.3} eval_median_ms={:.3} eval_max_ms={:.3} idle_probe_all_ms={:.3} props_redraw_ms={:.3}",
        facades.len(), source.len(), totals.uploads, totals.vertices, totals.indices, totals.bytes,
        mount_ms, eval_ms[eval_ms.len()/2], eval_ms.last().unwrap(), idle_probe_ms, update_ms);
}
