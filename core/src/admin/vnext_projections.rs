use super::*;
use crate::i18n::{t, tfp};

fn projection_now(generated_at: &str) -> chrono::DateTime<chrono::Utc> {
    parse_work_timestamp(generated_at).unwrap_or_else(chrono::Utc::now)
}

fn vnext_projected_work_item(
    item: &UnifiedInboxItem,
    locale: &str,
    now: chrono::DateTime<chrono::Utc>,
) -> VNextWorkItem {
    let mut work = vnext_work_item(item, locale);
    work.lane = vnext_lane_for_item_at(item, now).to_string();
    work
}

pub(super) fn vnext_home_projection(
    projection: &UnifiedInboxProjection,
    locale: &str,
) -> VNextHomeProjection {
    let now = projection_now(&projection.generated_at);
    let denoised = denoise_work_inbox_items(projection.items.clone(), now, locale);
    let items: Vec<VNextWorkItem> = denoised
        .iter()
        .map(|item| vnext_projected_work_item(item, locale, now))
        .collect();
    let top_now: Vec<VNextWorkItem> = items
        .iter()
        .filter(|item| item.lane == "now")
        .take(3)
        .cloned()
        .collect();
    let decision_previews: Vec<VNextWorkItem> = items
        .iter()
        .filter(|item| item.lane == "decide")
        .take(3)
        .cloned()
        .collect();
    let follow_up_highlights: Vec<VNextWorkItem> = denoised
        .iter()
        .filter(|item| vnext_lane_for_item_at(item, now) == "follow_up")
        .filter(|item| !is_grouped_receipt_item(item))
        .take(3)
        .map(|item| vnext_projected_work_item(item, locale, now))
        .collect();
    let lead = decision_previews.first().or_else(|| top_now.first());
    let source_attention = projection
        .sources
        .iter()
        .find(|source| vnext_source_needs_attention(&source.status));
    let brief = if let Some(item) = lead {
        vnext_home_brief_for_item(item, locale)
    } else if let Some(source) = source_attention {
        vnext_source_attention_summary(source, locale)
    } else {
        t("console.home.brief.quiet", locale).to_string()
    };

    VNextHomeProjection {
        schema_version: 1,
        generated_at: projection.generated_at.clone(),
        brief,
        top_now,
        decision_previews,
        follow_up_highlights,
        status_entry: vnext_status_entry(projection, locale),
    }
}

pub(super) fn vnext_home_brief_for_item(item: &VNextWorkItem, locale: &str) -> String {
    let title = owner_visible_title(&item.title, t("console.home.fallback_item", locale), 96);
    if item.state == "blocked_by_gate" {
        return tfp("console.home.brief.attention", locale, &[&title]);
    }
    let key = match item.lane.as_str() {
        "decide" => "console.home.brief.decide",
        "now" if item.state == "scheduled" => "console.home.brief.coming_up",
        "now" => "console.home.brief.ready",
        "follow_up" => "console.home.brief.follow_up",
        "backlog" => "console.home.brief.backlog",
        _ => "console.home.brief.start",
    };
    tfp(key, locale, &[&title])
}

pub(super) fn vnext_work_projection(
    projection: &UnifiedInboxProjection,
    locale: &str,
) -> VNextWorkProjection {
    const DECIDE_LIMIT: usize = 12;
    const NOW_LIMIT: usize = 12;
    const FOLLOW_UP_LIMIT: usize = 12;
    const BACKLOG_LIMIT: usize = 6;
    const DONE_LIMIT: usize = 12;

    let now_ts = projection_now(&projection.generated_at);
    let denoised = denoise_work_inbox_items(projection.items.clone(), now_ts, locale);
    let mut now = Vec::new();
    let mut decide = Vec::new();
    let mut follow_up = Vec::new();
    let mut backlog = Vec::new();
    let mut done = Vec::new();

    for item in &denoised {
        let work = vnext_projected_work_item(item, locale, now_ts);
        match work.lane.as_str() {
            "decide" => decide.push(work),
            "follow_up" => follow_up.push(work),
            "backlog" => backlog.push(work),
            "done" => {
                if follow_up_is_stale(item, now_ts) {
                    done.push(work);
                }
            }
            _ => now.push(work),
        }
    }

    VNextWorkProjection {
        schema_version: 1,
        generated_at: projection.generated_at.clone(),
        sources: projection.sources.clone(),
        counts: projection.counts.clone(),
        lanes: vec![
            VNextWorkLane {
                id: "now".into(),
                title: t("console.work.lane.now", locale).into(),
                collapsed: now.is_empty(),
                items: cap_vnext_lane(now, NOW_LIMIT),
            },
            VNextWorkLane {
                id: "decide".into(),
                title: t("console.work.lane.decide", locale).into(),
                collapsed: decide.is_empty(),
                items: cap_vnext_lane(decide, DECIDE_LIMIT),
            },
            VNextWorkLane {
                id: "follow_up".into(),
                title: t("console.work.lane.follow_up", locale).into(),
                collapsed: follow_up.is_empty(),
                items: cap_vnext_lane(follow_up, FOLLOW_UP_LIMIT),
            },
            VNextWorkLane {
                id: "backlog".into(),
                title: t("console.work.lane.backlog", locale).into(),
                collapsed: true,
                items: cap_vnext_lane(backlog, BACKLOG_LIMIT),
            },
            VNextWorkLane {
                id: "done".into(),
                title: t("console.work.lane.done", locale).into(),
                collapsed: true,
                items: cap_vnext_lane(done, DONE_LIMIT),
            },
        ],
    }
}

fn cap_vnext_lane(mut items: Vec<VNextWorkItem>, limit: usize) -> Vec<VNextWorkItem> {
    if items.len() > limit {
        items.truncate(limit);
    }
    items
}

pub(super) fn vnext_item_projection(
    item: &UnifiedInboxItem,
    generated_at: &str,
    locale: &str,
) -> VNextItemProjection {
    let now = projection_now(generated_at);
    VNextItemProjection {
        schema_version: 1,
        generated_at: generated_at.to_string(),
        item: vnext_projected_work_item(item, locale, now),
        safe_source_detail: item.summary.clone(),
        history: item
            .related_receipt_ids
            .iter()
            .map(|id| VNextReceiptRef {
                id: id.clone(),
                kind: "receipt".into(),
                label: t("console.item.receipt.record", locale).into(),
                created_at: item.updated_at.clone(),
            })
            .collect(),
        collaboration: VNextCollaborationEntry {
            focus_id: item.id.clone(),
            placeholder: t("console.collab.placeholder", locale).into(),
            suggested_prompts: vnext_suggested_prompts(item, locale),
        },
    }
}

/// `before_unix`: only messages strictly older than this timestamp (the
/// oldest one the console already shows); `None` = the newest window.
#[cfg(test)]
pub(super) async fn vnext_chat_history_projection(
    memory: &MemoryHandle,
    user_did: &str,
    before_unix: Option<i64>,
) -> VNextChatHistoryProjection {
    vnext_chat_history_projection_with_inflight(
        memory,
        user_did,
        before_unix,
        None,
        &HashSet::new(),
        crate::i18n::DEFAULT_LOCALE,
    )
    .await
}

pub(super) async fn vnext_chat_history_projection_with_inflight(
    memory: &MemoryHandle,
    user_did: &str,
    before_unix: Option<i64>,
    focus_turn: Option<&str>,
    inflight_turn_ids: &HashSet<String>,
    locale: &str,
) -> VNextChatHistoryProjection {
    let mut evidence_by_key = HashMap::new();
    for record in memory
        .list_recent_turn_evidence(user_did, 200)
        .await
        .unwrap_or_default()
    {
        evidence_by_key
            .entry(record.reply_key.clone())
            .or_insert(record);
    }

    let anchor_index = VNextChatHistoryAnchorIndex::from_events(
        memory
            .list_recent_assistant_events(user_did, WEBAPP_VNEXT_CHAT_HISTORY_ARCHIVE_SCAN_LIMIT)
            .await
            .unwrap_or_default(),
    );

    let archive_rows = memory
        .list_recent_l3(user_did, WEBAPP_VNEXT_CHAT_HISTORY_ARCHIVE_SCAN_LIMIT)
        .await
        .unwrap_or_default();
    let l1_rows = memory.get_l1(user_did).await.unwrap_or_default();
    let mut gate_ack_turn_ids = HashSet::new();
    for message in &l1_rows {
        if message.role.trim() == "assistant"
            && vnext_gate_pending_owner_ack(&message.content)
            && let Some(turn_id) = webapp_turn_anchor(
                message.anchor_kind.as_deref(),
                message.anchor_ref.as_deref(),
            )
        {
            gate_ack_turn_ids.insert(turn_id);
        }
    }
    for entry in &archive_rows {
        if entry.role.trim() == "assistant"
            && vnext_gate_pending_owner_ack(&entry.content)
            && let Some(turn_id) =
                webapp_turn_anchor(entry.anchor_kind.as_deref(), entry.anchor_ref.as_deref())
        {
            gate_ack_turn_ids.insert(turn_id);
        }
    }
    let smoke_reply_keys = vnext_smoke_reply_keys(
        l1_rows
            .iter()
            .map(|message| {
                (
                    message.role.as_str(),
                    message.content.as_str(),
                    message.timestamp,
                )
            })
            .chain(archive_rows.iter().map(|entry| {
                (
                    entry.role.as_str(),
                    entry.content.as_str(),
                    parse_vnext_chat_history_timestamp(&entry.created_at),
                )
            })),
    );
    let archive_messages = archive_rows.into_iter().filter_map(|entry| {
        // L3 rows written after the 2026-06-30 migration carry anchor_kind/anchor_ref
        // and are filtered structurally. Pre-migration rows (anchor_kind=NULL) have no
        // structural identity and are left visible — text-based suppression would violate
        // the structural-only filtering constraint and risk hiding legitimate chat turns.
        vnext_chat_history_message_from_parts(
            &entry.role,
            &entry.content,
            parse_vnext_chat_history_timestamp(&entry.created_at),
            entry.anchor_kind.as_deref(),
            entry.anchor_ref.as_deref(),
            entry.latency_ms,
            &anchor_index,
        )
    });

    let l1_messages = l1_rows.into_iter().filter_map(|message| {
        vnext_chat_history_message_from_parts(
            &message.role,
            &message.content,
            message.timestamp,
            message.anchor_kind.as_deref(),
            message.anchor_ref.as_deref(),
            message.latency_ms,
            &anchor_index,
        )
    });

    let mut seen = HashSet::new();
    let mut indexed_messages = l1_messages
        .chain(archive_messages)
        .filter(|message| {
            message.role != "aegis"
                || !smoke_reply_keys.contains(&(message.timestamp, message.text.trim().to_string()))
        })
        .filter(|message| seen.insert(vnext_chat_history_turn_key(message)))
        .enumerate()
        .collect::<Vec<_>>();
    indexed_messages.sort_by(|left, right| {
        left.1
            .timestamp
            .cmp(&right.1.timestamp)
            .then_with(|| left.0.cmp(&right.0))
    });
    let mut messages = vnext_collapse_repeated_alerts(
        indexed_messages
            .into_iter()
            .map(|(_, message)| message)
            .collect::<Vec<_>>(),
    );
    if let Some(before) = before_unix {
        messages.retain(|message| message.timestamp < before);
    }
    let (mut messages, has_more) = select_chat_history_window(messages, focus_turn);
    for message in &mut messages {
        if message.role == "aegis" {
            // Resolve before display clipping. New turns use their identity,
            // never another turn's evidence merely because the text is equal.
            let key = message
                .turn_id
                .as_deref()
                .map(crate::evidence::turn_key)
                .unwrap_or_else(|| crate::evidence::reply_key(&message.text));
            if let Some(record) = evidence_by_key.get(&key) {
                message.text =
                    crate::evidence::strip_unbacked_refs(&message.text, &record.evidence);
                message.evidence = (!record.evidence.is_empty()).then(|| record.evidence.clone());
                message.unsupported_numbers = (!record.unsupported_numbers.is_empty())
                    .then(|| record.unsupported_numbers.clone());
            }
        }
        message.text = message.text.chars().take(4_000).collect();
    }
    vnext_attach_unanswered_replies(&mut messages, inflight_turn_ids, &gate_ack_turn_ids, locale);
    vnext_attach_failure_retry(&mut messages);
    let approvals = owner_chat_approval_cards(memory, user_did).await;
    VNextChatHistoryProjection {
        schema_version: 1,
        messages,
        has_more,
        approvals,
    }
}

