//! W8 — ZeroizeHook trait + InMemoryZeroizer dev impl.
//!
//! Spec §5 Q5.7. Registers memory regions that must be zero-filled on
//! tamper / duress / owner command. Dev impl uses the `zeroize` crate's
//! constant-time `Zeroize::zeroize()`. Prod impl (P4) adds hardware-triggered
//! SE chip wipe.
//!
//! `TamperTrigger` registration is accepted but never fires in dev — Phase B
//! has no hardware input layer.

use std::sync::Arc;

use super::super::BundleId;
use super::types::{KekDomain, ZeroizeReason};

// ── Value types ───────────────────────────────────────────────────────────

/// Opaque handle returned when material is registered. Callers keep it so
/// they can later unregister a material (e.g. on bundle close).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MaterialHandle(u64);

impl MaterialHandle {
    pub const fn as_u64(&self) -> u64 {
        self.0
    }
}

/// Sensitive memory region to be wiped on demand.
///
/// The `zeroize` crate's `Zeroize` trait is applied to the owned buffer.
/// Scope tags let `zeroize_bundle()` / `zeroize_domain()` wipe subsets
/// without disturbing unrelated material.
pub struct ZeroizableMaterial {
    /// Owned bytes the hook will overwrite on zeroize.
    pub bytes: Box<[u8]>,
    /// Scope — which bundle this material belongs to. `None` = global.
    pub bundle: Option<BundleId>,
    /// Scope — which KEK domain this material belongs to. `None` = cross-domain.
    pub domain: Option<KekDomain>,
    /// Free-form tag for audit — `"KEK:user_data"`, `"session-dek-2026Q2"`, etc.
    pub tag: String,
}

impl std::fmt::Debug for ZeroizableMaterial {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never leak byte contents into logs.
        f.debug_struct("ZeroizableMaterial")
            .field("bytes_len", &self.bytes.len())
            .field("bundle", &self.bundle)
            .field("domain", &self.domain)
            .field("tag", &self.tag)
            .finish()
    }
}

/// Physical trigger that may fire zeroize — thermal sensor, tamper switch, etc.
/// Phase B accepts registrations but Phase B impls never fire them.
pub trait TamperTrigger: Send + Sync {
    /// Short tag for audit logging.
    fn tag(&self) -> &str;
}

// ── Trait ─────────────────────────────────────────────────────────────────

/// Registers and zeroizes sensitive memory regions. Spec §5 Q5.7.
pub trait ZeroizeHook: Send + Sync {
    fn register_material(&self, material: ZeroizableMaterial) -> MaterialHandle;

    /// Unregister a previously-registered material. Idempotent: unregistering
    /// a handle that is not currently registered returns `false` with no error.
    fn unregister_material(&self, handle: MaterialHandle) -> bool;

    /// Zeroize every registered material. Used by `OwnerCommand` / `CriticalBreach`.
    fn zeroize_all(&self, reason: ZeroizeReason) -> Result<(), ZeroizeError>;

    /// Zeroize only material tagged with this bundle. `None`-bundle material
    /// is untouched.
    fn zeroize_bundle(
        &self,
        bundle_id: &BundleId,
        reason: ZeroizeReason,
    ) -> Result<(), ZeroizeError>;

    /// Zeroize only material in this KEK domain.
    fn zeroize_domain(&self, domain: KekDomain, reason: ZeroizeReason) -> Result<(), ZeroizeError>;

    fn register_trigger(&self, trigger: Arc<dyn TamperTrigger>);

    /// Number of still-registered materials. Test + audit use.
    fn registered_count(&self) -> usize;
}

/// Errors from a ZeroizeHook. Distinct variants kept minimal in Phase B.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ZeroizeError {
    #[error("zeroize hook is in a poisoned state; further operations are unsafe")]
    Poisoned,
}

// ── InMemoryZeroizer ──────────────────────────────────────────────────────

use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use zeroize::Zeroize;

