use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::io::{Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
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
    pub body: crate::proxy::StreamBody,
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

/// 用户主目录。device_tier 也要用它来放效果档位覆盖文件，所以是 crate 可见。
pub(crate) fn user_home_dir() -> PathBuf {
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

/// 内存索引：避免每次缓存命中都读+写 index.json。
/// 播放期间 HTML5 Audio 会发起大量 Range 请求，磁盘往返成为热点。
/// 命中计数仅记内存，随下一次写操作/统计查询落盘。
struct MemoryIndex {
    dir: PathBuf,
    index: CacheIndex,
    hits_dirty: bool,
}

fn memory_index() -> &'static Mutex<Option<MemoryIndex>> {
    static MEM_INDEX: OnceLock<Mutex<Option<MemoryIndex>>> = OnceLock::new();
    MEM_INDEX.get_or_init(|| Mutex::new(None))
}

fn load_memory_index(dir: &Path) -> std::sync::MutexGuard<'static, Option<MemoryIndex>> {
    let mut guard = memory_index()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let needs_reload = match guard.as_ref() {
        Some(mem) => mem.dir != dir,
        None => true,
    };
    if needs_reload {
        *guard = Some(MemoryIndex {
            dir: dir.to_path_buf(),
            index: read_index(dir),
            hits_dirty: false,
        });
    }
    guard
}

fn persist_if_dirty(guard: &mut std::sync::MutexGuard<'static, Option<MemoryIndex>>) {
    if let Some(mem) = guard.as_mut() {
        if mem.hits_dirty {
            write_index(&mem.dir, &mem.index);
            mem.hits_dirty = false;
        }
    }
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
        // 紧凑 JSON：这个文件只给程序读，pretty 会让条目多起来后每次写盘都多出
        // 好几倍的字节，而这段写盘是压在全局互斥锁里的。
        if let Ok(text) = serde_json::to_string(index) {
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
    file.seek(SeekFrom::Start(start)).ok()?;

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
        body: crate::proxy::StreamBody::File {
            file,
            len: read_len,
        },
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
    let mut mem_guard = load_memory_index(&dir);
    let mem = mem_guard.as_mut()?;
    let key = cache_key_for(cache_id, url);
    let now = now_secs();
    // 只克隆轻量元数据，正文按需从磁盘读取。
    let entry = mem.index.entries.get(&key)?.clone();
    let path = dir.join(&entry.file_name);
    if !path.exists() {
        mem.index.entries.remove(&key);
        // 不在读路径上写盘：标脏，交给下一次缓存写入一起落盘。
        mem.hits_dirty = true;
        return None;
    }

    let response = response_from_file(&entry, &path, range_header)?;
    if let Some(stored) = mem.index.entries.get_mut(&key) {
        stored.hits = stored.hits.saturating_add(1);
        stored.last_access_at = now;
        stored.score = score_entry(stored, now);
        stored.category = category_entry(stored, now);
    }
    mem.hits_dirty = true;

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

    let evicted = record_entry(
        &dir,
        key,
        url,
        file_name,
        content_type,
        bytes.len() as u64,
        config.max_bytes,
    );
    for victim in evicted {
        let _ = std::fs::remove_file(victim);
    }
}

/// 为「边下边播」的旁路缓存准备一个独占的 `.part` 路径。
/// 返回 None 表示缓存被关掉或目录不可用，此时上层直接不做旁路写入。
///
/// 序号让同一首歌的两条并发连接不会写进同一个临时文件——否则两边交错写，
/// 先提交的那个会把一份损坏的音频永久留在缓存里。
pub(crate) fn begin_temp_write(url: &str, cache_id: Option<&str>) -> Option<PathBuf> {
    let config = active_config();
    if !config.enabled {
        return None;
    }

    let dir = active_cache_dir();
    std::fs::create_dir_all(&dir).ok()?;
    sweep_stale_temp_files(&dir);

    static TEMP_SEQ: AtomicU64 = AtomicU64::new(0);
    let seq = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let key = cache_key_for(cache_id, url);
    Some(dir.join(format!(
        "{}.{}.{}.part",
        key,
        extension_from_url(url),
        seq
    )))
}

/// 进程内只做一次：清掉上次崩溃/强杀留下的 `.part`。
/// 只删 10 分钟前的，避免误删同时在下的另一条连接。
fn sweep_stale_temp_files(dir: &Path) {
    static SWEPT: OnceLock<()> = OnceLock::new();
    if SWEPT.get().is_some() {
        return;
    }
    let _ = SWEPT.set(());

    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let cutoff = std::time::Duration::from_secs(600);
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("part") {
            continue;
        }
        let stale = entry
            .metadata()
            .and_then(|m| m.modified())
            .and_then(|t| SystemTime::now().duration_since(t).map_err(std::io::Error::other))
            .map(|age| age > cutoff)
            .unwrap_or(false);
        if stale {
            let _ = std::fs::remove_file(&path);
        }
    }
}

