//! Web-search and multimedia tool handlers extracted from `orchestrator.rs`.
//!
//! Contains `handle_tool_web` (web_search, x_search) and
//! `handle_tool_multimedia` (text_to_speech, generate_image).

use anyhow::Result;
use async_openai::types::chat::ChatCompletionMessageToolCall;
use serde_json::Value;

use crate::orchestrator::{Orchestrator, ToolContext, send_image_b64_to_telegram};
use crate::utils::{sanitized_tool_error, truncate_str};

fn search_route_receipt(raw: &str) -> Value {
    let value: Value = serde_json::from_str(raw).unwrap_or(Value::Null);
    let mut receipt = serde_json::json!({});
    for key in [
        "source",
        "model",
        "fallback_used",
        "fallback_attempted",
        "fallback_failure",
        "primary_provider",
        "primary_failure",
        "reason",
    ] {
        if let Some(field) = value.get(key) {
            receipt[key] = field.clone();
        }
    }
    receipt
}

impl Orchestrator {
    /// Handle web-search domain tools: web_search, x_search.
    #[allow(unused_variables, clippy::too_many_lines)]
    pub(crate) async fn handle_tool_web(
        &self,
        tool_call: &ChatCompletionMessageToolCall,
        tool_name: &str,
        tool_args_str: &str,
        ctx: &mut ToolContext<'_>,
    ) -> Result<Option<bool>> {
        tool_dispatch!(self, tool_call, tool_name, tool_args_str, ctx,
            "web_search"    => execute_web_search_tool,
            "x_search"      => execute_x_search_tool,
        );
        Ok(None)
    }

    /// Handle multimedia domain tools: text_to_speech, generate_image.
    #[allow(unused_variables, clippy::too_many_lines)]
    pub(crate) async fn handle_tool_multimedia(
        &self,
        tool_call: &ChatCompletionMessageToolCall,
        tool_name: &str,
        tool_args_str: &str,
        ctx: &mut ToolContext<'_>,
    ) -> Result<Option<bool>> {
        tool_dispatch!(self, tool_call, tool_name, tool_args_str, ctx,
            "text_to_speech" => execute_text_to_speech,
            "generate_image" => execute_generate_image,
        );
        Ok(None)
    }

    // ──────────────────────────────────────────────────────────────────────
    //  Web-search tool handlers
    // ──────────────────────────────────────────────────────────────────────

    async fn execute_web_search_tool(&self, args: Value, ctx: &mut ToolContext<'_>) -> String {
        if args.get("research_scope").and_then(Value::as_bool) == Some(true)
            && ctx.gate_pre_approved
        {
            let scope = crate::search_turn::query_from_tool_args(&args.to_string());
            return match self
                .execute_research_lookups(&scope, ctx.user_did, ctx.chat_id, &[])
                .await
            {
                Ok(text) => text,
                Err(error) => {
                    tracing::warn!(%error, "[Research] approved scope did not finish");
                    crate::search_turn::failure_line(&scope)
                }
            };
        }
        let prompt = ctx
            .search_turn
            .as_ref()
            .and_then(|ledger| ledger.query_override.clone())
            .filter(|query| !query.trim().is_empty())
            .unwrap_or_else(|| crate::search_turn::query_from_tool_args(&args.to_string()));
        if let Some(ledger) = ctx.search_turn.as_mut() {
            ledger.query_override = None;
        }
        if prompt.is_empty() {
            return serde_json::json!({
                "status": "error",
                "error": "search_prompt parameter is required and cannot be empty",
            })
            .to_string();
        }
        if ctx
            .search_turn
            .as_ref()
            .is_some_and(|ledger| ledger.suppress_outbound)
        {
            return crate::search_turn::outbound_stopped_result();
        }
        if ctx
            .search_turn
            .as_ref()
            .is_some_and(|ledger| !crate::search_turn::public_lookup_open(ledger))
        {
            return crate::search_turn::budget_stopped_result();
        }
        self.run_public_lookups(&prompt, ctx).await
    }

