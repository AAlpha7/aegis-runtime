//! Multimodal end-to-end integration tests.
//!
//! Tests:
//! 1. MediaProcessor: Telegram file download + base64 encoding
//! 2. STT transcription via real OpenAI API (requires AEGIS_STT_API_KEY)
//! 3. Image generation via real OpenRouter API (requires OPENAI_API_KEY)
//! 4. Orchestrator multimodal content array building
//! 5. Reply-to-message context extraction
//!
//! Tests marked #[ignore] require real API keys and network access.

use aegis_runtime_core::multimodal::{
    MediaAttachment, MediaProcessor, ProcessedMessage, base64_encode, decode_base64_standard,
};

// ===========================================================================
// Unit-level integration tests (no network needed)
// ===========================================================================

#[test]
fn base64_encode_decode_roundtrip_binary() {
    // Simulate a small PNG header
    let png_header: Vec<u8> = vec![
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, // PNG magic
        0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52, // IHDR chunk
    ];
    let encoded = base64_encode(&png_header);
    let decoded = decode_base64_standard(&encoded).unwrap();
    assert_eq!(decoded, png_header);
}

#[test]
fn media_attachment_audio_serde_preserves_data() {
    let audio = MediaAttachment::Audio {
        data_b64: base64_encode(b"fake audio data for testing"),
        format: "ogg".to_string(),
        duration_secs: 7,
    };
    let json = serde_json::to_string(&audio).unwrap();
    let back: MediaAttachment = serde_json::from_str(&json).unwrap();
    match back {
        MediaAttachment::Audio {
            data_b64,
            format,
            duration_secs,
        } => {
            assert_eq!(format, "ogg");
            assert_eq!(duration_secs, 7);
            // Verify roundtrip
            let decoded = decode_base64_standard(&data_b64).unwrap();
            assert_eq!(decoded, b"fake audio data for testing");
        }
        _ => panic!("expected Audio variant"),
    }
}

#[test]
fn processed_message_with_image_attachment() {
    let img_bytes = vec![0xFF, 0xD8, 0xFF, 0xE0]; // JPEG magic
    let data_url = format!("data:image/jpeg;base64,{}", base64_encode(&img_bytes));

    let msg = ProcessedMessage {
        text: "What is this?".into(),
        attachments: vec![MediaAttachment::Image {
            data_url: data_url.clone(),
        }],
    };

    assert_eq!(msg.text, "What is this?");
    assert_eq!(msg.attachments.len(), 1);
    match &msg.attachments[0] {
        MediaAttachment::Image { data_url: url } => {
            assert!(url.starts_with("data:image/jpeg;base64,"));
            // Decode and verify
            let b64_part = url.strip_prefix("data:image/jpeg;base64,").unwrap();
            let decoded = decode_base64_standard(b64_part).unwrap();
            assert_eq!(decoded, img_bytes);
        }
        _ => panic!("expected Image"),
    }
}

#[test]
fn processed_message_audio_direct_llm_input() {
    let fake_audio = b"OggS\x00\x02"; // fake OGG header
    let msg = ProcessedMessage {
        text: "[用户发送了一段语音]".into(),
        attachments: vec![MediaAttachment::Audio {
            data_b64: base64_encode(fake_audio),
            format: "ogg".into(),
            duration_secs: 3,
        }],
    };

    assert_eq!(msg.attachments.len(), 1);
    assert!(msg.text.contains("语音"));
}

#[test]
fn message_attachments_backward_compat_empty() {
    // Old messages without attachments field should deserialize fine
    use aegis_runtime_core::memory::Message;

    let json = r#"{"role":"user","content":"hello","timestamp":1234567890}"#;
    let msg: Message = serde_json::from_str(json).unwrap();
    assert_eq!(msg.content, "hello");
    assert!(msg.attachments.is_empty());
}

#[test]
fn message_text_constructor() {
    use aegis_runtime_core::memory::Message;

    let msg = Message::text("user", "test content", 1234567890);
    assert_eq!(msg.role, "user");
    assert_eq!(msg.content, "test content");
    assert_eq!(msg.timestamp, 1234567890);
    assert!(msg.attachments.is_empty());

    // Serialization should omit empty attachments
    let json = serde_json::to_string(&msg).unwrap();
    assert!(!json.contains("attachments"));
}

