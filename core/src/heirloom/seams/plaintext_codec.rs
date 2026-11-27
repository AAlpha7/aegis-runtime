//! W4 dev impl — `PlaintextCodec`.
//!
//! Identity encrypt/decrypt. The `ConstitutionTuple` is written into the
//! encoded blob's framing so Phase C tamper tests can still assert seal-chain
//! drift detection even under the dev codec. **Never use in production.**

use super::super::enums::PayloadEncoding;
use super::payload_codec::{CodecError, PayloadCodec};
use super::secure_enclave::SecureEnclave;
use super::types::{BundlePolicy, ConstitutionTuple};
use rand::RngCore;

const PLAINTEXT_FRAME_TAG: &[u8] = b"aegis:plaintext-v1:";

/// Dev codec. Emits `(PayloadEncoding::Plaintext, tag || canonical(ctuple) || plaintext)`
/// so Phase C can still test seal-chain drift detection with this codec.
#[derive(Debug, Default, Clone, Copy)]
pub struct PlaintextCodec;

impl PlaintextCodec {
    pub const fn new() -> Self {
        Self
    }

    fn frame(ctuple: &ConstitutionTuple, payload: &[u8]) -> Vec<u8> {
        let header = serde_jcs::to_string(ctuple).expect("ConstitutionTuple must serialize");
        let mut out =
            Vec::with_capacity(PLAINTEXT_FRAME_TAG.len() + header.len() + 1 + payload.len());
        out.extend_from_slice(PLAINTEXT_FRAME_TAG);
        out.extend_from_slice(header.as_bytes());
        out.push(b'\n');
        out.extend_from_slice(payload);
        out
    }

    fn parse(blob: &[u8]) -> Result<(ConstitutionTuple, &[u8]), CodecError> {
        if !blob.starts_with(PLAINTEXT_FRAME_TAG) {
            return Err(CodecError::InvalidFraming(
                "missing plaintext-v1 framing tag".to_string(),
            ));
        }
        let rest = &blob[PLAINTEXT_FRAME_TAG.len()..];
        let nl_idx = rest
            .iter()
            .position(|b| *b == b'\n')
            .ok_or_else(|| CodecError::InvalidFraming("missing header terminator".to_string()))?;
        let header = std::str::from_utf8(&rest[..nl_idx])
            .map_err(|e| CodecError::InvalidFraming(format!("header utf8: {e}")))?;
        let ctuple: ConstitutionTuple = serde_json::from_str(header)
            .map_err(|e| CodecError::InvalidFraming(format!("header json: {e}")))?;
        Ok((ctuple, &rest[nl_idx + 1..]))
    }
}

impl PayloadCodec for PlaintextCodec {
    fn encrypt(
        &self,
        plaintext: &[u8],
        _policy: &BundlePolicy,
        constitution: &ConstitutionTuple,
        _enclave: &dyn SecureEnclave,
        _rng: &mut dyn RngCore,
    ) -> Result<(PayloadEncoding, Vec<u8>), CodecError> {
        Ok((
            PayloadEncoding::Plaintext,
            Self::frame(constitution, plaintext),
        ))
    }

    fn decrypt(
        &self,
        encoding: &PayloadEncoding,
        ciphertext: &[u8],
        envelope: &super::super::EventEnvelope,
        _enclave: &dyn SecureEnclave,
    ) -> Result<Vec<u8>, CodecError> {
        if *encoding != PayloadEncoding::Plaintext {
            return Err(CodecError::UnsupportedEncoding(format!("{encoding:?}")));
        }
        let (embedded, body) = Self::parse(ciphertext)?;
        let expected = ConstitutionTuple {
            // Phase A envelope fields (see core/src/heirloom/envelope.rs:45-49):
            // l0_hash, l1_hash, l2_manifest_hash.
            l0_hash: envelope.l0_hash,
            l1_hash: envelope.l1_hash,
            l2_manifest_hash: envelope.l2_manifest_hash,
        };
        if embedded != expected {
            return Err(CodecError::ConstitutionDrift);
        }
        Ok(body.to_vec())
    }

