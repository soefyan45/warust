//! Voice notes: records the microphone and encodes it the way WhatsApp's own
//! voice notes are, Opus in Ogg at 48 kHz mono, with the 64-bar waveform the
//! bubble draws. Audio is encoded while it is recorded, so memory stays at
//! the size of the file (about 4 KB a second) and finishing is quick.

use crate::i18n::tr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

/// Opus works at 48 kHz; voice notes are mono.
pub const RATE: u32 = 48_000;
/// 20 ms per Opus packet.
const FRAME: usize = 960;
/// Plenty for speech; WhatsApp's own notes are about this size.
const BITRATE: i32 = 32_000;
/// Bars in the waveform WhatsApp shows on a voice note.
const BARS: usize = 64;
/// Any id works: the file holds one stream.
const SERIAL: u32 = 0x7761_7275;
/// Longest voice note; the composer stops and sends there.
pub const MAX_SECONDS: u64 = 15 * 60;
/// Shorter recordings are a slip of the finger, not a message.
const MIN_SAMPLES: u64 = RATE as u64 / 2;
/// How long the microphone may take to open.
const OPEN_TIMEOUT: Duration = Duration::from_secs(10);

/// One channel from interleaved samples (their average).
pub fn downmix(interleaved: &[f32], channels: usize) -> Vec<f32> {
    let channels = channels.max(1);
    if channels == 1 {
        return interleaved.to_vec();
    }
    interleaved.chunks(channels).map(|f| f.iter().sum::<f32>() / f.len() as f32).collect()
}

/// Mono audio at the device's rate to 48 kHz, chunk by chunk. Output sample
/// `k` is read at input position `k × rate / 48000`, counted exactly, so the
/// result does not depend on how the device splits its audio. Linear
/// interpolation is enough for speech.
struct Resampler {
    rate: u64,
    /// Input samples seen so far.
    seen: u64,
    /// Output samples made so far.
    made: u64,
    /// The input sample just before the current chunk.
    last: f32,
}

impl Resampler {
    fn new(rate: u32) -> Self {
        Self { rate: u64::from(rate.max(1)), seen: 0, made: 0, last: 0.0 }
    }

    /// Where output sample `k` falls: the input index and the fraction past it.
    fn position(&self, k: u64) -> (u64, f32) {
        let at = k * self.rate;
        (at / u64::from(RATE), (at % u64::from(RATE)) as f32 / RATE as f32)
    }

    fn push(&mut self, input: &[f32], out: &mut Vec<f32>) {
        if self.rate == u64::from(RATE) {
            out.extend_from_slice(input);
            return;
        }
        let first = self.seen;
        let end = first + input.len() as u64;
        // Index `first - 1` is `last`; the loop never reaches further back.
        let sample = |i: u64| if i < first { self.last } else { input[(i - first) as usize] };
        loop {
            let (i, part) = self.position(self.made);
            if i + 1 >= end {
                break;
            }
            let (a, b) = (sample(i), sample(i + 1));
            out.push(a + (b - a) * part);
            self.made += 1;
        }
        self.seen = end;
        if let Some(&last) = input.last() {
            self.last = last;
        }
    }

    /// The samples still owed at the end, held at the last value.
    fn flush(&mut self, out: &mut Vec<f32>) {
        if self.rate == u64::from(RATE) {
            return;
        }
        while self.position(self.made).0 < self.seen {
            out.push(self.last);
            self.made += 1;
        }
    }
}

/// Mean loudness of a stretch of audio.
fn level(samples: &[f32]) -> f32 {
    samples.iter().map(|s| s.abs()).sum::<f32>() / samples.len().max(1) as f32
}

/// Loudness per 20 ms of 48 kHz audio.
fn frame_levels(pcm: &[f32]) -> Vec<f32> {
    pcm.chunks(FRAME).map(level).collect()
}

