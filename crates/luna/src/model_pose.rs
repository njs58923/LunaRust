//! Transport-independent instance bindings and sparse absolute local poses.
//! No pose work is scheduled for an idle model; clip ownership is explicit.
use crate::{
    model_animation::ModelAnimationConfig,
    models::{ModelInstance, ModelStatus},
    AttributeUpdates, EntityMap,
};
use bevy::{prelude::*, render::mesh::skinning::SkinnedMesh, transform::TransformSystem};
use js_runtime::pose::{JointPose, PoseBatch, MAX_JOINTS};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Component)]
pub struct PendingJointBinding;

#[derive(Component)]
pub struct ModelPoseBinding {
    generation: u64,
    token: String,
    nodes: Vec<(Entity, Transform)>,
    mode: String,
    last_error: Option<&'static str>,
    pending: HashMap<usize, JointPose>,
}

#[derive(Component)]
pub struct PendingPose;

pub struct ModelPosePlugin;
impl Plugin for ModelPosePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            PostUpdate,
            apply_poses
                .after(bevy::animation::animate_targets)
                .before(TransformSystem::TransformPropagate),
        );
    }
}

fn emit(world: &mut World, node: u32, key: &str, value: impl Into<String>) {
    world
        .resource_mut::<AttributeUpdates>()
        .0
        .push((node, key.into(), value.into()));
}

// Bevy 0.14 glTF scenes have one synthetic root. Match the immutable source
// scene hierarchy with its instance, never by names (which may be duplicated).
// Using source transforms avoids accidentally capturing an evaluated clip as rest.
fn build_binding(
    world: &World,
    model: &ModelInstance,
) -> Result<(ModelPoseBinding, String), String> {
    let resource = model.resource.as_ref().ok_or("Model has no scene")?;
    if resource.generated.is_some() {
        // Generated meshes mount directly on the content entity. Preserve the
        // same one-node pose catalog without manufacturing a Bevy Scene.
        let content = model.content.ok_or("Model has no content")?;
        if world.get::<Transform>(content).is_none() {
            return Err("Instance hierarchy changed".into());
        }
        return Ok(finish_binding(
            model,
            vec![(content, Transform::IDENTITY)],
            vec![
                serde_json::json!({"index":0,"name":null,"parent":null,"path":[],
                "translation":[0.,0.,0.],"rotation":[0.,0.,0.,1.],"scale":[1.,1.,1.]}),
            ],
            Vec::new(),
        ));
    }
    let scene = world
        .resource::<Assets<Scene>>()
        .get(&resource.scene)
        .ok_or("Scene unavailable")?;
    let roots: Vec<_> = scene
        .world
        .iter_entities()
        .filter(|e| !e.contains::<Parent>())
        .map(|e| e.id())
        .collect();
    let content = model.content.ok_or("Model has no content")?;
    let actual_roots = world.get::<Children>(content).ok_or("Scene has no root")?;
    if roots.len() != 1 || actual_roots.len() != 1 {
        return Err("Pose binding requires a single scene root".into());
    }
    let mut stack = vec![(roots[0], actual_roots[0], None, Vec::<usize>::new())];
    let mut nodes = Vec::new();
    let mut catalog = Vec::new();
    let mut source_indices = HashMap::new();
    while let Some((source, actual, parent, path)) = stack.pop() {
        if nodes.len() >= MAX_JOINTS {
            return Err("Pose binding exceeds 4096 nodes".into());
        }
        let rest = *scene
            .world
            .get::<Transform>(source)
            .ok_or("Scene node has no transform")?;
        if world.get::<Transform>(actual).is_none() {
            return Err("Instance hierarchy changed".into());
        }
        let index = nodes.len();
        source_indices.insert(source, index);
        nodes.push((actual, rest));
        catalog.push(serde_json::json!({"index":index,"name":scene.world.get::<Name>(source).map(Name::as_str),
            "parent":parent,"path":path,"translation":rest.translation.to_array(),
            "rotation":rest.rotation.to_array(),"scale":rest.scale.to_array()}));
        let sc = scene
            .world
            .get::<Children>(source)
            .map(|c| &c[..])
            .unwrap_or(&[]);
        let ac = world.get::<Children>(actual).map(|c| &c[..]).unwrap_or(&[]);
        if sc.len() != ac.len() {
            return Err("Instance hierarchy does not match source scene".into());
        }
        for i in (0..sc.len()).rev() {
            let mut child_path = path.clone();
            child_path.push(i);
            stack.push((sc[i], ac[i], Some(index), child_path));
        }
    }
    let mut skins = Vec::new();
    // Skin joint arrays retain the loader's glTF order; catalog indices include
    // non-joint ancestors so local TRS and future humanoid adapters are lossless.
    for entry in scene.world.iter_entities() {
        if let Some(skin) = entry.get::<SkinnedMesh>() {
            let joints: Option<Vec<_>> = skin
                .joints
                .iter()
                .map(|j| source_indices.get(j).copied())
                .collect();
            let joints = joints.ok_or("Skin references a node outside this scene")?;
            skins.push(serde_json::json!({"node":source_indices[&entry.id()], "joints":joints}));
        }
    }
    skins.sort_by_key(|s| s["node"].as_u64());
    Ok(finish_binding(model, nodes, catalog, skins))
}

