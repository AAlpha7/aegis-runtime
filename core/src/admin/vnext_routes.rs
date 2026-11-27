use super::*;

/// GET /webapp/owner-work/queue — Return a read-only, redacted owner work queue.
pub(super) async fn webapp_assistant_inbox(
    State(state): State<Arc<AdminState>>,
    req: Request<axum::body::Body>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let principal = extract_web_console_auth(&state, req.headers()).await?;
    let chat_id = console_chat_id_for_principal(&state, &principal)?;
    let projection =
        build_unified_inbox_projection(&state, chat_id, &state.settings.display_language()).await;

    Ok(Json(serde_json::to_value(projection).map_err(|err| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Unable to serialize owner work queue: {err}"),
        )
    })?))
}

/// GET /webapp/vnext/home — Product-shaped, read-only owner home projection.
pub(super) async fn webapp_vnext_home(
    State(state): State<Arc<AdminState>>,
    req: Request<axum::body::Body>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let principal = extract_web_console_auth(&state, req.headers()).await?;
    let chat_id = console_chat_id_for_principal(&state, &principal)?;
    let projection =
        build_unified_inbox_projection(&state, chat_id, &state.settings.display_language()).await;
    let home = vnext_home_projection(&projection, &state.settings.display_language());

    Ok(Json(serde_json::to_value(home).map_err(|err| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Unable to serialize vNext home: {err}"),
        )
    })?))
}

/// GET /webapp/vnext/work — Product-shaped, read-only owner work lanes.
pub(super) async fn webapp_vnext_work(
    State(state): State<Arc<AdminState>>,
    req: Request<axum::body::Body>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let principal = extract_web_console_auth(&state, req.headers()).await?;
    let chat_id = console_chat_id_for_principal(&state, &principal)?;
    let projection =
        build_unified_inbox_projection(&state, chat_id, &state.settings.display_language()).await;
    let work = vnext_work_projection(&projection, &state.settings.display_language());

    Ok(Json(serde_json::to_value(work).map_err(|err| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Unable to serialize vNext work: {err}"),
        )
    })?))
}

/// `?turn=` asks for the history window that contains that webapp turn.
/// Empty, overlong, or unknown values leave the newest window in place.
fn chat_history_focus_turn(query: Option<&str>) -> Option<String> {
    let raw = query?.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        (key == "turn").then_some(value)
    })?;
    let decoded = urlencoding::decode(raw).ok()?;
    let trimmed = decoded.trim();
    if trimmed.is_empty() || trimmed.chars().count() > 256 {
        return None;
    }
    Some(trimmed.to_string())
}

/// GET /webapp/vnext/chat-history — Safe owner-visible projection of recent main chat turns.
pub(super) async fn webapp_vnext_chat_history(
    State(state): State<Arc<AdminState>>,
    req: Request<axum::body::Body>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let principal = extract_web_console_auth(&state, req.headers()).await?;
    let chat_id = console_chat_id_for_principal(&state, &principal)?;
    let user_did = webapp_canonical_user_did(chat_id);
    // `?before=<unix seconds>` pages older history (console 加载更早).
    let before_unix = req
        .uri()
        .query()
        .and_then(|query| {
            query
                .split('&')
                .find_map(|pair| pair.strip_prefix("before="))
                .and_then(|raw| raw.parse::<i64>().ok())
        })
        .filter(|value| *value > 0);
    let focus_turn = chat_history_focus_turn(req.uri().query());
    let projection = vnext_chat_history_projection_with_inflight(
        &state.memory,
        &user_did,
        before_unix,
        focus_turn.as_deref(),
        &webapp_inflight_turn_ids(),
        &state.settings.display_language(),
    )
    .await;

    Ok(Json(serde_json::to_value(projection).map_err(|err| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Unable to serialize vNext chat history: {err}"),
        )
    })?))
}

/// GET /webapp/vnext/avatar-profile — Owner-scoped WebApp avatar profile.
pub(super) async fn webapp_vnext_avatar_profile(
    State(state): State<Arc<AdminState>>,
    req: Request<axum::body::Body>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let principal = extract_web_console_auth(&state, req.headers()).await?;
    let (owner, _) = principal_debug_owner(&principal);
    let projection = load_vnext_avatar_profile_for_owner(&state, owner).await?;

    Ok(Json(serde_json::to_value(projection).map_err(|err| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Unable to serialize avatar profile: {err}"),
        )
    })?))
}

/// PUT /webapp/vnext/avatar-profile — Update owner-scoped WebApp avatar profile.
pub(super) async fn webapp_vnext_update_avatar_profile(
    State(state): State<Arc<AdminState>>,
    req: Request<axum::body::Body>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let (parts, body) = req.into_parts();
    let headers = parts.headers;
    let principal = extract_web_console_auth(&state, &headers).await?;
    require_web_console_csrf_for_principal(&headers, &principal)?;

    let body_bytes = axum::body::to_bytes(body, 2 * 1024 * 1024)
        .await
        .map_err(|_| {
            (
                StatusCode::BAD_REQUEST,
                "Invalid avatar profile body".to_string(),
            )
        })?;
    let request: VNextAvatarProfileUpdateRequest =
        serde_json::from_slice(&body_bytes).map_err(|_| {
            (
                StatusCode::BAD_REQUEST,
                "Invalid avatar profile JSON".to_string(),
            )
        })?;
    let profile = validate_vnext_avatar_update(request)?;
    let (owner, _) = principal_debug_owner(&principal);
    let saved = save_vnext_avatar_profile_for_owner(&state, owner, profile).await?;

    Ok(Json(serde_json::to_value(saved).map_err(|err| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Unable to serialize avatar profile: {err}"),
        )
    })?))
}

/// PUT /webapp/vnext/avatar-profile/auto-refresh — Configure Aegis weekly avatar refresh.
pub(super) async fn webapp_vnext_update_avatar_auto_refresh(
    State(state): State<Arc<AdminState>>,
    req: Request<axum::body::Body>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let (parts, body) = req.into_parts();
    let headers = parts.headers;
    let principal = extract_web_console_auth(&state, &headers).await?;
    require_web_console_csrf_for_principal(&headers, &principal)?;

    let body_bytes = axum::body::to_bytes(body, 32 * 1024).await.map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            "Invalid avatar auto-refresh body".to_string(),
        )
    })?;
    let request: VNextAvatarAutoRefreshUpdateRequest = serde_json::from_slice(&body_bytes)
        .map_err(|_| {
            (
                StatusCode::BAD_REQUEST,
                "Invalid avatar auto-refresh JSON".to_string(),
            )
        })?;
    validate_context_schema_version(request.schema_version)?;
    persist_runtime_setting(
        &state,
        "aegis_avatar_auto_refresh_enabled",
        serde_json::json!(request.enabled),
    )
    .await
    .map_err(|err| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Unable to save avatar auto-refresh setting: {err}"),
        )
    })?;
    if let Some(prompt_hint) = request.prompt_hint {
        persist_runtime_setting(
            &state,
            "aegis_avatar_auto_refresh_prompt_hint",
            serde_json::json!(sanitize_avatar_prompt_hint(&prompt_hint)),
        )
        .await
        .map_err(|err| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Unable to save avatar style hint: {err}"),
            )
        })?;
    }

    let (owner, _) = principal_debug_owner(&principal);
    let projection = load_vnext_avatar_profile_for_owner(&state, owner).await?;
    Ok(Json(serde_json::to_value(projection).map_err(|err| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Unable to serialize avatar profile: {err}"),
        )
    })?))
}

/// POST /webapp/vnext/avatar-profile/generate-aegis — Generate and save Aegis avatar.
pub(super) async fn webapp_vnext_generate_aegis_avatar(
    State(state): State<Arc<AdminState>>,
    req: Request<axum::body::Body>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let (parts, body) = req.into_parts();
    let headers = parts.headers;
    let principal = extract_web_console_auth(&state, &headers).await?;
    require_web_console_csrf_for_principal(&headers, &principal)?;
    let body_bytes = axum::body::to_bytes(body, 32 * 1024).await.map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            "Invalid avatar generation body".to_string(),
        )
    })?;
    let request: VNextAegisAvatarGenerateRequest =
        serde_json::from_slice(&body_bytes).map_err(|_| {
            (
                StatusCode::BAD_REQUEST,
                "Invalid avatar generation JSON".to_string(),
            )
        })?;
    validate_context_schema_version(request.schema_version)?;
    let prompt_hint = request
        .prompt_hint
        .as_deref()
        .map(sanitize_avatar_prompt_hint)
        .unwrap_or_else(|| {
            vnext_avatar_setting_string(&state, "aegis_avatar_auto_refresh_prompt_hint", "")
        });
    let (owner, _) = principal_debug_owner(&principal);
    let response = match generate_and_store_aegis_avatar(&state, owner, prompt_hint, false).await {
        Ok(response) => response,
        Err((status, message)) => {
            tracing::warn!(
                %status,
                error = %message,
                "Aegis avatar generation request failed"
            );
            return Err((status, message));
        }
    };
    Ok(Json(serde_json::to_value(response).map_err(|err| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Unable to serialize avatar generation response: {err}"),
        )
    })?))
}

