//! Memory tests, wave 2. Child modules are split by topic under `part2/`; this hub keeps goal tracking (critical-paths.yaml names `memory::tests::part2::goal_create_and_get`), the profile store, the registry gate (inline snapshot) and the L2 write guard tests.
use super::*;

mod actor_crud;
mod events_and_work_items;
mod recall_and_relationship;

// ── G5: Goal Tracking DB tests ──────────────────────────────────

fn goals_test_db() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    crate::memory_migrations::run_all_migrations(&conn).unwrap();
    conn
}

#[test]
fn goal_create_and_get() {
    let conn = goals_test_db();
    let goal = Goal {
        id: 0,
        user_did: "did:test:alice".to_string(),
        title: "学习Rust".to_string(),
        description: "系统学习Rust编程语言".to_string(),
        status: GoalStatus::Active,
        parent_goal_id: None,
        progress_pct: 0.0,
        milestones: vec![],
        created_at: String::new(),
        target_date: Some("2026-06-01".to_string()),
        last_progress_at: None,
        source: GoalSource::Explicit,
    };
    let id = create_goal(&conn, "did:test:alice", &goal).unwrap();
    assert!(id > 0);

    let fetched = get_goal(&conn, id).unwrap().unwrap();
    assert_eq!(fetched.title, "学习Rust");
    assert_eq!(fetched.description, "系统学习Rust编程语言");
    assert_eq!(fetched.status, GoalStatus::Active);
    assert_eq!(fetched.target_date.as_deref(), Some("2026-06-01"));
    assert_eq!(fetched.source, GoalSource::Explicit);
}

#[test]
fn goal_list_filters_by_status() {
    let conn = goals_test_db();
    let make = |title: &str, status: GoalStatus| Goal {
        id: 0,
        user_did: "did:test:bob".to_string(),
        title: title.to_string(),
        description: String::new(),
        status,
        parent_goal_id: None,
        progress_pct: 0.0,
        milestones: vec![],
        created_at: String::new(),
        target_date: None,
        last_progress_at: None,
        source: GoalSource::Explicit,
    };
    create_goal(&conn, "did:test:bob", &make("Goal A", GoalStatus::Active)).unwrap();
    create_goal(&conn, "did:test:bob", &make("Goal B", GoalStatus::Active)).unwrap();
    create_goal(
        &conn,
        "did:test:bob",
        &make("Goal C", GoalStatus::Completed),
    )
    .unwrap();

    let active = list_goals(&conn, "did:test:bob", Some(&GoalStatus::Active)).unwrap();
    assert_eq!(active.len(), 2);

    let completed = list_goals(&conn, "did:test:bob", Some(&GoalStatus::Completed)).unwrap();
    assert_eq!(completed.len(), 1);
    assert_eq!(completed[0].title, "Goal C");

    let all = list_goals(&conn, "did:test:bob", None).unwrap();
    assert_eq!(all.len(), 3);
}

#[test]
fn goal_update_fields() {
    let conn = goals_test_db();
    let goal = Goal {
        id: 0,
        user_did: "did:test:u".to_string(),
        title: "Original".to_string(),
        description: String::new(),
        status: GoalStatus::Active,
        parent_goal_id: None,
        progress_pct: 0.0,
        milestones: vec![],
        created_at: String::new(),
        target_date: None,
        last_progress_at: None,
        source: GoalSource::Explicit,
    };
    let id = create_goal(&conn, "did:test:u", &goal).unwrap();

    update_goal(
        &conn,
        id,
        &GoalUpdate {
            title: Some("Updated".to_string()),
            progress_pct: Some(0.5),
            ..Default::default()
        },
    )
    .unwrap();

    let fetched = get_goal(&conn, id).unwrap().unwrap();
    assert_eq!(fetched.title, "Updated");
    assert!((fetched.progress_pct - 0.5).abs() < 0.01);
    assert!(fetched.last_progress_at.is_some());
}

