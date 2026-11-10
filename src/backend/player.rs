//! Plays voice notes inside the app (issue #54): one note at a time, with
//! pause, seek and speeds 1× / 1.5× / 2×. The note is decoded once on its own
//! thread; faster speeds are time-stretched so voices keep their pitch. Audio
//! goes out through cpal's default output device.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, mpsc};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use super::voice::RATE;

/// The speeds a voice note plays at, in the order the chip cycles them.
pub const SPEEDS: [f32; 3] = [1.0, 1.5, 2.0];

/// What the bubble of a message shows about playback.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Playback {
    pub playing: bool,
    /// Seconds into the note, on the note's own clock (not the sped-up one).
    pub position: f32,
    pub duration: f32,
    pub speed: f32,
}

/// The note being played (or paused), shared with the audio callback.
struct Track {
    id: String,
    /// The note at 1×.
    original: Arc<Vec<f32>>,
    /// What the callback reads: `original`, or it stretched to `speed`.
    buffer: Arc<Vec<f32>>,
    speed: f32,
}

#[derive(Default)]
struct Shared {
    track: Mutex<Option<Track>>,
    /// Read position in `buffer`, in samples, as f64 bits.
    position: AtomicU64,
    playing: AtomicBool,
    /// The message being loaded, so its bubble can show it is starting.
    loading: Mutex<Option<String>>,
}

enum Command {
    Play { id: String, path: PathBuf, from: f32 },
    /// Samples already decoded: a recording listened to before it is sent.
    PlayBuffer { id: String, pcm: Arc<Vec<f32>> },
    Speed(f32),
}

struct Player {
    shared: Arc<Shared>,
    commands: mpsc::Sender<Command>,
}

fn player() -> &'static Player {
    static PLAYER: OnceLock<Player> = OnceLock::new();
    PLAYER.get_or_init(|| {
        let shared = Arc::new(Shared::default());
        let (commands, receive) = mpsc::channel();
        let worker = shared.clone();
        std::thread::Builder::new()
            .name("player".into())
            .spawn(move || run(worker, receive))
            .expect("spawn player thread");
        Player { shared, commands }
    })
}

/// Playback of message `id`, when it is the note loaded in the player.
pub fn status(id: &str) -> Option<Playback> {
    let shared = &player().shared;
    let track = shared.track.lock().ok()?;
    let track = track.as_ref().filter(|t| t.id == id)?;
    let at = f64::from_bits(shared.position.load(Ordering::Relaxed));
    Some(Playback {
        playing: shared.playing.load(Ordering::Relaxed),
        position: (at * f64::from(track.speed) / f64::from(RATE)) as f32,
        duration: track.original.len() as f32 / RATE as f32,
        speed: track.speed,
    })
}

/// Whether message `id` is being decoded to start playing.
pub fn is_loading(id: &str) -> bool {
    player().shared.loading.lock().is_ok_and(|l| l.as_deref() == Some(id))
}

/// Plays or pauses message `id` from `path`. Another note stops.
pub fn toggle(id: &str, path: &Path) {
    let shared = &player().shared;
    let loaded = shared.track.lock().is_ok_and(|t| t.as_ref().is_some_and(|t| t.id == id));
    if loaded {
        let playing = !shared.playing.load(Ordering::Relaxed);
        // A note that ran to its end starts again from the beginning.
        if playing && status(id).is_some_and(|s| s.position >= s.duration - 0.05) {
            shared.position.store(0f64.to_bits(), Ordering::Relaxed);
        }
        shared.playing.store(playing, Ordering::Relaxed);
    } else {
        let _ = player().commands.send(Command::Play { id: id.to_owned(), path: path.to_owned(), from: 0.0 });
    }
}

