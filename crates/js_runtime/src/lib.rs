use std::{
    cell::RefCell,
    collections::{BTreeSet, HashMap, HashSet},
    rc::Rc,
    sync::OnceLock,
    time::{Duration, Instant},
};

use anyhow::Result as AnyResult;
use deno_core::{op2, Extension, FastString, JsRuntime, OpState, RuntimeOptions};

// Public modules
pub mod cache;
pub mod csp;
pub mod storage;
mod location;
pub mod components;
pub mod ui_text;
pub mod audio;
pub mod binary;
pub mod settings;
pub mod keyboard;
pub mod fetch;
pub use fetch::{FetchRequest, FetchResponse};
use deno_core::Op;
pub mod mesh;
pub mod pose;
pub mod world_navigation;

pub use cache::{CacheStats, CachedResponse, CachedScript, FetchCache, ScriptCache};
pub use csp::{ContentSecurityPolicy, CorsValidation, CorsValidator};

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

type Shared<T> = Rc<RefCell<T>>;

#[inline]
fn shared<T>(value: T) -> Shared<T> {
    Rc::new(RefCell::new(value))
}

#[inline]
fn take_vec<T>(cell: &Shared<Vec<T>>) -> Vec<T> {
    std::mem::take(&mut *cell.borrow_mut())
}

#[inline]
fn take_map<K, V>(cell: &Shared<HashMap<K, V>>) -> HashMap<K, V> {
    std::mem::take(&mut *cell.borrow_mut())
}

#[inline]
fn replace_map<K, V>(cell: &Shared<HashMap<K, V>>, incoming: HashMap<K, V>)
where
    K: Eq + std::hash::Hash,
{
    let mut dst = cell.borrow_mut();
    dst.clear();
    dst.extend(incoming);
}

#[inline]
fn patch_map<K, V>(cell: &Shared<HashMap<K, V>>, incoming: HashMap<K, V>)
where
    K: Eq + std::hash::Hash,
{
    if incoming.is_empty() {
        return;
    }
    cell.borrow_mut().extend(incoming);
}

#[inline]
fn remove_map_keys<K, V>(cell: &Shared<HashMap<K, V>>, keys: &HashSet<K>)
where
    K: Eq + std::hash::Hash,
{
    if keys.is_empty() {
        return;
    }
    let mut dst = cell.borrow_mut();
    for key in keys {
        dst.remove(key);
    }
}

// ---------------------------------------------------------------------------
// State structs for OpState
// ---------------------------------------------------------------------------

// Timer callbacks already run only during the isolate's pump. Keeping deadlines
// here avoids a sleeping OS thread per timeout and retains nothing after cancel.
#[derive(Default)]
struct TimerQueue {
    deadlines: BTreeSet<(Instant, i32)>,
    scheduled: HashMap<i32, Instant>,
}

impl TimerQueue {
    fn schedule(&mut self, id: i32, deadline: Instant) {
        self.cancel(id);
        self.scheduled.insert(id, deadline);
        self.deadlines.insert((deadline, id));
    }

    fn cancel(&mut self, id: i32) {
        if let Some(deadline) = self.scheduled.remove(&id) {
            self.deadlines.remove(&(deadline, id));
        }
    }

    fn drain_ready(&mut self, now: Instant) -> Vec<i32> {
        let mut ready = Vec::new();
        while self.deadlines.first().is_some_and(|(deadline, _)| *deadline <= now) {
            let (_, id) = self.deadlines.pop_first().unwrap();
            self.scheduled.remove(&id);
            ready.push(id);
        }
        ready
    }

    fn next_deadline(&self) -> Option<Instant> {
        self.deadlines.first().map(|(deadline, _)| *deadline)
    }
}

#[cfg(test)]
mod timer_tests {
    use super::*;

    #[test]
    fn timer_queue_orders_deadlines_and_drops_cancelled_entries() {
        let now = Instant::now();
        let mut queue = TimerQueue::default();
        queue.schedule(1, now + Duration::from_secs(2));
        queue.schedule(2, now + Duration::from_secs(1));
        queue.schedule(3, now);
        queue.cancel(2);
        assert_eq!(queue.drain_ready(now), vec![3]);
        assert_eq!(queue.next_deadline(), Some(now + Duration::from_secs(2)));
        assert!(queue.drain_ready(now + Duration::from_secs(1)).is_empty());
        assert_eq!(queue.drain_ready(now + Duration::from_secs(2)), vec![1]);
        assert!(queue.scheduled.is_empty());
        assert!(queue.deadlines.is_empty());
    }

    #[test]
    fn timer_queue_reuses_ids_and_retains_no_cancelled_timers() {
        let now = Instant::now();
        let mut queue = TimerQueue::default();
        for id in 0..10000 {
            queue.schedule(id, now + Duration::from_secs(3600));
            queue.cancel(id);
        }
        assert!(queue.scheduled.is_empty());
        assert!(queue.deadlines.is_empty());
        queue.schedule(7, now);
        queue.schedule(7, now + Duration::from_secs(1));
        assert!(queue.drain_ready(now).is_empty());
        assert_eq!(queue.drain_ready(now + Duration::from_secs(1)), vec![7]);
    }
}

struct Timers {
    queue: Shared<TimerQueue>,
}
impl Default for Timers {
    fn default() -> Self {
        Self {
            queue: shared(TimerQueue::default()),
        }
    }
}

struct RafState {
    pending: Shared<Vec<i32>>,
    ready: Shared<Vec<(i32, f64)>>,
}
impl Default for RafState {
    fn default() -> Self {
        Self {
            pending: shared(Vec::new()),
            ready: shared(Vec::new()),
        }
    }
}

struct PerfState {
    start: Instant,
}
impl Default for PerfState {
    fn default() -> Self {
        Self {
            start: Instant::now(),
        }
    }
}

/// Captured console output: Vec<(level, message)>
pub struct ConsoleState {
    pub logs: Shared<Vec<(String, String)>>,
}
impl Default for ConsoleState {
    fn default() -> Self {
        Self {
            logs: shared(Vec::new()),
        }
    }
}

/// DOM attribute mutations from JS: Vec<(node_id, key, value)>
pub struct AttrUpdates {
    pub updates: Shared<Vec<(i32, String, String)>>,
}
impl Default for AttrUpdates {
    fn default() -> Self {
        Self {
            updates: shared(Vec::new()),
        }
    }
}

/// Read-only snapshot of attributes for JS to query
pub struct AttrSnapshot {
    pub data: Shared<HashMap<i32, HashMap<String, String>>>,
}
impl Default for AttrSnapshot {
    fn default() -> Self {
        Self {
            data: shared(HashMap::new()),
        }
    }
}

/// Queue of createElement(tag) commands: Vec<(request_id, tag_name)>
pub struct ElementCreationQueue {
    pub queue: Shared<Vec<(i32, String)>>,
    next_request_id: Shared<i32>,
}
impl Default for ElementCreationQueue {
    fn default() -> Self {
        Self {
            queue: shared(Vec::new()),
            next_request_id: shared(1),
        }
    }
}

/// Results of createElement: HashMap<request_id, node_id>
pub struct ElementCreationResults {
    pub results: Shared<HashMap<i32, i32>>,
}
impl Default for ElementCreationResults {
    fn default() -> Self {
        Self {
            results: shared(HashMap::new()),
        }
    }
}

/// Queue of appendChild/removeChild commands
pub struct HierarchyUpdateQueue {
    pub append: Shared<Vec<(i32, i32)>>,
    pub remove_child: Shared<Vec<(i32, i32)>>,
}
impl Default for HierarchyUpdateQueue {
    fn default() -> Self {
        Self {
            append: shared(Vec::new()),
            remove_child: shared(Vec::new()),
        }
    }
}

