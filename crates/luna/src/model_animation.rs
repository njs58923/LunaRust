//! GLB clip controls: shared graphs, independent scene players, no JS frame loop.
use crate::{
    models::{ModelInstance, ModelStatus},
    AttributeUpdates,
};
use bevy::{
    animation::{graph::AnimationNodeIndex, RepeatAnimation},
    asset::AssetId,
    gltf::Gltf,
    prelude::*,
};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Component, Clone, Debug, PartialEq)]
pub struct ModelAnimationConfig {
    pub(crate) pose_source: String,
    clip: String,
    state: String,
    looping: bool,
    speed: f32,
    seek: Option<f32>,
    restart: String,
    error: Option<String>,
}

impl ModelAnimationConfig {
    pub fn from_attrs(attrs: Option<&HashMap<String, String>>) -> Self {
        let get = |key: &str| attrs.and_then(|a| a.get(key)).map(|s| s.trim());
        let mut error = None;
        let mut number = |key: &str, default: Option<f32>| match get(key) {
            None | Some("") => default,
            Some(value) => match value.parse::<f32>() {
                Ok(n) if n.is_finite() && n >= 0.0 => Some(n),
                _ => {
                    error = Some(format!("{key} must be a finite nonnegative number"));
                    default
                }
            },
        };
        let speed = number("animation-speed", Some(1.0)).unwrap();
        let seek = number("animation-time", None);
        let state = get("animation-state").unwrap_or("playing").to_string();
        if !matches!(state.as_str(), "playing" | "paused" | "stopped") {
            error = Some("animation-state must be playing, paused or stopped".into());
        }
        let looping = match get("animation-loop").unwrap_or("true") {
            "true" | "1" => true,
            "false" | "0" => false,
            _ => {
                error = Some("animation-loop must be true or false".into());
                true
            }
        };
        Self {
            pose_source: get("pose-source")
                .filter(|s| !s.is_empty())
                .unwrap_or("clip")
                .into(),
            clip: get("animation-clip").unwrap_or("").into(),
            state,
            looping,
            speed,
            seek,
            restart: get("animation-restart").unwrap_or("").into(),
            error,
        }
    }
}

#[derive(Component)]
pub struct PendingModelAnimation;

#[derive(Component)]
pub struct WaitingClipCompletion;

pub fn sync_config(
    commands: &mut Commands,
    entity: Entity,
    attrs: Option<&HashMap<String, String>>,
    previous: Option<&ModelAnimationConfig>,
) {
    let config = ModelAnimationConfig::from_attrs(attrs);
    if previous != Some(&config) {
        commands
            .entity(entity)
            .insert((config, PendingModelAnimation));
    }
}

#[derive(Clone)]
struct ClipGraph {
    handle: Handle<AnimationGraph>,
    metadata: Arc<ClipMetadata>,
}

struct ClipMetadata {
    nodes: Vec<AnimationNodeIndex>,
    names: HashMap<String, usize>,
    durations: Vec<f32>,
    catalog: String,
}

impl std::ops::Deref for ClipGraph {
    type Target = ClipMetadata;
    fn deref(&self) -> &Self::Target {
        &self.metadata
    }
}

#[derive(Component, Clone)]
pub struct ModelPlayback {
    generation: u64,
    graph: ClipGraph,
    players: Vec<Entity>,
    applied: Option<ModelAnimationConfig>,
    selected: Option<usize>,
}

/// Cache contains weak graph handles, so it does not pin unloaded models.
#[derive(Default)]
pub struct ClipGraphCache(HashMap<AssetId<Gltf>, ClipGraph>);

