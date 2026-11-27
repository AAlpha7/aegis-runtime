use super::*;
use serde_json::json;

// ── Validation tests ────────────────────────────────────────────────

#[test]
fn validate_time_intent_ok() {
    let args = json!({
        "trigger_type": "time",
        "delay_seconds": 60,
        "execution_mode": "notify",
        "context_text": "Time for your meeting"
    });
    let result = validate_intent_request(&args).unwrap();
    assert_eq!(result.trigger_type, "time");
    assert!(result.trigger_at_unix.is_some());
    assert!(result.event_key.is_none());
    assert_eq!(result.execution_mode, "notify");
}

#[test]
fn validate_event_intent_ok() {
    let args = json!({
        "trigger_type": "event",
        "event_key": "email:new:primary",
        "execution_mode": "llm_callback",
        "context_text": "Skill proposal #17 is ready for final approval"
    });
    let result = validate_intent_request(&args).unwrap();
    assert_eq!(result.trigger_type, "event");
    assert!(result.trigger_at_unix.is_none());
    assert_eq!(result.event_key.as_deref(), Some("email:new:primary"));
    assert_eq!(result.execution_mode, "llm_callback");
}

#[test]
fn validate_rejects_missing_context() {
    let args = json!({"trigger_type": "time", "delay_seconds": 60});
    assert!(validate_intent_request(&args).is_err());
}

#[test]
fn validate_rejects_short_delay() {
    let args = json!({
        "trigger_type": "time",
        "delay_seconds": 5,
        "context_text": "too soon"
    });
    let err = validate_intent_request(&args).unwrap_err();
    assert!(err.to_string().contains("too short"));
}

#[test]
fn validate_rejects_long_delay() {
    let args = json!({
        "trigger_type": "time",
        "delay_seconds": 365 * 86400 + 1,
        "context_text": "too far"
    });
    let err = validate_intent_request(&args).unwrap_err();
    assert!(err.to_string().contains("too long"));
}

#[test]
fn validate_rejects_missing_event_key() {
    let args = json!({
        "trigger_type": "event",
        "context_text": "waiting for something"
    });
    let err = validate_intent_request(&args).unwrap_err();
    assert!(err.to_string().contains("event_key"));
}

#[test]
fn validate_rejects_invalid_trigger_type() {
    let args = json!({
        "trigger_type": "magic",
        "context_text": "nope"
    });
    let err = validate_intent_request(&args).unwrap_err();
    assert!(err.to_string().contains("Invalid trigger_type"));
}

#[test]
fn validate_rejects_invalid_execution_mode() {
    let args = json!({
        "trigger_type": "time",
        "delay_seconds": 60,
        "execution_mode": "auto_send",
        "context_text": "nope"
    });
    let err = validate_intent_request(&args).unwrap_err();
    assert!(err.to_string().contains("Invalid execution_mode"));
}

#[test]
fn validate_rejects_long_context() {
    let long_text = "x".repeat(MAX_CONTEXT_LEN + 1);
    let args = json!({
        "trigger_type": "time",
        "delay_seconds": 60,
        "context_text": long_text
    });
    let err = validate_intent_request(&args).unwrap_err();
    assert!(err.to_string().contains("too long"));
}

#[test]
fn validate_context_length_uses_chars_not_bytes() {
    // 2000 Chinese characters = 6000 bytes. With byte-based check this would
    // be rejected; with char-based check it should pass.
    let chinese_text = "\u{4f60}".repeat(MAX_CONTEXT_LEN);
    assert_eq!(chinese_text.len(), MAX_CONTEXT_LEN * 3); // 6000 bytes
    assert_eq!(chinese_text.chars().count(), MAX_CONTEXT_LEN); // 2000 chars

    let args = json!({
        "trigger_type": "time",
        "delay_seconds": 60,
        "context_text": chinese_text
    });
    // Should pass — exactly at the char limit
    assert!(validate_intent_request(&args).is_ok());

    // One more char should fail
    let too_long = "\u{4f60}".repeat(MAX_CONTEXT_LEN + 1);
    let args2 = json!({
        "trigger_type": "time",
        "delay_seconds": 60,
        "context_text": too_long
    });
    assert!(
        validate_intent_request(&args2)
            .unwrap_err()
            .to_string()
            .contains("too long")
    );
}

#[test]
fn validate_defaults_to_llm_callback() {
    let args = json!({
        "trigger_type": "time",
        "delay_seconds": 60,
        "context_text": "reminder"
    });
    let result = validate_intent_request(&args).unwrap();
    assert_eq!(result.execution_mode, "llm_callback");
}

#[test]
fn callback_detection_works() {
    assert!(is_callback_context("[INTENT_CALLBACK] do something"));
    assert!(is_callback_context("[INTENT_CALLBACK]"));
    assert!(!is_callback_context("normal message"));
    assert!(!is_callback_context(""));
}

// ── Memory integration tests ────────────────────────────────────────

#[tokio::test]
async fn store_and_retrieve_time_intent() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let now = chrono::Utc::now().timestamp();
    let id = mem
        .store_deferred_intent(
            "did:test:1",
            100,
            "time",
            Some(now - 10),
            None,
            "notify",
            "hello",
            now + 3600,
            false,
            None,
            0,
            false,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();
    assert!(id > 0);

    let due = mem.get_due_time_intents(now).await.unwrap();
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].id, id);
    assert_eq!(due[0].context_text, "hello");
    assert_eq!(due[0].trigger_type, "time");
}

#[tokio::test]
async fn store_and_match_event_intent() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let now = chrono::Utc::now().timestamp();
    let id = mem
        .store_deferred_intent(
            "did:test:1",
            100,
            "event",
            None,
            Some("email:new:work"),
            "llm_callback",
            "skill ready",
            now + 3600,
            false,
            None,
            0,
            false,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

    let matches = mem
        .get_matching_event_intents("email:new:work", "did:test:1")
        .await
        .unwrap();
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].id, id);

    // Non-matching key
    let no_match = mem
        .get_matching_event_intents("email:new:archive", "did:test:1")
        .await
        .unwrap();
    assert!(no_match.is_empty());
}

#[tokio::test]
async fn mark_fired_removes_from_due() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let now = chrono::Utc::now().timestamp();
    let id = mem
        .store_deferred_intent(
            "did:test:1",
            100,
            "time",
            Some(now - 10),
            None,
            "notify",
            "fire me",
            now + 3600,
            false,
            None,
            0,
            false,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

    mem.mark_intent_fired(id).await.unwrap();
    let due = mem.get_due_time_intents(now).await.unwrap();
    assert!(due.is_empty());
}

#[tokio::test]
async fn cancel_intent_works() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let now = chrono::Utc::now().timestamp();
    let id = mem
        .store_deferred_intent(
            "did:test:1",
            100,
            "time",
            Some(now + 60),
            None,
            "notify",
            "cancel me",
            now + 3600,
            false,
            None,
            0,
            false,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

    // Correct DID
    let cancelled = mem.cancel_intent(id, "did:test:1").await.unwrap();
    assert!(cancelled);

    // Already cancelled
    let again = mem.cancel_intent(id, "did:test:1").await.unwrap();
    assert!(!again);
}

#[tokio::test]
async fn cancel_intent_wrong_did_rejected() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let now = chrono::Utc::now().timestamp();
    let id = mem
        .store_deferred_intent(
            "did:test:1",
            100,
            "time",
            Some(now + 60),
            None,
            "notify",
            "mine",
            now + 3600,
            false,
            None,
            0,
            false,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

    let cancelled = mem.cancel_intent(id, "did:test:OTHER").await.unwrap();
    assert!(!cancelled);
}

#[tokio::test]
async fn expire_old_intents_works() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let now = chrono::Utc::now().timestamp();
    // Already expired
    mem.store_deferred_intent(
        "did:test:1",
        100,
        "time",
        Some(now - 100),
        None,
        "notify",
        "old",
        now - 10,
        false,
        None,
        0,
        false,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
    )
    .await
    .unwrap();
    // Still valid
    mem.store_deferred_intent(
        "did:test:1",
        100,
        "time",
        Some(now + 60),
        None,
        "notify",
        "new",
        now + 3600,
        false,
        None,
        0,
        false,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
    )
    .await
    .unwrap();

    let expired = mem.expire_old_intents(now).await.unwrap();
    assert_eq!(expired.len(), 1);
    assert_eq!(expired[0].1, "did:test:1"); // user_did
    assert_eq!(expired[0].2, "old"); // context_text

    let pending = mem.list_user_intents("did:test:1").await.unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].context_text, "new");
}

#[tokio::test]
async fn count_active_intents_respects_user() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let now = chrono::Utc::now().timestamp();
    for _ in 0..3 {
        mem.store_deferred_intent(
            "did:test:A",
            100,
            "time",
            Some(now + 60),
            None,
            "notify",
            "a",
            now + 3600,
            false,
            None,
            0,
            false,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();
    }
    mem.store_deferred_intent(
        "did:test:B",
        200,
        "time",
        Some(now + 60),
        None,
        "notify",
        "b",
        now + 3600,
        false,
        None,
        0,
        false,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
    )
    .await
    .unwrap();

    let count_a = mem.count_active_intents("did:test:A").await.unwrap();
    let count_b = mem.count_active_intents("did:test:B").await.unwrap();
    assert_eq!(count_a, 3);
    assert_eq!(count_b, 1);
}

#[tokio::test]
async fn list_user_intents_only_pending() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let now = chrono::Utc::now().timestamp();
    let id1 = mem
        .store_deferred_intent(
            "did:test:1",
            100,
            "time",
            Some(now - 10),
            None,
            "notify",
            "first",
            now + 3600,
            false,
            None,
            0,
            false,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();
    mem.store_deferred_intent(
        "did:test:1",
        100,
        "time",
        Some(now + 60),
        None,
        "notify",
        "second",
        now + 3600,
        false,
        None,
        0,
        false,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
    )
    .await
    .unwrap();

    // Fire the first one
    mem.mark_intent_fired(id1).await.unwrap();

    let pending = mem.list_user_intents("did:test:1").await.unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].context_text, "second");
}

#[tokio::test]
async fn count_daily_intents() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let now = chrono::Utc::now().timestamp();
    for _ in 0..5 {
        mem.store_deferred_intent(
            "did:test:1",
            100,
            "time",
            Some(now + 60),
            None,
            "notify",
            "daily",
            now + 3600,
            false,
            None,
            0,
            false,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();
    }
    let count = mem.count_daily_intents("did:test:1").await.unwrap();
    assert_eq!(count, 5);

    let other = mem.count_daily_intents("did:test:OTHER").await.unwrap();
    assert_eq!(other, 0);
}

// ── R1: Recurring intent tests ─────────────────────────────────────

