//! The local clock the vault writes: an ISO time with its offset, and the
//! `YYYY-MM-DD_HHMM` file stamp. Kept here rather than in `platform/` because
//! the vault is self-contained and only this module needs wall-clock time.

use std::time::{SystemTime, UNIX_EPOCH};

/// An instant and the local offset it was written with, in minutes east of UTC.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Moment {
    pub epoch: i64,
    pub offset_min: i32,
}

impl Moment {
    pub fn now() -> Moment {
        let epoch = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |delta| delta.as_secs() as i64);
        Moment { epoch, offset_min: local_offset_minutes(epoch) }
    }

    #[cfg(test)]
    pub fn at(epoch: i64, offset_min: i32) -> Moment {
        Moment { epoch, offset_min }
    }

    /// (year, month, day, hour, minute, second) as this offset reads them.
    fn local(&self) -> (i32, u32, u32, u32, u32, u32) {
        let total = self.epoch + self.offset_min as i64 * 60;
        let days = total.div_euclid(86_400);
        let secs = total.rem_euclid(86_400);
        let (year, month, day) = civil_from_days(days);
        (year, month, day, (secs / 3600) as u32, (secs % 3600 / 60) as u32, (secs % 60) as u32)
    }

    /// Local time as people write it in a note: `2026-10-19T12:08+03:00`.
    pub fn note_stamp(&self) -> String {
        let (year, month, day, hour, minute, _) = self.local();
        format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}{}", self.offset_text())
    }

    /// A mark's time, to the second: `2026-11-05T09:10:00+03:00`.
    pub fn mark_stamp(&self) -> String {
        let (year, month, day, hour, minute, second) = self.local();
        format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}{}", self.offset_text())
    }

    /// A note file's name: `2026-10-19_1208`.
    pub fn file_stamp(&self) -> String {
        let (year, month, day, hour, minute, _) = self.local();
        format!("{year:04}-{month:02}-{day:02}_{hour:02}{minute:02}")
    }

    fn offset_text(&self) -> String {
        let sign = if self.offset_min >= 0 { '+' } else { '-' };
        let absolute = self.offset_min.unsigned_abs();
        format!("{sign}{:02}:{:02}", absolute / 60, absolute % 60)
    }
}

/// Whether a note's `YYYY-MM-DD…` stamp is more than `days` before today.
pub fn older_than(stamp: Option<&str>, days: i64) -> bool {
    let Some(stamp) = stamp else { return false };
    let Some(date) = stamp.get(..10) else { return false };
    let (Some(year), Some(month), Some(day)) = (date.get(..4), date.get(5..7), date.get(8..10)) else {
        return false;
    };
    let (Ok(year), Ok(month), Ok(day)) = (year.parse::<i32>(), month.parse::<u32>(), day.parse::<u32>()) else {
        return false;
    };
    let (ty, tm, td, _, _, _) = Moment::now().local();
    days_from_civil(ty, tm, td) - days_from_civil(year, month, day) > days
}

/// Howard Hinnant's `days_from_civil`: days since 1970-01-01.
fn days_from_civil(year: i32, month: u32, day: u32) -> i64 {
    let year = if month <= 2 { year - 1 } else { year } as i64;
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let yoe = year - era * 400;
    let month = month as i64;
    let doy = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Howard Hinnant's `civil_from_days`: (year, month, day) from days since 1970-01-01.
fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    ((if month <= 2 { year + 1 } else { year }) as i32, month as u32, day as u32)
}

#[cfg(windows)]
fn local_offset_minutes(epoch: i64) -> i32 {
    use windows_sys::Win32::Foundation::SYSTEMTIME;
    use windows_sys::Win32::System::SystemInformation::GetLocalTime;
    let mut time: SYSTEMTIME = unsafe { std::mem::zeroed() };
    unsafe { GetLocalTime(&mut time) };
    let local = days_from_civil(time.wYear as i32, time.wMonth as u32, time.wDay as u32) * 86_400
        + time.wHour as i64 * 3600
        + time.wMinute as i64 * 60
        + time.wSecond as i64;
    (((local - epoch) as f64) / 60.0).round() as i32
}

#[cfg(unix)]
fn local_offset_minutes(epoch: i64) -> i32 {
    let mut time: libc::tm = unsafe { std::mem::zeroed() };
    // musl's libc crate deprecates the alias; it is still the type localtime_r takes.
    #[allow(deprecated)]
    let stamp: libc::time_t = epoch as libc::time_t;
    let converted = unsafe { libc::localtime_r(&stamp, &mut time) };
    if converted.is_null() {
        return 0;
    }
    let local = days_from_civil(time.tm_year + 1900, (time.tm_mon + 1) as u32, time.tm_mday as u32) * 86_400
        + time.tm_hour as i64 * 3600
        + time.tm_min as i64 * 60
        + time.tm_sec as i64;
    (((local - epoch) as f64) / 60.0).round() as i32
}

#[cfg(not(any(unix, windows)))]
fn local_offset_minutes(_: i64) -> i32 {
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_the_local_time_with_its_offset() {
        let east = Moment::at(0, 180);
        assert_eq!(east.note_stamp(), "1970-01-01T03:00+03:00");
        assert_eq!(east.mark_stamp(), "1970-01-01T03:00:00+03:00");
        assert_eq!(east.file_stamp(), "1970-01-01_0300");
        let west = Moment::at(0, -330);
        assert_eq!(west.note_stamp(), "1969-12-31T18:30-05:30");
    }

    #[test]
    fn the_civil_calendar_round_trips() {
        for days in [-1, 0, 1, 20_000, 30_000] {
            let (year, month, day) = civil_from_days(days);
            assert_eq!(days_from_civil(year, month, day), days, "{year}-{month}-{day}");
        }
    }

    #[test]
    fn a_note_more_than_a_week_old_is_old() {
        let (year, month, day, _, _, _) = Moment::now().local();
        let today = days_from_civil(year, month, day);
        let stamp = |days: i64| {
            let (year, month, day) = civil_from_days(days);
            format!("{year:04}-{month:02}-{day:02}T09:00+03:00")
        };
        assert!(older_than(Some(&stamp(today - 8)), 7));
        assert!(!older_than(Some(&stamp(today - 7)), 7));
        assert!(!older_than(None, 7));
    }
}
