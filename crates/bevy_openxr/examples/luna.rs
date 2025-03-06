use bevy::prelude::*;
use render::{apply_model, apply_transform};
use std::collections::HashMap;
use tokio::runtime::Runtime;
mod utils;
use utils::shapes;
use specs::{Entity as SpecEntity, ReadStorage, World as SpecWorld, WorldExt};
mod render;

use virtual_dom::{
    dom::{element::{build_world, Hierarchy, Tag, Transform2}, hsml::Model}, load_xml_from_url, parse_xml, serialize_xml,
};

/// Recurso que guarda el DOM virtual como un HashMap de nodos.
#[derive(Resource, Default)]
struct VirtualDomData {
    pub nodes: HashMap<u32, SpecEntity>,
}

/// Recurso que guarda los IDs de los nodos que necesitan actualizarse en el siguiente frame,
/// en el orden en el que deben procesarse (padre antes que hijo).
#[derive(Resource, Default)]
struct DirtyNodes(Vec<u32>);
/// Recurso que guarda los IDs de los nodos que necesitan actualizarse en el siguiente frame,
/// en el orden en el que deben procesarse (padre antes que hijo).
#[derive(Resource, Default)]
struct ElemenetWorld(SpecWorld);

/// Recurso que mapea el id de un nodo a su entidad en la escena.
#[derive(Resource, Default)]
struct EntityMap(HashMap<u32, Entity>);

/// Recurso para el modo debug que fuerza la recarga del XML cada segundo.
#[derive(Resource)]
struct DebugTimer(Timer);

/// Componente que identifica a la entidad que representa un nodo del DOM, mediante su id.
#[derive(Component)]
struct DomEntity {
    pub id: u32,
}
/// Componente que identifica a la entidad que representa un nodo del DOM, mediante su id.
#[derive(Component)]
struct SystemEntity {
    pub id: u32,
}

