use std::{
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
    fs,
    path::{Path, PathBuf},
};

use bevy::prelude::*;
use bitflags::bitflags;
use lazy_static::lazy_static;
use serde::{Deserialize, Serialize};
use specs::{Join, WorldExt};
use url::Url;
use virtual_dom::dom::element::{Attrs, BaseUrl, Hierarchy, Tag};

use crate::{CurrentUrl, ElemenetWorld};

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
        /// Capturar el frame renderizado a un archivo. Es lectura de pantalla:
        /// va elevada para que un origen remoto no pueda pedirla sin consenso.
        const CAPTURE_FRAME        = 1 << 18;
        const SPAWN                = 1 << 19;
    }
}

/// Permissions "elevadas" — requieren confirmación explícita del usuario,
/// salvo para el root y spaces trusted del UX shell.
pub const ELEVATED_CAPABILITIES: CapabilityBits = CapabilityBits::READ_SYSTEM_INPUT
    .union(CapabilityBits::MOUNT_ROOT_SPACE)
    .union(CapabilityBits::UNMOUNT_ROOT_SPACE)
    .union(CapabilityBits::UPDATE_ROOT_SPACE)
    .union(CapabilityBits::LIST_ROOT_SPACES)
    .union(CapabilityBits::UX_EMBED)
    .union(CapabilityBits::CAPTURE_FRAME);

const ELEVATED_CAPABILITY_DEFS: [(&str, CapabilityBits); 7] = [
    ("READ_SYSTEM_INPUT", CapabilityBits::READ_SYSTEM_INPUT),
    ("MOUNT_ROOT_SPACE", CapabilityBits::MOUNT_ROOT_SPACE),
    ("UNMOUNT_ROOT_SPACE", CapabilityBits::UNMOUNT_ROOT_SPACE),
    ("UPDATE_ROOT_SPACE", CapabilityBits::UPDATE_ROOT_SPACE),
    ("LIST_ROOT_SPACES", CapabilityBits::LIST_ROOT_SPACES),
    ("UX_EMBED", CapabilityBits::UX_EMBED),
    ("CAPTURE_FRAME", CapabilityBits::CAPTURE_FRAME),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionDecision {
    Allow,
    Deny,
}

#[derive(Resource, Debug, Clone, Default, Serialize, Deserialize)]
pub struct PermissionDecisionStore {
    #[serde(default)]
    decisions: BTreeMap<String, BTreeMap<String, PermissionDecision>>,
}

impl PermissionDecisionStore {
    pub fn path() -> PathBuf {
        crate::utils::folder::resolve_permission_decisions_path()
    }

    pub fn load() -> Self {
        Self::load_from(&Self::path()).unwrap_or_default()
    }

    fn load_from(path: &Path) -> Result<Self, String> {
        let contents = fs::read_to_string(path)
            .map_err(|err| format!("Read permission decisions failed: {err}"))?;
        serde_json::from_str(&contents)
            .map_err(|err| format!("Parse permission decisions failed: {err}"))
    }

    pub fn save(&self) -> Result<PathBuf, String> {
        let path = Self::path();
        self.save_to(&path)?;
        Ok(path)
    }

    fn save_to(&self, path: &Path) -> Result<(), String> {
        let parent = path
            .parent()
            .ok_or_else(|| format!("Invalid permission decisions path: {}", path.display()))?;
        fs::create_dir_all(parent)
            .map_err(|err| format!("Create permission decisions dir failed: {err}"))?;
        let json = serde_json::to_string_pretty(self)
            .map_err(|err| format!("Serialize permission decisions failed: {err}"))?;
        fs::write(path, json)
            .map_err(|err| format!("Write permission decisions failed: {err}"))
    }

    pub fn decision(
        &self,
        origin: &str,
        capability: CapabilityBits,
    ) -> Option<PermissionDecision> {
        let key = elevated_capability_key(capability)?;
        self.decisions
            .get(origin)
            .and_then(|by_capability| by_capability.get(key))
            .copied()
    }

    pub fn set_decision(
        &mut self,
        origin: String,
        capability: CapabilityBits,
        decision: PermissionDecision,
    ) -> bool {
        let Some(key) = elevated_capability_key(capability) else {
            return false;
        };
        self.decisions
            .entry(origin)
            .or_default()
            .insert(key.to_string(), decision)
            != Some(decision)
    }

    pub fn forget_decision(&mut self, origin: &str, capability: CapabilityBits) -> bool {
        let Some(key) = elevated_capability_key(capability) else {
            return false;
        };
        let Some(by_capability) = self.decisions.get_mut(origin) else {
            return false;
        };
        let removed = by_capability.remove(key).is_some();
        if by_capability.is_empty() {
            self.decisions.remove(origin);
        }
        removed
    }

    pub fn entries(&self) -> Vec<(String, CapabilityBits, PermissionDecision)> {
        let mut entries = Vec::new();
        for (origin, by_capability) in &self.decisions {
            for (key, decision) in by_capability {
                if let Some(capability) = elevated_capability_from_key(key) {
                    entries.push((origin.clone(), capability, *decision));
                }
            }
        }
        entries
    }
}

#[derive(Debug, Clone)]
pub struct PermissionPrompt {
    pub origin: String,
    pub capability: CapabilityBits,
    pub space_ids: Vec<u32>,
}

impl PermissionPrompt {
    pub fn capability_label(&self) -> &'static str {
        elevated_capability_key(self.capability).unwrap_or("UNKNOWN")
    }
}

