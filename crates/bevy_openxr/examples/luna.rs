use bevy::{
    asset::AssetPlugin,
    diagnostic::FrameTimeDiagnosticsPlugin,
    ecs::system::SystemParam,
    prelude::*,
    window::{PresentMode, PrimaryWindow, Window},
};
use fontdue::{
    layout::{CoordinateSystem, Layout, LayoutSettings, TextStyle},
    Font, FontSettings,
};
use bevy_egui::{egui, EguiContexts, EguiPlugin};
use specs::{Entity as SpecEntity, Join, ReadStorage, World as SpecWorld, WorldExt};
use std::{
    collections::HashMap,
    env::{self},
    fs,
    path::{Path, PathBuf},
    sync::{mpsc, OnceLock},
    thread::{self, JoinHandle},
    time::Instant,
};
use tokio::runtime::Runtime;
use url::Url;

use base64::{engine::general_purpose::STANDARD as Base64Engine, Engine as _};
use virtual_dom::{
    dom::{
        element::{build_world, Attrs, Hierarchy, Tag, Transform2},
        hsml::{Model, Include, Script},
        TRANSFORM_POSITION, TRANSFORM_ROTATION, TRANSFORM_SCALE
    },
    load_xml_from_url, parse_xml,
};
use js_runtime::Engine as JsEngine;
use anyhow::Result;
use bevy::{gltf::GltfPlugin, gltf::GltfLoaderSettings, prelude::*};
use bevy_mod_openxr::add_xr_plugins;
use bevy_mod_xr::session::{
    XrBeginSessionEvent, XrCreateSessionEvent, XrDestroySessionEvent, XrEndSessionEvent,
    XrRequestExitEvent, XrSessionPlugin, XrState, XrStateChanged,
};
use std::collections::HashSet;
use std::f32::consts::*;

// Módulos ficticios
mod render;
mod utils;
mod virtual_routes;
use render::apply_transform;
use utils::shapes;
use utils::folder;
use virtual_routes::VIRTUAL_ROUTES;

// --------------------------------------------------------------------------------------
// LOG
// --------------------------------------------------------------------------------------
#[derive(Debug, Clone, Copy)]
enum LogLevel {
    Error,
    Warn,
    Info,
}

#[derive(Debug, Clone)]
struct LogEntry {
    level: LogLevel,
    message: String,
}

impl LogEntry {
    fn new(level: LogLevel, message: impl Into<String>) -> Self {
        Self {
            level,
            message: message.into(),
        }
    }
}

#[derive(Resource, Default)]
struct LogPanel {
    logs: Vec<LogEntry>,
}

impl LogPanel {
    fn push_error(&mut self, msg: impl Into<String>) {
        self.push(LogLevel::Error, msg);
    }
    fn push_warn(&mut self, msg: impl Into<String>) {
        self.push(LogLevel::Warn, msg);
    }
    fn push_info(&mut self, msg: impl Into<String>) {
        self.push(LogLevel::Info, msg);
    }
    fn push(&mut self, level: LogLevel, msg: impl Into<String>) {
        const MAX_LOGS: usize = 300;
        if self.logs.len() >= MAX_LOGS {
            self.logs.remove(0);
        }
        self.logs.push(LogEntry::new(level, msg));
    }
    fn clear(&mut self) {
        self.logs.clear();
    }
}

// --------------------------------------------------------------------------------------
// RECURSOS
// --------------------------------------------------------------------------------------
#[derive(Resource, Default)]
struct VirtualDomData {
    pub nodes: HashMap<u32, SpecEntity>,
}

#[derive(Resource, Default)]
struct DirtyNodes(Vec<u32>);

#[derive(Resource, Default)]
struct ElemenetWorld(SpecWorld);

#[derive(Resource, Default)]
struct EntityMap(HashMap<u32, Entity>);

#[derive(Resource, Default)]
struct EntityCounter {
    count: usize,
}

#[derive(Resource)]
struct FpsCounter {
    timer: Timer,
    frame_count: u32,
    fps: u32,
}
impl Default for FpsCounter {
    fn default() -> Self {
        Self {
            timer: Timer::from_seconds(1.0, TimerMode::Repeating),
            frame_count: 0,
            fps: 0,
        }
    }
}

#[derive(Resource, Default)]
struct CurrentUrl(String);

#[derive(Resource)]
struct AutoLoadConfig {
    enabled: bool,
    start_url: String,
}

impl Default for AutoLoadConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            start_url: "luna://home".to_string(),
        }
    }
}

// SystemParam bundle to reduce parameter count in ui_system
#[derive(SystemParam)]
struct UiSystemParams<'w> {
    entity_counter: Res<'w, EntityCounter>,
    fps_counter: Res<'w, FpsCounter>,
    perf_stats: Res<'w, PerformanceStats>,
    auto_load_config: ResMut<'w, AutoLoadConfig>,
}

#[derive(Resource)]
struct RenderMode {
    is_vr: bool,
}

#[derive(Component)]
struct DesktopCamera;

#[derive(Resource, Default)]
struct DevtoolVisible(bool);

#[derive(Resource, Default)]
struct ReloadTrigger(bool);

#[derive(Resource, Default)]
struct AttributeUpdates(Vec<(u32, String, String)>);

#[derive(Component)]
struct Dirty;

#[derive(Resource, Default)]
struct DeleteRequests(Vec<u32>);

#[derive(Resource)]
struct SharedResources {
    cube_mesh: Handle<Mesh>,
    plane_mesh: Handle<Mesh>,
    default_material: Handle<StandardMaterial>,
}

// Recurso con el runtime de Tokio
#[derive(Resource)]
struct TokioRuntime(Runtime);

// CACHE: URL -> path local
#[derive(Resource, Default)]
struct ModelCache {
    cache: HashMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct TextMaterialKey {
    value: String,
    size_bits: u32,
    color_key: String,
}

#[derive(Resource, Default)]
struct TextMaterialCache {
    materials: HashMap<TextMaterialKey, Handle<StandardMaterial>>,
}

#[derive(SystemParam)]
struct TextRenderParams<'w> {
    materials: ResMut<'w, Assets<StandardMaterial>>,
    images: ResMut<'w, Assets<Image>>,
    text_material_cache: ResMut<'w, TextMaterialCache>,
}

/// Medición de performance
#[derive(Resource, Default)]
struct PerformanceStats {
    dom_sync_ms: f32,
}

/// JS engine wrapper (NonSend because V8 is single-threaded)
struct SpaceScriptContext {
    engine: JsEngine,
    loaded_scripts: HashSet<String>,
}

#[derive(Clone)]
struct SpaceSnapshots {
    attr_snap: HashMap<i32, HashMap<String, String>>,
    tag_snap: HashMap<i32, String>,
    positions: HashMap<i32, js_runtime::Vec3>,
    rotations: HashMap<i32, js_runtime::Vec3>,
    scales: HashMap<i32, js_runtime::Vec3>,
    global_positions: HashMap<i32, js_runtime::Vec3>,
    parents: HashMap<i32, i32>,
    children: HashMap<i32, Vec<i32>>,
}

struct JsTickData {
    logs: Vec<(String, String)>,
    attr_updates: Vec<(i32, String, String)>,
    pos_updates: Vec<(i32, js_runtime::Vec3)>,
    rot_updates: Vec<(i32, js_runtime::Vec3)>,
    scale_updates: Vec<(i32, js_runtime::Vec3)>,
    creation_queue: Vec<(i32, String)>,
    hierarchy_queue: Vec<(i32, i32)>,
    remove_queue: Vec<i32>,
    fetch_queue: Vec<(i32, String)>,
    navigate_queue: Vec<String>,
}

enum JsWorkerCommand {
    UpdateSnapshots(SpaceSnapshots),
    EvalScript { url: String, code: String },
    Tick { elapsed_ms: f64 },
    PushElementCreationResults(Vec<(i32, i32)>),
    PushFetchResults(Vec<(i32, std::result::Result<String, String>)>),
    Shutdown,
}

enum JsWorkerEvent {
    EvalResult {
        url: String,
        already_loaded: bool,
        error: Option<String>,
    },
    TickData(JsTickData),
    WorkerError(String),
}

struct SpaceScriptWorker {
    cmd_tx: mpsc::Sender<JsWorkerCommand>,
    event_rx: mpsc::Receiver<JsWorkerEvent>,
    join: Option<JoinHandle<()>>,
}

/// Runtime manager: one JS context/isolate per <space> node.
#[derive(Default)]
struct ScriptRuntimeManager {
    contexts: HashMap<u32, SpaceScriptWorker>,
}

impl Drop for ScriptRuntimeManager {
    fn drop(&mut self) {
        for worker in self.contexts.values_mut() {
            stop_space_worker(worker);
        }
    }
}

/// Scripts pending evaluation: Vec<(space_id, url, code)>
#[derive(Resource, Default)]
struct PendingScripts(Vec<(u32, String, String)>);

// --------------------------------------------------------------------------------------
// PESTAÑAS DEL DEVTOOL
// --------------------------------------------------------------------------------------
#[derive(PartialEq, Eq)]
enum DevtoolTab {
    Status,
    Hsml,
    Logs,
    Redes,
}

#[derive(Resource)]
struct DevtoolState {
    active_tab: DevtoolTab,
}

impl Default for DevtoolState {
    fn default() -> Self {
        DevtoolState {
            active_tab: DevtoolTab::Status,
        }
    }
}


// --------------------------------------------------------------------------------------
// MAIN
// --------------------------------------------------------------------------------------
fn main() {
    let mut app = App::new();

    // ── Variables ────────────────────────────────────────────────────────────────

    let mut ar_on = false;
    let start_url = "luna://home".to_string();
    let devtools_on = false;

    // ── Plugins ────────────────────────────────────────────────────────────────


    let args: Vec<String> = env::args().collect();
    // args[0] = nombre del binario (por ej. "luna")
    println!("Args: {:?}", &args[1..]);

    if args.iter().any(|a| a == "--ar") {
        ar_on = true;
    }

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
        add_xr_plugins(default_plugins)
            .set(XrSessionPlugin { auto_handle: false }),
    );
    app.add_plugins(bevy_xr_utils::hand_gizmos::HandGizmosPlugin);
    app.insert_resource(RenderMode { is_vr: ar_on });


    app.add_plugins(EguiPlugin);
    app.add_plugins(FrameTimeDiagnosticsPlugin);

    // ── Recursos ──────────────────────────────────────────────────────────────
    app.insert_resource(VirtualDomData::default());
    app.insert_resource(DirtyNodes::default());
    app.insert_resource(EntityMap::default());
    app.insert_resource(ElemenetWorld(build_world()));
    app.insert_resource(EntityCounter::default());
    app.insert_resource(FpsCounter::default());

    // Auto-load config
    let auto_load_config = AutoLoadConfig::default();
    let initial_url = if auto_load_config.enabled {
        auto_load_config.start_url.clone()
    } else {
        start_url.clone()
    };

    app.insert_resource(CurrentUrl(initial_url));
    app.insert_resource(AutoLoadConfig::default());
    app.insert_resource(ReloadTrigger(false));
    app.insert_resource(AttributeUpdates::default());
    app.insert_resource(DeleteRequests::default());
    app.insert_resource(DevtoolVisible(devtools_on));
    app.insert_resource(LogPanel::default());

    // Runtime
    app.insert_resource(TokioRuntime(
        Runtime::new().expect("No se pudo crear Tokio"),
    ));

    // Cache
    app.insert_resource(ModelCache::default());
    app.insert_resource(TextMaterialCache::default());
    // Stats
    app.insert_resource(PerformanceStats::default());

    // Estado del devtool
    app.insert_resource(DevtoolState::default());

    // JS Runtime resources (engine created in startup system)
    app.insert_resource(PendingScripts::default());

    // ── Sistemas ──────────────────────────────────────────────────────────────
    app.add_systems(Startup, (setup, init_js_runtime).chain());

    // DOM/XML systems
    app.add_systems(
        Update,
        (
            // Recarga de XML
            reload_xml_system.run_if(|r: Res<ReloadTrigger>| r.0),
            // Aplicar updates a atributos
            apply_attribute_updates
                .run_if(|a: Res<AttributeUpdates>| !a.0.is_empty()),
            // Marcar dirty
            mark_dirty_system,
            // Sincronizar con Bevy solo si hay nodos dirty
            dom_sync_system.run_if(|d: Res<DirtyNodes>| !d.0.is_empty()),
            // Borrar
            process_delete_requests
                .run_if(|del: Res<DeleteRequests>| !del.0.is_empty()),
        ),
    );

    // UI, stats, and input systems
    app.add_systems(Update, ui_system);
    app.add_systems(
        Update,
        update_entity_counter.run_if(|m: Res<EntityMap>| m.is_changed()),
    );
    app.add_systems(Update, update_fps_counter);
    app.add_systems(
        Update,
        camera_keyboard_movement_system.run_if(|rm: Res<RenderMode>| !rm.is_vr),
    );
    app.add_systems(Update, (xr_session_handler, toggle_render_mode));

    // JS systems - MUST run on main thread (V8 is !Send)
    // Registered separately to ensure single-threaded execution
    app.add_systems(
        Update,
        (
            js_update_snapshots_system,
            js_eval_pending_scripts,
            js_tick_system,
        )
            .chain(), // Force sequential execution on main thread
    );

    // ── Run ───────────────────────────────────────────────────────────────────
    app.run();
}

