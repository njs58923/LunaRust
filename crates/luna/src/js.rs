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
    request_fetch_text, AttributeUpdates, DirtyNodes, ElemenetWorld, IoService, LogLevel, LogPanel,
    ModelLoadStates, PendingModelLoads, PendingScripts, ReloadTrigger, ScriptLoadStates,
    SpaceHandleTable, SpaceHandleTables,
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
    PushTouchEvents(Vec<(i32, f32, f32, f32)>),
    Shutdown,
}

pub enum JsWorkerEvent {
    EvalResult {
        url: String,
        already_loaded: bool,
        error: Option<String>,
    },
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

    world
        .resource_mut::<LogPanel>()
        .push_info("[JS] Runtime initialization complete");
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

    Ok(SpaceScriptContext {
        engine,
        loaded_scripts: HashSet::new(),
    })
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
                            snap.positions,
                            snap.rotations,
                            snap.scales,
                            snap.global_positions,
                        );
                        ctx.engine
                            .update_hierarchy_snapshot(snap.parents, snap.children);
                    }
                    JsWorkerCommand::EvalScript { url, code } => {
                        if ctx.loaded_scripts.contains(&url) {
                            let _ = event_tx.send(JsWorkerEvent::EvalResult {
                                url,
                                already_loaded: true,
                                error: None,
                            });
                            continue;
                        }
                        let wrapped_code = format!("(function(){{\n{}\n}})();", code);
                        match ctx.engine.eval(&wrapped_code) {
                            Ok(_) => {
                                ctx.loaded_scripts.insert(url.clone());
                                let _ = event_tx.send(JsWorkerEvent::EvalResult {
                                    url,
                                    already_loaded: false,
                                    error: None,
                                });
                            }
                            Err(e) => {
                                let _ = event_tx.send(JsWorkerEvent::EvalResult {
                                    url,
                                    already_loaded: false,
                                    error: Some(e.to_string()),
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
                    JsWorkerCommand::PushTouchEvents(events) => {
                        for (node_id, x, y, z) in events {
                            ctx.engine.push_touch_event(node_id, x, y, z);
                        }
                    }
                    JsWorkerCommand::Shutdown => break,
                }
            }
        })
        .map_err(|e| format!("failed to spawn JS worker for space {}: {}", space_id, e))?;

    Ok(SpaceScriptWorker {
        cmd_tx,
        event_rx,
        join: Some(join),
    })
}

pub fn stop_space_worker(worker: &mut SpaceScriptWorker) {
    let _ = worker.cmd_tx.send(JsWorkerCommand::Shutdown);
    if let Some(join) = worker.join.take() {
        let _ = join.join();
    }
}

fn filter_snapshot_map<T: Clone>(
    source: &HashMap<i32, T>,
    allowed: &HashSet<i32>,
) -> HashMap<i32, T> {
    source
        .iter()
        .filter(|(node_id, _)| allowed.contains(node_id))
        .map(|(node_id, value)| (*node_id, value.clone()))
        .collect()
}

fn next_runtime_id(space_handle_tables: &mut SpaceHandleTables) -> u64 {
    space_handle_tables.next_runtime_id += 1;
    space_handle_tables.next_runtime_id
}

fn reset_space_handle_table(space_handle_tables: &mut SpaceHandleTables, space_id: u32) {
    let runtime_id = next_runtime_id(space_handle_tables);
    let mut table = SpaceHandleTable {
        runtime_id,
        next_local_id: 1,
        ..Default::default()
    };
    table.local_to_global.insert(0, space_id);
    table.global_to_local.insert(space_id, 0);
    space_handle_tables.by_space.insert(space_id, table);
}

fn ensure_space_handle_table<'a>(
    space_handle_tables: &'a mut SpaceHandleTables,
    space_id: u32,
) -> &'a mut SpaceHandleTable {
    if !space_handle_tables.by_space.contains_key(&space_id) {
        reset_space_handle_table(space_handle_tables, space_id);
    }
    space_handle_tables
        .by_space
        .get_mut(&space_id)
        .expect("space handle table must exist")
}

