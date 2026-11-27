//! Genesis sentinel for the hash chain.
//!
//! Every lineage's genesis event has `prev_hash = GENESIS_PREV_HASH`.
//! Spec §2 Q2.2 (Round-9 polish P2.3): this is a domain-scoped constant
//! rather than `[0u8; 32]` so:
//!   - It cannot accidentally collide with a real event_hash or be
//!     produced by tools that erase hash fields to zero.
//!   - It is human-auditable: the byte value = blake3(b"aegis:heirloom:genesis:v1").
//!
//! `build.rs` (Task 8) asserts this constant is pinned to the actual BLAKE3
//! value at compile time, so pin rot cannot happen silently.

/// The string input from which GENESIS_PREV_HASH is derived.
pub const GENESIS_DOMAIN: &[u8] = b"aegis:heirloom:genesis:v1";

/// Compute the genesis sentinel at runtime. Used by `build.rs` for the
/// compile-time pin assert and by tests.
pub fn compute_genesis_hash() -> [u8; 32] {
    *blake3::hash(GENESIS_DOMAIN).as_bytes()
}

/// Pinned genesis sentinel. `build.rs` asserts this equals
/// `compute_genesis_hash()` at compile time by regex-matching the hex bytes
/// in this file (formatting-tolerant — survives rustfmt's 8↔16 per-line
/// reformatting of `[u8; 32]` array literals).
///
/// Hex representation (for manual verification):
///   `0a7e7b0f1cd4c0741b2e6d4b23c89e7b3af25d4eeb64abb089d131bf24ddca24`
pub const GENESIS_PREV_HASH: [u8; 32] = [
    0x0a, 0x7e, 0x7b, 0x0f, 0x1c, 0xd4, 0xc0, 0x74, 0x1b, 0x2e, 0x6d, 0x4b, 0x23, 0xc8, 0x9e, 0x7b,
    0x3a, 0xf2, 0x5d, 0x4e, 0xeb, 0x64, 0xab, 0xb0, 0x89, 0xd1, 0x31, 0xbf, 0x24, 0xdd, 0xca, 0x24,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t7_genesis_hash_matches_domain_blake3() {
        let computed = compute_genesis_hash();
        assert_eq!(
            GENESIS_PREV_HASH, computed,
            "GENESIS_PREV_HASH pin is out of sync with blake3(GENESIS_DOMAIN). \
             Update the constant to the computed value: {:02x?}",
            computed
        );
    }

    #[test]
    fn t7_genesis_hash_is_32_bytes_nonzero() {
        assert_eq!(GENESIS_PREV_HASH.len(), 32);
        assert_ne!(GENESIS_PREV_HASH, [0u8; 32]);
    }

    #[test]
    fn t7_genesis_domain_exact_bytes() {
        assert_eq!(GENESIS_DOMAIN, b"aegis:heirloom:genesis:v1");
    }

    #[test]
    fn t7_genesis_hash_not_equal_to_common_sentinels() {
        // Belt-and-suspenders: must not collide with the usual "empty" values
        // that tools might produce.
        assert_ne!(GENESIS_PREV_HASH, [0u8; 32]);
        assert_ne!(GENESIS_PREV_HASH, [0xFFu8; 32]);
        assert_ne!(GENESIS_PREV_HASH, *blake3::hash(b"").as_bytes());
        assert_ne!(GENESIS_PREV_HASH, *blake3::hash(b"\0").as_bytes());
    }
}
