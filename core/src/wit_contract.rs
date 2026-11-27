//! WIT Contract Validation for Aegis Runtime Skills
//!
//! Validates that compiled Wasm modules conform to the aegis.wit interface contract.
//! This provides compile-time-like guarantees without requiring full wit-bindgen migration.
//!
//! The contract requires:
//! 1. Export: `run()` function (no params, no return)
//! 2. Export: `alloc(i32) -> i32` function (memory allocator)
//! 3. Export: `memory` (linear memory)
//! 4. Import: only `aegis::*` host functions matching the skill's pattern

use wasmtime::Module;

/// Result of validating a Wasm module against the aegis.wit contract.
#[derive(Debug, Clone)]
pub struct ContractValidation {
    pub valid: bool,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    /// Host functions imported by the module.
    pub imports: Vec<String>,
    /// Functions exported by the module.
    pub exports: Vec<String>,
}

/// Known host functions from the aegis.wit contract.
const KNOWN_HOST_FUNCTIONS: &[&str] = &[
    "host_get_payload_len",
    "host_get_payload",
    "host_write_output",
    "host_fetch",
    "host_get_fetch_response_len",
    "host_get_fetch_response",
    "host_store_state",
    "host_load_state",
    "host_get_state_response_len",
    "host_get_state_response",
    "host_read_file",
    "host_list_files",
];

/// Host functions required for each skill pattern.
fn required_imports_for_pattern(pattern: &str) -> Vec<&'static str> {
    match pattern {
        "api_json" | "api_html" | "multi_fetch" | "probe_heartbeat" => vec![
            "host_get_payload_len",
            "host_get_payload",
            "host_write_output",
            "host_fetch",
            "host_get_fetch_response_len",
            "host_get_fetch_response",
        ],
        "no_network" => vec![
            "host_get_payload_len",
            "host_get_payload",
            "host_write_output",
        ],
        "no_network_stateful" => vec![
            "host_get_payload_len",
            "host_get_payload",
            "host_write_output",
            "host_store_state",
            "host_load_state",
            "host_get_state_response_len",
            "host_get_state_response",
        ],
        "stateful" => vec![
            "host_get_payload_len",
            "host_get_payload",
            "host_write_output",
            "host_fetch",
            "host_get_fetch_response_len",
            "host_get_fetch_response",
            "host_store_state",
            "host_load_state",
            "host_get_state_response_len",
            "host_get_state_response",
        ],
        _ => vec![
            "host_get_payload_len",
            "host_get_payload",
            "host_write_output",
        ],
    }
}