fn ensure_local_id(table: &mut SpaceHandleTable, global_id: u32) -> i32 {
    if let Some(local_id) = table.global_to_local.get(&global_id).copied() {
        return local_id;
    }
    let local_id = table.next_local_id;
    table.next_local_id += 1;
    table.global_to_local.insert(global_id, local_id);
    table.local_to_global.insert(local_id, global_id);
    local_id
}

fn resolve_global_id(
    space_handle_tables: &SpaceHandleTables,
    space_id: u32,
    local_id: i32,
) -> Option<u32> {
    space_handle_tables
        .by_space
        .get(&space_id)
        .and_then(|table| table.local_to_global.get(&local_id).copied())
}

fn sync_space_handle_table(table: &mut SpaceHandleTable, space_id: u32, allowed: &HashSet<i32>) {
    table.global_to_local.entry(space_id).or_insert(0);
    table.local_to_global.entry(0).or_insert(space_id);
    table.detached_globals.remove(&space_id);

    let allowed_globals: HashSet<u32> = allowed.iter().map(|node_id| *node_id as u32).collect();
    let retained_globals: HashSet<u32> = allowed_globals
        .union(&table.detached_globals)
        .copied()
        .collect();

    table
        .global_to_local
        .retain(|global_id, _| retained_globals.contains(global_id));
    table
        .local_to_global
        .retain(|_, global_id| retained_globals.contains(global_id));

    for &global_id in &allowed_globals {
        ensure_local_id(table, global_id);
        table.detached_globals.remove(&global_id);
    }
}

fn build_local_space_snapshot(
    space_id: u32,
    allowed: &HashSet<i32>,
    table: &mut SpaceHandleTable,
    attr_snap: &HashMap<i32, HashMap<String, String>>,
    tag_snap: &HashMap<i32, String>,
    positions: &HashMap<i32, js_runtime::Vec3>,
    rotations: &HashMap<i32, js_runtime::Vec3>,
    scales: &HashMap<i32, js_runtime::Vec3>,
    global_positions: &HashMap<i32, js_runtime::Vec3>,
    parents: &HashMap<i32, i32>,
    children_map: &HashMap<i32, Vec<i32>>,
) -> SpaceSnapshots {
    sync_space_handle_table(table, space_id, allowed);

    let mut local_attr_snap = HashMap::new();
    let mut local_tag_snap = HashMap::new();
    let mut local_positions = HashMap::new();
    let mut local_rotations = HashMap::new();
    let mut local_scales = HashMap::new();
    let mut local_global_positions = HashMap::new();
    let mut local_parents = HashMap::new();
    let mut local_children = HashMap::new();

    for &global_node_id_i32 in allowed {
        let global_node_id = global_node_id_i32 as u32;
        let local_id = ensure_local_id(table, global_node_id);

        if let Some(attrs) = attr_snap.get(&global_node_id_i32) {
            local_attr_snap.insert(local_id, attrs.clone());
        }
        if let Some(tag) = tag_snap.get(&global_node_id_i32) {
            local_tag_snap.insert(local_id, tag.clone());
        }
        if let Some(pos) = positions.get(&global_node_id_i32) {
            local_positions.insert(local_id, pos.clone());
        }
        if let Some(rot) = rotations.get(&global_node_id_i32) {
            local_rotations.insert(local_id, rot.clone());
        }
        if let Some(scale) = scales.get(&global_node_id_i32) {
            local_scales.insert(local_id, scale.clone());
        }
        if let Some(global_pos) = global_positions.get(&global_node_id_i32) {
            local_global_positions.insert(local_id, global_pos.clone());
        }

        let parent_local = parents
            .get(&global_node_id_i32)
            .copied()
            .and_then(|parent_id| {
                if parent_id < 0 {
                    Some(-1)
                } else {
                    table.global_to_local.get(&(parent_id as u32)).copied()
                }
            })
            .unwrap_or(-1);
        local_parents.insert(local_id, parent_local);

        let children = children_map
            .get(&global_node_id_i32)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|child_id| table.global_to_local.get(&(child_id as u32)).copied())
            .collect::<Vec<_>>();
        local_children.insert(local_id, children);
    }

    SpaceSnapshots {
        attr_snap: local_attr_snap,
        tag_snap: local_tag_snap,
        positions: local_positions,
        rotations: local_rotations,
        scales: local_scales,
        global_positions: local_global_positions,
        parents: local_parents,
        children: local_children,
    }
}

