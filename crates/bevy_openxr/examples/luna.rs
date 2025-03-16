use bevy::{prelude::*, window::PresentMode, diagnostic::FrameTimeDiagnosticsPlugin};
use bevy_egui::{egui, EguiPlugin, EguiContexts};
use specs::{Entity as SpecEntity, Join, ReadStorage, World as SpecWorld, WorldExt};
use std::collections::HashMap;
use tokio::runtime::Runtime;
use virtual_dom::{
    dom::{element::{build_world, Attrs, Hierarchy, Tag, Transform2}, hsml::Model}, 
    load_xml_from_url, parse_xml
};
mod render;
mod utils;
use render::{apply_model, apply_transform};
use utils::shapes;

// Recursos existentes
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
struct EntityCounter { count: usize }

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

// Nuevos recursos para actualizaciones y eliminaciones
#[derive(Resource, Default)]
struct AttributeUpdates(Vec<(u32, String, String)>); // (entity_id, attr_key, new_value)

// Agregar un componente Dirty para marcar entidades que necesitan actualización
#[derive(Component)]
struct Dirty;

#[derive(Resource, Default)]
struct DeleteRequests(Vec<u32>); // IDs de entidades a eliminar

// Recurso para recursos compartidos
#[derive(Resource)]
struct SharedResources {
    cube_mesh: Handle<Mesh>,
    default_material: Handle<StandardMaterial>,
}
// Actualizar main para incluir el nuevo sistema
fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                present_mode: PresentMode::AutoNoVsync,
                ..default()
            }),
            ..default()
        }))
        .add_plugins(EguiPlugin)
        .add_plugins(FrameTimeDiagnosticsPlugin)
        .insert_resource(VirtualDomData::default())
        .insert_resource(DirtyNodes::default())
        .insert_resource(EntityMap::default())
        .insert_resource(ElemenetWorld(build_world()))
        .insert_resource(EntityCounter::default())
        .insert_resource(FpsCounter::default())
        .insert_resource(CurrentUrl("http://localhost:2052/static/main.hsml".to_string()))
        .insert_resource(ReloadTrigger(false))
        .insert_resource(AttributeUpdates::default())
        .insert_resource(DeleteRequests::default())
        .insert_resource(DevtoolVisible(true))
        .add_systems(Startup, setup)
        .add_systems(
            Update,
            (
                reload_xml_system.run_if(|reload_trigger: Res<ReloadTrigger>| reload_trigger.0),
                mark_dirty_system, // Nuevo sistema para marcar entidades sucias
                dom_sync_system.run_if(|dirty_nodes: Res<DirtyNodes>| !dirty_nodes.0.is_empty()),
                ui_system.run_if(|devtool: Res<DevtoolVisible>| devtool.0),
                apply_attribute_updates
                    .run_if(|attribute_updates: Res<AttributeUpdates>| !attribute_updates.0.is_empty()),
                process_delete_requests
                    .run_if(|delete_requests: Res<DeleteRequests>| !delete_requests.0.is_empty()),
                update_entity_counter.run_if(|entity_map: Res<EntityMap>| entity_map.is_changed()),
                update_fps_counter,
            ),
        )
        .run();
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut world: ResMut<ElemenetWorld>,
    mut dom_data: ResMut<VirtualDomData>,
    mut dirty_nodes: ResMut<DirtyNodes>,
    asset_server: Res<AssetServer>,
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

    // Cámara para UI 2D
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

    // Recursos compartidos
    let cube_mesh = meshes.add(shapes::create_cube());
    let default_material = materials.add(StandardMaterial {
        base_color: Color::rgb(0.5, 0.8, 0.8),
        ..Default::default()
    });
    commands.insert_resource(SharedResources {
        cube_mesh,
        default_material,
    });

    let (nodes, dirty) = load_and_flatten_xml(&mut world.0, "http://localhost:2052/static/main.hsml");
    dom_data.nodes = nodes;
    dirty_nodes.0 = dirty;
}

