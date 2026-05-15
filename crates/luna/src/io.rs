use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::PathBuf,
    sync::{mpsc, Mutex},
    time::{Duration, Instant},
};

use bevy::prelude::*;
use quick_xml::{events::Event, Reader};
use specs::WorldExt;
use tokio::{runtime::Runtime, task};
use virtual_dom::dom::hsml::Script;

use crate::{
    dom::resolve_node_relative_url,
    js::{find_owner_space_id, JsWorkerCommand, ScriptRuntimeManager},
    render::{encode_url_to_filename, resolve_remote_path},
    routes::VIRTUAL_ROUTES,
    utils::folder::{resolve_assets_and_cache_dirs, to_assets_relative},
    CurrentUrl, DirtyNodes, ElemenetWorld, LogPanel, PendingScripts,
};

pub enum IoResult {
    DocumentLoaded {
        network_id: u64,
        epoch: u64,
        url: String,
        result: Result<LoadedDocumentBundle, String>,
    },
    FetchCompleted {
        network_id: u64,
        space_id: u32,
        request_id: i32,
        result: Result<String, String>,
    },
    ScriptLoaded {
        network_id: u64,
        node_id: u32,
        url: String,
        result: Result<String, String>,
    },
    ModelPrepared {
        network_id: u64,
        url: String,
        result: Result<String, String>,
    },
    IncludeLoaded {
        network_id: u64,
        parent_node_id: u32,
        url: String,
        result: Result<String, String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkRequestKind {
    Document,
    Fetch,
    Script,
    Model,
    Include,
}

impl NetworkRequestKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Document => "document",
            Self::Fetch => "fetch",
            Self::Script => "script",
            Self::Model => "model",
            Self::Include => "include",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkRequestStatus {
    Queued,
    Ok,
    Error,
}

impl NetworkRequestStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Ok => "ok",
            Self::Error => "error",
        }
    }
}

#[derive(Debug, Clone)]
pub struct NetworkRequestEntry {
    pub id: u64,
    pub kind: NetworkRequestKind,
    pub url: String,
    pub owner: String,
    pub status: NetworkRequestStatus,
    pub started_at: Instant,
    pub finished_at: Option<Instant>,
    pub detail: Option<String>,
}

struct NetworkTracker {
    next_id: u64,
    entries: VecDeque<NetworkRequestEntry>,
}

impl Default for NetworkTracker {
    fn default() -> Self {
        Self {
            next_id: 1,
            entries: VecDeque::new(),
        }
    }
}

#[derive(Resource)]
pub struct IoService {
    result_tx: mpsc::Sender<IoResult>,
    result_rx: Mutex<mpsc::Receiver<IoResult>>,
    http_client: reqwest::Client,
    network_tracker: Mutex<NetworkTracker>,
}

impl Default for IoService {
    fn default() -> Self {
        let (result_tx, result_rx) = mpsc::channel();
        let http_client = reqwest::Client::builder()
            .user_agent("Luna/0.1")
            .timeout(Duration::from_secs(20))
            .connect_timeout(Duration::from_secs(10))
            .pool_idle_timeout(Duration::from_secs(30))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        Self {
            result_tx,
            result_rx: Mutex::new(result_rx),
            http_client,
            network_tracker: Mutex::new(NetworkTracker::default()),
        }
    }
}

impl IoService {
    fn sender(&self) -> mpsc::Sender<IoResult> {
        self.result_tx.clone()
    }

    pub fn http_client(&self) -> reqwest::Client {
        self.http_client.clone()
    }

    pub fn begin_request(
        &self,
        kind: NetworkRequestKind,
        url: impl Into<String>,
        owner: impl Into<String>,
    ) -> u64 {
        const MAX_NETWORK_ENTRIES: usize = 200;

        let Ok(mut tracker) = self.network_tracker.lock() else {
            return 0;
        };

        let id = tracker.next_id;
        tracker.next_id += 1;
        if tracker.entries.len() >= MAX_NETWORK_ENTRIES {
            tracker.entries.pop_front();
        }
        tracker.entries.push_back(NetworkRequestEntry {
            id,
            kind,
            url: url.into(),
            owner: owner.into(),
            status: NetworkRequestStatus::Queued,
            started_at: Instant::now(),
            finished_at: None,
            detail: None,
        });
        id
    }

