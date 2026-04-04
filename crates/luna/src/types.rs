use std::collections::HashMap;

use bevy::{ecs::system::SystemParam, prelude::*};
use specs::{Entity as SpecEntity, World as SpecWorld};
use tokio::runtime::Runtime;

// ─── Log ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy)]
pub enum LogLevel { Error, Warn, Info }

#[derive(Debug, Clone)]
pub struct LogEntry {
    pub level: LogLevel,
    pub message: String,
}

impl LogEntry {
    pub fn new(level: LogLevel, message: impl Into<String>) -> Self {
        Self { level, message: message.into() }
    }
}

#[derive(Resource, Default)]
pub struct LogPanel {
    pub logs: Vec<LogEntry>,
}

impl LogPanel {
    pub fn push_error(&mut self, msg: impl Into<String>) { self.push(LogLevel::Error, msg); }
    pub fn push_warn(&mut self, msg: impl Into<String>) { self.push(LogLevel::Warn, msg); }
    pub fn push_info(&mut self, msg: impl Into<String>) { self.push(LogLevel::Info, msg); }
    pub fn push(&mut self, level: LogLevel, msg: impl Into<String>) {
        const MAX_LOGS: usize = 300;
        if self.logs.len() >= MAX_LOGS { self.logs.remove(0); }
        self.logs.push(LogEntry::new(level, msg));
    }
    pub fn clear(&mut self) { self.logs.clear(); }
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
pub struct EntityCounter { pub count: usize }

#[derive(Resource)]
pub struct FpsCounter {
    pub timer: Timer,
    pub frame_count: u32,
    pub fps: u32,
}
impl Default for FpsCounter {
    fn default() -> Self {
        Self { timer: Timer::from_seconds(1.0, TimerMode::Repeating), frame_count: 0, fps: 0 }
    }
}

#[derive(Resource, Default)]
pub struct CurrentUrl(pub String);

#[derive(Resource)]
pub struct AutoLoadConfig {
    pub enabled: bool,
    pub start_url: String,
}
impl Default for AutoLoadConfig {
    fn default() -> Self {
        Self { enabled: true, start_url: "luna://home".to_string() }
    }
}

#[derive(Resource)]
pub struct RenderMode { pub is_vr: bool }

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

#[derive(Resource)]
pub struct SharedResources {
    pub cube_mesh: Handle<Mesh>,
    pub plane_mesh: Handle<Mesh>,
    pub default_material: Handle<StandardMaterial>,
}

#[derive(Resource)]
pub struct TokioRuntime(pub Runtime);

#[derive(Resource, Default)]
pub struct ModelCache { pub cache: HashMap<String, String> }

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
pub struct PerformanceStats { pub dom_sync_ms: f32 }

#[derive(Resource, Default)]
pub struct PendingScripts(pub Vec<(u32, String, String)>);

#[derive(PartialEq, Eq)]
pub enum DevtoolTab { Status, Hsml, Logs, Redes }

#[derive(Resource)]
pub struct DevtoolState { pub active_tab: DevtoolTab }
impl Default for DevtoolState {
    fn default() -> Self { DevtoolState { active_tab: DevtoolTab::Status } }
}

// ─── SystemParam bundles ─────────────────────────────────────────────────────

#[derive(SystemParam)]
pub struct UiSystemParams<'w> {
    pub entity_counter: Res<'w, EntityCounter>,
    pub fps_counter: Res<'w, FpsCounter>,
    pub perf_stats: Res<'w, PerformanceStats>,
    pub auto_load_config: ResMut<'w, AutoLoadConfig>,
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