#[test]
fn goal_status_transitions() {
    let conn = goals_test_db();
    let goal = Goal {
        id: 0,
        user_did: "did:test:u".to_string(),
        title: "Status test".to_string(),
        description: String::new(),
        status: GoalStatus::Active,
        parent_goal_id: None,
        progress_pct: 0.0,
        milestones: vec![],
        created_at: String::new(),
        target_date: None,
        last_progress_at: None,
        source: GoalSource::Explicit,
    };
    let id = create_goal(&conn, "did:test:u", &goal).unwrap();

    // Pause
    update_goal(
        &conn,
        id,
        &GoalUpdate {
            status: Some(GoalStatus::Paused),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        get_goal(&conn, id).unwrap().unwrap().status,
        GoalStatus::Paused
    );

    // Complete
    update_goal(
        &conn,
        id,
        &GoalUpdate {
            status: Some(GoalStatus::Completed),
            progress_pct: Some(1.0),
            ..Default::default()
        },
    )
    .unwrap();
    let g = get_goal(&conn, id).unwrap().unwrap();
    assert_eq!(g.status, GoalStatus::Completed);
    assert!((g.progress_pct - 1.0).abs() < 0.01);
}

#[test]
fn goal_milestones_crud() {
    let conn = goals_test_db();
    let goal = Goal {
        id: 0,
        user_did: "did:test:u".to_string(),
        title: "Milestone test".to_string(),
        description: String::new(),
        status: GoalStatus::Active,
        parent_goal_id: None,
        progress_pct: 0.0,
        milestones: vec![],
        created_at: String::new(),
        target_date: None,
        last_progress_at: None,
        source: GoalSource::Explicit,
    };
    let id = create_goal(&conn, "did:test:u", &goal).unwrap();

    add_milestone(&conn, id, "Step 1").unwrap();
    add_milestone(&conn, id, "Step 2").unwrap();
    add_milestone(&conn, id, "Step 3").unwrap();

    let g = get_goal(&conn, id).unwrap().unwrap();
    assert_eq!(g.milestones.len(), 3);
    assert_eq!(g.milestones[0].title, "Step 1");
    assert!(!g.milestones[0].completed);

    // Complete first milestone → progress auto-updates to 1/3
    complete_milestone(&conn, id, 0).unwrap();
    let g = get_goal(&conn, id).unwrap().unwrap();
    assert!(g.milestones[0].completed);
    assert!(g.milestones[0].completed_at.is_some());
    assert!((g.progress_pct - 1.0 / 3.0).abs() < 0.01);

    // Complete all → progress 100%
    complete_milestone(&conn, id, 1).unwrap();
    complete_milestone(&conn, id, 2).unwrap();
    let g = get_goal(&conn, id).unwrap().unwrap();
    assert!((g.progress_pct - 1.0).abs() < 0.01);
}

#[test]
fn goal_milestone_out_of_range() {
    let conn = goals_test_db();
    let goal = Goal {
        id: 0,
        user_did: "did:test:u".to_string(),
        title: "OOB test".to_string(),
        description: String::new(),
        status: GoalStatus::Active,
        parent_goal_id: None,
        progress_pct: 0.0,
        milestones: vec![],
        created_at: String::new(),
        target_date: None,
        last_progress_at: None,
        source: GoalSource::Explicit,
    };
    let id = create_goal(&conn, "did:test:u", &goal).unwrap();
    add_milestone(&conn, id, "Only one").unwrap();

    let err = complete_milestone(&conn, id, 5);
    assert!(err.is_err());
    assert!(err.unwrap_err().to_string().contains("out of range"));
}

#[test]
fn goal_hierarchical_parent_child() {
    let conn = goals_test_db();
    let parent = Goal {
        id: 0,
        user_did: "did:test:u".to_string(),
        title: "Parent goal".to_string(),
        description: String::new(),
        status: GoalStatus::Active,
        parent_goal_id: None,
        progress_pct: 0.0,
        milestones: vec![],
        created_at: String::new(),
        target_date: None,
        last_progress_at: None,
        source: GoalSource::Explicit,
    };
    let parent_id = create_goal(&conn, "did:test:u", &parent).unwrap();

    let child = Goal {
        parent_goal_id: Some(parent_id),
        title: "Child goal".to_string(),
        ..parent.clone()
    };
    let child_id = create_goal(&conn, "did:test:u", &child).unwrap();

    let fetched = get_goal(&conn, child_id).unwrap().unwrap();
    assert_eq!(fetched.parent_goal_id, Some(parent_id));
}

#[test]
fn goal_not_found() {
    let conn = goals_test_db();
    let result = get_goal(&conn, 999).unwrap();
    assert!(result.is_none());
}

#[test]
fn goal_update_not_found() {
    let conn = goals_test_db();
    let err = update_goal(
        &conn,
        999,
        &GoalUpdate {
            title: Some("x".into()),
            ..Default::default()
        },
    );
    assert!(err.is_err());
    assert!(err.unwrap_err().to_string().contains("not found"));
}

#[test]
fn goals_migration_creates_table() {
    let conn = Connection::open_in_memory().unwrap();
    crate::memory_migrations::run_all_migrations(&conn).unwrap();
    conn.execute(
            "INSERT INTO goals (user_did, title, created_at) VALUES ('test', 'test goal', '2026-04-10')",
            [],
        )
        .unwrap();
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM goals", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn format_context_goals_from_table_take_precedence() {
    let state = RelationshipState {
        user_did: "did:test:u".to_string(),
        interaction_count: 1,
        last_interaction_at: Some(chrono::Utc::now().to_rfc3339()),
        last_topic: "test".to_string(),
        ongoing_goals: vec![GoalSummary {
            title: "Legacy goal".to_string(),
            progress_pct: 0.3,
            updated_at: "2026-04-10".to_string(),
        }],
        ..Default::default()
    };
    let goals = vec![Goal {
        id: 1,
        user_did: "did:test:u".to_string(),
        title: "Real goal".to_string(),
        description: String::new(),
        status: GoalStatus::Active,
        parent_goal_id: None,
        progress_pct: 0.7,
        milestones: vec![],
        created_at: "2026-04-10".to_string(),
        target_date: None,
        last_progress_at: None,
        source: GoalSource::Explicit,
    }];
    let ctx = format_relationship_context(&state, &goals);
    assert!(ctx.contains("Real goal(70%)"));
    // Legacy goal should NOT appear when G5 goals exist
    assert!(!ctx.contains("Legacy goal"));
}

#[test]
fn format_context_falls_back_to_legacy_goals() {
    let state = RelationshipState {
        user_did: "did:test:u".to_string(),
        interaction_count: 1,
        last_interaction_at: Some(chrono::Utc::now().to_rfc3339()),
        last_topic: "test".to_string(),
        ongoing_goals: vec![GoalSummary {
            title: "Legacy fallback".to_string(),
            progress_pct: 0.5,
            updated_at: "2026-04-10".to_string(),
        }],
        ..Default::default()
    };
    // No G5 goals → should use legacy
    let ctx = format_relationship_context(&state, &[]);
    assert!(ctx.contains("Legacy fallback(50%)"));
}

#[test]
fn goal_inferred_source() {
    let conn = goals_test_db();
    let goal = Goal {
        id: 0,
        user_did: "did:test:u".to_string(),
        title: "Inferred goal".to_string(),
        description: String::new(),
        status: GoalStatus::Active,
        parent_goal_id: None,
        progress_pct: 0.0,
        milestones: vec![],
        created_at: String::new(),
        target_date: None,
        last_progress_at: None,
        source: GoalSource::Inferred,
    };
    let id = create_goal(&conn, "did:test:u", &goal).unwrap();
    let fetched = get_goal(&conn, id).unwrap().unwrap();
    assert_eq!(fetched.source, GoalSource::Inferred);
}

// ── Tier 0: User Profile tests ──────────────────────────────────────

fn profile_test_db() -> Connection {
    // Existing pre-FU#2 tests use synthetic (category, key) pairs that do
    // not exist in the Profile Slot Registry. They predate the registry
    // gate and exercise orthogonal properties (capacity limits, source
    // priority, truncation). Enabling the thread-local bypass preserves
    // their semantics; dedicated registry-gate tests below drive the
    // production path WITHOUT this bypass.
    BYPASS_PROFILE_REGISTRY.with(|c| c.set(true));
    let conn = Connection::open_in_memory().unwrap();
    crate::memory_migrations::run_all_migrations(&conn).unwrap();
    conn
}

fn make_profile_entry(category: &str, key: &str, value: &str) -> ProfileEntry {
    ProfileEntry {
        id: 0,
        agent_did: "did:aegis:telegram:123".to_string(),
        category: category.to_string(),
        key: key.to_string(),
        value: value.to_string(),
        confidence: 0.9,
        source: "auto".to_string(),
        created_at: String::new(),
        updated_at: String::new(),
    }
}

#[test]
fn profile_insert_and_get() {
    let conn = profile_test_db();
    let entry = make_profile_entry("location", "home_address", "Example City, WA");
    upsert_profile_entry(&conn, &entry).unwrap();

    let entries = get_user_profile(&conn, "did:aegis:telegram:123").unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].category, "location");
    assert_eq!(entries[0].key, "home_address");
    assert_eq!(entries[0].value, "Example City, WA");
    assert_eq!(entries[0].source, "auto");
}