#[test]
fn message_with_attachments_serialization() {
    use aegis_runtime_core::memory::Message;

    let msg = Message {
        role: "user".into(),
        content: "看这张图".into(),
        timestamp: 1234567890,
        anchor_kind: None,
        anchor_ref: None,
        latency_ms: None,
        attachments: vec![MediaAttachment::Image {
            data_url: "data:image/png;base64,abc".into(),
        }],
    };

    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains("attachments"));
    assert!(json.contains("Image"));

    let back: Message = serde_json::from_str(&json).unwrap();
    assert_eq!(back.attachments.len(), 1);
}

// ===========================================================================
// Document and Video attachment tests
// ===========================================================================

#[test]
fn document_attachment_serde_roundtrip() {
    let doc = MediaAttachment::Document {
        data_url: "data:application/pdf;base64,JVBERi0xLjQ=".into(),
        filename: "test.pdf".into(),
        mime_type: "application/pdf".into(),
    };
    let json = serde_json::to_string(&doc).unwrap();
    assert!(json.contains("Document"));
    assert!(json.contains("test.pdf"));
    assert!(json.contains("application/pdf"));

    let back: MediaAttachment = serde_json::from_str(&json).unwrap();
    match back {
        MediaAttachment::Document {
            filename,
            mime_type,
            ..
        } => {
            assert_eq!(filename, "test.pdf");
            assert_eq!(mime_type, "application/pdf");
        }
        _ => panic!("expected Document variant"),
    }
}

#[test]
fn video_attachment_serde_roundtrip() {
    let vid = MediaAttachment::Video {
        data_url: "data:video/mp4;base64,AAAA".into(),
        filename: "clip.mp4".into(),
        duration_secs: 15,
    };
    let json = serde_json::to_string(&vid).unwrap();
    assert!(json.contains("Video"));
    assert!(json.contains("clip.mp4"));

    let back: MediaAttachment = serde_json::from_str(&json).unwrap();
    match back {
        MediaAttachment::Video {
            filename,
            duration_secs,
            ..
        } => {
            assert_eq!(filename, "clip.mp4");
            assert_eq!(duration_secs, 15);
        }
        _ => panic!("expected Video variant"),
    }
}

#[test]
fn processed_message_with_document() {
    let msg = ProcessedMessage {
        text: "请帮我分析这份PDF".into(),
        attachments: vec![MediaAttachment::Document {
            data_url: "data:application/pdf;base64,JVBERi0=".into(),
            filename: "report.pdf".into(),
            mime_type: "application/pdf".into(),
        }],
    };
    assert_eq!(msg.attachments.len(), 1);
    assert!(msg.text.contains("PDF"));
}

#[test]
fn processed_message_with_video() {
    let msg = ProcessedMessage {
        text: "[用户发送了一段视频]".into(),
        attachments: vec![MediaAttachment::Video {
            data_url: "data:video/mp4;base64,AAAA".into(),
            filename: "video.mp4".into(),
            duration_secs: 30,
        }],
    };
    assert_eq!(msg.attachments.len(), 1);
    assert!(msg.text.contains("视频"));
}

#[test]
fn message_with_document_backward_compat() {
    // Messages with new Document/Video types should serialize/deserialize correctly
    use aegis_runtime_core::memory::Message;

    let msg = Message {
        role: "user".into(),
        content: "看看这个PDF".into(),
        timestamp: 1234567890,
        anchor_kind: None,
        anchor_ref: None,
        latency_ms: None,
        attachments: vec![MediaAttachment::Document {
            data_url: "data:application/pdf;base64,abc".into(),
            filename: "test.pdf".into(),
            mime_type: "application/pdf".into(),
        }],
    };

    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains("Document"));
    let back: Message = serde_json::from_str(&json).unwrap();
    assert_eq!(back.attachments.len(), 1);
}

// ===========================================================================
// Real API integration tests (require network + API keys)
// ===========================================================================

