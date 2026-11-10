//! WhatsApp voice calls (issue #39, stage 1: incoming 1:1 calls). One call at
//! a time across every number: it rings (a tone and a notification), can be
//! answered, declined or ended, muted, and the speaker silenced. Audio runs
//! between the default microphone and speaker and the call's 16 kHz mono
//! frames, on threads of its own. Each call leaves a line in the chat.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use whatsapp_rust::Client;
use whatsapp_rust::voip::{CallEvent, CallHandle};
use whatsapp_rust::wacore::types::call::{CallAction, CallEndedElsewhere, ElsewhereOutcome, IncomingCall, MissedCall};

use super::store::{self, Delivery, Store};
use super::Updater;
use crate::i18n::tr;

/// The call's audio: 16 kHz mono, 60 ms frames.
const CALL_RATE: u32 = 16_000;
const FRAME: usize = 960;

/// Where a call stands, for the UI.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Phase {
    Ringing,
    Connecting,
    Active,
}

/// The call the UI shows: ringing, connecting or in progress.
#[derive(Clone, Debug)]
pub struct CallView {
    pub session: String,
    pub call_id: String,
    /// The caller, normalized like chat JIDs.
    pub peer: String,
    /// The caller's push name, when WhatsApp sent one.
    pub name: Option<String>,
    pub video: bool,
    pub phase: Phase,
    /// When it started ringing, and when its media connected.
    pub since: Instant,
    pub connected_at: Option<Instant>,
    pub muted: bool,
    pub speaker_off: bool,
    /// Loudness of our microphone and of the caller, 0 to 1.
    pub levels: Arc<Levels>,
}

#[derive(Debug, Default)]
pub struct Levels {
    mic: AtomicU32,
    remote: AtomicU32,
}

impl Levels {
    pub fn remote(&self) -> f32 {
        f32::from_bits(self.remote.load(Ordering::Relaxed))
    }
}

/// The call being handled, with what it needs to go on.
struct Active {
    view: CallView,
    incoming: Box<IncomingCall>,
    client: Arc<Client>,
    store: Arc<Mutex<Store>>,
    handle: Option<CallHandle>,
    audio: Option<Bridge>,
    ring: Option<Arc<AtomicBool>>,
}

fn active() -> &'static Mutex<Option<Active>> {
    static ACTIVE: OnceLock<Mutex<Option<Active>>> = OnceLock::new();
    ACTIVE.get_or_init(Default::default)
}

/// The call shown now, if any.
pub fn view() -> Option<CallView> {
    active().lock().ok()?.as_ref().map(|a| a.view.clone())
}

fn publish(state: &Updater) {
    state.set_call(view());
}

/// A `<call>` from WhatsApp: an offer rings, a terminate ends it.
pub fn on_incoming(state: &Updater, session: &str, client: &Arc<Client>, store: &Arc<Mutex<Store>>, call: &IncomingCall) {
    match &call.action {
        CallAction::Offer { call_id, is_video, group_jid, .. } => {
            if call.offline || group_jid.is_some() {
                return;
            }
            let Ok(mut slot) = active().lock() else { return };
            if slot.is_some() {
                // One call at a time: a second one keeps ringing on the phone.
                log::info!("session {session}: call {call_id} while busy; left to the phone");
                return;
            }
            let peer = normalize(state, session, &call.from.to_string());
            let ring = Arc::new(AtomicBool::new(true));
            ring_tone(ring.clone());
            *slot = Some(Active {
                view: CallView {
                    session: session.to_owned(),
                    call_id: call_id.clone(),
                    peer: peer.clone(),
                    name: call.notify.clone().filter(|n| !n.is_empty()),
                    video: *is_video,
                    phase: Phase::Ringing,
                    since: Instant::now(),
                    connected_at: None,
                    muted: false,
                    speaker_off: false,
                    levels: Arc::default(),
                },
                incoming: Box::new(call.clone()),
                client: client.clone(),
                store: store.clone(),
                handle: None,
                audio: None,
                ring: Some(ring),
            });
            drop(slot);
            super::notify::incoming_call(state, session, &peer, call.notify.as_deref().unwrap_or_default(), *is_video);
            publish(state);
        }
        CallAction::Terminate { call_id, duration, .. } => {
            let ended = take_if(call_id);
            if let Some(a) = ended {
                let outcome = match a.view.phase {
                    Phase::Ringing => Outcome::Missed,
                    _ => Outcome::Ended(duration.unwrap_or_else(|| talk_seconds(&a.view))),
                };
                finish(state, a, outcome);
            }
        }
        _ => {}
    }
}

