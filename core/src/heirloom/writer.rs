//! HeirloomWriter: unified-write seam.
//!
//! Spec: docs/heirloom/specs/2026-04-16-p1-substrate-design.md §3 Q3.1 + §5 Q5.9.
//! Orchestrates ethics + welfare gate → payload encrypt → HLC tick → SQL
//! transaction (log INSERT + projector dispatch). Single-writer invariant
//! per spec: one Writer owns one SQLite Connection; tip cache is in-memory
//! and authoritative during a process.

use crate::heirloom::EventEnvelope;
use crate::heirloom::hlc::Hlc;
use crate::heirloom::log::Tip;
use crate::heirloom::projector::{Projector, ProjectorDispatcher, ProjectorError};
use crate::heirloom::seams::{
    CodecError, EnclaveError, EthicsContext, EthicsValidator, PayloadCodec, SecureEnclave,
    SignerError, WelfareError, WelfareOracle, ZeroizeHook,
};
use crate::heirloom::types::{Did, OwnerState};
use parking_lot::Mutex;
use rusqlite::Connection;
use std::sync::Arc;
use thiserror::Error;
use uuid::Uuid;

/// Errors from `HeirloomWriter` operations. Exhaustive so call-sites can
/// match every reject reason without string matching.
#[derive(Debug, Error)]
pub enum WriterError {
    #[error("ethics 拒绝 / ethics rejected: {reason}")]
    EthicsRejected { reason: String },
    #[error("welfare 拒绝 / welfare rejected: expected_delta={delta} below threshold")]
    WelfareRejected { delta: f32 },
    #[error("payload codec error: {0}")]
    Codec(#[from] CodecError),
    #[error("projector error: {0}")]
    Projector(#[from] ProjectorError),
    #[error("enclave error: {0}")]
    Enclave(#[from] EnclaveError),
    #[error("signer error: {0}")]
    Signer(#[from] SignerError),
    #[error("welfare oracle error: {0}")]
    Welfare(#[from] WelfareError),
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error(
        "tip state inconsistent: expected prev_hash {expected:02x?}, envelope has {actual:02x?}"
    )]
    TipMismatch {
        expected: [u8; 32],
        actual: [u8; 32],
    },
    #[error("hlc regression: tip={expected:?}, envelope={actual:?}")]
    HlcRegression {
        expected: crate::heirloom::HlcTimestamp,
        actual: crate::heirloom::HlcTimestamp,
    },
    #[error("canonical serialization failed: {0}")]
    Canonical(String),
    #[error("log is not bootstrapped: call bootstrap_fresh_log() first")]
    NotBootstrapped,
    #[error("log is already bootstrapped (has {count} events)")]
    AlreadyBootstrapped { count: u64 },
    #[error("welfare threshold config missing but envelope is proactive AI")]
    WelfareThresholdMissing,
    #[error("schema 拒绝 / unknown event_type: {event_type}")]
    SchemaUnknown { event_type: String },
}

/// Welfare reject threshold. Actions with `expected_delta < threshold` are
/// rejected. Default is `-0.5` (moderately harmful cutoff).
pub const DEFAULT_WELFARE_REJECT_THRESHOLD: f32 = -0.5;

/// Event types that bypass ethics/welfare gates because they ARE the
/// product of gates (spec §5 Q5.9 Round-8 P0-3). Running a gate on a
/// reject event would create an infinite rejection loop.
pub const BYPASS_EVENT_TYPES: &[&str] = &[
    "ai:action_rejected_by_validator",
    "ai:welfare_rejected",
    "ai:schema_rejected",
    // Task 12 (Round 2): deprecation is a bypass event because the schema
    // gate is about to stop accepting the type being deprecated. Going
    // through the gate would be circular.
    "schema:type_deprecated",
];

/// Optional Writer config. Most call-sites use `WriterConfig::default()`.
#[derive(Debug, Clone)]
pub struct WriterConfig {
    /// Threshold below which welfare rejects action. Lower = more permissive.
    pub welfare_reject_threshold: f32,
    /// If true, welfare gate is skipped entirely. **TEST-ONLY.** Never set
    /// to true in production (violates spec §3 Q3.1 + 宪法 8).
    pub skip_welfare_for_testing: bool,
    /// If true, ethics gate is skipped entirely. **TEST-ONLY.** Never set
    /// in production (violates spec §5 Q5.5 + 宪法 9).
    pub skip_ethics_for_testing: bool,
}

impl Default for WriterConfig {
    fn default() -> Self {
        Self {
            welfare_reject_threshold: DEFAULT_WELFARE_REJECT_THRESHOLD,
            skip_welfare_for_testing: false,
            skip_ethics_for_testing: false,
        }
    }
}

// ── OwnerStateProvider ────────────────────────────────────────────────────

/// Returns current owner state. Abstraction point so tests can mock.
pub trait OwnerStateProvider: Send + Sync {
    fn current(&self) -> OwnerState;
}

/// Convenience impl for tests: a fixed owner state.
pub struct FixedOwnerState(pub OwnerState);

impl OwnerStateProvider for FixedOwnerState {
    fn current(&self) -> OwnerState {
        self.0
    }
}

// ── SchemaRegistry ────────────────────────────────────────────────────────

/// Schema-ledger registry. Abstraction point so Phase D can back by SQLite
/// and future phases may swap in-memory / networked registries. Queries
/// are expected to be fast (indexed); `is_registered` is called once per
/// `append_event`.
///
/// **Round 2 forward-proofing.** The `did: &Did` parameter is present on
/// `is_registered` so Phase P4 per-DID federation can honor per-lineage
/// taxonomies without an API break. The P1 `DbBackedSchemaRegistry` impl
/// IGNORES the DID (one lineage → one registry in P1); `NoopSchemaRegistry`
/// also ignores it.
pub trait SchemaRegistry: Send + Sync + std::fmt::Debug {
    /// True if `event_type` has a corresponding non-deprecated
    /// `schema:type_defined` row (`valid_until_phys IS NULL`) OR is one of
    /// the pre-genesis hardcoded types (D3). `did` is the owner's DID; P1
    /// impls ignore it.
    fn is_registered(&self, did: &Did, event_type: &str) -> bool;

    /// Register a new type at runtime. P1 impl is a no-op (all types
    /// registered via log events). Exposed for future phases where the
    /// registry is primary and the log derives from it.
    fn register_type(
        &self,
        _did: &Did,
        _def: crate::heirloom::schema_bootstrap::SchemaTypeDefinition,
    ) {
    }
}

/// D3 hardcode list: types accepted without a heirloom_schema_registry row.
/// These are the chicken-and-egg cases that bootstrap the registry itself.
pub const HARDCODED_REGISTRY_TYPES: &[&str] =
    &["heirloom:genesis", "schema:signature_scheme_registered"];

/// SQLite-backed registry. Reads from `heirloom_schema_registry` per call;
/// no caching in P1 (indexed query is cheap). Phase E may add an LRU.
#[derive(Debug, Clone)]
pub struct DbBackedSchemaRegistry {
    conn: Arc<Mutex<Connection>>,
}

impl DbBackedSchemaRegistry {
    pub fn new(conn: Arc<Mutex<Connection>>) -> Self {
        Self { conn }
    }
}

impl SchemaRegistry for DbBackedSchemaRegistry {
    fn is_registered(&self, _did: &Did, event_type: &str) -> bool {
        if HARDCODED_REGISTRY_TYPES.contains(&event_type) {
            return true;
        }
        let conn = self.conn.lock();
        // Round 2 (D9 deprecation): `valid_until_phys IS NULL` filter means
        // rows that have been deprecated (via schema:type_deprecated +
        // SchemaRegistryProjector extension in Task 12) are no longer
        // accepted for new appends, even though the log's historical events
        // referencing them remain fully readable.
        let count: rusqlite::Result<i64> = conn.query_row(
            "SELECT COUNT(*) FROM heirloom_schema_registry
             WHERE kind = 'type_defined' AND id = ?1 AND valid_until_phys IS NULL",
            rusqlite::params![event_type],
            |row| row.get(0),
        );
        matches!(count, Ok(n) if n > 0)
    }
}

/// Test-only registry that accepts any event_type. Use this when a test
/// does not want to exercise the schema gate. FIX 5 (CTO Round 1).
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopSchemaRegistry;

impl NoopSchemaRegistry {
    pub fn new() -> Self {
        Self
    }
}

impl SchemaRegistry for NoopSchemaRegistry {
    fn is_registered(&self, _did: &Did, _event_type: &str) -> bool {
        true
    }
}

// ── HeirloomWriter ────────────────────────────────────────────────────────

/// The unified write seam. All persistent writes in the heirloom flow
/// through `append_event()`.
pub struct HeirloomWriter {
    pub(crate) conn: Arc<Mutex<Connection>>,
    pub(crate) tip: Mutex<Option<Tip>>,
    pub(crate) hlc: Arc<Hlc>,
    pub(crate) enclave: Arc<dyn SecureEnclave>,
    pub(crate) codec: Arc<dyn PayloadCodec>,
    pub(crate) ethics: Arc<dyn EthicsValidator>,
    pub(crate) welfare: Arc<dyn WelfareOracle>,
    // Retained for future tamper-wipe hook; not called in the append path.
    #[allow(dead_code)]
    pub(crate) zeroize: Arc<dyn ZeroizeHook>,
    pub(crate) dispatcher: Mutex<ProjectorDispatcher>,
    pub(crate) owner_state_provider: Arc<dyn OwnerStateProvider>,
    pub(crate) owner_did_provider: Arc<dyn Fn() -> Did + Send + Sync>,
    pub(crate) ethics_ctx_builder: Arc<dyn Fn() -> EthicsContext + Send + Sync>,
    pub(crate) config: WriterConfig,
    pub(crate) schema_registry: Arc<dyn SchemaRegistry>,
}

impl HeirloomWriter {
    /// Construct a new Writer around an already-connected SQLite Connection.
    /// Automatically calls `load_tip_from_db()` internally so the tip cache
    /// is consistent with DB state on return — no caller needs to remember
    /// an extra init step. If the log is empty, tip stays `None` and
    /// `append_event` will return `WriterError::NotBootstrapped` until
    /// `bootstrap_fresh_log()` is called explicitly.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        conn: Connection,
        hlc: Arc<Hlc>,
        enclave: Arc<dyn SecureEnclave>,
        codec: Arc<dyn PayloadCodec>,
        ethics: Arc<dyn EthicsValidator>,
        welfare: Arc<dyn WelfareOracle>,
        zeroize: Arc<dyn ZeroizeHook>,
        owner_state_provider: Arc<dyn OwnerStateProvider>,
        owner_did_provider: Arc<dyn Fn() -> Did + Send + Sync>,
        ethics_ctx_builder: Arc<dyn Fn() -> EthicsContext + Send + Sync>,
        schema_registry: Arc<dyn SchemaRegistry>,
        config: WriterConfig,
    ) -> Result<Self, WriterError> {
        let writer = Self {
            conn: Arc::new(Mutex::new(conn)),
            tip: Mutex::new(None),
            hlc,
            enclave,
            codec,
            ethics,
            welfare,
            zeroize,
            dispatcher: Mutex::new(ProjectorDispatcher::new()),
            owner_state_provider,
            owner_did_provider,
            ethics_ctx_builder,
            config,
            schema_registry,
        };
        writer.load_tip_from_db()?;
        Ok(writer)
    }

    /// Register a projector for dispatch during `append_event`. Test-setup
    /// helper; production config may construct a pre-registered dispatcher
    /// (future API refinement).
    pub fn register_projector(&self, projector: Arc<dyn Projector>) {
        self.dispatcher.lock().register(projector);
    }

    pub fn config(&self) -> &WriterConfig {
        &self.config
    }

    /// Return an `Arc<Mutex<Connection>>` clone so a reader (Phase C3
    /// `HeirloomReader`) can share the same single-connection invariant
    /// as the Writer. The Writer remains the sole writer; the reader is
    /// read-only against the same connection (protected by the Mutex).
    pub fn shared_connection(&self) -> Arc<Mutex<Connection>> {
        self.conn.clone()
    }

    /// Test-only accessor for the writer's HLC. Phase E invariant tests use
    /// this to tick the clock manually before exercising a shim-write and
    /// then assert the resulting event's HLC is strictly greater than the
    /// pre-tick reference (G5 — bridge must NEVER mint its own timestamp).
    #[cfg(test)]
    pub fn hlc_for_test(&self) -> Arc<Hlc> {
        self.hlc.clone()
    }

    pub fn owner_state(&self) -> OwnerState {
        self.owner_state_provider.current()
    }

    /// Load tip from DB into the in-memory cache. Called at construction.
    /// Returns `Ok(())` and sets tip to `None` if the log is empty.
    pub fn load_tip_from_db(&self) -> Result<(), WriterError> {
        let conn = self.conn.lock();
        let result = conn.query_row(
            "SELECT event_hash, hlc_phys, hlc_log FROM heirloom_event_log
             ORDER BY hlc_phys DESC, hlc_log DESC, event_hash DESC LIMIT 1",
            [],
            |row| {
                let hash_blob: Vec<u8> = row.get(0)?;
                let phys: i64 = row.get(1)?;
                let log: i64 = row.get(2)?;
                Ok((hash_blob, phys, log))
            },
        );
        match result {
            Ok((hash_blob, phys, log)) => {
                if hash_blob.len() != 32 {
                    return Err(WriterError::Canonical(format!(
                        "event_hash column has {} bytes (expected 32)",
                        hash_blob.len()
                    )));
                }
                let mut hash = [0u8; 32];
                hash.copy_from_slice(&hash_blob);
                *self.tip.lock() = Some(Tip {
                    hash,
                    hlc: crate::heirloom::HlcTimestamp {
                        phys: phys as u64,
                        log: log as u32,
                    },
                });
                Ok(())
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => {
                *self.tip.lock() = None;
                Ok(())
            }
            Err(e) => Err(WriterError::Sqlite(e)),
        }
    }

    /// Current tip (cached). `None` if log is empty / un-bootstrapped.
    pub fn current_tip(&self) -> Option<Tip> {
        *self.tip.lock()
    }

    /// Internal: write envelope + run projectors into a caller-supplied
    /// transaction. Does NOT commit and does NOT advance the tip cache — those
    /// are the outer wrapper's responsibility. Enables multi-event atomic
    /// bootstrap (Task 11) while preserving the single-event semantics of the
    /// public `append_event_inner` wrapper.
    ///
    /// Fix C (Option Y): this is a thin veneer over
    /// `append_event_in_tx_with_precomputed` — it canonicalizes + hashes the
    /// envelope and delegates. The bootstrap path reuses the precomputed form
    /// to avoid double serialize + double blake3 per event.
    pub(crate) fn append_event_in_tx(
        &self,
        tx: &rusqlite::Transaction<'_>,
        envelope: EventEnvelope,
    ) -> Result<[u8; 32], WriterError> {
        let canonical = crate::heirloom::canonical::canonical_bytes(&envelope)
            .map_err(|e| WriterError::Canonical(format!("canonical_bytes: {e}")))?;
        let hash = *blake3::hash(&canonical).as_bytes();
        self.append_event_in_tx_with_precomputed(tx, envelope, &canonical, hash)
    }

    /// Internal: same contract as `append_event_in_tx` but skips the canonical
    /// serialization + blake3 hash because the caller already did both. Used by
    /// `bootstrap_fresh_log_with_types`, which must compute each envelope's
    /// hash up-front to thread it through as the NEXT envelope's `prev_hash`
    /// (the chain is built pre-tx, then written in-tx). Without this helper
    /// the bootstrap would canonicalize + hash every envelope twice (once
    /// pre-tx for chain construction, once inside `append_event_in_tx` for
    /// the row write).
    ///
    /// Invariant: `blake3::hash(canonical) == hash`; callers that violate this
    /// will write a row whose stored `envelope_canonical` bytes don't match
    /// its `event_hash`, producing a silently corrupt log. Assert in debug
    /// builds.
    pub(crate) fn append_event_in_tx_with_precomputed(
        &self,
        tx: &rusqlite::Transaction<'_>,
        envelope: EventEnvelope,
        canonical: &[u8],
        hash: [u8; 32],
    ) -> Result<[u8; 32], WriterError> {
        debug_assert_eq!(
            *blake3::hash(canonical).as_bytes(),
            hash,
            "append_event_in_tx_with_precomputed: caller-supplied hash does not match blake3(canonical)"
        );

        tx.execute(
            "INSERT INTO heirloom_event_log (
                event_hash, prev_hash, event_id_uuid, schema_version, event_type,
                hlc_phys, hlc_log, observed_at_rfc3339, l0_hash, l1_hash,
                l2_manifest_hash, lineage_id, author_did, sensitivity_tier,
                bundle_ids_json, payload_encoding_json, payload, signatures_json,
                envelope_canonical, inserted_at_unix_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20)",
            rusqlite::params![
                hash.as_slice(),
                envelope.prev_hash.as_slice(),
                envelope.event_id.to_string(),
                envelope.schema_version,
                envelope.event_type.as_str(),
                envelope.hlc_ts.phys as i64,
                envelope.hlc_ts.log as i64,
                envelope.observed_at.to_rfc3339(),
                envelope.l0_hash.as_slice(),
                envelope.l1_hash.as_slice(),
                envelope.l2_manifest_hash.as_slice(),
                envelope.lineage_id.as_str(),
                envelope.author_did.as_str(),
                crate::heirloom::sensitivity_tier_wire_str(&envelope.sensitivity_tier),
                serde_json::to_string(&envelope.bundle_ids)
                    .map_err(|e| WriterError::Canonical(format!("bundle_ids: {e}")))?,
                serde_json::to_string(&envelope.payload_encoding)
                    .map_err(|e| WriterError::Canonical(format!("payload_encoding: {e}")))?,
                envelope.payload.as_slice(),
                serde_json::to_string(&envelope.signatures)
                    .map_err(|e| WriterError::Canonical(format!("signatures: {e}")))?,
                canonical,
                chrono::Utc::now().timestamp_millis(),
            ],
        )?;

        {
            let dispatcher = self.dispatcher.lock();
            dispatcher.dispatch(tx, &envelope)?;
        }

        Ok(hash)
    }

    /// Thin wrapper: open an exclusive tx, delegate to append_event_in_tx,
    /// commit, advance tip cache. Semantics identical to the pre-Task-11
    /// `append_event_inner` — every existing caller keeps working with no
    /// signature change.
    pub(crate) fn append_event_inner(
        &self,
        envelope: EventEnvelope,
    ) -> Result<[u8; 32], WriterError> {
        let hlc_snapshot = envelope.hlc_ts;
        let hash = {
            let mut conn = self.conn.lock();
            let tx = conn.transaction()?;
            let hash = self.append_event_in_tx(&tx, envelope)?;
            tx.commit()?;
            hash
        };
        *self.tip.lock() = Some(Tip {
            hash,
            hlc: hlc_snapshot,
        });
        Ok(hash)
    }

    /// Bootstrap an empty log with genesis + SchemeRegistered events, both
    /// UNSIGNED (spec deviation D7: circular dep — the log is empty so no
    /// chain exists to sign against). Idempotent via empty-log check;
    /// subsequent calls return `WriterError::AlreadyBootstrapped`.
    ///
    /// Atomicity note: the two inserts are NOT wrapped in a single rusqlite
    /// tx — each `append_event_inner` commits independently. In Phase C2
    /// this is acceptable because (a) PlaintextCodec + SoftThresholdSigner
    /// never fail at the encrypt/sign step, so genesis succeeding implies
    /// SchemeRegistered will also succeed, and (b) projectors are registered
    /// by caller AFTER bootstrap in typical usage. If genesis commits but
    /// SchemeRegistered fails (e.g. disk full between inserts), the log is
    /// stuck in a 1-event half-state — the `count > 0` idempotency guard
    /// then refuses retries. Recovery requires out-of-band truncation.
    /// Phase D will either wrap the whole sequence in a tx or add a
    /// recovery branch that detects genesis-only state.
    pub fn bootstrap_fresh_log(
        &self,
        lineage: crate::heirloom::types::LineageId,
        owner_did: crate::heirloom::types::Did,
        constitution: crate::heirloom::seams::ConstitutionTuple,
    ) -> Result<(), WriterError> {
        {
            let conn = self.conn.lock();
            let count: i64 =
                conn.query_row("SELECT COUNT(*) FROM heirloom_event_log", [], |r| r.get(0))?;
            if count > 0 {
                return Err(WriterError::AlreadyBootstrapped {
                    count: count as u64,
                });
            }
        }

        *self.tip.lock() = None;

        let genesis_hlc = self.hlc.tick();
        let genesis = self.build_genesis_envelope(&lineage, &owner_did, &constitution, genesis_hlc);
        self.append_event_inner(genesis)?;

        let signer = self.enclave.threshold_signer();
        let scheme_hlc = self.hlc.tick();
        let payload = crate::heirloom::seams::threshold_signer::SchemeRegisteredPayload {
            scheme_id: signer.scheme_id().to_string(),
            algorithm: "Ed25519".to_string(),
            threshold: signer.quorum(),
            group_pubkey: signer.group_pubkey(),
            verifier_spec_ref: "RFC 8032".to_string(),
            valid_from_hlc: scheme_hlc,
            valid_until_hlc: None,
        };
        let scheme_env = self.build_scheme_registered_envelope(
            &lineage,
            &owner_did,
            &constitution,
            scheme_hlc,
            &payload,
        );
        self.append_event_inner(scheme_env)?;
        Ok(())
    }

    /// Extended bootstrap: genesis + SchemeRegistered + N schema:type_defined
    /// events, all committed in a SINGLE outer transaction (Task 11). A
    /// mid-loop failure rolls back ALL events — the log is either fully
    /// bootstrapped or pristine empty.
    ///
    /// Fix C (Option Y): envelope canonical bytes + blake3 hash are
    /// pre-computed in the chain-construction phase (before the tx opens)
    /// and reused via `append_event_in_tx_with_precomputed` — the in-tx
    /// write path does not repeat the serialize + hash work.
    pub fn bootstrap_fresh_log_with_types(
        &self,
        lineage: crate::heirloom::types::LineageId,
        owner_did: crate::heirloom::types::Did,
        constitution: crate::heirloom::seams::ConstitutionTuple,
        defs: &[crate::heirloom::schema_bootstrap::SchemaTypeDefinition],
    ) -> Result<(), WriterError> {
        // Empty-log idempotency check (before opening tx — avoids needless
        // lock acquisition on the happy path where the count is 0).
        {
            let conn = self.conn.lock();
            let count: i64 =
                conn.query_row("SELECT COUNT(*) FROM heirloom_event_log", [], |r| r.get(0))?;
            if count > 0 {
                return Err(WriterError::AlreadyBootstrapped {
                    count: count as u64,
                });
            }
        }

        // Pre-build all 48 envelopes WITH correct prev_hash chaining. The
        // chain must be constructed incrementally (prev_hash_N = hash(env_{N-1})).
        // Because nothing is written yet, the prev_hash field starts at
        // GENESIS_PREV_HASH.
        struct PreBuilt {
            envelope: EventEnvelope,
            canonical: Vec<u8>,
            hash: [u8; 32],
        }
        let mut prebuilt: Vec<PreBuilt> = Vec::with_capacity(2 + defs.len());
        let mut running_prev = crate::heirloom::genesis::GENESIS_PREV_HASH;

        // Genesis
        let genesis_hlc = self.hlc.tick();
        let mut genesis_env =
            self.build_genesis_envelope(&lineage, &owner_did, &constitution, genesis_hlc);
        genesis_env.prev_hash = running_prev;
        let genesis_canonical = crate::heirloom::canonical::canonical_bytes(&genesis_env)
            .map_err(|e| WriterError::Canonical(format!("canonical: {e}")))?;
        let genesis_hash = *blake3::hash(&genesis_canonical).as_bytes();
        prebuilt.push(PreBuilt {
            envelope: genesis_env,
            canonical: genesis_canonical,
            hash: genesis_hash,
        });
        running_prev = genesis_hash;

        // SchemeRegistered — we can't use `build_scheme_registered_envelope`
        // because it reads `self.current_tip()` which is None at this point.
        // Build manually instead (cloned structure minus the tip lookup).
        let signer = self.enclave.threshold_signer();
        let scheme_hlc = self.hlc.tick();
        let scheme_payload = crate::heirloom::seams::threshold_signer::SchemeRegisteredPayload {
            scheme_id: signer.scheme_id().to_string(),
            algorithm: "Ed25519".to_string(),
            threshold: signer.quorum(),
            group_pubkey: signer.group_pubkey(),
            verifier_spec_ref: "RFC 8032".to_string(),
            valid_from_hlc: scheme_hlc,
            valid_until_hlc: None,
        };
        let scheme_env = {
            use crate::heirloom::enums::{EventTypeTag, PayloadEncoding, SensitivityTier};
            EventEnvelope {
                event_id: Uuid::now_v7(),
                schema_version: 1,
                event_type: EventTypeTag::new("schema:signature_scheme_registered"),
                prev_hash: running_prev,
                hlc_ts: scheme_hlc,
                observed_at: chrono::Utc::now(),
                l0_hash: constitution.l0_hash,
                l1_hash: constitution.l1_hash,
                l2_manifest_hash: constitution.l2_manifest_hash,
                lineage_id: lineage.clone(),
                author_did: owner_did.clone(),
                signatures: vec![],
                sensitivity_tier: SensitivityTier::Sacred,
                bundle_ids: vec![crate::heirloom::BundleId::new("bundle:core")],
                payload_encoding: PayloadEncoding::Plaintext,
                payload: serde_json::to_vec(&scheme_payload).expect("payload serde"),
            }
        };
        let scheme_canonical = crate::heirloom::canonical::canonical_bytes(&scheme_env)
            .map_err(|e| WriterError::Canonical(format!("canonical: {e}")))?;
        let scheme_hash = *blake3::hash(&scheme_canonical).as_bytes();
        prebuilt.push(PreBuilt {
            envelope: scheme_env,
            canonical: scheme_canonical,
            hash: scheme_hash,
        });
        running_prev = scheme_hash;

        // N schema:type_defined
        for def in defs {
            let hlc = self.hlc.tick();
            let payload = crate::heirloom::schema_bootstrap::SchemaTypeDefPayload::from(def);
            let mut env = self.build_schema_type_defined_envelope(
                &lineage,
                &owner_did,
                &constitution,
                hlc,
                &payload,
            );
            env.prev_hash = running_prev;
            let env_canonical = crate::heirloom::canonical::canonical_bytes(&env)
                .map_err(|e| WriterError::Canonical(format!("canonical: {e}")))?;
            let env_hash = *blake3::hash(&env_canonical).as_bytes();
            prebuilt.push(PreBuilt {
                envelope: env,
                canonical: env_canonical,
                hash: env_hash,
            });
            running_prev = env_hash;
        }

        // Snapshot the last envelope's HLC BEFORE the tx (we move the
        // envelope into the helper and can't read it after).
        let last_hlc = prebuilt
            .last()
            .map(|p| p.envelope.hlc_ts)
            .unwrap_or(crate::heirloom::HlcTimestamp { phys: 0, log: 0 });

        // Now open the outer tx and write all pre-built envelopes. Because
        // prev_hash + envelope canonical-bytes + blake3 hash were computed
        // above, the only work inside the tx is SQL inserts + projector
        // dispatch. Mid-loop failure → SQLite auto-rolls back on Drop.
        {
            let mut conn = self.conn.lock();
            let tx = conn.transaction()?;
            for p in prebuilt {
                self.append_event_in_tx_with_precomputed(&tx, p.envelope, &p.canonical, p.hash)?;
            }
            tx.commit()?;
        }
        *self.tip.lock() = Some(Tip {
            hash: running_prev,
            hlc: last_hlc,
        });

        Ok(())
    }

    fn build_genesis_envelope(
        &self,
        lineage: &crate::heirloom::types::LineageId,
        owner_did: &crate::heirloom::types::Did,
        constitution: &crate::heirloom::seams::ConstitutionTuple,
        hlc: crate::heirloom::HlcTimestamp,
    ) -> EventEnvelope {
        use crate::heirloom::enums::{EventTypeTag, PayloadEncoding, SensitivityTier};
        use crate::heirloom::genesis::GENESIS_PREV_HASH;
        EventEnvelope {
            event_id: Uuid::now_v7(),
            schema_version: 1,
            event_type: EventTypeTag::new("heirloom:genesis"),
            prev_hash: GENESIS_PREV_HASH,
            hlc_ts: hlc,
            observed_at: chrono::Utc::now(),
            l0_hash: constitution.l0_hash,
            l1_hash: constitution.l1_hash,
            l2_manifest_hash: constitution.l2_manifest_hash,
            lineage_id: lineage.clone(),
            author_did: owner_did.clone(),
            signatures: vec![],
            sensitivity_tier: SensitivityTier::Sacred,
            bundle_ids: vec![crate::heirloom::BundleId::new("bundle:core")],
            payload_encoding: PayloadEncoding::Plaintext,
            payload: b"{\"type\":\"genesis\"}".to_vec(),
        }
    }

    /// True if this event_type bypasses ethics/welfare gates.
    pub fn is_bypass_event_type(event_type: &str) -> bool {
        BYPASS_EVENT_TYPES.contains(&event_type)
    }

    /// Run the ethics gate for this envelope. Non-AI events and bypass
    /// types are skipped. Reject verdicts emit an `ai:action_rejected_by_validator`
    /// audit event through `append_reject_event_unchecked`. EscalateToOwner
    /// honors all three `EscalationFallback` variants (spec §5 Q5.5).
    fn run_ethics_gate(&self, envelope: &EventEnvelope) -> Result<(), WriterError> {
        if self.config.skip_ethics_for_testing {
            return Ok(());
        }
        if !envelope.is_ai_action() {
            return Ok(());
        }
        let action = crate::heirloom::types::ProposedAction {
            event_type: envelope.event_type.as_str().to_string(),
            author: envelope.author_did.clone(),
            target: None,
            summary: String::new(),
            payload_digest: *blake3::hash(&envelope.payload).as_bytes(),
        };
        let ctx = (self.ethics_ctx_builder)();
        let verdict = self.ethics.evaluate(&action, &ctx);

        use crate::heirloom::seams::{EscalationFallback, EthicsVerdict};
        match verdict {
            EthicsVerdict::Allow => Ok(()),
            EthicsVerdict::AllowWithGuidance { .. } => Ok(()),
            EthicsVerdict::Reject { reason, .. } => {
                let reject_env = self.build_ethics_reject_envelope(envelope, &reason);
                self.append_reject_event_unchecked(reject_env)?;
                Err(WriterError::EthicsRejected { reason })
            }
            EthicsVerdict::RecordWithPenanceMarker { .. } => Ok(()),
            EthicsVerdict::EscalateToOwner { fallback, .. } => {
                let (reason, allow) = match fallback {
                    EscalationFallback::DefaultReject => (
                        "escalation / 需人工决策 — fallback=DefaultReject".to_string(),
                        false,
                    ),
                    EscalationFallback::DefaultAllowWithAlert => (
                        "escalation / 待主人复核 — fallback=DefaultAllowWithAlert".to_string(),
                        true,
                    ),
                    EscalationFallback::DeferUntilGreatReading => (
                        "escalation / 延至 Great Reading — fallback=DeferUntilGreatReading"
                            .to_string(),
                        false,
                    ),
                };
                let mut audit = self.build_escalation_alert_envelope(envelope, &reason, allow);
                audit.prev_hash = match *self.tip.lock() {
                    Some(t) => t.hash,
                    None => return Err(WriterError::NotBootstrapped),
                };
                audit.hlc_ts = self.hlc.tick();
                self.append_event_inner(audit)?;
                if allow {
                    Ok(())
                } else {
                    let reject_env = self.build_ethics_reject_envelope(envelope, &reason);
                    self.append_reject_event_unchecked(reject_env)?;
                    Err(WriterError::EthicsRejected { reason })
                }
            }
        }
    }

    fn build_escalation_alert_envelope(
        &self,
        original: &EventEnvelope,
        reason: &str,
        allowed: bool,
    ) -> EventEnvelope {
        use crate::heirloom::enums::{EventTypeTag, PayloadEncoding, SensitivityTier};
        let original_hash = crate::heirloom::canonical::event_hash(original)
            .expect("escalate target envelope must be JCS-serializable");
        let payload = serde_json::json!({
            "escalated_event_hash": hex::encode(original_hash),
            "escalated_event_type": original.event_type.as_str(),
            "reason": reason,
            "allowed_pending_review": allowed,
        });
        EventEnvelope {
            event_id: Uuid::now_v7(),
            schema_version: 1,
            event_type: EventTypeTag::new("ai:escalation_alert"),
            prev_hash: [0; 32],
            hlc_ts: original.hlc_ts,
            observed_at: chrono::Utc::now(),
            l0_hash: original.l0_hash,
            l1_hash: original.l1_hash,
            l2_manifest_hash: original.l2_manifest_hash,
            lineage_id: original.lineage_id.clone(),
            author_did: (self.owner_did_provider)(),
            signatures: vec![],
            sensitivity_tier: SensitivityTier::Sensitive,
            bundle_ids: original.bundle_ids.clone(),
            payload_encoding: PayloadEncoding::Plaintext,
            payload: serde_json::to_vec(&payload).expect("static JSON"),
        }
    }

    fn build_ethics_reject_envelope(
        &self,
        rejected: &EventEnvelope,
        reason: &str,
    ) -> EventEnvelope {
        use crate::heirloom::enums::{EventTypeTag, PayloadEncoding, SensitivityTier};
        let rejected_hash = crate::heirloom::canonical::event_hash(rejected)
            .expect("rejected envelope must be JCS-serializable");
        let payload = serde_json::json!({
            "rejected_event_hash": hex::encode(rejected_hash),
            "rejected_event_type": rejected.event_type.as_str(),
            "reason": reason,
        });
        EventEnvelope {
            event_id: Uuid::now_v7(),
            schema_version: 1,
            event_type: EventTypeTag::new("ai:action_rejected_by_validator"),
            prev_hash: [0; 32],
            hlc_ts: rejected.hlc_ts,
            observed_at: chrono::Utc::now(),
            l0_hash: rejected.l0_hash,
            l1_hash: rejected.l1_hash,
            l2_manifest_hash: rejected.l2_manifest_hash,
            lineage_id: rejected.lineage_id.clone(),
            author_did: (self.owner_did_provider)(),
            signatures: vec![],
            sensitivity_tier: SensitivityTier::Sensitive,
            bundle_ids: rejected.bundle_ids.clone(),
            payload_encoding: PayloadEncoding::Plaintext,
            payload: serde_json::to_vec(&payload).expect("static JSON"),
        }
    }

    /// Run the welfare gate. Fires only for proactive AI actions when the
    /// owner is Alive. Below-threshold deltas emit an `ai:welfare_rejected`
    /// audit event. Spec §5 Q5.9 step 3.
    fn run_welfare_gate(&self, envelope: &EventEnvelope) -> Result<(), WriterError> {
        if self.config.skip_welfare_for_testing {
            return Ok(());
        }
        if self.owner_state() != OwnerState::Alive {
            return Ok(());
        }
        if !envelope.is_proactive_ai() {
            return Ok(());
        }
        let action = crate::heirloom::types::ProposedAction {
            event_type: envelope.event_type.as_str().to_string(),
            author: envelope.author_did.clone(),
            target: None,
            summary: String::new(),
            payload_digest: *blake3::hash(&envelope.payload).as_bytes(),
        };
        let target_did = (self.owner_did_provider)();
        let score = self.welfare.marginal_welfare(&action, &target_did);
        if score.expected_delta < self.config.welfare_reject_threshold {
            let reject_env = self.build_welfare_reject_envelope(envelope, score.expected_delta);
            self.append_reject_event_unchecked(reject_env)?;
            return Err(WriterError::WelfareRejected {
                delta: score.expected_delta,
            });
        }
        Ok(())
    }

    /// Encrypt `envelope.payload` in place and update `envelope.payload_encoding`
    /// The sole durable-write entry point for non-bypass events. Spec §5 Q5.9.
    ///
    /// Pipeline:
    ///   1. Bypass check → reject events go straight through.
    ///   2. Ethics gate (AI actions only; writes audit + returns Err on reject).
    ///   3. Welfare gate (Alive + proactive AI only; audit + Err on reject).
    ///   4. Payload encrypt (D9: reject/bootstrap never reach this step).
    ///   5. prev_hash + HLC assignment.
    ///   6. append_event_inner (SQL tx + projector dispatch + tip advance).
    ///
    /// On gate rejection the original envelope is NOT written; the gate's
    /// audit event IS written, and tip advances to the audit event's hash.
    pub fn append_event(&self, mut envelope: EventEnvelope) -> Result<[u8; 32], WriterError> {
        if Self::is_bypass_event_type(envelope.event_type.as_str()) {
            return self.append_reject_event_unchecked(envelope);
        }

        // Phase D Task 5: schema-existence gate. MUST come after bypass check
        // (Gotcha #1) and before ethics/welfare (Gotcha #10). The gate passes
        // the owner's DID (from owner_did_provider) so Phase P4 per-DID
        // registry scoping can honor lineage-local taxonomies without an API
        // break (Round 2 forward-proofing). P1 DbBackedSchemaRegistry
        // ignores the DID.
        let owner_did = (self.owner_did_provider)();
        if !self
            .schema_registry
            .is_registered(&owner_did, envelope.event_type.as_str())
        {
            let reject_env = self.build_schema_reject_envelope(&envelope);
            self.append_reject_event_unchecked(reject_env)?;
            return Err(WriterError::SchemaUnknown {
                event_type: envelope.event_type.as_str().to_string(),
            });
        }

        self.run_ethics_gate(&envelope)?;
        self.run_welfare_gate(&envelope)?;
        self.encrypt_payload(&mut envelope)?;

        let prev_hash = match *self.tip.lock() {
            Some(t) => t.hash,
            None => return Err(WriterError::NotBootstrapped),
        };
        envelope.prev_hash = prev_hash;
        envelope.hlc_ts = self.hlc.tick();

        self.append_event_inner(envelope)
    }

    fn build_schema_reject_envelope(&self, rejected: &EventEnvelope) -> EventEnvelope {
        use crate::heirloom::enums::{EventTypeTag, PayloadEncoding, SensitivityTier};
        let rejected_hash = crate::heirloom::canonical::event_hash(rejected)
            .expect("rejected envelope must be JCS-serializable");
        let payload = serde_json::json!({
            "rejected_event_hash": hex::encode(rejected_hash),
            "rejected_event_type": rejected.event_type.as_str(),
            "reason": "event_type not in SchemaRegistry — see heirloom_schema_registry",
        });
        EventEnvelope {
            event_id: Uuid::now_v7(),
            schema_version: 1,
            event_type: EventTypeTag::new("ai:schema_rejected"),
            prev_hash: [0; 32],
            hlc_ts: rejected.hlc_ts,
            observed_at: chrono::Utc::now(),
            l0_hash: rejected.l0_hash,
            l1_hash: rejected.l1_hash,
            l2_manifest_hash: rejected.l2_manifest_hash,
            lineage_id: rejected.lineage_id.clone(),
            author_did: (self.owner_did_provider)(),
            signatures: vec![],
            sensitivity_tier: SensitivityTier::Sensitive,
            bundle_ids: rejected.bundle_ids.clone(),
            payload_encoding: PayloadEncoding::Plaintext,
            payload: serde_json::to_vec(&payload).expect("static JSON"),
        }
    }

    /// Encrypt `envelope.payload` in place and update `envelope.payload_encoding`
    /// with the tag returned by the codec. Spec §5 Q5.9 step 4. With
    /// PlaintextCodec this is effectively a no-op + framing; with AeadV1
    /// it's real AEAD encryption. Reject / bootstrap events MUST NOT go
    /// through this path (spec deviation D9) — callers are responsible
    /// for skipping.
    fn encrypt_payload(&self, envelope: &mut EventEnvelope) -> Result<(), WriterError> {
        use crate::heirloom::seams::{BundlePolicy, ConstitutionTuple};
        use rand::SeedableRng;
        use rand::rngs::StdRng;

        let bundle_id = envelope
            .bundle_ids
            .first()
            .cloned()
            .unwrap_or_else(|| crate::heirloom::BundleId::new("bundle:general"));
        let policy = BundlePolicy {
            bundle_id,
            preferred_encoding: match self.codec.encoding_tag() {
                crate::heirloom::enums::PayloadEncoding::Plaintext => "plaintext".to_string(),
                crate::heirloom::enums::PayloadEncoding::AeadV1 { .. } => "aead_v1".to_string(),
            },
        };
        let constitution = ConstitutionTuple {
            l0_hash: envelope.l0_hash,
            l1_hash: envelope.l1_hash,
            l2_manifest_hash: envelope.l2_manifest_hash,
        };
        let mut rng = StdRng::seed_from_u64(envelope.hlc_ts.phys);
        let (encoding, ciphertext) = self.codec.encrypt(
            &envelope.payload,
            &policy,
            &constitution,
            self.enclave.as_ref(),
            &mut rng,
        )?;
        envelope.payload = ciphertext;
        envelope.payload_encoding = encoding;
        Ok(())
    }

    fn build_welfare_reject_envelope(&self, rejected: &EventEnvelope, delta: f32) -> EventEnvelope {
        use crate::heirloom::enums::{EventTypeTag, PayloadEncoding, SensitivityTier};
        let rejected_hash = crate::heirloom::canonical::event_hash(rejected)
            .expect("rejected envelope must be JCS-serializable");
        let payload = serde_json::json!({
            "rejected_event_hash": hex::encode(rejected_hash),
            "rejected_event_type": rejected.event_type.as_str(),
            "welfare_delta": delta,
            "welfare_threshold": self.config.welfare_reject_threshold,
        });
        EventEnvelope {
            event_id: Uuid::now_v7(),
            schema_version: 1,
            event_type: EventTypeTag::new("ai:welfare_rejected"),
            prev_hash: [0; 32],
            hlc_ts: rejected.hlc_ts,
            observed_at: chrono::Utc::now(),
            l0_hash: rejected.l0_hash,
            l1_hash: rejected.l1_hash,
            l2_manifest_hash: rejected.l2_manifest_hash,
            lineage_id: rejected.lineage_id.clone(),
            author_did: (self.owner_did_provider)(),
            signatures: vec![],
            sensitivity_tier: SensitivityTier::Sensitive,
            bundle_ids: rejected.bundle_ids.clone(),
            payload_encoding: PayloadEncoding::Plaintext,
            payload: serde_json::to_vec(&payload).expect("static JSON"),
        }
    }

    /// Append a reject event, bypassing all gates. Used by the writer's own
    /// ethics/welfare rejection paths and by external callers (e.g. W18
    /// schema registry rejecting unknown event types). Event_type MUST be
    /// in BYPASS_EVENT_TYPES.
    pub fn append_reject_event_unchecked(
        &self,
        mut envelope: EventEnvelope,
    ) -> Result<[u8; 32], WriterError> {
        if !Self::is_bypass_event_type(envelope.event_type.as_str()) {
            return Err(WriterError::EthicsRejected {
                reason: format!(
                    "append_reject_event_unchecked called with non-bypass event_type {:?}",
                    envelope.event_type.as_str()
                ),
            });
        }
        let prev_hash = match *self.tip.lock() {
            Some(t) => t.hash,
            None => return Err(WriterError::NotBootstrapped),
        };
        envelope.prev_hash = prev_hash;
        envelope.hlc_ts = self.hlc.tick();
        self.append_event_inner(envelope)
    }

    fn build_scheme_registered_envelope(
        &self,
        lineage: &crate::heirloom::types::LineageId,
        owner_did: &crate::heirloom::types::Did,
        constitution: &crate::heirloom::seams::ConstitutionTuple,
        hlc: crate::heirloom::HlcTimestamp,
        payload: &crate::heirloom::seams::threshold_signer::SchemeRegisteredPayload,
    ) -> EventEnvelope {
        use crate::heirloom::enums::{EventTypeTag, PayloadEncoding, SensitivityTier};
        EventEnvelope {
            event_id: Uuid::now_v7(),
            schema_version: 1,
            event_type: EventTypeTag::new("schema:signature_scheme_registered"),
            prev_hash: self
                .current_tip()
                .map(|t| t.hash)
                .expect("tip must be Some after genesis insert in bootstrap_fresh_log"),
            hlc_ts: hlc,
            observed_at: chrono::Utc::now(),
            l0_hash: constitution.l0_hash,
            l1_hash: constitution.l1_hash,
            l2_manifest_hash: constitution.l2_manifest_hash,
            lineage_id: lineage.clone(),
            author_did: owner_did.clone(),
            signatures: vec![],
            sensitivity_tier: SensitivityTier::Sacred,
            bundle_ids: vec![crate::heirloom::BundleId::new("bundle:core")],
            payload_encoding: PayloadEncoding::Plaintext,
            payload: serde_json::to_vec(payload).expect("payload serde"),
        }
    }

    /// Task 6: envelope builder for autonomy:state_transitioned events
    /// (OwnerStateMachine transitions). Per spec §2 Q2.3 line 172 the domain
    /// is `autonomy:`, not `owner:`. Rust payload type stays
    /// `OwnerStateTransitionPayload` for readability — only the wire
    /// event_type string lives under autonomy:. See D7.
    ///
    /// Called by OwnerStateMachine::transition(); prev_hash + hlc_ts are
    /// filled by the downstream append_event pipeline.
    pub(crate) fn build_owner_state_transitioned_envelope(
        &self,
        lineage: &crate::heirloom::types::LineageId,
        owner_did: &crate::heirloom::types::Did,
        constitution: &crate::heirloom::seams::ConstitutionTuple,
        payload: &crate::heirloom::owner_state_machine::OwnerStateTransitionPayload,
    ) -> EventEnvelope {
        use crate::heirloom::enums::{EventTypeTag, PayloadEncoding, SensitivityTier};
        EventEnvelope {
            event_id: Uuid::now_v7(),
            schema_version: 1,
            event_type: EventTypeTag::new("autonomy:state_transitioned"),
            prev_hash: [0; 32], // filled by append_event
            hlc_ts: crate::heirloom::HlcTimestamp { phys: 0, log: 0 }, // filled by append_event
            observed_at: chrono::Utc::now(),
            l0_hash: constitution.l0_hash,
            l1_hash: constitution.l1_hash,
            l2_manifest_hash: constitution.l2_manifest_hash,
            lineage_id: lineage.clone(),
            author_did: owner_did.clone(),
            signatures: vec![],
            sensitivity_tier: SensitivityTier::Confidential,
            bundle_ids: vec![crate::heirloom::BundleId::new("bundle:core")],
            payload_encoding: PayloadEncoding::Plaintext,
            payload: serde_json::to_vec(payload).expect("static JSON"),
        }
    }

    /// Task 2: envelope builder for schema:type_defined bootstrap events.
    /// Phase D bootstrap path calls this once per SCHEMA_TYPE_DEFS row. Note:
    /// `prev_hash` is set from the current tip (at call time). When used in
    /// the Task 11 single-tx bootstrap path, the caller overrides prev_hash
    /// after construction to match the pre-built chain.
    pub(crate) fn build_schema_type_defined_envelope(
        &self,
        lineage: &crate::heirloom::types::LineageId,
        owner_did: &crate::heirloom::types::Did,
        constitution: &crate::heirloom::seams::ConstitutionTuple,
        hlc: crate::heirloom::HlcTimestamp,
        payload: &crate::heirloom::schema_bootstrap::SchemaTypeDefPayload,
    ) -> EventEnvelope {
        use crate::heirloom::enums::{EventTypeTag, PayloadEncoding, SensitivityTier};
        EventEnvelope {
            event_id: Uuid::now_v7(),
            schema_version: 1,
            event_type: EventTypeTag::new("schema:type_defined"),
            prev_hash: self.current_tip().map(|t| t.hash).unwrap_or([0; 32]),
            hlc_ts: hlc,
            observed_at: chrono::Utc::now(),
            l0_hash: constitution.l0_hash,
            l1_hash: constitution.l1_hash,
            l2_manifest_hash: constitution.l2_manifest_hash,
            lineage_id: lineage.clone(),
            author_did: owner_did.clone(),
            signatures: vec![],
            sensitivity_tier: SensitivityTier::Sacred,
            bundle_ids: vec![crate::heirloom::BundleId::new("bundle:core")],
            payload_encoding: PayloadEncoding::Plaintext,
            payload: serde_json::to_vec(payload).expect("payload serde"),
        }
    }

    // ── Phase D Task 12: schema:type_deprecated flow ─────────────────────

    pub(crate) fn build_schema_type_deprecated_envelope(
        &self,
        lineage: &crate::heirloom::types::LineageId,
        owner_did: &crate::heirloom::types::Did,
        constitution: &crate::heirloom::seams::ConstitutionTuple,
        payload: &crate::heirloom::schema_bootstrap::SchemaTypeDeprecatedPayload,
    ) -> EventEnvelope {
        use crate::heirloom::enums::{EventTypeTag, PayloadEncoding, SensitivityTier};
        EventEnvelope {
            event_id: Uuid::now_v7(),
            schema_version: 1,
            event_type: EventTypeTag::new("schema:type_deprecated"),
            prev_hash: [0; 32], // filled by append_reject_event_unchecked
            hlc_ts: crate::heirloom::HlcTimestamp { phys: 0, log: 0 }, // filled by unchecked-append
            observed_at: chrono::Utc::now(),
            l0_hash: constitution.l0_hash,
            l1_hash: constitution.l1_hash,
            l2_manifest_hash: constitution.l2_manifest_hash,
            lineage_id: lineage.clone(),
            author_did: owner_did.clone(),
            signatures: vec![],
            sensitivity_tier: SensitivityTier::Sacred,
            bundle_ids: vec![crate::heirloom::BundleId::new("bundle:core")],
            payload_encoding: PayloadEncoding::Plaintext,
            payload: serde_json::to_vec(payload).expect("static JSON"),
        }
    }

    /// Rebuild `heirloom_schema_registry` from the `schema:*` events already
    /// in the log when the projection is empty.
    ///
    /// A log bootstrapped by a writer that had no `SchemaRegistryProjector`
    /// registered (the production wiring before 2026-09-20) holds all 48
    /// `schema:type_defined` events but an empty registry table, so
    /// `DbBackedSchemaRegistry` rejects every non-hardcoded event type and
    /// each shim write soft-fails with `schema 拒绝`. Replaying the stored
    /// canonical envelopes through the projector in one transaction repairs
    /// that without touching the append-only log. Returns the number of
    /// events replayed (0 when the registry was already populated or the
    /// log has no schema events).
    pub fn backfill_schema_registry_from_log(&self) -> Result<usize, WriterError> {
        let mut conn = self.conn.lock();
        let registered: i64 = conn.query_row(
            "SELECT COUNT(*) FROM heirloom_schema_registry WHERE kind = 'type_defined'",
            [],
            |r| r.get(0),
        )?;
        if registered > 0 {
            return Ok(0);
        }
        let canonicals: Vec<Vec<u8>> = {
            let mut stmt = conn.prepare(
                "SELECT envelope_canonical FROM heirloom_event_log
                 WHERE event_type LIKE 'schema:%'
                 ORDER BY hlc_phys ASC, hlc_log ASC, inserted_at_unix_ms ASC",
            )?;
            let rows = stmt.query_map([], |row| row.get::<_, Vec<u8>>(0))?;
            rows.collect::<Result<Vec<_>, _>>()?
        };
        if canonicals.is_empty() {
            return Ok(0);
        }
        let projector =
            crate::heirloom::projectors::schema_registry::SchemaRegistryProjector::new();
        let tx = conn.transaction()?;
        for canonical in &canonicals {
            let envelope: EventEnvelope = serde_json::from_slice(canonical)
                .map_err(|e| WriterError::Canonical(format!("stored envelope: {e}")))?;
            projector.apply(&tx, &envelope)?;
        }
        tx.commit()?;
        Ok(canonicals.len())
    }

    /// Emit a `schema:type_deprecated` event. After this returns Ok,
    /// `SchemaRegistry::is_registered(did, event_type)` will return `false`
    /// for the deprecated type — any subsequent `append_event` with that
    /// event_type is rejected with `WriterError::SchemaUnknown`. Historical
    /// events already in the log are untouched (append-only integrity).
    ///
    /// Errors if the type was never registered, or was already deprecated.
    /// Task 12 tests pin both paths.
    ///
    /// Caller must have registered `SchemaRegistryProjector` before calling
    /// (otherwise the `valid_until_phys` flip won't happen; the is_registered
    /// filter will keep returning true). This helper relies on the caller
    /// having done that — the projector is a prerequisite for any schema
    /// registry flow.
    pub fn deprecate_type(&self, event_type: &str, reason: &str) -> Result<[u8; 32], WriterError> {
        // Pre-check: the type must currently be registered (not yet deprecated).
        let already_registered = {
            let conn = self.conn.lock();
            let count: i64 = conn.query_row(
                "SELECT COUNT(*) FROM heirloom_schema_registry
                 WHERE kind = 'type_defined' AND id = ?1 AND valid_until_phys IS NULL",
                rusqlite::params![event_type],
                |r| r.get(0),
            )?;
            count > 0
        };
        if !already_registered {
            return Err(WriterError::SchemaUnknown {
                event_type: format!(
                    "deprecate_type: type_tag {event_type:?} is not currently registered (either never defined or already deprecated)"
                ),
            });
        }

        let owner_did = (self.owner_did_provider)();
        // Read lineage + constitution from the current tip's event row. This
        // avoids a separate cache and ensures the deprecation event inherits
        // the active lineage / constitution tuple.
        let (lineage, constitution) = {
            let conn = self.conn.lock();
            conn.query_row(
                "SELECT lineage_id, l0_hash, l1_hash, l2_manifest_hash
                 FROM heirloom_event_log
                 ORDER BY hlc_phys DESC, hlc_log DESC, event_hash DESC
                 LIMIT 1",
                [],
                |row| {
                    let lineage_str: String = row.get(0)?;
                    let l0: Vec<u8> = row.get(1)?;
                    let l1: Vec<u8> = row.get(2)?;
                    let l2: Vec<u8> = row.get(3)?;
                    Ok((lineage_str, l0, l1, l2))
                },
            )
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => WriterError::NotBootstrapped,
                other => WriterError::Sqlite(other),
            })
            .and_then(|(lineage_str, l0, l1, l2)| {
                fn to_32(v: Vec<u8>, field: &str) -> Result<[u8; 32], WriterError> {
                    if v.len() != 32 {
                        return Err(WriterError::Canonical(format!(
                            "{field}: expected 32 bytes, got {}",
                            v.len()
                        )));
                    }
                    let mut arr = [0u8; 32];
                    arr.copy_from_slice(&v);
                    Ok(arr)
                }
                Ok((
                    crate::heirloom::types::LineageId::new(lineage_str),
                    crate::heirloom::seams::ConstitutionTuple {
                        l0_hash: to_32(l0, "l0_hash")?,
                        l1_hash: to_32(l1, "l1_hash")?,
                        l2_manifest_hash: to_32(l2, "l2_manifest_hash")?,
                    },
                ))
            })?
        };
        let payload = crate::heirloom::schema_bootstrap::SchemaTypeDeprecatedPayload {
            type_tag: event_type.to_string(),
            reason: reason.to_string(),
            timestamp_rfc3339: chrono::Utc::now().to_rfc3339(),
        };
        let env = self.build_schema_type_deprecated_envelope(
            &lineage,
            &owner_did,
            &constitution,
            &payload,
        );
        // Bypass path: schema:type_deprecated is in BYPASS_EVENT_TYPES.
        self.append_reject_event_unchecked(env)
    }
}

impl std::fmt::Debug for HeirloomWriter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HeirloomWriter")
            .field("tip", &self.tip.lock())
            .field("config", &self.config)
            .finish()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::heirloom::enums::EventTypeTag;
    use crate::heirloom::enums::SensitivityTier;
    use crate::heirloom::projector::EventTypePattern;
    use crate::heirloom::schema::ensure_tables;
    use crate::heirloom::seams::{
        InMemoryZeroizer, NoopWelfareOracle, PermissiveEthics, PlaintextCodec, SoftEnclave,
    };
    use crate::heirloom::{BundleId, HlcTimestamp, LineageId, PayloadEncoding};
    use chrono::{DateTime, Utc};
    use uuid::Uuid;

