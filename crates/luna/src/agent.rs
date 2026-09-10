//! Local browser automation owned by the host, independent of page isolates.
use crate::{
    DesktopCamera, NextTabId, RenderMode, SpaceMountQueue, SpaceMountRequest, TokioRuntime,
};
use bevy::prelude::*;
use bevy_mod_xr::session::XrState;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::tungstenite::Message;

pub type Reply = oneshot::Sender<Result<Value, String>>;
struct Request {
    command: Command,
    reply: Reply,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "cmd", content = "args")]
enum Command {
    #[serde(rename = "status")]
    Status {},
    #[serde(rename = "logs")]
    Logs {
        #[serde(default)] limit: Option<usize>,
        #[serde(default)] after: Option<u64>,
        #[serde(default, rename = "tabId")] tab_id: Option<u64>,
        #[serde(default, rename = "spaceId")] space_id: Option<u32>,
        #[serde(default)] level: Option<String>,
        #[serde(default)] pattern: Option<String>,
    },
    #[serde(rename = "open")]
    Open { url: String },
    #[serde(rename = "capture")]
    Capture {
        #[serde(default)]
        camera: CameraChoice,
    },
    #[serde(rename = "camera")]
    Camera {
        #[serde(default)]
        camera: CameraChoice,
        position: [f32; 3],
        #[serde(rename = "lookAt")]
        look_at: [f32; 3],
    },
}

#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum CameraChoice {
    #[default]
    Auto,
    Desktop,
    Spectator,
}

