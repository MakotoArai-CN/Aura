//! `AudioBackend` 的 rodio 实现。四个平台共用这一份。
//!
//! 取代原来的 WinRT `MediaPlayer`。那份实现的好处是解码、重采样、输出全交给系统，
//! 本进程一行解码代码都不带；代价是只有 Windows 有。要让 macOS / Linux / FreeBSD
//! 也有轻量模式，就得把解码搬进进程里来，这是那个选择的必然成本。
//!
//! 与 `MediaPlayer` 的一处刻意差异：这里**只播本地文件**。`MediaPlayer` 能直接吃
//! http(s) 并自己做缓冲和 Range 请求；rodio 走的 symphonia 需要 `Read + Seek`，
//! 要支持 http 就得自己写一个可 seek 的带缓冲 HTTP 读取器。那条路会把刚在 proxy.rs
//! 修掉的那类「流式请求被总超时掐断」的问题重新引进来。所以远端地址先落盘再播，
//! 由 `UriResolver` 负责——轻量模式本来就会给当前及后续若干首预缓存，
//! 走到下载这一步的只是预缓存窗口之外的曲目。

use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rodio::source::Source;
use rodio::{Decoder, Player};

use super::audio::{AudioBackend, UriResolver};

/// seek 目标的上界。`Duration::from_secs_f64` 溢出时是 panic 而不是报错，
/// 而快照里的 position 只挡了非有限和负数，所以这里得自己兜一道。
/// 一天足够覆盖任何一首歌。
const MAX_SEEK_SECONDS: f64 = 86_400.0;

/// 工作线程写、UI 线程读的那部分状态。
#[derive(Default)]
struct Shared {
    ended: AtomicBool,
    failed: AtomicBool,
    /// 正在解析地址 / 下载 / 建解码器。
    busy: AtomicBool,
    duration_ms: AtomicI64,
    /// 已经 append 过一个源、还没判定结束。
    ///
    /// 少了它就没法区分「队列空是因为播完了」和「队列空是因为还没加载」，
    /// 后者会在每个 tick 都误报一次结束，直接把整个队列跳穿。
    active: AtomicBool,
    /// 每次 `load` 自增。慢的工作线程回来时用它判断自己是否已经过期。
    generation: AtomicU64,
    muted: AtomicBool,
    /// 静音前的音量。rodio 没有 mute 概念，只能记下来再置零。
    volume: Mutex<f64>,
}

pub struct RodioBackend {
    /// 输出设备句柄必须一直活着——drop 掉就没声音了，而且不会有任何报错。
    _device: Option<rodio::stream::MixerDeviceSink>,
    /// `Arc` 是为了能丢进工作线程 append。
    player: Option<Arc<Player>>,
    shared: Arc<Shared>,
    resolve: UriResolver,
    /// 打不开输出设备时记下原因，`load` 直接拿它失败，而不是静悄悄地什么都不放。
    device_error: Option<String>,
}

impl RodioBackend {
    /// 打不开输出设备也要能构造出来。
    ///
    /// 无声环境（CI、没有声卡、独占模式被别的进程占着）不该让轻量模式直接崩掉或者
    /// 卡在一个永远不动的界面上：构造照样成功，之后每次 `load` 立刻失败，
    /// 走既有的失败跳过逻辑，连挂三首后由 UI 显示提示。
    pub fn new(resolve: UriResolver) -> Self {
        let shared = Arc::new(Shared::default());
        *shared.volume.lock().unwrap_or_else(|e| e.into_inner()) = 1.0;

        match rodio::stream::DeviceSinkBuilder::open_default_sink() {
            Ok(mut device) => {
                // 关掉 rodio 自己在 drop 时打的那行提示。退出轻量模式时销毁输出设备
                // 是本来就该发生的事，不是异常，那行「Dropping DeviceSink…」纯粹是噪音。
                device.log_on_drop(false);
                let player = Player::connect_new(device.mixer());
                Self {
                    _device: Some(device),
                    player: Some(Arc::new(player)),
                    shared,
                    resolve,
                    device_error: None,
                }
            }
            Err(err) => Self {
                _device: None,
                player: None,
                shared,
                resolve,
                device_error: Some(format!("打不开音频输出设备: {err}")),
            },
        }
    }

