//! E2E tests for Telegram HTML formatting and the skill-registry DB flow.
//!
//! These tests use real Telegram API and production-like DB setup to verify:
//! 1. HTML formatting renders correctly in Telegram
//! 2. The skill_proposals registry reads work after DB migration
//!
//! Run with:
//!     cargo test -p aegis-runtime-core --test telegram_html_e2e -- --ignored --nocapture

mod common;

use aegis_runtime_core::memory::MemoryHandle;
use aegis_runtime_core::telegram::{markdown_to_telegram_html, send_html};
use teloxide::prelude::*;
use teloxide::types::ChatId;

use common::{get_env, init_tracing};

// ── Test 1: Send real HTML messages to Telegram ──────────────────────────────

#[tokio::test]
#[ignore]
async fn e2e_telegram_html_formatting() {
    init_tracing();
    let token = get_env("TELOXIDE_TOKEN");
    let chat_id: i64 = get_env("ADMIN_CHAT_ID")
        .parse()
        .expect("ADMIN_CHAT_ID must be i64");

    let bot = Bot::new(&token);
    let cid = ChatId(chat_id);

    // Test 1: Simple bold/italic/code
    let md1 = "**测试 HTML 格式化**\n\n这是 *斜体* 和 `inline code` 测试。";
    let html1 = markdown_to_telegram_html(md1);
    eprintln!("HTML1: {html1}");
    send_html(&bot, cid, &html1)
        .await
        .expect("send_html #1 failed");

    // Test 2: Code block
    let md2 = "代码块测试：\n\n```rust\nfn main() {\n    println!(\"Hello, Aegis Runtime!\");\n}\n```\n\n代码块结束。";
    let html2 = markdown_to_telegram_html(md2);
    eprintln!("HTML2: {html2}");
    send_html(&bot, cid, &html2)
        .await
        .expect("send_html #2 failed");

    // Test 3: Link
    let md3 = "链接测试：[点击这里](https://example.com)";
    let html3 = markdown_to_telegram_html(md3);
    eprintln!("HTML3: {html3}");
    send_html(&bot, cid, &html3)
        .await
        .expect("send_html #3 failed");

    // Test 4: Realistic LLM output (the type that was failing before)
    let md4 = r#"你好！👋 需要我帮您做什么？

| 选项 | 说明 |
|------|------|
| 查询 | 搜索信息 |
| 提醒 | 创建提醒 |

**主要功能：**
1. 创建和管理 *技能插件*
2. 设置 `定时提醒`
3. 查询 **加密货币** 价格

如果有任何问题，请随时告诉我！"#;
    let html4 = markdown_to_telegram_html(md4);
    eprintln!("HTML4: {html4}");
    send_html(&bot, cid, &html4)
        .await
        .expect("send_html #4 failed");

    // Test 5: Special chars that broke MarkdownV2
    let md5 = "特殊字符：价格 $100.50 - 涨幅 +3.2%\n折扣: 30% off! (限时优惠)\n#标签 @用户";
    let html5 = markdown_to_telegram_html(md5);
    eprintln!("HTML5: {html5}");
    send_html(&bot, cid, &html5)
        .await
        .expect("send_html #5 failed");

    eprintln!("\n✅ All 5 HTML messages sent successfully! Check Telegram for formatting.");
}

// ── Test 2: skill registry DB flow (row create → get → verify) ──────────────

#[tokio::test]
#[ignore]
async fn e2e_skill_registry_db_flow() {
    init_tracing();

    // Use in-memory DB (spawn_in_memory creates fresh tables with migration)
    let memory = MemoryHandle::spawn_in_memory().expect("memory init");

    // 1. Create a proposal
    let proposal_id = memory
        .create_skill_proposal("(pending)", "did:test:e2e", "创建增强版提醒技能", None)
        .await
        .expect("create_skill_proposal failed");

    eprintln!("Created proposal #{proposal_id}");
    assert!(proposal_id > 0);

    // 2. Get the proposal — this was failing before migration fix
    let proposal = memory
        .get_skill_proposal(proposal_id)
        .await
        .expect("get_skill_proposal returned Err");

    assert!(
        proposal.is_some(),
        "get_skill_proposal returned None — version column likely missing!"
    );

    let p = proposal.unwrap();
    eprintln!(
        "Proposal: id={}, skill_name={}, status={}, version={}",
        p.id, p.skill_name, p.status, p.version
    );

    assert_eq!(p.id, proposal_id);
    assert_eq!(p.status, "pending_approval");
    assert_eq!(p.version, 1, "default version should be 1");
    assert_eq!(p.user_request, "创建增强版提醒技能");

    // 3. List proposals — also uses the version column
    let proposals = memory
        .list_skill_proposals(Some("did:test:e2e"), Some("pending_approval"))
        .await
        .expect("list_skill_proposals failed");

    assert_eq!(proposals.len(), 1);
    assert_eq!(proposals[0].id, proposal_id);

    eprintln!("\n✅ skill registry DB flow works correctly.");
}

