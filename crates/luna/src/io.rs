use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::PathBuf,
    sync::Mutex,
    time::{Duration, Instant},
};

use bevy::prelude::*;
use quick_xml::{events::Event, Reader};
use specs::WorldExt;
use tokio::{runtime::Runtime, sync::mpsc, task};
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
        result: Result<js_runtime::FetchResponse, String>,
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
    SkyboxPrepared {
        network_id: u64,
        key: String,
        result: Result<[String; 6], String>,
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
    Skybox,
    Include,
    Image,
}

impl NetworkRequestKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Document => "document",
            Self::Fetch => "fetch",
            Self::Script => "script",
            Self::Model => "model",
            Self::Skybox => "skybox",
            Self::Include => "include",
            Self::Image => "image",
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
    fetch_client: reqwest::Client,
    network_tracker: Mutex<NetworkTracker>,
    pending_fetch_results: Mutex<HashMap<u32, VecDeque<(i32, Result<js_runtime::FetchResponse, String>)>>>,
    fetch_tasks: Mutex<HashMap<u32, HashMap<u64, Option<tokio::task::AbortHandle>>>>,
}

impl Default for IoService {
    fn default() -> Self {
        const IO_RESULT_CAPACITY: usize = 256;
        let (result_tx, result_rx) = mpsc::channel(IO_RESULT_CAPACITY);
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
            fetch_client: reqwest::Client::builder().redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(20)).connect_timeout(Duration::from_secs(10))
                .user_agent("Luna/0.1").build().expect("fetch HTTP client"),
            network_tracker: Mutex::new(NetworkTracker::default()),
            pending_fetch_results: Mutex::new(HashMap::new()),
            fetch_tasks: Mutex::new(HashMap::new()),
        }
    }
}

impl IoService {
    const MAX_OUTSTANDING_FETCHES_PER_SPACE: usize = 64;
    const PENDING_FETCH_RESULTS_PER_SPACE: usize = Self::MAX_OUTSTANDING_FETCHES_PER_SPACE;

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

    fn reserve_fetch_task(&self, space_id: u32, network_id: u64) -> bool {
        let pending_count = self
            .pending_fetch_results
            .lock()
            .ok()
            .and_then(|pending| pending.get(&space_id).map(VecDeque::len))
            .unwrap_or(0);
        let Ok(mut tasks) = self.fetch_tasks.lock() else {
            return false;
        };
        let by_id = tasks.entry(space_id).or_default();
        if by_id.len() + pending_count >= Self::MAX_OUTSTANDING_FETCHES_PER_SPACE {
            return false;
        }
        by_id.insert(network_id, None);
        true
    }

    fn track_fetch_task(
        &self,
        space_id: u32,
        network_id: u64,
        abort: tokio::task::AbortHandle,
    ) {
        if let Ok(mut tasks) = self.fetch_tasks.lock() {
            if let Some(slot) = tasks
                .get_mut(&space_id)
                .and_then(|by_id| by_id.get_mut(&network_id))
            {
                *slot = Some(abort);
            } else {
                abort.abort();
            }
        }
    }

    fn finish_fetch_task(&self, space_id: u32, network_id: u64) {
        if let Ok(mut tasks) = self.fetch_tasks.lock() {
            if let Some(by_id) = tasks.get_mut(&space_id) {
                by_id.remove(&network_id);
                if by_id.is_empty() {
                    tasks.remove(&space_id);
                }
            }
        }
    }

    fn defer_fetch_results(
        &self,
        space_id: u32,
        results: Vec<(i32, Result<js_runtime::FetchResponse, String>)>,
    ) {
        let Ok(mut pending) = self.pending_fetch_results.lock() else {
            return;
        };
        let queue = pending.entry(space_id).or_default();
        for result in results {
            if queue.len() == Self::PENDING_FETCH_RESULTS_PER_SPACE {
                queue.pop_front();
            }
            queue.push_back(result);
        }
    }

    pub fn cancel_space(&self, space_id: u32) {
        if let Ok(mut tasks) = self.fetch_tasks.lock() {
            if let Some(by_id) = tasks.remove(&space_id) {
                for (_, abort) in by_id {
                    if let Some(abort) = abort {
                        abort.abort();
                    }
                }
            }
        }
        if let Ok(mut pending) = self.pending_fetch_results.lock() {
            pending.remove(&space_id);
        }
    }