/// Jumps message `id` to `fraction` (0 to 1) of its length, starting it when
/// it is not the note loaded yet.
pub fn seek(id: &str, path: &Path, fraction: f32) {
    let fraction = fraction.clamp(0.0, 1.0);
    let shared = &player().shared;
    let Ok(track) = shared.track.lock() else { return };
    match track.as_ref().filter(|t| t.id == id) {
        Some(track) => {
            let at = fraction as f64 * track.buffer.len() as f64;
            shared.position.store(at.to_bits(), Ordering::Relaxed);
        }
        None => {
            let _ = player().commands.send(Command::Play { id: id.to_owned(), path: path.to_owned(), from: fraction });
        }
    }
}

/// Plays 48 kHz mono `pcm` as `id` (the voice note being recorded, heard
/// back while paused), from the start.
pub fn play_buffer(id: &str, pcm: Vec<f32>) {
    let _ = player().commands.send(Command::PlayBuffer { id: id.to_owned(), pcm: Arc::new(pcm) });
}

/// Stops message `id` if it is the note loaded.
pub fn forget(id: &str) {
    let shared = &player().shared;
    let Ok(mut track) = shared.track.lock() else { return };
    if track.as_ref().is_some_and(|t| t.id == id) {
        shared.playing.store(false, Ordering::Relaxed);
        *track = None;
    }
}

/// The next speed after the current one (1× → 1.5× → 2× → 1×).
pub fn next_speed(speed: f32) -> f32 {
    let at = SPEEDS.iter().position(|s| (s - speed).abs() < 0.01).unwrap_or(0);
    SPEEDS[(at + 1) % SPEEDS.len()]
}

/// Changes the speed of the note that is loaded.
pub fn set_speed(speed: f32) {
    let _ = player().commands.send(Command::Speed(speed));
}

/// The player thread: owns the output stream (cpal streams are not `Send`),
/// decodes notes and stretches them for other speeds.
fn run(shared: Arc<Shared>, commands: mpsc::Receiver<Command>) {
    let mut stream: Option<cpal::Stream> = None;
    while let Ok(command) = commands.recv() {
        match command {
            Command::Play { id, path, from } => {
                shared.playing.store(false, Ordering::Relaxed);
                if let Ok(mut loading) = shared.loading.lock() {
                    *loading = Some(id.clone());
                }
                let decoded = std::fs::read(&path).map_err(|e| e.to_string()).and_then(|bytes| super::voice::decode(&bytes));
                if let Ok(mut loading) = shared.loading.lock() {
                    *loading = None;
                }
                let pcm = match decoded {
                    Ok(pcm) if !pcm.is_empty() => Arc::new(pcm),
                    Ok(_) => continue,
                    Err(e) => {
                        log::warn!("cannot play voice note: {e}");
                        continue;
                    }
                };
                // Keeps the speed the previous note was played at, as WhatsApp does.
                let speed = shared.track.lock().ok().and_then(|t| t.as_ref().map(|t| t.speed)).unwrap_or(1.0);
                let buffer = if speed == 1.0 { pcm.clone() } else { Arc::new(stretch(&pcm, speed)) };
                let at = f64::from(from) * buffer.len() as f64;
                if let Ok(mut track) = shared.track.lock() {
                    *track = Some(Track { id, original: pcm, buffer, speed });
                }
                shared.position.store(at.to_bits(), Ordering::Relaxed);
                if stream.is_none() {
                    stream = open_output(shared.clone()).inspect_err(|e| log::warn!("audio output: {e}")).ok();
                }
                shared.playing.store(stream.is_some(), Ordering::Relaxed);
            }
            Command::PlayBuffer { id, pcm } => {
                if pcm.is_empty() {
                    continue;
                }
                if let Ok(mut track) = shared.track.lock() {
                    *track = Some(Track { id, buffer: pcm.clone(), original: pcm, speed: 1.0 });
                }
                shared.position.store(0f64.to_bits(), Ordering::Relaxed);
                if stream.is_none() {
                    stream = open_output(shared.clone()).inspect_err(|e| log::warn!("audio output: {e}")).ok();
                }
                shared.playing.store(stream.is_some(), Ordering::Relaxed);
            }
            Command::Speed(speed) => {
                let Some((original, old_speed, old_len)) = shared
                    .track
                    .lock()
                    .ok()
                    .and_then(|t| t.as_ref().map(|t| (t.original.clone(), t.speed, t.buffer.len())))
                else {
                    continue;
                };
                let buffer = if speed == 1.0 { original.clone() } else { Arc::new(stretch(&original, speed)) };
                // The same moment of the note, on the new buffer.
                let at = f64::from_bits(shared.position.load(Ordering::Relaxed));
                let fraction = if old_len == 0 { 0.0 } else { at / old_len as f64 };
                let new_at = fraction * buffer.len() as f64;
                if let Ok(mut track) = shared.track.lock() {
                    if let Some(track) = track.as_mut().filter(|t| (t.speed - old_speed).abs() < 0.01) {
                        track.buffer = buffer;
                        track.speed = speed;
                    }
                }
                shared.position.store(new_at.to_bits(), Ordering::Relaxed);
            }
        }
    }
}