/// GET /webapp/vnext/avatar-image/:filename — owner-scoped generated avatar bytes.
pub(super) async fn webapp_vnext_avatar_image(
    State(state): State<Arc<AdminState>>,
    Path(filename): Path<String>,
    req: Request<axum::body::Body>,
) -> Result<Response, (StatusCode, String)> {
    let principal = extract_web_console_auth(&state, req.headers()).await?;
    let (owner, _) = principal_debug_owner(&principal);
    if !is_safe_generated_avatar_filename(&filename) {
        return Err((StatusCode::NOT_FOUND, "Avatar image not found".into()));
    }
    let owner_hash = webapp_generated_avatar_owner_hash(&owner);
    if !filename.starts_with(&format!("aegis-{owner_hash}-")) {
        return Err((StatusCode::NOT_FOUND, "Avatar image not found".into()));
    }
    let path = webapp_generated_avatar_storage_dir(&state).join(&filename);
    let bytes = std::fs::read(&path)
        .map_err(|_| (StatusCode::NOT_FOUND, "Avatar image not found".to_string()))?;
    if bytes.len() > WEBAPP_GENERATED_AVATAR_MAX_BYTES {
        return Err((
            StatusCode::BAD_REQUEST,
            "Avatar image is too large".to_string(),
        ));
    }

    Response::builder()
        .status(StatusCode::OK)
        .header(
            axum::http::header::CONTENT_TYPE,
            generated_avatar_content_type(&filename),
        )
        .body(axum::body::Body::from(bytes))
        .map_err(|err| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Unable to serve avatar image: {err}"),
            )
        })
}

/// GET /webapp/vnext/generated-image/:filename — owner-scoped generated image bytes.
pub(super) async fn webapp_vnext_generated_image(
    State(state): State<Arc<AdminState>>,
    Path(filename): Path<String>,
    req: Request<axum::body::Body>,
) -> Result<Response, (StatusCode, String)> {
    let principal = extract_web_console_auth(&state, req.headers()).await?;
    let chat_id = console_chat_id_for_principal(&state, &principal)?;
    let owner_did = webapp_canonical_user_did(chat_id);
    if !crate::webapp_assets::is_safe_generated_image_filename(&filename) {
        return Err((StatusCode::NOT_FOUND, "Generated image not found".into()));
    }
    let owner_hash = crate::webapp_assets::webapp_asset_owner_hash(&owner_did);
    if !filename.starts_with(&format!("image-{owner_hash}-")) {
        return Err((StatusCode::NOT_FOUND, "Generated image not found".into()));
    }
    let path = crate::webapp_assets::webapp_generated_image_storage_dir(state.memory.db_path())
        .join(&filename);
    let bytes = std::fs::read(&path).map_err(|_| {
        (
            StatusCode::NOT_FOUND,
            "Generated image not found".to_string(),
        )
    })?;
    if bytes.len() > crate::webapp_assets::WEBAPP_GENERATED_IMAGE_MAX_BYTES {
        return Err((
            StatusCode::BAD_REQUEST,
            "Generated image is too large".to_string(),
        ));
    }

    Response::builder()
        .status(StatusCode::OK)
        .header(
            axum::http::header::CONTENT_TYPE,
            crate::webapp_assets::generated_image_content_type(&filename),
        )
        .body(axum::body::Body::from(bytes))
        .map_err(|err| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Unable to serve generated image: {err}"),
            )
        })
}

/// GET /webapp/vnext/work/:item_id — Product-shaped, read-only item detail shell.
pub(super) async fn webapp_vnext_item(
    State(state): State<Arc<AdminState>>,
    Path(item_id): Path<String>,
    req: Request<axum::body::Body>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    validate_context_focus_id(&item_id)?;
    let principal = extract_web_console_auth(&state, req.headers()).await?;
    let chat_id = console_chat_id_for_principal(&state, &principal)?;
    let projection =
        build_unified_inbox_projection(&state, chat_id, &state.settings.display_language()).await;
    let item = projection
        .items
        .iter()
        .find(|item| item.id == item_id)
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                "vNext work item not found".to_string(),
            )
        })?;
    let detail = vnext_item_projection(
        item,
        &projection.generated_at,
        &state.settings.display_language(),
    );

    Ok(Json(serde_json::to_value(detail).map_err(|err| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Unable to serialize vNext item: {err}"),
        )
    })?))
}

/// Slice 5: raw multipart body cap for `/webapp/vnext/upload` (enforced by the
/// axum `DefaultBodyLimit` layer). Set above the 10 MB document ceiling with
/// headroom for multipart framing + base64 expansion is NOT applicable here
/// (we receive RAW bytes, not base64) so this is the raw-file ceiling + slack.
pub(super) const VNEXT_UPLOAD_MAX_BODY_BYTES: usize = 12 * 1024 * 1024;
/// Max raw image bytes (jpeg/png/webp/gif).
pub(super) const VNEXT_UPLOAD_MAX_IMAGE_BYTES: usize = 5 * 1024 * 1024;
/// Max raw document bytes (pdf).
const VNEXT_UPLOAD_MAX_DOCUMENT_BYTES: usize = 10 * 1024 * 1024;
/// Max raw voice-note bytes. A 3-minute Opus/AAC note is well under 1 MB, so
/// 10 MB is generous headroom for a bad encoder without needing a bigger raw
/// body cap (`VNEXT_UPLOAD_MAX_BODY_BYTES` stays at 12 MB).
pub(super) const VNEXT_UPLOAD_MAX_VOICE_BYTES: usize = 10 * 1024 * 1024;
/// Low per-minute rate limit for uploads (cheaper than a command, but in-memory
/// base64 of multi-MB files is not free — keep it tight).
const VNEXT_UPLOAD_RATE_LIMIT_PER_MINUTE: usize = 12;

/// Result of classifying an uploaded file by magic-number (and cross-checked
/// against the client-declared content-type).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum VNextUploadKind {
    Image,
    Document,
    /// A browser-recorded voice note (`MediaRecorder` output). Transcribed
    /// server-side by the STT backend; the audio bytes never leave the request.
    Voice,
}

/// Slice 5: sniff the file kind + canonical MIME from the leading magic-number
/// bytes. Returns `None` for anything we do not accept. This is the AUTHORITATIVE
/// MIME source — the client-declared content-type is only used as a coarse
/// cross-check; a spoofed header (e.g. `image/png` on a PDF) is rejected because
/// the magic bytes win.
pub(super) fn sniff_upload_kind(bytes: &[u8]) -> Option<(VNextUploadKind, &'static str)> {
    // JPEG: FF D8 FF
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Some((VNextUploadKind::Image, "image/jpeg"));
    }
    // PNG: 89 50 4E 47 0D 0A 1A 0A
    if bytes.starts_with(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]) {
        return Some((VNextUploadKind::Image, "image/png"));
    }
    // GIF: "GIF87a" / "GIF89a"
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Some((VNextUploadKind::Image, "image/gif"));
    }
    // WEBP: "RIFF" .... "WEBP"
    if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        return Some((VNextUploadKind::Image, "image/webp"));
    }
    // WAV: "RIFF" .... "WAVE" (same container family as WEBP — the 4-byte form
    // tag at offset 8 is what separates them, so this must stay next to it).
    if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WAVE" {
        return Some((VNextUploadKind::Voice, "audio/wav"));
    }
    // PDF: "%PDF-"
    if bytes.starts_with(b"%PDF-") {
        return Some((VNextUploadKind::Document, "application/pdf"));
    }
    // Matroska/WebM (EBML): 1A 45 DF A3. Chrome/Firefox MediaRecorder emits
    // `audio/webm;codecs=opus` in this container.
    if bytes.starts_with(&[0x1A, 0x45, 0xDF, 0xA3]) {
        return Some((VNextUploadKind::Voice, "audio/webm"));
    }
    // Ogg: "OggS" (Firefox `audio/ogg;codecs=opus`).
    if bytes.starts_with(b"OggS") {
        return Some((VNextUploadKind::Voice, "audio/ogg"));
    }
    // ISO-BMFF / MP4: "ftyp" at offset 4. iOS Safari MediaRecorder emits
    // `audio/mp4` (AAC) — the only container it supports.
    if bytes.len() >= 8 && &bytes[4..8] == b"ftyp" {
        return Some((VNextUploadKind::Voice, "audio/mp4"));
    }
    // MP3: "ID3" tag, or a raw MPEG audio frame sync (FF Ex).
    if bytes.starts_with(b"ID3")
        || (bytes.len() >= 2 && bytes[0] == 0xFF && (bytes[1] & 0xE0) == 0xE0)
    {
        return Some((VNextUploadKind::Voice, "audio/mpeg"));
    }
    None
}

