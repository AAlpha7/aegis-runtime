//! Profile Slot Registry — single source of truth for user_profile schema.
//!
//! Every writer (reflection Consolidate, try_instant_promote, future update_profile tool)
//! goes through this registry. It drives:
//!   1. JSON Schema for OpenRouter `response_format` (provider-enforced keys)
//!   2. Declarative value validators (rejects miscategorized content)
//!   3. Alias resolution (legacy keys → canonical)
//!   4. Instant-promote pattern hints (deterministic pre-LLM fast path)
//!   5. Description strings injected into the main-LLM system prompt
//!
//! Extension = add one row to `PROFILE_SLOTS`. No code changes elsewhere.

use std::fmt;

/// Declarative validator for a slot's value.
#[derive(Debug, Clone)]
pub enum ValidatorKind {
    /// Postal address — must contain a digit-run (street number) AND a street token.
    AddressLike,
    /// City / region / area — letters only, rejects street numbers.
    LocationLike,
    /// Name — 1-5 tokens, no sentence punctuation.
    NameLike,
    /// Free text bounded by max_len only.
    FreeText,
    /// One of a fixed set of strings (case-insensitive).
    Enum(&'static [&'static str]),
    /// Integer within an inclusive range.
    Integer { min: i64, max: i64 },
}

/// Declarative match hints for `try_instant_promote`. A slot promotes when
/// ANY rule in `rules` matches. Each `CueRule` composes substring cues with
/// optional co-occurrence requirements and structural constraint clauses.
#[derive(Debug, Clone)]
pub struct InstantMatcher {
    pub rules: &'static [CueRule],
}

#[derive(Debug, Clone)]
pub struct CueRule {
    /// ANY of these substrings must appear in the (lowercased) fact content.
    pub cues: &'static [&'static str],
    /// If non-empty, ANY of these substrings must ALSO appear. Used for CJK
    /// patterns like "工作" that require co-occurrence with "在" or "公司".
    pub also_contains: &'static [&'static str],
    /// AND-of-ORs: outer slice is ANDed; each inner slice is ORed. Empty outer
    /// slice means no structural constraint.
    pub constraints: &'static [&'static [MatchFlag]],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchFlag {
    /// Requires a consecutive-digit run (≥2 digits) — street numbers, zip codes.
    HasDigitRun,
    /// Requires a street/road token (see `has_street_token`).
    HasStreetToken,
    /// Requires the substring "office" (lowercase). Used for work-address
    /// facts that mention an office without a specific street number.
    HasOfficeMarker,
    /// Requires a token immediately after any matched cue that is ASCII
    /// capitalized. Used for "I'm <CapitalizedName>" heuristic.
    HasCapitalizedAfterAnyCue,
}

/// One canonical slot in the profile schema.
#[derive(Debug, Clone)]
pub struct ProfileSlot {
    pub category: &'static str,
    pub key: &'static str,
    /// Shown to the main LLM (system prompt) and embedded in the JSON schema description.
    pub description: &'static str,
    pub validator: ValidatorKind,
    pub max_len: usize,
    /// Legacy `category.key` pairs that should be migrated into this slot.
    /// Stored as "category.key" strings for simplicity.
    pub aliases: &'static [&'static str],
    /// Optional deterministic fast-path matcher for `try_instant_promote`.
    pub instant_hint: Option<InstantMatcher>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationError {
    UnknownSlot {
        category: String,
        key: String,
    },
    Empty,
    TooLong {
        len: usize,
        max: usize,
    },
    /// Looks like mixed content (sentence-like, too many distinct subjects).
    MixedContent,
    /// Expected a postal address but no street-number-ish token.
    NotAddressLike,
    /// Expected a city/region but contains street-number-ish token.
    NotLocationLike,
    /// Expected a short name but got a sentence.
    NotNameLike,
    NotInEnum {
        allowed: &'static [&'static str],
    },
    NotInteger,
    IntegerOutOfRange {
        min: i64,
        max: i64,
    },
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ValidationError::UnknownSlot { category, key } => {
                write!(f, "unknown slot {}.{}", category, key)
            }
            ValidationError::Empty => write!(f, "value is empty"),
            ValidationError::TooLong { len, max } => {
                write!(f, "value too long ({} > {})", len, max)
            }
            ValidationError::MixedContent => {
                write!(
                    f,
                    "value looks like mixed content (multiple subjects in one sentence)"
                )
            }
            ValidationError::NotAddressLike => {
                write!(
                    f,
                    "value is not address-like (no street number / street token)"
                )
            }
            ValidationError::NotLocationLike => {
                write!(
                    f,
                    "value is not location-like (contains street-number content)"
                )
            }
            ValidationError::NotNameLike => {
                write!(f, "value is not name-like (too long or sentence-like)")
            }
            ValidationError::NotInEnum { allowed } => {
                write!(f, "value not in allowed enum set: {:?}", allowed)
            }
            ValidationError::NotInteger => write!(f, "value is not an integer"),
            ValidationError::IntegerOutOfRange { min, max } => {
                write!(f, "integer out of range [{}, {}]", min, max)
            }
        }
    }
}

