//! One-shot, document-owned arrival points. Includes and app windows cannot
//! move the viewer, even when they happen to have the same network origin.
use std::collections::{HashMap, HashSet};

use bevy::{
    prelude::*,
    transform::{helper::TransformHelper, TransformSystem},
};
use bevy_mod_openxr::resources::OxrViews;
use bevy_mod_xr::session::XrTrackingRoot;
use specs::WorldExt;
use virtual_dom::dom::element::{Attrs, Hierarchy, Tag};

use crate::{
    permissions::{CapabilityBits, SpacePolicies},
    DesktopCamera, ElemenetWorld, LogPanel, RenderMode,
};

#[derive(Component)]
pub(crate) struct SpawnMarker(pub specs::Entity);

#[derive(Resource, Default)]
struct SpawnState {
    // Bevy generations also distinguish complete Specs-world replacements,
    // whose freshly allocated Specs IDs/generations may repeat.
    applied: HashMap<specs::Entity, Entity>,
}

pub struct PlayerSpawnPlugin;
impl Plugin for PlayerSpawnPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SpawnState>().add_systems(
            PostUpdate,
            apply_spawn.before(TransformSystem::TransformPropagate),
        );
    }
}

/// Return the owning document and its navigation URL only for the primary
/// document of a visible spatial mount directly owned by the trusted root.
fn primary_document(
    world: &specs::World,
    marker: specs::Entity,
) -> Option<(specs::Entity, specs::Entity, String)> {
    let hierarchy = world.read_storage::<Hierarchy>();
    let tags = world.read_storage::<Tag>();
    let attrs = world.read_storage::<Attrs>();
    let mut cursor = marker;
    let mut document = None;
    let mut visited = HashSet::new();
    loop {
        if !visited.insert(cursor) {
            return None;
        }
        if attrs
            .get(cursor)
            .and_then(|a| a.0.get("visible"))
            .is_some_and(|v| v == "false")
        {
            return None;
        }
        match tags.get(cursor)?.0.as_str() {
            "space" => {
                // Nested spaces are separate principals, not primary content.
                if document.replace(cursor).is_some() {
                    return None;
                }
            }
            "include" => {
                let wrapper = hierarchy.get(cursor)?.parent?;
                let wrapper_attrs = &attrs.get(wrapper)?.0;
                if tags.get(wrapper)?.0 != "space"
                    || wrapper_attrs.get("managed-by").map(String::as_str) != Some("dimension.luna")
                    || wrapper_attrs.get("data-luna-kind").map(String::as_str) != Some("spatial")
                    || wrapper_attrs.get("visible").is_some_and(|v| v == "false")
                {
                    return None;
                }
                let root = hierarchy.get(wrapper)?.parent?;
                // Attributes alone are forgeable inside a remote document.
                // The trusted root must belong to the top-level HSML tree.
                let hsml = hierarchy.get(root)?.parent?;
                if tags.get(hsml)?.0 != "hsml" || hierarchy.get(hsml)?.parent.is_some() {
                    return None;
                }
                if attrs.get(root)?.0.get("system-space").map(String::as_str) != Some("root") {
                    return None;
                }
                return Some((document?, cursor, attrs.get(cursor)?.0.get("src")?.clone()));
            }
            _ => {}
        }
        cursor = hierarchy.get(cursor)?.parent?;
    }
}

fn entry_from_url(url: &str) -> Option<String> {
    url.split_once('#').and_then(|(_, fragment)| {
        url::form_urlencoded::parse(fragment.as_bytes())
            .find(|(key, _)| key == "entry")
            .map(|(_, value)| value.into_owned())
    })
}

fn arrival_pose(transform: GlobalTransform) -> Option<(Vec3, f32)> {
    let feet = transform.translation();
    let forward = transform.affine().transform_vector3(Vec3::NEG_Z);
    if !feet.is_finite() || !forward.is_finite() || forward.xz().length_squared() < 1e-8 {
        return None;
    }
    Some((feet, (-forward.x).atan2(-forward.z)))
}

