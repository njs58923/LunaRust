use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use bevy::prelude::*;
use specs::{Join, WorldExt};

use crate::dom::{collect_subtree_ids, find_nearest_ancestor_include, resolve_node_relative_url};
use crate::permissions::{CapabilityBits, SpacePolicies};
use js_runtime::Engine as JsEngine;
use virtual_dom::dom::{
    element::{Attrs, Hierarchy, Tag, Transform2},
    hsml::{Include, Model, Script},
};

use crate::{
    request_fetch_text, AttributeUpdates, DirtyNodes, ElemenetWorld, IoService, JsSnapshotState,
    LogLevel, LogPanel, ModelLoadStates, PendingJsAttachNodes, PendingModelLoads, PendingScripts,
    ReloadTrigger, ScriptLoadStates, SpaceHandleTable, SpaceHandleTables, TransformUpdates,
    VirtualDomData,
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

#[derive(Clone, Default)]
pub struct SpaceSnapshotPatch {
    pub removed_locals: Vec<i32>,
    pub attr_updates: HashMap<i32, HashMap<String, String>>,
    pub tag_updates: HashMap<i32, String>,
    pub positions: HashMap<i32, js_runtime::Vec3>,
    pub rotations: HashMap<i32, js_runtime::Vec3>,
    pub scales: HashMap<i32, js_runtime::Vec3>,
    pub global_positions: HashMap<i32, js_runtime::Vec3>,
    pub parents: HashMap<i32, i32>,
    pub children: HashMap<i32, Vec<i32>>,
}

#[derive(Clone)]
pub struct DomMirrorNode {
    pub attrs: HashMap<String, String>,
    pub tag: String,
    pub position: js_runtime::Vec3,
    pub rotation: js_runtime::Vec3,
    pub scale: js_runtime::Vec3,
    pub parent: i32,
    pub children: Vec<i32>,
}

#[derive(Resource, Default, Clone)]
pub struct DomMirror {
    pub version: u64,
    pub nodes: HashMap<i32, DomMirrorNode>,
    pub space_subtrees: HashMap<u32, HashSet<i32>>,
}

#[derive(Resource, Default)]
pub struct DomMirrorDirty {
    pub force_rebuild: bool,
    pub touched_nodes: HashSet<u32>,
    pub removed_nodes: HashSet<u32>,
}

impl DomMirrorDirty {
    pub fn force_rebuild(&mut self) {
        self.force_rebuild = true;
        self.touched_nodes.clear();
        self.removed_nodes.clear();
    }

    pub fn touch(&mut self, node_id: u32) {
        if !self.force_rebuild {
            self.touched_nodes.insert(node_id);
        }
    }

    pub fn remove(&mut self, node_id: u32) {
        if !self.force_rebuild {
            self.touched_nodes.remove(&node_id);
            self.removed_nodes.insert(node_id);
        }
    }

    fn take(&mut self) -> (bool, HashSet<u32>, HashSet<u32>) {
        let force_rebuild = self.force_rebuild;
        self.force_rebuild = false;
        (
            force_rebuild,
            std::mem::take(&mut self.touched_nodes),
            std::mem::take(&mut self.removed_nodes),
        )
    }
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
    PatchSnapshots(SpaceSnapshotPatch),
    EvalScript {
        url: String,
        code: String,
    },
    Tick {
        elapsed_ms: f64,
    },
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
    SnapshotApplied,
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

const JS_WORKER_COMMAND_CAPACITY: usize = 128;
const JS_WORKER_BACKLOG_CAPACITY: usize = 128;
const JS_WORKER_EVENT_CAPACITY: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JsWorkerQueueError {
    Full,
    Disconnected,
}

impl JsWorkerQueueError {
    pub fn is_full(self) -> bool {
        matches!(self, Self::Full)
    }
}

impl std::fmt::Display for JsWorkerQueueError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Full => f.write_str("JS worker command queue is full"),
            Self::Disconnected => f.write_str("JS worker command queue is disconnected"),
        }
    }
}

pub struct SpaceScriptWorker {
    pub cmd_tx: mpsc::SyncSender<JsWorkerCommand>,
    pub event_rx: mpsc::Receiver<JsWorkerEvent>,
    pending_commands: VecDeque<JsWorkerCommand>,
    pub snapshot_in_flight: bool,
    pub tick_in_flight: bool,
    pub needs_tick: bool,
    pub join: Option<JoinHandle<()>>,
    termination: WorkerTermination,
    pub bootstrap_scripts_enqueued: HashSet<String>,
    pub last_capabilities_bits: u64,
    /// True once `luna://internal/root_api.js` has been flushed into the worker's cmd channel.
    /// Guards any eval that calls `dimension.luna.*`.
    pub root_api_sent: bool,
}

impl SpaceScriptWorker {
    /// Encola sin bloquear el hilo principal. Si el canal está temporalmente
    /// lleno, conserva un backlog también acotado. Los streams de alta
    /// frecuencia se coalescen para retener sólo el valor más reciente.
    pub fn try_send(
        &mut self,
        command: JsWorkerCommand,
    ) -> std::result::Result<(), JsWorkerQueueError> {
        match self.flush_pending() {
            Ok(()) | Err(JsWorkerQueueError::Full) => {}
            Err(err) => return Err(err),
        }
        match self.cmd_tx.try_send(command) {
            Ok(()) => Ok(()),
            Err(mpsc::TrySendError::Disconnected(_)) => Err(JsWorkerQueueError::Disconnected),
            Err(mpsc::TrySendError::Full(command)) => self.defer_command(command),
        }
    }

    fn flush_pending(&mut self) -> std::result::Result<(), JsWorkerQueueError> {
        while let Some(command) = self.pending_commands.pop_front() {
            match self.cmd_tx.try_send(command) {
                Ok(()) => {}
                Err(mpsc::TrySendError::Full(command)) => {
                    self.pending_commands.push_front(command);
                    return Err(JsWorkerQueueError::Full);
                }
                Err(mpsc::TrySendError::Disconnected(_)) => {
                    self.pending_commands.clear();
                    return Err(JsWorkerQueueError::Disconnected);
                }
            }
        }
        Ok(())
    }

    fn defer_command(
        &mut self,
        command: JsWorkerCommand,
    ) -> std::result::Result<(), JsWorkerQueueError> {
        let replace = match &command {
            JsWorkerCommand::SetCapabilities(_) => self
                .pending_commands
                .iter()
                .rposition(|queued| matches!(queued, JsWorkerCommand::SetCapabilities(_))),
            JsWorkerCommand::SetViewerPose(_) => self
                .pending_commands
                .iter()
                .rposition(|queued| matches!(queued, JsWorkerCommand::SetViewerPose(_))),
            JsWorkerCommand::Tick { .. } => self
                .pending_commands
                .iter()
                .rposition(|queued| matches!(queued, JsWorkerCommand::Tick { .. })),
            JsWorkerCommand::PushPoseMoveEvents(_) => self.pending_commands.iter().rposition(
                |queued| matches!(queued, JsWorkerCommand::PushPoseMoveEvents(_)),
            ),
            _ => None,
        };
        if let Some(index) = replace {
            self.pending_commands[index] = command;
            return Ok(());
        }
        if self.pending_commands.len() == JS_WORKER_BACKLOG_CAPACITY {
            return Err(JsWorkerQueueError::Full);
        }
        self.pending_commands.push_back(command);
        Ok(())
    }
}

#[derive(Clone, Default)]
struct WorkerTermination {
    handle: Arc<Mutex<Option<js_runtime::ExecutionHandle>>>,
    shutdown_requested: Arc<AtomicBool>,
}

#[derive(Clone, Copy)]
struct WorkerExecutionLimits {
    eval: Duration,
    tick: Duration,
}

impl Default for WorkerExecutionLimits {
    fn default() -> Self {
        Self {
            eval: Duration::from_secs(2),
            tick: Duration::from_millis(50),
        }
    }
}

enum WatchdogCommand {
    Arm(Duration),
    Complete(mpsc::SyncSender<bool>),
    Shutdown,
}

struct ExecutionWatchdog {
    tx: mpsc::Sender<WatchdogCommand>,
    join: Option<JoinHandle<()>>,
}

impl ExecutionWatchdog {
    fn spawn(
        space_id: u32,
        execution_handle: js_runtime::ExecutionHandle,
    ) -> std::result::Result<Self, String> {
        let (tx, rx) = mpsc::channel();
        let join = thread::Builder::new()
            .name(format!("js-watchdog-{space_id}"))
            .spawn(move || watchdog_loop(rx, execution_handle))
            .map_err(|err| format!("failed to spawn JS watchdog for space {space_id}: {err}"))?;
        Ok(Self {
            tx,
            join: Some(join),
        })
    }

    fn arm(&self, budget: Duration) -> bool {
        self.tx.send(WatchdogCommand::Arm(budget)).is_ok()
    }

    fn complete(&self) -> std::result::Result<bool, String> {
        let (reply_tx, reply_rx) = mpsc::sync_channel(0);
        self.tx
            .send(WatchdogCommand::Complete(reply_tx))
            .map_err(|_| "JS watchdog disconnected".to_string())?;
        reply_rx
            .recv_timeout(Duration::from_secs(1))
            .map_err(|_| "JS watchdog did not acknowledge completion".to_string())
    }

    fn shutdown(mut self) {
        let _ = self.tx.send(WatchdogCommand::Shutdown);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

fn watchdog_loop(
    rx: mpsc::Receiver<WatchdogCommand>,
    execution_handle: js_runtime::ExecutionHandle,
) {
    'watchdog: while let Ok(command) = rx.recv() {
        match command {
            WatchdogCommand::Arm(budget) => match rx.recv_timeout(budget) {
                Ok(WatchdogCommand::Complete(reply)) => {
                    let _ = reply.send(false);
                }
                Ok(WatchdogCommand::Shutdown) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                    break;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    execution_handle.terminate_execution();
                    loop {
                        match rx.recv() {
                            Ok(WatchdogCommand::Complete(reply)) => {
                                let _ = reply.send(true);
                                break;
                            }
                            Ok(WatchdogCommand::Shutdown) | Err(_) => break 'watchdog,
                            Ok(WatchdogCommand::Arm(_)) => {}
                        }
                    }
                }
                Ok(WatchdogCommand::Arm(_)) => {}
            },
            WatchdogCommand::Shutdown => break,
            WatchdogCommand::Complete(reply) => {
                let _ = reply.send(false);
            }
        }
    }
}

#[derive(Default)]
pub struct ScriptRuntimeManager {
    pub contexts: HashMap<u32, SpaceScriptWorker>,
    /// Cambia únicamente cuando se agrega o quita un isolate. Los bridges de
    /// políticas lo usan junto con `SpacePolicies::generation` para evitar
    /// recorrer todos los spaces en cada frame.
    context_generation: u64,
}