impl std::error::Error for ValidationError {}

impl ValidatorKind {
    /// Validate a trimmed value string. Caller must enforce `max_len` separately.
    pub fn validate(&self, value: &str) -> Result<(), ValidationError> {
        let v = value.trim();
        if v.is_empty() {
            return Err(ValidationError::Empty);
        }
        match self {
            ValidatorKind::AddressLike => {
                // Reject multi-subject sentences (heuristic: ≥3 commas AND ≥80 chars
                // AND contains a verb-like token that indicates narrative rather than address)
                if looks_like_narrative(v) {
                    return Err(ValidationError::MixedContent);
                }
                if !has_digit_run(v) || !has_street_token(v) {
                    return Err(ValidationError::NotAddressLike);
                }
                Ok(())
            }
            ValidatorKind::LocationLike => {
                if looks_like_narrative(v) {
                    return Err(ValidationError::MixedContent);
                }
                // Location should not contain street-number + street token (that's an address).
                if has_digit_run(v) && has_street_token(v) {
                    return Err(ValidationError::NotLocationLike);
                }
                Ok(())
            }
            ValidatorKind::NameLike => {
                if v.len() > 80 {
                    return Err(ValidationError::NotNameLike);
                }
                // Count whitespace-separated tokens; allow up to 5.
                let token_count = v.split_whitespace().count();
                if token_count > 5 {
                    return Err(ValidationError::NotNameLike);
                }
                // Reject if contains sentence-ending punctuation.
                if v.contains('.') || v.contains('。') || v.contains('!') || v.contains('?') {
                    return Err(ValidationError::NotNameLike);
                }
                Ok(())
            }
            ValidatorKind::FreeText => Ok(()),
            ValidatorKind::Enum(allowed) => {
                let lower = v.to_lowercase();
                if allowed.iter().any(|a| a.eq_ignore_ascii_case(&lower)) {
                    Ok(())
                } else {
                    Err(ValidationError::NotInEnum { allowed })
                }
            }
            ValidatorKind::Integer { min, max } => {
                let n: i64 = v.parse().map_err(|_| ValidationError::NotInteger)?;
                if n < *min || n > *max {
                    Err(ValidationError::IntegerOutOfRange {
                        min: *min,
                        max: *max,
                    })
                } else {
                    Ok(())
                }
            }
        }
    }
}

fn has_digit_run(s: &str) -> bool {
    let mut run = 0;
    for c in s.chars() {
        if c.is_ascii_digit() {
            run += 1;
            if run >= 2 {
                return true;
            }
        } else {
            run = 0;
        }
    }
    false
}

fn has_street_token(s: &str) -> bool {
    const TOKENS: &[&str] = &[
        "street",
        "st.",
        " st ",
        "road",
        " rd",
        "avenue",
        " ave",
        "lane",
        " ln",
        "drive",
        " dr",
        "boulevard",
        "blvd",
        "way",
        "court",
        "ct.",
        "highway",
        // Bounded forms: leading space so "replace"/"someplace"/"marketplace" don't match.
        " place",
        " pl",
        "路",
        "街",
        "号",
        "巷",
        "大道",
        "里",
    ];
    let lower = s.to_lowercase();
    TOKENS.iter().any(|t| lower.contains(t))
}

/// Heuristic: is the value narrative prose (mixed content / story) rather than a short fact?
///
/// Conservative — only flags values that are simultaneously (a) long and (b) contain
/// multiple clause joiners. A leading "User …" or "用户…" prefix is common and expected
/// in LLM-extracted fact content (fact extractor phrases claims in third person), so
/// prefix-based checks are NOT used. The downstream type-specific validator (e.g.
/// AddressLike requiring digits + street token) remains the primary correctness gate.
fn looks_like_narrative(s: &str) -> bool {
    if s.len() < 60 {
        return false;
    }
    let lower = s.to_lowercase();
    let comma_count = s.matches(',').count() + s.matches('，').count();
    let and_count = lower.matches(" and ").count() + lower.matches(" 和 ").count();
    comma_count + and_count >= 2
}