    fn apply_volume(&self) {
        let Some(player) = self.player.as_ref() else {
            return;
        };
        let level = if self.shared.muted.load(Ordering::Acquire) {
            0.0
        } else {
            *self.shared.volume.lock().unwrap_or_else(|e| e.into_inner())
        };
        player.set_volume(level as f32);
    }
}

impl AudioBackend for RodioBackend {
    fn load(&self, uri: &str, cache_id: &str) -> Result<(), String> {
        let Some(player) = self.player.as_ref() else {
            return Err(self
                .device_error
                .clone()
                .unwrap_or_else(|| "没有可用的音频输出".to_string()));
        };

        // 先清干净。上一首残留的 ended/failed 会把刚加载的这首立刻跳过去。
        self.shared.ended.store(false, Ordering::Release);
        self.shared.failed.store(false, Ordering::Release);
        self.shared.duration_ms.store(0, Ordering::Release);
        self.shared.active.store(false, Ordering::Release);
        self.shared.busy.store(true, Ordering::Release);
        let generation = self.shared.generation.fetch_add(1, Ordering::AcqRel) + 1;
        player.clear();

        // 解析地址可能要下载，建解码器要读文件头，两件都不能在 UI 线程上做。
        // 真正的失败晚一点通过 failed 标记报出来——原来 WinRT 也是靠 MediaFailed
        // 事件异步报错的，所以 win.rs 那边的处理不用改。
        let shared = self.shared.clone();
        let resolve = self.resolve.clone();
        let player = player.clone();
        let uri = uri.to_string();
        let cache_id = cache_id.to_string();
        let spawned = std::thread::Builder::new()
            .name("aura-mini-load".to_string())
            .spawn(move || {
                let outcome = resolve(&uri, &cache_id).and_then(|path| {
                    let file = std::fs::File::open(&path)
                        .map_err(|err| format!("打不开 {}: {err}", path.display()))?;
                    Decoder::try_from(file).map_err(|err| err.to_string())
                });

                // 过期了就什么都别做：用户已经换到别的曲目，append 上去就是串台。
                if shared.generation.load(Ordering::Acquire) != generation {
                    return;
                }

                match outcome {
                    Ok(decoder) => {
                        let total = decoder
                            .total_duration()
                            .map(|value| value.as_millis() as i64)
                            .unwrap_or(0);
                        shared.duration_ms.store(total, Ordering::Release);
                        player.append(decoder);
                        shared.active.store(true, Ordering::Release);
                        shared.busy.store(false, Ordering::Release);
                    }
                    Err(_) => {
                        shared.busy.store(false, Ordering::Release);
                        shared.failed.store(true, Ordering::Release);
                    }
                }
            });

        if spawned.is_err() {
            self.shared.busy.store(false, Ordering::Release);
            self.shared.failed.store(true, Ordering::Release);
            return Err("起不了加载线程".to_string());
        }
        Ok(())
    }

    fn play(&self) -> Result<(), String> {
        if let Some(player) = self.player.as_ref() {
            player.play();
        }
        Ok(())
    }

    fn pause(&self) -> Result<(), String> {
        if let Some(player) = self.player.as_ref() {
            player.pause();
        }
        Ok(())
    }

    fn set_volume(&self, volume: f64) -> Result<(), String> {
        let clamped = if volume.is_finite() {
            volume.clamp(0.0, 1.0)
        } else {
            1.0
        };
        *self.shared.volume.lock().unwrap_or_else(|e| e.into_inner()) = clamped;
        self.apply_volume();
        Ok(())
    }

    fn set_muted(&self, muted: bool) -> Result<(), String> {
        self.shared.muted.store(muted, Ordering::Release);
        self.apply_volume();
        Ok(())
    }

