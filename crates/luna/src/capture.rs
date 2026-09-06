//! Captura del frame renderizado a un PNG en disco.
//!
//! El pedido nace en JS (`dimention.captureFrame(nombre)`), pasa por la cola
//! del worker y `js.rs` lo valida contra la cap `CAPTURE_FRAME` del space antes
//! de dejarlo acá. Este módulo sólo se ocupa de pedirle el frame a Bevy,
//! escribir el archivo y devolver el resultado al worker que lo pidió.
//!
//! La captura de Bevy es asíncrona: el callback corre cuando la GPU devolvió
//! el buffer, uno o dos frames después. Por eso el resultado viaja por un
//! buzón compartido en vez de resolverse en el acto — así quien espera la
//! promesa recibe la ruta con el archivo ya cerrado, sin adivinar tiempos.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use bevy::render::texture::Image;
use bevy::render::view::screenshot::ScreenshotManager;
use bevy::window::PrimaryWindow;

use crate::js::{JsWorkerCommand, ScriptRuntimeManager};
use crate::LogPanel;

/// Un pedido validado, listo para pedirle el frame a Bevy.
#[derive(Debug, Clone)]
pub struct CaptureRequest {
    pub space_id: u32,
    pub request_id: i32,
    pub name: String,
}

/// Cola de pedidos que `js.rs` llena tras validar la capacidad.
#[derive(Resource, Default)]
pub struct PendingFrameCaptures(pub Vec<CaptureRequest>);

/// Buzón que el callback de Bevy usa para devolver el resultado al hilo del
/// ECS. `Arc<Mutex<_>>` y no un canal porque un `Receiver` no es `Sync` y los
/// recursos de Bevy tienen que serlo.
#[derive(Resource, Clone, Default)]
pub struct CaptureOutbox(pub Arc<Mutex<Vec<(u32, i32, Result<String, String>)>>>);

/// Directorio donde quedan los PNG. Fuera del repo a propósito: son efímeros.
pub fn capture_dir() -> PathBuf {
    std::env::temp_dir().join("luna-captures")
}

/// `name` viene de JS: se queda sólo con el nombre de archivo y fuerza .png,
/// así un space no puede escribir donde se le antoje.
fn sanitize(name: &str) -> String {
    let base = name
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or("capture")
        .trim()
        .trim_matches('.');
    let limpio: String = base
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        .take(80)
        .collect();
    let limpio = if limpio.is_empty() {
        "capture".to_string()
    } else {
        limpio
    };
    if limpio.to_ascii_lowercase().ends_with(".png") {
        limpio
    } else {
        format!("{limpio}.png")
    }
}

fn guardar_png(imagen: Image, ruta: &Path) -> Result<String, String> {
    let dinamica = imagen
        .try_into_dynamic()
        .map_err(|err| format!("frame no convertible a imagen: {err:?}"))?;
    dinamica
        .save(ruta)
        .map_err(|err| format!("no se pudo escribir {}: {err}", ruta.display()))?;
    Ok(ruta.display().to_string())
}

/// Le pide a Bevy cada frame pendiente. El callback escribe el PNG y deja el
/// resultado en el buzón.
pub fn request_frame_captures_system(
    mut pendientes: ResMut<PendingFrameCaptures>,
    mut screenshots: ResMut<ScreenshotManager>,
    ventanas: Query<Entity, With<PrimaryWindow>>,
    buzon: Res<CaptureOutbox>,
    mut log_panel: ResMut<LogPanel>,
) {
    if pendientes.0.is_empty() {
        return;
    }

    let Ok(ventana) = ventanas.get_single() else {
        // Sin ventana no hay swapchain que capturar (headless, o cerrándose).
        for req in pendientes.0.drain(..) {
            if let Ok(mut cola) = buzon.0.lock() {
                cola.push((
                    req.space_id,
                    req.request_id,
                    Err("no hay ventana primaria para capturar".to_string()),
                ));
            }
        }
        return;
    };

    let dir = capture_dir();
    if let Err(err) = std::fs::create_dir_all(&dir) {
        for req in pendientes.0.drain(..) {
            if let Ok(mut cola) = buzon.0.lock() {
                cola.push((
                    req.space_id,
                    req.request_id,
                    Err(format!("no se pudo crear {}: {err}", dir.display())),
                ));
            }
        }
        return;
    }

    for req in pendientes.0.drain(..) {
        let ruta = dir.join(sanitize(&req.name));
        let buzon_cb = buzon.0.clone();
        let space_id = req.space_id;
        let request_id = req.request_id;
        let ruta_cb = ruta.clone();

        let pedido = screenshots.take_screenshot(ventana, move |imagen| {
            let resultado = guardar_png(imagen, &ruta_cb);
            if let Ok(mut cola) = buzon_cb.lock() {
                cola.push((space_id, request_id, resultado));
            }
        });

        match pedido {
            Ok(()) => log_panel.push_info(format!(
                "[capture][space:{space_id}] frame pedido -> {}",
                ruta.display()
            )),
            Err(err) => {
                if let Ok(mut cola) = buzon.0.lock() {
                    cola.push((space_id, request_id, Err(format!("{err:?}"))));
                }
            }
        }
    }
}

/// Devuelve los resultados al worker que pidió la captura.
pub fn deliver_frame_captures_system(
    buzon: Res<CaptureOutbox>,
    mut manager: NonSendMut<ScriptRuntimeManager>,
    mut log_panel: ResMut<LogPanel>,
) {
    let listos: Vec<(u32, i32, Result<String, String>)> = match buzon.0.lock() {
        Ok(mut cola) if !cola.is_empty() => cola.drain(..).collect(),
        _ => return,
    };

    for (space_id, request_id, resultado) in listos {
        if let Err(err) = &resultado {
            log_panel.push_warn(format!("[capture][space:{space_id}] falló: {err}"));
        }
        let Some(worker) = manager.contexts.get_mut(&space_id) else {
            continue; // el space se fue mientras la GPU trabajaba
        };
        if worker
            .try_send(JsWorkerCommand::PushCaptureResults(vec![(
                request_id, resultado,
            )]))
            .is_ok()
        {
            worker.needs_tick = true;
        }
    }
}

pub struct CapturePlugin;

impl Plugin for CapturePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PendingFrameCaptures>()
            .init_resource::<CaptureOutbox>()
            .add_systems(
                Update,
                (request_frame_captures_system, deliver_frame_captures_system).chain(),
            );
    }
}