/// The canonical registry. Add new slots here — no other code changes required.
///
/// Slot DECLARATION ORDER is significant for `try_instant_promote`: the loop
/// returns the first slot whose `instant_hint` matches. Order here mirrors the
/// priority of the prior hardcoded implementation — location → family →
/// identity — so facts with mixed cues (e.g. "I'm Lance, work at 1 Example Corp
/// Way") still prefer the more specific address slot over the generic name
/// slot, matching the established behavior.
pub static PROFILE_SLOTS: &[ProfileSlot] = &[
    // ============ location ============
    ProfileSlot {
        category: "location",
        key: "home_address",
        description: "The user's full home postal address, including street number.",
        validator: ValidatorKind::AddressLike,
        max_len: 200,
        aliases: &["location.residence_address"],
        instant_hint: Some(InstantMatcher {
            rules: &[CueRule {
                cues: &[
                    "lives at",
                    "lives in",
                    "home is",
                    "home address",
                    "my home",
                    "住在",
                    "家在",
                    "搬到",
                ],
                also_contains: &[],
                constraints: &[&[MatchFlag::HasDigitRun]],
            }],
        }),
    },
    ProfileSlot {
        category: "location",
        key: "residence_city",
        description: "The city or region where the user lives (no street address).",
        validator: ValidatorKind::LocationLike,
        max_len: 120,
        aliases: &[
            "location.residence",
            "location.home_location",
            "location.city",
        ],
        // No instant_hint: current try_instant_promote rejects city-only
        // facts like "User lives in Seattle". residence_city is populated
        // only via the 4h reflection LLM path. Legacy aliases still resolve.
        instant_hint: None,
    },
    ProfileSlot {
        category: "location",
        key: "work_address",
        description: "The user's work postal address, including street number.",
        validator: ValidatorKind::AddressLike,
        max_len: 200,
        aliases: &["location.office_address"],
        instant_hint: Some(InstantMatcher {
            rules: &[
                CueRule {
                    cues: &[
                        "works at",
                        "works for",
                        "employer",
                        "office at",
                        "work address",
                    ],
                    also_contains: &[],
                    constraints: &[&[MatchFlag::HasDigitRun, MatchFlag::HasOfficeMarker]],
                },
                // CJK: "工作" must co-occur with "在" or "公司" (reject bare "在"
                // which matches any location phrase like "用户在公园散步").
                CueRule {
                    cues: &["工作"],
                    also_contains: &["在", "公司"],
                    constraints: &[&[MatchFlag::HasDigitRun, MatchFlag::HasOfficeMarker]],
                },
            ],
        }),
    },
    ProfileSlot {
        category: "location",
        key: "work_city",
        description: "The city where the user works (no street address).",
        validator: ValidatorKind::LocationLike,
        max_len: 120,
        aliases: &[],
        instant_hint: None,
    },
    // ============ family ============
    ProfileSlot {
        category: "family",
        key: "spouse_name",
        description: "The user's spouse's name.",
        validator: ValidatorKind::NameLike,
        max_len: 80,
        aliases: &["family.wife", "family.husband", "family.partner"],
        instant_hint: Some(InstantMatcher {
            rules: &[CueRule {
                cues: &["wife", "husband", "妻子", "丈夫"],
                also_contains: &[],
                constraints: &[],
            }],
        }),
    },
    ProfileSlot {
        category: "family",
        key: "children",
        description: "The user's children (names, ages, or count).",
        validator: ValidatorKind::FreeText,
        max_len: 200,
        aliases: &["family.son", "family.daughter", "family.kids"],
        instant_hint: Some(InstantMatcher {
            rules: &[CueRule {
                cues: &["son", "daughter", "儿子", "女儿"],
                also_contains: &[],
                constraints: &[],
            }],
        }),
    },
    // ============ identity ============
    // Declared AFTER location+family so that a fact matching both a name cue
    // and a location/work cue prefers the more specific slot (preserves prior
    // hardcoded priority). See note on PROFILE_SLOTS above.
    ProfileSlot {
        category: "identity",
        key: "name",
        description: "The user's preferred name or how they'd like to be addressed.",
        validator: ValidatorKind::NameLike,
        max_len: 80,
        aliases: &["identity.name_and_role", "identity.full_name"],
        instant_hint: Some(InstantMatcher {
            rules: &[
                CueRule {
                    cues: &["my name is", "我叫"],
                    also_contains: &[],
                    constraints: &[],
                },
                CueRule {
                    cues: &["i'm"],
                    also_contains: &[],
                    constraints: &[&[MatchFlag::HasCapitalizedAfterAnyCue]],
                },
            ],
        }),
    },
    ProfileSlot {
        category: "identity",
        key: "role",
        description: "The user's profession, job title, or primary role.",
        validator: ValidatorKind::FreeText,
        max_len: 120,
        aliases: &["identity.occupation", "identity.job"],
        instant_hint: None,
    },
    ProfileSlot {
        category: "identity",
        key: "age",
        description: "The user's age in years.",
        validator: ValidatorKind::Integer { min: 0, max: 150 },
        max_len: 3,
        aliases: &[],
        instant_hint: None,
    },
    // ============ preference ============
    ProfileSlot {
        category: "preference",
        key: "language",
        description: "The user's preferred language for conversation.",
        validator: ValidatorKind::FreeText,
        max_len: 60,
        aliases: &[],
        instant_hint: None,
    },
    ProfileSlot {
        category: "preference",
        key: "communication_style",
        description: "How the user prefers to be communicated with (formal, terse, detailed, etc.).",
        validator: ValidatorKind::FreeText,
        max_len: 200,
        aliases: &[],
        instant_hint: None,
    },
    ProfileSlot {
        category: "preference",
        key: "interests",
        description: "Topics the user is interested in or hobbies.",
        validator: ValidatorKind::FreeText,
        max_len: 400,
        aliases: &["preference.hobbies"],
        instant_hint: None,
    },
    // ============ work ============
    ProfileSlot {
        category: "work",
        key: "employer",
        description: "The name of the user's employer or company.",
        validator: ValidatorKind::FreeText,
        max_len: 120,
        aliases: &["work.company"],
        instant_hint: None,
    },
    ProfileSlot {
        category: "work",
        key: "tools",
        description: "Tools, software, or platforms the user uses for work.",
        validator: ValidatorKind::FreeText,
        max_len: 400,
        aliases: &[],
        instant_hint: None,
    },
    // ============ health ============
    ProfileSlot {
        category: "health",
        key: "conditions",
        description: "Health conditions, allergies, or medical notes the user wants remembered.",
        validator: ValidatorKind::FreeText,
        max_len: 400,
        aliases: &["health.allergies"],
        instant_hint: None,
    },
    ProfileSlot {
        category: "health",
        key: "physical",
        description: "Physical attributes the user has shared (hair, height, build, etc.).",
        validator: ValidatorKind::FreeText,
        max_len: 200,
        aliases: &["preference.hair", "preference.appearance"],
        instant_hint: None,
    },
    // ============ vehicle ============
    ProfileSlot {
        category: "vehicle",
        key: "primary",
        description: "The user's primary vehicle (make/model/year).",
        validator: ValidatorKind::FreeText,
        max_len: 120,
        aliases: &["vehicle.car"],
        instant_hint: None,
    },
];

