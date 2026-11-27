#!/usr/bin/env bash
# deploy.sh — Capistrano-style production deployment for Aegis Runtime.
#
# Usage:
#   deploy.sh install   — First-time server setup
#   deploy.sh upgrade   — Deploy new release directory (backup → release → symlink → restart)
#   deploy.sh update    — In-place update (backup → stop → replace binary & skills → restart)
#   deploy.sh webapp-update — Frontend-only update (build → atomically sync WebApp assets; no service restart)
#   deploy.sh auto      — Choose the cheapest safe deploy layer from changes since last successful deploy
#   deploy.sh deploy-plan — Read-only recommendation for webapp-update vs update
#   deploy.sh rollback  — Revert to previous release
#   deploy.sh verify    — Dry-run: check all 4 deploy gates, then exit 0
#   deploy.sh backup    — Safe SQLite backup
#   deploy.sh status    — Show current deployment info
#
# Directory layout:
#   /opt/aegis-runtime/
#   ├── current -> releases/20260228-143022   (symlink)
#   ├── releases/                             (versioned binaries)
#   ├── shared/                               (persistent: .env, *.key, *.db)
#   └── backups/                              (database backups)
#
# Deploy gates (Plan §11.0, P0-D): destructive commands (install, upgrade,
# update, rollback, verify) require all 4 checks to pass before proceeding:
#   1. CWD is the canonical main worktree (override: AEGIS_RUNTIME_CANONICAL_REPO)   — exit 10
#   2. HEAD is on branch `main`                                   — exit 11
#   3. Working tree is clean (no uncommitted changes)             — exit 12
#   4. Local main == origin/main after fetch                      — exit 13
# See .claude/coordination/DEPLOY_RULES.md for remedy steps.

set -euo pipefail

# --- DEPLOY_SH_VERSION (bump on every meaningful change to gate logic or deploy flow) ---
# Recorded in every deploy-log.jsonl line so regressions can be mapped back
# to a specific deploy.sh revision.
#
# Changelog:
#   v1: P0-D initial (4 gates + verify + version header)
#   v2: P0.6 deploy-log.jsonl emission + EXIT trap (changes deploy flow:
#       every destructive command now logs exactly-once outcome)
#   v3: P2 Batch A — migration hard-lock (§11.5) on cmd_update when
#       core/src/memory_migrations.rs differs from last-deployed sha;
#       pre-deploy redundant-sha skip (§11.1 Phase C motivation): consult
#       last 10 deploy-log entries, no-op if same sha deployed ≤5 min ago.
#   v4: P2 Batch D (trimmed to §11.4 + §B.3 only, rest covered by Batch A/C):
#       - §11.4 hold-on-failure: on runtime deploy failure, drop a
#         persistent failure sentinel. Subsequent destructive deploys refuse
#         until a human clears it via `unlock` or a successful `rollback`.
#         This is the "deploy lock held through failure" invariant: not
#         literally the flock fd (which a single bash invocation cannot
#         keep held past exit), but a filesystem sentinel that reinstates
#         the effect across invocations.
#       - §B.3 post-deploy 5-minute health monitor: after a successful
#         upgrade/update, sample `systemctl is-active` + optional loopback
#         health endpoint every N seconds for 5 minutes. If ≥ threshold
#         samples fail (flap detection), mark the deploy as failed AFTER
#         the fact: drop the failure sentinel, broadcast + warn.
#       - EXIT trap discriminates gate-reject (exit codes 10-14) vs runtime
#         failure (everything else non-zero): gate-reject writes status
#         "rejected_gate", does NOT drop the failure sentinel; runtime
#         failure writes status "failed" and drops the sentinel.
#   v5: Post-deploy checklist MVP (c99735b, 2026-04-14): after §B.3 health
#       monitor passes on upgrade/update, invoke scripts/post-deploy-checklist.sh
#       to run advisory YAML-driven checks (journalctl grep, memory reads,
#       systemctl is-active, LLM usage recency). Checklist failures do NOT
#       block the deploy — §B.3 is the hard gate — but they surface via
#       bark push + deploy-log.jsonl so operators notice drift.
#       (Changelog entry added retroactively by v6 — v5 itself shipped without
#       updating this header.)
#   v6: Artifact path + freshness fix (2026-04-17 regression):
#       - build_release now exports DEPLOY_BUILD_OUTPUT_DIR (=
#         $CARGO_TARGET_DIR/release) and DEPLOY_BUILD_START_EPOCH, then
#         asserts the built binary's mtime ≥ build-start epoch. Catches
#         cargo no-op / stale-cache short-circuits that previously exited 0
#         with no fresh artifact produced.
#       - find_artifact preference order now matches build_release:
#         (1) tarball → (2) DEPLOY_BUILD_OUTPUT_DIR → (3)
#         AEGIS_RUNTIME_BUILD_CACHE/release → (4) legacy target/deploy/release
#         + target/release only if ≤2h old + loud warn. Fixes the silent-
#         stale-binary bug where CARGO_TARGET_DIR moved to build-cache but
#         find_artifact still read from target/deploy/release, shipping a
#         5-day-old binary on every deploy without anyone noticing.
#   v7: cmd_update hardening (2026-04-17 post-v6 audit):
#       - Pre-stop source validation: validate_binary fires on the
#         BUILD-OUTPUT artifact BEFORE systemctl stop, so a corrupted
#         cargo output can't take down production. cmd_upgrade also
#         uses the shared helper.
#       - Atomic binary swap: cp writes to $BINARY_NAME.new, validates
#         the staged copy, then mv -Tf swaps AFTER services stop. A kill
#         mid-cp no longer leaves a partial live binary (was a crashloop
#         risk — observed pattern on other services).
#       - Service-file updates moved BEFORE stop. daemon-reload is pure
#         state-reconcile; the stop→restart window only covers the
#         atomic rename, not file-system copies.
#       - aegis-executor binary now gets the same validate_binary check
#         (previously only aegis_runtime_core was validated — corrupted
#         executor binaries could ship unchecked).
#   v8: WebApp-only deploy fast path (2026-05-12):
#       - Add `webapp-update` for frontend-only changes. It keeps gates,
#         deploy lock, deploy logging, and production smoke, but skips Rust
#         release build, binary/skill replacement, DB backups, migrations,
#         service-file writes, and service restarts.
#       - Extract WebApp build + atomic static asset sync helpers so full
#         `update` and fast `webapp-update` share the same asset handling.
#   v9: Deploy layer guidance + duplicate WebApp build removal (2026-05-23):
#       - Add `deploy-plan`, a read-only classifier that recommends
#         webapp-update/update/upgrade from commits since last successful
#         deploy.
#       - `update` no longer rebuilds WebApp a second time after build_release;
#         full deploys still include WebApp assets via the shared build helper.
#       - Help output now shows the deployment layers explicitly so operators
#         pick the cheapest safe path.
#   v11: Phantom-success fix (2026-07-02): a deploy whose wrapper/build was
#       SIGTERM'd logged status "success" with no restart and a stale binary,
#       and the bogus entry then blocked the retry via the 300s redundant-
#       dedup window. Invariant flip: the EXIT trap never writes "success" —
#       only the explicit completion point at the bottom of the script does
#       (after §B.3 for update/upgrade); trap rc=0 without a record now logs
#       "exited_unrecorded". New TERM/INT/HUP trap records "aborted_signal";
#       it drops the §11.4 sentinel only when DEPLOY_DESTRUCTIVE_PHASE was
#       entered (service stop / payload activation / live-dir mutation) —
#       build-phase aborts are provably non-destructive and stay retryable.
#   v10: Deploy layer auto-routing (2026-05-24):
#       - Add `auto`, a gate-protected deploy command that resolves the
#         cheapest safe layer from deploy-plan and then runs no-deploy,
#         webapp-update, or update.
#       - `update` auto-routes WebApp-only diffs to webapp-update unless
#         AEGIS_RUNTIME_FORCE_FULL_UPDATE=1 is set.
#       - `webapp-update` refuses runtime/core diffs unless explicitly
#         overridden with AEGIS_RUNTIME_ALLOW_PARTIAL_WEBAPP_DEPLOY=1.
#       - Post-deploy health monitor defaults are now risk-tiered:
#         standard runtime deploys sample for 120s; migration/unknown-risk
#         deploys keep the 300s strict monitor.
#       - If `sccache` is installed, deploy Rust builds use it automatically
#         only when AEGIS_RUNTIME_USE_SCCACHE=1 (default off since 2026-09-04).
# v12 - Tier-2 full-suite freshness preflight (three-tier test strategy,
#       2026-07-10; refuse since 2026-09-24): Rust-shipping deploys check
#       the nightly/manual full-suite green stamp. Missing, older than 24h,
#       or not an ancestor of HEAD refuses the deploy.
#       AEGIS_RUNTIME_SKIP_FULL_SUITE_CHECK is refused. AEGIS_RUNTIME_TIER2_INLINE=1
#       may run the lib suite and proceed only when that suite is green.
# v14 - Adaptive host + auth-first install (2026-09-20, owner review of the
#       Cursor Cloud OSS deploy):
#       - detect_service_manager() runs before the worktree gates for every
#         lifecycle command. No usable systemd → install/upgrade/update/auto
#         exec scripts/first-run.sh (portable checkout-root layout under
#         scripts/aegis-runtime-local.sh), rollback/verify/webapp-update/tunnel
#         refuse with EXIT_NO_SYSTEMD (15). `status` shows whichever
#         supervisor applies. Override: AEGIS_RUNTIME_SERVICE_MANAGER.
#       - cmd_install starts with preflight_llm_auth: no grok/Codex/API-key
#         sign-in for the service user → exit EXIT_AUTH_PREFLIGHT (16) before
#         any build. After the release lands it binds owner chat via
#         `aegis_runtime_core setup-auth` as the service user, before the first
#         `systemctl start`.
DEPLOY_SH_VERSION=14

# ── Mutual-exclusion plumbing ───────────────────────────────────────
# Defined here but called at dispatch time (after gates). Gates run
# BEFORE the lock so a wrong-worktree/wrong-branch attempt doesn't even
# touch the lock file. Per Plan §15.8 + P0-D.
LOCK_DIR="${DEPLOY_ROOT:-/opt/aegis-runtime}"
LOCK_FILE="$LOCK_DIR/.deploy.lock"

acquire_deploy_lock() {
    mkdir -p "$LOCK_DIR" 2>/dev/null || true

    local script_dir
    script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
    local lock_helper="$script_dir/../.claude/hooks/lib/lock-acquire.sh"

    if [ -r "$lock_helper" ]; then
        # shellcheck source=../.claude/hooks/lib/lock-acquire.sh
        if ! source "$lock_helper" "$LOCK_FILE" 300; then
            echo "Another deploy is in progress (holder still alive after 300s wait)." >&2
            echo "  Investigate: fuser $LOCK_FILE" >&2
            exit 1
        fi
    else
        # Fallback to inline logic (helper missing — legacy deploy / fresh clone).
        touch "$LOCK_FILE" 2>/dev/null || true
        exec 200>"$LOCK_FILE"
        if ! flock -n 200; then
            local holder_pids alive
            holder_pids=$(fuser "$LOCK_FILE" 2>/dev/null | tr -s ' ' || echo "")
            alive=false
            for pid in $holder_pids; do
                if [ "$pid" != "$$" ] && kill -0 "$pid" 2>/dev/null; then
                    alive=true
                    break
                fi
            done
            if $alive; then
                echo "Another deploy (PID $holder_pids) is running. Aborting." >&2
                exit 1
            fi
            echo "Stale lock detected (holder process gone). Reclaiming." >&2
            exec 200>&-
            exec 200>"$LOCK_FILE"
            if ! flock -n 200; then
                echo "Cannot acquire lock even after stale detection. Aborting." >&2
                exit 1
            fi
        fi
    fi
}

# ── Configuration ────────────────────────────────────────────────────
DEPLOY_ROOT="${AEGIS_RUNTIME_DEPLOY_ROOT:-/opt/aegis-runtime}"
SERVICE_NAME="aegis-runtime"
SERVICE_USER="aegis-runtime"
BINARY_NAME="aegis_runtime_core"
EXECUTOR_BINARY="aegis-executor"
BACKUP_BINARY="aegis_runtime_backup_db"
ENCRYPT_BINARY="aegis_runtime_encrypt_db"
TUNNEL_SERVICE="aegis-runtime-cloudflare"
TUNNEL_URL_FILE="$DEPLOY_ROOT/shared/tunnel_url"
KEEP_RELEASES=5
KEEP_BACKUPS=10
BUILD_USER="${AEGIS_RUNTIME_BUILD_USER:-ubuntu}"

# ── Colors ───────────────────────────────────────────────────────────
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m'

info()  { echo -e "${CYAN}[deploy]${NC} $*"; }
ok()    { echo -e "${GREEN}[deploy]${NC} $*"; }
warn()  { echo -e "${YELLOW}[deploy]${NC} $*"; }
die()   { echo -e "${RED}[deploy]${NC} $*" >&2; exit 1; }

# ── owner notification (Telegram) ───────────────────────────────────
# Best-effort: reads TELOXIDE_TOKEN / ADMIN_CHAT_ID from the shared env
# file (root-readable); never prints the token. Used when a post-deploy
# smoke fails and the deploy auto-rolls back.
notify_owner() {
    local text="$1" env_file="$DEPLOY_ROOT/shared/.env" token chat
    [ -r "$env_file" ] || return 0
    token="$(grep -E '^TELOXIDE_TOKEN=' "$env_file" | head -1 | cut -d= -f2- | tr -d '"' | tr -d "'")"
    chat="$(grep -E '^ADMIN_CHAT_ID=' "$env_file" | head -1 | cut -d= -f2- | tr -d '"' | tr -d "'")"
    [ -n "$token" ] && [ -n "$chat" ] || return 0
    curl -s -m 10 -o /dev/null -X POST "https://api.telegram.org/bot${token}/sendMessage" \
        --data-urlencode "chat_id=${chat}" --data-urlencode "text=${text}" || true
}

# ── Helpers ──────────────────────────────────────────────────────────
require_root() {
    if [ "$(id -u)" -ne 0 ]; then
        die "This command requires root. Run with sudo."
    fi
}

current_release() {
    if [ -L "$DEPLOY_ROOT/current" ]; then
        basename "$(readlink -f "$DEPLOY_ROOT/current")"
    else
        echo "(none)"
    fi
}

# ── Host capability: which service manager can actually run the service? ──
# Everything below `cmd_install`/`cmd_update`/… assumes the /opt/aegis-runtime
# release layout supervised by systemd units. On Cursor Cloud, containers
# whose PID 1 is `tini`, macOS, or WSL without systemd, `systemctl` either
# does not exist or reports `offline`; pretending otherwise used to end in a
# dead `systemctl enable` and a manual fallback to the README. Detect first.
#
# Sets SERVICE_MANAGER (`systemd` | `none`) and SERVICE_MANAGER_REASON.
# Plain variables, not stdout: a `$(…)` caller would lose the reason.
# Override: AEGIS_RUNTIME_SERVICE_MANAGER=auto|systemd|local (local == none).
SERVICE_MANAGER=""
SERVICE_MANAGER_REASON=""
detect_service_manager() {
    local mode="${AEGIS_RUNTIME_SERVICE_MANAGER:-auto}"
    case "$mode" in
        systemd)
            SERVICE_MANAGER=systemd
            SERVICE_MANAGER_REASON="forced by AEGIS_RUNTIME_SERVICE_MANAGER=systemd"
            return 0 ;;
        local|none)
            SERVICE_MANAGER=none
            SERVICE_MANAGER_REASON="forced by AEGIS_RUNTIME_SERVICE_MANAGER=$mode"
            return 0 ;;
        auto) ;;
        *)
            die "AEGIS_RUNTIME_SERVICE_MANAGER must be auto, systemd, or local (got: $mode)" ;;
    esac
    if ! command -v systemctl >/dev/null 2>&1; then
        SERVICE_MANAGER=none
        SERVICE_MANAGER_REASON="systemctl is not on PATH"
        return 0
    fi
    # sd_booted(): systemd is PID 1 iff /run/systemd/system exists.
    if [ ! -d /run/systemd/system ]; then
        SERVICE_MANAGER=none
        SERVICE_MANAGER_REASON="/run/systemd/system is missing (PID 1 is $(ps -o comm= -p 1 2>/dev/null || echo unknown), not systemd)"
        return 0
    fi
    local state
    state="$(systemctl is-system-running 2>/dev/null || true)"
    case "$state" in
        offline|unknown|"")
            SERVICE_MANAGER=none
            SERVICE_MANAGER_REASON="systemctl is-system-running = ${state:-<no output>}"
            return 0 ;;
    esac
    # running | degraded | starting | initializing | maintenance | stopping:
    # units can be managed (degraded only means some unrelated unit failed).
    SERVICE_MANAGER=systemd
    SERVICE_MANAGER_REASON="systemctl is-system-running = $state"
}

# Non-systemd host: send the operator to the portable path instead of
# failing half-way through `systemctl enable`. `install`/`upgrade`/`update`/
# `auto` become the auth-first first-run flow (checkout-root layout +
# nohup supervisor); `status` shows that supervisor; the rest have no
# meaning without release history or units and refuse with EXIT_NO_SYSTEMD.
route_without_systemd() {
    local action="$1"
    local script_dir
    script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
    warn "systemd is not usable on this host ($SERVICE_MANAGER_REASON)."
    warn "The /opt/aegis-runtime release layout + units are systemd-only; using the portable path instead."
    case "$action" in
        install|upgrade|update|auto)
            [ -x "$script_dir/first-run.sh" ] || die "scripts/first-run.sh is missing — cannot fall back"
            info "→ exec scripts/first-run.sh (choose provider → sign in → build → bind chat → start under scripts/aegis-runtime-local.sh)"
            info "  Force the systemd path with AEGIS_RUNTIME_SERVICE_MANAGER=systemd if this detection is wrong."
            exec "$script_dir/first-run.sh"
            ;;
        *)
            reject_gate "$EXIT_NO_SYSTEMD" "\
REASON: '$action' needs the systemd release layout, which this host cannot run.
  Detected: $SERVICE_MANAGER_REASON

RULE: deploy.sh only manages systemd units under $DEPLOY_ROOT. Without
  systemd there is no release history to roll back, no unit to verify,
  and no tunnel unit.

TO FIX (portable path):
  scripts/first-run.sh                      # auth-first setup, build, bind chat, start
  scripts/aegis-runtime-local.sh status|logs|restart|stop
  (override: AEGIS_RUNTIME_SERVICE_MANAGER=systemd sudo $0 $action)"
            ;;
    esac
}

restart_service() {
    if systemctl is-active --quiet "$SERVICE_NAME" 2>/dev/null; then
        info "Restarting $SERVICE_NAME..."
        systemctl restart "$SERVICE_NAME" 200>&-
    elif systemctl is-enabled --quiet "$SERVICE_NAME" 2>/dev/null; then
        info "Starting $SERVICE_NAME..."
        systemctl start "$SERVICE_NAME"
    else
        warn "systemd service not enabled. Start manually or run: systemctl enable --now $SERVICE_NAME"
    fi
}

