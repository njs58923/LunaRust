use bevy::prelude::*;
use std::collections::HashMap;
use futures::executor::block_on;
use tokio::runtime::Runtime;

mod utils;
use utils::shapes;

use virtual_dom::{load_xml_from_url, parse_xml, VirtualNode};

/// Recurso que guarda el DOM virtual como un HashMap de nodos.
#[derive(Resource, Default)]
struct VirtualDomData {
    pub nodes: HashMap<usize, VirtualNode<Entity>>,
}

/// Recurso que guarda los IDs de los nodos que necesitan actualizarse en el siguiente frame.
#[derive(Resource, Default)]
struct DirtyNodes(HashMap<usize, bool>);

/// Recurso que mapea el id de un nodo a su entidad en la escena.
#[derive(Resource, Default)]
struct EntityMap(HashMap<usize, Entity>);

/// Componente que identifica a la entidad que representa un nodo del DOM, mediante su id.
#[derive(Component)]
struct DomEntity {
    pub id: usize,
}

fn main() {
    App::new()
        .add_plugins(DefaultPlugins)
        .insert_resource(VirtualDomData::default())
        .insert_resource(DirtyNodes::default())
        .insert_resource(EntityMap::default())
        .add_systems(Startup, setup)
        .add_systems(Update, dom_sync_system)
        .run();
}

/// Sistema de setup:
/// - Configura cámara y luz.
/// - Carga y parsea el XML, aplanando el árbol en el recurso VirtualDomData.
fn setup(
    mut commands: Commands, 
    mut dom_data: ResMut<VirtualDomData>,
    mut dirty_nodes: ResMut<DirtyNodes>,
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

    let rt = Runtime::new().expect("No se pudo crear el runtime de Tokio");
    // Cargar el XML usando block_on para esperar la operación asíncrona.
    let xml_content = rt.block_on(load_xml_from_url("http://localhost:2052/static/main.hsml"))
        .expect("Error al cargar XML");
    
    // Parsea el XML. Se usa Entity como tipo para Native.
    let root_node: VirtualNode<Entity> = parse_xml(&xml_content)
        .expect("Error al parsear el XML");
    
    // Función auxiliar para insertar recursivamente cada nodo en el HashMap.
    fn flatten_dom(
        node: VirtualNode<Entity>,
        map: &mut HashMap<usize, VirtualNode<Entity>>,
        dirty: &mut HashMap<usize, bool>,
    ) {
        dirty.insert(node.id, true);
        map.insert(node.id, node.clone());
        for child in node.children {
            flatten_dom(child, map, dirty);
        }
    }
    
    flatten_dom(root_node, &mut dom_data.nodes, &mut dirty_nodes.0);
}

/// Sistema que sincroniza el DOM virtual con las entidades en la escena.
/// Si el nodo ya tiene entidad, actualiza su transformación; si no, la crea y asigna su referencia a `native`.
fn dom_sync_system(
    mut commands: Commands,
    mut dom_data: ResMut<VirtualDomData>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut entity_map: ResMut<EntityMap>,
    mut dirty_nodes: ResMut<DirtyNodes>,
    mut query: Query<&mut Transform>,
) {
    for (&node_id, &dirty) in dirty_nodes.0.iter() {
        if !dirty {
            continue;
        }
        if let Some(node) = dom_data.nodes.get_mut(&node_id) {
            // Si la entidad ya existe, se actualiza el transform.
            if let Some(&entity) = entity_map.0.get(&node_id) {
                if let Ok(mut transform) = query.get_mut(entity) {
                    transform.translation = Vec3::new(node.x, node.y, node.z);
                    transform.rotation =
                        Quat::from_euler(EulerRot::XYZ, node.rx, node.ry, node.rz);
                }
            } else {
                println!("AÑADIDO!! {:?}", node.tag);
                // Crear la entidad para el nodo.
                let cube_handle = meshes.add(shapes::create_cube());
                let material_handle = materials.add(StandardMaterial {
                    base_color: Color::rgb(0.5, 0.8, 0.8),
                    ..Default::default()
                });
                let entity = commands
                    .spawn((
                        PbrBundle {
                            mesh: cube_handle,
                            material: material_handle,
                            transform: Transform::from_xyz(node.x, node.y, node.z)
                                .with_rotation(Quat::from_euler(
                                    EulerRot::XYZ,
                                    node.rx,
                                    node.ry,
                                    node.rz,
                                )),
                            ..Default::default()
                        },
                        DomEntity { id: node.id },
                    ))
                    .id();
                // Si el nodo tiene un padre, se añade como hijo de la entidad padre.
                if let Some(parent_id) = node.parent {
                    if let Some(&parent_entity) = entity_map.0.get(&parent_id) {
                        commands.entity(parent_entity).push_children(&[entity]);
                    }
                }
                entity_map.0.insert(node_id, entity);
                // Almacenamos la referencia a la entidad en el campo `native`.
                node.native = Some(entity);
            }
        } else {
            // Si el nodo fue eliminado, se despawnea la entidad y se elimina del mapeo.
            if let Some(&entity) = entity_map.0.get(&node_id) {
                commands.entity(entity).despawn_recursive();
                entity_map.0.remove(&node_id);
            }
        }
    }
    dirty_nodes.0.clear();
}

/// Sistema que simula cambios en el DOM virtual:
/// Se actualiza la rotación de todos los nodos y se marca cada uno como "dirty".
/// Cada 5 segundos se añade o se elimina un nodo (en este ejemplo, sin alterar la jerarquía).
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