    async fn run_public_lookups(&self, prompt: &str, ctx: &mut ToolContext<'_>) -> String {
        let redacted = crate::outbound_privacy::redact_outbound_text(prompt);
        let first = self.execute_web_search(&redacted.text).await;
        let mut routes = vec![search_route_receipt(&first)];
        let first_record = crate::search_turn::lookup_from_payload(&redacted.text, &first);
        let mut records = vec![first_record];
        if let Some(ledger) = ctx.search_turn.as_mut() {
            ledger.lookups_used = ledger.lookups_used.saturating_add(1);
            ledger.records.push(records[0].clone());
        }
        if crate::search_turn::needs_second_lookup(&records[0])
            && ctx
                .search_turn
                .as_ref()
                .is_none_or(|ledger| ledger.lookups_used < crate::search_turn::PUBLIC_LOOKUP_BUDGET)
        {
            crate::behavior_notice::record(
                "search_broadened",
                "首次搜索没有充分回答问题，已放宽检索词再查一次；原问题不变，但搜索范围扩大。",
                "The first search did not sufficiently answer the question. Retrying with broader search terms; the original question is unchanged, but the search scope is wider.",
            );
            let broader = crate::outbound_privacy::redact_outbound_text(
                &crate::search_turn::broaden_query(&redacted.text),
            );
            let second = self.execute_web_search(&broader.text).await;
            routes.push(search_route_receipt(&second));
            let second_record = crate::search_turn::lookup_from_payload(&broader.text, &second);
            if let Some(ledger) = ctx.search_turn.as_mut() {
                ledger.lookups_used = ledger.lookups_used.saturating_add(1);
                ledger.records.push(second_record.clone());
            }
            records.push(second_record);
        }
        if records.len() == 1 && records[0].failed {
            return self.with_privacy_note(first, &redacted.hits);
        }
        let summary = serde_json::from_str::<Value>(&first)
            .ok()
            .and_then(|value| {
                value
                    .get("summary")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .unwrap_or_default();
        let owner_question = ctx
            .search_turn
            .as_ref()
            .map(|ledger| ledger.owner_question.as_str())
            .unwrap_or("");
        let shaped =
            crate::search_turn::model_payload(&redacted.text, &records, &summary, owner_question);
        let mut shaped: Value =
            serde_json::from_str(&shaped).unwrap_or_else(|_| serde_json::json!({"status":"error"}));
        shaped["search_routes"] = Value::Array(routes);
        self.with_privacy_note(shaped.to_string(), &redacted.hits)
    }

    fn with_privacy_note(&self, result: String, hits: &[&'static str]) -> String {
        if hits.is_empty() {
            result
        } else {
            tracing::warn!(hits = ?hits, "[Privacy] withheld private literals from web_search prompt");
            format!(
                "{result}\n\n{}",
                crate::outbound_privacy::privacy_note(hits)
            )
        }
    }

    async fn execute_x_search_tool(&self, args: Value, ctx: &mut ToolContext<'_>) -> String {
        let query = args
            .get("query")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .unwrap_or("");
        if query.is_empty() {
            return serde_json::json!({
                "status": "error",
                "error": "query parameter is required and cannot be empty",
            })
            .to_string();
        }
        let allowed = string_list_arg(&args, "allowed_x_handles", 20);
        let excluded = string_list_arg(&args, "excluded_x_handles", 20);
        if !allowed.is_empty() && !excluded.is_empty() {
            return serde_json::json!({
                "status": "error",
                "error": "allowed_x_handles and excluded_x_handles are mutually exclusive",
            })
            .to_string();
        }
        let from_date = args
            .get("from_date")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let to_date = args
            .get("to_date")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty());
        if ctx
            .search_turn
            .as_ref()
            .is_some_and(|ledger| ledger.suppress_outbound)
        {
            return crate::search_turn::outbound_stopped_result();
        }
        let allow_web_fallback = ctx
            .search_turn
            .as_ref()
            .is_none_or(|ledger| crate::search_turn::public_lookup_open(ledger));
        let result = self
            .execute_x_search(
                query,
                &allowed,
                &excluded,
                from_date,
                to_date,
                allow_web_fallback,
            )
            .await;
        if serde_json::from_str::<Value>(&result)
            .ok()
            .is_some_and(|value| value["x_search_web_fallback"] == true)
            && let Some(ledger) = ctx.search_turn.as_mut()
        {
            let record = crate::search_turn::lookup_from_payload(query, &result);
            ledger.lookups_used = ledger.lookups_used.saturating_add(1);
            ledger.records.push(record);
        }
        result
    }

    /// One research job. A private scope is approved once; later lookups are not.
    pub(crate) async fn run_research_job(
        &self,
        scope: &str,
        user_did: &str,
        chat_id: Option<i64>,
        tried: &[String],
    ) -> anyhow::Result<String> {
        let scope = scope.trim();
        if scope.is_empty() {
            return Ok(crate::search_turn::failure_line(scope));
        }
        if let Some(wait) = self.hold_research_scope(scope, user_did, chat_id).await? {
            return Ok(wait);
        }
        self.execute_research_lookups(scope, user_did, chat_id, tried)
            .await
    }

    async fn hold_research_scope(
        &self,
        scope: &str,
        user_did: &str,
        chat_id: Option<i64>,
    ) -> anyhow::Result<Option<String>> {
        let tool_args = serde_json::json!({
            "query": scope,
            "search_prompt": scope,
            "research_scope": true,
        })
        .to_string();
        let observations = [crate::egress_gate::Observation::Owner(scope.to_string())];
        let secrets = crate::egress_gate::env_secret_values();
        let input = crate::egress_gate::EgressInput {
            tool_name: "web_search",
            tool_args: &tool_args,
            network_domains: &[],
            calendar_invite_others: false,
            observations: &observations,
            previous_results: &[],
            secret_values: &secrets,
        };
        let judge = crate::egress_gate::JevSensitivityJudge::with_key(self.jev_active_key());
        let crate::egress_gate::EgressDecision::Hold(hold) =
            crate::egress_gate::evaluate_egress(&input, &judge).await
        else {
            return Ok(None);
        };
        let approval_id = format!("gate-{:016x}", rand::random::<u64>());
        let summary = crate::egress_gate::with_owner_question(&hold.card_summary(), scope);
        if self
            .approved_search_hold_exists("web_search", &tool_args, &summary)
            .await
        {
            return Ok(None);
        }
        let approval_data = serde_json::json!({
            "approval_id": approval_id,
            "tool_name": "web_search",
            "tool_args": tool_args,
            "approval_summary": summary,
            "user_did": user_did,
            "chat_id": chat_id,
            "is_callback": false,
            "created_at": chrono::Utc::now().to_rfc3339(),
        });
        self.memory
            .store_skill_state("hitl", &approval_id, &approval_data.to_string())
            .await?;
        if let (Some(tx), Some(cid)) = (&self.progress_tx, chat_id) {
            let _ = tx.try_send(crate::progress_router::ProgressEvent::new(
                cid,
                crate::progress_router::ProgressPayload::NeedApproval {
                    approval_id: Some(approval_id),
                    tool_name: Some("web_search".to_string()),
                    summary: Some(summary.clone()),
                    approval_html: summary,
                },
            ));
        }
        Ok(Some("等待你批准这次搜索。".to_string()))
    }

    pub(crate) async fn execute_research_lookups(
        &self,
        scope: &str,
        user_did: &str,
        chat_id: Option<i64>,
        tried: &[String],
    ) -> anyhow::Result<String> {
        let _ = self
            .memory
            .delete_skill_state(crate::search_turn::RESEARCH_STOP_SKILL, user_did)
            .await;
        let job_id = format!("research-{:016x}", rand::random::<u64>());
        let running =
            crate::search_turn::research_job_json(job_id.as_str(), user_did, scope, "running", "");
        if let Err(error) = self
            .memory
            .store_skill_state(
                crate::search_turn::RESEARCH_JOB_SKILL,
                &job_id,
                &running.to_string(),
            )
            .await
        {
            tracing::warn!(%error, "[Research] could not store the running job");
        }
        if let (Some(tx), Some(cid)) = (&self.progress_tx, chat_id) {
            let _ = tx.try_send(crate::progress_router::ProgressEvent::new(
                cid,
                crate::progress_router::ProgressPayload::OwnerNotice {
                    text: crate::search_turn::research_progress_line(scope),
                    kind: crate::progress_router::OwnerNoticeKind::System,
                },
            ));
        }
        let queries = crate::search_turn::research_queries(scope, tried);
        let mut records = Vec::new();
        let mut stopped_early = false;
        for query in &queries {
            if self.research_stopped(user_did).await {
                stopped_early = true;
                break;
            }
            let redacted = crate::outbound_privacy::redact_outbound_text(query);
            let raw = self.execute_web_search(&redacted.text).await;
            let record = crate::search_turn::lookup_from_payload(&redacted.text, &raw);
            let answered = record.answered && !record.failed;
            records.push(record);
            if answered {
                break;
            }
        }
        let text = crate::search_turn::research_result_text(scope, &records, stopped_early);
        let status = if stopped_early { "stopped" } else { "done" };
        let finished =
            crate::search_turn::research_job_json(&job_id, user_did, scope, status, &text);
        if let Err(error) = self
            .memory
            .store_skill_state(
                crate::search_turn::RESEARCH_JOB_SKILL,
                &job_id,
                &finished.to_string(),
            )
            .await
        {
            tracing::warn!(%error, "[Research] could not store the finished job");
        }
        Ok(text)
    }

    async fn research_stopped(&self, user_did: &str) -> bool {
        self.memory
            .load_skill_state(crate::search_turn::RESEARCH_STOP_SKILL, user_did)
            .await
            .ok()
            .flatten()
            .as_deref()
            == Some("1")
    }

    // ──────────────────────────────────────────────────────────────────────
    //  Multimedia tool handlers
    // ──────────────────────────────────────────────────────────────────────

    async fn execute_text_to_speech(&self, args: Value, ctx: &mut ToolContext<'_>) -> String {
        let chat_id = ctx.chat_id;

        if let Some(tts) = &self.services.tts {
            let text = args.get("text").and_then(|v| v.as_str()).unwrap_or("");
            let language = args.get("language").and_then(|v| v.as_str());
            match tts.synthesize(text, language).await {
                Ok(audio_bytes) => {
                    // Send as voice message via Telegram
                    if let (Some(bot), Some(cid)) = (&self.services.telegram_bot, chat_id) {
                        use teloxide::prelude::Requester;
                        let input = teloxide::types::InputFile::memory(audio_bytes)
                            .file_name("tts_output.mp3");
                        use teloxide::payloads::SendVoiceSetters;
                        let thread = crate::telegram::wrap_thread_id(
                            crate::topic_router::resolve_thread_id(
                                cid,
                                crate::topic_router::TopicKind::Main,
                            )
                            .await,
                        );
                        let mut req = bot.send_voice(crate::telegram::admin_dm_chat(cid), input);
                        if let Some(tid) = thread {
                            req = req.message_thread_id(tid);
                        }
                        if let Err(e) = req.await {
                            sanitized_tool_error(
                                "text_to_speech",
                                "Failed to send voice message / \u{53d1}\u{9001}\u{8bed}\u{97f3}\u{6d88}\u{606f}\u{5931}\u{8d25}",
                                &e,
                                "telegram_error",
                                true,
                                None,
                            )
                        } else {
                            tracing::info!(text_len = text.len(), "TTS voice message sent");
                            "{\"status\":\"success\",\"message\":\"Voice message sent to user.\"}"
                                .to_string()
                        }
                    } else {
                        "{\"status\":\"error\",\"error\":\"Telegram bot not configured for TTS output\"}".to_string()
                    }
                }
                Err(e) => sanitized_tool_error(
                    "text_to_speech",
                    "TTS synthesis failed / \u{8bed}\u{97f3}\u{5408}\u{6210}\u{5931}\u{8d25}",
                    &e,
                    "tts_error",
                    true,
                    Some("Try again with shorter text."),
                ),
            }
        } else {
            "{\"status\":\"error\",\"error\":\"TTS not configured (AEGIS_TTS_API_KEY missing)\"}"
                .to_string()
        }
    }

    async fn execute_generate_image(&self, args: Value, ctx: &mut ToolContext<'_>) -> String {
        let chat_id = ctx.chat_id;
        let web_primary_delivery = ctx.data_access_context.is_some();

        let prompt = args.get("prompt").and_then(|v| v.as_str()).unwrap_or("");
        let aspect_ratio = args
            .get("aspect_ratio")
            .and_then(|v| v.as_str())
            .unwrap_or("1:1");

        let direct_key = std::env::var("AEGIS_IMAGEGEN_API_KEY")
            .ok()
            .filter(|k| !k.is_empty());

        let result_msg = if let Some(api_key) = direct_key {
            // -- Direct provider API (xAI Aurora / OpenAI / etc.) --
            let base_url = std::env::var("AEGIS_IMAGEGEN_BASE_URL")
                .unwrap_or_else(|_| "https://api.x.ai/v1".to_string());
            let model = std::env::var("AEGIS_IMAGEGEN_MODEL")
                .unwrap_or_else(|_| "grok-imagine-image-2.0".to_string());

            tracing::info!(prompt_len = prompt.len(), %model, "generating image via direct API");

            let body = serde_json::json!({
                "model": model,
                "prompt": prompt,
                "n": 1,
                "response_format": "b64_json",
                "aspect_ratio": aspect_ratio,
            });

            match self
                .http_client
                .post(format!("{}/images/generations", base_url))
                .header("Authorization", format!("Bearer {api_key}"))
                .json(&body)
                .send()
                .await
            {
                Ok(resp) if resp.status().is_success() => match resp.json::<Value>().await {
                    Ok(j) => {
                        if let Some(b64) = j["data"][0]["b64_json"].as_str() {
                            if web_primary_delivery {
                                self.store_generated_image_for_web(ctx.user_did, b64)
                            } else if let (Some(bot), Some(cid)) =
                                (&self.services.telegram_bot, chat_id)
                            {
                                send_image_b64_to_telegram(b64, bot, cid).await
                            } else {
                                self.store_generated_image_for_web(ctx.user_did, b64)
                            }
                        } else {
                            "{\"status\":\"error\",\"error\":\"No image data in response\"}".into()
                        }
                    }
                    Err(e) => sanitized_tool_error(
                        "generate_image",
                        "Image API response parse failed / \u{56fe}\u{7247}API\u{54cd}\u{5e94}\u{89e3}\u{6790}\u{5931}\u{8d25}",
                        &e,
                        "api_error",
                        true,
                        None,
                    ),
                },
                Ok(resp) => {
                    let s = resp.status();
                    let err = resp.text().await.unwrap_or_default();
                    tracing::warn!(%s, err = %truncate_str(&err, 500), "image gen direct API error response");
                    serde_json::json!({"status": "error", "error": format!("Image generation API error (status {})", s), "error_kind": "api_error", "retryable": true}).to_string()
                }
                Err(e) => sanitized_tool_error(
                    "generate_image",
                    "Image generation request failed / \u{56fe}\u{7247}\u{751f}\u{6210}\u{8bf7}\u{6c42}\u{5931}\u{8d25}",
                    &e,
                    "network_error",
                    true,
                    Some("Try again later."),
                ),
            }
        } else {
            // -- OpenRouter chat completions + modalities --
            let model = std::env::var("AEGIS_IMAGEGEN_MODEL")
                .unwrap_or_else(|_| "google/gemini-2.5-flash-image".to_string());

            tracing::info!(prompt_len = prompt.len(), %model, "generating image via OpenRouter");

            let body = serde_json::json!({
                "model": model,
                "messages": [{"role": "user", "content": prompt}],
                "modalities": ["image", "text"],
                "image_config": {"aspect_ratio": aspect_ratio},
            });

            match self
                .http_client
                .post(format!("{}/chat/completions", self.api_base))
                .header("Authorization", format!("Bearer {}", self.api_key))
                .json(&body)
                .send()
                .await
            {
                Ok(resp) if resp.status().is_success() => {
                    match resp.json::<Value>().await {
                        Ok(resp_json) => {
                            // Log response structure for debugging
                            let msg_keys: Vec<&str> = resp_json["choices"][0]["message"]
                                .as_object()
                                .map(|m| m.keys().map(|k| k.as_str()).collect())
                                .unwrap_or_default();
                            tracing::info!(
                                msg_keys = ?msg_keys,
                                has_images = resp_json["choices"][0]["message"]["images"].is_array(),
                                content_preview = %truncate_str(
                                    resp_json["choices"][0]["message"]["content"].as_str().unwrap_or("null"),
                                    200
                                ),
                                "image generation response structure"
                            );
                            // OpenRouter: images in choices[0].message.images[] or content as data URL
                            let b64_url =
                                resp_json["choices"][0]["message"]["images"][0]["image_url"]["url"]
                                    .as_str()
                                    .or_else(|| {
                                        let c = resp_json["choices"][0]["message"]["content"]
                                            .as_str()?;
                                        if c.starts_with("data:image") {
                                            Some(c)
                                        } else {
                                            None
                                        }
                                    });
                            if let Some(data_url) = b64_url {
                                if web_primary_delivery {
                                    self.store_generated_image_for_web(ctx.user_did, data_url)
                                } else if let (Some(bot), Some(cid)) =
                                    (&self.services.telegram_bot, chat_id)
                                {
                                    send_image_b64_to_telegram(data_url, bot, cid).await
                                } else {
                                    self.store_generated_image_for_web(ctx.user_did, data_url)
                                }
                            } else {
                                let text = resp_json["choices"][0]["message"]["content"]
                                    .as_str()
                                    .unwrap_or("No image returned");
                                tracing::warn!("image model returned no image data");
                                format!(
                                    "{{\"status\":\"error\",\"error\":\"No image returned: {}\"}}",
                                    truncate_str(text, 200)
                                )
                            }
                        }
                        Err(e) => sanitized_tool_error(
                            "generate_image",
                            "Image API response parse failed / \u{56fe}\u{7247}API\u{54cd}\u{5e94}\u{89e3}\u{6790}\u{5931}\u{8d25}",
                            &e,
                            "api_error",
                            true,
                            None,
                        ),
                    }
                }
                Ok(resp) => {
                    let s = resp.status();
                    let err = resp.text().await.unwrap_or_default();
                    tracing::warn!(%s, err = %truncate_str(&err, 500), "image gen OpenRouter error response");
                    serde_json::json!({"status": "error", "error": format!("Image generation API error (status {})", s), "error_kind": "api_error", "retryable": true}).to_string()
                }
                Err(e) => sanitized_tool_error(
                    "generate_image",
                    "Image generation request failed / \u{56fe}\u{7247}\u{751f}\u{6210}\u{8bf7}\u{6c42}\u{5931}\u{8d25}",
                    &e,
                    "network_error",
                    true,
                    Some("Try again later."),
                ),
            }
        };

        tracing::info!(prompt_len = prompt.len(), result = %truncate_str(&result_msg, 200), "generate_image complete");
        result_msg
    }

    fn store_generated_image_for_web(&self, owner_did: &str, image_data: &str) -> String {
        match crate::webapp_assets::store_generated_image_for_owner(
            self.memory.db_path(),
            owner_did,
            image_data,
            chrono::Utc::now(),
        ) {
            Ok(image_url) => serde_json::json!({
                "status": "success",
                "delivery_surface": "web",
                "image_url": image_url,
                "message": "Image generated and saved for the WebApp.",
            })
            .to_string(),
            Err(error) => serde_json::json!({
                "status": "error",
                "error": error,
                "error_kind": "storage_error",
                "retryable": true,
            })
            .to_string(),
        }
    }
}

fn string_list_arg(args: &Value, key: &str, max: usize) -> Vec<String> {
    args.get(key)
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .map(|s| s.trim().trim_start_matches('@').to_string())
                .filter(|s| !s.is_empty())
                .take(max)
                .collect()
        })
        .unwrap_or_default()
}

