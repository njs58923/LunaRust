use std::{
    collections::{HashMap, HashSet},
    fs,
    hash::Hash,
    path::PathBuf,
};

use bevy::{ecs::system::SystemParam, prelude::*};
use serde::{Deserialize, Serialize};
use specs::{Entity as SpecEntity, World as SpecWorld};
use tokio::runtime::Runtime;

// ─── Log ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy)]
pub enum LogLevel {
    Error,
    Warn,
    Info,
}

#[derive(Debug, Clone)]
pub struct LogEntry {
    pub sequence: u64,
    pub timestamp_ms: u64,
    pub tab_id: Option<u64>,
    pub runtime_id: Option<u64>,
    pub level: LogLevel,
    pub message: String,
    pub space_id: Option<u32>,
}

impl LogEntry {
    pub fn new(level: LogLevel, message: impl Into<String>) -> Self {
        Self {
            sequence: Self::next_sequence(),
            timestamp_ms: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as u64,
            tab_id: None,
            runtime_id: None,
            level,
            message: message.into(),
            space_id: None,
        }
    }
    pub fn with_space(level: LogLevel, message: impl Into<String>, space_id: u32) -> Self {
        let mut entry = Self::new(level, message);
        entry.space_id = Some(space_id);
        entry
    }
    fn next_sequence() -> u64 {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    }
}

#[derive(Resource, Default)]
pub struct LogPanel {
    pub logs: Vec<LogEntry>,
}

impl LogPanel {
    pub fn push_error(&mut self, msg: impl Into<String>) {
        self.push(LogLevel::Error, msg);
    }
    pub fn push_warn(&mut self, msg: impl Into<String>) {
        self.push(LogLevel::Warn, msg);
    }
    pub fn push_info(&mut self, msg: impl Into<String>) {
        self.push(LogLevel::Info, msg);
    }
    pub fn push(&mut self, level: LogLevel, msg: impl Into<String>) {
        self.push_entry(LogEntry::new(level, msg));
    }
    pub fn push_for_space(&mut self, level: LogLevel, msg: impl Into<String>, space_id: u32) {
        self.push_entry(LogEntry::with_space(level, msg, space_id));
    }
    pub fn push_entry(&mut self, mut entry: LogEntry) {
        // Limit both a noisy producer and total memory, including giant console strings.
        if entry.message.len() > 4096 {
            let mut end = 4096;
            while !entry.message.is_char_boundary(end) { end -= 1; }
            entry.message.truncate(end);
            entry.message.push_str(" [truncated]");
        }
        if self.logs.iter().filter(|e| e.space_id == entry.space_id).count() >= 300 {
            if let Some(i) = self.logs.iter().position(|e| e.space_id == entry.space_id) { self.logs.remove(i); }
        }
        if self.logs.len() >= 3000 { self.logs.remove(0); }
        self.logs.push(entry);
    }
    pub fn clear(&mut self) {
        self.logs.clear();
    }
    pub fn clear_for_space(&mut self, space_id: u32) {
        self.logs.retain(|e| e.space_id != Some(space_id));
    }
}

// ─── Resources & Components ──────────────────────────────────────────────────

#[derive(Resource, Default)]
pub struct VirtualDomData {
    pub nodes: HashMap<u32, SpecEntity>,
}

#[derive(Resource, Default)]
pub struct DirtyNodes(pub Vec<u32>);

impl DirtyNodes {
    pub fn dedup_in_place(&mut self) {
        self.0.sort_unstable();
        self.0.dedup();
    }

    pub fn take_unique(&mut self) -> Vec<u32> {
        self.dedup_in_place();
        self.0.drain(..).collect()
    }
}

/// Nodos creados desde JS que ya fueron conectados por appendChild,
/// pero cuya inserción visible en dom_data se difiere al próximo frame
/// para evitar render prematuro antes de attrs/transforms.
#[derive(Resource, Default)]
pub struct PendingJsAttachNodes(pub Vec<u32>);

