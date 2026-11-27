//! Hercules delegation client — Aegis Runtime hands *tasks* to Hercules
//! (`docs/2026-09-05-hercules-delegation-design.md`); Hercules owns attempts,
//! provider sessions, evidence and verification. Aegis Runtime never talks to the
//! coding runtime for a delegated task.
//!
//! RED stub: types are final, behaviour lands after the tests fail.
use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct HerculesLintFinding {
    pub severity: String,
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Default)]
pub struct HerculesLint {
    #[serde(default)]
    pub ok: bool,
    #[serde(default)]
    pub findings: Vec<HerculesLintFinding>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct HerculesDraft {
    pub title: String,
    #[serde(default)]
    pub slug: Option<String>,
    pub body: String,
    #[serde(default)]
    pub priority: Option<i64>,
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default)]
    pub vague: bool,
    #[serde(default)]
    pub warnings: Vec<String>,
    #[serde(default)]
    pub lint: HerculesLint,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
// Hercules speaks camelCase; the LLM-facing tool result stays snake_case.
#[serde(rename_all(deserialize = "camelCase"))]
pub struct HerculesTask {
    pub id: String,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub repo: Option<String>,
    #[serde(default)]
    pub adapter: Option<String>,
    #[serde(default)]
    pub verified_sha: Option<String>,
    #[serde(default)]
    pub pr_url: Option<String>,
    #[serde(default)]
    pub pr_status: Option<String>,
    #[serde(default)]
    pub labels: Vec<String>,
    /// Why the scheduler is not admitting a todo/retrying task (additive
    /// field, hercules 2026-09-05): `{code, …}` with codes such as
    /// repo_readiness_incomplete, plan_pending, capacity, retry_backoff.
    /// Missing = not held (older daemons).
    #[serde(default)]
    pub dispatch_hold_reason: Option<serde_json::Value>,
}

impl HerculesTask {
    pub fn hold_code(&self) -> Option<&str> {
        self.dispatch_hold_reason
            .as_ref()
            .and_then(|v| v.get("code"))
            .and_then(|c| c.as_str())
    }
}

/// States after which the poller stops refreshing a task. Hercules
/// `TASK_STATES` (hercules session, 2026-09-05): todo, in_progress, review,
/// done, cancelled, retrying, failed, deleted. `review` waits for a human and
/// is NOT terminal; `failed` can be reopened by an operator (an explicit
/// `status` call still refreshes it); `deleted` is soft-deleted and becomes a
/// 404 after purge. `missing` is Aegis Runtime's own marker for a 404 (staging
/// resets its store on every deploy).
pub fn is_terminal_state(state: &str) -> bool {
    matches!(
        state,
        "done" | "failed" | "cancelled" | "deleted" | "missing"
    )
}

#[cfg(test)]
mod terminal_state_tests {
    use super::is_terminal_state;

