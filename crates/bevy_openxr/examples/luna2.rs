//! A simple 3D scene with light shining over a cube sitting on a plane, including a custom model.

use bevy::{gltf::GltfPlugin, prelude::*};
use bevy_mod_openxr::add_xr_plugins;
use std::f32::consts::*;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins)
        .add_systems(Startup, setup)
        .add_systems(Update, rotate_silla) // Agregar el sistema de rotación
        .run();

    // App::new()
    //     .add_plugins(add_xr_plugins(DefaultPlugins))
    //     .add_plugins(bevy_xr_utils::hand_gizmos::HandGizmosPlugin)
    //     .add_systems(Startup, setup)
    //     .run();
}

/// Set up a simple 3D scene with a custom model
fn setup(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    // Circular base
    commands.spawn(PbrBundle {
        mesh: meshes.add(Circle::new(4.0)),
        material: materials.add(StandardMaterial {
            base_color: Color::WHITE,
            ..default()
        }),
        transform: Transform::from_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)),
        ..default()
    });

    // Cube
    // commands.spawn(PbrBundle {
    //     mesh: meshes.add(Cuboid::new(1.0, 1.0, 1.0)),
    //     material: materials.add(StandardMaterial {
    //         base_color: Color::rgb_u8(124, 144, 255),
    //         ..default()
    //     }),
    //     transform: Transform::from_xyz(0.0, 0.5, 0.0),
    //     ..default()
    // });

    // Carga el modelo usando el AssetServer
    let model_handle = asset_server.load(GltfAssetLabel::Scene(0).from_asset("silla.glb"));

    commands.spawn((
        SceneBundle {
            scene: model_handle,
            ..default()
        },
        Silla, // Marca personalizada para identificar esta entidad
    ));

    // Load a custom 3D model (GLTF/GLB)
    // commands.spawn(SceneBundle {
    //     scene: asset_server.load("models/silla.glb"),
    //     transform: Transform::from_xyz(0.0, 0.0, 0.0), // Adjust position as needed
    //     ..default()
    // });

    // Light
    commands.spawn(PointLightBundle {
        point_light: PointLight {
            intensity: 1500.0,
            shadows_enabled: true,
            ..default()
        },
        transform: Transform::from_xyz(4.0, 8.0, 4.0),
        ..default()
    });

    // Camera
    commands.spawn(Camera3dBundle {
        transform: Transform::from_xyz(-2.5, 4.5, 9.0).looking_at(Vec3::ZERO, Vec3::Y),
        ..default()
    });
}

#[derive(Component)]
struct Silla;

fn rotate_silla(mut query: Query<&mut Transform, With<Silla>>, time: Res<Time>) {
    for mut transform in query.iter_mut() {
        // Rotar la silla en el eje Y
        transform.rotate_y(1.0 * time.delta_seconds());
    }
}
