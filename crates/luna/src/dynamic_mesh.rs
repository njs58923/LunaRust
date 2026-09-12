//! Isolate-owned generated resources mounted through the common model lifecycle.
use crate::{
    models::{ModelInstance, ModelResource},
    DirtyNodes, TransformOnlyDirtyNodes,
};
use bevy::{
    prelude::*,
    render::{
        mesh::Indices, primitives::Aabb, render_asset::RenderAssetUsages,
        render_resource::PrimitiveTopology,
    },
};
use js_runtime::mesh::{MeshCommand, MeshData};
use std::collections::{HashMap, HashSet};

struct Entry {
    owner: u32,
    scope: u64,
    worker: Option<std::thread::ThreadId>,
    model: ModelResource,
    mesh: Handle<Mesh>,
}
#[derive(Resource, Default)]
pub struct DynamicMeshes {
    entries: HashMap<String, Entry>,
    material: Option<Handle<StandardMaterial>>,
    context_generation: Option<u64>,
    scopes: HashMap<u32, u64>,
}
impl DynamicMeshes {
    pub fn get(&self, src: &str, owner: Option<u32>) -> Option<&ModelResource> {
        self.entries
            .get(src)
            .filter(|entry| Some(entry.owner) == owner)
            .map(|entry| &entry.model)
    }
}

fn build_mesh(mut data: MeshData) -> Mesh {
    if data.normals.is_empty() {
        let mut normals = vec![Vec3::ZERO; data.positions.len()];
        for triangle in data.indices.chunks_exact(3) {
            let [a, b, c] = [
                triangle[0] as usize,
                triangle[1] as usize,
                triangle[2] as usize,
            ];
            let normal = (Vec3::from(data.positions[b]) - Vec3::from(data.positions[a]))
                .cross(Vec3::from(data.positions[c]) - Vec3::from(data.positions[a]));
            for i in [a, b, c] {
                normals[i] += normal;
            }
        }
        data.normals = normals
            .into_iter()
            .map(|n| n.normalize_or_zero().to_array())
            .collect();
    }
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, data.positions);
    mesh.insert_indices(Indices::U32(data.indices));
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, data.normals);
    if !data.uvs.is_empty() {
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, data.uvs);
    }
    if !data.colors.is_empty() {
        mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, data.colors);
    }
    mesh
}

fn invalidate(world: &mut World, sources: &HashSet<String>) {
    if sources.is_empty() {
        return;
    }
    let ids: Vec<_> = world
        .query::<&ModelInstance>()
        .iter(world)
        .filter(|m| sources.contains(&m.source))
        .map(|m| m.node_id)
        .collect();
    if let Some(mut dirty) = world.get_resource_mut::<DirtyNodes>() {
        dirty.0.extend(ids.iter().copied());
    }
    if let Some(mut fast) = world.get_resource_mut::<TransformOnlyDirtyNodes>() {
        for id in ids {
            fast.0.remove(&id);
        }
    }
}

