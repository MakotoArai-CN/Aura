use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

const DEFAULT_MAX_BYTES: u64 = 2 * 1024 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheConfig {
    pub enabled: bool,
    pub directory: Option<String>,
    pub max_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheEntry {
    pub url: String,
    pub file_name: String,
    pub content_type: String,
    pub size: u64,
    pub hits: u64,
    pub created_at: u64,
    pub last_access_at: u64,
    pub score: f64,
    #[serde(default = "default_category")]
    pub category: String,
}

#[derive(Debug, Serialize)]
pub struct CacheStats {
    pub enabled: bool,
    pub directory: String,
    pub max_bytes: u64,
    pub total_bytes: u64,
    pub entry_count: usize,
    pub hot_count: usize,
    pub warm_count: usize,
    pub cold_count: usize,
}

#[derive(Debug)]
pub struct CachedBytes {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
struct CacheIndex {
    entries: HashMap<String, CacheEntry>,
}

fn cache_config() -> &'static Mutex<CacheConfig> {
    static CONFIG: OnceLock<Mutex<CacheConfig>> = OnceLock::new();
    CONFIG.get_or_init(|| {
        Mutex::new(CacheConfig {
            enabled: true,
            directory: None,
            max_bytes: DEFAULT_MAX_BYTES,
        })
    })
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_secs())
        .unwrap_or(0)
}

fn default_category() -> String {
    "warm".to_string()
}

fn user_home_dir() -> PathBuf {
    #[cfg(windows)]
    {
        if let Ok(home) = std::env::var("USERPROFILE") {
            return PathBuf::from(home);
        }
    }

    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home);
    }

    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

fn default_cache_dir_path() -> PathBuf {
    user_home_dir().join("Listen1").join("Cache")
}

fn active_config() -> CacheConfig {
    cache_config()
        .lock()
        .map(|guard| guard.clone())
        .unwrap_or(CacheConfig {
            enabled: true,
            directory: None,
            max_bytes: DEFAULT_MAX_BYTES,
        })
}

fn active_cache_dir() -> PathBuf {
    active_config()
        .directory
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(default_cache_dir_path)
}

fn index_path(dir: &Path) -> PathBuf {
    dir.join("index.json")
}

fn read_index(dir: &Path) -> CacheIndex {
    let path = index_path(dir);
    let Ok(text) = std::fs::read_to_string(path) else {
        return CacheIndex::default();
    };
    serde_json::from_str(&text).unwrap_or_default()
}

fn write_index(dir: &Path, index: &CacheIndex) {
    if std::fs::create_dir_all(dir).is_ok() {
        if let Ok(text) = serde_json::to_string_pretty(index) {
            let _ = std::fs::write(index_path(dir), text);
        }
    }
}

