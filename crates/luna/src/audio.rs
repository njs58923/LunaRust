//! Native mixer bridge. Voices belong to the creating worker and stop on unload/restart.
use bevy::{audio::Decodable, prelude::*};
use bevy_mod_xr::session::XrState;
use js_runtime::audio::{AudioDecoder, SharedPlayback};
use rodio::cpal::traits::{DeviceTrait, HostTrait};
use rodio::{OutputStream, OutputStreamHandle, Sink};

#[derive(Asset, TypePath)]
pub struct SpaceAudioSource {
    playback: SharedPlayback,
}
impl Decodable for SpaceAudioSource {
    type DecoderItem = f32;
    type Decoder = AudioDecoder;
    fn decoder(&self) -> AudioDecoder {
        AudioDecoder::new(self.playback.clone())
    }
}
struct Voice {
    owner: u32,
    worker: Option<std::thread::ThreadId>,
    playback: SharedPlayback,
    entity: Entity,
    asset: Handle<SpaceAudioSource>,
    sink: Option<Sink>,
}
#[derive(Resource, Default)]
struct AudioVoices(Vec<Voice>);

// Own the stream instead of Bevy's private, permanently-default AudioOutput.
// NonSend keeps device creation/destruction on the main thread.
#[derive(Default)]
struct AudioOutputRouter {
    output: Option<(OutputStream, OutputStreamHandle)>,
    last_vr: Option<bool>,
}

fn headset_device_index(names: &[String], explicit: Option<&str>) -> Result<Option<usize>, String> {
    let matches: Vec<_> = names
        .iter()
        .enumerate()
        .filter(|(_, name)| {
            if let Some(explicit) = explicit {
                name.trim().eq_ignore_ascii_case(explicit.trim())
            } else {
                name.to_ascii_lowercase()
                    .contains("oculus virtual audio device")
            }
        })
        .map(|(index, _)| index)
        .collect();
    match matches.as_slice() {
        [index] => Ok(Some(*index)),
        [] if explicit.is_none() => Ok(None),
        [] => Err("Configured VR audio device was not found".into()),
        _ => Err(
            "Multiple matching VR audio devices; select a unique LUNA_VR_AUDIO_DEVICE name".into(),
        ),
    }
}

fn open_audio_output(vr: bool) -> Result<(OutputStream, OutputStreamHandle, String), String> {
    let host = rodio::cpal::default_host();
    if vr {
        let devices: Vec<_> = match host.output_devices() {
            Ok(devices) => devices
                .filter_map(|device| device.name().ok().map(|name| (device, name)))
                .collect(),
            Err(error) => {
                warn!("[Audio] Cannot enumerate VR outputs: {error}; using system default");
                Vec::new()
            }
        };
        let names: Vec<_> = devices.iter().map(|(_, name)| name.clone()).collect();
        let explicit = std::env::var("LUNA_VR_AUDIO_DEVICE")
            .ok()
            .filter(|name| !name.trim().is_empty());
        match headset_device_index(&names, explicit.as_deref()) {
            Ok(Some(index)) => {
                let (device, name) = &devices[index];
                match OutputStream::try_from_device(device) {
                    Ok((stream, handle)) => return Ok((stream, handle, name.clone())),
                    Err(error) => {
                        warn!("[Audio] Cannot open headset {name}: {error}; using system default")
                    }
                }
            }
            Ok(None) => warn!(
                "[Audio] Quest Link output unavailable; using system default. Outputs: {names:?}"
            ),
            Err(error) => warn!("[Audio] {error}; using system default. Outputs: {names:?}"),
        }
    }
    let device = host
        .default_output_device()
        .ok_or("No default audio output device")?;
    let name = device.name().unwrap_or_else(|_| "System default".into());
    OutputStream::try_from_device(&device)
        .map(|(stream, handle)| (stream, handle, name))
        .map_err(|e| e.to_string())
}

fn wants_headset_output(vr: bool, xr: Option<XrState>) -> bool {
    vr && matches!(
        xr,
        Some(XrState::Idle | XrState::Ready | XrState::Running | XrState::Stopping)
    )
}

