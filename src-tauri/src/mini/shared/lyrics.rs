//! LRC 解析与歌词轨的滚动状态。轻量模式没有 JS，这两件事都得在 Rust 侧自己做。
//!
//! 快照里存的是 LRC 原文（前端抓下来直接塞进去的），格式上要容忍现实里的脏数据：
//! 时间戳可能是两位或三位毫秒、一行可能挂多个时间戳（副歌复用）、开头可能有
//! `[ti:]`/`[ar:]`/`[al:]`/`[by:]`/`[offset:]` 这类元信息标签，顺序也不保证递增。
//!
//! 不含任何平台类型，三个后端共用。

/// 一行歌词。`translation` 由 `merge_translation` 从翻译轨对齐过来。
#[derive(Debug, Clone, PartialEq)]
pub struct LyricLine {
    pub seconds: f64,
    pub text: String,
    pub translation: Option<String>,
}

/// 翻译对齐的容差。两轨时间戳一般完全一致，留 300ms 是为了容忍手工轴的抖动。
const TRANSLATION_TOLERANCE: f64 = 0.3;

/// 把 `[mm:ss]` / `[mm:ss.xx]` / `[mm:ss.xxx]` 解析成秒。不是时间戳就返回 None，
/// 元信息标签正是靠这个被过滤掉的（`ti` 不是数字）。
fn parse_timestamp(body: &str) -> Option<f64> {
    let (minutes, rest) = body.split_once(':')?;
    let minutes: f64 = minutes.trim().parse().ok()?;
    if minutes < 0.0 {
        return None;
    }

    // 秒部分允许 `ss`、`ss.xx`、`ss.xxx`，也容忍 `ss:xxx` 这种少见写法。
    let rest = rest.trim();
    let (seconds, fraction) = match rest.split_once(['.', ':']) {
        Some((seconds, fraction)) => (seconds, Some(fraction)),
        None => (rest, None),
    };
    let seconds: f64 = seconds.parse().ok()?;
    if seconds < 0.0 {
        return None;
    }

    let fraction = match fraction {
        None => 0.0,
        Some(digits) => {
            if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
                return None;
            }
            let value: f64 = digits.parse().ok()?;
            value / 10f64.powi(digits.len() as i32)
        }
    };

    Some(minutes * 60.0 + seconds + fraction)
}

/// 解析一整段 LRC。返回按时间升序排好的行，无时间戳的行直接丢掉。
pub fn parse(raw: &str) -> Vec<LyricLine> {
    let mut lines: Vec<LyricLine> = Vec::new();

    for source in raw.lines() {
        let mut rest = source.trim_start();
        let mut stamps: Vec<f64> = Vec::new();

        // 只吃开头连续的 `[...]`：正文里出现的方括号不该被当成时间戳。
        while let Some(body) = rest.strip_prefix('[') {
            let Some(end) = body.find(']') else { break };
            let (inner, after) = body.split_at(end);
            if let Some(seconds) = parse_timestamp(inner) {
                stamps.push(seconds);
            }
            rest = after[1..].trim_start();
        }

        if stamps.is_empty() {
            continue;
        }
        let text = rest.trim().to_string();
        for seconds in stamps {
            lines.push(LyricLine {
                seconds,
                text: text.clone(),
                translation: None,
            });
        }
    }

    lines.sort_by(|left, right| left.seconds.total_cmp(&right.seconds));
    lines
}

/// 把翻译轨对齐到主轨。每条翻译找时间最近的主行，超出容差就丢掉；
/// 一行主歌词只接第一条命中的翻译，避免重复轴把它反复覆盖。
pub fn merge_translation(main: Vec<LyricLine>, translation: &[LyricLine]) -> Vec<LyricLine> {
    let mut merged = main;
    if merged.is_empty() {
        return merged;
    }

    for candidate in translation {
        if candidate.text.is_empty() {
            continue;
        }
        let mut best: Option<(usize, f64)> = None;
        for (index, line) in merged.iter().enumerate() {
            let distance = (line.seconds - candidate.seconds).abs();
            if distance > TRANSLATION_TOLERANCE {
                continue;
            }
            if best.map_or(true, |(_, current)| distance < current) {
                best = Some((index, distance));
            }
        }
        if let Some((index, _)) = best {
            if merged[index].translation.is_none() {
                merged[index].translation = Some(candidate.text.clone());
            }
        }
    }

    merged
}

