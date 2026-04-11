use bevy::{math::vec3, prelude::*};
use bevy_mod_openxr::{helper_traits::ToQuat, resources::OxrViews};
use bevy_mod_xr::{actions::ActionType, session::XrTrackingRoot};
use bevy_xr_utils::xr_utils_actions::{
    ActiveSet, XRUtilsAction, XRUtilsActionSet, XRUtilsActionState, XRUtilsActionSystemSet,
    XRUtilsActionsPlugin, XRUtilsBinding,
};

// ─── Markers ─────────────────────────────────────────────────────────────────

#[derive(Component)] pub struct LeftStick;
#[derive(Component)] pub struct RightStick;
#[derive(Component)] pub struct LeftTrigger;
#[derive(Component)] pub struct RightTrigger;
#[derive(Component)] pub struct LeftGrip;
#[derive(Component)] pub struct RightGrip;
#[derive(Component)] pub struct BtnA;   // right A / index right A
#[derive(Component)] pub struct BtnB;   // right B
#[derive(Component)] pub struct BtnX;   // left X  (oculus)
#[derive(Component)] pub struct BtnY;   // left Y  (oculus)
#[derive(Component)] pub struct LeftStickClick;
#[derive(Component)] pub struct RightStickClick;
#[derive(Component)] pub struct MenuBtn;

// ─── Snap turn cooldown ───────────────────────────────────────────────────────

#[derive(Resource)]
pub struct SnapTurnCooldown(pub Timer);

impl Default for SnapTurnCooldown {
    fn default() -> Self {
        let mut t = Timer::from_seconds(0.4, TimerMode::Once);
        t.tick(t.duration()); // start already finished so first snap fires immediately
        Self(t)
    }
}

// ─── Plugin ──────────────────────────────────────────────────────────────────

pub struct VrLocomotionPlugin;

impl Plugin for VrLocomotionPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(XRUtilsActionsPlugin)
            .insert_resource(SnapTurnCooldown::default())
            .add_systems(
                Startup,
                create_locomotion_actions.before(XRUtilsActionSystemSet::CreateEvents),
            )
            .add_systems(
                Update,
                (
                    handle_smooth_locomotion,
                    handle_snap_turn,
                    log_buttons,
                )
                    .after(XRUtilsActionSystemSet::SyncActionStates),
            );
    }
}

// ─── Action setup ─────────────────────────────────────────────────────────────

fn create_locomotion_actions(mut commands: Commands) {
    let set = commands
        .spawn((
            XRUtilsActionSet {
                name: "luna_locomotion".into(),
                pretty_name: "Luna Locomotion".into(),
                priority: u32::MIN,
            },
            ActiveSet,
        ))
        .id();

    // Helper to spawn an action + bindings and attach to set
    macro_rules! action {
        ($name:expr, $local:expr, $ty:expr, $marker:expr, $bindings:expr) => {{
            let a = commands
                .spawn((
                    XRUtilsAction {
                        action_name: $name.into(),
                        localized_name: $local.into(),
                        action_type: $ty,
                    },
                    $marker,
                ))
                .id();
            for (profile, binding) in $bindings {
                let b = commands
                    .spawn(XRUtilsBinding {
                        profile: profile.into(),
                        binding: binding.into(),
                    })
                    .id();
                commands.entity(a).add_child(b);
            }
            commands.entity(set).add_child(a);
        }};
    }

    let touch = "/interaction_profiles/oculus/touch_controller";
    let index = "/interaction_profiles/valve/index_controller";

    // Sticks (Vector)
    action!("left_stick",  "Left Stick",  ActionType::Vector, LeftStick,  [
        (touch, "/user/hand/left/input/thumbstick"),
        (index, "/user/hand/left/input/thumbstick"),
    ]);
    action!("right_stick", "Right Stick", ActionType::Vector, RightStick, [
        (touch, "/user/hand/right/input/thumbstick"),
        (index, "/user/hand/right/input/thumbstick"),
    ]);

    // Triggers (Float)
    action!("left_trigger",  "Left Trigger",  ActionType::Float, LeftTrigger,  [
        (touch, "/user/hand/left/input/trigger/value"),
        (index, "/user/hand/left/input/trigger/value"),
    ]);
    action!("right_trigger", "Right Trigger", ActionType::Float, RightTrigger, [
        (touch, "/user/hand/right/input/trigger/value"),
        (index, "/user/hand/right/input/trigger/value"),
    ]);

    // Grips (Float)
    action!("left_grip",  "Left Grip",  ActionType::Float, LeftGrip,  [
        (touch, "/user/hand/left/input/squeeze/value"),
        (index, "/user/hand/left/input/squeeze/value"),
    ]);
    action!("right_grip", "Right Grip", ActionType::Float, RightGrip, [
        (touch, "/user/hand/right/input/squeeze/value"),
        (index, "/user/hand/right/input/squeeze/value"),
    ]);

    // Buttons A/B right hand (Bool)
    action!("btn_a", "Button A", ActionType::Bool, BtnA, [
        (touch, "/user/hand/right/input/a/click"),
        (index, "/user/hand/right/input/a/click"),
    ]);
    action!("btn_b", "Button B", ActionType::Bool, BtnB, [
        (touch, "/user/hand/right/input/b/click"),
        (index, "/user/hand/right/input/b/click"),
    ]);

    // Buttons X/Y left hand — Oculus only (no X/Y on Index)
    action!("btn_x", "Button X", ActionType::Bool, BtnX, [
        (touch, "/user/hand/left/input/x/click"),
    ]);
    action!("btn_y", "Button Y", ActionType::Bool, BtnY, [
        (touch, "/user/hand/left/input/y/click"),
    ]);

    // Thumbstick clicks (Bool)
    action!("left_stick_click",  "Left Stick Click",  ActionType::Bool, LeftStickClick,  [
        (touch, "/user/hand/left/input/thumbstick/click"),
        (index, "/user/hand/left/input/thumbstick/click"),
    ]);
    action!("right_stick_click", "Right Stick Click", ActionType::Bool, RightStickClick, [
        (touch, "/user/hand/right/input/thumbstick/click"),
        (index, "/user/hand/right/input/thumbstick/click"),
    ]);

    // Menu (Bool) — Oculus left hand only
    action!("menu", "Menu", ActionType::Bool, MenuBtn, [
        (touch, "/user/hand/left/input/menu/click"),
    ]);
}

