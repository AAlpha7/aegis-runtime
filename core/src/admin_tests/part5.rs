use super::*;
// Route coverage: bare `/` redirect + CEO voice Realtime mint/status.
// These paths are registered on oauth_callback_router; oneshot coverage
// keeps the route-coverage ratchet green (core/tests/route_coverage_guard.rs).

/// Test env guard for the hermetic voice-session stub (no network).
/// Callers must hold `WEBAPP_DEBUG_ENV_LOCK` for the guard's full lifetime.
struct VoiceTestStubGuard {
    saved: Option<String>,
}

impl VoiceTestStubGuard {
    #[allow(unsafe_code)]
    fn set() -> Self {
        let saved = std::env::var("AEGIS_VOICE_TEST_STUB").ok();
        unsafe { std::env::set_var("AEGIS_VOICE_TEST_STUB", "1") };
        Self { saved }
    }
}

impl Drop for VoiceTestStubGuard {
    #[allow(unsafe_code)]
    fn drop(&mut self) {
        match &self.saved {
            Some(v) => unsafe { std::env::set_var("AEGIS_VOICE_TEST_STUB", v) },
            None => unsafe { std::env::remove_var("AEGIS_VOICE_TEST_STUB") },
        }
    }
}

/// Bookmarks on the bare domain must never 404: redirect to /ui/.
#[tokio::test]
async fn root_path_redirects_to_ui() {
    let state = webapp_test_state();
    let app = oauth_callback_router(state);
    let resp = app
        .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::TEMPORARY_REDIRECT);
    let location = resp
        .headers()
        .get(axum::http::header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert_eq!(location, "/ui/");
}

#[tokio::test]
async fn webapp_vnext_voice_status_requires_auth() {
    let state = webapp_test_state();
    let app = oauth_callback_router(state);
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/webapp/vnext/voice/status")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn webapp_vnext_voice_status_returns_schema_when_authenticated() {
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let (cookie, _csrf) = create_test_web_console_session_with_csrf(&db_path);
    mark_test_web_console_session_mfa_verified(&db_path);
    let state = webapp_test_state_with_db(Some(db_path));
    let app = oauth_callback_router(state);
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/webapp/vnext/voice/status")
                .header("Cookie", format!("__Host-aegis_session={cookie}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), 64 * 1024)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["schema_version"], 1);
    if !cfg!(feature = "experimental-realtime-voice") {
        assert_eq!(json["realtime_available"], false);
        assert_eq!(json["reason"], "deferred_to_next_release");
        assert!(json["key_source"].is_null());
    }
    assert!(
        json.get("realtime_available")
            .and_then(|v| v.as_bool())
            .is_some(),
        "status must report realtime_available, got {json}"
    );
    assert!(
        json.get("model").is_some(),
        "status must report model, got {json}"
    );
    assert!(
        json.get("max_session_seconds").is_some(),
        "status must report max_session_seconds, got {json}"
    );
}

#[tokio::test]
async fn webapp_vnext_voice_session_requires_auth() {
    let state = webapp_test_state();
    let app = oauth_callback_router(state);
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/webapp/vnext/voice/session")
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn webapp_vnext_voice_session_rejects_missing_csrf() {
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let (cookie, _csrf) = create_test_web_console_session_with_csrf(&db_path);
    mark_test_web_console_session_mfa_verified(&db_path);
    let state = webapp_test_state_with_db(Some(db_path));
    let app = oauth_callback_router(state);
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/webapp/vnext/voice/session")
                .header("Cookie", format!("__Host-aegis_session={cookie}"))
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(
        resp.status() == StatusCode::FORBIDDEN || resp.status() == StatusCode::UNAUTHORIZED,
        "CSRF-less voice session mint must be rejected before any side effect, got {}",
        resp.status()
    );
}

/// Authenticated + CSRF mint path with the hermetic Realtime stub (no network).
#[tokio::test]
async fn webapp_vnext_voice_session_mints_stub_client_secret() {
    let _env_lock = WEBAPP_DEBUG_ENV_LOCK.lock().await;
    let _env_guard = VoiceTestStubGuard::set();

    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let (cookie, csrf) = create_test_web_console_session_with_csrf(&db_path);
    mark_test_web_console_session_mfa_verified(&db_path);
    let state = webapp_test_state_with_db(Some(db_path));
    let app = oauth_callback_router(state);
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/webapp/vnext/voice/session")
                .header("Cookie", format!("__Host-aegis_session={cookie}"))
                .header(crate::web_console_auth::CSRF_HEADER_NAME, &csrf)
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    if !cfg!(feature = "experimental-realtime-voice") {
        assert_eq!(resp.status(), StatusCode::NOT_IMPLEMENTED);
        let bytes = axum::body::to_bytes(resp.into_body(), 64 * 1024)
            .await
            .unwrap();
        let body = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(body.contains("deferred to the next release"));
        assert!(!body.contains("client_secret"));
        return;
    }
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), 64 * 1024)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["schema_version"], 1);
    assert_eq!(json["key_source"], "stub");
    let secret = json["client_secret"].as_str().unwrap_or("");
    assert!(
        secret.starts_with("stub-voice-"),
        "stub mint must return a stub client_secret, got {json}"
    );
    assert!(
        json.get("ws_url").is_some(),
        "mint must return ws_url, got {json}"
    );
    assert!(
        json.get("model").is_some(),
        "mint must return model, got {json}"
    );
}

