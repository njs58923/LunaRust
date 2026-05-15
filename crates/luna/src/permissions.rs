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
        const READ_POSE_STREAM     = 1 << 2;
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
        const SKYBOX               = 1 << 15;
        const READ_SYSTEM_INPUT    = 1 << 16;
        const UX_EMBED             = 1 << 17;
    }
}

/// Permissions "elevadas" — requieren confirmación explícita del usuario.
/// Por ahora sólo concedidas a spaces `managed-by="dimension.luna"` (UX shell).
/// TODO(perm-prompt): cuando exista el UX de prompt de permisos, las apps no
/// trusted que las pidan deberían disparar un alert al usuario en vez de denegar.
pub const ELEVATED_CAPABILITIES: CapabilityBits = CapabilityBits::READ_SYSTEM_INPUT
    .union(CapabilityBits::MOUNT_ROOT_SPACE)
    .union(CapabilityBits::UNMOUNT_ROOT_SPACE)
    .union(CapabilityBits::UPDATE_ROOT_SPACE)
    .union(CapabilityBits::LIST_ROOT_SPACES)
    .union(CapabilityBits::UX_EMBED);

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub struct NativeServiceBits: u32 {
        const DESKTOP_CAMERA_CONTROL = 1 << 0;
        const VR_LOCOMOTION          = 1 << 1;
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
            "read_pose_stream",
            ResourceBundleDef {
                capabilities: CapabilityBits::READ_POSE_STREAM,
                native_services: NativeServiceBits::empty(),
                auto_scripts: &[],
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

        m.insert(
            "skybox",
            ResourceBundleDef {
                capabilities: CapabilityBits::SKYBOX,
                native_services: NativeServiceBits::empty(),
                auto_scripts: &[],
            },
        );

        m.insert(
            "read_system_input",
            ResourceBundleDef {
                capabilities: CapabilityBits::READ_SYSTEM_INPUT,
                native_services: NativeServiceBits::empty(),
                auto_scripts: &[],
            },
        );

        // Embedded app API — el shell concede esta cap a tabs montadas con
        // `kind: 'app-embedded'`. Otorga `dimention.embedded.*` (request slot,
        // close self, eventos del shell). ELEVATED — solo managed-by trusted.
        m.insert(
            "ux_embed",
            ResourceBundleDef {
                capabilities: CapabilityBits::UX_EMBED,
                native_services: NativeServiceBits::empty(),
                auto_scripts: &["luna://internal/embedded_api.js"],
            },
        );

        // API chrome.tabs-like — sólo para spaces UX shell (managed-by=dimension.luna).
        // Bundle agrupa las 4 caps de mount/unmount/update/list root spaces +
        // auto-inject del script que expone dimention.tabs.* en el isolate.
        m.insert(
            "manage_tabs",
            ResourceBundleDef {
                capabilities: CapabilityBits::MOUNT_ROOT_SPACE
                    .union(CapabilityBits::UNMOUNT_ROOT_SPACE)
                    .union(CapabilityBits::UPDATE_ROOT_SPACE)
                    .union(CapabilityBits::LIST_ROOT_SPACES),
                native_services: NativeServiceBits::empty(),
                auto_scripts: &["luna://internal/tabs_api.js"],
            },
        );

        // Pose del usuario (HMD en VR, cámara en desktop) — vía dimention.readViewerPose().
        // Cap regular (no elevada). Apps pueden pedirla; UX shell la usa para
        // posicionar el panel al frente al hacer toggle.
        m.insert(
            "read_hmd_pose",
            ResourceBundleDef {
                capabilities: CapabilityBits::READ_HMD_POSE,
                native_services: NativeServiceBits::empty(),
                auto_scripts: &["luna://internal/viewer_pose_api.js"],
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

pub fn capability_labels(bits: CapabilityBits) -> Vec<&'static str> {
    [
        ("ROOT", CapabilityBits::ROOT),
        ("READ_TOQUE_RAW", CapabilityBits::READ_TOQUE_RAW),
        ("READ_POSE_STREAM", CapabilityBits::READ_POSE_STREAM),
        ("NAVIGATE_SELF", CapabilityBits::NAVIGATE_SELF),
        ("NAVIGATE_GLOBAL", CapabilityBits::NAVIGATE_GLOBAL),
        ("FETCH_TEXT", CapabilityBits::FETCH_TEXT),
        ("LIST_ROOT_SPACES", CapabilityBits::LIST_ROOT_SPACES),
        ("MOUNT_ROOT_SPACE", CapabilityBits::MOUNT_ROOT_SPACE),
        ("UPDATE_ROOT_SPACE", CapabilityBits::UPDATE_ROOT_SPACE),
        ("UNMOUNT_ROOT_SPACE", CapabilityBits::UNMOUNT_ROOT_SPACE),
        ("READ_CAMERA_POSE", CapabilityBits::READ_CAMERA_POSE),
        ("READ_HMD_POSE", CapabilityBits::READ_HMD_POSE),
        ("READ_CONTROLLER_POSE", CapabilityBits::READ_CONTROLLER_POSE),
        ("DEVTOOLS_READ", CapabilityBits::DEVTOOLS_READ),
        ("DEVTOOLS_WRITE", CapabilityBits::DEVTOOLS_WRITE),
        ("SKYBOX", CapabilityBits::SKYBOX),
        ("READ_SYSTEM_INPUT", CapabilityBits::READ_SYSTEM_INPUT),
        ("UX_EMBED", CapabilityBits::UX_EMBED),
    ]
    .iter()
    .filter(|(_, flag)| bits.contains(*flag))
    .map(|(label, _)| *label)
    .collect()
}

pub fn native_service_labels(bits: NativeServiceBits) -> Vec<&'static str> {
    [
        ("DESKTOP_CAMERA_CONTROL", NativeServiceBits::DESKTOP_CAMERA_CONTROL),
        ("VR_LOCOMOTION", NativeServiceBits::VR_LOCOMOTION),
    ]
    .iter()
    .filter(|(_, flag)| bits.contains(*flag))
    .map(|(label, _)| *label)
    .collect()
}

pub fn describe_resource_tokens(tokens: &[String]) -> Vec<String> {
    tokens
        .iter()
        .filter_map(|token| {
            RESOURCE_BUNDLES.get(token.as_str()).map(|bundle| {
                let caps = capability_labels(bundle.capabilities);
                let native = native_service_labels(bundle.native_services);
                let mut parts = vec![token.to_string()];
                if !caps.is_empty() {
                    parts.push(format!("caps:[{}]", caps.join(",")));
                }
                if !native.is_empty() {
                    parts.push(format!("native:[{}]", native.join(",")));
                }
                if !bundle.auto_scripts.is_empty() {
                    parts.push(format!("auto_scripts:[{}]", bundle.auto_scripts.join(",")));
                }
                parts.join(" -> ")
            })
        })
        .collect()
}

pub fn describe_capability_bits(bits: CapabilityBits) -> String {
    let labels = capability_labels(bits);
    if labels.is_empty() {
        "none".to_string()
    } else {
        labels.join(" | ")
    }
}

pub fn describe_native_service_bits(bits: NativeServiceBits) -> String {
    let labels = native_service_labels(bits);
    if labels.is_empty() {
        "none".to_string()
    } else {
        labels.join(" | ")
    }
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

#[inline]
fn intersect_requested_or_take_entry(
    requested_resources: &[String],
    requested_caps: CapabilityBits,
    entry_caps: CapabilityBits,
) -> CapabilityBits {
    if entry_caps.is_empty() {
        return CapabilityBits::empty();
    }
    if requested_resources.is_empty() {
        entry_caps
    } else {
        requested_caps & entry_caps
    }
}

#[inline]
fn intersect_requested_or_take_entry_native(
    requested_resources: &[String],
    requested_native: NativeServiceBits,
    entry_native: NativeServiceBits,
) -> NativeServiceBits {
    if entry_native.is_empty() {
        return NativeServiceBits::empty();
    }
    if requested_resources.is_empty() {
        entry_native
    } else {
        requested_native & entry_native
    }
}

fn is_root_space(attrs: Option<&HashMap<String, String>>) -> bool {
    let Some(attrs) = attrs else {
        return false;
    };

    attrs.get("system-space").map(|v| v.as_str()) == Some("root")
        || attrs.get("id").map(|v| v.as_str()) == Some("luna_root")
}

/// El space declara ser parte del UX shell trusted (montado por dimension.luna).
fn is_dimension_luna_managed(attrs: Option<&HashMap<String, String>>) -> bool {
    let Some(attrs) = attrs else { return false };
    attrs.get("managed-by").map(|v| v.as_str()) == Some("dimension.luna")
}

/// Filtra capabilities elevadas si el space no es trusted UX shell.
/// TODO(perm-prompt): en el futuro, si una cap elevada se solicita por una app
/// no-managed, en lugar de filtrarla aquí, encolar una solicitud a la UX para
/// que pregunte al usuario via alert/confirm.
fn gate_elevated_capabilities(
    requested_caps: CapabilityBits,
    is_managed: bool,
    is_root: bool,
) -> CapabilityBits {
    if is_managed || is_root {
        return requested_caps;
    }
    requested_caps & !ELEVATED_CAPABILITIES
}

pub fn space_has_capability(space_id: u32, cap: CapabilityBits, policies: &SpacePolicies) -> bool {
    policies
        .by_space
        .get(&space_id)
        .map(|policy| policy.effective_caps.contains(cap))
        .unwrap_or(false)
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
    dom_data: Res<crate::VirtualDomData>,
    mut policies: ResMut<SpacePolicies>,
    mut log_panel: ResMut<crate::LogPanel>,
    mut history: ResMut<SpacePolicyHistory>,
) {
    if !policies.dirty {
        return;
    }

    let entities = world.0.entities();
    let hierarchies = world.0.read_storage::<Hierarchy>();
    let tags = world.0.read_storage::<Tag>();
    let attrs = world.0.read_storage::<Attrs>();
    let attached_ids: HashSet<u32> = dom_data.nodes.keys().copied().collect();

    fn walk(
        ent: specs::Entity,
        inherited_caps: CapabilityBits,
        inherited_native: NativeServiceBits,
        entry_caps: CapabilityBits,
        entry_native: NativeServiceBits,
        attached_ids: &HashSet<u32>,
        hierarchies: &specs::ReadStorage<Hierarchy>,
        tags: &specs::ReadStorage<Tag>,
        attrs: &specs::ReadStorage<Attrs>,
        out: &mut HashMap<u32, SpacePolicy>,
    ) {
        let tag_name = tags.get(ent).map(|t| t.0.as_str()).unwrap_or("");
        let attrs_map = attrs.get(ent).map(|a| &a.0);

        let mut child_caps = inherited_caps;
        let mut child_native = inherited_native;
        let mut child_entry_caps = entry_caps;
        let mut child_entry_native = entry_native;

        if tag_name == "include" {
            if let Some(raw) = attrs_map.and_then(|m| m.get("resources")) {
                let tokens = parse_resource_tokens(raw);
                let (grant_caps, grant_native, _) = resolve_resource_set(&tokens);
                // Los includes NO transmiten permisos por defecto.
                // Si declaran resources, generan un "entry grant" para el
                // documento cargado bajo ese include.
                child_caps = CapabilityBits::empty();
                child_native = NativeServiceBits::empty();
                child_entry_caps = inherited_caps & grant_caps;
                child_entry_native = inherited_native & grant_native;
            } else {
                // include sin resources => no pasa nada al contenido hijo
                child_caps = CapabilityBits::empty();
                child_native = NativeServiceBits::empty();
                child_entry_caps = CapabilityBits::empty();
                child_entry_native = NativeServiceBits::empty();
            }
        }

        if tag_name == "space" {
            let requested_resources = attrs_map
                .and_then(|m| m.get("resources"))
                .map(|raw| parse_resource_tokens(raw))
                .unwrap_or_default();

            let (requested_caps_raw, requested_native, auto_scripts) =
                resolve_resource_set(&requested_resources);

            let root_space = is_root_space(attrs_map);
            let managed = is_dimension_luna_managed(attrs_map);
            // Caps elevadas: sólo otorgables a root, spaces managed-by=dimension.luna,
            // o spaces cuyo padre managed las haya propagado vía entry_caps.
            let requested_caps =
                gate_elevated_capabilities(requested_caps_raw, managed, root_space);

            let effective_caps = if root_space {
                if requested_caps.is_empty() {
                    CapabilityBits::all()
                } else {
                    requested_caps
                }
            } else if !entry_caps.is_empty() {
                // El entry_caps ya fue gated en su origen (sólo se propaga si el
                // padre era managed). Usamos requested_caps_raw aquí para permitir
                // que caps elevadas heredadas crucen — el gate de entrada las protege.
                intersect_requested_or_take_entry(
                    &requested_resources,
                    requested_caps_raw,
                    entry_caps,
                )
            } else {
                requested_caps & inherited_caps
            };
            let effective_native = if root_space {
                requested_native
            } else if !entry_native.is_empty() {
                intersect_requested_or_take_entry_native(
                    &requested_resources,
                    requested_native,
                    entry_native,
                )
            } else {
                requested_native & inherited_native
            };

            let grant_ceiling = if root_space {
                CapabilityBits::all()
            } else {
                inherited_caps
            };

            let grant_native_ceiling = if root_space {
                NativeServiceBits::all()
            } else {
                inherited_native
            };

            out.insert(
                ent.id(),
                SpacePolicy {
                    requested_resources,
                    requested_caps,
                    requested_native,
                    grant_ceiling,
                    grant_native_ceiling,
                    effective_caps,
                    effective_native,
                    auto_scripts,
                },
            );

            child_caps = effective_caps;
            child_native = if root_space {
                NativeServiceBits::all()
            } else {
                effective_native
            };

            // El entry grant se consume en este space y no sigue heredándose.
            child_entry_caps = CapabilityBits::empty();
            child_entry_native = NativeServiceBits::empty();
        }

        if let Some(h) = hierarchies.get(ent) {
            for &child in &h.children {
                if !attached_ids.contains(&child.id()) {
                    continue;
                }
                walk(
                    child,
                    child_caps,
                    child_native,
                    child_entry_caps,
                    child_entry_native,
                    attached_ids,
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
        .filter(|(ent, _)| attached_ids.contains(&ent.id()))
        .filter(|(_, h)| h.parent.is_none())
        .map(|(ent, _)| ent)
        .collect();

    let mut next = HashMap::new();
    for root in roots {
        walk(
            root,
            CapabilityBits::empty(),
            NativeServiceBits::empty(),
            CapabilityBits::empty(),
            NativeServiceBits::empty(),
            &attached_ids,
            &hierarchies,
            &tags,
            &attrs,
            &mut next,
        );
    }

    let old_gen = policies.generation;
    policies.by_space = next;
    policies.dirty = false;
    policies.generation += 1;

    // Log changes and save snapshots
    for (space_id, policy) in policies.by_space.iter() {
        let snapshot = SpacePolicySnapshotEntry {
            generation: policies.generation,
            space_id: *space_id,
            requested_resources: policy.requested_resources.clone(),
            effective_caps: policy.effective_caps,
            effective_native: policy.effective_native,
            auto_scripts: policy.auto_scripts.clone(),
        };

        history.entries.push(snapshot.clone());
        if history.entries.len() > history.max_entries {
            history.entries.remove(0);
        }

        log_panel.push_info(format!(
            "[perm][space:{}] requested=[{}] effective_caps=[{}] effective_native=[{}]",
            space_id,
            policy.requested_resources.join(","),
            describe_capability_bits(policy.effective_caps),
            describe_native_service_bits(policy.effective_native)
        ));
    }

    if policies.generation != old_gen {
        log_panel.push_info(format!(
            "[perm] generation={} rebuilt spaces={}",
            policies.generation,
            policies.by_space.len()
        ));
    }
}

pub fn update_active_native_services_system(
    policies: Res<SpacePolicies>,
    mut active: ResMut<ActiveNativeServices>,
    mut log_panel: ResMut<crate::LogPanel>,
) {
    let mut enabled = NativeServiceBits::empty();
    for policy in policies.by_space.values() {
        enabled |= policy.effective_native;
    }

    if active.0 != enabled {
        active.0 = enabled;
        log_panel.push_info(format!(
            "[perm] active native services = {}",
            describe_native_service_bits(enabled)
        ));
    }
}

#[derive(Debug, Clone)]
pub struct SpacePolicySnapshotEntry {
    pub generation: u64,
    pub space_id: u32,
    pub requested_resources: Vec<String>,
    pub effective_caps: CapabilityBits,
    pub effective_native: NativeServiceBits,
    pub auto_scripts: Vec<String>,
}

#[derive(Resource, Default)]
pub struct SpacePolicyHistory {
    pub entries: Vec<SpacePolicySnapshotEntry>,
    pub max_entries: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::prelude::App;
    use specs::{Join, WorldExt};
    use virtual_dom::{dom::element::build_world, parse_xml};

    fn collect_attached_nodes(world: &specs::World, root: specs::Entity) -> HashMap<u32, specs::Entity> {
        let mut out = HashMap::new();
        let mut stack = vec![root];
        let hier = world.read_storage::<Hierarchy>();
        while let Some(ent) = stack.pop() {
            out.insert(ent.id(), ent);
            if let Some(node) = hier.get(ent) {
                for &child in &node.children {
                    stack.push(child);
                }
            }
        }
        out
    }

    fn build_app_with_xml(xml: &str) -> App {
        let mut specs_world = build_world();
        let root = parse_xml(&mut specs_world, xml).expect("xml parse failed");
        let dom_nodes = collect_attached_nodes(&specs_world, root);

        let mut app = App::new();
        app.insert_resource(crate::ElemenetWorld(specs_world));
        app.insert_resource(crate::VirtualDomData { nodes: dom_nodes });
        app.insert_resource(SpacePolicies::default());
        app.insert_resource(crate::LogPanel::default());
        app.insert_resource(SpacePolicyHistory {
            entries: Vec::new(),
            max_entries: 32,
        });
        app.add_systems(Update, rebuild_space_policies_system);
        app
    }

    fn find_space_id_by_attr_id(world: &crate::ElemenetWorld, attr_id: &str) -> u32 {
        let entities = world.0.entities();
        let tags = world.0.read_storage::<Tag>();
        let attrs = world.0.read_storage::<Attrs>();

        (&entities, &tags, &attrs)
            .join()
            .find_map(|(ent, tag, attrs)| {
                (tag.0 == "space" && attrs.0.get("id").map(|s| s.as_str()) == Some(attr_id))
                    .then_some(ent.id())
            })
            .expect("space id attr not found")
    }

    #[test]
    fn top_tab_include_grants_navigate_self_only_to_document_root_space() {
        let xml = r#"
        <hsml>
          <space id="luna_root" system-space="root" resources="root">
            <space id="tab_wrapper" managed-by="dimension.luna" resources="navigate_self">
              <include resources="navigate_self">
                <hsml>
                  <space id="doc_root">
                    <include>
                      <hsml>
                        <space id="nested_doc_root" />
                      </hsml>
                    </include>
                  </space>
                </hsml>
              </include>
            </space>
          </space>
        </hsml>
        "#;

        let mut app = build_app_with_xml(xml);
        app.update();

        let world = app.world().resource::<crate::ElemenetWorld>();
        let policies = app.world().resource::<SpacePolicies>();

        let wrapper_id = find_space_id_by_attr_id(world, "tab_wrapper");
        let doc_root_id = find_space_id_by_attr_id(world, "doc_root");
        let nested_doc_root_id = find_space_id_by_attr_id(world, "nested_doc_root");

        assert!(policies
            .by_space
            .get(&wrapper_id)
            .unwrap()
            .effective_caps
            .contains(CapabilityBits::NAVIGATE_SELF));

        assert!(policies
            .by_space
            .get(&doc_root_id)
            .unwrap()
            .effective_caps
            .contains(CapabilityBits::NAVIGATE_SELF));

        assert!(!policies
            .by_space
            .get(&nested_doc_root_id)
            .unwrap()
            .effective_caps
            .contains(CapabilityBits::NAVIGATE_SELF));
    }

    #[test]
    fn nested_include_only_gets_navigate_self_when_repassed_explicitly() {
        let xml = r#"
        <hsml>
          <space id="luna_root" system-space="root" resources="root">
            <space id="tab_wrapper" managed-by="dimension.luna" resources="navigate_self">
              <include resources="navigate_self">
                <hsml>
                  <space id="doc_root">
                    <include resources="navigate_self">
                      <hsml>
                        <space id="nested_doc_root" />
                      </hsml>
                    </include>
                  </space>
                </hsml>
              </include>
            </space>
          </space>
        </hsml>
        "#;

        let mut app = build_app_with_xml(xml);
        app.update();

        let world = app.world().resource::<crate::ElemenetWorld>();
        let policies = app.world().resource::<SpacePolicies>();

        let doc_root_id = find_space_id_by_attr_id(world, "doc_root");
        let nested_doc_root_id = find_space_id_by_attr_id(world, "nested_doc_root");

        assert!(policies
            .by_space
            .get(&doc_root_id)
            .unwrap()
            .effective_caps
            .contains(CapabilityBits::NAVIGATE_SELF));

        assert!(policies
            .by_space
            .get(&nested_doc_root_id)
            .unwrap()
            .effective_caps
            .contains(CapabilityBits::NAVIGATE_SELF));
    }
}
