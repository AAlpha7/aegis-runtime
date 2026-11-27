use super::*;

/// Extract tool name from a ChatCompletionTools.
fn tool_name(tool: &ChatCompletionTools) -> &str {
    match tool {
        ChatCompletionTools::Function(t) => &t.function.name,
        _ => panic!("Expected Function tool"),
    }
}

/// Extract "required" fields from a ChatCompletionTools.
fn required_fields(tool: &ChatCompletionTools) -> Vec<String> {
    match tool {
        ChatCompletionTools::Function(t) => t
            .function
            .parameters
            .as_ref()
            .and_then(|p| p.get("required"))
            .and_then(|r| r.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default(),
        _ => panic!("Expected Function tool"),
    }
}

/// Build a minimal SkillCapability for testing.
fn test_capability(name: &str, desc: &str, schema: serde_json::Value) -> SkillCapability {
    serde_json::from_value(serde_json::json!({
        "skill_name": name,
        "max_memory_mb": 2,
        "allowed_domains": [],
        "allowed_fs_paths": [],
        "description": desc,
        "payload_schema": schema,
    }))
    .unwrap()
}

fn load_repo_skill_manifest(skill_name: &str) -> SkillCapability {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("core has workspace parent")
        .join("skills")
        .join(skill_name)
        .join("manifest.json");
    let json = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!("read {}: {e}", path.display());
    });
    serde_json::from_str(&json).unwrap_or_else(|e| {
        panic!("parse {}: {e}", path.display());
    })
}

// ── All builders produce valid tools with correct names ─────────────

macro_rules! test_builder {
    ($test_name:ident, $builder:ident, $expected_name:literal) => {
        #[test]
        fn $test_name() {
            let tool = $builder().expect(concat!("build ", $expected_name));
            assert_eq!(tool_name(&tool), $expected_name);
        }
    };
}

test_builder!(tts, build_text_to_speech_tool, "text_to_speech");
test_builder!(image, build_generate_image_tool, "generate_image");
test_builder!(web_search, build_web_search_tool, "web_search");
test_builder!(x_search, build_x_search_tool, "x_search");
test_builder!(update_setting, build_update_setting_tool, "update_setting");
test_builder!(switch_model, build_switch_model_tool, "switch_model");
test_builder!(
    trigger_corpus_reembed,
    build_trigger_corpus_reembed_tool,
    "trigger_corpus_reembed"
);
test_builder!(notify_user, build_notify_user_tool, "notify_user");
test_builder!(get_time, build_get_current_time_tool, "get_current_time");
test_builder!(
    generate_personal_brief,
    build_generate_personal_brief_tool,
    "generate_personal_brief"
);
test_builder!(recall_memory, build_recall_memory_tool, "recall_memory");
test_builder!(view_logs, build_view_logs_tool, "view_logs");
test_builder!(set_log_level, build_set_log_level_tool, "set_log_level");
test_builder!(
    set_bot_commands,
    build_set_bot_commands_tool,
    "set_bot_commands"
);
test_builder!(
    deactivate_skill,
    build_deactivate_skill_tool,
    "deactivate_skill"
);
test_builder!(
    reactivate_skill,
    build_reactivate_skill_tool,
    "reactivate_skill"
);
test_builder!(manage_skill, build_manage_skill_tool, "manage_skill");
test_builder!(manage_tasks, build_manage_tasks_tool, "manage_tasks");
test_builder!(manage_goals, build_manage_goals_tool, "manage_goals");
test_builder!(delegate_task, build_delegate_task_tool, "delegate_task");
test_builder!(run_external, build_run_external_tool, "run_external");
test_builder!(browser_fetch, build_browser_fetch_tool, "browser_fetch");
test_builder!(
    secure_computer_use,
    build_secure_computer_use_tool,
    "secure_computer_use"
);
test_builder!(knowledge_base, build_knowledge_base_tool, "knowledge_base");
test_builder!(
    check_api_usage,
    build_check_api_usage_tool,
    "check_api_usage"
);
test_builder!(
    google_drive_upload,
    build_google_drive_upload_tool,
    "google_drive_upload"
);
test_builder!(setup_google, build_setup_google_tool, "setup_google");

