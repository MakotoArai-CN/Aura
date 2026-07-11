use crate::proxy;
use base64::Engine as _;
use lofty::config::WriteOptions;
use lofty::file::{AudioFile, TaggedFileExt};
use lofty::picture::{MimeType, Picture, PictureType};
use lofty::prelude::Accessor;
use lofty::tag::{ItemKey, Tag};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize)]
pub struct DownloadResult {
    pub path: String,
    pub file_name: String,
    pub metadata_written: bool,
    pub metadata_error: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct DownloadMetadata {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub lyrics: Option<String>,
    pub cover_url: Option<String>,
}

#[derive(Debug, Serialize)]
struct DownloadSidecar {
    title: Option<String>,
    artist: Option<String>,
    album: Option<String>,
    lyrics: Option<String>,
    cover_url: Option<String>,
    cover_data_url: Option<String>,
}

struct DownloadedCover {
    bytes: Vec<u8>,
    mime: MimeType,
    data_url: String,
}

fn default_music_dir_path() -> PathBuf {
    #[cfg(windows)]
    {
        if let Ok(home) = std::env::var("USERPROFILE") {
            return PathBuf::from(home).join("Music");
        }
    }

    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home).join("Music");
    }

    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

fn decode_once(value: &str) -> String {
    urlencoding::decode(value)
        .map(|decoded| decoded.to_string())
        .unwrap_or_else(|_| value.to_string())
}

fn normalize_download_url(url: &str) -> String {
    let mut normalized = url.trim().to_string();
    if normalized.contains("%3A") || normalized.contains("%3a") {
        let decoded = decode_once(&normalized);
        if decoded.contains("://") {
            normalized = decoded;
        }
    }

    let encoded_target = if let Some((_, rest)) = normalized.split_once("/stream/") {
        Some(rest.split('?').next().unwrap_or(rest))
    } else if let Some(rest) = normalized.strip_prefix("http://stream.localhost/") {
        Some(rest)
    } else {
        normalized.strip_prefix("stream://localhost/")
    };

    if let Some(encoded) = encoded_target {
        return decode_once(encoded);
    }

    if normalized.starts_with("file://")
        && normalized
            .get("file://".len()..)
            .is_some_and(|rest| rest.len() > 2 && rest.as_bytes()[1] == b':')
    {
        return normalized
            .replacen("file://", "file:///", 1)
            .replace('\\', "/");
    }

    normalized.replace('\\', "/")
}

fn sanitize_file_name(value: &str) -> String {
    let cleaned: String = value
        .chars()
        .map(|ch| match ch {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            ch if ch.is_control() => '_',
            ch => ch,
        })
        .collect();

    let trimmed = cleaned.trim().trim_matches('.').trim().to_string();
    if trimmed.is_empty() {
        "Listen1 Track".to_string()
    } else {
        trimmed
    }
}

fn extension_from_url(url: &str) -> Option<String> {
    let path = url.split('?').next().unwrap_or(url);
    let segment = path.rsplit('/').next().unwrap_or(path);
    Path::new(segment)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .filter(|ext| ext.len() <= 8)
}

fn extension_from_content_type(content_type: &str) -> Option<&'static str> {
    let value = content_type.split(';').next().unwrap_or("").trim();
    match value {
        "audio/flac" | "audio/x-flac" => Some("flac"),
        "audio/mpeg" | "audio/mp3" => Some("mp3"),
        "audio/mp4" | "audio/x-m4a" => Some("m4a"),
        "audio/aac" => Some("aac"),
        "audio/ogg" | "application/ogg" => Some("ogg"),
        "audio/opus" => Some("opus"),
        "audio/wav" | "audio/x-wav" => Some("wav"),
        "audio/aiff" | "audio/x-aiff" => Some("aiff"),
        "audio/webm" => Some("webm"),
        _ => None,
    }
}

fn choose_download_extension(url_ext: Option<String>, type_ext: Option<String>) -> Option<String> {
    match (url_ext.as_deref(), type_ext.as_deref()) {
        (Some("m4s"), _) => Some("m4a".into()),
        (Some("mp4"), Some("m4a")) => Some("m4a".into()),
        (Some("aac"), Some("m4a")) => Some("m4a".into()),
        (Some(ext), _) => Some(ext.to_string()),
        (None, Some(ext)) => Some(ext.to_string()),
        (None, None) => None,
    }
}

fn infer_picture_mime(bytes: &[u8], content_type: Option<&str>) -> Option<MimeType> {
    if let Some(content_type) = content_type {
        let value = content_type.split(';').next().unwrap_or("").trim();
        if value.starts_with("image/") {
            return Some(MimeType::from_str(value));
        }
    }
    if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        Some(MimeType::Png)
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some(MimeType::Jpeg)
    } else if bytes.starts_with(b"GIF8") {
        Some(MimeType::Gif)
    } else if bytes.starts_with(b"BM") {
        Some(MimeType::Bmp)
    } else {
        None
    }
}