/// Slice 5: does the client-declared content-type plausibly match the sniffed
/// kind? Used only as a coarse cross-check — the magic-number is authoritative.
/// An empty/absent declared type is allowed (browsers sometimes omit it); a
/// declared type that contradicts the sniffed kind is rejected as a spoof.
pub(super) fn declared_mime_matches_kind(declared: Option<&str>, kind: VNextUploadKind) -> bool {
    let Some(declared) = declared.map(str::trim).filter(|value| !value.is_empty()) else {
        return true;
    };
    let declared = declared.to_ascii_lowercase();
    match kind {
        VNextUploadKind::Image => declared.starts_with("image/"),
        VNextUploadKind::Document => declared == "application/pdf",
        // MediaRecorder blob types carry codec parameters (e.g.
        // `audio/webm;codecs=opus`), so match on the `audio/` prefix only.
        VNextUploadKind::Voice => declared.starts_with("audio/"),
    }
}

/// Sanitize an uploaded filename to a safe, short basename. Strips any path
/// components and control chars; falls back to a kind-appropriate default.
pub(super) fn sanitize_upload_filename(raw: Option<&str>, kind: VNextUploadKind) -> String {
    let default = match kind {
        VNextUploadKind::Image => "image",
        VNextUploadKind::Document => "document.pdf",
        VNextUploadKind::Voice => "voice-note",
    };
    let Some(raw) = raw else {
        return default.to_string();
    };
    // Take the basename only (drop any directory components from either separator).
    let base = raw.rsplit(['/', '\\']).next().unwrap_or(raw).trim();
    let cleaned: String = base
        .chars()
        .filter(|ch| !ch.is_control())
        .take(200)
        .collect();
    let cleaned = cleaned.trim();
    if cleaned.is_empty() {
        default.to_string()
    } else {
        cleaned.to_string()
    }
}

#[derive(Debug, Serialize)]
pub(super) struct VNextUploadResponse {
    /// Base64 payload for image/document uploads. Absent for voice: the audio
    /// bytes stay on the server (only the transcript rides the later command),
    /// so shipping a multi-MB data URL back to the browser would be pure waste.
    #[serde(skip_serializing_if = "Option::is_none")]
    data_url: Option<String>,
    /// Historical discriminator name (the frontend upload parser reads this).
    #[serde(rename = "type")]
    kind_type: &'static str,
    /// Same value as `type`, under the name the voice-note contract specifies.
    /// Both are emitted so neither spelling is a breaking change.
    kind: &'static str,
    filename: String,
    mime_type: &'static str,
    /// Server-side STT result. Voice uploads only.
    #[serde(skip_serializing_if = "Option::is_none")]
    transcript: Option<String>,
    /// Voice uploads only: probed duration, or JSON `null` when `ffprobe`
    /// could not read one. Omitted entirely for non-voice kinds (the outer
    /// `Option` is "is this a voice upload", the inner one "did probing work").
    #[serde(skip_serializing_if = "Option::is_none")]
    duration_secs: Option<Option<f32>>,
}

/// Process-wide speech-to-text backend for WebApp voice-note uploads.
///
/// Built once from the same environment knobs the Telegram `MediaProcessor`
/// uses (`AEGIS_STT_LOCAL_BIN` + `AEGIS_STT_LOCAL_MODEL` for local whisper.cpp,
/// else `AEGIS_STT_API_KEY`). A `OnceLock` rather than an `AdminState` field so
/// every existing state construction (production + a dozen test harnesses)
/// stays untouched.
pub(super) fn vnext_stt_backend() -> &'static crate::multimodal::SttBackend {
    static BACKEND: std::sync::OnceLock<crate::multimodal::SttBackend> = std::sync::OnceLock::new();
    BACKEND.get_or_init(crate::multimodal::SttBackend::from_env)
}

/// Map a finished transcription onto the voice upload result, or onto the
/// owner-visible rejection for an empty one. Pure — split out so the
/// "silence transcribes to nothing" contract is unit-testable without STT.
pub(super) fn vnext_voice_upload_result(
    transcript: &str,
    duration_secs: Option<f32>,
    filename: String,
    mime_type: &'static str,
) -> Result<VNextUploadResponse, (StatusCode, String)> {
    let transcript = transcript.trim();
    if transcript.is_empty() {
        return Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            "voice_empty: no speech was recognized in this recording".to_string(),
        ));
    }
    Ok(VNextUploadResponse {
        data_url: None,
        kind_type: "voice",
        kind: "voice",
        filename,
        mime_type,
        transcript: Some(transcript.to_string()),
        duration_secs: Some(duration_secs),
    })
}

/// Transcribe an uploaded voice note with `stt` and shape the upload response.
///
/// Owner-visible failure modes, all structured (never a silent drop):
/// - STT not configured on the host → `503`
/// - transcription failed (ffmpeg/whisper/API error) → `502`
/// - the recording carried no speech → `422 voice_empty`
pub(super) async fn vnext_transcribe_voice_upload(
    stt: &crate::multimodal::SttBackend,
    data: Vec<u8>,
    filename: String,
    mime_type: &'static str,
) -> Result<VNextUploadResponse, (StatusCode, String)> {
    if !stt.is_configured() {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "Voice notes need speech-to-text configured on the server (AEGIS_STT_LOCAL_BIN + AEGIS_STT_LOCAL_MODEL, or AEGIS_STT_API_KEY)."
                .to_string(),
        ));
    }
    let stt_file_name = vnext_voice_stt_file_name(mime_type);
    let duration_bytes = data.clone();
    let duration_name = stt_file_name.clone();
    let duration_secs = tokio::task::spawn_blocking(move || {
        crate::multimodal::probe_audio_duration_secs(&duration_bytes, &duration_name)
    })
    .await
    .unwrap_or(None);
    let transcript = crate::multimodal::transcribe_audio_with(stt, data, &stt_file_name)
        .await
        .map_err(|err| {
            tracing::warn!(error = %err, mime_type, "vNext voice upload transcription failed");
            (
                StatusCode::BAD_GATEWAY,
                "Could not transcribe this voice note. Try recording it again.".to_string(),
            )
        })?;
    vnext_voice_upload_result(&transcript, duration_secs, filename, mime_type)
}

/// ffmpeg picks its demuxer from the file extension we hand it, so the temp
/// file name must match the sniffed container (NOT the owner-supplied name,
/// which a browser may leave as a bare `blob`).
pub(super) fn vnext_voice_stt_file_name(mime_type: &str) -> String {
    let ext = match mime_type {
        "audio/webm" => "webm",
        "audio/ogg" => "ogg",
        "audio/mp4" => "m4a",
        "audio/wav" => "wav",
        "audio/mpeg" => "mp3",
        _ => "bin",
    };
    format!("voice-note.{ext}")
}