fn finish_binding(
    model: &ModelInstance,
    nodes: Vec<(Entity, Transform)>,
    catalog: Vec<serde_json::Value>,
    skins: Vec<serde_json::Value>,
) -> (ModelPoseBinding, String) {
    let mut description = serde_json::json!({"version":1,"nodes":catalog,"skins":skins});
    let schema = blake3::hash(description.to_string().as_bytes())
        .to_hex()
        .to_string();
    static NEXT_BINDING: AtomicU64 = AtomicU64::new(1);
    let token = format!(
        "pose:{}:{}",
        model.generation,
        NEXT_BINDING.fetch_add(1, Ordering::Relaxed)
    );
    description["schema"] = schema.into();
    description["binding"] = token.clone().into();
    (
        ModelPoseBinding {
            generation: model.generation,
            token,
            nodes,
            mode: String::new(),
            last_error: None,
            pending: HashMap::new(),
        },
        description.to_string(),
    )
}

/// Called in RenderSync after scene readiness and before snapshots.
pub fn sync_bindings(world: &mut World) {
    let ready: Vec<_> = world
        .query_filtered::<(Entity, &ModelInstance, &ModelAnimationConfig), Or<(With<PendingJointBinding>, Changed<ModelAnimationConfig>)>>()
        .iter(world)
        .filter(|(_, m, _)| m.status != ModelStatus::Loading)
        .map(|(e, m, c)| (e, m.clone(), c.pose_source.clone()))
        .collect();
    for (entity, model, mode) in ready {
        world.entity_mut(entity).remove::<PendingJointBinding>();
        if model.status != ModelStatus::Ready {
            emit(
                world,
                model.node_id,
                "pose-status",
                if model.status == ModelStatus::Empty {
                    "empty"
                } else {
                    "error"
                },
            );
            continue;
        }
        if world
            .get::<ModelPoseBinding>(entity)
            .is_some_and(|b| b.generation == model.generation)
        {
            continue;
        }
        // Ordinary clip-only instances never pay for a pose catalog or binding.
        if mode == "clip" {
            emit(world, model.node_id, "pose-status", "clip");
            continue;
        }
        if mode != "script" {
            emit(world, model.node_id, "pose-status", "error");
            emit(
                world,
                model.node_id,
                "pose-error",
                "pose-source must be clip or script; native streams are not implemented",
            );
            continue;
        }
        match build_binding(world, &model) {
            Ok((binding, catalog)) => {
                world.entity_mut(entity).insert(binding);
                emit(world, model.node_id, "animation-joints", catalog);
                emit(world, model.node_id, "pose-error", "");
            }
            Err(error) => {
                emit(world, model.node_id, "pose-error", error);
                emit(world, model.node_id, "pose-status", "error");
            }
        }
    }
}

pub fn sync_pose_control(
    mut commands: Commands,
    mut models: Query<
        (
            Entity,
            &ModelInstance,
            &ModelAnimationConfig,
            &mut ModelPoseBinding,
        ),
        Or<(Changed<ModelAnimationConfig>, Added<ModelPoseBinding>)>,
    >,
    mut transforms: Query<&mut Transform>,
    mut updates: ResMut<AttributeUpdates>,
) {
    for (entity, model, config, mut binding) in &mut models {
        let mode = config.pose_source.as_str();
        if binding.mode == mode {
            continue;
        }
        binding.mode = mode.into();
        binding.last_error = None;
        binding.pending.clear();
        commands.entity(entity).remove::<PendingPose>();
        for (node, rest) in &binding.nodes {
            if let Ok(mut t) = transforms.get_mut(*node) {
                if *t != *rest {
                    *t = *rest;
                }
            }
        }
        let valid = matches!(mode, "clip" | "script");
        updates.0.push((
            model.node_id,
            "pose-status".into(),
            if valid { mode } else { "error" }.into(),
        ));
        updates.0.push((
            model.node_id,
            "pose-error".into(),
            if valid {
                ""
            } else {
                "pose-source must be clip or script; native streams are not implemented"
            }
            .into(),
        ));
    }
}

