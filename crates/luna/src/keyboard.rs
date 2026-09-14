//! Keyboard focus belongs to one live document (including nested includes).
use crate::{
    js::{JsWorkerCommand, ScriptRuntimeManager},
    ElemenetWorld,
};
use bevy::input::{keyboard::Key, ButtonState};
use bevy::prelude::*;
use bevy::window::{Ime, PrimaryWindow};
use bevy::winit::WinitEvent;
use serde_json::{json, Value};
use specs::WorldExt;
use virtual_dom::dom::element::{Attrs, Hierarchy};

#[derive(Resource, Default)]
pub struct KeyboardFocus {
    pub space: Option<u32>,
    pub editable: bool,
    revision: u64,
    published: Option<(Option<u32>, bool, bool, Option<u32>, u64)>,
    previous: Option<u32>,
    clipboard_write: Option<(u32, u64)>,
}

fn visible(dom: &specs::World, id: u32) -> bool {
    let entities = dom.entities();
    let attrs = dom.read_storage::<Attrs>();
    let hierarchy = dom.read_storage::<Hierarchy>();
    let mut current = Some(entities.entity(id));
    for _ in 0..256 {
        let Some(node) = current else {
            return true;
        };
        if !entities.is_alive(node)
            || attrs
                .get(node)
                .and_then(|a| a.0.get("visible"))
                .is_some_and(|v| v == "false")
        {
            return false;
        }
        current = hierarchy.get(node).and_then(|h| h.parent);
    }
    false
}

fn in_shell(dom: &specs::World, id: u32, shell: u32) -> bool {
    let hierarchy = dom.read_storage::<Hierarchy>();
    let mut current = Some(dom.entities().entity(id));
    for _ in 0..256 {
        let Some(node) = current else {
            return false;
        };
        if node.id() == shell {
            return true;
        }
        current = hierarchy.get(node).and_then(|h| h.parent);
    }
    false
}

/// Runs before hits are drained. A keyboard key must not steal the edited field's focus.
pub fn focus_from_hits(
    hits: Res<crate::touch::HostToqueHits>,
    dom: Res<ElemenetWorld>,
    mut focus: ResMut<KeyboardFocus>,
) {
    let shell = crate::ui::find_system_shell_space(&dom.0);
    for hit in &hits.0 {
        let Some(space) =
            crate::js::find_owner_space_id(&dom.0, dom.0.entities().entity(hit.node_id))
        else {
            continue;
        };
        if shell.is_some_and(|s| in_shell(&dom.0, space, s)) {
            continue;
        }
        if focus.space != Some(space) {
            focus.space = Some(space);
            focus.editable = false;
            focus.revision += 1;
        }
    }
}

fn deliver(manager: &mut ScriptRuntimeManager, space: u32, value: Value) {
    if let Some(worker) = manager.contexts.get_mut(&space) {
        if worker
            .try_send(JsWorkerCommand::PushKeyboard(vec![value]))
            .is_ok()
        {
            worker.needs_tick = true;
        }
    }
}

fn normalize_packet(packet: &Value) -> Option<Value> {
    let kind = packet["type"].as_str()?;
    if !matches!(
        kind,
        "keydown" | "keyup" | "text" | "compositionstart" | "compositionupdate" | "compositionend"
    ) {
        return None;
    }
    let text = packet["text"].as_str().unwrap_or("");
    let key = packet["key"].as_str().unwrap_or("");
    let code = packet["code"].as_str().unwrap_or("");
    if text.len() > 4096 || key.len() > 128 || code.len() > 64 {
        return None;
    }
    Some(json!({"type":kind,"key":key,"code":code,"text":text,
        "location":if code.starts_with("Numpad") {3} else if matches!(code,"ShiftLeft"|"ControlLeft"|"AltLeft"|"MetaLeft") {1} else if matches!(code,"ShiftRight"|"ControlRight"|"AltRight"|"MetaRight") {2} else {0},
        "repeat":packet["repeat"].as_bool().unwrap_or(false),
        "ctrlKey":packet["ctrlKey"].as_bool().unwrap_or(false),"altKey":packet["altKey"].as_bool().unwrap_or(false),
        "shiftKey":packet["shiftKey"].as_bool().unwrap_or(false),"metaKey":packet["metaKey"].as_bool().unwrap_or(false),
        "isComposing":packet["isComposing"].as_bool().unwrap_or(false)}))
}

