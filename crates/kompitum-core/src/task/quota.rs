//! A refused turn, in the worker's own words. Pure: only provider-shaped
//! errors count, never free text, and an exit code alone is no evidence. The
//! port follows `policy.py`'s classifiers.

use crate::task::clock::{self, Civil};
use crate::task::types::cut;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::OnceLock;

/// Who set a block's time. `Named`: the worker's own message gave it.
#[derive(Serialize, Deserialize, schemars::JsonSchema, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Person,
    Named,
    #[default]
    Estimate,
}

/// Why a turn was refused, and when the worker may be tried again.
#[derive(Clone, Debug, PartialEq)]
pub struct Refusal {
    pub window: String,
    pub until_ms: u64,
    pub source: Source,
    /// A person's reason, or the worker's words, cut to 300 characters.
    pub message: String,
}

fn re(cell: &'static OnceLock<Regex>, pattern: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(pattern).expect("a quota pattern compiles"))
}

static QUOTA_TYPES: OnceLock<Regex> = OnceLock::new();
static QUOTA_SENTENCES: OnceLock<Regex> = OnceLock::new();
static QUOTA_STDERR: OnceLock<Regex> = OnceLock::new();
static QUOTA_JSON_FIELDS: OnceLock<Regex> = OnceLock::new();
static WINDOW_5H: OnceLock<Regex> = OnceLock::new();
static DAY_WORD: OnceLock<Regex> = OnceLock::new();
static RETRY_AFTER_MS: OnceLock<Regex> = OnceLock::new();
static RETRY_AFTER: OnceLock<Regex> = OnceLock::new();
static RESETS_IN: OnceLock<Regex> = OnceLock::new();
static REACHED: OnceLock<Regex> = OnceLock::new();
static NOT_AUTH: OnceLock<Regex> = OnceLock::new();
static RATE_LIMIT: OnceLock<Regex> = OnceLock::new();
static CLOCK_RESET: OnceLock<Regex> = OnceLock::new();
static NEXT_CLOCK: OnceLock<Regex> = OnceLock::new();
static CODEX_USAGE: OnceLock<Regex> = OnceLock::new();
static CODEX_TRY_AGAIN: OnceLock<Regex> = OnceLock::new();
static CODEX_DATE: OnceLock<Regex> = OnceLock::new();
static CODEX_CLOCK: OnceLock<Regex> = OnceLock::new();

/// The longest wait a worker's words can set: a garbled number must neither
/// overflow nor take a worker away for good.
const LONGEST_WAIT_MS: u64 = 366 * 86_400_000;

fn wait_until(now_ms: u64, wait_ms: u64) -> u64 {
    now_ms.saturating_add(wait_ms.min(LONGEST_WAIT_MS))
}

/// "Resets in 4d 22h", in either CLI's words.
fn resets_in() -> &'static Regex {
    re(&RESETS_IN, r"(?i)resets? in\s+(?:(\d+)\s*d)?\s*(?:(\d+)\s*h)?\s*(?:(\d+)\s*m)?")
}

fn quota_types() -> &'static Regex {
    re(
        &QUOTA_TYPES,
        r"(?i)^(?:provider\.free-limit|FreeUsageLimit(?:Error)?|RateLimit(?:Error)?|rate_limit_error|insufficient_quota|InsufficientBalance(?:Error)?|QuotaExceeded(?:Error)?|UsageLimit(?:Error|Exceeded)?|TooManyRequests(?:Error)?)$",
    )
}

fn quota_sentences() -> &'static Regex {
    re(
        &QUOTA_SENTENCES,
        r"(?i)free usage limit reached|reached your (?:5-hour|daily|weekly|monthly) usage limit|insufficient (?:balance|quota)\b",
    )
}

fn quota_stderr() -> &'static Regex {
    re(
        &QUOTA_STDERR,
        r#"FreeUsageLimitError|provider\.free-limit|"(?:status|statusCode)"\s*:\s*429\b|(?i:free usage limit reached)"#,
    )
}

