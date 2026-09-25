//! Bounded component transport. Only the host can bind ports; JS cannot select a peer.
use deno_core::{op2, OpState};
use serde::Serialize;
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        Arc, Mutex, Weak,
    },
};
pub const MAX_MESSAGE: usize = 64 * 1024;
pub const MAX_PENDING: usize = 64;
pub const MAX_CHANNELS: usize = 128;
pub const MAX_QUEUED_BYTES: usize = 1024 * 1024;

pub fn parse_json(text: &str, props: bool) -> Result<Value, String> {
    if text.len() > MAX_MESSAGE {
        return Err("Component payload exceeds 64 KiB".into());
    }
    let v: Value =
        serde_json::from_str(text).map_err(|e| format!("Invalid component JSON: {e}"))?;
    fn valid(v: &Value, depth: usize) -> bool {
        depth <= 32
            && match v {
                Value::Array(a) => a.iter().all(|v| valid(v, depth + 1)),
                Value::Object(o) => o.values().all(|v| valid(v, depth + 1)),
                _ => true,
            }
    }
    if !valid(&v, 0) {
        return Err("Component data exceeds depth 32".into());
    }
    if props && !v.is_object() {
        return Err("Component props must be a JSON object".into());
    }
    Ok(v)
}
pub fn event_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name.as_bytes()[0].is_ascii_lowercase()
        && name
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_' || c == b'-')
}
pub fn parse_events(raw: &str) -> Result<BTreeSet<String>, String> {
    if raw.len() > 4096 {
        return Err("Component event declaration too large".into());
    }
    let events: BTreeSet<_> = raw
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect();
    if events.len() > 32 || !events.iter().all(|s| event_name(s)) {
        return Err(
            "Expected up to 32 lowercase component event names; wildcards are not allowed".into(),
        );
    }
    Ok(events)
}
#[derive(Clone, Serialize)]
pub struct Snapshot {
    pub connected: bool,
    pub generation: String,
    pub revision: u64,
    pub props: Value,
}
impl Default for Snapshot {
    fn default() -> Self {
        Self {
            connected: false,
            generation: String::new(),
            revision: 0,
            props: serde_json::json!({}),
        }
    }
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Delivery {
    pub node_id: i32,
    #[serde(rename = "type")]
    pub kind: String,
    pub detail: Value,
    pub origin: String,
    pub generation: String,
    pub sequence: u64,
}
struct Queued {
    name: String,
    data: Value,
    bytes: usize,
    sequence: u64,
}
struct ChannelState {
    props: Value,
    revision: u64,
    events: BTreeSet<String>,
    closed: bool,
    queue: VecDeque<Queued>,
    sequence: u64,
    messages: VecDeque<Queued>,
    message_sequence: u64,
}
pub struct Channel {
    pub generation: String,
    pub local_id: i32,
    pub origin: String,
    parent: Weak<ComponentPort>,
    child: Weak<ComponentPort>,
    state: Mutex<ChannelState>,
}
impl Channel {
    pub fn new(
        parent: &Arc<ComponentPort>,
        child: &Arc<ComponentPort>,
        local_id: i32,
        origin: String,
        props: Value,
        events: BTreeSet<String>,
    ) -> Result<Arc<Self>, String> {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        if parent.closed.load(Ordering::Acquire) || child.closed.load(Ordering::Acquire) {
            return Err("Component worker closed".into());
        }
        let mut children = parent.children.lock().unwrap();
        if children.contains_key(&local_id) {
            return Err("Component include already bound".into());
        }
        if children.len() >= MAX_CHANNELS {
            return Err("Component channel limit reached".into());
        }
        let channel = Arc::new(Self {
            generation: NEXT.fetch_add(1, Ordering::Relaxed).to_string(),
            local_id,
            origin,
            parent: Arc::downgrade(parent),
            child: Arc::downgrade(child),
            state: Mutex::new(ChannelState {
                props,
                revision: 1,
                events,
                closed: false,
                queue: VecDeque::new(),
                sequence: 0,
                messages: VecDeque::new(),
                message_sequence: 0,
            }),
        });
        children.insert(local_id, channel.clone());
        drop(children);
        *child.incoming.lock().unwrap() = Some(channel.clone());
        child.wake.store(true, Ordering::Release);
        Ok(channel)
    }
    pub fn update_props(&self, props: Value) {
        let mut s = self.state.lock().unwrap();
        if !s.closed && s.props != props {
            s.props = props;
            s.revision += 1;
            if let Some(c) = self.child.upgrade() {
                c.wake.store(true, Ordering::Release);
            }
        }
    }
    pub fn close(&self) {
        let mut s = self.state.lock().unwrap();
        if s.closed {
            return;
        }
        s.closed = true;
        let bytes = s.queue.drain(..).map(|q| q.bytes).sum::<usize>()
            + s.messages.drain(..).map(|q| q.bytes).sum::<usize>();
        if let Some(p) = self.parent.upgrade() {
            p.queued_bytes.fetch_sub(bytes, Ordering::AcqRel);
        }
        if let Some(c) = self.child.upgrade() {
            c.wake.store(true, Ordering::Release);
        }
    }
    pub fn is_open(&self) -> bool {
        !self.state.lock().unwrap().closed
    }
    fn snapshot(&self) -> Snapshot {
        let s = self.state.lock().unwrap();
        if s.closed {
            return Snapshot::default();
        }
        Snapshot {
            connected: true,
            generation: self.generation.clone(),
            revision: s.revision,
            props: s.props.clone(),
        }
    }
    fn emit(&self, name: String, text: &str) -> Result<(), String> {
        if !event_name(&name) {
            return Err("Invalid component event name".into());
        }
        let data = parse_json(text, false)?;
        let mut s = self.state.lock().unwrap();
        if s.closed {
            return Err("Component channel disconnected".into());
        }
        if !s.events.contains(&name) {
            return Err(format!("Component event not declared by parent: {name}"));
        }
        if s.queue.len() >= MAX_PENDING {
            return Err("Component event queue full (64 pending)".into());
        }
        let parent = self
            .parent
            .upgrade()
            .ok_or("Component parent unavailable")?;
        if parent.closed.load(Ordering::Acquire) {
            return Err("Component parent closed".into());
        }
        let bytes = text.len() + name.len() + 64;
        parent
            .queued_bytes
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                n.checked_add(bytes).filter(|n| *n <= MAX_QUEUED_BYTES)
            })
            .map_err(|_| "Component parent queue exceeds 1 MiB")?;
        s.sequence += 1;
        let sequence = s.sequence;
        s.queue.push_back(Queued {
            name,
            data,
            bytes,
            sequence,
        });
        parent.wake.store(true, Ordering::Release);
        Ok(())
    }
}
#[derive(Default)]
pub struct ComponentPort {
    incoming: Mutex<Option<Arc<Channel>>>,
    children: Mutex<BTreeMap<i32, Arc<Channel>>>,
    wake: AtomicBool,
    closed: AtomicBool,
    queued_bytes: AtomicUsize,
}
impl ComponentPort {
    pub fn send(&self, id: i32, name: String, text: &str) -> Result<(), String> {
        if self.closed.load(Ordering::Acquire) { return Err("Component worker closed".into()); }
        if !event_name(&name) { return Err("Invalid component message name".into()); }
        let data = parse_json(text, false)?;
        let channel = self.children.lock().unwrap().get(&id).cloned()
            .ok_or("Include component channel is not connected")?;
        let mut s = channel.state.lock().unwrap();
        if s.closed { return Err("Component channel disconnected".into()); }
        let child = channel.child.upgrade().ok_or("Component child unavailable")?;
        if child.closed.load(Ordering::Acquire) { return Err("Component child closed".into()); }
        if s.messages.len() >= MAX_PENDING { return Err("Component message queue full (64 pending)".into()); }
        let bytes = text.len() + name.len() + 64;
        self.queued_bytes.fetch_update(Ordering::AcqRel, Ordering::Acquire,
            |n| n.checked_add(bytes).filter(|n| *n <= MAX_QUEUED_BYTES))
            .map_err(|_| "Component parent queue exceeds 1 MiB")?;
        s.message_sequence += 1;
        let sequence = s.message_sequence;
        s.messages.push_back(Queued { name, data, bytes, sequence });
        child.wake.store(true, Ordering::Release);
        Ok(())
    }

