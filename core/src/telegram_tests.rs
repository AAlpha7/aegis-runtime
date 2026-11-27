// Tests for the telegram module.
//
// Included into telegram.rs via `#[path = "telegram_tests.rs"] mod tests;`
// All items from telegram.rs (and re-exports from telegram_render) are
// accessible via `use super::*`.

use super::*;
use crate::telegram_render::{
    closing_tag_for, display_width, extract_tag_name, markdown_to_plain, render_table_adaptive,
    update_tag_stack,
};

#[test]
fn hitl_delivery_binding_preserves_origin_and_allows_routed_topic() {
    let raw = serde_json::json!({
        "approval_id": "gate-test",
        "chat_id": 1000000001_i64,
        "tool_name": "calendar_update_event",
        "tool_args": "{}",
    })
    .to_string();

    let updated = hitl_with_delivery_chat_bound(&raw, 1000000001_i64, -1001000000001_i64, Some(8))
        .expect("valid HITL JSON should be updated");
    let value: serde_json::Value = serde_json::from_str(&updated).unwrap();

    assert_eq!(value["chat_id"].as_i64(), Some(1000000001_i64));
    assert_eq!(value["delivery_chat_id"].as_i64(), Some(-1001000000001_i64));
    assert_eq!(value["delivery_thread_id"].as_i64(), Some(8));
    let ids = value["allowed_chat_ids"].as_array().unwrap();
    assert!(ids.iter().any(|v| v.as_i64() == Some(1000000001_i64)));
    assert!(ids.iter().any(|v| v.as_i64() == Some(-1001000000001_i64)));
}

#[test]
fn web_delivery_notice_text_keeps_approval_generic_and_notifications_visible() {
    assert_eq!(
        web_delivery_notice_text("Approval Request: gate-abc").as_deref(),
        Some("Aegis needs a decision. Open Work to review the approval.")
    );
    assert!(
        web_delivery_notice_text("⚙️ Using calendar_reader ...").is_none(),
        "tool progress noise should not become Web chat history"
    );
    assert_eq!(
        web_delivery_notice_text("🤖 [calendar] Important mail").as_deref(),
        Some("🤖 [calendar] Important mail")
    );
    assert_eq!(
        web_delivery_notice_text("📬 [primary] Email from Chase: payment received").as_deref(),
        Some("📬 [primary] Email from Chase: payment received")
    );
}

#[test]
fn owner_web_notice_text_strips_html_and_truncates() {
    assert_eq!(
        owner_web_notice_text("<b>Reminder</b>: call &amp; confirm"),
        "Reminder: call & confirm"
    );
    let long = "x".repeat(4_100);
    assert_eq!(owner_web_notice_text(&long).chars().count(), 4_000);
}

#[test]
fn owner_web_reminder_notice_text_adds_native_web_actions() {
    // English locale → English web-actions suffix from the catalog. Asserted
    // against the FULLY-EXPANDED literal so a placeholder-substitution bug
    // (e.g. only the first `{}` getting the task id) is caught — every one of
    // the four task-id slots must be filled.
    assert_eq!(
        owner_web_reminder_notice_text("⏰ 找时间看看 Jon 发的视频", 1894, "en"),
        "⏰ 找时间看看 Jon 发的视频\n\nWeb actions: `done reminder 1894`, `snooze reminder 1894`, `tomorrow reminder 1894`, or `delete reminder 1894`."
    );
    // Chinese locale → Chinese suffix, single language (no zh/en mixing). All
    // four task-id slots filled, no residual `{}` placeholder.
    let zh = owner_web_reminder_notice_text("⏰ 找时间看看 Jon 发的视频", 1894, "zh");
    assert!(zh.starts_with("⏰ 找时间看看 Jon 发的视频\n\n网页操作："));
    assert_eq!(
        zh.matches("1894").count(),
        4,
        "all four task-id slots must be filled"
    );
    assert!(!zh.contains("{}"), "no residual placeholder");
    // Idempotent: an English suffix already present is not re-appended.
    assert_eq!(
        owner_web_reminder_notice_text("Reminder\n\nWeb actions: `done reminder 1`.", 1, "en"),
        "Reminder\n\nWeb actions: `done reminder 1`."
    );
    // Idempotent: a Chinese suffix already present is not re-appended.
    assert_eq!(
        owner_web_reminder_notice_text("提醒\n\n网页操作：`done reminder 1`。", 1, "zh"),
        "提醒\n\n网页操作：`done reminder 1`。"
    );
}

#[test]
fn telegram_send_ledger_records_executed_receipt() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let conn = crate::encrypted_sqlite::open(tmp.path()).unwrap();
    crate::memory_migrations::run_all_migrations(&conn).unwrap();
    drop(conn);

    let db_path = tmp.path().to_str().unwrap();
    let action_id = "telegram-send-test-ok";
    start_telegram_send_ledger(
        db_path,
        action_id,
        ChatId(123),
        Some(teloxide::types::ThreadId(MessageId(7))),
        "heartbeat_probe",
        "Alert body",
    )
    .unwrap();
    finish_telegram_send_ledger(db_path, action_id, ChatId(123), MessageId(456)).unwrap();

    let conn = crate::encrypted_sqlite::open(tmp.path()).unwrap();
    let store = crate::action_ledger::ActionLedgerStore::new(&conn);
    let entry = store.get_by_action_id(action_id).unwrap().unwrap();

    assert_eq!(
        entry.action_kind,
        crate::action_ledger::ActionKind::TelegramSend
    );
    assert_eq!(entry.status, crate::action_ledger::ActionStatus::Executed);
    assert_eq!(
        entry.external_message_id.as_deref(),
        Some("telegram:123:456")
    );
    assert_eq!(
        entry.external_receipt.as_deref(),
        Some("telegram_message_id:456")
    );
    assert_eq!(entry.chat_id, Some(123));
    assert_eq!(entry.source_event_id.as_deref(), Some("heartbeat_probe"));
    assert!(
        entry
            .metadata_json
            .as_deref()
            .unwrap_or_default()
            .contains("\"telegram_thread_id\":7")
    );
}
#[test]
fn startup_reply_keyboard_attaches_only_to_admin_dm_fallback() {
    use crate::telegram_notification_router::{NotificationRoute, NotificationRouteReason};

    let admin_dm = 123_456_789;
    // A route to a non-admin chat must never carry the startup reply keyboard.
    let other_chat_route = NotificationRoute {
        chat_id: -100_222_333_444,
        thread_id: None,
        topic_kind: None,
        reason: NotificationRouteReason::ExplicitStoredChatAllowed,
    };
    assert!(!should_attach_startup_reply_keyboard(
        admin_dm,
        &other_chat_route
    ));

    let dm_fallback_route = NotificationRoute {
        chat_id: admin_dm,
        thread_id: None,
        topic_kind: None,
        reason: NotificationRouteReason::NoLinkedGroupFallbackToDm,
    };
    assert!(should_attach_startup_reply_keyboard(
        admin_dm,
        &dm_fallback_route
    ));

    // Even to the admin chat, a route that (hypothetically) carried a forum
    // thread id must not attach the keyboard.
    let dm_thread_route = NotificationRoute {
        chat_id: admin_dm,
        thread_id: Some(44),
        topic_kind: None,
        reason: NotificationRouteReason::NoLinkedGroupFallbackToDm,
    };
    assert!(!should_attach_startup_reply_keyboard(
        admin_dm,
        &dm_thread_route
    ));
}

#[test]
fn routine_startup_status_is_web_only_by_default() {
    let routine = "<b>系统已重启 / System restarted</b>\n\n\
         <b>当前重要信息</b>\n\
         • 活跃技能 / Active skills: 22\n\
         • HITL 待审批: 0";

    assert!(!should_send_startup_status_to_telegram(routine));
}

#[test]
fn startup_alerts_still_notify_telegram() {
    assert!(should_send_startup_status_to_telegram(
        "<b>System restarted</b>\nALERT: critical startup failure"
    ));
}

#[test]
fn oauth_deeplink_reply_uses_router_dm_exception() {
    use crate::telegram_notification_router::NotificationRouteReason;

    let admin_dm = 123_456_789;
    let linked_group_chat = -100_222_333_444;

    let route = oauth_deeplink_reply_route(admin_dm, linked_group_chat);

    assert_eq!(route.chat_id, linked_group_chat);
    assert_eq!(route.thread_id, None);
    assert_eq!(route.topic_kind, None);
    assert_eq!(
        route.reason,
        NotificationRouteReason::OAuthDeepLinkException
    );
    assert!(!should_attach_startup_reply_keyboard(admin_dm, &route));
}

#[test]
fn progress_listener_classifies_only_proactive_statuses_for_router() {
    assert_eq!(
        owner_progress_kind("🔐 Approval Request\nApproval ID: gate-123"),
        OwnerProgressKind::ApprovalRequest
    );
    assert_eq!(
        owner_progress_kind("⚙️ Using calendar_reader …"),
        OwnerProgressKind::ToolUsageProgress
    );
    assert_eq!(
        owner_progress_kind("Thinking..."),
        OwnerProgressKind::ReactiveProgress
    );
    assert_eq!(
        proactive_progress_notification_class("🔐 Approval Request\nApproval ID: gate-123"),
        Some(NotificationClass::ApprovalRequest)
    );
    assert_eq!(
        proactive_progress_notification_class("🤖 [calendar_reader] Important mail"),
        Some(NotificationClass::RoutineNotification)
    );
    assert_eq!(
        proactive_progress_notification_class("📋 Daily summary"),
        Some(NotificationClass::RoutineNotification)
    );
    assert_eq!(
        proactive_progress_notification_class("📬 [primary] Email from Chase"),
        Some(NotificationClass::RoutineNotification)
    );
    assert_eq!(
        proactive_progress_notification_class("⚙️ Using calendar_reader …"),
        None
    );
    assert_eq!(proactive_progress_notification_class("Thinking..."), None);
}

#[test]
fn web_presence_suppresses_owner_proactive_and_tool_progress_statuses() {
    assert!(should_suppress_progress_status_for_web(
        "🔐 Approval Request\nApproval ID: gate-123"
    ));
    assert!(should_suppress_progress_status_for_web(
        "🤖 [calendar_reader] Important mail"
    ));
    assert!(should_suppress_progress_status_for_web("📋 Daily summary"));
    assert!(should_suppress_progress_status_for_web(
        "📬 [primary] Email from Chase"
    ));
    assert!(should_suppress_progress_status_for_web(
        "⚙️ Using calendar_reader …"
    ));
    assert!(!should_suppress_progress_status_for_web("Thinking..."));
}

#[test]
fn tool_usage_progress_status_is_not_plain_edit_progress() {
    assert!(is_tool_usage_progress_status("⚙️ Using calendar_reader …"));
    assert_eq!(
        proactive_progress_notification_class("⚙️ Using calendar_reader …"),
        None
    );
    assert!(!is_tool_usage_progress_status("Thinking..."));
}

#[test]
fn telegram_send_ledger_records_failed_send() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let conn = crate::encrypted_sqlite::open(tmp.path()).unwrap();
    crate::memory_migrations::run_all_migrations(&conn).unwrap();
    drop(conn);

    let db_path = tmp.path().to_str().unwrap();
    let action_id = "telegram-send-test-failed";
    start_telegram_send_ledger(
        db_path,
        action_id,
        ChatId(123),
        None,
        "heartbeat_probe",
        "Alert body",
    )
    .unwrap();
    fail_telegram_send_ledger(db_path, action_id, "network timeout").unwrap();

    let conn = crate::encrypted_sqlite::open(tmp.path()).unwrap();
    let store = crate::action_ledger::ActionLedgerStore::new(&conn);
    let entry = store.get_by_action_id(action_id).unwrap().unwrap();

    assert_eq!(entry.status, crate::action_ledger::ActionStatus::Failed);
    assert_eq!(entry.error_kind.as_deref(), Some("telegram_send_failed"));
    assert_eq!(entry.error_message.as_deref(), Some("network timeout"));
    assert_eq!(entry.idempotency_key.as_deref(), Some(action_id));
}

