//! CEO Intent Contract — client/server shared shorthand layer.
//! Spec: docs/design/2026-07-25-ceo-intent-contract.md
//!
//! Expands high-frequency one-word / short CEO utterances into full natural
//! language so Telegram, voice, and any path that skips the WebApp client map
//! still get a usable prompt. The WebApp client may expand first; applying this
//! again is idempotent for already-expanded text (no key match → unchanged).

/// Resolved CEO shorthand hit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CeoShorthandResolution {
    pub intent_id: &'static str,
    /// Owner-facing label (Chinese preferred for this product).
    pub label_zh: &'static str,
    pub label_en: &'static str,
    pub expansion_zh: &'static str,
    pub expansion_en: &'static str,
    pub needs_review_hint: bool,
}

struct Entry {
    intent_id: &'static str,
    keys: &'static [&'static str],
    label_zh: &'static str,
    label_en: &'static str,
    expansion_zh: &'static str,
    expansion_en: &'static str,
    needs_review_hint: bool,
}

const ENTRIES: &[Entry] = &[
    Entry {
        intent_id: "commute_status",
        keys: &["通勤", "路况", "出门", "commute", "traffic"],
        label_zh: "查看上班路况与出发建议",
        label_en: "Commute and leave-by advice",
        expansion_zh: "看一下我现在的通勤路况，并建议我最晚什么时候出发。",
        expansion_en: "Check my commute conditions and recommend when I should leave.",
        needs_review_hint: false,
    },
    Entry {
        intent_id: "what_needs_me",
        keys: &["现在", "忙", "优先", "now", "busy"],
        label_zh: "现在最需要我拍板的事",
        label_en: "What needs me now",
        expansion_zh: "现在最需要我关注或拍板的是什么？只列最重要的几件。",
        expansion_en: "What needs my attention or decision right now? Only the most important items.",
        needs_review_hint: false,
    },
    Entry {
        intent_id: "lawyer_salience",
        keys: &["律师", "法务", "lawyer", "legal"],
        label_zh: "与律师/法务相关的事项",
        label_en: "Lawyer / legal items",
        expansion_zh: "找出与律师或法务相关的事项，给我简要情况和下一步建议。",
        expansion_en: "Find lawyer or legal-related work items, then brief me with next steps.",
        needs_review_hint: false,
    },
    Entry {
        intent_id: "billing_salience",
        keys: &["账单", "发票", "付款", "bill", "invoice", "billing"],
        label_zh: "账单与付款相关",
        label_en: "Bills and payments",
        expansion_zh: "汇总需要我看的账单或付款事项，写清金额和截止日期；不要自动付款。",
        expansion_en: "Summarize bills or payment items that need me, with amounts and due dates. Do not pay anything.",
        needs_review_hint: false,
    },
    Entry {
        intent_id: "draft_reply_focus",
        keys: &["回他", "回复", "回信", "reply"],
        label_zh: "起草回复（先不发送）",
        label_en: "Draft a reply (do not send)",
        expansion_zh: "针对当前焦点或最近需要回复的那封信，起草一封回复给我审阅，先不要发送。",
        expansion_en: "Draft a reply for the focused or most recent message that needs a response. Do not send it yet.",
        needs_review_hint: true,
    },
    Entry {
        intent_id: "approve_pending",
        keys: &["批", "批准", "就这样", "好", "ok", "approve"],
        label_zh: "批准当前待确认事项",
        label_en: "Approve the pending review",
        expansion_zh: "批准我当前待确认的那一项（若有多项，先说明是哪一项）。",
        expansion_en: "Approve the item currently waiting for my review (if several, say which one first).",
        needs_review_hint: false,
    },
    Entry {
        intent_id: "reject_pending",
        keys: &["不用", "否", "拒绝", "别", "no", "reject"],
        label_zh: "拒绝当前待确认事项",
        label_en: "Reject the pending review",
        expansion_zh: "拒绝或取消我当前待确认的那一项，不要继续执行。",
        expansion_en: "Reject or cancel the item currently waiting for my review. Do not proceed.",
        needs_review_hint: false,
    },
    Entry {
        intent_id: "today_brief",
        keys: &["今天", "今日", "today"],
        label_zh: "今日简报",
        label_en: "Today brief",
        expansion_zh: "给我今天的简报：日程、需要我的事、以及你已安静处理的部分。",
        expansion_en: "Give me today's brief: schedule, what needs me, and what you already handled quietly.",
        needs_review_hint: false,
    },
    Entry {
        intent_id: "quiet_mode",
        keys: &["安静", "勿扰", "quiet", "dnd"],
        label_zh: "减少主动打扰",
        label_en: "Quiet mode",
        expansion_zh: "暂时减少主动打扰和自启动检查；告诉我如何恢复。",
        expansion_en: "Reduce proactive pings and self-started checks for now, and tell me how to resume.",
        needs_review_hint: false,
    },
    Entry {
        intent_id: "triage_open_loops",
        keys: &["帮我处理", "处理", "handle", "triage"],
        label_zh: "帮我处理待办",
        label_en: "Handle open loops",
        expansion_zh: "帮我处理当前待办：能安静办的先办，需要我拍板的列出来。",
        expansion_en: "Handle my open loops: do what you can quietly, and list only what needs my decision.",
        needs_review_hint: true,
    },
];

