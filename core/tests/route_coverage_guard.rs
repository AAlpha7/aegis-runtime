//! Sensor 1 — Route-coverage ratchet.
//!
//! The recurring outage class this guards against (4-way audit 2026-07-08) is
//! "entry path exists, no test sees it": a new admin/webapp route lands with
//! zero router-level tests and its first exercise is production traffic.
//!
//! Mechanism: at test time we re-parse the ACTUAL route tables from
//! `core/src/admin.rs` source (`admin_router` + `oauth_callback_router` —
//! every `.route("…", …)` / `.nest("…", …)` / `.nest_service("…", …)`
//! registration) and demand, for every path, coverage EVIDENCE: some string
//! literal in `core/src/admin_tests.rs` matches the route template
//! SEGMENT-WISE (each `{param}` segment matches any non-empty candidate
//! segment; static segments must be equal; a trailing query string on the
//! candidate is ignored; `.nest`/`.nest_service` mounts match as a
//! segment-boundary prefix). Segment matching means a route that is a prefix
//! of a longer tested route — or a parameterized sibling sharing the same
//! stem — cannot borrow the longer route's evidence (Codex review 2026-07-08:
//! an earlier substring check false-covered 7 sibling routes).
//!
//! This is deliberately crude — its job is to catch NEW routes added with
//! zero router-level tests, not to prove test quality. Path-level granularity
//! only (a PUT and DELETE sharing one path count as covered together).
//!
//! `KNOWN_UNCOVERED` is the explicit exemption list. It only ever SHRINKS:
//! - adding a route without a test → FAIL (a) — write a router-level test or
//!   consciously exempt it here (a reviewed diff line, not a silent gap);
//! - deleting an exempted route → FAIL (b) — remove the stale exemption;
//! - adding a test for an exempted route → FAIL (c) — remove the exemption
//!   so the route can never silently lose coverage again (the ratchet).

// admin.rs was split into core/src/admin/ (Phase 0.1, 2026-07-11). The
// sensor scans the concatenation of every submodule so route registrations
// are seen wherever they live (routers sit in server.rs today, but a future
// re-slice must not blind this sensor).
const ADMIN_SRC: &str = concat!(
    include_str!("../src/admin/mod.rs"),
    include_str!("../src/admin/server.rs"),
    include_str!("../src/admin/ops_endpoints.rs"),
    include_str!("../src/admin/webapp_gateway.rs"),
    include_str!("../src/admin/debug_avatar.rs"),
    include_str!("../src/admin/vnext_routes.rs"),
    include_str!("../src/admin/settings_routes.rs"),
    include_str!("../src/admin/assistant.rs"),
    include_str!("../src/admin/inbox_projection.rs"),
    include_str!("../src/admin/vnext_projections.rs"),
    include_str!("../src/admin/console_handlers.rs"),
    include_str!("../src/admin/console_bridge.rs"),
    include_str!("../src/admin/diagnostics_routes.rs"),
    include_str!("../src/admin/remote_routes.rs"),
);
// admin_tests.rs was split into core/src/admin_tests/ (Phase 0.5, 2026-07-12);
// concatenate every submodule so the route-literal scan still sees the full
// test corpus. Add a submodule here if the split grows more parts.
const ADMIN_TESTS_SRC: &str = concat!(
    include_str!("../src/admin_tests/mod.rs"),
    include_str!("../src/admin_tests/settings_persistence.rs"),
    include_str!("../src/admin_tests/embedding_persistence.rs"),
    include_str!("../src/admin_tests/part1.rs"),
    include_str!("../src/admin_tests/part1_vnext_commands.rs"),
    include_str!("../src/admin_tests/part1_vnext_projections.rs"),
    include_str!("../src/admin_tests/part2.rs"),
    include_str!("../src/admin_tests/part2_memory_actions.rs"),
    include_str!("../src/admin_tests/part2_owner_work.rs"),
    include_str!("../src/admin_tests/part2_settings_providers.rs"),
    include_str!("../src/admin_tests/part3.rs"),
    include_str!("../src/admin_tests/part3_llm_settings.rs"),
    include_str!("../src/admin_tests/part3_vnext_runtime_commands.rs"),
    include_str!("../src/admin_tests/part3_web_hardening.rs"),
    include_str!("../src/admin_tests/part4.rs"),
    include_str!("../src/admin_tests/part5.rs"),
);

