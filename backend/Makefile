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
# SHA-256 of the official release tarballs (EmbarkStudios/cargo-deny $(CARGO_DENY_VERSION)), pinned here.
# cargo-deny is installed as this prebuilt binary, never compiled from source (SHAA-222).
CARGO_DENY_SHA256_aarch64 := 2b3567a60b7491c159d1cef8b7d8479d1ad2a31e29ef49462634ad4552fcc77d
CARGO_DENY_SHA256_x86_64 := 0e8c2aa59128612c90d9e09c02204e912f29a5b8d9a64671b94608cbe09e064f
# Optional podman label, e.g. CONTAINER_LABEL=shaal.issue=SHAA-191.
CONTAINER_LABEL ?=
# Memory cap per podman toolchain container; keeps two Rust builds within a 6 GiB VM (SHAA-222).
CONTAINER_MEMORY ?= 3g
JOBS ?= 2

export CARGO_HOME := $(CURDIR)/.cache/cargo
export CARGO_TERM_COLOR ?= never

ifeq ($(RUNNER),podman)
RUN = podman run --rm --memory $(CONTAINER_MEMORY) $(if $(CONTAINER_LABEL),--label $(CONTAINER_LABEL)) \
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

# Filled in by the API contract sub-issue (ADR-0004 section 4.1, utoipa code-first).
openapi:
	@echo "openapi: nothing to generate yet"

openapi-check:
	@echo "openapi-check: nothing to check yet"
