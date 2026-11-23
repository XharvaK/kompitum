//! A rate limit's wait: the same worker tries again, at 60 s and then 180 s.

use crate::task::clock;

/// How long the nth rate refusal in a task waits before the same worker tries
/// again. The third has no wait: the worker is blocked and the turn hands on.
pub fn wait_ms(refusals: u32) -> Option<u64> {
    match refusals {
        1 => Some(60_000),
        2 => Some(180_000),
        _ => None,
    }
}

/// What a queued task says while it waits for its worker's next try.
pub fn sentence(label: &str, until_ms: u64, now_ms: u64, offset_min: i32) -> String {
    format!("rate limited: trying {label} again {}", clock::when_words(until_ms, now_ms, offset_min))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rate_refusal_waits_a_minute_then_three_minutes_and_then_hands_on() {
        assert_eq!(wait_ms(1), Some(60_000));
        assert_eq!(wait_ms(2), Some(180_000));
        assert_eq!(wait_ms(3), None);
        assert_eq!(wait_ms(4), None);
    }
}