pub fn apply_commands(world: &mut World, owner: u32, scope: u64, commands: Vec<MeshCommand>) {
    if commands.is_empty() {
        // Most isolate ticks have no mesh work. Still process a changed scope:
        // a restarted isolate must release the previous generation's resources.
        match world.get_resource::<DynamicMeshes>() {
            None => return,
            Some(registry) if registry.scopes.get(&owner) == Some(&scope) => return,
            _ => {}
        }
    }
    world.init_resource::<DynamicMeshes>();
    let worker = world.get_non_send_resource::<crate::js::ScriptRuntimeManager>()
        .and_then(|m| m.contexts.get(&owner)).and_then(|w| w.join.as_ref()).map(|j| j.thread().id());
    let mut changed = HashSet::new();
    let mut bounds = HashMap::new();
    world.resource_scope(|world, mut registry: Mut<DynamicMeshes>| {
        // A restarted isolate cannot inherit resources from its predecessor.
        if registry.scopes.insert(owner, scope) != Some(scope) {
            registry.entries.retain(|src, entry| {
                let keep = entry.owner != owner || entry.scope == scope;
                if !keep {
                    changed.insert(src.clone());
                }
                keep
            });
        }
        for command in commands {
            match command {
                MeshCommand::Dispose(src) => {
                    if registry
                        .entries
                        .get(&src)
                        .is_some_and(|e| e.owner == owner && e.scope == scope)
                    {
                        registry.entries.remove(&src);
                        changed.insert(src);
                    }
                }
                MeshCommand::Upload(src, data) => {
                    let mesh = build_mesh(data);
                    if let Some(entry) = registry.entries.get(&src) {
                        if entry.owner != owner || entry.scope != scope {
                            continue;
                        }
                        if let Some(aabb) = mesh.compute_aabb() {
                            bounds.insert(entry.mesh.id(), aabb);
                        }
                        world
                            .resource_mut::<Assets<Mesh>>()
                            .insert(entry.mesh.id(), mesh);
                        // Scene and instance handles stay unchanged on geometry updates.
                    } else {
                        let handle = world.resource_mut::<Assets<Mesh>>().add(mesh);
                        let material = registry
                            .material
                            .get_or_insert_with(|| {
                                world.resource_mut::<Assets<StandardMaterial>>().add(
                                    StandardMaterial {
                                        perceptual_roughness: 0.8,
                                        ..default()
                                    },
                                )
                            })
                            .clone();
                        let mut scene = World::new();
                        scene.spawn(PbrBundle {
                            mesh: handle.clone(),
                            material,
                            ..default()
                        });
                        let scene = world.resource_mut::<Assets<Scene>>().add(Scene::new(scene));
                        registry.entries.insert(
                            src.clone(),
                            Entry {
                                owner,
                                scope,
                                worker,
                                mesh: handle,
                                model: ModelResource {
                                    asset_path: src.clone(),
                                    scene,
                                    gltf: None,
                                },
                            },
                        );
                        changed.insert(src);
                    }
                }
            }
        }
    });
    if !bounds.is_empty() {
        for (mesh, mut aabb) in world.query::<(&Handle<Mesh>, &mut Aabb)>().iter_mut(world) {
            if let Some(new) = bounds.get(&mesh.id()) {
                *aabb = *new;
            }
        }
    }
    invalidate(world, &changed);
}

/// Only inspect resource ownership when the set of live workers changes.
pub fn cleanup_contexts(world: &mut World) {
    let Some(registry) = world.get_resource::<DynamicMeshes>() else {
        return;
    };
    let Some(manager) = world.get_non_send_resource::<crate::js::ScriptRuntimeManager>() else {
        return;
    };
    if registry.context_generation == Some(manager.context_generation) {
        return;
    }
    let generation = manager.context_generation;
    let live: HashMap<_, _> = manager.contexts.iter().map(|(id, w)| (*id, w.join.as_ref().map(|j| j.thread().id()))).collect();
    let mut removed = HashSet::new();
    let mut registry = world.resource_mut::<DynamicMeshes>();
    registry.context_generation = Some(generation);
    registry.scopes.retain(|owner, _| live.contains_key(owner));
    registry.entries.retain(|src, e| {
        if live.get(&e.owner) == Some(&e.worker) {
            true
        } else {
            removed.insert(src.clone());
            false
        }
    });
    invalidate(world, &removed);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn empty_mesh_ticks_preserve_change_detection_but_process_scope_changes() {
        let mut world = World::new();
        apply_commands(&mut world, 1, 10, Vec::new());
        assert!(!world.contains_resource::<DynamicMeshes>());
        world.init_resource::<DynamicMeshes>();
        apply_commands(&mut world, 1, 10, Vec::new());
        world.clear_trackers();
        apply_commands(&mut world, 1, 10, Vec::new());
        assert!(!world.get_resource_ref::<DynamicMeshes>().unwrap().is_changed());
        apply_commands(&mut world, 1, 11, Vec::new());
        assert_eq!(world.resource::<DynamicMeshes>().scopes.get(&1), Some(&11));
        assert!(world.get_resource_ref::<DynamicMeshes>().unwrap().is_changed());
    }
}