/// 当前该高亮哪一行：时间戳 ≤ position 的最后一行。还没到第一行就返回 None。
pub fn active_index(lines: &[LyricLine], seconds: f64) -> Option<usize> {
    let mut found = None;
    for (index, line) in lines.iter().enumerate() {
        if line.seconds <= seconds {
            found = Some(index);
        } else {
            break;
        }
    }
    found
}

/// 换行滑动的最大幅度。跨很多行时（比如刚 seek 完）不要滑出天际。
const MAX_SHIFT: f32 = 120.0;
/// 每帧向 0 收敛的比例。0.68 在 100ms 的定时器下大约三四帧滑完。
const SHIFT_DECAY: f32 = 0.68;
/// 小于这个就直接归零，免得浮点尾数让它永远"在滑动"、每帧都重画。
const SHIFT_EPSILON: f32 = 0.5;

/// 当前曲目的歌词轨与它的滚动状态。
///
/// 解析结果按曲目下标缓存：`tick` 是 100ms 一次的，每次重新解析整段 LRC 太浪费。
#[derive(Debug, Default)]
pub struct LyricTrack {
    lines: Vec<LyricLine>,
    /// 已经解析过的曲目下标。-1 表示还没解析过任何一首。
    parsed_for: i64,
    /// 当前高亮行，-1 表示还没到第一行。
    active: i64,
    /// 换行时的滑动偏移，每帧向 0 收敛，让歌词是滑过去而不是跳过去。
    shift: f32,
}

impl LyricTrack {
    pub fn new() -> Self {
        Self {
            lines: Vec::new(),
            parsed_for: -1,
            active: -1,
            shift: 0.0,
        }
    }

    pub fn lines(&self) -> &[LyricLine] {
        &self.lines
    }

    pub fn active(&self) -> i64 {
        self.active
    }

    pub fn shift(&self) -> f32 {
        self.shift
    }

    pub fn is_sliding(&self) -> bool {
        self.shift != 0.0
    }

    /// 换曲时调用：丢掉缓存和滚动状态，下一次 `sync` 会重新解析。
    pub fn reset(&mut self) {
        self.parsed_for = -1;
        self.active = -1;
        self.shift = 0.0;
    }

    /// 确保当前解析的是 `index` 这首。已经是了就什么都不做。
    pub fn sync(&mut self, index: i64, lyric: Option<&str>, tlyric: Option<&str>) {
        if self.parsed_for == index {
            return;
        }
        self.parsed_for = index;
        let main = parse(lyric.unwrap_or(""));
        self.lines = match tlyric {
            Some(raw) if !raw.trim().is_empty() => merge_translation(main, &parse(raw)),
            _ => main,
        };
        self.active = -1;
    }

