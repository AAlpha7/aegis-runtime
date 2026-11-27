#!/usr/bin/env bash
# first-run.sh — auth-first Aegis Runtime setup for a fresh checkout.
#
# Order is the point: choose the owner-chat provider and finish its sign-in
# BEFORE building or booting, so the first boot never answers chat with
# 503 "Assistant model is unavailable".
#
#   1. choose provider       grok login (preferred) → codex login → API key (last)
#   2. CLI sign-in           grok: install CLI if missing + `grok login --device-auth`
#                            codex: `codex login --device-auth`
#                            api-key: OPENAI_API_KEY (+ OPENAI_API_BASE) into .env
#   3. prepare .env          copy .env.example, ensure ADMIN_CHAT_ID (workbench needs one)
#   4. build                 webapp (npm) + cargo build --release
#   5. pre-boot chat bind    aegis_runtime_core setup-auth --provider <p>
#                            grok = the SECOND xAI device code (in-app credential,
#                            separate from the grok CLI session) + "Use for chat"
#   6. start                 persistent systemd user service (WSL also needs Windows task)
#   7. prove it              /health → local login → one owner chat round-trip (HTTP 200)
#
# Usage:
#   scripts/first-run.sh [--provider grok|codex|api-key] [--skip-build]
#                        [--auth-only] [--no-start] [--skip-chat-probe] [--yes]
#                        [--access-mode local|private|public] [--accept-public-risk]
#                        [--portable] (explicit temporary background operation)
#
# Environment:
#   AEGIS_RUNTIME_AUTH_PROVIDER   same as --provider
#   AEGIS_RUNTIME_ROOT / AEGIS_RUNTIME_BIN / AEGIS_RUNTIME_STATE_DIR  see aegis-runtime-local.sh
#   AEGIS_RUNTIME_SKIP_LOCAL_EMBEDDING=1  skip Ollama nomic-embed-text install
#   AEGIS_RUNTIME_GROK_LOGIN_ATTEMPTS     grok login transport retries (default 5)
#   CARGO_BUILD_JOBS          forwarded to cargo (small VMs: 1)
#   AEGIS_RUNTIME_DB_KEY_FILE existing private raw key file, outside data/checkout;
#                             generated for new managed installations
#   AEGIS_RUNTIME_DB_KEY_HEX external injection for portable/foreground operation
#
# Re-running is safe: existing sessions, builds, and bindings are reused.
# deploy.sh execs this script when it finds no working systemd.
# Heirloom on this path uses SoftEnclave::new_dev — DEV custody only
# (keys in process memory; not a production hardware enclave).

set -euo pipefail
umask 077

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="${AEGIS_RUNTIME_ROOT:-$(cd "$SCRIPT_DIR/.." && pwd)}"
BIN="${AEGIS_RUNTIME_BIN:-${CARGO_TARGET_DIR:-$ROOT/target}/release/aegis_runtime_core}"
if [ -z "${AEGIS_RUNTIME_BIN:-}" ] && [ -x "$ROOT/aegis_runtime_core" ]; then BIN="$ROOT/aegis_runtime_core"; fi
export AEGIS_RUNTIME_BIN="$BIN"
LOCAL_SUPERVISOR="$SCRIPT_DIR/aegis-runtime-local.sh"
GROK_INSTALL_ONE_LINER='curl -fsSL https://x.ai/cli/install.sh | bash'

export PATH="${CARGO_HOME:-$HOME/.cargo}/bin:$HOME/.grok/bin:$PATH"

CYAN='\033[0;36m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; RED='\033[0;31m'; BOLD='\033[1m'; NC='\033[0m'
step() { echo -e "\n${BOLD}${CYAN}[first-run $1]${NC} ${BOLD}$2${NC}"; }
info() { echo -e "${CYAN}[first-run]${NC} $*"; }
ok()   { echo -e "${GREEN}[first-run]${NC} $*"; }
warn() { echo -e "${YELLOW}[first-run]${NC} $*"; }
die()  { echo -e "${RED}[first-run]${NC} $*" >&2; exit 1; }

PROVIDER="${AEGIS_RUNTIME_AUTH_PROVIDER:-}"
SKIP_BUILD=no
AUTH_ONLY=no
NO_START=no
SKIP_CHAT_PROBE=no
ASSUME_YES=no
ACCESS_MODE=""
ACCESS_CONSENT=()
PORTABLE=no
ORIGINAL_ARGS=("$@")

while [ $# -gt 0 ]; do
    case "$1" in
        --provider) PROVIDER="${2:-}"; shift ;;
        --provider=*) PROVIDER="${1#--provider=}" ;;
        --skip-build) SKIP_BUILD=yes ;;
        --auth-only) AUTH_ONLY=yes ;;
        --no-start) NO_START=yes ;;
        --skip-chat-probe) SKIP_CHAT_PROBE=yes ;;
        --yes|-y) ASSUME_YES=yes ;;
        --portable) PORTABLE=yes ;;
        --access-mode) ACCESS_MODE="${2:-}"; shift ;;
        --access-mode=*) ACCESS_MODE="${1#--access-mode=}" ;;
        --accept-public-risk) ACCESS_CONSENT=(--accept-public-risk) ;;
        -h|--help) sed -n '2,40p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) die "Unknown argument: $1 (see --help)" ;;
    esac
    shift
