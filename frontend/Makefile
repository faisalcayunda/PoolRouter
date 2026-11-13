# Frontend gate (ADR-0003 sections 3.2 and 5).
# Toolchain runs in a pinned Node container image, so nothing is installed natively.
# Use FRONTEND_RUNNER=local on a machine (or CI job) whose Node matches .nvmrc.

SHELL := /bin/sh
.DEFAULT_GOAL := help
.NOTPARALLEL:

FRONTEND_RUNNER ?= podman
CONTAINER_ENGINE := $(if $(filter docker,$(FRONTEND_RUNNER)),docker,podman)
# Must match .nvmrc major and package.json engines. Never `latest`.
NODE_IMAGE ?= docker.io/library/node:24.22.2-bookworm-slim@sha256:c86ecead9e55a5210ac1727de38a4ca840bf03dcaf34dc9749addbf383b1920c
# Playwright image: the tag must match @playwright/test in package.json, and the
# digest pins the multi-arch image index (ADR-0003 section 6). Never a bare tag.
PLAYWRIGHT_IMAGE ?= mcr.microsoft.com/playwright:v1.64.1-noble@sha256:2f05f8a734509a7b3f26a912639488ee08f58a7b292d14d62271bcff9038768f
# Podman label, required when FRONTEND_RUNNER=podman: CONTAINER_LABEL=arctiva.issue=ARC-<n>
# (playbook section 6, same contract as backend/Makefile after SHAA-222; checked by check-label).
CONTAINER_LABEL ?=
# Memory cap for every frontend container, same variable name as backend/Makefile.
CONTAINER_MEMORY ?= 2g
# pnpm is activated through corepack from the packageManager field.
PNPM_HOME_DIR := /work/frontend/.cache/corepack
# Keep the pnpm store and metadata cache in .cache/ (git- and lint-ignored) so retries are incremental.
CACHE_ENV := -e npm_config_store_dir=/work/frontend/.cache/pnpm-store -e npm_config_cache=/work/frontend/.cache/npm
PNPM_INSTALL_FLAGS ?= --network-concurrency 2
NODE_CMD = $(NODE_IMAGE) sh -c 'corepack enable && exec "$$@"' sh

# Shared "one heavy build per host" lock (SHAA-194, K-41). Same include and HEAVY
# shape as backend/Makefile; there is no second lock implementation in this repo.
include ../scripts/build-lock.mk

ifneq ($(filter podman docker,$(FRONTEND_RUNNER)),)
# The repository root is mounted at /work and the workdir is /work/frontend, so
# paths written relative to frontend/ resolve as they do on the host: ../docs for
# the ADR a test asserts against, ../backend/openapi for gen-api. Mounting only
# frontend/ silently broke both.
RUN_OPTS = $(CONTAINER_ENGINE) run --rm --memory $(CONTAINER_MEMORY) --label $(CONTAINER_LABEL) \
	-v "$(CURDIR)/..":/work -w /work/frontend \
	-e COREPACK_HOME=$(PNPM_HOME_DIR) -e COREPACK_ENABLE_DOWNLOAD_PROMPT=0 -e CI=1 $(CACHE_ENV)
RUN = $(RUN_OPTS) $(NODE_CMD)
# --ipc=host gives Chromium the host /dev/shm (Playwright's documented invocation);
# with the host IPC namespace, --shm-size is rejected, so it is not set.
# The Playwright image has no corepack and needs no package manager: the repo's
# node_modules is bind-mounted, so the pinned binaries run directly.
RUN_PW_OPTS = $(CONTAINER_ENGINE) run --rm --memory $(CONTAINER_MEMORY) --ipc=host --label $(CONTAINER_LABEL) \
	-e HOME=/tmp -e CI=1 \
	-v "$(CURDIR)/..":/work -w /work/frontend
RUN_PW = $(RUN_PW_OPTS) $(PLAYWRIGHT_IMAGE)
# Heavy targets take the lock: pnpm install, vite build, vitest, and the Chromium e2e run.
# lint, fmt, i18n-check, and clean stay unlocked (podman run only, no node_modules writes).
# On a held lock they exit 75 (EX_TEMPFAIL): do non-compile work and retry, never wait in a loop.
HEAVY = $(BUILD_LOCK_SH) run -- $(RUN_OPTS) $(BUILD_LOCK_OPTS) $(NODE_CMD)
HEAVY_PW = $(BUILD_LOCK_SH) run -- $(RUN_PW_OPTS) $(BUILD_LOCK_OPTS) $(PLAYWRIGHT_IMAGE)
else
RUN = corepack enable &&
RUN_PW = true
HEAVY = $(RUN)
HEAVY_PW = $(RUN_PW)
endif

PNPM = $(RUN) pnpm

