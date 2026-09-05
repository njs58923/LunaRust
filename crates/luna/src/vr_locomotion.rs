use bevy::{math::vec3, prelude::*};
use bevy_mod_openxr::{
    action_binding::{OxrSendActionBindings, OxrSuggestActionBinding},
    action_set_attaching::OxrAttachActionSet,
    action_set_syncing::{OxrActionSetSyncSet, OxrSyncActionSet},
    helper_traits::ToQuat,
    openxr_session_available, openxr_session_running,
    resources::{OxrInstance, OxrViews},
    session::OxrSession,
};
use bevy_mod_xr::session::{XrSessionCreated, XrTrackingRoot};
use openxr::{Path, Vector2f};

// ─── Resource that owns all OpenXR action objects ─────────────────────────────

#[derive(Resource)]
pub struct LunaLocomotionActions {
    pub set:               openxr::ActionSet,
    pub left_stick:        openxr::Action<Vector2f>,
    pub right_stick:       openxr::Action<Vector2f>,
    pub left_trigger:      openxr::Action<f32>,
    pub right_trigger:     openxr::Action<f32>,
    pub left_grip:         openxr::Action<f32>,
    pub right_grip:        openxr::Action<f32>,
    pub btn_a:             openxr::Action<bool>,
    pub btn_b:             openxr::Action<bool>,
    pub btn_x:             openxr::Action<bool>,
    pub btn_y:             openxr::Action<bool>,
    pub left_stick_click:  openxr::Action<bool>,
    pub right_stick_click: openxr::Action<bool>,
    pub menu:              openxr::Action<bool>,
}

// ─── Snap turn cooldown ───────────────────────────────────────────────────────

#[derive(Resource)]
pub struct SnapTurnCooldown(pub Timer);

impl Default for SnapTurnCooldown {
    fn default() -> Self {
        let mut t = Timer::from_seconds(0.4, TimerMode::Once);
        t.tick(t.duration());
        Self(t)
    }
}

// ─── Menu button debounce ─────────────────────────────────────────────────────

#[derive(Resource)]
pub struct MenuButtonDebounce(pub Timer);

impl Default for MenuButtonDebounce {
    fn default() -> Self {
        let mut t = Timer::from_seconds(0.2, TimerMode::Once);
        t.tick(t.duration());
        Self(t)
    }
}

// ─── Plugin ──────────────────────────────────────────────────────────────────

pub struct VrLocomotionPlugin;

impl Plugin for VrLocomotionPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(SnapTurnCooldown::default())
            .insert_resource(MenuButtonDebounce::default())
            // Create OXR objects as soon as OpenXR is available, and keep this
            // resilient if the runtime comes up after Startup.
            .add_systems(
                Startup,
                create_locomotion_actions.run_if(openxr_session_available),
            )
            .add_systems(
                Update,
                create_locomotion_actions.run_if(openxr_session_available),
            )
            // Suggest bindings when the session is being created (OxrSendActionBindings schedule)
            .add_systems(OxrSendActionBindings, suggest_locomotion_bindings)
            // Attach the set right when the session is created
            .add_systems(XrSessionCreated, attach_locomotion_set)
            // Sync every frame while session is running
            .add_systems(
                PreUpdate,
                sync_locomotion_set
                    .run_if(openxr_session_running)
                    .before(OxrActionSetSyncSet),
            )
            // Read + act on input after sync
            .add_systems(
                Update,
                (
                    handle_smooth_locomotion,
                    handle_snap_turn,
                    handle_menu_button,
                    log_buttons,
                )
                    .run_if(openxr_session_running)
                    .run_if(crate::permissions::vr_locomotion_enabled)
                    .run_if(resource_exists::<LunaLocomotionActions>),
            )
            // Snapshot HMD pose: corre siempre en sesión VR (independiente de
            // vr_locomotion_enabled). Workers sin READ_HMD_POSE no la reciben
            // — el filtrado pasa en propagate_viewer_pose_to_workers.
            .add_systems(
                Update,
                update_vr_viewer_pose.run_if(openxr_session_running),
            );
    }
}

// ─── Action creation (Startup) ────────────────────────────────────────────────

