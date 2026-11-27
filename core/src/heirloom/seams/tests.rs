//! Integration tests for Phase B seams. Exercises composition of 2+ seams
//! in the shape that W10 (Phase C) will follow.

use std::sync::Arc;

use rand::SeedableRng;
use rand::rngs::StdRng;

use crate::heirloom::BundleId;
use crate::heirloom::enums::{PayloadEncoding, SensitivityTier};
use crate::heirloom::seams::{
    BundlePolicy, ConstitutionTuple, EditionPolicy, EthicsContext, EthicsValidator, EthicsVerdict,
    InMemoryZeroizer, KekDomain, L2Manifest, NoopWelfareOracle, PayloadCodec, PermissiveEthics,
    PlaintextCodec, SecureEnclave, SoftEnclave, SoftThresholdSigner, TargetAudience,
    ThresholdSigner, WelfareOracle, WelfareScore, WrapContext, ZeroizeHook, ZeroizeReason,
    verify_aggregate,
};
use crate::heirloom::types::{Did, OwnerState, ProposedAction, WitnessChannel};

fn build_full_enclave(seed: u64) -> Arc<SoftEnclave> {
    let signer = Arc::new(SoftThresholdSigner::new_deterministic(seed));
    let zeroizer: Arc<dyn ZeroizeHook> = Arc::new(InMemoryZeroizer::new());
    Arc::new(SoftEnclave::new(
        signer,
        zeroizer,
        ConstitutionTuple::zeros(),
    ))
}

fn dev_policy() -> BundlePolicy {
    BundlePolicy {
        bundle_id: BundleId::new("bundle:general"),
        preferred_encoding: "plaintext".to_string(),
    }
}

fn dev_ethics_ctx() -> EthicsContext {
    EthicsContext {
        l0_text: "Layer 0 placeholder".to_string(),
        l1_text: "Core Soul placeholder".to_string(),
        l2_manifest: L2Manifest {
            merkle_root: [0u8; 32],
            version_tag: "l2-dev".to_string(),
        },
        edition_policy: EditionPolicy {
            primary: "dev".to_string(),
            secondaries: vec![],
        },
        owner_state: OwnerState::Alive,
        active_autonomy_grants: vec![],
        witness_channel: WitnessChannel::Mcp,
        current_sensitivity_tier: SensitivityTier::Ordinary,
        action_target_audience: TargetAudience::SelfOwner,
    }
}

fn action(event_type: &str) -> ProposedAction {
    ProposedAction {
        event_type: event_type.to_string(),
        author: Did::new("did:aegis:self"),
        target: None,
        summary: "test".to_string(),
        payload_digest: [0u8; 32],
    }
}

#[test]
fn i1_enclave_signer_sign_then_verify_using_exposed_pubkey() {
    let enclave = build_full_enclave(1);
    let signer = enclave.threshold_signer();
    let msg = b"heirloom:event:001";
    let sig = signer.sign(msg).unwrap();
    verify_aggregate(&signer.group_pubkey(), msg, &sig).expect("verify");
}

#[test]
fn i2_enclave_kek_wrap_then_unwrap_roundtrips_across_domains() {
    let enclave = build_full_enclave(1);
    let ctx = WrapContext {
        bundle_id: BundleId::new("bundle:general"),
        generation: 1,
        constitution_hash: ConstitutionTuple::zeros(),
    };
    for d in [
        KekDomain::UserData,
        KekDomain::Constitution,
        KekDomain::Loyalty,
        KekDomain::Session,
    ] {
        let kek = enclave.kek(d).unwrap();
        let dek = [0x42u8; 32];
        let wrapped = kek.wrap(&dek, &ctx).unwrap();
        let unwrapped = kek.unwrap(&wrapped, &ctx).unwrap();
        assert_eq!(unwrapped, dek, "domain {d:?} roundtrip");
        assert_eq!(wrapped.domain, d);
    }
}

