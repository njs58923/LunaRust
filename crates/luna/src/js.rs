use std::{
    collections::{HashMap, HashSet},
    sync::mpsc,
    thread::{self, JoinHandle},
    time::Instant,
};

use bevy::prelude::*;
use specs::{Join, WorldExt};

use js_runtime::Engine as JsEngine;
use virtual_dom::dom::element::{Attrs, Hierarchy, Tag, Transform2};

use crate::{
    AttributeUpdates, DeleteRequests, DirtyNodes, ElemenetWorld, LogLevel, LogPanel,
    PendingScripts, ReloadTrigger,
};

// ─── Types ───────────────────────────────────────────────────────────────────

pub struct SpaceScriptContext {
    pub engine: JsEngine,
    pub loaded_scripts: HashSet<String>,
}

#[derive(Clone)]
pub struct SpaceSnapshots {
    pub attr_snap: HashMap<i32, HashMap<String, String>>,
    pub tag_snap: HashMap<i32, String>,
    pub positions: HashMap<i32, js_runtime::Vec3>,
    pub rotations: HashMap<i32, js_runtime::Vec3>,
    pub scales: HashMap<i32, js_runtime::Vec3>,
    pub global_positions: HashMap<i32, js_runtime::Vec3>,
    pub parents: HashMap<i32, i32>,
    pub children: HashMap<i32, Vec<i32>>,
}

pub struct JsTickData {
    pub logs: Vec<(String, String)>,
    pub attr_updates: Vec<(i32, String, String)>,
    pub pos_updates: Vec<(i32, js_runtime::Vec3)>,
    pub rot_updates: Vec<(i32, js_runtime::Vec3)>,
    pub scale_updates: Vec<(i32, js_runtime::Vec3)>,
    pub creation_queue: Vec<(i32, String)>,
    pub hierarchy_queue: Vec<(i32, i32)>,
    pub remove_queue: Vec<i32>,
    pub fetch_queue: Vec<(i32, String)>,
    pub navigate_queue: Vec<String>,
}

pub enum JsWorkerCommand {
    UpdateSnapshots(SpaceSnapshots),
    EvalScript { url: String, code: String },
    Tick { elapsed_ms: f64 },
    PushElementCreationResults(Vec<(i32, i32)>),
    PushFetchResults(Vec<(i32, std::result::Result<String, String>)>),
    Shutdown,
}

pub enum JsWorkerEvent {
    EvalResult { url: String, already_loaded: bool, error: Option<String> },
    TickData(JsTickData),
    WorkerError(String),
}

pub struct SpaceScriptWorker {
    pub cmd_tx: mpsc::Sender<JsWorkerCommand>,
    pub event_rx: mpsc::Receiver<JsWorkerEvent>,
    pub join: Option<JoinHandle<()>>,
}

#[derive(Default)]
pub struct ScriptRuntimeManager {
    pub contexts: HashMap<u32, SpaceScriptWorker>,
}

impl Drop for ScriptRuntimeManager {
    fn drop(&mut self) {
        for worker in self.contexts.values_mut() {
            stop_space_worker(worker);
        }
    }
}

// ─── Init ────────────────────────────────────────────────────────────────────

pub fn init_js_runtime(world: &mut World) {
    let mut log_panel = world.resource_mut::<LogPanel>();
    log_panel.push_info("[JS] Initializing V8 runtime...");
    js_runtime::init_v8_platform();
    log_panel.push_info("[JS] Creating runtime manager (isolates per <space>)...");
    drop(log_panel);

    world.insert_non_send_resource(ScriptRuntimeManager::default());

    world.resource_mut::<LogPanel>().push_info("[JS] Runtime initialization complete");
}

// ─── Worker lifecycle ────────────────────────────────────────────────────────

fn create_space_context(space_id: u32) -> std::result::Result<SpaceScriptContext, String> {
    let mut engine = std::panic::catch_unwind(JsEngine::new)
        .map_err(|e| format!("panic creating JS engine for space {}: {:?}", space_id, e))?;

    let set_root = format!(
        "globalThis.hiperspace.dimention = new HSMLRootElement({});",
        space_id
    );
    engine
        .eval(&set_root)
        .map_err(|e| format!("failed to set JS root for space {}: {}", space_id, e))?;

    Ok(SpaceScriptContext { engine, loaded_scripts: HashSet::new() })
}

