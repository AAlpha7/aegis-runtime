//! Recall scoring, tokenizer/Jaccard/MMR reranking, recall stats, relationship state, companion identity and growth.
use super::*;

// ── Self-learning system tests ──────────────────────────────────────

#[test]
fn fnv1a_hash_deterministic() {
    let a = fnv1a_hash(b"hello world");
    let b = fnv1a_hash(b"hello world");
    assert_eq!(a, b);
    let c = fnv1a_hash(b"hello worlD");
    assert_ne!(a, c);
}

#[test]
fn fnv1a_hash_empty() {
    let h = fnv1a_hash(b"");
    assert_eq!(h, 0xcbf29ce484222325); // FNV offset basis
}

#[test]
fn compute_recall_score_no_recalls() {
    let stats = RecallStats::default();
    let score = compute_recall_score(&stats, &DEFAULT_RECALL_WEIGHTS);
    assert!(
        (score - 0.0).abs() < 0.01,
        "No recalls → near-zero score: {score}"
    );
}

#[test]
fn compute_recall_score_heavy_usage() {
    let stats = RecallStats {
        fact_id: 1,
        recall_count: 10,
        total_score: 8.5, // avg ~0.85
        max_score: 0.95,
        unique_queries: 6,
        query_hashes: (0..6).map(|i| format!("{i:016x}")).collect(),
        recall_days: vec![
            "2026-04-01".into(),
            "2026-04-03".into(),
            "2026-04-05".into(),
            "2026-04-07".into(),
            "2026-04-09".into(),
        ],
        first_recalled_at: Some("2026-04-01T00:00:00+00:00".into()),
        last_recalled_at: Some(Utc::now().to_rfc3339()),
    };
    let score = compute_recall_score(&stats, &DEFAULT_RECALL_WEIGHTS);
    assert!(score > 0.6, "Heavy usage → high score: {score}");
    assert!(score <= 1.0, "Score must be clamped: {score}");
}

#[test]
fn compute_recall_score_recency_decays() {
    let recent = RecallStats {
        fact_id: 1,
        recall_count: 3,
        total_score: 2.4,
        max_score: 0.9,
        unique_queries: 2,
        recall_days: vec!["2026-04-09".into()],
        last_recalled_at: Some(Utc::now().to_rfc3339()),
        ..Default::default()
    };
    let stale = RecallStats {
        last_recalled_at: Some("2025-01-01T00:00:00+00:00".into()),
        recall_days: vec!["2025-01-01".into()],
        ..recent.clone()
    };
    let score_recent = compute_recall_score(&recent, &DEFAULT_RECALL_WEIGHTS);
    let score_stale = compute_recall_score(&stale, &DEFAULT_RECALL_WEIGHTS);
    assert!(
        score_recent > score_stale,
        "Recent recall should score higher: {score_recent} vs {score_stale}"
    );
}

#[test]
fn composite_score_cold_start_protection() {
    // Brand new fact (0 days old) with no recalls
    let new_fact = composite_search_score(0.85, 0.0, 0.0);
    // Old fact (30 days) with no recalls
    let old_fact = composite_search_score(0.85, 0.0, 30.0);
    assert!(
        new_fact > old_fact,
        "Cold-start should protect new facts: {new_fact} vs {old_fact}"
    );
    // New fact should still have a reasonable score
    assert!(new_fact > 0.5, "New high-cosine fact score: {new_fact}");
}

#[test]
fn composite_score_recall_boost() {
    // Same cosine, different recall scores
    let no_recall = composite_search_score(0.8, 0.0, 30.0);
    let high_recall = composite_search_score(0.8, 0.9, 30.0);
    assert!(
        high_recall > no_recall,
        "Recall should boost score: {high_recall} vs {no_recall}"
    );
}

#[test]
fn is_cjk_basic() {
    assert!(is_cjk('中'));
    assert!(is_cjk('あ'));
    assert!(is_cjk('한'));
    assert!(!is_cjk('a'));
    assert!(!is_cjk('1'));
    assert!(!is_cjk(' '));
}

