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
    /// Nodes which can require an isolate; updated alongside mirror deltas.
    script_candidates: HashSet<i32>,
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
    pub keyboard_commands: Vec<serde_json::Value>,
    pub audio_commands: Vec<js_runtime::audio::SharedPlayback>,
    pub pose_batches: Vec<js_runtime::pose::PoseBatch>,
    pub mesh_commands: (u64, Vec<js_runtime::mesh::MeshCommand>),
    pub needs_continuous_ticks: bool,
    pub logs: Vec<(String, String)>,
    pub attr_updates: Vec<(i32, String, String)>,
    pub pos_updates: Vec<(i32, js_runtime::Vec3)>,
    pub rot_updates: Vec<(i32, js_runtime::Vec3)>,
    pub scale_updates: Vec<(i32, js_runtime::Vec3)>,
    pub creation_queue: Vec<(i32, String)>,
    pub hierarchy_queue: Vec<(i32, i32)>,
    pub remove_queue: Vec<i32>,
    pub fetch_queue: Vec<(i32, js_runtime::FetchRequest)>,
    pub capture_queue: Vec<(i32, String)>,
    pub navigate_queue: Vec<String>,
    pub world_navigation: Vec<String>,
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
    /// Lo que el host publica hacia el documento de ajustes: estado del MCP y
    /// configuración raíz, en un JSON. Reemplaza al `EvalScript` que le escribía
    /// atributos a nodos de ids fijos. Ver `js_runtime::settings`.
    PushSettings(String),
    PushKeyboard(Vec<serde_json::Value>),
    PushFetchResults(Vec<(i32, std::result::Result<js_runtime::FetchResponse, String>)>),
    /// Resultado de una captura de frame: Ok(ruta del PNG) o Err(motivo).
    PushCaptureResults(Vec<(i32, std::result::Result<String, String>)>),
    PushWsEvents(Vec<WsWorkerEvent>),
    PushDomToqueEvents(Vec<(i32, f32, f32, f32)>),
    PushLocalToqueEvents(Vec<(i32,f32,f32,f32,[f32;3])>),
    SetHoverTargets([Option<i32>; 3]),
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
    pub component_port: Arc<js_runtime::components::ComponentPort>,
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
    /// Last shell preference successfully queued for this isolate.
    pub shell_config_sent: Option<(bool, &'static str)>,
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
            JsWorkerCommand::SetHoverTargets(_) => self
                .pending_commands
                .iter()
                .rposition(|queued| matches!(queued, JsWorkerCommand::SetHoverTargets(_))),
            // Es un estado, no una secuencia: la publicación vieja no le sirve
            // a nadie si ya hay una nueva esperando.
            JsWorkerCommand::PushSettings(_) => self
                .pending_commands
                .iter()
                .rposition(|queued| matches!(queued, JsWorkerCommand::PushSettings(_))),
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
            // Hard runaway limit, not a frame budget. Workers are asynchronous;
            // a finite slow frame must not destroy the isolate and its listeners.
            tick: Duration::from_secs(2),
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
    pub(crate) context_generation: u64,
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
    spawn_space_worker_configured(space_id, limits, None)
}