/// An offer that must not ring (replayed after being offline), or one the
/// caller gave up on: logged as missed.
pub fn on_missed(state: &Updater, session: &str, store: &Arc<Mutex<Store>>, missed: &MissedCall) {
    if let Some(a) = take_if(&missed.call_id) {
        finish(state, a, Outcome::Missed);
    } else {
        let peer = normalize(state, session, &missed.from.to_string());
        log_call(state, session, store, &peer, &missed.call_id, false, Outcome::Missed, missed.timestamp.timestamp());
    }
}

/// Another of our devices answered or declined the call ringing here.
pub fn on_elsewhere(state: &Updater, ended: &CallEndedElsewhere) {
    if let Some(a) = take_if(&ended.call_id) {
        let outcome = if ended.outcome == ElsewhereOutcome::Accepted { Outcome::Elsewhere } else { Outcome::Declined };
        finish(state, a, outcome);
    }
}

/// Answers the ringing call: opens the microphone and speaker and joins.
pub fn answer(state: &Updater) {
    let (client, incoming, levels, call_id) = {
        let Ok(mut slot) = active().lock() else { return };
        let Some(a) = slot.as_mut().filter(|a| a.view.phase == Phase::Ringing) else { return };
        stop_ring(a);
        a.view.phase = Phase::Connecting;
        (a.client.clone(), a.incoming.clone(), a.view.levels.clone(), a.view.call_id.clone())
    };
    publish(state);
    let state = state.clone();
    tokio::spawn(async move {
        let (mic_tx, mic_rx) = async_channel::bounded::<Vec<i16>>(3);
        let (spk_tx, spk_rx) = async_channel::bounded::<Vec<i16>>(64);
        let bridge = match Bridge::open(mic_tx, spk_rx, levels) {
            Ok(bridge) => bridge,
            Err(e) => {
                log::warn!("call {call_id}: audio: {e}");
                let _ = client.voip().reject(&incoming).await;
                if let Some(a) = take_if(&call_id) {
                    finish(&state, a, Outcome::Failed(e));
                }
                return;
            }
        };
        match client.voip().accept(&incoming).audio(mic_rx, spk_tx).start().await {
            Ok(handle) => {
                let events = handle.take_events();
                let watched = handle.clone();
                let kept = match active().lock() {
                    Ok(mut slot) => match slot.as_mut().filter(|a| a.view.call_id == call_id) {
                        Some(a) => {
                            a.handle = Some(handle);
                            a.audio = Some(bridge);
                            true
                        }
                        None => false,
                    },
                    Err(_) => false,
                };
                // Ended (or declined) while it was being set up.
                if !kept {
                    let _ = watched.terminate().await;
                    return;
                }
                if let Some(mut events) = events {
                    let (state, call_id) = (state.clone(), call_id.clone());
                    tokio::spawn(async move {
                        while let Ok(event) = events.recv().await {
                            if event == CallEvent::MediaConnected {
                                if let Ok(mut slot) = active().lock() {
                                    if let Some(a) = slot.as_mut().filter(|a| a.view.call_id == call_id) {
                                        a.view.phase = Phase::Active;
                                        a.view.connected_at = Some(Instant::now());
                                    }
                                }
                                publish(&state);
                            }
                        }
                    });
                }
                // However it ends (either side, or the network), clean up once.
                watched.wait_ended().await;
                if let Some(a) = take_if(&call_id) {
                    let seconds = talk_seconds(&a.view);
                    finish(&state, a, Outcome::Ended(seconds));
                }
            }
            Err(e) => {
                log::warn!("call {call_id}: answer failed: {e}");
                let _ = client.voip().reject(&incoming).await;
                if let Some(a) = take_if(&call_id) {
                    finish(&state, a, Outcome::Failed(e.to_string()));
                }
            }
        }
    });
}

