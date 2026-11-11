//! Anti-peek "someone behind you" (#57 stage 2): the camera, read through
//! ffmpeg as two small grey frames a second, and faces found in them with
//! rustface on this computer. No frame is stored or sent; only how many faces
//! were seen leaves this module.
//!
//! The watcher lets go of the camera while a call or a photo needs it
//! ([`CameraBusy`]) and picks it up again afterwards.

use std::io::Read;
use std::process::{Child, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use rustface::ImageData;

use super::video;
use crate::i18n::tr;

/// The frames looked at: small enough to check in well under the half
/// second between them, big enough for a face a few steps behind.
const SIZE: (u32, u32) = (480, 360);
/// SeetaFace's frontal face model (BSD-2-Clause, see assets/face/LICENSE).
const MODEL: &[u8] = include_bytes!("../../assets/face/seeta_fd_frontal_v1.0.bin");
/// A camera that would not start is tried again after this.
const RETRY: Duration = Duration::from_secs(30);

/// What the watcher is doing, for the anti-peek menu.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Watch {
    #[default]
    Off,
    /// Off for now: a call or a photo has the camera.
    Paused,
    Starting,
    /// Faces in the last frame.
    Faces(usize),
    Failed(String),
}

/// The camera ffmpeg; dropping it stops the camera.
struct Running {
    child: Child,
    running: Arc<AtomicBool>,
}

impl Drop for Running {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[derive(Default)]
struct Guard {
    running: Option<Running>,
    status: Watch,
    /// Two frames in a row with more than one face.
    crowded: bool,
    failed_at: Option<Instant>,
}

fn guard() -> &'static Mutex<Guard> {
    static GUARD: OnceLock<Mutex<Guard>> = OnceLock::new();
    GUARD.get_or_init(Default::default)
}

/// Things using the camera right now.
static BUSY: AtomicUsize = AtomicUsize::new(0);

/// Held while a call or a photo uses the camera: the watcher lets go of it
/// first (only one program can read a camera on most systems).
pub struct CameraBusy(());

impl CameraBusy {
    pub fn take() -> Self {
        BUSY.fetch_add(1, Ordering::SeqCst);
        if let Ok(mut g) = guard().lock() {
            g.running = None;
            g.crowded = false;
        }
        Self(())
    }
}

impl Drop for CameraBusy {
    fn drop(&mut self) {
        BUSY.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Turns the watcher on or off; cheap, called every frame.
pub fn want(on: bool, ctx: &egui::Context) {
    let Ok(mut g) = guard().lock() else { return };
    if !on {
        g.running = None;
        g.status = Watch::Off;
        g.crowded = false;
        g.failed_at = None;
        return;
    }
    if BUSY.load(Ordering::SeqCst) > 0 {
        g.running = None;
        g.status = Watch::Paused;
        g.crowded = false;
        return;
    }
    // The camera went away (unplugged, taken by another app).
    if g.running.as_ref().is_some_and(|r| !r.running.load(Ordering::Relaxed)) {
        g.running = None;
        g.failed_at = Some(Instant::now());
    }
    if g.running.is_some() || g.failed_at.is_some_and(|at| at.elapsed() < RETRY) {
        return;
    }
    match start(ctx) {
        Ok(running) => {
            g.running = Some(running);
            g.status = Watch::Starting;
            g.failed_at = None;
        }
        Err(e) => {
            log::warn!("anti-peek camera: {e}");
            g.status = Watch::Failed(e);
            g.failed_at = Some(Instant::now());
        }
    }
}

/// What the watcher is doing, and whether someone else is looking.
pub fn status() -> (Watch, bool) {
    guard().lock().map(|g| (g.status.clone(), g.crowded)).unwrap_or_default()
}

fn start(ctx: &egui::Context) -> Result<Running, String> {
    let path = video::ffmpeg().ok_or(tr("Needs ffmpeg installed", "Butuh ffmpeg terpasang"))?;
    let (w, h) = SIZE;
    let filter = format!("fps=2,scale={w}:{h}:force_original_aspect_ratio=decrease,pad={w}:{h}:(ow-iw)/2:(oh-ih)/2,format=gray");
    let mut child = crate::platform::command(path)
        .args(["-hide_banner", "-loglevel", "quiet", "-nostats", "-nostdin"])
        .args(video::camera_input()?)
        .args(["-vf", &filter, "-f", "rawvideo", "pipe:1"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| crate::trf!("The camera cannot start: {e}", "Kamera tidak bisa dinyalakan: {e}"))?;
    let Some(stdout) = child.stdout.take() else {
        let _ = child.kill();
        return Err(tr("The camera cannot start", "Kamera tidak bisa dinyalakan").to_owned());
    };
    let running = Arc::new(AtomicBool::new(true));
    let flag = running.clone();
    let ctx = ctx.clone();
    std::thread::Builder::new().name("anti-peek".into()).spawn(move || watch(stdout, flag, ctx)).map_err(|e| e.to_string())?;
    Ok(Running { child, running })
}

/// Reads frames until the camera stops, counting the faces in each.
fn watch(mut pipe: impl Read, running: Arc<AtomicBool>, ctx: egui::Context) {
    let mut detector = match rustface::read_model(MODEL) {
        Ok(model) => rustface::create_detector_with_model(model),
        Err(e) => {
            log::warn!("anti-peek model: {e}");
            running.store(false, Ordering::Relaxed);
            return;
        }
    };
    detector.set_min_face_size(24);
    detector.set_score_thresh(2.0);
    detector.set_pyramid_scale_factor(0.8);
    detector.set_slide_window_step(4, 4);
    let (w, h) = SIZE;
    let mut frame = vec![0u8; (w * h) as usize];
    let mut before = 0;
    while pipe.read_exact(&mut frame).is_ok() {
        let faces = detector.detect(&ImageData::new(&frame, w, h)).len();
        let Ok(mut g) = guard().lock() else { break };
        if !running.load(Ordering::Relaxed) {
            return;
        }
        let crowded = faces > 1 && before > 1;
        let changed = g.status != Watch::Faces(faces) || g.crowded != crowded;
        g.status = Watch::Faces(faces);
        g.crowded = crowded;
        drop(g);
        before = faces;
        if changed {
            ctx.request_repaint();
        }
    }
    // The camera stopped by itself: `want` tries again later.
    if running.swap(false, Ordering::Relaxed) {
        if let Ok(mut g) = guard().lock() {
            g.status = Watch::Failed(tr("The camera stopped", "Kamera berhenti").to_owned());
            g.crowded = false;
        }
        ctx.request_repaint();
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_model_loads() {
        assert!(rustface::read_model(super::MODEL).is_ok());
    }
}

