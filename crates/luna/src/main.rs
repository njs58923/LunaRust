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

use bevy_mod_openxr::action_binding::OxrSendActionBindings;
use bevy_mod_openxr::add_xr_plugins;
use bevy_mod_xr::session::{
    XrBeginSessionEvent, XrCreateSessionEvent, XrDestroySessionEvent, XrEndSessionEvent,
    XrRequestExitEvent, XrSessionCreated, XrSessionPlugin, XrState, XrStateChanged,
};
use bevy_xr_utils::tracking_utils::{
    suggest_action_bindings, TrackingUtilitiesPlugin, XrTrackedLeftGrip, XrTrackedRightGrip,
};
use luna::vr_locomotion::VrLocomotionPlugin;

use luna::*;
use luna::{dom, io, js, permissions, touch, ui, utils};

// ─── main ────────────────────────────────────────────────────────────────────

fn main() {
    let mut app = App::new();

    let args: Vec<String> = env::args().collect();
    println!("Args: {:?}", &args[1..]);

    let ar_on = args.iter().any(|a| a == "--ar");
    let root_config = RootConfig::load();
    let root_shell_url = "luna://root".to_string();
    let initial_home_url = root_config.home_url.clone();
    let initial_render_mode = if ar_on {
        true
    } else {
        root_config.preferred_render_mode == PreferredRenderMode::Vr
    };

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

    app.add_plugins(add_xr_plugins(default_plugins).set(XrSessionPlugin { auto_handle: false }));
    app.add_plugins(bevy_xr_utils::hand_gizmos::HandGizmosPlugin);
    app.add_plugins(TrackingUtilitiesPlugin);
    app.add_systems(OxrSendActionBindings, suggest_action_bindings);
    app.add_plugins(VrLocomotionPlugin);
    app.add_systems(XrSessionCreated, spawn_controllers);
    app.insert_resource(RenderMode {
        is_vr: initial_render_mode,
    });
    app.add_plugins(EguiPlugin);
    app.add_plugins(FrameTimeDiagnosticsPlugin);

    app.insert_resource(VirtualDomData::default());
    app.insert_resource(DirtyNodes::default());
    app.insert_resource(EntityMap::default());
    app.insert_resource(ElemenetWorld(build_world()));
    app.insert_resource(EntityCounter::default());
    app.insert_resource(FpsCounter::default());
    app.insert_resource(CurrentUrl(root_shell_url));
    app.insert_resource(AddressBarState(
        if root_config.auto_load_home {
            initial_home_url.clone()
        } else {
            String::new()
        }
    ));
    app.insert_resource(root_config.clone());
    app.insert_resource(ReloadTrigger(false));
    app.insert_resource(AttributeUpdates::default());
    app.insert_resource(DeleteRequests::default());
    app.insert_resource(SpaceHandleTables::default());
    app.insert_resource(DevtoolVisible(false));
    app.insert_resource(LogPanel::default());
    app.insert_resource(TokioRuntime(
        Runtime::new().expect("Failed to create Tokio runtime"),
    ));
    app.insert_resource(ModelCache::default());
    app.insert_resource(TextMaterialCache::default());
    app.insert_resource(PrimitiveMaterialCache::default());
    app.insert_resource(PerformanceStats::default());
    app.insert_resource(JsSnapshotState::default());
    app.insert_resource(DevtoolState::default());
    app.insert_resource(PendingScripts::default());
    app.insert_resource(IoService::default());
    app.insert_resource(PendingDocumentLoads::default());
    app.insert_resource(DocumentLoadState::default());
    app.insert_resource(NavigationEpoch::default());
    app.insert_resource(ScriptLoadStates::default());
    app.insert_resource(PendingModelLoads::default());
    app.insert_resource(ModelLoadStates::default());
    app.insert_resource(touch::ToqueRawEvents::default());
    app.insert_resource(SpacePolicies::default());
    app.insert_resource(ActiveNativeServices::default());
    app.insert_resource(permissions::SpacePolicyHistory {
        entries: Vec::new(),
        max_entries: 300,
    });
    app.insert_resource(PendingIncludes::default());
    app.insert_resource(IncludeLoadStates::default());
    app.insert_resource(SpaceMountQueue(
        if root_config.auto_load_home {
            vec![initial_home_url.clone()]
        } else {
            Vec::new()
        }
    ));
    app.insert_resource(SpaceUnmountQueue::default());
    app.insert_resource(MountedSpaceList(
        if root_config.auto_load_home {
            vec![MountedSpaceEntry {
                url: initial_home_url.clone(),
                title: initial_home_url.clone(),
                is_home: true,
            }]
        } else {
            Vec::new()
        }
    ));
    app.insert_resource(ActiveSpaceIndex(
        if root_config.auto_load_home { Some(0) } else { None }
    ));
    app.insert_resource(GlobalDevtoolVisible::default());

    // Systems
    app.add_systems(Startup, (setup, js::init_js_runtime).chain());

    app.add_systems(
        Update,
        (
            io::poll_io_results_system,
            dom::request_navigation_system.run_if(|r: Res<ReloadTrigger>| r.0),
            dom::commit_pending_document_load_system
                .run_if(|p: Res<PendingDocumentLoads>| !p.0.is_empty()),
            dom::apply_attribute_updates.run_if(|a: Res<AttributeUpdates>| !a.0.is_empty()),
            dom::process_delete_requests.run_if(|del: Res<DeleteRequests>| !del.0.is_empty()),
            dom::commit_pending_includes_system.run_if(|p: Res<PendingIncludes>| !p.0.is_empty()),
            permissions::rebuild_space_policies_system.run_if(|p: Res<SpacePolicies>| p.dirty),
            permissions::update_active_native_services_system,
            dom::mark_dirty_system,
            dom::dom_sync_system.run_if(|d: Res<DirtyNodes>| !d.0.is_empty()),
        ),
    );

    app.add_systems(Update, ui::ui_system);
    app.add_systems(
        Update,
        update_entity_counter.run_if(|m: Res<EntityMap>| m.is_changed()),
    );
    app.add_systems(Update, update_fps_counter);
    app.add_systems(
        Update,
        camera_keyboard_movement_system
            .run_if(|rm: Res<RenderMode>| !rm.is_vr)
            .run_if(permissions::desktop_camera_control_enabled),
    );
    app.add_systems(Update, (xr_session_handler, toggle_render_mode));
    app.add_systems(
        Update,
        touch::desktop_toque_raycast_system
            .run_if(|rm: Res<RenderMode>| !rm.is_vr)
            .run_if(permissions::desktop_toque_source_enabled),
    );
    app.add_systems(
        Update,
        touch::vr_toque_raycast_system
            .run_if(bevy_mod_openxr::openxr_session_running)
            .run_if(resource_exists::<luna::vr_locomotion::LunaLocomotionActions>)
            .run_if(permissions::vr_toque_source_enabled),
    );
    app.add_systems(Update, touch::dispatch_toque_raw_events_to_js);

    app.add_systems(
        Update,
        (
            js::js_update_snapshots_system,
            js::js_sync_space_permissions_system,
            js::js_auto_inject_resource_scripts_system,
            js::js_eval_pending_scripts,
            js::js_tick_system,
            sync_root_mode_resources,
            process_space_unmount_queue.run_if(|q: Res<SpaceUnmountQueue>| !q.0.is_empty()),
            process_space_mount_queue.run_if(|q: Res<SpaceMountQueue>| !q.0.is_empty()),
        )
            .chain(),
    );

    app.run();
}

