//! Stable document instances backed by shared Bevy assets.
use crate::AttributeUpdates;
use bevy::{
    asset::LoadState,
    gltf::Gltf,
    prelude::*,
    scene::{SceneInstance, SceneSpawner},
};

/// Handles share the asset data; animation/visibility/pose belong to instances.
#[derive(Clone, Debug)]
pub struct ModelResource {
    pub asset_path: String,
    pub scene: Handle<Scene>,
    pub gltf: Option<Handle<Gltf>>,
}
impl ModelResource {
    fn load(server: &AssetServer, path: &str) -> Self {
        let gltf = path.ends_with(".glb") || path.ends_with(".gltf");
        Self {
            asset_path: path.into(),
            scene: server.load(if gltf {
                format!("{path}#Scene0")
            } else {
                path.into()
            }),
            gltf: gltf.then(|| server.load(path.to_string())),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModelStatus {
    Empty,
    Loading,
    Ready,
    Error(String),
}

#[derive(Component, Clone, Debug)]
pub struct ModelInstance {
    pub node_id: u32,
    pub source: String,
    pub generation: u64,
    pub resource: Option<ModelResource>,
    /// Only this child is replaced. User-authored children remain untouched.
    pub content: Option<Entity>,
    pub status: ModelStatus,
}
#[derive(Component)]
pub struct PendingModelInstance;
#[derive(Component)]
pub struct ModelContent {
    pub owner: Entity,
    pub generation: u64,
}

fn publish(instance: &ModelInstance, updates: &mut AttributeUpdates) {
    let (state, error) = match &instance.status {
        ModelStatus::Empty => ("empty", ""),
        ModelStatus::Loading => ("loading", ""),
        ModelStatus::Ready => ("ready", ""),
        ModelStatus::Error(error) => ("error", error.as_str()),
    };
    for (key, value) in [
        ("model-state", state),
        ("model-error", error),
        ("model-source", instance.source.as_str()),
    ] {
        updates.0.push((instance.node_id, key.into(), value.into()));
    }
}

pub fn set_source(
    commands: &mut Commands,
    server: &AssetServer,
    entity: Entity,
    node_id: u32,
    source: &str,
    path: Option<&str>,
    error: Option<&str>,
    previous: Option<&ModelInstance>,
    updates: &mut AttributeUpdates,
) {
    if let Some(old) = previous {
        if old.source == source
            && old.resource.as_ref().map(|r| r.asset_path.as_str()) == path
            && error.is_none_or(|e| old.status == ModelStatus::Error(e.into()))
        {
            return;
        }
        if let Some(content) = old.content {
            commands.entity(content).despawn_recursive();
        }
    }
    let generation = previous.map_or(1, |old| old.generation.wrapping_add(1));
    let resource = path.map(|p| ModelResource::load(server, p));
    let content = resource.as_ref().map(|resource| {
        let child = commands
            .spawn((
                SceneBundle {
                    scene: resource.scene.clone(),
                    ..default()
                },
                ModelContent {
                    owner: entity,
                    generation,
                },
            ))
            .id();
        commands.entity(entity).add_child(child);
        child
    });
    let status = if source.trim().is_empty() {
        ModelStatus::Empty
    } else if let Some(error) = error {
        ModelStatus::Error(error.into())
    } else {
        ModelStatus::Loading
    };
    let instance = ModelInstance {
        node_id,
        source: source.into(),
        generation,
        resource,
        content,
        status,
    };
    publish(&instance, updates);
    // A replacement scene must bind its own players, even when controls did not change.
    commands
        .entity(entity)
        .remove::<crate::model_animation::ModelPlayback>()
        .remove::<crate::model_animation::WaitingClipCompletion>()
        .insert(crate::model_animation::PendingModelAnimation);
    for (key, value) in [("animation-clips", "[]"), ("animation-error", "")] {
        updates.0.push((node_id, key.into(), value.into()));
    }
    updates.0.push((
        node_id,
        "animation-status".into(),
        if source.trim().is_empty() {
            "idle"
        } else {
            "loading"
        }
        .into(),
    ));
    if content.is_some() {
        commands.entity(entity).insert(PendingModelInstance);
    } else {
        commands.entity(entity).remove::<PendingModelInstance>();
    }
    commands.entity(entity).insert(instance);
}

/// Only unfinished scene instances are visited. Transform animation never queues
/// resource work and completed instances leave this query.
pub fn poll_model_instances(
    mut commands: Commands,
    server: Res<AssetServer>,
    spawner: Res<SceneSpawner>,
    mut pending: Query<(Entity, &mut ModelInstance), With<PendingModelInstance>>,
    scenes: Query<(&SceneInstance, &ModelContent)>,
    mut updates: ResMut<AttributeUpdates>,
) {
    for (entity, mut instance) in &mut pending {
        let Some(resource) = &instance.resource else {
            continue;
        };
        let failure = match server.get_load_state(resource.scene.id()) {
            Some(LoadState::Failed(error)) => Some(error.to_string()),
            _ => resource
                .gltf
                .as_ref()
                .and_then(|gltf| match server.get_load_state(gltf.id()) {
                    Some(LoadState::Failed(error)) => Some(error.to_string()),
                    _ => None,
                }),
        };
        let status = if let Some(error) = failure {
            Some(ModelStatus::Error(error.to_string()))
        } else if let Some((scene, content)) = instance.content.and_then(|id| scenes.get(id).ok()) {
            (content.owner == entity
                && content.generation == instance.generation
                && spawner.instance_is_ready(**scene))
            .then_some(ModelStatus::Ready)
        } else {
            None
        };
        if let Some(status) = status {
            instance.status = status;
            publish(&instance, &mut updates);
            commands.entity(entity).remove::<PendingModelInstance>();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::{ecs::world::CommandQueue, scene::ScenePlugin};
    fn app() -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default(), ScenePlugin))
            .init_asset::<Gltf>()
            .init_resource::<AttributeUpdates>()
            .add_systems(Update, poll_model_instances);
        app
    }
    fn assign(app: &mut App, root: Entity, src: &str, path: Option<&str>, error: Option<&str>) {
        let old = app.world().get::<ModelInstance>(root).cloned();
        let server = app.world().resource::<AssetServer>().clone();
        let mut queue = CommandQueue::default();
        let mut updates = AttributeUpdates::default();
        set_source(
            &mut Commands::new(&mut queue, app.world()),
            &server,
            root,
            7,
            src,
            path,
            error,
            old.as_ref(),
            &mut updates,
        );
        queue.apply(app.world_mut());
        app.world_mut()
            .resource_mut::<AttributeUpdates>()
            .0
            .extend(updates.0);
    }
    #[test]
    fn resource_changes_preserve_instance_pose_and_authored_children() {
        let mut app = app();
        let pose = Transform::from_xyz(3.0, 2.0, -4.0);
        let root = app
            .world_mut()
            .spawn(SpatialBundle {
                transform: pose,
                ..default()
            })
            .id();
        let authored = app.world_mut().spawn(SpatialBundle::default()).id();
        app.world_mut().entity_mut(root).add_child(authored);
        assign(&mut app, root, "a.glb", None, None);
        assert!(app.world().get::<PendingModelInstance>(root).is_none());
        assign(&mut app, root, "a.glb", Some("a.glb"), None);
        let first = app.world().get::<ModelInstance>(root).unwrap().clone();
        app.world_mut().resource_mut::<AttributeUpdates>().0.clear();
        assign(&mut app, root, "a.glb", Some("a.glb"), None);
        assert_eq!(
            app.world().get::<ModelInstance>(root).unwrap().generation,
            first.generation
        );
        assert!(app.world().resource::<AttributeUpdates>().0.is_empty());
        assign(&mut app, root, "b.glb", None, None);
        assert!(app.world().get_entity(first.content.unwrap()).is_none());
        assert_eq!(*app.world().get::<Transform>(root).unwrap(), pose);
        assert_eq!(app.world().get::<Parent>(authored).unwrap().get(), root);
        assign(&mut app, root, "b.glb", None, Some("download failed"));
        assert_eq!(
            app.world().get::<ModelInstance>(root).unwrap().status,
            ModelStatus::Error("download failed".into())
        );
        assign(&mut app, root, "", None, None);
        assert_eq!(
            app.world().get::<ModelInstance>(root).unwrap().status,
            ModelStatus::Empty
        );
        assert!(app.world().get_entity(authored).is_some());
        assign(&mut app, root, "   ", None, None);
        assert_eq!(
            app.world().get::<ModelInstance>(root).unwrap().status,
            ModelStatus::Empty
        );
    }
    #[test]
    fn scene_assets_are_shared_but_content_instances_are_distinct() {
        let mut app = app();
        let a = app.world_mut().spawn(SpatialBundle::default()).id();
        let b = app.world_mut().spawn(SpatialBundle::default()).id();
        assign(&mut app, a, "same.glb", Some("same.glb"), None);
        assign(&mut app, b, "same.glb", Some("same.glb"), None);
        let a = app.world().get::<ModelInstance>(a).unwrap();
        let b = app.world().get::<ModelInstance>(b).unwrap();
        assert_eq!(
            a.resource.as_ref().unwrap().scene.id(),
            b.resource.as_ref().unwrap().scene.id()
        );
        assert_eq!(
            a.resource.as_ref().unwrap().gltf,
            b.resource.as_ref().unwrap().gltf
        );
        assert_ne!(a.content, b.content);
        assert_eq!(a.status, ModelStatus::Loading);
    }
    #[test]
    fn ready_requires_current_scene_instance_and_leaves_pending_query() {
        let mut app = app();
        let root = app.world_mut().spawn(SpatialBundle::default()).id();
        assign(&mut app, root, "test.scene", Some("test.scene"), None);
        let scene = app
            .world_mut()
            .resource_mut::<Assets<Scene>>()
            .add(Scene::new(World::new()));
        let child = app
            .world()
            .get::<ModelInstance>(root)
            .unwrap()
            .content
            .unwrap();
        app.world_mut().entity_mut(child).insert(scene.clone());
        app.world_mut()
            .get_mut::<ModelInstance>(root)
            .unwrap()
            .resource
            .as_mut()
            .unwrap()
            .scene = scene;
        app.world_mut()
            .get_mut::<ModelContent>(child)
            .unwrap()
            .generation = 0;
        app.update();
        app.update();
        assert_eq!(
            app.world().get::<ModelInstance>(root).unwrap().status,
            ModelStatus::Loading
        );
        let generation = app.world().get::<ModelInstance>(root).unwrap().generation;
        app.world_mut()
            .get_mut::<ModelContent>(child)
            .unwrap()
            .generation = generation;
        app.update();
        assert_eq!(
            app.world().get::<ModelInstance>(root).unwrap().status,
            ModelStatus::Ready
        );
        assert!(app.world().get::<PendingModelInstance>(root).is_none());
        app.world_mut().entity_mut(root).despawn_recursive();
        assert!(app.world().get_entity(child).is_none());
    }
}