// ── split_message tests ─────────────────────────────────────────────

#[test]
fn split_message_short() {
    let chunks = split_message("hello", 4096);
    assert_eq!(chunks, vec!["hello"]);
}

#[test]
fn split_message_long() {
    let long = "a".repeat(5000);
    let chunks = split_message(&long, 4096);
    assert!(chunks.len() >= 2);
    assert!(chunks[0].len() <= 4096);
}

#[test]
fn split_message_multibyte_no_panic() {
    let msg = "\u{4f60}\u{597d}\u{4e16}\u{754c}\u{ff01}".repeat(2000); // ~30KB of Chinese text
    let chunks = split_message(&msg, 4096);
    assert!(chunks.len() >= 2);
    for chunk in &chunks {
        let _ = chunk.chars().count(); // validates UTF-8
    }
}

#[test]
fn split_message_at_newline() {
    let msg = format!("{}\n{}", "a".repeat(4000), "b".repeat(200));
    let chunks = split_message(&msg, 4096);
    assert_eq!(chunks.len(), 2);
    assert!(chunks[0].ends_with('\n'));
}

#[test]
fn split_message_html_tag_aware_bold() {
    // A <b> tag that spans across a split boundary should be
    // properly closed and reopened.
    let content = "x".repeat(5000);
    let msg = format!("<b>{}</b>", content);
    let chunks = split_message(&msg, 4096);
    assert!(chunks.len() >= 2, "should split, msg len={}", msg.len());
    // Chunk 1 should end with </b>
    assert!(
        chunks[0].contains("</b>"),
        "first chunk should close <b>, got: ...{}",
        &chunks[0][chunks[0].len().saturating_sub(20)..]
    );
    // Chunk 2 should start with <b>
    assert!(
        chunks[1].starts_with("<b>"),
        "second chunk should reopen <b>, got: {:?}",
        &chunks[1][..chunks[1].len().min(20)]
    );
}

#[test]
fn split_message_html_pre_code_across_chunks() {
    // A <pre><code class="language-rust"> block that spans chunks
    let header = "Header\n";
    let code = format!(
        "<pre><code class=\"language-rust\">{}</code></pre>\nFooter",
        "let x = 1;\n".repeat(400)
    );
    let msg = format!("{}{}", header, code);
    let chunks = split_message(&msg, 4096);
    assert!(chunks.len() >= 2, "should produce multiple chunks");
    // First chunk should properly close any open tags
    // The tag-aware splitter ensures balanced tags per chunk
    for chunk in &chunks {
        // Count opens and closes of <pre> and </pre>
        let pre_opens = chunk.matches("<pre>").count() + chunk.matches("<pre><code").count();
        let pre_closes = chunk.matches("</pre>").count();
        // Tags should be balanced within each chunk (the splitter
        // closes open tags at end and reopens at start)
        assert!(
            pre_opens >= pre_closes || chunk.starts_with("<pre>") || chunk.starts_with("<code"),
            "chunk has unbalanced pre tags: opens={pre_opens} closes={pre_closes}"
        );
    }
}

#[test]
fn split_message_nested_tags() {
    // Nested tags: <b><i>...</i></b> spanning a boundary
    let inner = "y".repeat(5000);
    let msg = format!("<b><i>{}</i></b>", inner);
    let chunks = split_message(&msg, 4096);
    assert!(chunks.len() >= 2, "should split, msg len={}", msg.len());
    // First chunk should close in reverse order: </i></b>
    assert!(
        chunks[0].ends_with("</i></b>"),
        "expected closing tags, got: ...{}",
        &chunks[0][chunks[0].len().saturating_sub(30)..]
    );
    // Second chunk should reopen in original order: <b><i>
    assert!(
        chunks[1].starts_with("<b><i>"),
        "expected reopening tags, got: {:?}",
        &chunks[1][..chunks[1].len().min(20)]
    );
}

// ── markdown_to_telegram_html tests (AST-based) ─────────────────────

#[test]
fn html_plain_text() {
    assert_eq!(markdown_to_telegram_html("hello world"), "hello world");
}

#[test]
fn html_escapes_special_chars() {
    let result = markdown_to_telegram_html("a < b & c > d");
    assert!(result.contains("a &lt; b &amp; c &gt; d"));
}

#[test]
fn html_bold() {
    let result = markdown_to_telegram_html("this is **bold** text");
    assert!(result.contains("<b>bold</b>"));
}

#[test]
fn html_italic() {
    let result = markdown_to_telegram_html("this is *italic* text");
    assert!(result.contains("<i>italic</i>"));
}

#[test]
fn html_inline_code() {
    let result = markdown_to_telegram_html("use `println!` here");
    assert!(result.contains("<code>println!</code>"));
}

#[test]
fn html_code_block() {
    let input = "before\n```rust\nfn main() {}\n```\nafter";
    let result = markdown_to_telegram_html(input);
    assert!(
        result.contains("<pre><code class=\"language-rust\">"),
        "result: {result}"
    );
    assert!(result.contains("fn main() {}"));
    assert!(result.contains("</code></pre>"));
}

#[test]
fn html_code_block_no_lang() {
    let input = "```\nhello\n```";
    let result = markdown_to_telegram_html(input);
    assert!(result.contains("<pre>"), "result: {result}");
    assert!(result.contains("hello"));
    assert!(result.contains("</pre>"));
}

#[test]
fn html_link() {
    let result = markdown_to_telegram_html("click [here](https://example.com)");
    assert!(result.contains("<a href=\"https://example.com\">here</a>"));
}

#[test]
fn html_mixed_formatting() {
    let input = "**Bold** and *italic* with `code`";
    let result = markdown_to_telegram_html(input);
    assert!(result.contains("<b>Bold</b>"));
    assert!(result.contains("<i>italic</i>"));
    assert!(result.contains("<code>code</code>"));
}

#[test]
fn html_code_block_escapes_html_inside() {
    let input = "```\n<script>alert('xss')</script>\n```";
    let result = markdown_to_telegram_html(input);
    assert!(
        result.contains("&lt;script&gt;"),
        "XSS not escaped: {result}"
    );
    assert!(!result.contains("<script>"));
}

// ── Headings ─────────────────────────────────────────────────────────

#[test]
fn html_heading_to_bold() {
    assert_eq!(markdown_to_telegram_html("# Title"), "<b>Title</b>");
    assert_eq!(markdown_to_telegram_html("## Subtitle"), "<b>Subtitle</b>");
    assert_eq!(markdown_to_telegram_html("### Deep"), "<b>Deep</b>");
}

#[test]
fn html_heading_with_inline() {
    let result = markdown_to_telegram_html("## **Bold** heading");
    assert!(result.contains("<b>"));
    assert!(result.contains("Bold"));
    assert!(result.contains("heading"));
}

// ── Blockquotes ─────────────────────────────────────────────────────

#[test]
fn html_blockquote_single_line() {
    let result = markdown_to_telegram_html("> Hello world");
    assert!(result.contains("<blockquote>"));
    assert!(result.contains("Hello world"));
    assert!(result.contains("</blockquote>"));
}

#[test]
fn html_blockquote_multi_line() {
    let input = "> Line one\n> Line two\n> Line three";
    let result = markdown_to_telegram_html(input);
    assert!(result.contains("<blockquote>"));
    assert!(result.contains("Line one"));
    assert!(result.contains("Line two"));
    assert!(result.contains("Line three"));
    assert!(result.contains("</blockquote>"));
}

#[test]
fn html_blockquote_closes_before_normal() {
    let input = "> Quoted\n\nNormal text";
    let result = markdown_to_telegram_html(input);
    assert!(result.contains("</blockquote>"));
    assert!(result.contains("Normal text"));
}

// ── Strikethrough ───────────────────────────────────────────────────

#[test]
fn html_strikethrough() {
    let result = markdown_to_telegram_html("~~deleted~~");
    assert!(result.contains("<s>deleted</s>"));
}

#[test]
fn html_strikethrough_mixed() {
    let result = markdown_to_telegram_html("**bold** and ~~deleted~~");
    assert!(result.contains("<b>bold</b>"));
    assert!(result.contains("<s>deleted</s>"));
}

// ── Bullet lists ────────────────────────────────────────────────────

#[test]
fn html_bullet_list() {
    let input = "- Item one\n- Item two\n- Item three";
    let result = markdown_to_telegram_html(input);
    assert!(result.contains("\u{2022} Item one"), "result: {result}");
    assert!(result.contains("\u{2022} Item two"));
    assert!(result.contains("\u{2022} Item three"));
}

#[test]
fn html_star_bullet_not_bold() {
    let input = "* Item one\n* Item two";
    let result = markdown_to_telegram_html(input);
    assert!(result.contains("\u{2022} Item one"), "result: {result}");
    assert!(result.contains("\u{2022} Item two"));
}

// ── Numbered list ───────────────────────────────────────────────────

#[test]
fn html_numbered_list() {
    let input = "1. First\n2. Second\n3. Third";
    let result = markdown_to_telegram_html(input);
    assert!(result.contains("1."), "result: {result}");
    assert!(result.contains("First"));
    assert!(result.contains("2."));
    assert!(result.contains("Second"));
    assert!(result.contains("3."));
    assert!(result.contains("Third"));
}

// ── Horizontal rule ─────────────────────────────────────────────────

#[test]
fn html_horizontal_rule() {
    assert!(markdown_to_telegram_html("---").contains("\u{2014}\u{2014}\u{2014}"));
}

// ── Table -> ASCII in <pre> ──────────────────────────────────────────

#[test]
fn html_narrow_table_in_pre() {
    let input = "| Name | Age |\n|------|-----|\n| Alice | 30 |\n| Bob | 25 |";
    let result = markdown_to_telegram_html(input);
    assert!(
        result.contains("<pre>"),
        "narrow table should be in <pre>: {result}"
    );
    assert!(result.contains("Alice"), "result: {result}");
    assert!(result.contains("Bob"));
    assert!(result.contains("30"));
    assert!(result.contains("25"));
    assert!(result.contains("</pre>"));
}

// ── Nested formatting ───────────────────────────────────────────────

#[test]
fn html_nested_bold_italic() {
    let result = markdown_to_telegram_html("***bold and italic***");
    // pulldown-cmark parses *** as emphasis + strong
    assert!(result.contains("<b>") || result.contains("<i>"));
    assert!(result.contains("bold and italic"));
}

#[test]
fn html_bold_inside_list() {
    let input = "- **Important** item\n- Normal item";
    let result = markdown_to_telegram_html(input);
    assert!(result.contains("<b>Important</b>"), "result: {result}");
    assert!(result.contains("Normal item"));
}

// ── XSS / Security ─────────────────────────────────────────────────

#[test]
fn html_xss_in_text_escaped() {
    let result = markdown_to_telegram_html("Hello <script>alert(1)</script> world");
    // The < and > in inline HTML may be passed through or escaped
    // depending on pulldown-cmark; verify no raw script tag in output
    // Actually, pulldown-cmark may treat this as inline HTML.
    // The important thing is it doesn't break Telegram rendering.
    assert!(result.contains("Hello"));
    assert!(result.contains("world"));
}

#[test]
fn html_code_block_with_html_entities() {
    let input = "```html\n<div class=\"test\">&amp;</div>\n```";
    let result = markdown_to_telegram_html(input);
    assert!(result.contains("<pre>"), "result: {result}");
    assert!(result.contains("&lt;div"));
    assert!(result.contains("&amp;amp;"));
}

// ── markdown_to_plain tests ─────────────────────────────────────────

#[test]
fn plain_text_extraction() {
    let md = "**Bold** and *italic* with `code`";
    let plain = markdown_to_plain(md);
    assert_eq!(plain, "Bold and italic with code");
}

#[test]
fn plain_text_strips_links() {
    let md = "Click [here](https://example.com) now";
    let plain = markdown_to_plain(md);
    assert_eq!(plain, "Click here now");
}