/// 64 bars from 0 to 100, relative to the loudest, from per-frame levels.
fn bars(levels: &[f32]) -> Vec<u8> {
    let loudest = levels.iter().copied().fold(0.0f32, f32::max);
    if levels.is_empty() || loudest < 1e-4 {
        return vec![0; BARS];
    }
    (0..BARS)
        .map(|bar| {
            let from = bar * levels.len() / BARS;
            let to = ((bar + 1) * levels.len() / BARS).max(from + 1).min(levels.len());
            let mean = levels[from..to].iter().sum::<f32>() / (to - from) as f32;
            (mean / loudest * 100.0).round().clamp(0.0, 100.0) as u8
        })
        .collect()
}

/// The waveform of 48 kHz mono samples.
pub fn waveform(pcm: &[f32]) -> Vec<u8> {
    bars(&frame_levels(pcm))
}

/// Whole seconds, rounded up, of a count of 48 kHz samples.
fn seconds_of(samples: u64) -> u32 {
    u32::try_from(samples.div_ceil(u64::from(RATE))).unwrap_or(u32::MAX)
}

/// Refuses recordings with nothing to send. macOS hands a blocked microphone
/// over as silence rather than an error, hence the hint.
fn check(samples: u64, levels: &[f32]) -> Result<(), String> {
    if samples < MIN_SAMPLES {
        return Err(tr("Recording is too short", "Rekaman terlalu pendek").to_owned());
    }
    if levels.iter().all(|&l| l < 1e-5) {
        return Err(tr(
            "No sound from the microphone. Check the microphone permission for warust in the system settings.",
            "Tidak ada suara dari mikrofon. Periksa izin mikrofon untuk warust di pengaturan sistem.",
        )
        .to_owned());
    }
    Ok(())
}

/// Turns mono audio into an Ogg Opus file (RFC 7845) as it comes in.
pub struct Encoder {
    resampler: Resampler,
    opus: opus::Encoder,
    writer: ogg::PacketWriter<'static, Vec<u8>>,
    /// Priming samples the decoder drops (the encoder's lookahead).
    pre_skip: u64,
    /// 48 kHz audio not yet a whole frame.
    pending: Vec<f32>,
    /// 48 kHz audio samples taken in so far.
    samples: u64,
    packets: u64,
    levels: Vec<f32>,
}

impl Encoder {
    /// For mono audio at `rate`.
    pub fn new(rate: u32) -> Result<Self, String> {
        let mut opus = opus::Encoder::new(RATE, opus::Channels::Mono, opus::Application::Voip).map_err(|e| e.to_string())?;
        opus.set_bitrate(opus::Bitrate::Bits(BITRATE)).map_err(|e| e.to_string())?;
        let pre_skip = u64::try_from(opus.get_lookahead().map_err(|e| e.to_string())?.max(0)).unwrap_or(0);

        let mut head = b"OpusHead".to_vec();
        head.push(1); // version
        head.push(1); // channels
        head.extend_from_slice(&(pre_skip as u16).to_le_bytes());
        head.extend_from_slice(&RATE.to_le_bytes());
        head.extend_from_slice(&0i16.to_le_bytes()); // output gain
        head.push(0); // channel mapping family
        let vendor = b"warust";
        let mut tags = b"OpusTags".to_vec();
        tags.extend_from_slice(&(vendor.len() as u32).to_le_bytes());
        tags.extend_from_slice(vendor);
        tags.extend_from_slice(&0u32.to_le_bytes()); // no comments

        let mut writer = ogg::PacketWriter::new(Vec::new());
        writer.write_packet(head, SERIAL, ogg::PacketWriteEndInfo::EndPage, 0).map_err(|e| e.to_string())?;
        writer.write_packet(tags, SERIAL, ogg::PacketWriteEndInfo::EndPage, 0).map_err(|e| e.to_string())?;
        Ok(Self { resampler: Resampler::new(rate), opus, writer, pre_skip, pending: Vec::new(), samples: 0, packets: 0, levels: Vec::new() })
    }

