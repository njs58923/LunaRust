use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use specs::WorldExt;
use std::collections::HashMap;
use crate::js::{find_owner_space_id, JsWorkerCommand, ScriptRuntimeManager};
use crate::permissions::{space_has_capability, CapabilityBits, SpacePolicies};
use crate::{ElemenetWorld, LogPanel, SpaceHandleTables};

/// Marker component for primitives that can receive a normalized `toque`.
#[derive(Component)]
pub struct Toqueable(pub u32);

/// Marker component for invisible/controller-tracked pose volumes.
#[derive(Component)]
pub struct PoseZone(pub u32);


/// Exact collision shape used by the toque raycast.
///
/// Sphere:  ray–sphere using `max(sx,sy,sz) * 0.5` as radius.
/// Box:     ray–OBB using the full non-uniform scale and world rotation.
/// Plane:   ray–rectangle in the local XY plane of the entity (Z = 0, normal +Z).
#[derive(Component, Clone, Copy, Debug)]
pub enum HitShape {
    Sphere,
    Box,
    Plane,
}

/// Returns `(t, hit_point)` if `ray` intersects the given shape, else `None`.
/// `t` is the ray parameter at the entry point (>= 0).
fn intersect_shape(
    ray_origin: Vec3,
    ray_dir: Vec3,
    entity_pos: Vec3,
    rotation: Quat,
    scale: Vec3,
    shape: HitShape,
) -> Option<(f32, Vec3)> {
    match shape {
        HitShape::Sphere => {
            let radius = scale.max_element() * 0.5;
            let oc = ray_origin - entity_pos;
            let a = ray_dir.dot(ray_dir);
            let b = 2.0 * oc.dot(ray_dir);
            let c = oc.dot(oc) - radius * radius;
            let discriminant = b * b - 4.0 * a * c;
            if discriminant < 0.0 {
                return None;
            }
            let t = (-b - discriminant.sqrt()) / (2.0 * a);
            if t <= 0.0 {
                return None;
            }
            Some((t, ray_origin + ray_dir * t))
        }
        HitShape::Box => {
            // Transform ray into the box's local (oriented, unit-cube) space.
            let inv_rot = rotation.inverse();
            let lo = inv_rot * (ray_origin - entity_pos);
            let ld = inv_rot * ray_dir;
            let half = scale * 0.5;

            // Slab method with safe divisions.
            let safe = |v: f32| if v.abs() > 1e-8 { v } else { 1e-8 };
            let inv = Vec3::new(1.0 / safe(ld.x), 1.0 / safe(ld.y), 1.0 / safe(ld.z));

            let t1 = (-half - lo) * inv;
            let t2 = (half - lo) * inv;
            let tmin = t1.min(t2);
            let tmax = t1.max(t2);
            let t_enter = tmin.x.max(tmin.y).max(tmin.z);
            let t_exit = tmax.x.min(tmax.y).min(tmax.z);

            if t_exit < t_enter.max(0.0) {
                return None;
            }
            let t = t_enter.max(0.0);
            if t <= 0.0 {
                return None;
            }
            Some((t, ray_origin + ray_dir * t))
        }
        HitShape::Plane => {
            // Plane mesh lives in local XY (z = 0) with normal +Z.
            let normal = (rotation * Vec3::Z).normalize_or_zero();
            if normal == Vec3::ZERO {
                return None;
            }
            let denom = normal.dot(ray_dir);
            if denom.abs() < 1e-6 {
                return None; // ray parallel to plane
            }
            let t = (entity_pos - ray_origin).dot(normal) / denom;
            if t <= 0.0 {
                return None;
            }
            let hit_world = ray_origin + ray_dir * t;
            // Back-transform to local coords to test the rectangle bounds.
            let inv_rot = rotation.inverse();
            let local_hit = inv_rot * (hit_world - entity_pos);
            let half_x = scale.x * 0.5;
            let half_y = scale.y * 0.5;
            if local_hit.x.abs() <= half_x && local_hit.y.abs() <= half_y {
                Some((t, hit_world))
            } else {
                None
            }
        }
    }
}

fn contains_point(
    point: Vec3,
    entity_pos: Vec3,
    rotation: Quat,
    scale: Vec3,
    shape: HitShape,
) -> bool {
    match shape {
        HitShape::Sphere => {
            let radius = scale.max_element() * 0.5;
            point.distance_squared(entity_pos) <= radius * radius
        }
        HitShape::Box => {
            let inv_rot = rotation.inverse();
            let local = inv_rot * (point - entity_pos);
            let half = scale * 0.5;
            local.x.abs() <= half.x && local.y.abs() <= half.y && local.z.abs() <= half.z
        }
        HitShape::Plane => {
            let inv_rot = rotation.inverse();
            let local = inv_rot * (point - entity_pos);
            let half_x = scale.x * 0.5;
            let half_y = scale.y * 0.5;
            let half_z = (scale.z * 0.5).max(0.02);
            local.x.abs() <= half_x
                && local.y.abs() <= half_y
                && local.z.abs() <= half_z
        }
    }
}