    pub fn cancel_all_spaces(&self) {
        if let Ok(mut tasks) = self.fetch_tasks.lock() {
            for (_, by_id) in tasks.drain() {
                for (_, abort) in by_id {
                    if let Some(abort) = abort {
                        abort.abort();
                    }
                }
            }
        }
        if let Ok(mut pending) = self.pending_fetch_results.lock() {
            pending.clear();
        }
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
    Prepared { url: String, asset_path: String },
    Failed { url: String, error: String },
}

#[derive(Resource, Default)]
pub struct ModelLoadStates(pub HashMap<u32, ModelLoadState>);

#[derive(Resource, Default)]
pub struct PendingModelLoads(pub HashMap<String, HashSet<u32>>, pub HashMap<String, u64>);

impl PendingModelLoads {
    pub fn enqueue(&mut self, url: &str, node_id: u32) -> bool {
        let waiters = self.0.entry(url.to_string()).or_default();
        let should_spawn = waiters.is_empty();
        waiters.insert(node_id);
        should_spawn
    }

    pub fn finish(&mut self, url: &str, request_id: u64) -> Vec<u32> {
        if self.1.get(url) != Some(&request_id) { return Vec::new(); }
        self.take_waiters(url)
    }

    pub fn clear(&mut self) { self.0.clear(); self.1.clear(); }

    pub fn take_waiters(&mut self, url: &str) -> Vec<u32> {
        self.1.remove(url);
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
        self.1.retain(|url, _| self.0.contains_key(url));
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
    request: js_runtime::FetchRequest,
    origin: Option<String>,
) -> Result<(), String> {
    let network_id = io_service.begin_request(
        NetworkRequestKind::Fetch,
        request.url.clone(),
        format!("space:{space_id}"),
    );
    if !io_service.reserve_fetch_task(space_id, network_id) {
        io_service.finish_request(
            network_id,
            NetworkRequestStatus::Error,
            Some("too many outstanding fetches for this space".to_string()),
        );
        return Err(format!(
            "fetch limit reached ({} outstanding requests per space)",
            IoService::MAX_OUTSTANDING_FETCHES_PER_SPACE
        ));
    }
    let tx = io_service.sender();
    let client = io_service.fetch_client.clone();
    let task = rt.spawn(async move {
        let result = tokio::time::timeout(Duration::from_secs(30),
            crate::http_fetch::execute(request, origin, &client)).await
            .unwrap_or_else(|_| Err("Fetch timed out".into()));
        let _ = tx.send(IoResult::FetchCompleted {
            network_id,
            space_id,
            request_id,
            result,
        }).await;
    });
    io_service.track_fetch_task(space_id, network_id, task.abort_handle());
    Ok(())
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
        }).await;
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
        }).await;
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
        }).await;
    });
}

pub fn request_model_prepare(rt: &Runtime, io_service: &IoService, url: String) -> u64 {
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
        }).await;
    });
    network_id
}

pub fn request_skybox_prepare(
    rt: &Runtime,
    io_service: &IoService,
    key: String,
    face_urls: [String; 6],
) {
    let network_id =
        io_service.begin_request(NetworkRequestKind::Skybox, key.clone(), "skybox-cache");
    let tx = io_service.sender();
    let client = io_service.http_client();
    rt.spawn(async move {
        let result = prepare_skybox_assets(face_urls, &client).await;
        let _ = tx.send(IoResult::SkyboxPrepared {
            network_id,
            key,
            result,
        }).await;
    });
}

pub(crate) fn same_origin_url(base: &str, requested: &str) -> Result<String, String> {
    let base = url::Url::parse(base).map_err(|_| "Invalid document URL")?;
    let target = base.join(requested).map_err(|_| "Invalid fetch URL")?;
    if !matches!(base.scheme(), "http" | "https") || !matches!(target.scheme(), "http" | "https")
        || target.origin() != base.origin() || !target.username().is_empty() || target.password().is_some() {
        return Err("fetch_text only allows the document's HTTP(S) origin".into());
    }
    Ok(target.to_string())
}

