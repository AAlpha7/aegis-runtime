//! Topic classification for Telegram sends (dormant routing scaffold).
//!
//! # Status
//!
//! Telegram forum/linked-group routing has been **removed**. Every outbound
//! send now targets the owner's admin DM. [`TopicKind`] and [`topic_for_class`]
//! are retained as a dormant classification vocabulary (still referenced by the
//! `send_*_to_topic` wrappers in `telegram.rs` and by the notification router),
//! but [`resolve_thread_id`] is now a constant `None`: no `message_thread_id`
//! is ever attached, so every send is a plain DM send.
//!
//! The store singleton (`ChatTopicStore`, `/link_forum` activation) that used
//! to populate real thread ids was deleted along with the group-routing
//! feature. Keeping the enum + the `async fn` signature avoids churning the
//! ~30 dormant `send_*_to_topic` callsites.

/// The four fixed Telegram forum topics Aegis Runtime classified messages into.
///
/// Dormant: no message is routed to a forum thread anymore, but the enum is
/// still used to tag/classify sends at their call sites.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub enum TopicKind {
    /// 💬 主对话 — LLM turn replies and user-visible tool progress.
    Main,
    /// 🔐 审批 — HITL approval prompts, scope downgrades, gate_clarify.
    Approval,
    /// 🔔 通知 — heartbeat proactive, reminders, KB alerts.
    Notif,
    /// 🛠 系统 — deploy pings, infra/health alerts.
    System,
}

/// Resolve the `message_thread_id` to attach for a `(chat_id, kind)` pair.
///
/// Always returns `None` now that forum/group routing is removed: every send
/// is a plain DM send with no `message_thread_id`. The `async` signature and
/// arguments are preserved so the dormant `send_*_to_topic` callsites do not
/// need to change.
pub async fn resolve_thread_id(_chat_id: i64, _kind: TopicKind) -> Option<i32> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The DM contract: `resolve_thread_id` always returns `None`, so no
    /// outbound Telegram send ever attaches a forum `message_thread_id`.
    #[tokio::test]
    async fn resolve_thread_id_always_none() {
        for chat_id in [i64::MIN + 7, 123_456_789, -1001_i64] {
            for kind in [
                TopicKind::Main,
                TopicKind::Approval,
                TopicKind::Notif,
                TopicKind::System,
            ] {
                assert_eq!(resolve_thread_id(chat_id, kind).await, None);
            }
        }
    }

    #[test]
    fn topic_kind_derives() {
        // Compile-time assertion that the four required traits are implemented.
        fn assert_traits<T: Copy + std::hash::Hash + Eq>() {}
        assert_traits::<TopicKind>();

        // Behavioral sanity: distinct variants compare unequal and copy is cheap.
        let a = TopicKind::Main;
        let b = a; // Copy
        assert_eq!(a, b);
        assert_ne!(TopicKind::Main, TopicKind::Approval);
        assert_ne!(TopicKind::Notif, TopicKind::System);

        // Hash: the four variants must produce a usable HashMap key.
        let mut set = std::collections::HashSet::new();
        set.insert(TopicKind::Main);
        set.insert(TopicKind::Approval);
        set.insert(TopicKind::Notif);
        set.insert(TopicKind::System);
        assert_eq!(set.len(), 4);
    }
}