/// Dev impl backed by a `HashMap<MaterialHandle, ZeroizableMaterial>` under
/// a `parking_lot::Mutex`. Calls `zeroize::Zeroize::zeroize()` on each
/// selected material's bytes. Retains the registration entry (bytes are
/// wiped in place so subsequent zeroize calls remain safe to issue).
#[derive(Default)]
pub struct InMemoryZeroizer {
    next_handle: AtomicU64,
    materials: Mutex<HashMap<MaterialHandle, ZeroizableMaterial>>,
    triggers: Mutex<Vec<Arc<dyn TamperTrigger>>>,
}

impl std::fmt::Debug for InMemoryZeroizer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InMemoryZeroizer")
            .field("registered_materials", &self.materials.lock().len())
            .field("registered_triggers", &self.triggers.lock().len())
            .finish()
    }
}

impl InMemoryZeroizer {
    pub fn new() -> Self {
        Self {
            next_handle: AtomicU64::new(1),
            materials: Mutex::new(HashMap::new()),
            triggers: Mutex::new(Vec::new()),
        }
    }

    // ── Test helpers ──────────────────────────────────────────────────────

    /// True if every byte of every registered material is zero.
    #[cfg(test)]
    pub fn all_material_is_zero(&self) -> bool {
        self.materials
            .lock()
            .values()
            .all(|m| m.bytes.iter().all(|b| *b == 0))
    }

    #[cfg(test)]
    pub fn material_is_zero(&self, handle: MaterialHandle) -> bool {
        self.materials
            .lock()
            .get(&handle)
            .map(|m| m.bytes.iter().all(|b| *b == 0))
            .unwrap_or(false)
    }

    #[cfg(test)]
    pub fn trigger_count(&self) -> usize {
        self.triggers.lock().len()
    }

    fn alloc_handle(&self) -> MaterialHandle {
        MaterialHandle(self.next_handle.fetch_add(1, Ordering::Relaxed))
    }
}

impl ZeroizeHook for InMemoryZeroizer {
    fn register_material(&self, material: ZeroizableMaterial) -> MaterialHandle {
        let handle = self.alloc_handle();
        self.materials.lock().insert(handle, material);
        handle
    }

    fn unregister_material(&self, handle: MaterialHandle) -> bool {
        let mut mats = self.materials.lock();
        if let Some(mut m) = mats.remove(&handle) {
            // Zeroize on unregister too — best-effort defense against
            // a caller that forgets to call zeroize_* first.
            m.bytes.zeroize();
            true
        } else {
            false
        }
    }

    fn zeroize_all(&self, _reason: ZeroizeReason) -> Result<(), ZeroizeError> {
        let mut mats = self.materials.lock();
        for m in mats.values_mut() {
            m.bytes.zeroize();
        }
        Ok(())
    }

    fn zeroize_bundle(
        &self,
        bundle_id: &BundleId,
        _reason: ZeroizeReason,
    ) -> Result<(), ZeroizeError> {
        let mut mats = self.materials.lock();
        for m in mats.values_mut() {
            if m.bundle.as_ref() == Some(bundle_id) {
                m.bytes.zeroize();
            }
        }
        Ok(())
    }

    fn zeroize_domain(
        &self,
        domain: KekDomain,
        _reason: ZeroizeReason,
    ) -> Result<(), ZeroizeError> {
        let mut mats = self.materials.lock();
        for m in mats.values_mut() {
            if m.domain == Some(domain) {
                m.bytes.zeroize();
            }
        }
        Ok(())
    }

    fn register_trigger(&self, trigger: Arc<dyn TamperTrigger>) {
        self.triggers.lock().push(trigger);
    }