fn clipboard_get(world: &mut World) -> String {
    #[cfg(not(target_os = "android"))]
    {
        return world
            .get_resource_mut::<bevy_egui::EguiClipboard>()
            .and_then(|mut c| c.get_contents())
            .unwrap_or_default();
    }
    #[cfg(target_os = "android")]
    {
        let _ = world;
        String::new()
    }
}
fn clipboard_set(world: &mut World, text: &str) {
    #[cfg(not(target_os = "android"))]
    if let Some(mut c) = world.get_resource_mut::<bevy_egui::EguiClipboard>() {
        c.set_contents(text);
    }
    #[cfg(target_os = "android")]
    let _ = (world, text);
}

pub fn apply_commands(world: &mut World, sender: u32, commands: Vec<Value>) {
    if commands.is_empty() {
        return;
    }
    if !world.contains_resource::<KeyboardFocus>() {
        world.init_resource::<KeyboardFocus>();
    }
    for command in commands {
        let shell = world
            .get_resource::<ElemenetWorld>()
            .and_then(|dom| crate::ui::find_system_shell_space(&dom.0));
        let action = command["action"].as_str().unwrap_or("");
        if action == "focus" || action == "blur" {
            let mut focus = world.resource_mut::<KeyboardFocus>();
            // A script cannot focus a background document: only a user hit selects it.
            if focus.space == Some(sender) {
                focus.editable = action == "focus" && command["editable"].as_bool() == Some(true);
                focus.revision += 1;
            }
        } else if action == "clipboard" {
            let mut focus = world.resource_mut::<KeyboardFocus>();
            let allowed = focus.space == Some(sender)
                && command["revision"].as_u64() == Some(focus.revision)
                && focus.clipboard_write == Some((sender, focus.revision));
            focus.clipboard_write = None;
            if allowed {
                if let Some(text) = command["text"].as_str().filter(|text| text.len() <= 4096) {
                    clipboard_set(world, text);
                }
            }
        } else if action == "send" && shell == Some(sender) {
            let focus = world.resource::<KeyboardFocus>();
            if command["revision"].as_u64() != Some(focus.revision) {
                continue;
            }
            let Some(target) = focus.space else {
                continue;
            };
            let Some(dom) = world.get_resource::<ElemenetWorld>() else {
                continue;
            };
            if !visible(&dom.0, target) {
                continue;
            }
            let Some(mut packet) = normalize_packet(&command["packet"]) else {
                continue;
            };
            if packet["type"] == "keydown"
                && (packet["ctrlKey"] == true || packet["metaKey"] == true)
            {
                let key = packet["key"].as_str().unwrap_or("").to_lowercase();
                if key == "v" {
                    let pasted = clipboard_get(world);
                    packet["clipboardText"] = Value::String(pasted.chars().take(4096).collect());
                } else if key == "c" || key == "x" {
                    let mut focus = world.resource_mut::<KeyboardFocus>();
                    focus.clipboard_write = Some((target, focus.revision));
                }
            }
            if let Some(mut manager) = world.get_non_send_resource_mut::<ScriptRuntimeManager>() {
                deliver(
                    &mut manager,
                    target,
                    json!({"type":"input","packet":packet}),
                );
            }
        }
    }
}