restart_tunnel() {
    if systemctl is-enabled --quiet "$TUNNEL_SERVICE" 2>/dev/null; then
        info "Restarting $TUNNEL_SERVICE..."
        systemctl restart "$TUNNEL_SERVICE"
        # Wait for URL capture (up to 20s)
        for _ in $(seq 1 20); do
            if [ -f "$TUNNEL_URL_FILE" ] && [ -s "$TUNNEL_URL_FILE" ]; then
                ok "Tunnel URL: $(cat "$TUNNEL_URL_FILE")"
                return 0
            fi
            sleep 1
        done
        warn "Tunnel URL not captured within 20s — check: journalctl -u $TUNNEL_SERVICE -f"
    fi
}

prepare_webapp_build_dir() {
    local webapp_dir="${1:-$PROJECT_ROOT/webapp}"
    local build_user="${2:-$BUILD_USER}"
    local dist_dir="$webapp_dir/dist"

    [ -d "$webapp_dir" ] || return 0
    mkdir -p "$dist_dir" 2>/dev/null || true
    chown -R "$build_user:$build_user" "$dist_dir" 2>/dev/null || true
}

build_webapp_frontend() {
    local project_root="${1:-$(project_root)}"
    local build_user="${2:-$BUILD_USER}"
    local mode="${3:-required}"
    local webapp_dir="$project_root/webapp"

    if [ ! -d "$webapp_dir/src" ]; then
        if [ "$mode" = "optional" ]; then
            warn "WebApp source not found — skipping WebApp build"
            return 0
        fi
        die "WebApp source not found at $webapp_dir/src"
    fi
    if ! command -v node >/dev/null 2>&1 || ! command -v npm >/dev/null 2>&1; then
        if [ "$mode" = "optional" ]; then
            warn "Node.js/npm not found — skipping WebApp build"
            return 0
        fi
        die "Node.js/npm not found — cannot build WebApp frontend"
    fi

    prepare_webapp_build_dir "$webapp_dir" "$build_user"
    local deps_stamp="$webapp_dir/node_modules/.package-lock.json"
    if [ ! -d "$webapp_dir/node_modules" ] \
       || [ ! -f "$deps_stamp" ] \
       || [ "$webapp_dir/package.json" -nt "$deps_stamp" ] \
       || { [ -f "$webapp_dir/package-lock.json" ] && [ "$webapp_dir/package-lock.json" -nt "$deps_stamp" ]; }; then
        info "Installing WebApp dependencies..."
        if ! sudo -u "$build_user" bash -c "cd '$webapp_dir' && npm install --no-audit --no-fund"; then
            if [ "$mode" = "optional" ]; then
                warn "WebApp dependency install failed (non-fatal for this deploy path)"
                return 0
            fi
            die "WebApp dependency install failed"
        fi
    fi

    info "Building WebApp frontend..."
    if ! sudo -u "$build_user" bash -c "cd '$webapp_dir' && npm run build"; then
        if [ "$mode" = "optional" ]; then
            warn "WebApp frontend build failed (non-fatal for this deploy path)"
            return 0
        fi
        die "WebApp frontend build failed"
    fi
    ok "WebApp frontend build complete"
}

sync_webapp_dist_to_release() {
    local release_dir="$1"
    local project_root="${2:-$(project_root)}"
    local dist_dir="$project_root/webapp/dist"

    [ -d "$release_dir" ] || die "Release directory missing: $release_dir"
    [ -f "$dist_dir/index.html" ] || die "WebApp dist missing or incomplete: $dist_dir/index.html"

    rm -rf "$release_dir/webapp.new" "$release_dir/webapp.prev"
    mkdir -p "$release_dir/webapp.new"
    cp -r "$dist_dir" "$release_dir/webapp.new/dist"
    if [ -d "$release_dir/webapp" ]; then
        mv "$release_dir/webapp" "$release_dir/webapp.prev"
    fi
    mv "$release_dir/webapp.new" "$release_dir/webapp"
    rm -rf "$release_dir/webapp.prev"
    chown -R "$SERVICE_USER":"$SERVICE_USER" "$release_dir/webapp" 2>/dev/null || true
    ok "WebApp tree swapped"
}

sync_webapp_dist_to_live_dir() {
    local release_dir="$1"
    local env_path="${2:-${ENV_FILE:-$DEPLOY_ROOT/shared/.env}}"

    [ -d "$release_dir/webapp/dist" ] || return 0

    local live_webapp_dir
    live_webapp_dir=$(grep -s '^AEGIS_WEBAPP_DIR=' "$env_path" | head -1 | cut -d= -f2)
    if [ -z "$live_webapp_dir" ] || [ "$live_webapp_dir" = "$release_dir/webapp/dist" ]; then
        return 0
    fi

    local parent tmp_dir prev_dir
    parent="$(dirname "$live_webapp_dir")"
    mkdir -p "$parent"
    tmp_dir="$parent/.webapp.new.$$"
    prev_dir="$parent/.webapp.prev.$$"
    rm -rf "$tmp_dir" "$prev_dir"
    mkdir -p "$tmp_dir"
    cp -r "$release_dir/webapp/dist/"* "$tmp_dir/"
    if [ -d "$live_webapp_dir" ]; then
        mv "$live_webapp_dir" "$prev_dir"
    fi
    mv "$tmp_dir" "$live_webapp_dir"
    rm -rf "$prev_dir"
    chown -R "$SERVICE_USER":"$SERVICE_USER" "$live_webapp_dir" 2>/dev/null || true
    ok "WebApp synced to $live_webapp_dir"
}

