# Backend gate (ADR-0003 section 5, ADR-0004 section 2).
# Toolchain runs in a pinned container image, so nothing is installed natively.
# Use RUNNER=native on a machine (or CI job) that already has Rust 1.99.1.

SHELL := /bin/sh
.DEFAULT_GOAL := help
.NOTPARALLEL:

RUNNER ?= podman
# Sub-make lewat variabel perantara: GNU make menjalankan baris resep yang memuat teks `$(MAKE)` walau dengan -n (SHAA-398).
# Jangan tulis `$(MAKE)` langsung di resep yang punya efek samping container; uji: `make test-dry-run` di akar.
SUBMAKE = $(MAKE)
# Engine kontainer untuk RUNNER=podman atau RUNNER=docker. Docker dipakai lane Docker (SHAA-373, keputusan Luca).
CONTAINER_ENGINE := $(if $(filter docker,$(RUNNER)),docker,podman)
export BUILD_LOCK_ENGINE := $(CONTAINER_ENGINE)
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
# Podman label, required when RUNNER=podman: CONTAINER_LABEL=arctiva.issue=ARC-<n> (checked by check-label).
# RUNNER=native is only for Linux/CI jobs that already have Rust 1.99.1; never use it on the owner's Mac.
CONTAINER_LABEL ?=
# Memory cap per podman toolchain container. On the current 2 CPU / 3814 MiB VM only one Rust build fits:
# run one build at a time with JOBS=1 (see docs/runbook/backend-toolchain.md, SHAA-222).
CONTAINER_MEMORY ?= 3g
JOBS ?= 2

export CARGO_HOME := $(CURDIR)/.cache/cargo
export CARGO_TERM_COLOR ?= never

include ../scripts/build-lock.mk

# Docker lane (SHAA-390): satu CARGO_TARGET_DIR bersama di / dan satu CARGO_HOME/RUSTUP_HOME bersama di /data,
# dipakai bergantian oleh semua workdir di bawah kunci poolrouter-build-heavy. Semua path lewat variabel ini (override
# lewat env atau argumen make). Container berjalan dengan uid:gid host, jadi user biasa bisa menghapus isinya tanpa sudo.
SHARED_TARGET_DIR ?= /var/tmp/poolrouter-build/poolrouter-target
SHARED_CARGO_HOME ?= /data/workspace/poolrouter-build/cargo-home
# Batas ukuran target bersama (MB): di atasnya check-disk menjalankan clean-shared di bawah kunci.
SHARED_TARGET_LIMIT_MB ?= 2560
# Batas ukuran cargo-home + rustup bersama di /data (MB, SHAA-394). Dilewati => check-disk gagal (exit non-nol),
# tidak menghapus apa pun; bersihkan dengan `make clean-cache-shared` (eksplisit, di bawah kunci).
SHARED_CARGO_HOME_LIMIT_MB ?= 2560
# Sisa bebas minimum di / untuk sistem (MB), di luar ruang tumbuh target sampai batasnya.
ROOT_MIN_FREE_MB ?= 2048

ifeq ($(RUNNER),docker)
# Di dalam container: target, cargo, dan rustup bersama di-mount di /cache/*.
CT_CARGO_HOME := /cache/cargo
CT_RUSTUP_HOME := /cache/rustup
# Subdirektori, bukan akar mount: `cargo clean` menghapus direktori targetnya sendiri, dan akar mount tidak bisa dihapus.
CT_TARGET_DIR := /cache/target/build
SHARED_OPTS = --user $(shell id -u):$(shell id -g) \
	-v "$(SHARED_TARGET_DIR)":/cache/target -v "$(SHARED_CARGO_HOME)/cargo":$(CT_CARGO_HOME) -v "$(SHARED_CARGO_HOME)/rustup":$(CT_RUSTUP_HOME) \
	-e CARGO_TARGET_DIR=$(CT_TARGET_DIR)