    #[test]
    fn hercules_state_table() {
        for open in ["todo", "in_progress", "review", "retrying", ""] {
            assert!(!is_terminal_state(open), "{open} must keep polling");
        }
        for done in ["done", "failed", "cancelled", "deleted", "missing"] {
            assert!(is_terminal_state(done), "{done} must stop polling");
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CreateTaskInput {
    /// Caller-chosen task id (lowercase letters/digits/hyphens). Hercules
    /// refuses a duplicate id instead of minting a second task, which is
    /// what makes a create retry after a timeout safe.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub title: String,
    pub repo: String,
    pub body: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<i64>,
    pub labels: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub adapter: Option<String>,
}

#[derive(Debug, Clone)]
pub struct HerculesClient {
    base: String,
    token: String,
    production: bool,
    http: reqwest::Client,
    /// `POST /api/tasks` commits to the control repo and may refresh a cold
    /// git mirror first (hercules session, 2026-09-05: budget 90 s).
    create_timeout: std::time::Duration,
    /// After a create timeout the task usually still lands (Hercules's live
    /// commit path rebuilt 200+ tasks of history in >150 s on 2026-09-05), so
    /// the client polls `GET /api/tasks/:id` for this long before giving up.
    create_confirm_window: std::time::Duration,
    create_confirm_interval: std::time::Duration,
}

/// How long to poll for a timed-out create to become visible.
const CREATE_CONFIRM_WINDOW: std::time::Duration = std::time::Duration::from_secs(240);
const CREATE_CONFIRM_INTERVAL: std::time::Duration = std::time::Duration::from_secs(15);

/// Default timeout for reads/drafts/lint.
const DEFAULT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
/// Default timeout for `create_task` (see `HerculesClient::create_timeout`).
// Hercules main 5bb4c6d (2026-09-05): create 12–13 s measured; 60 s budget.
const CREATE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

/// True when the error came from the client-side timeout: the outcome is
/// then UNKNOWN (Hercules may have created the task), not a failure.
pub fn is_timeout_error(err: &anyhow::Error) -> bool {
    let msg = err.to_string();
    msg.contains("timed out") || msg.contains("timeout")
}

/// Deterministic Hercules task id for a delegation:
/// `aegis-runtime-<title slug ≤32>-<8 hex of sha256(principal|title|body)>`.
/// Same request → same id, so a retry after a timeout is refused as a
/// duplicate instead of creating a second task.
pub fn task_id_for(principal: &str, title: &str, body: &str, attempt: u32) -> String {
    use sha2::{Digest, Sha256};
    let mut slug = String::new();
    let mut last_dash = true;
    for ch in title.chars().flat_map(|c| c.to_lowercase()) {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch);
            last_dash = false;
        } else if !last_dash {
            slug.push('-');
            last_dash = true;
        }
        if slug.len() >= 32 {
            break;
        }
    }
    let slug = slug.trim_matches('-').to_string();
    let mut hasher = Sha256::new();
    hasher.update(principal.as_bytes());
    hasher.update(b"|");
    hasher.update(title.as_bytes());
    hasher.update(b"|");
    hasher.update(body.as_bytes());
    if attempt > 1 {
        // A cancelled Hercules id stays taken; re-delegating the same spec
        // needs a fresh id, chosen explicitly by the caller (`attempt`).
        hasher.update(b"|attempt=");
        hasher.update(attempt.to_string().as_bytes());
    }
    let digest = hex::encode(hasher.finalize());
    if slug.is_empty() {
        format!("aegis-runtime-{}", &digest[..8])
    } else {
        format!("aegis-runtime-{slug}-{}", &digest[..8])
    }
}

impl HerculesClient {
    /// `production` marks a Hercules whose `create` dispatches real (paid)
    /// agent runs; the tool refuses such creates without explicit confirmation.
    pub fn new(base: &str, token: &str, production: bool) -> Self {
        Self {
            base: base.trim_end_matches('/').to_string(),
            token: token.to_string(),
            production,
            http: reqwest::Client::new(),
            create_timeout: CREATE_TIMEOUT,
            create_confirm_window: CREATE_CONFIRM_WINDOW,
            create_confirm_interval: CREATE_CONFIRM_INTERVAL,
        }
    }

    pub fn with_create_timeout(mut self, timeout: std::time::Duration) -> Self {
        self.create_timeout = timeout;
        self
    }

    pub fn with_create_confirm(
        mut self,
        window: std::time::Duration,
        interval: std::time::Duration,
    ) -> Self {
        self.create_confirm_window = window;
        self.create_confirm_interval = interval;
        self
    }

    /// Fire-and-poll create: POST, and if the POST times out keep polling
    /// `GET /api/tasks/:id` (the caller chose the id) until the task shows
    /// up or the confirm window ends. Returns `Ok(None)` when the outcome
    /// is still unknown after the window (never a spurious retry).
    pub async fn create_task_confirmed(
        &self,
        input: &CreateTaskInput,
    ) -> Result<Option<HerculesTask>> {
        match self.create_task(input).await {
            Ok(task) => Ok(Some(task)),
            Err(e) if is_timeout_error(&e) => {
                let Some(id) = input.id.as_deref() else {
                    return Err(e);
                };
                tracing::warn!(
                    task_id = id,
                    "hercules create timed out; polling for the task"
                );
                let deadline = std::time::Instant::now() + self.create_confirm_window;
                loop {
                    tokio::time::sleep(self.create_confirm_interval).await;
                    match self.get_task(id).await {
                        Ok(Some(task)) => return Ok(Some(task)),
                        Ok(None) => {}
                        Err(err) => {
                            tracing::warn!(task_id = id, error = %err, "hercules confirm poll failed")
                        }
                    }
                    if std::time::Instant::now() >= deadline {
                        return Ok(None);
                    }
                }
            }
            Err(e) => Err(e),
        }
    }

