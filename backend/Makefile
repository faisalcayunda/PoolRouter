# Backend gate (ADR-0003 section 5, ADR-0004 section 2).
# Toolchain runs in a pinned container image, so nothing is installed natively.
# Use RUNNER=native on a machine (or CI job) that already has Rust 1.99.1.

SHELL := /bin/sh
.DEFAULT_GOAL := help
.NOTPARALLEL:

RUNNER ?= podman
# Must match rust-toolchain.toml and [workspace.package] rust-version (ADR-0004 section 2). Never `latest`.
# Pinned by tag and digest, same literal as backend/Containerfile line 17, .github/workflows/backend.yml line 30,
# and .github/workflows/contract.yml line 29. Changing one means changing all four (SHAA-207).
RUST_IMAGE ?= docker.io/library/rust:1.99.1-bookworm@sha256:9181b56c2a21ab1d5e73cffe04fc762426d30aa7fac3c91141dd033f49a7fc8b
CARGO_DENY_VERSION ?= 0.19.0
# SHA-256 of the official release tarballs (EmbarkStudios/cargo-deny $(CARGO_DENY_VERSION)), pinned here.
# cargo-deny is installed as this prebuilt binary, never compiled from source (SHAA-222).
CARGO_DENY_SHA256_aarch64 := 2b3567a60b7491c159d1cef8b7d8479d1ad2a31e29ef49462634ad4552fcc77d
CARGO_DENY_SHA256_x86_64 := 0e8c2aa59128612c90d9e09c02204e912f29a5b8d9a64671b94608cbe09e064f
# oasdiff breaking-change gate (SHAA-192). Pinned release of oasdiff/oasdiff; the
# upstream 1.9.x line NIA referenced no longer exists in releases (current line
# 1.2x-1.3x), so we pin the latest stable at adoption time. Installed as a prebuilt
# binary with a pinned SHA-256, never compiled from source (pattern of SHAA-222).
OASDIFF_VERSION ?= 1.35.0
OASDIFF_SHA256_arm64 := 1f8b5aa033cd0240ae6c53460268bcd6e578526666a4487a781ece8cef39fbdd
OASDIFF_SHA256_amd64 := a8023002fcb2eaa7f5758148a9303cb63d33b939886df312268bc0fc7b939140
# Podman label, required when RUNNER=podman: CONTAINER_LABEL=shaal.issue=SHAA-<n> (checked by check-label).
# RUNNER=native is only for Linux/CI jobs that already have Rust 1.99.1; never use it on the owner's Mac.
CONTAINER_LABEL ?=
# Memory cap per podman toolchain container. On the current 2 CPU / 3814 MiB VM only one Rust build fits:
# run one build at a time with JOBS=1 (see docs/runbook/backend-toolchain.md, SHAA-222).
CONTAINER_MEMORY ?= 3g
JOBS ?= 2

export CARGO_HOME := $(CURDIR)/.cache/cargo
export CARGO_TERM_COLOR ?= never

include ../scripts/build-lock.mk

ifeq ($(RUNNER),podman)
RUN_OPTS = podman run --rm --memory $(CONTAINER_MEMORY) $(if $(CONTAINER_LABEL),--label $(CONTAINER_LABEL)) \
	-v "$(CURDIR)":/work -v "$(CURDIR)/../":/repo:ro -w /work \
	-e CARGO_HOME=/work/.cache/cargo -e RUSTUP_HOME=/work/.cache/rustup -e CARGO_TERM_COLOR=$(CARGO_TERM_COLOR) \
	-e CARGO_BUILD_JOBS=$(JOBS) -e SQLX_OFFLINE=true -e POOLROUTER_REPO_ROOT=/repo \
	-e PATH=/work/.cache/cargo/bin:/usr/local/cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
RUN = $(RUN_OPTS) $(RUST_IMAGE)
# Build berat (compile) lewat kunci bersama; satu build per host (K-41).
HEAVY = $(BUILD_LOCK_SH) run -- $(RUN_OPTS) $(BUILD_LOCK_OPTS) $(RUST_IMAGE)
else
RUN =
HEAVY = $(RUN)
export CARGO_BUILD_JOBS := $(JOBS)
export SQLX_OFFLINE := true
endif

.PHONY: help check-label setup test test-rust db-test lint fmt clean openapi openapi-check

# Podman toolchain containers must carry the owning issue label, so untracked containers cannot appear (SHAA-222).
ifeq ($(RUNNER),podman)
check-label:
	@echo "$(CONTAINER_LABEL)" | grep -Eq '^shaal\.issue=SHAA-[0-9]+$$' || { echo "CONTAINER_LABEL wajib shaal.issue=SHAA-<n> untuk RUNNER=podman (contoh: make -C backend lint CONTAINER_LABEL=shaal.issue=SHAA-222)" >&2; exit 2; }
else
check-label:
endif

help:
	@echo "targets: setup test test-rust db-test lint fmt clean openapi openapi-check build-lock-status build-lock-clean (RUNNER=podman|native)"

