//! Snapshot global de la pose del usuario (HMD en VR, cámara en desktop).
//!
//! Un sistema per-mode actualiza `ViewerPoseGlobalSnapshot` cada frame; otro
//! propaga el snapshot a workers JS que tengan `READ_HMD_POSE`. Los workers
//! sin la cap reciben `None` (el op JS `op_read_viewer_pose` devuelve null).
//!
//! La API JS `dimention.readViewerPose()` se auto-inyecta vía el bundle
//! `read_hmd_pose` (ver permissions.rs).

use bevy::prelude::*;

use crate::js::ScriptRuntimeManager;
use crate::permissions::{space_has_capability, CapabilityBits, SpacePolicies};
use crate::{DesktopCamera, RenderMode};

#[derive(Clone, Debug)]
pub struct ViewerPose {
    pub mode: ViewerMode,
    pub px: f32,
    pub py: f32,
    pub pz: f32,
    pub forward_x: f32,
    pub forward_y: f32,
    pub forward_z: f32,
    pub yaw: f32,
    pub pitch: f32,
    pub qx: f32,
    pub qy: f32,
    pub qz: f32,
    pub qw: f32,
    // Desktop-only extras (en VR quedan en 0).
    pub aspect: f32,
    pub fov_y_rad: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewerMode {
    Vr,
    Desktop,
}

impl ViewerMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            ViewerMode::Vr => "vr",
            ViewerMode::Desktop => "desktop",
        }
    }
}

/// Latest known viewer pose. None si todavía no hay info (ej. session VR
/// recién abriendo, o no se setearon cámaras desktop).
#[derive(Resource, Default)]
pub struct ViewerPoseGlobalSnapshot(pub Option<ViewerPose>);

/// System desktop: lee transform de DesktopCamera y proyecta yaw/pitch.
/// Corre cuando RenderMode no es VR.
pub fn update_desktop_viewer_pose(
    rm: Res<RenderMode>,
    cam: Query<(&Transform, &Projection), With<DesktopCamera>>,
    mut snapshot: ResMut<ViewerPoseGlobalSnapshot>,
) {
    if rm.is_vr {
        return;
    }
    let Ok((tf, projection)) = cam.get_single() else {
        return;
    };

    // Forward = -Z rotado por la rotación de la cámara.
    let forward = tf.rotation * Vec3::NEG_Z;
    let (yaw, pitch, _roll) = tf.rotation.to_euler(EulerRot::YXZ);

    let (aspect, fov_y) = match projection {
        Projection::Perspective(p) => (p.aspect_ratio, p.fov),
        _ => (1.0, 0.0),
    };

    snapshot.0 = Some(ViewerPose {
        mode: ViewerMode::Desktop,
        px: tf.translation.x,
        py: tf.translation.y,
        pz: tf.translation.z,
        forward_x: forward.x,
        forward_y: forward.y,
        forward_z: forward.z,
        yaw,
        pitch,
        qx: tf.rotation.x,
        qy: tf.rotation.y,
        qz: tf.rotation.z,
        qw: tf.rotation.w,
        aspect,
        fov_y_rad: fov_y,
    });
}

/// Propaga el snapshot global a cada worker JS, recortado según sus caps:
///
///   READ_HMD_POSE     → pose completa (posición + hacia dónde mira).
///   READ_CAMERA_POSE  → sólo posición; la orientación va en cero.
///   ninguna           → None (y limpia lo que hubiera, si la cap fue revocada).
///
/// El escalón del medio existe porque la posición ya se filtra igual con
/// `read_pose_stream`: un `<posezone>` entrega la pose de los mandos, que
/// ubican al jugador con medio metro de error. Lo que de verdad protege
/// READ_HMD_POSE es la mirada.
pub fn propagate_viewer_pose_to_workers(
    snapshot: Res<ViewerPoseGlobalSnapshot>,
    mut manager: NonSendMut<ScriptRuntimeManager>,
    space_policies: Res<SpacePolicies>,
) {
    for (space_id, worker) in manager.contexts.iter_mut() {
        let completa = space_has_capability(
            *space_id,
            CapabilityBits::READ_HMD_POSE,
            &space_policies,
        );
        let solo_posicion = !completa
            && space_has_capability(
                *space_id,
                CapabilityBits::READ_CAMERA_POSE,
                &space_policies,
            );
        let data = if completa || solo_posicion {
            snapshot.0.as_ref().map(|p| js_runtime::ViewerPoseData {
                mode: p.mode.as_str().to_string(),
                px: p.px,
                py: p.py,
                pz: p.pz,
                // Sin READ_HMD_POSE la orientación no se entrega. Va en cero y
                // no con el valor real "por si acaso": un forward en cero es
                // detectable desde JS, un valor stale no.
                forward_x: if completa { p.forward_x } else { 0.0 },
                forward_y: if completa { p.forward_y } else { 0.0 },
                forward_z: if completa { p.forward_z } else { 0.0 },
                yaw: if completa { p.yaw } else { 0.0 },
                pitch: if completa { p.pitch } else { 0.0 },
                qx: if completa { p.qx } else { 0.0 },
                qy: if completa { p.qy } else { 0.0 },
                qz: if completa { p.qz } else { 0.0 },
                qw: if completa { p.qw } else { 1.0 },
                aspect: if completa { p.aspect } else { 0.0 },
                fov_y_rad: if completa { p.fov_y_rad } else { 0.0 },
            })
        } else {
            None
        };
        // Envío vía canal mpsc al thread del worker (thread-safe). El worker
        // procesa SetViewerPose y llama engine.set_viewer_pose; las ops JS
        // (que corren en ese mismo thread) leen el Shared sin race.
        // Si el worker está saturado, el valor se coalesce naturalmente: el
        // siguiente frame vuelve a enviar la pose más reciente.
        let _ = worker.try_send(crate::js::JsWorkerCommand::SetViewerPose(data));
    }
}
