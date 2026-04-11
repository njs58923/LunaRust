use std::collections::{HashMap, HashSet};

use bevy::prelude::*;
use bitflags::bitflags;
use lazy_static::lazy_static;
use specs::{Join, WorldExt};
use virtual_dom::dom::element::{Attrs, Hierarchy, Tag};

use crate::ElemenetWorld;

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub struct CapabilityBits: u64 {
        const ROOT                 = 1 << 0;
        const READ_TOQUE_RAW       = 1 << 1;
        const DISPATCH_LOCAL_TOQUE = 1 << 2;
        const NAVIGATE_SELF        = 1 << 3;
        const NAVIGATE_GLOBAL      = 1 << 4;
        const FETCH_TEXT           = 1 << 5;
        const LIST_ROOT_SPACES     = 1 << 6;
        const MOUNT_ROOT_SPACE     = 1 << 7;
        const UPDATE_ROOT_SPACE    = 1 << 8;
        const UNMOUNT_ROOT_SPACE   = 1 << 9;
        const READ_CAMERA_POSE     = 1 << 10;
        const READ_HMD_POSE        = 1 << 11;
        const READ_CONTROLLER_POSE = 1 << 12;
        const DEVTOOLS_READ        = 1 << 13;
        const DEVTOOLS_WRITE       = 1 << 14;
    }
}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub struct NativeServiceBits: u32 {
        const DESKTOP_TOQUE_SOURCE   = 1 << 0;
        const VR_TOQUE_SOURCE        = 1 << 1;
        const DESKTOP_CAMERA_CONTROL = 1 << 2;
        const VR_LOCOMOTION          = 1 << 3;
    }
}

#[derive(Debug, Clone)]
pub struct ResourceBundleDef {
    pub capabilities: CapabilityBits,
    pub native_services: NativeServiceBits,
    pub auto_scripts: &'static [&'static str],
}

lazy_static! {
    static ref RESOURCE_BUNDLES: HashMap<&'static str, ResourceBundleDef> = {
        let mut m = HashMap::new();

        m.insert(
            "root",
            ResourceBundleDef {
                capabilities: CapabilityBits::all(),
                native_services: NativeServiceBits::empty(),
                auto_scripts: &["luna://internal/root_api.js"],
            },
        );

        m.insert(
            "controller_desktop",
            ResourceBundleDef {
                capabilities: CapabilityBits::READ_TOQUE_RAW | CapabilityBits::DISPATCH_LOCAL_TOQUE,
                native_services: NativeServiceBits::DESKTOP_TOQUE_SOURCE,
                auto_scripts: &["luna://internal/controller_toque.js"],
            },
        );

        m.insert(
            "controller_vr",
            ResourceBundleDef {
                capabilities: CapabilityBits::READ_TOQUE_RAW | CapabilityBits::DISPATCH_LOCAL_TOQUE,
                native_services: NativeServiceBits::VR_TOQUE_SOURCE,
                auto_scripts: &["luna://internal/controller_toque.js"],
            },
        );

        m.insert(
            "navigate_self",
            ResourceBundleDef {
                capabilities: CapabilityBits::NAVIGATE_SELF,
                native_services: NativeServiceBits::empty(),
                auto_scripts: &[],
            },
        );

        m.insert(
            "navigate_global",
            ResourceBundleDef {
                capabilities: CapabilityBits::NAVIGATE_GLOBAL,
                native_services: NativeServiceBits::empty(),
                auto_scripts: &[],
            },
        );

        m.insert(
            "fetch_text",
            ResourceBundleDef {
                capabilities: CapabilityBits::FETCH_TEXT,
                native_services: NativeServiceBits::empty(),
                auto_scripts: &[],
            },
        );

        m.insert(
            "desktop_camera_control",
            ResourceBundleDef {
                capabilities: CapabilityBits::empty(),
                native_services: NativeServiceBits::DESKTOP_CAMERA_CONTROL,
                auto_scripts: &[],
            },
        );

        m.insert(
            "vr_locomotion",
            ResourceBundleDef {
                capabilities: CapabilityBits::empty(),
                native_services: NativeServiceBits::VR_LOCOMOTION,
                auto_scripts: &[],
            },
        );

        m.insert(
            "list_root_spaces",
            ResourceBundleDef {
                capabilities: CapabilityBits::LIST_ROOT_SPACES,
                native_services: NativeServiceBits::empty(),
                auto_scripts: &[],
            },
        );

        m.insert(
            "mount_root_space",
            ResourceBundleDef {
                capabilities: CapabilityBits::MOUNT_ROOT_SPACE,
                native_services: NativeServiceBits::empty(),
                auto_scripts: &[],
            },
        );

        m.insert(
            "update_root_space",
            ResourceBundleDef {
                capabilities: CapabilityBits::UPDATE_ROOT_SPACE,
                native_services: NativeServiceBits::empty(),
                auto_scripts: &[],
            },
        );

        m.insert(
            "unmount_root_space",
            ResourceBundleDef {
                capabilities: CapabilityBits::UNMOUNT_ROOT_SPACE,
                native_services: NativeServiceBits::empty(),
                auto_scripts: &[],
            },
        );

        m
    };
}

