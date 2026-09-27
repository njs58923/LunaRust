//! Isolate-owned generated resources mounted through the common model lifecycle.
use crate::{
    models::{GeneratedModel, ModelInstance, ModelResource},
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
    fingerprint: blake3::Hash,
}

struct SharedGeometry {
    mesh: Handle<Mesh>,
    aabb: Aabb,
    owners: usize,
}
#[derive(Resource, Default)]
pub struct DynamicMeshes {
    entries: HashMap<String, Entry>,
    material: Option<Handle<StandardMaterial>>,
    context_generation: Option<u64>,
    scopes: HashMap<u32, u64>,
    geometry: HashMap<blake3::Hash, SharedGeometry>,
}
impl DynamicMeshes {
    pub fn get(&self, src: &str, owner: Option<u32>) -> Option<&ModelResource> {
        self.entries
            .get(src)
            .filter(|entry| Some(entry.owner) == owner)
            .map(|entry| &entry.model)
    }

    fn release_geometry(&mut self, fingerprint: blake3::Hash) {
        if let Some(shared) = self.geometry.get_mut(&fingerprint) {
            shared.owners -= 1;
            if shared.owners == 0 {
                self.geometry.remove(&fingerprint);
            }
        }
    }

    fn remove(&mut self, src: &str) -> Option<Entry> {
        let entry = self.entries.remove(src)?;
        self.release_geometry(entry.fingerprint);
        Some(entry)
    }
}

fn build_mesh(data: MeshData) -> Mesh {
    debug_assert_eq!(data.positions.len(), data.normals.len());
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        // Generated geometry has no CPU readers after upload: bounds live in
        // the registry and updates submit complete replacement buffers. Bevy
        // moves this mesh to extraction instead of retaining/cloning its data.
        RenderAssetUsages::RENDER_WORLD,
    );
    let indices = if data.positions.len() <= usize::from(u16::MAX) + 1 {
        Indices::U16(data.indices.into_iter().map(|i| i as u16).collect())
    } else {
        Indices::U32(data.indices)
    };
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, data.positions);
    mesh.insert_indices(indices);
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
    if commands.is_empty()
        && world
            .get_resource::<DynamicMeshes>()
            .is_none_or(|registry| registry.scopes.get(&owner) == Some(&scope))
    {
        return;
    }
    apply_command_batches(world, vec![(owner, scope, commands)]);
}