fn key_for_url(url: &str) -> String {
    let mut hasher = DefaultHasher::new();
    url.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// 稳定缓存键：优先用调用方传入的稳定标识（平台:歌曲ID），
/// 回退到整条 URL 哈希。稳定 ID 让带时效签名的 URL 不再反复 miss / 重复落盘。
pub fn cache_key_for(cache_id: Option<&str>, url: &str) -> String {
    match cache_id.map(str::trim).filter(|value| !value.is_empty()) {
        Some(id) => {
            let mut hasher = DefaultHasher::new();
            "id:".hash(&mut hasher);
            id.hash(&mut hasher);
            format!("{:016x}", hasher.finish())
        }
        None => key_for_url(url),
    }
}

fn extension_from_url(url: &str) -> &str {
    let path = url.split('?').next().unwrap_or(url);
    let segment = path.rsplit('/').next().unwrap_or(path);
    Path::new(segment)
        .extension()
        .and_then(|ext| ext.to_str())
        .filter(|ext| ext.len() <= 8)
        .unwrap_or("audio")
}

fn score_entry(entry: &CacheEntry, now: u64) -> f64 {
    let age_days = now.saturating_sub(entry.last_access_at) as f64 / 86_400.0;
    let listen_score = (entry.hits as f64).ln_1p() * 28.0;
    let size_penalty = (entry.size as f64 / (1024.0 * 1024.0)).ln_1p() * 1.5;
    listen_score - age_days * 2.0 - size_penalty
}

fn category_entry(entry: &CacheEntry, now: u64) -> String {
    let age_days = now.saturating_sub(entry.last_access_at) as f64 / 86_400.0;
    if entry.hits >= 8 && age_days <= 14.0 {
        "hot".to_string()
    } else if entry.score < -8.0 || age_days >= 45.0 {
        "cold".to_string()
    } else {
        "warm".to_string()
    }
}

fn total_size(index: &CacheIndex) -> u64 {
    index.entries.values().map(|entry| entry.size).sum()
}

fn response_from_file(
    entry: &CacheEntry,
    path: &Path,
    range_header: Option<&str>,
) -> Option<CachedBytes> {
    let mut file = std::fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    if len == 0 {
        return None;
    }

    let range = range_header.and_then(|value| parse_range_header(value, len));
    let (status, start, end) = match range {
        Some((start, end)) => (206, start, end),
        None => (200, 0, len - 1),
    };

    let read_len = end - start + 1;
    let mut body = vec![0; read_len as usize];
    file.seek(SeekFrom::Start(start)).ok()?;
    file.read_exact(&mut body).ok()?;

    let mut headers = vec![
        ("Content-Type".into(), entry.content_type.clone()),
        ("Accept-Ranges".into(), "bytes".into()),
        ("Content-Length".into(), read_len.to_string()),
        ("Access-Control-Allow-Origin".into(), "*".into()),
        (
            "Access-Control-Expose-Headers".into(),
            "Content-Length, Content-Range, Accept-Ranges, X-Listen1-Cache".into(),
        ),
        ("X-Listen1-Cache".into(), "hit".into()),
    ];

    if status == 206 {
        headers.push((
            "Content-Range".into(),
            format!("bytes {}-{}/{}", start, end, len),
        ));
    }

    Some(CachedBytes {
        status,
        headers,
        body,
    })
}

fn parse_range_header(range: &str, len: u64) -> Option<(u64, u64)> {
    if len == 0 {
        return None;
    }

    let value = range.trim().strip_prefix("bytes=")?;
    let first = value.split(',').next()?.trim();
    let (start_raw, end_raw) = first.split_once('-')?;

    if start_raw.is_empty() {
        let suffix_len = end_raw.parse::<u64>().ok()?;
        if suffix_len == 0 {
            return None;
        }
        let start = len.saturating_sub(suffix_len);
        return Some((start, len - 1));
    }

    let start = start_raw.parse::<u64>().ok()?;
    if start >= len {
        return None;
    }

    let end = if end_raw.is_empty() {
        len - 1
    } else {
        end_raw.parse::<u64>().ok()?.min(len - 1)
    };

    (start <= end).then_some((start, end))
}

pub fn try_read(url: &str, range_header: Option<&str>, cache_id: Option<&str>) -> Option<CachedBytes> {
    let config = active_config();
    if !config.enabled {
        return None;
    }

    let dir = active_cache_dir();
    let mut index = read_index(&dir);
    let key = cache_key_for(cache_id, url);
    let now = now_secs();
    let mut entry = index.entries.get(&key)?.clone();
    let path = dir.join(&entry.file_name);
    if !path.exists() {
        index.entries.remove(&key);
        write_index(&dir, &index);
        return None;
    }

    let response = response_from_file(&entry, &path, range_header)?;
    entry.hits = entry.hits.saturating_add(1);
    entry.last_access_at = now;
    entry.score = score_entry(&entry, now);
    entry.category = category_entry(&entry, now);
    index.entries.insert(key, entry);
    write_index(&dir, &index);

    Some(response)
}

pub fn write(url: &str, content_type: &str, bytes: &[u8], cache_id: Option<&str>) {
    let config = active_config();
    if !config.enabled || bytes.is_empty() {
        return;
    }

    let dir = active_cache_dir();
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }

    let key = cache_key_for(cache_id, url);
    let file_name = format!("{}.{}", key, extension_from_url(url));
    let path = dir.join(&file_name);
    if std::fs::write(&path, bytes).is_err() {
        return;
    }

    let now = now_secs();
    let mut index = read_index(&dir);
    let previous_hits = index.entries.get(&key).map(|entry| entry.hits).unwrap_or(0);
    let mut entry = CacheEntry {
        url: url.to_string(),
        file_name,
        content_type: content_type.to_string(),
        size: bytes.len() as u64,
        hits: previous_hits.saturating_add(1),
        created_at: index
            .entries
            .get(&key)
            .map(|entry| entry.created_at)
            .unwrap_or(now),
        last_access_at: now,
        score: 0.0,
        category: "warm".to_string(),
    };
    entry.score = score_entry(&entry, now);
    entry.category = category_entry(&entry, now);
    index.entries.insert(key, entry);
    cleanup_index(&dir, &mut index, config.max_bytes);
    write_index(&dir, &index);
}

