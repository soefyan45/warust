//! Voice notes: records the microphone and encodes it the way WhatsApp's own
//! voice notes are, Opus in Ogg at 48 kHz mono, with the 64-bar waveform the
//! bubble draws.

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

/// Interleaved samples from the microphone, as mono at 48 kHz (linear
/// interpolation is enough for speech).
pub fn to_48k_mono(interleaved: &[f32], channels: usize, rate: u32) -> Vec<f32> {
    let channels = channels.max(1);
    let mono: Vec<f32> = interleaved.chunks(channels).map(|f| f.iter().sum::<f32>() / f.len() as f32).collect();
    if rate == RATE || mono.is_empty() {
        return mono;
    }
    let len = (mono.len() as u64 * u64::from(RATE)).div_ceil(u64::from(rate.max(1))) as usize;
    let step = f64::from(rate) / f64::from(RATE);
    (0..len)
        .map(|i| {
            let at = i as f64 * step;
            let (index, part) = (at as usize, (at.fract()) as f32);
            let a = mono[index.min(mono.len() - 1)];
            let b = mono[(index + 1).min(mono.len() - 1)];
            a + (b - a) * part
        })
        .collect()
}

/// Whole seconds, rounded up, of 48 kHz mono samples.
pub fn seconds(pcm: &[f32]) -> u32 {
    (pcm.len() as u64).div_ceil(u64::from(RATE)) as u32
}

/// Loudness in 64 bars from 0 to 100, relative to the loudest.
pub fn waveform(pcm: &[f32]) -> Vec<u8> {
    if pcm.is_empty() {
        return vec![0; BARS];
    }
    let levels: Vec<f32> = (0..BARS)
        .map(|bar| {
            let part = &pcm[bar * pcm.len() / BARS..((bar + 1) * pcm.len() / BARS).max(bar * pcm.len() / BARS + 1).min(pcm.len())];
            part.iter().map(|s| s.abs()).sum::<f32>() / part.len().max(1) as f32
        })
        .collect();
    let loudest = levels.iter().copied().fold(0.0f32, f32::max);
    if loudest < 1e-4 {
        return vec![0; BARS];
    }
    levels.iter().map(|level| (level / loudest * 100.0).round().clamp(0.0, 100.0) as u8).collect()
}

/// 48 kHz mono samples as an Ogg Opus file (RFC 7845).
pub fn encode(pcm: &[f32]) -> Result<Vec<u8>, String> {
    let mut encoder = opus::Encoder::new(RATE, opus::Channels::Mono, opus::Application::Voip).map_err(|e| e.to_string())?;
    encoder.set_bitrate(opus::Bitrate::Bits(BITRATE)).map_err(|e| e.to_string())?;
    let pre_skip = encoder.get_lookahead().map_err(|e| e.to_string())?.max(0) as usize;

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
    let write = |writer: &mut ogg::PacketWriter<Vec<u8>>, packet: Vec<u8>, end, granule| {
        writer.write_packet(packet, SERIAL, end, granule).map_err(|e| e.to_string())
    };
    write(&mut writer, head, ogg::PacketWriteEndInfo::EndPage, 0)?;
    write(&mut writer, tags, ogg::PacketWriteEndInfo::EndPage, 0)?;

    // The decoder drops `pre_skip` samples first, so encode that much more.
    let total = (pre_skip + pcm.len()) as u64;
    let mut padded = pcm.to_vec();
    padded.resize((pre_skip + pcm.len()).div_ceil(FRAME).max(1) * FRAME, 0.0);
    let frames = padded.len() / FRAME;
    for (i, frame) in padded.chunks(FRAME).enumerate() {
        let packet = encoder.encode_vec_float(frame, 4_000).map_err(|e| e.to_string())?;
        let last = i + 1 == frames;
        let granule = (((i + 1) * FRAME) as u64).min(total);
        let end = if last { ogg::PacketWriteEndInfo::EndStream } else { ogg::PacketWriteEndInfo::NormalPacket };
        write(&mut writer, packet, end, granule)?;
    }
    Ok(writer.into_inner())
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

    #[test]
    fn mixes_down_and_resamples_to_48k() {
        let stereo = tone(44_100, 1.0, 2, 0.5);
        let mono = to_48k_mono(&stereo, 2, 44_100);
        assert_eq!(mono.len(), 48_000);
        assert!(mono.iter().all(|s| s.abs() <= 0.5 + 1e-3));
        let same = to_48k_mono(&tone(48_000, 0.5, 1, 0.5), 1, 48_000);
        assert_eq!(same.len(), 24_000);
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
        assert_eq!(seconds(&vec![0.0; 48_000]), 1);
        assert_eq!(seconds(&vec![0.0; 48_001]), 2);
        assert_eq!(seconds(&[]), 0);
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