#[derive(Resource, Default)]
pub struct AgentControl {
    pub enabled: bool,
    connected: Arc<AtomicBool>,
    task: Option<tokio::task::JoinHandle<()>>,
    incoming: Option<Mutex<mpsc::Receiver<Request>>>,
}
impl AgentControl {
    pub fn connection_label(&self) -> &'static str {
        if !self.enabled {
            "MCP disabled"
        } else if self.connected.load(Ordering::Relaxed) {
            "MCP connected (localhost)"
        } else {
            "MCP waiting for local adapter"
        }
    }
}
impl Drop for AgentControl {
    fn drop(&mut self) {
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}

pub fn is_settings_document(url: &str) -> bool {
    url::Url::parse(url).is_ok_and(|u| {
        u.scheme() == "luna"
            && matches!(u.host_str(), Some("settings" | "agent" | "agent_app"))
            && matches!(u.path(), "" | "/")
            && u.username().is_empty()
            && u.password().is_none()
            && u.port().is_none()
    })
}

async fn connect_loop(tx: mpsc::Sender<Request>, connected: Arc<AtomicBool>) {
    let port = std::env::var("LUNA_AGENT_PORT")
        .ok()
        .and_then(|s| s.parse::<u16>().ok())
        .filter(|p| *p > 0)
        .unwrap_or(2054);
    loop {
        if let Ok((mut socket, _)) =
            tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}/")).await
        {
            connected.store(true, Ordering::Relaxed);
            let _ = socket
                .send(Message::Text(
                    json!({"hello":"luna-host","version":2}).to_string(),
                ))
                .await;
            // Replies belong to this socket. Dropping it also drops outstanding receivers.
            // Serial dispatch keeps camera moves and captures in the caller's order.
            while let Some(Ok(message)) = socket.next().await {
                if message.is_close() {
                    break;
                }
                if message.is_ping() {
                    let _ = socket.send(Message::Pong(message.into_data())).await;
                    continue;
                }
                let Ok(text) = message.to_text() else {
                    continue;
                };
                if text.len() > 16_384 {
                    break;
                }
                let Ok(value) = serde_json::from_str::<Value>(text) else {
                    continue;
                };
                let Some(id) = value.get("id").and_then(Value::as_u64) else {
                    continue;
                };
                let result = match serde_json::from_value::<Command>(value) {
                    Err(error) => Err(format!("invalid command: {error}")),
                    Ok(command) => {
                        let (reply, rx) = oneshot::channel();
                        if tx.try_send(Request { command, reply }).is_err() {
                            Err("host busy".into())
                        } else {
                            match tokio::time::timeout(std::time::Duration::from_secs(25), rx).await
                            {
                                Ok(Ok(result)) => result,
                                _ => Err("host response timed out".into()),
                            }
                        }
                    }
                };
                let response = match result {
                    Ok(result) => json!({"id":id,"ok":true,"result":result}),
                    Err(error) => json!({"id":id,"ok":false,"error":error}),
                };
                if socket
                    .send(Message::Text(response.to_string()))
                    .await
                    .is_err()
                {
                    break;
                }
            }
        }
        connected.store(false, Ordering::Relaxed);
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
}

#[derive(Component)]
pub struct SpectatorCamera;

fn agent_system(world: &mut World) {
    let enabled = world.resource::<AgentControl>().enabled;
    if !enabled {
        let mut control = world.resource_mut::<AgentControl>();
        if let Some(task) = control.task.take() {
            task.abort();
        }
        control.incoming = None;
        control.connected.store(false, Ordering::Relaxed);
        return;
    }
    if world.resource::<AgentControl>().task.is_none() {
        let (tx, rx) = mpsc::channel(32);
        let connected = world.resource::<AgentControl>().connected.clone();
        let task = world
            .resource::<TokioRuntime>()
            .0
            .spawn(connect_loop(tx, connected));
        let mut control = world.resource_mut::<AgentControl>();
        control.task = Some(task);
        control.incoming = Some(Mutex::new(rx));
    }
    let requests: Vec<_> = {
        let control = world.resource::<AgentControl>();
        let mut rx = control.incoming.as_ref().unwrap().lock().unwrap();
        std::iter::from_fn(|| rx.try_recv().ok()).take(32).collect()
    };
    for request in requests {
        if request.reply.is_closed() {
            continue;
        }
        let vr = world.resource::<RenderMode>().is_vr;
        let result = match request.command {
            Command::Status {} => Ok(status(world)),
            Command::Logs { limit, after, tab_id, space_id, level, pattern } =>
                query_logs(world.resource::<crate::LogPanel>(), limit.unwrap_or(100), after, tab_id, space_id, level.as_deref(), pattern.as_deref()),
            Command::Open { url } => open(world, url),
            Command::Camera {
                camera,
                position,
                look_at,
            } => move_camera(world, choose(camera, vr), position, look_at),
            Command::Capture { camera } => {
                crate::agent_capture::capture(world, choose(camera, vr), request.reply);
                continue;
            }
        };
        let _ = request.reply.send(result);
    }
}

fn query_logs(panel: &crate::LogPanel, limit: usize, after: Option<u64>, tab: Option<u64>, space: Option<u32>, level: Option<&str>, pattern: Option<&str>) -> Result<Value, String> {
    if !(1..=300).contains(&limit) { return Err("limit must be between 1 and 300".into()); }
    if level.is_some_and(|l| !matches!(l, "info" | "warn" | "error")) { return Err("Invalid log level".into()); }
    let regex = pattern.map(|p| {
        if p.len() > 512 { return Err("pattern exceeds 512 bytes".to_string()); }
        regex::RegexBuilder::new(p).size_limit(1_000_000).dfa_size_limit(1_000_000).build().map_err(|e| e.to_string())
    }).transpose()?;
    let label = |l| match l { crate::LogLevel::Info => "info", crate::LogLevel::Warn => "warn", crate::LogLevel::Error => "error" };
    let matches: Vec<_> = panel.logs.iter().filter(|e|
        after.is_none_or(|n| e.sequence > n) && tab.is_none_or(|id| e.tab_id == Some(id))
        && space.is_none_or(|id| e.space_id == Some(id)) && level.is_none_or(|l| label(e.level) == l)
        && regex.as_ref().is_none_or(|r| r.is_match(&e.message))).collect();
    // Tail for the first request; ordered pagination when a cursor is supplied.
    let start = if after.is_some() { 0 } else { matches.len().saturating_sub(limit) };
    let selected: Vec<_> = matches.iter().skip(start).take(limit).collect();
    let next = selected.last().map(|e| e.sequence).or(after).unwrap_or(0);
    let entries: Vec<_> = selected.iter().map(|e| json!({"sequence":e.sequence,"timestampMs":e.timestamp_ms,
        "tabId":e.tab_id,"spaceId":e.space_id,"runtimeId":e.runtime_id,"level":label(e.level),"message":e.message})).collect();
    Ok(json!({"entries":entries,"nextCursor":next,"hasMore":after.is_some() && matches.len() > limit,
        "retention":"up to 300 per space, 3000 total; messages up to 4096 bytes",
        "oldestAvailable":panel.logs.first().map(|e|e.sequence)}))
}

#[cfg(test)]
mod log_query_tests {
    use super::*;
    #[test]
    fn filters_pages_and_preserves_other_producers() {
        let mut panel = crate::LogPanel::default();
        let mut entry = crate::LogEntry::with_space(crate::LogLevel::Warn, "target warning", 7);
        entry.tab_id = Some(42); entry.runtime_id = Some(3);
        panel.push_entry(entry);
        for n in 0..400 { panel.push_for_space(crate::LogLevel::Info, format!("noisy {n}"), 8); }
        assert_eq!(panel.logs.len(), 301);
        let found = query_logs(&panel, 10, None, Some(42), None, Some("warn"), Some("target.*")).unwrap();
        assert_eq!(found["entries"].as_array().unwrap().len(), 1);
        assert_eq!(found["entries"][0]["runtimeId"], 3);
        let first = query_logs(&panel, 2, Some(0), None, Some(8), None, None).unwrap();
        let next = query_logs(&panel, 2, first["nextCursor"].as_u64(), None, Some(8), None, None).unwrap();
        assert!(next["entries"][0]["sequence"].as_u64() > first["nextCursor"].as_u64());
        assert_eq!(first["hasMore"], true);
        assert!(query_logs(&panel, 301, None, None, None, None, None).is_err());
        assert!(query_logs(&panel, 1, None, None, None, None, Some("[")).is_err());
    }
}

fn choose(camera: CameraChoice, vr: bool) -> CameraChoice {
    match camera {
        CameraChoice::Auto if vr => CameraChoice::Spectator,
        CameraChoice::Auto => CameraChoice::Desktop,
        other => other,
    }
}

fn status(world: &mut World) -> Value {
    let vr = world.resource::<RenderMode>().is_vr;
    let xr = world
        .get_resource::<XrState>()
        .copied()
        .unwrap_or(XrState::Unavailable);
    let cameras: Vec<_> = world.query::<(&Camera, &Transform, Option<&DesktopCamera>, Option<&SpectatorCamera>)>()
        .iter(world).filter_map(|(c,t,d,s)| {
            let name = if d.is_some() { "desktop" } else if s.is_some() { "spectator" } else { return None; };
            Some(json!({"name":name,"active":c.is_active,"position":t.translation.to_array(),"forward":t.forward().to_array()}))
        }).collect();
    let snapshots =
        crate::ui::collect_mounted_space_snapshots(&world.resource::<crate::ElemenetWorld>().0);
    let mut entries: Vec<_> = snapshots.values().collect();
    entries.sort_by_key(|s| s.tab_id);
    let spaces = entries
        .iter()
        .map(|s| json!({"tabId":s.tab_id,"url":s.url,"title":s.title}))
        .collect::<Vec<_>>();
    let active_id = world
        .resource::<crate::ActiveSpaceIndex>()
        .0
        .and_then(|i| world.resource::<crate::MountedSpaceList>().0.get(i))
        .map(|s| s.tab_id);
    let active = spaces
        .iter()
        .find(|s| s["tabId"].as_u64() == active_id)
        .cloned();
    let includes = world
        .resource::<crate::IncludeLoadStates>()
        .0
        .iter()
        .map(|(id, state)| {
            let (state, url) = match state {
                crate::IncludeLoadState::Loading { url } => ("loading", url),
                crate::IncludeLoadState::Loaded { url } => ("loaded", url),
                crate::IncludeLoadState::Failed { url } => ("failed", url),
            };
            json!({"nodeId":id,"state":state,"url":url})
        })
        .collect::<Vec<_>>();
    let queued = world
        .resource::<SpaceMountQueue>()
        .0
        .iter()
        .map(|s| json!({"tabId":s.tab_id,"url":s.url}))
        .collect::<Vec<_>>();
    json!({"connected":true,"requestedMode":if vr {"vr"} else {"desktop"},
        "xrState":format!("{xr:?}"), "effectiveMode":if xr == XrState::Running {"vr"} else if !vr {"desktop"} else {"waiting-for-xr"},
        "defaultCamera":if vr {"spectator"} else {"desktop"},"cameras":cameras,
        "activeSpace":active,"spaces":spaces,"queuedNavigation":queued,"includes":includes})
}

fn open(world: &mut World, url: String) -> Result<Value, String> {
    let parsed = url::Url::parse(&url).map_err(|e| e.to_string())?;
    if url.len() > 8192 || !matches!(parsed.scheme(), "luna" | "http" | "https") {
        return Err("expected luna://, http:// or https:// URL".into());
    }
    let mut next = world.resource_mut::<NextTabId>();
    let id = next.0;
    next.0 = next.0.checked_add(1).ok_or("tab IDs exhausted")?;
    world
        .resource_mut::<SpaceMountQueue>()
        .0
        .push(SpaceMountRequest::new(id, url.clone()));
    Ok(json!({"tabId":id,"url":url,"accepted":true,"loading":true}))
}

fn move_camera(
    world: &mut World,
    choice: CameraChoice,
    position: [f32; 3],
    look_at: [f32; 3],
) -> Result<Value, String> {
    let eye = Vec3::from_array(position);
    let target = Vec3::from_array(look_at);
    if !eye.is_finite()
        || !target.is_finite()
        || (target - eye).length_squared() < 1e-8
        || eye.abs().max_element() > 1e6
        || target.abs().max_element() > 1e6
    {
        return Err("invalid camera position/lookAt".into());
    }
    let up = if (target - eye).normalize().dot(Vec3::Y).abs() > 0.999 {
        Vec3::Z
    } else {
        Vec3::Y
    };
    let pose = Transform::from_translation(eye).looking_at(target, up);
    let mut query = world.query_filtered::<(
        &mut Transform,
        Option<&DesktopCamera>,
        Option<&SpectatorCamera>,
    ), Or<(With<DesktopCamera>, With<SpectatorCamera>)>>();
    let mut found = false;
    for (mut transform, desktop, spectator) in query.iter_mut(world) {
        if (choice == CameraChoice::Desktop && desktop.is_some())
            || (choice == CameraChoice::Spectator && spectator.is_some())
        {
            *transform = pose;
            found = true;
            break;
        }
    }
    if !found {
        return Err("camera unavailable".into());
    }
    if choice == CameraChoice::Desktop {
        if let Some(mut pitch) =
            world.get_resource_mut::<crate::desktop_locomotion::DesktopCameraPitch>()
        {
            pitch.0 = pose.rotation.to_euler(EulerRot::YXZ).1;
        }
    }
    Ok(json!({"position":position,"lookAt":look_at}))
}

pub struct AgentPlugin;
impl Plugin for AgentPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<AgentControl>()
            .add_plugins(crate::agent_capture::AgentCapturePlugin)
            .add_systems(Update, (agent_system, settings_status));
    }
}