fn create_locomotion_actions(
    instance: Res<OxrInstance>,
    mut cmds: Commands,
    actions: Option<Res<LunaLocomotionActions>>,
) {
    if actions.is_some() {
        return;
    }

    let set = instance
        .create_action_set("luna_locomotion", "Luna Locomotion", 0)
        .unwrap();

    macro_rules! vec_action {
        ($name:expr, $local:expr) => {
            set.create_action::<Vector2f>($name, $local, &[]).unwrap()
        };
    }
    macro_rules! f32_action {
        ($name:expr, $local:expr) => {
            set.create_action::<f32>($name, $local, &[]).unwrap()
        };
    }
    macro_rules! bool_action {
        ($name:expr, $local:expr) => {
            set.create_action::<bool>($name, $local, &[]).unwrap()
        };
    }

    cmds.insert_resource(LunaLocomotionActions {
        left_stick:        vec_action!("left_stick",        "Left Stick"),
        right_stick:       vec_action!("right_stick",       "Right Stick"),
        left_trigger:      f32_action!("left_trigger",      "Left Trigger"),
        right_trigger:     f32_action!("right_trigger",     "Right Trigger"),
        left_grip:         f32_action!("left_grip",         "Left Grip"),
        right_grip:        f32_action!("right_grip",        "Right Grip"),
        btn_a:             bool_action!("btn_a",            "Button A"),
        btn_b:             bool_action!("btn_b",            "Button B"),
        btn_x:             bool_action!("btn_x",            "Button X"),
        btn_y:             bool_action!("btn_y",            "Button Y"),
        left_stick_click:  bool_action!("left_stick_click", "Left Stick Click"),
        right_stick_click: bool_action!("right_stick_click","Right Stick Click"),
        menu:              bool_action!("menu",             "Menu"),
        set,
    });
}

// ─── Binding suggestions (fires at session creation via OxrSendActionBindings) ─

fn suggest_locomotion_bindings(
    actions: Option<Res<LunaLocomotionActions>>,
    mut bindings: EventWriter<OxrSuggestActionBinding>,
) {
    let Some(a) = actions else { return };

    let touch = "/interaction_profiles/oculus/touch_controller";
    let index = "/interaction_profiles/valve/index_controller";

    macro_rules! bind {
        ($action:expr, $profile:expr, $path:expr) => {
            bindings.send(OxrSuggestActionBinding {
                action: $action.as_raw(),
                interaction_profile: $profile.into(),
                bindings: vec![$path.into()],
            });
        };
    }

    // Sticks
    bind!(a.left_stick,  touch, "/user/hand/left/input/thumbstick");
    bind!(a.left_stick,  index, "/user/hand/left/input/thumbstick");
    bind!(a.right_stick, touch, "/user/hand/right/input/thumbstick");
    bind!(a.right_stick, index, "/user/hand/right/input/thumbstick");

    // Triggers
    bind!(a.left_trigger,  touch, "/user/hand/left/input/trigger/value");
    bind!(a.left_trigger,  index, "/user/hand/left/input/trigger/value");
    bind!(a.right_trigger, touch, "/user/hand/right/input/trigger/value");
    bind!(a.right_trigger, index, "/user/hand/right/input/trigger/value");

    // Grips
    bind!(a.left_grip,  touch, "/user/hand/left/input/squeeze/value");
    bind!(a.left_grip,  index, "/user/hand/left/input/squeeze/value");
    bind!(a.right_grip, touch, "/user/hand/right/input/squeeze/value");
    bind!(a.right_grip, index, "/user/hand/right/input/squeeze/value");

    // Buttons A / B (right hand)
    bind!(a.btn_a, touch, "/user/hand/right/input/a/click");
    bind!(a.btn_a, index, "/user/hand/right/input/a/click");
    bind!(a.btn_b, touch, "/user/hand/right/input/b/click");
    bind!(a.btn_b, index, "/user/hand/right/input/b/click");

    // Buttons X / Y (left hand — Oculus only)
    bind!(a.btn_x, touch, "/user/hand/left/input/x/click");
    bind!(a.btn_y, touch, "/user/hand/left/input/y/click");

    // Thumbstick clicks
    bind!(a.left_stick_click,  touch, "/user/hand/left/input/thumbstick/click");
    bind!(a.left_stick_click,  index, "/user/hand/left/input/thumbstick/click");
    bind!(a.right_stick_click, touch, "/user/hand/right/input/thumbstick/click");
    bind!(a.right_stick_click, index, "/user/hand/right/input/thumbstick/click");

    // Menu (Oculus left hand only)
    bind!(a.menu, touch, "/user/hand/left/input/menu/click");
}