#[derive(Debug, Clone, Default)]
pub struct SpacePolicy {
    pub requested_resources: Vec<String>,
    pub requested_caps: CapabilityBits,
    pub requested_native: NativeServiceBits,
    pub grant_ceiling: CapabilityBits,
    pub grant_native_ceiling: NativeServiceBits,
    pub effective_caps: CapabilityBits,
    pub effective_native: NativeServiceBits,
    pub auto_scripts: Vec<String>,
}

#[derive(Resource, Debug, Clone)]
pub struct SpacePolicies {
    pub by_space: HashMap<u32, SpacePolicy>,
    pub dirty: bool,
    pub generation: u64,
}

impl Default for SpacePolicies {
    fn default() -> Self {
        Self {
            by_space: HashMap::new(),
            dirty: true,
            generation: 0,
        }
    }
}

#[derive(Resource, Debug, Clone, Copy, Default)]
pub struct ActiveNativeServices(pub NativeServiceBits);

pub fn parse_resource_tokens(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect()
}

fn resolve_resource_set(tokens: &[String]) -> (CapabilityBits, NativeServiceBits, Vec<String>) {
    let mut caps = CapabilityBits::empty();
    let mut native = NativeServiceBits::empty();
    let mut auto_scripts = Vec::new();
    let mut seen_scripts = HashSet::new();

    for token in tokens {
        let Some(bundle) = RESOURCE_BUNDLES.get(token.as_str()) else {
            continue;
        };
        caps |= bundle.capabilities;
        native |= bundle.native_services;
        for &script in bundle.auto_scripts {
            if seen_scripts.insert(script) {
                auto_scripts.push(script.to_string());
            }
        }
    }

    (caps, native, auto_scripts)
}

fn is_root_space(attrs: Option<&HashMap<String, String>>) -> bool {
    let Some(attrs) = attrs else {
        return false;
    };

    attrs.get("system-space").map(|v| v.as_str()) == Some("root")
        || attrs.get("id").map(|v| v.as_str()) == Some("luna_root")
}

pub fn space_has_capability(space_id: u32, cap: CapabilityBits, policies: &SpacePolicies) -> bool {
    policies
        .by_space
        .get(&space_id)
        .map(|policy| policy.effective_caps.contains(cap))
        .unwrap_or(false)
}

pub fn desktop_toque_source_enabled(active: Res<ActiveNativeServices>) -> bool {
    active.0.contains(NativeServiceBits::DESKTOP_TOQUE_SOURCE)
}

pub fn vr_toque_source_enabled(active: Res<ActiveNativeServices>) -> bool {
    active.0.contains(NativeServiceBits::VR_TOQUE_SOURCE)
}