    fn encoding_tag(&self) -> PayloadEncoding {
        PayloadEncoding::Plaintext
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::heirloom::enums::{EventTypeTag, SensitivityTier};
    use crate::heirloom::hlc::HlcTimestamp;
    use crate::heirloom::seams::soft_enclave::SoftEnclave;
    use crate::heirloom::types::{Did, LineageId};
    use crate::heirloom::{BundleId, EventEnvelope};
    use chrono::{DateTime, Utc};
    use rand::SeedableRng;
    use rand::rngs::StdRng;
    use uuid::Uuid;

    fn policy() -> BundlePolicy {
        BundlePolicy {
            bundle_id: BundleId::new("bundle:general"),
            preferred_encoding: "plaintext".to_string(),
        }
    }

    /// Helper: build a test EventEnvelope with the given constitution triple
    /// and payload bytes. Other fields are deterministic stubs sufficient for
    /// codec tests (we never hash-chain or sign in Task 10).
    fn envelope_with(ctuple: ConstitutionTuple, payload_bytes: Vec<u8>) -> EventEnvelope {
        EventEnvelope {
            event_id: Uuid::parse_str("01925b3e-0000-7000-8000-000000000001").unwrap(),
            schema_version: 1,
            event_type: EventTypeTag::new("observation:user_stated"),
            prev_hash: [0xAA; 32],
            hlc_ts: HlcTimestamp {
                phys: 1729123456789,
                log: 0,
            },
            observed_at: DateTime::parse_from_rfc3339("2026-04-17T09:30:00Z")
                .unwrap()
                .with_timezone(&Utc),
            l0_hash: ctuple.l0_hash,
            l1_hash: ctuple.l1_hash,
            l2_manifest_hash: ctuple.l2_manifest_hash,
            lineage_id: LineageId::new("lineage:test:001"),
            author_did: Did::new("did:aegis:test"),
            signatures: vec![],
            sensitivity_tier: SensitivityTier::Ordinary,
            bundle_ids: vec![BundleId::new("bundle:general")],
            payload_encoding: PayloadEncoding::Plaintext,
            payload: payload_bytes,
        }
    }

    #[test]
    fn t4_plaintext_codec_encrypt_returns_plaintext_encoding_tag() {
        let codec = PlaintextCodec::new();
        let enclave = SoftEnclave::new_dev(1);
        let mut rng = StdRng::seed_from_u64(0);
        let (enc, _blob) = codec
            .encrypt(
                b"hello",
                &policy(),
                &ConstitutionTuple::zeros(),
                &enclave,
                &mut rng,
            )
            .unwrap();
        assert_eq!(enc, PayloadEncoding::Plaintext);
    }

    #[test]
    fn t4_plaintext_codec_roundtrip_same_constitution_returns_original() {
        let codec = PlaintextCodec::new();
        let enclave = SoftEnclave::new_dev(1);
        let mut rng = StdRng::seed_from_u64(0);
        let plaintext = b"user said: the weather is nice today";
        let ctuple = ConstitutionTuple::zeros();
        let (enc, blob) = codec
            .encrypt(plaintext, &policy(), &ctuple, &enclave, &mut rng)
            .unwrap();
        let env = envelope_with(ctuple, blob.clone());
        let back = codec.decrypt(&enc, &blob, &env, &enclave).unwrap();
        assert_eq!(back, plaintext);
    }

    #[test]
    fn t4_plaintext_codec_decrypt_rejects_constitution_drift() {
        let codec = PlaintextCodec::new();
        let enclave = SoftEnclave::new_dev(1);
        let mut rng = StdRng::seed_from_u64(0);
        let wrap_ctuple = ConstitutionTuple::zeros();
        let (enc, blob) = codec
            .encrypt(b"x", &policy(), &wrap_ctuple, &enclave, &mut rng)
            .unwrap();
        // Envelope advertises a DIFFERENT constitution than the one used to
        // frame the ciphertext. Decrypt MUST refuse.
        let drifted_env = envelope_with(
            ConstitutionTuple {
                l0_hash: [0xEE; 32],
                l1_hash: [0xEE; 32],
                l2_manifest_hash: [0xEE; 32],
            },
            blob.clone(),
        );
        let err = codec
            .decrypt(&enc, &blob, &drifted_env, &enclave)
            .unwrap_err();
        assert!(matches!(err, CodecError::ConstitutionDrift));
    }

    #[test]
    fn t4_plaintext_codec_decrypt_rejects_non_plaintext_encoding() {
        let codec = PlaintextCodec::new();
        let enclave = SoftEnclave::new_dev(1);
        let env = envelope_with(ConstitutionTuple::zeros(), b"blob".to_vec());
        // AeadV1 is a struct variant (see enums.rs:51) — fill fields with stubs.
        let aead_tag = PayloadEncoding::AeadV1 {
            algo: "unused".to_string(),
            nonce: "00".to_string(),
            key_ref: "unused".to_string(),
        };
        let err = codec
            .decrypt(&aead_tag, b"blob", &env, &enclave)
            .unwrap_err();
        assert!(matches!(err, CodecError::UnsupportedEncoding(_)));
    }

    #[test]
    fn t4_plaintext_codec_decrypt_rejects_missing_framing_tag() {
        let codec = PlaintextCodec::new();
        let enclave = SoftEnclave::new_dev(1);
        let env = envelope_with(ConstitutionTuple::zeros(), b"no-tag-here".to_vec());
        let err = codec
            .decrypt(&PayloadEncoding::Plaintext, b"no-tag-here", &env, &enclave)
            .unwrap_err();
        assert!(matches!(err, CodecError::InvalidFraming(_)));
    }

    #[test]
    fn t4_plaintext_codec_encrypt_empty_plaintext_roundtrips() {
        let codec = PlaintextCodec::new();
        let enclave = SoftEnclave::new_dev(1);
        let mut rng = StdRng::seed_from_u64(0);
        let ctuple = ConstitutionTuple::zeros();
        let (enc, blob) = codec
            .encrypt(b"", &policy(), &ctuple, &enclave, &mut rng)
            .unwrap();
        let env = envelope_with(ctuple, blob.clone());
        let back = codec.decrypt(&enc, &blob, &env, &enclave).unwrap();
        assert!(back.is_empty());
    }

    #[test]
    fn t4_plaintext_codec_encoding_tag_is_plaintext() {
        assert_eq!(
            PlaintextCodec::new().encoding_tag(),
            PayloadEncoding::Plaintext
        );
    }
}
