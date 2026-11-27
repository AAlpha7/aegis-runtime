#!/usr/bin/env bash
# ensure-local-embedding.sh — install/wire a local Ollama embedding model.
#
# Grok / Codex first-run has no usable OPENAI_API_KEY. Without a separate
# embedding endpoint the core falls back to OpenRouter with an empty key and
# every embed() gets a noisy 401 while L2 stays empty. Production (Oracle)
# already runs a local Ollama embedder; this script mirrors that dependency
# for portable / Cloud hosts.
#
# Default model: nomic-embed-text (768d) — matches the L2 lesson threshold
# calibration in core (nomic-embed-text v1.5).
#
# Usage:
#   scripts/ensure-local-embedding.sh              # install + pull + write .env
#   scripts/ensure-local-embedding.sh --check      # exit 0 only if ready
#   scripts/ensure-local-embedding.sh --write-env  # only write .env (assume ollama ready)
#
# Environment:
#   AEGIS_RUNTIME_ROOT                 checkout root (default: parent of scripts/)
#   AEGIS_RUNTIME_EMBED_MODEL          default nomic-embed-text
#   AEGIS_RUNTIME_EMBED_DIMENSIONS     default 768
#   AEGIS_RUNTIME_EMBED_API_BASE       default http://127.0.0.1:11434/v1
#   AEGIS_RUNTIME_SKIP_LOCAL_EMBEDDING=1  no-op success (tests / air-gap skip)

set -euo pipefail
umask 077

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="${AEGIS_RUNTIME_ROOT:-$(cd "$SCRIPT_DIR/.." && pwd)}"
MODEL="${AEGIS_RUNTIME_EMBED_MODEL:-nomic-embed-text}"
DIMS="${AEGIS_RUNTIME_EMBED_DIMENSIONS:-768}"
API_BASE="${AEGIS_RUNTIME_EMBED_API_BASE:-http://127.0.0.1:11434/v1}"
API_KEY="${AEGIS_RUNTIME_EMBED_API_KEY:-ollama}"
OLLAMA_INSTALL='curl -fsSL https://ollama.com/install.sh | sh'

MODE=ensure
case "${1:-}" in
    --check) MODE=check ;;
    --write-env) MODE=write-env ;;
    -h|--help) sed -n '2,28p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; exit 0 ;;
    "") ;;
    *) echo "Unknown argument: $1 (see --help)" >&2; exit 2 ;;
esac

CYAN='\033[0;36m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; RED='\033[0;31m'; NC='\033[0m'
info() { echo -e "${CYAN}[local-embed]${NC} $*"; }
ok()   { echo -e "${GREEN}[local-embed]${NC} $*"; }
warn() { echo -e "${YELLOW}[local-embed]${NC} $*"; }
die()  { echo -e "${RED}[local-embed]${NC} $*" >&2; exit 1; }

install_steps() {
    cat <<EOF
Install steps (local embedding dependency — required for L2 without OPENAI_API_KEY):
  1) $OLLAMA_INSTALL
  2) ollama serve   # if not already running as a service
  3) ollama pull $MODEL
  4) scripts/ensure-local-embedding.sh --write-env
Or re-run: scripts/ensure-local-embedding.sh
EOF
}

if [ "${AEGIS_RUNTIME_SKIP_LOCAL_EMBEDDING:-0}" = 1 ]; then
    ok "skipped (AEGIS_RUNTIME_SKIP_LOCAL_EMBEDDING=1)"
    exit 0
fi

dotenv_get() {
    local key="$1"
    [ -f "$ROOT/.env" ] || return 0
    grep -E "^${key}=" "$ROOT/.env" | tail -1 | cut -d= -f2- | tr -d '"' | tr -d "'" || true
}

dotenv_set() {
    local key="$1" value="$2" tmp
    [ -f "$ROOT/.env" ] || { cp "$ROOT/.env.example" "$ROOT/.env" 2>/dev/null || touch "$ROOT/.env"; }
    tmp="$(mktemp)"
    if grep -qE "^${key}=" "$ROOT/.env"; then
        awk -v k="$key" -v v="$value" 'BEGIN{done=0} $0 ~ "^"k"=" && !done {print k"="v; done=1; next} {print}' "$ROOT/.env" > "$tmp"
    else
        cat "$ROOT/.env" > "$tmp"
        printf '%s=%s\n' "$key" "$value" >> "$tmp"
    fi
    cat "$tmp" > "$ROOT/.env"
    rm -f "$tmp"
}

write_env() {
    dotenv_set EMBEDDING_API_BASE "$API_BASE"
    dotenv_set EMBEDDING_API_KEY "$API_KEY"
    dotenv_set EMBEDDING_MODEL "$MODEL"
    dotenv_set EMBEDDING_DIMENSIONS "$DIMS"
    ok "wrote EMBEDDING_* to $ROOT/.env (base=$API_BASE model=$MODEL dims=$DIMS)"
}

