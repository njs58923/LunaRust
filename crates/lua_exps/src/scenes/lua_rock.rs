use bevy::prelude::*;
use bevy::render::mesh::{Indices, PrimitiveTopology};
use bevy::render::render_asset::RenderAssetUsages;
use mlua::{Function, Table};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use super::SceneRoot;
use crate::LuaVm;

pub const ROCK_SCRIPT: &str = include_str!("../lua/rock_generator.lua");

#[derive(Resource)]
pub struct RockState {
    pub seed: i64,
    pub dirty: bool,
}

impl Default for RockState {
    fn default() -> Self {
        Self { seed: 42, dirty: true }
    }
}

#[derive(Resource, Default)]
pub struct RockCache {
    meshes: HashMap<u64, Handle<Mesh>>,
    script_loaded: bool,
    pub hits: u32,
    pub misses: u32,
}

impl RockCache {
    pub fn ensure_loaded(&mut self, lua: &mlua::Lua) {
        if self.script_loaded {
            return;
        }
        if let Err(e) = lua.load(ROCK_SCRIPT).exec() {
            error!("rock script load failed: {e}");
            return;
        }
        self.script_loaded = true;
    }

    pub fn get(&self, key: &u64) -> Option<Handle<Mesh>> {
        self.meshes.get(key).cloned()
    }

    pub fn insert(&mut self, key: u64, h: Handle<Mesh>) {
        self.meshes.insert(key, h);
    }

    pub fn len(&self) -> usize {
        self.meshes.len()
    }

    pub fn clear(&mut self) {
        self.meshes.clear();
        self.hits = 0;
        self.misses = 0;
    }
}

#[derive(Resource)]
pub struct RockMaterial(pub Handle<StandardMaterial>);

#[derive(Component)]
pub struct RockEntity;

#[derive(Component)]
pub struct RockHud;

pub fn cache_key(seed: i64, script: &str) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    seed.hash(&mut h);
    script.hash(&mut h);
    h.finish()
}

pub fn setup(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut state: ResMut<RockState>,
) {
    state.dirty = true;

    let tex = asset_server.load("textura_roca.png");
    let mat = materials.add(StandardMaterial {
        base_color_texture: Some(tex),
        perceptual_roughness: 0.9,
        metallic: 0.0,
        ..default()
    });
    commands.insert_resource(RockMaterial(mat));

    commands
        .spawn((SceneRoot, SpatialBundle::default(), Name::new("LuaRockScene")))
        .with_children(|root| {
            root.spawn(PointLightBundle {
                point_light: PointLight {
                    intensity: 2_000_000.0,
                    shadows_enabled: true,
                    ..default()
                },
                transform: Transform::from_xyz(4.0, 6.0, 4.0),
                ..default()
            });
            root.spawn(Camera3dBundle {
                transform: Transform::from_xyz(0.0, 1.2, 3.5)
                    .looking_at(Vec3::ZERO, Vec3::Y),
                ..default()
            });
        });

    commands.spawn((
        TextBundle::from_section(
            "",
            TextStyle {
                font_size: 18.0,
                color: Color::srgb(0.9, 0.9, 0.5),
                ..default()
            },
        )
        .with_style(Style {
            position_type: PositionType::Absolute,
            bottom: Val::Px(8.0),
            left: Val::Px(8.0),
            ..default()
        }),
        RockHud,
        SceneRoot,
    ));
}

pub fn input(keys: Res<ButtonInput<KeyCode>>, mut state: ResMut<RockState>) {
    if keys.just_pressed(KeyCode::BracketLeft) {
        state.seed -= 1;
        state.dirty = true;
    }
    if keys.just_pressed(KeyCode::BracketRight) {
        state.seed += 1;
        state.dirty = true;
    }
    if keys.just_pressed(KeyCode::KeyR) {
        state.seed = random_seed();
        state.dirty = true;
    }
}

fn random_seed() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let n = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos() as i64;
    n & 0x7FFF_FFFF
}