fn clean_meta(value: Option<String>) -> Option<String> {
    value.map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

async fn fetch_cover(url: &str) -> Result<Option<(Vec<u8>, MimeType)>, String> {
    if url.trim().is_empty() {
        return Ok(None);
    }
    if url.starts_with("data:") {
        let Some((header, body)) = url.split_once(',') else {
            return Ok(None);
        };
        let mime = header
            .strip_prefix("data:")
            .and_then(|value| value.split(';').next())
            .map(MimeType::from_str)
            .unwrap_or_else(|| MimeType::Unknown("application/octet-stream".into()));
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(body)
            .map_err(|e| e.to_string())?;
        return Ok(Some((bytes, mime)));
    }

    let client = proxy::build_client()?;
    let mut headers = HashMap::new();
    proxy::inject_headers(url, &mut headers);
    let mut request = client.get(url);
    for (name, value) in &headers {
        request = request.header(name.as_str(), value.as_str());
    }
    let response = request.send().await.map_err(|e| e.to_string())?;
    if !response.status().is_success() {
        return Ok(None);
    }
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let bytes = response.bytes().await.map_err(|e| e.to_string())?.to_vec();
    let Some(mime) = infer_picture_mime(&bytes, content_type.as_deref()) else {
        return Ok(None);
    };
    Ok(Some((bytes, mime)))
}

fn sidecar_path(path: &Path) -> PathBuf {
    let mut path = path.to_path_buf();
    path.set_extension("listen1.json");
    path
}

fn mime_to_string(mime: &MimeType) -> String {
    mime.as_str().to_string()
}

fn cover_data_url(bytes: &[u8], mime: &MimeType) -> String {
    format!(
        "data:{};base64,{}",
        mime_to_string(mime),
        base64::engine::general_purpose::STANDARD.encode(bytes)
    )
}

async fn fetch_download_cover(cover_url: Option<&str>) -> Result<Option<DownloadedCover>, String> {
    let Some(url) = cover_url else {
        return Ok(None);
    };
    let Some((bytes, mime)) = fetch_cover(url).await? else {
        return Ok(None);
    };
    let data_url = cover_data_url(&bytes, &mime);
    Ok(Some(DownloadedCover {
        bytes,
        mime,
        data_url,
    }))
}

fn write_sidecar_metadata(
    path: &Path,
    title: Option<String>,
    artist: Option<String>,
    album: Option<String>,
    lyrics: Option<String>,
    cover_url: Option<String>,
    cover_data_url: Option<String>,
) -> Result<bool, String> {
    if title.is_none()
        && artist.is_none()
        && album.is_none()
        && lyrics.is_none()
        && cover_url.is_none()
        && cover_data_url.is_none()
    {
        return Ok(false);
    }

    if let Some(lyrics) = lyrics.as_deref().filter(|value| !value.trim().is_empty()) {
        std::fs::write(path.with_extension("lrc"), lyrics).map_err(|e| e.to_string())?;
    }

    let sidecar = DownloadSidecar {
        title,
        artist,
        album,
        lyrics,
        cover_url,
        cover_data_url,
    };
    let text = serde_json::to_string_pretty(&sidecar).map_err(|e| e.to_string())?;
    std::fs::write(sidecar_path(path), text).map_err(|e| e.to_string())?;
    Ok(true)
}

async fn write_download_metadata(path: &Path, metadata: Option<DownloadMetadata>) -> Result<bool, String> {
    let Some(metadata) = metadata else {
        return Ok(false);
    };
    let title = clean_meta(metadata.title);
    let artist = clean_meta(metadata.artist);
    let album = clean_meta(metadata.album);
    let lyrics = clean_meta(metadata.lyrics);
    let cover_url = clean_meta(metadata.cover_url);
    if title.is_none() && artist.is_none() && album.is_none() && lyrics.is_none() && cover_url.is_none() {
        return Ok(false);
    }

    let cover = fetch_download_cover(cover_url.as_deref()).await?;
    let sidecar_written = write_sidecar_metadata(
        path,
        title.clone(),
        artist.clone(),
        album.clone(),
        lyrics.clone(),
        cover_url.clone(),
        cover.as_ref().map(|cover| cover.data_url.clone()),
    )?;

    let tag_result = (|| -> Result<(), String> {
        let mut tagged_file = lofty::read_from_path(path).map_err(|e| e.to_string())?;
        let tag_type = tagged_file.primary_tag_type();
        if tagged_file.tag_mut(tag_type).is_none() {
            tagged_file.insert_tag(Tag::new(tag_type));
        }
        let tag = tagged_file
            .tag_mut(tag_type)
            .ok_or_else(|| "failed to create metadata tag".to_string())?;

        if let Some(value) = title {
            tag.set_title(value);
        }
        if let Some(value) = artist {
            tag.set_artist(value);
        }
        if let Some(value) = album {
            tag.set_album(value);
        }
        if let Some(value) = lyrics {
            tag.insert_text(ItemKey::Lyrics, value);
        }
        if let Some(cover) = cover {
            tag.remove_picture_type(PictureType::CoverFront);
            tag.push_picture(Picture::new_unchecked(
                PictureType::CoverFront,
                Some(cover.mime),
                None,
                cover.bytes,
            ));
        }

        tagged_file
            .save_to_path(path, WriteOptions::default())
            .map_err(|e| e.to_string())?;
        Ok(())
    })();

    match tag_result {
        Ok(()) => Ok(true),
        Err(error) if sidecar_written => {
            #[cfg(debug_assertions)]
            eprintln!("[listen1] embedded metadata write failed, sidecar kept: {error}");
            Ok(true)
        }
        Err(error) => Err(error),
    }
}

fn with_extension(file_name: &str, ext: Option<&str>) -> String {
    if Path::new(file_name).extension().is_some() {
        return file_name.to_string();
    }

    match ext {
        Some(ext) if !ext.is_empty() => format!("{file_name}.{ext}"),
        _ => format!("{file_name}.mp3"),
    }
}

fn unique_path(dir: &Path, file_name: &str) -> PathBuf {
    let initial = dir.join(file_name);
    if !initial.exists() {
        return initial;
    }

    let path = Path::new(file_name);
    let stem = path
        .file_stem()
        .and_then(|v| v.to_str())
        .unwrap_or("Listen1 Track");
    let ext = path.extension().and_then(|v| v.to_str());

    for i in 1..1000 {
        let candidate = match ext {
            Some(ext) => dir.join(format!("{stem} ({i}).{ext}")),
            None => dir.join(format!("{stem} ({i})")),
        };
        if !candidate.exists() {
            return candidate;
        }
    }

    dir.join(file_name)
}

#[tauri::command]
pub fn default_music_dir() -> String {
    default_music_dir_path().to_string_lossy().to_string()
}

#[tauri::command]
pub async fn download_track(
    url: String,
    file_name: String,
    directory: Option<String>,
    metadata: Option<DownloadMetadata>,
) -> Result<DownloadResult, String> {
    let normalized_url = normalize_download_url(&url);
    let target_dir = directory
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(default_music_dir_path);

    std::fs::create_dir_all(&target_dir).map_err(|e| e.to_string())?;

    if normalized_url.starts_with("file://") {
        let source = proxy::file_url_to_path(&normalized_url);
        let ext = source
            .extension()
            .and_then(|value| value.to_str())
            .map(|value| value.to_ascii_lowercase());
        let safe_name = with_extension(&sanitize_file_name(&file_name), ext.as_deref());
        let target = unique_path(&target_dir, &safe_name);
        std::fs::copy(&source, &target).map_err(|e| e.to_string())?;
        let metadata_result = write_download_metadata(&target, metadata).await;
        let (metadata_written, metadata_error) = match metadata_result {
            Ok(written) => (written, None),
            Err(error) => (false, Some(error)),
        };
        return Ok(DownloadResult {
            path: target.to_string_lossy().to_string(),
            file_name: target
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or(&safe_name)
                .to_string(),
            metadata_written,
            metadata_error,
        });
    }

    let client = proxy::build_client()?;
    let mut headers = HashMap::new();
    proxy::inject_headers(&normalized_url, &mut headers);

    let mut request = client.get(&normalized_url);
    for (name, value) in &headers {
        request = request.header(name.as_str(), value.as_str());
    }

    let response = request.send().await.map_err(|e| e.to_string())?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("download failed: HTTP {}", status.as_u16()));
    }

    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
        .unwrap_or_default();
    let bytes = response.bytes().await.map_err(|e| e.to_string())?;

    let url_ext = extension_from_url(&normalized_url);
    let type_ext = extension_from_content_type(&content_type).map(str::to_string);
    let ext = choose_download_extension(url_ext, type_ext);
    let safe_name = with_extension(&sanitize_file_name(&file_name), ext.as_deref());
    let target = unique_path(&target_dir, &safe_name);

    std::fs::write(&target, bytes).map_err(|e| e.to_string())?;
    let metadata_result = write_download_metadata(&target, metadata).await;
    let (metadata_written, metadata_error) = match metadata_result {
        Ok(written) => (written, None),
        Err(error) => (false, Some(error)),
    };

    Ok(DownloadResult {
        path: target.to_string_lossy().to_string(),
        file_name: target
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or(&safe_name)
            .to_string(),
        metadata_written,
        metadata_error,
    })
}