/// `.part` 改名为正式缓存文件并登记索引。返回是否提交成功；false 时临时文件
/// 仍在原处，由调用方删除。
pub(crate) fn commit_temp_write(
    temp_path: &Path,
    url: &str,
    content_type: &str,
    size: u64,
    cache_id: Option<&str>,
) -> bool {
    let config = active_config();
    if !config.enabled || size == 0 {
        return false;
    }

    let dir = active_cache_dir();
    let key = cache_key_for(cache_id, url);
    let file_name = format!("{}.{}", key, extension_from_url(url));
    let path = dir.join(&file_name);
    // Windows 上 rename 不会覆盖已存在的目标文件。
    if path.exists() {
        let _ = std::fs::remove_file(&path);
    }
    if std::fs::rename(temp_path, &path).is_err() {
        return false;
    }

    #[cfg(debug_assertions)]
    let started = std::time::Instant::now();

    let evicted = record_entry(
        &dir,
        key,
        url,
        file_name,
        content_type,
        size,
        config.max_bytes,
    );
    let evicted_count = evicted.len();
    for victim in evicted {
        let _ = std::fs::remove_file(victim);
    }

    #[cfg(debug_assertions)]
    {
        let _ = evicted_count;
        let elapsed = started.elapsed();
        if elapsed > std::time::Duration::from_millis(120) {
            eprintln!(
                "[cache][slow] index update took {}ms (size={size}, evicted={evicted_count})",
                elapsed.as_millis()
            );
        }
    }
    #[cfg(not(debug_assertions))]
    let _ = evicted_count;

    true
}

/// 把一个已经落盘的缓存文件登记进索引，返回需要在**锁外**删除的淘汰文件。
///
/// 序列化在锁内（纯 CPU），写盘放到锁外：播放期间每个 Range 请求都要抢
/// `memory_index()` 这把锁，把 index.json 的写盘和淘汰删除压在锁里，
/// 等于让音频缓冲跟磁盘 IO 排队。
fn record_entry(
    dir: &Path,
    key: String,
    url: &str,
    file_name: String,
    content_type: &str,
    size: u64,
    max_bytes: u64,
) -> Vec<PathBuf> {
    let now = now_secs();
    let mut mem_guard = load_memory_index(dir);
    let Some(mem) = mem_guard.as_mut() else {
        return Vec::new();
    };

    let previous = mem.index.entries.get(&key);
    let previous_hits = previous.map(|entry| entry.hits).unwrap_or(0);
    let created_at = previous.map(|entry| entry.created_at).unwrap_or(now);
    let mut entry = CacheEntry {
        url: url.to_string(),
        file_name,
        content_type: content_type.to_string(),
        size,
        hits: previous_hits.saturating_add(1),
        created_at,
        last_access_at: now,
        score: 0.0,
        category: "warm".to_string(),
    };
    entry.score = score_entry(&entry, now);
    entry.category = category_entry(&entry, now);
    mem.index.entries.insert(key, entry);

    let evicted = cleanup_index(dir, &mut mem.index, max_bytes);
    let text = serde_json::to_string(&mem.index).ok();
    mem.hits_dirty = false;
    drop(mem_guard);

    if let Some(text) = text {
        if std::fs::create_dir_all(dir).is_ok() {
            let _ = std::fs::write(index_path(dir), text);
        }
    }

    evicted
}