#[derive(Debug, Clone, Copy)]
pub enum ToqueSource {
    Desktop,
    Vr,
}

#[derive(Debug, Clone, Copy)]
pub struct HostToqueHit {
    pub node_id: u32,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub source: ToqueSource,
}

/// Hits detectados por el host; alimentan el evento DOM `toque`
/// y opcionalmente el canal raw privilegiado.
#[derive(Resource, Default)]
pub struct HostToqueHits(pub Vec<HostToqueHit>);

#[derive(Debug, Clone)]
pub struct HostPoseMoveHit {
    pub node_id: u32,
    pub hand: String,
    pub px: f32,
    pub py: f32,
    pub pz: f32,
    pub dx: f32,
    pub dy: f32,
    pub dz: f32,
    pub trigger: f32,
    pub grip: f32,
    // Orientación del controlador (cuaternión world-space).
    pub qx: f32,
    pub qy: f32,
    pub qz: f32,
    pub qw: f32,
}

/// posemove detectado por el host; se despacha como DOM event si el space tiene permiso.
#[derive(Resource, Default)]
pub struct HostPoseMoveEvents(pub Vec<HostPoseMoveHit>);


pub fn desktop_toque_raycast_system(
    mouse_button: Res<ButtonInput<MouseButton>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    camera_query: Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    // InheritedVisibility refleja la cadena Visibility::Inherited/Hidden/Visible
    // propagada por Bevy en PostUpdate. Si un ancestro está Hidden, todos los
    // descendientes (incluso con Visible explícito) leen `iv.get() == false`.
    // Filtramos acá para que un panel oculto no reciba clicks de sus hijos.
    toqueable_query: Query<(&GlobalTransform, &Toqueable, &InheritedVisibility, Option<&HitShape>)>,
    mut toque_hits: ResMut<HostToqueHits>,
    mut log_panel: ResMut<LogPanel>,
    shooter: Option<Res<crate::desktop_locomotion::DesktopShooterActive>>,
) {
    if !mouse_button.just_pressed(MouseButton::Left) {
        return;
    }
    let Ok(window) = windows.get_single() else {
        return;
    };
    let shooter_active = shooter.map(|s| s.0).unwrap_or(false);
    let cursor_pos = if shooter_active {
        // Shooter mode: raycast from center of window
        Vec2::new(window.width() / 2.0, window.height() / 2.0)
    } else {
        let Some(pos) = window.cursor_position() else {
            return;
        };
        pos
    };
    let Ok((camera, camera_transform)) = camera_query.get_single() else {
        return;
    };

    let Some(ray) = camera.viewport_to_world(camera_transform, cursor_pos) else {
        return;
    };

    let mut closest: Option<(f32, u32, Vec3)> = None;

    for (global_transform, toqueable, inherited_vis, hit_shape) in toqueable_query.iter() {
        if !inherited_vis.get() {
            continue;
        }
        let (scale, rotation, entity_pos) = global_transform.to_scale_rotation_translation();
        let shape = hit_shape.copied().unwrap_or(HitShape::Sphere);

        if let Some((t, hit_point)) = intersect_shape(
            ray.origin,
            *ray.direction,
            entity_pos,
            rotation,
            scale,
            shape,
        ) {
            if closest.is_none() || t < closest.unwrap().0 {
                closest = Some((t, toqueable.0, hit_point));
            }
        }
    }

    if let Some((_, node_id, hit_point)) = closest {
        toque_hits.0.push(HostToqueHit {
            node_id,
            x: hit_point.x,
            y: hit_point.y,
            z: hit_point.z,
            source: ToqueSource::Desktop,
        });
        log_panel.push_info(format!(
            "[toque-host][desktop] HIT node={} at ({:.2}, {:.2}, {:.2})",
            node_id, hit_point.x, hit_point.y, hit_point.z
        ));
    }
}