.PHONY: help check-label setup test lint fmt clean gen-api gen-api-check build build-e2e e2e e2e-api-asli e2e-smoke i18n-check i18n-types build-lock-status build-lock-clean

help:
	@echo "targets: setup test lint fmt clean gen-api gen-api-check build build-e2e e2e e2e-api-asli e2e-smoke i18n-check i18n-types"
	@echo "gate resmi (dari akar): make setup test lint RUNNER=<podman|docker> FRONTEND_RUNNER=<podman|docker> CONTAINER_LABEL=arctiva.issue=ARC-<n>"
	@echo "lock:    build-lock-status build-lock-clean (K-41 heavy build lock, shared with backend)"

# Every podman container must carry arctiva.issue=ARC-<n> (playbook section 6).
# Skipped for FRONTEND_RUNNER=local, where no container is created.

ifneq ($(filter podman docker,$(FRONTEND_RUNNER)),)
# Suffix opsional (mis. -ci-<run_id>, dipakai .github/workflows/smoke.yml ARC-378 spec.md
# bagian 5 untuk keunikan antar run workflow_dispatch) tetap diterima; label masih harus
# berawal arctiva.issue=ARC-<n>.
check-label:
	@echo "$(CONTAINER_LABEL)" | grep -Eq '^arctiva\.issue=ARC-[0-9]+(-[A-Za-z0-9._-]+)?$$' || { echo "CONTAINER_LABEL wajib arctiva.issue=ARC-<n> untuk FRONTEND_RUNNER=$(FRONTEND_RUNNER) (contoh: make -C frontend lint CONTAINER_LABEL=arctiva.issue=ARC-197)" >&2; exit 2; }
else
check-label:
endif

CHECK_DISK := $(if $(filter docker,$(FRONTEND_RUNNER)),check-disk,)
check-disk:
	@$(if $(filter docker,$(FRONTEND_RUNNER)),docker ps --filter label=arctiva.issue=$(patsubst arctiva.issue=%,%,$(CONTAINER_LABEL)) --format '{{.ID}} {{.Names}} {{.Status}}'; ../scripts/docker-lane.sh check,true)

# Heavy targets run under the K-41 lock: they create node_modules or compile bundles.
# setup (pnpm install), build (vite build), test (vitest), and e2e (Chromium) are locked.
# A held lock exits 75 (EX_TEMPFAIL): switch to non-compile work and retry, don't wait.
setup: check-label $(CHECK_DISK)
	$(HEAVY) pnpm install --frozen-lockfile $(PNPM_INSTALL_FLAGS)

test: check-label $(CHECK_DISK)
	$(HEAVY) pnpm exec vitest run

# lint = ESLint + Prettier --check + tsc --noEmit + i18n-check (Addendum A section 8).
lint: check-label $(CHECK_DISK) i18n-check
	$(PNPM) exec eslint .
	$(PNPM) exec prettier --check .
	$(PNPM) exec tsc --noEmit

# i18n-check: Addendum A rev 2 section 8. checkIdentical is a gate, see scripts/i18n-check.sh.
i18n-check: check-label $(CHECK_DISK)
	$(RUN) sh scripts/i18n-check.sh

i18n-types: check-label $(CHECK_DISK)
	$(PNPM) exec i18next-cli types

fmt: check-label
	$(PNPM) exec prettier --write .

# clean (SHAA-391): node_modules dan .cache dibuat container sebagai root, jadi rm dari host gagal
# Permission denied. Container berlabel menghapusnya; yang di-mount hanya direktori frontend checkout ini.
# Nama yang dihapus ditulis literal. Penjaga: cwd harus /work dan berisi package.json. Tidak memakai kunci build.
# FRONTEND_RUNNER=local (CI): rm host dengan penjaga yang sama terhadap direktori ini.
CLEAN_PATHS := node_modules .cache dist coverage test-results playwright-report
ifneq ($(filter podman docker,$(FRONTEND_RUNNER)),)
clean: check-label
	$(CONTAINER_ENGINE) run --rm --label $(CONTAINER_LABEL) -v "$(CURDIR)":/work -w /work $(NODE_IMAGE) \
		sh -c 'set -eu; [ "$$(pwd -P)" = /work ] && [ -f /work/package.json ] || { echo "clean: bukan direktori frontend, berhenti" >&2; exit 1; }; rm -rf -- $(CLEAN_PATHS)'
else
clean:
	@set -eu; d=$$(pwd -P); [ -f "$$d/package.json" ] && [ -e "$$d/../.git" ] || { echo "clean: bukan direktori frontend checkout, berhenti" >&2; exit 1; }; \
	cd "$$d"; rm -rf -- $(CLEAN_PATHS)
endif

