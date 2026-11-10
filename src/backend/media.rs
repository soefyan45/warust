//! Media in chats. Files the user sends: photos go out as images (JPEG with a
//! thumbnail), MP4/MOV as playable videos, everything else as a document. Received
//! media: the inline thumbnail is kept in `data/media/<session>/` and the CDN
//! fields are stored with the message so the file can be downloaded on demand.

use std::path::{Path, PathBuf};

use whatsapp_rust::download::{DownloadParams, MediaType};
use whatsapp_rust::serde_json::{Value, json};
use whatsapp_rust::media::{DocumentOptions, ImageOptions, VideoOptions, document_message, image_message, video_message};
use whatsapp_rust::prelude::*;
use whatsapp_rust::upload::UploadOptions;

/// Files are read into memory before upload; anything larger is refused.
pub const MAX_BYTES: u64 = 64 * 1024 * 1024;
/// Received files larger than this are not downloaded.
const MAX_DOWNLOAD: u64 = 100 * 1024 * 1024;
const THUMBNAIL_SIDE: u32 = 96;

fn extension(path: &Path) -> String {
    path.extension().and_then(|e| e.to_str()).unwrap_or_default().to_ascii_lowercase()
}

pub fn is_picture(path: &Path) -> bool {
    matches!(extension(path).as_str(), "jpg" | "jpeg" | "png")
}

/// Videos WhatsApp plays inline. Other formats (mkv, webm, avi, ...) go out
/// as documents.
pub fn is_video(path: &Path) -> bool {
    matches!(extension(path).as_str(), "mp4" | "m4v" | "mov")
}

pub fn file_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "file".to_owned())
}

/// What the chat shows for a sent file, the same way `session::body` labels
/// received media.
pub fn label(path: &Path, caption: &str) -> String {
    let tail = if caption.is_empty() { String::new() } else { format!(" {caption}") };
    if is_picture(path) {
        format!("[Gambar]{tail}")
    } else if is_video(path) {
        format!("[Video]{tail}")
    } else {
        format!("[Dokumen] {}{tail}", file_name(path))
    }
}

fn document_mime(path: &Path) -> &'static str {
    match extension(path).as_str() {
        "pdf" => "application/pdf",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xls" => "application/vnd.ms-excel",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "ppt" => "application/vnd.ms-powerpoint",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "txt" => "text/plain",
        "csv" => "text/csv",
        "zip" => "application/zip",
        "rar" => "application/vnd.rar",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "mp4" => "video/mp4",
        "mp3" => "audio/mpeg",
        "ogg" => "audio/ogg",
        "apk" => "application/vnd.android.package-archive",
        _ => "application/octet-stream",
    }
}

/// What a video message carries besides the file: read from the MP4 header.
#[derive(Debug, PartialEq)]
struct VideoInfo {
    seconds: u32,
    width: u32,
    height: u32,
}

/// The boxes directly inside `data`, as (type, payload).
fn mp4_boxes(mut data: &[u8]) -> impl Iterator<Item = (&[u8], &[u8])> {
    std::iter::from_fn(move || {
        let size = u32::from_be_bytes(data.get(..4)?.try_into().ok()?) as u64;
        let kind = data.get(4..8)?;
        let (header, size) = match size {
            0 => (8, data.len() as u64),
            1 => (16, u64::from_be_bytes(data.get(8..16)?.try_into().ok()?)),
            n => (8, n),
        };
        let size = usize::try_from(size).ok().filter(|&s| s >= header && s <= data.len())?;
        let payload = &data[header..size];
        data = &data[size..];
        Some((kind, payload))
    })
}

fn mp4_child<'a>(data: &'a [u8], kind: &[u8]) -> Option<&'a [u8]> {
    mp4_boxes(data).find(|(k, _)| *k == kind).map(|(_, payload)| payload)
}

fn be_u32(data: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(data.get(at..at + 4)?.try_into().ok()?))
}