    pub fn validate_message(&self, generation: &str) -> bool {
        self.incoming.lock().unwrap().as_ref()
            .is_some_and(|c| c.generation == generation && c.is_open())
    }

    pub fn drain_messages(&self) -> Vec<MessageDelivery> {
        let Some(c) = self.incoming.lock().unwrap().clone() else { return Vec::new(); };
        let mut s = c.state.lock().unwrap();
        if s.closed { return Vec::new(); }
        let mut out = Vec::with_capacity(s.messages.len());
        while let Some(q) = s.messages.pop_front() {
            if let Some(parent) = c.parent.upgrade() {
                parent.queued_bytes.fetch_sub(q.bytes, Ordering::AcqRel);
            }
            out.push(MessageDelivery { kind:format!("message:{}", q.name), detail:q.data,
                generation:c.generation.clone(), sequence:q.sequence });
        }
        out
    }
    pub fn take_wake(&self) -> bool {
        // Idle ports are polled by the host each frame. Avoid an atomic write
        // (and exclusive cache-line ownership) unless a producer requested work.
        self.wake.load(Ordering::Acquire) && self.wake.swap(false, Ordering::AcqRel)
    }
    pub fn close(&self) {
        self.closed.store(true, Ordering::Release);
        if let Some(c) = self.incoming.lock().unwrap().take() {
            c.close();
        }
        let children = std::mem::take(&mut *self.children.lock().unwrap());
        for c in children.values() {
            c.close();
        }
    }
    pub fn remove_child(&self, local_id: i32, generation: &str) {
        let mut children = self.children.lock().unwrap();
        if children
            .get(&local_id)
            .is_some_and(|c| c.generation == generation)
        {
            if let Some(c) = children.remove(&local_id) {
                c.close();
            }
        }
    }
    pub fn context(&self) -> Snapshot {
        self.incoming
            .lock()
            .unwrap()
            .as_ref()
            .map(|c| c.snapshot())
            .unwrap_or_default()
    }
    pub fn child_props(&self, id: i32) -> Option<Value> {
        self.children
            .lock()
            .unwrap()
            .get(&id)
            .filter(|c| c.is_open())
            .map(|c| c.snapshot().props)
    }
    pub fn emit(&self, name: String, text: &str) -> Result<(), String> {
        if self.closed.load(Ordering::Acquire) {
            return Err("Component worker closed".into());
        }
        let c = self
            .incoming
            .lock()
            .unwrap()
            .clone()
            .ok_or("No parent component channel")?;
        c.emit(name, text)
    }
    pub fn validate(&self, id: i32, generation: &str) -> bool {
        self.children
            .lock()
            .unwrap()
            .get(&id)
            .is_some_and(|c| c.generation == generation && c.is_open())
    }
    pub fn drain(&self) -> Vec<Delivery> {
        let mut out = Vec::new();
        let children = self.children.lock().unwrap();
        for (id, c) in children.iter() {
            let mut s = c.state.lock().unwrap();
            if s.closed {
                continue;
            }
            while let Some(q) = s.queue.pop_front() {
                self.queued_bytes.fetch_sub(q.bytes, Ordering::AcqRel);
                out.push(Delivery {
                    node_id: *id,
                    kind: format!("component:{}", q.name),
                    detail: q.data,
                    origin: c.origin.clone(),
                    generation: c.generation.clone(),
                    sequence: q.sequence,
                });
            }
        }
        out
    }
}