# Build release binary + Wasm skills from source.
# Runs as BUILD_USER since cargo/source are not owned by root.
# Reuses cargo fingerprints; staleness is verified against source mtimes
# (see below). scripts/prebuild-release.sh warms the same cache with the
# same flags so a deploy can be a pure copy.
build_release() {
    SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
    PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

    # Linker flags come from .cargo/config.toml ([target.aarch64-unknown-linux-gnu]
    # rustflags = mold), which every build on this host shares. Do NOT export
    # RUSTFLAGS here: `sudo -u $BUILD_USER` resets the environment, so the
    # export never reached cargo anyway, and a RUSTFLAGS value that differs
    # from the interactive/prebuild flags changes cargo's fingerprint and
    # forces a full ~35-minute rebuild on every deploy (found 2026-09-04).
    if [ "$(uname -m)" = "aarch64" ] && command -v mold &>/dev/null; then
        info "mold linker configured via .cargo/config.toml"
    fi
    local SCCACHE_BIN="" CARGO_BUILD_ENV
    # 2026-09-04: sccache is OFF by default. Measured 0 cache hits on this
    # host, and setting RUSTC_WRAPPER changes cargo's fingerprint, so every
    # deploy rebuilt all ~760 crates from scratch (~30 min) instead of reusing
    # the interactive/pre-built artifacts in the same target dir. Opt in with
    # AEGIS_RUNTIME_USE_SCCACHE=1 only if a shared cache actually exists.
    if [ "${AEGIS_RUNTIME_USE_SCCACHE:-0}" = "1" ]; then
        SCCACHE_BIN="$(command -v sccache 2>/dev/null || true)"
        if [ -n "$SCCACHE_BIN" ]; then
            info "Using sccache for Rust build caching"
        fi
    fi

    info "Building release binary..."
    # 2026-09-04: no more `touch core/src/lib.rs`. That forced a full ~20-min
    # recompile of the core crate on every deploy even when the artifact was
    # already current (and threw away any pre-built binary). Cargo's
    # fingerprinting is trusted; staleness is caught below by comparing the
    # artifact mtime against the newest source file instead of build start.

    # Per Plan D10: deploy builds use a dedicated cache outside the repo tree.
    # Isolates from any interactive cargo build in main/worktrees; survives
    # repo-level `git clean -fdx`; persists across releases for warm-cache builds.
    # Overridable via AEGIS_RUNTIME_BUILD_CACHE for testing / alternate layouts.
    local DEPLOY_TARGET="${AEGIS_RUNTIME_BUILD_CACHE:-/opt/aegis-runtime/build-cache}"
    mkdir -p "$DEPLOY_TARGET" 2>/dev/null || true
    chown -R "$BUILD_USER:$BUILD_USER" "$DEPLOY_TARGET" 2>/dev/null || true

    # Capture pre-build epoch + export the build output dir for
    # find_artifact. This pair closes a real bug observed 2026-04-17:
    # DEPLOY_TARGET moved from $PROJECT_ROOT/target/deploy to
    # /opt/aegis-runtime/build-cache, but find_artifact still looked at the
    # old path. Cargo built the right binary; find_artifact copied a
    # five-day-old stale one. Cargo exits 0 on no-op builds (fingerprint
    # cache hit), so without a freshness check we couldn't notice.
    DEPLOY_BUILD_START_EPOCH=$(date -u +%s)
    export DEPLOY_BUILD_START_EPOCH
    DEPLOY_BUILD_OUTPUT_DIR="$DEPLOY_TARGET/release"
    export DEPLOY_BUILD_OUTPUT_DIR

    CARGO_BUILD_ENV="CARGO_TARGET_DIR='$DEPLOY_TARGET'"
    if [ -n "$SCCACHE_BIN" ]; then
        CARGO_BUILD_ENV="$CARGO_BUILD_ENV RUSTC_WRAPPER='$SCCACHE_BIN'"
    fi
    sudo -u "$BUILD_USER" bash -c "source \$HOME/.cargo/env 2>/dev/null; cd '$PROJECT_ROOT' && $CARGO_BUILD_ENV nice -n 15 cargo build --release -p aegis-runtime-core" \
        200>&- || die "Release build failed"

    # Post-build freshness check. Cargo returns 0 for both a real build
    # and a cache-hit no-op; `stat %Y` on the output discriminates.
    local built_bin="$DEPLOY_BUILD_OUTPUT_DIR/$BINARY_NAME"
    if [ ! -f "$built_bin" ]; then
        die "Release build reported success but no binary at $built_bin — CARGO_TARGET_DIR mismatch? Inspect the cargo invocation + $DEPLOY_TARGET ownership."
    fi
    local built_mtime newest_src
    built_mtime=$(stat -c '%Y' "$built_bin" 2>/dev/null || echo 0)
    # Newest source input the binary must be at least as new as. If cargo
    # rebuilt, the artifact is newer than every source; if it short-circuited
    # (fingerprint hit) the sources are older too. Only a wrong-path / stale
    # artifact fails this.
    newest_src=$(find "$PROJECT_ROOT/core/src" "$PROJECT_ROOT/core/build.rs" "$PROJECT_ROOT/core/Cargo.toml" "$PROJECT_ROOT/Cargo.toml" "$PROJECT_ROOT/Cargo.lock" "$PROJECT_ROOT/skills" -type f -not -path '*/target/*' -printf '%T@\n' 2>/dev/null | sort -n | tail -1 | cut -d. -f1)
    if [ "$built_mtime" -lt "${newest_src:-0}" ]; then
        die "Built binary mtime ($(date -u -d "@$built_mtime" +%Y-%m-%dT%H:%M:%SZ)) is older than the newest source file ($(date -u -d "@$newest_src" +%Y-%m-%dT%H:%M:%SZ)). Cargo did not rebuild although sources changed. Investigate: CARGO_TARGET_DIR mismatch? Is Cargo.lock current? Disk full on $DEPLOY_TARGET?"
    fi
    if [ "$built_mtime" -ge "$DEPLOY_BUILD_START_EPOCH" ]; then
        ok "Release build complete ($(awk "BEGIN{printf \"%.1f\", $(stat -c '%s' "$built_bin")/1048576}")MB, rebuilt this run)"
    else
        ok "Release build complete ($(awk "BEGIN{printf \"%.1f\", $(stat -c '%s' "$built_bin")/1048576}")MB, reused up-to-date artifact from $(date -u -d "@$built_mtime" +%Y-%m-%dT%H:%M:%SZ))"
    fi

    # Verify Wasm skills are not stale (source newer than compiled .wasm)
    local STALE_COUNT=0
    for wasm in "$PROJECT_ROOT"/compiled_skills/*.wasm; do
        [ -f "$wasm" ] || continue
        local SKILL_NAME
        SKILL_NAME="$(basename "$wasm" .wasm)"
        local SRC="$PROJECT_ROOT/skills/$SKILL_NAME/src/lib.rs"
        if [ -f "$SRC" ] && [ "$SRC" -nt "$wasm" ]; then
            warn "Stale Wasm: $SKILL_NAME (source newer than compiled .wasm)"
            STALE_COUNT=$((STALE_COUNT + 1))
        fi
    done
    if [ "$STALE_COUNT" -gt 0 ]; then
        die "$STALE_COUNT stale Wasm skill(s) detected — recompile with: cargo build --release -p aegis-runtime-core"
    fi

    # Build Aegis Executor (privilege-separated command execution service)
    if [ -d "$PROJECT_ROOT/executor" ]; then
        info "Building Aegis Executor..."
        sudo -u "$BUILD_USER" bash -c "source \$HOME/.cargo/env 2>/dev/null; cd '$PROJECT_ROOT' && $CARGO_BUILD_ENV nice -n 15 cargo build --release -p aegis-executor" \
            200>&- || warn "Executor build failed (non-fatal — sysadmin features unavailable)"
        ok "Executor build complete"
    fi

    build_webapp_frontend "$PROJECT_ROOT" "$BUILD_USER" optional

    # Clean up debug/check artifacts (can grow to 20GB+)
    if [ -d "$PROJECT_ROOT/target/debug" ]; then
        local DEBUG_SIZE
        DEBUG_SIZE="$(du -sm "$PROJECT_ROOT/target/debug" 2>/dev/null | cut -f1)"
        if [ "${DEBUG_SIZE:-0}" -gt 500 ]; then
            info "Cleaning debug artifacts (${DEBUG_SIZE}MB)..."
            sudo -u "$BUILD_USER" bash -c "cd '$PROJECT_ROOT' && cargo clean --profile dev" 2>/dev/null \
                || rm -rf "$PROJECT_ROOT/target/debug"
            ok "Debug artifacts cleaned"
        fi
    fi
}

# Set up runtime prerequisites: the service user needs access to the built-in
# Wasm skills in the source workspace, a writable coordination inbox, and the
# shared .env keys the runtime reads at startup.
setup_runtime_env() {
    SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd)"
    PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

    # Detect the workspace owner (the user who owns Cargo.toml)
    if [ -f "$PROJECT_ROOT/Cargo.toml" ]; then
        local WS_OWNER
        WS_OWNER="$(stat -c '%U' "$PROJECT_ROOT/Cargo.toml")"

        # Add service user to workspace owner's group for write access
        if [ "$WS_OWNER" != "$SERVICE_USER" ]; then
            if ! id -nG "$SERVICE_USER" 2>/dev/null | grep -qw "$WS_OWNER"; then
                usermod -a -G "$WS_OWNER" "$SERVICE_USER"
                ok "Added $SERVICE_USER to $WS_OWNER group (skill workspace access)"
            fi

            # Ensure skill dirs are group-writable so both build user and service can write.
            # Do NOT chown compiled_skills/*.wasm — that breaks subsequent cargo builds.
            # The setgid bit on directories ensures new files inherit the correct group.
            chown -R "$SERVICE_USER":"$WS_OWNER" "$PROJECT_ROOT/skills/" 2>/dev/null || true
            chmod -R g+w "$PROJECT_ROOT/skills/" 2>/dev/null || true
            # compiled_skills: keep original owner, just ensure group-writable
            chgrp "$WS_OWNER" "$PROJECT_ROOT/compiled_skills/"*.wasm 2>/dev/null || true
            chmod g+w "$PROJECT_ROOT/compiled_skills/"*.wasm 2>/dev/null || true
            ok "Skill files group-writable for both $SERVICE_USER and $WS_OWNER"
        fi

        # Ensure runtime-write dirs are group-writable for both the build user
        # and service user. This runs even when WS_OWNER == SERVICE_USER so
        # tests/fresh installs still prepare the coordination inbox.
        # The ceo-inbox is where nightly / merge escalation reports are written
        # under AEGIS_REPO_ROOT; without group write the watcher path is
        # discoverable but report writes fail under systemd.
        for d in skills compiled_skills target .claude/coordination/ceo-inbox; do
            local DIR="$PROJECT_ROOT/$d"
            mkdir -p "$DIR"
            chgrp "$WS_OWNER" "$DIR" 2>/dev/null || true
            chmod g+ws "$DIR"
        done
        ok "Workspace directories set to group-writable (with setgid)"

        # Auto-configure env vars in .env if not already present
        local ENV_FILE="$DEPLOY_ROOT/shared/.env"
        if [ -f "$ENV_FILE" ]; then
            # Credential store path (must be writable by service)
            if ! grep -q "^CREDENTIAL_DB_PATH=" "$ENV_FILE" 2>/dev/null; then
                echo "CREDENTIAL_DB_PATH=$DEPLOY_ROOT/shared/aegis_runtime_credentials.db" >> "$ENV_FILE"
                ok "Added CREDENTIAL_DB_PATH to .env"
            fi

            # WebApp dist directory (for SDUI Mini App)
            if ! grep -q "^AEGIS_WEBAPP_DIR=" "$ENV_FILE" 2>/dev/null; then
                echo "AEGIS_WEBAPP_DIR=$DEPLOY_ROOT/current/webapp/dist" >> "$ENV_FILE"
                ok "Added AEGIS_WEBAPP_DIR to .env"
            fi

            # OSS default: workbench chat context without starting Telegram.
            # A configured TELOXIDE_TOKEN is the opt-in. Do not invent
            # ADMIN_CHAT_ID=1 in that case — chat id 1 plus a token would
            # start long-polling.
            if ! grep -qE '^ADMIN_CHAT_ID=' "$ENV_FILE" 2>/dev/null; then
                if grep -qE '^TELOXIDE_TOKEN=.+' "$ENV_FILE" 2>/dev/null; then
                    warn "TELOXIDE_TOKEN is set but ADMIN_CHAT_ID is missing — not defaulting it to 1 (that would start the bot). Set ADMIN_CHAT_ID to opt in."
                else
                    printf '\n# Workbench chat context. Telegram stays off until TELOXIDE_TOKEN is set.\nADMIN_CHAT_ID=1\n' >> "$ENV_FILE"
                    ok "ADMIN_CHAT_ID=1 added (workbench; Telegram off)"
                fi
            fi

            # Web Console session cookie signing key. Generate once and keep
            # stable across restarts so existing sessions are not invalidated.
            # Do not print the generated value to deploy logs.
            if ! grep -q "^SESSION_COOKIE_SECRET=" "$ENV_FILE" 2>/dev/null; then
                SESSION_SECRET=$(openssl rand -hex 32)
                echo "SESSION_COOKIE_SECRET=$SESSION_SECRET" >> "$ENV_FILE"
                ok "Generated SESSION_COOKIE_SECRET in .env"
            fi

            # Multimodal: STT (Speech-to-Text)
            if ! grep -qE "^#? ?AEGIS_STT_API_KEY=" "$ENV_FILE" 2>/dev/null; then
                echo "# Multimodal: STT via OpenAI (gpt-4o-mini-transcribe)" >> "$ENV_FILE"
                echo "# AEGIS_STT_API_KEY=sk-your-openai-key" >> "$ENV_FILE"
                echo "# AEGIS_STT_MODEL=gpt-4o-mini-transcribe" >> "$ENV_FILE"
                info "Add AEGIS_STT_API_KEY to .env for voice message transcription"
            fi

            # Multimodal: TTS (Text-to-Speech) via xAI
            if ! grep -qE "^#? ?AEGIS_TTS_API_KEY=" "$ENV_FILE" 2>/dev/null; then
                echo "# Multimodal: TTS via xAI" >> "$ENV_FILE"
                echo "# AEGIS_TTS_API_KEY=xai-your-key" >> "$ENV_FILE"
                echo "# AEGIS_TTS_VOICE=eve" >> "$ENV_FILE"
                info "Add AEGIS_TTS_API_KEY to .env for voice reply capability"
            fi

            # Multimodal: Image Generation via OpenRouter (uses existing API key)
            # Optional: override model with AEGIS_IMAGEGEN_MODEL
            if ! grep -qE "^#? ?AEGIS_IMAGEGEN_MODEL=" "$ENV_FILE" 2>/dev/null; then
                echo "# Multimodal: Image Generation (xAI; needs AEGIS_IMAGEGEN_API_KEY)" >> "$ENV_FILE"
                echo "# AEGIS_IMAGEGEN_MODEL=grok-imagine-image-2.0" >> "$ENV_FILE"
            fi

        fi
    fi
}

# Validate a compiled binary is safe to deploy: exists, executable, above a
# minimum-size threshold (below = likely linker broke or partial cp), and
# responds to --version (advisory — warn only; binaries without --version
# are legitimate). Used by cmd_upgrade + cmd_update BEFORE services are
# stopped so a bad artifact can't leave production without code.
validate_binary() {
    local path="$1"
    local name="${2:-$(basename "$path")}"
    if [ ! -x "$path" ]; then
        die "$name at $path is not executable — aborting deploy"
    fi
    local size
    size=$(stat -c%s "$path" 2>/dev/null || echo 0)
    if [ "$size" -lt 1048576 ]; then
        die "$name too small (${size} bytes, expected >1MB) — likely corrupted"
    fi
    if ! "$path" --version >/dev/null 2>&1; then
        warn "$name --version check failed (may not support --version flag, continuing)"
    fi
    ok "$name validated ($((size / 1048576))MB, executable)"
}

# Validate a systemd .service unit file syntactically BEFORE any
# service-state change. `systemctl daemon-reload` does NOT validate
# syntax — a typo in `ExecStart=` or a missing `[Service]` stanza is
# silently accepted and only surfaces at `systemctl start` time, which
# in cmd_update is AFTER we've already stopped the old service. This
# gate fires BEFORE the stop so bad unit files can't take down
# production. Uses `systemd-analyze verify` which statically parses
# the unit. Loud warn on non-zero exit (missing binary paths are a
# common false positive; die only on clearly-broken syntax).
validate_systemd_unit() {
    local unit_path="$1"
    if [ ! -f "$unit_path" ]; then
        die "systemd unit $unit_path does not exist"
    fi
    if ! command -v systemd-analyze >/dev/null 2>&1; then
        warn "systemd-analyze not available; skipping unit-file verification for $unit_path"
        return 0
    fi
    local verify_out verify_rc
    verify_out=$(systemd-analyze verify "$unit_path" 2>&1) || verify_rc=$?
    verify_rc=${verify_rc:-0}
    # `systemd-analyze verify` exits non-zero for syntax errors, missing
    # mandatory sections, or undefined Type=. It ALSO warns on things
    # like missing ExecStart binary (which is usually fine because we
    # don't deploy the binary until after this runs). Distinguish:
    # - Exit 0 or 1 with empty output: clean.
    # - Exit non-zero with syntax-level error ("Assignment outside of
    #   section", "Unknown lvalue", "Cannot add dependency", "Failed to
    #   parse"): die.
    # - Otherwise: warn (likely missing-binary heuristic) but proceed.
    if [ "$verify_rc" -eq 0 ] && [ -z "$verify_out" ]; then
        return 0
    fi
    if printf '%s' "$verify_out" | grep -qE 'Assignment outside of section|Unknown lvalue|Cannot add dependency|Failed to parse|Unknown section'; then
        die "systemd unit $unit_path failed syntax verification:
$verify_out"
    fi
    warn "systemd-analyze verify flagged warnings on $unit_path (likely missing runtime deps not-yet-installed); proceeding:
$verify_out"
}

# Find the tarball or compiled binary to deploy.
# Preference order:
#   1. Packaged tarball in dist/
#   2. Build output from this deploy run — DEPLOY_BUILD_OUTPUT_DIR exported
#      by build_release. This is the authoritative source for update/upgrade
#      since build_release already freshness-checked it.
#   3. AEGIS_RUNTIME_BUILD_CACHE (or its default /opt/aegis-runtime/build-cache) —
#      for paths where build_release hasn't been called (e.g. direct install
#      after an external build).
#   4. Legacy $PROJECT_ROOT/target/deploy/release and $PROJECT_ROOT/target/release,
#      gated behind mtime freshness (≤2h) + loud warning. Prevents the
#      silent-stale-binary regression observed 2026-04-17 where a 5-day-old
#      binary from a previous CARGO_TARGET_DIR layout was shipped.
find_artifact() {
    SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
    # PROJECT_ROOT derivation — AEGIS_RUNTIME_PROJECT_ROOT env override is
    # SCOPED TO find_artifact only and exists purely so the hermetic
    # test at scripts/test-deploy-find-artifact.sh can simulate
    # legacy-path states without depending on the real worktree. DO
    # NOT set AEGIS_RUNTIME_PROJECT_ROOT in a real deploy invocation —
    # other call sites (e.g. compiled_skills + skills/ copy in
    # cmd_update) derive PROJECT_ROOT independently and will not
    # honor it, leading to a half-overridden deploy.
    PROJECT_ROOT="${AEGIS_RUNTIME_PROJECT_ROOT:-$(cd "$SCRIPT_DIR/.." && pwd)}"

    # Prefer packaged tarball
    TARBALL="$(ls -t "$PROJECT_ROOT"/dist/aegis-runtime-*.tar.gz 2>/dev/null | head -1 || true)"
    if [ -n "$TARBALL" ]; then
        echo "$TARBALL"
        return
    fi

    local RELEASE_BIN=""

    # Preferred: use the build output dir exported by build_release.
    if [ -n "${DEPLOY_BUILD_OUTPUT_DIR:-}" ] && [ -f "$DEPLOY_BUILD_OUTPUT_DIR/$BINARY_NAME" ]; then
        RELEASE_BIN="$DEPLOY_BUILD_OUTPUT_DIR"
    fi

    # Second: AEGIS_RUNTIME_BUILD_CACHE (same logic as build_release, in case a
    # caller invokes find_artifact without having gone through build_release).
    if [ -z "$RELEASE_BIN" ]; then
        local CACHE_DIR="${AEGIS_RUNTIME_BUILD_CACHE:-/opt/aegis-runtime/build-cache}/release"
        if [ -f "$CACHE_DIR/$BINARY_NAME" ]; then
            RELEASE_BIN="$CACHE_DIR"
        fi
    fi

    # Third (legacy): the old target/deploy/release layout. Only accept it if
    # the binary is fresh (≤2h) AND we loudly warn. Stale binaries from this
    # path caused the 2026-04-17 silent-regression incident — see CLAUDE.md
    # deploy anti-patterns.
    if [ -z "$RELEASE_BIN" ]; then
        for legacy in "$PROJECT_ROOT/target/deploy/release" "$PROJECT_ROOT/target/release"; do
            [ -f "$legacy/$BINARY_NAME" ] || continue
            local mtime now age_secs
            mtime=$(stat -c '%Y' "$legacy/$BINARY_NAME" 2>/dev/null || echo 0)
            now=$(date -u +%s)
            age_secs=$(( now - mtime ))
            if [ "$age_secs" -gt 7200 ]; then
                # Redirect to stderr — find_artifact's stdout is reserved for
                # the return value that callers capture via $(). A warn on
                # stdout would contaminate ARTIFACT with multi-line garbage.
                warn "Ignoring stale legacy artifact: $legacy/$BINARY_NAME (age $((age_secs / 60))min > 2h)" >&2
                continue
            fi
            warn "Using legacy build path $legacy ($(( age_secs / 60 ))min old). Build output should land in \$AEGIS_RUNTIME_BUILD_CACHE — investigate whether build_release ran." >&2
            RELEASE_BIN="$legacy"
            break
        done
    fi

    if [ -n "$RELEASE_BIN" ]; then
        echo "$RELEASE_BIN"
        return
    fi

    die "No artifact found. Looked in:
  - \$DEPLOY_BUILD_OUTPUT_DIR = ${DEPLOY_BUILD_OUTPUT_DIR:-(unset)}
  - \${AEGIS_RUNTIME_BUILD_CACHE:-/opt/aegis-runtime/build-cache}/release
  - $PROJECT_ROOT/target/deploy/release (legacy, mtime ≤2h required)
  - $PROJECT_ROOT/target/release (legacy, mtime ≤2h required)
Run 'make package' or 'cargo build --release' first."
}

project_root() {
    local script_dir
    script_dir="$(cd "$(dirname "$0")" && pwd)"
    echo "$(cd "$script_dir/.." && pwd)"
}

stage_release_payload() {
    local artifact="$1"
    local release_dir="$2"
    local source_root="${3:-}"
    local asset_root=""
    local artifact_bin_root=""
    local temp_root=""

    if [[ "$artifact" == *.tar.gz ]]; then
        temp_root="$(mktemp -d /tmp/aegis-runtime-update-stage.XXXXXX)"
        tar xzf "$artifact" -C "$temp_root" --strip-components=1 \
            || die "Tarball extraction failed — archive may be corrupt: $artifact"
        asset_root="$temp_root"
        artifact_bin_root="$temp_root"
    else
        [ -n "$source_root" ] || die "stage_release_payload requires source_root for directory artifacts"
        asset_root="$source_root"
        artifact_bin_root="$artifact"
    fi

    rm -f "$release_dir/$BINARY_NAME.new" "$release_dir/$EXECUTOR_BINARY.new" "$release_dir/$BACKUP_BINARY.new" "$release_dir/$ENCRYPT_BINARY.new"
    rm -rf "$release_dir/compiled_skills.new" "$release_dir/skills.new" "$release_dir/webapp.new"

    cp "$artifact_bin_root/$BINARY_NAME" "$release_dir/$BINARY_NAME.new"
    validate_binary "$release_dir/$BINARY_NAME.new" "aegis_runtime_core (staged)"
    if [ -f "$artifact_bin_root/$EXECUTOR_BINARY" ]; then
        cp "$artifact_bin_root/$EXECUTOR_BINARY" "$release_dir/$EXECUTOR_BINARY.new"
        validate_binary "$release_dir/$EXECUTOR_BINARY.new" "aegis-executor (staged)"
    fi
    if [ -f "$artifact_bin_root/$BACKUP_BINARY" ]; then
        cp "$artifact_bin_root/$BACKUP_BINARY" "$release_dir/$BACKUP_BINARY.new"
        validate_binary "$release_dir/$BACKUP_BINARY.new" "encrypted backup helper (staged)"
    fi
    if [ -f "$artifact_bin_root/$ENCRYPT_BINARY" ]; then
        cp "$artifact_bin_root/$ENCRYPT_BINARY" "$release_dir/$ENCRYPT_BINARY.new"
        validate_binary "$release_dir/$ENCRYPT_BINARY.new" "database encryption helper (staged)"
    fi

    mkdir -p "$release_dir/compiled_skills.new"
    if [ -d "$asset_root/compiled_skills" ]; then
        cp -r "$asset_root/compiled_skills/"* "$release_dir/compiled_skills.new/"
    fi

    if [ -d "$asset_root/skills" ]; then
        mkdir -p "$release_dir/skills.new"
        cp -r "$asset_root/skills/"* "$release_dir/skills.new/"
    fi

    if [ -d "$asset_root/webapp/dist" ]; then
        mkdir -p "$release_dir/webapp.new"
        cp -r "$asset_root/webapp/dist" "$release_dir/webapp.new/dist"
    fi

    if [ -n "$temp_root" ]; then
        rm -rf "$temp_root"
    fi
}

activate_staged_payload() {
    DEPLOY_DESTRUCTIVE_PHASE="yes"
    local release_dir="$1"

    if [ -f "$release_dir/$BINARY_NAME.new" ]; then
        mv -Tf "$release_dir/$BINARY_NAME.new" "$release_dir/$BINARY_NAME"
        ok "Binary atomically swapped: $BINARY_NAME"
    fi
    if [ -f "$release_dir/$EXECUTOR_BINARY.new" ]; then
        mv -Tf "$release_dir/$EXECUTOR_BINARY.new" "$release_dir/$EXECUTOR_BINARY"
        ok "Executor binary atomically swapped: $EXECUTOR_BINARY"
    fi
    if [ -f "$release_dir/$BACKUP_BINARY.new" ]; then
        mv -Tf "$release_dir/$BACKUP_BINARY.new" "$release_dir/$BACKUP_BINARY"
        ok "Encrypted backup helper atomically swapped: $BACKUP_BINARY"
    fi
    if [ -f "$release_dir/$ENCRYPT_BINARY.new" ]; then
        mv -Tf "$release_dir/$ENCRYPT_BINARY.new" "$release_dir/$ENCRYPT_BINARY"
        ok "Database encryption helper atomically swapped: $ENCRYPT_BINARY"
    fi

    if [ -d "$release_dir/compiled_skills.new" ]; then
        rm -rf "$release_dir/compiled_skills"
        mv "$release_dir/compiled_skills.new" "$release_dir/compiled_skills"
        touch "$release_dir/compiled_skills/"*.wasm 2>/dev/null || true
        ok "Compiled skills swapped"
    fi
    if [ -d "$release_dir/skills.new" ]; then
        rm -rf "$release_dir/skills"
        mv "$release_dir/skills.new" "$release_dir/skills"
        ok "Skill source tree swapped"
    fi
    if [ -d "$release_dir/webapp.new" ]; then
        rm -rf "$release_dir/webapp"
        mv "$release_dir/webapp.new" "$release_dir/webapp"
        ok "WebApp tree swapped"
    fi
}

# ── auth-first preflight (install) ───────────────────────────────────
# The service refuses to boot without grok / Codex / a usable OPENAI_API_KEY,
# and owner chat additionally needs the provider bound for chat. Probing
# this BEFORE the release build means a missing sign-in costs seconds, not a
# 10-minute build followed by a 503. Order of preference: grok login, then
# Codex login, then API key. Sets AUTH_PREFLIGHT_PROVIDER for the post-install
# bind step. Skip with AEGIS_RUNTIME_SKIP_AUTH_PREFLIGHT=1.
service_user_home() {
    # AEGIS_RUNTIME_SERVICE_USER_HOME exists for hermetic tests (scripts/test-deploy.sh).
    if [ -n "${AEGIS_RUNTIME_SERVICE_USER_HOME:-}" ]; then
        echo "$AEGIS_RUNTIME_SERVICE_USER_HOME"
        return 0
    fi
    getent passwd "$SERVICE_USER" 2>/dev/null | cut -d: -f6 || true
}

preflight_llm_auth() {
    AUTH_PREFLIGHT_PROVIDER=""
    if [ "${AEGIS_RUNTIME_SKIP_AUTH_PREFLIGHT:-0}" = "1" ]; then
        warn "Auth preflight skipped (AEGIS_RUNTIME_SKIP_AUTH_PREFLIGHT=1) — the service will not boot until grok/Codex/OPENAI_API_KEY exists for $SERVICE_USER"
        return 0
    fi
    local svc_home env_file="$DEPLOY_ROOT/shared/.env" probed=()
    svc_home="$(service_user_home)"
    [ -n "$svc_home" ] || svc_home="/home/$SERVICE_USER"

    # 1) grok CLI session in the SERVICE user's home (that is the HOME the unit runs with).
    if [ -s "$svc_home/.grok/auth.json" ] && { [ -x "$svc_home/.grok/bin/grok" ] || command -v grok >/dev/null 2>&1; }; then
        AUTH_PREFLIGHT_PROVIDER="grok"
        ok "Auth preflight: grok CLI session found for $SERVICE_USER ($svc_home/.grok/auth.json)"
        return 0
    fi
    probed+=("grok: $svc_home/.grok/auth.json missing (or grok CLI not installed)")

    # 2) Codex CLI logged in (ChatGPT) for the service user.
    if command -v codex >/dev/null 2>&1 \
       && sudo -u "$SERVICE_USER" -H env CODEX_HOME="$svc_home/.codex" codex login status 2>/dev/null | grep -q "Logged in using ChatGPT"; then
        AUTH_PREFLIGHT_PROVIDER="codex"
        ok "Auth preflight: Codex CLI logged in for $SERVICE_USER"
        return 0
    fi
    probed+=("codex: not logged in for $SERVICE_USER (codex login status)")

    # 3) usable OPENAI_API_KEY in the shared env file (last resort).
    local key
    key="$(grep -E '^OPENAI_API_KEY=' "$env_file" 2>/dev/null | tail -1 | cut -d= -f2- | tr -d '"' | tr -d "'")"
    case "${key// /}" in
        ""|sk-placeholder|sk-or-v1-xxxxxxxx)
            probed+=("api key: $env_file has no usable OPENAI_API_KEY") ;;
        *)
            AUTH_PREFLIGHT_PROVIDER="api-key"
            ok "Auth preflight: usable OPENAI_API_KEY in $env_file"
            return 0 ;;
    esac

    GATE_REJECTED="yes"
    GATE_REJECT_CODE="$EXIT_AUTH_PREFLIGHT"
    echo -e "${RED}[deploy] INSTALL STOPPED BEFORE BUILD — no owner-chat provider sign-in yet${NC}" >&2
    printf '  probed: %s\n' "${probed[@]}" >&2
    cat >&2 <<EOF

Complete ONE sign-in first (login before paid keys), then re-run install:

  A) Grok (preferred) — as the service user, so the unit's HOME sees it:
       curl -fsSL https://x.ai/cli/install.sh | sudo -u $SERVICE_USER -H bash
       sudo -u $SERVICE_USER -H $svc_home/.grok/bin/grok login --device-auth
     (the in-app Grok chat bind is a SECOND device code; install runs it
      after the build via: aegis_runtime_core setup-auth --provider grok)
  B) Codex:
       sudo -u $SERVICE_USER -H codex login --device-auth
  C) API key (last resort): put OPENAI_API_KEY=… and OPENAI_API_BASE=… in
       $env_file

Skip this check (service will not boot until fixed): AEGIS_RUNTIME_SKIP_AUTH_PREFLIGHT=1
EOF
    exit "$EXIT_AUTH_PREFLIGHT"
}

# After the release is on disk: bind owner chat as the service user with the
# shared env, BEFORE the operator ever runs `systemctl start`. Same binary
# mode the portable path uses (scripts/first-run.sh step 5).
bind_owner_chat_as_service_user() {
    local provider="${AEGIS_RUNTIME_AUTH_PROVIDER:-$AUTH_PREFLIGHT_PROVIDER}"
    local env_file="$DEPLOY_ROOT/shared/.env" svc_home
    svc_home="$(service_user_home)"
    [ -n "$svc_home" ] || svc_home="/home/$SERVICE_USER"
    local cmd="sudo --preserve-env=AEGIS_RUNTIME_DB_KEY_HEX -u $SERVICE_USER -H bash -c 'set -a; . $env_file; set +a; export IDENTITY_KEY_PATH=$DEPLOY_ROOT/shared/aegis_runtime_identity.key MEMORY_DB_PATH=$DEPLOY_ROOT/shared/aegis_runtime_memory.db CREDENTIAL_DB_PATH=$DEPLOY_ROOT/shared/aegis_runtime_credentials.db; cd $DEPLOY_ROOT/current && exec ./$BINARY_NAME setup-auth --provider ${provider:-grok}'"
    if [ -z "$provider" ]; then
        warn "No provider known for the pre-start chat bind. Run it before systemctl start:"
        echo "    $cmd" >&2
        return 0
    fi
    if [ ! -t 0 ] || [ ! -t 1 ]; then
        warn "No TTY — the $provider chat bind needs a human to open a URL. Run it before systemctl start:"
        echo "    $cmd" >&2
        return 0
    fi
    info "Binding owner chat to $provider as $SERVICE_USER (before first systemctl start)…"
    if bash -c "$cmd" 200>&-; then
        ok "Owner chat bound to $provider"
    else
        warn "Chat bind did not finish (exit $?). Re-run before systemctl start:"
        echo "    $cmd" >&2
    fi
}

# ── install ──────────────────────────────────────────────────────────
cmd_install() {
    require_root
    info "First-time installation to $DEPLOY_ROOT"

    # Auth FIRST — before users, directories, and the release build.
    preflight_llm_auth

    # Create service user
    if ! id "$SERVICE_USER" &>/dev/null; then
        useradd -r -s /usr/sbin/nologin "$SERVICE_USER"
        ok "Created user: $SERVICE_USER"
    else
        ok "User $SERVICE_USER already exists"
    fi

    # Grant journal read access (needed for view_logs pseudo-tool)
    if getent group systemd-journal &>/dev/null; then
        usermod -aG systemd-journal "$SERVICE_USER"
        ok "Added $SERVICE_USER to systemd-journal group"
    fi

    # Create directory structure
    mkdir -p "$DEPLOY_ROOT"/{releases,shared,backups}
    ok "Directory structure created"

    # Bootstrap host packages before the first source build. This keeps
    # production installs reproducible on fresh machines instead of relying
    # on whatever was manually installed on the original host.
    SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
    if [ -x "$SCRIPT_DIR/bootstrap-system-deps.sh" ]; then
        "$SCRIPT_DIR/bootstrap-system-deps.sh"
    fi

    # Deploy first release
    cmd_upgrade

    # Install systemd service
    if [ -f "$SCRIPT_DIR/aegis-runtime.service" ]; then
        cp "$SCRIPT_DIR/aegis-runtime.service" /etc/systemd/system/aegis-runtime.service
        systemctl daemon-reload
        systemctl enable "$SERVICE_NAME"
        ok "systemd service installed and enabled"
    else
        warn "aegis-runtime.service not found in scripts/ — install manually"
    fi

    # Install Aegis Executor service (privilege-separated sysadmin)
    if [ -f "$SCRIPT_DIR/aegis-executor.service" ]; then
        cp "$SCRIPT_DIR/aegis-executor.service" /etc/systemd/system/aegis-executor.service
        systemctl daemon-reload
        systemctl enable aegis-executor
        ok "aegis-executor service installed and enabled"
        # The executor reads ONLY executor.env (never the shared .env). Without
        # AEGIS_EXECUTOR_TOKEN there, diagnostics refuse every request.
        if ! grep -q '^AEGIS_EXECUTOR_TOKEN=.\+' "$DEPLOY_ROOT/shared/executor.env" 2>/dev/null; then
            warn "shared/executor.env has no AEGIS_EXECUTOR_TOKEN — executor diagnostics will refuse requests (fail closed)"
        fi
        # The trust anchor is per-install config now (it was a literal in the unit).
        # Without it the executor fails closed on /execute and every HITL
        # sysadmin approval stops working — so this is an error, not a warning.
        if ! grep -q '^AEGIS_PUBKEY_HEX=[0-9a-f]\{64\}$' "$DEPLOY_ROOT/shared/executor.env" 2>/dev/null; then
            err "shared/executor.env has no AEGIS_PUBKEY_HEX — run: IDENTITY_KEY_PATH=$DEPLOY_ROOT/shared/aegis_runtime_identity.key $DEPLOY_ROOT/current/aegis_runtime_core --print-pubkey  and append AEGIS_PUBKEY_HEX=<value> to $DEPLOY_ROOT/shared/executor.env"
            exit 1
        fi
    fi

    # Prompt for .env
    if [ ! -f "$DEPLOY_ROOT/shared/.env" ]; then
        SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
        PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
        if [ -f "$PROJECT_ROOT/.env.example" ]; then
            cp "$PROJECT_ROOT/.env.example" "$DEPLOY_ROOT/shared/.env"
            chmod 640 "$DEPLOY_ROOT/shared/.env"
            warn "Created $DEPLOY_ROOT/shared/.env from template — edit with your API keys before starting"
        fi
    fi

    chown -R "$SERVICE_USER":"$SERVICE_USER" "$DEPLOY_ROOT"

    # ── Runtime env setup: skill workspace access + shared .env keys ──
    setup_runtime_env

    # ── Cloudflare tunnel setup ──
    if [ -f "$SCRIPT_DIR/setup-tunnel.sh" ]; then
        info "Setting up Cloudflare tunnel..."
        bash "$SCRIPT_DIR/setup-tunnel.sh" || warn "Tunnel setup failed (non-fatal — set up manually later)"
    fi

    # ── Database backup cron ──
    if [ -f "$SCRIPT_DIR/backup-db.sh" ]; then
        # Install daily 3 AM backup cron (idempotent — overwrites if exists)
        local cron_line="0 3 * * * $SCRIPT_DIR/backup-db.sh >> /var/log/aegis-runtime-backup.log 2>&1"
        if ! crontab -l 2>/dev/null | grep -q "backup-db.sh"; then
            ( crontab -l 2>/dev/null; echo "$cron_line" ) | crontab -
            ok "Daily backup cron installed (3 AM)"
        else
            ok "Daily backup cron already installed"
        fi
    fi

    # ── Owner-chat bind BEFORE the first start (auth-first, step 2 of 2) ──
    bind_owner_chat_as_service_user

    ok "Installation complete!"
    echo ""
    echo "Next steps:"
    echo "  1. Review $DEPLOY_ROOT/shared/.env (ADMIN_CHAT_ID, optional Telegram/Google)"
    echo "  2. If the chat bind above was skipped: run the printed setup-auth command (check: … setup-auth status)"
    echo "  3. sudo systemctl start $SERVICE_NAME"
    echo "  4. sudo journalctl -u $SERVICE_NAME -f   # then one owner chat round-trip, not just /health"
}

# ── upgrade ──────────────────────────────────────────────────────────
cmd_upgrade() {
    require_root
    local PROJECT_ROOT
    PROJECT_ROOT="$(project_root)"
    local RELEASE_NAME
    RELEASE_NAME="$(date +%Y%m%d-%H%M%S)"
    local RELEASE_DIR="$DEPLOY_ROOT/releases/$RELEASE_NAME"

    info "Creating release: $RELEASE_NAME"

    # Build from source (ensures binary matches current code)
    build_release

    # Backup databases before upgrade
    if [ -f "$DEPLOY_ROOT/shared/aegis_runtime_memory.db" ]; then
        cmd_backup
    fi
    # Phase E (W17): back up heirloom_event_log.db alongside the legacy DB.
    if [ -f "$DEPLOY_ROOT/shared/heirloom_event_log.db" ]; then
        cmd_backup_heirloom
    fi

    ARTIFACT="$(find_artifact)"
    mkdir -p "$RELEASE_DIR"
    stage_release_payload "$ARTIFACT" "$RELEASE_DIR" "$PROJECT_ROOT"
    activate_staged_payload "$RELEASE_DIR"
    ok "Release payload staged and activated"

    # Switch symlink atomically
    ln -sfn "$RELEASE_DIR" "$DEPLOY_ROOT/current.tmp"
    mv -Tf "$DEPLOY_ROOT/current.tmp" "$DEPLOY_ROOT/current"
    ok "Symlink updated: current -> releases/$RELEASE_NAME"

    # Fix ownership
    chown -R "$SERVICE_USER":"$SERVICE_USER" "$RELEASE_DIR"

    # Ensure runtime env + skill workspace access are configured
    setup_runtime_env

    # Restart if already running
    restart_service

    # Clean old releases
    cleanup_releases

    # Post-deploy smoke test
    SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
    if [ -f "$SCRIPT_DIR/smoke_test.sh" ]; then
        info "Running post-deploy smoke test..."
        if bash "$SCRIPT_DIR/smoke_test.sh"; then
            ok "Smoke test passed"
        else
            warn "Smoke test FAILED — consider rolling back with: $0 rollback"
        fi
    fi

    ok "Upgrade complete (release: $RELEASE_NAME)"
}

# ── update (in-place) ────────────────────────────────────────────────
cmd_update() {
    require_root
    local PROJECT_ROOT
    PROJECT_ROOT="$(project_root)"

    if [ ! -L "$DEPLOY_ROOT/current" ]; then
        die "No current release found. Run 'install' or 'upgrade' first."
    fi

    local RELEASE_DIR
    RELEASE_DIR="$(readlink -f "$DEPLOY_ROOT/current")"
    info "In-place update of $(basename "$RELEASE_DIR")"

    # Build from source (ensures binary matches current code)
    build_release

    # Backup databases
    if [ -f "$DEPLOY_ROOT/shared/aegis_runtime_memory.db" ]; then
        cmd_backup
    fi
    # Phase E (W17): back up heirloom_event_log.db alongside the legacy DB.
    if [ -f "$DEPLOY_ROOT/shared/heirloom_event_log.db" ]; then
        cmd_backup_heirloom
    fi

    ARTIFACT="$(find_artifact)"

    if [[ "$ARTIFACT" == *.tar.gz ]]; then
        die "cmd_update only supports local directory build artifacts. Tarball deploys must use 'upgrade' so release activation stays versioned and symlink-based."
    fi

    # Pre-stop validation: confirm the SOURCE artifact is sane BEFORE any
    # service-state change. If artifact is corrupted or missing, we die
    # with the old code still running — no outage.
    validate_binary "$ARTIFACT/$BINARY_NAME" "aegis_runtime_core (source)"
    if [ -f "$ARTIFACT/$EXECUTOR_BINARY" ]; then
        validate_binary "$ARTIFACT/$EXECUTOR_BINARY" "aegis-executor (source)"
    fi

    # build_release already built WebApp assets once into $PROJECT_ROOT/webapp/dist.
    # Do not rebuild here; full deploys should not pay the frontend build cost twice.

    # Stage binaries, skills, and WebApp in `.new` locations while the old
    # service is still live. Nothing under the active release is mutated in
    # place until after services stop.
    # Keep the previous binary + bundle so a failed post-deploy smoke can
    # restore them (owner rule 2026-09-06: a broken deploy must never wait
    # for the owner to notice). `update` is in-place, so the releases/
    # rollback path has nothing older to switch to.
    local PRE_UPDATE_BACKUP="$DEPLOY_ROOT/backups/pre-update-$(date +%Y%m%d-%H%M%S)"
    mkdir -p -m 750 "$PRE_UPDATE_BACKUP"
    [ -f "$RELEASE_DIR/$BINARY_NAME" ] && cp -p "$RELEASE_DIR/$BINARY_NAME" "$PRE_UPDATE_BACKUP/"
    [ -f "$RELEASE_DIR/$EXECUTOR_BINARY" ] && cp -p "$RELEASE_DIR/$EXECUTOR_BINARY" "$PRE_UPDATE_BACKUP/"
    [ -d "$RELEASE_DIR/webapp/dist" ] && cp -a "$RELEASE_DIR/webapp/dist" "$PRE_UPDATE_BACKUP/webapp-dist"
    ls -1dt "$DEPLOY_ROOT"/backups/pre-update-* 2>/dev/null | tail -n +4 | xargs -r rm -rf
    stage_release_payload "$ARTIFACT" "$RELEASE_DIR" "$PROJECT_ROOT"
    ok "Binary, skills, and WebApp staged (.new files/dirs)"

    # Update systemd service files BEFORE stopping anything.
    # Rationale: if a new service file has a typo that breaks systemctl
    # start, validate_systemd_unit → die fires NOW with the OLD service
    # still running — no outage. systemctl daemon-reload itself is purely
    # state-reconcile (doesn't validate .service syntax) so we use
    # `systemd-analyze verify` as the actual gate.
    SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
    if [ -f "$SCRIPT_DIR/aegis-runtime.service" ]; then
        cp "$SCRIPT_DIR/aegis-runtime.service" /etc/systemd/system/aegis-runtime.service
        validate_systemd_unit /etc/systemd/system/aegis-runtime.service
        ok "Service file updated + verified"
    fi
    if [ -f "$SCRIPT_DIR/aegis-runtime-cloudflare.service" ]; then
        cp "$SCRIPT_DIR/aegis-runtime-cloudflare.service" /etc/systemd/system/aegis-runtime-cloudflare.service
        validate_systemd_unit /etc/systemd/system/aegis-runtime-cloudflare.service
        ok "Tunnel service file updated + verified"
    fi
    local EXECUTOR_SERVICE_INSTALLED=false
    if [ -f "$SCRIPT_DIR/aegis-executor.service" ] && { [ -f "$ARTIFACT/$EXECUTOR_BINARY" ] || [ -f "$RELEASE_DIR/$EXECUTOR_BINARY" ]; }; then
        cp "$SCRIPT_DIR/aegis-executor.service" /etc/systemd/system/aegis-executor.service
        validate_systemd_unit /etc/systemd/system/aegis-executor.service
        EXECUTOR_SERVICE_INSTALLED=true
        ok "Aegis Executor service file updated + verified"
    fi
    systemctl daemon-reload

    # Now stop services, perform the atomic binary swap, and restart.
    # The window between stop and restart is the ONLY no-service window;
    # anything that could die before it (validation, copy, service-file
    # check) has already run with the old service still live.
    DEPLOY_DESTRUCTIVE_PHASE="yes"
    local WAS_RUNNING=false
    if systemctl is-active --quiet "$SERVICE_NAME" 2>/dev/null; then
        WAS_RUNNING=true
        info "Stopping $SERVICE_NAME..."
        systemctl stop "$SERVICE_NAME"
    fi
    local EXECUTOR_WAS_RUNNING=false
    if systemctl is-active --quiet aegis-executor 2>/dev/null; then
        EXECUTOR_WAS_RUNNING=true
        info "Stopping aegis-executor..."
        systemctl stop aegis-executor
    fi

    activate_staged_payload "$RELEASE_DIR"

    # Also sync to AEGIS_WEBAPP_DIR (the path the service actually reads).
    sync_webapp_dist_to_live_dir "$RELEASE_DIR"

    # Start executor (was stopped above if running; start either way if
    # service file was installed so first-time executor comes up).
    if $EXECUTOR_SERVICE_INSTALLED; then
        systemctl enable aegis-executor 2>/dev/null || true
        systemctl start aegis-executor
        ok "Aegis Executor started"
    elif $EXECUTOR_WAS_RUNNING; then
        systemctl start aegis-executor
    fi

    # Restart tunnel if it was running (gets new URL, updates .env)
    restart_tunnel

    chown -R "$SERVICE_USER":"$SERVICE_USER" "$RELEASE_DIR"

    # Ensure runtime env + skill workspace access are configured
    setup_runtime_env

    # Restart/start main service to match the documented `update` contract:
    # replace then restart. `restart_service` itself preserves the current
    # enablement semantics and will start enabled-but-stopped services.
    restart_service

    # Post-deploy smoke test — catches regressions before user notices
    SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
    if [ -f "$SCRIPT_DIR/smoke_test.sh" ]; then
        info "Running post-deploy smoke test..."
        if bash "$SCRIPT_DIR/smoke_test.sh"; then
            ok "Smoke test passed"
        else
            warn "Smoke test FAILED — restoring the previous binary and bundle from $PRE_UPDATE_BACKUP"
            if [ -f "$PRE_UPDATE_BACKUP/$BINARY_NAME" ]; then
                # The old binary is still executing: copy beside it and rename
                # over it (cp onto a running executable fails with ETXTBSY —
                # found by the 2026-09-06 rollback drill).
                cp -p "$PRE_UPDATE_BACKUP/$BINARY_NAME" "$RELEASE_DIR/$BINARY_NAME.rollback"
                mv -Tf "$RELEASE_DIR/$BINARY_NAME.rollback" "$RELEASE_DIR/$BINARY_NAME"
                if [ -f "$PRE_UPDATE_BACKUP/$EXECUTOR_BINARY" ]; then
                    cp -p "$PRE_UPDATE_BACKUP/$EXECUTOR_BINARY" "$RELEASE_DIR/$EXECUTOR_BINARY.rollback"
                    mv -Tf "$RELEASE_DIR/$EXECUTOR_BINARY.rollback" "$RELEASE_DIR/$EXECUTOR_BINARY"
                fi
                if [ -d "$PRE_UPDATE_BACKUP/webapp-dist" ]; then
                    rm -rf "$RELEASE_DIR/webapp/dist"
                    cp -a "$PRE_UPDATE_BACKUP/webapp-dist" "$RELEASE_DIR/webapp/dist"
                    sync_webapp_dist_to_live_dir "$RELEASE_DIR"
                fi
                restart_service
                notify_owner "Aegis Runtime 部署冒烟失败${AEGIS_SMOKE_FORCE_FAIL:+（回滚演练）}，已自动回滚到上一版本。请看部署日志。"
                die "Smoke test FAILED — rolled back to the previous binary/bundle (auto)."
            else
                notify_owner "Aegis Runtime 部署冒烟失败，且没有可回滚的备份。请立即检查。"
                die "Smoke test FAILED and no pre-update backup found."
            fi
        fi
    fi

    ok "In-place update complete"
}

# ── webapp-update (frontend-only) ────────────────────────────────────
cmd_webapp_update() {
    require_root
    local PROJECT_ROOT RELEASE_DIR SCRIPT_DIR
    PROJECT_ROOT="$(project_root)"

    if [ ! -L "$DEPLOY_ROOT/current" ]; then
        die "No current release found. Run 'install' or 'upgrade' first."
    fi

    RELEASE_DIR="$(readlink -f "$DEPLOY_ROOT/current")"
    info "WebApp-only update of $(basename "$RELEASE_DIR")"

    build_webapp_frontend "$PROJECT_ROOT" "$BUILD_USER" required
    DEPLOY_DESTRUCTIVE_PHASE="yes"
    sync_webapp_dist_to_release "$RELEASE_DIR" "$PROJECT_ROOT"
    sync_webapp_dist_to_live_dir "$RELEASE_DIR"

    SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
    if [ -x "$SCRIPT_DIR/test-webapp-production-smoke.sh" ]; then
        info "Running WebApp production smoke test..."
        if AEGIS_ENV_FILE="${ENV_FILE:-$DEPLOY_ROOT/shared/.env}" \
            AEGIS_WEBAPP_DIST_DIR="$PROJECT_ROOT/webapp/dist" \
            bash "$SCRIPT_DIR/test-webapp-production-smoke.sh"; then
            ok "WebApp production smoke passed"
        else
            die "WebApp production smoke failed"
        fi
    fi

    ok "WebApp-only update complete"
}

# ── rollback ─────────────────────────────────────────────────────────
cmd_rollback() {
    require_root
    DEPLOY_DESTRUCTIVE_PHASE="yes"
    local CURRENT
    CURRENT="$(current_release)"

    # Find previous release
    local RELEASES
    RELEASES=($(ls -1 "$DEPLOY_ROOT/releases/" | sort -r))

    if [ ${#RELEASES[@]} -lt 2 ]; then
        die "No previous release to roll back to"
    fi

    local PREVIOUS=""
    for rel in "${RELEASES[@]}"; do
        if [ "$rel" != "$CURRENT" ]; then
            PREVIOUS="$rel"
            break
        fi
    done

    if [ -z "$PREVIOUS" ]; then
        die "No previous release found"
    fi

    info "Rolling back: $CURRENT -> $PREVIOUS"

    ln -sfn "$DEPLOY_ROOT/releases/$PREVIOUS" "$DEPLOY_ROOT/current.tmp"
    mv -Tf "$DEPLOY_ROOT/current.tmp" "$DEPLOY_ROOT/current"

    restart_service
    ok "Rolled back to release: $PREVIOUS"
}

# ── backup ───────────────────────────────────────────────────────────
encrypted_backup_bin() {
    if [ -n "${DEPLOY_BUILD_OUTPUT_DIR:-}" ] && [ -x "$DEPLOY_BUILD_OUTPUT_DIR/$BACKUP_BINARY" ]; then
        echo "$DEPLOY_BUILD_OUTPUT_DIR/$BACKUP_BINARY"
    elif [ -x "$DEPLOY_ROOT/current/$BACKUP_BINARY" ]; then
        echo "$DEPLOY_ROOT/current/$BACKUP_BINARY"
    else
        die "Encrypted backup helper missing; build this release first or use a release containing $BACKUP_BINARY"
    fi
}

backup_encrypted_database() {
    local source="$1" output="$2" backup_bin
    [ -n "${AEGIS_RUNTIME_DB_KEY_HEX:-}" ] || die "Backup needs AEGIS_RUNTIME_DB_KEY_HEX injected from outside .env; refusing an unkeyed sqlite3/cp backup"
    backup_bin="$(encrypted_backup_bin)"
    "$backup_bin" "$source" "$output" || die "Encrypted backup failed for $source; no plaintext or partial backup accepted"
    chmod 600 "$output" 2>/dev/null || warn "could not chmod 600 $output"
}

cmd_backup() {
    local DB_PATH="$DEPLOY_ROOT/shared/aegis_runtime_memory.db"
    if [ ! -f "$DB_PATH" ]; then
        warn "No database found at $DB_PATH"
        return 0
    fi

    local BACKUP_NAME="aegis_runtime_memory_$(date +%Y%m%d-%H%M%S).db"
    local BACKUP_PATH="$DEPLOY_ROOT/backups/$BACKUP_NAME"

    mkdir -p -m 750 "$DEPLOY_ROOT/backups"

    backup_encrypted_database "$DB_PATH" "$BACKUP_PATH"
    ok "Encrypted database backed up: $BACKUP_PATH ($(du -h "$BACKUP_PATH" | cut -f1))"

    cleanup_backups
}

# Phase E (W17): back up heirloom_event_log.db alongside aegis_runtime_memory.db.
# Shares the same backup directory + retention policy so operators can audit
# paired state across deploy cycles.
cmd_backup_heirloom() {
    local DB_PATH="$DEPLOY_ROOT/shared/heirloom_event_log.db"
    if [ ! -f "$DB_PATH" ]; then
        warn "No heirloom DB found at $DB_PATH"
        return 0
    fi

    local BACKUP_NAME="heirloom_event_log_$(date +%Y%m%d-%H%M%S).db"
    local BACKUP_PATH="$DEPLOY_ROOT/backups/$BACKUP_NAME"

    mkdir -p -m 750 "$DEPLOY_ROOT/backups"

    backup_encrypted_database "$DB_PATH" "$BACKUP_PATH"
    ok "Encrypted heirloom DB backed up: $BACKUP_PATH ($(du -h "$BACKUP_PATH" | cut -f1))"

    cleanup_backups
}

# ── status ───────────────────────────────────────────────────────────
cmd_status() {
    echo -e "${CYAN}=== Aegis Runtime Deployment Status ===${NC}"
    echo ""

    # Current release
    echo -e "Current release: ${GREEN}$(current_release)${NC}"

    # Available releases
    if [ -d "$DEPLOY_ROOT/releases" ]; then
        local COUNT
        COUNT=$(ls -1 "$DEPLOY_ROOT/releases/" 2>/dev/null | wc -l)
        echo "Available releases: $COUNT"
        if [ "$COUNT" -gt 0 ]; then
            ls -1 "$DEPLOY_ROOT/releases/" | sort -r | head -5 | while read -r rel; do
                if [ "$rel" = "$(current_release)" ]; then
                    echo -e "  ${GREEN}$rel${NC} (active)"
                else
                    echo "  $rel"
                fi
            done
        fi
    else
        echo "Available releases: (not installed)"
    fi

    echo ""

    # Service status — adaptive: systemd unit when the host runs systemd,
    # otherwise the portable nohup supervisor (scripts/aegis-runtime-local.sh).
    detect_service_manager
    echo "Service manager: $SERVICE_MANAGER ($SERVICE_MANAGER_REASON)"
    if [ "$SERVICE_MANAGER" != "systemd" ]; then
        local script_dir
        script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
        if [ -x "$script_dir/aegis-runtime-local.sh" ]; then
            "$script_dir/aegis-runtime-local.sh" status || true
        else
            echo -e "Service: ${YELLOW}not installed${NC} (no systemd; scripts/aegis-runtime-local.sh missing)"
        fi
        echo ""
        echo "Tunnel: n/a without systemd (run cloudflared manually if needed)"
    else
        if systemctl is-active --quiet "$SERVICE_NAME" 2>/dev/null; then
            echo -e "Service: ${GREEN}running${NC}"
            systemctl show "$SERVICE_NAME" --property=MainPID,ActiveEnterTimestamp --no-pager 2>/dev/null | sed 's/^/  /'
        elif systemctl is-enabled --quiet "$SERVICE_NAME" 2>/dev/null; then
            echo -e "Service: ${YELLOW}stopped (enabled)${NC}"
        else
            echo -e "Service: ${YELLOW}not installed${NC}"
        fi

        # Tunnel status
        echo ""
        if systemctl is-active --quiet "$TUNNEL_SERVICE" 2>/dev/null; then
            echo -e "Tunnel: ${GREEN}running${NC}"
            if [ -f "$TUNNEL_URL_FILE" ] && [ -s "$TUNNEL_URL_FILE" ]; then
                echo "  URL: $(cat "$TUNNEL_URL_FILE")"
            fi
        elif systemctl is-enabled --quiet "$TUNNEL_SERVICE" 2>/dev/null; then
            echo -e "Tunnel: ${YELLOW}stopped (enabled)${NC}"
        else
            echo -e "Tunnel: ${YELLOW}not installed${NC}"
        fi
    fi

    # Database size
    local DB_PATH="$DEPLOY_ROOT/shared/aegis_runtime_memory.db"
    if [ -f "$DB_PATH" ]; then
        echo ""
        echo "Database: $(du -h "$DB_PATH" | cut -f1)"
    fi
    # Phase E (W17): heirloom DB size (separate file, separate retention).
    local HEIRLOOM_DB_PATH="$DEPLOY_ROOT/shared/heirloom_event_log.db"
    if [ -f "$HEIRLOOM_DB_PATH" ]; then
        echo "Heirloom DB: $(du -h "$HEIRLOOM_DB_PATH" | cut -f1)"
    fi

    # Backup count
    if [ -d "$DEPLOY_ROOT/backups" ]; then
        local BACKUP_COUNT
        BACKUP_COUNT=$(ls -1 "$DEPLOY_ROOT/backups/" 2>/dev/null | wc -l)
        echo "Backups: $BACKUP_COUNT (retention: $KEEP_BACKUPS)"
    fi
}

deploy_plan_changed_files() {
    local last_sha current_sha
    last_sha=$(last_deployed_sha 2>/dev/null || echo "unknown")
    current_sha=$(git -C "$CANONICAL_REPO" rev-parse HEAD 2>/dev/null || echo "")
    if [ -z "$current_sha" ]; then
        return 1
    fi
    if [ -z "$last_sha" ] || [ "$last_sha" = "unknown" ]; then
        git -C "$CANONICAL_REPO" ls-files
        return 0
    fi
    git -C "$CANONICAL_REPO" diff --name-only "$last_sha" "$current_sha"
}

deploy_plan_recommendation() {
    local files="$1"
    if [ -z "$files" ]; then
        echo "none"
        return 0
    fi

    local webapp_only=true
    local docs_or_tests_only=true
    local has_migration=false
    local file
    while IFS= read -r file; do
        [ -z "$file" ] && continue
        case "$file" in
            webapp/*)
                docs_or_tests_only=false
                ;;
            docs/*|*.md|Makefile|.claude/coordination/DEPLOY_RULES.md|scripts/bootstrap-system-deps.sh|scripts/deploy.sh|scripts/setup-dev.sh|scripts/test-*|webapp/tests/*|core/tests/*)
                webapp_only=false
                ;;
            *)
                webapp_only=false
                docs_or_tests_only=false
                ;;
        esac
        case "$file" in
            core/src/memory_migrations.rs|core/src/credential_store.rs|core/src/memory.rs|core/src/memory/*.rs|core/src/sdui.rs)
                # memory.rs split into core/src/memory/ (Phase 0.3, 2026-07-13);
                # any submodule change keeps the migration-lock recommendation.
                has_migration=true
                ;;
        esac
    done <<< "$files"

    if $webapp_only; then
        echo "webapp-update"
    elif $docs_or_tests_only; then
        echo "no-deploy"
    elif $has_migration; then
        echo "update-with-migration-lock"
    else
        echo "update"
    fi
}

cmd_deploy_plan() {
    local files recommendation last_sha current_sha
    files="$(deploy_plan_changed_files || true)"
    recommendation="$(deploy_plan_recommendation "$files")"
    last_sha=$(last_deployed_sha 2>/dev/null || echo "unknown")
    current_sha=$(git -C "$CANONICAL_REPO" rev-parse --short HEAD 2>/dev/null || echo "unknown")

    echo -e "${CYAN}=== Aegis Runtime Deploy Plan ===${NC}"
    echo "Current HEAD: $current_sha"
    echo "Last success: $last_sha"
    echo ""
    case "$recommendation" in
        none)
            ok "No deploy needed: this SHA already matches the last successful deploy."
            ;;
        webapp-update)
            ok "Recommended: sudo scripts/deploy.sh webapp-update"
            echo "Layer: WebApp fast path — build/sync assets, no Rust build, no service restart."
            ;;
        no-deploy)
            ok "Recommended: no deploy"
            echo "Layer: docs/tests/script-only changes. Run checks and push; production runtime is unchanged."
            ;;
        update-with-migration-lock)
            warn "Recommended: AEGIS_RUNTIME_FORCE_DEPLOY=1 sudo scripts/deploy.sh update"
            echo "Layer: full runtime path with migration lock. Includes backup, Rust build, restart, smoke, and health monitor."
            ;;
        *)
            ok "Recommended: AEGIS_RUNTIME_FORCE_DEPLOY=1 sudo scripts/deploy.sh update"
            echo "Layer: core/runtime path — Rust build, restart, smoke, and health monitor."
            ;;
    esac
    echo ""
    echo "Changed files since last successful deploy:"
    if [ -z "$files" ]; then
        echo "  (none)"
    else
        printf '%s\n' "$files" | sed 's/^/  - /'
    fi
}

normalize_deploy_action() {
    local requested="$1"
    local files recommendation
    files="$(deploy_plan_changed_files || true)"
    recommendation="$(deploy_plan_recommendation "$files")"

    case "$requested" in
        auto)
            case "$recommendation" in
                none)
                    ok "Auto deploy: no deploy needed; current HEAD already matches the last successful deploy."
                    return 1
                    ;;
                no-deploy)
                    ok "Auto deploy: no production deploy needed for docs/tests/script-only changes."
                    echo "Changed files:"
                    printf '%s\n' "$files" | sed 's/^/  - /'
                    NORMALIZED_NO_DEPLOY="yes"
                    return 1
                    ;;
                webapp-update)
                    info "Auto deploy selected layer: webapp-update"
                    ACTION="webapp-update"
                    return 0
                    ;;
                update-with-migration-lock)
                    info "Auto deploy selected layer: update (migration-aware)"
                    ACTION="update"
                    return 0
                    ;;
                *)
                    info "Auto deploy selected layer: update"
                    ACTION="update"
                    return 0
                    ;;
            esac
            ;;
        update)
            if [ "$recommendation" = "webapp-update" ] && [ -z "${AEGIS_RUNTIME_FORCE_FULL_UPDATE:-}" ]; then
                info "Layer guard: update requested, but changed files are WebApp-only."
                info "Auto-routing to webapp-update. Use AEGIS_RUNTIME_FORCE_FULL_UPDATE=1 to force a full runtime deploy."
                ACTION="webapp-update"
            elif [ "$recommendation" = "no-deploy" ] && [ -z "${AEGIS_RUNTIME_FORCE_FULL_UPDATE:-}" ]; then
                ok "Layer guard: no production deploy needed for docs/tests/script-only changes."
                echo "Changed files:"
                printf '%s\n' "$files" | sed 's/^/  - /'
                NORMALIZED_NO_DEPLOY="yes"
                return 1
            fi
            ;;
        webapp-update)
            case "$recommendation" in
                webapp-update|none)
                    ;;
                no-deploy)
                    if [ -z "${AEGIS_RUNTIME_ALLOW_PARTIAL_WEBAPP_DEPLOY:-}" ]; then
                        ok "Layer guard: no production deploy needed for docs/tests/script-only changes."
                        NORMALIZED_NO_DEPLOY="yes"
                        return 1
                    fi
                    ;;
                *)
                    if [ -z "${AEGIS_RUNTIME_ALLOW_PARTIAL_WEBAPP_DEPLOY:-}" ]; then
                        die "webapp-update refused: changes since last successful deploy include runtime/core files. Run 'sudo scripts/deploy.sh auto' or full 'update'. Override only if you intentionally want a partial WebApp deploy: AEGIS_RUNTIME_ALLOW_PARTIAL_WEBAPP_DEPLOY=1 sudo scripts/deploy.sh webapp-update"
                    fi
                    warn "webapp-update override active despite recommendation=$recommendation"
                    ;;
            esac
            ;;
    esac
    return 0
}

# ── cleanup ──────────────────────────────────────────────────────────
cleanup_releases() {
    local RELEASES
    RELEASES=($(ls -1 "$DEPLOY_ROOT/releases/" | sort -r))
    local COUNT=${#RELEASES[@]}

    if [ "$COUNT" -le "$KEEP_RELEASES" ]; then
        return
    fi

    # Get current symlink target to avoid deleting the active release
    local CURRENT_TARGET=""
    if [ -L "$DEPLOY_ROOT/current" ]; then
        CURRENT_TARGET="$(basename "$(readlink -f "$DEPLOY_ROOT/current")")"
    fi

    info "Cleaning old releases (keeping $KEEP_RELEASES)..."
    local keep_non_current="$KEEP_RELEASES"
    if [ -n "$CURRENT_TARGET" ] && [ "$KEEP_RELEASES" -gt 0 ]; then
        keep_non_current=$((KEEP_RELEASES - 1))
    fi
    local kept_non_current=0
    local release
    for release in "${RELEASES[@]}"; do
        if [ "$release" = "$CURRENT_TARGET" ]; then
            continue
        fi
        if [ "$kept_non_current" -lt "$keep_non_current" ]; then
            kept_non_current=$((kept_non_current + 1))
            continue
        fi
        if [ -d "${DEPLOY_ROOT:?}/releases/$release" ]; then
            rm -rf "${DEPLOY_ROOT:?}/releases/$release"
            info "  Removed: $release"
        fi
    done
}

# ── cleanup_backups ──────────────────────────────────────────────────
# ── Deploy gates (Plan §11.0 — P0-D) ─────────────────────────────────
# Gate checks run BEFORE any destructive action. Each failure exits with
# a distinct code and a verbose multi-line message telling the caller the
# rule being enforced and exact remedy steps.

# CANONICAL_REPO is env-overridable for hermetic tests (test-p2-batch-d-trim.sh
# stubs it into a temp fixture repo). Otherwise it defaults to the main worktree.
# Default: the main worktree of the repo this script lives in (the parent of
# git's common dir), so linked worktrees still resolve to the one main checkout.
_deploy_script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
_deploy_common_dir="$(git -C "$_deploy_script_dir" rev-parse --path-format=absolute --git-common-dir 2>/dev/null || true)"
_deploy_default_repo="${_deploy_common_dir%/.git}"
[ -n "$_deploy_common_dir" ] && [ "$_deploy_default_repo" != "$_deploy_common_dir" ] || _deploy_default_repo="$(cd "$_deploy_script_dir/.." && pwd)"
CANONICAL_REPO="${AEGIS_RUNTIME_CANONICAL_REPO:-$_deploy_default_repo}"
PLAN_REF="docs/multi-cc-team-coordination-plan.md §11"
RULES_REF=".claude/coordination/DEPLOY_RULES.md"

EXIT_WRONG_WORKTREE=10
EXIT_WRONG_BRANCH=11
EXIT_DIRTY_TREE=12
EXIT_OUT_OF_SYNC=13
EXIT_MIGRATION_LOCKED=14
# Host has no usable systemd and the requested action has no portable
# equivalent (see route_without_systemd). Gate-class code: no sentinel.
EXIT_NO_SYSTEMD=15
# Owner-chat provider sign-in missing at the START of install (auth-first).
EXIT_AUTH_PREFLIGHT=16
# Runtime deploy failure (post-gate, post-lock, inside cmd_*). Distinct from
# gate-reject codes 10-14 so EXIT trap can tell them apart.
EXIT_DEPLOY_FAIL=20
# Refusal because a prior deploy failed and left the sentinel. Human must
# clear it via `unlock` or a successful `rollback` before retrying.
EXIT_DEPLOY_HELD=21

# §11.4 failure sentinel. Env-overridable for tests.
DEPLOY_FAIL_SENTINEL="${AEGIS_RUNTIME_DEPLOY_FAIL_SENTINEL:-/var/lock/aegis-runtime-deploy-failed.sentinel}"

# Set by reject_gate() just before `exit` so the EXIT trap can record
# "rejected_gate" status instead of "failed" and skip sentinel drop.
GATE_REJECTED=""

# Plan §11.5 — migration hard-lock. Any deploy carrying a migration must
# additionally serialize against /var/lock/aegis-runtime-migrations-applied.lock
# to prevent two migration-carrying deploys from interleaving their schema
# mutations. Uses FD 201 (FD 200 is already held by the deploy lock).
MIGRATION_LOCK_FILE="/var/lock/aegis-runtime-migrations-applied.lock"
# Migration detection is content-aware (see deploy_carries_migrations below).
# No single sentinel path — we scan all core/src/*.rs for DDL on each deploy.

acquire_migration_lock() {
    # Idempotent: if caller already holds the lock (FD 201 open), skip.
    if { true >&201; } 2>/dev/null; then
        return 0
    fi

    # Try to create/touch lock file. /var/lock is root-writable by default;
    # when script runs under sudo this is fine.
    if ! touch "$MIGRATION_LOCK_FILE" 2>/dev/null; then
        reject_gate $EXIT_MIGRATION_LOCKED "\
REASON: cannot create migration lock file.
  Path:  $MIGRATION_LOCK_FILE
  errno: permission denied / fs error

RULE (Plan §11.5): deploys carrying schema migrations must hold
  $MIGRATION_LOCK_FILE for the migration duration to prevent concurrent
  CC deploys from interleaving inconsistent schema mutations.

TO FIX:
  1. Ensure /var/lock is writable (normally yes under sudo).
  2. Retry."
    fi

    exec 201>"$MIGRATION_LOCK_FILE"

    # Non-blocking first; then bounded wait (migrations rarely exceed 60s).
    if flock -n 201; then
        info "Migration lock acquired: $MIGRATION_LOCK_FILE"
        return 0
    fi

    warn "Migration lock held by another deploy — waiting up to 300s..."
    if ! flock -w 300 201; then
        reject_gate $EXIT_MIGRATION_LOCKED "\
REASON: another deploy is applying migrations — timed out waiting 300s.
  Lock:   $MIGRATION_LOCK_FILE
  Holder: $(fuser "$MIGRATION_LOCK_FILE" 2>/dev/null | tr -s ' ' || echo 'unknown')

RULE (Plan §11.5): migration deploys are serialized cluster-wide.
  Only one CC may be mid-migration at a time.

TO FIX:
  1. Check the holder process: ps -fp <pid>
  2. If stale: fuser -k $MIGRATION_LOCK_FILE   (kill stale holder)
  3. Retry."
    fi
    info "Migration lock acquired after wait: $MIGRATION_LOCK_FILE"
}

# Return 0 (migrations present) / 1 (no migrations) / 2 (unknown — be safe)
# Heuristic: compare sentinel file between last successfully-deployed sha
# (from deploy-log.jsonl) and current HEAD. If they differ → migrations may
# have changed → acquire the lock. If no prior deploy log, assume YES (safe).
deploy_carries_migrations() {
    # Content-aware detection per CTO review 2026-04-13: schema DDL lives in
    # multiple files (memory.rs, credential_store.rs, sdui.rs,
    # memory_migrations.rs, and future additions). A single sentinel file is
    # an underfit heuristic. Instead: diff all changed core/src/*.rs between
    # last-deployed sha and current HEAD; if any changed file contains
    # CREATE TABLE / ALTER TABLE / DROP TABLE patterns, assume migration.
    # Matches feedback_no_static_matching.md — structural over keyword-listing.
    local last_sha
    last_sha=$(last_deployed_sha 2>/dev/null || echo "")
    if [ -z "$last_sha" ] || [ "$last_sha" = "unknown" ]; then
        # Never deployed (or log empty) — conservative: assume migrations.
        return 0
    fi

    local current_sha
    current_sha=$(git -C "$CANONICAL_REPO" rev-parse HEAD 2>/dev/null || echo "")
    if [ -z "$current_sha" ] || [ "$current_sha" = "$last_sha" ]; then
        return 1   # identical tree
    fi

    # List changed .rs files under core/src/, then check each for DDL patterns.
    # Using `git diff --name-only` + `git show` avoids touching the working tree.
    local changed_files f
    changed_files=$(git -C "$CANONICAL_REPO" diff --name-only "$last_sha" "$current_sha" -- 'core/src/*.rs' 2>/dev/null)
    [ -z "$changed_files" ] && return 1   # no .rs changes in core

    while IFS= read -r f; do
        [ -z "$f" ] && continue
        # Check BOTH before + after content for DDL — a migration could be
        # added OR removed (even removal shouldn't race with peers).
        if git -C "$CANONICAL_REPO" show "$current_sha:$f" 2>/dev/null | \
             grep -qE 'CREATE[[:space:]]+TABLE|ALTER[[:space:]]+TABLE|DROP[[:space:]]+TABLE'; then
            return 0
        fi
        if git -C "$CANONICAL_REPO" show "$last_sha:$f" 2>/dev/null | \
             grep -qE 'CREATE[[:space:]]+TABLE|ALTER[[:space:]]+TABLE|DROP[[:space:]]+TABLE'; then
            return 0
        fi
    done <<< "$changed_files"

    return 1
}

# Source session-lib.sh for sesslib_bark_notify + sesslib_write_ceo_inbox.
# If the lib is missing (older repo / standalone deploy), stub them so Gap 4
# escalation silently no-ops rather than breaking the deploy gate.
# shellcheck disable=SC1091
if [ -r "$CANONICAL_REPO/.claude/hooks/lib/session-lib.sh" ]; then
    source "$CANONICAL_REPO/.claude/hooks/lib/session-lib.sh" 2>/dev/null || true
fi
if ! command -v sesslib_bark_notify >/dev/null 2>&1; then
    sesslib_bark_notify() { :; }  # no-op fallback
fi
if ! command -v sesslib_write_ceo_inbox >/dev/null 2>&1; then
    sesslib_write_ceo_inbox() { :; }  # no-op fallback
fi

# Gap 4 escalation: called from Gate 3 BEFORE the gate reject exit.
# Pushes a bark notification AND writes a ceo-inbox blocking item so the
# user sees the problem immediately, not hours later.
gap4_escalate_dirty() {
    local dirty_list="$1"
    local short_list
    short_list=$(printf '%s\n' "$dirty_list" | head -n 5 | sed 's/^/  /')
    local bark_body="main dirty — all peer deploys blocked. Files: $(printf '%s' "$dirty_list" | head -n 3 | tr '\n' ' ' | cut -c1-120)"
    # Bark push (best-effort, non-blocking)
    sesslib_bark_notify "aegis-runtime: deploy blocked" "$bark_body" "" 2>/dev/null || true

    # Ceo-inbox entry (so /decide flow has somewhere to resolve it)
    local tmp
    tmp=$(mktemp 2>/dev/null) || return 0
    {
        echo "# Main worktree dirty — deploy gate #3 REJECTED"
        echo
        echo "Plan §15.10 (Gap 4): peer deploys are blocked because the"
        echo "canonical main worktree ($CANONICAL_REPO) has uncommitted"
        echo "changes in tracked files."
        echo
        echo "## Dirty files"
        echo
        echo '```'
        printf '%s\n' "$short_list"
        echo '```'
        echo
        echo "## Resolution"
        echo
        echo "- Commit or stash the edits and retry deploy, OR"
        echo "- Run '/decide main-dirty-$(date -u +%Y%m%dT%H%M%SZ) force-cleanup'"
        echo "  to let aegis-runtime auto-stash and retry."
    } > "$tmp"
    sesslib_write_ceo_inbox "blocking" "main-dirty-$(date -u +%Y%m%dT%H%M%SZ)" \
        "$CANONICAL_REPO" "deploy-gate" "$tmp" 2>/dev/null || true
    rm -f "$tmp" 2>/dev/null
    return 0
}

reject_gate() {
    local code="$1"; shift
    # Signal to EXIT trap: this is a gate reject, not a runtime deploy failure.
    # Peer CCs/observers should see a distinct log status and NO §11.4 sentinel
    # drop (gate rejects are recoverable by the caller without human unlock).
    GATE_REJECTED="yes"
    GATE_REJECT_CODE="$code"
    echo -e "${RED}═══════════════════════════════════════════════════════${NC}" >&2
    echo -e "${RED}[deploy] DEPLOY REJECTED${NC}" >&2
    echo -e "${RED}═══════════════════════════════════════════════════════${NC}" >&2
    echo "$@" >&2
    echo -e "${RED}───────────────────────────────────────────────────────${NC}" >&2
    echo "Full rules: $PLAN_REF  |  Quick ref: $RULES_REF" >&2
    echo -e "${RED}═══════════════════════════════════════════════════════${NC}" >&2
    exit "$code"
}

run_deploy_gates() {
    local action="$1"

    # Gate 1: canonical main worktree
    if [ "$(pwd)" != "$CANONICAL_REPO" ]; then
        reject_gate $EXIT_WRONG_WORKTREE "\
REASON: deploy.sh must run from the canonical main worktree.
  Current directory: $(pwd)
  Required:          $CANONICAL_REPO

RULE: Deploys happen only from main, and main lives only at $CANONICAL_REPO.
  Feature-branch worktrees under .claude/worktrees/<name>/ cannot deploy.
  This makes it physically impossible to deploy feature-branch code by
  mistake.

TO FIX:
  1. If your branch is not yet merged to main, merge it first:
       # from your feature worktree:
       git push origin HEAD:main
  2. Switch to canonical:
       cd $CANONICAL_REPO
  3. Fast-forward local main:
       git pull --ff-only origin main
  4. Retry:
       sudo scripts/deploy.sh $action"
    fi

    # Gate 2: on main branch
    local current_branch
    current_branch=$(git rev-parse --abbrev-ref HEAD 2>/dev/null || echo "detached")
    if [ "$current_branch" != "main" ]; then
        reject_gate $EXIT_WRONG_BRANCH "\
REASON: deploy.sh requires HEAD = main on the canonical worktree.
  Current branch: $current_branch
  Required:       main

RULE: $CANONICAL_REPO is the custodian worktree — reserved for merges
  and deploys. It must always be on main.

TO FIX:
  1. Stash/commit any uncommitted work:
       git stash                   (shelve — later: git stash pop)
       # OR: git commit -am \"...\" (if edits belong on main)
  2. Switch to main:
       git checkout main
  3. Pull latest:
       git pull --ff-only origin main
  4. Retry."
    fi

    # Gate 3: working tree clean wrt TRACKED files.
    # Untracked files (??) are ignored: they're not in the deploy artifact
    # anyway, and rejecting on every scratch doc was observed to false-positive
    # in post-P0 verify. Use --untracked-files=no to scope to modified/staged
    # tracked files only (the actual "WIP that shouldn't ship" signal).
    local dirty
    dirty=$(git status --porcelain --untracked-files=no 2>/dev/null || echo "")
    if [ -n "$dirty" ]; then
        # Gap 4 (Plan §15.10): loudly escalate to user so they know main
        # is blocking all peer deploys — bark push + ceo-inbox entry.
        # Best-effort; never fail the gate reject itself.
        gap4_escalate_dirty "$dirty" || true
        reject_gate $EXIT_DIRTY_TREE "\
REASON: deploy.sh refuses when tracked files have uncommitted changes.
  Modified/staged tracked files in $CANONICAL_REPO:
$(echo "$dirty" | sed 's/^/    /')

RULE: The main worktree is production's source of truth. No WIP ships.
  (Untracked scratch files in the tree are OK — they don't reach production.
  If you ARE working on untracked files intentionally, gitignore them or
  commit them before deploying.)

TO FIX (pick one):
  A) Commit or stash (preferred):
       git stash                   (shelve for later)
    OR git add <files> && git commit -m \"...\"
  B) Discard (DESTRUCTIVE — loses edits):
       git restore .
  Then retry deploy.

WARNING: if these edits are NOT yours, STOP — another CC may have touched
  the main worktree. Investigate via /roster and broadcasts/ before forcing through."
    fi

    # Gate 4: local == origin/main
    local fetch_ok=true
    git fetch --quiet origin main 2>/dev/null || fetch_ok=false
    local local_sha remote_sha
    local_sha=$(git rev-parse HEAD 2>/dev/null || echo "")
    remote_sha=$(git rev-parse origin/main 2>/dev/null || echo "")

    if [ -z "$remote_sha" ]; then
        reject_gate $EXIT_OUT_OF_SYNC "\
REASON: cannot determine origin/main ref (fetch_ok=$fetch_ok).
  Local HEAD:    $local_sha
  origin/main:   (unknown — ref not found)

RULE: deploy.sh requires a known origin/main to verify parity. If fetch
  failed (offline, DNS, creds) or origin/main was never synced on this
  clone, we cannot proceed safely.

TO FIX:
  1. Verify network + origin URL:   git remote -v
  2. Manual fetch:                   git fetch origin main
  3. If a fresh clone: ensure origin/main exists:  git ls-remote origin main
  4. Retry deploy once fetch succeeds."
    fi

    if [ "$local_sha" != "$remote_sha" ]; then
        reject_gate $EXIT_OUT_OF_SYNC "\
REASON: local main is out of sync with origin/main.
  Local HEAD:    $local_sha
  origin/main:   $remote_sha

RULE: deploy.sh ships the exact commit at local HEAD. Divergence from
  origin means something was pushed while you were preparing; reconcile
  so production reflects the actual tip of origin/main, not a stale view.

TO FIX:
  1. Fast-forward pull:
       git pull --ff-only origin main
  2a. If pull succeeds — retry deploy. (A peer CC probably merged while
      you were setting up; their code will now be included in your deploy.)
  2b. If pull FAILS (your local has commits origin does not) — STOP.
      git log HEAD..origin/main --oneline   (incoming)
      git log origin/main..HEAD --oneline   (local-only)
      Decide: rebase or reset? Do NOT deploy until reconciled.
      If a peer force-pushed main, this is anomalous — post to ceo-inbox."
    fi

    # All 4 gates passed
    info "Deploy gates 1-4 PASSED (action=$action, DEPLOY_SH_VERSION=$DEPLOY_SH_VERSION)"
}

cmd_verify() {
    # Gates were already run by dispatch for verify; if we got here they passed.
    ok "All 4 deploy gates PASSED. Dry-run complete — no deploy performed."
    ok "DEPLOY_SH_VERSION=$DEPLOY_SH_VERSION"
}

# ── Tier-2 full-suite freshness preflight (2026-07-10) ──────────────
# Tier 1 (commit gate) is selective and Tier 3 (nightly) is the only
# FULL run — this check keeps Rust-shipping deploys honest: if no FULL
# green run (stamp from scripts/full-suite.sh) covers an ancestor of
# HEAD within 24h, the deploy is refused. AEGIS_RUNTIME_SKIP_FULL_SUITE_CHECK
# does not bypass it. AEGIS_RUNTIME_TIER2_INLINE=1 may run the lib suite and
# proceed only when that suite is green.
FULL_SUITE_STAMP="${AEGIS_RUNTIME_FULL_SUITE_STAMP:-/opt/aegis-runtime/shared/.full-suite-green-stamp}"
check_full_suite_freshness() {
    if [ "${AEGIS_RUNTIME_SKIP_FULL_SUITE_CHECK:-}" = "1" ]; then
        die "Tier-2 full-suite freshness check cannot be skipped (AEGIS_RUNTIME_SKIP_FULL_SUITE_CHECK is refused)"
    fi
    local why="" stamp_epoch="" stamp_sha="" now age_h
    if [ ! -f "$FULL_SUITE_STAMP" ]; then
        why="stamp missing ($FULL_SUITE_STAMP)"
    else
        stamp_epoch=$(sed -n 's/^epoch=//p' "$FULL_SUITE_STAMP" | head -n1)
        stamp_sha=$(sed -n 's/^sha=//p' "$FULL_SUITE_STAMP" | head -n1)
        now=$(date +%s)
        if ! printf '%s' "$stamp_epoch" | grep -qE '^[0-9]+$'; then
            why="stamp unparseable (epoch='$stamp_epoch')"
        elif [ $((now - stamp_epoch)) -gt 86400 ]; then
            age_h=$(( (now - stamp_epoch) / 3600 ))
            why="stamp older than 24h (${age_h}h old)"
        elif [ -z "$stamp_sha" ] || ! git -C "$CANONICAL_REPO" merge-base --is-ancestor "$stamp_sha" HEAD 2>/dev/null; then
            why="stamp sha '${stamp_sha:-<none>}' is not an ancestor of HEAD"
        fi
    fi
    if [ -z "$why" ]; then
        info "Tier-2: full-suite stamp fresh (sha=$stamp_sha) — OK"
        return 0
    fi
    if [ "${AEGIS_RUNTIME_TIER2_INLINE:-0}" != "1" ]; then
        die "Tier-2: refusing deploy ($why). A fresh full-suite stamp is required; AEGIS_RUNTIME_SKIP_FULL_SUITE_CHECK does not bypass this."
    fi
    # Inline escape hatch (owner-invoked or nightly-adjacent runs only).
    # Isolated CARGO_TARGET_DIR (2026-06-01 cross-tree contamination
    # lesson): never test in .shared-target where interactive sessions
    # build. Fixed path, reused warm across deploys.
    local tier2_target="${AEGIS_RUNTIME_BUILD_CACHE:-/opt/aegis-runtime/build-cache}/tier2-test-target"
    mkdir -p "$tier2_target" 2>/dev/null || true
    chown -R "$BUILD_USER:$BUILD_USER" "$tier2_target" 2>/dev/null || true
    if ! sudo -u "$BUILD_USER" bash -c "source \$HOME/.cargo/env 2>/dev/null; cd '$CANONICAL_REPO' && CARGO_TARGET_DIR='$tier2_target' SKIP_SKILL_BUILD=1 cargo test -p aegis-runtime-core --lib" >&2; then
        die "Tier-2: inline full lib suite FAILED — fix the red tests (or investigate the nightly report) before deploying."
    fi
    ok "Tier-2: inline full lib suite green — proceeding."
}

# ── Deploy log writes (P0.6 Gap 4, Plan §11.1 Phase 5) ──────────────
# Append-only JSON lines record of every destructive-command invocation
# result. Location: .claude/coordination/deploy-log.jsonl (gitignored).
# Consumers (P1+): deploy history / dashboard / rollback-target suggestion.

DEPLOY_LOG_FILE="$CANONICAL_REPO/.claude/coordination/deploy-log.jsonl"
DEPLOY_LOG_WRITTEN=""   # set to "yes" once we've recorded — prevents dup-writes from trap
# Set to "yes" the moment a command enters its destructive phase (service
# stop / payload activation / live-dir mutation). Signal aborts BEFORE this
# point are provably non-destructive (build/validate only) and do not drop
# the §11.4 sentinel; aborts AFTER it force human review.
DEPLOY_DESTRUCTIVE_PHASE=""

# Read last-deployed sha from the most recent success entry in deploy-log.
# Pure-bash JSON-lite parse; returns "unknown" on any failure.
last_deployed_sha() {
    [ -f "$DEPLOY_LOG_FILE" ] || { echo "unknown"; return 0; }
    local line
    line=$(grep '"status":"success"' "$DEPLOY_LOG_FILE" 2>/dev/null | tail -n 1)
    [ -z "$line" ] && { echo "unknown"; return 0; }
    # Extract sha field (quoted string after "sha":)
    local sha
    sha=$(printf '%s' "$line" | sed -n 's/.*"sha":"\([^"]*\)".*/\1/p')
    [ -z "$sha" ] && { echo "unknown"; return 0; }
    echo "$sha"
}