#[derive(Resource, Debug, Default)]
pub struct PermissionPromptQueue {
    pub pending: VecDeque<PermissionPrompt>,
}

impl PermissionPromptQueue {
    pub fn resolve(&mut self, origin: &str, capability: CapabilityBits) {
        self.pending
            .retain(|prompt| prompt.origin != origin || prompt.capability != capability);
    }

    fn reconcile(&mut self, requested: HashMap<(String, u64), HashSet<u32>>) {
        self.pending.retain_mut(|prompt| {
            let key = (prompt.origin.clone(), prompt.capability.bits());
            let Some(space_ids) = requested.get(&key) else {
                return false;
            };
            prompt.space_ids = space_ids.iter().copied().collect();
            prompt.space_ids.sort_unstable();
            true
        });

        let existing: HashSet<(String, u64)> = self
            .pending
            .iter()
            .map(|prompt| (prompt.origin.clone(), prompt.capability.bits()))
            .collect();
        let mut missing = requested
            .into_iter()
            .filter(|(key, _)| !existing.contains(key))
            .collect::<Vec<_>>();
        missing.sort_by(|((origin_a, bits_a), _), ((origin_b, bits_b), _)| {
            origin_a.cmp(origin_b).then(bits_a.cmp(bits_b))
        });
        for ((origin, bits), space_ids) in missing {
            let mut space_ids = space_ids.into_iter().collect::<Vec<_>>();
            space_ids.sort_unstable();
            self.pending.push_back(PermissionPrompt {
                origin,
                capability: CapabilityBits::from_bits_retain(bits),
                space_ids,
            });
        }
    }
}

fn elevated_capability_key(capability: CapabilityBits) -> Option<&'static str> {
    ELEVATED_CAPABILITY_DEFS
        .iter()
        .find_map(|(key, bit)| (*bit == capability).then_some(*key))
}

fn elevated_capability_from_key(key: &str) -> Option<CapabilityBits> {
    ELEVATED_CAPABILITY_DEFS
        .iter()
        .find_map(|(candidate, bit)| (*candidate == key).then_some(*bit))
}

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
            "capture_frame",
            ResourceBundleDef {
                capabilities: CapabilityBits::CAPTURE_FRAME,
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

        m.insert("spawn", ResourceBundleDef {
            capabilities: CapabilityBits::SPAWN,
            native_services: NativeServiceBits::empty(),
            auto_scripts: &[],
        });

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

        // Sólo la posición del visitante, sin hacia dónde mira. No es una
        // capacidad menor de READ_HMD_POSE por comodidad: es que la posición
        // ya se entrega con `read_pose_stream`, porque la pose de los mandos
        // ubica al jugador con medio metro de error. Gatearla no protegía
        // nada y sí rompía escritorio, donde no hay mandos. Lo que sigue
        // detrás de `read_hmd_pose` es la mirada, que es el dato sensible de
        // verdad y que ningún mando revela.
        m.insert(
            "read_camera_pose",
            ResourceBundleDef {
                capabilities: CapabilityBits::READ_CAMERA_POSE,
                native_services: NativeServiceBits::empty(),
                auto_scripts: &["luna://internal/viewer_pose_api.js"],
            },
        );

        m
    };
}

