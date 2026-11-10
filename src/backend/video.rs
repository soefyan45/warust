//! Video for calls (issue #55) through ffmpeg run as a separate process, with
//! no codec in the app, as whatsapp-rust's own `voip-cli` does:
//! - the camera: ffmpeg captures it, scales it to 640x360 at 15 fps and
//!   encodes H.264 Constrained Baseline as Annex-B; the access units go to
//!   the call. A second, small output is the "You" preview, mirrored;
//! - the caller: their access units go into another ffmpeg, which writes RGBA
//!   frames of a fixed size that the UI draws.
//!
//! Without ffmpeg (or a camera) video is simply not offered; calls stay voice.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Child, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use whatsapp_rust::voip::VideoFrame;
use whatsapp_rust::wacore::voip::h264::{AnnexBAuSplitter, au_has_idr};

use crate::i18n::tr;

/// What we send: 640x360 at 15 fps (the call's default 90 kHz step of 6000).
const SEND: (usize, usize) = (640, 360);
const FPS: u32 = 15;
/// The "You" preview.
const PREVIEW: (usize, usize) = (192, 108);
/// The caller's video is drawn on a square canvas, so portrait and landscape
/// both fit without knowing which comes.
const CANVAS: usize = 640;

/// A video picture for the UI, replaced as new ones arrive.
#[derive(Clone)]
pub struct Picture {
    pub width: usize,
    pub height: usize,
    pub rgba: Arc<Vec<u8>>,
    pub generation: u64,
}

impl std::fmt::Debug for Picture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Picture({}x{} #{})", self.width, self.height, self.generation)
    }
}

/// The latest picture of a stream, shared with the UI.
pub type Latest = Arc<Mutex<Option<Picture>>>;

/// ffmpeg's path: on PATH, or where Homebrew puts it (an app opened from
/// Finder does not get Homebrew's PATH). Checked once.
pub fn ffmpeg() -> Option<PathBuf> {
    static FOUND: OnceLock<Option<PathBuf>> = OnceLock::new();
    FOUND
        .get_or_init(|| {
            let mut candidates = vec![PathBuf::from("ffmpeg")];
            if cfg!(target_os = "macos") {
                candidates.insert(0, PathBuf::from("/opt/homebrew/bin/ffmpeg"));
                candidates.insert(1, PathBuf::from("/usr/local/bin/ffmpeg"));
            }
            candidates.into_iter().find(|path| {
                crate::platform::command(path).arg("-version").stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success())
            })
        })
        .clone()
}

/// The camera ffmpeg; dropping it stops the camera.
pub struct Camera {
    child: Child,
    running: Arc<AtomicBool>,
}

impl Drop for Camera {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The ffmpeg input for the default camera on this OS.
fn camera_input() -> Result<Vec<String>, String> {
    let fps = FPS.to_string();
    if cfg!(target_os = "macos") {
        // AVFoundation cameras usually only offer 30 fps; the filter makes it 15.
        Ok(["-f", "avfoundation", "-framerate", "30", "-i", "0:none"].map(str::to_owned).to_vec())
    } else if cfg!(windows) {
        let name = windows_camera().ok_or(tr("No camera found", "Kamera tidak ditemukan"))?;
        Ok(vec!["-f".into(), "dshow".into(), "-rtbufsize".into(), "64M".into(), "-i".into(), format!("video={name}")])
    } else {
        let device = (0..4).map(|i| format!("/dev/video{i}")).find(|d| std::path::Path::new(d).exists()).ok_or(tr("No camera found", "Kamera tidak ditemukan"))?;
        Ok(vec!["-f".into(), "v4l2".into(), "-framerate".into(), fps, "-i".into(), device])
    }
}

/// The first DirectShow video device, from ffmpeg's own listing.
fn windows_camera() -> Option<String> {
    let path = ffmpeg()?;
    let out = crate::platform::command(path).args(["-hide_banner", "-list_devices", "true", "-f", "dshow", "-i", "dummy"]).output().ok()?;
    let text = String::from_utf8_lossy(&out.stderr);
    text.lines().filter(|l| l.contains("(video)")).find_map(|l| l.split('"').nth(1).map(str::to_owned))
}

/// Takes one photo with the camera into `dir` as a JPEG (#71). The first
/// frames are skipped, while the camera still sets its exposure.
pub fn snapshot(dir: &std::path::Path) -> Result<std::path::PathBuf, String> {
    let path = ffmpeg().ok_or(tr("Taking a photo needs ffmpeg installed", "Mengambil foto butuh ffmpeg terpasang"))?;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    let out = dir.join(format!("camera-{stamp}.jpg"));
    let status = crate::platform::command(path)
        .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-y"])
        .args(camera_input()?)
        .args(["-vf", "select=gte(n\\,15)", "-frames:v", "1", "-q:v", "2"])
        .arg(&out)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|e| e.to_string())?;
    if status.success() && out.is_file() {
        Ok(out)
    } else {
        Err(tr("The camera could not take a photo", "Kamera tidak bisa mengambil foto").to_owned())
    }
}