#[derive(Resource, Default)]
struct JsPolicyBridgeState {
    capabilities_generation: Option<(u64, u64)>,
    auto_scripts_generation: Option<(u64, u64)>,
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
    spawn_space_worker_with_limits(space_id, WorkerExecutionLimits::default())
}

fn spawn_space_worker_with_limits(
    space_id: u32,
    limits: WorkerExecutionLimits,
) -> std::result::Result<SpaceScriptWorker, String> {
    let (cmd_tx, cmd_rx) = mpsc::sync_channel::<JsWorkerCommand>(JS_WORKER_COMMAND_CAPACITY);
    let (event_tx, event_rx) = mpsc::sync_channel::<JsWorkerEvent>(JS_WORKER_EVENT_CAPACITY);
    let termination = WorkerTermination::default();
    let worker_termination = termination.clone();

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

            let execution_handle = ctx.engine.execution_handle();
            if let Ok(mut handle) = worker_termination.handle.lock() {
                *handle = Some(execution_handle.clone());
            }
            let watchdog = match ExecutionWatchdog::spawn(space_id, execution_handle) {
                Ok(watchdog) => watchdog,
                Err(err) => {
                    let _ = event_tx.send(JsWorkerEvent::WorkerError(err));
                    if let Ok(mut handle) = worker_termination.handle.lock() {
                        *handle = None;
                    }
                    return;
                }
            };

            'worker: loop {
                if worker_termination.shutdown_requested.load(Ordering::Acquire) {
                    break;
                }
                let cmd = match cmd_rx.recv_timeout(Duration::from_millis(50)) {
                    Ok(cmd) => cmd,
                    Err(mpsc::RecvTimeoutError::Timeout) => continue,
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                };
                if worker_termination.shutdown_requested.load(Ordering::Acquire) {
                    break;
                }
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
                        let _ = event_tx.send(JsWorkerEvent::SnapshotApplied);
                    }
                    JsWorkerCommand::PatchSnapshots(patch) => {
                        ctx.engine.remove_snapshot_nodes(patch.removed_locals);
                        ctx.engine.patch_attr_snapshot(patch.attr_updates);
                        ctx.engine.patch_tag_snapshot(patch.tag_updates);
                        ctx.engine.patch_transform_snapshot(
                            patch.positions,
                            patch.rotations,
                            patch.scales,
                            patch.global_positions,
                        );
                        ctx.engine
                            .patch_hierarchy_snapshot(patch.parents, patch.children);
                        let _ = event_tx.send(JsWorkerEvent::SnapshotApplied);
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
                        if !watchdog.arm(limits.eval) {
                            let _ = event_tx.send(JsWorkerEvent::WorkerError(
                                "JS watchdog disconnected before eval".to_string(),
                            ));
                            break;
                        }
                        let eval_result = ctx.engine.eval(&wrapped_code);
                        let timed_out = match watchdog.complete() {
                            Ok(timed_out) => timed_out,
                            Err(err) => {
                                let _ = event_tx.send(JsWorkerEvent::WorkerError(err));
                                break;
                            }
                        };
                        if worker_termination.shutdown_requested.load(Ordering::Acquire) {
                            break;
                        }
                        if timed_out {
                            let _ = event_tx.send(JsWorkerEvent::WorkerError(format!(
                                "JS eval exceeded its {} ms execution limit ({url})",
                                limits.eval.as_millis()
                            )));
                            break;
                        }
                        match eval_result {
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
                        if !watchdog.arm(limits.tick) {
                            let _ = event_tx.send(JsWorkerEvent::WorkerError(
                                "JS watchdog disconnected before tick".to_string(),
                            ));
                            break;
                        }
                        ctx.engine.fire_raf(elapsed_ms);
                        let timed_out = match watchdog.complete() {
                            Ok(timed_out) => timed_out,
                            Err(err) => {
                                let _ = event_tx.send(JsWorkerEvent::WorkerError(err));
                                break;
                            }
                        };
                        if worker_termination.shutdown_requested.load(Ordering::Acquire) {
                            break;
                        }
                        if timed_out {
                            let _ = event_tx.send(JsWorkerEvent::WorkerError(format!(
                                "JS tick exceeded its {} ms execution limit",
                                limits.tick.as_millis()
                            )));
                            break;
                        }
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
                    JsWorkerCommand::Shutdown => break 'worker,
                }
            }

            if let Ok(mut handle) = worker_termination.handle.lock() {
                *handle = None;
            }
            watchdog.shutdown();
        })
        .map_err(|e| format!("failed to spawn JS worker for space {}: {}", space_id, e))?;

    Ok(SpaceScriptWorker {
        cmd_tx,
        event_rx,
        pending_commands: VecDeque::new(),
        snapshot_in_flight: false,
        tick_in_flight: false,
        needs_tick: true,
        join: Some(join),
        termination,
        bootstrap_scripts_enqueued: HashSet::new(),
        last_capabilities_bits: 0,
        root_api_sent: false,
    })
}

pub fn stop_space_worker(worker: &mut SpaceScriptWorker) {
    worker
        .termination
        .shutdown_requested
        .store(true, Ordering::Release);
    if let Ok(handle) = worker.termination.handle.lock() {
        if let Some(handle) = handle.as_ref() {
            handle.terminate_execution();
        }
    }
    worker.pending_commands.clear();
    let _ = worker.cmd_tx.try_send(JsWorkerCommand::Shutdown);
    if let Some(join) = worker.join.take() {
        if join.is_finished() {
            let _ = join.join();
        } else {
            // Never make Bevy's main thread wait for a JS isolate to unwind.
            // The reaper owns the join handle until V8 has finished shutting down.
            let _ = thread::Builder::new()
                .name("js-worker-reaper".to_string())
                .spawn(move || {
                    let _ = join.join();
                });
        }
    }
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

fn insert_tag_specific_components(world: &mut specs::World, entity: specs::Entity, tag_name: &str) {
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

fn build_dom_mirror_from_specs(
    specs_world: &specs::World,
    attached_node_ids: &HashSet<u32>,
    previous_version: u64,
) -> DomMirror {
    let entities = specs_world.entities();
    let attrs_storage = specs_world.read_storage::<Attrs>();
    let tags_storage = specs_world.read_storage::<Tag>();
    let transforms_storage = specs_world.read_storage::<Transform2>();
    let hierarchies_storage = specs_world.read_storage::<Hierarchy>();

    let mut nodes = HashMap::new();

    for (ent, tag) in (&entities, &tags_storage).join() {
        if !attached_node_ids.contains(&ent.id()) {
            continue;
        }

        let attrs = attrs_storage
            .get(ent)
            .map(|attrs| attrs.0.clone())
            .unwrap_or_default();
        let transform = transforms_storage.get(ent);
        let position = transform
            .map(|tr| js_runtime::Vec3 {
                x: tr.position.x,
                y: tr.position.y,
                z: tr.position.z,
            })
            .unwrap_or(js_runtime::Vec3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            });
        let rotation = transform
            .map(|tr| js_runtime::Vec3 {
                x: tr.rotation.x,
                y: tr.rotation.y,
                z: tr.rotation.z,
            })
            .unwrap_or(js_runtime::Vec3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            });
        let scale = transform
            .map(|tr| js_runtime::Vec3 {
                x: tr.scale.x,
                y: tr.scale.y,
                z: tr.scale.z,
            })
            .unwrap_or(js_runtime::Vec3 {
                x: 1.0,
                y: 1.0,
                z: 1.0,
            });
        let (parent, children) = hierarchies_storage
            .get(ent)
            .map(|hier| {
                (
                    hier.parent.map(|p| p.id() as i32).unwrap_or(-1),
                    hier.children
                        .iter()
                        .filter(|child| attached_node_ids.contains(&child.id()))
                        .map(|child| child.id() as i32)
                        .collect::<Vec<_>>(),
                )
            })
            .unwrap_or((-1, Vec::new()));

        nodes.insert(
            ent.id() as i32,
            DomMirrorNode {
                attrs,
                tag: tag.0.clone(),
                position,
                rotation,
                scale,
                parent,
                children,
            },
        );
    }

    let mut space_subtrees = HashMap::new();
    for (&node_id, node) in &nodes {
        if node.tag != "space" {
            continue;
        }
        let mut set = HashSet::new();
        let mut stack = vec![node_id];
        while let Some(curr) = stack.pop() {
            if !set.insert(curr) {
                continue;
            }
            if let Some(curr_node) = nodes.get(&curr) {
                for child in &curr_node.children {
                    stack.push(*child);
                }
            }
        }
        space_subtrees.insert(node_id as u32, set);
    }

    DomMirror {
        version: previous_version.saturating_add(1),
        nodes,
        space_subtrees,
    }
}

fn mirror_node_from_specs(specs_world: &specs::World, node_id: u32) -> Option<DomMirrorNode> {
    let entities = specs_world.entities();
    let ent = entities.entity(node_id);
    if !entities.is_alive(ent) {
        return None;
    }

    let attrs_storage = specs_world.read_storage::<Attrs>();
    let tags_storage = specs_world.read_storage::<Tag>();
    let transforms_storage = specs_world.read_storage::<Transform2>();
    let hierarchies_storage = specs_world.read_storage::<Hierarchy>();

    let tag = tags_storage.get(ent)?.0.clone();
    let attrs = attrs_storage
        .get(ent)
        .map(|attrs| attrs.0.clone())
        .unwrap_or_default();
    let transform = transforms_storage.get(ent);
    let position = transform
        .map(|tr| js_runtime::Vec3 {
            x: tr.position.x,
            y: tr.position.y,
            z: tr.position.z,
        })
        .unwrap_or(js_runtime::Vec3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        });
    let rotation = transform
        .map(|tr| js_runtime::Vec3 {
            x: tr.rotation.x,
            y: tr.rotation.y,
            z: tr.rotation.z,
        })
        .unwrap_or(js_runtime::Vec3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        });
    let scale = transform
        .map(|tr| js_runtime::Vec3 {
            x: tr.scale.x,
            y: tr.scale.y,
            z: tr.scale.z,
        })
        .unwrap_or(js_runtime::Vec3 {
            x: 1.0,
            y: 1.0,
            z: 1.0,
        });
    let (parent, children) = hierarchies_storage
        .get(ent)
        .map(|hier| {
            (
                hier.parent.map(|p| p.id() as i32).unwrap_or(-1),
                hier.children
                    .iter()
                    .map(|child| child.id() as i32)
                    .collect::<Vec<_>>(),
            )
        })
        .unwrap_or((-1, Vec::new()));

    Some(DomMirrorNode {
        attrs,
        tag,
        position,
        rotation,
        scale,
        parent,
        children,
    })
}