/// Apply the frame's isolate responses in order, then visit mounted instances
/// once. Streets can finish hundreds of independent uploads in the same frame;
/// scanning all instances after each response makes that mount cost quadratic.
pub fn apply_command_batches(world: &mut World, batches: Vec<(u32, u64, Vec<MeshCommand>)>) {
    let registry = world.get_resource::<DynamicMeshes>();
    let Some(first_work) = batches.iter().position(|(owner, scope, commands)| {
        !commands.is_empty()
            || registry.is_some_and(|registry| registry.scopes.get(owner) != Some(scope))
    }) else {
        // Empty ticks still need a changed scope processed, but must not dirty
        // the registry or create it before the first generated resource exists.
        return;
    };
    world.init_resource::<DynamicMeshes>();
    let mut changed = HashSet::new();
    let mut replacements = HashMap::new();
    world.resource_scope(|world, mut registry: Mut<DynamicMeshes>| {
        for (owner, scope, commands) in batches.into_iter().skip(first_work) {
            if commands.is_empty() && registry.scopes.get(&owner) == Some(&scope) {
                continue;
            }
            let worker = world
                .get_non_send_resource::<crate::js::ScriptRuntimeManager>()
                .and_then(|m| m.contexts.get(&owner))
                .and_then(|w| w.join.as_ref())
                .map(|j| j.thread().id());
            // A restarted isolate cannot inherit resources from its predecessor.
            // A first upload cannot own any previous entries. Avoid an O(n) scan
            // per new isolate when a street loads thousands of independent pieces.
            if registry
                .scopes
                .insert(owner, scope)
                .is_some_and(|old| old != scope)
            {
                let stale: Vec<_> = registry
                    .entries
                    .iter()
                    .filter(|(_, entry)| entry.owner == owner && entry.scope != scope)
                    .map(|(src, _)| src.clone())
                    .collect();
                for src in stale {
                    registry.remove(&src);
                    replacements.remove(&src);
                    changed.insert(src);
                }
            }
            for command in commands {
                match command {
                    MeshCommand::Dispose(src) => {
                        if registry
                            .entries
                            .get(&src)
                            .is_some_and(|e| e.owner == owner && e.scope == scope)
                        {
                            registry.remove(&src);
                            replacements.remove(&src);
                            changed.insert(src);
                        }
                    }
                    MeshCommand::Upload(src, upload) => {
                        if let Some(entry) = registry.entries.get(&src) {
                            if entry.owner != owner || entry.scope != scope {
                                continue;
                            }
                        }
                        let fingerprint = blake3::Hash::from_bytes(upload.fingerprint);
                        if registry
                            .entries
                            .get(&src)
                            .is_some_and(|e| e.fingerprint == fingerprint)
                        {
                            // Repeated uploads need no normals, asset events or bounds updates.
                            continue;
                        }
                        let previous = registry.entries.remove(&src);
                        let reusable = previous.as_ref().and_then(|entry| {
                            (registry.geometry[&entry.fingerprint].owners == 1)
                                .then(|| entry.mesh.clone())
                        });
                        if let Some(entry) = &previous {
                            registry.release_geometry(entry.fingerprint);
                        }
                        let (handle, aabb) =
                            if let Some(shared) = registry.geometry.get_mut(&fingerprint) {
                                shared.owners += 1;
                                (shared.mesh.clone(), shared.aabb)
                            } else {
                                let aabb = Aabb::from_min_max(
                                    upload.bounds_min.into(),
                                    upload.bounds_max.into(),
                                );
                                let mesh = build_mesh(upload.data);
                                let mut assets = world.resource_mut::<Assets<Mesh>>();
                                let handle = if let Some(handle) = reusable {
                                    // Animated meshes keep their allocation when no other resource
                                    // shares it. A shared static mesh detaches on its first edit.
                                    assets.insert(handle.id(), mesh);
                                    handle
                                } else {
                                    assets.add(mesh)
                                };
                                registry.geometry.insert(
                                    fingerprint,
                                    SharedGeometry {
                                        mesh: handle.clone(),
                                        aabb,
                                        owners: 1,
                                    },
                                );
                                (handle, aabb)
                            };
                        // Keep only the final live upload for each source.
                        // Dispose/reupload can retain the same asset path, for
                        // which model reconciliation preserves the instance;
                        // it therefore also needs the final mesh and bounds.
                        replacements.insert(src.clone(), (handle.clone(), aabb));
                        if let Some(mut entry) = previous {
                            entry.mesh = handle.clone();
                            entry.fingerprint = fingerprint;
                            entry.model.generated.as_mut().unwrap().mesh = handle.clone();
                            entry.model.generated.as_mut().unwrap().aabb = aabb;
                            registry.entries.insert(src.clone(), entry);
                        } else {
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
                            registry.entries.insert(
                                src.clone(),
                                Entry {
                                    owner,
                                    scope,
                                    worker,
                                    mesh: handle.clone(),
                                    fingerprint,
                                    model: ModelResource {
                                        asset_path: src.clone(),
                                        scene: Handle::default(),
                                        gltf: None,
                                        generated: Some(GeneratedModel {
                                            mesh: handle,
                                            material,
                                            aabb,
                                        }),
                                    },
                                },
                            );
                            changed.insert(src);
                        }
                    }
                }
            }
        }
    });
    if !changed.is_empty() || !replacements.is_empty() {
        let mut contents = Vec::new();
        let mut dirty_ids = Vec::new();
        for mut instance in world.query::<&mut ModelInstance>().iter_mut(world) {
            if changed.contains(&instance.source) {
                dirty_ids.push(instance.node_id);
            }
            if let Some((mesh, aabb)) = replacements.get(&instance.source) {
                if !instance
                    .resource
                    .as_ref()
                    .is_some_and(|r| r.generated.is_some())
                {
                    continue;
                }
                if let Some(generated) = instance
                    .resource
                    .as_mut()
                    .and_then(|r| r.generated.as_mut())
                {
                    generated.mesh = mesh.clone();
                    generated.aabb = *aabb;
                    if let Some(content) = instance.content {
                        contents.push((content, mesh.clone(), *aabb));
                    }
                }
            }
        }
        for (content, mesh, aabb) in contents {
            if let Some(mut entity) = world.get_entity_mut(content) {
                entity.insert((mesh, aabb));
            }
        }
        if !dirty_ids.is_empty() {
            if let Some(mut dirty) = world.get_resource_mut::<DirtyNodes>() {
                dirty.0.extend(dirty_ids.iter().copied());
            }
            if let Some(mut fast) = world.get_resource_mut::<TransformOnlyDirtyNodes>() {
                for id in dirty_ids {
                    fast.0.remove(&id);
                }
            }
        }
    }
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
    let live: HashMap<_, _> = manager
        .contexts
        .iter()
        .map(|(id, w)| (*id, w.join.as_ref().map(|j| j.thread().id())))
        .collect();
    let mut removed = HashSet::new();
    let mut registry = world.resource_mut::<DynamicMeshes>();
    registry.context_generation = Some(generation);
    registry.scopes.retain(|owner, _| live.contains_key(owner));
    removed.extend(
        registry
            .entries
            .iter()
            .filter(|(_, e)| live.get(&e.owner) != Some(&e.worker))
            .map(|(src, _)| src.clone()),
    );
    for src in &removed {
        registry.remove(src);
    }
    invalidate(world, &removed);
}

