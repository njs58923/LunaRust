use bevy::input::mouse::MouseMotion;
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorMoved, PrimaryWindow, WindowMode};
use crate::remote_mouse::{AbsolutePointer, RemoteMouse, refresh_remote_mouse};

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
            .init_resource::<RemoteMouse>()
            .add_systems(Update, desktop_fullscreen_toggle_system)
            .add_systems(Update, refresh_remote_mouse.before(desktop_shooter_toggle_system))
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

// Host window shortcut: independent of world camera permissions and UI focus.
fn desktop_fullscreen_toggle_system(
    keyboard: Res<ButtonInput<KeyCode>>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
) {
    if !keyboard.just_pressed(KeyCode::F11) { return; }
    let Ok(mut window) = windows.get_single_mut() else { return; };
    if !window.focused { return; }
    window.mode = if window.mode == WindowMode::Windowed {
        WindowMode::BorderlessFullscreen
    } else {
        WindowMode::Windowed
    };
}

fn desktop_shooter_toggle_system(
    remote: Res<RemoteMouse>,
    windows: Query<&Window, With<PrimaryWindow>>,
    focus: Res<crate::keyboard::KeyboardFocus>,
    mouse_button: Res<ButtonInput<MouseButton>>,
    keyboard: Res<ButtonInput<KeyCode>>,
    devtool_visible: Res<DevtoolVisible>,
    global_devtool: Res<GlobalDevtoolVisible>,
    mut shooter: ResMut<DesktopShooterActive>,
    mut sys_events: ResMut<crate::system_input::HostSystemInputEvents>,
) {
    // Devtool open → force off
    if devtool_visible.0 || global_devtool.0 || !windows.get_single().is_ok_and(|w| w.focused) {
        if shooter.0 {
            shooter.0 = false;
        }
        return;
    }

    // Escape: si shooter activo → consume y sale (preserva comportamiento).
    if focus.editable { shooter.0 = false; return; }
    // Si shooter inactivo → dispatcha systeminput (shell open via UX).
    if keyboard.just_pressed(KeyCode::Escape) {
        if shooter.0 {
            shooter.0 = false;
        } else {
            sys_events.0.push(crate::system_input::SystemInputEvent {
                action: crate::system_input::SystemInputAction::Shell,
                source: crate::system_input::SystemInputSource::KbEscape,
            });
        }
        return;
    }

    // Right click → enter shooter mode
    if remote.is_changed() { shooter.0 = false; }
    if mouse_button.just_pressed(MouseButton::Right) && !shooter.0 {
        shooter.0 = true;
    }
}

// ─── Cursor grab / release ───────────────────────────────────────────────────

fn desktop_cursor_lock_system(
    remote: Res<RemoteMouse>,
    shooter: Res<DesktopShooterActive>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
) {
    if !shooter.is_changed() && !remote.is_changed() {
        return;
    }
    let Ok(mut window) = windows.get_single_mut() else {
        return;
    };

    if shooter.0 {
        window.cursor.grab_mode = if remote.absolute { CursorGrabMode::Confined } else { CursorGrabMode::Locked };
        // winit 0.30.5 on Windows confines a HIDDEN + GRABBED cursor to
        // one pixel, even in Confined mode. Absolute RDP input requires
        // a movable cursor; hiding it would suppress all CursorMoved deltas.
        window.cursor.visible = remote.absolute;
    } else {
        window.cursor.grab_mode = CursorGrabMode::None;
        window.cursor.visible = true;
    }
}

// ─── Mouse look (yaw + pitch) ────────────────────────────────────────────────

const MOUSE_SENSITIVITY: f32 = 0.003;
const PITCH_LIMIT: f32 = std::f32::consts::FRAC_PI_2 - 0.05; // ~85°

