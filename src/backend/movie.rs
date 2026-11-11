//! Plays a video file inside the app (#84), for statuses and chats. ffmpeg
//! decodes the picture into RGBA frames and the sound into 48 kHz PCM, which
//! `player` plays; its position is the clock the frames follow, so the two
//! stay together. A video without sound runs on the wall clock. One video at
//! a time; it stops by itself once no screen asks for it.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use super::player;
use super::video::{Latest, Picture, ffmpeg};
use super::voice::RATE;

/// Frames per second the picture is decoded at.
const FPS: f64 = 30.0;
/// The longest side of the decoded picture.
const MAX_SIDE: u32 = 960;
/// Longer videos keep opening in the system player: their sound would not
/// fit comfortably in memory.
pub const MAX_SECONDS: f32 = 20.0 * 60.0;
/// A video no screen asked about for this long stops.
const UNSEEN: Duration = Duration::from_millis(1500);

/// What a screen shows about the video it plays.
#[derive(Clone)]
pub struct Status {
    pub playing: bool,
    pub position: f32,
    pub duration: f32,
    /// Still reading the file (sound being decoded).
    pub loading: bool,
    pub ended: bool,
    pub muted: bool,
    /// The latest frame.
    pub picture: Latest,
    /// Known to be sound alone (no picture to wait for).
    pub sound_only: bool,
}

/// What ffmpeg says about a file.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Info {
    duration: f32,
    /// As shown, rotation included.
    width: u32,
    height: u32,
    audio: bool,
}

/// The wall clock of a video without sound (and before its sound is ready).
#[derive(Default)]
struct Wall {
    base: f64,
    since: Option<Instant>,
}

impl Wall {
    fn now(&self) -> f64 {
        self.base + self.since.map_or(0.0, |t| t.elapsed().as_secs_f64())
    }
}

struct Current {
    id: String,
    path: PathBuf,
    info: Option<Info>,
    picture: Latest,
    /// Bumped on seek and stop, so older decoders give up.
    epoch: Arc<AtomicU64>,
    /// The sound is loaded in `player` under `audio_id`.
    audio: Arc<AtomicBool>,
    audio_id: String,
    wall: Arc<Mutex<Wall>>,
    /// Wanted playing (the player or the wall clock follows it).
    playing: bool,
    seen: Arc<Mutex<Instant>>,
    repaint: Arc<dyn Fn() + Send + Sync>,
}

fn current() -> &'static Mutex<Option<Current>> {
    static CURRENT: OnceLock<Mutex<Option<Current>>> = OnceLock::new();
    CURRENT.get_or_init(Default::default)
}

/// Whether videos can play inside the app here.
pub fn available() -> bool {
    ffmpeg().is_some()
}

/// Starts `id` from `path`, or resumes it when it is the one loaded.
pub fn play(id: &str, path: &Path, repaint: Arc<dyn Fn() + Send + Sync>) {
    let Ok(mut slot) = current().lock() else { return };
    if let Some(c) = slot.as_mut().filter(|c| c.id == id) {
        set_playing(c, true);
        return;
    }
    if let Some(old) = slot.take() {
        retire(&old);
    }
    let c = Current {
        id: id.to_owned(),
        path: path.to_owned(),
        info: None,
        picture: Arc::default(),
        epoch: Arc::default(),
        audio: Arc::default(),
        audio_id: format!("movie:{id}"),
        wall: Arc::default(),
        playing: true,
        seen: Arc::new(Mutex::new(Instant::now())),
        repaint,
    };
    let (epoch, path, audio, audio_id, wall, picture, seen, repaint) =
        (c.epoch.clone(), c.path.clone(), c.audio.clone(), c.audio_id.clone(), c.wall.clone(), c.picture.clone(), c.seen.clone(), c.repaint.clone());
    let id = id.to_owned();
    *slot = Some(c);
    drop(slot);
    std::thread::Builder::new()
        .name("movie".into())
        .spawn(move || {
            let Some(info) = probe(&path) else {
                log::warn!("video {}: ffmpeg could not read it", path.display());
                return;
            };
            if let Ok(mut slot) = current().lock() {
                match slot.as_mut().filter(|c| c.id == id) {
                    Some(c) => c.info = Some(info),
                    None => return,
                }
            }
            if info.audio && info.duration <= MAX_SECONDS {
                match decode_audio(&path) {
                    Ok(pcm) if !pcm.is_empty() => {
                        player::play_buffer(&audio_id, pcm);
                        audio.store(true, Ordering::Relaxed);
                        // The wanted state: paused while the sound was decoding stays paused.
                        if let Ok(slot) = current().lock() {
                            if let Some(c) = slot.as_ref().filter(|c| c.id == id) {
                                player::set_playing(&c.audio_id, c.playing);
                            }
                        }
                    }
                    Ok(_) => {}
                    Err(e) => log::warn!("video {}: no sound: {e}", path.display()),
                }
            }
            if !audio.load(Ordering::Relaxed) {
                if let Ok(slot) = current().lock() {
                    if let (Some(c), Ok(mut w)) = (slot.as_ref().filter(|c| c.id == id), wall.lock()) {
                        w.since = c.playing.then(Instant::now);
                    }
                }
            }
            let start = epoch.load(Ordering::Relaxed);
            decode_frames(&path, info, 0.0, start, &epoch, &picture, &audio, &audio_id, &wall, &seen, &repaint);
        })
        .ok();
}