/// Routes with no router-level test evidence in admin_tests.rs.
/// Audited 2026-07-08 (S1 coverage-sensor audit). Sorted. Shrink-only —
/// see module docs. One line per route so removals are reviewable diffs.
const KNOWN_UNCOVERED: &[&str] = &[
    "/api/cost-summary",                                // audited 2026-07-08
    "/auth/google/setup",                               // audited 2026-07-08
    "/auth/google/status",                              // audited 2026-07-08
    "/auth/google/store",                               // audited 2026-07-08
    "/cost-dashboard",                                  // audited 2026-07-08
    "/health/memory",                                   // audited 2026-07-08
    "/memory_status",                                   // audited 2026-07-08
    "/skills/registry",                                 // audited 2026-09-05
    "/skills/registry/{id}",                            // audited 2026-09-05
    "/skills/registry/{id}/deactivate",                 // audited 2026-09-05
    "/webapp-debug-leases",                             // audited 2026-07-08
    "/webapp-debug-snapshots",                          // audited 2026-07-08
    "/webapp-debug-snapshots/{snapshot_id}",            // audited 2026-07-08
    "/webapp/vnext/avatar-image/{filename}",            // audited 2026-07-08
    "/webapp/vnext/avatar-profile/auto-refresh",        // audited 2026-07-08
    "/webapp/vnext/avatar-profile/generate-aegis",      // audited 2026-07-08
    "/webapp/vnext/debug-jobs/{job_id}",                // audited 2026-07-08
    "/webapp/vnext/settings/codex/model",               // audited 2026-07-08
    "/webapp/vnext/settings/codex/poll-device-code",    // audited 2026-07-08
    "/webapp/vnext/settings/codex/start-device-code",   // audited 2026-07-08
    "/webapp/vnext/settings/codex/status",              // audited 2026-07-08
    "/webapp/vnext/settings/codex/stop-using-for-chat", // audited 2026-07-08
    "/webapp/vnext/settings/codex/use-for-chat",        // audited 2026-07-08
    "/webapp/vnext/settings/display-language",          // audited 2026-07-08
    "/webapp/vnext/settings/grok/poll-device-code",     // audited 2026-07-08
    "/webapp/vnext/settings/grok/revoke",               // audited 2026-07-08
    "/webapp/vnext/settings/grok/start-device-code",    // audited 2026-07-08
    "/webapp/vnext/settings/grok/status",               // audited 2026-07-08
    "/webapp/vnext/settings/grok/stop-using-for-chat",  // audited 2026-07-08
    "/webapp/vnext/settings/grok/use-for-chat",         // audited 2026-07-08
    "/webapp/vnext/settings/paid-api-fallback",         // audited 2026-07-08
    "/webapp/vnext/settings/web-push/status",           // audited 2026-07-08
    "/webapp/vnext/settings/web-push/subscribe",        // audited 2026-07-08
    "/webapp/vnext/settings/web-push/test",             // audited 2026-07-08
    "/webapp/vnext/settings/web-push/unsubscribe",      // audited 2026-07-08
];

/// A registered route path plus whether it was mounted as a prefix
/// (`.nest` / `.nest_service`) rather than an exact `.route`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct RegisteredPath {
    path: String,
    is_prefix_mount: bool,
}

