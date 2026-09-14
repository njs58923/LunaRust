//! Bounded keyboard mailbox. Focus and injection are authorized by the host.
use deno_core::{op2, OpState};
use serde_json::Value;

#[derive(Default)]
pub struct KeyboardQueue {
    pub outgoing: Vec<Value>,
    pub incoming: Vec<Value>,
}

#[op2]
pub fn op_keyboard_command(
    state: &mut OpState,
    #[serde] value: serde_json::Value,
) -> Result<(), deno_core::error::AnyError> {
    let queue = state.borrow_mut::<KeyboardQueue>();
    if queue.outgoing.len() >= 128 || value.to_string().len() > 8192 {
        return Err(deno_core::error::type_error(
            "Keyboard queue or payload limit exceeded",
        ));
    }
    queue.outgoing.push(value);
    Ok(())
}

#[op2]
#[serde]
pub fn op_keyboard_read(state: &mut OpState) -> Vec<Value> {
    std::mem::take(&mut state.borrow_mut::<KeyboardQueue>().incoming)
}