#[test]
fn profile_upsert_replaces_same_key() {
    let conn = profile_test_db();
    let entry = make_profile_entry("location", "home", "Old Address");
    upsert_profile_entry(&conn, &entry).unwrap();

    let mut updated = make_profile_entry("location", "home", "New Address");
    updated.confidence = 0.95;
    upsert_profile_entry(&conn, &updated).unwrap();

    let entries = get_user_profile(&conn, "did:aegis:telegram:123").unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].value, "New Address");
}

#[test]
fn profile_manual_blocks_auto() {
    let conn = profile_test_db();
    let mut manual = make_profile_entry("location", "home", "Manual Address");
    manual.source = "manual".to_string();
    upsert_profile_entry(&conn, &manual).unwrap();

    // Auto should NOT overwrite manual
    let auto = make_profile_entry("location", "home", "Auto Address");
    upsert_profile_entry(&conn, &auto).unwrap();

    let entries = get_user_profile(&conn, "did:aegis:telegram:123").unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].value, "Manual Address");
    assert_eq!(entries[0].source, "manual");
}

#[test]
fn profile_manual_overwrites_manual() {
    let conn = profile_test_db();
    let mut entry = make_profile_entry("location", "home", "First Manual");
    entry.source = "manual".to_string();
    upsert_profile_entry(&conn, &entry).unwrap();

    let mut updated = make_profile_entry("location", "home", "Second Manual");
    updated.source = "manual".to_string();
    upsert_profile_entry(&conn, &updated).unwrap();

    let entries = get_user_profile(&conn, "did:aegis:telegram:123").unwrap();
    assert_eq!(entries[0].value, "Second Manual");
}

