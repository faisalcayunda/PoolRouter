# Frontend gate (ADR-0003 sections 3.2 and 5).
# Toolchain runs in a pinned Node container image, so nothing is installed natively.
# Use FRONTEND_RUNNER=local on a machine (or CI job) whose Node matches .nvmrc.

SHELL := /bin/sh
.DEFAULT_GOAL := help
.NOTPARALLEL:

FRONTEND_RUNNER ?= podman
# Penjaga nilai FRONTEND_RUNNER (SHAA-373): nilai di luar daftar gagal dengan exit 2 saat parse,
# tanpa fallback diam-diam ke jalur native (corepack enable di host).
ifeq ($(filter podman docker local,$(FRONTEND_RUNNER)),)
$(error FRONTEND_RUNNER=$(FRONTEND_RUNNER) tidak dikenal: pakai podman, docker, atau local)
endif
CONTAINER_ENGINE := $(if $(filter docker,$(FRONTEND_RUNNER)),docker,podman)
# Must match .nvmrc major and package.json engines. Never `latest`.
NODE_IMAGE ?= docker.io/library/node:24.22.2-bookworm-slim@sha256:c86ecead9e55a5210ac1727de38a4ca840bf03dcaf34dc9749addbf383b1920c
# Playwright image: the tag must match @playwright/test in package.json, and the
# digest pins the multi-arch image index (ADR-0003 section 6). Never a bare tag.
PLAYWRIGHT_IMAGE ?= mcr.microsoft.com/playwright:v1.64.1-noble@sha256:2f05f8a734509a7b3f26a912639488ee08f58a7b292d14d62271bcff9038768f
# Podman label, required when FRONTEND_RUNNER=podman: CONTAINER_LABEL=shaal.issue=SHAA-<n>
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

.PHONY: help check-label setup test lint fmt clean gen-api gen-api-check build build-e2e e2e i18n-check i18n-types build-lock-status build-lock-clean

help:
	@echo "targets: setup test lint fmt clean gen-api gen-api-check build build-e2e e2e i18n-check i18n-types"
	@echo "lock:    build-lock-status build-lock-clean (K-41 heavy build lock, shared with backend)"

# Every podman container must carry shaal.issue=SHAA-<n> (playbook section 6).
# Skipped for FRONTEND_RUNNER=local, where no container is created.

ifneq ($(filter podman docker,$(FRONTEND_RUNNER)),)
check-label:
	@echo "$(CONTAINER_LABEL)" | grep -Eq '^shaal\.issue=SHAA-[0-9]+$$' || { echo "CONTAINER_LABEL wajib shaal.issue=SHAA-<n> untuk FRONTEND_RUNNER=$(FRONTEND_RUNNER) (contoh: make -C frontend lint CONTAINER_LABEL=shaal.issue=SHAA-197)" >&2; exit 2; }
else
check-label:
endif

CHECK_DISK := $(if $(filter docker,$(FRONTEND_RUNNER)),check-disk,)
check-disk:
	@$(if $(filter docker,$(FRONTEND_RUNNER)),docker ps --filter label=shaal.issue=$(patsubst shaal.issue=%,%,$(CONTAINER_LABEL)) --format '{{.ID}} {{.Names}} {{.Status}}'; ../scripts/docker-lane.sh check,true)

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

clean: check-label
	rm -rf dist coverage test-results playwright-report node_modules/.vite

# gen-api: ADR-0003 section 4.2. backend/openapi/admin-api.yaml is owned by the backend stack.
gen-api: check-label
	@test -f ../backend/openapi/admin-api.yaml || { echo "gen-api: ../backend/openapi/admin-api.yaml does not exist yet (backend stack); nothing to generate" >&2; exit 0; }
	$(PNPM) exec openapi-typescript ../backend/openapi/admin-api.yaml -o src/api/schema.gen.ts

gen-api-check: check-label
	@test -f ../backend/openapi/admin-api.yaml || { echo "gen-api-check: ../backend/openapi/admin-api.yaml does not exist yet; skipped" >&2; exit 0; }
	$(MAKE) gen-api
	git diff --exit-code -- src/api/schema.gen.ts

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