fn normalize(raw: &str) -> String {
    raw.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// Resolve an exact CEO shorthand key. Returns `None` for free text.
/// Greeting / thanks / acknowledgement: short, no task content. Such turns
/// get a plain reply instead of the chief-of-staff frame.
pub fn is_casual_utterance(text: &str) -> bool {
    let t = text
        .trim()
        .trim_end_matches(['!', '！', '.', '。', '~', '～', '?', '？'])
        .trim();
    if t.is_empty() || t.chars().count() > 12 {
        return false;
    }
    let lower = t.to_lowercase();
    const CASUAL: &[&str] = &[
        "hi",
        "hello",
        "hey",
        "yo",
        "hi hi",
        "hello hello",
        "hi, hello",
        "hello, hello",
        "good morning",
        "good night",
        "thanks",
        "thank you",
        "thx",
        // "ok" / "好" are approval shorthands (CEO_SHORTHANDS) and stay out.
        "okay",
        "got it",
        "cool",
        "nice",
        "great",
        "在吗",
        "在不在",
        "在么",
        "你好",
        "您好",
        "嗨",
        "哈喽",
        "哈罗",
        "早",
        "早上好",
        "早安",
        "午安",
        "晚上好",
        "晚安",
        "谢谢",
        "谢啦",
        "多谢",
        "好的",
        "嗯",
        "嗯嗯",
        "收到",
        "知道了",
        "明白",
        "了解",
        "辛苦了",
        "不错",
        "很好",
        "可以",
        "行",
    ];
    CASUAL.iter().any(|c| *c == lower)
}

pub fn resolve_ceo_shorthand(raw: &str) -> Option<CeoShorthandResolution> {
    let key = normalize(raw);
    if key.is_empty() {
        return None;
    }
    let trimmed = raw.trim();
    for entry in ENTRIES {
        let hit = entry
            .keys
            .iter()
            .any(|k| *k == trimmed || k.eq_ignore_ascii_case(&key) || normalize(k) == key);
        if !hit {
            continue;
        }
        return Some(CeoShorthandResolution {
            intent_id: entry.intent_id,
            label_zh: entry.label_zh,
            label_en: entry.label_en,
            expansion_zh: entry.expansion_zh,
            expansion_en: entry.expansion_en,
            needs_review_hint: entry.needs_review_hint,
        });
    }
    None
}

/// Expand short CEO utterances for the orchestrator. `prefer_zh` selects
/// Chinese expansions (owner display_language zh). Already-expanded text that
/// does not match a key is returned unchanged.
pub fn expand_ceo_utterance_for_runtime(raw: &str, prefer_zh: bool) -> String {
    prepare_ceo_runtime_user_text(raw, prefer_zh).0
}

/// Prepare runtime user text + optional shorthand hit.
/// - Shorthand / known expansion → expanded prompt + hit
/// - Free text → CEO contract frame so the model acts as chief of staff
pub fn prepare_ceo_runtime_user_text(
    raw: &str,
    prefer_zh: bool,
) -> (String, Option<CeoShorthandResolution>) {
    if let Some(hit) = resolve_ceo_intent_from_text(raw) {
        let expansion = if prefer_zh {
            hit.expansion_zh.to_string()
        } else {
            hit.expansion_en.to_string()
        };
        return (expansion, Some(hit));
    }
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return (raw.to_string(), None);
    }
    // Greetings, thanks and one-word acknowledgements get no chief-of-staff
    // frame: framing them produced "你这是打个招呼…回执：…" (owner, 2026-09-06).
    if is_casual_utterance(trimmed) {
        return (trimmed.to_string(), None);
    }
    let framed = if prefer_zh {
        format!(
            "【幕僚长模式】用户是极忙的 CEO。不要复述或点评用户的意图，直接处理或编排；\
只有意图确实不清时才问一个澄清问题。外发消息、付款、删除、扩大隐私访问必须先请求确认。\
不要让用户先选模块（工作）。真正执行了动作时，结尾给一句结果与回执；纯回答不加回执。\n\n用户原话：\n{trimmed}"
        )
    } else {
        format!(
            "[Chief of staff mode] The owner is a busy CEO. Do not restate or comment on their intent; act or orchestrate directly, \
asking one clarifying question only when the intent is genuinely unclear. Outbound messages, payments, deletes, and privacy upgrades \
require review first. Do not ask them to pick a module (Work). When an action was actually executed, end with a one-line \
result and receipt; plain answers carry no receipt.\n\nOwner said:\n{trimmed}"
        )
    };
    (framed, None)
}

