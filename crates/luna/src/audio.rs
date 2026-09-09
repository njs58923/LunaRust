//! Native mixer bridge. Voices belong to the creating worker and stop on unload/restart.
use bevy::{
    audio::{AddAudioSource, AudioSourceBundle, Decodable, PlaybackSettings},
    prelude::*,
};
use js_runtime::audio::{AudioDecoder, SharedPlayback};

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
}
#[derive(Resource, Default)]
struct AudioVoices(Vec<Voice>);
pub struct SpaceAudioPlugin;
impl Plugin for SpaceAudioPlugin {
    fn build(&self, app: &mut App) {
        app.add_audio_source::<SpaceAudioSource>()
            .init_resource::<AudioVoices>()
            .add_systems(Update, maintain_audio);
    }
}
pub fn apply_commands(world: &mut World, owner: u32, commands: Vec<SharedPlayback>) {
    if commands.is_empty() {
        return;
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
        let entity = world
            .spawn(AudioSourceBundle {
                source: asset.clone(),
                settings: PlaybackSettings::ONCE,
                ..default()
            })
            .id();
        world.resource_mut::<AudioVoices>().0.push(Voice {
            owner,
            worker,
            playback,
            entity,
            asset,
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
