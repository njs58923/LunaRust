use std::{
    collections::{HashMap, HashSet},
    sync::mpsc,
    thread::{self, JoinHandle},
    time::Instant,
};

use bevy::prelude::*;
use specs::{Join, WorldExt};

use js_runtime::Engine as JsEngine;
use crate::dom::{collect_subtree_ids, find_nearest_ancestor_include, resolve_node_relative_url};
use crate::permissions::{CapabilityBits, SpacePolicies};
use virtual_dom::dom::{
    element::{Attrs, Hierarchy, Tag, Transform2},
    hsml::{Include, Model, Script},
};

use crate::{
    request_fetch_text, AttributeUpdates, DirtyNodes, ElemenetWorld, IoService, JsSnapshotState,
    LogLevel, LogPanel, ModelLoadStates, PendingModelLoads, PendingScripts, ReloadTrigger,
    ScriptLoadStates, SpaceHandleTable, SpaceHandleTables, TransformUpdates, VirtualDomData,
    PendingJsAttachNodes,
};

// ─── Types ───────────────────────────────────────────────────────────────────

pub struct SpaceScriptContext {
    pub engine: JsEngine,
    pub loaded_scripts: HashSet<String>,
    pub capabilities_bits: u64,
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
    pub needs_continuous_ticks: bool,
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
    pub tab_action_queue: Vec<js_runtime::TabAction>,
    pub shell_outbox: Vec<js_runtime::ShellMessage>,
    pub ws_connect_queue: Vec<(i32, String)>,
    pub ws_send_queue: Vec<(i32, String)>,
    pub ws_close_queue: Vec<i32>,
}

/// Evento que el bridge WS (`ws.rs`) inyecta de vuelta en un worker JS.
pub enum WsWorkerEvent {
    /// Cambio de estado: "open" | "closed" | "error: ...".
    Status { conn_id: i32, status: String },
    /// Mensaje entrante de texto.
    Message { conn_id: i32, data: String },
}

#[derive(Debug, Clone)]
pub struct PoseMoveEventData {
    pub node_id: i32,
    pub hand: String,
    pub px: f32,
    pub py: f32,
    pub pz: f32,
    pub dx: f32,
    pub dy: f32,
    pub dz: f32,
    pub trigger: f32,
    pub grip: f32,
    pub qx: f32,
    pub qy: f32,
    pub qz: f32,
    pub qw: f32,
}

pub enum JsWorkerCommand {
    SetCapabilities(u64),
    UpdateSnapshots(SpaceSnapshots),
    EvalScript { url: String, code: String },
    Tick { elapsed_ms: f64 },
    PushElementCreationResults(Vec<(i32, i32)>),
    PushFetchResults(Vec<(i32, std::result::Result<String, String>)>),
    PushWsEvents(Vec<WsWorkerEvent>),
    PushDomToqueEvents(Vec<(i32, f32, f32, f32)>),
    PushPoseMoveEvents(Vec<PoseMoveEventData>),
    PushToqueRawEvents(Vec<(i32, f32, f32, f32)>),
    /// (action, source) pairs — broadcast a spaces con READ_SYSTEM_INPUT.
    PushSystemInputEvents(Vec<(String, String)>),
    /// Actualiza snapshot de viewer pose visible vía op_read_viewer_pose.
    /// `None` limpia (worker sin cap READ_HMD_POSE).
    SetViewerPose(Option<js_runtime::ViewerPoseData>),
    /// Mensajes shell ↔ app. El campo `target_tab_id` en cada msg, al
    /// llegar al worker, indica "de quién viene" (the host swaps direction
    /// para el receptor).
    PushShellMessages(Vec<js_runtime::ShellMessage>),
    RequestDebugState,
    Shutdown,
}

pub enum JsWorkerEvent {
    EvalResult {
        url: String,
        already_loaded: bool,
        error: Option<String>,
    },
    TickData(JsTickData),
    DebugState {
        space_id: u32,
        loaded_scripts: Vec<String>,
        capabilities_bits: u64,
    },
    WorkerError(String),
}

pub struct SpaceScriptWorker {
    pub cmd_tx: mpsc::Sender<JsWorkerCommand>,
    pub event_rx: mpsc::Receiver<JsWorkerEvent>,
    pub snapshot_in_flight: bool,
    pub tick_in_flight: bool,
    pub needs_tick: bool,
    pub join: Option<JoinHandle<()>>,
    pub bootstrap_scripts_enqueued: HashSet<String>,
    pub last_capabilities_bits: u64,
    /// True once `luna://internal/root_api.js` has been flushed into the worker's cmd channel.
    /// Guards any eval that calls `dimension.luna.*`.
    pub root_api_sent: bool,
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

    // The root element uses local_id=0, which maps to global space_id
    // in the SpaceHandleTable. JS must use the local ID, not the global one.
    let set_root = "globalThis.hiperspace.dimention = new HSMLRootElement(0);".to_string();
    engine
        .eval(&set_root)
        .map_err(|e| format!("failed to set JS root for space {}: {}", space_id, e))?;