/// Codex chat has no Realtime channel. The mint route must refuse with the
/// owner-facing message and must not return a stub secret. Grok still mints
/// through the hermetic stub.
#[tokio::test]
#[cfg(feature = "experimental-realtime-voice")]
async fn webapp_vnext_voice_session_rejects_codex_and_mints_grok() {
    let _env_lock = WEBAPP_DEBUG_ENV_LOCK.lock().await;
    let _env_guard = VoiceTestStubGuard::set();

    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let (cookie, csrf) = create_test_web_console_session_with_csrf(&db_path);
    mark_test_web_console_session_mfa_verified(&db_path);
    let state = webapp_test_state_with_db(Some(db_path));
    state
        .settings
        .set(
            "llm_provider",
            serde_json::Value::String("codex_oauth".into()),
        )
        .unwrap();
    let app = oauth_callback_router(state.clone());
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/webapp/vnext/voice/session")
                .header("Cookie", format!("__Host-aegis_session={cookie}"))
                .header(crate::web_console_auth::CSRF_HEADER_NAME, &csrf)
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let bytes = axum::body::to_bytes(resp.into_body(), 64 * 1024)
        .await
        .unwrap();
    let body = String::from_utf8(bytes.to_vec()).unwrap();
    assert_eq!(
        body,
        "Realtime voice is not supported on the Codex subscription; switch to Grok for voice, or use chat."
    );
    assert!(
        !body.contains("client_secret"),
        "Codex mint must not return a voice secret, got {body}"
    );

    state
        .settings
        .set(
            "llm_provider",
            serde_json::Value::String("xai_subscription".into()),
        )
        .unwrap();
    let app = oauth_callback_router(state);
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/webapp/vnext/voice/session")
                .header("Cookie", format!("__Host-aegis_session={cookie}"))
                .header(crate::web_console_auth::CSRF_HEADER_NAME, &csrf)
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), 64 * 1024)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["key_source"], "stub");
    let secret = json["client_secret"].as_str().unwrap_or("");
    assert!(
        secret.starts_with("stub-voice-"),
        "Grok mint must still return a stub client_secret, got {json}"
    );
}

// ═════════════════════════════════════════════════════════════════════════════
// Voice notes (webapp composer): audio upload sniffing, server-side STT glue,
// and attachment-only command turns.
//
// The owner asked for a real voice note (Telegram-style: he speaks, Aegis
// hears) instead of the browser dictation the composer used to run. The audio
// arrives at `/webapp/vnext/upload`, is classified by magic number like every
// other upload, and is transcribed on the host before anything reaches the
// orchestrator.
// ═════════════════════════════════════════════════════════════════════════════