fn envelope_with(ctuple: ConstitutionTuple, payload: Vec<u8>) -> crate::heirloom::EventEnvelope {
    use crate::heirloom::enums::EventTypeTag;
    use crate::heirloom::hlc::HlcTimestamp;
    use crate::heirloom::types::LineageId;
    use chrono::{DateTime, Utc};
    use uuid::Uuid;
    crate::heirloom::EventEnvelope {
        event_id: Uuid::parse_str("01925b3e-0000-7000-8000-0000000000aa").unwrap(),
        schema_version: 1,
        event_type: EventTypeTag::new("observation:user_stated"),
        prev_hash: [0; 32],
        hlc_ts: HlcTimestamp {
            phys: 1729123456789,
            log: 0,
        },
        observed_at: DateTime::parse_from_rfc3339("2026-04-17T10:00:00Z")
            .unwrap()
            .with_timezone(&Utc),
        l0_hash: ctuple.l0_hash,
        l1_hash: ctuple.l1_hash,
        l2_manifest_hash: ctuple.l2_manifest_hash,
        lineage_id: LineageId::new("lineage:test:int"),
        author_did: Did::new("did:aegis:test"),
        signatures: vec![],
        sensitivity_tier: SensitivityTier::Ordinary,
        bundle_ids: vec![BundleId::new("bundle:general")],
        payload_encoding: crate::heirloom::enums::PayloadEncoding::Plaintext,
        payload,
    }
}

#[test]
fn i3_plaintext_codec_roundtrip_under_enclave_and_envelope() {
    let enclave = build_full_enclave(1);
    let codec = PlaintextCodec::new();
    let mut rng = StdRng::seed_from_u64(0);
    let ctuple = enclave.current_constitution();
    let plaintext = b"hello from phase B integration test";
    let (enc, blob) = codec
        .encrypt(
            plaintext,
            &dev_policy(),
            &ctuple,
            enclave.as_ref(),
            &mut rng,
        )
        .unwrap();
    assert_eq!(enc, PayloadEncoding::Plaintext);
    let env = envelope_with(ctuple, blob.clone());
    let back = codec.decrypt(&enc, &blob, &env, enclave.as_ref()).unwrap();
    assert_eq!(back, plaintext);
}

#[test]
fn i4_plaintext_codec_detects_drift_when_envelope_advertises_different_constitution() {
    let enclave = build_full_enclave(1);
    let codec = PlaintextCodec::new();
    let mut rng = StdRng::seed_from_u64(0);
    let original_ctuple = enclave.current_constitution();
    let (enc, blob) = codec
        .encrypt(
            b"x",
            &dev_policy(),
            &original_ctuple,
            enclave.as_ref(),
            &mut rng,
        )
        .unwrap();

    // Build an envelope that advertises a DIFFERENT constitution triple.
    // Decrypt MUST refuse with ConstitutionDrift.
    let drifted_env = envelope_with(
        ConstitutionTuple {
            l0_hash: [0xEE; 32],
            l1_hash: [0xEE; 32],
            l2_manifest_hash: [0xEE; 32],
        },
        blob.clone(),
    );
    let err = codec
        .decrypt(&enc, &blob, &drifted_env, enclave.as_ref())
        .unwrap_err();
    assert!(matches!(
        err,
        crate::heirloom::seams::CodecError::ConstitutionDrift
    ));
}

#[test]
fn i5_permissive_ethics_allows_neutral_rejects_hard_reject() {
    let e = PermissiveEthics::new();
    let ctx = dev_ethics_ctx();
    assert_eq!(
        e.evaluate(&action("ai:message_sent"), &ctx),
        EthicsVerdict::Allow,
    );
    assert!(matches!(
        e.evaluate(&action("ai:harm_owner"), &ctx),
        EthicsVerdict::Reject { .. }
    ));
}

#[test]
fn i6_welfare_oracle_noop_returns_neutral_always() {
    let w = NoopWelfareOracle::new();
    let target = Did::new("did:aegis:telegram:4242");
    let s = w.marginal_welfare(&action("ai:proactive_reminder"), &target);
    assert_eq!(s, WelfareScore::neutral());
}

