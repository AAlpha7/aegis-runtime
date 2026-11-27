//! Runtime-authored disclosures, independent of the model's final wording.
//! A task-local scope keeps concurrent owner turns isolated. Notices are sent
//! live and prepended to the durable reply, even when the progress queue is full.

use crate::progress_router::{
    OwnerNoticeKind, ProgressEvent, ProgressPayload, ProgressRouterSender,
};
use parking_lot::Mutex;
use std::future::Future;

struct NoticeScope {
    language: String,
    entries: Mutex<Vec<(String, String)>>,
    progress: Option<ProgressRouterSender>,
    chat_id: Option<i64>,
}

tokio::task_local! { static SCOPE: NoticeScope; }

pub async fn collect<T>(
    language: &str,
    progress: Option<ProgressRouterSender>,
    chat_id: Option<i64>,
    work: impl Future<Output = T>,
) -> (T, Vec<String>) {
    SCOPE
        .scope(
            NoticeScope {
                language: language.to_string(),
                entries: Mutex::new(Vec::new()),
                progress,
                chat_id,
            },
            async {
                let result = work.await;
                let notices = SCOPE.with(|scope| {
                    scope
                        .entries
                        .lock()
                        .iter()
                        .map(|(_, text)| text.clone())
                        .collect()
                });
                (result, notices)
            },
        )
        .await
}

/// Only runtime-authored reasons belong here. Never pass provider errors,
/// search queries, credentials, page content, or connector output as copy.
pub fn record(code: &str, zh: &str, en: &str) {
    record_inner(code, zh, en, true);
}

/// For paths that already emit a typed progress event.
pub fn remember(code: &str, zh: &str, en: &str) {
    record_inner(code, zh, en, false);
}

fn record_inner(code: &str, zh: &str, en: &str, emit: bool) {
    let _ = SCOPE.try_with(|scope| {
        let mut entries = scope.entries.lock();
        if entries.iter().any(|(existing, _)| existing == code) {
            return;
        }
        let text = if scope.language.starts_with("zh") {
            zh
        } else {
            en
        }
        .to_string();
        entries.push((code.to_string(), text.clone()));
        drop(entries);
        if emit && let (Some(tx), Some(cid)) = (&scope.progress, scope.chat_id) {
            let _ = tx.try_send(ProgressEvent::new(
                cid,
                ProgressPayload::OwnerNotice {
                    text,
                    kind: OwnerNoticeKind::System,
                },
            ));
        }
    });
}

pub fn prepend(reply: &str, notices: &[String]) -> String {
    if notices.is_empty() {
        return reply.to_string();
    }
    let prefix = notices
        .iter()
        .map(|text| format!("⚠️ {text}"))
        .collect::<Vec<_>>()
        .join("\n\n");
    format!("{prefix}\n\n{reply}")
}

/// Failure hints keep their leading sentence so history retains Retry actions.
pub fn append_failure(reply: &str, notices: &[String]) -> String {
    if notices.is_empty() {
        return reply.to_string();
    }
    format!("{reply}\n\n{}", prepend("", notices).trim_end())
}

/// Keep notices as typed context: failure classifiers still see the original
/// error, while history can preserve safe runtime copy without parsing errors.
#[derive(Debug)]
pub struct FailedTurnNotices {
    message: String,
    pub notices: Vec<String>,
}

impl std::fmt::Display for FailedTurnNotices {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

pub fn attach_failure(error: anyhow::Error, notices: Vec<String>) -> anyhow::Error {
    if notices.is_empty() {
        return error;
    }
    let message = error.to_string();
    error.context(FailedTurnNotices { message, notices })
}

/// Fixed copy only: untrusted skill errors/body are never rendered as notices.
pub fn skill_route(tool: &str, raw: &str) {
    if tool != "web_reader" {
        return;
    }
    let Ok(value) = serde_json::from_str::<serde_json::Value>(raw) else {
        return;
    };
    if value["via"] == "jina_reader" {
        record(
            "reader_jina",
            "网页直读失败，已改用 Jina Reader；页面地址已交给第三方读取，内容完整性和时效可能不同。",
            "Direct page reading failed; Jina Reader was used. The page URL was sent to a third party, and completeness and freshness may differ.",
        );
    } else if value["status"] == "fetch_failed"
        && value["warning"]
            .as_str()
            .is_some_and(|s| s.contains("Jina Reader fallback also failed"))
    {
        record(
            "reader_jina_failed",
            "网页直读和 Jina Reader 兜底均失败；页面地址已交给 Jina，但未取得可用正文，不能声称已读到原文。",
            "Direct reading and Jina Reader fallback both failed. The URL was sent to Jina, but no usable page text was retrieved; the original content has not been verified.",
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_turn_preserves_original_reason_and_typed_disclosures() {
        let error = attach_failure(
            anyhow::anyhow!("primary and backup unavailable"),
            vec!["Fallback attempted".into()],
        );
        assert_eq!(error.to_string(), "primary and backup unavailable");
        assert_eq!(
            error.downcast_ref::<FailedTurnNotices>().unwrap().notices,
            ["Fallback attempted"]
        );
    }

    #[tokio::test]
    async fn live_notice_and_full_queue_both_keep_durable_copy() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        let (_, notices) = collect(
            "en",
            Some(ProgressRouterSender::direct(tx)),
            Some(7),
            async {
                record("first", "首个", "First change");
                record("second", "第二个", "Second change");
            },
        )
        .await;
        let event = rx.recv().await.unwrap();
        assert!(
            matches!(*event.payload, ProgressPayload::OwnerNotice { text, .. } if text == "First change")
        );
        assert_eq!(notices, ["First change", "Second change"]);
    }

    #[tokio::test]
    async fn reader_fallback_discloses_third_party_without_echoing_body_or_error() {
        let (_, notices) = collect("en", None, None, async {
            skill_route("web_reader", r#"{"via":"jina_reader","body":"private body"}"#);
            skill_route("web_reader", r#"{"status":"fetch_failed","warning":"Jina Reader fallback also failed secret-token"}"#);
            skill_route("other", r#"{"via":"jina_reader"}"#);
        }).await;
        assert_eq!(notices.len(), 2);
        assert!(notices[0].contains("third party"));
        assert!(notices[1].contains("no usable page text"));
        assert!(!notices.join(" ").contains("private body"));
        assert!(!notices.join(" ").contains("secret-token"));
    }

    #[tokio::test]
    async fn disclosures_survive_without_progress_and_do_not_cross_turns() {
        let ((_, zh), (_, en)) = tokio::join!(
            collect("zh", None, None, async {
                record("fallback", "搜索已降级", "Search degraded");
                tokio::task::yield_now().await;
                record("fallback", "重复", "Duplicate");
            }),
            collect("en", None, None, async {
                record("fallback", "另一个会话", "Another turn");
            })
        );
        assert_eq!(zh, ["搜索已降级"]);
        assert_eq!(en, ["Another turn"]);
        assert!(prepend("answer\n@@search-receipt", &zh).starts_with("⚠️ 搜索已降级"));
        let (_, fresh) = collect("zh", None, None, async {}).await;
        assert!(fresh.is_empty());
    }
}