/// Minimal byte fixtures for each accepted audio container. Only the leading
/// magic matters to the sniffer, so no real audio is needed here.
fn audio_magic_fixtures() -> Vec<(&'static str, Vec<u8>, &'static str)> {
    let mut webm = vec![0x1A, 0x45, 0xDF, 0xA3];
    webm.extend_from_slice(b"matroska-ish-payload");
    let mut ogg = b"OggS".to_vec();
    ogg.extend_from_slice(b"\x00\x02opus-ish-payload");
    let mut mp4 = vec![0x00, 0x00, 0x00, 0x20];
    mp4.extend_from_slice(b"ftypM4A ");
    mp4.extend_from_slice(b"payload");
    let mut wav = b"RIFF".to_vec();
    wav.extend_from_slice(&[0x24, 0x00, 0x00, 0x00]);
    wav.extend_from_slice(b"WAVEfmt ");
    let mut mp3_id3 = b"ID3".to_vec();
    mp3_id3.extend_from_slice(b"\x03\x00\x00\x00\x00\x00\x00payload");
    let mut mp3_sync = vec![0xFF, 0xFB, 0x90, 0x00];
    mp3_sync.extend_from_slice(b"payload");
    vec![
        ("webm", webm, "audio/webm"),
        ("ogg", ogg, "audio/ogg"),
        ("mp4", mp4, "audio/mp4"),
        ("wav", wav, "audio/wav"),
        ("mp3-id3", mp3_id3, "audio/mpeg"),
        ("mp3-sync", mp3_sync, "audio/mpeg"),
    ]
}

#[test]
fn vnext_upload_sniffs_every_supported_voice_container() {
    for (label, bytes, expected_mime) in audio_magic_fixtures() {
        let sniffed = sniff_upload_kind(&bytes)
            .unwrap_or_else(|| panic!("{label} must be recognized as audio"));
        assert_eq!(
            sniffed,
            (VNextUploadKind::Voice, expected_mime),
            "{label} sniffed to the wrong kind/mime"
        );
        // An `audio/*` declared type (MediaRecorder blobs carry codec params)
        // and an absent one both pass the cross-check.
        assert!(declared_mime_matches_kind(
            Some("audio/webm;codecs=opus"),
            VNextUploadKind::Voice
        ));
        assert!(declared_mime_matches_kind(None, VNextUploadKind::Voice));
        assert!(declared_mime_matches_kind(Some(""), VNextUploadKind::Voice));
        // A non-audio declared type over real audio bytes is a spoof.
        assert!(!declared_mime_matches_kind(
            Some("image/png"),
            VNextUploadKind::Voice
        ));
    }
}

#[test]
fn vnext_upload_audio_sniffing_does_not_shadow_image_or_document_kinds() {
    // WEBP and WAV share the RIFF container: the form tag at offset 8 decides.
    let mut webp = b"RIFF".to_vec();
    webp.extend_from_slice(&[0x1A, 0x00, 0x00, 0x00]);
    webp.extend_from_slice(b"WEBPVP8 ");
    assert_eq!(
        sniff_upload_kind(&webp),
        Some((VNextUploadKind::Image, "image/webp"))
    );
    // JPEG starts FF D8 FF — it must NOT be caught by the MPEG frame-sync rule.
    let mut jpeg = vec![0xFF, 0xD8, 0xFF, 0xE0];
    jpeg.extend_from_slice(b"JFIF");
    assert_eq!(
        sniff_upload_kind(&jpeg),
        Some((VNextUploadKind::Image, "image/jpeg"))
    );
    // PDF stays a document.
    assert_eq!(
        sniff_upload_kind(b"%PDF-1.7 payload"),
        Some((VNextUploadKind::Document, "application/pdf"))
    );
    // Still nothing for unrecognized bytes.
    assert_eq!(sniff_upload_kind(b"plain text, not media"), None);
}

#[test]
fn vnext_voice_upload_names_the_temp_file_after_the_sniffed_container() {
    // ffmpeg selects its demuxer from the extension, so the name must follow
    // the SNIFFED mime, not whatever the browser called the blob.
    assert_eq!(vnext_voice_stt_file_name("audio/webm"), "voice-note.webm");
    assert_eq!(vnext_voice_stt_file_name("audio/ogg"), "voice-note.ogg");
    assert_eq!(vnext_voice_stt_file_name("audio/mp4"), "voice-note.m4a");
    assert_eq!(vnext_voice_stt_file_name("audio/wav"), "voice-note.wav");
    assert_eq!(vnext_voice_stt_file_name("audio/mpeg"), "voice-note.mp3");
    // A voice upload with no filename still gets a sane default basename.
    assert_eq!(
        sanitize_upload_filename(None, VNextUploadKind::Voice),
        "voice-note"
    );
}

