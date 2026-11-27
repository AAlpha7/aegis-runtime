//! Validate candidate call settings without mutating the active configuration.
use crate::admin::AdminState;
use std::{
    future::Future,
    sync::{Arc, LazyLock},
    time::Duration,
};

// Deliberately no Debug/Serialize: Jev candidates contain a secret.
pub enum Candidate {
    Chat {
        provider: String,
        model: String,
        effort: Option<String>,
    },
    Search {
        provider: String,
        model: String,
    },
    Jev {
        key: String,
    },
}

pub type SettingsWriteGuard = tokio::sync::OwnedMutexGuard<()>;
pub static SETTINGS_WRITE: LazyLock<Arc<tokio::sync::Mutex<()>>> =
    LazyLock::new(|| Arc::new(tokio::sync::Mutex::new(())));

/// Includes disables/resets, which still change a concurrent probe's context.
/// Unrelated settings need only the memory actor's transaction serialization.
pub(crate) fn affects_call_validation(key: &str) -> bool {
    matches!(
        key,
        "llm_provider"
            | "llm_model"
            | "web_search_provider"
            | "web_search_model"
            | "codex_model"
            | "codex_reasoning_effort"
            | "xai_subscription_model"
            | "fallback_model"
            | "paid_api_fallback_enabled"
            | "fallback_model_authorized"
            | "codex_enabled"
            | "xai_subscription_enabled"
            | "external_executor_enabled"
            | "codex_cli_binary"
            | "codex_cli_home"
            | "codex_cli_workdir"
            | "xai_subscription_api_base"
    )
}

/// Generic admin/chat setting writes cannot bypass the verified owner flow.
/// Disabling or clearing a backup is deliberately possible during an outage.
pub fn requires_verified_settings_path(key: &str, value: &serde_json::Value) -> bool {
    match key {
        "llm_provider"
        | "llm_model"
        | "web_search_provider"
        | "web_search_model"
        | "codex_model"
        | "codex_reasoning_effort"
        | "xai_subscription_model" => true,
        "fallback_model" => value.as_str() != Some(""),
        "paid_api_fallback_enabled"
        | "fallback_model_authorized"
        | "codex_enabled"
        | "xai_subscription_enabled" => value.as_bool() != Some(false),
        _ => false,
    }
}

pub fn settings_path_message() -> &'static str {
    "未保存：请在设置 → 高级 → AI 与花费中验证并保存调用配置。Not saved: verify and save call configuration in Settings → Advanced → AI and spending."
}

type Probe = Arc<dyn Fn(&Candidate) -> anyhow::Result<()> + Send + Sync>;
tokio::task_local! { static INJECTED_PROBE: Probe; }

/// Explicit dependency injection for embedded test routers. No HTTP request,
/// stored setting or environment flag can select this implementation.
pub async fn with_test_probe<T>(probe: Probe, work: impl Future<Output = T>) -> T {
    INJECTED_PROBE.scope(probe, work).await
}

pub async fn verify(
    state: &AdminState,
    candidate: &Candidate,
) -> Result<(), (axum::http::StatusCode, String)> {
    let work = async {
        if let Ok(result) = INJECTED_PROBE.try_with(|probe| probe(candidate)) {
            return result;
        }
        match candidate {
            Candidate::Jev { key } => verify_jev(key).await,
            _ => {
                state
                    .orchestrator
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("runtime unavailable"))?
                    .probe_call_configuration(candidate)
                    .await
            }
        }
    };
    match tokio::time::timeout(Duration::from_secs(70), work).await {
        Ok(Ok(())) => Ok(()),
        result => {
            // Never return raw provider output, auth headers, keys or subprocess logs.
            let reason = match result {
                Err(_) => "timeout",
                Ok(Err(ref err)) => {
                    let text = err.to_string().to_lowercase();
                    if text.contains("401")
                        || text.contains("403")
                        || text.contains("auth")
                        || text.contains("credential")
                    {
                        "authentication"
                    } else if text.contains("429") || text.contains("quota") || text.contains("402")
                    {
                        "quota"
                    } else if text.contains("empty")
                        || text.contains("response")
                        || text.contains("source")
                    {
                        "invalid_response"
                    } else {
                        "unavailable"
                    }
                }
                _ => unreachable!(),
            };
            let zh = state.settings.display_language() == "zh";
            let detail = match (zh, reason) {
                (true, "timeout") => "请求超时",
                (true, "authentication") => "认证失败或凭据不可用",
                (true, "quota") => "额度不足或受到限流",
                (true, "invalid_response") => "未收到有效回复或搜索来源",
                (true, _) => "服务或运行时不可用",
                (false, "timeout") => "request timed out",
                (false, "authentication") => "authentication failed or credentials unavailable",
                (false, "quota") => "quota exhausted or rate limited",
                (false, "invalid_response") => "no valid response or search sources",
                (false, _) => "service or runtime unavailable",
            };
            Err((
                axum::http::StatusCode::UNPROCESSABLE_ENTITY,
                if zh {
                    format!("验证失败：{detail}。未保存，原配置保持不变。")
                } else {
                    format!(
                        "Validation failed: {detail}. Not saved; previous configuration is unchanged."
                    )
                },
            ))
        }
    }
}