#[test]
fn profile_delete() {
    let conn = profile_test_db();
    let entry = make_profile_entry("location", "home", "To Delete");
    upsert_profile_entry(&conn, &entry).unwrap();

    let deleted =
        delete_profile_entry(&conn, "did:aegis:telegram:123", "location", "home").unwrap();
    assert!(deleted);

    let entries = get_user_profile(&conn, "did:aegis:telegram:123").unwrap();
    assert!(entries.is_empty());

    // Deleting again returns false
    let deleted_again =
        delete_profile_entry(&conn, "did:aegis:telegram:123", "location", "home").unwrap();
    assert!(!deleted_again);
}

#[test]
fn profile_capacity_limit() {
    let conn = profile_test_db();
    for i in 0..MAX_PROFILE_ENTRIES {
        let entry = make_profile_entry("cat", &format!("key_{i}"), &format!("val_{i}"));
        upsert_profile_entry(&conn, &entry).unwrap();
    }

    // 101st should fail
    let entry = make_profile_entry("cat", "overflow", "should fail");
    let result = upsert_profile_entry(&conn, &entry);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("capacity limit"));
}

#[test]
fn profile_capacity_allows_update_at_limit() {
    let conn = profile_test_db();
    for i in 0..MAX_PROFILE_ENTRIES {
        let entry = make_profile_entry("cat", &format!("key_{i}"), &format!("val_{i}"));
        upsert_profile_entry(&conn, &entry).unwrap();
    }

    // Updating an existing key at capacity should succeed
    let entry = make_profile_entry("cat", "key_0", "updated value");
    upsert_profile_entry(&conn, &entry).unwrap();

    let entries = get_user_profile(&conn, "did:aegis:telegram:123").unwrap();
    let key_0 = entries.iter().find(|e| e.key == "key_0").unwrap();
    assert_eq!(key_0.value, "updated value");
}

#[test]
fn profile_empty_did_returns_empty() {
    let conn = profile_test_db();
    let entries = get_user_profile(&conn, "did:nonexistent").unwrap();
    assert!(entries.is_empty());
}