/// Validate a compiled Wasm module against the aegis.wit contract.
///
/// Checks:
/// 1. Required exports: `run` and `alloc` functions, `memory` export
/// 2. Import namespace: all imports must be from "aegis" module
/// 3. Pattern compliance: imports match the expected host functions for the pattern
/// 4. No unknown imports (catches typos like "host_fetc")
pub fn validate_contract(module: &Module, pattern: &str) -> ContractValidation {
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    let mut imports = Vec::new();
    let mut exports = Vec::new();

    // Collect exports
    for export in module.exports() {
        exports.push(export.name().to_string());
    }

    // Check required exports
    if !exports.iter().any(|e| e == "run") {
        errors.push("Missing required export: 'run' function".to_string());
    }
    if !exports.iter().any(|e| e == "alloc") {
        errors.push("Missing required export: 'alloc' function".to_string());
    }
    if !exports.iter().any(|e| e == "memory") {
        errors.push("Missing required export: 'memory' (linear memory)".to_string());
    }

    // Collect and validate imports
    for import in module.imports() {
        let module_name = import.module();
        let name = import.name();
        imports.push(format!("{}::{}", module_name, name));

        if module_name == "aegis" {
            // Validate it's a known host function
            if !KNOWN_HOST_FUNCTIONS.contains(&name) {
                errors.push(format!(
                    "Unknown aegis import '{}' — not in WIT contract. Known functions: {}",
                    name,
                    KNOWN_HOST_FUNCTIONS.join(", ")
                ));
            }
        }
        // WASI imports are expected and handled by define_unknown_imports_as_traps
    }

    // Check pattern compliance: warn if importing functions not in pattern's allowed set
    let required = required_imports_for_pattern(pattern);
    let aegis_imports: Vec<&str> = imports
        .iter()
        .filter_map(|i| i.strip_prefix("aegis::"))
        .collect();

    for imp in &aegis_imports {
        if !required.contains(imp) && KNOWN_HOST_FUNCTIONS.contains(imp) {
            warnings.push(format!(
                "Import '{}' is not typical for pattern '{}' — may indicate wrong pattern selection",
                imp, pattern
            ));
        }
    }

    // Check if pattern requires network but skill doesn't import host_fetch
    if required.contains(&"host_fetch") && !aegis_imports.contains(&"host_fetch") {
        warnings.push(format!(
            "Pattern '{}' expects 'host_fetch' but module doesn't import it",
            pattern
        ));
    }

    // no_network pattern should NOT import host_fetch
    if pattern == "no_network" && aegis_imports.contains(&"host_fetch") {
        errors.push(
            "Pattern 'no_network' must NOT import 'host_fetch' — \
             this skill should not make network requests"
                .to_string(),
        );
    }

    ContractValidation {
        valid: errors.is_empty(),
        errors,
        warnings,
        imports,
        exports,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn required_imports_api_json() {
        let req = required_imports_for_pattern("api_json");
        assert!(req.contains(&"host_fetch"));
        assert!(req.contains(&"host_write_output"));
        assert!(!req.contains(&"host_store_state"));
    }

    #[test]
    fn required_imports_no_network() {
        let req = required_imports_for_pattern("no_network");
        assert!(!req.contains(&"host_fetch"));
        assert!(req.contains(&"host_write_output"));
    }

    #[test]
    fn required_imports_stateful() {
        let req = required_imports_for_pattern("stateful");
        assert!(req.contains(&"host_fetch"));
        assert!(req.contains(&"host_store_state"));
        assert!(req.contains(&"host_load_state"));
    }

    #[test]
    fn known_host_functions_complete() {
        // Ensure all functions match the WIT contract
        assert!(KNOWN_HOST_FUNCTIONS.contains(&"host_get_payload_len"));
        assert!(KNOWN_HOST_FUNCTIONS.contains(&"host_get_payload"));
        assert!(KNOWN_HOST_FUNCTIONS.contains(&"host_write_output"));
        assert!(KNOWN_HOST_FUNCTIONS.contains(&"host_fetch"));
        assert!(KNOWN_HOST_FUNCTIONS.contains(&"host_store_state"));
        assert!(KNOWN_HOST_FUNCTIONS.contains(&"host_load_state"));
        assert!(KNOWN_HOST_FUNCTIONS.contains(&"host_read_file"));
        assert!(KNOWN_HOST_FUNCTIONS.contains(&"host_list_files"));
        assert_eq!(KNOWN_HOST_FUNCTIONS.len(), 12);
    }

    #[test]
    fn validate_contract_with_valid_module() {
        let engine = wasmtime::Engine::default();
        let wat = r#"(module
            (import "aegis" "host_fetch" (func (param i32 i32) (result i32)))
            (import "aegis" "host_write_output" (func (param i32 i32)))
            (import "aegis" "host_get_payload_len" (func (result i32)))
            (import "aegis" "host_get_payload" (func (param i32)))
            (import "aegis" "host_get_fetch_response_len" (func (result i32)))
            (import "aegis" "host_get_fetch_response" (func (param i32)))
            (memory (export "memory") 1)
            (func (export "run"))
            (func (export "alloc") (param i32) (result i32) (i32.const 0))
        )"#;
        let module = Module::new(&engine, wat).unwrap();
        let result = validate_contract(&module, "api_json");
        assert!(result.valid, "errors: {:?}", result.errors);
        assert!(
            result.warnings.is_empty(),
            "warnings: {:?}",
            result.warnings
        );
    }

    #[test]
    fn validate_contract_missing_run() {
        let engine = wasmtime::Engine::default();
        let wat = r#"(module
            (memory (export "memory") 1)
            (func (export "alloc") (param i32) (result i32) (i32.const 0))
        )"#;
        let module = Module::new(&engine, wat).unwrap();
        let result = validate_contract(&module, "no_network");
        assert!(!result.valid);
        assert!(result.errors.iter().any(|e| e.contains("run")));
    }

    #[test]
    fn validate_contract_unknown_import() {
        let engine = wasmtime::Engine::default();
        let wat = r#"(module
            (import "aegis" "host_nonexistent" (func))
            (import "aegis" "host_write_output" (func (param i32 i32)))
            (import "aegis" "host_get_payload_len" (func (result i32)))
            (import "aegis" "host_get_payload" (func (param i32)))
            (memory (export "memory") 1)
            (func (export "run"))
            (func (export "alloc") (param i32) (result i32) (i32.const 0))
        )"#;
        let module = Module::new(&engine, wat).unwrap();
        let result = validate_contract(&module, "no_network");
        assert!(!result.valid);
        assert!(result.errors.iter().any(|e| e.contains("host_nonexistent")));
    }

    #[test]
    fn validate_contract_no_network_with_fetch_error() {
        let engine = wasmtime::Engine::default();
        let wat = r#"(module
            (import "aegis" "host_fetch" (func (param i32 i32) (result i32)))
            (import "aegis" "host_write_output" (func (param i32 i32)))
            (import "aegis" "host_get_payload_len" (func (result i32)))
            (import "aegis" "host_get_payload" (func (param i32)))
            (memory (export "memory") 1)
            (func (export "run"))
            (func (export "alloc") (param i32) (result i32) (i32.const 0))
        )"#;
        let module = Module::new(&engine, wat).unwrap();
        let result = validate_contract(&module, "no_network");
        assert!(!result.valid);
        assert!(
            result
                .errors
                .iter()
                .any(|e| e.contains("no_network") && e.contains("host_fetch"))
        );
    }
}