    fn write(&mut self, frame: &[f32], end: ogg::PacketWriteEndInfo, granule: u64) -> Result<(), String> {
        let packet = self.opus.encode_vec_float(frame, 4_000).map_err(|e| e.to_string())?;
        self.writer.write_packet(packet, SERIAL, end, granule).map_err(|e| e.to_string())?;
        self.packets += 1;
        Ok(())
    }

    /// Takes in more mono audio; whole 20 ms frames are encoded right away.
    /// Audio past `MAX_SECONDS` is dropped.
    pub fn push(&mut self, mono: &[f32]) -> Result<(), String> {
        let before = self.pending.len();
        self.resampler.push(mono, &mut self.pending);
        self.take(before)
    }

    /// Counts the samples added to `pending` after `before`, within the limit,
    /// and encodes the whole frames.
    fn take(&mut self, before: usize) -> Result<(), String> {
        let room = usize::try_from((MAX_SECONDS * u64::from(RATE)).saturating_sub(self.samples)).unwrap_or(usize::MAX);
        let added = (self.pending.len() - before).min(room);
        self.pending.truncate(before + added);
        self.samples += added as u64;
        while self.pending.len() >= FRAME {
            let frame: Vec<f32> = self.pending.drain(..FRAME).collect();
            self.levels.push(level(&frame));
            let granule = (self.packets + 1) * FRAME as u64;
            self.write(&frame, ogg::PacketWriteEndInfo::NormalPacket, granule)?;
        }
        Ok(())
    }

    /// Ends the stream: the last audio, the decoder's priming made up with
    /// silence, and the end-of-stream mark. Gives the Ogg Opus file.
    pub fn finish(mut self) -> Result<Vec<u8>, String> {
        let before = self.pending.len();
        self.resampler.flush(&mut self.pending);
        self.take(before)?;
        if !self.pending.is_empty() {
            self.levels.push(level(&self.pending));
        }
        check(self.samples, &self.levels)?;
        // The last granule is where the audio ends, counting the priming.
        let total = self.pre_skip + self.samples;
        let frames = total.div_ceil(FRAME as u64).max(self.packets + 1);
        let mut tail = std::mem::take(&mut self.pending);
        tail.resize(((frames - self.packets) as usize) * FRAME, 0.0);
        let count = tail.len() / FRAME;
        for (i, frame) in tail.chunks(FRAME).enumerate() {
            let last = i + 1 == count;
            let granule = ((self.packets + 1) * FRAME as u64).min(total);
            let end = if last { ogg::PacketWriteEndInfo::EndStream } else { ogg::PacketWriteEndInfo::NormalPacket };
            self.write(frame, end, granule)?;
        }
        Ok(self.writer.into_inner())
    }
}

/// Length in whole seconds and the waveform of an Ogg Opus voice note, read
/// back from the file so a queued note can be sent again from its path.
pub fn inspect(ogg: &[u8]) -> Result<(u32, Vec<u8>), String> {
    let mut reader = ogg::PacketReader::new(std::io::Cursor::new(ogg));
    let head = reader.read_packet_expected().map_err(|e| crate::trf!("Not a voice message: {e}", "Bukan pesan suara: {e}"))?;
    if !head.data.starts_with(b"OpusHead") || head.data.len() < 19 {
        return Err(tr("Not an Opus voice message", "Bukan pesan suara Opus").to_owned());
    }
    let pre_skip = usize::from(u16::from_le_bytes([head.data[10], head.data[11]]));
    reader.read_packet_expected().map_err(|e| crate::trf!("Broken voice message: {e}", "Pesan suara rusak: {e}"))?; // OpusTags
    let mut decoder = opus::Decoder::new(RATE, opus::Channels::Mono).map_err(|e| e.to_string())?;
    let (mut pcm, mut end) = (Vec::new(), 0u64);
    let mut out = vec![0.0f32; 5_760];
    while let Some(packet) = reader.read_packet().map_err(|e| crate::trf!("Broken voice message: {e}", "Pesan suara rusak: {e}"))? {
        let n = decoder.decode_float(&packet.data, &mut out, false).map_err(|e| e.to_string())?;
        pcm.extend_from_slice(&out[..n]);
        end = packet.absgp_page();
    }
    // The last granule marks where the audio ends; the first `pre_skip` samples are priming.
    let end = usize::try_from(end).unwrap_or(usize::MAX).min(pcm.len());
    let audio = pcm.get(pre_skip.min(end)..end).unwrap_or_default();
    Ok((seconds_of(audio.len() as u64), waveform(audio)))
}