// Sistema de recarga optimizado
fn reload_xml_system(
    mut world: ResMut<ElemenetWorld>,
    mut dom_data: ResMut<VirtualDomData>,
    mut dirty_nodes: ResMut<DirtyNodes>,
    mut commands: Commands,
    mut entity_map: ResMut<EntityMap>,
    url: Res<CurrentUrl>,
    mut reload_trigger: ResMut<ReloadTrigger>,
) {
    if !reload_trigger.0 {
        return;
    }

    let (new_nodes, new_dirty) = load_and_flatten_xml(&mut world.0, &url.0);

    // Identificar nodos eliminados
    let old_nodes: Vec<u32> = dom_data.nodes.keys().cloned().collect();
    let mut entities_to_delete = Vec::new();
    for old_id in old_nodes {
        if !new_nodes.contains_key(&old_id) {
            if let Some(entity) = entity_map.0.remove(&old_id) {
                commands.entity(entity).despawn_recursive();
            }
            entities_to_delete.push(old_id);
        }
    }
    let entities_to_delete: Vec<_> = entities_to_delete
        .into_iter()
        .map(|old_id| world.0.entities().entity(old_id))
        .collect();
    for entity in entities_to_delete {
        world.0.delete_entity(entity).ok();
    }

    // Actualizar nodos existentes y agregar nuevos
    dom_data.nodes = new_nodes;
    dirty_nodes.0 = new_dirty;
    reload_trigger.0 = false;
}

fn load_and_flatten_xml(world: &mut SpecWorld, url: &str) -> (HashMap<u32, SpecEntity>, Vec<u32>) {
    let rt = Runtime::new().expect("No se pudo crear el runtime de Tokio");
    let xml_content = rt
        .block_on(load_xml_from_url(url))
        .expect("Error al cargar XML");
    
    let root_node: SpecEntity = parse_xml(world, &xml_content)
        .expect("Error al parsear el XML");
    
    let mut map = HashMap::new();
    let mut dirty = Vec::new();

    let hierarchys = world.read_storage::<Hierarchy>();
    
    fn flatten_dom(
        node: SpecEntity,
        map: &mut HashMap<u32, SpecEntity>,
        dirty: &mut Vec<u32>,
        hierarchys: &ReadStorage<Hierarchy>,
    ) {
        dirty.push(node.id());
        map.insert(node.id(), node.clone());
        if let Some(element) = hierarchys.get(node) {
            for child in &element.children {
                flatten_dom(child.clone(), map, dirty, hierarchys);
            }
        }
    }
    
    flatten_dom(root_node, &mut map, &mut dirty, &hierarchys);
    (map, dirty)
}

// Sistema de UI actualizado
fn ui_system(
    mut contexts: EguiContexts,
    mut url: ResMut<CurrentUrl>,
    mut reload_trigger: ResMut<ReloadTrigger>,
    entity_counter: Res<EntityCounter>,
    fps_counter: Res<FpsCounter>,
    mut devtool_visible: ResMut<DevtoolVisible>,
    world: Res<ElemenetWorld>,
    entity_map: Res<EntityMap>,
    mut commands: Commands,
    mut camera_query: Query<&mut Transform, With<Camera3d>>,
    dom_data: Res<VirtualDomData>,
    mut attribute_updates: ResMut<AttributeUpdates>,
    mut delete_requests: ResMut<DeleteRequests>,
) {
    egui::Window::new("Navegador").show(contexts.ctx_mut(), |ui| {
        ui.horizontal(|ui| {
            ui.label("URL:");
            let response = ui.text_edit_singleline(&mut url.0);
            if response.lost_focus() && response.ctx.input(|i| i.key_pressed(egui::Key::Enter)) {
                reload_trigger.0 = true;
            }
            if ui.button("Ir").clicked() || ui.button("Recargar").clicked() {
                reload_trigger.0 = true;
            }
        });
        ui.label(format!("Entities: {}", entity_counter.count));
        ui.label(format!("FPS: {}", fps_counter.fps));
        if ui.button("Toggle Devtool").clicked() {
            devtool_visible.0 = !devtool_visible.0;
        }
    });

    if devtool_visible.0 {
        egui::Window::new("Devtool").show(contexts.ctx_mut(), |ui| {
            ui.heading("Árbol de Elementos");
            ui.separator();
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
                );
            } else {
                ui.label("No hay elementos en la escena.");
            }
        });
    }
}