const HITL_RESOLVED_KEEP_SECS: i64 = 7 * 24 * 60 * 60;

/// Pending tool holds collapse to one card per owner-visible identity.
/// A resolved hold stays on the thread so refresh still shows the decision.
async fn owner_chat_approval_cards(
    memory: &MemoryHandle,
    user_did: &str,
) -> Vec<VNextChatApprovalCard> {
    let now = chrono::Utc::now().timestamp();
    let pending = memory.list_hitl_pending().await.unwrap_or_default();
    let mut pending_by_identity: HashMap<String, VNextChatApprovalCard> = HashMap::new();
    for (key, raw) in pending {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) else {
            continue;
        };
        if !approval_json_belongs_to_user(&value, user_did) {
            continue;
        }
        let Some(tool_name) = value
            .get("tool_name")
            .and_then(|tool| tool.as_str())
            .map(str::trim)
            .filter(|tool| !tool.is_empty())
        else {
            continue;
        };
        let tool_args = value
            .get("tool_args")
            .and_then(|args| args.as_str())
            .unwrap_or("");
        let stored_summary = hold_summary_text(&value);
        let identity =
            crate::egress_gate::owner_hold_identity(tool_name, tool_args, &stored_summary);
        let approval_id = value
            .get("approval_id")
            .and_then(|id| id.as_str())
            .filter(|id| !id.is_empty())
            .unwrap_or(key.as_str())
            .to_string();
        let card = VNextChatApprovalCard {
            approval_id,
            tool_name: tool_name.chars().take(120).collect(),
            summary: owner_visible_hold_summary(&stored_summary),
            state: "pending".into(),
            timestamp: json_unix_timestamp(&value, "created_at").unwrap_or(now),
            outcome: None,
        };
        match pending_by_identity.get(&identity) {
            Some(existing) if existing.timestamp <= card.timestamp => {}
            _ => {
                pending_by_identity.insert(identity, card);
            }
        }
    }

    let pending_identities: HashSet<String> = pending_by_identity.keys().cloned().collect();
    let mut cards: Vec<VNextChatApprovalCard> = pending_by_identity.into_values().collect();
    let resolved = memory
        .list_skill_state_entries(crate::egress_gate::HITL_RESOLVED_SKILL)
        .await
        .unwrap_or_default();
    for (_key, raw) in resolved {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) else {
            continue;
        };
        if !approval_json_belongs_to_user(&value, user_did) {
            continue;
        }
        let state = value
            .get("state")
            .and_then(|item| item.as_str())
            .unwrap_or("");
        if state != "approved" && state != "rejected" {
            continue;
        }
        let Some(resolved_at) = json_unix_timestamp(&value, "resolved_at") else {
            continue;
        };
        if now.saturating_sub(resolved_at) > HITL_RESOLVED_KEEP_SECS {
            continue;
        }
        let Some(tool_name) = value
            .get("tool_name")
            .and_then(|tool| tool.as_str())
            .map(str::trim)
            .filter(|tool| !tool.is_empty())
        else {
            continue;
        };
        let tool_args = value
            .get("tool_args")
            .and_then(|args| args.as_str())
            .unwrap_or("");
        let stored_summary = hold_summary_text(&value);
        let identity =
            crate::egress_gate::owner_hold_identity(tool_name, tool_args, &stored_summary);
        if pending_identities.contains(&identity) {
            continue;
        }
        let Some(approval_id) = value
            .get("approval_id")
            .and_then(|id| id.as_str())
            .filter(|id| !id.is_empty())
        else {
            continue;
        };
        if cards.iter().any(|card| card.approval_id == approval_id) {
            continue;
        }
        cards.push(VNextChatApprovalCard {
            approval_id: approval_id.to_string(),
            tool_name: tool_name.chars().take(120).collect(),
            summary: owner_visible_hold_summary(&stored_summary),
            state: state.to_string(),
            timestamp: resolved_at,
            outcome: hold_outcome_text(&value),
        });
    }
    cards.sort_by(|left, right| {
        left.timestamp
            .cmp(&right.timestamp)
            .then_with(|| left.approval_id.cmp(&right.approval_id))
    });
    cards.truncate(24);
    cards
}

fn hold_summary_text(value: &serde_json::Value) -> String {
    value
        .get("approval_summary")
        .and_then(|item| item.as_str())
        .or_else(|| value.get("summary").and_then(|item| item.as_str()))
        .unwrap_or("")
        .to_string()
}

fn owner_visible_hold_summary(summary: &str) -> String {
    if crate::egress_gate::parse_card_summary(summary).is_some() {
        summary.chars().take(2_000).collect()
    } else {
        String::new()
    }
}

fn json_unix_timestamp(value: &serde_json::Value, field: &str) -> Option<i64> {
    let text = value.get(field).and_then(|item| item.as_str())?;
    chrono::DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|stamp| stamp.timestamp())
}

fn approval_json_belongs_to_user(value: &serde_json::Value, user_did: &str) -> bool {
    let stored_did = value
        .get("user_did")
        .and_then(|item| item.as_str())
        .unwrap_or("");
    let requested = value
        .get("requested_by")
        .and_then(|item| item.as_str())
        .unwrap_or("");
    if !stored_did.is_empty() && stored_did != user_did {
        return false;
    }
    if !requested.is_empty() && requested != user_did {
        return false;
    }
    if stored_did.is_empty() && requested.is_empty() {
        let Some(chat_id) = value.get("chat_id").and_then(|item| item.as_i64()) else {
            return false;
        };
        return user_did.ends_with(&format!(":{chat_id}"));
    }
    true
}

/// An owner web turn whose assistant row never landed (the model failed after
/// the user line was saved, the process stopped, or the saved reply was blank)
/// still has a cite and no answer. Pair it with the same sentence the failure
/// path writes, unless that turn is still running.
fn hold_outcome_text(value: &serde_json::Value) -> Option<String> {
    let text = value
        .get("outcome")
        .and_then(|item| item.as_str())
        .unwrap_or("")
        .trim();
    if text.is_empty() {
        None
    } else {
        Some(text.chars().take(2_000).collect())
    }
}

fn webapp_turn_anchor(kind: Option<&str>, anchor_ref: Option<&str>) -> Option<String> {
    if kind != Some(crate::progress_router::WEBAPP_TURN_ANCHOR_KIND) {
        return None;
    }
    anchor_ref
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn vnext_attach_unanswered_replies(
    messages: &mut Vec<VNextChatHistoryMessage>,
    inflight_turn_ids: &HashSet<String>,
    gate_ack_turn_ids: &HashSet<String>,
    locale: &str,
) {
    let reply_text = t("assistant.turn_unanswered", locale).to_string();
    let mut inserts: Vec<usize> = Vec::new();
    for (index, message) in messages.iter().enumerate() {
        if message.role != "owner" {
            continue;
        }
        let Some(turn_id) = message.turn_id.as_deref() else {
            continue;
        };
        if turn_id.is_empty()
            || inflight_turn_ids.contains(turn_id)
            || gate_ack_turn_ids.contains(turn_id)
        {
            continue;
        }
        let next_owner = messages
            .iter()
            .enumerate()
            .skip(index + 1)
            .find(|(_, candidate)| candidate.role == "owner")
            .map(|(next, _)| next);
        let end = next_owner.unwrap_or(messages.len());
        let followed_by_reply = messages[index + 1..end]
            .iter()
            .any(|candidate| candidate.role == "aegis");
        let paired = messages.iter().any(|candidate| {
            candidate.role == "aegis" && candidate.turn_id.as_deref() == Some(turn_id)
        });
        if !followed_by_reply && !paired {
            inserts.push(index);
        }
    }
    for index in inserts.into_iter().rev() {
        let turn_id = messages[index].turn_id.clone();
        let timestamp = messages[index].timestamp;
        messages.insert(
            index + 1,
            VNextChatHistoryMessage {
                role: "aegis".to_string(),
                turn_id,
                text: reply_text.clone(),
                timestamp,
                severity: "warning".to_string(),
                latency_ms: None,
                repeat_count: None,
                first_timestamp: None,
                alert_group: None,
                work_item_id: None,
                evidence: None,
                unsupported_numbers: None,
                retry_text: None,
            },
        );
    }
}

/// The failure hint is the only history row that should offer 重试. Match both
/// display languages: a row stored in one locale stays that sentence after the
/// owner switches language, and the button still belongs next to it.
fn vnext_failure_hint(text: &str) -> bool {
    const KEYS: [&str; 2] = ["assistant.turn_unanswered", "assistant.empty_response"];
    KEYS.iter().any(|key| {
        ["zh", "en"].iter().any(|locale| {
            let hint = t(key, locale);
            if text == hint {
                return true;
            }
            // The stored failure continues the same sentence with the reason.
            let Some(rest) = text.strip_prefix(hint) else {
                return false;
            };
            let rest = rest.trim_start();
            rest.starts_with("原因：") || rest.starts_with("Reason:") || rest.starts_with("⚠️ ")
        })
    })
}

fn vnext_attach_failure_retry(messages: &mut [VNextChatHistoryMessage]) {
    let mut owner_text: Option<String> = None;
    for message in messages.iter_mut() {
        if message.role == "owner" {
            let trimmed = message.text.trim();
            owner_text = (!trimmed.is_empty()).then(|| trimmed.to_string());
            continue;
        }
        if message.role == "aegis" && vnext_failure_hint(&message.text) {
            message.retry_text = owner_text.clone();
        }
    }
}

/// Newest `WEBAPP_VNEXT_CHAT_HISTORY_LIMIT` messages, or the window that
/// contains `focus_turn` when that turn is in the scanned list.
///
/// A missing turn keeps the newest window. `has_more` means older rows exist
/// before the returned slice. The window may grow past the limit (up to 64)
/// so a same-second group is not split.
pub(super) fn select_chat_history_window(
    mut messages: Vec<VNextChatHistoryMessage>,
    focus_turn: Option<&str>,
) -> (Vec<VNextChatHistoryMessage>, bool) {
    let focus = focus_turn.map(str::trim).filter(|value| !value.is_empty());
    if let Some(turn) = focus
        && let Some(last_idx) = messages
            .iter()
            .rposition(|message| message.turn_id.as_deref() == Some(turn))
    {
        let first_idx = messages
            .iter()
            .position(|message| message.turn_id.as_deref() == Some(turn))
            .unwrap_or(last_idx);
        let span = last_idx - first_idx + 1;
        let limit = WEBAPP_VNEXT_CHAT_HISTORY_LIMIT.max(span).min(64);
        let after = (messages.len() - last_idx - 1).min(2);
        let mut end = last_idx + 1 + after;
        if end - first_idx > limit {
            end = first_idx + limit;
        }
        let mut start = end.saturating_sub(limit).min(first_idx);
        while start > 0
            && end - start < 64
            && messages[start - 1].timestamp == messages[start].timestamp
        {
            start -= 1;
        }
        let has_more = start > 0;
        messages = messages.split_off(start);
        messages.truncate(end - start);
        return (messages, has_more);
    }
    // Never cut inside a same-second group: the console pages with
    // `before = oldest shown timestamp` (strict <), so a group split across
    // the boundary would lose its unseen half forever. The window may exceed
    // the limit by the size of that group (the guard allows up to 64).
    let mut cut = messages
        .len()
        .saturating_sub(WEBAPP_VNEXT_CHAT_HISTORY_LIMIT);
    while cut > 0 && messages[cut - 1].timestamp == messages[cut].timestamp {
        cut -= 1;
    }
    let has_more = cut > 0;
    if has_more {
        messages = messages.split_off(cut);
    }
    (messages, has_more)
}

fn vnext_chat_history_message_from_parts(
    raw_role: &str,
    raw_text: &str,
    timestamp: i64,
    anchor_kind: Option<&str>,
    anchor_ref: Option<&str>,
    latency_ms: Option<u64>,
    anchor_index: &VNextChatHistoryAnchorIndex,
) -> Option<VNextChatHistoryMessage> {
    let role = raw_role.trim();
    if role != "user" && role != "assistant" {
        return None;
    }
    let text = raw_text.trim();
    if text.is_empty() {
        return None;
    }
    // Historical filter: routine inbox notifications (Calendar non-urgent)
    // are hidden from the chat-history projection. They remain in L1 for audit
    // but are already visible in the Mail view as first-class items.
    // Filter is structural (anchor_kind + anchor_ref → event ID lookup) —
    // never text-based.
    if anchor_index.is_routine_assistant_event_anchor(anchor_kind, anchor_ref) {
        return None;
    }
    if vnext_low_value_system_chat_notice(role, text) {
        return None;
    }
    Some(VNextChatHistoryMessage {
        role: if role == "user" { "owner" } else { "aegis" }.to_string(),
        turn_id: (anchor_kind == Some(crate::progress_router::WEBAPP_TURN_ANCHOR_KIND))
            .then_some(anchor_ref)
            .flatten()
            .filter(|value| !value.trim().is_empty())
            .map(str::to_string),
        text: text.to_string(),
        timestamp,
        // Historical L1/archive messages carry no structured severity signal,
        // so the chat-history badge defaults to "info". We deliberately do NOT
        // scan the reply text for keywords (that anti-pattern badged routine
        // reminder confirmations as IMPORTANT). Live responses still get their
        // severity from structured signals in contextual_assistant_severity().
        severity: "info".to_string(),
        latency_ms: if role == "assistant" {
            latency_ms
        } else {
            None
        },
        repeat_count: None,
        first_timestamp: None,
        alert_group: match (role, anchor_kind, anchor_ref) {
            ("assistant", Some(crate::telegram::HEARTBEAT_ALERT_ANCHOR_KIND), Some(skill)) => {
                Some(format!(
                    "{}:{}",
                    crate::telegram::HEARTBEAT_ALERT_ANCHOR_KIND,
                    skill.trim()
                ))
            }
            _ => None,
        },
        work_item_id: vnext_chat_history_work_item_id(anchor_kind, anchor_ref, anchor_index),
        evidence: None,
        unsupported_numbers: None,
        retry_text: None,
    })
}

/// Collapse runs of consecutive alerts from the same heartbeat skill into the
/// newest copy, carrying how many there were and when the run started. Only
/// structurally anchored rows (`heartbeat_alert`) take part; an owner turn or
/// any other message between two alerts ends the run.
pub(super) fn vnext_collapse_repeated_alerts(
    messages: Vec<VNextChatHistoryMessage>,
) -> Vec<VNextChatHistoryMessage> {
    let mut out: Vec<VNextChatHistoryMessage> = Vec::with_capacity(messages.len());
    for message in messages {
        if let (Some(group), Some(last)) = (message.alert_group.as_deref(), out.last_mut())
            && last.alert_group.as_deref() == Some(group)
        {
            let first_timestamp = last.first_timestamp.unwrap_or(last.timestamp);
            let repeat_count = last.repeat_count.unwrap_or(1) + 1;
            let mut latest = message;
            latest.first_timestamp = Some(first_timestamp.min(latest.timestamp));
            latest.repeat_count = Some(repeat_count);
            *last = latest;
            continue;
        }
        out.push(message);
    }
    out
}

/// Keys `(timestamp, text)` of assistant replies that directly answer a hidden
/// deploy-smoke turn (`部署冒烟：…`). The smoke question is already dropped by
/// `vnext_low_value_system_chat_notice`; without this its reply ("Example
/// Domain", the calendar one-liner) stayed behind as an orphan bubble in the
/// owner's conversation (owner walk 2026-09-12). Rows are examined in
/// chronological order regardless of the input order.
fn vnext_smoke_reply_keys<'a>(
    rows: impl Iterator<Item = (&'a str, &'a str, i64)>,
) -> HashSet<(i64, String)> {
    let mut ordered: Vec<(&str, &str, i64)> = rows.collect();
    ordered.sort_by_key(|(_, _, timestamp)| *timestamp);
    let mut keys = HashSet::new();
    let mut pending_smoke_reply = false;
    for (role, text, timestamp) in ordered {
        let role = role.trim();
        if role == "user" {
            pending_smoke_reply = vnext_low_value_system_chat_notice("user", text);
        } else if role == "assistant" && pending_smoke_reply {
            keys.insert((timestamp, text.trim().to_string()));
            pending_smoke_reply = false;
        }
    }
    keys
}