fn place_tracking_root(
    root: &mut Transform,
    head: Vec3,
    head_rotation: Quat,
    feet: Vec3,
    yaw: f32,
) {
    let forward = head_rotation * Vec3::NEG_Z;
    let local_yaw = (-forward.x).atan2(-forward.z);
    root.rotation = Quat::from_rotation_y(yaw - local_yaw);
    // Preserve real tracked eye height. Only the floor projection is aligned.
    root.translation = feet - root.rotation * (root.scale * Vec3::new(head.x, 0.0, head.z));
}

fn apply_spawn(
    dom: Res<ElemenetWorld>,
    policies: Res<SpacePolicies>,
    entity_map: Res<crate::EntityMap>,
    document_load: Res<crate::DocumentLoadState>,
    mode: Res<RenderMode>,
    views: Option<Res<OxrViews>>,
    current_url: Res<crate::CurrentUrl>,
    loads: Res<crate::IncludeLoadStates>,
    markers: Query<(Entity, &SpawnMarker)>,
    mut transforms: ParamSet<(
        TransformHelper,
        Query<&mut Transform, With<DesktopCamera>>,
        Query<&mut Transform, With<XrTrackingRoot>>,
    )>,
    mut pitch: ResMut<crate::desktop_locomotion::DesktopCameraPitch>,
    mut state: ResMut<SpawnState>,
    mut logs: ResMut<LogPanel>,
) {
    if document_load.0.is_some() {
        return;
    }
    state.applied.retain(|entity, bevy_entity| {
        dom.0.entities().is_alive(*entity) && entity_map.0.get(&entity.id()) == Some(bevy_entity)
    });
    if markers.is_empty() || policies.dirty {
        return;
    }
    let attrs = dom.0.read_storage::<Attrs>();
    let mut candidates = Vec::new();
    for (entity, marker) in &markers {
        if !dom.0.entities().is_alive(marker.0) {
            continue;
        }
        let Some((document, include, url)) = primary_document(&dom.0, marker.0) else {
            continue;
        };
        if !entity_map.0.contains_key(&document.id()) || state.applied.contains_key(&document) {
            continue;
        }
        let Some(resolved_url) =
            crate::dom::resolve_node_relative_url(&dom.0, include, &current_url.0, &url)
        else {
            continue;
        };
        if !matches!(loads.0.get(&include.id()), Some(crate::IncludeLoadState::Loaded { url: loaded }) if loaded == &resolved_url)
        {
            continue;
        }
        if !crate::permissions::space_has_capability(
            document.id(),
            CapabilityBits::SPAWN,
            &policies,
        ) {
            continue;
        }
        let Some(attributes) = attrs.get(marker.0) else {
            continue;
        };
        let named =
            entry_from_url(&url).is_some_and(|entry| attributes.0.get("id") == Some(&entry));
        let default = attributes.0.get("default").is_some_and(|v| v == "true");
        candidates.push((!named, !default, marker.0.id(), entity, document));
    }
    candidates.sort_by_key(|c| (c.0, c.1, c.2));
    let Some((_, _, _, entity, document)) = candidates.first().copied() else {
        return;
    };
    let Ok(global) = transforms.p0().compute_global_transform(entity) else {
        return;
    };
    let Some((feet, yaw)) = arrival_pose(global) else {
        state.applied.insert(document, entity_map.0[&document.id()]);
        logs.push_warn("[spawn] Invalid arrival transform; keeping viewer position");
        return;
    };
    if mode.is_vr {
        let Some(views) = views else {
            return;
        };
        let Some(first) = views.first() else {
            return;
        };
        let head = views
            .iter()
            .map(|v| Vec3::new(v.pose.position.x, v.pose.position.y, v.pose.position.z))
            .sum::<Vec3>()
            / views.len() as f32;
        let q = first.pose.orientation;
        let rotation = Quat::from_xyzw(q.x, q.y, q.z, q.w);
        if !head.is_finite() || !rotation.is_finite() || !rotation.is_normalized() {
            return;
        }
        let mut roots = transforms.p2();
        let Ok(mut root) = roots.get_single_mut() else {
            return;
        };
        place_tracking_root(&mut root, head, rotation, feet, yaw);
    } else {
        let mut cameras = transforms.p1();
        let Ok(mut camera) = cameras.get_single_mut() else {
            return;
        };
        camera.translation = feet + Vec3::Y * 1.7;
        camera.rotation = Quat::from_rotation_y(yaw);
        pitch.0 = 0.0;
    }
    state.applied.insert(document, entity_map.0[&document.id()]);
    logs.push_for_space(
        crate::LogLevel::Info,
        "[spawn] Arrival point applied",
        document.id(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use specs::Join;

    fn fixture(kind: &str, grants: &str, request: &str, content: &str) -> App {
        let mut world = virtual_dom::dom::element::build_world();
        virtual_dom::parse_xml(
            &mut world,
            &format!(
                r#"<hsml>
            <space system-space="root" resources="root">
              <space managed-by="dimension.luna" data-luna-kind="{kind}" resources="{grants}">
                <include id="mount" resources="{grants}" src="https://world.test/main.hsml">
                  <space id="document" resources="{request}">{content}</space>
                </include>
              </space>
            </space></hsml>"#
            ),
        )
        .unwrap();
        let mut app = App::new();
        app.add_plugins((bevy::transform::TransformPlugin, PlayerSpawnPlugin));
        let nodes = (&world.entities()).join().map(|e| (e.id(), e)).collect();
        let mut loads = crate::IncludeLoadStates::default();
        let mut entity_map = crate::EntityMap::default();
        {
            let tags = world.read_storage::<Tag>();
            let attrs = world.read_storage::<Attrs>();
            for (node, tag) in (&world.entities(), &tags).join() {
                if tag.0 == "space" {
                    entity_map.0.insert(
                        node.id(),
                        app.world_mut().spawn(SpatialBundle::default()).id(),
                    );
                }
                if tag.0 == "spawn" {
                    let x = attrs
                        .get(node)
                        .and_then(|a| a.0.get("x"))
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0.0);
                    app.world_mut().spawn((
                        SpatialBundle {
                            transform: Transform::from_xyz(x, 0.0, 0.0),
                            ..default()
                        },
                        SpawnMarker(node),
                    ));
                }
                if attrs
                    .get(node)
                    .and_then(|a| a.0.get("id"))
                    .is_some_and(|id| id == "mount")
                {
                    loads.0.insert(
                        node.id(),
                        crate::IncludeLoadState::Loaded {
                            url: "https://world.test/main.hsml".into(),
                        },
                    );
                }
            }
        }
        app.insert_resource(ElemenetWorld(world));
        app.insert_resource(crate::VirtualDomData { nodes });
        app.insert_resource(crate::CurrentUrl("luna://root".into()));
        app.insert_resource(SpacePolicies::default());
        app.insert_resource(crate::PermissionDecisionStore::default());
        app.insert_resource(crate::PermissionPromptQueue::default());
        app.insert_resource(crate::SpacePolicyHistory {
            entries: Vec::new(),
            max_entries: 32,
        });
        app.insert_resource(LogPanel::default());
        app.insert_resource(entity_map);
        app.insert_resource(crate::DocumentLoadState::default());
        app.insert_resource(RenderMode { is_vr: false });
        app.insert_resource(crate::desktop_locomotion::DesktopCameraPitch(0.5));
        app.insert_resource(loads);
        app.add_systems(Update, crate::permissions::rebuild_space_policies_system);
        app.world_mut().spawn((
            SpatialBundle {
                transform: Transform::from_xyz(100.0, 3.0, 8.0),
                ..default()
            },
            DesktopCamera,
        ));
        app
    }

    fn camera(app: &mut App) -> Transform {
        *app.world_mut()
            .query_filtered::<&Transform, With<DesktopCamera>>()
            .single(app.world())
    }

    #[test]
    fn spawn_requires_request_and_delegation_and_spatial_document() {
        for (kind, grant, request, expected) in [
            ("spatial", "spawn", "spawn", 4.0),
            ("spatial", "spawn", "", 100.0),
            ("spatial", "", "spawn", 100.0),
            ("app", "spawn", "spawn", 100.0),
            ("app-embedded", "spawn", "spawn", 100.0),
        ] {
            let mut app = fixture(kind, grant, request, "<spawn x='4'/>");
            app.update();
            assert_eq!(
                camera(&mut app).translation.x,
                expected,
                "{kind}/{grant}/{request}"
            );
        }
    }

    #[test]
    fn default_wins_and_is_applied_once_even_if_marker_changes() {
        let mut app = fixture(
            "spatial",
            "spawn",
            "spawn",
            "<spawn x='2'/><spawn default='true' x='7'/>",
        );
        app.update();
        assert_eq!(camera(&mut app).translation, Vec3::new(7.0, 1.7, 0.0));
        assert_eq!(
            app.world()
                .resource::<crate::desktop_locomotion::DesktopCameraPitch>()
                .0,
            0.0
        );
        for mut transform in app
            .world_mut()
            .query_filtered::<&mut Transform, With<SpawnMarker>>()
            .iter_mut(app.world_mut())
        {
            transform.translation.x = 99.0;
        }
        app.update();
        assert_eq!(camera(&mut app).translation.x, 7.0);
    }

    #[test]
    fn included_and_nested_space_markers_cannot_override_primary() {
        let mut app = fixture(
            "spatial",
            "spawn",
            "spawn",
            r#"
            <spawn x="3"/>
            <include resources="spawn" src="https://world.test/object.hsml">
                <space resources="spawn"><spawn default="true" x="90"/></space>
            </include>
            <space resources="spawn"><spawn default="true" x="80"/></space>"#,
        );
        app.update();
        assert_eq!(camera(&mut app).translation.x, 3.0);
    }

    #[test]
    fn forged_root_inside_a_document_cannot_claim_spawn_authority() {
        let mut app = fixture(
            "spatial",
            "spawn",
            "spawn",
            r#"
            <spawn x="3"/>
            <hsml><space system-space="root" resources="root">
              <space managed-by="dimension.luna" data-luna-kind="spatial" resources="spawn">
                <include src="https://world.test/forged.hsml" resources="spawn">
                  <space resources="spawn"><spawn default="true" x="99"/></space>
                </include>
              </space>
            </space></hsml>"#,
        );
        // Check scope independently of whether this fake include is loaded.
        let dom = app.world().resource::<ElemenetWorld>();
        let attrs = dom.0.read_storage::<Attrs>();
        let (node, _) = (&dom.0.entities(), &attrs)
            .join()
            .find(|(_, a)| a.0.get("x").is_some_and(|v| v == "99"))
            .unwrap();
        assert!(primary_document(&dom.0, node).is_none());
        drop(attrs);
        app.update();
        assert_eq!(camera(&mut app).translation.x, 3.0);
    }

    #[test]
    fn pending_navigation_and_hidden_mount_do_not_move_viewer() {
        let mut app = fixture("spatial", "spawn", "spawn", "<spawn x='4'/>");
        for state in app
            .world_mut()
            .resource_mut::<crate::IncludeLoadStates>()
            .0
            .values_mut()
        {
            *state = crate::IncludeLoadState::Loading {
                url: "https://world.test/new.hsml".into(),
            };
        }
        app.update();
        assert_eq!(camera(&mut app).translation.x, 100.0);
        for state in app
            .world_mut()
            .resource_mut::<crate::IncludeLoadStates>()
            .0
            .values_mut()
        {
            *state = crate::IncludeLoadState::Loaded {
                url: "https://world.test/main.hsml".into(),
            };
        }
        {
            let dom = app.world().resource::<ElemenetWorld>();
            for attr in (&mut dom.0.write_storage::<Attrs>()).join() {
                if attr.0.contains_key("managed-by") {
                    attr.0.insert("visible".into(), "false".into());
                }
            }
        }
        app.update();
        assert_eq!(camera(&mut app).translation.x, 100.0);
    }

    #[test]
    fn arrival_uses_parent_transform_and_rejects_invalid_values() {
        let parent = Transform::from_xyz(8.0, 2.0, 3.0).with_rotation(Quat::from_rotation_y(1.0));
        let child = Transform::from_xyz(2.0, 0.0, 0.0);
        let (feet, yaw) = arrival_pose(GlobalTransform::from(parent) * child).unwrap();
        assert!(feet.distance(parent.transform_point(child.translation)) < 1e-5);
        assert!((yaw - 1.0).abs() < 1e-5);
        assert!(arrival_pose(GlobalTransform::from_translation(Vec3::splat(f32::NAN))).is_none());
    }

    #[test]
    fn vr_arrival_preserves_physical_height_and_aligns_head_not_room_origin() {
        let mut root = Transform::from_xyz(30.0, 0.0, 40.0);
        let head = Vec3::new(2.0, 1.62, -3.0);
        let head_rotation = Quat::from_euler(EulerRot::YXZ, 0.7, 0.2, 0.1);
        let feet = Vec3::new(-8.0, 5.0, 9.0);
        place_tracking_root(&mut root, head, head_rotation, feet, -1.2);
        assert!(root.transform_point(head).distance(feet + Vec3::Y * head.y) < 1e-5);
        let forward = root.rotation * head_rotation * Vec3::NEG_Z;
        assert!(((-forward.x).atan2(-forward.z) + 1.2).abs() < 1e-5);
    }

    #[test]
    fn entry_fragment_decodes_names() {
        assert_eq!(
            entry_from_url("https://a.test/world?seed=2#entry=puerta%20sur"),
            Some("puerta sur".into())
        );
        assert_eq!(entry_from_url("https://a.test/world?entry=ignored"), None);
    }

    #[test]
    fn failed_global_navigation_does_not_respawn_but_replaced_document_does() {
        let mut app = fixture("spatial", "spawn", "spawn", "<spawn x='4'/>");
        app.update();
        app.world_mut()
            .query_filtered::<&mut Transform, With<DesktopCamera>>()
            .single_mut(app.world_mut())
            .translation
            .x = 50.0;
        app.world_mut().resource_mut::<crate::DocumentLoadState>().0 =
            Some(crate::ActiveDocumentLoad {
                epoch: 2,
                url: "https://world.test/unavailable.hsml".into(),
            });
        app.update();
        assert_eq!(camera(&mut app).translation.x, 50.0);
        app.world_mut().resource_mut::<crate::DocumentLoadState>().0 = None;
        app.update();
        assert_eq!(camera(&mut app).translation.x, 50.0);

        let (document, old_entity) = app
            .world()
            .resource::<SpawnState>()
            .applied
            .iter()
            .map(|(doc, entity)| (*doc, *entity))
            .next()
            .unwrap();
        app.world_mut().despawn(old_entity);
        let replacement = app.world_mut().spawn(SpatialBundle::default()).id();
        app.world_mut()
            .resource_mut::<crate::EntityMap>()
            .0
            .insert(document.id(), replacement);
        app.update();
        assert_eq!(camera(&mut app).translation.x, 4.0);
    }

    #[test]
    fn named_entry_overrides_default_after_load_commits() {
        let mut app = fixture(
            "spatial",
            "spawn",
            "spawn",
            "<spawn default='true' x='2'/><spawn id='sur' x='8'/>",
        );
        let url = "https://world.test/main.hsml#entry=sur";
        {
            let dom = app.world().resource::<ElemenetWorld>();
            for attr in (&mut dom.0.write_storage::<Attrs>()).join() {
                if attr.0.get("id").is_some_and(|id| id == "mount") {
                    attr.0.insert("src".into(), url.into());
                }
            }
        }
        // Old Loaded state is not enough for the new address.
        app.update();
        assert_eq!(camera(&mut app).translation.x, 100.0);
        for state in app
            .world_mut()
            .resource_mut::<crate::IncludeLoadStates>()
            .0
            .values_mut()
        {
            *state = crate::IncludeLoadState::Loaded { url: url.into() };
        }
        app.update();
        assert_eq!(camera(&mut app).translation.x, 8.0);
    }
}
