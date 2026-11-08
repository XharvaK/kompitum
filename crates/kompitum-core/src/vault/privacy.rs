//! What never goes into the vault: keys, tokens and passwords, platform ids,
//! IP addresses. A note that holds anything shaped like one is refused before
//! it is written, and the refusal names the kind, never the text.

use regex::Regex;
use std::sync::OnceLock;

fn rules() -> &'static [(&'static str, Regex)] {
    static RULES: OnceLock<Vec<(&'static str, Regex)>> = OnceLock::new();
    RULES.get_or_init(|| {
        vec![
            ("a private key block", regex(r"-----BEGIN [A-Z ]*PRIVATE KEY-----")),
            ("a provider key", regex(r"(?-u:\b)(sk|rk|pk)-[A-Za-z0-9_-]{16,}")),
            ("a provider key", regex(r"(?-u:\b)[a-z]{2,16}_(sk|pk|live|test|secret|key|api)_[A-Za-z0-9_-]{12,}")),
            (
                "a GitHub token",
                regex(r"(?-u:\b)(ghp|gho|ghu|ghs|ghr)_[A-Za-z0-9]{20,}|(?-u:\b)github_pat_[A-Za-z0-9_]{20,}"),
            ),
            ("a Slack token", regex(r"(?-u:\b)xox[abprs]-[A-Za-z0-9-]{10,}")),
            ("a cloud key id", regex(r"(?-u:\b)AKIA[0-9A-Z]{16}(?-u:\b)")),
            ("a bot token", regex(r"(?-u:\b)[A-Za-z0-9_-]{23,28}\.[A-Za-z0-9_-]{6}\.[A-Za-z0-9_-]{27,}")),
            (
                "a password line",
                regex(r"(?i)(?-u:\b)(password|passwd|api[_-]?key|secret|token)[ \t\n\r\f\v]*[:=][ \t\n\r\f\v]*\S{6,}"),
            ),
            (
                "an IP address",
                regex(r"(?-u:\b)(25[0-5]|2[0-4][0-9]|1?[0-9]?[0-9])(\.(25[0-5]|2[0-4][0-9]|1?[0-9]?[0-9])){3}(?-u:\b)"),
            ),
        ]
    })
}

fn regex(pattern: &str) -> Regex {
    Regex::new(pattern).expect("a built-in privacy pattern compiles")
}

/// Long mixed-case letter-and-digit runs read as secrets. Plain hex (commit
/// and content hashes) does not.
fn token_shaped() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"[A-Za-z0-9_+/=-]{32,}").expect("a token-shaped run"))
}

fn hex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^[0-9a-f]+$").expect("hex"))
}

/// The kinds of private text found, one entry per kind. Empty when the text is
/// fit for the vault.
pub fn private_kinds(text: &str) -> Vec<&'static str> {
    let mut found = Vec::new();
    for (kind, pattern) in rules() {
        if !found.contains(kind) && pattern.is_match(text) {
            found.push(*kind);
        }
    }
    if !found.contains(&"a platform id") && has_platform_id(text) {
        found.push("a platform id");
    }
    if !found.contains(&"a token-shaped value") {
        for run in token_shaped().find_iter(text) {
            let run = run.as_str();
            if hex().is_match(run) {
                continue;
            }
            let (mut upper, mut lower, mut digit) = (false, false, false);
            for ch in run.chars() {
                upper |= ch.is_ascii_uppercase();
                lower |= ch.is_ascii_lowercase();
                digit |= ch.is_ascii_digit();
            }
            if upper && lower && digit {
                found.push("a token-shaped value");
                break;
            }
        }
    }
    found
}

/// `(?<![\d.])\d{17,20}(?![\d.])`, which the regex crate cannot write without
/// look-around: a run of 17 to 20 digits, with no digit or dot beside it.
fn has_platform_id(text: &str) -> bool {
    let bytes = text.as_bytes();
    let mut at = 0;
    while at < bytes.len() {
        if !bytes[at].is_ascii_digit() {
            at += 1;
            continue;
        }
        let start = at;
        while at < bytes.len() && bytes[at].is_ascii_digit() {
            at += 1;
        }
        let len = at - start;
        if (17..=20).contains(&len) {
            let before = start == 0 || (bytes[start - 1] != b'.' && !bytes[start - 1].is_ascii_digit());
            let after = at == bytes.len() || (bytes[at] != b'.' && !bytes[at].is_ascii_digit());
            if before && after {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn join(parts: &[&str]) -> String {
        parts.concat()
    }

    #[test]
    fn passes_ordinary_notes_commit_hashes_and_dates() {
        assert_eq!(
            private_kinds("Natalis 60f625cd0a1b2c3d4e5f60718293a4b5c6d7e8f9 is live since 2026-10-19T12:08+03:00."),
            Vec::<&str>::new()
        );
        assert_eq!(private_kinds("river seq 6134, 33 rows, 1718 tests, version 1.32.1"), Vec::<&str>::new());
    }

    #[test]
    fn names_each_kind_it_finds_never_the_text() {
        let cases = [
            (join(&["sk", "-", "abcdefghij", "klmnopqrst"]), "a provider key"),
            (join(&["moltbook", "_sk_", "Ab3dEf6hIj9kLm2n"]), "a provider key"),
            (join(&["gh", "p_", "aB3dE6gH9jK2mN5pQ8sT1vW4"]), "a GitHub token"),
            (join(&["AKIA", "ABCDEFGHIJKLMNOP"]), "a cloud key id"),
            (join(&["pass", "word: ", "hunter22"]), "a password line"),
            (join(&["10.", "0.", "0.", "12"]), "an IP address"),
            (join(&["1234567", "8901234567"]), "a platform id"),
            (join(&["aB3dE6gH9jK2mN5p", "Q8sT1vW4yZ7bC0eF"]), "a token-shaped value"),
            (join(&["-----BEGIN ", "OPENSSH PRIVATE KEY-----"]), "a private key block"),
        ];
        for (value, kind) in cases {
            let kinds = private_kinds(&format!("note: {value} here"));
            assert!(kinds.contains(&kind), "{kind} not in {kinds:?}");
            assert!(!kinds.join(" ").contains(&value), "the text itself is never named");
        }
    }

    #[test]
    fn word_edges_and_digits_are_ascii_as_in_javascript() {
        assert!(private_kinds(&join(&["é", "sk", "-", "abcdefghij", "klmnopqrst"])).contains(&"a provider key"));
        assert!(private_kinds(&join(&["ü", "pass", "word: ", "hunter22"])).contains(&"a password line"));
        assert_eq!(private_kinds(&join(&["٠١٢٣٤٥٦٧٨٩", "٠١٢٣٤٥٦٧٨٩"])), Vec::<&str>::new());
    }
}
