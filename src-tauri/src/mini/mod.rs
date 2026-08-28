//! 轻量模式：零 WebView 的原生迷你播放器。
//!
//! 整个应用平时是 Tauri + WebView，CPU 与内存的大头都在 WebView2 那几个进程上。
//! 轻量模式把 WebView 彻底销毁，只留一个自绘的 Win32 窗口 + 系统播放器：
//!
//! - 音频交给 WinRT `Windows.Media.Playback.MediaPlayer`，解码和输出都是系统做的，
//!   本进程不引入任何解码库。
//! - 界面用 Direct2D + DirectWrite + WIC 自绘，没有浏览器，也没有 JS。
//! - 数据靠切换前写下的快照（见 `snapshot`），加上 `cache::audio_cache_lookup`
//!   这条不经过前端的磁盘缓存查找。
//!
//! 搜索、发现、设置这些要联网解析的界面不在轻量模式里，触发时回到完整模式。

pub mod commands;
pub mod snapshot;

#[cfg(target_os = "windows")]
mod audio;
#[cfg(target_os = "windows")]
mod lyrics;
#[cfg(target_os = "windows")]
mod win;

pub use snapshot::{MiniSnapshot, MiniTrack};

/// 托盘菜单能对轻量模式发的播放动作。
#[derive(Copy, Clone, Debug)]
pub enum LiteAction {
    PlayPause,
    Prev,
    Next,
}

/// 关掉原生窗口之后是要退进程，还是回完整模式。
///
/// 两条路都得先让原生窗口把进度写完（见 `win::Ui::finish`），区别只在收尾：
/// 一个建回 WebView，一个直接退。少了这个标志，轻量模式下点托盘「退出」
/// 会被 `ExitRequested` 那边无条件拦住，变成一个没有任何反应的死按钮。
static SHUTTING_DOWN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn begin_shutdown() {
    SHUTTING_DOWN.store(true, std::sync::atomic::Ordering::Release);
}

/// 每次进轻量模式都清一遍旗子。上一轮的关机意图不该泄漏到下一轮，
/// 否则再点「回到完整模式」会变成直接退进程。
pub fn clear_shutdown() {
    SHUTTING_DOWN.store(false, std::sync::atomic::Ordering::Release);
}

pub fn is_shutting_down() -> bool {
    SHUTTING_DOWN.load(std::sync::atomic::Ordering::Acquire)
}

/// 原生迷你窗口是否开着。
///
/// `lib.rs` 用它决定"最后一个 WebView 没了"时要不要拦住进程退出——轻量模式下
/// main 窗口是真的被销毁的，默认行为会连带把还在放音乐的原生窗口一起收走。
#[cfg(target_os = "windows")]
pub fn is_lite_active() -> bool {
    win::is_open()
}

#[cfg(not(target_os = "windows"))]
pub fn is_lite_active() -> bool {
    false
}

/// 请求关掉原生窗口。真正的收尾（写回进度、决定回完整模式还是退进程）在窗口
/// 线程的 `WM_DESTROY` 里做，这里只是把请求发过去。
#[cfg(target_os = "windows")]
pub fn request_close() {
    win::close();
}

#[cfg(not(target_os = "windows"))]
pub fn request_close() {}

/// 托盘点「显示」时把原生窗口拉到前面。轻量模式下没有 WebView 可以 show。
#[cfg(target_os = "windows")]
pub fn focus_lite() {
    win::focus();
}

#[cfg(not(target_os = "windows"))]
pub fn focus_lite() {}

/// 把托盘的播放控制转给原生窗口。
#[cfg(target_os = "windows")]
pub fn lite_action(action: LiteAction) {
    win::transport(action);
}

#[cfg(not(target_os = "windows"))]
pub fn lite_action(_action: LiteAction) {}

/// 把本地路径转成 `MediaPlayer` 能吃的 `file:///` URI。
///
/// Windows 路径里的反斜杠要换成正斜杠，盘符前面补一个斜杠。空格等字符不做百分号
/// 编码——`Uri::CreateUri` 自己会处理，提前编码反而会把已经编码过的路径搞坏。
fn file_uri(path: &str) -> String {
    let normalized = path.replace('\\', "/");
    let trimmed = normalized.trim_start_matches('/');
    format!("file:///{trimmed}")
}