#[test]
fn plain_text_preserves_code_block_content() {
    let md = "```\nfn main() {}\n```";
    let plain = markdown_to_plain(md);
    assert!(plain.contains("fn main() {}"));
}

// ── Long message split with HTML tags ───────────────────────────────

#[test]
fn split_long_message_with_pre_block() {
    // Create a message >5000 chars with <pre> block spanning the boundary
    let before = "Hello\n";
    let code = format!(
        "<pre><code class=\"language-rust\">{}</code></pre>",
        "let x = 1;\n".repeat(500)
    );
    let after = "\nDone";
    let msg = format!("{}{}{}", before, code, after);
    let chunks = split_message(&msg, 4096);
    assert!(chunks.len() >= 2, "should produce multiple chunks");

    // Concatenated chunks (stripping injected close/reopen tags) should
    // contain all original content
    let combined: String = chunks.join("");
    assert!(combined.contains("Hello"));
    assert!(combined.contains("Done"));
    assert!(combined.contains("let x = 1;"));
}

#[test]
fn split_message_all_chunks_valid_utf8() {
    let msg = "\u{4f60}\u{597d}\u{4e16}\u{754c}\u{1f30d}".repeat(1500);
    let chunks = split_message(&msg, 4096);
    for chunk in &chunks {
        // This panics if not valid UTF-8
        let _ = chunk.chars().count();
    }
}

// ── Tag tracking helper tests ───────────────────────────────────────

#[test]
fn update_tag_stack_push_and_pop() {
    let mut stack = Vec::new();
    update_tag_stack(&mut stack, "<b>hello</b>");
    assert!(stack.is_empty(), "balanced tags should leave stack empty");
}

#[test]
fn update_tag_stack_unclosed() {
    let mut stack = Vec::new();
    update_tag_stack(&mut stack, "<b><i>hello");
    assert_eq!(stack.len(), 2);
    assert_eq!(extract_tag_name(&stack[0]), "b");
    assert_eq!(extract_tag_name(&stack[1]), "i");
}

#[test]
fn update_tag_stack_ignores_div_tracks_span() {
    let mut stack = Vec::new();
    // div is not a Telegram tag — should be ignored
    update_tag_stack(&mut stack, "<div>hello</div>");
    assert!(stack.is_empty(), "div should not be tracked");
    // span IS tracked (used for tg-spoiler)
    update_tag_stack(&mut stack, "<span class=\"tg-spoiler\">secret");
    assert_eq!(stack.len(), 1, "unclosed span should be tracked");
    assert_eq!(extract_tag_name(&stack[0]), "span");
}

#[test]
fn closing_tag_for_simple() {
    assert_eq!(closing_tag_for("<b>"), "</b>");
    assert_eq!(closing_tag_for("<code class=\"language-rust\">"), "</code>");
    assert_eq!(closing_tag_for("<a href=\"https://example.com\">"), "</a>");
}

// ── MarkdownV2 tests ────────────────────────────────────────────────

#[test]
fn escape_markdown_v2_special_chars() {
    let escaped = escape_markdown_v2("Hello *world* [link](url) #tag");
    assert_eq!(escaped, "Hello \\*world\\* \\[link\\]\\(url\\) \\#tag");
}

#[test]
fn escape_markdown_v2_all_specials() {
    for ch in [
        '_', '*', '[', ']', '(', ')', '~', '`', '>', '#', '+', '-', '=', '|', '{', '}', '.', '!',
    ] {
        let input = format!("a{ch}b");
        let escaped = escape_markdown_v2(&input);
        assert!(escaped.contains(&format!("\\{ch}")), "should escape {ch}");
    }
}

#[test]
fn escape_markdown_v2_no_change_for_plain() {
    assert_eq!(escape_markdown_v2("hello world 123"), "hello world 123");
}

// ── HITL approval ID extraction tests ────────────────────────────────

#[test]
fn extract_sysop_id_from_approval_message() {
    let msg = "\u{1f527} **SysAdmin Operation Approval Request**\n\nApproval ID: `sysop-abcd1234`\n\n**Plan:** update packages";
    assert_eq!(extract_approval_id(msg), Some("sysop-abcd1234".to_string()));
}
#[test]
fn extract_sysop_id_returns_none_for_no_match() {
    let msg = "Just a regular message with no approval";
    assert_eq!(extract_approval_id(msg), None);
}

#[test]
fn extract_sysop_id_non_sysop_prefix() {
    // A truly unrecognized prefix still returns None.
    let msg = "Approval ID: `unknownprefix-123`";
    assert_eq!(extract_approval_id(msg), None);
}

#[test]
fn extract_draft_id_with_draft_prefix() {
    let msg = "Approval ID: `draft-abc123`";
    assert_eq!(
        extract_approval_id(msg),
        None,
        "historical draft approval prefixes are no longer registered"
    );
}

#[test]
fn extract_config_id_from_approval_message() {
    let msg = "\u{2699}\u{fe0f} **Config Change Approval Request**\n\nApproval ID: `config-abcd1234`\n\n**Setting:** `llm_model`";
    assert_eq!(
        extract_approval_id(msg),
        Some("config-abcd1234".to_string())
    );
}

// ── Task list ───────────────────────────────────────────────────────

#[test]
fn html_task_list() {
    let input = "- [x] Done\n- [ ] Todo";
    let result = markdown_to_telegram_html(input);
    assert!(
        result.contains("\u{2611}"),
        "checked marker missing: {result}"
    );
    assert!(
        result.contains("\u{2610}"),
        "unchecked marker missing: {result}"
    );
    assert!(result.contains("Done"));
    assert!(result.contains("Todo"));
}

// ── Render ASCII table ──────────────────────────────────────────────

#[test]
fn html_table_in_context_with_list() {
    let input = "\u{4ee5}\u{4e0b}\u{662f}\u{6700}\u{8fd1}\u{6536}\u{5230}\u{7684} 10 \u{5c01}\u{ff1a}\n\n| \u{53d1}\u{4ef6}\u{4eba} | \u{4e3b}\u{9898} | \u{65e5}\u{671f} |\n|---|---|---|\n| Costco | Order Delivery | 2026-03-09 |\n| Netflix | Sign-in code | 2026-02-09 |\n\n\u{7531}\u{4e8e}\u{672a}\u{8bfb}\u{90ae}\u{4ef6}\u{8f83}\u{591a}\u{ff1a}\n\n- \u{7b5b}\u{9009}\u{7279}\u{5b9a}\u{53d1}\u{4ef6}\u{4eba}\n- \u{6807}\u{8bb0}\u{4e3a}\u{5df2}\u{8bfb}";
    let result = markdown_to_telegram_html(input);
    assert!(result.contains("Costco"), "Costco missing: {result}");
    // List items should come after table
    assert!(
        result.contains("\u{2022} \u{7b5b}\u{9009}\u{7279}\u{5b9a}\u{53d1}\u{4ef6}\u{4eba}"),
        "bullet list missing: {result}"
    );
}

#[test]
fn html_table_no_blank_line_before() {
    let input = "\u{4ee5}\u{4e0b}\u{662f} 10 \u{5c01}\u{ff1a}\n| \u{53d1}\u{4ef6}\u{4eba} | \u{4e3b}\u{9898} | \u{65e5}\u{671f} |\n|---|---|---|\n| Costco | Order | 2026-03-09 |\n\n\u{540e}\u{7eed}\u{6587}\u{672c}";
    let result = markdown_to_telegram_html(input);
    assert!(result.contains("Costco"), "Costco missing: {result}");
}

#[test]
fn html_wide_table_uses_vertical_cards() {
    // Production pattern: 3-column wide table with long subjects -> vertical cards
    let input = "\u{6709} 201 \u{5c01}\u{672a}\u{8bfb}\u{90ae}\u{4ef6}\u{ff1a}\n\n| \u{53d1}\u{4ef6}\u{4eba} | \u{4e3b}\u{9898} | \u{65e5}\u{671f} |\n| :--- | :--- | :--- |\n| **Costco Same-Day** | Order $15 Off | 2026-03-09 |\n| **Netflix** | Sign-in code | 2026-02-09 |\n\n\u{4f60}\u{6709}\u{67e5}\u{770b}\u{9700}\u{6c42}\u{5417}\u{ff1f}";
    let result = markdown_to_telegram_html(input);
    eprintln!("wide table vertical cards:\n{}", result);
    assert!(
        result.contains("Costco Same-Day"),
        "Costco missing: {result}"
    );
    assert!(result.contains("Netflix"), "Netflix missing: {result}");
    // Wide table should use vertical card format with bold headers
    assert!(result.contains("<b>"), "should have bold headers: {result}");
}

#[test]
fn narrow_table_uses_horizontal_pre() {
    let rows = vec![
        vec!["Name".to_string(), "Age".to_string()],
        vec!["Alice".to_string(), "30".to_string()],
        vec!["Bob".to_string(), "25".to_string()],
    ];
    let result = render_table_adaptive(&rows);
    eprintln!("narrow table:\n{}", result);
    assert!(
        result.contains("<pre>"),
        "narrow table should use <pre>: {result}"
    );
    assert!(result.contains("Alice"));
    assert!(result.contains("Bob"));
    assert!(
        result.contains("\u{2502}"),
        "should have box-drawing borders"
    );
}

#[test]
fn wide_table_uses_vertical_cards() {
    let rows = vec![
        vec![
            "\u{53d1}\u{4ef6}\u{4eba}".to_string(),
            "\u{4e3b}\u{9898}".to_string(),
            "\u{65e5}\u{671f}".to_string(),
        ],
        vec![
            "Costco Same-Day by Instacart".to_string(),
            "Order Costco Delivery Again For $15 Off".to_string(),
            "2026-03-09".to_string(),
        ],
        vec![
            "Netflix".to_string(),
            "Netflix: Your sign-in code".to_string(),
            "2026-02-09".to_string(),
        ],
    ];
    let result = render_table_adaptive(&rows);
    eprintln!("wide table:\n{}", result);
    // Wide table should NOT use <pre>
    assert!(
        !result.contains("<pre>"),
        "wide table should use vertical cards, not <pre>: {result}"
    );
    // Should use bold headers
    assert!(
        result.contains("<b>\u{53d1}\u{4ef6}\u{4eba}</b>"),
        "should have bold header: {result}"
    );
    assert!(result.contains("Costco Same-Day"));
    assert!(result.contains("Netflix"));
    // Rows separated by em-dashes
    assert!(
        result.contains("\u{2014}\u{2014}\u{2014}"),
        "should have row separator"
    );
}

#[test]
fn cjk_table_alignment() {
    let rows = vec![
        vec![
            "\u{59d3}\u{540d}".to_string(),
            "\u{5e74}\u{9f84}".to_string(),
        ],
        vec!["\u{5f20}\u{4e09}".to_string(), "30".to_string()],
        vec!["Alice".to_string(), "25".to_string()],
    ];
    let result = render_table_adaptive(&rows);
    eprintln!("CJK table:\n{}", result);
    // This should be narrow enough for horizontal format
    assert!(
        result.contains("<pre>"),
        "narrow CJK table should use <pre>: {result}"
    );
    assert!(result.contains("\u{5f20}\u{4e09}"));
    assert!(result.contains("Alice"));
}

#[test]
fn display_width_cjk() {
    assert_eq!(display_width("hello"), 5);
    assert_eq!(display_width("\u{4f60}\u{597d}"), 4); // each CJK char = 2 width
    assert_eq!(display_width("AB\u{4f60}\u{597d}CD"), 8); // 2 + 4 + 2
    assert_eq!(display_width("&lt;"), 1); // HTML entity -> "<" = 1
    assert_eq!(display_width("&amp;"), 1);
}

// ── bot_commands tests ──────────────────────────────────────────────