struct VNextChatHistoryAnchorIndex {
    by_id: HashMap<i64, String>,
    by_dedup_key: HashMap<String, String>,
    /// IDs of events that `assistant_event_enters_conversation` classified as
    /// routine (Calendar non-urgent inbox) — L1 messages anchored to
    /// these events are filtered out of the chat-history projection.
    routine_event_ids: HashSet<i64>,
    /// Dedup keys of routine events, for anchor_ref values that store a dedup key
    /// instead of a numeric `assistant_event:<id>` reference.
    routine_event_dedup_keys: HashSet<String>,
}

impl VNextChatHistoryAnchorIndex {
    fn from_events(events: Vec<crate::assistant_events::StoredAssistantEvent>) -> Self {
        let mut by_id = HashMap::new();
        let mut by_dedup_key = HashMap::new();
        let mut routine_event_ids = HashSet::new();
        let mut routine_event_dedup_keys = HashSet::new();
        for event in &events {
            if !crate::assistant_events::assistant_event_enters_conversation(&event.event) {
                routine_event_ids.insert(event.id);
                routine_event_dedup_keys.insert(event.event.dedup_key.clone());
            }
        }
        for event in events {
            let Some(work_item_id) = vnext_chat_history_assistant_event_work_item_id(&event) else {
                continue;
            };
            by_id.insert(event.id, work_item_id.clone());
            by_dedup_key.insert(event.event.dedup_key.clone(), work_item_id);
        }
        Self {
            by_id,
            by_dedup_key,
            routine_event_ids,
            routine_event_dedup_keys,
        }
    }

    /// Returns `true` if the given anchor resolves to a routine inbox event
    /// (one that `assistant_event_enters_conversation` classified as non-conversation).
    /// L1 messages with such anchors should be hidden from the chat projection.
    fn is_routine_assistant_event_anchor(
        &self,
        anchor_kind: Option<&str>,
        anchor_ref: Option<&str>,
    ) -> bool {
        if anchor_kind.map(str::trim) != Some("assistant_event") {
            return false;
        }
        let value = match anchor_ref.map(str::trim) {
            Some(v) if !v.is_empty() => v,
            _ => return false,
        };
        // Try numeric-ID form: "assistant_event:<id>"
        if let Some(event_id) = value
            .strip_prefix("assistant_event:")
            .and_then(|raw| raw.parse::<i64>().ok())
        {
            return self.routine_event_ids.contains(&event_id);
        }
        // Fall back to dedup-key form (for anchor_refs stored as the raw dedup key).
        self.routine_event_dedup_keys.contains(value)
    }
}

fn vnext_chat_history_work_item_id(
    anchor_kind: Option<&str>,
    anchor_ref: Option<&str>,
    anchor_index: &VNextChatHistoryAnchorIndex,
) -> Option<String> {
    if anchor_kind?.trim() != "assistant_event" {
        return None;
    }
    let value = anchor_ref?.trim();
    if value.is_empty() {
        return None;
    }
    if let Some(id) = value
        .strip_prefix("assistant_event:")
        .and_then(|raw| raw.parse::<i64>().ok())
    {
        return anchor_index.by_id.get(&id).cloned();
    }
    anchor_index.by_dedup_key.get(value).cloned()
}

fn vnext_chat_history_assistant_event_work_item_id(
    event: &crate::assistant_events::StoredAssistantEvent,
) -> Option<String> {
    match (event.event.source, event.event.kind) {
        (
            crate::assistant_events::AssistantEventSource::Calendar,
            crate::assistant_events::AssistantEventKind::CalendarInviteReceived
            | crate::assistant_events::AssistantEventKind::CalendarEventChanged,
        ) => Some(format!("calendar:{}", event.id)),
        _ => None,
    }
}

fn parse_vnext_chat_history_timestamp(created_at: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(created_at)
        .map(|value| value.timestamp())
        .unwrap_or(0)
}

fn vnext_chat_history_turn_key(message: &VNextChatHistoryMessage) -> String {
    if let Some(turn_id) = &message.turn_id {
        return format!("{}\nturn:{turn_id}", message.role);
    }
    format!(
        "{}\n{}\n{}",
        message.role,
        message
            .text
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" "),
        // Exact second: L1 and L3 keep the same timestamp for one message,
        // while a 5-minute bucket swallowed a genuine second "好" minutes later.
        message.timestamp
    )
}

/// The fixed "waiting for approval" sentence. It is the turn's stored reply
/// when a tool hold is pending, and the chat card is the decision. A longer
/// answer that merely mentions 批准 is not this sentence.
pub(super) fn vnext_gate_pending_owner_ack(text: &str) -> bool {
    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized == "等待你批准这次搜索。" || normalized == "等待你批准这次搜索"
    {
        return true;
    }
    let body = normalized.trim_start_matches('⏳').trim();
    body.contains('·')
        && body.contains("等待你批准")
        && body.contains("awaiting your approval")
        && body.chars().count() <= 180
}

pub(super) fn vnext_low_value_system_chat_notice(role: &str, text: &str) -> bool {
    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    // Deploy smoke turns (scripts/smoke-web-turn.py) are real owner turns by
    // design, but they are noise in the owner's conversation view.
    if role == "user" && normalized.starts_with("部署冒烟：") {
        return true;
    }
    if role == "assistant" && normalized == "冒烟正常" {
        return true;
    }
    if role != "assistant" {
        return false;
    }
    if vnext_gate_pending_owner_ack(&normalized) {
        return true;
    }
    let lower = normalized.to_ascii_lowercase();
    if normalized.contains("系统已重启") || lower.contains("system restarted") {
        return true;
    }
    if lower.starts_with("system integrity report")
        && lower.contains("invariants:")
        && lower.contains("rate monitors:")
    {
        return true;
    }
    lower.starts_with("daily brief") && lower.contains("calendar not queried")
}

#[cfg(test)]
pub(super) fn vnext_status_projection(
    projection: &UnifiedInboxProjection,
    locale: &str,
) -> VNextStatusProjection {
    vnext_status_projection_with_settings_and_events(
        projection,
        None,
        Vec::new(),
        None,
        None,
        locale,
    )
}

#[cfg(test)]
pub(super) fn vnext_status_projection_with_settings(
    projection: &UnifiedInboxProjection,
    settings: Option<&RuntimeSettings>,
    locale: &str,
) -> VNextStatusProjection {
    vnext_status_projection_with_settings_and_events(
        projection,
        settings,
        Vec::new(),
        None,
        None,
        locale,
    )
}

