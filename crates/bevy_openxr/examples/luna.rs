use bevy::prelude::*;
use std::collections::HashMap;

mod utils;
use utils::shapes;

use virtual_dom::load_xml_from_url;

/// Datos de un nodo en el DOM virtual, ahora con un campo `parent` opcional.
/// Si `parent` es `None`, el nodo es raíz; si es `Some(id)`, es hijo del nodo con ese id.
#[derive(Debug, Default)]
struct DomNode {
    pub id: usize,
    pub parent: Option<usize>,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub rx: f32,
    pub ry: f32,
    pub rz: f32,
}

/// Recurso que guarda el DOM virtual como un HashMap de nodos.
#[derive(Resource, Default)]
struct VirtualDomData {
    pub nodes: HashMap<usize, DomNode>,
}

/// Recurso para generar un nuevo id para cada nodo.
#[derive(Resource, Default)]
struct NextNodeId(usize);

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

/// Estado local para el sistema fake_update_dom_system.
struct FakeUpdateState {
    timer: Timer,
    add_mode: bool,
}

impl Default for FakeUpdateState {
    fn default() -> Self {
        Self {
            timer: Timer::from_seconds(5.0, TimerMode::Repeating),
            add_mode: true,
        }
    }
}

fn main() {
    App::new()
        .add_plugins(DefaultPlugins)
        .insert_resource(VirtualDomData::default())
        .insert_resource(NextNodeId::default())
        .insert_resource(DirtyNodes::default())
        .insert_resource(EntityMap::default())
        .add_systems(Startup, setup)
        // Se asegura que fake_update_dom_system se ejecute antes que dom_sync_system
        .add_systems(Update, (fake_update_dom_system, dom_sync_system))
        .run();
}

/// En este ejemplo se crean dos nodos:
/// - Un nodo raíz (sin padre).
/// - Un nodo hijo (con `parent: Some(id_del_padre)`).
fn setup(
    mut commands: Commands, 
    mut dom_data: ResMut<VirtualDomData>,
    mut next_id: ResMut<NextNodeId>,
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

    // Nodo raíz.
    let id0 = next_id.0;
    next_id.0 += 1;
    dom_data.nodes.insert(id0, DomNode {
        id: id0,
        parent: None, // Nodo raíz
        x: 0.0,
        y: 0.0,
        z: 0.0,
        rx: 0.0,
        ry: 0.0,
        rz: 0.0,
    });

    // Nodo hijo del nodo raíz.
    let id1 = next_id.0;
    next_id.0 += 1;
    dom_data.nodes.insert(id1, DomNode {
        id: id1,
        parent: Some(id0), // Este nodo es hijo del nodo con id `id0`
        x: 2.0,
        y: 0.0,
        z: 0.0,
        rx: 0.0,
        ry: 0.5,
        rz: 0.0,
    });
}

/// Sistema que simula cambios en el DOM virtual:
/// Se actualiza la rotación de todos los nodos y se marca cada uno como "dirty".
/// Cada 5 segundos se añade o se elimina un nodo (en este ejemplo, sin alterar la jerarquía).
fn fake_update_dom_system(
    time: Res<Time>,
    mut state: Local<FakeUpdateState>,
    mut dom_data: ResMut<VirtualDomData>,
    mut next_id: ResMut<NextNodeId>,
    mut dirty_nodes: ResMut<DirtyNodes>,
) {
    // Actualiza la rotación de cada nodo y márcalo como "dirty".
    for (_key, node) in dom_data.nodes.iter_mut() {
        node.ry += 2.0 * time.delta_seconds();
        dirty_nodes.0.insert(node.id, true);
    }
    
    if state.timer.tick(time.delta()).just_finished() {
        if state.add_mode {
            // Añadir un nodo raíz (en un caso real podrías definir la jerarquía según tus necesidades).
            let new_id = next_id.0;
            next_id.0 += 1;
            let new_node = DomNode {
                id: new_id,
                parent: None,
                x: new_id as f32 * 1.5,
                y: 0.0,
                z: 0.0,
                rx: 0.0,
                ry: 0.0,
                rz: 0.0,
            };
            dom_data.nodes.insert(new_id, new_node);
            dirty_nodes.0.insert(new_id, true);
            info!("Se agregó el nodo raíz con id: {}", new_id);
        } else {
            // Quitar el nodo con el id más alto (último agregado).
            if let Some(&max_id) = dom_data.nodes.keys().max() {
                if let Some(removed) = dom_data.nodes.remove(&max_id) {
                    info!("Se eliminó el nodo con id: {}", removed.id);
                    dirty_nodes.0.insert(removed.id, true);
                }
            }
        }
        // Alternar entre añadir y quitar.
        state.add_mode = !state.add_mode;
    }
}

/// Sistema que sincroniza el DOM virtual con las entidades en la escena.
/// Se crean o actualizan las entidades únicamente para los nodos marcados como "dirty".
/// Además, si un nodo tiene un padre, se establece la relación de jerarquía.
fn dom_sync_system(
    mut commands: Commands,
    dom_data: Res<VirtualDomData>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut entity_map: ResMut<EntityMap>,
    mut dirty_nodes: ResMut<DirtyNodes>,
    mut query: Query<&mut Transform>,
) {
    // Itera solo sobre los nodos que han sido marcados como "dirty".
    for (&node_id, &dirty) in dirty_nodes.0.iter() {
        if !dirty {
            continue;
        }
        if let Some(node) = dom_data.nodes.get(&node_id) {
            // Si la entidad ya existe, se actualiza el transform.
            if let Some(&entity) = entity_map.0.get(&node_id) {
                if let Ok(mut transform) = query.get_mut(entity) {
                    transform.translation = Vec3::new(node.x, node.y, node.z);
                    transform.rotation =
                        Quat::from_euler(EulerRot::XYZ, node.rx, node.ry, node.rz);
                }
            } else {
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
