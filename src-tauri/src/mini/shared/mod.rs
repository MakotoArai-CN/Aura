//! 轻量模式里与平台无关的那一半。
//!
//! 轻量模式在每个桌面平台上都是原生绘制的（Windows: Direct2D/DirectWrite，
//! macOS: CoreGraphics/CoreText，Linux/FreeBSD: Cairo/Pango），但只有"怎么画"和
//! "怎么出声"是各平台不同的。布局坐标、命中测试、换曲与循环、洗牌、失败退避、
//! 歌词时间轴与滚动、重绘判定，这些四个平台一模一样，全部放在这里。
//!
//! 这个模块是**无条件编译**的，不带 `cfg(windows)`：它必须能在四个目标上都过编译，
//! 否则新后端一开工就得先改这里。硬性约束是它不引用任何平台 crate，也不直接调用
//! 音频实现——播放状态机通过返回 [`state::Action`] 把要做的事交给调用方。

pub mod audio;
pub mod format;
pub mod frame;
pub mod icon;
pub mod lyrics;
pub mod layout;
pub mod rodio_backend;
pub mod state;

// 只在这里重导出后端真正用得上的那几个。其余的走全路径引用，免得攒出一堆
// 谁都没在用的 re-export。
pub use audio::{AudioBackend, MediaControls, NoopMediaControls, NowPlaying, UriResolver};
pub use format::fmt_time;
pub use frame::{Drag, PaintKey};
pub use icon::{Fallback, Icon};
pub use layout::{Layout, LOGICAL_HEIGHT, LOGICAL_WIDTH};
pub use rodio_backend::RodioBackend;