fn quota_json_fields() -> &'static Regex {
    re(
        &QUOTA_JSON_FIELDS,
        r#""(?:status|statusCode|status_code|code)"\s*:\s*"?429\b|"(?:type|name|code)"\s*:\s*"(?:provider\.free-limit|FreeUsageLimit(?:Error)?|RateLimit(?:Error)?|rate_limit_error|insufficient_quota|InsufficientBalance(?:Error)?|QuotaExceeded(?:Error)?|UsageLimit(?:Error|Exceeded)?|TooManyRequests(?:Error)?)""#,
    )
}

/// True only for a provider-shaped limit: structured error fields, or the
/// provider's own JSON on stderr.
fn error_is_quota(value: &Value, depth: u32) -> bool {
    if depth > 4 {
        return false;
    }
    match value {
        Value::String(text) => match serde_json::from_str::<Value>(text) {
            Ok(parsed) => !matches!(parsed, Value::String(_)) && error_is_quota(&parsed, depth + 1),
            Err(_) => quota_json_fields().is_match(text) || quota_stderr().is_match(text),
        },
        Value::Object(object) => {
            let status = ["status", "statusCode", "status_code", "code"]
                .iter()
                .filter_map(|key| object.get(*key))
                .any(|value| value.as_str().is_some_and(|text| text.trim() == "429") || value.as_u64() == Some(429));
            if status {
                return true;
            }
            for key in ["type", "name", "code"] {
                if object.get(key).and_then(Value::as_str).is_some_and(|text| quota_types().is_match(text.trim())) {
                    return true;
                }
            }
            if object.get("message").and_then(Value::as_str).is_some_and(|text| quota_sentences().is_match(text)) {
                return true;
            }
            for key in ["error", "data", "response", "cause"] {
                let Some(nested) = object.get(key) else { continue };
                if key == "response"
                    && let Some(body) = nested.get("body").and_then(Value::as_str)
                    && let Ok(parsed) = serde_json::from_str::<Value>(body)
                    && error_is_quota(&parsed, depth + 1)
                {
                    return true;
                }
                if error_is_quota(nested, depth + 1) {
                    return true;
                }
            }
            false
        }
        _ => false,
    }
}

fn quota_window(blob: &str) -> String {
    let low = blob.to_lowercase();
    if re(&WINDOW_5H, r"5[- ]?h(our)?\b|five[- ]hour|rolling").is_match(&low) {
        return "5-hour".into();
    }
    if low.contains("week") {
        return "weekly".into();
    }
    if low.contains("month") {
        return "monthly".into();
    }
    if re(&DAY_WORD, r"daily|\bday\b|24[- ]?h|free usage limit").is_match(&low) {
        return "daily".into();
    }
    "unknown".into()
}

/// The next 00:00 UTC, as `parse_reset`'s default (Free's daily reset).
fn next_utc_midnight(now_ms: u64) -> u64 {
    let today = clock::civil(now_ms, 0);
    let midnight = clock::to_ms(&Civil { hour: 0, minute: 0, second: 0, ..today }, 0);
    if midnight > now_ms { midnight } else { midnight + 86_400_000 }
}

fn groups(captures: &regex::Captures) -> Vec<u64> {
    // A number too long to read is a long wait, not no wait.
    (1..captures.len()).map(|i| captures.get(i).map_or(0, |m| m.as_str().parse().unwrap_or(u64::MAX))).collect()
}

fn span_ms(parts: &[u64]) -> u64 {
    parts[0]
        .saturating_mul(86_400_000)
        .saturating_add(parts[1].saturating_mul(3_600_000))
        .saturating_add(parts[2].saturating_mul(60_000))
}