/// Duration and display size of an MP4/MOV file, or `None` when it has no
/// movie header or no video track.
fn video_info(file: &[u8]) -> Option<VideoInfo> {
    let moov = mp4_child(file, b"moov")?;
    let mvhd = mp4_child(moov, b"mvhd")?;
    let (timescale, duration) = if *mvhd.first()? == 1 {
        (be_u32(mvhd, 20)?, u64::from_be_bytes(mvhd.get(24..32)?.try_into().ok()?))
    } else {
        (be_u32(mvhd, 12)?, u64::from(be_u32(mvhd, 16)?))
    };
    let seconds = u32::try_from(duration.div_ceil(u64::from(timescale.max(1)))).unwrap_or(u32::MAX);
    let tkhd = mp4_boxes(moov).filter(|(k, _)| *k == b"trak").find_map(|(_, trak)| {
        let hdlr = mp4_child(mp4_child(trak, b"mdia")?, b"hdlr")?;
        (hdlr.get(8..12)? == b"vide").then_some(())?;
        mp4_child(trak, b"tkhd")
    })?;
    // The matrix and size sit after the version-dependent time fields.
    let at = if *tkhd.first()? == 1 { 52 } else { 40 };
    let rotated = be_u32(tkhd, at)? == 0;
    let (width, height) = (be_u32(tkhd, at + 36)? >> 16, be_u32(tkhd, at + 40)? >> 16);
    let (width, height) = if rotated { (height, width) } else { (width, height) };
    Some(VideoInfo { seconds, width, height })
}

/// Whether the file starts with an ISO media `ftyp` box (MP4, MOV, M4V).
fn is_mp4(bytes: &[u8]) -> bool {
    matches!(mp4_boxes(bytes).next(), Some((kind, _)) if kind == b"ftyp")
}

/// How long ffmpeg may take for one thumbnail.
const FFMPEG_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// A still from the video, as a small JPEG: ffmpeg when installed, else
/// Quick Look on macOS. `None` without either; the video still sends.
/// ffmpeg is held to the MOV/MP4 demuxer and local files, with a timeout.
fn video_thumbnail(path: &Path) -> Option<Vec<u8>> {
    use crate::platform::command;
    // `file:` so a name with a colon is not read as another protocol.
    let mut input = std::ffi::OsString::from("file:");
    input.push(path.as_os_str());
    let mut ffmpeg = command("ffmpeg");
    ffmpeg
        .args(["-nostdin", "-v", "error", "-protocol_whitelist", "file", "-f", "mov", "-i"])
        .arg(&input)
        .args(["-vf", "thumbnail", "-frames:v", "1", "-f", "image2pipe", "-c:v", "png", "pipe:1"]);
    let still = output_within(ffmpeg, FFMPEG_TIMEOUT).filter(|out| !out.is_empty()).or_else(|| quick_look(path))?;
    let decoded = image::load_from_memory(&still).ok()?;
    encode_jpeg(&decoded.thumbnail(THUMBNAIL_SIDE, THUMBNAIL_SIDE), 60).ok()
}

/// Runs `command` and returns its stdout if it exits successfully within
/// `limit`; a command that takes longer is killed.
fn output_within(mut command: std::process::Command, limit: std::time::Duration) -> Option<Vec<u8>> {
    use std::io::Read;
    use std::process::Stdio;
    let mut child = command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().ok()?;
    let mut stdout = child.stdout.take()?;
    // Read on a thread, so a full pipe cannot stall the process.
    let reader = std::thread::spawn(move || {
        let mut out = Vec::new();
        stdout.read_to_end(&mut out).map(|_| out)
    });
    let started = std::time::Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if started.elapsed() < limit => std::thread::sleep(std::time::Duration::from_millis(50)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };
    let out = reader.join().ok()?.ok()?;
    status.filter(|s| s.success()).map(|_| out)
}

#[cfg(target_os = "macos")]
fn quick_look(path: &Path) -> Option<Vec<u8>> {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let out = std::env::temp_dir().join(format!("warust-thumb-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&out).ok()?;
    let made = crate::platform::command("qlmanage").args(["-t", "-s", "320", "-o"]).arg(&out).arg(path).output();
    let png = made.ok().and_then(|_| std::fs::read(out.join(format!("{}.png", file_name(path)))).ok());
    let _ = std::fs::remove_dir_all(&out);
    png
}

#[cfg(not(target_os = "macos"))]
fn quick_look(_: &Path) -> Option<Vec<u8>> {
    None
}

fn encode_jpeg(image: &image::DynamicImage, quality: u8) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, quality)
        .encode_image(&image.to_rgb8())
        .map_err(|e| e.to_string())?;
    Ok(bytes)
}