# Plan §11.1 Phase C motivation / P2 item: before running tests + restart,
# consult last 10 deploy-log entries. If another CC successfully deployed
# the same sha within the last 5 minutes, skip this redundant deploy.
# Returns 0 if redundant (caller should exit 0), 1 if not redundant.
REDUNDANT_DEPLOY_WINDOW_SECS=300
check_redundant_deploy() {
    [ -f "$DEPLOY_LOG_FILE" ] || return 1

    local current_sha
    current_sha=$(git -C "$CANONICAL_REPO" rev-parse HEAD 2>/dev/null || echo "")
    [ -z "$current_sha" ] && return 1

    local now_epoch
    now_epoch=$(date -u +%s)

    local line matched_ts matched_user matched_sha
    # Walk last 10 entries, newest last — we want the most recent success for our sha.
    # Use tail to bound; entries are append-only so order-in-file == chronological.
    while IFS= read -r line; do
        [ -z "$line" ] && continue
        # Only consider success lines for our sha
        case "$line" in
            *'"status":"success"'*"\"sha\":\"$current_sha\""*) : ;;
            *) continue ;;
        esac
        matched_ts=$(printf '%s' "$line" | sed -n 's/.*"ts":"\([^"]*\)".*/\1/p')
        matched_user=$(printf '%s' "$line" | sed -n 's/.*"user":"\([^"]*\)".*/\1/p')
        matched_sha="$current_sha"
    done < <(tail -n 10 "$DEPLOY_LOG_FILE" 2>/dev/null)

    [ -z "${matched_ts:-}" ] && return 1

    # Compute age in seconds
    local matched_epoch age
    matched_epoch=$(date -u -d "$matched_ts" +%s 2>/dev/null || echo "0")
    [ "$matched_epoch" = "0" ] && return 1
    age=$(( now_epoch - matched_epoch ))

    if [ "$age" -lt "$REDUNDANT_DEPLOY_WINDOW_SECS" ]; then
        ok "Redundant deploy skipped: sha=$matched_sha already deployed ${age}s ago by ${matched_user:-unknown}"
        info "  (threshold: ${REDUNDANT_DEPLOY_WINDOW_SECS}s — see Plan §11.1 Phase C)"
        info "  Force re-deploy: AEGIS_RUNTIME_FORCE_DEPLOY=1 sudo scripts/deploy.sh ..."
        # Record a no-op success entry so observers know we ran and skipped.
        DEPLOY_LOG_WRITTEN="yes"   # prevent trap from writing "failed"
        local iso safe_sha
        iso=$(date -u +"%Y-%m-%dT%H:%M:%SZ")
        safe_sha="${current_sha//\"/}"
        mkdir -p "$(dirname "$DEPLOY_LOG_FILE")" 2>/dev/null || true
        printf '{"ts":"%s","user":"%s","action":"%s","status":"skipped_redundant","sha":"%s","release":"-","deploy_sh_version":%d}\n' \
            "$iso" "$(id -un 2>/dev/null || echo unknown)" "${ACTION:-unknown}" "$safe_sha" "$DEPLOY_SH_VERSION" \
            >> "$DEPLOY_LOG_FILE" 2>/dev/null || true
        return 0
    fi
    return 1
}

