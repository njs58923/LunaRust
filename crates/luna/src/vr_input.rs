//! Native side of the input subcontroller. Anchors never need to know XR names.
use crate::permissions::{CapabilityBits, NativeServiceBits};
use crate::{anchors::NativeReferences, ElemenetWorld, SpacePolicies};
use bevy::{prelude::*, render::view::VisibilitySystems, transform::TransformSystem};
use bevy_mod_xr::{session::XrState, spaces::XrSpaceLocationFlags};
use bevy_xr_utils::tracking_utils::{LeftGrip, RightGrip, XrTrackedLeftGrip, XrTrackedRightGrip};
use specs::WorldExt;
use std::collections::HashMap;
use virtual_dom::dom::element::Tag;

#[derive(Resource, Default)]
struct Providers(HashMap<specs::Entity, [u64; 2]>);
pub struct VrInputPlugin;
impl Plugin for VrInputPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Providers>()
            .init_resource::<NativeReferences>()
            .add_systems(
                Update,
                prepare_references
                    .after(crate::permissions::update_active_native_services_system)
                    .before(crate::js::js_tick_system),
            )
            .add_systems(
                PostUpdate,
                refresh_references
                    .after(TransformSystem::TransformPropagate)
                    .after(VisibilitySystems::VisibilityPropagate)
                    .before(crate::anchors::follow),
            );
    }
}
pub(crate) fn prepare_references(world: &mut World) {
    let Some(specs) = world.get_resource::<ElemenetWorld>() else {
        return;
    };
    let Some(policies) = world.get_resource::<SpacePolicies>() else {
        return;
    };
    let tags = specs.0.read_storage::<Tag>();
    let owners: Vec<_> = policies
        .by_space
        .iter()
        .filter(|(_, p)| p.effective_native.contains(NativeServiceBits::VR_INPUT))
        .map(|(&id, _)| specs.0.entities().entity(id))
        .filter(|e| {
            specs.0.entities().is_alive(*e) && tags.get(*e).is_some_and(|tag| tag.0 == "space")
        })
        .collect();
    drop(tags);
    for owner in owners {
        if world.resource::<Providers>().0.contains_key(&owner) {
            continue;
        }
        let mut refs = world.resource_mut::<NativeReferences>();
        let left = refs.register(owner, "__vr_left_grip_root", None);
        let right = refs.register(owner, "__vr_right_grip_root", None);
        for id in [left, right] {
            refs.restrict(
                id,
                CapabilityBits::READ_POSE_STREAM,
                NativeServiceBits::VR_INPUT,
            );
        }
        world
            .resource_mut::<Providers>()
            .0
            .insert(owner, [left, right]);
    }
}
fn refresh_references(world: &mut World) {
    if world.resource::<Providers>().0.is_empty() {
        return;
    }
    let fallback_left = world
        .query_filtered::<(Entity, &Visibility), With<XrTrackedLeftGrip>>()
        .get_single(world)
        .ok()
        .filter(|(_, v)| **v != Visibility::Hidden)
        .map(|(e, _)| e);
    let fallback_right = world
        .query_filtered::<(Entity, &Visibility), With<XrTrackedRightGrip>>()
        .get_single(world)
        .ok()
        .filter(|(_, v)| **v != Visibility::Hidden)
        .map(|(e, _)| e);
    let left_raw = world
        .query_filtered::<(Entity, Option<&XrSpaceLocationFlags>), With<LeftGrip>>()
        .get_single(world)
        .ok()
        .map(|(e, flags)| {
            (
                e,
                flags.is_none_or(|f| f.position_tracked && f.rotation_tracked),
            )
        });
    let right_raw = world
        .query_filtered::<(Entity, Option<&XrSpaceLocationFlags>), With<RightGrip>>()
        .get_single(world)
        .ok()
        .map(|(e, flags)| {
            (
                e,
                flags.is_none_or(|f| f.position_tracked && f.rotation_tracked),
            )
        });
    let running = world
        .get_resource::<crate::RenderMode>()
        .is_none_or(|m| m.is_vr)
        && world
            .get_resource::<XrState>()
            .is_none_or(|s| *s == XrState::Running)
        && world
            .get_resource::<bevy_mod_openxr::resources::OxrFrameState>()
            .is_none_or(|frame| frame.should_render);
    // Prefer the tracked pivot, without a visual model's scale or offsets.
    // A present but invalid tracked pivot never falls back to a stale proxy.
    let left = left_raw
        .map(|(e, valid)| valid.then_some(e))
        .unwrap_or(fallback_left)
        .filter(|_| running);
    let right = right_raw
        .map(|(e, valid)| valid.then_some(e))
        .unwrap_or(fallback_right)
        .filter(|_| running);
    // Input and tracked poses come from the same OpenXR action sync. Generic
    // anchor listeners can receive this optional metadata without naming XR.
    let input = world
        .get_resource::<crate::vr_locomotion::LunaLocomotionActions>()
        .zip(world.get_resource::<bevy_mod_openxr::session::OxrSession>())
        .map(|(actions, session)| {
            let axis = |action: &openxr::Action<f32>| {
                action
                    .state(session, openxr::Path::NULL)
                    .ok()
                    .filter(|s| s.is_active)
                    .map(|s| s.current_state)
                    .unwrap_or(0.)
            };
            let button = |action: &openxr::Action<bool>| {
                action
                    .state(session, openxr::Path::NULL)
                    .ok()
                    .filter(|s| s.is_active)
                    .is_some_and(|s| s.current_state)
            };
            [
                js_runtime::anchors::ControllerInput {
                    trigger: axis(&actions.left_trigger),
                    grip: axis(&actions.left_grip),
                    primary: button(&actions.btn_x),
                    secondary: button(&actions.btn_y),
                },
                js_runtime::anchors::ControllerInput {
                    trigger: axis(&actions.right_trigger),
                    grip: axis(&actions.right_grip),
                    primary: button(&actions.btn_a),
                    secondary: button(&actions.btn_b),
                },
            ]
        });
    for (index, entity) in [left, right].into_iter().enumerate() {
        if let Some(entity) = entity {
            if let Some(input) = &input {
                world
                    .entity_mut(entity)
                    .insert(crate::anchors::NativeControllerInput(input[index].clone()));
            } else {
                world
                    .entity_mut(entity)
                    .remove::<crate::anchors::NativeControllerInput>();
            }
        }
    }
    let updates: Vec<_> = world
        .resource::<Providers>()
        .0
        .iter()
        .map(|(&owner, &ids)| {
            let alive = world
                .get_resource::<ElemenetWorld>()
                .is_some_and(|s| s.0.entities().is_alive(owner));
            let allowed = alive
                && world
                    .get_resource::<SpacePolicies>()
                    .and_then(|p| p.by_space.get(&owner.id()))
                    .is_some_and(|p| p.effective_native.contains(NativeServiceBits::VR_INPUT));
            (owner, ids, alive, allowed)
        })
        .collect();
    for (owner, ids, alive, allowed) in updates {
        let mut refs = world.resource_mut::<NativeReferences>();
        refs.set_target(ids[0], left.filter(|_| allowed));
        refs.set_target(ids[1], right.filter(|_| allowed));
        if !alive {
            world.resource_mut::<Providers>().0.remove(&owner);
        }
    }
}