/// A file read and ready to upload, by how it goes out.
enum Prepared {
    Picture { jpeg: Vec<u8>, thumbnail: Option<Vec<u8>>, width: u32, height: u32 },
    Video { bytes: Vec<u8>, thumbnail: Option<Vec<u8>>, info: Option<VideoInfo> },
    Document(Vec<u8>),
}

fn prepare(path: &Path) -> Result<Prepared, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("File tidak terbaca: {e}"))?;
    if is_video(path) {
        let info = video_info(&bytes);
        // Only a real MP4/MOV goes to ffmpeg: a playlist named .mp4 (HLS,
        // concat) could make it read other files or URLs into the thumbnail.
        let thumbnail = (is_mp4(&bytes) && info.is_some()).then(|| video_thumbnail(path)).flatten();
        return Ok(Prepared::Video { thumbnail, info, bytes });
    }
    if !is_picture(path) {
        return Ok(Prepared::Document(bytes));
    }
    let decoded = image::load_from_memory(&bytes).map_err(|e| format!("Gambar rusak: {e}"))?;
    let jpeg = if matches!(extension(path).as_str(), "jpg" | "jpeg") {
        bytes
    } else {
        encode_jpeg(&decoded, 88)?
    };
    let thumbnail = encode_jpeg(&decoded.thumbnail(THUMBNAIL_SIDE, THUMBNAIL_SIDE), 60).ok();
    Ok(Prepared::Picture { jpeg, thumbnail, width: decoded.width(), height: decoded.height() })
}

/// Reads, uploads and wraps the file as a message ready for `send_message`.
pub async fn message(client: &Client, path: &Path, caption: &str) -> Result<wa::Message, String> {
    let size = std::fs::metadata(path).map_err(|e| format!("File tidak terbaca: {e}"))?.len();
    if size > MAX_BYTES {
        return Err(format!("File lebih dari {} MB", MAX_BYTES / 1024 / 1024));
    }
    let owned = path.to_owned();
    // Reading, re-encoding and taking a still can take a moment: keep it off the runtime.
    let prepared = tokio::task::spawn_blocking(move || prepare(&owned)).await.map_err(|e| e.to_string())??;

    let caption = (!caption.is_empty()).then(|| caption.to_owned());
    let upload = |bytes, kind| async move {
        client.upload(bytes, kind, UploadOptions::default()).await.map_err(|e| format!("Upload gagal: {e}"))
    };
    match prepared {
        Prepared::Picture { jpeg, thumbnail, width, height } => {
            let mut options = ImageOptions::default().with_mimetype("image/jpeg");
            options.jpeg_thumbnail = thumbnail;
            options.caption = caption;
            let mut message = image_message(upload(jpeg, MediaType::Image).await?, options);
            if let Some(image) = message.image_message.as_option_mut() {
                image.width = Some(width);
                image.height = Some(height);
            }
            Ok(message)
        }
        Prepared::Video { bytes, thumbnail, info } => {
            let mut options = VideoOptions::default().with_mimetype("video/mp4");
            options.jpeg_thumbnail = thumbnail;
            options.caption = caption;
            options.duration_seconds = info.as_ref().map(|i| i.seconds);
            let mut message = video_message(upload(bytes, MediaType::Video).await?, options);
            if let (Some(video), Some(info)) = (message.video_message.as_option_mut(), info) {
                video.width = Some(info.width);
                video.height = Some(info.height);
            }
            Ok(message)
        }
        Prepared::Document(bytes) => {
            let name = file_name(path);
            let mut options = DocumentOptions::default()
                .with_mimetype(document_mime(path))
                .with_file_name(name.clone())
                .with_title(name);
            options.caption = caption;
            Ok(document_message(upload(bytes, MediaType::Document).await?, options))
        }
    }
}

pub fn dir(data_dir: &Path, session: &str) -> PathBuf {
    data_dir.join("media").join(session)
}

fn stem(msg_id: &str) -> String {
    msg_id.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(text: &str) -> Option<Vec<u8>> {
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(text.get(i..i + 2)?, 16).ok())
        .collect()
}

/// WhatsApp stickers are 512×512 WebP.
const STICKER_SIDE: u32 = 512;