#[test]
fn tokenize_ascii_words() {
    let tokens = tokenize_for_mmr("hello world test");
    assert!(tokens.contains("hello"));
    assert!(tokens.contains("world"));
    assert!(tokens.contains("test"));
}

#[test]
fn tokenize_cjk_bigrams() {
    let tokens = tokenize_for_mmr("用户喜欢");
    assert!(tokens.contains("用"), "Unigrams present");
    assert!(tokens.contains("用户"), "Bigram: 用户");
    assert!(tokens.contains("户喜"), "Bigram: 户喜");
    assert!(tokens.contains("喜欢"), "Bigram: 喜欢");
}

#[test]
fn tokenize_mixed() {
    let tokens = tokenize_for_mmr("user likes 简洁");
    assert!(tokens.contains("user"));
    assert!(tokens.contains("likes"));
    assert!(tokens.contains("简"));
    assert!(tokens.contains("简洁"));
}

#[test]
fn jaccard_identical_sets() {
    let a: std::collections::HashSet<String> =
        ["a", "b", "c"].iter().map(|s| s.to_string()).collect();
    let b = a.clone();
    assert!((jaccard_similarity(&a, &b) - 1.0).abs() < f64::EPSILON);
}

#[test]
fn jaccard_disjoint_sets() {
    let a: std::collections::HashSet<String> = ["a", "b"].iter().map(|s| s.to_string()).collect();
    let b: std::collections::HashSet<String> = ["c", "d"].iter().map(|s| s.to_string()).collect();
    assert!((jaccard_similarity(&a, &b) - 0.0).abs() < f64::EPSILON);
}

#[test]
fn jaccard_both_empty() {
    let a = std::collections::HashSet::<String>::new();
    let b = std::collections::HashSet::<String>::new();
    assert!((jaccard_similarity(&a, &b) - 1.0).abs() < f64::EPSILON);
}

#[test]
fn jaccard_partial_overlap() {
    let a: std::collections::HashSet<String> =
        ["a", "b", "c"].iter().map(|s| s.to_string()).collect();
    let b: std::collections::HashSet<String> =
        ["b", "c", "d"].iter().map(|s| s.to_string()).collect();
    // intersection = 2, union = 4
    assert!((jaccard_similarity(&a, &b) - 0.5).abs() < f64::EPSILON);
}

#[test]
fn mmr_rerank_preserves_single() {
    let candidates = vec![(
        Experience {
            id: 1,
            agent_did: "did:test".into(),
            topic: "t".into(),
            fact_summary: "hello".into(),
            created_at: Utc::now().to_rfc3339(),
            confidence: 0.6,
            is_pinned: false,
        },
        0.9,
    )];
    let result = mmr_rerank(candidates.clone(), 3, 0.7);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].0.id, candidates[0].0.id);
}

#[test]
fn mmr_rerank_diversifies() {
    let now = Utc::now().to_rfc3339();
    // #1 and #2 are very similar (share most CJK bigrams)
    // #3 is diverse (English, no overlap) with score between #1 and #2
    let candidates = vec![
        (
            Experience {
                id: 1,
                agent_did: "d".into(),
                topic: "t".into(),
                fact_summary: "用户喜欢简洁的回答".into(),
                created_at: now.clone(),
                confidence: 0.6,
                is_pinned: false,
            },
            0.95,
        ),
        (
            Experience {
                id: 2,
                agent_did: "d".into(),
                topic: "t".into(),
                fact_summary: "用户喜欢简洁的格式".into(), // very similar to #1
                created_at: now.clone(),
                confidence: 0.6,
                is_pinned: false,
            },
            0.85, // lowest score
        ),
        (
            Experience {
                id: 3,
                agent_did: "d".into(),
                topic: "t".into(),
                fact_summary: "email API uses OAuth2 tokens".into(), // diverse
                created_at: now.clone(),
                confidence: 0.6,
                is_pinned: false,
            },
            0.90, // middle score — normalized relevance > #2
        ),
    ];
    let result = mmr_rerank(candidates, 2, 0.7);
    assert_eq!(result.len(), 2);
    assert_eq!(result[0].0.id, 1, "Highest scoring selected first");
    // MMR: #3 has higher normalized relevance AND zero similarity to #1
    // while #2 has zero normalized relevance AND high similarity to #1
    assert_eq!(
        result[1].0.id, 3,
        "MMR should select diverse candidate over similar one"
    );
}