pub fn spawn_space_worker(space_id: u32) -> std::result::Result<SpaceScriptWorker, String> {
    let (cmd_tx, cmd_rx) = mpsc::channel::<JsWorkerCommand>();
    let (event_tx, event_rx) = mpsc::channel::<JsWorkerEvent>();

    let join = thread::Builder::new()
        .name(format!("js-space-{}", space_id))
        .spawn(move || {
            let mut ctx = match create_space_context(space_id) {
                Ok(ctx) => ctx,
                Err(err) => {
                    let _ = event_tx.send(JsWorkerEvent::WorkerError(err));
                    return;
                }
            };

            while let Ok(cmd) = cmd_rx.recv() {
                match cmd {
                    JsWorkerCommand::UpdateSnapshots(snap) => {
                        ctx.engine.update_attr_snapshot(snap.attr_snap);
                        ctx.engine.update_tag_snapshot(snap.tag_snap);
                        ctx.engine.update_transform_snapshot(
                            snap.positions, snap.rotations, snap.scales, snap.global_positions,
                        );
                        ctx.engine.update_hierarchy_snapshot(snap.parents, snap.children);
                    }
                    JsWorkerCommand::EvalScript { url, code } => {
                        if ctx.loaded_scripts.contains(&url) {
                            let _ = event_tx.send(JsWorkerEvent::EvalResult {
                                url, already_loaded: true, error: None,
                            });
                            continue;
                        }
                        let wrapped_code = format!("(function(){{\n{}\n}})();", code);
                        match ctx.engine.eval(&wrapped_code) {
                            Ok(_) => {
                                ctx.loaded_scripts.insert(url.clone());
                                let _ = event_tx.send(JsWorkerEvent::EvalResult {
                                    url, already_loaded: false, error: None,
                                });
                            }
                            Err(e) => {
                                let _ = event_tx.send(JsWorkerEvent::EvalResult {
                                    url, already_loaded: false, error: Some(e.to_string()),
                                });
                            }
                        }
                    }
                    JsWorkerCommand::Tick { elapsed_ms } => {
                        ctx.engine.fire_raf(elapsed_ms);
                        let tick_data = JsTickData {
                            logs: ctx.engine.drain_logs(),
                            attr_updates: ctx.engine.drain_attr_updates(),
                            pos_updates: ctx.engine.drain_transform_position_updates(),
                            rot_updates: ctx.engine.drain_transform_rotation_updates(),
                            scale_updates: ctx.engine.drain_transform_scale_updates(),
                            creation_queue: ctx.engine.drain_element_creation_queue(),
                            hierarchy_queue: ctx.engine.drain_hierarchy_append_queue(),
                            remove_queue: ctx.engine.drain_remove_element_queue(),
                            fetch_queue: ctx.engine.drain_fetch_queue(),
                            navigate_queue: ctx.engine.drain_navigate_queue(),
                        };
                        let _ = event_tx.send(JsWorkerEvent::TickData(tick_data));
                    }
                    JsWorkerCommand::PushElementCreationResults(results) => {
                        for (request_id, node_id) in results {
                            ctx.engine.push_element_creation_result(request_id, node_id);
                        }
                    }
                    JsWorkerCommand::PushFetchResults(results) => {
                        for (request_id, result) in results {
                            ctx.engine.push_fetch_result(request_id, result);
                        }
                    }
                    JsWorkerCommand::Shutdown => break,
                }
            }
        })
        .map_err(|e| format!("failed to spawn JS worker for space {}: {}", space_id, e))?;

    Ok(SpaceScriptWorker { cmd_tx, event_rx, join: Some(join) })
}

pub fn stop_space_worker(worker: &mut SpaceScriptWorker) {
    let _ = worker.cmd_tx.send(JsWorkerCommand::Shutdown);
    if let Some(join) = worker.join.take() {
        let _ = join.join();
    }
}

fn filter_snapshot_map<T: Clone>(source: &HashMap<i32, T>, allowed: &HashSet<i32>) -> HashMap<i32, T> {
    source.iter()
        .filter(|(node_id, _)| allowed.contains(node_id))
        .map(|(node_id, value)| (*node_id, value.clone()))
        .collect()
}

