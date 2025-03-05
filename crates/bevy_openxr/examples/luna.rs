use bevy::prelude::*;
use render::{apply_hsml_element, apply_model_element};
use std::{collections::HashMap, rc::Rc, sync::Arc};
use tokio::runtime::Runtime;

mod utils;
use utils::shapes;

mod render;

use virtual_dom::{
    dom::hsml::{hsml::HSMLElement, HSMLEnum, ProxyElement},
    load_xml_from_url,
    parse_xml,
    serialize_xml,
};

/// Recurso que guarda el DOM virtual como un HashMap de nodos.
#[derive(Resource, Default)]
struct VirtualDomData {
    pub nodes: HashMap<usize, HSMLEnum<Entity>>,
}

/// Recurso que guarda los IDs de los nodos que necesitan actualizarse en el siguiente frame,
/// en el orden en el que deben procesarse (padre antes que hijo).
#[derive(Resource, Default)]
struct DirtyNodes(Vec<usize>);

/// Recurso que mapea el id de un nodo a su entidad en la escena.
#[derive(Resource, Default)]
struct EntityMap(HashMap<usize, Entity>);

/// Recurso para el modo debug que fuerza la recarga del XML cada segundo.
#[derive(Resource)]
struct DebugTimer(Timer);

/// Componente que identifica a la entidad que representa un nodo del DOM, mediante su id.
#[derive(Component)]
struct DomEntity {
    pub id: usize,
}
/// Componente que identifica a la entidad que representa un nodo del DOM, mediante su id.
#[derive(Component)]
struct SystemEntity {
    pub id: usize,
}

/// Función auxiliar para cargar el XML, parsearlo y aplanar el DOM.
/// Retorna un tuple con:
/// - Un HashMap con los nodos, y
/// - Un Vec con los IDs de los nodos en el orden en que se deben procesar (padre antes que hijo).
fn load_and_flatten_xml(url: &str) -> (HashMap<usize, HSMLEnum<Entity>>, Vec<usize>) {
    let rt = Runtime::new().expect("No se pudo crear el runtime de Tokio");
    let xml_content = rt
        .block_on(load_xml_from_url(url))
        .expect("Error al cargar XML");
    println!("XML cargado: {:?}", xml_content);
    
    let root_node: HSMLEnum<Entity> = parse_xml(&xml_content)
        .expect("Error al parsear el XML");
    
    let demo_hsml = serialize_xml(&root_node);
    println!("PREVIEW: {:?}", demo_hsml);
    
    let mut map = HashMap::new();
    let mut dirty = Vec::new();
    
    fn flatten_dom(
        node: HSMLEnum<Entity>,
        map: &mut HashMap<usize, HSMLEnum<Entity>>,
        dirty: &mut Vec<usize>,
    ) {
        dirty.push(node.id());
        map.insert(node.id(), node.clone());
        if let Some(element) = node.get_element() {
            for child in &element.children {
                flatten_dom(child.clone(), map, dirty);
            }
        }
    }
    
    flatten_dom(root_node, &mut map, &mut dirty);
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
        .insert_resource(DebugTimer(Timer::from_seconds(10.0, TimerMode::Repeating)))
        // Añadimos el recurso del contador de FPS
        // .insert_resource(FpsCounter {
        //     timer: Timer::from_seconds(1.0, TimerMode::Once),
        //     frame_count: 0,
        //     fps: 0.0,
        // })
        .add_systems(Startup, setup)
        .add_systems(Update, reload_xml_system)
        .add_systems(Update, dom_sync_system)
        // Añadimos el sistema para el contador de FPS
        // .add_systems(Update, fps_counter_system)
        .run();
}

/// Sistema de setup:
/// - Configura cámara y luz.
/// - Carga el XML inicial y actualiza los recursos VirtualDomData y DirtyNodes.
fn setup(
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
            transform: Transform::from_xyz(-2.0, 8.0, 0.0), // Posición en espacio 3D
            ..default()
        },
        // Marcamos esta entidad como UI para identificarla después
        SystemEntity{id: 0 }
    ));

    let (nodes, dirty) = load_and_flatten_xml("http://localhost:2052/static/main.hsml");
    dom_data.nodes = nodes;
    dirty_nodes.0 = dirty;
}

/// Sistema que recarga el XML cada segundo, actualiza el DOM virtual y elimina las entidades obsoletas.
fn reload_xml_system(
    time: Res<Time>,
    mut debug_timer: ResMut<DebugTimer>,
    mut dom_data: ResMut<VirtualDomData>,
    mut dirty_nodes: ResMut<DirtyNodes>,
    mut commands: Commands,
    mut entity_map: ResMut<EntityMap>,
) {
    debug_timer.0.tick(time.delta());
    if debug_timer.0.finished() {
        let (new_nodes, new_dirty) = load_and_flatten_xml("http://localhost:2052/static/main.hsml");

        // Eliminar las entidades cuyos nodos ya no existen en el nuevo XML.
        let old_ids: Vec<usize> = entity_map.0.keys().cloned().collect();
        for id in old_ids {
            if !new_nodes.contains_key(&id) {
                if let Some(entity) = entity_map.0.remove(&id) {
                    commands.entity(entity).despawn_recursive();
                }
            }
        }
        
        // Actualizar el recurso con los nuevos nodos y el nuevo orden de dirty.
        dom_data.nodes = new_nodes;
        dirty_nodes.0 = new_dirty;
    }
}