/// Starts the camera. Access units go to the returned channel, from the
/// first keyframe on; the mirrored preview goes to `preview`.
pub fn open_camera(preview: Latest, repaint: Arc<dyn Fn() + Send + Sync>) -> Result<(Camera, async_channel::Receiver<Vec<u8>>), String> {
    let path = ffmpeg().ok_or(tr("Video calls need ffmpeg installed", "Video call butuh ffmpeg terpasang"))?;
    let (w, h) = SEND;
    let (pw, ph) = PREVIEW;
    let graph = format!(
        "[0:v]scale={w}:{h}:force_original_aspect_ratio=decrease:force_divisible_by=2,pad={w}:{h}:(ow-iw)/2:(oh-ih)/2,setsar=1,fps={FPS},format=yuv420p,split=2[send][look];\
         [look]scale={pw}:{ph},hflip,format=rgba[you]"
    );
    let gop = (FPS * 2).to_string();
    let mut command = crate::platform::command(path);
    command
        .args(["-hide_banner", "-loglevel", "quiet", "-nostats", "-nostdin"])
        .args(camera_input()?)
        .args(["-filter_complex", &graph])
        .args(["-map", "[send]", "-c:v", "libx264", "-profile:v", "baseline", "-level:v", "3.1", "-pix_fmt", "yuv420p"])
        .args(["-preset", "veryfast", "-tune", "zerolatency", "-g", &gop, "-keyint_min", &gop, "-sc_threshold", "0"])
        .args(["-b:v", "800k", "-maxrate", "800k", "-bufsize", "200k", "-x264-params", "repeat-headers=1:sliced-threads=0:threads=1"])
        .args(["-bsf:v", "h264_metadata=aud=insert", "-an", "-f", "h264", "pipe:1"])
        // The preview rides on stderr, which stays quiet otherwise.
        .args(["-map", "[you]", "-f", "rawvideo", "pipe:2"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().map_err(|e| crate::trf!("The camera cannot start: {e}", "Kamera tidak bisa dinyalakan: {e}"))?;
    let (Some(mut stdout), Some(stderr)) = (child.stdout.take(), child.stderr.take()) else {
        let _ = child.kill();
        return Err(tr("The camera cannot start", "Kamera tidak bisa dinyalakan").to_owned());
    };
    let running = Arc::new(AtomicBool::new(true));
    let (send, frames) = async_channel::bounded::<Vec<u8>>(4);
    // Access units, from the first IDR; a full channel drops until the next one.
    {
        let running = running.clone();
        std::thread::Builder::new()
            .name("camera".into())
            .spawn(move || {
                let mut splitter = AnnexBAuSplitter::default();
                let mut out = Vec::new();
                let mut buffer = vec![0u8; 32 * 1024];
                let (mut started, mut dropping) = (false, false);
                while running.load(Ordering::Relaxed) {
                    let Ok(n) = stdout.read(&mut buffer) else { break };
                    if n == 0 {
                        break;
                    }
                    splitter.push(&buffer[..n], &mut out);
                    for au in out.drain(..) {
                        let idr = au_has_idr(&au);
                        if !started && !idr {
                            continue;
                        }
                        started = true;
                        if dropping && !idr {
                            continue;
                        }
                        dropping = match send.try_send(au) {
                            Ok(()) => false,
                            Err(async_channel::TrySendError::Full(_)) => true,
                            Err(async_channel::TrySendError::Closed(_)) => return,
                        };
                    }
                }
            })
            .map_err(|e| e.to_string())?;
    }
    read_pictures(stderr, PREVIEW, preview, running.clone(), repaint);
    Ok((Camera { child, running }, frames))
}

/// Reads fixed-size RGBA frames from `pipe` into `latest`.
fn read_pictures(mut pipe: impl Read + Send + 'static, (w, h): (usize, usize), latest: Latest, running: Arc<AtomicBool>, repaint: Arc<dyn Fn() + Send + Sync>) {
    static GENERATION: AtomicU64 = AtomicU64::new(1);
    let _ = std::thread::Builder::new().name("video-frames".into()).spawn(move || {
        let mut frame = vec![0u8; w * h * 4];
        while running.load(Ordering::Relaxed) {
            if pipe.read_exact(&mut frame).is_err() {
                break;
            }
            let picture = Picture { width: w, height: h, rgba: Arc::new(frame.clone()), generation: GENERATION.fetch_add(1, Ordering::Relaxed) };
            if let Ok(mut slot) = latest.lock() {
                *slot = Some(picture);
            }
            repaint();
        }
    });
}

/// The decoder of the caller's video; dropping it stops it.
pub struct Decoder {
    running: Arc<AtomicBool>,
    child: Arc<Mutex<Option<Child>>>,
}

impl Drop for Decoder {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        if let Some(mut child) = self.child.lock().ok().and_then(|mut c| c.take()) {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// The filter that undoes the caller's rotation (quarter turns).
fn rotation(orientation: u8) -> &'static str {
    match orientation & 3 {
        1 => "transpose=cclock,",
        2 => "hflip,vflip,",
        3 => "transpose=clock,",
        _ => "",
    }
}

/// Decodes the call's incoming video (`frames`) into `latest`. ffmpeg starts
/// at the first keyframe, and again when the rotation changes.
pub fn open_decoder(frames: async_channel::Receiver<VideoFrame>, latest: Latest, repaint: Arc<dyn Fn() + Send + Sync>) -> Decoder {
    let running = Arc::new(AtomicBool::new(true));
    let child: Arc<Mutex<Option<Child>>> = Arc::default();
    let (alive, slot) = (running.clone(), child.clone());
    let _ = std::thread::Builder::new().name("video-decode".into()).spawn(move || {
        let mut stdin: Option<std::process::ChildStdin> = None;
        let mut turned: Option<u8> = None;
        let mut reader: Option<Arc<AtomicBool>> = None;
        while alive.load(Ordering::Relaxed) {
            let Ok(frame) = frames.recv_blocking() else { break };
            // A new decoder at a keyframe: the first one, or the rotation changed.
            if frame.keyframe && turned != Some(frame.orientation) {
                if let Some(stop) = reader.take() {
                    stop.store(false, Ordering::Relaxed);
                }
                drop(stdin.take());
                if let Some(mut old) = slot.lock().ok().and_then(|mut c| c.take()) {
                    let _ = old.kill();
                    let _ = old.wait();
                }
                let Some(path) = ffmpeg() else { break };
                let filter = format!(
                    "{}scale={CANVAS}:{CANVAS}:force_original_aspect_ratio=decrease,pad={CANVAS}:{CANVAS}:(ow-iw)/2:(oh-ih)/2:color=black,format=rgba",
                    rotation(frame.orientation)
                );
                let fps = FPS.to_string();
                let spawned = crate::platform::command(path)
                    .args(["-hide_banner", "-loglevel", "quiet", "-nostats"])
                    // No `nobuffer`: on a pipe it loses what was read while probing, and
                    // then nothing comes out.
                    .args(["-fflags", "discardcorrupt", "-flags", "low_delay", "-probesize", "32", "-analyzeduration", "0"])
                    .args(["-f", "h264", "-framerate", &fps, "-i", "pipe:0", "-vf", &filter, "-fps_mode", "passthrough"])
                    .args(["-f", "rawvideo", "-pix_fmt", "rgba", "pipe:1"])
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::null())
                    .spawn();
                let Ok(mut process) = spawned else {
                    log::warn!("video: the decoder cannot start");
                    break;
                };
                stdin = process.stdin.take();
                let stop = Arc::new(AtomicBool::new(true));
                if let Some(stdout) = process.stdout.take() {
                    read_pictures(stdout, (CANVAS, CANVAS), latest.clone(), stop.clone(), repaint.clone());
                }
                reader = Some(stop);
                turned = Some(frame.orientation);
                if let Ok(mut c) = slot.lock() {
                    *c = Some(process);
                }
            }
            if let Some(pipe) = stdin.as_mut() {
                if pipe.write_all(&frame.data).is_err() {
                    stdin = None;
                    turned = None;
                }
            }
        }
        if let Some(stop) = reader {
            stop.store(false, Ordering::Relaxed);
        }
    });
    Decoder { running, child }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotation_filters() {
        assert_eq!(rotation(0), "");
        assert_eq!(rotation(1), "transpose=cclock,");
        assert_eq!(rotation(6), "hflip,vflip,");
        assert_eq!(rotation(3), "transpose=clock,");
    }

    #[test]
    fn send_rate_fits_the_call_clock() {
        // The call steps its 90 kHz clock by 90000/15 per access unit by default.
        assert_eq!(90_000 % FPS, 0);
        assert_eq!(90_000 / FPS, 6_000);
    }
}