/// Nodos JS ya adjuntados al árbol lógico pero que deben esperar
/// un frame completo antes del primer render para evitar estados
/// intermedios (material/color/mesh default).
#[derive(Resource, Default)]
pub struct PendingJsFirstRenderNodes(pub Vec<(u32, u8)>);

#[derive(Resource, Default)]
pub struct ElemenetWorld(pub SpecWorld);

#[derive(Resource, Default)]
pub struct EntityMap(pub HashMap<u32, Entity>);

#[derive(Resource, Default)]
pub struct EntityCounter {
    pub count: usize,
}

#[derive(Resource)]
pub struct FpsCounter {
    pub timer: Timer,
    pub frame_count: u32,
    pub fps: u32,
}
impl Default for FpsCounter {
    fn default() -> Self {
        Self {
            timer: Timer::from_seconds(1.0, TimerMode::Repeating),
            frame_count: 0,
            fps: 0,
        }
    }
}

#[derive(Resource, Default)]
pub struct CurrentUrl(pub String);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum PreferredRenderMode {
    #[default]
    Desktop,
    Vr,
}

#[derive(Debug, Clone, Serialize, Deserialize, Resource)]
pub struct RootConfig {
    pub auto_load_home: bool,
    pub home_url: String,
    pub preferred_render_mode: PreferredRenderMode,
}

impl Default for RootConfig {
    fn default() -> Self {
        Self {
            auto_load_home: true,
            home_url: "luna://home".to_string(),
            preferred_render_mode: PreferredRenderMode::Desktop,
        }
    }
}

impl RootConfig {
    pub fn path() -> PathBuf {
        crate::utils::folder::resolve_root_config_path()
    }

    pub fn load() -> Self {
        let path = Self::path();
        let Ok(contents) = fs::read_to_string(&path) else {
            return Self::default();
        };
        let mut cfg = serde_json::from_str(&contents).unwrap_or_else(|_| Self::default());

        // Migración de configs viejas:
        // antes home_url podía estar apuntando al root shell.
        if cfg.home_url.trim().is_empty() || cfg.home_url == "luna://root" {
            cfg.home_url = "luna://home".to_string();
        }

        cfg
    }

    pub fn save(&self) -> Result<PathBuf, String> {
        let path = Self::path();
        let parent = path
            .parent()
            .ok_or_else(|| format!("Invalid root config path: {}", path.display()))?;
        fs::create_dir_all(parent).map_err(|e| format!("Create config dir failed: {e}"))?;
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| format!("Serialize root config failed: {e}"))?;
        fs::write(&path, json).map_err(|e| format!("Write root config failed: {e}"))?;
        Ok(path)
    }
}

#[derive(Resource)]
pub struct RenderMode {
    pub is_vr: bool,
}

#[derive(Component)]
pub struct DesktopCamera;

#[derive(Resource, Default)]
pub struct DevtoolVisible(pub bool);

#[derive(Resource, Default)]
pub struct ReloadTrigger(pub bool);

#[derive(Resource, Default)]
pub struct AttributeUpdates(pub Vec<(u32, String, String)>);

impl AttributeUpdates {
    pub fn drain_coalesced(&mut self) -> Vec<(u32, String, String)> {
        let mut last_values: HashMap<(u32, String), String> = HashMap::new();
        for (ent_id, key, value) in self.0.drain(..) {
            last_values.insert((ent_id, key), value);
        }
        last_values
            .into_iter()
            .map(|((ent_id, key), value)| (ent_id, key, value))
            .collect()
    }
}
#[derive(Resource, Default)]
pub struct TransformUpdates {
    pub positions: Vec<(u32, js_runtime::Vec3)>,
    pub rotations: Vec<(u32, js_runtime::Vec3)>,
    pub scales: Vec<(u32, js_runtime::Vec3)>,
}

impl TransformUpdates {
    pub fn is_empty(&self) -> bool {
        self.positions.is_empty() && self.rotations.is_empty() && self.scales.is_empty()
    }
}

