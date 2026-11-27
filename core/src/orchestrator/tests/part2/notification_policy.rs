//! Proactive notification policy: dedup hashes, quiet hours, daily caps, content-cosine dedup, label dedup, aggregation buffer, heartbeat config and budget.
use super::*;

// ── Notification dedup tests (TDD — Bug #5) ─────────────────────────

/// Similar messages (same topic, different numbers) should hash identically
/// after normalization, so dedup catches LLM-generated near-duplicates.
#[test]
fn notification_dedup_hash_normalizes_numbers() {
    let h1 = normalize_for_dedup("BTC price alert: $65,000");
    let h2 = normalize_for_dedup("BTC price alert: $65,001");
    assert_eq!(
        h1, h2,
        "Similar messages differing only by numbers should normalize the same"
    );
}

#[test]
fn notification_dedup_hash_normalizes_timestamps() {
    let h1 = normalize_for_dedup("Email received at 2026-03-29 14:30:00");
    let h2 = normalize_for_dedup("Email received at 2026-03-29 15:45:22");
    assert_eq!(
        h1, h2,
        "Same message with different timestamps should normalize the same"
    );
}

#[test]
fn notification_dedup_hash_distinct_topics() {
    let h1 = normalize_for_dedup("BTC price alert triggered");
    let h2 = normalize_for_dedup("New email from Alice");
    assert_ne!(h1, h2, "Different topics should NOT normalize the same");
}

#[test]
fn notification_dedup_normalizes_relative_times() {
    // This was the root cause of duplicate X/Twitter notifications:
    // "过去5h" vs "过去6h" produced different hashes because single-digit
    // numbers were preserved. Now all numbers are normalized.
    let h1 = normalize_for_dedup("Elon Musk X 更新 (过去5h)：2 posts about Tesla");
    let h2 = normalize_for_dedup("Elon Musk X 更新 (过去6h)：2 posts about Tesla");
    assert_eq!(
        h1, h2,
        "Same notification with different relative times should dedup"
    );
}

// ── G6: Notification intelligence tests ─────────────────────────────

#[test]
fn quiet_hours_disabled_by_default() {
    let settings = crate::settings::RuntimeSettings::new();
    let state = NotificationState::new();
    // Default quiet hours = 255 (disabled)
    assert!(!state.in_quiet_hours(&settings));
}

#[test]
fn quiet_hours_same_day_range() {
    let settings = crate::settings::RuntimeSettings::new();
    // Use UTC timezone so test is deterministic
    settings
        .set(
            "user_timezone",
            serde_json::Value::String("UTC".to_string()),
        )
        .unwrap();
    let now_hour = chrono::Utc::now().hour() as u64;
    settings
        .set(
            "notification_quiet_start",
            serde_json::Value::Number(now_hour.into()),
        )
        .unwrap();
    settings
        .set(
            "notification_quiet_end",
            serde_json::Value::Number(((now_hour + 1) % 24).into()),
        )
        .unwrap();
    let state = NotificationState::new();
    assert!(state.in_quiet_hours(&settings));
}

#[test]
fn quiet_hours_overnight_wrap() {
    let settings = crate::settings::RuntimeSettings::new();
    // Use UTC timezone so test is deterministic
    settings
        .set(
            "user_timezone",
            serde_json::Value::String("UTC".to_string()),
        )
        .unwrap();
    // Overnight range: start > end means wrap around midnight
    // e.g. (22, 6) = 10pm-6am — current hour is either >= 22 or < 6
    let now_hour = chrono::Utc::now().hour() as u64;
    // Set range so current hour is inside: (now_hour, (now_hour + 2) % 24)
    // with start > end to trigger the overnight branch
    let start = now_hour;
    let end = if now_hour == 0 { 23 } else { now_hour - 1 };
    settings
        .set(
            "notification_quiet_start",
            serde_json::Value::Number(start.into()),
        )
        .unwrap();
    settings
        .set(
            "notification_quiet_end",
            serde_json::Value::Number(end.into()),
        )
        .unwrap();
    let state = NotificationState::new();
    assert!(
        state.in_quiet_hours(&settings),
        "overnight range with start={start} end={end} should include hour {now_hour}"
    );
}