done

# The normal installation has a real service lifetime. Foreground/auth-only
# workflows remain available; portable background mode is an explicit choice.
if [ "$AUTH_ONLY" != yes ] && [ "$NO_START" != yes ] && [ "$PORTABLE" != yes ] \
   && [ "${AEGIS_RUNTIME_MANAGED_SETUP:-0}" != 1 ]; then
    command -v python3 >/dev/null || die "Install Python 3, then rerun setup. See docs/INSTALLATION.md."
    exec python3 "$SCRIPT_DIR/managed-runtime.py" setup "${ORIGINAL_ARGS[@]}"
fi
if [ "$PORTABLE" = yes ]; then
    warn "Portable mode selected: closing the agent/terminal or WSL may stop this service. No autostart or durable deployment is promised."
fi

# Choose access separately from provider authentication. A general --yes never
# implies consent to a public endpoint. Unattended installs keep local access.
if [ "$AUTH_ONLY" != yes ]; then
    if [ -z "$ACCESS_MODE" ]; then
        ACCESS_MODE="$(sed -n 's/^AEGIS_ACCESS_MODE=//p' "$ROOT/.env" 2>/dev/null | tail -1 | tr -d "\"'" || true)"
        if [ -z "$ACCESS_MODE" ] && [ -t 0 ] && [ "$ASSUME_YES" != yes ]; then
            echo "Access: 1) Local only  2) Private Tailscale VPN (recommended remote)  3) Public browser access (higher exposure)"
            read -r -p "Choose [1/2/3, default 1]: " access_choice
            case "$access_choice" in
                ""|1) ACCESS_MODE=local ;;
                2) ACCESS_MODE=private ;;
                3) ACCESS_MODE=public ;;
                *) die "Invalid access mode" ;;
            esac
        fi
        ACCESS_MODE="${ACCESS_MODE:-local}"
    fi
    case "$ACCESS_MODE" in local|private|public) ;; *) die "Invalid --access-mode" ;; esac
    if [ "$ACCESS_MODE" != local ]; then
        command -v python3 >/dev/null || die "Remote setup requires Python 3."
        command -v tailscale >/dev/null || die "Install and sign into Tailscale in this Linux/WSL environment first: https://tailscale.com/download"
        python3 "$SCRIPT_DIR/remote-access.py" --check-choice --mode "$ACCESS_MODE" "${ACCESS_CONSENT[@]}"
        if [ "$ACCESS_MODE" = public ]; then ACCESS_CONSENT=(--accept-public-risk); fi
    fi
fi

finish_access() {
    if [ "$ACCESS_MODE" != local ] || [ -f "$ROOT/.aegis-access.json" ] || [ -n "$(dotenv_get AEGIS_PUBLIC_URL)" ]; then
        python3 "$SCRIPT_DIR/remote-access.py" --root "$ROOT" --mode "$ACCESS_MODE" "${ACCESS_CONSENT[@]}"
    else
        ok "Local-only access. Configure phone access later with: python3 scripts/remote-access.py"
    fi
}

if [ -n "${AEGIS_RUNTIME_DB_KEY_HEX:-}" ]; then
    [[ "$AEGIS_RUNTIME_DB_KEY_HEX" =~ ^[[:xdigit:]]{64}$ ]] || die "AEGIS_RUNTIME_DB_KEY_HEX must be exactly 64 hex characters."
elif [ "$NO_START" != yes ] && [ "$AUTH_ONLY" != yes ]; then
    die "Background start needs AEGIS_RUNTIME_DB_KEY_HEX injected from outside .env. See README Database encryption. For a foreground passphrase setup, use --no-start."