/// Where a recording stands.
pub enum Mic {
    /// The microphone is still opening.
    Opening,
    Recording,
    Failed(String),
}

enum Message {
    Audio(Vec<f32>),
    Finish(PathBuf),
    Cancel,
}

/// A recording in progress. The microphone and the encoder run on their own
/// thread, so nothing here waits on them. Dropping it cancels the recording.
pub struct Recorder {
    to: mpsc::Sender<Message>,
    opened: mpsc::Receiver<Result<(), String>>,
    saved: Option<mpsc::Receiver<Result<PathBuf, String>>>,
    level: Arc<AtomicU32>,
    asked: Instant,
    started: Option<Instant>,
    failed: Option<String>,
}

impl Recorder {
    /// Starts opening the default microphone; `poll` tells when it records.
    pub fn start() -> Self {
        let (to, from) = mpsc::channel::<Message>();
        let (ready, opened) = mpsc::channel();
        let (done, saved) = mpsc::channel();
        let level = Arc::new(AtomicU32::new(0));
        let (audio, peak) = (to.clone(), level.clone());
        let spawned = std::thread::Builder::new().name("mic".to_owned()).spawn(move || record(audio, peak, from, ready, done));
        let failed = spawned.err().map(|e| crate::trf!("The microphone cannot be used: {e}", "Mikrofon tidak bisa dipakai: {e}"));
        Self { to, opened, saved: Some(saved), level, asked: Instant::now(), started: None, failed }
    }

    pub fn poll(&mut self) -> Mic {
        if let Some(e) = &self.failed {
            return Mic::Failed(e.clone());
        }
        if self.started.is_some() {
            return Mic::Recording;
        }
        match self.opened.try_recv() {
            Ok(Ok(())) => {
                self.started = Some(Instant::now());
                return Mic::Recording;
            }
            Ok(Err(e)) => self.failed = Some(e),
            Err(mpsc::TryRecvError::Disconnected) => self.failed = Some(tr("The microphone stopped", "Mikrofon berhenti").to_owned()),
            Err(mpsc::TryRecvError::Empty) if self.asked.elapsed() > OPEN_TIMEOUT => {
                self.failed = Some(tr("The microphone is not responding", "Mikrofon tidak merespons").to_owned());
            }
            Err(mpsc::TryRecvError::Empty) => return Mic::Opening,
        }
        self.poll()
    }

    /// Time recorded (zero while the microphone opens).
    pub fn elapsed(&self) -> Duration {
        self.started.map(|t| t.elapsed()).unwrap_or_default()
    }

    /// Loudness of the latest audio, 0 to 1, for the recording meter.
    pub fn level(&self) -> f32 {
        f32::from_bits(self.level.load(Ordering::Relaxed)).clamp(0.0, 1.0)
    }

    /// Stops recording; the note is finished and written into `dir` on the
    /// recording thread. The receiver gets its path, or why there is none.
    pub fn finish(mut self, dir: PathBuf) -> mpsc::Receiver<Result<PathBuf, String>> {
        let _ = self.to.send(Message::Finish(dir));
        let (_, empty) = mpsc::channel();
        self.saved.take().unwrap_or(empty)
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        // After finish() the thread is already saving; otherwise this cancels.
        if self.saved.is_some() {
            let _ = self.to.send(Message::Cancel);
        }
    }
}