// ── Test 3: Simulate production DB schema migration ──────────────────────────
//
// This simulates what happens on a production server:
// 1. Old DB exists with tables created by old schema (no version column)
// 2. New code starts with ALTER TABLE migration
// 3. Queries should work correctly

#[tokio::test]
#[ignore]
async fn e2e_production_db_migration_scenario() {
    init_tracing();

    // Create a temp file for the DB
    let tmp = tempfile::NamedTempFile::new().expect("temp file");
    let db_path = tmp.path().to_path_buf();
    eprintln!("Test DB path: {}", db_path.display());

    // Step 1: Create OLD schema (without version column) using raw SQLite
    {
        let conn = rusqlite::Connection::open(&db_path).expect("open db");
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS skill_proposals (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                skill_name TEXT NOT NULL,
                requested_by TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'pending_approval',
                stage_error TEXT,
                retry_count INTEGER NOT NULL DEFAULT 0,
                user_request TEXT NOT NULL DEFAULT '',
                proposal_summary TEXT,
                design_json TEXT,
                generated_source TEXT,
                generated_manifest TEXT,
                generated_cargo_toml TEXT,
                compiled_wasm BLOB,
                audit_static_result TEXT,
                audit_llm_result TEXT,
                audit_overridden INTEGER NOT NULL DEFAULT 0,
                test_result TEXT,
                requires_auth INTEGER NOT NULL DEFAULT 0,
                auth_domain TEXT,
                total_llm_calls INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now'))
            );",
        )
        .expect("create old schema");

        // Insert a row with old schema
        conn.execute(
            "INSERT INTO skill_proposals (skill_name, requested_by, status, user_request)
             VALUES ('test_skill', 'did:test:migration', 'pending_approval', '测试技能')",
            [],
        )
        .expect("insert old row");

        // Verify: old schema has no version column
        let cols: Vec<String> = conn
            .prepare("PRAGMA table_info(skill_proposals)")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();
        assert!(
            !cols.contains(&"version".to_string()),
            "Old schema should NOT have version column"
        );
        eprintln!("Step 1: Old schema created (columns: {cols:?})");
    }

    // Step 2: Open with MemoryHandle (which runs migration)
    let memory = MemoryHandle::spawn(&db_path).expect("spawn with migration");

    // Step 3: Verify the migration worked — get the old proposal
    let proposal = memory
        .get_skill_proposal(1)
        .await
        .expect("get_skill_proposal after migration");

    assert!(
        proposal.is_some(),
        "CRITICAL: get_skill_proposal returned None after migration! The version column migration is broken."
    );

    let p = proposal.unwrap();
    eprintln!(
        "Step 3: Retrieved proposal after migration: id={}, skill={}, version={}",
        p.id, p.skill_name, p.version
    );
    assert_eq!(p.skill_name, "test_skill");
    assert_eq!(p.version, 1, "migrated row should have default version=1");

    // Step 4: Create new proposal (should also work)
    let new_id = memory
        .create_skill_proposal("new_skill", "did:test:migration", "新技能请求", None)
        .await
        .expect("create new proposal after migration");

    let new_p = memory
        .get_skill_proposal(new_id)
        .await
        .expect("get new proposal")
        .expect("new proposal should exist");

    assert_eq!(new_p.version, 1);
    eprintln!(
        "Step 4: Created and retrieved new proposal after migration: id={}",
        new_id
    );

    eprintln!("\n✅ Production DB migration scenario passed!");
}