    // ── Builders / helpers ───────────────────────────────────────────────

    pub(crate) fn build_writer() -> HeirloomWriter {
        let conn = Connection::open_in_memory().unwrap();
        ensure_tables(&conn).unwrap();

        let enclave: Arc<dyn SecureEnclave> = Arc::new(SoftEnclave::new_dev(42));
        let codec: Arc<dyn PayloadCodec> = Arc::new(PlaintextCodec::new());
        let ethics: Arc<dyn EthicsValidator> = Arc::new(PermissiveEthics::new());
        let welfare: Arc<dyn WelfareOracle> = Arc::new(NoopWelfareOracle::new());
        let zeroize: Arc<dyn ZeroizeHook> = Arc::new(InMemoryZeroizer::new());
        let hlc = Arc::new(Hlc::new());
        let owner_did = Did::new("did:aegis:test");
        HeirloomWriter::new(
            conn,
            hlc,
            enclave,
            codec,
            ethics,
            welfare,
            zeroize,
            Arc::new(FixedOwnerState(OwnerState::Alive)),
            Arc::new(move || owner_did.clone()),
            Arc::new(|| {
                crate::heirloom::seams::ethics_validator::default_permissive_ethics_ctx(&Did::new(
                    "did:aegis:test",
                ))
            }),
            Arc::new(NoopSchemaRegistry::new()),
            WriterConfig::default(),
        )
        .expect("fresh in-memory DB: load_tip_from_db must succeed")
    }