#[cfg(test)]
mod tests {
    use super::*;
    use js_runtime::mesh::MeshUpload;

    fn triangle(width: f32) -> MeshData {
        MeshData {
            positions: vec![[0., 0., 0.], [width, 0., 0.], [0., 1., 0.]],
            indices: vec![0, 1, 2],
            normals: Vec::new(),
            uvs: Vec::new(),
            colors: Vec::new(),
        }
    }

    fn world() -> World {
        let mut world = World::new();
        world.init_resource::<Assets<Mesh>>();
        world.init_resource::<Assets<StandardMaterial>>();
        world
    }

    fn upload(world: &mut World, owner: u32, src: &str, width: f32) {
        apply_commands(
            world,
            owner,
            u64::from(owner),
            vec![MeshCommand::Upload(
                src.into(),
                MeshUpload::new(triangle(width)),
            )],
        );
    }

    fn mount(world: &mut World, src: &str, owner: u32) -> (Entity, Entity) {
        let resource = world
            .resource::<DynamicMeshes>()
            .get(src, Some(owner))
            .unwrap()
            .clone();
        let generated = resource.generated.as_ref().unwrap();
        let content = world
            .spawn((
                PbrBundle {
                    mesh: generated.mesh.clone(),
                    material: generated.material.clone(),
                    transform: Transform::from_xyz(4., 5., 6.),
                    ..default()
                },
                generated.aabb,
            ))
            .id();
        let root = world
            .spawn(ModelInstance {
                node_id: owner,
                source: src.into(),
                generation: 7,
                resource: Some(resource),
                content: Some(content),
                status: crate::models::ModelStatus::Ready,
            })
            .id();
        (root, content)
    }

    #[test]
    fn shared_geometry_detaches_on_edits_and_preserves_instance_state() {
        let mut world = world();
        upload(&mut world, 1, "mesh://1/1", 1.);
        upload(&mut world, 2, "mesh://2/1", 1.);
        assert_eq!(world.resource::<Assets<Mesh>>().len(), 1);
        assert_eq!(world.resource::<DynamicMeshes>().geometry.len(), 1);
        assert!(world
            .resource::<DynamicMeshes>()
            .get("mesh://1/1", Some(2))
            .is_none());
        let (root_a, a) = mount(&mut world, "mesh://1/1", 1);
        let (_, b) = mount(&mut world, "mesh://2/1", 2);
        let original = world.get::<Handle<Mesh>>(b).unwrap().clone();
        let material = world.get::<Handle<StandardMaterial>>(a).unwrap().clone();
        assert_eq!(world.get::<Handle<Mesh>>(a).unwrap(), &original);
        upload(&mut world, 1, "mesh://1/1", 2.);
        let detached = world.get::<Handle<Mesh>>(a).unwrap().clone();
        assert_ne!(detached, original);
        assert_eq!(world.get::<Handle<Mesh>>(b).unwrap(), &original);
        assert_eq!(world.get::<Aabb>(a).unwrap().half_extents.x, 1.);
        assert_eq!(world.get::<Aabb>(b).unwrap().half_extents.x, 0.5);
        assert_eq!(
            *world.get::<Transform>(a).unwrap(),
            Transform::from_xyz(4., 5., 6.)
        );
        assert_eq!(world.get::<Handle<StandardMaterial>>(a).unwrap(), &material);
        assert_eq!(world.get::<ModelInstance>(root_a).unwrap().generation, 7);
        // An unshared animated mesh continues reusing its asset allocation.
        upload(&mut world, 1, "mesh://1/1", 3.);
        assert_eq!(world.get::<Handle<Mesh>>(a).unwrap(), &detached);
        assert_eq!(world.resource::<Assets<Mesh>>().len(), 2);
        assert_eq!(world.get::<Aabb>(a).unwrap().half_extents.x, 1.5);
        // Returning to identical geometry rejoins the original shared asset.
        upload(&mut world, 1, "mesh://1/1", 1.);
        assert_eq!(world.get::<Handle<Mesh>>(a).unwrap(), &original);
        assert_eq!(world.resource::<DynamicMeshes>().geometry.len(), 1);
        assert_eq!(
            world
                .resource::<DynamicMeshes>()
                .geometry
                .values()
                .next()
                .unwrap()
                .owners,
            2
        );
    }