/// When the reset is, and who said so.
fn opencode_reset(blob: &str, now_ms: u64, window: &str) -> (u64, Source) {
    if let Some(captures) = re(&RETRY_AFTER_MS, r#"(?i)retry_?after_?ms[\\"]*\s*[:=]\s*[\\"]*(\d+)"#).captures(blob) {
        let ms: u64 = captures[1].parse().unwrap_or(u64::MAX);
        return (wait_until(now_ms, ms), Source::Named);
    }
    if let Some(captures) = re(&RETRY_AFTER, r#"(?i)retry-?after[\\"]*\s*[:=]\s*[\\"]*(\d+)"#).captures(blob) {
        let seconds: u64 = captures[1].parse().unwrap_or(u64::MAX);
        return (wait_until(now_ms, seconds.saturating_mul(1000)), Source::Named);
    }
    if let Some(captures) = resets_in().captures(blob) {
        let parts = groups(&captures);
        if parts.iter().any(|part| *part > 0) {
            return (wait_until(now_ms, span_ms(&parts)), Source::Named);
        }
    }
    let length = match window {
        "5-hour" => 5 * 3_600_000,
        "weekly" => 7 * 86_400_000,
        "monthly" => 30 * 86_400_000,
        _ => 0,
    };
    if length > 0 {
        (now_ms.saturating_add(length), Source::Estimate)
    } else {
        (next_utc_midnight(now_ms), Source::Estimate)
    }
}

fn evidence(raw_errors: &[Value], stderr: &str) -> String {
    for value in raw_errors {
        if let Some(message) = value.get("message").and_then(Value::as_str).filter(|text| !text.is_empty()) {
            return message.to_string();
        }
        if let Some(message) =
            value.get("error").and_then(|error| error.get("message")).and_then(Value::as_str).filter(|t| !t.is_empty())
        {
            return message.to_string();
        }
    }
    if let Some(line) = stderr.lines().map(str::trim).find(|line| !line.is_empty()) {
        return line.to_string();
    }
    raw_errors.first().map(Value::to_string).unwrap_or_default()
}

/// An OpenCode turn's refusal, when its errors or stderr name a provider limit.
pub fn opencode_refusal(raw_errors: &[Value], stderr: &str, now_ms: u64) -> Option<Refusal> {
    let provider_shaped = raw_errors.iter().any(|error| error_is_quota(error, 0)) || quota_stderr().is_match(stderr);
    if !provider_shaped {
        return None;
    }
    let blob = format!("{}\n{stderr}", raw_errors.iter().map(Value::to_string).collect::<Vec<_>>().join("\n"));
    let window = quota_window(&blob);
    let (until_ms, source) = opencode_reset(&blob, now_ms, &window);
    Some(Refusal { window, until_ms, source, message: cut(&evidence(raw_errors, stderr), 300) })
}

fn weekday(name: &str) -> Option<u32> {
    ["mon", "tue", "wed", "thu", "fri", "sat", "sun"]
        .iter()
        .position(|day| name.to_lowercase().starts_with(day))
        .map(|i| i as u32)
}

/// A weekday/time in parentheses after "Resets in", when it is within 3 hours
/// of the estimate.
fn clock_reset(after: &str, estimate_ms: u64, offset_min: i32) -> Option<u64> {
    let captures =
        re(&CLOCK_RESET, r"(?i)\(\s*([A-Za-z]{3})[A-Za-z]*\.?\s+(\d{1,2}):(\d{2})\s*(am|pm)?\s*\)").captures(after)?;
    let day = weekday(&captures[1])?;
    let mut hour: i64 = captures[2].parse().ok()?;
    let minute: i64 = captures[3].parse().ok()?;
    let marker = captures.get(4).map(|m| m.as_str().to_lowercase()).unwrap_or_default();
    if marker == "pm" && hour < 12 {
        hour += 12;
    } else if marker == "am" && hour == 12 {
        hour = 0;
    }
    if hour > 23 || minute > 59 {
        return None;
    }
    let local = clock::civil(estimate_ms, offset_min);
    let days = (day as i64 - local.weekday as i64).rem_euclid(7);
    let candidate = clock::to_ms(
        &Civil {
            year: local.year,
            month: local.month,
            day: local.day,
            hour: hour as u32,
            minute: minute as u32,
            second: 0,
            weekday: 0,
        },
        offset_min,
    ) as i64
        + days * 86_400_000;
    let week = 7 * 86_400_000i64;
    let nearest =
        [candidate, candidate - week].into_iter().min_by_key(|moment| (*moment - estimate_ms as i64).abs())?;
    ((nearest - estimate_ms as i64).abs() <= 3 * 3_600_000).then_some(nearest as u64)
}