/// Declines the ringing call.
pub fn decline(state: &Updater) {
    let Some(a) = take_ringing() else { return };
    let (client, incoming) = (a.client.clone(), a.incoming.clone());
    finish(state, a, Outcome::Declined);
    tokio::spawn(async move {
        if let Err(e) = client.voip().reject(&incoming).await {
            log::warn!("call: decline failed: {e}");
        }
    });
}

/// Hangs up the call in progress.
pub fn end(state: &Updater) {
    let handle = active().lock().ok().and_then(|slot| slot.as_ref().and_then(|a| a.handle.clone()));
    match handle {
        // `wait_ended` in `answer` logs it and cleans up.
        Some(handle) => {
            tokio::spawn(async move {
                let _ = handle.terminate().await;
            });
        }
        None => {
            if let Some(a) = active().lock().ok().and_then(|mut slot| slot.take()) {
                let (client, incoming) = (a.client.clone(), a.incoming.clone());
                finish(state, a, Outcome::Declined);
                tokio::spawn(async move {
                    let _ = client.voip().reject(&incoming).await;
                });
            }
        }
    }
}

/// Mutes or unmutes our microphone (the caller is told).
pub fn set_muted(state: &Updater, muted: bool) {
    let handle = {
        let Ok(mut slot) = active().lock() else { return };
        let Some(a) = slot.as_mut() else { return };
        a.view.muted = muted;
        a.handle.clone()
    };
    publish(state);
    if let Some(handle) = handle {
        tokio::spawn(async move {
            if let Err(e) = handle.set_muted(muted).await {
                log::warn!("call: mute failed: {e}");
            }
        });
    }
}

/// Silences (or restores) the caller on our speaker.
pub fn set_speaker_off(state: &Updater, off: bool) {
    if let Ok(mut slot) = active().lock() {
        if let Some(a) = slot.as_mut() {
            a.view.speaker_off = off;
            if let Some(audio) = &a.audio {
                audio.speaker_off.store(off, Ordering::Relaxed);
            }
        }
    }
    publish(state);
}

fn take_if(call_id: &str) -> Option<Active> {
    let mut slot = active().lock().ok()?;
    if slot.as_ref().is_some_and(|a| a.view.call_id == call_id) { slot.take() } else { None }
}

fn take_ringing() -> Option<Active> {
    let mut slot = active().lock().ok()?;
    if slot.as_ref().is_some_and(|a| a.view.phase == Phase::Ringing) { slot.take() } else { None }
}

fn stop_ring(a: &mut Active) {
    if let Some(ring) = a.ring.take() {
        ring.store(false, Ordering::Relaxed);
    }
}

fn talk_seconds(view: &CallView) -> u32 {
    view.connected_at.map(|t| t.elapsed().as_secs() as u32).unwrap_or(0)
}

/// How a call ended, for its line in the chat.
#[derive(Clone, Debug, PartialEq)]
enum Outcome {
    Missed,
    Declined,
    /// Answered on another of our devices.
    Elsewhere,
    /// Talked for this many seconds.
    Ended(u32),
    Failed(String),
}

impl Outcome {
    fn key(&self) -> &'static str {
        match self {
            Outcome::Missed => "missed",
            Outcome::Declined => "declined",
            Outcome::Elsewhere => "elsewhere",
            Outcome::Ended(_) => "ended",
            Outcome::Failed(_) => "failed",
        }
    }
}

