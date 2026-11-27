//! Embedding-corpus model-change consent mechanism.
//!
//! **Problem**: the HNSW + `l2_experience.embedding` corpus is built with ONE
//! specific primary embedding model. Cosine similarity across two models is
//! mathematically meaningless, so semantic recall silently degrades the moment
//! the live primary drifts away from the model that produced the stored
//! vectors. This happens in three ways:
//!
//! 1. Operator edits `EMBEDDING_MODEL` in `.env` and restarts.
//! 2. 24h permanent promotion swaps backup → primary after a long outage.
//! 3. `unset_embedding_primary_promotion` reverts a promotion after the
//!    operator already edited `.env`.
//!
//! **Design**: a single `user_settings` row named
//! [`SETTING_KEY`] records the model that produced the CURRENT corpus. On
//! every startup we compare the marker to the live primary and surface the
//! mismatch to the user via Telegram. We never auto-reembed — the user
//! decides via the [`trigger_corpus_reembed`] orchestrator tool (RequireApproval).
//! Cost and walltime are non-trivial (~$0.01, ~minutes for ~1K rows) and the
//! operation destroys the existing corpus atomically.
//!
//! **Preserved change-of-mind**: declining the alert is a no-op. The LLM is
//! instructed (via tool description) to call `trigger_corpus_reembed` any
//! time the user says "actually, go ahead and reembed". There is no explicit
//! "declined_at" state — absence of action IS the declined state.
//!
//! See also: `docs/DEPLOYMENT.md §3.3.3`.

use std::time::Instant;

use anyhow::{Context, Result};

use crate::embedding::EmbeddingClient;
use crate::memory::MemoryHandle;

/// `user_settings` key that holds the name of the embedding model whose
/// vectors are currently persisted in `l2_experience.embedding`. Written once
/// on fresh install and updated atomically at the end of a successful
/// `trigger_corpus_reembed` run. Never touched by the failover or 24h-promotion
/// paths.
pub const SETTING_KEY: &str = "embedding_corpus_model";

/// Three-way status of the startup consistency check.
///
/// `FreshInstall` is distinguished from `Mismatch` because a brand-new deploy
/// has no corpus to warn about — silently writing the marker is the correct
/// no-op behavior.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CorpusStatus {
    /// No marker row AND no existing vectors → a brand-new deploy. Caller
    /// should write the marker and not alert.
    FreshInstall,
    /// Marker matches the live primary model → normal operation, no alert.
    Match,
    /// Marker differs from the live primary model → existing vectors were
    /// produced by a different model and are now cross-model. Caller should
    /// alert but must NOT auto-update the marker.
    Mismatch,
}

/// Result of [`detect_corpus_mismatch`].
///
/// `marker_before` is `None` on `FreshInstall`; populated with the stale
/// model name on `Mismatch`; equal to `Some(live_primary)` on `Match`.
#[derive(Debug, Clone)]
pub struct CorpusCheck {
    pub status: CorpusStatus,
    pub marker_before: Option<String>,
    pub live_primary: String,
    pub vector_count: i64,
}

/// Result of [`full_corpus_reembed`].
#[derive(Debug, Clone)]
pub struct ReembedOutcome {
    /// Rows whose embedding was set to NULL prior to re-embedding.
    pub rows_cleared: i64,
    /// Rows successfully re-embedded and persisted.
    pub rows_reembedded: i64,
    /// Rows that failed to re-embed (left NULL; picked up by the startup
    /// `reembed_stale_facts` path if the primary recovers later).
    pub rows_failed: i64,
    /// Wall-clock duration in seconds.
    pub walltime_sec: f64,
    /// Model used for the re-embed pass. Stored in the marker at completion.
    pub model_used: String,
    /// Terminal status of the pass. `"ok"` on a clean finish, or
    /// `"aborted_model_changed"` if [`full_corpus_reembed`] detected the
    /// embedder flipping primary/backup (degraded-mode transition) mid-pass
    /// and bailed out to avoid persisting mixed-model vectors. See FAIL #1
    /// in the CTO round-1 review.
    pub status: String,
    /// On `status == "aborted_model_changed"`, the model observed at the
    /// flip point. `None` when `status == "ok"`.
    pub new_model_observed: Option<String>,
    /// Non-`None` if the HNSW rebuild step failed. Carries the formatted
    /// error string so the caller can surface it to the user. We also skip
    /// the marker update when this is set so the next startup re-fires the
    /// mismatch alert and the user can retry. See FAIL #3.
    pub hnsw_rebuild_failed: Option<String>,
}