// --------------------------------------------------------------------------------------
// INIT JS RUNTIME
// --------------------------------------------------------------------------------------
fn init_js_runtime(
    world: &mut World,
) {
    let mut log_panel = world.resource_mut::<LogPanel>();
    log_panel.push_info("[JS] Initializing V8 runtime...");

    // Initialize V8 platform (safe to call multiple times)
    js_runtime::init_v8_platform();

    log_panel.push_info("[JS] Creating runtime manager (isolates will be created per <space>)...");
    drop(log_panel); // Release borrow before inserting NonSend resource

    world.insert_non_send_resource(ScriptRuntimeManager::default());

    let mut log_panel = world.resource_mut::<LogPanel>();
    log_panel.push_info("[JS] Runtime initialization complete");
}

// --------------------------------------------------------------------------------------
// SETUP
// --------------------------------------------------------------------------------------
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
    // Cámara 3D (desktop)
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

    // Cámara 2D
    commands.spawn(Camera2dBundle {
        camera: Camera {
            order: 1,
            ..default()
        },
        ..default()
    });

    // Luz
    commands.spawn(PointLightBundle {
        transform: Transform::from_xyz(3.0, 8.0, 3.0),
        ..default()
    });

    // Recursos
    let cube_mesh = meshes.add(shapes::create_cube());
    let plane_mesh = meshes.add(shapes::create_plane());
    let default_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.5, 0.8, 0.8),
        ..default()
    });
    commands.insert_resource(SharedResources {
        cube_mesh,
        plane_mesh,
        default_material,
    });

    // Cargar XML inicial si auto-load está habilitado
    if auto_load_config.enabled {
        log_panel.push_info(format!("Auto-load habilitado. Cargando: {}", auto_load_config.start_url));

        match load_and_flatten_xml(
            &mut world.0,
            &auto_load_config.start_url,
            &tokio_rt.0,
            &mut log_panel,
        ) {
            Ok((nodes, dirty)) => {
                dom_data.nodes = nodes;
                dirty_nodes.0 = dirty;
                log_panel.push_info("XML inicial cargado correctamente.");
            }
            Err(e) => {
                log_panel.push_error(format!("Error al cargar XML inicial: {e}"));
            }
        }
    } else {
        log_panel.push_info("Auto-load deshabilitado. No se carga contenido inicial.");
    }
}

// --------------------------------------------------------------------------------------
// RELOAD XML
// --------------------------------------------------------------------------------------
fn reload_xml_system(
    mut world: ResMut<ElemenetWorld>,
    mut dom_data: ResMut<VirtualDomData>,
    mut dirty_nodes: ResMut<DirtyNodes>,
    mut commands: Commands,
    mut entity_map: ResMut<EntityMap>,
    url: Res<CurrentUrl>,
    mut reload_trigger: ResMut<ReloadTrigger>,
    mut log_panel: ResMut<LogPanel>,
    tokio_rt: Res<TokioRuntime>,
) {
    log_panel.push_info(format!("Iniciando recarga de XML desde: {}", url.0));

    match load_and_flatten_xml(&mut world.0, &url.0, &tokio_rt.0, &mut log_panel) {
        Ok((new_nodes, new_dirty)) => {
            // Borrar viejos
            let old_nodes: Vec<u32> = dom_data.nodes.keys().cloned().collect();
            let mut to_delete = Vec::new();
            for old_id in old_nodes {
                if !new_nodes.contains_key(&old_id) {
                    if let Some(ent) = entity_map.0.remove(&old_id) {
                        commands.entity(ent).despawn_recursive();
                    }
                    to_delete.push(old_id);
                }
            }
            // Borrar en specs
            let to_delete: Vec<_> = to_delete
                .into_iter()
                .map(|id| world.0.entities().entity(id))
                .collect();
            for ent in to_delete {
                world.0.delete_entity(ent).ok();
            }

            // Actualizar
            dom_data.nodes = new_nodes;
            dirty_nodes.0 = new_dirty;
            reload_trigger.0 = false;
            log_panel.push_info("Recarga de XML completada.");
        }
        Err(e) => {
            log_panel.push_error(format!("Error al recargar XML: {e}"));
            reload_trigger.0 = false;
        }
    }
}

// --------------------------------------------------------------------------------------
// LOAD & FLATTEN
// --------------------------------------------------------------------------------------
fn load_and_flatten_xml(
    world: &mut SpecWorld,
    url: &str,
    rt: &Runtime,
    log_panel: &mut LogPanel,
) -> Result<(HashMap<u32, SpecEntity>, Vec<u32>)> {
    log_panel.push_info(format!("Intentando cargar documento desde: {url}"));

    // INTERCEPTOR DE RUTAS VIRTUALES
    let xml_content = if virtual_routes::VirtualRoutes::is_virtual_url(url) {
        match VIRTUAL_ROUTES.resolve(url) {
            Some(content) => {
                log_panel.push_info(format!("✓ Virtual route resolved: {url}"));
                content
            }
            None => {
                log_panel.push_error(format!("✗ Virtual route not found: {url}"));
                return Err(anyhow::anyhow!("Virtual route not found: {url}"));
            }
        }
    } else {
        // HTTP fetch estándar
        match rt.block_on(load_xml_from_url(url)) {
            Ok(c) => c,
            Err(e) => {
                return Err(anyhow::anyhow!("Error al descargar XML desde {url}: {e}"));
            }
        }
    };

    log_panel.push_info(format!(
        "Contenido obtenido. Longitud: {} caracteres",
        xml_content.len()
    ));

    let root_node = match parse_xml(world, &xml_content) {
        Ok(r) => r,
        Err(e) => {
            return Err(anyhow::anyhow!("Error al parsear el XML: {e}"));
        }
    };

    let mut include_dirty = expand_includes(world, url, rt, log_panel).unwrap_or_default();

    println!("include_dirty: {:?}", include_dirty.len());

    let mut map = HashMap::new();
    let mut dirty = Vec::new();
    let hierarchies = world.read_storage::<Hierarchy>();

    fn flatten_dom(
        node: SpecEntity,
        map: &mut HashMap<u32, SpecEntity>,
        dirty: &mut Vec<u32>,
        hierarchies: &ReadStorage<Hierarchy>,
    ) {
        dirty.push(node.id());
        map.insert(node.id(), node);
        if let Some(h) = hierarchies.get(node) {
            for &child in &h.children {
                flatten_dom(child, map, dirty, hierarchies);
            }
        }
    }

    flatten_dom(root_node, &mut map, &mut dirty, &hierarchies);

    dirty.extend(include_dirty.drain(..));
    dirty.sort_unstable();
    dirty.dedup();

    log_panel.push_info(format!(
        "Árbol DOM parseado. Se encontraron {} nodos.",
        map.len()
    ));

    Ok((map, dirty))
}

// --------------------------------------------------------------------------------------
// UI (Devtool con 4 pestañas: Status, HSML, Logs, Redes)
// --------------------------------------------------------------------------------------
fn ui_system(
    mut contexts: EguiContexts,
    mut url: ResMut<CurrentUrl>,
    mut reload_trigger: ResMut<ReloadTrigger>,
    mut devtool_visible: ResMut<DevtoolVisible>,
    mut devtool_state: ResMut<DevtoolState>,
    world: Res<ElemenetWorld>,
    entity_map: Res<EntityMap>,
    mut commands: Commands,
    mut camera_query: Query<&mut Transform, With<Camera3d>>,
    dom_data: Res<VirtualDomData>,
    mut attribute_updates: ResMut<AttributeUpdates>,
    mut delete_requests: ResMut<DeleteRequests>,
    mut log_panel: ResMut<LogPanel>,
    mut ui_params: UiSystemParams,
    mut render_mode: ResMut<RenderMode>,
) {
    // Ventana NAVEGADOR (siempre se muestra)
    egui::Window::new("Navegador").show(contexts.ctx_mut(), |ui| {
        ui.horizontal(|ui| {
            ui.label("URL:");
            let resp = ui.text_edit_singleline(&mut url.0);
            if resp.lost_focus() && ui.ctx().input(|i| i.key_pressed(egui::Key::Enter)) {
                reload_trigger.0 = true;
            }
            if ui.button("Ir").clicked() || ui.button("Recargar").clicked() {
                reload_trigger.0 = true;
            }
        });

        // Botón VR/Desktop toggle
        let label = if render_mode.is_vr {
            "Cambiar a Desktop"
        } else {
            "Cambiar a VR"
        };
        if ui.button(label).clicked() {
            render_mode.is_vr = !render_mode.is_vr;
            log_panel.push_info(format!(
                "Modo cambiado a: {}",
                if render_mode.is_vr { "VR" } else { "Desktop" }
            ));
        }

        // Aquí solo un botón para mostrar/ocultar Devtool:
        if ui.button("Toggle Devtool").clicked() {
            devtool_visible.0 = !devtool_visible.0;
        }
    });

    // --------------------------------------------------------------------
    // DEVTOOL: cuatro pestañas (Status, HSML, Logs, Redes)
    // --------------------------------------------------------------------
    if devtool_visible.0 {
        egui::Window::new("Devtool")
            .id(egui::Id::new("devtool_window"))
            .show(contexts.ctx_mut(), |ui| {
                ui.horizontal(|ui| {
                    // Botones de pestañas
                    if ui
                        .selectable_label(devtool_state.active_tab == DevtoolTab::Status, "Status")
                        .clicked()
                    {
                        devtool_state.active_tab = DevtoolTab::Status;
                    }
                    if ui
                        .selectable_label(devtool_state.active_tab == DevtoolTab::Hsml, "HSML")
                        .clicked()
                    {
                        devtool_state.active_tab = DevtoolTab::Hsml;
                    }
                    if ui
                        .selectable_label(devtool_state.active_tab == DevtoolTab::Logs, "Consola")
                        .clicked()
                    {
                        devtool_state.active_tab = DevtoolTab::Logs;
                    }
                    if ui
                        .selectable_label(devtool_state.active_tab == DevtoolTab::Redes, "Redes")
                        .clicked()
                    {
                        devtool_state.active_tab = DevtoolTab::Redes;
                    }
                });
                ui.separator();

                // Contenido de cada pestaña
                match devtool_state.active_tab {
                    DevtoolTab::Status => {
                        ui.heading("Estado General");
                        ui.separator();

                        ui.label(format!("Entities: {}", ui_params.entity_counter.count));
                        ui.label(format!("FPS: {}", ui_params.fps_counter.fps));
                        ui.label(format!("Último dom_sync: {:.2} ms", ui_params.perf_stats.dom_sync_ms));

                        ui.separator();
                        ui.heading("Configuración");

                        // Auto-load checkbox
                        let mut auto_load_enabled = ui_params.auto_load_config.enabled;
                        if ui.checkbox(&mut auto_load_enabled, "Auto-cargar al iniciar").changed() {
                            ui_params.auto_load_config.enabled = auto_load_enabled;
                            log_panel.push_info(format!("Auto-load {}", if auto_load_enabled { "habilitado" } else { "deshabilitado" }));
                        }

                        // URL de inicio editable
                        ui.horizontal(|ui| {
                            ui.label("URL inicial:");
                            if ui.text_edit_singleline(&mut ui_params.auto_load_config.start_url).changed() {
                                log_panel.push_info(format!("URL inicial cambiada a: {}", ui_params.auto_load_config.start_url));
                            }
                        });

                        ui.label(format!("URL actual: {}", url.0));
                    }
                    DevtoolTab::Hsml => {
                        ui.heading("Árbol de Elementos (HSML)");
                        ui.separator();

                        let w = ui.available_width();
                        ui.set_width(w);

                        egui::ScrollArea::vertical()
                            .id_source("tree_scroll_area")
                            .max_width(w)
                            .max_height(300.0)
                            .show(ui, |ui| {
                                if let Some(root) = get_root_entity(&world.0) {
                                    show_element_tree(
                                        ui,
                                        root,
                                        &world.0,
                                        &entity_map,
                                        &mut commands,
                                        &mut camera_query,
                                        &dom_data,
                                        &mut attribute_updates,
                                        &mut delete_requests,
                                        &mut log_panel,
                                    );
                                } else {
                                    ui.label("No hay elementos en la escena.");
                                }
                            });
                    }
                    DevtoolTab::Logs => {
                        ui.heading("Consola");
                        ui.separator();

                        let w2 = ui.available_width();
                        ui.set_width(w2);

                        egui::ScrollArea::vertical()
                            .id_source("logs_scroll_area")
                            .max_width(w2)
                            .max_height(200.0)
                            .show(ui, |ui| {
                                for entry in &log_panel.logs {
                                    match entry.level {
                                        LogLevel::Error => {
                                            ui.colored_label(egui::Color32::RED, &entry.message);
                                        }
                                        LogLevel::Warn => {
                                            ui.colored_label(
                                                egui::Color32::YELLOW,
                                                &entry.message,
                                            );
                                        }
                                        LogLevel::Info => {
                                            ui.label(&entry.message);
                                        }
                                    }
                                }
                            });

                        ui.horizontal(|ui| {
                            if ui.button("Limpiar logs").clicked() {
                                log_panel.clear();
                            }

                            if ui.button("Copiar logs").clicked() {
                                let logs_text: String = log_panel.logs.iter()
                                    .map(|entry| {
                                        let prefix = match entry.level {
                                            LogLevel::Error => "[ERROR] ",
                                            LogLevel::Warn => "[WARN] ",
                                            LogLevel::Info => "[INFO] ",
                                        };
                                        format!("{}{}", prefix, entry.message)
                                    })
                                    .collect::<Vec<_>>()
                                    .join("\n");

                                ui.output_mut(|o| o.copied_text = logs_text);
                            }
                        });
                    }
                    DevtoolTab::Redes => {
                        ui.heading("Redes");
                        ui.separator();
                        ui.label("En blanco por ahora...");
                    }
                }
            });
    }
}