    #[test]
    fn identical_uploads_do_not_touch_assets_or_instances_and_scopes_release_cache() {
        let mut world = world();
        upload(&mut world, 1, "mesh://1/1", 1.);
        upload(&mut world, 2, "mesh://2/1", 1.);
        let (root, content) = mount(&mut world, "mesh://1/1", 1);
        world.clear_trackers();
        upload(&mut world, 1, "mesh://1/1", 1.);
        assert!(!world
            .get_resource_ref::<Assets<Mesh>>()
            .unwrap()
            .is_changed());
        assert!(!world
            .entity(root)
            .get_ref::<ModelInstance>()
            .unwrap()
            .is_changed());
        assert!(!world
            .entity(content)
            .get_ref::<Handle<Mesh>>()
            .unwrap()
            .is_changed());
        apply_commands(&mut world, 1, 11, Vec::new());
        let registry = world.resource::<DynamicMeshes>();
        assert!(registry.get("mesh://1/1", Some(1)).is_none());
        assert_eq!(registry.geometry.values().next().unwrap().owners, 1);
        apply_commands(
            &mut world,
            2,
            2,
            vec![MeshCommand::Dispose("mesh://2/1".into())],
        );
        assert!(world.resource::<DynamicMeshes>().geometry.is_empty());
    }

    #[test]
    fn command_batches_keep_final_updates_and_invalidate_disposed_or_recreated_sources() {
        let mut world = world();
        world.init_resource::<DirtyNodes>();
        world.init_resource::<TransformOnlyDirtyNodes>();
        for owner in 1..=4 {
            upload(&mut world, owner, &format!("mesh://{owner}/1"), 1.);
        }
        let mounted: Vec<_> = (1..=4)
            .map(|owner| mount(&mut world, &format!("mesh://{owner}/1"), owner))
            .collect();
        let original = world.get::<Handle<Mesh>>(mounted[0].1).unwrap().clone();
        world
            .resource_mut::<TransformOnlyDirtyNodes>()
            .0
            .extend(1..=4);
        world.clear_trackers();
        let put = |owner, width| {
            MeshCommand::Upload(
                format!("mesh://{owner}/1"),
                MeshUpload::new(triangle(width)),
            )
        };
        apply_command_batches(
            &mut world,
            vec![
                (1, 1, vec![put(1, 2.)]),
                (2, 2, vec![put(2, 4.)]),
                (3, 3, vec![put(3, 5.)]),
                (1, 1, vec![put(1, 3.), put(1, 3.)]),
                (2, 2, vec![MeshCommand::Dispose("mesh://2/1".into())]),
                (3, 33, Vec::new()),
                (3, 33, vec![put(3, 6.), put(3, 7.)]),
                (
                    4,
                    4,
                    vec![MeshCommand::Dispose("mesh://1/1".into()), put(1, 99.)],
                ),
            ],
        );
        let registry = world.resource::<DynamicMeshes>();
        let final_a = registry.get("mesh://1/1", Some(1)).unwrap();
        assert_eq!(final_a.generated.as_ref().unwrap().aabb.half_extents.x, 1.5);
        assert!(registry.get("mesh://2/1", Some(2)).is_none());
        assert_eq!(
            registry
                .get("mesh://3/1", Some(3))
                .unwrap()
                .generated
                .as_ref()
                .unwrap()
                .aabb
                .half_extents
                .x,
            3.5
        );
        assert_eq!(registry.geometry.len(), 3);
        assert!(registry
            .geometry
            .values()
            .all(|geometry| geometry.owners == 1));
        assert_eq!(world.get::<Aabb>(mounted[0].1).unwrap().half_extents.x, 1.5);
        assert_eq!(
            world.get::<Handle<Mesh>>(mounted[0].1).unwrap(),
            &final_a.generated.as_ref().unwrap().mesh
        );
        // Finally disposed and untouched models never receive an intermediate
        // replacement; recreated sources receive the final live resource.
        for index in [1, 3] {
            let (root, content) = mounted[index];
            assert_eq!(world.get::<Handle<Mesh>>(content).unwrap(), &original);
            assert_eq!(world.get::<Aabb>(content).unwrap().half_extents.x, 0.5);
            assert!(!world
                .entity(root)
                .get_ref::<ModelInstance>()
                .unwrap()
                .is_changed());
        }
        assert_eq!(world.get::<Aabb>(mounted[2].1).unwrap().half_extents.x, 3.5);
        assert_eq!(
            world.get::<Handle<Mesh>>(mounted[2].1).unwrap(),
            &registry
                .get("mesh://3/1", Some(3))
                .unwrap()
                .generated
                .as_ref()
                .unwrap()
                .mesh
        );
        assert_eq!(world.resource::<DirtyNodes>().0.len(), 2);
        assert_eq!(
            world
                .resource::<DirtyNodes>()
                .0
                .iter()
                .copied()
                .collect::<HashSet<_>>(),
            HashSet::from([2, 3])
        );
        assert_eq!(
            world.resource::<TransformOnlyDirtyNodes>().0,
            HashSet::from([1, 4])
        );
    }