pub(super) fn vnext_status_projection_with_settings_and_events(
    projection: &UnifiedInboxProjection,
    settings: Option<&RuntimeSettings>,
    system_events: Vec<VNextSystemEvent>,
    llm_degraded: Option<crate::orchestrator::LlmDegraded>,
    embedder_degraded: Option<crate::orchestrator::EmbedderDegraded>,
    locale: &str,
) -> VNextStatusProjection {
    let now_ts = projection_now(&projection.generated_at);
    let denoised = denoise_work_inbox_items(projection.items.clone(), now_ts, locale);
    let active_items = denoised
        .iter()
        .filter(|item| vnext_lane_for_item_at(item, now_ts) != "done")
        .count();
    let decision_items = denoised
        .iter()
        .filter(|item| vnext_lane_for_item_at(item, now_ts) == "decide")
        .count();
    let follow_up_items = denoised
        .iter()
        .filter(|item| vnext_lane_for_item_at(item, now_ts) == "follow_up")
        .count();
    let mut sections = vec![
        VNextStatusSummary {
            label: "Background Review".into(), // identity: the WebApp localizes section labels
            health: if active_items > 0 { "ready" } else { "quiet" }.into(),
            owner_message: if active_items > 0 {
                t("console.status.background.open", locale).into()
            } else {
                t("console.status.background.quiet", locale).into()
            },
            owner_next_step: if active_items > 0 {
                Some(t("console.status.background.next", locale).into())
            } else {
                None
            },
        },
        VNextStatusSummary {
            label: "Recommendations".into(), // identity: the WebApp localizes section labels
            health: if decision_items > 0 {
                "attention"
            } else {
                "quiet"
            }
            .into(),
            owner_message: if decision_items > 0 {
                tfp(
                    "console.status.recommendations.need_choice",
                    locale,
                    &[&decision_items.to_string()],
                )
            } else {
                t("console.status.recommendations.none", locale).into()
            },
            owner_next_step: if decision_items > 0 {
                Some(t("console.status.recommendations.next", locale).into())
            } else {
                None
            },
        },
        VNextStatusSummary {
            label: "Learning".into(), // identity: the WebApp localizes section labels
            health: "ready".into(),
            owner_message: t("console.status.learning.message", locale).into(),
            owner_next_step: None,
        },
    ];

    if let Some(settings) = settings {
        sections.push(vnext_cost_controls_status(settings, locale));
    }

    sections.extend([
        VNextStatusSummary {
            label: "Standing Instructions".into(), // identity: the WebApp localizes section labels
            health: "ready".into(),
            owner_message: t("console.status.standing.message", locale).into(),
            owner_next_step: None,
        },
        VNextStatusSummary {
            label: "Follow-ups".into(), // identity: the WebApp localizes section labels
            health: if follow_up_items > 0 {
                "ready"
            } else {
                "quiet"
            }
            .into(),
            owner_message: if follow_up_items > 0 {
                tfp(
                    "console.status.followups.some",
                    locale,
                    &[&follow_up_items.to_string()],
                )
            } else {
                t("console.status.followups.none", locale).into()
            },
            owner_next_step: if follow_up_items > 0 {
                Some(t("console.status.followups.next", locale).into())
            } else {
                None
            },
        },
        VNextStatusSummary {
            label: "Receipts".into(), // identity: the WebApp localizes section labels
            health: "ready".into(),
            owner_message: t("console.status.receipts.message", locale).into(),
            owner_next_step: None,
        },
        vnext_status_entry(projection, locale),
    ]);

    if let Some(settings) = settings {
        apply_settings_audit_to_status(&mut sections, settings, locale);
    }

    VNextStatusProjection {
        schema_version: 1,
        generated_at: projection.generated_at.clone(),
        sections,
        system_events,
        advanced_diagnostics_available: webapp_debug_snapshots_enabled(),
        llm_degraded,
        embedder_degraded,
    }
}

/// Telegram connectivity as the console renders it: the state the bot
/// reported last, when that state was entered, and the error behind a
/// failure. `degraded` is the server's own verdict
/// ([`crate::telegram_health::TelegramHealthSnapshot::is_degraded_at`]) so the
/// console does not have to trust its clock against `since_unix` to decide
/// whether a ten-minute-old `unreachable` is still a blip.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct VNextTelegramRail {
    pub(crate) state: crate::telegram_health::TelegramState,
    pub(crate) since_unix: i64,
    #[serde(default)]
    pub(crate) last_error: Option<String>,
    #[serde(default)]
    pub(crate) degraded: bool,
}

/// Project the process-global Telegram snapshot for the console.
pub(crate) fn vnext_telegram_rail(
    snapshot: &crate::telegram_health::TelegramHealthSnapshot,
    now_unix: i64,
) -> VNextTelegramRail {
    VNextTelegramRail {
        state: snapshot.state,
        since_unix: snapshot.since_unix,
        last_error: snapshot.last_error.clone(),
        degraded: snapshot.is_degraded_at(now_unix),
    }
}

/// Wire shape of `GET /webapp/vnext/status`: the frozen
/// [`VNextStatusProjection`] plus the two runtime rails added after incident
/// 2026-09-15 (the bot token was revoked for hours and the console never
/// said a word). `#[serde(flatten)]` keeps the object flat, so old clients
/// see exactly the fields they know and new clients read `telegram_rail` and
/// `nightly` next to `llm_degraded` / `embedder_degraded`.
#[derive(Serialize)]
pub(super) struct VNextStatusResponse {
    #[serde(flatten)]
    pub(super) projection: VNextStatusProjection,
    pub(super) telegram_rail: VNextTelegramRail,
    /// Last nightly verdict (`null` when the status file is absent).
    pub(super) nightly: Option<NightlyStatus>,
}

fn apply_settings_audit_to_status(
    sections: &mut Vec<VNextStatusSummary>,
    settings: &RuntimeSettings,
    locale: &str,
) {
    let catalog = crate::model_catalog::cached_model_catalog();
    let findings =
        crate::settings_audit::audit_runtime_settings(settings, locale, catalog.as_ref());
    let attention: Vec<_> = findings
        .into_iter()
        .filter(|finding| finding.severity == "attention")
        .collect();
    if attention.is_empty() {
        return;
    }

    if let Some(health) = sections
        .iter_mut()
        .find(|section| section.label == "System Health")
    {
        health.health = "attention".into();
        if health.owner_next_step.is_none() {
            health.owner_message = attention[0].message.clone();
            health.owner_next_step = Some(t("settings.audit.next", locale).into());
        }
    }

    for finding in attention {
        sections.push(VNextStatusSummary {
            label: finding.message.clone(),
            health: "attention".into(),
            owner_message: finding.message,
            owner_next_step: Some(t("settings.audit.next", locale).into()),
        });
    }
}

pub(super) async fn vnext_system_events_from_memory(
    memory: &MemoryHandle,
    user_did: &str,
    locale: &str,
) -> Vec<VNextSystemEvent> {
    let entries = memory
        .list_recent_l3(user_did, WEBAPP_VNEXT_CHAT_HISTORY_ARCHIVE_SCAN_LIMIT)
        .await
        .unwrap_or_default();
    vnext_system_events_from_l3(entries, locale)
}

pub(super) fn vnext_system_events_from_l3(
    entries: Vec<crate::memory::L3Entry>,
    locale: &str,
) -> Vec<VNextSystemEvent> {
    let mut grouped: BTreeMap<String, VNextSystemEvent> = BTreeMap::new();
    for entry in entries {
        let occurred_at = parse_vnext_chat_history_timestamp(&entry.created_at);
        let Some(draft) =
            vnext_system_event_draft(&entry.role, &entry.content, occurred_at, locale)
        else {
            continue;
        };
        grouped
            .entry(draft.kind.clone())
            .and_modify(|event| {
                event.count += 1;
                event.window_start_at = event.window_start_at.min(draft.occurred_at);
                if draft.occurred_at >= event.occurred_at {
                    event.occurred_at = draft.occurred_at;
                    event.summary = draft.summary.clone();
                    event.severity = draft.severity.clone();
                    event.title = draft.title.clone();
                }
                push_system_event_detail(&mut event.details, draft.detail.clone());
            })
            .or_insert_with(|| VNextSystemEvent {
                id: format!("system:{}:{}", draft.kind, draft.occurred_at),
                kind: draft.kind,
                severity: draft.severity,
                title: draft.title,
                summary: draft.summary,
                occurred_at: draft.occurred_at,
                window_start_at: draft.occurred_at,
                count: 1,
                details: vec![draft.detail],
            });
    }
    let mut events = grouped.into_values().collect::<Vec<_>>();
    for event in &mut events {
        event.id = format!("system:{}:{}", event.kind, event.occurred_at);
        event.details.truncate(5);
    }
    events.sort_by_key(|left| std::cmp::Reverse(left.occurred_at));
    events.truncate(8);
    events
}

struct VNextSystemEventDraft {
    kind: String,
    severity: String,
    title: String,
    summary: String,
    detail: String,
    occurred_at: i64,
}

fn vnext_system_event_draft(
    role: &str,
    text: &str,
    occurred_at: i64,
    locale: &str,
) -> Option<VNextSystemEventDraft> {
    if role.trim() != "assistant" {
        return None;
    }
    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let lower = normalized.to_ascii_lowercase();
    if normalized.contains("系统已重启") || lower.contains("system restarted") {
        return Some(VNextSystemEventDraft {
            kind: "restart".into(),
            severity: "quiet".into(),
            title: t("console.event.restart.title", locale).into(),
            summary: t("console.event.restart.summary", locale).into(),
            detail: system_event_detail(&normalized, locale),
            occurred_at,
        });
    }
    if lower.starts_with("system integrity report")
        && lower.contains("invariants:")
        && lower.contains("rate monitors:")
    {
        let has_warning = normalized.contains('⚠') || lower.contains("warn");
        return Some(VNextSystemEventDraft {
            kind: "integrity_report".into(),
            severity: if has_warning { "notice" } else { "quiet" }.into(),
            title: t("console.event.integrity.title", locale).into(),
            summary: if has_warning {
                t("console.event.integrity.warn", locale).into()
            } else {
                t("console.event.integrity.ok", locale).into()
            },
            detail: system_event_detail(&normalized, locale),
            occurred_at,
        });
    }
    if (lower.starts_with("daily brief") || lower.starts_with("# daily brief"))
        && lower.contains("calendar not queried")
    {
        return Some(VNextSystemEventDraft {
            kind: "incomplete_daily_brief".into(),
            severity: "notice".into(),
            title: t("console.event.brief.title", locale).into(),
            summary: t("console.event.brief.summary", locale).into(),
            detail: system_event_detail(&normalized, locale),
            occurred_at,
        });
    }
    None
}

fn system_event_detail(text: &str, locale: &str) -> String {
    // The card title and summary already say what happened. The stored notice
    // is a bilingual dump ("系统已重启 / System restarted", "Active skills").
    // Keep only the counts, in the owner's language.
    let mut parts = Vec::new();
    if let Some(count) = number_after_marker(
        text,
        &["Active skills", "活跃技能 / Active skills", "活跃技能"],
    ) {
        parts.push(tfp("console.event.restart.skills", locale, &[&count]));
    }
    if let Some(count) = number_after_marker(text, &["HITL 待审批", "Pending approvals"]) {
        parts.push(tfp("console.event.restart.pending", locale, &[&count]));
    }
    parts.join(" · ")
}

fn number_after_marker(text: &str, markers: &[&str]) -> Option<String> {
    let mut found: Option<String> = None;
    for marker in markers {
        let Some(index) = text.find(marker) else {
            continue;
        };
        let rest = text[index + marker.len()..].trim_start_matches([':', '：', ' ', '/']);
        let digits: String = rest.chars().take_while(|ch| ch.is_ascii_digit()).collect();
        if !digits.is_empty() {
            found = Some(digits);
            break;
        }
    }
    found
}

fn push_system_event_detail(details: &mut Vec<String>, detail: String) {
    if detail.trim().is_empty() || details.iter().any(|existing| existing == &detail) {
        return;
    }
    details.insert(0, detail);
    details.truncate(5);
}