/// The recording thread: opens the microphone, encodes what it hears, and on
/// Finish writes the file.
fn record(
    audio: mpsc::Sender<Message>,
    level: Arc<AtomicU32>,
    from: mpsc::Receiver<Message>,
    ready: mpsc::Sender<Result<(), String>>,
    done: mpsc::Sender<Result<PathBuf, String>>,
) {
    let (stream, rate) = match open(audio, level) {
        Ok(opened) => opened,
        Err(e) => {
            let _ = ready.send(Err(e));
            return;
        }
    };
    let mut encoder = match Encoder::new(rate) {
        Ok(encoder) => encoder,
        Err(e) => {
            let _ = ready.send(Err(crate::trf!("Voice encoder failed: {e}", "Encoder suara gagal: {e}")));
            return;
        }
    };
    let _ = ready.send(Ok(()));
    let mut stream = Some(stream);
    let mut failure = None;
    while let Ok(message) = from.recv() {
        match message {
            Message::Audio(mono) => {
                if failure.is_none() {
                    failure = encoder.push(&mono).err();
                }
            }
            Message::Cancel => return,
            Message::Finish(dir) => {
                drop(stream.take());
                // What the microphone sent before it closed.
                while let Ok(Message::Audio(mono)) = from.try_recv() {
                    if failure.is_none() {
                        failure = encoder.push(&mono).err();
                    }
                }
                let saved = match failure {
                    Some(e) => Err(crate::trf!("Voice message processing failed: {e}", "Pesan suara gagal diproses: {e}")),
                    None => encoder.finish().and_then(|ogg| save(&dir, &ogg)),
                };
                // Nobody waits any more (its window closed): the file would only
                // be left behind.
                if let Err(mpsc::SendError(Ok(path))) = done.send(saved) {
                    let _ = std::fs::remove_file(path);
                }
                return;
            }
        }
    }
}

fn save(dir: &std::path::Path, ogg: &[u8]) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|e| crate::trf!("The voice message folder cannot be created: {e}", "Folder pesan suara tidak bisa dibuat: {e}"))?;
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let path = dir.join(format!("voice-{nanos}.ogg"));
    std::fs::write(&path, ogg).map_err(|e| crate::trf!("The voice message could not be saved: {e}", "Pesan suara tidak tersimpan: {e}"))?;
    Ok(path)
}

/// Opens the default microphone; its callback hands mono chunks to `audio`.
fn open(audio: mpsc::Sender<Message>, level: Arc<AtomicU32>) -> Result<(cpal::Stream, u32), String> {
    let device = cpal::default_host().default_input_device().ok_or(tr("No microphone", "Tidak ada mikrofon"))?;
    let config = device.default_input_config().map_err(|e| crate::trf!("The microphone cannot be used: {e}", "Mikrofon tidak bisa dipakai: {e}"))?;
    let rate = config.sample_rate();
    let stream = match config.sample_format() {
        cpal::SampleFormat::F32 => listen::<f32>(&device, config.into(), audio, level),
        cpal::SampleFormat::I16 => listen::<i16>(&device, config.into(), audio, level),
        cpal::SampleFormat::I32 => listen::<i32>(&device, config.into(), audio, level),
        cpal::SampleFormat::U16 => listen::<u16>(&device, config.into(), audio, level),
        other => return Err(crate::trf!("Microphone format {other} is not supported yet", "Format mikrofon {other} belum didukung")),
    }
    .map_err(|e| crate::trf!("The microphone cannot be opened: {e}", "Mikrofon tidak bisa dibuka: {e}"))?;
    stream.play().map_err(|e| crate::trf!("The microphone cannot start recording: {e}", "Mikrofon tidak bisa mulai merekam: {e}"))?;
    Ok((stream, rate))
}