fi
if [ "$SKIP_BUILD" != yes ] && [ "$AUTH_ONLY" != yes ] && [ -d "$ROOT/webapp" ]; then
    NODE_MAJOR="$(node -p 'process.versions.node.split(".")[0]' 2>/dev/null || true)"
    [[ "$NODE_MAJOR" =~ ^[0-9]+$ ]] && [ "$NODE_MAJOR" -ge 22 ] && command -v npm >/dev/null 2>&1 \
        || die "WebApp build needs Node.js 22+ and npm; install Node 22 before beginning provider sign-in."
fi

is_tty() { [ -t 0 ] && [ -t 1 ]; }

# ── helpers ────────────────────────────────────────────────────────────
# Always exits 0: an absent key is an empty string, not a failure (this runs
# inside `$(…)` assignments under `set -e -o pipefail`).
dotenv_get() {
    local key="$1"
    [ -f "$ROOT/.env" ] || return 0
    grep -E "^${key}=" "$ROOT/.env" | tail -1 | cut -d= -f2- | tr -d '"' | tr -d "'" || true
}

# Set KEY=VALUE in .env (replace an existing active line, else append).
dotenv_set() {
    local key="$1" value="$2" tmp
    tmp="$(mktemp "$ROOT/.env.stage.XXXXXX")"
    if grep -qE "^${key}=" "$ROOT/.env"; then
        awk -v k="$key" -v v="$value" 'BEGIN{done=0} $0 ~ "^"k"=" && !done {print k"="v; done=1; next} {print}' "$ROOT/.env" > "$tmp"
    else
        cat "$ROOT/.env" > "$tmp"
        printf '%s=%s\n' "$key" "$value" >> "$tmp"
    fi
    mv "$tmp" "$ROOT/.env"
}

api_key_usable() {
    local key="$1"
    [ -n "${key// /}" ] || return 1
    case "$key" in
        sk-placeholder|sk-or-v1-xxxxxxxx) return 1 ;;
    esac
    return 0
}

# Jev consent is separate from general installation confirmation. Never print
# credentials or reuse a generic OpenAI key for it. Probe only after opt-in.
offer_jev_for_api_key() {
    local configured key source answer
    configured="${AEGIS_JEV_ENABLED-$(dotenv_get AEGIS_JEV_ENABLED)}"
    configured="${configured#"${configured%%[![:space:]]*}"}"
    configured="${configured%"${configured##*[![:space:]]}"}"
    if [ -n "$configured" ]; then
        case "${configured,,}" in
            1|true|yes) info "Jev opt-in already configured; a valid separate OpenRouter credential is still required." ;;
            *) info "Jev remains off (existing choice preserved)." ;;
        esac
        return 0
    fi
    info "Jev is optional and off by default. It sends proposed outbound content/action context to OpenRouter Decisions for safety assessment and incurs separate API charges."
    info "Grok/ChatGPT subscriptions and generic OpenAI keys do not cover Jev. Local checks and required approvals remain when it is off."
    if ! is_tty || [ "$ASSUME_YES" = yes ]; then
        info "Jev stays off: unattended setup and --yes are not Jev consent. To opt in later, use Settings → Advanced → AI and spending; verification is required before saving."
        return 0
    fi
    answer=""
    read -rp "Enable optional Jev safety assessment? [y/N]: " answer || true
    case "${answer,,}" in
        y|yes)
            for source in AEGIS_JEV_API_KEY OPENROUTER_API_KEY OPENAI_API_KEY; do
                key="${!source-$(dotenv_get "$source")}"
                # Match the runtime resolver: trim whitespace, reject placeholders.
                key="${key#"${key%%[![:space:]]*}"}"
                key="${key%"${key##*[![:space:]]}"}"
                [ "${#key}" -gt 20 ] || continue
                [[ "$key" != *xxxxxxxx* ]] || continue
                if [ "$source" = OPENAI_API_KEY ] && [[ "$key" != sk-or-* ]]; then continue; fi
                info "Validating Jev with a fixed test action (separate API charge); configuration is saved only after success."
                if AEGIS_JEV_API_KEY="$key" "$BIN" setup-auth verify-jev; then
                    dotenv_set AEGIS_JEV_ENABLED 1
                    export AEGIS_JEV_ENABLED=1
                    ok "Jev enabled by your choice after live validation; its API calls use separate OpenRouter billing."
                else
                    warn "Jev validation failed; configuration was not saved. Existing settings are unchanged. You can retry in Settings → Advanced → AI and spending."
                fi
                return 0
            done
            warn "Jev remains off: no usable OpenRouter credential found. Inject AEGIS_JEV_API_KEY or OPENROUTER_API_KEY externally and rerun setup to opt in. Your chat key is unchanged."
            ;;
        *)
            dotenv_set AEGIS_JEV_ENABLED 0
            export AEGIS_JEV_ENABLED=0
            info "Jev stays off. To change this later, use Settings → Advanced → AI and spending; verification is required before saving."
            ;;
    esac
}