#[test]
fn validate_recurring_intent_ok() {
    let args = json!({
        "trigger_type": "time",
        "delay_seconds": 60,
        "interval_seconds": 300,
        "execution_mode": "notify",
        "context_text": "Check every 5 minutes",
        "group_key": "test_check"
    });
    let result = validate_intent_request(&args).unwrap();
    assert_eq!(result.interval_seconds, Some(300));
    assert_eq!(result.group_key, Some("test_check".to_string()));
    assert!(result.trigger_at_unix.is_some());
    // Recurring intents get a 30-day expiry, not 1hr grace
    let now = chrono::Utc::now().timestamp();
    assert!(result.expires_at_unix > now + 29 * 24 * 3600);
}

#[test]
fn validate_recurring_interval_too_short() {
    let args = json!({
        "trigger_type": "time",
        "delay_seconds": 60,
        "interval_seconds": 10,
        "context_text": "too fast"
    });
    let err = validate_intent_request(&args).unwrap_err();
    assert!(err.to_string().contains("interval_seconds too short"));
}

#[test]
fn validate_recurring_interval_too_long() {
    let args = json!({
        "trigger_type": "time",
        "delay_seconds": 60,
        "interval_seconds": 365 * 86400 + 1,
        "context_text": "too slow"
    });
    let err = validate_intent_request(&args).unwrap_err();
    assert!(err.to_string().contains("interval_seconds too long"));
}

#[test]
fn validate_recurring_event_trigger_rejected() {
    let args = json!({
        "trigger_type": "event",
        "event_key": "test:event",
        "interval_seconds": 300,
        "context_text": "events cannot recur"
    });
    let err = validate_intent_request(&args).unwrap_err();
    assert!(
        err.to_string()
            .contains("only valid for trigger_type='time'")
    );
}

#[test]
fn validate_no_interval_is_none() {
    let args = json!({
        "trigger_type": "time",
        "delay_seconds": 60,
        "context_text": "one-shot"
    });
    let result = validate_intent_request(&args).unwrap();
    assert!(result.interval_seconds.is_none());
}

#[test]
fn validate_recurring_requires_group_key() {
    let args = json!({
        "trigger_type": "time",
        "delay_seconds": 60,
        "interval_seconds": 300,
        "context_text": "no group_key"
    });
    let err = validate_intent_request(&args).unwrap_err();
    assert!(
        err.to_string().contains("group_key"),
        "Expected group_key error, got: {}",
        err
    );
}

#[test]
fn validate_group_key_invalid_chars() {
    let args = json!({
        "trigger_type": "time",
        "delay_seconds": 60,
        "context_text": "test",
        "group_key": "has spaces!"
    });
    let err = validate_intent_request(&args).unwrap_err();
    assert!(
        err.to_string().contains("alphanumeric"),
        "Expected charset error, got: {}",
        err
    );
}

#[test]
fn validate_group_key_too_long() {
    let args = json!({
        "trigger_type": "time",
        "delay_seconds": 60,
        "context_text": "test",
        "group_key": "a".repeat(101)
    });
    let err = validate_intent_request(&args).unwrap_err();
    assert!(err.to_string().contains("too long"));
}

#[test]
fn validate_max_fires_zero_rejected() {
    let args = json!({
        "trigger_type": "time",
        "delay_seconds": 60,
        "context_text": "test",
        "max_fires": 0
    });
    let err = validate_intent_request(&args).unwrap_err();
    assert!(err.to_string().contains("max_fires must be >= 1"));
}

#[test]
fn validate_max_fires_ok() {
    let args = json!({
        "trigger_type": "time",
        "delay_seconds": 60,
        "interval_seconds": 300,
        "context_text": "test recurring",
        "group_key": "test_mf",
        "max_fires": 5
    });
    let result = validate_intent_request(&args).unwrap();
    assert_eq!(result.max_fires, Some(5));
    assert_eq!(result.group_key, Some("test_mf".to_string()));
}

#[test]
fn validate_one_shot_group_key_optional() {
    // One-shot intents don't require group_key
    let args = json!({
        "trigger_type": "time",
        "delay_seconds": 60,
        "context_text": "one-shot without group_key"
    });
    let result = validate_intent_request(&args).unwrap();
    assert!(result.group_key.is_none());
    assert!(result.interval_seconds.is_none());
}

#[tokio::test]
async fn group_key_upsert_cancels_previous() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let now = chrono::Utc::now().timestamp();

    // Create first intent with group_key
    let id1 = mem
        .store_deferred_intent(
            "did:test:1",
            100,
            "time",
            Some(now + 60),
            None,
            "notify",
            "first version",
            now + 3600,
            false,
            Some(300),
            0,
            false,
            Some("commute_monitor"),
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

    // Create second intent with same group_key — should cancel first
    let id2 = mem
        .store_deferred_intent(
            "did:test:1",
            100,
            "time",
            Some(now + 120),
            None,
            "notify",
            "second version",
            now + 3600,
            false,
            Some(300),
            0,
            false,
            Some("commute_monitor"),
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

    assert_ne!(id1, id2);

    // First should be cancelled
    let intents = mem.list_user_intents("did:test:1").await.unwrap();
    assert_eq!(intents.len(), 1, "Should have exactly 1 pending intent");
    assert_eq!(intents[0].id, id2);
    assert_eq!(intents[0].group_key.as_deref(), Some("commute_monitor"));
}

#[tokio::test]
async fn group_key_different_users_no_conflict() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let now = chrono::Utc::now().timestamp();

    let id1 = mem
        .store_deferred_intent(
            "did:test:alice",
            100,
            "time",
            Some(now + 60),
            None,
            "notify",
            "alice's monitor",
            now + 3600,
            false,
            Some(300),
            0,
            false,
            Some("commute_monitor"),
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

    let id2 = mem
        .store_deferred_intent(
            "did:test:bob",
            200,
            "time",
            Some(now + 60),
            None,
            "notify",
            "bob's monitor",
            now + 3600,
            false,
            Some(300),
            0,
            false,
            Some("commute_monitor"),
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

    // Both should remain — different users
    let alice = mem.list_user_intents("did:test:alice").await.unwrap();
    let bob = mem.list_user_intents("did:test:bob").await.unwrap();
    assert_eq!(alice.len(), 1);
    assert_eq!(bob.len(), 1);
    assert_eq!(alice[0].id, id1);
    assert_eq!(bob[0].id, id2);
}

#[tokio::test]
async fn null_group_key_no_dedup() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let now = chrono::Utc::now().timestamp();

    // Two intents without group_key — both should remain
    mem.store_deferred_intent(
        "did:test:1",
        100,
        "time",
        Some(now + 60),
        None,
        "notify",
        "first",
        now + 3600,
        false,
        None,
        0,
        false,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
    )
    .await
    .unwrap();

    mem.store_deferred_intent(
        "did:test:1",
        100,
        "time",
        Some(now + 120),
        None,
        "notify",
        "second",
        now + 3600,
        false,
        None,
        0,
        false,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
    )
    .await
    .unwrap();

    let intents = mem.list_user_intents("did:test:1").await.unwrap();
    assert_eq!(
        intents.len(),
        2,
        "Both intents without group_key should remain"
    );
}

#[tokio::test]
async fn list_recurring_intents() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let now = chrono::Utc::now().timestamp();

    // One recurring
    mem.store_deferred_intent(
        "did:test:1",
        100,
        "time",
        Some(now + 60),
        None,
        "notify",
        "recurring",
        now + 3600,
        false,
        Some(300),
        0,
        false,
        Some("test_recurring"),
        None,
        None,
        None,
        None,
        None,
        None,
    )
    .await
    .unwrap();

    // One one-shot
    mem.store_deferred_intent(
        "did:test:1",
        100,
        "time",
        Some(now + 60),
        None,
        "notify",
        "one-shot",
        now + 3600,
        false,
        None,
        0,
        false,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
    )
    .await
    .unwrap();

    let recurring = mem.list_recurring_intents("did:test:1").await.unwrap();
    assert_eq!(recurring.len(), 1);
    assert_eq!(recurring[0].group_key.as_deref(), Some("test_recurring"));
}

#[tokio::test]
async fn max_fires_stops_rescheduling() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let now = chrono::Utc::now().timestamp();

    // Create recurring intent with max_fires=2
    let id = mem
        .store_deferred_intent(
            "did:test:1",
            100,
            "time",
            Some(now - 10),
            None,
            "notify",
            "check twice",
            now + 86400 * 30,
            false,
            Some(60),
            0,
            false,
            Some("limited_check"),
            Some(2),
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

    // Verify it was stored with max_fires
    let intents = mem.list_user_intents("did:test:1").await.unwrap();
    assert_eq!(intents.len(), 1);
    assert_eq!(intents[0].max_fires, Some(2));
    assert_eq!(intents[0].fire_count, 0);

    // Simulate first fire + reschedule (fire_count goes to 1)
    mem.mark_intent_fired(id).await.unwrap();
    mem.reschedule_recurring_intent(id, now + 60, now + 86400 * 30)
        .await
        .unwrap();

    let intents = mem.list_user_intents("did:test:1").await.unwrap();
    assert_eq!(intents[0].fire_count, 1);
    assert_eq!(intents[0].status, "pending");

    // Second fire + reschedule (fire_count goes to 2)
    mem.mark_intent_fired(id).await.unwrap();
    mem.reschedule_recurring_intent(id, now + 120, now + 86400 * 30)
        .await
        .unwrap();

    let intents = mem.list_user_intents("did:test:1").await.unwrap();
    assert_eq!(intents[0].fire_count, 2);
    // fire_count (2) == max_fires (2) — execute_intent would NOT reschedule
}

#[tokio::test]
async fn recurring_intent_reschedule() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let now = chrono::Utc::now().timestamp();
    let id = mem
        .store_deferred_intent(
            "did:test:1",
            100,
            "time",
            Some(now - 10),
            None,
            "notify",
            "recurring check",
            now + 86400 * 30,
            false,
            Some(300),
            0,
            false,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

    // Verify it has interval_seconds
    let due = mem.get_due_time_intents(now).await.unwrap();
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].interval_seconds, Some(300));

    // Simulate firing: mark fired
    mem.mark_intent_fired(id).await.unwrap();
    let due_after_fire = mem.get_due_time_intents(now).await.unwrap();
    assert!(due_after_fire.is_empty());

    // Reschedule
    let next_trigger = now + 300;
    let next_expires = now + 86400 * 30;
    mem.reschedule_recurring_intent(id, next_trigger, next_expires)
        .await
        .unwrap();

    // Should now be pending again but not yet due
    let due_before = mem.get_due_time_intents(now).await.unwrap();
    assert!(due_before.is_empty());

    // Should be due after next_trigger
    let due_after = mem.get_due_time_intents(now + 301).await.unwrap();
    assert_eq!(due_after.len(), 1);
    assert_eq!(due_after[0].id, id);
    assert_eq!(due_after[0].status, "pending");
}