// ─── Smooth locomotion (left stick) ──────────────────────────────────────────

fn handle_smooth_locomotion(
    q: Query<&XRUtilsActionState, With<LeftStick>>,
    mut root: Query<&mut Transform, With<XrTrackingRoot>>,
    views: ResMut<OxrViews>,
    time: Res<Time>,
) {
    let Ok(mut root_tf) = root.get_single_mut() else { return };

    for state in q.iter() {
        if let XRUtilsActionState::Vector(v) = state {
            let input = vec3(v.current_state[0], 0.0, -v.current_state[1]);
            if input.length_squared() < 0.01 { continue; }

            let speed = 3.0;
            let dir = if let Some(view) = views.first() {
                // HMD-relative but only on XZ plane
                let mut fwd = view.pose.orientation.to_quat().mul_vec3(input);
                fwd.y = 0.0;
                fwd.normalize_or_zero()
            } else {
                input.normalize_or_zero()
            };

            root_tf.translation += dir * speed * time.delta_seconds();
        }
    }
}

// ─── Snap turn (right stick) ──────────────────────────────────────────────────

const SNAP_ANGLE: f32 = std::f32::consts::FRAC_PI_4; // 45°
const SNAP_THRESHOLD: f32 = 0.6;

fn handle_snap_turn(
    q: Query<&XRUtilsActionState, With<RightStick>>,
    mut root: Query<&mut Transform, With<XrTrackingRoot>>,
    mut cooldown: ResMut<SnapTurnCooldown>,
    time: Res<Time>,
) {
    cooldown.0.tick(time.delta());
    if !cooldown.0.finished() { return; }

    let Ok(mut root_tf) = root.get_single_mut() else { return };

    for state in q.iter() {
        if let XRUtilsActionState::Vector(v) = state {
            let x = v.current_state[0];
            if x.abs() < SNAP_THRESHOLD { continue; }

            let angle = if x > 0.0 { -SNAP_ANGLE } else { SNAP_ANGLE };
            root_tf.rotation *= Quat::from_rotation_y(angle);
            cooldown.0.reset();
        }
    }
}

// ─── Button logging ───────────────────────────────────────────────────────────

macro_rules! log_bool {
    ($q:expr, $label:expr) => {
        for state in $q.iter() {
            if let XRUtilsActionState::Bool(s) = state {
                if s.current_state && s.changed_since_last_sync {
                    info!("[VR] {} pressed", $label);
                }
            }
        }
    };
    (float $q:expr, $label:expr) => {
        for state in $q.iter() {
            if let XRUtilsActionState::Float(s) = state {
                if s.current_state > 0.05 {
                    info!("[VR] {} = {:.2}", $label, s.current_state);
                }
            }
        }
    };
}

#[allow(clippy::too_many_arguments)]
fn log_buttons(
    q_lt: Query<&XRUtilsActionState, With<LeftTrigger>>,
    q_rt: Query<&XRUtilsActionState, With<RightTrigger>>,
    q_lg: Query<&XRUtilsActionState, With<LeftGrip>>,
    q_rg: Query<&XRUtilsActionState, With<RightGrip>>,
    q_a:  Query<&XRUtilsActionState, With<BtnA>>,
    q_b:  Query<&XRUtilsActionState, With<BtnB>>,
    q_x:  Query<&XRUtilsActionState, With<BtnX>>,
    q_y:  Query<&XRUtilsActionState, With<BtnY>>,
    q_lc: Query<&XRUtilsActionState, With<LeftStickClick>>,
    q_rc: Query<&XRUtilsActionState, With<RightStickClick>>,
    q_m:  Query<&XRUtilsActionState, With<MenuBtn>>,
) {
    log_bool!(float q_lt, "Left Trigger");
    log_bool!(float q_rt, "Right Trigger");
    log_bool!(float q_lg, "Left Grip");
    log_bool!(float q_rg, "Right Grip");
    log_bool!(q_a,  "A");
    log_bool!(q_b,  "B");
    log_bool!(q_x,  "X");
    log_bool!(q_y,  "Y");
    log_bool!(q_lc, "Left Stick Click");
    log_bool!(q_rc, "Right Stick Click");
    log_bool!(q_m,  "Menu");
}
