//! Host-owned window placement. DOM ownership stays separate from render placement.
use crate::{ElemenetWorld, EntityMap, NavigationEpoch};
use bevy::prelude::*;
use specs::WorldExt;
use std::collections::HashMap;

#[derive(Clone, Copy)]
pub(crate) struct WindowBinding {
    pub anchor: specs::Entity,
    pub content: specs::Entity,
    pub epoch: u64,
    pub revision: u64,
    pub runtime_id: u64,
}

#[derive(Resource, Default)]
pub(crate) struct EmbeddedWindows(pub HashMap<u32, WindowBinding>);

fn current_pose(world: &World, mut entity: Entity) -> Option<(GlobalTransform, bool)> {
    let mut chain = Vec::new();
    let mut visible = true;
    for _ in 0..256 {
        let node = world.get_entity(entity)?;
        chain.push(*node.get::<Transform>()?);
        visible &= !matches!(node.get::<Visibility>(), Some(Visibility::Hidden));
        if let Some(parent) = node.get::<Parent>() {
            entity = parent.get();
        } else {
            let mut pose = GlobalTransform::IDENTITY;
            for local in chain.iter().rev() {
                pose = pose.mul_transform(*local);
            }
            return Some((pose, visible));
        }
    }
    None
}

/// Run after DOM commands, before Bevy propagates transforms. Copy from the
/// actual frame hierarchy, never from a delayed worker snapshot. Do not parent
/// content under chrome: deleting/rebuilding chrome must not despawn an app.
pub fn sync_embedded_windows(world: &mut World) {
    if !world.contains_resource::<EmbeddedWindows>() {
        return;
    }
    world.resource_scope(|world, mut windows: Mut<EmbeddedWindows>| {
        let epoch = world
            .get_resource::<NavigationEpoch>()
            .map(|e| e.0)
            .unwrap_or(0);
        windows.0.retain(|_, binding| {
            let (content_alive, anchor_alive) = {
                let Some(specs) = world.get_resource::<ElemenetWorld>() else {
                    return false;
                };
                let entities = specs.0.entities();
                (
                    entities.is_alive(binding.content),
                    entities.is_alive(binding.anchor),
                )
            };
            if !content_alive || binding.epoch != epoch {
                return false;
            }
            let Some(map) = world.get_resource::<EntityMap>() else {
                return true;
            };
            let content = map.0.get(&binding.content.id()).copied();
            let anchor = map.0.get(&binding.anchor.id()).copied();
            let Some(content) = content else {
                return anchor_alive;
            };
            let placement = anchor
                .filter(|_| anchor_alive)
                .and_then(|e| current_pose(world, e));
            let Some((anchor_pose, visible)) = placement else {
                if let Some(mut visibility) = world.get_mut::<Visibility>(content) {
                    *visibility = Visibility::Hidden;
                }
                return anchor_alive;
            };
            let parent_pose = world
                .get::<Parent>(content)
                .map(|p| current_pose(world, p.get()).map(|p| p.0))
                .unwrap_or(Some(GlobalTransform::IDENTITY));
            let Some(parent_pose) = parent_pose else {
                return true;
            };
            let local = anchor_pose.reparented_to(&parent_pose);
            if !local.compute_matrix().is_finite() {
                return true;
            }
            if let Some(mut transform) = world.get_mut::<Transform>(content) {
                if *transform != local {
                    *transform = local;
                }
            }
            if let Some(mut visibility) = world.get_mut::<Visibility>(content) {
                let next = if visible {
                    Visibility::Inherited
                } else {
                    Visibility::Hidden
                };
                if *visibility != next {
                    *visibility = next;
                }
            }
            true
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use specs::Builder;

    #[test]
    fn content_follows_real_frame_without_js_and_survives_chrome_replacement() {
        let mut world = World::new();
        let mut specs = virtual_dom::dom::element::build_world();
        let anchor_spec = specs.create_entity().build();
        let content_spec = specs.create_entity().build();
        let shell = world
            .spawn(SpatialBundle::from_transform(
                Transform::from_xyz(4.0, 1.7, -3.0).with_rotation(Quat::from_rotation_y(0.8)),
            ))
            .id();
        let frame = world
            .spawn(SpatialBundle::from_transform(Transform::from_xyz(
                1.0, 0.2, -2.0,
            )))
            .id();
        world.entity_mut(frame).set_parent(shell);
        let app_parent = world
            .spawn(SpatialBundle::from_transform(
                Transform::from_xyz(-5.0, 0.0, 7.0).with_rotation(Quat::from_rotation_y(-0.5)),
            ))
            .id();
        let content = world
            .spawn(SpatialBundle::from_transform(Transform::from_xyz(
                99.0, 9.0, 9.0,
            )))
            .id();
        world.entity_mut(content).set_parent(app_parent);
        world.insert_resource(ElemenetWorld(specs));
        world.insert_resource(EntityMap(HashMap::from([
            (anchor_spec.id(), frame),
            (content_spec.id(), content),
        ])));
        world.insert_resource(EmbeddedWindows(HashMap::from([(
            content_spec.id(),
            WindowBinding {
                anchor: anchor_spec,
                content: content_spec,
                epoch: 0,
                revision: 1,
                runtime_id: 1,
            },
        )])));
        for step in 0..10 {
            *world.get_mut::<Transform>(shell).unwrap() =
                Transform::from_xyz(step as f32, 1.7, -3.0)
                    .with_rotation(Quat::from_rotation_y(step as f32 * 0.3));
            // Simulate late DOM writes from an app worker. Host placement wins.
            world.get_mut::<Transform>(content).unwrap().translation = Vec3::splat(99.0);
            sync_embedded_windows(&mut world);
            let expected = current_pose(&world, frame).unwrap().0;
            let actual = current_pose(&world, content).unwrap().0;
            assert!(expected
                .compute_matrix()
                .abs_diff_eq(actual.compute_matrix(), 0.0001));
        }
        *world.get_mut::<Visibility>(shell).unwrap() = Visibility::Hidden;
        sync_embedded_windows(&mut world);
        assert_eq!(
            *world.get::<Visibility>(content).unwrap(),
            Visibility::Hidden
        );
        *world.get_mut::<Visibility>(shell).unwrap() = Visibility::Inherited;
        sync_embedded_windows(&mut world);
        assert_eq!(
            *world.get::<Visibility>(content).unwrap(),
            Visibility::Inherited
        );
        world.entity_mut(frame).despawn_recursive();
        sync_embedded_windows(&mut world);
        assert!(
            world.get_entity(content).is_some(),
            "chrome deletion must not despawn the app"
        );
        assert_eq!(
            *world.get::<Visibility>(content).unwrap(),
            Visibility::Hidden
        );
        let replacement = world
            .spawn(SpatialBundle::from_transform(Transform::from_xyz(
                2.0, 3.0, 4.0,
            )))
            .id();
        world
            .resource_mut::<EntityMap>()
            .0
            .insert(anchor_spec.id(), replacement);
        sync_embedded_windows(&mut world);
        assert!(current_pose(&world, content)
            .unwrap()
            .0
            .translation()
            .abs_diff_eq(Vec3::new(2.0, 3.0, 4.0), 0.0001));
        world.insert_resource(NavigationEpoch(1));
        sync_embedded_windows(&mut world);
        assert!(world.resource::<EmbeddedWindows>().0.is_empty());
    }
}