fn settings_status(
    control: Res<AgentControl>,
    config: Res<crate::RootConfig>,
    decisions: Res<crate::permissions::PermissionDecisionStore>,
    root_url: Res<crate::types::CurrentUrl>,
    time: Res<Time>,
    mut last: Local<f64>,
    dom: Res<crate::ElemenetWorld>,
    mut manager: NonSendMut<crate::js::ScriptRuntimeManager>,
) {
    use specs::WorldExt;
    if time.elapsed_seconds_f64() - *last < 0.5 {
        return;
    }
    *last = time.elapsed_seconds_f64();
    // Un solo JSON con todo lo que el documento de ajustes necesita saber. Se
    // arma una vez por barrido, no una por worker: no depende de a quién va.
    //
    // Antes esto se mandaba como JavaScript inyectado que le escribía atributos
    // a nodos de ids fijos. Andaba, y era un rodeo: el id quedaba acordado a
    // mano entre este archivo y el documento, y si alguien lo borraba la página
    // no se enteraba nunca, sin un solo error. Ahora va por el buzón tipado,
    // que es el mismo camino que ya usan fetch, las capturas y el hover.
    let publicacion = crate::settings::publish(
        control.connection_label(),
        config.mcp_auto_start,
        &config,
        &decisions,
        &root_url.0,
    );
    for (&id, worker) in &mut manager.contexts {
        if !is_settings_document(&crate::dom::find_node_base_url(
            &dom.0,
            dom.0.entities().entity(id),
            "",
        )) {
            continue;
        }
        if worker
            .try_send(crate::js::JsWorkerCommand::PushSettings(publicacion.clone()))
            .is_ok()
        {
            worker.needs_tick = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn settings_scope() {
        assert!(is_settings_document("luna://settings"));
        for url in [
            "https://settings",
            "luna://settings.evil",
            "luna://settings/evil",
            "luna://user@settings",
        ] {
            assert!(!is_settings_document(url));
        }
    }
    #[test]
    fn navigation_ack_and_camera_isolation() {
        let mut world = World::new();
        world.init_resource::<NextTabId>();
        world.init_resource::<SpaceMountQueue>();
        assert_eq!(open(&mut world, "luna://home".into()).unwrap()["tabId"], 1);
        assert!(open(&mut world, "file:///secret".into()).is_err());
        let desktop = world.spawn((DesktopCamera, Transform::default())).id();
        world.spawn((SpectatorCamera, Transform::default()));
        move_camera(
            &mut world,
            CameraChoice::Spectator,
            [1., 2., 3.],
            [0., 0., 0.],
        )
        .unwrap();
        assert_eq!(
            world.get::<Transform>(desktop).unwrap().translation,
            Vec3::ZERO
        );
        assert!(move_camera(&mut world, CameraChoice::Desktop, [0.; 3], [0.; 3]).is_err());
    }
}