write_deploy_log() {
    local status="$1"  # success | failed | rejected_gate | flap_detected
    # Guard: don't double-log (success path writes; trap would write "failed" again)
    [ "$DEPLOY_LOG_WRITTEN" = "yes" ] && return 0
    DEPLOY_LOG_WRITTEN="yes"

    mkdir -p "$(dirname "$DEPLOY_LOG_FILE")" 2>/dev/null || true

    local iso user sha release_ts
    iso=$(date -u +"%Y-%m-%dT%H:%M:%SZ")
    user=$(id -un 2>/dev/null || echo "unknown")
    sha=$(git -C "$CANONICAL_REPO" rev-parse HEAD 2>/dev/null || echo "unknown")
    release_ts=$(basename "$(readlink -f "$DEPLOY_ROOT/current" 2>/dev/null)" 2>/dev/null || echo "")
    # basename of empty string returns "." — normalize to "none" for clarity
    [ "$release_ts" = "." ] && release_ts="none"

    # Strip any " or \ from fields to keep JSON valid (matches session-lib's policy)
    local safe_user safe_sha safe_rel
    safe_user="${user//\"/}"; safe_user="${safe_user//\\/}"
    safe_sha="${sha//\"/}"; safe_sha="${safe_sha//\\/}"
    safe_rel="${release_ts//\"/}"; safe_rel="${safe_rel//\\/}"

    printf '{"ts":"%s","user":"%s","action":"%s","status":"%s","sha":"%s","release":"%s","deploy_sh_version":%d}\n' \
        "$iso" "$safe_user" "${ACTION:-unknown}" "$status" "$safe_sha" "$safe_rel" "$DEPLOY_SH_VERSION" \
        >> "$DEPLOY_LOG_FILE" 2>/dev/null || true
}

