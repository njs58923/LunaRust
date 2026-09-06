use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    rc::Rc,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use anyhow::Result as AnyResult;
use deno_core::{op2, Extension, FastString, JsRuntime, OpState, RuntimeOptions};

// Public modules
pub mod cache;
pub mod csp;
pub mod storage;

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

// Timers sí son cross-thread: setTimeout usa thread::spawn.
struct Timers {
    ready: Arc<Mutex<Vec<i32>>>,
    cancelled: Arc<Mutex<HashSet<i32>>>,
    scheduled: Arc<Mutex<HashSet<i32>>>,
}
impl Default for Timers {
    fn default() -> Self {
        Self {
            ready: Arc::new(Mutex::new(Vec::new())),
            cancelled: Arc::new(Mutex::new(HashSet::new())),
            scheduled: Arc::new(Mutex::new(HashSet::new())),
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
    pub requests: Shared<Vec<(i32, String)>>,
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
    pub results: Shared<HashMap<i32, std::result::Result<String, String>>>,
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
    let ready = timers.ready.clone();
    let cancelled = timers.cancelled.clone();
    let scheduled = timers.scheduled.clone();

    scheduled.lock().unwrap().insert(id);

    if ms <= 0 {
        if !cancelled.lock().unwrap().contains(&id) {
            ready.lock().unwrap().push(id);
        }
        return;
    }

    thread::spawn(move || {
        thread::sleep(Duration::from_millis(ms as u64));
        if cancelled.lock().unwrap().contains(&id) {
            return;
        }
        ready.lock().unwrap().push(id);
    });
}

#[op2(fast)]
fn op_clear_timeout(state: &mut OpState, #[smi] id: i32) {
    let timers = state.borrow::<Timers>();
    timers.cancelled.lock().unwrap().insert(id);
    timers.scheduled.lock().unwrap().remove(&id);
}

#[op2]
#[serde]
fn op_timers_poll(state: &mut OpState) -> serde_json::Value {
    let timers = state.borrow::<Timers>();
    let mut ready = timers.ready.lock().unwrap();
    let ids: Vec<i32> = ready.drain(..).collect();
    if !ids.is_empty() {
        let mut scheduled = timers.scheduled.lock().unwrap();
        for id in &ids {
            scheduled.remove(id);
        }
    }
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
) {
    let updates = state.borrow::<AttrUpdates>();
    updates
        .updates
        .borrow_mut()
        .push((node_id, key.to_string(), value.to_string()));
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

#[op2(fast)]
fn op_fetch_request(state: &mut OpState, #[string] url: &str) -> i32 {
    let queue = state.borrow::<FetchQueue>();
    let mut next_id = queue.next_request_id.borrow_mut();
    let request_id = *next_id;
    *next_id += 1;
    queue
        .requests
        .borrow_mut()
        .push((request_id, url.to_string()));
    request_id
}

#[op2]
#[serde]
fn op_fetch_poll(state: &mut OpState, #[smi] request_id: i32) -> serde_json::Value {
    let results = state.borrow::<FetchResults>();
    let mut map = results.results.borrow_mut();
    if let Some(result) = map.remove(&request_id) {
        match result {
            Ok(text) => serde_json::json!({"status": "ok", "text": text}),
            Err(err) => serde_json::json!({"status": "error", "error": err}),
        }
    } else {
        serde_json::json!({"status": "pending"})
    }
}

// --- Capture ops ---

#[op2(fast)]
fn op_luna_mcp_enabled(state: &mut OpState, enabled: bool) {
    state.borrow::<TabActionQueue>().queue.borrow_mut()
        .push(TabAction::SetMcpEnabled { enabled });
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
    timers_scheduled: Arc<Mutex<HashSet<i32>>>,

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
    fetch_queue: Shared<Vec<(i32, String)>>,
    fetch_results: Shared<HashMap<i32, std::result::Result<String, String>>>,
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
            ready: timers.ready.clone(),
            cancelled: timers.cancelled.clone(),
            scheduled: timers.scheduled.clone(),
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
                op_fetch_request::decl(),
                op_fetch_poll::decl(),
                op_capture_frame::decl(),
                op_luna_mcp_enabled::decl(),
                op_capture_poll::decl(),
                op_navigate::decl(),
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
                state.put::<Timers>(Timers {
                    ready: timers_for_state.ready.clone(),
                    cancelled: timers_for_state.cancelled.clone(),
                    scheduled: timers_for_state.scheduled.clone(),
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

        eprintln!("[js_runtime] Creating JsRuntime...");
        let mut rt = JsRuntime::new(RuntimeOptions {
            extensions: vec![ext],
            ..Default::default()
        });

        eprintln!("[js_runtime] Injecting bootstrap...");
        rt.execute_script("<bootstrap>", FastString::Static(BOOTSTRAP_JS))
            .expect("bootstrap failed");

        rt.execute_script("<storage>", FastString::Static(include_str!("../storage.js")))
            .expect("storage bootstrap failed");

        eprintln!("[js_runtime] Injecting runtime.js...");
        rt.execute_script("<runtime>", FastString::Static(RUNTIME_JS))
            .expect("runtime.js failed");

        eprintln!("[js_runtime] Engine created successfully");

        Self {
            rt,
            timers_scheduled: timers.scheduled,
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
            .execute_script("<pump>", FastString::Static("__luna_pump()"))
        {
            eprintln!("[js_runtime] Error calling pump: {:?}", e);
        }
    }

    pub fn needs_continuous_ticks(&self) -> bool {
        if !self.raf_pending.borrow().is_empty() || !self.raf_ready.borrow().is_empty() {
            return true;
        }
        !self.timers_scheduled.lock().unwrap().is_empty()
    }

    // --- Drain methods ---

    pub fn drain_logs(&self) -> Vec<(String, String)> {
        take_vec(&self.console_logs)
    }

    pub fn drain_attr_updates(&self) -> Vec<(i32, String, String)> {
        take_vec(&self.attr_updates)
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

    pub fn drain_fetch_queue(&self) -> Vec<(i32, String)> {
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
        result: std::result::Result<String, String>,
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