#[tokio::test]
async fn recurring_intent_cancellation() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let now = chrono::Utc::now().timestamp();
    let id = mem
        .store_deferred_intent(
            "did:test:1",
            100,
            "time",
            Some(now + 60),
            None,
            "notify",
            "cancel recurring",
            now + 86400 * 30,
            false,
            Some(300),
            0,
            false,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

    let cancelled = mem.cancel_intent(id, "did:test:1").await.unwrap();
    assert!(cancelled);

    // Should not appear in due intents even after trigger time
    let due = mem.get_due_time_intents(now + 120).await.unwrap();
    assert!(due.is_empty());
}

// ── Validate warning on group_key without interval ─────────────────

#[test]
fn validate_warns_group_key_without_interval() {
    // group_key set but interval_seconds missing → should succeed with a warning
    let args = json!({
        "trigger_type": "time",
        "delay_seconds": 60,
        "context_text": "check something once",
        "group_key": "test_check"
    });
    let result = validate_intent_request(&args).unwrap();
    assert!(
        result.warning.is_some(),
        "Expected a warning when group_key is set without interval_seconds"
    );
    let warning = result.warning.as_deref().unwrap();
    assert!(
        warning.contains("ONE-SHOT"),
        "Warning should mention ONE-SHOT, got: {}",
        warning
    );
    // Verify the intent itself is valid (Ok, not Err)
    assert!(result.interval_seconds.is_none());
    assert_eq!(result.group_key, Some("test_check".to_string()));
}

#[test]
fn validate_no_warning_when_both_present() {
    // Both group_key and interval_seconds → no warning
    let args = json!({
        "trigger_type": "time",
        "delay_seconds": 60,
        "interval_seconds": 300,
        "context_text": "recurring check",
        "group_key": "test_recurring"
    });
    let result = validate_intent_request(&args).unwrap();
    assert!(
        result.warning.is_none(),
        "No warning expected when both group_key and interval_seconds are set, got: {:?}",
        result.warning
    );
}

#[test]
fn validate_no_warning_when_neither_present() {
    // Neither group_key nor interval_seconds → standard one-shot, no warning
    let args = json!({
        "trigger_type": "time",
        "delay_seconds": 60,
        "context_text": "simple reminder"
    });
    let result = validate_intent_request(&args).unwrap();
    assert!(
        result.warning.is_none(),
        "No warning expected for plain one-shot intent, got: {:?}",
        result.warning
    );
}

// ── Atomic fire_and_advance_recurring tests ──────────────────────

#[tokio::test]
async fn fire_and_advance_recurring_keeps_status_active() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let now = chrono::Utc::now().timestamp();
    let interval = 3600i64;

    // Create a recurring intent (interval_seconds=3600)
    let id = mem
        .store_deferred_intent(
            "did:test:1",
            100,
            "time",
            Some(now - 10),
            None,
            "notify",
            "hourly check",
            now + 86400 * 30,
            false,
            Some(interval),
            0,
            false,
            Some("hourly_check"),
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

    // Verify initial state: due, status='active' (get_due_time_intents returns it)
    let due = mem.get_due_time_intents(now).await.unwrap();
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].fire_count, 0);

    // Call fire_and_advance_recurring
    let next_trigger = now + interval;
    let next_expires = now + 86400 * 30;
    mem.fire_and_advance_recurring(id, next_trigger, next_expires)
        .await
        .unwrap();

    // Verify: status stays 'active' (intent should appear with future trigger)
    // It should NOT appear as due now (trigger_at advanced to the future)
    let due_now = mem.get_due_time_intents(now).await.unwrap();
    assert!(
        due_now.is_empty(),
        "Intent should not be due immediately after advancing trigger_at"
    );

    // But it SHOULD appear as due after next_trigger
    let due_later = mem.get_due_time_intents(now + interval + 1).await.unwrap();
    assert_eq!(
        due_later.len(),
        1,
        "Intent should be due after next trigger time"
    );
    assert_eq!(due_later[0].id, id);
    // fired_at should be set (non-None)
    assert!(
        due_later[0].fired_at.is_some(),
        "fired_at should be set after fire_and_advance_recurring"
    );
    // fire_count should be incremented to 1
    assert_eq!(
        due_later[0].fire_count, 1,
        "fire_count should be incremented from 0 to 1"
    );
    // trigger_at should be advanced
    assert_eq!(
        due_later[0].trigger_at_unix,
        Some(next_trigger),
        "trigger_at_unix should be advanced to next_trigger"
    );
}

#[tokio::test]
async fn fire_and_advance_recurring_vs_mark_fired_for_one_shot() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let now = chrono::Utc::now().timestamp();

    // Create a one-shot intent (no interval_seconds)
    let id = mem
        .store_deferred_intent(
            "did:test:1",
            100,
            "time",
            Some(now - 10),
            None,
            "notify",
            "one-shot reminder",
            now + 3600,
            false,
            None,
            0,
            false,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

    // mark_intent_fired sets status='fired' (terminal for one-shot)
    mem.mark_intent_fired(id).await.unwrap();

    // Should no longer be due (status='fired', not 'active')
    let due = mem.get_due_time_intents(now).await.unwrap();
    assert!(
        due.is_empty(),
        "One-shot intent should not be due after mark_intent_fired"
    );

    // Should not appear in pending list either
    let pending = mem.list_user_intents("did:test:1").await.unwrap();
    assert!(
        pending.is_empty(),
        "mark_intent_fired sets status='fired', so it should not appear in pending list"
    );
}

#[tokio::test]
async fn recurring_last_fire_uses_mark_intent_fired() {
    // Recurring intent with max_fires=2, fire_count=1 → next fire should be terminal
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let now = chrono::Utc::now().timestamp();

    let id = mem
        .store_deferred_intent(
            "did:test:1",
            100,
            "time",
            Some(now - 10),
            None,
            "notify",
            "check twice",
            now + 86400 * 30,
            false,
            Some(60),
            0,
            false,
            Some("limited_check_v2"),
            Some(2),
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

    // Simulate first fire using fire_and_advance_recurring (fire_count: 0 -> 1)
    let next1 = now + 60;
    mem.fire_and_advance_recurring(id, next1, now + 86400 * 30)
        .await
        .unwrap();

    // Verify fire_count = 1, still active
    let due = mem.get_due_time_intents(now + 61).await.unwrap();
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].fire_count, 1);

    // For the last fire (fire_count will reach max_fires), use mark_intent_fired (terminal)
    mem.mark_intent_fired(id).await.unwrap();

    // Should no longer be due — status='fired'
    let due_after = mem.get_due_time_intents(now + 200).await.unwrap();
    assert!(
        due_after.is_empty(),
        "After mark_intent_fired on last fire, intent should be terminal (status='fired')"
    );

    // Should not appear in pending list
    let pending = mem.list_user_intents("did:test:1").await.unwrap();
    assert!(
        pending.is_empty(),
        "Terminal intent should not appear in pending list"
    );
}

// ── R2: Chain depth tests ──────────────────────────────────────────

#[test]
fn parse_callback_depth_with_depth() {
    assert_eq!(
        parse_callback_depth("[INTENT_CALLBACK:depth=3] do something"),
        Some(3)
    );
    assert_eq!(
        parse_callback_depth("[INTENT_CALLBACK:depth=0] init"),
        Some(0)
    );
    assert_eq!(
        parse_callback_depth("[INTENT_CALLBACK:depth=5] max"),
        Some(5)
    );
}

#[test]
fn parse_callback_depth_legacy_format() {
    // Legacy format without depth encoding = depth 0
    assert_eq!(
        parse_callback_depth("[INTENT_CALLBACK] old format"),
        Some(0)
    );
    assert_eq!(parse_callback_depth("[INTENT_CALLBACK]"), Some(0));
}

#[test]
fn parse_callback_depth_non_callback() {
    assert_eq!(parse_callback_depth("normal message"), None);
    assert_eq!(parse_callback_depth(""), None);
}

#[test]
fn strip_callback_prefix_with_depth() {
    assert_eq!(
        strip_callback_prefix(
            "[INTENT_CALLBACK:depth=0] \u{83b7}\u{53d6}\u{4ea4}\u{901a}\u{65f6}\u{95f4}"
        ),
        "\u{83b7}\u{53d6}\u{4ea4}\u{901a}\u{65f6}\u{95f4}"
    );
    assert_eq!(
        strip_callback_prefix("[INTENT_CALLBACK:depth=3] do something"),
        "do something"
    );
}

#[test]
fn strip_callback_prefix_legacy() {
    assert_eq!(
        strip_callback_prefix("[INTENT_CALLBACK] old format task"),
        "old format task"
    );
    assert_eq!(strip_callback_prefix("[INTENT_CALLBACK]"), "");
}

#[test]
fn strip_callback_prefix_non_callback() {
    assert_eq!(strip_callback_prefix("normal message"), "normal message");
    assert_eq!(strip_callback_prefix(""), "");
}

#[test]
fn is_callback_context_recognizes_depth_format() {
    assert!(is_callback_context("[INTENT_CALLBACK:depth=2] msg"));
    assert!(is_callback_context("[INTENT_CALLBACK] legacy"));
    assert!(!is_callback_context("normal"));
}

#[tokio::test]
async fn chain_depth_stored_and_retrieved() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let now = chrono::Utc::now().timestamp();
    let id = mem
        .store_deferred_intent(
            "did:test:1",
            100,
            "time",
            Some(now - 10),
            None,
            "llm_callback",
            "chained action",
            now + 3600,
            true,
            None,
            3,
            false,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

    let due = mem.get_due_time_intents(now).await.unwrap();
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].id, id);
    assert_eq!(due[0].chain_depth, 3);
    assert!(due[0].is_callback_origin);
}

#[tokio::test]
async fn chain_depth_zero_default_backward_compat() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let now = chrono::Utc::now().timestamp();
    let id = mem
        .store_deferred_intent(
            "did:test:1",
            100,
            "time",
            Some(now - 10),
            None,
            "notify",
            "normal intent",
            now + 3600,
            false,
            None,
            0,
            false,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

    let due = mem.get_due_time_intents(now).await.unwrap();
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].id, id);
    assert_eq!(due[0].chain_depth, 0);
    assert!(due[0].interval_seconds.is_none());
}

#[test]
fn suppress_callback_empty() {
    assert!(should_suppress_callback(""));
}

#[test]
fn suppress_callback_no_action() {
    assert!(should_suppress_callback("[NO_ACTION]"));
}

#[test]
fn suppress_callback_no_action_whitespace() {
    assert!(should_suppress_callback("  [NO_ACTION]  "));
}

#[test]
fn suppress_callback_no_action_with_extra_text() {
    // LLM sometimes wraps [NO_ACTION] with explanation text
    assert!(should_suppress_callback(
        "I checked and found nothing. [NO_ACTION]"
    ));
    assert!(should_suppress_callback(
        "[NO_ACTION]\n\nNo new emails found."
    ));
    assert!(should_suppress_callback(
        "No actionable items. [NO_ACTION] Done."
    ));
}