// ─── Setup ───────────────────────────────────────────────────────────────────

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut log_panel: ResMut<LogPanel>,
    mut reload_trigger: ResMut<ReloadTrigger>,
    root_config: Res<RootConfig>,
) {
    commands.spawn((
        Camera3dBundle {
            camera: Camera {
                order: 0,
                ..default()
            },
            transform: Transform::from_xyz(0.0, 3.0, 8.0).looking_at(Vec3::ZERO, Vec3::Y),
            ..default()
        },
        DesktopCamera,
    ));
    commands.spawn(Camera2dBundle {
        camera: Camera {
            order: 1,
            ..default()
        },
        ..default()
    });
    commands.spawn(PointLightBundle {
        transform: Transform::from_xyz(3.0, 8.0, 3.0),
        ..default()
    });

    let cube_mesh = meshes.add(utils::shapes::create_cube());
    let plane_mesh = meshes.add(utils::shapes::create_plane());
    let sphere_mesh = meshes.add(Sphere::new(0.5));
    let cylinder_mesh = meshes.add(Cylinder::new(0.5, 1.0));
    let default_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.5, 0.8, 0.8),
        ..default()
    });
    commands.insert_resource(SharedResources {
        cube_mesh,
        plane_mesh,
        sphere_mesh,
        cylinder_mesh,
        default_material,
    });

    reload_trigger.0 = true;
    log_panel.push_info("Queued initial root shell load: luna://root");

    if root_config.auto_load_home {
        log_panel.push_info(format!(
            "Queued initial home tab mount: {}",
            root_config.home_url
        ));
    } else {
        log_panel.push_info("Auto-load home tab disabled.");
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
    let Ok(mut transform) = query.get_single_mut() else {
        return;
    };
    let mut direction = Vec3::ZERO;
    let forward = transform.forward().as_vec3();
    let right = transform.right().as_vec3();
    let up = Vec3::Y;
    let speed = 5.0;

    if keyboard.pressed(KeyCode::KeyW) {
        direction += forward;
    }
    if keyboard.pressed(KeyCode::KeyS) {
        direction -= forward;
    }
    if keyboard.pressed(KeyCode::KeyD) {
        direction += right;
    }
    if keyboard.pressed(KeyCode::KeyA) {
        direction -= right;
    }
    if keyboard.pressed(KeyCode::KeyE) {
        direction += up;
    }
    if keyboard.pressed(KeyCode::KeyQ) {
        direction -= up;
    }

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
            XrState::Available => {
                if render_mode.is_vr {
                    create_session.send_default();
                }
            }
            XrState::Ready => {
                if render_mode.is_vr {
                    begin_session.send_default();
                }
            }
            XrState::Stopping => {
                end_session.send_default();
            }
            XrState::Exiting { .. } => {
                destroy_session.send_default();
            }
            _ => {}
        }
    }
}