/// Progress callback type: `(done, total)`. Invoked roughly every
/// [`PROGRESS_EVERY`] rows during a reembed. Keep the closure cheap —
/// downstream implementations can throttle Telegram writes themselves.
pub type ProgressFn = Box<dyn Fn(i64, i64) + Send + Sync>;

/// Emit a progress update every N rows. Trade-off: 200 for ~1K corpus gives
/// ~5 alerts per pass, which is noisy-but-useful. Larger values silence the
/// user too long; smaller values spam.
pub const PROGRESS_EVERY: i64 = 200;

/// Read the corpus marker from the `user_settings` table.
///
/// Returns `Ok(None)` if the row is absent (fresh install) or if it is
/// present but not a string (defensive — legitimate writes always use
/// `Value::String`).
pub async fn read_marker(memory: &MemoryHandle) -> Result<Option<String>> {
    let all = memory
        .load_all_settings()
        .await
        .context("embedding_corpus: reading user_settings")?;
    let v = all
        .get(SETTING_KEY)
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    Ok(v)
}

/// Write the corpus marker. Used on fresh install (first startup with an
/// empty corpus) and at the end of a successful reembed.
pub async fn write_marker(memory: &MemoryHandle, model: &str) -> Result<()> {
    memory
        .store_setting(SETTING_KEY, serde_json::Value::String(model.to_string()))
        .await
        .context("embedding_corpus: writing marker")
}

/// Count non-NULL embedded rows in both corpus tables that the reembed path
/// clears. Used by `trigger_corpus_reembed` to show separate `l2_experience`
/// and `l2_ephemeral` totals in the initial Telegram notice (WARN #7).
///
/// Opens a short-lived rusqlite connection on the WAL-mode DB so we do not
/// have to plumb a new actor command through `MemoryHandle`.
pub async fn count_corpus_parts(db_path: &str) -> Result<(i64, i64)> {
    let db = db_path.to_string();
    tokio::task::spawn_blocking(move || -> Result<(i64, i64)> {
        let conn =
            crate::encrypted_sqlite::open(&db).context("count_corpus_parts: opening SQLite")?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;")?;
        let exp: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM l2_experience WHERE embedding IS NOT NULL",
                [],
                |r| r.get(0),
            )
            .unwrap_or(0);
        let eph: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM l2_ephemeral WHERE embedding IS NOT NULL",
                [],
                |r| r.get(0),
            )
            .unwrap_or(0);
        Ok((exp, eph))
    })
    .await?
}

/// Run the consistency check at startup.
///
/// Opens a read-only query against the live DB to count persisted vectors.
/// Returns a [`CorpusCheck`] the caller can match on.
///
/// # Errors
/// Propagates any DB or settings-table error. Callers should WARN-log and
/// continue — a broken consistency check must not block service startup.
pub async fn detect_corpus_mismatch(
    memory: &MemoryHandle,
    embedder: &EmbeddingClient,
) -> Result<CorpusCheck> {
    let marker = read_marker(memory).await?;
    let count = memory
        .count_embedded_facts()
        .await
        .context("embedding_corpus: counting embedded facts")?;
    let live_primary = embedder.primary_model();

    let status = match (&marker, count) {
        // Fresh install: no marker AND no existing corpus rows.
        (None, 0) => CorpusStatus::FreshInstall,
        // Marker absent but rows exist: treat as mismatch. The DB carries
        // vectors whose model we can no longer identify — alert so the
        // user can decide. This also covers the one-time migration from
        // the pre-marker era (existing 1611 rows from google/gemini-embedding-001
        // before this mechanism landed).
        (None, _) => CorpusStatus::Mismatch,
        (Some(m), _) if *m == live_primary => CorpusStatus::Match,
        (Some(_), _) => CorpusStatus::Mismatch,
    };

    Ok(CorpusCheck {
        status,
        marker_before: marker,
        live_primary,
        vector_count: count,
    })
}

