//! LRC 解析。轻量模式没有 JS，歌词得在 Rust 侧自己解析。
//!
//! 快照里存的是 LRC 原文（前端抓下来直接塞进去的），格式上要容忍现实里的脏数据：
//! 时间戳可能是两位或三位毫秒、一行可能挂多个时间戳（副歌复用）、开头可能有
//! `[ti:]`/`[ar:]`/`[al:]`/`[by:]`/`[offset:]` 这类元信息标签，顺序也不保证递增。

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
}