fn spawn_controllers(
    mut cmds: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    root: Query<Entity, With<bevy_mod_xr::session::XrTrackingRoot>>,
) {
    let mesh = meshes.add(Cuboid::new(0.1, 0.1, 0.05));
    let mat = materials.add(Color::srgb_u8(124, 144, 255));
    let left = cmds
        .spawn((
            PbrBundle {
                mesh: mesh.clone(),
                material: mat.clone(),
                ..default()
            },
            XrTrackedLeftGrip,
        ))
        .id();
    let right = cmds
        .spawn((
            PbrBundle {
                mesh,
                material: mat,
                ..default()
            },
            XrTrackedRightGrip,
        ))
        .id();
    if let Ok(root_entity) = root.get_single() {
        cmds.entity(root_entity).push_children(&[left, right]);
    }
}

/// Finds the root space worker (space with id="luna_root") and returns its space_id.
fn find_root_worker_space_id(
    specs_world: &specs::World,
    manager: &js::ScriptRuntimeManager,
) -> Option<u32> {
    use specs::WorldExt;
    use virtual_dom::dom::element::Attrs;
    let attrs = specs_world.read_storage::<Attrs>();
    for &space_id in manager.contexts.keys() {
        let ent = specs_world.entities().entity(space_id);
        if let Some(a) = attrs.get(ent) {
            if a.0.get("id").map(|v| v.as_str()) == Some("luna_root") {
                return Some(space_id);
            }
        }
    }
    None
}

