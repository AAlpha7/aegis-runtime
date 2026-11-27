//! Outbound privacy guard: strips street addresses, VINs and phone numbers from
//! arguments before they leave the box through network tools (web search,
//! web_reader, http_api, …). The model may use these details internally; a
//! third-party search engine does not need them (2026-09-08: the owner's home
//! address went out verbatim inside a web-search prompt and a Maps URL).

use regex::Regex;
use std::sync::LazyLock;

pub const STREET_PLACEHOLDER: &str = "[street address withheld]";
pub const VIN_PLACEHOLDER: &str = "[vehicle id withheld]";
pub const PHONE_PLACEHOLDER: &str = "[phone withheld]";

// "123 Example Pl SE", "456 Example Ave NE", "123+Example+Pl+SE" (URL form),
// "1 Example Corp Way". Number, up to four name tokens, a street suffix, an
// optional quadrant.
static STREET: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\b\d{1,6}(?:\s|\+|%20)+(?:[A-Za-z0-9'.\-]+(?:\s|\+|%20)+){0,4}(?:st|street|ave|avenue|pl|place|rd|road|dr|drive|ln|lane|way|blvd|boulevard|ct|court|cir|circle|ter|terrace|hwy|highway|pkwy|parkway)\b\.?(?:(?:\s|\+|%20)+(?:n|s|e|w|ne|nw|se|sw)\b)?",
    )
    .expect("street regex")
});
// 16–17 uppercase alphanumerics without I/O/Q (VIN alphabet; the stored
// value may be truncated); must mix letters and digits.
static VIN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b[A-HJ-NPR-Z0-9]{16,17}\b").expect("vin regex"));
static PHONE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:\+?1[\s.\-]?)?\(?\b\d{3}\)?[\s.\-]\d{3}[\s.\-]\d{4}\b").expect("phone regex")
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Redaction {
    pub text: String,
    /// Placeholder kinds applied, in order of first appearance.
    pub hits: Vec<&'static str>,
}

/// Redact private literals from free text. Returns the text unchanged with no
/// hits when nothing matched.
pub fn redact_outbound_text(text: &str) -> Redaction {
    let mut hits: Vec<&'static str> = Vec::new();
    let mut out = STREET.replace_all(text, STREET_PLACEHOLDER).into_owned();
    if out != text {
        hits.push("street_address");
    }
    let before = out.clone();
    out = VIN
        .replace_all(&out, |caps: &regex::Captures| {
            let m = caps.get(0).map(|m| m.as_str()).unwrap_or("");
            let has_digit = m.chars().any(|c| c.is_ascii_digit());
            let has_alpha = m.chars().any(|c| c.is_ascii_alphabetic());
            if has_digit && has_alpha {
                VIN_PLACEHOLDER.to_string()
            } else {
                m.to_string()
            }
        })
        .into_owned();
    if out != before {
        hits.push("vehicle_id");
    }
    let before = out.clone();
    out = PHONE.replace_all(&out, PHONE_PLACEHOLDER).into_owned();
    if out != before {
        hits.push("phone");
    }
    Redaction { text: out, hits }
}

/// Redact every string inside a JSON value in place; returns the kinds hit.
pub fn redact_outbound_json(value: &mut serde_json::Value) -> Vec<&'static str> {
    let mut hits: Vec<&'static str> = Vec::new();
    fn walk(value: &mut serde_json::Value, hits: &mut Vec<&'static str>) {
        match value {
            serde_json::Value::String(s) => {
                let r = redact_outbound_text(s);
                if !r.hits.is_empty() {
                    for h in r.hits {
                        if !hits.contains(&h) {
                            hits.push(h);
                        }
                    }
                    *s = r.text;
                }
            }
            serde_json::Value::Array(items) => items.iter_mut().for_each(|v| walk(v, hits)),
            serde_json::Value::Object(map) => map.values_mut().for_each(|v| walk(v, hits)),
            _ => {}
        }
    }
    walk(value, &mut hits);
    hits
}

/// One-line note for the model explaining what was withheld.
pub fn privacy_note(hits: &[&'static str]) -> String {
    format!(
        "[privacy] withheld from the outbound request: {}. Use city-level places for external lookups.",
        hits.join(", ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn street_addresses_are_withheld_in_prose_and_urls() {
        let r = redact_outbound_text(
            "commute from Example City, WA (near 123 Example Pl SE) to 456 Example Ave NE Worktown",
        );
        assert_eq!(
            r.text,
            "commute from Example City, WA (near [street address withheld]) to [street address withheld] Worktown"
        );
        assert_eq!(r.hits, vec!["street_address"]);

        let url = redact_outbound_text(
            "https://www.google.com/maps/dir/123+Example+Pl+SE,+Example+City,+WA+00000/456+Example+Ave+NE,+Worktown",
        );
        assert!(!url.text.contains("123 Example"), "{}", url.text);
        assert!(!url.text.contains("148th"), "{}", url.text);
    }

    #[test]
    fn city_level_text_and_ordinary_numbers_pass_through() {
        for text in [
            "drive time from Example City WA to Worktown WA on weekday mornings",
            "the 2026 Tesla Model 3 gets 5 stars",
            "meeting at 15:30, budget 1200 dollars, 42 km",
            "Highway 2 was closed",
        ] {
            let r = redact_outbound_text(text);
            assert_eq!(r.text, text, "must not touch: {text}");
            assert!(r.hits.is_empty());
        }
    }

    #[test]
    fn vin_and_phone_are_withheld() {
        let r = redact_outbound_text("VIN 5YJSE1E9RT156789, call 425-555-0142 or (206) 555 0199");
        assert_eq!(
            r.text,
            "VIN [vehicle id withheld], call [phone withheld] or [phone withheld]"
        );
        assert_eq!(r.hits, vec!["vehicle_id", "phone"]);
        // A 17-char hex commit id is not a VIN (lowercase / no mixed alphabet).
        let h = redact_outbound_text("commit 71609494b90f0fc3a");
        assert!(h.hits.is_empty());
    }

    #[test]
    fn json_payloads_are_redacted_recursively() {
        let mut v = serde_json::json!({
            "url": "https://maps.example/dir/123+Example+Pl+SE/456+Example+Ave+NE",
            "nested": { "note": "lives at 123 Example Pl SE" },
            "count": 3
        });
        let hits = redact_outbound_json(&mut v);
        assert_eq!(hits, vec!["street_address"]);
        assert!(!v.to_string().contains("123 Example"));
        assert_eq!(v["count"], 3);
    }
}