#[test]
fn compute_day_span_basic() {
    let days = vec!["2026-04-01".into(), "2026-04-08".into()];
    assert!((compute_day_span(&days) - 7.0).abs() < f64::EPSILON);
}

#[test]
fn compute_day_span_single() {
    let days = vec!["2026-04-01".into()];
    assert!((compute_day_span(&days) - 0.0).abs() < f64::EPSILON);
}

#[test]
fn compute_day_span_empty() {
    let days: Vec<String> = vec![];
    assert!((compute_day_span(&days) - 0.0).abs() < f64::EPSILON);
}

fn recall_test_db() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    crate::memory_migrations::run_all_migrations(&conn).unwrap();
    conn
}

#[test]
fn recall_stats_migration_creates_table() {
    let conn = recall_test_db();
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM l2_recall_stats", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn record_and_fetch_recall_events() {
    let conn = recall_test_db();
    let event1 = RecallEvent {
        fact_id: 42,
        composite_score: 0.85,
        query_hash: 12345,
    };
    let event2 = RecallEvent {
        fact_id: 42,
        composite_score: 0.90,
        query_hash: 67890,
    };
    record_recall_events(&conn, std::slice::from_ref(&event1));
    record_recall_events(&conn, std::slice::from_ref(&event2));

    let stats = batch_get_recall_stats(&conn, &[42]);
    let s = stats.get(&42).expect("stats should exist");
    assert_eq!(s.recall_count, 2);
    assert!((s.total_score - 1.75).abs() < 0.01);
    assert!((s.max_score - 0.90).abs() < 0.01);
    assert_eq!(s.unique_queries, 2);
    assert_eq!(s.query_hashes.len(), 2);
    assert_eq!(s.recall_days.len(), 1); // same day
}

#[test]
fn record_recall_deduplicates_query_hashes() {
    let conn = recall_test_db();
    let event = RecallEvent {
        fact_id: 1,
        composite_score: 0.8,
        query_hash: 999,
    };
    // Record same hash 3 times
    for _ in 0..3 {
        record_recall_events(&conn, std::slice::from_ref(&event));
    }
    let stats = batch_get_recall_stats(&conn, &[1]);
    let s = stats.get(&1).unwrap();
    assert_eq!(s.recall_count, 3, "recall_count increments every time");
    assert_eq!(
        s.unique_queries, 1,
        "unique_queries stays at 1 for same hash"
    );
}

#[test]
fn batch_get_recall_stats_empty() {
    let conn = recall_test_db();
    let stats = batch_get_recall_stats(&conn, &[999, 1000]);
    assert!(stats.is_empty());
}

// ── G1: Relationship State tests ──────────────────────────────────

fn relationship_test_db() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE relationship_state (
                user_did           TEXT PRIMARY KEY,
                current_focus      TEXT NOT NULL DEFAULT '[]',
                ongoing_goals      TEXT NOT NULL DEFAULT '[]',
                narrative          TEXT NOT NULL DEFAULT '',
                last_interaction_at TEXT,
                last_topic         TEXT NOT NULL DEFAULT '',
                interaction_count  INTEGER NOT NULL DEFAULT 0,
                streak_days        INTEGER NOT NULL DEFAULT 0,
                updated_at         TEXT NOT NULL DEFAULT '',
                active_hours       TEXT NOT NULL DEFAULT '{}'
            );",
    )
    .unwrap();
    conn
}

