use base64::{engine::general_purpose, Engine as _};
use lofty::prelude::{Accessor, AudioFile, TaggedFileExt};
use lofty::tag::{ItemKey, Tag};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize, Deserialize)]
pub struct AudioMeta {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub lyrics: Option<String>,
    pub cover: Option<String>,
    pub duration: Option<f64>,
    pub bitrate: Option<f64>,
}

#[derive(Debug, Default, Deserialize)]
struct SidecarMeta {
    title: Option<String>,
    artist: Option<String>,
    album: Option<String>,
    lyrics: Option<String>,
    cover_url: Option<String>,
    cover_data_url: Option<String>,
}

fn clean_text(value: Option<String>) -> Option<String> {
    value
        .map(|v| v.trim_matches('\u{feff}').trim().to_string())
        .filter(|v| !v.is_empty())
}

fn first_tag_text<F>(tags: &[&Tag], reader: F) -> Option<String>
where
    F: Fn(&Tag) -> Option<String>,
{
    tags.iter().find_map(|tag| clean_text(reader(tag)))
}

fn infer_mime_type(bytes: &[u8]) -> &'static str {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        "image/png"
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        "image/jpeg"
    } else if bytes.starts_with(b"GIF8") {
        "image/gif"
    } else if bytes.starts_with(b"BM") {
        "image/bmp"
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        "image/webp"
    } else {
        "application/octet-stream"
    }
}

fn picture_data_url(tags: &[&Tag]) -> Option<String> {
    let picture = tags
        .iter()
        .find_map(|tag| tag.get_picture_type(lofty::picture::PictureType::CoverFront))
        .or_else(|| tags.iter().find_map(|tag| tag.pictures().first()))?;

    let data = picture.data();
    if data.is_empty() {
        return None;
    }

    let mime = picture
        .mime_type()
        .map(|mime| mime.as_str())
        .unwrap_or_else(|| infer_mime_type(data));
    Some(format!(
        "data:{};base64,{}",
        mime,
        general_purpose::STANDARD.encode(data)
    ))
}

