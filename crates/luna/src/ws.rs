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
    collections::{HashMap, VecDeque},
    sync::Mutex,
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

const WS_INBOUND_CAPACITY: usize = 512;
const WS_OUTBOUND_CAPACITY: usize = 128;
const WS_PENDING_PER_SPACE: usize = 256;
const WS_DRAIN_PER_FRAME: usize = 256;

/// Comando saliente hacia la task de una conexión concreta.
enum WsOutbound {
    Send(String),
    Close,
}

struct WsConnection {
    sender: tokio_mpsc::Sender<WsOutbound>,
    abort: tokio::task::AbortHandle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WsSendError {
    MissingConnection,
    QueueFull,
    Disconnected,
}

impl std::fmt::Display for WsSendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingConnection => f.write_str("WebSocket connection does not exist"),
            Self::QueueFull => f.write_str("WebSocket outbound queue is full"),
            Self::Disconnected => f.write_str("WebSocket task is disconnected"),
        }
    }
}

#[derive(Resource)]
pub struct WsService {
    result_tx: tokio_mpsc::Sender<(u32, i32, WsInbound)>,
    result_rx: Mutex<tokio_mpsc::Receiver<(u32, i32, WsInbound)>>,
    /// (space_id, conn_id) -> canal hacia la task de esa conexión.
    conns: Mutex<HashMap<(u32, i32), WsConnection>>,
    pending_to_workers: Mutex<HashMap<u32, VecDeque<WsWorkerEvent>>>,
}

impl Default for WsService {
    fn default() -> Self {
        let (result_tx, result_rx) = tokio_mpsc::channel(WS_INBOUND_CAPACITY);
        Self {
            result_tx,
            result_rx: Mutex::new(result_rx),
            conns: Mutex::new(HashMap::new()),
            pending_to_workers: Mutex::new(HashMap::new()),
        }
    }
}

impl WsService {
    fn remove_conn(&self, space_id: u32, conn_id: i32) {
        if let Ok(mut conns) = self.conns.lock() {
            if let Some(conn) = conns.remove(&(space_id, conn_id)) {
                conn.abort.abort();
            }
        }
    }

    pub fn close_space(&self, space_id: u32) {
        if let Ok(mut conns) = self.conns.lock() {
            let keys = conns
                .keys()
                .copied()
                .filter(|(owner, _)| *owner == space_id)
                .collect::<Vec<_>>();
            for key in keys {
                if let Some(conn) = conns.remove(&key) {
                    conn.abort.abort();
                }
            }
        }
        if let Ok(mut pending) = self.pending_to_workers.lock() {
            pending.remove(&space_id);
        }
    }

    pub fn close_all(&self) {
        if let Ok(mut conns) = self.conns.lock() {
            for (_, conn) in conns.drain() {
                conn.abort.abort();
            }
        }
        if let Ok(mut pending) = self.pending_to_workers.lock() {
            pending.clear();
        }
    }