// --------------------------------------------------------------------------------------
// Mostrar árbol
// --------------------------------------------------------------------------------------
fn show_element_tree(
    ui: &mut egui::Ui,
    entity: SpecEntity,
    world: &SpecWorld,
    entity_map: &EntityMap,
    commands: &mut Commands,
    camera_query: &mut Query<&mut Transform, With<Camera3d>>,
    dom_data: &VirtualDomData,
    attribute_updates: &mut ResMut<AttributeUpdates>,
    delete_requests: &mut ResMut<DeleteRequests>,
    log_panel: &mut ResMut<LogPanel>,
) {
    let hierarchies = world.read_storage::<Hierarchy>();
    let tags = world.read_storage::<Tag>();
    let attrs = world.read_storage::<Attrs>();
    let transforms = world.read_storage::<Transform2>();

    if let Some(tg) = tags.get(entity) {
        ui.collapsing(format!("{} (ID: {:?})", tg.0, entity), |ui| {
            // Atributos
            if let Some(a) = attrs.get(entity) {
                ui.label("Atributos:");
                for (k, v) in &a.0 {
                    ui.horizontal(|ui| {
                        ui.label(k);
                        let mut val = v.clone();
                        if ui.text_edit_singleline(&mut val).changed() {
                            // Guardar este cambio para aplicarlo en Specs y en el DOM
                            attribute_updates
                                .0
                                .push((entity.id(), k.clone(), val.clone()));
                            log_panel.push_info(format!(
                                "Cambio de atributo: Entidad({:?}) [{}] = {}",
                                entity, k, val
                            ));
                        }
                        if ui.button("🗑").on_hover_text("Eliminar atributo").clicked() {
                            attribute_updates
                                .0
                                .push((entity.id(), k.clone(), "[DEL]".to_string())); // <- en vez de String::from_str
                            log_panel.push_warn(format!(
                                "Eliminar atributo: Entidad({:?}) [{}]",
                                entity, k
                            ));
                        }
                    });
                }
            }
            ui.horizontal(|ui| {
                ui.label("Crear atributo:");
            
                let id = ui.make_persistent_id(("new_attr_key", entity.id()));

                // leer (inmutable)
                let mut key = ui.data_mut(|d| d.get_persisted::<String>(id).unwrap_or_default());

                // editar
                let te_resp = ui.text_edit_singleline(&mut key);

                // escribir (mutable)
                ui.data_mut(|d| d.insert_persisted(id, key.clone()));

                if ui.button("Crear").clicked() && !key.trim().is_empty() {
                    attribute_updates.0.push((entity.id(), key.clone(), String::new()));
                    log_panel.push_info(format!("Crear de atributo: Entidad({:?}) [{}] = ''", entity, key));

                    // limpiar buffer y re-persistir
                    key.clear();
                    ui.data_mut(|d| d.insert_persisted(id, key.clone()));
                    te_resp.request_focus();
                }

            });
            
            // Botones
            ui.horizontal(|ui| {
                if ui.button("Mirar").clicked() {
                    if let Some(tr) = transforms.get(entity) {
                        if let Ok(mut cam) = camera_query.get_single_mut() {
                            let pos = Vec3::new(tr.position.x, tr.position.y, tr.position.z);
                            *cam = Transform::from_translation(pos + Vec3::new(0.0, 3.0, 8.0))
                                .looking_at(pos, Vec3::Y);
                            log_panel.push_info(format!(
                                "Cámara ajustada para mirar la entidad: {:?}",
                                entity
                            ));
                        }
                    }
                }
                if ui.button("Borrar").clicked() {
                    delete_requests.0.push(entity.id());
                    log_panel.push_warn(format!(
                        "Solicitud de borrado para la entidad {:?}",
                        entity
                    ));
                }
            });

            // Hijos
            if let Some(h) = hierarchies.get(entity) {
                for child in &h.children {
                    show_element_tree(
                        ui,
                        *child,
                        world,
                        entity_map,
                        commands,
                        camera_query,
                        dom_data,
                        attribute_updates,
                        delete_requests,
                        log_panel,
                    );
                }
            }
        });
    }
}

// --------------------------------------------------------------------------------------
// Actualizar atributos (MARCA la entidad como Dirty)
// --------------------------------------------------------------------------------------
const ATTR_DELETE_SENTINEL: &str = "[DEL]";

fn apply_attribute_updates(
    mut attribute_updates: ResMut<AttributeUpdates>,
    mut world: ResMut<ElemenetWorld>,
    mut dirty_nodes: ResMut<DirtyNodes>,
) {
    let entities = world.0.entities();

    let mut attrs_storage   = world.0.write_storage::<Attrs>();
    let mut tr_storage      = world.0.write_storage::<Transform2>();
    let mut model_storage   = world.0.write_storage::<Model>();
    let mut include_storage = world.0.write_storage::<Include>();

    // alias (los de tu schema + los "cortos")
    let (px, py, pz) = (TRANSFORM_POSITION[0], TRANSFORM_POSITION[1], TRANSFORM_POSITION[2]);
    let (rx, ry, rz) = (TRANSFORM_ROTATION[0], TRANSFORM_ROTATION[1], TRANSFORM_ROTATION[2]);
    let (sx, sy, sz) = (TRANSFORM_SCALE[0],    TRANSFORM_SCALE[1],    TRANSFORM_SCALE[2]);

    for (ent_id, key, val) in attribute_updates.0.drain(..) {
        let ent = entities.entity(ent_id);
        if !entities.is_alive(ent) { continue; }

        // Asegurar Attrs
        if attrs_storage.get(ent).is_none() {
            let _ = attrs_storage.insert(ent, Attrs(HashMap::new()));
        }

        // Set / Del en Attrs
        if let Some(a) = attrs_storage.get_mut(ent) {
            if val == ATTR_DELETE_SENTINEL {
                a.0.remove(&key);
                if let Some(tr) = tr_storage.get_mut(ent) {
                    match key.as_str() {
                        // posición
                        "x" => tr.position.x = 0.0, 
                        "y" => tr.position.y = 0.0,
                        "z" => tr.position.z = 0.0,

                        // rotación
                        "rx" => tr.rotation.x = 0.0, // o 0.0
                        "ry" => tr.rotation.y = 0.0,
                        "rz" => tr.rotation.z = 0.0,

                        // escala
                        "s"  => { /* podrías no tocar nada o resetear a 1.0 */ }
                        "sx" => tr.scale.x = 1.0, // o 1.0
                        "sy" => tr.scale.y = 1.0,
                        "sz" => tr.scale.z = 1.0,

                        _ => {}
                    }
                }
            } else {
                a.0.insert(key.clone(), val.clone());
            }
        }

        // Reflejar en Transform2 si existe
        if let Some(tr) = tr_storage.get_mut(ent) {
            let parse_f32 = || -> Option<f32> { val.trim().parse::<f32>().ok() };
            match key.as_str() {
                // posición (acepta alias de schema y cortos)
                k if k == px || k == "x"  => if let Some(f) = parse_f32() { tr.position.x = f; },
                k if k == py || k == "y"  => if let Some(f) = parse_f32() { tr.position.y = f; },
                k if k == pz || k == "z"  => if let Some(f) = parse_f32() { tr.position.z = f; },

                // rotación
                k if k == rx || k == "rx" => if let Some(f) = parse_f32() { tr.rotation.x = f; },
                k if k == ry || k == "ry" => if let Some(f) = parse_f32() { tr.rotation.y = f; },
                k if k == rz || k == "rz" => if let Some(f) = parse_f32() { tr.rotation.z = f; },

                // escala uniforme
                "s" => if let Some(f) = parse_f32() {
                    tr.scale.x = f; tr.scale.y = f; tr.scale.z = f;
                },

                // escala (alias)
                k if k == sx || k == "sx" => if let Some(f) = parse_f32() { tr.scale.x = f; },
                k if k == sy || k == "sy" => if let Some(f) = parse_f32() { tr.scale.y = f; },
                k if k == sz || k == "sz" => if let Some(f) = parse_f32() { tr.scale.z = f; },

                _ => {}
            }
        }

        // Sincronizar alto nivel
        if key == "src" {
            if let Some(m) = model_storage.get_mut(ent) {
                m.src = (val != ATTR_DELETE_SENTINEL).then(|| val.clone());
            }
            if let Some(i) = include_storage.get_mut(ent) {
                i.src = (val != ATTR_DELETE_SENTINEL).then(|| val.clone());
            }
        }

        // marcar dirty
        dirty_nodes.0.push(ent_id);
    }
}




// --------------------------------------------------------------------------------------
// Borrar
// --------------------------------------------------------------------------------------
fn process_delete_requests(
    mut delete_requests: ResMut<DeleteRequests>,
    mut world: ResMut<ElemenetWorld>,
    mut entity_map: ResMut<EntityMap>,
    mut commands: Commands,
    mut log_panel: ResMut<LogPanel>,
) {
    for ent_id in delete_requests.0.drain(..) {
        if let Some(bevy_ent) = entity_map.0.remove(&ent_id) {
            commands.entity(bevy_ent).despawn_recursive();
        }
        let sp_ent = world.0.entities().entity(ent_id);
        match world.0.delete_entity(sp_ent) {
            Ok(_) => log_panel.push_info(format!("Entidad (ID={}) eliminada correctamente.", ent_id)),
            Err(_) => log_panel.push_error(format!("Error al eliminar la entidad (ID={}).", ent_id)),
        }
    }
}

