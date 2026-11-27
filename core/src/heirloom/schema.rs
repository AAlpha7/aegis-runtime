//! Heirloom event log + projection table DDL.
//!
//! Phase C1 (W9): defines `heirloom_event_log` + 3 projection tables. The
//! `ensure_tables()` function is idempotent — can be called at every startup;
//! existing tables are unaltered.
//!
//! Phase E (W17 migration) wires this from `memory_migrations.rs`.

use rusqlite::Connection;

/// Run all heirloom DDL statements in idempotent mode. Safe to call on every
/// startup; safe to call multiple times in a single process.
pub fn ensure_tables(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(DDL)?;
    // Phase E (W17) — persistent divergence counter. FIX 4 CTO Round 1.
    // Tracks cumulative soft-fail counts across process restarts so operators
    // can query post-deploy: `sqlite3 /opt/aegis-runtime/shared/heirloom_event_log.db
    //   "SELECT value FROM heirloom_divergence_counter WHERE key='bridge_soft_fail_total'"`
    conn.execute(
        "CREATE TABLE IF NOT EXISTS heirloom_divergence_counter (
             key TEXT PRIMARY KEY,
             value INTEGER NOT NULL DEFAULT 0
         ) STRICT;",
        [],
    )?;
    Ok(())
}

/// Batched DDL covering all 4 tables + their indexes. Kept as one string so
/// `execute_batch` processes them in-order within an implicit transaction.
pub const DDL: &str = r#"
CREATE TABLE IF NOT EXISTS heirloom_event_log (
    event_hash              BLOB PRIMARY KEY NOT NULL,
    prev_hash               BLOB NOT NULL,
    event_id_uuid           TEXT NOT NULL,
    schema_version          INTEGER NOT NULL,
    event_type              TEXT NOT NULL,
    hlc_phys                INTEGER NOT NULL,
    hlc_log                 INTEGER NOT NULL,
    observed_at_rfc3339     TEXT NOT NULL,
    l0_hash                 BLOB NOT NULL,
    l1_hash                 BLOB NOT NULL,
    l2_manifest_hash        BLOB NOT NULL,
    lineage_id              TEXT NOT NULL,
    author_did              TEXT NOT NULL,
    sensitivity_tier        TEXT NOT NULL,
    bundle_ids_json         TEXT NOT NULL,
    payload_encoding_json   TEXT NOT NULL,
    payload                 BLOB NOT NULL,
    signatures_json         TEXT NOT NULL,
    envelope_canonical      BLOB NOT NULL,
    inserted_at_unix_ms     INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_heirloom_event_log_hlc
    ON heirloom_event_log (hlc_phys, hlc_log);
CREATE INDEX IF NOT EXISTS idx_heirloom_event_log_event_type
    ON heirloom_event_log (event_type);
CREATE INDEX IF NOT EXISTS idx_heirloom_event_log_author_did
    ON heirloom_event_log (author_did);
CREATE INDEX IF NOT EXISTS idx_heirloom_event_log_prev_hash
    ON heirloom_event_log (prev_hash);

CREATE TABLE IF NOT EXISTS heirloom_bundle_index (
    bundle_id       TEXT NOT NULL,
    event_hash      BLOB NOT NULL,
    hlc_phys        INTEGER NOT NULL,
    PRIMARY KEY (bundle_id, event_hash),
    FOREIGN KEY (event_hash) REFERENCES heirloom_event_log(event_hash)
);
CREATE INDEX IF NOT EXISTS idx_heirloom_bundle_index_bundle_hlc
    ON heirloom_bundle_index (bundle_id, hlc_phys);

CREATE TABLE IF NOT EXISTS heirloom_entity_index (
    entity_did      TEXT NOT NULL,
    event_hash      BLOB NOT NULL,
    role            TEXT NOT NULL,
    hlc_phys        INTEGER NOT NULL,
    PRIMARY KEY (entity_did, event_hash, role),
    FOREIGN KEY (event_hash) REFERENCES heirloom_event_log(event_hash)
);
CREATE INDEX IF NOT EXISTS idx_heirloom_entity_index_did_hlc
    ON heirloom_entity_index (entity_did, hlc_phys);

CREATE TABLE IF NOT EXISTS heirloom_schema_registry (
    kind                TEXT NOT NULL,
    id                  TEXT NOT NULL,
    payload_json        TEXT NOT NULL,
    valid_from_phys     INTEGER NOT NULL,
    valid_from_log      INTEGER NOT NULL,
    valid_until_phys    INTEGER,
    valid_until_log     INTEGER,
    registered_event_hash BLOB NOT NULL,
    PRIMARY KEY (kind, id),
    FOREIGN KEY (registered_event_hash) REFERENCES heirloom_event_log(event_hash)
);
CREATE INDEX IF NOT EXISTS idx_heirloom_schema_registry_kind
    ON heirloom_schema_registry (kind);

-- Phase C3 Task 11: Hebbian access_count projection (spec §4 Q4.5).
-- Real-time bump; daily batch rolls up into ai:access_summary events.
CREATE TABLE IF NOT EXISTS heirloom_access_count (
    event_hash          BLOB PRIMARY KEY NOT NULL,
    count               INTEGER NOT NULL DEFAULT 0,
    last_hit_rfc3339    TEXT NOT NULL,
    FOREIGN KEY (event_hash) REFERENCES heirloom_event_log(event_hash)
);
CREATE INDEX IF NOT EXISTS idx_heirloom_access_count_count
    ON heirloom_access_count (count DESC);

-- Phase C3 Task 11 also adds a lineage index to heirloom_event_log to
-- avoid table-scans in load_candidates. Existing index covers
-- (hlc_phys, hlc_log); reader queries filter by lineage_id first, then
-- order by HLC. At 50M events a table scan per query would be catastrophic.
CREATE INDEX IF NOT EXISTS idx_heirloom_event_log_lineage
    ON heirloom_event_log (lineage_id, hlc_phys);
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn fresh_conn() -> Connection {
        Connection::open_in_memory().expect("open in-memory sqlite")
    }

    #[test]
    fn t9_ensure_tables_creates_all_four_tables() {
        let conn = fresh_conn();
        ensure_tables(&conn).expect("ensure_tables");
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table'
                 AND name IN ('heirloom_event_log', 'heirloom_bundle_index',
                              'heirloom_entity_index', 'heirloom_schema_registry')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 4);
    }

    #[test]
    fn t9_ensure_tables_is_idempotent_across_repeat_calls() {
        let conn = fresh_conn();
        ensure_tables(&conn).expect("first call");
        ensure_tables(&conn).expect("second call");
        ensure_tables(&conn).expect("third call");
        // All six tables still exist and no duplicate-key or syntax error.
        // Six: heirloom_event_log + 3 Phase C1 projections + heirloom_access_count (C3)
        // + heirloom_divergence_counter (Phase E).
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table'
                 AND name LIKE 'heirloom_%'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 6);
    }

    #[test]
    fn t9_ensure_tables_creates_expected_indexes() {
        let conn = fresh_conn();
        ensure_tables(&conn).unwrap();
        let index_names: Vec<String> = conn
            .prepare(
                "SELECT name FROM sqlite_master WHERE type = 'index'
                 AND name LIKE 'idx_heirloom_%' ORDER BY name",
            )
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();

        // 4 on event_log (C1) + 1 on bundle_index + 1 on entity_index + 1 on
        // schema_registry = 7. Phase C3 Task 11 adds 2 more:
        // idx_heirloom_event_log_lineage + idx_heirloom_access_count_count = 9.
        assert_eq!(index_names.len(), 9, "got indexes: {index_names:?}");
        assert!(index_names.contains(&"idx_heirloom_event_log_hlc".to_string()));
        assert!(index_names.contains(&"idx_heirloom_schema_registry_kind".to_string()));
    }

    #[test]
    fn t9_event_log_table_rejects_null_required_columns() {
        let conn = fresh_conn();
        ensure_tables(&conn).unwrap();
        // Try inserting a row with all NULLs — every NOT NULL column should fire.
        let err = conn.execute(
            "INSERT INTO heirloom_event_log (event_hash) VALUES (NULL)",
            [],
        );
        assert!(err.is_err(), "NULL event_hash must be rejected");
    }

    #[test]
    fn t9_bundle_index_foreign_key_references_event_log() {
        let conn = fresh_conn();
        ensure_tables(&conn).unwrap();
        // Enabling foreign keys mode (off by default in SQLite for legacy reasons).
        conn.execute("PRAGMA foreign_keys = ON", []).unwrap();

        // Insert into bundle_index without a matching event_log row → FK violation.
        let err = conn.execute(
            "INSERT INTO heirloom_bundle_index (bundle_id, event_hash, hlc_phys)
             VALUES ('bundle:general', X'00', 0)",
            [],
        );
        assert!(
            err.is_err(),
            "orphan bundle_index row must be rejected when FKs on"
        );
    }

    #[test]
    fn t_e1_5_ensure_tables_creates_divergence_counter() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_tables(&conn).unwrap();
        // Row absent until first increment — just confirm table exists.
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='heirloom_divergence_counter'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            n, 1,
            "heirloom_divergence_counter must be created by ensure_tables"
        );
    }

    #[test]
    fn t9_ensure_tables_direct_call_creates_heirloom_tables() {
        // Direct-call sanity test (NOT a wiring regression test — see note below).
        //
        // Task 8 Step 2 adds `heirloom::schema::ensure_tables(conn)?` to the
        // migration runner. The integration test `t9_ensure_tables_direct_call_*`
        // here verifies `ensure_tables` alone works on a fresh DB.
        //
        // **Wiring regression is protected transitively**: ~14 existing
        // `memory.rs` / `memory_migrations.rs` tests call `run_all_migrations`
        // and touch heirloom tables via downstream flows; if the wiring call
        // were removed, those tests would start seeing "no such table:
        // heirloom_*" errors.
        let conn = Connection::open_in_memory().unwrap();
        ensure_tables(&conn).expect("ensure_tables must succeed on a fresh DB");
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master
                 WHERE type='table' AND name LIKE 'heirloom_%'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 6);
    }
}