    fn defer_worker_events(&self, space_id: u32, events: Vec<WsWorkerEvent>) {
        let Ok(mut pending) = self.pending_to_workers.lock() else {
            return;
        };
        let queue = pending.entry(space_id).or_default();
        for event in events {
            if queue.len() == WS_PENDING_PER_SPACE {
                queue.pop_front();
            }
            queue.push_back(event);
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
    let (out_tx, mut out_rx) = tokio_mpsc::channel::<WsOutbound>(WS_OUTBOUND_CAPACITY);
    let tx = ws_service.result_tx.clone();

    let task = rt.spawn(async move {
        let stream = match tokio_tungstenite::connect_async(url.as_str()).await {
            Ok((stream, _resp)) => stream,
            Err(e) => {
                let _ = tx
                    .send((space_id, conn_id, WsInbound::Error(e.to_string())))
                    .await;
                return;
            }
        };

        let _ = tx.send((space_id, conn_id, WsInbound::Open)).await;
        let (mut write, mut read) = stream.split();

        loop {
            tokio::select! {
                incoming = read.next() => match incoming {
                    Some(Ok(Message::Text(t))) => {
                        let _ = tx.send((space_id, conn_id, WsInbound::Message(t))).await;
                    }
                    Some(Ok(Message::Binary(b))) => {
                        let _ = tx.send((
                            space_id,
                            conn_id,
                            WsInbound::Message(String::from_utf8_lossy(&b).into_owned()),
                        )).await;
                    }
                    Some(Ok(Message::Close(_))) | None => {
                        let _ = tx.send((space_id, conn_id, WsInbound::Closed)).await;
                        break;
                    }
                    // Ping/Pong/Frame: tungstenite los gestiona, ignoramos.
                    Some(Ok(_)) => {}
                    Some(Err(e)) => {
                        let _ = tx.send((space_id, conn_id, WsInbound::Error(e.to_string()))).await;
                        break;
                    }
                },
                outgoing = out_rx.recv() => match outgoing {
                    Some(WsOutbound::Send(s)) => {
                        if let Err(e) = write.send(Message::Text(s)).await {
                            let _ = tx.send((space_id, conn_id, WsInbound::Error(e.to_string()))).await;
                            break;
                        }
                    }
                    Some(WsOutbound::Close) | None => {
                        let _ = write.close().await;
                        let _ = tx.send((space_id, conn_id, WsInbound::Closed)).await;
                        break;
                    }
                },
            }
        }
    });
    let connection = WsConnection {
        sender: out_tx,
        abort: task.abort_handle(),
    };
    if let Ok(mut conns) = ws_service.conns.lock() {
        if let Some(previous) = conns.insert((space_id, conn_id), connection) {
            previous.abort.abort();
        }
    } else {
        task.abort();
    }
}

/// Encola un mensaje saliente en la conexión indicada.
pub fn ws_send(
    ws_service: &WsService,
    space_id: u32,
    conn_id: i32,
    message: String,
) -> Result<(), WsSendError> {
    let conns = ws_service
        .conns
        .lock()
        .map_err(|_| WsSendError::Disconnected)?;
    let conn = conns
        .get(&(space_id, conn_id))
        .ok_or(WsSendError::MissingConnection)?;
    conn.sender.try_send(WsOutbound::Send(message)).map_err(|err| match err {
        tokio_mpsc::error::TrySendError::Full(_) => WsSendError::QueueFull,
        tokio_mpsc::error::TrySendError::Closed(_) => WsSendError::Disconnected,
    })
}

/// Solicita el cierre de la conexión indicada.
pub fn ws_close(ws_service: &WsService, space_id: u32, conn_id: i32) {
    if let Ok(mut conns) = ws_service.conns.lock() {
        if let Some(conn) = conns.remove(&(space_id, conn_id)) {
            if conn.sender.try_send(WsOutbound::Close).is_err() {
                conn.abort.abort();
            }
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
        let Ok(mut rx) = ws_service.result_rx.lock() else {
            return;
        };
        let mut out = Vec::new();
        while out.len() < WS_DRAIN_PER_FRAME {
            let Ok(item) = rx.try_recv() else {
                break;
            };
            out.push(item);
        }
        out
    };

    let mut batched: HashMap<u32, Vec<WsWorkerEvent>> = ws_service
        .pending_to_workers
        .lock()
        .map(|mut pending| {
            std::mem::take(&mut *pending)
                .into_iter()
                .map(|(space_id, events)| (space_id, events.into_iter().collect()))
                .collect()
        })
        .unwrap_or_default();
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
            match worker
                .cmd_tx
                .try_send(JsWorkerCommand::PushWsEvents(events))
            {
                Ok(()) => worker.needs_tick = true,
                Err(std::sync::mpsc::TrySendError::Full(
                    JsWorkerCommand::PushWsEvents(events),
                )) => {
                    // No perder mensajes: queda un backlog acotado por space y
                    // se reintenta el próximo frame.
                    ws_service.defer_worker_events(space_id, events);
                }
                Err(_) => {}
            }
        }
    }
    for (space_id, conn_id) in to_remove {
        ws_service.remove_conn(space_id, conn_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outbound_queue_is_bounded_and_space_cleanup_aborts_it() {
        let service = WsService::default();
        let runtime = Runtime::new().unwrap();
        let (sender, _receiver) = tokio_mpsc::channel(WS_OUTBOUND_CAPACITY);
        let task = runtime.spawn(std::future::pending::<()>());
        let abort = task.abort_handle();
        service.conns.lock().unwrap().insert(
            (5, 9),
            WsConnection {
                sender,
                abort: abort.clone(),
            },
        );

        for _ in 0..WS_OUTBOUND_CAPACITY {
            ws_send(&service, 5, 9, "payload".to_string()).unwrap();
        }
        assert_eq!(
            ws_send(&service, 5, 9, "overflow".to_string()),
            Err(WsSendError::QueueFull)
        );

        service.close_space(5);
        assert!(!service.conns.lock().unwrap().contains_key(&(5, 9)));
        let join_error = runtime
            .block_on(task)
            .expect_err("space cleanup should abort the WebSocket task");
        assert!(join_error.is_cancelled());
    }

    #[test]
    fn pending_worker_events_are_bounded_per_space() {
        let service = WsService::default();
        for index in 0..(WS_PENDING_PER_SPACE + 12) {
            service.defer_worker_events(
                3,
                vec![WsWorkerEvent::Message {
                    conn_id: 1,
                    data: index.to_string(),
                }],
            );
        }

        let pending = service.pending_to_workers.lock().unwrap();
        let queue = pending.get(&3).unwrap();
        assert_eq!(queue.len(), WS_PENDING_PER_SPACE);
        assert!(matches!(
            queue.front(),
            Some(WsWorkerEvent::Message { data, .. }) if data == "12"
        ));
    }
}
