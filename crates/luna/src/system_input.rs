//! System-level input normalizado (shell toggle, futuro: back/capture).
//!
//! Los dispatchers (`vr_locomotion`, `desktop_locomotion`) pushean
//! [`SystemInputEvent`] al recurso [`HostSystemInputEvents`]. El sistema
//! [`dispatch_system_input_events_to_js`] enruta cada evento a TODOS los spaces
//! con capability [`CapabilityBits::READ_SYSTEM_INPUT`]. La permission es
//! "elevada" — sólo concedida a spaces `managed-by="dimension.luna"`.

use bevy::prelude::*;

use crate::js::{JsWorkerCommand, ScriptRuntimeManager};
use crate::permissions::{space_has_capability, CapabilityBits, SpacePolicies};

/// Acciones semánticas del system input. Hardware-agnóstico — el dispatcher
/// VR o Desktop traduce raw → action.
#[derive(Debug, Clone, Copy)]
pub enum SystemInputAction {
    /// Pedido de "abrir/mostrar shell" (botón menu VR, ESC desktop).
    Shell,
}

impl SystemInputAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            SystemInputAction::Shell => "shell",
        }
    }
}

/// Origen físico del input — informativo, las apps suelen quedarse con `action`.
#[derive(Debug, Clone, Copy)]
pub enum SystemInputSource {
    VrMenu,
    KbEscape,
}

impl SystemInputSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            SystemInputSource::VrMenu => "vr_menu",
            SystemInputSource::KbEscape => "kb_escape",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct SystemInputEvent {
    pub action: SystemInputAction,
    pub source: SystemInputSource,
}

/// Cola host → JS. Drenada cada frame por `dispatch_system_input_events_to_js`.
#[derive(Resource, Default)]
pub struct HostSystemInputEvents(pub Vec<SystemInputEvent>);

/// Broadcast a todos los spaces con cap `READ_SYSTEM_INPUT`.
pub fn dispatch_system_input_events_to_js(
    mut events: ResMut<HostSystemInputEvents>,
    mut manager: NonSendMut<ScriptRuntimeManager>,
    space_policies: Res<SpacePolicies>,
) {
    if events.0.is_empty() {
        return;
    }

    let batch: Vec<(String, String)> = events
        .0
        .drain(..)
        .map(|e| (e.action.as_str().to_string(), e.source.as_str().to_string()))
        .collect();

    for (space_id, worker) in manager.contexts.iter_mut() {
        if !space_has_capability(*space_id, CapabilityBits::READ_SYSTEM_INPUT, &space_policies) {
            continue;
        }
        let send_result = worker
            .cmd_tx
            .send(JsWorkerCommand::PushSystemInputEvents(batch.clone()));
        if send_result.is_ok() {
            worker.needs_tick = true;
        }
    }
}