pub fn find_owner_space_id(world: &specs::World, mut node: specs::Entity) -> Option<u32> {
    let entities = world.entities();
    let hier = world.read_storage::<Hierarchy>();
    let tags = world.read_storage::<Tag>();
    loop {
        if let Some(tag) = tags.get(node) {
            if tag.0 == "space" {
                return Some(node.id());
            }
        }
        let parent_id = hier.get(node).and_then(|h| h.parent)?;
        node = entities.entity(parent_id);
    }
}

// ─── Systems ─────────────────────────────────────────────────────────────────

pub fn js_update_snapshots_system(world: &mut World) {
    let (
        attr_snap,
        tag_snap,
        positions,
        rotations,
        scales,
        global_positions,
        parents,
        children_map,
        space_subtrees,
    ) = {
        let Some(specs_world) = world.get_resource::<ElemenetWorld>() else {
            return;
        };
        let entities = specs_world.0.entities();
        let attrs_storage = specs_world.0.read_storage::<Attrs>();
        let tags_storage = specs_world.0.read_storage::<Tag>();
        let transforms_storage = specs_world.0.read_storage::<Transform2>();
        let hierarchies_storage = specs_world.0.read_storage::<Hierarchy>();

        let mut attr_snap = HashMap::new();
        for (ent, attrs) in (&entities, &attrs_storage).join() {
            let mut map = HashMap::new();
            for (k, v) in &attrs.0 {
                map.insert(k.clone(), v.clone());
            }
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
            positions.insert(
                ent.id() as i32,
                Vec3 {
                    x: tr.position.x,
                    y: tr.position.y,
                    z: tr.position.z,
                },
            );
            rotations.insert(
                ent.id() as i32,
                Vec3 {
                    x: tr.rotation.x,
                    y: tr.rotation.y,
                    z: tr.rotation.z,
                },
            );
            scales.insert(
                ent.id() as i32,
                Vec3 {
                    x: tr.scale.x,
                    y: tr.scale.y,
                    z: tr.scale.z,
                },
            );
            global_positions.insert(
                ent.id() as i32,
                Vec3 {
                    x: tr.position.x,
                    y: tr.position.y,
                    z: tr.position.z,
                },
            );
        }

        let mut parents = HashMap::new();
        let mut children_map = HashMap::new();
        for (ent, hier) in (&entities, &hierarchies_storage).join() {
            parents.insert(ent.id() as i32, hier.parent.map(|p| p as i32).unwrap_or(-1));
            children_map.insert(
                ent.id() as i32,
                hier.children
                    .iter()
                    .map(|c| c.id() as i32)
                    .collect::<Vec<_>>(),
            );
        }

        let mut space_subtrees: HashMap<u32, HashSet<i32>> = HashMap::new();
        for (node_id, tag_name) in &tag_snap {
            if tag_name != "space" {
                continue;
            }
            let mut set = HashSet::new();
            let mut stack = vec![*node_id];
            while let Some(curr) = stack.pop() {
                if !set.insert(curr) {
                    continue;
                }
                if let Some(children) = children_map.get(&curr) {
                    for child in children {
                        stack.push(*child);
                    }
                }
            }
            space_subtrees.insert(*node_id as u32, set);
        }

        (
            attr_snap,
            tag_snap,
            positions,
            rotations,
            scales,
            global_positions,
            parents,
            children_map,
            space_subtrees,
        )
    };

    let active_space_ids: HashSet<u32> = space_subtrees.keys().copied().collect();
    let mut removed_contexts = Vec::new();
    let mut created_contexts = Vec::new();
    let mut errors = Vec::new();

    {
        let Some(mut manager) = world.get_non_send_resource_mut::<ScriptRuntimeManager>() else {
            return;
        };

        let stale_ids: Vec<u32> = manager
            .contexts
            .keys()
            .copied()
            .filter(|id| !active_space_ids.contains(id))
            .collect();
        for space_id in stale_ids {
            if let Some(mut worker) = manager.contexts.remove(&space_id) {
                stop_space_worker(&mut worker);
                removed_contexts.push(space_id);
            }
        }

        for space_id in &active_space_ids {
            if manager.contexts.contains_key(space_id) {
                continue;
            }
            match spawn_space_worker(*space_id) {
                Ok(worker) => {
                    manager.contexts.insert(*space_id, worker);
                    created_contexts.push(*space_id);
                }
                Err(err) => {
                    errors.push(err);
                }
            }
        }
    }

    if !removed_contexts.is_empty() || !created_contexts.is_empty() {
        let Some(mut space_handle_tables) = world.get_resource_mut::<SpaceHandleTables>() else {
            return;
        };
        for &space_id in &removed_contexts {
            space_handle_tables.by_space.remove(&space_id);
        }
        for &space_id in &created_contexts {
            reset_space_handle_table(&mut space_handle_tables, space_id);
        }
    }

    let snapshot_batches = {
        let Some(mut space_handle_tables) = world.get_resource_mut::<SpaceHandleTables>() else {
            return;
        };
        let mut snapshot_batches = Vec::new();
        for space_id in &active_space_ids {
            let Some(allowed) = space_subtrees.get(space_id) else {
                continue;
            };
            let table = ensure_space_handle_table(&mut space_handle_tables, *space_id);
            let snap = build_local_space_snapshot(
                *space_id,
                allowed,
                table,
                &attr_snap,
                &tag_snap,
                &positions,
                &rotations,
                &scales,
                &global_positions,
                &parents,
                &children_map,
            );
            snapshot_batches.push((*space_id, snap));
        }
        snapshot_batches
    };

    {
        let Some(mut manager) = world.get_non_send_resource_mut::<ScriptRuntimeManager>() else {
            return;
        };
        let mut broken_contexts = Vec::new();
        for (space_id, snap) in snapshot_batches {
            let Some(worker) = manager.contexts.get_mut(&space_id) else {
                continue;
            };
            if let Err(e) = worker.cmd_tx.send(JsWorkerCommand::UpdateSnapshots(snap)) {
                errors.push(format!(
                    "failed to send snapshots to space {}: {}",
                    space_id, e
                ));
                broken_contexts.push(space_id);
            }
        }

        for space_id in broken_contexts {
            if let Some(mut worker) = manager.contexts.remove(&space_id) {
                stop_space_worker(&mut worker);
                removed_contexts.push(space_id);
            }
        }
    }

    if !removed_contexts.is_empty() {
        let Some(mut space_handle_tables) = world.get_resource_mut::<SpaceHandleTables>() else {
            return;
        };
        for &space_id in &removed_contexts {
            space_handle_tables.by_space.remove(&space_id);
        }
    }

    if !removed_contexts.is_empty() || !created_contexts.is_empty() || !errors.is_empty() {
        let Some(mut log_panel) = world.get_resource_mut::<LogPanel>() else {
            return;
        };
        for id in removed_contexts {
            log_panel.push_info(format!("[JS][space:{}] Context destroyed", id));
        }
        for id in created_contexts {
            log_panel.push_info(format!("[JS][space:{}] Context created", id));
        }
        for err in errors {
            log_panel.push_error(format!("[JS] {}", err));
        }
    }
}

