//! Headless Chrome page-fetch pseudo-tool (`browser_fetch`).
//!
//! Renders one http(s) page in the machine's real Chrome and returns readable
//! text or a screenshot. Multi-step browsing (logins, clicks) stays on
//! `run_external`.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use anyhow::Result;
use async_openai::types::chat::ChatCompletionMessageToolCall;
use base64::Engine as _;
use scraper::{Html, Selector};
use serde_json::{Value, json};

use crate::orchestrator::{Orchestrator, ToolContext};
use crate::outbound_privacy::{privacy_note, redact_outbound_text};
use crate::sandbox_network::{is_private_ip, is_private_ip_addr};
use crate::snapshot::html_to_aria_markdown;
use crate::utils::truncate_chars;
use crate::webapp_assets::{WEBAPP_GENERATED_IMAGE_MAX_BYTES, store_generated_image_for_owner};

const CHROME_CANDIDATES: &[&str] = &[
    "/opt/google/chrome/chrome",
    "/usr/bin/google-chrome",
    "/usr/bin/google-chrome-stable",
    "/usr/bin/chromium",
    "/usr/bin/chromium-browser",
];

const DEFAULT_WAIT_MS: u64 = 1_500;
const MAX_WAIT_MS: u64 = 10_000;
const DEFAULT_MAX_CHARS: u64 = 6_000;
const MIN_MAX_CHARS: u64 = 500;
const MAX_MAX_CHARS: u64 = 20_000;
const RUN_TIMEOUT: Duration = Duration::from_secs(45);

const CHROME_FLAGS: &[&str] = &[
    "--headless=new",
    "--disable-gpu",
    "--hide-scrollbars",
    "--no-first-run",
    "--no-default-browser-check",
    "--disable-dev-shm-usage",
    "--disable-extensions",
    "--mute-audio",
    "--window-size=1280,2000",
];