pub fn find_owner_space_id(world: &specs::World, mut node: specs::Entity) -> Option<u32> {
    let entities = world.entities();
    let hier = world.read_storage::<Hierarchy>();
    let tags = world.read_storage::<Tag>();
    loop {
        if let Some(tag) = tags.get(node) {
            if tag.0 == "space" { return Some(node.id()); }
        }
        let parent_id = hier.get(node).and_then(|h| h.parent)?;
        node = entities.entity(parent_id);
    }
}

// ─── Systems ─────────────────────────────────────────────────────────────────

pub fn js_update_snapshots_system(world: &mut World) {
    let (
        attr_snap, tag_snap, positions, rotations, scales,
        global_positions, parents, children_map, space_subtrees,
    ) = {
        let Some(specs_world) = world.get_resource::<ElemenetWorld>() else { return; };
        let entities = specs_world.0.entities();
        let attrs_storage = specs_world.0.read_storage::<Attrs>();
        let tags_storage = specs_world.0.read_storage::<Tag>();
        let transforms_storage = specs_world.0.read_storage::<Transform2>();
        let hierarchies_storage = specs_world.0.read_storage::<Hierarchy>();

        let mut attr_snap = HashMap::new();
        for (ent, attrs) in (&entities, &attrs_storage).join() {
            let mut map = HashMap::new();
            for (k, v) in &attrs.0 { map.insert(k.clone(), v.clone()); }
            attr_snap.insert(ent.id() as i32, map);
        }

        let mut tag_snap = HashMap::new();
        for (ent, tag) in (&entities, &tags_storage).join() {
            tag_snap.insert(ent.id() as i32, tag.0.clone());
        }

        let mut positions = HashMap::new();
        let mut rotations = HashMap::new();
        let mut scales = HashMap::new();
        let mut global_positions = HashMap::new();
        for (ent, tr) in (&entities, &transforms_storage).join() {
            use js_runtime::Vec3;
            positions.insert(ent.id() as i32, Vec3 { x: tr.position.x, y: tr.position.y, z: tr.position.z });
            rotations.insert(ent.id() as i32, Vec3 { x: tr.rotation.x, y: tr.rotation.y, z: tr.rotation.z });
            scales.insert(ent.id() as i32, Vec3 { x: tr.scale.x, y: tr.scale.y, z: tr.scale.z });
            global_positions.insert(ent.id() as i32, Vec3 { x: tr.position.x, y: tr.position.y, z: tr.position.z });
        }

        let mut parents = HashMap::new();
        let mut children_map = HashMap::new();
        for (ent, hier) in (&entities, &hierarchies_storage).join() {
            parents.insert(ent.id() as i32, hier.parent.map(|p| p as i32).unwrap_or(-1));
            children_map.insert(ent.id() as i32, hier.children.iter().map(|c| c.id() as i32).collect::<Vec<_>>());
        }

        let mut space_subtrees: HashMap<u32, HashSet<i32>> = HashMap::new();
        for (node_id, tag_name) in &tag_snap {
            if tag_name != "space" { continue; }
            let mut set = HashSet::new();
            let mut stack = vec![*node_id];
            while let Some(curr) = stack.pop() {
                if !set.insert(curr) { continue; }
                if let Some(children) = children_map.get(&curr) {
                    for child in children { stack.push(*child); }
                }
            }
            space_subtrees.insert(*node_id as u32, set);
        }

        (attr_snap, tag_snap, positions, rotations, scales, global_positions, parents, children_map, space_subtrees)
    };

    let active_space_ids: HashSet<u32> = space_subtrees.keys().copied().collect();
    let mut removed_contexts = Vec::new();
    let mut created_contexts = Vec::new();
    let mut errors = Vec::new();

    {
        let Some(mut manager) = world.get_non_send_resource_mut::<ScriptRuntimeManager>() else { return; };

        let stale_ids: Vec<u32> = manager.contexts.keys().copied()
            .filter(|id| !active_space_ids.contains(id))
            .collect();
        for space_id in stale_ids {
            if let Some(mut worker) = manager.contexts.remove(&space_id) {
                stop_space_worker(&mut worker);
                removed_contexts.push(space_id);
            }
        }

        for space_id in &active_space_ids {
            if manager.contexts.contains_key(space_id) { continue; }
            match spawn_space_worker(*space_id) {
                Ok(worker) => { manager.contexts.insert(*space_id, worker); created_contexts.push(*space_id); }
                Err(err) => { errors.push(err); }
            }
        }

        let mut broken_contexts = Vec::new();
        for (space_id, worker) in manager.contexts.iter_mut() {
            let Some(allowed) = space_subtrees.get(space_id) else { continue; };

            let mut filtered_parents = HashMap::new();
            let mut filtered_children = HashMap::new();
            for node_id in allowed {
                let parent = parents.get(node_id).copied().unwrap_or(-1);
                let normalized_parent = if parent >= 0 && !allowed.contains(&parent) { -1 } else { parent };
                filtered_parents.insert(*node_id, normalized_parent);
                let children = children_map.get(node_id).cloned().unwrap_or_default()
                    .into_iter().filter(|c| allowed.contains(c)).collect();
                filtered_children.insert(*node_id, children);
            }

            let snap = SpaceSnapshots {
                attr_snap: filter_snapshot_map(&attr_snap, allowed),
                tag_snap: filter_snapshot_map(&tag_snap, allowed),
                positions: filter_snapshot_map(&positions, allowed),
                rotations: filter_snapshot_map(&rotations, allowed),
                scales: filter_snapshot_map(&scales, allowed),
                global_positions: filter_snapshot_map(&global_positions, allowed),
                parents: filtered_parents,
                children: filtered_children,
            };

            if let Err(e) = worker.cmd_tx.send(JsWorkerCommand::UpdateSnapshots(snap)) {
                errors.push(format!("failed to send snapshots to space {}: {}", space_id, e));
                broken_contexts.push(*space_id);
            }
        }

        for space_id in broken_contexts {
            if let Some(mut worker) = manager.contexts.remove(&space_id) {
                stop_space_worker(&mut worker);
                removed_contexts.push(space_id);
            }
        }
    }

    if !removed_contexts.is_empty() || !created_contexts.is_empty() || !errors.is_empty() {
        let Some(mut log_panel) = world.get_resource_mut::<LogPanel>() else { return; };
        for id in removed_contexts { log_panel.push_info(format!("[JS][space:{}] Context destroyed", id)); }
        for id in created_contexts { log_panel.push_info(format!("[JS][space:{}] Context created", id)); }
        for err in errors { log_panel.push_error(format!("[JS] {}", err)); }
    }
}