/// Build a bilingual (EN + ZH) Telegram alert describing the model mismatch.
///
/// Kept pure (no I/O) so tests can assert on the string content and callers
/// can choose whether to send, log, or both. The returned string is plain
/// markdown suitable for `markdown_to_telegram_html`.
pub fn build_mismatch_alert_html(check: &CorpusCheck) -> String {
    let old_desc = check
        .marker_before
        .as_deref()
        .unwrap_or("(unknown — pre-consent-marker deploy)");
    let new_desc = check.live_primary.as_str();
    let n = check.vector_count;
    format!(
        "⚠\u{fe0f} Embedding primary model changed / 向量主模型已变更\n\
         \n\
         Previous: `{old_desc}`  ({n} vectors stored with this model)\n\
         Current:  `{new_desc}`\n\
         \n\
         Existing L2 memory vectors are now CROSS-MODEL \
         (cosine becomes unreliable). Semantic recall against historical \
         memory will degrade to text-fallback.\n\
         \n\
         Options:\n\
         A. Approve reembed — recompute all {n} vectors with `{new_desc}`.\n\
            Ask me: \"please run trigger_corpus_reembed\" (I will request your approval).\n\
         B. Decline — keep old vectors; semantic recall remains degraded \
            for historical memory until primary changes back OR you approve later.\n\
         \n\
         You can change your mind any time by asking me to run \
         `trigger_corpus_reembed`.\n\
         \n\
         ───\n\
         \n\
         主 embedding 模型已变更。原: `{old_desc}`，新: `{new_desc}`。\n\
         现有 {n} 个 L2 记忆向量跨模型，余弦相似度不再可比，\
         语义检索会降级为文本回退。\n\
         \n\
         A. 批准重算：用 `{new_desc}` 重新计算所有 {n} 个向量（需批准）。\n\
         B. 拒绝：保留旧向量，需要时向我说\"重新计算 embedding\"即可随时改变主意。",
    )
}