/// Owner-facing free-text intent label for the working bubble / SSE.
pub fn free_text_intent_label(prefer_zh: bool) -> &'static str {
    if prefer_zh {
        "理解你的意图并处理"
    } else {
        "Understanding and handling your request"
    }
}

/// Contract §3 IntentResolution JSON for free-text (model path placeholder).
/// Full model-emitted IntentResolution lands when the orchestrator returns a
/// structured resolve step; this frame keeps schema_version stable for clients.
pub fn free_text_intent_resolution_json(raw: &str, prefer_zh: bool) -> serde_json::Value {
    let trimmed = raw.trim();
    let label = free_text_intent_label(prefer_zh);
    serde_json::json!({
        "schema_version": 1,
        "raw_input": trimmed,
        "display_input": trimmed,
        "intent_id": "free_text",
        "intent_label": label,
        "confidence": 0.55,
        "targets": [],
        "plan": [
            { "step": "confirm_intent", "risk": "read" },
            { "step": "act_or_ask", "risk": "organize" }
        ],
        "risk_class": "organize",
        "needs_review": false,
        "clarification": null,
        "say_to_owner": label,
    })
}

/// IntentResolution JSON for a resolved CEO shorthand hit.
pub fn shorthand_intent_resolution_json(
    hit: &CeoShorthandResolution,
    raw: &str,
    prefer_zh: bool,
) -> serde_json::Value {
    let label = if prefer_zh {
        hit.label_zh
    } else {
        hit.label_en
    };
    let expansion = if prefer_zh {
        hit.expansion_zh
    } else {
        hit.expansion_en
    };
    serde_json::json!({
        "schema_version": 1,
        "raw_input": raw.trim(),
        "display_input": raw.trim(),
        "intent_id": hit.intent_id,
        "intent_label": label,
        "confidence": 0.9,
        "targets": [],
        "plan": [
            { "step": "expand_shorthand", "risk": "read" },
            { "step": "orchestrate", "risk": if hit.needs_review_hint { "draft" } else { "read" } }
        ],
        "risk_class": if hit.needs_review_hint { "draft" } else { "read" },
        "needs_review": hit.needs_review_hint,
        "clarification": null,
        "say_to_owner": expansion,
    })
}