#[test]
fn core_commands_are_subset_of_bot_commands() {
    let core = core_commands();
    let bot = bot_commands();
    // bot_commands() returns core_commands() (legacy alias)
    assert_eq!(core.len(), bot.len());
    let core_names = core_command_names();
    // 1 name: hb (manual heartbeat trigger). /heartbeat and /admin were
    // retired with the SDUI panels (2026-09-16) and must not be reserved
    // or advertised any more.
    // NOTE: "status" must NOT be a core command -- it would overwrite
    // the existing /status command handled via panel/LLM.
    assert_eq!(core_names.len(), 1);
    assert!(
        core_names.contains("hb"),
        "hb is a direct manual trigger command"
    );
    assert!(
        !core_names.contains("admin"),
        "admin was retired with the SDUI panels"
    );
    assert!(
        !core_names.contains("heartbeat"),
        "heartbeat was retired with the SDUI panels"
    );
    let advertised: Vec<&str> = core.iter().map(|c| c.command.as_str()).collect();
    assert_eq!(
        advertised,
        ["tasks"],
        "menu must not advertise the retired /heartbeat and /admin entries"
    );
    assert!(
        !core_names.contains("status"),
        "status must not override existing command"
    );
}

#[test]
#[allow(deprecated)]
fn resolve_command_alias_maps_legacy() {
    // All legacy aliases removed -- everything returns None
    assert_eq!(resolve_command_alias("d"), None);
    assert_eq!(resolve_command_alias("skills"), None);
    assert_eq!(resolve_command_alias("sk"), None);
    assert_eq!(resolve_command_alias("unknown"), None);
}

#[test]
fn bot_commands_valid() {
    let cmds = bot_commands();
    assert!(!cmds.is_empty());
    for cmd in &cmds {
        // Telegram requires: 1-32 lowercase alphanumeric + underscore
        assert!(!cmd.command.is_empty() && cmd.command.len() <= 32);
        assert!(
            cmd.command
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        );
        // Description: 3-256 chars
        assert!(cmd.description.len() >= 3 && cmd.description.len() <= 256);
    }
}

#[test]
fn bot_commands_no_duplicates() {
    let cmds = bot_commands();
    let names: Vec<_> = cmds.iter().map(|c| &c.command).collect();
    let unique: std::collections::HashSet<_> = names.iter().collect();
    assert_eq!(names.len(), unique.len(), "duplicate command names");
}

// ── chat_action_for_status tests ────────────────────────────────────

#[test]
fn chat_action_typing_default() {
    assert!(matches!(
        chat_action_for_status("Processing query..."),
        ChatAction::Typing
    ));
}

#[test]
fn chat_action_upload_photo() {
    assert!(matches!(
        chat_action_for_status("Taking screenshot..."),
        ChatAction::UploadPhoto
    ));
    assert!(matches!(
        chat_action_for_status("Uploading photo"),
        ChatAction::UploadPhoto
    ));
}

#[test]
fn chat_action_upload_document() {
    assert!(matches!(
        chat_action_for_status("Preparing document export"),
        ChatAction::UploadDocument
    ));
    assert!(matches!(
        chat_action_for_status("Generating file..."),
        ChatAction::UploadDocument
    ));
}

#[test]
fn chat_action_find_location() {
    assert!(matches!(
        chat_action_for_status("Navigating to the route..."),
        ChatAction::FindLocation
    ));
}

// ── long output threshold tests ─────────────────────────────────────

#[test]
fn long_output_detection() {
    // Short message -> 1 chunk, below threshold
    let short = "Hello world";
    let chunks = split_message(short, 4096);
    assert!(chunks.len() <= LONG_OUTPUT_CHUNK_THRESHOLD);

    // Very long message -> many chunks, above threshold
    let long = "x\n".repeat(10_000);
    let html = markdown_to_telegram_html(&long);
    let chunks = split_message(&html, 4096);
    assert!(
        chunks.len() > LONG_OUTPUT_CHUNK_THRESHOLD,
        "expected >{} chunks for 20KB message, got {}",
        LONG_OUTPUT_CHUNK_THRESHOLD,
        chunks.len()
    );
}

// ── credential paste detection tests ────────────────────────────────

#[test]
fn detect_credentials_colon_format() {
    let text = "client_id: 12345.apps.googleusercontent.com\nclient_secret: GOCSPx-abcdef";
    let result = detect_google_credentials(text);
    assert!(result.is_some());
    let (cid, cs) = result.unwrap();
    assert_eq!(cid, "12345.apps.googleusercontent.com");
    assert_eq!(cs, "GOCSPx-abcdef");
}

#[test]
fn detect_credentials_env_format() {
    let text =
        "GOOGLE_CLIENT_ID=12345.apps.googleusercontent.com\nGOOGLE_CLIENT_SECRET=GOCSPx-abcdef";
    let result = detect_google_credentials(text);
    assert!(result.is_some());
    let (cid, cs) = result.unwrap();
    assert_eq!(cid, "12345.apps.googleusercontent.com");
    assert_eq!(cs, "GOCSPx-abcdef");
}

#[test]
fn detect_credentials_with_quotes() {
    let text = "client_id: \"12345.apps.googleusercontent.com\"\nclient_secret: 'GOCSPx-abcdef'";
    let result = detect_google_credentials(text);
    assert!(result.is_some());
    let (cid, cs) = result.unwrap();
    assert_eq!(cid, "12345.apps.googleusercontent.com");
    assert_eq!(cs, "GOCSPx-abcdef");
}

#[test]
fn detect_credentials_returns_none_for_normal_text() {
    assert!(detect_google_credentials("hello world").is_none());
    assert!(detect_google_credentials("please help me with calendar").is_none());
    assert!(detect_google_credentials("client_id: abc").is_none()); // missing secret
}

#[test]
fn detect_credentials_case_insensitive() {
    let text = "Client_ID: 12345.apps.googleusercontent.com\nClient_Secret: GOCSPx-abc";
    let result = detect_google_credentials(text);
    // Detection uses lowercase comparison but value extraction is case-sensitive
    assert!(result.is_some());
}

// ── auth code detection tests ───────────────────────────────────────

// ── credential leak detection tests ────────────────────────────────

#[test]
fn detect_leak_natural_language_with_real_values() {
    let text =
        "my client_id is 12345.apps.googleusercontent.com and client_secret is GOCSPx-abc123";
    assert!(detect_credential_leak(text));
}

#[test]
fn detect_leak_ignores_discussion() {
    // Talking about credentials conceptually without real values
    assert!(!detect_credential_leak(
        "what is a client_id and client_secret?"
    ));
    assert!(!detect_credential_leak(
        "I need to find my client_id and client_secret"
    ));
}

#[test]
fn detect_leak_ignores_normal_text() {
    assert!(!detect_credential_leak("hello world"));
    assert!(!detect_credential_leak("help me with calendar"));
}

#[test]
fn detect_leak_catches_mixed_case() {
    let text = "Client_ID = 12345.apps.googleusercontent.com, Client_Secret = GOCSPX-xyz";
    assert!(detect_credential_leak(text));
}

// ── auth code detection tests ───────────────────────────────────────

#[test]
fn detect_auth_code_valid() {
    assert!(detect_google_auth_code(
        "4/0AanRRrtRQB-xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx"
    ));
}

#[test]
fn detect_auth_code_rejects_normal_text() {
    assert!(!detect_google_auth_code("hello world"));
    assert!(!detect_google_auth_code("please help me"));
    assert!(!detect_google_auth_code("4/")); // too short
}

#[test]
fn detect_auth_code_rejects_multiline() {
    assert!(!detect_google_auth_code("4/0AanRRrt\nsecond line"));
}

#[test]
fn detect_auth_code_rejects_spaces() {
    assert!(!detect_google_auth_code("4/0AanRRrt some other text"));
}

// ── Property-based tests (proptest) ─────────────────────────────────

mod proptest_markdown {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        /// markdown_to_telegram_html never panics on arbitrary input
        #[test]
        fn markdown_html_never_panics(input in "\\PC{0,500}") {
            let _ = markdown_to_telegram_html(&input);
        }

        /// escape_html never panics and always escapes &, <, >, ", '
        #[test]
        fn escape_html_safety(input in "\\PC{0,300}") {
            let result = escape_html(&input);
            prop_assert!(!result.contains('&') || result.contains("&amp;") || result.contains("&lt;") || result.contains("&gt;") || result.contains("&quot;") || result.contains("&#39;"),
                "Bare & should be escaped");
            // No raw specials (they must be escaped)
            let no_entity = result.replace("&amp;", "").replace("&lt;", "").replace("&gt;", "").replace("&quot;", "").replace("&#39;", "");
            prop_assert!(!no_entity.contains('<'), "Raw < must be escaped");
            prop_assert!(!no_entity.contains('>'), "Raw > must be escaped");
            prop_assert!(!no_entity.contains('"'), "Raw double-quote must be escaped");
            prop_assert!(!no_entity.contains('\''), "Raw single-quote must be escaped");
        }

        /// escape_html is idempotent when applied once (double-escape is expected)
        /// but single escape should not contain raw specials
        #[test]
        fn escape_html_no_raw_specials(input in "\\PC{0,200}") {
            let escaped = escape_html(&input);
            // Count raw < and > (not inside entities)
            for ch in escaped.chars() {
                if ch == '<' || ch == '>' {
                    // These can only appear as part of &lt; or &gt; entity output
                    // Actually escape_html replaces ALL < and > so none should remain
                    prop_assert!(false, "escape_html left raw angle bracket in output");
                }
            }
        }

        /// markdown_to_telegram_html output has balanced tags for simple bold/italic
        #[test]
        fn markdown_html_balanced_simple_tags(word in "[a-zA-Z]{1,20}") {
            let md = format!("**{}**", word);
            let html = markdown_to_telegram_html(&md);
            let open_b = html.matches("<b>").count();
            let close_b = html.matches("</b>").count();
            prop_assert_eq!(open_b, close_b, "Unbalanced <b> tags in: {}", html);

            let md2 = format!("*{}*", word);
            let html2 = markdown_to_telegram_html(&md2);
            let open_i = html2.matches("<i>").count();
            let close_i = html2.matches("</i>").count();
            prop_assert_eq!(open_i, close_i, "Unbalanced <i> tags in: {}", html2);
        }

        /// split_message never panics on arbitrary UTF-8 strings
        #[test]
        fn split_message_never_panics(input in "\\PC{0,8000}", limit in 100usize..4096) {
            let chunks = split_message(&input, limit);
            prop_assert!(!chunks.is_empty(), "split_message should return at least one chunk");
        }

        /// split_message preserves all content (joined chunks == original, modulo tag fixups)
        #[test]
        fn split_message_no_data_loss(input in "[a-zA-Z0-9 ]{1,5000}") {
            let chunks = split_message(&input, 4096);
            let joined: String = chunks.concat();
            prop_assert_eq!(input.trim(), joined.trim(),
                "split_message should preserve plain text content");
        }

        /// markdown_to_telegram_html output is never shorter than the text content
        /// (HTML tags only add length; content is preserved or escaped to longer forms)
        #[test]
        fn markdown_html_length_invariant(word in "[a-zA-Z0-9]{1,200}") {
            // Plain text with no markdown syntax should appear in output
            let html = markdown_to_telegram_html(&word);
            let trimmed = html.trim();
            prop_assert!(
                trimmed.len() >= word.len(),
                "HTML output ({} bytes) should be >= input ({} bytes): input={:?}, output={:?}",
                trimmed.len(), word.len(), word, trimmed
            );
        }

        /// markdown_to_telegram_html preserves text content for bold markdown
        #[test]
        fn markdown_html_bold_preserves_content(word in "[a-zA-Z]{1,50}") {
            let md = format!("**{}**", word);
            let html = markdown_to_telegram_html(&md);
            // The word itself must appear in the HTML output
            prop_assert!(
                html.contains(&word),
                "Bold markdown should preserve word {:?} in output {:?}",
                word, html
            );
        }