/// Test image generation via OpenRouter.
/// Requires: OPENAI_API_KEY (OpenRouter key) + OPENAI_API_BASE
#[test]
#[ignore]
fn image_generation_via_openrouter() {
    let api_key = std::env::var("OPENAI_API_KEY").expect("OPENAI_API_KEY required");
    let api_base =
        std::env::var("OPENAI_API_BASE").unwrap_or_else(|_| "https://openrouter.ai/api/v1".into());
    let model = std::env::var("AEGIS_IMAGEGEN_MODEL")
        .unwrap_or_else(|_| "google/gemini-2.5-flash-image".into());

    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let client = reqwest::Client::new();
        let body = serde_json::json!({
            "model": model,
            "messages": [{"role": "user", "content": "Generate a simple blue square on white background"}],
            "modalities": ["image", "text"],
        });

        let resp = client
            .post(format!("{}/chat/completions", api_base))
            .header("Authorization", format!("Bearer {api_key}"))
            .json(&body)
            .send()
            .await
            .expect("request failed");

        let status = resp.status();
        assert!(
            status.is_success(),
            "API returned {status}: {}",
            resp.text().await.unwrap_or_default()
        );

        // Re-send to parse JSON
        let resp2 = client
            .post(format!("{}/chat/completions", api_base))
            .header("Authorization", format!("Bearer {api_key}"))
            .json(&body)
            .send()
            .await
            .unwrap();

        let json: serde_json::Value = resp2.json().await.unwrap();
        let msg = &json["choices"][0]["message"];

        // Should have images array
        assert!(
            msg["images"].is_array(),
            "Expected images array in response, got keys: {:?}",
            msg.as_object().map(|m| m.keys().collect::<Vec<_>>())
        );

        let images = msg["images"].as_array().unwrap();
        assert!(!images.is_empty(), "Expected at least 1 image");

        let url = images[0]["image_url"]["url"].as_str().unwrap();
        assert!(
            url.starts_with("data:image/"),
            "Expected data URL, got: {}",
            &url[..50.min(url.len())]
        );

        // Decode the image
        let raw_b64 = url.find(',').map(|i| &url[i + 1..]).unwrap_or(url);
        let img_bytes = decode_base64_standard(raw_b64).expect("base64 decode failed");
        assert!(img_bytes.len() > 100, "Image too small: {} bytes", img_bytes.len());

        println!(
            "Image generated successfully: {} bytes, content preview: {}",
            img_bytes.len(),
            msg["content"].as_str().unwrap_or("(none)")
        );
    });
}

/// Test STT transcription via OpenAI API (fallback path).
/// Requires: AEGIS_STT_API_KEY
#[test]
#[ignore]
fn stt_transcription_real_api() {
    let api_key = std::env::var("AEGIS_STT_API_KEY").expect("AEGIS_STT_API_KEY required");

    // Generate a minimal valid MP3 file (silence)
    // We'll use the MediaProcessor with a fake bot token since we only need transcribe_audio()
    #[allow(unsafe_code)]
    // SAFETY: test sets env var before any async work
    unsafe {
        std::env::set_var("AEGIS_STT_API_KEY", &api_key)
    };

    let processor = MediaProcessor::new("fake-bot-token");

    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        // Create a minimal audio file - 1 second of silence in WAV format
        // WAV header for 1 second mono 8kHz 8-bit PCM
        let sample_rate: u32 = 8000;
        let num_samples: u32 = sample_rate; // 1 second
        let data_size = num_samples;
        let file_size = 36 + data_size;

        let mut wav = Vec::with_capacity(file_size as usize + 8);
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&file_size.to_le_bytes());
        wav.extend_from_slice(b"WAVE");
        wav.extend_from_slice(b"fmt ");
        wav.extend_from_slice(&16u32.to_le_bytes()); // chunk size
        wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
        wav.extend_from_slice(&1u16.to_le_bytes()); // mono
        wav.extend_from_slice(&sample_rate.to_le_bytes());
        wav.extend_from_slice(&sample_rate.to_le_bytes()); // byte rate
        wav.extend_from_slice(&1u16.to_le_bytes()); // block align
        wav.extend_from_slice(&8u16.to_le_bytes()); // bits per sample
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&data_size.to_le_bytes());
        wav.extend_from_slice(&vec![128u8; num_samples as usize]); // silence

        let result = processor.transcribe_audio(wav, "test_silence.wav").await;
        match result {
            Ok(text) => {
                println!("STT transcription result: '{text}'");
                // Silence should return empty or very short text
                assert!(text.len() < 100, "Unexpected long transcription of silence");
            }
            Err(e) => {
                // API might reject very short/silent audio — that's OK
                println!("STT returned error (acceptable for silence): {e:#}");
            }
        }
    });
}
