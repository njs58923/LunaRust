//! Isolate-owned audio. Decoding runs on the JS worker, never on Bevy's render thread.
use deno_core::{op2, OpState};
use rodio::Source;
use serde::Serialize;
use std::{
    collections::{HashMap, VecDeque},
    io::Cursor,
    sync::{Arc, Mutex},
    time::Duration,
};
const MAX_BYTES: usize = 64 * 1024 * 1024;
const MAX_VOICES: usize = 16;

pub enum Samples {
    Clip(Vec<f32>),
    Stream {
        queue: VecDeque<f32>,
        capacity: usize,
        eof: bool,
    },
}
pub struct Playback {
    pub samples: Samples,
    pub channels: u16,
    pub rate: u32,
    pub cursor: usize,
    pub playing: bool,
    pub looping: bool,
    pub volume: f32,
    pub disposed: bool,
    pub ready: bool,
    pub allowed: bool,
    pub error: Option<String>,
    pub ended: bool,
}
pub type SharedPlayback = Arc<Mutex<Playback>>;
#[derive(Default)]
pub struct AudioQueue {
    next: u32,
    entries: HashMap<u32, SharedPlayback>,
    pending: Vec<SharedPlayback>,
    sizes: HashMap<u32, usize>,
}
impl Drop for AudioQueue {
    fn drop(&mut self) {
        for p in self.entries.values().chain(self.pending.iter()) {
            if let Ok(mut p) = p.lock() {
                p.disposed = true;
                p.samples = Samples::Clip(Vec::new());
            }
        }
    }
}
impl AudioQueue {
    pub fn drain(&mut self) -> Vec<SharedPlayback> {
        std::mem::take(&mut self.pending)
    }
    fn get(&self, id: u32) -> Result<SharedPlayback, anyhow::Error> {
        self.entries
            .get(&id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Unknown or disposed audio in this isolate"))
    }
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    ready: bool,
    paused: bool,
    ended: bool,
    current_time: f64,
    duration: Option<f64>,
    buffered_frames: usize,
    writable_frames: usize,
    error: Option<String>,
}
impl Playback {
    fn status(&self) -> Status {
        let (duration, buffered, free) = match &self.samples {
            Samples::Clip(v) => (
                Some(v.len() as f64 / self.channels as f64 / self.rate as f64),
                0,
                0,
            ),
            Samples::Stream {
                queue, capacity, ..
            } => (
                None,
                queue.len() / self.channels as usize,
                (capacity - queue.len()) / self.channels as usize,
            ),
        };
        Status {
            ready: self.ready,
            paused: !self.playing,
            ended: self.ended,
            current_time: self.cursor as f64 / self.channels as f64 / self.rate as f64,
            duration,
            buffered_frames: buffered,
            writable_frames: free,
            error: self.error.clone(),
        }
    }
}
#[op2(fast)]
#[smi]
pub fn op_audio_create(
    state: &mut OpState,
    #[string] kind: String,
    #[buffer] bytes: &[u8],
    sample_rate: u32,
    channels: u32,
    seconds: f64,
) -> Result<u32, anyhow::Error> {
    let q = state.borrow_mut::<AudioQueue>();
    anyhow::ensure!(
        q.entries.len() < MAX_VOICES && q.pending.len() < MAX_VOICES * 2,
        "Audio voice/creation queue limit reached"
    );
    q.pending.retain(|p| !p.lock().unwrap().disposed);
    let available = MAX_BYTES - q.sizes.values().sum::<usize>();
    let (samples, channels, rate, size) = if kind == "clip" {
        anyhow::ensure!(
            bytes.len() <= 8 * 1024 * 1024,
            "Encoded audio exceeds 8 MiB"
        );
        let decoder = rodio::Decoder::new(Cursor::new(bytes.to_vec()))?;
        let channels = decoder.channels();
        let rate = decoder.sample_rate();
        anyhow::ensure!(
            (1..=2).contains(&channels) && (8000..=192000).contains(&rate),
            "Audio must be mono/stereo, 8..192 kHz"
        );
        let mut data: Vec<f32> = decoder
            .convert_samples::<f32>()
            .take(available / 4 + 1)
            .collect();
        anyhow::ensure!(
            !data.is_empty() && data.len() * 4 <= available && data.len() % channels as usize == 0,
            "Decoded audio exceeds quota or has no complete frames"
        );
        anyhow::ensure!(
            data.iter().all(|v| v.is_finite()),
            "Decoded audio contains invalid samples"
        );
        for v in &mut data {
            *v = v.clamp(-1., 1.);
        }
        let size = data.len() * 4;
        (Samples::Clip(data), channels, rate, size)
    } else {
        anyhow::ensure!(kind == "stream", "Unknown audio source kind");
        anyhow::ensure!(
            (1..=2).contains(&channels) && (8000..=192000).contains(&sample_rate),
            "PCM must be mono/stereo, 8..192 kHz"
        );
        anyhow::ensure!(
            seconds.is_finite() && (0.1..=10.0).contains(&seconds),
            "Buffer duration must be 0.1..10 seconds"
        );
        let capacity = (seconds * sample_rate as f64).ceil() as usize * channels as usize;
        anyhow::ensure!(capacity * 4 <= available, "Audio memory quota exceeded");
        (
            Samples::Stream {
                queue: VecDeque::with_capacity(capacity),
                capacity,
                eof: false,
            },
            channels as u16,
            sample_rate,
            capacity * 4,
        )
    };
    let p = Arc::new(Mutex::new(Playback {
        samples,
        channels,
        rate,
        cursor: 0,
        playing: false,
        looping: false,
        volume: 1.,
        disposed: false,
        ready: false,
        allowed: false,
        error: None,
        ended: false,
    }));
    q.next = q
        .next
        .checked_add(1)
        .ok_or_else(|| anyhow::anyhow!("Audio ID exhausted"))?;
    let id = q.next;
    q.sizes.insert(id, size);
    q.entries.insert(id, p.clone());
    q.pending.push(p);
    Ok(id)
}
#[op2]
#[serde]
pub fn op_audio_control(
    state: &mut OpState,
    id: u32,
    #[string] action: String,
    value: f64,
) -> Result<Status, anyhow::Error> {
    let q = state.borrow_mut::<AudioQueue>();
    let shared = q.get(id)?;
    let mut p = shared.lock().unwrap();
    match action.as_str() {
        "status" => {}
        "dispose" => {
            p.disposed = true;
            p.playing = false;
            p.samples = Samples::Clip(Vec::new());
            q.entries.remove(&id);
            q.sizes.remove(&id);
        }
        "play" => {
            anyhow::ensure!(!p.disposed, "Audio disposed");
            if let Some(e) = &p.error {
                anyhow::bail!(e.clone());
            }
            if p.ended {
                match &p.samples {
                    Samples::Clip(_) => p.cursor = 0,
                    _ => anyhow::bail!("An ended PCM stream cannot restart"),
                }
            }
            p.ended = false;
            p.playing = true;
        }
        "pause" => p.playing = false,
        "volume" => {
            anyhow::ensure!(
                value.is_finite() && (0.0..=1.0).contains(&value),
                "Volume must be 0..1"
            );
            p.volume = value as f32;
        }
        "loop" => {
            anyhow::ensure!(
                matches!(p.samples, Samples::Clip(_)),
                "Live streams cannot loop"
            );
            p.looping = value != 0.;
        }
        "seek" => {
            anyhow::ensure!(value.is_finite() && value >= 0., "Invalid playback time");
            let Samples::Clip(v) = &p.samples else {
                anyhow::bail!("Live streams cannot seek")
            };
            p.cursor = ((value * p.rate as f64).floor() as usize)
                .saturating_mul(p.channels as usize)
                .min(v.len());
            p.ended = false;
        }
        "end" => {
            let Samples::Stream { eof, .. } = &mut p.samples else {
                anyhow::bail!("Not a PCM stream")
            };
            *eof = true;
        }
        _ => anyhow::bail!("Unknown audio operation"),
    }
    Ok(p.status())
}
#[op2(fast)]
#[smi]
pub fn op_audio_write(
    state: &mut OpState,
    id: u32,
    #[buffer] bytes: &[u8],
) -> Result<u32, anyhow::Error> {
    let shared = state.borrow::<AudioQueue>().get(id)?;
    let mut p = shared.lock().unwrap();
    anyhow::ensure!(!p.disposed, "Audio disposed");
    if let Some(e) = &p.error {
        anyhow::bail!(e.clone());
    }
    let channels = p.channels as usize;
    anyhow::ensure!(
        bytes.len() % (channels * 4) == 0,
        "PCM must contain complete interleaved Float32 frames"
    );
    let Samples::Stream {
        queue,
        capacity,
        eof,
    } = &mut p.samples
    else {
        anyhow::bail!("Not a PCM stream")
    };
    anyhow::ensure!(!*eof, "PCM stream already ended");
    let n = (bytes.len() / 4).min(*capacity - queue.len());
    let values: Vec<f32> = bytes[..n * 4]
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
        .collect();
    anyhow::ensure!(
        values.iter().all(|v| v.is_finite() && v.abs() <= 1.),
        "PCM samples must be finite and between -1 and 1"
    );
    queue.extend(values);
    Ok((n / channels) as u32)
}

/// Mixer reads small blocks, with no allocation or network work in the callback.
/// Temporary PCM starvation produces silence; EOF is explicit, never inferred.
pub struct AudioDecoder {
    shared: SharedPlayback,
    block: [f32; 256],
    at: usize,
    len: usize,
    channels: u16,
    rate: u32,
}
impl AudioDecoder {
    pub fn new(shared: SharedPlayback) -> Self {
        let (channels, rate) = {
            let mut p = shared.lock().unwrap();
            p.ready = true;
            (p.channels, p.rate)
        };
        Self {
            shared,
            block: [0.; 256],
            at: 0,
            len: 0,
            channels,
            rate,
        }
    }
}
impl Iterator for AudioDecoder {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        if self.at == self.len {
            // Never wait for the JS producer on the device's audio thread.
            match self.shared.try_lock() {
                Ok(mut p) => {
                    if p.disposed {
                        return None;
                    }
                    self.block.fill(0.);
                    self.len = 256;
                    if p.playing && p.allowed {
                        for out in &mut self.block {
                            let cursor = p.cursor;
                            let looping = p.looping;
                            let next = match &mut p.samples {
                                Samples::Clip(v) => {
                                    if cursor < v.len() {
                                        Some(v[cursor])
                                    } else if looping {
                                        Some(v[cursor % v.len()])
                                    } else {
                                        None
                                    }
                                }
                                Samples::Stream { queue, .. } => queue.pop_front(),
                            };
                            if let Some(v) = next {
                                *out = v * p.volume;
                                p.cursor += 1;
                                if p.looping {
                                    if let Samples::Clip(v) = &p.samples {
                                        p.cursor %= v.len();
                                    }
                                }
                            } else {
                                let eof = match &p.samples {
                                    Samples::Clip(_) => true,
                                    Samples::Stream { eof, .. } => *eof,
                                };
                                if eof {
                                    p.ended = true;
                                    p.playing = false;
                                }
                                break;
                            }
                        }
                    }
                }
                Err(std::sync::TryLockError::WouldBlock) => {
                    self.block.fill(0.);
                    self.len = 256;
                }
                Err(_) => return None,
            }
            self.at = 0;
        }
        let value = self.block[self.at];
        self.at += 1;
        Some(value)
    }
}
impl Source for AudioDecoder {
    fn current_frame_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> u16 {
        self.channels
    }
    fn sample_rate(&self) -> u32 {
        self.rate
    }
    fn total_duration(&self) -> Option<Duration> {
        None
    }
}