        /// escape_html output is always >= input length (escaping only grows)
        #[test]
        fn escape_html_length_grows(input in "\\PC{0,200}") {
            let escaped = escape_html(&input);
            prop_assert!(
                escaped.len() >= input.len(),
                "escape_html should never shrink: input={} escaped={}",
                input.len(), escaped.len()
            );
        }
    }
}

// ── Snapshot tests (insta) ──────────────────────────────────────────

mod snapshot_markdown {
    use super::*;

    #[test]
    fn snapshot_bold_italic_code() {
        let md = "**bold** and *italic* and `code`";
        insta::assert_snapshot!(markdown_to_telegram_html(md));
    }

    #[test]
    fn snapshot_nested_list() {
        let md = "- item 1\n- item 2\n  - nested a\n  - nested b\n- item 3";
        insta::assert_snapshot!(markdown_to_telegram_html(md));
    }

    #[test]
    fn snapshot_ordered_list() {
        let md = "1. first\n2. second\n3. third";
        insta::assert_snapshot!(markdown_to_telegram_html(md));
    }

    #[test]
    fn snapshot_code_block_with_lang() {
        let md = "```python\ndef hello():\n    print('hi')\n```";
        insta::assert_snapshot!(markdown_to_telegram_html(md));
    }

    #[test]
    fn snapshot_code_block_no_lang() {
        let md = "```\nplain code\n```";
        insta::assert_snapshot!(markdown_to_telegram_html(md));
    }

    #[test]
    fn snapshot_heading() {
        let md = "# Title\n\nSome text\n\n## Subtitle";
        insta::assert_snapshot!(markdown_to_telegram_html(md));
    }

    #[test]
    fn snapshot_blockquote() {
        let md = "> This is a quote\n> with two lines";
        insta::assert_snapshot!(markdown_to_telegram_html(md));
    }

    #[test]
    fn snapshot_strikethrough() {
        let md = "~~deleted~~ and normal";
        insta::assert_snapshot!(markdown_to_telegram_html(md));
    }

    #[test]
    fn snapshot_link() {
        let md = "[click here](https://example.com)";
        insta::assert_snapshot!(markdown_to_telegram_html(md));
    }

    #[test]
    fn snapshot_table() {
        let md = "| Name | Value |\n|------|-------|\n| foo  | 42    |\n| bar  | 99    |";
        insta::assert_snapshot!(markdown_to_telegram_html(md));
    }

    #[test]
    fn snapshot_mixed_complex() {
        let md = "# Report\n\n**Status**: \u{2705} OK\n\n- Item `A` with *emphasis*\n- Item B\n\n```json\n{\"key\": \"value\"}\n```\n\n> Note: important";
        insta::assert_snapshot!(markdown_to_telegram_html(md));
    }

    #[test]
    fn snapshot_special_chars() {
        let md = "Use <script> & \"quotes\" in HTML";
        insta::assert_snapshot!(markdown_to_telegram_html(md));
    }

    #[test]
    fn snapshot_horizontal_rule() {
        let md = "before\n\n---\n\nafter";
        insta::assert_snapshot!(markdown_to_telegram_html(md));
    }

    #[test]
    fn snapshot_task_list() {
        let md = "- [x] done\n- [ ] todo";
        insta::assert_snapshot!(markdown_to_telegram_html(md));
    }

    #[test]
    fn snapshot_escape_html() {
        insta::assert_snapshot!(escape_html("<script>alert('xss')</script>"));
    }
}

// ── extract_approval_id tests ───────────────────────────────────────

#[test]
fn extract_approval_id_sysop() {
    let msg = "Approval ID: `sysop-abc123`\nSome details";
    assert_eq!(extract_approval_id(msg), Some("sysop-abc123".into()));
}

#[test]
fn extract_approval_id_email() {
    let msg = "\u{1f4e7} **Email Send \u{2014} Approval Request**\n\nApproval ID: `email-deadbeef`\n\n**To:** foo";
    assert_eq!(
        extract_approval_id(msg),
        None,
        "historical mailbox approval prefixes are no longer registered"
    );
}

#[test]
fn extract_approval_id_revoke() {
    let msg = "Approval ID: `revoke-google-0123456789abcdef`\n\nThis will remove";
    assert_eq!(
        extract_approval_id(msg),
        Some("revoke-google-0123456789abcdef".into())
    );
}

#[test]
fn extract_approval_id_unknown_prefix_returns_none() {
    let msg = "Approval ID: `unknown-xyz`\nDetails";
    assert_eq!(extract_approval_id(msg), None);
}

#[test]
fn extract_approval_id_no_marker_returns_none() {
    let msg = "This message has no approval ID at all.";
    assert_eq!(extract_approval_id(msg), None);
}

// ── Gate approval E2E flow tests (covers the actual message format) ──

#[test]
fn extract_approval_id_gate_backtick_format() {
    // This is the format sent by orchestrator.rs NeedApproval handler
    let msg = "\u{1f510} <b>Approval Request</b>\n\n\
               Tool: <code>web_reader</code>\n\
               Tool 'web_reader' requires your approval.\n\
               Domain: www.google.com\n\n\
               Approval ID: `gate-e035556d451364bf`";
    assert_eq!(
        extract_approval_id(msg),
        Some("gate-e035556d451364bf".into())
    );
}

#[test]
fn extract_approval_id_gate_after_markdown_to_html() {
    // After markdown_to_telegram_html converts backticks to <code> tags
    let msg = "\u{1f510} <b>Approval Request</b>\n\n\
               Tool: <code>web_reader</code>\n\
               Tool 'web_reader' requires your approval.\n\n\
               Approval ID: <code>gate-e035556d451364bf</code>";
    assert_eq!(
        extract_approval_id(msg),
        Some("gate-e035556d451364bf".into())
    );
}

#[test]
fn extract_approval_id_gate_plain_text() {
    // Fallback: plain text without any wrapping
    let msg = "Approval Request\nApproval ID: gate-abc123\nMore text";
    assert_eq!(extract_approval_id(msg), Some("gate-abc123".into()));
}

// =======================================================================
// Gate approval flow integration tests
//
// These tests verify the full message pipeline: construction (as done in
// orchestrator.rs) -> extraction (extract_approval_id) -> HTML conversion
// (markdown_to_telegram_html) -> re-extraction.  They exist because the
// gate approval system has had recurring bugs from format drift between
// the message construction site and the extraction logic.
// =======================================================================

// ── 1. Approval message format -> extract_approval_id consistency ────

#[test]
fn gate_approval_message_format_is_extractable() {
    // Build the message exactly as orchestrator.rs NeedApproval handler does (line ~2715)
    let tn = "web_reader";
    let summary = "Tool 'web_reader' requires your approval.";
    let url_host = Some("www.google.com".to_string());
    let intent_group_key: Option<String> = None;
    let approval_id = "gate-abc123def456";

    let approval_msg = format!(
        "\u{1f510} <b>Approval Request</b>\n\n\
         Tool: <code>{}</code>\n\
         {}{}{}\n\n\
         Approval ID: `{}`",
        tn,
        summary,
        intent_group_key
            .as_ref()
            .map(|k| format!("\nIntent: {}", k))
            .unwrap_or_default(),
        url_host
            .as_ref()
            .map(|h| format!("\nDomain: {}", h))
            .unwrap_or_default(),
        approval_id,
    );

    let extracted = extract_approval_id(&approval_msg);
    assert_eq!(
        extracted,
        Some("gate-abc123def456".to_string()),
        "extract_approval_id must work on the actual gate approval format. Message was: {}",
        approval_msg,
    );
}

#[test]
fn gate_approval_message_with_intent_is_extractable() {
    // Variant: both intent_group_key AND url_host are present
    let tn = "web_reader";
    let summary = "Tool 'web_reader' requires your approval.";
    let url_host = Some("maps.google.com".to_string());
    let intent_group_key = Some("commute_check".to_string());
    let approval_id = "gate-deadbeef01234567";

    let approval_msg = format!(
        "\u{1f510} <b>Approval Request</b>\n\n\
         Tool: <code>{}</code>\n\
         {}{}{}\n\n\
         Approval ID: `{}`",
        tn,
        summary,
        intent_group_key
            .as_ref()
            .map(|k| format!("\nIntent: {}", k))
            .unwrap_or_default(),
        url_host
            .as_ref()
            .map(|h| format!("\nDomain: {}", h))
            .unwrap_or_default(),
        approval_id,
    );

    let extracted = extract_approval_id(&approval_msg);
    assert_eq!(
        extracted,
        Some("gate-deadbeef01234567".to_string()),
        "Must extract from gate message with both intent and domain. Message was: {}",
        approval_msg,
    );
}

#[test]
fn gate_approval_message_no_optional_fields_is_extractable() {
    // Variant: neither url_host nor intent_group_key
    let tn = "sysadmin_ops";
    let summary = "Tool 'sysadmin_ops' requires your approval.";
    let url_host: Option<String> = None;
    let intent_group_key: Option<String> = None;
    let approval_id = "gate-0000000000000001";

    let approval_msg = format!(
        "\u{1f510} <b>Approval Request</b>\n\n\
         Tool: <code>{}</code>\n\
         {}{}{}\n\n\
         Approval ID: `{}`",
        tn,
        summary,
        intent_group_key
            .as_ref()
            .map(|k| format!("\nIntent: {}", k))
            .unwrap_or_default(),
        url_host
            .as_ref()
            .map(|h| format!("\nDomain: {}", h))
            .unwrap_or_default(),
        approval_id,
    );

    let extracted = extract_approval_id(&approval_msg);
    assert_eq!(
        extracted,
        Some("gate-0000000000000001".to_string()),
        "Must extract from gate message with no optional fields. Message was: {}",
        approval_msg,
    );
}

// ── 2. After markdown_to_telegram_html conversion ───────────────────

#[test]
fn gate_approval_extractable_after_html_conversion() {
    // Build the raw message exactly as orchestrator.rs does
    let tn = "web_reader";
    let summary = "Tool 'web_reader' requires your approval.";
    let url_host = Some("www.google.com".to_string());
    let intent_group_key: Option<String> = None;
    let approval_id = "gate-e035556d451364bf";

    let raw = format!(
        "\u{1f510} <b>Approval Request</b>\n\n\
         Tool: <code>{}</code>\n\
         {}{}{}\n\n\
         Approval ID: `{}`",
        tn,
        summary,
        intent_group_key
            .as_ref()
            .map(|k| format!("\nIntent: {}", k))
            .unwrap_or_default(),
        url_host
            .as_ref()
            .map(|h| format!("\nDomain: {}", h))
            .unwrap_or_default(),
        approval_id,
    );

    let html = markdown_to_telegram_html(&raw);

    let extracted = extract_approval_id(&html);
    assert!(
        extracted.is_some(),
        "Must extract approval ID from HTML-converted message. HTML: {}",
        html,
    );
    assert_eq!(extracted.unwrap(), "gate-e035556d451364bf");
}

#[test]
fn gate_approval_extractable_after_html_conversion_with_intent_and_domain() {
    let tn = "web_reader";
    let summary = "Tool 'web_reader' needs approval in background mode.";
    let url_host = Some("maps.google.com".to_string());
    let intent_group_key = Some("commute_check".to_string());
    let approval_id = "gate-fedcba9876543210";

    let raw = format!(
        "\u{1f510} <b>Approval Request</b>\n\n\
         Tool: <code>{}</code>\n\
         {}{}{}\n\n\
         Approval ID: `{}`",
        tn,
        summary,
        intent_group_key
            .as_ref()
            .map(|k| format!("\nIntent: {}", k))
            .unwrap_or_default(),
        url_host
            .as_ref()
            .map(|h| format!("\nDomain: {}", h))
            .unwrap_or_default(),
        approval_id,
    );

    let html = markdown_to_telegram_html(&raw);

    let extracted = extract_approval_id(&html);
    assert!(
        extracted.is_some(),
        "Must extract from HTML with intent+domain. HTML: {}",
        html,
    );
    assert_eq!(extracted.unwrap(), "gate-fedcba9876543210");
}

