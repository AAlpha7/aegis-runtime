use super::*;
use crate::remote_access;

// Bound unauthenticated work without trusting spoofable proxy/IP headers.
// The key is installation-controlled, not supplied by the request.
fn allow_attempt(scope: &str, timestamp: u64) -> bool {
    static LIMITS: OnceLock<parking_lot::Mutex<HashMap<String, (u64, u32)>>> = OnceLock::new();
    let mut limits = LIMITS.get_or_init(Default::default).lock();
    limits.retain(|_, (started, _)| timestamp.saturating_sub(*started) < 60);
    if !limits.contains_key(scope) && limits.len() >= 128 {
        return false;
    }
    let entry = limits.entry(scope.to_owned()).or_insert((timestamp, 0));
    if entry.1 >= 90 {
        return false;
    }
    entry.1 += 1;
    true
}

pub(super) fn router() -> Router<Arc<AdminState>> {
    Router::new()
        .route("/access", get(page))
        .route("/access.js", get(script))
        .route("/access.css", get(styles))
        .route("/auth/passkey/{action}", post(action))
        .layer(DefaultBodyLimit::max(32 * 1024))
}

async fn page() -> impl IntoResponse {
    axum::response::Html(include_str!("../../assets/access.html"))
}

async fn script() -> impl IntoResponse {
    (
        [(
            axum::http::header::CONTENT_TYPE,
            "text/javascript; charset=utf-8",
        )],
        include_str!("../../assets/access.js"),
    )
}

async fn styles() -> impl IntoResponse {
    (
        [(axum::http::header::CONTENT_TYPE, "text/css; charset=utf-8")],
        concat!(
            include_str!("../../../webapp/src/vnext/design/theme.css"),
            include_str!("../../assets/access.css")
        ),
    )
}

async fn action(
    State(state): State<Arc<AdminState>>,
    Path(action): Path<String>,
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> Result<Response, (StatusCode, String)> {
    let origin = state.public_url.clone().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Remote access is not configured".into(),
    ))?;
    let parsed = remote_access::origin_url(&origin).map_err(|_| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            "Invalid remote origin configuration".into(),
        )
    })?;
    if headers.get("origin").and_then(|v| v.to_str().ok())
        != Some(parsed.origin().ascii_serialization().as_str())
    {
        return Err((StatusCode::FORBIDDEN, "Origin rejected".into()));
    }
    let db = state.db_path.clone().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Session store unavailable".into(),
    ))?;
    if !allow_attempt(&db, now()) {
        return Err((
            StatusCode::TOO_MANY_REQUESTS,
            "Too many attempts; retry in a minute".into(),
        ));
    }
    if action == "revoke-sessions" {
        let session = super::server::web_console_session_from_request(&state, &headers)
            .await
            .ok_or((StatusCode::UNAUTHORIZED, "Sign in first".into()))?;
        super::server::require_web_console_csrf(&headers, &session)?;
        if session.google_sub != web_console_auth::LOCAL_LOGIN_SUB
            || session.mfa_level != web_console_auth::MfaLevel::WebAuthn
            || !session
                .mfa_verified_at
                .is_some_and(|t| t <= now() && now() - t < 300)
        {
            return Err((
                StatusCode::FORBIDDEN,
                "Verify your Passkey again first".into(),
            ));
        }
    }
    let result = tokio::task::spawn_blocking(
        move || -> anyhow::Result<(serde_json::Value, Option<String>)> {
            let conn = crate::encrypted_sqlite::open(db)?;
            let id = body
                .get("challenge_id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let credential = body
                .get("credential")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            match action.as_str() {
                "register-start" => Ok((
                    remote_access::register_start(
                        &conn,
                        &origin,
                        body.get("pairing_code")
                            .and_then(|v| v.as_str())
                            .unwrap_or(""),
                        now(),
                    )?,
                    None,
                )),
                "register-finish" => {
                    remote_access::register_finish(&conn, &origin, id, credential, now())?;
                    Ok((serde_json::json!({"registered": true}), None))
                }
                "login-start" => Ok((remote_access::login_start(&conn, &origin, now())?, None)),
                "login-finish" => {
                    let created =
                        remote_access::login_finish(&conn, &origin, id, credential, now())?;
                    let cookie = web_console_auth::build_session_cookie_header(
                        &web_console_auth::WebConsoleLoginConfig::from_env(),
                        &created.cookie_token,
                    )?;
                    Ok((serde_json::json!({"authenticated": true}), Some(cookie)))
                }
                "revoke-sessions" => {
                    remote_access::revoke_sessions(&conn, now())?;
                    Ok((
                        serde_json::json!({"revoked": true}),
                        Some(web_console_auth::build_clear_session_cookie_header(
                            &web_console_auth::WebConsoleLoginConfig::from_env(),
                        )?),
                    ))
                }
                _ => anyhow::bail!("unknown action"),
            }
        },
    )
    .await
    .map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Authentication unavailable".into(),
        )
    })?
    .map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            "Authentication failed. Retry, or obtain a new pairing code on your computer.".into(),
        )
    })?;
    let mut response = Json(result.0).into_response();
    response.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store"),
    );
    if let Some(cookie) = result.1 {
        response.headers_mut().insert(
            axum::http::header::SET_COOKIE,
            cookie.parse().map_err(|_| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Session unavailable".into(),
                )
            })?,
        );
    }
    Ok(response)
}

fn now() -> u64 {
    chrono::Utc::now().timestamp().max(0) as u64
}

#[cfg(test)]
mod tests {
    #[test]
    fn attempts_are_bounded_and_recover_after_window() {
        for _ in 0..90 {
            assert!(super::allow_attempt("rate-test-installation", 100));
        }
        assert!(!super::allow_attempt("rate-test-installation", 159));
        assert!(super::allow_attempt("rate-test-installation", 160));
    }
}