// ─── Attach + sync ────────────────────────────────────────────────────────────

fn attach_locomotion_set(
    actions: Option<Res<LunaLocomotionActions>>,
    mut attach: EventWriter<OxrAttachActionSet>,
) {
    let Some(a) = actions else { return };
    attach.send(OxrAttachActionSet(a.set.clone()));
}

fn sync_locomotion_set(
    actions: Option<Res<LunaLocomotionActions>>,
    mut sync: EventWriter<OxrSyncActionSet>,
) {
    let Some(a) = actions else { return };
    sync.send(OxrSyncActionSet(a.set.clone()));
}

// ─── Smooth locomotion (left stick) ──────────────────────────────────────────

fn handle_smooth_locomotion(
    actions: Res<LunaLocomotionActions>,
    session: Res<OxrSession>,
    mut root: Query<&mut Transform, With<XrTrackingRoot>>,
    views: ResMut<OxrViews>,
    time: Res<Time>,
) {
    let Ok(state) = actions.left_stick.state(&session, Path::NULL) else { return };
    let input = vec3(state.current_state.x, 0.0, -state.current_state.y);
    let magnitude = input.length().min(1.0);
    if magnitude < 0.01 { return; }

    let Ok(mut root_tf) = root.get_single_mut() else { return };
    let max_speed = 3.0;

    // view.pose.orientation is in tracking space; root rotation is tracking→world.
    // Compose both so locomotion follows where the player is actually looking.
    let dir = if let Some(view) = views.first() {
        let hmd_dir = view.pose.orientation.to_quat().mul_vec3(input.normalize_or_zero());
        let mut world_dir = root_tf.rotation.mul_vec3(hmd_dir);
        world_dir.y = 0.0;
        world_dir.normalize_or_zero()
    } else {
        input.normalize_or_zero()
    };

    root_tf.translation += dir * max_speed * magnitude * time.delta_seconds();
}

// ─── Snap turn (right stick) ──────────────────────────────────────────────────

const SNAP_ANGLE: f32 = std::f32::consts::FRAC_PI_4; // 45°
const SNAP_THRESHOLD: f32 = 0.6;

fn handle_snap_turn(
    actions: Res<LunaLocomotionActions>,
    session: Res<OxrSession>,
    views: Res<OxrViews>,
    mut root: Query<&mut Transform, With<XrTrackingRoot>>,
    mut cooldown: ResMut<SnapTurnCooldown>,
    time: Res<Time>,
) {
    cooldown.0.tick(time.delta());
    if !cooldown.0.finished() { return; }

    let Ok(state) = actions.right_stick.state(&session, Path::NULL) else { return };
    let x = state.current_state.x;
    if x.abs() < SNAP_THRESHOLD { return; }

    let Ok(mut root_tf) = root.get_single_mut() else { return };
    let angle = if x > 0.0 { -SNAP_ANGLE } else { SNAP_ANGLE };
    if views.is_empty() { return; }
    // Midpoint of both eyes, in tracking coordinates (also works for mono).
    let hmd_position = views.iter().map(|v| Vec3::new(
        v.pose.position.x, v.pose.position.y, v.pose.position.z,
    )).sum::<Vec3>() / views.len() as f32;
    rotate_tracking_root_about_head(&mut root_tf, hmd_position, angle);
    cooldown.0.reset();
}

fn rotate_tracking_root_about_head(root: &mut Transform, head_local: Vec3, angle: f32) {
    let head_world = root.transform_point(head_local);
    root.rotation = Quat::from_rotation_y(angle) * root.rotation;
    root.translation = head_world - root.rotation * (root.scale * head_local);
}

#[cfg(test)]
mod snap_turn_tests {
    use super::*;