ollama_bin() {
    if command -v ollama >/dev/null 2>&1; then command -v ollama; return 0; fi
    [ -x /usr/local/bin/ollama ] && { echo /usr/local/bin/ollama; return 0; }
    return 1
}

embedding_ready() {
    local base="${API_BASE%/}"
    local probe
    probe="$(mktemp)"
    if curl -fsS -m 30 -o "$probe" \
        -H "Content-Type: application/json" \
        -H "Authorization: Bearer $API_KEY" \
        -d "{\"model\":\"$MODEL\",\"input\":\"ping\",\"dimensions\":$DIMS}" \
        "$base/embeddings" 2>/dev/null; then
        if python3 -c 'import json, math, sys; v=json.load(open(sys.argv[1]))["data"][0]["embedding"]; assert len(v)==int(sys.argv[2]) and all(isinstance(x,(int,float)) and not isinstance(x,bool) and math.isfinite(x) for x in v)' "$probe" "$DIMS" 2>/dev/null; then
            rm -f "$probe"
            return 0
        fi
    fi
    rm -f "$probe"
    return 1
}

ensure_persistence() {
    if [ "${AEGIS_RUNTIME_MANAGED_SETUP:-0}" = 1 ]; then
        case "$API_BASE" in
            http://127.0.0.1:11434/v1|http://localhost:11434/v1)
                python3 "$SCRIPT_DIR/managed-runtime.py" embedding-start ;;
            *) info "Using your configured embedding endpoint; its service lifetime is managed separately." ;;
        esac
    fi
}

model_present() {
    local bin
    bin="$(ollama_bin)" || return 1
    "$bin" list 2>/dev/null | awk 'NR>1 {print $1}' | grep -qx "$MODEL" \
        || "$bin" list 2>/dev/null | awk 'NR>1 {print $1}' | grep -q "^${MODEL}:"
}

case "$MODE" in
    write-env)
        embedding_ready || die "Embedding validation failed; existing configuration is unchanged."
        ensure_persistence
        write_env
        exit 0
        ;;
    check)
        if embedding_ready; then
            ok "ready — $MODEL via $API_BASE"
            exit 0
        fi
        warn "not ready"
        install_steps >&2
        exit 1
        ;;
esac

# ── ensure ────────────────────────────────────────────────────────────
if embedding_ready; then
    ensure_persistence
    ok "already serving $MODEL at $API_BASE"
    write_env
    exit 0
fi

if ! ollama_bin >/dev/null; then
    info "ollama not found — installing: $OLLAMA_INSTALL"
    if [ -t 0 ] && [ -t 1 ] \
       && [ "${AEGIS_RUNTIME_ASSUME_YES:-0}" != 1 ] \
       && [ "${AEGIS_RUNTIME_ASSUME_YES:-}" != yes ]; then
        read -rp "Run the Ollama installer now? [Y/n] " answer
        [ -z "$answer" ] || [[ "$answer" =~ ^[Yy] ]] || {
            install_steps >&2
            die "ollama is required for local embedding (L2) without OPENAI_API_KEY"
        }
    fi
    bash -c "$OLLAMA_INSTALL" || {
        install_steps >&2
        die "ollama install failed"
    }
    export PATH="/usr/local/bin:$PATH"
    ollama_bin >/dev/null || {
        install_steps >&2
        die "ollama still not on PATH after install"
    }
fi
ok "ollama: $(ollama_bin)"
ensure_persistence

# Start daemon if needed (Cloud VMs often have no systemd unit yet).
if ! curl -fsS -m 2 -o /dev/null http://127.0.0.1:11434/api/tags 2>/dev/null; then
    if [ "${AEGIS_RUNTIME_MANAGED_SETUP:-0}" != 1 ]; then
        warn "Starting temporary Ollama process: agent exit/logout may stop semantic memory. Managed first-run uses a persistent service."
        nohup "$(ollama_bin)" serve >>"${TMPDIR:-/tmp}/ollama-serve.log" 2>&1 &
    fi
    for _ in 1 2 3 4 5 6 7 8 9 10; do
        curl -fsS -m 2 -o /dev/null http://127.0.0.1:11434/api/tags 2>/dev/null && break
        sleep 1
    done
    curl -fsS -m 2 -o /dev/null http://127.0.0.1:11434/api/tags 2>/dev/null \
        || { install_steps >&2; die "ollama serve did not become ready (see ${TMPDIR:-/tmp}/ollama-serve.log)"; }
fi

if ! model_present; then
    info "pulling $MODEL (first pull can take a few minutes)"
    "$(ollama_bin)" pull "$MODEL" || {
        install_steps >&2
        die "ollama pull $MODEL failed"
    }
fi

if ! embedding_ready; then
    install_steps >&2
    die "embed probe against $API_BASE/embeddings failed after pull — L2 would be unavailable"
fi

write_env
ok "local embedding dependency ready"
exit 0
