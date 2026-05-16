use bevy::prelude::*;

use super::SceneRoot;

pub fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands
        .spawn((SceneRoot, SpatialBundle::default(), Name::new("DefaultScene")))
        .with_children(|root| {
            root.spawn(PbrBundle {
                mesh: meshes.add(Plane3d::default().mesh().size(8.0, 8.0)),
                material: materials.add(Color::srgb(0.3, 0.5, 0.3)),
                ..default()
            });
            root.spawn(PbrBundle {
                mesh: meshes.add(Sphere::new(0.5)),
                material: materials.add(Color::srgb(0.8, 0.3, 0.3)),
                transform: Transform::from_xyz(0.0, 0.5, 0.0),
                ..default()
            });
            root.spawn(PointLightBundle {
                point_light: PointLight {
                    intensity: 1_500_000.0,
                    shadows_enabled: true,
                    ..default()
                },
                transform: Transform::from_xyz(4.0, 6.0, 4.0),
                ..default()
            });
            root.spawn(Camera3dBundle {
                transform: Transform::from_xyz(0.0, 2.0, 5.0)
                    .looking_at(Vec3::new(0.0, 0.5, 0.0), Vec3::Y),
                ..default()
            });
        });
}