fn route_audio(
    mode: Option<Res<crate::RenderMode>>,
    xr: Option<Res<XrState>>,
    mut router: NonSendMut<AudioOutputRouter>,
    mut voices: ResMut<AudioVoices>,
    mut log: Option<ResMut<crate::LogPanel>>,
) {
    let vr = wants_headset_output(mode.is_some_and(|mode| mode.is_vr), xr.map(|state| *state));
    if router.last_vr != Some(vr) {
        router.last_vr = Some(vr);
        match open_audio_output(vr) {
            Ok((stream, handle, name)) => {
                for voice in &mut voices.0 {
                    if let Some(sink) = voice.sink.take() {
                        sink.stop();
                    }
                }
                // Drop the old device before attaching new decoders, so two
                // mixers never drain the same SharedPlayback concurrently.
                router.output.take();
                router.output = Some((stream, handle));
                let message = format!(
                    "[Audio] {} output: {name}",
                    if vr { "VR" } else { "Desktop" }
                );
                info!("{message}");
                if let Some(log) = log.as_mut() {
                    log.push_info(message);
                }
            }
            Err(error) => {
                let message =
                    format!("[Audio] Output change failed: {error}; retaining previous output");
                warn!("{message}");
                if let Some(log) = log.as_mut() {
                    log.push_warn(message);
                }
            }
        }
    }
    let Some((_, handle)) = &router.output else {
        for voice in &voices.0 {
            let mut playback = voice.playback.lock().unwrap();
            if playback.error.is_none() {
                playback.error = Some("No audio output device is available".into());
                playback.playing = false;
            }
        }
        return;
    };
    for voice in &mut voices.0 {
        if voice.sink.is_some() {
            continue;
        }
        match Sink::try_new(handle) {
            Ok(sink) => {
                voice.playback.lock().unwrap().error = None;
                sink.append(AudioDecoder::new(voice.playback.clone()));
                voice.sink = Some(sink);
            }
            Err(error) => {
                let mut playback = voice.playback.lock().unwrap();
                playback.error = Some(format!("Audio output: {error}"));
                playback.playing = false;
            }
        }
    }
}
pub struct SpaceAudioPlugin;
impl Plugin for SpaceAudioPlugin {
    fn build(&self, app: &mut App) {
        app.init_asset::<SpaceAudioSource>()
            .init_resource::<AudioVoices>()
            .init_non_send_resource::<AudioOutputRouter>()
            .add_systems(Update, (maintain_audio, route_audio).chain());
    }
}
pub fn apply_commands(world: &mut World, owner: u32, commands: Vec<SharedPlayback>) {
    if commands.is_empty() {
        return;
    }
    // Retry unavailable hardware only on a new audio request or mode change,
    // never by polling the devices every frame.
    if let Some(mut router) = world.get_non_send_resource_mut::<AudioOutputRouter>() {
        if router.output.is_none() {
            router.last_vr = None;
        }
    }
    let allowed = world
        .get_resource::<crate::SpacePolicies>()
        .and_then(|p| p.by_space.get(&owner))
        .is_some_and(|p| p.effective_caps.contains(crate::CapabilityBits::AUDIO));
    let worker = world
        .get_non_send_resource::<crate::js::ScriptRuntimeManager>()
        .and_then(|m| m.contexts.get(&owner))
        .and_then(|w| w.join.as_ref())
        .map(|j| j.thread().id());
    for playback in commands {
        {
            let mut p = playback.lock().unwrap();
            if p.disposed {
                continue;
            }
            if !allowed || !world.contains_resource::<AudioVoices>() {
                p.error = Some(
                    if !allowed {
                        "Permission denied: audio was not granted"
                    } else {
                        "Audio output is not installed"
                    }
                    .into(),
                );
                p.playing = false;
                continue;
            }
            p.allowed = true;
        }
        let asset = world
            .resource_mut::<Assets<SpaceAudioSource>>()
            .add(SpaceAudioSource {
                playback: playback.clone(),
            });
        let entity = world.spawn_empty().id();
        world.resource_mut::<AudioVoices>().0.push(Voice {
            owner,
            worker,
            playback,
            entity,
            asset,
            sink: None,
        });
    }
}
fn maintain_audio(world: &mut World) {
    let live = world
        .get_non_send_resource::<crate::js::ScriptRuntimeManager>()
        .map(|m| {
            m.contexts
                .iter()
                .map(|(id, w)| (*id, w.join.as_ref().map(|j| j.thread().id())))
                .collect::<std::collections::HashMap<_, _>>()
        })
        .unwrap_or_default();
    world.resource_scope(|world, mut voices: Mut<AudioVoices>| {
        voices.0.retain(|v| {
            let allowed = world
                .get_resource::<crate::SpacePolicies>()
                .and_then(|p| p.by_space.get(&v.owner))
                .is_some_and(|p| p.effective_caps.contains(crate::CapabilityBits::AUDIO));
            let mut p = v.playback.lock().unwrap();
            if live.get(&v.owner) != Some(&v.worker) {
                p.disposed = true;
            }
            if !allowed {
                p.allowed = false;
                p.playing = false;
                p.error = Some("Audio permission revoked".into());
            }
            if p.disposed || !allowed {
                p.disposed = true;
                drop(p);
                if let Some(entity) = world.get_entity_mut(v.entity) {
                    entity.despawn_recursive();
                }
                world
                    .resource_mut::<Assets<SpaceAudioSource>>()
                    .remove(v.asset.id());
                false
            } else {
                true
            }
        });
    });
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn headset_selection_uses_link_output_or_explicit_name_without_guessing() {
        let names = vec![
            "Speakers (Realtek)".into(),
            "Auriculares (Oculus Virtual Audio Device)".into(),
        ];
        assert_eq!(headset_device_index(&names, None).unwrap(), Some(1));
        assert_eq!(
            headset_device_index(&names, Some("SPEAKERS (REALTEK)")).unwrap(),
            Some(0)
        );
        assert!(headset_device_index(&names, Some("missing")).is_err());
        assert_eq!(headset_device_index(&names[..1], None).unwrap(), None);
        assert!(headset_device_index(&[names[1].clone(), names[1].clone()], None).is_err());
    }
    #[test]
    fn audio_stays_on_headset_during_session_idle_but_returns_on_desktop() {
        for state in [
            XrState::Idle,
            XrState::Ready,
            XrState::Running,
            XrState::Stopping,
        ] {
            assert!(wants_headset_output(true, Some(state)));
            assert!(!wants_headset_output(false, Some(state)));
        }
        assert!(!wants_headset_output(true, None));
        assert!(!wants_headset_output(true, Some(XrState::Available)));
        assert!(!wants_headset_output(
            true,
            Some(XrState::Exiting {
                should_restart: true
            })
        ));
    }
    #[test]
    fn output_decoder_recreation_preserves_clip_playback_state() {
        let mut engine = js_runtime::Engine::new();
        engine
            .eval("Deno.core.ops.op_audio_create('stream',new Uint8Array(),8000,1,0.1)")
            .unwrap();
        let playback = engine.drain_audio_commands().pop().unwrap();
        {
            let mut p = playback.lock().unwrap();
            p.samples = js_runtime::audio::Samples::Clip(vec![0.0; 2000]);
            p.allowed = true;
            p.playing = false;
            p.looping = true;
            p.cursor = 123;
            p.volume = 0.25;
        }
        drop(AudioDecoder::new(playback.clone()));
        let _replacement = AudioDecoder::new(playback.clone());
        let p = playback.lock().unwrap();
        assert_eq!(p.cursor, 123);
        assert_eq!(p.volume, 0.25);
        assert!(p.looping);
        assert!(!p.playing);
        assert!(!p.disposed);
    }
    #[test]
    #[ignore = "requires a connected Quest Link audio output; opens a silent mixer"]
    fn quest_link_output_can_be_opened() {
        let (_stream, _handle, name) = open_audio_output(true).expect("open Quest Link output");
        assert!(
            name.to_ascii_lowercase()
                .contains("oculus virtual audio device"),
            "fallback selected: {name}"
        );
        eprintln!("Quest Link output opened: {name}");
    }
    #[test]
    fn audio_denial_and_unload_stop_output_and_release_asset() {
        let mut engine = js_runtime::Engine::new();
        engine
            .eval("Deno.core.ops.op_audio_create('stream',new Uint8Array(),8000,1,0.1)")
            .unwrap();
        let p = engine.drain_audio_commands().pop().unwrap();
        let mut world = World::new();
        apply_commands(&mut world, 7, vec![p.clone()]);
        assert!(p
            .lock()
            .unwrap()
            .error
            .as_ref()
            .unwrap()
            .contains("Permission denied"));
        world.init_resource::<AudioVoices>();
        world.init_resource::<Assets<SpaceAudioSource>>();
        let mut policies = crate::SpacePolicies::default();
        policies.by_space.insert(
            7,
            crate::SpacePolicy {
                effective_caps: crate::CapabilityBits::AUDIO,
                ..default()
            },
        );
        world.insert_resource(policies);
        p.lock().unwrap().error = None;
        apply_commands(&mut world, 7, vec![p.clone()]);
        let entity = world.resource::<AudioVoices>().0[0].entity;
        assert_eq!(world.resource::<Assets<SpaceAudioSource>>().len(), 1);
        // Missing owning worker is equivalent to unloading the space.
        maintain_audio(&mut world);
        assert!(p.lock().unwrap().disposed);
        assert!(world.get_entity(entity).is_none());
        assert_eq!(world.resource::<Assets<SpaceAudioSource>>().len(), 0);
    }
}