#[test]
fn relationship_state_default_when_no_row() {
    let conn = relationship_test_db();
    let state = get_relationship_state(&conn, "did:test:user1").unwrap();
    assert_eq!(state.user_did, "did:test:user1");
    assert_eq!(state.interaction_count, 0);
    assert_eq!(state.streak_days, 0);
    assert!(state.current_focus.is_empty());
    assert!(state.last_interaction_at.is_none());
}

#[test]
fn relationship_interaction_creates_and_increments() {
    let conn = relationship_test_db();
    let did = "did:test:user1";

    update_relationship_interaction(&conn, did, "Rust architecture", "UTC").unwrap();
    let s = get_relationship_state(&conn, did).unwrap();
    assert_eq!(s.interaction_count, 1);
    assert_eq!(s.last_topic, "Rust architecture");
    assert!(s.last_interaction_at.is_some());

    update_relationship_interaction(&conn, did, "deployment pipeline", "UTC").unwrap();
    let s = get_relationship_state(&conn, did).unwrap();
    assert_eq!(s.interaction_count, 2);
    assert_eq!(s.last_topic, "deployment pipeline");
}

#[test]
fn compute_streak_first_interaction() {
    let state = RelationshipState::default();
    assert_eq!(compute_streak_days(&state, "2026-04-09"), 1);
}

#[test]
fn compute_streak_same_day() {
    let state = RelationshipState {
        last_interaction_at: Some("2026-04-09T10:00:00+00:00".to_string()),
        streak_days: 3,
        ..Default::default()
    };
    assert_eq!(compute_streak_days(&state, "2026-04-09"), 3);
}

#[test]
fn compute_streak_consecutive_day() {
    let state = RelationshipState {
        last_interaction_at: Some("2026-04-08T23:00:00+00:00".to_string()),
        streak_days: 5,
        ..Default::default()
    };
    assert_eq!(compute_streak_days(&state, "2026-04-09"), 6);
}

#[test]
fn compute_streak_broken() {
    let state = RelationshipState {
        last_interaction_at: Some("2026-04-06T10:00:00+00:00".to_string()),
        streak_days: 10,
        ..Default::default()
    };
    // Gap of 3 days — streak resets
    assert_eq!(compute_streak_days(&state, "2026-04-09"), 1);
}

#[test]
fn humanize_elapsed_just_now() {
    let now = chrono::Utc::now().to_rfc3339();
    let result = humanize_elapsed(&now);
    assert_eq!(result, "刚刚");
}

#[test]
fn humanize_elapsed_hours() {
    let ts = (chrono::Utc::now() - chrono::Duration::hours(3)).to_rfc3339();
    let result = humanize_elapsed(&ts);
    assert_eq!(result, "3小时前");
}

#[test]
fn humanize_elapsed_yesterday() {
    let ts = (chrono::Utc::now() - chrono::Duration::days(1)).to_rfc3339();
    let result = humanize_elapsed(&ts);
    assert_eq!(result, "昨天");
}

#[test]
fn humanize_elapsed_days() {
    let ts = (chrono::Utc::now() - chrono::Duration::days(4)).to_rfc3339();
    let result = humanize_elapsed(&ts);
    assert_eq!(result, "4天前");
}

#[test]
fn humanize_elapsed_invalid() {
    assert_eq!(humanize_elapsed("not-a-date"), "未知时间");
}

#[test]
fn extract_topic_empty() {
    assert_eq!(extract_topic_from_message(""), "");
}

#[test]
fn extract_topic_first_sentence_chinese() {
    let msg = "帮我检查一下服务器状态。然后看看日志。";
    assert_eq!(extract_topic_from_message(msg), "帮我检查一下服务器状态");
}

#[test]
fn extract_topic_first_sentence_english() {
    let msg = "Fix the login bug. Then deploy.";
    assert_eq!(extract_topic_from_message(msg), "Fix the login bug");
}