/// POST /webapp/vnext/upload — Slice 5 multimodal upload.
///
/// Accepts one `multipart/form-data` file, validates its MIME by BOTH the
/// client-declared content-type AND the leading magic-number bytes (the latter
/// is authoritative — a spoofed header is rejected), enforces per-kind size
/// caps (images ≤5 MB, PDFs ≤10 MB), base64-encodes it IN MEMORY (no disk
/// write), and returns a `data_url` descriptor the composer attaches to a
/// later command. Auth = web-console session + CSRF; a low per-minute rate
/// limit guards the in-memory encode cost. The raw body size is capped at the
/// transport layer by the `DefaultBodyLimit` route layer.
pub(super) async fn webapp_vnext_upload(
    State(state): State<Arc<AdminState>>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> Result<Json<VNextUploadResponse>, (StatusCode, String)> {
    let principal = extract_web_console_auth(&state, &headers).await?;
    require_web_console_csrf_for_principal(&headers, &principal)?;
    check_webapp_rate_limit(
        &principal,
        "vnext_upload",
        "upload",
        VNEXT_UPLOAD_RATE_LIMIT_PER_MINUTE,
    )?;

    // Read the first file field. We accept exactly one file; extra fields are ignored.
    let mut file_field: Option<(Option<String>, Option<String>, Vec<u8>)> = None;
    loop {
        let field = multipart.next_field().await.map_err(|err| {
            // A body-limit overflow surfaces here as a multipart error.
            tracing::warn!(error = %err, "vNext upload multipart read failed");
            (
                StatusCode::BAD_REQUEST,
                "Invalid or oversized upload".to_string(),
            )
        })?;
        let Some(field) = field else { break };
        // Only consider the first field that carries a filename (i.e. a file part).
        let declared_filename = field.file_name().map(str::to_string);
        let declared_content_type = field.content_type().map(str::to_string);
        let data = field.bytes().await.map_err(|err| {
            tracing::warn!(error = %err, "vNext upload field read failed");
            (
                StatusCode::BAD_REQUEST,
                "Invalid or oversized upload".to_string(),
            )
        })?;
        if declared_filename.is_some() || !data.is_empty() {
            file_field = Some((declared_filename, declared_content_type, data.to_vec()));
            break;
        }
    }

    let Some((declared_filename, declared_content_type, data)) = file_field else {
        return Err((StatusCode::BAD_REQUEST, "No file in upload".to_string()));
    };
    if data.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "Empty file upload".to_string()));
    }

    // Authoritative classification by magic-number.
    let Some((kind, mime_type)) = sniff_upload_kind(&data) else {
        return Err((
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "Unsupported file type".to_string(),
        ));
    };
    // Cross-check the declared content-type; reject an obvious spoof.
    if !declared_mime_matches_kind(declared_content_type.as_deref(), kind) {
        tracing::warn!(
            principal = %principal.log_subject(),
            declared = ?declared_content_type,
            sniffed = mime_type,
            "vNext upload content-type / magic-number mismatch (rejected)"
        );
        return Err((
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "Declared file type does not match contents".to_string(),
        ));
    }
    // Per-kind size cap.
    let max_bytes = match kind {
        VNextUploadKind::Image => VNEXT_UPLOAD_MAX_IMAGE_BYTES,
        VNextUploadKind::Document => VNEXT_UPLOAD_MAX_DOCUMENT_BYTES,
        VNextUploadKind::Voice => VNEXT_UPLOAD_MAX_VOICE_BYTES,
    };
    if data.len() > max_bytes {
        return Err((
            StatusCode::PAYLOAD_TOO_LARGE,
            "File exceeds the size limit".to_string(),
        ));
    }

    let filename = sanitize_upload_filename(declared_filename.as_deref(), kind);
    let kind_label = match kind {
        VNextUploadKind::Image => "image",
        VNextUploadKind::Document => "document",
        VNextUploadKind::Voice => "voice",
    };
    tracing::info!(
        principal = %principal.log_subject(),
        kind = kind_label,
        mime_type,
        raw_bytes = data.len(),
        "vNext upload accepted"
    );
    // Voice: transcribe on the host and return TEXT, not audio. This is a real
    // voice note (Telegram-style), not browser dictation — the owner's speech
    // reaches the orchestrator through the same STT path Telegram uses.
    if kind == VNextUploadKind::Voice {
        let raw_bytes = data.len();
        let response =
            vnext_transcribe_voice_upload(vnext_stt_backend(), data, filename, mime_type).await?;
        tracing::info!(
            principal = %principal.log_subject(),
            mime_type,
            raw_bytes,
            transcript_len = response.transcript.as_deref().map(str::len).unwrap_or(0),
            "vNext voice upload transcribed"
        );
        return Ok(Json(response));
    }
    let encoded = base64::engine::general_purpose::STANDARD.encode(&data);
    let data_url = format!("data:{mime_type};base64,{encoded}");
    Ok(Json(VNextUploadResponse {
        data_url: Some(data_url),
        kind_type: kind_label,
        kind: kind_label,
        filename,
        mime_type,
        transcript: None,
        duration_secs: None,
    }))
}

