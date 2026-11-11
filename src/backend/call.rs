//! WhatsApp voice calls (issue #39): incoming 1:1 calls (stage 1) and calls
//! placed from a chat (stage 2). One call at a time across every number: it
//! rings (a tone and a notification), can be answered, declined or ended,
//! muted, and the speaker silenced. Audio runs
//! between the default microphone and speaker and the call's 16 kHz mono
//! frames, on threads of its own. Each call leaves a line in the chat.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use whatsapp_rust::Client;
use whatsapp_rust::voip::{CallEvent, CallHandle, VideoFrame, VideoUpgradeToken};
use whatsapp_rust::wacore::types::call::{CallAction, CallEndedElsewhere, ElsewhereOutcome, IncomingCall, MissedCall};

use super::store::{self, Delivery, Store};
use super::video::{self, Camera, Decoder, Latest};
use super::Updater;
use crate::i18n::tr;

/// The call's audio: 16 kHz mono, 60 ms frames.
const CALL_RATE: u32 = 16_000;
const FRAME: usize = 960;

/// Where a call stands, for the UI.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Phase {
    /// A call to us, waiting for an answer.
    Ringing,
    /// Our call, waiting for theirs (see `CallView::ringing`).
    Calling,
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
    /// We placed the call (#39 stage 2); `ringing` once their phone rings.
    pub outgoing: bool,
    pub ringing: bool,
    pub phase: Phase,
    /// When it started ringing, and when its media connected.
    pub since: Instant,
    pub connected_at: Option<Instant>,
    pub muted: bool,
    pub speaker_off: bool,
    /// Loudness of our microphone and of the caller, 0 to 1.
    pub levels: Arc<Levels>,
    /// Video (#55): whether it is on, our camera, the caller asking for it,
    /// and the latest pictures of the caller and of us.
    pub video_on: bool,
    pub camera_on: bool,
    pub video_request: bool,
    pub remote: Latest,
    pub local: Latest,
    /// Why our camera could not start, shown in the call.
    pub camera_error: Option<String>,
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
    /// The offer we answer; none for a call we placed.
    incoming: Option<Box<IncomingCall>>,
    /// Our call was hung up before it was answered, or they declined it.
    canceled: bool,
    declined: bool,
    client: Arc<Client>,
    store: Arc<Mutex<Store>>,
    handle: Option<CallHandle>,
    audio: Option<Bridge>,
    ring: Option<Arc<AtomicBool>>,
    camera: Option<Camera>,
    decoder: Option<Decoder>,
    /// Where the call puts the caller's video, kept to resume our camera.
    video_sink: Option<async_channel::Sender<VideoFrame>>,
    /// Keeps a camera-less video source open (we receive, but send nothing).
    idle_source: Option<async_channel::Sender<Vec<u8>>>,
    upgrade: Option<VideoUpgradeToken>,
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
                    outgoing: false,
                    ringing: false,
                    phase: Phase::Ringing,
                    since: Instant::now(),
                    connected_at: None,
                    muted: false,
                    speaker_off: false,
                    levels: Arc::default(),
                    video_on: false,
                    camera_on: false,
                    video_request: false,
                    remote: Arc::default(),
                    local: Arc::default(),
                    camera_error: None,
                },
                incoming: Some(Box::new(call.clone())),
                canceled: false,
                declined: false,
                client: client.clone(),
                store: store.clone(),
                handle: None,
                audio: None,
                ring: Some(ring),
                camera: None,
                decoder: None,
                video_sink: None,
                idle_source: None,
                upgrade: None,
            });
            drop(slot);
            super::notify::incoming_call(state, session, &peer, call.notify.as_deref().unwrap_or_default(), *is_video);
            publish(state);
        }
        // The person we call declined (no reason): our handle ends next.
        CallAction::Reject { call_id, reason: None, .. } => {
            let handle = active().lock().ok().and_then(|mut slot| {
                let a = slot.as_mut().filter(|a| a.view.call_id == *call_id && a.view.outgoing)?;
                a.declined = true;
                a.handle.clone()
            });
            if let Some(handle) = handle {
                tokio::spawn(async move {
                    let _ = handle.terminate().await;
                });
            }
        }
        // Our own call is wound up by `follow` once its handle ends.
        CallAction::Terminate { call_id, .. } if active().lock().ok().is_some_and(|s| s.as_ref().is_some_and(|a| a.view.call_id == *call_id && a.view.outgoing)) => {}
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
        log_call(state, session, store, &peer, &missed.call_id, false, false, Outcome::Missed, missed.timestamp.timestamp());
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
    let (client, incoming, levels, call_id, wants_video, remote, local) = {
        let Ok(mut slot) = active().lock() else { return };
        let Some(a) = slot.as_mut().filter(|a| a.view.phase == Phase::Ringing) else { return };
        stop_ring(a);
        a.view.phase = Phase::Connecting;
        let wants_video = a.view.video && video::ffmpeg().is_some();
        let Some(incoming) = a.incoming.clone() else { return };
        (a.client.clone(), incoming, a.view.levels.clone(), a.view.call_id.clone(), wants_video, a.view.remote.clone(), a.view.local.clone())
    };
    let repaint = state.repaint_handle();
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
        // A video call: the caller's video is decoded; ours comes from the
        // camera, or from nothing when it cannot start (we still see them).
        let mut parts = None;
        let voip = client.voip();
        let accept = voip.accept(&incoming).audio(mic_rx, spk_tx);
        let accept = if wants_video {
            let (sink, frames) = async_channel::bounded::<VideoFrame>(8);
            let decoder = video::open_decoder(frames, remote, repaint.clone());
            let (camera, source, idle, error) = match video::open_camera(local, repaint) {
                Ok((camera, source)) => (Some(camera), source, None, None),
                Err(e) => {
                    let (idle, source) = async_channel::bounded::<Vec<u8>>(1);
                    (None, source, Some(idle), Some(e))
                }
            };
            parts = Some((camera, decoder, sink.clone(), idle, error));
            accept.video(source, sink)
        } else {
            accept
        };
        match accept.start().await {
            Ok(handle) => follow(state, call_id, handle, bridge, parts).await,
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

/// What a started call brings along: camera, decoder, where their video
/// goes, a camera-less source, and why the camera did not start.
type VideoParts = (Option<Camera>, Decoder, async_channel::Sender<VideoFrame>, Option<async_channel::Sender<Vec<u8>>>, Option<String>);

/// A started call (answered or placed), until it ends: keeps its handle,
/// audio and video, follows its events, and logs it once it is over.
async fn follow(state: Updater, call_id: String, handle: CallHandle, bridge: Bridge, mut parts: Option<VideoParts>) {
    let events = handle.take_events();
    let watched = handle.clone();
    let kept = match active().lock() {
        Ok(mut slot) => match slot.as_mut().filter(|a| a.view.call_id == call_id) {
            Some(a) => {
                a.handle = Some(handle);
                a.audio = Some(bridge);
                if let Some((camera, decoder, sink, idle, error)) = parts.take() {
                    a.view.video_on = true;
                    a.view.camera_on = camera.is_some();
                    a.view.camera_error = error;
                    a.camera = camera;
                    a.decoder = Some(decoder);
                    a.video_sink = Some(sink);
                    a.idle_source = idle;
                }
                true
            }
            None => false,
        },
        Err(_) => false,
    };
    publish(&state);
    // Ended (or declined) while it was being set up.
    if !kept {
        let _ = watched.terminate().await;
        return;
    }
    if let Some(mut events) = events {
        let (state, call_id) = (state.clone(), call_id.clone());
        tokio::spawn(async move {
            while let Ok(event) = events.recv().await {
                // The caller asks to turn the call into video.
                if let CallEvent::PeerVideoStateChanged { upgrade_token: Some(token), .. } = &event {
                    if let Ok(mut slot) = active().lock() {
                        if let Some(a) = slot.as_mut().filter(|a| a.view.call_id == call_id && !a.view.video_on) {
                            a.upgrade = Some(token.clone());
                            a.view.video_request = true;
                        }
                    }
                    publish(&state);
                }
                if event == CallEvent::MediaConnected {
                    if let Ok(mut slot) = active().lock() {
                        if let Some(a) = slot.as_mut().filter(|a| a.view.call_id == call_id) {
                            stop_ring(a);
                            a.view.phase = Phase::Active;
                            a.view.connected_at = Some(Instant::now());
                        }
                    }
                    publish(&state);
                }
            }
        });
    }
    // Our call: "Ringing…" once their phone rings, and given up after a while.
    if active().lock().ok().is_some_and(|s| s.as_ref().is_some_and(|a| a.view.call_id == call_id && a.view.outgoing)) {
        let (state, call_id, handle) = (state.clone(), call_id.clone(), watched.clone());
        tokio::spawn(async move {
            let started = Instant::now();
            loop {
                tokio::time::sleep(Duration::from_millis(400)).await;
                let ringing = handle.phase() == Some(whatsapp_rust::wacore::voip_control::signaling::CallPhase::Ringing);
                let (waiting, changed) = match active().lock() {
                    Ok(mut slot) => match slot.as_mut().filter(|a| a.view.call_id == call_id && a.view.phase == Phase::Calling) {
                        Some(a) => {
                            let changed = ringing && !a.view.ringing;
                            a.view.ringing |= ringing;
                            (true, changed)
                        }
                        None => (false, false),
                    },
                    Err(_) => (false, false),
                };
                if changed {
                    publish(&state);
                }
                if !waiting {
                    break;
                }
                if started.elapsed() > NO_ANSWER_AFTER {
                    let _ = handle.terminate().await;
                    break;
                }
            }
        });
    }
    // However it ends (either side, or the network), clean up once.
    watched.wait_ended().await;
    if let Some(a) = take_if(&call_id) {
        let outcome = match (a.view.outgoing, a.view.connected_at.is_some()) {
            (_, true) => Outcome::Ended(talk_seconds(&a.view)),
            (true, false) if a.declined => Outcome::Declined,
            (true, false) if a.canceled => Outcome::Canceled,
            (true, false) => Outcome::Unanswered,
            (false, false) => Outcome::Ended(0),
        };
        finish(&state, a, outcome);
    }
}

/// How long our call rings on their side before we give up.
const NO_ANSWER_AFTER: Duration = Duration::from_secs(60);

/// Places a call from a chat to `peer` (a phone-number JID) on `session`'s
/// number. Refused while another call is on.
pub fn place(state: &Updater, session: &str, client: &Arc<Client>, store: &Arc<Mutex<Store>>, peer: &str, video: bool) {
    let wants_video = video && video::ffmpeg().is_some();
    let pending = format!("placing-{}", chrono_now());
    let levels: Arc<Levels> = Arc::default();
    let (remote, local): (Latest, Latest) = (Arc::default(), Arc::default());
    {
        let Ok(mut slot) = active().lock() else { return };
        if slot.is_some() {
            return log::info!("session {session}: a call is already on; not placing another");
        }
        let ring = Arc::new(AtomicBool::new(true));
        ring_tone(ring.clone());
        *slot = Some(Active {
            view: CallView {
                session: session.to_owned(),
                call_id: pending.clone(),
                peer: normalize(state, session, peer),
                name: None,
                video: wants_video,
                outgoing: true,
                ringing: false,
                phase: Phase::Calling,
                since: Instant::now(),
                connected_at: None,
                muted: false,
                speaker_off: false,
                levels: levels.clone(),
                video_on: false,
                camera_on: false,
                video_request: false,
                remote: remote.clone(),
                local: local.clone(),
                camera_error: None,
            },
            incoming: None,
            canceled: false,
            declined: false,
            client: client.clone(),
            store: store.clone(),
            handle: None,
            audio: None,
            ring: Some(ring),
            camera: None,
            decoder: None,
            video_sink: None,
            idle_source: None,
            upgrade: None,
        });
    }
    publish(state);
    let (state, client, peer) = (state.clone(), client.clone(), peer.to_owned());
    let repaint = state.repaint_handle();
    tokio::spawn(async move {
        let fail = |why: String| {
            log::warn!("call to a contact failed: {why}");
            if let Some(a) = take_if(&pending) {
                finish(&state, a, Outcome::Failed(why));
            }
        };
        let Ok(jid) = peer.parse::<whatsapp_rust::Jid>() else { return fail("bad number".into()) };
        let (mic_tx, mic_rx) = async_channel::bounded::<Vec<i16>>(3);
        let (spk_tx, spk_rx) = async_channel::bounded::<Vec<i16>>(64);
        let bridge = match Bridge::open(mic_tx, spk_rx, levels) {
            Ok(bridge) => bridge,
            Err(e) => return fail(e),
        };
        let voip = client.voip();
        let mut parts = None;
        let call = voip.call(&jid).audio(mic_rx, spk_tx);
        let call = if wants_video {
            let (sink, frames) = async_channel::bounded::<VideoFrame>(8);
            let decoder = video::open_decoder(frames, remote, repaint.clone());
            let (camera, source, idle, error) = match video::open_camera(local, repaint) {
                Ok((camera, source)) => (Some(camera), source, None, None),
                Err(e) => {
                    let (idle, source) = async_channel::bounded::<Vec<u8>>(1);
                    (None, source, Some(idle), Some(e))
                }
            };
            parts = Some((camera, decoder, sink.clone(), idle, error));
            call.video(source, sink)
        } else {
            call
        };
        match call.start().await {
            Ok(handle) => {
                // The call has its real id now; a hang-up while it was being set up ends it.
                let call_id = handle.call_id().to_owned();
                let canceled = match active().lock() {
                    Ok(mut slot) => match slot.as_mut().filter(|a| a.view.call_id == pending) {
                        Some(a) => {
                            a.view.call_id = call_id.clone();
                            false
                        }
                        None => true,
                    },
                    Err(_) => true,
                };
                if canceled {
                    let _ = handle.terminate().await;
                    return;
                }
                follow(state, call_id, handle, bridge, parts).await;
            }
            Err(e) => fail(e.to_string()),
        }
    });
}

/// Declines the ringing call.
pub fn decline(state: &Updater) {
    let Some(a) = take_ringing() else { return };
    let (client, incoming) = (a.client.clone(), a.incoming.clone());
    finish(state, a, Outcome::Declined);
    let Some(incoming) = incoming else { return };
    tokio::spawn(async move {
        if let Err(e) = client.voip().reject(&incoming).await {
            log::warn!("call: decline failed: {e}");
        }
    });
}

/// Hangs up the call in progress.
pub fn end(state: &Updater) {
    let handle = active().lock().ok().and_then(|mut slot| {
        let a = slot.as_mut()?;
        // Our call hung up before they answered.
        a.canceled |= a.view.outgoing && a.view.connected_at.is_none();
        a.handle.clone()
    });
    match handle {
        // `wait_ended` in `follow` logs it and cleans up.
        Some(handle) => {
            tokio::spawn(async move {
                let _ = handle.terminate().await;
            });
        }
        None => {
            if let Some(a) = active().lock().ok().and_then(|mut slot| slot.take()) {
                let (client, incoming) = (a.client.clone(), a.incoming.clone());
                // Our call still being placed: `place` ends it once it has started.
                let outcome = if a.view.outgoing { Outcome::Canceled } else { Outcome::Declined };
                finish(state, a, outcome);
                if let Some(incoming) = incoming {
                    tokio::spawn(async move {
                        let _ = client.voip().reject(&incoming).await;
                    });
                }
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

/// Opens what a call needs for video: the decoder of the caller's video and
/// our camera. On a camera error the call still shows the caller.
fn open_video(state: &Updater, view: &CallView) -> (async_channel::Sender<VideoFrame>, Decoder, Result<(Camera, async_channel::Receiver<Vec<u8>>), String>) {
    let repaint = state.repaint_handle();
    let (sink, frames) = async_channel::bounded::<VideoFrame>(8);
    let decoder = video::open_decoder(frames, view.remote.clone(), repaint.clone());
    let camera = video::open_camera(view.local.clone(), repaint);
    (sink, decoder, camera)
}

/// Turns the voice call into a video call (the caller is asked).
pub fn start_video(state: &Updater) {
    start_or_accept_video(state, false);
}

/// Answers the caller's request to turn on video.
pub fn accept_video(state: &Updater) {
    start_or_accept_video(state, true);
}

fn start_or_accept_video(state: &Updater, accept: bool) {
    let Some((handle, view, token)) = active().lock().ok().and_then(|mut slot| {
        let a = slot.as_mut()?;
        let token = if accept { Some(a.upgrade.take()?) } else { None };
        a.view.video_request = false;
        Some((a.handle.clone()?, a.view.clone(), token))
    }) else {
        return;
    };
    let (sink, decoder, camera) = open_video(state, &view);
    let (camera, source) = match camera {
        Ok((camera, source)) => (camera, source),
        Err(e) => {
            if let Ok(mut slot) = active().lock() {
                if let Some(a) = slot.as_mut() {
                    a.view.camera_error = Some(e);
                }
            }
            publish(state);
            return;
        }
    };
    let state = state.clone();
    tokio::spawn(async move {
        let started = match token {
            Some(token) => handle.accept_video(token, source, sink.clone()).await,
            None => handle.start_video(source, sink.clone()).await,
        };
        match started {
            Ok(()) => {
                if let Ok(mut slot) = active().lock() {
                    if let Some(a) = slot.as_mut().filter(|a| a.view.call_id == view.call_id) {
                        a.view.video_on = true;
                        a.view.camera_on = true;
                        a.view.camera_error = None;
                        a.camera = Some(camera);
                        a.decoder = Some(decoder);
                        a.video_sink = Some(sink);
                    }
                }
            }
            Err(e) => log::warn!("call: video failed: {e}"),
        }
        publish(&state);
    });
}

/// Leaves the caller's request for video unanswered (it times out on their side).
pub fn decline_video(state: &Updater) {
    if let Ok(mut slot) = active().lock() {
        if let Some(a) = slot.as_mut() {
            a.upgrade = None;
            a.view.video_request = false;
        }
    }
    publish(state);
}

/// Turns our camera off (the caller stays visible) or on again.
pub fn set_camera(state: &Updater, on: bool) {
    let Some((handle, view, sink)) = active().lock().ok().and_then(|slot| {
        let a = slot.as_ref()?;
        Some((a.handle.clone()?, a.view.clone(), a.video_sink.clone()?))
    }) else {
        return;
    };
    if !on {
        if let Ok(mut slot) = active().lock() {
            if let Some(a) = slot.as_mut() {
                a.camera = None;
                a.view.camera_on = false;
                if let Ok(mut picture) = a.view.local.lock() {
                    *picture = None;
                }
            }
        }
        publish(state);
        tokio::spawn(async move {
            let _ = handle.stop_video().await;
        });
        return;
    }
    let repaint = state.repaint_handle();
    let (camera, source) = match video::open_camera(view.local.clone(), repaint) {
        Ok(opened) => opened,
        Err(e) => {
            if let Ok(mut slot) = active().lock() {
                if let Some(a) = slot.as_mut() {
                    a.view.camera_error = Some(e);
                }
            }
            publish(state);
            return;
        }
    };
    let state = state.clone();
    tokio::spawn(async move {
        match handle.resume_video(source, sink).await {
            Ok(()) => {
                if let Ok(mut slot) = active().lock() {
                    if let Some(a) = slot.as_mut().filter(|a| a.view.call_id == view.call_id) {
                        a.camera = Some(camera);
                        a.view.camera_on = true;
                        a.view.camera_error = None;
                        a.idle_source = None;
                    }
                }
            }
            Err(e) => log::warn!("call: camera on failed: {e}"),
        }
        publish(&state);
    });
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
    /// Our call: hung up before they answered, or never answered.
    Canceled,
    Unanswered,
}

impl Outcome {
    fn key(&self) -> &'static str {
        match self {
            Outcome::Missed => "missed",
            Outcome::Declined => "declined",
            Outcome::Elsewhere => "elsewhere",
            Outcome::Ended(_) => "ended",
            Outcome::Failed(_) => "failed",
            Outcome::Canceled => "canceled",
            Outcome::Unanswered => "unanswered",
        }
    }
}

/// Tears a call down (ring, audio) and logs it.
fn finish(state: &Updater, mut a: Active, outcome: Outcome) {
    stop_ring(&mut a);
    drop(a.audio.take());
    // Camera and decoder ffmpegs stop here.
    drop(a.camera.take());
    drop(a.decoder.take());
    let started = chrono_now() - a.view.since.elapsed().as_secs() as i64;
    log_call(state, &a.view.session, &a.store, &a.view.peer, &a.view.call_id, a.view.video, a.view.outgoing, outcome, started);
    publish(state);
}

fn chrono_now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// The call's line in the chat: a message from the caller whose media is
/// `{"k": "call", ...}`, so it sorts, previews and syncs like any other.
#[allow(clippy::too_many_arguments)]
fn log_call(state: &Updater, session: &str, store: &Mutex<Store>, peer: &str, call_id: &str, video: bool, outgoing: bool, outcome: Outcome, ts: i64) {
    let seconds = if let Outcome::Ended(s) = outcome { s } else { 0 };
    // A call we placed and talked in is "outgoing"; the rest keep their outcome.
    let key = if outgoing && matches!(outcome, Outcome::Ended(_)) { "outgoing" } else { outcome.key() };
    let label = call_label(key, video, seconds);
    let (sender, delivery) = if outgoing { ("", Delivery::Sent) } else { (peer, Delivery::Received) };
    let record = store::msg_record(peer, &format!("call-{call_id}"), outgoing, sender, &label, ts, &delivery, false);
    let media = whatsapp_rust::serde_json::json!({ "k": "call", "out": key, "secs": seconds, "video": video, "mine": outgoing });
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
        ("canceled", _) => tr("[Canceled call]", "[Panggilan dibatalkan]").to_owned(),
        ("unanswered", _) => tr("[Unanswered call]", "[Panggilan tidak dijawab]").to_owned(),
        ("outgoing", true) => crate::trf!("[Outgoing video call · {duration}]", "[Panggilan video keluar · {duration}]"),
        ("outgoing", false) => crate::trf!("[Outgoing voice call · {duration}]", "[Panggilan suara keluar · {duration}]"),
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
    fn outgoing_calls_have_their_own_lines() {
        assert_eq!(call_label("outgoing", false, 156), "[Outgoing voice call · 2:36]");
        assert_eq!(call_label("outgoing", true, 5), "[Outgoing video call · 0:05]");
        assert_eq!(call_label("canceled", false, 0), "[Canceled call]");
        assert_eq!(call_label("unanswered", true, 0), "[Unanswered call]");
    }

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
