//! Multimodal message processing for Aegis Runtime.
//!
//! Handles media detection, download from Telegram, speech-to-text transcription
//! via OpenAI API, and image encoding for LLM vision.

use anyhow::{Context, Result};
use async_openai::Client as OpenAIClient;
use async_openai::config::OpenAIConfig;
use async_openai::types::InputSource;
use async_openai::types::audio::{AudioInput, CreateTranscriptionRequest};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// A media attachment extracted from a Telegram message.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum MediaAttachment {
    /// Image encoded as base64 data URL for LLM vision.
    Image { data_url: String },
    /// Audio encoded as base64 for direct LLM audio input (no STT step).
    Audio {
        data_b64: String,
        format: String, // "mp3", "wav", "ogg" etc.
        duration_secs: u32,
    },
    /// Transcribed audio/voice via dedicated STT API (fallback path).
    Voice {
        transcript: String,
        duration_secs: u32,
    },
    /// Document (PDF, DOCX, etc.) encoded as base64 data URL for LLM file input.
    Document {
        data_url: String,
        filename: String,
        mime_type: String,
    },
    /// Video encoded as base64 data URL for LLM video input.
    Video {
        data_url: String,
        filename: String,
        duration_secs: u32,
    },
}

/// Result of processing a Telegram message's media content.
#[derive(Debug, Clone)]
pub struct ProcessedMessage {
    /// Text content (original text + any transcriptions).
    pub text: String,
    /// Media attachments for LLM (images as base64 data URLs).
    pub attachments: Vec<MediaAttachment>,
}

fn render_voice_text(companion_text: &str, attachment: &MediaAttachment) -> String {
    match attachment {
        MediaAttachment::Voice { transcript, .. } if !transcript.trim().is_empty() => {
            format!("{companion_text}\n\n[语音转写 / Voice transcript]\n{transcript}")
        }
        _ => companion_text.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Telegram getFile API response
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct TelegramFileResponse {
    ok: bool,
    result: Option<TelegramFile>,
    description: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TelegramFile {
    #[allow(dead_code)]
    file_id: String,
    file_path: Option<String>,
}

// ---------------------------------------------------------------------------
// MIME detection
// ---------------------------------------------------------------------------

/// Detect MIME type from a file path/extension.
fn mime_from_path(path: &str) -> &'static str {
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
        "image/jpeg"
    } else if lower.ends_with(".png") {
        "image/png"
    } else if lower.ends_with(".gif") {
        "image/gif"
    } else if lower.ends_with(".webp") {
        "image/webp"
    } else if lower.ends_with(".ogg") || lower.ends_with(".oga") {
        "audio/ogg"
    } else if lower.ends_with(".mp3") {
        "audio/mpeg"
    } else if lower.ends_with(".m4a") {
        "audio/mp4"
    } else if lower.ends_with(".wav") {
        "audio/wav"
    } else if lower.ends_with(".webm") {
        "audio/webm"
    } else if lower.ends_with(".pdf") {
        "application/pdf"
    } else if lower.ends_with(".docx") {
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
    } else if lower.ends_with(".xlsx") {
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
    } else if lower.ends_with(".pptx") {
        "application/vnd.openxmlformats-officedocument.presentationml.presentation"
    } else if lower.ends_with(".csv") {
        "text/csv"
    } else if lower.ends_with(".txt") {
        "text/plain"
    } else if lower.ends_with(".json") {
        "application/json"
    } else if lower.ends_with(".xml") {
        "application/xml"
    } else if lower.ends_with(".html") || lower.ends_with(".htm") {
        "text/html"
    } else if lower.ends_with(".mp4") {
        "video/mp4"
    } else if lower.ends_with(".mov") {
        "video/quicktime"
    } else if lower.ends_with(".avi") {
        "video/x-msvideo"
    } else if lower.ends_with(".mkv") {
        "video/x-matroska"
    } else {
        "application/octet-stream"
    }
}

/// Detect audio format string for OpenAI input_audio from file extension.
/// Returns format names compatible with `InputAudioFormat` (mp3, wav, etc.).
fn audio_format_from_path(path: &str) -> &'static str {
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".mp3") {
        "mp3"
    } else if lower.ends_with(".wav") {
        "wav"
    } else if lower.ends_with(".ogg") || lower.ends_with(".oga") {
        "ogg"
    } else if lower.ends_with(".m4a") || lower.ends_with(".mp4") {
        "m4a"
    } else if lower.ends_with(".webm") {
        "webm"
    } else if lower.ends_with(".flac") {
        "flac"
    } else {
        // Telegram voice messages are .oga (Opus in OGG), default to mp3
        // since most LLMs accept it
        "mp3"
    }
}

// ---------------------------------------------------------------------------
// Base64 encoding (standard alphabet, with padding)
// ---------------------------------------------------------------------------

const B64_CHARS: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn base64_encode(input: &[u8]) -> String {
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    let mut i = 0;
    while i + 2 < input.len() {
        let n = ((input[i] as u32) << 16) | ((input[i + 1] as u32) << 8) | (input[i + 2] as u32);
        out.push(B64_CHARS[((n >> 18) & 63) as usize] as char);
        out.push(B64_CHARS[((n >> 12) & 63) as usize] as char);
        out.push(B64_CHARS[((n >> 6) & 63) as usize] as char);
        out.push(B64_CHARS[(n & 63) as usize] as char);
        i += 3;
    }
    let remaining = input.len() - i;
    if remaining == 2 {
        let n = ((input[i] as u32) << 16) | ((input[i + 1] as u32) << 8);
        out.push(B64_CHARS[((n >> 18) & 63) as usize] as char);
        out.push(B64_CHARS[((n >> 12) & 63) as usize] as char);
        out.push(B64_CHARS[((n >> 6) & 63) as usize] as char);
        out.push('=');
    } else if remaining == 1 {
        let n = (input[i] as u32) << 16;
        out.push(B64_CHARS[((n >> 18) & 63) as usize] as char);
        out.push(B64_CHARS[((n >> 12) & 63) as usize] as char);
        out.push('=');
        out.push('=');
    }
    out
}

// ---------------------------------------------------------------------------
// MediaProcessor
// ---------------------------------------------------------------------------

/// Processes multimodal Telegram messages: images, voice, audio.
pub struct MediaProcessor {
    stt: SttBackend,
    http: reqwest::Client,
    bot_token: String,
}

const DEFAULT_STT_BASE_URL: &str = "https://api.openai.com/v1";
const DEFAULT_STT_MODEL: &str = "gpt-4o-mini-transcribe";
const IMAGE_SIZE_WARN_BYTES: usize = 5 * 1024 * 1024;

/// Speech-to-text backend. Local whisper.cpp keeps voice notes on the host
/// (no API traffic); the OpenAI-compatible path is the paid fallback.
pub enum SttBackend {
    Disabled,
    LocalWhisper {
        bin: std::path::PathBuf,
        model: std::path::PathBuf,
        language: String,
        /// Smaller model used only for language detection when `language`
        /// is `auto` (whisper.cpp runs a full encoder pass to detect; with
        /// `small` that doubled transcription time on 2026-09-05).
        detect_model: Option<std::path::PathBuf>,
    },
    OpenAiCompat {
        client: OpenAIClient<OpenAIConfig>,
        model: String,
    },
}

impl SttBackend {
    /// Environment: `AEGIS_STT_LOCAL_BIN` + `AEGIS_STT_LOCAL_MODEL` (+ optional
    /// `AEGIS_STT_LOCAL_LANGUAGE`, default `auto`) select local whisper.cpp;
    /// otherwise `AEGIS_STT_API_KEY` / `AEGIS_STT_BASE_URL` / `AEGIS_STT_MODEL`.
    pub fn from_env() -> Self {
        let env = |k: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty());
        Self::from_values(
            env("AEGIS_STT_LOCAL_BIN").as_deref(),
            env("AEGIS_STT_LOCAL_MODEL").as_deref(),
            env("AEGIS_STT_LOCAL_LANGUAGE").as_deref(),
            env("AEGIS_STT_LOCAL_DETECT_MODEL").as_deref(),
            &env("AEGIS_STT_API_KEY").unwrap_or_default(),
            &env("AEGIS_STT_BASE_URL").unwrap_or_else(|| DEFAULT_STT_BASE_URL.to_string()),
            &env("AEGIS_STT_MODEL").unwrap_or_else(|| DEFAULT_STT_MODEL.to_string()),
        )
    }

    pub fn from_values(
        local_bin: Option<&str>,
        local_model: Option<&str>,
        local_language: Option<&str>,
        local_detect_model: Option<&str>,
        api_key: &str,
        base_url: &str,
        model: &str,
    ) -> Self {
        if let (Some(bin), Some(model_path)) = (local_bin, local_model) {
            let model = std::path::PathBuf::from(model_path);
            let detect_model = local_detect_model
                .map(str::trim)
                .filter(|p| !p.is_empty())
                .map(std::path::PathBuf::from)
                .or_else(|| {
                    // Default: a sibling `ggml-base.bin` next to the main model.
                    let sibling = model.with_file_name("ggml-base.bin");
                    sibling.exists().then_some(sibling)
                });
            return Self::LocalWhisper {
                bin: std::path::PathBuf::from(bin),
                model,
                language: local_language
                    .map(str::trim)
                    .filter(|l| !l.is_empty())
                    .unwrap_or("auto")
                    .to_string(),
                detect_model,
            };
        }
        if api_key.is_empty() {
            return Self::Disabled;
        }
        Self::OpenAiCompat {
            client: crate::llm_client::create_llm_client(api_key, base_url),
            model: model.to_string(),
        }
    }

    pub fn is_configured(&self) -> bool {
        !matches!(self, Self::Disabled)
    }

    pub fn kind(&self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::LocalWhisper { .. } => "local_whisper",
            Self::OpenAiCompat { .. } => "openai_compat",
        }
    }

    pub fn language(&self) -> &str {
        match self {
            Self::LocalWhisper { language, .. } => language,
            _ => "",
        }
    }
}