grok_bin() {
    if command -v grok >/dev/null 2>&1; then command -v grok; return 0; fi
    [ -x "$HOME/.grok/bin/grok" ] && { echo "$HOME/.grok/bin/grok"; return 0; }
    return 1
}
grok_session_present() { [ -s "$HOME/.grok/auth.json" ]; }

codex_logged_in() {
    command -v codex >/dev/null 2>&1 || return 1
    codex login status 2>&1 | grep -q "Logged in using ChatGPT"
}

json_field() {
    # json_field '<json>' key.path
    local json="$1" path="$2"
    if command -v python3 >/dev/null 2>&1; then
        python3 -c '
import json, sys
obj = json.loads(sys.argv[1])
for part in sys.argv[2].split("."):
    obj = obj.get(part) if isinstance(obj, dict) else None
    if obj is None:
        break
print("" if obj is None else obj)
' "$json" "$path" 2>/dev/null || true
    elif command -v jq >/dev/null 2>&1; then
        printf '%s' "$json" | jq -r ".${path} // empty" 2>/dev/null || true
    fi
}

confirm() {
    [ "$ASSUME_YES" = yes ] && return 0
    is_tty || return 0
    local answer
    read -rp "$1 [Y/n] " answer
    [ -z "$answer" ] || [[ "$answer" =~ ^[Yy] ]]
}

# True when grok login failed for a flaky transport reason that is worth
# retrying (keep-alive / timeout / http2 / connection reset). Auth expiry
# and user cancel are NOT retried — those need a fresh human click.
grok_login_is_transport_flake() {
    local log="$1"
    grep -qiE 'keep-alive timed out|operation timed out|http2 error|error sending request|connection reset|connection refused|timed out|SendRequest' "$log" 2>/dev/null
}

# Run `grok login --device-auth` with short retries on transport flakes so a
# successful human click is not dropped by an http2 keep-alive timeout.
# Prints wait / expiry guidance (device codes typically last ~15–30 min).
run_grok_login_with_retry() {
    local attempts="${AEGIS_RUNTIME_GROK_LOGIN_ATTEMPTS:-5}" i=1 log rc
    info "Session 1 of 2 — grok CLI device login."
    info "Open the URL + enter the code when printed. Codes usually expire in ~15–30 min — authorize promptly."
    info "Transport flakes (keep-alive / http2 / timeout) are retried up to ${attempts}x; expiry needs a new code."
    while [ "$i" -le "$attempts" ]; do
        log="$(mktemp)"
        info "grok login attempt $i/$attempts ($(date -u +%H:%M:%SZ))"
        set +e
        "$(grok_bin)" login --device-auth 2>&1 | tee "$log"
        rc=${PIPESTATUS[0]}
        set -e
        if [ "$rc" -eq 0 ] && grok_session_present; then
            rm -f "$log"
            ok "grok CLI session created (~/.grok/auth.json)"
            return 0
        fi
        if grok_session_present; then
            rm -f "$log"
            ok "grok CLI session present after attempt $i"
            return 0
        fi
        if grok_login_is_transport_flake "$log" && [ "$i" -lt "$attempts" ]; then
            warn "transport flake on attempt $i (see keep-alive/timeout/http2 above) — retrying in 3s so a completed click is not lost"
            rm -f "$log"
            sleep 3
            i=$((i + 1))
            continue
        fi
        # Non-flake failure or final attempt: surface the log and stop.
        warn "last grok login output (tail): $(tail -3 "$log" | tr '\n' ' ')"
        rm -f "$log"
        die "grok login failed (attempt $i/$attempts). Re-run after fixing network, or authorize a fresh code before it expires (~15–30 min)."
    done
    die "grok login failed after $attempts attempts"
}

# ── 0. host ───────────────────────────────────────────────────────────
step 0/7 "Host"
info "checkout: $ROOT"
if [ -d /run/systemd/system ] && command -v systemctl >/dev/null 2>&1 \
   && ! systemctl is-system-running 2>/dev/null | grep -qE '^(offline|unknown)$'; then
    info "systemd detected; normal setup uses a persistent user service. --portable explicitly selects temporary operation."
else
    info "systemd not confirmed in this process; a sandbox denial does not establish host capability."