fn clip_graph(
    gltf: &Gltf,
    clips: &Assets<AnimationClip>,
    graphs: &mut Assets<AnimationGraph>,
) -> ClipGraph {
    let mut graph = AnimationGraph::new();
    let root = graph.root;
    let nodes = graph
        .add_clips(gltf.animations.iter().cloned(), 1.0, root)
        .collect();
    let names: HashMap<String, usize> = gltf
        .named_animations
        .iter()
        .filter_map(|(name, handle)| {
            gltf.animations
                .iter()
                .position(|a| a == handle)
                .map(|i| (name.to_string(), i))
        })
        .collect();
    let durations: Vec<_> = gltf
        .animations
        .iter()
        .map(|a| clips.get(a).map_or(0.0, AnimationClip::duration))
        .collect();
    let catalog: Vec<_> = durations
        .iter()
        .enumerate()
        .map(|(index, duration)| {
            let mut aliases: Vec<_> = names
                .iter()
                .filter(|(_, i)| **i == index)
                .map(|(n, _)| n.as_str())
                .collect();
            aliases.sort_unstable();
            serde_json::json!({"index": index, "names": aliases, "duration": duration})
        })
        .collect();
    ClipGraph {
        handle: graphs.add(graph),
        metadata: Arc::new(ClipMetadata {
            nodes,
            names,
            durations,
            catalog: serde_json::to_string(&catalog).unwrap(),
        }),
    }
}

fn select_clip(graph: &ClipGraph, value: &str) -> Result<Option<usize>, String> {
    if value.is_empty() {
        return Ok(None);
    }
    graph
        .names
        .get(value)
        .copied()
        .or_else(|| {
            value
                .parse::<usize>()
                .ok()
                .filter(|i| *i < graph.nodes.len())
        })
        .map(Some)
        .ok_or_else(|| format!("Unknown animation clip: {value}"))
}

fn apply_control(
    player: &mut AnimationPlayer,
    playback: &ModelPlayback,
    config: &ModelAnimationConfig,
    selected: Option<usize>,
) {
    let Some(index) = selected else {
        player.stop_all();
        return;
    };
    if config.state == "stopped" {
        player.stop_all();
        player.start(playback.graph.nodes[index]).pause();
        return;
    }
    let previous = playback.applied.as_ref();
    let restart = previous
        .is_none_or(|old| old.state == "stopped" || old.restart != config.restart)
        || playback.selected != selected;
    let node = playback.graph.nodes[index];
    if restart {
        player.stop_all();
        player.start(node);
    }
    let active = player.play(node);
    active
        .set_speed(config.speed)
        .set_repeat(if config.looping {
            RepeatAnimation::Forever
        } else {
            RepeatAnimation::Never
        });
    if restart || previous.is_none_or(|old| old.seek != config.seek) {
        if let Some(time) = config.seek {
            active.replay();
            active.seek_to(time.min(playback.graph.durations[index]));
        }
    }
    if config.state == "paused" || playback.graph.durations[index] <= 0.0 {
        active.pause();
    } else {
        active.resume();
    }
}

