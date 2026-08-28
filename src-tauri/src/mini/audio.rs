//! 轻量模式的播放引擎：WinRT `Windows.Media.Playback.MediaPlayer`。
//!
//! 选它而不是 rodio/symphonia 是因为轻量模式的目的就是省开销：解码、重采样、混音、
//! 输出全由系统的媒体管线做，能吃到硬件卸载，本进程一行解码代码都不用带，二进制也
//! 不会因为多塞一个解码器变大。它同时吃 `file:///` 和 http(s)，本地缓存文件和本地
//! 代理地址都能直接喂进去，缓冲和 Range 请求也是它自己管。
//!
//! 事件（播放结束、失败）在系统线程上回调，所以状态一律放原子量里，UI 线程轮询读。

use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::Arc;

use windows::Foundation::{TimeSpan, TypedEventHandler, Uri};
use windows::Media::Core::MediaSource;
use windows::Media::Playback::{MediaPlaybackState, MediaPlayer};

/// TimeSpan 的单位是 100 纳秒。
const TICKS_PER_SECOND: f64 = 10_000_000.0;

fn ticks_to_seconds(span: TimeSpan) -> f64 {
    span.Duration as f64 / TICKS_PER_SECOND
}

fn seconds_to_ticks(seconds: f64) -> TimeSpan {
    let clamped = if seconds.is_finite() && seconds > 0.0 { seconds } else { 0.0 };
    TimeSpan {
        Duration: (clamped * TICKS_PER_SECOND) as i64,
    }
}

/// 系统线程回调写、UI 线程读的那部分状态。
#[derive(Default)]
struct Shared {
    /// 一首播完了。UI 线程取走后清零，用它驱动"下一首"。
    ended: AtomicBool,
    /// 这首播不了（直链过期、网络断了、格式不支持）。同样由 UI 线程取走。
    failed: AtomicBool,
    /// 系统报的时长，100ns。快照里的 duration 常常是 0，这个才准。
    duration_ticks: AtomicI64,
}

pub struct Engine {
    player: MediaPlayer,
    shared: Arc<Shared>,
}

impl Engine {
    pub fn new() -> windows::core::Result<Self> {
        let player = MediaPlayer::new()?;
        // 系统默认会在播完后停在末尾；自动推进由我们自己控制，这里只要事件。
        player.SetAutoPlay(false)?;
        let shared = Arc::new(Shared::default());

        let ended_shared = shared.clone();
        player.MediaEnded(&TypedEventHandler::new(move |_, _| {
            ended_shared.ended.store(true, Ordering::Release);
            Ok(())
        }))?;

        let failed_shared = shared.clone();
        player.MediaFailed(&TypedEventHandler::new(move |_, _| {
            failed_shared.failed.store(true, Ordering::Release);
            Ok(())
        }))?;

        let opened_shared = shared.clone();
        player.MediaOpened(&TypedEventHandler::new(
            move |sender: windows::core::Ref<'_, MediaPlayer>, _| {
                if let Some(player) = sender.as_ref() {
                    if let Ok(session) = player.PlaybackSession() {
                        if let Ok(duration) = session.NaturalDuration() {
                            opened_shared
                                .duration_ticks
                                .store(duration.Duration, Ordering::Release);
                        }
                    }
                }
                Ok(())
            },
        ))?;

        Ok(Self { player, shared })
    }

    /// 换源。`uri` 可以是 `file:///…` 或 http(s)。换源会清掉上一首的结束/失败标记，
    /// 否则刚加载的新曲目会被上一首的残留事件立刻跳过。
    pub fn load(&self, uri: &str) -> windows::core::Result<()> {
        self.shared.ended.store(false, Ordering::Release);
        self.shared.failed.store(false, Ordering::Release);
        self.shared.duration_ticks.store(0, Ordering::Release);
        let source = MediaSource::CreateFromUri(&Uri::CreateUri(&uri.into())?)?;
        self.player.SetSource(&source)
    }

    pub fn play(&self) -> windows::core::Result<()> {
        self.player.Play()
    }

    pub fn pause(&self) -> windows::core::Result<()> {
        self.player.Pause()
    }

    /// 0.0 ~ 1.0。超范围的值先夹住，系统对越界音量的行为没有保证。
    pub fn set_volume(&self, volume: f64) -> windows::core::Result<()> {
        let clamped = if volume.is_finite() { volume.clamp(0.0, 1.0) } else { 1.0 };
        self.player.SetVolume(clamped)
    }

    pub fn set_muted(&self, muted: bool) -> windows::core::Result<()> {
        self.player.SetIsMuted(muted)
    }

    pub fn seek(&self, seconds: f64) -> windows::core::Result<()> {
        self.player.PlaybackSession()?.SetPosition(seconds_to_ticks(seconds))
    }

    pub fn position(&self) -> f64 {
        self.player
            .PlaybackSession()
            .and_then(|session| session.Position())
            .map(ticks_to_seconds)
            .unwrap_or(0.0)
    }

    /// 系统报的时长。还没解析出来时是 0，调用方该退回快照里的值。
    pub fn duration(&self) -> f64 {
        let ticks = self.shared.duration_ticks.load(Ordering::Acquire);
        if ticks > 0 {
            return ticks as f64 / TICKS_PER_SECOND;
        }
        self.player
            .PlaybackSession()
            .and_then(|session| session.NaturalDuration())
            .map(ticks_to_seconds)
            .unwrap_or(0.0)
    }

    pub fn is_playing(&self) -> bool {
        self.player
            .PlaybackSession()
            .and_then(|session| session.PlaybackState())
            .map(|state| state == MediaPlaybackState::Playing)
            .unwrap_or(false)
    }

    /// 缓冲/打开中。UI 拿它显示加载态，也用来避免把缓冲误判成暂停。
    pub fn is_busy(&self) -> bool {
        self.player
            .PlaybackSession()
            .and_then(|session| session.PlaybackState())
            .map(|state| {
                state == MediaPlaybackState::Opening || state == MediaPlaybackState::Buffering
            })
            .unwrap_or(false)
    }

    /// 取走"播完了"标记。取一次就清掉，避免同一次结束推进两首。
    pub fn take_ended(&self) -> bool {
        self.shared.ended.swap(false, Ordering::AcqRel)
    }

    /// 取走"播不了"标记。
    pub fn take_failed(&self) -> bool {
        self.shared.failed.swap(false, Ordering::AcqRel)
    }
}
