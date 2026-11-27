//! Heirloom projectors (Phase C1).
//!
//! Each projector owns one projection table; the `Projector` trait routes
//! events into them via `ProjectorDispatcher`.

pub mod bundle_index;
pub mod entity_index;
pub mod profile;
pub mod schema_registry;
pub mod tasks;

#[cfg(test)]
mod tests;

pub use bundle_index::BundleIndexProjector;
pub use entity_index::EntityIndexProjector;
pub use profile::{
    ProfileProjection, ProfileReadProjector, ProfileShadowDebugConfig, ProfileShadowDebugSurface,
    ProfileSlotProjection,
};
pub use schema_registry::{SchemaRegistryEntry, SchemaRegistryProjector};
pub use tasks::{
    LegacyTaskReminderRow, TaskReminderBucket, TaskReminderDivergenceReport,
    TaskReminderItemProjection, TaskReminderProjection, TaskReminderProjectorConfig,
    TaskReminderReadProjector, TaskReminderStatusMismatch, TaskReminderTimingMismatch,
};
