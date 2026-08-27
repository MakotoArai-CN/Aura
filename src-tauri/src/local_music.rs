use lofty::prelude::{Accessor, AudioFile, TaggedFileExt};
use lofty::tag::{ItemKey, Tag};
use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize, Deserialize)]
pub struct AudioMeta {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub lyrics: Option<String>,
    /// 可直接当 `src` 用的封面地址（来自 sidecar 的 http / data URL）。
    pub cover: Option<String>,
    /// 内嵌封面落盘后的绝对路径。前端转成 `file://` 存起来，渲染时再经本机流服务器取。
    ///
    /// 之所以不像以前那样直接回 base64 data URL：那串东西前端要么塞进 localStorage
    /// 把配额撑爆，要么每次渲染都得重新读一遍文件。落盘之后存储里只剩一个短路径。
    pub cover_path: Option<String>,
    pub duration: Option<f64>,
    pub bitrate: Option<f64>,
    /// 标签是否真的读出来了。false = 文件打不开/格式不认，前端不能把这次当成
    /// 「已扫描完毕、这首歌就是没信息」缓存下来，否则永远不会再重试。
    pub tags_read: bool,
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

/// 内嵌封面的落盘目录，和音频缓存同级。
fn cover_cache_dir() -> PathBuf {
    crate::cache::user_home_dir()
        .join("Listen1")
        .join("Cache")
        .join("covers")
}

/// 缓存文件名。除路径外还把「文件长度 + 修改时间」揉进去：用户重新打过标签换了
/// 封面之后这两个值会变，于是自然算出新文件名，不会一直拿旧图。
fn cover_cache_key(path: &str) -> String {
    let mut hasher = DefaultHasher::new();
    path.hash(&mut hasher);
    if let Ok(meta) = fs::metadata(path) {
        meta.len().hash(&mut hasher);
        if let Ok(modified) = meta.modified() {
            if let Ok(since_epoch) = modified.duration_since(std::time::UNIX_EPOCH) {
                since_epoch.as_secs().hash(&mut hasher);
            }
        }
    }
    format!("{:016x}", hasher.finish())
}

fn cover_extension(mime: &str) -> &'static str {
    match mime.trim().to_ascii_lowercase().as_str() {
        "image/png" => "png",
        "image/webp" => "webp",
        "image/gif" => "gif",
        "image/bmp" | "image/x-ms-bmp" => "bmp",
        _ => "jpg",
    }
}

/// 把内嵌封面写进缓存目录，返回绝对路径。已经存在就直接复用，不重复写盘。
fn extract_cover_to_cache(path: &str, tags: &[&Tag]) -> Option<String> {
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
    let file = cover_cache_dir().join(format!(
        "{}.{}",
        cover_cache_key(path),
        cover_extension(mime)
    ));
    if file.is_file() {
        return Some(file.to_string_lossy().into_owned());
    }
    fs::create_dir_all(file.parent()?).ok()?;
    fs::write(&file, data).ok()?;
    Some(file.to_string_lossy().into_owned())
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
        cover_path: None,
        duration: None,
        bitrate: None,
        tags_read: false,
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
            // 读不出标签不算失败：至少还能给出文件名和 sidecar/.lrc 里的东西。
            // 返回 Err 的老做法让前端把这首歌记成「扫过了，没信息」，再也不会重试，
            // 也就再没机会联网补齐——这正是本地歌显示不出歌手/封面的一条路径。
            eprintln!("[listen1] read tags failed, falling back to sidecar: {path} ({error})");
            return Ok(sidecar_audio_meta(&path, sidecar));
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
        cover: clean_text(sidecar.cover_data_url).or_else(|| clean_text(sidecar.cover_url)),
        cover_path: extract_cover_to_cache(&path, &tags),
        duration: Some(duration),
        bitrate,
        tags_read: true,
    })
}

#[tauri::command]
pub fn scan_music_directory(directory: String) -> Result<Vec<String>, String> {
    let root = PathBuf::from(&directory);
    if !root.exists() {
        return Err(format!("目录不存在: {directory}"));
    }
    if !root.is_dir() {
        return Err(format!("路径不是文件夹: {directory}"));
    }

    let mut files = Vec::new();
    scan_dir_recursive(root, &mut files, 20_000)
        .map_err(|e| format!("扫描目录失败 ({directory}): {e}"))?;
    files.sort();
    files.dedup();
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::{cover_cache_key, cover_extension, infer_mime_type};

    #[test]
    fn maps_mime_to_extension() {
        assert_eq!(cover_extension("image/png"), "png");
        assert_eq!(cover_extension("IMAGE/WebP"), "webp");
        assert_eq!(cover_extension("image/bmp"), "bmp");
        // 未知类型统一按 jpg 落盘：浏览器按内容嗅探，扩展名只是给人看的。
        assert_eq!(cover_extension("application/octet-stream"), "jpg");
        assert_eq!(cover_extension(""), "jpg");
    }

    #[test]
    fn sniffs_common_image_headers() {
        assert_eq!(infer_mime_type(&[0x89, b'P', b'N', b'G']), "image/png");
        assert_eq!(infer_mime_type(&[0xff, 0xd8, 0xff, 0xe0]), "image/jpeg");
        assert_eq!(infer_mime_type(b"RIFF____WEBPVP8 "), "image/webp");
        assert_eq!(infer_mime_type(b"nonsense"), "application/octet-stream");
    }

    #[test]
    fn cache_key_is_stable_per_path_and_differs_across_paths() {
        // 同一路径两次调用必须一致，否则每次扫描都会多写一份封面。
        let once = cover_cache_key("/music/a.flac");
        assert_eq!(once, cover_cache_key("/music/a.flac"));
        assert_ne!(once, cover_cache_key("/music/b.flac"));
        assert_eq!(once.len(), 16);
    }
}
