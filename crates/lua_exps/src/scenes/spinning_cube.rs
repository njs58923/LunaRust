use bevy::prelude::*;

use super::SceneRoot;

#[derive(Component)]
pub struct Spin;

pub fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands
        .spawn((SceneRoot, SpatialBundle::default(), Name::new("SpinningCube")))
        .with_children(|root| {
            root.spawn((
                PbrBundle {
                    mesh: meshes.add(Cuboid::new(1.0, 1.0, 1.0)),
                    material: materials.add(Color::srgb(0.3, 0.5, 0.9)),
                    transform: Transform::from_xyz(0.0, 1.0, 0.0),
                    ..default()
                },
                Spin,
            ));
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
                transform: Transform::from_xyz(0.0, 2.5, 4.0)
                    .looking_at(Vec3::new(0.0, 1.0, 0.0), Vec3::Y),
                ..default()
            });
        });
}

pub fn rotate(time: Res<Time>, mut q: Query<&mut Transform, With<Spin>>) {
    for mut t in &mut q {
        t.rotate_y(time.delta_seconds() * 1.5);
    }
}