fn listen<T>(device: &cpal::Device, config: cpal::StreamConfig, audio: mpsc::Sender<Message>, level: Arc<AtomicU32>) -> Result<cpal::Stream, cpal::Error>
where
    T: cpal::SizedSample,
    f32: cpal::FromSample<T>,
{
    let channels = usize::from(config.channels);
    device.build_input_stream(
        config,
        move |data: &[T], _: &cpal::InputCallbackInfo| {
            let samples: Vec<f32> = data.iter().map(|&s| <f32 as cpal::FromSample<T>>::from_sample_(s)).collect();
            let peak = samples.iter().fold(0.0f32, |m, s| m.max(s.abs()));
            level.store(peak.to_bits(), Ordering::Relaxed);
            let _ = audio.send(Message::Audio(downmix(&samples, channels)));
        },
        |e| log::warn!("microphone: {e}"),
        None,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(rate: u32, seconds: f32, channels: usize, loudness: f32) -> Vec<f32> {
        let frames = (rate as f32 * seconds) as usize;
        (0..frames)
            .flat_map(|i| {
                let sample = loudness * (i as f32 * 440.0 * std::f32::consts::TAU / rate as f32).sin();
                std::iter::repeat_n(sample, channels)
            })
            .collect()
    }

    /// A whole recording at 48 kHz mono, in one go.
    fn encode(pcm: &[f32]) -> Result<Vec<u8>, String> {
        let mut encoder = Encoder::new(RATE)?;
        encoder.push(pcm)?;
        encoder.finish()
    }

    fn resample(input: &[f32], rate: u32, chunk: usize) -> Vec<f32> {
        let mut resampler = Resampler::new(rate);
        let mut out = Vec::new();
        for part in input.chunks(chunk) {
            resampler.push(part, &mut out);
        }
        resampler.flush(&mut out);
        out
    }

    #[test]
    fn downmixes_and_resamples_to_48k_in_chunks() {
        let stereo = tone(44_100, 1.0, 2, 0.5);
        let mono = downmix(&stereo, 2);
        assert_eq!(mono.len(), 44_100);
        let whole = resample(&mono, 44_100, mono.len());
        assert!((whole.len() as i64 - 48_000).abs() <= 1, "{}", whole.len());
        assert!(whole.iter().all(|s| s.abs() <= 0.5 + 1e-3));
        // The same samples whatever size the device's chunks are.
        for chunk in [1, 7, 441, 512] {
            let parts = resample(&mono, 44_100, chunk);
            assert_eq!(parts.len(), whole.len(), "chunk {chunk}");
            assert!(parts.iter().zip(&whole).all(|(a, b)| (a - b).abs() < 1e-5), "chunk {chunk}");
        }
        assert_eq!(resample(&tone(48_000, 0.5, 1, 0.5), 48_000, 100).len(), 24_000);
    }

    #[test]
    fn waveform_has_64_bars_from_0_to_100() {
        let mut pcm = vec![0.0; 24_000];
        pcm.extend(tone(48_000, 0.5, 1, 0.9));
        let bars = waveform(&pcm);
        assert_eq!(bars.len(), 64);
        assert!(bars.iter().all(|&b| b <= 100));
        assert!(bars[..30].iter().all(|&b| b == 0));
        assert!(bars[40..].iter().all(|&b| b > 50));
        assert_eq!(waveform(&[]), vec![0; 64]);
    }

    #[test]
    fn duration_rounds_up_to_whole_seconds() {
        assert_eq!(seconds_of(48_000), 1);
        assert_eq!(seconds_of(48_001), 2);
        assert_eq!(seconds_of(0), 0);
    }

    #[test]
    fn refuses_short_or_silent_recordings() {
        assert_eq!(check(1_000, &[0.3]), Err("Recording is too short".to_owned()));
        assert!(check(48_000, &vec![0.0; 50]).unwrap_err().contains("microphone permission"));
        assert_eq!(check(48_000, &frame_levels(&tone(48_000, 1.0, 1, 0.2))), Ok(()));
        let mut encoder = Encoder::new(RATE).unwrap();
        encoder.push(&vec![0.0; 48_000]).unwrap();
        assert!(encoder.finish().is_err());
    }

    #[test]
    fn chunks_encode_the_same_file_as_one_go() {
        let pcm = tone(48_000, 1.3, 1, 0.5);
        let whole = encode(&pcm).unwrap();
        let mut encoder = Encoder::new(RATE).unwrap();
        for part in pcm.chunks(441) {
            encoder.push(part).unwrap();
        }
        let ogg = encoder.finish().unwrap();
        assert_eq!(ogg, whole);
        assert_eq!(inspect(&ogg).unwrap().0, 2);
    }

    #[test]
    fn encodes_ogg_opus_that_decodes_back() {
        let pcm = tone(48_000, 1.0, 1, 0.5);
        let ogg = encode(&pcm).unwrap();
        let mut reader = ogg::PacketReader::new(std::io::Cursor::new(&ogg));
        let head = reader.read_packet_expected().unwrap();
        assert_eq!(&head.data[..8], b"OpusHead");
        assert_eq!(head.data[9], 1, "mono");
        assert_eq!(u32::from_le_bytes(head.data[12..16].try_into().unwrap()), 48_000);
        let pre_skip = u16::from_le_bytes(head.data[10..12].try_into().unwrap()) as u64;
        let tags = reader.read_packet_expected().unwrap();
        assert_eq!(&tags.data[..8], b"OpusTags");

        let mut decoder = opus::Decoder::new(48_000, opus::Channels::Mono).unwrap();
        let (mut decoded, mut last) = (Vec::new(), None);
        while let Some(packet) = reader.read_packet().unwrap() {
            let mut out = vec![0.0f32; 5_760];
            let n = decoder.decode_float(&packet.data, &mut out, false).unwrap();
            decoded.extend_from_slice(&out[..n]);
            last = Some((packet.absgp_page(), packet.last_in_stream()));
        }
        assert_eq!(last, Some((pre_skip + 48_000, true)));
        assert!(decoded.len() as u64 >= pre_skip + 48_000);
        let energy: f32 = decoded.iter().map(|s| s * s).sum::<f32>() / decoded.len() as f32;
        assert!(energy > 0.05, "the tone survives encoding: {energy}");
    }

    #[test]
    fn inspect_reads_back_length_and_waveform() {
        let mut pcm = tone(48_000, 1.5, 1, 0.8);
        pcm.extend(vec![0.0; 72_000]);
        let (secs, bars) = inspect(&encode(&pcm).unwrap()).unwrap();
        assert_eq!(secs, 3);
        assert_eq!(bars.len(), 64);
        let expected = waveform(&pcm);
        let off = bars.iter().zip(&expected).filter(|(a, b)| a.abs_diff(**b) > 15).count();
        assert!(off <= 2, "{bars:?} vs {expected:?}");
        assert!(inspect(b"not ogg").is_err());
    }

    /// ffprobe reads the file as one second of mono Opus (skipped without it).
    #[test]
    fn ffprobe_accepts_the_file() {
        let path = std::env::temp_dir().join(format!("warust-voice-{}.ogg", std::process::id()));
        std::fs::write(&path, encode(&tone(48_000, 1.0, 1, 0.5)).unwrap()).unwrap();
        let probed = std::process::Command::new("ffprobe")
            .args(["-v", "error", "-show_entries", "stream=codec_name,channels,sample_rate:format=duration", "-of", "csv=p=0"])
            .arg(&path)
            .output();
        let _ = std::fs::remove_file(&path);
        let Ok(probed) = probed else { return };
        let text = String::from_utf8_lossy(&probed.stdout);
        assert!(probed.status.success(), "{}", String::from_utf8_lossy(&probed.stderr));
        assert!(text.contains("opus,48000,1"), "{text}");
        let duration: f32 = text.lines().last().unwrap().trim().parse().unwrap();
        assert!((duration - 1.0).abs() < 0.03, "{duration}");
    }
}