/// Opens the default output device; the callback reads the shared track.
fn open_output(shared: Arc<Shared>) -> Result<cpal::Stream, String> {
    let device = cpal::default_host().default_output_device().ok_or("no audio output device")?;
    let config = device.default_output_config().map_err(|e| e.to_string())?;
    let format = config.sample_format();
    let config: cpal::StreamConfig = config.into();
    let stream = match format {
        cpal::SampleFormat::F32 => speak::<f32>(&device, config, shared),
        cpal::SampleFormat::I16 => speak::<i16>(&device, config, shared),
        cpal::SampleFormat::I32 => speak::<i32>(&device, config, shared),
        cpal::SampleFormat::U16 => speak::<u16>(&device, config, shared),
        other => return Err(format!("output format {other} is not supported")),
    }
    .map_err(|e| e.to_string())?;
    stream.play().map_err(|e| e.to_string())?;
    Ok(stream)
}

fn speak<T>(device: &cpal::Device, config: cpal::StreamConfig, shared: Arc<Shared>) -> Result<cpal::Stream, cpal::Error>
where
    T: cpal::SizedSample + cpal::FromSample<f32>,
{
    let channels = usize::from(config.channels.max(1));
    // Steps through the 48 kHz buffer at the device's own rate.
    let step = f64::from(RATE) / f64::from(config.sample_rate.max(1));
    device.build_output_stream(
        config,
        move |out: &mut [T], _: &cpal::OutputCallbackInfo| {
            let silence = T::from_sample_(0.0);
            // Never wait in the audio callback: a busy lock is one silent block.
            let track = shared.track.try_lock();
            let buffer = match &track {
                Ok(guard) => guard.as_ref().map(|t| t.buffer.clone()),
                Err(_) => None,
            };
            drop(track);
            let (Some(buffer), true) = (buffer, shared.playing.load(Ordering::Relaxed)) else {
                out.fill(silence);
                return;
            };
            let mut at = f64::from_bits(shared.position.load(Ordering::Relaxed));
            for frame in out.chunks_mut(channels) {
                let sample = sample_at(&buffer, at);
                frame.fill(T::from_sample_(sample));
                if at < buffer.len() as f64 {
                    at += step;
                }
            }
            if at >= buffer.len() as f64 {
                // Ran to the end: stop there, showing the full bar.
                at = buffer.len() as f64;
                shared.playing.store(false, Ordering::Relaxed);
            }
            shared.position.store(at.to_bits(), Ordering::Relaxed);
        },
        |e| log::warn!("audio output: {e}"),
        None,
    )
}

/// Linear interpolation between samples; silence past the end.
fn sample_at(buffer: &[f32], at: f64) -> f32 {
    let i = at.floor() as usize;
    let Some(&a) = buffer.get(i) else { return 0.0 };
    let b = buffer.get(i + 1).copied().unwrap_or(a);
    let t = (at - i as f64) as f32;
    a + (b - a) * t
}

