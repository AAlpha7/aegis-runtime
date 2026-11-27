//! Projector trait + event-type pattern matcher + error type.
//!
//! Phase C1 (W9): defines the seam between event log append and projection
//! updates. Spec §3 Q3.5.
//!
//! **Sync vs async:** spec shows `async fn apply`; rusqlite is sync across
//! this repo. Phase C1 uses a sync trait. If P2+ migrates to sqlx, the trait
//! signature can grow `async`.

use crate::heirloom::EventEnvelope;
use std::sync::Arc;

/// Pattern that matches an event_type string. Exact match, domain wildcard,
/// or universal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventTypePattern {
    /// Match the full `"domain:action"` string exactly.
    Exact(String),
    /// Match any action under the given domain: e.g. `"schema:*"` matches
    /// `"schema:type_defined"` and `"schema:signature_scheme_registered"`.
    DomainWildcard(String),
    /// Match every event. Use sparingly (audit / metrics only).
    Any,
}

impl EventTypePattern {
    /// Return true if `event_type` matches this pattern.
    pub fn matches(&self, event_type: &str) -> bool {
        match self {
            EventTypePattern::Exact(s) => s == event_type,
            EventTypePattern::DomainWildcard(domain) => event_type
                .split_once(':')
                .map(|(d, _)| d == domain)
                .unwrap_or(false),
            EventTypePattern::Any => true,
        }
    }

    /// Convenience: make an exact-match pattern.
    pub fn exact(s: impl Into<String>) -> Self {
        Self::Exact(s.into())
    }

    /// Convenience: make a domain-wildcard pattern.
    pub fn domain(s: impl Into<String>) -> Self {
        Self::DomainWildcard(s.into())
    }
}

/// Projector trait — maps events into projection-table updates.
///
/// Every projector owns exactly one projection table. `apply` is called
/// inside the unified-write transaction (Phase C2 W10); if it returns Err,
/// the whole transaction — including the event log append — rolls back.
pub trait Projector: Send + Sync {
    /// Event-type patterns this projector reacts to. The dispatcher calls
    /// `apply()` only for events whose `event_type` matches one of these.
    fn for_event_types(&self) -> Vec<EventTypePattern>;

    /// Apply this event to the projector's table inside the open transaction.
    fn apply(
        &self,
        tx: &rusqlite::Transaction<'_>,
        envelope: &EventEnvelope,
    ) -> Result<(), ProjectorError>;

    /// Revert the effect of a previously-applied event. P4 Phase D rollback
    /// tooling only; Phase C1 impls return `RevertNotImplemented`.
    fn revert(
        &self,
        _tx: &rusqlite::Transaction<'_>,
        _envelope: &EventEnvelope,
    ) -> Result<(), ProjectorError> {
        Err(ProjectorError::RevertNotImplemented)
    }

    /// Tag used in error messages + logs. Convention: struct name in snake_case.
    fn projector_name(&self) -> &'static str;
}

/// Errors from projector operations.
#[derive(Debug, thiserror::Error)]
pub enum ProjectorError {
    #[error("projector {0}: sqlite error: {1}")]
    Sqlite(String, #[source] rusqlite::Error),
    #[error("projector {projector}: payload decode failed: {reason}")]
    PayloadDecode { projector: String, reason: String },
    #[error("projector {projector}: invariant violated: {reason}")]
    Invariant { projector: String, reason: String },
    #[error("revert is not implemented for this projector (Phase C1 scope)")]
    RevertNotImplemented,
}

// ── ProjectorDispatcher ───────────────────────────────────────────────────

/// Holds all registered projectors and routes events by `event_type`
/// pattern match. Phase C2 HeirloomWriter holds one instance.
pub struct ProjectorDispatcher {
    projectors: Vec<Arc<dyn Projector>>,
}

impl ProjectorDispatcher {
    pub fn new() -> Self {
        Self {
            projectors: Vec::new(),
        }
    }

    pub fn register(&mut self, projector: Arc<dyn Projector>) {
        self.projectors.push(projector);
    }

    /// Return every projector whose pattern list matches `event_type`.
    /// Order follows registration order (stable).
    pub fn matching(&self, event_type: &str) -> Vec<Arc<dyn Projector>> {
        self.projectors
            .iter()
            .filter(|p| {
                p.for_event_types()
                    .iter()
                    .any(|pat| pat.matches(event_type))
            })
            .cloned()
            .collect()
    }

