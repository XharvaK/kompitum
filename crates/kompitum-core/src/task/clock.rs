//! Time without a crate: a local calendar, the words for when a block ends,
//! and the ways a person may say when. Times are `u64` milliseconds, as
//! `started_ms` is; `offset_min` is local time east of UTC.

/// A moment on a local calendar. `weekday` is 0 for Monday.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Civil {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
    pub weekday: u32,
}

const DAY_MS: i64 = 86_400_000;
const MINUTE_MS: i64 = 60_000;

/// Days since 1970-01-01 for a proleptic Gregorian date (Howard Hinnant's).
fn days_from_civil(year: i32, month: u32, day: u32) -> i64 {
    let y = if month <= 2 { year - 1 } else { year } as i64;
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let m = month as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + day as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    ((if month <= 2 { y + 1 } else { y }) as i32, month as u32, day as u32)
}

/// The local calendar for a moment.
pub fn civil(ms: u64, offset_min: i32) -> Civil {
    let local = ms as i64 + offset_min as i64 * MINUTE_MS;
    let days = local.div_euclid(DAY_MS);
    let rest = local.rem_euclid(DAY_MS);
    let (year, month, day) = civil_from_days(days);
    // 1970-01-01 was a Thursday; Monday is 0.
    let weekday = (days + 3).rem_euclid(7) as u32;
    Civil {
        year,
        month,
        day,
        hour: (rest / 3_600_000) as u32,
        minute: (rest % 3_600_000 / MINUTE_MS) as u32,
        second: (rest % MINUTE_MS / 1000) as u32,
        weekday,
    }
}

/// The moment a local calendar names.
pub fn to_ms(c: &Civil, offset_min: i32) -> u64 {
    let days = days_from_civil(c.year, c.month, c.day);
    let ms = days * DAY_MS + c.hour as i64 * 3_600_000 + c.minute as i64 * MINUTE_MS + c.second as i64 * 1000
        - offset_min as i64 * MINUTE_MS;
    ms.max(0) as u64
}

const WEEKDAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

fn clock_words(c: &Civil) -> String {
    format!("{:02}:{:02}", c.hour, c.minute)
}

/// When a moment is, in words that always carry their preposition.
pub fn when_words(ms: u64, now_ms: u64, offset_min: i32) -> String {
    let at = civil(ms, offset_min);
    let now = civil(now_ms, offset_min);
    let days = days_from_civil(at.year, at.month, at.day) - days_from_civil(now.year, now.month, now.day);
    match days {
        0 => format!("at {}", clock_words(&at)),
        1 => format!("tomorrow at {}", clock_words(&at)),
        _ => {
            let year = if at.year != now.year { format!(" {}", at.year) } else { String::new() };
            format!(
                "on {} {} {}{} at {}",
                WEEKDAYS[at.weekday as usize % 7],
                at.day,
                MONTHS[(at.month - 1) as usize % 12],
                year,
                clock_words(&at)
            )
        }
    }
}

fn bad(text: &str) -> String {
    format!("cannot read {text:?}: use 2026-11-23, 2026-11-23 15:36, 2026-11-23T15:36:00Z, or a span such as 3h or 2d")
}

/// A century: past it a span is a slip of the finger, and past `i64` the
/// calendar's own sums would overflow.
const LONGEST_SPAN_MS: u64 = 100 * 365 * 86_400_000;

/// A span like `45m`, `3h`, `2d` or `1d2h`, in milliseconds from now.
fn span(text: &str) -> Option<u64> {
    let bytes = text.as_bytes();
    let mut i = 0;
    let mut total = 0u64;
    let mut parts = 0;
    while i < bytes.len() {
        let start = i;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        if i == start || i >= bytes.len() {
            return None;
        }
        let n: u64 = text[start..i].parse().ok()?;
        let ms = match bytes[i] {
            b'd' => n.checked_mul(86_400_000)?,
            b'h' => n.checked_mul(3_600_000)?,
            b'm' => n.checked_mul(MINUTE_MS as u64)?,
            _ => return None,
        };
        total = total.checked_add(ms)?;
        parts += 1;
        i += 1;
    }
    (parts > 0 && total <= LONGEST_SPAN_MS).then_some(total)
}

/// The date at the front of the text and what follows it.
fn date_prefix(text: &str) -> Option<(i32, u32, u32, &str)> {
    let bytes = text.as_bytes();
    if bytes.len() < 10 || bytes[4] != b'-' || bytes[7] != b'-' {
        return None;
    }
    let digits = |range: std::ops::Range<usize>| -> Option<u32> {
        bytes.get(range.clone()).filter(|b| b.iter().all(u8::is_ascii_digit)).and_then(|_| text[range].parse().ok())
    };
    Some((digits(0..4)? as i32, digits(5..7)?, digits(8..10)?, &text[10..]))
}

