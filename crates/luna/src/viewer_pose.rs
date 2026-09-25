//! Snapshot global de la pose del usuario (HMD en VR, cámara en desktop).
//!
//! Un sistema per-mode actualiza `ViewerPoseGlobalSnapshot` cada frame; otro
//! propaga el snapshot a workers JS que tengan `READ_HMD_POSE`. Los workers
//! sin la cap reciben `None` (el op JS `op_read_viewer_pose` devuelve null).
//!
//! La API JS `dimention.readViewerPose()` se auto-inyecta vía el bundle
//! `read_hmd_pose` (ver permissions.rs).

use bevy::prelude::*;
use std::{
    collections::HashMap,
    sync::{Arc, Weak},
};

use crate::js::ScriptRuntimeManager;
use crate::permissions::{space_has_capability, CapabilityBits, SpacePolicies};
use crate::{DesktopCamera, RenderMode};

#[derive(Clone, Debug, PartialEq)]
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

    let pose = Some(ViewerPose {
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
    if snapshot.0 != pose {
        snapshot.0 = pose;
    }
}

struct Recipient {
    port: Weak<js_runtime::components::ComponentPort>,
    enabled: bool,
    full: bool,
    last: Option<ViewerPose>,
}

/// Only subscribed workers participate in the per-frame bridge. Keeping a weak
/// identity distinguishes a restarted isolate even when its space ID is reused.
#[derive(Default)]
pub struct ViewerPoseBridge {
    generation: Option<(u64, u64)>,
    recipients: HashMap<u32, Recipient>,
}

impl ViewerPose {
    fn position_only(&self) -> Self {
        Self {
            mode: self.mode,
            px: self.px,
            py: self.py,
            pz: self.pz,
            forward_x: 0.0,
            forward_y: 0.0,
            forward_z: 0.0,
            yaw: 0.0,
            pitch: 0.0,
            qx: 0.0,
            qy: 0.0,
            qz: 0.0,
            qw: 1.0,
            aspect: 0.0,
            fov_y_rad: 0.0,
        }
    }

    fn to_worker(&self) -> js_runtime::ViewerPoseData {
        js_runtime::ViewerPoseData {
            mode: self.mode.as_str().to_owned(),
            px: self.px,
            py: self.py,
            pz: self.pz,
            forward_x: self.forward_x,
            forward_y: self.forward_y,
            forward_z: self.forward_z,
            yaw: self.yaw,
            pitch: self.pitch,
            qx: self.qx,
            qy: self.qy,
            qz: self.qz,
            qw: self.qw,
            aspect: self.aspect,
            fov_y_rad: self.fov_y_rad,
        }
    }
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
    mut bridge: Local<ViewerPoseBridge>,
) {
    let generation = (space_policies.generation, manager.context_generation);
    if bridge.generation != Some(generation) {
        bridge.recipients.retain(|id, recipient| {
            manager.contexts.get(id).is_some_and(|worker| {
                recipient
                    .port
                    .ptr_eq(&Arc::downgrade(&worker.component_port))
            })
        });
        for (&id, worker) in &manager.contexts {
            let full = space_has_capability(id, CapabilityBits::READ_HMD_POSE, &space_policies);
            let enabled =
                full || space_has_capability(id, CapabilityBits::READ_CAMERA_POSE, &space_policies);
            // A new isolate already contains None. Sending None every frame
            // needlessly wakes every static facade in a large street.
            if enabled || bridge.recipients.contains_key(&id) {
                let recipient = bridge.recipients.entry(id).or_insert_with(|| Recipient {
                    port: Arc::downgrade(&worker.component_port),
                    enabled,
                    full,
                    last: None,
                });
                recipient.enabled = enabled;
                recipient.full = full;
            }
        }
        bridge.generation = Some(generation);
    }
    if bridge.recipients.is_empty() {
        return;
    }
    let position = snapshot.0.as_ref().map(ViewerPose::position_only);
    bridge.recipients.retain(|id, recipient| {
        let next = if !recipient.enabled {
            &None
        } else if recipient.full {
            &snapshot.0
        } else {
            &position
        };
        if &recipient.last != next {
            let Some(worker) = manager.contexts.get_mut(id) else {
                return false;
            };
            let command =
                crate::js::JsWorkerCommand::SetViewerPose(next.as_ref().map(ViewerPose::to_worker));
            // Failed sends must retry, including a revocation while saturated.
            if worker.try_send(command).is_ok() {
                recipient.last.clone_from(next);
            }
        }
        recipient.enabled || recipient.last.is_some()
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        js::{fake_worker, JsWorkerCommand},
        permissions::SpacePolicy,
    };
    use std::sync::mpsc::Receiver;

    fn pose() -> ViewerPose {
        ViewerPose {
            mode: ViewerMode::Desktop,
            px: 1.0,
            py: 1.6,
            pz: -3.0,
            forward_x: 0.0,
            forward_y: 0.0,
            forward_z: -1.0,
            yaw: 0.0,
            pitch: 0.0,
            qx: 0.0,
            qy: 0.0,
            qz: 0.0,
            qw: 1.0,
            aspect: 1.7,
            fov_y_rad: 1.0,
        }
    }

    fn fixture(count: u32) -> (App, Vec<Receiver<JsWorkerCommand>>) {
        let mut app = App::new();
        let mut manager = ScriptRuntimeManager::default();
        let mut receivers = Vec::new();
        for id in 0..count {
            let (worker, receiver, _) = fake_worker(false);
            manager.contexts.insert(id, worker);
            receivers.push(receiver);
        }
        app.insert_non_send_resource(manager)
            .insert_resource(SpacePolicies::default())
            .insert_resource(ViewerPoseGlobalSnapshot(Some(pose())))
            .add_systems(Update, propagate_viewer_pose_to_workers);
        (app, receivers)
    }

    fn grant(app: &mut App, id: u32, caps: CapabilityBits) {
        let mut policies = app.world_mut().resource_mut::<SpacePolicies>();
        policies.by_space.insert(
            id,
            SpacePolicy {
                effective_caps: caps,
                ..default()
            },
        );
        policies.generation += 1;
    }

    fn received(rx: &Receiver<JsWorkerCommand>) -> Option<js_runtime::ViewerPoseData> {
        match rx.try_recv().expect("pose update") {
            JsWorkerCommand::SetViewerPose(pose) => pose,
            _ => panic!("unexpected command"),
        }
    }

    #[test]
    fn static_workers_stay_asleep_and_pose_subscriptions_only_receive_changes() {
        let (mut app, rx) = fixture(130);
        grant(&mut app, 128, CapabilityBits::READ_HMD_POSE);
        grant(&mut app, 129, CapabilityBits::READ_CAMERA_POSE);
        app.update();
        assert_eq!(received(&rx[128]).unwrap().forward_z, -1.0);
        let position = received(&rx[129]).unwrap();
        assert_eq!(
            (position.px, position.forward_z, position.fov_y_rad),
            (1.0, 0.0, 0.0)
        );
        for _ in 0..100 {
            app.update();
        }
        assert!(rx.iter().all(|rx| rx.try_recv().is_err()));

        app.world_mut()
            .resource_mut::<ViewerPoseGlobalSnapshot>()
            .0
            .as_mut()
            .unwrap()
            .yaw = 0.5;
        app.update();
        assert_eq!(received(&rx[128]).unwrap().yaw, 0.5);
        assert!(
            rx[129].try_recv().is_err(),
            "orientation is irrelevant to a position subscriber"
        );
        app.world_mut()
            .resource_mut::<ViewerPoseGlobalSnapshot>()
            .0
            .as_mut()
            .unwrap()
            .px = 2.0;
        app.update();
        assert_eq!(received(&rx[128]).unwrap().px, 2.0);
        assert_eq!(received(&rx[129]).unwrap().px, 2.0);
        assert!(rx[..128].iter().all(|rx| rx.try_recv().is_err()));
    }

    #[test]
    fn downgrade_revocation_and_restarted_worker_refresh_the_snapshot() {
        let (mut app, rx) = fixture(1);
        grant(&mut app, 0, CapabilityBits::READ_HMD_POSE);
        app.update();
        assert_eq!(received(&rx[0]).unwrap().forward_z, -1.0);
        grant(&mut app, 0, CapabilityBits::READ_CAMERA_POSE);
        app.update();
        assert_eq!(received(&rx[0]).unwrap().forward_z, 0.0);
        grant(&mut app, 0, CapabilityBits::empty());
        app.update();
        assert!(received(&rx[0]).is_none());
        app.update();
        assert!(rx[0].try_recv().is_err());

        grant(&mut app, 0, CapabilityBits::READ_HMD_POSE);
        app.update();
        assert!(received(&rx[0]).is_some());
        let (worker, replacement_rx, _) = fake_worker(false);
        {
            let mut manager = app
                .world_mut()
                .non_send_resource_mut::<ScriptRuntimeManager>();
            manager.contexts.insert(0, worker);
            manager.context_generation += 1;
        }
        app.update();
        assert!(
            received(&replacement_rx).is_some(),
            "a replacement must receive the unchanged pose"
        );
        app.world_mut().resource_mut::<ViewerPoseGlobalSnapshot>().0 = None;
        app.update();
        assert!(received(&replacement_rx).is_none());
    }

    #[test]
    fn saturated_worker_retries_revocation_until_it_is_queued() {
        let (mut app, rx) = fixture(1);
        grant(&mut app, 0, CapabilityBits::READ_HMD_POSE);
        app.update();
        assert!(received(&rx[0]).is_some());
        {
            let mut manager = app
                .world_mut()
                .non_send_resource_mut::<ScriptRuntimeManager>();
            let worker = manager.contexts.get_mut(&0).unwrap();
            while worker
                .try_send(JsWorkerCommand::PushShellMessages(Vec::new()))
                .is_ok()
            {}
        }
        grant(&mut app, 0, CapabilityBits::empty());
        app.update(); // Both queue and backlog are full: revocation must retry.
        let mut cleared = 0;
        for _ in 0..5 {
            for command in rx[0].try_iter() {
                if matches!(command, JsWorkerCommand::SetViewerPose(None)) {
                    cleared += 1;
                }
            }
            app.update();
            // In the real app js_tick_system also flushes accepted backlog.
            let mut manager = app
                .world_mut()
                .non_send_resource_mut::<ScriptRuntimeManager>();
            let _ = manager
                .contexts
                .get_mut(&0)
                .unwrap()
                .try_send(JsWorkerCommand::RequestDebugState);
        }
        assert_eq!(cleared, 1);
    }

    #[test]
    #[ignore = "manual CPU bridge benchmark; fake channels, no V8, GPU or FPS claim"]
    fn benchmark_static_viewer_pose_bridge() {
        fn broadcast_none(mut manager: NonSendMut<ScriptRuntimeManager>) {
            for worker in manager.contexts.values_mut() {
                let _ = worker.try_send(JsWorkerCommand::SetViewerPose(None));
            }
        }
        for count in [100, 1_000, 5_000] {
            let (mut optimized, optimized_rx) = fixture(count);
            let (mut old, old_rx) = fixture(count);
            // No grants: this reproduces the useless baseline broadcasts to facades.
            old.add_systems(Update, broadcast_none);
            let mut before = Vec::new();
            let mut after = Vec::new();
            for frame in 0..220 {
                for baseline in if frame % 2 == 0 {
                    [true, false]
                } else {
                    [false, true]
                } {
                    let (app, rx, samples) = if baseline {
                        (&mut old, &old_rx, &mut before)
                    } else {
                        (&mut optimized, &optimized_rx, &mut after)
                    };
                    let start = std::time::Instant::now();
                    app.update();
                    if frame >= 20 {
                        samples.push(start.elapsed().as_secs_f64() * 1000.0);
                    }
                    for rx in rx {
                        for command in rx.try_iter() {
                            std::hint::black_box(command);
                        }
                    }
                }
            }
            before.sort_by(f64::total_cmp);
            after.sort_by(f64::total_cmp);
            println!("static_workers={count} broadcast_median_ms={:.6} subscribed_median_ms={:.6} messages_per_idle_frame={count}->0", before[100], after[100]);
            assert!(optimized_rx.iter().all(|rx| rx.try_recv().is_err()));
        }
    }
}