// Mostrar y editar el árbol de elementos
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
) {
    let hierarchies = world.read_storage::<Hierarchy>();
    let tags = world.read_storage::<Tag>();
    let attrs = world.read_storage::<Attrs>();
    let transforms = world.read_storage::<Transform2>();

    if let Some(tag) = tags.get(entity) {
        ui.collapsing(format!("{} (ID: {})", tag.0, entity.id()), |ui| {
            // Editar atributos
            if let Some(attrs) = attrs.get(entity) {
                ui.label("Atributos:");
                for (key, value) in &attrs.0 {
                    ui.horizontal(|ui| {
                        ui.label(key);
                        let mut val = value.clone();
                        if ui.text_edit_singleline(&mut val).changed() {
                            attribute_updates.0.push((entity.id(), key.clone(), val));
                        }
                    });
                }
            }

            // Botones "Mirar" y "Borrar"
            ui.horizontal(|ui| {
                if ui.button("Mirar").clicked() {
                    if let Some(transform) = transforms.get(entity) {
                        if let Ok(mut camera_transform) = camera_query.get_single_mut() {
                            let pos = Vec3::new(transform.position.x, transform.position.y, transform.position.z);
                            *camera_transform = Transform::from_translation(pos + Vec3::new(0.0, 3.0, 8.0))
                                .looking_at(pos, Vec3::Y);
                        }
                    }
                }
                if ui.button("Borrar").clicked() {
                    delete_requests.0.push(entity.id());
                }
            });

            // Mostrar hijos
            if let Some(hierarchy) = hierarchies.get(entity) {
                for child in &hierarchy.children {
                    show_element_tree(ui, *child, world, entity_map, commands, camera_query, dom_data, attribute_updates, delete_requests);
                }
            }
        });
    }
}

// Sistema para aplicar actualizaciones de atributos
fn apply_attribute_updates(
    mut attribute_updates: ResMut<AttributeUpdates>,
    mut world: ResMut<ElemenetWorld>,
) {
    let mut attrs_storage = world.0.write_storage::<Attrs>();
    for (entity_id, key, new_value) in attribute_updates.0.drain(..) {
        if let Some(attrs) = attrs_storage.get_mut(world.0.entities().entity(entity_id)) {
            attrs.0.insert(key, new_value);
        }
    }
}

// Sistema para procesar eliminaciones
fn process_delete_requests(
    mut delete_requests: ResMut<DeleteRequests>,
    mut world: ResMut<ElemenetWorld>,
    mut entity_map: ResMut<EntityMap>,
    mut commands: Commands,
) {
    for entity_id in delete_requests.0.drain(..) {
        if let Some(bevy_entity) = entity_map.0.remove(&entity_id) {
            commands.entity(bevy_entity).despawn_recursive();
        }
        let entity = world.0.entities().entity(entity_id);
        world.0.delete_entity(entity).expect("Error al eliminar entidad de ElemenetWorld");
    }
}

// Nuevo sistema para marcar entidades sucias
fn mark_dirty_system(
    world: Res<ElemenetWorld>,
    mut commands: Commands,
    entity_map: Res<EntityMap>,
    mut dirty_nodes: ResMut<DirtyNodes>,
) {
    for node_id in dirty_nodes.0.drain(..) {
        if let Some(&entity) = entity_map.0.get(&node_id) {
            commands.entity(entity).insert(Dirty);
        }
    }
}


