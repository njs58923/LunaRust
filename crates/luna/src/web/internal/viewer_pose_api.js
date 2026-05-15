// luna://internal/viewer_pose_api.js
// Expone `dimention.readViewerPose()` — devuelve la pose actual del usuario
// (HMD en VR, cámara en desktop). Auto-inyectado vía bundle `read_hmd_pose`.
//
// Retorno: null si no hay snapshot disponible o el space no tiene la cap.
// Si hay snapshot:
//   {
//     mode: "vr" | "desktop",
//     px, py, pz,                        // posición world space
//     forwardX, forwardY, forwardZ,      // vector forward (normalizado)
//     yaw, pitch,                        // euler Y-X (rad)
//     qx, qy, qz, qw,                    // quaternion completo
//     aspect, fovY                       // sólo desktop (en VR = 0)
//   }
//
// El snapshot lo actualiza el host una vez por frame; readViewerPose() es
// O(1) y no bloquea. UX shells suelen leerlo al momento de mostrarse para
// posicionarse al frente del usuario sin perseguir.

(function (global) {
  const core = Deno.core;
  const ops = core.ops;
  if (!ops.op_read_viewer_pose) {
    console.error('[viewer_pose_api] op_read_viewer_pose missing — runtime sin la op');
    return;
  }

  function readViewerPose() {
    return ops.op_read_viewer_pose();
  }

  function attach(target) {
    if (target && !target.readViewerPose) {
      target.readViewerPose = readViewerPose;
    }
  }
  attach(global.hiperspace && global.hiperspace.dimention);
  attach(global.dimention);

  console.log('[viewer_pose_api] dimention.readViewerPose ready');
})(globalThis);
