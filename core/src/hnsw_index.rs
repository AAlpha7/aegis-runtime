//! HNSW (Hierarchical Navigable Small World) vector index for O(log N) approximate
//! nearest neighbor search over L2 experience embeddings.
//!
//! Replaces the O(N) SQL `vec_cosine_sim` full-table scan with a persistent,
//! incrementally-updatable HNSW graph. Supports concurrent reads and parallel
//! batch search via rayon.
//!
//! Architecture:
//! - HNSW index lives in-memory, rebuilt from SQLite on startup
//! - On `store_l2`: insert into both SQLite (source of truth) + HNSW (fast search)
//! - On `query_l2_semantic`: use HNSW instead of SQL scan
//! - On `delete_l2`: mark deleted; periodic rebuild prunes them
//! - Periodic dump to disk for fast restart (avoids full rebuild)

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use anyhow::{Context, Result};
use hnsw_rs::hnsw::Hnsw;
use parking_lot::RwLock;

/// Cosine distance (1.0 - cosine_similarity). HNSW minimizes distance,
/// so we need distance = 1 - similarity.
use anndists::dist::distances::DistCosine;

/// Result of an ANN search: (l2_experience row ID, cosine similarity).
#[derive(Debug, Clone)]
pub struct AnnResult {
    pub l2_id: i64,
    pub similarity: f32,
}

/// Configuration for the HNSW index.
#[derive(Debug, Clone)]
pub struct HnswConfig {
    /// Max connections per node per layer (higher = more accurate, more memory).
    /// Typical: 16-48. Default: 24.
    pub max_nb_connection: usize,
    /// Max layers in the graph. Default: 16.
    pub max_layer: usize,
    /// Exploration factor during construction (higher = slower build, better recall).
    /// Default: 400.
    pub ef_construction: usize,
    /// Exploration factor during search (higher = slower search, better recall).
    /// Default: 64.
    pub ef_search: usize,
    /// Path for persisting the index to disk. None = no persistence.
    pub dump_dir: Option<PathBuf>,
}

impl Default for HnswConfig {
    fn default() -> Self {
        Self {
            max_nb_connection: 24,
            max_layer: 16,
            ef_construction: 400,
            ef_search: 64,
            dump_dir: None,
        }
    }
}

/// Thread-safe wrapper around the HNSW index.
///
/// The index maps `data_id` (sequential usize) → `l2_id` (SQLite row ID).
/// A reverse map allows lookup by l2_id for deletion tracking.
pub struct HnswIndex {
    /// The HNSW graph. Protected by RwLock for concurrent reads.
    /// Uses `'static` lifetime because we own all point data.
    inner: RwLock<Option<Hnsw<'static, f32, DistCosine>>>,
    /// Maps HNSW data_id → L2 experience row ID.
    id_map: RwLock<Vec<i64>>,
    /// Set of deleted L2 IDs (lazily pruned on rebuild).
    deleted: RwLock<std::collections::HashSet<i64>>,
    /// Next data_id to assign.
    next_id: AtomicU64,
    /// Whether the index is ready for queries.
    ready: AtomicBool,
    /// Configuration.
    config: HnswConfig,
    /// Embedding dimensions (set on first insert).
    dimensions: AtomicU64,
}

impl HnswIndex {
    /// Create a new empty HNSW index.
    pub fn new(config: HnswConfig) -> Self {
        Self {
            inner: RwLock::new(None),
            id_map: RwLock::new(Vec::new()),
            deleted: RwLock::new(std::collections::HashSet::new()),
            next_id: AtomicU64::new(0),
            ready: AtomicBool::new(false),
            config,
            dimensions: AtomicU64::new(0),
        }
    }