/// Host ingress shared by JS and future native pose sources. Caller resolves and
/// authorizes its document-local handle first. Validate the whole batch before writes.
pub fn submit(world: &mut World, batch: PoseBatch) {
    let Some(entity) = world
        .get_resource::<EntityMap>()
        .and_then(|m| m.0.get(&(batch.node as u32)))
        .copied()
    else {
        return;
    };
    let generation = world.get::<ModelInstance>(entity).map(|m| m.generation);
    let result = if let Some(mut binding) = world.get_mut::<ModelPoseBinding>(entity) {
        if Some(binding.generation) != generation || binding.token != batch.binding {
            Err("Stale pose binding; read animation-joints again")
        } else if binding.mode != "script" {
            Err("setJointBatch requires pose-source=script")
        } else if batch.joints.iter().any(|j| j.index >= binding.nodes.len()) {
            Err("Joint index is outside this binding")
        } else {
            for joint in batch.joints {
                binding.pending.insert(joint.index, joint);
            }
            Ok(())
        }
    } else {
        Err("Model pose binding is not ready")
    };
    let error = result.as_ref().err().copied();
    let changed = if let Some(mut binding) = world.get_mut::<ModelPoseBinding>(entity) {
        let changed = binding.last_error != error;
        binding.last_error = error;
        changed
    } else {
        true
    };
    if changed {
        emit(world, batch.node as u32, "pose-error", error.unwrap_or(""));
    }
    match result {
        Ok(()) => {
            world.entity_mut(entity).insert(PendingPose);
        }
        Err(_) => {}
    }
}