// --------------------------------------------------------------------------------------
// Mark Dirty
// --------------------------------------------------------------------------------------
fn mark_dirty_system(
    world: Res<ElemenetWorld>,
    mut commands: Commands,
    entity_map: Res<EntityMap>,
    dirty_nodes: ResMut<DirtyNodes>, // <- no necesitamos mut del Vec
) {
    // NO drenar: iteramos por copia/refs y dejamos que dom_sync drene
    for node_id in dirty_nodes.0.iter().copied() {
        if let Some(&ent) = entity_map.0.get(&node_id) {
            commands.entity(ent).insert(Dirty);
        }
    }
}

// --------------------------------------------------------------------------------------
// HELPER FUNCTIONS FOR ATTRIBUTE PARSING
// --------------------------------------------------------------------------------------

/// Parse a hex color string (e.g., "#FF5733") to Bevy Color
fn parse_hex_color(hex: &str) -> Option<Color> {
    let hex = hex.trim_start_matches('#');
    if hex.len() != 6 {
        return None;
    }

    let r = u8::from_str_radix(&hex[0..2], 16).ok()? as f32 / 255.0;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()? as f32 / 255.0;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()? as f32 / 255.0;

    Some(Color::srgb(r, g, b))
}

/// Get an attribute as f32, with a default value
fn get_attr_f32(attrs: &HashMap<String, String>, key: &str, default: f32) -> f32 {
    attrs.get(key)
        .and_then(|s| s.parse::<f32>().ok())
        .unwrap_or(default)
}

/// Get an attribute as String, with a default value
fn get_attr_string(attrs: &HashMap<String, String>, key: &str, default: &str) -> String {
    attrs.get(key)
        .map(|s| s.to_string())
        .unwrap_or_else(|| default.to_string())
}

fn get_text_font() -> &'static Font {
    static FONT: OnceLock<Font> = OnceLock::new();
    FONT.get_or_init(|| {
        let font_bytes: &[u8] = include_bytes!("../assets/fonts/FiraSans-Regular.ttf");
        Font::from_bytes(font_bytes, FontSettings::default())
            .expect("No se pudo cargar la fuente FiraSans-Regular.ttf")
    })
}

/// Create a text texture using fontdue rasterization.
fn create_text_texture(
    text: &str,
    color: Color,
    images: &mut Assets<Image>,
) -> Handle<Image> {
    let text_font = get_text_font();
    let normalized_text = if text.is_empty() { " " } else { text };
    let font_px = 48.0f32;
    let padding = 4u32;

    let mut layout = Layout::new(CoordinateSystem::PositiveYDown);
    layout.reset(&LayoutSettings {
        x: 0.0,
        y: 0.0,
        ..LayoutSettings::default()
    });
    layout.append(&[text_font], &TextStyle::new(normalized_text, font_px, 0));

    let glyphs = layout.glyphs();
    let mut max_x = 0f32;
    let mut max_y = 0f32;
    for glyph in glyphs {
        max_x = max_x.max(glyph.x + glyph.width as f32);
        max_y = max_y.max(glyph.y + glyph.height as f32);
    }

    let width = (max_x.ceil() as u32 + padding * 2).max(32);
    let height = (max_y.ceil() as u32 + padding * 2).max(16);

    let mut data = vec![0u8; (width * height * 4) as usize];

    let color_array = color.to_srgba().to_u8_array();
    let r = color_array[0];
    let g = color_array[1];
    let b = color_array[2];

    for glyph in glyphs {
        let (_, bitmap) = text_font.rasterize_config(glyph.key);
        let base_x = padding as i32 + glyph.x.floor() as i32;
        let base_y = padding as i32 + glyph.y.floor() as i32;

        for gy in 0..glyph.height {
            for gx in 0..glyph.width {
                let alpha = bitmap[gy * glyph.width + gx];
                if alpha == 0 {
                    continue;
                }

                let x = base_x + gx as i32;
                let y = base_y + gy as i32;
                if x < 0 || y < 0 || x >= width as i32 || y >= height as i32 {
                    continue;
                }

                let idx = ((y as u32 * width + x as u32) * 4) as usize;
                data[idx] = r;
                data[idx + 1] = g;
                data[idx + 2] = b;
                data[idx + 3] = alpha;
            }
        }
    }

    let image = Image::new(
        bevy::render::render_resource::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        bevy::render::render_resource::TextureDimension::D2,
        data,
        bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
        bevy::render::render_asset::RenderAssetUsages::RENDER_WORLD,
    );

    images.add(image)
}

fn parse_text_attrs(attrs_map: &HashMap<String, String>) -> (String, f32, Color) {
    let text_value = get_attr_string(attrs_map, "value", "Text");
    let text_size = get_attr_f32(attrs_map, "size", 0.1);
    let text_color = attrs_map
        .get("color")
        .and_then(|c| parse_hex_color(c))
        .unwrap_or(Color::srgb(1.0, 1.0, 1.0));
    (text_value, text_size, text_color)
}

fn build_text_transform(mut base_transform: Transform, text_value: &str, text_size: f32) -> Transform {
    let text_width = text_size * text_value.chars().count() as f32 * 0.6;
    let text_height = text_size;
    base_transform.scale = Vec3::new(text_width.max(0.01), text_height.max(0.01), 1.0);
    base_transform
}

fn get_or_create_text_material(
    text_cache: &mut TextMaterialCache,
    materials: &mut Assets<StandardMaterial>,
    images: &mut Assets<Image>,
    text_value: &str,
    text_size: f32,
    text_color: Color,
) -> Handle<StandardMaterial> {
    let [r, g, b, a] = text_color.to_srgba().to_u8_array();
    let cache_key = TextMaterialKey {
        value: text_value.to_string(),
        size_bits: text_size.to_bits(),
        color_key: format!("{r:02x}{g:02x}{b:02x}{a:02x}"),
    };

    if let Some(handle) = text_cache.materials.get(&cache_key) {
        return handle.clone();
    }

    let text_texture = create_text_texture(text_value, text_color, images);
    let text_material = materials.add(StandardMaterial {
        base_color_texture: Some(text_texture),
        alpha_mode: bevy::prelude::AlphaMode::Blend,
        unlit: true,
        ..Default::default()
    });

    text_cache.materials.insert(cache_key, text_material.clone());
    text_material
}

const DOM_SYNC_VERBOSE_LOGS: bool = false;

