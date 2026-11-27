//! WIT Component Model bindings for Aegis Runtime v2 skills.
//!
//! Generated from `wit/aegis.wit` via `wasmtime::component::bindgen!`.
//! Provides type-safe host↔guest interfaces — no pointer arithmetic,
//! no JSON field name mismatches, compile-time guarantees on both sides.

// ── Standard skill world (aegis-skill-v2) ───────────────────────────────

/// Bindings for `aegis-skill-v2` world: standard skills that take/return JSON strings.
pub mod skill_v2 {
    wasmtime::component::bindgen!({
        path: "../wit",
        world: "aegis-skill-v2",
    });
}

// ── Probe skill world (aegis-probe-v2) ──────────────────────────────────

/// Bindings for `aegis-probe-v2` world: probe/heartbeat skills that return typed ProbeSignal.
pub mod probe_v2 {
    wasmtime::component::bindgen!({
        path: "../wit",
        world: "aegis-probe-v2",
    });
}

/// Convert WIT-generated ProbeSignal → models::ProbeSignal for unified downstream use.
impl From<probe_v2::aegis::skill::types::ProbeSignal> for crate::models::ProbeSignal {
    fn from(wit: probe_v2::aegis::skill::types::ProbeSignal) -> Self {
        Self {
            require_llm_attention: wit.require_llm_attention,
            event_payload: wit.event_payload,
            severity: Default::default(),
            suggested_actions: Vec::new(),
            probe_data: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::probe_v2::aegis::skill::types::ProbeSignal;

    #[test]
    fn probe_signal_conversion_alert() {
        let wit_signal = ProbeSignal {
            require_llm_attention: true,
            event_payload: Some("BTC price alert: $95000 > $90000".to_string()),
        };
        let model: crate::models::ProbeSignal = wit_signal.into();
        assert!(model.require_llm_attention);
        assert_eq!(
            model.event_payload.as_deref(),
            Some("BTC price alert: $95000 > $90000")
        );
    }

    #[test]
    fn probe_signal_conversion_silent() {
        let wit_signal = ProbeSignal {
            require_llm_attention: false,
            event_payload: None,
        };
        let model: crate::models::ProbeSignal = wit_signal.into();
        assert!(!model.require_llm_attention);
        assert!(model.event_payload.is_none());
    }
}