#[cfg(test)]
mod same_origin_tests {
    use super::*;
    #[test]
    fn resolves_relative_urls_and_rejects_origin_changes() {
        let base = "https://example.test:443/app/index.hsml";
        assert_eq!(same_origin_url(base, "api?q=1").unwrap(), "https://example.test/app/api?q=1");
        for target in ["http://example.test/", "https://example.test:444/", "//evil.test/", "file:///secret", "luna://settings", "https://user:pass@example.test/"] {
            assert!(same_origin_url(base, target).is_err(), "{target}");
        }
    }
    #[test]
    fn follows_local_redirects_but_never_sends_cross_origin_redirect() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let foreign = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        foreign.set_nonblocking(true).unwrap();
        let target = format!("http://{}/secret", foreign.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            for response in [
                "HTTP/1.1 302 Found\r\nLocation: /ok\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_string(),
                "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok".to_string(),
                format!("HTTP/1.1 302 Found\r\nLocation: {target}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"),
                "HTTP/1.1 200 OK\r\nContent-Length: 99999999\r\nConnection: close\r\n\r\n".to_string(),
            ] {
                let (mut stream, _) = listener.accept().unwrap();
                stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                let mut buf = [0; 4096]; let _ = stream.read(&mut buf);
                stream.write_all(response.as_bytes()).unwrap();
            }
        });
        Runtime::new().unwrap().block_on(async {
            let client = reqwest::Client::builder().no_proxy().redirect(reqwest::redirect::Policy::none()).timeout(Duration::from_secs(5)).build().unwrap();
            assert_eq!(load_same_origin_text(&url, &url, &client).await.unwrap(), "ok");
            assert!(load_same_origin_text(&url, &url, &client).await.unwrap_err().contains("origin"));
            assert!(load_same_origin_text(&url, &url, &client).await.unwrap_err().contains("8 MiB"));
        });
        assert_eq!(foreign.accept().unwrap_err().kind(), std::io::ErrorKind::WouldBlock);
        server.join().unwrap();
    }
}