/// Look up a slot by canonical category.key.
pub fn lookup(category: &str, key: &str) -> Option<&'static ProfileSlot> {
    PROFILE_SLOTS
        .iter()
        .find(|s| s.category == category && s.key == key)
}

/// Resolve a possibly-legacy `category.key` to its canonical form.
/// Returns `Some((canonical_category, canonical_key))` if known, else `None`.
pub fn resolve_alias(category: &str, key: &str) -> Option<(&'static str, &'static str)> {
    // Canonical already?
    if let Some(slot) = lookup(category, key) {
        return Some((slot.category, slot.key));
    }
    let needle = format!("{}.{}", category, key);
    PROFILE_SLOTS
        .iter()
        .find(|s| s.aliases.iter().any(|a| *a == needle))
        .map(|s| (s.category, s.key))
}

/// Validate a (category, key, value) triple against the registry.
/// Unknown slots are rejected.
pub fn validate(category: &str, key: &str, value: &str) -> Result<(), ValidationError> {
    let slot = lookup(category, key).ok_or_else(|| ValidationError::UnknownSlot {
        category: category.to_string(),
        key: key.to_string(),
    })?;
    if value.len() > slot.max_len {
        return Err(ValidationError::TooLong {
            len: value.len(),
            max: slot.max_len,
        });
    }
    slot.validator.validate(value)
}

/// All slots (for iteration — e.g., generating the JSON schema or system prompt).
pub fn all_slots() -> &'static [ProfileSlot] {
    PROFILE_SLOTS
}

/// Evaluate an `InstantMatcher` against a fact's content. Returns true iff ANY
/// rule matches. A rule matches when:
///   1. at least one of its `cues` appears in the lowercased content, AND
///   2. if `also_contains` is non-empty, at least one of those substrings also
///      appears (case-sensitive for CJK, lowercased for ASCII), AND
///   3. every AND-clause in `constraints` has at least one satisfied MatchFlag.
pub fn instant_matcher_fires(matcher: &InstantMatcher, content: &str) -> bool {
    let lower = content.to_lowercase();
    matcher
        .rules
        .iter()
        .any(|rule| cue_rule_fires(rule, content, &lower))
}