    /// 推进一帧。返回高亮行是否发生了变化。
    ///
    /// 滑动偏移在这里既被设置也被衰减：换行的那一帧按跨过的行数给一个初速度，
    /// 之后每帧乘 `SHIFT_DECAY` 收敛回 0。
    pub fn tick(&mut self, position: f64, row_height: f32) -> bool {
        let active = active_index(&self.lines, position)
            .map(|index| index as i64)
            .unwrap_or(-1);
        let changed = active != self.active;
        if changed {
            // 从上一行的位置滑过来，而不是直接跳。第一次进入歌词区（active 从 -1 起）
            // 不给初速度，否则一开播就会莫名其妙地滑一下。
            if self.active >= 0 {
                let delta = (active - self.active) as f32;
                self.shift = (delta * row_height).clamp(-MAX_SHIFT, MAX_SHIFT);
            }
            self.active = active;
        }
        if self.shift.abs() > SHIFT_EPSILON {
            self.shift *= SHIFT_DECAY;
        } else {
            self.shift = 0.0;
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(lines: &[LyricLine], index: usize) -> (f64, &str) {
        (lines[index].seconds, lines[index].text.as_str())
    }

    #[test]
    fn parses_two_and_three_digit_fractions_and_bare_seconds() {
        let lines = parse("[00:01]a\n[00:02.50]b\n[00:03.125]c");
        assert_eq!(lines.len(), 3);
        assert_eq!(at(&lines, 0), (1.0, "a"));
        assert_eq!(at(&lines, 1), (2.5, "b"));
        assert_eq!(at(&lines, 2), (3.125, "c"));
    }

    #[test]
    fn a_line_with_several_timestamps_becomes_several_lines() {
        let lines = parse("[00:10.00][01:00.00][02:30.00]副歌");
        assert_eq!(lines.len(), 3);
        assert_eq!(at(&lines, 0), (10.0, "副歌"));
        assert_eq!(at(&lines, 1), (60.0, "副歌"));
        assert_eq!(at(&lines, 2), (150.0, "副歌"));
    }

    #[test]
    fn metadata_tags_are_dropped_not_rendered() {
        let lines = parse("[ti:标题]\n[ar:歌手]\n[by:某人]\n[offset:+500]\n[00:05.00]正文");
        assert_eq!(lines.len(), 1);
        assert_eq!(at(&lines, 0), (5.0, "正文"));
    }

    #[test]
    fn out_of_order_input_comes_back_sorted() {
        let lines = parse("[00:30.00]c\n[00:10.00]a\n[00:20.00]b");
        assert_eq!(
            lines.iter().map(|line| line.text.as_str()).collect::<Vec<_>>(),
            vec!["a", "b", "c"]
        );
    }

    #[test]
    fn brackets_inside_the_text_are_left_alone() {
        let lines = parse("[00:01.00]前奏 [笑] 后面");
        assert_eq!(lines.len(), 1);
        assert_eq!(at(&lines, 0), (1.0, "前奏 [笑] 后面"));
    }

    #[test]
    fn lines_without_a_timestamp_are_skipped() {
        assert!(parse("纯文本\n\n[不是时间]也不是").is_empty());
    }

    #[test]
    fn active_index_before_the_first_line_is_none() {
        let lines = parse("[00:05.00]a\n[00:10.00]b");
        assert_eq!(active_index(&lines, 0.0), None);
        assert_eq!(active_index(&lines, 4.999), None);
    }

    #[test]
    fn active_index_is_inclusive_on_the_boundary() {
        let lines = parse("[00:05.00]a\n[00:10.00]b");
        assert_eq!(active_index(&lines, 5.0), Some(0));
        assert_eq!(active_index(&lines, 9.999), Some(0));
        assert_eq!(active_index(&lines, 10.0), Some(1));
    }

    #[test]
    fn active_index_after_the_last_line_stays_on_the_last_line() {
        let lines = parse("[00:05.00]a\n[00:10.00]b");
        assert_eq!(active_index(&lines, 9999.0), Some(1));
        assert_eq!(active_index(&[], 1.0), None);
    }

    #[test]
    fn translation_attaches_within_the_tolerance_and_is_dropped_outside_it() {
        let main = parse("[00:05.00]hello\n[00:10.00]world");
        let translation = parse("[00:05.20]你好\n[00:10.40]世界");
        let merged = merge_translation(main, &translation);
        assert_eq!(merged[0].translation.as_deref(), Some("你好"));
        assert_eq!(merged[1].translation, None);
    }

    #[test]
    fn translation_picks_the_nearest_line_when_several_are_in_range() {
        let main = parse("[00:05.00]a\n[00:05.25]b");
        let translation = parse("[00:05.20]译");
        let merged = merge_translation(main, &translation);
        assert_eq!(merged[0].translation, None);
        assert_eq!(merged[1].translation.as_deref(), Some("译"));
    }

    #[test]
    fn merging_into_an_empty_main_track_yields_nothing() {
        assert!(merge_translation(Vec::new(), &parse("[00:01.00]译")).is_empty());
    }

    #[test]
    fn track_parses_once_per_index() {
        let mut track = LyricTrack::new();
        track.sync(0, Some("[00:01.00]a"), None);
        assert_eq!(track.lines().len(), 1);
        // 同一个下标再 sync 一次不该重新解析，传别的歌词也应当被忽略
        track.sync(0, Some("[00:01.00]x\n[00:02.00]y"), None);
        assert_eq!(track.lines().len(), 1);
        assert_eq!(track.lines()[0].text, "a");
        // 换了下标才重新解析
        track.sync(1, Some("[00:01.00]x\n[00:02.00]y"), None);
        assert_eq!(track.lines().len(), 2);
    }

    #[test]
    fn track_attaches_translation_only_when_it_is_not_blank() {
        let mut track = LyricTrack::new();
        track.sync(0, Some("[00:05.00]hello"), Some("   \n  "));
        assert_eq!(track.lines()[0].translation, None, "空白翻译轨要当作没有");

        track.sync(1, Some("[00:05.00]hello"), Some("[00:05.00]你好"));
        assert_eq!(track.lines()[0].translation.as_deref(), Some("你好"));
    }

    #[test]
    fn track_reset_forces_a_reparse() {
        let mut track = LyricTrack::new();
        track.sync(0, Some("[00:01.00]a"), None);
        track.reset();
        assert_eq!(track.active(), -1);
        assert_eq!(track.shift(), 0.0);
        track.sync(0, Some("[00:01.00]b"), None);
        assert_eq!(track.lines()[0].text, "b", "reset 之后同一个下标也要重新解析");
    }

    #[test]
    fn first_line_does_not_kick_off_a_slide() {
        let mut track = LyricTrack::new();
        track.sync(0, Some("[00:01.00]a\n[00:02.00]b"), None);
        assert!(track.tick(1.0, 26.0), "从 -1 进到第 0 行算变化");
        assert_eq!(track.active(), 0);
        assert_eq!(track.shift(), 0.0, "刚进入歌词区不该滑");
        assert!(!track.is_sliding());
    }

    #[test]
    fn changing_line_starts_a_slide_that_decays_to_zero() {
        let mut track = LyricTrack::new();
        track.sync(0, Some("[00:01.00]a\n[00:02.00]b"), None);
        track.tick(1.0, 26.0);
        assert!(track.tick(2.0, 26.0));
        // 跨一行 → 初速度 26，同一帧内已经衰减过一次
        assert!(track.shift() > 0.0 && track.shift() < 26.0);
        assert!(track.is_sliding());

        // 反复 tick 必须收敛到恰好 0，不能留浮点尾数让它永远在"滑动"
        for _ in 0..50 {
            track.tick(2.0, 26.0);
        }
        assert_eq!(track.shift(), 0.0);
        assert!(!track.is_sliding());
    }

    #[test]
    fn a_big_jump_is_clamped() {
        let mut track = LyricTrack::new();
        let lrc: String = (0..40)
            .map(|i| format!("[00:{:02}.00]line{i}\n", i))
            .collect();
        let mut track2 = LyricTrack::new();
        track.sync(0, Some(&lrc), None);
        track.tick(1.0, 26.0);
        // 一口气 seek 到很后面：跨 30 多行，偏移必须被夹住
        track.tick(35.0, 26.0);
        assert!(track.shift() <= MAX_SHIFT, "shift={} 没夹住", track.shift());

        // 反方向同样要夹住
        track2.sync(0, Some(&lrc), None);
        track2.tick(35.0, 26.0);
        track2.tick(1.0, 26.0);
        assert!(track2.shift() >= -MAX_SHIFT, "shift={} 没夹住", track2.shift());
    }

    #[test]
    fn tick_on_an_empty_lyric_list_stays_put() {
        let mut track = LyricTrack::new();
        track.sync(0, None, None);
        assert!(track.lines().is_empty());
        assert!(!track.tick(10.0, 26.0), "没有歌词就没有变化");
        assert_eq!(track.active(), -1);
        assert_eq!(track.shift(), 0.0);
    }
}