#[test]
fn extract_topic_long_message_truncates() {
    let msg = "这是一段非常非常长的消息内容用来测试截断功能确保不会超过五十个字符的限制因为我们需要保持主题简洁明了不要太长了好吗";
    let result = extract_topic_from_message(msg);
    assert!(result.chars().count() <= 51); // 50 + "…"
    assert!(result.ends_with('…'));
}

#[test]
fn format_context_empty_for_new_user() {
    let state = RelationshipState::default();
    assert_eq!(format_relationship_context(&state, &[]), "");
}

#[test]
fn format_context_basic() {
    let now = chrono::Utc::now().to_rfc3339();
    let state = RelationshipState {
        user_did: "did:test:u".to_string(),
        last_interaction_at: Some(now),
        last_topic: "Rust代码审查".to_string(),
        interaction_count: 5,
        streak_days: 3,
        current_focus: vec![FocusTopic {
            topic: "Aegis Runtime架构".to_string(),
            since: "2026-04-01".to_string(),
            strength: 0.8,
        }],
        ongoing_goals: vec![GoalSummary {
            title: "发布v2.0".to_string(),
            progress_pct: 0.6,
            updated_at: "2026-04-09".to_string(),
        }],
        narrative: "用户专注于系统架构改进".to_string(),
        updated_at: "2026-04-09".to_string(),
        active_hours: std::collections::HashMap::new(),
    };
    // Goals from G5 table take precedence over legacy ongoing_goals
    let goals = vec![Goal {
        id: 1,
        user_did: "did:test:u".to_string(),
        title: "发布v2.0".to_string(),
        description: String::new(),
        status: GoalStatus::Active,
        parent_goal_id: None,
        progress_pct: 0.6,
        milestones: vec![],
        created_at: "2026-04-09".to_string(),
        target_date: None,
        last_progress_at: None,
        source: GoalSource::Explicit,
    }];
    let ctx = format_relationship_context(&state, &goals);
    assert!(ctx.contains("话题: Rust代码审查"));
    assert!(ctx.contains("Aegis Runtime架构"));
    assert!(ctx.contains("发布v2.0(60%)"));
    assert!(ctx.contains("用户专注于系统架构改进"));
    assert!(ctx.contains("累计交互: 5 次"));
}

#[test]
fn format_context_filters_weak_focus() {
    let state = RelationshipState {
        user_did: "did:test:u".to_string(),
        interaction_count: 1,
        last_interaction_at: Some(chrono::Utc::now().to_rfc3339()),
        last_topic: "test".to_string(),
        current_focus: vec![
            FocusTopic {
                topic: "weak".to_string(),
                since: "2026-04-01".to_string(),
                strength: 0.2, // below 0.3 threshold
            },
            FocusTopic {
                topic: "strong".to_string(),
                since: "2026-04-01".to_string(),
                strength: 0.9,
            },
        ],
        ..Default::default()
    };
    let ctx = format_relationship_context(&state, &[]);
    assert!(!ctx.contains("weak"));
    assert!(ctx.contains("strong"));
}

#[test]
fn relationship_state_migration_creates_table() {
    let conn = Connection::open_in_memory().unwrap();
    crate::memory_migrations::run_all_migrations(&conn).unwrap();
    // Verify table exists by inserting a row
    conn.execute(
        "INSERT INTO relationship_state (user_did, updated_at) VALUES ('test', '2026-04-09')",
        [],
    )
    .unwrap();
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM relationship_state", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(count, 1);
}

// ── Layer C: Companion Growth tests ──────────────────────────────

#[test]
fn companion_identity_default_and_serde() {
    let id = CompanionIdentity::default();
    assert_eq!(id.growth_stage, GrowthStage::ChuYu);
    assert!(id.core_values.contains("Aegis"));

    // Serialize and deserialize round-trip
    let json = serde_json::to_string(&id).unwrap();
    let deser: CompanionIdentity = serde_json::from_str(&json).unwrap();
    assert_eq!(deser.growth_stage, GrowthStage::ChuYu);
    assert_eq!(deser.personality.warmth, 0.5);
    assert_eq!(deser.personality.humor, 0.2);
}