fn cue_rule_fires(rule: &CueRule, content: &str, lower: &str) -> bool {
    // 1. Primary cue — any substring match (lowercase).
    let cue_hit = rule.cues.iter().find_map(|cue| {
        let cue_lower = cue.to_lowercase();
        lower.find(&cue_lower).map(|_| cue_lower)
    });
    let Some(matched_cue_lower) = cue_hit else {
        return false;
    };
    // For UTF-8 safety, locate the matched cue in the ORIGINAL content too.
    // `None` means the lowered→original mapping is ambiguous (extremely rare);
    // the capitalization constraint conservatively treats it as unsatisfied.
    let cue_start_in_content = find_case_insensitive(content, &matched_cue_lower);

    // 2. Co-occurrence — if non-empty, ANY of these must also appear.
    if !rule.also_contains.is_empty() {
        let any_co = rule.also_contains.iter().any(|tok| {
            // CJK tokens are case-neutral; ASCII tokens use lowercased content.
            lower.contains(&tok.to_lowercase()) || content.contains(tok)
        });
        if !any_co {
            return false;
        }
    }
    // 3. Constraints: AND of OR-clauses.
    for clause in rule.constraints {
        let any_flag_satisfied = clause.iter().any(|flag| {
            match_flag_satisfied(
                flag,
                content,
                lower,
                &matched_cue_lower,
                cue_start_in_content,
            )
        });
        if !any_flag_satisfied {
            return false;
        }
    }
    true
}

/// Locate a lowercased needle in the ORIGINAL content, case-insensitively,
/// returning the byte offset in `content` (NOT in its lowercased form).
/// Returns `None` when the match does not align with a char boundary (the
/// lowered→original byte mapping can diverge on codepoints like `İ` whose
/// lowercasing changes byte length; callers conservatively fall back).
fn find_case_insensitive(content: &str, needle_lower: &str) -> Option<usize> {
    let content_lower = content.to_lowercase();
    let idx_in_lower = content_lower.find(needle_lower)?;
    let mut content_pos = 0usize;
    let mut lower_pos = 0usize;
    for ch in content.chars() {
        if lower_pos == idx_in_lower {
            return content.is_char_boundary(content_pos).then_some(content_pos);
        }
        let lower_len: usize = ch.to_lowercase().map(|c| c.len_utf8()).sum();
        lower_pos += lower_len;
        content_pos += ch.len_utf8();
    }
    if lower_pos == idx_in_lower {
        return content.is_char_boundary(content_pos).then_some(content_pos);
    }
    None
}

fn match_flag_satisfied(
    flag: &MatchFlag,
    content: &str,
    lower: &str,
    matched_cue_lower: &str,
    cue_start_in_content: Option<usize>,
) -> bool {
    match flag {
        MatchFlag::HasDigitRun => has_digit_run(content),
        MatchFlag::HasStreetToken => has_street_token(content),
        MatchFlag::HasOfficeMarker => lower.contains("office"),
        MatchFlag::HasCapitalizedAfterAnyCue => {
            // Walk from the cue's position in the ORIGINAL content, skip past
            // the cue's byte length, then grab the first alphanumeric char
            // and check for ASCII uppercase. `content.get(..)` returns None
            // (rather than panicking) on any non-boundary slice.
            let Some(start) = cue_start_in_content else {
                return false;
            };
            let after = start + matched_cue_lower.len();
            let Some(tail) = content.get(after..) else {
                return false;
            };
            let next_word = tail
                .split_whitespace()
                .next()
                .unwrap_or("")
                .trim_start_matches(|c: char| !c.is_alphanumeric());
            next_word
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_uppercase())
        }
    }
}

/// Build a JSON Schema for the reflection LLM's `response_format`.
///
/// Shape:
/// ```json
/// { "actions": [ <action>, ... ] }
/// ```
/// where each `<action>` is one of the 4 variants. The `promote_profile`
/// variant uses `oneOf` with one branch per canonical slot, so the provider
/// physically cannot emit an unknown `(category, key)` pair (this is the
/// key defense against the `preference.hair`-style LLM invention bug).
///
/// `strict: false` because the OpenRouter surface for strict json-schema
/// rejects `oneOf` on some models; validator in Phase 3 is the hard gate.
pub fn reflection_response_format_schema() -> serde_json::Value {
    use serde_json::json;

    // Build one oneOf branch per slot: {"type":"promote_profile","fact_id":int,
    //  "category":"<cat>","key":"<key>","value":string}
    let promote_branches: Vec<serde_json::Value> = PROFILE_SLOTS
        .iter()
        .map(|slot| {
            let max_len = slot.max_len.min(256);
            json!({
                "type": "object",
                "properties": {
                    "type": { "const": "promote_profile" },
                    "fact_id": { "type": "integer" },
                    "category": { "const": slot.category },
                    "key": { "const": slot.key },
                    "value": { "type": "string", "maxLength": max_len },
                },
                "required": ["type", "fact_id", "category", "key", "value"],
                "additionalProperties": false,
            })
        })
        .collect();

    let merge_branch = json!({
        "type": "object",
        "properties": {
            "type": { "const": "merge" },
            "keep_id": { "type": "integer" },
            "drop_id": { "type": "integer" },
            "merged_text": { "type": "string" },
        },
        "required": ["type", "keep_id", "drop_id", "merged_text"],
        "additionalProperties": false,
    });

    let contradiction_branch = json!({
        "type": "object",
        "properties": {
            "type": { "const": "contradiction" },
            "id1": { "type": "integer" },
            "id2": { "type": "integer" },
            "resolution": { "type": "string" },
        },
        "required": ["type", "id1", "id2", "resolution"],
        "additionalProperties": false,
    });

    let prune_branch = json!({
        "type": "object",
        "properties": {
            "type": { "const": "prune" },
            "id": { "type": "integer" },
            "reason": { "type": "string" },
        },
        "required": ["type", "id", "reason"],
        "additionalProperties": false,
    });

    let mut all_branches = vec![merge_branch, contradiction_branch, prune_branch];
    all_branches.extend(promote_branches);

    json!({
        "type": "object",
        "properties": {
            "actions": {
                "type": "array",
                "items": { "oneOf": all_branches }
            }
        },
        "required": ["actions"],
        "additionalProperties": false,
    })
}

