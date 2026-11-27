//! Policy layer for Telegram notification destinations.
//!
//! Telegram forum/linked-group routing was removed, so every send now targets
//! the owner's admin DM. This module still exists to (a) apply the DM/OAuth
//! routing rules and privacy fallbacks, and (b) render notification payloads
//! with sensitivity-aware redaction per channel.

use crate::topic_router::TopicKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationPrivacy {
    PublicSafe,
    PersonalMetadata,
    SensitiveRedacted,
    HighlySensitiveRedacted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationClass {
    DirectUserReply,
    ProactiveAlert,
    ApprovalRequest,
    RoutineNotification,
    SystemStatus,
    DailyBrief,
    EveningReview,
    OAuthDeepLinkReply,
    StartupKeyboard,
    PrivacySensitiveDmOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrivacyLevel {
    PrivateDmOnly,
    SensitivePreferDm,
    SystemOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationRouteReason {
    DirectDmContinuation,
    PrivacyDmOnly,
    SensitivePreferDm,
    OAuthDeepLinkException,
    NoLinkedGroupFallbackToDm,
    ExplicitStoredChatAllowed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TelegramNotificationRouter {
    admin_chat_id: i64,
}

impl TelegramNotificationRouter {
    pub fn new(admin_chat_id: i64) -> Self {
        Self { admin_chat_id }
    }

    /// Retained for call-site stability. Linked-forum/group routing was
    /// removed — every destination is now the owner's admin DM — so this is
    /// just `Self::new(admin_chat_id)`.
    pub fn from_topic_store(admin_chat_id: i64) -> Self {
        Self::new(admin_chat_id)
    }

    pub fn route(&self, request: NotificationRouteRequest) -> NotificationRoute {
        if request.notification_class == NotificationClass::OAuthDeepLinkReply {
            return NotificationRoute {
                chat_id: request
                    .origin_chat_id
                    .or(request.stored_chat_id)
                    .unwrap_or(self.admin_chat_id),
                thread_id: None,
                topic_kind: None,
                reason: NotificationRouteReason::OAuthDeepLinkException,
            };
        }

        if request.notification_class == NotificationClass::PrivacySensitiveDmOnly
            || request.privacy_level == PrivacyLevel::PrivateDmOnly
        {
            return self.dm_route(
                self.admin_dm_only(request.origin_chat_id.or(request.stored_chat_id)),
                NotificationRouteReason::PrivacyDmOnly,
            );
        }

        if request.privacy_level == PrivacyLevel::SensitivePreferDm {
            return self.dm_route(
                self.admin_dm_only(request.origin_chat_id.or(request.stored_chat_id)),
                NotificationRouteReason::SensitivePreferDm,
            );
        }

        if request.notification_class == NotificationClass::DirectUserReply
            && request.user_initiated
            && request.allow_dm_continuation
        {
            if let Some(origin_chat_id) = request.origin_chat_id
                && origin_chat_id == self.admin_chat_id
            {
                return NotificationRoute {
                    chat_id: origin_chat_id,
                    thread_id: None,
                    topic_kind: None,
                    reason: NotificationRouteReason::DirectDmContinuation,
                };
            }

            if let Some(stored_chat_id) = request.stored_chat_id {
                return NotificationRoute {
                    chat_id: stored_chat_id,
                    thread_id: None,
                    topic_kind: None,
                    reason: NotificationRouteReason::ExplicitStoredChatAllowed,
                };
            }
        }

        // Group/forum routing was removed: everything else lands in the
        // owner's admin DM with no forum thread.
        self.dm_route(None, NotificationRouteReason::NoLinkedGroupFallbackToDm)
    }

    /// Clamp a requested destination to the owner's admin DM.
    ///
    /// The privacy-DM branches (`PrivateDmOnly` / `SensitivePreferDm`) must
    /// never deliver to a caller-supplied chat id that is not the admin DM.
    /// With group/forum routing removed, a `stored_chat_id` carried in legacy
    /// queued state could be a negative group/supergroup id; honoring it would
    /// post owner-visible / approval content to a raw group. Only pass through
    /// a requested id when it is exactly `admin_chat_id`; otherwise fall back
    /// to the admin DM (`None` → `dm_route` uses `admin_chat_id`).
    fn admin_dm_only(&self, requested_chat_id: Option<i64>) -> Option<i64> {
        requested_chat_id.filter(|id| *id == self.admin_chat_id)
    }

    fn dm_route(
        &self,
        requested_chat_id: Option<i64>,
        reason: NotificationRouteReason,
    ) -> NotificationRoute {
        NotificationRoute {
            chat_id: requested_chat_id.unwrap_or(self.admin_chat_id),
            thread_id: None,
            topic_kind: None,
            reason,
        }
    }
}

pub fn route_telegram_notification(
    admin_chat_id: i64,
    request: NotificationRouteRequest,
) -> NotificationRoute {
    let route = TelegramNotificationRouter::from_topic_store(admin_chat_id).route(request);
    // debug: this fires for every notification decision (five lines every
    // 30 s in production — 71% of the journal), drowning real warnings.
    tracing::debug!(
        notification_class = ?request.notification_class,
        privacy_level = ?request.privacy_level,
        origin_chat_id = request.origin_chat_id,
        stored_chat_id = request.stored_chat_id,
        resolved_chat_id = route.chat_id,
        resolved_thread_id = route.thread_id,
        topic_kind = ?route.topic_kind,
        reason = ?route.reason,
        "[Telegram Notification Route]"
    );
    route
}

pub fn proactive_route_request(
    notification_class: NotificationClass,
    privacy_level: PrivacyLevel,
    stored_chat_id: Option<i64>,
) -> NotificationRouteRequest {
    NotificationRouteRequest {
        origin_chat_id: None,
        stored_chat_id,
        notification_class,
        privacy_level,
        user_initiated: false,
        allow_dm_continuation: false,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NotificationRouteRequest {
    pub origin_chat_id: Option<i64>,
    pub stored_chat_id: Option<i64>,
    pub notification_class: NotificationClass,
    pub privacy_level: PrivacyLevel,
    pub user_initiated: bool,
    pub allow_dm_continuation: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NotificationRoute {
    pub chat_id: i64,
    pub thread_id: Option<i32>,
    pub topic_kind: Option<TopicKind>,
    pub reason: NotificationRouteReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationUrgency {
    Immediate,
    SameDay,
    Digest,
    DashboardOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationChannel {
    TelegramDm,
    WebPush,
    IosPush,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationRenderRequest {
    pub notification_class: NotificationClass,
    pub sensitivity: crate::data_access_gate::DataSensitivity,
    pub urgency: NotificationUrgency,
    pub channel: NotificationChannel,
    pub title: String,
    pub safe_summary: String,
    pub sensitive_summary: Option<String>,
    pub allow_sensitive_detail: bool,
    pub detail_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationRenderResult {
    pub title: String,
    pub body: String,
    pub privacy: NotificationPrivacy,
    pub detail_url: Option<String>,
    pub redacted: bool,
}

pub fn privacy_for_sensitivity(
    sensitivity: crate::data_access_gate::DataSensitivity,
) -> NotificationPrivacy {
    match sensitivity {
        crate::data_access_gate::DataSensitivity::S0Public
        | crate::data_access_gate::DataSensitivity::S1LowPersonal => {
            NotificationPrivacy::PublicSafe
        }
        crate::data_access_gate::DataSensitivity::S2Personal => {
            NotificationPrivacy::PersonalMetadata
        }
        crate::data_access_gate::DataSensitivity::S3Sensitive => {
            NotificationPrivacy::SensitiveRedacted
        }
        crate::data_access_gate::DataSensitivity::S4HighlySensitive => {
            NotificationPrivacy::HighlySensitiveRedacted
        }
    }
}

pub fn render_notification_payload(request: NotificationRenderRequest) -> NotificationRenderResult {
    let privacy = privacy_for_sensitivity(request.sensitivity);
    let channel_forces_redaction = matches!(
        request.channel,
        NotificationChannel::WebPush | NotificationChannel::IosPush
    ) && matches!(
        privacy,
        NotificationPrivacy::SensitiveRedacted | NotificationPrivacy::HighlySensitiveRedacted
    );
    let allow_sensitive_detail = request.allow_sensitive_detail
        && !channel_forces_redaction
        && matches!(request.channel, NotificationChannel::TelegramDm);

    match privacy {
        NotificationPrivacy::PublicSafe => NotificationRenderResult {
            title: request.title,
            body: choose_nonempty(&request.safe_summary, "Notification available."),
            privacy,
            detail_url: request.detail_url,
            redacted: false,
        },
        NotificationPrivacy::PersonalMetadata => NotificationRenderResult {
            title: request.title,
            body: choose_nonempty(&request.safe_summary, "Personal update available."),
            privacy,
            detail_url: request.detail_url,
            redacted: false,
        },
        NotificationPrivacy::SensitiveRedacted if allow_sensitive_detail => {
            NotificationRenderResult {
                title: request.title,
                body: request
                    .sensitive_summary
                    .filter(|summary| !summary.trim().is_empty())
                    .unwrap_or_else(|| {
                        choose_nonempty(&request.safe_summary, "Sensitive update available.")
                    }),
                privacy,
                detail_url: request.detail_url,
                redacted: false,
            }
        }
        NotificationPrivacy::HighlySensitiveRedacted if allow_sensitive_detail => {
            NotificationRenderResult {
                title: request.title,
                body: request
                    .sensitive_summary
                    .filter(|summary| !summary.trim().is_empty())
                    .unwrap_or_else(|| {
                        choose_nonempty(&request.safe_summary, "Highly sensitive update available.")
                    }),
                privacy,
                detail_url: request.detail_url,
                redacted: false,
            }
        }
        NotificationPrivacy::SensitiveRedacted => NotificationRenderResult {
            title: "Sensitive item needs review".into(),
            body: "Open the Web Console and complete step-up MFA to view details.".into(),
            privacy,
            detail_url: request.detail_url,
            redacted: true,
        },
        NotificationPrivacy::HighlySensitiveRedacted => NotificationRenderResult {
            title: "Highly sensitive item needs review".into(),
            body: "Open the Web Console and complete step-up MFA to view details.".into(),
            privacy,
            detail_url: request.detail_url,
            redacted: true,
        },
    }
}

fn choose_nonempty(value: &str, fallback: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        fallback.into()
    } else {
        trimmed.into()
    }
}

pub fn topic_for_class(notification_class: NotificationClass) -> TopicKind {
    match notification_class {
        NotificationClass::DirectUserReply
        | NotificationClass::DailyBrief
        | NotificationClass::EveningReview
        | NotificationClass::OAuthDeepLinkReply => TopicKind::Main,
        NotificationClass::ApprovalRequest => TopicKind::Approval,
        NotificationClass::ProactiveAlert
        | NotificationClass::RoutineNotification
        | NotificationClass::PrivacySensitiveDmOnly => TopicKind::Notif,
        NotificationClass::SystemStatus | NotificationClass::StartupKeyboard => TopicKind::System,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data_access_gate::DataSensitivity;

    const ADMIN_DM: i64 = 123_456_789;
    const STORED_DM: i64 = 987_654_321;

    fn request(notification_class: NotificationClass) -> NotificationRouteRequest {
        NotificationRouteRequest {
            origin_chat_id: None,
            stored_chat_id: None,
            notification_class,
            // Non-DM / non-sensitive default so route() falls through to the
            // admin-DM fallback (group/forum routing has been removed).
            privacy_level: PrivacyLevel::SystemOnly,
            user_initiated: false,
            allow_dm_continuation: false,
        }
    }

    /// DM invariant: every proactive class that could previously route to a
    /// linked forum topic now resolves to the owner's admin DM with no forum
    /// thread. Exercises the real public entrypoint (`route_telegram_notification`
    /// → `from_topic_store`).
    #[test]
    fn proactive_sends_resolve_to_admin_dm_after_group_routing_removed() {
        for notification_class in [
            NotificationClass::ProactiveAlert,
            NotificationClass::RoutineNotification,
            NotificationClass::SystemStatus,
            NotificationClass::ApprovalRequest,
            NotificationClass::DailyBrief,
            NotificationClass::EveningReview,
            NotificationClass::StartupKeyboard,
        ] {
            for stored in [None, Some(STORED_DM)] {
                let route = route_telegram_notification(
                    ADMIN_DM,
                    proactive_route_request(notification_class, PrivacyLevel::SystemOnly, stored),
                );
                assert_eq!(
                    route.chat_id, ADMIN_DM,
                    "{notification_class:?} must resolve to admin DM"
                );
                assert_eq!(route.thread_id, None, "no forum thread after removal");
                assert_eq!(route.topic_kind, None, "no forum topic after removal");
            }
        }
    }

    #[test]
    fn direct_user_dm_reply_stays_in_dm() {
        let mut req = request(NotificationClass::DirectUserReply);
        req.origin_chat_id = Some(ADMIN_DM);
        req.user_initiated = true;
        req.allow_dm_continuation = true;

        let route = TelegramNotificationRouter::new(ADMIN_DM).route(req);

        assert_eq!(route.chat_id, ADMIN_DM);
        assert_eq!(route.thread_id, None);
        assert_eq!(route.topic_kind, None);
        assert_eq!(route.reason, NotificationRouteReason::DirectDmContinuation);
    }

    #[test]
    fn oauth_deeplink_reply_is_explicit_dm_exception() {
        let mut req = request(NotificationClass::OAuthDeepLinkReply);
        req.origin_chat_id = Some(ADMIN_DM);

        let route = TelegramNotificationRouter::new(ADMIN_DM).route(req);

        assert_eq!(route.chat_id, ADMIN_DM);
        assert_eq!(route.thread_id, None);
        assert_eq!(
            route.reason,
            NotificationRouteReason::OAuthDeepLinkException
        );
    }

    #[test]
    fn privacy_sensitive_dm_only_never_routes_to_group() {
        let mut req = request(NotificationClass::PrivacySensitiveDmOnly);
        req.privacy_level = PrivacyLevel::PrivateDmOnly;

        let route = TelegramNotificationRouter::new(ADMIN_DM).route(req);

        assert_eq!(route.chat_id, ADMIN_DM);
        assert_eq!(route.thread_id, None);
        assert_eq!(route.reason, NotificationRouteReason::PrivacyDmOnly);
    }

    /// Regression: a proactive DM-privacy send must clamp a legacy non-admin
    /// (e.g. group/supergroup) `stored_chat_id` to the owner's admin DM — it
    /// must never deliver owner/approval content to a raw group id.
    #[test]
    fn dm_privacy_branches_clamp_non_admin_stored_chat_to_admin() {
        const LEGACY_GROUP: i64 = -100_222_333_444;
        for privacy_level in [PrivacyLevel::PrivateDmOnly, PrivacyLevel::SensitivePreferDm] {
            let route = route_telegram_notification(
                ADMIN_DM,
                proactive_route_request(
                    NotificationClass::ProactiveAlert,
                    privacy_level,
                    Some(LEGACY_GROUP),
                ),
            );
            assert_eq!(
                route.chat_id, ADMIN_DM,
                "{privacy_level:?} must clamp a non-admin stored chat to the admin DM"
            );
            assert_eq!(route.thread_id, None);
            assert_eq!(route.topic_kind, None);
        }

        // The class-driven PrivacySensitiveDmOnly path clamps too.
        let route = route_telegram_notification(
            ADMIN_DM,
            proactive_route_request(
                NotificationClass::PrivacySensitiveDmOnly,
                PrivacyLevel::PrivateDmOnly,
                Some(LEGACY_GROUP),
            ),
        );
        assert_eq!(route.chat_id, ADMIN_DM);
        assert_eq!(route.reason, NotificationRouteReason::PrivacyDmOnly);
    }

    #[test]
    fn no_linked_group_falls_back_to_admin_dm() {
        let route = TelegramNotificationRouter::new(ADMIN_DM)
            .route(request(NotificationClass::ProactiveAlert));

        assert_eq!(route.chat_id, ADMIN_DM);
        assert_eq!(route.thread_id, None);
        assert_eq!(
            route.reason,
            NotificationRouteReason::NoLinkedGroupFallbackToDm
        );
    }

    #[test]
    fn proactive_route_request_never_marks_user_initiated() {
        let req = proactive_route_request(
            NotificationClass::SystemStatus,
            PrivacyLevel::SystemOnly,
            Some(STORED_DM),
        );

        assert_eq!(req.origin_chat_id, None);
        assert_eq!(req.stored_chat_id, Some(STORED_DM));
        assert_eq!(req.notification_class, NotificationClass::SystemStatus);
        assert_eq!(req.privacy_level, PrivacyLevel::SystemOnly);
        assert!(!req.user_initiated);
        assert!(!req.allow_dm_continuation);
    }

    fn render_request(
        sensitivity: DataSensitivity,
        channel: NotificationChannel,
    ) -> NotificationRenderRequest {
        NotificationRenderRequest {
            notification_class: NotificationClass::RoutineNotification,
            sensitivity,
            urgency: NotificationUrgency::SameDay,
            channel,
            title: "Email from Alice about medical report".into(),
            safe_summary: "A sensitive email requires review.".into(),
            sensitive_summary: Some("Alice says the medical report is attached.".into()),
            allow_sensitive_detail: false,
            detail_url: Some("/ui/?tab=mail".into()),
        }
    }

    #[test]
    fn s3_private_body_notification_is_redacted_without_consent() {
        let result = render_notification_payload(render_request(
            DataSensitivity::S3Sensitive,
            NotificationChannel::TelegramDm,
        ));

        assert!(result.redacted);
        assert_eq!(result.privacy, NotificationPrivacy::SensitiveRedacted);
        assert_eq!(result.title, "Sensitive item needs review");
        assert!(!result.body.contains("Alice"));
        assert!(!result.body.contains("medical report"));
        assert_eq!(result.detail_url.as_deref(), Some("/ui/?tab=mail"));
    }

    #[test]
    fn s4_image_notification_omits_description_even_in_dm_without_consent() {
        let mut request = render_request(
            DataSensitivity::S4HighlySensitive,
            NotificationChannel::TelegramDm,
        );
        request.title = "Portrait attachment from school".into();
        request.safe_summary = "An image attachment needs authorization before analysis.".into();
        request.sensitive_summary = Some("The image appears to show a child at school.".into());

        let result = render_notification_payload(request);

        assert!(result.redacted);
        assert_eq!(result.privacy, NotificationPrivacy::HighlySensitiveRedacted);
        assert!(!result.title.contains("school"));
        assert!(!result.body.contains("child"));
        assert!(!result.body.contains("image appears"));
    }

    #[test]
    fn non_dm_channel_redacts_sensitive_detail_even_when_dm_would_allow_it() {
        // Telegram now only ever sends to the owner's DM, but the render layer
        // still force-redacts sensitive detail on non-DM channels (WebPush /
        // iOS push). DM with consent may show detail; a push channel must not.
        let mut dm_request = render_request(
            DataSensitivity::S3Sensitive,
            NotificationChannel::TelegramDm,
        );
        dm_request.allow_sensitive_detail = true;
        let dm = render_notification_payload(dm_request);

        let mut push_request =
            render_request(DataSensitivity::S3Sensitive, NotificationChannel::WebPush);
        push_request.allow_sensitive_detail = true;
        let push = render_notification_payload(push_request);

        assert!(!dm.redacted);
        assert!(dm.body.contains("medical report"));
        assert!(push.redacted);
        assert!(!push.body.contains("medical report"));
        assert_eq!(push.title, "Sensitive item needs review");
    }

    #[test]
    fn notification_builder_omits_sensitive_summary_without_consent() {
        let request = render_request(
            DataSensitivity::S3Sensitive,
            NotificationChannel::TelegramDm,
        );

        let result = render_notification_payload(request);

        assert!(result.redacted);
        assert!(!result.body.contains("Alice"));
        assert!(!result.body.contains("attached"));
    }

    #[test]
    fn personal_metadata_uses_safe_summary_not_sensitive_summary() {
        let result = render_notification_payload(render_request(
            DataSensitivity::S2Personal,
            NotificationChannel::TelegramDm,
        ));

        assert!(!result.redacted);
        assert_eq!(result.privacy, NotificationPrivacy::PersonalMetadata);
        assert_eq!(result.body, "A sensitive email requires review.");
        assert!(!result.body.contains("medical report"));
    }
}