fn is_animated_webp(bytes: &[u8]) -> bool {
    image::codecs::webp::WebPDecoder::new(std::io::Cursor::new(bytes)).is_ok_and(|d| d.has_animation())
}

/// Turns any picture into a sticker file in `dir`: fitted into 512×512 on a
/// transparent canvas, saved as lossless WebP. Animated WebP is copied as is.
pub fn make_sticker(source: &Path, dir: &Path) -> Result<PathBuf, String> {
    let bytes = std::fs::read(source).map_err(|e| format!("File tidak terbaca: {e}"))?;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let stem: String = source
        .file_stem()
        .map(|s| s.to_string_lossy().chars().map(|c| if c.is_alphanumeric() || c == '-' { c } else { '_' }).collect())
        .unwrap_or_else(|| "stiker".to_owned());
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let target = dir.join(format!("{stem}-{nanos}.webp"));
    if is_animated_webp(&bytes) {
        std::fs::write(&target, &bytes).map_err(|e| e.to_string())?;
        return Ok(target);
    }
    let decoded = image::load_from_memory(&bytes).map_err(|e| format!("Gambar tidak didukung: {e}"))?;
    let fitted = decoded.resize(STICKER_SIDE, STICKER_SIDE, image::imageops::FilterType::Lanczos3).to_rgba8();
    let mut canvas = image::RgbaImage::new(STICKER_SIDE, STICKER_SIDE);
    let x = (STICKER_SIDE - fitted.width()) / 2;
    let y = (STICKER_SIDE - fitted.height()) / 2;
    image::imageops::overlay(&mut canvas, &fitted, x as i64, y as i64);
    let mut out = Vec::new();
    image::codecs::webp::WebPEncoder::new_lossless(&mut out)
        .encode(&canvas, STICKER_SIDE, STICKER_SIDE, image::ExtendedColorType::Rgba8)
        .map_err(|e| e.to_string())?;
    std::fs::write(&target, out).map_err(|e| e.to_string())?;
    Ok(target)
}

/// Uploads a sticker file and builds the message. Files not already 512×512
/// WebP are converted first.
pub async fn sticker_message(client: &Client, path: &Path) -> Result<wa::Message, String> {
    let owned = path.to_owned();
    let (bytes, animated, width, height) = tokio::task::spawn_blocking(move || -> Result<_, String> {
        let mut bytes = std::fs::read(&owned).map_err(|e| format!("File tidak terbaca: {e}"))?;
        let animated = is_animated_webp(&bytes);
        let (mut width, mut height) =
            image::ImageReader::new(std::io::Cursor::new(&bytes)).with_guessed_format().ok()
                .and_then(|r| r.into_dimensions().ok())
                .ok_or("Gambar tidak didukung")?;
        let webp = matches!(extension(&owned).as_str(), "webp");
        if !animated && !(webp && width == STICKER_SIDE && height == STICKER_SIDE) {
            let tmp = std::env::temp_dir().join(format!("warust-sticker-{}", std::process::id()));
            let made = make_sticker(&owned, &tmp)?;
            bytes = std::fs::read(&made).map_err(|e| e.to_string())?;
            let _ = std::fs::remove_file(&made);
            (width, height) = (STICKER_SIDE, STICKER_SIDE);
        }
        Ok((bytes, animated, width, height))
    })
    .await
    .map_err(|e| e.to_string())??;
    let upload = client
        .upload(bytes, MediaType::Sticker, UploadOptions::default())
        .await
        .map_err(|e| format!("Upload gagal: {e}"))?;
    let mut sticker = wa::message::StickerMessage::default();
    sticker.url = Some(upload.url);
    sticker.direct_path = Some(upload.direct_path);
    sticker.media_key = Some(upload.media_key.to_vec());
    sticker.file_enc_sha256 = Some(upload.file_enc_sha256.to_vec());
    sticker.file_sha256 = Some(upload.file_sha256.to_vec());
    sticker.file_length = Some(upload.file_length);
    sticker.mimetype = Some("image/webp".to_owned());
    sticker.media_key_timestamp = Some(upload.media_key_timestamp);
    sticker.is_animated = Some(animated);
    sticker.width = Some(width);
    sticker.height = Some(height);
    let mut message = wa::Message::default();
    message.sticker_message = MessageField::some(sticker);
    Ok(message)
}

