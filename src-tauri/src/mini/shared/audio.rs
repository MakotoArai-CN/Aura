//! 轻量模式的音频与系统媒体接口。
//!
//! 这两个 trait 存在的理由是同一个：轻量模式要在 Windows / macOS / Linux / FreeBSD
//! 上各用一套原生绘制后端，但状态机、布局、命中测试只应该有一份。播放和系统媒体控制
//! 是唯一两处真正需要按平台分叉的运行时能力，把它们收在 trait 后面，
//! `shared/` 就能保持对任何平台 crate 的零依赖。
//!
//! `AudioBackend` 的方法签名是照着原来那份 WinRT `MediaPlayer` 包装一比一抄下来的，
//! 因为 `win.rs` 已经按那个形状写好了。有两条不变量是承重的，换实现时必须一起搬：
//!
//! 1. `load` 要先把 ended / failed / duration 清干净。否则上一首残留的结束事件会把
//!    刚加载的这首立刻跳过去。
//! 2. `take_ended` / `take_failed` 是取走即清的（`swap(false)`），一次结束只能推进一首。

use std::path::PathBuf;
use std::sync::Arc;

/// 把播放地址解析成一个本地文件。
///
/// 第一个参数是地址（`file:///…` 或远端直链），第二个是缓存键（形如 `source:id`，
/// 不知道时给空串）。远端地址要先落盘才能播，落盘就得有个键去查和写磁盘缓存。
///
/// 解析必然要碰 `crate::cache` 和 `crate::proxy`——那些都不该出现在 `shared/` 里。
/// 所以这一步由调用方注入：`shared/` 只知道"给我一个能打开的文件路径"，
/// 不知道它是怎么来的。这同时也是给另外三个平台留的接缝。
pub type UriResolver = Arc<dyn Fn(&str, &str) -> Result<PathBuf, String> + Send + Sync>;

/// 轻量模式的播放引擎。
pub trait AudioBackend: Send {
    /// 换源。`cache_id` 只是转交给 [`UriResolver`] 的提示，引擎自己不关心缓存。
    ///
    /// 实现可以是异步的：真正的失败允许晚一点通过 `take_failed` 报出来，
    /// 这跟原来 WinRT 用 `MediaFailed` 事件报错的行为一致。
    fn load(&self, uri: &str, cache_id: &str) -> Result<(), String>;

    fn play(&self) -> Result<(), String>;
    fn pause(&self) -> Result<(), String>;

    /// 0.0 ~ 1.0，越界和非有限值一律夹住。
    fn set_volume(&self, volume: f64) -> Result<(), String>;
    fn set_muted(&self, muted: bool) -> Result<(), String>;

    fn seek(&self, seconds: f64) -> Result<(), String>;
    fn position(&self) -> f64;

    /// 解析出来的时长。还不知道时返回 0，调用方会退回快照里的值。
    fn duration(&self) -> f64;

    fn is_playing(&self) -> bool;

    /// 正在打开 / 下载 / 缓冲。UI 拿它显示加载态，且不能把它误判成暂停。
    fn is_busy(&self) -> bool;

    /// 取走"播完了"。取一次就清掉。
    fn take_ended(&self) -> bool;
    /// 取走"播不了"。取一次就清掉。
    fn take_failed(&self) -> bool;
}

/// 交给系统媒体控制显示的曲目信息。
///
/// 封面只给本地文件路径：原生窗口没有 WebView 的图片加载能力，快照里存的本来就是
/// 已经下载到 `covers/` 的本地文件。
pub struct NowPlaying<'a> {
    pub title: &'a str,
    pub artist: &'a str,
    pub album: &'a str,
    pub cover_path: Option<&'a str>,
}

/// 系统媒体控制：Windows 的 SMTC、Linux/FreeBSD 的 MPRIS、macOS 的
/// `MPNowPlayingInfoCenter`。
///
/// 这一层不是可选的装饰。原来的 WinRT `MediaPlayer` 顺带就把媒体键、系统音量 OSD、
/// 锁屏曲目信息全接上了；换成自己解码之后这些一个都不剩，必须自己补回来，
/// 否则「换个解码器」就变成了纯功能倒退。
pub trait MediaControls: Send {
    fn set_now_playing(&self, info: &NowPlaying<'_>);
    fn set_playing(&self, playing: bool);
    /// 退出轻量模式时收摊，别在系统里留一条永远停在那首歌的记录。
    fn clear(&self);
}

/// 还没接系统媒体控制的平台用这个。
pub struct NoopMediaControls;

impl MediaControls for NoopMediaControls {
    fn set_now_playing(&self, _info: &NowPlaying<'_>) {}
    fn set_playing(&self, _playing: bool) {}
    fn clear(&self) {}
}