pub fn vr_toque_raycast_system(
    actions: Res<crate::vr_locomotion::LunaLocomotionActions>,
    session: Res<bevy_mod_openxr::session::OxrSession>,
    controller_query: Query<&GlobalTransform, With<bevy_xr_utils::tracking_utils::XrTrackedRightGrip>>,
    // Ver doc en desktop_toque_raycast_system para el filtro de InheritedVisibility.
    toqueable_query: Query<(&GlobalTransform, &Toqueable, &InheritedVisibility, Option<&HitShape>)>,
    mut toque_hits: ResMut<HostToqueHits>,
    mut log_panel: ResMut<LogPanel>,
    mut last_trigger: Local<bool>,
) {
    let Ok(state) = actions.right_trigger.state(&session, openxr::Path::NULL) else {
        log_panel.push_warn("[vr_toque] right_trigger.state failed");
        return;
    };

    let pressed = state.current_state > 0.8;
    let just_pressed = pressed && !*last_trigger;
    *last_trigger = pressed;

    if !just_pressed {
        return;
    }

    let controller_tf = match controller_query.get_single() {
        Ok(tf) => tf,
        Err(e) => {
            log_panel.push_warn(format!("[vr_toque] controller query failed: {:?}", e));
            return;
        }
    };

    let ray_origin = controller_tf.translation();
    let ray_dir = -controller_tf.up().as_vec3();

    let mut closest: Option<(f32, u32, Vec3)> = None;

    for (global_transform, toqueable, inherited_vis, hit_shape) in toqueable_query.iter() {
        if !inherited_vis.get() {
            continue;
        }
        let (scale, rotation, entity_pos) = global_transform.to_scale_rotation_translation();
        let shape = hit_shape.copied().unwrap_or(HitShape::Sphere);

        if let Some((t, hit_point)) =
            intersect_shape(ray_origin, ray_dir, entity_pos, rotation, scale, shape)
        {
            if t < 20.0 && (closest.is_none() || t < closest.unwrap().0) {
                closest = Some((t, toqueable.0, hit_point));
            }
        }
    }

    if let Some((_, node_id, hit_point)) = closest {
        toque_hits.0.push(HostToqueHit {
            node_id,
            x: hit_point.x,
            y: hit_point.y,
            z: hit_point.z,
            source: ToqueSource::Vr,
        });
        log_panel.push_info(format!(
            "[toque-host][vr] HIT node={} at ({:.2}, {:.2}, {:.2})",
            node_id, hit_point.x, hit_point.y, hit_point.z
        ));
    }
}
fn push_pose_events_for_hand(
    hand: &str,
    controller_tf: &GlobalTransform,
    trigger: f32,
    grip: f32,
    posezone_query: &Query<(&GlobalTransform, &PoseZone, &InheritedVisibility, Option<&HitShape>)>,
    pose_events: &mut HostPoseMoveEvents,
) {
    let point = controller_tf.translation();
    let dir = (-controller_tf.up().as_vec3()).normalize_or_zero();
    if dir == Vec3::ZERO {
        return;
    }
    let (_, controller_rot, _) = controller_tf.to_scale_rotation_translation();

    for (global_transform, posezone, inherited_vis, hit_shape) in posezone_query.iter() {
        if !inherited_vis.get() {
            continue;
        }
        let (scale, rotation, entity_pos) = global_transform.to_scale_rotation_translation();
        let shape = hit_shape.copied().unwrap_or(HitShape::Box);

        if contains_point(point, entity_pos, rotation, scale, shape) {
            pose_events.0.push(HostPoseMoveHit {
                node_id: posezone.0,
                hand: hand.to_string(),
                px: point.x,
                py: point.y,
                pz: point.z,
                dx: dir.x,
                dy: dir.y,
                dz: dir.z,
                trigger,
                grip,
                qx: controller_rot.x,
                qy: controller_rot.y,
                qz: controller_rot.z,
                qw: controller_rot.w,
            });
        }
    }
}