/// Queue of remove(node_id) commands
pub struct RemoveElementQueue {
    pub queue: Shared<Vec<i32>>,
}
impl Default for RemoveElementQueue {
    fn default() -> Self {
        Self {
            queue: shared(Vec::new()),
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

pub struct TransformSnapshot {
    pub positions: Shared<HashMap<i32, Vec3>>,
    pub rotations: Shared<HashMap<i32, Vec3>>,
    pub scales: Shared<HashMap<i32, Vec3>>,
    pub global_positions: Shared<HashMap<i32, Vec3>>,
}
impl Default for TransformSnapshot {
    fn default() -> Self {
        Self {
            positions: shared(HashMap::new()),
            rotations: shared(HashMap::new()),
            scales: shared(HashMap::new()),
            global_positions: shared(HashMap::new()),
        }
    }
}

/// Transform mutation queue
pub struct TransformUpdateQueue {
    pub positions: Shared<Vec<(i32, Vec3)>>,
    pub rotations: Shared<Vec<(i32, Vec3)>>,
    pub scales: Shared<Vec<(i32, Vec3)>>,
    pub global_positions: Shared<Vec<(i32, Vec3)>>,
}
impl Default for TransformUpdateQueue {
    fn default() -> Self {
        Self {
            positions: shared(Vec::new()),
            rotations: shared(Vec::new()),
            scales: shared(Vec::new()),
            global_positions: shared(Vec::new()),
        }
    }
}

pub struct HierarchySnapshot {
    pub parents: Shared<HashMap<i32, i32>>,
    pub children: Shared<HashMap<i32, Vec<i32>>>,
}
impl Default for HierarchySnapshot {
    fn default() -> Self {
        Self {
            parents: shared(HashMap::new()),
            children: shared(HashMap::new()),
        }
    }
}

pub struct TagSnapshot {
    pub tags: Shared<HashMap<i32, String>>,
}
impl Default for TagSnapshot {
    fn default() -> Self {
        Self {
            tags: shared(HashMap::new()),
        }
    }
}

pub struct FetchQueue {
    pub requests: Shared<Vec<(i32, FetchRequest)>>,
    next_request_id: Shared<i32>,
}
impl Default for FetchQueue {
    fn default() -> Self {
        Self {
            requests: shared(Vec::new()),
            next_request_id: shared(1),
        }
    }
}

pub struct FetchResults {
    pub results: Shared<HashMap<i32, std::result::Result<FetchResponse, String>>>,
}
impl Default for FetchResults {
    fn default() -> Self {
        Self {
            results: shared(HashMap::new()),
        }
    }
}

/// Pedidos de captura de frame. Misma forma que FetchQueue: el worker encola
/// (request_id, nombre) y el host responde por CaptureResults. La captura de
/// Bevy tarda uno o dos frames, así que no puede ser síncrona.
pub struct CaptureQueue {
    pub requests: Shared<Vec<(i32, String)>>,
    next_request_id: Shared<i32>,
}
impl Default for CaptureQueue {
    fn default() -> Self {
        Self {
            requests: shared(Vec::new()),
            next_request_id: shared(1),
        }
    }
}

/// Resultado de cada captura: Ok(ruta absoluta del PNG) o Err(motivo).
pub struct CaptureResults {
    pub results: Shared<HashMap<i32, std::result::Result<String, String>>>,
}
impl Default for CaptureResults {
    fn default() -> Self {
        Self {
            results: shared(HashMap::new()),
        }
    }
}

pub struct NavigateQueue {
    pub queue: Shared<Vec<String>>,
}
impl Default for NavigateQueue {
    fn default() -> Self {
        Self {
            queue: shared(Vec::new()),
        }
    }
}

/// chrome.tabs-like requests desde JS. El host valida cap y traduce a
/// SpaceMountQueue / SpaceUnmountQueue / etc.
#[derive(Clone, Debug)]
pub enum TabAction {
    /// Host-only settings action; the host verifies the source document.
    SetMcpEnabled { enabled: bool },
    SetMcpAutoStart { enabled: bool },
    /// Parche de la configuración raíz, en JSON. Va como texto y no como campos
    /// para que agregar una preferencia no toque este enum ni el runtime: el
    /// host parsea y aplica lo que reconoce. Mismo control que las dos de
    /// arriba — sólo vale desde el documento de ajustes.
    SetRootSettings { patch: String },
    /// Abre nueva tab cargando url. `kind` opaco — JS shell aplica policy.
    Open { url: String, kind: String },
    /// Cierra tab por su tab_id.
    Close { tab_id: u64 },
    /// Cambia visibilidad de una tab existente.
    SetVisible { tab_id: u64, visible: bool },
    /// Setea pose (position + rotation Euler) del outer wrapper de una tab.
    /// Usado por el shell para posicionar apps embedded directamente,
    /// sin depender de que la app aplique pose en su isolate (evita race).
    SetPose {
        tab_id: u64,
        px: f32, py: f32, pz: f32,
        rx: f32, ry: f32, rz: f32,
    },
}

pub struct TabActionQueue {
    pub queue: Shared<Vec<TabAction>>,
}
impl Default for TabActionQueue {
    fn default() -> Self {
        Self {
            queue: shared(Vec::new()),
        }
    }
}

/// Snapshot de la pose del usuario (HMD en VR, cámara en desktop), actualizado
/// por el host cada frame para workers con cap READ_HMD_POSE. Workers sin la
/// cap mantienen `None` y `op_read_viewer_pose` devuelve null.
#[derive(Clone, Debug)]
pub struct ViewerPoseData {
    pub mode: String,         // "vr" | "desktop"
    pub px: f32,
    pub py: f32,
    pub pz: f32,
    pub forward_x: f32,
    pub forward_y: f32,
    pub forward_z: f32,
    pub yaw: f32,             // rotación Y (rad), 0 = mirando -Z
    pub pitch: f32,           // rotación X (rad), 0 = horizonte
    // VR extras (quaternion completo del HMD).
    pub qx: f32,
    pub qy: f32,
    pub qz: f32,
    pub qw: f32,
    // Desktop extras (proyección).
    pub aspect: f32,
    pub fov_y_rad: f32,
}

pub struct ViewerPoseState {
    pub data: Shared<Option<ViewerPoseData>>,
}
impl Default for ViewerPoseState {
    fn default() -> Self {
        Self {
            data: shared(None),
        }
    }
}

/// Mensajes shell ↔ embedded app. El JS los serializa como JSON string;
/// el host los rutea por `target_tab_id` (apuntando al worker dueño del space
/// con ese tabId). Direction lo decide el caller — host no impone sentido.
#[derive(Clone, Debug)]
pub struct ShellMessage {
    /// Si caller es app: dejar `target_tab_id = 0` → rutea al shell.
    /// Si caller es shell: target_tab_id = tab id de la app destino.
    pub target_tab_id: u64,
    /// Payload arbitrario (JSON string). Convención: `{ "type": "...", ... }`.
    pub payload: String,
}

pub struct ShellMessageOutbox {
    pub queue: Shared<Vec<ShellMessage>>,
}
impl Default for ShellMessageOutbox {
    fn default() -> Self {
        Self { queue: shared(Vec::new()) }
    }
}

pub struct ShellMessageInbox {
    /// Mensajes que llegan al worker, leídos vía `op_shell_poll_messages`.
    pub queue: Shared<Vec<ShellMessage>>,
}
impl Default for ShellMessageInbox {
    fn default() -> Self {
        Self { queue: shared(Vec::new()) }
    }
}

/// Queue of ws connect requests from JS: Vec<(conn_id, url)>
pub struct WsConnectQueue {
    pub requests: Shared<Vec<(i32, String)>>,
    next_id: Shared<i32>,
}
impl Default for WsConnectQueue {
    fn default() -> Self {
        Self {
            requests: shared(Vec::new()),
            next_id: shared(1),
        }
    }
}

/// Inbox of messages received from remote: HashMap<conn_id, Vec<message>>
pub struct WsInbox {
    pub messages: Shared<HashMap<i32, Vec<String>>>,
}
impl Default for WsInbox {
    fn default() -> Self {
        Self {
            messages: shared(HashMap::new()),
        }
    }
}

/// Queue of messages JS wants to send: Vec<(conn_id, message)>
pub struct WsSendQueue {
    pub queue: Shared<Vec<(i32, String)>>,
}
impl Default for WsSendQueue {
    fn default() -> Self {
        Self {
            queue: shared(Vec::new()),
        }
    }
}

/// Status per connection
pub struct WsStatusMap {
    pub status: Shared<HashMap<i32, String>>,
}
impl Default for WsStatusMap {
    fn default() -> Self {
        Self {
            status: shared(HashMap::new()),
        }
    }
}

/// Queue of close requests from JS
pub struct WsCloseQueue {
    pub queue: Shared<Vec<i32>>,
}
impl Default for WsCloseQueue {
    fn default() -> Self {
        Self {
            queue: shared(Vec::new()),
        }
    }
}

#[derive(Clone, Debug)]
pub struct DomEvent {
    pub local: Option<[f32;3]>,
    // posemove: dirección y giro del mando en el marco del padre del posezone
    // (la posición va en `local`).
    pub local_d: Option<[f32;3]>,
    pub local_q: Option<[f32;4]>,
    pub event_type: String,
    pub node_id: i32,
    pub x: Option<f32>,
    pub y: Option<f32>,
    pub z: Option<f32>,
    pub hand: Option<String>,
    pub px: Option<f32>,
    pub py: Option<f32>,
    pub pz: Option<f32>,
    pub dx: Option<f32>,
    pub dy: Option<f32>,
    pub dz: Option<f32>,
    pub trigger: Option<f32>,
    pub grip: Option<f32>,
    // Orientación del controlador (cuaternión), solo en posemove.
    pub qx: Option<f32>,
    pub qy: Option<f32>,
    pub qz: Option<f32>,
    pub qw: Option<f32>,
    // systeminput: acción semántica normalizada (ej. "shell") y origen
    // hardware-agnóstico ("vr_menu" | "kb_escape" | ...).
    pub action: Option<String>,
    pub source: Option<String>,
}

/// DOM events normalizados host -> JS runtime
pub struct DomEventQueue {
    pub events: Shared<Vec<DomEvent>>,
}
impl Default for DomEventQueue {
    fn default() -> Self {
        Self {
            events: shared(Vec::new()),
        }
    }
}

/// Touch raw events pushed from Rust into JS
pub struct TouchEventQueue {
    pub events: Shared<Vec<(i32, f32, f32, f32)>>,
}
impl Default for TouchEventQueue {
    fn default() -> Self {
        Self {
            events: shared(Vec::new()),
        }
    }
}

// ---------------------------------------------------------------------------
// Ops
// ---------------------------------------------------------------------------

// --- Console ops ---

#[op2(fast)]
fn op_console_log(state: &mut OpState, #[string] msg: &str) {
    let console = state.borrow::<ConsoleState>();
    console
        .logs
        .borrow_mut()
        .push(("log".to_string(), msg.to_string()));
}

#[op2(fast)]
fn op_console_warn(state: &mut OpState, #[string] msg: &str) {
    let console = state.borrow::<ConsoleState>();
    console
        .logs
        .borrow_mut()
        .push(("warn".to_string(), msg.to_string()));
}

#[op2(fast)]
fn op_console_error(state: &mut OpState, #[string] msg: &str) {
    let console = state.borrow::<ConsoleState>();
    console
        .logs
        .borrow_mut()
        .push(("error".to_string(), msg.to_string()));
}

// --- Performance ops ---

#[op2(fast)]
fn op_now(state: &mut OpState) -> f64 {
    let perf = state.borrow::<PerfState>();
    perf.start.elapsed().as_secs_f64() * 1000.0
}

// --- Timer ops ---

#[op2(fast)]
fn op_set_timeout(state: &mut OpState, #[smi] id: i32, #[smi] ms: i32) {
    let timers = state.borrow::<Timers>();
    timers.queue.borrow_mut().schedule(id,
        Instant::now() + Duration::from_millis(ms.max(0) as u64));
}

#[op2(fast)]
fn op_clear_timeout(state: &mut OpState, #[smi] id: i32) {
    let timers = state.borrow::<Timers>();
    timers.queue.borrow_mut().cancel(id);
}

#[op2]
#[serde]
fn op_timers_poll(state: &mut OpState) -> serde_json::Value {
    let timers = state.borrow::<Timers>();
    let ids = timers.queue.borrow_mut().drain_ready(Instant::now());
    serde_json::Value::from(ids)
}

// --- RAF ops ---

#[op2(fast)]
fn op_raf_register(state: &mut OpState, #[smi] id: i32) {
    let raf = state.borrow::<RafState>();
    raf.pending.borrow_mut().push(id);
}

#[op2]
#[serde]
fn op_raf_poll(state: &mut OpState) -> serde_json::Value {
    let raf = state.borrow::<RafState>();
    let list = take_vec(&raf.ready);
    serde_json::to_value(list).unwrap()
}

// --- Attribute ops ---

#[op2(fast)]
fn op_hsml_set_attr(
    state: &mut OpState,
    #[smi] node_id: i32,
    #[string] key: &str,
    #[string] value: &str,
) -> Result<(), anyhow::Error> {
    if key == "props" { components::parse_json(value, true).map_err(anyhow::Error::msg)?; }
    if key == "events" { components::parse_events(value).map_err(anyhow::Error::msg)?; }
    let updates = state.borrow::<AttrUpdates>();
    updates
        .updates
        .borrow_mut()
        .push((node_id, key.to_string(), value.to_string()));
    Ok(())
}

#[op2]
#[string]
fn op_hsml_get_attr(state: &mut OpState, #[smi] node_id: i32, #[string] key: &str) -> String {
    let snap = state.borrow::<AttrSnapshot>();
    snap.data
        .borrow()
        .get(&node_id)
        .and_then(|m| m.get(key))
        .cloned()
        .unwrap_or_default()
}

// --- Element creation ops ---

#[op2(fast)]
fn op_hsml_create_element(state: &mut OpState, #[string] tag: &str) -> i32 {
    let queue = state.borrow::<ElementCreationQueue>();
    let mut next_id = queue.next_request_id.borrow_mut();
    let request_id = *next_id;
    *next_id += 1;
    queue
        .queue
        .borrow_mut()
        .push((request_id, tag.to_string()));
    request_id
}

#[op2(fast)]
fn op_hsml_poll_created_element(state: &mut OpState, #[smi] request_id: i32) -> i32 {
    let results = state.borrow::<ElementCreationResults>();
    results
        .results
        .borrow_mut()
        .remove(&request_id)
        .unwrap_or(-1)
}

// --- Hierarchy ops ---

#[op2(fast)]
fn op_hsml_append_child(state: &mut OpState, #[smi] parent_id: i32, #[smi] child_id: i32) {
    let queue = state.borrow::<HierarchyUpdateQueue>();
    queue.append.borrow_mut().push((parent_id, child_id));
}

#[op2(fast)]
fn op_hsml_remove(state: &mut OpState, #[smi] node_id: i32) {
    let queue = state.borrow::<RemoveElementQueue>();
    queue.queue.borrow_mut().push(node_id);
}

#[op2]
#[serde]
fn op_hsml_get_children(state: &mut OpState, #[smi] node_id: i32) -> Vec<i32> {
    let snap = state.borrow::<HierarchySnapshot>();
    snap.children
        .borrow()
        .get(&node_id)
        .cloned()
        .unwrap_or_default()
}

#[op2(fast)]
fn op_hsml_get_parent(state: &mut OpState, #[smi] node_id: i32) -> i32 {
    let snap = state.borrow::<HierarchySnapshot>();
    snap.parents.borrow().get(&node_id).copied().unwrap_or(-1)
}

// --- Tag ops ---

#[op2]
#[string]
fn op_hsml_get_tag(state: &mut OpState, #[smi] node_id: i32) -> String {
    let snap = state.borrow::<TagSnapshot>();
    snap.tags.borrow().get(&node_id).cloned().unwrap_or_default()
}

// --- Transform ops (getters) ---

#[op2]
#[serde]
fn op_hsml_get_position(state: &mut OpState, #[smi] node_id: i32) -> Vec<f32> {
    let snap = state.borrow::<TransformSnapshot>();
    let pos = snap
        .positions
        .borrow()
        .get(&node_id)
        .copied()
        .unwrap_or_default();
    vec![pos.x, pos.y, pos.z]
}

#[op2]
#[serde]
fn op_hsml_get_rotation(state: &mut OpState, #[smi] node_id: i32) -> Vec<f32> {
    let snap = state.borrow::<TransformSnapshot>();
    let rot = snap
        .rotations
        .borrow()
        .get(&node_id)
        .copied()
        .unwrap_or_default();
    vec![rot.x, rot.y, rot.z]
}

#[op2]
#[serde]
fn op_hsml_get_scale(state: &mut OpState, #[smi] node_id: i32) -> Vec<f32> {
    let snap = state.borrow::<TransformSnapshot>();
    let scale = snap
        .scales
        .borrow()
        .get(&node_id)
        .copied()
        .unwrap_or(Vec3 {
            x: 1.0,
            y: 1.0,
            z: 1.0,
        });
    vec![scale.x, scale.y, scale.z]
}

#[op2]
#[serde]
fn op_hsml_get_global_position(state: &mut OpState, #[smi] node_id: i32) -> Vec<f32> {
    let snap = state.borrow::<TransformSnapshot>();
    let pos = snap
        .global_positions
        .borrow()
        .get(&node_id)
        .copied()
        .unwrap_or_default();
    vec![pos.x, pos.y, pos.z]
}

// --- Transform ops (setters) ---

#[op2(fast)]
fn op_hsml_set_position(state: &mut OpState, #[smi] node_id: i32, x: f32, y: f32, z: f32) {
    let queue = state.borrow::<TransformUpdateQueue>();
    queue
        .positions
        .borrow_mut()
        .push((node_id, Vec3 { x, y, z }));
}

#[op2(fast)]
fn op_hsml_set_rotation(state: &mut OpState, #[smi] node_id: i32, x: f32, y: f32, z: f32) {
    let queue = state.borrow::<TransformUpdateQueue>();
    queue
        .rotations
        .borrow_mut()
        .push((node_id, Vec3 { x, y, z }));
}

#[op2(fast)]
fn op_hsml_set_scale(state: &mut OpState, #[smi] node_id: i32, x: f32, y: f32, z: f32) {
    let queue = state.borrow::<TransformUpdateQueue>();
    queue
        .scales
        .borrow_mut()
        .push((node_id, Vec3 { x, y, z }));
}

#[op2(fast)]
fn op_hsml_set_global_position(
    state: &mut OpState,
    #[smi] node_id: i32,
    x: f32,
    y: f32,
    z: f32,
) {
    let queue = state.borrow::<TransformUpdateQueue>();
    queue
        .global_positions
        .borrow_mut()
        .push((node_id, Vec3 { x, y, z }));
}

// --- Transform batch op ---
// Conservado por compatibilidad, aunque ya no lo uses.
#[op2]
fn op_hsml_set_transform_batch(state: &mut OpState, #[serde] updates: Vec<f64>) {
    if updates.len() < 7 {
        return;
    }

    let queue = state.borrow::<TransformUpdateQueue>();
    let count = updates.len() / 7;

    let mut positions = queue.positions.borrow_mut();
    let mut rotations = queue.rotations.borrow_mut();
    positions.reserve(count);
    rotations.reserve(count);

    for chunk in updates.chunks_exact(7) {
        let node_id = chunk[0] as i32;
        positions.push((
            node_id,
            Vec3 {
                x: chunk[1] as f32,
                y: chunk[2] as f32,
                z: chunk[3] as f32,
            },
        ));
        rotations.push((
            node_id,
            Vec3 {
                x: chunk[4] as f32,
                y: chunk[5] as f32,
                z: chunk[6] as f32,
            },
        ));
    }
}

// --- Fetch ops ---

#[op2]
#[smi]
fn op_fetch_request(state: &mut OpState, #[serde] mut request: FetchRequest, #[buffer] bytes: &[u8], binary: bool) -> Result<i32, anyhow::Error> {
    if bytes.len() > 1024 * 1024 { return Err(anyhow::anyhow!("Fetch request exceeds 1 MiB")); }
    if binary { request.body_bytes = Some(bytes.to_vec()); }
    let queue = state.borrow::<FetchQueue>();
    let mut next_id = queue.next_request_id.borrow_mut();
    let request_id = *next_id;
    *next_id += 1;
    queue
        .requests
        .borrow_mut()
        .push((request_id, request));
    Ok(request_id)
}

#[derive(serde::Serialize)]
struct FetchPoll { status: &'static str, response: Option<FetchResponse>, error: Option<String> }

#[op2]
#[serde]
fn op_fetch_poll(state: &mut OpState, #[smi] request_id: i32) -> FetchPoll {
    let results = state.borrow::<FetchResults>();
    let mut map = results.results.borrow_mut();
    if let Some(result) = map.remove(&request_id) {
        match result {
            Ok(response) => FetchPoll {status:"ok", response:Some(response), error:None},
            Err(err) => FetchPoll {status:"error", response:None, error:Some(err)},
        }
    } else {
        FetchPoll {status:"pending", response:None, error:None}
    }
}

// --- Capture ops ---

#[op2(fast)]
fn op_luna_mcp_enabled(state: &mut OpState, enabled: bool) {
    state.borrow::<TabActionQueue>().queue.borrow_mut()
        .push(TabAction::SetMcpEnabled { enabled });
}
#[op2(fast)]
fn op_luna_mcp_auto_start(state: &mut OpState, enabled: bool) {
    state.borrow::<TabActionQueue>().queue.borrow_mut()
        .push(TabAction::SetMcpAutoStart { enabled });
}

/// El parche llega serializado. Se acota acá para que un documento no pueda
/// encolar megabytes; el host valida el contenido.
#[op2(fast)]
fn op_luna_root_settings(state: &mut OpState, #[string] patch: String) -> Result<(), anyhow::Error> {
    if patch.len() > 64 * 1024 {
        return Err(anyhow::anyhow!("Root settings patch exceeds 64 KiB"));
    }
    state.borrow::<TabActionQueue>().queue.borrow_mut()
        .push(TabAction::SetRootSettings { patch });
    Ok(())
}

// El host valida la cap CAPTURE_FRAME por space antes de ejecutar nada; acá
// sólo se encola. `name` es un nombre de archivo, no una ruta: el host decide
// el directorio.

#[op2(fast)]
fn op_capture_frame(state: &mut OpState, #[string] name: &str) -> i32 {
    let queue = state.borrow::<CaptureQueue>();
    let mut next_id = queue.next_request_id.borrow_mut();
    let request_id = *next_id;
    *next_id += 1;
    queue
        .requests
        .borrow_mut()
        .push((request_id, name.to_string()));
    request_id
}

#[op2]
#[serde]
fn op_capture_poll(state: &mut OpState, #[smi] request_id: i32) -> serde_json::Value {
    let results = state.borrow::<CaptureResults>();
    let mut map = results.results.borrow_mut();
    if let Some(result) = map.remove(&request_id) {
        match result {
            Ok(path) => serde_json::json!({"status": "ok", "path": path}),
            Err(err) => serde_json::json!({"status": "error", "error": err}),
        }
    } else {
        serde_json::json!({"status": "pending"})
    }
}

// --- Navigate op ---

#[op2(fast)]
fn op_navigate(state: &mut OpState, #[string] url: &str) {
    let queue = state.borrow::<NavigateQueue>();
    queue.queue.borrow_mut().push(url.to_string());
}

// --- Tabs ops (chrome.tabs-like, gated por cap manage_tabs en main thread) ---

#[op2(fast)]
fn op_tab_open(state: &mut OpState, #[string] url: &str, #[string] kind: &str) {
    let queue = state.borrow::<TabActionQueue>();
    queue
        .queue
        .borrow_mut()
        .push(TabAction::Open {
            url: url.to_string(),
            kind: kind.to_string(),
        });
}

#[op2(fast)]
fn op_tab_close(state: &mut OpState, #[bigint] tab_id: u64) {
    let queue = state.borrow::<TabActionQueue>();
    queue
        .queue
        .borrow_mut()
        .push(TabAction::Close { tab_id });
}

#[op2(fast)]
fn op_tab_set_visible(state: &mut OpState, #[bigint] tab_id: u64, visible: bool) {
    let queue = state.borrow::<TabActionQueue>();
    queue
        .queue
        .borrow_mut()
        .push(TabAction::SetVisible { tab_id, visible });
}

#[op2(fast)]
#[allow(clippy::too_many_arguments)]
fn op_tab_set_pose(
    state: &mut OpState,
    #[bigint] tab_id: u64,
    px: f32, py: f32, pz: f32,
    rx: f32, ry: f32, rz: f32,
) {
    let queue = state.borrow::<TabActionQueue>();
    queue
        .queue
        .borrow_mut()
        .push(TabAction::SetPose { tab_id, px, py, pz, rx, ry, rz });
}

// --- Shell ↔ app message bus (cap UX_EMBED para apps; el shell siempre lo tiene) ---

#[op2(fast)]
fn op_shell_send_message(state: &mut OpState, #[bigint] target_tab_id: u64, #[string] payload: &str) {
    let q = state.borrow::<ShellMessageOutbox>();
    q.queue.borrow_mut().push(ShellMessage {
        target_tab_id,
        payload: payload.to_string(),
    });
}

#[op2]
#[serde]
fn op_shell_poll_messages(state: &mut OpState) -> serde_json::Value {
    let inbox = state.borrow::<ShellMessageInbox>();
    let msgs = take_vec(&inbox.queue);
    if msgs.is_empty() {
        return serde_json::json!([]);
    }
    let arr: Vec<serde_json::Value> = msgs
        .into_iter()
        .map(|m| serde_json::json!({
            "fromTabId": m.target_tab_id,  // host completa al ruteo: "de quién vino"
            "payload": m.payload,
        }))
        .collect();
    serde_json::Value::Array(arr)
}

// --- Viewer pose op (read-only, cap READ_HMD_POSE) ---

#[op2]
#[serde]
fn op_read_viewer_pose(state: &mut OpState) -> serde_json::Value {
    let snap = state.borrow::<ViewerPoseState>();
    let borrow = snap.data.borrow();
    match borrow.as_ref() {
        None => serde_json::Value::Null,
        Some(d) => serde_json::json!({
            "mode": d.mode,
            "px": d.px, "py": d.py, "pz": d.pz,
            "forwardX": d.forward_x, "forwardY": d.forward_y, "forwardZ": d.forward_z,
            "yaw": d.yaw,
            "pitch": d.pitch,
            "qx": d.qx, "qy": d.qy, "qz": d.qz, "qw": d.qw,
            "aspect": d.aspect,
            "fovY": d.fov_y_rad,
        }),
    }
}

// --- WebSocket ops ---

#[op2(fast)]
fn op_ws_connect(state: &mut OpState, #[string] url: &str) -> i32 {
    let queue = state.borrow::<WsConnectQueue>();
    let mut next_id = queue.next_id.borrow_mut();
    let conn_id = *next_id;
    *next_id += 1;

    let status_map = state.borrow::<WsStatusMap>();
    status_map
        .status
        .borrow_mut()
        .insert(conn_id, "connecting".to_string());

    queue
        .requests
        .borrow_mut()
        .push((conn_id, url.to_string()));

    conn_id
}

#[op2(fast)]
fn op_ws_send(state: &mut OpState, #[smi] conn_id: i32, #[string] message: &str) {
    let queue = state.borrow::<WsSendQueue>();
    queue
        .queue
        .borrow_mut()
        .push((conn_id, message.to_string()));
}

#[op2]
#[serde]
fn op_ws_recv(state: &mut OpState, #[smi] conn_id: i32) -> serde_json::Value {
    let inbox = state.borrow::<WsInbox>();
    let mut messages = inbox.messages.borrow_mut();
    if let Some(msgs) = messages.get_mut(&conn_id) {
        if !msgs.is_empty() {
            let msg = msgs.remove(0);
            return serde_json::json!({ "status": "ok", "data": msg });
        }
    }
    serde_json::json!({ "status": "empty" })
}

#[op2]
#[string]
fn op_ws_get_status(state: &mut OpState, #[smi] conn_id: i32) -> String {
    let status_map = state.borrow::<WsStatusMap>();
    status_map
        .status
        .borrow()
        .get(&conn_id)
        .cloned()
        .unwrap_or_else(|| "closed".to_string())
}

#[op2(fast)]
fn op_ws_close(state: &mut OpState, #[smi] conn_id: i32) {
    let queue = state.borrow::<WsCloseQueue>();
    queue.queue.borrow_mut().push(conn_id);

    let status_map = state.borrow::<WsStatusMap>();
    status_map
        .status
        .borrow_mut()
        .insert(conn_id, "closed".to_string());
}

// --- DOM events + touch raw ops ---

#[op2]
#[serde]
fn op_poll_dom_events(state: &mut OpState) -> serde_json::Value {
    let queue = state.borrow::<DomEventQueue>();
    let events = take_vec(&queue.events);
    if events.is_empty() {
        return serde_json::json!([]);
    }

    let result: Vec<serde_json::Value> = events
        .into_iter()
        .map(|evt| {
            serde_json::json!({
                "type": evt.event_type,
                "nodeId": evt.node_id,
                "x": evt.x,
                "y": evt.y,
                "z": evt.z,
                "hand": evt.hand,
                "px": evt.px,
                "py": evt.py,
                "pz": evt.pz,
                "dx": evt.dx,
                "dy": evt.dy,
                "dz": evt.dz,
                "localX": evt.local.map(|p|p[0]),
                "localY": evt.local.map(|p|p[1]),
                "localZ": evt.local.map(|p|p[2]),
                "ldx": evt.local_d.map(|p|p[0]),
                "ldy": evt.local_d.map(|p|p[1]),
                "ldz": evt.local_d.map(|p|p[2]),
                "lqx": evt.local_q.map(|p|p[0]),
                "lqy": evt.local_q.map(|p|p[1]),
                "lqz": evt.local_q.map(|p|p[2]),
                "lqw": evt.local_q.map(|p|p[3]),
                "trigger": evt.trigger,
                "grip": evt.grip,
                "qx": evt.qx,
                "qy": evt.qy,
                "qz": evt.qz,
                "qw": evt.qw,
                "action": evt.action,
                "source": evt.source,
            })
        })
        .collect();

    serde_json::Value::Array(result)
}

#[op2]
#[serde]
fn op_poll_touch_events(state: &mut OpState) -> serde_json::Value {
    let queue = state.borrow::<TouchEventQueue>();
    let events = take_vec(&queue.events);
    if events.is_empty() {
        return serde_json::json!([]);
    }

    let result: Vec<serde_json::Value> = events
        .into_iter()
        .map(|(node_id, x, y, z)| {
            serde_json::json!({"nodeId": node_id, "x": x, "y": y, "z": z})
        })
        .collect();

    serde_json::Value::Array(result)
}

// ---------------------------------------------------------------------------
// V8 Platform Initialization
// ---------------------------------------------------------------------------

pub fn init_v8_platform() {
    static INIT: std::sync::Once = std::sync::Once::new();
    INIT.call_once(|| {
        eprintln!("[js_runtime] V8 platform initializing...");
    });
}

// ---------------------------------------------------------------------------
// Engine
// ---------------------------------------------------------------------------

pub struct Engine {
    rt: JsRuntime,

    // Timers
    timers: Shared<TimerQueue>,

    // RAF
    raf_pending: Shared<Vec<i32>>,
    raf_ready: Shared<Vec<(i32, f64)>>,

    // Console
    console_logs: Shared<Vec<(String, String)>>,

    // Attributes
    attr_updates: Shared<Vec<(i32, String, String)>>,
    attr_snapshot: Shared<HashMap<i32, HashMap<String, String>>>,

    // Element creation
    element_creation_queue: Shared<Vec<(i32, String)>>,
    element_creation_results: Shared<HashMap<i32, i32>>,

    // Hierarchy
    hierarchy_update_queue_append: Shared<Vec<(i32, i32)>>,
    remove_element_queue: Shared<Vec<i32>>,
    hierarchy_snapshot_parents: Shared<HashMap<i32, i32>>,
    hierarchy_snapshot_children: Shared<HashMap<i32, Vec<i32>>>,

    // Tags
    tag_snapshot: Shared<HashMap<i32, String>>,

    // Transforms
    transform_update_positions: Shared<Vec<(i32, Vec3)>>,
    transform_update_rotations: Shared<Vec<(i32, Vec3)>>,
    transform_update_scales: Shared<Vec<(i32, Vec3)>>,
    transform_update_global_positions: Shared<Vec<(i32, Vec3)>>,
    transform_snapshot_positions: Shared<HashMap<i32, Vec3>>,
    transform_snapshot_rotations: Shared<HashMap<i32, Vec3>>,
    transform_snapshot_scales: Shared<HashMap<i32, Vec3>>,
    transform_snapshot_global_positions: Shared<HashMap<i32, Vec3>>,

    // Fetch
    fetch_queue: Shared<Vec<(i32, FetchRequest)>>,
    fetch_results: Shared<HashMap<i32, std::result::Result<FetchResponse, String>>>,
    capture_queue: Shared<Vec<(i32, String)>>,
    capture_results: Shared<HashMap<i32, std::result::Result<String, String>>>,

    // Navigate
    navigate_queue: Shared<Vec<String>>,

    // Tabs (chrome.tabs-like)
    tab_action_queue: Shared<Vec<TabAction>>,

    // Viewer pose (HMD/desktop camera) — populated solo si worker tiene READ_HMD_POSE.
    viewer_pose: Shared<Option<ViewerPoseData>>,

    // Shell ↔ embedded app message bus.
    shell_outbox: Shared<Vec<ShellMessage>>,
    shell_inbox: Shared<Vec<ShellMessage>>,

    // WebSocket
    ws_connect_queue: Shared<Vec<(i32, String)>>,
    ws_inbox: Shared<HashMap<i32, Vec<String>>>,
    ws_send_queue: Shared<Vec<(i32, String)>>,
    ws_status_map: Shared<HashMap<i32, String>>,
    ws_close_queue: Shared<Vec<i32>>,

    // DOM events + Touch raw
    dom_events: Shared<Vec<DomEvent>>,
    touch_events: Shared<Vec<(i32, f32, f32, f32)>>,

    // HTTP Cache & Security
    script_cache: Shared<ScriptCache>,
    fetch_cache: Shared<FetchCache>,
    csp: Shared<ContentSecurityPolicy>,
    document_origin: Shared<String>,
}

/// Thread-safe control handle for interrupting JavaScript currently running in
/// an [`Engine`]. Keeping the V8 type private prevents callers from depending
/// on the runtime implementation details.
#[derive(Clone, Debug)]
pub struct ExecutionHandle(deno_core::v8::IsolateHandle);

impl ExecutionHandle {
    /// Interrupts JavaScript currently executing in the isolate.
    ///
    /// Returns `false` when the isolate has already been destroyed.
    pub fn terminate_execution(&self) -> bool {
        self.0.terminate_execution()
    }
}

impl Engine {
    pub fn new() -> Self {
        Self::new_with_bootstrap_cache(true)
    }

    fn new_with_bootstrap_cache(use_cache: bool) -> Self {
        let timers = Timers::default();
        let raf_state = RafState::default();
        let console = ConsoleState::default();
        let attr_updates = AttrUpdates::default();
        let attr_snapshot = AttrSnapshot::default();
        let element_creation_queue = ElementCreationQueue::default();
        let element_creation_results = ElementCreationResults::default();
        let hierarchy_update_queue = HierarchyUpdateQueue::default();
        let remove_element_queue = RemoveElementQueue::default();
        let hierarchy_snapshot = HierarchySnapshot::default();
        let tag_snapshot = TagSnapshot::default();
        let transform_update_queue = TransformUpdateQueue::default();
        let transform_snapshot = TransformSnapshot::default();
        let fetch_queue = FetchQueue::default();
        let fetch_results = FetchResults::default();
        let capture_queue = CaptureQueue::default();
        let capture_results = CaptureResults::default();
        let navigate_queue = NavigateQueue::default();
        let tab_action_queue = TabActionQueue::default();
        let viewer_pose_state = ViewerPoseState::default();
        let shell_outbox = ShellMessageOutbox::default();
        let shell_inbox = ShellMessageInbox::default();
        let ws_connect_queue = WsConnectQueue::default();
        let ws_inbox = WsInbox::default();
        let ws_send_queue = WsSendQueue::default();
        let ws_status_map = WsStatusMap::default();
        let ws_close_queue = WsCloseQueue::default();
        let dom_event_queue = DomEventQueue::default();
        let touch_event_queue = TouchEventQueue::default();

        let timers_for_state = Timers {
            queue: timers.queue.clone(),
        };
        let raf_for_state = RafState {
            pending: raf_state.pending.clone(),
            ready: raf_state.ready.clone(),
        };
        let console_for_state = ConsoleState {
            logs: console.logs.clone(),
        };
        let attr_updates_for_state = AttrUpdates {
            updates: attr_updates.updates.clone(),
        };
        let attr_snapshot_for_state = AttrSnapshot {
            data: attr_snapshot.data.clone(),
        };
        let element_creation_queue_for_state = ElementCreationQueue {
            queue: element_creation_queue.queue.clone(),
            next_request_id: element_creation_queue.next_request_id.clone(),
        };
        let element_creation_results_for_state = ElementCreationResults {
            results: element_creation_results.results.clone(),
        };
        let hierarchy_update_queue_for_state = HierarchyUpdateQueue {
            append: hierarchy_update_queue.append.clone(),
            remove_child: hierarchy_update_queue.remove_child.clone(),
        };
        let remove_element_queue_for_state = RemoveElementQueue {
            queue: remove_element_queue.queue.clone(),
        };
        let hierarchy_snapshot_for_state = HierarchySnapshot {
            parents: hierarchy_snapshot.parents.clone(),
            children: hierarchy_snapshot.children.clone(),
        };
        let tag_snapshot_for_state = TagSnapshot {
            tags: tag_snapshot.tags.clone(),
        };
        let transform_update_queue_for_state = TransformUpdateQueue {
            positions: transform_update_queue.positions.clone(),
            rotations: transform_update_queue.rotations.clone(),
            scales: transform_update_queue.scales.clone(),
            global_positions: transform_update_queue.global_positions.clone(),
        };
        let transform_snapshot_for_state = TransformSnapshot {
            positions: transform_snapshot.positions.clone(),
            rotations: transform_snapshot.rotations.clone(),
            scales: transform_snapshot.scales.clone(),
            global_positions: transform_snapshot.global_positions.clone(),
        };
        let fetch_queue_for_state = FetchQueue {
            requests: fetch_queue.requests.clone(),
            next_request_id: fetch_queue.next_request_id.clone(),
        };
        let fetch_results_for_state = FetchResults {
            results: fetch_results.results.clone(),
        };
        let capture_queue_for_state = CaptureQueue {
            requests: capture_queue.requests.clone(),
            next_request_id: capture_queue.next_request_id.clone(),
        };
        let capture_results_for_state = CaptureResults {
            results: capture_results.results.clone(),
        };
        let navigate_queue_for_state = NavigateQueue {
            queue: navigate_queue.queue.clone(),
        };
        let tab_action_queue_for_state = TabActionQueue {
            queue: tab_action_queue.queue.clone(),
        };
        let viewer_pose_state_for_state = ViewerPoseState {
            data: viewer_pose_state.data.clone(),
        };
        let shell_outbox_for_state = ShellMessageOutbox {
            queue: shell_outbox.queue.clone(),
        };
        let shell_inbox_for_state = ShellMessageInbox {
            queue: shell_inbox.queue.clone(),
        };
        let ws_connect_queue_for_state = WsConnectQueue {
            requests: ws_connect_queue.requests.clone(),
            next_id: ws_connect_queue.next_id.clone(),
        };
        let ws_inbox_for_state = WsInbox {
            messages: ws_inbox.messages.clone(),
        };
        let ws_send_queue_for_state = WsSendQueue {
            queue: ws_send_queue.queue.clone(),
        };
        let ws_status_map_for_state = WsStatusMap {
            status: ws_status_map.status.clone(),
        };
        let ws_close_queue_for_state = WsCloseQueue {
            queue: ws_close_queue.queue.clone(),
        };
        let dom_event_queue_for_state = DomEventQueue {
            events: dom_event_queue.events.clone(),
        };
        let touch_event_queue_for_state = TouchEventQueue {
            events: touch_event_queue.events.clone(),
        };

        let ext = Extension::builder("luna_runtime")
            .ops(vec![
                op_console_log::decl(),
                op_console_warn::decl(),
                op_console_error::decl(),
                op_now::decl(),
                storage::op_local_storage::decl(),
                location::op_document_location::DECL,
                location::op_url_parse::DECL,
                location::op_query_parse::DECL,
                location::op_query_encode::DECL,
                mesh::op_mesh_resource::decl(),
                pose::op_set_joint_batch::decl(),
                op_set_timeout::decl(),
                op_clear_timeout::decl(),
                op_timers_poll::decl(),
                op_raf_register::decl(),
                op_raf_poll::decl(),
                op_hsml_set_attr::decl(),
                op_hsml_get_attr::decl(),
                op_hsml_create_element::decl(),
                op_hsml_poll_created_element::decl(),
                op_hsml_append_child::decl(),
                op_hsml_remove::decl(),
                op_hsml_get_children::decl(),
                op_hsml_get_parent::decl(),
                op_hsml_get_tag::decl(),
                op_hsml_get_position::decl(),
                op_hsml_get_rotation::decl(),
                op_hsml_get_scale::decl(),
                op_hsml_get_global_position::decl(),
                op_hsml_set_position::decl(),
                op_hsml_set_rotation::decl(),
                op_hsml_set_scale::decl(),
                op_hsml_set_transform_batch::decl(),
                op_hsml_set_global_position::decl(),
                ui_text::op_ui_text::decl(),
                ui_text::op_ui_path::decl(),
                components::op_component_context::decl(),
                components::op_component_props::decl(),
                components::op_component_emit::decl(),
                components::op_component_send::decl(),
                components::op_component_poll_messages::decl(),
                components::op_component_validate_message::decl(),
                components::op_component_poll::decl(),
                components::op_component_validate::decl(),
                audio::op_audio_create::decl(),
                audio::op_audio_control::decl(),
                audio::op_audio_write::decl(),
                binary::op_encode_utf8::decl(),
                binary::op_decode_utf8::decl(),
                op_fetch_request::decl(),
                op_fetch_poll::decl(),
                op_capture_frame::decl(),
                op_luna_mcp_enabled::decl(),
                op_luna_mcp_auto_start::decl(),
                op_luna_root_settings::decl(),
                settings::op_settings_read::decl(),
                keyboard::op_keyboard_command::decl(),
                keyboard::op_keyboard_read::decl(),
                op_capture_poll::decl(),
                op_navigate::decl(),
                world_navigation::op_navigate_world::decl(),
                op_tab_open::decl(),
                op_tab_close::decl(),
                op_tab_set_visible::decl(),
                op_tab_set_pose::decl(),
                op_read_viewer_pose::decl(),
                op_shell_send_message::decl(),
                op_shell_poll_messages::decl(),
                op_ws_connect::decl(),
                op_ws_send::decl(),
                op_ws_recv::decl(),
                op_ws_get_status::decl(),
                op_ws_close::decl(),
                op_poll_dom_events::decl(),
                op_poll_touch_events::decl(),
            ])
            .state(move |state| {
                state.put(storage::StorageContext::default());
                state.put::<PerfState>(PerfState::default());
                state.put(location::DocumentLocation("about:blank".into()));
                state.put::<Timers>(Timers {
                    queue: timers_for_state.queue.clone(),
                });
                state.put::<RafState>(RafState {
                    pending: raf_for_state.pending.clone(),
                    ready: raf_for_state.ready.clone(),
                });
                state.put::<ConsoleState>(ConsoleState {
                    logs: console_for_state.logs.clone(),
                });
                state.put::<AttrUpdates>(AttrUpdates {
                    updates: attr_updates_for_state.updates.clone(),
                });
                state.put(mesh::MeshQueue::default());
                state.put(pose::PoseQueue::default());
                state.put(world_navigation::WorldNavigationQueue::default());
                state.put(audio::AudioQueue::default());
                state.put(settings::SettingsInbox::default());
                state.put(keyboard::KeyboardQueue::default());
                state.put(std::sync::Arc::new(components::ComponentPort::default()));
                state.put::<AttrSnapshot>(AttrSnapshot {
                    data: attr_snapshot_for_state.data.clone(),
                });
                state.put::<ElementCreationQueue>(ElementCreationQueue {
                    queue: element_creation_queue_for_state.queue.clone(),
                    next_request_id: element_creation_queue_for_state.next_request_id.clone(),
                });
                state.put::<ElementCreationResults>(ElementCreationResults {
                    results: element_creation_results_for_state.results.clone(),
                });
                state.put::<HierarchyUpdateQueue>(HierarchyUpdateQueue {
                    append: hierarchy_update_queue_for_state.append.clone(),
                    remove_child: hierarchy_update_queue_for_state.remove_child.clone(),
                });
                state.put::<RemoveElementQueue>(RemoveElementQueue {
                    queue: remove_element_queue_for_state.queue.clone(),
                });
                state.put::<HierarchySnapshot>(HierarchySnapshot {
                    parents: hierarchy_snapshot_for_state.parents.clone(),
                    children: hierarchy_snapshot_for_state.children.clone(),
                });
                state.put::<TagSnapshot>(TagSnapshot {
                    tags: tag_snapshot_for_state.tags.clone(),
                });
                state.put::<TransformUpdateQueue>(TransformUpdateQueue {
                    positions: transform_update_queue_for_state.positions.clone(),
                    rotations: transform_update_queue_for_state.rotations.clone(),
                    scales: transform_update_queue_for_state.scales.clone(),
                    global_positions: transform_update_queue_for_state.global_positions.clone(),
                });
                state.put::<TransformSnapshot>(TransformSnapshot {
                    positions: transform_snapshot_for_state.positions.clone(),
                    rotations: transform_snapshot_for_state.rotations.clone(),
                    scales: transform_snapshot_for_state.scales.clone(),
                    global_positions: transform_snapshot_for_state.global_positions.clone(),
                });
                state.put::<FetchQueue>(FetchQueue {
                    requests: fetch_queue_for_state.requests.clone(),
                    next_request_id: fetch_queue_for_state.next_request_id.clone(),
                });
                state.put::<FetchResults>(FetchResults {
                    results: fetch_results_for_state.results.clone(),
                });
                state.put::<CaptureQueue>(CaptureQueue {
                    requests: capture_queue_for_state.requests.clone(),
                    next_request_id: capture_queue_for_state.next_request_id.clone(),
                });
                state.put::<CaptureResults>(CaptureResults {
                    results: capture_results_for_state.results.clone(),
                });
                state.put::<NavigateQueue>(NavigateQueue {
                    queue: navigate_queue_for_state.queue.clone(),
                });
                state.put::<TabActionQueue>(TabActionQueue {
                    queue: tab_action_queue_for_state.queue.clone(),
                });
                state.put::<ViewerPoseState>(ViewerPoseState {
                    data: viewer_pose_state_for_state.data.clone(),
                });
                state.put::<ShellMessageOutbox>(ShellMessageOutbox {
                    queue: shell_outbox_for_state.queue.clone(),
                });
                state.put::<ShellMessageInbox>(ShellMessageInbox {
                    queue: shell_inbox_for_state.queue.clone(),
                });
                state.put::<WsConnectQueue>(WsConnectQueue {
                    requests: ws_connect_queue_for_state.requests.clone(),
                    next_id: ws_connect_queue_for_state.next_id.clone(),
                });
                state.put::<WsInbox>(WsInbox {
                    messages: ws_inbox_for_state.messages.clone(),
                });
                state.put::<WsSendQueue>(WsSendQueue {
                    queue: ws_send_queue_for_state.queue.clone(),
                });
                state.put::<WsStatusMap>(WsStatusMap {
                    status: ws_status_map_for_state.status.clone(),
                });
                state.put::<WsCloseQueue>(WsCloseQueue {
                    queue: ws_close_queue_for_state.queue.clone(),
                });
                state.put::<DomEventQueue>(DomEventQueue {
                    events: dom_event_queue_for_state.events.clone(),
                });
                state.put::<TouchEventQueue>(TouchEventQueue {
                    events: touch_event_queue_for_state.events.clone(),
                });
            })
            .build();

        let mut rt = JsRuntime::new(RuntimeOptions {
            extensions: vec![ext],
            ..Default::default()
        });

        for script in &BOOTSTRAP_SCRIPTS {
            let result = if use_cache {
                script.execute(&mut rt)
            } else {
                rt.execute_script(script.name, FastString::Static(script.code)).map(|_| ())
            };
            result.unwrap_or_else(|error| panic!("{} bootstrap failed: {error}", script.name));
        }

        Self {
            rt,
            timers: timers.queue,
            raf_pending: raf_state.pending,
            raf_ready: raf_state.ready,
            console_logs: console.logs,
            attr_updates: attr_updates.updates,
            attr_snapshot: attr_snapshot.data,
            element_creation_queue: element_creation_queue.queue,
            element_creation_results: element_creation_results.results,
            hierarchy_update_queue_append: hierarchy_update_queue.append,
            remove_element_queue: remove_element_queue.queue,
            hierarchy_snapshot_parents: hierarchy_snapshot.parents,
            hierarchy_snapshot_children: hierarchy_snapshot.children,
            tag_snapshot: tag_snapshot.tags,
            transform_update_positions: transform_update_queue.positions,
            transform_update_rotations: transform_update_queue.rotations,
            transform_update_scales: transform_update_queue.scales,
            transform_update_global_positions: transform_update_queue.global_positions,
            transform_snapshot_positions: transform_snapshot.positions,
            transform_snapshot_rotations: transform_snapshot.rotations,
            transform_snapshot_scales: transform_snapshot.scales,
            transform_snapshot_global_positions: transform_snapshot.global_positions,
            fetch_queue: fetch_queue.requests,
            fetch_results: fetch_results.results,
            capture_queue: capture_queue.requests,
            capture_results: capture_results.results,
            navigate_queue: navigate_queue.queue,
            tab_action_queue: tab_action_queue.queue,
            viewer_pose: viewer_pose_state.data,
            shell_outbox: shell_outbox.queue,
            shell_inbox: shell_inbox.queue,
            ws_connect_queue: ws_connect_queue.requests,
            ws_inbox: ws_inbox.messages,
            ws_send_queue: ws_send_queue.queue,
            ws_status_map: ws_status_map.status,
            ws_close_queue: ws_close_queue.queue,
            dom_events: dom_event_queue.events,
            touch_events: touch_event_queue.events,
            script_cache: shared(ScriptCache::new(100)),
            fetch_cache: shared(FetchCache::new(200)),
            csp: shared(ContentSecurityPolicy::permissive()),
            document_origin: shared(String::new()),
        }
    }

    /// Host-only document binding. Never derive this from the script URL.
    pub fn configure_document_location(&mut self, document_url: String) {
        self.rt.op_state().borrow_mut().put(location::DocumentLocation(document_url));
    }

    /// Host-only storage binding, independent from script-visible location.
    pub fn configure_local_storage(&mut self, path: std::path::PathBuf, document_url: String) {
        self.rt.op_state().borrow_mut().put(storage::StorageContext::new(path, document_url));
    }

    pub fn eval(&mut self, code: &str) -> AnyResult<()> {
        let script_fast = FastString::Owned(code.to_string().into_boxed_str());
        self.rt.execute_script("<eval>", script_fast)?;
        Ok(())
    }

    /// Returns a thread-safe handle that can interrupt a long-running script.
    pub fn execution_handle(&mut self) -> ExecutionHandle {
        ExecutionHandle(self.rt.v8_isolate().thread_safe_handle())
    }

    pub fn configure_text_backend(&mut self, backend: ui_text::TextBackend) {
        self.rt.op_state().borrow_mut().put(backend);
    }

    pub fn configure_component_port(&mut self, port: std::sync::Arc<components::ComponentPort>) {
        self.rt.op_state().borrow_mut().put(port);
    }

    pub fn fire_raf(&mut self, timestamp_ms: f64) {
        let ids = take_vec(&self.raf_pending);

        if !ids.is_empty() {
            let mut ready = self.raf_ready.borrow_mut();
            ready.reserve(ids.len());
            for id in ids {
                ready.push((id, timestamp_ms));
            }
        }

        if let Err(e) = self
            .rt
            .execute_script("<pump>", FastString::Static("__luna_component_pump(); __luna_keyboard_pump(); __luna_pump()"))
        {
            eprintln!("[js_runtime] Error calling pump: {:?}", e);
        }
    }

    pub fn needs_continuous_ticks(&self) -> bool {
        self.has_pending_animation_frames() || self.next_timer_deadline().is_some()
    }

    /// RAF needs the next frame. Timers can instead sleep until their deadline.
    pub fn has_pending_animation_frames(&self) -> bool {
        !self.raf_pending.borrow().is_empty() || !self.raf_ready.borrow().is_empty()
    }

    pub fn next_timer_deadline(&self) -> Option<Instant> {
        self.timers.borrow().next_deadline()
    }

    // --- Drain methods ---

    pub fn drain_logs(&self) -> Vec<(String, String)> {
        take_vec(&self.console_logs)
    }

    pub fn drain_attr_updates(&self) -> Vec<(i32, String, String)> {
        take_vec(&self.attr_updates)
    }
    /// Dejar en el buzón lo último que el host quiere que sepa el documento de
    /// ajustes. No despierta nada por sí solo: el isolate lo lee cuando pasa.
    pub fn publish_settings(&mut self, json: String) {
        self.rt
            .op_state()
            .borrow_mut()
            .borrow_mut::<settings::SettingsInbox>()
            .publish(json);
    }

    pub fn drain_audio_commands(&mut self) -> Vec<audio::SharedPlayback> {
        self.rt.op_state().borrow_mut().borrow_mut::<audio::AudioQueue>().drain()
    }

    pub fn drain_pose_batches(&mut self) -> Vec<pose::PoseBatch> {
        self.rt.op_state().borrow_mut().borrow_mut::<pose::PoseQueue>().drain()
    }

    pub fn drain_mesh_commands(&mut self) -> (u64, Vec<mesh::MeshCommand>) {
        self.rt.op_state().borrow_mut().borrow_mut::<mesh::MeshQueue>().drain()
    }

    pub fn drain_element_creation_queue(&self) -> Vec<(i32, String)> {
        take_vec(&self.element_creation_queue)
    }

    pub fn drain_hierarchy_append_queue(&self) -> Vec<(i32, i32)> {
        take_vec(&self.hierarchy_update_queue_append)
    }

    pub fn drain_remove_element_queue(&self) -> Vec<i32> {
        take_vec(&self.remove_element_queue)
    }

    pub fn drain_transform_position_updates(&self) -> Vec<(i32, Vec3)> {
        take_vec(&self.transform_update_positions)
    }

    pub fn drain_transform_rotation_updates(&self) -> Vec<(i32, Vec3)> {
        take_vec(&self.transform_update_rotations)
    }

    pub fn drain_transform_scale_updates(&self) -> Vec<(i32, Vec3)> {
        take_vec(&self.transform_update_scales)
    }

    pub fn drain_transform_global_position_updates(&self) -> Vec<(i32, Vec3)> {
        take_vec(&self.transform_update_global_positions)
    }

    pub fn drain_fetch_queue(&self) -> Vec<(i32, FetchRequest)> {
        take_vec(&self.fetch_queue)
    }

    pub fn drain_capture_queue(&self) -> Vec<(i32, String)> {
        take_vec(&self.capture_queue)
    }

    pub fn push_capture_result(
        &self,
        request_id: i32,
        result: std::result::Result<String, String>,
    ) {
        self.capture_results.borrow_mut().insert(request_id, result);
    }

    pub fn drain_navigate_queue(&self) -> Vec<String> {
        take_vec(&self.navigate_queue)
    }

    pub fn drain_keyboard_commands(&mut self) -> Vec<serde_json::Value> {
        std::mem::take(&mut self.rt.op_state().borrow_mut().borrow_mut::<keyboard::KeyboardQueue>().outgoing)
    }

    pub fn push_keyboard_events(&mut self, events: Vec<serde_json::Value>) {
        let state = self.rt.op_state();
        let mut state = state.borrow_mut();
        let queue = state.borrow_mut::<keyboard::KeyboardQueue>();
        // Never retain an unbounded stream while an isolate is stalled.
        if queue.incoming.len() + events.len() > 256 { queue.incoming.clear(); }
        queue.incoming.extend(events.into_iter().take(256));
    }

    pub fn drain_world_navigation(&mut self) -> Vec<String> {
        std::mem::take(&mut self.rt.op_state().borrow_mut()
            .borrow_mut::<world_navigation::WorldNavigationQueue>().0)
    }

    pub fn drain_tab_action_queue(&self) -> Vec<TabAction> {
        take_vec(&self.tab_action_queue)
    }

    /// Setea/limpia el snapshot de viewer pose. Llamar con `None` para
    /// limpiar (ej. cuando el worker pierde la cap). El op JS ve el nuevo
    /// valor inmediatamente (referencia compartida).
    pub fn set_viewer_pose(&self, data: Option<ViewerPoseData>) {
        *self.viewer_pose.borrow_mut() = data;
    }

    /// Drena los mensajes que la app JS quiso enviar al shell (o viceversa,
    /// depende quién opere el worker). El host rutea según target_tab_id.
    pub fn drain_shell_outbox(&self) -> Vec<ShellMessage> {
        take_vec(&self.shell_outbox)
    }

    /// Empuja mensajes entrantes al inbox del worker. El JS los lee con
    /// `op_shell_poll_messages()` en su tick.
    pub fn push_shell_messages(&self, msgs: Vec<ShellMessage>) {
        if msgs.is_empty() {
            return;
        }
        self.shell_inbox.borrow_mut().extend(msgs);
    }

    // --- Update snapshot methods ---

    pub fn update_attr_snapshot(&self, snapshot: HashMap<i32, HashMap<String, String>>) {
        replace_map(&self.attr_snapshot, snapshot);
    }

    pub fn patch_attr_snapshot(&self, updates: HashMap<i32, HashMap<String, String>>) {
        patch_map(&self.attr_snapshot, updates);
    }

    pub fn remove_snapshot_nodes(&self, node_ids: Vec<i32>) {
        if node_ids.is_empty() {
            return;
        }
        let node_ids: HashSet<i32> = node_ids.into_iter().collect();
        remove_map_keys(&self.attr_snapshot, &node_ids);
        remove_map_keys(&self.tag_snapshot, &node_ids);
        remove_map_keys(&self.transform_snapshot_positions, &node_ids);
        remove_map_keys(&self.transform_snapshot_rotations, &node_ids);
        remove_map_keys(&self.transform_snapshot_scales, &node_ids);
        remove_map_keys(&self.transform_snapshot_global_positions, &node_ids);
        remove_map_keys(&self.hierarchy_snapshot_parents, &node_ids);
        remove_map_keys(&self.hierarchy_snapshot_children, &node_ids);

        for children in self.hierarchy_snapshot_children.borrow_mut().values_mut() {
            children.retain(|child_id| !node_ids.contains(child_id));
        }
    }

    pub fn push_element_creation_result(&self, request_id: i32, node_id: i32) {
        self.element_creation_results
            .borrow_mut()
            .insert(request_id, node_id);
    }

    pub fn update_hierarchy_snapshot(
        &self,
        parents: HashMap<i32, i32>,
        children: HashMap<i32, Vec<i32>>,
    ) {
        replace_map(&self.hierarchy_snapshot_parents, parents);
        replace_map(&self.hierarchy_snapshot_children, children);
    }

    pub fn patch_hierarchy_snapshot(
        &self,
        parents: HashMap<i32, i32>,
        children: HashMap<i32, Vec<i32>>,
    ) {
        patch_map(&self.hierarchy_snapshot_parents, parents);
        patch_map(&self.hierarchy_snapshot_children, children);
    }

    pub fn update_tag_snapshot(&self, tags: HashMap<i32, String>) {
        replace_map(&self.tag_snapshot, tags);
    }

    pub fn patch_tag_snapshot(&self, tags: HashMap<i32, String>) {
        patch_map(&self.tag_snapshot, tags);
    }

    pub fn update_transform_snapshot(
        &self,
        positions: HashMap<i32, Vec3>,
        rotations: HashMap<i32, Vec3>,
        scales: HashMap<i32, Vec3>,
        global_positions: HashMap<i32, Vec3>,
    ) {
        replace_map(&self.transform_snapshot_positions, positions);
        replace_map(&self.transform_snapshot_rotations, rotations);
        replace_map(&self.transform_snapshot_scales, scales);
        replace_map(&self.transform_snapshot_global_positions, global_positions);
    }

    pub fn patch_transform_snapshot(
        &self,
        positions: HashMap<i32, Vec3>,
        rotations: HashMap<i32, Vec3>,
        scales: HashMap<i32, Vec3>,
        global_positions: HashMap<i32, Vec3>,
    ) {
        patch_map(&self.transform_snapshot_positions, positions);
        patch_map(&self.transform_snapshot_rotations, rotations);
        patch_map(&self.transform_snapshot_scales, scales);
        patch_map(&self.transform_snapshot_global_positions, global_positions);
    }

    pub fn push_fetch_result(
        &self,
        request_id: i32,
        result: std::result::Result<FetchResponse, String>,
    ) {
        self.fetch_results.borrow_mut().insert(request_id, result);
    }

    // --- WebSocket methods ---

    pub fn drain_ws_connect_queue(&self) -> Vec<(i32, String)> {
        take_vec(&self.ws_connect_queue)
    }

    pub fn drain_ws_send_queue(&self) -> Vec<(i32, String)> {
        take_vec(&self.ws_send_queue)
    }

    pub fn drain_ws_close_queue(&self) -> Vec<i32> {
        take_vec(&self.ws_close_queue)
    }

    pub fn push_ws_message(&self, conn_id: i32, message: String) {
        self.ws_inbox
            .borrow_mut()
            .entry(conn_id)
            .or_default()
            .push(message);
    }

    pub fn set_ws_status(&self, conn_id: i32, status: String) {
        self.ws_status_map.borrow_mut().insert(conn_id, status);
    }

    pub fn remove_ws_connection(&self, conn_id: i32) {
        self.ws_inbox.borrow_mut().remove(&conn_id);
        self.ws_status_map.borrow_mut().remove(&conn_id);
    }

    // --- DOM events ---

    pub fn push_dom_event(
        &self,
        event_type: impl Into<String>,
        node_id: i32,
        x: Option<f32>,
        y: Option<f32>,
        z: Option<f32>,
    ) {
        self.dom_events.borrow_mut().push(DomEvent {
            local: None,
            local_d: None,
            local_q: None,
            event_type: event_type.into(),
            node_id,
            x,
            y,
            z,
            hand: None,
            px: None,
            py: None,
            pz: None,
            dx: None,
            dy: None,
            dz: None,
            trigger: None,
            grip: None,
            qx: None,
            qy: None,
            qz: None,
            qw: None,
            action: None,
            source: None,
        });
    }

    pub fn push_dom_toque_event(&self, node_id: i32, x: f32, y: f32, z: f32) {
        self.push_dom_event("toque", node_id, Some(x), Some(y), Some(z));
    }

    pub fn push_local_toque_event(&self,node_id:i32,x:f32,y:f32,z:f32,local:[f32;3]) {
        self.push_dom_toque_event(node_id,x,y,z);
        if let Some(event)=self.dom_events.borrow_mut().last_mut() {
            if local.iter().all(|v|v.is_finite()) {event.local=Some(local);}
        }
    }

    /// Completa el último posemove con la pose en el marco del padre del
    /// posezone: `localX..Z`, `ldx..ldz` y `lqx..lqw` del lado de JS.
    pub fn set_last_posemove_local(&self, local: [f32; 3], dir: [f32; 3], rot: [f32; 4]) {
        if let Some(event) = self.dom_events.borrow_mut().last_mut() {
            if event.event_type != "posemove" {
                return;
            }
            if local.iter().all(|v| v.is_finite()) { event.local = Some(local); }
            if dir.iter().all(|v| v.is_finite()) { event.local_d = Some(dir); }
            if rot.iter().all(|v| v.is_finite()) { event.local_q = Some(rot); }
        }
    }

    pub fn push_posemove_event(
        &self,
        node_id: i32,
        hand: impl Into<String>,
        px: f32,
        py: f32,
        pz: f32,
        dx: f32,
        dy: f32,
        dz: f32,
        trigger: f32,
        grip: f32,
        qx: f32,
        qy: f32,
        qz: f32,
        qw: f32,
    ) {
        self.dom_events.borrow_mut().push(DomEvent {
            local: None,
            local_d: None,
            local_q: None,
            event_type: "posemove".to_string(),
            node_id,
            x: None,
            y: None,
            z: None,
            hand: Some(hand.into()),
            px: Some(px),
            py: Some(py),
            pz: Some(pz),
            dx: Some(dx),
            dy: Some(dy),
            dz: Some(dz),
            trigger: Some(trigger),
            grip: Some(grip),
            qx: Some(qx),
            qy: Some(qy),
            qz: Some(qz),
            qw: Some(qw),
            action: None,
            source: None,
        });
    }

    /// systeminput: evento system-level normalizado, dispatcheado en el root (nodeId=0).
    /// `action` es la acción semántica ("shell", futuro: "back", "capture"...);
    /// `source` el origen hardware-agnóstico ("vr_menu", "kb_escape", ...).
    pub fn push_system_input_event(
        &self,
        action: impl Into<String>,
        source: impl Into<String>,
    ) {
        self.dom_events.borrow_mut().push(DomEvent {
            local: None,
            local_d: None,
            local_q: None,
            event_type: "systeminput".to_string(),
            node_id: 0,
            x: None,
            y: None,
            z: None,
            hand: None,
            px: None,
            py: None,
            pz: None,
            dx: None,
            dy: None,
            dz: None,
            trigger: None,
            grip: None,
            qx: None,
            qy: None,
            qz: None,
            qw: None,
            action: Some(action.into()),
            source: Some(source.into()),
        });
    }

    pub fn push_touch_event(&self, node_id: i32, x: f32, y: f32, z: f32) {
        self.touch_events.borrow_mut().push((node_id, x, y, z));
    }

    // -------------------------------------------------------------------------
    // HTTP Cache & Security Methods
    // -------------------------------------------------------------------------

    pub fn set_document_origin(&self, origin: String) {
        *self.document_origin.borrow_mut() = origin;
    }

    pub fn get_document_origin(&self) -> String {
        self.document_origin.borrow().clone()
    }

    pub fn set_csp(&self, csp: ContentSecurityPolicy) {
        *self.csp.borrow_mut() = csp;
    }

    pub fn get_csp(&self) -> ContentSecurityPolicy {
        self.csp.borrow().clone()
    }

    pub fn csp_allows_script(&self, url: &str) -> bool {
        let csp = self.csp.borrow();
        let origin = self.document_origin.borrow();
        csp.allows_script(url, &origin)
    }

    pub fn csp_allows_fetch(&self, url: &str) -> bool {
        let csp = self.csp.borrow();
        let origin = self.document_origin.borrow();
        csp.allows_connect(url, &origin)
    }

    pub fn get_cached_script(&self, url: &str) -> Option<CachedScript> {
        self.script_cache.borrow_mut().get(url)
    }

    pub fn get_script_for_revalidation(&self, url: &str) -> Option<CachedScript> {
        self.script_cache.borrow_mut().get_for_revalidation(url)
    }

    pub fn cache_script(&self, url: String, script: CachedScript) {
        self.script_cache.borrow_mut().insert(url, script);
    }

    pub fn get_cache_stats(&self) -> CacheStats {
        self.script_cache.borrow().stats()
    }

    pub fn clear_caches(&self) {
        self.script_cache.borrow_mut().clear();
        self.fetch_cache.borrow_mut().clear();
    }

    pub fn get_cached_response(&self, url: &str) -> Option<CachedResponse> {
        self.fetch_cache.borrow_mut().get(url)
    }

    pub fn cache_response(&self, url: String, response: CachedResponse) {
        self.fetch_cache.borrow_mut().insert(url, response);
    }
}

// ---------------------------------------------------------------------------
// Bootstrap JS
// ---------------------------------------------------------------------------

const BOOTSTRAP_JS: &str = r#"
(function (global) {
  global.window = global;
  const core = Deno.core;

  global.console = {
    log: (...a) => core.ops.op_console_log(a.map(x => String(x)).join(" ")),
    warn: (...a) => core.ops.op_console_warn(a.map(x => String(x)).join(" ")),
    error: (...a) => core.ops.op_console_error(a.map(x => String(x)).join(" ")),
  };

  global.performance = { now: () => core.ops.op_now() };

  const callbacks = new Map();
  let nextId = 1;

  global.setTimeout = (cb, ms = 0, ...args) => {
    const id = nextId++;
    callbacks.set(id, () => cb(...args));
    core.ops.op_set_timeout(id|0, ms|0);
    return id;
  };
  global.clearTimeout = (id) => {
    core.ops.op_clear_timeout(id|0);
    callbacks.delete(id|0);
  };

  // setInterval sobre setTimeout: cada disparo agenda el siguiente. Faltaba, y
  // un script que lo usaba moria entero con ReferenceError apenas llegaba a esa
  // linea. El id es estable aunque cada vuelta use un timeout nuevo; `vivos`
  // guarda cual es el pendiente para que clearInterval lo encuentre.
  const vivos = new Map();
  global.setInterval = (cb, ms = 0, ...args) => {
    const id = nextId++;
    const espera = Math.max(1, ms|0);
    const vuelta = () => {
      if (!vivos.has(id)) return;
      vivos.set(id, global.setTimeout(vuelta, espera));
      cb(...args);
    };
    vivos.set(id, global.setTimeout(vuelta, espera));
    return id;
  };
  global.clearInterval = (id) => {
    const t = vivos.get(id|0);
    vivos.delete(id|0);
    if (t !== undefined) global.clearTimeout(t);
  };

  const rafCallbacks = new Map();
  global.requestAnimationFrame = (cb) => {
    const id = nextId++;
    rafCallbacks.set(id, cb);
    core.ops.op_raf_register(id|0);
    return id;
  };
  global.cancelAnimationFrame = (id) => {
    rafCallbacks.delete(id|0);
  };

  function pump() {
    const tids = core.ops.op_timers_poll();
    for (const id of tids) {
      const fn_ = callbacks.get(id);
      if (fn_) {
        callbacks.delete(id);
        try { fn_(); } catch (e) { console.error(e); }
      }
    }

    if (typeof global.__luna_set_hover_targets === 'function' && global.__luna_pending_hover_targets) {
      try { global.__luna_set_hover_targets(global.__luna_pending_hover_targets); } catch (e) { console.error(e); }
    }
    if (typeof global.__luna_dispatch_dom_events === 'function') {
      const domEvents = core.ops.op_poll_dom_events();
      if (domEvents && domEvents.length > 0) {
        try { global.__luna_dispatch_dom_events(domEvents); } catch (e) { console.error(e); }
      }
    }

    const rafs = core.ops.op_raf_poll();
    for (const pair of rafs) {
      const id = pair[0], ts = pair[1];
      const fn_ = rafCallbacks.get(id);
      if (fn_) {
        rafCallbacks.delete(id);
        try { fn_(ts); } catch (e) { console.error(e); }
      }
    }
  }

  global.__luna_pump = pump;

})(globalThis);
"#;

// ---------------------------------------------------------------------------
// Runtime JS
// ---------------------------------------------------------------------------

const RUNTIME_JS: &str = include_str!("../runtime.js");

/// Only compilation bytes are shared. Each isolate still executes these scripts
/// against its own globals and native op state, including independent callbacks.
struct CachedBootstrap {
    name: &'static str,
    code: &'static str,
    cache: OnceLock<Box<[u8]>>,
}

impl CachedBootstrap {
    const fn new(name: &'static str, code: &'static str) -> Self {
        Self { name, code, cache: OnceLock::new() }
    }

    fn execute(&self, runtime: &mut JsRuntime) -> AnyResult<()> {
        use deno_core::v8::{self, script_compiler};
        let scope = &mut runtime.handle_scope();
        let source = if self.code.is_ascii() {
            v8::String::new_external_onebyte_static(scope, self.code.as_bytes())
        } else {
            v8::String::new(scope, self.code)
        }.ok_or_else(|| anyhow::anyhow!("could not allocate bootstrap source"))?;
        let name = v8::String::new_external_onebyte_static(scope, self.name.as_bytes()).unwrap();
        let source_map_url = v8::String::empty(scope);
        // Match JsRuntime::execute_script's origin, preserving stack traces.
        let origin = v8::ScriptOrigin::new(scope, name.into(), 0, 0, false, 123,
            source_map_url.into(), true, false, false);
        let cached = self.cache.get();
        let (source, options) = match cached {
            Some(bytes) => (script_compiler::Source::new_with_cached_data(source,
                Some(&origin), v8::CachedData::new(bytes)),
                script_compiler::CompileOptions::ConsumeCodeCache),
            None => (script_compiler::Source::new(source, Some(&origin)),
                script_compiler::CompileOptions::NoCompileOptions),
        };
        let scope = &mut v8::TryCatch::new(scope);
        let result = script_compiler::compile(scope, source, options,
            script_compiler::NoCacheReason::NoReason);
        let script = match result {
            Some(script) => script,
            None => return Err(bootstrap_exception(scope)),
        };
        if script.run(scope).is_none() {
            return Err(bootstrap_exception(scope));
        }
        if cached.is_none() {
            // Capture after execution to include bootstrap functions that ran,
            // while preserving V8's lazy compilation for unused API methods.
            if let Some(bytes) = script.get_unbound_script(scope).create_code_cache() {
                let _ = self.cache.set(bytes.to_vec().into_boxed_slice());
            }
        }
        Ok(())
    }
}

fn bootstrap_exception(scope: &mut deno_core::v8::TryCatch<deno_core::v8::HandleScope>) -> anyhow::Error {
    match scope.exception() {
        Some(exception) => deno_core::error::JsError::from_v8_exception(scope, exception).into(),
        None => anyhow::anyhow!("bootstrap execution terminated"),
    }
}

static BOOTSTRAP_SCRIPTS: [CachedBootstrap; 12] = [
    CachedBootstrap::new("<bootstrap>", BOOTSTRAP_JS),
    CachedBootstrap::new("<storage>", include_str!("../storage.js")),
    CachedBootstrap::new("<location>", include_str!("../location.js")),
    CachedBootstrap::new("<runtime>", RUNTIME_JS),
    CachedBootstrap::new("<keyboard>", include_str!("../keyboard.js")),
    CachedBootstrap::new("<mesh>", include_str!("../mesh.js")),
    CachedBootstrap::new("<binary>", include_str!("../binary.js")),
    CachedBootstrap::new("<fetch>", include_str!("../fetch.js")),
    CachedBootstrap::new("<audio>", include_str!("../audio.js")),
    CachedBootstrap::new("<ui-path>", "globalThis.PathGeometry=Object.freeze({tessellate(commands,{tolerance=0.0001,strokeWidth=0,nonZero=false}={}){return Deno.core.ops.op_ui_path({commands,tolerance,strokeWidth,nonZero});}});"),
    CachedBootstrap::new("<ui-text>", "globalThis.TextLayout = Object.freeze({create(text,size,width=0){return Deno.core.ops.op_ui_text(String(text),Number(size),Number(width));}});"),
    CachedBootstrap::new("<components>", include_str!("../components.js")),
];

#[cfg(test)]
mod bootstrap_cache_tests {
    use super::*;

    #[test]
    fn cached_bootstrap_preserves_isolate_globals_callbacks_and_native_state() {
        let mut first = Engine::new();
        first.eval(r#"
            globalThis.onlyInFirst = 123;
            HSMLElement.prototype.onlyInFirst = true;
            setTimeout(() => console.log('FIRST_TIMER'), 0);
            console.log('FIRST_LOG');
        "#).unwrap();
        let mut second = Engine::new();
        second.eval(r#"
            if (typeof onlyInFirst !== 'undefined' || HSMLElement.prototype.onlyInFirst)
                throw new Error('bootstrap cache shared JavaScript state');
            console.log('SECOND_LOG');
        "#).unwrap();
        assert!(second.next_timer_deadline().is_none());
        second.fire_raf(16.0);
        let second_logs = second.drain_logs();
        assert!(second_logs.iter().any(|(_, message)| message == "SECOND_LOG"));
        assert!(!second_logs.iter().any(|(_, message)| message.starts_with("FIRST_")));
        // V8 isolates are entered in stack order on this thread.
        drop(second);
        first.fire_raf(16.0);
        assert!(first.drain_logs().iter().any(|(_, message)| message == "FIRST_TIMER"));
        assert!(BOOTSTRAP_SCRIPTS.iter().all(|script| script.cache.get().is_some()));
    }

    #[test]
    fn bootstrap_cache_preserves_compile_and_runtime_errors() {
        let mut runtime = JsRuntime::new(RuntimeOptions::default());
        let invalid = CachedBootstrap::new("<invalid-bootstrap>", "function (");
        assert!(invalid.execute(&mut runtime).unwrap_err().to_string().contains("SyntaxError"));
        assert!(invalid.cache.get().is_none());
        let throws = CachedBootstrap::new("<throwing-bootstrap>", "throw new Error('EXPECTED_BOOTSTRAP_ERROR');");
        assert!(throws.execute(&mut runtime).unwrap_err().to_string().contains("EXPECTED_BOOTSTRAP_ERROR"));
        assert!(throws.cache.get().is_none());
    }

    #[test]
    fn bootstrap_cache_rejection_falls_back_to_compiling_source() {
        let script = CachedBootstrap::new("<rejected-cache>", "globalThis.cacheFallback = 42;");
        script.cache.set(vec![0; 128].into_boxed_slice()).unwrap();
        let mut runtime = JsRuntime::new(RuntimeOptions::default());
        script.execute(&mut runtime).unwrap();
        runtime.execute_script("<verify>", FastString::Static(
            "if (cacheFallback !== 42) throw new Error('cache fallback did not execute');"
        )).unwrap();
    }

    #[test]
    #[ignore = "manual cold-isolate bootstrap compilation benchmark"]
    fn benchmark_bootstrap_compilation_cache() {
        drop(Engine::new());
        let mut uncached_samples = Vec::new();
        let mut cached_samples = Vec::new();
        let mut uncached_heap = 0;
        let mut cached_heap = 0;
        for sample in 0..24 {
            for use_cache in if sample % 2 == 0 { [false, true] } else { [true, false] } {
                let started = Instant::now();
                let mut engine = Engine::new_with_bootstrap_cache(use_cache);
                let elapsed = started.elapsed().as_secs_f64() * 1000.0;
                let mut heap = deno_core::v8::HeapStatistics::default();
                engine.rt.v8_isolate().get_heap_statistics(&mut heap);
                if use_cache {
                    cached_samples.push(elapsed);
                    cached_heap += heap.used_heap_size();
                } else {
                    uncached_samples.push(elapsed);
                    uncached_heap += heap.used_heap_size();
                }
            }
        }
        uncached_samples.sort_by(f64::total_cmp);
        cached_samples.sort_by(f64::total_cmp);
        let cache_bytes: usize = BOOTSTRAP_SCRIPTS.iter()
            .filter_map(|script| script.cache.get()).map(|bytes| bytes.len()).sum();
        eprintln!("bootstrap_cache uncached_median_ms={:.3} cached_median_ms={:.3} uncached_heap_bytes={} cached_heap_bytes={} shared_cache_bytes={cache_bytes}",
            uncached_samples[12], cached_samples[12], uncached_heap / 24, cached_heap / 24);
    }
}

// ---------------------------------------------------------------------------
// Standalone run_js
// ---------------------------------------------------------------------------

use deno_core::{error::AnyError, serde_v8};
use serde_json::Value;

pub fn run_js(script: &str) -> Result<Value, AnyError> {
    let mut runtime = JsRuntime::new(RuntimeOptions::default());
    let script_fast = FastString::Owned(script.to_string().into_boxed_str());
    let result = runtime.execute_script("<init>", script_fast)?;
    let scope = &mut runtime.handle_scope();
    let local = deno_core::v8::Local::new(scope, result);
    let value: Value = serde_v8::from_v8(scope, local)?;
    Ok(value)
}