/// Resolve a CEO intent from either a shorthand key **or** a known expansion
/// string (WebApp client may already expand before POST).
pub fn resolve_ceo_intent_from_text(raw: &str) -> Option<CeoShorthandResolution> {
    if let Some(hit) = resolve_ceo_shorthand(raw) {
        return Some(hit);
    }
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    for entry in ENTRIES {
        if entry.expansion_zh == trimmed || entry.expansion_en == trimmed {
            return Some(CeoShorthandResolution {
                intent_id: entry.intent_id,
                label_zh: entry.label_zh,
                label_en: entry.label_en,
                expansion_zh: entry.expansion_zh,
                expansion_en: entry.expansion_en,
                needs_review_hint: entry.needs_review_hint,
            });
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn casual_utterances_skip_the_chief_of_staff_frame() {
        for t in [
            "hi",
            "Hello, hello.",
            "你好",
            "早上好！",
            "谢谢",
            "okay",
            "在吗？",
            "收到",
        ] {
            assert!(super::is_casual_utterance(t), "{t}");
            let (prepared, hit) = super::prepare_ceo_runtime_user_text(t, true);
            assert!(hit.is_none());
            assert!(
                !prepared.contains("幕僚长模式"),
                "{t} must not be framed: {prepared}"
            );
        }
        for t in [
            "帮我看看今天的安排",
            "hi, can you check my calendar",
            "把牙医预约改到周四",
            "还有什么事情需要做",
        ] {
            assert!(!super::is_casual_utterance(t), "{t}");
        }
        // Approval shorthands keep their meaning and are never treated as chit-chat.
        for t in ["ok", "好"] {
            assert!(!super::is_casual_utterance(t), "{t}");
            assert!(super::prepare_ceo_runtime_user_text(t, true).1.is_some());
        }
    }

    #[test]
    fn frame_no_longer_asks_to_restate_intent() {
        let (zh, _) = super::prepare_ceo_runtime_user_text("帮我看看今天的安排", true);
        assert!(zh.contains("不要复述"), "{zh}");
        assert!(!zh.contains("先用一句中文确认"), "{zh}");
        let (en, _) = super::prepare_ceo_runtime_user_text("check my calendar", false);
        assert!(en.contains("Do not restate"), "{en}");
    }

    #[test]
    fn expands_commute_zh() {
        let hit = resolve_ceo_shorthand("通勤").expect("hit");
        assert_eq!(hit.intent_id, "commute_status");
        assert!(hit.expansion_zh.contains("通勤") || hit.expansion_zh.contains("出发"));
        assert!(!hit.needs_review_hint);
    }

    #[test]
    fn free_text_has_no_shorthand_hit() {
        let long = "请帮我仔细分析一下这周所有的法务往来文件并写纪要";
        assert!(resolve_ceo_shorthand(long).is_none());
        assert!(resolve_ceo_intent_from_text(long).is_none());
        // Free text is framed for the model, not left raw.
        let expanded = expand_ceo_utterance_for_runtime(long, true);
        assert!(expanded.contains("幕僚长模式"));
        assert!(expanded.contains(long));
    }

    #[test]
    fn expand_idempotent_on_non_key() {
        let already = "看一下我现在的通勤路况，并建议我最晚什么时候出发。";
        assert_eq!(expand_ceo_utterance_for_runtime(already, true), already);
    }

    #[test]
    fn resolve_from_client_expanded_prompt() {
        let expansion = "看一下我现在的通勤路况，并建议我最晚什么时候出发。";
        let hit = resolve_ceo_intent_from_text(expansion).expect("reverse map");
        assert_eq!(hit.intent_id, "commute_status");
    }

    #[test]
    fn free_text_gets_ceo_frame() {
        let (text, hit) = prepare_ceo_runtime_user_text("帮我看看下周和投资人怎么谈", true);
        assert!(hit.is_none());
        assert!(text.contains("幕僚长模式"));
        assert!(text.contains("帮我看看下周和投资人怎么谈"));
    }

    #[test]
    fn free_text_intent_resolution_has_schema() {
        let v = free_text_intent_resolution_json("帮我处理本周法务", true);
        assert_eq!(v["schema_version"], 1);
        assert_eq!(v["intent_id"], "free_text");
        assert!(v["intent_label"].as_str().unwrap_or("").contains("理解"));
        assert!(v["plan"].as_array().map(|a| !a.is_empty()).unwrap_or(false));
    }

    #[test]
    fn shorthand_intent_resolution_has_schema() {
        let hit = resolve_ceo_shorthand("通勤").expect("hit");
        let v = shorthand_intent_resolution_json(&hit, "通勤", true);
        assert_eq!(v["intent_id"], "commute_status");
        assert_eq!(v["confidence"], 0.9);
        assert_eq!(v["needs_review"], false);
    }
}