#[cfg(test)]
async fn load_same_origin_text(url: &str, origin: &str, client: &reqwest::Client) -> Result<String, String> {
    crate::http_fetch::execute(js_runtime::FetchRequest { url:url.into(), method:"GET".into(),
        headers:vec![], body:None, redirect:"follow".into() }, Some(origin.into()), client)
        .await.map(|response| response.body)
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
        bundle.warnings.extend(crate::diagnostics::document_warnings(&xml).into_iter().map(|m| format!("[diagnostic] {base_url}: {m}")));
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

fn model_resource_filename(url: &str, bytes: &[u8]) -> String {
    let path = url::Url::parse(url).ok().map(|u| u.path().to_string())
        .unwrap_or_else(|| url.to_string());
    let ext = std::path::Path::new(&path).extension().and_then(|e| e.to_str()).unwrap_or("bin");
    let mut hash = blake3::Hasher::new();
    hash.update(url.as_bytes());
    hash.update(&[0]);
    hash.update(bytes);
    format!("model-{}.{}", hash.finalize().to_hex(), ext)
}

async fn prepare_model_asset(url: &str, client: &reqwest::Client) -> Result<String, String> {
    let (assets_dir, cache_dir) = resolve_assets_and_cache_dirs();
    let bytes = if url.starts_with("http://") || url.starts_with("https://") {
        client.get(url).send().await.map_err(|e| format!("HTTP error: {e}"))?
            .error_for_status().map_err(|e| format!("HTTP status error: {e}"))?
            .bytes().await.map_err(|e| format!("Read bytes error: {e}"))?.to_vec()
    } else {
        tokio::fs::read(url).await.map_err(|e| format!("Read model error: {e}"))?
    };
    // Immutable revisions prevent a late download from overwriting a newer asset.
    // Equal URL/content pairs reuse the same Bevy asset handles.
    let local_path = cache_dir.join(model_resource_filename(url, &bytes));
    let write_path = local_path.clone();
    task::spawn_blocking(move || -> Result<(), String> {
        std::fs::create_dir_all(&cache_dir).map_err(|e| e.to_string())?;
        if !write_path.exists() {
            static NEXT_TEMP: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let id = NEXT_TEMP.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let tmp = write_path.with_extension(format!("{}.{}.tmp", std::process::id(), id));
            std::fs::write(&tmp, bytes).map_err(|e| e.to_string())?;
            if let Err(error) = std::fs::rename(&tmp, &write_path) {
                let _ = std::fs::remove_file(&tmp);
                if !write_path.exists() { return Err(error.to_string()); }
            }
        }
        Ok(())
    }).await.map_err(|e| e.to_string())??;
    to_assets_relative(&local_path, &assets_dir)
        .map(|path| path.replace('\\', "/"))
        .ok_or_else(|| "Model cache path is outside assets dir".into())
}

async fn prepare_cached_image(url: String, client: reqwest::Client) -> Result<String, String> {
    let (assets_dir, cache_dir) = resolve_assets_and_cache_dirs();
    let local_path = cache_dir.join(encode_url_to_filename(&url));

    if !local_path.exists() {
        if url.starts_with("http://") || url.starts_with("https://") {
            let bytes = client
                .get(&url)
                .send()
                .await
                .map_err(|e| format!("HTTP error for '{url}': {e}"))?
                .error_for_status()
                .map_err(|e| format!("HTTP status error for '{url}': {e}"))?
                .bytes()
                .await
                .map_err(|e| format!("Read bytes error for '{url}': {e}"))?
                .to_vec();
            write_bytes_atomic(local_path.clone(), bytes).await?;
        } else {
            let from = PathBuf::from(&url);
            if !from.exists() {
                return Err(format!("Local skybox face not found: {url}"));
            }
            copy_file_atomic(from, local_path.clone()).await?;
        }
    }

    to_assets_relative(&local_path, &assets_dir)
        .map(|path| path.replace('\\', "/"))
        .ok_or_else(|| {
            format!(
                "Cached skybox face is outside assets dir: {}",
                local_path.display()
            )
        })
}

async fn prepare_skybox_assets(
    face_urls: [String; 6],
    client: &reqwest::Client,
) -> Result<[String; 6], String> {
    let unique_urls = face_urls.iter().cloned().collect::<HashSet<_>>();
    let preparations = unique_urls.into_iter().map(|url| {
        let prepared_url = url.clone();
        let client = client.clone();
        async move {
            let path = prepare_cached_image(url, client).await?;
            Ok::<_, String>((prepared_url, path))
        }
    });
    let prepared_by_url = futures_util::future::try_join_all(preparations)
        .await?
        .into_iter()
        .collect::<HashMap<_, _>>();

    let mut paths = Vec::with_capacity(face_urls.len());
    for url in face_urls {
        let path = prepared_by_url
            .get(&url)
            .cloned()
            .ok_or_else(|| format!("Missing prepared skybox face: {url}"))?;
        paths.push(path);
    }
    paths
        .try_into()
        .map_err(|_| "Skybox preparation returned an invalid face count".to_string())
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
    mut skybox: ResMut<crate::SkyboxEntity>,
    mut dirty_nodes: ResMut<DirtyNodes>,
    mut manager: NonSendMut<ScriptRuntimeManager>,
    mut pending_includes: ResMut<crate::PendingIncludes>,
    mut include_load_states: ResMut<crate::IncludeLoadStates>,
) {
    let deferred_fetches = io_service
        .pending_fetch_results
        .lock()
        .map(|mut pending| std::mem::take(&mut *pending))
        .unwrap_or_default();
    for (space_id, results) in deferred_fetches {
        let results = results.into_iter().collect::<Vec<_>>();
        let Some(worker) = manager.contexts.get_mut(&space_id) else {
            continue;
        };
        match worker
            .cmd_tx
            .try_send(JsWorkerCommand::PushFetchResults(results))
        {
            Ok(()) => worker.needs_tick = true,
            Err(std::sync::mpsc::TrySendError::Full(
                JsWorkerCommand::PushFetchResults(results),
            )) => io_service.defer_fetch_results(space_id, results),
            Err(_) => {}
        }
    }

    let Ok(mut rx) = io_service.result_rx.lock() else {
        return;
    };

    loop {
        let result = match rx.try_recv() {
            Ok(result) => result,
            Err(mpsc::error::TryRecvError::Empty)
            | Err(mpsc::error::TryRecvError::Disconnected) => break,
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
                io_service.finish_fetch_task(space_id, network_id);
                let status = if result.as_ref().is_ok_and(|response| response.status < 400) {
                    NetworkRequestStatus::Ok
                } else {
                    NetworkRequestStatus::Error
                };
                let detail = Some(match &result {
                    Ok(response) => format!("HTTP {} {}", response.status, response.status_text),
                    Err(error) => error.clone(),
                });
                io_service.finish_request(network_id, status, detail);
                if let Some(worker) = manager.contexts.get_mut(&space_id) {
                    match worker.cmd_tx.try_send(JsWorkerCommand::PushFetchResults(vec![(
                        request_id, result,
                    )])) {
                        Ok(()) => worker.needs_tick = true,
                        Err(std::sync::mpsc::TrySendError::Full(
                            JsWorkerCommand::PushFetchResults(results),
                        )) => io_service.defer_fetch_results(space_id, results),
                        Err(_) => {}
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
                let waiters: Vec<_> = pending_model_loads.finish(&url, network_id).into_iter()
                    .filter(|id| matches!(model_load_states.0.get(id),
                        Some(ModelLoadState::Requested { url: requested }) if requested == &url))
                    .collect();
                if waiters.is_empty() {
                    continue;
                }

                for node_id in &waiters {
                    match &result {
                        Ok(asset_path) => {
                            model_load_states.0.insert(
                                *node_id,
                                ModelLoadState::Prepared {
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
            IoResult::SkyboxPrepared {
                network_id,
                key,
                result,
            } => {
                let status = if result.is_ok() {
                    NetworkRequestStatus::Ok
                } else {
                    NetworkRequestStatus::Error
                };
                let detail = result.as_ref().err().cloned();
                io_service.finish_request(network_id, status, detail);
                let waiters = skybox.take_waiters(&key);

                for node_id in waiters {
                    let Some(node_state) = skybox.nodes.get_mut(&node_id) else {
                        continue;
                    };
                    if node_state.key != key
                        || !matches!(node_state.status, crate::SkyboxLoadStatus::Requested)
                    {
                        continue;
                    }

                    match &result {
                        Ok(asset_paths) => {
                            node_state.status = crate::SkyboxLoadStatus::Ready {
                                asset_paths: asset_paths.clone(),
                            };
                            log_panel.push_info(format!(
                                "Skybox prepared asynchronously for node {node_id}"
                            ));
                        }
                        Err(error) => {
                            node_state.status = crate::SkyboxLoadStatus::Failed {
                                error: error.clone(),
                            };
                            log_panel.push_error(format!(
                                "Skybox prepare failed for node {node_id}: {error}"
                            ));
                        }
                    }
                    dirty_nodes.0.push(node_id);
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

#[cfg(test)]
mod backpressure_tests {
    use super::*;

    #[test]
    fn deferred_fetch_results_are_bounded_and_cleared_per_space() {
        let service = IoService::default();
        let space_id = 17;

        for request_id in 0..(IoService::PENDING_FETCH_RESULTS_PER_SPACE as i32 + 20) {
            service.defer_fetch_results(space_id, vec![(request_id, Ok(js_runtime::FetchResponse { body: "ok".into(), ..Default::default() }))]);
        }

        let pending = service.pending_fetch_results.lock().unwrap();
        let queue = pending.get(&space_id).unwrap();
        assert_eq!(queue.len(), IoService::PENDING_FETCH_RESULTS_PER_SPACE);
        assert_eq!(queue.front().map(|(id, _)| *id), Some(20));
        drop(pending);

        service.cancel_space(space_id);
        assert!(!service
            .pending_fetch_results
            .lock()
            .unwrap()
            .contains_key(&space_id));
    }
}

#[cfg(test)]
mod model_resource_tests {
    use super::*;
    #[test]
    fn obsolete_completion_cannot_consume_replacement_waiters() {
        let mut pending = PendingModelLoads::default();
        assert!(pending.enqueue("a", 7)); pending.1.insert("a".into(), 1);
        pending.remove_node(7);
        assert!(pending.enqueue("a", 7)); pending.1.insert("a".into(), 2);
        assert!(pending.finish("a", 1).is_empty());
        assert!(!pending.enqueue("a", 8));
        let mut waiters = pending.finish("a", 2); waiters.sort();
        assert_eq!(waiters, vec![7, 8]);
        assert!(pending.finish("a", 2).is_empty());
        assert!(pending.enqueue("b", 7)); pending.1.insert("b".into(), 3);
        pending.clear();
        assert!(pending.finish("b", 3).is_empty());
    }
    #[test]
    fn content_revisions_are_immutable_and_query_does_not_break_extension() {
        let url = "https://example.test/butterfly.glb?version=2";
        let first = model_resource_filename(url, b"first");
        assert!(first.ends_with(".glb"));
        assert_eq!(first, model_resource_filename(url, b"first"));
        assert_ne!(first, model_resource_filename(url, b"second"));
    }
}