pub fn js_eval_pending_scripts(world: &mut World) {
    const MAX_SCRIPTS_PER_FRAME: usize = 2;
    const MAX_ENQUEUE_BUDGET_MS: f32 = 1.5;

    let pending_scripts = {
        let Some(mut pending) = world.get_resource_mut::<PendingScripts>() else {
            return;
        };
        pending.0.drain(..).collect::<Vec<_>>()
    };
    if pending_scripts.is_empty() {
        return;
    }

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
            let Some(mut manager) = world.get_non_send_resource_mut::<ScriptRuntimeManager>()
            else {
                return;
            };
            if let Some(worker) = manager.contexts.get_mut(&space_id) {
                worker
                    .cmd_tx
                    .send(JsWorkerCommand::EvalScript {
                        url: url.clone(),
                        code,
                    })
                    .map_err(|e| e.to_string())
            } else {
                Err(format!("missing JS context for space {}", space_id))
            }
        };

        match enqueue_result {
            Ok(_) => log_messages.push(crate::LogEntry::new(
                LogLevel::Info,
                format!("[JS][space:{}] Script queued: {}", space_id, url),
            )),
            Err(err) => log_messages.push(crate::LogEntry::new(
                LogLevel::Error,
                format!("[JS][space:{}] Error queuing {}: {}", space_id, url, err),
            )),
        }
    }

    if !deferred_scripts.is_empty() {
        let deferred_count = deferred_scripts.len();
        if let Some(mut pending) = world.get_resource_mut::<PendingScripts>() {
            pending.0.extend(deferred_scripts);
        }
        log_messages.push(crate::LogEntry::new(
            LogLevel::Info,
            format!("[JS] Throttle: {} script(s) deferred", deferred_count),
        ));
    }

    if !log_messages.is_empty() {
        let Some(mut log_panel) = world.get_resource_mut::<LogPanel>() else {
            return;
        };
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
        let Some(time) = world.get_resource::<Time>() else {
            return;
        };
        time.elapsed_seconds_f64() * 1000.0
    };

    let mut eval_events: Vec<(u32, String, bool, Option<String>)> = Vec::new();
    let mut tick_batches: Vec<(u32, JsTickData)> = Vec::new();
    let mut worker_errors: Vec<String> = Vec::new();

    {
        let Some(mut manager) = world.get_non_send_resource_mut::<ScriptRuntimeManager>() else {
            return;
        };

        for worker in manager.contexts.values_mut() {
            let _ = worker.cmd_tx.send(JsWorkerCommand::Tick { elapsed_ms });
        }

        let mut broken_contexts = Vec::new();
        for (space_id, worker) in manager.contexts.iter_mut() {
            loop {
                match worker.event_rx.try_recv() {
                    Ok(JsWorkerEvent::EvalResult {
                        url,
                        already_loaded,
                        error,
                    }) => {
                        eval_events.push((*space_id, url, already_loaded, error));
                    }
                    Ok(JsWorkerEvent::TickData(data)) => {
                        tick_batches.push((*space_id, data));
                    }
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
        let Some(mut log_panel) = world.get_resource_mut::<LogPanel>() else {
            return;
        };
        for (space_id, url, already_loaded, error) in eval_events {
            if already_loaded {
                log_panel.push_info(format!(
                    "[JS][space:{}] Script already loaded, skipping: {}",
                    space_id, url
                ));
            } else if let Some(err) = error {
                log_panel.push_error(format!(
                    "[JS][space:{}] Error evaluating {}: {}",
                    space_id, url, err
                ));
            } else {
                log_panel.push_info(format!(
                    "[JS][space:{}] Script evaluated OK: {}",
                    space_id, url
                ));
            }
        }
        for err in worker_errors {
            log_panel.push_error(err);
        }
    }

    let mut logs_by_context = Vec::new();
    let mut attr_update_batches = Vec::new();
    let mut pos_update_batches = Vec::new();
    let mut rot_update_batches = Vec::new();
    let mut scale_update_batches = Vec::new();
    let mut creation_batches = Vec::new();
    let mut hierarchy_batches = Vec::new();
    let mut remove_batches = Vec::new();
    let mut fetch_batches = Vec::new();
    let mut navigate_batches = Vec::new();

    for (space_id, data) in tick_batches {
        if !data.logs.is_empty() {
            logs_by_context.push((space_id, data.logs));
        }
        if !data.creation_queue.is_empty() {
            creation_batches.push((space_id, data.creation_queue));
        }
        if !data.hierarchy_queue.is_empty() {
            hierarchy_batches.push((space_id, data.hierarchy_queue));
        }
        if !data.remove_queue.is_empty() {
            remove_batches.push((space_id, data.remove_queue));
        }
        if !data.fetch_queue.is_empty() {
            fetch_batches.push((space_id, data.fetch_queue));
        }
        if !data.navigate_queue.is_empty() {
            navigate_batches.push((space_id, data.navigate_queue));
        }
        if !data.attr_updates.is_empty() {
            attr_update_batches.push((space_id, data.attr_updates));
        }
        if !data.pos_updates.is_empty() {
            pos_update_batches.push((space_id, data.pos_updates));
        }
        if !data.rot_updates.is_empty() {
            rot_update_batches.push((space_id, data.rot_updates));
        }
        if !data.scale_updates.is_empty() {
            scale_update_batches.push((space_id, data.scale_updates));
        }
    }

    {
        let Some(mut log_panel) = world.get_resource_mut::<LogPanel>() else {
            return;
        };
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

    let mut ownership_logs = Vec::new();
    let validated_attribute_updates = {
        let Some(space_handle_tables) = world.get_resource::<SpaceHandleTables>() else {
            return;
        };
        let mut validated = Vec::new();

        for (space_id, updates) in attr_update_batches {
            for (local_id, key, value) in updates {
                if let Some(global_id) = resolve_global_id(&space_handle_tables, space_id, local_id)
                {
                    validated.push((global_id, key, value));
                } else {
                    ownership_logs.push(format!(
                        "[JS][space:{space_id}] Blocked invalid local attribute write: local_id={local_id}, key={key}"
                    ));
                }
            }
        }

        for (space_id, updates) in pos_update_batches {
            for (local_id, pos) in updates {
                if let Some(global_id) = resolve_global_id(&space_handle_tables, space_id, local_id)
                {
                    validated.push((global_id, "x".to_string(), pos.x.to_string()));
                    validated.push((global_id, "y".to_string(), pos.y.to_string()));
                    validated.push((global_id, "z".to_string(), pos.z.to_string()));
                } else {
                    ownership_logs.push(format!(
                        "[JS][space:{space_id}] Blocked invalid local position write: local_id={local_id}"
                    ));
                }
            }
        }

        for (space_id, updates) in rot_update_batches {
            for (local_id, rot) in updates {
                if let Some(global_id) = resolve_global_id(&space_handle_tables, space_id, local_id)
                {
                    validated.push((global_id, "rx".to_string(), rot.x.to_string()));
                    validated.push((global_id, "ry".to_string(), rot.y.to_string()));
                    validated.push((global_id, "rz".to_string(), rot.z.to_string()));
                } else {
                    ownership_logs.push(format!(
                        "[JS][space:{space_id}] Blocked invalid local rotation write: local_id={local_id}"
                    ));
                }
            }
        }

        for (space_id, updates) in scale_update_batches {
            for (local_id, scale) in updates {
                if let Some(global_id) = resolve_global_id(&space_handle_tables, space_id, local_id)
                {
                    validated.push((global_id, "sx".to_string(), scale.x.to_string()));
                    validated.push((global_id, "sy".to_string(), scale.y.to_string()));
                    validated.push((global_id, "sz".to_string(), scale.z.to_string()));
                } else {
                    ownership_logs.push(format!(
                        "[JS][space:{space_id}] Blocked invalid local scale write: local_id={local_id}"
                    ));
                }
            }
        }

        validated
    };

    {
        let Some(mut attribute_updates) = world.get_resource_mut::<AttributeUpdates>() else {
            return;
        };
        attribute_updates.0.extend(validated_attribute_updates);
    }

    if !ownership_logs.is_empty() {
        let Some(mut log_panel) = world.get_resource_mut::<LogPanel>() else {
            return;
        };
        for msg in ownership_logs.drain(..) {
            log_panel.push_warn(msg);
        }
    }

    // Element creation
    for (space_id, creation_queue) in creation_batches {
        use virtual_dom::dom::element::Vec3 as DomVec3;
        let (created_nodes, log_messages) = {
            let Some(mut specs_world) = world.get_resource_mut::<ElemenetWorld>() else {
                return;
            };
            let mut created_nodes: Vec<(i32, u32, specs::Entity)> = Vec::new();
            let mut log_messages = Vec::new();

            for (request_id, tag_name) in creation_queue {
                let new_ent = { specs_world.0.entities().create() };
                let new_ent_id = {
                    let mut tags_storage = specs_world.0.write_storage::<Tag>();
                    let mut attrs_storage = specs_world.0.write_storage::<Attrs>();
                    let mut transform_storage = specs_world.0.write_storage::<Transform2>();
                    let mut hier_storage = specs_world.0.write_storage::<Hierarchy>();
                    tags_storage.insert(new_ent, Tag(tag_name.clone())).ok();
                    attrs_storage
                        .insert(new_ent, Attrs(std::collections::HashMap::new()))
                        .ok();
                    transform_storage
                        .insert(
                            new_ent,
                            Transform2 {
                                position: DomVec3 {
                                    x: 0.0,
                                    y: 0.0,
                                    z: 0.0,
                                },
                                rotation: DomVec3 {
                                    x: 0.0,
                                    y: 0.0,
                                    z: 0.0,
                                },
                                scale: DomVec3 {
                                    x: 1.0,
                                    y: 1.0,
                                    z: 1.0,
                                },
                            },
                        )
                        .ok();
                    hier_storage
                        .insert(
                            new_ent,
                            Hierarchy {
                                parent: None,
                                children: Vec::new(),
                            },
                        )
                        .ok();
                    new_ent.id()
                };
                created_nodes.push((request_id, new_ent_id, new_ent));
                log_messages.push(format!(
                    "[JS][space:{}] createElement('{}') -> node_id={}",
                    space_id, tag_name, new_ent_id
                ));
            }
            (created_nodes, log_messages)
        };

        let creation_results = {
            let Some(mut space_handle_tables) = world.get_resource_mut::<SpaceHandleTables>()
            else {
                return;
            };
            let table = ensure_space_handle_table(&mut space_handle_tables, space_id);
            let mut creation_results = Vec::new();
            for &(request_id, global_id, _) in &created_nodes {
                let local_id = ensure_local_id(table, global_id);
                table.detached_globals.insert(global_id);
                creation_results.push((request_id, local_id));
            }
            creation_results
        };

        if let Some(mut dom_data) = world.get_resource_mut::<crate::VirtualDomData>() {
            for &(_, id, ent) in &created_nodes {
                dom_data.nodes.insert(id, ent);
            }
        }
        if let Some(mut dirty_nodes) = world.get_resource_mut::<DirtyNodes>() {
            dirty_nodes
                .0
                .extend(created_nodes.iter().map(|&(_, id, _)| id));
        }
        if let Some(mut log_panel) = world.get_resource_mut::<LogPanel>() {
            for msg in log_messages {
                log_panel.push_info(msg);
            }
        }
        if let Some(mut manager) = world.get_non_send_resource_mut::<ScriptRuntimeManager>() {
            if let Some(worker) = manager.contexts.get_mut(&space_id) {
                let _ = worker
                    .cmd_tx
                    .send(JsWorkerCommand::PushElementCreationResults(
                        creation_results,
                    ));
            }
        }
    }

    // Hierarchy append
    for (space_id, hierarchy_queue) in hierarchy_batches {
        let (allowed_appends, rejected_logs) = {
            let Some(space_handle_tables) = world.get_resource::<SpaceHandleTables>() else {
                return;
            };
            let mut allowed_appends = Vec::new();
            let mut rejected_logs = Vec::new();
            for (parent_local_id, child_local_id) in hierarchy_queue {
                let Some(parent_id) =
                    resolve_global_id(&space_handle_tables, space_id, parent_local_id)
                else {
                    rejected_logs.push(format!(
                        "[JS][space:{space_id}] Blocked invalid local appendChild parent: local_id={parent_local_id}"
                    ));
                    continue;
                };
                let Some(child_id) =
                    resolve_global_id(&space_handle_tables, space_id, child_local_id)
                else {
                    rejected_logs.push(format!(
                        "[JS][space:{space_id}] Blocked invalid local appendChild child: local_id={child_local_id}"
                    ));
                    continue;
                };
                allowed_appends.push((parent_id, child_id));
            }
            (allowed_appends, rejected_logs)
        };

        let (dirty_child_ids, log_messages) = {
            let Some(mut specs_world) = world.get_resource_mut::<ElemenetWorld>() else {
                return;
            };
            let mut dirty_child_ids = Vec::new();
            let mut log_messages = Vec::new();
            for (parent_id, child_id) in allowed_appends {
                let (parent_ent, child_ent, are_alive) = {
                    let entities = specs_world.0.entities();
                    let parent_ent = entities.entity(parent_id);
                    let child_ent = entities.entity(child_id);
                    let are_alive = entities.is_alive(parent_ent) && entities.is_alive(child_ent);
                    (parent_ent, child_ent, are_alive)
                };
                if are_alive {
                    Hierarchy::add_child(&mut specs_world.0, parent_ent, child_ent);
                    dirty_child_ids.push(child_id);
                    log_messages.push(format!(
                        "[JS][space:{}] appendChild: parent={} child={}",
                        space_id, parent_id, child_id
                    ));
                }
            }
            (dirty_child_ids, log_messages)
        };

        if let Some(mut space_handle_tables) = world.get_resource_mut::<SpaceHandleTables>() {
            let Some(table) = space_handle_tables.by_space.get_mut(&space_id) else {
                return;
            };
            for child_id in &dirty_child_ids {
                table.detached_globals.remove(child_id);
            }
        }
        if let Some(mut dirty_nodes) = world.get_resource_mut::<DirtyNodes>() {
            dirty_nodes.0.extend(dirty_child_ids);
        }
        if let Some(mut log_panel) = world.get_resource_mut::<LogPanel>() {
            for msg in log_messages {
                log_panel.push_info(msg);
            }
            for msg in rejected_logs {
                log_panel.push_warn(msg);
            }
        }
    }

    // Remove elements
    for (space_id, remove_queue) in remove_batches {
        let (allowed_remove_ids, rejected_logs) = {
            let Some(space_handle_tables) = world.get_resource::<SpaceHandleTables>() else {
                return;
            };
            let mut allowed_remove_ids = Vec::new();
            let mut rejected_logs = Vec::new();
            for local_id in remove_queue {
                if let Some(node_id) = resolve_global_id(&space_handle_tables, space_id, local_id) {
                    allowed_remove_ids.push(node_id);
                } else {
                    rejected_logs.push(format!(
                        "[JS][space:{space_id}] Blocked invalid local remove: local_id={local_id}"
                    ));
                }
            }
            (allowed_remove_ids, rejected_logs)
        };

        if let Some(mut script_load_states) = world.get_resource_mut::<ScriptLoadStates>() {
            for node_id in &allowed_remove_ids {
                script_load_states.0.remove(node_id);
            }
        }
        if let Some(mut pending_model_loads) = world.get_resource_mut::<PendingModelLoads>() {
            for node_id in &allowed_remove_ids {
                pending_model_loads.remove_node(*node_id);
            }
        }
        if let Some(mut model_load_states) = world.get_resource_mut::<ModelLoadStates>() {
            for node_id in &allowed_remove_ids {
                model_load_states.0.remove(node_id);
            }
        }

        let log_messages = {
            let Some(mut specs_world) = world.get_resource_mut::<ElemenetWorld>() else {
                return;
            };
            let mut log_messages = Vec::new();
            for &node_id in &allowed_remove_ids {
                let (ent, is_alive) = {
                    let entities = specs_world.0.entities();
                    let ent = entities.entity(node_id);
                    (ent, entities.is_alive(ent))
                };
                if is_alive {
                    specs_world.0.delete_entity(ent).ok();
                    log_messages.push(format!(
                        "[JS][space:{}] remove: node_id={}",
                        space_id, node_id
                    ));
                }
            }
            log_messages
        };
        if let Some(mut space_handle_tables) = world.get_resource_mut::<SpaceHandleTables>() {
            let Some(table) = space_handle_tables.by_space.get_mut(&space_id) else {
                return;
            };
            for node_id in &allowed_remove_ids {
                if let Some(local_id) = table.global_to_local.remove(node_id) {
                    table.local_to_global.remove(&local_id);
                }
                table.detached_globals.remove(node_id);
            }
        }
        if let Some(mut log_panel) = world.get_resource_mut::<LogPanel>() {
            for msg in log_messages {
                log_panel.push_info(msg);
            }
            for msg in rejected_logs {
                log_panel.push_warn(msg);
            }
        }
    }

    // Fetch
    for (space_id, fetch_queue) in fetch_batches {
        let Some(tokio_rt) = world.get_resource::<crate::TokioRuntime>() else {
            return;
        };
        let Some(io_service) = world.get_resource::<IoService>() else {
            return;
        };
        for (request_id, url) in &fetch_queue {
            request_fetch_text(&tokio_rt.0, &io_service, space_id, *request_id, url.clone());
        }
        if let Some(mut log_panel) = world.get_resource_mut::<LogPanel>() {
            for (_, url) in &fetch_queue {
                log_panel.push_info(format!("[JS][space:{}] fetch queued: {}", space_id, url));
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