#[test]
fn setup_google_description_does_not_claim_partial_consent_screen() {
    let tool = build_setup_google_tool().expect("build setup_google");
    let desc = match tool {
        ChatCompletionTools::Function(t) => t.function.description.unwrap_or_default(),
        _ => panic!("Expected Function tool"),
    };
    let forbidden_phrases = &[
        "only ask for the NEW scopes",
        "only ask consent for the NEW scopes",
        "only prompt for the NEW scopes",
        "only asks for the NEW scopes",
    ];
    for phrase in forbidden_phrases {
        assert!(
            !desc.contains(phrase),
            "setup_google description must not claim partial consent-screen behavior (found forbidden phrase: {phrase:?}). Full description: {desc}"
        );
    }
    assert!(
        desc.contains("full consent screen") || desc.contains("UNION"),
        "description must correctly describe the token-level union semantics: {desc}"
    );
    assert!(
        desc.contains("include_granted_scopes=true"),
        "description must cite include_granted_scopes=true as the actual mechanism: {desc}"
    );
}
test_builder!(
    calendar_create_event,
    build_calendar_create_event_tool,
    "calendar_create_event"
);
test_builder!(
    calendar_update_event,
    build_calendar_update_event_tool,
    "calendar_update_event"
);
test_builder!(
    calendar_delete_event,
    build_calendar_delete_event_tool,
    "calendar_delete_event"
);
test_builder!(
    schedule_intent,
    build_schedule_intent_tool,
    "schedule_intent"
);
test_builder!(list_intents, build_list_intents_tool, "list_intents");
test_builder!(cancel_intent, build_cancel_intent_tool, "cancel_intent");
test_builder!(
    set_monitoring,
    build_set_monitoring_intent_tool,
    "set_monitoring_intent"
);
test_builder!(
    list_monitoring,
    build_list_monitoring_intents_tool,
    "list_monitoring_intents"
);
test_builder!(
    remove_monitoring,
    build_remove_monitoring_intent_tool,
    "remove_monitoring_intent"
);
test_builder!(
    set_alert_policy,
    build_set_alert_policy_tool,
    "set_alert_policy"
);
test_builder!(
    list_alert_policies,
    build_list_alert_policies_tool,
    "list_alert_policies"
);
test_builder!(mcp_list, build_mcp_list_servers_tool, "mcp_list_servers");
test_builder!(
    mcp_reconnect,
    build_mcp_reconnect_server_tool,
    "mcp_reconnect_server"
);
test_builder!(
    configure_autonomous,
    build_configure_autonomous_tool,
    "configure_autonomous"
);
test_builder!(
    heartbeat_memo,
    build_update_heartbeat_memo_tool,
    "update_heartbeat_memo"
);
test_builder!(
    heartbeat_audit,
    build_view_heartbeat_audit_tool,
    "view_heartbeat_audit"
);
test_builder!(view_growth, build_view_growth_tool, "view_growth");
test_builder!(
    adjust_personality,
    build_adjust_personality_tool,
    "adjust_personality"
);

// ── Required field validation ──────────────────────────────────────

#[test]
fn tts_requires_text() {
    let tool = build_text_to_speech_tool().unwrap();
    assert!(required_fields(&tool).contains(&"text".to_string()));
}

#[test]
fn web_search_requires_search_prompt() {
    let tool = build_web_search_tool().unwrap();
    assert!(required_fields(&tool).contains(&"search_prompt".to_string()));
}

#[test]
fn x_search_requires_query() {
    let tool = build_x_search_tool().unwrap();
    assert_eq!(tool_name(&tool), "x_search");
    assert!(required_fields(&tool).contains(&"query".to_string()));
}

