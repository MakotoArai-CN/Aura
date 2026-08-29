//! 要画的图标，以及图标字体缺席时的替代画法。都不含平台类型。
//!
//! 码位是 Segoe Fluent Icons / Segoe MDL2 Assets 的私有区，只有 Windows 后端用得上；
//! `Fallback` 那套几何形状是三个平台都能画的，字体不在时统一退到它。

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    Prev,
    Next,
    Play,
    Pause,
    Minimize,
    Close,
    FullMode,
    LoopSequence,
    LoopOne,
    Shuffle,
    Volume,
    Muted,
    Note,
}

/// 图标字体缺席时的替代画法。只用矩形和椭圆，不碰路径几何——
/// 这条分支在 Win10 之后基本不会走到，简陋一点比画不出来好。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fallback {
    TriangleRight,
    TriangleLeft,
    TwoBars,
    Bar,
    Cross,
    Frame,
    Dot,
}

impl Icon {
    /// Segoe Fluent Icons / Segoe MDL2 Assets 里的私有区码位，两套字体这些码位一致。
    pub fn glyph(self) -> char {
        match self {
            Icon::Prev => '\u{E100}',
            Icon::Next => '\u{E101}',
            Icon::Play => '\u{E102}',
            Icon::Pause => '\u{E103}',
            Icon::Minimize => '\u{E921}',
            Icon::Close => '\u{E8BB}',
            Icon::FullMode => '\u{E740}',
            Icon::LoopSequence => '\u{E1CD}',
            Icon::LoopOne => '\u{E1CC}',
            Icon::Shuffle => '\u{E14B}',
            Icon::Volume => '\u{E767}',
            Icon::Muted => '\u{E74F}',
            Icon::Note => '\u{E8D6}',
        }
    }

    pub fn fallback(self) -> Fallback {
        match self {
            Icon::Play | Icon::Next | Icon::Volume | Icon::Muted => Fallback::TriangleRight,
            Icon::Prev => Fallback::TriangleLeft,
            Icon::Pause => Fallback::TwoBars,
            Icon::Minimize => Fallback::Bar,
            Icon::Close => Fallback::Cross,
            Icon::FullMode | Icon::LoopSequence | Icon::LoopOne | Icon::Shuffle => Fallback::Frame,
            Icon::Note => Fallback::Dot,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [Icon; 13] = [
        Icon::Prev,
        Icon::Next,
        Icon::Play,
        Icon::Pause,
        Icon::Minimize,
        Icon::Close,
        Icon::FullMode,
        Icon::LoopSequence,
        Icon::LoopOne,
        Icon::Shuffle,
        Icon::Volume,
        Icon::Muted,
        Icon::Note,
    ];

    #[test]
    fn every_icon_has_a_private_use_glyph_and_a_fallback() {
        for icon in ALL {
            let glyph = icon.glyph() as u32;
            assert!(
                (0xE000..=0xF8FF).contains(&glyph),
                "{icon:?} 的码位 {glyph:#X} 不在私有区里，多半是抄错了"
            );
            // fallback 必须能取到，不能 panic
            let _ = icon.fallback();
        }
    }

    #[test]
    fn glyphs_are_distinct() {
        let mut seen = Vec::new();
        for icon in ALL {
            let glyph = icon.glyph();
            assert!(!seen.contains(&glyph), "{icon:?} 的码位和前面某个图标撞了");
            seen.push(glyph);
        }
    }
}