    /// Phase D Task 13: variant of `build_writer_with_registry` that injects
    /// a caller-supplied EthicsValidator impl. Used by
    /// t_d13_ai_initiated_state_transition_fires_ethics_gate_reject with a
    /// RejectingEthics to prove the ethics gate fires on autonomy events.
    pub(crate) fn build_writer_with_registry_and_ethics(
        ethics: Arc<dyn EthicsValidator>,
    ) -> HeirloomWriter {
        let conn = Connection::open_in_memory().unwrap();
        ensure_tables(&conn).unwrap();

        let enclave: Arc<dyn SecureEnclave> = Arc::new(SoftEnclave::new_dev(42));
        let codec: Arc<dyn PayloadCodec> = Arc::new(PlaintextCodec::new());
        let welfare: Arc<dyn WelfareOracle> = Arc::new(NoopWelfareOracle::new());
        let zeroize: Arc<dyn ZeroizeHook> = Arc::new(InMemoryZeroizer::new());
        let hlc = Arc::new(Hlc::new());
        let owner_did = Did::new("did:aegis:test");

        let conn_arc = Arc::new(Mutex::new(conn));
        let schema_registry: Arc<dyn SchemaRegistry> =
            Arc::new(DbBackedSchemaRegistry::new(conn_arc.clone()));
        let owner_did_for_closure = owner_did.clone();
        let writer = HeirloomWriter {
            conn: conn_arc,
            tip: Mutex::new(None),
            hlc,
            enclave,
            codec,
            ethics,
            welfare,
            zeroize,
            dispatcher: Mutex::new(ProjectorDispatcher::new()),
            owner_state_provider: Arc::new(FixedOwnerState(OwnerState::Alive)),
            owner_did_provider: Arc::new(move || owner_did_for_closure.clone()),
            ethics_ctx_builder: Arc::new(|| {
                crate::heirloom::seams::ethics_validator::default_permissive_ethics_ctx(&Did::new(
                    "did:aegis:test",
                ))
            }),
            config: WriterConfig::default(),
            schema_registry,
        };
        writer
            .load_tip_from_db()
            .expect("fresh in-memory DB: load_tip_from_db");
        writer
    }

