//! Media in chats. Files the user sends: photos go out as images (JPEG with a
//! thumbnail), everything else as a document. Received
//! media: the inline thumbnail is kept in `data/media/<session>/` and the CDN
//! fields are stored with the message so the file can be downloaded on demand.

use std::path::{Path, PathBuf};

use whatsapp_rust::download::{DownloadParams, MediaType};
use whatsapp_rust::serde_json::{Value, json};
use whatsapp_rust::media::{DocumentOptions, ImageOptions, document_message, image_message};
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

pub fn file_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "file".to_owned())
}

/// What the chat shows for a sent file, the same way `session::body` labels
/// received media.
pub fn label(path: &Path, caption: &str) -> String {
    let tail = if caption.is_empty() { String::new() } else { format!(" {caption}") };
    if is_picture(path) {
        format!("[Gambar]{tail}")
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

fn encode_jpeg(image: &image::DynamicImage, quality: u8) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, quality)
        .encode_image(&image.to_rgb8())
        .map_err(|e| e.to_string())?;
    Ok(bytes)
}

/// Reads, uploads and wraps the file as a message ready for `send_message`.
pub async fn message(client: &Client, path: &Path, caption: &str) -> Result<wa::Message, String> {
    let size = std::fs::metadata(path).map_err(|e| format!("File tidak terbaca: {e}"))?.len();
    if size > MAX_BYTES {
        return Err(format!("File lebih dari {} MB", MAX_BYTES / 1024 / 1024));
    }
    let picture = is_picture(path);
    let owned = path.to_owned();
    // Reading and re-encoding can take a moment: keep it off the runtime.
    let prepared = tokio::task::spawn_blocking(move || -> Result<_, String> {
        let bytes = std::fs::read(&owned).map_err(|e| format!("File tidak terbaca: {e}"))?;
        if !picture {
            return Ok((bytes, None));
        }
        let decoded = image::load_from_memory(&bytes).map_err(|e| format!("Gambar rusak: {e}"))?;
        let jpeg = if matches!(extension(&owned).as_str(), "jpg" | "jpeg") {
            bytes
        } else {
            encode_jpeg(&decoded, 88)?
        };
        let thumbnail = encode_jpeg(&decoded.thumbnail(THUMBNAIL_SIDE, THUMBNAIL_SIDE), 60).ok();
        Ok((jpeg, Some((thumbnail, decoded.width(), decoded.height()))))
    })
    .await
    .map_err(|e| e.to_string())??;

    let caption = (!caption.is_empty()).then(|| caption.to_owned());
    match prepared {
        (jpeg, Some((thumbnail, width, height))) => {
            let upload = client
                .upload(jpeg, MediaType::Image, UploadOptions::default())
                .await
                .map_err(|e| format!("Upload gagal: {e}"))?;
            let mut options = ImageOptions::default().with_mimetype("image/jpeg");
            options.jpeg_thumbnail = thumbnail;
            options.caption = caption;
            let mut message = image_message(upload, options);
            if let Some(image) = message.image_message.as_option_mut() {
                image.width = Some(width);
                image.height = Some(height);
            }
            Ok(message)
        }
        (bytes, None) => {
            let upload = client
                .upload(bytes, MediaType::Document, UploadOptions::default())
                .await
                .map_err(|e| format!("Upload gagal: {e}"))?;
            let name = file_name(path);
            let mut options = DocumentOptions::default()
                .with_mimetype(document_mime(path))
                .with_file_name(name.clone())
                .with_title(name);
            options.caption = caption;
            Ok(document_message(upload, options))
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
        "k": if picture { "image" } else { "document" },
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
