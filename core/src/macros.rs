//! Shared macros for the Aegis Runtime core crate.
//!
//! This module is declared with `#[macro_use]` in `lib.rs` so all macros
//! defined here are available to subsequently-declared modules.

/// Dispatch a tool call through the standard pipeline:
/// push tool name → parse JSON args → call handler method → push result → return.
///
/// Used by all `orchestrator_tools_*.rs` dispatcher functions. Tools that need
/// non-standard return values (e.g. `Ok(Some(false))`) are handled manually
/// outside this macro.
macro_rules! tool_dispatch {
    ($self:expr, $tool_call:expr, $tool_name:expr, $tool_args:expr, $ctx:expr,
     $($name:literal => $method:ident),+ $(,)?) => {
        match $tool_name {
            $($name => {
                $ctx.tools_called.push($name.to_string());
                let result = match serde_json::from_str::<serde_json::Value>($tool_args) {
                    Ok(args) => $self.$method(args, $ctx).await,
                    Err(e) => crate::utils::json_parse_tool_error($name, &e),
                };
                crate::orchestrator::push_tool_result($ctx.messages, &$tool_call.id, &result)?;
                // SECURITY: the exfiltration guard (detect_exfiltration in the WASM and
                // MCP dispatch paths) scans `previous_tool_results`. Macro-dispatched
                // tools (web_search, recall_memory, google, …) surface real data, so their
                // results MUST land here too — otherwise a later network tool could
                // exfiltrate that data unseen. Also fixes source_evidence pairing.
                $ctx.previous_tool_results.push(result);
                return Ok(Some(true));
            })+
            _ => {}
        }
    }
}
