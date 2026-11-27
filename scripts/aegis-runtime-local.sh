#!/usr/bin/env bash
# aegis-runtime-local.sh — managed service commands and explicit portable operation.
#
# Runs the release binary from the checkout root under nohup with a PID file,
# so the README first-run layout (relative data files, webapp/dist, .env in
# the checkout) keeps working on Cursor Cloud, containers whose PID 1 is
# tini, macOS, or any box where `systemctl` is offline. This temporary background
# mode is not a durable installation. For Linux containers, see the explicit
# foreground-runtime.py preflight/build/run route in docs/INSTALLATION.md.
#
# Usage:
#   scripts/aegis-runtime-local.sh start     # background start + wait for /health
#   scripts/aegis-runtime-local.sh stop      # SIGTERM, then SIGKILL after 15s
#   scripts/aegis-runtime-local.sh restart
#   scripts/aegis-runtime-local.sh status    # exit 0 running, 3 not running (LSB)
#   scripts/aegis-runtime-local.sh logs [-f] # tail the log
#   scripts/aegis-runtime-local.sh run       # foreground (containers / tmux)
#   scripts/aegis-runtime-local.sh paths     # print the resolved paths
#
# Environment:
#   AEGIS_RUNTIME_ROOT       checkout root (default: parent of scripts/)
#   AEGIS_RUNTIME_BIN        binary (default: $AEGIS_RUNTIME_ROOT/target/release/aegis_runtime_core)
#   AEGIS_RUNTIME_STATE_DIR  pid + log dir (default: ${XDG_STATE_HOME:-~/.local/state}/aegis-runtime).
#                        first-run.sh also wires MEMORY_DB_PATH / IDENTITY_KEY_PATH /
#                        CREDENTIAL_DB_PATH under this directory — backup/migrate it
#                        with .env only while stopped; use the encrypted backup
#                        helper for live DBs, and retain the key separately.
#                        SoftEnclave on this path is DEV custody.
#   AXUM_PORT            /health port, read from .env when unset (default 3000)
#   AEGIS_RUNTIME_START_TIMEOUT  seconds to wait for /health (default 90)

set -euo pipefail
umask 077

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="${AEGIS_RUNTIME_ROOT:-$(cd "$SCRIPT_DIR/.." && pwd)}"
BIN="${AEGIS_RUNTIME_BIN:-${CARGO_TARGET_DIR:-$ROOT/target}/release/aegis_runtime_core}"
STATE_DIR="${AEGIS_RUNTIME_STATE_DIR:-${XDG_STATE_HOME:-$HOME/.local/state}/aegis-runtime}"
PID_FILE="$STATE_DIR/aegis-runtime.pid"
LOG_FILE="$STATE_DIR/aegis-runtime.log"
START_TIMEOUT="${AEGIS_RUNTIME_START_TIMEOUT:-90}"

# Keep the established CLI and remote-access helper compatible with managed
# installs. Do not adopt another checkout or fall back if its manifest is bad.
if command -v python3 >/dev/null 2>&1; then
    managed_rc=0
    python3 "$SCRIPT_DIR/managed-runtime.py" is-installed >/dev/null || managed_rc=$?
    if [ "$managed_rc" = 0 ]; then
        case "${1:-status}" in
            start|stop|restart|status|paths|login|remove|smoke)
                exec python3 "$SCRIPT_DIR/managed-runtime.py" "${1:-status}" ;;
            logs)
                LOG_FILE="$(python3 "$SCRIPT_DIR/managed-runtime.py" paths | sed -n 's/^log_file=//p')" ;;
            run) echo 'Managed service installed: use start/stop/restart; refusing a duplicate foreground process.' >&2; exit 1 ;;
        esac
    elif [ "$managed_rc" != 3 ]; then
        exit "$managed_rc"
    fi
fi