    #[test]
    fn command_batches_preserve_scope_order_and_skip_all_idle_responses() {
        let mut world = world();
        apply_command_batches(&mut world, vec![(1, 1, Vec::new()), (2, 2, Vec::new())]);
        assert!(!world.contains_resource::<DynamicMeshes>());
        upload(&mut world, 1, "mesh://1/1", 1.);
        let (root, content) = mount(&mut world, "mesh://1/1", 1);
        world.init_resource::<DirtyNodes>();
        world.clear_trackers();
        apply_command_batches(&mut world, vec![(1, 1, Vec::new()), (1, 1, Vec::new())]);
        assert!(!world
            .get_resource_ref::<DynamicMeshes>()
            .unwrap()
            .is_changed());
        assert!(!world
            .entity(root)
            .get_ref::<ModelInstance>()
            .unwrap()
            .is_changed());
        assert!(!world
            .entity(content)
            .get_ref::<Aabb>()
            .unwrap()
            .is_changed());
        assert!(!world.get_resource_ref::<DirtyNodes>().unwrap().is_changed());
        // Returning to the initially active scope is still a transition after
        // the preceding response removed its resources.
        apply_command_batches(&mut world, vec![(1, 11, Vec::new()), (1, 1, Vec::new())]);
        assert_eq!(world.resource::<DynamicMeshes>().scopes.get(&1), Some(&1));
        assert!(world.resource::<DynamicMeshes>().entries.is_empty());
        assert_eq!(world.resource::<DirtyNodes>().0, vec![1]);
    }

    #[test]
    fn batched_dispose_and_reupload_updates_the_surviving_instance_to_final_geometry() {
        let mut world = world();
        upload(&mut world, 1, "mesh://1/1", 1.);
        upload(&mut world, 2, "mesh://2/1", 1.);
        let (root, content) = mount(&mut world, "mesh://1/1", 1);
        let original = world.get::<Handle<Mesh>>(content).unwrap().clone();
        world.init_resource::<DirtyNodes>();
        world.clear_trackers();
        apply_command_batches(
            &mut world,
            vec![
                (
                    1,
                    1,
                    vec![MeshCommand::Upload(
                        "mesh://1/1".into(),
                        MeshUpload::new(triangle(2.)),
                    )],
                ),
                (1, 1, vec![MeshCommand::Dispose("mesh://1/1".into())]),
                (
                    1,
                    1,
                    vec![MeshCommand::Upload(
                        "mesh://1/1".into(),
                        MeshUpload::new(triangle(3.)),
                    )],
                ),
            ],
        );
        let registry = world.resource::<DynamicMeshes>();
        assert_eq!(
            registry
                .get("mesh://1/1", Some(1))
                .unwrap()
                .generated
                .as_ref()
                .unwrap()
                .aabb
                .half_extents
                .x,
            1.5
        );
        assert_eq!(registry.geometry.len(), 2);
        assert_ne!(world.get::<Handle<Mesh>>(content).unwrap(), &original);
        assert_eq!(
            world.get::<Handle<Mesh>>(content).unwrap(),
            &registry
                .get("mesh://1/1", Some(1))
                .unwrap()
                .generated
                .as_ref()
                .unwrap()
                .mesh
        );
        assert_eq!(world.get::<Aabb>(content).unwrap().half_extents.x, 1.5);
        assert!(world
            .entity(root)
            .get_ref::<ModelInstance>()
            .unwrap()
            .is_changed());
        assert_eq!(world.get::<ModelInstance>(root).unwrap().generation, 7);
        assert_eq!(world.resource::<DirtyNodes>().0, vec![1]);
    }