fi
for tool in cargo node npm curl; do
    command -v "$tool" >/dev/null 2>&1 || warn "$tool not on PATH — see README §1 System dependencies (scripts/bootstrap-system-deps.sh on Ubuntu/Debian)"
done

# ── 1. choose provider ────────────────────────────────────────────────
step 1/7 "Choose the owner-chat provider (login first, API key last)"
if [ -z "$PROVIDER" ]; then
    if is_tty; then
        echo "  1) Grok    — grok CLI login + in-app xAI device code (two sessions, both before boot)  [recommended]"
        echo "  2) Codex   — codex CLI login (ChatGPT subscription)"
        echo "  3) API key — OPENAI_API_KEY / OPENAI_API_BASE (paid; last resort)"
        read -rp "Provider [1/2/3, default 1]: " choice
        case "${choice:-1}" in
            1|grok) PROVIDER=grok ;;
            2|codex) PROVIDER=codex ;;
            3|api-key|key) PROVIDER=api-key ;;
            *) die "Unknown choice: $choice" ;;
        esac
    else
        # Non-interactive: reuse whatever session already exists, in the preferred order.
        if grok_bin >/dev/null && grok_session_present; then PROVIDER=grok
        elif codex_logged_in; then PROVIDER=codex
        elif api_key_usable "${OPENAI_API_KEY:-$(dotenv_get OPENAI_API_KEY)}"; then PROVIDER=api-key
        else
            die "No provider chosen and no existing session found. Re-run with --provider grok|codex|api-key (or AEGIS_RUNTIME_AUTH_PROVIDER). Grok: $GROK_INSTALL_ONE_LINER && grok login --device-auth"
        fi
        info "non-interactive: detected existing session → $PROVIDER"
    fi
fi
case "$PROVIDER" in
    grok|codex|api-key) ok "provider: $PROVIDER" ;;
    *) die "--provider must be grok, codex, or api-key (got: $PROVIDER)" ;;
esac

# ── 2. CLI sign-in (before any build) ─────────────────────────────────
step 2/7 "Complete the provider sign-in"
case "$PROVIDER" in
    grok)
        if ! grok_bin >/dev/null; then
            info "grok CLI not found. Installing: $GROK_INSTALL_ONE_LINER"
            confirm "Run the xAI installer now?" || die "Install the grok CLI, then re-run."
            bash -c "$GROK_INSTALL_ONE_LINER"
            export PATH="$HOME/.grok/bin:$PATH"
            grok_bin >/dev/null || die "grok still not found after install (expected ~/.grok/bin/grok)"
        fi
        ok "grok CLI: $(grok_bin)"
        if grok_session_present; then
            ok "grok CLI session present (~/.grok/auth.json)"
        else
            run_grok_login_with_retry
        fi
        if [ "${AEGIS_RUNTIME_SKIP_GROK_SMOKE:-0}" != 1 ]; then
            info "smoke: grok -p (30s timeout)"
            if out="$(timeout 30 "$(grok_bin)" -p 'reply with exactly: grok-login-ok' 2>&1)" && echo "$out" | grep -q grok-login-ok; then
                ok "grok -p answered"
            else
                warn "grok -p smoke did not return grok-login-ok (continuing; output: $(echo "$out" | tail -1))"
            fi
        fi
        ;;
    codex)
        command -v codex >/dev/null 2>&1 || die "codex CLI not on PATH — install it (npm install -g @openai/codex), then re-run."
        if codex_logged_in; then
            ok "codex CLI logged in (ChatGPT)"
        else
            info "codex CLI login: follow the URL + code printed by codex. Codes expire — authorize promptly."
            codex login --device-auth || die "codex login failed"
            codex_logged_in || die "codex login finished but 'codex login status' does not report a ChatGPT login"
            ok "codex CLI session created"
        fi
        ;;
    api-key)
        KEY="${OPENAI_API_KEY:-$(dotenv_get OPENAI_API_KEY)}"
        if ! api_key_usable "$KEY"; then
            is_tty || die "OPENAI_API_KEY is unset or the template placeholder. Export it (or set it in .env) and re-run."
            read -rsp "OPENAI_API_KEY (input hidden): " KEY; echo
            api_key_usable "$KEY" || die "That key is empty or a placeholder."
        fi
        export OPENAI_API_KEY="$KEY"
        ok "OPENAI_API_KEY accepted (not printed)"
        ;;
esac