#[test]
fn notify_user_requires_message() {
    let tool = build_notify_user_tool().unwrap();
    assert!(required_fields(&tool).contains(&"message".to_string()));
}

#[test]
fn manage_skill_requires_action_and_skill_name() {
    let tool = build_manage_skill_tool().unwrap();
    let req = required_fields(&tool);
    assert!(req.contains(&"action".to_string()));
    assert!(req.contains(&"skill_name".to_string()));
}

#[test]
fn schedule_intent_requires_fields() {
    let tool = build_schedule_intent_tool().unwrap();
    let req = required_fields(&tool);
    assert!(req.contains(&"trigger_type".to_string()));
    assert!(req.contains(&"context_text".to_string()));
}

#[test]
fn schedule_intent_exposes_windowed_recurrence_fields() {
    // Codex F9: schedule_intent must ADVERTISE the windowed-recurrence params
    // (parity with manage_tasks), so a caller can create windowed recurrence
    // through the declared tool contract — not only when raw JSON happens to
    // include them.
    let tool = build_schedule_intent_tool().unwrap();
    let props = match &tool {
        ChatCompletionTools::Function(t) => t
            .function
            .parameters
            .as_ref()
            .and_then(|p| p.get("properties"))
            .and_then(|p| p.as_object())
            .expect("schedule_intent has properties"),
        _ => panic!("Expected Function tool"),
    };
    for field in [
        "recur_weekdays",
        "recur_window_start",
        "recur_window_end",
        "recur_timezone",
    ] {
        assert!(
            props.contains_key(field),
            "schedule_intent schema missing windowed field {field}"
        );
    }
}

#[test]
fn adjust_personality_requires_fields() {
    let tool = build_adjust_personality_tool().unwrap();
    let req = required_fields(&tool);
    assert!(req.contains(&"trait_name".to_string()));
    assert!(req.contains(&"value".to_string()));
    assert!(req.contains(&"reason".to_string()));
}

#[test]
fn view_growth_has_no_required_fields() {
    let tool = build_view_growth_tool().unwrap();
    let req = required_fields(&tool);
    assert!(req.is_empty());
}

// ── build_tool_from_capability ─────────────────────────────────────

#[test]
fn capability_tool_normalizes_empty_schema() {
    let cap = test_capability("test_skill", "desc", serde_json::json!({}));
    let tool = build_tool_from_capability(&cap).unwrap();
    if let ChatCompletionTools::Function(t) = tool {
        let params = t.function.parameters.as_ref().unwrap();
        assert_eq!(params.get("type").and_then(|v| v.as_str()), Some("object"));
        assert!(
            params
                .get("properties")
                .and_then(|v| v.as_object())
                .is_some_and(|m| m.is_empty())
        );
    } else {
        panic!("Expected Function tool");
    }
}

#[test]
fn capability_tool_basic() {
    let cap = test_capability(
        "my_skill",
        "Does something useful",
        serde_json::json!({
            "type": "object",
            "properties": { "query": { "type": "string" } },
            "required": ["query"]
        }),
    );
    let tool = build_tool_from_capability(&cap).unwrap();
    assert_eq!(tool_name(&tool), "my_skill");
    assert!(required_fields(&tool).contains(&"query".to_string()));
}

#[test]
fn capability_tool_with_limitations() {
    let mut cap = test_capability(
        "limited_skill",
        "Base desc",
        serde_json::json!({
            "type": "object", "properties": {}
        }),
    );
    cap.limitations = vec!["Cannot do X".into(), "Max 100 items".into()];
    let tool = build_tool_from_capability(&cap).unwrap();
    match &tool {
        ChatCompletionTools::Function(t) => {
            let desc = t.function.description.as_deref().unwrap_or("");
            assert!(desc.contains("Cannot do X"));
            assert!(desc.contains("LIMITATIONS"));
        }
        _ => panic!("Expected Function tool"),
    }
}

