use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use specs::WorldExt;

use crate::js::{find_owner_space_id, JsWorkerCommand, ScriptRuntimeManager};
use crate::permissions::{space_has_capability, CapabilityBits, SpacePolicies};
use crate::{ElemenetWorld, LogPanel, SpaceHandleTables};

/// Marker component for primitives that can receive a normalized `toque`.
#[derive(Component)]
pub struct Toqueable(pub u32);

#[derive(Debug, Clone, Copy)]
pub enum ToqueSource {
    Desktop,
    Vr,
}

#[derive(Debug, Clone, Copy)]
pub struct ToqueRawEvent {
    pub node_id: u32,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub source: ToqueSource,
}

/// Cola de eventos raw; el controller resource los normaliza a `toque`.
#[derive(Resource, Default)]
pub struct ToqueRawEvents(pub Vec<ToqueRawEvent>);

pub fn desktop_toque_raycast_system(
    mouse_button: Res<ButtonInput<MouseButton>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    camera_query: Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    toqueable_query: Query<(&GlobalTransform, &Toqueable)>,
    mut toque_events: ResMut<ToqueRawEvents>,
    mut log_panel: ResMut<LogPanel>,
) {
    if !mouse_button.just_pressed(MouseButton::Left) {
        return;
    }
    let Ok(window) = windows.get_single() else {
        return;
    };
    let Some(cursor_pos) = window.cursor_position() else {
        return;
    };
    let Ok((camera, camera_transform)) = camera_query.get_single() else {
        return;
    };

    let Some(ray) = camera.viewport_to_world(camera_transform, cursor_pos) else {
        return;
    };

    let mut closest: Option<(f32, u32, Vec3)> = None;

    for (global_transform, toqueable) in toqueable_query.iter() {
        let entity_pos = global_transform.translation();
        let scale = global_transform.to_scale_rotation_translation().0;
        let radius = scale.max_element() * 0.5;

        let oc = ray.origin - entity_pos;
        let a = ray.direction.dot(*ray.direction);
        let b = 2.0 * oc.dot(*ray.direction);
        let c = oc.dot(oc) - radius * radius;
        let discriminant = b * b - 4.0 * a * c;

        if discriminant >= 0.0_f32 {
            let t = (-b - discriminant.sqrt()) / (2.0 * a);
            if t > 0.0 {
                if closest.is_none() || t < closest.unwrap().0 {
                    let hit_point = ray.origin + *ray.direction * t;
                    closest = Some((t, toqueable.0, hit_point));
                }
            }
        }
    }

    if let Some((_, node_id, hit_point)) = closest {
        toque_events.0.push(ToqueRawEvent {
            node_id,
            x: hit_point.x,
            y: hit_point.y,
            z: hit_point.z,
            source: ToqueSource::Desktop,
        });
        log_panel.push_info(format!(
            "[toque-raw][desktop] HIT node={} at ({:.2}, {:.2}, {:.2})",
            node_id, hit_point.x, hit_point.y, hit_point.z
        ));
    }
}

pub fn vr_toque_raycast_system(
    actions: Res<crate::vr_locomotion::LunaLocomotionActions>,
    session: Res<bevy_mod_openxr::session::OxrSession>,
    controller_query: Query<&GlobalTransform, With<bevy_xr_utils::tracking_utils::XrTrackedRightGrip>>,
    toqueable_query: Query<(&GlobalTransform, &Toqueable)>,
    mut toque_events: ResMut<ToqueRawEvents>,
    mut log_panel: ResMut<LogPanel>,
    mut last_trigger: Local<bool>,
) {
    let Ok(state) = actions.right_trigger.state(&session, openxr::Path::NULL) else {
        return;
    };

    let pressed = state.current_state > 0.8;
    let just_pressed = pressed && !*last_trigger;
    *last_trigger = pressed;

    if !just_pressed {
        return;
    }

    let Ok(controller_tf) = controller_query.get_single() else {
        return;
    };

    let ray_origin = controller_tf.translation();
    let ray_dir = controller_tf.forward().as_vec3();

    let mut closest: Option<(f32, u32, Vec3)> = None;

    for (global_transform, toqueable) in toqueable_query.iter() {
        let entity_pos = global_transform.translation();
        let scale = global_transform.to_scale_rotation_translation().0;
        let radius = scale.max_element() * 0.5;
        let oc = ray_origin - entity_pos;
        let a = ray_dir.dot(ray_dir);
        let b = 2.0 * oc.dot(ray_dir);
        let c = oc.dot(oc) - radius * radius;
        let discriminant = b * b - 4.0 * a * c;

        if discriminant >= 0.0 {
            let t = (-b - discriminant.sqrt()) / (2.0 * a);
            if t > 0.0 && t < 20.0 {
                if closest.is_none() || t < closest.unwrap().0 {
                    let hit_point = ray_origin + ray_dir * t;
                    closest = Some((t, toqueable.0, hit_point));
                }
            }
        }
    }

    if let Some((_, node_id, hit_point)) = closest {
        toque_events.0.push(ToqueRawEvent {
            node_id,
            x: hit_point.x,
            y: hit_point.y,
            z: hit_point.z,
            source: ToqueSource::Vr,
        });
        log_panel.push_info(format!(
            "[toque-raw][vr] HIT node={} at ({:.2}, {:.2}, {:.2})",
            node_id, hit_point.x, hit_point.y, hit_point.z
        ));
    }
}

/// Enruta el raw input únicamente al worker del space dueño del nodo.
pub fn dispatch_toque_raw_events_to_js(
    mut toque_events: ResMut<ToqueRawEvents>,
    world: Res<ElemenetWorld>,
    mut manager: NonSendMut<ScriptRuntimeManager>,
    space_handle_tables: Res<SpaceHandleTables>,
    space_policies: Res<SpacePolicies>,
) {
    if toque_events.0.is_empty() {
        return;
    }

    let events: Vec<ToqueRawEvent> = toque_events.0.drain(..).collect();

    for evt in events {
        let ent = world.0.entities().entity(evt.node_id);
        if !world.0.entities().is_alive(ent) {
            continue;
        }

        let Some(space_id) = find_owner_space_id(&world.0, ent) else {
            continue;
        };

        if !space_has_capability(space_id, CapabilityBits::READ_TOQUE_RAW, &space_policies) {
            continue;
        }

        let Some(table) = space_handle_tables.by_space.get(&space_id) else {
            continue;
        };
        let Some(&local_id) = table.global_to_local.get(&evt.node_id) else {
            continue;
        };

        if let Some(worker) = manager.contexts.get(&space_id) {
            let _ = worker
                .cmd_tx
                .send(JsWorkerCommand::PushToqueRawEvents(vec![(
                    local_id, evt.x, evt.y, evt.z,
                )]));
        }
    }
}