#[test]
fn i7_zeroizer_scoped_zeroize_does_not_affect_other_domains() {
    let z = InMemoryZeroizer::new();
    let h_ud = z.register_material(crate::heirloom::seams::ZeroizableMaterial {
        bytes: vec![0xABu8; 4].into_boxed_slice(),
        bundle: None,
        domain: Some(KekDomain::UserData),
        tag: "user-data-dek".to_string(),
    });
    let h_ct = z.register_material(crate::heirloom::seams::ZeroizableMaterial {
        bytes: vec![0xABu8; 4].into_boxed_slice(),
        bundle: None,
        domain: Some(KekDomain::Constitution),
        tag: "constitution-dek".to_string(),
    });
    z.zeroize_domain(KekDomain::UserData, ZeroizeReason::OwnerCommand)
        .unwrap();
    assert!(z.material_is_zero(h_ud));
    assert!(!z.material_is_zero(h_ct));
}

#[test]
fn i8_full_mini_flow_ethics_then_codec_then_signer_all_compose() {
    // Shape: ethics allows -> codec encrypts -> signer signs canonical bytes.
    // This is the path W10 (Phase C) will follow for every proactive AI action.
    let enclave = build_full_enclave(1);
    let codec = PlaintextCodec::new();
    let ethics = PermissiveEthics::new();
    let mut rng = StdRng::seed_from_u64(42);

    let proposed = action("ai:message_sent");
    let ctx = dev_ethics_ctx();
    let verdict = ethics.evaluate(&proposed, &ctx);
    assert_eq!(verdict, EthicsVerdict::Allow);

    let ctuple = enclave.current_constitution();
    let (enc, blob) = codec
        .encrypt(
            b"reply body",
            &dev_policy(),
            &ctuple,
            enclave.as_ref(),
            &mut rng,
        )
        .unwrap();
    assert_eq!(enc, PayloadEncoding::Plaintext);

    // Signer signs the (encoding || blob) byte string — stand-in for the
    // canonical envelope bytes Phase C will compute.
    let signer = enclave.threshold_signer();
    let mut to_sign = Vec::new();
    to_sign.extend_from_slice(format!("{enc:?}").as_bytes());
    to_sign.extend_from_slice(&blob);
    let sig = signer.sign(&to_sign).unwrap();
    verify_aggregate(&signer.group_pubkey(), &to_sign, &sig).expect("sig verifies");
}

#[test]
fn i9_enclave_request_zeroize_fires_zeroizer() {
    let signer = Arc::new(SoftThresholdSigner::new_deterministic(1));
    let zeroizer: Arc<InMemoryZeroizer> = Arc::new(InMemoryZeroizer::new());
    let h = zeroizer.register_material(crate::heirloom::seams::ZeroizableMaterial {
        bytes: vec![0xFFu8; 8].into_boxed_slice(),
        bundle: None,
        domain: None,
        tag: "test".to_string(),
    });
    let enclave = SoftEnclave::new(
        signer,
        Arc::clone(&zeroizer) as Arc<dyn ZeroizeHook>,
        ConstitutionTuple::zeros(),
    );
    enclave
        .request_zeroize(ZeroizeReason::OwnerCommand)
        .unwrap();
    assert!(zeroizer.material_is_zero(h));
}

#[test]
fn i10_trait_objects_hold_all_seams_by_arc_dyn() {
    // Proves Phase C can hold `Arc<dyn Seam>` for every seam type — i.e.
    // all traits are object-safe and Send+Sync+'static.
    let enclave = build_full_enclave(1);
    let _signer_dyn: Arc<dyn ThresholdSigner> = enclave.threshold_signer();
    let _codec_dyn: Arc<dyn PayloadCodec> = Arc::new(PlaintextCodec::new());
    let _enclave_dyn: Arc<dyn SecureEnclave> = enclave.clone();
    let _ethics_dyn: Arc<dyn EthicsValidator> = Arc::new(PermissiveEthics::new());
    let _welfare_dyn: Arc<dyn WelfareOracle> = Arc::new(NoopWelfareOracle::new());
    let _zeroizer_dyn: Arc<dyn ZeroizeHook> = Arc::new(InMemoryZeroizer::new());
}