    pub fn finish_request(&self, id: u64, status: NetworkRequestStatus, detail: Option<String>) {
        if id == 0 {
            return;
        }

        let Ok(mut tracker) = self.network_tracker.lock() else {
            return;
        };

        if let Some(entry) = tracker.entries.iter_mut().find(|entry| entry.id == id) {
            entry.status = status;
            entry.finished_at = Some(Instant::now());
            entry.detail = detail;
        }
    }

    pub fn network_entries(&self) -> Vec<NetworkRequestEntry> {
        let Ok(tracker) = self.network_tracker.lock() else {
            return Vec::new();
        };
        tracker.entries.iter().cloned().collect()
    }

    pub fn clear_network_entries(&self) {
        let Ok(mut tracker) = self.network_tracker.lock() else {
            return;
        };
        tracker.entries.clear();
    }
}

#[derive(Debug, Clone)]
pub struct CompletedDocumentLoad {
    pub epoch: u64,
    pub url: String,
    pub result: Result<LoadedDocumentBundle, String>,
}

#[derive(Resource, Default)]
pub struct PendingDocumentLoads(pub Vec<CompletedDocumentLoad>);

#[derive(Debug, Clone)]
pub struct ActiveDocumentLoad {
    pub epoch: u64,
    pub url: String,
}

#[derive(Resource, Default)]
pub struct DocumentLoadState(pub Option<ActiveDocumentLoad>);

#[derive(Resource, Default)]
pub struct NavigationEpoch(pub u64);

#[derive(Debug, Clone, Default)]
pub struct LoadedDocumentBundle {
    pub root_xml: String,
    pub includes: HashMap<String, String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone)]
pub enum ScriptLoadState {
    Requested { url: String },
    Loaded { url: String },
    Failed { url: String, error: String },
}

#[derive(Resource, Default)]
pub struct ScriptLoadStates(pub HashMap<u32, ScriptLoadState>);

#[derive(Debug, Clone)]
pub enum ModelLoadState {
    Requested { url: String },
    Ready { url: String, asset_path: String },
    Failed { url: String, error: String },
}

#[derive(Resource, Default)]
pub struct ModelLoadStates(pub HashMap<u32, ModelLoadState>);

#[derive(Resource, Default)]
pub struct PendingModelLoads(pub HashMap<String, HashSet<u32>>);

impl PendingModelLoads {
    pub fn enqueue(&mut self, url: &str, node_id: u32) -> bool {
        let waiters = self.0.entry(url.to_string()).or_default();
        let should_spawn = waiters.is_empty();
        waiters.insert(node_id);
        should_spawn
    }

    pub fn take_waiters(&mut self, url: &str) -> Vec<u32> {
        self.0
            .remove(url)
            .map(|waiters| waiters.into_iter().collect())
            .unwrap_or_default()
    }

    pub fn remove_node(&mut self, node_id: u32) {
        self.0.retain(|_, waiters| {
            waiters.remove(&node_id);
            !waiters.is_empty()
        });
    }
}

pub fn clear_async_node_state(
    node_id: u32,
    script_loads: &mut ScriptLoadStates,
    pending_model_loads: &mut PendingModelLoads,
    model_loads: &mut ModelLoadStates,
) {
    script_loads.0.remove(&node_id);
    pending_model_loads.remove_node(node_id);
    model_loads.0.remove(&node_id);
}

pub fn request_fetch_text(
    rt: &Runtime,
    io_service: &IoService,
    space_id: u32,
    request_id: i32,
    url: String,
) {
    let network_id = io_service.begin_request(
        NetworkRequestKind::Fetch,
        url.clone(),
        format!("space:{space_id}"),
    );
    let tx = io_service.sender();
    let client = io_service.http_client();
    rt.spawn(async move {
        let result = load_text_resource(&url, &client).await;
        let _ = tx.send(IoResult::FetchCompleted {
            network_id,
            space_id,
            request_id,
            result,
        });
    });
}

pub fn request_document_load(rt: &Runtime, io_service: &IoService, epoch: u64, url: String) {
    let network_id = io_service.begin_request(
        NetworkRequestKind::Document,
        url.clone(),
        format!("epoch:{epoch}"),
    );
    let tx = io_service.sender();
    let client = io_service.http_client();
    rt.spawn(async move {
        let result = load_document_bundle(&url, &client).await;
        let _ = tx.send(IoResult::DocumentLoaded {
            network_id,
            epoch,
            url,
            result,
        });
    });
}