// --------------------------------------------------------------------------------------
// DOM -> Bevy + medicion de tiempo
// --------------------------------------------------------------------------------------
fn dom_sync_system(
    world: Res<ElemenetWorld>,
    mut commands: Commands,
    dom_data: Res<VirtualDomData>,
    shared_resources: Res<SharedResources>,
    mut entity_map: ResMut<EntityMap>,
    mut dirty_nodes: ResMut<DirtyNodes>,
    mut query: Query<(
        Entity,
        &mut Transform,
        Option<&Dirty>,
        Option<&mut Handle<StandardMaterial>>,
    )>,
    asset_server: Res<AssetServer>,
    mut log_panel: ResMut<LogPanel>,
    mut model_cache: ResMut<ModelCache>,
    tokio_rt: Res<TokioRuntime>,
    current_url: Res<CurrentUrl>,
    mut perf_stats: ResMut<PerformanceStats>,
    mut pending_scripts: ResMut<PendingScripts>,
    mut text_render: TextRenderParams,
) {
    let start_time = Instant::now();

    if dirty_nodes.0.is_empty() {
        return;
    }

    let tags = world.0.read_storage::<Tag>();
    let transforms = world.0.read_storage::<Transform2>();
    let hierarchies = world.0.read_storage::<Hierarchy>();
    let models = world.0.read_storage::<Model>();
    let attrs_storage = world.0.read_storage::<Attrs>();

    log_panel.push_info(format!(
        "dom_sync_system: Procesando {} dirty nodes...",
        dirty_nodes.0.len()
    ));

    for node_id in dirty_nodes.0.drain(..) {
        if DOM_SYNC_VERBOSE_LOGS {
            log_panel.push_info(format!("  Revisando node_id={}", node_id));
        }
        if let Some(node) = dom_data.nodes.get(&node_id) {
            // Tag
            let tag = tags.get(*node).map(|t| t.0.clone()).unwrap_or_default();
            if DOM_SYNC_VERBOSE_LOGS {
                log_panel.push_info(format!("    Tag='{}'", tag));
            }

            let hierarchy = hierarchies.get(*node);
            let parent_id = hierarchy.and_then(|h| h.parent);

            // Calculamos la Transform de Specs
            let mut transform_b = Transform::default();
            if let Some(tr2) = transforms.get(*node) {
                apply_transform(tr2, &mut transform_b);
            }

            // ¿existe la entidad de Bevy asociada a este node_id?
            if let Some(&bevy_ent) = entity_map.0.get(&node_id) { // ********* CAMBIO CLAVE *********
                if tag == "model" {
                    // 1) eliminar entidad vieja
                    commands.entity(bevy_ent).despawn_recursive();
                    entity_map.0.remove(&node_id);
    
                    // 2) crear nueva con el src actual
                    let new_ent = {
                        if let Some(model_data) = models.get(*node) {
                            if let Some(ref original_src) = model_data.src {
                                if let Some(final_url) = resolve_remote_path(&current_url.0, original_src) {
                                    let scene_handle = apply_model_with_cache(
                                        &final_url,
                                        &asset_server,
                                        &mut log_panel,
                                        &mut model_cache,
                                        &tokio_rt.0,
                                    );
                                    commands.spawn((
                                        SceneBundle {
                                            scene: scene_handle,
                                            transform: transform_b,
                                            ..Default::default()
                                        },
                                        Dirty,
                                    )).id()
                                } else {
                                    // src inválido → placeholder vacío
                                    commands.spawn((
                                        SpatialBundle { transform: transform_b, ..Default::default() },
                                        Dirty,
                                    )).id()
                                }
                            } else {
                                // sin src → placeholder
                                commands.spawn((
                                    SpatialBundle { transform: transform_b, ..Default::default() },
                                    Dirty,
                                )).id()
                            }
                        } else {
                            // no hay componente Model → placeholder
                            commands.spawn((
                                SpatialBundle { transform: transform_b, ..Default::default() },
                                Dirty,
                            )).id()
                        }
                    };
    
                    // 3) restaurar parent
                    if let Some(pid) = parent_id {
                        if let Some(&parent_bevy_ent) = entity_map.0.get(&pid) {
                            commands.entity(new_ent).set_parent(parent_bevy_ent);
                        }
                    } else {
                        commands.entity(new_ent).remove_parent();
                    }
    
                    // 4) actualizar mapping
                    entity_map.0.insert(node_id, new_ent);
                    continue; // ya procesamos este node_id
                }

                if tag == "text" {
                    let empty_map = HashMap::new();
                    let attrs_map = attrs_storage
                        .get(*node)
                        .map(|a| &a.0)
                        .unwrap_or(&empty_map);

                    let (text_value, text_size, text_color) = parse_text_attrs(attrs_map);
                    let text_material = get_or_create_text_material(
                        &mut text_render.text_material_cache,
                        &mut text_render.materials,
                        &mut text_render.images,
                        &text_value,
                        text_size,
                        text_color,
                    );
                    let text_transform = build_text_transform(transform_b, &text_value, text_size);

                    let mut updated_in_place = false;
                    if let Ok((_, mut t, dirty, maybe_material)) = query.get_mut(bevy_ent) {
                        *t = text_transform;
                        if let Some(mut material_handle) = maybe_material {
                            *material_handle = text_material.clone();
                            if dirty.is_some() {
                                commands.entity(bevy_ent).remove::<Dirty>();
                            }
                            updated_in_place = true;
                        }
                    }
                    if updated_in_place {
                        continue;
                    }

                    commands.entity(bevy_ent).despawn_recursive();
                    entity_map.0.remove(&node_id);

                    let new_ent = commands
                        .spawn((
                            PbrBundle {
                                mesh: shared_resources.plane_mesh.clone(),
                                material: text_material,
                                transform: text_transform,
                                ..Default::default()
                            },
                            Dirty,
                        ))
                        .id();

                    if let Some(pid) = parent_id {
                        if let Some(&parent_bevy_ent) = entity_map.0.get(&pid) {
                            commands.entity(new_ent).set_parent(parent_bevy_ent);
                        }
                    } else {
                        commands.entity(new_ent).remove_parent();
                    }

                    entity_map.0.insert(node_id, new_ent);
                    continue;
                }

                // caso normal (no-model): solo actualizar transform si tiene Dirty
                if let Ok((_, mut t, dirty, _)) = query.get_mut(bevy_ent) {
                    if dirty.is_some() {
                        *t = transform_b;
                        commands.entity(bevy_ent).remove::<Dirty>();
                    }
                }
            } else {
                // Crear nueva
                if DOM_SYNC_VERBOSE_LOGS {
                    log_panel.push_info("    -> No existe, creando nueva entidad...");
                }

                let new_ent = match tag.as_str() {
                    "model" => {
                        if DOM_SYNC_VERBOSE_LOGS {
                            log_panel.push_info("    -> Tag='model'");
                        }
                        if let Some(model_data) = models.get(*node) {
                            if let Some(ref original_src) = model_data.src {
                                if let Some(final_url) =
                                    resolve_remote_path(&current_url.0, original_src)
                                {
                                    if DOM_SYNC_VERBOSE_LOGS {
                                        log_panel.push_info(format!("       final_url={}", final_url));
                                    }
                                    // Descargamos o usamos caché
                                    let scene_handle = apply_model_with_cache(
                                        &final_url,
                                        &asset_server,
                                        &mut log_panel,
                                        &mut model_cache,
                                        &tokio_rt.0,
                                    );
                                    // Creamos un SceneBundle con transform_b:
                                    commands
                                        .spawn((
                                            SceneBundle {
                                                scene: scene_handle,
                                                transform: transform_b,
                                                ..Default::default()
                                            },
                                            Dirty,
                                        ))
                                        .id()
                                } else {
                                    log_panel.push_error(format!(
                                        "No se pudo unir '{}' con base '{}'",
                                        original_src, current_url.0
                                    ));
                                    // Creamos de todas formas la entidad con un "Bundle" vacío
                                    commands
                                        .spawn((
                                            SpatialBundle {
                                                transform: transform_b,
                                                ..Default::default()
                                            },
                                            Dirty,
                                        ))
                                        .id()
                                }
                            } else {
                                log_panel
                                    .push_warn("No hay src en <model>. Creando caja por defecto.");
                                commands
                                    .spawn((
                                        PbrBundle {
                                            mesh: shared_resources.cube_mesh.clone(),
                                            material: shared_resources.default_material.clone(),
                                            transform: transform_b,
                                            ..default()
                                        },
                                        Dirty,
                                    ))
                                    .id()
                            }
                        } else {
                            // No hay Model en Specs => caja default
                            commands
                                .spawn((
                                    PbrBundle {
                                        mesh: shared_resources.cube_mesh.clone(),
                                        material: shared_resources.default_material.clone(),
                                        transform: transform_b,
                                        ..default()
                                    },
                                    Dirty,
                                ))
                                .id()
                        }
                    }
                    "script" => {
                        if DOM_SYNC_VERBOSE_LOGS {
                            log_panel.push_info("    -> script tag encontrado");
                        }
                        let scripts_storage = world.0.read_storage::<Script>();
                        if let Some(script_comp) = scripts_storage.get(*node) {
                            if let Some(ref src) = script_comp.src {
                                if let Some(final_url) = resolve_remote_path(&current_url.0, src) {

                                    // MANEJAR SCRIPTS VIRTUALES
                                    let code_result = if virtual_routes::VirtualRoutes::is_virtual_url(&final_url) {
                                        match VIRTUAL_ROUTES.resolve(&final_url) {
                                            Some(code) => {
                                                log_panel.push_info(format!("✓ Virtual script: {}", final_url));
                                                Ok(code)
                                            }
                                            None => {
                                                log_panel.push_error(format!("✗ Virtual script not found: {}", final_url));
                                                Err(format!("Virtual script not found: {}", final_url))
                                            }
                                        }
                                    } else {
                                        // HTTP download estándar
                                        log_panel.push_info(format!("    -> Descargando script: {}", final_url));
                                        tokio_rt.0.block_on(async {
                                            let resp = reqwest::get(&final_url).await
                                                .map_err(|e| format!("Error HTTP: {}", e))?;
                                            resp.text().await
                                                .map_err(|e| format!("Error leyendo texto: {}", e))
                                        })
                                    };

                                    match code_result {
                                        Ok(code) => {
                                            if let Some(space_id) = find_owner_space_id(&world.0, *node) {
                                                pending_scripts.0.push((space_id, final_url, code));
                                            } else {
                                                log_panel.push_warn(
                                                    "script sin <space> ancestro: se omite la evaluacion"
                                                );
                                            }
                                        }
                                        Err(e) => {
                                            log_panel.push_error(format!("Error cargando script {}: {}", final_url, e));
                                        }
                                    }
                                }
                            }
                        }
                        commands
                            .spawn((
                                SpatialBundle {
                                    transform: transform_b,
                                    ..Default::default()
                                },
                                Dirty,
                            ))
                            .id()
                    }
                    "space" => {
                        if DOM_SYNC_VERBOSE_LOGS {
                            log_panel.push_info("    -> space, no spawneamos nada 3D");
                        }
                        commands
                            .spawn((
                                SpatialBundle {
                                    transform: transform_b,
                                    ..Default::default()
                                },
                                Dirty,
                            ))
                            .id()
                    }
                    "include" => {
                        if DOM_SYNC_VERBOSE_LOGS {
                            log_panel
                                .push_info("    -> include, no spawneamos nada 3D");
                        }
                        commands
                            .spawn((
                                SpatialBundle {
                                    transform: transform_b,
                                    ..Default::default()
                                },
                                Dirty,
                            ))
                            .id()

                            // Aqui debo de poder cargar otro .hsml
                            // Sample: <include src="/zonas/plaza.hsml"/>
                    }
                    "groud" => {
                        let new_ent_empty = commands
                            .spawn((
                                SpatialBundle {
                                    transform: transform_b,
                                    ..Default::default()
                                },
                                Dirty,
                            ))
                            .id();
                        new_ent_empty
                    }
                    "box" => {
                        if DOM_SYNC_VERBOSE_LOGS {
                            log_panel.push_info("    -> box element");
                        }

                        // Parse box attributes
                        let color = attrs_storage.get(*node)
                            .and_then(|a| a.0.get("color"))
                            .and_then(|c| parse_hex_color(c))
                            .unwrap_or(Color::srgb(0.5, 0.5, 0.5));

                        // Create material with the specified color
                        let material = text_render.materials.add(StandardMaterial {
                            base_color: color,
                            ..Default::default()
                        });

                        commands
                            .spawn((
                                PbrBundle {
                                    mesh: shared_resources.cube_mesh.clone(),
                                    material,
                                    transform: transform_b,
                                    ..Default::default()
                                },
                                Dirty,
                            ))
                            .id()
                    }
                    "text" => {
                        if DOM_SYNC_VERBOSE_LOGS {
                            log_panel.push_info("    -> text element");
                        }

                        // Parse text attributes
                        let empty_map = HashMap::new();
                        let attrs_map = attrs_storage.get(*node)
                            .map(|a| &a.0)
                            .unwrap_or(&empty_map);

                        let (text_value, text_size, text_color) = parse_text_attrs(attrs_map);
                        let text_material = get_or_create_text_material(
                            &mut text_render.text_material_cache,
                            &mut text_render.materials,
                            &mut text_render.images,
                            &text_value,
                            text_size,
                            text_color,
                        );
                        let text_transform = build_text_transform(transform_b, &text_value, text_size);

                        // Create text as a 3D plane in world space
                        commands
                            .spawn((
                                PbrBundle {
                                    mesh: shared_resources.plane_mesh.clone(),
                                    material: text_material,
                                    transform: text_transform,
                                    ..Default::default()
                                },
                                Dirty,
                            ))
                            .id()
                    }
                    other => {
                        if DOM_SYNC_VERBOSE_LOGS {
                            log_panel.push_info(format!("    -> Tag='{}', elemento desconocido", other));
                        }

                        // Para elementos desconocidos, crear un cubo genérico
                        commands
                            .spawn((
                                PbrBundle {
                                    mesh: shared_resources.cube_mesh.clone(),
                                    material: shared_resources.default_material.clone(),
                                    transform: transform_b,
                                    ..Default::default()
                                },
                                Dirty,
                            ))
                            .id()
                    }
                };

                // Jerarquía (parent-child en Bevy)
                if let Some(pid) = parent_id {
                    if let Some(&parent_bevy_ent) = entity_map.0.get(&pid) {
                        commands.entity(new_ent).set_parent(parent_bevy_ent);
                    }
                } else {
                    commands.entity(new_ent).remove_parent();
                }

                // Guardamos la relación node_id -> bevy_entity
                entity_map.0.insert(node_id, new_ent);
            }
        } else {
            log_panel.push_error(format!("    No existe dom_data.nodes para node_id={}", node_id));
        }
    }

    // Tiempo transcurrido
    let elapsed = start_time.elapsed().as_secs_f32() * 1000.0;
    perf_stats.dom_sync_ms = elapsed;
}

// --------------------------------------------------------------------------------------
// Contador de entidades
// --------------------------------------------------------------------------------------
fn update_entity_counter(mut c: ResMut<EntityCounter>, q: Query<Entity>) {
    c.count = q.iter().count();
}

// --------------------------------------------------------------------------------------
// FPS
// --------------------------------------------------------------------------------------
fn update_fps_counter(time: Res<Time>, mut f: ResMut<FpsCounter>) {
    f.frame_count += 1;
    if f.timer.tick(time.delta()).just_finished() {
        f.fps = f.frame_count;
        f.frame_count = 0;
    }
}

// --------------------------------------------------------------------------------------
// Root
// --------------------------------------------------------------------------------------
fn get_root_entity(world: &SpecWorld) -> Option<SpecEntity> {
    let hier = world.read_storage::<Hierarchy>();
    let tags = world.read_storage::<Tag>();

    let mut roots = Vec::new();
    for (ent, h) in (&world.entities(), &hier).join() {
        if h.parent.is_none() {
            let tag_name = tags.get(ent).map(|t| t.0.as_str()).unwrap_or("???");
            roots.push((ent, tag_name.to_string()));
        }
    }

    // Priorizar 'hsml' o 'space' como root, ignorar otros
    let hsml_root = roots.iter().find(|(_, tag)| tag == "hsml").map(|(ent, _)| *ent);
    if hsml_root.is_some() {
        return hsml_root;
    }

    let space_root = roots.iter().find(|(_, tag)| tag == "space").map(|(ent, _)| *ent);
    if space_root.is_some() {
        return space_root;
    }

    // Fallback: retornar el primero
    roots.into_iter().next().map(|(ent, _)| ent)
}

// --------------------------------------------------------------------------------------
// Genera un nombre base64
// --------------------------------------------------------------------------------------
fn encode_url_to_filename(url: &str) -> String {
    let b64 = Base64Engine.encode(url);
    let safe_b64 = b64.replace('/', "_").replace('+', "-");
    let ext = match Path::new(url).extension() {
        Some(e) => e.to_string_lossy().to_string(),
        None => "bin".to_string(),
    };
    format!("{safe_b64}.{ext}")
}