/// Scene traversal happens only on a new instance generation. Control edits use
/// cached player IDs; animation frames are evaluated by Bevy, not the DOM.
pub fn sync_model_animations(
    mut commands: Commands,
    pending: Query<
        (
            Entity,
            &ModelInstance,
            &ModelAnimationConfig,
            Option<&ModelPlayback>,
        ),
        With<PendingModelAnimation>,
    >,
    gltfs: Res<Assets<Gltf>>,
    clips: Res<Assets<AnimationClip>>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
    children: Query<&Children>,
    mut players: Query<&mut AnimationPlayer>,
    mut cache: Local<ClipGraphCache>,
    mut updates: ResMut<AttributeUpdates>,
) {
    for (entity, instance, config, old) in &pending {
        if instance.status == ModelStatus::Loading {
            continue;
        }
        let mut emit =
            |key: &str, value: String| updates.0.push((instance.node_id, key.into(), value));
        if instance.status != ModelStatus::Ready {
            commands
                .entity(entity)
                .remove::<ModelPlayback>()
                .remove::<PendingModelAnimation>();
            emit("animation-clips", "[]".into());
            emit("animation-error", String::new());
            emit(
                "animation-status",
                if matches!(instance.status, ModelStatus::Error(_)) {
                    "error"
                } else {
                    "idle"
                }
                .into(),
            );
            continue;
        }
        let mut playback = if let Some(old) = old.filter(|p| p.generation == instance.generation) {
            old.clone()
        } else {
            let Some(gltf_handle) = instance.resource.as_ref().and_then(|r| r.gltf.as_ref()) else {
                emit("animation-clips", "[]".into());
                emit(
                    "animation-status",
                    if config.clip.is_empty() {
                        "idle"
                    } else {
                        "error"
                    }
                    .into(),
                );
                emit(
                    "animation-error",
                    if config.clip.is_empty() {
                        String::new()
                    } else {
                        "Model has no GLB/glTF clips".into()
                    },
                );
                commands
                    .entity(entity)
                    .remove::<PendingModelAnimation>()
                    .remove::<ModelPlayback>();
                continue;
            };
            let Some(gltf) = gltfs.get(gltf_handle) else {
                continue;
            };
            if gltf.animations.iter().any(|a| !clips.contains(a.id())) {
                continue;
            }
            let graph = if let Some((cached, handle)) = cache
                .0
                .get(&gltf_handle.id())
                .and_then(|g| graphs.get_strong_handle(g.handle.id()).map(|h| (g, h)))
            {
                let mut graph = cached.clone();
                graph.handle = handle;
                graph
            } else {
                cache.0.retain(|_, g| graphs.contains(g.handle.id()));
                let graph = clip_graph(gltf, &clips, &mut graphs);
                let mut weak = graph.clone();
                weak.handle = graph.handle.clone_weak();
                cache.0.insert(gltf_handle.id(), weak);
                graph
            };
            let mut found = Vec::new();
            let mut stack: Vec<_> = instance.content.into_iter().collect();
            while let Some(child) = stack.pop() {
                if players.contains(child) {
                    found.push(child);
                    commands.entity(child).insert(graph.handle.clone());
                }
                if let Ok(descendants) = children.get(child) {
                    stack.extend(descendants.iter().copied());
                }
            }
            emit("animation-clips", graph.catalog.clone());
            ModelPlayback {
                generation: instance.generation,
                graph,
                players: found,
                applied: None,
                selected: None,
            }
        };
        if config.pose_source != "clip" {
            // Removing the graph prevents paused clips and transitions from writing
            // transforms. Keep player state so returning to clips is explicit.
            for id in &playback.players {
                commands.entity(*id).remove::<Handle<AnimationGraph>>();
            }
            playback.applied = None;
            commands
                .entity(entity)
                .insert(playback)
                .remove::<PendingModelAnimation>()
                .remove::<WaitingClipCompletion>();
            emit("animation-status", "idle".into());
            continue;
        }
        for id in &playback.players {
            commands.entity(*id).insert(playback.graph.handle.clone());
        }
        let selection = config
            .error
            .clone()
            .map_or_else(|| select_clip(&playback.graph, &config.clip), Err)
            .and_then(|selected| {
                if selected.is_some() && playback.players.is_empty() {
                    Err("Selected scene has no animation player".into())
                } else {
                    Ok(selected)
                }
            });
        match selection {
            Ok(selected) => {
                for id in &playback.players {
                    if let Ok(mut player) = players.get_mut(*id) {
                        apply_control(&mut player, &playback, config, selected);
                    }
                }
                playback.selected = selected;
                playback.applied = Some(config.clone());
                emit("animation-error", String::new());
                emit(
                    "animation-status",
                    if selected.is_none() {
                        "idle"
                    } else {
                        config.state.as_str()
                    }
                    .into(),
                );
                if selected.is_some() && config.state == "playing" && !config.looping {
                    commands.entity(entity).insert(WaitingClipCompletion);
                } else {
                    commands.entity(entity).remove::<WaitingClipCompletion>();
                }
            }
            Err(error) => {
                emit("animation-error", error);
                emit("animation-status", "error".into());
                commands.entity(entity).remove::<WaitingClipCompletion>();
            }
        }
        commands
            .entity(entity)
            .insert(playback)
            .remove::<PendingModelAnimation>();
    }
}

