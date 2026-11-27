//! Read-only task/reminder projection rebuilt from Heirloom task events.
//!
//! This projector is intentionally not wired into production task reads. It is
//! a P2 shadow-read building block for comparing future Heirloom-derived task
//! state against the legacy `tasks` table.

use std::collections::{BTreeMap, BTreeSet};

use crate::heirloom::EventEnvelope;
use serde::{Deserialize, Serialize};

pub const TASK_UPSERTED: &str = "task:upserted";
pub const TASK_STATUS_CHANGED: &str = "task:status_changed";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TaskReminderProjection {
    pub tasks: Vec<TaskReminderItemProjection>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TaskReminderItemProjection {
    pub task_id: String,
    pub title: String,
    pub status: String,
    pub bucket: TaskReminderBucket,
    pub priority: String,
    pub source: String,
    pub due_at_unix: Option<i64>,
    pub trigger_at_unix: Option<i64>,
    pub waiting_for: Option<String>,
    pub source_event_ids: Vec<String>,
    pub last_updated_hlc_phys: u64,
    pub last_updated_hlc_log: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskReminderBucket {
    Active,
    DueSoon,
    Completed,
    Stale,
    WaitingFor,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LegacyTaskReminderRow {
    pub task_id: String,
    pub title: String,
    pub status: String,
    pub due_at_unix: Option<i64>,
    pub trigger_at_unix: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TaskReminderDivergenceReport {
    pub matching_rows: usize,
    pub missing_in_projector: Vec<LegacyTaskReminderRow>,
    pub missing_in_legacy: Vec<TaskReminderItemProjection>,
    pub status_mismatches: Vec<TaskReminderStatusMismatch>,
    pub timing_mismatches: Vec<TaskReminderTimingMismatch>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TaskReminderStatusMismatch {
    pub task_id: String,
    pub projector_status: String,
    pub legacy_status: String,
    pub projector_bucket: TaskReminderBucket,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TaskReminderTimingMismatch {
    pub task_id: String,
    pub projector_due_at_unix: Option<i64>,
    pub legacy_due_at_unix: Option<i64>,
    pub projector_trigger_at_unix: Option<i64>,
    pub legacy_trigger_at_unix: Option<i64>,
}

#[derive(Debug, thiserror::Error)]
pub enum TaskReminderProjectionError {
    #[error("task/reminder projector payload decode failed for event {event_id}: {reason}")]
    PayloadDecode { event_id: String, reason: String },
    #[error("task/reminder projector missing required field {field} for event {event_id}")]
    MissingField {
        event_id: String,
        field: &'static str,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TaskReminderProjectorConfig {
    pub now_unix: i64,
    pub due_soon_window_secs: i64,
    pub stale_after_secs: i64,
}

impl TaskReminderProjectorConfig {
    pub const fn new(now_unix: i64) -> Self {
        Self {
            now_unix,
            due_soon_window_secs: 24 * 60 * 60,
            stale_after_secs: 30 * 24 * 60 * 60,
        }
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct TaskReminderReadProjector;

#[derive(Debug, Clone)]
struct TaskAccumulator {
    task_id: String,
    title: String,
    status: String,
    priority: String,
    source: String,
    due_at_unix: Option<i64>,
    trigger_at_unix: Option<i64>,
    waiting_for: Option<String>,
    source_event_ids: Vec<String>,
    last_updated_hlc_phys: u64,
    last_updated_hlc_log: u32,
}

#[derive(Debug, Deserialize)]
struct TaskReminderPayload {
    task_id: Option<String>,
    title: Option<String>,
    status: Option<String>,
    priority: Option<String>,
    source: Option<String>,
    due_at_unix: Option<i64>,
    trigger_at_unix: Option<i64>,
    waiting_for: Option<String>,
}

impl TaskReminderReadProjector {
    pub const fn new() -> Self {
        Self
    }

    pub fn name(&self) -> &'static str {
        "TaskReminderReadProjector"
    }

    pub fn version(&self) -> u32 {
        1
    }

    pub fn project(
        &self,
        events: &[EventEnvelope],
        config: TaskReminderProjectorConfig,
    ) -> Result<TaskReminderProjection, TaskReminderProjectionError> {
        let mut relevant: Vec<&EventEnvelope> = events
            .iter()
            .filter(|event| {
                matches!(
                    event.event_type.as_str(),
                    TASK_UPSERTED | TASK_STATUS_CHANGED
                )
            })
            .collect();
        relevant.sort_by_key(|event| {
            (
                event.hlc_ts.phys,
                event.hlc_ts.log,
                event.observed_at,
                event.event_id,
            )
        });

        let mut tasks: BTreeMap<String, TaskAccumulator> = BTreeMap::new();
        for event in relevant {
            let event_id = event.event_id.to_string();
            let payload: TaskReminderPayload =
                serde_json::from_slice(&event.payload).map_err(|e| {
                    TaskReminderProjectionError::PayloadDecode {
                        event_id: event_id.clone(),
                        reason: e.to_string(),
                    }
                })?;
            let task_id = required(payload.task_id, &event_id, "task_id")?;
            let entry = tasks
                .entry(task_id.clone())
                .or_insert_with(|| TaskAccumulator {
                    task_id,
                    title: String::new(),
                    status: "active".to_string(),
                    priority: "normal".to_string(),
                    source: "unknown".to_string(),
                    due_at_unix: None,
                    trigger_at_unix: None,
                    waiting_for: None,
                    source_event_ids: Vec::new(),
                    last_updated_hlc_phys: event.hlc_ts.phys,
                    last_updated_hlc_log: event.hlc_ts.log,
                });

            entry.source_event_ids.push(event_id);
            if let Some(title) = payload.title {
                entry.title = title;
            }
            if let Some(status) = payload.status {
                entry.status = status;
            }
            if let Some(priority) = payload.priority {
                entry.priority = priority;
            }
            if let Some(source) = payload.source {
                entry.source = source;
            }
            if payload.due_at_unix.is_some() {
                entry.due_at_unix = payload.due_at_unix;
            }
            if payload.trigger_at_unix.is_some() {
                entry.trigger_at_unix = payload.trigger_at_unix;
            }
            if payload.waiting_for.is_some() {
                entry.waiting_for = payload.waiting_for;
            }
            entry.last_updated_hlc_phys = event.hlc_ts.phys;
            entry.last_updated_hlc_log = event.hlc_ts.log;
        }

        Ok(TaskReminderProjection {
            tasks: tasks
                .into_values()
                .map(|task| {
                    let bucket = classify_bucket(&task, config);
                    TaskReminderItemProjection {
                        task_id: task.task_id,
                        title: task.title,
                        status: task.status,
                        bucket,
                        priority: task.priority,
                        source: task.source,
                        due_at_unix: task.due_at_unix,
                        trigger_at_unix: task.trigger_at_unix,
                        waiting_for: task.waiting_for,
                        source_event_ids: task.source_event_ids,
                        last_updated_hlc_phys: task.last_updated_hlc_phys,
                        last_updated_hlc_log: task.last_updated_hlc_log,
                    }
                })
                .collect(),
        })
    }

    pub fn compare_with_legacy(
        &self,
        projection: &TaskReminderProjection,
        legacy_rows: &[LegacyTaskReminderRow],
    ) -> TaskReminderDivergenceReport {
        let projected_by_id: BTreeMap<&str, &TaskReminderItemProjection> = projection
            .tasks
            .iter()
            .map(|task| (task.task_id.as_str(), task))
            .collect();
        let legacy_by_id: BTreeMap<&str, &LegacyTaskReminderRow> = legacy_rows
            .iter()
            .map(|task| (task.task_id.as_str(), task))
            .collect();
        let all_ids: BTreeSet<&str> = projected_by_id
            .keys()
            .chain(legacy_by_id.keys())
            .copied()
            .collect();

        let mut matching_rows = 0;
        let mut missing_in_projector = Vec::new();
        let mut missing_in_legacy = Vec::new();
        let mut status_mismatches = Vec::new();
        let mut timing_mismatches = Vec::new();

        for task_id in all_ids {
            match (projected_by_id.get(task_id), legacy_by_id.get(task_id)) {
                (Some(projected), Some(legacy)) => {
                    let status_matches = projected.status == legacy.status;
                    let timing_matches = projected.due_at_unix == legacy.due_at_unix
                        && projected.trigger_at_unix == legacy.trigger_at_unix;
                    if status_matches && timing_matches {
                        matching_rows += 1;
                    }
                    if !status_matches {
                        status_mismatches.push(TaskReminderStatusMismatch {
                            task_id: (*task_id).to_string(),
                            projector_status: projected.status.clone(),
                            legacy_status: legacy.status.clone(),
                            projector_bucket: projected.bucket,
                        });
                    }
                    if !timing_matches {
                        timing_mismatches.push(TaskReminderTimingMismatch {
                            task_id: (*task_id).to_string(),
                            projector_due_at_unix: projected.due_at_unix,
                            legacy_due_at_unix: legacy.due_at_unix,
                            projector_trigger_at_unix: projected.trigger_at_unix,
                            legacy_trigger_at_unix: legacy.trigger_at_unix,
                        });
                    }
                }
                (None, Some(legacy)) => missing_in_projector.push((*legacy).clone()),
                (Some(projected), None) => missing_in_legacy.push((*projected).clone()),
                (None, None) => {}
            }
        }

        TaskReminderDivergenceReport {
            matching_rows,
            missing_in_projector,
            missing_in_legacy,
            status_mismatches,
            timing_mismatches,
        }
    }
}

fn classify_bucket(
    task: &TaskAccumulator,
    config: TaskReminderProjectorConfig,
) -> TaskReminderBucket {
    if matches!(
        task.status.as_str(),
        "done" | "completed" | "fired" | "cancelled" | "archived"
    ) {
        return TaskReminderBucket::Completed;
    }
    if task
        .waiting_for
        .as_deref()
        .is_some_and(|s| !s.trim().is_empty())
        || task.status == "waiting"
    {
        return TaskReminderBucket::WaitingFor;
    }
    let due_or_trigger = task.due_at_unix.or(task.trigger_at_unix);
    if let Some(ts) = due_or_trigger
        && ts >= config.now_unix
        && ts <= config.now_unix + config.due_soon_window_secs
    {
        return TaskReminderBucket::DueSoon;
    }
    if let Some(ts) = due_or_trigger
        && ts < config.now_unix - config.stale_after_secs
    {
        return TaskReminderBucket::Stale;
    }
    TaskReminderBucket::Active
}

fn required(
    value: Option<String>,
    event_id: &str,
    field: &'static str,
) -> Result<String, TaskReminderProjectionError> {
    match value {
        Some(value) if !value.trim().is_empty() => Ok(value),
        _ => Err(TaskReminderProjectionError::MissingField {
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
    use crate::heirloom::types::{BundleId, Did, LineageId};
    use chrono::{DateTime, Utc};
    use serde_json::json;
    use uuid::Uuid;

    fn envelope(
        event_type: &str,
        hlc_phys: u64,
        id_suffix: u8,
        payload: serde_json::Value,
    ) -> EventEnvelope {
        let mut id_bytes = [0u8; 16];
        id_bytes[15] = id_suffix;
        EventEnvelope {
            event_id: Uuid::from_bytes(id_bytes),
            schema_version: 1,
            event_type: EventTypeTag::new(event_type),
            prev_hash: [0; 32],
            hlc_ts: HlcTimestamp {
                phys: hlc_phys,
                log: 0,
            },
            observed_at: DateTime::parse_from_rfc3339("2026-05-01T00:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
            l0_hash: [0; 32],
            l1_hash: [0; 32],
            l2_manifest_hash: [0; 32],
            lineage_id: LineageId::new("lineage:test"),
            author_did: Did::new("did:aegis:test"),
            signatures: vec![],
            sensitivity_tier: SensitivityTier::Ordinary,
            bundle_ids: vec![BundleId::new("bundle:tasks")],
            payload_encoding: PayloadEncoding::Plaintext,
            payload: serde_json::to_vec(&payload).unwrap(),
        }
    }

    #[test]
    fn task_projector_rebuilds_deterministically_and_tracks_source_events() {
        let events = vec![
            envelope(
                TASK_UPSERTED,
                20,
                2,
                json!({"task_id":"t1","title":"Updated","status":"active"}),
            ),
            envelope(
                TASK_UPSERTED,
                10,
                1,
                json!({"task_id":"t1","title":"Original","status":"active","priority":"high","source":"user","trigger_at_unix":1000}),
            ),
            envelope("observation:user_stated", 30, 3, json!({"ignored":true})),
        ];

        let projection = TaskReminderReadProjector::new()
            .project(&events, TaskReminderProjectorConfig::new(500))
            .unwrap();

        assert_eq!(projection.tasks.len(), 1);
        let task = &projection.tasks[0];
        assert_eq!(task.task_id, "t1");
        assert_eq!(task.title, "Updated");
        assert_eq!(task.priority, "high");
        assert_eq!(task.trigger_at_unix, Some(1000));
        assert_eq!(task.source_event_ids.len(), 2);
        assert_eq!(task.last_updated_hlc_phys, 20);
    }

    #[test]
    fn task_projector_distinguishes_core_buckets() {
        let now = 1_000_000;
        let events = vec![
            envelope(
                TASK_UPSERTED,
                1,
                1,
                json!({"task_id":"active","title":"Active","status":"active"}),
            ),
            envelope(
                TASK_UPSERTED,
                2,
                2,
                json!({"task_id":"due","title":"Due","status":"active","trigger_at_unix":now + 3600}),
            ),
            envelope(
                TASK_UPSERTED,
                3,
                3,
                json!({"task_id":"done","title":"Done","status":"done"}),
            ),
            envelope(
                TASK_UPSERTED,
                4,
                4,
                json!({"task_id":"stale","title":"Stale","status":"active","due_at_unix":now - 40 * 24 * 60 * 60}),
            ),
            envelope(
                TASK_UPSERTED,
                5,
                5,
                json!({"task_id":"waiting","title":"Waiting","status":"active","waiting_for":"Alice"}),
            ),
        ];

        let projection = TaskReminderReadProjector::new()
            .project(&events, TaskReminderProjectorConfig::new(now))
            .unwrap();
        let buckets: BTreeMap<_, _> = projection
            .tasks
            .iter()
            .map(|task| (task.task_id.as_str(), task.bucket))
            .collect();

        assert_eq!(buckets["active"], TaskReminderBucket::Active);
        assert_eq!(buckets["due"], TaskReminderBucket::DueSoon);
        assert_eq!(buckets["done"], TaskReminderBucket::Completed);
        assert_eq!(buckets["stale"], TaskReminderBucket::Stale);
        assert_eq!(buckets["waiting"], TaskReminderBucket::WaitingFor);
    }

    #[test]
    fn task_projector_reports_legacy_divergence() {
        let events = vec![
            envelope(
                TASK_UPSERTED,
                1,
                1,
                json!({"task_id":"match","title":"Match","status":"active","trigger_at_unix":100}),
            ),
            envelope(
                TASK_UPSERTED,
                2,
                2,
                json!({"task_id":"status","title":"Status","status":"done"}),
            ),
            envelope(
                TASK_UPSERTED,
                3,
                3,
                json!({"task_id":"timing","title":"Timing","status":"active","trigger_at_unix":300}),
            ),
            envelope(
                TASK_UPSERTED,
                4,
                4,
                json!({"task_id":"projector_only","title":"Projector","status":"active"}),
            ),
        ];
        let projector = TaskReminderReadProjector::new();
        let projection = projector
            .project(&events, TaskReminderProjectorConfig::new(1))
            .unwrap();
        let legacy = vec![
            LegacyTaskReminderRow {
                task_id: "match".to_string(),
                title: "Match".to_string(),
                status: "active".to_string(),
                due_at_unix: None,
                trigger_at_unix: Some(100),
            },
            LegacyTaskReminderRow {
                task_id: "status".to_string(),
                title: "Status".to_string(),
                status: "active".to_string(),
                due_at_unix: None,
                trigger_at_unix: None,
            },
            LegacyTaskReminderRow {
                task_id: "timing".to_string(),
                title: "Timing".to_string(),
                status: "active".to_string(),
                due_at_unix: None,
                trigger_at_unix: Some(301),
            },
            LegacyTaskReminderRow {
                task_id: "legacy_only".to_string(),
                title: "Legacy".to_string(),
                status: "active".to_string(),
                due_at_unix: None,
                trigger_at_unix: None,
            },
        ];

        let report = projector.compare_with_legacy(&projection, &legacy);

        assert_eq!(report.matching_rows, 1);
        assert_eq!(report.status_mismatches.len(), 1);
        assert_eq!(report.timing_mismatches.len(), 1);
        assert_eq!(report.missing_in_projector[0].task_id, "legacy_only");
        assert_eq!(report.missing_in_legacy[0].task_id, "projector_only");
    }
}