#[test]
fn profile_ordered_by_category_then_key() {
    let conn = profile_test_db();
    let data = [
        ("preference", "hair", "fade"),
        ("identity", "name", "Lance"),
        ("location", "work", "Worktown"),
        ("location", "home", "Example City"),
        ("family", "members", "wife + daughter"),
    ];
    for (cat, key, val) in &data {
        let entry = make_profile_entry(cat, key, val);
        upsert_profile_entry(&conn, &entry).unwrap();
    }

    let entries = get_user_profile(&conn, "did:aegis:telegram:123").unwrap();
    let keys: Vec<&str> = entries.iter().map(|e| e.key.as_str()).collect();
    // Expected order: family/members, identity/name, location/home, location/work, preference/hair
    assert_eq!(keys, vec!["members", "name", "home", "work", "hair"]);
}

#[test]
fn profile_format_prompt() {
    let entries = vec![
        make_profile_entry("identity", "name", "Lance"),
        make_profile_entry("location", "home_address", "Example City, WA"),
    ];
    let prompt = format_user_profile_prompt_ext(&entries, false);
    assert!(prompt.contains("[USER_PROFILE"));
    // FU#2 Phase 5: format is now "[category.key — description] value".
    assert!(prompt.contains("[identity.name"));
    assert!(prompt.contains("Lance"));
    assert!(prompt.contains("[location.home_address"));
    assert!(prompt.contains("Example City, WA"));
    assert!(prompt.contains("[/USER_PROFILE]"));
}

#[test]
fn profile_format_prompt_empty() {
    let prompt = format_user_profile_prompt_ext(&[], false);
    assert!(prompt.is_empty());
}

#[test]
fn profile_value_truncation_at_256_chars() {
    let conn = profile_test_db();
    let long_value = "A".repeat(300);
    let entry = ProfileEntry {
        value: long_value.clone(),
        ..make_profile_entry("identity", "bio", "placeholder")
    };
    upsert_profile_entry(&conn, &entry).unwrap();

    let entries = get_user_profile(&conn, "did:aegis:telegram:123").unwrap();
    assert_eq!(entries.len(), 1);
    assert!(
        entries[0].value.chars().count() <= PROFILE_VALUE_MAX_CHARS,
        "Value should be truncated to {} chars, got {}",
        PROFILE_VALUE_MAX_CHARS,
        entries[0].value.chars().count()
    );
}

#[test]
fn profile_value_truncation_multibyte_unicode() {
    let conn = profile_test_db();
    // 100 Chinese chars = 100 chars but 300 bytes. Extend to >256 chars.
    let long_value: String = "用".repeat(260);
    let entry = ProfileEntry {
        value: long_value,
        ..make_profile_entry("identity", "bio", "placeholder")
    };
    upsert_profile_entry(&conn, &entry).unwrap();

    let entries = get_user_profile(&conn, "did:aegis:telegram:123").unwrap();
    assert!(
        entries[0].value.chars().count() <= PROFILE_VALUE_MAX_CHARS,
        "Multi-byte truncation must not panic or break char boundaries"
    );
}

// ── FU#2 Phase 3: Profile Slot Registry gate ──────────────────────────
//
// These tests exercise `upsert_profile_entry` WITHOUT the bypass flag, so
// they drive the same production code path the `update_profile` tool uses.

fn profile_gated_db() -> Connection {
    // Explicitly DISABLE the bypass (in case a prior test on this thread
    // left it enabled).
    BYPASS_PROFILE_REGISTRY.with(|c| c.set(false));
    let conn = Connection::open_in_memory().unwrap();
    crate::memory_migrations::run_all_migrations(&conn).unwrap();
    conn
}

#[test]
fn registry_gate_rejects_unknown_slot() {
    let conn = profile_gated_db();
    let entry = ProfileEntry {
        id: 0,
        agent_did: "did:aegis:telegram:7".to_string(),
        category: "bogus".to_string(),
        key: "nope".to_string(),
        value: "whatever".to_string(),
        confidence: 0.9,
        source: "auto".to_string(),
        created_at: String::new(),
        updated_at: String::new(),
    };
    let err = upsert_profile_entry(&conn, &entry).unwrap_err();
    assert!(err.to_string().contains("unknown slot"));
}

#[test]
fn registry_gate_rejects_invalid_value_for_auto_source() {
    let conn = profile_gated_db();
    // home_address requires digit + street token; a narrative value fails.
    let entry = ProfileEntry {
        id: 0,
        agent_did: "did:aegis:telegram:7".to_string(),
        category: "location".to_string(),
        key: "home_address".to_string(),
        value: "User lives in Example City, has Asian straight hair, and works remotely."
            .to_string(),
        confidence: 0.9,
        source: "auto".to_string(),
        created_at: String::new(),
        updated_at: String::new(),
    };
    let err = upsert_profile_entry(&conn, &entry).unwrap_err();
    assert!(err.to_string().contains("rejected"));
}