/// Exactly two ASCII digits, as a number.
fn two_digits(text: &str) -> Option<u32> {
    let bytes = text.as_bytes();
    (bytes.len() == 2 && bytes.iter().all(u8::is_ascii_digit)).then(|| text.parse().ok())?
}

/// An RFC 3339 offset, the whole of the text: its minutes east.
fn offset_suffix(text: &str) -> Option<i32> {
    if text == "Z" || text == "z" {
        return Some(0);
    }
    let sign = match text.as_bytes().first()? {
        b'+' => 1,
        b'-' => -1,
        _ => return None,
    };
    let rest = &text[1..];
    let (hour, minute) = match rest.len() {
        2 => (two_digits(rest)?, 0),
        5 if rest.as_bytes()[2] == b':' => (two_digits(rest.get(..2)?)?, two_digits(rest.get(3..)?)?),
        4 => (two_digits(rest.get(..2)?)?, two_digits(rest.get(2..)?)?),
        _ => return None,
    };
    (hour <= 23 && minute <= 59).then_some(sign * (hour as i32 * 60 + minute as i32))
}

fn clock_prefix(text: &str) -> Option<(u32, u32, u32, &str)> {
    let bytes = text.as_bytes();
    if bytes.len() < 5 || bytes[2] != b':' {
        return None;
    }
    let hour = two_digits(text.get(..2)?)?;
    let minute = two_digits(text.get(3..5)?)?;
    let (second, rest) = if bytes.get(5) == Some(&b':') {
        let second = two_digits(text.get(6..8)?)?;
        let mut at = 8;
        if bytes.get(at) == Some(&b'.') {
            at += 1;
            let start = at;
            while at < bytes.len() && bytes[at].is_ascii_digit() {
                at += 1;
            }
            if at == start {
                return None;
            }
        }
        (second, &text[at..])
    } else {
        (0, &text[5..])
    };
    (hour <= 23 && minute <= 59 && second <= 59).then_some((hour, minute, second, rest))
}