/// Tears a call down (ring, audio) and logs it.
fn finish(state: &Updater, mut a: Active, outcome: Outcome) {
    stop_ring(&mut a);
    drop(a.audio.take());
    let started = chrono_now() - a.view.since.elapsed().as_secs() as i64;
    log_call(state, &a.view.session, &a.store, &a.view.peer, &a.view.call_id, a.view.video, outcome, started);
    publish(state);
}

fn chrono_now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// The call's line in the chat: a message from the caller whose media is
/// `{"k": "call", ...}`, so it sorts, previews and syncs like any other.
#[allow(clippy::too_many_arguments)]
fn log_call(state: &Updater, session: &str, store: &Mutex<Store>, peer: &str, call_id: &str, video: bool, outcome: Outcome, ts: i64) {
    let seconds = if let Outcome::Ended(s) = outcome { s } else { 0 };
    let label = call_label(outcome.key(), video, seconds);
    let record = store::msg_record(peer, &format!("call-{call_id}"), false, peer, &label, ts, &Delivery::Received, false);
    let media = whatsapp_rust::serde_json::json!({ "k": "call", "out": outcome.key(), "secs": seconds, "video": video });
    state.record(session, store, &[store::with_media(record, Some(media))]);
}

/// The text a call line shows in chat previews: "[Missed voice call]".
pub fn call_label(outcome: &str, video: bool, seconds: u32) -> String {
    let duration = format!("{}:{:02}", seconds / 60, seconds % 60);
    match (outcome, video) {
        ("missed", false) => tr("[Missed voice call]", "[Panggilan suara tak terjawab]").to_owned(),
        ("missed", true) => tr("[Missed video call]", "[Panggilan video tak terjawab]").to_owned(),
        ("declined", _) => tr("[Declined call]", "[Panggilan ditolak]").to_owned(),
        ("elsewhere", _) => tr("[Answered on another device]", "[Dijawab di perangkat lain]").to_owned(),
        ("failed", _) => tr("[Call could not be connected]", "[Panggilan gagal tersambung]").to_owned(),
        (_, true) => crate::trf!("[Video call · {duration}]", "[Panggilan video · {duration}]"),
        _ => crate::trf!("[Voice call · {duration}]", "[Panggilan suara · {duration}]"),
    }
}

fn normalize(state: &Updater, session: &str, jid: &str) -> String {
    let bare = store::bare(jid);
    state.book(session).map(|b| b.normalize(&bare)).unwrap_or(bare)
}

/// The ring while a call waits: two tones, two seconds on and four off, at a
/// gentle volume, until `on` is cleared.
fn ring_tone(on: Arc<AtomicBool>) {
    std::thread::Builder::new()
        .name("ring".into())
        .spawn(move || {
            let Some(device) = cpal::default_host().default_output_device() else { return };
            let Ok(config) = device.default_output_config() else { return };
            if config.sample_format() != cpal::SampleFormat::F32 {
                return;
            }
            let config: cpal::StreamConfig = config.into();
            let (rate, channels) = (config.sample_rate as f32, usize::from(config.channels.max(1)));
            let mut n = 0u64;
            let playing = on.clone();
            let stream = device.build_output_stream(
                config,
                move |out: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    for frame in out.chunks_mut(channels) {
                        let t = n as f32 / rate;
                        n += 1;
                        let sounding = playing.load(Ordering::Relaxed) && (t % 6.0) < 2.0;
                        let v = if sounding {
                            let p = std::f32::consts::TAU;
                            0.08 * ((p * 440.0 * t).sin() + (p * 480.0 * t).sin())
                        } else {
                            0.0
                        };
                        frame.fill(v);
                    }
                },
                |e| log::warn!("ring: {e}"),
                None,
            );
            let Ok(stream) = stream else { return };
            if stream.play().is_err() {
                return;
            }
            while on.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(100));
            }
        })
        .ok();
}

/// The microphone and speaker of a call: cpal streams on their own threads
/// (they are not `Send` everywhere), joined to the call's channels by two
/// tokio tasks that resample between the devices' rates and 16 kHz.
struct Bridge {
    running: Arc<AtomicBool>,
    speaker_off: Arc<AtomicBool>,
}