fn process_space_mount_queue(
    mut mount_queue: ResMut<SpaceMountQueue>,
    manager: NonSendMut<js::ScriptRuntimeManager>,
    specs_world: Res<ElemenetWorld>,
    render_mode: Res<RenderMode>,
    mut log_panel: ResMut<LogPanel>,
) {
    let Some(root_id) = find_root_worker_space_id(&specs_world.0, &manager) else {
        if !mount_queue.0.is_empty() {
            log_panel.push_info("[mount-queue] waiting for root worker...");
        }
        return;
    };
    let Some(worker) = manager.contexts.get(&root_id) else {
        return;
    };
    let urls: Vec<String> = mount_queue.0.drain(..).collect();
    let grants = if render_mode.is_vr {
        "['controller_vr','navigate_self']"
    } else {
        "['controller_desktop','navigate_self']"
    };
    for url in urls {
        let escaped = url.replace('\\', "\\\\").replace('\'', "\\'");
        let code = format!(
            "dimension.luna.mountSpace('{}', {{ grants: {} }});",
            escaped, grants
        );
        let _ = worker.cmd_tx.send(js::JsWorkerCommand::EvalScript {
            url: format!("eval://mount/{}", url),
            code,
        });
    }
}

fn process_space_unmount_queue(
    mut unmount_queue: ResMut<SpaceUnmountQueue>,
    manager: NonSendMut<js::ScriptRuntimeManager>,
    specs_world: Res<ElemenetWorld>,
    mut log_panel: ResMut<LogPanel>,
) {
    let Some(root_id) = find_root_worker_space_id(&specs_world.0, &manager) else {
        if !unmount_queue.0.is_empty() {
            log_panel.push_info("[unmount-queue] waiting for root worker...");
        }
        return;
    };
    let Some(worker) = manager.contexts.get(&root_id) else {
        return;
    };
    let urls: Vec<String> = unmount_queue.0.drain(..).collect();
    for url in urls {
        let escaped = url.replace('\\', "\\\\").replace('\'', "\\'");
        let code = format!(
            "(() => {{ var list = dimension.luna.listMountedSpaces(); for (var i = 0; i < list.length; i++) {{ if (list[i].url === '{}') {{ dimension.luna.unmountSpace(list[i].id); break; }} }} }})()",
            escaped
        );
        let _ = worker.cmd_tx.send(js::JsWorkerCommand::EvalScript {
            url: format!("eval://unmount/{}", url),
            code,
        });
    }
}

fn toggle_render_mode(
    render_mode: Res<RenderMode>,
    xr_state: Res<XrState>,
    mut create_session: EventWriter<XrCreateSessionEvent>,
    mut request_exit: EventWriter<XrRequestExitEvent>,
    mut desktop_cameras: Query<&mut Camera, With<DesktopCamera>>,
) {
    if !render_mode.is_changed() {
        return;
    }

    if render_mode.is_vr {
        for mut cam in desktop_cameras.iter_mut() {
            cam.is_active = false;
        }
        if *xr_state == XrState::Available {
            create_session.send_default();
        }
    } else {
        for mut cam in desktop_cameras.iter_mut() {
            cam.is_active = true;
        }
        if *xr_state == XrState::Running || *xr_state == XrState::Ready {
            request_exit.send_default();
        }
    }
}

fn sync_root_mode_resources(
    render_mode: Res<RenderMode>,
    manager: NonSendMut<js::ScriptRuntimeManager>,
    specs_world: Res<ElemenetWorld>,
    mut ran_once: Local<bool>,
) {
    let should_run = !*ran_once || render_mode.is_changed();
    if !should_run {
        return;
    }

    let Some(root_id) = find_root_worker_space_id(&specs_world.0, &manager) else {
        return;
    };
    let Some(worker) = manager.contexts.get(&root_id) else {
        return;
    };

    let mode = if render_mode.is_vr { "vr" } else { "desktop" };
    let code = format!(
        "dimension.luna.switchMode('{mode}'); dimension.luna.regrantMountedSpaces('{mode}');"
    );
    let _ = worker.cmd_tx.send(js::JsWorkerCommand::EvalScript {
        url: format!("eval://mode/{}", mode),
        code,
    });

    *ran_once = true;
}
