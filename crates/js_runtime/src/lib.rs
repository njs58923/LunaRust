use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use anyhow::Result;
use deno_core::{op2, Extension, FastString, JsRuntime, OpState, RuntimeOptions};

// Public modules
pub mod cache;
pub mod csp;

pub use cache::{ScriptCache, CachedScript, FetchCache, CachedResponse, CacheStats};
pub use csp::{ContentSecurityPolicy, CorsValidator, CorsValidation};

// ---------------------------------------------------------------------------
// State structs for OpState
// ---------------------------------------------------------------------------

struct Timers {
    ready: Arc<Mutex<Vec<i32>>>,
    cancelled: Arc<Mutex<HashSet<i32>>>,
}
impl Default for Timers {
    fn default() -> Self {
        Self {
            ready: Arc::new(Mutex::new(Vec::new())),
            cancelled: Arc::new(Mutex::new(HashSet::new())),
        }
    }
}

struct RafState {
    pending: Arc<Mutex<Vec<i32>>>,
    ready: Arc<Mutex<Vec<(i32, f64)>>>,
}
impl Default for RafState {
    fn default() -> Self {
        Self {
            pending: Arc::new(Mutex::new(Vec::new())),
            ready: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

struct PerfState {
    start: Instant,
}
impl Default for PerfState {
    fn default() -> Self {
        Self { start: Instant::now() }
    }
}

/// Captured console output: Vec<(level, message)>
pub struct ConsoleState {
    pub logs: Arc<Mutex<Vec<(String, String)>>>,
}
impl Default for ConsoleState {
    fn default() -> Self {
        Self {
            logs: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

/// DOM attribute mutations from JS: Vec<(node_id, key, value)>
pub struct AttrUpdates {
    pub updates: Arc<Mutex<Vec<(i32, String, String)>>>,
}
impl Default for AttrUpdates {
    fn default() -> Self {
        Self {
            updates: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

/// Read-only snapshot of attributes for JS to query
pub struct AttrSnapshot {
    pub data: Arc<Mutex<HashMap<i32, HashMap<String, String>>>>,
}
impl Default for AttrSnapshot {
    fn default() -> Self {
        Self {
            data: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

// --- NEW: DOM mutation queues ---

/// Queue of createElement(tag) commands: Vec<(request_id, tag_name)>
/// Returns node_id via ElementCreationResults
pub struct ElementCreationQueue {
    pub queue: Arc<Mutex<Vec<(i32, String)>>>,
    next_request_id: Arc<Mutex<i32>>,
}
impl Default for ElementCreationQueue {
    fn default() -> Self {
        Self {
            queue: Arc::new(Mutex::new(Vec::new())),
            next_request_id: Arc::new(Mutex::new(1)),
        }
    }
}

/// Results of createElement: HashMap<request_id, node_id>
pub struct ElementCreationResults {
    pub results: Arc<Mutex<HashMap<i32, i32>>>,
}
impl Default for ElementCreationResults {
    fn default() -> Self {
        Self {
            results: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

/// Queue of appendChild/removeChild commands: Vec<(parent_id, child_id)>
pub struct HierarchyUpdateQueue {
    pub append: Arc<Mutex<Vec<(i32, i32)>>>,
    pub remove_child: Arc<Mutex<Vec<(i32, i32)>>>,
}
impl Default for HierarchyUpdateQueue {
    fn default() -> Self {
        Self {
            append: Arc::new(Mutex::new(Vec::new())),
            remove_child: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

/// Queue of remove(node_id) commands: Vec<node_id>
pub struct RemoveElementQueue {
    pub queue: Arc<Mutex<Vec<i32>>>,
}
impl Default for RemoveElementQueue {
    fn default() -> Self {
        Self {
            queue: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

// --- NEW: Transform snapshots (read-only for JS) ---

#[derive(Clone, Debug)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

pub struct TransformSnapshot {
    pub positions: Arc<Mutex<HashMap<i32, Vec3>>>,
    pub rotations: Arc<Mutex<HashMap<i32, Vec3>>>,
    pub scales: Arc<Mutex<HashMap<i32, Vec3>>>,
    pub global_positions: Arc<Mutex<HashMap<i32, Vec3>>>,
}
impl Default for TransformSnapshot {
    fn default() -> Self {
        Self {
            positions: Arc::new(Mutex::new(HashMap::new())),
            rotations: Arc::new(Mutex::new(HashMap::new())),
            scales: Arc::new(Mutex::new(HashMap::new())),
            global_positions: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

/// Transform mutation queue: Vec<(node_id, Vec3)>
pub struct TransformUpdateQueue {
    pub positions: Arc<Mutex<Vec<(i32, Vec3)>>>,
    pub rotations: Arc<Mutex<Vec<(i32, Vec3)>>>,
    pub scales: Arc<Mutex<Vec<(i32, Vec3)>>>,
    pub global_positions: Arc<Mutex<Vec<(i32, Vec3)>>>,
}
impl Default for TransformUpdateQueue {
    fn default() -> Self {
        Self {
            positions: Arc::new(Mutex::new(Vec::new())),
            rotations: Arc::new(Mutex::new(Vec::new())),
            scales: Arc::new(Mutex::new(Vec::new())),
            global_positions: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

// --- NEW: Hierarchy snapshot ---

pub struct HierarchySnapshot {
    pub parents: Arc<Mutex<HashMap<i32, i32>>>,    // node_id -> parent_id (or -1)
    pub children: Arc<Mutex<HashMap<i32, Vec<i32>>>>, // node_id -> [child_ids]
}
impl Default for HierarchySnapshot {
    fn default() -> Self {
        Self {
            parents: Arc::new(Mutex::new(HashMap::new())),
            children: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

// --- NEW: Tag snapshot ---

pub struct TagSnapshot {
    pub tags: Arc<Mutex<HashMap<i32, String>>>,
}
impl Default for TagSnapshot {
    fn default() -> Self {
        Self {
            tags: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

// --- NEW: Fetch queue ---

pub struct FetchQueue {
    pub requests: Arc<Mutex<Vec<(i32, String)>>>, // (request_id, url)
    next_request_id: Arc<Mutex<i32>>,
}
impl Default for FetchQueue {
    fn default() -> Self {
        Self {
            requests: Arc::new(Mutex::new(Vec::new())),
            next_request_id: Arc::new(Mutex::new(1)),
        }
    }
}

pub struct FetchResults {
    pub results: Arc<Mutex<HashMap<i32, Result<String, String>>>>, // request_id -> result
}
impl Default for FetchResults {
    fn default() -> Self {
        Self {
            results: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

// --- NEW: Navigate queue ---

pub struct NavigateQueue {
    pub queue: Arc<Mutex<Vec<String>>>,
}
impl Default for NavigateQueue {
    fn default() -> Self {
        Self {
            queue: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

// --- WebSocket queues ---

/// Queue of ws connect requests from JS: Vec<(conn_id, url)>
pub struct WsConnectQueue {
    pub requests: Arc<Mutex<Vec<(i32, String)>>>,
    next_id: Arc<Mutex<i32>>,
}
impl Default for WsConnectQueue {
    fn default() -> Self {
        Self {
            requests: Arc::new(Mutex::new(Vec::new())),
            next_id: Arc::new(Mutex::new(1)),
        }
    }
}

/// Inbox of messages received from remote: HashMap<conn_id, Vec<message>>
pub struct WsInbox {
    pub messages: Arc<Mutex<HashMap<i32, Vec<String>>>>,
}
impl Default for WsInbox {
    fn default() -> Self {
        Self {
            messages: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

/// Queue of messages JS wants to send: Vec<(conn_id, message)>
pub struct WsSendQueue {
    pub queue: Arc<Mutex<Vec<(i32, String)>>>,
}
impl Default for WsSendQueue {
    fn default() -> Self {
        Self {
            queue: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

/// Status per connection: "connecting" | "open" | "closed" | "error:<msg>"
pub struct WsStatusMap {
    pub status: Arc<Mutex<HashMap<i32, String>>>,
}
impl Default for WsStatusMap {
    fn default() -> Self {
        Self {
            status: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

/// Queue of close requests from JS: Vec<conn_id>
pub struct WsCloseQueue {
    pub queue: Arc<Mutex<Vec<i32>>>,
}
impl Default for WsCloseQueue {
    fn default() -> Self {
        Self {
            queue: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

// --- Controller registration + touch events ---

/// Stores the registered controller mode ("desktop" | "vr" | "")
pub struct ControllerRegistration {
    pub mode: Arc<Mutex<String>>,
}
impl Default for ControllerRegistration {
    fn default() -> Self {
        Self {
            mode: Arc::new(Mutex::new(String::new())),
        }
    }
}

/// Touch events pushed from Rust into JS: Vec<(node_id, x, y, z)>
pub struct TouchEventQueue {
    pub events: Arc<Mutex<Vec<(i32, f32, f32, f32)>>>,
}
impl Default for TouchEventQueue {
    fn default() -> Self {
        Self {
            events: Arc::new(Mutex::new(Vec::new())),
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
    console.logs.lock().unwrap().push(("log".to_string(), msg.to_string()));
}

#[op2(fast)]
fn op_console_warn(state: &mut OpState, #[string] msg: &str) {
    let console = state.borrow::<ConsoleState>();
    console.logs.lock().unwrap().push(("warn".to_string(), msg.to_string()));
}

#[op2(fast)]
fn op_console_error(state: &mut OpState, #[string] msg: &str) {
    let console = state.borrow::<ConsoleState>();
    console.logs.lock().unwrap().push(("error".to_string(), msg.to_string()));
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
    // setTimeout(fn, 0) fires on the next pump — no thread needed.
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
}

#[op2]
#[serde]
fn op_timers_poll(state: &mut OpState) -> serde_json::Value {
    let timers = state.borrow::<Timers>();
    let mut ready = timers.ready.lock().unwrap();
    let ids: Vec<i32> = ready.drain(..).collect();
    serde_json::Value::from(ids)
}

// --- RAF ops ---

#[op2(fast)]
fn op_raf_register(state: &mut OpState, #[smi] id: i32) {
    let raf = state.borrow::<RafState>();
    raf.pending.lock().unwrap().push(id);
}

#[op2]
#[serde]
fn op_raf_poll(state: &mut OpState) -> serde_json::Value {
    let raf = state.borrow::<RafState>();
    let mut ready = raf.ready.lock().unwrap();
    let list: Vec<(i32, f64)> = ready.drain(..).collect();
    serde_json::to_value(list).unwrap()
}

// --- Attribute ops ---

#[op2(fast)]
fn op_hsml_set_attr(state: &mut OpState, #[smi] node_id: i32, #[string] key: &str, #[string] value: &str) {
    let updates = state.borrow::<AttrUpdates>();
    updates.updates.lock().unwrap().push((node_id, key.to_string(), value.to_string()));
}

#[op2]
#[string]
fn op_hsml_get_attr(state: &mut OpState, #[smi] node_id: i32, #[string] key: &str) -> String {
    let snap = state.borrow::<AttrSnapshot>();
    let data = snap.data.lock().unwrap();
    data.get(&node_id)
        .and_then(|m| m.get(key))
        .cloned()
        .unwrap_or_default()
}

// --- Element creation ops ---

#[op2(fast)]
fn op_hsml_create_element(state: &mut OpState, #[string] tag: &str) -> i32 {
    let queue = state.borrow::<ElementCreationQueue>();
    let mut next_id = queue.next_request_id.lock().unwrap();
    let request_id = *next_id;
    *next_id += 1;
    queue.queue.lock().unwrap().push((request_id, tag.to_string()));
    request_id
}

#[op2(fast)]
fn op_hsml_poll_created_element(state: &mut OpState, #[smi] request_id: i32) -> i32 {
    let results = state.borrow::<ElementCreationResults>();
    results.results.lock().unwrap().remove(&request_id).unwrap_or(-1)
}

// --- Hierarchy ops ---

#[op2(fast)]
fn op_hsml_append_child(state: &mut OpState, #[smi] parent_id: i32, #[smi] child_id: i32) {
    let queue = state.borrow::<HierarchyUpdateQueue>();
    queue.append.lock().unwrap().push((parent_id, child_id));
}

#[op2(fast)]
fn op_hsml_remove(state: &mut OpState, #[smi] node_id: i32) {
    let queue = state.borrow::<RemoveElementQueue>();
    queue.queue.lock().unwrap().push(node_id);
}

#[op2]
#[serde]
fn op_hsml_get_children(state: &mut OpState, #[smi] node_id: i32) -> Vec<i32> {
    let snap = state.borrow::<HierarchySnapshot>();
    snap.children.lock().unwrap().get(&node_id).cloned().unwrap_or_default()
}

#[op2(fast)]
fn op_hsml_get_parent(state: &mut OpState, #[smi] node_id: i32) -> i32 {
    let snap = state.borrow::<HierarchySnapshot>();
    snap.parents.lock().unwrap().get(&node_id).cloned().unwrap_or(-1)
}

// --- Tag ops ---

#[op2]
#[string]
fn op_hsml_get_tag(state: &mut OpState, #[smi] node_id: i32) -> String {
    let snap = state.borrow::<TagSnapshot>();
    snap.tags.lock().unwrap().get(&node_id).cloned().unwrap_or_default()
}

// --- Transform ops (getters) ---

#[op2]
#[serde]
fn op_hsml_get_position(state: &mut OpState, #[smi] node_id: i32) -> Vec<f32> {
    let snap = state.borrow::<TransformSnapshot>();
    let pos = snap.positions.lock().unwrap().get(&node_id).cloned().unwrap_or(Vec3 { x: 0.0, y: 0.0, z: 0.0 });
    vec![pos.x, pos.y, pos.z]
}

#[op2]
#[serde]
fn op_hsml_get_rotation(state: &mut OpState, #[smi] node_id: i32) -> Vec<f32> {
    let snap = state.borrow::<TransformSnapshot>();
    let rot = snap.rotations.lock().unwrap().get(&node_id).cloned().unwrap_or(Vec3 { x: 0.0, y: 0.0, z: 0.0 });
    vec![rot.x, rot.y, rot.z]
}

#[op2]
#[serde]
fn op_hsml_get_scale(state: &mut OpState, #[smi] node_id: i32) -> Vec<f32> {
    let snap = state.borrow::<TransformSnapshot>();
    let scale = snap.scales.lock().unwrap().get(&node_id).cloned().unwrap_or(Vec3 { x: 1.0, y: 1.0, z: 1.0 });
    vec![scale.x, scale.y, scale.z]
}

#[op2]
#[serde]
fn op_hsml_get_global_position(state: &mut OpState, #[smi] node_id: i32) -> Vec<f32> {
    let snap = state.borrow::<TransformSnapshot>();
    let pos = snap.global_positions.lock().unwrap().get(&node_id).cloned().unwrap_or(Vec3 { x: 0.0, y: 0.0, z: 0.0 });
    vec![pos.x, pos.y, pos.z]
}

// --- Transform ops (setters) ---

#[op2(fast)]
fn op_hsml_set_position(state: &mut OpState, #[smi] node_id: i32, x: f32, y: f32, z: f32) {
    let queue = state.borrow::<TransformUpdateQueue>();
    queue.positions.lock().unwrap().push((node_id, Vec3 { x, y, z }));
}

#[op2(fast)]
fn op_hsml_set_rotation(state: &mut OpState, #[smi] node_id: i32, x: f32, y: f32, z: f32) {
    let queue = state.borrow::<TransformUpdateQueue>();
    queue.rotations.lock().unwrap().push((node_id, Vec3 { x, y, z }));
}

#[op2(fast)]
fn op_hsml_set_scale(state: &mut OpState, #[smi] node_id: i32, x: f32, y: f32, z: f32) {
    let queue = state.borrow::<TransformUpdateQueue>();
    queue.scales.lock().unwrap().push((node_id, Vec3 { x, y, z }));
}

#[op2(fast)]
fn op_hsml_set_global_position(state: &mut OpState, #[smi] node_id: i32, x: f32, y: f32, z: f32) {
    let queue = state.borrow::<TransformUpdateQueue>();
    queue.global_positions.lock().unwrap().push((node_id, Vec3 { x, y, z }));
}

// --- Fetch ops ---

#[op2(fast)]
fn op_fetch_request(state: &mut OpState, #[string] url: &str) -> i32 {
    let queue = state.borrow::<FetchQueue>();
    let mut next_id = queue.next_request_id.lock().unwrap();
    let request_id = *next_id;
    *next_id += 1;
    queue.requests.lock().unwrap().push((request_id, url.to_string()));
    request_id
}

#[op2]
#[serde]
fn op_fetch_poll(state: &mut OpState, #[smi] request_id: i32) -> serde_json::Value {
    let results = state.borrow::<FetchResults>();
    let mut map = results.results.lock().unwrap();
    if let Some(result) = map.remove(&request_id) {
        match result {
            Ok(text) => serde_json::json!({"status": "ok", "text": text}),
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
    queue.queue.lock().unwrap().push(url.to_string());
}

// --- WebSocket ops ---

#[op2(fast)]
fn op_ws_connect(state: &mut OpState, #[string] url: &str) -> i32 {
    let queue = state.borrow::<WsConnectQueue>();
    let mut next_id = queue.next_id.lock().unwrap();
    let conn_id = *next_id;
    *next_id += 1;
    // Set initial status
    let status_map = state.borrow::<WsStatusMap>();
    status_map.status.lock().unwrap().insert(conn_id, "connecting".to_string());
    queue.requests.lock().unwrap().push((conn_id, url.to_string()));
    conn_id
}

#[op2(fast)]
fn op_ws_send(state: &mut OpState, #[smi] conn_id: i32, #[string] message: &str) {
    let queue = state.borrow::<WsSendQueue>();
    queue.queue.lock().unwrap().push((conn_id, message.to_string()));
}

#[op2]
#[serde]
fn op_ws_recv(state: &mut OpState, #[smi] conn_id: i32) -> serde_json::Value {
    let inbox = state.borrow::<WsInbox>();
    let mut messages = inbox.messages.lock().unwrap();
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
    status_map.status.lock().unwrap()
        .get(&conn_id)
        .cloned()
        .unwrap_or_else(|| "closed".to_string())
}

#[op2(fast)]
fn op_ws_close(state: &mut OpState, #[smi] conn_id: i32) {
    let queue = state.borrow::<WsCloseQueue>();
    queue.queue.lock().unwrap().push(conn_id);
    let status_map = state.borrow::<WsStatusMap>();
    status_map.status.lock().unwrap().insert(conn_id, "closed".to_string());
}

// --- Controller + Touch ops ---

#[op2(fast)]
fn op_register_controller(state: &mut OpState, #[string] mode: &str) {
    let reg = state.borrow::<ControllerRegistration>();
    *reg.mode.lock().unwrap() = mode.to_string();
}

#[op2]
#[serde]
fn op_poll_touch_events(state: &mut OpState) -> serde_json::Value {
    let queue = state.borrow::<TouchEventQueue>();
    let mut events = queue.events.lock().unwrap();
    if events.is_empty() {
        return serde_json::json!([]);
    }
    let result: Vec<serde_json::Value> = events
        .drain(..)
        .map(|(node_id, x, y, z)| {
            serde_json::json!({"nodeId": node_id, "x": x, "y": y, "z": z})
        })
        .collect();
    serde_json::Value::Array(result)
}

// ---------------------------------------------------------------------------
// V8 Platform Initialization
// ---------------------------------------------------------------------------

/// Initialize V8 platform. MUST be called once before creating any Engine.
/// This is NOT thread-safe and should be called from main thread.
pub fn init_v8_platform() {
    static INIT: std::sync::Once = std::sync::Once::new();
    INIT.call_once(|| {
        // V8 platform initialization happens automatically in deno_core
        // when first JsRuntime is created, but we can force it here
        eprintln!("[js_runtime] V8 platform initializing...");
    });
}

// ---------------------------------------------------------------------------
// Engine
// ---------------------------------------------------------------------------

pub struct Engine {
    rt: JsRuntime,

    // RAF
    raf_pending: Arc<Mutex<Vec<i32>>>,
    raf_ready: Arc<Mutex<Vec<(i32, f64)>>>,

    // Console
    console_logs: Arc<Mutex<Vec<(String, String)>>>,

    // Attributes
    attr_updates: Arc<Mutex<Vec<(i32, String, String)>>>,
    attr_snapshot: Arc<Mutex<HashMap<i32, HashMap<String, String>>>>,

    // Element creation
    element_creation_queue: Arc<Mutex<Vec<(i32, String)>>>,
    element_creation_results: Arc<Mutex<HashMap<i32, i32>>>,

    // Hierarchy
    hierarchy_update_queue_append: Arc<Mutex<Vec<(i32, i32)>>>,
    remove_element_queue: Arc<Mutex<Vec<i32>>>,
    hierarchy_snapshot_parents: Arc<Mutex<HashMap<i32, i32>>>,
    hierarchy_snapshot_children: Arc<Mutex<HashMap<i32, Vec<i32>>>>,

    // Tags
    tag_snapshot: Arc<Mutex<HashMap<i32, String>>>,

    // Transforms
    transform_update_positions: Arc<Mutex<Vec<(i32, Vec3)>>>,
    transform_update_rotations: Arc<Mutex<Vec<(i32, Vec3)>>>,
    transform_update_scales: Arc<Mutex<Vec<(i32, Vec3)>>>,
    transform_update_global_positions: Arc<Mutex<Vec<(i32, Vec3)>>>,
    transform_snapshot_positions: Arc<Mutex<HashMap<i32, Vec3>>>,
    transform_snapshot_rotations: Arc<Mutex<HashMap<i32, Vec3>>>,
    transform_snapshot_scales: Arc<Mutex<HashMap<i32, Vec3>>>,
    transform_snapshot_global_positions: Arc<Mutex<HashMap<i32, Vec3>>>,

    // Fetch
    fetch_queue: Arc<Mutex<Vec<(i32, String)>>>,
    fetch_results: Arc<Mutex<HashMap<i32, Result<String, String>>>>,

    // Navigate
    navigate_queue: Arc<Mutex<Vec<String>>>,

    // WebSocket
    ws_connect_queue: Arc<Mutex<Vec<(i32, String)>>>,
    ws_inbox: Arc<Mutex<HashMap<i32, Vec<String>>>>,
    ws_send_queue: Arc<Mutex<Vec<(i32, String)>>>,
    ws_status_map: Arc<Mutex<HashMap<i32, String>>>,
    ws_close_queue: Arc<Mutex<Vec<i32>>>,

    // Controller + Touch
    controller_mode: Arc<Mutex<String>>,
    touch_events: Arc<Mutex<Vec<(i32, f32, f32, f32)>>>,

    // HTTP Cache & Security
    script_cache: Arc<Mutex<ScriptCache>>,
    fetch_cache: Arc<Mutex<FetchCache>>,
    csp: Arc<Mutex<ContentSecurityPolicy>>,
    document_origin: Arc<Mutex<String>>,
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
        let navigate_queue = NavigateQueue::default();
        let ws_connect_queue = WsConnectQueue::default();
        let ws_inbox = WsInbox::default();
        let ws_send_queue = WsSendQueue::default();
        let ws_status_map = WsStatusMap::default();
        let ws_close_queue = WsCloseQueue::default();
        let controller_registration = ControllerRegistration::default();
        let touch_event_queue = TouchEventQueue::default();

        // Clone Arcs for OpState
        let timers_for_state = Timers {
            ready: timers.ready.clone(),
            cancelled: timers.cancelled.clone(),
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
        let navigate_queue_for_state = NavigateQueue {
            queue: navigate_queue.queue.clone(),
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
        let controller_registration_for_state = ControllerRegistration {
            mode: controller_registration.mode.clone(),
        };
        let touch_event_queue_for_state = TouchEventQueue {
            events: touch_event_queue.events.clone(),
        };

        let ext = Extension::builder("luna_runtime")
            .ops(vec![
                // Console
                op_console_log::decl(),
                op_console_warn::decl(),
                op_console_error::decl(),
                // Performance
                op_now::decl(),
                // Timers
                op_set_timeout::decl(),
                op_clear_timeout::decl(),
                op_timers_poll::decl(),
                // RAF
                op_raf_register::decl(),
                op_raf_poll::decl(),
                // Attributes
                op_hsml_set_attr::decl(),
                op_hsml_get_attr::decl(),
                // Element creation
                op_hsml_create_element::decl(),
                op_hsml_poll_created_element::decl(),
                // Hierarchy
                op_hsml_append_child::decl(),
                op_hsml_remove::decl(),
                op_hsml_get_children::decl(),
                op_hsml_get_parent::decl(),
                // Tags
                op_hsml_get_tag::decl(),
                // Transforms (getters)
                op_hsml_get_position::decl(),
                op_hsml_get_rotation::decl(),
                op_hsml_get_scale::decl(),
                op_hsml_get_global_position::decl(),
                // Transforms (setters)
                op_hsml_set_position::decl(),
                op_hsml_set_rotation::decl(),
                op_hsml_set_scale::decl(),
                op_hsml_set_global_position::decl(),
                // Fetch
                op_fetch_request::decl(),
                op_fetch_poll::decl(),
                // Navigate
                op_navigate::decl(),
                // WebSocket
                op_ws_connect::decl(),
                op_ws_send::decl(),
                op_ws_recv::decl(),
                op_ws_get_status::decl(),
                op_ws_close::decl(),
                // Controller + Touch
                op_register_controller::decl(),
                op_poll_touch_events::decl(),
            ])
            .state(move |state| {
                state.put::<PerfState>(PerfState::default());
                state.put::<Timers>(Timers {
                    ready: timers_for_state.ready.clone(),
                    cancelled: timers_for_state.cancelled.clone(),
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
                state.put::<NavigateQueue>(NavigateQueue {
                    queue: navigate_queue_for_state.queue.clone(),
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
                state.put::<ControllerRegistration>(ControllerRegistration {
                    mode: controller_registration_for_state.mode.clone(),
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
        // Inject bootstrap
        rt.execute_script("<bootstrap>", FastString::Static(BOOTSTRAP_JS))
            .expect("bootstrap failed");

        eprintln!("[js_runtime] Injecting runtime.js...");
        // Inject runtime.js
        rt.execute_script("<runtime>", FastString::Static(RUNTIME_JS))
            .expect("runtime.js failed");

        eprintln!("[js_runtime] Engine created successfully");

        Self {
            rt,
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
            navigate_queue: navigate_queue.queue,
            ws_connect_queue: ws_connect_queue.requests,
            ws_inbox: ws_inbox.messages,
            ws_send_queue: ws_send_queue.queue,
            ws_status_map: ws_status_map.status,
            ws_close_queue: ws_close_queue.queue,
            controller_mode: controller_registration.mode,
            touch_events: touch_event_queue.events,
            // HTTP Cache & Security (initialized with defaults)
            script_cache: Arc::new(Mutex::new(ScriptCache::new(100))), // Max 100 scripts
            fetch_cache: Arc::new(Mutex::new(FetchCache::new(200))),   // Max 200 responses
            csp: Arc::new(Mutex::new(ContentSecurityPolicy::permissive())), // Permissive by default
            document_origin: Arc::new(Mutex::new(String::new())), // Will be set when document loads
        }
    }

    pub fn eval(&mut self, code: &str) -> Result<()> {
        let script_fast = FastString::Owned(code.to_string().into_boxed_str());
        self.rt.execute_script("<eval>", script_fast)?;
        Ok(())
    }

    pub fn fire_raf(&mut self, timestamp_ms: f64) {
        let mut pend = self.raf_pending.lock().unwrap();
        let ids: Vec<i32> = pend.drain(..).collect();
        drop(pend);

        if !ids.is_empty() {
            let mut ready = self.raf_ready.lock().unwrap();
            for id in ids {
                ready.push((id, timestamp_ms));
            }
            drop(ready);
        }

        // Always pump timers/fetch callbacks, even when no RAF callbacks are pending.
        if let Err(e) = self.rt.execute_script("<pump>", FastString::Static("__luna_pump()")) {
            eprintln!("[js_runtime] Error calling pump: {:?}", e);
        }
    }

    // --- Drain methods (called by Bevy systems) ---

    pub fn drain_logs(&self) -> Vec<(String, String)> {
        self.console_logs.lock().unwrap().drain(..).collect()
    }

    pub fn drain_attr_updates(&self) -> Vec<(i32, String, String)> {
        self.attr_updates.lock().unwrap().drain(..).collect()
    }

    pub fn drain_element_creation_queue(&self) -> Vec<(i32, String)> {
        self.element_creation_queue.lock().unwrap().drain(..).collect()
    }

    pub fn drain_hierarchy_append_queue(&self) -> Vec<(i32, i32)> {
        self.hierarchy_update_queue_append.lock().unwrap().drain(..).collect()
    }

    pub fn drain_remove_element_queue(&self) -> Vec<i32> {
        self.remove_element_queue.lock().unwrap().drain(..).collect()
    }

    pub fn drain_transform_position_updates(&self) -> Vec<(i32, Vec3)> {
        self.transform_update_positions.lock().unwrap().drain(..).collect()
    }

    pub fn drain_transform_rotation_updates(&self) -> Vec<(i32, Vec3)> {
        self.transform_update_rotations.lock().unwrap().drain(..).collect()
    }

    pub fn drain_transform_scale_updates(&self) -> Vec<(i32, Vec3)> {
        self.transform_update_scales.lock().unwrap().drain(..).collect()
    }

    pub fn drain_transform_global_position_updates(&self) -> Vec<(i32, Vec3)> {
        self.transform_update_global_positions.lock().unwrap().drain(..).collect()
    }

    pub fn drain_fetch_queue(&self) -> Vec<(i32, String)> {
        self.fetch_queue.lock().unwrap().drain(..).collect()
    }

    pub fn drain_navigate_queue(&self) -> Vec<String> {
        self.navigate_queue.lock().unwrap().drain(..).collect()
    }

    // --- Update snapshot methods (called by Bevy systems) ---

    pub fn update_attr_snapshot(&self, snapshot: HashMap<i32, HashMap<String, String>>) {
        *self.attr_snapshot.lock().unwrap() = snapshot;
    }

    pub fn push_element_creation_result(&self, request_id: i32, node_id: i32) {
        self.element_creation_results.lock().unwrap().insert(request_id, node_id);
    }

    pub fn update_hierarchy_snapshot(
        &self,
        parents: HashMap<i32, i32>,
        children: HashMap<i32, Vec<i32>>,
    ) {
        *self.hierarchy_snapshot_parents.lock().unwrap() = parents;
        *self.hierarchy_snapshot_children.lock().unwrap() = children;
    }

    pub fn update_tag_snapshot(&self, tags: HashMap<i32, String>) {
        *self.tag_snapshot.lock().unwrap() = tags;
    }

    pub fn update_transform_snapshot(
        &self,
        positions: HashMap<i32, Vec3>,
        rotations: HashMap<i32, Vec3>,
        scales: HashMap<i32, Vec3>,
        global_positions: HashMap<i32, Vec3>,
    ) {
        *self.transform_snapshot_positions.lock().unwrap() = positions;
        *self.transform_snapshot_rotations.lock().unwrap() = rotations;
        *self.transform_snapshot_scales.lock().unwrap() = scales;
        *self.transform_snapshot_global_positions.lock().unwrap() = global_positions;
    }

    pub fn push_fetch_result(&self, request_id: i32, result: Result<String, String>) {
        self.fetch_results.lock().unwrap().insert(request_id, result);
    }

    // --- WebSocket methods ---

    /// Drain pending WS connection requests: Vec<(conn_id, url)>
    pub fn drain_ws_connect_queue(&self) -> Vec<(i32, String)> {
        self.ws_connect_queue.lock().unwrap().drain(..).collect()
    }

    /// Drain pending WS send requests: Vec<(conn_id, message)>
    pub fn drain_ws_send_queue(&self) -> Vec<(i32, String)> {
        self.ws_send_queue.lock().unwrap().drain(..).collect()
    }

    /// Drain pending WS close requests: Vec<conn_id>
    pub fn drain_ws_close_queue(&self) -> Vec<i32> {
        self.ws_close_queue.lock().unwrap().drain(..).collect()
    }

    /// Push a received message into JS inbox
    pub fn push_ws_message(&self, conn_id: i32, message: String) {
        self.ws_inbox.lock().unwrap()
            .entry(conn_id)
            .or_default()
            .push(message);
    }

    /// Update connection status ("connecting" | "open" | "closed" | "error:<msg>")
    pub fn set_ws_status(&self, conn_id: i32, status: String) {
        self.ws_status_map.lock().unwrap().insert(conn_id, status);
    }

    /// Remove all state for a closed connection
    pub fn remove_ws_connection(&self, conn_id: i32) {
        self.ws_inbox.lock().unwrap().remove(&conn_id);
        self.ws_status_map.lock().unwrap().remove(&conn_id);
    }

    // --- Controller + Touch ---

    /// Get the registered controller mode for this engine ("desktop" | "vr" | "")
    pub fn get_controller_mode(&self) -> String {
        self.controller_mode.lock().unwrap().clone()
    }

    /// Push a touch event into this engine's JS-visible queue
    pub fn push_touch_event(&self, node_id: i32, x: f32, y: f32, z: f32) {
        self.touch_events.lock().unwrap().push((node_id, x, y, z));
    }

    // -------------------------------------------------------------------------
    // HTTP Cache & Security Methods
    // -------------------------------------------------------------------------

    /// Set the document origin (for CSP and CORS validation)
    pub fn set_document_origin(&self, origin: String) {
        *self.document_origin.lock().unwrap() = origin;
    }

    /// Get the document origin
    pub fn get_document_origin(&self) -> String {
        self.document_origin.lock().unwrap().clone()
    }

    /// Update the CSP
    pub fn set_csp(&self, csp: ContentSecurityPolicy) {
        *self.csp.lock().unwrap() = csp;
    }

    /// Get a reference to the CSP for validation
    pub fn get_csp(&self) -> ContentSecurityPolicy {
        self.csp.lock().unwrap().clone()
    }

    /// Check if a script URL is allowed by CSP
    pub fn csp_allows_script(&self, url: &str) -> bool {
        let csp = self.csp.lock().unwrap();
        let origin = self.document_origin.lock().unwrap();
        csp.allows_script(url, &origin)
    }

    /// Check if a fetch URL is allowed by CSP
    pub fn csp_allows_fetch(&self, url: &str) -> bool {
        let csp = self.csp.lock().unwrap();
        let origin = self.document_origin.lock().unwrap();
        csp.allows_connect(url, &origin)
    }

    /// Get a cached script if valid
    pub fn get_cached_script(&self, url: &str) -> Option<CachedScript> {
        self.script_cache.lock().unwrap().get(url)
    }

    /// Get script for revalidation (even if expired)
    pub fn get_script_for_revalidation(&self, url: &str) -> Option<CachedScript> {
        self.script_cache.lock().unwrap().get_for_revalidation(url)
    }

    /// Cache a script
    pub fn cache_script(&self, url: String, script: CachedScript) {
        self.script_cache.lock().unwrap().insert(url, script);
    }

    /// Get cache statistics
    pub fn get_cache_stats(&self) -> CacheStats {
        self.script_cache.lock().unwrap().stats()
    }

    /// Clear all caches
    pub fn clear_caches(&self) {
        self.script_cache.lock().unwrap().clear();
        self.fetch_cache.lock().unwrap().clear();
    }

    /// Get a cached fetch response if valid
    pub fn get_cached_response(&self, url: &str) -> Option<CachedResponse> {
        self.fetch_cache.lock().unwrap().get(url)
    }

    /// Cache a fetch response
    pub fn cache_response(&self, url: String, response: CachedResponse) {
        self.fetch_cache.lock().unwrap().insert(url, response);
    }
}

// ---------------------------------------------------------------------------
// Bootstrap JS (minimal console/timers/RAF setup)
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

  // Pump function - polls timers and RAF callbacks
  // Called externally by fire_raf, not automatically
  function pump() {
    const tids = core.ops.op_timers_poll();
    for (const id of tids) {
      const fn_ = callbacks.get(id);
      if (fn_) {
        callbacks.delete(id);
        try { fn_(); } catch (e) { console.error(e); }
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

  // Expose pump globally so it can be called from Rust
  global.__luna_pump = pump;

})(globalThis);
"#;

// ---------------------------------------------------------------------------
// Runtime JS (HSML DOM API)
// ---------------------------------------------------------------------------
const RUNTIME_JS: &str = include_str!("../runtime.js");

// ---------------------------------------------------------------------------
// Standalone run_js (kept for backward compat)
// ---------------------------------------------------------------------------
use deno_core::{
    error::AnyError, serde_v8,
};
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