/// Extract every string-literal path registered via `.route("…")`,
/// `.nest("…")` or `.nest_service("…")` in the given source. Tolerates the
/// multi-line rustfmt form (`.route(\n    "path",\n    handler,\n)`).
/// Non-string registrations (e.g. `TelegramNotificationRouter::route(expr)`)
/// are skipped because the next non-whitespace char is not a quote.
fn extract_route_paths(source: &str) -> Vec<RegisteredPath> {
    let mut out: Vec<RegisteredPath> = Vec::new();
    for (marker, is_prefix_mount) in [
        (".route(", false),
        (".nest(", true),
        (".nest_service(", true),
    ] {
        let mut search_from = 0;
        while let Some(pos) = source[search_from..].find(marker) {
            let after = search_from + pos + marker.len();
            let rest = source[after..].trim_start();
            if let Some(quoted) = rest.strip_prefix('"')
                && let Some(end) = quoted.find('"')
            {
                out.push(RegisteredPath {
                    path: quoted[..end].to_string(),
                    is_prefix_mount,
                });
            }
            search_from = after;
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Extract every string literal from Rust source with a crude escape-aware
/// scanner, keeping only path-like ones (starting with '/'). Misparses around
/// exotic literals (raw strings) merely yield garbage candidates that match
/// no route template — they cannot fabricate evidence unless the garbage IS
/// segment-for-segment a registered path, in which case the surrounding text
/// contains that path anyway.
fn extract_path_literals(source: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes = source.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'"' {
            let mut j = i + 1;
            let mut buf = String::new();
            while j < bytes.len() {
                match bytes[j] {
                    b'\\' => {
                        // Escaped char: keep a placeholder so segment content
                        // stays non-empty but never equals a real segment.
                        buf.push('\u{FFFD}');
                        j += 2;
                    }
                    b'"' => break,
                    _ => {
                        buf.push(bytes[j] as char);
                        j += 1;
                    }
                }
            }
            if buf.starts_with('/') {
                out.push(buf);
            }
            i = j + 1;
        } else {
            i += 1;
        }
    }
    out
}

/// Segment-wise route-template match (Codex review 2026-07-08: substring
/// evidence let prefix-sharing routes borrow each other's coverage).
/// - candidate query string is ignored;
/// - exact routes need the SAME segment count; prefix mounts (`.nest`) match
///   any candidate with at least the mount's segments;
/// - `{param}` template segments match any non-empty candidate segment
///   (including a `format!` placeholder like `{}` or `{draft_id}`);
/// - static segments must be byte-equal.
fn template_matches(route: &RegisteredPath, candidate: &str) -> bool {
    let candidate_path = candidate.split('?').next().unwrap_or(candidate);
    let template_segs: Vec<&str> = route.path.split('/').collect();
    let candidate_segs: Vec<&str> = candidate_path.split('/').collect();
    if route.is_prefix_mount {
        if candidate_segs.len() < template_segs.len() {
            return false;
        }
    } else if template_segs.len() != candidate_segs.len() {
        return false;
    }
    template_segs
        .iter()
        .zip(candidate_segs.iter())
        .all(|(t, c)| {
            if t.starts_with('{') && t.ends_with('}') {
                !c.is_empty()
            } else {
                t == c
            }
        })
}

/// Number of files baked into `ADMIN_SRC` above. Bump BOTH together.
const ADMIN_SRC_FILE_COUNT: usize = 14;

#[test]
fn route_coverage_ratchet() {
    // Completeness self-check: a new core/src/admin/ submodule added without
    // extending the ADMIN_SRC concat! list would silently blind this sensor
    // to any routes registered there.
    let admin_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/admin");
    let on_disk = std::fs::read_dir(&admin_dir)
        .expect("read src/admin")
        .filter(|entry| {
            entry
                .as_ref()
                .is_ok_and(|e| e.path().extension().is_some_and(|ext| ext == "rs"))
        })
        .count();
    assert_eq!(
        on_disk, ADMIN_SRC_FILE_COUNT,
        "src/admin/ has {on_disk} .rs files but ADMIN_SRC includes \
         {ADMIN_SRC_FILE_COUNT} — extend the concat! list (and this count) so \
         the route sensor sees every submodule"
    );

    let routes = extract_route_paths(ADMIN_SRC);
    let literals = extract_path_literals(ADMIN_TESTS_SRC);
    let is_evidenced =
        |route: &RegisteredPath| literals.iter().any(|lit| template_matches(route, lit));

    // Parser self-checks: if either extraction regresses (rustfmt layout
    // change, marker rename), assertion (a) would pass vacuously. admin.rs
    // registers well over 100 paths today and admin_tests.rs exercises
    // hundreds of path literals.
    assert!(
        routes.len() >= 100,
        "route extraction found only {} paths — the source parser in this \
         test has likely rotted against admin.rs's current layout",
        routes.len()
    );
    assert!(
        literals.len() >= 100,
        "literal extraction found only {} path literals in admin_tests.rs — \
         the string scanner in this test has likely rotted",
        literals.len()
    );

    // Exemption-list hygiene: sorted + unique so diffs stay reviewable.
    let mut sorted = KNOWN_UNCOVERED.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(
        KNOWN_UNCOVERED,
        sorted.as_slice(),
        "KNOWN_UNCOVERED must be sorted and free of duplicates"
    );

    let exempt: std::collections::HashSet<&str> = KNOWN_UNCOVERED.iter().copied().collect();
    let route_set: std::collections::HashSet<&str> =
        routes.iter().map(|r| r.path.as_str()).collect();

    // (a) Every registered route is either evidenced or explicitly exempted.
    let new_uncovered: Vec<&str> = routes
        .iter()
        .filter(|r| !exempt.contains(r.path.as_str()) && !is_evidenced(r))
        .map(|r| r.path.as_str())
        .collect();
    assert!(
        new_uncovered.is_empty(),
        "NEW route(s) without any router-level test in admin_tests.rs:\n  {}\n\
         Add a router-level test (oneshot through admin_router / \
         oauth_callback_router) that exercises the path, or — as a conscious, \
         reviewed decision — add the path to KNOWN_UNCOVERED in this file.",
        new_uncovered.join("\n  ")
    );

    // (b) No stale exemptions: every exempted path must still be a registered
    // route (renamed/deleted routes must drop out of the list).
    let stale: Vec<&str> = KNOWN_UNCOVERED
        .iter()
        .copied()
        .filter(|p| !route_set.contains(p))
        .collect();
    assert!(
        stale.is_empty(),
        "Stale KNOWN_UNCOVERED entries (no longer registered as routes):\n  {}\n\
         Remove them from KNOWN_UNCOVERED.",
        stale.join("\n  ")
    );

    // (c) RATCHET: every exemption must still be UN-evidenced. The moment a
    // router-level test lands for an exempted route, the exemption must be
    // deleted so the route can never silently lose coverage again.
    let now_covered: Vec<&str> = routes
        .iter()
        .filter(|r| exempt.contains(r.path.as_str()) && is_evidenced(r))
        .map(|r| r.path.as_str())
        .collect();
    assert!(
        now_covered.is_empty(),
        "These KNOWN_UNCOVERED routes now HAVE test evidence — ratchet them:\n  {}\n\
         Delete the entries from KNOWN_UNCOVERED (the list only shrinks).",
        now_covered.join("\n  ")
    );
}

#[test]
fn template_matcher_semantics() {
    let route = |path: &str, prefix: bool| RegisteredPath {
        path: path.to_string(),
        is_prefix_mount: prefix,
    };
    // Exact static route: query ignored, prefix-of-longer rejected.
    assert!(template_matches(
        &route("/webapp/vnext/work/inbox", false),
        "/webapp/vnext/work/inbox?limit=25"
    ));
    assert!(!template_matches(
        &route("/webapp/vnext/work", false),
        "/webapp/vnext/work/inbox"
    ));
    // Param segments: real ids and format! placeholders both count; a
    // parameterized sibling with a different tail cannot borrow evidence.
    assert!(template_matches(
        &route(
            "/webapp/vnext/work/message/{account}/{id}/read-state",
            false
        ),
        "/webapp/vnext/work/message/{}/{}/read-state"
    ));
    assert!(!template_matches(
        &route("/webapp/vnext/work/message/{account}/{id}/star", false),
        "/webapp/vnext/work/message/{}/{}/read-state"
    ));
    assert!(!template_matches(
        &route("/webapp/vnext/work/message/{account}/{id}", false),
        "/webapp/vnext/work/message/{}/{}/read-state"
    ));
    // Prefix mounts (.nest / .nest_service) match at segment boundaries only.
    assert!(template_matches(
        &route("/calendar", true),
        "/calendar/watch/push"
    ));
    assert!(!template_matches(
        &route("/calendar", true),
        "/calendarish/thing"
    ));
}