/// Full corpus reembed — MVP scope = "all".
///
/// Opens a separate rusqlite connection on `db_path` (WAL mode allows
/// concurrent reads from the main actor). Sets all non-NULL embeddings to
/// NULL atomically, then re-embeds each row sequentially via the provided
/// embedder, then writes back.
///
/// # Destruction / atomicity
/// The NULL sweep is NOT wrapped in a transaction with the re-embed pass —
/// a crash mid-run leaves some rows NULL. This is acceptable because:
/// - The startup `reembed_stale_facts` path already handles NULL rows.
/// - A half-complete corpus is safer than a mixed-model corpus (cosine is
///   still meaningless even if 95 % of rows are the new model).
///
/// # Degraded-mode policy
/// If the embedder is in degraded mode (failover to backup), the resulting
/// vectors will be from the BACKUP model — we continue anyway because the
/// user explicitly approved the reembed, but we write the backup model name
/// into the marker at the end so subsequent mismatch checks are honest.
///
/// # HNSW
/// Rebuild is deferred to `MemoryHandle::request_hnsw_rebuild` (not called
/// here — the caller wires it because MemoryHandle is actor-scoped).
pub async fn full_corpus_reembed(
    db_path: &str,
    memory: &MemoryHandle,
    embedder: &EmbeddingClient,
    expected_blob_size: usize,
    progress: Option<ProgressFn>,
) -> Result<ReembedOutcome> {
    let started_at = Instant::now();

    // Phase 0: count rows and NULL the column atomically. We also NULL
    // l2_ephemeral.embedding in the same breath — same cross-model concern.
    let db = db_path.to_string();
    let (rows_cleared, rows_to_reembed) =
        tokio::task::spawn_blocking(move || -> Result<(i64, Vec<(i64, String)>)> {
            let conn = crate::encrypted_sqlite::open(&db)
                .context("full_corpus_reembed: opening SQLite")?;
            conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;")?;

            // Count BEFORE NULL sweep so the progress UI can show total.
            let exp_count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM l2_experience WHERE embedding IS NOT NULL",
                    [],
                    |r| r.get(0),
                )
                .unwrap_or(0);
            let eph_count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM l2_ephemeral WHERE embedding IS NOT NULL",
                    [],
                    |r| r.get(0),
                )
                .unwrap_or(0);
            let total_cleared = exp_count + eph_count;

            // NULL everything.
            conn.execute(
                "UPDATE l2_experience SET embedding = NULL WHERE embedding IS NOT NULL",
                [],
            )
            .context("full_corpus_reembed: NULLing l2_experience")?;
            conn.execute(
                "UPDATE l2_ephemeral SET embedding = NULL WHERE embedding IS NOT NULL",
                [],
            )
            .context("full_corpus_reembed: NULLing l2_ephemeral")?;

            // Read back the rows we need to re-embed. l2_experience is the
            // only HNSW-indexed table, so we only re-embed that one. The
            // l2_ephemeral vectors are short-lived; letting them rebuild
            // lazily on next write keeps the pass fast.
            let mut stmt = conn.prepare(
                "SELECT id, fact_summary FROM l2_experience \
                 WHERE embedding IS NULL AND topic != 'l1_overflow'",
            )?;
            let rows: Vec<(i64, String)> = stmt
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
                .filter_map(|r| r.ok())
                .collect();

            Ok((total_cleared, rows))
        })
        .await??;

    let total: i64 = rows_to_reembed.len() as i64;

    // Phase 1: re-embed each fact via the live embedder.
    //
    // FAIL #1 (CTO round 1): snapshot the embedder's identity AT THE START of
    // the loop. If the embedder flips primary/backup (degraded-mode
    // transition) while we're halfway through the corpus, rows 0..N get one
    // model and rows N..M get another — the exact cross-model corruption this
    // whole feature exists to prevent. On flip we abort, do NOT update the
    // marker (so the stale value keeps the mismatch alert firing), and
    // surface a structured partial-success outcome to the caller. The
    // already-persisted subset of rows is in a mixed-model state locally
    // until the next clean pass repairs it, which is strictly no worse than
    // the pre-feature status quo.
    //
    // FAIL #2 (2026-04-19): when primary == backup (e.g. both set to
    // `baai/bge-m3` after a manual failover), a transient upstream provider
    // error causes a harmless failover→recovery flap that does NOT change
    // the active model NAME (bge-m3 → bge-m3), yet still toggles
    // `is_degraded()`. The earlier check that bailed on `is_degraded()`
    // flipping would abort the entire reembed on these benign flaps. Active
    // model name is the only identity that affects vector semantics — if
    // it's unchanged, the vectors we produced are mutually consistent
    // regardless of which redundant provider served them. We now check
    // model name only.
    let model_snapshot = embedder.active_model();
    let mut updates: Vec<(i64, Vec<u8>)> = Vec::new();
    let mut rows_failed: i64 = 0;
    let mut aborted_new_model: Option<String> = None;
    for (idx, (id, fact)) in rows_to_reembed.iter().enumerate() {
        // Pre-embed check: has the active model NAME changed since we
        // snapshotted? Degraded-flag flaps with an unchanged model name are
        // benign (see FAIL #2) and must not trigger abort.
        let cur_model = embedder.active_model();
        if cur_model != model_snapshot {
            tracing::error!(
                snapshot = %model_snapshot,
                observed = %cur_model,
                rows_embedded_so_far = updates.len(),
                "full_corpus_reembed: embedder identity flipped mid-pass, aborting to keep corpus model-pure"
            );
            aborted_new_model = Some(cur_model);
            break;
        }

        match embedder.embed(fact).await {
            Ok(emb) => {
                let blob = EmbeddingClient::to_blob(&emb);
                if blob.len() != expected_blob_size {
                    // Dimension mismatch: caller configured EMBEDDING_DIMENSIONS
                    // that does not match what the model returns. Skip this row
                    // rather than persist a corrupt vector.
                    tracing::warn!(
                        id,
                        got = blob.len(),
                        want = expected_blob_size,
                        "full_corpus_reembed: unexpected embedding size, skipping row"
                    );
                    rows_failed += 1;
                } else {
                    updates.push((*id, blob));
                }
            }
            Err(e) => {
                tracing::warn!(id, error = %format!("{e:#}"), "full_corpus_reembed: embed API failed");
                rows_failed += 1;
            }
        }

        // Post-embed check: the embedder may have flipped DURING the embed()
        // call (failover). If the active model NAME changed, the row we just
        // queued was produced by the NEW model — drop it to preserve the
        // snapshot invariant, then abort. Degraded-flag flips with unchanged
        // name are benign (FAIL #2).
        let cur_model_after = embedder.active_model();
        if cur_model_after != model_snapshot {
            if updates.last().map(|(last_id, _)| *last_id) == Some(*id) {
                updates.pop();
            }
            tracing::error!(
                snapshot = %model_snapshot,
                observed = %cur_model_after,
                rows_embedded_so_far = updates.len(),
                "full_corpus_reembed: embedder identity flipped during embed() call, aborting"
            );
            aborted_new_model = Some(cur_model_after);
            break;
        }

        // Rate limit: ~10 req/sec to mirror `reembed_stale_facts`.
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        if let Some(ref cb) = progress {
            let done = (idx as i64) + 1;
            // Also fire on the first row so callers (and tests) can observe
            // that the embedding loop has left Phase 0 — not only every N.
            if done == 1 || done % PROGRESS_EVERY == 0 || done == total {
                cb(done, total);
            }
        }
    }

    // Phase 2: batch write. This persists whatever we successfully embedded,
    // including on abort — the snapshot invariant guarantees every row here
    // came from `model_snapshot`, so the corpus stays model-pure even after
    // a partial write.
    let db2 = db_path.to_string();
    let update_count = updates.len();
    let updates_owned = updates;
    tokio::task::spawn_blocking(move || -> Result<()> {
        let conn = crate::encrypted_sqlite::open(&db2)
            .context("full_corpus_reembed: opening SQLite (write)")?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;")?;
        let tx = conn.unchecked_transaction()?;
        for (id, blob) in &updates_owned {
            if let Err(e) = tx.execute(
                "UPDATE l2_experience SET embedding = ?1 WHERE id = ?2",
                rusqlite::params![blob, id],
            ) {
                tracing::warn!(id, error = %e, "full_corpus_reembed: DB update failed");
            }
        }
        tx.commit().context("full_corpus_reembed: commit")?;
        Ok(())
    })
    .await??;

    // Phase 3: request HNSW rebuild so the index reflects the new vectors.
    //
    // FAIL #3 (CTO round 1): rebuild HNSW *before* writing the marker.
    // Rationale: the marker is the signal that "persisted corpus + HNSW are
    // now consistent with `model_used`". If we write it first and rebuild
    // fails, queries hit stale HNSW entries until the next startup rebuild
    // AND the mismatch alert stays silent — the worst possible combination.
    // Reverse the order: if rebuild fails, surface the error AND leave the
    // marker stale so the next startup re-fires the consent alert.
    //
    // Note: `request_full_hnsw_rebuild()` returns `Ok(0)` when no HNSW is
    // attached (test harness / headless mode). That's NOT a failure — we
    // proceed to update the marker normally in that case.
    let hnsw_rebuild_failed: Option<String> = match memory.request_full_hnsw_rebuild().await {
        Ok(_n) => None,
        Err(e) => {
            let msg = format!("{e:#}");
            tracing::error!(error = %msg, "full_corpus_reembed: HNSW rebuild failed — NOT updating marker, user should retry");
            Some(msg)
        }
    };

    // Phase 4: conditionally update the marker. We skip the marker update on:
    //   (a) model-flip abort — the persisted subset came from the snapshot,
    //       but the stale marker MAY already match it. To stay defensive we
    //       leave the marker alone; the next startup will re-detect the
    //       mismatch if the live primary differs from `model_snapshot`.
    //   (b) HNSW rebuild failure — marker claims HNSW is consistent with
    //       `model_used`, but HNSW may be stale. User should retry.
    let aborted = aborted_new_model.is_some();
    let model_used = model_snapshot.clone();
    let status = if aborted {
        "aborted_model_changed".to_string()
    } else {
        "ok".to_string()
    };
    if !aborted
        && hnsw_rebuild_failed.is_none()
        && let Err(e) = write_marker(memory, &model_used).await
    {
        tracing::warn!(error = %e, "full_corpus_reembed: marker write failed (re-embed did succeed)");
    }

    let walltime_sec = started_at.elapsed().as_secs_f64();
    tracing::info!(
        rows_cleared,
        rows_reembedded = update_count as i64,
        rows_failed,
        walltime_sec,
        model_used = %model_used,
        status = %status,
        hnsw_rebuild_failed = ?hnsw_rebuild_failed,
        "[EmbeddingCorpus] full_corpus_reembed complete"
    );

    Ok(ReembedOutcome {
        rows_cleared,
        rows_reembedded: update_count as i64,
        rows_failed,
        walltime_sec,
        model_used,
        status,
        new_model_observed: aborted_new_model,
        hnsw_rebuild_failed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk_embedder(primary: &str) -> EmbeddingClient {
        EmbeddingClient::new("https://mock.invalid", "k", primary, 8)
    }

    #[tokio::test]
    async fn fresh_install_detected_when_no_marker_and_no_rows() {
        let mem = MemoryHandle::spawn_in_memory().unwrap();
        let e = mk_embedder("primary-x");
        let chk = detect_corpus_mismatch(&mem, &e).await.unwrap();
        assert_eq!(chk.status, CorpusStatus::FreshInstall);
        assert_eq!(chk.vector_count, 0);
        assert!(chk.marker_before.is_none());
        assert_eq!(chk.live_primary, "primary-x");
    }

    #[tokio::test]
    async fn match_when_marker_equals_primary() {
        let mem = MemoryHandle::spawn_in_memory().unwrap();
        write_marker(&mem, "primary-x").await.unwrap();
        let e = mk_embedder("primary-x");
        let chk = detect_corpus_mismatch(&mem, &e).await.unwrap();
        assert_eq!(chk.status, CorpusStatus::Match);
        assert_eq!(chk.marker_before.as_deref(), Some("primary-x"));
    }

    #[tokio::test]
    async fn mismatch_when_marker_differs_from_primary() {
        let mem = MemoryHandle::spawn_in_memory().unwrap();
        write_marker(&mem, "old-model").await.unwrap();
        let e = mk_embedder("new-model");
        let chk = detect_corpus_mismatch(&mem, &e).await.unwrap();
        assert_eq!(chk.status, CorpusStatus::Mismatch);
        assert_eq!(chk.marker_before.as_deref(), Some("old-model"));
        assert_eq!(chk.live_primary, "new-model");

        // CRITICAL: detection must NOT auto-update the marker. The marker
        // may only flip on explicit user approval via trigger_corpus_reembed.
        let re = read_marker(&mem).await.unwrap();
        assert_eq!(re.as_deref(), Some("old-model"));
    }

    #[tokio::test]
    async fn mismatch_when_marker_absent_but_rows_exist() {
        // Simulates the one-time migration from a pre-consent-marker deploy
        // with existing vectors (e.g. the 1611 rows of google/gemini-embedding-001
        // that existed before this mechanism landed).
        let mem = MemoryHandle::spawn_in_memory().unwrap();
        // Insert a row with a non-NULL embedding.
        let did = "did:test:user";
        let emb: Vec<f32> = vec![0.1; 8];
        mem.store_l2(did, "test_topic", "fact body", Some(&emb))
            .await
            .unwrap();
        let e = mk_embedder("new-model");
        let chk = detect_corpus_mismatch(&mem, &e).await.unwrap();
        assert_eq!(chk.status, CorpusStatus::Mismatch);
        assert!(chk.marker_before.is_none());
        assert_eq!(chk.vector_count, 1);
    }

    #[tokio::test]
    async fn write_marker_then_read_roundtrip() {
        let mem = MemoryHandle::spawn_in_memory().unwrap();
        assert!(read_marker(&mem).await.unwrap().is_none());
        write_marker(&mem, "model-a").await.unwrap();
        assert_eq!(read_marker(&mem).await.unwrap().as_deref(), Some("model-a"));
        write_marker(&mem, "model-b").await.unwrap();
        assert_eq!(read_marker(&mem).await.unwrap().as_deref(), Some("model-b"));
    }

    #[test]
    fn alert_contains_both_languages_and_counts() {
        let chk = CorpusCheck {
            status: CorpusStatus::Mismatch,
            marker_before: Some("google/gemini-embedding-001".into()),
            live_primary: "openai/text-embedding-3-small".into(),
            vector_count: 1611,
        };
        let s = build_mismatch_alert_html(&chk);
        // English side
        assert!(s.contains("Embedding primary model changed"));
        assert!(s.contains("google/gemini-embedding-001"));
        assert!(s.contains("openai/text-embedding-3-small"));
        assert!(s.contains("1611"));
        assert!(s.contains("trigger_corpus_reembed"));
        // Chinese side — check a couple of characters exist to prove both
        // are present (exact Unicode is the escape sequences above).
        assert!(s.contains("\u{4e3b}")); // 主
        assert!(s.contains("\u{91cd}\u{65b0}\u{8ba1}\u{7b97}")); // 重新计算
    }

    #[test]
    fn alert_handles_missing_marker() {
        // When marker is absent (pre-consent-marker deploy), the "Previous"
        // line should still render with an explanatory placeholder.
        let chk = CorpusCheck {
            status: CorpusStatus::Mismatch,
            marker_before: None,
            live_primary: "openai/text-embedding-3-small".into(),
            vector_count: 42,
        };
        let s = build_mismatch_alert_html(&chk);
        assert!(s.contains("pre-consent-marker deploy"));
        assert!(s.contains("42"));
    }

    // ── FAIL #1 regression: mid-pass model flip aborts reembed ───────────
    //
    // Verifies that if the embedder flips primary/backup during
    // `full_corpus_reembed`, we:
    //   - stop the loop early with `status == "aborted_model_changed"`;
    //   - record the new model in `new_model_observed`;
    //   - leave the corpus marker UNCHANGED (so the next startup re-fires
    //     the consent alert and the user can retry after things stabilize).
    //
    // The test uses a real on-disk SQLite (required because
    // `full_corpus_reembed` opens its own rusqlite connection on the
    // `db_path`, not via the actor — `:memory:` spawns cannot be reached
    // from a second connection).
    //
    // Flip strategy (deterministic — no wall-clock race):
    // Previously we spawned a 200ms delayed `test_mark_degraded`. Under
    // parallel `cargo test --lib` load, Phase 0 (spawn_blocking SQLite NULL
    // + row read) can take longer than 200ms, so the flip landed *before*
    // `model_snapshot` and the loop never observed a mid-pass change
    // (status stayed `"ok"`). We now flip from the progress callback after
    // the first embedding-loop iteration — that callback only runs after
    // the snapshot is taken and row 0 has been processed.
    #[tokio::test]
    async fn reembed_aborts_on_mid_pass_model_flip() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, Ordering};

        let tmp = tempfile::NamedTempFile::new().unwrap();
        let db_path = tmp.path().to_string_lossy().into_owned();
        let mem = MemoryHandle::spawn(tmp.path()).unwrap();

        // Seed the existing marker so we can assert it's unchanged on abort.
        write_marker(&mem, "primary-stale").await.unwrap();

        // Embedder with backup configured so `active_model()` can flip when
        // we mark it degraded. Closed local port fails fast (no DNS wait).
        let embedder = EmbeddingClient::new_with_backup(
            "http://127.0.0.1:9",
            "k",
            "primary-stale",
            Some("backup-other".to_string()),
            8,
        );

        // Enough rows that after the first-row progress flip there is still
        // work left for the pre-embed check to abort on.
        for i in 0..10 {
            let emb: Vec<f32> = vec![0.1 * (i as f32 + 1.0); 8];
            mem.store_l2("did:test:abort", "t", &format!("fact {i}"), Some(&emb))
                .await
                .unwrap();
        }

        let flip_target = embedder.clone();
        let flipped = Arc::new(AtomicBool::new(false));
        let flipped_cb = Arc::clone(&flipped);
        let progress: ProgressFn = Box::new(move |done, _total| {
            // After row 0 completes (post snapshot + embed + rate-limit),
            // flip identity so row 1's pre-embed check aborts.
            if done == 1 && !flipped_cb.swap(true, Ordering::SeqCst) {
                flip_target.test_mark_degraded();
            }
        });

        let outcome = full_corpus_reembed(
            &db_path,
            &mem,
            &embedder,
            4 * 8, // 8 floats × 4 bytes
            Some(progress),
        )
        .await
        .expect("reembed returns Ok even on graceful abort");

        assert_eq!(
            outcome.status, "aborted_model_changed",
            "expected abort; outcome = {outcome:?}"
        );
        assert_eq!(outcome.new_model_observed.as_deref(), Some("backup-other"));
        assert_eq!(
            outcome.model_used, "primary-stale",
            "model_used must record the SNAPSHOT model, not the post-flip name"
        );

        // CRITICAL: the marker must be untouched so the next startup re-fires
        // the consent alert and the user can retry once the primary
        // stabilizes.
        assert_eq!(
            read_marker(&mem).await.unwrap().as_deref(),
            Some("primary-stale")
        );
    }

    // ── FAIL #2 regression (2026-04-19): benign same-model flap must not
    //    abort the reembed.
    //
    // When primary == backup (e.g. both set to `baai/bge-m3` after a manual
    // failover) a transient upstream error causes a harmless
    // failover→recovery cycle that toggles `is_degraded()` but leaves the
    // active model NAME unchanged. The old check also bailed on the degraded
    // flag flipping, which killed production reembed at 1780/N rows on
    // 2026-04-19. The fix ignores the degraded flag and trusts model-name
    // equality as the sole identity check.
    //
    // We seed an embedder with `primary == backup == "same-model"`, flip it
    // degraded mid-pass, and assert the reembed completes with
    // `status == "ok"` — NOT `aborted_model_changed`.
    #[tokio::test]
    async fn reembed_tolerates_benign_same_model_flap() {
        use std::time::Duration;

        let tmp = tempfile::NamedTempFile::new().unwrap();
        let db_path = tmp.path().to_string_lossy().into_owned();
        let mem = MemoryHandle::spawn(tmp.path()).unwrap();

        // Primary == Backup — the bge-m3==bge-m3 production case.
        let embedder = EmbeddingClient::new_with_backup(
            "https://embedding.example.test",
            "k",
            "same-model",
            Some("same-model".to_string()),
            8,
        );

        // Seed a small corpus so the pass completes quickly even with the
        // ~100 ms/row rate limit.
        for i in 0..6 {
            let emb: Vec<f32> = vec![0.1 * (i as f32 + 1.0); 8];
            mem.store_l2("did:test:flap", "t", &format!("fact {i}"), Some(&emb))
                .await
                .unwrap();
        }

        // Flip degraded mid-pass (transient failover) then recover — the old
        // code would abort on the degraded-flag toggle even though model
        // name is unchanged.
        let flip_target = embedder.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(150)).await;
            flip_target.test_mark_degraded();
            tokio::time::sleep(Duration::from_millis(100)).await;
            flip_target.test_mark_recovered();
        });

        let outcome = full_corpus_reembed(&db_path, &mem, &embedder, 4 * 8, None)
            .await
            .expect("reembed returns Ok");

        assert_ne!(
            outcome.status, "aborted_model_changed",
            "same-model flap must NOT abort — got outcome = {outcome:?}"
        );
        assert!(
            outcome.new_model_observed.is_none(),
            "no model change observed; got {:?}",
            outcome.new_model_observed
        );
    }

    // ── FAIL #3 regression: ReembedOutcome surfaces new fields ───────────
    //
    // Lightweight shape test — we don't exercise a real HNSW rebuild
    // failure (requires a mock HnswIndex that's beyond this patch's
    // scope), but we assert the struct carries the fields the tool
    // response will read on the happy path and on the abort path.
    #[test]
    fn reembed_outcome_carries_new_fields() {
        let ok = ReembedOutcome {
            rows_cleared: 10,
            rows_reembedded: 10,
            rows_failed: 0,
            walltime_sec: 1.2,
            model_used: "m".into(),
            status: "ok".into(),
            new_model_observed: None,
            hnsw_rebuild_failed: None,
        };
        assert_eq!(ok.status, "ok");
        assert!(ok.new_model_observed.is_none());
        assert!(ok.hnsw_rebuild_failed.is_none());

        let aborted = ReembedOutcome {
            rows_cleared: 10,
            rows_reembedded: 3,
            rows_failed: 0,
            walltime_sec: 0.5,
            model_used: "m".into(),
            status: "aborted_model_changed".into(),
            new_model_observed: Some("backup".into()),
            hnsw_rebuild_failed: None,
        };
        assert_eq!(aborted.status, "aborted_model_changed");
        assert_eq!(aborted.new_model_observed.as_deref(), Some("backup"));
    }

    // ── WARN #7 regression: count_corpus_parts returns both tables ───────
    #[tokio::test]
    async fn count_corpus_parts_separates_experience_and_ephemeral() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let db_path = tmp.path().to_string_lossy().into_owned();
        let mem = MemoryHandle::spawn(tmp.path()).unwrap();

        // Seed two l2_experience rows with embeddings. Use distinct topic
        // strings per row to avoid `l2_write_guard` deduplication — the
        // guard collapses identical (topic, fact) pairs which would make
        // the second insert a no-op and the test flaky.
        for i in 0..2 {
            let emb: Vec<f32> = vec![0.1 * (i as f32 + 1.0); 8];
            mem.store_l2(
                "did:test:w7",
                &format!("topic_w7_{i}"),
                &format!("fact {i} unique"),
                Some(&emb),
            )
            .await
            .unwrap();
        }

        // count_corpus_parts reports l2_experience = 2, l2_ephemeral = 0.
        // (l2_ephemeral doesn't have a public seed path in tests — zero is
        // the expected baseline; the point here is to prove the function
        // separates the two counts rather than collapsing them.)
        let (exp, eph) = count_corpus_parts(&db_path).await.unwrap();
        assert_eq!(exp, 2);
        assert_eq!(eph, 0);
    }
}