    fn registered_count(&self) -> usize {
        self.materials.lock().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mat(
        tag: &str,
        len: usize,
        bundle: Option<BundleId>,
        domain: Option<KekDomain>,
    ) -> ZeroizableMaterial {
        ZeroizableMaterial {
            bytes: vec![0xABu8; len].into_boxed_slice(),
            bundle,
            domain,
            tag: tag.to_string(),
        }
    }

    #[test]
    fn t8_register_returns_distinct_handles() {
        let z = InMemoryZeroizer::new();
        let h1 = z.register_material(mat("a", 8, None, None));
        let h2 = z.register_material(mat("b", 8, None, None));
        assert_ne!(h1, h2);
        assert_eq!(z.registered_count(), 2);
    }

    #[test]
    fn t8_zeroize_all_overwrites_bytes_but_retains_registration() {
        let z = InMemoryZeroizer::new();
        let _h = z.register_material(mat("x", 16, None, None));
        z.zeroize_all(ZeroizeReason::OwnerCommand).unwrap();
        // Material is still registered; its bytes are now all zero.
        assert_eq!(z.registered_count(), 1);
        assert!(
            z.all_material_is_zero(),
            "bytes must be zeroed after zeroize_all"
        );
    }

    #[test]
    fn t8_unregister_material_removes_it() {
        let z = InMemoryZeroizer::new();
        let h = z.register_material(mat("x", 8, None, None));
        assert!(z.unregister_material(h));
        assert_eq!(z.registered_count(), 0);
        // Unregistering again returns false — idempotent.
        assert!(!z.unregister_material(h));
    }

    #[test]
    fn t8_zeroize_bundle_touches_only_matching_bundle() {
        let z = InMemoryZeroizer::new();
        let b_alpha = BundleId::new("bundle:alpha");
        let b_beta = BundleId::new("bundle:beta");
        let h_alpha = z.register_material(mat("a-dek", 8, Some(b_alpha.clone()), None));
        let h_beta = z.register_material(mat("b-dek", 8, Some(b_beta), None));

        z.zeroize_bundle(&b_alpha, ZeroizeReason::OwnerCommand)
            .unwrap();

        assert!(z.material_is_zero(h_alpha), "alpha material must be zeroed");
        assert!(
            !z.material_is_zero(h_beta),
            "beta material must be untouched when zeroizing alpha",
        );
    }

    #[test]
    fn t8_zeroize_domain_touches_only_matching_domain() {
        let z = InMemoryZeroizer::new();
        let h_ud = z.register_material(mat("u", 4, None, Some(KekDomain::UserData)));
        let h_c = z.register_material(mat("c", 4, None, Some(KekDomain::Constitution)));

        z.zeroize_domain(KekDomain::Constitution, ZeroizeReason::TamperDetected)
            .unwrap();

        assert!(z.material_is_zero(h_c));
        assert!(!z.material_is_zero(h_ud));
    }

    #[test]
    fn t8_zeroize_bundle_ignores_none_bundle_material() {
        let z = InMemoryZeroizer::new();
        let b = BundleId::new("bundle:only");
        let h_scoped = z.register_material(mat("s", 4, Some(b.clone()), None));
        let h_global = z.register_material(mat("g", 4, None, None));
        z.zeroize_bundle(&b, ZeroizeReason::OwnerCommand).unwrap();
        assert!(z.material_is_zero(h_scoped));
        assert!(
            !z.material_is_zero(h_global),
            "global (None-bundle) material must survive a bundle-scoped zeroize",
        );
    }

    #[test]
    fn t8_register_trigger_is_stored_but_not_fired() {
        struct DummyTrigger;
        impl TamperTrigger for DummyTrigger {
            fn tag(&self) -> &'static str {
                "dummy"
            }
        }

        let z = InMemoryZeroizer::new();
        let h = z.register_material(mat("x", 4, None, None));
        z.register_trigger(Arc::new(DummyTrigger));

        // Trigger registration must not zeroize anything in the dev impl.
        assert!(
            !z.material_is_zero(h),
            "dev impl must not zeroize on register_trigger"
        );
        assert_eq!(z.trigger_count(), 1);
    }
}