#[derive(Debug, Clone, Default)]
pub struct SpacePolicy {
    pub origin: String,
    pub requested_resources: Vec<String>,
    pub requested_caps: CapabilityBits,
    pub pending_elevated_caps: CapabilityBits,
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
        ("SPAWN", CapabilityBits::SPAWN),
        ("READ_SYSTEM_INPUT", CapabilityBits::READ_SYSTEM_INPUT),
        ("UX_EMBED", CapabilityBits::UX_EMBED),
        ("CAPTURE_FRAME", CapabilityBits::CAPTURE_FRAME),
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

fn resolve_effective_auto_scripts(
    tokens: &[String],
    effective_caps: CapabilityBits,
    effective_native: NativeServiceBits,
) -> Vec<String> {
    let mut auto_scripts = Vec::new();
    let mut seen_scripts = HashSet::new();
    for token in tokens {
        let Some(bundle) = RESOURCE_BUNDLES.get(token.as_str()) else {
            continue;
        };
        if !effective_caps.contains(bundle.capabilities)
            || !effective_native.contains(bundle.native_services)
        {
            continue;
        }
        for &script in bundle.auto_scripts {
            if seen_scripts.insert(script) {
                auto_scripts.push(script.to_string());
            }
        }
    }
    auto_scripts
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

fn normalize_permission_origin(raw_url: &str) -> String {
    let Ok(url) = Url::parse(raw_url) else {
        return raw_url.trim().to_string();
    };
    let serialized_origin = url.origin().ascii_serialization();
    if serialized_origin != "null" {
        return serialized_origin;
    }
    match url.host_str() {
        Some(host) => format!("{}://{}", url.scheme(), host),
        None => format!("{}://", url.scheme()),
    }
}

fn find_permission_origin(
    ent: specs::Entity,
    fallback_url: &str,
    hierarchies: &specs::ReadStorage<Hierarchy>,
    base_urls: &specs::ReadStorage<BaseUrl>,
) -> String {
    let mut current = Some(ent);
    while let Some(node) = current {
        if let Some(base_url) = base_urls.get(node) {
            return normalize_permission_origin(&base_url.0);
        }
        current = hierarchies
            .get(node)
            .and_then(|hierarchy| hierarchy.parent);
    }
    normalize_permission_origin(fallback_url)
}

fn approved_elevated_capabilities(
    origin: &str,
    requested: CapabilityBits,
    decisions: &PermissionDecisionStore,
) -> CapabilityBits {
    let mut approved = CapabilityBits::empty();
    for (_, capability) in ELEVATED_CAPABILITY_DEFS {
        if requested.contains(capability)
            && decisions.decision(origin, capability) == Some(PermissionDecision::Allow)
        {
            approved |= capability;
        }
    }
    approved
}

/// Filtra capabilities elevadas si el space no es trusted UX shell y todavía
/// no existe una aprobación persistida para su origen.
fn gate_elevated_capabilities(
    requested_caps: CapabilityBits,
    is_managed: bool,
    is_root: bool,
    approved_elevated: CapabilityBits,
) -> CapabilityBits {
    if is_managed || is_root {
        return requested_caps;
    }
    (requested_caps & !ELEVATED_CAPABILITIES) | (requested_caps & approved_elevated)
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
    current_url: Res<CurrentUrl>,
    decisions: Res<PermissionDecisionStore>,
    mut prompts: ResMut<PermissionPromptQueue>,
    mut policies: ResMut<SpacePolicies>,
    mut log_panel: ResMut<crate::LogPanel>,
    mut history: ResMut<SpacePolicyHistory>,
    mut diagnostic_cache: Local<HashSet<(specs::Entity, String)>>,
) {
    let _profile = crate::profiling::span("rebuild_space_policies_system");
    if !policies.dirty {
        return;
    }

    let entities = world.0.entities();
    let hierarchies = world.0.read_storage::<Hierarchy>();
    let tags = world.0.read_storage::<Tag>();
    let attrs = world.0.read_storage::<Attrs>();
    let base_urls = world.0.read_storage::<BaseUrl>();
    diagnostic_cache.retain(|(node, _)| entities.is_alive(*node));

    // Only spaces and includes change inheritance. Build their ancestry directly
    // instead of evaluating every static mesh in the scene on each chunk swap.
    let relevant: HashSet<specs::Entity> = (&entities, &tags, &hierarchies)
        .join()
        .filter(|(ent, tag, _)| {
            matches!(tag.0.as_str(), "space" | "include") && dom_data.nodes.contains_key(&ent.id())
        })
        .map(|(ent, _, _)| ent)
        .collect();
    let mut policy_children: HashMap<specs::Entity, Vec<specs::Entity>> = HashMap::new();
    let mut roots = Vec::new();
    for &ent in &relevant {
        if let Some(raw) = attrs.get(ent).and_then(|a| a.0.get("resources")) {
            for token in parse_resource_tokens(raw) {
                if !RESOURCE_BUNDLES.contains_key(token.as_str()) && diagnostic_cache.insert((ent, format!("resource:{token}"))) {
                    log_panel.push_for_space(crate::LogLevel::Warn,
                        format!("[diagnostic] Unknown resource '{token}'. Separate resource names with commas."), ent.id());
                }
            }
        }
        let mut parent = hierarchies.get(ent).and_then(|h| h.parent);
        let mut visited = vec![ent];
        let mut reachable = true;
        while let Some(ancestor) = parent {
            if !dom_data.nodes.contains_key(&ancestor.id()) || visited.contains(&ancestor) {
                reachable = false;
                break;
            }
            visited.push(ancestor);
            let Some(hierarchy) = hierarchies.get(ancestor) else {
                reachable = false;
                break;
            };
            if relevant.contains(&ancestor) {
                break;
            }
            parent = hierarchy.parent;
        }
        if !reachable {
            continue;
        }
        if let Some(parent) = parent {
            policy_children.entry(parent).or_default().push(ent);
        } else {
            roots.push(ent);
        }
    }

    fn walk(
        ent: specs::Entity,
        inherited_caps: CapabilityBits,
        inherited_native: NativeServiceBits,
        entry_caps: CapabilityBits,
        entry_native: NativeServiceBits,
        policy_children: &HashMap<specs::Entity, Vec<specs::Entity>>,
        fallback_url: &str,
        decisions: &PermissionDecisionStore,
        pending_prompts: &mut HashMap<(String, u64), HashSet<u32>>,
        hierarchies: &specs::ReadStorage<Hierarchy>,
        tags: &specs::ReadStorage<Tag>,
        attrs: &specs::ReadStorage<Attrs>,
        base_urls: &specs::ReadStorage<BaseUrl>,
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

            let (requested_caps_raw, requested_native, _) =
                resolve_resource_set(&requested_resources);

            let root_space = is_root_space(attrs_map);
            let managed = is_dimension_luna_managed(attrs_map);
            let origin = find_permission_origin(ent, fallback_url, hierarchies, base_urls);

            // Un entry grant elevado ya fue delegado explícitamente por el UX
            // shell. Para solicitudes directas de un origen no trusted, sólo
            // pasan decisiones Allow persistidas; las desconocidas generan UX.
            let approval_candidates = if managed || root_space || !entry_caps.is_empty() {
                CapabilityBits::empty()
            } else {
                requested_caps_raw & ELEVATED_CAPABILITIES & inherited_caps
            };
            let approved_elevated =
                approved_elevated_capabilities(&origin, approval_candidates, decisions);
            let mut pending_elevated_caps = CapabilityBits::empty();
            for (_, capability) in ELEVATED_CAPABILITY_DEFS {
                if approval_candidates.contains(capability)
                    && decisions.decision(&origin, capability).is_none()
                {
                    pending_elevated_caps |= capability;
                    pending_prompts
                        .entry((origin.clone(), capability.bits()))
                        .or_default()
                        .insert(ent.id());
                }
            }
            let requested_caps = gate_elevated_capabilities(
                requested_caps_raw,
                managed,
                root_space,
                approved_elevated,
            );

            let mut effective_caps = if root_space {
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
            // Network reads remain opt-in even when an entry grant has defaults.
            if !root_space && !requested_caps_raw.contains(CapabilityBits::FETCH_TEXT) {
                effective_caps.remove(CapabilityBits::FETCH_TEXT);
            }
            if !root_space && !requested_caps_raw.contains(CapabilityBits::SPAWN) {
                effective_caps.remove(CapabilityBits::SPAWN);
            }
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
            let auto_scripts = resolve_effective_auto_scripts(
                &requested_resources,
                effective_caps,
                effective_native,
            );

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
                    origin,
                    requested_resources,
                    requested_caps,
                    pending_elevated_caps,
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

        if let Some(children) = policy_children.get(&ent) {
            for &child in children {
                walk(
                    child,
                    child_caps,
                    child_native,
                    child_entry_caps,
                    child_entry_native,
                    policy_children,
                    fallback_url,
                    decisions,
                    pending_prompts,
                    hierarchies,
                    tags,
                    attrs,
                    base_urls,
                    out,
                );
            }
        }
    }

    let mut next = HashMap::new();
    let mut pending_prompts = HashMap::new();
    for root in roots {
        walk(
            root,
            CapabilityBits::empty(),
            NativeServiceBits::empty(),
            CapabilityBits::empty(),
            NativeServiceBits::empty(),
            &policy_children,
            &current_url.0,
            &decisions,
            &mut pending_prompts,
            &hierarchies,
            &tags,
            &attrs,
            &base_urls,
            &mut next,
        );
    }

    prompts.reconcile(pending_prompts);

    let old_gen = policies.generation;
    policies.by_space = next;
    policies.dirty = false;
    policies.generation += 1;

    // Log changes and save snapshots
    for (space_id, policy) in policies.by_space.iter() {
        let denied = policy.requested_caps & !policy.effective_caps;
        let node = entities.entity(*space_id);
        let denied_message = format!("denied:{}", denied.bits());
        if !denied.is_empty() && diagnostic_cache.insert((node, denied_message)) {
            log_panel.push_for_space(crate::LogLevel::Warn,
                format!("[diagnostic] Requested capabilities not granted: {}", describe_capability_bits(denied)), *space_id);
        }
        let snapshot = SpacePolicySnapshotEntry {
            generation: policies.generation,
            space_id: *space_id,
            origin: policy.origin.clone(),
            requested_resources: policy.requested_resources.clone(),
            pending_elevated_caps: policy.pending_elevated_caps,
            effective_caps: policy.effective_caps,
            effective_native: policy.effective_native,
            auto_scripts: policy.auto_scripts.clone(),
        };

        history.entries.push(snapshot.clone());
        if history.entries.len() > history.max_entries {
            history.entries.remove(0);
        }

        log_panel.push_info(format!(
            "[perm][space:{}] origin={} requested=[{}] pending=[{}] effective_caps=[{}] effective_native=[{}]",
            space_id,
            policy.origin,
            policy.requested_resources.join(","),
            describe_capability_bits(policy.pending_elevated_caps),
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
    mut applied_generation: Local<Option<u64>>,
) {
    if *applied_generation == Some(policies.generation) {
        return;
    }
    *applied_generation = Some(policies.generation);

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
    pub origin: String,
    pub requested_resources: Vec<String>,
    pub pending_elevated_caps: CapabilityBits,
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
        app.insert_resource(CurrentUrl("https://fallback.invalid/root.hsml".to_string()));
        app.insert_resource(PermissionDecisionStore::default());
        app.insert_resource(PermissionPromptQueue::default());
        app.insert_resource(SpacePolicies::default());
        app.insert_resource(crate::LogPanel::default());
        app.insert_resource(SpacePolicyHistory {
            entries: Vec::new(),
            max_entries: 32,
        });
        app.add_systems(Update, rebuild_space_policies_system);
        app
    }

    #[test]
    fn diagnostics_report_bad_resources_and_denials_without_repeat_spam() {
        let mut app = build_app_with_xml("<hsml><space resources='fetch_text,unknown_bundle'><include resources='read_camera_pose fetch_text'/></space></hsml>");
        app.update();
        let messages: Vec<_> = app.world().resource::<crate::LogPanel>().logs.iter().filter(|e| e.message.contains("[diagnostic]")).map(|e| e.message.clone()).collect();
        assert!(messages.iter().any(|m| m.contains("unknown_bundle")));
        assert!(messages.iter().any(|m| m.contains("Separate resource names")));
        assert!(messages.iter().any(|m| m.contains("not granted")));
        app.world_mut().resource_mut::<SpacePolicies>().dirty = true;
        app.update();
        assert_eq!(app.world().resource::<crate::LogPanel>().logs.iter().filter(|e|e.message.contains("[diagnostic]")).count(), messages.len());
    }

    #[test]
    fn fetch_needs_both_delegation_and_explicit_request() {
        let mut app = build_app_with_xml("<hsml><space system-space='root' resources='fetch_text'><include resources='fetch_text'><space id='asked' resources='fetch_text'/><space id='silent'/></include><include><space id='blocked' resources='fetch_text'/></include></space></hsml>");
        app.update();
        let doc = app.world().resource::<crate::ElemenetWorld>();
        let attrs = doc.0.read_storage::<Attrs>();
        let policies = app.world().resource::<SpacePolicies>();
        for (id, expected) in [("asked", true), ("silent", false), ("blocked", false)] {
            let (node, _) = (&doc.0.entities(), &attrs).join().find(|(_, a)| a.0.get("id").is_some_and(|s|s == id)).unwrap();
            assert_eq!(policies.by_space[&node.id()].effective_caps.contains(CapabilityBits::FETCH_TEXT), expected, "{id}");
        }
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
    fn sparse_policy_walk_preserves_include_barriers_and_skips_detached_ancestors() {
        let xml = format!(
            r#"<hsml><space id="luna_root" system-space="root" resources="root">
          <group id="container">{}
            <space id="inherited" resources="navigate_self"/>
            <include><group><space id="isolated" resources="navigate_self"/></group></include>
            <include resources="navigate_self"><group><space id="granted" resources="navigate_self"/></group></include>
          </group>
        </space></hsml>"#,
            "<box/>".repeat(4000)
        );
        let mut app = build_app_with_xml(&xml);
        app.update();
        let dom = app.world().resource::<ElemenetWorld>();
        let inherited = find_space_id_by_attr_id(dom, "inherited");
        let isolated = find_space_id_by_attr_id(dom, "isolated");
        let granted = find_space_id_by_attr_id(dom, "granted");
        let policies = app.world().resource::<SpacePolicies>();
        assert!(policies.by_space[&inherited]
            .effective_caps
            .contains(CapabilityBits::NAVIGATE_SELF));
        assert!(policies.by_space[&isolated].effective_caps.is_empty());
        assert!(policies.by_space[&granted]
            .effective_caps
            .contains(CapabilityBits::NAVIGATE_SELF));
        assert_eq!(policies.by_space.len(), 4);
        let container = {
            let entities = dom.0.entities();
            let attrs = dom.0.read_storage::<Attrs>();
            (&entities, &attrs)
                .join()
                .find(|(_, a)| a.0.get("id").is_some_and(|id| id == "container"))
                .unwrap()
                .0
                .id()
        };
        // Attached child IDs do not make a subtree reachable through a detached
        // ancestor. In particular, they must not become new policy roots.
        app.world_mut()
            .resource_mut::<crate::VirtualDomData>()
            .nodes
            .remove(&container);
        app.world_mut().resource_mut::<SpacePolicies>().dirty = true;
        app.update();
        let policies = app.world().resource::<SpacePolicies>();
        assert!(!policies.by_space.contains_key(&inherited));
        assert!(!policies.by_space.contains_key(&isolated));
        assert!(!policies.by_space.contains_key(&granted));
    }

    #[test]
    fn unknown_elevated_capability_prompts_then_respects_allow_and_revoke() {
        let xml = r#"
        <hsml>
          <space id="luna_root" system-space="root" resources="root">
            <space id="third_party" resources="read_system_input" />
          </space>
        </hsml>
        "#;

        let mut app = build_app_with_xml(xml);
        app.update();

        let space_id = find_space_id_by_attr_id(
            app.world().resource::<crate::ElemenetWorld>(),
            "third_party",
        );
        let origin = "https://fallback.invalid";
        assert!(!app
            .world()
            .resource::<SpacePolicies>()
            .by_space
            .get(&space_id)
            .unwrap()
            .effective_caps
            .contains(CapabilityBits::READ_SYSTEM_INPUT));
        let prompts = &app.world().resource::<PermissionPromptQueue>().pending;
        assert_eq!(prompts.len(), 1);
        assert_eq!(prompts[0].origin, origin);
        assert_eq!(prompts[0].capability, CapabilityBits::READ_SYSTEM_INPUT);

        app.world_mut()
            .resource_mut::<PermissionDecisionStore>()
            .set_decision(
                origin.to_string(),
                CapabilityBits::READ_SYSTEM_INPUT,
                PermissionDecision::Allow,
            );
        app.world_mut().resource_mut::<SpacePolicies>().dirty = true;
        app.update();

        assert!(app
            .world()
            .resource::<SpacePolicies>()
            .by_space
            .get(&space_id)
            .unwrap()
            .effective_caps
            .contains(CapabilityBits::READ_SYSTEM_INPUT));
        assert!(app
            .world()
            .resource::<PermissionPromptQueue>()
            .pending
            .is_empty());

        app.world_mut()
            .resource_mut::<PermissionDecisionStore>()
            .set_decision(
                origin.to_string(),
                CapabilityBits::READ_SYSTEM_INPUT,
                PermissionDecision::Deny,
            );
        app.world_mut().resource_mut::<SpacePolicies>().dirty = true;
        app.update();

        assert!(!app
            .world()
            .resource::<SpacePolicies>()
            .by_space
            .get(&space_id)
            .unwrap()
            .effective_caps
            .contains(CapabilityBits::READ_SYSTEM_INPUT));
        assert!(app
            .world()
            .resource::<PermissionPromptQueue>()
            .pending
            .is_empty());
    }

    #[test]
    fn trusted_shell_space_bypasses_user_prompt() {
        let xml = r#"
        <hsml>
          <space id="luna_root" system-space="root" resources="root">
            <space id="trusted" managed-by="dimension.luna" resources="read_system_input" />
          </space>
        </hsml>
        "#;

        let mut app = build_app_with_xml(xml);
        app.update();
        let space_id = find_space_id_by_attr_id(
            app.world().resource::<crate::ElemenetWorld>(),
            "trusted",
        );

        assert!(app
            .world()
            .resource::<SpacePolicies>()
            .by_space
            .get(&space_id)
            .unwrap()
            .effective_caps
            .contains(CapabilityBits::READ_SYSTEM_INPUT));
        assert!(app
            .world()
            .resource::<PermissionPromptQueue>()
            .pending
            .is_empty());
    }

    #[test]
    fn decisions_round_trip_through_json_file() {
        let unique = format!(
            "luna-permissions-{}-{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let path = std::env::temp_dir().join(unique);
        let mut store = PermissionDecisionStore::default();
        store.set_decision(
            "https://example.com".to_string(),
            CapabilityBits::UX_EMBED,
            PermissionDecision::Allow,
        );

        store.save_to(&path).unwrap();
        let loaded = PermissionDecisionStore::load_from(&path).unwrap();
        let _ = fs::remove_file(&path);

        assert_eq!(
            loaded.decision("https://example.com", CapabilityBits::UX_EMBED),
            Some(PermissionDecision::Allow)
        );
    }

    #[test]
    fn permission_origin_discards_paths_and_normalizes_default_ports() {
        assert_eq!(
            normalize_permission_origin("https://example.com:443/app/index.hsml?x=1"),
            "https://example.com"
        );
        assert_eq!(
            normalize_permission_origin("http://example.com:8080/a"),
            "http://example.com:8080"
        );
    }

    #[test]
    fn elevated_auto_script_is_only_injected_after_capability_is_effective() {
        let resources = vec!["ux_embed".to_string()];
        assert!(resolve_effective_auto_scripts(
            &resources,
            CapabilityBits::empty(),
            NativeServiceBits::empty(),
        )
        .is_empty());
        assert_eq!(
            resolve_effective_auto_scripts(
                &resources,
                CapabilityBits::UX_EMBED,
                NativeServiceBits::empty(),
            ),
            vec!["luna://internal/embedded_api.js".to_string()]
        );
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
