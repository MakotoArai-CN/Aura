//! 轻量模式的交接快照。
//!
//! 轻量模式下 WebView 是真的被销毁的，前端那套 provider 解析、歌词抓取、封面代理
//! 全都不在了。所以切换之前必须把原生播放器需要的一切写到磁盘上：队列、当前位置、
//! 音量、循环模式，以及每首歌的歌词原文和封面本地路径。
//!
//! 歌词必须内嵌：`MediaService.getLyric` 是每次播放都重新抓的，磁盘上没有任何歌词
//! 缓存，原生窗口没法自己去取。封面同理，`Track.img_url` 多数是远端地址，只有
//! `file://` 与 b 站域名会过本地代理，所以前端得先把图下载到 `covers/` 再登记路径。
//!
//! 音频则不用内嵌：`cache::audio_cache_lookup` 用 `source:id` 就能在磁盘缓存里找到
//! 完整可播文件，这条路不经过前端。快照里的 `url` 只是没命中缓存时的备用，它带时效
//! 签名，过期后就废了——这也是为什么切换前要先把后续几首预缓存下来。

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// 快照格式版本。结构不兼容地变了就加一，读到不认识的版本直接当没有快照。
pub const SNAPSHOT_VERSION: u32 = 1;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MiniTrack {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub artist: String,
    #[serde(default)]
    pub album: String,
    #[serde(default)]
    pub source: String,
    /// 秒。前端拿不到时长时是 0，原生侧改用系统播放器报的时长。
    #[serde(default)]
    pub duration: f64,
    /// 快照时已解析出的直链。带时效签名，仅作缓存未命中时的备用。
    #[serde(default)]
    pub url: Option<String>,
    /// 本地音乐的真实路径，或前端预缓存后登记的文件路径。永不过期。
    #[serde(default)]
    pub local_path: Option<String>,
    /// 已下载到 `covers/` 的封面文件路径。
    #[serde(default)]
    pub cover_path: Option<String>,
    /// LRC 原文。原生侧自己解析。
    #[serde(default)]
    pub lyric: Option<String>,
    /// 翻译 LRC 原文。
    #[serde(default)]
    pub tlyric: Option<String>,
}

impl MiniTrack {
    /// 磁盘音频缓存的查找键，必须和前端 `player.ts` 的 `cacheIdFor` 一字不差，
    /// 否则查不到前端自己写下的文件。本地曲目不参与缓存，返回空串。
    pub fn cache_id(&self) -> String {
        if self.id.is_empty() || self.local_path.is_some() {
            return String::new();
        }
        format!("{}:{}", self.source, self.id)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MiniSnapshot {
    pub version: u32,
    /// Unix 秒。用于判断快照是不是太老了（直链大概率已死）。
    #[serde(default)]
    pub saved_at: i64,
    /// 当前曲目下标。越界或为负时原生侧从 0 开始。
    #[serde(default)]
    pub index: i64,
    /// 当前播放进度，秒。
    #[serde(default)]
    pub position: f64,
    /// 0.0 ~ 1.0。前端存的是 0~100，写快照时换算。
    #[serde(default = "default_volume")]
    pub volume: f64,
    #[serde(default)]
    pub muted: bool,
    /// 0 顺序 1 单曲循环 2 随机，与前端 `LoopMode` 一致。
    #[serde(default)]
    pub loop_mode: u8,
    #[serde(default)]
    pub tracks: Vec<MiniTrack>,
}

fn default_volume() -> f64 {
    0.9
}

impl Default for MiniSnapshot {
    fn default() -> Self {
        Self {
            version: SNAPSHOT_VERSION,
            saved_at: 0,
            index: 0,
            position: 0.0,
            volume: default_volume(),
            muted: false,
            loop_mode: 0,
            tracks: Vec::new(),
        }
    }
}

impl MiniSnapshot {
    /// 把越界下标、离谱音量这类脏数据收拾干净，免得每个消费点都得自己防一遍。
    pub fn normalize(&mut self) {
        if self.tracks.is_empty() {
            self.index = 0;
        } else {
            let last = self.tracks.len() as i64 - 1;
            self.index = self.index.clamp(0, last);
        }
        if !self.position.is_finite() || self.position < 0.0 {
            self.position = 0.0;
        }
        if !self.volume.is_finite() {
            self.volume = default_volume();
        }
        self.volume = self.volume.clamp(0.0, 1.0);
        if self.loop_mode > 2 {
            self.loop_mode = 0;
        }
    }

    pub fn current(&self) -> Option<&MiniTrack> {
        self.tracks.get(self.index.max(0) as usize)
    }
}

/// 轻量模式的数据目录。放在 `Listen1/` 下但不进 `Cache/`——清缓存不该把交接
/// 快照和封面一起清掉。
pub fn mini_dir() -> PathBuf {
    crate::cache::user_home_dir().join("Listen1").join("mini")
}

pub fn snapshot_path() -> PathBuf {
    mini_dir().join("snapshot.json")
}

pub fn covers_dir() -> PathBuf {
    mini_dir().join("covers")
}

/// 原子写：先写同目录的临时文件再 rename，避免切换过程中崩了留下半个 JSON。
pub fn save(snapshot: &MiniSnapshot) -> Result<(), String> {
    let dir = mini_dir();
    std::fs::create_dir_all(&dir).map_err(|err| format!("创建轻量模式目录失败: {err}"))?;
    let body = serde_json::to_vec_pretty(snapshot).map_err(|err| format!("序列化快照失败: {err}"))?;
    let target = snapshot_path();
    let temp = dir.join("snapshot.json.tmp");
    std::fs::write(&temp, &body).map_err(|err| format!("写入快照失败: {err}"))?;
    std::fs::rename(&temp, &target).map_err(|err| format!("替换快照失败: {err}"))?;
    Ok(())
}

pub fn load() -> Option<MiniSnapshot> {
    load_from(&snapshot_path())
}

/// 删掉快照文件。读回来一次就该删：里面的进度只对"刚从轻量模式回来"这一次有意义，
/// 留着的话下次冷启动会莫名其妙地跳回几天前的位置。
pub fn clear() {
    let _ = std::fs::remove_file(snapshot_path());
}

pub fn load_from(path: &Path) -> Option<MiniSnapshot> {
    let body = std::fs::read(path).ok()?;
    let mut snapshot: MiniSnapshot = serde_json::from_slice(&body).ok()?;
    if snapshot.version != SNAPSHOT_VERSION {
        return None;
    }
    snapshot.normalize();
    Some(snapshot)
}