/// 返回被淘汰条目的文件路径，由调用方在放开锁之后删除。
///
/// 没超限就**什么都不做**：原先每次缓存写入都要 stat 所有条目 + 重算所有分数 +
/// 全量排序，几百条目就是几百次系统调用，全压在跟 `try_read` 共享的那把锁里。
fn cleanup_index(dir: &Path, index: &mut CacheIndex, max_bytes: u64) -> Vec<PathBuf> {
    if total_size(index) <= max_bytes {
        return Vec::new();
    }

    let now = now_secs();
    index
        .entries
        .retain(|_, entry| dir.join(&entry.file_name).exists());

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

    let mut removed = Vec::new();
    let mut total = total_size(index);
    for (key, _) in entries {
        if total <= max_bytes {
            break;
        }
        if let Some(entry) = index.entries.remove(&key) {
            total = total.saturating_sub(entry.size);
            removed.push(dir.join(entry.file_name));
        }
    }
    removed
}

#[tauri::command]
pub fn default_cache_dir() -> String {
    default_cache_dir_path().to_string_lossy().to_string()
}

/// 离线播放探测：给定稳定缓存键（平台:歌曲ID），返回已落盘音频的绝对路径。
/// 断网时前端拿不到新的播放地址，改用这里返回的本地文件直接播放。
#[tauri::command]
pub fn audio_cache_lookup(cache_id: String) -> Option<String> {
    let trimmed = cache_id.trim();
    if trimmed.is_empty() || !active_config().enabled {
        return None;
    }

    let dir = active_cache_dir();
    let key = cache_key_for(Some(trimmed), "");
    let mut mem_guard = load_memory_index(&dir);
    let (size, file_name) = mem_guard
        .as_ref()?
        .index
        .entries
        .get(&key)
        .map(|entry| (entry.size, entry.file_name.clone()))?;
    if size == 0 {
        return None;
    }

    let path = dir.join(&file_name);
    if !path.exists() {
        // 索引与磁盘不一致：清掉这条，免得每次都白探测一遍。
        if let Some(mem) = mem_guard.as_mut() {
            mem.index.entries.remove(&key);
            mem.hits_dirty = true;
        }
        persist_if_dirty(&mut mem_guard);
        return None;
    }

    Some(path.to_string_lossy().to_string())
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
    let mut mem_guard = load_memory_index(&dir);
    if mem_guard.is_none() {
        return CacheStats {
            enabled: config.enabled,
            directory: dir.to_string_lossy().to_string(),
            max_bytes: config.max_bytes,
            total_bytes: 0,
            entry_count: 0,
            hot_count: 0,
            warm_count: 0,
            cold_count: 0,
        };
    }
    let index = &mut mem_guard.as_mut().unwrap().index;
    // cleanup_index 现在只在真的超限时才重算分数，统计面板要自己刷新一次。
    refresh_scores(index);
    let evicted = cleanup_index(&dir, index, config.max_bytes);

    let stats = CacheStats {
        enabled: config.enabled,
        directory: dir.to_string_lossy().to_string(),
        max_bytes: config.max_bytes,
        total_bytes: total_size(index),
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
    };
    persist_if_dirty(&mut mem_guard);
    if let Some(mem) = mem_guard.as_ref() {
        write_index(&dir, &mem.index);
    }
    drop(mem_guard);
    for victim in evicted {
        let _ = std::fs::remove_file(victim);
    }
    stats
}

fn refresh_scores(index: &mut CacheIndex) {
    let now = now_secs();
    for entry in index.entries.values_mut() {
        entry.score = score_entry(entry, now);
        entry.category = category_entry(entry, now);
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
    // 同步重置内存索引，避免残留旧条目。
    if let Ok(mut guard) = memory_index().lock() {
        *guard = Some(MemoryIndex {
            dir: dir.clone(),
            index: CacheIndex::default(),
            hits_dirty: false,
        });
    }
    write_index(&dir, &CacheIndex::default());
    Ok(())
}