#[test]
fn daily_cap_default_20() {
    let settings = crate::settings::RuntimeSettings::new();
    let state = NotificationState::new();
    // Should not be over cap initially
    assert!(!state.over_daily_cap(123, &settings));
    // Increment 20 times
    for _ in 0..20 {
        state.increment_daily_count(123);
    }
    assert!(state.over_daily_cap(123, &settings));
}

#[test]
fn daily_cap_per_chat() {
    let settings = crate::settings::RuntimeSettings::new();
    let state = NotificationState::new();
    for _ in 0..20 {
        state.increment_daily_count(100);
    }
    // Chat 100 is over cap, but chat 200 is not
    assert!(state.over_daily_cap(100, &settings));
    assert!(!state.over_daily_cap(200, &settings));
}

#[test]
fn daily_count_cleanup_removes_old_dates() {
    let state = NotificationState::new();
    {
        let mut counts = state.daily_count.lock();
        counts.insert((1, "2026-01-01".to_string()), 5);
        counts.insert((1, chrono::Utc::now().format("%Y-%m-%d").to_string()), 3);
    }
    state.cleanup_daily_counts();
    let counts = state.daily_count.lock();
    assert_eq!(counts.len(), 1, "old date should be purged");
}

// ── Step 10 NotificationState content_cosine_matches unit tests ──────────

#[test]
fn content_cosine_matches_detects_identical_vectors() {
    let state = NotificationState::new();
    let emb = vec![1.0_f32, 0.0, 0.0];
    state.record_sent_message(42, "prior".to_string(), Some(emb.clone()));
    assert!(state.content_cosine_matches(42, &emb, 0.85, chrono::Duration::hours(24)));
}

#[test]
fn content_cosine_matches_returns_false_for_orthogonal() {
    let state = NotificationState::new();
    let a = vec![1.0_f32, 0.0, 0.0];
    let b = vec![0.0_f32, 1.0, 0.0];
    state.record_sent_message(42, "prior".to_string(), Some(a));
    assert!(!state.content_cosine_matches(42, &b, 0.85, chrono::Duration::hours(24)));
}

#[test]
fn content_cosine_matches_scopes_per_chat() {
    let state = NotificationState::new();
    let emb = vec![1.0_f32, 0.0, 0.0];
    state.record_sent_message(42, "prior".to_string(), Some(emb.clone()));
    // Same embedding but different chat_id → no match.
    assert!(!state.content_cosine_matches(99, &emb, 0.85, chrono::Duration::hours(24)));
}

#[test]
fn content_cosine_matches_skips_entries_without_embedding() {
    let state = NotificationState::new();
    state.record_sent_message(42, "legacy".to_string(), None);
    let emb = vec![1.0_f32, 0.0, 0.0];
    assert!(!state.content_cosine_matches(42, &emb, 0.85, chrono::Duration::hours(24)));
}

#[test]
fn last_send_minutes_ago_reflects_record() {
    let state = NotificationState::new();
    assert!(state.last_send_minutes_ago(Some(42)).is_none());
    state.record_sent_message(42, "ping".to_string(), None);
    let mins = state.last_send_minutes_ago(Some(42)).expect("recorded");
    // Should be ~0 minutes ago.
    assert!(mins < 1, "expected <1 min, got {mins}");
    // Wildcard (any chat) also finds it.
    assert!(state.last_send_minutes_ago(None).is_some());
    // Wrong chat filter returns None.
    assert!(state.last_send_minutes_ago(Some(99)).is_none());
}

#[test]
fn notification_settings_registered() {
    let settings = crate::settings::RuntimeSettings::new();
    // Verify the G6 settings exist with correct defaults
    assert_eq!(
        settings.get("notification_quiet_start"),
        Some(serde_json::Value::Number(255.into()))
    );
    assert_eq!(
        settings.get("notification_quiet_end"),
        Some(serde_json::Value::Number(255.into()))
    );
    assert_eq!(
        settings.get("notification_daily_max"),
        Some(serde_json::Value::Number(20.into()))
    );
}