pub fn js_eval_pending_scripts(world: &mut World) {
    const MAX_SCRIPTS_PER_FRAME: usize = 2;
    const MAX_ENQUEUE_BUDGET_MS: f32 = 1.5;

    let pending_scripts = {
        let Some(mut pending) = world.get_resource_mut::<PendingScripts>() else { return; };
        pending.0.drain(..).collect::<Vec<_>>()
    };
    if pending_scripts.is_empty() { return; }

    let enqueue_start = Instant::now();
    let mut queued_this_frame = 0usize;
    let mut deferred_scripts: Vec<(u32, String, String)> = Vec::new();
    let mut log_messages = Vec::new();

    for (space_id, url, code) in pending_scripts {
        let elapsed_ms = enqueue_start.elapsed().as_secs_f32() * 1000.0;
        if queued_this_frame >= MAX_SCRIPTS_PER_FRAME || elapsed_ms >= MAX_ENQUEUE_BUDGET_MS {
            deferred_scripts.push((space_id, url, code));
            continue;
        }
        queued_this_frame += 1;

        let enqueue_result = {
            let Some(mut manager) = world.get_non_send_resource_mut::<ScriptRuntimeManager>() else { return; };
            if let Some(worker) = manager.contexts.get_mut(&space_id) {
                worker.cmd_tx.send(JsWorkerCommand::EvalScript { url: url.clone(), code })
                    .map_err(|e| e.to_string())
            } else {
                Err(format!("missing JS context for space {}", space_id))
            }
        };

        match enqueue_result {
            Ok(_) => log_messages.push(crate::LogEntry::new(LogLevel::Info, format!("[JS][space:{}] Script queued: {}", space_id, url))),
            Err(err) => log_messages.push(crate::LogEntry::new(LogLevel::Error, format!("[JS][space:{}] Error queuing {}: {}", space_id, url, err))),
        }
    }

    if !deferred_scripts.is_empty() {
        let deferred_count = deferred_scripts.len();
        if let Some(mut pending) = world.get_resource_mut::<PendingScripts>() {
            pending.0.extend(deferred_scripts);
        }
        log_messages.push(crate::LogEntry::new(LogLevel::Info, format!("[JS] Throttle: {} script(s) deferred", deferred_count)));
    }

    if !log_messages.is_empty() {
        let Some(mut log_panel) = world.get_resource_mut::<LogPanel>() else { return; };
        for entry in log_messages {
            match entry.level {
                LogLevel::Error => log_panel.push_error(entry.message),
                LogLevel::Warn => log_panel.push_warn(entry.message),
                LogLevel::Info => log_panel.push_info(entry.message),
            }
        }
    }
}