else
CT_CARGO_HOME := /work/.cache/cargo
CT_RUSTUP_HOME := /work/.cache/rustup
SHARED_OPTS =
endif

ifneq ($(filter podman docker,$(RUNNER)),)
RUN_OPTS = $(CONTAINER_ENGINE) run --rm --memory $(CONTAINER_MEMORY) $(if $(CONTAINER_LABEL),--label $(CONTAINER_LABEL)) $(SHARED_OPTS) \
	-v "$(CURDIR)":/work -v "$(CURDIR)/../":/repo:ro -w /work \
	-e CARGO_HOME=$(CT_CARGO_HOME) -e RUSTUP_HOME=$(CT_RUSTUP_HOME) -e CARGO_TERM_COLOR=$(CARGO_TERM_COLOR) \
	-e CARGO_BUILD_JOBS=$(JOBS) -e CARGO_INCREMENTAL=0 -e CARGO_PROFILE_DEV_DEBUG=0 -e CARGO_PROFILE_TEST_DEBUG=0 -e SQLX_OFFLINE=true -e POOLROUTER_REPO_ROOT=/repo \
	-e PATH=$(CT_CARGO_HOME)/bin:/usr/local/cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
RUN = $(RUN_OPTS) $(RUST_IMAGE)
# Build berat (compile) lewat kunci bersama; satu build per host (K-41).
HEAVY = $(BUILD_LOCK_SH) run -- $(RUN_OPTS) $(BUILD_LOCK_OPTS) $(RUST_IMAGE)
else
RUN =
HEAVY = $(RUN)
export CARGO_BUILD_JOBS := $(JOBS)
export SQLX_OFFLINE := true
endif

# Path backend di sisi eksekusi: /work di dalam container Podman, direktori checkout untuk RUNNER=native (CI).
WORK_DIR := $(if $(filter podman docker,$(RUNNER)),/work,$(CURDIR))
# Repo root on the execution side (/repo is the read-only mount in the podman container).
REPO_DIR := $(if $(filter podman docker,$(RUNNER)),/repo,$(CURDIR)/..)

.PHONY: help check-label check-disk clean-shared clean-cache-shared setup test test-rust db-test uji-pg uji-pg-cargo uji-lua uji-redis uji-redis-cargo lua-rakit lua-rakit-check lint fmt clean openapi openapi-check

# Podman toolchain containers must carry the owning issue label, so untracked containers cannot appear (SHAA-222).
# Docker lane: cek ruang kosong sebelum target apa pun yang memakai container (SHAA-373).
CHECK_DISK := $(if $(filter docker,$(RUNNER)),check-disk,)
check-disk:
ifeq ($(RUNNER),docker)
	@mkdir -p "$(SHARED_TARGET_DIR)" "$(SHARED_CARGO_HOME)/cargo" "$(SHARED_CARGO_HOME)/rustup"
	@SHARED_CARGO_HOME="$(SHARED_CARGO_HOME)" SHARED_CARGO_HOME_LIMIT_MB="$(SHARED_CARGO_HOME_LIMIT_MB)" \
	  SHARED_TARGET_DIR="$(SHARED_TARGET_DIR)" SHARED_TARGET_LIMIT_MB="$(SHARED_TARGET_LIMIT_MB)" ROOT_MIN_FREE_MB="$(ROOT_MIN_FREE_MB)" \
	  CLEAN_SHARED_CMD='$(SUBMAKE) --no-print-directory clean-shared' ../scripts/docker-lane.sh check
else
	@true
endif

ifneq ($(filter podman docker,$(RUNNER)),)
check-label:
	@echo "$(CONTAINER_LABEL)" | grep -Eq '^arctiva\.issue=ARC-[0-9]+$$' || { echo "CONTAINER_LABEL wajib arctiva.issue=ARC-<n> untuk RUNNER=$(RUNNER) (contoh: make -C backend lint CONTAINER_LABEL=arctiva.issue=ARC-222)" >&2; exit 2; }