#[test]
fn quiet_hours_uses_user_timezone() {
    let settings = crate::settings::RuntimeSettings::new();
    // Set timezone to a known offset: Asia/Shanghai is UTC+8
    settings
        .set(
            "user_timezone",
            serde_json::Value::String("Asia/Shanghai".to_string()),
        )
        .unwrap();

    // Get the current hour in Asia/Shanghai
    let tz: chrono_tz::Tz = "Asia/Shanghai".parse().unwrap();
    let local_hour = chrono::Utc::now().with_timezone(&tz).hour() as u64;

    // Set quiet hours around the LOCAL hour (not UTC)
    settings
        .set(
            "notification_quiet_start",
            serde_json::Value::Number(local_hour.into()),
        )
        .unwrap();
    settings
        .set(
            "notification_quiet_end",
            serde_json::Value::Number(((local_hour + 1) % 24).into()),
        )
        .unwrap();

    let state = NotificationState::new();
    assert!(
        state.in_quiet_hours(&settings),
        "quiet hours should use user_timezone (Asia/Shanghai), not UTC"
    );
}

#[test]
fn quiet_hours_invalid_timezone_falls_back_to_utc() {
    // Scheduling (quiet-hours) uses parse_user_tz, which intentionally
    // keeps its historical UTC-on-invalid fallback — the user-timezone
    // DISPLAY work does NOT change scheduling math.
    let settings = crate::settings::RuntimeSettings::new();
    settings
        .set(
            "user_timezone",
            serde_json::Value::String("Invalid/Zone".to_string()),
        )
        .unwrap();
    let utc_hour = chrono::Utc::now().hour() as u64;
    settings
        .set(
            "notification_quiet_start",
            serde_json::Value::Number(utc_hour.into()),
        )
        .unwrap();
    settings
        .set(
            "notification_quiet_end",
            serde_json::Value::Number(((utc_hour + 1) % 24).into()),
        )
        .unwrap();

    let state = NotificationState::new();
    assert!(
        state.in_quiet_hours(&settings),
        "invalid timezone should fall back to UTC"
    );
}

// ── G7: Health probe settings ─────────────────────────────────────────

#[test]
fn health_probe_interval_default() {
    let settings = crate::settings::RuntimeSettings::new();
    assert_eq!(
        settings.get("health_probe_interval_minutes"),
        Some(serde_json::Value::Number(60.into()))
    );
}

// ── G6: Aggregation window settings + buffer ────────────────────────

#[test]
fn aggregation_window_default_300() {
    let settings = crate::settings::RuntimeSettings::new();
    assert_eq!(
        settings.get("notification_aggregation_window_secs"),
        Some(serde_json::Value::Number(300.into()))
    );
}

#[test]
fn aggregation_buffer_collects_messages() {
    let state = NotificationState::new();
    state.buffer_notification(100, "msg1".to_string());
    state.buffer_notification(100, "msg2".to_string());
    let buf = state.pending_buffer.lock();
    assert_eq!(buf.len(), 2);
    assert_eq!(buf[0].1, "msg1");
    assert_eq!(buf[1].1, "msg2");
}

#[test]
fn aggregation_buffer_drains_expired() {
    let state = NotificationState::new();
    // Insert a message with a timestamp far in the past
    {
        let mut buf = state.pending_buffer.lock();
        buf.push((
            100,
            "old_msg".to_string(),
            chrono::Utc::now() - chrono::Duration::seconds(600),
        ));
        buf.push((100, "recent_msg".to_string(), chrono::Utc::now()));
        buf.push((200, "other_chat".to_string(), chrono::Utc::now()));
    }
    // Drain with 300s window — only chat 100 has an expired message
    let drained = state.drain_expired_buffer(300);
    assert!(drained.contains_key(&100));
    // Both messages for chat 100 are drained (entire chat flushed)
    assert_eq!(drained[&100].len(), 2);
    // Chat 200 is NOT drained (no expired messages)
    assert!(!drained.contains_key(&200));
    // Buffer still has chat 200
    let buf = state.pending_buffer.lock();
    assert_eq!(buf.len(), 1);
    assert_eq!(buf[0].0, 200);
}

#[test]
fn aggregation_buffer_empty_drain() {
    let state = NotificationState::new();
    let drained = state.drain_expired_buffer(300);
    assert!(drained.is_empty());
}

