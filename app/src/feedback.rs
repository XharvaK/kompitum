//! Explicit feedback becomes a GitHub issue the person reads and submits.
//! Nothing is sent from here.
const ISSUE_URL: &str = "https://github.com/XharvaK/kompitum/issues/new";
const MAX_URL_BYTES: usize = 7500;

/// The issue a person or agent can read and submit themselves.
pub fn issue_url(message: &str, from: &str) -> String {
    let title = title(message);
    let message = message.trim();
    let url = url_for(&title, &body(message, from));
    if url.len() <= MAX_URL_BYTES {
        return url;
    }
    // The longest start of the message that fits with "…" after it. A character can take up
    // to 12 bytes once encoded, so the cut is searched for, not computed from the excess.
    let starts: Vec<usize> = message.char_indices().map(|(i, _)| i).collect();
    let cut = |n: usize| url_for(&title, &body(&format!("{}…", &message[..starts[n]]), from));
    // cut(lo) fits (the bare "…" always does); the whole message, at hi, does not.
    let (mut lo, mut hi) = (0, starts.len());
    while hi - lo > 1 {
        let mid = (lo + hi) / 2;
        if cut(mid).len() <= MAX_URL_BYTES {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    cut(lo)
}

/// The message's first non-empty line, trimmed, at most 80 characters.
fn title(message: &str) -> String {
    message
        .lines()
        .find(|line| !line.trim().is_empty())
        .map(str::trim)
        .unwrap_or_default()
        .chars()
        .take(80)
        .collect()
}

fn body(message: &str, from: &str) -> String {
    let core = crate::update::core()
        .version
        .as_deref()
        .map(crate::telemetry::version)
        .unwrap_or_else(|| "unknown".to_owned());
    let mut body = String::new();
    if from == "agent" {
        body.push_str("_Drafted by an agent: read it before you submit._\n\n");
    }
    body.push_str(message);
    body.push_str("\n\n---\n");
    body.push_str(&format!(
        "Kompitum {} · kom {} · {} {}",
        env!("CARGO_PKG_VERSION"),
        core,
        std::env::consts::OS,
        std::env::consts::ARCH,
    ));
    body
}

fn url_for(title: &str, body: &str) -> String {
    format!("{ISSUE_URL}?title={}&body={}", encode(title), encode(body))
}

/// Percent-encode every byte except `A-Z a-z 0-9 - . _ ~`.
fn encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode(text: &str) -> String {
        let bytes = text.as_bytes();
        let mut out = Vec::new();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'%' && i + 2 < bytes.len() {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap();
                out.push(u8::from_str_radix(hex, 16).unwrap());
                i += 3;
            } else {
                out.push(bytes[i]);
                i += 1;
            }
        }
        String::from_utf8(out).unwrap()
    }

    fn parts(url: &str) -> (String, String) {
        let (_, query) = url.split_once('?').unwrap();
        let (title, body) = query.split_once("&body=").unwrap();
        (decode(title.strip_prefix("title=").unwrap()), decode(body))
    }

    fn version_line() -> String {
        let core = crate::update::core()
            .version
            .as_deref()
            .map(crate::telemetry::version)
            .unwrap_or_else(|| "unknown".to_owned());
        format!(
            "Kompitum {} · kom {} · {} {}",
            env!("CARGO_PKG_VERSION"),
            core,
            std::env::consts::OS,
            std::env::consts::ARCH,
        )
    }

    #[test]
    fn title_is_the_first_line_and_the_body_ends_with_versions() {
        let url = issue_url("  \n  Broken pane  \nmore\n", "person");
        let (got_title, body) = parts(&url);
        assert_eq!(got_title, "Broken pane");
        assert!(body.starts_with("Broken pane  \nmore\n"), "{body:?}");
        assert!(
            body.ends_with(&format!("\n\n---\n{}", version_line())),
            "{body:?}"
        );
        let long = "x".repeat(100);
        assert_eq!(super::title(&long).chars().count(), 80);
        let (got_title, body) = parts(&issue_url("", "person"));
        assert_eq!(got_title, "");
        assert!(body.ends_with(&format!("\n\n---\n{}", version_line())));
    }

    #[test]
    fn agent_feedback_says_it_was_drafted() {
        let (_, body) = parts(&issue_url("Fix this", "agent"));
        assert!(
            body.starts_with("_Drafted by an agent: read it before you submit._\n\nFix this\n\n---\n"),
            "{body:?}"
        );
        let (_, body) = parts(&issue_url("Fix this", "person"));
        assert!(body.starts_with("Fix this\n\n---\n"), "{body:?}");
        assert!(!body.contains("Drafted by an agent"));
    }

    #[test]
    fn spaces_ampersands_hashes_newlines_and_umlauts_are_encoded() {
        let url = issue_url("a b&c#d\ne ü", "person");
        assert!(url.contains("title=a%20b%26c%23d"), "{url}");
        assert!(url.contains("a%20b%26c%23d%0Ae%20%C3%BC"), "{url}");
        let (_, body) = parts(&url);
        assert!(body.starts_with("a b&c#d\ne ü\n\n---\n"), "{body:?}");
    }

    #[test]
    fn long_messages_are_cut_on_a_char_boundary_until_the_url_fits() {
        let url = issue_url(&"x".repeat(20_000), "person");
        assert_eq!(url.len(), MAX_URL_BYTES);
        let (_, body) = parts(&url);
        let text = body.split("\n\n---\n").next().unwrap();
        assert!(text.ends_with('…'), "{text:?}");

        // Each ü is 6 bytes encoded: as much of it is kept as fits, not cut down to the "…".
        let url = issue_url(&"ü".repeat(10_000), "agent");
        assert!(url.len() <= MAX_URL_BYTES && MAX_URL_BYTES - url.len() < 6, "{}", url.len());
        let (_, body) = parts(&url);
        let text = body.split("\n\n---\n").next().unwrap();
        let text = text
            .strip_prefix("_Drafted by an agent: read it before you submit._\n\n")
            .unwrap();
        assert!(text.ends_with('…'), "{text:?}");
        assert!(
            text[..text.len() - '…'.len_utf8()]
                .chars()
                .all(|c| c == 'ü'),
            "cut on a char boundary"
        );
    }
}