else
check-label:
endif

help:
	@echo "targets: setup test test-rust db-test uji-lua lua-rakit lua-rakit-check lint fmt clean openapi openapi-check build-lock-status build-lock-clean (RUNNER=podman|docker|native)"
	@echo "gate resmi (dari akar): make setup test lint RUNNER=docker FRONTEND_RUNNER=docker CONTAINER_LABEL=arctiva.issue=ARC-<n>"

# Fetch the locked dependency set and install the pinned cargo-deny. Toolchain copy, fetch dan install cargo-deny
# menulis ke cache bersama, jadi jalan di bawah kunci poolrouter-build-heavy (SHAA-394); checkout kedua hanya
# menemukan toolchain dan registry sudah ada dan tidak mengunduh ulang.
setup: check-label $(CHECK_DISK)
	$(if $(filter docker,$(RUNNER)),,mkdir -p .cache/cargo .cache/rustup)
	$(HEAVY) sh -c "[ -d $(CT_RUSTUP_HOME)/toolchains ] || cp -a /usr/local/rustup/. $(CT_RUSTUP_HOME)/; cargo fetch --locked"
	$(HEAVY) sh -c 'set -eu; \
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

test: test-rust db-test uji-pg uji-lua uji-redis

test-rust: check-label $(CHECK_DISK)
	$(HEAVY) cargo test --workspace --all-targets --locked

# Schema tests (SHAA-193): roles bootstrap, migrations from empty, grant matrix, constraints, functions.
# Starts a throwaway PostgreSQL 17 container labelled arctiva.issue=<n> (from CONTAINER_LABEL) and removes it afterwards.
# It needs a container engine on the host, so it is independent of RUNNER.
DB_ENGINE ?= $(if $(filter docker,$(RUNNER)),docker,podman)
db-test: check-label $(CHECK_DISK)
	CONTAINER_ENGINE=$(DB_ENGINE) ARC_ISSUE=$(patsubst arctiva.issue=%,%,$(CONTAINER_LABEL)) sh db/test/run.sh

# Uji integrasi Rust terhadap PostgreSQL sungguhan (SHAA-196): skema dari migrasi, koneksi sebagai pr_admin_api.
# Container Postgres independen dari RUNNER (seperti db-test); `uji-pg-cargo` menjalankan uji `#[ignore]` lewat kunci build.
uji-pg: check-label $(CHECK_DISK)
	CONTAINER_ENGINE=$(DB_ENGINE) ARC_ISSUE=$(patsubst arctiva.issue=%,%,$(CONTAINER_LABEL)) \
	sh db/test/uji-pg.sh "$(SUBMAKE) uji-pg-cargo RUNNER=$(RUNNER) CONTAINER_LABEL=$(CONTAINER_LABEL)"

uji-pg-cargo: BUILD_LOCK_OPTS += --network host -e POOLROUTER_TEST_DATABASE_URL -e POOLROUTER_TEST_DATABASE_URL_SU -e POOLROUTER_TEST_REDIS
uji-pg-cargo: check-label $(CHECK_DISK)
	$(HEAVY) cargo test --locked -p poolrouter-core --test sesi_store_pg -p poolrouter-webserver --test admin_sesi_pg -- --ignored --test-threads=1 --nocapture

# Lua quota scripts (SHAA-195). `lua/` holds sources with `-- sisip:` markers;
# `scripts/rakit-lua.sh` inlines `potongan/*.lua` into `rakitan/*.lua`, and that
# assembled form is what Redis loads (the sandbox has no `dofile`). Sources are
# not valid on their own, so the artifact is committed and gated: rakitan/ must
# always equal a fresh assembly.
LUA_SKRIP := crates/core/src/kuota/lua

lua-rakit:
	sh ../scripts/rakit-lua.sh