    /// Isolates the host-side cost of scanning mounted models per response.
    /// Both paths receive already prepared geometry and use the same registry;
    /// only the grouping of responses changes. This does not measure rendering.
    #[test]
    #[ignore = "manual CPU benchmark: run with --ignored --nocapture"]
    fn benchmark_mesh_command_batches() {
        use std::{hint::black_box, time::Instant};
        const INSTANCES: u32 = 5_000;
        const SAMPLES: usize = 5;
        fn fixture(owners: u32, update: bool) -> World {
            let mut world = world();
            world.init_resource::<DirtyNodes>();
            if update {
                apply_command_batches(
                    &mut world,
                    (1..=owners)
                        .map(|owner| {
                            (
                                owner,
                                u64::from(owner),
                                vec![MeshCommand::Upload(
                                    format!("mesh://{owner}/1"),
                                    MeshUpload::new(triangle(1.)),
                                )],
                            )
                        })
                        .collect(),
                );
            }
            for id in 0..INSTANCES {
                let owner = id % owners + 1;
                let source = format!("mesh://{owner}/1");
                let resource = world
                    .get_resource::<DynamicMeshes>()
                    .and_then(|registry| registry.get(&source, Some(owner)))
                    .cloned();
                world.spawn(ModelInstance {
                    node_id: id,
                    source,
                    generation: 1,
                    resource,
                    content: None,
                    status: crate::models::ModelStatus::Loading,
                });
            }
            world
        }
        for owners in [25, 250, 1_000] {
            for update in [false, true] {
                let mut individual_ms = Vec::new();
                let mut batched_ms = Vec::new();
                for sample in 0..SAMPLES {
                    for batched in [sample % 2 == 0, sample % 2 != 0] {
                        let mut world = fixture(owners, update);
                        let batches: Vec<_> = (1..=owners)
                            .map(|owner| {
                                (
                                    owner,
                                    u64::from(owner),
                                    vec![MeshCommand::Upload(
                                        format!("mesh://{owner}/1"),
                                        MeshUpload::new(triangle(2. + owner as f32 * 0.01)),
                                    )],
                                )
                            })
                            .collect();
                        let start = Instant::now();
                        if batched {
                            apply_command_batches(&mut world, batches);
                        } else {
                            for (owner, scope, commands) in batches {
                                apply_commands(&mut world, owner, scope, commands);
                            }
                        }
                        let elapsed = start.elapsed().as_secs_f64() * 1_000.;
                        if batched {
                            batched_ms.push(elapsed);
                        } else {
                            individual_ms.push(elapsed);
                        }
                        assert_eq!(
                            world.resource::<DynamicMeshes>().entries.len(),
                            owners as usize
                        );
                        assert_eq!(
                            world.resource::<DirtyNodes>().0.len(),
                            if update { 0 } else { INSTANCES as usize }
                        );
                        if update {
                            for instance in world.query::<&ModelInstance>().iter(&world) {
                                let owner = instance.node_id % owners + 1;
                                let actual = instance
                                    .resource
                                    .as_ref()
                                    .unwrap()
                                    .generated
                                    .as_ref()
                                    .unwrap()
                                    .aabb
                                    .half_extents
                                    .x;
                                assert_eq!(actual, (2. + owner as f32 * 0.01) * 0.5);
                            }
                        }
                        black_box(world.resource::<DynamicMeshes>().geometry.len());
                    }
                }
                individual_ms.sort_by(f64::total_cmp);
                batched_ms.sort_by(f64::total_cmp);
                let individual = individual_ms[SAMPLES / 2];
                let batched = batched_ms[SAMPLES / 2];
                eprintln!("mesh_command_batches instances={INSTANCES} owners={owners} update={update} samples={SAMPLES} individual_median_ms={individual:.4} batched_median_ms={batched:.4} ratio={:.4}", batched / individual);
            }
        }
    }

    #[test]
    fn render_only_meshes_can_be_shared_updated_and_released_after_extraction() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .init_asset::<Mesh>()
            .init_asset::<StandardMaterial>()
            .init_resource::<DirtyNodes>();
        let world = app.world_mut();
        upload(world, 1, "mesh://1/1", 1.);
        let (root_a, a) = mount(world, "mesh://1/1", 1);
        let original = world.get::<Handle<Mesh>>(a).unwrap().clone();
        // Bevy 0.14 extraction moves RENDER_WORLD-only data with Assets::remove;
        // handles remain alive and refer to the prepared mesh in RenderAssets.
        let extracted = world
            .resource_mut::<Assets<Mesh>>()
            .remove(original.id())
            .unwrap();
        assert_eq!(extracted.asset_usage, RenderAssetUsages::RENDER_WORLD);
        drop(extracted);
        world.clear_trackers();
        upload(world, 1, "mesh://1/1", 1.);
        assert!(!world
            .get_resource_ref::<Assets<Mesh>>()
            .unwrap()
            .is_changed());
        upload(world, 2, "mesh://2/1", 1.);
        assert!(world.resource::<Assets<Mesh>>().is_empty());
        let (root_b, b) = mount(world, "mesh://2/1", 2);
        assert_eq!(world.get::<Handle<Mesh>>(b).unwrap(), &original);
        assert_eq!(world.get::<Aabb>(b).unwrap().half_extents.x, 0.5);

        upload(world, 1, "mesh://1/1", 2.);
        let detached = world.get::<Handle<Mesh>>(a).unwrap().clone();
        assert_ne!(detached, original);
        assert_eq!(world.get::<Handle<Mesh>>(b).unwrap(), &original);
        assert_eq!(world.get::<Aabb>(a).unwrap().half_extents.x, 1.);
        world
            .resource_mut::<Assets<Mesh>>()
            .remove(detached.id())
            .unwrap();
        upload(world, 1, "mesh://1/1", 3.);
        assert_eq!(world.get::<Handle<Mesh>>(a).unwrap(), &detached);
        assert!(world.resource::<Assets<Mesh>>().contains(detached.id()));
        assert_eq!(world.get::<Aabb>(a).unwrap().half_extents.x, 1.5);
        world
            .resource_mut::<Assets<Mesh>>()
            .remove(detached.id())
            .unwrap();