fn rebuild_dom_mirror_space_subtrees(mirror: &mut DomMirror) {
    mirror.space_subtrees.clear();
    for (&node_id, node) in &mirror.nodes {
        if node.tag != "space" {
            continue;
        }
        let mut set = HashSet::new();
        let mut stack = vec![node_id];
        while let Some(curr) = stack.pop() {
            if !set.insert(curr) {
                continue;
            }
            if let Some(curr_node) = mirror.nodes.get(&curr) {
                for child in &curr_node.children {
                    if mirror.nodes.contains_key(child) {
                        stack.push(*child);
                    }
                }
            }
        }
        mirror.space_subtrees.insert(node_id as u32, set);
    }
}

/// Aplica el delta (touched/removed) sobre `mirror` IN-PLACE.
///
/// Antes era funcional-pura y clonaba el mirror entero (`previous_mirror.clone()`)
/// cada frame con cambios. Con 10k nodos animados eso era un deep-clone de 10k
/// nodos (String tag + HashMap attrs + Vec children c/u) por frame — el costo
/// dominante. Mutando in-place el costo pasa a O(touched), no O(total).
fn refresh_dom_mirror_in_place(
    mirror: &mut DomMirror,
    specs_world: &specs::World,
    attached_node_ids: &HashSet<u32>,
    force_rebuild: bool,
    touched_nodes: &HashSet<u32>,
    removed_nodes: &HashSet<u32>,
) {
    if force_rebuild || mirror.nodes.is_empty() {
        *mirror = build_dom_mirror_from_specs(specs_world, attached_node_ids, mirror.version);
        return;
    }

    if touched_nodes.is_empty() && removed_nodes.is_empty() {
        return;
    }

    let mut changed = false;
    for &node_id in removed_nodes {
        let node_id_i32 = node_id as i32;
        changed |= mirror.nodes.remove(&node_id_i32).is_some();
        for node in mirror.nodes.values_mut() {
            let before = node.children.len();
            node.children.retain(|child| *child != node_id_i32);
            changed |= node.children.len() != before;
        }
    }

    for &node_id in touched_nodes {
        let node_id_i32 = node_id as i32;
        if !attached_node_ids.contains(&node_id) {
            changed |= mirror.nodes.remove(&node_id_i32).is_some();
            continue;
        }

        if let Some(mut node) = mirror_node_from_specs(specs_world, node_id) {
            node.children
                .retain(|child| attached_node_ids.contains(&(*child as u32)));
            mirror.nodes.insert(node_id_i32, node);
            changed = true;
        } else {
            changed |= mirror.nodes.remove(&node_id_i32).is_some();
        }
    }

    if changed {
        mirror.version = mirror.version.saturating_add(1);
        rebuild_dom_mirror_space_subtrees(mirror);
    }
}

fn build_local_space_snapshot_from_mirror(
    space_id: u32,
    allowed: &HashSet<i32>,
    table: &mut SpaceHandleTable,
    mirror: &DomMirror,
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
        let Some(node) = mirror.nodes.get(&global_node_id_i32) else {
            continue;
        };

        local_attr_snap.insert(local_id, node.attrs.clone());
        local_tag_snap.insert(local_id, node.tag.clone());
        local_positions.insert(local_id, node.position.clone());
        local_rotations.insert(local_id, node.rotation.clone());
        local_scales.insert(local_id, node.scale.clone());
        local_global_positions.insert(local_id, node.position.clone());

        let parent_local = if node.parent < 0 {
            -1
        } else {
            table
                .global_to_local
                .get(&(node.parent as u32))
                .copied()
                .unwrap_or(-1)
        };
        local_parents.insert(local_id, parent_local);

        let children = node
            .children
            .iter()
            .filter_map(|child_id| table.global_to_local.get(&(*child_id as u32)).copied())
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

fn build_local_space_patch_from_mirror(
    allowed: &HashSet<i32>,
    touched_globals: &HashSet<u32>,
    table: &mut SpaceHandleTable,
    mirror: &DomMirror,
) -> SpaceSnapshotPatch {
    let mut patch = SpaceSnapshotPatch::default();
    patch
        .removed_locals
        .extend(table.pending_removed_locals.iter().copied());

    for &global_node_id in touched_globals {
        let global_node_id_i32 = global_node_id as i32;
        if !allowed.contains(&global_node_id_i32) {
            continue;
        }
        // Nodo touched aún no presente en el mirror (detached / no construido):
        // se omite del patch (no se bail-ea el patch entero como antes — ese
        // bail forzaba un full snapshot por cada add). Se re-tocará al attach.
        let Some(node) = mirror.nodes.get(&global_node_id_i32) else {
            continue;
        };
        // Asignar local id si es nuevo (nodos creados desde JS ya tienen uno;
        // includes/HSML pueden no). El worker hace upsert por local id, así que
        // el nodo nuevo se agrega a su vista vía el patch.
        let local_id = ensure_local_id(table, global_node_id);

        patch.attr_updates.insert(local_id, node.attrs.clone());
        patch.tag_updates.insert(local_id, node.tag.clone());
        patch.positions.insert(local_id, node.position.clone());
        patch.rotations.insert(local_id, node.rotation.clone());
        patch.scales.insert(local_id, node.scale.clone());
        patch
            .global_positions
            .insert(local_id, node.position.clone());

        let parent_local = if node.parent < 0 {
            -1
        } else {
            table
                .global_to_local
                .get(&(node.parent as u32))
                .copied()
                .unwrap_or(-1)
        };
        patch.parents.insert(local_id, parent_local);

        let children = node
            .children
            .iter()
            .filter_map(|child_id| table.global_to_local.get(&(*child_id as u32)).copied())
            .collect::<Vec<_>>();
        patch.children.insert(local_id, children);
    }

    patch
}

pub fn find_owner_space_id(world: &specs::World, mut node: specs::Entity) -> Option<u32> {
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
    world.init_resource::<JsPolicyBridgeState>();

    let policy_generation = {
        let Some(policies) = world.get_resource::<SpacePolicies>() else {
            return;
        };
        policies.generation
    };
    let context_generation = {
        let Some(manager) = world.get_non_send_resource::<ScriptRuntimeManager>() else {
            return;
        };
        manager.context_generation
    };
    if world
        .resource::<JsPolicyBridgeState>()
        .capabilities_generation
        == Some((policy_generation, context_generation))
    {
        return;
    }

    let desired: Vec<(u32, u64)> = {
        let policies = world.resource::<SpacePolicies>();
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
                if worker
                    .try_send(JsWorkerCommand::SetCapabilities(bits))
                    .is_ok()
                {
                    worker.last_capabilities_bits = bits;
                }
            }
        }
    }

    world
        .resource_mut::<JsPolicyBridgeState>()
        .capabilities_generation = Some((policy_generation, context_generation));
}

pub fn js_auto_inject_resource_scripts_system(world: &mut World) {
    world.init_resource::<JsPolicyBridgeState>();

    let policy_generation = {
        let Some(policies) = world.get_resource::<SpacePolicies>() else {
            return;
        };
        policies.generation
    };
    let context_generation = {
        let Some(manager) = world.get_non_send_resource::<ScriptRuntimeManager>() else {
            return;
        };
        manager.context_generation
    };
    if world
        .resource::<JsPolicyBridgeState>()
        .auto_scripts_generation
        == Some((policy_generation, context_generation))
    {
        return;
    }

    let desired: Vec<(u32, Vec<String>)> = {
        let policies = world.resource::<SpacePolicies>();
        policies
            .by_space
            .iter()
            .map(|(&space_id, policy)| (space_id, policy.auto_scripts.clone()))
            .collect()
    };

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

    world
        .resource_mut::<JsPolicyBridgeState>()
        .auto_scripts_generation = Some((policy_generation, context_generation));
}

// ─── Systems ─────────────────────────────────────────────────────────────────

pub fn js_update_snapshots_system(world: &mut World) {
    let snapshot_start = Instant::now();
    let should_refresh = world
        .get_resource::<JsSnapshotState>()
        .map(|state| state.dirty)
        .unwrap_or(true);
    if !should_refresh {
        // Idle: el system salió sin trabajo. Reset de stats para que el panel
        // no muestre valores stale del último frame con actividad.
        if let Some(mut perf) = world.get_resource_mut::<crate::PerformanceStats>() {
            perf.js_snapshot_ms = 0.0;
            perf.mirror_full_rebuild = false;
            perf.snapshots_sent = 0;
            perf.waiting_on_ack = 0;
        }
        return;
    }

    // Mientras TODOS los workers esperan el ACK y no entró ningún cambio de
    // mirror, no hay nada que preparar ni enviar. Evitar aquí el hot path O(N):
    // construir `attached_node_ids` y recorrer/clonar el DomMirror sólo para
    // descubrir más abajo que ningún worker está disponible.
    let mirror_has_pending_work = world
        .get_resource::<DomMirrorDirty>()
        .map(|dirty| {
            dirty.force_rebuild
                || !dirty.touched_nodes.is_empty()
                || !dirty.removed_nodes.is_empty()
        })
        .unwrap_or(true);
    let mirror_rebuild_requested = world
        .get_resource::<JsSnapshotState>()
        .map(|state| state.mirror_force_rebuild)
        .unwrap_or(true);
    let (worker_count, workers_waiting_on_ack) = world
        .get_non_send_resource::<ScriptRuntimeManager>()
        .map(|manager| {
            (
                manager.contexts.len(),
                manager
                    .contexts
                    .values()
                    .filter(|worker| worker.snapshot_in_flight)
                    .count(),
            )
        })
        .unwrap_or_default();
    let (mirror_nodes, active_space_count) = world
        .get_resource::<DomMirror>()
        .map(|mirror| (mirror.nodes.len(), mirror.space_subtrees.len()))
        .unwrap_or_default();
    if !mirror_has_pending_work
        && !mirror_rebuild_requested
        && worker_count > 0
        && worker_count == active_space_count
        && workers_waiting_on_ack == worker_count
    {
        if let Some(mut perf) = world.get_resource_mut::<crate::PerformanceStats>() {
            perf.js_snapshot_ms = snapshot_start.elapsed().as_secs_f32() * 1000.0;
            perf.mirror_full_rebuild = false;
            perf.mirror_nodes = mirror_nodes;
            perf.snapshots_sent = 0;
            perf.waiting_on_ack = workers_waiting_on_ack;
        }
        return;
    }

    let attached_node_ids: HashSet<u32> = world
        .get_resource::<VirtualDomData>()
        .map(|dom| dom.nodes.keys().copied().collect())
        .unwrap_or_default();

    let (force_mirror_rebuild, touched_mirror_nodes, removed_mirror_nodes) = world
        .get_resource_mut::<DomMirrorDirty>()
        .map(|mut dirty| dirty.take())
        .unwrap_or((true, HashSet::new(), HashSet::new()));
    let snapshot_forces_mirror_rebuild = world
        .get_resource_mut::<JsSnapshotState>()
        .map(|mut state| {
            let force = state.mirror_force_rebuild;
            state.mirror_force_rebuild = false;
            force
        })
        .unwrap_or(true);
    let requires_full_snapshot = force_mirror_rebuild || snapshot_forces_mirror_rebuild;
    world.resource_scope(|world, mut mirror: Mut<DomMirror>| {
        sync_snapshots_with_mirror(
            world,
            &mut mirror,
            snapshot_start,
            &attached_node_ids,
            requires_full_snapshot,
            &touched_mirror_nodes,
            &removed_mirror_nodes,
        );
    });
}