pub fn request_script_load(rt: &Runtime, io_service: &IoService, node_id: u32, url: String) {
    let network_id = io_service.begin_request(
        NetworkRequestKind::Script,
        url.clone(),
        format!("node:{node_id}"),
    );
    let tx = io_service.sender();
    let client = io_service.http_client();
    rt.spawn(async move {
        let result = load_text_resource(&url, &client).await;
        let _ = tx.send(IoResult::ScriptLoaded {
            network_id,
            node_id,
            url,
            result,
        });
    });
}

pub fn request_include_load(
    rt: &Runtime,
    io_service: &IoService,
    parent_node_id: u32,
    url: String,
) {
    let network_id = io_service.begin_request(
        NetworkRequestKind::Include,
        url.clone(),
        format!("include-parent:{parent_node_id}"),
    );
    let tx = io_service.sender();
    let client = io_service.http_client();
    rt.spawn(async move {
        let result = load_text_resource(&url, &client).await;
        let _ = tx.send(IoResult::IncludeLoaded {
            network_id,
            parent_node_id,
            url,
            result,
        });
    });
}

pub fn request_model_prepare(rt: &Runtime, io_service: &IoService, url: String) {
    let network_id =
        io_service.begin_request(NetworkRequestKind::Model, url.clone(), "model-cache");
    let tx = io_service.sender();
    let client = io_service.http_client();
    rt.spawn(async move {
        let result = prepare_model_asset(&url, &client).await;
        let _ = tx.send(IoResult::ModelPrepared {
            network_id,
            url,
            result,
        });
    });
}

async fn load_text_resource(url: &str, client: &reqwest::Client) -> Result<String, String> {
    if crate::routes::VirtualRoutes::is_virtual_url(url) {
        return VIRTUAL_ROUTES
            .resolve(url)
            .ok_or_else(|| format!("Virtual URL not found: {url}"));
    }

    client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("HTTP error: {e}"))?
        .error_for_status()
        .map_err(|e| format!("HTTP status error: {e}"))?
        .text()
        .await
        .map_err(|e| format!("Read error: {e}"))
}