/// POST /webapp/vnext/command — Product-shaped command wrapper over owner-work runtime.
pub(super) async fn webapp_vnext_command(
    State(state): State<Arc<AdminState>>,
    req: Request<axum::body::Body>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let (parts, body) = req.into_parts();
    let headers = parts.headers;
    // Diagnostic visibility for the "webapp chat returns 我没能完成这件事"
    // recurrence: every pre-agent rejection (auth / CSRF / body / validation)
    // used to `?`-propagate SILENTLY, so a command that never reached the agent
    // left zero server trace and the failure was impossible to attribute from
    // logs. Each gate now emits ONE `warn!` carrying the stage, status, and
    // whether the request even presented a cookie / CSRF header — enough to tell
    // a dropped-cookie 401 from a stale-CSRF 403 the next time it happens.
    // Behaviour is UNCHANGED: same status/message returned, just observable.
    let has_cookie = headers.contains_key(axum::http::header::COOKIE);
    let has_csrf = headers.contains_key(web_console_auth::CSRF_HEADER_NAME);
    let principal = match extract_web_console_auth(&state, &headers).await {
        Ok(principal) => principal,
        Err((status, message)) => {
            tracing::warn!(
                stage = "auth",
                %status,
                has_cookie,
                has_csrf,
                error = %message,
                "WebApp vNext command REJECTED pre-agent"
            );
            return Err((status, message));
        }
    };
    if let Err((status, message)) = require_web_console_csrf_for_principal(&headers, &principal) {
        tracing::warn!(
            stage = "csrf",
            %status,
            has_cookie,
            has_csrf,
            principal = %principal.log_subject(),
            error = %message,
            "WebApp vNext command REJECTED pre-agent"
        );
        return Err((status, message));
    }

    let body_bytes = match axum::body::to_bytes(body, VNEXT_COMMAND_MAX_BODY_BYTES).await {
        Ok(bytes) => bytes,
        Err(_) => {
            tracing::warn!(
                stage = "body",
                principal = %principal.log_subject(),
                "WebApp vNext command REJECTED pre-agent"
            );
            return Err((
                StatusCode::BAD_REQUEST,
                "Invalid vNext command body".to_string(),
            ));
        }
    };
    let mut request: VNextCommandRequest = match serde_json::from_slice(&body_bytes) {
        Ok(request) => request,
        Err(_) => {
            tracing::warn!(
                stage = "json",
                principal = %principal.log_subject(),
                "WebApp vNext command REJECTED pre-agent"
            );
            return Err((
                StatusCode::BAD_REQUEST,
                "Invalid vNext command JSON".to_string(),
            ));
        }
    };
    // Older/greedier clients send more context than the validator allows;
    // keep the screen turn plus the newest ones instead of failing the turn
    // (2026-09-06: every owner message 400'd after the chat-panel merge).
    if let Some(turns) = request.conversation_context.as_mut() {
        trim_vnext_conversation_context(turns);
    }
    if let Err((status, message)) = validate_context_schema_version(request.schema_version)
        .and_then(|()| validate_vnext_command_request(&request))
    {
        tracing::warn!(
            stage = "validate",
            %status,
            principal = %principal.log_subject(),
            error = %message,
            "WebApp vNext command REJECTED pre-agent"
        );
        return Err((status, message));
    }

    let focus_id = request
        .focus_item_id
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(GENERAL_ASSISTANT_FOCUS_ID);
    let intent_kind = request.intent_kind.as_deref().unwrap_or("explain");
    // Attachment-only turn: derive the message text from the attachment (voice
    // transcript / image caption / document filename) so the orchestrator sees
    // a real message. Identical to `request.input` whenever the owner typed
    // something.
    let effective_input = vnext_command_effective_input(
        &request.input,
        &request.attachments,
        &state.settings.display_language(),
    );
    tracing::info!(
        principal = %principal.log_subject(),
        focus_id,
        intent_kind,
        client_context = %request.client_context,
        mode = %request.mode,
        has_idempotency_key = request.idempotency_key.is_some(),
        "WebApp vNext command accepted"
    );
    let idempotency_key = request.idempotency_key.clone();
    let command_chat_id = if request.client_context == "home" {
        Some(console_chat_id_for_principal(&state, &principal)?)
    } else {
        None
    };
    if is_vnext_tasks_slash_command(&effective_input) {
        let chat_id = match command_chat_id {
            Some(chat_id) => chat_id,
            None => console_chat_id_for_principal(&state, &principal)?,
        };
        return webapp_vnext_tasks_command_response(&state, chat_id).await;
    }
    // Snapshot of work-item ids from before the turn, so a work item the turn
    // creates can be attached to the reply. The projection is slow (mail,
    // approvals, tasks: ~10 s on 2026-09-05), so it runs concurrently with the
    // turn; `finalize_vnext_command_response` awaits it (or builds it itself
    // if the background task was dropped) via the init-once cell.
    let prior_work_item_ids: Option<PriorWorkItemSnapshot> = command_chat_id.map(|chat_id| {
        let cell: PriorWorkItemSnapshot = Arc::new(tokio::sync::OnceCell::new());
        let cell_for_task = cell.clone();
        let state_for_task = state.clone();
        tokio::spawn(async move {
            let _ = cell_for_task
                .get_or_init(|| snapshot_prior_work_item_ids(state_for_task, chat_id))
                .await;
        });
        cell
    });
    // Slice 5: forward multimodal attachments to the assistant handler. Only the
    // general-chat (`home`) path consumes them downstream; the focused/contextual
    // branches in `webapp_assistant_respond` never read `attachments`, so passing
    // them for a non-home context is inert (and home is the only context the
    // composer can attach files from).
    let attachments_json =
        serde_json::to_value(&request.attachments).unwrap_or(serde_json::Value::Null);
    let owner_request = serde_json::json!({
        "schema_version": 1,
        "focus_id": focus_id,
        "intent_kind": intent_kind,
        "user_text": effective_input,
        "requested_output_kind": "text",
        "idempotency_key": idempotency_key,
        "attachments": attachments_json,
        "client_context_hint": {
            "surface": vnext_command_surface_for_context(&request.client_context),
            "client_context": request.client_context,
            "mode": request.mode,
            "conversation_context": vnext_command_conversation_context_json(
                request.conversation_context.as_deref()
            )
        }
    });
    // Final response envelope, post-processed at its point of production so
    // every delivery path (foreground wait, joined wait, status poll) is
    // byte-identical.
    let response = if let Some(idempotency_key) = idempotency_key
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let singleflight_key =
            webapp_command_singleflight_key(&principal, focus_id, intent_kind, idempotency_key);
        let (entry, should_execute) = webapp_command_singleflight_start(singleflight_key.clone());
        if should_execute {
            let state_for_task = state.clone();
            let headers_for_task = headers.clone();
            let owner_request_for_task = owner_request.clone();
            let entry_for_task = entry.clone();
            // Owner DID for the home-context owner so a bg-completed turn can
            // fire a web push (the foreground HTTP wait may have already
            // 504'd and the client gone silent). `None` for non-home context
            // (no canonical owner chat) — then we simply skip the push.
            let push_owner_did = command_chat_id.map(webapp_canonical_user_did);
            // Snapshot of work-item ids taken before the turn ran, so the bg
            // task can apply the SAME post-processing the synchronized path
            // applies and store the FINAL envelope (see F1 / Codex review).
            let prior_ids_for_task = prior_work_item_ids.clone();
            let inflight_turn_id = idempotency_key.to_string();
            tokio::spawn(async move {
                let _inflight = WebappInflightTurnGuard::enter(&inflight_turn_id);
                let raw = webapp_assistant_respond(
                    State(state_for_task.clone()),
                    forwarded_json_request(headers_for_task, owner_request_for_task),
                )
                .await
                .map(|response| response.0);
                // Post-process INSIDE the bg task so the stored result is the
                // final response envelope — foreground waiters, joined-request
                // waiters, AND the `/command/status` poll all read identical
                // bodies. (Previously post-processing ran only in the foreground,
                // so a polled/timed-out command returned a different shape.)
                let result = match raw {
                    Ok(owner_response) => {
                        finalize_vnext_command_response(
                            &state_for_task,
                            command_chat_id,
                            prior_ids_for_task.as_ref(),
                            owner_response,
                        )
                        .await
                    }
                    Err(err) => Err(err),
                };
                let completed_ok = result.is_ok();
                webapp_command_singleflight_finish(&entry_for_task, result);
                // Never-silent: on Ok completion, fire a fire-and-forget web
                // push so the owner knows to reopen the app / refresh chat
                // history even if their foreground request already timed out.
                // No registered subscriptions = no-op early return (fine).
                // We always push on Ok (cheap; frontend de-dupes fast cases).
                if completed_ok
                    && let Some(owner_did) = push_owner_did
                    && let Ok(notice) = WebPushNotice::new(
                        "Aegis responded",
                        "Your request finished — tap to see the reply.",
                        "aegis-bg-response",
                        WEB_PUSH_DEFAULT_URL,
                    )
                {
                    let _ =
                        web_push_send_notice_to_owner(&state_for_task.memory, &owner_did, notice)
                            .await;
                }
            });
        } else {
            tracing::info!(
                principal = %principal.log_subject(),
                focus_id,
                intent_kind,
                "WebApp vNext command joined existing idempotent request"
            );
        }

        match tokio::time::timeout(
            WEBAPP_COMMAND_SINGLEFLIGHT_WAIT,
            webapp_command_singleflight_wait(entry),
        )
        .await
        {
            Ok(Ok(value)) => value,
            Ok(Err((status, message))) => {
                tracing::warn!(
                    principal = %principal.log_subject(),
                    focus_id,
                    intent_kind,
                    client_context = %request.client_context,
                    status = %status,
                    error = %message,
                    "WebApp vNext command failed"
                );
                return Err((status, message));
            }
            Err(_) => {
                tracing::warn!(
                    principal = %principal.log_subject(),
                    focus_id,
                    intent_kind,
                    client_context = %request.client_context,
                    "WebApp vNext command still running after reconnect wait"
                );
                return Err((
                    StatusCode::GATEWAY_TIMEOUT,
                    "Aegis is still working on this request. Try again in a moment.".to_string(),
                ));
            }
        }
    } else {
        let owner_response = match webapp_assistant_respond(
            State(state.clone()),
            forwarded_json_request(headers, owner_request),
        )
        .await
        {
            Ok(response) => response.0,
            Err((status, message)) => {
                tracing::warn!(
                    principal = %principal.log_subject(),
                    focus_id,
                    intent_kind,
                    client_context = %request.client_context,
                    status = %status,
                    error = %message,
                    "WebApp vNext command failed"
                );
                return Err((status, message));
            }
        };
        // Same post-processing the bg-task path applies, so both yield the
        // final envelope.
        finalize_vnext_command_response(
            &state,
            command_chat_id,
            prior_work_item_ids.as_ref(),
            owner_response,
        )
        .await?
    };
    let outcome = response
        .get("outcome")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown");
    tracing::info!(
        principal = %principal.log_subject(),
        focus_id,
        intent_kind,
        client_context = %request.client_context,
        outcome,
        "WebApp vNext command completed"
    );
    Ok(Json(response))
}