/// The next local occurrence of "resets at H:MM[ AM|PM]" after now.
fn next_clock(text: &str, now_ms: u64, offset_min: i32) -> Option<u64> {
    let captures = re(&NEXT_CLOCK, r"(?i)resets at\s+(\d{1,2}):(\d{2})\s*(am|pm)?").captures(text)?;
    let mut hour: u32 = captures[1].parse().ok()?;
    let minute: u32 = captures[2].parse().ok()?;
    let marker = captures.get(3).map(|m| m.as_str().to_lowercase()).unwrap_or_default();
    if marker == "pm" && hour < 12 {
        hour += 12;
    } else if marker == "am" && hour == 12 {
        hour = 0;
    }
    if hour > 23 || minute > 59 {
        return None;
    }
    let local = clock::civil(now_ms, offset_min);
    let at = clock::to_ms(
        &Civil { year: local.year, month: local.month, day: local.day, hour, minute, second: 0, weekday: 0 },
        offset_min,
    );
    Some(if at <= now_ms { at + 86_400_000 } else { at })
}

fn command_code_reset(text: &str, now_ms: u64, offset_min: i32, window: &str) -> (u64, Source) {
    if let Some(captures) = resets_in().captures(text) {
        let parts = groups(&captures);
        if parts.iter().any(|part| *part > 0) {
            let estimate = wait_until(now_ms, span_ms(&parts));
            let after = &text[captures.get(0).map_or(0, |m| m.end())..];
            if let Some(clock) = clock_reset(after, estimate, offset_min) {
                return (clock, Source::Named);
            }
            return (estimate, Source::Named);
        }
    }
    if let Some(clock) = next_clock(text, now_ms, offset_min) {
        return (clock, Source::Named);
    }
    let length = match window {
        "5-hour" => 5 * 3_600_000,
        "weekly" => 7 * 86_400_000,
        "monthly" => 30 * 86_400_000,
        _ => 3_600_000,
    };
    (now_ms.saturating_add(length), Source::Estimate)
}

/// A Command Code limit or sign-in failure, or None for unrelated text. A
/// successful run is never a block, whatever its text says.
pub fn command_code_refusal(text: &str, failed: bool, now_ms: u64, offset_min: i32) -> Option<Refusal> {
    if !failed {
        return None;
    }
    let low = text.to_lowercase();
    let (window, reason) =
        if let Some(captures) = re(&REACHED, r"reached your (5-hour|weekly|monthly) usage limit").captures(&low) {
            let window = match &captures[1] {
                "5-hour" => "5-hour",
                "weekly" => "weekly",
                "monthly" => "monthly",
                _ => "unknown",
            };
            (window, "quota")
        } else if low.contains("insufficient credits") {
            ("credits", "credits")
        } else if re(&NOT_AUTH, r"\bnot authenticated\b|\blogin required\b").is_match(&low) {
            ("auth", "auth")
        } else if re(&RATE_LIMIT, r"\brate limit (?:reached|exceeded)\b|\btoo many requests\b|\b429 too many\b")
            .is_match(&low)
        {
            ("rate", "rate")
        } else {
            return None;
        };
    let (until_ms, source) = match reason {
        "quota" => command_code_reset(text, now_ms, offset_min, window),
        "rate" => (now_ms.saturating_add(10 * 60_000), Source::Estimate),
        _ => (now_ms.saturating_add(3_600_000), Source::Estimate),
    };
    Some(Refusal { window: window.into(), until_ms, source, message: cut(text.trim(), 300) })
}

fn codex_usage_limit(message: &str) -> bool {
    re(&CODEX_USAGE, r"(?i)you['’]ve hit your usage limit").is_match(message)
}

