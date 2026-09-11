use std::env;
use std::path::PathBuf;

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
    XrRequestExitEvent, XrSessionCreated, XrSessionPlugin, XrState,
};
use bevy_xr_utils::tracking_utils::{
    suggest_action_bindings, TrackingUtilitiesPlugin, XrTrackedLeftGrip, XrTrackedRightGrip,
};
use luna::desktop_locomotion::DesktopLocomotionPlugin;
use luna::vr_locomotion::VrLocomotionPlugin;

use luna::*;
use luna::{dom, io, js, permissions, touch, ui, utils, ws};
use bevy::prelude::AmbientLight;

// ─── main ────────────────────────────────────────────────────────────────────

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum LunaUpdatePhase {
    HostIngress,
    DomPrepare,
    DomCommit,
    DomFinalize,
    Permissions,
    RenderSync,
    JsSnapshot,
    JsPolicy,
    JsExecute,
    LifecyclePrepare,
    LifecycleUnmount,
    LifecycleMount,
}

fn main() {
    let mut app = App::new();

    let args: Vec<String> = env::args().collect();
    println!("Args: {:?}", &args[1..]);

    let ar_on = args.iter().any(|a| a == "--ar");

    // `--dev-web[=DIR]` lee las páginas internas del árbol de fuentes en vez de
    // las embebidas, para no recompilar por cada ajuste de UI. Sin valor usa el
    // `src/web` de este crate, resuelto en tiempo de compilación: así funciona
    // desde cualquier directorio de trabajo y apunta al árbol del que salió el
    // binario, no al que uno tenga abierto.
    if let Some(arg) = args
        .iter()
        .find(|a| *a == "--dev-web" || a.starts_with("--dev-web="))
    {
        let dir = match arg.split_once('=') {
            Some((_, value)) => PathBuf::from(value),
            None => PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src").join("web"),
        };
        println!("[luna] dev-web: páginas internas desde {}", dir.display());
        luna::routes::set_dev_web_dir(dir);
    }
    let root_config = RootConfig::load();
    let root_shell_url = "luna://root".to_string();
    let initial_home_url = root_config.home_url.clone();
    let initial_render_mode = if ar_on {
        true
    } else {
        root_config.preferred_render_mode == PreferredRenderMode::Vr
    };

    let assets_dir = luna::utils::folder::resolve_assets_dir();
    println!("Assets dir: {}", assets_dir.display());

    let default_plugins = DefaultPlugins
        .set(AssetPlugin {
            file_path: assets_dir.to_string_lossy().into_owned(),
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
    app.add_plugins(luna::audio::SpaceAudioPlugin);
    app.add_plugins(bevy_xr_utils::hand_gizmos::HandGizmosPlugin);
    app.add_plugins(TrackingUtilitiesPlugin);
    app.add_systems(OxrSendActionBindings, suggest_action_bindings);
    app.add_plugins(VrLocomotionPlugin);
    app.add_plugins(DesktopLocomotionPlugin);
    app.add_plugins(luna::player_spawn::PlayerSpawnPlugin);
    app.add_plugins(luna::capture::CapturePlugin);
    app.add_plugins(luna::agent::AgentPlugin);
    app.world_mut().resource_mut::<luna::agent::AgentControl>().enabled =
        root_config.mcp_enabled_on_startup(args.iter().any(|arg| arg == "--mcp"));
    app.add_plugins(luna::surface::SurfacePlugin);
    app.add_systems(XrSessionCreated, spawn_controllers);
    app.add_systems(bevy_mod_xr::session::XrPreDestroySession, cleanup_controllers);
    app.insert_resource(RenderMode {
        is_vr: initial_render_mode,
    });
    app.add_plugins(EguiPlugin);
    app.add_plugins(FrameTimeDiagnosticsPlugin);

    app.insert_resource(VirtualDomData::default());
    app.insert_resource(DirtyNodes::default());
    app.insert_resource(PendingJsAttachNodes::default());
    app.insert_resource(PendingJsFirstRenderNodes::default());
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
    app.insert_resource(TransformUpdates::default());
    app.insert_resource(TransformOnlyDirtyNodes::default());
    app.insert_resource(SpaceHandleTables::default());
    app.insert_resource(js::DomMirror::default());
    app.insert_resource(js::DomMirrorDirty::default());
    app.insert_resource(DevtoolVisible(false));
    app.insert_resource(LogPanel::default());
    app.insert_resource(TokioRuntime(
        Runtime::new().expect("Failed to create Tokio runtime"),
    ));
    app.insert_resource(crate::SkyboxEntity::default());
    app.insert_resource(TextMaterialCache::default());
    app.insert_resource(PrimitiveMaterialCache::default());
    app.insert_resource(RoundedMeshCache::default());
    app.insert_resource(PerformanceStats::default());
    app.insert_resource(JsSnapshotState::default());
    app.insert_resource(DevtoolState::default());
    app.insert_resource(PendingScripts::default());
    app.insert_resource(DeferredSpaceMounts::default());
    app.insert_resource(IoService::default());
    app.insert_resource(ws::WsService::default());
    app.insert_resource(PendingDocumentLoads::default());
    app.insert_resource(DocumentLoadState::default());
    app.insert_resource(NavigationEpoch::default());
    app.insert_resource(ScriptLoadStates::default());
    app.insert_resource(PendingModelLoads::default());
    app.insert_resource(ModelLoadStates::default());
    app.insert_resource(touch::HostToqueHits::default());
    app.init_resource::<touch::HostHoverTargets>();
    app.insert_resource(touch::HostPoseMoveEvents::default());
    app.insert_resource(luna::system_input::HostSystemInputEvents::default());
    app.insert_resource(luna::viewer_pose::ViewerPoseGlobalSnapshot::default());
    app.insert_resource(PermissionDecisionStore::load());
    app.insert_resource(PermissionPromptQueue::default());
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
            vec![SpaceMountRequest::new(1, initial_home_url.clone())]
        } else {
            Vec::new()
        }
    ));
    app.insert_resource(SpaceUnmountQueue::default());
    app.insert_resource(MountedSpaceList(
        if root_config.auto_load_home {
            vec![MountedSpaceEntry {
                tab_id: 1,
                url: initial_home_url.clone(),
                title: initial_home_url.clone(),
            }]
        } else {
            Vec::new()
        }
    ));
    app.insert_resource(NextTabId(if root_config.auto_load_home { 2 } else { 1 }));
    app.insert_resource(ActiveSpaceIndex(
        if root_config.auto_load_home { Some(0) } else { None }
    ));
    app.insert_resource(GlobalDevtoolVisible::default());
    app.insert_resource(ConfigVisible::default());
    app.insert_resource(KeepLogsOnReload::default());

    // Systems
    app.add_systems(Startup, (setup, js::init_js_runtime).chain());

    // El pipeline se expresa por fases, no como una única cadena de sistemas.
    // Dentro de cada fase Bevy puede ejecutar trabajo independiente en paralelo;
    // entre fases se conservan los invariantes JS → SPECS → render y el orden de
    // lifecycle de tabs.
    app.configure_sets(
        Update,
        (
            LunaUpdatePhase::HostIngress,
            LunaUpdatePhase::DomPrepare.after(LunaUpdatePhase::HostIngress),
            LunaUpdatePhase::DomCommit.after(LunaUpdatePhase::DomPrepare),
            LunaUpdatePhase::DomFinalize.after(LunaUpdatePhase::DomCommit),
            LunaUpdatePhase::Permissions.after(LunaUpdatePhase::DomFinalize),
            LunaUpdatePhase::RenderSync.after(LunaUpdatePhase::Permissions),
            LunaUpdatePhase::JsSnapshot.after(LunaUpdatePhase::RenderSync),
            LunaUpdatePhase::JsPolicy.after(LunaUpdatePhase::JsSnapshot),
            LunaUpdatePhase::JsExecute.after(LunaUpdatePhase::JsPolicy),
            LunaUpdatePhase::LifecyclePrepare.after(LunaUpdatePhase::JsExecute),
            LunaUpdatePhase::LifecycleUnmount.after(LunaUpdatePhase::LifecyclePrepare),
            LunaUpdatePhase::LifecycleMount.after(LunaUpdatePhase::LifecycleUnmount),
        ),
    );

    app.add_systems(
        Update,
        (io::poll_io_results_system, ws::poll_ws_results_system)
            .in_set(LunaUpdatePhase::HostIngress),
    );
    app.add_systems(
        Update,
        (
            dom::commit_pending_js_attaches_system
                .run_if(|p: Res<PendingJsAttachNodes>| !p.0.is_empty()),
            dom::request_navigation_system.run_if(|r: Res<ReloadTrigger>| r.0),
        )
            .in_set(LunaUpdatePhase::DomPrepare),
    );
    app.add_systems(
        Update,
        (
            dom::apply_transform_updates.run_if(|u: Res<TransformUpdates>| !u.is_empty()),
            dom::commit_pending_document_load_system
                .run_if(|p: Res<PendingDocumentLoads>| !p.0.is_empty()),
        )
            .chain()
            .in_set(LunaUpdatePhase::DomCommit),
    );
    app.add_systems(
        Update,
        (
            dom::apply_attribute_updates.run_if(|a: Res<AttributeUpdates>| !a.0.is_empty()),
            dom::activate_pending_js_first_render_system
                .run_if(|p: Res<PendingJsFirstRenderNodes>| !p.0.is_empty()),
            dom::process_delete_requests.run_if(|del: Res<DeleteRequests>| !del.0.is_empty()),
            dom::commit_pending_includes_system.run_if(|p: Res<PendingIncludes>| !p.0.is_empty()),
        )
            .chain()
            .in_set(LunaUpdatePhase::DomFinalize),
    );
    app.add_systems(
        Update,
        (
            permissions::rebuild_space_policies_system.run_if(|p: Res<SpacePolicies>| p.dirty),
            permissions::update_active_native_services_system,
        )
            .chain()
            .in_set(LunaUpdatePhase::Permissions),
    );
    app.add_systems(
        Update,
        (dom::dom_sync_system.run_if(|d: Res<DirtyNodes>| !d.0.is_empty()),
            luna::models::poll_model_instances,
            luna::surface::sync_surfaces,
            luna::model_animation::sync_model_animations,
            luna::model_animation::report_clip_completion,
            luna::embedded::sync_embedded_windows)
            .chain().in_set(LunaUpdatePhase::RenderSync),
    );

    app.add_systems(
        Update,
        js::js_update_snapshots_system.in_set(LunaUpdatePhase::JsSnapshot),
    );
    app.add_systems(
        Update,
        (
            js::js_sync_space_permissions_system,
            js::js_auto_inject_resource_scripts_system,
        )
            .in_set(LunaUpdatePhase::JsPolicy),
    );
    app.add_systems(
        Update,
        (js::js_eval_pending_scripts, js::js_tick_system)
            .chain()
            .in_set(LunaUpdatePhase::JsExecute),
    );
    app.add_systems(
        Update,
        (sync_root_mode_resources, ui::flush_deferred_space_mounts_system)
            .in_set(LunaUpdatePhase::LifecyclePrepare),
    );
    app.add_systems(
        Update,
        process_space_unmount_queue
            .run_if(|q: Res<SpaceUnmountQueue>| !q.0.is_empty())
            .in_set(LunaUpdatePhase::LifecycleUnmount),
    );
    app.add_systems(
        Update,
        process_space_mount_queue
            .run_if(|q: Res<SpaceMountQueue>| !q.0.is_empty())
            .in_set(LunaUpdatePhase::LifecycleMount),
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
    app.add_systems(Update, xr_session_handler);
    app.add_systems(
        Update,
        touch::desktop_toque_raycast_system
            .run_if(|rm: Res<RenderMode>| !rm.is_vr),
    );
    app.add_systems(
        Update,
        touch::vr_toque_raycast_system
            .run_if(|rm: Res<RenderMode>| rm.is_vr)
            .run_if(bevy_mod_openxr::openxr_session_running)
            .run_if(resource_exists::<luna::vr_locomotion::LunaLocomotionActions>),
    );
    app.add_systems(
        Update,
        touch::vr_posemove_system
            .run_if(|mode: Res<RenderMode>| mode.is_vr)
            .run_if(bevy_mod_openxr::openxr_session_running)
            .run_if(resource_exists::<luna::vr_locomotion::LunaLocomotionActions>),
    );
    app.add_systems(Update, touch::vr_left_hover_raycast_system
        .run_if(resource_exists::<luna::vr_locomotion::LunaLocomotionActions>)
        .run_if(bevy_mod_openxr::openxr_session_running)
        .run_if(|rm: Res<RenderMode>| rm.is_vr));
    app.add_systems(Update, touch::dispatch_hover_events_to_js
        .after(touch::desktop_toque_raycast_system)
        .after(touch::vr_toque_raycast_system)
        .after(touch::vr_left_hover_raycast_system)
        .before(touch::dispatch_toque_events_to_js));
    app.add_systems(Update, touch::dispatch_toque_events_to_js);
    app.add_systems(Update, touch::dispatch_posemove_events_to_js.run_if(|e: Res<touch::HostPoseMoveEvents>| !e.0.is_empty()));
    app.add_systems(
        Update,
        luna::system_input::dispatch_system_input_events_to_js
            .run_if(|e: Res<luna::system_input::HostSystemInputEvents>| !e.0.is_empty()),
    );
    // Viewer pose: snapshot desktop (VR snapshot vive en vr_locomotion plugin)
    // + propagación a workers JS con cap READ_HMD_POSE.
    app.add_systems(
        Update,
        (
            luna::viewer_pose::update_desktop_viewer_pose,
            luna::viewer_pose::propagate_viewer_pose_to_workers,
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
    
    commands.insert_resource(AmbientLight {
        color: Color::WHITE,
        brightness: 500.0, // subí/bajá este valor
        ..default()
    });

    // bevy_egui renders a window pass after CameraDriver; it needs no 2D camera.
    // An extra active camera also runs mesh visibility over every GLB primitive.
    // commands.spawn(PointLightBundle {
    //     transform: Transform::from_xyz(3.0, 8.0, 3.0),
    //     ..default()
    // });

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
    mut query: Query<&mut Transform, With<DesktopCamera>>,
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

// Reconcile desired mode with current state, including changes made while idle.
// Retry transient create failures without flooding the runtime every frame.
fn xr_session_handler(
    mut render_mode: ResMut<RenderMode>,
    state: Res<XrState>,
    time: Res<Time>,
    mut last: Local<Option<(bool, XrState)>>,
    mut next_attempt: Local<f64>,
    mut exit_requested: Local<bool>,
    mut create_session: EventWriter<XrCreateSessionEvent>,
    mut begin_session: EventWriter<XrBeginSessionEvent>,
    mut end_session: EventWriter<XrEndSessionEvent>,
    mut destroy_session: EventWriter<XrDestroySessionEvent>,
    mut request_exit: EventWriter<XrRequestExitEvent>,
    mut cameras: Query<&mut Camera, With<DesktopCamera>>,
    frame: Option<Res<bevy_mod_openxr::resources::OxrFrameState>>,
    mut controllers: Query<
        &mut Visibility,
        Or<(With<XrTrackedLeftGrip>, With<XrTrackedRightGrip>)>,
    >,
) {
    let vr_running = render_mode.is_vr
        && *state == XrState::Running
        && frame.is_some_and(|frame| frame.should_render);
    for mut visibility in &mut controllers {
        let desired = if vr_running {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if *visibility != desired {
            *visibility = desired;
        }
    }
    for mut camera in &mut cameras {
        if camera.is_active == vr_running {
            camera.is_active = !vr_running;
        }
    }
    let current = (render_mode.is_vr, *state);
    let now = time.elapsed_seconds_f64();
    if *last == Some(current) && now < *next_attempt {
        return;
    }
    *last = Some(current);
    *next_attempt = now + 2.0;
    match *state {
        XrState::Available => {
            *exit_requested = false;
            if render_mode.is_vr {
                create_session.send_default();
            }
        }
        XrState::Ready if render_mode.is_vr => {
            begin_session.send_default();
        }
        XrState::Running if !render_mode.is_vr => {
            *exit_requested = true;
            request_exit.send_default();
        }
        XrState::Stopping => {
            end_session.send_default();
        }
        XrState::Exiting { should_restart } => {
            // Runtime/user exit is not device loss. Respect leaving VR instead
            // of immediately reopening it when cleanup returns Available.
            if !should_restart && !*exit_requested && render_mode.is_vr {
                render_mode.is_vr = false;
            }
            destroy_session.send_default();
        }
        _ => {}
    }
}

fn cleanup_controllers(
    mut commands: Commands,
    controllers: Query<Entity, Or<(With<XrTrackedLeftGrip>, With<XrTrackedRightGrip>)>>,
) {
    for entity in &controllers {
        commands.entity(entity).despawn_recursive();
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
    mut manager: NonSendMut<js::ScriptRuntimeManager>,
    specs_world: Res<ElemenetWorld>,
    mut log_panel: ResMut<LogPanel>,
) {
    let Some(root_id) = find_root_worker_space_id(&specs_world.0, &manager) else {
        if !mount_queue.0.is_empty() {
            log_panel.push_info("[mount-queue] waiting for root worker...");
        }
        return;
    };
    let Some(worker) = manager.contexts.get_mut(&root_id) else {
        return;
    };
    if !worker.root_api_sent {
        if !mount_queue.0.is_empty() {
            log_panel.push_info("[mount-queue] waiting for root API to initialize...");
        }
        return;
    }
    let requests: Vec<SpaceMountRequest> = mount_queue.0.drain(..).collect();
    let mut deferred = Vec::new();
    for request in requests {
        let escaped_url = request.url.replace('\\', "\\\\").replace('\'', "\\'");
        let kind_raw = if request.kind.is_empty() { "spatial" } else { request.kind.as_str() };
        let escaped_kind = kind_raw.replace('\\', "\\\\").replace('\'', "\\'");
        // Grants los decide `dimension.luna.mountSpace` en JS según `kind`:
        //   spatial → navigate_self, read_pose_stream, skybox
        //   app     → navigate_self
        //   app-embedded → navigate_self, ux_embed
        let code = format!(
            "dimension.luna.mountSpace('{}', {{ tabId: {}, kind: '{}' }});",
            escaped_url, request.tab_id, escaped_kind
        );
        let send_result = worker.try_send(js::JsWorkerCommand::EvalScript {
            url: format!("eval://mount/{}", request.url),
            code,
        });
        if send_result.is_ok() {
            worker.needs_tick = true;
        } else if send_result.is_err_and(js::JsWorkerQueueError::is_full) {
            deferred.push(request);
        }
    }
    mount_queue.0.extend(deferred);
}

fn process_space_unmount_queue(
    mut unmount_queue: ResMut<SpaceUnmountQueue>,
    mut manager: NonSendMut<js::ScriptRuntimeManager>,
    specs_world: Res<ElemenetWorld>,
    mut log_panel: ResMut<LogPanel>,
) {
    let Some(root_id) = find_root_worker_space_id(&specs_world.0, &manager) else {
        if !unmount_queue.0.is_empty() {
            log_panel.push_info("[unmount-queue] waiting for root worker...");
        }
        return;
    };
    let Some(worker) = manager.contexts.get_mut(&root_id) else {
        return;
    };
    let requests: Vec<SpaceUnmountRequest> = unmount_queue.0.drain(..).collect();
    let mut deferred = Vec::new();
    for request in requests {
        let code = format!(
            "(() => {{ var list = dimension.luna.listMountedSpaces(); for (var i = 0; i < list.length; i++) {{ if (String(list[i].tabId) === '{}' ) {{ dimension.luna.unmountSpace(list[i].id); break; }} }} }})()",
            request.tab_id
        );
        let send_result = worker.try_send(js::JsWorkerCommand::EvalScript {
            url: format!("eval://unmount/{}", request.url),
            code,
        });
        if send_result.is_ok() {
            worker.needs_tick = true;
        } else if send_result.is_err_and(js::JsWorkerQueueError::is_full) {
            deferred.push(request);
        }
    }
    unmount_queue.0.extend(deferred);
}


fn sync_root_mode_resources(
    render_mode: Res<RenderMode>,
    mut manager: NonSendMut<js::ScriptRuntimeManager>,
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
    let Some(worker) = manager.contexts.get_mut(&root_id) else {
        return;
    };
    if !worker.root_api_sent {
        return;
    }

    let mode = if render_mode.is_vr { "vr" } else { "desktop" };
    let code = format!(
        "dimension.luna.switchMode('{mode}'); dimension.luna.regrantMountedSpaces('{mode}');"
    );
    let send_result = worker.try_send(js::JsWorkerCommand::EvalScript {
        url: format!("eval://mode/{}", mode),
        code,
    });
    if send_result.is_ok() {
        worker.needs_tick = true;
    }

    if send_result.is_ok() {
        *ran_once = true;
    }
}

#[cfg(test)]
mod xr_lifecycle_tests {
    use super::*;
    fn app(state: XrState, vr: bool) -> App {
        let mut app = App::new();
        app.insert_resource(state)
            .insert_resource(RenderMode { is_vr: vr })
            .insert_resource(Time::<()>::default())
            .add_event::<XrCreateSessionEvent>()
            .add_event::<XrBeginSessionEvent>()
            .add_event::<XrEndSessionEvent>()
            .add_event::<XrDestroySessionEvent>()
            .add_event::<XrRequestExitEvent>()
            .add_systems(Update, xr_session_handler);
        app.world_mut()
            .insert_resource(bevy_mod_openxr::resources::OxrFrameState(
                openxr::FrameState {
                    predicted_display_time: openxr::Time::from_nanos(1),
                    predicted_display_period: openxr::Duration::from_nanos(1),
                    should_render: true,
                },
            ));
        app.world_mut().spawn((Camera::default(), DesktopCamera));
        app
    }
    #[test]
    fn switching_to_vr_while_already_ready_begins_session() {
        let mut app = app(XrState::Ready, false);
        app.update();
        assert_eq!(
            app.world().resource::<Events<XrRequestExitEvent>>().len(),
            0
        );
        app.world_mut().resource_mut::<RenderMode>().is_vr = true;
        app.update();
        assert_eq!(
            app.world().resource::<Events<XrBeginSessionEvent>>().len(),
            1
        );
    }
    #[test]
    fn create_retries_are_throttled_and_desktop_remains_visible() {
        let mut app = app(XrState::Available, true);
        app.update();
        assert_eq!(
            app.world().resource::<Events<XrCreateSessionEvent>>().len(),
            1
        );
        app.world_mut()
            .resource_mut::<Events<XrCreateSessionEvent>>()
            .clear();
        app.update();
        assert!(app
            .world()
            .resource::<Events<XrCreateSessionEvent>>()
            .is_empty());
        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(std::time::Duration::from_secs(3));
        app.update();
        assert_eq!(
            app.world().resource::<Events<XrCreateSessionEvent>>().len(),
            1
        );
        let world = app.world_mut();
        assert!(world.query::<&Camera>().single(world).is_active);
    }
    #[test]
    fn stop_idle_ready_cycle_and_exit_are_reconciled_without_transition_events() {
        let mut app = app(XrState::Stopping, true);
        app.update();
        assert_eq!(app.world().resource::<Events<XrEndSessionEvent>>().len(), 1);
        app.world_mut().insert_resource(XrState::Idle);
        app.update();
        app.world_mut().insert_resource(XrState::Ready);
        app.update();
        assert_eq!(
            app.world().resource::<Events<XrBeginSessionEvent>>().len(),
            1
        );
        app.world_mut().insert_resource(XrState::Running);
        app.update();
        {
            let world = app.world_mut();
            assert!(!world.query::<&Camera>().single(world).is_active);
        }
        app.world_mut().resource_mut::<RenderMode>().is_vr = false;
        app.update();
        assert_eq!(
            app.world().resource::<Events<XrRequestExitEvent>>().len(),
            1
        );
        app.world_mut().insert_resource(XrState::Exiting {
            should_restart: true,
        });
        app.update();
        assert_eq!(
            app.world()
                .resource::<Events<XrDestroySessionEvent>>()
                .len(),
            1
        );
    }
    #[test]
    fn runtime_exit_does_not_immediately_reopen_vr() {
        let mut app = app(
            XrState::Exiting {
                should_restart: false,
            },
            true,
        );
        app.update();
        assert!(!app.world().resource::<RenderMode>().is_vr);
        app.world_mut().insert_resource(XrState::Available);
        app.update();
        assert!(app
            .world()
            .resource::<Events<XrCreateSessionEvent>>()
            .is_empty());
    }
    #[test]
    fn rapid_desktop_vr_toggle_preserves_latest_intent_during_exit() {
        let mut app = app(XrState::Running, false);
        app.update();
        app.world_mut().resource_mut::<RenderMode>().is_vr = true;
        app.world_mut().insert_resource(XrState::Exiting {
            should_restart: false,
        });
        app.update();
        assert!(app.world().resource::<RenderMode>().is_vr);
        app.world_mut().insert_resource(XrState::Available);
        app.update();
        assert_eq!(
            app.world().resource::<Events<XrCreateSessionEvent>>().len(),
            1
        );
    }
    #[test]
    fn controller_cleanup_leaves_no_duplicate_targets() {
        let mut app = App::new();
        app.add_systems(Update, cleanup_controllers);
        for _ in 0..3 {
            app.world_mut().spawn(XrTrackedLeftGrip);
            app.world_mut().spawn(XrTrackedRightGrip);
            app.update();
            let world = app.world_mut();
            assert_eq!(world.query::<&XrTrackedLeftGrip>().iter(world).count(), 0);
        }
    }
}