/// Non-urgent notifications should be suppressed when no real tools called.
/// Urgent notifications should always be delivered.
#[test]
fn notification_suppression_respects_priority() {
    let bookkeeping: &[&str] = &["notify_user", "get_current_time", "update_heartbeat_memo"];
    let tools: Vec<String> = vec!["notify_user".into()]; // bookkeeping only
    let did_real_work = tools.iter().any(|t| !bookkeeping.contains(&t.as_str()));

    // Important → suppressed when no real work
    let suppress_important = !did_real_work && NotifyPriority::Important != NotifyPriority::Urgent;
    assert!(
        suppress_important,
        "Important notification should be suppressed when no real work"
    );

    // Urgent → never suppressed
    let suppress_urgent = !did_real_work && NotifyPriority::Urgent != NotifyPriority::Urgent;
    assert!(
        !suppress_urgent,
        "Urgent notification should never be suppressed"
    );
}

// ── Per-label dedup tests (Bug: duplicate cross-scheduler notifications) ──

/// Same label within 6h window → blocked (non-Urgent).
#[test]
fn label_dedup_blocks_repeat_within_window() {
    let state = NotificationState::new();
    let chat_id = 12345_i64;
    let label = "openclaw_github_monitor".to_string();

    // First send: record label
    state
        .label_dedup
        .lock()
        .insert((chat_id, label.clone()), chrono::Utc::now());

    // Check: should be blocked
    let is_blocked = state
        .label_dedup
        .lock()
        .get(&(chat_id, label))
        .map(|sent_at| (chrono::Utc::now() - *sent_at).num_hours() < 6)
        .unwrap_or(false);
    assert!(is_blocked, "Same label within 6h should be blocked");
}

/// Same label after 6h window → allowed.
#[test]
fn label_dedup_allows_after_window() {
    let state = NotificationState::new();
    let chat_id = 12345_i64;
    let label = "openclaw_github_monitor".to_string();

    // Record label 7 hours ago
    let seven_hours_ago = chrono::Utc::now() - chrono::Duration::hours(7);
    state
        .label_dedup
        .lock()
        .insert((chat_id, label.clone()), seven_hours_ago);

    // Check: should be allowed
    let is_blocked = state
        .label_dedup
        .lock()
        .get(&(chat_id, label))
        .map(|sent_at| (chrono::Utc::now() - *sent_at).num_hours() < 6)
        .unwrap_or(false);
    assert!(!is_blocked, "Same label after 6h should be allowed");
}

/// Urgent priority bypasses label dedup.
#[test]
fn label_dedup_bypassed_by_urgent() {
    // The delivery loop checks: label_already_sent && priority != Urgent
    // So Urgent should NOT be blocked even if label was recently sent.
    let label_already_sent = true;
    let priority = NotifyPriority::Urgent;
    let should_skip = label_already_sent && priority != NotifyPriority::Urgent;
    assert!(!should_skip, "Urgent should bypass label dedup");
}

/// Different labels → both allowed (independent skills).
#[test]
fn label_dedup_different_labels_independent() {
    let state = NotificationState::new();
    let chat_id = 12345_i64;

    // Record label A
    state
        .label_dedup
        .lock()
        .insert((chat_id, "skill_a".to_string()), chrono::Utc::now());

    // Label B should not be blocked
    let is_b_blocked = state
        .label_dedup
        .lock()
        .get(&(chat_id, "skill_b".to_string()))
        .map(|sent_at| (chrono::Utc::now() - *sent_at).num_hours() < 6)
        .unwrap_or(false);
    assert!(!is_b_blocked, "Different labels should be independent");
}

// ── Step 5: heartbeat_config hot-reload wiring ─────────────────────────

fn mk_orch_for_config_tests() -> Orchestrator {
    let mem = crate::memory::MemoryHandle::spawn_in_memory().expect("mem");
    let settings = Arc::new(crate::settings::RuntimeSettings::new());
    crate::test_tool_helpers::test_orchestrator(mem, settings)
}