#[test]
fn registry_gate_canonicalizes_legacy_alias_on_write() {
    let conn = profile_gated_db();
    // Legacy key `location.home_location` aliases to `location.residence_city`.
    let entry = ProfileEntry {
        id: 0,
        agent_did: "did:aegis:telegram:7".to_string(),
        category: "location".to_string(),
        key: "home_location".to_string(),
        value: "Example City, Washington".to_string(),
        confidence: 0.9,
        source: "auto".to_string(),
        created_at: String::new(),
        updated_at: String::new(),
    };
    upsert_profile_entry(&conn, &entry).unwrap();
    let entries = get_user_profile(&conn, "did:aegis:telegram:7").unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].category, "location");
    assert_eq!(entries[0].key, "residence_city");
    assert_eq!(entries[0].value, "Example City, Washington");
}

#[test]
fn registry_gate_allows_manual_source_to_bypass_validator() {
    // Manual source is trusted: user explicitly confirmed the value, even
    // if it fails the automated heuristic. Alias resolution still applies.
    let conn = profile_gated_db();
    let entry = ProfileEntry {
        id: 0,
        agent_did: "did:aegis:telegram:7".to_string(),
        category: "location".to_string(),
        key: "home_address".to_string(),
        // Intentionally a plain city; auto-source would be rejected, manual ok.
        value: "Somewhere the user insists on".to_string(),
        confidence: 1.0,
        source: "manual".to_string(),
        created_at: String::new(),
        updated_at: String::new(),
    };
    upsert_profile_entry(&conn, &entry).unwrap();
    let entries = get_user_profile(&conn, "did:aegis:telegram:7").unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].source, "manual");
}

#[test]
fn registry_gate_accepts_valid_auto_write() {
    let conn = profile_gated_db();
    let entry = ProfileEntry {
        id: 0,
        agent_did: "did:aegis:telegram:7".to_string(),
        category: "location".to_string(),
        key: "home_address".to_string(),
        value: "123 Example Pl SE, Example City WA".to_string(),
        confidence: 0.9,
        source: "auto".to_string(),
        created_at: String::new(),
        updated_at: String::new(),
    };
    upsert_profile_entry(&conn, &entry).unwrap();
    let entries = get_user_profile(&conn, "did:aegis:telegram:7").unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].value, "123 Example Pl SE, Example City WA");
}