    Ok(SpaceScriptContext {
        engine,
        loaded_scripts: HashSet::new(),
        capabilities_bits: 0,
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
                    JsWorkerCommand::SetCapabilities(bits) => {
                        ctx.capabilities_bits = bits;
                    }
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
                        let is_ephemeral = url.starts_with("eval://");
                        // if !is_ephemeral && ctx.loaded_scripts.contains(&url) {
                        //     let _ = event_tx.send(JsWorkerEvent::EvalResult {
                        //         url,
                        //         already_loaded: true,
                        //         error: None,
                        //     });
                        //     continue;
                        // }
                        let wrapped_code = format!("(function(){{\n{}\n}})();", code);
                        match ctx.engine.eval(&wrapped_code) {
                            Ok(_) => {
                                if !is_ephemeral {
                                    ctx.loaded_scripts.insert(url.clone());
                                }
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
                            needs_continuous_ticks: ctx.engine.needs_continuous_ticks(),
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
                            tab_action_queue: ctx.engine.drain_tab_action_queue(),
                            shell_outbox: ctx.engine.drain_shell_outbox(),
                            ws_connect_queue: ctx.engine.drain_ws_connect_queue(),
                            ws_send_queue: ctx.engine.drain_ws_send_queue(),
                            ws_close_queue: ctx.engine.drain_ws_close_queue(),
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
                    JsWorkerCommand::PushWsEvents(events) => {
                        for evt in events {
                            match evt {
                                WsWorkerEvent::Status { conn_id, status } => {
                                    ctx.engine.set_ws_status(conn_id, status);
                                }
                                WsWorkerEvent::Message { conn_id, data } => {
                                    ctx.engine.push_ws_message(conn_id, data);
                                }
                            }
                        }
                    }
                    JsWorkerCommand::PushDomToqueEvents(events) => {
                        for (node_id, x, y, z) in events {
                            ctx.engine.push_dom_toque_event(node_id, x, y, z);
                        }
                    }
                    JsWorkerCommand::PushPoseMoveEvents(events) => {
                        for evt in events {
                            ctx.engine.push_posemove_event(
                                evt.node_id,
                                evt.hand,
                                evt.px,
                                evt.py,
                                evt.pz,
                                evt.dx,
                                evt.dy,
                                evt.dz,
                                evt.trigger,
                                evt.grip,
                                evt.qx,
                                evt.qy,
                                evt.qz,
                                evt.qw,
                            );
                        }
                    }
                    JsWorkerCommand::PushToqueRawEvents(events) => {
                        for (node_id, x, y, z) in events {
                            ctx.engine.push_touch_event(node_id, x, y, z);
                        }
                    }
                    JsWorkerCommand::PushSystemInputEvents(events) => {
                        for (action, source) in events {
                            ctx.engine.push_system_input_event(action, source);
                        }
                    }
                    JsWorkerCommand::SetViewerPose(data) => {
                        ctx.engine.set_viewer_pose(data);
                    }
                    JsWorkerCommand::PushShellMessages(msgs) => {
                        ctx.engine.push_shell_messages(msgs);
                    }
                    JsWorkerCommand::RequestDebugState => {
                        let _ = event_tx.send(JsWorkerEvent::DebugState {
                            space_id,
                            loaded_scripts: ctx.loaded_scripts.iter().cloned().collect(),
                            capabilities_bits: ctx.capabilities_bits,
                        });
                    }
                    JsWorkerCommand::Shutdown => break,
                }
            }
        })
        .map_err(|e| format!("failed to spawn JS worker for space {}: {}", space_id, e))?;