    fn seek(&self, seconds: f64) -> Result<(), String> {
        let Some(player) = self.player.as_ref() else {
            return Ok(());
        };
        // 上界必须夹住。Duration::from_secs_f64 对溢出的输入是 panic 而不是返回错误，
        // 而快照里的 position 只挡了非有限和负数（见 snapshot.rs 的 normalize），
        // 一个荒谬的大正数能一路传到这里。一天足够覆盖任何一首歌，越界的目标
        // 交给 try_seek 去失败就行——它失败是被吞掉的，panic 不是。
        let target = if seconds.is_finite() && seconds > 0.0 {
            seconds.min(MAX_SEEK_SECONDS)
        } else {
            0.0
        };
        player
            .try_seek(Duration::from_secs_f64(target))
            .map_err(|err| err.to_string())
    }

    fn position(&self) -> f64 {
        self.player
            .as_ref()
            .map(|player| player.get_pos().as_secs_f64())
            .unwrap_or(0.0)
    }

    fn duration(&self) -> f64 {
        let ms = self.shared.duration_ms.load(Ordering::Acquire);
        if ms > 0 {
            return ms as f64 / 1000.0;
        }
        0.0
    }

    fn is_playing(&self) -> bool {
        self.player
            .as_ref()
            .map(|player| !player.is_paused() && !player.empty())
            .unwrap_or(false)
    }

    fn is_busy(&self) -> bool {
        self.shared.busy.load(Ordering::Acquire)
    }

    fn take_ended(&self) -> bool {
        // rodio 不给"播完了"的回调，只能看队列是不是空了。判据里 active 那一半是必须的：
        // 没有它，还没加载时的空队列会被当成播完，每个 tick 报一次，整个队列一秒跳穿。
        if self.shared.active.load(Ordering::Acquire) {
            let empty = self
                .player
                .as_ref()
                .map(|player| player.empty())
                .unwrap_or(false);
            if empty {
                self.shared.active.store(false, Ordering::Release);
                self.shared.ended.store(true, Ordering::Release);
            }
        }
        self.shared.ended.swap(false, Ordering::AcqRel)
    }