#[test]
fn capability_tool_multi_account_injects_param() {
    let mut cap = test_capability(
        "multi_acc",
        "Reads email",
        serde_json::json!({
            "type": "object",
            "properties": { "folder": { "type": "string" } }
        }),
    );
    cap.supports_multi_account = true;
    let tool = build_tool_from_capability(&cap).unwrap();
    match &tool {
        ChatCompletionTools::Function(t) => {
            let params = t.function.parameters.as_ref().unwrap();
            let props = params.get("properties").unwrap().as_object().unwrap();
            assert!(
                props.contains_key("account"),
                "multi_account skill should have 'account' param"
            );
        }
        _ => panic!("Expected Function tool"),
    }
}

// ── Pseudo-shadow names ────────────────────────────────────────────

#[test]
fn shadow_names_includes_expected() {
    assert!(PSEUDO_SHADOW_NAMES.contains(&"web_search"));
    assert!(PSEUDO_SHADOW_NAMES.contains(&"x_search"));
    assert!(PSEUDO_SHADOW_NAMES.contains(&"personal_tasks"));
}

// ── Activate agent skill ───────────────────────────────────────────

#[test]
fn activate_agent_skill_with_skills() {
    let skills = vec![
        crate::agent_skills::AgentSkill {
            name: "skill_a".into(),
            description: "Skill A".into(),
            body: String::new(),
            skill_dir: std::path::PathBuf::new(),
            compatibility: None,
        },
        crate::agent_skills::AgentSkill {
            name: "skill_b".into(),
            description: "Skill B".into(),
            body: String::new(),
            skill_dir: std::path::PathBuf::new(),
            compatibility: None,
        },
    ];
    let tool = build_activate_agent_skill_tool(&skills).unwrap();
    assert_eq!(tool_name(&tool), "activate_agent_skill");
    // Schema should list skill names in the enum
    match &tool {
        ChatCompletionTools::Function(t) => {
            let params = t.function.parameters.as_ref().unwrap();
            let name_enum = params["properties"]["name"]["enum"].as_array().unwrap();
            let names: Vec<&str> = name_enum.iter().map(|v| v.as_str().unwrap()).collect();
            assert!(names.contains(&"skill_a"));
            assert!(names.contains(&"skill_b"));
        }
        _ => panic!("Expected Function tool"),
    }
}

#[test]
fn activate_agent_skill_empty_list() {
    let tool = build_activate_agent_skill_tool(&[]).unwrap();
    assert_eq!(tool_name(&tool), "activate_agent_skill");
}

// ── Deferred tool loading tests ─────────────────────────────────────────

#[test]
fn search_tools_schema_valid() {
    let tool = build_search_tools_tool().unwrap();
    assert_eq!(tool_name(&tool), "search_tools");
    let required = required_fields(&tool);
    assert!(
        required.contains(&"query".to_string()),
        "search_tools must require query param"
    );
}

#[test]
fn stub_tool_has_deferred_marker() {
    let tool = build_stub_tool("calendar_create_event", "Create a calendar event").unwrap();
    assert_eq!(tool_name(&tool), "calendar_create_event");
    match &tool {
        ChatCompletionTools::Function(t) => {
            assert!(
                t.function
                    .description
                    .as_deref()
                    .unwrap_or("")
                    .contains("[DEFERRED]"),
                "Stub description must contain [DEFERRED] marker"
            );
        }
        _ => panic!("Expected Function tool"),
    }
}

#[test]
fn stub_tool_has_empty_params() {
    let tool = build_stub_tool("test_tool", "Test").unwrap();
    match &tool {
        ChatCompletionTools::Function(t) => {
            let params = t.function.parameters.as_ref().unwrap();
            let props = params.get("properties").and_then(|p| p.as_object());
            assert!(
                props.is_none_or(|p| p.is_empty()),
                "Stub should have empty properties"
            );
        }
        _ => panic!("Expected Function tool"),
    }
}