/// whisper.cpp prints bracketed markers for non-speech segments; drop them
/// and join the remaining lines into one line.
pub fn clean_whisper_transcript(raw: &str) -> String {
    raw.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter(|line| {
            !((line.starts_with('[') && line.ends_with(']'))
                || (line.starts_with('(') && line.ends_with(')')))
        })
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string()
}

/// Transcode any Telegram audio to 16 kHz mono WAV with ffmpeg, then run
/// `whisper-cli` on it. Blocking; call from `spawn_blocking`.
/// whisper.cpp arguments for the transcription pass: greedy decoding
/// (`-bs 1 -bo 1`) halves latency versus the default beam search with no
/// visible quality loss on short voice notes (measured 2026-09-05: 16.8 s →
/// 7.8 s for an 11 s clip on 4 cores).
/// whisper.cpp always encodes a 30 s window (1500 audio-context frames), so
/// a 3 s note costs the same as a 30 s one. Shrinking `-ac` to the clip's
/// length plus margin cuts the encoder cost proportionally (measured
/// 2026-09-06 on a 3 s clip: 8.3 s → 2.4 s at `-ac 256`).
pub fn whisper_audio_ctx_for(duration_secs: f32) -> u32 {
    if !duration_secs.is_finite() || duration_secs <= 0.0 {
        return 1500;
    }
    let frames = (duration_secs / 30.0 * 1500.0).ceil() as u32 + 128;
    frames.clamp(256, 1500)
}

/// Duration of a 16 kHz mono 16-bit PCM wav from its byte length.
pub fn wav16k_mono_duration_secs(bytes: usize) -> f32 {
    bytes.saturating_sub(44) as f32 / 32_000.0
}

/// Wall-clock cap for one whisper.cpp process. A 2.5 s note transcribes in
/// 6–8 s on this host and a 10 s note in ~17 s under load; a looping
/// hallucination (2026-09-13: 1165 chars from a 4 s clip, 305 s) never
/// finishes usefully, so it is killed and reported instead.
pub fn whisper_deadline_for(audio_secs: f32) -> std::time::Duration {
    let secs = 45.0 + audio_secs.max(0.0) * 4.0;
    std::time::Duration::from_secs_f32(secs.min(240.0))
}

/// Run a command with a wall-clock deadline: the child is killed on timeout
/// (blocking; call from `spawn_blocking`).
pub(crate) fn run_with_deadline(
    cmd: &mut std::process::Command,
    deadline: std::time::Duration,
    label: &str,
) -> anyhow::Result<std::process::Output> {
    use std::io::Read;
    use std::process::Stdio;
    let mut child = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("spawning {label}"))?;
    let mut out_pipe = child.stdout.take();
    let mut err_pipe = child.stderr.take();
    let out_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(p) = out_pipe.as_mut() {
            let _ = p.read_to_end(&mut buf);
        }
        buf
    });
    let err_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(p) = err_pipe.as_mut() {
            let _ = p.read_to_end(&mut buf);
        }
        buf
    });
    let started = std::time::Instant::now();
    let status = loop {
        if let Some(status) = child
            .try_wait()
            .with_context(|| format!("waiting for {label}"))?
        {
            break status;
        }
        if started.elapsed() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            anyhow::bail!(
                "{label} exceeded its {}s deadline and was killed",
                deadline.as_secs()
            );
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    };
    let stdout = out_thread.join().unwrap_or_default();
    let stderr = err_thread.join().unwrap_or_default();
    Ok(std::process::Output {
        status,
        stdout,
        stderr,
    })
}

/// Serializes local whisper runs (see `transcribe_audio_with`).
static LOCAL_WHISPER_SLOT: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(1);

/// Rough duration from the compressed upload size (Opus/AAC voice notes run
/// 4–16 kB/s). Only used to bound the transcript length; generous on purpose.
pub fn probe_free_duration_estimate_secs(compressed_bytes: usize) -> f32 {
    (compressed_bytes as f32 / 4_000.0).max(3.0)
}

/// Speech tops out near 8 characters (CJK) or 4 words per second; a transcript
/// far beyond that for its duration is whisper repeating itself.
pub fn transcript_is_implausibly_long(transcript: &str, approx_secs: f32) -> bool {
    let chars = transcript.chars().count() as f32;
    chars > 80.0 + approx_secs * 12.0
}

pub fn whisper_transcribe_args(language: &str, audio_ctx: u32) -> Vec<String> {
    vec![
        // No text context carried between segments: the classic whisper
        // repetition loop feeds on its own previous output.
        "-mc".into(),
        "0".into(),
        "-ac".into(),
        audio_ctx.to_string(),
        "-l".into(),
        language.into(),
        "-nt".into(),
        "-np".into(),
        "-t".into(),
        "4".into(),
        "-bs".into(),
        "1".into(),
        "-bo".into(),
        "1".into(),
    ]
}

/// Parse whisper.cpp's `-dl` (detect-language-only) output, e.g.
/// `whisper_full_with_state: auto-detected language: zh (p = 0.97)`.
pub fn parse_detected_language(output: &str) -> Option<String> {
    let idx = output.find("auto-detected language:")?;
    let rest = &output[idx + "auto-detected language:".len()..];
    let code: String = rest
        .trim_start()
        .chars()
        .take_while(|c| c.is_ascii_alphabetic())
        .collect();
    (!code.is_empty() && code.len() <= 3).then_some(code.to_ascii_lowercase())
}