#[test]
fn suppress_callback_real_reply() {
    assert!(!should_suppress_callback("Hello, you have 3 new emails"));
}

#[test]
fn suppress_callback_whitespace_only() {
    assert!(should_suppress_callback("   "));
}

// ── parse_trigger_at tests ────────────────────────────────────────

#[test]
fn parse_trigger_at_iso8601_with_offset() {
    let now = chrono::DateTime::parse_from_rfc3339("2026-03-17T14:00:00-07:00")
        .unwrap()
        .timestamp();
    let result = parse_trigger_at("2026-03-17T22:00:00-07:00", None, now).unwrap();
    // 22:00 PDT = 05:00 UTC next day
    assert_eq!(
        result,
        chrono::DateTime::parse_from_rfc3339("2026-03-18T05:00:00+00:00")
            .unwrap()
            .timestamp()
    );
}

#[test]
fn parse_trigger_at_time_only_with_tz() {
    // Now is 14:10 PDT (21:10 UTC)
    let now = chrono::DateTime::parse_from_rfc3339("2026-03-17T21:10:00+00:00")
        .unwrap()
        .timestamp();
    let result = parse_trigger_at("22:00", Some("America/Los_Angeles"), now).unwrap();
    // 22:00 PDT = 05:00 UTC
    let expected = chrono::DateTime::parse_from_rfc3339("2026-03-18T05:00:00+00:00")
        .unwrap()
        .timestamp();
    assert_eq!(result, expected);
}

#[test]
fn parse_trigger_at_time_only_past_wraps_to_tomorrow() {
    // Now is 23:00 PDT, asking for 08:00 → should be tomorrow 08:00
    let now = chrono::DateTime::parse_from_rfc3339("2026-03-18T06:00:00+00:00")
        .unwrap()
        .timestamp(); // 23:00 PDT
    let result = parse_trigger_at("08:00", Some("America/Los_Angeles"), now).unwrap();
    // 08:00 PDT tomorrow = 15:00 UTC March 18
    let expected = chrono::DateTime::parse_from_rfc3339("2026-03-18T15:00:00+00:00")
        .unwrap()
        .timestamp();
    assert_eq!(result, expected);
}

#[test]
fn parse_trigger_at_date_time_no_offset() {
    let now = 0i64; // doesn't matter for absolute date+time
    let result = parse_trigger_at("2026-03-18 22:00", Some("UTC-7"), now).unwrap();
    let expected = chrono::DateTime::parse_from_rfc3339("2026-03-19T05:00:00+00:00")
        .unwrap()
        .timestamp();
    assert_eq!(result, expected);
}

#[test]
fn parse_trigger_at_invalid_format() {
    let result = parse_trigger_at("next tuesday", None, 0);
    assert!(result.is_err());
}

// ── parse_trigger_at tz-abbreviation tests (2026-04-17 fix) ────────

#[test]
fn parse_trigger_at_accepts_pdt_suffix() {
    // PDT = UTC-7 (active late-March through early-November).
    // Using 2026-04-16, which is squarely inside PDT.
    let now = chrono::DateTime::parse_from_rfc3339("2026-04-16T12:00:00+00:00")
        .unwrap()
        .timestamp();
    let result = parse_trigger_at("2026-04-16 18:00 PDT", None, now).unwrap();
    // 18:00 PDT → 01:00 UTC next day
    let expected = chrono::DateTime::parse_from_rfc3339("2026-04-17T01:00:00+00:00")
        .unwrap()
        .timestamp();
    assert_eq!(result, expected);
}

#[test]
fn parse_trigger_at_accepts_est_suffix() {
    // EST = UTC-5 (active November through early-March).
    // Using 2026-12-01, which is squarely inside EST.
    let now = chrono::DateTime::parse_from_rfc3339("2026-12-01T00:00:00+00:00")
        .unwrap()
        .timestamp();
    let result = parse_trigger_at("2026-12-01 09:00 EST", None, now).unwrap();
    // 09:00 EST = 14:00 UTC
    let expected = chrono::DateTime::parse_from_rfc3339("2026-12-01T14:00:00+00:00")
        .unwrap()
        .timestamp();
    assert_eq!(result, expected);
}

#[test]
fn parse_trigger_at_accepts_utc_suffix() {
    let now = 0i64;
    let result = parse_trigger_at("2026-04-16 18:00 UTC", None, now).unwrap();
    let expected = chrono::DateTime::parse_from_rfc3339("2026-04-16T18:00:00+00:00")
        .unwrap()
        .timestamp();
    assert_eq!(result, expected);
}

#[test]
fn parse_trigger_at_tz_abbr_case_insensitive() {
    // Lowercase variants produced by voice-dictation / casual typing.
    let now = chrono::DateTime::parse_from_rfc3339("2026-04-16T12:00:00+00:00")
        .unwrap()
        .timestamp();
    let result = parse_trigger_at("2026-04-16 18:00 pdt", None, now).unwrap();
    let expected = chrono::DateTime::parse_from_rfc3339("2026-04-17T01:00:00+00:00")
        .unwrap()
        .timestamp();
    assert_eq!(result, expected);
}

#[test]
fn parse_trigger_at_inline_abbr_beats_tz_param() {
    // When both inline abbr and `tz` param are present, inline wins
    // (user was explicit — do not silently re-interpret under caller default).
    let now = chrono::DateTime::parse_from_rfc3339("2026-04-16T12:00:00+00:00")
        .unwrap()
        .timestamp();
    // Pass UTC+8 as the fallback tz; the "PDT" suffix must still take effect.
    let result = parse_trigger_at("2026-04-16 18:00 PDT", Some("UTC+8"), now).unwrap();
    let expected = chrono::DateTime::parse_from_rfc3339("2026-04-17T01:00:00+00:00")
        .unwrap()
        .timestamp();
    assert_eq!(result, expected);
}

#[test]
fn parse_trigger_at_still_accepts_iso_and_iana() {
    // Regression: ensure the iso8601 + IANA paths still resolve correctly
    // after adding the abbr-suffix short-circuit.
    let now = chrono::DateTime::parse_from_rfc3339("2026-04-16T12:00:00+00:00")
        .unwrap()
        .timestamp();

    // ISO 8601 with offset — tz abbr path must NOT intercept.
    let r1 = parse_trigger_at("2026-04-16T18:00:00-07:00", None, now).unwrap();
    assert_eq!(
        r1,
        chrono::DateTime::parse_from_rfc3339("2026-04-17T01:00:00+00:00")
            .unwrap()
            .timestamp()
    );

    // Date+time + explicit IANA via `tz` param.
    let r2 = parse_trigger_at("2026-04-16 18:00", Some("America/Los_Angeles"), now).unwrap();
    assert_eq!(
        r2,
        chrono::DateTime::parse_from_rfc3339("2026-04-17T01:00:00+00:00")
            .unwrap()
            .timestamp()
    );
}

#[test]
fn parse_trigger_at_rejects_unknown_tz_abbr() {
    // Unknown abbreviation must NOT be silently accepted — the original
    // strict error path fires instead.
    let now = 0i64;
    let err = parse_trigger_at("2026-04-16 18:00 XYZ", None, now).unwrap_err();
    let msg = format!("{err}");
    assert!(
        msg.contains("Cannot parse trigger_at"),
        "expected the strict error, got: {msg}"
    );
}

#[test]
fn resolve_tz_offset_accepts_us_abbreviations() {
    // Abbreviations are mapped to FIXED offsets (by construction — that's
    // what makes them unambiguous vs IANA names).
    assert_eq!(resolve_tz_offset("PDT").unwrap(), -7 * 3600);
    assert_eq!(resolve_tz_offset("PST").unwrap(), -8 * 3600);
    assert_eq!(resolve_tz_offset("MDT").unwrap(), -6 * 3600);
    assert_eq!(resolve_tz_offset("MST").unwrap(), -7 * 3600);
    assert_eq!(resolve_tz_offset("CDT").unwrap(), -5 * 3600);
    assert_eq!(resolve_tz_offset("CST").unwrap(), -6 * 3600);
    assert_eq!(resolve_tz_offset("EDT").unwrap(), -4 * 3600);
    assert_eq!(resolve_tz_offset("EST").unwrap(), -5 * 3600);
    assert_eq!(resolve_tz_offset("UTC").unwrap(), 0);
    assert_eq!(resolve_tz_offset("GMT").unwrap(), 0);
    // Lowercase.
    assert_eq!(resolve_tz_offset("pdt").unwrap(), -7 * 3600);
}

// ── resolve_tz_offset tests ───────────────────────────────────────

#[test]
fn resolve_tz_offset_rejects_iana_names() {
    // A single offset for an IANA zone is only right on one side of DST, so
    // IANA names are resolved per target date in `parse_trigger_at` instead.
    assert!(resolve_tz_offset("America/Los_Angeles").is_err());
}

#[test]
fn resolve_tz_utc_minus() {
    assert_eq!(resolve_tz_offset("UTC-7").unwrap(), -25200);
    assert_eq!(resolve_tz_offset("UTC+8").unwrap(), 28800);
    assert_eq!(resolve_tz_offset("UTC").unwrap(), 0);
}

#[test]
fn resolve_tz_offset_colon() {
    assert_eq!(resolve_tz_offset("-07:00").unwrap(), -25200);
    assert_eq!(resolve_tz_offset("+05:30").unwrap(), 19800);
}

#[test]
fn resolve_tz_invalid() {
    assert!(resolve_tz_offset("Mars/Olympus").is_err());
}

// ── validate_intent_request with trigger_at ───────────────────────

#[test]
fn validate_intent_trigger_at_preferred_over_delay() {
    // Use a date ~30 days in the future (within 365-day limit)
    let future = chrono::Utc::now() + chrono::Duration::days(30);
    let future_str = future.to_rfc3339();
    let args = serde_json::json!({
        "trigger_type": "time",
        "trigger_at": future_str,
        "delay_seconds": 60,
        "context_text": "test"
    });
    let result = validate_intent_request(&args).unwrap();
    // trigger_at should win over delay_seconds
    let expected = chrono::DateTime::parse_from_rfc3339(&future_str)
        .unwrap()
        .timestamp();
    assert_eq!(result.trigger_at_unix.unwrap(), expected);
}

#[test]
fn validate_intent_no_delay_no_trigger_at_errors() {
    let args = serde_json::json!({
        "trigger_type": "time",
        "context_text": "test"
    });
    let err = validate_intent_request(&args).unwrap_err();
    assert!(err.to_string().contains("trigger_at"));
}

// ── execution_mode default tests ──────────────────────────────────

#[test]
fn default_execution_mode_is_llm_callback() {
    // When execution_mode is omitted, default to llm_callback (fail-safe)
    let args = json!({
        "trigger_type": "time",
        "delay_seconds": 60,
        "context_text": "\u{68c0}\u{67e5}\u{90ae}\u{4ef6}"
    });
    let result = validate_intent_request(&args).unwrap();
    assert_eq!(
        result.execution_mode, "llm_callback",
        "Default should be llm_callback, not notify"
    );
}

