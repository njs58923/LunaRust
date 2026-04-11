use std::{collections::HashMap, fs, path::PathBuf};

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
    pub level: LogLevel,
    pub message: String,
}

impl LogEntry {
    pub fn new(level: LogLevel, message: impl Into<String>) -> Self {
        Self {
            level,
            message: message.into(),
        }
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
        const MAX_LOGS: usize = 300;
        if self.logs.len() >= MAX_LOGS {
            self.logs.remove(0);
        }
        self.logs.push(LogEntry::new(level, msg));
    }
    pub fn clear(&mut self) {
        self.logs.clear();
    }
}

// ─── Resources & Components ──────────────────────────────────────────────────

#[derive(Resource, Default)]
pub struct VirtualDomData {
    pub nodes: HashMap<u32, SpecEntity>,
}

#[derive(Resource, Default)]
pub struct DirtyNodes(pub Vec<u32>);

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
            home_url: "luna://root".to_string(),
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
        serde_json::from_str(&contents).unwrap_or_else(|_| Self::default())
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

#[derive(Component)]
pub struct Dirty;

#[derive(Resource, Default)]
pub struct DeleteRequests(pub Vec<u32>);

#[derive(Default)]
pub struct SpaceHandleTable {
    pub runtime_id: u64,
    pub next_local_id: i32,
    pub local_to_global: HashMap<i32, u32>,
    pub global_to_local: HashMap<u32, i32>,
    pub detached_globals: std::collections::HashSet<u32>,
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

#[derive(Resource)]
pub struct TokioRuntime(pub Runtime);

#[derive(Resource, Default)]
pub struct ModelCache {
    pub cache: HashMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TextMaterialKey {
    pub value: String,
    pub size_bits: u32,
    pub color_key: String,
}

#[derive(Resource, Default)]
pub struct TextMaterialCache {
    pub materials: HashMap<TextMaterialKey, Handle<StandardMaterial>>,
}

#[derive(Resource, Default)]
pub struct PerformanceStats {
    pub dom_sync_ms: f32,
}

#[derive(Resource, Default)]
pub struct PendingScripts(pub Vec<(u32, String, String)>);

#[derive(PartialEq, Eq)]
pub enum DevtoolTab {
    Status,
    Hsml,
    Logs,
    Redes,
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
    pub entity_counter: Res<'w, EntityCounter>,
    pub fps_counter: Res<'w, FpsCounter>,
    pub perf_stats: Res<'w, PerformanceStats>,
    pub root_config: ResMut<'w, RootConfig>,
}

#[derive(SystemParam)]
pub struct TextRenderParams<'w> {
    pub materials: ResMut<'w, Assets<StandardMaterial>>,
    pub images: ResMut<'w, Assets<Image>>,
    pub text_material_cache: ResMut<'w, TextMaterialCache>,
}

#[derive(SystemParam)]
pub struct AsyncDomParams<'w> {
    pub tokio_rt: Res<'w, TokioRuntime>,
    pub io_service: Res<'w, crate::IoService>,
    pub current_url: Res<'w, CurrentUrl>,
    pub script_load_states: ResMut<'w, crate::ScriptLoadStates>,
    pub pending_model_loads: ResMut<'w, crate::PendingModelLoads>,
    pub model_load_states: ResMut<'w, crate::ModelLoadStates>,
}
