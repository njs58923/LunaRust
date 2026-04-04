use std::env;

use bevy::{
    asset::AssetPlugin,
    diagnostic::FrameTimeDiagnosticsPlugin,
    prelude::*,
    window::{PresentMode, Window},
};
use bevy_egui::EguiPlugin;
use tokio::runtime::Runtime;
use virtual_dom::dom::element::build_world;

use bevy_mod_openxr::add_xr_plugins;
use bevy_mod_xr::session::{
    XrBeginSessionEvent, XrCreateSessionEvent, XrDestroySessionEvent, XrEndSessionEvent,
    XrRequestExitEvent, XrSessionPlugin, XrState, XrStateChanged,
};

use luna::{dom, js, ui, utils};
use luna::*;

// ─── main ────────────────────────────────────────────────────────────────────

fn main() {
    let mut app = App::new();

    let args: Vec<String> = env::args().collect();
    println!("Args: {:?}", &args[1..]);

    let ar_on = args.iter().any(|a| a == "--ar");

    let default_plugins = DefaultPlugins
        .set(AssetPlugin {
            file_path: "assets".into(),
            watch_for_changes_override: Some(false),
            ..Default::default()
        })
        .set(WindowPlugin {
            primary_window: Some(Window {
                present_mode: PresentMode::Immediate,
                ..default()
            }),
            ..default()
        });

    app.add_plugins(
        add_xr_plugins(default_plugins).set(XrSessionPlugin { auto_handle: false }),
    );
    app.add_plugins(bevy_xr_utils::hand_gizmos::HandGizmosPlugin);
    app.insert_resource(RenderMode { is_vr: ar_on });
    app.add_plugins(EguiPlugin);
    app.add_plugins(FrameTimeDiagnosticsPlugin);

    // Resources
    let auto_load_config = AutoLoadConfig::default();
    let initial_url = if auto_load_config.enabled {
        auto_load_config.start_url.clone()
    } else {
        "luna://home".to_string()
    };

    app.insert_resource(VirtualDomData::default());
    app.insert_resource(DirtyNodes::default());
    app.insert_resource(EntityMap::default());
    app.insert_resource(ElemenetWorld(build_world()));
    app.insert_resource(EntityCounter::default());
    app.insert_resource(FpsCounter::default());
    app.insert_resource(CurrentUrl(initial_url));
    app.insert_resource(AutoLoadConfig::default());
    app.insert_resource(ReloadTrigger(false));
    app.insert_resource(AttributeUpdates::default());
    app.insert_resource(DeleteRequests::default());
    app.insert_resource(DevtoolVisible(false));
    app.insert_resource(LogPanel::default());
    app.insert_resource(TokioRuntime(Runtime::new().expect("Failed to create Tokio runtime")));
    app.insert_resource(ModelCache::default());
    app.insert_resource(TextMaterialCache::default());
    app.insert_resource(PerformanceStats::default());
    app.insert_resource(DevtoolState::default());
    app.insert_resource(PendingScripts::default());

    // Systems
    app.add_systems(Startup, (setup, js::init_js_runtime).chain());

    app.add_systems(
        Update,
        (
            dom::reload_xml_system.run_if(|r: Res<ReloadTrigger>| r.0),
            dom::apply_attribute_updates.run_if(|a: Res<AttributeUpdates>| !a.0.is_empty()),
            dom::mark_dirty_system,
            dom::dom_sync_system.run_if(|d: Res<DirtyNodes>| !d.0.is_empty()),
            dom::process_delete_requests.run_if(|del: Res<DeleteRequests>| !del.0.is_empty()),
        ),
    );

    app.add_systems(Update, ui::ui_system);
    app.add_systems(Update, update_entity_counter.run_if(|m: Res<EntityMap>| m.is_changed()));
    app.add_systems(Update, update_fps_counter);
    app.add_systems(Update, camera_keyboard_movement_system.run_if(|rm: Res<RenderMode>| !rm.is_vr));
    app.add_systems(Update, (xr_session_handler, toggle_render_mode));

    app.add_systems(
        Update,
        (js::js_update_snapshots_system, js::js_eval_pending_scripts, js::js_tick_system).chain(),
    );

    app.run();
}