    /// Apply `envelope` to every matching projector, stopping at the first
    /// error (the caller rolls back the transaction).
    pub fn dispatch(
        &self,
        tx: &rusqlite::Transaction<'_>,
        envelope: &EventEnvelope,
    ) -> Result<usize, ProjectorError> {
        let matching = self.matching(envelope.event_type.as_str());
        let count = matching.len();
        for p in matching {
            p.apply(tx, envelope)?;
        }
        Ok(count)
    }

    /// Projector count — audit / debug.
    pub fn len(&self) -> usize {
        self.projectors.len()
    }

    pub fn is_empty(&self) -> bool {
        self.projectors.is_empty()
    }
}

impl Default for ProjectorDispatcher {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for ProjectorDispatcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let names: Vec<&str> = self.projectors.iter().map(|p| p.projector_name()).collect();
        f.debug_struct("ProjectorDispatcher")
            .field("projectors", &names)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t9_pattern_exact_matches_only_exact_string() {
        let p = EventTypePattern::exact("schema:type_defined");
        assert!(p.matches("schema:type_defined"));
        assert!(!p.matches("schema:type_updated"));
        assert!(!p.matches("ai:tool_invoked"));
    }

    #[test]
    fn t9_pattern_domain_wildcard_matches_all_actions_in_domain() {
        let p = EventTypePattern::domain("schema");
        assert!(p.matches("schema:type_defined"));
        assert!(p.matches("schema:signature_scheme_registered"));
        assert!(p.matches("schema:signature_scheme_deprecated"));
        assert!(!p.matches("ai:tool_invoked"));
    }

    #[test]
    fn t9_pattern_domain_wildcard_rejects_event_type_without_colon() {
        let p = EventTypePattern::domain("schema");
        assert!(!p.matches("schema"));
        assert!(!p.matches(""));
    }

    #[test]
    fn t9_pattern_any_matches_everything() {
        let p = EventTypePattern::Any;
        assert!(p.matches("schema:type_defined"));
        assert!(p.matches(""));
        assert!(p.matches("arbitrary_garbage"));
    }

    #[test]
    fn t9_projector_error_display_includes_projector_name() {
        let e = ProjectorError::Invariant {
            projector: "BundleIndexProjector".to_string(),
            reason: "empty bundle_ids".to_string(),
        };
        let s = format!("{e}");
        assert!(s.contains("BundleIndexProjector"));
        assert!(s.contains("empty bundle_ids"));
    }

    #[test]
    fn t9_projector_trait_object_safety() {
        // If this compiles, the trait is dyn-compatible. No runtime assertion.
        fn _assert_object_safe(_p: &dyn Projector) {}
    }

    // ── Dispatcher tests ─────────────────────────────────────────────────

    use std::sync::atomic::{AtomicUsize, Ordering};

    struct CountingProjector {
        name: &'static str,
        patterns: Vec<EventTypePattern>,
        apply_count: Arc<AtomicUsize>,
    }

    impl Projector for CountingProjector {
        fn for_event_types(&self) -> Vec<EventTypePattern> {
            self.patterns.clone()
        }
        fn apply(
            &self,
            _tx: &rusqlite::Transaction<'_>,
            _envelope: &EventEnvelope,
        ) -> Result<(), ProjectorError> {
            self.apply_count.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }
        fn projector_name(&self) -> &'static str {
            self.name
        }
    }