fn lyrics_text(tags: &[&Tag]) -> Option<String> {
    tags.iter()
        .flat_map(|tag| tag.get_strings(&ItemKey::Lyrics))
        .map(str::trim)
        .find(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn decode_text_bytes(bytes: &[u8]) -> String {
    let (utf8_text, _, had_errors) = encoding_rs::UTF_8.decode(bytes);
    if !had_errors {
        return utf8_text.into_owned();
    }

    let (gb_text, _, _) = encoding_rs::GB18030.decode(bytes);
    gb_text.into_owned()
}

fn sidecar_lyrics(path: &str) -> Option<String> {
    let lrc_path = Path::new(path).with_extension("lrc");
    let bytes = fs::read(lrc_path).ok()?;
    clean_text(Some(decode_text_bytes(&bytes)))
}

fn sidecar_path(path: &str) -> PathBuf {
    let mut sidecar = PathBuf::from(path);
    sidecar.set_extension("listen1.json");
    sidecar
}

fn read_sidecar(path: &str) -> SidecarMeta {
    let Ok(text) = fs::read_to_string(sidecar_path(path)) else {
        return SidecarMeta::default();
    };
    serde_json::from_str(&text).unwrap_or_default()
}

fn filename_title(path: &str) -> Option<String> {
    Path::new(path)
        .file_stem()
        .and_then(|s| s.to_str())
        .map(|s| s.to_string())
}

fn sidecar_audio_meta(path: &str, sidecar: SidecarMeta) -> AudioMeta {
    AudioMeta {
        title: clean_text(sidecar.title).or_else(|| filename_title(path)),
        artist: clean_text(sidecar.artist),
        album: clean_text(sidecar.album),
        lyrics: sidecar_lyrics(path).or_else(|| clean_text(sidecar.lyrics)),
        cover: clean_text(sidecar.cover_data_url).or_else(|| clean_text(sidecar.cover_url)),
        duration: None,
        bitrate: None,
    }
}

fn is_audio_file(path: &Path) -> bool {
    let Some(ext) = path.extension().and_then(|value| value.to_str()) else {
        return false;
    };
    matches!(
        ext.to_ascii_lowercase().as_str(),
        "mp3" | "flac" | "ogg" | "oga" | "opus" | "wav" | "aif" | "aiff" | "m4a" | "mp4" | "aac" | "webm"
    )
}

fn scan_dir_recursive(dir: PathBuf, output: &mut Vec<String>, limit: usize) -> Result<(), String> {
    if output.len() >= limit {
        return Ok(());
    }
    let entries = match fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(_) => return Ok(()),
    };

    for entry in entries {
        if output.len() >= limit {
            break;
        }
        let Ok(entry) = entry else {
            continue;
        };
        let path = entry.path();
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if meta.is_dir() {
            let name = path.file_name().and_then(|value| value.to_str()).unwrap_or_default();
            if name.starts_with('.') {
                continue;
            }
            scan_dir_recursive(path, output, limit)?;
        } else if meta.is_file() && is_audio_file(&path) {
            output.push(path.to_string_lossy().to_string());
        }
    }
    Ok(())
}

#[tauri::command]
pub fn read_audio_tags(path: String) -> Result<AudioMeta, String> {
    let sidecar = read_sidecar(&path);
    let tagged_file = match lofty::read_from_path(&path) {
        Ok(file) => file,
        Err(error) => {
            if sidecar.title.is_some()
                || sidecar.artist.is_some()
                || sidecar.album.is_some()
                || sidecar.lyrics.is_some()
                || sidecar.cover_url.is_some()
                || sidecar.cover_data_url.is_some()
                || sidecar_lyrics(&path).is_some()
            {
                return Ok(sidecar_audio_meta(&path, sidecar));
            }
            return Err(error.to_string());
        }
    };
    let mut tags = Vec::new();
    if let Some(tag) = tagged_file.primary_tag() {
        tags.push(tag);
    }
    for tag in tagged_file.tags() {
        if !tags.iter().any(|existing| std::ptr::eq(*existing, tag)) {
            tags.push(tag);
        }
    }

    let duration = tagged_file.properties().duration().as_secs_f64();
    let bitrate = fs::metadata(&path)
        .ok()
        .and_then(|meta| {
            (duration > 0.0).then_some((meta.len() as f64 * 8.0) / duration / 1000.0)
        });

    Ok(AudioMeta {
        title: first_tag_text(&tags, |tag| tag.title().map(|v| v.into_owned()))
            .or_else(|| clean_text(sidecar.title))
            .or_else(|| filename_title(&path)),
        artist: first_tag_text(&tags, |tag| tag.artist().map(|v| v.into_owned()))
            .or_else(|| clean_text(sidecar.artist)),
        album: first_tag_text(&tags, |tag| tag.album().map(|v| v.into_owned()))
            .or_else(|| clean_text(sidecar.album)),
        lyrics: lyrics_text(&tags)
            .or_else(|| sidecar_lyrics(&path))
            .or_else(|| clean_text(sidecar.lyrics)),
        cover: picture_data_url(&tags)
            .or_else(|| clean_text(sidecar.cover_data_url))
            .or_else(|| clean_text(sidecar.cover_url)),
        duration: Some(duration),
        bitrate,
    })
}

#[tauri::command]
pub fn scan_music_directory(directory: String) -> Result<Vec<String>, String> {
    let root = PathBuf::from(directory);
    if !root.exists() {
        return Err("music directory does not exist".to_string());
    }
    if !root.is_dir() {
        return Err("music directory is not a folder".to_string());
    }

    let mut files = Vec::new();
    scan_dir_recursive(root, &mut files, 20_000)?;
    files.sort();
    files.dedup();
    Ok(files)
}
