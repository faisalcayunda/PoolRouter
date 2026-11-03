# Backend gate (ADR-0003 section 5, ADR-0004 section 2).
# Toolchain runs in a pinned container image, so nothing is installed natively.
# Use RUNNER=native on a machine (or CI job) that already has Rust 1.99.1.

SHELL := /bin/sh
.DEFAULT_GOAL := help
.NOTPARALLEL:

RUNNER ?= podman
# Must match rust-toolchain.toml and [workspace.package] rust-version (ADR-0004 section 2). Never `latest`.
RUST_IMAGE ?= docker.io/library/rust:1.99.1-bookworm
CARGO_DENY_VERSION ?= 0.19.0
# Optional podman label, e.g. CONTAINER_LABEL=shaal.issue=SHAA-191.
CONTAINER_LABEL ?=
JOBS ?= 2

export CARGO_HOME := $(CURDIR)/.cache/cargo
export CARGO_TERM_COLOR ?= never

ifeq ($(RUNNER),podman)
RUN = podman run --rm $(if $(CONTAINER_LABEL),--label $(CONTAINER_LABEL)) \
	-v "$(CURDIR)":/work -w /work \
	-e CARGO_HOME=/work/.cache/cargo -e RUSTUP_HOME=/work/.cache/rustup -e CARGO_TERM_COLOR=$(CARGO_TERM_COLOR) \
	-e CARGO_BUILD_JOBS=$(JOBS) -e SQLX_OFFLINE=true \
	-e PATH=/work/.cache/cargo/bin:/usr/local/cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin \
	$(RUST_IMAGE)
else
RUN =
export CARGO_BUILD_JOBS := $(JOBS)
export SQLX_OFFLINE := true
endif

.PHONY: help setup test lint fmt clean openapi openapi-check

help:
	@echo "targets: setup test lint fmt clean openapi openapi-check (RUNNER=podman|native)"

# Fetch the locked dependency set and install the pinned cargo-deny.
setup:
	mkdir -p .cache/cargo .cache/rustup
	$(RUN) sh -c "[ -d /work/.cache/rustup/toolchains ] || cp -a /usr/local/rustup/. /work/.cache/rustup/; cargo fetch --locked"
	$(RUN) sh -c 'cargo deny --version 2>/dev/null | grep -q "$(CARGO_DENY_VERSION)" || CARGO_PROFILE_RELEASE_LTO=off CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16 CARGO_BUILD_JOBS=1 cargo install cargo-deny --locked --version $(CARGO_DENY_VERSION)'

test:
	$(RUN) cargo test --workspace --all-targets --locked

lint:
	$(RUN) cargo fmt --all --check
	$(RUN) cargo clippy --workspace --all-targets --locked -- -D warnings
	$(RUN) cargo deny --locked check

fmt:
	$(RUN) cargo fmt --all

clean:
	$(RUN) cargo clean
	rm -rf -- .cache/cargo/registry/cache

# OpenAPI is generated from code (ADR-0004 section 4.1, utoipa code-first).
openapi:
	mkdir -p openapi
	$(RUN) cargo run --locked -q -p poolrouter-webserver --bin openapi-gen -- openapi

# Fails when the committed YAML differs from what the code generates.
openapi-check: openapi
	git diff --exit-code -- openapi