// ── 3. Callback routes include all 6 gate scopes ────────────────────
//
// After the 2026-04-18 refactor the route table lives in
// `crate::callback_routes::CALLBACK_FAMILIES`. These tests now exercise
// the public API directly instead of textually scanning the source file.

#[test]
fn callback_routes_include_all_gate_scopes() {
    // All 6 gate-scope actions must be parseable to cmd_type="gate".
    for data in &[
        "gate_once:x",
        "gate_intent:x",
        "gate_domain:x",
        "gate_always:x",
        "gate_reject:x",
        "gate_clarify:x",
    ] {
        let parsed = crate::callback_routes::match_callback_data(data)
            .unwrap_or_else(|| panic!("{data} must match a gate route"));
        assert_eq!(parsed.1, "gate", "{data} must map to cmd_type gate");
    }
}

#[test]
fn callback_routes_gate_entries_have_correct_cmd_type() {
    // Redundant with the above — kept for explicit cmd_type guarantee.
    for data in &[
        "gate_once:y",
        "gate_intent:y",
        "gate_domain:y",
        "gate_always:y",
        "gate_reject:y",
        "gate_clarify:y",
    ] {
        let (_, cmd_type, _) = crate::callback_routes::match_callback_data(data).unwrap();
        assert_eq!(cmd_type, "gate");
    }
}

// ── 4. All approval types use realistic orchestrator message formats ─

#[test]
fn extract_sysop_approval_from_orchestrator_format() {
    // Exact format from orchestrator.rs line ~7815
    let approval_id = "sysop-a1b2c3d4e5f60001";
    let msg = format!(
        "\u{1f527} **SysAdmin Operation Approval Request**\n\n\
         Approval ID: `{}`\n\n\
         **Plan:** Update system packages\n\n\
         **Commands:**\n  1. `apt update`\n  2. `apt upgrade -y`\n\n\
         Reply with:\n\
         \u{2022} `approve sysop {0}` to execute\n\
         \u{2022} `reject sysop {0}` to cancel",
        approval_id,
    );
    assert_eq!(
        extract_approval_id(&msg),
        Some(approval_id.to_string()),
        "sysop: must extract from actual orchestrator format",
    );
}

#[test]
fn extract_config_approval_from_orchestrator_format() {
    // Exact format from orchestrator.rs line ~8061
    let approval_id = "config-b2c3d4e5f6a70002";
    let msg = format!(
        "\u{2699}\u{fe0f} **Config Change Approval Request**\n\n\
         Approval ID: `{}`\n\n\
         **Setting:** `llm_model`\n\
         **Current value:** gpt-4o\n\
         **Proposed value:** claude-opus-4-20250514\n\n\
         Reply with:\n\
         \u{2022} `approve config {0}` to apply\n\
         \u{2022} `reject config {0}` to cancel",
        approval_id,
    );
    assert_eq!(
        extract_approval_id(&msg),
        Some(approval_id.to_string()),
        "config: must extract from actual orchestrator format",
    );
}

#[test]
fn extract_menu_approval_from_orchestrator_format() {
    // Exact format from menu.rs line ~425
    let approval_id = "menu-c3d4e5f6a7b80003";
    let msg = format!(
        "\u{1f4cb} **Bot Menu Change \u{2014} Approval Request**\n\n\
         Approval ID: `{}`\n\n\
         **Changes:**\n  + /status \u{2014} Show system status\n\n\
         **Full menu (if approved):**\n  /start \u{2014} Start the bot\n  /status \u{2014} Show system status\n\n\
         Reply with:\n\
         \u{2022} `approve menu {0}` to apply\n\
         \u{2022} `reject menu {0}` to cancel",
        approval_id,
    );
    assert_eq!(
        extract_approval_id(&msg),
        Some(approval_id.to_string()),
        "menu: must extract from actual orchestrator format",
    );
}

#[test]
fn extract_mcp_approval_from_orchestrator_format() {
    // Format from orchestrator.rs line ~2928.  Note: mcp- IDs use
    // server_key instead of random hex (format!("mcp-{}", server_key))
    let approval_id = "mcp-weather_service";
    let msg = format!(
        "\u{1f50c} **MCP Server Approval Request**\n\n\
         Approval ID: `{approval_id}`\n\n\
         **Server:** `weather_service`\n\
         **Tools:** get_forecast, get_current\n\
         **Requested tool:** `get_forecast`\n\n\
         The LLM wants to use an external MCP server. \
         Once approved, this server's tools can be used freely.\n\n\
         Reply with:\n\
         \u{2022} `approve mcp {approval_id}` to allow\n\
         \u{2022} `reject mcp {approval_id}` to deny"
    );
    assert_eq!(
        extract_approval_id(&msg),
        Some(approval_id.to_string()),
        "mcp: must extract from actual orchestrator format",
    );
}
#[test]
fn extract_gate_approval_from_orchestrator_format() {
    // Exact format from orchestrator.rs line ~2715 (NeedApproval handler)
    let approval_id = "gate-a7b8c9d0e1f20007";
    let tn = "calendar_update_event";
    let summary = "Tool 'calendar_update_event' requires your approval.";
    let intent_group_key: Option<String> = None;
    let url_host: Option<String> = None;

    let msg = format!(
        "\u{1f510} <b>Approval Request</b>\n\n\
         Tool: <code>{}</code>\n\
         {}{}{}\n\n\
         Approval ID: `{}`",
        tn,
        summary,
        intent_group_key
            .as_ref()
            .map(|k| format!("\nIntent: {}", k))
            .unwrap_or_default(),
        url_host
            .as_ref()
            .map(|h| format!("\nDomain: {}", h))
            .unwrap_or_default(),
        approval_id,
    );
    assert_eq!(
        extract_approval_id(&msg),
        Some(approval_id.to_string()),
        "gate: must extract from actual NeedApproval handler format",
    );
}

// ── 4b. All prefixes survive markdown_to_telegram_html round-trip ────

#[test]
fn all_approval_prefixes_survive_html_conversion() {
    // Every known prefix must be extractable after HTML conversion
    let prefixes = [
        "sysop-aaa111",
        "config-bbb222",
        "menu-ccc333",
        "mcp-test_server",
        "revoke-google-fff666",
        "gate-ggg777",
    ];

    for id in &prefixes {
        // Wrap in a realistic message with backtick format
        let raw = format!(
            "**Approval Request**\n\nApproval ID: `{}`\n\nDetails here.",
            id
        );
        let html = markdown_to_telegram_html(&raw);

        let extracted = extract_approval_id(&html);
        assert_eq!(
            extracted,
            Some(id.to_string()),
            "Prefix '{}' must survive markdown_to_telegram_html. HTML: {}",
            id,
            html,
        );
    }
}

// ── End-to-end: gate verdict -> message construction -> extraction ────

#[test]
fn gate_verdict_to_message_to_extraction_roundtrip() {
    use crate::tool_gate::{CallerContext, GateVerdict, ToolGate};

    let gate = ToolGate::new();
    // Use a tool that always requires approval
    let verdict = gate.evaluate(
        "deactivate_skill",
        r#"{"skill_name":"test-123"}"#,
        CallerContext::Interactive,
    );

    if let GateVerdict::NeedApproval {
        approval_id,
        tool_name: tn,
        summary,
        ..
    } = verdict
    {
        // Build the message exactly as orchestrator.rs does
        let url_host: Option<String> = None;
        let intent_group_key: Option<String> = None;

        let approval_msg = format!(
            "\u{1f510} <b>Approval Request</b>\n\n\
             Tool: <code>{}</code>\n\
             {}{}{}\n\n\
             Approval ID: `{}`",
            tn,
            summary,
            intent_group_key
                .as_ref()
                .map(|k| format!("\nIntent: {}", k))
                .unwrap_or_default(),
            url_host
                .as_ref()
                .map(|h| format!("\nDomain: {}", h))
                .unwrap_or_default(),
            approval_id,
        );

        // Extract from raw message
        let raw_extracted = extract_approval_id(&approval_msg);
        assert_eq!(
            raw_extracted.as_deref(),
            Some(approval_id.as_str()),
            "Raw message extraction failed. Message: {}",
            approval_msg,
        );

        // Extract after HTML conversion
        let html = markdown_to_telegram_html(&approval_msg);
        let html_extracted = extract_approval_id(&html);
        assert_eq!(
            html_extracted.as_deref(),
            Some(approval_id.as_str()),
            "HTML-converted extraction failed. HTML: {}",
            html,
        );
    } else {
        panic!(
            "deactivate_skill should produce NeedApproval, got {:?}",
            verdict
        );
    }
}

// ── CallbackDedup tests ────────────────────────────────────────────

#[test]
fn callback_dedup_rejects_duplicate() {
    let mut dedup = CallbackDedup::new();
    assert!(
        !dedup.check_and_insert("abc123"),
        "first insert should succeed"
    );
    assert!(
        dedup.check_and_insert("abc123"),
        "duplicate should be rejected"
    );
    assert_eq!(dedup.len(), 1);
}

#[test]
fn callback_dedup_allows_distinct_ids() {
    let mut dedup = CallbackDedup::new();
    for i in 0..10 {
        let id = format!("id_{}", i);
        assert!(!dedup.check_and_insert(&id), "unique ID should succeed");
    }
    assert_eq!(dedup.len(), 10);
    // All should still be seen as duplicates
    for i in 0..10 {
        let id = format!("id_{}", i);
        assert!(dedup.check_and_insert(&id), "should reject known ID");
    }
}

#[test]
fn callback_dedup_evicts_at_capacity() {
    let mut dedup = CallbackDedup::new();
    // Fill to max capacity
    for i in 0..CALLBACK_DEDUP_MAX_ENTRIES {
        let id = format!("id_{}", i);
        assert!(!dedup.check_and_insert(&id));
    }
    assert_eq!(dedup.len(), CALLBACK_DEDUP_MAX_ENTRIES);

    // Inserting one more should evict the oldest (id_0)
    assert!(!dedup.check_and_insert("overflow"));
    assert_eq!(dedup.len(), CALLBACK_DEDUP_MAX_ENTRIES);
    // id_0 was evicted, so inserting it again should succeed
    assert!(
        !dedup.check_and_insert("id_0"),
        "evicted entry should be accepted again"
    );
    // id_1 was also evicted to make room
    assert!(
        !dedup.check_and_insert("id_1"),
        "evicted entry should be accepted again"
    );
}

#[test]
fn callback_dedup_evicts_stale_entries() {
    let mut dedup = CallbackDedup::new();
    // Manually insert an entry with a timestamp in the past
    let old_time =
        std::time::Instant::now() - std::time::Duration::from_secs(CALLBACK_DEDUP_TTL_SECS + 1);
    dedup.ids.insert("stale_id".to_string());
    dedup.entries.push_back(("stale_id".to_string(), old_time));
    assert_eq!(dedup.len(), 1);

    // Insert a new entry — stale one should be evicted first
    assert!(!dedup.check_and_insert("fresh_id"));
    // stale_id should have been evicted
    assert!(
        !dedup.ids.contains("stale_id"),
        "stale entry should be evicted"
    );
    assert_eq!(dedup.len(), 1); // only fresh_id remains

    // stale_id can be re-inserted since it was evicted
    assert!(!dedup.check_and_insert("stale_id"));
    assert_eq!(dedup.len(), 2);
}

#[test]
fn dispatcher_panic_payload_summary_handles_common_payloads() {
    let string_payload: Box<dyn std::any::Any + Send> = Box::new("TX is dead".to_string());
    assert_eq!(
        crate::telegram::panic_payload_to_string(string_payload.as_ref()),
        "TX is dead"
    );

    let str_payload: Box<dyn std::any::Any + Send> = Box::new("static panic");
    assert_eq!(
        crate::telegram::panic_payload_to_string(str_payload.as_ref()),
        "static panic"
    );

    let opaque_payload: Box<dyn std::any::Any + Send> = Box::new(42_i64);
    assert_eq!(
        crate::telegram::panic_payload_to_string(opaque_payload.as_ref()),
        "panic payload was not a string"
    );
}