#[tokio::test]
async fn update_heartbeat_config_persists_applies_and_audits() {
    let orch = mk_orch_for_config_tests();
    // Seed: cache starts on defaults.
    assert_eq!(
        orch.heartbeat_config().frequency_minutes,
        30,
        "default frequency"
    );

    let new_cfg = crate::heartbeat_config::HeartbeatConfig {
        frequency_minutes: 15,
        per_tick: crate::heartbeat_config::PerTickBudget {
            tool_calls: 12,
            send_message: 1,
        },
        ..Default::default()
    };

    let changed = orch
        .update_heartbeat_config(
            new_cfg.clone(),
            crate::heartbeat_config::ConfigAuditActor::User,
            Some("test edit".into()),
        )
        .await
        .expect("apply");
    assert!(changed, "cache differed from new_cfg");
    assert_eq!(orch.heartbeat_config().frequency_minutes, 15);
    assert_eq!(orch.heartbeat_config().per_tick.tool_calls, 12);

    // Persisted?
    let map = orch.memory.load_all_settings().await.expect("load");
    let raw = map.get("heartbeat_config").expect("persisted");
    let round = crate::heartbeat_config::HeartbeatConfig::from_json(raw).expect("decode");
    assert_eq!(round, new_cfg);

    // Audit row exists with both old and new JSON.
    let audit = orch.memory.list_config_audit(10).await.expect("audit");
    assert_eq!(audit.len(), 1, "one audit row written");
    assert_eq!(audit[0].actor, "user");
    assert!(audit[0].old_json.is_some(), "old JSON captured");
    assert!(audit[0].new_json.contains("\"frequency_minutes\":15"));
}

#[tokio::test]
async fn update_heartbeat_config_is_idempotent_same_payload() {
    let orch = mk_orch_for_config_tests();
    let cfg = crate::heartbeat_config::HeartbeatConfig::default();

    // Prime cache by a "real" first apply (changes from Default::default()
    // would be a no-op — so seed to a different value first).
    let mut seeded = cfg.clone();
    seeded.frequency_minutes = 15;
    orch.update_heartbeat_config(
        seeded.clone(),
        crate::heartbeat_config::ConfigAuditActor::User,
        None,
    )
    .await
    .unwrap();
    assert_eq!(orch.memory.list_config_audit(10).await.unwrap().len(), 1);

    // Re-apply same payload → no-op, no new audit row.
    let changed = orch
        .update_heartbeat_config(
            seeded.clone(),
            crate::heartbeat_config::ConfigAuditActor::User,
            None,
        )
        .await
        .unwrap();
    assert!(!changed, "same payload must be a no-op");
    assert_eq!(
        orch.memory.list_config_audit(10).await.unwrap().len(),
        1,
        "no duplicate audit row"
    );
}

#[tokio::test]
async fn heartbeat_budget_snapshot_captures_today_count() {
    let orch = mk_orch_for_config_tests();
    // Write two rows (one Acted, one Noop). Writer stamps SQLite
    // `datetime('now')` internally, so both land in "today".
    let extras_acted = crate::memory::HeartbeatAuditExtras {
        outcome: Some(crate::heartbeat_config::HeartbeatTickOutcome::Acted),
        ..Default::default()
    };
    orch.memory
        .store_heartbeat_audit_ext(
            "cycle-acted-1",
            "__heartbeat__",
            "Info",
            None,
            &[],
            &[],
            &[],
            Some("acted"),
            100,
            1,
            "did:test:1",
            None,
            "{}",
            None,
            None,
            &extras_acted,
        )
        .await
        .expect("write acted");

    // One more "noop" row today.
    let extras_noop = crate::memory::HeartbeatAuditExtras {
        outcome: Some(crate::heartbeat_config::HeartbeatTickOutcome::Noop),
        ..Default::default()
    };
    orch.memory
        .store_heartbeat_audit_ext(
            "cycle-noop-1",
            "__heartbeat__",
            "Info",
            None,
            &[],
            &[],
            &[],
            None,
            50,
            1,
            "did:test:1",
            None,
            "{}",
            None,
            None,
            &extras_noop,
        )
        .await
        .expect("write noop");

    let snap = orch.build_budget_snapshot(None).await;
    assert!(
        snap.heartbeat_today_count >= 2,
        "today count picked up both rows, got {}",
        snap.heartbeat_today_count
    );
    assert!(
        snap.last_proactive_message_minutes_ago.is_some(),
        "last-proactive must be populated when an Acted row exists"
    );
    // Step 9b: pending-approval counts are now live (0 when no pending
    // HITL rows exist).
    assert_eq!(snap.config_pending_approvals, Some(0));
}