#[tokio::test]
async fn vnext_voice_upload_without_stt_configured_is_503() {
    // Disabled backend = the host has neither local whisper nor an STT API key.
    // The owner must get a clear "not configured" answer, never a silent drop.
    let disabled = crate::multimodal::SttBackend::from_values(
        None,
        None,
        None,
        None,
        "",
        "https://api.openai.com/v1",
        "gpt-4o-mini-transcribe",
    );
    assert!(!disabled.is_configured());
    let (status, message) = vnext_transcribe_voice_upload(
        &disabled,
        b"OggS-fake-voice-bytes".to_vec(),
        "voice-note".to_string(),
        "audio/ogg",
    )
    .await
    .expect_err("an unconfigured STT backend must reject the upload");
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(
        message.contains("speech-to-text"),
        "the 503 must name the missing capability, got: {message}"
    );
}

#[test]
fn vnext_voice_upload_result_rejects_a_silent_recording_with_422() {
    // whisper returns "" (or only bracketed [BLANK_AUDIO] markers, already
    // stripped) for silence. That is not a message — 422 `voice_empty`.
    for silent in ["", "   ", "\n\t "] {
        let (status, message) =
            vnext_voice_upload_result(silent, Some(1.5), "voice-note".into(), "audio/ogg")
                .expect_err("a silent recording must not become an empty message");
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(
            message.starts_with("voice_empty"),
            "the 422 must carry the voice_empty code, got: {message}"
        );
    }

    // A real transcript serializes as the documented voice contract.
    let ok = vnext_voice_upload_result(
        "  帮我看看今天的邮件  ",
        Some(4.25),
        "voice-note".into(),
        "audio/webm",
    )
    .expect("a non-empty transcript is accepted");
    let json = serde_json::to_value(&ok).expect("voice upload response serializes");
    assert_eq!(json["kind"], "voice");
    assert_eq!(json["type"], "voice");
    assert_eq!(json["transcript"], "帮我看看今天的邮件");
    assert_eq!(json["mime_type"], "audio/webm");
    assert!(
        (json["duration_secs"].as_f64().unwrap_or_default() - 4.25).abs() < 0.001,
        "duration must round-trip, got {json}"
    );
    // No audio bytes travel back to the browser.
    assert!(
        json.get("data_url").is_none(),
        "a voice upload must not echo the audio back: {json}"
    );

    // An unprobed duration is explicit JSON null (not a missing field), so the
    // client can tell "unknown" from "zero".
    let unknown =
        vnext_voice_upload_result("hello", None, "voice-note".into(), "audio/mp4").unwrap();
    let json = serde_json::to_value(&unknown).unwrap();
    assert!(json["duration_secs"].is_null(), "expected null: {json}");
}

#[tokio::test]
async fn webapp_vnext_upload_accepts_audio_and_reaches_the_stt_stage() {
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let (cookie_token, csrf_token) = create_test_web_console_session_with_csrf(&db_path);
    mark_test_web_console_session_mfa_verified(&db_path);
    let state = webapp_test_state_with_db(Some(db_path));

    let mut ogg = b"OggS".to_vec();
    ogg.extend_from_slice(b"\x00\x02not-real-audio");
    let resp = oauth_callback_router(state)
        .oneshot(build_upload_request(
            &cookie_token,
            Some(&csrf_token),
            "voice-note.ogg",
            Some("audio/ogg;codecs=opus"),
            &ogg,
        ))
        .await
        .unwrap();

    // The route no longer rejects audio as an unsupported type — it now runs
    // the voice branch. What that branch returns depends on the host's STT
    // configuration, so assert on the branch we can prove we reached:
    //   - no STT configured → 503 (the documented contract);
    //   - STT configured    → 502/422, because these bytes are not real audio.
    let status = resp.status();
    assert_ne!(
        status,
        StatusCode::UNSUPPORTED_MEDIA_TYPE,
        "audio must be an accepted upload kind"
    );
    if vnext_stt_backend().is_configured() {
        assert!(
            matches!(
                status,
                StatusCode::BAD_GATEWAY | StatusCode::UNPROCESSABLE_ENTITY
            ),
            "garbage audio with STT configured must fail transcription, got {status}"
        );
    } else {
        assert_eq!(
            status,
            StatusCode::SERVICE_UNAVAILABLE,
            "with STT unconfigured the voice branch must answer 503"
        );
    }
}

