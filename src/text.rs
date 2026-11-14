//! Text the module writes itself from a stranger's text: excerpts, reply links, labels. Each
//! is still visitor-derived, so it reaches a page only through `$h{…}`.

use ikigai_fn::html::url_segment;

/// The longest excerpt, in characters, before the ellipsis.
pub const EXCERPT_CHARS: usize = 100;

/// One line from `text`: every run of whitespace or control characters (line breaks,
/// tabs, bidirectional overrides and isolates, zero-width marks) becomes one space, the
/// ends are trimmed, and anything past [`EXCERPT_CHARS`] is cut at a word boundary where
/// one is near, with an ellipsis.
pub fn excerpt(text: &str) -> String {
    let mut line = String::new();
    let mut space = false;
    for c in text.chars() {
        if c.is_whitespace() || c.is_control() || is_format(c) {
            space = !line.is_empty();
        } else {
            if space {
                line.push(' ');
                space = false;
            }
            line.push(c);
        }
    }
    if line.chars().count() <= EXCERPT_CHARS {
        return line;
    }
    let cut: String = line.chars().take(EXCERPT_CHARS).collect();
    let cut = match cut.rfind(' ') {
        Some(at) if cut[..at].chars().count() >= EXCERPT_CHARS * 3 / 4 => &cut[..at],
        _ => cut.as_str(),
    };
    format!("{}…", cut.trim_end())
}

/// The invisible formatting characters an excerpt drops: bidirectional embeddings,
/// overrides, isolates and marks, zero-width joiners and spaces, the BOM.
fn is_format(c: char) -> bool {
    matches!(c,
        '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{2064}'
        | '\u{2066}'..='\u{2069}' | '\u{FEFF}' | '\u{061C}')
}

/// A `mailto:` link to `address` with a subject naming `site`, or `None` when the address
/// is not one plain `local@domain` a link can carry unchanged: no spaces, controls, or the
/// `?`, `&`, `#`, `%` and quote characters that would change what the link means.
pub fn mailto(address: &str, site: &str) -> Option<String> {
    let (local, domain) = address.split_once('@')?;
    let local_ok = !local.is_empty()
        && local.len() <= 64
        && local
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"!$'*+-/=^_`{|}~.".contains(&b));
    let domain_ok = !domain.is_empty()
        && domain.len() <= 253
        && domain.contains('.')
        && domain.split('.').all(|label| {
            !label.is_empty()
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        });
    (local_ok && domain_ok).then(|| {
        format!(
            "mailto:{local}@{domain}?subject={}",
            url_segment(&format!("Your inquiry on {site}"))
        )
    })
}

/// An `xsd:dateTime` as a person reads it: `2026-11-14 09:30 UTC`. Anything that is not
/// the layer's own `YYYY-MM-DDTHH:MM…Z` is shown as it is.
pub fn when(stamp: &str) -> String {
    let b = stamp.as_bytes();
    let shaped = b.len() >= 17
        && stamp.ends_with('Z')
        && b[4] == b'-'
        && b[7] == b'-'
        && b[10] == b'T'
        && b[13] == b':'
        && [0, 1, 2, 3, 5, 6, 8, 9, 11, 12, 14, 15]
            .iter()
            .all(|&i| b[i].is_ascii_digit());
    if shaped {
        format!("{} {} UTC", &stamp[..10], &stamp[11..16])
    } else {
        stamp.to_string()
    }
}

/// What sent an inquiry to its layer, in words.
pub fn reason(word: Option<&str>) -> String {
    match word {
        Some("score") => "the filter's score".to_string(),
        Some("blocked") => "a blocked sender".to_string(),
        Some("example") => "closeness to confirmed spam".to_string(),
        Some("model") => "the local model".to_string(),
        Some("flagged") => "a person flagged it".to_string(),
        Some("not-spam") => "a person marked it not spam".to_string(),
        Some(other) => other.to_string(),
        None => "not recorded".to_string(),
    }
}

/// `site` as lowercase hex: a key that is safe in any IRI segment and HTML id.
pub fn key(site: &str) -> String {
    site.bytes().map(|b| format!("{b:02x}")).collect()
}

/// The site a [`key`] names, if it names one.
pub fn unkey(key: &str) -> Option<String> {
    if !key.len().is_multiple_of(2) {
        return None;
    }
    let bytes = (0..key.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(key.get(i..i + 2)?, 16).ok())
        .collect::<Option<Vec<u8>>>()?;
    String::from_utf8(bytes).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_excerpt_is_one_line() {
        assert_eq!(
            excerpt("  Hello\r\n\tthere  \u{202E}you "),
            "Hello there you"
        );
        assert_eq!(excerpt("a\u{200B}b"), "a b");
        assert_eq!(excerpt(""), "");
    }

    #[test]
    fn a_long_excerpt_is_cut_with_an_ellipsis() {
        let long = "word ".repeat(40);
        let cut = excerpt(&long);
        assert!(cut.ends_with('…'));
        assert!(cut.chars().count() <= EXCERPT_CHARS + 1);
        assert!(!cut.contains("  "));
        let unbroken = "x".repeat(300);
        assert_eq!(excerpt(&unbroken).chars().count(), EXCERPT_CHARS + 1);
    }

    #[test]
    fn only_a_plain_address_becomes_a_link() {
        assert_eq!(
            mailto("ada@example.org", "a.example").as_deref(),
            Some("mailto:ada@example.org?subject=Your%20inquiry%20on%20a.example")
        );
        for bad in [
            "ada",
            "@example.org",
            "ada@",
            "ada@localhost",
            "a da@example.org",
            "ada@example.org?cc=x@y.z",
            "ada@example.org&body=x",
            "a%40@example.org",
            "\"ada\"@example.org",
            "ada@exa mple.org",
            "javascript:alert(1)//@example.org",
        ] {
            assert_eq!(mailto(bad, "s"), None, "{bad}");
        }
    }

    #[test]
    fn a_stamp_reads_as_a_time() {
        assert_eq!(when("2026-11-14T09:30:54.123Z"), "2026-11-14 09:30 UTC");
        assert_eq!(when("yesterday"), "yesterday");
    }

    #[test]
    fn a_key_round_trips() {
        for site in ["a.example", "", "<b>ü</b>"] {
            assert_eq!(unkey(&key(site)).as_deref(), Some(site));
        }
        assert_eq!(unkey("zz"), None);
        assert_eq!(unkey("a"), None);
    }
}