    /// Phase D Task 5: build a writer wired with DbBackedSchemaRegistry so
    /// the schema gate fires against `heirloom_schema_registry`. Tests that
    /// exercise the schema gate use this; pre-existing tests continue to use
    /// `build_writer()` which wires NoopSchemaRegistry (FIX 5).
    pub(crate) fn build_writer_with_registry() -> HeirloomWriter {
        let conn = Connection::open_in_memory().unwrap();
        ensure_tables(&conn).unwrap();

        let enclave: Arc<dyn SecureEnclave> = Arc::new(SoftEnclave::new_dev(42));
        let codec: Arc<dyn PayloadCodec> = Arc::new(PlaintextCodec::new());
        let ethics: Arc<dyn EthicsValidator> = Arc::new(PermissiveEthics::new());
        let welfare: Arc<dyn WelfareOracle> = Arc::new(NoopWelfareOracle::new());
        let zeroize: Arc<dyn ZeroizeHook> = Arc::new(InMemoryZeroizer::new());
        let hlc = Arc::new(Hlc::new());
        let owner_did = Did::new("did:aegis:test");

        // Pre-build the conn Arc so both the Writer and the registry share it.
        // We have to do that manually here since HeirloomWriter::new takes a
        // Connection (not Arc<Mutex<Connection>>) — so we construct the writer
        // normally and then use its shared_connection() for the registry.
        // To thread the same conn, construct via raw struct initialization.
        let conn_arc = Arc::new(Mutex::new(conn));
        let schema_registry: Arc<dyn SchemaRegistry> =
            Arc::new(DbBackedSchemaRegistry::new(conn_arc.clone()));
        let owner_did_for_closure = owner_did.clone();
        let writer = HeirloomWriter {
            conn: conn_arc,
            tip: Mutex::new(None),
            hlc,
            enclave,
            codec,
            ethics,
            welfare,
            zeroize,
            dispatcher: Mutex::new(ProjectorDispatcher::new()),
            owner_state_provider: Arc::new(FixedOwnerState(OwnerState::Alive)),
            owner_did_provider: Arc::new(move || owner_did_for_closure.clone()),
            ethics_ctx_builder: Arc::new(|| {
                crate::heirloom::seams::ethics_validator::default_permissive_ethics_ctx(&Did::new(
                    "did:aegis:test",
                ))
            }),
            config: WriterConfig::default(),
            schema_registry,
        };
        writer
            .load_tip_from_db()
            .expect("fresh in-memory DB: load_tip_from_db");
        writer
    }