#[test]
fn explicit_notify_mode_preserved() {
    // When LLM explicitly chooses notify, respect it
    let args = json!({
        "trigger_type": "time",
        "delay_seconds": 60,
        "execution_mode": "notify",
        "context_text": "\u{8be5}\u{5403}\u{836f}\u{4e86}"
    });
    let result = validate_intent_request(&args).unwrap();
    assert_eq!(result.execution_mode, "notify");
}

#[test]
fn explicit_llm_callback_mode_preserved() {
    let args = json!({
        "trigger_type": "time",
        "delay_seconds": 60,
        "execution_mode": "llm_callback",
        "context_text": "\u{67e5}BTC\u{4ef7}\u{683c}"
    });
    let result = validate_intent_request(&args).unwrap();
    assert_eq!(result.execution_mode, "llm_callback");
}

// ── Property-based tests (proptest) ─────────────────────────────────

mod proptest_intent {
    use super::*;
    use proptest::prelude::*;
    use serde_json::json;

    proptest! {
        #![proptest_config(ProptestConfig { cases: 200, .. ProptestConfig::default() })]

        /// validate_intent_request never panics on arbitrary JSON values.
        #[test]
        fn validate_intent_never_panics(
            trigger in prop::sample::select(vec![
                "time", "event", "bogus", "", "Time", "EVENT",
            ]),
            has_context in proptest::bool::ANY,
            context in "\\PC{0,100}",
            delay in 0i64..100000,
            has_delay in proptest::bool::ANY,
            has_mode in proptest::bool::ANY,
            mode in prop::sample::select(vec![
                "notify", "llm_callback", "silent", "bogus", "",
            ]),
            extra_key in "[a-z_]{1,15}",
            extra_val in "\\PC{0,30}",
        ) {
            let mut obj = serde_json::Map::new();
            obj.insert("trigger_type".into(), json!(trigger));
            if has_context {
                obj.insert("context_text".into(), json!(context));
            }
            if has_delay {
                obj.insert("delay_seconds".into(), json!(delay));
            }
            if has_mode {
                obj.insert("execution_mode".into(), json!(mode));
            }
            obj.insert(extra_key, json!(extra_val));
            let val = serde_json::Value::Object(obj);
            // Must not panic — Ok or Err are both fine
            let _ = validate_intent_request(&val);
        }

        /// When execution_mode is omitted, valid time intents default to "llm_callback".
        #[test]
        fn execution_mode_default_always_llm_callback(
            context in "[a-zA-Z]{3,50}",
            delay in 60i64..86400,
        ) {
            let args = json!({
                "trigger_type": "time",
                "delay_seconds": delay,
                "context_text": context,
            });
            let result = validate_intent_request(&args);
            if let Ok(intent) = result {
                prop_assert_eq!(
                    intent.execution_mode.as_str(),
                    "llm_callback",
                    "Default execution_mode should be llm_callback, got: {}",
                    intent.execution_mode
                );
            }
        }
    }
}

// ── Callback suppression regression tests (incident 2026-03-20) ──

/// [NO_ACTION] with no tools → suppress (correct: no real work done)
#[test]
fn suppress_no_tools_no_action() {
    assert!(should_suppress_callback("[NO_ACTION]"));
}

/// Empty reply with no tools → suppress
#[test]
fn suppress_empty_reply() {
    assert!(should_suppress_callback(""));
    assert!(should_suppress_callback("  \n  "));
}

/// [NO_ACTION] wrapped in text → suppress
#[test]
fn suppress_wrapped_no_action() {
    assert!(should_suppress_callback("I checked. [NO_ACTION]"));
}

/// Real reply text → don't suppress
#[test]
fn dont_suppress_real_reply() {
    assert!(!should_suppress_callback(
        "Your commute is 45 minutes today."
    ));
    assert!(!should_suppress_callback(
        "\u{5df2}\u{6267}\u{884c}: calendar_reader"
    ));
}

/// **Incident 2026-03-20 (revised 2026-03-21)**: Suppression is based purely
/// on reply content — `[NO_ACTION]` means "nothing actionable" regardless of
/// whether tools ran. A calendar_reader that returns "no new emails" followed by
/// `[NO_ACTION]` SHOULD be suppressed — the user doesn't need a "nothing happened" push.
#[test]
fn suppress_is_content_only_check() {
    // should_suppress_callback only checks reply content
    assert!(should_suppress_callback("[NO_ACTION]"));
    // execute_callback now uses: suppress = should_suppress_callback(&reply)
    // So: tools called + [NO_ACTION] → suppress = true (correctly suppressed)
    let suppress = should_suppress_callback("[NO_ACTION]");
    assert!(
        suppress,
        "[NO_ACTION] must be suppressed even when tools were called"
    );
}

// ── execute_callback combined-logic tests ──
// These test the FULL suppression decision as used in execute_callback,
// combining tools_called + reply content — the gap that allowed the
// 2026-03-21 "已保持静默" leak.

/// Helper: mirrors the suppression decision in execute_callback.
fn callback_should_suppress(reply: &str, _tools_called: &[&str]) -> bool {
    // Current logic (2026-03-21): suppress = should_suppress_callback(&reply)
    // tools_called is intentionally unused — suppression is content-only.
    should_suppress_callback(reply)
}

/// Tools called + [NO_ACTION] → suppress.
/// The email checker ran calendar_reader, found nothing, LLM correctly said [NO_ACTION].
#[test]
fn callback_tools_called_no_action_suppresses() {
    assert!(callback_should_suppress(
        "[NO_ACTION]",
        &["calendar_reader"]
    ));
    assert!(callback_should_suppress(
        "[NO_ACTION]",
        &["calendar_reader", "calendar_update_event"]
    ));
}

/// No tools + [NO_ACTION] → suppress (trivial case, LLM decided nothing to do).
#[test]
fn callback_no_tools_no_action_suppresses() {
    assert!(callback_should_suppress("[NO_ACTION]", &[]));
    assert!(callback_should_suppress("", &[]));
}

/// Tools called + substantive reply → forward to user.
/// calendar_reader found 3 new emails, LLM summarized them.
#[test]
fn callback_tools_called_real_reply_forwards() {
    assert!(!callback_should_suppress(
        "You have 3 new emails from Alice.",
        &["calendar_reader"]
    ));
}

/// No tools + substantive reply → forward to user.
/// Edge case: LLM produced text without calling tools (e.g., summarized from memory).
#[test]
fn callback_no_tools_real_reply_forwards() {
    assert!(!callback_should_suppress(
        "Based on previous data, your commute is 45 min.",
        &[]
    ));
}

/// **Incident 2026-03-21**: LLM wrote natural-language "nothing to report" instead
/// of [NO_ACTION]. This is NOT suppressed — the code can't detect it structurally.
/// The fix is prompt-side (stronger [NO_ACTION] instruction), not code-side.
#[test]
fn callback_natural_language_nothing_not_suppressed() {
    // These bypass suppression because they don't contain [NO_ACTION].
    // This is expected — the prompt must enforce [NO_ACTION] compliance.
    assert!(!callback_should_suppress(
        "\u{6536}\u{5230}\u{3002}Primary \u{8d26}\u{6237}\u{76ee}\u{524d}\u{65e0}\u{65b0}\u{90ae}\u{4ef6}\u{3002}\u{5df2}\u{4fdd}\u{6301}\u{9759}\u{9ed8}\u{3002}",
        &["calendar_reader"]
    ));
    assert!(!callback_should_suppress(
        "No new emails found. Staying silent.",
        &["calendar_reader"]
    ));
}

/// [NO_ACTION] wrapped in explanation text + tools → still suppress.
/// LLM sometimes explains before saying [NO_ACTION].
#[test]
fn callback_wrapped_no_action_with_tools_suppresses() {
    assert!(callback_should_suppress(
        "I checked calendar_reader. No new emails. [NO_ACTION]",
        &["calendar_reader"]
    ));
}

// ── notify_user model tests ──────────────────────────────────────────

use crate::orchestrator::{NotifyUserRequest, OrchestratorResult};

/// Empty notifications = silent callback (no messages delivered).
#[test]
fn notify_model_empty_notifications_is_silent() {
    let result = OrchestratorResult {
        reply: "Internal reasoning text".into(),
        notifications: vec![],
        ..Default::default()
    };
    assert!(
        result.notifications.is_empty(),
        "Empty notifications should mean silent callback"
    );
}

/// Notifications present = messages delivered to user.
#[test]
fn notify_model_notifications_present_means_delivery() {
    let result = OrchestratorResult {
        reply: "Internal text".into(),
        notifications: vec![
            NotifyUserRequest {
                message: "You have 3 new emails.".into(),
                pin: false,
                priority: crate::orchestrator::NotifyPriority::Important,
                source_tools: vec!["calendar_reader".into()],
            },
            NotifyUserRequest {
                message: "BTC price alert triggered!".into(),
                pin: true,
                priority: crate::orchestrator::NotifyPriority::Urgent,
                source_tools: vec!["crypto_tracker".into()],
            },
        ],
        ..Default::default()
    };
    assert_eq!(result.notifications.len(), 2);
    assert_eq!(result.notifications[0].message, "You have 3 new emails.");
    assert!(!result.notifications[0].pin);
    assert_eq!(
        result.notifications[1].message,
        "BTC price alert triggered!"
    );
    assert!(result.notifications[1].pin);
}

/// NotifyUserRequest with pin=true is preserved.
#[test]
fn notify_model_pin_flag_preserved() {
    let notif = NotifyUserRequest {
        message: "Critical alert".into(),
        pin: true,
        priority: crate::orchestrator::NotifyPriority::Urgent,
        source_tools: vec![],
    };
    assert!(notif.pin);
    assert_eq!(notif.message, "Critical alert");
}

/// should_suppress_callback still works for backward compatibility
/// (used in L1 history suppression in orchestrator).
#[test]
fn legacy_suppress_callback_backward_compat() {
    // Still correctly identifies suppressible content
    assert!(should_suppress_callback("[NO_ACTION]"));
    assert!(should_suppress_callback(""));
    assert!(should_suppress_callback("  \n  "));
    assert!(should_suppress_callback("Checked. [NO_ACTION]"));
    // Still correctly identifies non-suppressible content
    assert!(!should_suppress_callback("You have new emails."));
    assert!(!should_suppress_callback("BTC price: $95,000"));
}

// ── Notify fallback tests ─────────────────────────────────────────────

/// Empty context_text for notify mode should produce a fallback,
/// never an empty string (which causes Telegram "message text is empty" error).
#[test]
fn notify_empty_context_text_uses_fallback() {
    let context_text = "";
    let intent_id = 759i64;
    // Replicate the fallback logic from execute_intent (intent.rs ~line 331)
    let text = if context_text.trim().is_empty() {
        format!("⏰ Reminder #{}", intent_id)
    } else {
        context_text.to_string()
    };
    assert_eq!(text, "⏰ Reminder #759");
    assert!(!text.is_empty(), "Must never be empty");
}