// ── Gemini compatibility: required ⊆ properties ─────────────────────
//
// Google AI Studio (Gemini) strictly validates that every name in a tool's
// `parameters.required` array exists as a key in `parameters.properties`.
// OpenAI-compatible providers (including x-ai/grok via OpenRouter) are
// lenient, which hid bugs where `required` listed properties that were
// never declared. Gemini returns HTTP 400 INVALID_ARGUMENT for every call
// with tools attached when this invariant is violated, so we enforce it at
// build time.
//
// This test walks EVERY public builder + `mcp_pseudo_tool_schemas()` and
// collects all offenders in one run, so a single CI failure lists every
// broken tool rather than stopping at the first.

/// Verify `required ⊆ properties` for one tool; returns an offender string
/// describing the violation, or `None` if the schema is valid (or non-object).
fn required_subset_offender(tool: &ChatCompletionTools) -> Option<String> {
    let (name, params) = match tool {
        ChatCompletionTools::Function(t) => (
            t.function.name.clone(),
            t.function.parameters.as_ref()?.clone(),
        ),
        _ => return None,
    };
    // Only object-typed schemas are subject to this check.
    let is_object = params
        .get("type")
        .and_then(|v| v.as_str())
        .is_some_and(|s| s == "object");
    if !is_object {
        return None;
    }
    let required: Vec<String> = params
        .get("required")
        .and_then(|r| r.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let properties_keys: std::collections::BTreeSet<String> = params
        .get("properties")
        .and_then(|p| p.as_object())
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default();

    let missing: Vec<String> = required
        .iter()
        .filter(|r| !properties_keys.contains(*r))
        .cloned()
        .collect();

    if missing.is_empty() {
        None
    } else {
        Some(format!(
            "Tool {:?} has required={:?} but properties keys are {:?}; offenders: {:?}",
            name, required, properties_keys, missing
        ))
    }
}

fn all_static_tools() -> Vec<ChatCompletionTools> {
    let builders: &[fn() -> Result<ChatCompletionTools>] = &[
        build_text_to_speech_tool,
        build_generate_image_tool,
        build_final_answer_tool,
        build_search_tools_tool,
        build_web_search_tool,
        build_x_search_tool,
        build_update_setting_tool,
        build_switch_model_tool,
        build_trigger_corpus_reembed_tool,
        build_unset_embedding_primary_promotion_tool,
        build_notify_user_tool,
        build_get_current_time_tool,
        build_generate_personal_brief_tool,
        build_recall_memory_tool,
        build_view_logs_tool,
        build_set_log_level_tool,
        build_set_bot_commands_tool,
        build_deactivate_skill_tool,
        build_reactivate_skill_tool,
        build_manage_skill_tool,
        build_manage_tasks_tool,
        build_manage_goals_tool,
        build_delegate_task_tool,
        build_run_external_tool,
        build_browser_fetch_tool,
        build_secure_computer_use_tool,
        build_knowledge_base_tool,
        build_check_api_usage_tool,
        build_setup_google_tool,
        build_google_drive_upload_tool,
        build_calendar_create_event_tool,
        build_calendar_update_event_tool,
        build_calendar_delete_event_tool,
        build_schedule_intent_tool,
        build_list_intents_tool,
        build_cancel_intent_tool,
        build_set_monitoring_intent_tool,
        build_list_monitoring_intents_tool,
        build_remove_monitoring_intent_tool,
        build_set_alert_policy_tool,
        build_list_alert_policies_tool,
        build_mcp_list_servers_tool,
        build_mcp_reconnect_server_tool,
        build_configure_autonomous_tool,
        build_noop_tool,
        build_write_memory_tool,
        build_write_next_tick_note_tool,
        build_propose_config_change_tool,
        build_send_message_tool,
        build_update_heartbeat_memo_tool,
        build_update_profile_tool,
        build_view_heartbeat_audit_tool,
        build_view_growth_tool,
        build_adjust_personality_tool,
    ];

    let mut out: Vec<ChatCompletionTools> = builders
        .iter()
        .map(|b| b().expect("static tool builder must succeed"))
        .collect();

    out.push(build_stub_tool("deferred_probe", "probe").unwrap());
    out.push(build_activate_agent_skill_tool(&[]).unwrap());
    out.push(
        build_activate_agent_skill_tool(&[crate::agent_skills::AgentSkill {
            name: "probe_skill".into(),
            description: "probe".into(),
            body: String::new(),
            skill_dir: std::path::PathBuf::new(),
            compatibility: None,
        }])
        .unwrap(),
    );

    out.extend(mcp_pseudo_tool_schemas());
    out
}

#[test]
fn all_static_tools_required_subset_of_properties() {
    let tools = all_static_tools();
    let offenders: Vec<String> = tools.iter().filter_map(required_subset_offender).collect();
    assert!(
        offenders.is_empty(),
        "Gemini strict-validation would reject {} tool schema(s):\n{}",
        offenders.len(),
        offenders.join("\n")
    );
}

// ── Gemini rejects array-typed `type` (JSON-Schema union) ──────────────
//
// OpenAI / Anthropic accept `"type": ["string", "boolean", "number"]`, but
// Gemini's strict validator requires `type` to be a single string. When
// it sees an array, it silently drops the offending property and then
// complains that `required` references an undefined property, returning
// HTTP 400 INVALID_ARGUMENT. Every Gemini call with such a tool attached
// fails, so we walk every static builder recursively and forbid array-
// typed `type` at any depth (top-level, nested objects, oneOf branches,
// items schemas, etc.).
//
// Regression: commit 8588257 (2026-03-30) introduced this on
// `update_setting.value`; Gemini 3.1 Flash Lite traffic (vision routing)
// started failing silently for three weeks before detection.

/// Recursively walk a JSON schema value; push a descriptive offender
/// string into `out` for every location where `"type"` is a JSON array.
fn find_array_type_offenders(
    tool_name: &str,
    path: &str,
    node: &serde_json::Value,
    out: &mut Vec<String>,
) {
    match node {
        serde_json::Value::Object(map) => {
            if let Some(t) = map.get("type")
                && t.is_array()
            {
                out.push(format!(
                    "Tool {tool_name:?} at {path}: type is a JSON array ({t}); Gemini rejects union types",
                ));
            }
            for (k, v) in map {
                let child_path = if path.is_empty() {
                    k.clone()
                } else {
                    format!("{path}.{k}")
                };
                find_array_type_offenders(tool_name, &child_path, v, out);
            }
        }
        serde_json::Value::Array(items) => {
            for (i, v) in items.iter().enumerate() {
                let child_path = format!("{path}[{i}]");
                find_array_type_offenders(tool_name, &child_path, v, out);
            }
        }
        _ => {}
    }
}

#[test]
fn all_static_tools_have_single_type_not_union() {
    let tools = all_static_tools();
    let mut offenders: Vec<String> = Vec::new();
    for tool in &tools {
        let ChatCompletionTools::Function(t) = tool else {
            continue;
        };
        let name = t.function.name.clone();
        if let Some(params) = t.function.parameters.as_ref() {
            find_array_type_offenders(&name, "", params, &mut offenders);
        }
    }
    assert!(
        offenders.is_empty(),
        "Gemini rejects array-typed `type` in {} location(s):\n{}",
        offenders.len(),
        offenders.join("\n")
    );
}

#[test]
fn all_static_tools_have_object_parameters_schema() {
    let tools = all_static_tools();
    let mut offenders: Vec<String> = Vec::new();
    for tool in &tools {
        let ChatCompletionTools::Function(t) = tool else {
            continue;
        };
        let Some(params) = t.function.parameters.as_ref() else {
            offenders.push(format!("{}: missing parameters", t.function.name));
            continue;
        };
        if params.get("type").and_then(serde_json::Value::as_str) != Some("object") {
            offenders.push(format!(
                "{}: parameters.type must be object, got {}",
                t.function.name, params
            ));
        }
    }
    assert!(
        offenders.is_empty(),
        "OpenAI/Codex function tools require object parameters:\n{}",
        offenders.join("\n")
    );
}

#[test]
fn build_tool_from_capability_rejects_required_not_in_properties() {
    // Skill manifests can embed arbitrary JSON schemas; guard against
    // malformed schemas leaking into the Gemini tool list.
    let cap = test_capability(
        "broken_skill",
        "Has a stale required entry",
        serde_json::json!({
            "type": "object",
            "properties": { "query": { "type": "string" } },
            "required": ["query", "missing_field"]
        }),
    );
    let err =
        build_tool_from_capability(&cap).expect_err("malformed skill schema must be rejected");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("broken_skill"),
        "error must name the skill, got: {msg}"
    );
    assert!(
        msg.contains("missing_field"),
        "error must name the offending required key, got: {msg}"
    );
}