pub fn desktop_camera_control_enabled(active: Res<ActiveNativeServices>) -> bool {
    active
        .0
        .contains(NativeServiceBits::DESKTOP_CAMERA_CONTROL)
}

pub fn vr_locomotion_enabled(active: Res<ActiveNativeServices>) -> bool {
    active.0.contains(NativeServiceBits::VR_LOCOMOTION)
}

pub fn rebuild_space_policies_system(
    world: Res<ElemenetWorld>,
    mut policies: ResMut<SpacePolicies>,
) {
    if !policies.dirty {
        return;
    }

    let entities = world.0.entities();
    let hierarchies = world.0.read_storage::<Hierarchy>();
    let tags = world.0.read_storage::<Tag>();
    let attrs = world.0.read_storage::<Attrs>();

    fn walk(
        ent: specs::Entity,
        inherited_caps: CapabilityBits,
        inherited_native: NativeServiceBits,
        hierarchies: &specs::ReadStorage<Hierarchy>,
        tags: &specs::ReadStorage<Tag>,
        attrs: &specs::ReadStorage<Attrs>,
        out: &mut HashMap<u32, SpacePolicy>,
    ) {
        let tag_name = tags.get(ent).map(|t| t.0.as_str()).unwrap_or("");
        let attrs_map = attrs.get(ent).map(|a| &a.0);

        let mut child_caps = inherited_caps;
        let mut child_native = inherited_native;

        if tag_name == "include" {
            if let Some(raw) = attrs_map.and_then(|m| m.get("resources")) {
                let tokens = parse_resource_tokens(raw);
                let (grant_caps, grant_native, _) = resolve_resource_set(&tokens);
                child_caps = inherited_caps & grant_caps;
                child_native = inherited_native & grant_native;
            }
        }

        if tag_name == "space" {
            let requested_resources = attrs_map
                .and_then(|m| m.get("resources"))
                .map(|raw| parse_resource_tokens(raw))
                .unwrap_or_default();

            let (requested_caps, requested_native, auto_scripts) =
                resolve_resource_set(&requested_resources);

            let root_space = is_root_space(attrs_map);
            let effective_caps = if root_space {
                if requested_caps.is_empty() {
                    CapabilityBits::all()
                } else {
                    requested_caps
                }
            } else {
                requested_caps & inherited_caps
            };
            let effective_native = if root_space {
                requested_native
            } else {
                requested_native & inherited_native
            };

            out.insert(
                ent.id(),
                SpacePolicy {
                    requested_resources,
                    requested_caps,
                    requested_native,
                    grant_ceiling: inherited_caps,
                    grant_native_ceiling: inherited_native,
                    effective_caps,
                    effective_native,
                    auto_scripts,
                },
            );

            child_caps = effective_caps;
            child_native = effective_native;
        }

        if let Some(h) = hierarchies.get(ent) {
            for &child in &h.children {
                walk(
                    child,
                    child_caps,
                    child_native,
                    hierarchies,
                    tags,
                    attrs,
                    out,
                );
            }
        }
    }

    let roots: Vec<_> = (&entities, &hierarchies)
        .join()
        .filter(|(_, h)| h.parent.is_none())
        .map(|(ent, _)| ent)
        .collect();

    let mut next = HashMap::new();
    for root in roots {
        walk(
            root,
            CapabilityBits::empty(),
            NativeServiceBits::empty(),
            &hierarchies,
            &tags,
            &attrs,
            &mut next,
        );
    }

    policies.by_space = next;
    policies.dirty = false;
    policies.generation += 1;
}

pub fn update_active_native_services_system(
    policies: Res<SpacePolicies>,
    mut active: ResMut<ActiveNativeServices>,
) {
    let mut enabled = NativeServiceBits::empty();
    for policy in policies.by_space.values() {
        enabled |= policy.effective_native;
    }

    if active.0 != enabled {
        active.0 = enabled;
    }
}