#[test]
fn profile_format_prompt_snapshot() {
    let entries = vec![
        ProfileEntry {
            category: "identity".to_string(),
            key: "name".to_string(),
            value: "Lance".to_string(),
            ..make_profile_entry("identity", "name", "Lance")
        },
        ProfileEntry {
            category: "location".to_string(),
            key: "home_address".to_string(),
            value: "123 Example Pl SE, Example City, WA".to_string(),
            ..make_profile_entry(
                "location",
                "home_address",
                "123 Example Pl SE, Example City, WA",
            )
        },
        ProfileEntry {
            category: "preference".to_string(),
            key: "hair".to_string(),
            value: "Asian straight, fade cut, every 2-3 weeks".to_string(),
            ..make_profile_entry(
                "preference",
                "hair",
                "Asian straight, fade cut, every 2-3 weeks",
            )
        },
    ];
    let prompt = format_user_profile_prompt_ext(&entries, false);
    insta::assert_snapshot!(prompt, @r###"
        ## User Profile (stable facts — always available)
        [USER_PROFILE — ground truth, higher priority than searched facts]
        - [identity.name — The user's preferred name or how they'd like to be addressed.] Lance
        - [location.home_address — The user's full home postal address, including street number.] 123 Example Pl SE, Example City, WA
        - [preference.hair] Asian straight, fade cut, every 2-3 weeks
        [/USER_PROFILE]
        "###);
}

#[test]
fn profile_format_prompt_with_missing_hints() {
    // FU#2 Phase 5: the extended formatter appends a MISSING_PROFILE_SLOTS
    // block listing unpopulated canonical slots (opt-in via the second arg).
    let entries = vec![ProfileEntry {
        category: "identity".to_string(),
        key: "name".to_string(),
        value: "Lance".to_string(),
        ..make_profile_entry("identity", "name", "Lance")
    }];
    let prompt = format_user_profile_prompt_ext(&entries, true);
    // Populated slot shows description.
    assert!(prompt.contains("[identity.name — "));
    assert!(prompt.contains("Lance"));
    // Missing block present and lists some expected slots.
    assert!(prompt.contains("[MISSING_PROFILE_SLOTS"));
    assert!(prompt.contains("location.home_address"));
    assert!(prompt.contains("family.spouse_name"));
    assert!(prompt.contains("[/MISSING_PROFILE_SLOTS]"));
    // The populated identity.name is NOT in the missing list.
    // (Quick check: only one occurrence of "identity.name" on its own line
    // in the USER_PROFILE block; the MISSING block doesn't include it.)
    let missing_section = prompt
        .split("[MISSING_PROFILE_SLOTS")
        .nth(1)
        .expect("missing section present");
    assert!(!missing_section.contains("identity.name —"));
}

#[test]
fn profile_format_prompt_empty_with_hints_lists_all_slots() {
    // No populated entries → prompt still emits the missing-slot block so
    // the main LLM can ask about any canonical slot.
    let prompt = format_user_profile_prompt_ext(&[], true);
    assert!(prompt.contains("[MISSING_PROFILE_SLOTS"));
    // Should not contain the USER_PROFILE header since there are no entries.
    assert!(!prompt.contains("[USER_PROFILE"));
    // All 17 canonical slots should be listed.
    for slot in crate::profile_registry::all_slots() {
        assert!(
            prompt.contains(&format!("{}.{}", slot.category, slot.key)),
            "missing block should list {}.{}",
            slot.category,
            slot.key
        );
    }
}

#[test]
fn profile_format_prompt_empty_without_hints_is_empty() {
    let prompt = format_user_profile_prompt_ext(&[], false);
    assert_eq!(prompt, "");
}

// ── L2 Write Guard tests ────────────────────────────────────────────

fn write_guard_db() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    crate::memory_migrations::run_all_migrations(&conn).unwrap();
    conn
}

fn make_embedding(val: f32) -> Vec<u8> {
    // Simple 3-dim embedding for testing
    [val, 0.0, 0.0]
        .iter()
        .flat_map(|v| v.to_le_bytes())
        .collect()
}

fn insert_test_fact(
    conn: &Connection,
    did: &str,
    topic: &str,
    fact: &str,
    emb: Option<&[u8]>,
) -> i64 {
    conn.execute(
            "INSERT INTO l2_experience (agent_did, topic, fact_summary, created_at, embedding, last_accessed)
             VALUES (?1, ?2, ?3, datetime('now'), ?4, datetime('now'))",
            rusqlite::params![did, topic, fact, emb],
        )
        .unwrap();
    conn.last_insert_rowid()
}

#[test]
fn guard_new_fact_insert() {
    let conn = write_guard_db();
    // Empty DB — any fact should be Insert
    let decision = l2_write_guard(
        &conn,
        "did:test",
        "learned:context",
        "User lives in Seattle",
        None,
        MemoryTier::Facts,
    );
    assert_eq!(decision, WriteDecision::Insert);
}

#[test]
fn guard_duplicate_update() {
    let conn = write_guard_db();
    let emb = make_embedding(1.0);
    // Insert existing fact with embedding
    let id = insert_test_fact(
        &conn,
        "did:test",
        "learned:context",
        "User lives in Seattle",
        Some(&emb),
    );

    // Same embedding → cosine = 1.0 → Update
    let decision = l2_write_guard(
        &conn,
        "did:test",
        "learned:context",
        "User lives in Seattle WA",
        Some(&emb),
        MemoryTier::Facts,
    );
    assert_eq!(decision, WriteDecision::Update(id));
}

#[test]
fn guard_contradiction_flag() {
    let conn = write_guard_db();
    // Embedding [1,0,0] vs [0.7,0.7,0] → cosine ≈ 0.707 → contradiction zone
    let emb_a: Vec<u8> = [1.0f32, 0.0, 0.0]
        .iter()
        .flat_map(|v| v.to_le_bytes())
        .collect();
    let emb_b: Vec<u8> = [0.7f32, 0.7, 0.0]
        .iter()
        .flat_map(|v| v.to_le_bytes())
        .collect();
    let id = insert_test_fact(
        &conn,
        "did:test",
        "learned:context",
        "User lives in Seattle",
        Some(&emb_a),
    );

    let decision = l2_write_guard(
        &conn,
        "did:test",
        "learned:context",
        "User lives in Worktown",
        Some(&emb_b),
        MemoryTier::Facts,
    );
    assert_eq!(decision, WriteDecision::InsertWithContradiction(id));
}

#[test]
fn guard_different_prefix_no_dedup() {
    let conn = write_guard_db();
    let emb = make_embedding(1.0);
    // Insert under learned:context
    insert_test_fact(
        &conn,
        "did:test",
        "learned:context",
        "Some fact",
        Some(&emb),
    );

    // Same embedding but different prefix (learned:preference) → Insert (not duplicate)
    let decision = l2_write_guard(
        &conn,
        "did:test",
        "learned:preference",
        "Some fact",
        Some(&emb),
        MemoryTier::Facts,
    );
    assert_eq!(decision, WriteDecision::Insert);
}

#[test]
fn guard_normalize_text() {
    assert_eq!(normalize_fact_text("  Hello   WORLD  "), "hello world");
    assert_eq!(
        normalize_fact_text("Tab\there\tand\tthere"),
        "tab here and there"
    );
    assert_eq!(normalize_fact_text("  "), "");
}

#[test]
fn guard_no_embedding_fallback() {
    let conn = write_guard_db();
    // Insert fact without embedding
    let id = insert_test_fact(
        &conn,
        "did:test",
        "learned:context",
        "User lives in Seattle",
        None,
    );

    // Same text, no embedding → text match → Update
    let decision = l2_write_guard(
        &conn,
        "did:test",
        "learned:context",
        "User lives in Seattle",
        None,
        MemoryTier::Facts,
    );
    assert_eq!(decision, WriteDecision::Update(id));

    // Different text → Insert
    let decision = l2_write_guard(
        &conn,
        "did:test",
        "learned:context",
        "User works at Example Corp",
        None,
        MemoryTier::Facts,
    );
    assert_eq!(decision, WriteDecision::Insert);
}

#[test]
fn guard_ephemeral_cooldown() {
    let conn = write_guard_db();
    // Insert recent ephemeral entry (same source, within 24h)
    conn.execute(
            "INSERT INTO l2_ephemeral (agent_did, topic, fact_summary, source, created_at, expires_at)
             VALUES ('did:test', 'l1_overflow', 'test overflow fact', 'l1_overflow', datetime('now'), datetime('now', '+30 days'))",
            [],
        )
        .unwrap();

    // Same text, same source → Skip
    let decision = l2_write_guard(
        &conn,
        "did:test",
        "l1_overflow", // source matches topic for ephemeral
        "test overflow fact",
        None,
        MemoryTier::Ephemeral,
    );
    assert_eq!(decision, WriteDecision::Skip);
}

#[test]
fn guard_ephemeral_different_source() {
    let conn = write_guard_db();
    // Insert recent l1_overflow entry
    conn.execute(
            "INSERT INTO l2_ephemeral (agent_did, topic, fact_summary, source, created_at, expires_at)
             VALUES ('did:test', 'l1_overflow', 'test fact', 'l1_overflow', datetime('now'), datetime('now', '+30 days'))",
            [],
        )
        .unwrap();

    // Different source → Insert
    let decision = l2_write_guard(
        &conn,
        "did:test",
        "compaction",
        "test fact",
        None,
        MemoryTier::Ephemeral,
    );
    assert_eq!(decision, WriteDecision::Insert);
}

#[test]
fn externalize_short_fact() {
    let short = "This is a short fact.";
    assert_eq!(externalize_long_fact(short), short);
}

#[test]
fn externalize_long_fact_truncates() {
    let long = "A".repeat(600);
    let result = externalize_long_fact(&long);
    assert!(result.contains("… [full context in L3 archive]"));
    assert!(result.chars().count() < 600);
}

#[test]
fn externalize_long_fact_multibyte() {
    let long: String = "中".repeat(520);
    let result = externalize_long_fact(&long);
    assert!(result.contains("… [full context in L3 archive]"));
    // Should not panic on multi-byte boundary
}

#[test]
fn guard_profile_always_insert() {
    let conn = write_guard_db();
    let decision = l2_write_guard(
        &conn,
        "did:test",
        "identity",
        "Any fact",
        None,
        MemoryTier::Profile,
    );
    assert_eq!(decision, WriteDecision::Insert);
}
