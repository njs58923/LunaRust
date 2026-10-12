//! Registration commands only. Pose following belongs to the host ECS.
use deno_core::{op2, OpState};
use serde::Serialize;

pub const MAX_ANCHORS: usize = 64;
#[derive(Debug, Clone)]
pub enum Source {
    Native(String),
    Element(i32),
}
#[derive(Debug)]
pub struct Request {
    pub id: u32,
    pub key: String,
    pub source: Option<Source>,
    pub guard: Option<u32>,
}
#[derive(Serialize)]
pub struct Reply {
    pub id: u32,
    pub error: Option<String>,
}
#[derive(Default)]
pub struct AnchorQueue {
    next: u32,
    pub requests: Vec<Request>,
    pub replies: Vec<Reply>,
}

pub fn valid_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= 128
        && key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-.:".contains(&b))
}

#[op2(fast)]
pub fn op_anchor_request(
    state: &mut OpState,
    #[string] key: &str,
    #[string] native: &str,
    #[smi] node: i32,
    remove: bool,
    expected: u32,
) -> Result<u32, anyhow::Error> {
    anyhow::ensure!(valid_key(key), "Invalid anchor key");
    let queue = state.borrow_mut::<AnchorQueue>();
    anyhow::ensure!(
        queue.requests.len() < MAX_ANCHORS,
        "Anchor request queue full"
    );
    let source = if remove {
        None
    } else if !native.is_empty() {
        anyhow::ensure!(
            native.starts_with("__") && valid_key(native),
            "Invalid native reference"
        );
        Some(Source::Native(native.into()))
    } else {
        anyhow::ensure!(node >= 0, "Anchor element must be resolved");
        Some(Source::Element(node))
    };
    queue.next = queue
        .next
        .checked_add(1)
        .ok_or_else(|| anyhow::anyhow!("Anchor request ID exhausted"))?;
    let id = queue.next;
    queue.requests.push(Request {
        id,
        key: key.into(),
        source,
        guard: (expected != 0).then_some(expected),
    });
    Ok(id)
}
#[op2]
#[serde]
pub fn op_anchor_poll(state: &mut OpState) -> Vec<Reply> {
    std::mem::take(&mut state.borrow_mut::<AnchorQueue>().replies)
}