pub fn vr_posemove_system(
    actions: Res<crate::vr_locomotion::LunaLocomotionActions>,
    session: Res<bevy_mod_openxr::session::OxrSession>,
    left_controller_query: Query<
        &GlobalTransform,
        With<bevy_xr_utils::tracking_utils::XrTrackedLeftGrip>,
    >,
    right_controller_query: Query<
        &GlobalTransform,
        With<bevy_xr_utils::tracking_utils::XrTrackedRightGrip>,
    >,
    posezone_query: Query<(&GlobalTransform, &PoseZone, &InheritedVisibility, Option<&HitShape>)>,
    mut pose_events: ResMut<HostPoseMoveEvents>,
) {
    if posezone_query.is_empty() {
        return;
    }

    let left_trigger = actions
        .left_trigger
        .state(&session, openxr::Path::NULL)
        .map(|s| s.current_state)
        .unwrap_or(0.0);
    let right_trigger = actions
        .right_trigger
        .state(&session, openxr::Path::NULL)
        .map(|s| s.current_state)
        .unwrap_or(0.0);
    let left_grip = actions
        .left_grip
        .state(&session, openxr::Path::NULL)
        .map(|s| s.current_state)
        .unwrap_or(0.0);
    let right_grip = actions
        .right_grip
        .state(&session, openxr::Path::NULL)
        .map(|s| s.current_state)
        .unwrap_or(0.0);

    if let Ok(tf) = left_controller_query.get_single() {
        push_pose_events_for_hand("left", tf, left_trigger, left_grip, &posezone_query, &mut pose_events);
    }
    if let Ok(tf) = right_controller_query.get_single() {
        push_pose_events_for_hand("right", tf, right_trigger, right_grip, &posezone_query, &mut pose_events);
    }
}


/// Enruta `toque` DOM siempre al owner del target y raw solo si el space tiene READ_TOQUE_RAW.
pub fn dispatch_toque_events_to_js(
    mut toque_hits: ResMut<HostToqueHits>,
    world: Res<ElemenetWorld>,
    mut manager: NonSendMut<ScriptRuntimeManager>,
    space_handle_tables: Res<SpaceHandleTables>,
    space_policies: Res<SpacePolicies>,
) {
    if toque_hits.0.is_empty() {
        return;
    }

    let events: Vec<HostToqueHit> = toque_hits.0.drain(..).collect();

    for evt in events {
        let ent = world.0.entities().entity(evt.node_id);
        if !world.0.entities().is_alive(ent) {
            continue;
        }

        let Some(space_id) = find_owner_space_id(&world.0, ent) else {
            continue;
        };

        let Some(table) = space_handle_tables.by_space.get(&space_id) else {
            continue;
        };
        let Some(&local_id) = table.global_to_local.get(&evt.node_id) else {
            continue;
        };

        let allow_raw =
            space_has_capability(space_id, CapabilityBits::READ_TOQUE_RAW, &space_policies);

        if let Some(worker) = manager.contexts.get(&space_id) {
            let _ = worker
                .cmd_tx
                .send(JsWorkerCommand::PushDomToqueEvents(vec![(
                    local_id, evt.x, evt.y, evt.z,
                )]));

            if allow_raw {
                let _ = worker
                    .cmd_tx
                    .send(JsWorkerCommand::PushToqueRawEvents(vec![(
                        local_id, evt.x, evt.y, evt.z,
                    )]));
            }
        }
    }
}



/// Enruta `posemove` solo al owner del posezone y solo si el space tiene READ_POSE_STREAM.
pub fn dispatch_posemove_events_to_js(
    mut pose_events: ResMut<HostPoseMoveEvents>,
    world: Res<ElemenetWorld>,
    mut manager: NonSendMut<ScriptRuntimeManager>,
    space_handle_tables: Res<SpaceHandleTables>,
    space_policies: Res<SpacePolicies>,
) {
    if pose_events.0.is_empty() {
        return;
    }

    let events: Vec<HostPoseMoveHit> = pose_events.0.drain(..).collect();
    let mut per_space: HashMap<u32, Vec<crate::js::PoseMoveEventData>> = HashMap::new();

    for evt in events {
        let ent = world.0.entities().entity(evt.node_id);
        if !world.0.entities().is_alive(ent) {
            continue;
        }

        let Some(space_id) = find_owner_space_id(&world.0, ent) else {
            continue;
        };

        if !space_has_capability(space_id, CapabilityBits::READ_POSE_STREAM, &space_policies) {
            continue;
        }

        let Some(table) = space_handle_tables.by_space.get(&space_id) else {
            continue;
        };
        let Some(&local_id) = table.global_to_local.get(&evt.node_id) else {
            continue;
        };

        per_space
            .entry(space_id)
            .or_default()
            .push(crate::js::PoseMoveEventData {
                node_id: local_id,
                hand: evt.hand,
                px: evt.px,
                py: evt.py,
                pz: evt.pz,
                dx: evt.dx,
                dy: evt.dy,
                dz: evt.dz,
                trigger: evt.trigger,
                grip: evt.grip,
                qx: evt.qx,
                qy: evt.qy,
                qz: evt.qz,
                qw: evt.qw,
            });
    }

    for (space_id, batch) in per_space {
        if let Some(worker) = manager.contexts.get_mut(&space_id) {
            let _ = worker.cmd_tx.send(JsWorkerCommand::PushPoseMoveEvents(batch));
        }
    }
}