/// Persist the reply's evidence for the home chat so `GET /webapp/vnext/chat-history`
/// can reattach it on another device. Same DID derivation as that route:
/// `webapp_canonical_user_did(console_chat_id_for_principal(...))` — the
/// `command_chat_id` here is exactly that id (Some only for home-context commands).
pub(super) async fn persist_vnext_command_turn_evidence(
    state: &Arc<AdminState>,
    command_chat_id: Option<i64>,
    response: &serde_json::Value,
) {
    let Some(chat_id) = command_chat_id else {
        return;
    };
    let Some(assistant) = response.get("assistant_response") else {
        return;
    };
    let visible_answer = assistant
        .get("visible_answer")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    let evidence: Vec<crate::evidence::EvidenceItem> = assistant
        .get("evidence")
        .cloned()
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default();
    let unsupported_numbers: Vec<String> = assistant
        .get("unsupported_numbers")
        .cloned()
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default();
    let turn_id = assistant
        .get("turn_id")
        .and_then(serde_json::Value::as_str)
        .filter(|turn_id| !turn_id.is_empty());
    if visible_answer.trim().is_empty()
        || (turn_id.is_none() && evidence.is_empty() && unsupported_numbers.is_empty())
    {
        return;
    }
    let user_did = webapp_canonical_user_did(chat_id);
    let evidence_key = turn_id
        .map(crate::evidence::turn_key)
        .unwrap_or_else(|| crate::evidence::reply_key(visible_answer));
    if let Err(error) = state
        .memory
        .store_turn_evidence(&user_did, &evidence_key, &evidence, &unsupported_numbers)
        .await
    {
        tracing::warn!(error = %error, "WebApp turn evidence store failed");
    }
}

/// Apply the post-processing the synchronized `/command` success path applies
/// to a raw owner_response: shape it via `vnext_command_response` and, for a
/// home-context command, attach any newly-created work item (diffed against the
/// `prior_work_item_ids` snapshot taken before the turn ran). Producing the
/// FINAL envelope from ONE place lets the foreground wait, a joined request's
/// wait, AND the `/command/status` poll all return byte-identical bodies — so a
/// client cannot observe a different success shape depending on which path
/// delivered the reply.
/// Init-once holder for the pre-turn work-item id snapshot (see
/// `webapp_vnext_command`): built in the background, awaited at finalize.
pub(super) type PriorWorkItemSnapshot = Arc<tokio::sync::OnceCell<HashSet<String>>>;

async fn snapshot_prior_work_item_ids(state: Arc<AdminState>, chat_id: i64) -> HashSet<String> {
    build_unified_inbox_projection(&state, chat_id, &state.settings.display_language())
        .await
        .items
        .iter()
        .map(|item| item.id.clone())
        .collect()
}

async fn finalize_vnext_command_response(
    state: &Arc<AdminState>,
    command_chat_id: Option<i64>,
    prior_work_item_ids: Option<&PriorWorkItemSnapshot>,
    owner_response: serde_json::Value,
) -> Result<serde_json::Value, (StatusCode, String)> {
    let mut response = vnext_command_response(owner_response);
    if let (Some(chat_id), Some(cell)) = (command_chat_id, prior_work_item_ids) {
        let prior_ids = cell
            .get_or_init(|| snapshot_prior_work_item_ids(state.clone(), chat_id))
            .await;
        attach_new_vnext_work_item_after_command(state, chat_id, prior_ids, &mut response).await?;
    }
    // Both the synchronous return and the background (polled) completion pass
    // through here, so evidence is kept for long turns too.
    persist_vnext_command_turn_evidence(state, command_chat_id, &response).await;
    Ok(response)
}

#[derive(Debug, Default, Deserialize)]
pub(super) struct VNextCommandStatusQuery {
    focus_id: Option<String>,
    intent_kind: Option<String>,
    idempotency_key: Option<String>,
}

/// GET /webapp/vnext/command/status — poll for the eventual result of a
/// `/webapp/vnext/command` turn whose foreground singleflight wait already
/// timed out (504). The bg task keeps running and stores its result in the
/// singleflight entry; this lets the web client recover the reply instead of
/// seeing pure silence.
///
/// Auth is identical to `/command` (web-console auth + CSRF): a poll endpoint
/// without auth would be a security regression (lets any caller probe another
/// owner's in-flight results).
///
/// Status codes:
/// - 404 — no singleflight entry (never started, or reaped after TTL).
/// - 202 `{"status":"pending"}` — bg task still running.
/// - 200 — done Ok; the bg task already stored the FINAL response envelope
///   (post-processed identically to the synchronized `/command` success path,
///   including any attached work item), so it is returned verbatim.
/// - error — done Err; returns the stored `(status, message)`.
pub(super) async fn webapp_vnext_command_status(
    State(state): State<Arc<AdminState>>,
    Query(query): Query<VNextCommandStatusQuery>,
    req: Request<axum::body::Body>,
) -> Result<(StatusCode, Json<serde_json::Value>), (StatusCode, String)> {
    let headers = req.headers();
    let principal = extract_web_console_auth(&state, headers).await?;
    require_web_console_csrf_for_principal(headers, &principal)?;

    let focus_id = query
        .focus_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(GENERAL_ASSISTANT_FOCUS_ID);
    let intent_kind = query
        .intent_kind
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("explain");
    let idempotency_key = query
        .idempotency_key
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("");

    let singleflight_key =
        webapp_command_singleflight_key(&principal, focus_id, intent_kind, idempotency_key);

    match webapp_command_singleflight_peek(&singleflight_key) {
        // No entry: never started or reaped after TTL.
        None => Err((
            StatusCode::NOT_FOUND,
            "No such request (it may have expired).".to_string(),
        )),
        // Entry present, bg task still running.
        Some(None) => Ok((
            StatusCode::ACCEPTED,
            Json(serde_json::json!({ "status": "pending" })),
        )),
        // Done, Ok: the bg task already stored the final, fully post-processed
        // envelope (vnext_command_response + any work-item attach), so the body
        // is byte-identical to what the synchronized `/command` path returns.
        // Return it verbatim.
        Some(Some(Ok(final_response))) => Ok((StatusCode::OK, Json(final_response))),
        // Done, Err: surface the stored failure verbatim.
        Some(Some(Err((status, message)))) => Err((status, message)),
    }
}

/// POST /webapp/vnext/work/:item_id/actions — Product-shaped action wrapper.
pub(super) async fn webapp_vnext_work_action(
    State(state): State<Arc<AdminState>>,
    Path(item_id): Path<String>,
    req: Request<axum::body::Body>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    validate_context_focus_id(&item_id)?;
    let (parts, body) = req.into_parts();
    let headers = parts.headers;
    let principal = extract_web_console_auth(&state, &headers).await?;
    require_web_console_csrf_for_principal(&headers, &principal)?;

    let body_bytes = axum::body::to_bytes(body, 8 * 1024).await.map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            "Invalid vNext action body".to_string(),
        )
    })?;
    let request: VNextWorkActionRequest = serde_json::from_slice(&body_bytes).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            "Invalid vNext action JSON".to_string(),
        )
    })?;
    validate_context_schema_version(request.schema_version)?;
    validate_vnext_work_action_request(&request)?;

    let chat_id = console_chat_id_for_principal(&state, &principal)?;
    let projection =
        build_unified_inbox_projection(&state, chat_id, &state.settings.display_language()).await;
    let item = projection
        .items
        .iter()
        .find(|item| item.id == item_id)
        .ok_or_else(|| (StatusCode::NOT_FOUND, "Work item not found".to_string()))?;
    if !vnext_work_action_is_projected_for_item(item, &request.action_id) {
        return Err((
            StatusCode::BAD_REQUEST,
            "Action does not apply to this item".to_string(),
        ));
    }

    let raw_result = dispatch_vnext_work_action(&state, &headers, chat_id, item, &request).await?;
    Ok(Json(vnext_work_action_response(
        item,
        &request.action_id,
        raw_result,
        &state.settings.display_language(),
    )))
}

/// GET /webapp/vnext/status — Read-only owner-language operations status.
pub(super) async fn webapp_vnext_status(
    State(state): State<Arc<AdminState>>,
    req: Request<axum::body::Body>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let principal = extract_web_console_auth(&state, req.headers()).await?;
    let chat_id = console_chat_id_for_principal(&state, &principal)?;
    let projection =
        build_unified_inbox_projection(&state, chat_id, &state.settings.display_language()).await;
    let user_did = webapp_canonical_user_did(chat_id);
    let system_events = vnext_system_events_from_memory(
        &state.memory,
        &user_did,
        &state.settings.display_language(),
    )
    .await;
    let llm_degraded = state
        .orchestrator
        .as_ref()
        .and_then(|o| o.llm_degraded_snapshot());
    let embedder_degraded = state
        .orchestrator
        .as_ref()
        .and_then(|o| o.embedder_degraded_snapshot());
    let status = vnext_status_projection_with_settings_and_events(
        &projection,
        Some(state.settings.as_ref()),
        system_events,
        llm_degraded,
        embedder_degraded,
        &state.settings.display_language(),
    );
    // Runtime rails the console renders next to the degraded banners: the
    // Telegram bot's connectivity (incident 2026-09-15: a revoked token was
    // invisible here for hours) and the last nightly verdict.
    let response = VNextStatusResponse {
        projection: status,
        telegram_rail: vnext_telegram_rail(
            &crate::telegram_health::snapshot(),
            crate::telegram_health::now_unix(),
        ),
        nightly: read_nightly_status(),
    };

    Ok(Json(serde_json::to_value(response).map_err(|err| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Unable to serialize vNext status: {err}"),
        )
    })?))
}