    /// OSS / default deploy: Hercules delegation is **off** unless this flag
    /// is truthy. Credentials alone do not enable the tool or prompt.
    pub fn delegation_enabled() -> bool {
        match std::env::var("AEGIS_HERCULES_DELEGATION_ENABLED") {
            Ok(v) => {
                let t = v.trim();
                t == "1" || t.eq_ignore_ascii_case("true") || t.eq_ignore_ascii_case("yes")
            }
            Err(_) => false,
        }
    }

    /// `AEGIS_HERCULES_DELEGATION_ENABLED=1` **and** `HERCULES_API_BASE` +
    /// `HERCULES_TOKEN`. Flag off (the OSS default) → `None` even if creds
    /// are present. `HERCULES_ENV=staging|production` overrides the
    /// port-based guess (staging listens on :8788 by convention).
    pub fn from_env() -> Option<Self> {
        if !Self::delegation_enabled() {
            return None;
        }
        let base = std::env::var("HERCULES_API_BASE").ok()?;
        let token = std::env::var("HERCULES_TOKEN").ok()?;
        if base.trim().is_empty() || token.trim().is_empty() {
            return None;
        }
        let production = match std::env::var("HERCULES_ENV").ok().as_deref() {
            Some("staging") => false,
            Some("production") => true,
            _ => !(base.contains(":8788") || base.contains("staging")),
        };
        Some(Self::new(&base, &token, production))
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    pub fn is_production(&self) -> bool {
        self.production
    }

    async fn request<T: serde::de::DeserializeOwned>(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<&serde_json::Value>,
        timeout: std::time::Duration,
    ) -> Result<Option<T>> {
        let url = format!("{}{}", self.base, path);
        let mut req = self
            .http
            .request(method, &url)
            .bearer_auth(&self.token)
            .timeout(timeout);
        if let Some(b) = body {
            req = req.json(b);
        }
        let resp = req.send().await.map_err(|e| {
            if e.is_timeout() {
                anyhow::anyhow!("hercules {path}: timed out after {timeout:?}")
            } else {
                // Display of a reqwest error hides its source chain; keep it.
                let chain: Vec<String> =
                    std::iter::successors(std::error::Error::source(&e), |err| err.source())
                        .map(|err| err.to_string())
                        .collect();
                anyhow::anyhow!(
                    "hercules {path}: request failed: {e} ({})",
                    chain.join(": ")
                )
            }
        })?;
        let status = resp.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            let snippet: String = text.chars().take(200).collect();
            anyhow::bail!("hercules {path}: HTTP {}: {snippet}", status.as_u16());
        }
        let parsed = serde_json::from_str::<T>(&text)
            .map_err(|e| anyhow::anyhow!("hercules {path}: bad response: {e}"))?;
        Ok(Some(parsed))
    }

    pub async fn draft_from_intent(&self, intent: &str, repo: &str) -> Result<HerculesDraft> {
        #[derive(Deserialize)]
        struct Resp {
            draft: HerculesDraft,
        }
        let body = serde_json::json!({ "intent": intent, "repo": repo });
        let r: Resp = self
            .request(
                reqwest::Method::POST,
                "/api/tasks/draft-from-intent",
                Some(&body),
                DEFAULT_TIMEOUT,
            )
            .await?
            .ok_or_else(|| anyhow::anyhow!("hercules draft-from-intent: not found"))?;
        Ok(r.draft)
    }

    pub async fn lint_acceptance(&self, body: &str) -> Result<HerculesLint> {
        let payload = serde_json::json!({ "body": body });
        self.request(
            reqwest::Method::POST,
            "/api/tasks/lint-acceptance",
            Some(&payload),
            DEFAULT_TIMEOUT,
        )
        .await?
        .ok_or_else(|| anyhow::anyhow!("hercules lint-acceptance: not found"))
    }