# gen-api: ADR-0003 section 4.2. backend/openapi/admin-api.yaml is owned by the backend stack.
gen-api: check-label
	@test -f ../backend/openapi/admin-api.yaml || { echo "gen-api: ../backend/openapi/admin-api.yaml does not exist yet (backend stack); nothing to generate" >&2; exit 0; }
	$(PNPM) exec openapi-typescript ../backend/openapi/admin-api.yaml -o src/api/schema.gen.ts

gen-api-check: check-label
	@test -f ../backend/openapi/admin-api.yaml || { echo "gen-api-check: ../backend/openapi/admin-api.yaml does not exist yet; skipped" >&2; exit 0; }
	$(PNPM) exec openapi-typescript ../backend/openapi/admin-api.yaml -o .cache/schema.gen.check.ts
	diff -u src/api/schema.gen.ts .cache/schema.gen.check.ts || { echo "gen-api-check: src/api/schema.gen.ts is stale; run 'make -C frontend gen-api' and commit" >&2; exit 1; }

build: check-label $(CHECK_DISK)
	$(HEAVY) pnpm exec vite build

# The e2e suite needs the role seam (`?peran=`) that production builds must not have, so it builds
# with the `playwright` mode; see `define` in vite.config.ts.
build-e2e: check-label $(CHECK_DISK)
	$(HEAVY) pnpm exec vite build --mode playwright

# e2e: Playwright (Chromium) against the built SPA served by `vite preview`, in the pinned
# Playwright image. Full Compose stack arrives with the screens. The `build-e2e` prerequisite takes
# the lock and releases it before this recipe runs, so the two never nest.
e2e: check-label $(CHECK_DISK) build-e2e
	$(HEAVY_PW) ./node_modules/.bin/playwright test

# e2e-api-asli (ARC-251): suite `@api-asli` terhadap backend sungguhan. Dipanggil oleh
# scripts/e2e-api-asli.sh di akar repo, yang menyalakan stack dan memberi E2E_NETNS (container frontend
# compose, supaya origin 127.0.0.1:8080 sama dengan POOLROUTER_UI_ORIGIN dan cookie __Host- Secure
# diterima) dan E2E_ENV_FILE (E2E_BASE_URL dan E2E_SESSION_TOKEN, di luar repo). Tidak menyentuh
# suite mock di `e2e`. Hanya untuk runner kontainer.
ifneq ($(filter podman docker,$(FRONTEND_RUNNER)),)
e2e-api-asli: check-label
	@test -n "$(E2E_NETNS)" && test -n "$(E2E_ENV_FILE)" || { echo "e2e-api-asli dipanggil lewat make e2e-api-asli di akar repo (butuh E2E_NETNS dan E2E_ENV_FILE)" >&2; exit 2; }
	$(BUILD_LOCK_SH) run -- $(RUN_PW_OPTS) --network container:$(E2E_NETNS) --env-file $(E2E_ENV_FILE) \
		$(BUILD_LOCK_OPTS) $(PLAYWRIGHT_IMAGE) ./node_modules/.bin/playwright test -c playwright.api-asli.config.ts
else
e2e-api-asli:
	@echo "e2e-api-asli hanya untuk FRONTEND_RUNNER=podman atau docker" >&2; exit 2
endif

# e2e-smoke (ARC-378): suite `@smoke` di netns container frontend compose, sama alasan dengan
# e2e-api-asli: origin 127.0.0.1:8080 harus sama dengan POOLROUTER_UI_ORIGIN (cookie __Host-
# Secure), dan backend-webserver:8081 (downstream API, grup klien G3) hanya lewat network compose
# internal, tidak dipublikasikan ke host. Dipanggil oleh scripts/smoke.sh di akar repo, yang
# menyalakan stack dan memberi E2E_NETNS (container frontend) dan E2E_ENV_FILE (token sesi per
# peran, di luar repo). PLAYWRIGHT_JSON_OUTPUT_NAME opsional (reporter JSON untuk laporan.mjs).
ifneq ($(filter podman docker,$(FRONTEND_RUNNER)),)
e2e-smoke: check-label
	@test -n "$(E2E_NETNS)" && test -n "$(E2E_ENV_FILE)" || { echo "e2e-smoke dipanggil lewat make smoke di akar repo (butuh E2E_NETNS dan E2E_ENV_FILE)" >&2; exit 2; }
	$(BUILD_LOCK_SH) run -- $(RUN_PW_OPTS) --network container:$(E2E_NETNS) --env-file $(E2E_ENV_FILE) \
		-e PLAYWRIGHT_JSON_OUTPUT_NAME=$(PLAYWRIGHT_JSON_OUTPUT_NAME) \
		$(BUILD_LOCK_OPTS) $(PLAYWRIGHT_IMAGE) ./node_modules/.bin/playwright test -c playwright.smoke.config.ts --reporter=json,list
else
e2e-smoke:
	@echo "e2e-smoke hanya untuk FRONTEND_RUNNER=podman atau docker" >&2; exit 2
endif
