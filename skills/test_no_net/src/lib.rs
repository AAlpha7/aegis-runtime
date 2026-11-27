#![allow(unsafe_code)] // #[no_mangle] on WASM entry point
use aegis_runtime_wasm_core as core;

#[derive(serde::Deserialize)]
struct Payload {
    text: String,
    #[serde(default)]
    priority: Option<String>,
}

#[derive(serde::Serialize)]
struct Output {
    status: &'static str,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error_kind: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    retryable: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    hint: Option<&'static str>,
}

#[unsafe(no_mangle)]
pub extern "C" fn run() {
    let payload: Payload = match core::read_payload() {
        Ok(p) => p,
        Err(_) => {
            core::write_json(&Output {
                status: "error",
                message: String::new(),
                error: Some("Invalid payload".into()),
                error_kind: Some("validation"),
                retryable: Some(false),
                hint: Some("Provide a JSON object with a \"text\" field"),
            });
            return;
        }
    };

    if payload.text.trim().is_empty() {
        core::write_json(&Output {
            status: "error",
            message: String::new(),
            error: Some("text is required".into()),
            error_kind: Some("validation"),
            retryable: Some(false),
            hint: Some("Provide a non-empty \"text\" field"),
        });
        return;
    }

    let priority = payload.priority.as_deref().unwrap_or("normal");
    core::write_json(&Output {
        status: "ok",
        message: format!("Reminder set (priority: {}): {}", priority, payload.text),
        error: None,
        error_kind: None,
        retryable: None,
        hint: None,
    });
}