/// A `home`/chat command request carrying `input` + `attachments`, with every
/// other field at the shape the composer actually sends.
fn vnext_test_command_request(
    input: &str,
    attachments: Vec<VNextCommandAttachment>,
) -> VNextCommandRequest {
    let mut body = serde_json::json!({
        "schema_version": 1,
        "input": input,
        "mode": "text",
        "intent_kind": "chat",
        "client_context": "home",
    });
    if !attachments.is_empty() {
        body["attachments"] = serde_json::to_value(&attachments).expect("attachments serialize");
    }
    serde_json::from_value(body).expect("test command request deserializes")
}

#[test]
fn vnext_command_accepts_empty_input_when_an_attachment_carries_the_message() {
    // The owner's bug: attaching an image and tapping Send did nothing. The
    // upload succeeded, then the command was rejected for an empty `input` and
    // the browser dropped it silently. An attachment IS a message.
    let image_only = vnext_test_command_request(
        "",
        vec![VNextCommandAttachment::Image {
            data_url: "data:image/png;base64,AAAA".into(),
        }],
    );
    assert!(
        validate_vnext_command_request(&image_only).is_ok(),
        "an image-only turn must be accepted"
    );

    let voice_only = vnext_test_command_request(
        "",
        vec![VNextCommandAttachment::Voice {
            transcript: "提醒我五点给张总回电话".into(),
            duration_secs: 6,
        }],
    );
    assert!(
        validate_vnext_command_request(&voice_only).is_ok(),
        "a voice-note-only turn must be accepted"
    );

    // A turn with NEITHER text nor an attachment is still empty → 400.
    let nothing = vnext_test_command_request("   ", vec![]);
    assert_eq!(
        validate_vnext_command_request(&nothing).unwrap_err().0,
        StatusCode::BAD_REQUEST,
        "a turn with no text and no attachment must still be rejected"
    );

    // A voice attachment whose transcript is blank carries nothing either.
    let blank_voice = vnext_test_command_request(
        "",
        vec![VNextCommandAttachment::Voice {
            transcript: "   ".into(),
            duration_secs: 3,
        }],
    );
    assert_eq!(
        validate_vnext_command_request(&blank_voice).unwrap_err().0,
        StatusCode::BAD_REQUEST
    );
}

#[test]
fn vnext_command_effective_input_derives_text_from_the_attachment() {
    // Typed text always wins, even with an attachment present.
    assert_eq!(
        vnext_command_effective_input(
            "  这张图里写了什么  ",
            &[VNextCommandAttachment::Image {
                data_url: "data:image/png;base64,AAAA".into(),
            }],
            "zh",
        ),
        "这张图里写了什么"
    );
    // Voice: the transcript IS the message (no caption wrapper).
    assert_eq!(
        vnext_command_effective_input(
            "",
            &[VNextCommandAttachment::Voice {
                transcript: " 帮我订明天上午的会 ".into(),
                duration_secs: 5,
            }],
            "zh",
        ),
        "帮我订明天上午的会"
    );
    // Image-only: a localized caption, mirroring the Telegram photo path.
    assert_eq!(
        vnext_command_effective_input(
            "",
            &[VNextCommandAttachment::Image {
                data_url: "data:image/png;base64,AAAA".into(),
            }],
            "zh",
        ),
        "[用户发送了一张图片]"
    );
    assert_eq!(
        vnext_command_effective_input(
            "",
            &[VNextCommandAttachment::Image {
                data_url: "data:image/png;base64,AAAA".into(),
            }],
            "en",
        ),
        "[The owner sent an image]"
    );
    // Document-only: the filename is the only thing the owner named.
    assert_eq!(
        vnext_command_effective_input(
            "",
            &[VNextCommandAttachment::Document {
                data_url: "data:application/pdf;base64,JVBER".into(),
                filename: "2026-Q3-report.pdf".into(),
                mime_type: "application/pdf".into(),
            }],
            "zh",
        ),
        "2026-Q3-report.pdf"
    );
    // Nothing at all stays empty (the validator rejects that turn earlier).
    assert_eq!(vnext_command_effective_input("  ", &[], "zh"), "");
}