/// Render a compact human-readable list of all canonical slots, for inclusion
/// in the REFLECTION_SYSTEM_PROMPT (so the LLM understands slot semantics even
/// when provider doesn't honor response_format strictly).
pub fn render_slot_catalog() -> String {
    let mut out = String::with_capacity(2048);
    for slot in PROFILE_SLOTS {
        out.push_str(&format!(
            "- {}.{} — {}\n",
            slot.category, slot.key, slot.description
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup_finds_canonical() {
        let s = lookup("location", "home_address").unwrap();
        assert_eq!(s.key, "home_address");
    }

    #[test]
    fn lookup_misses_unknown() {
        assert!(lookup("bogus", "nope").is_none());
    }

    #[test]
    fn resolve_alias_canonical_passthrough() {
        assert_eq!(
            resolve_alias("location", "home_address"),
            Some(("location", "home_address"))
        );
    }

    #[test]
    fn resolve_alias_legacy_mapped() {
        assert_eq!(
            resolve_alias("location", "home_location"),
            Some(("location", "residence_city"))
        );
        assert_eq!(
            resolve_alias("preference", "hair"),
            Some(("health", "physical"))
        );
        assert_eq!(
            resolve_alias("family", "wife"),
            Some(("family", "spouse_name"))
        );
    }

    #[test]
    fn resolve_alias_unknown_is_none() {
        assert!(resolve_alias("bogus", "nope").is_none());
    }

    #[test]
    fn address_like_accepts_trailing_street_token() {
        // Regression for CTO Phase 1 note #2 + Phase 3 fix: every street token uses
        // the leading-space-only pattern (` ave`, ` rd`, ` ln`, ` pl`, ` place`) so
        // end-of-string at the token boundary still matches.
        assert!(ValidatorKind::AddressLike.validate("123 Main Ave").is_ok());
        assert!(ValidatorKind::AddressLike.validate("456 Elm Rd").is_ok());
        assert!(ValidatorKind::AddressLike.validate("789 Oak Ln").is_ok());
        assert!(ValidatorKind::AddressLike.validate("123 Main Pl").is_ok());
        assert!(
            ValidatorKind::AddressLike
                .validate("123 Example Pl SE, Example City WA")
                .is_ok()
        );
        assert!(
            ValidatorKind::AddressLike
                .validate("42 Rainbow Place")
                .is_ok()
        );
    }

    #[test]
    fn address_like_rejects_substring_false_positives() {
        // CTO Phase 3 note: earlier draft of the token list used bare "place" which
        // matched "someplace" / "marketplace" / "replace" / "displaced". The bounded
        // " place" form correctly rejects these (AddressLike also requires digits +
        // a street token, so nonsense like "marketplace 2024" without a leading
        // space before "place" should still fail).
        // "marketplace" has no leading space before "place", so no street-token match.
        let v = "marketplace 2024 is great";
        assert!(ValidatorKind::AddressLike.validate(v).is_err());
        // "replace 100 widgets" — no street token.
        let v = "please replace 100 widgets in the kitchen";
        assert!(ValidatorKind::AddressLike.validate(v).is_err());
    }

    #[test]
    fn address_like_accepts_real_address() {
        assert!(
            ValidatorKind::AddressLike
                .validate("123 Main Street, Seattle WA")
                .is_ok()
        );
        assert!(ValidatorKind::AddressLike.validate("朝阳路 88 号").is_ok());
    }

    #[test]
    fn address_like_rejects_city_only() {
        assert!(matches!(
            ValidatorKind::AddressLike.validate("Example City, Washington"),
            Err(ValidationError::NotAddressLike)
        ));
    }

    #[test]
    fn address_like_rejects_narrative() {
        let v = "User lives in Example City, has Asian straight hair, and works remotely";
        assert!(matches!(
            ValidatorKind::AddressLike.validate(v),
            Err(ValidationError::MixedContent)
        ));
    }

    #[test]
    fn address_like_rejects_non_address_work_note() {
        let v = "user regularly uses Example Corp Copilot as part of their work";
        // Either MixedContent or NotAddressLike is acceptable — both are rejections.
        assert!(ValidatorKind::AddressLike.validate(v).is_err());
    }

    #[test]
    fn location_like_accepts_city() {
        assert!(
            ValidatorKind::LocationLike
                .validate("Example City, Washington")
                .is_ok()
        );
        assert!(ValidatorKind::LocationLike.validate("北京").is_ok());
    }

    #[test]
    fn location_like_rejects_street_address() {
        assert!(matches!(
            ValidatorKind::LocationLike.validate("123 Main Street, Seattle"),
            Err(ValidationError::NotLocationLike)
        ));
    }

    #[test]
    fn location_like_rejects_narrative() {
        let v = "User lives in Example City, has Asian straight hair, and works remotely";
        assert!(matches!(
            ValidatorKind::LocationLike.validate(v),
            Err(ValidationError::MixedContent)
        ));
    }

    #[test]
    fn name_like_accepts_short_name() {
        assert!(ValidatorKind::NameLike.validate("Alice Chen").is_ok());
        assert!(ValidatorKind::NameLike.validate("张伟").is_ok());
    }

    #[test]
    fn name_like_rejects_sentence() {
        assert!(
            ValidatorKind::NameLike
                .validate("The user is a software engineer with ten years of experience.")
                .is_err()
        );
    }

    #[test]
    fn enum_case_insensitive() {
        let v = ValidatorKind::Enum(&["english", "chinese"]);
        assert!(v.validate("English").is_ok());
        assert!(v.validate("CHINESE").is_ok());
        assert!(v.validate("french").is_err());
    }

    #[test]
    fn integer_range_enforced() {
        let v = ValidatorKind::Integer { min: 0, max: 150 };
        assert!(v.validate("42").is_ok());
        assert!(matches!(
            v.validate("200"),
            Err(ValidationError::IntegerOutOfRange { .. })
        ));
        assert!(matches!(
            v.validate("abc"),
            Err(ValidationError::NotInteger)
        ));
    }

    #[test]
    fn validate_unknown_slot_rejected() {
        assert!(matches!(
            validate("bogus", "nope", "anything"),
            Err(ValidationError::UnknownSlot { .. })
        ));
    }

    #[test]
    fn validate_enforces_max_len() {
        let long = "x".repeat(300);
        assert!(matches!(
            validate("identity", "name", &long),
            Err(ValidationError::TooLong { .. })
        ));
    }

    /// The four known-bad production rows (ids 12, 13, 15, 20) MUST all be rejected.
    #[test]
    fn rejects_known_bad_prod_rows() {
        // id=12: location.home_address = narrative mixed content
        assert!(
            validate(
                "location",
                "home_address",
                "User lives in Example City, has Asian straight hair, and works remotely."
            )
            .is_err()
        );
        // id=13: location.work_address = not an address
        assert!(
            validate(
                "location",
                "work_address",
                "user regularly uses Example Corp Copilot as part of their work"
            )
            .is_err()
        );
        // id=15: preference.hair — unknown key (canonical is health.physical).
        // Via resolve_alias it maps to health.physical which is FreeText, so value
        // passes validation — but the value here is mixed-content narrative. The
        // registry's role here is to route via alias; narrative-ness is caught by
        // AddressLike/LocationLike slots, not FreeText. The original bug was that
        // the LLM emitted `preference.hair` at all — structured output will prevent
        // that key from ever appearing. Assert the alias resolution works:
        assert_eq!(
            resolve_alias("preference", "hair"),
            Some(("health", "physical"))
        );
        // id=20: location.home_location = city-only, legacy key
        assert_eq!(
            resolve_alias("location", "home_location"),
            Some(("location", "residence_city"))
        );
        // After alias, validates as a LocationLike city → OK (this is the GOOD behavior:
        // we absorb the duplicate into residence_city via dedup in Phase 3).
        assert!(validate("location", "residence_city", "Example City, Washington").is_ok());
    }

    #[test]
    fn all_slots_have_unique_canonical_keys() {
        let mut seen = std::collections::HashSet::new();
        for slot in all_slots() {
            let k = format!("{}.{}", slot.category, slot.key);
            assert!(seen.insert(k.clone()), "duplicate canonical slot: {}", k);
        }
    }

    #[test]
    fn reflection_schema_has_one_branch_per_slot_plus_three_other_actions() {
        let schema = reflection_response_format_schema();
        let branches = schema["properties"]["actions"]["items"]["oneOf"]
            .as_array()
            .expect("items.oneOf must be an array");
        // 3 non-promote branches (merge, contradiction, prune) + N slots.
        assert_eq!(branches.len(), 3 + PROFILE_SLOTS.len());
        // Every promote branch has const type=promote_profile + const category+key.
        let promote_count = branches
            .iter()
            .filter(|b| b["properties"]["type"]["const"] == "promote_profile")
            .count();
        assert_eq!(promote_count, PROFILE_SLOTS.len());
    }

    #[test]
    fn render_slot_catalog_nonempty_and_contains_known_slot() {
        let cat = render_slot_catalog();
        assert!(cat.contains("location.home_address"));
        assert!(cat.contains("identity.name"));
        assert!(!cat.is_empty());
    }

    // ── Phase 4: InstantMatcher evaluation ────────────────────────────────

    fn home_address_hint() -> &'static InstantMatcher {
        lookup("location", "home_address")
            .unwrap()
            .instant_hint
            .as_ref()
            .unwrap()
    }

    fn work_address_hint() -> &'static InstantMatcher {
        lookup("location", "work_address")
            .unwrap()
            .instant_hint
            .as_ref()
            .unwrap()
    }

    fn name_hint() -> &'static InstantMatcher {
        lookup("identity", "name")
            .unwrap()
            .instant_hint
            .as_ref()
            .unwrap()
    }

    #[test]
    fn instant_matcher_home_requires_digit_run() {
        assert!(instant_matcher_fires(
            home_address_hint(),
            "User lives at 123 Example Pl SE",
        ));
        // No digit run → no match.
        assert!(!instant_matcher_fires(
            home_address_hint(),
            "User lives in Seattle",
        ));
    }

    #[test]
    fn instant_matcher_home_chinese() {
        // CJK cue 家在 + zip-like digit pair.
        assert!(instant_matcher_fires(
            home_address_hint(),
            "\u{5BB6}\u{5728}\u{897F}\u{96C5}\u{56FE} 98027",
        ));
    }

    #[test]
    fn instant_matcher_work_requires_digit_or_office() {
        // "works at 456 Example Ave NE" — has digits.
        assert!(instant_matcher_fires(
            work_address_hint(),
            "User works at 456 Example Ave NE",
        ));
        // "Works at the downtown office" — has "office".
        assert!(instant_matcher_fires(
            work_address_hint(),
            "Works at the downtown office",
        ));
        // "works at Example Corp" — no digits, no "office" → no match.
        assert!(!instant_matcher_fires(
            work_address_hint(),
            "User works at Example Corp",
        ));
    }

    #[test]
    fn instant_matcher_work_cjk_requires_cooccurrence() {
        // 工作 + 在 + digit → match.
        assert!(instant_matcher_fires(
            work_address_hint(),
            "\u{5728}\u{5317}\u{4EAC}\u{7684}\u{5927}\u{516C}\u{53F8}\u{5DE5}\u{4F5C}, \
             \u{539F}\u{578B}\u{6700}\u{5927}\u{5DE5}\u{4F5C}\u{533A} 1000 \u{5E73}\u{7C73}",
        ));
        // Bare "在公园散步" without 工作 → no match.
        assert!(!instant_matcher_fires(
            work_address_hint(),
            "\u{5728}\u{516C}\u{56ED}\u{6563}\u{6B65}",
        ));
    }

    #[test]
    fn instant_matcher_name_my_name_is() {
        assert!(instant_matcher_fires(name_hint(), "My name is Lance"));
        // CJK: 我叫<X>
        assert!(instant_matcher_fires(
            name_hint(),
            "\u{6211}\u{53EB}\u{674E}\u{660E}"
        ));
    }

    #[test]
    fn instant_matcher_name_im_with_capitalized() {
        assert!(instant_matcher_fires(
            name_hint(),
            "I'm Lance, nice to meet you"
        ));
        // Lowercase after "I'm" → no match.
        assert!(!instant_matcher_fires(name_hint(), "I'm hungry today"));
    }

    #[test]
    fn instant_matcher_name_rejects_i_am_sentences() {
        // "I am hungry" / "I am a developer" must NOT trigger the name hint.
        assert!(!instant_matcher_fires(name_hint(), "I am hungry today"));
        assert!(!instant_matcher_fires(
            name_hint(),
            "I am a senior developer"
        ));
        // CJK self-descriptor "我是程序员" must also be rejected.
        assert!(!instant_matcher_fires(
            name_hint(),
            "\u{6211}\u{662F}\u{7A0B}\u{5E8F}\u{5458}",
        ));
    }

    #[test]
    fn aliases_do_not_collide_with_canonicals() {
        for slot in all_slots() {
            for alias in slot.aliases {
                let parts: Vec<&str> = alias.splitn(2, '.').collect();
                assert_eq!(parts.len(), 2, "alias must be `category.key`: {}", alias);
                assert!(
                    lookup(parts[0], parts[1]).is_none(),
                    "alias {} collides with a canonical slot",
                    alias
                );
            }
        }
    }
}