impl Drop for Bridge {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
    }
}

type Ring = Arc<Mutex<VecDeque<f32>>>;

impl Bridge {
    fn open(mic: async_channel::Sender<Vec<i16>>, speaker: async_channel::Receiver<Vec<i16>>, levels: Arc<Levels>) -> Result<Self, String> {
        let host = cpal::default_host();
        let input = host.default_input_device().ok_or(tr("No microphone", "Tidak ada mikrofon"))?;
        let output = host.default_output_device().ok_or(tr("No speaker", "Tidak ada speaker"))?;
        let in_config = input.default_input_config().map_err(|e| e.to_string())?;
        let out_config = output.default_output_config().map_err(|e| e.to_string())?;
        if in_config.sample_format() != cpal::SampleFormat::F32 || out_config.sample_format() != cpal::SampleFormat::F32 {
            return Err(tr("This audio device format is not supported yet", "Format perangkat audio ini belum didukung").to_owned());
        }
        let in_config: cpal::StreamConfig = in_config.into();
        let out_config: cpal::StreamConfig = out_config.into();
        let (in_rate, out_rate) = (in_config.sample_rate, out_config.sample_rate);
        let running = Arc::new(AtomicBool::new(true));
        let speaker_off = Arc::new(AtomicBool::new(false));
        let heard: Ring = Arc::default();
        let playing: Ring = Arc::default();

        // Microphone: the callback only downmixes into a buffer.
        {
            let (running, heard, levels) = (running.clone(), heard.clone(), levels.clone());
            let channels = usize::from(in_config.channels.max(1));
            std::thread::Builder::new()
                .name("call-mic".into())
                .spawn(move || {
                    let buffer = heard.clone();
                    let stream = input.build_input_stream(
                        in_config,
                        move |data: &[f32], _: &cpal::InputCallbackInfo| {
                            let mut peak = 0.0f32;
                            if let Ok(mut buffer) = buffer.try_lock() {
                                for frame in data.chunks(channels) {
                                    let v = frame.iter().sum::<f32>() / channels as f32;
                                    peak = peak.max(v.abs());
                                    buffer.push_back(v);
                                }
                                // Keep at most a second if nobody reads it.
                                let excess = buffer.len().saturating_sub(in_rate as usize);
                                buffer.drain(..excess);
                            }
                            levels.mic.store(peak.min(1.0).to_bits(), Ordering::Relaxed);
                        },
                        |e| log::warn!("call microphone: {e}"),
                        None,
                    );
                    hold(stream, &running);
                })
                .map_err(|e| e.to_string())?;
        }
        // Speaker: the callback only takes from a buffer, silence when short.
        {
            let (running, playing, off) = (running.clone(), playing.clone(), speaker_off.clone());
            let channels = usize::from(out_config.channels.max(1));
            std::thread::Builder::new()
                .name("call-speaker".into())
                .spawn(move || {
                    let buffer = playing.clone();
                    let stream = output.build_output_stream(
                        out_config,
                        move |out: &mut [f32], _: &cpal::OutputCallbackInfo| {
                            let mut buffer = buffer.try_lock().ok();
                            let silent = off.load(Ordering::Relaxed);
                            for frame in out.chunks_mut(channels) {
                                let v = buffer.as_mut().and_then(|b| b.pop_front()).unwrap_or(0.0);
                                frame.fill(if silent { 0.0 } else { v });
                            }
                        },
                        |e| log::warn!("call speaker: {e}"),
                        None,
                    );
                    hold(stream, &running);
                })
                .map_err(|e| e.to_string())?;
        }
        // Microphone → call: 16 kHz, 960-sample frames.
        {
            let running = running.clone();
            tokio::spawn(async move {
                let mut resample = Resampler::new(in_rate, CALL_RATE);
                let mut pending: Vec<i16> = Vec::with_capacity(FRAME * 2);
                while running.load(Ordering::Relaxed) {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                    let taken: Vec<f32> = match heard.lock() {
                        Ok(mut buffer) => buffer.drain(..).collect(),
                        Err(_) => continue,
                    };
                    for v in resample.push(&taken) {
                        pending.push((v.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16);
                    }
                    while pending.len() >= FRAME {
                        let frame: Vec<i16> = pending.drain(..FRAME).collect();
                        // A full channel drops the frame, as the call's own path does.
                        if mic.try_send(frame).is_err() && mic.is_closed() {
                            return;
                        }
                    }
                }
            });
        }
        // Call → speaker, noting how loud the caller is.
        {
            let running = running.clone();
            tokio::spawn(async move {
                let mut resample = Resampler::new(CALL_RATE, out_rate);
                while running.load(Ordering::Relaxed) {
                    let Ok(frame) = speaker.recv().await else { return };
                    let samples: Vec<f32> = frame.iter().map(|&s| f32::from(s) / f32::from(i16::MAX)).collect();
                    let peak = samples.iter().fold(0.0f32, |m, s| m.max(s.abs()));
                    levels.remote.store(peak.min(1.0).to_bits(), Ordering::Relaxed);
                    let out = resample.push(&samples);
                    if let Ok(mut buffer) = playing.lock() {
                        buffer.extend(out);
                        // More than half a second queued: the device fell behind, catch up.
                        let excess = buffer.len().saturating_sub(out_rate as usize / 2);
                        buffer.drain(..excess);
                    }
                }
            });
        }
        Ok(Self { running, speaker_off })
    }
}

/// Plays `stream` until `running` is cleared, then drops it on this thread.
fn hold(stream: Result<cpal::Stream, cpal::Error>, running: &AtomicBool) {
    let stream = match stream {
        Ok(stream) => stream,
        Err(e) => {
            log::warn!("call audio: {e}");
            return;
        }
    };
    if let Err(e) = stream.play() {
        log::warn!("call audio: {e}");
        return;
    }
    while running.load(Ordering::Relaxed) {
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Linear resampling that keeps its place across chunks.
struct Resampler {
    step: f64,
    at: f64,
    last: f32,
}

impl Resampler {
    fn new(from: u32, to: u32) -> Self {
        Self { step: f64::from(from) / f64::from(to.max(1)), at: 0.0, last: 0.0 }
    }

    /// Resamples `input`, which follows the previous chunk's last sample.
    fn push(&mut self, input: &[f32]) -> Vec<f32> {
        let n = input.len();
        if n == 0 {
            return Vec::new();
        }
        let mut out = Vec::with_capacity((n as f64 / self.step) as usize + 1);
        // Index 0 is the previous chunk's last sample, 1..=n this chunk.
        let at = |k: usize| if k == 0 { self.last } else { input[k - 1] };
        while self.at < n as f64 {
            let i = self.at.floor() as usize;
            let t = (self.at - i as f64) as f32;
            let (a, b) = (at(i), at(i + 1));
            out.push(a + (b - a) * t);
            self.at += self.step;
        }
        self.last = input[n - 1];
        self.at -= n as f64;
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resampler_keeps_rate_across_chunks() {
        let mut r = Resampler::new(48_000, 16_000);
        let total: usize = (0..100).map(|_| r.push(&[0.5; 480]).len()).sum();
        // 48 000 samples in, 16 000 out (give or take one).
        assert!((total as i64 - 16_000).abs() <= 1, "{total}");
        let mut up = Resampler::new(16_000, 48_000);
        let total: usize = (0..10).map(|_| up.push(&[0.25; 960]).len()).sum();
        assert!((total as i64 - 28_800).abs() <= 1, "{total}");
    }

    #[test]
    fn call_labels() {
        assert_eq!(call_label("missed", false, 0), "[Missed voice call]");
        assert_eq!(call_label("ended", false, 125), "[Voice call · 2:05]");
        assert_eq!(call_label("ended", true, 768), "[Video call · 12:48]");
    }
}