/// A date, a date and clock, RFC 3339, or a span from now. Anything else, or
/// a moment not after `now_ms`, is an error in words.
pub fn parse_when(text: &str, now_ms: u64, offset_min: i32) -> Result<u64, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err(bad(text));
    }
    if let Some(ms) = span(text) {
        let at = now_ms.saturating_add(ms);
        return if at > now_ms { Ok(at) } else { Err(format!("{text:?} is not after now")) };
    }
    let Some((year, month, day, rest)) = date_prefix(text) else { return Err(bad(text)) };
    let (hour, minute, second, offset) = if rest.is_empty() {
        (0, 0, 0, None)
    } else if let Some(rest) = rest.strip_prefix('T').or_else(|| rest.strip_prefix(' ')) {
        let Some((hour, minute, second, tail)) = clock_prefix(rest) else { return Err(bad(text)) };
        let offset = if tail.is_empty() {
            None
        } else {
            let Some(offset) = offset_suffix(tail) else { return Err(bad(text)) };
            Some(offset)
        };
        (hour, minute, second, offset)
    } else {
        return Err(bad(text));
    };
    let at = Civil { year, month, day, hour, minute, second, weekday: 0 };
    let ms = to_ms(&at, offset.unwrap_or(offset_min));
    let back = civil(ms, offset.unwrap_or(offset_min));
    if back.year != year || back.month != month || back.day != day || back.hour != hour {
        return Err(bad(text));
    }
    if ms <= now_ms {
        return Err(format!("{text} has passed: give a time after now"));
    }
    Ok(ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_date_round_trips_through_leap_years_and_offsets() {
        let leap = to_ms(&Civil { year: 2024, month: 2, day: 29, hour: 12, minute: 34, second: 56, weekday: 0 }, 0);
        let back = civil(leap, 0);
        assert_eq!((back.year, back.month, back.day, back.hour, back.minute, back.second), (2024, 2, 29, 12, 34, 56));
        assert_eq!(back.weekday, 3, "2024-02-29 was a Thursday");
        for offset in [-480, -60, 0, 180, 345] {
            let at = Civil { year: 2026, month: 11, day: 23, hour: 15, minute: 36, second: 0, weekday: 0 };
            let ms = to_ms(&at, offset);
            let round = civil(ms, offset);
            assert_eq!((round.year, round.month, round.day, round.hour, round.minute), (2026, 11, 23, 15, 36));
            assert_eq!(ms, to_ms(&round, offset));
        }
        assert_eq!(civil(0, 0).weekday, 3, "1970-01-01 was a Thursday");
        assert_eq!(civil(0, 0).year, 1970);
        // The same instant is another day east of UTC.
        let midnight = to_ms(&Civil { year: 2026, month: 11, day: 23, hour: 0, minute: 0, second: 0, weekday: 0 }, 0);
        assert_eq!(civil(midnight, 180).hour, 3);
        assert_eq!(civil(midnight, -300).day, 22);
    }

    #[test]
    fn when_words_say_today_tomorrow_or_the_date() {
        let now = to_ms(&Civil { year: 2026, month: 11, day: 5, hour: 9, minute: 0, second: 0, weekday: 0 }, 0);
        let same = to_ms(&Civil { year: 2026, month: 11, day: 5, hour: 15, minute: 36, second: 0, weekday: 0 }, 0);
        assert_eq!(when_words(same, now, 0), "at 15:36");
        let tomorrow = to_ms(&Civil { year: 2026, month: 11, day: 6, hour: 15, minute: 36, second: 0, weekday: 0 }, 0);
        assert_eq!(when_words(tomorrow, now, 0), "tomorrow at 15:36");
        let later = to_ms(&Civil { year: 2026, month: 11, day: 7, hour: 21, minute: 0, second: 0, weekday: 0 }, 0);
        assert_eq!(when_words(later, now, 0), "on Sat 7 Nov at 21:00");
        let next_year = to_ms(&Civil { year: 2027, month: 1, day: 1, hour: 0, minute: 0, second: 0, weekday: 0 }, 0);
        assert_eq!(when_words(next_year, now, 0), "on Fri 1 Jan 2027 at 00:00");
        // Local days, not UTC days.
        let utc_midnight =
            to_ms(&Civil { year: 2026, month: 11, day: 6, hour: 0, minute: 0, second: 0, weekday: 0 }, 0);
        assert_eq!(when_words(utc_midnight, now, 180), "tomorrow at 03:00");
    }

    #[test]
    fn parse_when_takes_a_date_a_clock_a_span_and_refuses_the_rest() {
        let now = to_ms(&Civil { year: 2026, month: 11, day: 6, hour: 9, minute: 0, second: 0, weekday: 0 }, 180);
        let local_midnight =
            to_ms(&Civil { year: 2026, month: 11, day: 23, hour: 0, minute: 0, second: 0, weekday: 0 }, 180);
        assert_eq!(parse_when("2026-11-23", now, 180), Ok(local_midnight));
        let local_clock =
            to_ms(&Civil { year: 2026, month: 11, day: 23, hour: 15, minute: 36, second: 0, weekday: 0 }, 180);
        assert_eq!(parse_when("2026-11-23 15:36", now, 180), Ok(local_clock));
        assert_eq!(parse_when("2026-11-23T15:36", now, 180), Ok(local_clock));
        assert_eq!(parse_when("2026-11-23T15:36:00+03:00", now, -120), Ok(local_clock));
        assert_eq!(parse_when("2026-11-23T12:36:00Z", now, 180), Ok(local_clock));
        assert_eq!(parse_when("45m", now, 180), Ok(now + 45 * 60_000));
        assert_eq!(parse_when("3h", now, 180), Ok(now + 3 * 3_600_000));
        assert_eq!(parse_when("2d", now, 180), Ok(now + 2 * 86_400_000));
        assert_eq!(parse_when("1d2h", now, 180), Ok(now + 26 * 3_600_000));
        for bad in [
            "",
            "tomorrow",
            "2026-13-01",
            "2026-02-30",
            "2026-11-23 25:00",
            "2026-11-23T15:36:00+3",
            "5x",
            "1\u{e9}",
            "\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}",
            "2026-11-23 1\u{e9}:00",
            "2026-11-23T12:3\u{e9}",
            "2026-11-23T15:36+a\u{e9}1",
            "2026-11-23T15:36+12:-5",
            "2026-11-23T+5:00",
            "0001-01-01",
            "106751991167d",
        ] {
            assert!(parse_when(bad, now, 180).is_err(), "{bad}");
        }
        let past = "2026-11-06 08:00";
        assert!(parse_when(past, now, 180).unwrap_err().contains("passed"));
        assert!(parse_when("0m", now, 180).is_err(), "now is not after now");
        assert!(parse_when("2026-11-23", now, 180).unwrap() > now);
    }
}