// --------------------------------------------------------------------------------------
// DESCARGA si no está en cache (assets/cache/xxx)
// --------------------------------------------------------------------------------------
fn download_model_if_needed(
    rt: &Runtime,
    url: &str,
    cache: &mut ModelCache,
    log_panel: &mut LogPanel,
) -> Result<String> {
    // 1) ¿Está en la cache?
    if let Some(cached_path) = cache.cache.get(url) {
        if Path::new(cached_path).exists() {
            log_panel.push_info(format!("Ya estaba en cache: {} -> {}", url, cached_path));
            return Ok(cached_path.clone());
        } else {
            log_panel.push_warn(format!(
                "Cache decía {}->{} pero no existe el archivo. Se descarga de nuevo.",
                url, cached_path
            ));
        }
    } else {
        log_panel.push_info(format!("No estaba en cache, se descargará: {}", url));
    }

    let (assets_dir, cache_dir) = folder::resolve_assets_and_cache_dirs();

    // 2) Loggear a dónde escribimos
    log_panel.push_info(format!("Assets dir: {}", assets_dir.display()));
    log_panel.push_info(format!("Cache dir:  {}", cache_dir.display()));

    // path.display() = ***\bevy_oxr\assets/cache

    // Crear carpeta cache
    let _ = fs::create_dir_all(cache_dir.clone());
    // Nombre base64                 
    let filename = encode_url_to_filename(url);
    let local_path = cache_dir.join(filename).to_string_lossy().to_string();

    // Distinguimos HTTP vs local
    if url.starts_with("http://") || url.starts_with("https://") {
        log_panel.push_info(format!("Descargando HTTP: {}", url));
        let bytes = rt
            .block_on(load_bytes_from_url(url))
            .map_err(|e| anyhow::anyhow!("Fallo en descarga: {e}"))?;
        fs::write(&local_path, bytes)
            .map_err(|e| anyhow::anyhow!("No se pudo escribir archivo: {e}"))?;
    } else {
        let from = PathBuf::from(url);
        if !from.exists() {
            return Err(anyhow::anyhow!("El archivo local no existe: {url}"));
        }
        fs::copy(&from, &local_path)
            .map_err(|e| anyhow::anyhow!("No se pudo copiar archivo local: {e}"))?;
    }

    cache.cache.insert(url.to_string(), local_path.clone());
    log_panel.push_info(format!(
        "Guardado en cache => url={} -> local_path={}",
        url, local_path
    ));
    Ok(local_path)
}

// --------------------------------------------------------------------------------------
// APLICAR model
// --------------------------------------------------------------------------------------
fn apply_model_with_cache(
    remote_path: &str,
    asset_server: &AssetServer,
    log_panel: &mut LogPanel,
    model_cache: &mut ModelCache,
    rt: &Runtime,
) -> Handle<Scene> {
    match download_model_if_needed(rt, remote_path, model_cache, log_panel) {
        Ok(local_path) => {
            log_panel.push_info(format!("Descarga/caché OK => {local_path}"));
            // Convertir la ruta a algo relativo a "assets/", si procede
            let relative: String = local_path
                .strip_prefix("crates/bevy_openxr/assets/")
                .map(|s| s.to_string())
                .unwrap_or_else(|| local_path.clone());

            log_panel.push_info(format!("Cargando con asset_server.load('{relative}')"));

            // Para archivos .gltf o .glb se puede usar “#Scene0”
            if relative.ends_with(".gltf") || relative.ends_with(".glb") {
                // Se puede cargar escena 0 con “#Scene0”
                let final_path = format!("{relative}#Scene0");
                let scene_handle: Handle<Scene> = asset_server.load_with_settings(final_path, |settings: &mut GltfLoaderSettings| {
                    settings.load_cameras = false;
                    settings.load_lights = false;
                });
                scene_handle
            } else {
                // Carga normal como Scene
                let scene_handle: Handle<Scene> = asset_server.load_with_settings(relative, |settings: &mut GltfLoaderSettings| {
                    settings.load_cameras = false;
                    settings.load_lights = false;
                });
                scene_handle
            }
        }
        Err(e) => {
            log_panel.push_error(format!("No se pudo preparar el modelo '{remote_path}': {e}"));
            // Retornamos un handle vacío para no romper
            Handle::default()
        }
    }
}

// --------------------------------------------------------------------------------------
// Carga bytes
// --------------------------------------------------------------------------------------
async fn load_bytes_from_url(url: &str) -> Result<Vec<u8>> {
    let resp = reqwest::get(url).await?;
    if !resp.status().is_success() {
        return Err(anyhow::anyhow!(
            "Status code {} al descargar {}",
            resp.status(),
            url
        ));
    }
    let bytes = resp.bytes().await?;
    Ok(bytes.to_vec())
}

// --------------------------------------------------------------------------------------
// Combina la URL base con la ruta
// --------------------------------------------------------------------------------------
fn resolve_remote_path(base_url: &str, remote_path: &str) -> Option<String> {
    // 1. Check luna:// FIRST
    if virtual_routes::VirtualRoutes::is_virtual_url(remote_path) {
        return Some(remote_path.to_string());
    }

    // 2. HTTP/HTTPS absolutos
    if remote_path.starts_with("http://") || remote_path.starts_with("https://") {
        return Some(remote_path.to_string());
    }

    // 3. Resolución relativa
    let Ok(base) = Url::parse(base_url) else {
        return None;
    };
    let Ok(final_url) = base.join(remote_path) else {
        return None;
    };
    Some(final_url.to_string())
}

/// Recolecta IDs de un subárbol para marcarlos dirty
fn collect_subtree_ids(
    world: &SpecWorld,
    root: SpecEntity,
    out: &mut Vec<u32>,
) {
    let hier = world.read_storage::<Hierarchy>();
    out.push(root.id());
    if let Some(h) = hier.get(root) {
        for &c in &h.children {
            collect_subtree_ids(world, c, out);
        }
    }
}

/// Expande TODOS los <include src="..."> del world.
/// Devuelve IDs que deben marcarse dirty.
fn expand_includes(
    world: &mut SpecWorld,
    base_url: &str,
    rt: &Runtime,
    log: &mut LogPanel,
) -> anyhow::Result<Vec<u32>> {
    use specs::Join;

    // 1) FASE DE LECTURA (inmutable) EN UN BLOQUE
    let targets: Vec<(SpecEntity, String)> = {
        let entities   = world.entities();                 // <- inmutable
        let includes_r = world.read_storage::<Include>();  // <- inmutable

        let mut v = Vec::new();
        for (ent, inc) in (&entities, &includes_r).join() {
            if let Some(ref src) = inc.src {
                v.push((ent, src.clone()));
            }
        }
        // al salir del bloque, se liberan 'entities' e 'includes_r'
        v
    }; // <- aquí se sueltan TODOS los borrows inmutables

    // 2) FASE DE MUTACIÓN (ya podemos usar &mut World)
    let mut new_dirty = Vec::new();

    for (parent_ent, src) in targets {
        // resolver URL
        let Some(final_url) = resolve_remote_path(base_url, &src) else {
            log.push_warn(format!("include: no se pudo resolver src='{src}' contra base='{base_url}'"));
            continue;
        };

        // descargar (con soporte para luna://)
        let xml = if virtual_routes::VirtualRoutes::is_virtual_url(&final_url) {
            match VIRTUAL_ROUTES.resolve(&final_url) {
                Some(content) => {
                    log.push_info(format!("✓ Virtual include: {}", final_url));
                    content
                }
                None => {
                    log.push_error(format!("✗ Virtual include not found: {}", final_url));
                    continue;
                }
            }
        } else {
            // HTTP download estándar
            match rt.block_on(load_xml_from_url(&final_url)) {
                Ok(x) => x,
                Err(e) => {
                    log.push_error(format!("include: error descargando {} -> {e}", final_url));
                    continue;
                }
            }
        };

        // parsear (requiere &mut World)  ✅ ahora compila
        let child_root = match parse_xml(world, &xml) {
            Ok(r) => r,
            Err(e) => {
                log.push_error(format!("include: error parseando {} -> {e}", final_url));
                continue;
            }
        };

        // colgar como hijo (requiere &mut World)  ✅ ahora compila
        Hierarchy::add_child(world, parent_ent, child_root);

        // recolectar IDs del subárbol para marcarlos dirty
        collect_subtree_ids(world, child_root, &mut new_dirty);
    }

    Ok(new_dirty)
}

fn find_owner_space_id(world: &SpecWorld, mut node: SpecEntity) -> Option<u32> {
    let entities = world.entities();
    let hier = world.read_storage::<Hierarchy>();
    let tags = world.read_storage::<Tag>();

    loop {
        if let Some(tag) = tags.get(node) {
            if tag.0 == "space" {
                return Some(node.id());
            }
        }
        let parent_id = hier.get(node).and_then(|h| h.parent)?;
        node = entities.entity(parent_id);
    }
}

fn create_space_context(space_id: u32) -> std::result::Result<SpaceScriptContext, String> {
    let mut engine = std::panic::catch_unwind(JsEngine::new)
        .map_err(|e| format!("panic creating JS engine for space {}: {:?}", space_id, e))?;

    let set_root = format!(
        "globalThis.hiperspace.dimention = new HSMLRootElement({});",
        space_id
    );
    engine
        .eval(&set_root)
        .map_err(|e| format!("failed to set JS root for space {}: {}", space_id, e))?;

    Ok(SpaceScriptContext {
        engine,
        loaded_scripts: HashSet::new(),
    })
}

fn spawn_space_worker(space_id: u32) -> std::result::Result<SpaceScriptWorker, String> {
    let (cmd_tx, cmd_rx) = mpsc::channel::<JsWorkerCommand>();
    let (event_tx, event_rx) = mpsc::channel::<JsWorkerEvent>();

    let join = thread::Builder::new()
        .name(format!("js-space-{}", space_id))
        .spawn(move || {
            let mut ctx = match create_space_context(space_id) {
                Ok(ctx) => ctx,
                Err(err) => {
                    let _ = event_tx.send(JsWorkerEvent::WorkerError(err));
                    return;
                }
            };

            while let Ok(cmd) = cmd_rx.recv() {
                match cmd {
                    JsWorkerCommand::UpdateSnapshots(snap) => {
                        ctx.engine.update_attr_snapshot(snap.attr_snap);
                        ctx.engine.update_tag_snapshot(snap.tag_snap);
                        ctx.engine.update_transform_snapshot(
                            snap.positions,
                            snap.rotations,
                            snap.scales,
                            snap.global_positions,
                        );
                        ctx.engine
                            .update_hierarchy_snapshot(snap.parents, snap.children);
                    }
                    JsWorkerCommand::EvalScript { url, code } => {
                        if ctx.loaded_scripts.contains(&url) {
                            let _ = event_tx.send(JsWorkerEvent::EvalResult {
                                url,
                                already_loaded: true,
                                error: None,
                            });
                            continue;
                        }

                        let wrapped_code = format!("(function(){{\n{}\n}})();", code);
                        match ctx.engine.eval(&wrapped_code) {
                            Ok(_) => {
                                ctx.loaded_scripts.insert(url.clone());
                                let _ = event_tx.send(JsWorkerEvent::EvalResult {
                                    url,
                                    already_loaded: false,
                                    error: None,
                                });
                            }
                            Err(e) => {
                                let _ = event_tx.send(JsWorkerEvent::EvalResult {
                                    url,
                                    already_loaded: false,
                                    error: Some(e.to_string()),
                                });
                            }
                        }
                    }
                    JsWorkerCommand::Tick { elapsed_ms } => {
                        ctx.engine.fire_raf(elapsed_ms);
                        let tick_data = JsTickData {
                            logs: ctx.engine.drain_logs(),
                            attr_updates: ctx.engine.drain_attr_updates(),
                            pos_updates: ctx.engine.drain_transform_position_updates(),
                            rot_updates: ctx.engine.drain_transform_rotation_updates(),
                            scale_updates: ctx.engine.drain_transform_scale_updates(),
                            creation_queue: ctx.engine.drain_element_creation_queue(),
                            hierarchy_queue: ctx.engine.drain_hierarchy_append_queue(),
                            remove_queue: ctx.engine.drain_remove_element_queue(),
                            fetch_queue: ctx.engine.drain_fetch_queue(),
                            navigate_queue: ctx.engine.drain_navigate_queue(),
                        };
                        let _ = event_tx.send(JsWorkerEvent::TickData(tick_data));
                    }
                    JsWorkerCommand::PushElementCreationResults(results) => {
                        for (request_id, node_id) in results {
                            ctx.engine.push_element_creation_result(request_id, node_id);
                        }
                    }
                    JsWorkerCommand::PushFetchResults(results) => {
                        for (request_id, result) in results {
                            ctx.engine.push_fetch_result(request_id, result);
                        }
                    }
                    JsWorkerCommand::Shutdown => break,
                }
            }
        })
        .map_err(|e| format!("failed to spawn JS worker for space {}: {}", space_id, e))?;

    Ok(SpaceScriptWorker {
        cmd_tx,
        event_rx,
        join: Some(join),
    })
}

