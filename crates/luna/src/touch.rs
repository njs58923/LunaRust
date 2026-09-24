use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use specs::WorldExt;
use std::collections::HashMap;
use crate::js::{find_owner_space_id, JsWorkerCommand, ScriptRuntimeManager};
use crate::permissions::{space_has_capability, CapabilityBits, SpacePolicies};
use crate::{ElemenetWorld, LogPanel, SpaceHandleTables};

/// Marker component for primitives that can receive a normalized `toque`.
#[derive(Component, Clone, Copy)]
pub struct Toqueable(pub u32, pub bool);
// The second field enables event delivery. False is a native-only blocker.


type PointerHit = (f32, Option<u32>, Vec3, GlobalTransform);
fn consider_pointer_hit(closest: &mut Option<PointerHit>, distance: f32, target: &Toqueable, point: Vec3, transform: &GlobalTransform) {
    if closest.as_ref().is_none_or(|hit| distance < hit.0) {
        *closest = Some((distance, target.1.then_some(target.0), point, *transform));
    }
}

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

/// Derived once per changed target, shared by the desktop and both VR rays.
#[derive(Component, Clone, Copy)]
pub struct PreparedPointerShape {
    scale: Vec3,
    rotation: Quat,
    position: Vec3,
    shape: HitShape,
}