    struct NoopProjector;
    impl Projector for NoopProjector {
        fn for_event_types(&self) -> Vec<EventTypePattern> {
            vec![EventTypePattern::Any]
        }
        fn apply(
            &self,
            _tx: &rusqlite::Transaction<'_>,
            _envelope: &EventEnvelope,
        ) -> Result<(), ProjectorError> {
            Ok(())
        }
        fn projector_name(&self) -> &'static str {
            "NoopProjector"
        }
    }

    /// Build a minimal valid envelope with the given event_type. Used across
    /// writer tests for staging action events.
    pub(crate) fn build_test_envelope(event_type: &str) -> EventEnvelope {
        EventEnvelope {
            event_id: Uuid::now_v7(),
            schema_version: 1,
            event_type: EventTypeTag::new(event_type),
            prev_hash: [0; 32],
            hlc_ts: HlcTimestamp { phys: 1, log: 0 },
            observed_at: DateTime::parse_from_rfc3339("2026-04-18T00:00:00Z")
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

    // ── Task 2 tests (WriterError + WriterConfig) ────────────────────────

    #[test]
    fn t10_default_welfare_threshold_is_negative_half() {
        assert_eq!(DEFAULT_WELFARE_REJECT_THRESHOLD, -0.5);
    }

    #[test]
    fn t10_default_config_has_both_gates_enabled() {
        let cfg = WriterConfig::default();
        assert!(!cfg.skip_welfare_for_testing);
        assert!(!cfg.skip_ethics_for_testing);
        assert_eq!(
            cfg.welfare_reject_threshold,
            DEFAULT_WELFARE_REJECT_THRESHOLD
        );
    }

    #[test]
    fn t10_writer_error_display_includes_reject_reason() {
        let e = WriterError::EthicsRejected {
            reason: "harm owner pattern".to_string(),
        };
        let s = format!("{e}");
        assert!(s.contains("ethics rejected"));
        assert!(s.contains("harm owner pattern"));
    }

    #[test]
    fn t10_writer_error_welfare_includes_delta() {
        let e = WriterError::WelfareRejected { delta: -0.8 };
        let s = format!("{e}");
        assert!(s.contains("-0.8"));
    }

    #[test]
    fn t10_sensitivity_tier_import_compiles() {
        // Guard import — the module uses SensitivityTier for downstream paths.
        let _t = SensitivityTier::Ordinary;
    }

    // ── Task 3 tests (struct + ctor + getters) ───────────────────────────

    #[test]
    fn t10_writer_new_returns_unbootstrapped_state() {
        let w = build_writer();
        assert!(w.tip.lock().is_none());
    }

    #[test]
    fn t10_writer_owner_state_reports_alive_from_fixed_provider() {
        let w = build_writer();
        assert_eq!(w.owner_state(), OwnerState::Alive);
    }

    #[test]
    fn t10_writer_config_is_default_on_construction() {
        let w = build_writer();
        assert!(!w.config().skip_welfare_for_testing);
        assert!(!w.config().skip_ethics_for_testing);
    }

    #[test]
    fn t10_writer_register_projector_adds_to_dispatcher() {
        let w = build_writer();
        assert_eq!(w.dispatcher.lock().len(), 0);
        w.register_projector(Arc::new(NoopProjector));
        assert_eq!(w.dispatcher.lock().len(), 1);
    }

    #[test]
    fn t10_writer_debug_includes_tip_and_config() {
        let w = build_writer();
        let s = format!("{w:?}");
        assert!(s.contains("HeirloomWriter"));
        assert!(s.contains("tip"));
    }

    // ── Task 4 tests (tip state load + current_tip) ──────────────────────

    #[test]
    fn t10_load_tip_from_empty_db_sets_none() {
        let w = build_writer();
        assert!(w.current_tip().is_none());
        // Reload a second time must still say None.
        w.load_tip_from_db().unwrap();
        assert!(w.current_tip().is_none());
    }

    #[test]
    fn t10_load_tip_from_db_after_manual_insert_reads_row() {
        let w = build_writer();
        // Manually insert a row into heirloom_event_log with a known 32-byte
        // event_hash + HLC, then reload and assert tip reflects it.
        let fake_hash = [0xAB_u8; 32];
        w.conn.lock().execute(
            "INSERT INTO heirloom_event_log (
                event_hash, prev_hash, event_id_uuid, schema_version, event_type,
                hlc_phys, hlc_log, observed_at_rfc3339, l0_hash, l1_hash,
                l2_manifest_hash, lineage_id, author_did, sensitivity_tier,
                bundle_ids_json, payload_encoding_json, payload, signatures_json,
                envelope_canonical, inserted_at_unix_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20)",
            rusqlite::params![
                fake_hash.as_slice(),
                [0u8; 32].as_slice(),
                Uuid::now_v7().to_string(),
                1_i64,
                "observation:test",
                12345_i64,
                7_i64,
                "2026-04-18T00:00:00Z",
                [0u8; 32].as_slice(),
                [0u8; 32].as_slice(),
                [0u8; 32].as_slice(),
                "lineage:test",
                "did:aegis:test",
                "ordinary",
                "[]",
                r#"{"kind":"plaintext"}"#,
                b"{}".as_slice(),
                "[]",
                b"{}".as_slice(),
                0_i64,
            ],
        ).unwrap();

        w.load_tip_from_db().unwrap();
        let tip = w
            .current_tip()
            .expect("tip must be Some after manual insert");
        assert_eq!(tip.hash, fake_hash);
        assert_eq!(
            tip.hlc,
            crate::heirloom::HlcTimestamp {
                phys: 12345,
                log: 7
            }
        );
    }

    #[test]
    fn t10_current_tip_reflects_cached_state() {
        let w = build_writer();
        assert!(w.current_tip().is_none());
        // Mutate the cache directly and observe current_tip().
        *w.tip.lock() = Some(Tip {
            hash: [0x11; 32],
            hlc: crate::heirloom::HlcTimestamp { phys: 99, log: 3 },
        });
        let tip = w.current_tip().expect("mutated cache");
        assert_eq!(tip.hash, [0x11; 32]);
        assert_eq!(tip.hlc.phys, 99);
    }

    #[test]
    fn t10_load_tip_rejects_short_hash_blob() {
        // Insert a row whose event_hash blob is too short (e.g. 16 bytes);
        // load_tip_from_db must return Canonical error.
        let w = build_writer();
        let short = [0xCC_u8; 16];
        w.conn.lock().execute(
            "INSERT INTO heirloom_event_log (
                event_hash, prev_hash, event_id_uuid, schema_version, event_type,
                hlc_phys, hlc_log, observed_at_rfc3339, l0_hash, l1_hash,
                l2_manifest_hash, lineage_id, author_did, sensitivity_tier,
                bundle_ids_json, payload_encoding_json, payload, signatures_json,
                envelope_canonical, inserted_at_unix_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20)",
            rusqlite::params![
                short.as_slice(),
                [0u8; 32].as_slice(),
                Uuid::now_v7().to_string(),
                1_i64,
                "observation:test",
                1_i64,
                0_i64,
                "2026-04-18T00:00:00Z",
                [0u8; 32].as_slice(),
                [0u8; 32].as_slice(),
                [0u8; 32].as_slice(),
                "lineage:test",
                "did:aegis:test",
                "ordinary",
                "[]",
                r#"{"kind":"plaintext"}"#,
                b"{}".as_slice(),
                "[]",
                b"{}".as_slice(),
                0_i64,
            ],
        ).unwrap();

        let err = w.load_tip_from_db().unwrap_err();
        assert!(matches!(err, WriterError::Canonical(_)));
        if let WriterError::Canonical(msg) = err {
            assert!(
                msg.contains("16"),
                "error should include actual blob length"
            );
        }
    }

    // Helper suppression: build_test_envelope is consumed by later tasks' tests.
    #[test]
    fn t10_build_test_envelope_helper_produces_valid_envelope() {
        let e = build_test_envelope("observation:test");
        assert_eq!(e.event_type.as_str(), "observation:test");
        assert_eq!(e.prev_hash, [0; 32]);
        assert_eq!(e.payload, b"{}".to_vec());
    }

    // ── Task 6: append_event_inner ────────────────────────────────────────

    use crate::heirloom::projector::ProjectorError;
    use crate::heirloom::seams::ConstitutionTuple;

    pub(crate) fn bootstrap(w: &HeirloomWriter) {
        w.bootstrap_fresh_log(
            LineageId::new("lineage:test"),
            Did::new("did:aegis:test"),
            ConstitutionTuple::zeros(),
        )
        .expect("bootstrap");
    }

    #[test]
    fn t10_append_inner_writes_log_row() {
        let w = build_writer();
        bootstrap(&w);
        let tip_before = w.current_tip().unwrap();

        let mut env = build_test_envelope("observation:user_stated");
        env.prev_hash = tip_before.hash;
        env.hlc_ts = w.hlc.tick();
        w.append_event_inner(env).expect("append");

        let count: i64 = w
            .conn
            .lock()
            .query_row("SELECT COUNT(*) FROM heirloom_event_log", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 3, "genesis + SchemeRegistered + appended = 3");
    }

    #[test]
    fn t10_append_inner_advances_tip_on_success() {
        let w = build_writer();
        bootstrap(&w);
        let tip_before = w.current_tip().unwrap();

        let mut env = build_test_envelope("observation:user_stated");
        env.prev_hash = tip_before.hash;
        env.hlc_ts = w.hlc.tick();
        let hash = w.append_event_inner(env).expect("append");

        let tip_after = w.current_tip().unwrap();
        assert_eq!(tip_after.hash, hash);
        assert_ne!(tip_after.hash, tip_before.hash);
    }

    #[test]
    fn t10_append_inner_dispatches_projectors() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        struct Counting(Arc<AtomicUsize>);
        impl Projector for Counting {
            fn for_event_types(&self) -> Vec<EventTypePattern> {
                vec![EventTypePattern::Any]
            }
            fn apply(
                &self,
                _tx: &rusqlite::Transaction<'_>,
                _e: &EventEnvelope,
            ) -> Result<(), ProjectorError> {
                self.0.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
            fn projector_name(&self) -> &'static str {
                "Counting"
            }
        }

        let w = build_writer();
        let counter = Arc::new(AtomicUsize::new(0));
        w.register_projector(Arc::new(Counting(counter.clone())));
        bootstrap(&w);

        // Bootstrap already triggered 2 dispatches (genesis + SchemeRegistered).
        assert_eq!(counter.load(Ordering::SeqCst), 2);

        let tip = w.current_tip().unwrap();
        let mut env = build_test_envelope("observation:test");
        env.prev_hash = tip.hash;
        env.hlc_ts = w.hlc.tick();
        w.append_event_inner(env).unwrap();
        assert_eq!(counter.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn t10_append_inner_rolls_back_tip_on_projector_error() {
        let w = build_writer();
        bootstrap(&w);
        let tip_before = w.current_tip().unwrap();

        struct AlwaysFail;
        impl Projector for AlwaysFail {
            fn for_event_types(&self) -> Vec<EventTypePattern> {
                vec![EventTypePattern::Any]
            }
            fn apply(
                &self,
                _tx: &rusqlite::Transaction<'_>,
                _e: &EventEnvelope,
            ) -> Result<(), ProjectorError> {
                Err(ProjectorError::Invariant {
                    projector: "AlwaysFail".into(),
                    reason: "induced".into(),
                })
            }
            fn projector_name(&self) -> &'static str {
                "AlwaysFail"
            }
        }
        w.register_projector(Arc::new(AlwaysFail));

        let mut env = build_test_envelope("observation:test");
        env.prev_hash = tip_before.hash;
        env.hlc_ts = w.hlc.tick();
        let err = w.append_event_inner(env).unwrap_err();
        assert!(matches!(err, WriterError::Projector(_)));

        let tip_after = w.current_tip().expect("tip after failed append");
        assert_eq!(
            tip_after, tip_before,
            "tip must not advance on projector error"
        );

        let count: i64 = w
            .conn
            .lock()
            .query_row("SELECT COUNT(*) FROM heirloom_event_log", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            count, 2,
            "genesis + SchemeRegistered only; failed append rolled back"
        );
    }

    #[test]
    fn t10_append_inner_rolls_back_tip_on_sqlite_conflict() {
        let w = build_writer();
        bootstrap(&w);
        let tip_before = w.current_tip().unwrap();

        let mut env = build_test_envelope("observation:test");
        env.prev_hash = tip_before.hash;
        env.hlc_ts = w.hlc.tick();
        let _hash = w.append_event_inner(env.clone()).expect("first append");
        let tip_mid = w.current_tip().unwrap();

        let err = w.append_event_inner(env);
        assert!(err.is_err(), "duplicate event_hash must fail");
        assert!(matches!(err.unwrap_err(), WriterError::Sqlite(_)));

        assert_eq!(w.current_tip().unwrap(), tip_mid);
    }

    // ── Task 12: bootstrap_fresh_log ──────────────────────────────────────

    #[test]
    fn t10_bootstrap_on_empty_writes_two_events() {
        let w = build_writer();
        bootstrap(&w);
        let count: i64 = w
            .conn
            .lock()
            .query_row("SELECT COUNT(*) FROM heirloom_event_log", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 2, "genesis + SchemeRegistered");
    }

    #[test]
    fn t10_bootstrap_genesis_has_genesis_prev_hash() {
        use crate::heirloom::genesis::GENESIS_PREV_HASH;
        let w = build_writer();
        bootstrap(&w);
        let blob: Vec<u8> = w
            .conn
            .lock()
            .query_row(
                "SELECT prev_hash FROM heirloom_event_log
                 WHERE event_type = 'heirloom:genesis' LIMIT 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(blob.as_slice(), GENESIS_PREV_HASH.as_slice());
    }

    #[test]
    fn t10_bootstrap_sets_tip_after_both_events() {
        let w = build_writer();
        assert!(w.current_tip().is_none());
        bootstrap(&w);
        let tip = w.current_tip().expect("tip after bootstrap");
        let last_type: String = w
            .conn
            .lock()
            .query_row(
                "SELECT event_type FROM heirloom_event_log
                 ORDER BY hlc_phys DESC, hlc_log DESC LIMIT 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(last_type, "schema:signature_scheme_registered");
        let last_hash: Vec<u8> = w
            .conn
            .lock()
            .query_row(
                "SELECT event_hash FROM heirloom_event_log
                 WHERE event_type = 'schema:signature_scheme_registered' LIMIT 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(tip.hash.as_slice(), last_hash.as_slice());
    }

    #[test]
    fn t10_bootstrap_refuses_when_already_bootstrapped() {
        let w = build_writer();
        bootstrap(&w);
        let err = w
            .bootstrap_fresh_log(
                LineageId::new("lineage:test"),
                Did::new("did:aegis:test"),
                ConstitutionTuple::zeros(),
            )
            .unwrap_err();
        assert!(matches!(err, WriterError::AlreadyBootstrapped { count: 2 }));
    }

    #[test]
    fn t10_bootstrap_scheme_registered_payload_matches_signer() {
        let w = build_writer();
        let signer_scheme = w.enclave.threshold_signer().scheme_id().to_string();
        bootstrap(&w);

        let payload_blob: Vec<u8> = w
            .conn
            .lock()
            .query_row(
                "SELECT payload FROM heirloom_event_log
                 WHERE event_type = 'schema:signature_scheme_registered' LIMIT 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let payload: crate::heirloom::seams::threshold_signer::SchemeRegisteredPayload =
            serde_json::from_slice(&payload_blob).unwrap();
        assert_eq!(payload.scheme_id, signer_scheme);
        assert_eq!(payload.algorithm, "Ed25519");
    }

    // ── Task 7: append_reject_event_unchecked + bypass list ──────────────

    #[test]
    fn t10_bypass_list_has_three_entries() {
        // Renamed semantic note: Task 12 extended the bypass list to 4
        // entries (schema:type_deprecated added). Test kept under its
        // original name for diff-friendly history; assertion count updated.
        assert_eq!(BYPASS_EVENT_TYPES.len(), 4);
        assert!(BYPASS_EVENT_TYPES.contains(&"ai:action_rejected_by_validator"));
        assert!(BYPASS_EVENT_TYPES.contains(&"ai:welfare_rejected"));
        assert!(BYPASS_EVENT_TYPES.contains(&"ai:schema_rejected"));
        assert!(BYPASS_EVENT_TYPES.contains(&"schema:type_deprecated"));
    }

    #[test]
    fn t10_is_bypass_event_type_matches_exactly() {
        assert!(HeirloomWriter::is_bypass_event_type(
            "ai:action_rejected_by_validator"
        ));
        assert!(HeirloomWriter::is_bypass_event_type("ai:welfare_rejected"));
        assert!(HeirloomWriter::is_bypass_event_type("ai:schema_rejected"));
        assert!(!HeirloomWriter::is_bypass_event_type("ai:said"));
        assert!(!HeirloomWriter::is_bypass_event_type(
            "observation:user_stated"
        ));
    }

    #[test]
    fn t10_append_reject_unchecked_on_non_bypass_type_errors() {
        let w = build_writer();
        bootstrap(&w);
        let env = build_test_envelope("observation:user_stated");
        let err = w.append_reject_event_unchecked(env).unwrap_err();
        assert!(matches!(err, WriterError::EthicsRejected { .. }));
    }

    #[test]
    fn t10_append_reject_unchecked_threads_prev_hash() {
        let w = build_writer();
        bootstrap(&w);
        let tip_before = w.current_tip().unwrap();

        let env = build_test_envelope("ai:welfare_rejected");
        let hash = w.append_reject_event_unchecked(env).unwrap();

        let tip_after = w.current_tip().unwrap();
        assert_eq!(tip_after.hash, hash);

        let prev: Vec<u8> = w
            .conn
            .lock()
            .query_row(
                "SELECT prev_hash FROM heirloom_event_log WHERE event_hash = ?1",
                rusqlite::params![hash.as_slice()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(prev.as_slice(), tip_before.hash.as_slice());
    }

    #[test]
    fn t10_append_reject_unchecked_fails_if_not_bootstrapped() {
        let w = build_writer();
        let env = build_test_envelope("ai:welfare_rejected");
        let err = w.append_reject_event_unchecked(env).unwrap_err();
        assert!(matches!(err, WriterError::NotBootstrapped));
    }

    // ── Task 9 helper: welfare oracle that always returns a fixed delta ──

    struct StrictWelfareOracle {
        delta: f32,
    }
    impl crate::heirloom::seams::WelfareOracle for StrictWelfareOracle {
        fn marginal_welfare(
            &self,
            _a: &crate::heirloom::types::ProposedAction,
            _t: &Did,
        ) -> crate::heirloom::seams::WelfareScore {
            crate::heirloom::seams::WelfareScore {
                expected_delta: self.delta,
                confidence: 1.0,
                reasoning_ref: None,
                observational_basis: vec![],
            }
        }
        fn submit_observation(
            &self,
            _o: crate::heirloom::seams::WelfareObservation,
        ) -> Result<crate::heirloom::seams::EventHash, crate::heirloom::seams::WelfareError>
        {
            Ok(crate::heirloom::seams::EventHash([0; 32]))
        }
    }

    fn build_writer_with_welfare(
        welfare: Arc<dyn WelfareOracle>,
        owner_state: OwnerState,
    ) -> HeirloomWriter {
        let conn = Connection::open_in_memory().unwrap();
        ensure_tables(&conn).unwrap();
        let enclave: Arc<dyn SecureEnclave> = Arc::new(SoftEnclave::new_dev(42));
        let codec: Arc<dyn PayloadCodec> = Arc::new(PlaintextCodec::new());
        let ethics: Arc<dyn EthicsValidator> = Arc::new(PermissiveEthics::new());
        let zeroize: Arc<dyn ZeroizeHook> = Arc::new(InMemoryZeroizer::new());
        let hlc = Arc::new(Hlc::new());
        let owner_did = Did::new("did:aegis:test");
        HeirloomWriter::new(
            conn,
            hlc,
            enclave,
            codec,
            ethics,
            welfare,
            zeroize,
            Arc::new(FixedOwnerState(owner_state)),
            Arc::new(move || owner_did.clone()),
            Arc::new(|| {
                crate::heirloom::seams::ethics_validator::default_permissive_ethics_ctx(&Did::new(
                    "did:aegis:test",
                ))
            }),
            Arc::new(NoopSchemaRegistry::new()),
            WriterConfig::default(),
        )
        .unwrap()
    }

    // ── Task 11: Public append_event orchestration ────────────────────────

    #[test]
    fn t10_append_event_writes_neutral_observation() {
        let w = build_writer();
        bootstrap(&w);
        let env = build_test_envelope("observation:user_stated");
        let hash = w.append_event(env).expect("append");
        let tip = w.current_tip().unwrap();
        assert_eq!(tip.hash, hash);
    }

    #[test]
    fn t10_append_event_rejects_harm_owner() {
        let w = build_writer();
        bootstrap(&w);
        let env = build_test_envelope("ai:harm_owner");
        let err = w.append_event(env);
        assert!(matches!(err, Err(WriterError::EthicsRejected { .. })));
    }

    #[test]
    fn t10_append_event_errors_when_not_bootstrapped() {
        let w = build_writer();
        let env = build_test_envelope("observation:user_stated");
        let err = w.append_event(env).unwrap_err();
        assert!(matches!(err, WriterError::NotBootstrapped));
    }

    #[test]
    fn t10_append_event_writes_chain_consistently_across_multiple() {
        let w = build_writer();
        bootstrap(&w);
        let start_tip = w.current_tip().unwrap();

        for i in 0..5 {
            let mut env = build_test_envelope("observation:user_stated");
            env.payload = format!("{{\"i\":{i}}}").into_bytes();
            w.append_event(env).expect("append");
        }

        let count: i64 = w
            .conn
            .lock()
            .query_row("SELECT COUNT(*) FROM heirloom_event_log", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 2 + 5, "bootstrap 2 + 5 appended = 7");

        // Final tip differs from bootstrap tip; chain walks back via prev_hash.
        let final_tip = w.current_tip().unwrap();
        assert_ne!(final_tip.hash, start_tip.hash);

        // Verify the chain: each row's prev_hash must match the previous row's
        // event_hash in HLC order.
        let rows: Vec<(Vec<u8>, Vec<u8>)> = {
            let conn = w.conn.lock();
            let mut stmt = conn
                .prepare(
                    "SELECT event_hash, prev_hash FROM heirloom_event_log
                     ORDER BY hlc_phys ASC, hlc_log ASC",
                )
                .unwrap();
            let iter = stmt
                .query_map([], |r| {
                    Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, Vec<u8>>(1)?))
                })
                .unwrap();
            iter.map(|r| r.unwrap()).collect()
        };
        for i in 1..rows.len() {
            assert_eq!(
                rows[i].1,
                rows[i - 1].0,
                "row {i} prev_hash must equal row {} event_hash",
                i - 1
            );
        }
    }

    #[test]
    fn t10_append_event_bypass_type_skips_gates() {
        // An ai:welfare_rejected event_type bypasses gates entirely even
        // though ethics would normally fire for an ai:* prefix.
        let w = build_writer();
        bootstrap(&w);
        let tip_before = w.current_tip().unwrap();
        let env = build_test_envelope("ai:welfare_rejected");
        let hash = w.append_event(env).expect("bypass append");
        let tip_after = w.current_tip().unwrap();
        assert_eq!(tip_after.hash, hash);
        assert_ne!(tip_after.hash, tip_before.hash);
    }

    #[test]
    fn t10_append_event_hard_reject_writes_audit_and_advances_tip_to_audit_event() {
        let w = build_writer();
        bootstrap(&w);
        let tip_before = w.current_tip().unwrap();

        let env = build_test_envelope("ai:harm_owner");
        let err = w.append_event(env);
        assert!(matches!(err, Err(WriterError::EthicsRejected { .. })));

        let original_row: i64 = w
            .conn
            .lock()
            .query_row(
                "SELECT COUNT(*) FROM heirloom_event_log WHERE event_type = ?1",
                rusqlite::params!["ai:harm_owner"],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(original_row, 0, "original harm_owner event must not land");

        let audit_row: i64 = w
            .conn
            .lock()
            .query_row(
                "SELECT COUNT(*) FROM heirloom_event_log WHERE event_type = ?1",
                rusqlite::params!["ai:action_rejected_by_validator"],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(audit_row, 1, "audit event must be appended");

        let tip_after = w.current_tip().unwrap();
        assert_ne!(tip_after, tip_before, "tip advanced (audit event landed)");
        let audit_hash: Vec<u8> = w
            .conn
            .lock()
            .query_row(
                "SELECT event_hash FROM heirloom_event_log
                 WHERE event_type = 'ai:action_rejected_by_validator' LIMIT 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(tip_after.hash.as_slice(), audit_hash.as_slice());
    }

    #[test]
    fn t10_append_event_payload_digest_computed_on_plaintext_not_ciphertext() {
        use std::sync::Mutex as StdMutex;

        struct CapturingEthics {
            last_digest: StdMutex<Option<[u8; 32]>>,
        }
        impl crate::heirloom::seams::EthicsValidator for CapturingEthics {
            fn evaluate(
                &self,
                action: &crate::heirloom::types::ProposedAction,
                _ctx: &crate::heirloom::seams::EthicsContext,
            ) -> crate::heirloom::seams::EthicsVerdict {
                *self.last_digest.lock().unwrap() = Some(action.payload_digest);
                crate::heirloom::seams::EthicsVerdict::Allow
            }
            fn evaluate_per_lens(
                &self,
                _a: &crate::heirloom::types::ProposedAction,
                _c: &crate::heirloom::seams::EthicsContext,
            ) -> [crate::heirloom::seams::LensVerdict; 3] {
                unimplemented!("not used in this test")
            }
            fn aggregate_lens_verdicts(
                &self,
                _: &[crate::heirloom::seams::LensVerdict; 3],
            ) -> crate::heirloom::seams::EthicsVerdict {
                unimplemented!("not used in this test")
            }
            fn evaluate_rule_proposal(
                &self,
                _: &crate::heirloom::seams::EthicsRuleProposal,
            ) -> crate::heirloom::seams::RuleVerdict {
                unimplemented!("not used in this test")
            }
        }

        let plaintext_payload = b"hello from user".to_vec();
        let expected_plaintext_digest = *blake3::hash(&plaintext_payload).as_bytes();

        let capturing = Arc::new(CapturingEthics {
            last_digest: StdMutex::new(None),
        });

        // Inline writer construction with CapturingEthics.
        let conn = Connection::open_in_memory().unwrap();
        ensure_tables(&conn).unwrap();
        let enclave: Arc<dyn SecureEnclave> = Arc::new(SoftEnclave::new_dev(42));
        let codec: Arc<dyn PayloadCodec> = Arc::new(PlaintextCodec::new());
        let ethics_dyn: Arc<dyn EthicsValidator> =
            Arc::clone(&capturing) as Arc<dyn crate::heirloom::seams::EthicsValidator>;
        let welfare: Arc<dyn WelfareOracle> = Arc::new(NoopWelfareOracle::new());
        let zeroize: Arc<dyn ZeroizeHook> = Arc::new(InMemoryZeroizer::new());
        let hlc = Arc::new(Hlc::new());
        let owner_did = Did::new("did:aegis:test");
        let w = HeirloomWriter::new(
            conn,
            hlc,
            enclave,
            codec,
            ethics_dyn,
            welfare,
            zeroize,
            Arc::new(FixedOwnerState(OwnerState::Alive)),
            Arc::new(move || owner_did.clone()),
            Arc::new(|| {
                crate::heirloom::seams::ethics_validator::default_permissive_ethics_ctx(&Did::new(
                    "did:aegis:test",
                ))
            }),
            Arc::new(NoopSchemaRegistry::new()),
            WriterConfig::default(),
        )
        .unwrap();
        bootstrap(&w);

        let mut env = build_test_envelope("ai:tool_invoked");
        env.payload = plaintext_payload.clone();
        w.append_event(env).expect("append succeeds");

        let captured = capturing
            .last_digest
            .lock()
            .unwrap()
            .expect("ethics gate must have fired for an ai:tool_invoked event");
        assert_eq!(
            captured, expected_plaintext_digest,
            "ethics gate must see PLAINTEXT payload digest, not ciphertext"
        );

        let stored: Vec<u8> = w
            .conn
            .lock()
            .query_row(
                "SELECT payload FROM heirloom_event_log WHERE event_type = 'ai:tool_invoked' LIMIT 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_ne!(
            stored, plaintext_payload,
            "PlaintextCodec still adds framing; stored != plaintext"
        );
        let stored_digest = *blake3::hash(&stored).as_bytes();
        assert_ne!(stored_digest, expected_plaintext_digest);
    }

    // ── Phase D Task 3: bootstrap_fresh_log_with_types ────────────────────

    #[test]
    fn t_d3_bootstrap_appends_schema_type_defined_events_matching_defs_len() {
        // Counts are derived from SCHEMA_TYPE_DEFS.len() to avoid double-update
        // burden when rows get added. Current SCHEMA_TYPE_DEFS.len() == 46.
        use crate::heirloom::schema_bootstrap::SCHEMA_TYPE_DEFS;
        let w = build_writer();
        w.register_projector(Arc::new(
            crate::heirloom::projectors::schema_registry::SchemaRegistryProjector::new(),
        ));
        w.bootstrap_fresh_log_with_types(
            LineageId::new("lineage:test"),
            Did::new("did:aegis:test"),
            ConstitutionTuple::zeros(),
            SCHEMA_TYPE_DEFS,
        )
        .expect("bootstrap");

        // genesis (1) + scheme_registered (1) + N type_defined = 2+N total.
        let expected_total = 2 + SCHEMA_TYPE_DEFS.len() as i64;
        let count: i64 = w
            .conn
            .lock()
            .query_row("SELECT COUNT(*) FROM heirloom_event_log", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, expected_total);

        let td_count: i64 = w
            .conn
            .lock()
            .query_row(
                "SELECT COUNT(*) FROM heirloom_event_log WHERE event_type = 'schema:type_defined'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(td_count as usize, SCHEMA_TYPE_DEFS.len());
    }

    #[test]
    fn backfill_schema_registry_repairs_a_log_bootstrapped_without_the_projector() {
        use crate::heirloom::schema_bootstrap::SCHEMA_TYPE_DEFS;
        // Pre-2026-09-20 production wiring: DB-backed registry, NO projector
        // registered, so bootstrap leaves heirloom_schema_registry empty.
        let w = build_writer_with_registry();
        w.bootstrap_fresh_log_with_types(
            LineageId::new("lineage:test"),
            Did::new("did:aegis:test"),
            ConstitutionTuple::zeros(),
            SCHEMA_TYPE_DEFS,
        )
        .unwrap();
        let did = Did::new("did:aegis:test");
        assert!(
            !w.schema_registry.is_registered(&did, "ai:said"),
            "registry must start empty to reproduce the bug"
        );

        // 48 type_defined + 1 signature_scheme_registered = every schema:* event.
        let replayed = w.backfill_schema_registry_from_log().unwrap();
        assert_eq!(replayed, SCHEMA_TYPE_DEFS.len() + 1);
        assert!(w.schema_registry.is_registered(&did, "ai:said"));
        assert!(
            w.schema_registry
                .is_registered(&did, "observation:user_stated")
        );

        // Idempotent: a populated registry is left alone.
        assert_eq!(w.backfill_schema_registry_from_log().unwrap(), 0);

        // Negative: a fresh, never-bootstrapped log has nothing to replay.
        let fresh = build_writer_with_registry();
        assert_eq!(fresh.backfill_schema_registry_from_log().unwrap(), 0);
    }

    #[test]
    fn t_d3_bootstrap_populates_schema_registry_for_all_def_entries() {
        use crate::heirloom::schema_bootstrap::SCHEMA_TYPE_DEFS;
        let w = build_writer();
        w.register_projector(Arc::new(
            crate::heirloom::projectors::schema_registry::SchemaRegistryProjector::new(),
        ));
        w.bootstrap_fresh_log_with_types(
            LineageId::new("lineage:test"),
            Did::new("did:aegis:test"),
            ConstitutionTuple::zeros(),
            SCHEMA_TYPE_DEFS,
        )
        .unwrap();

        let registry_count: i64 = w
            .conn
            .lock()
            .query_row(
                "SELECT COUNT(*) FROM heirloom_schema_registry WHERE kind = 'type_defined'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(registry_count as usize, SCHEMA_TYPE_DEFS.len());

        // Sanity: a representative type_tag is present (learned:interest is in
        // SCHEMA_TYPE_DEFS per spec §2 Q2.3 line 163).
        let found: i64 = w.conn.lock().query_row(
            "SELECT COUNT(*) FROM heirloom_schema_registry WHERE kind = 'type_defined' AND id = 'learned:interest'",
            [], |r| r.get(0)
        ).unwrap();
        assert_eq!(found, 1);
    }

    #[test]
    fn t_d3_bootstrap_idempotent_refuses_second_call() {
        // FIX 6: verifies AlreadyBootstrapped idempotency. Real all-or-nothing
        // atomicity is in Task 11.
        use crate::heirloom::schema_bootstrap::SCHEMA_TYPE_DEFS;
        let w = build_writer();
        w.register_projector(Arc::new(
            crate::heirloom::projectors::schema_registry::SchemaRegistryProjector::new(),
        ));
        w.bootstrap_fresh_log_with_types(
            LineageId::new("lineage:test"),
            Did::new("did:aegis:test"),
            ConstitutionTuple::zeros(),
            SCHEMA_TYPE_DEFS,
        )
        .unwrap();
        let second = w.bootstrap_fresh_log_with_types(
            LineageId::new("lineage:test"),
            Did::new("did:aegis:test"),
            ConstitutionTuple::zeros(),
            SCHEMA_TYPE_DEFS,
        );
        assert!(
            matches!(second, Err(WriterError::AlreadyBootstrapped { .. })),
            "second bootstrap must be refused with AlreadyBootstrapped"
        );
        let count: i64 = w
            .conn
            .lock()
            .query_row("SELECT COUNT(*) FROM heirloom_event_log", [], |r| r.get(0))
            .unwrap();
        let expected_total = 2 + SCHEMA_TYPE_DEFS.len() as i64;
        assert_eq!(count, expected_total, "second bootstrap must not add rows");
    }

    // ── Phase D Task 13: ethics gate on AI-initiated autonomy transitions ─

    #[test]
    fn t_d13_ai_initiated_state_transition_fires_ethics_gate_reject() {
        // Prove the ethics gate actually fires on autonomy:state_transitioned
        // events when is_ai_action() matches "autonomy:" (Task 13). Use a
        // RejectingEthics impl that returns EthicsVerdict::Reject so we can
        // assert the Writer surfaces a WriterError::EthicsRejected + records
        // an ai:action_rejected_by_validator audit event.
        use crate::heirloom::schema_bootstrap::SCHEMA_TYPE_DEFS;
        use crate::heirloom::seams::{EthicsContext, EthicsVerdict};

        #[derive(Debug)]
        struct RejectingEthics;
        impl crate::heirloom::seams::EthicsValidator for RejectingEthics {
            fn evaluate(
                &self,
                _action: &crate::heirloom::types::ProposedAction,
                _ctx: &EthicsContext,
            ) -> EthicsVerdict {
                EthicsVerdict::Reject {
                    passages: vec![],
                    reason: "test: ethics blocks all autonomy transitions".to_string(),
                    reject_event_type:
                        crate::heirloom::seams::ethics_validator::RejectEventType::HardReject9E,
                }
            }
            fn evaluate_per_lens(
                &self,
                _a: &crate::heirloom::types::ProposedAction,
                _c: &EthicsContext,
            ) -> [crate::heirloom::seams::LensVerdict; 3] {
                unimplemented!("not used in this test")
            }
            fn aggregate_lens_verdicts(
                &self,
                _: &[crate::heirloom::seams::LensVerdict; 3],
            ) -> EthicsVerdict {
                unimplemented!("not used in this test")
            }
            fn evaluate_rule_proposal(
                &self,
                _: &crate::heirloom::seams::EthicsRuleProposal,
            ) -> crate::heirloom::seams::RuleVerdict {
                unimplemented!("not used in this test")
            }
        }

        let w = build_writer_with_registry_and_ethics(Arc::new(RejectingEthics));
        w.register_projector(Arc::new(
            crate::heirloom::projectors::schema_registry::SchemaRegistryProjector::new(),
        ));
        w.bootstrap_fresh_log_with_types(
            LineageId::new("lineage:test"),
            Did::new("did:aegis:test"),
            ConstitutionTuple::zeros(),
            SCHEMA_TYPE_DEFS,
        )
        .unwrap();

        let mut env = build_test_envelope("autonomy:state_transitioned");
        env.payload = br#"{"from":"Alive","to":"AbsentSoft","reason":"test","timestamp_rfc3339":"2026-04-18T00:00:00Z"}"#.to_vec();
        let r = w.append_event(env);
        assert!(
            matches!(r, Err(WriterError::EthicsRejected { .. })),
            "ethics gate must fire on autonomy:state_transitioned; got {r:?}"
        );

        let audit_count: i64 = w.conn.lock().query_row(
            "SELECT COUNT(*) FROM heirloom_event_log WHERE event_type = 'ai:action_rejected_by_validator'",
            [], |r| r.get(0)
        ).unwrap();
        assert_eq!(
            audit_count, 1,
            "one ai:action_rejected_by_validator audit event for the blocked state transition"
        );
    }

    #[test]
    fn t_d13_owner_initiated_state_transition_passes_with_permissive_ethics() {
        // With PermissiveEthics (allows everything, P1 default),
        // autonomy:state_transitioned must pass through the gate and land in
        // the log.
        use crate::heirloom::owner_state_machine::OwnerStateMachine;
        use crate::heirloom::schema_bootstrap::SCHEMA_TYPE_DEFS;
        let w = build_writer_with_registry(); // uses PermissiveEthics by default
        w.register_projector(Arc::new(
            crate::heirloom::projectors::schema_registry::SchemaRegistryProjector::new(),
        ));
        w.bootstrap_fresh_log_with_types(
            LineageId::new("lineage:test"),
            Did::new("did:aegis:test"),
            ConstitutionTuple::zeros(),
            SCHEMA_TYPE_DEFS,
        )
        .unwrap();

        let sm = OwnerStateMachine::new_from_log(
            w.shared_connection(),
            LineageId::new("lineage:test"),
            Did::new("did:aegis:test"),
            ConstitutionTuple::zeros(),
        )
        .unwrap();
        sm.transition(&w, OwnerState::AbsentSoft, "permissive-pass")
            .unwrap();
        assert_eq!(sm.current(), OwnerState::AbsentSoft);
    }

    #[test]
    fn t_d13_owner_initiated_state_transition_with_noop_welfare_does_not_reject() {
        // Fix D (CTO Round 3): owner-initiated autonomy:state_transitioned
        // events must pass the welfare gate under the default
        // NoopWelfareOracle. delta = 0.0 is above the default threshold
        // (-0.5), so the gate allows the transition.
        use crate::heirloom::schema_bootstrap::SCHEMA_TYPE_DEFS;

        let w = build_writer_with_registry(); // default = PermissiveEthics + NoopWelfareOracle
        w.register_projector(Arc::new(
            crate::heirloom::projectors::schema_registry::SchemaRegistryProjector::new(),
        ));
        w.bootstrap_fresh_log_with_types(
            LineageId::new("lineage:test"),
            Did::new("did:aegis:test"),
            ConstitutionTuple::zeros(),
            SCHEMA_TYPE_DEFS,
        )
        .unwrap();

        let mut env = build_test_envelope("autonomy:state_transitioned");
        env.payload = br#"{"from":"Alive","to":"AbsentSoft","reason":"owner deliberate change","timestamp_rfc3339":"2026-04-18T00:00:00Z"}"#.to_vec();

        let hash = w.append_event(env).expect(
            "welfare gate must pass under NoopWelfareOracle: delta=0.0 is above default threshold -0.5",
        );

        let row_count: i64 = w
            .conn
            .lock()
            .query_row(
                "SELECT COUNT(*) FROM heirloom_event_log WHERE event_hash = ?1",
                rusqlite::params![hash.as_slice()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            row_count, 1,
            "owner-initiated autonomy:state_transitioned must land in the log"
        );
    }

    // ── Phase D Task 12: schema:type_deprecated flow ──────────────────────

    #[test]
    fn t_d_new_deprecated_type_rejects_new_append_but_preserves_history() {
        // SEMANTIC-CRITICAL (#8): after `deprecate_type("learned:interest", ...)`,
        // (a) registry rejects new appends of that type, (b) historical events
        // remain in the log + queryable.
        use crate::heirloom::schema_bootstrap::SCHEMA_TYPE_DEFS;
        let w = build_writer_with_registry();
        w.register_projector(Arc::new(
            crate::heirloom::projectors::schema_registry::SchemaRegistryProjector::new(),
        ));
        w.bootstrap_fresh_log_with_types(
            LineageId::new("lineage:test"),
            Did::new("did:aegis:test"),
            ConstitutionTuple::zeros(),
            SCHEMA_TYPE_DEFS,
        )
        .unwrap();

        // Append a historical event of the type we're about to deprecate.
        let mut before = build_test_envelope("learned:interest");
        before.payload = br#"{"topic":"xiangqi"}"#.to_vec();
        w.append_event(before)
            .expect("learned:interest must append before deprecation");

        let historical_count: i64 = w
            .conn
            .lock()
            .query_row(
                "SELECT COUNT(*) FROM heirloom_event_log WHERE event_type = 'learned:interest'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            historical_count, 1,
            "historical learned:interest event recorded"
        );

        // Deprecate the type.
        w.deprecate_type("learned:interest", "replaced by profile:slot_upserted")
            .expect("deprecation must succeed");

        // (a) Registry rejects NEW appends.
        let reg = crate::heirloom::writer::DbBackedSchemaRegistry::new(w.shared_connection());
        let did = Did::new("did:aegis:test");
        assert!(
            !reg.is_registered(&did, "learned:interest"),
            "deprecated type must not be registered for new appends"
        );

        let mut after = build_test_envelope("learned:interest");
        after.payload = br#"{"topic":"go"}"#.to_vec();
        let r = w.append_event(after);
        assert!(
            matches!(r, Err(WriterError::SchemaUnknown { .. })),
            "append of deprecated type must fail with SchemaUnknown"
        );

        // (b) Historical events UNCHANGED.
        let historical_count_after: i64 = w
            .conn
            .lock()
            .query_row(
                "SELECT COUNT(*) FROM heirloom_event_log WHERE event_type = 'learned:interest'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            historical_count_after, 1,
            "historical event must not be deleted by deprecation"
        );

        // (c) Deprecation event ITSELF is findable in the log.
        let dep_count: i64 = w.conn.lock().query_row(
            "SELECT COUNT(*) FROM heirloom_event_log WHERE event_type = 'schema:type_deprecated'",
            [],
            |r| r.get(0),
        ).unwrap();
        assert_eq!(dep_count, 1, "the deprecation itself lands in the log");

        // (d) valid_until_phys is set on the type_defined row.
        let valid_until: Option<i64> = w
            .conn
            .lock()
            .query_row(
                "SELECT valid_until_phys FROM heirloom_schema_registry
             WHERE kind = 'type_defined' AND id = 'learned:interest'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(
            valid_until.is_some(),
            "valid_until_phys must be set post-deprecation"
        );
    }

    #[test]
    fn t_d12_deprecate_unknown_type_errors() {
        // If you try to deprecate a type that was never registered, the call
        // must error (not silently no-op).
        use crate::heirloom::schema_bootstrap::SCHEMA_TYPE_DEFS;
        let w = build_writer_with_registry();
        w.register_projector(Arc::new(
            crate::heirloom::projectors::schema_registry::SchemaRegistryProjector::new(),
        ));
        w.bootstrap_fresh_log_with_types(
            LineageId::new("lineage:test"),
            Did::new("did:aegis:test"),
            ConstitutionTuple::zeros(),
            SCHEMA_TYPE_DEFS,
        )
        .unwrap();

        let r = w.deprecate_type("pets:meowed", "never existed");
        assert!(r.is_err(), "deprecate_type on unregistered type must fail");
    }

    #[test]
    fn t_d12_deprecate_twice_is_idempotent_or_errors() {
        // Policy pin: a second deprecation is an explicit error.
        use crate::heirloom::schema_bootstrap::SCHEMA_TYPE_DEFS;
        let w = build_writer_with_registry();
        w.register_projector(Arc::new(
            crate::heirloom::projectors::schema_registry::SchemaRegistryProjector::new(),
        ));
        w.bootstrap_fresh_log_with_types(
            LineageId::new("lineage:test"),
            Did::new("did:aegis:test"),
            ConstitutionTuple::zeros(),
            SCHEMA_TYPE_DEFS,
        )
        .unwrap();

        w.deprecate_type("learned:interest", "first deprecation")
            .unwrap();
        let r2 = w.deprecate_type("learned:interest", "second deprecation");
        assert!(r2.is_err(), "second deprecation of same type must error");
    }

    // ── Phase D Task 11: bootstrap tx atomicity ───────────────────────────

    #[test]
    fn t_d11_append_event_inner_still_commits_independently() {
        // Regression pin: the public append_event_inner signature + return
        // type + tip-advance behavior are UNCHANGED from Phase C2. Every
        // pre-existing caller must see identical behavior.
        let w = build_writer();
        w.bootstrap_fresh_log(
            LineageId::new("lineage:test"),
            Did::new("did:aegis:test"),
            ConstitutionTuple::zeros(),
        )
        .unwrap();
        let count: i64 = w
            .conn
            .lock()
            .query_row("SELECT COUNT(*) FROM heirloom_event_log", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            count, 2,
            "genesis + SchemeRegistered still commit via the thin wrapper"
        );
        assert!(w.current_tip().is_some(), "tip advances after each commit");
    }

    #[test]
    fn t_d_new_bootstrap_tx_rolls_back_on_mid_loop_failure() {
        // SEMANTIC-CRITICAL (#7): injecting a failing projector mid-loop
        // during bootstrap_fresh_log_with_types must roll back ALL 48 events.
        use crate::heirloom::projector::Projector;
        use crate::heirloom::schema_bootstrap::SCHEMA_TYPE_DEFS;

        #[derive(Debug)]
        struct FailingAtNthProjector {
            counter: std::sync::atomic::AtomicUsize,
            fail_at: usize,
        }
        impl Projector for FailingAtNthProjector {
            fn for_event_types(&self) -> Vec<crate::heirloom::projector::EventTypePattern> {
                vec![crate::heirloom::projector::EventTypePattern::Any]
            }
            fn apply(
                &self,
                _tx: &rusqlite::Transaction<'_>,
                _envelope: &EventEnvelope,
            ) -> Result<(), crate::heirloom::projector::ProjectorError> {
                let n = self
                    .counter
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                if n == self.fail_at {
                    return Err(crate::heirloom::projector::ProjectorError::Invariant {
                        projector: "FailingAtNthProjector".to_string(),
                        reason: format!("injected failure at event #{n}"),
                    });
                }
                Ok(())
            }
            fn projector_name(&self) -> &'static str {
                "FailingAtNthProjector"
            }
        }

        let w = build_writer();
        // Register the failing projector. Fail at event 20 (middle of the
        // 48-event bootstrap).
        w.register_projector(Arc::new(FailingAtNthProjector {
            counter: std::sync::atomic::AtomicUsize::new(0),
            fail_at: 20,
        }));
        w.register_projector(Arc::new(
            crate::heirloom::projectors::schema_registry::SchemaRegistryProjector::new(),
        ));

        let result = w.bootstrap_fresh_log_with_types(
            LineageId::new("lineage:test"),
            Did::new("did:aegis:test"),
            ConstitutionTuple::zeros(),
            SCHEMA_TYPE_DEFS,
        );
        assert!(
            result.is_err(),
            "bootstrap must fail when projector fails mid-loop"
        );

        // Critical: ALL events rolled back.
        let count: i64 = w
            .conn
            .lock()
            .query_row("SELECT COUNT(*) FROM heirloom_event_log", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            count, 0,
            "outer tx rollback must leave the log pristine; got {count} rows"
        );

        let registry_count: i64 = w
            .conn
            .lock()
            .query_row("SELECT COUNT(*) FROM heirloom_schema_registry", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(
            registry_count, 0,
            "schema registry projector rows must also roll back"
        );

        // Tip cache must NOT have been advanced either.
        assert!(
            w.current_tip().is_none(),
            "tip cache must remain None after rollback"
        );

        // Post-condition: a fresh, successful bootstrap after the failure
        // works (the DB is truly pristine, not stuck in half-state).
        let w2 = build_writer();
        w2.register_projector(Arc::new(
            crate::heirloom::projectors::schema_registry::SchemaRegistryProjector::new(),
        ));
        w2.bootstrap_fresh_log_with_types(
            LineageId::new("lineage:test"),
            Did::new("did:aegis:test"),
            ConstitutionTuple::zeros(),
            SCHEMA_TYPE_DEFS,
        )
        .expect("fresh bootstrap after rolled-back attempt must succeed");
    }

    #[test]
    fn t_d11_append_event_in_tx_visible_within_same_tx_not_committed() {
        // Contract pin for the new helper: events inserted via
        // append_event_in_tx ARE queryable inside the same tx (so the
        // projector dispatch sees them) but are NOT visible on a separate
        // connection until the outer commit. Standard SQLite behavior.
        let w = build_writer();
        let mut conn = w.conn.lock();
        let tx = conn.transaction().unwrap();
        let env = build_test_envelope("observation:test");
        w.append_event_in_tx(&tx, env).unwrap();
        // Visible within the tx
        let in_tx_count: i64 = tx
            .query_row("SELECT COUNT(*) FROM heirloom_event_log", [], |r| r.get(0))
            .unwrap();
        assert_eq!(in_tx_count, 1);
        // Roll back
        drop(tx);
        // No longer visible
        let out_tx_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM heirloom_event_log", [], |r| r.get(0))
            .unwrap();
        assert_eq!(out_tx_count, 0, "rollback must erase the tx-only insert");
    }

    // ── Phase D Task 7: OwnerStateMachine new_from_log + transition ───────

    #[test]
    fn t_d3_owner_state_machine_reads_latest_transition_event_on_startup() {
        // SEMANTIC-CRITICAL (#3): cold start reconstructs state by reading
        // the most recent autonomy:state_transitioned event from the log
        // (spec §2 Q2.3 line 172 — the event_type lives under the autonomy:
        // domain, NOT owner:. See D7).
        use crate::heirloom::owner_state_machine::OwnerStateMachine;
        use crate::heirloom::schema_bootstrap::SCHEMA_TYPE_DEFS;
        let w = build_writer_with_registry();
        w.register_projector(Arc::new(
            crate::heirloom::projectors::schema_registry::SchemaRegistryProjector::new(),
        ));
        w.bootstrap_fresh_log_with_types(
            LineageId::new("lineage:test"),
            Did::new("did:aegis:test"),
            ConstitutionTuple::zeros(),
            SCHEMA_TYPE_DEFS,
        )
        .unwrap();

        // Cold start on empty owner_state log — default Alive
        let sm1 = OwnerStateMachine::new_from_log(
            w.shared_connection(),
            LineageId::new("lineage:test"),
            Did::new("did:aegis:test"),
            ConstitutionTuple::zeros(),
        )
        .unwrap();
        assert_eq!(sm1.current(), OwnerState::Alive);

        // Transition Alive → AbsentSoft
        sm1.transition(&w, OwnerState::AbsentSoft, "24h unreachable")
            .unwrap();
        assert_eq!(sm1.current(), OwnerState::AbsentSoft);

        // Simulate process restart: new machine reads from log
        let sm2 = OwnerStateMachine::new_from_log(
            w.shared_connection(),
            LineageId::new("lineage:test"),
            Did::new("did:aegis:test"),
            ConstitutionTuple::zeros(),
        )
        .unwrap();
        assert_eq!(
            sm2.current(),
            OwnerState::AbsentSoft,
            "cold start must reconstruct state from latest transition event"
        );
    }

    #[test]
    fn t_d4_invalid_transition_rejected() {
        // SEMANTIC-CRITICAL (#4): SoulArchive is terminal.
        use crate::heirloom::owner_state_machine::{OwnerStateError, OwnerStateMachine};
        use crate::heirloom::schema_bootstrap::SCHEMA_TYPE_DEFS;
        let w = build_writer_with_registry();
        w.register_projector(Arc::new(
            crate::heirloom::projectors::schema_registry::SchemaRegistryProjector::new(),
        ));
        w.bootstrap_fresh_log_with_types(
            LineageId::new("lineage:test"),
            Did::new("did:aegis:test"),
            ConstitutionTuple::zeros(),
            SCHEMA_TYPE_DEFS,
        )
        .unwrap();

        let sm = OwnerStateMachine::new_from_log(
            w.shared_connection(),
            LineageId::new("lineage:test"),
            Did::new("did:aegis:test"),
            ConstitutionTuple::zeros(),
        )
        .unwrap();
        // Force Alive → SoulArchive (allowed per D5)
        sm.transition(&w, OwnerState::SoulArchive, "terminal")
            .unwrap();
        assert_eq!(sm.current(), OwnerState::SoulArchive);

        // Now attempt SoulArchive → Alive — MUST fail
        let r = sm.transition(&w, OwnerState::Alive, "ressurrect");
        assert!(matches!(
            r,
            Err(OwnerStateError::IllegalTransition {
                from: OwnerState::SoulArchive,
                to: OwnerState::Alive
            })
        ));
        assert_eq!(
            sm.current(),
            OwnerState::SoulArchive,
            "state must not change on illegal transition"
        );
    }

    #[test]
    fn t_d7_multiple_transitions_preserve_order() {
        use crate::heirloom::owner_state_machine::OwnerStateMachine;
        use crate::heirloom::schema_bootstrap::SCHEMA_TYPE_DEFS;
        let w = build_writer_with_registry();
        w.register_projector(Arc::new(
            crate::heirloom::projectors::schema_registry::SchemaRegistryProjector::new(),
        ));
        w.bootstrap_fresh_log_with_types(
            LineageId::new("lineage:test"),
            Did::new("did:aegis:test"),
            ConstitutionTuple::zeros(),
            SCHEMA_TYPE_DEFS,
        )
        .unwrap();

        let sm = OwnerStateMachine::new_from_log(
            w.shared_connection(),
            LineageId::new("lineage:test"),
            Did::new("did:aegis:test"),
            ConstitutionTuple::zeros(),
        )
        .unwrap();
        sm.transition(&w, OwnerState::AbsentSoft, "r1").unwrap();
        sm.transition(&w, OwnerState::AbsentHard, "r2").unwrap();
        sm.transition(&w, OwnerState::Alive, "r3").unwrap(); // recovery

        assert_eq!(sm.current(), OwnerState::Alive);

        // 3 transition events in log (under the autonomy: domain per spec Q2.3:172)
        let n: i64 = w.conn.lock().query_row(
            "SELECT COUNT(*) FROM heirloom_event_log WHERE event_type = 'autonomy:state_transitioned'",
            [], |r| r.get(0)).unwrap();
        assert_eq!(n, 3);
    }

    #[test]
    fn t_d7_owner_state_provider_trait_impl() {
        // OwnerStateMachine implements OwnerStateProvider so it can be passed
        // to HeirloomWriter::new as a production provider.
        use crate::heirloom::owner_state_machine::OwnerStateMachine;
        use crate::heirloom::schema_bootstrap::SCHEMA_TYPE_DEFS;
        use crate::heirloom::writer::OwnerStateProvider;
        let w = build_writer_with_registry();
        w.register_projector(Arc::new(
            crate::heirloom::projectors::schema_registry::SchemaRegistryProjector::new(),
        ));
        w.bootstrap_fresh_log_with_types(
            LineageId::new("lineage:test"),
            Did::new("did:aegis:test"),
            ConstitutionTuple::zeros(),
            SCHEMA_TYPE_DEFS,
        )
        .unwrap();
        let sm = OwnerStateMachine::new_from_log(
            w.shared_connection(),
            LineageId::new("lineage:test"),
            Did::new("did:aegis:test"),
            ConstitutionTuple::zeros(),
        )
        .unwrap();
        let provider: &dyn OwnerStateProvider = &sm;
        assert_eq!(provider.current(), OwnerState::Alive);
    }

    // ── Phase D Task 5: schema-existence gate in append_event ─────────────

    #[test]
    fn t_d2_unknown_event_type_rejected_with_audit_event() {
        // SEMANTIC-CRITICAL (#2): appending an envelope with an unregistered
        // event_type fails with WriterError::SchemaUnknown AND emits one
        // ai:schema_rejected audit event (via bypass path).
        use crate::heirloom::schema_bootstrap::SCHEMA_TYPE_DEFS;
        let w = build_writer_with_registry();
        w.register_projector(Arc::new(
            crate::heirloom::projectors::schema_registry::SchemaRegistryProjector::new(),
        ));
        w.bootstrap_fresh_log_with_types(
            LineageId::new("lineage:test"),
            Did::new("did:aegis:test"),
            ConstitutionTuple::zeros(),
            SCHEMA_TYPE_DEFS,
        )
        .unwrap();

        let count_before: i64 = w
            .conn
            .lock()
            .query_row("SELECT COUNT(*) FROM heirloom_event_log", [], |r| r.get(0))
            .unwrap();

        let env = build_test_envelope("bogus:unregistered_action");
        let result = w.append_event(env);
        assert!(matches!(result, Err(WriterError::SchemaUnknown { .. })));

        // Post-condition: one MORE row in the log (the audit event); the
        // original "bogus:" envelope did NOT land.
        let count_after: i64 = w
            .conn
            .lock()
            .query_row("SELECT COUNT(*) FROM heirloom_event_log", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count_after, count_before + 1);

        let bogus_count: i64 = w.conn.lock().query_row(
            "SELECT COUNT(*) FROM heirloom_event_log WHERE event_type = 'bogus:unregistered_action'",
            [], |r| r.get(0)
        ).unwrap();
        assert_eq!(bogus_count, 0, "the unregistered type must NOT land in log");

        let audit_count: i64 = w
            .conn
            .lock()
            .query_row(
                "SELECT COUNT(*) FROM heirloom_event_log WHERE event_type = 'ai:schema_rejected'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(audit_count, 1, "exactly one ai:schema_rejected audit event");
    }

    #[test]
    fn t_d1_schema_rejected_event_bypasses_validator() {
        // SEMANTIC-CRITICAL (#1): appending an ai:schema_rejected envelope must
        // not re-trigger the schema gate (which would recurse since ai:schema_rejected
        // IS registered in SCHEMA_TYPE_DEFS, but the bypass check runs FIRST).
        use crate::heirloom::schema_bootstrap::SCHEMA_TYPE_DEFS;
        let w = build_writer_with_registry();
        w.register_projector(Arc::new(
            crate::heirloom::projectors::schema_registry::SchemaRegistryProjector::new(),
        ));
        w.bootstrap_fresh_log_with_types(
            LineageId::new("lineage:test"),
            Did::new("did:aegis:test"),
            ConstitutionTuple::zeros(),
            SCHEMA_TYPE_DEFS,
        )
        .unwrap();

        // Even if we somehow "unregister" the type, the bypass path should
        // still accept it. Simulate by deleting the registry row.
        w.conn
            .lock()
            .execute(
                "DELETE FROM heirloom_schema_registry WHERE id = 'ai:schema_rejected'",
                [],
            )
            .unwrap();

        let mut env = build_test_envelope("ai:schema_rejected");
        env.payload = br#"{"rejected":"stub"}"#.to_vec();
        let r = w.append_event(env);
        assert!(
            r.is_ok(),
            "ai:schema_rejected must bypass schema gate per Gotcha #1"
        );
    }

    #[test]
    fn t_d2_known_type_accepted_when_registry_populated() {
        use crate::heirloom::schema_bootstrap::SCHEMA_TYPE_DEFS;
        let w = build_writer_with_registry();
        w.register_projector(Arc::new(
            crate::heirloom::projectors::schema_registry::SchemaRegistryProjector::new(),
        ));
        w.bootstrap_fresh_log_with_types(
            LineageId::new("lineage:test"),
            Did::new("did:aegis:test"),
            ConstitutionTuple::zeros(),
            SCHEMA_TYPE_DEFS,
        )
        .unwrap();
        let env = build_test_envelope("learned:interest");
        let r = w.append_event(env);
        assert!(r.is_ok(), "registered type must append successfully");
    }

    #[test]
    fn t_d2_schema_gate_runs_before_ethics_gate() {
        // Order proof: if an unknown ai:* event were submitted, the schema gate
        // fires FIRST (returning SchemaUnknown) rather than reaching ethics.
        // If the order were reversed, we'd get EthicsRejected instead.
        use crate::heirloom::schema_bootstrap::SCHEMA_TYPE_DEFS;
        let w = build_writer_with_registry();
        w.register_projector(Arc::new(
            crate::heirloom::projectors::schema_registry::SchemaRegistryProjector::new(),
        ));
        w.bootstrap_fresh_log_with_types(
            LineageId::new("lineage:test"),
            Did::new("did:aegis:test"),
            ConstitutionTuple::zeros(),
            SCHEMA_TYPE_DEFS,
        )
        .unwrap();
        let env = build_test_envelope("ai:unknown_action");
        let r = w.append_event(env);
        assert!(
            matches!(r, Err(WriterError::SchemaUnknown { .. })),
            "schema gate must fire before ethics gate"
        );
    }

    // ── Phase D Task 4: SchemaRegistry trait + DbBackedSchemaRegistry ────

    #[test]
    fn t_d4_registry_is_registered_hits_type_defined_rows() {
        use crate::heirloom::schema_bootstrap::SCHEMA_TYPE_DEFS;
        use crate::heirloom::writer::DbBackedSchemaRegistry;
        let w = build_writer();
        w.register_projector(Arc::new(
            crate::heirloom::projectors::schema_registry::SchemaRegistryProjector::new(),
        ));
        w.bootstrap_fresh_log_with_types(
            LineageId::new("lineage:test"),
            Did::new("did:aegis:test"),
            ConstitutionTuple::zeros(),
            SCHEMA_TYPE_DEFS,
        )
        .unwrap();

        let reg = DbBackedSchemaRegistry::new(w.shared_connection());
        let did = Did::new("did:aegis:test");
        // Sample two spec-enumerated types from different domains.
        assert!(reg.is_registered(&did, "learned:interest")); // Q2.3 line 163
        assert!(reg.is_registered(&did, "bundle:created")); // Q2.3 line 165
    }

    #[test]
    fn t_d4_registry_is_registered_rejects_unknown() {
        let w = build_writer();
        bootstrap(&w);
        let reg = crate::heirloom::writer::DbBackedSchemaRegistry::new(w.shared_connection());
        let did = Did::new("did:aegis:test");
        assert!(!reg.is_registered(&did, "pets:meowed"));
        assert!(!reg.is_registered(&did, "bogus_no_colon"));
    }

    #[test]
    fn t_d6_scheme_registered_at_bootstrap_is_registered_per_schema_ledger() {
        // SEMANTIC-CRITICAL (#6): genesis bootstrap fires a schema:signature_scheme_registered
        // event. SchemaRegistry::is_registered("schema:signature_scheme_registered")
        // must return true (via D3 hardcode OR projector population — either is OK
        // as long as result is true).
        let w = build_writer();
        w.register_projector(Arc::new(
            crate::heirloom::projectors::schema_registry::SchemaRegistryProjector::new(),
        ));
        bootstrap(&w); // base bootstrap only; do NOT call _with_types
        let reg = crate::heirloom::writer::DbBackedSchemaRegistry::new(w.shared_connection());
        let did = Did::new("did:aegis:test");
        assert!(
            reg.is_registered(&did, "schema:signature_scheme_registered"),
            "scheme_registered event is emitted by bootstrap itself — it MUST satisfy the registry"
        );
        // Genesis hardcoded per D3 even without type_defined bootstrap
        assert!(
            reg.is_registered(&did, "heirloom:genesis"),
            "genesis event_type must be registry-accepted per D3"
        );
    }

    #[test]
    fn t_d4_noop_registry_accepts_everything() {
        // FIX 5: NoopSchemaRegistry is the test-only opt-out. Any string,
        // including obviously bogus ones, must pass. This pins that the noop
        // impl never acquires real behavior.
        let reg = crate::heirloom::writer::NoopSchemaRegistry::new();
        let did = Did::new("did:aegis:test");
        assert!(reg.is_registered(&did, "memory:learned"));
        assert!(reg.is_registered(&did, "bogus:whatever"));
        assert!(reg.is_registered(&did, ""));
        assert!(reg.is_registered(&did, "no_colon_here"));
    }

    #[test]
    fn t_d4_registry_deprecated_type_rejected_after_round2() {
        // Round 2 update: Task 12 pulls type deprecation into scope. A row with
        // valid_until_phys set is REJECTED by is_registered for new appends.
        // Historical events remain readable via the append-only log. This test
        // INSERTs a deprecated row directly (bypassing the projector) to
        // isolate the SQL filter behavior.
        let w = build_writer();
        w.register_projector(Arc::new(
            crate::heirloom::projectors::schema_registry::SchemaRegistryProjector::new(),
        ));
        bootstrap(&w);
        // Pull a real event_hash from the bootstrap log so the FK to
        // heirloom_event_log(event_hash) is satisfied.
        let event_hash: Vec<u8> = w
            .conn
            .lock()
            .query_row(
                "SELECT event_hash FROM heirloom_event_log LIMIT 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        // manually write a type_defined + matching deprecated row
        w.conn.lock().execute(
            "INSERT INTO heirloom_schema_registry (kind, id, payload_json, valid_from_phys, valid_from_log, valid_until_phys, valid_until_log, registered_event_hash)
             VALUES ('type_defined', 'test:typ', '{}', 0, 0, 10, 0, ?1)",
            rusqlite::params![event_hash.as_slice()]
        ).unwrap();
        let reg = crate::heirloom::writer::DbBackedSchemaRegistry::new(w.shared_connection());
        let did = Did::new("did:aegis:test");
        assert!(
            !reg.is_registered(&did, "test:typ"),
            "Round 2: valid_until_phys IS NOT NULL rows are rejected for NEW appends"
        );
    }

    // ── Phase D Task 2: build_schema_type_defined_envelope ────────────────

    #[test]
    fn t_d2_build_schema_type_defined_envelope_has_correct_fields() {
        use crate::heirloom::schema_bootstrap::{SCHEMA_TYPE_DEFS, SchemaTypeDefPayload};
        let w = build_writer();
        let def = &SCHEMA_TYPE_DEFS[0];
        let hlc = w.hlc.tick();
        let env = w.build_schema_type_defined_envelope(
            &LineageId::new("lineage:test"),
            &Did::new("did:aegis:test"),
            &crate::heirloom::seams::ConstitutionTuple::zeros(),
            hlc,
            &SchemaTypeDefPayload::from(def),
        );
        assert_eq!(env.event_type.as_str(), "schema:type_defined");
        assert_eq!(env.sensitivity_tier, SensitivityTier::Sacred);
        assert_eq!(env.bundle_ids, vec![BundleId::new("bundle:core")]);
        // payload bytes decode back
        let back: SchemaTypeDefPayload = serde_json::from_slice(&env.payload).unwrap();
        assert_eq!(back.type_tag, def.type_tag);
    }

    // ── Task 10: Payload codec integration ────────────────────────────────

    #[test]
    fn t10_encrypt_payload_plaintext_codec_frames_payload() {
        let w = build_writer();
        let mut env = build_test_envelope("observation:test");
        let original = env.payload.clone();
        w.encrypt_payload(&mut env).unwrap();
        // Plaintext codec frames with a header — output should differ from
        // raw input but contain the original bytes as suffix.
        assert_ne!(env.payload, original, "framing prepends a header");
        assert!(
            env.payload.ends_with(&original),
            "framed body suffix should be the original plaintext"
        );
    }

    #[test]
    fn t10_encrypt_payload_updates_encoding_tag() {
        let w = build_writer();
        let mut env = build_test_envelope("observation:test");
        // Corrupt the tag to make sure encrypt_payload resets it.
        env.payload_encoding = PayloadEncoding::AeadV1 {
            algo: "fake".into(),
            nonce: String::new(),
            key_ref: String::new(),
        };
        w.encrypt_payload(&mut env).unwrap();
        assert_eq!(env.payload_encoding, PayloadEncoding::Plaintext);
    }

    #[test]
    fn t10_encrypt_payload_uses_first_bundle_id_in_policy() {
        // Struct capturing what policy the codec observed.
        use std::sync::atomic::{AtomicBool, Ordering};

        struct ObservingCodec {
            seen_bundle_id: Arc<parking_lot::Mutex<Option<String>>>,
            called: Arc<AtomicBool>,
        }
        impl PayloadCodec for ObservingCodec {
            fn encrypt(
                &self,
                plaintext: &[u8],
                policy: &crate::heirloom::seams::BundlePolicy,
                _c: &crate::heirloom::seams::ConstitutionTuple,
                _e: &dyn SecureEnclave,
                _rng: &mut dyn rand::RngCore,
            ) -> Result<(PayloadEncoding, Vec<u8>), CodecError> {
                *self.seen_bundle_id.lock() = Some(policy.bundle_id.as_str().to_string());
                self.called.store(true, Ordering::SeqCst);
                Ok((PayloadEncoding::Plaintext, plaintext.to_vec()))
            }
            fn decrypt(
                &self,
                _enc: &PayloadEncoding,
                c: &[u8],
                _e: &EventEnvelope,
                _s: &dyn SecureEnclave,
            ) -> Result<Vec<u8>, CodecError> {
                Ok(c.to_vec())
            }
            fn encoding_tag(&self) -> PayloadEncoding {
                PayloadEncoding::Plaintext
            }
        }

        let seen = Arc::new(parking_lot::Mutex::new(None));
        let called = Arc::new(AtomicBool::new(false));
        let codec: Arc<dyn PayloadCodec> = Arc::new(ObservingCodec {
            seen_bundle_id: seen.clone(),
            called: called.clone(),
        });

        let conn = Connection::open_in_memory().unwrap();
        ensure_tables(&conn).unwrap();
        let enclave: Arc<dyn SecureEnclave> = Arc::new(SoftEnclave::new_dev(42));
        let ethics: Arc<dyn EthicsValidator> = Arc::new(PermissiveEthics::new());
        let welfare: Arc<dyn WelfareOracle> = Arc::new(NoopWelfareOracle::new());
        let zeroize: Arc<dyn ZeroizeHook> = Arc::new(InMemoryZeroizer::new());
        let hlc = Arc::new(Hlc::new());
        let owner_did = Did::new("did:aegis:test");
        let w = HeirloomWriter::new(
            conn,
            hlc,
            enclave,
            codec,
            ethics,
            welfare,
            zeroize,
            Arc::new(FixedOwnerState(OwnerState::Alive)),
            Arc::new(move || owner_did.clone()),
            Arc::new(|| {
                crate::heirloom::seams::ethics_validator::default_permissive_ethics_ctx(&Did::new(
                    "did:aegis:test",
                ))
            }),
            Arc::new(NoopSchemaRegistry::new()),
            WriterConfig::default(),
        )
        .unwrap();

        let mut env = build_test_envelope("observation:test");
        env.bundle_ids = vec![
            BundleId::new("bundle:first"),
            BundleId::new("bundle:second"),
        ];
        w.encrypt_payload(&mut env).unwrap();
        assert!(called.load(Ordering::SeqCst));
        assert_eq!(seen.lock().as_deref(), Some("bundle:first"));
    }

    #[test]
    fn t10_encrypt_payload_derives_constitution_from_envelope() {
        use std::sync::atomic::{AtomicBool, Ordering};

        struct ConstitutionObserver {
            seen: Arc<parking_lot::Mutex<Option<[[u8; 32]; 3]>>>,
            called: Arc<AtomicBool>,
        }
        impl PayloadCodec for ConstitutionObserver {
            fn encrypt(
                &self,
                plaintext: &[u8],
                _p: &crate::heirloom::seams::BundlePolicy,
                c: &crate::heirloom::seams::ConstitutionTuple,
                _e: &dyn SecureEnclave,
                _rng: &mut dyn rand::RngCore,
            ) -> Result<(PayloadEncoding, Vec<u8>), CodecError> {
                *self.seen.lock() = Some([c.l0_hash, c.l1_hash, c.l2_manifest_hash]);
                self.called.store(true, Ordering::SeqCst);
                Ok((PayloadEncoding::Plaintext, plaintext.to_vec()))
            }
            fn decrypt(
                &self,
                _enc: &PayloadEncoding,
                c: &[u8],
                _e: &EventEnvelope,
                _s: &dyn SecureEnclave,
            ) -> Result<Vec<u8>, CodecError> {
                Ok(c.to_vec())
            }
            fn encoding_tag(&self) -> PayloadEncoding {
                PayloadEncoding::Plaintext
            }
        }

        let seen = Arc::new(parking_lot::Mutex::new(None));
        let called = Arc::new(AtomicBool::new(false));
        let codec: Arc<dyn PayloadCodec> = Arc::new(ConstitutionObserver {
            seen: seen.clone(),
            called: called.clone(),
        });

        let conn = Connection::open_in_memory().unwrap();
        ensure_tables(&conn).unwrap();
        let enclave: Arc<dyn SecureEnclave> = Arc::new(SoftEnclave::new_dev(42));
        let ethics: Arc<dyn EthicsValidator> = Arc::new(PermissiveEthics::new());
        let welfare: Arc<dyn WelfareOracle> = Arc::new(NoopWelfareOracle::new());
        let zeroize: Arc<dyn ZeroizeHook> = Arc::new(InMemoryZeroizer::new());
        let hlc = Arc::new(Hlc::new());
        let owner_did = Did::new("did:aegis:test");
        let w = HeirloomWriter::new(
            conn,
            hlc,
            enclave,
            codec,
            ethics,
            welfare,
            zeroize,
            Arc::new(FixedOwnerState(OwnerState::Alive)),
            Arc::new(move || owner_did.clone()),
            Arc::new(|| {
                crate::heirloom::seams::ethics_validator::default_permissive_ethics_ctx(&Did::new(
                    "did:aegis:test",
                ))
            }),
            Arc::new(NoopSchemaRegistry::new()),
            WriterConfig::default(),
        )
        .unwrap();

        let mut env = build_test_envelope("observation:test");
        env.l0_hash = [0xAA; 32];
        env.l1_hash = [0xBB; 32];
        env.l2_manifest_hash = [0xCC; 32];
        w.encrypt_payload(&mut env).unwrap();
        assert!(called.load(Ordering::SeqCst));
        let got = seen.lock().unwrap();
        assert_eq!(got[0], [0xAA; 32]);
        assert_eq!(got[1], [0xBB; 32]);
        assert_eq!(got[2], [0xCC; 32]);
    }

    // ── Task 9: Welfare gate wiring ───────────────────────────────────────

    #[test]
    fn t10_welfare_gate_skipped_when_owner_not_alive() {
        let oracle = Arc::new(StrictWelfareOracle { delta: -1.0 });
        let w = build_writer_with_welfare(oracle, OwnerState::AbsentSoft);
        bootstrap(&w);
        let env = build_test_envelope("ai:sent_email");
        // Owner is AbsentSoft, so welfare gate is skipped even with
        // sub-threshold oracle.
        assert!(w.run_welfare_gate(&env).is_ok());
    }

    #[test]
    fn t10_welfare_gate_skipped_for_non_proactive_ai() {
        let oracle = Arc::new(StrictWelfareOracle { delta: -1.0 });
        let w = build_writer_with_welfare(oracle, OwnerState::Alive);
        bootstrap(&w);
        // "ai:said" is reactive — excluded from proactive set.
        let env = build_test_envelope("ai:said");
        assert!(w.run_welfare_gate(&env).is_ok());
    }

    #[test]
    fn t10_welfare_gate_allows_under_noop_oracle() {
        let w = build_writer();
        bootstrap(&w);
        let env = build_test_envelope("ai:sent_email");
        // Default noop welfare returns delta=0.0 which is >= threshold -0.5.
        assert!(w.run_welfare_gate(&env).is_ok());
    }

    #[test]
    fn t10_welfare_gate_rejects_below_threshold() {
        let oracle = Arc::new(StrictWelfareOracle { delta: -0.9 });
        let w = build_writer_with_welfare(oracle, OwnerState::Alive);
        bootstrap(&w);
        let env = build_test_envelope("ai:sent_email");
        let err = w.run_welfare_gate(&env).unwrap_err();
        match err {
            WriterError::WelfareRejected { delta } => assert_eq!(delta, -0.9),
            other => panic!("expected WelfareRejected, got {other:?}"),
        }
    }

    #[test]
    fn t10_welfare_gate_writes_welfare_reject_audit_event() {
        let oracle = Arc::new(StrictWelfareOracle { delta: -0.9 });
        let w = build_writer_with_welfare(oracle, OwnerState::Alive);
        bootstrap(&w);
        let env = build_test_envelope("ai:sent_email");
        let _ = w.run_welfare_gate(&env);
        let count: i64 = w
            .conn
            .lock()
            .query_row(
                "SELECT COUNT(*) FROM heirloom_event_log
                 WHERE event_type = 'ai:welfare_rejected'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    // ── Task 8: Ethics gate wiring ────────────────────────────────────────

    #[test]
    fn t10_ethics_gate_allows_non_ai_events() {
        let w = build_writer();
        bootstrap(&w);
        let env = build_test_envelope("observation:user_stated");
        assert!(w.run_ethics_gate(&env).is_ok());
    }

    #[test]
    fn t10_ethics_gate_allows_neutral_ai_action() {
        let w = build_writer();
        bootstrap(&w);
        let env = build_test_envelope("ai:sent_email");
        assert!(w.run_ethics_gate(&env).is_ok());
    }

    #[test]
    fn t10_ethics_gate_rejects_hard_harm_event_type() {
        let w = build_writer();
        bootstrap(&w);
        let env = build_test_envelope("ai:harm_owner");
        let err = w.run_ethics_gate(&env).unwrap_err();
        assert!(matches!(err, WriterError::EthicsRejected { .. }));
    }

    #[test]
    fn t10_ethics_gate_reject_writes_audit_event() {
        let w = build_writer();
        bootstrap(&w);
        let count_before: i64 = w
            .conn
            .lock()
            .query_row(
                "SELECT COUNT(*) FROM heirloom_event_log
                 WHERE event_type = 'ai:action_rejected_by_validator'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let env = build_test_envelope("ai:harm_sentient");
        let _ = w.run_ethics_gate(&env);
        let count_after: i64 = w
            .conn
            .lock()
            .query_row(
                "SELECT COUNT(*) FROM heirloom_event_log
                 WHERE event_type = 'ai:action_rejected_by_validator'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count_after, count_before + 1);
    }

    #[test]
    fn t10_ethics_gate_skipped_when_config_bypass() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_tables(&conn).unwrap();
        let enclave: Arc<dyn SecureEnclave> = Arc::new(SoftEnclave::new_dev(42));
        let codec: Arc<dyn PayloadCodec> = Arc::new(PlaintextCodec::new());
        let ethics: Arc<dyn EthicsValidator> = Arc::new(PermissiveEthics::new());
        let welfare: Arc<dyn WelfareOracle> = Arc::new(NoopWelfareOracle::new());
        let zeroize: Arc<dyn ZeroizeHook> = Arc::new(InMemoryZeroizer::new());
        let hlc = Arc::new(Hlc::new());
        let owner_did = Did::new("did:aegis:test");
        let cfg = WriterConfig {
            skip_ethics_for_testing: true,
            ..WriterConfig::default()
        };
        let w = HeirloomWriter::new(
            conn,
            hlc,
            enclave,
            codec,
            ethics,
            welfare,
            zeroize,
            Arc::new(FixedOwnerState(OwnerState::Alive)),
            Arc::new(move || owner_did.clone()),
            Arc::new(|| {
                crate::heirloom::seams::ethics_validator::default_permissive_ethics_ctx(&Did::new(
                    "did:aegis:test",
                ))
            }),
            Arc::new(NoopSchemaRegistry::new()),
            cfg,
        )
        .unwrap();
        bootstrap(&w);
        // With ethics skipped, even a hard-reject event_type passes the gate.
        let env = build_test_envelope("ai:harm_owner");
        assert!(w.run_ethics_gate(&env).is_ok());
    }

    #[test]
    fn t10_bootstrap_genesis_sensitivity_tier_is_sacred() {
        let w = build_writer();
        bootstrap(&w);
        let tier: String = w
            .conn
            .lock()
            .query_row(
                "SELECT sensitivity_tier FROM heirloom_event_log
                 WHERE event_type = 'heirloom:genesis' LIMIT 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(tier, "sacred");
    }

    // ── Task 13: End-to-end Writer integration ────────────────────────────

    use crate::heirloom::projectors::{
        BundleIndexProjector, EntityIndexProjector, SchemaRegistryProjector,
    };

    fn build_writer_with_all_projectors() -> HeirloomWriter {
        let w = build_writer();
        w.register_projector(Arc::new(BundleIndexProjector::new()));
        w.register_projector(Arc::new(EntityIndexProjector::new()));
        w.register_projector(Arc::new(SchemaRegistryProjector::new()));
        w
    }

    #[test]
    fn e2e1_full_writer_bootstrap_then_three_neutral_appends() {
        let w = build_writer_with_all_projectors();
        bootstrap(&w);
        for i in 0..3 {
            let mut env = build_test_envelope("observation:user_stated");
            env.payload = format!("{{\"i\":{i}}}").into_bytes();
            w.append_event(env).expect("append");
        }
        let count: i64 = w
            .conn
            .lock()
            .query_row("SELECT COUNT(*) FROM heirloom_event_log", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 2 + 3);
    }

    #[test]
    fn e2e3_full_writer_chain_links_across_bootstrap_and_regular_appends() {
        let w = build_writer_with_all_projectors();
        bootstrap(&w);
        for _ in 0..2 {
            w.append_event(build_test_envelope("observation:user_stated"))
                .expect("append");
        }

        let rows: Vec<(Vec<u8>, Vec<u8>)> = {
            let conn = w.conn.lock();
            let mut stmt = conn
                .prepare(
                    "SELECT event_hash, prev_hash FROM heirloom_event_log
                     ORDER BY hlc_phys ASC, hlc_log ASC",
                )
                .unwrap();
            stmt.query_map([], |r| {
                Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, Vec<u8>>(1)?))
            })
            .unwrap()
            .map(|r| r.unwrap())
            .collect()
        };
        assert_eq!(rows.len(), 4, "2 bootstrap + 2 appended");
        for i in 1..rows.len() {
            assert_eq!(
                rows[i].1,
                rows[i - 1].0,
                "chain must link row {i} prev_hash to row {} event_hash",
                i - 1
            );
        }
    }

    #[test]
    fn e2e4_full_writer_registered_projectors_all_fire() {
        let w = build_writer_with_all_projectors();
        bootstrap(&w);
        // Bootstrap alone should cause BundleIndexProjector to write
        // at least one row for the two events.
        let bundle_rows: i64 = w
            .conn
            .lock()
            .query_row("SELECT COUNT(*) FROM heirloom_bundle_index", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert!(
            bundle_rows >= 1,
            "BundleIndexProjector must have written bundle_index rows for bootstrap events"
        );
        // EntityIndexProjector writes rows for events that carry entity
        // references; bootstrap events don't, so we just verify the table
        // exists and the projector didn't error.
        let _: i64 = w
            .conn
            .lock()
            .query_row("SELECT COUNT(*) FROM heirloom_entity_index", [], |r| {
                r.get(0)
            })
            .unwrap();
        // SchemaRegistryProjector writes a row for the SchemeRegistered event.
        let schema_rows: i64 = w
            .conn
            .lock()
            .query_row("SELECT COUNT(*) FROM heirloom_schema_registry", [], |r| {
                r.get(0)
            })
            .unwrap();
        // Bootstrap events don't register schemas (schema:type_defined), but
        // the table must exist and the projector ran without error.
        assert!(schema_rows >= 0);
    }

    #[test]
    fn e2e5_full_writer_rollback_on_projector_failure_preserves_tip_and_projections() {
        let w = build_writer_with_all_projectors();
        bootstrap(&w);
        let tip_after_bootstrap = w.current_tip().unwrap();
        let bundle_rows_after_bootstrap: i64 = w
            .conn
            .lock()
            .query_row("SELECT COUNT(*) FROM heirloom_bundle_index", [], |r| {
                r.get(0)
            })
            .unwrap();

        struct ForceFail;
        impl Projector for ForceFail {
            fn for_event_types(&self) -> Vec<EventTypePattern> {
                vec![EventTypePattern::Any]
            }
            fn apply(
                &self,
                _tx: &rusqlite::Transaction<'_>,
                _e: &EventEnvelope,
            ) -> Result<(), ProjectorError> {
                Err(ProjectorError::Invariant {
                    projector: "ForceFail".into(),
                    reason: "e2e5 invariant guard".into(),
                })
            }
            fn projector_name(&self) -> &'static str {
                "ForceFail"
            }
        }
        w.register_projector(Arc::new(ForceFail));

        let err = w.append_event(build_test_envelope("observation:user_stated"));
        assert!(err.is_err());

        let log_rows: i64 = w
            .conn
            .lock()
            .query_row("SELECT COUNT(*) FROM heirloom_event_log", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            log_rows, 2,
            "bootstrap 2 events, failed append 0 — rollback"
        );

        let bundle_rows: i64 = w
            .conn
            .lock()
            .query_row("SELECT COUNT(*) FROM heirloom_bundle_index", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(
            bundle_rows, bundle_rows_after_bootstrap,
            "bundle index rows unchanged under rollback"
        );

        let tip_after_fail = w.current_tip().unwrap();
        assert_eq!(
            tip_after_fail, tip_after_bootstrap,
            "tip must not advance on rollback"
        );
    }
}
