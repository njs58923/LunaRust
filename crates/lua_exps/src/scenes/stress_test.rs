use bevy::prelude::*;
use std::time::Instant;

use super::lua_rock::{build_mesh_for_seed, cache_key, ROCK_SCRIPT, RockCache};
use super::SceneRoot;
use crate::LuaVm;

pub const MAX_LEVEL: u32 = 4;

#[derive(Resource)]
pub struct StressState {
    pub level: u32,
    pub dirty: bool,
    pub last_count: usize,
    pub last_gen_ms: f32,
    pub last_spawn_ms: f32,
    pub last_hits: u32,
    pub last_misses: u32,
}

impl Default for StressState {
    fn default() -> Self {
        Self {
            level: 0,
            dirty: true,
            last_count: 0,
            last_gen_ms: 0.0,
            last_spawn_ms: 0.0,
            last_hits: 0,
            last_misses: 0,
        }
    }
}

#[derive(Resource)]
pub struct StressMaterial(pub Handle<StandardMaterial>);

#[derive(Component)]
pub struct StressInstance;

#[derive(Component)]
pub struct StressCamera;

#[derive(Component)]
pub struct StressHud;

pub fn setup(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut state: ResMut<StressState>,
) {
    state.dirty = true;

    let tex = asset_server.load("textura_roca.png");
    let mat = materials.add(StandardMaterial {
        base_color_texture: Some(tex),
        perceptual_roughness: 0.9,
        ..default()
    });
    commands.insert_resource(StressMaterial(mat));

    commands
        .spawn((SceneRoot, SpatialBundle::default(), Name::new("StressScene")))
        .with_children(|root| {
            root.spawn(DirectionalLightBundle {
                directional_light: DirectionalLight {
                    illuminance: 12_000.0,
                    shadows_enabled: false,
                    ..default()
                },
                transform: Transform::from_xyz(6.0, 12.0, 6.0)
                    .looking_at(Vec3::ZERO, Vec3::Y),
                ..default()
            });
            root.spawn((
                Camera3dBundle {
                    transform: Transform::from_xyz(0.0, 4.0, 6.0)
                        .looking_at(Vec3::ZERO, Vec3::Y),
                    ..default()
                },
                StressCamera,
            ));
        });

    commands.spawn((
        TextBundle::from_section(
            "",
            TextStyle {
                font_size: 18.0,
                color: Color::srgb(0.6, 1.0, 0.6),
                ..default()
            },
        )
        .with_style(Style {
            position_type: PositionType::Absolute,
            bottom: Val::Px(8.0),
            left: Val::Px(8.0),
            ..default()
        }),
        StressHud,
        SceneRoot,
    ));
}

pub fn input(
    keys: Res<ButtonInput<KeyCode>>,
    mut state: ResMut<StressState>,
    mut cache: ResMut<RockCache>,
) {
    if keys.just_pressed(KeyCode::ArrowUp) && state.level < MAX_LEVEL {
        state.level += 1;
        state.dirty = true;
    }
    if keys.just_pressed(KeyCode::ArrowDown) && state.level > 0 {
        state.level -= 1;
        state.dirty = true;
    }
    if keys.just_pressed(KeyCode::KeyC) {
        cache.clear();
        state.dirty = true;
        info!("rock cache cleared");
    }
}

pub fn regenerate(
    mut state: ResMut<StressState>,
    lua: Res<LuaVm>,
    mut cache: ResMut<RockCache>,
    mut meshes: ResMut<Assets<Mesh>>,
    material: Option<Res<StressMaterial>>,
    mut commands: Commands,
    q_old: Query<Entity, With<StressInstance>>,
    q_root: Query<Entity, (With<SceneRoot>, With<Name>)>,
    mut q_cam: Query<&mut Transform, With<StressCamera>>,
) {
    if !state.dirty {
        return;
    }
    let Some(material) = material else { return };

    cache.ensure_loaded(&lua.0);

    for e in &q_old {
        commands.entity(e).despawn_recursive();
    }

    let count = 10_usize.pow(state.level);
    let hits_before = cache.hits;
    let misses_before = cache.misses;

    let gen_start = Instant::now();
    let mut handles: Vec<Handle<Mesh>> = Vec::with_capacity(count);
    for i in 0..count {
        let seed = (state.level as i64) * 1_000_000 + i as i64;
        let key = cache_key(seed, ROCK_SCRIPT);
        let h = if let Some(h) = cache.get(&key) {
            cache.hits += 1;
            h
        } else {
            match build_mesh_for_seed(&lua.0, seed) {
                Ok(mesh) => {
                    let h = meshes.add(mesh);
                    cache.insert(key, h.clone());
                    cache.misses += 1;
                    h
                }
                Err(e) => {
                    error!("rock build failed seed={seed}: {e}");
                    state.dirty = false;
                    return;
                }
            }
        };
        handles.push(h);
    }
    let gen_ms = gen_start.elapsed().as_secs_f64() as f32 * 1000.0;

    let side = (count as f32).sqrt().ceil() as usize;
    let spacing = 2.2;
    let half = (side.saturating_sub(1)) as f32 * spacing * 0.5;

    let Ok(root) = q_root.get_single() else {
        return;
    };
    let spawn_start = Instant::now();
    commands.entity(root).with_children(|p| {
        for (i, h) in handles.into_iter().enumerate() {
            let x = (i % side) as f32 * spacing - half;
            let z = (i / side) as f32 * spacing - half;
            p.spawn((
                PbrBundle {
                    mesh: h,
                    material: material.0.clone(),
                    transform: Transform::from_xyz(x, 0.0, z),
                    ..default()
                },
                StressInstance,
            ));
        }
    });
    let spawn_ms = spawn_start.elapsed().as_secs_f64() as f32 * 1000.0;

    if let Ok(mut cam) = q_cam.get_single_mut() {
        let extent = (side as f32) * spacing;
        let dist = extent.max(4.0);
        *cam = Transform::from_xyz(0.0, dist * 0.7, dist * 0.9)
            .looking_at(Vec3::ZERO, Vec3::Y);
    }

    state.last_count = count;
    state.last_gen_ms = gen_ms;
    state.last_spawn_ms = spawn_ms;
    state.last_hits = cache.hits - hits_before;
    state.last_misses = cache.misses - misses_before;
    state.dirty = false;

    info!(
        "stress level={} count={} gen={:.2}ms spawn={:.2}ms hits={} misses={} cache={}",
        state.level,
        count,
        gen_ms,
        spawn_ms,
        state.last_hits,
        state.last_misses,
        cache.len(),
    );
}

pub fn update_hud(
    state: Res<StressState>,
    cache: Res<RockCache>,
    mut q: Query<&mut Text, With<StressHud>>,
) {
    let Ok(mut text) = q.get_single_mut() else {
        return;
    };
    text.sections[0].value = format!(
        "Stress Test\n  level={}/{}  count={}\n  gen={:.2}ms  spawn={:.2}ms  total={:.2}ms\n  hits(last)={}  misses(last)={}  cache_total={}\n  Up/Down = level   C = clear cache",
        state.level,
        MAX_LEVEL,
        state.last_count,
        state.last_gen_ms,
        state.last_spawn_ms,
        state.last_gen_ms + state.last_spawn_ms,
        state.last_hits,
        state.last_misses,
        cache.len(),
    );
}