// ─── Setup ───────────────────────────────────────────────────────────────────

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut world: ResMut<ElemenetWorld>,
    mut dom_data: ResMut<VirtualDomData>,
    mut dirty_nodes: ResMut<DirtyNodes>,
    mut log_panel: ResMut<LogPanel>,
    tokio_rt: Res<TokioRuntime>,
    auto_load_config: Res<AutoLoadConfig>,
) {
    commands.spawn((
        Camera3dBundle {
            camera: Camera { order: 0, ..default() },
            transform: Transform::from_xyz(0.0, 3.0, 8.0).looking_at(Vec3::ZERO, Vec3::Y),
            ..default()
        },
        DesktopCamera,
    ));
    commands.spawn(Camera2dBundle {
        camera: Camera { order: 1, ..default() },
        ..default()
    });
    commands.spawn(PointLightBundle {
        transform: Transform::from_xyz(3.0, 8.0, 3.0),
        ..default()
    });

    let cube_mesh = meshes.add(utils::shapes::create_cube());
    let plane_mesh = meshes.add(utils::shapes::create_plane());
    let default_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.5, 0.8, 0.8),
        ..default()
    });
    commands.insert_resource(SharedResources { cube_mesh, plane_mesh, default_material });

    if auto_load_config.enabled {
        log_panel.push_info(format!("Auto-load enabled. Loading: {}", auto_load_config.start_url));
        match dom::load_and_flatten_xml(&mut world.0, &auto_load_config.start_url, &tokio_rt.0, &mut log_panel) {
            Ok((nodes, dirty)) => {
                dom_data.nodes = nodes;
                dirty_nodes.0 = dirty;
                log_panel.push_info("Initial XML loaded.");
            }
            Err(e) => log_panel.push_error(format!("Error loading initial XML: {e}")),
        }
    } else {
        log_panel.push_info("Auto-load disabled.");
    }
}

// ─── Simple systems ───────────────────────────────────────────────────────────

fn update_entity_counter(mut c: ResMut<EntityCounter>, q: Query<Entity>) {
    c.count = q.iter().count();
}

fn update_fps_counter(time: Res<Time>, mut f: ResMut<FpsCounter>) {
    f.frame_count += 1;
    if f.timer.tick(time.delta()).just_finished() {
        f.fps = f.frame_count;
        f.frame_count = 0;
    }
}

fn camera_keyboard_movement_system(
    time: Res<Time>,
    keyboard: Res<ButtonInput<KeyCode>>,
    mut query: Query<&mut Transform, With<Camera3d>>,
) {
    let Ok(mut transform) = query.get_single_mut() else { return; };
    let mut direction = Vec3::ZERO;
    let forward = transform.forward().as_vec3();
    let right = transform.right().as_vec3();
    let up = Vec3::Y;
    let speed = 5.0;

    if keyboard.pressed(KeyCode::KeyW) { direction += forward; }
    if keyboard.pressed(KeyCode::KeyS) { direction -= forward; }
    if keyboard.pressed(KeyCode::KeyD) { direction += right; }
    if keyboard.pressed(KeyCode::KeyA) { direction -= right; }
    if keyboard.pressed(KeyCode::KeyE) { direction += up; }
    if keyboard.pressed(KeyCode::KeyQ) { direction -= up; }

    if direction.length_squared() > 0.0 {
        direction = direction.normalize();
        transform.translation += direction * speed * time.delta_seconds();
    }
}

fn xr_session_handler(
    render_mode: Res<RenderMode>,
    mut state_changed: EventReader<XrStateChanged>,
    mut create_session: EventWriter<XrCreateSessionEvent>,
    mut begin_session: EventWriter<XrBeginSessionEvent>,
    mut end_session: EventWriter<XrEndSessionEvent>,
    mut destroy_session: EventWriter<XrDestroySessionEvent>,
) {
    for XrStateChanged(state) in state_changed.read() {
        match state {
            XrState::Available => { if render_mode.is_vr { create_session.send_default(); } }
            XrState::Ready => { if render_mode.is_vr { begin_session.send_default(); } }
            XrState::Stopping => { end_session.send_default(); }
            XrState::Exiting { .. } => { destroy_session.send_default(); }
            _ => {}
        }
    }
}

fn toggle_render_mode(
    render_mode: Res<RenderMode>,
    xr_state: Res<XrState>,
    mut create_session: EventWriter<XrCreateSessionEvent>,
    mut request_exit: EventWriter<XrRequestExitEvent>,
    mut desktop_cameras: Query<&mut Camera, With<DesktopCamera>>,
) {
    if !render_mode.is_changed() { return; }

    if render_mode.is_vr {
        for mut cam in desktop_cameras.iter_mut() { cam.is_active = false; }
        if *xr_state == XrState::Available { create_session.send_default(); }
    } else {
        for mut cam in desktop_cameras.iter_mut() { cam.is_active = true; }
        if *xr_state == XrState::Running || *xr_state == XrState::Ready {
            request_exit.send_default();
        }
    }
}