fn month_number(name: &str) -> Option<u32> {
    const MONTHS: [&str; 12] = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];
    let low = name.to_ascii_lowercase();
    MONTHS.iter().position(|month| low.starts_with(month)).map(|index| index as u32 + 1)
}

fn hour_24(hour: u32, marker: Option<&str>) -> Option<u32> {
    let hour = match marker.map(str::to_ascii_lowercase).as_deref() {
        Some("pm") if hour < 12 => hour + 12,
        Some("am") if hour == 12 => 0,
        _ => hour,
    };
    (hour <= 23).then_some(hour)
}

/// The moment `Try again at …` names: a clock today or tomorrow, a dated time
/// as it is. Local time, as the message is.
fn codex_try_again(text: &str, now_ms: u64, offset_min: i32) -> Option<u64> {
    let captures = re(
        &CODEX_TRY_AGAIN,
        r"(?i)try again at\s+([A-Za-z]{3,9}\.?\s+\d{1,2}(?:st|nd|rd|th)?,?\s+\d{4}\s+\d{1,2}:\d{2}(?:\s*[ap]m)?|\d{1,2}:\d{2}(?:\s*[ap]m)?)",
    )
    .captures(text)?;
    let when = captures[1].trim();
    if let Some(captures) = re(
        &CODEX_DATE,
        r"(?i)^([A-Za-z]{3,9})\.?\s+(\d{1,2})(?:st|nd|rd|th)?,?\s+(\d{4})\s+(\d{1,2}):(\d{2})(?:\s*([ap]m))?$",
    )
    .captures(when)
    {
        let month = month_number(&captures[1])?;
        let day: u32 = captures[2].parse().ok()?;
        let year: i32 = captures[3].parse().ok()?;
        let hour = hour_24(captures[4].parse().ok()?, captures.get(6).map(|marker| marker.as_str()))?;
        let minute: u32 = captures[5].parse().ok()?;
        if !(1..=31).contains(&day) || minute > 59 {
            return None;
        }
        return Some(clock::to_ms(&Civil { year, month, day, hour, minute, second: 0, weekday: 0 }, offset_min));
    }
    let captures = re(&CODEX_CLOCK, r"(?i)^(\d{1,2}):(\d{2})(?:\s*([ap]m))?$").captures(when)?;
    let hour = hour_24(captures[1].parse().ok()?, captures.get(3).map(|marker| marker.as_str()))?;
    let minute: u32 = captures[2].parse().ok()?;
    if minute > 59 {
        return None;
    }
    let local = clock::civil(now_ms, offset_min);
    let at = clock::to_ms(
        &Civil { year: local.year, month: local.month, day: local.day, hour, minute, second: 0, weekday: 0 },
        offset_min,
    );
    Some(if at <= now_ms { at + 86_400_000 } else { at })
}