fn vnext_cost_controls_status(settings: &RuntimeSettings, locale: &str) -> VNextStatusSummary {
    let health_probe_minutes =
        settings.get_u64_clamped("health_probe_interval_minutes", 60, 0, 24 * 60);
    let true_heartbeat = settings
        .get("true_heartbeat_config")
        .and_then(|value| value.get("enabled").and_then(serde_json::Value::as_bool))
        .unwrap_or(false);
    let heartbeat_agent_paused = settings
        .get("heartbeat_config")
        .and_then(|value| crate::heartbeat_config::HeartbeatConfig::from_json(&value))
        .map(|config| {
            config.per_tick.tool_calls == 0
                && config.per_tick.send_message == 0
                && !config.controlled_thaw.enabled
        })
        .unwrap_or(false);
    let skill_toggle_summary = skill_heartbeat_toggle_pause_summary(settings);
    let all_autonomous_paused = health_probe_minutes == 0
        && !true_heartbeat
        && heartbeat_agent_paused
        && skill_toggle_summary.enabled_count == 0;

    let owner_message = if all_autonomous_paused {
        t("console.status.cost.paused", locale).to_string()
    } else {
        let mut parts = Vec::new();
        parts.push(
            t(
                if true_heartbeat {
                    "console.status.cost.review_on"
                } else {
                    "console.status.cost.review_off"
                },
                locale,
            )
            .to_string(),
        );
        parts.push(
            t(
                if heartbeat_agent_paused {
                    "console.status.cost.budget_off"
                } else {
                    "console.status.cost.budget_on"
                },
                locale,
            )
            .to_string(),
        );
        parts.push(if skill_toggle_summary.total_count == 0 {
            t("console.status.cost.skills_none", locale).to_string()
        } else if skill_toggle_summary.enabled_count == 0 {
            tfp(
                "console.status.cost.skills_paused",
                locale,
                &[&skill_toggle_summary.total_count.to_string()],
            )
        } else {
            tfp(
                "console.status.cost.skills_partial",
                locale,
                &[
                    &skill_toggle_summary.enabled_count.to_string(),
                    &skill_toggle_summary.total_count.to_string(),
                ],
            )
        });
        parts.push(if health_probe_minutes == 0 {
            t("console.status.cost.health_off", locale).to_string()
        } else {
            tfp(
                "console.status.cost.health_every",
                locale,
                &[&health_probe_minutes.to_string()],
            )
        });
        tfp("console.status.cost.summary", locale, &[&parts.join("; ")])
    };

    VNextStatusSummary {
        label: "Cost controls".into(), // identity: the WebApp localizes section labels
        health: if all_autonomous_paused {
            "quiet"
        } else {
            "ready"
        }
        .into(),
        owner_message,
        owner_next_step: Some(t("console.status.cost.next", locale).into()),
    }
}

pub(super) fn vnext_cost_saving_paused(settings: &RuntimeSettings) -> bool {
    let health_probe_minutes =
        settings.get_u64_clamped("health_probe_interval_minutes", 60, 0, 24 * 60);
    let true_heartbeat = settings
        .get("true_heartbeat_config")
        .and_then(|value| value.get("enabled").and_then(serde_json::Value::as_bool))
        .unwrap_or(false);
    let heartbeat_agent_paused = settings
        .get("heartbeat_config")
        .and_then(|value| crate::heartbeat_config::HeartbeatConfig::from_json(&value))
        .map(|config| {
            config.per_tick.tool_calls == 0
                && config.per_tick.send_message == 0
                && !config.controlled_thaw.enabled
        })
        .unwrap_or(false);
    health_probe_minutes == 0
        && !true_heartbeat
        && heartbeat_agent_paused
        && skill_heartbeat_toggle_pause_summary(settings).enabled_count == 0
}

const COST_SAVING_PAUSE_SNAPSHOT_SETTING: &str = "cost_saving_pause_snapshot";

const COST_SAVING_RUNTIME_KEYS: &[&str] = &[
    "health_probe_interval_minutes",
    "true_heartbeat_config",
    "heartbeat_config",
];

