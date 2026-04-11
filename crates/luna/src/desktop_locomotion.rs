use bevy::input::mouse::MouseMotion;
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, PrimaryWindow};

use crate::{DesktopCamera, DevtoolVisible, GlobalDevtoolVisible};

// ─── Resources ───────────────────────────────────────────────────────────────

/// Whether the shooter-style mouse look is currently active.
#[derive(Resource)]
pub struct DesktopShooterActive(pub bool);

impl Default for DesktopShooterActive {
    fn default() -> Self {
        Self(false)
    }
}

/// Accumulated pitch to clamp vertical look.
#[derive(Resource, Default)]
pub struct DesktopCameraPitch(pub f32);

// ─── Plugin ──────────────────────────────────────────────────────────────────

pub struct DesktopLocomotionPlugin;

impl Plugin for DesktopLocomotionPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(DesktopShooterActive::default())
            .insert_resource(DesktopCameraPitch::default())
            .add_systems(
                Update,
                (
                    desktop_shooter_toggle_system,
                    desktop_cursor_lock_system,
                    desktop_mouse_look_system,
                )
                    .chain()
                    .run_if(|rm: Res<crate::RenderMode>| !rm.is_vr)
                    .run_if(crate::permissions::desktop_camera_control_enabled),
            );
    }
}

// ─── Toggle: right-click enters shooter mode, Escape exits ───────────────────

fn desktop_shooter_toggle_system(
    mouse_button: Res<ButtonInput<MouseButton>>,
    keyboard: Res<ButtonInput<KeyCode>>,
    devtool_visible: Res<DevtoolVisible>,
    global_devtool: Res<GlobalDevtoolVisible>,
    mut shooter: ResMut<DesktopShooterActive>,
) {
    // Devtool open → force off
    if devtool_visible.0 || global_devtool.0 {
        if shooter.0 {
            shooter.0 = false;
        }
        return;
    }

    // Escape → exit shooter mode
    if keyboard.just_pressed(KeyCode::Escape) && shooter.0 {
        shooter.0 = false;
        return;
    }

    // Right click → enter shooter mode
    if mouse_button.just_pressed(MouseButton::Right) && !shooter.0 {
        shooter.0 = true;
    }
}

// ─── Cursor grab / release ───────────────────────────────────────────────────

fn desktop_cursor_lock_system(
    shooter: Res<DesktopShooterActive>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
) {
    if !shooter.is_changed() {
        return;
    }
    let Ok(mut window) = windows.get_single_mut() else {
        return;
    };

    if shooter.0 {
        window.cursor.grab_mode = CursorGrabMode::Locked;
        window.cursor.visible = false;
    } else {
        window.cursor.grab_mode = CursorGrabMode::None;
        window.cursor.visible = true;
    }
}

// ─── Mouse look (yaw + pitch) ────────────────────────────────────────────────

const MOUSE_SENSITIVITY: f32 = 0.003;
const PITCH_LIMIT: f32 = std::f32::consts::FRAC_PI_2 - 0.05; // ~85°

fn desktop_mouse_look_system(
    shooter: Res<DesktopShooterActive>,
    mut motion_events: EventReader<MouseMotion>,
    mut camera_query: Query<&mut Transform, With<DesktopCamera>>,
    mut pitch: ResMut<DesktopCameraPitch>,
) {
    if !shooter.0 {
        // Drain events so they don't accumulate
        motion_events.clear();
        return;
    }

    let mut delta = Vec2::ZERO;
    for ev in motion_events.read() {
        delta += ev.delta;
    }
    if delta == Vec2::ZERO {
        return;
    }

    let Ok(mut transform) = camera_query.get_single_mut() else {
        return;
    };

    // Yaw (rotate around world Y)
    let yaw = -delta.x * MOUSE_SENSITIVITY;
    // Pitch (rotate around local X)
    let pitch_delta = -delta.y * MOUSE_SENSITIVITY;

    pitch.0 = (pitch.0 + pitch_delta).clamp(-PITCH_LIMIT, PITCH_LIMIT);

    // Reconstruct rotation: yaw first, then pitch
    let (current_yaw, _, _) = transform.rotation.to_euler(EulerRot::YXZ);
    let new_yaw = current_yaw + yaw;
    transform.rotation = Quat::from_euler(EulerRot::YXZ, new_yaw, pitch.0, 0.0);
}
