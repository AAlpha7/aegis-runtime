//! Sensitivity-tier wire-format helpers.
//!
//! Phase C1 U5 fix: `format!("{:?}", tier)` yielded PascalCase; the type's
//! `#[serde(rename_all = "snake_case")]` attribute yields `"ordinary"` etc.
//! Every call-site that stores `sensitivity_tier` in a DB column must use
//! the snake_case form so the value round-trips through
//! `serde_json::from_str::<SensitivityTier>`.

use crate::heirloom::enums::SensitivityTier;

/// Return the canonical snake_case wire representation of a `SensitivityTier`.
/// Matches the strings produced by `serde_json::to_string` on the type (with
/// surrounding quotes stripped).
pub fn sensitivity_tier_wire_str(tier: &SensitivityTier) -> &'static str {
    match tier {
        SensitivityTier::Public => "public",
        SensitivityTier::Ordinary => "ordinary",
        SensitivityTier::Sensitive => "sensitive",
        SensitivityTier::Confidential => "confidential",
        SensitivityTier::Secret => "secret",
        SensitivityTier::Sacred => "sacred",
    }
}

/// Inverse: parse a snake_case tier string back. Case-sensitive.
///
/// **Caller**: Phase C3 `HeirloomReader` Filter gate 2 (Sensitivity) will
/// call this to parse the `heirloom_event_log.sensitivity_tier` TEXT column
/// back into a typed `SensitivityTier` for comparison against the query's
/// tier ceiling. Also useful for any debug tool that inspects raw DB rows.
pub fn sensitivity_tier_from_wire(s: &str) -> Option<SensitivityTier> {
    match s {
        "public" => Some(SensitivityTier::Public),
        "ordinary" => Some(SensitivityTier::Ordinary),
        "sensitive" => Some(SensitivityTier::Sensitive),
        "confidential" => Some(SensitivityTier::Confidential),
        "secret" => Some(SensitivityTier::Secret),
        "sacred" => Some(SensitivityTier::Sacred),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t10_wire_str_matches_serde_for_all_tiers() {
        for tier in [
            SensitivityTier::Public,
            SensitivityTier::Ordinary,
            SensitivityTier::Sensitive,
            SensitivityTier::Confidential,
            SensitivityTier::Secret,
            SensitivityTier::Sacred,
        ] {
            let serde = serde_json::to_string(&tier).unwrap();
            // serde_json produces "\"ordinary\""; strip surrounding quotes.
            let serde_stripped = serde.trim_matches('"');
            let helper = sensitivity_tier_wire_str(&tier);
            assert_eq!(
                helper, serde_stripped,
                "helper and serde must agree for {tier:?}"
            );
        }
    }

    #[test]
    fn t10_wire_str_is_strictly_lowercase_snake_case() {
        for tier in [
            SensitivityTier::Public,
            SensitivityTier::Ordinary,
            SensitivityTier::Sensitive,
            SensitivityTier::Confidential,
            SensitivityTier::Secret,
            SensitivityTier::Sacred,
        ] {
            let s = sensitivity_tier_wire_str(&tier);
            assert!(
                s.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
                "{s} is not snake_case",
            );
        }
    }

    #[test]
    fn t10_from_wire_roundtrip_every_tier() {
        for tier in [
            SensitivityTier::Public,
            SensitivityTier::Ordinary,
            SensitivityTier::Sensitive,
            SensitivityTier::Confidential,
            SensitivityTier::Secret,
            SensitivityTier::Sacred,
        ] {
            let s = sensitivity_tier_wire_str(&tier);
            let back = sensitivity_tier_from_wire(s).expect("roundtrip");
            assert_eq!(back, tier);
        }
    }

    #[test]
    fn t10_from_wire_rejects_pascalcase() {
        assert_eq!(sensitivity_tier_from_wire("Ordinary"), None);
        assert_eq!(sensitivity_tier_from_wire("ORDINARY"), None);
    }

    #[test]
    fn t10_from_wire_rejects_unknown() {
        assert_eq!(sensitivity_tier_from_wire(""), None);
        assert_eq!(sensitivity_tier_from_wire("garbage"), None);
        assert_eq!(sensitivity_tier_from_wire("top_secret"), None);
    }
}