lua-rakit-check:
	tmp=$$(mktemp -d); trap 'rm -rf "$$tmp"' EXIT; \
	sh ../scripts/rakit-lua.sh --keluaran "$$tmp" >/dev/null; \
	diff -ru "$(LUA_SKRIP)/rakitan" "$$tmp"

# Harness runs the scripts against a real Redis (throwaway container, pinned image).
# Like db-test it needs a container engine on the host, so it is independent of RUNNER.
LUA_ENGINE ?= $(if $(filter docker,$(RUNNER)),docker,podman)
uji-lua: check-label $(CHECK_DISK) lua-rakit-check
	CONTAINER_ENGINE=$(LUA_ENGINE) ARC_ISSUE=$(patsubst arctiva.issue=%,%,$(CONTAINER_LABEL)) sh lua/test/run.sh

# Fail-closed, failover (`run_id` berubah) dan rebuild terhadap Redis sungguhan (SHAA-195).
# Redis jalan di jaringan host (port acak per proses, atau UJI_REDIS_PORT) supaya container cargo
# menjangkaunya; `uji-redis-cargo` menjalankan uji `#[ignore]` di crate core lewat kunci build.
uji-redis: check-label $(CHECK_DISK)
	CONTAINER_ENGINE=$(LUA_ENGINE) ARC_ISSUE=$(patsubst arctiva.issue=%,%,$(CONTAINER_LABEL)) \
	sh lua/test/uji-redis.sh "$(SUBMAKE) uji-redis-cargo RUNNER=$(RUNNER) CONTAINER_LABEL=$(CONTAINER_LABEL)"

uji-redis-cargo: BUILD_LOCK_OPTS += --network host -e POOLROUTER_TEST_REDIS -e POOLROUTER_TEST_FASE -e POOLROUTER_TEST_RUN_ID_LAMA
uji-redis-cargo: check-label $(CHECK_DISK)
	$(HEAVY) cargo test --locked -p poolrouter-core --test pemulihan_redis --test kuota_store_redis -- --ignored --test-threads=1 --nocapture

lint: check-label $(CHECK_DISK)
	$(RUN) cargo fmt --all --check
	$(HEAVY) cargo clippy --workspace --all-targets --locked -- -D warnings
	$(RUN) cargo deny --locked check

fmt: check-label $(CHECK_DISK)
	$(RUN) cargo fmt --all

# clean hanya membersihkan sisa lokal di workdir ini (aturan akhir run gate). Di RUNNER=docker target dan cargo-home
# bersama TIDAK disentuh; itu urusan clean-shared (dijalankan penjaga ukuran di check-disk, di bawah kunci).
ifeq ($(RUNNER),docker)
# Sisa lama (root-owned) dibersihkan lewat container root, hanya di dalam workdir ini.
clean: check-label
	docker run --rm $(if $(CONTAINER_LABEL),--label $(CONTAINER_LABEL)) -v "$(CURDIR)":/work -w /work $(RUST_IMAGE) \
	  rm -rf -- target .cache

# Hapus isi target bersama dengan `cargo clean`, di bawah kunci heavy (exit 75 bila kunci dipegang build lain).
# Hanya target build; cargo-home bersama tidak disentuh.
clean-shared: check-label
	@mkdir -p "$(SHARED_TARGET_DIR)" "$(SHARED_CARGO_HOME)/cargo" "$(SHARED_CARGO_HOME)/rustup"
	$(HEAVY) cargo clean

# Pembersihan cache bersama (registry, git, dan opsional toolchain), eksplisit dan di bawah kunci heavy (SHAA-394).
# Tanpa argumen: hanya cache registry/git yang bisa diunduh ulang (setup berikutnya mengisi lagi). Toolchain rustup
# dan cargo-deny hanya dihapus dengan CLEAN_TOOLCHAIN=1. Path tetap di dalam mount bersama, tanpa glob ke luar.
clean-cache-shared: check-label
	@mkdir -p "$(SHARED_TARGET_DIR)" "$(SHARED_CARGO_HOME)/cargo" "$(SHARED_CARGO_HOME)/rustup"
	$(HEAVY) sh -c 'rm -rf -- $(CT_CARGO_HOME)/registry $(CT_CARGO_HOME)/git $(if $(CLEAN_TOOLCHAIN),$(CT_CARGO_HOME)/bin $(CT_RUSTUP_HOME)/toolchains $(CT_RUSTUP_HOME)/settings.toml $(CT_RUSTUP_HOME)/update-hashes)'