// Sistema de sincronización optimizado
fn dom_sync_system(
    world: Res<ElemenetWorld>,
    mut commands: Commands,
    dom_data: Res<VirtualDomData>,
    shared_resources: Res<SharedResources>,
    mut entity_map: ResMut<EntityMap>,
    mut dirty_nodes: ResMut<DirtyNodes>,
    mut query: Query<(Entity, &mut Transform, Option<&Dirty>)>,
    asset_server: Res<AssetServer>,
) {
    if dirty_nodes.0.is_empty() {
        return; // Salir temprano si no hay cambios
    }

    let tags = world.0.read_storage::<Tag>();
    let transforms = world.0.read_storage::<Transform2>();
    let hierarchys = world.0.read_storage::<Hierarchy>();
    let models = world.0.read_storage::<Model>();

    for node_id in dirty_nodes.0.drain(..) {
        if let Some(node) = dom_data.nodes.get(&node_id) {
            let tag = tags.get(*node).unwrap().0.clone();
            let hierarchy = hierarchys.get(*node).unwrap();
            let parent_id = hierarchy.parent;

            if let Some(&entity) = entity_map.0.get(&node_id) {
                // Si la entidad ya existe, solo actualiza si está marcada como Dirty
                if let Ok((_, mut transform, dirty)) = query.get_mut(entity) {
                    if dirty.is_some() {
                        if let Some(node_transform) = transforms.get(*node) {
                            apply_transform(node_transform, &mut transform);
                        }
                        commands.entity(entity).remove::<Dirty>();
                    }
                }
            } else {
                // Crear nueva entidad
                let mut transform = Transform::default();
                if let Some(node_transform) = transforms.get(*node) {
                    apply_transform(node_transform, &mut transform);
                }

                let mut entity = commands
                    .spawn((
                        TransformBundle::from_transform(transform),
                        VisibilityBundle::default(),
                        Dirty, // Marcar como Dirty al crearse
                    ))
                    .id();

                match tag.as_str() {
                    "model" => {
                        if let Some(model) = models.get(*node) {
                            apply_model(model, &asset_server, &mut commands, &mut entity);
                        }
                    }
                    "script" | "space2" | "include" => {}
                    _ => {
                        let cube_handle = shared_resources.cube_mesh.clone();
                        let material_handle = shared_resources.default_material.clone();
                        let child_entity = commands
                            .spawn(PbrBundle {
                                mesh: cube_handle,
                                material: material_handle,
                                transform: Transform::from_scale(Vec3::splat(0.2)),
                                ..Default::default()
                            })
                            .id();
                        commands.entity(entity).push_children(&[child_entity]);
                    }
                }

                // Actualizar jerarquía solo si es necesario
                if let Some(parent_id) = parent_id {
                    if let Some(&parent_entity) = entity_map.0.get(&parent_id) {
                        commands.entity(entity).set_parent(parent_entity);
                    }
                } else {
                    commands.entity(entity).remove_parent();
                }

                entity_map.0.insert(node_id, entity);
            }

        }
    }
}

fn update_entity_counter(
    mut counter: ResMut<EntityCounter>,
    query: Query<Entity>,
) {
    counter.count = query.iter().count();
}

fn update_fps_counter(
    time: Res<Time>,
    mut fps_counter: ResMut<FpsCounter>,
) {
    fps_counter.frame_count += 1;
    if fps_counter.timer.tick(time.delta()).just_finished() {
        fps_counter.fps = fps_counter.frame_count;
        fps_counter.frame_count = 0;
    }
}

// Obtener la entidad raíz del árbol (sin padre)
fn get_root_entity(world: &SpecWorld) -> Option<SpecEntity> {
    let hierarchies = world.read_storage::<Hierarchy>();
    for (entity, hierarchy) in (&world.entities(), &hierarchies).join() {
        if hierarchy.parent.is_none() {
            return Some(entity);
        }
    }
    None
}