        apply_commands(world, 1, 1, vec![MeshCommand::Dispose("mesh://1/1".into())]);
        assert!(world.resource::<DirtyNodes>().0.contains(&1));
        assert_eq!(world.resource::<DynamicMeshes>().geometry.len(), 1);
        apply_commands(world, 2, 2, vec![MeshCommand::Dispose("mesh://2/1".into())]);
        assert!(world.resource::<DirtyNodes>().0.contains(&2));
        assert!(world.resource::<DynamicMeshes>().geometry.is_empty());
        // Removing document instances releases the final handles, even though
        // neither mesh has a CPU asset left for the asset tracker to remove.
        for entity in [root_a, root_b, a, b] {
            world.despawn(entity);
        }
        let ids = [original.id(), detached.id()];
        drop((original, detached));
        app.update();
        let unused: HashSet<_> = app
            .world_mut()
            .resource_mut::<Events<AssetEvent<Mesh>>>()
            .drain()
            .filter_map(|event| match event {
                AssetEvent::Unused { id } => Some(id),
                _ => None,
            })
            .collect();
        assert!(ids.iter().all(|id| unused.contains(id)));
    }

    #[test]
    fn fingerprint_includes_all_attributes_and_indices_choose_safe_width() {
        let base = MeshUpload::new(triangle(1.)).fingerprint;
        assert_ne!(base, MeshUpload::new(triangle(2.)).fingerprint);
        let mut changed = triangle(1.);
        changed.colors = vec![[1., 0., 0., 1.]; 3];
        assert_ne!(base, MeshUpload::new(changed).fingerprint);
        let mut changed = triangle(1.);
        changed.normals = vec![[0., 0., 1.]; 3];
        assert_eq!(base, MeshUpload::new(changed).fingerprint);
        let mut changed = triangle(1.);
        changed.normals = vec![[0., 1., 0.]; 3];
        assert_ne!(base, MeshUpload::new(changed).fingerprint);
        let mut changed = triangle(1.);
        changed.uvs = vec![[0., 0.]; 3];
        assert_ne!(base, MeshUpload::new(changed).fingerprint);
        let mut changed = triangle(1.);
        changed.indices.reverse();
        assert_ne!(base, MeshUpload::new(changed).fingerprint);
        assert!(matches!(
            build_mesh(MeshUpload::new(triangle(1.)).data).indices(),
            Some(Indices::U16(_))
        ));
        for count in [65_536, 65_537] {
            let mesh = build_mesh(MeshData {
                positions: vec![[0., 0., 0.]; count],
                indices: vec![0, 1, count as u32 - 1],
                normals: vec![[0., 1., 0.]; count],
                uvs: Vec::new(),
                colors: Vec::new(),
            });
            match mesh.indices().unwrap() {
                Indices::U16(indices) => {
                    assert_eq!(count, 65_536);
                    assert_eq!(indices[2], u16::MAX);
                }
                Indices::U32(indices) => {
                    assert_eq!(count, 65_537);
                    assert_eq!(indices[2], 65_536);
                }
            }
        }
    }

    #[test]
    fn worker_preparation_preserves_area_weighted_normals_and_bevy_bounds() {
        let upload = MeshUpload::new(MeshData {
            positions: vec![
                [0., 0., 0.],
                [2., 0., 0.],
                [0., 3., 0.],
                [0., 0., 4.],
                [-2., -3., -4.],
                [7., 8., 9.],
            ],
            indices: vec![0, 1, 2, 0, 3, 1, 4, 4, 4],
            normals: Vec::new(),
            uvs: Vec::new(),
            colors: Vec::new(),
        });
        assert_eq!(
            upload.data.normals,
            vec![
                [0., 0.8, 0.6],
                [0., 0.8, 0.6],
                [0., 0., 1.],
                [0., 1., 0.],
                [0., 0., 0.],
                [0., 0., 0.],
            ]
        );
        let bounds = Aabb::from_min_max(upload.bounds_min.into(), upload.bounds_max.into());
        assert_eq!(build_mesh(upload.data).compute_aabb(), Some(bounds));
        assert_eq!(upload.bounds_min, [-2., -3., -4.]);
        assert_eq!(upload.bounds_max, [7., 8., 9.]);
        let mut supplied = triangle(1.);
        supplied.normals = vec![[0.2, 0.3, 0.4]; 3];
        let upload = MeshUpload::new(supplied);
        assert_eq!(upload.data.normals, vec![[0.2, 0.3, 0.4]; 3]);
    }

    /// CPU upload benchmark; excludes fixture construction, DOM/JS execution,
    /// rendering and GPU submission. Worker preparation is timed separately from
    /// the rendering thread; its total is reported without claiming a CPU saving.
    /// The reference builds/adds identical meshes and computes their bounds.
    #[test]
    #[ignore = "manual CPU benchmark: run with --ignored --nocapture"]
    fn benchmark_static_mesh_uploads() {
        use std::{hint::black_box, time::Instant};
        const COPIES: u32 = 48;
        fn fixture(index: u32, normals: bool) -> MeshData {
            const CUBES: usize = 4096;
            const FACES: [u32; 36] = [
                0, 1, 3, 0, 3, 2, 4, 6, 7, 4, 7, 5, 0, 4, 5, 0, 5, 1, 2, 3, 7, 2, 7, 6, 0, 2, 6, 0,
                6, 4, 1, 5, 7, 1, 7, 3,
            ];
            let mut data = MeshData {
                positions: Vec::with_capacity(CUBES * 8),
                indices: Vec::with_capacity(CUBES * 36),
                normals: Vec::new(),
                colors: Vec::with_capacity(CUBES * 8),
                uvs: Vec::new(),
            };
            for cube in 0..CUBES {
                for corner in 0..8 {
                    data.positions.push([
                        (cube % 64) as f32 + ((corner & 4) >> 2) as f32 * 0.9,
                        (cube / 64) as f32 + ((corner & 2) >> 1) as f32 * 0.9,
                        (corner & 1) as f32 * 0.2,
                    ]);
                    data.colors
                        .push([0.35 + (cube % 9) as f32 * 0.01, 0.2, 0.15, 1.]);
                }
                data.indices.extend(FACES.map(|i| cube as u32 * 8 + i));
            }
            data.positions[0][0] += index as f32 * 0.0001;
            if normals {
                data.normals = vec![[0., 0., 1.]; data.positions.len()];
            }
            data
        }
        for normals in [false, true] {
            for case in ["cold_unique", "hot_shared", "animated_exclusive"] {
                let fixtures = || {
                    (0..COPIES)
                        .map(|i| fixture(if case == "hot_shared" { 0 } else { i }, normals))
                        .collect::<Vec<_>>()
                };
                let mut reference = Assets::<Mesh>::default();
                let mut reference_handle: Option<Handle<Mesh>> = None;
                let data = fixtures();
                let start = Instant::now();
                for mut data in data {
                    data.generate_missing_normals();
                    let mesh = build_mesh(data);
                    black_box(mesh.compute_aabb());
                    if case == "animated_exclusive" && reference_handle.is_some() {
                        reference.insert(reference_handle.as_ref().unwrap().id(), mesh);
                    } else {
                        reference_handle = Some(reference.add(mesh));
                    }
                }
                let reference_ms = start.elapsed().as_secs_f64() * 1000.;
                black_box(reference.len());
                drop(reference);
                let mut world = world();
                let data = fixtures();
                let start = Instant::now();
                let uploads: Vec<_> = data.into_iter().map(MeshUpload::new).collect();
                let worker_prepare_ms = start.elapsed().as_secs_f64() * 1000.;
                let start = Instant::now();
                for (i, upload) in uploads.into_iter().enumerate() {
                    let owner = if case == "animated_exclusive" {
                        1
                    } else {
                        i as u32 + 1
                    };
                    apply_commands(
                        &mut world,
                        owner,
                        u64::from(owner),
                        vec![MeshCommand::Upload(format!("mesh://{owner}/1"), upload)],
                    );
                }
                let shared_ms = start.elapsed().as_secs_f64() * 1000.;
                let assets = world.resource::<Assets<Mesh>>().len();
                assert_eq!(
                    assets,
                    if case == "cold_unique" {
                        COPIES as usize
                    } else {
                        1
                    }
                );
                eprintln!("mesh_upload case={case} normals={normals} copies={COPIES} vertices=32768 indices=147456 reference_ms={reference_ms:.3} worker_prepare_ms={worker_prepare_ms:.3} optimized_ms={shared_ms:.3} main_ratio={:.3} combined_ms={:.3} assets={assets}", shared_ms / reference_ms, worker_prepare_ms + shared_ms);
            }
        }
    }

    #[test]
    fn empty_mesh_ticks_preserve_change_detection_but_process_scope_changes() {
        let mut world = World::new();
        apply_commands(&mut world, 1, 10, Vec::new());
        assert!(!world.contains_resource::<DynamicMeshes>());
        world.init_resource::<DynamicMeshes>();
        apply_commands(&mut world, 1, 10, Vec::new());
        world.clear_trackers();
        apply_commands(&mut world, 1, 10, Vec::new());
        assert!(!world
            .get_resource_ref::<DynamicMeshes>()
            .unwrap()
            .is_changed());
        apply_commands(&mut world, 1, 11, Vec::new());
        assert_eq!(world.resource::<DynamicMeshes>().scopes.get(&1), Some(&11));
        assert!(world
            .get_resource_ref::<DynamicMeshes>()
            .unwrap()
            .is_changed());
    }
}