/// GET /webapp/vnext/settings — Read-only owner settings shell projection.
pub(super) async fn webapp_vnext_settings(
    State(state): State<Arc<AdminState>>,
    req: Request<axum::body::Body>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let principal = extract_web_console_auth(&state, req.headers()).await?;
    let chat_id = console_chat_id_for_principal(&state, &principal)?;
    let projection =
        build_unified_inbox_projection(&state, chat_id, &state.settings.display_language()).await;
    let settings = vnext_settings_projection(&principal, &projection, Some(&state.settings));

    Ok(Json(serde_json::to_value(settings).map_err(|err| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Unable to serialize vNext settings: {err}"),
        )
    })?))
}

/// POST /webapp/vnext/presence — Mark the signed-in Web UX as active.
pub(super) async fn webapp_vnext_presence(
    State(state): State<Arc<AdminState>>,
    req: Request<axum::body::Body>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let (parts, body) = req.into_parts();
    let headers = parts.headers;
    let principal = extract_web_console_auth(&state, &headers).await?;
    require_web_console_csrf_for_principal(&headers, &principal)?;
    let chat_id = console_chat_id_for_principal(&state, &principal)?;
    let owner_did = webapp_canonical_user_did(chat_id);
    let body_bytes = axum::body::to_bytes(body, 2 * 1024).await.map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            "Invalid presence request body".to_string(),
        )
    })?;
    let surface = serde_json::from_slice::<serde_json::Value>(&body_bytes)
        .ok()
        .and_then(|value| {
            value
                .get("surface")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_else(|| "web".to_string());

    crate::owner_presence::mark_owner_web_presence(&state.memory, &owner_did, &surface)
        .await
        .map_err(|err| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Unable to record Web presence: {err}"),
            )
        })?;

    Ok(Json(serde_json::json!({
        "schema_version": 1,
        "status": "ok",
        "owner_did": owner_did,
        "ttl_seconds": crate::owner_presence::OWNER_WEB_PRESENCE_TTL_SECS,
    })))
}

/// GET /webapp/vnext/settings/web-push/status — browser notification readiness for the owner device.
pub(super) async fn webapp_vnext_web_push_status(
    State(state): State<Arc<AdminState>>,
    req: Request<axum::body::Body>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let principal = extract_web_console_auth(&state, req.headers()).await?;
    let chat_id = console_chat_id_for_principal(&state, &principal)?;
    let owner_did = webapp_canonical_user_did(chat_id);
    let status = web_push_status_for_owner(
        &state.memory,
        &owner_did,
        &state.settings.display_language(),
    )
    .await;
    Ok(Json(serde_json::to_value(status).map_err(|err| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Unable to serialize Web Push status: {err}"),
        )
    })?))
}

/// POST /webapp/vnext/settings/web-push/subscribe — save this signed-in browser's PushSubscription.
pub(super) async fn webapp_vnext_web_push_subscribe(
    State(state): State<Arc<AdminState>>,
    req: Request<axum::body::Body>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let (parts, body) = req.into_parts();
    let headers = parts.headers;
    let principal = extract_web_console_auth(&state, &headers).await?;
    require_web_console_csrf_for_principal(&headers, &principal)?;
    let chat_id = console_chat_id_for_principal(&state, &principal)?;
    let owner_did = webapp_canonical_user_did(chat_id);
    if web_push_vapid_public_key().is_none() {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "Web Push is not configured on this server.".to_string(),
        ));
    }
    let body_bytes = axum::body::to_bytes(body, 12 * 1024).await.map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            "Invalid Web Push subscription body".to_string(),
        )
    })?;
    let request: VNextWebPushSubscribeRequest =
        serde_json::from_slice(&body_bytes).map_err(|_| {
            (
                StatusCode::BAD_REQUEST,
                "Invalid Web Push subscription JSON".to_string(),
            )
        })?;
    let subscription = web_push_subscription_record_from_request(request)?;
    web_push_store_subscription(&state.memory, &owner_did, subscription).await?;
    let status = web_push_status_for_owner(
        &state.memory,
        &owner_did,
        &state.settings.display_language(),
    )
    .await;
    Ok(Json(serde_json::to_value(status).map_err(|err| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Unable to serialize Web Push status: {err}"),
        )
    })?))
}

/// POST /webapp/vnext/settings/web-push/unsubscribe — remove this signed-in browser's subscription.
pub(super) async fn webapp_vnext_web_push_unsubscribe(
    State(state): State<Arc<AdminState>>,
    req: Request<axum::body::Body>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let (parts, body) = req.into_parts();
    let headers = parts.headers;
    let principal = extract_web_console_auth(&state, &headers).await?;
    require_web_console_csrf_for_principal(&headers, &principal)?;
    let chat_id = console_chat_id_for_principal(&state, &principal)?;
    let owner_did = webapp_canonical_user_did(chat_id);
    let body_bytes = axum::body::to_bytes(body, 4 * 1024).await.map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            "Invalid Web Push unsubscribe body".to_string(),
        )
    })?;
    let request: VNextWebPushUnsubscribeRequest =
        serde_json::from_slice(&body_bytes).map_err(|_| {
            (
                StatusCode::BAD_REQUEST,
                "Invalid Web Push unsubscribe JSON".to_string(),
            )
        })?;
    let endpoint = validate_web_push_endpoint(&request.endpoint)?;
    web_push_remove_subscription(&state.memory, &owner_did, &endpoint).await?;
    let status = web_push_status_for_owner(
        &state.memory,
        &owner_did,
        &state.settings.display_language(),
    )
    .await;
    Ok(Json(serde_json::to_value(status).map_err(|err| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Unable to serialize Web Push status: {err}"),
        )
    })?))
}

/// POST /webapp/vnext/settings/web-push/test — send a receipt-backed test notification to saved devices.
pub(super) async fn webapp_vnext_web_push_test(
    State(state): State<Arc<AdminState>>,
    req: Request<axum::body::Body>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let headers = req.headers();
    let principal = extract_web_console_auth(&state, headers).await?;
    require_web_console_csrf_for_principal(headers, &principal)?;
    let chat_id = console_chat_id_for_principal(&state, &principal)?;
    let owner_did = webapp_canonical_user_did(chat_id);
    let notice = WebPushNotice::new(
        "Aegis test",
        "WebApp notifications are connected on this device.",
        "aegis-web-push-test",
        WEB_PUSH_DEFAULT_URL,
    )?;
    let report = web_push_send_notice_to_owner(&state.memory, &owner_did, notice).await?;
    Ok(Json(serde_json::to_value(report).map_err(|err| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Unable to serialize Web Push test result: {err}"),
        )
    })?))
}