# ── §11.4 failure sentinel helpers ───────────────────────────────────
# When a destructive deploy (upgrade/update/webapp-update/install) fails AFTER the gates
# have passed (i.e., a real runtime failure: cargo build, systemctl restart,
# post-deploy health flap), drop a sentinel. Subsequent destructive deploys
# refuse-at-dispatch until a human clears it via `unlock` or runs a
# successful `rollback`.
#
# Rationale (Plan §11.4): a failing deploy must not look like throughput —
# it must force human review. A flock held for one process lifetime cannot
# enforce this across invocations, so we use a persistent filesystem marker.
drop_fail_sentinel() {
    local reason="$1"
    # Best-effort; never abort the trap over this.
    mkdir -p "$(dirname "$DEPLOY_FAIL_SENTINEL")" 2>/dev/null || true
    {
        echo "ts=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
        echo "action=${ACTION:-unknown}"
        echo "reason=$reason"
        echo "sha=$(git -C "$CANONICAL_REPO" rev-parse HEAD 2>/dev/null || echo unknown)"
        echo "user=$(id -un 2>/dev/null || echo unknown)"
        echo "deploy_sh_version=$DEPLOY_SH_VERSION"
        echo "clear_via=\"sudo scripts/deploy.sh unlock\" (after human review) OR a successful \"rollback\""
    } > "$DEPLOY_FAIL_SENTINEL" 2>/dev/null || true
}