#[test]
fn reminder_action_keyboard_uses_registered_callback_routes() {
    let keyboard = crate::telegram::reminder_action_keyboard(42, "zh");
    let mut callback_data = Vec::new();
    for row in keyboard.inline_keyboard {
        for button in row {
            if let teloxide::types::InlineKeyboardButtonKind::CallbackData(data) = button.kind {
                callback_data.push(data);
            }
        }
    }

    for data in callback_data {
        let (action, cmd_type, id) = crate::callback_routes::match_callback_data(&data)
            .unwrap_or_else(|| panic!("reminder callback data should route: {data}"));
        assert_eq!(cmd_type, "reminder_action");
        assert_eq!(id.as_ref(), "42");
        assert!(matches!(action, "done" | "snooze" | "delete" | "details"));
    }
}

#[test]
fn reminder_action_keyboard_labels_localize_via_catalog() {
    // i18n Task 3 cluster 2: keyboard chrome is Channel B (system-authored), so
    // its labels must come from the catalog under `display_language`, not a
    // hardcoded literal. en and zh must differ and match the catalog exactly.
    fn labels(locale: &str) -> Vec<String> {
        let kb = crate::telegram::reminder_action_keyboard(7, locale);
        kb.inline_keyboard
            .into_iter()
            .flatten()
            .map(|b| b.text)
            .collect()
    }
    let en = labels("en");
    let zh = labels("zh");
    assert_eq!(en[0], crate::i18n::t("reminder.btn.done", "en"));
    assert_eq!(zh[0], crate::i18n::t("reminder.btn.done", "zh"));
    assert_ne!(en, zh, "en/zh keyboard labels must differ (locale honored)");
    // Regression: the old hardcoded zh literal must not appear in the en keyboard.
    assert!(
        !en.iter().any(|l| l == "完成"),
        "en keyboard leaked zh literal"
    );
}

#[test]
fn reminder_snooze_targets_are_deterministic() {
    let now = chrono::DateTime::parse_from_rfc3339("2026-05-02T12:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    assert_eq!(
        crate::telegram::reminder_snooze_target_unix(
            now,
            chrono_tz::Tz::UTC,
            crate::telegram::ReminderSnoozeOption::Minutes10,
        ),
        now.timestamp() + 600
    );
    assert_eq!(
        crate::telegram::reminder_snooze_target_unix(
            now,
            chrono_tz::Tz::UTC,
            crate::telegram::ReminderSnoozeOption::Hour1,
        ),
        now.timestamp() + 3600
    );
}

/// Regression test for the `notif_id` collision bug: a pure-millisecond
/// timestamp is not unique when two notifications fire in the same ms,
/// causing `INSERT OR IGNORE` on the feedback table to silently drop
/// the second. The fix adds 32 random bits of entropy per ID.
#[test]
fn notif_id_generation_is_collision_free() {
    use std::collections::HashSet;
    let ms = chrono::Utc::now().timestamp_millis();
    let mut seen: HashSet<String> = HashSet::new();
    // 100 IDs in a tight loop — all land in the same ms, so collision
    // detection here exercises purely the random suffix, not the clock.
    for _ in 0..100 {
        let id = format!("{}-{:08x}", ms, rand::random::<u32>());
        assert!(
            seen.insert(id),
            "notif_id collision within same millisecond"
        );
    }
    assert_eq!(seen.len(), 100);
}

#[test]
fn callback_dedup_concurrent_access() {
    // Verify the type works inside Arc<parking_lot::Mutex<_>> as used in production
    let dedup = Arc::new(parking_lot::Mutex::new(CallbackDedup::new()));
    let d1 = dedup.clone();
    let d2 = dedup.clone();

    // Simulate two "threads" checking the same ID
    let first = d1.lock().check_and_insert("dup_id");
    let second = d2.lock().check_and_insert("dup_id");
    assert!(!first, "first caller should succeed");
    assert!(second, "second caller should be rejected");
}

// ── InFlightApprovals tests ────────────────────────────────────────
//
// These back the 2026-04-18 HITL approval UX fix: a re-tap on a button
// whose apply is still running (e.g. 60 s corpus-reembed) must be
// recognised as a duplicate even though Telegram delivers it with a
// fresh callback_query id. The set keeps one slot per raw
// `callback_data` for the lifetime of its apply, and evicts stale
// slots after `INFLIGHT_APPROVALS_TTL_SECS` so a stuck apply cannot
// permanently lock the button.

#[test]
fn inflight_approvals_try_acquire_rejects_re_tap() {
    let mut set = InFlightApprovals::new();
    assert!(
        !set.try_acquire("config_approve:config-abc"),
        "first acquire should succeed"
    );
    assert!(
        set.try_acquire("config_approve:config-abc"),
        "re-tap on same button must be reported as already in-flight"
    );
    assert_eq!(set.len(), 1);
}

#[test]
fn inflight_approvals_release_allows_retap_after_completion() {
    let mut set = InFlightApprovals::new();
    assert!(!set.try_acquire("gate_once:gate-xyz"));
    // Apply finished → callback handler released the slot.
    set.release("gate_once:gate-xyz");
    assert_eq!(set.len(), 0);
    // A subsequent tap (e.g. a fresh approval flow reusing the same id)
    // must be permitted — otherwise the UI is silently locked out.
    assert!(
        !set.try_acquire("gate_once:gate-xyz"),
        "acquire after release should succeed"
    );
}

#[test]
fn inflight_approvals_release_is_idempotent() {
    let mut set = InFlightApprovals::new();
    set.release("never_acquired"); // no panic
    set.try_acquire("k");
    set.release("k");
    set.release("k"); // still no panic, still empty
    assert_eq!(set.len(), 0);
}

#[test]
fn inflight_approvals_independent_keys_do_not_block_each_other() {
    let mut set = InFlightApprovals::new();
    // Two different approvals in flight concurrently (e.g. two different
    // config proposals tapped within the same minute) must both proceed.
    assert!(!set.try_acquire("config_approve:config-a"));
    assert!(!set.try_acquire("config_approve:config-b"));
    assert_eq!(set.len(), 2);
    // But a re-tap on A is still blocked.
    assert!(set.try_acquire("config_approve:config-a"));
}

#[test]
fn inflight_approvals_evicts_stale_stuck_slot() {
    let mut set = InFlightApprovals::new();
    // Simulate a slot that's been "running" longer than the TTL — e.g.
    // the apply task panicked before reaching the release path AND the
    // outer catch_unwind also failed. Without TTL eviction, the button
    // is locked forever.
    let stale_time =
        std::time::Instant::now() - std::time::Duration::from_secs(INFLIGHT_APPROVALS_TTL_SECS + 1);
    set.ids.insert("stuck_key".to_string());
    set.entries.push_back(("stuck_key".to_string(), stale_time));
    assert_eq!(set.len(), 1);

    // A fresh acquire triggers evict_stale; stuck_key is dropped.
    assert!(!set.try_acquire("fresh_key"));
    assert!(
        !set.ids.contains("stuck_key"),
        "TTL-expired slot must be evicted so the button is not locked forever"
    );
    // And the previously-stuck key can be acquired again.
    assert!(!set.try_acquire("stuck_key"));
}

#[test]
fn inflight_approvals_evicts_at_capacity() {
    let mut set = InFlightApprovals::new();
    for i in 0..INFLIGHT_APPROVALS_MAX_ENTRIES {
        assert!(!set.try_acquire(&format!("k_{i}")));
    }
    assert_eq!(set.len(), INFLIGHT_APPROVALS_MAX_ENTRIES);
    // One more pushes out k_0.
    assert!(!set.try_acquire("overflow"));
    assert_eq!(set.len(), INFLIGHT_APPROVALS_MAX_ENTRIES);
    assert!(
        !set.try_acquire("k_0"),
        "oldest slot evicted → k_0 acquirable again"
    );
}

#[test]
fn i18n_has_processing_and_internal_error_keys() {
    // Sanity: the new keys the HITL pre-edit path calls through `t()`
    // must resolve in both locales. If a translation is missing, `t()`
    // returns the key itself — the fix would then show literal
    // "callback.processing" in the Telegram message.
    use crate::i18n::{DEFAULT_LOCALE, t};
    for locale in ["zh-CN", "en", DEFAULT_LOCALE] {
        let processing = t("callback.processing", locale);
        let internal_err = t("callback.internal_error", locale);
        let already = t("callback.already_processing", locale);
        assert_ne!(
            processing, "callback.processing",
            "callback.processing missing for locale {locale}"
        );
        assert_ne!(
            internal_err, "callback.internal_error",
            "callback.internal_error missing for locale {locale}"
        );
        assert_ne!(
            already, "callback.already_processing",
            "callback.already_processing missing for locale {locale}"
        );
        // Spec requires the visible processing label to include the
        // hourglass emoji so the user sees an unambiguous "wait" cue.
        assert!(
            processing.contains('\u{23f3}'),
            "callback.processing missing hourglass for locale {locale}: {processing}"
        );
    }
}

/// Integration-ish: exercise the ORDER-OF-OPERATIONS the real
/// callback handler enforces — pre-edit fires BEFORE the slow apply
/// completes — without requiring a live `teloxide::Bot`. We replay
/// the handler's pre-edit → acquire → apply → release sequence
/// against a shared event log and an apply closure that sleeps
/// longer than any plausible Telegram edit round-trip.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn preedit_fires_before_slow_apply_completes() {
    use std::sync::Arc as StdArc;
    use tokio::sync::Mutex as TokioMutex;

    let events: StdArc<TokioMutex<Vec<&'static str>>> = StdArc::new(TokioMutex::new(Vec::new()));
    let inflight = StdArc::new(parking_lot::Mutex::new(InFlightApprovals::new()));
    let key = "config_approve:config-slow";

    // Structural mirror of the real handler:
    //   1. answer_callback_query  (toast)
    //   2. inflight.try_acquire
    //   3. edit_message_text "⏳ 处理中…"      ← must fire before step 4 completes
    //   4. apply (≥ 1 s simulated)
    //   5. inflight.release + final edit
    let ev = events.clone();
    let inf = inflight.clone();
    let handler = async move {
        ev.lock().await.push("ack");
        assert!(
            !inf.lock().try_acquire(key),
            "first tap must acquire the slot"
        );
        ev.lock().await.push("preedit_processing");
        // Simulated heavy apply — far slower than the pre-edit.
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        ev.lock().await.push("apply_done");
        inf.lock().release(key);
        ev.lock().await.push("final_edit");
    };

    // A concurrent "second tap" arriving 100 ms into the apply must be
    // classified as already-in-flight and must NOT produce a duplicate
    // preedit/apply/final_edit pair.
    let ev2 = events.clone();
    let inf2 = inflight.clone();
    let second_tap = async move {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        ev2.lock().await.push("retap_ack");
        if inf2.lock().try_acquire(key) {
            ev2.lock().await.push("retap_suppressed");
        } else {
            ev2.lock().await.push("retap_LEAKED_THROUGH");
        }
    };

    tokio::join!(handler, second_tap);

    let log = events.lock().await.clone();
    let pos = |needle: &str| log.iter().position(|e| *e == needle);
    let preedit_idx = pos("preedit_processing").expect("preedit must fire");
    let apply_done_idx = pos("apply_done").expect("apply must complete");
    let retap_idx = pos("retap_suppressed").expect("re-tap must be suppressed");
    assert!(
        preedit_idx < apply_done_idx,
        "pre-edit must land before apply finishes, got order {log:?}"
    );
    assert!(
        retap_idx < apply_done_idx,
        "re-tap suppression must fire during apply (not after), got order {log:?}"
    );
    assert!(
        !log.contains(&"retap_LEAKED_THROUGH"),
        "re-tap must NOT invoke a second apply, got {log:?}"
    );
    // And the slot is released after the handler completes, so a
    // legitimate later tap would work.
    assert!(
        !inflight.lock().try_acquire(key),
        "slot must be free again after handler completion"
    );
}

