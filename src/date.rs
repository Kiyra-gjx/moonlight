//! 轻量日期工具：内部统一用「距 1970-01-01 的天数」表示日期，
//! 对外读写 `YYYY-MM-DD` 字符串。不引入第三方日期库。

use std::time::{SystemTime, UNIX_EPOCH};

/// 东八区偏移秒数
const TZ_OFFSET: u64 = 8 * 3600;

/// 今天（东八区）对应的天数
pub fn today() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| ((d.as_secs() + TZ_OFFSET) / 86400) as i64)
        .unwrap_or(0)
}

/// `YYYY-MM-DD` -> 天数，格式非法时返回 None
pub fn parse(s: &str) -> Option<i64> {
    let parts: Vec<&str> = s.trim().split('-').collect();
    if parts.len() != 3 {
        return None;
    }
    let y: i64 = parts[0].parse().ok()?;
    let m: i64 = parts[1].parse().ok()?;
    let d: i64 = parts[2].parse().ok()?;
    if !(1..=9999).contains(&y) || !(1..=12).contains(&m) {
        return None;
    }
    let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    let max_day = match m {
        2 if leap => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    if !(1..=max_day).contains(&d) {
        return None;
    }
    // days_from_civil：把 3 月当作一年的起点，规避闰日的特殊处理
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146097 + doe - 719468)
}

/// 天数 -> `YYYY-MM-DD`
pub fn format(days: i64) -> String {
    // civil_from_days，parse 的逆运算
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{:04}-{:02}-{:02}", y, m, d)
}

/// 相对今天偏移 n 天的日期字符串，用于生成演示数据
pub fn from_today(offset: i64) -> String {
    format(today() + offset)
}

/// 从起始日之后计入工作日，跳过周六、周日；暂不包含法定节假日。
pub fn add_workdays(start: i64, count: i64) -> i64 {
    let mut day = start;
    let mut remaining = count.max(0);
    // 先对齐到首个工作日，再整周跳跃，避免大工时输入导致逐日长循环。
    while remaining > 0 && (day + 3).rem_euclid(7) >= 5 {
        day += 1;
        if (day + 3).rem_euclid(7) < 5 {
            remaining -= 1;
        }
    }
    let weeks = remaining / 5;
    day = day.saturating_add(weeks.saturating_mul(7));
    remaining %= 5;
    while remaining > 0 {
        day += 1;
        // 1970-01-01 为周四；0 表示周一。
        if (day + 3).rem_euclid(7) < 5 {
            remaining -= 1;
        }
    }
    day
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 日期互转应当自洽() {
        for s in ["1970-01-01", "2000-02-29", "2026-09-03", "2100-12-31"] {
            let d = parse(s).unwrap();
            assert_eq!(format(d), s);
        }
        assert_eq!(parse("1970-01-01"), Some(0));
        assert_eq!(parse("1970-01-02"), Some(1));
    }

    #[test]
    fn 非法日期应当返回_none() {
        assert!(parse("2026-13-01").is_none());
        assert!(parse("2026-09").is_none());
        assert!(parse("abc").is_none());
        assert!(parse("2026-02-29").is_none());
        assert!(parse("2026-02-31").is_none());
        assert!(parse("2024-02-29").is_some());
    }

    #[test]
    fn 工作日交付应跳过周末() {
        let friday = parse("2026-09-04").unwrap();
        assert_eq!(format(add_workdays(friday, 1)), "2026-09-07");
        assert_eq!(format(add_workdays(friday, 5)), "2026-09-11");
        assert_eq!(add_workdays(friday, 0), friday);
        assert_eq!(
            format(add_workdays(parse("2026-09-05").unwrap(), 1)),
            "2026-09-07"
        );
    }
}