/// Only finite clips awaiting completion are checked; no per-frame DOM writes.
pub fn report_clip_completion(
    mut commands: Commands,
    instances: Query<(Entity, &ModelInstance, &ModelPlayback), With<WaitingClipCompletion>>,
    players: Query<&AnimationPlayer>,
    mut updates: ResMut<AttributeUpdates>,
) {
    for (entity, instance, playback) in &instances {
        if playback.generation == instance.generation
            && (playback
                .selected
                .is_some_and(|i| playback.graph.durations[i] <= 0.0)
                || playback
                    .players
                    .iter()
                    .all(|id| players.get(*id).is_ok_and(AnimationPlayer::all_finished)))
        {
            updates.0.push((
                instance.node_id,
                "animation-status".into(),
                "finished".into(),
            ));
            commands.entity(entity).remove::<WaitingClipCompletion>();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::{
        animation::AnimationPlugin, ecs::world::CommandQueue, gltf::GltfPlugin, scene::ScenePlugin,
        time::TimeUpdateStrategy,
    };

    fn config(pairs: &[(&str, &str)]) -> ModelAnimationConfig {
        ModelAnimationConfig::from_attrs(Some(
            &pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        ))
    }

    #[test]
    fn rejects_invalid_controls_without_nan_changes() {
        for (key, value) in [
            ("animation-speed", "NaN"),
            ("animation-speed", "-1"),
            ("animation-time", "inf"),
            ("animation-loop", "maybe"),
            ("animation-state", "invalid"),
        ] {
            let a = config(&[(key, value)]);
            assert!(a.error.is_some());
            assert_eq!(a, config(&[(key, value)]));
        }
    }

    // A self-contained GLB, with one translation track and no renderer dependency.
    // Exercises the real GltfLoader, SceneSpawner and AnimationPlugin together.
    fn glb() -> Vec<u8> {
        let mut json = serde_json::to_vec(&serde_json::json!({
            "asset": {"version":"2.0"}, "scene":0, "scenes":[{"nodes":[0]}],
            "nodes":[{"name":"Wing"}], "buffers":[{"byteLength":32}],
            "bufferViews":[{"buffer":0,"byteOffset":0,"byteLength":8},
                {"buffer":0,"byteOffset":8,"byteLength":24}],
            "accessors":[{"bufferView":0,"componentType":5126,"count":2,"type":"SCALAR","min":[0],"max":[1]},
                {"bufferView":1,"componentType":5126,"count":2,"type":"VEC3"}],
            "animations":[{"name":"Fly","samplers":[{"input":0,"output":1,"interpolation":"LINEAR"}],
                "channels":[{"sampler":0,"target":{"node":0,"path":"translation"}}]}]
        })).unwrap();
        while json.len() % 4 != 0 {
            json.push(b' ');
        }
        let mut bytes = Vec::new();
        for v in [
            0x46546c67u32,
            2,
            (12 + 8 + json.len() + 8 + 32) as u32,
            json.len() as u32,
            0x4e4f534a,
        ] {
            bytes.extend(v.to_le_bytes());
        }
        bytes.extend(json);
        bytes.extend(32u32.to_le_bytes());
        bytes.extend(0x004e4942u32.to_le_bytes());
        for v in [0.0f32, 1.0, 0.0, 0.0, 0.0, 2.0, 0.0, 0.0] {
            bytes.extend(v.to_le_bytes());
        }
        bytes
    }

    fn mount(app: &mut App, root: Entity, path: &str, cfg: ModelAnimationConfig) {
        let old = app.world().get::<ModelInstance>(root).cloned();
        let server = app.world().resource::<AssetServer>().clone();
        let mut queue = CommandQueue::default();
        let mut updates = AttributeUpdates::default();
        crate::models::set_source(
            &mut Commands::new(&mut queue, app.world()),
            &server,
            root,
            root.index(),
            path,
            Some(path),
            None,
            old.as_ref(),
            &mut updates,
        );
        queue.apply(app.world_mut());
        app.world_mut()
            .entity_mut(root)
            .insert((cfg, PendingModelAnimation));
    }

    fn update_control(app: &mut App, root: Entity, cfg: ModelAnimationConfig) {
        app.world_mut()
            .entity_mut(root)
            .insert((cfg, PendingModelAnimation));
        app.update();
    }

    #[test]
    fn real_glb_instances_play_independently_and_rebind_after_source_change() {
        let dir = std::env::temp_dir().join(format!(
            "luna-clips-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("wing.glb"), glb()).unwrap();
        std::fs::write(dir.join("replacement.glb"), glb()).unwrap();
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            AssetPlugin {
                file_path: dir.to_string_lossy().into(),
                ..default()
            },
            TransformPlugin,
            HierarchyPlugin,
            ScenePlugin,
            AnimationPlugin,
            GltfPlugin::default(),
            crate::model_pose::ModelPosePlugin,
        ))
        .init_asset::<Mesh>()
        .init_asset::<StandardMaterial>()
        .init_asset::<Image>()
        .register_type::<Name>()
        .register_type::<Visibility>()
        .register_type::<InheritedVisibility>()
        .register_type::<ViewVisibility>()
        .init_resource::<AttributeUpdates>()
        .init_resource::<crate::EntityMap>()
        .insert_resource(TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_millis(100),
        ))
        .add_systems(
            Update,
            (
                crate::models::poll_model_instances,
                crate::model_pose::sync_bindings,
                crate::model_pose::sync_pose_control,
                sync_model_animations,
                report_clip_completion,
            )
                .chain(),
        );
        app.finish();
        app.cleanup();
        let a = app.world_mut().spawn(SpatialBundle::default()).id();
        let b = app.world_mut().spawn(SpatialBundle::default()).id();
        mount(
            &mut app,
            a,
            "wing.glb",
            config(&[("animation-clip", "Fly")]),
        );
        mount(
            &mut app,
            b,
            "wing.glb",
            config(&[
                ("animation-clip", "0"),
                ("animation-state", "paused"),
                ("animation-time", "0.25"),
            ]),
        );
        let wait = |app: &mut App, root| {
            let start = std::time::Instant::now();
            while app.world().get::<ModelPlayback>(root).is_none() {
                assert!(
                    start.elapsed().as_secs() < 10,
                    "scene or animation binding timed out"
                );
                app.update();
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
        };
        wait(&mut app, a);
        wait(&mut app, b);
        let pa = app.world().get::<ModelPlayback>(a).unwrap().clone();
        let pb = app.world().get::<ModelPlayback>(b).unwrap().clone();
        assert_eq!(pa.graph.handle.id(), pb.graph.handle.id());
        assert!(Arc::ptr_eq(&pa.graph.metadata, &pb.graph.metadata));
        assert_ne!(pa.players, pb.players);
        assert!(app.world().get::<PendingModelAnimation>(a).is_none());
        assert_eq!(select_clip(&pa.graph, "Fly"), Ok(Some(0)));
        assert!(select_clip(&pa.graph, "99").is_err());
        assert!(pa.graph.catalog.contains("Fly"));
        let wing_b = app
            .world_mut()
            .query::<(Entity, &Name)>()
            .iter(app.world())
            .find(|(e, n)| {
                n.as_str() == "Wing"
                    && app
                        .world()
                        .get::<bevy::animation::AnimationTarget>(*e)
                        .is_some_and(|t| pb.players.contains(&t.player))
            })
            .unwrap()
            .0;
        for _ in 0..3 {
            app.update();
        }
        assert!((app.world().get::<Transform>(wing_b).unwrap().translation.x - 0.5).abs() < 0.001);
        let player_a = app.world().get::<AnimationPlayer>(pa.players[0]).unwrap();
        assert!(player_a.playing_animations().next().unwrap().1.seek_time() > 0.0);
        update_control(
            &mut app,
            b,
            config(&[
                ("animation-clip", "Fly"),
                ("animation-state", "paused"),
                ("animation-time", "0.75"),
            ]),
        );
        assert!((app.world().get::<Transform>(wing_b).unwrap().translation.x - 1.5).abs() < 0.001);
        update_control(
            &mut app,
            b,
            config(&[("animation-clip", "Fly"), ("animation-state", "stopped")]),
        );
        assert!(
            app.world()
                .get::<Transform>(wing_b)
                .unwrap()
                .translation
                .x
                .abs()
                < 0.001
        );
        update_control(
            &mut app,
            b,
            config(&[
                ("animation-clip", "Fly"),
                ("animation-speed", "2"),
                ("animation-loop", "false"),
            ]),
        );
        for _ in 0..12 {
            app.update();
        }
        assert!(app
            .world()
            .get::<AnimationPlayer>(pb.players[0])
            .unwrap()
            .all_finished());
        assert!(!app
            .world()
            .get::<AnimationPlayer>(pa.players[0])
            .unwrap()
            .all_finished());
        assert!(app.world().get::<WaitingClipCompletion>(b).is_none());
        assert!(app
            .world()
            .resource::<AttributeUpdates>()
            .0
            .iter()
            .any(|(id, k, v)| *id == b.index() && k == "animation-status" && v == "finished"));
        update_control(
            &mut app,
            b,
            config(&[
                ("animation-clip", "Fly"),
                ("animation-restart", "1"),
                ("animation-loop", "false"),
            ]),
        );
        assert!(!app
            .world()
            .get::<AnimationPlayer>(pb.players[0])
            .unwrap()
            .all_finished());
        update_control(
            &mut app,
            b,
            config(&[
                ("animation-clip", "Fly"),
                ("animation-time", "0.5"),
                ("animation-state", "paused"),
            ]),
        );
        assert!((app.world().get::<Transform>(wing_b).unwrap().translation.x - 1.0).abs() < 0.001);
        // Real GLB: a paused player must not overwrite script poses on later frames.
        update_control(
            &mut app,
            b,
            config(&[
                ("animation-clip", "Fly"),
                ("animation-state", "paused"),
                ("pose-source", "script"),
            ]),
        );
        let catalog: serde_json::Value = serde_json::from_str(
            &app.world()
                .resource::<AttributeUpdates>()
                .0
                .iter()
                .rev()
                .find(|(id, k, _)| *id == b.index() && k == "animation-joints")
                .unwrap()
                .2,
        )
        .unwrap();
        let joint_index = catalog["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|n| n["name"] == "Wing")
            .unwrap()["index"]
            .as_u64()
            .unwrap() as usize;
        app.world_mut()
            .resource_mut::<crate::EntityMap>()
            .0
            .insert(b.index(), b);
        crate::model_pose::submit(
            app.world_mut(),
            js_runtime::pose::PoseBatch {
                node: b.index() as i32,
                binding: catalog["binding"].as_str().unwrap().into(),
                joints: vec![js_runtime::pose::JointPose {
                    index: joint_index,
                    rotation: [0., 0., 0., 1.],
                    translation: Some([9., 0., 0.]),
                    scale: None,
                }],
            },
        );
        for _ in 0..3 {
            app.update();
        }
        assert_eq!(
            app.world().get::<Transform>(wing_b).unwrap().translation.x,
            9.
        );
        assert!(
            (app.world()
                .get::<GlobalTransform>(wing_b)
                .unwrap()
                .translation()
                .x
                - 9.)
                .abs()
                < 0.001
        );
        assert!(app
            .world()
            .get::<Handle<AnimationGraph>>(pb.players[0])
            .is_none());
        update_control(
            &mut app,
            b,
            config(&[
                ("animation-clip", "Fly"),
                ("animation-state", "paused"),
                ("animation-time", "0.25"),
            ]),
        );
        assert!((app.world().get::<Transform>(wing_b).unwrap().translation.x - 0.5).abs() < 0.001);
        // Bad selections leave the previous playback intact and report a useful error.
        update_control(&mut app, b, config(&[("animation-clip", "Missing")]));
        assert!(app
            .world()
            .resource::<AttributeUpdates>()
            .0
            .iter()
            .any(|(_, k, v)| k == "animation-error" && v.contains("Missing")));
        mount(
            &mut app,
            b,
            "replacement.glb",
            config(&[("animation-clip", "Fly")]),
        );
        assert!(app.world().get_entity(wing_b).is_none());
        wait(&mut app, b);
        assert_ne!(
            app.world().get::<ModelPlayback>(b).unwrap().generation,
            pb.generation
        );
        assert!(app.world().get_entity(pa.players[0]).is_some());
        app.world_mut().entity_mut(b).despawn_recursive();
        assert!(app.world().get_entity(a).is_some());
        drop(app);
        // Remove only the two files created by this test, then its empty directory.
        std::fs::remove_file(dir.join("wing.glb")).unwrap();
        std::fs::remove_file(dir.join("replacement.glb")).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }
}