/// Window of the overlap-add time stretch: about 40 ms at 48 kHz.
const WINDOW: usize = 1920;
/// Output hop, a quarter window, so the Hann windows add up flat.
const HOP: usize = WINDOW / 4;

/// `pcm` played `speed` times faster at the same pitch: overlap-add of Hann
/// windowed slices taken `speed` hops apart, each nudged (within a few ms) to
/// line up with the output so far, which keeps speech free of warble.
pub fn stretch(pcm: &[f32], speed: f32) -> Vec<f32> {
    if pcm.len() < WINDOW * 2 || speed <= 0.0 {
        return pcm.to_vec();
    }
    let window: Vec<f32> = (0..WINDOW)
        .map(|i| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / WINDOW as f32).cos())
        .collect();
    let frames = ((pcm.len() as f32 / speed - WINDOW as f32) / HOP as f32).max(0.0) as usize + 1;
    let mut out = vec![0.0f32; frames * HOP + WINDOW];
    let mut weight = vec![0.0f32; out.len()];
    // How far a slice may move to match the overlap: ±6 ms.
    let search = 288usize;
    for frame in 0..frames {
        let at_out = frame * HOP;
        let nominal = (frame as f32 * HOP as f32 * speed) as usize;
        let mut start = nominal.min(pcm.len().saturating_sub(WINDOW));
        if frame > 0 {
            // Best match between the new slice and what is already there.
            let overlap = WINDOW - HOP;
            let lo = nominal.saturating_sub(search);
            let hi = (nominal + search).min(pcm.len().saturating_sub(WINDOW));
            let mut best = (f32::MIN, start);
            for candidate in (lo..=hi).step_by(4) {
                let score: f32 = (0..overlap)
                    .step_by(8)
                    .map(|i| out[at_out + i] * pcm[candidate + i])
                    .sum();
                if score > best.0 {
                    best = (score, candidate);
                }
            }
            start = best.1;
        }
        for i in 0..WINDOW {
            out[at_out + i] += pcm[start + i] * window[i];
            weight[at_out + i] += window[i];
        }
    }
    for (sample, w) in out.iter_mut().zip(&weight) {
        if *w > 1e-3 {
            *sample /= w;
        }
    }
    let length = (pcm.len() as f32 / speed) as usize;
    out.truncate(length.min(out.len()));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(seconds: f32, hz: f32) -> Vec<f32> {
        (0..(RATE as f32 * seconds) as usize)
            .map(|i| (2.0 * std::f32::consts::PI * hz * i as f32 / RATE as f32).sin() * 0.5)
            .collect()
    }

    /// Zero crossings per second, a rough pitch.
    fn crossings(pcm: &[f32]) -> f32 {
        let n = pcm.windows(2).filter(|w| (w[0] < 0.0) != (w[1] < 0.0)).count();
        n as f32 / (pcm.len() as f32 / RATE as f32)
    }

    #[test]
    fn stretch_keeps_pitch_and_shortens() {
        let pcm = tone(2.0, 220.0);
        for speed in [1.5, 2.0] {
            let fast = stretch(&pcm, speed);
            let expected = pcm.len() as f32 / speed;
            assert!((fast.len() as f32 - expected).abs() < 0.02 * expected, "length at {speed}×");
            let (a, b) = (crossings(&pcm), crossings(&fast[WINDOW..fast.len() - WINDOW]));
            assert!((a - b).abs() / a < 0.05, "pitch at {speed}×: {a} vs {b}");
        }
    }

    #[test]
    fn speeds_cycle() {
        assert_eq!(next_speed(1.0), 1.5);
        assert_eq!(next_speed(1.5), 2.0);
        assert_eq!(next_speed(2.0), 1.0);
        assert_eq!(next_speed(0.7), 1.5);
    }

    #[test]
    fn interpolates_and_ends_silent() {
        let buffer = [0.0, 1.0];
        assert_eq!(sample_at(&buffer, 0.5), 0.5);
        assert_eq!(sample_at(&buffer, 5.0), 0.0);
    }
}
