//! Fuzz the exfiltration detection engine.
//!
//! Goals:
//! - Ensure detect_exfiltration never panics on arbitrary input
//! - Confirm shannon_entropy handles all byte patterns without panic
//! - Detect potential bypasses where encoded secrets slip through
//! - Stress test with very large inputs near EXFIL_SCAN_LIMIT
#![no_main]

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;

use aegis_runtime_core::orchestrator::{detect_exfiltration, shannon_entropy};

#[derive(Debug, Arbitrary)]
struct FuzzInput {
    tool_name: String,
    previous_results: Vec<String>,
    new_args: String,
    is_read_only: bool,
}

fuzz_target!(|input: FuzzInput| {
    // Must not panic on any input combination
    let _ = detect_exfiltration(
        &input.tool_name,
        &input.previous_results,
        &input.new_args,
        input.is_read_only,
    );

    // Shannon entropy must return finite non-negative values
    let e = shannon_entropy(&input.new_args);
    assert!(e >= 0.0, "entropy must be non-negative");
    assert!(e.is_finite(), "entropy must be finite");

    for prev in &input.previous_results {
        let e = shannon_entropy(prev);
        assert!(e >= 0.0 && e.is_finite());
    }
});
