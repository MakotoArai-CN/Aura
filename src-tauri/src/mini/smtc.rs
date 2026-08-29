//! Windows 的系统媒体控制（SMTC）。
//!
//! 原来这一套是 WinRT `MediaPlayer` 免费带来的：媒体键、音量 OSD、锁屏上的曲目信息，
//! 都是它自动接的。换成自己解码之后一个都不剩，所以必须在这里补回来——否则「换个
//! 解码器」对用户来说就是「媒体键失灵了」。
//!
//! `SystemMediaTransportControls` 平时是从 `MediaPlayer` 上取的。没有 `MediaPlayer`
//! 的情况下走 `ISystemMediaTransportControlsInterop::GetForWindow`，拿原生窗口的
//! HWND 换一个出来——这也是所有非 UWP 桌面程序接 SMTC 的正规路子。

use windows::core::{factory, Ref, HSTRING};
use windows::Foundation::{TypedEventHandler, Uri};
use windows::Media::{
    MediaPlaybackStatus, MediaPlaybackType, SystemMediaTransportControls,
    SystemMediaTransportControlsButton, SystemMediaTransportControlsButtonPressedEventArgs,
};
use windows::Storage::Streams::RandomAccessStreamReference;
use windows::Win32::Foundation::HWND;
use windows::Win32::System::WinRT::ISystemMediaTransportControlsInterop;

use super::shared::{MediaControls, NowPlaying};
use super::LiteAction;

pub struct Smtc {
    controls: SystemMediaTransportControls,
}

impl Smtc {
    pub fn new(hwnd: HWND) -> windows::core::Result<Self> {
        let interop: ISystemMediaTransportControlsInterop =
            factory::<SystemMediaTransportControls, ISystemMediaTransportControlsInterop>()?;
        // GetForWindow 是 unsafe 的：它要求 hwnd 是本进程里活着的窗口。
        // 这里传进来的是原生迷你窗口刚创建出来的句柄，调用方保证它有效。
        let controls: SystemMediaTransportControls = unsafe { interop.GetForWindow(hwnd)? };

        controls.SetIsEnabled(true)?;
        controls.SetIsPlayEnabled(true)?;
        controls.SetIsPauseEnabled(true)?;
        controls.SetIsNextEnabled(true)?;
        controls.SetIsPreviousEnabled(true)?;
        // 停止按钮不开：轻量模式没有"停止"这个状态，暂停已经够用，
        // 开了反而会在系统面板上多出一个按下去没有明确结果的键。
        controls.SetIsStopEnabled(false)?;

        // 按键事件在系统线程上回调。这里不碰任何 UI 状态，只把动作转成一条窗口消息，
        // 由窗口线程去执行——`Ui` 不是线程安全的，这条路和托盘菜单走的是同一个入口。
        controls.ButtonPressed(&TypedEventHandler::new(
            move |_, args: Ref<'_, SystemMediaTransportControlsButtonPressedEventArgs>| {
                let Some(args) = args.as_ref() else {
                    return Ok(());
                };
                let Ok(button) = args.Button() else {
                    return Ok(());
                };
                // 用 if 链而不是 match：这些是 newtype 上的关联常量，不是枚举变体，
                // 拿来做 match 模式要求类型满足结构化匹配，windows crate 的类型不保证。
                let action = if button == SystemMediaTransportControlsButton::Play
                    || button == SystemMediaTransportControlsButton::Pause
                {
                    Some(LiteAction::PlayPause)
                } else if button == SystemMediaTransportControlsButton::Next {
                    Some(LiteAction::Next)
                } else if button == SystemMediaTransportControlsButton::Previous {
                    Some(LiteAction::Prev)
                } else {
                    None
                };
                if let Some(action) = action {
                    crate::mini::lite_action(action);
                }
                Ok(())
            },
        ))?;

        Ok(Self { controls })
    }

    fn push(&self, info: &NowPlaying<'_>) -> windows::core::Result<()> {
        let updater = self.controls.DisplayUpdater()?;
        updater.SetType(MediaPlaybackType::Music)?;
        let music = updater.MusicProperties()?;
        music.SetTitle(&HSTRING::from(info.title))?;
        music.SetArtist(&HSTRING::from(info.artist))?;
        music.SetAlbumTitle(&HSTRING::from(info.album))?;

        if let Some(path) = info.cover_path.filter(|value| !value.trim().is_empty()) {
            // 走 CreateFromUri 而不是 StorageFile::GetFileFromPathAsync：后者是异步的，
            // 在这条同步路径上只能 .get() 阻塞等待，而这里是窗口线程。
            let uri = Uri::CreateUri(&HSTRING::from(super::file_uri(path)))?;
            let thumb = RandomAccessStreamReference::CreateFromUri(&uri)?;
            updater.SetThumbnail(&thumb)?;
        }

        updater.Update()
    }
}

impl MediaControls for Smtc {
    fn set_now_playing(&self, info: &NowPlaying<'_>) {
        // 系统面板刷不出来不是能让播放停下的理由，出错就算了。
        let _ = self.push(info);
    }

    fn set_playing(&self, playing: bool) {
        let status = if playing {
            MediaPlaybackStatus::Playing
        } else {
            MediaPlaybackStatus::Paused
        };
        let _ = self.controls.SetPlaybackStatus(status);
    }

    fn clear(&self) {
        // 退出轻量模式时收摊，别在系统里留一条永远停在那首歌的记录。
        let _ = self.controls.SetPlaybackStatus(MediaPlaybackStatus::Closed);
        let _ = self.controls.SetIsEnabled(false);
        if let Ok(updater) = self.controls.DisplayUpdater() {
            let _ = updater.ClearAll();
        }
    }
}
