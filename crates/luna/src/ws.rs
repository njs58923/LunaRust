//! Bridge WebSocket entre el JS runtime y la red.
//!
//! `js_runtime` provee el lado JS (clase `WebSocket` + ops + colas), pero no abre
//! sockets reales. Este módulo es el transporte: drena `ws_connect_queue` /
//! `ws_send_queue` / `ws_close_queue` del engine (vía `js_tick_system`), abre una
//! task tokio por conexión con `tokio-tungstenite`, y devuelve estado + mensajes
//! al worker JF del space mediante `JsWorkerCommand::PushWsEvents`.
//!
//! Espejo del patrón `fetch` de `io.rs`. Los `conn_id` los asigna `op_ws_connect`
//! por-engine, así que sólo son únicos dentro de un space → la clave es
//! `(space_id, conn_id)`.

use std::{
    collections::HashMap,
    sync::{mpsc, Mutex},
};

use bevy::prelude::*;
use futures_util::{SinkExt, StreamExt};
use tokio::runtime::Runtime;
use tokio::sync::mpsc as tokio_mpsc;
use tokio_tungstenite::tungstenite::Message;

use crate::js::{JsWorkerCommand, ScriptRuntimeManager, WsWorkerEvent};

/// Evento entrante de una conexión WS, encaminado al worker JS del space.
pub enum WsInbound {
    Open,
    Message(String),
    Error(String),
    Closed,
}

/// Comando saliente hacia la task de una conexión concreta.
enum WsOutbound {
    Send(String),
    Close,
}

#[derive(Resource)]
pub struct WsService {
    result_tx: mpsc::Sender<(u32, i32, WsInbound)>,
    result_rx: Mutex<mpsc::Receiver<(u32, i32, WsInbound)>>,
    /// (space_id, conn_id) -> canal hacia la task de esa conexión.
    conns: Mutex<HashMap<(u32, i32), tokio_mpsc::UnboundedSender<WsOutbound>>>,
}

impl Default for WsService {
    fn default() -> Self {
        let (result_tx, result_rx) = mpsc::channel();
        Self {
            result_tx,
            result_rx: Mutex::new(result_rx),
            conns: Mutex::new(HashMap::new()),
        }
    }
}

impl WsService {
    fn remove_conn(&self, space_id: u32, conn_id: i32) {
        if let Ok(mut conns) = self.conns.lock() {
            conns.remove(&(space_id, conn_id));
        }
    }
}

/// Abre una conexión WS y lanza la task que la bombea.
pub fn request_ws_connect(
    rt: &Runtime,
    ws_service: &WsService,
    space_id: u32,
    conn_id: i32,
    url: String,
) {
    let (out_tx, mut out_rx) = tokio_mpsc::unbounded_channel::<WsOutbound>();
    {
        let Ok(mut conns) = ws_service.conns.lock() else {
            return;
        };
        conns.insert((space_id, conn_id), out_tx);
    }
    let tx = ws_service.result_tx.clone();

    rt.spawn(async move {
        let stream = match tokio_tungstenite::connect_async(url.as_str()).await {
            Ok((stream, _resp)) => stream,
            Err(e) => {
                let _ = tx.send((space_id, conn_id, WsInbound::Error(e.to_string())));
                return;
            }
        };

        let _ = tx.send((space_id, conn_id, WsInbound::Open));
        let (mut write, mut read) = stream.split();

        loop {
            tokio::select! {
                incoming = read.next() => match incoming {
                    Some(Ok(Message::Text(t))) => {
                        let _ = tx.send((space_id, conn_id, WsInbound::Message(t)));
                    }
                    Some(Ok(Message::Binary(b))) => {
                        let _ = tx.send((
                            space_id,
                            conn_id,
                            WsInbound::Message(String::from_utf8_lossy(&b).into_owned()),
                        ));
                    }
                    Some(Ok(Message::Close(_))) | None => {
                        let _ = tx.send((space_id, conn_id, WsInbound::Closed));
                        break;
                    }
                    // Ping/Pong/Frame: tungstenite los gestiona, ignoramos.
                    Some(Ok(_)) => {}
                    Some(Err(e)) => {
                        let _ = tx.send((space_id, conn_id, WsInbound::Error(e.to_string())));
                        break;
                    }
                },
                outgoing = out_rx.recv() => match outgoing {
                    Some(WsOutbound::Send(s)) => {
                        if let Err(e) = write.send(Message::Text(s)).await {
                            let _ = tx.send((space_id, conn_id, WsInbound::Error(e.to_string())));
                            break;
                        }
                    }
                    Some(WsOutbound::Close) | None => {
                        let _ = write.close().await;
                        let _ = tx.send((space_id, conn_id, WsInbound::Closed));
                        break;
                    }
                },
            }
        }
    });
}

/// Encola un mensaje saliente en la conexión indicada.
pub fn ws_send(ws_service: &WsService, space_id: u32, conn_id: i32, message: String) {
    if let Ok(conns) = ws_service.conns.lock() {
        if let Some(tx) = conns.get(&(space_id, conn_id)) {
            let _ = tx.send(WsOutbound::Send(message));
        }
    }
}

/// Solicita el cierre de la conexión indicada.
pub fn ws_close(ws_service: &WsService, space_id: u32, conn_id: i32) {
    if let Ok(conns) = ws_service.conns.lock() {
        if let Some(tx) = conns.get(&(space_id, conn_id)) {
            let _ = tx.send(WsOutbound::Close);
        }
    }
}

/// Drena eventos entrantes de todas las conexiones WS y los encamina al worker JS
/// del space correspondiente. Espejo de `poll_io_results_system`.
pub fn poll_ws_results_system(
    ws_service: Res<WsService>,
    mut manager: NonSendMut<ScriptRuntimeManager>,
) {
    let drained: Vec<(u32, i32, WsInbound)> = {
        let Ok(rx) = ws_service.result_rx.lock() else {
            return;
        };
        let mut out = Vec::new();
        while let Ok(item) = rx.try_recv() {
            out.push(item);
        }
        out
    };
    if drained.is_empty() {
        return;
    }

    let mut batched: HashMap<u32, Vec<WsWorkerEvent>> = HashMap::new();
    let mut to_remove: Vec<(u32, i32)> = Vec::new();

    for (space_id, conn_id, inbound) in drained {
        let evt = match inbound {
            WsInbound::Open => WsWorkerEvent::Status {
                conn_id,
                status: "open".to_string(),
            },
            WsInbound::Message(m) => WsWorkerEvent::Message { conn_id, data: m },
            WsInbound::Error(e) => {
                to_remove.push((space_id, conn_id));
                WsWorkerEvent::Status {
                    conn_id,
                    status: format!("error: {e}"),
                }
            }
            WsInbound::Closed => {
                to_remove.push((space_id, conn_id));
                WsWorkerEvent::Status {
                    conn_id,
                    status: "closed".to_string(),
                }
            }
        };
        batched.entry(space_id).or_default().push(evt);
    }

    for (space_id, events) in batched {
        if let Some(worker) = manager.contexts.get_mut(&space_id) {
            let send_result = worker.cmd_tx.send(JsWorkerCommand::PushWsEvents(events));
            if send_result.is_ok() {
                worker.needs_tick = true;
            }
        }
    }
    for (space_id, conn_id) in to_remove {
        ws_service.remove_conn(space_id, conn_id);
    }
}