# ── 3. .env ───────────────────────────────────────────────────────────
step 3/7 "Prepare .env"
cd "$ROOT"
ENV_BACKUP="$(mktemp)"
ENV_EXISTED=no
CONFIG_VALIDATED=no
if [ -f .env ]; then cp .env "$ENV_BACKUP"; ENV_EXISTED=yes; fi
cleanup_staged_env() {
    if [ "$CONFIG_VALIDATED" != yes ]; then
        if [ "$ENV_EXISTED" = yes ]; then cp "$ENV_BACKUP" "$ROOT/.env"; else rm -f "$ROOT/.env"; fi
        warn "Setup did not validate the provider; the previous .env configuration was restored."
    fi
    rm -f "$ENV_BACKUP"
}
trap cleanup_staged_env EXIT
if [ ! -f .env ]; then
    cp .env.example .env
    ok "created .env from .env.example"
else
    ok ".env exists (not overwritten)"
fi
if [ -z "$(dotenv_get ADMIN_CHAT_ID)" ]; then
    dotenv_set ADMIN_CHAT_ID 1
    ok "ADMIN_CHAT_ID=1 added (workbench APIs need a chat context even without Telegram)"
fi
if [ "$PROVIDER" = api-key ]; then
    dotenv_set OPENAI_API_KEY "$OPENAI_API_KEY"
    if [ -n "${OPENAI_API_BASE:-}" ]; then
        dotenv_set OPENAI_API_BASE "$OPENAI_API_BASE"
    fi
    if [ -n "${LLM_MODEL:-}" ]; then
        dotenv_set LLM_MODEL "$LLM_MODEL"
    elif [ -z "$(dotenv_get LLM_MODEL)" ]; then
        case "$(dotenv_get OPENAI_API_BASE)" in
            https://openrouter.ai/api/v1|https://openrouter.ai/api/v1/)
                dotenv_set LLM_MODEL "deepseek/deepseek-v4.1-flash"
                info "Recommended API model: DeepSeek V4.1 Flash. Setup will verify it before accepting this configuration; API calls are billed separately."
                ;;
        esac
    fi
    ok "OPENAI_API_KEY written to .env (OPENAI_API_BASE: $(dotenv_get OPENAI_API_BASE))"
fi