pub fn keyboard_system(
    mut events: EventReader<WinitEvent>,
    mode: Res<crate::RenderMode>,
    dom: Res<ElemenetWorld>,
    mut focus: ResMut<KeyboardFocus>,
    mut manager: NonSendMut<ScriptRuntimeManager>,
    mut windows: Query<(Entity, &mut Window), With<PrimaryWindow>>,
    devtool: Res<crate::DevtoolVisible>,
    global_devtool: Res<crate::GlobalDevtoolVisible>,
    mut composing: Local<bool>,
    mut held: Local<std::collections::HashSet<KeyCode>>,
    mut egui: bevy_egui::EguiContexts,
    xr: Option<Res<bevy_mod_xr::session::XrState>>,
) {
    let shell = crate::ui::find_system_shell_space(&dom.0);
    let window_active = windows.get_single().is_ok_and(|(_, w)| w.focused)
        && !egui
            .try_ctx_mut()
            .is_some_and(|ctx| ctx.wants_keyboard_input());
    // VR focus does not depend on the desktop spectator window being foreground.
    let ui_active = if mode.is_vr {
        matches!(xr.as_deref(), Some(bevy_mod_xr::session::XrState::Running))
    } else {
        window_active
    };
    if !ui_active
        || devtool.0
        || global_devtool.0
        || focus
            .space
            .is_some_and(|s| !manager.contexts.contains_key(&s) || !visible(&dom.0, s))
    {
        if focus.space.take().is_some() {
            focus.revision += 1;
        }
        focus.editable = false;
        *composing = false;
        held.clear();
    }
    if let Ok((_, mut window)) = windows.get_single_mut() {
        window.ime_enabled = focus.editable;
    }
    let state = (
        focus.space,
        focus.editable,
        mode.is_vr,
        shell,
        focus.revision,
    );
    if focus.published != Some(state) {
        if let Some(old) = focus.previous.filter(|old| Some(*old) != focus.space) {
            deliver(&mut manager, old, json!({"type":"state","focused":false}));
        }
        if let Some(target) = focus.space {
            deliver(
                &mut manager,
                target,
                json!({"type":"state","focused":true,"editable":focus.editable,"revision":focus.revision,"vr":mode.is_vr}),
            );
        }
        if let Some(shell) = shell {
            deliver(
                &mut manager,
                shell,
                json!({"type":"state","focused":false,"hasTarget":focus.space.is_some(),"editable":focus.editable,"revision":focus.revision,"vr":mode.is_vr}),
            );
        }
        focus.previous = focus.space;
        focus.published = Some(state);
    }
    let batch: Vec<_> = events.read().cloned().collect();
    if window_active && focus.space.is_some() && !devtool.0 && !global_devtool.0 {
        if let Ok((window, _)) = windows.get_single() {
            for packet in device_packets(&batch, window, &mut held, &mut composing) {
                if let Some(shell) = shell {
                    deliver(
                        &mut manager,
                        shell,
                        json!({"type":"device","revision":focus.revision,"packet":packet}),
                    );
                }
            }
        }
    }
}

