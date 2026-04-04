use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::PathBuf,
    sync::{mpsc, Mutex},
};

use bevy::prelude::*;
use quick_xml::{events::Event, Reader};
use specs::WorldExt;
use tokio::{runtime::Runtime, task};
use virtual_dom::dom::hsml::Script;

use crate::{
    js::{find_owner_space_id, JsWorkerCommand, ScriptRuntimeManager},
    render::{encode_url_to_filename, resolve_remote_path},
    routes::VIRTUAL_ROUTES,
    utils::folder::{resolve_assets_and_cache_dirs, to_assets_relative},
    CurrentUrl, DirtyNodes, ElemenetWorld, LogPanel, PendingScripts,
};

pub enum IoResult {
    DocumentLoaded {
        epoch: u64,
        url: String,
        result: Result<LoadedDocumentBundle, String>,
    },
    FetchCompleted {
        space_id: u32,
        request_id: i32,
        result: Result<String, String>,
    },
    ScriptLoaded {
        node_id: u32,
        url: String,
        result: Result<String, String>,
    },
    ModelPrepared {
        url: String,
        result: Result<String, String>,
    },
}

#[derive(Resource)]
pub struct IoService {
    result_tx: mpsc::Sender<IoResult>,
    result_rx: Mutex<mpsc::Receiver<IoResult>>,
}

impl Default for IoService {
    fn default() -> Self {
        let (result_tx, result_rx) = mpsc::channel();
        Self {
            result_tx,
            result_rx: Mutex::new(result_rx),
        }
    }
}

impl IoService {
    fn sender(&self) -> mpsc::Sender<IoResult> {
        self.result_tx.clone()
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
        self.0.remove(url)
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
    let tx = io_service.sender();
    rt.spawn(async move {
        let result = load_text_resource(&url).await;
        let _ = tx.send(IoResult::FetchCompleted {
            space_id,
            request_id,
            result,
        });
    });
}

pub fn request_document_load(
    rt: &Runtime,
    io_service: &IoService,
    epoch: u64,
    url: String,
) {
    let tx = io_service.sender();
    rt.spawn(async move {
        let result = load_document_bundle(&url).await;
        let _ = tx.send(IoResult::DocumentLoaded { epoch, url, result });
    });
}

pub fn request_script_load(
    rt: &Runtime,
    io_service: &IoService,
    node_id: u32,
    url: String,
) {
    let tx = io_service.sender();
    rt.spawn(async move {
        let result = load_text_resource(&url).await;
        let _ = tx.send(IoResult::ScriptLoaded {
            node_id,
            url,
            result,
        });
    });
}

pub fn request_model_prepare(
    rt: &Runtime,
    io_service: &IoService,
    url: String,
) {
    let tx = io_service.sender();
    rt.spawn(async move {
        let result = prepare_model_asset(&url).await;
        let _ = tx.send(IoResult::ModelPrepared { url, result });
    });
}

async fn load_text_resource(url: &str) -> Result<String, String> {
    if crate::routes::VirtualRoutes::is_virtual_url(url) {
        return VIRTUAL_ROUTES
            .resolve(url)
            .ok_or_else(|| format!("Virtual route not found: {url}"));
    }

    let response = reqwest::get(url)
        .await
        .map_err(|e| format!("HTTP error: {e}"))?;
    response
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

async fn load_document_bundle(url: &str) -> Result<LoadedDocumentBundle, String> {
    let root_xml = load_text_resource(url).await?;
    let mut bundle = LoadedDocumentBundle {
        root_xml: root_xml.clone(),
        includes: HashMap::new(),
        warnings: Vec::new(),
    };
    let mut queue = VecDeque::from([(url.to_string(), root_xml)]);
    let mut visited = HashSet::new();

    while let Some((base_url, xml)) = queue.pop_front() {
        let include_sources = match extract_include_sources(&xml) {
            Ok(sources) => sources,
            Err(error) => {
                bundle.warnings.push(format!(
                    "include scan failed for {base_url}: {error}"
                ));
                continue;
            }
        };

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

            match load_text_resource(&final_url).await {
                Ok(include_xml) => {
                    queue.push_back((final_url.clone(), include_xml.clone()));
                    bundle.includes.insert(final_url, include_xml);
                }
                Err(error) => {
                    bundle.warnings.push(format!(
                        "include: load error {final_url} -> {error}"
                    ));
                }
            }
        }
    }

    Ok(bundle)
}

async fn prepare_model_asset(url: &str) -> Result<String, String> {
    let (assets_dir, cache_dir) = resolve_assets_and_cache_dirs();
    let filename = encode_url_to_filename(url);
    let local_path = cache_dir.join(filename);

    if !local_path.exists() {
        let cache_dir_for_create = cache_dir.clone();
        task::spawn_blocking(move || std::fs::create_dir_all(cache_dir_for_create))
            .await
            .map_err(|e| format!("Join error creating cache dir: {e}"))?
            .map_err(|e| format!("Create cache dir failed: {e}"))?;

        if url.starts_with("http://") || url.starts_with("https://") {
            let bytes = reqwest::get(url)
                .await
                .map_err(|e| format!("HTTP error: {e}"))?
                .bytes()
                .await
                .map_err(|e| format!("Read bytes error: {e}"))?;
            let path_for_write = local_path.clone();
            task::spawn_blocking(move || std::fs::write(path_for_write, bytes))
                .await
                .map_err(|e| format!("Join error writing cache file: {e}"))?
                .map_err(|e| format!("Write cache file failed: {e}"))?;
        } else {
            let from = PathBuf::from(url);
            if !from.exists() {
                return Err(format!("Local file not found: {url}"));
            }
            let to = local_path.clone();
            task::spawn_blocking(move || std::fs::copy(from, to))
                .await
                .map_err(|e| format!("Join error copying cache file: {e}"))?
                .map_err(|e| format!("Copy cache file failed: {e}"))?;
        }
    }

    to_assets_relative(&local_path, &assets_dir)
        .map(|path| path.replace('\\', "/"))
        .ok_or_else(|| format!("Cached model path is outside assets dir: {}", local_path.display()))
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
    let current_resolved = resolve_remote_path(current_url, src)?;
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
) {
    let Ok(rx) = io_service.result_rx.lock() else { return; };

    loop {
        let result = match rx.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) | Err(mpsc::TryRecvError::Disconnected) => break,
        };

        match result {
            IoResult::DocumentLoaded { epoch, url, result } => {
                pending_document_loads.0.push(CompletedDocumentLoad { epoch, url, result });
            }
            IoResult::FetchCompleted {
                space_id,
                request_id,
                result,
            } => {
                if let Some(worker) = manager.contexts.get_mut(&space_id) {
                    let _ = worker.cmd_tx.send(JsWorkerCommand::PushFetchResults(vec![(
                        request_id,
                        result,
                    )]));
                } else {
                    log_panel.push_warn(format!(
                        "[JS][space:{space_id}] Fetch result dropped: missing JS context"
                    ));
                }
            }
            IoResult::ScriptLoaded {
                node_id,
                url,
                result,
            } => {
                match result {
                    Ok(code) => {
                        if let Some(space_id) =
                            matches_current_script_url(&world.0, &current_url.0, node_id, &url)
                        {
                            pending_scripts.0.push((space_id, url.clone(), code));
                            script_load_states.0.insert(
                                node_id,
                                ScriptLoadState::Loaded { url: url.clone() },
                            );
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
            IoResult::ModelPrepared { url, result } => {
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
        }
    }
}