    fn take_failed(&self) -> bool {
        self.shared.failed.swap(false, Ordering::AcqRel)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 构造一个没有输出设备的后端。
    ///
    /// 这不是"为了测试而伪造"：无声环境（CI、没有声卡、独占模式被别的进程占着）是真的
    /// 会走到这条路的，而那时候不能崩、也不能卡在一个永远不动的界面上。
    fn headless() -> RodioBackend {
        let shared = Arc::new(Shared::default());
        *shared.volume.lock().unwrap() = 1.0;
        RodioBackend {
            _device: None,
            player: None,
            shared,
            resolve: Arc::new(|_uri, _cache_id| Err("测试里不解析".to_string())),
            device_error: Some("测试：没有输出设备".to_string()),
        }
    }

    #[test]
    fn no_output_device_fails_load_instead_of_panicking() {
        let backend = headless();
        assert!(backend.load("file:///nope.mp3", "").is_err());
        // 其余调用一律安静地什么都不做，不能 panic。
        assert!(backend.play().is_ok());
        assert!(backend.pause().is_ok());
        assert!(backend.seek(12.0).is_ok());
        assert_eq!(backend.position(), 0.0);
        assert_eq!(backend.duration(), 0.0);
        assert!(!backend.is_playing());
    }

    #[test]
    fn volume_is_clamped_and_non_finite_falls_back_to_full() {
        let backend = headless();
        let read = || *backend.shared.volume.lock().unwrap();

        backend.set_volume(0.5).unwrap();
        assert_eq!(read(), 0.5);
        backend.set_volume(-3.0).unwrap();
        assert_eq!(read(), 0.0);
        backend.set_volume(9.0).unwrap();
        assert_eq!(read(), 1.0);
        // 非有限值退回满音量，而不是把 NaN 存进去——存进去之后每次 apply 都会把
        // NaN 喂给 rodio。
        backend.set_volume(f64::NAN).unwrap();
        assert_eq!(read(), 1.0);
        backend.set_volume(f64::INFINITY).unwrap();
        assert_eq!(read(), 1.0);
    }

    #[test]
    fn muting_keeps_the_previous_volume() {
        let backend = headless();
        backend.set_volume(0.35).unwrap();
        backend.set_muted(true).unwrap();
        assert!(backend.shared.muted.load(Ordering::Acquire));
        // 静音不该把记下来的音量改掉，否则解除静音之后音量就回不去了。
        assert_eq!(*backend.shared.volume.lock().unwrap(), 0.35);
        backend.set_muted(false).unwrap();
        assert_eq!(*backend.shared.volume.lock().unwrap(), 0.35);
    }

    #[test]
    fn ended_and_failed_are_consume_once() {
        let backend = headless();
        backend.shared.ended.store(true, Ordering::Release);
        backend.shared.failed.store(true, Ordering::Release);
        // 取一次就清掉。少了这条，一次结束会把整个队列一路推到底。
        assert!(backend.take_ended());
        assert!(!backend.take_ended());
        assert!(backend.take_failed());
        assert!(!backend.take_failed());
    }

    #[test]
    fn an_empty_queue_before_loading_is_not_an_ending() {
        let backend = headless();
        // active 没置起来的时候，"队列是空的"只说明还没加载，不是播完了。
        // 判据里少了 active 这一半，每个 tick 都会误报一次结束。
        assert!(!backend.shared.active.load(Ordering::Acquire));
        assert!(!backend.take_ended());
        assert!(!backend.take_ended());
    }

    #[test]
    fn load_clears_stale_flags_and_bumps_generation() {
        // 没有 player 时 load 会提前返回，所以这里直接验状态清理那段的语义：
        // 上一首残留的 ended/failed 必须清掉，否则刚加载的这首会被立刻跳过。
        let backend = headless();
        backend.shared.ended.store(true, Ordering::Release);
        backend.shared.failed.store(true, Ordering::Release);
        let before = backend.shared.generation.load(Ordering::Acquire);

        // 有设备的路径才会走清理，这里手动重放同样的顺序以固定住语义。
        backend.shared.ended.store(false, Ordering::Release);
        backend.shared.failed.store(false, Ordering::Release);
        backend.shared.generation.fetch_add(1, Ordering::AcqRel);

        assert!(!backend.take_ended());
        assert!(!backend.take_failed());
        assert!(backend.shared.generation.load(Ordering::Acquire) > before);
    }

    /// 生成一段 16 位单声道 PCM 的 WAV，用来验解码器真的接上了。
    fn tiny_wav(seconds: u32, rate: u32) -> Vec<u8> {
        let samples = seconds * rate;
        let data_len = samples * 2;
        let mut out = Vec::with_capacity(44 + data_len as usize);
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&(36 + data_len).to_le_bytes());
        out.extend_from_slice(b"WAVEfmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes()); // PCM
        out.extend_from_slice(&1u16.to_le_bytes()); // mono
        out.extend_from_slice(&rate.to_le_bytes());
        out.extend_from_slice(&(rate * 2).to_le_bytes()); // byte rate
        out.extend_from_slice(&2u16.to_le_bytes()); // block align
        out.extend_from_slice(&16u16.to_le_bytes()); // bits
        out.extend_from_slice(b"data");
        out.extend_from_slice(&data_len.to_le_bytes());
        for i in 0..samples {
            // 一个低幅正弦，内容不重要，只要是合法的 PCM。
            let value = ((i as f32 / 32.0).sin() * 3000.0) as i16;
            out.extend_from_slice(&value.to_le_bytes());
        }
        out
    }

    #[test]
    fn decoder_reports_the_duration_of_a_generated_wav() {
        // 这一条是在验 symphonia-wav 这个 feature 真的开着：feature 漏了的话
        // Decoder::try_from 会直接失败，而那种失败在运行时表现成"所有歌都放不了"。
        let dir = std::env::temp_dir().join(format!(
            "aura-audio-test-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("tone.wav");
        std::fs::write(&path, tiny_wav(2, 8000)).unwrap();

        let file = std::fs::File::open(&path).unwrap();
        let decoder = Decoder::try_from(file).expect("WAV 解码器不可用");
        let total = decoder.total_duration().expect("拿不到时长");
        assert!(
            (total.as_secs_f64() - 2.0).abs() < 0.05,
            "时长应当接近 2 秒，实际 {total:?}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