clear_fail_sentinel() {
    rm -f "$DEPLOY_FAIL_SENTINEL" 2>/dev/null || true
}

# Called at dispatch-top for destructive actions. If the sentinel exists,
# refuse with EXIT_DEPLOY_HELD and explain how to clear.
enforce_fail_sentinel() {
    [ -f "$DEPLOY_FAIL_SENTINEL" ] || return 0
    echo -e "${RED}═══════════════════════════════════════════════════════${NC}" >&2
    echo -e "${RED}[deploy] DEPLOY LOCK HELD (Plan §11.4)${NC}" >&2
    echo -e "${RED}═══════════════════════════════════════════════════════${NC}" >&2
    echo "A prior deploy failed and left a persistent failure marker:" >&2
    echo "  $DEPLOY_FAIL_SENTINEL" >&2
    echo "" >&2
    sed 's/^/  /' "$DEPLOY_FAIL_SENTINEL" 2>/dev/null >&2 || true
    echo "" >&2
    echo "RULE: §11.4 holds the deploy lock across process exits until a human" >&2
    echo "  reviews what happened. This prevents silently re-deploying over a" >&2
    echo "  broken state." >&2
    echo "" >&2
    echo "TO CLEAR (pick one):" >&2
    echo "  A) Inspect + manually unlock:" >&2
    echo "       cat $DEPLOY_FAIL_SENTINEL" >&2
    echo "       # investigate journalctl, deploy-log.jsonl, broadcasts/" >&2
    echo "       sudo scripts/deploy.sh unlock" >&2
    echo "  B) Roll back (auto-clears sentinel on success):" >&2
    echo "       sudo scripts/deploy.sh rollback" >&2
    echo -e "${RED}═══════════════════════════════════════════════════════${NC}" >&2
    exit "$EXIT_DEPLOY_HELD"
}

# ── §B.3 post-deploy risk-tiered health monitor ──────────────────────
# After a successful upgrade/update, sleep HEALTH_MONITOR_WARMUP_SEC so
# systemd can finish its `activating` → `active` transition, then sample
# health every HEALTH_MONITOR_INTERVAL_SEC for HEALTH_MONITOR_DURATION_SEC.
# Each sample classifies the service state as:
#   healthy   — `systemctl is-active` = active + loopback /health is 2xx/3xx
#               (a 2xx body with "status":"degraded" is healthy + a WARNING)
#   transient — state = activating|reloading|deactivating (NOT counted)
#   unhealthy — state = inactive|failed|unknown, OR /health returns 4xx/5xx
# A transient sample is a strong signal that warm-up was too short; we log
# and skip it, but don't fail on it. This avoids the well-known systemd
# quirk where `is-active --quiet` briefly reports non-zero right after
# `systemctl restart` returns (CTO review 2026-04-14 flagged this as a
# latent false-flap source).
#
# If unhealthy_samples >= HEALTH_MONITOR_FLAP_THRESHOLD, declare a flap:
# log flap_detected, drop §11.4 sentinel, return non-zero. Does NOT
# restart or rollback — that's a human decision.
#
# Profiles:
#   standard — normal runtime deploy: 20s warm-up + 120s monitor
#   strict   — migration/unknown-risk deploy: 30s warm-up + 300s monitor
#   fast     — operator-selected emergency speed path: 10s warm-up + 60s monitor
#
# Tunables (env-overridable for tests/operators):
#   AEGIS_RUNTIME_DEPLOY_HEALTH_PROFILE=standard|strict|fast
#   HEALTH_MONITOR_WARMUP_SEC
#   HEALTH_MONITOR_DURATION_SEC
#   HEALTH_MONITOR_INTERVAL_SEC
#   HEALTH_MONITOR_FLAP_THRESHOLD
#   AEGIS_HEALTH_PORT   (default: AXUM_PORT from shared/.env, else 3000)
#   AEGIS_HEALTH_PATH   (default: /health)
#
# Return codes from _health_sample: 0 healthy, 1 unhealthy, 2 transient.
_health_sample() {
    local state
    # `systemctl is-active` prints the state to stdout and exits 0 only
    # if state=="active". We need the string to discriminate transient
    # (activating/reloading) from truly-unhealthy (failed/inactive).
    state=$(systemctl is-active "$SERVICE_NAME" 2>/dev/null || true)

    # Test-fixture override. `yes` → active, `no` → failed, `transient` →
    # activating. Only consulted when set; otherwise we trust systemctl.
    case "${HEALTH_SERVICE_ACTIVE:-}" in
        yes)       state="active" ;;
        no)        state="failed" ;;
        transient) state="activating" ;;
    esac

    case "$state" in
        active)                            ;; # fall through to curl probe
        activating|reloading|deactivating) return 2 ;;
        *)                                 return 1 ;;
    esac

    # Loopback HTTP probe — only when state=active, since /health against a
    # still-activating process is meaningless. The port is AEGIS_HEALTH_PORT,
    # else AXUM_PORT from the deployed env file (default 3000), so the probe
    # runs on a stock install instead of silently never firing. Only an HTTP
    # failure (4xx/5xx) counts as unhealthy; a 2xx JSON body that carries
    # "status":"degraded" is a WARNING for the operator, never a flap signal.
    local port
    port="$(health_probe_port)"
    if command -v curl >/dev/null 2>&1 && [ -n "$port" ]; then
        local path="${AEGIS_HEALTH_PATH:-/health}"
        local code body_file rc=0
        body_file="$(mktemp "${TMPDIR:-/tmp}/aegis-health-body.XXXXXX" 2>/dev/null || echo "")"
        # curl prints "000" through -w even when the connection fails, so the
        # fallback must replace, not append (the old `|| echo 000` yielded
        # "000000" and mis-filed a refused connection as unhealthy).
        code=$(curl -sS -o "${body_file:-/dev/null}" -w '%{http_code}' --max-time 3 \
            "http://127.0.0.1:${port}${path}" 2>/dev/null) || code="000"
        case "$code" in
            2??|3??)
                if [ -n "$body_file" ] && health_body_is_degraded "$body_file"; then
                    warn "§B.3 WARNING: ${path} answered HTTP ${code} with status=degraded — service is up; check the console diagnostics (not counted as unhealthy)."
                fi
                ;;
            000)
                # No answer (refused/timeout): the HTTP check is optional and
                # skipped for this sample, as before.
                info "§B.3 loopback probe http://127.0.0.1:${port}${path} gave no answer — HTTP check skipped for this sample."
                ;;
            *)
                rc=1
                ;;
        esac
        [ -n "$body_file" ] && rm -f "$body_file"
        return "$rc"
    fi
    return 0
}

# Port for the §B.3 loopback probe: AEGIS_HEALTH_PORT wins; otherwise
# AXUM_PORT from the deployed env file (the value the service binds), else 3000.
health_probe_port() {
    if [ -n "${AEGIS_HEALTH_PORT:-}" ]; then
        echo "$AEGIS_HEALTH_PORT"
        return 0
    fi
    local env_path="${ENV_FILE:-$DEPLOY_ROOT/shared/.env}" port=""
    port=$(grep -s '^AXUM_PORT=' "$env_path" | tail -1 | cut -d= -f2- | tr -d '[:space:]"'"'"'')
    case "$port" in
        ''|*[!0-9]*) port=3000 ;;
    esac
    echo "$port"
}

# True when a /health JSON body reports "status":"degraded" (whitespace-tolerant).
health_body_is_degraded() {
    grep -Eq '"status"[[:space:]]*:[[:space:]]*"degraded"' "$1" 2>/dev/null
}

deploy_health_monitor_profile() {
    if [ -n "${AEGIS_RUNTIME_DEPLOY_HEALTH_PROFILE:-}" ]; then
        echo "$AEGIS_RUNTIME_DEPLOY_HEALTH_PROFILE"
        return 0
    fi
    if deploy_carries_migrations; then
        echo "strict"
    else
        echo "standard"
    fi
}

run_health_monitor() {
    local profile default_warmup default_duration default_interval default_threshold
    profile="$(deploy_health_monitor_profile)"
    case "$profile" in
        fast)
            default_warmup=10
            default_duration=60
            default_interval=10
            default_threshold=2
            ;;
        strict)
            default_warmup=30
            default_duration=300
            default_interval=20
            default_threshold=2
            ;;
        standard|"")
            profile="standard"
            default_warmup=20
            default_duration=120
            default_interval=20
            default_threshold=2
            ;;
        *)
            warn "Unknown AEGIS_RUNTIME_DEPLOY_HEALTH_PROFILE=$profile; using strict monitor."
            profile="strict"
            default_warmup=30
            default_duration=300
            default_interval=20
            default_threshold=2
            ;;
    esac

    local warmup="${HEALTH_MONITOR_WARMUP_SEC:-$default_warmup}"
    local duration="${HEALTH_MONITOR_DURATION_SEC:-$default_duration}"
    local interval="${HEALTH_MONITOR_INTERVAL_SEC:-$default_interval}"
    local threshold="${HEALTH_MONITOR_FLAP_THRESHOLD:-$default_threshold}"

    # Guard against nonsensical config.
    if [ "$duration" -le 0 ] || [ "$interval" -le 0 ]; then
        info "Post-deploy health monitor disabled (duration=$duration interval=$interval)."
        return 0
    fi

    if [ "$warmup" -gt 0 ]; then
        info "§B.3 ${profile} warm-up: sleeping ${warmup}s to let systemd reach steady state before sampling."
        sleep "$warmup"
    fi

    local samples_total=$(( duration / interval ))
    [ "$samples_total" -lt 1 ] && samples_total=1

    info "§B.3 post-deploy health monitor (${profile}): ${samples_total} samples over ${duration}s (interval ${interval}s, flap threshold ${threshold})."

    local i=0 unhealthy=0 transient=0
    while [ "$i" -lt "$samples_total" ]; do
        _health_sample
        local rc=$?
        case "$rc" in
            0) : ;;
            2)
                transient=$((transient + 1))
                info "§B.3 transient sample at t=$((i * interval))s (systemd activating/reloading) — not counted"
                ;;
            *)
                unhealthy=$((unhealthy + 1))
                warn "§B.3 unhealthy sample ${unhealthy}/${threshold} at t=$((i * interval))s"
                ;;
        esac
        if [ "$unhealthy" -ge "$threshold" ]; then
            warn "§B.3 flap detected: ${unhealthy} unhealthy samples >= threshold ${threshold}."
            write_deploy_log "flap_detected"
            drop_fail_sentinel "post-deploy health flap (${unhealthy} unhealthy samples of $((i + 1)) taken, ${transient} transient skipped)"
            return 1
        fi
        i=$((i + 1))
        # Don't sleep after the final sample.
        if [ "$i" -lt "$samples_total" ]; then
            sleep "$interval"
        fi
    done

    ok "§B.3 health monitor: ${samples_total} samples OK (${transient} transient skipped, ${unhealthy} unhealthy, threshold ${threshold})."
    return 0
}