CYAN='\033[0;36m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; RED='\033[0;31m'; NC='\033[0m'
info() { echo -e "${CYAN}[aegis-runtime-local]${NC} $*"; }
ok()   { echo -e "${GREEN}[aegis-runtime-local]${NC} $*"; }
warn() { echo -e "${YELLOW}[aegis-runtime-local]${NC} $*"; }
die()  { echo -e "${RED}[aegis-runtime-local]${NC} $*" >&2; exit 1; }

# Value of KEY from the environment, else from $ROOT/.env, else $2. The grep
# pipeline ends in `|| true`: under `set -e -o pipefail` a missing key must be
# "use the default", not a silent exit from inside `$(…)`.
env_or_dotenv() {
    local key="$1" default="$2" value=""
    value="${!key:-}"
    if [ -z "$value" ] && [ -f "$ROOT/.env" ]; then
        value="$(grep -E "^${key}=" "$ROOT/.env" | tail -1 | cut -d= -f2- | tr -d '"' | tr -d "'" || true)"
    fi
    echo "${value:-$default}"
}

health_port() { env_or_dotenv AXUM_PORT 3000; }
console_port() { env_or_dotenv OAUTH_CALLBACK_PORT 8765; }

running_pid() {
    [ -f "$PID_FILE" ] || return 1
    local pid
    pid="$(cat "$PID_FILE" 2>/dev/null || true)"
    [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null || return 1
    echo "$pid"
}

health_ok() {
    curl -fsS -m 3 -o /dev/null "http://127.0.0.1:$(health_port)/health" 2>/dev/null
}

# The service prints its one-time login link on every boot; surface the
# newest one so the operator (or first-run.sh) can open the console.
print_login_link() {
    local link origin
    origin="$(env_or_dotenv AEGIS_PUBLIC_URL '')"
    if [ -n "$origin" ]; then
        info "Console: ${origin%/}/access (Passkey login; local login link disabled)"
        return
    fi
    link="$(grep -o 'http://127.0.0.1:[0-9]*/auth/login/local?token=[A-Za-z0-9_-]*' "$LOG_FILE" 2>/dev/null | tail -1 || true)"
    if [ -n "$link" ]; then
        ok "Console login: $link"
    else
        info "Console: http://127.0.0.1:$(console_port)/ui/ (login link: see $LOG_FILE)"
    fi
}

cmd_paths() {
    echo "root=$ROOT"
    echo "bin=$BIN"
    echo "pid_file=$PID_FILE"
    echo "log_file=$LOG_FILE"
    echo "health_url=http://127.0.0.1:$(health_port)/health"
    echo "console_url=http://127.0.0.1:$(console_port)/ui/"
}

cmd_start() {
    if pid="$(running_pid)"; then
        ok "Already running (pid $pid)"
        print_login_link
        return 0
    fi
    [ -x "$BIN" ] || die "Binary not found: $BIN — run: cargo build --release -p aegis-runtime-core (or scripts/first-run.sh)"
    [ -n "${AEGIS_RUNTIME_DB_KEY_HEX:-}" ] || die "Background start requires AEGIS_RUNTIME_DB_KEY_HEX from an external secret manager (never .env); use 'run' for a foreground passphrase prompt."
    [ -f "$ROOT/webapp/dist/index.html" ] || warn "webapp/dist/index.html missing — /ui/ will be empty until you run: cd webapp && npm ci && npm run build"
    mkdir -p "$STATE_DIR"
    rm -f "$PID_FILE"

    info "Starting $BIN from $ROOT (log: $LOG_FILE)"
    warn "Portable background process: agent exit, logout or WSL idle shutdown may stop it. Use first-run without --portable for managed installation."
    {
        echo "───── $(date -u +%Y-%m-%dT%H:%M:%SZ) aegis-runtime-local start ─────"
    } >> "$LOG_FILE"
    # `$!` must be the binary's own pid: keep the async command a *simple*
    # command (bash execs it directly). `cd … && nohup … &` would background
    # the whole list in a wrapper subshell, record that wrapper's pid, and
    # leave the wrapper holding the caller's stdout open. No setsid either:
    # it forks under job control, which also records the wrong pid.
    (
        cd "$ROOT" || exit 1
        nohup "$BIN" >> "$LOG_FILE" 2>&1 < /dev/null &
        echo $! > "$PID_FILE"
    )

    local waited=0 pid
    pid="$(cat "$PID_FILE")"
    while [ "$waited" -lt "$START_TIMEOUT" ]; do
        if ! kill -0 "$pid" 2>/dev/null; then
            echo "" >&2
            tail -n 40 "$LOG_FILE" >&2
            rm -f "$PID_FILE"
            die "Process exited during startup (see log above). A '[Reasoner]' / OPENAI_API_KEY error means the auth step was skipped — run: scripts/first-run.sh"
        fi
        if health_ok; then
            ok "Running (pid $pid) — /health on port $(health_port) is up after ${waited}s"
            print_login_link
            return 0
        fi
        sleep 1
        waited=$((waited + 1))
    done
    warn "Process is alive (pid $pid) but /health did not answer within ${START_TIMEOUT}s; check $LOG_FILE"
    return 1
}

cmd_stop() {
    local pid
    if ! pid="$(running_pid)"; then
        info "Not running"
        rm -f "$PID_FILE"
        return 0
    fi
    info "Stopping pid $pid…"
    kill -TERM "$pid" 2>/dev/null || true
    local waited=0
    while kill -0 "$pid" 2>/dev/null && [ "$waited" -lt 15 ]; do
        sleep 1
        waited=$((waited + 1))
    done
    if kill -0 "$pid" 2>/dev/null; then
        warn "Still alive after 15s — sending SIGKILL"
        kill -KILL "$pid" 2>/dev/null || true
    fi
    rm -f "$PID_FILE"
    ok "Stopped"
}

cmd_status() {
    local pid
    if pid="$(running_pid)"; then
        if health_ok; then
            ok "Service: running (pid $pid, /health ok on port $(health_port))"
        else
            warn "Service: running (pid $pid) but /health on port $(health_port) is not answering"
        fi
        print_login_link
        return 0
    fi
    echo "Service: not running (supervisor: aegis-runtime-local, none active)"
    echo "  start with: $SCRIPT_DIR/aegis-runtime-local.sh start"
    return 3
}

cmd_logs() {
    [ -f "$LOG_FILE" ] || die "No log yet at $LOG_FILE"
    if [ "${1:-}" = "-f" ]; then
        exec tail -n 100 -f "$LOG_FILE"
    fi
    tail -n "${1:-100}" "$LOG_FILE"
}

cmd_run() {
    [ -x "$BIN" ] || die "Binary not found: $BIN"
    cd "$ROOT"
    exec "$BIN"
}

case "${1:-help}" in
    start)   cmd_start ;;
    stop)    cmd_stop ;;
    restart) cmd_stop; cmd_start ;;
    status)  cmd_status ;;
    logs)    shift; cmd_logs "$@" ;;
    run)     cmd_run ;;
    paths)   cmd_paths ;;
    login)   print_login_link ;;
    help|--help|-h)
        sed -n '2,25p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
        ;;
    *) die "Unknown command: $1 (try: start|stop|restart|status|logs|run|paths)" ;;
esac