/// POST /webapp/vnext/voice/session — mint ephemeral xAI Realtime client secret
/// for the CEO Voice Agent (browser never sees the real key).
pub(super) async fn webapp_vnext_voice_session(
    State(state): State<Arc<AdminState>>,
    req: Request<axum::body::Body>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let (parts, body) = req.into_parts();
    let headers = parts.headers;
    let principal = extract_web_console_auth(&state, &headers).await?;
    require_web_console_csrf_for_principal(&headers, &principal)?;
    let _chat_id = console_chat_id_for_principal(&state, &principal)?;
    // Empty body is fine; cap at 2KB to match other settings POSTs.
    let _body_bytes = axum::body::to_bytes(body, 2 * 1024).await.map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            "Invalid voice session request body".to_string(),
        )
    })?;

    // Mint boundary: Codex chat has no Realtime channel. Refuse before any
    if !cfg!(feature = "experimental-realtime-voice") {
        return Err((StatusCode::NOT_IMPLEMENTED,
            "Realtime voice is deferred to the next release. Voice notes remain available when STT is configured.".to_string()));
    }

    // Mint boundary: Codex chat has no Realtime channel. Refuse before any
    // bearer lookup so we do not invent a Codex session or fall through to
    // OpenRouter. Unset `llm_provider` (registry seed only) and Grok keep
    // the existing credential cascade below.
    if crate::voice_session::llm_provider_blocks_realtime_voice(
        state
            .settings
            .get_if_stored("llm_provider")
            .as_ref()
            .and_then(|value| value.as_str()),
    ) {
        return Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            crate::voice_session::CODEX_REALTIME_VOICE_UNSUPPORTED.to_string(),
        ));
    }

    // Credential cascade with *mint-level* fallback (not just resolve-level):
    // a still-present but revoked/invalid subscription access token used to
    // short-circuit mint with 403 and never try CLI — the owner-visible
    // "voice agent does not work" path while Maka (CLI token) was fine.
    let candidates = collect_ceo_voice_mint_bearers(&state).await;
    if candidates.is_empty() {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "请先在设置中连接 Grok（订阅），或在本机登录 Grok CLI。口令听写在无 Realtime 时仍可用。 / Connect Grok in Settings (subscription), or log in Grok CLI on this host. Dictation still works without Realtime.".to_string(),
        ));
    }

    let mut last_err = String::from("Voice mint failed");
    for (bearer, source) in candidates {
        match crate::voice_session::mint_realtime_client_secret(
            &bearer,
            crate::voice_session::CLIENT_SECRET_TTL_SECONDS,
            source,
        )
        .await
        {
            Ok(minted) => {
                return Ok(Json(serde_json::json!({
                    "schema_version": 1,
                    "client_secret": minted.client_secret,
                    "expires_in_seconds": minted.expires_in_seconds,
                    "model": minted.model,
                    "ws_url": minted.ws_url,
                    "max_session_seconds": minted.max_session_seconds,
                    "key_source": minted.key_source,
                })));
            }
            Err(err) => {
                tracing::warn!(
                    error = %err,
                    source = %source.as_str(),
                    "CEO voice session mint failed; trying next credential"
                );
                last_err = err;
            }
        }
    }

    Err((StatusCode::BAD_GATEWAY, last_err))
}

/// Ordered mint candidates: subscription (if usable), CLI, paid key, stub.
async fn collect_ceo_voice_mint_bearers(
    state: &AdminState,
) -> Vec<(String, crate::voice_session::VoiceKeySource)> {
    use crate::voice_session::VoiceKeySource;
    let mut out = Vec::new();

    if std::env::var("AEGIS_VOICE_TEST_STUB").ok().as_deref() == Some("1") {
        out.push(("stub".into(), VoiceKeySource::Stub));
        return out;
    }

    // 1) Aegis Grok subscription (device-code OAuth), auto-refresh like avatar path.
    if let Ok(store) = vnext_xai_credential_store(state) {
        match crate::llm::XaiSubscriptionAuthManager::load_credential(&store) {
            Ok(Some(mut credential)) => {
                if credential.needs_refresh(crate::llm::xai_subscription::now_ms()) {
                    let manager = crate::llm::XaiSubscriptionAuthManager::new(Default::default());
                    match manager.refresh_credential(&credential).await {
                        Ok(refreshed) => {
                            credential = refreshed;
                            let _ = crate::llm::XaiSubscriptionAuthManager::store_credential(
                                &store,
                                &credential,
                            );
                        }
                        Err(err) => {
                            tracing::warn!(
                                error = %err.message,
                                "Grok subscription refresh failed for voice mint; will try CLI"
                            );
                        }
                    }
                }
                // Only offer a non-expired access token; expired ones 403 at mint
                // and previously blocked the CLI fallback entirely.
                if crate::voice_session::subscription_access_token_usable(
                    &credential.access_token,
                    credential.expires_at_ms,
                    crate::llm::xai_subscription::now_ms(),
                ) {
                    out.push((
                        credential.access_token.clone(),
                        VoiceKeySource::XaiSubscription,
                    ));
                }
            }
            Ok(None) => {}
            Err(err) => {
                tracing::debug!(error = %err.message, "voice: could not load Grok subscription");
            }
        }
    }

    // 2) Grok CLI / gtok machine login (~/.grok/auth.json) — Hercules path.
    if let Some(token) = crate::voice_session::read_grok_cli_access_token_from_env_home() {
        out.push((token, VoiceKeySource::GrokCli));
    }

    // 3) Paid API key last resort only.
    if let Some(key) = crate::voice_session::resolve_paid_api_key_from_env() {
        out.push((key, VoiceKeySource::ApiKey));
    }

    out
}

/// GET /webapp/vnext/voice/status — whether Realtime mint is configured (no side effects).
pub(super) async fn webapp_vnext_voice_status(
    State(state): State<Arc<AdminState>>,
    req: Request<axum::body::Body>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let principal = extract_web_console_auth(&state, req.headers()).await?;
    let _chat_id = console_chat_id_for_principal(&state, &principal)?;

    if !cfg!(feature = "experimental-realtime-voice") {
        let stt = vnext_stt_backend();
        return Ok(Json(serde_json::json!({
            "schema_version": 1, "stt_configured": stt.is_configured(), "stt_backend": stt.kind(),
            "realtime_available": false, "key_source": null, "subscription_connected": false,
            "subscription_stale": false, "model": null, "max_session_seconds": 0,
            "reason": "deferred_to_next_release",
        })));
    }

    let mut key_source: Option<&'static str> = None;
    let mut subscription_connected = false;
    let mut subscription_stale = false;
    if let Ok(store) = vnext_xai_credential_store(&state)
        && let Ok(Some(cred)) = crate::llm::XaiSubscriptionAuthManager::load_credential(&store)
    {
        subscription_connected = true;
        let now_ms = crate::llm::xai_subscription::now_ms();
        if crate::voice_session::subscription_access_token_usable(
            &cred.access_token,
            cred.expires_at_ms,
            now_ms,
        ) {
            key_source = Some(crate::voice_session::VoiceKeySource::XaiSubscription.as_str());
        } else if !cred.access_token.trim().is_empty() || cred.expires_at_ms.is_some() {
            // Connected in Settings, but access token is expired/empty — mint
            // will try refresh; do not advertise Realtime until a live bearer exists.
            subscription_stale = true;
        }
    }
    if key_source.is_none()
        && crate::voice_session::read_grok_cli_access_token_from_env_home().is_some()
    {
        key_source = Some(crate::voice_session::VoiceKeySource::GrokCli.as_str());
    }
    if key_source.is_none() && crate::voice_session::resolve_paid_api_key_from_env().is_some() {
        key_source = Some(crate::voice_session::VoiceKeySource::ApiKey.as_str());
    }
    if key_source.is_none() && std::env::var("AEGIS_VOICE_TEST_STUB").ok().as_deref() == Some("1") {
        key_source = Some(crate::voice_session::VoiceKeySource::Stub.as_str());
    }

    let configured = key_source.is_some();
    let stt = vnext_stt_backend();
    let stt_configured = stt.is_configured();
    let stt_backend = stt.kind();
    Ok(Json(serde_json::json!({
        // Speech-to-text (voice notes) — distinct from the Realtime voice
        // session below; the deploy smoke keys on `stt_configured`.
        "stt_configured": stt_configured,
        "stt_backend": stt_backend,
        "schema_version": 1,
        "realtime_available": configured,
        "key_source": key_source,
        "subscription_connected": subscription_connected,
        "subscription_stale": subscription_stale,
        "model": crate::voice_session::voice_agent_model_from_env(),
        "max_session_seconds": crate::voice_session::max_session_seconds_from_env(),
    })))
}

/// POST /webapp/vnext/research/stop — stop the rest of the current research job.
pub(super) async fn webapp_vnext_research_stop(
    State(state): State<Arc<AdminState>>,
    req: Request<axum::body::Body>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let headers = req.headers();
    let principal = extract_web_console_auth(&state, headers).await?;
    require_web_console_csrf_for_principal(headers, &principal)?;
    let chat_id = console_chat_id_for_principal(&state, &principal)?;
    let user_did = webapp_canonical_user_did(chat_id);
    state
        .memory
        .store_skill_state(crate::search_turn::RESEARCH_STOP_SKILL, &user_did, "1")
        .await
        .map_err(|error| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Unable to stop research: {error}"),
            )
        })?;
    Ok(Json(serde_json::json!({ "status": "ok" })))
}