pub fn run_local_whisper(
    bin: &std::path::Path,
    model: &std::path::Path,
    language: &str,
    detect_model: Option<&std::path::Path>,
    audio_bytes: &[u8],
    file_name: &str,
) -> Result<String> {
    use std::process::{Command, Stdio};
    let dir = std::env::temp_dir().join(format!(
        "aegis-stt-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).context("creating stt temp dir")?;
    let ext = std::path::Path::new(file_name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("ogg");
    // Distinct basenames: a `.wav` upload used to collide with the converted
    // file ("Output … same as Input", found by the live e2e on 2026-09-05).
    let input = dir.join(format!("source.{ext}"));
    let wav = dir.join("converted.wav");
    std::fs::write(&input, audio_bytes).context("writing stt input")?;
    let result = (|| -> Result<String> {
        let stage_started = std::time::Instant::now();
        let ff = Command::new("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
            .arg(&input)
            .args(["-vn", "-ac", "1", "-ar", "16000", "-f", "wav"])
            .arg(&wav)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output()
            .context("spawning ffmpeg for stt (is ffmpeg installed?)")?;
        if !ff.status.success() {
            anyhow::bail!(
                "ffmpeg failed to convert voice note to wav: {}",
                String::from_utf8_lossy(&ff.stderr).trim()
            );
        }
        // Two-pass: a cheap language detection with the small detect model
        // (base: ~2.8 s vs ~13.5 s with `small` for an 11 s clip), then one
        // greedy transcription pass with the language fixed.
        let wav_len = std::fs::metadata(&wav)
            .map(|m| m.len() as usize)
            .unwrap_or(0);
        let audio_secs = wav16k_mono_duration_secs(wav_len);
        let audio_ctx = whisper_audio_ctx_for(audio_secs);
        tracing::info!(
            audio_secs = format!("{audio_secs:.1}"),
            audio_ctx,
            ffmpeg_ms = stage_started.elapsed().as_millis() as u64,
            "local whisper input ready"
        );
        let mut language = language.to_string();
        if language == "auto"
            && let Some(detect) = detect_model
        {
            let started = std::time::Instant::now();
            let mut det_cmd = Command::new(bin);
            det_cmd.arg("-m").arg(detect).arg("-f").arg(&wav).args([
                "-dl",
                "-t",
                "4",
                "-ac",
                &audio_ctx.to_string(),
            ]);
            let det = run_with_deadline(
                &mut det_cmd,
                whisper_deadline_for(audio_secs),
                "local whisper (detect)",
            )?;
            let text = format!(
                "{}\n{}",
                String::from_utf8_lossy(&det.stderr),
                String::from_utf8_lossy(&det.stdout)
            );
            match parse_detected_language(&text) {
                Some(code) => {
                    tracing::info!(language = %code, elapsed_ms = started.elapsed().as_millis() as u64, "local whisper detected language");
                    language = code;
                }
                None => tracing::warn!(
                    "local whisper language detection produced no result; transcribing with auto"
                ),
            }
        }
        let transcribe_started = std::time::Instant::now();
        let mut cmd = Command::new(bin);
        cmd.arg("-m")
            .arg(model)
            .arg("-f")
            .arg(&wav)
            .args(whisper_transcribe_args(&language, audio_ctx));
        let out = run_with_deadline(
            &mut cmd,
            whisper_deadline_for(audio_secs),
            "local whisper (transcribe)",
        )?;
        if !out.status.success() {
            anyhow::bail!(
                "local whisper failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        tracing::info!(
            language = %language,
            transcribe_ms = transcribe_started.elapsed().as_millis() as u64,
            "local whisper transcription pass finished"
        );
        Ok(clean_whisper_transcript(&String::from_utf8_lossy(
            &out.stdout,
        )))
    })();
    let _ = std::fs::remove_dir_all(&dir);
    result
}

/// Probe an audio blob's duration in seconds with `ffprobe`.
///
/// Blocking (spawns a process); call from `spawn_blocking`. Returns `None` for
/// anything that does not probe cleanly — a missing `ffprobe`, an unparseable
/// container, or a non-finite duration. Duration is decorative metadata on a
/// voice note, so a probe failure must never fail the upload.
pub fn probe_audio_duration_secs(audio_bytes: &[u8], file_name: &str) -> Option<f32> {
    use std::process::{Command, Stdio};
    let dir = std::env::temp_dir().join(format!(
        "aegis-probe-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).ok()?;
    let ext = std::path::Path::new(file_name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("bin");
    let input = dir.join(format!("probe.{ext}"));
    let probed = std::fs::write(&input, audio_bytes).ok().and_then(|()| {
        let out = Command::new("ffprobe")
            .args([
                "-v",
                "error",
                "-show_entries",
                "format=duration",
                "-of",
                "default=noprint_wrappers=1:nokey=1",
            ])
            .arg(&input)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        String::from_utf8_lossy(&out.stdout)
            .trim()
            .parse::<f32>()
            .ok()
            .filter(|secs| secs.is_finite() && *secs >= 0.0)
    });
    let _ = std::fs::remove_dir_all(&dir);
    probed
}

/// Transcribe audio bytes with an explicit [`SttBackend`].
///
/// Shared by the Telegram `MediaProcessor` and the WebApp voice-note upload
/// route so the local-whisper / OpenAI-compatible dispatch lives in exactly one
/// place. Errors when the backend is `Disabled` — callers map that onto their
/// own "speech-to-text is not configured" surface.
pub async fn transcribe_audio_with(
    stt: &SttBackend,
    audio_bytes: Vec<u8>,
    file_name: &str,
) -> Result<String> {
    info!(
        file_name,
        bytes = audio_bytes.len(),
        backend = stt.kind(),
        "starting STT transcription"
    );
    let text = match stt {
        SttBackend::Disabled => anyhow::bail!("speech-to-text is not configured"),
        SttBackend::LocalWhisper {
            bin,
            model,
            language,
            detect_model,
        } => {
            let (bin, model, language, detect_model, name) = (
                bin.clone(),
                model.clone(),
                language.clone(),
                detect_model.clone(),
                file_name.to_string(),
            );
            // One whisper at a time: each pass loads a model and asks for all
            // four cores; three notes in flight together would push the
            // service past its memory cap.
            let _permit = LOCAL_WHISPER_SLOT
                .acquire()
                .await
                .context("local whisper slot closed")?;
            let audio_len = audio_bytes.len();
            let transcript = tokio::task::spawn_blocking(move || {
                run_local_whisper(
                    &bin,
                    &model,
                    &language,
                    detect_model.as_deref(),
                    &audio_bytes,
                    &name,
                )
            })
            .await
            .context("local whisper task panicked")??;
            // Runaway output (repetition loop) is not a transcript.
            let approx_secs = probe_free_duration_estimate_secs(audio_len);
            if transcript_is_implausibly_long(&transcript, approx_secs) {
                anyhow::bail!(
                    "local whisper produced {} characters for ~{:.0} s of audio — likely a repetition loop, not speech",
                    transcript.chars().count(),
                    approx_secs
                );
            }
            transcript
        }
        SttBackend::OpenAiCompat { client, model } => {
            let request = CreateTranscriptionRequest {
                file: AudioInput {
                    source: InputSource::VecU8 {
                        filename: file_name.to_string(),
                        vec: audio_bytes,
                    },
                },
                model: model.clone(),
                ..Default::default()
            };
            let response = client
                .audio()
                .transcription()
                .create(request)
                .await
                .context("STT transcription API call failed")?;
            response.text.trim().to_string()
        }
    };
    info!(transcript_len = text.len(), "STT transcription complete");
    Ok(text)
}

impl MediaProcessor {
    /// Create a new `MediaProcessor`.
    ///
    /// STT configuration is read from environment variables:
    /// - `AEGIS_STT_API_KEY` — required for transcription (STT fails gracefully if absent)
    /// - `AEGIS_STT_BASE_URL` — defaults to `https://api.openai.com/v1`
    /// - `AEGIS_STT_MODEL` — defaults to `gpt-4o-mini-transcribe`
    pub fn new(bot_token: &str) -> Self {
        let stt = SttBackend::from_env();
        match &stt {
            SttBackend::Disabled => tracing::warn!(
                "speech-to-text disabled: set AEGIS_STT_LOCAL_BIN + AEGIS_STT_LOCAL_MODEL (local whisper) or AEGIS_STT_API_KEY"
            ),
            other => info!(
                backend = other.kind(),
                language = other.language(),
                "speech-to-text configured"
            ),
        }
        Self {
            stt,
            http: reqwest::Client::new(),
            bot_token: bot_token.to_string(),
        }
    }

    /// Whether STT (speech-to-text) is configured and available for transcription.
    pub fn is_stt_configured(&self) -> bool {
        self.stt.is_configured()
    }

    // -----------------------------------------------------------------------
    // Telegram file download
    // -----------------------------------------------------------------------

    /// Download a file from Telegram by `file_id`.
    ///
    /// 1. Calls `getFile` to resolve the file path.
    /// 2. Downloads the raw bytes from Telegram's file storage.
    ///
    /// Returns `(bytes, file_path)` where `file_path` can be used for extension detection.
    async fn download_telegram_file(&self, file_id: &str) -> Result<(Vec<u8>, String)> {
        // Step 1: resolve file_id → file_path
        let url = format!(
            "https://api.telegram.org/bot{}/getFile?file_id={}",
            self.bot_token, file_id
        );
        let resp: TelegramFileResponse = self
            .http
            .get(&url)
            .send()
            .await
            .context("getFile request failed")?
            .json()
            .await
            .context("getFile response parse failed")?;

        if !resp.ok {
            anyhow::bail!(
                "Telegram getFile failed: {}",
                resp.description.unwrap_or_else(|| "unknown error".into())
            );
        }
        let file_path = resp
            .result
            .and_then(|f| f.file_path)
            .context("Telegram getFile returned no file_path")?;

        // Step 2: download raw bytes
        let download_url = format!(
            "https://api.telegram.org/file/bot{}/{}",
            self.bot_token, file_path
        );
        let bytes = self
            .http
            .get(&download_url)
            .send()
            .await
            .context("file download request failed")?
            .bytes()
            .await
            .context("file download body read failed")?
            .to_vec();

        info!(
            file_id,
            file_path,
            size_bytes = bytes.len(),
            "downloaded telegram file"
        );

        Ok((bytes, file_path))
    }

    // -----------------------------------------------------------------------
    // Speech-to-Text
    // -----------------------------------------------------------------------

    /// Transcribe audio bytes with this processor's configured STT backend.
    /// Thin delegation to [`transcribe_audio_with`] — the WebApp voice-note
    /// upload route calls that same function with its own backend.
    pub async fn transcribe_audio(&self, audio_bytes: Vec<u8>, file_name: &str) -> Result<String> {
        transcribe_audio_with(&self.stt, audio_bytes, file_name).await
    }

    // -----------------------------------------------------------------------
    // Image encoding
    // -----------------------------------------------------------------------

    /// Encode image bytes as a base64 data URL suitable for LLM vision APIs.
    fn encode_image_data_url(bytes: &[u8], mime: &str) -> String {
        let encoded = base64_encode(bytes);
        format!("data:{mime};base64,{encoded}")
    }

    // -----------------------------------------------------------------------
    // High-level processors
    // -----------------------------------------------------------------------

    /// Process a voice/audio message: download and encode as base64 for direct LLM input.
    ///
    /// The audio is sent directly to the LLM (via `input_audio` content part),
    /// which understands the speech and responds with text. No separate STT step.
    pub async fn process_voice(&self, file_id: &str, duration: u32) -> Result<MediaAttachment> {
        let (bytes, file_path) = self.download_telegram_file(file_id).await?;

        // Detect audio format from file extension; Telegram voice notes are
        // ogg/opus, which chat audio parts cannot carry, so transcode to mp3.
        let source_format = audio_format_from_path(&file_path);
        let (bytes, format) =
            tokio::task::spawn_blocking(move || prepare_audio_for_llm(&bytes, source_format))
                .await
                .context("voice transcoding task panicked")??;
        let data_b64 = base64_encode(&bytes);

        info!(
            file_path,
            format,
            duration_secs = duration,
            audio_bytes = bytes.len(),
            b64_len = data_b64.len(),
            "encoded audio for direct LLM input"
        );

        Ok(MediaAttachment::Audio {
            data_b64,
            format: format.to_string(),
            duration_secs: duration,
        })
    }

    /// Fallback: process voice via dedicated STT transcription (if configured).
    /// Only used when the LLM model doesn't support audio input.
    pub async fn process_voice_stt(&self, file_id: &str, duration: u32) -> Result<MediaAttachment> {
        let (bytes, file_path) = self.download_telegram_file(file_id).await?;

        let file_name = file_path
            .rsplit('/')
            .next()
            .unwrap_or("voice.ogg")
            .to_string();

        let transcript = self.transcribe_audio(bytes, &file_name).await?;

        Ok(MediaAttachment::Voice {
            transcript,
            duration_secs: duration,
        })
    }

    /// Process a photo: download from Telegram and encode as base64 data URL.
    pub async fn process_photo(&self, file_id: &str) -> Result<MediaAttachment> {
        let (bytes, file_path) = self.download_telegram_file(file_id).await?;

        if bytes.len() > IMAGE_SIZE_WARN_BYTES {
            warn!(
                size_bytes = bytes.len(),
                file_path, "image exceeds 5 MB — processing anyway"
            );
        }

        let mime = mime_from_path(&file_path);
        let data_url = Self::encode_image_data_url(&bytes, mime);

        Ok(MediaAttachment::Image { data_url })
    }

    /// Process a document file (PDF, DOCX, etc.): download and encode as base64 data URL.
    pub async fn process_document(
        &self,
        file_id: &str,
        mime_type: &str,
        filename: Option<&str>,
    ) -> Result<MediaAttachment> {
        let (bytes, file_path) = self.download_telegram_file(file_id).await?;

        let actual_mime = if mime_type == "application/octet-stream" || mime_type.is_empty() {
            mime_from_path(&file_path)
        } else {
            mime_type
        };
        let actual_filename =
            filename.unwrap_or_else(|| file_path.rsplit('/').next().unwrap_or("document"));

        let encoded = base64_encode(&bytes);
        let data_url = format!("data:{actual_mime};base64,{encoded}");

        info!(
            file_path,
            mime = actual_mime,
            filename = actual_filename,
            size_bytes = bytes.len(),
            "encoded document for LLM file input"
        );

        Ok(MediaAttachment::Document {
            data_url,
            filename: actual_filename.to_string(),
            mime_type: actual_mime.to_string(),
        })
    }

    /// Process a video message: download and encode as base64 data URL.
    pub async fn process_video(
        &self,
        file_id: &str,
        duration: u32,
        filename: Option<&str>,
    ) -> Result<MediaAttachment> {
        let (bytes, file_path) = self.download_telegram_file(file_id).await?;

        let mime = mime_from_path(&file_path);
        let actual_filename =
            filename.unwrap_or_else(|| file_path.rsplit('/').next().unwrap_or("video.mp4"));

        let encoded = base64_encode(&bytes);
        let data_url = format!("data:{mime};base64,{encoded}");

        info!(
            file_path,
            mime,
            duration_secs = duration,
            size_bytes = bytes.len(),
            "encoded video for LLM input"
        );

        Ok(MediaAttachment::Video {
            data_url,
            filename: actual_filename.to_string(),
            duration_secs: duration,
        })
    }

    /// Process a complete Telegram message with potential media.
    ///
    /// This is the main entry point called from `telegram.rs`.
    ///
    /// Precedence:
    /// - Voice/audio takes priority (transcription replaces text).
    /// - Photo produces an image attachment; caption (or fallback) becomes text.
    /// - Video produces a video attachment.
    /// - Documents: image → vision, PDF/DOCX/etc → file input, others → text notice.
    /// - Text-only messages pass through unchanged.
    pub async fn process_telegram_message(
        &self,
        text: Option<&str>,
        voice_file_id: Option<(&str, u32)>,
        photo_file_id: Option<&str>,
        document: Option<(&str, Option<&str>, Option<&str>)>, // (file_id, mime_type, filename)
        video_file_id: Option<(&str, u32, Option<&str>)>,     // (file_id, duration, filename)
        model_supports_audio: bool,
    ) -> Result<ProcessedMessage> {
        // --- Voice message ---
        // If the active LLM model supports audio input, send raw audio for native understanding.
        // Otherwise, use STT transcription fallback so voice still works with text-only models.
        if let Some((file_id, duration)) = voice_file_id {
            let result = if model_supports_audio {
                match self.process_voice(file_id, duration).await {
                    Ok(attachment) => Ok(attachment),
                    Err(e) if self.stt.is_configured() => {
                        warn!(
                            error = %format!("{e:#}"),
                            "native voice processing unavailable; falling back to STT"
                        );
                        self.process_voice_stt(file_id, duration).await
                    }
                    Err(e) => Err(e),
                }
            } else {
                self.process_voice_stt(file_id, duration).await
            };
            match result {
                Ok(attachment) => {
                    let companion_text = text.unwrap_or("[用户发送了一段语音]");
                    let final_text = render_voice_text(companion_text, &attachment);
                    return Ok(ProcessedMessage {
                        text: final_text,
                        attachments: vec![attachment],
                    });
                }
                Err(e) => {
                    warn!(error = %format!("{e:#}"), "voice audio processing failed");
                    return Ok(ProcessedMessage {
                        text: "[语音处理失败，请重新发送]".to_string(),
                        attachments: vec![],
                    });
                }
            }
        }

        // --- Photo ---
        if let Some(file_id) = photo_file_id {
            let caption = text.unwrap_or("[用户发送了一张图片]");
            match self.process_photo(file_id).await {
                Ok(attachment) => {
                    return Ok(ProcessedMessage {
                        text: caption.to_string(),
                        attachments: vec![attachment],
                    });
                }
                Err(e) => {
                    warn!(error = %format!("{e:#}"), "photo processing failed");
                    return Ok(ProcessedMessage {
                        text: caption.to_string(),
                        attachments: vec![],
                    });
                }
            }
        }

        // --- Video ---
        if let Some((file_id, duration, filename)) = video_file_id {
            let caption = text.unwrap_or("[用户发送了一段视频]");
            match self.process_video(file_id, duration, filename).await {
                Ok(attachment) => {
                    return Ok(ProcessedMessage {
                        text: caption.to_string(),
                        attachments: vec![attachment],
                    });
                }
                Err(e) => {
                    warn!(error = %format!("{e:#}"), "video processing failed");
                    return Ok(ProcessedMessage {
                        text: format!("{caption}\n\n[视频处理失败: {e:#}]"),
                        attachments: vec![],
                    });
                }
            }
        }

        // --- Document ---
        if let Some((file_id, mime, filename)) = document {
            let mime_str = mime.unwrap_or("application/octet-stream");
            let is_image = mime_str.starts_with("image/");

            if is_image {
                // Image documents → vision path (same as photos)
                let caption = text.unwrap_or("[用户发送了一张图片文件]");
                match self.process_photo(file_id).await {
                    Ok(attachment) => {
                        return Ok(ProcessedMessage {
                            text: caption.to_string(),
                            attachments: vec![attachment],
                        });
                    }
                    Err(e) => {
                        warn!(error = %format!("{e:#}"), "document image processing failed");
                        return Ok(ProcessedMessage {
                            text: caption.to_string(),
                            attachments: vec![],
                        });
                    }
                }
            }

            // PDF, DOCX, TXT, CSV, etc. → file input via OpenRouter
            let is_supported_doc = matches!(
                mime_str,
                "application/pdf"
                    | "text/plain"
                    | "text/csv"
                    | "text/html"
                    | "application/json"
                    | "application/xml"
                    | "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
                    | "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
                    | "application/vnd.openxmlformats-officedocument.presentationml.presentation"
            ) || mime_str.starts_with("text/");

            if is_supported_doc {
                let caption = text.unwrap_or("[用户发送了一份文档]");
                match self.process_document(file_id, mime_str, filename).await {
                    Ok(attachment) => {
                        return Ok(ProcessedMessage {
                            text: caption.to_string(),
                            attachments: vec![attachment],
                        });
                    }
                    Err(e) => {
                        warn!(error = %format!("{e:#}"), "document processing failed");
                        return Ok(ProcessedMessage {
                            text: format!("{caption}\n\n[文档处理失败: {e:#}]"),
                            attachments: vec![],
                        });
                    }
                }
            }

            // Unsupported file type → inform LLM
            let caption = text.unwrap_or("");
            let doc_text = if caption.is_empty() {
                format!("[用户发送了一个文件 (类型: {mime_str})，当前不支持直接处理此文件类型]")
            } else {
                format!("{caption}\n\n[附件: 文件类型 {mime_str}，当前不支持直接处理此文件类型]")
            };
            return Ok(ProcessedMessage {
                text: doc_text,
                attachments: vec![],
            });
        }

        // --- Text only ---
        Ok(ProcessedMessage {
            text: text.unwrap_or_default().to_string(),
            attachments: vec![],
        })
    }
}

// ---------------------------------------------------------------------------
// Base64 decoding (standard alphabet)
// ---------------------------------------------------------------------------

/// Decode a standard base64 string to bytes.
pub fn decode_base64_standard(input: &str) -> Result<Vec<u8>> {
    let input = input.trim();
    let mut out = Vec::with_capacity(input.len() * 3 / 4);

    let table = |c: u8| -> Result<u8> {
        match c {
            b'A'..=b'Z' => Ok(c - b'A'),
            b'a'..=b'z' => Ok(c - b'a' + 26),
            b'0'..=b'9' => Ok(c - b'0' + 52),
            b'+' => Ok(62),
            b'/' => Ok(63),
            b'=' => Ok(0), // padding
            _ => anyhow::bail!("invalid base64 character: {c}"),
        }
    };

    let bytes: Vec<u8> = input
        .bytes()
        .filter(|&b| b != b'\n' && b != b'\r' && b != b' ')
        .collect();
    if !bytes.len().is_multiple_of(4) && !bytes.is_empty() {
        anyhow::bail!("invalid base64 length: {}", bytes.len());
    }

    for chunk in bytes.chunks(4) {
        if chunk.len() < 4 {
            break;
        }
        let a = table(chunk[0])?;
        let b = table(chunk[1])?;
        let c = table(chunk[2])?;
        let d = table(chunk[3])?;

        out.push((a << 2) | (b >> 4));
        if chunk[2] != b'=' {
            out.push((b << 4) | (c >> 2));
        }
        if chunk[3] != b'=' {
            out.push((c << 6) | d);
        }
    }

    Ok(out)
}

// ---------------------------------------------------------------------------
// TTS (Text-to-Speech) via xAI API
// ---------------------------------------------------------------------------

const DEFAULT_TTS_BASE_URL: &str = "https://api.x.ai/v1";
const DEFAULT_TTS_VOICE: &str = "eve";
const TTS_MAX_CHARS: usize = 15_000;

/// Text-to-Speech synthesizer using xAI TTS API.
pub struct TtsSynthesizer {
    http: reqwest::Client,
    api_key: String,
    base_url: String,
    voice: String,
}

impl TtsSynthesizer {
    /// Create a new TTS synthesizer.
    ///
    /// Configuration via environment variables:
    /// - `AEGIS_TTS_API_KEY` — required (xAI API key)
    /// - `AEGIS_TTS_BASE_URL` — defaults to `https://api.x.ai/v1`
    /// - `AEGIS_TTS_VOICE` — defaults to `eve` (options: eve, ara, rex, sal, leo)
    pub fn new() -> Self {
        Self {
            http: reqwest::Client::new(),
            api_key: std::env::var("AEGIS_TTS_API_KEY").unwrap_or_else(|_| {
                tracing::warn!("AEGIS_TTS_API_KEY not set — text-to-speech disabled");
                String::new()
            }),
            base_url: std::env::var("AEGIS_TTS_BASE_URL")
                .unwrap_or_else(|_| DEFAULT_TTS_BASE_URL.to_string()),
            voice: std::env::var("AEGIS_TTS_VOICE")
                .unwrap_or_else(|_| DEFAULT_TTS_VOICE.to_string()),
        }
    }

    /// Returns true if TTS is configured (API key is set).
    pub fn is_configured(&self) -> bool {
        !self.api_key.is_empty()
    }

    /// Synthesize text to audio bytes (MP3).
    ///
    /// `language`: BCP-47 code (e.g. "zh", "en") or "auto".
    pub async fn synthesize(&self, text: &str, language: Option<&str>) -> Result<Vec<u8>> {
        if self.api_key.is_empty() {
            anyhow::bail!("TTS not configured: AEGIS_TTS_API_KEY is not set");
        }

        let truncated = if text.len() > TTS_MAX_CHARS {
            warn!(
                original_len = text.len(),
                "TTS text exceeds 15K chars, truncating"
            );
            &text[..text
                .char_indices()
                .take_while(|(i, _)| *i < TTS_MAX_CHARS)
                .last()
                .map(|(i, c)| i + c.len_utf8())
                .unwrap_or(0)]
        } else {
            text
        };

        let body = serde_json::json!({
            "text": truncated,
            "voice_id": self.voice,
            "language": language.unwrap_or("auto"),
            "output_format": {
                "codec": "mp3",
                "sample_rate": 24000,
                "bit_rate": 128000,
            }
        });

        info!(
            text_len = truncated.len(),
            voice = %self.voice,
            "starting TTS synthesis"
        );

        let resp = self
            .http
            .post(format!("{}/tts", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .context("TTS request failed")?;

        let status = resp.status();
        if !status.is_success() {
            let err_body = resp.text().await.unwrap_or_default();
            anyhow::bail!("TTS API returned {status}: {err_body}");
        }

        let bytes = resp
            .bytes()
            .await
            .context("TTS response read failed")?
            .to_vec();
        info!(audio_bytes = bytes.len(), "TTS synthesis complete");
        Ok(bytes)
    }
}

// ---------------------------------------------------------------------------
// Image Generation via xAI Aurora API
// ---------------------------------------------------------------------------

const DEFAULT_IMAGEGEN_BASE_URL: &str = "https://api.x.ai/v1";
const DEFAULT_IMAGEGEN_MODEL: &str = "grok-imagine-image-2.0";

/// Image generator using the xAI Imagine image API (`grok-imagine-image-2.0`).
pub struct ImageGenerator {
    http: reqwest::Client,
    api_key: String,
    base_url: String,
    model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneratedImage {
    /// Base64-encoded image data (JPEG/PNG).
    pub b64_data: String,
    /// Revised prompt (if model modified it).
    pub revised_prompt: Option<String>,
}

impl ImageGenerator {
    /// Create a new image generator.
    ///
    /// Configuration via environment variables:
    /// - `AEGIS_IMAGEGEN_API_KEY` — required (xAI API key)
    /// - `AEGIS_IMAGEGEN_BASE_URL` — defaults to `https://api.x.ai/v1`
    /// - `AEGIS_IMAGEGEN_MODEL` — defaults to `grok-imagine-image-2.0`
    pub fn new() -> Self {
        Self {
            http: reqwest::Client::new(),
            api_key: std::env::var("AEGIS_IMAGEGEN_API_KEY").unwrap_or_else(|_| {
                tracing::warn!("AEGIS_IMAGEGEN_API_KEY not set — image generation disabled");
                String::new()
            }),
            base_url: std::env::var("AEGIS_IMAGEGEN_BASE_URL")
                .unwrap_or_else(|_| DEFAULT_IMAGEGEN_BASE_URL.to_string()),
            model: std::env::var("AEGIS_IMAGEGEN_MODEL")
                .unwrap_or_else(|_| DEFAULT_IMAGEGEN_MODEL.to_string()),
        }
    }

    /// Returns true if image generation is configured (API key is set).
    pub fn is_configured(&self) -> bool {
        !self.api_key.is_empty()
    }

    /// Generate an image from a text prompt.
    ///
    /// `aspect_ratio`: e.g. "1:1", "16:9", "9:16" (default "1:1")
    pub async fn generate(
        &self,
        prompt: &str,
        aspect_ratio: Option<&str>,
    ) -> Result<GeneratedImage> {
        if self.api_key.is_empty() {
            anyhow::bail!("Image generation not configured: AEGIS_IMAGEGEN_API_KEY is not set");
        }

        let body = serde_json::json!({
            "model": self.model,
            "prompt": prompt,
            "n": 1,
            "response_format": "b64_json",
            "aspect_ratio": aspect_ratio.unwrap_or("1:1"),
        });

        info!(
            prompt_len = prompt.len(),
            model = %self.model,
            "starting image generation"
        );

        let resp = self
            .http
            .post(format!("{}/images/generations", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .context("image generation request failed")?;

        let status = resp.status();
        if !status.is_success() {
            let err_body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Image generation API returned {status}: {err_body}");
        }

        let resp_json: serde_json::Value = resp
            .json()
            .await
            .context("image generation response parse failed")?;

        let data = resp_json["data"]
            .as_array()
            .and_then(|arr| arr.first())
            .context("image generation returned no data")?;

        let b64_data = data["b64_json"]
            .as_str()
            .context("image generation returned no b64_json")?
            .to_string();

        let revised_prompt = data["revised_prompt"].as_str().map(|s| s.to_string());

        info!(b64_len = b64_data.len(), "image generation complete");

        Ok(GeneratedImage {
            b64_data,
            revised_prompt,
        })
    }
}

// ===========================================================================
// Tests
// ===========================================================================

/// Prepare raw Telegram audio for native LLM `input_audio`.
///
/// Chat audio content parts accept only mp3/wav, while Telegram voice notes
/// are ogg/opus, so anything else is transcoded to mp3 with the host `ffmpeg`
/// (stdin -> stdout, no temp files). Errors if ffmpeg is missing or fails,
/// which lets the caller fall back to STT.
pub fn prepare_audio_for_llm(bytes: &[u8], format: &str) -> Result<(Vec<u8>, &'static str)> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    if crate::model_capabilities::is_chat_input_audio_format_supported(format) {
        let canonical = if format == "wav" { "wav" } else { "mp3" };
        return Ok((bytes.to_vec(), canonical));
    }
    let mut child = Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-i",
            "pipe:0",
            "-vn",
            "-ac",
            "1",
            "-ar",
            "24000",
            "-codec:a",
            "libmp3lame",
            "-b:a",
            "64k",
            "-f",
            "mp3",
            "pipe:1",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("spawning ffmpeg for voice transcoding (is ffmpeg installed?)")?;
    let mut stdin = child.stdin.take().context("ffmpeg stdin")?;
    let input = bytes.to_vec();
    let writer = std::thread::spawn(move || stdin.write_all(&input));
    let output = child.wait_with_output().context("waiting for ffmpeg")?;
    let _ = writer.join();
    if !output.status.success() {
        anyhow::bail!(
            "ffmpeg failed to transcode {format} to mp3: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    if output.stdout.is_empty() {
        anyhow::bail!("ffmpeg produced no mp3 output for {format} input");
    }
    Ok((output.stdout, "mp3"))
}

#[cfg(test)]
mod tests {
    /// Regression: a `.wav` voice note must not make ffmpeg's output path
    /// equal its input path. Uses a fake whisper binary so the test only
    /// needs ffmpeg.
    #[test]
    fn run_local_whisper_accepts_wav_input() {
        if std::process::Command::new("ffmpeg")
            .arg("-version")
            .output()
            .is_err()
        {
            eprintln!("ffmpeg not installed; skipping");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let fake_bin = dir.path().join("whisper-cli");
        std::fs::write(&fake_bin, "#!/bin/sh\necho ' hello from fake whisper'\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&fake_bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        // 0.1 s of 16 kHz mono silence as a valid RIFF/WAVE file.
        let samples: u32 = 1600;
        let data_len = samples * 2;
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + data_len).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&16000u32.to_le_bytes());
        wav.extend_from_slice(&32000u32.to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&data_len.to_le_bytes());
        wav.extend(std::iter::repeat_n(0u8, data_len as usize));
        let text = super::run_local_whisper(
            &fake_bin,
            std::path::Path::new("/nonexistent/model.bin"),
            "en",
            None,
            &wav,
            "voice-note.wav",
        )
        .expect("wav input must convert and transcribe");
        assert!(text.contains("hello from fake whisper"), "{text}");
    }

    #[test]
    fn detected_language_is_parsed_from_whisper_output() {
        assert_eq!(
            super::parse_detected_language(
                "whisper_init...\nwhisper_full_with_state: auto-detected language: zh (p = 0.973)\n"
            )
            .as_deref(),
            Some("zh")
        );
        assert_eq!(
            super::parse_detected_language("auto-detected language: EN (p = 0.9)").as_deref(),
            Some("en")
        );
        assert_eq!(super::parse_detected_language("no detection here"), None);
        assert_eq!(
            super::parse_detected_language("auto-detected language: (p=0)"),
            None
        );
    }

    #[test]
    fn audio_ctx_scales_with_clip_length() {
        assert_eq!(
            super::whisper_audio_ctx_for(0.0),
            1500,
            "unknown length: full window"
        );
        assert_eq!(super::whisper_audio_ctx_for(3.0), 278);
        assert_eq!(super::whisper_audio_ctx_for(11.0), 678);
        assert_eq!(
            super::whisper_audio_ctx_for(29.0),
            1500,
            "clamped to the full window"
        );
        assert_eq!(super::whisper_audio_ctx_for(120.0), 1500);
        assert!((super::wav16k_mono_duration_secs(44 + 96_000) - 3.0).abs() < 1e-6);
    }

    #[test]
    fn transcribe_args_use_greedy_decoding_and_fixed_language() {
        let args = super::whisper_transcribe_args("zh", 278);
        // No carried text context (repetition-loop guard), then the audio window.
        assert_eq!(args[0..6], ["-mc", "0", "-ac", "278", "-l", "zh"]);
        assert!(args.windows(2).any(|w| w == ["-bs", "1"]), "{args:?}");
        assert!(args.windows(2).any(|w| w == ["-bo", "1"]), "{args:?}");
        assert!(args.windows(2).any(|w| w == ["-t", "4"]), "{args:?}");
    }

    #[test]
    fn whisper_deadline_scales_with_audio_and_kills_a_runaway_process() {
        assert_eq!(super::whisper_deadline_for(0.0).as_secs(), 45);
        assert_eq!(super::whisper_deadline_for(10.0).as_secs(), 85);
        assert_eq!(super::whisper_deadline_for(1000.0).as_secs(), 240);
        let mut sleep = std::process::Command::new("sleep");
        sleep.arg("30");
        let started = std::time::Instant::now();
        let err =
            super::run_with_deadline(&mut sleep, std::time::Duration::from_millis(300), "sleep")
                .expect_err("a process past its deadline must be killed");
        assert!(err.to_string().contains("deadline"), "{err}");
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
        let mut echo = std::process::Command::new("echo");
        echo.arg("hi");
        let out =
            super::run_with_deadline(&mut echo, std::time::Duration::from_secs(5), "echo").unwrap();
        assert!(out.status.success());
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "hi");
    }

    #[test]
    fn runaway_transcripts_are_rejected() {
        // 2026-09-13: 1165 characters from a 4 s clip (looped English, language
        // pinned to zh) reached the orchestrator as the owner's words.
        let looped = "and so my fellow americans ".repeat(45);
        assert!(super::transcript_is_implausibly_long(&looped, 4.4));
        assert!(!super::transcript_is_implausibly_long(
            "恢复第36项其他两项可以删除",
            4.4
        ));
        assert!(!super::transcript_is_implausibly_long(
            &"字".repeat(200),
            20.0
        ));
        assert!(super::probe_free_duration_estimate_secs(328_811) > 60.0);
        assert_eq!(super::probe_free_duration_estimate_secs(0), 3.0);
    }

    #[test]
    fn local_whisper_defaults_detect_model_to_sibling_base_when_present() {
        let dir = tempfile::tempdir().unwrap();
        let model = dir.path().join("ggml-small.bin");
        std::fs::write(&model, b"x").unwrap();
        let without = super::SttBackend::from_values(
            Some("/usr/bin/true"),
            Some(model.to_str().unwrap()),
            None,
            None,
            "",
            super::DEFAULT_STT_BASE_URL,
            super::DEFAULT_STT_MODEL,
        );
        assert!(matches!(
            &without,
            super::SttBackend::LocalWhisper {
                detect_model: None,
                ..
            }
        ));
        std::fs::write(dir.path().join("ggml-base.bin"), b"y").unwrap();
        let with = super::SttBackend::from_values(
            Some("/usr/bin/true"),
            Some(model.to_str().unwrap()),
            None,
            None,
            "",
            super::DEFAULT_STT_BASE_URL,
            super::DEFAULT_STT_MODEL,
        );
        assert!(
            matches!(&with, super::SttBackend::LocalWhisper { detect_model: Some(p), .. } if p.ends_with("ggml-base.bin"))
        );
        let explicit = super::SttBackend::from_values(
            Some("/usr/bin/true"),
            Some(model.to_str().unwrap()),
            None,
            Some("/x/ggml-tiny.bin"),
            "",
            super::DEFAULT_STT_BASE_URL,
            super::DEFAULT_STT_MODEL,
        );
        assert!(
            matches!(&explicit, super::SttBackend::LocalWhisper { detect_model: Some(p), .. } if p.ends_with("ggml-tiny.bin"))
        );
    }

    fn ffmpeg_available() -> bool {
        std::process::Command::new("ffmpeg")
            .arg("-version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    #[test]
    fn ogg_voice_is_transcoded_to_mp3_for_native_audio_input() {
        if !ffmpeg_available() {
            eprintln!("skipping: ffmpeg not on PATH");
            return;
        }
        let dir = std::env::temp_dir().join(format!("aegis-voice-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let ogg_path = dir.join("voice.ogg");
        let status = std::process::Command::new("ffmpeg")
            .args([
                "-y",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:duration=0.5",
                "-c:a",
                "libopus",
            ])
            .arg(&ogg_path)
            .status()
            .unwrap();
        assert!(
            status.success(),
            "test fixture: ffmpeg could not synthesize an ogg/opus clip"
        );
        let ogg = std::fs::read(&ogg_path).unwrap();
        assert!(ogg.starts_with(b"OggS"));

        let (bytes, format) =
            prepare_audio_for_llm(&ogg, "ogg").expect("ogg must be transcoded, not rejected");
        assert_eq!(format, "mp3");
        assert!(bytes.len() > 200, "mp3 output too small: {}", bytes.len());
        let is_mp3 = bytes.starts_with(b"ID3") || (bytes[0] == 0xFF && (bytes[1] & 0xE0) == 0xE0);
        assert!(is_mp3, "output is not an mp3 stream");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn stt_backend_prefers_local_whisper_when_binary_and_model_are_set() {
        let backend = SttBackend::from_values(
            Some("/opt/whisper/whisper-cli"),
            Some("/opt/whisper/ggml-small.bin"),
            None,
            None,
            "",
            DEFAULT_STT_BASE_URL,
            DEFAULT_STT_MODEL,
        );
        assert!(backend.is_configured());
        assert_eq!(backend.kind(), "local_whisper");
        assert_eq!(backend.language(), "auto");

        let api = SttBackend::from_values(
            None,
            None,
            None,
            None,
            "sk-x",
            DEFAULT_STT_BASE_URL,
            DEFAULT_STT_MODEL,
        );
        assert!(api.is_configured());
        assert_eq!(api.kind(), "openai_compat");

        let off = SttBackend::from_values(
            None,
            None,
            None,
            None,
            "",
            DEFAULT_STT_BASE_URL,
            DEFAULT_STT_MODEL,
        );
        assert!(!off.is_configured());
        assert_eq!(off.kind(), "disabled");
    }

    #[test]
    fn whisper_transcript_cleanup_drops_markers_and_joins_lines() {
        assert_eq!(clean_whisper_transcript("[BLANK_AUDIO]\n"), "");
        assert_eq!(
            clean_whisper_transcript(" 你好，今天天气不错。\n[BLANK_AUDIO]\n 我们下午三点见。 \n"),
            "你好，今天天气不错。 我们下午三点见。"
        );
        assert_eq!(
            clean_whisper_transcript("(music)\n[MUSIC]\nhello there"),
            "hello there"
        );
    }

    #[test]
    fn local_whisper_runs_on_synthesized_audio_when_installed() {
        let bin = std::env::var("AEGIS_STT_LOCAL_BIN").unwrap_or_else(|_| {
            format!(
                "{}/.local/share/aegis-runtime/whisper/whisper.cpp/build/bin/whisper-cli",
                std::env::var("HOME").unwrap_or_default()
            )
        });
        let model = std::env::var("AEGIS_STT_LOCAL_MODEL").unwrap_or_else(|_| {
            format!(
                "{}/.local/share/aegis-runtime/whisper/ggml-small.bin",
                std::env::var("HOME").unwrap_or_default()
            )
        });
        if !std::path::Path::new(&bin).exists()
            || !std::path::Path::new(&model).exists()
            || !ffmpeg_available()
        {
            eprintln!("skipping: whisper-cli / model / ffmpeg not available");
            return;
        }
        let dir = std::env::temp_dir().join(format!("aegis-whisper-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let ogg_path = dir.join("tone.ogg");
        let status = std::process::Command::new("ffmpeg")
            .args([
                "-y",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:duration=1.0",
                "-c:a",
                "libopus",
            ])
            .arg(&ogg_path)
            .status()
            .unwrap();
        assert!(status.success());
        let ogg = std::fs::read(&ogg_path).unwrap();
        let text = run_local_whisper(
            std::path::Path::new(&bin),
            std::path::Path::new(&model),
            "auto",
            None,
            &ogg,
            "tone.ogg",
        )
        .expect("local whisper must run end to end (ffmpeg -> wav -> whisper-cli)");
        assert!(
            text.len() < 200,
            "a 1s tone must not produce a long transcript: {text:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn mp3_and_wav_voice_pass_through_untouched() {
        let (bytes, format) = prepare_audio_for_llm(b"ID3fake", "mp3").unwrap();
        assert_eq!((bytes.as_slice(), format), (&b"ID3fake"[..], "mp3"));
        let (bytes, format) = prepare_audio_for_llm(b"RIFFfake", "wav").unwrap();
        assert_eq!((bytes.as_slice(), format), (&b"RIFFfake"[..], "wav"));
    }

    use super::*;

    #[test]
    fn test_base64_encode_decode_roundtrip() {
        let inputs: &[&[u8]] = &[
            b"",
            b"a",
            b"ab",
            b"abc",
            b"hello world",
            b"\x00\x01\x02\xff",
        ];
        for input in inputs {
            let encoded = base64_encode(input);
            let decoded = decode_base64_standard(&encoded).unwrap();
            assert_eq!(&decoded, input, "roundtrip failed for len={}", input.len());
        }
    }

    #[test]
    fn test_decode_base64_standard_known() {
        assert_eq!(
            decode_base64_standard("aGVsbG8gd29ybGQ=").unwrap(),
            b"hello world"
        );
        assert_eq!(decode_base64_standard("").unwrap(), b"");
        assert_eq!(decode_base64_standard("YQ==").unwrap(), b"a");
        assert_eq!(decode_base64_standard("YWI=").unwrap(), b"ab");
    }

    #[test]
    fn test_decode_base64_standard_invalid() {
        assert!(decode_base64_standard("!!!").is_err());
    }

    #[test]
    fn test_encode_image_data_url() {
        let bytes = b"hello world";
        let url = MediaProcessor::encode_image_data_url(bytes, "image/jpeg");
        assert!(url.starts_with("data:image/jpeg;base64,"));

        // Verify the base64 portion is correct (standard encoding of "hello world")
        let expected_b64 = "aGVsbG8gd29ybGQ=";
        assert_eq!(url, format!("data:image/jpeg;base64,{expected_b64}"));
    }

    #[test]
    fn test_encode_image_data_url_png() {
        let bytes = &[0x89, 0x50, 0x4E, 0x47]; // PNG magic bytes
        let url = MediaProcessor::encode_image_data_url(bytes, "image/png");
        assert!(url.starts_with("data:image/png;base64,"));
        // 4 bytes → ceil(4/3)*4 = 8 base64 chars
        let b64_part = url.strip_prefix("data:image/png;base64,").unwrap();
        assert_eq!(b64_part.len(), 8); // 4 bytes → 8 chars with padding
    }

    #[test]
    fn test_encode_image_data_url_empty() {
        let url = MediaProcessor::encode_image_data_url(&[], "image/jpeg");
        assert_eq!(url, "data:image/jpeg;base64,");
    }

    #[test]
    fn test_base64_encode_roundtrip_lengths() {
        // Verify padding for all remainder classes (0, 1, 2 mod 3)
        for len in 0..=6 {
            let input: Vec<u8> = (0..len).collect();
            let encoded = base64_encode(&input);
            // base64 output is always a multiple of 4
            assert_eq!(encoded.len() % 4, 0, "len={len}");
        }
    }

    #[test]
    fn test_mime_from_extension() {
        assert_eq!(mime_from_path("photos/IMG_001.jpg"), "image/jpeg");
        assert_eq!(mime_from_path("photos/IMG_001.JPEG"), "image/jpeg");
        assert_eq!(mime_from_path("screenshot.png"), "image/png");
        assert_eq!(mime_from_path("anim.gif"), "image/gif");
        assert_eq!(mime_from_path("sticker.webp"), "image/webp");
        assert_eq!(mime_from_path("voice.ogg"), "audio/ogg");
        assert_eq!(mime_from_path("voice.oga"), "audio/ogg");
        assert_eq!(mime_from_path("song.mp3"), "audio/mpeg");
        assert_eq!(mime_from_path("memo.m4a"), "audio/mp4");
        assert_eq!(mime_from_path("clip.wav"), "audio/wav");
        assert_eq!(mime_from_path("rec.webm"), "audio/webm");
        assert_eq!(mime_from_path("data.bin"), "application/octet-stream");
        assert_eq!(mime_from_path("noext"), "application/octet-stream");
    }

    #[test]
    fn test_process_text_only() {
        // process_telegram_message is async, but text-only path doesn't need runtime
        let rt = tokio::runtime::Runtime::new().unwrap();
        let _guard = rt.enter();

        // We can't call the real method without a valid bot token hitting the network,
        // so test the ProcessedMessage construction directly.
        let msg = ProcessedMessage {
            text: "Hello, world!".to_string(),
            attachments: vec![],
        };
        assert_eq!(msg.text, "Hello, world!");
        assert!(msg.attachments.is_empty());
    }

    #[test]
    fn test_processed_message_audio_direct() {
        let attachment = MediaAttachment::Audio {
            data_b64: "SGVsbG8=".to_string(),
            format: "mp3".to_string(),
            duration_secs: 5,
        };

        let msg = ProcessedMessage {
            text: "[用户发送了一段语音]".to_string(),
            attachments: vec![attachment],
        };

        assert_eq!(msg.attachments.len(), 1);
        match &msg.attachments[0] {
            MediaAttachment::Audio {
                data_b64,
                format,
                duration_secs,
            } => {
                assert_eq!(data_b64, "SGVsbG8=");
                assert_eq!(format, "mp3");
                assert_eq!(*duration_secs, 5);
            }
            _ => panic!("expected Audio attachment"),
        }
    }

    #[test]
    fn test_processed_message_voice_stt_fallback() {
        // STT fallback path still works
        let attachment = MediaAttachment::Voice {
            transcript: "你好，世界".to_string(),
            duration_secs: 3,
        };
        let msg = ProcessedMessage {
            text: "[语音消息转写] 你好，世界".to_string(),
            attachments: vec![attachment],
        };
        assert!(msg.text.contains("你好，世界"));
    }

    #[test]
    fn render_voice_text_includes_stt_transcript() {
        let attachment = MediaAttachment::Voice {
            transcript: "turn on the lights".to_string(),
            duration_secs: 2,
        };
        let text = render_voice_text("[用户发送了一段语音]", &attachment);
        assert!(text.contains("Voice transcript"));
        assert!(text.contains("turn on the lights"));
    }

    #[test]
    fn test_audio_format_from_path() {
        assert_eq!(audio_format_from_path("voice.ogg"), "ogg");
        assert_eq!(audio_format_from_path("voice.oga"), "ogg");
        assert_eq!(audio_format_from_path("song.mp3"), "mp3");
        assert_eq!(audio_format_from_path("memo.m4a"), "m4a");
        assert_eq!(audio_format_from_path("clip.wav"), "wav");
        assert_eq!(audio_format_from_path("rec.webm"), "webm");
        assert_eq!(audio_format_from_path("music.flac"), "flac");
        assert_eq!(audio_format_from_path("unknown.xyz"), "mp3"); // fallback
    }

    #[test]
    fn test_processed_message_photo_with_caption() {
        let msg = ProcessedMessage {
            text: "Look at this!".to_string(),
            attachments: vec![MediaAttachment::Image {
                data_url: "data:image/jpeg;base64,/9j/4AAQ".to_string(),
            }],
        };

        assert_eq!(msg.text, "Look at this!");
        assert_eq!(msg.attachments.len(), 1);
        match &msg.attachments[0] {
            MediaAttachment::Image { data_url } => {
                assert!(data_url.starts_with("data:image/jpeg;base64,"));
            }
            _ => panic!("expected Image attachment"),
        }
    }

    #[test]
    fn test_processed_message_photo_no_caption() {
        let msg = ProcessedMessage {
            text: "[用户发送了一张图片]".to_string(),
            attachments: vec![MediaAttachment::Image {
                data_url: "data:image/png;base64,iVBOR".to_string(),
            }],
        };
        assert_eq!(msg.text, "[用户发送了一张图片]");
    }

    #[test]
    fn test_tts_synthesizer_unconfigured() {
        let tts = TtsSynthesizer {
            http: reqwest::Client::new(),
            api_key: String::new(),
            base_url: DEFAULT_TTS_BASE_URL.to_string(),
            voice: "eve".to_string(),
        };
        assert!(!tts.is_configured());
    }

    #[test]
    fn test_tts_synthesizer_configured() {
        let tts = TtsSynthesizer {
            http: reqwest::Client::new(),
            api_key: "xai-test-key".to_string(),
            base_url: DEFAULT_TTS_BASE_URL.to_string(),
            voice: "ara".to_string(),
        };
        assert!(tts.is_configured());
    }

    #[test]
    fn test_imagegen_unconfigured() {
        let img_gen = ImageGenerator {
            http: reqwest::Client::new(),
            api_key: String::new(),
            base_url: DEFAULT_IMAGEGEN_BASE_URL.to_string(),
            model: DEFAULT_IMAGEGEN_MODEL.to_string(),
        };
        assert!(!img_gen.is_configured());
    }

    #[test]
    fn test_imagegen_configured() {
        let img_gen = ImageGenerator {
            http: reqwest::Client::new(),
            api_key: "xai-test-key".to_string(),
            base_url: DEFAULT_IMAGEGEN_BASE_URL.to_string(),
            model: DEFAULT_IMAGEGEN_MODEL.to_string(),
        };
        assert!(img_gen.is_configured());
    }

    #[test]
    fn test_generated_image_serde() {
        let img = GeneratedImage {
            b64_data: "iVBORw0KGgo=".to_string(),
            revised_prompt: Some("A beautiful sunset".to_string()),
        };
        let json = serde_json::to_string(&img).unwrap();
        let back: GeneratedImage = serde_json::from_str(&json).unwrap();
        assert_eq!(back.b64_data, "iVBORw0KGgo=");
        assert_eq!(back.revised_prompt.as_deref(), Some("A beautiful sunset"));
    }

    #[test]
    fn test_generated_image_no_revised_prompt() {
        let img = GeneratedImage {
            b64_data: "abc123".to_string(),
            revised_prompt: None,
        };
        let json = serde_json::to_string(&img).unwrap();
        let back: GeneratedImage = serde_json::from_str(&json).unwrap();
        assert!(back.revised_prompt.is_none());
    }

    #[test]
    fn test_media_attachment_serde_roundtrip() {
        let image = MediaAttachment::Image {
            data_url: "data:image/jpeg;base64,abc".to_string(),
        };
        let json = serde_json::to_string(&image).unwrap();
        assert!(json.contains(r#""type":"Image"#));
        let back: MediaAttachment = serde_json::from_str(&json).unwrap();
        match back {
            MediaAttachment::Image { data_url } => {
                assert_eq!(data_url, "data:image/jpeg;base64,abc");
            }
            _ => panic!("wrong variant"),
        }

        let voice = MediaAttachment::Voice {
            transcript: "hello".to_string(),
            duration_secs: 3,
        };
        let json = serde_json::to_string(&voice).unwrap();
        assert!(json.contains(r#""type":"Voice"#));
        let back: MediaAttachment = serde_json::from_str(&json).unwrap();
        match back {
            MediaAttachment::Voice {
                transcript,
                duration_secs,
            } => {
                assert_eq!(transcript, "hello");
                assert_eq!(duration_secs, 3);
            }
            _ => panic!("wrong variant"),
        }

        let audio = MediaAttachment::Audio {
            data_b64: "AAAA".to_string(),
            format: "ogg".to_string(),
            duration_secs: 10,
        };
        let json = serde_json::to_string(&audio).unwrap();
        assert!(json.contains(r#""type":"Audio"#));
        let back: MediaAttachment = serde_json::from_str(&json).unwrap();
        match back {
            MediaAttachment::Audio {
                data_b64,
                format,
                duration_secs,
            } => {
                assert_eq!(data_b64, "AAAA");
                assert_eq!(format, "ogg");
                assert_eq!(duration_secs, 10);
            }
            _ => panic!("wrong variant"),
        }
    }
}