/// Nodos que solo cambiaron transform en este frame.
/// Permite usar un fast-path en dom_sync_system sin recalcular attrs/materiales.
#[derive(Resource, Default)]
pub struct TransformOnlyDirtyNodes(pub HashSet<u32>);

#[derive(Component)]
pub struct Dirty;

#[derive(Resource, Default)]
pub struct DeleteRequests(pub Vec<u32>);

#[derive(Debug, Clone)]
pub struct MountedSpaceEntry {
    pub tab_id: u64,
    pub url: String,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpaceMountRequest {
    pub tab_id: u64,
    pub url: String,
    /// Hint opaco al shell JS. Convenciones: "spatial" | "app". Vacío = default spatial.
    pub kind: String,
}

impl SpaceMountRequest {
    /// Constructor para callers que no especifican kind — quedan como spatial
    /// (compat con código pre-Phase 1 tab kinds).
    pub fn new(tab_id: u64, url: String) -> Self {
        Self {
            tab_id,
            url,
            kind: String::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpaceUnmountRequest {
    pub tab_id: u64,
    pub url: String,
}

#[derive(Resource, Default)]
pub struct AddressBarState(pub String);

#[derive(Resource, Default)]
pub struct SpaceMountQueue(pub Vec<SpaceMountRequest>);

#[derive(Resource, Default)]
pub struct SpaceUnmountQueue(pub Vec<SpaceUnmountRequest>);

#[derive(Resource, Default)]
pub struct MountedSpaceList(pub Vec<MountedSpaceEntry>);

#[derive(Resource, Default)]
pub struct ActiveSpaceIndex(pub Option<usize>);

#[derive(Resource)]
pub struct NextTabId(pub u64);

impl Default for NextTabId {
    fn default() -> Self {
        Self(1)
    }
}

#[derive(Resource, Default)]
pub struct GlobalDevtoolVisible(pub bool);

#[derive(Resource, Default)]
pub struct ConfigVisible(pub bool);

#[derive(Resource, Default)]
pub struct KeepLogsOnReload(pub bool);

#[derive(Default)]
pub struct SpaceHandleTable {
    pub runtime_id: u64,
    pub next_local_id: i32,
    pub local_to_global: HashMap<i32, u32>,
    pub global_to_local: HashMap<u32, i32>,
    pub detached_globals: std::collections::HashSet<u32>,
    pub pending_removed_locals: std::collections::HashSet<i32>,
    /// Globals touched (attr/transform/estructura) que este worker AÚN no
    /// recibió. Se acumulan frame a frame mientras el worker está `in_flight`
    /// y se mandan como patch al ack — evita el full-rebuild O(N)/frame que
    /// antes se forzaba para no perder cambios durante el ack.
    pub pending_touched_globals: std::collections::HashSet<u32>,
    /// `true` una vez que el worker recibió su snapshot FULL inicial. Hasta
    /// entonces se le manda full (bootstrap); después, sólo patches.
    pub bootstrapped: bool,
}

#[derive(Resource, Default)]
pub struct SpaceHandleTables {
    pub next_runtime_id: u64,
    pub by_space: HashMap<u32, SpaceHandleTable>,
}

#[derive(Resource)]
pub struct SharedResources {
    pub cube_mesh: Handle<Mesh>,
    pub plane_mesh: Handle<Mesh>,
    pub sphere_mesh: Handle<Mesh>,
    pub cylinder_mesh: Handle<Mesh>,
    pub default_material: Handle<StandardMaterial>,
}

#[derive(Debug, Clone)]
pub enum SkyboxLoadStatus {
    Requested,
    Ready { asset_paths: [String; 6] },
    Mounted,
    Failed { error: String },
}

#[derive(Debug, Clone)]
pub struct MountedSkybox {
    pub asset_paths: [String; 6],
    pub face_entities: Vec<Entity>,
}

#[derive(Debug, Clone)]
pub struct SkyboxNodeState {
    pub key: String,
    pub status: SkyboxLoadStatus,
    /// The currently visible faces remain mounted while a changed `src` loads.
    pub mounted: Option<MountedSkybox>,
}

/// Runtime state for skybox preparation and the last-wins active skybox.
#[derive(Resource, Default)]
pub struct SkyboxEntity {
    pub active: Option<(u32, Entity)>,
    pub nodes: HashMap<u32, SkyboxNodeState>,
    pub pending: HashMap<String, HashSet<u32>>,
}

impl SkyboxEntity {
    pub fn enqueue(&mut self, key: &str, node_id: u32) -> bool {
        self.remove_pending_node(node_id);
        let waiters = self.pending.entry(key.to_string()).or_default();
        let should_spawn = waiters.is_empty();
        waiters.insert(node_id);
        should_spawn
    }

    pub fn take_waiters(&mut self, key: &str) -> Vec<u32> {
        self.pending
            .remove(key)
            .map(|waiters| waiters.into_iter().collect())
            .unwrap_or_default()
    }

    pub fn remove_pending_node(&mut self, node_id: u32) {
        self.pending.retain(|_, waiters| {
            waiters.remove(&node_id);
            !waiters.is_empty()
        });
    }

    pub fn clear_node(&mut self, node_id: u32) {
        self.remove_pending_node(node_id);
        self.nodes.remove(&node_id);
        if matches!(self.active, Some((active_node, _)) if active_node == node_id) {
            self.active = None;
        }
    }

    pub fn clear_runtime(&mut self) {
        self.active = None;
        self.nodes.clear();
        self.pending.clear();
    }
}

#[cfg(test)]
mod skybox_state_tests {
    use super::*;

    #[test]
    fn pending_skybox_preparation_is_shared_by_key() {
        let mut state = SkyboxEntity::default();

        assert!(state.enqueue("https://example.test/sky_$1.png", 10));
        assert!(!state.enqueue("https://example.test/sky_$1.png", 11));
        assert_eq!(state.pending.len(), 1);
        assert_eq!(state.pending.values().next().map(HashSet::len), Some(2));
    }

    #[test]
    fn changing_key_removes_the_node_from_its_stale_request() {
        let mut state = SkyboxEntity::default();
        state.enqueue("old", 10);
        state.enqueue("old", 11);

        assert!(state.enqueue("new", 10));
        assert_eq!(state.pending.get("old"), Some(&HashSet::from([11])));
        assert_eq!(state.pending.get("new"), Some(&HashSet::from([10])));
    }

    #[test]
    fn clearing_node_drops_pending_load_and_active_reference() {
        let mut state = SkyboxEntity::default();
        state.enqueue("sky", 10);
        state.nodes.insert(
            10,
            SkyboxNodeState {
                key: "sky".to_string(),
                status: SkyboxLoadStatus::Requested,
                mounted: None,
            },
        );
        state.active = Some((10, Entity::from_raw(7)));

        state.clear_node(10);

        assert!(state.pending.is_empty());
        assert!(state.nodes.is_empty());
        assert!(state.active.is_none());
    }
}

#[derive(Resource)]
pub struct TokioRuntime(pub Runtime);

pub const TEXT_MATERIAL_CACHE_CAPACITY: usize = 256;
pub const PRIMITIVE_MATERIAL_CACHE_CAPACITY: usize = 512;
pub const ROUNDED_MESH_CACHE_CAPACITY: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TextMaterialKey {
    pub value: String,
    pub color_rgba: [u8; 4],
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PrimitiveMaterialKey {
    pub color_rgba: [u8; 4],
    pub double_sided: bool,
}

#[derive(Debug, Clone)]
struct CachedAsset<V> {
    value: V,
    last_used: u64,
}

/// Caché LRU acotada. La búsqueda es O(1); únicamente busca la entrada menos
/// reciente (O(capacidad)) al insertar un miss cuando ya alcanzó el límite.
///
/// Expulsar un Handle de aquí no invalida entidades que aún lo usan: sus
/// propios Handles fuertes mantienen vivo el asset en Bevy.
#[derive(Debug, Clone)]
struct BoundedAssetCache<K, V> {
    entries: HashMap<K, CachedAsset<V>>,
    clock: u64,
    capacity: usize,
}

impl<K, V> BoundedAssetCache<K, V>
where
    K: Eq + Hash + Clone,
    V: Clone,
{
    fn new(capacity: usize) -> Self {
        Self {
            entries: HashMap::new(),
            clock: 0,
            capacity,
        }
    }

    fn next_stamp(&mut self) -> u64 {
        self.clock = self.clock.saturating_add(1);
        self.clock
    }

    fn get_cloned(&mut self, key: &K) -> Option<V> {
        let stamp = self.next_stamp();
        let entry = self.entries.get_mut(key)?;
        entry.last_used = stamp;
        Some(entry.value.clone())
    }

    fn insert(&mut self, key: K, value: V) {
        if self.capacity == 0 {
            return;
        }

        let stamp = self.next_stamp();
        if let Some(entry) = self.entries.get_mut(&key) {
            entry.value = value;
            entry.last_used = stamp;
            return;
        }

        if self.entries.len() >= self.capacity {
            let lru_key = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.last_used)
                .map(|(key, _)| key.clone());
            if let Some(lru_key) = lru_key {
                self.entries.remove(&lru_key);
            }
        }

        self.entries.insert(
            key,
            CachedAsset {
                value,
                last_used: stamp,
            },
        );
    }

    fn clear(&mut self) {
        self.entries.clear();
        self.clock = 0;
    }

    fn len(&self) -> usize {
        self.entries.len()
    }
}

#[derive(Resource)]
pub struct PrimitiveMaterialCache {
    materials: BoundedAssetCache<PrimitiveMaterialKey, Handle<StandardMaterial>>,
}

impl Default for PrimitiveMaterialCache {
    fn default() -> Self {
        Self {
            materials: BoundedAssetCache::new(PRIMITIVE_MATERIAL_CACHE_CAPACITY),
        }
    }
}

impl PrimitiveMaterialCache {
    pub fn get(&mut self, key: &PrimitiveMaterialKey) -> Option<Handle<StandardMaterial>> {
        self.materials.get_cloned(key)
    }

    pub fn insert(&mut self, key: PrimitiveMaterialKey, value: Handle<StandardMaterial>) {
        self.materials.insert(key, value);
    }

    pub fn clear(&mut self) {
        self.materials.clear();
    }

    pub fn len(&self) -> usize {
        self.materials.len()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RoundedMeshKind {
    Cube,
    Plane,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RoundedMeshKey {
    pub kind: RoundedMeshKind,
    pub radius_x_quantized: u16,
    pub radius_y_quantized: u16,
    pub radius_z_quantized: u16,
    pub segments: u32,
}

#[derive(Resource)]
pub struct RoundedMeshCache {
    meshes: BoundedAssetCache<RoundedMeshKey, Handle<Mesh>>,
}

impl Default for RoundedMeshCache {
    fn default() -> Self {
        Self {
            meshes: BoundedAssetCache::new(ROUNDED_MESH_CACHE_CAPACITY),
        }
    }
}

impl RoundedMeshCache {
    pub fn get(&mut self, key: &RoundedMeshKey) -> Option<Handle<Mesh>> {
        self.meshes.get_cloned(key)
    }

    pub fn insert(&mut self, key: RoundedMeshKey, value: Handle<Mesh>) {
        self.meshes.insert(key, value);
    }

    pub fn clear(&mut self) {
        self.meshes.clear();
    }

    pub fn len(&self) -> usize {
        self.meshes.len()
    }
}

#[derive(Resource)]
pub struct TextMaterialCache {
    materials: BoundedAssetCache<TextMaterialKey, Handle<StandardMaterial>>,
}

impl Default for TextMaterialCache {
    fn default() -> Self {
        Self {
            materials: BoundedAssetCache::new(TEXT_MATERIAL_CACHE_CAPACITY),
        }
    }
}

impl TextMaterialCache {
    pub fn get(&mut self, key: &TextMaterialKey) -> Option<Handle<StandardMaterial>> {
        self.materials.get_cloned(key)
    }

    pub fn insert(&mut self, key: TextMaterialKey, value: Handle<StandardMaterial>) {
        self.materials.insert(key, value);
    }

    pub fn clear(&mut self) {
        self.materials.clear();
    }

    pub fn len(&self) -> usize {
        self.materials.len()
    }
}

#[cfg(test)]
mod bounded_asset_cache_tests {
    use super::*;

    #[test]
    fn evicts_the_least_recently_used_entry_at_capacity() {
        let mut cache = BoundedAssetCache::new(2);
        cache.insert("a", 1);
        cache.insert("b", 2);

        assert_eq!(cache.get_cloned(&"a"), Some(1));
        cache.insert("c", 3);

        assert_eq!(cache.get_cloned(&"a"), Some(1));
        assert_eq!(cache.get_cloned(&"b"), None);
        assert_eq!(cache.get_cloned(&"c"), Some(3));
        assert_eq!(cache.len(), 2);
    }

    #[test]
    fn clear_drops_entries_and_resets_cache_for_reuse() {
        let mut cache = BoundedAssetCache::new(1);
        cache.insert("a", 1);
        cache.clear();
        cache.insert("b", 2);

        assert_eq!(cache.get_cloned(&"a"), None);
        assert_eq!(cache.get_cloned(&"b"), Some(2));
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn configured_asset_caches_never_exceed_their_limits() {
        let mut text = TextMaterialCache::default();
        for i in 0..=TEXT_MATERIAL_CACHE_CAPACITY {
            text.insert(
                TextMaterialKey {
                    value: i.to_string(),
                    color_rgba: [255; 4],
                },
                Handle::default(),
            );
        }

        let mut primitives = PrimitiveMaterialCache::default();
        for i in 0..=PRIMITIVE_MATERIAL_CACHE_CAPACITY {
            primitives.insert(
                PrimitiveMaterialKey {
                    color_rgba: (i as u32).to_le_bytes(),
                    double_sided: false,
                },
                Handle::default(),
            );
        }

        let mut rounded = RoundedMeshCache::default();
        for i in 0..=ROUNDED_MESH_CACHE_CAPACITY {
            rounded.insert(
                RoundedMeshKey {
                    kind: RoundedMeshKind::Plane,
                    radius_x_quantized: i as u16,
                    radius_y_quantized: 0,
                    radius_z_quantized: 0,
                    segments: 6,
                },
                Handle::default(),
            );
        }

        assert_eq!(text.len(), TEXT_MATERIAL_CACHE_CAPACITY);
        assert_eq!(primitives.len(), PRIMITIVE_MATERIAL_CACHE_CAPACITY);
        assert_eq!(rounded.len(), ROUNDED_MESH_CACHE_CAPACITY);
    }
}

#[derive(Resource, Default)]
pub struct PerformanceStats {
    pub dom_sync_ms: f32,
    /// Nodos que `dom_sync_system` procesó este frame (tras dedup).
    pub dom_sync_dirty_in: usize,
    /// Cuántos de esos se resolvieron por el fast-lane transform-only.
    pub dom_sync_fastlane: usize,
    /// Descendientes re-encolados por `next_dirty` (despawn diferido).
    pub dom_sync_requeued: usize,
    /// Tamaño de `TransformOnlyDirtyNodes` al final del frame.
    pub transform_only_len: usize,
    /// Tiempo en `js_update_snapshots_system` este frame.
    pub js_snapshot_ms: f32,
    /// Hubo full-rebuild del DomMirror este frame (O(N)).
    pub mirror_full_rebuild: bool,
    /// Nodos en el DomMirror.
    pub mirror_nodes: usize,
    /// Snapshots enviados a workers este frame.
    pub snapshots_sent: usize,
    /// Spaces cuyo worker está `in_flight` (esperando ack).
    pub waiting_on_ack: usize,
}

#[derive(Resource)]
pub struct JsSnapshotState {
    pub dirty: bool,
    pub mirror_force_rebuild: bool,
}

impl Default for JsSnapshotState {
    fn default() -> Self {
        Self {
            dirty: true,
            mirror_force_rebuild: true,
        }
    }
}

#[derive(Resource, Default)]
pub struct PendingScripts(pub Vec<(u32, String, String)>);

#[derive(Debug, Clone)]
pub struct DeferredSpaceMount {
    pub wait_gone_tab_id: u64,
    pub mount: SpaceMountRequest,
}

#[derive(Resource, Default)]
pub struct DeferredSpaceMounts(pub Vec<DeferredSpaceMount>);

#[derive(Debug, Clone)]
pub struct PendingInclude {
    pub parent_node_id: u32,
    pub url: String,
    pub xml: String,
}

#[derive(Resource, Default)]
pub struct PendingIncludes(pub Vec<PendingInclude>);

/// Tracks which include nodes have already been requested (to avoid duplicate loads).
#[derive(Resource, Default)]
pub struct IncludeLoadStates(pub HashMap<u32, IncludeLoadState>);

#[derive(Debug, Clone)]
pub enum IncludeLoadState {
    Loading { url: String },
    Loaded { url: String },
    Failed { url: String },
}

#[derive(PartialEq, Eq)]
pub enum DevtoolTab {
    Status,
    Hsml,
    Logs,
    Redes,
    Resources,
}

#[derive(Resource)]
pub struct DevtoolState {
    pub active_tab: DevtoolTab,
}
impl Default for DevtoolState {
    fn default() -> Self {
        DevtoolState {
            active_tab: DevtoolTab::Status,
        }
    }
}

// ─── SystemParam bundles ─────────────────────────────────────────────────────

#[derive(SystemParam)]
pub struct UiSystemParams<'w> {
    pub agent: ResMut<'w, crate::agent::AgentControl>,
    pub entity_counter: Res<'w, EntityCounter>,
    pub fps_counter: Res<'w, FpsCounter>,
    pub perf_stats: Res<'w, PerformanceStats>,
    pub dom_mirror: Res<'w, crate::js::DomMirror>,
    pub permission_decisions: ResMut<'w, crate::permissions::PermissionDecisionStore>,
    pub permission_prompts: ResMut<'w, crate::permissions::PermissionPromptQueue>,
    pub space_policies: ResMut<'w, crate::permissions::SpacePolicies>,
    pub root_config: ResMut<'w, RootConfig>,
}

#[derive(SystemParam)]
pub struct TextRenderParams<'w> {
    pub surfaces: Option<ResMut<'w, crate::surface::SurfaceQueue>>,
    pub materials: ResMut<'w, Assets<StandardMaterial>>,
    pub images: ResMut<'w, Assets<Image>>,
    pub text_material_cache: ResMut<'w, TextMaterialCache>,
    pub primitive_material_cache: ResMut<'w, PrimitiveMaterialCache>,
}

#[derive(SystemParam)]
pub struct SpaceParams<'w> {
    pub mount_queue: ResMut<'w, SpaceMountQueue>,
    pub unmount_queue: ResMut<'w, SpaceUnmountQueue>,
    pub mounted_spaces: ResMut<'w, MountedSpaceList>,
    pub deferred_mounts: ResMut<'w, DeferredSpaceMounts>,
    pub next_tab_id: ResMut<'w, NextTabId>,
}

#[derive(SystemParam)]
pub struct DevtoolParams<'w> {
    pub visible: ResMut<'w, DevtoolVisible>,
    pub state: ResMut<'w, DevtoolState>,
    pub attribute_updates: ResMut<'w, AttributeUpdates>,
    pub delete_requests: ResMut<'w, DeleteRequests>,
    pub config_visible: ResMut<'w, ConfigVisible>,
    pub keep_logs: ResMut<'w, KeepLogsOnReload>,
}

#[derive(SystemParam)]
pub struct AsyncDomParams<'w, 's> {
    pub tokio_rt: Res<'w, TokioRuntime>,
    pub io_service: Res<'w, crate::IoService>,
    pub current_url: Res<'w, CurrentUrl>,
    pub script_load_states: ResMut<'w, crate::ScriptLoadStates>,
    pub pending_model_loads: ResMut<'w, crate::PendingModelLoads>,
    pub model_load_states: ResMut<'w, crate::ModelLoadStates>,
    pub transform_only_dirty: ResMut<'w, crate::TransformOnlyDirtyNodes>,
    pub pending_scripts: ResMut<'w, PendingScripts>,
    pub include_load_states: ResMut<'w, crate::IncludeLoadStates>,
    /// Lectura de la instancia estable de cada `<model>` (acceso disjunto del
    /// `&mut Transform` de la query principal de `dom_sync_system`).
    pub mounted_models: Query<'w, 's, &'static crate::models::ModelInstance>,
    pub model_status_updates: ResMut<'w, AttributeUpdates>,
    pub model_animation_configs: Query<'w, 's, &'static crate::model_animation::ModelAnimationConfig>,
    pub dynamic_meshes: Option<Res<'w, crate::dynamic_mesh::DynamicMeshes>>,
}

#[derive(SystemParam)]
pub struct SkyboxParams<'w> {
    pub skybox_entity: ResMut<'w, SkyboxEntity>,
    pub space_policies: Res<'w, crate::permissions::SpacePolicies>,
}

#[derive(SystemParam)]
pub struct AsyncNodeCleanupParams<'w> {
    pub script_load_states: ResMut<'w, crate::ScriptLoadStates>,
    pub pending_model_loads: ResMut<'w, crate::PendingModelLoads>,
    pub model_load_states: ResMut<'w, crate::ModelLoadStates>,
    pub skybox: ResMut<'w, SkyboxEntity>,
    pub space_handle_tables: ResMut<'w, SpaceHandleTables>,
}

#[derive(SystemParam)]
pub struct DocumentCommitParams<'w> {
    pub attribute_updates: ResMut<'w, AttributeUpdates>,
    pub delete_requests: ResMut<'w, DeleteRequests>,
    pub pending_scripts: ResMut<'w, PendingScripts>,
    pub script_load_states: ResMut<'w, crate::ScriptLoadStates>,
    pub pending_model_loads: ResMut<'w, crate::PendingModelLoads>,
    pub model_load_states: ResMut<'w, crate::ModelLoadStates>,
    pub skybox: ResMut<'w, SkyboxEntity>,
    pub pending_includes: ResMut<'w, crate::PendingIncludes>,
    pub include_load_states: ResMut<'w, crate::IncludeLoadStates>,
    pub space_handle_tables: ResMut<'w, SpaceHandleTables>,
    pub text_material_cache: ResMut<'w, TextMaterialCache>,
    pub primitive_material_cache: ResMut<'w, PrimitiveMaterialCache>,
    pub rounded_mesh_cache: ResMut<'w, RoundedMeshCache>,
    pub ws_service: Option<Res<'w, crate::WsService>>,
    pub io_service: Option<Res<'w, crate::IoService>>,
    pub js_snapshot_state: ResMut<'w, JsSnapshotState>,
    // Colas de ops del JS pendientes del documento anterior. Si no se limpian,
    // las pos/rot/attr/attaches stale se aplicarían sobre el SPECS world nuevo,
    // donde los slot ids pueden coincidir con entidades del nuevo HSML →
    // corrupción ("luna://home no carga ningún elemento" tras uso intenso).
    pub transform_updates: ResMut<'w, TransformUpdates>,
    pub transform_only_dirty: ResMut<'w, TransformOnlyDirtyNodes>,
    pub pending_js_attaches: ResMut<'w, PendingJsAttachNodes>,
    pub pending_js_first_render: ResMut<'w, PendingJsFirstRenderNodes>,
}