else
# Mode podman menulis target dan .cache sebagai root di checkout ini; hapus lewat container, bukan `rm` host.
clean: check-label
	$(RUN) rm -rf -- target .cache

clean-shared:
	@echo "clean-shared hanya untuk RUNNER=docker" >&2; exit 2

clean-cache-shared:
	@echo "clean-cache-shared hanya untuk RUNNER=docker" >&2; exit 2
endif

# OpenAPI is generated from code (ADR-0004 section 4.1, utoipa code-first).
openapi: check-label $(CHECK_DISK)
	mkdir -p openapi
	$(HEAVY) cargo run --locked -q -p poolrouter-webserver --bin openapi-gen -- openapi

# Fails when the committed YAML differs from what the code generates, and when
# the working-tree contracts break the baseline OASDIFF_BASE (oasdiff, SHAA-192, SHAA-278).
openapi-check: check-label $(CHECK_DISK) openapi oasdiff-breaking
	@test -z "$$(git status --porcelain -- openapi)" || { echo "openapi: generated files are uncommitted or modified" >&2; git status --short -- openapi; exit 1; }
	git diff --exit-code -- openapi

# oasdiff `breaking` against an explicit baseline (SHAA-278 D1-D6, SHAA-279).
# OASDIFF_BASE is a git rev. Locally it defaults to origin/main; in CI (CI=true) it must be
# set explicitly by the workflow (a shallow detached checkout has no `main`), otherwise exit 1.
# OASDIFF_BASE_SOURCE only labels the log line (push.before | dispatch | default).
# Unresolvable base rev -> exit 1. A contract missing in the baseline -> ::warning:: for that file only.
# downstream-api.yaml breaking -> exit 1; admin-api.yaml breaking -> ::warning:: only (ADR-0003 section 4.2 item 3).
_OASDIFF_SRC := $(if $(filter undefined,$(origin OASDIFF_BASE)),default,explicit)
OASDIFF_BASE_SOURCE ?= $(_OASDIFF_SRC)
OASDIFF_BASE ?= $(if $(filter true,$(CI)),,origin/main)
oasdiff-breaking: check-label $(CHECK_DISK)
	@set -eu; \
	base="$(OASDIFF_BASE)"; \
	if [ -z "$$base" ]; then echo "oasdiff: OASDIFF_BASE must be set explicitly when CI=true" >&2; exit 1; fi; \
	sha=$$(git rev-parse --verify --quiet "$$base^{commit}") || { echo "oasdiff: baseline ref '$$base' cannot be resolved" >&2; exit 1; }; \
	echo "oasdiff: baseline $$sha ($(OASDIFF_BASE_SOURCE))"; \
	mkdir -p .cache/oasdiff; \
	for name in downstream-api admin-api; do \
	  rm -f ".cache/oasdiff/base-$$name.yaml"; \
	  if git cat-file -e "$$sha:backend/openapi/$$name.yaml" 2>/dev/null; then \
	    git show "$$sha:backend/openapi/$$name.yaml" > ".cache/oasdiff/base-$$name.yaml"; \
	  else \
	    echo "::warning title=oasdiff baseline::$$name.yaml not present in baseline $$sha; breaking check skipped for this file"; \
	  fi; \
	done; \
	$(RUN) sh $(REPO_DIR)/scripts/oasdiff-compare.sh "$(WORK_DIR)" "$(OASDIFF_VERSION)" "$(OASDIFF_SHA256_arm64)" "$(OASDIFF_SHA256_amd64)"
