//! 轻量模式的布局与命中测试，不含任何平台类型。
//!
//! 绘制和命中测试读同一份 `Layout`，这样就不可能算出两套坐标。各平台后端只负责把
//! 这里的 `Rect` 翻译成自己的矩形类型（Direct2D 的 `D2D_RECT_F`、Cairo 的
//! `rectangle()`、CoreGraphics 的 `CGRect`），几何本身一份就够。

/// 逻辑尺寸（96 DPI 下的像素）。乘上缩放才是真实像素。
pub const LOGICAL_WIDTH: f32 = 420.0;
pub const LOGICAL_HEIGHT: f32 = 620.0;

/// 平台中立的矩形。字段名刻意和 `D2D_RECT_F` 保持一致，Windows 后端可以直接搬。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

pub fn rect(left: f32, top: f32, right: f32, bottom: f32) -> Rect {
    Rect {
        left,
        top,
        right,
        bottom,
    }
}

impl Rect {
    pub fn width(&self) -> f32 {
        self.right - self.left
    }

    pub fn height(&self) -> f32 {
        self.bottom - self.top
    }

    /// 右边界与下边界都取开区间，相邻的两个矩形才不会同时命中。
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.left && x < self.right && y >= self.top && y < self.bottom
    }

    /// 横向命中位置换成 0~1 的比例。进度条和音量条共用。
    ///
    /// 宽度为 0 时返回 0 而不是除出 NaN——NaN 会一路传到 seek 里去。
    pub fn ratio_at(&self, x: f32) -> f64 {
        let width = self.width();
        if width <= 0.0 {
            return 0.0;
        }
        (((x - self.left) / width) as f64).clamp(0.0, 1.0)
    }
}

/// 一次算好、绘制和命中测试共用的布局。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Layout {
    pub scale: f32,
    pub width: f32,
    pub height: f32,
    pub title_bar: Rect,
    pub btn_full: Rect,
    pub btn_min: Rect,
    pub btn_close: Rect,
    pub cover: Rect,
    pub title: Rect,
    pub subtitle: Rect,
    pub progress: Rect,
    pub progress_hit: Rect,
    pub time_row: Rect,
    pub btn_loop: Rect,
    pub btn_prev: Rect,
    pub btn_play: Rect,
    pub btn_next: Rect,
    pub volume_icon: Rect,
    pub volume_slider: Rect,
    pub volume_hit: Rect,
    pub lyrics: Rect,
    pub queue: Rect,
    pub row_height: f32,
}

impl Layout {
    pub fn compute(scale: f32) -> Self {
        let s = |value: f32| value * scale;
        Self {
            scale,
            width: s(LOGICAL_WIDTH),
            height: s(LOGICAL_HEIGHT),
            title_bar: rect(0.0, 0.0, s(420.0), s(36.0)),
            btn_full: rect(s(312.0), s(4.0), s(340.0), s(32.0)),
            btn_min: rect(s(348.0), s(4.0), s(376.0), s(32.0)),
            btn_close: rect(s(384.0), s(4.0), s(412.0), s(32.0)),
            cover: rect(s(120.0), s(48.0), s(300.0), s(228.0)),
            title: rect(s(24.0), s(240.0), s(396.0), s(264.0)),
            subtitle: rect(s(24.0), s(266.0), s(396.0), s(286.0)),
            progress: rect(s(24.0), s(304.0), s(396.0), s(308.0)),
            progress_hit: rect(s(24.0), s(296.0), s(396.0), s(316.0)),
            time_row: rect(s(24.0), s(314.0), s(396.0), s(330.0)),
            btn_loop: rect(s(24.0), s(340.0), s(48.0), s(364.0)),
            btn_prev: rect(s(136.0), s(336.0), s(168.0), s(368.0)),
            btn_play: rect(s(188.0), s(330.0), s(232.0), s(374.0)),
            btn_next: rect(s(252.0), s(336.0), s(284.0), s(368.0)),
            volume_icon: rect(s(312.0), s(343.0), s(330.0), s(361.0)),
            volume_slider: rect(s(336.0), s(350.0), s(396.0), s(354.0)),
            volume_hit: rect(s(330.0), s(342.0), s(400.0), s(362.0)),
            lyrics: rect(s(24.0), s(390.0), s(396.0), s(510.0)),
            queue: rect(s(12.0), s(520.0), s(408.0), s(612.0)),
            row_height: s(26.0),
        }
    }