/// Resolve the Chrome binary: `AEGIS_CHROME_BIN` if set and present, else the
/// first existing well-known path.
pub(crate) fn chrome_binary() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("AEGIS_CHROME_BIN") {
        let trimmed = path.trim();
        if !trimmed.is_empty() {
            let candidate = PathBuf::from(trimmed);
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }
    CHROME_CANDIDATES
        .iter()
        .map(|p| PathBuf::from(*p))
        .find(|p| p.exists())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BrowserFetchError {
    pub kind: &'static str,
    pub message: String,
    pub privacy_note: Option<String>,
}

impl BrowserFetchError {
    fn new(kind: &'static str, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            privacy_note: None,
        }
    }

    fn privacy(hits: &[&'static str]) -> Self {
        Self {
            kind: "privacy",
            message: "URL contains private details that must not be sent to the network".into(),
            privacy_note: Some(privacy_note(hits)),
        }
    }

    fn to_json(&self) -> String {
        if let Some(note) = &self.privacy_note {
            json!({
                "status": "error",
                "error_kind": self.kind,
                "message": self.message,
                "privacy_note": note,
            })
            .to_string()
        } else {
            json!({
                "status": "error",
                "error_kind": self.kind,
                "message": self.message,
            })
            .to_string()
        }
    }
}

/// Static URL checks (scheme, host, literal private IP, privacy). DNS
/// rebinding is applied separately in [`ensure_public_dns`] before Chrome runs.
pub(crate) fn validate_browser_url(url: &str) -> Result<url::Url, BrowserFetchError> {
    let parsed = url::Url::parse(url.trim())
        .map_err(|_| BrowserFetchError::new("bad_url", "url must be a valid http(s) address"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(BrowserFetchError::new(
            "bad_url",
            "url scheme must be http or https",
        ));
    }
    let Some(host) = parsed.host_str().map(str::trim).filter(|h| !h.is_empty()) else {
        return Err(BrowserFetchError::new(
            "bad_url",
            "url host must not be empty",
        ));
    };
    let host_l = host.to_ascii_lowercase();
    if host_l == "localhost"
        || host_l.ends_with(".localhost")
        || host_l.ends_with(".local")
        || host_l.ends_with(".internal")
        || is_private_ip(host)
    {
        return Err(BrowserFetchError::new(
            "blocked_private",
            "host is not a public internet address",
        ));
    }

    let redaction = redact_outbound_text(url);
    if !redaction.hits.is_empty() {
        return Err(BrowserFetchError::privacy(&redaction.hits));
    }
    Ok(parsed)
}

/// Resolve the host and return its public addresses (private ones dropped).
/// Errors when nothing public is left, so a private-only host never reaches Chrome.
async fn ensure_public_dns(parsed: &url::Url) -> Result<Vec<std::net::IpAddr>, BrowserFetchError> {
    let host = parsed
        .host_str()
        .ok_or_else(|| BrowserFetchError::new("bad_url", "url host must not be empty"))?;
    let port = parsed
        .port_or_known_default()
        .unwrap_or_else(|| if parsed.scheme() == "https" { 443 } else { 80 });
    let addrs = tokio::net::lookup_host((host, port)).await.map_err(|_| {
        BrowserFetchError::new(
            "blocked_private",
            "host failed to resolve to a public address",
        )
    })?;
    let public: Vec<std::net::IpAddr> = addrs
        .map(|a| a.ip())
        .filter(|ip| !is_private_ip_addr(*ip))
        .collect();
    if public.is_empty() {
        return Err(BrowserFetchError::new(
            "blocked_private",
            "host resolved only to private addresses",
        ));
    }
    Ok(public)
}

/// Static asset hosts a page may load from besides its own host. These are
/// operator-run CDNs whose DNS an attacker does not control, so they are
/// safe to reach without pinning. Everything else is routed to a dead proxy.
const ASSET_HOST_ALLOWLIST: &[&str] = &[
    "*.gstatic.com",
    "*.googleapis.com",
    "*.cloudflare.com",
    "cdnjs.cloudflare.com",
    "cdn.jsdelivr.net",
    "unpkg.com",
    "*.jquery.com",
    "*.bootstrapcdn.com",
    "*.cloudfront.net",
    "*.akamaihd.net",
    "*.akamaized.net",
    "*.fastly.net",
    "*.jsdelivr.net",
];

/// Network containment for one Chrome run. The validated host is pinned to
/// the public address we resolved (no DNS rebinding); every other host —
/// including redirect targets, IP literals such as 127.0.0.1 and loopback,
/// which Chrome would otherwise bypass — is sent to a proxy that does not
/// exist, so it cannot reach local or private services. Only the allow-listed
/// CDN hosts stay direct.
fn chrome_network_flags(host: &str, pinned: std::net::IpAddr) -> Vec<String> {
    let pinned_text = match pinned {
        std::net::IpAddr::V4(v4) => v4.to_string(),
        std::net::IpAddr::V6(v6) => format!("[{v6}]"),
    };
    let mut bypass: Vec<String> = vec![host.to_string()];
    bypass.extend(ASSET_HOST_ALLOWLIST.iter().map(|h| (*h).to_string()));
    bypass.push("<-loopback>".to_string());
    vec![
        format!(
            "--host-resolver-rules=MAP {host} {pinned_text},MAP localhost ~NOTFOUND,MAP *.localhost ~NOTFOUND,MAP *.local ~NOTFOUND,MAP *.internal ~NOTFOUND"
        ),
        "--proxy-server=127.0.0.1:1".to_string(),
        format!("--proxy-bypass-list={}", bypass.join(";")),
    ]
}

struct TmpGuard(PathBuf);

impl Drop for TmpGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn new_tmp_dir() -> Result<(PathBuf, TmpGuard), BrowserFetchError> {
    let dir = std::env::temp_dir().join(format!("aegis-browser-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&dir)
        .map_err(|_| BrowserFetchError::new("launch_failed", "could not prepare chrome profile"))?;
    Ok((dir.clone(), TmpGuard(dir)))
}

fn clamp_u64_arg(args: &Value, key: &str, default: u64, min: u64, max: u64) -> u64 {
    match args.get(key) {
        None | Some(Value::Null) => default,
        Some(v) => v
            .as_u64()
            .or_else(|| v.as_i64().and_then(|n| u64::try_from(n).ok()))
            .map(|n| n.clamp(min, max))
            .unwrap_or(default),
    }
}

fn extract_title(document: &Html) -> String {
    Selector::parse("title")
        .ok()
        .and_then(|sel| document.select(&sel).next())
        .map(|el| el.text().collect::<String>())
        .unwrap_or_default()
        .trim()
        .to_string()
}

fn text_from_selector(
    document: &Html,
    selector: &str,
) -> Result<(String, bool), BrowserFetchError> {
    let sel = Selector::parse(selector)
        .map_err(|_| BrowserFetchError::new("bad_selector", "invalid CSS selector"))?;
    let Some(el) = document.select(&sel).next() else {
        return Ok((String::new(), false));
    };
    let inner = el.inner_html();
    let md = html_to_aria_markdown(&inner);
    let text = if md.trim().is_empty() {
        el.text().collect::<String>()
    } else {
        md
    };
    Ok((text, true))
}

fn redact_tmp(stderr: &str, tmp: &Path) -> String {
    let raw = tmp.to_string_lossy();
    stderr.replace(raw.as_ref(), "[tmp]")
}

impl Orchestrator {
    /// Handle the `browser_fetch` pseudo-tool.
    pub(crate) async fn handle_tool_browser(
        &self,
        tool_call: &ChatCompletionMessageToolCall,
        tool_name: &str,
        tool_args_str: &str,
        ctx: &mut ToolContext<'_>,
    ) -> Result<Option<bool>> {
        tool_dispatch!(self, tool_call, tool_name, tool_args_str, ctx,
            "browser_fetch" => execute_browser_fetch,
        );
        Ok(None)
    }

    async fn execute_browser_fetch(&self, args: Value, ctx: &mut ToolContext<'_>) -> String {
        match self.execute_browser_fetch_inner(&args, ctx).await {
            Ok(body) => body,
            Err(err) => err.to_json(),
        }
    }

    async fn execute_browser_fetch_inner(
        &self,
        args: &Value,
        ctx: &ToolContext<'_>,
    ) -> Result<String, BrowserFetchError> {
        let url_raw = args
            .get("url")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| BrowserFetchError::new("bad_url", "url is required"))?;
        let parsed = validate_browser_url(url_raw)?;
        let public_addrs = ensure_public_dns(&parsed).await?;
        let pinned = public_addrs
            .iter()
            .find(|ip| ip.is_ipv4())
            .copied()
            .unwrap_or(public_addrs[0]);
        let host = parsed
            .host_str()
            .ok_or_else(|| BrowserFetchError::new("bad_url", "url host must not be empty"))?
            .to_ascii_lowercase();

        let mode = args
            .get("mode")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("text");
        let mode = if mode == "screenshot" {
            "screenshot"
        } else {
            "text"
        };
        let selector = args
            .get("selector")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty());
        if let Some(sel) = selector
            && Selector::parse(sel).is_err()
        {
            return Err(BrowserFetchError::new(
                "bad_selector",
                "invalid CSS selector",
            ));
        }
        let wait_ms = clamp_u64_arg(args, "wait_ms", DEFAULT_WAIT_MS, 0, MAX_WAIT_MS);
        let max_chars = clamp_u64_arg(
            args,
            "max_chars",
            DEFAULT_MAX_CHARS,
            MIN_MAX_CHARS,
            MAX_MAX_CHARS,
        ) as usize;

        let Some(bin) = chrome_binary() else {
            return Err(BrowserFetchError::new(
                "chrome_missing",
                "headless Chrome is not installed on this machine",
            ));
        };

        let started = Instant::now();
        let (tmp, _guard) = new_tmp_dir()?;
        let screenshot_path = tmp.join("page.png");

        let mut cmd = tokio::process::Command::new(&bin);
        cmd.kill_on_drop(true)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .args(CHROME_FLAGS)
            .args(chrome_network_flags(&host, pinned))
            .arg(format!("--user-data-dir={}", tmp.display()))
            .arg(format!("--virtual-time-budget={wait_ms}"))
            .arg("--timeout=30000");
        if mode == "screenshot" {
            cmd.arg(format!("--screenshot={}", screenshot_path.display()))
                .arg("--window-size=1280,2000")
                .arg(parsed.as_str());
        } else {
            cmd.arg("--dump-dom").arg(parsed.as_str());
        }

        let output = match tokio::time::timeout(RUN_TIMEOUT, cmd.output()).await {
            Ok(Ok(output)) => output,
            Ok(Err(_)) => {
                return Err(BrowserFetchError::new(
                    "launch_failed",
                    "failed to launch chrome",
                ));
            }
            Err(_) => {
                return Err(BrowserFetchError::new("timeout", "chrome exceeded 45s"));
            }
        };

        let stderr = String::from_utf8_lossy(&output.stderr);
        tracing::debug!(stderr = %redact_tmp(&stderr, &tmp), "browser_fetch chrome stderr");

        if mode == "screenshot" {
            return self.finish_screenshot(
                &parsed,
                &screenshot_path,
                ctx.user_did,
                started,
                output.status.success(),
            );
        }

        let dom = String::from_utf8_lossy(&output.stdout).into_owned();
        if dom.trim().is_empty() && !output.status.success() {
            return Err(BrowserFetchError::new(
                "launch_failed",
                "chrome produced no page content",
            ));
        }

        let document = Html::parse_document(&dom);
        let title = extract_title(&document);
        let (raw_text, matched) = if let Some(sel) = selector {
            text_from_selector(&document, sel)?
        } else {
            (html_to_aria_markdown(&dom), true)
        };
        let truncated = raw_text.chars().count() > max_chars;
        let text = truncate_chars(&raw_text, max_chars);
        let chars = text.chars().count();
        let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);

        Ok(json!({
            "status": "ok",
            "url": parsed.as_str(),
            "final_url": parsed.as_str(),
            "title": title,
            "mode": "text",
            "text": text,
            "chars": chars,
            "truncated": truncated,
            "matched": matched,
            "image_url": Value::Null,
            "elapsed_ms": elapsed_ms,
        })
        .to_string())
    }

    fn finish_screenshot(
        &self,
        parsed: &url::Url,
        screenshot_path: &Path,
        owner_did: &str,
        started: Instant,
        _chrome_ok: bool,
    ) -> Result<String, BrowserFetchError> {
        let png = std::fs::read(screenshot_path).map_err(|_| {
            BrowserFetchError::new("launch_failed", "chrome did not write a screenshot")
        })?;
        if png.len() > WEBAPP_GENERATED_IMAGE_MAX_BYTES {
            return Err(BrowserFetchError::new(
                "too_large",
                "screenshot exceeds the generated-image size limit",
            ));
        }
        if png.is_empty() {
            return Err(BrowserFetchError::new(
                "launch_failed",
                "chrome wrote an empty screenshot",
            ));
        }
        let data_url = format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(&png)
        );
        let image_url = store_generated_image_for_owner(
            self.memory.db_path(),
            owner_did,
            &data_url,
            chrono::Utc::now(),
        )
        .map_err(|_| {
            BrowserFetchError::new("launch_failed", "could not store screenshot for the owner")
        })?;
        let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        Ok(json!({
            "status": "ok",
            "url": parsed.as_str(),
            "final_url": parsed.as_str(),
            "title": "",
            "mode": "screenshot",
            "text": "",
            "chars": 0,
            "truncated": false,
            "matched": true,
            "image_url": image_url,
            "elapsed_ms": elapsed_ms,
        })
        .to_string())
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::path::PathBuf;

    use serde_json::{Value, json};
    use tokio::sync::{Mutex, MutexGuard};

    use super::{BrowserFetchError, chrome_binary, ensure_public_dns, validate_browser_url};
    use crate::memory::MemoryHandle;
    use crate::orchestrator::Orchestrator;
    use crate::settings::RuntimeSettings;
    use crate::test_tool_helpers::{CtxState, make_tool_call, test_orchestrator};

    static CHROME_BIN_ENV_LOCK: Mutex<()> = Mutex::const_new(());

    struct ChromeBinGuard {
        prev: Option<String>,
        _lock: MutexGuard<'static, ()>,
    }

    impl ChromeBinGuard {
        #[allow(unsafe_code)]
        async fn set(path: &str) -> Self {
            let lock = CHROME_BIN_ENV_LOCK.lock().await;
            let prev = std::env::var("AEGIS_CHROME_BIN").ok();
            unsafe { std::env::set_var("AEGIS_CHROME_BIN", path) };
            Self { prev, _lock: lock }
        }

        #[allow(unsafe_code)]
        async fn clear() -> Self {
            let lock = CHROME_BIN_ENV_LOCK.lock().await;
            let prev = std::env::var("AEGIS_CHROME_BIN").ok();
            unsafe { std::env::remove_var("AEGIS_CHROME_BIN") };
            Self { prev, _lock: lock }
        }
    }

    impl Drop for ChromeBinGuard {
        #[allow(unsafe_code)]
        fn drop(&mut self) {
            match &self.prev {
                Some(v) => unsafe { std::env::set_var("AEGIS_CHROME_BIN", v) },
                None => unsafe { std::env::remove_var("AEGIS_CHROME_BIN") },
            }
        }
    }

    fn err_kind(r: Result<url::Url, BrowserFetchError>) -> &'static str {
        r.expect_err("expected rejection").kind
    }

    fn last_tool_json(state: &CtxState) -> Value {
        let msg = state.messages.last().expect("tool result message");
        let text = match msg {
            async_openai::types::chat::ChatCompletionRequestMessage::Tool(t) => match &t.content {
                async_openai::types::chat::ChatCompletionRequestToolMessageContent::Text(s) => {
                    s.clone()
                }
                other => panic!("unexpected tool content: {other:?}"),
            },
            other => panic!("expected tool message, got {other:?}"),
        };
        serde_json::from_str(&text).unwrap_or_else(|e| panic!("tool result not JSON ({e}): {text}"))
    }

    async fn run(orch: &Orchestrator, args: Value) -> (Option<bool>, CtxState) {
        let args_str = args.to_string();
        let tc = make_tool_call("browser_fetch", &args_str);
        let mut state = CtxState::new();
        let r = orch
            .handle_tool_browser(&tc, "browser_fetch", &args_str, &mut state.ctx())
            .await
            .expect("handler must not error");
        (r, state)
    }

    fn orch_mem() -> Orchestrator {
        let mem = MemoryHandle::spawn_in_memory().expect("in-memory DB");
        test_orchestrator(mem, std::sync::Arc::new(RuntimeSettings::new()))
    }

    fn write_fake_chrome(dir: &std::path::Path, body: &str) -> PathBuf {
        let script = dir.join("fake-chrome");
        let mut f = std::fs::File::create(&script).expect("create fake chrome");
        writeln!(f, "#!/bin/sh").unwrap();
        // Single-quoted so the HTML is emitted verbatim regardless of Chrome flags.
        writeln!(f, "echo '{body}'").unwrap();
        f.flush().unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        script
    }

    #[test]
    fn chrome_network_flags_pin_host_and_kill_other_hosts() {
        let flags = super::chrome_network_flags("example.com", "93.184.216.34".parse().unwrap());
        assert_eq!(flags.len(), 3);
        assert!(flags[0].starts_with("--host-resolver-rules=MAP example.com 93.184.216.34,"));
        assert!(flags[0].contains("MAP localhost ~NOTFOUND"));
        assert_eq!(flags[1], "--proxy-server=127.0.0.1:1");
        assert!(flags[2].starts_with("--proxy-bypass-list=example.com;"));
        // Loopback and IP literals must NOT be bypassed: Chrome's implicit
        // loopback bypass is removed so 127.0.0.1 redirects hit the dead proxy.
        assert!(flags[2].ends_with(";<-loopback>"));
        assert!(!flags[2].contains("127.0.0.1"));
        let v6 = super::chrome_network_flags(
            "example.com",
            "2606:2800:220:1:248:1893:25c8:1946".parse().unwrap(),
        );
        assert!(v6[0].contains("MAP example.com [2606:2800:220:1:248:1893:25c8:1946],"));
    }

    #[test]
    fn validate_browser_url_rejects_private_and_privacy() {
        assert_eq!(
            err_kind(validate_browser_url("file:///etc/passwd")),
            "bad_url"
        );
        assert_eq!(
            err_kind(validate_browser_url("http://localhost:3000")),
            "blocked_private"
        );
        assert_eq!(
            err_kind(validate_browser_url("http://127.0.0.1")),
            "blocked_private"
        );
        assert_eq!(
            err_kind(validate_browser_url("http://10.0.0.5")),
            "blocked_private"
        );
        assert_eq!(
            err_kind(validate_browser_url("http://[::1]")),
            "blocked_private"
        );
        assert_eq!(
            err_kind(validate_browser_url("http://foo.local")),
            "blocked_private"
        );
        let privacy =
            validate_browser_url("https://www.google.com/maps/dir/123+Example+Pl+SE,+Example+City")
                .expect_err("street address in URL must be privacy");
        assert_eq!(privacy.kind, "privacy");
        assert!(privacy.privacy_note.is_some());

        let ok = validate_browser_url("https://example.com/a?b=c").expect("public https url");
        assert_eq!(ok.scheme(), "https");
        assert_eq!(ok.host_str(), Some("example.com"));
    }

    #[tokio::test]
    async fn chrome_binary_env_override() {
        let dir = tempfile::tempdir().expect("tempdir");
        let fake = dir.path().join("override-chrome");
        std::fs::write(&fake, b"").unwrap();
        let _guard = ChromeBinGuard::set(fake.to_str().unwrap()).await;
        let got = chrome_binary().expect("override must win");
        assert_eq!(got, fake);
    }

    #[tokio::test]
    async fn unknown_tool_passes_through() {
        let orch = orch_mem();
        let tc = make_tool_call("run_external", "{}");
        let mut state = CtxState::new();
        let r = orch
            .handle_tool_browser(&tc, "run_external", "{}", &mut state.ctx())
            .await
            .unwrap();
        assert_eq!(r, None);
    }

    #[tokio::test]
    async fn pipeline_text_mode_without_chrome() {
        let dir = tempfile::tempdir().expect("tempdir");
        let script = write_fake_chrome(
            dir.path(),
            "<html><title>T</title><body><h1>Hi</h1><p>para</p></body></html>",
        );
        let _guard = ChromeBinGuard::set(script.to_str().unwrap()).await;
        ensure_public_dns(&url::Url::parse("https://example.com").unwrap())
            .await
            .expect("example.com must resolve publicly");

        let orch = orch_mem();
        let (r, state) = run(
            &orch,
            json!({"url": "https://example.com/a?b=c", "wait_ms": 0}),
        )
        .await;
        assert_eq!(r, Some(true));
        assert_eq!(state.tools_called, vec!["browser_fetch".to_string()]);
        assert_eq!(state.prev_results.len(), 1);
        let v = last_tool_json(&state);
        assert_eq!(v["status"], "ok");
        assert_eq!(v["title"], "T");
        assert_eq!(v["mode"], "text");
        let text = v["text"].as_str().unwrap();
        assert!(text.contains("Hi"), "text={text}");
        assert!(text.contains("para"), "text={text}");
        assert_eq!(v["truncated"], false);
        assert_eq!(v["matched"], true);
    }

    #[tokio::test]
    async fn pipeline_truncates_long_body() {
        let long = "x".repeat(800);
        let html = format!("<html><title>T</title><body><p>{long}</p></body></html>");
        let dir = tempfile::tempdir().expect("tempdir");
        let script = write_fake_chrome(dir.path(), &html);
        let _guard = ChromeBinGuard::set(script.to_str().unwrap()).await;
        let orch = orch_mem();
        let (r, state) = run(
            &orch,
            json!({
                "url": "https://example.com",
                "wait_ms": 0,
                "max_chars": 500
            }),
        )
        .await;
        assert_eq!(r, Some(true));
        let v = last_tool_json(&state);
        assert_eq!(v["status"], "ok");
        assert_eq!(v["truncated"], true);
        assert_eq!(v["chars"], 500);
        assert_eq!(v["text"].as_str().unwrap().chars().count(), 500);
    }

    #[tokio::test]
    async fn pipeline_selector_returns_matching_element() {
        let dir = tempfile::tempdir().expect("tempdir");
        let script = write_fake_chrome(
            dir.path(),
            "<html><title>T</title><body><h1>Hi</h1><p>para</p></body></html>",
        );
        let _guard = ChromeBinGuard::set(script.to_str().unwrap()).await;
        let orch = orch_mem();
        let (r, state) = run(
            &orch,
            json!({
                "url": "https://example.com",
                "wait_ms": 0,
                "selector": "p"
            }),
        )
        .await;
        assert_eq!(r, Some(true));
        let v = last_tool_json(&state);
        assert_eq!(v["status"], "ok");
        let text = v["text"].as_str().unwrap();
        assert!(text.contains("para"), "text={text}");
        assert!(!text.contains("Hi"), "selector should drop the h1: {text}");
        assert_eq!(v["matched"], true);
    }

    /// Live containment check (needs Chrome + network). Serves a marker on a
    /// loopback port and asserts Chrome cannot read it directly, nor via an
    /// HTTP redirect from a public host, while the pinned host still loads.
    /// Run with: cargo test -p aegis-runtime-core --lib -- --ignored browser_fetch_live_containment --nocapture
    #[tokio::test]
    #[ignore]
    async fn browser_fetch_live_containment() {
        use tokio::io::AsyncWriteExt;
        if !PathBuf::from("/opt/google/chrome/chrome").exists() {
            return;
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let _ = socket
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 12\r\n\r\nLOCAL_MARKER",
                    )
                    .await;
            }
        });
        let _guard = ChromeBinGuard::clear().await;
        let orch = orch_mem();

        // Direct loopback is refused before Chrome starts.
        let (_, state) = run(&orch, json!({"url": format!("http://127.0.0.1:{port}/m")})).await;
        assert_eq!(last_tool_json(&state)["error_kind"], "blocked_private");

        // A public host redirecting to loopback must not leak the marker.
        let redirect =
            format!("https://httpbin.org/redirect-to?url=http%3A%2F%2F127.0.0.1%3A{port}%2Fm");
        let (_, state) = run(&orch, json!({"url": redirect, "wait_ms": 1500})).await;
        let v = last_tool_json(&state);
        let text = v["text"].as_str().unwrap_or("");
        assert!(
            !text.contains("LOCAL_MARKER"),
            "redirect leaked loopback content: {v}"
        );

        // The pinned host itself still renders.
        let (_, state) = run(
            &orch,
            json!({"url": "https://example.com", "wait_ms": 1500}),
        )
        .await;
        let v = last_tool_json(&state);
        assert_eq!(v["status"], "ok", "pinned host must load: {v}");
        assert_eq!(v["title"], "Example Domain");
    }

    #[tokio::test]
    #[ignore]
    async fn browser_fetch_live_example_com() {
        let chrome = PathBuf::from("/opt/google/chrome/chrome");
        if !chrome.exists() {
            return;
        }
        let _guard = ChromeBinGuard::clear().await;
        let orch = orch_mem();
        let (r, state) = run(
            &orch,
            json!({"url": "https://example.com", "wait_ms": 1500}),
        )
        .await;
        assert_eq!(r, Some(true));
        let v = last_tool_json(&state);
        assert_eq!(v["status"], "ok", "live fetch failed: {v}");
        let elapsed = v["elapsed_ms"].as_u64().unwrap();
        eprintln!(
            "browser_fetch_live_example_com title={:?} elapsed_ms={elapsed}",
            v["title"]
        );
        assert_eq!(v["title"], "Example Domain");
        assert!(
            elapsed < 15_000,
            "example.com dump-dom took {elapsed} ms (budget 15000)"
        );
    }
}