fn extract_include_sources(xml: &str) -> Result<Vec<String>, String> {
    let mut reader = Reader::from_str(xml);
    reader.trim_text(true);
    let mut includes = Vec::new();

    loop {
        match reader.read_event() {
            Ok(Event::Start(ref e)) | Ok(Event::Empty(ref e)) => {
                if e.name().as_ref() != b"include" {
                    continue;
                }
                for attr in e.attributes() {
                    let attr = attr.map_err(|e| format!("Include attr read error: {e}"))?;
                    if attr.key.as_ref() == b"src" {
                        let value = attr
                            .unescape_value()
                            .map_err(|e| format!("Include attr decode error: {e}"))?
                            .to_string();
                        includes.push(value);
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(format!("Include scan XML error: {e}")),
            _ => {}
        }
    }

    Ok(includes)
}

async fn load_document_bundle(
    url: &str,
    client: &reqwest::Client,
) -> Result<LoadedDocumentBundle, String> {
    let root_xml = load_text_resource(url, client).await?;
    let mut bundle = LoadedDocumentBundle {
        root_xml: root_xml.clone(),
        includes: HashMap::new(),
        warnings: Vec::new(),
    };
    let mut queue = VecDeque::from([(url.to_string(), root_xml)]);
    let mut visited = HashSet::from([url.to_string()]);

    while let Some((base_url, xml)) = queue.pop_front() {
        let include_sources = extract_include_sources(&xml)?;

        for src in include_sources {
            let Some(final_url) = resolve_remote_path(&base_url, &src) else {
                bundle.warnings.push(format!(
                    "include: cannot resolve src='{src}' against base='{base_url}'"
                ));
                continue;
            };
            if !visited.insert(final_url.clone()) {
                continue;
            }

            match load_text_resource(&final_url, client).await {
                Ok(include_xml) => {
                    queue.push_back((final_url.clone(), include_xml.clone()));
                    bundle.includes.insert(final_url, include_xml);
                }
                Err(error) => {
                    bundle
                        .warnings
                        .push(format!("include: load error {final_url} -> {error}"));
                }
            }
        }
    }

    Ok(bundle)
}

async fn write_bytes_atomic(path: PathBuf, bytes: Vec<u8>) -> Result<(), String> {
    let tmp_path = PathBuf::from(format!("{}.tmp", path.display()));
    task::spawn_blocking(move || {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let _ = std::fs::remove_file(&tmp_path);
        std::fs::write(&tmp_path, bytes)?;
        if path.exists() {
            let _ = std::fs::remove_file(&path);
        }
        std::fs::rename(&tmp_path, &path)?;
        Ok::<(), std::io::Error>(())
    })
    .await
    .map_err(|e| format!("Join error writing cache file: {e}"))?
    .map_err(|e| format!("Write cache file failed: {e}"))
}

async fn copy_file_atomic(from: PathBuf, to: PathBuf) -> Result<(), String> {
    let tmp_path = PathBuf::from(format!("{}.tmp", to.display()));
    task::spawn_blocking(move || {
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let _ = std::fs::remove_file(&tmp_path);
        std::fs::copy(&from, &tmp_path)?;
        if to.exists() {
            let _ = std::fs::remove_file(&to);
        }
        std::fs::rename(&tmp_path, &to)?;
        Ok::<(), std::io::Error>(())
    })
    .await
    .map_err(|e| format!("Join error copying cache file: {e}"))?
    .map_err(|e| format!("Copy cache file failed: {e}"))
}

async fn prepare_model_asset(url: &str, client: &reqwest::Client) -> Result<String, String> {
    let (assets_dir, cache_dir) = resolve_assets_and_cache_dirs();
    let filename = encode_url_to_filename(url);
    let local_path = cache_dir.join(filename);

    if !cache_dir.exists() {
        std::fs::create_dir_all(&cache_dir).map_err(|e| format!("Create cache dir failed: {e}"))?;
    }

    if url.starts_with("http://") || url.starts_with("https://") {
        let bytes = client
            .get(url)
            .send()
            .await
            .map_err(|e| format!("HTTP error: {e}"))?
            .error_for_status()
            .map_err(|e| format!("HTTP status error: {e}"))?
            .bytes()
            .await
            .map_err(|e| format!("Read bytes error: {e}"))?
            .to_vec();
        write_bytes_atomic(local_path.clone(), bytes).await?;
    } else {
        let from = PathBuf::from(url);
        if !from.exists() {
            return Err(format!("Local file not found: {url}"));
        }
        copy_file_atomic(from, local_path.clone()).await?;
    }

    to_assets_relative(&local_path, &assets_dir)
        .map(|path| path.replace('\\', "/"))
        .ok_or_else(|| {
            format!(
                "Cached model path is outside assets dir: {}",
                local_path.display()
            )
        })
}

fn matches_current_script_url(
    specs_world: &specs::World,
    current_url: &str,
    node_id: u32,
    resolved_url: &str,
) -> Option<u32> {
    let entities = specs_world.entities();
    let ent = entities.entity(node_id);
    if !entities.is_alive(ent) {
        return None;
    }

    let scripts = specs_world.read_storage::<Script>();
    let script = scripts.get(ent)?;
    let src = script.src.as_ref()?;
    let current_resolved = resolve_node_relative_url(specs_world, ent, current_url, src)?;
    if current_resolved != resolved_url {
        return None;
    }

    find_owner_space_id(specs_world, ent)
}

pub fn poll_io_results_system(
    io_service: Res<IoService>,
    world: Res<ElemenetWorld>,
    current_url: Res<CurrentUrl>,
    mut pending_document_loads: ResMut<PendingDocumentLoads>,
    mut log_panel: ResMut<LogPanel>,
    mut pending_scripts: ResMut<PendingScripts>,
    mut script_load_states: ResMut<ScriptLoadStates>,
    mut pending_model_loads: ResMut<PendingModelLoads>,
    mut model_load_states: ResMut<ModelLoadStates>,
    mut dirty_nodes: ResMut<DirtyNodes>,
    mut manager: NonSendMut<ScriptRuntimeManager>,
    mut pending_includes: ResMut<crate::PendingIncludes>,
    mut include_load_states: ResMut<crate::IncludeLoadStates>,
) {
    let Ok(rx) = io_service.result_rx.lock() else {
        return;
    };

    loop {
        let result = match rx.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) | Err(mpsc::TryRecvError::Disconnected) => break,
        };

        match result {
            IoResult::DocumentLoaded {
                network_id,
                epoch,
                url,
                result,
            } => {
                let status = if result.is_ok() {
                    NetworkRequestStatus::Ok
                } else {
                    NetworkRequestStatus::Error
                };
                let detail = result.as_ref().err().cloned();
                io_service.finish_request(network_id, status, detail);
                pending_document_loads
                    .0
                    .push(CompletedDocumentLoad { epoch, url, result });
            }
            IoResult::FetchCompleted {
                network_id,
                space_id,
                request_id,
                result,
            } => {
                let status = if result.is_ok() {
                    NetworkRequestStatus::Ok
                } else {
                    NetworkRequestStatus::Error
                };
                let detail = result.as_ref().err().cloned();
                io_service.finish_request(network_id, status, detail);
                if let Some(worker) = manager.contexts.get_mut(&space_id) {
                    let send_result = worker.cmd_tx.send(JsWorkerCommand::PushFetchResults(vec![(
                        request_id, result,
                    )]));
                    if send_result.is_ok() {
                        worker.needs_tick = true;
                    }
                } else {
                    log_panel.push_warn(format!(
                        "[JS][space:{space_id}] Fetch result dropped: missing JS context"
                    ));
                }
            }
            IoResult::ScriptLoaded {
                network_id,
                node_id,
                url,
                result,
            } => {
                let status = if result.is_ok() {
                    NetworkRequestStatus::Ok
                } else {
                    NetworkRequestStatus::Error
                };
                let detail = result.as_ref().err().cloned();
                io_service.finish_request(network_id, status, detail);
                match result {
                    Ok(code) => {
                        if let Some(space_id) =
                            matches_current_script_url(&world.0, &current_url.0, node_id, &url)
                        {
                            pending_scripts.0.push((space_id, url.clone(), code));
                            script_load_states
                                .0
                                .insert(node_id, ScriptLoadState::Loaded { url: url.clone() });
                            log_panel.push_info(format!(
                                "[JS][space:{space_id}] Script loaded asynchronously: {url}"
                            ));
                        } else {
                            script_load_states.0.remove(&node_id);
                        }
                    }
                    Err(error) => {
                        script_load_states.0.insert(
                            node_id,
                            ScriptLoadState::Failed {
                                url: url.clone(),
                                error: error.clone(),
                            },
                        );
                        log_panel.push_error(format!(
                            "[JS] Script load error for node {node_id} ({url}): {error}"
                        ));
                    }
                }
            }
            IoResult::ModelPrepared {
                network_id,
                url,
                result,
            } => {
                let status = if result.is_ok() {
                    NetworkRequestStatus::Ok
                } else {
                    NetworkRequestStatus::Error
                };
                let detail = result.as_ref().err().cloned();
                io_service.finish_request(network_id, status, detail);
                let waiters = pending_model_loads.take_waiters(&url);
                if waiters.is_empty() {
                    continue;
                }

                for node_id in &waiters {
                    match &result {
                        Ok(asset_path) => {
                            model_load_states.0.insert(
                                *node_id,
                                ModelLoadState::Ready {
                                    url: url.clone(),
                                    asset_path: asset_path.clone(),
                                },
                            );
                        }
                        Err(error) => {
                            model_load_states.0.insert(
                                *node_id,
                                ModelLoadState::Failed {
                                    url: url.clone(),
                                    error: error.clone(),
                                },
                            );
                        }
                    }
                }

                dirty_nodes.0.extend(waiters);

                match result {
                    Ok(asset_path) => {
                        log_panel.push_info(format!(
                            "Model prepared asynchronously: {url} -> {asset_path}"
                        ));
                    }
                    Err(error) => {
                        log_panel.push_error(format!(
                            "Model prepare failed asynchronously: {url} -> {error}"
                        ));
                    }
                }
            }
            IoResult::IncludeLoaded {
                network_id,
                parent_node_id,
                url,
                result,
            } => {
                let status = if result.is_ok() {
                    NetworkRequestStatus::Ok
                } else {
                    NetworkRequestStatus::Error
                };
                let detail = result.as_ref().err().cloned();
                io_service.finish_request(network_id, status, detail);
                match result {
                    Ok(xml) => {
                        log_panel.push_info(format!(
                            "Include loaded: {url} -> parent node {parent_node_id}"
                        ));
                        pending_includes.0.push(crate::PendingInclude {
                            parent_node_id,
                            url,
                            xml,
                        });
                    }
                    Err(error) => {
                        include_load_states.0.insert(
                            parent_node_id,
                            crate::IncludeLoadState::Failed { url: url.clone() },
                        );
                        log_panel.push_error(format!(
                            "Include load failed: {url} (parent {parent_node_id}): {error}"
                        ));
                    }
                }
            }
        }
    }
}