fn stop_space_worker(worker: &mut SpaceScriptWorker) {
    let _ = worker.cmd_tx.send(JsWorkerCommand::Shutdown);
    if let Some(join) = worker.join.take() {
        let _ = join.join();
    }
}

fn filter_snapshot_map<T: Clone>(source: &HashMap<i32, T>, allowed: &HashSet<i32>) -> HashMap<i32, T> {
    source
        .iter()
        .filter(|(node_id, _)| allowed.contains(node_id))
        .map(|(node_id, value)| (*node_id, value.clone()))
        .collect()
}


// --------------------------------------------------------------------------------------
// JS: Evaluar scripts pendientes
// EXCLUSIVE SYSTEM - must run on main thread
// --------------------------------------------------------------------------------------
fn js_eval_pending_scripts(world: &mut World) {
    const MAX_SCRIPTS_PER_FRAME: usize = 2;
    const MAX_ENQUEUE_BUDGET_MS: f32 = 1.5;

    let pending_scripts = {
        let Some(mut pending) = world.get_resource_mut::<PendingScripts>() else {
            return;
        };
        pending.0.drain(..).collect::<Vec<_>>()
    };

    if pending_scripts.is_empty() {
        return;
    }

    let enqueue_start = Instant::now();
    let mut queued_this_frame = 0usize;
    let mut deferred_scripts: Vec<(u32, String, String)> = Vec::new();
    let mut log_messages = Vec::new();

    for (space_id, url, code) in pending_scripts {
        let elapsed_ms = enqueue_start.elapsed().as_secs_f32() * 1000.0;
        if queued_this_frame >= MAX_SCRIPTS_PER_FRAME || elapsed_ms >= MAX_ENQUEUE_BUDGET_MS {
            deferred_scripts.push((space_id, url, code));
            continue;
        }

        queued_this_frame += 1;

        let enqueue_result = {
            let Some(mut manager) = world.get_non_send_resource_mut::<ScriptRuntimeManager>() else {
                return;
            };
            if let Some(worker) = manager.contexts.get_mut(&space_id) {
                worker
                    .cmd_tx
                    .send(JsWorkerCommand::EvalScript {
                        url: url.clone(),
                        code,
                    })
                    .map_err(|e| e.to_string())
            } else {
                Err(format!("missing JS context for space {}", space_id))
            }
        };

        match enqueue_result {
            Ok(_) => {
                log_messages.push(LogEntry::new(
                    LogLevel::Info,
                    format!("[JS][space:{}] Script encolado: {}", space_id, url),
                ));
            }
            Err(err) => {
                log_messages.push(LogEntry::new(
                    LogLevel::Error,
                    format!("[JS][space:{}] Error encolando {}: {}", space_id, url, err),
                ));
            }
        }
    }

    if !deferred_scripts.is_empty() {
        let deferred_count = deferred_scripts.len();
        {
            let Some(mut pending) = world.get_resource_mut::<PendingScripts>() else {
                return;
            };
            pending.0.extend(deferred_scripts.into_iter());
        }
        log_messages.push(LogEntry::new(
            LogLevel::Info,
            format!(
                "[JS] Throttle de eval: {} script(s) diferidos al siguiente frame",
                deferred_count
            ),
        ));
    }

    if !log_messages.is_empty() {
        let Some(mut log_panel) = world.get_resource_mut::<LogPanel>() else {
            return;
        };
        for entry in log_messages {
            match entry.level {
                LogLevel::Error => log_panel.push_error(entry.message),
                LogLevel::Warn => log_panel.push_warn(entry.message),
                LogLevel::Info => log_panel.push_info(entry.message),
            }
        }
    }
}

// --------------------------------------------------------------------------------------
// JS: Update snapshots (called before JS code runs)
// EXCLUSIVE SYSTEM - must run on main thread
// --------------------------------------------------------------------------------------
fn js_update_snapshots_system(world: &mut World) {
    let (
        attr_snap,
        tag_snap,
        positions,
        rotations,
        scales,
        global_positions,
        parents,
        children_map,
        space_subtrees,
    ) = {
        let Some(specs_world) = world.get_resource::<ElemenetWorld>() else {
            return;
        };
        let entities = specs_world.0.entities();
        let attrs_storage = specs_world.0.read_storage::<Attrs>();
        let tags_storage = specs_world.0.read_storage::<Tag>();
        let transforms_storage = specs_world.0.read_storage::<Transform2>();
        let hierarchies_storage = specs_world.0.read_storage::<Hierarchy>();

        let mut attr_snap = HashMap::new();
        for (ent, attrs) in (&entities, &attrs_storage).join() {
            let mut map = HashMap::new();
            for (k, v) in &attrs.0 {
                map.insert(k.clone(), v.clone());
            }
            attr_snap.insert(ent.id() as i32, map);
        }

        let mut tag_snap = HashMap::new();
        for (ent, tag) in (&entities, &tags_storage).join() {
            tag_snap.insert(ent.id() as i32, tag.0.clone());
        }

        let mut positions = HashMap::new();
        let mut rotations = HashMap::new();
        let mut scales = HashMap::new();
        let mut global_positions = HashMap::new();
        for (ent, tr) in (&entities, &transforms_storage).join() {
            use js_runtime::Vec3;
            positions.insert(ent.id() as i32, Vec3 { x: tr.position.x, y: tr.position.y, z: tr.position.z });
            rotations.insert(ent.id() as i32, Vec3 { x: tr.rotation.x, y: tr.rotation.y, z: tr.rotation.z });
            scales.insert(ent.id() as i32, Vec3 { x: tr.scale.x, y: tr.scale.y, z: tr.scale.z });
            global_positions.insert(ent.id() as i32, Vec3 { x: tr.position.x, y: tr.position.y, z: tr.position.z });
        }

        let mut parents = HashMap::new();
        let mut children_map = HashMap::new();
        for (ent, hier) in (&entities, &hierarchies_storage).join() {
            if let Some(parent_id) = hier.parent {
                parents.insert(ent.id() as i32, parent_id as i32);
            } else {
                parents.insert(ent.id() as i32, -1);
            }
            let child_ids: Vec<i32> = hier.children.iter().map(|child_ent| child_ent.id() as i32).collect();
            children_map.insert(ent.id() as i32, child_ids);
        }

        let mut space_subtrees: HashMap<u32, HashSet<i32>> = HashMap::new();
        for (node_id, tag_name) in &tag_snap {
            if tag_name != "space" {
                continue;
            }
            let mut set = HashSet::new();
            let mut stack = vec![*node_id];
            while let Some(curr) = stack.pop() {
                if !set.insert(curr) {
                    continue;
                }
                if let Some(children) = children_map.get(&curr) {
                    for child in children {
                        stack.push(*child);
                    }
                }
            }
            space_subtrees.insert(*node_id as u32, set);
        }

        (
            attr_snap,
            tag_snap,
            positions,
            rotations,
            scales,
            global_positions,
            parents,
            children_map,
            space_subtrees,
        )
    };

    let active_space_ids: HashSet<u32> = space_subtrees.keys().copied().collect();
    let mut removed_contexts = Vec::new();
    let mut created_contexts = Vec::new();
    let mut errors = Vec::new();

    {
        let Some(mut manager) = world.get_non_send_resource_mut::<ScriptRuntimeManager>() else {
            return;
        };

        let stale_ids: Vec<u32> = manager
            .contexts
            .keys()
            .copied()
            .filter(|space_id| !active_space_ids.contains(space_id))
            .collect();
        for space_id in stale_ids {
            if let Some(mut worker) = manager.contexts.remove(&space_id) {
                stop_space_worker(&mut worker);
                removed_contexts.push(space_id);
            }
        }

        for space_id in &active_space_ids {
            if manager.contexts.contains_key(space_id) {
                continue;
            }
            match spawn_space_worker(*space_id) {
                Ok(worker) => {
                    manager.contexts.insert(*space_id, worker);
                    created_contexts.push(*space_id);
                }
                Err(err) => {
                    errors.push(err);
                }
            }
        }

        let mut broken_contexts = Vec::new();
        for (space_id, worker) in manager.contexts.iter_mut() {
            let Some(allowed) = space_subtrees.get(space_id) else {
                continue;
            };

            let filtered_attrs = filter_snapshot_map(&attr_snap, allowed);
            let filtered_tags = filter_snapshot_map(&tag_snap, allowed);
            let filtered_positions = filter_snapshot_map(&positions, allowed);
            let filtered_rotations = filter_snapshot_map(&rotations, allowed);
            let filtered_scales = filter_snapshot_map(&scales, allowed);
            let filtered_global_positions = filter_snapshot_map(&global_positions, allowed);

            let mut filtered_parents = HashMap::new();
            let mut filtered_children = HashMap::new();
            for node_id in allowed {
                let parent = parents.get(node_id).copied().unwrap_or(-1);
                let normalized_parent = if parent >= 0 && !allowed.contains(&parent) {
                    -1
                } else {
                    parent
                };
                filtered_parents.insert(*node_id, normalized_parent);

                let children = children_map
                    .get(node_id)
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|child_id| allowed.contains(child_id))
                    .collect::<Vec<_>>();
                filtered_children.insert(*node_id, children);
            }

            let snap = SpaceSnapshots {
                attr_snap: filtered_attrs,
                tag_snap: filtered_tags,
                positions: filtered_positions,
                rotations: filtered_rotations,
                scales: filtered_scales,
                global_positions: filtered_global_positions,
                parents: filtered_parents,
                children: filtered_children,
            };

            if let Err(e) = worker.cmd_tx.send(JsWorkerCommand::UpdateSnapshots(snap)) {
                errors.push(format!(
                    "failed to send snapshots to JS worker for space {}: {}",
                    space_id, e
                ));
                broken_contexts.push(*space_id);
            }
        }

        for space_id in broken_contexts {
            if let Some(mut worker) = manager.contexts.remove(&space_id) {
                stop_space_worker(&mut worker);
                removed_contexts.push(space_id);
            }
        }
    }

    if !removed_contexts.is_empty() || !created_contexts.is_empty() || !errors.is_empty() {
        let Some(mut log_panel) = world.get_resource_mut::<LogPanel>() else {
            return;
        };

        for space_id in removed_contexts {
            log_panel.push_info(format!(
                "[JS][space:{}] Context destroyed (space removed)",
                space_id
            ));
        }
        for space_id in created_contexts {
            log_panel.push_info(format!("[JS][space:{}] Context created", space_id));
        }
        for err in errors {
            log_panel.push_error(format!("[JS] {}", err));
        }
    }
}

