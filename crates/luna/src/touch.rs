use bevy::prelude::*;
use bevy::window::PrimaryWindow;

use crate::LogPanel;

/// Marker component added to Bevy entities that correspond to touchable primitives
/// (box, sphere, plane, cylinder). Stores the DOM node_id for reverse lookup.
#[derive(Component)]
pub struct Touchable(pub u32);

/// Queue of touch events to be dispatched to JS runtimes.
/// Each entry: (dom_node_id, hit_x, hit_y, hit_z)
#[derive(Resource, Default)]
pub struct TouchEvents(pub Vec<(u32, f32, f32, f32)>);

/// Desktop raycast picking system.
/// On left-click, casts a ray from the camera through the mouse position
/// and tests against all entities with the Touchable component.
pub fn desktop_raycast_system(
    mouse_button: Res<ButtonInput<MouseButton>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    camera_query: Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    touchable_query: Query<(&GlobalTransform, &Touchable)>,
    mut touch_events: ResMut<TouchEvents>,
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

    for (global_transform, touchable) in touchable_query.iter() {
        let entity_pos = global_transform.translation();
        let scale = global_transform.to_scale_rotation_translation().0;
        let radius = scale.max_element() * 0.5;

        // Ray-sphere intersection
        let oc = ray.origin - entity_pos;
        let a = ray.direction.dot(*ray.direction);
        let b = 2.0 * oc.dot(*ray.direction);
        let c = oc.dot(oc) - radius * radius;
        let discriminant = b * b - 4.0 * a * c;

        if discriminant >= 0.0 {
            let t = (-b - discriminant.sqrt()) / (2.0 * a);
            if t > 0.0 {
                if closest.is_none() || t < closest.unwrap().0 {
                    let hit_point = ray.origin + *ray.direction * t;
                    closest = Some((t, touchable.0, hit_point));
                }
            }
        }
    }

    if let Some((_, node_id, hit_point)) = closest {
        touch_events.0.push((node_id, hit_point.x, hit_point.y, hit_point.z));
        log_panel.push_info(format!(
            "[touch] HIT node={} at ({:.2}, {:.2}, {:.2})",
            node_id, hit_point.x, hit_point.y, hit_point.z
        ));
    }
}

/// VR controller raycast picking system.
/// When the right trigger exceeds the threshold, casts a ray from the right controller
/// and tests against all entities with the Touchable component.
pub fn vr_raycast_system(
    actions: Res<crate::vr_locomotion::LunaLocomotionActions>,
    session: Res<bevy_mod_openxr::session::OxrSession>,
    controller_query: Query<&GlobalTransform, With<bevy_xr_utils::tracking_utils::XrTrackedRightGrip>>,
    touchable_query: Query<(&GlobalTransform, &Touchable)>,
    mut touch_events: ResMut<TouchEvents>,
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

    for (global_transform, touchable) in touchable_query.iter() {
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
                    closest = Some((t, touchable.0, hit_point));
                }
            }
        }
    }

    if let Some((_, node_id, hit_point)) = closest {
        touch_events.0.push((node_id, hit_point.x, hit_point.y, hit_point.z));
        log_panel.push_info(format!(
            "[vr-touch] HIT node={} at ({:.2}, {:.2}, {:.2})",
            node_id, hit_point.x, hit_point.y, hit_point.z
        ));
    }
}