#[test]
fn growth_stage_from_interaction_count() {
    assert_eq!(GrowthStage::from_interaction_count(0), GrowthStage::ChuYu);
    assert_eq!(GrowthStage::from_interaction_count(50), GrowthStage::ChuYu);
    assert_eq!(
        GrowthStage::from_interaction_count(100),
        GrowthStage::XiangZhi
    );
    assert_eq!(
        GrowthStage::from_interaction_count(499),
        GrowthStage::XiangZhi
    );
    assert_eq!(GrowthStage::from_interaction_count(500), GrowthStage::MoQi);
    assert_eq!(
        GrowthStage::from_interaction_count(2000),
        GrowthStage::GongSheng
    );
    assert_eq!(
        GrowthStage::from_interaction_count(10000),
        GrowthStage::JueWu
    );
    assert_eq!(
        GrowthStage::from_interaction_count(50000),
        GrowthStage::JueWu
    );
}

#[test]
fn growth_stage_mixed_acceleration() {
    // Floor = ChuYu (count=50), depth=0.6 -> accelerate to XiangZhi
    assert_eq!(GrowthStage::from_mixed(50, 0.6), GrowthStage::XiangZhi);
    // Floor = XiangZhi (count=200), depth=0.6 -> accelerate to MoQi
    assert_eq!(GrowthStage::from_mixed(200, 0.6), GrowthStage::MoQi);
    // Floor = JueWu (count=10000), depth=0.6 -> can't go beyond JueWu
    assert_eq!(GrowthStage::from_mixed(10000, 0.6), GrowthStage::JueWu);
    // Floor = ChuYu (count=50), depth=0.3 -> no acceleration
    assert_eq!(GrowthStage::from_mixed(50, 0.3), GrowthStage::ChuYu);
}

#[test]
fn growth_stage_str_round_trip() {
    for stage in &[
        GrowthStage::ChuYu,
        GrowthStage::XiangZhi,
        GrowthStage::MoQi,
        GrowthStage::GongSheng,
        GrowthStage::JueWu,
    ] {
        assert_eq!(GrowthStage::from_str_lossy(stage.as_str()), *stage);
    }
    // Unknown string defaults to ChuYu
    assert_eq!(GrowthStage::from_str_lossy("unknown"), GrowthStage::ChuYu);
}