/// Fixed-context live probe shared by settings and the first-run installer.
pub async fn verify_jev(key: &str) -> anyhow::Result<()> {
    use crate::computer_use::{DecisionsClient, JevModel};
    let model = JevModel::from_env();
    let client = DecisionsClient::new(key, model.clone())?;
    verify_jev_client(&client).await
}

async fn verify_jev_client(client: &crate::computer_use::DecisionsClient) -> anyhow::Result<()> {
    use crate::computer_use::{DecisionsAnswer, DecisionsQuestion, DecisionsRequest};
    let response = client.decide(DecisionsRequest {
        model: client.model().as_str().into(),
        state: serde_json::json!({"purpose":"configuration_check", "proposed_action":"Read the public example.com home page. No user data."}),
        questions: [
            ("verdict".into(), DecisionsQuestion::tool_gate_choice("Allow, ask or deny this read-only public-page action.")),
            ("sensitive".into(), DecisionsQuestion::high_risk_noul("Does the proposed action disclose private data?")),
        ].into_iter().collect(),
    }).await?;
    anyhow::ensure!(!response.model.trim().is_empty(), "invalid response model");
    anyhow::ensure!(
        matches!(response.answers.get("verdict"), Some(DecisionsAnswer::Choice { choice, .. }) if matches!(choice.as_str(), "allow" | "ask" | "deny")),
        "invalid response verdict"
    );
    anyhow::ensure!(
        matches!(response.answers.get("sensitive"), Some(DecisionsAnswer::Noul { noul }) if noul.is_finite() && (0.0..=1.0).contains(noul)),
        "invalid response probability"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generic_call_settings_cannot_bypass_verification_but_can_disable() {
        for key in [
            "llm_provider",
            "llm_model",
            "web_search_provider",
            "web_search_model",
            "codex_model",
            "codex_reasoning_effort",
            "xai_subscription_model",
        ] {
            assert!(requires_verified_settings_path(
                key,
                &serde_json::json!("anything")
            ));
            assert!(requires_verified_settings_path(key, &serde_json::json!("")));
        }
        for key in [
            "paid_api_fallback_enabled",
            "fallback_model_authorized",
            "codex_enabled",
            "xai_subscription_enabled",
        ] {
            assert!(requires_verified_settings_path(
                key,
                &serde_json::json!(true)
            ));
            assert!(!requires_verified_settings_path(
                key,
                &serde_json::json!(false)
            ));
        }
        assert!(!requires_verified_settings_path(
            "fallback_model",
            &serde_json::json!("")
        ));
        assert!(!requires_verified_settings_path(
            "display_language",
            &serde_json::json!("en")
        ));
    }

    #[tokio::test]
    async fn jev_call_validation_requires_semantic_answers_not_just_http_success() {
        use crate::computer_use::{DecisionsClient, JevModel};
        use axum::{Router, routing::post};
        for (status, body, valid) in [
            (
                200,
                serde_json::json!({"model":"test", "answers":{"verdict":{"type":"choice","choice":"allow"},"sensitive":{"type":"noul","noul":0.01}}}),
                true,
            ),
            (
                200,
                serde_json::json!({"model":"test", "answers":{}}),
                false,
            ),
            (
                200,
                serde_json::json!({"model":"test", "answers":{"verdict":{"type":"choice","choice":"unexpected"},"sensitive":{"type":"noul","noul":0.01}}}),
                false,
            ),
            (
                200,
                serde_json::json!({"model":"test", "answers":{"verdict":{"type":"choice","choice":"ask"},"sensitive":{"type":"noul","noul":2.0}}}),
                false,
            ),
            (200, serde_json::json!({"error":"invalid protocol"}), false),
            (
                401,
                serde_json::json!({"error":"synthetic authentication failure"}),
                false,
            ),
        ] {
            let requests = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let counter = requests.clone();
            let app = Router::new().route(
                "/decisions",
                post(move |axum::Json(request): axum::Json<serde_json::Value>| {
                    let body = body.clone();
                    counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    async move {
                        assert_eq!(request["state"]["purpose"], "configuration_check");
                        (
                            axum::http::StatusCode::from_u16(status).unwrap(),
                            axum::Json(body),
                        )
                    }
                }),
            );
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
            let client = DecisionsClient::new("synthetic-key", JevModel::from_env())
                .unwrap()
                .with_url(format!("http://{addr}/decisions"));
            assert_eq!(verify_jev_client(&client).await.is_ok(), valid);
            assert_eq!(requests.load(std::sync::atomic::Ordering::SeqCst), 1);
            server.abort();
        }
    }
}