pub fn regenerate(
    mut state: ResMut<RockState>,
    lua: Res<LuaVm>,
    mut cache: ResMut<RockCache>,
    mut meshes: ResMut<Assets<Mesh>>,
    material: Option<Res<RockMaterial>>,
    mut commands: Commands,
    q_old: Query<Entity, With<RockEntity>>,
    q_root: Query<Entity, (With<SceneRoot>, With<Name>)>,
) {
    if !state.dirty {
        return;
    }
    let Some(material) = material else { return };

    cache.ensure_loaded(&lua.0);

    for e in &q_old {
        commands.entity(e).despawn_recursive();
    }

    let key = cache_key(state.seed, ROCK_SCRIPT);
    let mesh_handle = if let Some(h) = cache.get(&key) {
        cache.hits += 1;
        info!("rock cache HIT seed={} (cache={})", state.seed, cache.len());
        h
    } else {
        match build_mesh_for_seed(&lua.0, state.seed) {
            Ok(mesh) => {
                let h = meshes.add(mesh);
                cache.insert(key, h.clone());
                cache.misses += 1;
                info!("rock cache MISS seed={} (cache={})", state.seed, cache.len());
                h
            }
            Err(e) => {
                error!("rock build failed: {e}");
                state.dirty = false;
                return;
            }
        }
    };

    let Ok(root) = q_root.get_single() else {
        return;
    };
    commands.entity(root).with_children(|p| {
        p.spawn((
            PbrBundle {
                mesh: mesh_handle,
                material: material.0.clone(),
                ..default()
            },
            RockEntity,
        ));
    });

    state.dirty = false;
}

pub fn update_hud(
    state: Res<RockState>,
    cache: Res<RockCache>,
    mut q: Query<&mut Text, With<RockHud>>,
) {
    let Ok(mut text) = q.get_single_mut() else {
        return;
    };
    text.sections[0].value = format!(
        "Lua Rock\n  seed={}  cache={}  hits={}  misses={}\n  [ / ] -+1 seed   R random",
        state.seed,
        cache.len(),
        cache.hits,
        cache.misses,
    );
}

pub fn build_mesh_for_seed(lua: &mlua::Lua, seed: i64) -> mlua::Result<Mesh> {
    let gen: Function = lua.globals().get("generate_rock")?;
    let rock: Table = gen.call((seed, mlua::Value::Nil))?;
    let verts_tbl: Table = rock.get("vertices")?;
    let faces_tbl: Table = rock.get("faces")?;

    let mut positions: Vec<[f32; 3]> = Vec::new();
    for v in verts_tbl.sequence_values::<Table>() {
        let v = v?;
        let x: f32 = v.get("x")?;
        let y: f32 = v.get("y")?;
        let z: f32 = v.get("z")?;
        positions.push([x, y, z]);
    }

    let mut indices: Vec<u32> = Vec::new();
    for f in faces_tbl.sequence_values::<Table>() {
        let f = f?;
        let a: u32 = f.get(1)?;
        let b: u32 = f.get(2)?;
        let c: u32 = f.get(3)?;
        indices.extend_from_slice(&[a - 1, b - 1, c - 1]);
    }

    let mut normals: Vec<[f32; 3]> = vec![[0.0; 3]; positions.len()];
    for tri in indices.chunks_exact(3) {
        let p0 = Vec3::from(positions[tri[0] as usize]);
        let p1 = Vec3::from(positions[tri[1] as usize]);
        let p2 = Vec3::from(positions[tri[2] as usize]);
        let n = (p1 - p0).cross(p2 - p0).normalize_or_zero();
        for &i in tri {
            let idx = i as usize;
            normals[idx][0] += n.x;
            normals[idx][1] += n.y;
            normals[idx][2] += n.z;
        }
    }
    for n in &mut normals {
        let v = Vec3::from(*n).normalize_or_zero();
        *n = [v.x, v.y, v.z];
    }

    let uvs: Vec<[f32; 2]> = positions
        .iter()
        .map(|p| {
            let v = Vec3::from(*p).normalize_or_zero();
            let u = 0.5 + v.z.atan2(v.x) / (2.0 * std::f32::consts::PI);
            let vv = 0.5 - v.y.asin() / std::f32::consts::PI;
            [u, vv]
        })
        .collect();

    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_indices(Indices::U32(indices));
    Ok(mesh)
}