/// Whitespace-only context_text should also trigger fallback.
#[test]
fn notify_whitespace_context_text_uses_fallback() {
    let context_text = "  \n  ";
    let intent_id = 100i64;
    let text = if context_text.trim().is_empty() {
        format!("⏰ Reminder #{}", intent_id)
    } else {
        context_text.to_string()
    };
    assert_eq!(text, "⏰ Reminder #100");
}

/// Non-empty context_text should be used as-is.
#[test]
fn notify_nonempty_context_text_passthrough() {
    let context_text = "Time to drink water!";
    let intent_id = 200i64;
    let text = if context_text.trim().is_empty() {
        format!("⏰ Reminder #{}", intent_id)
    } else {
        context_text.to_string()
    };
    assert_eq!(text, "Time to drink water!");
}

/// Creation-time fallback: notify mode + no context_text → uses title.
#[test]
fn task_add_notify_no_context_uses_title() {
    let title = "喝水提醒";
    let execution_mode = "notify";
    let context_text: Option<&str> = None;

    // Replicate the fallback logic from task_action_add
    let effective =
        if context_text.is_none_or(|s| s.trim().is_empty()) && execution_mode == "notify" {
            Some(format!("⏰ {}", title))
        } else {
            context_text.map(String::from)
        };
    assert_eq!(effective, Some("⏰ 喝水提醒".to_string()));
}

/// Creation-time fallback: llm_callback mode + no context_text → stays None.
#[test]
fn task_add_callback_no_context_stays_none() {
    let title = "Check emails";
    let execution_mode = "llm_callback";
    let context_text: Option<&str> = None;

    let effective =
        if context_text.is_none_or(|s| s.trim().is_empty()) && execution_mode == "notify" {
            Some(format!("⏰ {}", title))
        } else {
            context_text.map(String::from)
        };
    assert_eq!(
        effective, None,
        "llm_callback should not trigger notify fallback"
    );
}