/// Sistema que sincroniza el DOM virtual con las entidades en la escena.
/// Se procesan los nodos en el orden definido en el array para asegurar que
/// los padres se creen antes que los hijos.
fn dom_sync_system(
    mut commands: Commands,
    mut dom_data: ResMut<VirtualDomData>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut entity_map: ResMut<EntityMap>,
    mut dirty_nodes: ResMut<DirtyNodes>,
    mut query: Query<&mut Transform>,
    asset_server: Res<AssetServer>,
) {
    // Se recorre el array en el orden definido (padre -> hijo).
    for node_id in dirty_nodes.0.iter() {
        if let Some(node) = dom_data.nodes.get_mut(node_id) {
            // Si la entidad ya existe, se actualiza su transformación.
            if let Some(&entity) = entity_map.0.get(node_id) {
                if let Some(hsml) = node.get_hsml_element() {

                    if let Ok(mut transform) = query.get_mut(entity) {
                        apply_hsml_element(&hsml, &mut transform );
                    }
                }
            } else {
                let tag_id = String::from(node.tag());

                let node_id = node.id().clone();
                let parent_id = node.parent().clone();
                let mut node_clone = node.clone();
                if let Some(hsml) = node_clone.get_hsml_element_mut() {
                    // Crear la entidad para el nodo.
                    let cube_handle = meshes.add(shapes::create_cube());
                    let material_handle = materials.add(StandardMaterial {
                        base_color: Color::rgb(0.5, 0.8, 0.8),
                        ..Default::default()
                    });
                    let mut transform = Transform::default();
                    
                    apply_hsml_element(&hsml, &mut transform );

                    let mut entity = commands.spawn_empty()
                    .insert(TransformBundle::from_transform(transform))
                    .insert(VisibilityBundle::default()).id();

                    let mut node_clone = node.clone();
                    match &mut node_clone {
                        HSMLEnum::MODELElement(e)=> {
                                                apply_model_element(e, &asset_server, &mut commands, &mut entity);
                                            }
                        _=> {
                            let entity_emply = commands
                            .spawn((
                                PbrBundle {
                                    mesh: cube_handle,
                                    material: material_handle,
                                    transform: transform,
                                    ..Default::default()
                                },
                                DomEntity { id: node_id },
                            ))
                            .id();
                            commands.entity(entity).push_children(&[entity_emply]);
                        }
                    }

                    // Si el nodo tiene un padre, se añade como hijo de la entidad padre.
                    if let Some(parent_id) = parent_id {
                        if let Some(&parent_entity) = entity_map.0.get(&parent_id) {
                            commands.entity(parent_entity).push_children(&[entity]);
                        }
                    }

                    entity_map.0.insert(node_id, entity);
                    hsml.native = Some(entity);

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
    mut query: Query<&mut Text>,
) {
    fps_counter.frame_count += 1;
    fps_counter.timer.tick(time.delta());

    if fps_counter.timer.finished() {
        // Calcular FPS
        fps_counter.fps = fps_counter.frame_count as f32 / fps_counter.timer.duration().as_secs_f32();
        
        // Actualizar el texto
        for mut text in query.iter_mut() {
            text.sections[0].value = format!("FPS: {:.1}", fps_counter.fps);
        }

        // Reiniciar el contador
        fps_counter.frame_count = 0;
        fps_counter.timer.reset();
    }
}

// /// Sistema que simula cambios en el DOM virtual:
// /// Se actualiza la rotación de todos los nodos y se marca cada uno como "dirty".
// fn fake_update_dom_system(
//     time: Res<Time>,
//     mut state: Local<FakeUpdateState>,
//     mut dom_data: ResMut<VirtualDomData>,
//     mut next_id: ResMut<NextNodeId>,
//     mut dirty_nodes: ResMut<DirtyNodes>,
// ) {
//     // Actualiza la rotación de cada nodo y márcalo como "dirty".
//     for (_key, node) in dom_data.nodes.iter_mut() {
//         node.ry += 2.0 * time.delta_seconds();
//         dirty_nodes.0.insert(node.id, true);
//     }
    
//     if state.timer.tick(time.delta()).just_finished() {
//         if state.add_mode {
//             // Añadir un nodo raíz (en un caso real podrías definir la jerarquía según tus necesidades).
//             let new_id = next_id.0;
//             next_id.0 += 1;
//             let new_node = DomNode {
//                 id: new_id,
//                 parent: None,
//                 x: new_id as f32 * 1.5,
//                 y: 0.0,
//                 z: 0.0,
//                 rx: 0.0,
//                 ry: 0.0,
//                 rz: 0.0,
//             };
//             dom_data.nodes.insert(new_id, new_node);
//             dirty_nodes.0.insert(new_id, true);
//             info!("Se agregó el nodo raíz con id: {}", new_id);
//         } else {
//             // Quitar el nodo con el id más alto (último agregado).
//             if let Some(&max_id) = dom_data.nodes.keys().max() {
//                 if let Some(removed) = dom_data.nodes.remove(&max_id) {
//                     info!("Se eliminó el nodo con id: {}", removed.id);
//                     dirty_nodes.0.insert(removed.id, true);
//                 }
//             }
//         }
//         // Alternar entre añadir y quitar.
//         state.add_mode = !state.add_mode;
//     }
// }