fn apply_poses(
    mut commands: Commands,
    mut models: Query<(Entity, &ModelInstance, &mut ModelPoseBinding), With<PendingPose>>,
    mut transforms: Query<&mut Transform>,
) {
    for (entity, model, mut binding) in &mut models {
        commands.entity(entity).remove::<PendingPose>();
        if binding.generation != model.generation || binding.mode != "script" {
            binding.pending.clear();
            continue;
        }
        let ModelPoseBinding { nodes, pending, .. } = &mut *binding;
        for (index, pose) in pending.drain() {
            let (node, rest) = nodes[index];
            if let Ok(mut t) = transforms.get_mut(node) {
                let next = Transform {
                    translation: pose
                        .translation
                        .map(Vec3::from_array)
                        .unwrap_or(rest.translation),
                    rotation: Quat::from_array(pose.rotation),
                    scale: pose.scale.map(Vec3::from_array).unwrap_or(rest.scale),
                };
                if *t != next {
                    *t = next;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::ModelResource;

    fn fixture() -> (App, Entity, Entity, String) {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            TransformPlugin,
            HierarchyPlugin,
            ModelPosePlugin,
        ))
        .init_resource::<Assets<Scene>>()
        .init_resource::<AttributeUpdates>()
        .init_resource::<EntityMap>()
        .add_systems(Update, (sync_bindings, sync_pose_control).chain());
        let mut source = World::new();
        let sr = source.spawn(Transform::IDENTITY).id();
        let sp = source
            .spawn((Transform::from_xyz(1., 0., 0.), Name::new("duplicate")))
            .id();
        let sj = source
            .spawn((Transform::from_xyz(0., 2., 0.), Name::new("duplicate")))
            .id();
        let sm = source
            .spawn((
                Transform::IDENTITY,
                SkinnedMesh {
                    inverse_bindposes: default(),
                    joints: vec![sj, sp],
                },
            ))
            .id();
        source.entity_mut(sr).push_children(&[sp, sm]);
        source.entity_mut(sp).add_child(sj);
        let scene = app
            .world_mut()
            .resource_mut::<Assets<Scene>>()
            .add(Scene::new(source));
        let world = app.world_mut();
        let root = world.spawn(SpatialBundle::default()).id();
        let content = world.spawn(SpatialBundle::default()).id();
        let ar = world.spawn(SpatialBundle::default()).id();
        let ap = world.spawn(SpatialBundle::default()).id();
        // An evaluated clip has already changed this transform. Binding must
        // obtain reference TRS from the source, not the current instance.
        let aj = world
            .spawn(SpatialBundle::from_transform(Transform::from_xyz(
                99., 99., 99.,
            )))
            .id();
        let am = world.spawn(SpatialBundle::default()).id();
        world.entity_mut(root).add_child(content);
        world.entity_mut(content).add_child(ar);
        world.entity_mut(ar).push_children(&[ap, am]);
        world.entity_mut(ap).add_child(aj);
        world.resource_mut::<EntityMap>().0.insert(7, root);
        let config = ModelAnimationConfig::from_attrs(Some(&HashMap::from([(
            "pose-source".into(),
            "script".into(),
        )])));
        world.entity_mut(root).insert((
            config,
            PendingJointBinding,
            ModelInstance {
                node_id: 7,
                generation: 1,
                source: "fixture.glb".into(),
                content: Some(content),
                status: ModelStatus::Ready,
                resource: Some(ModelResource {
                    asset_path: "fixture.glb".into(),
                    scene,
                    gltf: None,
                    generated: None,
                }),
            },
        ));
        app.update();
        let token = app
            .world()
            .get::<ModelPoseBinding>(root)
            .unwrap()
            .token
            .clone();
        (app, root, aj, token)
    }

    fn batch(token: &str, x: f32) -> PoseBatch {
        PoseBatch {
            node: 7,
            binding: token.into(),
            joints: vec![JointPose {
                index: 2,
                rotation: [0., 0., 0., 1.],
                translation: Some([x, 3., 4.]),
                scale: Some([2., 2., 2.]),
            }],
        }
    }

    #[test]
    fn generated_pose_binding_needs_no_scene_and_uses_identity_rest_pose() {
        let mut world = World::new();
        let content = world.spawn(Transform::from_xyz(8., 9., 10.)).id();
        let model = ModelInstance {
            node_id: 7,
            source: "mesh://1/1".into(),
            generation: 1,
            content: Some(content),
            status: ModelStatus::Ready,
            resource: Some(crate::models::ModelResource {
                asset_path: "mesh://1/1".into(),
                scene: Handle::default(),
                gltf: None,
                generated: Some(crate::models::GeneratedModel {
                    mesh: Handle::default(),
                    material: Handle::default(),
                    aabb: default(),
                }),
            }),
        };
        let (binding, catalog) = build_binding(&world, &model).unwrap();
        assert_eq!(binding.nodes, vec![(content, Transform::IDENTITY)]);
        let catalog: serde_json::Value = serde_json::from_str(&catalog).unwrap();
        assert_eq!(catalog["nodes"].as_array().unwrap().len(), 1);
        assert_eq!(catalog["skins"], serde_json::json!([]));
        assert_eq!(
            catalog["nodes"][0]["translation"],
            serde_json::json!([0., 0., 0.])
        );
    }

    #[test]
    fn binding_preserves_intermediates_rest_and_skin_order() {
        let (app, _, joint, _) = fixture();
        assert_eq!(
            app.world().get::<Transform>(joint).unwrap().translation,
            Vec3::new(0., 2., 0.)
        );
        let updates = &app.world().resource::<AttributeUpdates>().0;
        let json = &updates
            .iter()
            .find(|(_, k, _)| k == "animation-joints")
            .unwrap()
            .2;
        let catalog: serde_json::Value = serde_json::from_str(json).unwrap();
        assert_eq!(catalog["nodes"][2]["parent"], 1);
        assert_eq!(catalog["nodes"][1]["name"], catalog["nodes"][2]["name"]);
        assert_eq!(catalog["skins"][0]["joints"], serde_json::json!([2, 1]));
    }

    #[test]
    fn sparse_poses_persist_coalesce_and_reject_stale_or_invalid_batches() {
        let (mut app, root, joint, token) = fixture();
        submit(app.world_mut(), batch(&token, 5.));
        submit(app.world_mut(), batch(&token, 6.));
        app.update();
        assert_eq!(
            app.world().get::<Transform>(joint).unwrap().translation.x,
            6.
        );
        for _ in 0..3 {
            app.update();
        }
        assert_eq!(
            app.world().get::<Transform>(joint).unwrap().translation.x,
            6.
        );
        assert!(app.world().get::<PendingPose>(root).is_none());
        let mut invalid = batch(&token, 42.);
        invalid.joints.push(JointPose {
            index: 999,
            ..invalid.joints[0].clone()
        });
        submit(app.world_mut(), invalid);
        submit(app.world_mut(), batch("stale", 100.));
        app.update();
        assert_eq!(
            app.world().get::<Transform>(joint).unwrap().translation.x,
            6.
        );
        let mut rotation = batch(&token, 0.);
        rotation.joints[0].translation = None;
        rotation.joints[0].scale = None;
        submit(app.world_mut(), rotation);
        app.update();
        assert_eq!(
            *app.world().get::<Transform>(joint).unwrap(),
            Transform::from_xyz(0., 2., 0.)
        );
        // The catalog token alone cannot survive a resource generation change.
        app.world_mut()
            .get_mut::<ModelInstance>(root)
            .unwrap()
            .generation += 1;
        submit(app.world_mut(), batch(&token, 100.));
        app.update();
        assert_eq!(
            app.world().get::<Transform>(joint).unwrap().translation.x,
            0.
        );
    }
}