# Grok-first path: empty LLM_MODEL so logs/routing use the provider default
# (xai_subscription_model / grok-4.x), not the OpenRouter bootstrap slug from
# .env.example (configured_model vs routed model noise).
if [ "$PROVIDER" = grok ] || [ "$PROVIDER" = codex ]; then
    case "$(dotenv_get LLM_MODEL)" in
        ""|google/gemini*|openai/*|anthropic/*|deepseek/*)
            dotenv_set LLM_MODEL ""
            ok "LLM_MODEL cleared — using $PROVIDER provider default (not an OpenRouter bootstrap slug)"
            ;;
    esac
fi

# Portable state dir: prefer AEGIS_RUNTIME_STATE_DIR for DBs/keys (not checkout root).
# Pid/log already live there via aegis-runtime-local.sh. Override any path explicitly.
STATE_DIR="${AEGIS_RUNTIME_STATE_DIR:-${XDG_STATE_HOME:-$HOME/.local/state}/aegis-runtime}"
mkdir -p "$STATE_DIR"
export AEGIS_RUNTIME_STATE_DIR="$STATE_DIR"
wire_state_path() {
    local key="$1" filename="$2" current
    current="$(dotenv_get "$key")"
    case "$current" in
        ""|./"$filename"|././"$filename")
            dotenv_set "$key" "$STATE_DIR/$filename"
            ok "$key=$STATE_DIR/$filename"
            ;;
        *)
            info "$key already set ($current) — leaving in place"
            ;;
    esac
}
wire_state_path MEMORY_DB_PATH aegis_runtime_memory.db
wire_state_path IDENTITY_KEY_PATH aegis_runtime_identity.key
wire_state_path CREDENTIAL_DB_PATH aegis_runtime_credentials.db
info "state directory: $STATE_DIR (use the encrypted DB backup helper for live data; keep the key separately)"

# Local embedding dependency when no usable remote embedding key (Grok/Codex
# installs). Fail loudly with install steps — do not leave a silent 401 path.
embedding_key_usable() {
    local key="$1"
    [ -n "${key// /}" ] || return 1
    case "$key" in
        sk-placeholder|sk-or-v1-xxxxxxxx|sk-xxxxxxxx) return 1 ;;
    esac
    return 0
}
NEED_LOCAL_EMBED=no
if ! embedding_key_usable "${EMBEDDING_API_KEY:-$(dotenv_get EMBEDDING_API_KEY)}" \
   && ! embedding_key_usable "${OPENAI_API_KEY:-$(dotenv_get OPENAI_API_KEY)}"; then
    NEED_LOCAL_EMBED=yes
fi
# Explicit remote embed base without a key still needs the local dep.
if [ "$NEED_LOCAL_EMBED" = yes ] || { [ "$PROVIDER" != api-key ] && [ -z "$(dotenv_get EMBEDDING_API_BASE)" ]; }; then
    if [ "${AEGIS_RUNTIME_SKIP_LOCAL_EMBEDDING:-0}" = 1 ]; then
        warn "skipping local embedding (AEGIS_RUNTIME_SKIP_LOCAL_EMBEDDING=1) — L2 semantic memory will be unavailable until you run scripts/ensure-local-embedding.sh"
    else
        step 3b/7 "Local embedding dependency (Ollama nomic-embed-text — L2 without OPENAI_API_KEY)"
        AEGIS_RUNTIME_ASSUME_YES="$ASSUME_YES" "$SCRIPT_DIR/ensure-local-embedding.sh" \
            || die "local embedding is required for L2 on a $PROVIDER install without OPENAI_API_KEY. Fix the errors above, then re-run."
    fi
fi

if [ "${AEGIS_RUNTIME_HEIRLOOM_BRIDGE:-$(dotenv_get AEGIS_RUNTIME_HEIRLOOM_BRIDGE)}" = on ]; then
    warn "Experimental Heirloom is enabled with software custody; this is not a hardware enclave."
fi

if [ "$AUTH_ONLY" = yes ] && [ "$SKIP_BUILD" = no ] && [ ! -x "$BIN" ]; then
    ok "--auth-only: CLI sign-in done; the in-app bind needs the built binary — run without --auth-only later."
    exit 2
fi

# ── 4. build ──────────────────────────────────────────────────────────
step 4/7 "Build"
if [ "$SKIP_BUILD" = yes ]; then
    [ -x "$BIN" ] || die "--skip-build but $BIN does not exist"
    info "skipping build (--skip-build)"
else
    if [ -d webapp ]; then
        if [ ! -d webapp/node_modules ]; then
            info "webapp: npm ci"
            (cd webapp && npm ci --no-audit --no-fund)
        fi
        info "webapp: npm run build"
        (cd webapp && npm run build -- --configLoader runner)
        ok "webapp/dist ready"
    fi
    info "cargo build --release -p aegis-runtime-core -p aegis-executor (first build takes ~10+ min)"
    CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}" cargo build --release -p aegis-runtime-core -p aegis-executor
    ok "binary: $BIN"
fi

if [ "$PROVIDER" = api-key ]; then offer_jev_for_api_key; fi

# ── 5. pre-boot owner-chat bind ───────────────────────────────────────
step 5/7 "Bind owner chat BEFORE boot (aegis_runtime_core setup-auth --provider $PROVIDER)"
if [ "$PROVIDER" = grok ]; then
    info "Session 2 of 2 — in-app xAI device code. This is a different session from the grok CLI login."
fi
set +e
"$BIN" setup-auth --provider "$PROVIDER"
SETUP_RC=$?
set -e
case "$SETUP_RC" in
    0) CONFIG_VALIDATED=yes; cleanup_staged_env; trap - EXIT; ok "owner chat validated and bound to $PROVIDER" ;;
    2) die "setup-auth reports NOT READY (see lines above). Fix that first; nothing was started." ;;
    *) die "setup-auth failed (exit $SETUP_RC)" ;;
esac
if [ "$AUTH_ONLY" = yes ]; then
    ok "--auth-only: done. Start later with: $LOCAL_SUPERVISOR start"
    exit 0
fi

# ── 6. start ──────────────────────────────────────────────────────────
step 6/7 "Start"
if [ "$NO_START" = yes ]; then
    ok "--no-start: not starting. For foreground operation: $LOCAL_SUPERVISOR run"
    info "Chosen access: $ACCESS_MODE. After starting, configure with: python3 scripts/remote-access.py --mode $ACCESS_MODE"
    exit 0
fi
if [ "${AEGIS_RUNTIME_MANAGED_SETUP:-0}" = 1 ]; then
    python3 "$SCRIPT_DIR/managed-runtime.py" install
fi
if "$LOCAL_SUPERVISOR" status >/dev/null 2>&1; then
    info "service already running → restart to pick up the new binding"
    "$LOCAL_SUPERVISOR" restart
else
    "$LOCAL_SUPERVISOR" start
fi

# ── 7. prove one owner chat round-trip ────────────────────────────────
step 7/7 "Owner chat round-trip (setup is not done on /health alone)"
if [ "$SKIP_CHAT_PROBE" = yes ]; then
    warn "--skip-chat-probe: not verified. Open the console login link above and send one message."
    finish_access
    exit 2
fi
if ! command -v python3 >/dev/null 2>&1 && ! command -v jq >/dev/null 2>&1; then
    warn "neither python3 nor jq available — cannot parse JSON; open the console and send one message instead."
    finish_access
    exit 2
fi

if [ -n "$(dotenv_get AEGIS_PUBLIC_URL)" ]; then
    warn "Existing remote deployment: local login is disabled; verify chat in the authenticated browser."
    finish_access
    exit 2
fi

CONSOLE_PORT="${OAUTH_CALLBACK_PORT:-$(dotenv_get OAUTH_CALLBACK_PORT)}"; CONSOLE_PORT="${CONSOLE_PORT:-8765}"
BASE="http://127.0.0.1:$CONSOLE_PORT"
LOGIN_TOKEN="${AEGIS_LOCAL_LOGIN_TOKEN:-$(dotenv_get AEGIS_LOCAL_LOGIN_TOKEN)}"
if [ -z "$LOGIN_TOKEN" ] || [ "$LOGIN_TOKEN" = off ]; then
    LOG_FILE="$("$LOCAL_SUPERVISOR" paths | sed -n 's/^log_file=//p')"
    LOGIN_TOKEN="$(grep -o 'auth/login/local?token=[A-Za-z0-9_-]*' "$LOG_FILE" 2>/dev/null | tail -1 | cut -d= -f2 || true)"
fi
[ -n "$LOGIN_TOKEN" ] || die "no local login token found (set AEGIS_LOCAL_LOGIN_TOKEN in .env or read the link from the log)"

COOKIE_JAR="$(mktemp)"
trap 'rm -f "$COOKIE_JAR"' EXIT
curl -sS -o /dev/null -c "$COOKIE_JAR" "$BASE/auth/login/local?token=$LOGIN_TOKEN" \
    || die "local login request failed"
SESSION_JSON="$(curl -sS -b "$COOKIE_JAR" "$BASE/api/session")"
CSRF="$(json_field "$SESSION_JSON" csrf_token)"
[ -n "$CSRF" ] || die "owner session was not established; inspect the private service log locally"
ok "owner session minted"

info "POST /webapp/vnext/command (up to 120s)…"
HTTP_BODY="$(mktemp)"
HTTP_CODE="$(curl -sS -m 120 -o "$HTTP_BODY" -w '%{http_code}' -X POST "$BASE/webapp/vnext/command" \
    -b "$COOKIE_JAR" -H "Content-Type: application/json" -H "x-aegis-csrf: $CSRF" \
    --data '{"schema_version":1,"input":"Reply with exactly: aegis-chat-ok","mode":"text","intent_kind":"chat","client_context":"home"}')" || true
ANSWER="$(json_field "$(cat "$HTTP_BODY")" assistant_response.visible_answer)"
rm -f "$HTTP_BODY"
if [ "$HTTP_CODE" = 200 ] && [[ "$ANSWER" == *aegis-chat-ok* ]]; then
    ok "CHAT PASS — HTTP 200 and expected answer marker (provider output withheld)"
    "$LOCAL_SUPERVISOR" status || true
    echo ""
    ok "Chat verified. Manage the service with: $LOCAL_SUPERVISOR {status|logs -f|restart|stop|login}"
    finish_access
    if [ "${AEGIS_RUNTIME_MANAGED_SETUP:-0}" = 1 ]; then
        # The probe consumed the one-time login token. Restart for the owner.
        "$LOCAL_SUPERVISOR" restart
        if grep -qi microsoft /proc/sys/kernel/osrelease; then
            warn "WSL installation is not complete until the Windows session task is installed and the host-side idle check passes. Run scripts/install-windows.ps1 on Windows (see docs/INSTALLATION.md). Linux services alone do not keep WSL alive."
            exit 2
        fi
        ok "Persistent Linux installation ready. Use '$LOCAL_SUPERVISOR login' for your private one-time login link."
    fi
    exit 0
fi
warn "CHAT FAIL — HTTP $HTTP_CODE or missing expected answer. Provider output withheld; inspect the private service log locally."
if [ "$HTTP_CODE" = 503 ]; then
    warn "503 = configuration: run '$BIN setup-auth status' — the stored chat provider must have a live credential (Grok in-app / codex login / usable OPENAI_API_KEY)."
fi
exit 1