/// Fields shared by every media message, read out of the proto.
struct Found {
    kind: &'static str,
    direct_path: Option<String>,
    media_key: Option<Vec<u8>>,
    enc: Option<Vec<u8>>,
    sha: Option<Vec<u8>>,
    size: Option<u64>,
    mime: Option<String>,
    thumbnail: Option<Vec<u8>>,
    width: Option<u32>,
    height: Option<u32>,
    name: Option<String>,
}

macro_rules! found {
    ($m:expr, $kind:literal) => {
        Found {
            kind: $kind,
            direct_path: $m.direct_path.clone(),
            media_key: $m.media_key.clone(),
            enc: $m.file_enc_sha256.clone(),
            sha: $m.file_sha256.clone(),
            size: $m.file_length,
            mime: $m.mimetype.clone(),
            thumbnail: None,
            width: None,
            height: None,
            name: None,
        }
    };
}

/// The media part of a received message for its chat record, or `None` for
/// plain text. Writes the inline thumbnail (when there is one) to `dir`.
pub fn describe(message: &wa::Message, dir: &Path, msg_id: &str) -> Option<Value> {
    let base = message.get_base_message();
    let found = if let Some(m) = base.image_message.as_option() {
        Found { thumbnail: m.jpeg_thumbnail.clone(), width: m.width, height: m.height, ..found!(m, "image") }
    } else if let Some(m) = base.video_message.as_option() {
        Found { thumbnail: m.jpeg_thumbnail.clone(), width: m.width, height: m.height, ..found!(m, "video") }
    } else if let Some(m) = base.sticker_message.as_option() {
        Found { width: m.width, height: m.height, ..found!(m, "sticker") }
    } else if let Some(m) = base.document_message.as_option() {
        Found { thumbnail: m.jpeg_thumbnail.clone(), name: m.file_name.clone(), ..found!(m, "document") }
    } else if let Some(m) = base.audio_message.as_option() {
        found!(m, "audio")
    } else {
        return None;
    };
    let thumb = found.thumbnail.as_ref().filter(|t| !t.is_empty()).and_then(|bytes| {
        let path = dir.join(format!("{}.thumb.jpg", stem(msg_id)));
        if !path.exists() {
            std::fs::create_dir_all(dir).ok()?;
            std::fs::write(&path, bytes).ok()?;
        }
        Some(path.to_string_lossy().into_owned())
    });
    let download = match (&found.direct_path, &found.media_key, &found.enc, &found.sha) {
        (Some(path), Some(key), Some(enc), Some(sha)) => {
            json!({ "p": path, "key": hex(key), "enc": hex(enc), "sha": hex(sha) })
        }
        _ => Value::Null,
    };
    Some(json!({
        "k": found.kind,
        "mime": found.mime,
        "size": found.size,
        "w": found.width,
        "h": found.height,
        "name": found.name,
        "thumb": thumb,
        "dl": download,
    }))
}

/// The media part of a sent sticker's chat record.
pub fn describe_sent_sticker(path: &Path) -> Value {
    json!({ "k": "sticker", "file": path.to_string_lossy() })
}

/// The media part of a sent file's chat record: it shows from the original file.
pub fn describe_sent(path: &Path) -> Value {
    let picture = is_picture(path);
    let size = std::fs::metadata(path).map(|m| m.len()).ok();
    let (width, height) = if picture {
        image::image_dimensions(path).map(|(w, h)| (Some(w), Some(h))).unwrap_or((None, None))
    } else {
        (None, None)
    };
    json!({
        "k": if picture { "image" } else if is_video(path) { "video" } else { "document" },
        "size": size,
        "w": width,
        "h": height,
        "name": file_name(path),
        "file": path.to_string_lossy(),
    })
}

fn media_type(kind: &str) -> MediaType {
    match kind {
        "image" => MediaType::Image,
        "video" => MediaType::Video,
        "sticker" => MediaType::Sticker,
        "audio" => MediaType::Audio,
        _ => MediaType::Document,
    }
}