# Fetch the locked dependency set and install the pinned cargo-deny.
setup: check-label
	mkdir -p .cache/cargo .cache/rustup
	$(RUN) sh -c "[ -d /work/.cache/rustup/toolchains ] || cp -a /usr/local/rustup/. /work/.cache/rustup/; cargo fetch --locked"
	$(RUN) sh -c 'set -eu; \
	if cargo deny --version 2>/dev/null | grep -q "$(CARGO_DENY_VERSION)"; then exit 0; fi; \
	arch=$$(uname -m); \
	case "$$arch" in \
	  aarch64) sha="$(CARGO_DENY_SHA256_aarch64)" ;; \
	  x86_64) sha="$(CARGO_DENY_SHA256_x86_64)" ;; \
	  *) echo "setup: unsupported arch $$arch" >&2; exit 1 ;; \
	esac; \
	t=$$(mktemp -d); trap "rm -rf \"$$t\"" EXIT; \
	url="https://github.com/EmbarkStudios/cargo-deny/releases/download/$(CARGO_DENY_VERSION)/cargo-deny-$(CARGO_DENY_VERSION)-$$arch-unknown-linux-musl.tar.gz"; \
	curl -fsSL -o "$$t/cargo-deny.tar.gz" "$$url"; \
	echo "$$sha  $$t/cargo-deny.tar.gz" | sha256sum -c -; \
	tar -xzf "$$t/cargo-deny.tar.gz" -C "$$t"; \
	mkdir -p "$$CARGO_HOME/bin"; \
	install -m 0755 "$$t/cargo-deny-$(CARGO_DENY_VERSION)-$$arch-unknown-linux-musl/cargo-deny" "$$CARGO_HOME/bin/cargo-deny"; \
	cargo deny --version'

test: test-rust db-test

test-rust: check-label
	$(HEAVY) cargo test --workspace --all-targets --locked

# Schema tests (SHAA-193): roles bootstrap, migrations from empty, grant matrix, constraints, functions.
# Starts a throwaway PostgreSQL 17 container labelled shaal.issue=<n> (from CONTAINER_LABEL) and removes it afterwards.
# It needs a container engine on the host, so it is independent of RUNNER.
DB_ENGINE ?= podman
db-test: check-label
	CONTAINER_ENGINE=$(DB_ENGINE) SHAA_ISSUE=$(patsubst shaal.issue=%,%,$(CONTAINER_LABEL)) sh db/test/run.sh

lint: check-label
	$(RUN) cargo fmt --all --check
	$(HEAVY) cargo clippy --workspace --all-targets --locked -- -D warnings
	$(RUN) cargo deny --locked check

fmt: check-label
	$(RUN) cargo fmt --all

clean: check-label
	$(RUN) cargo clean
	rm -rf -- .cache/cargo/registry/cache

# OpenAPI is generated from code (ADR-0004 section 4.1, utoipa code-first).
openapi: check-label
	mkdir -p openapi
	$(HEAVY) cargo run --locked -q -p poolrouter-webserver --bin openapi-gen -- openapi

# Fails when the committed YAML differs from what the code generates, and when
# the working-tree contracts break the ones committed on main (oasdiff, SHAA-192).
openapi-check: check-label openapi oasdiff-breaking
	@test -z "$$(git status --porcelain -- openapi)" || { echo "openapi: generated files are uncommitted or modified" >&2; git status --short -- openapi; exit 1; }
	git diff --exit-code -- openapi

# oasdiff `breaking` against the contracts on main. Skipped until main carries
# both YAML files (first merge of SHAA-192), after that every contract change
# is diffed and a breaking change fails the gate (ADR-0001 rev 15 section 5a.7).
oasdiff-breaking: check-label
	@set -eu; \
	if ! git cat-file -e "main:backend/openapi/downstream-api.yaml" 2>/dev/null || ! git cat-file -e "main:backend/openapi/admin-api.yaml" 2>/dev/null; then \
	  echo "oasdiff: baseline belum ada di main (pertama kali); dilewati"; exit 0; \
	fi; \
	mkdir -p .cache/oasdiff; \
	git show main:backend/openapi/downstream-api.yaml > .cache/oasdiff/base-downstream.yaml; \
	git show main:backend/openapi/admin-api.yaml > .cache/oasdiff/base-admin.yaml; \
	$(RUN) sh -c 'set -eu; \
	if [ ! -x /work/.cache/oasdiff/oasdiff ] || ! /work/.cache/oasdiff/oasdiff --version 2>/dev/null | grep -q "$(OASDIFF_VERSION)"; then \
	  arch=$$(uname -m); \
	  case "$$arch" in \
	    aarch64) urlarch=arm64; sha="$(OASDIFF_SHA256_arm64)" ;; \
	    x86_64) urlarch=amd64; sha="$(OASDIFF_SHA256_amd64)" ;; \
	    *) echo "oasdiff: unsupported arch $$arch" >&2; exit 1 ;; \
	  esac; \
	  t=$$(mktemp -d); trap "rm -rf \"$$t\"" EXIT; \
	  url="https://github.com/oasdiff/oasdiff/releases/download/v$(OASDIFF_VERSION)/oasdiff_$(OASDIFF_VERSION)_linux_$${urlarch}.tar.gz"; \
	  curl -fsSL -o "$$t/o.tar.gz" "$$url"; \
	  echo "$$sha  $$t/o.tar.gz" | sha256sum -c -; \
	  tar -xzf "$$t/o.tar.gz" -C "$$t" oasdiff; \
	  mv "$$t/oasdiff" /work/.cache/oasdiff/oasdiff; \
	fi; \
	rc=0; \
	for pair in "base-downstream.yaml downstream-api.yaml" "base-admin.yaml admin-api.yaml"; do \
	  set -- $$pair; \
	  if ! /work/.cache/oasdiff/oasdiff breaking --fail-on ERR "/work/.cache/oasdiff/$$1" "/work/openapi/$$2"; then \
	    echo "oasdiff: perubahan breaking pada $$2 terhadap main" >&2; \
	    rc=1; \
	  fi; \
	done; \
	exit $$rc'