    #[test]
    fn snap_turn_preserves_head_position_away_from_tracking_origin() {
        let mut root = Transform::from_xyz(4.0, 0.5, -7.0)
            .with_rotation(Quat::from_rotation_y(0.7))
            .with_scale(Vec3::splat(1.5));
        let head = Vec3::new(2.0, 1.7, -3.0);
        let before = root.transform_point(head);
        let original = root;
        for _ in 0..8 {
            rotate_tracking_root_about_head(&mut root, head, SNAP_ANGLE);
            assert!(root.transform_point(head).distance(before) < 0.0001);
        }
        assert!(root.translation.distance(original.translation) < 0.0001);
        assert!(root.rotation.abs_diff_eq(original.rotation, 0.0001)
            || root.rotation.abs_diff_eq(-original.rotation, 0.0001));
    }
}

// ─── Button logging ───────────────────────────────────────────────────────────

fn log_buttons(actions: Res<LunaLocomotionActions>, session: Res<OxrSession>) {
    macro_rules! log_bool {
        ($action:expr, $label:expr) => {
            if let Ok(s) = $action.state(&session, Path::NULL) {
                if s.current_state && s.changed_since_last_sync {
                    info!("[VR] {} pressed", $label);
                }
            }
        };
    }
    macro_rules! log_float {
        ($action:expr, $label:expr) => {
            if let Ok(s) = $action.state(&session, Path::NULL) {
                if s.current_state > 0.05 && s.changed_since_last_sync {
                    info!("[VR] {} = {:.2}", $label, s.current_state);
                }
            }
        };
    }

    log_bool!(actions.btn_a,              "A");
    log_bool!(actions.btn_b,              "B");
    log_bool!(actions.btn_x,              "X");
    log_bool!(actions.btn_y,              "Y");
    log_bool!(actions.left_stick_click,   "Left Stick Click");
    log_bool!(actions.right_stick_click,  "Right Stick Click");
    log_bool!(actions.menu,               "Menu");
}

// ─── Viewer pose snapshot (HMD en world space) ────────────────────────────────

/// Actualiza el snapshot global de pose del usuario para que workers JS con
/// READ_HMD_POSE puedan leerlo vía `dimention.readViewerPose()`. Compone
/// pose del HMD (tracking space) con `XrTrackingRoot` para coords mundo.
pub fn update_vr_viewer_pose(
    views: Res<OxrViews>,
    tracking_root: Query<&Transform, With<XrTrackingRoot>>,
    mut snapshot: ResMut<crate::viewer_pose::ViewerPoseGlobalSnapshot>,
) {
    let Some(view) = views.first() else { return };
    let Ok(root_tf) = tracking_root.get_single() else { return };

    let hmd_pos_tracking = vec3(
        view.pose.position.x,
        view.pose.position.y,
        view.pose.position.z,
    );
    let hmd_rot_tracking: Quat = view.pose.orientation.to_quat();

    // Transformar a world space.
    let world_pos = root_tf.translation + root_tf.rotation * hmd_pos_tracking;
    let world_rot = root_tf.rotation * hmd_rot_tracking;
    let forward = world_rot * Vec3::NEG_Z;
    let (yaw, pitch, _roll) = world_rot.to_euler(EulerRot::YXZ);

    snapshot.0 = Some(crate::viewer_pose::ViewerPose {
        mode: crate::viewer_pose::ViewerMode::Vr,
        px: world_pos.x,
        py: world_pos.y,
        pz: world_pos.z,
        forward_x: forward.x,
        forward_y: forward.y,
        forward_z: forward.z,
        yaw,
        pitch,
        qx: world_rot.x,
        qy: world_rot.y,
        qz: world_rot.z,
        qw: world_rot.w,
        aspect: 0.0,
        fov_y_rad: 0.0,
    });
}

// ─── Menu button → systeminput dispatch ───────────────────────────────────────

fn handle_menu_button(
    actions: Res<LunaLocomotionActions>,
    session: Res<OxrSession>,
    mut debounce: ResMut<MenuButtonDebounce>,
    mut events: ResMut<crate::system_input::HostSystemInputEvents>,
    time: Res<Time>,
) {
    debounce.0.tick(time.delta());

    let Ok(state) = actions.menu.state(&session, Path::NULL) else { return };
    if !(state.current_state && state.changed_since_last_sync) {
        return;
    }
    if !debounce.0.finished() {
        return;
    }
    debounce.0.reset();

    events.0.push(crate::system_input::SystemInputEvent {
        action: crate::system_input::SystemInputAction::Shell,
        source: crate::system_input::SystemInputSource::VrMenu,
    });
}