/// Where a downloaded file goes: its own name for documents, else by kind.
fn target(dir: &Path, msg_id: &str, kind: &str, name: Option<&str>) -> PathBuf {
    let tail = match (kind, name) {
        (_, Some(name)) if !name.is_empty() => {
            let safe: String = name.chars().map(|c| if c == '/' || c == '\\' { '_' } else { c }).collect();
            format!("-{safe}")
        }
        ("image", _) => ".jpg".to_owned(),
        ("video", _) => ".mp4".to_owned(),
        ("sticker", _) => ".webp".to_owned(),
        ("audio", _) => ".ogg".to_owned(),
        _ => ".bin".to_owned(),
    };
    dir.join(format!("{}{tail}", stem(msg_id)))
}

/// Downloads and decrypts a received file into `dir`, using the fields that
/// `describe` stored.
pub async fn download(client: &Client, media: &Value, dir: &Path, msg_id: &str) -> Result<PathBuf, String> {
    let text = |key: &str| media.get(key).and_then(Value::as_str);
    let kind = text("k").unwrap_or("document");
    let size = media.get("size").and_then(Value::as_u64).unwrap_or(0);
    if size > MAX_DOWNLOAD {
        return Err(format!("File lebih dari {} MB", MAX_DOWNLOAD / 1024 / 1024));
    }
    let dl = media.get("dl").filter(|d| !d.is_null()).ok_or("Data unduhan tidak ada")?;
    let field = |key: &str| dl.get(key).and_then(Value::as_str).and_then(unhex).ok_or("Data unduhan rusak");
    let direct_path = dl.get("p").and_then(Value::as_str).ok_or("Data unduhan rusak")?;
    let params = DownloadParams::encrypted(
        direct_path,
        &field("key")?,
        &field("sha")?,
        &field("enc")?,
        size,
        media_type(kind),
    )
    .map_err(|e| format!("Data unduhan rusak: {e:?}"))?;
    let bytes = client
        .download(&params, Default::default())
        .await
        .map_err(|e| format!("Unduh gagal: {e:?}"))?;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let path = target(dir, msg_id, kind, text("name"));
    std::fs::write(&path, bytes).map_err(|e| format!("Gagal menyimpan: {e}"))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_match_received_media() {
        assert_eq!(label(Path::new("/a/Foto.PNG"), ""), "[Gambar]");
        assert_eq!(label(Path::new("/a/b.jpg"), "hai"), "[Gambar] hai");
        assert_eq!(label(Path::new("/a/laporan.pdf"), ""), "[Dokumen] laporan.pdf");
        assert_eq!(document_mime(Path::new("x.XLSX")).ends_with("sheet"), true);
        assert_eq!(label(Path::new("/a/klip.MP4"), ""), "[Video]");
        assert_eq!(label(Path::new("/a/klip.mov"), "lihat"), "[Video] lihat");
        assert_eq!(label(Path::new("/a/klip.mkv"), ""), "[Dokumen] klip.mkv");
    }

    fn mp4_box(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut out = ((payload.len() + 8) as u32).to_be_bytes().to_vec();
        out.extend_from_slice(kind);
        out.extend_from_slice(payload);
        out
    }

    /// A version-0 `tkhd` payload with the given matrix a/b and size.
    fn tkhd(a: i32, b: i32, width: u32, height: u32) -> Vec<u8> {
        let mut p = vec![0u8; 84];
        p[40..44].copy_from_slice(&a.to_be_bytes());
        p[44..48].copy_from_slice(&b.to_be_bytes());
        p[76..80].copy_from_slice(&(width << 16).to_be_bytes());
        p[80..84].copy_from_slice(&(height << 16).to_be_bytes());
        p
    }

    fn track(handler: &[u8; 4], tkhd_payload: Vec<u8>) -> Vec<u8> {
        let mut hdlr = vec![0u8; 8];
        hdlr.extend_from_slice(handler);
        hdlr.extend_from_slice(&[0u8; 13]);
        let mdia = mp4_box(b"mdia", &mp4_box(b"hdlr", &hdlr));
        mp4_box(b"trak", &[mp4_box(b"tkhd", &tkhd_payload), mdia].concat())
    }

    fn movie(mvhd: Vec<u8>, tracks: &[Vec<u8>]) -> Vec<u8> {
        let moov = mp4_box(b"moov", &[mp4_box(b"mvhd", &mvhd), tracks.concat()].concat());
        [mp4_box(b"ftyp", b"isom\0\0\0\0"), mp4_box(b"mdat", &[1, 2, 3]), moov].concat()
    }

    #[test]
    fn reads_duration_and_size_of_the_video_track() {
        // Version 0: timescale 1000, duration 12.4 s (rounds up to 13).
        let mut mvhd = vec![0u8; 20];
        mvhd[12..16].copy_from_slice(&1000u32.to_be_bytes());
        mvhd[16..20].copy_from_slice(&12_400u32.to_be_bytes());
        let one = 1 << 16;
        let file = movie(mvhd, &[track(b"soun", tkhd(one, 0, 0, 0)), track(b"vide", tkhd(one, 0, 1280, 720))]);
        assert_eq!(video_info(&file), Some(VideoInfo { seconds: 13, width: 1280, height: 720 }));
    }

    #[test]
    fn rotated_video_swaps_width_and_height() {
        // Version 1 mvhd (64-bit times), and a phone video turned 90°.
        let mut mvhd = vec![0u8; 32];
        mvhd[0] = 1;
        mvhd[20..24].copy_from_slice(&600u32.to_be_bytes());
        mvhd[24..32].copy_from_slice(&3_000u64.to_be_bytes());
        let file = movie(mvhd, &[track(b"vide", tkhd(0, 1 << 16, 1920, 1080))]);
        assert_eq!(video_info(&file), Some(VideoInfo { seconds: 5, width: 1080, height: 1920 }));
    }

    /// Reads a real MP4 made by ffmpeg (skipped when it is not installed).
    #[test]
    fn real_video_has_info_and_thumbnail() {
        let dir = std::env::temp_dir().join(format!("warust-video-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let clip = dir.join("klip.mp4");
        let made = std::process::Command::new("ffmpeg")
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc=duration=2:size=320x240:rate=10", "-pix_fmt", "yuv420p"])
            .arg(&clip)
            .status();
        if !made.is_ok_and(|s| s.success()) {
            return;
        }
        let bytes = std::fs::read(&clip).unwrap();
        assert_eq!(video_info(&bytes), Some(VideoInfo { seconds: 2, width: 320, height: 240 }));
        let thumbnail = image::load_from_memory(&video_thumbnail(&clip).unwrap()).unwrap();
        assert_eq!((thumbnail.width(), thumbnail.height()), (THUMBNAIL_SIDE, 72));
        assert_eq!(describe_sent(&clip)["k"], "video");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn not_a_movie() {
        assert_eq!(video_info(b"plain text, not an mp4"), None);
        assert_eq!(video_info(&mp4_box(b"ftyp", b"isom")), None);
        // Playlists named .mp4 never reach ffmpeg.
        assert!(is_mp4(&mp4_box(b"ftyp", b"isom")));
        assert!(!is_mp4(b"#EXTM3U\n#EXT-X-MEDIA-SEQUENCE:0\nfile:///etc/passwd\n"));
        assert!(!is_mp4(b"ffconcat version 1.0\nfile /etc/passwd\n"));
    }

    #[test]
    fn pictures_become_512_webp_stickers() {
        let dir = std::env::temp_dir().join(format!("warust-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("wide.png");
        image::RgbaImage::from_pixel(300, 100, image::Rgba([200, 40, 40, 255])).save(&source).unwrap();
        let sticker = make_sticker(&source, &dir.join("out")).unwrap();
        let bytes = std::fs::read(&sticker).unwrap();
        let decoded = image::load_from_memory(&bytes).unwrap().to_rgba8();
        assert_eq!(decoded.dimensions(), (512, 512));
        // Letterboxed: transparent above, the picture in the middle.
        assert_eq!(decoded.get_pixel(256, 10)[3], 0);
        assert_eq!(decoded.get_pixel(256, 256)[3], 255);
        assert!(!is_animated_webp(&bytes));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn hex_round_trip_and_targets() {
        assert_eq!(unhex(&hex(&[0, 15, 255])), Some(vec![0, 15, 255]));
        assert_eq!(unhex("zz"), None);
        let dir = Path::new("/m");
        assert_eq!(target(dir, "AB:1", "image", None), Path::new("/m/AB_1.jpg"));
        assert_eq!(target(dir, "X", "document", Some("a/b.pdf")), Path::new("/m/X-a_b.pdf"));
    }
}