# Traps (v11 invariant: traps NEVER write "success" — the sole success writer
# is the explicit completion point at the script tail, which runs only after
# the command handler and, for update/upgrade, after the §B.3 monitor +
# checklist). For a destructive command with no record yet:
#   - _deploy_signal_trap (TERM/INT/HUP): status=aborted_signal. Drops the
#     §11.4 sentinel ONLY when DEPLOY_DESTRUCTIVE_PHASE=yes (service stop /
#     payload activation / live-dir mutation reached) — build-phase aborts
#     are non-destructive and stay retryable without unlock. NOTE: the phase
#     flag is deliberately never reset, so a SIGTERM during the §B.3 monitor
#     window (the longest sleep in the script) also drops the sentinel even
#     though the swap+restart succeeded: an interrupted verification means
#     unknown health, and forcing `unlock` review is the fail-safe choice.
#     SIGKILL is uncatchable — a SIGKILL'd deploy leaves NO record and NO
#     sentinel; that residual is inherent to bash and intentionally accepted.
#   - _deploy_exit_trap (EXIT), gate reject (GATE_REJECTED=yes, exit 10-14):
#     status=rejected_gate, NO sentinel (caller fixes and retries).
#   - _deploy_exit_trap, runtime failure (rc != 0): status=failed + sentinel.
#   - _deploy_exit_trap, rc == 0 with no record: status=exited_unrecorded —
#     an ANOMALY, never trusted as a deploy: last_deployed_sha() and
#     check_redundant_deploy() match only "success", so a phantom can no
#     longer poison rollback-target selection or the 300s dedup window
#     (2026-07-02 incident).
_deploy_signal_trap() {
    case "${ACTION:-}" in
        install|upgrade|update|webapp-update|rollback)
            if [ "$DEPLOY_LOG_WRITTEN" != "yes" ]; then
                write_deploy_log "aborted_signal"
                if [ "$DEPLOY_DESTRUCTIVE_PHASE" = "yes" ]; then
                    drop_fail_sentinel "aborted by signal during destructive phase (action=${ACTION:-unknown})"
                fi
            fi
            ;;
    esac
    trap - EXIT
    exit 143
}

_deploy_exit_trap() {
    local rc=$?
    case "${ACTION:-}" in
        install|upgrade|update|webapp-update|rollback)
            if [ "$DEPLOY_LOG_WRITTEN" != "yes" ]; then
                if [ "$rc" -eq 0 ]; then
                    # v11 invariant: the trap NEVER writes "success". A real
                    # success is written explicitly by the completion point at
                    # the bottom of the script; reaching EXIT rc=0 without it
                    # means some path exited early — record the anomaly with a
                    # status that last_deployed_sha / check_redundant_deploy
                    # (which match only "success") will ignore. Motivated by
                    # the 2026-07-02 phantom: a killed deploy left a "success"
                    # entry that blocked the retry via the 300s dedup window
                    # while production still ran the old binary.
                    write_deploy_log "exited_unrecorded"
                elif [ "$GATE_REJECTED" = "yes" ]; then
                    write_deploy_log "rejected_gate"
                    # NOTE: do NOT drop sentinel on gate reject. Gate rejects
                    # are recoverable by the caller (commit/stash, fix branch,
                    # pull --ff-only). §11.4 hold-on-failure is for RUNTIME
                    # failures where production state may be degraded.
                else
                    write_deploy_log "failed"
                    drop_fail_sentinel "runtime failure (rc=$rc, action=${ACTION})"
                fi
            fi
            ;;
    esac
}

cleanup_backups() {
    [ -d "$DEPLOY_ROOT/backups" ] || return
    local BACKUPS
    mapfile -t BACKUPS < <(
        find "$DEPLOY_ROOT/backups" -maxdepth 1 -type f -printf '%T@ %f\n' 2>/dev/null \
            | sort -rn \
            | cut -d' ' -f2-
    )
    local COUNT=${#BACKUPS[@]}

    if [ "$COUNT" -le "$KEEP_BACKUPS" ]; then
        return
    fi

    info "Cleaning old backups (keeping $KEEP_BACKUPS)..."
    for ((i = KEEP_BACKUPS; i < COUNT; i++)); do
        local OLD="${BACKUPS[$i]}"
        rm -f "${DEPLOY_ROOT:?}/backups/$OLD"
        info "  Removed: $OLD"
    done
}

# ── tunnel ───────────────────────────────────────────────────────────
cmd_tunnel() {
    local ACTION="${1:-status}"
    case "$ACTION" in
        start)
            require_root
            if ! systemctl is-enabled --quiet "$TUNNEL_SERVICE" 2>/dev/null; then
                SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
                if [ -f "$SCRIPT_DIR/setup-tunnel.sh" ]; then
                    info "Tunnel not set up — running setup-tunnel.sh first..."
                    bash "$SCRIPT_DIR/setup-tunnel.sh"
                    return
                else
                    die "Tunnel service not installed. Run setup-tunnel.sh first."
                fi
            fi
            systemctl start "$TUNNEL_SERVICE"
            ok "Tunnel service started"
            # Wait for URL
            for _ in $(seq 1 20); do
                if [ -f "$TUNNEL_URL_FILE" ] && [ -s "$TUNNEL_URL_FILE" ]; then
                    ok "Tunnel URL: $(cat "$TUNNEL_URL_FILE")"
                    return 0
                fi
                sleep 1
            done
            warn "Tunnel URL not captured yet — check: journalctl -u $TUNNEL_SERVICE -f"
            ;;
        stop)
            require_root
            systemctl stop "$TUNNEL_SERVICE" 2>/dev/null || true
            ok "Tunnel service stopped"
            ;;
        restart)
            require_root
            restart_tunnel
            ;;
        url)
            if [ -f "$TUNNEL_URL_FILE" ] && [ -s "$TUNNEL_URL_FILE" ]; then
                cat "$TUNNEL_URL_FILE"
            else
                die "No tunnel URL available. Is the tunnel running?"
            fi
            ;;
        status)
            echo -e "${CYAN}=== Cloudflare Tunnel Status ===${NC}"
            if systemctl is-active --quiet "$TUNNEL_SERVICE" 2>/dev/null; then
                echo -e "Service: ${GREEN}running${NC}"
                systemctl show "$TUNNEL_SERVICE" --property=MainPID,ActiveEnterTimestamp --no-pager 2>/dev/null | sed 's/^/  /'
            elif systemctl is-enabled --quiet "$TUNNEL_SERVICE" 2>/dev/null; then
                echo -e "Service: ${YELLOW}stopped (enabled)${NC}"
            else
                echo -e "Service: ${YELLOW}not installed${NC}"
                echo "  Run: sudo $0 tunnel start  (or scripts/setup-tunnel.sh)"
                return
            fi
            if [ -f "$TUNNEL_URL_FILE" ] && [ -s "$TUNNEL_URL_FILE" ]; then
                echo -e "URL: ${GREEN}$(cat "$TUNNEL_URL_FILE")${NC}"
                # Show age of URL file
                local AGE_SECS
                AGE_SECS=$(( $(date +%s) - $(stat -c %Y "$TUNNEL_URL_FILE") ))
                local AGE_MINS=$(( AGE_SECS / 60 ))
                echo "  URL age: ${AGE_MINS}m"
            else
                echo -e "URL: ${YELLOW}(not available)${NC}"
            fi
            ;;
        *)
            die "Unknown tunnel action: $ACTION (try: start|stop|restart|status|url)"
            ;;
    esac
}

# ── help ─────────────────────────────────────────────────────────────
cmd_help() {
    echo "Aegis Runtime Deployment Tool"
    echo ""
    echo "Usage: $0 <command>"
    echo ""
    echo "Commands:"
    echo "  install   First-time server setup (requires root; gate-protected)"
    echo "  upgrade   Deploy a new release directory (supports local build output or tarball; requires root; gate-protected)"
    echo "  update    In-place update from local build output only; replaces current release files then restarts (requires root; gate-protected)"
    echo "  webapp-update"
    echo "            Frontend-only update: build and atomically sync WebApp assets without service restart (requires root; gate-protected)"
    echo "  auto"
    echo "            Choose the cheapest safe layer automatically: no-deploy, webapp-update, or update (requires root when deploy is needed)"
    echo "  deploy-plan"
    echo "            Read-only recommendation: choose no-deploy, webapp-update, or update from changes since last successful deploy"
    echo "  rollback  Revert to previous release (requires root; gate-protected)"
    echo "  verify    Dry-run: check all 4 deploy gates without deploying"
    echo "  backup    Create a safe SQLite database backup"
    echo "  status    Show deployment info and service status"
    echo "  unlock    Clear §11.4 failure sentinel after human review (requires root)"
    echo "  tunnel    Manage Cloudflare tunnel [start|stop|restart|status|url]"
    echo "  help      Show this message"
    echo ""
    echo "Deployment layers:"
    echo "  no-deploy      docs/tests/script-only; push and record checks"
    echo "  webapp-update  WebApp assets only; no Rust build, no service restart"
    echo "  update         Rust/runtime/service changes; backup, restart, smoke, health monitor"
    echo "  upgrade        New release directory or tarball activation"
    echo ""
    echo "Deploy gates (for destructive commands):"
    echo "  1. CWD must be $CANONICAL_REPO (canonical main worktree)   [exit 10]"
    echo "  2. HEAD must be on branch main                                    [exit 11]"
    echo "  3. Working tree must be clean                                     [exit 12]"
    echo "  4. Local main must equal origin/main after fetch                  [exit 13]"
    echo "  5. Migration lock (if schema changed vs last deploy, §11.5)       [exit 14]"
    echo "  See .claude/coordination/DEPLOY_RULES.md"
    echo ""
    echo "Redundant-deploy skip (Plan §11.1 Phase C):"
    echo "  update/upgrade/webapp-update are no-op'd if the same sha was deployed <5m ago."
    echo "  Override: AEGIS_RUNTIME_FORCE_DEPLOY=1 sudo $0 update"
    echo ""
    echo "Layer guards:"
    echo "  update auto-routes WebApp-only diffs to webapp-update unless AEGIS_RUNTIME_FORCE_FULL_UPDATE=1."
    echo "  webapp-update refuses runtime/core diffs unless AEGIS_RUNTIME_ALLOW_PARTIAL_WEBAPP_DEPLOY=1."
    echo ""
    echo "Host detection:"
    echo "  install/upgrade/update/auto/rollback/verify/tunnel need systemd (/run/systemd/system +"
    echo "  systemctl is-system-running != offline). Without it, install/upgrade/update/auto exec"
    echo "  scripts/first-run.sh (auth-first, nohup supervisor) and the rest exit $EXIT_NO_SYSTEMD."
    echo "  install stops BEFORE the build (exit $EXIT_AUTH_PREFLIGHT) until grok/Codex/OPENAI_API_KEY exists."
    echo ""
    echo "Environment:"
    echo "  AEGIS_RUNTIME_DEPLOY_ROOT  Override deploy path (default: /opt/aegis-runtime)"
    echo "  AEGIS_RUNTIME_SERVICE_MANAGER=auto|systemd|local   Override host detection"
    echo "  AEGIS_RUNTIME_AUTH_PROVIDER=grok|codex|api-key     Provider for the pre-start chat bind"
    echo "  AEGIS_RUNTIME_SKIP_AUTH_PREFLIGHT=1                Skip the install auth preflight"
    echo "  AEGIS_RUNTIME_DEPLOY_HEALTH_PROFILE=standard|strict|fast"
    echo "  AEGIS_RUNTIME_FORCE_FULL_UPDATE=1        Force full update even for WebApp-only diffs"
    echo "  AEGIS_RUNTIME_ALLOW_PARTIAL_WEBAPP_DEPLOY=1  Override webapp-update runtime-diff guard"
}

# ── Main dispatch (skipped when sourced) ─────────────────────────────
# Allow sourcing this script for testing: `source deploy.sh` imports
# functions without executing the dispatch block.
if [[ "${BASH_SOURCE[0]}" == "${0}" ]]; then
ACTION="${1:-help}"
trap _deploy_signal_trap TERM INT HUP
trap _deploy_exit_trap EXIT

# 0. §11.4 failure-sentinel enforcement. If a prior deploy failed and dropped
#    the sentinel, refuse destructive actions until a human clears it via
#    `unlock` or a successful `rollback`. `verify` is dry-run and allowed
#    so operators can diagnose without first unlocking.
case "$ACTION" in
    install|upgrade|update|webapp-update|auto)
        enforce_fail_sentinel
        ;;
esac

# 0b. Host capability. Every lifecycle command below drives systemd units
#     under $DEPLOY_ROOT; on a host without a working systemd, branch to the
#     portable path (or refuse clearly) BEFORE the worktree gates, which are
#     themselves part of the author-machine layout. `status`, `backup`,
#     `deploy-plan`, `unlock`, `help` stay: they read or are adaptive.
case "$ACTION" in
    install|upgrade|update|webapp-update|auto|rollback|verify|tunnel)
        detect_service_manager
        if [ "$SERVICE_MANAGER" != "systemd" ]; then
            route_without_systemd "$ACTION"
        fi
        ;;
esac

# 1. Deploy gates first — for destructive commands + verify (dry-run).
#    Gates run before mutex so wrong-worktree/wrong-branch attempts don't
#    touch the lock file at all.
#    status / backup / tunnel / deploy-plan / help / unlock are read-only (or operator-only)
#    and need no gate.
case "$ACTION" in
    install|upgrade|update|webapp-update|auto|rollback|verify)
        run_deploy_gates "$ACTION"
        ;;
esac

# 1a. Normalize deployment layer before redundant-skip, lock, migration
#     lock, and dispatch. This prevents the common slow-path mistake where
#     a WebApp-only change is sent through full Rust rebuild + service restart.
case "$ACTION" in
    auto|update|webapp-update)
        if ! normalize_deploy_action "$ACTION"; then
            if [ "${NORMALIZED_NO_DEPLOY:-}" = "yes" ]; then
                write_deploy_log "success"
            else
                DEPLOY_LOG_WRITTEN="yes"
            fi
            exit 0
        fi
        ;;
esac

# 1b. Redundant-deploy short-circuit (P2, Plan §11.1 Phase C motivation).
#     If another CC successfully deployed this exact sha within the last
#     5 min, skip. Bypass with AEGIS_RUNTIME_FORCE_DEPLOY=1.
case "$ACTION" in
    update|upgrade|webapp-update)
        if [ -z "${AEGIS_RUNTIME_FORCE_DEPLOY:-}" ]; then
            if check_redundant_deploy; then
                exit 0
            fi
        fi
        ;;
esac

# 1c. Tier-2 full-suite freshness preflight (2026-07-10). Rust-shipping
#     actions only (webapp-update ships no Rust; rollback must stay
#     instant). Runs AFTER normalize (so a webapp-only `update` that was
#     auto-routed to webapp-update doesn't pay a Rust suite) and AFTER
#     the redundant-deploy short-circuit (a skipped deploy pays nothing).
case "$ACTION" in
    install|upgrade|update)
        check_full_suite_freshness
        ;;
esac

# 2. Acquire mutex only for commands that modify production state.
#    verify is dry-run (no mutation) so skips the lock.
#    backup writes to backups/ but is serial with itself via filesystem
#    naming; no need for the deploy lock.
case "$ACTION" in
    install|upgrade|update|webapp-update|rollback)
        acquire_deploy_lock
        ;;
esac

# 2b. Migration hard-lock (Plan §11.5). For update/upgrade, if the sentinel
#     migrations file changed vs last-deployed sha, additionally acquire the
#     migration lock so two migration-carrying deploys can't interleave.
case "$ACTION" in
    update|upgrade)
        if deploy_carries_migrations; then
            info "Deploy carries schema migrations — acquiring migration lock..."
            acquire_migration_lock
        else
            info "No migration changes detected vs last deploy — skipping migration lock."
        fi
        ;;
esac

# 3. Dispatch to command handler.
case "$ACTION" in
    install)  cmd_install  ;;
    upgrade)  cmd_upgrade  ;;
    update)   cmd_update   ;;
    webapp-update) cmd_webapp_update ;;
    rollback) cmd_rollback ;;
    verify)   cmd_verify   ;;
    backup)
        cmd_backup
        # Phase E (W17): pair with heirloom DB backup.
        if [ -f "$DEPLOY_ROOT/shared/heirloom_event_log.db" ]; then
            cmd_backup_heirloom
        fi
        ;;
    status)   cmd_status   ;;
    deploy-plan) cmd_deploy_plan ;;
    unlock)
        require_root
        if [ -f "$DEPLOY_FAIL_SENTINEL" ]; then
            info "Clearing §11.4 deploy failure sentinel: $DEPLOY_FAIL_SENTINEL"
            sed 's/^/  /' "$DEPLOY_FAIL_SENTINEL" 2>/dev/null || true
            clear_fail_sentinel
            ok "Deploy lock released. Next destructive deploy will proceed through gates normally."
        else
            info "No §11.4 failure sentinel present — nothing to clear."
        fi
        ;;
    tunnel)   shift; cmd_tunnel "$@" ;;
    help|--help|-h) cmd_help ;;
    *)
        die "Unknown command: $1 (try '$0 help')"
        ;;
esac

# 4. §B.3 post-deploy health monitor. Runs only after a successful
#    upgrade/update reaches this point (cmd_* returned 0 and we haven't
#    exited non-zero). If the monitor detects a flap, it drops the §11.4
#    sentinel; we surface a non-zero exit so observers see the degradation.
case "$ACTION" in
    upgrade|update)
        if ! run_health_monitor; then
            # Monitor already wrote flap_detected log + dropped sentinel.
            # Emit a non-zero exit so callers know post-deploy verification
            # did not pass; EXIT trap will NOT double-log because
            # DEPLOY_LOG_WRITTEN was set by run_health_monitor via
            # write_deploy_log("flap_detected").
            warn "Post-deploy health monitor flagged this deploy. Review broadcasts/ and deploy-log.jsonl."
            warn "§11.4 failure sentinel dropped. Clear via 'sudo scripts/deploy.sh unlock' after review."
            exit "$EXIT_DEPLOY_FAIL"
        fi
        # §B.3 passed. Run the advisory post-deploy checklist (YAML-driven,
        # per docs/post-deploy-checklist-plan.md). Failures here do NOT block
        # the deploy — the flap-sentinel is the hard gate. The checklist
        # appends one jsonl line per check and bark-pushes on any FAIL so
        # the operator is paged.
        if [ -x "$CANONICAL_REPO/scripts/post-deploy-checklist.sh" ]; then
            info "Running post-deploy checklist (advisory)..."
            if ! AEGIS_RUNTIME_REPO_ROOT="$CANONICAL_REPO" \
                    bash "$CANONICAL_REPO/scripts/post-deploy-checklist.sh"; then
                warn "Post-deploy checklist flagged FAIL items. Deploy continues (advisory), but review .claude/coordination/deploy-log.jsonl."
            fi
        fi
        ;;
esac

# 5. Explicit success record (v11). This line — not the EXIT trap — is the
#    ONLY writer of status "success" for destructive actions: it runs after
#    the command handler AND (for update/upgrade) after the §B.3 monitor +
#    checklist, i.e. only when the deploy demonstrably completed. Paths that
#    exited earlier (gate reject, redundant skip, normalize no-op, signal
#    abort, runtime failure) have already recorded their own status.
case "$ACTION" in
    install|upgrade|update|webapp-update|rollback)
        write_deploy_log "success"
        clear_fail_sentinel
        ;;
esac
fi