/// Pauses or resumes `id`.
pub fn toggle(id: &str) {
    let Ok(mut slot) = current().lock() else { return };
    if let Some(c) = slot.as_mut().filter(|c| c.id == id) {
        // One that ran to its end starts again.
        let ended = status_of(c).ended;
        if ended {
            drop(slot);
            seek(id, 0.0);
            if let Ok(mut slot) = current().lock() {
                if let Some(c) = slot.as_mut().filter(|c| c.id == id) {
                    set_playing(c, true);
                }
            }
            return;
        }
        let playing = !c.playing;
        set_playing(c, playing);
    }
}

/// Pauses `id` (the status viewer while typing a comment).
pub fn pause(id: &str) {
    if let Ok(mut slot) = current().lock() {
        if let Some(c) = slot.as_mut().filter(|c| c.id == id && c.playing) {
            set_playing(c, false);
        }
    }
}

/// Jumps `id` to `fraction` (0 to 1) of its length.
pub fn seek(id: &str, fraction: f32) {
    let Ok(slot) = current().lock() else { return };
    let Some(c) = slot.as_ref().filter(|c| c.id == id) else { return };
    let Some(info) = c.info else { return };
    let at = (fraction.clamp(0.0, 1.0) * info.duration).max(0.0);
    let start = c.epoch.fetch_add(1, Ordering::Relaxed) + 1;
    if c.audio.load(Ordering::Relaxed) {
        player::seek_seconds(&c.audio_id, at);
    }
    if let Ok(mut w) = c.wall.lock() {
        w.base = f64::from(at);
        w.since = (c.playing && !c.audio.load(Ordering::Relaxed)).then(Instant::now);
    }
    let (path, epoch, picture, audio, audio_id, wall, seen, repaint) =
        (c.path.clone(), c.epoch.clone(), c.picture.clone(), c.audio.clone(), c.audio_id.clone(), c.wall.clone(), c.seen.clone(), c.repaint.clone());
    drop(slot);
    std::thread::Builder::new()
        .name("movie".into())
        .spawn(move || decode_frames(&path, info, f64::from(at), start, &epoch, &picture, &audio, &audio_id, &wall, &seen, &repaint))
        .ok();
}

/// Stops the video playing unless it is `keep` (a screen moved on to
/// something else).
pub fn stop_unless(keep: Option<&str>) {
    let other = current().lock().ok().is_some_and(|slot| slot.as_ref().is_some_and(|c| Some(c.id.as_str()) != keep));
    if other {
        stop();
    }
}

/// A repaint handle for `play`, from the egui context.
pub fn repaint_for(ctx: &egui::Context) -> Arc<dyn Fn() + Send + Sync> {
    let ctx = ctx.clone();
    Arc::new(move || ctx.request_repaint())
}

/// Stops whatever video plays.
pub fn stop() {
    if let Some(old) = current().lock().ok().and_then(|mut slot| slot.take()) {
        retire(&old);
    }
}

/// Mutes or unmutes the video's sound.
pub fn set_muted(muted: bool) {
    player::set_muted(muted);
}

/// What `id` is doing, when it is the video loaded. Asking keeps it going:
/// a video no screen asks about stops.
pub fn status(id: &str) -> Option<Status> {
    let slot = current().lock().ok()?;
    let c = slot.as_ref().filter(|c| c.id == id)?;
    if let Ok(mut seen) = c.seen.lock() {
        *seen = Instant::now();
    }
    Some(status_of(c))
}

fn status_of(c: &Current) -> Status {
    let duration = c.info.map_or(0.0, |i| i.duration);
    let (position, playing) = clock(&c.audio, &c.audio_id, &c.wall);
    let position = (position as f32).min(duration.max(0.0));
    let ended = duration > 0.0 && position >= duration - 0.15;
    Status {
        playing: c.playing && !ended,
        position,
        duration,
        loading: c.info.is_none() || (c.info.is_some_and(|i| i.audio) && !c.audio.load(Ordering::Relaxed) && playing),
        ended,
        muted: player::is_muted(),
        picture: c.picture.clone(),
        sound_only: c.info.is_some_and(|i| i.width == 0),
    }
}