#[test]
fn build_tool_from_capability_validates_after_multi_account_injection() {
    // `account` is injected by build_tool_from_capability when
    // supports_multi_account is true. The validator MUST see the injected
    // property — otherwise a multi-account skill that legitimately lists
    // "account" in `required` would be dropped as malformed.
    //
    // This test pins the ordering contract: `required` names "account"
    // which is NOT in `properties` pre-injection. If validation ever runs
    // BEFORE injection (ordering regression), this test fails. It passes
    // only when validation runs AFTER injection so the injected `account`
    // property satisfies the required-subset-of-properties invariant.
    let mut cap = test_capability(
        "multi_acc_required_account",
        "Reads email",
        serde_json::json!({
            "type": "object",
            "properties": { "folder": { "type": "string" } },
            "required": ["folder", "account"]
        }),
    );
    cap.supports_multi_account = true;
    let tool = build_tool_from_capability(&cap).expect(
        "multi-account tool with required=[account] must build (validation runs AFTER injection)",
    );
    assert!(
        required_subset_offender(&tool).is_none(),
        "multi-account skill with valid required must pass validator"
    );
}

#[test]
fn skill_stub_summary_takes_first_sentence_and_caps_at_120() {
    assert_eq!(
        skill_stub_summary("Read calendar events. Extra detail follows."),
        "Read calendar events."
    );
    assert_eq!(skill_stub_summary("中文说明。后面还有解释"), "中文说明。");
    let long = "x".repeat(200);
    let out = skill_stub_summary(&long);
    assert!(out.chars().count() <= 120, "{}", out.chars().count());
    assert!(out.ends_with('…'));
}

#[test]
fn google_calendar_and_drive_manifests_expose_account_parameter() {
    let private_manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("core has workspace parent")
        .join("skills/calendar_reader/manifest.json");
    if !private_manifest.exists() {
        // These account-scoped Google skill crates are not distributed in the
        // curated OSS snapshot; this assertion remains for private builds.
        return;
    }
    for skill_name in ["calendar_reader", "google_drive_reader"] {
        let cap = load_repo_skill_manifest(skill_name);
        assert!(
            cap.supports_multi_account,
            "{skill_name} must opt into account-scoped Google auth"
        );
        let tool = build_tool_from_capability(&cap).expect("build Google skill tool schema");
        match tool {
            ChatCompletionTools::Function(ct) => {
                let params = ct.function.parameters.expect("tool params");
                let props = params
                    .get("properties")
                    .and_then(serde_json::Value::as_object)
                    .expect("object properties");
                assert!(
                    props.contains_key("account"),
                    "{skill_name} schema must expose account"
                );
            }
            _ => panic!("expected Function tool"),
        }
    }
}
