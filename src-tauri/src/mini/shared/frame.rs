//! 每帧的重绘判定与鼠标拖拽状态。都是纯数据，三个后端共用。

/// 鼠标正在拖什么。
///
/// 窗口本身的拖动不在这里：那个交给各平台的标题栏命中测试（Windows 上是
/// `WM_NCHITTEST` 返回 `HTCAPTION`），这样能白拿系统的贴边和动画，不用自己算偏移。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Drag {
    #[default]
    None,
    Progress,
    Volume,
}

/// 决定"这一帧和上一帧比有没有变化"。只有它变了才请求重绘——
/// 轻量模式要是无条件每秒重画十次，那就白轻量了。
///
/// 时间取四分之一秒的整数份：进度条画不出更细的差别，按秒又会让秒数跳变时慢半拍。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PaintKey {
    pub quarter: i64,
    pub duration: i64,
    pub playing: bool,
    pub busy: bool,
    pub index: i64,
    pub lyric: i64,
    pub volume: i64,
    pub muted: bool,
    pub loop_mode: u8,
    pub scroll: i32,
    pub sliding: bool,
    pub notice: bool,
}

impl PaintKey {
    /// 把连续量量化成整数份，浮点抖动就不会每帧都判定成"变了"。
    pub fn quantize_seconds(seconds: f64) -> i64 {
        if !seconds.is_finite() {
            return 0;
        }
        (seconds * 4.0) as i64
    }

    pub fn quantize_volume(volume: f64) -> i64 {
        if !volume.is_finite() {
            return 0;
        }
        (volume * 100.0) as i64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quantize_ignores_differences_finer_than_a_quarter_second() {
        assert_eq!(PaintKey::quantize_seconds(1.0), PaintKey::quantize_seconds(1.2));
        assert_ne!(PaintKey::quantize_seconds(1.0), PaintKey::quantize_seconds(1.3));
    }

    #[test]
    fn quantize_survives_nonfinite_input() {
        // 源没打开时 duration 是 NaN，`as i64` 出来的是垃圾，会让每帧都判定成变了。
        assert_eq!(PaintKey::quantize_seconds(f64::NAN), 0);
        assert_eq!(PaintKey::quantize_seconds(f64::INFINITY), 0);
        assert_eq!(PaintKey::quantize_volume(f64::NAN), 0);
    }

    #[test]
    fn default_key_differs_from_a_playing_one() {
        let idle = PaintKey::default();
        let playing = PaintKey {
            playing: true,
            ..PaintKey::default()
        };
        assert_ne!(idle, playing);
    }

    #[test]
    fn drag_defaults_to_none() {
        assert_eq!(Drag::default(), Drag::None);
    }
}