fn set_playing(c: &mut Current, playing: bool) {
    c.playing = playing;
    if c.audio.load(Ordering::Relaxed) {
        player::set_playing(&c.audio_id, playing);
    } else if let Ok(mut w) = c.wall.lock() {
        let now = w.now();
        w.base = now;
        w.since = playing.then(Instant::now);
    }
    (c.repaint)();
}

fn retire(c: &Current) {
    c.epoch.fetch_add(1, Ordering::Relaxed);
    player::forget(&c.audio_id);
    player::set_muted(false);
}

/// Seconds into the video, and whether time runs: the sound's position
/// once it plays, else the wall clock.
fn clock(audio: &AtomicBool, audio_id: &str, wall: &Mutex<Wall>) -> (f64, bool) {
    if audio.load(Ordering::Relaxed) {
        // A voice note took the speaker: the video stands still.
        return match player::status(audio_id) {
            Some(s) => (f64::from(s.position), s.playing),
            None => (wall.lock().map_or(0.0, |w| w.base), false),
        };
    }
    wall.lock().map_or((0.0, false), |w| (w.now(), w.since.is_some()))
}

/// Reads frames from `from` seconds on and shows each when its time comes;
/// gives up once `epoch` moves past `start`, or no screen asks any more.
#[allow(clippy::too_many_arguments)]
fn decode_frames(
    path: &Path,
    info: Info,
    from: f64,
    start: u64,
    epoch: &AtomicU64,
    picture: &Latest,
    audio: &AtomicBool,
    audio_id: &str,
    wall: &Mutex<Wall>,
    seen: &Mutex<Instant>,
    repaint: &Arc<dyn Fn() + Send + Sync>,
) {
    // Nothing to draw for sound alone.
    if info.width == 0 {
        return;
    }
    let Some(ffmpeg) = ffmpeg() else { return };
    let (w, h) = fit(info.width, info.height);
    let filter = format!("fps={FPS},scale={w}:{h},format=rgba");
    let spawned = crate::platform::command(ffmpeg)
        .args(["-hide_banner", "-loglevel", "quiet", "-nostdin", "-ss", &format!("{from:.3}"), "-i"])
        .arg(path)
        .args(["-an", "-vf", &filter, "-f", "rawvideo", "-pix_fmt", "rgba", "pipe:1"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn();
    let mut child = match spawned {
        Ok(child) => child,
        Err(e) => return log::warn!("video: cannot start ffmpeg: {e}"),
    };
    let Some(mut out) = child.stdout.take() else { return };
    let frame_bytes = (w * h * 4) as usize;
    let gone = || epoch.load(Ordering::Relaxed) != start;
    let unseen = || seen.lock().is_ok_and(|t| t.elapsed() > UNSEEN);
    let mut n: u64 = 0;
    loop {
        let mut buffer = vec![0u8; frame_bytes];
        if gone() || out.read_exact(&mut buffer).is_err() {
            break;
        }
        let due = from + n as f64 / FPS;
        // The first frame shows at once (a paused video still has its picture).
        if n > 0 {
            loop {
                if gone() {
                    let _ = child.kill();
                    return;
                }
                if unseen() {
                    stop();
                    let _ = child.kill();
                    return;
                }
                let (now, running) = clock(audio, audio_id, wall);
                if now + 0.005 >= due {
                    break;
                }
                let wait = if running { (due - now).clamp(0.001, 0.03) } else { 0.03 };
                std::thread::sleep(Duration::from_secs_f64(wait));
            }
            // Too late to be worth showing: the next one will be on time.
            let (now, _) = clock(audio, audio_id, wall);
            if now - due > 0.1 {
                n += 1;
                continue;
            }
        }
        n += 1;
        let generation = picture.lock().ok().and_then(|p| p.as_ref().map(|p| p.generation)).unwrap_or(0) + 1;
        if let Ok(mut latest) = picture.lock() {
            *latest = Some(Picture { width: w as usize, height: h as usize, rgba: Arc::new(buffer), generation });
        }
        repaint();
    }
    let _ = child.kill();
    let _ = child.wait();
    // Out of frames: a video without sound stops its clock at the end.
    if !gone() && !audio.load(Ordering::Relaxed) {
        if let Ok(mut w) = wall.lock() {
            w.base = f64::from(info.duration);
            w.since = None;
        }
    }
    repaint();
}

/// The sound as 48 kHz mono samples.
fn decode_audio(path: &Path) -> Result<Vec<f32>, String> {
    let ffmpeg = ffmpeg().ok_or("ffmpeg is missing")?;
    let out = crate::platform::command(ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-i"])
        .arg(path)
        .args(["-vn", "-ac", "1", "-ar", &RATE.to_string(), "-f", "f32le", "pipe:1"])
        .stdin(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_owned());
    }
    Ok(out.stdout.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect())
}

/// Length, shown size and sound of `path`, from ffmpeg's own report.
fn probe(path: &Path) -> Option<Info> {
    let out = crate::platform::command(ffmpeg()?).args(["-hide_banner", "-nostdin", "-i"]).arg(path).stdin(Stdio::null()).output().ok()?;
    parse_probe(&String::from_utf8_lossy(&out.stderr))
}

/// Reads `Duration: 00:00:12.34`, the video stream's `720x1280`, a 90° or
/// 270° rotation (which swaps the sides) and whether there is an audio stream.
fn parse_probe(text: &str) -> Option<Info> {
    let duration = text
        .lines()
        .find_map(|l| l.trim().strip_prefix("Duration: "))
        .and_then(|rest| {
            let mut parts = rest.split(',').next()?.trim().split(':');
            let (h, m, s): (f32, f32, f32) = (parts.next()?.parse().ok()?, parts.next()?.parse().ok()?, parts.next()?.parse().ok()?);
            Some(h * 3600.0 + m * 60.0 + s)
        })
        .unwrap_or(0.0);
    let audio = text.lines().any(|l| l.contains("Audio:"));
    let size = text.lines().find(|l| l.contains("Video:")).and_then(|video| {
        video.split([' ', ',']).find_map(|token| {
            let (a, b) = token.split_once('x')?;
            let (w, h) = (a.parse::<u32>().ok()?, b.parse::<u32>().ok()?);
            (w >= 16 && h >= 16).then_some((w, h))
        })
    });
    // Sound alone (an mp3, a voice memo) plays too, without a picture (#86).
    let Some((width, height)) = size else {
        return audio.then_some(Info { duration, width: 0, height: 0, audio });
    };
    let turned = text.lines().map(str::trim).any(|l| {
        let quarter = |v: &str| v.trim_start_matches('-').starts_with("90") || v.trim_start_matches('-').starts_with("270");
        (l.starts_with("rotate") && l.split(':').nth(1).is_some_and(|v| quarter(v.trim())))
            || l.split("rotation of ").nth(1).is_some_and(quarter)
    });
    let (width, height) = if turned { (height, width) } else { (width, height) };
    Some(Info { duration, width, height, audio })
}

/// The decoded size: the video's shape, its longest side at most `MAX_SIDE`,
/// both sides even.
fn fit(width: u32, height: u32) -> (u32, u32) {
    let scale = (MAX_SIDE as f32 / width.max(height).max(1) as f32).min(1.0);
    let even = |v: u32| (((v as f32 * scale) / 2.0).round() as u32 * 2).max(2);
    (even(width), even(height))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PORTRAIT: &str = "Input #0, mov,mp4,m4a,3gp,3g2,mj2, from 'a.mp4':
  Duration: 00:00:12.34, start: 0.000000, bitrate: 1171 kb/s
  Stream #0:0[0x1](und): Video: h264 (High) (avc1 / 0x31637661), yuv420p(tv, bt709, progressive), 1280x720, 1033 kb/s, 30 fps
      Side data:
        displaymatrix: rotation of -90.00 degrees
  Stream #0:1[0x2](und): Audio: aac (LC) (mp4a / 0x6134706D), 44100 Hz, stereo, fltp, 128 kb/s";

    #[test]
    fn ffmpeg_reports_are_read() {
        let info = parse_probe(PORTRAIT).unwrap();
        assert!((info.duration - 12.34).abs() < 0.01);
        // The phone filmed it upright: the sides swap.
        assert_eq!((info.width, info.height), (720, 1280));
        assert!(info.audio);
        let silent = PORTRAIT.replace("        displaymatrix: rotation of -90.00 degrees\n", "").lines().filter(|l| !l.contains("Audio:")).collect::<Vec<_>>().join("\n");
        let info = parse_probe(&silent).unwrap();
        assert_eq!((info.width, info.height), (1280, 720));
        assert!(!info.audio);
        assert_eq!(parse_probe("no streams here"), None);
        let mp3 = "  Duration: 00:03:05.00, start: 0.025057, bitrate: 128 kb/s\n  Stream #0:0: Audio: mp3, 44100 Hz, stereo, fltp, 128 kb/s";
        let info = parse_probe(mp3).unwrap();
        assert_eq!((info.width, info.height, info.audio), (0, 0, true));
        assert!((info.duration - 185.0).abs() < 0.01);
    }

    #[test]
    fn frames_keep_the_shape_and_even_sides() {
        assert_eq!(fit(720, 1280), (540, 960));
        assert_eq!(fit(1920, 1080), (960, 540));
        assert_eq!(fit(320, 241), (320, 242));
    }
}