#[test]
fn apply_growth_evidence_updates_depth_and_logs() {
    let conn = Connection::open_in_memory().unwrap();
    crate::memory_migrations::run_all_migrations(&conn).unwrap();

    // Apply positive evidence to rapport
    apply_growth_evidence(
        &conn,
        "rapport",
        0.05,
        "User adapted to style",
        &serde_json::json!({"source": "test"}),
    )
    .unwrap();

    // Verify depth increased
    let (depth, velocity, evidence_count): (f64, f64, i64) = conn
            .query_row(
                "SELECT depth, velocity, evidence_count FROM growth_dimensions WHERE dimension_key = 'rapport'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
    assert!(depth >= 0.05, "depth should be >= 0.05, got {depth}");
    assert!(
        velocity > 0.0,
        "velocity should be positive after positive delta"
    );
    assert!(evidence_count >= 1, "evidence_count should be >= 1");

    // Verify growth_log entry was created
    let (event_type, dim_key, delta): (String, String, f64) = conn
        .query_row(
            "SELECT event_type, dimension_key, delta FROM growth_log ORDER BY id DESC LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(event_type, "dimension_shift");
    assert_eq!(dim_key, "rapport");
    assert!((delta - 0.05).abs() < 1e-6);
}

#[test]
fn apply_growth_evidence_depth_unbounded_min_zero() {
    let conn = Connection::open_in_memory().unwrap();
    crate::memory_migrations::run_all_migrations(&conn).unwrap();

    // Apply large negative delta — should clamp at 0.0
    apply_growth_evidence(
        &conn,
        "trust",
        -10.0,
        "Large negative test",
        &serde_json::json!({}),
    )
    .unwrap();
    let depth: f64 = conn
        .query_row(
            "SELECT depth FROM growth_dimensions WHERE dimension_key = 'trust'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(depth, 0.0, "depth should be clamped at 0.0");

    // Apply large positive delta — should be unbounded
    apply_growth_evidence(
        &conn,
        "trust",
        100.0,
        "Large positive test",
        &serde_json::json!({}),
    )
    .unwrap();
    let depth: f64 = conn
        .query_row(
            "SELECT depth FROM growth_dimensions WHERE dimension_key = 'trust'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(depth, 100.0, "depth should be unbounded upward");
}

#[test]
fn format_self_awareness_basic() {
    let identity = CompanionIdentity {
        name: "Aegis".into(),
        core_values: "Care\nProtection".into(),
        growth_stage: GrowthStage::XiangZhi,
        created_at: "2026-04-10T00:00:00+00:00".into(),
        ..Default::default()
    };
    let dims = vec![GrowthDimension {
        dimension_key: "rapport".into(),
        display_name: "\u{9ed8}\u{5951} Rapport".into(),
        depth: 0.43,
        velocity: 0.005,
        evidence_count: 50,
        last_evidence_at: None,
        description: String::new(),
        updated_at: String::new(),
    }];
    let output = format_self_awareness(&identity, &dims, 152);
    assert!(output.contains("Aegis"));
    assert!(output.contains("Coming to Know"));
    assert!(output.contains("152 interactions"));
    assert!(output.contains("Rapport"));
    assert!(output.contains("0.43"));
    assert!(output.contains("(rising)"));
}

#[test]
fn personality_weights_serde_with_extra() {
    let mut pw = PersonalityWeights::default();
    pw.extra.insert("creativity".to_string(), 0.8);
    let json = serde_json::to_string(&pw).unwrap();
    let deser: PersonalityWeights = serde_json::from_str(&json).unwrap();
    assert_eq!(deser.warmth, 0.5);
    assert_eq!(deser.extra.get("creativity"), Some(&0.8));
}

#[test]
fn adjust_personality_trait_known_and_custom() {
    let conn = Connection::open_in_memory().unwrap();
    crate::memory_migrations::run_all_migrations(&conn).unwrap();

    // Adjust known trait
    adjust_personality_trait(&conn, "warmth", 0.8).unwrap();
    let json: String = conn
        .query_row(
            "SELECT personality FROM companion_identity WHERE id = 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let pw: PersonalityWeights = serde_json::from_str(&json).unwrap();
    assert!((pw.warmth - 0.8).abs() < 1e-6);

    // Adjust custom trait
    adjust_personality_trait(&conn, "creativity", 0.9).unwrap();
    let json: String = conn
        .query_row(
            "SELECT personality FROM companion_identity WHERE id = 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let pw: PersonalityWeights = serde_json::from_str(&json).unwrap();
    assert_eq!(pw.extra.get("creativity"), Some(&0.9));
}

#[test]
fn log_and_query_growth_events() {
    let conn = Connection::open_in_memory().unwrap();
    crate::memory_migrations::run_all_migrations(&conn).unwrap();

    let id = log_growth_event(
        &conn,
        &NewGrowthEvent {
            event_type: "milestone".into(),
            dimension_key: Some("rapport".into()),
            detail: "Stage transition".into(),
            delta: None,
            metadata: serde_json::json!({"from": "chu_yu", "to": "xiang_zhi"}),
        },
    )
    .unwrap();
    assert!(id > 0);

    let events = query_growth_log(&conn, Some("milestone"), 10).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_type, "milestone");
    assert_eq!(events[0].detail, "Stage transition");

    // Query all event types
    let all = query_growth_log(&conn, None, 10).unwrap();
    assert_eq!(all.len(), 1);
}