fn cleanup_index(dir: &Path, index: &mut CacheIndex, max_bytes: u64) {
    let now = now_secs();

    index.entries.retain(|_, entry| {
        let path = dir.join(&entry.file_name);
        if !path.exists() {
            return false;
        }
        true
    });

    let mut entries: Vec<(String, f64)> = index
        .entries
        .iter_mut()
        .map(|(key, entry)| {
            entry.score = score_entry(entry, now);
            entry.category = category_entry(entry, now);
            (key.clone(), entry.score)
        })
        .collect();

    entries.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));

    while total_size(index) > max_bytes {
        let Some((key, _)) = entries.first().cloned() else {
            break;
        };
        entries.remove(0);
        if let Some(entry) = index.entries.remove(&key) {
            let _ = std::fs::remove_file(dir.join(entry.file_name));
        }
    }
}

#[tauri::command]
pub fn default_cache_dir() -> String {
    default_cache_dir_path().to_string_lossy().to_string()
}

#[tauri::command]
pub fn set_cache_config(config: CacheConfig) {
    if let Ok(mut guard) = cache_config().lock() {
        *guard = CacheConfig {
            max_bytes: if config.max_bytes == 0 {
                DEFAULT_MAX_BYTES
            } else {
                config.max_bytes
            },
            ..config
        };
    }
}

#[tauri::command]
pub fn get_cache_stats() -> CacheStats {
    let config = active_config();
    let dir = active_cache_dir();
    let mut index = read_index(&dir);
    cleanup_index(&dir, &mut index, config.max_bytes);
    write_index(&dir, &index);

    CacheStats {
        enabled: config.enabled,
        directory: dir.to_string_lossy().to_string(),
        max_bytes: config.max_bytes,
        total_bytes: total_size(&index),
        entry_count: index.entries.len(),
        hot_count: index
            .entries
            .values()
            .filter(|entry| entry.category == "hot")
            .count(),
        warm_count: index
            .entries
            .values()
            .filter(|entry| entry.category == "warm")
            .count(),
        cold_count: index
            .entries
            .values()
            .filter(|entry| entry.category == "cold")
            .count(),
    }
}

#[tauri::command]
pub fn clear_audio_cache() -> Result<(), String> {
    let dir = active_cache_dir();
    if dir.exists() {
        for entry in std::fs::read_dir(&dir).map_err(|e| e.to_string())? {
            let path = entry.map_err(|e| e.to_string())?.path();
            if path.is_file() {
                std::fs::remove_file(path).map_err(|e| e.to_string())?;
            }
        }
    }
    write_index(&dir, &CacheIndex::default());
    Ok(())
}
