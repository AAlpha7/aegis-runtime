//! Read-only profile projection rebuilt from Heirloom profile events.
//!
//! This projector is intentionally not wired into production reads. It is the
//! first P2 shadow-read building block for comparing Heirloom-derived profile
//! state against the legacy `user_profile` table.

use std::collections::{BTreeMap, BTreeSet};

use crate::heirloom::EventEnvelope;
use serde::{Deserialize, Serialize};

pub const PROFILE_SLOT_UPSERTED: &str = "profile:slot_upserted";

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ProfileProjection {
    pub slots: Vec<ProfileSlotProjection>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ProfileSlotProjection {
    pub category: String,
    pub key: String,
    pub value: String,
    pub confidence: f64,
    pub source: String,
    pub source_event_ids: Vec<String>,
    pub last_updated_hlc_phys: u64,
    pub last_updated_hlc_log: u32,
    pub has_conflict: bool,
    pub conflict_values: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LegacyProfileSlot {
    pub category: String,
    pub key: String,
    pub value: String,
    pub confidence: f64,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ProfileDivergenceReport {
    pub matching_rows: usize,
    pub missing_in_projector: Vec<LegacyProfileSlot>,
    pub missing_in_legacy: Vec<ProfileSlotProjection>,
    pub value_mismatches: Vec<ProfileValueMismatch>,
    pub conflicts: Vec<ProfileSlotProjection>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ProfileValueMismatch {
    pub category: String,
    pub key: String,
    pub projector_value: String,
    pub legacy_value: String,
    pub projector_confidence: f64,
    pub legacy_confidence: f64,
    pub projector_source: String,
    pub legacy_source: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProfileShadowDebugConfig {
    pub enabled: bool,
    pub include_values: bool,
}

impl ProfileShadowDebugConfig {
    pub const fn disabled() -> Self {
        Self {
            enabled: false,
            include_values: false,
        }
    }

    pub const fn redacted_enabled() -> Self {
        Self {
            enabled: true,
            include_values: false,
        }
    }

    pub const fn values_enabled() -> Self {
        Self {
            enabled: true,
            include_values: true,
        }
    }
}

impl Default for ProfileShadowDebugConfig {
    fn default() -> Self {
        Self::disabled()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ProfileShadowDebugSurface {
    pub projector_name: &'static str,
    pub projector_version: u32,
    pub slot_count: usize,
    pub legacy_slot_count: usize,
    pub divergence: ProfileShadowDivergenceSummary,
    pub slots: Vec<ProfileShadowSlotDebug>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProfileShadowDivergenceSummary {
    pub matching_rows: usize,
    pub missing_in_projector: usize,
    pub missing_in_legacy: usize,
    pub value_mismatches: usize,
    pub conflicts: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ProfileShadowSlotDebug {
    pub category: String,
    pub key: String,
    pub value: Option<String>,
    pub confidence: f64,
    pub source: String,
    pub source_event_ids: Vec<String>,
    pub last_updated_hlc_phys: u64,
    pub last_updated_hlc_log: u32,
    pub has_conflict: bool,
    pub conflict_values: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum ProfileProjectionError {
    #[error("profile projector payload decode failed for event {event_id}: {reason}")]
    PayloadDecode { event_id: String, reason: String },
    #[error("profile projector missing required field {field} for event {event_id}")]
    MissingField {
        event_id: String,
        field: &'static str,
    },
}

#[derive(Debug, Default, Clone, Copy)]
pub struct ProfileReadProjector;

#[derive(Debug, Clone)]
struct SlotAccumulator {
    category: String,
    key: String,
    value: String,
    confidence: f64,
    source: String,
    last_updated_hlc_phys: u64,
    last_updated_hlc_log: u32,
    source_event_ids: Vec<String>,
    values_seen: BTreeSet<String>,
}

#[derive(Debug, Deserialize)]
struct ProfileSlotPayload {
    category: Option<String>,
    key: Option<String>,
    value: Option<String>,
    confidence: Option<f64>,
    source: Option<String>,
}

impl ProfileReadProjector {
    pub const fn new() -> Self {
        Self
    }

    pub fn name(&self) -> &'static str {
        "ProfileReadProjector"
    }

    pub fn version(&self) -> u32 {
        1
    }

    pub fn project(
        &self,
        events: &[EventEnvelope],
    ) -> Result<ProfileProjection, ProfileProjectionError> {
        let mut relevant: Vec<&EventEnvelope> = events
            .iter()
            .filter(|event| event.event_type.as_str() == PROFILE_SLOT_UPSERTED)
            .collect();
        relevant.sort_by_key(|event| {
            (
                event.hlc_ts.phys,
                event.hlc_ts.log,
                event.observed_at,
                event.event_id,
            )
        });

        let mut slots: BTreeMap<(String, String), SlotAccumulator> = BTreeMap::new();
        for event in relevant {
            let event_id = event.event_id.to_string();
            let payload: ProfileSlotPayload =
                serde_json::from_slice(&event.payload).map_err(|e| {
                    ProfileProjectionError::PayloadDecode {
                        event_id: event_id.clone(),
                        reason: e.to_string(),
                    }
                })?;
            let category = required(payload.category, &event_id, "category")?;
            let key = required(payload.key, &event_id, "key")?;
            let value = required(payload.value, &event_id, "value")?;
            let confidence = payload.confidence.unwrap_or(0.0);
            let source = payload.source.unwrap_or_else(|| "unknown".to_string());
            let slot_key = (category.clone(), key.clone());
            let entry = slots.entry(slot_key).or_insert_with(|| SlotAccumulator {
                category,
                key,
                value: value.clone(),
                confidence,
                source: source.clone(),
                last_updated_hlc_phys: event.hlc_ts.phys,
                last_updated_hlc_log: event.hlc_ts.log,
                source_event_ids: Vec::new(),
                values_seen: BTreeSet::new(),
            });
            entry.source_event_ids.push(event_id);
            entry.values_seen.insert(value.clone());
            entry.value = value;
            entry.confidence = confidence;
            entry.source = source;
            entry.last_updated_hlc_phys = event.hlc_ts.phys;
            entry.last_updated_hlc_log = event.hlc_ts.log;
        }

        Ok(ProfileProjection {
            slots: slots
                .into_values()
                .map(|slot| {
                    let conflict_values: Vec<String> = slot.values_seen.into_iter().collect();
                    ProfileSlotProjection {
                        category: slot.category,
                        key: slot.key,
                        value: slot.value,
                        confidence: slot.confidence,
                        source: slot.source,
                        source_event_ids: slot.source_event_ids,
                        last_updated_hlc_phys: slot.last_updated_hlc_phys,
                        last_updated_hlc_log: slot.last_updated_hlc_log,
                        has_conflict: conflict_values.len() > 1,
                        conflict_values,
                    }
                })
                .collect(),
        })
    }

    pub fn compare_with_legacy(
        &self,
        projection: &ProfileProjection,
        legacy_slots: &[LegacyProfileSlot],
    ) -> ProfileDivergenceReport {
        let projected_by_key: BTreeMap<(String, String), &ProfileSlotProjection> = projection
            .slots
            .iter()
            .map(|slot| ((slot.category.clone(), slot.key.clone()), slot))
            .collect();
        let legacy_by_key: BTreeMap<(String, String), &LegacyProfileSlot> = legacy_slots
            .iter()
            .map(|slot| ((slot.category.clone(), slot.key.clone()), slot))
            .collect();

        let mut matching_rows = 0;
        let mut missing_in_projector = Vec::new();
        let mut missing_in_legacy = Vec::new();
        let mut value_mismatches = Vec::new();
        let mut conflicts = Vec::new();

        for (slot_key, legacy) in &legacy_by_key {
            match projected_by_key.get(slot_key) {
                Some(projected) if projected.value == legacy.value => matching_rows += 1,
                Some(projected) => value_mismatches.push(ProfileValueMismatch {
                    category: legacy.category.clone(),
                    key: legacy.key.clone(),
                    projector_value: projected.value.clone(),
                    legacy_value: legacy.value.clone(),
                    projector_confidence: projected.confidence,
                    legacy_confidence: legacy.confidence,
                    projector_source: projected.source.clone(),
                    legacy_source: legacy.source.clone(),
                }),
                None => missing_in_projector.push((*legacy).clone()),
            }
        }

        for (slot_key, projected) in &projected_by_key {
            if !legacy_by_key.contains_key(slot_key) {
                missing_in_legacy.push((*projected).clone());
            }
            if projected.has_conflict {
                conflicts.push((*projected).clone());
            }
        }

        ProfileDivergenceReport {
            matching_rows,
            missing_in_projector,
            missing_in_legacy,
            value_mismatches,
            conflicts,
        }
    }

    pub fn shadow_debug_surface(
        &self,
        config: ProfileShadowDebugConfig,
        projection: &ProfileProjection,
        legacy_slots: &[LegacyProfileSlot],
    ) -> Option<ProfileShadowDebugSurface> {
        if !config.enabled {
            return None;
        }

        let divergence = self.compare_with_legacy(projection, legacy_slots);
        Some(ProfileShadowDebugSurface {
            projector_name: self.name(),
            projector_version: self.version(),
            slot_count: projection.slots.len(),
            legacy_slot_count: legacy_slots.len(),
            divergence: ProfileShadowDivergenceSummary {
                matching_rows: divergence.matching_rows,
                missing_in_projector: divergence.missing_in_projector.len(),
                missing_in_legacy: divergence.missing_in_legacy.len(),
                value_mismatches: divergence.value_mismatches.len(),
                conflicts: divergence.conflicts.len(),
            },
            slots: projection
                .slots
                .iter()
                .map(|slot| ProfileShadowSlotDebug {
                    category: slot.category.clone(),
                    key: slot.key.clone(),
                    value: config.include_values.then(|| slot.value.clone()),
                    confidence: slot.confidence,
                    source: slot.source.clone(),
                    source_event_ids: slot.source_event_ids.clone(),
                    last_updated_hlc_phys: slot.last_updated_hlc_phys,
                    last_updated_hlc_log: slot.last_updated_hlc_log,
                    has_conflict: slot.has_conflict,
                    conflict_values: if config.include_values {
                        slot.conflict_values.clone()
                    } else {
                        Vec::new()
                    },
                })
                .collect(),
        })
    }
}

fn required(
    value: Option<String>,
    event_id: &str,
    field: &'static str,
) -> Result<String, ProfileProjectionError> {
    match value {
        Some(value) if !value.is_empty() => Ok(value),
        _ => Err(ProfileProjectionError::MissingField {
            event_id: event_id.to_string(),
            field,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::heirloom::enums::{EventTypeTag, PayloadEncoding, SensitivityTier};
    use crate::heirloom::hlc::HlcTimestamp;
    use crate::heirloom::types::{Did, LineageId};
    use chrono::{DateTime, Utc};
    use uuid::Uuid;

    fn profile_event(
        id_suffix: u8,
        category: &str,
        key: &str,
        value: &str,
        confidence: f64,
        hlc_phys: u64,
    ) -> EventEnvelope {
        let mut id_bytes = [0u8; 16];
        id_bytes[15] = id_suffix;
        EventEnvelope {
            event_id: Uuid::from_bytes(id_bytes),
            schema_version: 1,
            event_type: EventTypeTag::new(PROFILE_SLOT_UPSERTED),
            prev_hash: [0; 32],
            hlc_ts: HlcTimestamp {
                phys: hlc_phys,
                log: 0,
            },
            observed_at: DateTime::parse_from_rfc3339("2026-04-29T00:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
            l0_hash: [0; 32],
            l1_hash: [0; 32],
            l2_manifest_hash: [0; 32],
            lineage_id: LineageId::new("lineage:test"),
            author_did: Did::new("did:aegis:test"),
            signatures: vec![],
            sensitivity_tier: SensitivityTier::Sensitive,
            bundle_ids: vec![],
            payload_encoding: PayloadEncoding::Plaintext,
            payload: serde_json::json!({
                "category": category,
                "key": key,
                "value": value,
                "confidence": confidence,
                "source": "fixture",
            })
            .to_string()
            .into_bytes(),
        }
    }

    fn unrelated_event() -> EventEnvelope {
        let mut event = profile_event(99, "identity", "name", "ignored", 0.1, 99);
        event.event_type = EventTypeTag::new("observation:user_stated");
        event
    }

    fn legacy_slot(category: &str, key: &str, value: &str) -> LegacyProfileSlot {
        LegacyProfileSlot {
            category: category.to_string(),
            key: key.to_string(),
            value: value.to_string(),
            confidence: 0.8,
            source: "legacy-fixture".to_string(),
        }
    }

    #[test]
    fn profile_projector_rebuild_is_deterministic() {
        let old = profile_event(1, "location", "residence_city", "Old", 0.4, 1);
        let new = profile_event(2, "location", "residence_city", "New", 0.9, 2);
        let ignored = unrelated_event();

        let forward = ProfileReadProjector::new()
            .project(&[old.clone(), ignored.clone(), new.clone()])
            .unwrap();
        let reversed = ProfileReadProjector::new()
            .project(&[new, ignored, old])
            .unwrap();

        assert_eq!(forward, reversed);
        assert_eq!(forward.slots.len(), 1);
        assert_eq!(forward.slots[0].value, "New");
        assert_eq!(forward.slots[0].confidence, 0.9);
    }

    #[test]
    fn profile_projector_retains_source_event_ids() {
        let first = profile_event(1, "identity", "name", "Alpha", 0.5, 1);
        let second = profile_event(2, "identity", "name", "Alpha", 0.8, 2);
        let first_id = first.event_id.to_string();
        let second_id = second.event_id.to_string();

        let projection = ProfileReadProjector::new()
            .project(&[second, first])
            .unwrap();

        assert_eq!(
            projection.slots[0].source_event_ids,
            vec![first_id, second_id]
        );
    }

    #[test]
    fn profile_projector_handles_conflicting_events() {
        let first = profile_event(1, "location", "residence_city", "Seattle", 0.5, 1);
        let second = profile_event(2, "location", "residence_city", "Example City", 0.9, 2);

        let projection = ProfileReadProjector::new()
            .project(&[first, second])
            .unwrap();
        let slot = &projection.slots[0];

        assert_eq!(slot.value, "Example City");
        assert!(slot.has_conflict);
        assert_eq!(
            slot.conflict_values,
            vec!["Example City".to_string(), "Seattle".to_string()]
        );
    }

    #[test]
    fn profile_divergence_report_counts_matching_rows() {
        let projection = ProfileReadProjector::new()
            .project(&[profile_event(1, "identity", "name", "Alpha", 0.9, 1)])
            .unwrap();

        let report = ProfileReadProjector::new()
            .compare_with_legacy(&projection, &[legacy_slot("identity", "name", "Alpha")]);

        assert_eq!(report.matching_rows, 1);
        assert!(report.missing_in_projector.is_empty());
        assert!(report.missing_in_legacy.is_empty());
        assert!(report.value_mismatches.is_empty());
        assert!(report.conflicts.is_empty());
    }

    #[test]
    fn profile_divergence_report_detects_missing_projector_rows() {
        let projection = ProfileReadProjector::new().project(&[]).unwrap();

        let report = ProfileReadProjector::new().compare_with_legacy(
            &projection,
            &[legacy_slot("location", "residence_city", "Seattle")],
        );

        assert_eq!(report.matching_rows, 0);
        assert_eq!(report.missing_in_projector.len(), 1);
        assert_eq!(report.missing_in_projector[0].key, "residence_city");
    }

    #[test]
    fn profile_divergence_report_detects_missing_legacy_rows() {
        let projection = ProfileReadProjector::new()
            .project(&[profile_event(
                1,
                "location",
                "residence_city",
                "Seattle",
                0.9,
                1,
            )])
            .unwrap();

        let report = ProfileReadProjector::new().compare_with_legacy(&projection, &[]);

        assert_eq!(report.matching_rows, 0);
        assert_eq!(report.missing_in_legacy.len(), 1);
        assert_eq!(report.missing_in_legacy[0].key, "residence_city");
    }

    #[test]
    fn profile_divergence_report_detects_value_mismatches() {
        let projection = ProfileReadProjector::new()
            .project(&[profile_event(
                1,
                "location",
                "residence_city",
                "Example City",
                0.9,
                1,
            )])
            .unwrap();

        let report = ProfileReadProjector::new().compare_with_legacy(
            &projection,
            &[legacy_slot("location", "residence_city", "Seattle")],
        );

        assert_eq!(report.value_mismatches.len(), 1);
        assert_eq!(report.value_mismatches[0].projector_value, "Example City");
        assert_eq!(report.value_mismatches[0].legacy_value, "Seattle");
    }

    #[test]
    fn profile_divergence_report_tracks_projector_conflicts() {
        let projection = ProfileReadProjector::new()
            .project(&[
                profile_event(1, "location", "residence_city", "Seattle", 0.5, 1),
                profile_event(2, "location", "residence_city", "Example City", 0.9, 2),
            ])
            .unwrap();

        let report = ProfileReadProjector::new().compare_with_legacy(
            &projection,
            &[legacy_slot("location", "residence_city", "Example City")],
        );

        assert_eq!(report.matching_rows, 1);
        assert_eq!(report.conflicts.len(), 1);
        assert_eq!(
            report.conflicts[0].conflict_values,
            vec!["Example City".to_string(), "Seattle".to_string()]
        );
    }

    #[test]
    fn profile_shadow_debug_surface_is_disabled_by_default() {
        let projection = ProfileReadProjector::new()
            .project(&[profile_event(1, "identity", "name", "Alpha", 0.9, 1)])
            .unwrap();

        let surface = ProfileReadProjector::new().shadow_debug_surface(
            ProfileShadowDebugConfig::default(),
            &projection,
            &[legacy_slot("identity", "name", "Alpha")],
        );

        assert!(surface.is_none());
    }

    #[test]
    fn profile_shadow_debug_surface_redacts_values() {
        let projection = ProfileReadProjector::new()
            .project(&[
                profile_event(1, "location", "residence_city", "Seattle", 0.5, 1),
                profile_event(2, "location", "residence_city", "Example City", 0.9, 2),
            ])
            .unwrap();

        let surface = ProfileReadProjector::new()
            .shadow_debug_surface(
                ProfileShadowDebugConfig::redacted_enabled(),
                &projection,
                &[legacy_slot("location", "residence_city", "Example City")],
            )
            .unwrap();

        assert_eq!(surface.projector_name, "ProfileReadProjector");
        assert_eq!(surface.projector_version, 1);
        assert_eq!(surface.slot_count, 1);
        assert_eq!(surface.legacy_slot_count, 1);
        assert_eq!(surface.divergence.matching_rows, 1);
        assert_eq!(surface.divergence.conflicts, 1);
        assert_eq!(surface.slots[0].value, None);
        assert!(surface.slots[0].conflict_values.is_empty());
    }

    #[test]
    fn profile_shadow_debug_surface_can_include_values_when_enabled() {
        let projection = ProfileReadProjector::new()
            .project(&[
                profile_event(1, "location", "residence_city", "Seattle", 0.5, 1),
                profile_event(2, "location", "residence_city", "Example City", 0.9, 2),
            ])
            .unwrap();

        let surface = ProfileReadProjector::new()
            .shadow_debug_surface(
                ProfileShadowDebugConfig::values_enabled(),
                &projection,
                &[legacy_slot("location", "residence_city", "Seattle")],
            )
            .unwrap();

        assert_eq!(surface.divergence.value_mismatches, 1);
        assert_eq!(surface.slots[0].value.as_deref(), Some("Example City"));
        assert_eq!(
            surface.slots[0].conflict_values,
            vec!["Example City".to_string(), "Seattle".to_string()]
        );
    }
}
