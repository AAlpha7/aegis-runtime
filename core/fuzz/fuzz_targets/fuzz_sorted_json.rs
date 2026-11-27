//! Fuzz the sorted_json deterministic serialization function.
//!
//! Goals:
//! - Ensure sorted_json never panics on any serde_json::Value
//! - Verify that output is always deterministic (same input → same output)
//! - Stress deeply nested structures, large arrays, unicode keys
#![no_main]

use libfuzzer_sys::fuzz_target;

use aegis_runtime_core::identity::sorted_json;

fuzz_target!(|data: &[u8]| {
    // Try to parse arbitrary bytes as JSON
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(data) else {
        return;
    };

    // sorted_json must not panic
    let s1 = sorted_json(&value);
    let s2 = sorted_json(&value);

    // Must be deterministic
    assert_eq!(s1, s2, "sorted_json must produce identical output for identical input");

    // If the output is valid JSON, re-parsing and re-sorting must be idempotent
    if let Ok(reparsed) = serde_json::from_str::<serde_json::Value>(&s1) {
        let s3 = sorted_json(&reparsed);
        assert_eq!(s1, s3, "sorted_json must be idempotent");
    }
});
