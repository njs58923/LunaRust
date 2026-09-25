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

/// Hash the validated upload before building normals or allocating a GPU asset.
/// Length-prefix each attribute so different layouts cannot alias. These are
/// in-memory keys, so native-endian POD slices avoid packing or copying uploads.
fn mesh_fingerprint(data: &MeshData) -> blake3::Hash {
    let mut hasher = blake3::Hasher::new();
    for bytes in [
        bytemuck::cast_slice(&data.positions),
        bytemuck::cast_slice(&data.indices),
        bytemuck::cast_slice(&data.normals),
        bytemuck::cast_slice(&data.uvs),
        bytemuck::cast_slice(&data.colors),
    ] {
        hasher.update(&(bytes.len() as u64).to_le_bytes());
        hasher.update(bytes);
    }
    hasher.finalize()
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
    let worker = world
        .get_non_send_resource::<crate::js::ScriptRuntimeManager>()
        .and_then(|m| m.contexts.get(&owner))
        .and_then(|w| w.join.as_ref())
        .map(|j| j.thread().id());
    let mut changed = HashSet::new();
    let mut replacements = HashMap::new();
    world.resource_scope(|world, mut registry: Mut<DynamicMeshes>| {
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
                        changed.insert(src);
                    }
                }
                MeshCommand::Upload(src, data) => {
                    if let Some(entry) = registry.entries.get(&src) {
                        if entry.owner != owner || entry.scope != scope {
                            continue;
                        }
                    }
                    let fingerprint = mesh_fingerprint(&data);
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
                            let mesh = build_mesh(data);
                            let aabb = mesh.compute_aabb().expect("validated mesh has positions");
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
                    if let Some(mut entry) = previous {
                        entry.mesh = handle.clone();
                        entry.fingerprint = fingerprint;
                        entry.model.generated.as_mut().unwrap().mesh = handle.clone();
                        entry.model.generated.as_mut().unwrap().aabb = aabb;
                        registry.entries.insert(src.clone(), entry);
                        replacements.insert(src, (handle, aabb));
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
    });
    if !replacements.is_empty() {
        let mut contents = Vec::new();
        for mut instance in world.query::<&mut ModelInstance>().iter_mut(world) {
            if let Some((mesh, aabb)) = replacements.get(&instance.source) {
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
            vec![MeshCommand::Upload(src.into(), triangle(width))],
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
        let base = mesh_fingerprint(&triangle(1.));
        assert_ne!(base, mesh_fingerprint(&triangle(2.)));
        let mut changed = triangle(1.);
        changed.colors = vec![[1., 0., 0., 1.]; 3];
        assert_ne!(base, mesh_fingerprint(&changed));
        let mut changed = triangle(1.);
        changed.normals = vec![[0., 0., 1.]; 3];
        assert_ne!(base, mesh_fingerprint(&changed));
        let mut changed = triangle(1.);
        changed.uvs = vec![[0., 0.]; 3];
        assert_ne!(base, mesh_fingerprint(&changed));
        let mut changed = triangle(1.);
        changed.indices.reverse();
        assert_ne!(base, mesh_fingerprint(&changed));
        assert!(matches!(
            build_mesh(triangle(1.)).indices(),
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

    /// CPU upload benchmark; excludes fixture construction, DOM/JS execution,
    /// rendering and GPU submission. The reference builds/adds exactly the same
    /// meshes and computes the bounds required for rendering, without sharing.
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
                for data in data {
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
                for (i, data) in data.into_iter().enumerate() {
                    let owner = if case == "animated_exclusive" {
                        1
                    } else {
                        i as u32 + 1
                    };
                    apply_commands(
                        &mut world,
                        owner,
                        u64::from(owner),
                        vec![MeshCommand::Upload(format!("mesh://{owner}/1"), data)],
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
                eprintln!("mesh_upload case={case} normals={normals} copies={COPIES} vertices=32768 indices=147456 reference_ms={reference_ms:.3} optimized_ms={shared_ms:.3} ratio={:.3} assets={assets}", shared_ms / reference_ms);
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