fn sync_snapshots_with_mirror(
    world: &mut World,
    mirror: &mut DomMirror,
    snapshot_start: Instant,
    attached_node_ids: &HashSet<u32>,
    requires_full_snapshot: bool,
    touched_mirror_nodes: &HashSet<u32>,
    removed_mirror_nodes: &HashSet<u32>,
) {
    // El mirror se mantiene INCREMENTAL (touched/removed). Sólo se fuerza
    // full por motivos genuinos: navegación / bootstrap del recurso. Los
    // adds entran vía touched (nodo+padre en commit_pending_js_attaches) y
    // los removes vía removed, así que `node_count_changed` ya NO fuerza
    // full — antes era O(N) por cada add (10k) aunque sólo cambiaran 25.
    let Some(specs_world) = world.get_resource::<ElemenetWorld>() else {
        return;
    };
    refresh_dom_mirror_in_place(
        mirror,
        &specs_world.0,
        attached_node_ids,
        requires_full_snapshot,
        touched_mirror_nodes,
        removed_mirror_nodes,
    );
    // Red de seguridad anti-desync: si tras el refresh incremental el conteo
    // no coincide, algún productor dejó touched/removed incompletos. Full
    // rebuild UNA vez (correctness > velocidad ante bug). Si esto se queda
    // pegado en true en el panel, hay un productor que no marca dirty.
    let desynced = mirror.nodes.len() != attached_node_ids.len();
    if !requires_full_snapshot && desynced {
        refresh_dom_mirror_in_place(
            mirror,
            &specs_world.0,
            attached_node_ids,
            true,
            &HashSet::new(),
            &HashSet::new(),
        );
    }
    let requires_full_snapshot = requires_full_snapshot || desynced;

    let active_space_ids: HashSet<u32> = mirror.space_subtrees.keys().copied().collect();
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

        if !removed_contexts.is_empty() || !created_contexts.is_empty() {
            manager.context_generation = manager.context_generation.wrapping_add(1);
        }
    }

    if !removed_contexts.is_empty() || !created_contexts.is_empty() {
        if let Some(ws_service) = world.get_resource::<crate::WsService>() {
            for &space_id in &removed_contexts {
                ws_service.close_space(space_id);
            }
        }
        if let Some(io_service) = world.get_resource::<IoService>() {
            for &space_id in &removed_contexts {
                io_service.cancel_space(space_id);
            }
        }
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

    // Acumular los touched de ESTE frame en cada space (∩ su subtree permitido).
    // Garantiza que un worker `in_flight` reciba los cambios ocurridos durante
    // el ack al volver a estar ready — sin forzar un full-rebuild O(N). Durante
    // animación pura este set viene vacío (opción C), así que esto no corre.
    if !touched_mirror_nodes.is_empty() {
        if let Some(mut tables) = world.get_resource_mut::<SpaceHandleTables>() {
            for (space_id, allowed) in &mirror.space_subtrees {
                let table = ensure_space_handle_table(&mut tables, *space_id);
                for &g in touched_mirror_nodes {
                    if allowed.contains(&(g as i32)) {
                        table.pending_touched_globals.insert(g);
                    }
                }
            }
        }
    }

    let (snapshot_target_space_ids, spaces_waiting_on_ack, missing_contexts) = {
        let Some(manager) = world.get_non_send_resource::<ScriptRuntimeManager>() else {
            return;
        };
        let mut ready = Vec::new();
        let mut waiting = 0usize;
        let mut missing = false;
        for &space_id in &active_space_ids {
            match manager.contexts.get(&space_id) {
                Some(worker) if worker.snapshot_in_flight => waiting += 1,
                Some(_) => ready.push(space_id),
                None => missing = true,
            }
        }
        (ready, waiting, missing)
    };

    let mut all_snapshots_sent = true;
    if snapshot_target_space_ids.is_empty() {
        // Path "nada que enviar": típicamente todos los workers están in_flight
        // (esperando ack). Si esto corre cada frame con mirror_full_rebuild=true,
        // ése es el O(N)/frame atascado.
        if let Some(mut perf) = world.get_resource_mut::<crate::PerformanceStats>() {
            perf.js_snapshot_ms = snapshot_start.elapsed().as_secs_f32() * 1000.0;
            perf.mirror_full_rebuild = requires_full_snapshot;
            perf.mirror_nodes = mirror.nodes.len();
            perf.snapshots_sent = 0;
            perf.waiting_on_ack = spaces_waiting_on_ack;
        }
        let keep_dirty = spaces_waiting_on_ack > 0 || missing_contexts || !errors.is_empty();
        if let Some(mut snapshot_state) = world.get_resource_mut::<JsSnapshotState>() {
            // Sin force-rebuild: el delta queda acumulado por space (arriba) y
            // se manda como patch cuando el worker vuelva a ready.
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
        return;
    }

    // Full SÓLO por force genuino (nav/desync). El estar esperando ack ya NO
    // fuerza full: los cambios acumulados van como patch.
    let dispatch_full_snapshots = requires_full_snapshot;

    enum SnapshotBatch {
        Full(SpaceSnapshots),
        Patch(SpaceSnapshotPatch),
    }

    let snapshot_batches = {
        let Some(mut space_handle_tables) = world.get_resource_mut::<SpaceHandleTables>() else {
            return;
        };
        let mut snapshot_batches = Vec::new();
        for space_id in &snapshot_target_space_ids {
            let Some(allowed) = mirror.space_subtrees.get(space_id) else {
                continue;
            };
            let table = ensure_space_handle_table(&mut space_handle_tables, *space_id);
            // Full si: force genuino, worker recién creado, o el worker aún no
            // recibió su snapshot full inicial (bootstrap). Si no → patch del
            // delta acumulado (O(Δ)).
            let needs_full = dispatch_full_snapshots
                || created_contexts.contains(space_id)
                || !table.bootstrapped;
            let batch = if needs_full {
                SnapshotBatch::Full(build_local_space_snapshot_from_mirror(
                    *space_id, allowed, table, &mirror,
                ))
            } else {
                // Patch desde el set acumulado (este frame + lo retenido durante
                // el ack). Clonado para poder limpiarlo recién al confirmar envío.
                let pending: HashSet<u32> = table.pending_touched_globals.clone();
                SnapshotBatch::Patch(build_local_space_patch_from_mirror(
                    allowed, &pending, table, &mirror,
                ))
            };
            snapshot_batches.push((*space_id, batch));
        }
        snapshot_batches
    };

    let mut snapshots_sent_this_run = 0usize;
    let mut sent_snapshot_space_ids = Vec::new();
    {
        let Some(mut manager) = world.get_non_send_resource_mut::<ScriptRuntimeManager>() else {
            return;
        };
        let mut broken_contexts = Vec::new();
        for (space_id, batch) in snapshot_batches {
            let Some(worker) = manager.contexts.get_mut(&space_id) else {
                continue;
            };
            if worker.snapshot_in_flight {
                all_snapshots_sent = false;
                continue;
            }
            let send_result = match batch {
                SnapshotBatch::Full(snap) => {
                    worker.try_send(JsWorkerCommand::UpdateSnapshots(snap))
                }
                SnapshotBatch::Patch(patch) => {
                    worker.try_send(JsWorkerCommand::PatchSnapshots(patch))
                }
            };
            if let Err(e) = send_result {
                all_snapshots_sent = false;
                if e.is_full() {
                    // El delta permanece en SpaceHandleTable y se reintenta en
                    // el siguiente frame sin bloquear ni perder cambios.
                    continue;
                }
                errors.push(format!(
                    "failed to send snapshots to space {}: {}",
                    space_id, e
                ));
                broken_contexts.push(space_id);
            } else {
                worker.snapshot_in_flight = true;
                snapshots_sent_this_run += 1;
                sent_snapshot_space_ids.push(space_id);
            }
        }

        let mut removed_broken_context = false;
        for space_id in broken_contexts {
            if let Some(mut worker) = manager.contexts.remove(&space_id) {
                stop_space_worker(&mut worker);
                removed_contexts.push(space_id);
                removed_broken_context = true;
            }
        }
        if removed_broken_context {
            manager.context_generation = manager.context_generation.wrapping_add(1);
        }
    }

    if !sent_snapshot_space_ids.is_empty() {
        if let Some(mut space_handle_tables) = world.get_resource_mut::<SpaceHandleTables>() {
            for space_id in sent_snapshot_space_ids {
                if let Some(table) = space_handle_tables.by_space.get_mut(&space_id) {
                    table.pending_removed_locals.clear();
                    // El delta acumulado ya viajó (en el patch o subsumido por el
                    // full). Limpiar para no re-enviarlo.
                    table.pending_touched_globals.clear();
                    // Primer envío = full → worker bootstrapeado. Idempotente.
                    table.bootstrapped = true;
                }
            }
        }
    }

    if !removed_contexts.is_empty() {
        if let Some(ws_service) = world.get_resource::<crate::WsService>() {
            for &space_id in &removed_contexts {
                ws_service.close_space(space_id);
            }
        }
        if let Some(io_service) = world.get_resource::<IoService>() {
            for &space_id in &removed_contexts {
                io_service.cancel_space(space_id);
            }
        }
        let Some(mut space_handle_tables) = world.get_resource_mut::<SpaceHandleTables>() else {
            return;
        };
        for &space_id in &removed_contexts {
            space_handle_tables.by_space.remove(&space_id);
        }
    }

    if let Some(mut perf) = world.get_resource_mut::<crate::PerformanceStats>() {
        perf.js_snapshot_ms = snapshot_start.elapsed().as_secs_f32() * 1000.0;
        perf.mirror_full_rebuild = requires_full_snapshot;
        perf.mirror_nodes = mirror.nodes.len();
        perf.snapshots_sent = snapshots_sent_this_run;
        perf.waiting_on_ack = spaces_waiting_on_ack;
    }

    let keep_dirty = snapshots_sent_this_run > 0
        || spaces_waiting_on_ack > 0
        || missing_contexts
        || !all_snapshots_sent
        || !errors.is_empty();
    if let Some(mut snapshot_state) = world.get_resource_mut::<JsSnapshotState>() {
        // Ya NO se fuerza full-rebuild al esperar ack: los cambios ocurridos
        // durante el in_flight quedan acumulados en `pending_touched_globals`
        // por space y se mandan como patch al volver a ready.
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

    enum ScriptEnqueueResult {
        Queued,
        DeferUntilContext,
        Error(String),
    }

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

        let enqueue_result = {
            let Some(mut manager) = world.get_non_send_resource_mut::<ScriptRuntimeManager>()
            else {
                return;
            };
            if let Some(worker) = manager.contexts.get_mut(&space_id) {
                let result = worker
                    .try_send(JsWorkerCommand::EvalScript {
                        url: url.clone(),
                        code: code.clone(),
                    })
                    .map_err(|e| e);
                if result.is_ok() && url == "luna://internal/root_api.js" {
                    worker.root_api_sent = true;
                }
                if result.is_ok() {
                    worker.needs_tick = true;
                }
                match result {
                    Ok(_) => ScriptEnqueueResult::Queued,
                    Err(err) if err.is_full() => ScriptEnqueueResult::DeferUntilContext,
                    Err(err) => ScriptEnqueueResult::Error(err.to_string()),
                }
            } else {
                let space_still_attached = world
                    .get_resource::<VirtualDomData>()
                    .map(|dom| dom.nodes.contains_key(&space_id))
                    .unwrap_or(false);
                if space_still_attached {
                    ScriptEnqueueResult::DeferUntilContext
                } else {
                    ScriptEnqueueResult::Error(format!("missing JS context for space {}", space_id))
                }
            }
        };

        match enqueue_result {
            ScriptEnqueueResult::Queued => {
                queued_this_frame += 1;
                log_messages.push(crate::LogEntry::new(
                    LogLevel::Info,
                    format!("[JS][space:{}] Script queued: {}", space_id, url),
                ))
            }
            ScriptEnqueueResult::DeferUntilContext => deferred_scripts.push((space_id, url, code)),
            ScriptEnqueueResult::Error(err) => log_messages.push(crate::LogEntry::new(
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
    let mut removed_worker_space_ids = Vec::new();
    let mut snapshot_acks = 0usize;
    let mut contexts_removed = false;
    let pending_snapshot_in_flight;

    {
        let Some(mut manager) = world.get_non_send_resource_mut::<ScriptRuntimeManager>() else {
            return;
        };

        let mut broken_contexts = Vec::new();
        for (space_id, worker) in manager.contexts.iter_mut() {
            if matches!(
                worker.flush_pending(),
                Err(JsWorkerQueueError::Disconnected)
            ) {
                broken_contexts.push(*space_id);
                continue;
            }
            if worker.tick_in_flight || !worker.needs_tick {
                continue;
            }
            match worker.try_send(JsWorkerCommand::Tick { elapsed_ms }) {
                Ok(_) => worker.tick_in_flight = true,
                Err(err) if err.is_full() => {}
                Err(_) => broken_contexts.push(*space_id),
            }
        }

        for (space_id, worker) in manager.contexts.iter_mut() {
            loop {
                match worker.event_rx.try_recv() {
                    Ok(JsWorkerEvent::SnapshotApplied) => {
                        worker.snapshot_in_flight = false;
                        snapshot_acks += 1;
                    }
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
                removed_worker_space_ids.push(space_id);
                contexts_removed = true;
            }
        }

        if contexts_removed {
            manager.context_generation = manager.context_generation.wrapping_add(1);
        }

        pending_snapshot_in_flight = manager
            .contexts
            .values()
            .any(|worker| worker.snapshot_in_flight);
    }

    if let Some(ws_service) = world.get_resource::<crate::WsService>() {
        for &space_id in &removed_worker_space_ids {
            ws_service.close_space(space_id);
        }
    }
    if let Some(io_service) = world.get_resource::<IoService>() {
        for space_id in removed_worker_space_ids {
            io_service.cancel_space(space_id);
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
                    format!(
                        "[JS][space:{}] Script already loaded, skipping: {}",
                        space_id, url
                    ),
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

    let dom_structure_changed = !creation_batches.is_empty() || !hierarchy_batches.is_empty();
    let dom_removals_changed = !remove_batches.is_empty();

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
        transform_updates
            .positions
            .extend(validated_position_updates);
        transform_updates
            .rotations
            .extend(validated_rotation_updates);
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
                let send_result = worker.try_send(JsWorkerCommand::PushElementCreationResults(
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
            pending_js_attaches
                .0
                .extend(newly_attached_ids.iter().copied());
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
        // Tocar el mirror para los nodos re-parentados YA attached: no pasan por
        // commit_pending_js_attaches (que cubre los newly_attached), así que sin
        // esto su jerarquía quedaría stale al quitar el force-rebuild blanket.
        if !dirty_ids_vec.is_empty() {
            if let Some(mut mirror_dirty) = world.get_resource_mut::<DomMirrorDirty>() {
                for &nid in &dirty_ids_vec {
                    mirror_dirty.touch(nid);
                }
            }
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
    let mut frame_deleted_global: std::collections::HashSet<u32> = std::collections::HashSet::new();
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
            for table in space_handle_tables.by_space.values_mut() {
                for &nid in &all_removed_ids {
                    if let Some(local_id) = table.global_to_local.remove(&nid) {
                        table.local_to_global.remove(&local_id);
                        table.pending_removed_locals.insert(local_id);
                    }
                    table.detached_globals.remove(&nid);
                }
            }
        }
        if let Some(mut mirror_dirty) = world.get_resource_mut::<DomMirrorDirty>() {
            for &nid in &all_removed_ids {
                mirror_dirty.remove(nid);
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
        let mut rejected = Vec::new();
        for (request_id, url) in &fetch_queue {
            if let Err(error) =
                request_fetch_text(&tokio_rt.0, &io_service, space_id, *request_id, url.clone())
            {
                rejected.push((*request_id, Err(error)));
            }
        }
        if !rejected.is_empty() {
            let rejected_count = rejected.len();
            if let Some(mut manager) = world.get_non_send_resource_mut::<ScriptRuntimeManager>() {
                if let Some(worker) = manager.contexts.get_mut(&space_id) {
                    if worker
                        .try_send(JsWorkerCommand::PushFetchResults(rejected))
                        .is_ok()
                    {
                        worker.needs_tick = true;
                    }
                }
            }
            if let Some(mut log_panel) = world.get_resource_mut::<LogPanel>() {
                log_panel.push_warn(format!(
                    "[JS][space:{space_id}] rejected {rejected_count} fetch request(s): backpressure limit"
                ));
            }
        }
        if let Some(mut log_panel) = world.get_resource_mut::<LogPanel>() {
            for (_, url) in &fetch_queue {
                log_panel.push_info(format!("[JS][space:{}] fetch queued: {}", space_id, url));
            }
        }
    }

    // WebSocket: abre conexiones, encola sends, cierra. El transporte real vive
    // en `ws.rs`; aquí sólo encaminamos las colas drenadas del engine.
    let mut ws_send_failures = Vec::new();
    if !ws_connect_batches.is_empty() || !ws_send_batches.is_empty() || !ws_close_batches.is_empty()
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
                if let Err(err) = crate::ws::ws_send(ws_service, space_id, conn_id, msg) {
                    ws_send_failures.push((space_id, conn_id, err.to_string()));
                    if matches!(
                        err,
                        crate::ws::WsSendError::QueueFull
                            | crate::ws::WsSendError::Disconnected
                    ) {
                        crate::ws::ws_close(ws_service, space_id, conn_id);
                    }
                }
            }
        }
        for (space_id, queue) in ws_close_batches {
            for conn_id in queue {
                crate::ws::ws_close(ws_service, space_id, conn_id);
            }
        }
    }
    if !ws_send_failures.is_empty() {
        if let Some(mut manager) = world.get_non_send_resource_mut::<ScriptRuntimeManager>() {
            for (space_id, conn_id, error) in ws_send_failures {
                if let Some(worker) = manager.contexts.get_mut(&space_id) {
                    let result = worker.try_send(JsWorkerCommand::PushWsEvents(vec![
                        WsWorkerEvent::Status {
                            conn_id,
                            status: format!("error: {error}"),
                        },
                    ]));
                    if result.is_ok() {
                        worker.needs_tick = true;
                    }
                }
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
                    plan_navigation_for_space(
                        &specs_world.0,
                        &current_url,
                        space_id,
                        &requested_url,
                        caps,
                    ),
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
                    log_panel
                        .push_warn(format!("[JS][space:{}] global navigate: {}", space_id, url));
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
                        } else if let Some(mut log_panel) = world.get_resource_mut::<LogPanel>() {
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
                        } else if let Some(mut log_panel) = world.get_resource_mut::<LogPanel>() {
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
                        } else if let Some(mut log_panel) = world.get_resource_mut::<LogPanel>() {
                            log_panel.push_warn(format!(
                                "[JS][space:{}] tabs.setVisible denied (missing UPDATE_ROOT_SPACE)",
                                space_id
                            ));
                        }
                    }
                    js_runtime::TabAction::SetPose {
                        tab_id,
                        px,
                        py,
                        pz,
                        rx,
                        ry,
                        rz,
                    } => {
                        let caps = capabilities_by_space
                            .get(&space_id)
                            .copied()
                            .unwrap_or_default();
                        if caps.contains(CapabilityBits::UPDATE_ROOT_SPACE) {
                            pose_requests.push((space_id, tab_id, [px, py, pz, rx, ry, rz]));
                        } else if let Some(mut log_panel) = world.get_resource_mut::<LogPanel>() {
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
            if let Some(mut unmount_queue) = world.get_resource_mut::<crate::SpaceUnmountQueue>() {
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
                if let Some(mut manager) = world.get_non_send_resource_mut::<ScriptRuntimeManager>()
                {
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
                                let send_result = worker.try_send(JsWorkerCommand::EvalScript {
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
                                let send_result = worker.try_send(JsWorkerCommand::EvalScript {
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
                routes
                    .entry(target_space_id)
                    .or_default()
                    .push(js_runtime::ShellMessage {
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
                let Some(mut manager) = world.get_non_send_resource_mut::<ScriptRuntimeManager>()
                else {
                    return;
                };
                for (target_space_id, msgs) in routes {
                    if let Some(worker) = manager.contexts.get_mut(&target_space_id) {
                        let send_result = worker.try_send(JsWorkerCommand::PushShellMessages(msgs));
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
                        log_panel
                            .push_warn(format!("[shell-bus] no worker for target space:{}", sid));
                    }
                }
            }
        }
    }

    if snapshot_dirty || contexts_removed || pending_snapshot_in_flight {
        if let Some(mut snapshot_state) = world.get_resource_mut::<JsSnapshotState>() {
            snapshot_state.dirty = true;
        }
    } else if snapshot_acks > 0 {
        // Al ack-ear, mantener dirty si quedó algún delta pendiente por enviar
        // (touched/removed acumulado mientras el worker estaba in_flight) o un
        // rebuild forzado. Antes esto dependía sólo de `mirror_force_rebuild`;
        // con la acumulación per-space (opción A) hay que chequear los deltas.
        let has_pending_delta = world
            .get_resource::<SpaceHandleTables>()
            .map(|t| {
                t.by_space.values().any(|tbl| {
                    !tbl.pending_touched_globals.is_empty()
                        || !tbl.pending_removed_locals.is_empty()
                })
            })
            .unwrap_or(false);
        if let Some(mut snapshot_state) = world.get_resource_mut::<JsSnapshotState>() {
            snapshot_state.dirty = has_pending_delta || snapshot_state.mirror_force_rebuild;
        }
    }

    if dom_structure_changed || dom_removals_changed {
        if let Some(mut policies) = world.get_resource_mut::<SpacePolicies>() {
            policies.dirty = true;
        }
    }

    // NOTA: antes acá se forzaba `mirror_force_rebuild` + `force_rebuild()` ante
    // CUALQUIER cambio estructural → full rebuild O(N) del mirror por cada add
    // (25 cubos sobre 10k = full de 10k). Ahora el mirror se actualiza
    // incremental: newly_attached vía commit_pending_js_attaches (nodo+padre),
    // re-parents vía el touch de `dirty_ids_vec` arriba, y removes vía
    // `mirror_dirty.remove`. La red de seguridad anti-desync en
    // `js_update_snapshots_system` reconstruye full si algo quedó incompleto.
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::prelude::{App, Time};
    use specs::WorldExt;
    use virtual_dom::dom::element::{build_world, Vec3 as DomVec3};

    fn snapshot_test_app_with_spaces(space_count: usize) -> (App, Vec<u32>) {
        let mut app = App::new();
        app.insert_resource(JsSnapshotState::default());
        app.insert_resource(VirtualDomData::default());
        app.insert_resource(SpaceHandleTables::default());
        app.insert_resource(LogPanel::default());
        app.insert_resource(Time::<()>::default());
        app.insert_resource(AttributeUpdates::default());
        app.insert_resource(TransformUpdates::default());
        app.insert_resource(DirtyNodes::default());
        app.insert_resource(PendingJsAttachNodes::default());
        app.insert_resource(DomMirror::default());
        app.insert_resource(DomMirrorDirty::default());
        app.insert_resource(crate::PerformanceStats::default());
        app.insert_non_send_resource(ScriptRuntimeManager::default());

        let specs_world = build_world();
        let mut space_ids = Vec::with_capacity(space_count);
        for _ in 0..space_count {
            let space_ent = specs_world.entities().create();
            let space_id = space_ent.id();
            {
                let mut tags = specs_world.write_storage::<Tag>();
                let mut attrs = specs_world.write_storage::<Attrs>();
                let mut transforms = specs_world.write_storage::<Transform2>();
                let mut hier = specs_world.write_storage::<Hierarchy>();
                tags.insert(space_ent, Tag("space".into())).ok();
                attrs.insert(space_ent, Attrs(HashMap::new())).ok();
                transforms
                    .insert(
                        space_ent,
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
                hier.insert(
                    space_ent,
                    Hierarchy {
                        parent: None,
                        children: vec![],
                    },
                )
                .ok();
            }
            app.world_mut()
                .resource_mut::<VirtualDomData>()
                .nodes
                .insert(space_id, space_ent);
            space_ids.push(space_id);
        }

        app.insert_resource(ElemenetWorld(specs_world));
        (app, space_ids)
    }

    fn snapshot_test_app() -> (App, u32) {
        let (app, space_ids) = snapshot_test_app_with_spaces(1);
        (app, space_ids[0])
    }

    fn snapshot_test_app_with_child() -> (App, u32, u32, specs::Entity, specs::Entity) {
        let mut app = App::new();
        app.insert_resource(JsSnapshotState::default());
        app.insert_resource(VirtualDomData::default());
        app.insert_resource(SpaceHandleTables::default());
        app.insert_resource(LogPanel::default());
        app.insert_resource(Time::<()>::default());
        app.insert_resource(AttributeUpdates::default());
        app.insert_resource(TransformUpdates::default());
        app.insert_resource(DirtyNodes::default());
        app.insert_resource(PendingJsAttachNodes::default());
        app.insert_resource(DomMirror::default());
        app.insert_resource(DomMirrorDirty::default());
        app.insert_non_send_resource(ScriptRuntimeManager::default());

        let specs_world = build_world();
        let space_ent = specs_world.entities().create();
        let child_ent = specs_world.entities().create();
        {
            let mut tags = specs_world.write_storage::<Tag>();
            let mut attrs = specs_world.write_storage::<Attrs>();
            let mut transforms = specs_world.write_storage::<Transform2>();
            let mut hier = specs_world.write_storage::<Hierarchy>();

            tags.insert(space_ent, Tag("space".into())).ok();
            tags.insert(child_ent, Tag("box".into())).ok();
            attrs.insert(space_ent, Attrs(HashMap::new())).ok();
            attrs.insert(child_ent, Attrs(HashMap::new())).ok();
            transforms
                .insert(
                    space_ent,
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
            transforms
                .insert(
                    child_ent,
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
            hier.insert(
                space_ent,
                Hierarchy {
                    parent: None,
                    children: vec![child_ent],
                },
            )
            .ok();
            hier.insert(
                child_ent,
                Hierarchy {
                    parent: Some(space_ent),
                    children: vec![],
                },
            )
            .ok();
        }

        app.world_mut()
            .resource_mut::<VirtualDomData>()
            .nodes
            .extend([(space_ent.id(), space_ent), (child_ent.id(), child_ent)]);
        app.insert_resource(ElemenetWorld(specs_world));
        (app, space_ent.id(), child_ent.id(), space_ent, child_ent)
    }

    #[test]
    fn dom_mirror_builds_full_space_snapshot_equivalent_shape() {
        let specs_world = build_world();
        let space_ent = specs_world.entities().create();
        let child_ent = specs_world.entities().create();
        {
            let mut tags = specs_world.write_storage::<Tag>();
            let mut attrs = specs_world.write_storage::<Attrs>();
            let mut transforms = specs_world.write_storage::<Transform2>();
            let mut hier = specs_world.write_storage::<Hierarchy>();

            tags.insert(space_ent, Tag("space".into())).ok();
            tags.insert(child_ent, Tag("box".into())).ok();

            attrs.insert(space_ent, Attrs(HashMap::new())).ok();
            attrs
                .insert(
                    child_ent,
                    Attrs(HashMap::from([("color".into(), "#fff".into())])),
                )
                .ok();

            transforms
                .insert(
                    space_ent,
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
            transforms
                .insert(
                    child_ent,
                    Transform2 {
                        position: DomVec3 {
                            x: 1.0,
                            y: 2.0,
                            z: 3.0,
                        },
                        rotation: DomVec3 {
                            x: 0.1,
                            y: 0.2,
                            z: 0.3,
                        },
                        scale: DomVec3 {
                            x: 4.0,
                            y: 5.0,
                            z: 6.0,
                        },
                    },
                )
                .ok();

            hier.insert(
                space_ent,
                Hierarchy {
                    parent: None,
                    children: vec![child_ent],
                },
            )
            .ok();
            hier.insert(
                child_ent,
                Hierarchy {
                    parent: Some(space_ent),
                    children: vec![],
                },
            )
            .ok();
        }

        let attached = HashSet::from([space_ent.id(), child_ent.id()]);
        let mirror = build_dom_mirror_from_specs(&specs_world, &attached, 7);
        let allowed = mirror
            .space_subtrees
            .get(&space_ent.id())
            .expect("space subtree should exist");
        let mut table = SpaceHandleTable {
            next_local_id: 1,
            ..Default::default()
        };
        let snap =
            build_local_space_snapshot_from_mirror(space_ent.id(), allowed, &mut table, &mirror);

        let child_local = table
            .global_to_local
            .get(&child_ent.id())
            .copied()
            .expect("child should get a local id");

        assert_eq!(mirror.version, 8);
        assert_eq!(snap.tag_snap.get(&0).map(String::as_str), Some("space"));
        assert_eq!(
            snap.tag_snap.get(&child_local).map(String::as_str),
            Some("box")
        );
        assert_eq!(
            snap.attr_snap
                .get(&child_local)
                .and_then(|attrs| attrs.get("color"))
                .map(String::as_str),
            Some("#fff")
        );
        assert_eq!(snap.parents.get(&child_local), Some(&0));
        assert_eq!(snap.children.get(&0), Some(&vec![child_local]));
        assert_eq!(
            snap.positions.get(&child_local).map(|p| (p.x, p.y, p.z)),
            Some((1.0, 2.0, 3.0))
        );
    }

    fn fake_worker(
        needs_tick: bool,
    ) -> (
        SpaceScriptWorker,
        std::sync::mpsc::Receiver<JsWorkerCommand>,
        std::sync::mpsc::SyncSender<JsWorkerEvent>,
    ) {
        let (cmd_tx, cmd_rx) =
            std::sync::mpsc::sync_channel::<JsWorkerCommand>(JS_WORKER_COMMAND_CAPACITY);
        let (event_tx, event_rx) =
            std::sync::mpsc::sync_channel::<JsWorkerEvent>(JS_WORKER_EVENT_CAPACITY);
        (
            SpaceScriptWorker {
                cmd_tx,
                event_rx,
                pending_commands: VecDeque::new(),
                snapshot_in_flight: false,
                tick_in_flight: false,
                needs_tick,
                join: None,
                termination: WorkerTermination::default(),
                bootstrap_scripts_enqueued: HashSet::new(),
                last_capabilities_bits: 0,
                root_api_sent: false,
            },
            cmd_rx,
            event_tx,
        )
    }

    #[test]
    fn capability_bridge_only_rescans_after_generation_change() {
        let space_id = 41;
        let (worker, cmd_rx, _event_tx) = fake_worker(false);
        let mut manager = ScriptRuntimeManager::default();
        manager.contexts.insert(space_id, worker);

        let mut policies = SpacePolicies::default();
        policies.generation = 7;
        policies.by_space.insert(
            space_id,
            crate::permissions::SpacePolicy {
                effective_caps: CapabilityBits::FETCH_TEXT,
                ..Default::default()
            },
        );

        let mut app = App::new();
        app.insert_resource(policies);
        app.insert_non_send_resource(manager);

        js_sync_space_permissions_system(app.world_mut());
        assert!(matches!(
            cmd_rx.try_recv(),
            Ok(JsWorkerCommand::SetCapabilities(bits))
                if bits == CapabilityBits::FETCH_TEXT.bits()
        ));

        app.world_mut()
            .resource_mut::<SpacePolicies>()
            .by_space
            .get_mut(&space_id)
            .unwrap()
            .effective_caps = CapabilityBits::NAVIGATE_SELF;
        js_sync_space_permissions_system(app.world_mut());
        assert!(matches!(cmd_rx.try_recv(), Err(mpsc::TryRecvError::Empty)));

        app.world_mut().resource_mut::<SpacePolicies>().generation += 1;
        js_sync_space_permissions_system(app.world_mut());
        assert!(matches!(
            cmd_rx.try_recv(),
            Ok(JsWorkerCommand::SetCapabilities(bits))
                if bits == CapabilityBits::NAVIGATE_SELF.bits()
        ));
    }

    #[test]
    fn auto_script_bridge_only_rescans_after_policy_or_worker_change() {
        let space_id = 52;
        let (worker, _cmd_rx, _event_tx) = fake_worker(false);
        let mut manager = ScriptRuntimeManager::default();
        manager.contexts.insert(space_id, worker);

        let mut policies = SpacePolicies::default();
        policies.generation = 3;
        policies.by_space.insert(
            space_id,
            crate::permissions::SpacePolicy {
                auto_scripts: vec!["luna://internal/viewer_pose_api.js".to_string()],
                ..Default::default()
            },
        );

        let mut app = App::new();
        app.insert_resource(policies);
        app.insert_resource(PendingScripts::default());
        app.insert_non_send_resource(manager);

        js_auto_inject_resource_scripts_system(app.world_mut());
        assert_eq!(app.world().resource::<PendingScripts>().0.len(), 1);
        app.world_mut().resource_mut::<PendingScripts>().0.clear();
        app.world_mut()
            .non_send_resource_mut::<ScriptRuntimeManager>()
            .contexts
            .get_mut(&space_id)
            .unwrap()
            .bootstrap_scripts_enqueued
            .clear();

        js_auto_inject_resource_scripts_system(app.world_mut());
        assert!(app.world().resource::<PendingScripts>().0.is_empty());

        app.world_mut()
            .non_send_resource_mut::<ScriptRuntimeManager>()
            .context_generation += 1;
        js_auto_inject_resource_scripts_system(app.world_mut());
        assert_eq!(app.world().resource::<PendingScripts>().0.len(), 1);
    }

    #[test]
    fn worker_command_queue_is_bounded_and_non_blocking() {
        let (mut worker, _cmd_rx, _event_tx) = fake_worker(false);

        for _ in 0..(JS_WORKER_COMMAND_CAPACITY + JS_WORKER_BACKLOG_CAPACITY) {
            worker
                .try_send(JsWorkerCommand::PushShellMessages(Vec::new()))
                .expect("queue should accept commands up to its capacity");
        }

        assert_eq!(
            worker.try_send(JsWorkerCommand::PushShellMessages(Vec::new())),
            Err(JsWorkerQueueError::Full)
        );
    }

    #[test]
    fn saturated_worker_coalesces_high_frequency_commands() {
        let (mut worker, _cmd_rx, _event_tx) = fake_worker(false);
        for _ in 0..JS_WORKER_COMMAND_CAPACITY {
            worker
                .try_send(JsWorkerCommand::PushShellMessages(Vec::new()))
                .unwrap();
        }

        worker
            .try_send(JsWorkerCommand::SetViewerPose(None))
            .unwrap();
        worker
            .try_send(JsWorkerCommand::SetViewerPose(None))
            .unwrap();

        assert_eq!(worker.pending_commands.len(), 1);
        assert!(matches!(
            worker.pending_commands.front(),
            Some(JsWorkerCommand::SetViewerPose(None))
        ));
    }

    fn wait_for_worker_error(
        worker: &SpaceScriptWorker,
        timeout: Duration,
    ) -> Option<String> {
        let deadline = Instant::now() + timeout;
        while let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
            match worker.event_rx.recv_timeout(remaining) {
                Ok(JsWorkerEvent::WorkerError(err)) => return Some(err),
                Ok(_) => {}
                Err(_) => return None,
            }
        }
        None
    }

    #[test]
    fn infinite_eval_is_terminated_at_its_execution_limit() {
        let mut worker = spawn_space_worker_with_limits(
            90_001,
            WorkerExecutionLimits {
                eval: Duration::from_millis(100),
                tick: Duration::from_millis(100),
            },
        )
        .expect("worker should spawn");

        worker
            .cmd_tx
            .send(JsWorkerCommand::EvalScript {
                url: "eval://infinite".to_string(),
                code: "for (;;) {}".to_string(),
            })
            .expect("infinite eval should be queued");

        let error = wait_for_worker_error(&worker, Duration::from_secs(5));
        stop_space_worker(&mut worker);
        let error = error.expect("watchdog should report the execution limit");
        assert!(error.contains("eval exceeded its 100 ms execution limit"));
    }

    #[test]
    fn infinite_animation_frame_is_terminated_at_the_tick_limit() {
        let mut worker = spawn_space_worker_with_limits(
            90_002,
            WorkerExecutionLimits {
                eval: Duration::from_secs(1),
                tick: Duration::from_millis(100),
            },
        )
        .expect("worker should spawn");

        worker
            .cmd_tx
            .send(JsWorkerCommand::EvalScript {
                url: "eval://raf-infinite".to_string(),
                code: "requestAnimationFrame(() => { for (;;) {} });".to_string(),
            })
            .expect("RAF registration should be queued");
        match worker.event_rx.recv_timeout(Duration::from_secs(5)) {
            Ok(JsWorkerEvent::EvalResult { error: None, .. }) => {}
            _ => panic!("RAF registration should complete successfully"),
        }

        worker
            .cmd_tx
            .send(JsWorkerCommand::Tick { elapsed_ms: 16.0 })
            .expect("tick should be queued");
        let error = wait_for_worker_error(&worker, Duration::from_secs(5));
        stop_space_worker(&mut worker);
        let error = error.expect("watchdog should report the tick limit");
        assert!(error.contains("tick exceeded its 100 ms execution limit"));
    }

    #[test]
    fn stopping_worker_interrupts_running_js_without_waiting_on_main_thread() {
        let mut worker = spawn_space_worker_with_limits(
            90_003,
            WorkerExecutionLimits {
                eval: Duration::from_secs(30),
                tick: Duration::from_secs(30),
            },
        )
        .expect("worker should spawn");

        worker
            .cmd_tx
            .send(JsWorkerCommand::EvalScript {
                url: "eval://ready".to_string(),
                code: "globalThis.__workerReady = true;".to_string(),
            })
            .expect("readiness eval should be queued");
        match worker.event_rx.recv_timeout(Duration::from_secs(5)) {
            Ok(JsWorkerEvent::EvalResult { error: None, .. }) => {}
            _ => panic!("worker should finish its readiness eval"),
        }

        worker
            .cmd_tx
            .send(JsWorkerCommand::EvalScript {
                url: "eval://shutdown-infinite".to_string(),
                code: "for (;;) {}".to_string(),
            })
            .expect("infinite eval should be queued");
        thread::sleep(Duration::from_millis(50));

        let started = Instant::now();
        stop_space_worker(&mut worker);
        assert!(
            started.elapsed() < Duration::from_millis(500),
            "stop must not join a running JS worker on the caller thread"
        );
        assert!(
            matches!(
                worker.event_rx.recv_timeout(Duration::from_secs(2)),
                Err(mpsc::RecvTimeoutError::Disconnected)
            ),
            "interrupting the isolate should let the worker exit promptly"
        );
    }

    #[test]
    fn snapshots_stay_dirty_until_worker_ack_then_clear() {
        let (mut app, space_id) = snapshot_test_app();
        let (worker, cmd_rx, event_tx) = fake_worker(false);
        app.world_mut()
            .non_send_resource_mut::<ScriptRuntimeManager>()
            .contexts
            .insert(space_id, worker);

        js_update_snapshots_system(app.world_mut());

        let manager = app.world().non_send_resource::<ScriptRuntimeManager>();
        let worker = manager
            .contexts
            .get(&space_id)
            .expect("fake worker should still be registered");
        assert!(
            worker.snapshot_in_flight,
            "snapshot dispatch must set in-flight"
        );
        assert!(
            app.world().resource::<JsSnapshotState>().dirty,
            "dirty should stay true until the worker acks the snapshot"
        );
        let cmd = cmd_rx.recv().expect("snapshot command should be queued");
        assert!(matches!(cmd, JsWorkerCommand::UpdateSnapshots(_)));

        event_tx
            .send(JsWorkerEvent::SnapshotApplied)
            .expect("test should be able to inject snapshot ack");
        js_tick_system(app.world_mut());

        let manager = app.world().non_send_resource::<ScriptRuntimeManager>();
        let worker = manager
            .contexts
            .get(&space_id)
            .expect("fake worker should still be registered after ack");
        assert!(
            !worker.snapshot_in_flight,
            "snapshot ack must clear in-flight state"
        );
        assert!(
            !app.world().resource::<JsSnapshotState>().dirty,
            "snapshot ack should clear dirty when nothing else changed"
        );
    }

    #[test]
    fn snapshots_do_not_redispatch_every_frame_after_ack() {
        let (mut app, space_id) = snapshot_test_app();
        let (worker, cmd_rx, event_tx) = fake_worker(false);
        app.world_mut()
            .non_send_resource_mut::<ScriptRuntimeManager>()
            .contexts
            .insert(space_id, worker);

        js_update_snapshots_system(app.world_mut());
        assert!(
            matches!(cmd_rx.recv(), Ok(JsWorkerCommand::UpdateSnapshots(_))),
            "first dirty frame must dispatch one snapshot"
        );

        event_tx
            .send(JsWorkerEvent::SnapshotApplied)
            .expect("test should be able to inject snapshot ack");
        js_tick_system(app.world_mut());
        assert!(
            !app.world().resource::<JsSnapshotState>().dirty,
            "ack must clear dirty so idle scenes stop snapshotting"
        );

        js_update_snapshots_system(app.world_mut());
        assert!(
            matches!(cmd_rx.try_recv(), Err(std::sync::mpsc::TryRecvError::Empty)),
            "without new DOM changes the next frame must not redispatch another snapshot"
        );
    }

    #[test]
    fn snapshots_do_not_duplicate_while_ack_is_pending() {
        let (mut app, space_id) = snapshot_test_app();
        let (worker, cmd_rx, _event_tx) = fake_worker(false);
        app.world_mut()
            .non_send_resource_mut::<ScriptRuntimeManager>()
            .contexts
            .insert(space_id, worker);

        js_update_snapshots_system(app.world_mut());
        assert!(
            matches!(cmd_rx.recv(), Ok(JsWorkerCommand::UpdateSnapshots(_))),
            "first dirty frame must dispatch one snapshot"
        );

        js_update_snapshots_system(app.world_mut());
        assert!(
            matches!(cmd_rx.try_recv(), Err(std::sync::mpsc::TryRecvError::Empty)),
            "same worker must not receive duplicate snapshots every frame while the previous one is still in flight"
        );
        assert!(
            app.world().resource::<JsSnapshotState>().dirty,
            "dirty should stay true while the ack is still pending"
        );
    }

    #[test]
    fn dom_mirror_does_not_rebuild_while_only_snapshot_ack_is_pending() {
        let (mut app, space_id) = snapshot_test_app();
        let (worker, cmd_rx, _event_tx) = fake_worker(false);
        app.world_mut()
            .non_send_resource_mut::<ScriptRuntimeManager>()
            .contexts
            .insert(space_id, worker);

        js_update_snapshots_system(app.world_mut());
        assert!(
            matches!(cmd_rx.recv(), Ok(JsWorkerCommand::UpdateSnapshots(_))),
            "first dirty frame must dispatch one snapshot"
        );
        let version_after_first_send = app.world().resource::<DomMirror>().version;

        js_update_snapshots_system(app.world_mut());

        assert_eq!(
            app.world().resource::<DomMirror>().version,
            version_after_first_send,
            "pending snapshot ACK alone must not rebuild the mirror from Specs each frame"
        );
        let perf = app.world().resource::<crate::PerformanceStats>();
        assert_eq!(
            perf.waiting_on_ack, 1,
            "ACK-only fast path must report the waiting worker"
        );
        assert_eq!(
            perf.snapshots_sent, 0,
            "ACK-only fast path must not enqueue an empty snapshot"
        );
        assert!(
            app.world().resource::<JsSnapshotState>().dirty,
            "ACK-only fast path must leave dirty set until js_tick consumes the ACK"
        );
    }

    #[test]
    fn touched_dom_node_sends_snapshot_patch_after_initial_full_snapshot() {
        let mut app = App::new();
        app.insert_resource(JsSnapshotState::default());
        app.insert_resource(VirtualDomData::default());
        app.insert_resource(SpaceHandleTables::default());
        app.insert_resource(LogPanel::default());
        app.insert_resource(Time::<()>::default());
        app.insert_resource(AttributeUpdates::default());
        app.insert_resource(TransformUpdates::default());
        app.insert_resource(DirtyNodes::default());
        app.insert_resource(PendingJsAttachNodes::default());
        app.insert_resource(DomMirror::default());
        app.insert_resource(DomMirrorDirty::default());
        app.insert_non_send_resource(ScriptRuntimeManager::default());

        let specs_world = build_world();
        let space_ent = specs_world.entities().create();
        let child_ent = specs_world.entities().create();
        {
            let mut tags = specs_world.write_storage::<Tag>();
            let mut attrs = specs_world.write_storage::<Attrs>();
            let mut transforms = specs_world.write_storage::<Transform2>();
            let mut hier = specs_world.write_storage::<Hierarchy>();

            tags.insert(space_ent, Tag("space".into())).ok();
            tags.insert(child_ent, Tag("box".into())).ok();
            attrs.insert(space_ent, Attrs(HashMap::new())).ok();
            attrs.insert(child_ent, Attrs(HashMap::new())).ok();
            transforms
                .insert(
                    space_ent,
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
            transforms
                .insert(
                    child_ent,
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
            hier.insert(
                space_ent,
                Hierarchy {
                    parent: None,
                    children: vec![child_ent],
                },
            )
            .ok();
            hier.insert(
                child_ent,
                Hierarchy {
                    parent: Some(space_ent),
                    children: vec![],
                },
            )
            .ok();
        }

        app.world_mut()
            .resource_mut::<VirtualDomData>()
            .nodes
            .extend([(space_ent.id(), space_ent), (child_ent.id(), child_ent)]);
        app.insert_resource(ElemenetWorld(specs_world));

        let (worker, cmd_rx, event_tx) = fake_worker(false);
        app.world_mut()
            .non_send_resource_mut::<ScriptRuntimeManager>()
            .contexts
            .insert(space_ent.id(), worker);

        js_update_snapshots_system(app.world_mut());
        assert!(
            matches!(cmd_rx.recv(), Ok(JsWorkerCommand::UpdateSnapshots(_))),
            "initial sync must be a full snapshot"
        );
        event_tx
            .send(JsWorkerEvent::SnapshotApplied)
            .expect("test should be able to inject snapshot ack");
        js_tick_system(app.world_mut());

        {
            let specs_world = &mut app.world_mut().resource_mut::<ElemenetWorld>().0;
            let mut transforms = specs_world.write_storage::<Transform2>();
            transforms.get_mut(child_ent).unwrap().position.x = 42.0;
        }
        app.world_mut()
            .resource_mut::<DomMirrorDirty>()
            .touch(child_ent.id());
        app.world_mut().resource_mut::<JsSnapshotState>().dirty = true;

        js_update_snapshots_system(app.world_mut());

        let cmd = cmd_rx.recv().expect("patch command should be queued");
        match cmd {
            JsWorkerCommand::PatchSnapshots(patch) => {
                assert_eq!(patch.positions.len(), 1);
                assert_eq!(
                    patch.positions.values().next().map(|p| (p.x, p.y, p.z)),
                    Some((42.0, 0.0, 0.0))
                );
            }
            _ => panic!("touched node should send patch, not full snapshot"),
        }
    }

    #[test]
    fn removed_dom_node_sends_tombstone_patch_after_initial_full_snapshot() {
        let (mut app, space_id, child_id, _space_ent, _child_ent) = snapshot_test_app_with_child();
        let (worker, cmd_rx, event_tx) = fake_worker(false);
        app.world_mut()
            .non_send_resource_mut::<ScriptRuntimeManager>()
            .contexts
            .insert(space_id, worker);

        js_update_snapshots_system(app.world_mut());
        assert!(
            matches!(cmd_rx.recv(), Ok(JsWorkerCommand::UpdateSnapshots(_))),
            "initial sync must be a full snapshot"
        );
        event_tx
            .send(JsWorkerEvent::SnapshotApplied)
            .expect("test should be able to inject snapshot ack");
        js_tick_system(app.world_mut());

        let child_local_id = {
            let mut tables = app.world_mut().resource_mut::<SpaceHandleTables>();
            let table = tables
                .by_space
                .get_mut(&space_id)
                .expect("initial full snapshot should create a table");
            let local_id = table
                .global_to_local
                .remove(&child_id)
                .expect("initial full snapshot should map the child");
            table.local_to_global.remove(&local_id);
            table.pending_removed_locals.insert(local_id);
            local_id
        };
        app.world_mut()
            .resource_mut::<VirtualDomData>()
            .nodes
            .remove(&child_id);
        app.world_mut()
            .resource_mut::<DomMirrorDirty>()
            .remove(child_id);
        app.world_mut().resource_mut::<JsSnapshotState>().dirty = true;

        js_update_snapshots_system(app.world_mut());

        let cmd = cmd_rx.recv().expect("patch command should be queued");
        match cmd {
            JsWorkerCommand::PatchSnapshots(patch) => {
                assert_eq!(patch.removed_locals, vec![child_local_id]);
                assert!(patch.attr_updates.is_empty());
                assert!(patch.positions.is_empty());
            }
            _ => panic!("removed node should send a tombstone patch, not full snapshot"),
        }
        assert!(
            app.world()
                .resource::<SpaceHandleTables>()
                .by_space
                .get(&space_id)
                .map(|table| table.pending_removed_locals.is_empty())
                .unwrap_or(false),
            "sent tombstones should be cleared after enqueueing the snapshot command"
        );
    }

    #[test]
    fn ack_does_not_clear_dirty_when_snapshot_update_waited_on_inflight_worker() {
        let (mut app, space_id) = snapshot_test_app();
        let (mut worker, _cmd_rx, event_tx) = fake_worker(false);
        worker.snapshot_in_flight = true;
        app.world_mut()
            .non_send_resource_mut::<ScriptRuntimeManager>()
            .contexts
            .insert(space_id, worker);
        app.world_mut().resource_mut::<JsSnapshotState>().dirty = true;
        app.world_mut()
            .resource_mut::<JsSnapshotState>()
            .mirror_force_rebuild = false;
        app.world_mut()
            .resource_mut::<DomMirrorDirty>()
            .touch(space_id);

        js_update_snapshots_system(app.world_mut());

        // Nuevo mecanismo (opción A): el touch ocurrido mientras el worker estaba
        // in_flight queda ACUMULADO en el delta pendiente del space — en vez de
        // forzar un full-rebuild O(N). Se mandará como patch al volver a ready.
        assert!(
            app.world()
                .resource::<SpaceHandleTables>()
                .by_space
                .get(&space_id)
                .map(|t| t.pending_touched_globals.contains(&space_id))
                .unwrap_or(false),
            "dirty update blocked by an in-flight worker must be retained in pending_touched_globals"
        );

        event_tx
            .send(JsWorkerEvent::SnapshotApplied)
            .expect("test should be able to inject snapshot ack");
        js_tick_system(app.world_mut());

        assert!(
            app.world().resource::<JsSnapshotState>().dirty,
            "ACK must not clear dirty while a retained delta is pending"
        );
    }

    #[test]
    fn worker_disconnect_marks_snapshots_dirty() {
        let (mut app, space_id) = snapshot_test_app();
        let (worker, cmd_rx, event_tx) = fake_worker(true);
        app.world_mut()
            .non_send_resource_mut::<ScriptRuntimeManager>()
            .contexts
            .insert(space_id, worker);

        drop(cmd_rx);
        drop(event_tx);
        app.world_mut().resource_mut::<JsSnapshotState>().dirty = false;

        js_tick_system(app.world_mut());

        assert!(
            app.world().resource::<JsSnapshotState>().dirty,
            "disconnected worker must re-mark snapshots dirty for recovery"
        );
        let manager = app.world().non_send_resource::<ScriptRuntimeManager>();
        assert!(
            !manager.contexts.contains_key(&space_id),
            "disconnected worker should be removed from the manager"
        );
    }

    #[test]
    fn snapshots_continue_for_idle_spaces_while_other_space_is_in_flight() {
        let (mut app, space_ids) = snapshot_test_app_with_spaces(2);
        let busy_space_id = space_ids[0];
        let idle_space_id = space_ids[1];

        let (mut busy_worker, busy_cmd_rx, _busy_event_tx) = fake_worker(false);
        busy_worker.snapshot_in_flight = true;
        let (idle_worker, idle_cmd_rx, _idle_event_tx) = fake_worker(false);

        let mut manager = app
            .world_mut()
            .non_send_resource_mut::<ScriptRuntimeManager>();
        manager.contexts.insert(busy_space_id, busy_worker);
        manager.contexts.insert(idle_space_id, idle_worker);
        drop(manager);

        js_update_snapshots_system(app.world_mut());

        assert!(
            matches!(idle_cmd_rx.recv(), Ok(JsWorkerCommand::UpdateSnapshots(_))),
            "idle space should still receive its snapshot even if another worker is in flight"
        );
        assert!(
            matches!(
                busy_cmd_rx.try_recv(),
                Err(std::sync::mpsc::TryRecvError::Empty)
            ),
            "busy space must not receive another snapshot while one is already in flight"
        );
    }

    #[test]
    fn pending_scripts_wait_for_context_creation() {
        let (mut app, space_id) = snapshot_test_app();
        app.insert_resource(PendingScripts(vec![(
            space_id,
            "luna://internal/home_navigation.js".into(),
            "export {}".into(),
        )]));

        js_eval_pending_scripts(app.world_mut());

        let pending = app.world().resource::<PendingScripts>();
        assert_eq!(
            pending.0.len(),
            1,
            "script should stay queued until context exists"
        );
        assert_eq!(pending.0[0].0, space_id);
    }
}