/// Codex's usage limit, its reset, and its credits, from the errors a turn
/// reported. `Try again later` and a message with no time get the 5-hour estimate.
pub fn codex_refusal(errors: &[String], now_ms: u64, offset_min: i32) -> Option<Refusal> {
    let message = errors
        .iter()
        .find(|message| codex_usage_limit(message))
        .or_else(|| errors.iter().find(|message| message.to_lowercase().contains("out of credits")))?;
    if !codex_usage_limit(message) {
        return Some(Refusal {
            window: "credits".into(),
            until_ms: wait_until(now_ms, 3_600_000),
            source: Source::Estimate,
            message: cut(message.trim(), 300),
        });
    }
    let (until_ms, source) = match codex_try_again(message, now_ms, offset_min) {
        Some(until) => (until, Source::Named),
        None => (wait_until(now_ms, 5 * 3_600_000), Source::Estimate),
    };
    let window =
        if source == Source::Named && until_ms.saturating_sub(now_ms) > 5 * 3_600_000 { "weekly" } else { "5-hour" };
    Some(Refusal { window: window.into(), until_ms, source, message: cut(message.trim(), 300) })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> u64 {
        // 2026-10-26 13:00 UTC
        clock::to_ms(&Civil { year: 2026, month: 10, day: 26, hour: 13, minute: 0, second: 0, weekday: 0 }, 0)
    }

    #[test]
    fn only_provider_shaped_errors_are_quota_errors() {
        for event in [
            serde_json::json!({"name": "UnknownError", "message": "tool bash failed: test_quota_reset FAILED"}),
            serde_json::json!({"type": "error", "message": "assert 429 == status in test_http"}),
            serde_json::json!({"name": "ToolError", "message": "rate limit helper raised; usage limit test failed"}),
        ] {
            assert!(opencode_refusal(&[event], "", now()).is_none());
        }
        assert!(
            opencode_refusal(&[serde_json::json!({"name": "UnknownError"})], "FAILED test_quota 429 rate limit", now())
                .is_none()
        );
        let free = serde_json::json!({"type": "provider.free-limit", "message": "Free usage limit reached.",
            "status": 429, "retryAfterMs": 1000});
        assert!(opencode_refusal(&[free], "", now()).is_some());
        assert!(opencode_refusal(&[serde_json::json!({"name": "APIError", "statusCode": 429})], "", now()).is_some());
        assert!(
            opencode_refusal(&[serde_json::json!({"error": {"type": "FreeUsageLimitError"}})], "", now()).is_some()
        );
        assert!(
            opencode_refusal(
                &[serde_json::json!({"name": "UnknownError"})],
                r#"{"type":"error","error":{"type":"FreeUsageLimitError"}}"#,
                now()
            )
            .is_some()
        );
    }

    #[test]
    fn opencode_names_the_window_and_uses_the_reset_it_was_given() {
        let free = serde_json::json!({"type": "provider.free-limit", "message": "Free usage limit reached.",
            "status": 429, "retryAfterMs": 3_600_000});
        let refusal = opencode_refusal(&[free], "", now()).unwrap();
        assert_eq!(refusal.window, "daily");
        assert_eq!(refusal.source, Source::Named);
        assert_eq!(refusal.until_ms, now() + 3_600_000);
        assert_eq!(refusal.message, "Free usage limit reached.");

        // Free's daily reset is 00:00 UTC, as parse_reset's default: now is 13:00.
        let no_reset = serde_json::json!({"type": "provider.free-limit",
            "message": "Free usage limit reached.", "status": 429});
        let refusal = opencode_refusal(&[no_reset], "", now()).unwrap();
        assert_eq!((refusal.window.as_str(), refusal.source), ("daily", Source::Estimate));
        assert_eq!(refusal.until_ms, now() + 11 * 3_600_000);

        let weekly = serde_json::json!({"message": "reached your weekly usage limit", "status": 429});
        let refusal = opencode_refusal(&[weekly], "", now()).unwrap();
        assert_eq!(refusal.window, "weekly");
        assert_eq!(refusal.source, Source::Estimate);
        assert_eq!(refusal.until_ms, now() + 7 * 86_400_000);

        let five = serde_json::json!({"message": "5-hour limit", "status": 429});
        assert_eq!(opencode_refusal(&[five], "", now()).unwrap().window, "5-hour");
        let monthly = serde_json::json!({"message": "monthly limit reached", "status": 429});
        assert_eq!(opencode_refusal(&[monthly], "", now()).unwrap().window, "monthly");
        let resets = serde_json::json!({"message": "monthly limit reached; resets in 2d 3h", "status": 429});
        let refusal = opencode_refusal(&[resets], "", now()).unwrap();
        assert_eq!(refusal.source, Source::Named);
        assert_eq!(refusal.until_ms, now() + 2 * 86_400_000 + 3 * 3_600_000);
    }

    #[test]
    fn command_code_takes_its_own_sentences_and_exit_codes_alone_are_no_evidence() {
        let now = now();
        let weekly = command_code_refusal(
            "You've reached your weekly usage limit. Resets in 4d 22h (Sat 1:55).",
            true,
            now,
            180,
        )
        .unwrap();
        assert_eq!((weekly.window.as_str(), weekly.source), ("weekly", Source::Named));
        assert!(weekly.until_ms > now + 4 * 86_400_000);
        assert!(weekly.until_ms < now + 5 * 86_400_000);

        let before = now - 3 * 3_600_000; // 10:00 UTC is 13:00 at UTC+3
        let five = command_code_refusal(
            "You've reached your 5-hour usage limit for your plan. Your limit resets at 3:00 PM.",
            true,
            before,
            180,
        )
        .unwrap();
        assert_eq!((five.window.as_str(), five.source), ("5-hour", Source::Named));
        assert_eq!(five.until_ms, before + 2 * 3_600_000, "15:00 at UTC+3 is 12:00 UTC");
        let after = before + 3 * 3_600_000; // 16:00 local: tomorrow's 15:00
        let later = command_code_refusal(
            "You've reached your 5-hour usage limit for your plan. Your limit resets at 3:00 PM.",
            true,
            after,
            180,
        )
        .unwrap();
        assert_eq!(later.until_ms, after + 23 * 3_600_000);

        let credits = command_code_refusal("insufficient credits", true, now, 0).unwrap();
        assert_eq!((credits.window.as_str(), credits.source), ("credits", Source::Estimate));
        assert_eq!(credits.until_ms, now + 3_600_000);
        let auth = command_code_refusal("Not authenticated", true, now, 0).unwrap();
        assert_eq!((auth.window.as_str(), auth.source), ("auth", Source::Estimate));
        let rate = command_code_refusal("rate limit exceeded", true, now, 0).unwrap();
        assert_eq!((rate.window.as_str(), rate.source), ("rate", Source::Estimate));
        assert_eq!(rate.until_ms, now + 10 * 60_000);

        assert!(command_code_refusal("everything is fine", false, now, 0).is_none());
        for code_text in ["worker failed: assertion", "we discussed the rate limit handling"] {
            assert!(command_code_refusal(code_text, true, now, 0).is_none(), "{code_text}");
        }
        assert!(command_code_refusal("rate limit exceeded", false, now, 0).is_none());
    }

    #[test]
    fn a_clock_beside_the_estimate_wins_and_a_far_or_false_one_does_not() {
        let local = |day, hour, minute| {
            clock::to_ms(&Civil { year: 2026, month: 10, day, hour, minute, second: 0, weekday: 0 }, 180)
        };
        let now = local(26, 3, 0);
        let near = "You've reached your weekly usage limit. Resets in 4d 22h (Sat 1:55).";
        let refusal = command_code_refusal(near, true, now, 180).unwrap();
        assert_eq!((refusal.until_ms, refusal.source), (local(31, 1, 55), Source::Named));
        let far = "You've reached your weekly usage limit. Resets in 4d 10h (Sat 1:55).";
        assert_eq!(command_code_refusal(far, true, now, 180).unwrap().until_ms, local(30, 13, 0));
        let false_clock = "You've reached your weekly usage limit. Resets in 4d 22h (Sat 99:99).";
        assert_eq!(command_code_refusal(false_clock, true, now, 180).unwrap().until_ms, local(31, 1, 0));
    }

    #[test]
    fn both_clis_read_a_reset_with_the_one_pattern() {
        let opencode = serde_json::json!({"message": "limit; reset in 2h", "status": 429});
        let refusal = opencode_refusal(&[opencode], "", now()).unwrap();
        assert_eq!((refusal.until_ms, refusal.source), (now() + 2 * 3_600_000, Source::Named));
        let cc = "You've reached your weekly usage limit. Resets in 2h.";
        let refusal = command_code_refusal(cc, true, now(), 0).unwrap();
        assert_eq!((refusal.until_ms, refusal.source), (now() + 2 * 3_600_000, Source::Named));
    }

    #[test]
    fn a_garbled_number_can_neither_overflow_nor_block_for_good() {
        let most = now() + LONGEST_WAIT_MS;
        for error in [
            serde_json::json!({"status": 429, "message": "resets in 99999999999999999d"}),
            serde_json::json!({"status": 429, "message": "resets in 99999999999999999999999999h"}),
            serde_json::json!({"status": 429, "retryAfterMs": u64::MAX}),
            serde_json::json!({"status": 429, "message": "retry-after: 99999999999999999999999999"}),
        ] {
            assert_eq!(opencode_refusal(std::slice::from_ref(&error), "", now()).unwrap().until_ms, most, "{error}");
        }
        for text in [
            "You've reached your weekly usage limit. Resets in 99999999999999999d (Sat 1:55).",
            "You've reached your weekly usage limit. Resets in 99999999999999999999999999d.",
        ] {
            let until = command_code_refusal(text, true, now(), 180).unwrap().until_ms;
            assert!(until <= most + 3 * 3_600_000, "{text}");
        }
    }

    #[test]
    fn the_message_is_cut_to_three_hundred_characters() {
        let text = format!("insufficient credits {}", "x".repeat(1000));
        let refusal = command_code_refusal(&text, true, now(), 0).unwrap();
        assert_eq!(refusal.message.chars().count(), 300);
    }

    #[test]
    fn a_codex_usage_limit_reads_its_apostrophe_clock_and_date() {
        let local = |day, hour, minute| {
            clock::to_ms(&Civil { year: 2026, month: 10, day, hour, minute, second: 0, weekday: 0 }, 180)
        };
        let nine = local(26, 9, 0);
        for message in [
            "You’ve hit your usage limit for the 5-hour limit. Try again at 1:36 PM.",
            "You've hit your usage limit. Try again at 1:36 PM.",
        ] {
            let refusal = codex_refusal(&[message.to_string()], nine, 180).unwrap();
            assert_eq!((refusal.window.as_str(), refusal.source), ("5-hour", Source::Named), "{message}");
            assert_eq!(refusal.until_ms, local(26, 13, 36), "{message}");
        }

        let sixteen = local(26, 16, 0);
        let passed =
            codex_refusal(&["You’ve hit your usage limit. Try again at 1:36 PM.".to_string()], sixteen, 180).unwrap();
        assert_eq!(passed.until_ms, local(27, 13, 36), "a clock that passed is tomorrow");
        assert_eq!(passed.window, "weekly", "tomorrow's reset is further than five hours");

        let dated =
            codex_refusal(&["You’ve hit your usage limit. Try again at Nov 2nd, 2026 2:14 AM.".to_string()], nine, 180)
                .unwrap();
        assert_eq!((dated.window.as_str(), dated.source), ("weekly", Source::Named));
        let november =
            clock::to_ms(&Civil { year: 2026, month: 11, day: 2, hour: 2, minute: 14, second: 0, weekday: 0 }, 180);
        assert_eq!(dated.until_ms, november, "a date is that local moment");
    }

    #[test]
    fn a_codex_limit_without_a_time_estimates_and_credits_are_their_own_window() {
        let nine =
            clock::to_ms(&Civil { year: 2026, month: 10, day: 26, hour: 9, minute: 0, second: 0, weekday: 0 }, 180);
        for message in ["You’ve hit your usage limit. Try again later.", "You've hit your usage limit."] {
            let refusal = codex_refusal(&[message.to_string()], nine, 180).unwrap();
            assert_eq!((refusal.window.as_str(), refusal.source), ("5-hour", Source::Estimate), "{message}");
            assert_eq!(refusal.until_ms, nine + 5 * 3_600_000, "{message}");
        }
        let credits = codex_refusal(&["Your workspace is out of credits.".to_string()], nine, 180).unwrap();
        assert_eq!((credits.window.as_str(), credits.source), ("credits", Source::Estimate));
        assert_eq!(credits.until_ms, nine + 3_600_000, "as Command Code's credits refusal");
        assert!(codex_refusal(&["the tool exploded".to_string()], nine, 180).is_none());
    }
}
