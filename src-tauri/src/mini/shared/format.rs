//! 轻量模式里的纯格式化，不含任何平台类型。

/// 秒数转 `mm:ss`。
///
/// 非有限值和负数一律回 `00:00`：时长在源打开前是 0 或 NaN，直接 `as u64` 出来的是
/// 垃圾数字，画在界面上比 `00:00` 难看得多。
pub fn fmt_time(seconds: f64) -> String {
    if !seconds.is_finite() || seconds <= 0.0 {
        return "00:00".to_string();
    }
    let total = seconds as u64;
    format!("{:02}:{:02}", total / 60, total % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_and_negative_and_nonfinite_all_read_zero() {
        assert_eq!(fmt_time(0.0), "00:00");
        assert_eq!(fmt_time(-1.0), "00:00");
        assert_eq!(fmt_time(f64::NAN), "00:00");
        assert_eq!(fmt_time(f64::INFINITY), "00:00");
        assert_eq!(fmt_time(f64::NEG_INFINITY), "00:00");
    }

    #[test]
    fn sub_minute_and_normal_track_lengths() {
        assert_eq!(fmt_time(0.4), "00:00");
        assert_eq!(fmt_time(1.0), "00:01");
        assert_eq!(fmt_time(59.9), "00:59");
        assert_eq!(fmt_time(60.0), "01:00");
        assert_eq!(fmt_time(215.0), "03:35");
    }

    #[test]
    fn over_an_hour_keeps_counting_minutes() {
        // 刻意不进位到 hh:mm:ss——一首歌不会有一小时，而长播客读 "72:15" 也不会误解。
        assert_eq!(fmt_time(3600.0), "60:00");
        assert_eq!(fmt_time(4335.0), "72:15");
    }
}