// Preserve Winit ordering: ReceivedCharacter precedes its KeyboardInput,
// including Enter/Tab/control characters. Independent readers mis-pair them.
#[allow(deprecated)]
fn device_packets(
    events: &[WinitEvent],
    window: Entity,
    held: &mut std::collections::HashSet<KeyCode>,
    composing: &mut bool,
) -> Vec<Value> {
    let mut packets = Vec::new();
    let mut text = String::new();
    for event in events {
        match event {
            WinitEvent::ReceivedCharacter(e) if e.window == window => text = e.char.to_string(),
            WinitEvent::KeyboardInput(key) if key.window == window => {
                let repeat = if key.state == ButtonState::Pressed {
                    !held.insert(key.key_code)
                } else {
                    held.remove(&key.key_code);
                    false
                };
                let logical = match &key.logical_key {
                    Key::Character(s) => s.to_string(),
                    Key::Space => " ".into(),
                    Key::Super => "Meta".into(),
                    other => format!("{other:?}"),
                };
                let code = match key.key_code {
                    KeyCode::SuperLeft => "MetaLeft".into(),
                    KeyCode::SuperRight => "MetaRight".into(),
                    other => format!("{other:?}"),
                };
                packets.push(json!({"type":if key.state==ButtonState::Pressed{"keydown"}else{"keyup"},"key":logical,"code":code,"text":std::mem::take(&mut text),"repeat":repeat,
                    "ctrlKey":held.contains(&KeyCode::ControlLeft)||held.contains(&KeyCode::ControlRight),
                    "shiftKey":held.contains(&KeyCode::ShiftLeft)||held.contains(&KeyCode::ShiftRight),
                    "altKey":held.contains(&KeyCode::AltLeft)||held.contains(&KeyCode::AltRight),
                    "metaKey":held.contains(&KeyCode::SuperLeft)||held.contains(&KeyCode::SuperRight),"isComposing":*composing}));
            }
            WinitEvent::Ime(Ime::Preedit {
                window: w, value, ..
            }) if *w == window => {
                if !*composing {
                    packets.push(json!({"type":"compositionstart","text":""}));
                }
                *composing = true;
                packets.push(json!({"type":"compositionupdate","text":value}));
            }
            WinitEvent::Ime(Ime::Commit { window: w, value }) if *w == window => {
                *composing = false;
                packets.push(json!({"type":"compositionend","text":value}));
                packets.push(json!({"type":"text","text":value}));
            }
            WinitEvent::Ime(Ime::Disabled { window: w }) if *w == window => {
                if *composing {
                    packets.push(json!({"type":"compositionend","text":""}));
                }
                *composing = false;
            }
            _ => {}
        }
    }
    packets
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[allow(deprecated)]
    fn keyboard_preserves_text_and_modifier_order() {
        use bevy::input::keyboard::KeyboardInput;
        use bevy::window::ReceivedCharacter;
        let window = Entity::PLACEHOLDER;
        let key = |code, logical, state| {
            WinitEvent::KeyboardInput(KeyboardInput {
                key_code: code,
                logical_key: logical,
                state,
                window,
            })
        };
        let text = |value: &str| {
            WinitEvent::ReceivedCharacter(ReceivedCharacter {
                window,
                char: value.into(),
            })
        };
        let events = [
            text("\r"),
            key(KeyCode::Enter, Key::Enter, ButtonState::Pressed),
            key(KeyCode::ShiftLeft, Key::Shift, ButtonState::Pressed),
            text("Á"),
            key(
                KeyCode::KeyA,
                Key::Character("Á".into()),
                ButtonState::Pressed,
            ),
            key(KeyCode::ShiftLeft, Key::Shift, ButtonState::Released),
        ];
        let packets = device_packets(&events, window, &mut default(), &mut false);
        assert_eq!(packets[0]["text"], "\r");
        assert_eq!(packets[2]["text"], "Á");
        assert_eq!(packets[2]["shiftKey"], true);
        assert_eq!(packets[3]["shiftKey"], false);
    }

    #[test]
    fn keyboard_packets_cannot_forge_targets_or_trust() {
        let packet = normalize_packet(&json!({"type":"keydown","key":"a","code":"KeyA","target":55,"isTrusted":true,"clipboardText":"secret"})).unwrap();
        assert!(packet.get("target").is_none());
        assert!(packet.get("isTrusted").is_none());
        assert!(packet.get("clipboardText").is_none());
        assert_eq!(packet["key"], "a");
        assert!(normalize_packet(&json!({"type":"focus"})).is_none());
        assert!(normalize_packet(&json!({"type":"text","text":"x".repeat(4097)})).is_none());
    }

    #[test]
    fn keyboard_background_document_cannot_take_focus() {
        let mut world = World::new();
        world.insert_resource(KeyboardFocus {
            space: Some(7),
            ..default()
        });
        apply_commands(
            &mut world,
            99,
            vec![json!({"action":"focus","editable":true})],
        );
        assert!(!world.resource::<KeyboardFocus>().editable);
        apply_commands(
            &mut world,
            7,
            vec![json!({"action":"focus","editable":true})],
        );
        assert!(world.resource::<KeyboardFocus>().editable);
        apply_commands(&mut world, 99, vec![json!({"action":"blur"})]);
        assert!(world.resource::<KeyboardFocus>().editable);
        apply_commands(&mut world, 7, vec![json!({"action":"blur"})]);
        assert!(!world.resource::<KeyboardFocus>().editable);
    }
}