const COST_SAVING_COMPAT_KEYS: &[&str] = &[
    "heartbeat_autonomy_v2",
    "news_digest",
    "calendar_briefing",
    "server_monitoring",
    "github_alerts",
    "email_notifications",
    "skill_news_digest",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SkillHeartbeatTogglePauseSummary {
    total_count: usize,
    enabled_count: usize,
}

fn is_skill_heartbeat_toggle_key(key: &str) -> bool {
    key.starts_with("skill_")
}

fn skill_heartbeat_toggle_keys(settings: &RuntimeSettings) -> Vec<String> {
    settings
        .all()
        .into_iter()
        .filter(|snapshot| is_skill_heartbeat_toggle_key(&snapshot.key))
        .map(|snapshot| snapshot.key)
        .collect()
}

fn skill_heartbeat_toggle_pause_summary(
    settings: &RuntimeSettings,
) -> SkillHeartbeatTogglePauseSummary {
    let keys = skill_heartbeat_toggle_keys(settings);
    let enabled_count = keys
        .iter()
        .filter(|key| settings.is_enabled(key.as_str()))
        .count();
    SkillHeartbeatTogglePauseSummary {
        total_count: keys.len(),
        enabled_count,
    }
}

pub(super) async fn persist_runtime_setting(
    state: &AdminState,
    key: &str,
    value: serde_json::Value,
) -> anyhow::Result<()> {
    persist_runtime_settings(state, vec![(key, value)]).await
}

pub(super) async fn persist_runtime_settings(
    state: &AdminState,
    updates: Vec<(&str, serde_json::Value)>,
) -> anyhow::Result<()> {
    state
        .memory
        .commit_runtime_settings(
            state.settings.clone(),
            updates
                .into_iter()
                .map(|(key, value)| (key.to_owned(), Some(value)))
                .collect(),
        )
        .await
        .context("commit runtime settings")?;
    Ok(())
}

pub(super) async fn persist_runtime_setting_locked(
    state: &AdminState,
    key: &str,
    value: serde_json::Value,
    guard: crate::call_validation::SettingsWriteGuard,
) -> anyhow::Result<()> {
    persist_runtime_settings_locked(state, vec![(key, value)], guard).await
}

pub(super) async fn persist_runtime_settings_locked(
    state: &AdminState,
    updates: Vec<(&str, serde_json::Value)>,
    guard: crate::call_validation::SettingsWriteGuard,
) -> anyhow::Result<()> {
    state
        .memory
        .commit_runtime_settings_locked(
            state.settings.clone(),
            updates
                .into_iter()
                .map(|(key, value)| (key.to_owned(), Some(value)))
                .collect(),
            guard,
        )
        .await
        .context("commit runtime settings")?;
    Ok(())
}

async fn persist_compat_runtime_setting(
    state: &AdminState,
    key: &str,
    value: serde_json::Value,
) -> anyhow::Result<()> {
    if state
        .settings
        .prepare_updates(vec![(key.to_owned(), Some(value.clone()))])
        .is_err()
    {
        // Retired and pre-registration skill keys may be absent from the live
        // registry. Persist them anyway so a future restart or migration does
        // not resurrect old autonomous behavior from defaults.
        state.memory.store_setting(key, value).await
    } else {
        persist_runtime_setting(state, key, value).await
    }
}

async fn persist_true_heartbeat_config(
    state: &AdminState,
    config: crate::chronos::TrueHeartbeatConfig,
) -> anyhow::Result<()> {
    persist_runtime_setting(
        state,
        "true_heartbeat_config",
        serde_json::to_value(&config)?,
    )
    .await?;
    if let Some(orchestrator) = state.orchestrator.as_ref() {
        orchestrator.apply_true_heartbeat_config(config);
    }
    Ok(())
}

async fn persist_heartbeat_agent_config(
    state: &AdminState,
    config: crate::heartbeat_config::HeartbeatConfig,
    reason: &str,
) -> anyhow::Result<()> {
    if let Some(orchestrator) = state.orchestrator.as_ref() {
        orchestrator
            .update_heartbeat_config(
                config,
                crate::heartbeat_config::ConfigAuditActor::User,
                Some(reason.into()),
            )
            .await?;
    } else {
        persist_runtime_setting(state, "heartbeat_config", config.to_json()).await?;
    }
    Ok(())
}

async fn cost_saving_pause_snapshot(state: &AdminState) -> anyhow::Result<serde_json::Value> {
    let persisted = state.memory.load_all_settings().await.unwrap_or_default();
    let mut snapshot = serde_json::Map::new();

    for key in COST_SAVING_RUNTIME_KEYS {
        if let Some(value) = state.settings.get(key) {
            snapshot.insert((*key).into(), value);
        }
    }
    for key in COST_SAVING_COMPAT_KEYS {
        if let Some(value) = state
            .settings
            .get(key)
            .or_else(|| persisted.get(*key).cloned())
        {
            snapshot.insert((*key).into(), value);
        }
    }
    for key in skill_heartbeat_toggle_keys(&state.settings) {
        if let Some(value) = state.settings.get(&key) {
            snapshot.insert(key, value);
        }
    }

    Ok(serde_json::Value::Object(snapshot))
}

pub(super) async fn pause_background_autonomy_for_cost_saving(
    state: &AdminState,
) -> anyhow::Result<()> {
    if !vnext_cost_saving_paused(&state.settings) {
        let snapshot = cost_saving_pause_snapshot(state).await?;
        persist_runtime_setting(state, COST_SAVING_PAUSE_SNAPSHOT_SETTING, snapshot).await?;
    }

    for key in COST_SAVING_COMPAT_KEYS {
        // Retired/legacy autonomy switches are kept false so persisted settings
        // cannot imply background work is allowed after a restart or migration.
        persist_compat_runtime_setting(state, key, serde_json::Value::Bool(false)).await?;
    }
    for key in skill_heartbeat_toggle_keys(&state.settings) {
        persist_runtime_setting(state, &key, serde_json::Value::Bool(false)).await?;
    }
    persist_runtime_setting(state, "health_probe_interval_minutes", serde_json::json!(0)).await?;

    persist_true_heartbeat_config(
        state,
        crate::chronos::TrueHeartbeatConfig {
            enabled: false,
            interval_minutes: 120,
            quiet_hours: None,
            autonomous_overrides: None,
        },
    )
    .await?;

    let mut heartbeat_config = state
        .settings
        .get("heartbeat_config")
        .and_then(|value| crate::heartbeat_config::HeartbeatConfig::from_json(&value))
        .or_else(|| {
            state
                .orchestrator
                .as_ref()
                .map(|orchestrator| orchestrator.heartbeat_config())
        })
        .unwrap_or_default();
    heartbeat_config.frequency_minutes = 120;
    heartbeat_config.quiet_hours = crate::heartbeat_config::QuietHours {
        start: "00:00".into(),
        end: "23:59".into(),
    };
    heartbeat_config.per_tick.tool_calls = 0;
    heartbeat_config.per_tick.send_message = 0;
    heartbeat_config.controlled_thaw.enabled = false;
    heartbeat_config.controlled_thaw.max_llm_wakeups_per_hour = 0;
    heartbeat_config
        .controlled_thaw
        .max_background_proposals_per_day = 0;
    heartbeat_config
        .controlled_thaw
        .require_approval_for_external_actions = true;

    persist_heartbeat_agent_config(state, heartbeat_config, "webapp settings cost-saving pause")
        .await?;
    Ok(())
}

async fn restore_cost_saving_setting(
    state: &AdminState,
    key: &str,
    value: serde_json::Value,
) -> anyhow::Result<()> {
    if key == "true_heartbeat_config" {
        let config: crate::chronos::TrueHeartbeatConfig = serde_json::from_value(value)
            .context("decode saved true heartbeat cost-saving snapshot")?;
        persist_true_heartbeat_config(state, config).await?;
    } else if key == "heartbeat_config" {
        let Some(config) = crate::heartbeat_config::HeartbeatConfig::from_json(&value) else {
            anyhow::bail!("decode saved heartbeat cost-saving snapshot");
        };
        persist_heartbeat_agent_config(
            state,
            config,
            "webapp settings cost-saving resume from snapshot",
        )
        .await?;
    } else {
        persist_compat_runtime_setting(state, key, value).await?;
    }
    Ok(())
}

pub(super) async fn resume_background_autonomy_after_cost_saving(
    state: &AdminState,
) -> anyhow::Result<()> {
    let snapshot = state
        .settings
        .get(COST_SAVING_PAUSE_SNAPSHOT_SETTING)
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default();

    if snapshot.is_empty() {
        // No prior snapshot exists (for example, a deployment that predates
        // snapshots already paused cost-saving mode). Resume only low-cost,
        // owner-adjacent background help; do not reopen scheduled skill checks
        // without a stored owner-approved snapshot.
        persist_runtime_setting(
            state,
            "health_probe_interval_minutes",
            serde_json::json!(120),
        )
        .await?;
        persist_true_heartbeat_config(
            state,
            crate::chronos::TrueHeartbeatConfig {
                enabled: false,
                interval_minutes: 120,
                quiet_hours: None,
                autonomous_overrides: None,
            },
        )
        .await?;
        let mut heartbeat_config = crate::heartbeat_config::HeartbeatConfig::default();
        heartbeat_config.frequency_minutes = 120;
        heartbeat_config.quiet_hours = crate::heartbeat_config::QuietHours {
            start: "00:00".into(),
            end: "23:59".into(),
        };
        heartbeat_config.per_tick.tool_calls = 0;
        heartbeat_config.per_tick.send_message = 0;
        heartbeat_config.controlled_thaw.enabled = false;
        heartbeat_config.controlled_thaw.max_llm_wakeups_per_hour = 0;
        heartbeat_config
            .controlled_thaw
            .max_background_proposals_per_day = 0;
        persist_heartbeat_agent_config(
            state,
            heartbeat_config,
            "webapp settings cost-saving resume with low-cost defaults",
        )
        .await?;
    } else {
        for (key, value) in snapshot {
            restore_cost_saving_setting(state, &key, value).await?;
        }
    }

    persist_runtime_setting(
        state,
        COST_SAVING_PAUSE_SNAPSHOT_SETTING,
        serde_json::json!({}),
    )
    .await?;
    Ok(())
}

/// Sources that are genuine external connections; everything else is an
/// internal always-on projection source.
const EXTERNAL_SERVICE_SOURCES: &[&str] =
    &["calendar", "google_drive", "drive", "hercules", "telegram"];

pub(super) fn vnext_settings_projection(
    principal: &WebConsolePrincipal,
    projection: &UnifiedInboxProjection,
    settings: Option<&RuntimeSettings>,
) -> VNextSettingsProjection {
    let locale_owned = settings
        .map(RuntimeSettings::display_language)
        .unwrap_or_else(|| crate::i18n::DEFAULT_LOCALE.to_string());
    let locale = locale_owned.as_str();
    // The Web Push card on the Settings page already renders /web-push/status,
    // so notification preferences carry only Telegram and the cost controls.
    let mut notification_preferences = vec![VNextStatusSummary {
        label: "Telegram".into(), // identity: the WebApp localizes section labels
        health: "ready".into(),
        owner_message: t("console.settings.telegram.message", locale).into(),
        owner_next_step: None,
    }];
    if let Some(settings) = settings {
        notification_preferences.push(vnext_cost_controls_status(settings, locale));
    }
    let (web_search_provider, web_search_model, web_search_model_options) =
        web_search_projection(settings);

    VNextSettingsProjection {
        schema_version: 1,
        signed_in_owner: vnext_principal_label(principal, locale),
        connected_services: projection
            .sources
            .iter()
            // Settings shows real connections and anything needing attention;
            // internal always-on sources (tasks, goals, memory, …) stay off the
            // page so it fits on a phone (owner principle: show what is useful).
            .filter(|source| {
                EXTERNAL_SERVICE_SOURCES.contains(&source.source.as_str())
                    || vnext_source_needs_attention(&source.status)
            })
            .map(|source| {
                let delegated_calendar =
                    source.source == "calendar" && source.status == "delegated";
                VNextStatusSummary {
                    label: vnext_source_label(&source.source, locale),
                    health: if vnext_source_needs_attention(&source.status) {
                        "attention"
                    } else if source.status == "deferred" || delegated_calendar {
                        "quiet"
                    } else {
                        "ready"
                    }
                    .into(),
                    owner_message: if delegated_calendar {
                        t("console.settings.calendar.delegated", locale).into()
                    } else {
                        source
                            .reason
                            .as_deref()
                            .map(|reason| vnext_source_reason(reason, locale))
                            .unwrap_or_else(|| t("console.status.ready", locale).into())
                    },
                    owner_next_step: if delegated_calendar {
                        None
                    } else {
                        vnext_source_next_step(source, locale)
                    },
                }
            })
            .collect(),
        security: vec![VNextStatusSummary {
            label: "Owner session".into(), // identity: the WebApp localizes section labels
            health: "ready".into(),
            owner_message: t("console.status.session.message", locale).into(),
            owner_next_step: None,
        }],
        notification_preferences,
        cost_saving_paused: settings.is_some_and(vnext_cost_saving_paused),
        advanced_diagnostics_available: webapp_debug_snapshots_enabled(),
        user_timezone: settings
            .map(crate::settings::resolve_user_tz_string)
            .unwrap_or_else(crate::settings::user_tz_default),
        display_language: settings
            .map(RuntimeSettings::display_language)
            .unwrap_or_else(|| "zh".to_string()),
        llm_provider: settings
            .and_then(|s| s.get("llm_provider"))
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_else(|| "openrouter".to_string()),
        llm_model: settings
            .and_then(|s| s.get("llm_model"))
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_default(),
        paid_api_fallback_enabled: settings
            .and_then(|s| s.get("paid_api_fallback_enabled"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        web_search_provider,
        web_search_model,
        web_search_model_options,
    }
}

fn web_search_projection(settings: Option<&RuntimeSettings>) -> (String, String, Vec<String>) {
    let provider = settings
        .and_then(|s| s.get("web_search_provider"))
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_else(|| "auto".into());
    let model = settings
        .and_then(|s| s.get("web_search_model"))
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default();
    if crate::web_search::normalize_web_search_provider(&provider).is_none() {
        // Preserve an unsupported saved choice for the Settings repair prompt.
        // Do not present a silent migration to automatic/subscription search.
        return (provider, String::new(), Vec::new());
    }
    let (provider, model) = crate::web_search::effective_web_search(&provider, &model);
    let mut options: Vec<String> = crate::llm_client::web_search_model_options(provider)
        .iter()
        .map(|id| (*id).to_string())
        .collect();
    if !model.is_empty() && !options.iter().any(|id| id == &model) {
        options.insert(0, model.clone());
    }
    (provider.to_string(), model, options)
}

pub(super) async fn web_push_status_for_owner(
    memory: &MemoryHandle,
    owner_did: &str,
    locale: &str,
) -> VNextWebPushStatus {
    let public_key = web_push_vapid_public_key();
    let send_configured = web_push_vapid_private_key().is_some();
    let subscription_count = web_push_load_state(memory, owner_did)
        .await
        .map(|state| state.subscriptions.len())
        .unwrap_or(0);
    let last_delivery = web_push_load_delivery_receipts(memory, owner_did)
        .await
        .ok()
        .and_then(|state| state.receipts.into_iter().next())
        .map(|receipt| VNextWebPushDeliverySummary {
            id: receipt.id,
            status: receipt.status,
            delivered: receipt.delivered,
            attempted: receipt.attempted,
            removed: receipt.removed,
            created_at_unix: receipt.created_at_unix,
        });
    let configured = public_key.is_some();
    let subscribed = subscription_count > 0;
    VNextWebPushStatus {
        schema_version: 1,
        configured,
        send_configured,
        subscribed,
        subscription_count,
        public_key,
        last_delivery,
        owner_message: if !configured {
            t("console.push.unconfigured", locale).into()
        } else if !send_configured {
            t("console.push.no_send_key", locale).into()
        } else if subscribed {
            t("console.push.subscribed", locale).into()
        } else {
            t("console.push.ready", locale).into()
        },
        owner_next_step: if !configured {
            Some(t("console.push.next.unconfigured", locale).into())
        } else if !send_configured {
            Some(t("console.push.next.no_send_key", locale).into())
        } else if subscribed {
            Some(t("console.push.next.subscribed", locale).into())
        } else {
            Some(t("console.push.next.ready", locale).into())
        },
    }
}

pub(super) fn web_push_vapid_public_key() -> Option<String> {
    std::env::var("AEGIS_WEB_PUSH_VAPID_PUBLIC_KEY")
        .ok()
        .or_else(|| std::env::var("WEB_PUSH_VAPID_PUBLIC_KEY").ok())
        .map(|value| value.trim().to_string())
        .filter(|value| validate_web_push_public_key(value))
}

#[derive(Debug, Clone)]
pub(super) enum WebPushVapidPrivateKey {
    RawBase64(String),
    Pem(String),
}

fn web_push_vapid_private_key() -> Option<WebPushVapidPrivateKey> {
    std::env::var("AEGIS_WEB_PUSH_VAPID_PRIVATE_KEY")
        .ok()
        .or_else(|| std::env::var("WEB_PUSH_VAPID_PRIVATE_KEY").ok())
        .and_then(|value| parse_web_push_vapid_private_key(&value))
}

pub(super) fn parse_web_push_vapid_private_key(value: &str) -> Option<WebPushVapidPrivateKey> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.len() > WEB_PUSH_VAPID_PRIVATE_KEY_MAX_CHARS {
        return None;
    }
    if trimmed.contains("BEGIN EC PRIVATE KEY") || trimmed.contains("BEGIN PRIVATE KEY") {
        return Some(WebPushVapidPrivateKey::Pem(trimmed.to_string()));
    }
    let normalized = trimmed.trim_end_matches('=');
    let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(normalized)
        .ok()?;
    if decoded.len() == 32 {
        Some(WebPushVapidPrivateKey::RawBase64(normalized.to_string()))
    } else {
        None
    }
}

fn web_push_vapid_subject() -> String {
    std::env::var("AEGIS_WEB_PUSH_VAPID_SUBJECT")
        .ok()
        .or_else(|| std::env::var("WEB_PUSH_VAPID_SUBJECT").ok())
        .map(|value| value.trim().to_string())
        .filter(|value| {
            value.len() <= 200 && (value.starts_with("mailto:") || value.starts_with("https://"))
        })
        .unwrap_or_else(|| "mailto:admin@example.invalid".to_string())
}

pub(super) fn validate_web_push_public_key(value: &str) -> bool {
    let normalized = value.trim();
    if normalized.is_empty() || normalized.len() > WEB_PUSH_MAX_KEY_CHARS {
        return false;
    }
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(normalized)
        .or_else(|_| base64::engine::general_purpose::URL_SAFE.decode(normalized))
        .is_ok_and(|decoded| decoded.len() == 65)
}

pub(super) fn web_push_subscription_record_from_request(
    request: VNextWebPushSubscribeRequest,
) -> Result<WebPushSubscriptionRecord, (StatusCode, String)> {
    let endpoint = validate_web_push_endpoint(&request.subscription.endpoint)?;
    let keys = WebPushSubscriptionKeys {
        p256dh: validate_web_push_key("p256dh", &request.subscription.keys.p256dh)?,
        auth: validate_web_push_key("auth", &request.subscription.keys.auth)?,
    };
    let now = chrono::Utc::now().timestamp();
    Ok(WebPushSubscriptionRecord {
        endpoint,
        keys,
        user_agent: request
            .user_agent
            .map(|value| value.trim().chars().take(240).collect::<String>())
            .filter(|value| !value.is_empty()),
        created_at_unix: now,
        updated_at_unix: now,
    })
}

pub(super) fn validate_web_push_endpoint(value: &str) -> Result<String, (StatusCode, String)> {
    let endpoint = value.trim();
    if endpoint.is_empty() || endpoint.len() > WEB_PUSH_MAX_ENDPOINT_CHARS {
        return Err((StatusCode::BAD_REQUEST, "Invalid Web Push endpoint.".into()));
    }
    let parsed = reqwest::Url::parse(endpoint).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            "Invalid Web Push endpoint.".to_string(),
        )
    })?;
    if parsed.scheme() != "https"
        || parsed.host_str().is_none()
        || parsed.username() != ""
        || parsed.password().is_some()
    {
        return Err((StatusCode::BAD_REQUEST, "Invalid Web Push endpoint.".into()));
    }
    Ok(endpoint.to_string())
}

fn validate_web_push_key(label: &str, value: &str) -> Result<String, (StatusCode, String)> {
    let key = value.trim();
    if key.is_empty() || key.len() > WEB_PUSH_MAX_KEY_CHARS {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("Invalid Web Push {label} key."),
        ));
    }
    let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(key)
        .or_else(|_| base64::engine::general_purpose::URL_SAFE.decode(key))
        .or_else(|_| base64::engine::general_purpose::STANDARD.decode(key))
        .map_err(|_| {
            (
                StatusCode::BAD_REQUEST,
                format!("Invalid Web Push {label} key."),
            )
        })?;
    let valid_shape = match label {
        "p256dh" => decoded.len() == 65,
        "auth" => (8..=32).contains(&decoded.len()),
        _ => !decoded.is_empty(),
    };
    if !valid_shape {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("Invalid Web Push {label} key."),
        ));
    }
    Ok(key.to_string())
}