    Ok(SpaceScriptWorker {
        cmd_tx,
        event_rx,
        snapshot_in_flight: false,
        tick_in_flight: false,
        needs_tick: true,
        join: Some(join),
        bootstrap_scripts_enqueued: HashSet::new(),
        last_capabilities_bits: 0,
        root_api_sent: false,
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

/// Encuentra el space_id del worker root (id="luna_root"). Duplicado del helper
/// en main.rs para evitar dependencia inversa.
fn find_root_worker_space_id_local(
    specs_world: &specs::World,
    manager: &ScriptRuntimeManager,
) -> Option<u32> {
    let attrs = specs_world.read_storage::<Attrs>();
    for &space_id in manager.contexts.keys() {
        let ent = specs_world.entities().entity(space_id);
        if let Some(a) = attrs.get(ent) {
            if a.0.get("id").map(|v| v.as_str()) == Some("luna_root") {
                return Some(space_id);
            }
        }
    }
    None
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

fn insert_tag_specific_components(
    world: &mut specs::World,
    entity: specs::Entity,
    tag_name: &str,
) {
    match tag_name {
        "model" => {
            let mut storage = world.write_storage::<Model>();
            let _ = storage.insert(entity, Model { src: None });
        }
        "include" => {
            let mut storage = world.write_storage::<Include>();
            let _ = storage.insert(entity, Include { src: None });
        }
        "script" => {
            let mut storage = world.write_storage::<Script>();
            let _ = storage.insert(
                entity,
                Script {
                    src: None,
                    inline: None,
                },
            );
        }
        _ => {}
    }
}

fn sync_space_handle_table(table: &mut SpaceHandleTable, space_id: u32, allowed: &HashSet<i32>) {
    table.global_to_local.entry(space_id).or_insert(0);
    table.local_to_global.entry(0).or_insert(space_id);
    table.detached_globals.remove(&space_id);

    let allowed_globals: HashSet<u32> = allowed.iter().map(|node_id| *node_id as u32).collect();
    let mut retained_globals: HashSet<u32> = allowed_globals
        .union(&table.detached_globals)
        .copied()
        .collect();
    // El space root del isolate (`local 0 → global space_id`) SIEMPRE debe
    // sobrevivir. Si el space global aún no está en `dom_data.nodes` (porque
    // se acaba de crear async), el retain lo borraría — rompiendo el primer
    // `op_hsml_set_position` del worker. Esto se manifestaba como apps
    // embedded apareciendo en (0,0,0) la primera vez.
    retained_globals.insert(space_id);

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
        let parent = hier.get(node).and_then(|h| h.parent)?;
        node = parent;
    }
}

fn space_capabilities_snapshot(world: &World) -> HashMap<u32, CapabilityBits> {
    world
        .get_resource::<SpacePolicies>()
        .map(|policies| {
            policies
                .by_space
                .iter()
                .map(|(&space_id, policy)| (space_id, policy.effective_caps))
                .collect::<HashMap<_, _>>()
        })
        .unwrap_or_default()
}

enum NavigationPlan {
    SelfNav { include_id: u32, url: String },
    GlobalNav { url: String },
    Blocked { url: String, reason: String },
}

fn plan_navigation_for_space(
    specs_world: &specs::World,
    current_url: &str,
    space_id: u32,
    requested_url: &str,
    caps: CapabilityBits,
) -> NavigationPlan {
    let entities = specs_world.entities();
    let space_ent = entities.entity(space_id);
    if !entities.is_alive(space_ent) {
        return NavigationPlan::Blocked {
            url: requested_url.to_string(),
            reason: "space not alive".to_string(),
        };
    }

    let final_url = resolve_node_relative_url(specs_world, space_ent, current_url, requested_url)
        .unwrap_or_else(|| requested_url.to_string());

    if caps.contains(CapabilityBits::NAVIGATE_SELF) {
        if let Some(include_ent) = find_nearest_ancestor_include(specs_world, space_ent) {
            return NavigationPlan::SelfNav {
                include_id: include_ent.id(),
                url: final_url,
            };
        }
    }

    if caps.contains(CapabilityBits::NAVIGATE_GLOBAL) {
        return NavigationPlan::GlobalNav { url: final_url };
    }

    NavigationPlan::Blocked {
        url: final_url,
        reason: "missing navigate_self / navigate_global capability".to_string(),
    }
}

pub fn js_sync_space_permissions_system(world: &mut World) {
    let desired: Vec<(u32, u64)> = {
        let Some(policies) = world.get_resource::<SpacePolicies>() else {
            return;
        };
        policies
            .by_space
            .iter()
            .map(|(&space_id, policy)| (space_id, policy.effective_caps.bits()))
            .collect()
    };

    let Some(mut manager) = world.get_non_send_resource_mut::<ScriptRuntimeManager>() else {
        return;
    };

    for (space_id, bits) in desired {
        if let Some(worker) = manager.contexts.get_mut(&space_id) {
            if worker.last_capabilities_bits != bits {
                let _ = worker.cmd_tx.send(JsWorkerCommand::SetCapabilities(bits));
                worker.last_capabilities_bits = bits;
            }
        }
    }
}

pub fn js_auto_inject_resource_scripts_system(world: &mut World) {
    let desired: Vec<(u32, Vec<String>)> = {
        let Some(policies) = world.get_resource::<SpacePolicies>() else {
            return;
        };
        policies
            .by_space
            .iter()
            .map(|(&space_id, policy)| (space_id, policy.auto_scripts.clone()))
            .collect()
    };

    if desired.is_empty() {
        return;
    }

    let mut queued = Vec::new();
    {
        let Some(mut manager) = world.get_non_send_resource_mut::<ScriptRuntimeManager>() else {
            return;
        };
        for (space_id, scripts) in desired {
            let Some(worker) = manager.contexts.get_mut(&space_id) else {
                continue;
            };
            for script_url in scripts {
                if !worker.bootstrap_scripts_enqueued.insert(script_url.clone()) {
                    continue;
                }
                if let Some(code) = crate::VIRTUAL_ROUTES.resolve(&script_url) {
                    queued.push((space_id, script_url, code));
                }
            }
        }
    }

    if !queued.is_empty() {
        if let Some(mut pending_scripts) = world.get_resource_mut::<PendingScripts>() {
            pending_scripts.0.extend(queued);
        }
    }
}

// ─── Systems ─────────────────────────────────────────────────────────────────

pub fn js_update_snapshots_system(world: &mut World) {
    let should_refresh = world
        .get_resource::<JsSnapshotState>()
        .map(|state| state.dirty)
        .unwrap_or(true);
    if !should_refresh {
        return;
    }

    let attached_node_ids: HashSet<u32> = world
    .get_resource::<VirtualDomData>()
    .map(|dom| dom.nodes.keys().copied().collect())
    .unwrap_or_default();

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
            if !attached_node_ids.contains(&ent.id()) {
                continue;
            }
            let mut map = HashMap::new();
            for (k, v) in &attrs.0 {
                map.insert(k.clone(), v.clone());
            }
            attr_snap.insert(ent.id() as i32, map);
        }

        let mut tag_snap = HashMap::new();
        for (ent, tag) in (&entities, &tags_storage).join() {
            if !attached_node_ids.contains(&ent.id()) {
                continue;
            }
            tag_snap.insert(ent.id() as i32, tag.0.clone());
        }

        let mut positions = HashMap::new();
        let mut rotations = HashMap::new();
        let mut scales = HashMap::new();
        let mut global_positions = HashMap::new();
        for (ent, tr) in (&entities, &transforms_storage).join() {
            if !attached_node_ids.contains(&ent.id()) {
                continue;
            }
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
            if !attached_node_ids.contains(&ent.id()) {
                continue;
            }
            parents.insert(ent.id() as i32, hier.parent.map(|p| p.id() as i32).unwrap_or(-1));
            children_map.insert(
                ent.id() as i32,
                hier.children
                    .iter()
                    .filter(|child| attached_node_ids.contains(&child.id()))
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

    let mut all_snapshots_sent = true;
    {
        let Some(mut manager) = world.get_non_send_resource_mut::<ScriptRuntimeManager>() else {
            return;
        };
        let mut broken_contexts = Vec::new();
        for (space_id, snap) in snapshot_batches {
            let Some(worker) = manager.contexts.get_mut(&space_id) else {
                continue;
            };
            if worker.snapshot_in_flight {
                all_snapshots_sent = false;
                continue;
            }
            if let Err(e) = worker.cmd_tx.send(JsWorkerCommand::UpdateSnapshots(snap)) {
                errors.push(format!(
                    "failed to send snapshots to space {}: {}",
                    space_id, e
                ));
                broken_contexts.push(space_id);
                all_snapshots_sent = false;
            } else {
                worker.snapshot_in_flight = true;
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

    let keep_dirty = !all_snapshots_sent || !errors.is_empty();
    if let Some(mut snapshot_state) = world.get_resource_mut::<JsSnapshotState>() {
        snapshot_state.dirty = keep_dirty;
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
                let result = worker
                    .cmd_tx
                    .send(JsWorkerCommand::EvalScript {
                        url: url.clone(),
                        code,
                    })
                    .map_err(|e| e.to_string());
                if result.is_ok() && url == "luna://internal/root_api.js" {
                    worker.root_api_sent = true;
                }
                if result.is_ok() {
                    worker.needs_tick = true;
                }
                result
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

        let mut broken_contexts = Vec::new();
        for (space_id, worker) in manager.contexts.iter_mut() {
            if worker.tick_in_flight || !worker.needs_tick {
                continue;
            }
            match worker.cmd_tx.send(JsWorkerCommand::Tick { elapsed_ms }) {
                Ok(_) => worker.tick_in_flight = true,
                Err(_) => broken_contexts.push(*space_id),
            }
        }

        for (space_id, worker) in manager.contexts.iter_mut() {
            loop {
                match worker.event_rx.try_recv() {
                    Ok(JsWorkerEvent::EvalResult {
                        url,
                        already_loaded,
                        error,
                    }) => {
                        worker.snapshot_in_flight = false;
                        worker.tick_in_flight = false;
                        worker.needs_tick = true;
                        eval_events.push((*space_id, url, already_loaded, error));
                    }
                    Ok(JsWorkerEvent::TickData(data)) => {
                        worker.snapshot_in_flight = false;
                        worker.tick_in_flight = false;
                        worker.needs_tick = data.needs_continuous_ticks;
                        tick_batches.push((*space_id, data));
                    }
                    Ok(JsWorkerEvent::DebugState {
                        space_id: _debug_space_id,
                        loaded_scripts: _loaded_scripts,
                        capabilities_bits: _capabilities_bits,
                    }) => {
                        // TODO: guardar en resource de debug si se va a mostrar en UI
                    }
                    Ok(JsWorkerEvent::WorkerError(err)) => {
                        worker.snapshot_in_flight = false;
                        worker.tick_in_flight = false;
                        worker.needs_tick = false;
                        worker_errors.push(format!("[JS][space:{}] {}", space_id, err));
                    }
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        worker.snapshot_in_flight = false;
                        worker.tick_in_flight = false;
                        worker.needs_tick = false;
                        broken_contexts.push(*space_id);
                        break;
                    }
                }
            }
        }

        broken_contexts.sort_unstable();
        broken_contexts.dedup();

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
                log_panel.push_for_space(
                    LogLevel::Info,
                    format!("[JS][space:{}] Script already loaded, skipping: {}", space_id, url),
                    space_id,
                );
            } else if let Some(err) = error {
                log_panel.push_for_space(
                    LogLevel::Error,
                    format!("[JS][space:{}] Error evaluating {}: {}", space_id, url, err),
                    space_id,
                );
            } else {
                log_panel.push_for_space(
                    LogLevel::Info,
                    format!("[JS][space:{}] Script evaluated OK: {}", space_id, url),
                    space_id,
                );
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
    let mut tab_action_batches: Vec<(u32, Vec<js_runtime::TabAction>)> = Vec::new();
    let mut shell_message_batches: Vec<(u32, Vec<js_runtime::ShellMessage>)> = Vec::new();
    let mut ws_connect_batches: Vec<(u32, Vec<(i32, String)>)> = Vec::new();
    let mut ws_send_batches: Vec<(u32, Vec<(i32, String)>)> = Vec::new();
    let mut ws_close_batches: Vec<(u32, Vec<i32>)> = Vec::new();
    let mut snapshot_dirty = false;

    let capabilities_by_space = space_capabilities_snapshot(world);

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
        if !data.tab_action_queue.is_empty() {
            tab_action_batches.push((space_id, data.tab_action_queue));
        }
        if !data.shell_outbox.is_empty() {
            shell_message_batches.push((space_id, data.shell_outbox));
        }
        if !data.ws_connect_queue.is_empty() {
            ws_connect_batches.push((space_id, data.ws_connect_queue));
        }
        if !data.ws_send_queue.is_empty() {
            ws_send_batches.push((space_id, data.ws_send_queue));
        }
        if !data.ws_close_queue.is_empty() {
            ws_close_batches.push((space_id, data.ws_close_queue));
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

    let dom_structure_changed =
        !creation_batches.is_empty() || !hierarchy_batches.is_empty() || !remove_batches.is_empty();

    {
        let Some(mut log_panel) = world.get_resource_mut::<LogPanel>() else {
            return;
        };
        for (space_id, logs) in logs_by_context {
            for (level, msg) in logs {
                let log_level = match level.as_str() {
                    "warn" => LogLevel::Warn,
                    "error" => LogLevel::Error,
                    _ => LogLevel::Info,
                };
                log_panel.push_for_space(
                    log_level,
                    format!("[JS][space:{}] {}", space_id, msg),
                    space_id,
                );
            }
        }
    }

    let mut ownership_logs = Vec::new();
    let (
        validated_attribute_updates,
        validated_position_updates,
        validated_rotation_updates,
        validated_scale_updates,
    ) = {
        let Some(space_handle_tables) = world.get_resource::<SpaceHandleTables>() else {
            return;
        };
        let mut validated = Vec::new();
        let mut validated_positions = Vec::new();
        let mut validated_rotations = Vec::new();
        let mut validated_scales = Vec::new();

        for (space_id, updates) in attr_update_batches {
            let Some(table) = space_handle_tables.by_space.get(&space_id) else {
                continue;
            };
            for (local_id, key, value) in updates {
                if let Some(global_id) = table.local_to_global.get(&local_id).copied() {
                    validated.push((global_id, key, value));
                } else {
                    ownership_logs.push(format!(
                        "[JS][space:{space_id}] Blocked invalid local attribute write: local_id={local_id}, key={key}"
                    ));
                }
            }
        }

        for (space_id, updates) in pos_update_batches {
            let Some(table) = space_handle_tables.by_space.get(&space_id) else {
                continue;
            };
            for (local_id, pos) in updates {
                if let Some(global_id) = table.local_to_global.get(&local_id).copied() {
                    validated_positions.push((global_id, pos));
                } else {
                    ownership_logs.push(format!(
                        "[JS][space:{space_id}] Blocked invalid local position write: local_id={local_id}"
                    ));
                }
            }
        }

        for (space_id, updates) in rot_update_batches {
            let Some(table) = space_handle_tables.by_space.get(&space_id) else {
                continue;
            };
            for (local_id, rot) in updates {
                if let Some(global_id) = table.local_to_global.get(&local_id).copied() {
                    validated_rotations.push((global_id, rot));
                } else {
                    ownership_logs.push(format!(
                        "[JS][space:{space_id}] Blocked invalid local rotation write: local_id={local_id}"
                    ));
                }
            }
        }

        for (space_id, updates) in scale_update_batches {
            let Some(table) = space_handle_tables.by_space.get(&space_id) else {
                continue;
            };
            for (local_id, scale) in updates {
                if let Some(global_id) = table.local_to_global.get(&local_id).copied() {
                    validated_scales.push((global_id, scale));
                } else {
                    ownership_logs.push(format!(
                        "[JS][space:{space_id}] Blocked invalid local scale write: local_id={local_id}"
                    ));
                }
            }
        }

        (
            validated,
            validated_positions,
            validated_rotations,
            validated_scales,
        )
    };

    {
        let Some(mut attribute_updates) = world.get_resource_mut::<AttributeUpdates>() else {
            return;
        };
        attribute_updates.0.extend(validated_attribute_updates);
    }
    
    {
        let Some(mut transform_updates) = world.get_resource_mut::<TransformUpdates>() else {
            return;
        };
        transform_updates.positions.extend(validated_position_updates);
        transform_updates.rotations.extend(validated_rotation_updates);
        transform_updates.scales.extend(validated_scale_updates);
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
                    {
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
                    }
                    insert_tag_specific_components(&mut specs_world.0, new_ent, &tag_name);
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

        if let Some(mut log_panel) = world.get_resource_mut::<LogPanel>() {
            for msg in log_messages {
                log_panel.push_info(msg);
            }
        }
        if let Some(mut manager) = world.get_non_send_resource_mut::<ScriptRuntimeManager>() {
            if let Some(worker) = manager.contexts.get_mut(&space_id) {
                let send_result = worker
                    .cmd_tx
                    .send(JsWorkerCommand::PushElementCreationResults(
                        creation_results,
                    ));
                if send_result.is_ok() {
                    worker.needs_tick = true;
                }
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

        let mut attached_now: HashSet<u32> = world
            .get_resource::<crate::VirtualDomData>()
            .map(|dom| dom.nodes.keys().copied().collect())
            .unwrap_or_default();

        let (newly_attached_ids, already_attached_dirty_ids, log_messages) = {
            let Some(mut specs_world) = world.get_resource_mut::<ElemenetWorld>() else {
                return;
            };
            let mut newly_attached_ids: Vec<u32> = Vec::new();
            let mut newly_attached_seen = HashSet::new();
            let mut already_attached_dirty_ids = HashSet::new();
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

                    if attached_now.contains(&parent_id) {
                        let mut subtree_ids = Vec::new();
                        collect_subtree_ids(&specs_world.0, child_ent, &mut subtree_ids);

                        for node_id in subtree_ids {
                            if attached_now.insert(node_id) {
                                if newly_attached_seen.insert(node_id) {
                                    newly_attached_ids.push(node_id);
                                }
                            } else {
                                already_attached_dirty_ids.insert(node_id);
                            }
                        }
                    }                   

                    log_messages.push(format!(
                        "[JS][space:{}] appendChild: parent={} child={}",
                        space_id, parent_id, child_id
                    ));
                }
            }
            (newly_attached_ids, already_attached_dirty_ids, log_messages)
        };

        if let Some(mut pending_js_attaches) = world.get_resource_mut::<PendingJsAttachNodes>() {
            pending_js_attaches.0.extend(newly_attached_ids.iter().copied());
        }

        let dirty_ids_vec: Vec<u32> = already_attached_dirty_ids.into_iter().collect();

        // IMPORTANT: solo retiramos de detached_globals para nodos YA presentes
        // en dom_data (`already_attached_dirty_ids`). Para `newly_attached_ids`,
        // dom_data aún no los conoce (commit_pending_js_attaches los inserta
        // en el mismo frame, pero js_update_snapshots_system del frame siguiente
        // podría correr antes que ese commit y `sync_space_handle_table` podaría
        // el mapping local→global, dejando futuras escrituras de pos/rot del JS
        // como "Blocked invalid local position write" (zombies bullets stuck).
        // El retiro de detached_globals para newly_attached lo hace
        // `commit_pending_js_attaches_system` cuando dom_data ya los contiene.
        if let Some(mut space_handle_tables) = world.get_resource_mut::<SpaceHandleTables>() {
            let Some(table) = space_handle_tables.by_space.get_mut(&space_id) else {
                return;
            };
            for node_id in dirty_ids_vec.iter() {
                table.detached_globals.remove(node_id);
            }
        }
        if let Some(mut dirty_nodes) = world.get_resource_mut::<DirtyNodes>() {
            dirty_nodes.0.extend(dirty_ids_vec.iter().copied());
        }
        if !dirty_ids_vec.is_empty() || !newly_attached_ids.is_empty() {
            snapshot_dirty = true;
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

    // Remove elements.
    // `frame_deleted_global` deduplica ACROSS workers: el root worker puede
    // pedir remover el subtree completo (spatial space) mientras el worker
    // del spatial — todavía vivo este frame — pide remover algún hijo suyo.
    // Si ambos llegan al mismo specs entity, el segundo delete_entity
    // panic-ea en debug_assert de Generation::die(). HashSet vive a lo largo
    // de todos los batches del frame.
    let mut frame_deleted_global: std::collections::HashSet<u32> =
        std::collections::HashSet::new();
    // Snapshot ONE-TIME del set "attached" para todo el frame. Si hay 5 workers
    // que piden removes, el snapshot vale para los 5 (dom_data.nodes no se
    // modifica dentro de este sistema — el cleanup definitivo lo hace
    // dom_sync_system después, en otro pase).
    let attached_nodes: std::collections::HashSet<u32> = if remove_batches.is_empty() {
        std::collections::HashSet::new()
    } else {
        world
            .get_resource::<crate::VirtualDomData>()
            .map(|d| d.nodes.keys().copied().collect())
            .unwrap_or_default()
    };
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

        let (log_messages, all_removed_ids) = {
            let Some(mut specs_world) = world.get_resource_mut::<ElemenetWorld>() else {
                return;
            };
            let mut log_messages = Vec::new();
            let mut all_removed_ids: Vec<u32> = Vec::new();
            for &node_id in &allowed_remove_ids {
                if !attached_nodes.contains(&node_id) {
                    // Stale id de cola JS — el delete causaría panic por
                    // false-positive de is_alive sobre slot virginal.
                    continue;
                }
                let subtree = {
                    let entities = specs_world.0.entities();
                    let hier = specs_world.0.read_storage::<Hierarchy>();
                    let root_ent = entities.entity(node_id);
                    if !entities.is_alive(root_ent) {
                        continue;
                    }
                    let mut to_delete = Vec::new();
                    let mut stack = vec![root_ent];
                    let mut local_seen: std::collections::HashSet<u32> =
                        std::collections::HashSet::new();
                    while let Some(ent) = stack.pop() {
                        let id = ent.id();
                        if !local_seen.insert(id) {
                            continue;
                        }
                        // Sólo procesar nodos realmente attached.
                        if !attached_nodes.contains(&id) {
                            continue;
                        }
                        to_delete.push(id);
                        if let Some(h) = hier.get(ent) {
                            for &child in &h.children {
                                let cid = child.id();
                                if !attached_nodes.contains(&cid) {
                                    continue;
                                }
                                let child_ent = entities.entity(cid);
                                if entities.is_alive(child_ent) {
                                    stack.push(child_ent);
                                }
                            }
                        }
                    }
                    to_delete
                };
                all_removed_ids.extend_from_slice(&subtree);
                let mut deleted = 0usize;
                let mut skipped = 0usize;
                for &nid in &subtree {
                    if !frame_deleted_global.insert(nid) {
                        skipped += 1;
                        continue;
                    }
                    let ent = specs_world.0.entities().entity(nid);
                    if !specs_world.0.entities().is_alive(ent) {
                        skipped += 1;
                        continue;
                    }
                    match specs_world.0.delete_entity(ent) {
                        Ok(()) => deleted += 1,
                        Err(_) => skipped += 1,
                    }
                }
                // Loguear sólo si hay anomalías (skipped > 0). Estado normal
                // = quieto. Si querés trace siempre, comentá la guarda.
                if skipped > 0 {
                    log_messages.push(format!(
                        "[JS][space:{}] remove node_id={} subtree={} deleted={} skipped={}",
                        space_id,
                        node_id,
                        subtree.len(),
                        deleted,
                        skipped,
                    ));
                }
            }
            (log_messages, all_removed_ids)
        };
        // Collect Bevy entities to despawn, then despawn them
        let bevy_entities_to_despawn: Vec<Entity> = {
            let mut to_despawn = Vec::new();
            if let Some(mut entity_map) = world.get_resource_mut::<crate::EntityMap>() {
                for &nid in &all_removed_ids {
                    if let Some(bevy_ent) = entity_map.0.remove(&nid) {
                        to_despawn.push(bevy_ent);
                    }
                }
            }
            to_despawn
        };
        for bevy_ent in bevy_entities_to_despawn {
            if world.get_entity(bevy_ent).is_some() {
                world.entity_mut(bevy_ent).despawn_recursive();
            }
        }
        // Clean up async node state (scripts, models)
        {
            let mut script_loads = world.resource_mut::<crate::ScriptLoadStates>();
            for &nid in &all_removed_ids {
                script_loads.0.remove(&nid);
            }
        }
        {
            let mut pending_models = world.resource_mut::<crate::PendingModelLoads>();
            for &nid in &all_removed_ids {
                pending_models.remove_node(nid);
            }
        }
        {
            let mut model_loads = world.resource_mut::<crate::ModelLoadStates>();
            for &nid in &all_removed_ids {
                model_loads.0.remove(&nid);
            }
        }
        // Clean up dom_data entries
        if let Some(mut dom_data) = world.get_resource_mut::<crate::VirtualDomData>() {
            for &nid in &all_removed_ids {
                dom_data.nodes.remove(&nid);
            }
        }
        // Clean up include load states
        if let Some(mut include_states) = world.get_resource_mut::<crate::IncludeLoadStates>() {
            for &nid in &all_removed_ids {
                include_states.0.remove(&nid);
            }
        }
        // Clean up handle table entries for ALL subtree nodes
        if let Some(mut space_handle_tables) = world.get_resource_mut::<SpaceHandleTables>() {
            let Some(table) = space_handle_tables.by_space.get_mut(&space_id) else {
                return;
            };
            for &nid in &all_removed_ids {
                if let Some(local_id) = table.global_to_local.remove(&nid) {
                    table.local_to_global.remove(&local_id);
                }
                table.detached_globals.remove(&nid);
            }
        }
        if !all_removed_ids.is_empty() {
            snapshot_dirty = true;
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
        let can_fetch = capabilities_by_space
            .get(&space_id)
            .map(|caps| caps.contains(CapabilityBits::FETCH_TEXT))
            .unwrap_or(false);
        if !can_fetch {
            if let Some(mut log_panel) = world.get_resource_mut::<LogPanel>() {
                for (_, url) in &fetch_queue {
                    log_panel.push_warn(format!(
                        "[JS][space:{}] blocked fetch (missing fetch_text): {}",
                        space_id, url
                    ));
                }
            }
            continue;
        }

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

    // WebSocket: abre conexiones, encola sends, cierra. El transporte real vive
    // en `ws.rs`; aquí sólo encaminamos las colas drenadas del engine.
    if !ws_connect_batches.is_empty()
        || !ws_send_batches.is_empty()
        || !ws_close_batches.is_empty()
    {
        let (Some(tokio_rt), Some(ws_service)) = (
            world.get_resource::<crate::TokioRuntime>(),
            world.get_resource::<crate::WsService>(),
        ) else {
            return;
        };
        for (space_id, queue) in ws_connect_batches {
            for (conn_id, url) in queue {
                crate::ws::request_ws_connect(&tokio_rt.0, ws_service, space_id, conn_id, url);
            }
        }
        for (space_id, queue) in ws_send_batches {
            for (conn_id, msg) in queue {
                crate::ws::ws_send(ws_service, space_id, conn_id, msg);
            }
        }
        for (space_id, queue) in ws_close_batches {
            for conn_id in queue {
                crate::ws::ws_close(ws_service, space_id, conn_id);
            }
        }
    }

    // Navigate (self include if allowed; otherwise global if allowed)
    let navigation_plans = {
        let Some(specs_world) = world.get_resource::<ElemenetWorld>() else {
            return;
        };
        let current_url = world
            .get_resource::<crate::CurrentUrl>()
            .map(|u| u.0.clone())
            .unwrap_or_default();

        let mut plans = Vec::new();
        for (space_id, urls) in navigate_batches {
            let caps = capabilities_by_space
                .get(&space_id)
                .copied()
                .unwrap_or_default();
            for requested_url in urls {
                plans.push((
                    space_id,
                    plan_navigation_for_space(&specs_world.0, &current_url, space_id, &requested_url, caps),
                ));
            }
        }
        plans
    };

    let mut last_global_navigation: Option<String> = None;
    for (space_id, plan) in navigation_plans {
        if let Some(mut log_panel) = world.get_resource_mut::<LogPanel>() {
            match &plan {
                NavigationPlan::SelfNav { include_id, url } => {
                    log_panel.push_info(format!(
                        "[JS][space:{}] self-navigate include={} -> {}",
                        space_id, include_id, url
                    ));
                }
                NavigationPlan::GlobalNav { url } => {
                    log_panel.push_warn(format!("[JS][space:{}] global navigate: {}", space_id, url));
                }
                NavigationPlan::Blocked { url, reason } => {
                    log_panel.push_warn(format!(
                        "[JS][space:{}] blocked navigate {} ({})",
                        space_id, url, reason
                    ));
                }
            }
        }

        match plan {
            NavigationPlan::SelfNav { include_id, url } => {
                if let Some(mut attribute_updates) = world.get_resource_mut::<AttributeUpdates>() {
                    attribute_updates
                        .0
                        .push((include_id, "src".to_string(), url));
                }
            }
            NavigationPlan::GlobalNav { url } => {
                last_global_navigation = Some(url);
            }
            NavigationPlan::Blocked { .. } => {}
        }
    }

    if let Some(url) = last_global_navigation {
        if let Some(mut current_url) = world.get_resource_mut::<crate::CurrentUrl>() {
            current_url.0 = url;
        }
        if let Some(mut reload_trigger) = world.get_resource_mut::<ReloadTrigger>() {
            reload_trigger.0 = true;
        }
    }

    // ── Tab actions (chrome.tabs-like API) ──────────────────────────────────
    // Cada acción se valida por cap del space caller, luego se traduce a la
    // queue host correspondiente (SpaceMountQueue / SpaceUnmountQueue).
    if !tab_action_batches.is_empty() {
        let mut open_requests: Vec<(u32, String, String)> = Vec::new(); // (space_id, url, kind)
        let mut close_requests: Vec<(u32, u64)> = Vec::new();
        let mut visibility_requests: Vec<(u32, u64, bool)> = Vec::new();
        let mut pose_requests: Vec<(u32, u64, [f32; 6])> = Vec::new(); // (space, tab, [px,py,pz,rx,ry,rz])

        for (space_id, actions) in tab_action_batches {
            for action in actions {
                match action {
                    js_runtime::TabAction::Open { url, kind } => {
                        let caps = capabilities_by_space
                            .get(&space_id)
                            .copied()
                            .unwrap_or_default();
                        if caps.contains(CapabilityBits::MOUNT_ROOT_SPACE) {
                            open_requests.push((space_id, url, kind));
                        } else if let Some(mut log_panel) =
                            world.get_resource_mut::<LogPanel>()
                        {
                            log_panel.push_warn(format!(
                                "[JS][space:{}] tabs.open denied (missing MOUNT_ROOT_SPACE): {} ({})",
                                space_id, url, kind
                            ));
                        }
                    }
                    js_runtime::TabAction::Close { tab_id } => {
                        let caps = capabilities_by_space
                            .get(&space_id)
                            .copied()
                            .unwrap_or_default();
                        if caps.contains(CapabilityBits::UNMOUNT_ROOT_SPACE) {
                            close_requests.push((space_id, tab_id));
                        } else if let Some(mut log_panel) =
                            world.get_resource_mut::<LogPanel>()
                        {
                            log_panel.push_warn(format!(
                                "[JS][space:{}] tabs.close denied (missing UNMOUNT_ROOT_SPACE): tab_id={}",
                                space_id, tab_id
                            ));
                        }
                    }
                    js_runtime::TabAction::SetVisible { tab_id, visible } => {
                        let caps = capabilities_by_space
                            .get(&space_id)
                            .copied()
                            .unwrap_or_default();
                        if caps.contains(CapabilityBits::UPDATE_ROOT_SPACE) {
                            visibility_requests.push((space_id, tab_id, visible));
                        } else if let Some(mut log_panel) =
                            world.get_resource_mut::<LogPanel>()
                        {
                            log_panel.push_warn(format!(
                                "[JS][space:{}] tabs.setVisible denied (missing UPDATE_ROOT_SPACE)",
                                space_id
                            ));
                        }
                    }
                    js_runtime::TabAction::SetPose { tab_id, px, py, pz, rx, ry, rz } => {
                        let caps = capabilities_by_space
                            .get(&space_id)
                            .copied()
                            .unwrap_or_default();
                        if caps.contains(CapabilityBits::UPDATE_ROOT_SPACE) {
                            pose_requests.push((space_id, tab_id, [px, py, pz, rx, ry, rz]));
                        } else if let Some(mut log_panel) =
                            world.get_resource_mut::<LogPanel>()
                        {
                            log_panel.push_warn(format!(
                                "[JS][space:{}] tabs.setPose denied (missing UPDATE_ROOT_SPACE)",
                                space_id
                            ));
                        }
                    }
                }
            }
        }

        if !open_requests.is_empty() {
            // Allocate tab IDs y empujar a SpaceMountQueue. El procesador
            // (process_space_mount_queue) reenvía al root worker como
            // dimension.luna.mountSpace(url, {grants, tabId}).
            let mut next_id_value: u64 = world
                .get_resource::<crate::NextTabId>()
                .map(|n| n.0)
                .unwrap_or(1);
            let mut mount_pushes: Vec<crate::SpaceMountRequest> = Vec::new();
            for (_space_id, url, kind) in open_requests {
                mount_pushes.push(crate::SpaceMountRequest {
                    tab_id: next_id_value,
                    url,
                    kind,
                });
                next_id_value = next_id_value.saturating_add(1);
            }
            if let Some(mut next_tab_id) = world.get_resource_mut::<crate::NextTabId>() {
                next_tab_id.0 = next_id_value;
            }
            if let Some(mut mount_queue) = world.get_resource_mut::<crate::SpaceMountQueue>() {
                mount_queue.0.extend(mount_pushes);
            }
        }

        if !close_requests.is_empty() {
            let mut unmount_pushes: Vec<crate::SpaceUnmountRequest> = Vec::new();
            for (_space_id, tab_id) in close_requests {
                unmount_pushes.push(crate::SpaceUnmountRequest {
                    tab_id,
                    url: String::new(),
                });
            }
            if let Some(mut unmount_queue) =
                world.get_resource_mut::<crate::SpaceUnmountQueue>()
            {
                unmount_queue.0.extend(unmount_pushes);
            }
        }

        // tabs.setVisible / tabs.setPose — reenvía al root worker que aplica
        // el cambio al outer wrapper del tab via root_api. Con Visibility::Inherited
        // en los hijos, el cambio propaga a todo el subárbol del tab.
        if !visibility_requests.is_empty() || !pose_requests.is_empty() {
            let root_worker_id: Option<u32> = {
                let Some(specs_world) = world.get_resource::<ElemenetWorld>() else {
                    return;
                };
                let Some(manager) = world.get_non_send_resource::<ScriptRuntimeManager>() else {
                    return;
                };
                find_root_worker_space_id_local(&specs_world.0, manager)
            };
            if let Some(root_id) = root_worker_id {
                if let Some(mut manager) = world.get_non_send_resource_mut::<ScriptRuntimeManager>() {
                    if let Some(worker) = manager.contexts.get_mut(&root_id) {
                        if worker.root_api_sent {
                            // Pose primero, después visibility. Garantiza que
                            // un pattern típico `setPose + setVisible(true)` aplique
                            // pose antes de mostrar — sin frame visible en (0,0,0).
                            for (_sender_space_id, tab_id, p) in pose_requests {
                                let code = format!(
                                    "dimension.luna.setSpacePoseByTabId({}, {{x:{},y:{},z:{}}}, {{x:{},y:{},z:{}}});",
                                    tab_id, p[0], p[1], p[2], p[3], p[4], p[5]
                                );
                                let send_result = worker.cmd_tx.send(JsWorkerCommand::EvalScript {
                                    url: format!("eval://setPose/{}", tab_id),
                                    code,
                                });
                                if send_result.is_ok() {
                                    worker.needs_tick = true;
                                }
                            }
                            for (_sender_space_id, tab_id, visible) in visibility_requests {
                                let code = format!(
                                    "dimension.luna.setSpaceVisibleByTabId({}, {});",
                                    tab_id,
                                    if visible { "true" } else { "false" }
                                );
                                let send_result = worker.cmd_tx.send(JsWorkerCommand::EvalScript {
                                    url: format!("eval://setVisible/{}/{}", tab_id, visible),
                                    code,
                                });
                                if send_result.is_ok() {
                                    worker.needs_tick = true;
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // ── Shell ↔ embedded app messages ──────────────────────────────────────
    // Rute payloads del outbox de cada worker hacia el inbox del target.
    // - Si el sender tiene UX_EMBED: puede mandar al shell (target_tab_id=0).
    // - Si el sender es el shell (managed-by=dimension.luna + system-shell):
    //   puede mandar a cualquier tab.
    // Mensaje al destino lleva el tab_id del origen como "fromTabId".
    if !shell_message_batches.is_empty() {
        let (shell_space_id, sender_tab_ids): (Option<u32>, std::collections::HashMap<u32, u64>) = {
            let Some(specs_world) = world.get_resource::<ElemenetWorld>() else {
                return;
            };
            let shell_id = crate::ui::find_system_shell_space(&specs_world.0);
            let mut sender_map: std::collections::HashMap<u32, u64> =
                std::collections::HashMap::new();
            for (sid, _) in &shell_message_batches {
                if let Some(tid) = crate::ui::find_tab_id_for_space(&specs_world.0, *sid) {
                    sender_map.insert(*sid, tid);
                }
            }
            (shell_id, sender_map)
        };


        // Acumular routes: target_space_id → Vec<ShellMessage con fromTabId>.
        let mut routes: std::collections::HashMap<u32, Vec<js_runtime::ShellMessage>> =
            std::collections::HashMap::new();
        let mut rejected: Vec<String> = Vec::new();

        for (sender_space_id, msgs) in shell_message_batches {
            let sender_caps = capabilities_by_space
                .get(&sender_space_id)
                .copied()
                .unwrap_or_default();
            let is_shell = Some(sender_space_id) == shell_space_id;
            let has_embed = sender_caps.contains(CapabilityBits::UX_EMBED);

            if !is_shell && !has_embed {
                rejected.push(format!(
                    "[JS][space:{sender_space_id}] shell.sendMessage denied (missing UX_EMBED)"
                ));
                continue;
            }

            let sender_tab_id = sender_tab_ids.get(&sender_space_id).copied().unwrap_or(0);

            for msg in msgs {
                let target_space_id: Option<u32> = if msg.target_tab_id == 0 {
                    // Apps que mandan a tab_id=0 → shell. El shell vive en el
                    // inner space del HSML cargado por include — busca por attr.
                    shell_space_id
                } else {
                    // Shell o app que manda a otra tab. Rutea al worker del
                    // inner (donde vive la app), no al outer wrapper.
                    let Some(specs_world) = world.get_resource::<ElemenetWorld>() else {
                        return;
                    };
                    crate::ui::find_app_worker_space_by_tab_id(&specs_world.0, msg.target_tab_id)
                };

                let Some(target_space_id) = target_space_id else {
                    rejected.push(format!(
                        "[JS][space:{sender_space_id}] shell.sendMessage target tab_id={} not found",
                        msg.target_tab_id
                    ));
                    continue;
                };

                // En el inbox del destino, `target_tab_id` lo usamos como
                // "fromTabId" — quien lo originó. Convención del bus.
                routes.entry(target_space_id).or_default().push(js_runtime::ShellMessage {
                    target_tab_id: sender_tab_id,
                    payload: msg.payload,
                });
            }
        }

        if !rejected.is_empty() {
            if let Some(mut log_panel) = world.get_resource_mut::<LogPanel>() {
                for r in rejected {
                    log_panel.push_warn(r);
                }
            }
        }

        // Despachar a cada worker target.
        if !routes.is_empty() {
            let mut missing_targets: Vec<u32> = Vec::new();
            {
                let Some(mut manager) = world.get_non_send_resource_mut::<ScriptRuntimeManager>() else {
                    return;
                };
                for (target_space_id, msgs) in routes {
                    if let Some(worker) = manager.contexts.get_mut(&target_space_id) {
                        let send_result = worker
                            .cmd_tx
                            .send(JsWorkerCommand::PushShellMessages(msgs));
                        if send_result.is_ok() {
                            worker.needs_tick = true;
                        }
                    } else {
                        missing_targets.push(target_space_id);
                    }
                }
            }
            if !missing_targets.is_empty() {
                if let Some(mut log_panel) = world.get_resource_mut::<LogPanel>() {
                    for sid in missing_targets {
                        log_panel.push_warn(format!(
                            "[shell-bus] no worker for target space:{}", sid
                        ));
                    }
                }
            }
        }
    }

    if snapshot_dirty {
        if let Some(mut snapshot_state) = world.get_resource_mut::<JsSnapshotState>() {
            snapshot_state.dirty = true;
        }
    }

    if dom_structure_changed {
        if let Some(mut policies) = world.get_resource_mut::<SpacePolicies>() {
            policies.dirty = true;
        }
    }
}