#[derive(Serialize)]
pub struct MessageDelivery {
    #[serde(rename = "type")]
    pub kind: String,
    pub detail: Value,
    pub generation: String,
    pub sequence: u64,
}

#[op2(fast)]
pub fn op_component_send(state: &mut OpState, #[smi] id: i32,
    #[string] name: String, #[string] payload: String) -> Result<(), anyhow::Error> {
    state.borrow::<Arc<ComponentPort>>().send(id, name, &payload).map_err(anyhow::Error::msg)
}
#[op2]
#[serde]
pub fn op_component_poll_messages(state: &mut OpState) -> Vec<MessageDelivery> {
    state.borrow::<Arc<ComponentPort>>().drain_messages()
}
#[op2(fast)]
pub fn op_component_validate_message(state: &mut OpState, #[string] generation: String) -> bool {
    state.borrow::<Arc<ComponentPort>>().validate_message(&generation)
}
#[op2]
#[serde]
pub fn op_component_context(state: &mut OpState) -> Snapshot {
    state.borrow::<Arc<ComponentPort>>().context()
}
#[op2]
#[serde]
pub fn op_component_props(state: &mut OpState, #[smi] id: i32) -> Option<Value> {
    state.borrow::<Arc<ComponentPort>>().child_props(id)
}
#[op2(fast)]
pub fn op_component_emit(
    state: &mut OpState,
    #[string] name: String,
    #[string] payload: String,
) -> Result<(), anyhow::Error> {
    state
        .borrow::<Arc<ComponentPort>>()
        .emit(name, &payload)
        .map_err(anyhow::Error::msg)
}
#[op2]
#[serde]
pub fn op_component_poll(state: &mut OpState) -> Vec<Delivery> {
    state.borrow::<Arc<ComponentPort>>().drain()
}
#[op2(fast)]
pub fn op_component_validate(
    state: &mut OpState,
    #[smi] id: i32,
    #[string] generation: String,
) -> bool {
    state
        .borrow::<Arc<ComponentPort>>()
        .validate(id, &generation)
}