async fn web_push_load_state(
    memory: &MemoryHandle,
    owner_did: &str,
) -> anyhow::Result<WebPushSubscriptionState> {
    let raw = memory
        .load_skill_state(WEB_PUSH_SUBSCRIPTIONS_SKILL, owner_did)
        .await?;
    let Some(raw) = raw else {
        return Ok(WebPushSubscriptionState {
            schema_version: 1,
            owner_did: owner_did.to_string(),
            subscriptions: Vec::new(),
        });
    };
    let mut state: WebPushSubscriptionState = serde_json::from_str(&raw)?;
    if state.schema_version != 1 || state.owner_did != owner_did {
        state = WebPushSubscriptionState {
            schema_version: 1,
            owner_did: owner_did.to_string(),
            subscriptions: Vec::new(),
        };
    }
    state.subscriptions.retain(|subscription| {
        validate_web_push_endpoint(&subscription.endpoint).is_ok()
            && validate_web_push_key("p256dh", &subscription.keys.p256dh).is_ok()
            && validate_web_push_key("auth", &subscription.keys.auth).is_ok()
    });
    Ok(state)
}

pub(super) async fn web_push_store_subscription(
    memory: &MemoryHandle,
    owner_did: &str,
    subscription: WebPushSubscriptionRecord,
) -> Result<(), (StatusCode, String)> {
    let mut state = web_push_load_state(memory, owner_did)
        .await
        .map_err(|err| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Unable to load Web Push subscriptions: {err}"),
            )
        })?;
    if let Some(existing) = state
        .subscriptions
        .iter_mut()
        .find(|existing| existing.endpoint == subscription.endpoint)
    {
        existing.keys = subscription.keys;
        existing.user_agent = subscription.user_agent;
        existing.updated_at_unix = subscription.updated_at_unix;
    } else {
        state.subscriptions.push(subscription);
        state
            .subscriptions
            .sort_by_key(|subscription| std::cmp::Reverse(subscription.updated_at_unix));
        state
            .subscriptions
            .truncate(WEB_PUSH_MAX_SUBSCRIPTIONS_PER_OWNER);
    }
    memory
        .store_skill_state(
            WEB_PUSH_SUBSCRIPTIONS_SKILL,
            owner_did,
            &serde_json::to_string(&state).map_err(|err| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Unable to serialize Web Push subscriptions: {err}"),
                )
            })?,
        )
        .await
        .map_err(|err| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Unable to save Web Push subscription: {err}"),
            )
        })
}

pub(super) async fn web_push_remove_subscription(
    memory: &MemoryHandle,
    owner_did: &str,
    endpoint: &str,
) -> Result<(), (StatusCode, String)> {
    let mut state = web_push_load_state(memory, owner_did)
        .await
        .map_err(|err| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Unable to load Web Push subscriptions: {err}"),
            )
        })?;
    state
        .subscriptions
        .retain(|subscription| subscription.endpoint != endpoint);
    memory
        .store_skill_state(
            WEB_PUSH_SUBSCRIPTIONS_SKILL,
            owner_did,
            &serde_json::to_string(&state).map_err(|err| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Unable to serialize Web Push subscriptions: {err}"),
                )
            })?,
        )
        .await
        .map_err(|err| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Unable to save Web Push subscription: {err}"),
            )
        })
}

impl WebPushNotice {
    pub(super) fn new(
        title: impl AsRef<str>,
        body: impl AsRef<str>,
        tag: impl AsRef<str>,
        url: impl AsRef<str>,
    ) -> Result<Self, (StatusCode, String)> {
        let title = truncate_chars(title.as_ref().trim(), WEB_PUSH_MAX_NOTICE_TITLE_CHARS);
        let body = truncate_chars(body.as_ref().trim(), WEB_PUSH_MAX_NOTICE_BODY_CHARS);
        let tag = truncate_chars(tag.as_ref().trim(), WEB_PUSH_MAX_NOTICE_TAG_CHARS);
        let url = sanitize_web_push_url(url.as_ref())?;
        Ok(Self {
            title: if title.is_empty() {
                "Aegis".into()
            } else {
                title
            },
            body: if body.is_empty() {
                t("console.push.notice.fallback", crate::i18n::DEFAULT_LOCALE).into()
            } else {
                body
            },
            tag: if tag.is_empty() {
                "aegis-owner-notice".into()
            } else {
                tag
            },
            url,
        })
    }
}

fn sanitize_web_push_url(value: &str) -> Result<String, (StatusCode, String)> {
    let url = value.trim();
    if url.is_empty() {
        return Ok(WEB_PUSH_DEFAULT_URL.to_string());
    }
    if url.len() > WEB_PUSH_MAX_NOTICE_URL_CHARS
        || !url.starts_with("/ui/")
        || url.contains('\n')
        || url.contains('\r')
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "Invalid Web Push target URL.".into(),
        ));
    }
    Ok(url.to_string())
}

pub(super) async fn web_push_send_notice_to_owner(
    memory: &MemoryHandle,
    owner_did: &str,
    notice: WebPushNotice,
) -> Result<VNextWebPushTestResponse, (StatusCode, String)> {
    let private_key = web_push_vapid_private_key().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            "Web Push send key is not configured.".to_string(),
        )
    })?;
    let subject = web_push_vapid_subject();
    let mut state = web_push_load_state(memory, owner_did)
        .await
        .map_err(|err| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Unable to load Web Push subscriptions: {err}"),
            )
        })?;
    if state.subscriptions.is_empty() {
        return Ok(VNextWebPushTestResponse {
            schema_version: 1,
            status: "no_subscriptions".into(),
            attempted: 0,
            delivered: 0,
            failed: 0,
            removed: 0,
            receipt: "web_push:none".into(),
            owner_message: t("console.push.none_registered", crate::i18n::DEFAULT_LOCALE).into(),
        });
    }

    let client = IsahcWebPushClient::new().map_err(|err| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            format!("Unable to create Web Push client: {err}"),
        )
    })?;
    let mut delivered = 0;
    let mut failed = 0;
    let mut expired_endpoints = HashSet::new();
    let attempted = state.subscriptions.len();
    for subscription in state.subscriptions.clone() {
        match web_push_send_subscription(&client, &private_key, &subject, &subscription, &notice)
            .await
        {
            Ok(()) => delivered += 1,
            Err(err) => {
                failed += 1;
                if err.expired_subscription {
                    expired_endpoints.insert(subscription.endpoint.clone());
                }
                tracing::warn!(error = %err.message, "[WebPush] Failed to send owner notification");
            }
        }
    }
    let removed = expired_endpoints.len();
    if removed > 0 {
        state
            .subscriptions
            .retain(|subscription| !expired_endpoints.contains(&subscription.endpoint));
        memory
            .store_skill_state(
                WEB_PUSH_SUBSCRIPTIONS_SKILL,
                owner_did,
                &serde_json::to_string(&state).map_err(|err| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("Unable to serialize Web Push subscriptions: {err}"),
                    )
                })?,
            )
            .await
            .map_err(|err| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Unable to save Web Push subscription cleanup: {err}"),
                )
            })?;
    }
    let receipt = format!(
        "web_push:{}:{}:{}",
        notice.tag,
        chrono::Utc::now().timestamp(),
        delivered
    );
    let status = if delivered > 0 { "delivered" } else { "failed" };
    let delivery_receipt = WebPushDeliveryReceipt {
        id: receipt.clone(),
        created_at_unix: chrono::Utc::now().timestamp(),
        status: status.into(),
        tag: notice.tag.clone(),
        title: notice.title.clone(),
        attempted,
        delivered,
        failed,
        removed,
    };
    web_push_store_delivery_receipt(memory, owner_did, delivery_receipt)
        .await
        .map_err(|err| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Unable to store Web Push delivery receipt: {err}"),
            )
        })?;
    Ok(VNextWebPushTestResponse {
        schema_version: 1,
        status: status.into(),
        attempted,
        delivered,
        failed,
        removed,
        receipt,
        owner_message: if delivered > 0 {
            tfp(
                "console.push.delivered",
                crate::i18n::DEFAULT_LOCALE,
                &[&delivered.to_string(), &attempted.to_string()],
            )
        } else if removed > 0 {
            t("console.push.cleaned", crate::i18n::DEFAULT_LOCALE).into()
        } else {
            t("console.push.failed", crate::i18n::DEFAULT_LOCALE).into()
        },
    })
}

pub(crate) fn spawn_web_push_owner_notice(
    memory: MemoryHandle,
    owner_did: String,
    notice_text: String,
) {
    let Some(notice) = web_push_notice_from_owner_timeline_text(&notice_text) else {
        return;
    };
    tokio::spawn(async move {
        match web_push_send_notice_to_owner(&memory, &owner_did, notice).await {
            Ok(report) => {
                tracing::info!(
                    receipt = %report.receipt,
                    delivered = report.delivered,
                    attempted = report.attempted,
                    "[WebPush] Owner notice delivery attempted"
                );
            }
            Err((status, err)) => {
                if status != StatusCode::SERVICE_UNAVAILABLE {
                    tracing::warn!(status = %status, error = %err, "[WebPush] Owner notice delivery failed");
                }
            }
        }
    });
}

pub(crate) async fn try_owner_web_push_notice(
    memory: &MemoryHandle,
    owner_did: &str,
    notice_text: &str,
) -> bool {
    let Some(notice) = web_push_notice_from_owner_timeline_text(notice_text) else {
        return false;
    };
    match web_push_send_notice_to_owner(memory, owner_did, notice).await {
        Ok(report) if report.delivered > 0 => {
            tracing::info!(
                receipt = %report.receipt,
                delivered = report.delivered,
                attempted = report.attempted,
                "[WebPush] Owner notice delivered"
            );
            true
        }
        Ok(report) => {
            tracing::warn!(
                receipt = %report.receipt,
                status = %report.status,
                delivered = report.delivered,
                attempted = report.attempted,
                failed = report.failed,
                removed = report.removed,
                "[WebPush] Owner notice not delivered; Telegram fallback remains eligible"
            );
            false
        }
        Err((status, err)) => {
            tracing::warn!(
                status = %status,
                error = %err,
                "[WebPush] Owner notice delivery failed before send; Telegram fallback remains eligible"
            );
            false
        }
    }
}

pub(super) fn web_push_notice_from_owner_timeline_text(text: &str) -> Option<WebPushNotice> {
    let source = owner_push_source_text(text);
    let body = owner_push_body_text(text);
    if body.trim().is_empty() || !owner_notice_is_phone_worthy(&source) {
        return None;
    }
    let title = owner_push_title(&source);
    let tag = owner_push_tag(&source);
    WebPushNotice::new(title, body, tag, WEB_PUSH_DEFAULT_URL).ok()
}

fn owner_push_source_text(text: &str) -> String {
    let mut source = text.trim().to_string();
    if let Some((before_actions, _)) = source.split_once("\n\nWeb actions:") {
        source = before_actions.trim().to_string();
    }
    source
}

fn owner_push_body_text(text: &str) -> String {
    let mut body = owner_push_source_text(text);
    if owner_notice_is_active_alert(&body)
        && let Some((_, rest)) = body.split_once("\n\n")
    {
        body = rest.trim().to_string();
    }
    truncate_chars(&body, WEB_PUSH_MAX_NOTICE_BODY_CHARS)
}

fn owner_notice_is_resolved_alert(text: &str) -> bool {
    let trimmed = text.trim_start();
    if trimmed.contains("[RESOLVED]") {
        return true;
    }
    trimmed.starts_with('\u{2705}')
        && (trimmed.contains("已恢复正常")
            || trimmed.contains("back to normal")
            || trimmed.contains("[Aegis Runtime Alert]")
            || trimmed.contains("服务器：")
            || trimmed.contains("Server:"))
}

fn owner_notice_is_active_alert(text: &str) -> bool {
    if owner_notice_is_resolved_alert(text) {
        return false;
    }
    let trimmed = text.trim_start();
    trimmed.starts_with('\u{1f6a8}')
        || trimmed.starts_with('\u{26a0}')
        || trimmed.contains("[Aegis Runtime Alert]")
}

fn owner_alert_push_title(text: &str) -> Option<String> {
    if !owner_notice_is_active_alert(text) {
        return None;
    }
    let first = text.lines().next()?.trim();
    let stripped = first
        .trim_start_matches('\u{1f6a8}')
        .trim_start_matches('\u{26a0}')
        .trim_start_matches('\u{fe0f}')
        .trim();
    if stripped.is_empty() {
        None
    } else {
        Some(stripped.to_string())
    }
}