// ══════════════════════════════════════════════════════════════════════════════
//  Tests
// ══════════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::memory::MemoryHandle;
    use crate::settings::RuntimeSettings;
    use crate::test_tool_helpers::{CtxState, make_tool_call, test_orchestrator};

    // ── Routing ──────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn web_unknown_tool_passes_through() {
        let mem = MemoryHandle::spawn_in_memory().expect("in-memory DB");
        let settings = Arc::new(RuntimeSettings::new());
        let orch = test_orchestrator(mem, settings);

        let tc = make_tool_call("imaginary_web_tool", "{}");
        let mut state = CtxState::new();
        let result = orch
            .handle_tool_web(&tc, "imaginary_web_tool", "{}", &mut state.ctx())
            .await
            .expect("should not error");
        assert_eq!(result, None, "unknown tool must pass through");
        assert!(state.tools_called.is_empty());
    }

    #[tokio::test]
    async fn multimedia_unknown_tool_passes_through() {
        let mem = MemoryHandle::spawn_in_memory().expect("in-memory DB");
        let settings = Arc::new(RuntimeSettings::new());
        let orch = test_orchestrator(mem, settings);

        let tc = make_tool_call("auto_compose_music", "{}");
        let mut state = CtxState::new();
        let result = orch
            .handle_tool_multimedia(&tc, "auto_compose_music", "{}", &mut state.ctx())
            .await
            .expect("should not error");
        assert_eq!(result, None);
    }

    // ── x_search error paths ─────────────────────────────────────────────────

    #[tokio::test]
    async fn x_search_empty_query_returns_error() {
        let mem = MemoryHandle::spawn_in_memory().expect("in-memory DB");
        let settings = Arc::new(RuntimeSettings::new());
        let orch = test_orchestrator(mem, settings);

        let tc = make_tool_call("x_search", r#"{"query":""}"#);
        let mut state = CtxState::new();
        let result = orch
            .handle_tool_web(&tc, "x_search", r#"{"query":""}"#, &mut state.ctx())
            .await
            .expect("should not error");

        assert_eq!(result, Some(true));
        let content = format!("{:?}", state.messages.last().unwrap());
        assert!(
            content.contains("required") || content.contains("empty"),
            "must report empty query error, got: {content}"
        );
    }

    #[tokio::test]
    async fn x_search_rejects_both_handle_filters() {
        let mem = MemoryHandle::spawn_in_memory().expect("in-memory DB");
        let settings = Arc::new(RuntimeSettings::new());
        let orch = test_orchestrator(mem, settings);

        let args = r#"{"query":"ai","allowed_x_handles":["a"],"excluded_x_handles":["b"]}"#;
        let tc = make_tool_call("x_search", args);
        let mut state = CtxState::new();
        let result = orch
            .handle_tool_web(&tc, "x_search", args, &mut state.ctx())
            .await
            .expect("should not error");

        assert_eq!(result, Some(true));
        let content = format!("{:?}", state.messages.last().unwrap());
        assert!(
            content.contains("mutually exclusive"),
            "must reject both filters, got: {content}"
        );
    }

    // ── web_search error paths ───────────────────────────────────────────────

    #[tokio::test]
    async fn web_search_empty_prompt_returns_error() {
        let mem = MemoryHandle::spawn_in_memory().expect("in-memory DB");
        let settings = Arc::new(RuntimeSettings::new());
        let orch = test_orchestrator(mem, settings);

        let tc = make_tool_call("web_search", r#"{"search_prompt":""}"#);
        let mut state = CtxState::new();
        let result = orch
            .handle_tool_web(
                &tc,
                "web_search",
                r#"{"search_prompt":""}"#,
                &mut state.ctx(),
            )
            .await
            .expect("should not error");

        assert_eq!(result, Some(true));
        let content = format!("{:?}", state.messages.last().unwrap());
        assert!(
            content.contains("required") || content.contains("empty"),
            "must report empty prompt error, got: {content}"
        );
    }

    #[tokio::test]
    async fn web_search_bad_json_returns_error() {
        let mem = MemoryHandle::spawn_in_memory().expect("in-memory DB");
        let settings = Arc::new(RuntimeSettings::new());
        let orch = test_orchestrator(mem, settings);

        let tc = make_tool_call("web_search", "{{BAD");
        let mut state = CtxState::new();
        let result = orch
            .handle_tool_web(&tc, "web_search", "{{BAD", &mut state.ctx())
            .await
            .expect("should not error");

        assert_eq!(result, Some(true));
        let content = format!("{:?}", state.messages.last().unwrap());
        assert!(content.contains("JSON") || content.contains("parse") || content.contains("error"));
    }

    #[tokio::test]
    async fn macro_tool_result_feeds_exfil_guard_input() {
        // #9 security regression: macro-dispatched tools (web_search/recall_memory/
        // google/…) must push their result into `previous_tool_results` — the input the
        // exfiltration guard scans — not only into `messages`. Without the macro fix a
        // later network (MCP/WASM) tool could exfiltrate macro-surfaced data unseen.
        // Bad JSON gives a deterministic result (the parse-error string) with no network.
        let mem = MemoryHandle::spawn_in_memory().expect("in-memory DB");
        let settings = Arc::new(RuntimeSettings::new());
        let orch = test_orchestrator(mem, settings);

        let tc = make_tool_call("web_search", "{{BAD");
        let mut state = CtxState::new();
        assert!(
            state.prev_results.is_empty(),
            "precondition: no prior results"
        );
        orch.handle_tool_web(&tc, "web_search", "{{BAD", &mut state.ctx())
            .await
            .expect("should not error");

        // The macro tool's result must now be visible to the exfiltration guard.
        // (Without the fix the macro pushed only to `messages`, leaving this empty.)
        assert_eq!(
            state.prev_results.len(),
            1,
            "macro-dispatched tool result must enter previous_tool_results (exfil-guard input)"
        );
    }

    // ── generate_image bad JSON ──────────────────────────────────────────────

    #[tokio::test]
    async fn generate_image_bad_json_returns_error() {
        let mem = MemoryHandle::spawn_in_memory().expect("in-memory DB");
        let settings = Arc::new(RuntimeSettings::new());
        let orch = test_orchestrator(mem, settings);

        let tc = make_tool_call("generate_image", "NOT JSON");
        let mut state = CtxState::new();
        let result = orch
            .handle_tool_multimedia(&tc, "generate_image", "NOT JSON", &mut state.ctx())
            .await
            .expect("should not error");

        assert_eq!(result, Some(true));
        let content = format!("{:?}", state.messages.last().unwrap());
        assert!(content.contains("JSON") || content.contains("parse") || content.contains("error"));
    }
}