pub fn js_tick_system(world: &mut World) {
    let elapsed_ms = {
        let Some(time) = world.get_resource::<Time>() else { return; };
        time.elapsed_seconds_f64() * 1000.0
    };

    let mut eval_events: Vec<(u32, String, bool, Option<String>)> = Vec::new();
    let mut tick_batches: Vec<(u32, JsTickData)> = Vec::new();
    let mut worker_errors: Vec<String> = Vec::new();

    {
        let Some(mut manager) = world.get_non_send_resource_mut::<ScriptRuntimeManager>() else { return; };

        for worker in manager.contexts.values_mut() {
            let _ = worker.cmd_tx.send(JsWorkerCommand::Tick { elapsed_ms });
        }

        let mut broken_contexts = Vec::new();
        for (space_id, worker) in manager.contexts.iter_mut() {
            loop {
                match worker.event_rx.try_recv() {
                    Ok(JsWorkerEvent::EvalResult { url, already_loaded, error }) => {
                        eval_events.push((*space_id, url, already_loaded, error));
                    }
                    Ok(JsWorkerEvent::TickData(data)) => { tick_batches.push((*space_id, data)); }
                    Ok(JsWorkerEvent::WorkerError(err)) => {
                        worker_errors.push(format!("[JS][space:{}] {}", space_id, err));
                    }
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        broken_contexts.push(*space_id);
                        break;
                    }
                }
            }
        }

        for space_id in broken_contexts {
            if let Some(mut worker) = manager.contexts.remove(&space_id) {
                stop_space_worker(&mut worker);
                worker_errors.push(format!("[JS][space:{}] worker disconnected", space_id));
            }
        }
    }

    if !eval_events.is_empty() || !worker_errors.is_empty() {
        let Some(mut log_panel) = world.get_resource_mut::<LogPanel>() else { return; };
        for (space_id, url, already_loaded, error) in eval_events {
            if already_loaded {
                log_panel.push_info(format!("[JS][space:{}] Script already loaded, skipping: {}", space_id, url));
            } else if let Some(err) = error {
                log_panel.push_error(format!("[JS][space:{}] Error evaluating {}: {}", space_id, url, err));
            } else {
                log_panel.push_info(format!("[JS][space:{}] Script evaluated OK: {}", space_id, url));
            }
        }
        for err in worker_errors { log_panel.push_error(err); }
    }

    let mut logs_by_context = Vec::new();
    let mut attr_updates = Vec::new();
    let mut pos_updates = Vec::new();
    let mut rot_updates = Vec::new();
    let mut scale_updates = Vec::new();
    let mut creation_batches = Vec::new();
    let mut hierarchy_batches = Vec::new();
    let mut remove_batches = Vec::new();
    let mut fetch_batches = Vec::new();
    let mut navigate_batches = Vec::new();

    for (space_id, data) in tick_batches {
        if !data.logs.is_empty() { logs_by_context.push((space_id, data.logs)); }
        if !data.creation_queue.is_empty() { creation_batches.push((space_id, data.creation_queue)); }
        if !data.hierarchy_queue.is_empty() { hierarchy_batches.push((space_id, data.hierarchy_queue)); }
        if !data.remove_queue.is_empty() { remove_batches.push((space_id, data.remove_queue)); }
        if !data.fetch_queue.is_empty() { fetch_batches.push((space_id, data.fetch_queue)); }
        if !data.navigate_queue.is_empty() { navigate_batches.push((space_id, data.navigate_queue)); }
        attr_updates.extend(data.attr_updates);
        pos_updates.extend(data.pos_updates);
        rot_updates.extend(data.rot_updates);
        scale_updates.extend(data.scale_updates);
    }

    {
        let Some(mut log_panel) = world.get_resource_mut::<LogPanel>() else { return; };
        for (space_id, logs) in logs_by_context {
            for (level, msg) in logs {
                match level.as_str() {
                    "warn" => log_panel.push_warn(format!("[JS][space:{}] {}", space_id, msg)),
                    "error" => log_panel.push_error(format!("[JS][space:{}] {}", space_id, msg)),
                    _ => log_panel.push_info(format!("[JS][space:{}] {}", space_id, msg)),
                }
            }
        }
    }

    {
        let Some(mut attribute_updates) = world.get_resource_mut::<AttributeUpdates>() else { return; };
        for (node_id, key, value) in attr_updates {
            attribute_updates.0.push((node_id as u32, key, value));
        }
        for (node_id, pos) in pos_updates {
            attribute_updates.0.push((node_id as u32, "x".to_string(), pos.x.to_string()));
            attribute_updates.0.push((node_id as u32, "y".to_string(), pos.y.to_string()));
            attribute_updates.0.push((node_id as u32, "z".to_string(), pos.z.to_string()));
        }
        for (node_id, rot) in rot_updates {
            attribute_updates.0.push((node_id as u32, "rx".to_string(), rot.x.to_string()));
            attribute_updates.0.push((node_id as u32, "ry".to_string(), rot.y.to_string()));
            attribute_updates.0.push((node_id as u32, "rz".to_string(), rot.z.to_string()));
        }
        for (node_id, scale) in scale_updates {
            attribute_updates.0.push((node_id as u32, "sx".to_string(), scale.x.to_string()));
            attribute_updates.0.push((node_id as u32, "sy".to_string(), scale.y.to_string()));
            attribute_updates.0.push((node_id as u32, "sz".to_string(), scale.z.to_string()));
        }
    }

    // Element creation
    for (space_id, creation_queue) in creation_batches {
        use virtual_dom::dom::element::Vec3 as DomVec3;
        let (creation_results, created_entities, log_messages) = {
            let Some(mut specs_world) = world.get_resource_mut::<ElemenetWorld>() else { return; };
            let mut creation_results = Vec::new();
            let mut created_entities: Vec<(u32, specs::Entity)> = Vec::new();
            let mut log_messages = Vec::new();

            for (request_id, tag_name) in creation_queue {
                let new_ent = { specs_world.0.entities().create() };
                let new_ent_id = {
                    let mut tags_storage = specs_world.0.write_storage::<Tag>();
                    let mut attrs_storage = specs_world.0.write_storage::<Attrs>();
                    let mut transform_storage = specs_world.0.write_storage::<Transform2>();
                    let mut hier_storage = specs_world.0.write_storage::<Hierarchy>();
                    tags_storage.insert(new_ent, Tag(tag_name.clone())).ok();
                    attrs_storage.insert(new_ent, Attrs(std::collections::HashMap::new())).ok();
                    transform_storage.insert(new_ent, Transform2 {
                        position: DomVec3 { x: 0.0, y: 0.0, z: 0.0 },
                        rotation: DomVec3 { x: 0.0, y: 0.0, z: 0.0 },
                        scale: DomVec3 { x: 1.0, y: 1.0, z: 1.0 },
                    }).ok();
                    hier_storage.insert(new_ent, Hierarchy { parent: None, children: Vec::new() }).ok();
                    new_ent.id()
                };
                creation_results.push((request_id, new_ent_id as i32));
                created_entities.push((new_ent_id, new_ent));
                log_messages.push(format!("[JS][space:{}] createElement('{}') -> node_id={}", space_id, tag_name, new_ent_id));
            }
            (creation_results, created_entities, log_messages)
        };

        if let Some(mut dom_data) = world.get_resource_mut::<crate::VirtualDomData>() {
            for &(id, ent) in &created_entities {
                dom_data.nodes.insert(id, ent);
            }
        }
        if let Some(mut dirty_nodes) = world.get_resource_mut::<DirtyNodes>() {
            dirty_nodes.0.extend(created_entities.iter().map(|&(id, _)| id));
        }
        if let Some(mut log_panel) = world.get_resource_mut::<LogPanel>() {
            for msg in log_messages { log_panel.push_info(msg); }
        }
        if let Some(mut manager) = world.get_non_send_resource_mut::<ScriptRuntimeManager>() {
            if let Some(worker) = manager.contexts.get_mut(&space_id) {
                let _ = worker.cmd_tx.send(JsWorkerCommand::PushElementCreationResults(creation_results));
            }
        }
    }

    // Hierarchy append
    for (space_id, hierarchy_queue) in hierarchy_batches {
        let (dirty_child_ids, log_messages) = {
            let Some(mut specs_world) = world.get_resource_mut::<ElemenetWorld>() else { return; };
            let mut dirty_child_ids = Vec::new();
            let mut log_messages = Vec::new();
            for (parent_id, child_id) in hierarchy_queue {
                let (parent_ent, child_ent, are_alive) = {
                    let entities = specs_world.0.entities();
                    let parent_ent = entities.entity(parent_id as u32);
                    let child_ent = entities.entity(child_id as u32);
                    let are_alive = entities.is_alive(parent_ent) && entities.is_alive(child_ent);
                    (parent_ent, child_ent, are_alive)
                };
                if are_alive {
                    Hierarchy::add_child(&mut specs_world.0, parent_ent, child_ent);
                    dirty_child_ids.push(child_id as u32);
                    log_messages.push(format!("[JS][space:{}] appendChild: parent={} child={}", space_id, parent_id, child_id));
                }
            }
            (dirty_child_ids, log_messages)
        };
        if let Some(mut dirty_nodes) = world.get_resource_mut::<DirtyNodes>() {
            dirty_nodes.0.extend(dirty_child_ids);
        }
        if let Some(mut log_panel) = world.get_resource_mut::<LogPanel>() {
            for msg in log_messages { log_panel.push_info(msg); }
        }
    }

    // Remove elements
    for (space_id, remove_queue) in remove_batches {
        let log_messages = {
            let Some(mut specs_world) = world.get_resource_mut::<ElemenetWorld>() else { return; };
            let mut log_messages = Vec::new();
            for node_id in remove_queue {
                let (ent, is_alive) = {
                    let entities = specs_world.0.entities();
                    let ent = entities.entity(node_id as u32);
                    (ent, entities.is_alive(ent))
                };
                if is_alive {
                    specs_world.0.delete_entity(ent).ok();
                    log_messages.push(format!("[JS][space:{}] remove: node_id={}", space_id, node_id));
                }
            }
            log_messages
        };
        if let Some(mut log_panel) = world.get_resource_mut::<LogPanel>() {
            for msg in log_messages { log_panel.push_info(msg); }
        }
    }

    // Fetch
    for (space_id, fetch_queue) in fetch_batches {
        let fetch_results = {
            let Some(tokio_rt) = world.get_resource::<crate::TokioRuntime>() else { return; };
            let mut fetch_results = Vec::new();
            for (request_id, url) in &fetch_queue {
                let result = tokio_rt.0.block_on(async {
                    match reqwest::get(url).await {
                        Ok(resp) => match resp.text().await {
                            Ok(text) => Ok(text),
                            Err(e) => Err(format!("Failed to read response: {}", e)),
                        },
                        Err(e) => Err(format!("HTTP error: {}", e)),
                    }
                });
                fetch_results.push((*request_id, result));
            }
            fetch_results
        };
        if let Some(mut log_panel) = world.get_resource_mut::<LogPanel>() {
            for (_, url) in &fetch_queue {
                log_panel.push_info(format!("[JS][space:{}] fetch: {}", space_id, url));
            }
        }
        if let Some(mut manager) = world.get_non_send_resource_mut::<ScriptRuntimeManager>() {
            if let Some(worker) = manager.contexts.get_mut(&space_id) {
                let _ = worker.cmd_tx.send(JsWorkerCommand::PushFetchResults(fetch_results));
            }
        }
    }

    // Navigate
    if !navigate_batches.is_empty() {
        if let Some(mut log_panel) = world.get_resource_mut::<LogPanel>() {
            for (space_id, urls) in &navigate_batches {
                for url in urls {
                    log_panel.push_warn(format!("[JS][space:{}] navigate: {}", space_id, url));
                }
            }
        }
        if let Some(mut current_url) = world.get_resource_mut::<crate::CurrentUrl>() {
            if let Some((_, urls)) = navigate_batches.first() {
                if let Some(url) = urls.first() {
                    current_url.0 = url.clone();
                }
            }
        }
        if let Some(mut reload_trigger) = world.get_resource_mut::<ReloadTrigger>() {
            reload_trigger.0 = true;
        }
    }
}