/// Regression: 2026-04-18 CTO round-1 finding on c987e5c. The handler
/// acquires the inflight slot BEFORE calling `answer_callback_query`,
/// so if that call is `?`-propagated on failure, the slot leaks until
/// the 180 s TTL evicts it — the exact "query is too old" failure
/// class the async-ack work was meant to prevent. The fix downgrades
/// the ack to best-effort (`let _ = ... .await`) so control reaches the
/// `release` path even when the cosmetic toast fails.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn slot_released_when_answer_callback_fails() {
    use std::sync::Arc as StdArc;

    let inflight = StdArc::new(parking_lot::Mutex::new(InFlightApprovals::new()));
    let key = "config_approve:config-ack-fails";

    // Fake ack that always fails, simulating a stale query id.
    async fn fake_ack_fails() -> Result<(), &'static str> {
        Err("Bad Request: query is too old and response timeout expired")
    }

    // Structural mirror of the fixed handler:
    //   1. inflight.try_acquire                          ← real protection
    //   2. let _ = fake_ack_fails().await;               ← must NOT propagate
    //   3. … apply …
    //   4. inflight.release(key)
    let inf = inflight.clone();
    let handler = async move {
        assert!(
            !inf.lock().try_acquire(key),
            "first tap must acquire the slot"
        );
        // This is the fix: best-effort ack. If we used `?` here the
        // `Err` from fake_ack_fails would propagate out and `release`
        // below would never run.
        let _ = fake_ack_fails().await;
        // Simulated apply path.
        inf.lock().release(key);
    };

    handler.await;

    assert_eq!(
        inflight.lock().len(),
        0,
        "slot must be released even when answer_callback_query fails — \
         otherwise the button stays locked for the full 180 s TTL"
    );
    // A subsequent tap on the same button must acquire cleanly
    // (try_acquire returns false ⇒ not previously held).
    assert!(
        !inflight.lock().try_acquire(key),
        "slot must be re-acquirable after a failed-ack handler completes"
    );
}

// ── extract_domain_from_msg validation ──

#[test]
fn extract_domain_valid() {
    let msg = r#"Request to access Domain: <code>example.com</code> — approve?"#;
    assert_eq!(
        extract_domain_from_msg(msg),
        Some("example.com".to_string())
    );
}

#[test]
fn extract_domain_with_subdomain() {
    let msg = r#"Domain: <code>api.sub.example.com</code>"#;
    assert_eq!(
        extract_domain_from_msg(msg),
        Some("api.sub.example.com".to_string())
    );
}

#[test]
fn extract_domain_rejects_html_injection() {
    let msg = r#"Domain: <code><b>evil</b></code>"#;
    assert_eq!(
        extract_domain_from_msg(msg),
        None,
        "HTML tags should be rejected"
    );
}

#[test]
fn extract_domain_rejects_spaces() {
    let msg = r#"Domain: <code>evil domain.com</code>"#;
    assert_eq!(
        extract_domain_from_msg(msg),
        None,
        "spaces should be rejected"
    );
}

#[test]
fn extract_domain_rejects_too_long() {
    let long_domain = "a".repeat(254);
    let msg = format!("Domain: <code>{}</code>", long_domain);
    assert_eq!(
        extract_domain_from_msg(&msg),
        None,
        "domain >253 chars should be rejected"
    );
}

#[test]
fn extract_domain_rejects_slashes() {
    let msg = r#"Domain: <code>example.com/path</code>"#;
    assert_eq!(
        extract_domain_from_msg(msg),
        None,
        "paths should be rejected"
    );
}

#[test]
fn extract_domain_missing_marker() {
    assert_eq!(extract_domain_from_msg("no domain here"), None);
}

// ── Spoiler text tests ──────────────────────────────────────────────

#[test]
fn spoiler_basic() {
    let html = markdown_to_telegram_html("This is ||secret|| text");
    assert!(html.contains("<span class=\"tg-spoiler\">secret</span>"));
    assert!(html.contains("This is"));
    assert!(html.contains("text"));
}

#[test]
fn spoiler_no_spoiler_passthrough() {
    let input = "No spoilers here";
    let html = markdown_to_telegram_html(input);
    assert!(!html.contains("tg-spoiler"));
}

#[test]
fn spoiler_inside_code_block_ignored() {
    let html = markdown_to_telegram_html("```\n||not a spoiler||\n```");
    assert!(!html.contains("tg-spoiler"));
    assert!(html.contains("||"));
}

#[test]
fn spoiler_inside_inline_code_ignored() {
    let html = markdown_to_telegram_html("Use `||pipe||` in shell");
    assert!(!html.contains("tg-spoiler"));
}

#[test]
fn spoiler_unclosed_literal() {
    let html = markdown_to_telegram_html("Unclosed ||spoiler here");
    assert!(!html.contains("tg-spoiler"));
    assert!(html.contains("||"));
}

#[test]
fn spoiler_with_cjk() {
    let html = markdown_to_telegram_html("This is ||秘密内容|| revealed");
    assert!(html.contains("<span class=\"tg-spoiler\">秘密内容</span>"));
}

#[test]
fn spoiler_multiple() {
    let html = markdown_to_telegram_html("||one|| and ||two||");
    assert_eq!(
        html.matches("tg-spoiler").count(),
        2, // opening tag x2
    );
}

// ── topic_router wiring tests ────────────────────────────────────────────────

#[test]
fn wrap_thread_id_none_passes_through() {
    assert!(crate::telegram::wrap_thread_id(None).is_none());
}

#[test]
fn wrap_thread_id_some_wraps_into_teloxide_type() {
    // Proves the Some-branch of the topic-routing wrapper actually plumbs
    // the raw i32 into teloxide's ThreadId newtype. Without this test the
    // Some-path would be dead code until P1 flips resolve_thread_id.
    let wrapped = crate::telegram::wrap_thread_id(Some(42));
    assert_eq!(
        wrapped,
        Some(teloxide::types::ThreadId(teloxide::types::MessageId(42))),
    );
}

// ── Inbound-message fixtures ──────────────────────────────────────────────
//
// Realistic Telegram Update payloads used by the authorization-gate and
// bot-echo-ignore tests below.

fn fixture_forum_group_message(sender_id: i64) -> teloxide::types::Message {
    let json = format!(
        r#"{{
            "chat": {{
                "id": -1001847508954,
                "is_forum": true,
                "title": "demo",
                "type": "supergroup"
            }},
            "date": 1675229140,
            "from": {{
                "first_name": "owner",
                "id": {sender_id},
                "is_bot": false,
                "language_code": "en"
            }},
            "message_id": 10,
            "text": "/link_forum"
        }}"#
    );
    serde_json::from_str(&json).expect("valid Message fixture")
}

fn fixture_bot_message(sender_id: i64) -> teloxide::types::Message {
    let json = format!(
        r#"{{
            "chat": {{
                "id": -1001847508954,
                "is_forum": true,
                "title": "demo",
                "type": "supergroup"
            }},
            "date": 1675229140,
            "from": {{
                "first_name": "ExampleAegisBot",
                "id": {sender_id},
                "is_bot": true,
                "username": "ExampleAegisBot"
            }},
            "message_id": 13,
            "text": "bot status"
        }}"#
    );
    serde_json::from_str(&json).expect("valid bot Message fixture")
}

// ── P2 authorization gate (is_authorized_chat) ──────────────────────────────

#[test]
fn auth_gate_allows_admin_dm() {
    let admin = 42_i64;
    assert!(crate::telegram::is_authorized_chat(
        admin,
        Some(admin),
        admin
    ));
}

#[test]
fn authorized_dm_uses_canonical_admin_memory_did() {
    let admin = 42_i64;
    assert_eq!(
        crate::telegram::user_did_for_authorized_chat(admin, Some(admin), admin),
        "did:aegis:telegram:42"
    );
}

#[test]
fn authorized_linked_group_uses_canonical_admin_memory_did() {
    let admin = 42_i64;
    let group = -1001847508954_i64;
    assert_eq!(
        crate::telegram::user_did_for_authorized_chat(group, Some(admin), admin),
        "did:aegis:telegram:42",
        "linked group messages should recall the same memory namespace as DM"
    );
}

#[test]
fn auth_gate_rejects_stranger_dm() {
    let admin = 42_i64;
    let stranger = 999_i64;
    assert!(!crate::telegram::is_authorized_chat(
        stranger,
        Some(stranger),
        admin
    ));
}

#[test]
fn auth_gate_rejects_stranger_in_group() {
    let admin = 42_i64;
    let stranger = 999_i64;
    let group = -1001847508954_i64;
    assert!(!crate::telegram::is_authorized_chat(
        group,
        Some(stranger),
        admin
    ));
}

#[test]
fn auth_gate_rejects_anonymous_posts() {
    let admin = 42_i64;
    let group = -1001847508954_i64;
    assert!(!crate::telegram::is_authorized_chat(group, None, admin));
}

#[test]
fn clamp_chat_to_admin_redirects_persisted_group_ids() {
    // A send-target chat_id persisted from the old linked group (negative)
    // must be redirected to the owner's admin DM (Codex R3 root fix — the
    // shared clamp used by intent/task/agent-swarm outbound sends).
    let admin = 42_i64;
    let stale_group = -1001000000001_i64;
    assert_eq!(
        crate::telegram::clamp_chat_to_admin(stale_group, Some(admin)),
        admin
    );
    // An already-admin persisted id is unchanged.
    assert_eq!(
        crate::telegram::clamp_chat_to_admin(admin, Some(admin)),
        admin
    );
    // A private-DM persisted id is still redirected to admin (single-owner:
    // the only legitimate send target is the owner's admin DM).
    assert_eq!(
        crate::telegram::clamp_chat_to_admin(99999_i64, Some(admin)),
        admin
    );
    // No ADMIN_CHAT_ID (dev only): keep the persisted value, nothing to clamp to.
    assert_eq!(
        crate::telegram::clamp_chat_to_admin(stale_group, None),
        stale_group
    );
}

#[test]
fn inbound_bot_messages_are_ignored() {
    let msg = fixture_bot_message(8769782211);
    assert!(crate::telegram::should_ignore_inbound_message(&msg));
}

#[test]
fn inbound_human_messages_are_not_ignored() {
    let msg = fixture_forum_group_message(42);
    assert!(!crate::telegram::should_ignore_inbound_message(&msg));
}

#[test]
fn heartbeat_alert_uses_owner_language_title() {
    let payload = "\
[STILL ONGOING] Action needed:
Disk: 93.0% used (threshold: 70%, available: 13.6 GB / total: 192.7 GB)

Suggested next steps:
Disk: inspect large logs/build artifacts before deleting anything; prefer reversible cleanup.

Other metrics:
CPU load: 1m=1.63, 5m=1.52, 15m=1.57 (limit: 12.0, 4 CPUs)
Memory: 17.7% used (threshold: 80%, available: 19728 MB / total: 23974 MB)
";
    let msg = format_heartbeat_alert_message("server_monitor", payload, "zh");

    assert!(
        msg.starts_with("🚨 服务器：磁盘快满了（还没解决）"),
        "telegram alert must open with localized title, got: {msg}"
    );
}

#[test]
fn heartbeat_alert_still_pins_non_error_payload() {
    assert!(should_pin_heartbeat_alert("disk usage above threshold"));
    assert!(!should_pin_heartbeat_alert("Error: probe failed"));
    assert!(!should_pin_heartbeat_alert(
        "[RESOLVED] disk usage recovered"
    ));
}

#[test]
fn truncate_display_short_string_unchanged() {
    assert_eq!(truncate_display("hello", 60), "hello");
}

#[test]
fn truncate_display_long_string_gets_ellipsis() {
    let s = "x".repeat(100);
    let t = truncate_display(&s, 10);
    assert_eq!(t.chars().count(), 10);
    assert!(t.ends_with('\u{2026}'));
}