pub fn prepare_pointer_shapes(
    mut commands: Commands,
    mut targets: Query<(Entity, &GlobalTransform, Option<&HitShape>, Option<&mut PreparedPointerShape>),
        (With<Toqueable>, Or<(Changed<GlobalTransform>, Changed<HitShape>, Changed<Toqueable>)>)>,
) {
    for (entity, transform, shape, cached) in &mut targets {
        let (scale, rotation, position) = transform.to_scale_rotation_translation();
        let prepared = PreparedPointerShape { scale, rotation, position,
            shape: shape.copied().unwrap_or(HitShape::Sphere) };
        if let Some(mut cached) = cached { *cached = prepared; }
        else { commands.entity(entity).insert(prepared); }
    }
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
    let scale = scale.abs();
    if !ray_origin.is_finite() || !ray_dir.is_finite() || !entity_pos.is_finite()
        || !scale.is_finite() || !rotation.is_finite() || ray_dir.length_squared() == 0.0 {
        return None;
    }
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
            let root = discriminant.sqrt();
            let entry = (-b - root) / (2.0 * a);
            let t = if entry > 0.0 { entry } else { (-b + root) / (2.0 * a) };
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

            // Parallel axes constrain membership without invented directions.
            // This also handles rays exactly on a face without 0 * infinity.
            let mut t_enter = f32::NEG_INFINITY;
            let mut t_exit = f32::INFINITY;
            for axis in 0..3 {
                if ld[axis] == 0.0 {
                    if lo[axis].abs() > half[axis] { return None; }
                    continue;
                }
                let t1 = (-half[axis] - lo[axis]) / ld[axis];
                let t2 = (half[axis] - lo[axis]) / ld[axis];
                t_enter = t_enter.max(t1.min(t2));
                t_exit = t_exit.min(t1.max(t2));
                if t_enter > t_exit { return None; }
            }
            let t = if t_enter > 0.0 { t_enter } else { t_exit };
            if t <= 0.0 || !t.is_finite() { return None; }
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
    let scale = scale.abs();
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

fn ancestors_are_visible(
    entity: Entity,
    parent_query: &Query<&Parent>,
    visibility_query: &Query<&Visibility>,
) -> bool {
    let mut current = entity;
    while let Ok(parent) = parent_query.get(current) {
        let parent_entity = parent.get();
        if matches!(visibility_query.get(parent_entity), Ok(Visibility::Hidden)) {
            return false;
        }
        current = parent_entity;
    }
    true
}

fn compose_tracking_pose(root_tf: &Transform, local_tf: &Transform) -> (Vec3, Quat) {
    let scaled_local = root_tf.scale * local_tf.translation;
    (
        root_tf.translation + root_tf.rotation * scaled_local,
        root_tf.rotation * local_tf.rotation,
    )
}

fn controller_aim_direction(rotation: Quat) -> Vec3 {
    controller_ui_ray_direction(rotation)
}

fn controller_ui_ray_direction(rotation: Quat) -> Vec3 {
    -(rotation * Vec3::Y).normalize_or_zero()
}

#[derive(Debug, Clone, Copy)]
pub enum ToqueSource {
    Desktop,
    Vr,
}

#[derive(Debug, Clone, Copy)]
pub struct HostToqueHit {
    pub local:[f32;3],
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
    // La misma pose en el marco del padre del posezone: las coordenadas de
    // quien lo puso. Un include no puede leer la transformación de quien lo
    // aloja, así que sin esto no hay forma de saber dónde quedó la mano
    // respecto de sus piezas.
    pub local: [f32; 3],
    pub local_dir: [f32; 3],
    pub local_rot: [f32; 4],
}

/// posemove detectado por el host; se despacha como DOM event si el space tiene permiso.
#[derive(Resource, Default)]
pub struct HostPoseMoveEvents(pub Vec<HostPoseMoveHit>);


/// Latest targets are consumed each frame, so disabled input clears hover too.
#[derive(Resource, Default)]
pub struct HostHoverTargets(pub [Option<u32>; 3]);

pub fn dispatch_hover_events_to_js(
    mut targets: ResMut<HostHoverTargets>,
    world: Res<ElemenetWorld>,
    tables: Res<SpaceHandleTables>,
    mut manager: NonSendMut<ScriptRuntimeManager>,
    mut sent: Local<HashMap<u32, [Option<i32>; 3]>>,
    mut generation: Local<u64>,
    mut desired: Local<HashMap<u32, [Option<i32>; 3]>>,
    mut spaces: Local<Vec<u32>>,
) {
    let contexts_changed = *generation != manager.context_generation;
    *generation = manager.context_generation;
    desired.clear();
    for (pointer, target) in std::mem::take(&mut targets.0).into_iter().enumerate() {
        let Some(id) = target else { continue };
        let entity = world.0.entities().entity(id);
        if !world.0.entities().is_alive(entity) { continue; }
        let Some(owner) = find_owner_space_id(&world.0, entity) else { continue };
        let Some(local) = tables.by_space.get(&owner).and_then(|t| t.global_to_local.get(&id)) else { continue };
        desired.entry(owner).or_default()[pointer] = Some(*local);
    }
    // At most three active spaces, plus spaces needing a leave/retry.
    spaces.clear();
    spaces.extend(desired.keys().copied());
    spaces.extend(sent.keys().filter(|id| !desired.contains_key(id)).copied());
    for space in spaces.iter().copied() {
        let Some(worker) = manager.contexts.get_mut(&space) else { sent.remove(&space); continue };
        let next = desired.get(&space).copied().unwrap_or_default();
        if !contexts_changed && sent.get(&space).copied().unwrap_or_default() == next { continue; }
        // Retry unchanged targets if the worker queue was full. Never lose a leave.
        if worker.try_send(JsWorkerCommand::SetHoverTargets(next)).is_ok() {
            if next == [None; 3] { sent.remove(&space); } else { sent.insert(space, next); }
            worker.needs_tick = true;
        }
    }
}

pub fn desktop_toque_raycast_system(
    mouse_button: Res<ButtonInput<MouseButton>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    camera_query: Query<(&Camera, &GlobalTransform), With<crate::DesktopCamera>>,
    // InheritedVisibility refleja la cadena Visibility::Inherited/Hidden/Visible
    // propagada por Bevy en PostUpdate. Si un ancestro está Hidden, todos los
    // descendientes (incluso con Visible explícito) leen `iv.get() == false`.
    // Filtramos acá para que un panel oculto no reciba clicks de sus hijos.
    toqueable_query: Query<(&GlobalTransform, &Toqueable, &InheritedVisibility, &PreparedPointerShape)>,
    mut toque_hits: ResMut<HostToqueHits>,
    mut hover: ResMut<HostHoverTargets>,
    mut log_panel: ResMut<LogPanel>,
    shooter: Option<Res<crate::desktop_locomotion::DesktopShooterActive>>,
) {
    hover.0[0] = None;
    let Ok(window) = windows.get_single() else {
        return;
    };
    if !window.focused { return; }
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

    if !camera.is_active { return; }
    let Some(ray) = camera.viewport_to_world(camera_transform, cursor_pos) else {
        return;
    };

    let mut closest: Option<PointerHit> = None;

    for (global_transform, toqueable, inherited_vis, hit_shape) in toqueable_query.iter() {
        if !inherited_vis.get() {
            continue;
        }
        let PreparedPointerShape { scale, rotation, position: entity_pos, shape } = *hit_shape;

        if let Some((t, hit_point)) = intersect_shape(
            ray.origin,
            *ray.direction,
            entity_pos,
            rotation,
            scale,
            shape,
        ) {
            consider_pointer_hit(&mut closest, t, toqueable, hit_point, global_transform);
        }
    }

    hover.0[0] = closest.and_then(|(_, id, _, _)| id);
    if !mouse_button.just_pressed(MouseButton::Left) { return; }
    if let Some((_, Some(node_id), hit_point, local)) = closest {
        let local = local.affine().inverse().transform_point3(hit_point);
        toque_hits.0.push(HostToqueHit {
            local:local.to_array(),
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
    controller_query: Query<&Transform, With<bevy_xr_utils::tracking_utils::XrTrackedRightGrip>>,
    tracking_root: Query<
        &Transform,
        (
            With<bevy_mod_xr::session::XrTrackingRoot>,
            Without<bevy_xr_utils::tracking_utils::XrTrackedRightGrip>,
        ),
    >,
    // Ver doc en desktop_toque_raycast_system para el filtro de InheritedVisibility.
    toqueable_query: Query<(&GlobalTransform, &Toqueable, &InheritedVisibility, &PreparedPointerShape)>,
    mut toque_hits: ResMut<HostToqueHits>,
    mut hover: ResMut<HostHoverTargets>,
    mut log_panel: ResMut<LogPanel>,
    mut last_trigger: Local<bool>,
) {
    let Ok(state) = actions.right_trigger.state(&session, openxr::Path::NULL) else {
        log_panel.push_warn("[vr_toque] right_trigger.state failed");
        return;
    };

    if !state.is_active { *last_trigger = false; return; }
    let pressed = state.current_state > 0.8;
    let just_pressed = pressed && !*last_trigger;
    *last_trigger = pressed;



    let controller_tf = match controller_query.get_single() {
        Ok(tf) => tf,
        Err(e) => {
            log_panel.push_warn(format!("[vr_toque] controller query failed: {:?}", e));
            return;
        }
    };
    let Ok(root_tf) = tracking_root.get_single() else {
        log_panel.push_warn("[vr_toque] tracking root query failed");
        return;
    };

    let (ray_origin, controller_rot) = compose_tracking_pose(root_tf, controller_tf);
    let ray_dir = controller_ui_ray_direction(controller_rot);
    if ray_dir == Vec3::ZERO {
        return;
    }

    let mut closest: Option<PointerHit> = None;

    for (global_transform, toqueable, inherited_vis, hit_shape) in toqueable_query.iter() {
        if !inherited_vis.get() {
            continue;
        }
        let PreparedPointerShape { scale, rotation, position: entity_pos, shape } = *hit_shape;

        if let Some((t, hit_point)) =
            intersect_shape(ray_origin, ray_dir, entity_pos, rotation, scale, shape)
        {
            if t < 20.0 {
                consider_pointer_hit(&mut closest, t, toqueable, hit_point, global_transform);
            }
        }
    }

    hover.0[2] = closest.and_then(|(_, id, _, _)| id);
    if !just_pressed { return; }
    if let Some((_, Some(node_id), hit_point, local)) = closest {
        let local = local.affine().inverse().transform_point3(hit_point);
        toque_hits.0.push(HostToqueHit {
            local:local.to_array(),
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
/// Secondary VR ray shares the same hit rules as the primary controller.
pub fn vr_left_hover_raycast_system(
    actions: Res<crate::vr_locomotion::LunaLocomotionActions>,
    session: Res<bevy_mod_openxr::session::OxrSession>,
    controller: Query<&Transform, With<bevy_xr_utils::tracking_utils::XrTrackedLeftGrip>>,
    root: Query<&Transform, (With<bevy_mod_xr::session::XrTrackingRoot>, Without<bevy_xr_utils::tracking_utils::XrTrackedLeftGrip>)>,
    shapes: Query<(&GlobalTransform, &Toqueable, &InheritedVisibility, &PreparedPointerShape)>,
    mut hover: ResMut<HostHoverTargets>,
) {
    if !actions.left_trigger.state(&session, openxr::Path::NULL).map(|state| state.is_active).unwrap_or(false) { return; }
    let (Ok(controller), Ok(root)) = (controller.get_single(), root.get_single()) else { return };
    let (origin, rotation) = compose_tracking_pose(root, controller);
    let direction = controller_ui_ray_direction(rotation);
    if direction == Vec3::ZERO { return; }
    let mut nearest = None;
    for (transform, target, visible, shape) in &shapes {
        if !visible.get() { continue; }
        let PreparedPointerShape { scale, rotation, position, shape } = *shape;
        if let Some((distance, point)) = intersect_shape(origin, direction, position, rotation, scale, shape) {
            if distance < 20.0 { consider_pointer_hit(&mut nearest, distance, target, point, transform); }
        }
    }
    hover.0[1] = nearest.and_then(|(_, id, _, _)| id);
}

fn push_pose_events_for_hand(
    hand: &str,
    root_tf: &Transform,
    controller_tf: &Transform,
    trigger: f32,
    grip: f32,
    posezone_query: &Query<(Entity, &GlobalTransform, &PoseZone, Option<&HitShape>)>,
    parent_query: &Query<&Parent>,
    visibility_query: &Query<&Visibility>,
    global_query: &Query<&GlobalTransform>,
    pose_events: &mut HostPoseMoveEvents,
) {
    let (point, controller_rot) = compose_tracking_pose(root_tf, controller_tf);
    let dir = controller_aim_direction(controller_rot);
    if dir == Vec3::ZERO {
        return;
    }

    for (entity, global_transform, posezone, hit_shape) in posezone_query.iter() {
        if !ancestors_are_visible(entity, parent_query, visibility_query) {
            continue;
        }
        let (scale, rotation, entity_pos) = global_transform.to_scale_rotation_translation();
        let shape = hit_shape.copied().unwrap_or(HitShape::Box);

        if contains_point(point, entity_pos, rotation, scale, shape) {
            let (local, local_dir, local_rot) =
                pose_in_parent_frame(entity, point, dir, controller_rot, parent_query, global_query);
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
                local,
                local_dir,
                local_rot,
            });
        }
    }
}

/// La pose del mando en el marco del padre del posezone (escala incluida:
/// un objeto agrandado al doble ve la mano a la mitad de distancia). En JS
/// llega como `localX..localZ` (posición), `ldx..ldz` (hacia dónde apunta;
/// su largo es 1/escala) y `lqx..lqw` (giro). Sin padre, el mundo.
fn pose_in_parent_frame(
    entity: Entity,
    point: Vec3,
    dir: Vec3,
    rot: Quat,
    parent_query: &Query<&Parent>,
    global_query: &Query<&GlobalTransform>,
) -> ([f32; 3], [f32; 3], [f32; 4]) {
    let parent_gt = parent_query
        .get(entity)
        .ok()
        .and_then(|p| global_query.get(p.get()).ok());
    let Some(gt) = parent_gt else {
        return (point.to_array(), dir.to_array(), rot.to_array());
    };
    let inv = gt.affine().inverse();
    let (_, parent_rot, _) = gt.to_scale_rotation_translation();
    let local = inv.transform_point3(point);
    // Sin normalizar: con escala uniforme su largo es 1/escala del padre, y
    // así quien lo recibe puede rearmar el marco entero de un solo evento.
    let local_dir = inv.transform_vector3(dir);
    let local_rot = (parent_rot.inverse() * rot).normalize();
    if !local.is_finite() || !local_dir.is_finite() || !local_rot.is_finite() {
        return (point.to_array(), dir.to_array(), rot.to_array());
    }
    (local.to_array(), local_dir.to_array(), local_rot.to_array())
}

pub fn vr_posemove_system(
    actions: Res<crate::vr_locomotion::LunaLocomotionActions>,
    session: Res<bevy_mod_openxr::session::OxrSession>,
    left_controller_query: Query<
        &Transform,
        With<bevy_xr_utils::tracking_utils::XrTrackedLeftGrip>,
    >,
    right_controller_query: Query<
        &Transform,
        With<bevy_xr_utils::tracking_utils::XrTrackedRightGrip>,
    >,
    tracking_root: Query<
        &Transform,
        (
            With<bevy_mod_xr::session::XrTrackingRoot>,
            Without<bevy_xr_utils::tracking_utils::XrTrackedLeftGrip>,
            Without<bevy_xr_utils::tracking_utils::XrTrackedRightGrip>,
        ),
    >,
    posezone_query: Query<(Entity, &GlobalTransform, &PoseZone, Option<&HitShape>)>,
    parent_query: Query<&Parent>,
    visibility_query: Query<&Visibility>,
    global_query: Query<&GlobalTransform>,
    mut pose_events: ResMut<HostPoseMoveEvents>,
) {
    if posezone_query.is_empty() {
        return;
    }
    let Ok(root_tf) = tracking_root.get_single() else {
        return;
    };

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
        push_pose_events_for_hand(
            "left",
            root_tf,
            tf,
            left_trigger,
            left_grip,
            &posezone_query,
            &parent_query,
            &visibility_query,
            &global_query,
            &mut pose_events,
        );
    }
    if let Ok(tf) = right_controller_query.get_single() {
        push_pose_events_for_hand(
            "right",
            root_tf,
            tf,
            right_trigger,
            right_grip,
            &posezone_query,
            &parent_query,
            &visibility_query,
            &global_query,
            &mut pose_events,
        );
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

    let mut dom_by_space: HashMap<u32, Vec<(i32, f32, f32, f32,[f32;3])>> = HashMap::new();
    let mut raw_by_space: HashMap<u32, Vec<(i32, f32, f32, f32)>> = HashMap::new();

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

        dom_by_space
            .entry(space_id)
            .or_default()
            .push((local_id, evt.x, evt.y, evt.z,evt.local));
        if allow_raw {
            raw_by_space
                .entry(space_id)
                .or_default()
                .push((local_id, evt.x, evt.y, evt.z));
        }
    }

    for (space_id, batch) in dom_by_space {
        if let Some(worker) = manager.contexts.get_mut(&space_id) {
            let send_result = worker.try_send(JsWorkerCommand::PushLocalToqueEvents(batch));
            if send_result.is_ok() {
                worker.needs_tick = true;
            }
        }
    }
    for (space_id, batch) in raw_by_space {
        if let Some(worker) = manager.contexts.get_mut(&space_id) {
            let send_result = worker.try_send(JsWorkerCommand::PushToqueRawEvents(batch));
            if send_result.is_ok() {
                worker.needs_tick = true;
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
                local: evt.local,
                local_dir: evt.local_dir,
                local_rot: evt.local_rot,
            });
    }

    for (space_id, batch) in per_space {
        if let Some(worker) = manager.contexts.get_mut(&space_id) {
            let send_result = worker.try_send(JsWorkerCommand::PushPoseMoveEvents(batch));
            if send_result.is_ok() {
                worker.needs_tick = true;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn pointer_shapes_support_mirroring_parallel_rays_and_inside_origins() {
        use super::*;
        for shape in [HitShape::Plane, HitShape::Box, HitShape::Sphere] {
            let positive = intersect_shape(Vec3::Z * 3.0, Vec3::NEG_Z, Vec3::ZERO, Quat::IDENTITY, Vec3::splat(2.0), shape).unwrap();
            let mirrored = intersect_shape(Vec3::Z * 3.0, Vec3::NEG_Z, Vec3::ZERO, Quat::IDENTITY, Vec3::splat(-2.0), shape).unwrap();
            assert_eq!(positive, mirrored);
            assert!(contains_point(Vec3::ZERO, Vec3::ZERO, Quat::IDENTITY, Vec3::splat(-2.0), shape));
            assert!(intersect_shape(Vec3::Z, Vec3::ZERO, Vec3::ZERO, Quat::IDENTITY, Vec3::ONE, shape).is_none());
        }
        for shape in [HitShape::Box, HitShape::Sphere] {
            let (distance, _) = intersect_shape(Vec3::ZERO, Vec3::X, Vec3::ZERO, Quat::IDENTITY, Vec3::splat(2.0), shape).unwrap();
            assert!((distance - 1.0).abs() < 1e-6);
        }
        assert!(intersect_shape(Vec3::new(2.0, 0.0, 3.0), Vec3::NEG_Z, Vec3::ZERO, Quat::IDENTITY, Vec3::splat(2.0), HitShape::Box).is_none());
        let (distance, _) = intersect_shape(Vec3::new(1.0, 0.0, 3.0), Vec3::NEG_Z, Vec3::ZERO, Quat::IDENTITY, Vec3::splat(2.0), HitShape::Box).unwrap();
        assert_eq!(distance, 2.0);
    }

    #[test]
    fn pointer_geometry_cache_only_changes_when_source_changes() {
        use super::*;
        let mut app = App::new();
        app.add_systems(Update, prepare_pointer_shapes);
        let entity = app.world_mut().spawn((GlobalTransform::from_translation(Vec3::X), Toqueable(7, true), HitShape::Plane)).id();
        app.update();
        assert_eq!(app.world().get::<PreparedPointerShape>(entity).unwrap().position, Vec3::X);
        app.update();
        assert!(!app.world().entity(entity).get_ref::<PreparedPointerShape>().unwrap().is_changed());
        *app.world_mut().get_mut::<GlobalTransform>(entity).unwrap() = GlobalTransform::from_translation(Vec3::Y);
        app.update();
        assert_eq!(app.world().get::<PreparedPointerShape>(entity).unwrap().position, Vec3::Y);
    }

    #[test]
    fn pointer_blocking_stops_both_click_and_hover_without_event_target() {
        use super::*;
        let transform = GlobalTransform::IDENTITY;
        let mut hit = None;
        consider_pointer_hit(&mut hit, 3.0, &Toqueable(1, true), Vec3::ZERO, &transform);
        consider_pointer_hit(&mut hit, 2.0, &Toqueable(2, false), Vec3::ZERO, &transform);
        consider_pointer_hit(&mut hit, 4.0, &Toqueable(3, true), Vec3::ZERO, &transform);
        assert_eq!(hit.unwrap().1, None);
        consider_pointer_hit(&mut hit, 1.0, &Toqueable(4, true), Vec3::ZERO, &transform);
        assert_eq!(hit.unwrap().1, Some(4));
    }

    use super::*;

    #[derive(Resource)]
    struct VisibilityTestIds {
        hidden_self_posezone: Entity,
        hidden_parent_posezone: Entity,
    }

    #[derive(Resource, Default)]
    struct VisibilityTestResult {
        hidden_self_active: bool,
        hidden_parent_active: bool,
    }

    fn check_posezone_visibility_rule(
        ids: Res<VisibilityTestIds>,
        parent_query: Query<&Parent>,
        visibility_query: Query<&Visibility>,
        mut result: ResMut<VisibilityTestResult>,
    ) {
        result.hidden_self_active = ancestors_are_visible(
            ids.hidden_self_posezone,
            &parent_query,
            &visibility_query,
        );
        result.hidden_parent_active = ancestors_are_visible(
            ids.hidden_parent_posezone,
            &parent_query,
            &visibility_query,
        );
    }

    #[test]
    fn posezone_input_ignores_own_visibility_but_respects_hidden_parent() {
        let mut app = App::new();

        let visible_parent = app.world_mut().spawn(Visibility::Visible).id();
        let hidden_self_posezone = app.world_mut().spawn(Visibility::Hidden).id();
        app.world_mut()
            .entity_mut(visible_parent)
            .push_children(&[hidden_self_posezone]);

        let hidden_parent = app.world_mut().spawn(Visibility::Hidden).id();
        let hidden_parent_posezone = app.world_mut().spawn(Visibility::Visible).id();
        app.world_mut()
            .entity_mut(hidden_parent)
            .push_children(&[hidden_parent_posezone]);

        app.insert_resource(VisibilityTestIds {
            hidden_self_posezone,
            hidden_parent_posezone,
        });
        app.insert_resource(VisibilityTestResult::default());
        app.add_systems(Update, check_posezone_visibility_rule);
        app.update();

        let result = app.world().resource::<VisibilityTestResult>();
        assert!(
            result.hidden_self_active,
            "posezone visible=false should remain active as an input volume"
        );
        assert!(
            !result.hidden_parent_active,
            "posezone under a hidden parent should not receive input"
        );
    }

    #[test]
    fn controller_pose_composes_tracking_root_and_local_grip_pose() {
        let root_tf = Transform {
            translation: Vec3::new(10.0, 1.0, -4.0),
            rotation: Quat::from_rotation_y(std::f32::consts::FRAC_PI_2),
            scale: Vec3::ONE,
        };
        let local_tf = Transform {
            translation: Vec3::new(0.0, 2.0, -3.0),
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
        };

        let (world_pos, world_rot) = compose_tracking_pose(&root_tf, &local_tf);

        assert!(
            world_pos.distance(Vec3::new(7.0, 3.0, -4.0)) < 0.0001,
            "controller world position should be root * local, got {:?}",
            world_pos
        );
        assert!(
            world_rot.abs_diff_eq(root_tf.rotation, 0.0001),
            "controller world rotation should include tracking root rotation"
        );
    }

    #[test]
    fn controller_aim_direction_uses_controller_negative_y_axis() {
        let dir = controller_aim_direction(Quat::IDENTITY);
        assert!(dir.abs_diff_eq(Vec3::NEG_Y, 0.0001));

        let rotated = controller_aim_direction(Quat::from_rotation_z(std::f32::consts::FRAC_PI_2));
        assert!(rotated.abs_diff_eq(Vec3::X, 0.0001));
    }

    #[test]
    fn controller_ui_ray_direction_uses_legacy_negative_y_axis() {
        let dir = controller_ui_ray_direction(Quat::IDENTITY);
        assert!(dir.abs_diff_eq(Vec3::NEG_Y, 0.0001));

        let rotated = controller_ui_ray_direction(Quat::from_rotation_z(std::f32::consts::FRAC_PI_2));
        assert!(rotated.abs_diff_eq(Vec3::X, 0.0001));
    }
}