fn spawn_space_worker_configured(
    space_id: u32,
    limits: WorkerExecutionLimits,
    storage: Option<(std::path::PathBuf, String)>,
) -> std::result::Result<SpaceScriptWorker, String> {
    let component_port = Arc::new(js_runtime::components::ComponentPort::default());
    let thread_component_port = component_port.clone();
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

            ctx.engine.configure_component_port(thread_component_port.clone());
            ctx.engine.configure_text_backend(js_runtime::ui_text::TextBackend(crate::ui_text::layout));
            if let Some((path, url)) = storage {
                ctx.engine.configure_document_location(url.clone());
                ctx.engine.configure_local_storage(path, url);
            }

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
                    JsWorkerCommand::PushKeyboard(events) => ctx.engine.push_keyboard_events(events),
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
                            audio_commands: ctx.engine.drain_audio_commands(),
                            mesh_commands: ctx.engine.drain_mesh_commands(),
                            pose_batches: ctx.engine.drain_pose_batches(),
                            keyboard_commands: ctx.engine.drain_keyboard_commands(),
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
                            capture_queue: ctx.engine.drain_capture_queue(),
                            navigate_queue: ctx.engine.drain_navigate_queue(),
                            world_navigation: ctx.engine.drain_world_navigation(),
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
                    JsWorkerCommand::PushCaptureResults(results) => {
                        for (request_id, result) in results {
                            ctx.engine.push_capture_result(request_id, result);
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
                    JsWorkerCommand::PushSettings(json) => {
                        ctx.engine.publish_settings(json);
                    }
                    JsWorkerCommand::SetHoverTargets(targets) => {
                        // Data only: callbacks run in the guarded worker tick, never here.
                        for (pointer, target) in targets.into_iter().enumerate() {
                            ctx.engine.push_dom_event("__luna_hover", target.unwrap_or(-1), Some(pointer as f32), None, None);
                        }
                    }
                    JsWorkerCommand::PushLocalToqueEvents(events) => {
                        for (id,x,y,z,local) in events {ctx.engine.push_local_toque_event(id,x,y,z,local);}
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
            thread_component_port.close();
            watchdog.shutdown();
        })
        .map_err(|e| format!("failed to spawn JS worker for space {}: {}", space_id, e))?;

    Ok(SpaceScriptWorker {
        component_port,
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
        shell_config_sent: None,
    })
}

pub fn stop_space_worker(worker: &mut SpaceScriptWorker) {
    worker.component_port.close();
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

/// Borrow the DOM's existing membership map in production. Tests can provide a
/// set; neither path needs to copy every attached ID for an incremental update.
trait AttachedNodeIds {
    fn contains(&self, id: &u32) -> bool;
}
impl AttachedNodeIds for HashSet<u32> {
    fn contains(&self, id: &u32) -> bool { HashSet::contains(self, id) }
}
impl<V> AttachedNodeIds for HashMap<u32, V> {
    fn contains(&self, id: &u32) -> bool { self.contains_key(id) }
}

fn build_dom_mirror_from_specs(
    specs_world: &specs::World,
    attached_node_ids: &impl AttachedNodeIds,
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

    let script_candidates = nodes.iter()
        .filter_map(|(&id, node)| node_requires_script_owner(node).then_some(id)).collect();
    DomMirror {
        version: previous_version.saturating_add(1),
        nodes,
        space_subtrees,
        script_candidates,
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

#[cfg(test)]
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

fn mirror_descendants(mirror: &DomMirror, roots: &HashSet<i32>) -> HashSet<i32> {
    let mut result = HashSet::new();
    let mut stack: Vec<_> = roots.iter().copied().collect();
    while let Some(id) = stack.pop() {
        if !result.insert(id) {
            continue;
        }
        if let Some(node) = mirror.nodes.get(&id) {
            stack.extend(&node.children);
        }
    }
    result
}

fn update_space_membership(mirror: &mut DomMirror, affected: &HashSet<i32>, insert: bool) {
    let mut visited = Vec::new();
    for &id in affected {
        let mut cursor = id;
        visited.clear();
        while let Some(node) = mirror.nodes.get(&cursor) {
            if visited.contains(&cursor) {
                break;
            }
            visited.push(cursor);
            if node.tag == "space" {
                if insert {
                    mirror
                        .space_subtrees
                        .entry(cursor as u32)
                        .or_default()
                        .insert(id);
                } else if let Some(set) = mirror.space_subtrees.get_mut(&(cursor as u32)) {
                    set.remove(&id);
                }
            }
            cursor = node.parent;
        }
    }
}

// Includes usually append or remove one child from a large, stable sibling list.
// Trim its common ends before allocating sets for the changed middle.
fn changed_child_roots(old: &[i32], new: &[i32], roots: &mut HashSet<i32>) {
    let prefix = old.iter().zip(new).take_while(|(a, b)| a == b).count();
    let old = &old[prefix..];
    let new = &new[prefix..];
    let suffix = old
        .iter()
        .rev()
        .zip(new.iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    let old: HashSet<_> = old[..old.len() - suffix].iter().copied().collect();
    let new: HashSet<_> = new[..new.len() - suffix].iter().copied().collect();
    roots.extend(old.symmetric_difference(&new).copied());
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
    attached_node_ids: &impl AttachedNodeIds,
    force_rebuild: bool,
    touched_nodes: &HashSet<u32>,
    removed_nodes: &HashSet<u32>,
) {
    let _profile = crate::profiling::span("mirror_refresh");
    if force_rebuild || mirror.nodes.is_empty() {
        *mirror = build_dom_mirror_from_specs(specs_world, attached_node_ids, mirror.version);
        return;
    }

    if touched_nodes.is_empty() && removed_nodes.is_empty() {
        return;
    }

    let mut removed: HashSet<i32> = removed_nodes.iter().map(|&id| id as i32).collect();
    let mut updates = HashMap::new();
    let mut roots = removed.clone();
    for &node_id in touched_nodes {
        let id = node_id as i32;
        let node = attached_node_ids
            .contains(&node_id)
            .then(|| mirror_node_from_specs(specs_world, node_id))
            .flatten();
        if let Some(mut node) = node {
            node.children
                .retain(|child| attached_node_ids.contains(&(*child as u32)));
            if let Some(old) = mirror.nodes.get(&id) {
                if old.parent != node.parent || (old.tag == "space") != (node.tag == "space") {
                    roots.insert(id);
                }
                if old.children != node.children {
                    changed_child_roots(&old.children, &node.children, &mut roots);
                }
            } else {
                roots.insert(id);
            }
            // A Specs slot may be removed and reused in the same batch.
            updates.insert(id, node);
        } else {
            removed.insert(id);
            roots.insert(id);
        }
    }

    let mut affected = mirror_descendants(mirror, &roots);
    update_space_membership(mirror, &affected, false);

    let mut parents = HashSet::new();
    let mut unknown_removed = false;
    for id in &removed {
        if let Some(old) = mirror.nodes.get(id) {
            parents.insert(old.parent);
        } else {
            unknown_removed = true;
            for set in mirror.space_subtrees.values_mut() {
                set.remove(id);
            }
        }
    }
    let mut changed = !updates.is_empty();
    for id in &removed {
        changed |= mirror.nodes.remove(id).is_some();
        mirror.script_candidates.remove(id);
    }
    // Known deletions only change their parents. Preserve the repair fallback
    // for a stale ID referenced by a node whose parent can no longer be found.
    if unknown_removed {
        parents.extend(mirror.nodes.keys().copied());
    }
    for parent in parents {
        if let Some(node) = mirror.nodes.get_mut(&parent) {
            let before = node.children.len();
            node.children.retain(|child| !removed.contains(child));
            changed |= before != node.children.len();
        }
    }
    for (&id, node) in &updates {
        if node_requires_script_owner(node) { mirror.script_candidates.insert(id); }
        else { mirror.script_candidates.remove(&id); }
    }
    mirror.nodes.extend(updates);
    affected.extend(mirror_descendants(mirror, &roots));
    mirror.space_subtrees.retain(|id, _| {
        mirror
            .nodes
            .get(&(*id as i32))
            .is_some_and(|n| n.tag == "space")
    });
    update_space_membership(mirror, &affected, true);
    if changed {
        mirror.version = mirror.version.saturating_add(1);
    }
}

fn build_local_space_snapshot_from_mirror(
    space_id: u32,
    allowed: &HashSet<i32>,
    table: &mut SpaceHandleTable,
    mirror: &DomMirror,
) -> SpaceSnapshots {
    let _profile = crate::profiling::span("snapshot_full");
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
    let _profile = crate::profiling::span("snapshot_patch");
    let mut patch = SpaceSnapshotPatch::default();
    patch
        .removed_locals
        .extend(table.pending_removed_locals.iter().copied());

    // Resolve IDs before serializing any edges. The touched set is unordered:
    // a parent may otherwise be serialized before its new children get IDs.
    for &global_node_id in touched_globals {
        let id = global_node_id as i32;
        if allowed.contains(&id) && mirror.nodes.contains_key(&id) {
            ensure_local_id(table, global_node_id);
        }
    }

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

/// Resolve the spatial mount owned directly by the trusted shell, never a
/// remote lookalike with forged attributes or a neighboring tab/app.
fn world_include(world: &specs::World, space_id: u32) -> Option<u32> {
    let entities = world.entities();
    let hierarchy = world.read_storage::<Hierarchy>();
    let tags = world.read_storage::<Tag>();
    let attrs = world.read_storage::<Attrs>();
    let mut cursor = entities.entity(space_id);
    let mut visited = HashSet::new();
    while entities.is_alive(cursor) && visited.insert(cursor) {
        let parent = hierarchy.get(cursor)?.parent?;
        if tags.get(cursor)?.0 == "include" && tags.get(parent)?.0 == "space" {
            if let Some(root) = hierarchy.get(parent).and_then(|h| h.parent) {
                if let Some(hsml) = hierarchy.get(root).and_then(|h| h.parent) {
                    let trusted = tags.get(hsml).is_some_and(|t| t.0 == "hsml")
                        && hierarchy.get(hsml).is_some_and(|h| h.parent.is_none())
                        && attrs.get(root).and_then(|a| a.0.get("system-space")).is_some_and(|s| s == "root");
                    if trusted {
                        let a = &attrs.get(parent)?.0;
                        return (a.get("managed-by").is_some_and(|s| s == "dimension.luna")
                            && a.get("data-luna-kind").is_some_and(|s| s == "spatial"))
                            .then_some(cursor.id());
                    }
                }
            }
        }
        cursor = parent;
    }
    None
}

fn plan_world_navigation(world: &specs::World, current_url: &str, space_id: u32,
    requested_url: &str, caps: CapabilityBits) -> NavigationPlan {
    if caps.contains(CapabilityBits::NAVIGATE_WORLD) {
        if let Some(include_id) = world_include(world, space_id) {
            let url = resolve_node_relative_url(world, world.entities().entity(space_id), current_url, requested_url)
                .unwrap_or_else(|| requested_url.into());
            return NavigationPlan::SelfNav { include_id, url };
        }
    }
    NavigationPlan::Blocked { url: requested_url.into(), reason: "world navigation requires navigate_world and a spatial mount".into() }
}

fn prepare_include_reload(world: &mut World, include_id: u32, url: &str) {
    // A committed same-URL navigation reloads; an identical in-flight load
    // stays coalesced so its response cannot race a redundant request.
    let reload = world.get_resource::<crate::IncludeLoadStates>()
        .is_some_and(|states| matches!(states.0.get(&include_id),
            Some(crate::IncludeLoadState::Loaded { url: loaded }) if loaded == url));
    if reload {
        world.resource_mut::<crate::IncludeLoadStates>().0.remove(&include_id);
        world.resource_mut::<crate::DirtyNodes>().0.push(include_id);
        if let Some(mut transform_only) = world.get_resource_mut::<crate::TransformOnlyDirtyNodes>() {
            transform_only.0.remove(&include_id);
        }
    }
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
    let _profile = crate::profiling::span("js_update_snapshots_system");
    let snapshot_start = Instant::now();
    // Granting a resource can introduce a bootstrap script into a previously
    // static space, even when its DOM has not changed.
    let policy_scripts_changed = world.get_resource::<SpacePolicies>().is_some_and(|policies| {
        world.get_resource::<JsPolicyBridgeState>()
            .and_then(|state| state.auto_scripts_generation)
            .map(|(generation, _)| generation != policies.generation)
            .unwrap_or(policies.generation > 0)
    });
    let should_refresh = world
        .get_resource::<JsSnapshotState>()
        .map(|state| state.dirty)
        .unwrap_or(true);
    if !should_refresh && !policy_scripts_changed {
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
        && !policy_scripts_changed
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
        let requires_full_snapshot = {
            // El mirror se mantiene INCREMENTAL (touched/removed). Sólo se fuerza
            // full por motivos genuinos: navegación / bootstrap del recurso. Los
            // adds entran vía touched (nodo+padre en commit_pending_js_attaches) y
            // los removes vía removed, así que `node_count_changed` ya NO fuerza
            // full — antes era O(N) por cada add (10k) aunque sólo cambiaran 25.
            let empty = HashMap::new();
            let attached_node_ids = world.get_resource::<VirtualDomData>()
                .map(|dom| &dom.nodes).unwrap_or(&empty);
            let Some(specs_world) = world.get_resource::<ElemenetWorld>() else {
                return;
            };
            refresh_dom_mirror_in_place(
                &mut mirror,
                &specs_world.0,
                attached_node_ids,
                requires_full_snapshot,
                &touched_mirror_nodes,
                &removed_mirror_nodes,
            );
            // Red de seguridad anti-desync: si tras el refresh incremental el conteo
            // no coincide, algún productor dejó touched/removed incompletos. Full
            // rebuild UNA vez (correctness > velocidad ante bug). Si esto se queda
            // pegado en true en el panel, hay un productor que no marca dirty.
            let desynced = mirror.nodes.len() != attached_node_ids.len();
            if !requires_full_snapshot && desynced {
                refresh_dom_mirror_in_place(
                    &mut mirror,
                    &specs_world.0,
                    attached_node_ids,
                    true,
                    &HashSet::new(),
                    &HashSet::new(),
                );
            }
            requires_full_snapshot || desynced
        };
        sync_snapshots_with_mirror(
            world,
            &mut mirror,
            snapshot_start,
            requires_full_snapshot,
            &touched_mirror_nodes,
        );
    });
}

fn node_requires_script_owner(node: &DomMirrorNode) -> bool {
    node.tag == "script" || (node.tag == "include"
        && (node.attrs.contains_key("props") || node.attrs.contains_key("events")))
}

fn scripted_space_ids(mirror: &DomMirror) -> HashSet<u32> {
    let mut spaces = HashSet::new();
    for id in &mirror.script_candidates {
        let Some(node) = mirror.nodes.get(id) else { continue; };
        let mut parent = node.parent;
        // Only the nearest space owns execution, not every containing space.
        for _ in 0..mirror.nodes.len() {
            let Some(ancestor) = mirror.nodes.get(&parent) else { break; };
            if ancestor.tag == "space" {
                spaces.insert(parent as u32);
                break;
            }
            parent = ancestor.parent;
        }
    }
    spaces
}

fn sync_snapshots_with_mirror(
    world: &mut World,
    mirror: &mut DomMirror,
    snapshot_start: Instant,
    requires_full_snapshot: bool,
    touched_mirror_nodes: &HashSet<u32>,
) {
    let mut active_space_ids = scripted_space_ids(mirror);
    if let Some(pending) = world.get_resource::<PendingScripts>() {
        active_space_ids.extend(pending.0.iter().map(|(id, _, _)| *id));
    }
    if let Some(policies) = world.get_resource::<SpacePolicies>() {
        active_space_ids.extend(policies.by_space.iter()
            .filter(|(_, policy)| !policy.auto_scripts.is_empty()).map(|(&id, _)| id));
    }
    // Keep started runtimes alive: removing a script tag must not cancel its
    // timers, event listeners or local state. Unmount still destroys them.
    if let Some(manager) = world.get_non_send_resource::<ScriptRuntimeManager>() {
        active_space_ids.extend(manager.contexts.keys().copied());
    }
    active_space_ids.retain(|id| mirror.space_subtrees.contains_key(id));
    let fallback_url = world.get_resource::<crate::CurrentUrl>()
        .map(|u| u.0.as_str()).unwrap_or("");
    let existing_manager = world.get_non_send_resource::<ScriptRuntimeManager>();
    let Some(specs_world) = world.get_resource::<ElemenetWorld>() else { return; };
    let document_urls: HashMap<u32, String> = active_space_ids.iter()
        .filter(|id| !existing_manager.is_some_and(|m| m.contexts.contains_key(id)))
        .map(|&id| {
        (id, crate::dom::find_node_base_url(&specs_world.0,
            specs_world.0.entities().entity(id), fallback_url))
    }).collect();
    let storage_path = crate::utils::folder::resolve_luna_state_dir().join("local-storage.sqlite3");
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
            match spawn_space_worker_configured(*space_id, WorkerExecutionLimits::default(),
                Some((storage_path.clone(), document_urls[space_id].clone()))) {
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
                if !active_space_ids.contains(space_id) { continue; }
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
            if !needs_full && table.pending_touched_globals.is_empty()
                && table.pending_removed_locals.is_empty() {
                // A different space changed. Do not wake this isolate just to
                // apply an empty patch and acknowledge it on the next frame.
                continue;
            }
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
    crate::components::sync_components(world);
    let _profile = crate::profiling::span("js_eval_pending_scripts");
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
                    worker.shell_config_sent = None;
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

fn bind_embedded_slot(
    world: &mut World,
    sender: u32,
    content_id: u32,
    payload: &serde_json::Value,
) -> bool {
    let Some(local) = payload
        .get("anchorNodeId")
        .and_then(|v| v.as_i64())
        .and_then(|v| i32::try_from(v).ok())
    else {
        return false;
    };
    let Some(revision) = payload.get("revision").and_then(|v| v.as_u64()) else {
        return false;
    };
    let runtime_id = world
        .get_resource::<SpaceHandleTables>()
        .and_then(|tables| tables.by_space.get(&sender))
        .map(|table| table.runtime_id)
        .unwrap_or(0);
    let Some(anchor_id) = world
        .get_resource::<SpaceHandleTables>()
        .and_then(|tables| resolve_global_id(tables, sender, local))
    else {
        return false;
    };
    let Some(specs) = world.get_resource::<ElemenetWorld>() else {
        return false;
    };
    let entities = specs.0.entities();
    let anchor = entities.entity(anchor_id);
    let content = entities.entity(content_id);
    if !entities.is_alive(anchor)
        || !entities.is_alive(content)
        || find_owner_space_id(&specs.0, anchor) != Some(sender)
    {
        return false;
    }
    drop(entities);
    let epoch = world
        .get_resource::<crate::NavigationEpoch>()
        .map(|e| e.0)
        .unwrap_or(0);
    world.init_resource::<crate::embedded::EmbeddedWindows>();
    let mut windows = world.resource_mut::<crate::embedded::EmbeddedWindows>();
    if windows.0.get(&content_id).is_some_and(|old| {
        old.epoch == epoch
            && old.content == content
            && old.runtime_id == runtime_id
            && old.revision >= revision
    }) {
        return false;
    }
    windows.0.insert(
        content_id,
        crate::embedded::WindowBinding {
            anchor,
            content,
            epoch,
            revision,
            runtime_id,
        },
    );
    true
}

pub fn js_tick_system(world: &mut World) {
    crate::dynamic_mesh::cleanup_contexts(world);
    let _profile = crate::profiling::span("js_tick_system");
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
            worker.needs_tick |= worker.component_port.take_wake();
            if worker.tick_in_flight || !worker.needs_tick {
                continue;
            }
            match worker.try_send(JsWorkerCommand::Tick { elapsed_ms }) {
                Ok(_) => {
                    worker.tick_in_flight = true;
                    // Consume only the wakeup covered by this tick. Events queued
                    // while it runs can request another tick independently.
                    worker.needs_tick = false;
                }
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
                        worker.needs_tick |= data.needs_continuous_ticks;
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
    let mut capture_batches: Vec<(u32, Vec<(i32, String)>)> = Vec::new();
    let mut navigate_batches = Vec::new();
    let mut world_navigation_batches = Vec::new();
    let mut tab_action_batches: Vec<(u32, Vec<js_runtime::TabAction>)> = Vec::new();
    let mut shell_message_batches: Vec<(u32, Vec<js_runtime::ShellMessage>)> = Vec::new();
    let mut ws_connect_batches: Vec<(u32, Vec<(i32, String)>)> = Vec::new();
    let mut ws_send_batches: Vec<(u32, Vec<(i32, String)>)> = Vec::new();
    let mut ws_close_batches: Vec<(u32, Vec<i32>)> = Vec::new();
    let mut snapshot_dirty = false;

    let capabilities_by_space = space_capabilities_snapshot(world);

    for (space_id, data) in tick_batches {
        crate::keyboard::apply_commands(world, space_id, data.keyboard_commands);
        if !data.world_navigation.is_empty() {
            world_navigation_batches.push((space_id, data.world_navigation));
        }
        for mut batch in data.pose_batches {
            let target = world.get_resource::<SpaceHandleTables>()
                .and_then(|tables| resolve_global_id(tables, space_id, batch.node));
            if let Some(target) = target {
                batch.node = target as i32;
                crate::model_pose::submit(world, batch);
            }
        }
        crate::audio::apply_commands(world, space_id, data.audio_commands);
        crate::dynamic_mesh::apply_commands(world, space_id, data.mesh_commands.0, data.mesh_commands.1);
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
        if !data.capture_queue.is_empty() {
            capture_batches.push((space_id, data.capture_queue));
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
    let log_contexts: HashMap<_, _> = logs_by_context.iter().map(|(space_id, _)| {
        let tab = world.get_resource::<ElemenetWorld>().and_then(|w| crate::ui::find_tab_id_for_space(&w.0, *space_id));
        let runtime = world.get_resource::<SpaceHandleTables>().and_then(|t| t.by_space.get(space_id)).map(|t| t.runtime_id);
        (*space_id, (tab, runtime))
    }).collect();

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
                let mut entry = crate::LogEntry::with_space(log_level, msg, space_id);
                (entry.tab_id, entry.runtime_id) = log_contexts[&space_id];
                log_panel.push_entry(entry);
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

        let (newly_attached_ids, already_attached_dirty_ids, changed_parent_ids, log_messages) = {
            let Some(mut specs_world) = world.get_resource_mut::<ElemenetWorld>() else {
                return;
            };
            let mut newly_attached_ids: Vec<u32> = Vec::new();
            let mut newly_attached_seen = HashSet::new();
            let mut already_attached_dirty_ids = HashSet::new();
            let mut log_messages = Vec::new();
            let mut changed_parent_ids = HashSet::new();
            for (parent_id, child_id) in allowed_appends {
                let (parent_ent, child_ent, are_alive) = {
                    let entities = specs_world.0.entities();
                    let parent_ent = entities.entity(parent_id);
                    let child_ent = entities.entity(child_id);
                    let are_alive = entities.is_alive(parent_ent) && entities.is_alive(child_ent);
                    (parent_ent, child_ent, are_alive)
                };
                if are_alive {
                    let old_parent = specs_world.0.read_storage::<Hierarchy>()
                        .get(child_ent).and_then(|h| h.parent);
                    match Hierarchy::try_add_child(&mut specs_world.0, parent_ent, child_ent) {
                        Ok(true) => {}
                        Ok(false) => continue,
                        Err(reason) => {
                            log_messages.push(format!("[JS][space:{space_id}] Blocked appendChild: {reason}"));
                            continue;
                        }
                    }
                    if let Some(old_parent) = old_parent {
                        changed_parent_ids.insert(old_parent.id());
                    }
                    changed_parent_ids.insert(parent_id);

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
            (newly_attached_ids, already_attached_dirty_ids, changed_parent_ids, log_messages)
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
        if let Some(mut transform_only) = world.get_resource_mut::<crate::TransformOnlyDirtyNodes>() {
            for id in &dirty_ids_vec { transform_only.0.remove(id); }
        }
        if let Some(mut dirty_nodes) = world.get_resource_mut::<DirtyNodes>() {
            dirty_nodes.0.extend(dirty_ids_vec.iter().copied());
        }
        // Tocar el mirror para los nodos re-parentados YA attached: no pasan por
        // commit_pending_js_attaches (que cubre los newly_attached), así que sin
        // esto su jerarquía quedaría stale al quitar el force-rebuild blanket.
        if !dirty_ids_vec.is_empty() || !changed_parent_ids.is_empty() {
            if let Some(mut mirror_dirty) = world.get_resource_mut::<DomMirrorDirty>() {
                for &nid in dirty_ids_vec.iter().chain(changed_parent_ids.iter()) {
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
        let mut known: std::collections::HashSet<u32> = world
            .get_resource::<crate::VirtualDomData>()
            .map(|d| d.nodes.keys().copied().collect())
            .unwrap_or_default();
        // Created nodes already have valid handles but are not in VirtualDomData
        // until the deferred attach commits. Removing them in that interval must
        // delete them too, otherwise replaced menus leave orphaned geometry.
        if let Some(tables) = world.get_resource::<SpaceHandleTables>() {
            for table in tables.by_space.values() {
                known.extend(table.detached_globals.iter().copied());
            }
        }
        known
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
        // JS removal also drives root.unmountSpace(). Forget skybox ownership
        // and pending waiters before Specs reuses these numeric node IDs.
        if let Some(mut skybox) = world.get_resource_mut::<crate::SkyboxEntity>() {
            for &nid in &all_removed_ids {
                skybox.clear_node(nid);
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
            pending_models.remove_nodes(&all_removed_ids.iter().copied().collect());
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
        // A cancelled attach must not later resurrect a recycled Specs id.
        if let Some(mut pending) = world.get_resource_mut::<PendingJsAttachNodes>() {
            pending.0.retain(|id| !frame_deleted_global.contains(id));
        }
        if let Some(mut pending) = world.get_resource_mut::<crate::PendingJsFirstRenderNodes>() {
            pending.0.retain(|(id, _)| !frame_deleted_global.contains(id));
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
            .map(|caps| caps.intersects(CapabilityBits::FETCH_TEXT | CapabilityBits::FETCH_HTTP))
            .unwrap_or(false);
        if !can_fetch {
            let failures = fetch_queue.iter().map(|(id, _)| (*id, Err("Permission denied: fetch_text / fetch_http was not granted".into()))).collect();
            if let Some(mut manager) = world.get_non_send_resource_mut::<ScriptRuntimeManager>() {
                if let Some(worker) = manager.contexts.get_mut(&space_id) {
                    if worker.try_send(JsWorkerCommand::PushFetchResults(failures)).is_ok() { worker.needs_tick = true; }
                }
            }
            if let Some(mut log_panel) = world.get_resource_mut::<LogPanel>() {
                for (_, request) in &fetch_queue {
                    log_panel.push_warn(format!(
                        "[JS][space:{}] blocked fetch (missing fetch_text): {}",
                        space_id, request.url
                    ));
                }
            }
            continue;
        }

        let fetch_base = {
            let doc = world.resource::<ElemenetWorld>();
            crate::dom::find_node_base_url(&doc.0, doc.0.entities().entity(space_id), &world.resource::<crate::CurrentUrl>().0)
        };
        let native_fetch = world.resource::<crate::SpacePolicies>().by_space.get(&space_id)
            .is_some_and(|p| p.origin.starts_with("luna:"));
        let Some(tokio_rt) = world.get_resource::<crate::TokioRuntime>() else {
            return;
        };
        let Some(io_service) = world.get_resource::<IoService>() else {
            return;
        };
        let mut rejected = Vec::new();
        for (request_id, request) in &fetch_queue {
            if !matches!(request.method.as_str(), "GET" | "HEAD") && !capabilities_by_space.get(&space_id)
                .is_some_and(|caps| caps.contains(CapabilityBits::FETCH_HTTP)) {
                rejected.push((*request_id, Err("Permission denied: method requires fetch_http".into())));
                continue;
            }
            let resolved = if native_fetch { Ok(request.url.clone()) } else { crate::io::same_origin_url(&fetch_base, &request.url) };
            let resolved = match resolved {
                Ok(url) => url,
                Err(error) => { rejected.push((*request_id, Err(error))); continue; }
            };
            let mut request = request.clone();
            request.url = resolved;
            if let Err(error) =
                request_fetch_text(&tokio_rt.0, &io_service, space_id, *request_id, request, (!native_fetch).then(|| fetch_base.clone()))
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
                    "[JS][space:{space_id}] rejected {rejected_count} fetch request(s); errors returned to caller"
                ));
            }
        }
        if let Some(mut log_panel) = world.get_resource_mut::<LogPanel>() {
            for (_, request) in &fetch_queue {
                log_panel.push_info(format!("[JS][space:{}] fetch queued: {} {}", space_id, request.method, request.url));
            }
        }
    }

    // Capturas de frame: se valida CAPTURE_FRAME por space y recién ahí pasan a
    // la cola del host. Un space sin la cap recibe el rechazo por la misma vía
    // que el éxito, así la promesa de JS siempre termina.
    for (space_id, requests) in capture_batches {
        let caps = capabilities_by_space
            .get(&space_id)
            .copied()
            .unwrap_or_default();

        if caps.contains(CapabilityBits::CAPTURE_FRAME) {
            let runtime_id = world.resource::<SpaceHandleTables>().by_space.get(&space_id).map(|t|t.runtime_id).unwrap_or(0);
            if let Some(mut pendientes) =
                world.get_resource_mut::<crate::capture::PendingFrameCaptures>()
            {
                for (request_id, name) in requests {
                    pendientes.0.push(crate::capture::CaptureRequest {
                        runtime_id,
                        deadline: std::time::Instant::now() + std::time::Duration::from_secs(25),
                        space_id,
                        request_id,
                        name,
                    });
                }
            }
            continue;
        }

        let rechazos: Vec<(i32, std::result::Result<String, String>)> = requests
            .into_iter()
            .map(|(request_id, _)| {
                (
                    request_id,
                    Err("captureFrame denied (missing CAPTURE_FRAME)".to_string()),
                )
            })
            .collect();
        let rechazados = rechazos.len();
        if let Some(mut manager) = world.get_non_send_resource_mut::<ScriptRuntimeManager>() {
            if let Some(worker) = manager.contexts.get_mut(&space_id) {
                if worker
                    .try_send(JsWorkerCommand::PushCaptureResults(rechazos))
                    .is_ok()
                {
                    worker.needs_tick = true;
                }
            }
        }
        if let Some(mut log_panel) = world.get_resource_mut::<LogPanel>() {
            log_panel.push_warn(format!(
                "[JS][space:{space_id}] captureFrame denied (missing CAPTURE_FRAME): {rechazados} pedido(s)"
            ));
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
        for (space_id, urls) in world_navigation_batches {
            let caps = capabilities_by_space.get(&space_id).copied().unwrap_or_default();
            for url in urls {
                plans.push((space_id, plan_world_navigation(&specs_world.0, &current_url, space_id, &url, caps)));
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
                prepare_include_reload(world, include_id, &url);
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
                    js_runtime::TabAction::SetMcpEnabled { enabled } => {
                        let allowed = world.get_resource::<ElemenetWorld>().is_some_and(|dom| {
                            let entity = dom.0.entities().entity(space_id);
                            crate::agent::is_settings_document(&crate::dom::find_node_base_url(&dom.0, entity, ""))
                        });
                        if allowed {
                            if let Some(mut agent) = world.get_resource_mut::<crate::agent::AgentControl>() {
                                agent.enabled = enabled;
                            }
                        }
                    }
                    js_runtime::TabAction::SetMcpAutoStart { enabled } => {
                        let allowed = world.get_resource::<ElemenetWorld>().is_some_and(|dom| {
                            let entity = dom.0.entities().entity(space_id);
                            crate::agent::is_settings_document(&crate::dom::find_node_base_url(&dom.0, entity, ""))
                        });
                        if allowed {
                            let result = world.get_resource_mut::<crate::RootConfig>().map(|mut config| {
                                let mut updated = config.clone();
                                updated.mcp_auto_start = enabled;
                                updated.save().map(|_| { *config = updated; })
                            });
                            if let Some(Err(error)) = result {
                                if let Some(mut log) = world.get_resource_mut::<LogPanel>() {
                                    log.push_warn(format!("[MCP] Could not save startup preference: {error}"));
                                }
                            }
                        }
                    }
                    js_runtime::TabAction::SetRootSettings { patch } => {
                        // Mismo control que las dos del MCP: la autoridad es la
                        // URL del documento, no una capacidad. Ver
                        // `is_settings_document`.
                        let allowed = world.get_resource::<ElemenetWorld>().is_some_and(|dom| {
                            let entity = dom.0.entities().entity(space_id);
                            crate::agent::is_settings_document(&crate::dom::find_node_base_url(&dom.0, entity, ""))
                        });
                        if allowed {
                            if let Err(error) = crate::settings::apply_root_patch(world, &patch) {
                                if let Some(mut log) = world.get_resource_mut::<LogPanel>() {
                                    log.push_warn(format!("[settings] {error}"));
                                }
                            }
                        }
                    }
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
            let mut opened = Vec::new();
            for (space_id, url, kind) in open_requests {
                if kind == "app-embedded" {
                    opened.push((space_id, serde_json::json!({
                        "type": "tabopened", "tabId": next_id_value, "url": url, "kind": kind,
                    }).to_string()));
                }
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
            if let Some(mut manager) = world.get_non_send_resource_mut::<ScriptRuntimeManager>() {
                for (space_id, payload) in opened {
                    if let Some(worker) = manager.contexts.get_mut(&space_id) {
                        if worker.try_send(JsWorkerCommand::PushShellMessages(vec![js_runtime::ShellMessage {
                            target_tab_id: 0, payload,
                        }])).is_ok() { worker.needs_tick = true; }
                    }
                }
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
                if !is_shell && msg.target_tab_id != 0 {
                    rejected.push(format!("[JS][space:{sender_space_id}] apps may only message the shell"));
                    continue;
                }

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

                // Window placement is a host contract, accepted only from the
                // trusted shell for embedded targets. Stale slots cannot rebind.
                if let Ok(payload) = serde_json::from_str::<serde_json::Value>(&msg.payload) {
                    if payload.get("type").and_then(|v| v.as_str()) == Some("slot")
                        && payload.get("coordinateSpace").and_then(|v| v.as_str()) == Some("window-local") {
                        if !is_shell || !capabilities_by_space.get(&target_space_id)
                            .copied().unwrap_or_default().contains(CapabilityBits::UX_EMBED)
                            || !bind_embedded_slot(world, sender_space_id, target_space_id, &payload) {
                            continue;
                        }
                    }
                }

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
    #[test]
    fn world_navigation_targets_own_spatial_mount_and_keeps_self_semantics() {
        let mut world = virtual_dom::dom::element::build_world();
        virtual_dom::parse_xml(&mut world, r#"<hsml><space system-space="root">
          <space managed-by="dimension.luna" data-luna-kind="spatial">
            <include id="world"><hsml><space><include id="door"><hsml><space id="caller"/></hsml></include></space></hsml></include>
          </space>
          <space managed-by="dimension.luna" data-luna-kind="app-embedded">
            <include><hsml><space id="app"><hsml><space system-space="root">
              <space managed-by="dimension.luna" data-luna-kind="spatial"><include><space id="forged"/></include></space>
            </space></hsml></space></hsml></include>
          </space>
        </space></hsml>"#).unwrap();
        let find = |id: &str| {
            let attrs = world.read_storage::<Attrs>();
            (&world.entities(), &attrs).join().find(|(_,a)| a.0.get("id").is_some_and(|s| s == id)).unwrap().0.id()
        };
        let (caller, target, door, app) = (find("caller"), find("world"), find("door"), find("app"));
        let forged = find("forged");
        let both = CapabilityBits::NAVIGATE_SELF | CapabilityBits::NAVIGATE_WORLD;
        let url = "https://example.test/next#entry=door";
        assert!(matches!(super::plan_world_navigation(&world, "luna://root", caller, url, both),
            super::NavigationPlan::SelfNav {include_id, url: u} if include_id == target && u == url));
        assert!(matches!(super::plan_navigation_for_space(&world, "luna://root", caller, url, both),
            super::NavigationPlan::SelfNav {include_id, ..} if include_id == door));
        assert!(matches!(super::plan_world_navigation(&world, "luna://root", caller, url, CapabilityBits::NAVIGATE_SELF),
            super::NavigationPlan::Blocked {..}));
        assert!(matches!(super::plan_world_navigation(&world, "luna://root", app, url, CapabilityBits::all()),
            super::NavigationPlan::Blocked {..}));
        assert!(matches!(super::plan_world_navigation(&world, "luna://root", forged, url, CapabilityBits::all()),
            super::NavigationPlan::Blocked {..}));
        world.write_storage::<virtual_dom::dom::element::BaseUrl>().insert(world.entities().entity(caller),
            virtual_dom::dom::element::BaseUrl("https://doors.test/components/door.hsml?to=x".into())).unwrap();
        assert!(matches!(super::plan_world_navigation(&world, "luna://root", caller, "../room.hsml#entry=back", both),
            super::NavigationPlan::SelfNav {url, ..} if url == "https://doors.test/room.hsml#entry=back"));
    }

    #[test]
    fn same_url_navigation_reloads_committed_include_without_restarting_inflight() {
        let mut world = bevy::prelude::World::new();
        world.insert_resource(crate::IncludeLoadStates::default());
        world.insert_resource(crate::DirtyNodes::default());
        world.insert_resource(crate::TransformOnlyDirtyNodes::default());
        let url = "https://example.test/door?uuid=one#arrival";
        world.resource_mut::<crate::IncludeLoadStates>().0.insert(7,
            crate::IncludeLoadState::Loaded { url: url.into() });
        world.resource_mut::<crate::TransformOnlyDirtyNodes>().0.insert(7);
        super::prepare_include_reload(&mut world, 7, url);
        assert!(!world.resource::<crate::IncludeLoadStates>().0.contains_key(&7));
        assert_eq!(world.resource::<crate::DirtyNodes>().0, vec![7]);
        assert!(!world.resource::<crate::TransformOnlyDirtyNodes>().0.contains(&7));

        world.resource_mut::<crate::DirtyNodes>().0.clear();
        world.resource_mut::<crate::IncludeLoadStates>().0.insert(7,
            crate::IncludeLoadState::Loading { url: url.into() });
        super::prepare_include_reload(&mut world, 7, url);
        assert!(world.resource::<crate::IncludeLoadStates>().0.contains_key(&7));
        assert!(world.resource::<crate::DirtyNodes>().0.is_empty());
    }
    use super::*;
    use bevy::prelude::{App, Time};
    use specs::WorldExt;
    use virtual_dom::dom::element::{build_world, Vec3 as DomVec3};

    #[test]
    fn static_includes_stay_dormant_and_script_insertion_wakes_only_its_owner() {
        let mut specs = build_world();
        let root = virtual_dom::parse_xml(&mut specs, "<space><space><box/></space><space><box/></space></space>").unwrap();
        let owner = specs.read_storage::<Hierarchy>().get(root).unwrap().children[0];
        let mut world = World::new();
        let nodes = specs.entities().join().map(|e|(e.id(),e)).collect();
        world.insert_resource(ElemenetWorld(specs));
        world.insert_resource(VirtualDomData {nodes});
        world.init_resource::<DomMirror>();
        world.init_resource::<DomMirrorDirty>();
        world.init_resource::<JsSnapshotState>();
        world.init_resource::<SpaceHandleTables>();
        world.insert_non_send_resource(ScriptRuntimeManager::default());
        js_update_snapshots_system(&mut world);
        assert!(world.non_send_resource::<ScriptRuntimeManager>().contexts.is_empty());
        assert!(world.resource::<SpaceHandleTables>().by_space.is_empty());
        let script = {
            let mut specs = world.resource_mut::<ElemenetWorld>();
            let script = virtual_dom::parse_xml(&mut specs.0, "<script>globalThis.awake = true;</script>").unwrap();
            Hierarchy::add_child(&mut specs.0, owner, script);
            script
        };
        world.resource_mut::<VirtualDomData>().nodes.insert(script.id(),script);
        world.resource_mut::<DomMirrorDirty>().touch(script.id());
        world.resource_mut::<DomMirrorDirty>().touch(owner.id());
        world.resource_mut::<JsSnapshotState>().dirty = true;
        js_update_snapshots_system(&mut world);
        let manager = world.non_send_resource::<ScriptRuntimeManager>();
        assert_eq!(manager.contexts.len(),1);
        assert!(manager.contexts.contains_key(&owner.id()));
        assert_eq!(world.resource::<SpaceHandleTables>().by_space.len(),1);
    }

    #[test]
    fn script_candidate_index_tracks_props_reparent_and_removal() {
        let mut world = build_world();
        let root = virtual_dom::parse_xml(&mut world,
            "<space><space><include src='ui.hsml'/></space><space><script/></space></space>").unwrap();
        let owners = world.read_storage::<Hierarchy>().get(root).unwrap().children.clone();
        let include = world.read_storage::<Hierarchy>().get(owners[0]).unwrap().children[0];
        let script = world.read_storage::<Hierarchy>().get(owners[1]).unwrap().children[0];
        let mut attached: HashMap<u32, _> = world.entities().join().map(|e| (e.id(), e)).collect();
        let mut mirror = build_dom_mirror_from_specs(&world, &attached, 0);
        assert_eq!(scripted_space_ids(&mirror), HashSet::from([owners[1].id()]));
        world.write_storage::<Attrs>().get_mut(include).unwrap().0.insert("props".into(), "{}".into());
        refresh_dom_mirror_in_place(&mut mirror, &world, &attached, false,
            &HashSet::from([include.id()]), &HashSet::new());
        assert_eq!(scripted_space_ids(&mirror), HashSet::from([owners[0].id(), owners[1].id()]));
        Hierarchy::add_child(&mut world, owners[0], script);
        refresh_dom_mirror_in_place(&mut mirror, &world, &attached, false,
            &HashSet::from([script.id(), owners[0].id(), owners[1].id()]), &HashSet::new());
        assert_eq!(scripted_space_ids(&mirror), HashSet::from([owners[0].id()]));
        world.write_storage::<Attrs>().get_mut(include).unwrap().0.remove("props");
        attached.remove(&script.id());
        refresh_dom_mirror_in_place(&mut mirror, &world, &attached, false,
            &HashSet::from([include.id()]), &HashSet::from([script.id()]));
        assert!(mirror.script_candidates.is_empty());
        assert!(scripted_space_ids(&mirror).is_empty());
        let full = build_dom_mirror_from_specs(&world, &attached, 0);
        assert_eq!(mirror.script_candidates, full.script_candidates);
    }

    // Measures only the mirror phase; not GPU frame time or HTTP/parse latency.
    #[test]
    #[ignore = "manual include mirror benchmark"]
    fn benchmark_include_mirror_delta() {
        for background in [4_000, 14_000] {
            let mut world = build_world();
            let xml = format!("<space>{}<include src='chunk.hsml'/></space>", "<box color='#123456'/>".repeat(background));
            let root = virtual_dom::parse_xml(&mut world, &xml).unwrap();
            let include = *world.read_storage::<Hierarchy>().get(root).unwrap().children.last().unwrap();
            let mut attached: HashSet<u32> = world.entities().join().map(|e|e.id()).collect();
            let baseline = build_dom_mirror_from_specs(&world, &attached, 0);
            let chunk = virtual_dom::parse_xml(&mut world, "<space><box/><box/><box/><box/><box/><box/><box/><box/></space>").unwrap();
            Hierarchy::add_child(&mut world, include, chunk);
            let after: HashSet<u32> = world.entities().join().map(|e|e.id()).collect();
            let mut touched: HashSet<u32> = after.difference(&attached).copied().collect();
            touched.insert(include.id());
            attached = after;
            let mut full = baseline.clone();
            let mut delta = baseline.clone();
            let mut full_ms = 0.0;
            let mut delta_ms = 0.0;
            for _ in 0..100 {
                full = baseline.clone();
                let start = Instant::now();
                refresh_dom_mirror_in_place(&mut full, &world, &attached, true, &touched, &HashSet::new());
                full_ms += start.elapsed().as_secs_f64() * 10.0;
            }
            for _ in 0..100 {
                delta = baseline.clone();
                let start = Instant::now();
                refresh_dom_mirror_in_place(&mut delta, &world, &attached, false, &touched, &HashSet::new());
                delta_ms += start.elapsed().as_secs_f64() * 10.0;
            }
            assert_eq!(delta.nodes.len(), full.nodes.len());
            assert_eq!(delta.space_subtrees, full.space_subtrees);
            for (id, node) in &full.nodes {
                assert_eq!(delta.nodes[id].attrs, node.attrs);
                assert_eq!(delta.nodes[id].children, node.children);
                assert_eq!(delta.nodes[id].parent, node.parent);
            }
            eprintln!("include mirror background={background}: full={full_ms:.3}ms delta={delta_ms:.3}ms ratio={:.2}x",full_ms/delta_ms);
        }
    }

    fn deletion_test_mirror(count: i32) -> DomMirror {
        let mut mirror = DomMirror::default();
        for id in 0..=count {
            mirror.nodes.insert(
                id,
                DomMirrorNode {
                    attrs: HashMap::new(),
                    tag: if id == 0 { "space" } else { "box" }.into(),
                    position: js_runtime::Vec3 {
                        x: 0.0,
                        y: 0.0,
                        z: 0.0,
                    },
                    rotation: js_runtime::Vec3 {
                        x: 0.0,
                        y: 0.0,
                        z: 0.0,
                    },
                    scale: js_runtime::Vec3 {
                        x: 1.0,
                        y: 1.0,
                        z: 1.0,
                    },
                    parent: if id == 0 { -1 } else { 0 },
                    children: if id == 0 {
                        (1..=count).collect()
                    } else {
                        vec![]
                    },
                },
            );
        }
        rebuild_dom_mirror_space_subtrees(&mut mirror);
        mirror
    }

    #[test]
    fn include_patch_allocates_all_ids_before_serializing_edges() {
        let mirror = deletion_test_mirror(64);
        let allowed = &mirror.space_subtrees[&0];
        for _ in 0..32 {
            let touched: HashSet<u32> = (0..=64).collect();
            let mut table = SpaceHandleTable { next_local_id: 1, ..Default::default() };
            table.global_to_local.insert(0, 0);
            table.local_to_global.insert(0, 0);
            let patch = build_local_space_patch_from_mirror(allowed, &touched, &mut table, &mirror);
            assert_eq!(patch.children[&0].len(), 64);
            for child in &patch.children[&0] {
                assert_eq!(patch.parents[child], 0);
                assert_eq!(patch.tag_updates[child], "box");
            }
        }
    }

    #[test]
    fn incremental_space_index_matches_full_rebuild_during_scroll_and_reparent() {
        fn compare(
            world: &specs::World,
            mirror: &mut DomMirror,
            touched: HashSet<u32>,
            removed: HashSet<u32>,
        ) {
            let attached: HashSet<u32> = world.entities().join().map(|e| e.id()).collect();
            refresh_dom_mirror_in_place(mirror, world, &attached, false, &touched, &removed);
            let full = build_dom_mirror_from_specs(world, &attached, 0);
            assert_eq!(mirror.space_subtrees, full.space_subtrees);
            assert_eq!(mirror.nodes.len(), full.nodes.len());
            for (id, node) in full.nodes {
                let actual = &mirror.nodes[&id];
                assert_eq!(actual.parent, node.parent);
                assert_eq!(actual.children, node.children);
                assert_eq!(actual.attrs, node.attrs);
                assert_eq!(actual.tag, node.tag);
            }
        }
        let mut world = build_world();
        let root = virtual_dom::parse_xml(
            &mut world,
            &format!("<space><space/><space/>{}</space>", "<box/>".repeat(256)),
        )
        .unwrap();
        let children = world
            .read_storage::<Hierarchy>()
            .get(root)
            .unwrap()
            .children
            .clone();
        let (left, right) = (children[0], children[1]);
        let attached: HashSet<u32> = world.entities().join().map(|e| e.id()).collect();
        let mut mirror = build_dom_mirror_from_specs(&world, &attached, 0);
        for iteration in 0..32 {
            let before: HashSet<u32> = world.entities().join().map(|e| e.id()).collect();
            let chunk =
                virtual_dom::parse_xml(&mut world, "<group><space><box/><box/></space></group>")
                    .unwrap();
            Hierarchy::add_child(&mut world, left, chunk);
            let inserted: HashSet<u32> = world
                .entities()
                .join()
                .map(|e| e.id())
                .filter(|id| !before.contains(id))
                .collect();
            let mut touched = inserted.clone();
            touched.insert(left.id());
            compare(&world, &mut mirror, touched, HashSet::new());

            world
                .write_storage::<Hierarchy>()
                .get_mut(left)
                .unwrap()
                .children
                .retain(|e| *e != chunk);
            Hierarchy::add_child(&mut world, right, chunk);
            compare(
                &world,
                &mut mirror,
                HashSet::from([left.id(), right.id(), chunk.id()]),
                HashSet::new(),
            );

            world.write_storage::<Tag>().get_mut(chunk).unwrap().0 = "space".into();
            compare(
                &world,
                &mut mirror,
                HashSet::from([chunk.id()]),
                HashSet::new(),
            );
            world
                .write_storage::<Attrs>()
                .get_mut(chunk)
                .unwrap()
                .0
                .insert("test".into(), iteration.to_string());
            compare(
                &world,
                &mut mirror,
                HashSet::from([chunk.id()]),
                HashSet::new(),
            );
            world.write_storage::<Tag>().get_mut(chunk).unwrap().0 = "group".into();
            compare(
                &world,
                &mut mirror,
                HashSet::from([chunk.id()]),
                HashSet::new(),
            );

            world
                .write_storage::<Hierarchy>()
                .get_mut(right)
                .unwrap()
                .children
                .retain(|e| *e != chunk);
            for &id in &inserted {
                let entity = world.entities().entity(id);
                world.delete_entity(entity).unwrap();
            }
            world.maintain();
            compare(&world, &mut mirror, HashSet::from([right.id()]), inserted);
        }
    }

    #[test]
    fn mirror_batch_removal_preserves_survivors_and_cleans_dangling_links() {
        let world = build_world();
        let mut mirror = deletion_test_mirror(10_000);
        // Un ID ausente también puede quedar referenciado por un padre.
        mirror.nodes.get_mut(&0).unwrap().children.push(20_000);
        let removed: HashSet<u32> = (1..=1_000).chain([20_000]).collect();
        let attached: HashSet<u32> = std::iter::once(0).chain(1_001..=10_000).collect();
        refresh_dom_mirror_in_place(
            &mut mirror,
            &world,
            &attached,
            false,
            &HashSet::new(),
            &removed,
        );
        assert_eq!(mirror.version, 1);
        assert_eq!(mirror.nodes.len(), 9_001);
        assert_eq!(
            mirror.nodes[&0].children,
            (1_001..=10_000).collect::<Vec<_>>()
        );
        assert_eq!(
            mirror.space_subtrees[&0],
            attached.iter().map(|&id| id as i32).collect()
        );
        // Repetir el mismo batch es un no-op y no cambia la versión.
        refresh_dom_mirror_in_place(
            &mut mirror,
            &world,
            &attached,
            false,
            &HashSet::new(),
            &removed,
        );
        assert_eq!(mirror.version, 1);
    }

    #[test]
    #[ignore = "manual mirror deletion benchmark"]
    fn benchmark_mirror_batch_removal() {
        let world = build_world();
        let original = deletion_test_mirror(10_000);
        let removed: HashSet<u32> = (1..=1_000).collect();
        let attached: HashSet<u32> = std::iter::once(0).chain(1_001..=10_000).collect();
        let mut old_samples = Vec::new();
        let mut new_samples = Vec::new();
        for _ in 0..12 {
            let mut old = original.clone();
            let start = Instant::now();
            // Algoritmo anterior, conservado sólo como referencia del benchmark.
            for &id in &removed {
                old.nodes.remove(&(id as i32));
                for node in old.nodes.values_mut() {
                    node.children.retain(|child| *child != id as i32);
                }
            }
            old.version += 1;
            rebuild_dom_mirror_space_subtrees(&mut old);
            old_samples.push(start.elapsed().as_secs_f64() * 1000.0);

            let mut new = original.clone();
            let start = Instant::now();
            refresh_dom_mirror_in_place(
                &mut new,
                &world,
                &attached,
                false,
                &HashSet::new(),
                &removed,
            );
            new_samples.push(start.elapsed().as_secs_f64() * 1000.0);
            assert_eq!(old.nodes.len(), new.nodes.len());
            assert_eq!(old.nodes[&0].children, new.nodes[&0].children);
            assert_eq!(old.space_subtrees, new.space_subtrees);
        }
        old_samples.sort_by(f64::total_cmp);
        new_samples.sort_by(f64::total_cmp);
        eprintln!(
            "mirror_delete nodes=10000 removed=1000 old_median_ms={:.3} new_median_ms={:.3}",
            old_samples[6], new_samples[6]
        );
    }

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
                component_port: Arc::new(js_runtime::components::ComponentPort::default()),
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
        shell_config_sent: None,
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
    fn click_wakeup_survives_completion_of_an_older_idle_tick() {
        let (worker, cmd_rx, event_tx) = fake_worker(true);
        let mut manager = ScriptRuntimeManager::default();
        manager.contexts.insert(42, worker);
        let mut app = App::new();
        app.insert_resource(Time::<()>::default());
        app.insert_non_send_resource(manager);
        js_tick_system(app.world_mut());
        assert!(matches!(
            cmd_rx.try_recv(),
            Ok(JsWorkerCommand::Tick { .. })
        ));
        {
            let mut manager = app
                .world_mut()
                .non_send_resource_mut::<ScriptRuntimeManager>();
            let worker = manager.contexts.get_mut(&42).unwrap();
            worker
                .try_send(JsWorkerCommand::PushDomToqueEvents(vec![(
                    1, 0.0, 0.0, 0.0,
                )]))
                .unwrap();
            worker.needs_tick = true;
        }
        assert!(matches!(
            cmd_rx.try_recv(),
            Ok(JsWorkerCommand::PushDomToqueEvents(_))
        ));
        event_tx
            .send(JsWorkerEvent::TickData(JsTickData {
                audio_commands: Vec::new(),
                mesh_commands: (0, Vec::new()),
                keyboard_commands: Vec::new(), pose_batches: Vec::new(),
                needs_continuous_ticks: false,
                logs: Vec::new(),
                attr_updates: Vec::new(),
                pos_updates: Vec::new(),
                rot_updates: Vec::new(),
                scale_updates: Vec::new(),
                creation_queue: Vec::new(),
                hierarchy_queue: Vec::new(),
                remove_queue: Vec::new(),
                fetch_queue: Vec::new(),
                navigate_queue: Vec::new(),
                world_navigation: Vec::new(),
                tab_action_queue: Vec::new(),
                capture_queue: Vec::new(),
                shell_outbox: Vec::new(),
                ws_connect_queue: Vec::new(),
                ws_send_queue: Vec::new(),
                ws_close_queue: Vec::new(),
            }))
            .unwrap();
        js_tick_system(app.world_mut());
        js_tick_system(app.world_mut());
        assert!(
            matches!(cmd_rx.try_recv(), Ok(JsWorkerCommand::Tick { .. })),
            "a click arriving during the previous tick must schedule a new pump"
        );
    }

    #[test]
    fn embedded_binding_rejects_stale_slots_and_foreign_anchors() {
        use specs::Builder;
        let mut specs = build_world();
        let shell = specs.create_entity().with(Tag("space".into())).with(Hierarchy::default()).build();
        let app = specs.create_entity().with(Tag("space".into())).with(Hierarchy::default()).build();
        let frame = specs.create_entity().with(Tag("group".into())).with(Hierarchy::default()).build();
        Hierarchy::add_child(&mut specs, shell, frame);
        let mut world = World::new();
        world.insert_resource(ElemenetWorld(specs));
        let mut tables = SpaceHandleTables::default();
        let mut table = SpaceHandleTable::default();
        table.runtime_id = 1;
        table.local_to_global.insert(10, frame.id());
        table.local_to_global.insert(11, app.id());
        tables.by_space.insert(shell.id(), table);
        world.insert_resource(tables);
        let slot = serde_json::json!({"anchorNodeId":10,"revision":2});
        assert!(bind_embedded_slot(&mut world, shell.id(), app.id(), &slot));
        assert!(!bind_embedded_slot(&mut world, shell.id(), app.id(), &slot));
        assert!(!bind_embedded_slot(&mut world, shell.id(), app.id(),
            &serde_json::json!({"anchorNodeId":10,"revision":1})));
        assert!(!bind_embedded_slot(&mut world, shell.id(), app.id(),
            &serde_json::json!({"anchorNodeId":11,"revision":3})));
        world.resource_mut::<SpaceHandleTables>().by_space.get_mut(&shell.id()).unwrap().runtime_id = 2;
        assert!(bind_embedded_slot(&mut world, shell.id(), app.id(),
            &serde_json::json!({"anchorNodeId":10,"revision":1})));
    }

    fn worker_eval(worker: &mut SpaceScriptWorker, code: String) {
        worker
            .try_send(JsWorkerCommand::EvalScript {
                url: "eval://regression".into(),
                code,
            })
            .unwrap();
        match worker
            .event_rx
            .recv_timeout(Duration::from_secs(10))
            .unwrap()
        {
            JsWorkerEvent::EvalResult { error: None, .. } => {}
            JsWorkerEvent::WorkerError(error) => panic!("{error}"),
            _ => panic!("eval failed"),
        }
    }

    fn worker_tick(worker: &mut SpaceScriptWorker, elapsed_ms: f64) -> JsTickData {
        worker
            .try_send(JsWorkerCommand::Tick { elapsed_ms })
            .unwrap();
        match worker
            .event_rx
            .recv_timeout(Duration::from_secs(10))
            .unwrap()
        {
            JsWorkerEvent::TickData(data) => data,
            JsWorkerEvent::WorkerError(error) => panic!("{error}"),
            _ => panic!("tick failed"),
        }
    }

    #[test]
    fn finite_slow_animation_does_not_destroy_worker_or_click_listeners() {
        let mut worker = spawn_space_worker(90_010).unwrap();
        worker_eval(
            &mut worker,
            r#"
                const root = hiperspace.dimention;
                root.addEventListener('toque', () => console.log('CLICK_ALIVE'));
                requestAnimationFrame(function frame() {
                    const until = Date.now() + 80;
                    while (Date.now() < until) {}
                    console.log('FRAME_ALIVE');
                    requestAnimationFrame(frame);
                });
            "#
            .into(),
        );
        let first = worker_tick(&mut worker, 16.0);
        assert!(first.needs_continuous_ticks);
        worker
            .try_send(JsWorkerCommand::PushDomToqueEvents(vec![(
                0, 0.0, 0.0, 0.0,
            )]))
            .unwrap();
        let second = worker_tick(&mut worker, 32.0);
        assert!(second.needs_continuous_ticks);
        assert!(second
            .logs
            .iter()
            .any(|(_, msg)| msg.contains("CLICK_ALIVE")));
        assert!(second
            .logs
            .iter()
            .any(|(_, msg)| msg.contains("FRAME_ALIVE")));
        stop_space_worker(&mut worker);
    }

    #[test]
    fn scale_demo_5000_nodes_keeps_animating_and_handles_controls() {
        let mut worker = spawn_space_worker(90_011).unwrap();
        let controls = [
            "demo_toggle_anim",
            "demo_recolor",
            "demo_anim",
            "demo_count",
            "demo_status",
            "demo_hint",
            "demo_clear",
        ];
        let mut patch = SpaceSnapshotPatch::default();
        patch.tag_updates.insert(0, "space".into());
        let mut children = Vec::new();
        for (i, name) in controls.iter().enumerate() {
            let id = i as i32 + 1;
            patch
                .attr_updates
                .insert(id, HashMap::from([("id".into(), name.to_string())]));
            patch.tag_updates.insert(id, "box".into());
            patch.parents.insert(id, 0);
            children.push(id);
        }
        for id in 100..5100 {
            patch.tag_updates.insert(id, "box".into());
            patch.parents.insert(id, 0);
            children.push(id);
        }
        patch.children.insert(0, children);
        worker
            .try_send(JsWorkerCommand::PatchSnapshots(patch))
            .unwrap();
        assert!(matches!(
            worker.event_rx.recv_timeout(Duration::from_secs(10)),
            Ok(JsWorkerEvent::SnapshotApplied)
        ));
        let document = crate::routes::VIRTUAL_ROUTES
            .resolve("luna://scale_demo")
            .unwrap();
        let script = document
            .split_once("<script>")
            .unwrap()
            .1
            .split_once("</script>")
            .unwrap()
            .0;
        worker_eval(
            &mut worker,
            format!(
                r#"{script}
                for (const el of root.children) {{
                    if (el._nodeId >= 100) dynamicNodes.push({{ el, bx: 1, by: 2, bz: 3, off: el._nodeId * 0.38 }});
                }}
            "#
            ),
        );
        worker
            .try_send(JsWorkerCommand::PushDomToqueEvents(vec![(
                1, 0.0, 0.0, 0.0,
            )]))
            .unwrap();
        assert!(worker_tick(&mut worker, 16.0).needs_continuous_ticks);
        let first = worker_tick(&mut worker, 32.0);
        let second = worker_tick(&mut worker, 640.0);
        assert_eq!(first.pos_updates.len(), 5000);
        assert_eq!(second.pos_updates.len(), 5000);
        assert_ne!(first.rot_updates[0].1.y, second.rot_updates[0].1.y);
        worker
            .try_send(JsWorkerCommand::PushDomToqueEvents(vec![(
                2, 0.0, 0.0, 0.0,
            )]))
            .unwrap();
        let recolor = worker_tick(&mut worker, 656.0);
        assert_eq!(
            recolor
                .attr_updates
                .iter()
                .filter(|(id, key, _)| *id >= 100 && key == "color")
                .count(),
            5000
        );
        worker
            .try_send(JsWorkerCommand::PushDomToqueEvents(vec![(
                1, 0.0, 0.0, 0.0,
            )]))
            .unwrap();
        worker_tick(&mut worker, 672.0);
        assert!(!worker_tick(&mut worker, 688.0).needs_continuous_ticks);
        worker
            .try_send(JsWorkerCommand::PushDomToqueEvents(vec![(
                7, 0.0, 0.0, 0.0,
            )]))
            .unwrap();
        assert_eq!(worker_tick(&mut worker, 704.0).remove_queue.len(), 5000);
        stop_space_worker(&mut worker);
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
    fn changing_one_space_does_not_send_empty_patches_to_other_spaces() {
        let (mut app, spaces) = snapshot_test_app_with_spaces(2);
        let (a, a_commands, a_events) = fake_worker(false);
        let (b, b_commands, b_events) = fake_worker(false);
        {
            let mut manager = app.world_mut().non_send_resource_mut::<ScriptRuntimeManager>();
            manager.contexts.insert(spaces[0], a);
            manager.contexts.insert(spaces[1], b);
        }
        js_update_snapshots_system(app.world_mut());
        assert!(matches!(a_commands.try_recv(), Ok(JsWorkerCommand::UpdateSnapshots(_))));
        assert!(matches!(b_commands.try_recv(), Ok(JsWorkerCommand::UpdateSnapshots(_))));
        a_events.send(JsWorkerEvent::SnapshotApplied).unwrap();
        b_events.send(JsWorkerEvent::SnapshotApplied).unwrap();
        js_tick_system(app.world_mut());
        {
            let dom = app.world().resource::<ElemenetWorld>();
            let node = dom.0.entities().entity(spaces[0]);
            dom.0.write_storage::<Attrs>().get_mut(node).unwrap().0.insert("title".into(), "changed".into());
        }
        app.world_mut().resource_mut::<DomMirrorDirty>().touch(spaces[0]);
        app.world_mut().resource_mut::<JsSnapshotState>().dirty = true;
        js_update_snapshots_system(app.world_mut());
        assert!(matches!(a_commands.try_recv(), Ok(JsWorkerCommand::PatchSnapshots(_))));
        assert!(matches!(b_commands.try_recv(), Err(mpsc::TryRecvError::Empty)));
        assert!(!app.world().non_send_resource::<ScriptRuntimeManager>().contexts[&spaces[1]].snapshot_in_flight);
        assert_eq!(app.world().resource::<crate::PerformanceStats>().snapshots_sent, 1);
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
    fn removing_skybox_through_js_clears_active_owner_and_pending_preparation() {
        let (mut app, space_id, child_id, _, _) = snapshot_test_app_with_child();
        app.init_resource::<crate::EntityMap>();
        app.init_resource::<crate::ScriptLoadStates>();
        app.init_resource::<crate::PendingModelLoads>();
        app.init_resource::<crate::ModelLoadStates>();
        app.init_resource::<crate::SkyboxEntity>();
        let bevy_entity = app.world_mut().spawn_empty().id();
        app.world_mut().resource_mut::<crate::EntityMap>().0.insert(child_id, bevy_entity);
        {
            let mut sky = app.world_mut().resource_mut::<crate::SkyboxEntity>();
            sky.active = Some((child_id, bevy_entity));
            sky.nodes.insert(child_id, crate::SkyboxNodeState {
                key: "old-sky".into(), status: crate::SkyboxLoadStatus::Requested, mounted: None,
            });
            sky.enqueue("old-sky", child_id);
            sky.enqueue("other-sky", 9000);
        }
        let (worker, _commands, events) = fake_worker(false);
        app.world_mut().non_send_resource_mut::<ScriptRuntimeManager>().contexts.insert(space_id, worker);
        js_update_snapshots_system(app.world_mut());
        let local = app.world().resource::<SpaceHandleTables>().by_space[&space_id].global_to_local[&child_id];
        events.send(JsWorkerEvent::TickData(JsTickData {
                audio_commands: Vec::new(),
                mesh_commands: (0, Vec::new()),
                keyboard_commands: Vec::new(), pose_batches: Vec::new(),
                needs_continuous_ticks: false,
                logs: Vec::new(),
                attr_updates: Vec::new(),
                pos_updates: Vec::new(),
                rot_updates: Vec::new(),
                scale_updates: Vec::new(),
                creation_queue: Vec::new(),
                hierarchy_queue: Vec::new(),
                remove_queue: vec![local],
                fetch_queue: Vec::new(),
                navigate_queue: Vec::new(),
                world_navigation: Vec::new(),
                tab_action_queue: Vec::new(),
                capture_queue: Vec::new(),
                shell_outbox: Vec::new(),
                ws_connect_queue: Vec::new(),
                ws_send_queue: Vec::new(),
                ws_close_queue: Vec::new(),
            })).unwrap();
        js_tick_system(app.world_mut());
        let sky = app.world().resource::<crate::SkyboxEntity>();
        assert!(sky.active.is_none());
        assert!(!sky.nodes.contains_key(&child_id));
        assert!(!sky.pending.contains_key("old-sky"));
        assert!(sky.pending.contains_key("other-sky"));
        assert!(app.world().get_entity(bevy_entity).is_none());
    }

    #[test]
    fn removing_pending_js_node_does_not_leave_orphaned_geometry() {
        let (mut app, space_id, child_id, _, _) = snapshot_test_app_with_child();
        app.init_resource::<crate::EntityMap>();
        app.init_resource::<crate::ScriptLoadStates>();
        app.init_resource::<crate::PendingModelLoads>();
        app.init_resource::<crate::ModelLoadStates>();
        app.init_resource::<crate::SkyboxEntity>();
        let bevy_entity = app.world_mut().spawn_empty().id();
        app.world_mut().resource_mut::<crate::EntityMap>().0.insert(child_id, bevy_entity);
        {
            let mut sky = app.world_mut().resource_mut::<crate::SkyboxEntity>();
            sky.active = Some((child_id, bevy_entity));
            sky.nodes.insert(child_id, crate::SkyboxNodeState {
                key: "old-sky".into(), status: crate::SkyboxLoadStatus::Requested, mounted: None,
            });
            sky.enqueue("old-sky", child_id);
            sky.enqueue("other-sky", 9000);
        }
        let (worker, _commands, events) = fake_worker(false);
        app.world_mut().non_send_resource_mut::<ScriptRuntimeManager>().contexts.insert(space_id, worker);
        js_update_snapshots_system(app.world_mut());
        let local = app.world().resource::<SpaceHandleTables>().by_space[&space_id].global_to_local[&child_id];
        // A newly created element has a handle before the deferred DOM commit.
        app.world_mut().resource_mut::<VirtualDomData>().nodes.remove(&child_id);
        app.world_mut().resource_mut::<SpaceHandleTables>().by_space.get_mut(&space_id)
            .unwrap().detached_globals.insert(child_id);
        events.send(JsWorkerEvent::TickData(JsTickData {
                audio_commands: Vec::new(),
                mesh_commands: (0, Vec::new()),
                keyboard_commands: Vec::new(), pose_batches: Vec::new(),
                needs_continuous_ticks: false,
                logs: Vec::new(),
                attr_updates: Vec::new(),
                pos_updates: Vec::new(),
                rot_updates: Vec::new(),
                scale_updates: Vec::new(),
                creation_queue: Vec::new(),
                hierarchy_queue: Vec::new(),
                remove_queue: vec![local],
                fetch_queue: Vec::new(),
                navigate_queue: Vec::new(),
                world_navigation: Vec::new(),
                tab_action_queue: Vec::new(),
                capture_queue: Vec::new(),
                shell_outbox: Vec::new(),
                ws_connect_queue: Vec::new(),
                ws_send_queue: Vec::new(),
                ws_close_queue: Vec::new(),
            })).unwrap();
        js_tick_system(app.world_mut());
        let sky = app.world().resource::<crate::SkyboxEntity>();
        assert!(sky.active.is_none());
        assert!(!sky.nodes.contains_key(&child_id));
        assert!(!sky.pending.contains_key("old-sky"));
        assert!(sky.pending.contains_key("other-sky"));
        assert!(app.world().get_entity(bevy_entity).is_none());
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