    /// 进度条上的横向位置换成秒。时长未知（还没打开或流没有长度）时返回 0——
    /// 不能拿 0 时长去乘比例，那样拖到哪里都是 0，不如明确不动。
    pub fn position_from(&self, x: f32, duration: f64) -> f64 {
        if duration <= 0.0 {
            return 0.0;
        }
        self.progress.ratio_at(x) * duration
    }

    /// 队列里点到了第几行。顶部留了一行标题的高度。
    ///
    /// `row_height` 为 0 时直接返回 None，否则下面那个除法会得到 inf。
    pub fn queue_index_at(&self, y: f32, scroll: f32, count: i64) -> Option<i64> {
        let row = self.row_height;
        let top = self.queue.top + row;
        if row <= 0.0 || y < top {
            return None;
        }
        let index = ((y - top + scroll) / row).floor() as i64;
        if index >= 0 && index < count {
            Some(index)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contains_is_half_open_so_neighbours_do_not_both_hit() {
        let area = rect(10.0, 10.0, 20.0, 20.0);
        assert!(area.contains(10.0, 10.0), "左上角属于自己");
        assert!(!area.contains(20.0, 15.0), "右边界属于下一个矩形");
        assert!(!area.contains(15.0, 20.0), "下边界属于下一个矩形");
        assert!(!area.contains(9.99, 15.0));
    }

    #[test]
    fn ratio_at_clamps_and_never_divides_by_zero() {
        let area = rect(100.0, 0.0, 200.0, 10.0);
        assert_eq!(area.ratio_at(100.0), 0.0);
        assert_eq!(area.ratio_at(150.0), 0.5);
        assert_eq!(area.ratio_at(200.0), 1.0);
        assert_eq!(area.ratio_at(-500.0), 0.0, "左边界外夹到 0");
        assert_eq!(area.ratio_at(9999.0), 1.0, "右边界外夹到 1");

        // 宽度为 0：必须是 0 而不是 NaN，NaN 会一路传进 seek。
        let empty = rect(50.0, 0.0, 50.0, 10.0);
        assert_eq!(empty.ratio_at(50.0), 0.0);
        assert!(empty.ratio_at(80.0).is_finite());
    }

    #[test]
    fn position_from_needs_a_known_duration() {
        let layout = Layout::compute(1.0);
        let mid = (layout.progress.left + layout.progress.right) / 2.0;
        assert!((layout.position_from(mid, 200.0) - 100.0).abs() < 0.001);
        assert_eq!(layout.position_from(mid, 0.0), 0.0, "时长未知时不要瞎猜");
        assert_eq!(layout.position_from(mid, -5.0), 0.0);
    }

    #[test]
    fn queue_index_at_boundaries() {
        let layout = Layout::compute(1.0);
        let row = layout.row_height;
        let first_row_top = layout.queue.top + row;

        assert_eq!(layout.queue_index_at(first_row_top - 0.1, 0.0, 10), None, "标题行不算");
        assert_eq!(layout.queue_index_at(first_row_top, 0.0, 10), Some(0));
        assert_eq!(layout.queue_index_at(first_row_top + row, 0.0, 10), Some(1));
        assert_eq!(layout.queue_index_at(first_row_top + row * 9.5, 0.0, 10), Some(9));
        assert_eq!(layout.queue_index_at(first_row_top + row * 10.0, 0.0, 10), None, "越过最后一首");
        assert_eq!(layout.queue_index_at(first_row_top, 0.0, 0), None, "空队列");
        // 滚动之后同一个 y 落在更后面的行上
        assert_eq!(layout.queue_index_at(first_row_top, row * 3.0, 10), Some(3));
    }

    #[test]
    fn queue_index_at_with_zero_row_height_does_not_divide_by_zero() {
        let layout = Layout::compute(0.0);
        assert_eq!(layout.row_height, 0.0);
        assert_eq!(layout.queue_index_at(0.0, 0.0, 10), None);
        assert_eq!(layout.queue_index_at(100.0, 0.0, 10), None);
    }

    #[test]
    fn compute_scales_every_rect_uniformly() {
        let one = Layout::compute(1.0);
        let two = Layout::compute(2.0);
        assert_eq!(two.width, one.width * 2.0);
        assert_eq!(two.height, one.height * 2.0);
        assert_eq!(two.cover.left, one.cover.left * 2.0);
        assert_eq!(two.row_height, one.row_height * 2.0);
        assert_eq!(one.width, LOGICAL_WIDTH);
        assert_eq!(one.height, LOGICAL_HEIGHT);
    }
}