/// 决定一首歌到底从哪儿播，按"越不容易失效越优先"排序：
///
/// 1. `local_path` —— 本地音乐或前端预缓存后登记的文件，永不过期。
/// 2. 磁盘音频缓存 —— 用 `source:id` 查，断网也能播，且不经过前端。
/// 3. 快照里的 `url` —— 带时效签名的直链，随时可能已经死了，只能兜底。
///
/// 三条都没有就返回 None，原生侧跳过这首。
pub fn playable_uri(track: &MiniTrack) -> Option<String> {
    if let Some(path) = track
        .local_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Some(file_uri(path));
    }

    let cache_id = track.cache_id();
    if !cache_id.is_empty() {
        if let Some(path) = crate::cache::audio_cache_lookup(cache_id) {
            return Some(file_uri(&path));
        }
    }

    track
        .url
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(id: &str, source: &str) -> MiniTrack {
        MiniTrack {
            id: id.to_string(),
            title: "t".to_string(),
            source: source.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn local_path_becomes_a_file_uri_with_forward_slashes() {
        let mut item = track("1", "kuwo");
        item.local_path = Some(r"C:\Users\me\My Music\a.flac".to_string());
        assert_eq!(
            playable_uri(&item).unwrap(),
            "file:///C:/Users/me/My Music/a.flac"
        );
    }

    #[test]
    fn cache_id_matches_the_frontend_format() {
        assert_eq!(track("12345", "kuwo").cache_id(), "kuwo:12345");
    }

    #[test]
    fn local_tracks_do_not_participate_in_the_audio_cache() {
        let mut item = track("12345", "localmusic");
        item.local_path = Some("/tmp/a.mp3".to_string());
        assert_eq!(item.cache_id(), "");
    }

    #[test]
    fn remote_url_is_the_last_resort() {
        let mut item = track("nope-not-cached", "qq");
        item.url = Some("http://127.0.0.1:1/stream/x?sig=1".to_string());
        assert_eq!(
            playable_uri(&item).unwrap(),
            "http://127.0.0.1:1/stream/x?sig=1"
        );
    }

    #[test]
    fn a_track_with_nothing_playable_is_skipped() {
        assert!(playable_uri(&track("x", "qq")).is_none());
    }

    #[test]
    fn normalize_clamps_index_position_volume_and_loop_mode() {
        let mut snap = MiniSnapshot {
            index: 99,
            position: -3.0,
            volume: 4.5,
            loop_mode: 7,
            tracks: vec![track("a", "qq"), track("b", "qq")],
            ..Default::default()
        };
        snap.normalize();
        assert_eq!(snap.index, 1);
        assert_eq!(snap.position, 0.0);
        assert_eq!(snap.volume, 1.0);
        assert_eq!(snap.loop_mode, 0);

        let mut empty = MiniSnapshot {
            index: 5,
            ..Default::default()
        };
        empty.normalize();
        assert_eq!(empty.index, 0);
        assert!(empty.current().is_none());
    }

    #[test]
    fn snapshot_survives_a_json_round_trip() {
        let mut original = MiniSnapshot {
            index: 1,
            position: 42.5,
            volume: 0.35,
            muted: true,
            loop_mode: 2,
            tracks: vec![track("a", "qq"), track("b", "kuwo")],
            ..Default::default()
        };
        original.tracks[0].lyric = Some("[00:01.00]hi".to_string());
        original.tracks[1].cover_path = Some(r"C:\covers\b.jpg".to_string());

        let body = serde_json::to_vec(&original).unwrap();
        let back: MiniSnapshot = serde_json::from_slice(&body).unwrap();
        assert_eq!(back.index, 1);
        assert_eq!(back.position, 42.5);
        assert_eq!(back.volume, 0.35);
        assert!(back.muted);
        assert_eq!(back.loop_mode, 2);
        assert_eq!(back.tracks.len(), 2);
        assert_eq!(back.tracks[0].lyric.as_deref(), Some("[00:01.00]hi"));
        assert_eq!(back.tracks[1].cover_path.as_deref(), Some(r"C:\covers\b.jpg"));
    }

    #[test]
    fn an_unknown_snapshot_version_is_ignored_rather_than_half_read() {
        let dir = std::env::temp_dir().join("aura-mini-snapshot-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("future.json");
        std::fs::write(&path, br#"{"version":9999,"tracks":[]}"#).unwrap();
        assert!(snapshot::load_from(&path).is_none());
        let _ = std::fs::remove_file(&path);
    }
}
