use bevy::{
    asset::AssetPlugin,
    diagnostic::FrameTimeDiagnosticsPlugin,
    prelude::*,
    window::{PresentMode, PrimaryWindow, Window},
};
use bevy_egui::{egui, EguiContexts, EguiPlugin};
use specs::{Entity as SpecEntity, Join, ReadStorage, World as SpecWorld, WorldExt};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    time::Instant,
};
use tokio::runtime::Runtime;
use url::Url;

use base64::{engine::general_purpose::STANDARD as Base64Engine, Engine as _};
use virtual_dom::{
    dom::{
        element::{build_world, Attrs, Hierarchy, Tag, Transform2},
        hsml::Model,
    },
    load_xml_from_url, parse_xml,
};
use anyhow::Result;

// Módulos ficticios
mod render;
mod utils;
use render::apply_transform;
use utils::shapes;

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

/// Medición de performance
#[derive(Resource, Default)]
struct PerformanceStats {
    dom_sync_ms: f32,
}

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

const ATTR_DELETE_SENTINEL: &str = "[DEL]";

// --------------------------------------------------------------------------------------
// MAIN
// --------------------------------------------------------------------------------------
fn main() {
    App::new()
        .add_plugins(
            DefaultPlugins
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
                }),
        )
        .add_plugins(EguiPlugin)
        .add_plugins(FrameTimeDiagnosticsPlugin)
        // Nuestros recursos
        .insert_resource(VirtualDomData::default())
        .insert_resource(DirtyNodes::default())
        .insert_resource(EntityMap::default())
        .insert_resource(ElemenetWorld(build_world()))
        .insert_resource(EntityCounter::default())
        .insert_resource(FpsCounter::default())
        .insert_resource(CurrentUrl(
            "http://localhost:2052/static/main.hsml".to_string(),
        ))
        .insert_resource(ReloadTrigger(false))
        .insert_resource(AttributeUpdates::default())
        .insert_resource(DeleteRequests::default())
        .insert_resource(DevtoolVisible(true))
        .insert_resource(LogPanel::default())
        // Runtime
        .insert_resource(TokioRuntime(
            Runtime::new().expect("No se pudo crear Tokio"),
        ))
        // Cache
        .insert_resource(ModelCache::default())
        // Stats
        .insert_resource(PerformanceStats::default())
        // Estado del devtool
        .insert_resource(DevtoolState::default())
        // Sistemas
        .add_systems(Startup, setup)
        .add_systems(
            Update,
            (
                // Recarga de XML
                reload_xml_system.run_if(|r: Res<ReloadTrigger>| r.0),
                // Aplicar updates a atributos
                apply_attribute_updates.run_if(|a: Res<AttributeUpdates>| !a.0.is_empty()),
                // Marcar dirty
                mark_dirty_system,
                // Sincronizar con Bevy solo si hay nodos dirty
                dom_sync_system.run_if(|d: Res<DirtyNodes>| !d.0.is_empty()),
                // Siempre mostrar la UI, y dentro ya decidimos si mostramos el devtool
                ui_system,
                // Borrar
                process_delete_requests.run_if(|del: Res<DeleteRequests>| !del.0.is_empty()),
                // Contador de entidades
                update_entity_counter.run_if(|m: Res<EntityMap>| m.is_changed()),
                // FPS
                update_fps_counter,
            ),
        )
        .run();
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
) {
    // Cámara 3D
    commands.spawn(Camera3dBundle {
        camera: Camera {
            order: 0,
            ..default()
        },
        transform: Transform::from_xyz(0.0, 3.0, 8.0).looking_at(Vec3::ZERO, Vec3::Y),
        ..default()
    });

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
    let default_material = materials.add(StandardMaterial {
        base_color: Color::rgb(0.5, 0.8, 0.8),
        ..default()
    });
    commands.insert_resource(SharedResources {
        cube_mesh,
        default_material,
    });

    // Cargar XML inicial
    match load_and_flatten_xml(
        &mut world.0,
        "http://localhost:2052/static/main.hsml",
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
    log_panel.push_info(format!("Intentando descargar XML desde: {url}"));

    let xml_content = match rt.block_on(load_xml_from_url(url)) {
        Ok(c) => c,
        Err(e) => {
            return Err(anyhow::anyhow!("Error al descargar XML desde {url}: {e}"));
        }
    };

    log_panel.push_info(format!(
        "Descarga OK. Longitud de XML: {} caracteres",
        xml_content.len()
    ));

    let root_node = match parse_xml(world, &xml_content) {
        Ok(r) => r,
        Err(e) => {
            return Err(anyhow::anyhow!("Error al parsear el XML: {e}"));
        }
    };

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
    entity_counter: Res<EntityCounter>,
    fps_counter: Res<FpsCounter>,
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
    perf_stats: Res<PerformanceStats>,
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
        // Se movieron la info de Entities, FPS y dom_sync al tab "Status".
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
                        // Resolución de la ventana
                        // if let Ok(window) = windows.get_single() {
                        //     let w = window.resolution.physical_width();
                        //     let h = window.resolution.physical_height();
                        //     ui.label(format!("Resolución: {} x {}", w, h));
                        // } else {
                        //     ui.label("No se pudo obtener la ventana principal.");
                        // }

                        ui.label(format!("Entities: {}", entity_counter.count));
                        ui.label(format!("FPS: {}", fps_counter.fps));
                        ui.label(format!("Último dom_sync: {:.2} ms", perf_stats.dom_sync_ms));
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

                        if ui.button("Limpiar logs").clicked() {
                            log_panel.clear();
                        }
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
fn apply_attribute_updates(
    mut attribute_updates: ResMut<AttributeUpdates>,
    mut world: ResMut<ElemenetWorld>,
    mut dirty_nodes: ResMut<DirtyNodes>,
) {
    let entities = world.0.entities();

    let mut attrs_storage = world.0.write_storage::<Attrs>();
    let mut tr_storage    = world.0.write_storage::<Transform2>();

    for (ent_id, key, val) in attribute_updates.0.drain(..) {
        let ent = entities.entity(ent_id);
        if !entities.is_alive(ent) {
            continue;
        }

        // Asegurar que haya Attrs
        if attrs_storage.get(ent).is_none() {
            let _ = attrs_storage.insert(ent, Attrs(HashMap::new()));
        }

        if let Some(a) = attrs_storage.get_mut(ent) {
            if val == ATTR_DELETE_SENTINEL {
                // --- eliminar atributo ---
                a.0.remove(&key);

                // (Opcional) si querés "revertir" efectos en Transform2 cuando
                // se borran keys como x/y/z/sx/sy/sz/rx/ry/rz, podés resetear:
                if let Some(tr) = tr_storage.get_mut(ent) {
                    match key.as_str() {
                        // posición
                        "x" => tr.position.x = 0.0, // o 0.0 si querés resetear
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
                // --- set/update atributo ---
                a.0.insert(key.clone(), val.clone());

                // reflejar en Transform2 si corresponde
                if let Some(tr) = tr_storage.get_mut(ent) {
                    let parse_f32 = || -> Option<f32> { val.trim().parse::<f32>().ok() };
                    match key.as_str() {
                        "x" => if let Some(f) = parse_f32() { tr.position.x = f; },
                        "y" => if let Some(f) = parse_f32() { tr.position.y = f; },
                        "z" => if let Some(f) = parse_f32() { tr.position.z = f; },

                        "rx" => if let Some(f) = parse_f32() { tr.rotation.x = f; },
                        "ry" => if let Some(f) = parse_f32() { tr.rotation.y = f; },
                        "rz" => if let Some(f) = parse_f32() { tr.rotation.z = f; },

                        "s"  => if let Some(f) = parse_f32() {
                            tr.scale.x = f; tr.scale.y = f; tr.scale.z = f;
                        }
                        "sx" => if let Some(f) = parse_f32() { tr.scale.x = f; },
                        "sy" => if let Some(f) = parse_f32() { tr.scale.y = f; },
                        "sz" => if let Some(f) = parse_f32() { tr.scale.z = f; },

                        _ => {}
                    }
                }
            }
        }

        // Marcar dirty (y que `dom_sync_system` drene después)
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
// DOM -> Bevy + medición de tiempo
// --------------------------------------------------------------------------------------
fn dom_sync_system(
    world: Res<ElemenetWorld>,
    mut commands: Commands,
    dom_data: Res<VirtualDomData>,
    shared_resources: Res<SharedResources>,
    mut entity_map: ResMut<EntityMap>,
    mut dirty_nodes: ResMut<DirtyNodes>,
    mut query: Query<(Entity, &mut Transform, Option<&Dirty>)>,
    asset_server: Res<AssetServer>,
    mut log_panel: ResMut<LogPanel>,
    mut model_cache: ResMut<ModelCache>,
    tokio_rt: Res<TokioRuntime>,
    current_url: Res<CurrentUrl>,
    mut perf_stats: ResMut<PerformanceStats>,
) {
    let start_time = Instant::now();

    if dirty_nodes.0.is_empty() {
        return;
    }

    let tags = world.0.read_storage::<Tag>();
    let transforms = world.0.read_storage::<Transform2>();
    let hierarchies = world.0.read_storage::<Hierarchy>();
    let models = world.0.read_storage::<Model>();

    log_panel.push_info(format!(
        "dom_sync_system: Procesando {} dirty nodes...",
        dirty_nodes.0.len()
    ));

    for node_id in dirty_nodes.0.drain(..) {
        log_panel.push_info(format!("  Revisando node_id={}", node_id));
        if let Some(node) = dom_data.nodes.get(&node_id) {
            // Tag
            let tag = tags.get(*node).map(|t| t.0.clone()).unwrap_or_default();
            log_panel.push_info(format!("    Tag='{}'", tag));

            let hierarchy = hierarchies.get(*node);
            let parent_id = hierarchy.and_then(|h| h.parent);

            // Calculamos la Transform de Specs
            let mut transform_b = Transform::default();
            if let Some(tr2) = transforms.get(*node) {
                apply_transform(tr2, &mut transform_b);
            }

            // ¿existe la entidad de Bevy asociada a este node_id?
            if let Some(&bevy_ent) = entity_map.0.get(&node_id) {
                // Si existe, solo actualizamos su Transform si está marcado con Dirty
                if let Ok((_, mut t, dirty)) = query.get_mut(bevy_ent) {
                    *t = transform_b;
                    commands.entity(bevy_ent).remove::<Dirty>();
                    log_panel.push_info(format!(
                        "    Actualizado transform en entidad existente {:?}",
                        bevy_ent
                    ));
                }
            } else {
                // Crear nueva
                log_panel.push_info("    -> No existe, creando nueva entidad...");

                let new_ent = match tag.as_str() {
                    "model" => {
                        log_panel.push_info("    -> Tag='model'");
                        if let Some(model_data) = models.get(*node) {
                            if let Some(ref original_src) = model_data.src {
                                if let Some(final_url) =
                                    resolve_remote_path(&current_url.0, original_src)
                                {
                                    log_panel.push_info(format!("       final_url={}", final_url));
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
                    "script" | "space" => {
                        log_panel
                            .push_info("    -> script/space, no spawneamos nada 3D");
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
                        log_panel
                            .push_info("    -> include, no spawneamos nada 3D");
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
                    other => {
                        log_panel.push_info(format!("    -> Tag='{}', generamos un cubo", other));
                        let new_ent_empty = commands
                            .spawn((
                                SpatialBundle {
                                    transform: transform_b,
                                    ..Default::default()
                                },
                                Dirty,
                            ))
                            .id();
                        if false {
                            let child = commands
                                .spawn(PbrBundle {
                                    mesh: shared_resources.cube_mesh.clone(),
                                    material: shared_resources.default_material.clone(),
                                    transform: Transform::from_scale(Vec3::splat(0.2)),
                                    ..Default::default()
                                })
                                .id();
                            commands.entity(new_ent_empty).push_children(&[child]);
                        }
                        new_ent_empty
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
    for (ent, h) in (&world.entities(), &hier).join() {
        if h.parent.is_none() {
            return Some(ent);
        }
    }
    None
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

    // Crear carpeta cache
    let _ = fs::create_dir_all("crates/bevy_openxr/assets/cache");
    // Nombre base64
    let filename = encode_url_to_filename(url);
    let local_path = format!("crates/bevy_openxr/assets/cache/{}", filename);

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
                let scene_handle: Handle<Scene> = asset_server.load(final_path);
                scene_handle
            } else {
                // Carga normal como Scene
                let scene_handle: Handle<Scene> = asset_server.load(relative);
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
    if remote_path.starts_with("http://") || remote_path.starts_with("https://") {
        return Some(remote_path.to_string());
    }
    let Ok(base) = Url::parse(base_url) else {
        return None;
    };
    let Ok(final_url) = base.join(remote_path) else {
        return None;
    };
    Some(final_url.to_string())
}