    pub async fn create_task(&self, input: &CreateTaskInput) -> Result<HerculesTask> {
        let payload = serde_json::to_value(input)?;
        self.request(
            reqwest::Method::POST,
            "/api/tasks",
            Some(&payload),
            self.create_timeout,
        )
        .await?
        .ok_or_else(|| anyhow::anyhow!("hercules create task: not found"))
    }

    /// `Ok(None)` on 404 — staging tasks are disposable.
    pub async fn get_task(&self, id: &str) -> Result<Option<HerculesTask>> {
        let path = format!("/api/tasks/{}", urlencoding_component(id));
        self.request(reqwest::Method::GET, &path, None, DEFAULT_TIMEOUT)
            .await
    }
}

/// Percent-encode a path segment (task ids are opaque strings).
fn urlencoding_component(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
pub(crate) mod mock {
    //! Minimal Hercules stand-in for tests (axum on 127.0.0.1:0).
    use axum::{
        Json, Router,
        extract::Path,
        http::{HeaderMap, StatusCode},
        routing::{get, post},
    };
    use serde_json::{Value, json};
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    pub struct Seen {
        pub created: Vec<Value>,
        pub auth_headers: Vec<String>,
    }

    pub struct MockHercules {
        pub base: String,
        pub seen: Arc<Mutex<Seen>>,
    }

    fn auth_ok(headers: &HeaderMap, seen: &Arc<Mutex<Seen>>) -> bool {
        let h = headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        seen.lock().unwrap().auth_headers.push(h.clone());
        h == "Bearer test-token"
    }

    pub async fn start() -> MockHercules {
        let seen = Arc::new(Mutex::new(Seen::default()));
        let s1 = seen.clone();
        let s2 = seen.clone();
        let s3 = seen.clone();
        let s4 = seen.clone();
        let app = Router::new()
            .route(
                "/api/tasks/draft-from-intent",
                post(move |headers: HeaderMap, Json(body): Json<Value>| {
                    let seen = s1.clone();
                    async move {
                        if !auth_ok(&headers, &seen) {
                            return (StatusCode::UNAUTHORIZED, Json(json!({"error":"unauthorized"})));
                        }
                        let intent = body["intent"].as_str().unwrap_or("").to_string();
                        (StatusCode::OK, Json(json!({
                            "draft": {
                                "title": intent.chars().take(40).collect::<String>(),
                                "slug": "draft-slug",
                                "body": format!("## Task\n{intent}\n\n## Repo\n{}\n\n## Definition of Done\n- [ ] (mechanical) Implementation complete; gates green @gate:test\n- [ ] (reviewer) Matches intent; no scope creep\n", body["repo"].as_str().unwrap_or("")),
                                "priority": 50,
                                "labels": ["needs-plan"],
                                "vague": false,
                                "warnings": [],
                                "lint": {"ok": true, "findings": [{"severity":"warn","code":"all-gate-bound","message":"every mechanical criterion is @gate:"}], "counts": {"error":0,"warn":1,"info":0}}
                            },
                            "jobId": "dj-test"
                        })))
                    }
                }),
            )
            .route(
                "/api/tasks/lint-acceptance",
                post(move |headers: HeaderMap, Json(body): Json<Value>| {
                    let seen = s2.clone();
                    async move {
                        if !auth_ok(&headers, &seen) {
                            return (StatusCode::UNAUTHORIZED, Json(json!({"error":"unauthorized"})));
                        }
                        let ok = body["body"].as_str().unwrap_or("").contains("## Definition of Done");
                        (StatusCode::OK, Json(json!({"ok": ok, "findings": [], "counts": {"error": if ok {0} else {1}, "warn":0, "info":0}})))
                    }
                }),
            )
            .route(
                "/api/tasks",
                post(move |headers: HeaderMap, Json(body): Json<Value>| {
                    let seen = s3.clone();
                    async move {
                        if !auth_ok(&headers, &seen) {
                            return (StatusCode::UNAUTHORIZED, Json(json!({"error":"unauthorized"})));
                        }
                        seen.lock().unwrap().created.push(body.clone());
                        if body["title"].as_str().unwrap_or("").starts_with("slow:") {
                            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                        }
                        let id = body["id"].as_str().unwrap_or("t-100").to_string();
                        (StatusCode::CREATED, Json(json!({
                            "id": id, "state": "todo", "title": body["title"], "repo": body["repo"],
                            "labels": body["labels"], "adapter": body["adapter"],
                            "acLint": {"ok": true, "findings": []}
                        })))
                    }
                }),
            )
            .route(
                "/api/tasks/{id}",
                get(move |headers: HeaderMap, Path(id): Path<String>| {
                    let seen = s4.clone();
                    async move {
                        if !auth_ok(&headers, &seen) {
                            return (StatusCode::UNAUTHORIZED, Json(json!({"error":"unauthorized"})));
                        }
                        if id == "gone" || id.contains("invisible") {
                            return (StatusCode::NOT_FOUND, Json(json!({"error":"not found"})));
                        }
                        if id.contains("held") {
                            return (StatusCode::OK, Json(json!({
                                "id": id, "state": "todo",
                                "dispatchHoldReason": {"code": "repo_readiness_incomplete", "repo": "aegis"}
                            })));
                        }
                        (StatusCode::OK, Json(json!({
                            "id": id, "state": "review", "verifiedSha": "abc1234", "prUrl": "https://example.invalid/pr/1",
                            "prStatus": "open", "labels": ["work_item_id=7", "principal=did:aegis:test"]
                        })))
                    }
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.ok();
        });
        MockHercules {
            base: format!("http://{addr}"),
            seen,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn draft_from_intent_parses_draft_and_lint_and_sends_bearer() {
        let mock = mock::start().await;
        let client = HerculesClient::new(&mock.base, "test-token", false);
        let draft = client
            .draft_from_intent("Add a --version flag", "aegis")
            .await
            .expect("draft");
        assert!(draft.body.contains("## Definition of Done"));
        assert_eq!(draft.labels, vec!["needs-plan".to_string()]);
        assert!(draft.lint.ok);
        assert_eq!(draft.lint.findings[0].code, "all-gate-bound");
        assert!(
            mock.seen
                .lock()
                .unwrap()
                .auth_headers
                .iter()
                .all(|h| h == "Bearer test-token")
        );
    }

    #[tokio::test]
    async fn create_task_posts_labels_and_returns_task_id() {
        let mock = mock::start().await;
        let client = HerculesClient::new(&mock.base, "test-token", false);
        let task = client
            .create_task(&CreateTaskInput {
                id: None,
                title: "t".into(),
                repo: "aegis".into(),
                body: "## Definition of Done\n- [ ] (reviewer) x".into(),
                priority: Some(50),
                labels: vec!["work_item_id=7".into(), "principal=did:aegis:test".into()],
                adapter: None,
            })
            .await
            .expect("create");
        assert_eq!(task.id, "t-100");
        assert_eq!(task.state, "todo");
        let created = mock.seen.lock().unwrap().created.clone();
        assert_eq!(created.len(), 1);
        assert_eq!(created[0]["labels"][0], "work_item_id=7");
        assert!(
            created[0].get("adapter").is_none(),
            "null adapter is omitted"
        );
    }

    #[tokio::test]
    async fn create_task_sends_caller_chosen_id() {
        let mock = mock::start().await;
        let client = HerculesClient::new(&mock.base, "test-token", false);
        let task = client
            .create_task(&CreateTaskInput {
                id: Some("aegis-runtime-add-version-1a2b3c4d".into()),
                title: "t".into(),
                repo: "aegis".into(),
                body: "## Definition of Done\n- [ ] (reviewer) x".into(),
                priority: None,
                labels: vec![],
                adapter: None,
            })
            .await
            .expect("create");
        assert_eq!(
            task.id, "aegis-runtime-add-version-1a2b3c4d",
            "mock echoes the id"
        );
        let created = mock.seen.lock().unwrap().created.clone();
        assert_eq!(created[0]["id"], "aegis-runtime-add-version-1a2b3c4d");
    }

    #[tokio::test]
    async fn create_timeout_is_reported_as_timeout() {
        let mock = mock::start().await;
        let client = HerculesClient::new(&mock.base, "test-token", false)
            .with_create_timeout(std::time::Duration::from_millis(200));
        let err = client
            .create_task(&CreateTaskInput {
                id: None,
                title: "slow: t".into(),
                repo: "aegis".into(),
                body: "## Definition of Done".into(),
                priority: None,
                labels: vec![],
                adapter: None,
            })
            .await
            .expect_err("mock sleeps longer than the timeout");
        assert!(is_timeout_error(&err), "{err:#}");
    }

    #[tokio::test]
    async fn create_confirmed_polls_after_timeout_until_visible() {
        let mock = mock::start().await;
        let client = HerculesClient::new(&mock.base, "test-token", false)
            .with_create_timeout(std::time::Duration::from_millis(200))
            .with_create_confirm(
                std::time::Duration::from_secs(5),
                std::time::Duration::from_millis(100),
            );
        let started = std::time::Instant::now();
        let task = client
            .create_task_confirmed(&CreateTaskInput {
                id: Some("aegis-runtime-slow-x-11111111".into()),
                title: "slow: x".into(),
                repo: "hercules".into(),
                body: "## Definition of Done".into(),
                priority: None,
                labels: vec![],
                adapter: Some("grok".into()),
            })
            .await
            .expect("no hard error")
            .expect("task became visible");
        assert_eq!(task.id, "aegis-runtime-slow-x-11111111");
        assert!(
            started.elapsed() < std::time::Duration::from_secs(3),
            "found on an early poll"
        );
    }

    #[tokio::test]
    async fn create_confirmed_gives_up_as_unknown_after_window() {
        let mock = mock::start().await;
        let client = HerculesClient::new(&mock.base, "test-token", false)
            .with_create_timeout(std::time::Duration::from_millis(200))
            .with_create_confirm(
                std::time::Duration::from_millis(400),
                std::time::Duration::from_millis(100),
            );
        let out = client
            .create_task_confirmed(&CreateTaskInput {
                id: Some("aegis-runtime-slow-invisible-22222222".into()),
                title: "slow: invisible".into(),
                repo: "hercules".into(),
                body: "## Definition of Done".into(),
                priority: None,
                labels: vec![],
                adapter: Some("grok".into()),
            })
            .await
            .expect("no hard error");
        assert!(out.is_none(), "still unknown after the window");
    }

    #[test]
    fn task_id_is_deterministic_and_valid() {
        let a = task_id_for("did:aegis:owner", "Add --version flag!", "body", 1);
        let b = task_id_for("did:aegis:owner", "Add --version flag!", "body", 1);
        let c = task_id_for("did:aegis:owner", "Add --version flag!", "body", 2);
        let d = task_id_for("did:aegis:owner", "Add --version flag!", "other body", 1);
        assert_eq!(a, b);
        assert_ne!(a, c, "attempt 2 must mint a fresh id");
        assert_ne!(a, d);
        assert!(a.starts_with("aegis-runtime-add-version-flag-"), "{a}");
        assert!(
            a.chars()
                .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-'),
            "{a}"
        );
        assert!(a.len() <= 64, "{a}");
        let long = task_id_for("p", &"word ".repeat(40), "b", 1);
        assert!(long.len() <= 64, "{long}");
    }

    #[tokio::test]
    async fn get_task_maps_404_to_none_and_parses_camel_case() {
        let mock = mock::start().await;
        let client = HerculesClient::new(&mock.base, "test-token", false);
        assert!(
            client
                .get_task("gone")
                .await
                .expect("404 is Ok(None)")
                .is_none()
        );
        let task = client.get_task("t-1").await.unwrap().expect("task");
        assert_eq!(task.state, "review");
        assert_eq!(task.verified_sha.as_deref(), Some("abc1234"));
        assert_eq!(task.pr_url.as_deref(), Some("https://example.invalid/pr/1"));
    }

    #[tokio::test]
    async fn get_task_exposes_dispatch_hold_reason() {
        let mock = mock::start().await;
        let client = HerculesClient::new(&mock.base, "test-token", false);
        let held = client.get_task("t-held").await.unwrap().unwrap();
        assert_eq!(held.hold_code(), Some("repo_readiness_incomplete"));
        let free = client.get_task("t-1").await.unwrap().unwrap();
        assert_eq!(free.hold_code(), None, "missing field means not held");
    }

    #[tokio::test]
    async fn wrong_token_is_an_error_not_a_panic() {
        let mock = mock::start().await;
        let client = HerculesClient::new(&mock.base, "bad", false);
        let err = client.get_task("t-1").await.expect_err("401 must error");
        assert!(err.to_string().contains("401"), "{err:#}");
    }

    /// Contract check against a live Hercules (staging only). Run explicitly:
    /// `HERCULES_API_BASE=http://127.0.0.1:8788 HERCULES_TOKEN=… cargo test
    /// -- --ignored staging_roundtrip`. Refuses production bases.
    #[tokio::test]
    #[ignore]
    async fn staging_roundtrip() {
        let client = HerculesClient::from_env().expect("HERCULES_API_BASE/HERCULES_TOKEN");
        assert!(
            !client.is_production(),
            "never run the roundtrip against production"
        );
        // `HERCULES_E2E_TASK_FILE`: create exactly the task agreed with the
        // Hercules side (id/title/body/labels), skipping the draft step.
        if let Ok(path) = std::env::var("HERCULES_E2E_TASK_FILE") {
            let spec: serde_json::Value =
                serde_json::from_str(&std::fs::read_to_string(&path).expect("task file")).unwrap();
            let input = CreateTaskInput {
                id: spec["id"].as_str().map(|s| s.to_string()),
                title: spec["title"].as_str().unwrap().to_string(),
                repo: spec["repo"].as_str().unwrap_or("hercules").to_string(),
                body: spec["body"].as_str().unwrap().to_string(),
                priority: spec["priority"].as_i64(),
                labels: spec["labels"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|l| l.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default(),
                adapter: spec["adapter"].as_str().map(|s| s.to_string()),
            };
            let task = client.create_task(&input).await.expect("create from file");
            let fetched = client
                .get_task(&task.id)
                .await
                .expect("get")
                .expect("exists");
            eprintln!(
                "staging create ok: id={} state={}",
                fetched.id, fetched.state
            );
            return;
        }
        let draft = client
            .draft_from_intent(
                "Add a --version flag to the aegis_runtime_core binary",
                "aegis",
            )
            .await
            .expect("draft-from-intent");
        assert!(
            draft.body.contains("## Definition of Done"),
            "{}",
            draft.body
        );
        let lint = client.lint_acceptance(&draft.body).await.expect("lint");
        assert!(lint.ok, "{lint:?}");
        let task = client
            .create_task(&CreateTaskInput {
                id: Some(task_id_for("did:aegis:e2e", &draft.title, &draft.body, 1)),
                title: format!("[aegis-runtime e2e] {}", draft.title),
                repo: "aegis".into(),
                body: draft.body.clone(),
                priority: draft.priority,
                labels: vec!["source=aegis-runtime".into(), "e2e=true".into()],
                adapter: None,
            })
            .await
            .expect("create");
        assert!(!task.id.is_empty());
        let fetched = client
            .get_task(&task.id)
            .await
            .expect("get")
            .expect("exists");
        assert_eq!(fetched.id, task.id);
        assert!(!fetched.state.is_empty());
        eprintln!(
            "staging roundtrip ok: id={} state={}",
            fetched.id, fetched.state
        );
    }

    #[test]
    fn base_is_normalized_and_production_flag_kept() {
        let c = HerculesClient::new("http://127.0.0.1:8788/", "t", false);
        assert_eq!(c.base(), "http://127.0.0.1:8788");
        assert!(!c.is_production());
        assert!(HerculesClient::new("http://127.0.0.1:8787", "t", true).is_production());
    }

    #[test]
    fn delegation_flag_defaults_off() {
        // Unset in unit tests → OSS default. Parallel tests may set the env;
        // only assert the parser contract + from_env refuses without the flag.
        fn parse(v: &str) -> bool {
            let t = v.trim();
            t == "1" || t.eq_ignore_ascii_case("true") || t.eq_ignore_ascii_case("yes")
        }
        assert!(parse("1"));
        assert!(parse("true"));
        assert!(!parse("0"));
        assert!(!parse(""));
        if std::env::var("AEGIS_HERCULES_DELEGATION_ENABLED").is_err() {
            assert!(
                !HerculesClient::delegation_enabled(),
                "OSS default must keep Hercules delegation off"
            );
            assert!(
                HerculesClient::from_env().is_none(),
                "credentials alone must not enable Hercules when the flag is unset"
            );
        }
    }
}