    struct FailingProjector;
    impl Projector for FailingProjector {
        fn for_event_types(&self) -> Vec<EventTypePattern> {
            vec![EventTypePattern::Any]
        }
        fn apply(
            &self,
            _tx: &rusqlite::Transaction<'_>,
            _envelope: &EventEnvelope,
        ) -> Result<(), ProjectorError> {
            Err(ProjectorError::Invariant {
                projector: "FailingProjector".to_string(),
                reason: "test-induced failure".to_string(),
            })
        }
        fn projector_name(&self) -> &'static str {
            "FailingProjector"
        }
    }

    fn sample_envelope(event_type: &str) -> EventEnvelope {
        use crate::heirloom::enums::{EventTypeTag, PayloadEncoding, SensitivityTier};
        use crate::heirloom::hlc::HlcTimestamp;
        use crate::heirloom::types::{BundleId, Did, LineageId};
        use chrono::{DateTime, Utc};
        use uuid::Uuid;
        EventEnvelope {
            event_id: Uuid::parse_str("01925b3e-0000-7000-8000-000000000001").unwrap(),
            schema_version: 1,
            event_type: EventTypeTag::new(event_type),
            prev_hash: [0; 32],
            hlc_ts: HlcTimestamp { phys: 1, log: 0 },
            observed_at: DateTime::parse_from_rfc3339("2026-04-17T00:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
            l0_hash: [0; 32],
            l1_hash: [0; 32],
            l2_manifest_hash: [0; 32],
            lineage_id: LineageId::new("lineage:test"),
            author_did: Did::new("did:aegis:test"),
            signatures: vec![],
            sensitivity_tier: SensitivityTier::Ordinary,
            bundle_ids: vec![BundleId::new("bundle:general")],
            payload_encoding: PayloadEncoding::Plaintext,
            payload: b"{}".to_vec(),
        }
    }

    fn fresh_conn_with_schema() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::heirloom::schema::ensure_tables(&conn).unwrap();
        conn
    }

    #[test]
    fn t9_dispatcher_starts_empty() {
        let d = ProjectorDispatcher::new();
        assert!(d.is_empty());
        assert_eq!(d.len(), 0);
    }

    #[test]
    fn t9_dispatcher_register_grows_by_one() {
        let mut d = ProjectorDispatcher::new();
        let p = Arc::new(CountingProjector {
            name: "p1",
            patterns: vec![EventTypePattern::Any],
            apply_count: Arc::new(AtomicUsize::new(0)),
        });
        d.register(p);
        assert_eq!(d.len(), 1);
    }

    #[test]
    fn t9_dispatcher_matching_returns_only_patterns_that_match() {
        let mut d = ProjectorDispatcher::new();
        let c1 = Arc::new(AtomicUsize::new(0));
        let c2 = Arc::new(AtomicUsize::new(0));
        d.register(Arc::new(CountingProjector {
            name: "schema_only",
            patterns: vec![EventTypePattern::domain("schema")],
            apply_count: Arc::clone(&c1),
        }));
        d.register(Arc::new(CountingProjector {
            name: "ai_only",
            patterns: vec![EventTypePattern::domain("ai")],
            apply_count: Arc::clone(&c2),
        }));
        let matches = d.matching("schema:type_defined");
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].projector_name(), "schema_only");
    }

    #[test]
    fn t9_dispatcher_dispatch_calls_apply_on_every_matching_projector() {
        let mut d = ProjectorDispatcher::new();
        let c = Arc::new(AtomicUsize::new(0));
        d.register(Arc::new(CountingProjector {
            name: "any",
            patterns: vec![EventTypePattern::Any],
            apply_count: Arc::clone(&c),
        }));
        d.register(Arc::new(CountingProjector {
            name: "schema_only",
            patterns: vec![EventTypePattern::domain("schema")],
            apply_count: Arc::clone(&c),
        }));
        let conn = fresh_conn_with_schema();
        let tx = conn.unchecked_transaction().unwrap();
        let hit = d
            .dispatch(&tx, &sample_envelope("schema:type_defined"))
            .unwrap();
        tx.commit().unwrap();
        assert_eq!(hit, 2);
        assert_eq!(c.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn t9_dispatcher_dispatch_returns_zero_when_no_matches() {
        let mut d = ProjectorDispatcher::new();
        let c = Arc::new(AtomicUsize::new(0));
        d.register(Arc::new(CountingProjector {
            name: "schema_only",
            patterns: vec![EventTypePattern::domain("schema")],
            apply_count: Arc::clone(&c),
        }));
        let conn = fresh_conn_with_schema();
        let tx = conn.unchecked_transaction().unwrap();
        let hit = d
            .dispatch(&tx, &sample_envelope("ai:tool_invoked"))
            .unwrap();
        tx.commit().unwrap();
        assert_eq!(hit, 0);
        assert_eq!(c.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn t9_dispatcher_dispatch_propagates_projector_error() {
        let mut d = ProjectorDispatcher::new();
        d.register(Arc::new(FailingProjector));
        let conn = fresh_conn_with_schema();
        let tx = conn.unchecked_transaction().unwrap();
        let err = d
            .dispatch(&tx, &sample_envelope("anything:goes"))
            .unwrap_err();
        match err {
            ProjectorError::Invariant { projector, .. } => {
                assert_eq!(projector, "FailingProjector");
            }
            other => panic!("expected Invariant, got {other:?}"),
        }
    }

    #[test]
    fn t9_dispatcher_debug_includes_projector_names() {
        let mut d = ProjectorDispatcher::new();
        d.register(Arc::new(CountingProjector {
            name: "p_alpha",
            patterns: vec![EventTypePattern::Any],
            apply_count: Arc::new(AtomicUsize::new(0)),
        }));
        d.register(Arc::new(FailingProjector));
        let s = format!("{d:?}");
        assert!(s.contains("p_alpha"));
        assert!(s.contains("FailingProjector"));
    }
}