#[tokio::test]
async fn llm_callback_intent_enqueues_deferred_intent_work_item() {
    let memory = crate::memory::MemoryHandle::spawn_in_memory().unwrap();
    let settings = std::sync::Arc::new(crate::settings::RuntimeSettings::new());
    let orch = std::sync::Arc::new(crate::test_tool_helpers::test_orchestrator(
        memory.clone(),
        settings,
    ));
    let bot = teloxide::Bot::new("123456:TEST_TOKEN");
    let now = chrono::Utc::now().timestamp();
    let intent_id = memory
        .store_deferred_intent(
            "did:aegis:test",
            12345,
            "time",
            Some(now - 1),
            None,
            "llm_callback",
            "Follow up on the contract review.",
            now + 3600,
            false,
            None,
            0,
            false,
            Some("followup:contract-review"),
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();
    let intent = memory
        .get_due_time_intents(now)
        .await
        .unwrap()
        .into_iter()
        .find(|intent| intent.id == intent_id)
        .expect("stored intent should be due");

    execute_intent(&memory, &orch, &bot, &intent).await;

    let queued = memory
        .list_assistant_work_items("did:aegis:test", Some(&["queued", "running", "done"]), 10)
        .await
        .unwrap();
    assert_eq!(queued.len(), 1);
    assert_eq!(
        queued[0].item.source_kind,
        crate::assistant_work_items::AssistantWorkSourceKind::DeferredIntent
    );
    assert_eq!(
        queued[0].item.mode,
        crate::assistant_work_items::AssistantWorkMode::BackgroundAction
    );
    assert_eq!(
        queued[0]
            .item
            .context
            .get("intent_id")
            .and_then(|v| v.as_i64()),
        Some(intent_id)
    );
}

#[tokio::test]
async fn event_triggered_llm_callback_intent_matches_and_enqueues_work_item() {
    let memory = crate::memory::MemoryHandle::spawn_in_memory().unwrap();
    let settings = std::sync::Arc::new(crate::settings::RuntimeSettings::new());
    let orch = std::sync::Arc::new(crate::test_tool_helpers::test_orchestrator(
        memory.clone(),
        settings,
    ));
    let bot = teloxide::Bot::new("123456:TEST_TOKEN");
    let now = chrono::Utc::now().timestamp();
    let event_key = "calendar:thread:contract-review";
    let intent_id = memory
        .store_deferred_intent(
            "did:aegis:test",
            12345,
            "event",
            None,
            Some(event_key),
            "llm_callback",
            "Review the contract follow-up if this thread changes.",
            now + 3600,
            false,
            None,
            0,
            false,
            Some("followup:contract-review"),
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

    let matched = memory
        .get_matching_event_intents(event_key, "did:aegis:test")
        .await
        .unwrap();
    assert_eq!(matched.len(), 1);
    assert_eq!(matched[0].id, intent_id);

    execute_intent(&memory, &orch, &bot, &matched[0]).await;

    let queued = memory
        .list_assistant_work_items("did:aegis:test", Some(&["queued", "running", "done"]), 10)
        .await
        .unwrap();
    assert_eq!(queued.len(), 1);
    assert_eq!(
        queued[0].item.source_kind,
        crate::assistant_work_items::AssistantWorkSourceKind::DeferredIntent
    );
    assert_eq!(
        queued[0].item.mode,
        crate::assistant_work_items::AssistantWorkMode::BackgroundAction
    );
    assert_eq!(
        queued[0]
            .item
            .context
            .get("event_key")
            .and_then(|v| v.as_str()),
        Some(event_key)
    );
}

// ── Windowed weekday recurrence (Option C) ──────────────────────────────────

/// Build a minimal recurring DeferredIntent for next_windowed_trigger tests.
fn windowed_intent(
    weekdays: Option<Vec<u8>>,
    start: Option<&str>,
    end: Option<&str>,
    tz: Option<&str>,
) -> crate::memory::DeferredIntent {
    crate::memory::DeferredIntent {
        id: 1,
        user_did: "did:test".to_string(),
        chat_id: 1,
        trigger_type: "time".to_string(),
        trigger_at_unix: Some(0),
        event_key: None,
        execution_mode: "llm_callback".to_string(),
        context_text: "windowed".to_string(),
        status: "pending".to_string(),
        created_at: "2026-06-03T00:00:00Z".to_string(),
        expires_at_unix: 0,
        fired_at: None,
        is_callback_origin: false,
        retry_count: 0,
        interval_seconds: Some(1200), // 20 min
        chain_depth: 0,
        pin_on_fire: false,
        group_key: Some("commute".to_string()),
        max_fires: None,
        fire_count: 0,
        recur_weekdays: weekdays,
        recur_window_start: start.map(String::from),
        recur_window_end: end.map(String::from),
        recur_timezone: tz.map(String::from),
        embedding: None,
    }
}

/// Convert "YYYY-MM-DDTHH:MM:SS+ZZ:ZZ" → unix for readable test anchors.
fn ts(rfc3339: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(rfc3339)
        .unwrap()
        .timestamp()
}

#[test]
fn windowed_no_fields_is_classic_passthrough() {
    // REGRESSION GUARD: no window fields → byte-identical to now + interval.
    let intent = windowed_intent(None, None, None, None);
    let now = 1_700_000_000;
    let interval = 1200;
    assert_eq!(
        next_windowed_trigger(now, &intent, interval),
        now + interval,
        "no window fields must be classic now+interval"
    );
}

#[test]
fn windowed_within_window_unchanged() {
    // Window Tue-Thu 17:30-18:30 PT. now+interval lands at 17:50 PT Wed → kept.
    let intent = windowed_intent(
        Some(vec![2, 3, 4]),
        Some("17:30"),
        Some("18:30"),
        Some("America/Los_Angeles"),
    );
    // Wed 2026-06-03 17:30 PT, +20min interval from a now of 17:30 → 17:50.
    let now = ts("2026-06-03T17:30:00-07:00");
    let next = next_windowed_trigger(now, &intent, 1200);
    assert_eq!(next, now + 1200, "in-window candidate must be unchanged");
}

#[test]
fn windowed_before_window_advances_to_window_start() {
    // now+interval at 09:00 PT Wed (a valid weekday) but before 17:30 → snap to 17:30 Wed.
    let intent = windowed_intent(
        Some(vec![2, 3, 4]),
        Some("17:30"),
        Some("18:30"),
        Some("America/Los_Angeles"),
    );
    let now = ts("2026-06-03T08:40:00-07:00"); // +20m → 09:00 PT Wed
    let next = next_windowed_trigger(now, &intent, 1200);
    assert_eq!(next, ts("2026-06-03T17:30:00-07:00"));
}

#[test]
fn windowed_after_window_advances_to_next_valid_day() {
    // now+interval at 19:00 PT Wed (after 18:30) → next valid day Thu 17:30.
    let intent = windowed_intent(
        Some(vec![2, 3, 4]),
        Some("17:30"),
        Some("18:30"),
        Some("America/Los_Angeles"),
    );
    let now = ts("2026-06-03T18:40:00-07:00"); // +20m → 19:00 PT Wed
    let next = next_windowed_trigger(now, &intent, 1200);
    assert_eq!(
        next,
        ts("2026-06-04T17:30:00-07:00"),
        "should jump to Thu 17:30"
    );
}

#[test]
fn windowed_wrong_weekday_advances_to_next_allowed() {
    // Sunday 2026-06-07; allowed Tue-Thu → next is Tue 2026-06-09 17:30.
    let intent = windowed_intent(
        Some(vec![2, 3, 4]),
        Some("17:30"),
        Some("18:30"),
        Some("America/Los_Angeles"),
    );
    let now = ts("2026-06-07T17:30:00-07:00"); // Sunday, +20m → still Sunday
    let next = next_windowed_trigger(now, &intent, 1200);
    assert_eq!(next, ts("2026-06-09T17:30:00-07:00"), "Sun → Tue 17:30");
}

#[test]
fn windowed_midnight_straddle_window() {
    // Window 23:00-01:00 (straddles midnight). Candidate at 00:30 is inside.
    let intent = windowed_intent(
        None, // no weekday restriction
        Some("23:00"),
        Some("01:00"),
        Some("UTC"),
    );
    let now = ts("2026-06-03T00:10:00+00:00"); // +20m → 00:30 UTC, inside wrap window
    let next = next_windowed_trigger(now, &intent, 1200);
    assert_eq!(
        next,
        now + 1200,
        "00:30 is inside the 23:00-01:00 wrap window"
    );

    // Candidate at 12:00 (outside) → snaps to today's 23:00 start.
    let now2 = ts("2026-06-03T11:40:00+00:00"); // +20m → 12:00
    let next2 = next_windowed_trigger(now2, &intent, 1200);
    assert_eq!(next2, ts("2026-06-03T23:00:00+00:00"));
}

#[test]
fn windowed_timezone_evaluated_across_utc_now() {
    // A now expressed in UTC must be evaluated in PT. 2026-06-03T20:00Z = 13:00 PT Wed,
    // before the 17:30 PT window → snap to 17:30 PT Wed (= 2026-06-04T00:30Z).
    let intent = windowed_intent(
        Some(vec![2, 3, 4]),
        Some("17:30"),
        Some("18:30"),
        Some("America/Los_Angeles"),
    );
    let now = ts("2026-06-03T19:40:00+00:00"); // +20m → 20:00Z = 13:00 PT Wed
    let next = next_windowed_trigger(now, &intent, 1200);
    assert_eq!(next, ts("2026-06-03T17:30:00-07:00"));
}

#[test]
fn windowed_default_utc_when_tz_none_but_window_set() {
    // Window set, tz None → evaluated in UTC. 09:00 UTC before 17:30 → 17:30 UTC.
    let intent = windowed_intent(None, Some("17:30"), Some("18:30"), None);
    let now = ts("2026-06-03T08:40:00+00:00"); // +20m → 09:00 UTC
    let next = next_windowed_trigger(now, &intent, 1200);
    assert_eq!(next, ts("2026-06-03T17:30:00+00:00"));
}

#[test]
fn windowed_long_interval_not_fired_before_interval_elapses() {
    // REGRESSION (Codex F1): now 08:00 PT Wed, interval 12h → candidate 20:00 PT
    // Wed (after the 17:30-18:30 window). Must NOT snap back to today's 17:30 —
    // that fires ~9.5h later, sooner than the configured 12h interval. The next
    // valid window start at/after the candidate is Thu 17:30.
    let intent = windowed_intent(
        Some(vec![2, 3, 4]),
        Some("17:30"),
        Some("18:30"),
        Some("America/Los_Angeles"),
    );
    let now = ts("2026-06-03T08:00:00-07:00"); // Wed
    let interval = 12 * 3600;
    let next = next_windowed_trigger(now, &intent, interval);
    assert!(
        next >= now + interval,
        "must not fire before the interval elapses: next={next} now+interval={}",
        now + interval
    );
    assert_eq!(
        next,
        ts("2026-06-04T17:30:00-07:00"),
        "should snap to Thu 17:30, not Wed 17:30"
    );
}

#[test]
fn windowed_dst_spring_forward_gap_advances_within_window() {
    // REGRESSION (Codex F3): 2026-03-08 is US spring-forward (02:00→03:00 PT), so
    // local 02:30 does not exist. Window 02:30-03:30, no weekday restriction. now
    // 00:00 PST, interval 1h → candidate 01:00 PST (before the window). The window
    // start 02:30 falls in the gap → must advance to the first existing instant
    // (03:00 PDT) rather than skipping the whole day for a week.
    let intent = windowed_intent(
        None,
        Some("02:30"),
        Some("03:30"),
        Some("America/Los_Angeles"),
    );
    let now = ts("2026-03-08T00:00:00-08:00"); // PST, before the transition
    let next = next_windowed_trigger(now, &intent, 3600);
    assert_eq!(
        next,
        ts("2026-03-08T03:00:00-07:00"), // PDT — first instant that exists in the window
        "spring-forward gap at window start must advance to 03:00, not skip the day"
    );
}

// ── Validator: windowed recurrence parsing + strict tz ──────────────────────

#[test]
fn validate_windowed_recurrence_ok() {
    let args = json!({
        "trigger_type": "time",
        "delay_seconds": 60,
        "execution_mode": "llm_callback",
        "context_text": "estimate commute",
        "interval_seconds": 1200,
        "group_key": "commute",
        "recur_weekdays": [4, 2, 3, 2], // unsorted + dup → normalized
        "recur_window_start": "17:30",
        "recur_window_end": "18:30",
        "recur_timezone": "America/Los_Angeles",
    });
    let v = validate_intent_request(&args).unwrap();
    assert_eq!(v.recur_weekdays, Some(vec![2, 3, 4]), "deduped + sorted");
    assert_eq!(v.recur_window_start.as_deref(), Some("17:30"));
    assert_eq!(v.recur_window_end.as_deref(), Some("18:30"));
    assert_eq!(v.recur_timezone.as_deref(), Some("America/Los_Angeles"));
}

#[test]
fn validate_invalid_timezone_rejected() {
    let args = json!({
        "trigger_type": "time",
        "delay_seconds": 60,
        "execution_mode": "llm_callback",
        "context_text": "x",
        "interval_seconds": 1200,
        "group_key": "g",
        "recur_window_start": "17:30",
        "recur_window_end": "18:30",
        "recur_timezone": "Not/AZone",
    });
    let err = validate_intent_request(&args).unwrap_err().to_string();
    assert!(err.contains("not a valid IANA timezone"), "got: {err}");
}

#[test]
fn validate_window_without_interval_rejected() {
    let args = json!({
        "trigger_type": "time",
        "delay_seconds": 60,
        "execution_mode": "llm_callback",
        "context_text": "x",
        "recur_window_start": "17:30",
        "recur_window_end": "18:30",
    });
    let err = validate_intent_request(&args).unwrap_err().to_string();
    assert!(err.contains("require interval_seconds"), "got: {err}");
}

#[test]
fn validate_partial_window_rejected() {
    let args = json!({
        "trigger_type": "time",
        "delay_seconds": 60,
        "execution_mode": "llm_callback",
        "context_text": "x",
        "interval_seconds": 1200,
        "group_key": "g",
        "recur_window_start": "17:30",
    });
    let err = validate_intent_request(&args).unwrap_err().to_string();
    assert!(err.contains("must be provided together"), "got: {err}");
}

#[test]
fn validate_overnight_window_with_weekdays_rejected() {
    // Codex F2: an overnight (wrapping, end<start) window combined with weekdays
    // makes the post-midnight segment's owning weekday ambiguous → reject.
    let args = json!({
        "trigger_type": "time",
        "delay_seconds": 60,
        "execution_mode": "llm_callback",
        "context_text": "x",
        "interval_seconds": 1200,
        "group_key": "g",
        "recur_weekdays": [5],
        "recur_window_start": "23:00",
        "recur_window_end": "01:00",
        "recur_timezone": "UTC",
    });
    let err = validate_intent_request(&args).unwrap_err().to_string();
    assert!(err.contains("overnight window"), "got: {err}");
}

#[test]
fn validate_overnight_window_without_weekdays_ok() {
    // The same overnight window WITHOUT a weekday restriction is fully supported
    // (no cross-midnight weekday-ownership ambiguity).
    let args = json!({
        "trigger_type": "time",
        "delay_seconds": 60,
        "execution_mode": "llm_callback",
        "context_text": "x",
        "interval_seconds": 1200,
        "group_key": "g",
        "recur_window_start": "23:00",
        "recur_window_end": "01:00",
        "recur_timezone": "UTC",
    });
    let v = validate_intent_request(&args).unwrap();
    assert_eq!(v.recur_window_start.as_deref(), Some("23:00"));
    assert_eq!(v.recur_window_end.as_deref(), Some("01:00"));
    assert!(v.recur_weekdays.is_none());
}

#[test]
fn validate_windowed_initial_trigger_snapped_into_window() {
    // Codex F4: a windowed recurrence created with a small initial delay must NOT
    // first fire at that raw delay (which is almost always outside the window).
    // The validated initial trigger_at_unix must land inside the window on an
    // allowed weekday — robust to whenever this test runs (asserts the property,
    // not an exact instant).
    use chrono::{Datelike, TimeZone, Timelike};
    let args = json!({
        "trigger_type": "time",
        "delay_seconds": 60, // ~now — essentially never inside 17:30-18:30
        "execution_mode": "llm_callback",
        "context_text": "estimate commute",
        "interval_seconds": 1200,
        "group_key": "commute",
        "recur_weekdays": [2, 3, 4],
        "recur_window_start": "17:30",
        "recur_window_end": "18:30",
        "recur_timezone": "America/Los_Angeles",
    });
    let v = validate_intent_request(&args).unwrap();
    let trig = v
        .trigger_at_unix
        .expect("time trigger has an initial instant");
    let tz: chrono_tz::Tz = "America/Los_Angeles".parse().unwrap();
    let local = tz.timestamp_opt(trig, 0).single().unwrap();
    let iso = local.weekday().number_from_monday() as u8;
    let minutes = i64::from(local.hour() * 60 + local.minute());
    assert!(
        [2, 3, 4].contains(&iso),
        "initial trigger weekday {iso} must be Tue-Thu"
    );
    assert!(
        (17 * 60 + 30..=18 * 60 + 30).contains(&minutes),
        "initial trigger local minutes {minutes} must be inside 17:30-18:30"
    );
}

#[test]
fn validate_weekday_out_of_range_rejected() {
    let args = json!({
        "trigger_type": "time",
        "delay_seconds": 60,
        "execution_mode": "llm_callback",
        "context_text": "x",
        "interval_seconds": 1200,
        "group_key": "g",
        "recur_weekdays": [0, 8],
    });
    let err = validate_intent_request(&args).unwrap_err().to_string();
    assert!(err.contains("must be 1..7"), "got: {err}");
}

#[test]
fn validate_no_window_fields_leaves_recur_none() {
    // BACKWARD-COMPAT: a classic recurring intent has all recur_* = None.
    let args = json!({
        "trigger_type": "time",
        "delay_seconds": 60,
        "execution_mode": "llm_callback",
        "context_text": "x",
        "interval_seconds": 1200,
        "group_key": "g",
    });
    let v = validate_intent_request(&args).unwrap();
    assert!(v.recur_weekdays.is_none());
    assert!(v.recur_window_start.is_none());
    assert!(v.recur_window_end.is_none());
    assert!(v.recur_timezone.is_none());
}

// ── i18n Task 3: Channel-B proactive text builders ───────────────────
// These assert the proactive reminder/scheduler strings are produced via the
// catalog under `display_language`, NOT a hardcoded literal. They fail (or panic
// on `[MISSING]`) if a literal is reintroduced or a catalog key is dropped.

#[test]
fn reminder_fallback_uses_display_language_catalog() {
    // zh locale → Chinese fallback from catalog.
    assert_eq!(
        reminder_fallback_text("zh", 42),
        crate::i18n::tf("reminder.fallback", "zh", "42")
    );
    // en locale → English fallback; must differ from zh (proves locale is honored).
    assert_eq!(
        reminder_fallback_text("en", 42),
        crate::i18n::tf("reminder.fallback", "en", "42")
    );
    assert_ne!(
        reminder_fallback_text("zh", 42),
        reminder_fallback_text("en", 42)
    );
    // Regression guard: the old hardcoded literal must NOT be the zh output.
    assert_ne!(reminder_fallback_text("zh", 42), "⏰ Reminder #42");
    // Catalog key resolved (no [MISSING] leak).
    assert!(!reminder_fallback_text("en", 42).contains("[MISSING]"));
    assert!(reminder_fallback_text("en", 42).contains("42"));
}

#[test]
fn scheduled_action_error_uses_display_language_catalog() {
    // Timeout, zh: single language (no "/ Scheduled action" English mixing).
    // The message is a localized PREFIX (retry count) + the raw error concatenated.
    let zh_timeout = scheduled_action_error_text("zh", true, 3, "db locked");
    assert_eq!(
        zh_timeout,
        format!(
            "{}{}",
            crate::i18n::tf("reminder.scheduled_timed_out", "zh", "3"),
            "db locked"
        )
    );
    assert!(
        !zh_timeout.contains("Scheduled action"),
        "zh output must not mix English"
    );
    assert!(zh_timeout.contains('3') && zh_timeout.contains("db locked"));

    // Timeout, en.
    let en_timeout = scheduled_action_error_text("en", true, 3, "db locked");
    assert_eq!(
        en_timeout,
        format!(
            "{}{}",
            crate::i18n::tf("reminder.scheduled_timed_out", "en", "3"),
            "db locked"
        )
    );
    assert_ne!(zh_timeout, en_timeout);

    // Regression (cto-review WARN): an error string containing a literal `{}`
    // must survive intact — the error is concatenated, never placeholder-
    // substituted, so a re-scanning formatter cannot clobber the embedded braces.
    let braced = scheduled_action_error_text("en", true, 1, "serde error at {} token");
    assert!(
        braced.ends_with("serde error at {} token"),
        "error text with literal braces must not be corrupted: {braced}"
    );

    // Non-timeout failure path, both locales.
    let zh_fail = scheduled_action_error_text("zh", false, 0, "boom");
    assert_eq!(
        zh_fail,
        crate::i18n::tf("reminder.scheduled_failed", "zh", "boom")
    );
    assert!(
        !zh_fail.contains("Scheduled action"),
        "zh output must not mix English"
    );
    let en_fail = scheduled_action_error_text("en", false, 0, "boom");
    assert_eq!(
        en_fail,
        crate::i18n::tf("reminder.scheduled_failed", "en", "boom")
    );
    assert!(!en_fail.contains("[MISSING]"));
}

// ── 2026-09-13 review fixes ──────────────────────────────────────────────

#[test]
fn recurring_next_trigger_keeps_the_scheduled_phase() {
    // Monday 10:00 fired 3 minutes late → next is Monday 10:00 + 1 week, not 10:03.
    let week = 7 * 86_400;
    let scheduled = 1_800_000_000;
    assert_eq!(
        next_trigger_base(scheduled + 180, Some(scheduled), week),
        scheduled + week
    );
    // Three-day outage: skip the missed slots, land on the next in-phase slot.
    let now = scheduled + 3 * 86_400;
    assert_eq!(
        next_trigger_base(now, Some(scheduled), week),
        scheduled + week
    );
    // Two whole weeks down: still in phase.
    let now = scheduled + 15 * 86_400;
    assert_eq!(
        next_trigger_base(now, Some(scheduled), week),
        scheduled + 3 * week
    );
    // No scheduled time → the old behaviour.
    assert_eq!(next_trigger_base(now, None, 3600), now + 3600);
    // Scheduled in the future (early poll) → one interval after it.
    assert_eq!(
        next_trigger_base(scheduled - 10, Some(scheduled), 3600),
        scheduled + 3600
    );
}

#[test]
fn missed_reminder_digest_lists_expired_reminders() {
    let items = vec![
        (12, "did:x".to_string(), "医保缴费".to_string()),
        (13, "did:x".to_string(), "   ".to_string()),
    ];
    let zh = missed_reminders_text("zh", &items);
    assert!(zh.starts_with("⏰ 这些提醒在我离线时到期了"), "{zh}");
    assert!(zh.contains("- 医保缴费 (#12)"), "{zh}");
    assert!(!zh.contains("#13"), "blank contexts are skipped: {zh}");
    assert!(
        !zh.contains("还有"),
        "a blank leftover is not counted as more: {zh}"
    );
    let en = missed_reminders_text("en", &items);
    assert!(en.starts_with("⏰ These reminders came due"), "{en}");
    assert_eq!(missed_reminders_text("zh", &[]), "");
    let many: Vec<(i64, String, String)> = (1..=14)
        .map(|i| (i, "d".into(), format!("提醒 {i}")))
        .collect();
    let text = missed_reminders_text("zh", &many);
    assert!(text.contains("…还有 4 条"), "{text}");
}

#[test]
fn recurring_start_overtaken_by_tool_latency_keeps_cadence() {
    let now = 1_789_572_000;
    for lag in [23, 48, 300] {
        let original = now - lag;
        let args = json!({"trigger_at": chrono::DateTime::from_timestamp(original, 0).unwrap().to_rfc3339()});
        let (next, _, advanced) = validate_time_trigger(&args, now, Some(1200)).unwrap();
        assert!(advanced);
        assert!(next >= now + MIN_DELAY_SECS);
        assert_eq!((next - original) % 1200, 0);
        assert!(next - 1200 < now + MIN_DELAY_SECS);
        assert!(
            validate_time_trigger(&args, now, None).is_err(),
            "one-shot must not silently move"
        );
    }
    let args =
        json!({"trigger_at": chrono::DateTime::from_timestamp(now - 301, 0).unwrap().to_rfc3339()});
    assert!(validate_time_trigger(&args, now, Some(1200)).is_err());
}

#[test]
fn recurring_near_future_start_and_window_still_validate() {
    let now = chrono::Utc::now().timestamp();
    let mut args = json!({"trigger_type":"time", "context_text":"commute monitoring",
        "trigger_at": chrono::DateTime::from_timestamp(now - 48, 0).unwrap().to_rfc3339(),
        "interval_seconds":1200,"group_key":"commute-latency", "recur_weekdays":[1,2,3,4,5],
        "recur_window_start":"17:30","recur_window_end":"18:30","recur_timezone":"America/Los_Angeles"});
    let result = validate_intent_request(&args).unwrap();
    let next = result.trigger_at_unix.unwrap();
    assert!(result.warning.unwrap().contains("No catch-up"));
    assert_eq!(
        snap_into_window(
            next,
            result.recur_weekdays.as_deref(),
            result.recur_window_start.as_deref(),
            result.recur_window_end.as_deref(),
            result.recur_timezone.as_deref()
        ),
        next
    );
    args["interval_seconds"] = json!(5);
    assert!(validate_intent_request(&args).is_err());
    args["interval_seconds"] = json!(1200);
    args["group_key"] = json!("");
    assert!(validate_intent_request(&args).is_err());
}

// ── IANA zones resolve at the target date, not today (2026-11-05 fix) ──
// Before the fix the IANA offset came from `Utc::now()`, so these results
// shifted by an hour depending on whether the suite ran in summer or winter.

#[test]
fn parse_trigger_at_iana_uses_target_date_offset_in_winter() {
    let now = chrono::DateTime::parse_from_rfc3339("2026-07-01T12:00:00+00:00")
        .unwrap()
        .timestamp();
    // 09:00 PST (UTC-8) on a December date, scheduled from July.
    let result = parse_trigger_at("2026-12-01 09:00", Some("America/Los_Angeles"), now).unwrap();
    let expected = chrono::DateTime::parse_from_rfc3339("2026-12-01T17:00:00+00:00")
        .unwrap()
        .timestamp();
    assert_eq!(result, expected);
}

#[test]
fn parse_trigger_at_iana_uses_target_date_offset_in_summer() {
    let now = chrono::DateTime::parse_from_rfc3339("2026-12-01T12:00:00+00:00")
        .unwrap()
        .timestamp();
    // 09:00 PDT (UTC-7) on a July date, scheduled from December.
    let result = parse_trigger_at("2026-07-01 09:00", Some("America/Los_Angeles"), now).unwrap();
    let expected = chrono::DateTime::parse_from_rfc3339("2026-07-01T16:00:00+00:00")
        .unwrap()
        .timestamp();
    assert_eq!(result, expected);
}

#[test]
fn parse_trigger_at_time_only_wraps_across_spring_forward() {
    // 23:00 PST on 2026-03-07; DST starts 2026-03-08 02:00 local.
    let now = chrono::DateTime::parse_from_rfc3339("2026-03-08T07:00:00+00:00")
        .unwrap()
        .timestamp();
    let result = parse_trigger_at("08:00", Some("America/Los_Angeles"), now).unwrap();
    // Next 08:00 is PDT (UTC-7) = 15:00 UTC, not now+24h-style 16:00 UTC.
    let expected = chrono::DateTime::parse_from_rfc3339("2026-03-08T15:00:00+00:00")
        .unwrap()
        .timestamp();
    assert_eq!(result, expected);
}

#[test]
fn parse_trigger_at_time_only_wraps_across_fall_back() {
    // 23:00 PDT on 2026-10-31; DST ends 2026-11-01 02:00 local.
    let now = chrono::DateTime::parse_from_rfc3339("2026-11-01T06:00:00+00:00")
        .unwrap()
        .timestamp();
    let result = parse_trigger_at("08:00", Some("America/Los_Angeles"), now).unwrap();
    // Next 08:00 is PST (UTC-8) = 16:00 UTC.
    let expected = chrono::DateTime::parse_from_rfc3339("2026-11-01T16:00:00+00:00")
        .unwrap()
        .timestamp();
    assert_eq!(result, expected);
}

#[test]
fn parse_trigger_at_skipped_local_time_moves_one_hour_later() {
    // 02:30 does not exist on 2026-03-08 in Los Angeles; it reads as 03:30 PDT.
    let result = parse_trigger_at("2026-03-08 02:30", Some("America/Los_Angeles"), 0).unwrap();
    let expected = chrono::DateTime::parse_from_rfc3339("2026-03-08T10:30:00+00:00")
        .unwrap()
        .timestamp();
    assert_eq!(result, expected);
}

#[test]
fn parse_trigger_at_ambiguous_local_time_takes_earlier_instant() {
    // 01:30 happens twice on 2026-11-01 in Los Angeles; take the PDT one.
    let result = parse_trigger_at("2026-11-01 01:30", Some("America/Los_Angeles"), 0).unwrap();
    let expected = chrono::DateTime::parse_from_rfc3339("2026-11-01T08:30:00+00:00")
        .unwrap()
        .timestamp();
    assert_eq!(result, expected);
}

#[test]
fn parse_trigger_at_far_future_date_is_an_error_not_a_panic() {
    // Near chrono's max date the UTC result overflows (LocalResult::None);
    // the skipped-time fallback must not panic adding an hour.
    for (s, tz) in [
        ("+262142-12-31 23:30", Some("-07:00")),
        ("+262142-12-31T23:30:00", Some("America/Los_Angeles")),
    ] {
        let r = std::panic::catch_unwind(|| parse_trigger_at(s, tz, 0));
        assert!(r.is_ok(), "panicked on {s}");
    }
}