// --------------------------------------------------------------------------------------
// JS: Tick cada frame (fire RAF, drain logs, drain all updates)
// EXCLUSIVE SYSTEM - must run on main thread
// --------------------------------------------------------------------------------------
fn js_tick_system(world: &mut World) {
    let elapsed_ms = {
        let Some(time) = world.get_resource::<Time>() else {
            return;
        };
        time.elapsed_seconds_f64() * 1000.0
    };
    let mut eval_events: Vec<(u32, String, bool, Option<String>)> = Vec::new();
    let mut tick_batches: Vec<(u32, JsTickData)> = Vec::new();
    let mut worker_errors: Vec<String> = Vec::new();

    {
        let Some(mut manager) = world.get_non_send_resource_mut::<ScriptRuntimeManager>() else {
            return;
        };

        for worker in manager.contexts.values_mut() {
            let _ = worker.cmd_tx.send(JsWorkerCommand::Tick { elapsed_ms });
        }

        let mut broken_contexts = Vec::new();
        for (space_id, worker) in manager.contexts.iter_mut() {
            loop {
                match worker.event_rx.try_recv() {
                    Ok(JsWorkerEvent::EvalResult {
                        url,
                        already_loaded,
                        error,
                    }) => {
                        eval_events.push((*space_id, url, already_loaded, error));
                    }
                    Ok(JsWorkerEvent::TickData(data)) => {
                        tick_batches.push((*space_id, data));
                    }
                    Ok(JsWorkerEvent::WorkerError(err)) => {
                        worker_errors.push(format!("[JS][space:{}] {}", space_id, err));
                    }
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        broken_contexts.push(*space_id);
                        break;
                    }
                }
            }
        }

        for space_id in broken_contexts {
            if let Some(mut worker) = manager.contexts.remove(&space_id) {
                stop_space_worker(&mut worker);
                worker_errors.push(format!(
                    "[JS][space:{}] worker disconnected and context removed",
                    space_id
                ));
            }
        }
    }

    if !eval_events.is_empty() || !worker_errors.is_empty() {
        let Some(mut log_panel) = world.get_resource_mut::<LogPanel>() else {
            return;
        };
        for (space_id, url, already_loaded, error) in eval_events {
            if already_loaded {
                log_panel.push_info(format!(
                    "[JS][space:{}] Script ya cargado, omitiendo: {}",
                    space_id, url
                ));
            } else if let Some(err) = error {
                log_panel.push_error(format!(
                    "[JS][space:{}] Error evaluando {}: {}",
                    space_id, url, err
                ));
            } else {
                log_panel.push_info(format!(
                    "[JS][space:{}] Script evaluado OK: {}",
                    space_id, url
                ));
            }
        }
        for err in worker_errors {
            log_panel.push_error(err);
        }
    }

    let mut logs_by_context = Vec::new();
    let mut attr_updates = Vec::new();
    let mut pos_updates = Vec::new();
    let mut rot_updates = Vec::new();
    let mut scale_updates = Vec::new();
    let mut creation_batches = Vec::new();
    let mut hierarchy_batches = Vec::new();
    let mut remove_batches = Vec::new();
    let mut fetch_batches = Vec::new();
    let mut navigate_batches = Vec::new();

    for (space_id, data) in tick_batches {
        if !data.logs.is_empty() {
            logs_by_context.push((space_id, data.logs));
        }
        if !data.creation_queue.is_empty() {
            creation_batches.push((space_id, data.creation_queue));
        }
        if !data.hierarchy_queue.is_empty() {
            hierarchy_batches.push((space_id, data.hierarchy_queue));
        }
        if !data.remove_queue.is_empty() {
            remove_batches.push((space_id, data.remove_queue));
        }
        if !data.fetch_queue.is_empty() {
            fetch_batches.push((space_id, data.fetch_queue));
        }
        if !data.navigate_queue.is_empty() {
            navigate_batches.push((space_id, data.navigate_queue));
        }

        attr_updates.extend(data.attr_updates);
        pos_updates.extend(data.pos_updates);
        rot_updates.extend(data.rot_updates);
        scale_updates.extend(data.scale_updates);
    }

    {
        let Some(mut log_panel) = world.get_resource_mut::<LogPanel>() else {
            return;
        };
        for (space_id, logs) in logs_by_context {
            for (level, msg) in logs {
                match level.as_str() {
                    "warn" => log_panel.push_warn(format!("[JS][space:{}] {}", space_id, msg)),
                    "error" => log_panel.push_error(format!("[JS][space:{}] {}", space_id, msg)),
                    _ => log_panel.push_info(format!("[JS][space:{}] {}", space_id, msg)),
                }
            }
        }
    }

    {
        let Some(mut attribute_updates) = world.get_resource_mut::<AttributeUpdates>() else {
            return;
        };

        for (node_id, key, value) in attr_updates {
            attribute_updates.0.push((node_id as u32, key, value));
        }
        for (node_id, pos) in pos_updates {
            attribute_updates.0.push((node_id as u32, "x".to_string(), pos.x.to_string()));
            attribute_updates.0.push((node_id as u32, "y".to_string(), pos.y.to_string()));
            attribute_updates.0.push((node_id as u32, "z".to_string(), pos.z.to_string()));
        }
        for (node_id, rot) in rot_updates {
            attribute_updates.0.push((node_id as u32, "rx".to_string(), rot.x.to_string()));
            attribute_updates.0.push((node_id as u32, "ry".to_string(), rot.y.to_string()));
            attribute_updates.0.push((node_id as u32, "rz".to_string(), rot.z.to_string()));
        }
        for (node_id, scale) in scale_updates {
            attribute_updates.0.push((node_id as u32, "sx".to_string(), scale.x.to_string()));
            attribute_updates.0.push((node_id as u32, "sy".to_string(), scale.y.to_string()));
            attribute_updates.0.push((node_id as u32, "sz".to_string(), scale.z.to_string()));
        }
    }

    for (space_id, creation_queue) in creation_batches {
        use virtual_dom::dom::element::Vec3 as DomVec3;

        let (creation_results, created_node_ids, log_messages) = {
            let Some(mut specs_world) = world.get_resource_mut::<ElemenetWorld>() else {
                return;
            };

            let mut creation_results = Vec::new();
            let mut created_node_ids = Vec::new();
            let mut log_messages = Vec::new();

            for (request_id, tag_name) in creation_queue {
                let new_ent = {
                    let entities = specs_world.0.entities();
                    entities.create()
                };

                let new_ent_id = {
                    let mut tags_storage = specs_world.0.write_storage::<Tag>();
                    let mut attrs_storage = specs_world.0.write_storage::<Attrs>();
                    let mut transform_storage = specs_world.0.write_storage::<Transform2>();
                    let mut hier_storage = specs_world.0.write_storage::<Hierarchy>();

                    tags_storage.insert(new_ent, Tag(tag_name.clone())).ok();
                    attrs_storage.insert(new_ent, Attrs(HashMap::new())).ok();
                    transform_storage.insert(new_ent, Transform2 {
                        position: DomVec3 { x: 0.0, y: 0.0, z: 0.0 },
                        rotation: DomVec3 { x: 0.0, y: 0.0, z: 0.0 },
                        scale: DomVec3 { x: 1.0, y: 1.0, z: 1.0 },
                    }).ok();
                    hier_storage.insert(new_ent, Hierarchy { parent: None, children: Vec::new() }).ok();

                    new_ent.id()
                };

                creation_results.push((request_id, new_ent_id as i32));
                created_node_ids.push(new_ent_id);
                log_messages.push(format!(
                    "[JS][space:{}] createElement('{}') -> node_id={}",
                    space_id, tag_name, new_ent_id
                ));
            }

            (creation_results, created_node_ids, log_messages)
        };

        {
            let Some(mut dirty_nodes) = world.get_resource_mut::<DirtyNodes>() else {
                return;
            };
            dirty_nodes.0.extend(created_node_ids);
        }
        {
            let Some(mut log_panel) = world.get_resource_mut::<LogPanel>() else {
                return;
            };
            for msg in log_messages {
                log_panel.push_info(msg);
            }
        }
        {
            let Some(mut manager) = world.get_non_send_resource_mut::<ScriptRuntimeManager>() else {
                return;
            };
            if let Some(worker) = manager.contexts.get_mut(&space_id) {
                let _ = worker
                    .cmd_tx
                    .send(JsWorkerCommand::PushElementCreationResults(creation_results));
            }
        }
    }

    for (space_id, hierarchy_queue) in hierarchy_batches {
        let (dirty_child_ids, log_messages) = {
            let Some(mut specs_world) = world.get_resource_mut::<ElemenetWorld>() else {
                return;
            };

            let mut dirty_child_ids = Vec::new();
            let mut log_messages = Vec::new();

            for (parent_id, child_id) in hierarchy_queue {
                let (parent_ent, child_ent, are_alive) = {
                    let entities = specs_world.0.entities();
                    let parent_ent = entities.entity(parent_id as u32);
                    let child_ent = entities.entity(child_id as u32);
                    let are_alive = entities.is_alive(parent_ent) && entities.is_alive(child_ent);
                    (parent_ent, child_ent, are_alive)
                };

                if are_alive {
                    Hierarchy::add_child(&mut specs_world.0, parent_ent, child_ent);
                    dirty_child_ids.push(child_id as u32);
                    log_messages.push(format!(
                        "[JS][space:{}] appendChild: parent={} child={}",
                        space_id, parent_id, child_id
                    ));
                }
            }

            (dirty_child_ids, log_messages)
        };

        {
            let Some(mut dirty_nodes) = world.get_resource_mut::<DirtyNodes>() else {
                return;
            };
            dirty_nodes.0.extend(dirty_child_ids);
        }
        {
            let Some(mut log_panel) = world.get_resource_mut::<LogPanel>() else {
                return;
            };
            for msg in log_messages {
                log_panel.push_info(msg);
            }
        }
    }

    for (space_id, remove_queue) in remove_batches {
        let log_messages = {
            let Some(mut specs_world) = world.get_resource_mut::<ElemenetWorld>() else {
                return;
            };
            let mut log_messages = Vec::new();
            for node_id in remove_queue {
                let (ent, is_alive) = {
                    let entities = specs_world.0.entities();
                    let ent = entities.entity(node_id as u32);
                    (ent, entities.is_alive(ent))
                };
                if is_alive {
                    specs_world.0.delete_entity(ent).ok();
                    log_messages.push(format!("[JS][space:{}] remove: node_id={}", space_id, node_id));
                }
            }
            log_messages
        };

        let Some(mut log_panel) = world.get_resource_mut::<LogPanel>() else {
            return;
        };
        for msg in log_messages {
            log_panel.push_info(msg);
        }
    }

    for (space_id, fetch_queue) in fetch_batches {
        let fetch_results = {
            let Some(tokio_rt) = world.get_resource::<TokioRuntime>() else {
                return;
            };

            let mut fetch_results = Vec::new();
            for (request_id, url) in &fetch_queue {
                let result = tokio_rt.0.block_on(async {
                    match reqwest::get(url).await {
                        Ok(resp) => match resp.text().await {
                            Ok(text) => Ok(text),
                            Err(e) => Err(format!("Failed to read response: {}", e)),
                        },
                        Err(e) => Err(format!("HTTP error: {}", e)),
                    }
                });
                fetch_results.push((*request_id, result));
            }
            fetch_results
        };

        {
            let Some(mut log_panel) = world.get_resource_mut::<LogPanel>() else {
                return;
            };
            for (_, url) in &fetch_queue {
                log_panel.push_info(format!("[JS][space:{}] fetch: {}", space_id, url));
            }
        }
        {
            let Some(mut manager) = world.get_non_send_resource_mut::<ScriptRuntimeManager>() else {
                return;
            };
            if let Some(worker) = manager.contexts.get_mut(&space_id) {
                let _ = worker
                    .cmd_tx
                    .send(JsWorkerCommand::PushFetchResults(fetch_results));
            }
        }
    }

    if !navigate_batches.is_empty() {
        {
            let Some(mut log_panel) = world.get_resource_mut::<LogPanel>() else {
                return;
            };
            for (space_id, urls) in &navigate_batches {
                for url in urls {
                    log_panel.push_warn(format!("[JS][space:{}] navigate: {}", space_id, url));
                }
            }
        }
        {
            let Some(mut reload_trigger) = world.get_resource_mut::<ReloadTrigger>() else {
                return;
            };
            reload_trigger.0 = true;
        }
    }
}

// --------------------------------------------------------------------------------------
// XR SESSION MANAGEMENT
// --------------------------------------------------------------------------------------
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
        // Switching to VR: disable desktop camera and start XR session
        for mut cam in desktop_cameras.iter_mut() {
            cam.is_active = false;
        }
        if *xr_state == XrState::Available {
            create_session.send_default();
        }
    } else {
        // Switching to Desktop: enable desktop camera and request XR exit
        for mut cam in desktop_cameras.iter_mut() {
            cam.is_active = true;
        }
        if *xr_state == XrState::Running || *xr_state == XrState::Ready {
            request_exit.send_default();
        }
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

    // Usamos Vec3 como acumulador
    let mut direction = Vec3::ZERO;

    // forward / right ahora son Dir3 -> los convertimos a Vec3
    let forward = transform.forward().as_vec3();
    let right = transform.right().as_vec3();
    let up = Vec3::Y;

    let speed = 5.0;

    // WASD para moverte en el plano
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

    // Q/E para subir/bajar
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