/// Función auxiliar para cargar el XML, parsearlo y aplanar el DOM.
/// Retorna un tuple con:
/// - Un HashMap con los nodos, y
/// - Un Vec con los IDs de los nodos en el orden en que se deben procesar (padre antes que hijo).
fn load_and_flatten_xml(mut world: &mut SpecWorld,url: &str) -> (HashMap<u32, SpecEntity>, Vec<u32>) {
    let rt = Runtime::new().expect("No se pudo crear el runtime de Tokio");
    let xml_content = rt
        .block_on(load_xml_from_url(url))
        .expect("Error al cargar XML");
    println!("XML cargado: {:?}", xml_content);

    
    let root_node: SpecEntity = parse_xml(&mut world, &xml_content)
        .expect("Error al parsear el XML");
    
    let demo_hsml = serialize_xml(&mut world, root_node.clone());
    println!("PREVIEW: {:?}", demo_hsml);
    
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

/// Nuevo recurso para el contador de FPS
#[derive(Resource)]
struct FpsCounter {
    timer: Timer,
    frame_count: u32,
    fps: f32,
}


fn main() {
    App::new()
        .add_plugins(DefaultPlugins)
        .insert_resource(VirtualDomData::default())
        .insert_resource(DirtyNodes::default())
        .insert_resource(EntityMap::default())
        .insert_resource(ElemenetWorld(build_world()))
        .insert_resource(DebugTimer(Timer::from_seconds(1.0, TimerMode::Repeating)))
        // Añadimos el recurso del contador de FPS
        .insert_resource(FpsCounter {
            timer: Timer::from_seconds(1.0, TimerMode::Once),
            frame_count: 0,
            fps: 0.0,
        })
        .add_systems(Startup, setup)
        .add_systems(Update, reload_xml_system)
        .add_systems(Update, dom_sync_system)
        // Añadimos el sistema para el contador de FPS
        .add_systems(Update, fps_counter_system)
        .run();
}

/// Sistema de setup:
/// - Configura cámara y luz.
/// - Carga el XML inicial y actualiza los recursos VirtualDomData y DirtyNodes.
fn setup(
    mut world: ResMut<ElemenetWorld>, 
    mut commands: Commands, 
    mut dom_data: ResMut<VirtualDomData>,
    mut dirty_nodes: ResMut<DirtyNodes>,
    asset_server: Res<AssetServer>,
) {
    // Cámara 3D.
    commands.spawn(Camera3dBundle {
        transform: Transform::from_xyz(0.0, 3.0, 8.0).looking_at(Vec3::ZERO, Vec3::Y),
        ..default()
    });

    // Luz.
    commands.spawn(PointLightBundle {
        transform: Transform::from_xyz(3.0, 8.0, 3.0),
        ..default()
    });

    // Texto FPS como un elemento 2D en el espacio 3D
    commands.spawn((
        TextBundle {
            text: Text::from_section(
                "FPS: 0",
                TextStyle {
                    font: asset_server.load("fonts/FiraSans-Bold.ttf"),
                    font_size: 0.2, // Tamaño ajustado para espacio 3D
                    color: Color::WHITE,
                },
            ),
            transform: Transform::from_xyz(-2.0, 5.0, 0.0), // Posición en espacio 3D
            ..default()
        },
        // Marcamos esta entidad como UI para identificarla después
        SystemEntity{id: 0 }
    ));

    let (nodes, dirty) = load_and_flatten_xml(&mut world.0,"http://localhost:2052/static/main.hsml");
    dom_data.nodes = nodes;
    dirty_nodes.0 = dirty;
}

/// Sistema que recarga el XML cada segundo, actualiza el DOM virtual y elimina las entidades obsoletas.
fn reload_xml_system(
    time: Res<Time>,
    mut world: ResMut<ElemenetWorld>, 
    mut debug_timer: ResMut<DebugTimer>,
    mut dom_data: ResMut<VirtualDomData>,
    mut dirty_nodes: ResMut<DirtyNodes>,
    mut commands: Commands,
    mut entity_map: ResMut<EntityMap>,
) {
    debug_timer.0.tick(time.delta());

    if debug_timer.0.finished() {
        let (new_nodes, new_dirty) = load_and_flatten_xml(&mut world.0, "http://localhost:2052/static/main.hsml");

        // Eliminar únicamente los nodos que realmente ya no existen.
        entity_map.0.retain(|&id, &mut entity| {
            if !new_nodes.contains_key(&id) {
                commands.entity(entity).despawn_recursive();
                false
            } else {
                true
            }
        });

        // Solo insertar en dirty_nodes los nodos realmente nuevos o que cambiaron.
        let mut refined_dirty = Vec::new();
        for node_id in new_dirty.iter() {
            match (dom_data.nodes.get(node_id), new_nodes.get(node_id)) {
                (Some(old_node), Some(new_node)) => {
                    if old_node != new_node {
                        refined_dirty.push(*node_id);
                    }
                }
                (None, Some(_)) => refined_dirty.push(*node_id), // Nodo nuevo
                _ => {},
            }
        }

        // Actualizar los datos del DOM virtual con los nuevos valores.
        dom_data.nodes = new_nodes;
        dirty_nodes.0 = refined_dirty;
    }
}


/// Sistema que sincroniza el DOM virtual con las entidades en la escena.
/// Se procesan los nodos en el orden definido en el array para asegurar que
/// los padres se creen antes que los hijos.
fn dom_sync_system(
    worls: ResMut<ElemenetWorld>, 
    mut commands: Commands,
    mut dom_data: ResMut<VirtualDomData>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut entity_map: ResMut<EntityMap>,
    mut dirty_nodes: ResMut<DirtyNodes>,
    mut query: Query<&mut Transform>,
    asset_server: Res<AssetServer>,
) {

    let tags = worls.0.read_storage::<Tag>();
    let transforms = worls.0.read_storage::<Transform2>();
    let hierarchys = worls.0.read_storage::<Hierarchy>();
    let models = worls.0.read_storage::<Model>();


    // Se recorre el array en el orden definido (padre -> hijo).
    for node_id in dirty_nodes.0.iter() {
        if let Some(node) = dom_data.nodes.get_mut(node_id) {
            let tag = &tags.get(node.clone()).unwrap().0;
            
            // Si la entidad ya existe, se actualiza su transformación.
            if let Some(&entity) = entity_map.0.get(node_id) {
                if let Some(node_transform) = transforms.get(*node) {
                    if let Ok(mut transform) = query.get_mut(entity) {
                        apply_transform(&node_transform, &mut transform );
                    }
                }
            } else {
                let node_id = node.id().clone();
                let hierarchy = hierarchys.get(*node).unwrap();
                let parent_id = hierarchy.parent;
                if let Some(node_transform) = transforms.get(*node) {
                    // Crear la entidad para el nodo.
                    let cube_handle = meshes.add(shapes::create_cube());
                    let material_handle = materials.add(StandardMaterial {
                        base_color: Color::rgb(0.5, 0.8, 0.8),
                        ..Default::default()
                    });
                    let mut transform = Transform::default();
                    
                    apply_transform(&node_transform, &mut transform );

                    let mut entity = commands.spawn_empty()
                    .insert(TransformBundle::from_transform(transform))
                    .insert(VisibilityBundle::default()).id();

                    if tag == "model"{
                        if let Some(model) = models.get(*node){
                            apply_model(&model, &asset_server, &mut commands, &mut entity);
                        }
                    }else if tag == "script"{
                    }else if tag == "space"{
                    }else{
                        let entity_emply = commands
                        .spawn((
                            PbrBundle {
                                mesh: cube_handle,
                                material: material_handle,
                                transform: transform.with_scale(Vec3 { x: 0.2, y: 0.2, z: 0.2 }),
                                ..Default::default()
                            },
                            DomEntity { id: node_id },
                        ))
                        .id();
                        commands.entity(entity).push_children(&[entity_emply]);
                    }

                    // Si el nodo tiene un padre, se añade como hijo de la entidad padre.
                    if let Some(parent_id) = parent_id {
                        if let Some(&parent_entity) = entity_map.0.get(&parent_id) {
                            commands.entity(parent_entity).push_children(&[entity]);
                        }
                    }

                    entity_map.0.insert(node_id, entity);

                }

            }
        }
        // No se intenta despawnear aquí, ya que reload_xml_system se encarga de eliminar los nodos obsoletos.
    }
    dirty_nodes.0.clear();
}

/// Nuevo sistema para actualizar el contador de FPS
fn fps_counter_system(
    time: Res<Time>,
    mut fps_counter: ResMut<FpsCounter>,
    mut query: Query<&mut Text, With<SystemEntity>>,
) {
    fps_counter.frame_count += 1;
    fps_counter.timer.tick(time.delta());

    if fps_counter.timer.finished() {
        fps_counter.fps = fps_counter.frame_count as f32 / fps_counter.timer.elapsed_secs();

        for mut text in query.iter_mut() {
            text.sections[0].value = format!("FPS: {:.1}", fps_counter.fps);
        }

        fps_counter.frame_count = 0;
        fps_counter.timer.reset();
    }
}