fn owner_notice_is_phone_worthy(text: &str) -> bool {
    // A recovery rides the same channel as its alert: with the push tag keyed
    // by subject it REPLACES the 🚨 on the phone. Treating it as not
    // phone-worthy made the caller fall back to Telegram for every ✅ while
    // the 🚨 had gone to the console.
    if owner_notice_is_resolved_alert(text) {
        return true;
    }
    let lower = text.to_ascii_lowercase();
    text.starts_with("⏰")
        || text.contains("‼️")
        || owner_notice_is_active_alert(text)
        || text.contains("等待批准")
        || text.contains("等待你批准")
        || lower.contains("awaiting approval")
        || lower.contains("needs a decision")
        || lower.contains("approval")
        || text.starts_with("📬")
        || lower.contains("authentication code")
        || lower.contains("security alert")
        || text.contains("安全警报")
}

fn owner_push_title(text: &str) -> String {
    let lower = text.to_ascii_lowercase();
    if text.starts_with("⏰") {
        "Reminder".into()
    } else if text.starts_with("📬") {
        "New mail".into()
    } else if text.contains("等待批准")
        || text.contains("等待你批准")
        || lower.contains("awaiting approval")
        || lower.contains("approval")
        || lower.contains("needs a decision")
    {
        "Aegis needs your decision".into()
    } else if let Some(title) = owner_alert_push_title(text) {
        title
    } else if text.contains("‼️") {
        "Important Aegis update".into()
    } else {
        "Aegis".into()
    }
}

fn owner_push_tag(text: &str) -> String {
    let kind = if text.starts_with("⏰") {
        "reminder"
    } else if text.starts_with("📬") {
        "mail"
    } else if text.to_ascii_lowercase().contains("approval") || text.contains("批准") {
        "approval"
    } else if owner_notice_is_active_alert(text)
        || owner_notice_is_resolved_alert(text)
        || text.contains("‼️")
    {
        "important"
    } else {
        "notice"
    };
    let mut hasher = DefaultHasher::new();
    // Alerts hash their SUBJECT ("服务器"), not the body: 70.1% / 70.3% /
    // ✅ then replace one another instead of stacking 14 notifications.
    if owner_notice_is_active_alert(text) || owner_notice_is_resolved_alert(text) {
        let first = text.lines().next().unwrap_or("").trim();
        let subject = first
            .trim_start_matches(|c: char| {
                !c.is_alphanumeric() && !('\u{4e00}'..='\u{9fff}').contains(&c)
            })
            .split(['：', ':'])
            .next()
            .unwrap_or(first)
            .trim();
        "alert".hash(&mut hasher);
        subject.hash(&mut hasher);
    } else {
        text.hash(&mut hasher);
    }
    format!("aegis-{kind}-{:x}", hasher.finish())
}

async fn web_push_store_delivery_receipt(
    memory: &MemoryHandle,
    owner_did: &str,
    receipt: WebPushDeliveryReceipt,
) -> anyhow::Result<()> {
    let mut state = web_push_load_delivery_receipts(memory, owner_did).await?;
    state.receipts.push(receipt);
    state
        .receipts
        .sort_by_key(|receipt| std::cmp::Reverse(receipt.created_at_unix));
    state
        .receipts
        .truncate(WEB_PUSH_MAX_DELIVERY_RECEIPTS_PER_OWNER);
    memory
        .store_skill_state(
            WEB_PUSH_DELIVERY_RECEIPTS_SKILL,
            owner_did,
            &serde_json::to_string(&state)?,
        )
        .await
}

async fn web_push_load_delivery_receipts(
    memory: &MemoryHandle,
    owner_did: &str,
) -> anyhow::Result<WebPushDeliveryReceiptState> {
    let raw = memory
        .load_skill_state(WEB_PUSH_DELIVERY_RECEIPTS_SKILL, owner_did)
        .await?;
    let Some(raw) = raw else {
        return Ok(WebPushDeliveryReceiptState {
            schema_version: 1,
            owner_did: owner_did.to_string(),
            receipts: Vec::new(),
        });
    };
    let mut state: WebPushDeliveryReceiptState = serde_json::from_str(&raw)?;
    if state.schema_version != 1 || state.owner_did != owner_did {
        state = WebPushDeliveryReceiptState {
            schema_version: 1,
            owner_did: owner_did.to_string(),
            receipts: Vec::new(),
        };
    }
    Ok(state)
}

async fn web_push_send_subscription(
    client: &IsahcWebPushClient,
    private_key: &WebPushVapidPrivateKey,
    subject: &str,
    subscription: &WebPushSubscriptionRecord,
    notice: &WebPushNotice,
) -> Result<(), WebPushSendError> {
    let subscription_info = SubscriptionInfo::new(
        subscription.endpoint.clone(),
        subscription.keys.p256dh.clone(),
        subscription.keys.auth.clone(),
    );
    let mut sig_builder = match private_key {
        WebPushVapidPrivateKey::RawBase64(value) => {
            VapidSignatureBuilder::from_base64(value, &subscription_info)
                .map_err(web_push_send_setup_error)?
        }
        WebPushVapidPrivateKey::Pem(value) => VapidSignatureBuilder::from_pem(
            std::io::Cursor::new(value.as_bytes()),
            &subscription_info,
        )
        .map_err(web_push_send_setup_error)?,
    };
    sig_builder.add_claim("sub", subject.to_string());
    let mut builder = WebPushMessageBuilder::new(&subscription_info);
    let payload = serde_json::to_vec(&serde_json::json!({
        "schema_version": 1,
        "source": "aegis",
        "title": notice.title,
        "body": notice.body,
        "tag": notice.tag,
        "url": notice.url,
    }))
    .map_err(web_push_send_other_error)?;
    builder.set_payload(ContentEncoding::Aes128Gcm, &payload);
    builder.set_vapid_signature(sig_builder.build().map_err(web_push_send_setup_error)?);
    let message = builder.build().map_err(web_push_send_setup_error)?;
    tokio::time::timeout(Duration::from_secs(20), client.send(message))
        .await
        .map_err(|_| WebPushSendError {
            expired_subscription: false,
            message: "Web Push send timed out".into(),
        })?
        .map_err(web_push_send_delivery_error)?;
    Ok(())
}

fn web_push_send_setup_error(err: WebPushError) -> WebPushSendError {
    WebPushSendError {
        expired_subscription: false,
        message: err.to_string(),
    }
}

fn web_push_send_delivery_error(err: WebPushError) -> WebPushSendError {
    WebPushSendError {
        expired_subscription: matches!(
            err,
            WebPushError::EndpointNotFound(_) | WebPushError::EndpointNotValid(_)
        ),
        message: err.to_string(),
    }
}

fn web_push_send_other_error(err: impl std::fmt::Display) -> WebPushSendError {
    WebPushSendError {
        expired_subscription: false,
        message: err.to_string(),
    }
}

#[cfg(test)]
mod status_rails_tests {
    use super::*;
    use crate::telegram_health::{
        TelegramHealthSnapshot, TelegramState, UNREACHABLE_DEGRADED_AFTER_SECS,
    };

    fn snapshot(state: TelegramState, since_unix: i64) -> TelegramHealthSnapshot {
        TelegramHealthSnapshot {
            state,
            since_unix,
            bot_username: Some("aegis_bot".into()),
            last_error: matches!(
                state,
                TelegramState::InvalidToken | TelegramState::Unreachable
            )
            .then(|| "A Telegram's error: Invalid bot token".to_string()),
            consecutive_failures: 0,
        }
    }

    fn empty_projection() -> UnifiedInboxProjection {
        UnifiedInboxProjection {
            schema_version: 1,
            generated_at: "2026-09-16T00:00:00Z".into(),
            owner: "owner".into(),
            sources: Vec::new(),
            counts: UnifiedInboxCounts::default(),
            items: Vec::new(),
        }
    }

    #[test]
    fn telegram_rail_carries_state_since_error_and_the_server_verdict() {
        let now = 1_789_700_000;
        let rail = vnext_telegram_rail(&snapshot(TelegramState::InvalidToken, now - 5), now);
        assert_eq!(rail.state, TelegramState::InvalidToken);
        assert_eq!(rail.since_unix, now - 5);
        assert_eq!(
            rail.last_error.as_deref(),
            Some("A Telegram's error: Invalid bot token")
        );
        assert!(rail.degraded, "an invalid token is degraded immediately");

        // Negative controls: reachable is never degraded; a fresh outage is a
        // blip, an old one is degraded.
        let rail = vnext_telegram_rail(&snapshot(TelegramState::Reachable, now - 5), now);
        assert!(!rail.degraded);
        assert_eq!(rail.last_error, None);
        let fresh = vnext_telegram_rail(&snapshot(TelegramState::Unreachable, now - 30), now);
        assert!(!fresh.degraded);
        let old = vnext_telegram_rail(
            &snapshot(
                TelegramState::Unreachable,
                now - UNREACHABLE_DEGRADED_AFTER_SECS - 1,
            ),
            now,
        );
        assert!(old.degraded);
    }

    #[test]
    fn status_response_is_flat_and_additive() {
        let now = 1_789_700_000;
        let projection = empty_projection();
        let status = vnext_status_projection(&projection, "en");
        let response = VNextStatusResponse {
            projection: status,
            telegram_rail: vnext_telegram_rail(
                &snapshot(TelegramState::InvalidToken, now - 5),
                now,
            ),
            nightly: Some(NightlyStatus {
                verdict: "RED".into(),
                sha: "02c0d462".into(),
                finished_at_unix: now - 3600,
                failed_steps: vec!["cargo test".into()],
                report: None,
            }),
        };
        let json = serde_json::to_value(&response).unwrap();

        // Every frozen key is still at the top level (flatten, not nesting).
        for key in [
            "schema_version",
            "generated_at",
            "sections",
            "system_events",
            "advanced_diagnostics_available",
            "llm_degraded",
            "embedder_degraded",
        ] {
            assert!(
                json.get(key).is_some(),
                "{key} missing from flat response: {json}"
            );
        }
        assert!(
            json.get("projection").is_none(),
            "flatten must not nest: {json}"
        );

        let rail = &json["telegram_rail"];
        assert_eq!(
            rail["state"], "invalid_token",
            "snake_case wire state: {rail}"
        );
        assert_eq!(rail["since_unix"], now - 5);
        assert_eq!(rail["last_error"], "A Telegram's error: Invalid bot token");
        assert_eq!(rail["degraded"], true);

        let nightly = &json["nightly"];
        assert_eq!(nightly["verdict"], "RED");
        assert_eq!(nightly["sha"], "02c0d462");
        assert_eq!(nightly["finished_at_unix"], now - 3600);
        assert_eq!(nightly["failed_steps"], serde_json::json!(["cargo test"]));
        assert!(nightly["report"].is_null());
    }

    #[test]
    fn status_response_serialises_nulls_for_absent_signals() {
        let now = 1_789_700_000;
        let response = VNextStatusResponse {
            projection: vnext_status_projection(&empty_projection(), "zh"),
            telegram_rail: vnext_telegram_rail(&snapshot(TelegramState::Unconfigured, now), now),
            nightly: None,
        };
        let json = serde_json::to_value(&response).unwrap();
        // Explicit nulls, never missing keys: the console treats a missing
        // key as "old server", a null as "healthy / no nightly yet".
        assert!(json["nightly"].is_null());
        assert!(json.get("nightly").is_some());
        assert_eq!(json["telegram_rail"]["state"], "unconfigured");
        assert!(json["telegram_rail"]["last_error"].is_null());
        assert_eq!(json["telegram_rail"]["degraded"], false);
    }
}

#[cfg(test)]
mod retired_search_projection_tests {
    use super::*;
    #[test]
    fn unsupported_saved_search_is_not_projected_as_auto() {
        let settings = RuntimeSettings::new();
        settings
            .set("web_search_provider", serde_json::json!("duckduckgo"))
            .unwrap();
        let (provider, model, options) = web_search_projection(Some(&settings));
        assert_eq!(provider, "duckduckgo");
        assert!(model.is_empty() && options.is_empty());
    }
}