fn desktop_mouse_look_system(
    remote: Res<RemoteMouse>,
    mut cursor_events: EventReader<CursorMoved>,
    mut windows: Query<(Entity, &mut Window), With<PrimaryWindow>>,
    mut absolute: Local<AbsolutePointer>,
    shooter: Res<DesktopShooterActive>,
    mut motion_events: EventReader<MouseMotion>,
    mut camera_query: Query<&mut Transform, With<DesktopCamera>>,
    mut pitch: ResMut<DesktopCameraPitch>,
) {
    if !shooter.0 {
        // Drain events so they don't accumulate
        motion_events.clear();
        cursor_events.clear(); absolute.reset();
        return;
    }

    let Ok((window_id, mut window)) = windows.get_single_mut() else { motion_events.clear(); cursor_events.clear(); absolute.reset(); return; };
    if !window.focused { motion_events.clear(); cursor_events.clear(); absolute.reset(); return; }
    let Ok(mut transform) = camera_query.get_single_mut() else {
        motion_events.clear(); cursor_events.clear(); absolute.reset(); return;
    };
    if shooter.is_changed() || remote.is_changed() {
        let (_, current_pitch, _) = transform.rotation.to_euler(EulerRot::YXZ);
        pitch.0 = current_pitch.clamp(-PITCH_LIMIT, PITCH_LIMIT);
    }
    let mut delta = Vec2::ZERO;
    if remote.absolute {
        motion_events.clear(); // Never add raw motion to absolute samples.
        if shooter.is_changed() || remote.is_changed() {
            absolute.reset(); cursor_events.clear();
            if let Some(position) = window.cursor_position() {
                absolute.sample(position,Vec2::new(window.width(),window.height()),window.scale_factor());
            }
            return;
        }
        let size = Vec2::new(window.width(), window.height());
        for ev in cursor_events.read() {
            if ev.window == window_id { delta += absolute.sample(ev.position,size,window.scale_factor()); }
        }
        if window.cursor_position().is_none() { absolute.reset(); }
        if let Some(center) = absolute.recenter() {
            window.set_cursor_position(Some(center));
        }
    } else {
        cursor_events.clear(); absolute.reset();
        for ev in motion_events.read() { delta += ev.delta; }
    }
    if delta == Vec2::ZERO {
        return;
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_mouse_capture_preserves_absolute_motion_and_releases_on_exit() {
        let mut app = App::new();
        app.insert_resource(RemoteMouse::forced(true))
            .insert_resource(DesktopShooterActive(true))
            .add_systems(Update, desktop_cursor_lock_system);
        let entity = app.world_mut().spawn((Window::default(), PrimaryWindow)).id();
        app.update();
        let window = app.world().get::<Window>(entity).unwrap();
        assert_eq!(window.cursor.grab_mode, CursorGrabMode::Confined);
        // Hidden + confined becomes a one-pixel lock in the Windows backend.
        assert!(window.cursor.visible);
        app.world_mut().resource_mut::<DesktopShooterActive>().0 = false;
        app.update();
        let window = app.world().get::<Window>(entity).unwrap();
        assert_eq!(window.cursor.grab_mode, CursorGrabMode::None);
        assert!(window.cursor.visible);
    }

    #[test]
    fn remote_mouse_local_capture_still_hides_cursor() {
        let mut app = App::new();
        app.insert_resource(RemoteMouse::forced(false))
            .insert_resource(DesktopShooterActive(true))
            .add_systems(Update, desktop_cursor_lock_system);
        let entity = app.world_mut().spawn((Window::default(), PrimaryWindow)).id();
        app.update();
        let window = app.world().get::<Window>(entity).unwrap();
        assert_eq!(window.cursor.grab_mode, CursorGrabMode::Locked);
        assert!(!window.cursor.visible);
    }

    #[test]
    fn remote_mouse_does_not_mix_absolute_positions_with_raw_motion() {
        let mut app = App::new();
        app.insert_resource(RemoteMouse::forced(true))
            .insert_resource(DesktopShooterActive(true))
            .init_resource::<DesktopCameraPitch>()
            .add_event::<MouseMotion>().add_event::<CursorMoved>()
            .add_systems(Update,desktop_mouse_look_system);
        let mut window = Window::default();
        window.focused=true;
        window.set_cursor_position(Some(Vec2::new(100.0,100.0)));
        let window=app.world_mut().spawn((window,PrimaryWindow)).id();
        let camera=app.world_mut().spawn((DesktopCamera,Transform::default())).id();
        app.update();
        app.world_mut().resource_mut::<Events<MouseMotion>>().send(MouseMotion{delta:Vec2::splat(65535.0)});
        app.world_mut().resource_mut::<Events<CursorMoved>>().send(CursorMoved{window,position:Vec2::new(110.0,100.0),delta:None});
        app.update();
        let rotation=app.world().get::<Transform>(camera).unwrap().rotation;
        let (yaw,pitch,_)=rotation.to_euler(EulerRot::YXZ);
        assert!((yaw+10.0*MOUSE_SENSITIVITY).abs()<0.00001);
        assert!(pitch.abs()<0.00001);
        app.update();
        assert_eq!(app.world().get::<Transform>(camera).unwrap().rotation,rotation);
        app.world_mut().resource_mut::<DesktopShooterActive>().0=false;
        app.world_mut().resource_mut::<Events<CursorMoved>>().send(CursorMoved{window,position:Vec2::new(300.0,300.0),delta:None});
        app.update();
        assert_eq!(app.world().get::<Transform>(camera).unwrap().rotation,rotation);
    }

    #[test]
    fn remote_mouse_fallback_keeps_local_raw_input_unchanged() {
        let mut app=App::new();
        app.insert_resource(RemoteMouse::forced(false))
            .insert_resource(DesktopShooterActive(true))
            .init_resource::<DesktopCameraPitch>()
            .add_event::<MouseMotion>().add_event::<CursorMoved>()
            .add_systems(Update,desktop_mouse_look_system);
        let window=app.world_mut().spawn((Window::default(),PrimaryWindow)).id();
        let camera=app.world_mut().spawn((DesktopCamera,Transform::default())).id();
        app.update();
        app.world_mut().resource_mut::<Events<MouseMotion>>().send(MouseMotion{delta:Vec2::new(10.0,0.0)});
        app.world_mut().resource_mut::<Events<CursorMoved>>().send(CursorMoved{window,position:Vec2::splat(500.0),delta:None});
        app.update();
        let (yaw,_,_)=app.world().get::<Transform>(camera).unwrap().rotation.to_euler(EulerRot::YXZ);
        assert!((yaw+10.0*MOUSE_SENSITIVITY).abs()<0.00001);
    }
}
