#!/usr/bin/env bash
# bootstrap-system-deps.sh — idempotent host package bootstrap for Aegis Runtime.
#
# Installs required Ubuntu/Debian packages plus best-effort build accelerators.
# Rust itself is still managed by rustup in setup-dev.sh so production and
# developer hosts can choose the toolchain channel explicitly.

set -euo pipefail

export DEBIAN_FRONTEND="${DEBIAN_FRONTEND:-noninteractive}"
export NEEDRESTART_MODE="${NEEDRESTART_MODE:-a}"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m'

info() { echo -e "${CYAN}[bootstrap]${NC} $*"; }
ok() { echo -e "${GREEN}[bootstrap]${NC} $*"; }
warn() { echo -e "${YELLOW}[bootstrap]${NC} $*"; }
die() { echo -e "${RED}[bootstrap]${NC} $*" >&2; exit 1; }

if [ "$(uname -s)" != "Linux" ] || ! command -v apt-get >/dev/null 2>&1; then
    warn "Automatic package bootstrap currently supports apt-based Linux only."
    warn "Install equivalent packages manually: compiler, pkg-config, OpenSSL headers, Node.js/npm, clang/mold/sccache."
    exit 0
fi

APT=(apt-get)
if [ "$(id -u)" -ne 0 ]; then
    command -v sudo >/dev/null 2>&1 || die "sudo is required to install packages as a non-root user"
    APT=(sudo apt-get)
fi

required_packages=(
    build-essential
    pkg-config
    libssl-dev
    ca-certificates
    curl
    fontconfig
    fonts-noto-cjk
    git
)

optional_packages=(
    clang
    mold
)

install_latest_sccache() {
    [ "${AEGIS_RUNTIME_SKIP_SCCACHE_INSTALL:-0}" = "1" ] && return 0

    local version="${AEGIS_RUNTIME_SCCACHE_VERSION:-latest}"
    if [ "$version" = "latest" ]; then
        info "Resolving latest sccache release..."
        local latest_url
        latest_url=$(curl -fsSLI -o /dev/null -w '%{url_effective}' \
            https://github.com/mozilla/sccache/releases/latest) || {
            warn "Could not resolve latest sccache release"
            return 0
        }
        version="${latest_url##*/}"
    fi
    case "$version" in
        v*) ;;
        *) version="v$version" ;;
    esac

    local current=""
    if command -v sccache >/dev/null 2>&1; then
        current="$(sccache --version 2>/dev/null | awk '{print "v"$2}' || true)"
    fi
    if [ "$current" = "$version" ]; then
        ok "sccache $version already installed"
        return 0
    fi

    local target
    case "$(uname -m)" in
        aarch64|arm64) target="aarch64-unknown-linux-musl" ;;
        x86_64|amd64) target="x86_64-unknown-linux-musl" ;;
        *)
            warn "No prebuilt sccache target mapping for architecture: $(uname -m)"
            return 0
            ;;
    esac

    local asset="sccache-${version}-${target}.tar.gz"
    local base="https://github.com/mozilla/sccache/releases/download/${version}"
    local work="/tmp/aegis-runtime-sccache-${version}-${target}"
    rm -rf "$work"
    mkdir -p "$work"

    info "Installing sccache $version from official release asset..."
    curl -fsSL -o "$work/$asset" "$base/$asset"
    curl -fsSL -o "$work/$asset.sha256" "$base/$asset.sha256"

    local expected actual bin
    expected="$(tr -d '[:space:]' < "$work/$asset.sha256")"
    actual="$(sha256sum "$work/$asset" | awk '{print $1}')"
    if [ "$expected" != "$actual" ]; then
        rm -rf "$work"
        warn "sccache checksum mismatch; leaving existing installation untouched"
        return 0
    fi

    tar -xzf "$work/$asset" -C "$work"
    bin="$(find "$work" -maxdepth 3 -type f -name sccache -print -quit)"
    if [ -z "$bin" ]; then
        rm -rf "$work"
        warn "sccache archive did not contain an sccache binary"
        return 0
    fi

    if [ "$(id -u)" -eq 0 ]; then
        install -m 0755 "$bin" /usr/local/bin/sccache
    else
        sudo install -m 0755 "$bin" /usr/local/bin/sccache
    fi
    rm -rf "$work"
    ok "Installed $(/usr/local/bin/sccache --version) at /usr/local/bin/sccache"
}

missing_required=()
for pkg in "${required_packages[@]}"; do
    if ! dpkg -s "$pkg" >/dev/null 2>&1; then
        missing_required+=("$pkg")
    fi
done

missing_optional=()
if [ "${AEGIS_RUNTIME_SKIP_OPTIONAL_DEPS:-0}" != "1" ]; then
    for pkg in "${optional_packages[@]}"; do
        if dpkg -s "$pkg" >/dev/null 2>&1; then
            continue
        fi
        if apt-cache policy "$pkg" 2>/dev/null | grep -q 'Candidate: (none)'; then
            warn "Optional package unavailable from configured apt sources: $pkg"
            continue
        fi
        missing_optional+=("$pkg")
    done
fi

if [ "${#missing_required[@]}" -eq 0 ] && [ "${#missing_optional[@]}" -eq 0 ]; then
    ok "System packages already installed"
else
    if [ "${AEGIS_RUNTIME_SKIP_APT_UPDATE:-0}" != "1" ]; then
        info "Updating apt package index..."
        "${APT[@]}" update -qq
    fi
fi

if [ "${#missing_required[@]}" -gt 0 ]; then
    info "Installing required packages: ${missing_required[*]}"
    "${APT[@]}" install -y -qq "${missing_required[@]}"
fi

if [ "${#missing_optional[@]}" -gt 0 ]; then
    info "Installing build accelerators: ${missing_optional[*]}"
    "${APT[@]}" install -y -qq "${missing_optional[@]}"
fi

install_latest_sccache

node_major="$(node -p 'process.versions.node.split(".")[0]' 2>/dev/null || true)"
if ! [[ "$node_major" =~ ^[0-9]+$ ]] || [ "$node_major" -lt 22 ] || ! command -v npm >/dev/null 2>&1; then
    warn "WebApp builds need Node.js 22+ and npm; install Node 22 from the official distribution or a version manager. Ubuntu's nodejs/npm packages may be older."
fi

ok "System package bootstrap complete"
