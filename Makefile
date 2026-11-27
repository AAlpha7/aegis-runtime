.PHONY: build test test-unit lint skills-check check rust-fast-check rust-critical-paths webapp webapp-test remote-access-test heirloom-mode-test package clean

BINARY   := aegis_runtime_core
TARGET_DIR := $(if $(CARGO_TARGET_DIR),$(CARGO_TARGET_DIR),target)
VERSION  := $(shell grep '^version' core/Cargo.toml | head -1 | sed 's/.*"\(.*\)"/\1/')
DIST_DIR := dist
ARCHIVE  := $(DIST_DIR)/aegis-runtime-$(VERSION)-$(shell uname -s | tr A-Z a-z)-$(shell uname -m).tar.gz
RUST_NATIVE_TEST_PACKAGES := -p aegis-runtime-core -p aegis-executor

# Use mold linker on aarch64 Linux if available (30-50% faster linking)
MOLD_FLAGS :=
ifeq ($(shell uname -m),aarch64)
  ifneq (,$(shell command -v mold 2>/dev/null))
    ifneq (,$(shell command -v clang 2>/dev/null))
      MOLD_FLAGS := -C linker=clang -C link-arg=-fuse-ld=mold
    endif
  endif
endif
export RUSTFLAGS += $(MOLD_FLAGS)

# Host binaries only. skills/* are wasm32-wasip1 crates: core/build.rs
# compiles them into compiled_skills/ (see SKIP_SKILL_BUILD below).
build:
	cargo build --release -p aegis-runtime-core -p aegis-executor

# compiled_skills/*.wasm are gitignored. Unless SKIP_SKILL_BUILD=1, core/build.rs
# compiles every skills/* crate for wasm32-wasip1 (needs
# `rustup target add wasm32-wasip1`, which rust-toolchain.toml requests) and
# several integration tests load the result. Default: skip only when the
# artifacts already exist (web_reader.wasm is the canary), so the first
# `make test` on a clean clone builds them and later runs stay fast.
# Override explicitly with `make test SKIP_SKILL_BUILD=0` (CI) or
# `SKIP_SKILL_BUILD=1`.
SKIP_SKILL_BUILD ?= $(shell test -f compiled_skills/web_reader.wasm && echo 1 || echo 0)
test:
	SKIP_SKILL_BUILD=$(SKIP_SKILL_BUILD) cargo test $(RUST_NATIVE_TEST_PACKAGES) --no-fail-fast

test-unit:
	SKIP_SKILL_BUILD=$(SKIP_SKILL_BUILD) cargo test -p aegis-runtime-core --lib

lint:
	cargo clippy -p aegis-runtime-core --all-targets -- -D warnings

# Type-check every wasm skill crate (all workspace members except the host
# crates) for its real target; the host build never compiles them natively.
# RUSTFLAGS is cleared for the same reason core/build.rs clears it: host
# linker flags (mold/clang) must not leak into the wasm32 build.
skills-check:
	RUSTFLAGS= cargo check --workspace --exclude aegis-runtime-core --exclude aegis-executor --target wasm32-wasip1

check: lint test webapp-test jev-setup-test installer-test release-packaging-test

.PHONY: release-packaging-test
release-packaging-test:
	python3 scripts/test_release_packaging.py
	python3 scripts/test_privacy_history.py
	python3 scripts/test_release_compiler.py

.PHONY: installer-test install-smoke
installer-test:
	python3 scripts/test_managed_runtime.py
	python3 scripts/test_foreground_runtime.py

# Uses the selected provider quota; deliberately separate from developer tests.
install-smoke:
	python3 scripts/managed-runtime.py smoke

.PHONY: jev-setup-test
jev-setup-test:
	python3 scripts/test_jev_setup.py

rust-fast-check:
	cargo fmt --all -- --check
	cargo clippy -p aegis-runtime-core --all-targets -- -D warnings
	SKIP_SKILL_BUILD=1 cargo test -p aegis-runtime-core --lib
	cargo test -p aegis-executor

# Critical user-journey integration tests: route/HITL sensors and the
# production-pattern journey binary.
rust-critical-paths:
	SKIP_SKILL_BUILD=1 cargo test -p aegis-runtime-core --test smoke_critical_paths --test structural_guards --test user_journeys --test user_journey_v2 --test route_coverage_guard --test hitl_routing_linkage

webapp:
	cd webapp && npm ci && npm run build

webapp-test:
	cd webapp && npm ci && npm test -- --run

# No subscriptions, model keys or Tailscale account. Install Chromium with
# `cd webapp && npx playwright install chromium` before the first browser run.
remote-access-test:
	python3 scripts/test_remote_access.py
	SKIP_SKILL_BUILD=$(SKIP_SKILL_BUILD) cargo test -p aegis-runtime-core --lib remote_access::tests
	SKIP_SKILL_BUILD=$(SKIP_SKILL_BUILD) cargo build -p aegis-runtime-core --bin aegis_runtime_access --example remote_access_harness
	node webapp/scripts/test-remote-access.mjs
	python3 scripts/test_access_lifecycle.py
	python3 scripts/test_foreground_lifecycle.py

heirloom-mode-test:
	SKIP_SKILL_BUILD=$(SKIP_SKILL_BUILD) cargo build -p aegis-runtime-core --bin aegis_runtime_core
	python3 scripts/test_heirloom_mode.py

.PHONY: release-build
release-build:
	python3 scripts/build_release.py

package: release-build webapp
	python3 scripts/package_release.py --target $(TARGET_DIR) --archive $(ARCHIVE)

clean:
	cargo clean
	rm -rf compiled_skills $(DIST_DIR)
	cd webapp && rm -rf dist node_modules || true