    /// Build the index from existing L2 embeddings in SQLite.
    /// Called once on startup. Blocks until complete.
    pub fn build_from_db(&self, conn: &rusqlite::Connection, _did: &str) -> Result<usize> {
        // Load ALL embeddings regardless of DID — HNSW is a global index.
        // DID filtering is applied later in hnsw_fetch_experiences() via SQL.
        let mut stmt = conn
            .prepare(
                "SELECT id, embedding FROM l2_experience \
             WHERE embedding IS NOT NULL AND topic != 'l1_overflow' \
             ORDER BY id ASC",
            )
            .context("preparing HNSW build query")?;

        let rows: Vec<(i64, Vec<u8>)> = stmt
            .query_map([], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?))
            })
            .context("querying embeddings for HNSW build")?
            .filter_map(|r| r.ok())
            .collect();

        if rows.is_empty() {
            self.ready.store(true, Ordering::Release);
            return Ok(0);
        }

        // Determine dimensions from first embedding
        let dims = rows[0].1.len() / 4;
        self.dimensions.store(dims as u64, Ordering::Release);

        // Convert blobs to f32 vectors
        let embeddings: Vec<(i64, Vec<f32>)> = rows
            .iter()
            .filter_map(|(id, blob)| {
                if blob.len() % 4 != 0 || blob.len() / 4 != dims {
                    return None; // Skip mismatched dimensions
                }
                let vec: Vec<f32> = blob
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                    .collect();
                Some((*id, vec))
            })
            .collect();

        let count = embeddings.len();

        // Create HNSW index
        let hnsw = Hnsw::<f32, DistCosine>::new(
            self.config.max_nb_connection,
            count.max(1000), // hint for allocation
            self.config.max_layer,
            self.config.ef_construction,
            DistCosine,
        );

        // Build id_map and insert in parallel
        let mut id_map = Vec::with_capacity(count);
        let insert_data: Vec<(&Vec<f32>, usize)> = embeddings
            .iter()
            .enumerate()
            .map(|(idx, (l2_id, vec))| {
                id_map.push(*l2_id);
                (vec, idx)
            })
            .collect();

        // Parallel bulk insert
        hnsw.parallel_insert(&insert_data);

        self.next_id.store(count as u64, Ordering::Release);
        *self.id_map.write() = id_map;
        *self.inner.write() = Some(hnsw);
        self.ready.store(true, Ordering::Release);

        tracing::info!(count, dims, "HNSW index built from DB");
        Ok(count)
    }

    /// Insert a single embedding into the index.
    /// Returns the assigned data_id.
    pub fn insert(&self, l2_id: i64, embedding: &[f32]) -> Option<usize> {
        let dims = self.dimensions.load(Ordering::Acquire) as usize;
        if dims == 0 {
            // First insert sets dimensions
            self.dimensions
                .store(embedding.len() as u64, Ordering::Release);
        } else if embedding.len() != dims {
            tracing::warn!(
                expected = dims,
                got = embedding.len(),
                "HNSW insert: dimension mismatch, skipping"
            );
            return None;
        }

        let data_id = self.next_id.fetch_add(1, Ordering::AcqRel) as usize;

        // Extend id_map
        {
            let mut map = self.id_map.write();
            if data_id >= map.len() {
                map.resize(data_id + 1, -1);
            }
            map[data_id] = l2_id;
        }

        // Insert into HNSW (need write lock since we might need to init)
        {
            let mut guard = self.inner.write();
            if guard.is_none() {
                // Lazy init on first insert
                *guard = Some(Hnsw::<f32, DistCosine>::new(
                    self.config.max_nb_connection,
                    10_000,
                    self.config.max_layer,
                    self.config.ef_construction,
                    DistCosine,
                ));
            }
            if let Some(ref hnsw) = *guard {
                hnsw.insert_slice((embedding, data_id));
            }
        }

        // Remove from deleted set if re-inserted
        self.deleted.write().remove(&l2_id);

        Some(data_id)
    }

    /// Mark an L2 ID as deleted. The point remains in the HNSW graph
    /// but is filtered out of search results. Pruned on next rebuild.
    pub fn mark_deleted(&self, l2_id: i64) {
        self.deleted.write().insert(l2_id);
    }

    /// Search for the k nearest neighbors of a query embedding.
    /// Returns results sorted by descending similarity (highest first).
    pub fn search(&self, query: &[f32], k: usize, min_similarity: f32) -> Vec<AnnResult> {
        if !self.ready.load(Ordering::Acquire) {
            return Vec::new();
        }

        let guard = self.inner.read();
        let hnsw = match guard.as_ref() {
            Some(h) => h,
            None => return Vec::new(),
        };

        let ef = self.config.ef_search.max(k);
        let neighbours = hnsw.search(query, k, ef);

        let id_map = self.id_map.read();
        let deleted = self.deleted.read();

        let mut results: Vec<AnnResult> = neighbours
            .into_iter()
            .filter_map(|n| {
                let data_id = n.d_id;
                if data_id >= id_map.len() {
                    return None;
                }
                let l2_id = id_map[data_id];
                if l2_id < 0 || deleted.contains(&l2_id) {
                    return None;
                }
                // DistCosine returns 1.0 - cosine_similarity
                let similarity = 1.0 - n.distance;
                if similarity < min_similarity {
                    return None;
                }
                Some(AnnResult { l2_id, similarity })
            })
            .collect();

        // Sort by similarity descending
        results.sort_by(|a, b| {
            b.similarity
                .partial_cmp(&a.similarity)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        results
    }

    /// Parallel batch search (for multiple queries at once).
    /// Each query returns up to k results.
    pub fn parallel_search(
        &self,
        queries: &[Vec<f32>],
        k: usize,
        min_similarity: f32,
    ) -> Vec<Vec<AnnResult>> {
        if !self.ready.load(Ordering::Acquire) || queries.is_empty() {
            return vec![Vec::new(); queries.len()];
        }

        let guard = self.inner.read();
        let hnsw = match guard.as_ref() {
            Some(h) => h,
            None => return vec![Vec::new(); queries.len()],
        };

        let ef = self.config.ef_search.max(k);
        let all_neighbours = hnsw.parallel_search(queries, k, ef);

        let id_map = self.id_map.read();
        let deleted = self.deleted.read();

        all_neighbours
            .into_iter()
            .map(|neighbours| {
                let mut results: Vec<AnnResult> = neighbours
                    .into_iter()
                    .filter_map(|n| {
                        let data_id = n.d_id;
                        if data_id >= id_map.len() {
                            return None;
                        }
                        let l2_id = id_map[data_id];
                        if l2_id < 0 || deleted.contains(&l2_id) {
                            return None;
                        }
                        let similarity = 1.0 - n.distance;
                        if similarity < min_similarity {
                            return None;
                        }
                        Some(AnnResult { l2_id, similarity })
                    })
                    .collect();
                results.sort_by(|a, b| {
                    b.similarity
                        .partial_cmp(&a.similarity)
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
                results
            })
            .collect()
    }

    /// Returns the number of points in the index (including deleted).
    pub fn len(&self) -> usize {
        self.next_id.load(Ordering::Acquire) as usize
    }

    /// Returns the number of deleted points pending prune.
    pub fn deleted_count(&self) -> usize {
        self.deleted.read().len()
    }

    /// Whether the index is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Whether the index is ready for queries.
    pub fn is_ready(&self) -> bool {
        self.ready.load(Ordering::Acquire)
    }

    /// Ratio of deleted entries to total entries. Used to decide when rebuild is needed.
    pub fn deleted_ratio(&self) -> f32 {
        let total = self.len();
        if total == 0 {
            return 0.0;
        }
        self.deleted_count() as f32 / total as f32
    }

    /// Returns true if the deleted ratio exceeds the rebuild threshold (20%).
    /// Caller should trigger a full rebuild via `build_from_db` when this returns true.
    pub fn needs_rebuild(&self) -> bool {
        const REBUILD_THRESHOLD: f32 = 0.20;
        const MIN_DELETED_FOR_REBUILD: usize = 50;
        self.deleted_count() >= MIN_DELETED_FOR_REBUILD && self.deleted_ratio() > REBUILD_THRESHOLD
    }

    /// Clear the deleted set. Call after a successful `build_from_db` rebuild,
    /// since the rebuilt index no longer contains the old deleted entries.
    pub fn clear_deleted(&self) {
        self.deleted.write().clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_index() -> HnswIndex {
        HnswIndex::new(HnswConfig {
            max_nb_connection: 16,
            max_layer: 8,
            ef_construction: 200,
            ef_search: 32,
            dump_dir: None,
        })
    }

    #[test]
    fn insert_and_search() {
        let idx = make_index();
        idx.ready.store(true, Ordering::Release);

        // Insert 3 vectors
        idx.insert(1, &[1.0, 0.0, 0.0]);
        idx.insert(2, &[0.0, 1.0, 0.0]);
        idx.insert(3, &[0.9, 0.1, 0.0]); // similar to #1

        let results = idx.search(&[1.0, 0.0, 0.0], 3, 0.0);
        assert!(!results.is_empty());
        // First result should be ID 1 (exact match)
        assert_eq!(results[0].l2_id, 1);
        assert!(results[0].similarity > 0.99);
    }

    #[test]
    fn search_with_deletion() {
        let idx = make_index();
        idx.ready.store(true, Ordering::Release);

        idx.insert(10, &[1.0, 0.0, 0.0]);
        idx.insert(20, &[0.9, 0.1, 0.0]);

        idx.mark_deleted(10);

        let results = idx.search(&[1.0, 0.0, 0.0], 5, 0.0);
        // ID 10 should be filtered out
        assert!(results.iter().all(|r| r.l2_id != 10));
    }

    #[test]
    fn similarity_threshold() {
        let idx = make_index();
        idx.ready.store(true, Ordering::Release);

        idx.insert(1, &[1.0, 0.0, 0.0]);
        idx.insert(2, &[0.0, 1.0, 0.0]); // orthogonal, similarity ≈ 0

        let results = idx.search(&[1.0, 0.0, 0.0], 5, 0.5);
        // Only #1 should pass threshold
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].l2_id, 1);
    }

    #[test]
    fn empty_index_search() {
        let idx = make_index();
        idx.ready.store(true, Ordering::Release);
        let results = idx.search(&[1.0, 0.0], 5, 0.0);
        assert!(results.is_empty());
    }

    #[test]
    fn parallel_search_works() {
        let idx = make_index();
        idx.ready.store(true, Ordering::Release);

        for i in 0..100 {
            let v: Vec<f32> = (0..8)
                .map(|j| if j == (i % 8) { 1.0 } else { 0.0 })
                .collect();
            idx.insert(i as i64, &v);
        }

        let queries: Vec<Vec<f32>> = (0..4)
            .map(|i| (0..8).map(|j| if j == i { 1.0 } else { 0.0 }).collect())
            .collect();

        let results = idx.parallel_search(&queries, 5, 0.5);
        assert_eq!(results.len(), 4);
        for batch in &results {
            assert!(!batch.is_empty());
        }
    }

    #[test]
    fn dimension_mismatch_rejected() {
        let idx = make_index();
        idx.insert(1, &[1.0, 0.0, 0.0]); // sets dims to 3
        let result = idx.insert(2, &[1.0, 0.0]); // wrong dims
        assert!(result.is_none());
    }

    #[test]
    fn needs_rebuild_below_threshold() {
        let idx = make_index();
        idx.ready.store(true, Ordering::Release);
        // Insert 100 items, delete 10 (10% < 20% threshold)
        for i in 0..100 {
            idx.insert(i, &[1.0, 0.0, 0.0]);
        }
        for i in 0..10 {
            idx.mark_deleted(i);
        }
        assert!(!idx.needs_rebuild());
    }

    #[test]
    fn needs_rebuild_above_threshold() {
        let idx = make_index();
        idx.ready.store(true, Ordering::Release);
        // Insert 200 items, delete 60 (30% > 20% threshold, count > 50)
        for i in 0..200 {
            idx.insert(i, &[1.0, 0.0, 0.0]);
        }
        for i in 0..60 {
            idx.mark_deleted(i);
        }
        assert!(idx.needs_rebuild());
    }

    #[test]
    fn needs_rebuild_requires_min_count() {
        let idx = make_index();
        idx.ready.store(true, Ordering::Release);
        // Insert 10, delete 5 (50% ratio but count < 50 minimum)
        for i in 0..10 {
            idx.insert(i, &[1.0, 0.0, 0.0]);
        }
        for i in 0..5 {
            idx.mark_deleted(i);
        }
        assert!(
            !idx.needs_rebuild(),
            "should not rebuild with <50 deleted even at high ratio"
        );
    }

    #[test]
    fn clear_deleted_resets_state() {
        let idx = make_index();
        idx.ready.store(true, Ordering::Release);
        for i in 0..100 {
            idx.insert(i, &[1.0, 0.0, 0.0]);
        }
        for i in 0..60 {
            idx.mark_deleted(i);
        }
        assert_eq!(idx.deleted_count(), 60);
        idx.clear_deleted();
        assert_eq!(idx.deleted_count(), 0);
        assert!(!idx.needs_rebuild());
    }

    #[test]
    fn deleted_ratio_empty_index() {
        let idx = make_index();
        assert_eq!(idx.deleted_ratio(), 0.0);
    }
}
