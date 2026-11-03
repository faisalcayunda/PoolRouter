# Frontend gate (ADR-0003 sections 3.2 and 5).
# Toolchain runs in a pinned Node container image, so nothing is installed natively.
# Use FRONTEND_RUNNER=local on a machine (or CI job) whose Node matches .nvmrc.

SHELL := /bin/sh
.DEFAULT_GOAL := help
.NOTPARALLEL:

FRONTEND_RUNNER ?= podman
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

ifeq ($(FRONTEND_RUNNER),podman)
# The repository root is mounted at /work and the workdir is /work/frontend, so
# paths written relative to frontend/ resolve as they do on the host: ../docs for
# the ADR a test asserts against, ../backend/openapi for gen-api. Mounting only
# frontend/ silently broke both.
RUN = podman run --rm --memory $(CONTAINER_MEMORY) --label $(CONTAINER_LABEL) \
	-v "$(CURDIR)/..":/work -w /work/frontend \
	-e COREPACK_HOME=$(PNPM_HOME_DIR) -e COREPACK_ENABLE_DOWNLOAD_PROMPT=0 -e CI=1 $(CACHE_ENV) \
	$(NODE_IMAGE) sh -c 'corepack enable && exec "$$@"' sh
# --ipc=host gives Chromium the host /dev/shm (Playwright's documented invocation);
# with the host IPC namespace, --shm-size is rejected, so it is not set.
# The Playwright image has no corepack and needs no package manager: the repo's
# node_modules is bind-mounted, so the pinned binaries run directly.
RUN_PW = podman run --rm --memory $(CONTAINER_MEMORY) --ipc=host --label $(CONTAINER_LABEL) \
	-e HOME=/tmp -e CI=1 \
	-v "$(CURDIR)/..":/work -w /work/frontend \
	$(PLAYWRIGHT_IMAGE)
else
RUN = corepack enable &&
RUN_PW = true
endif

PNPM = $(RUN) pnpm

.PHONY: help check-label setup test lint fmt clean gen-api gen-api-check build e2e i18n-check i18n-types

help:
	@echo "targets: setup test lint fmt clean gen-api gen-api-check build e2e i18n-check i18n-types"

# Every podman container must carry shaal.issue=SHAA-<n> (playbook section 6).
# Skipped for FRONTEND_RUNNER=local, where no container is created.

ifeq ($(FRONTEND_RUNNER),podman)
check-label:
	@echo "$(CONTAINER_LABEL)" | grep -Eq '^shaal\.issue=SHAA-[0-9]+$$' || { echo "CONTAINER_LABEL wajib shaal.issue=SHAA-<n> untuk FRONTEND_RUNNER=podman (contoh: make -C frontend lint CONTAINER_LABEL=shaal.issue=SHAA-197)" >&2; exit 2; }
else
check-label:
endif

setup: check-label
	$(PNPM) install --frozen-lockfile $(PNPM_INSTALL_FLAGS)

test: check-label
	$(PNPM) exec vitest run

# lint = ESLint + Prettier --check + tsc --noEmit + i18n-check (Addendum A section 8).
lint: check-label i18n-check
	$(PNPM) exec eslint .
	$(PNPM) exec prettier --check .
	$(PNPM) exec tsc --noEmit

# i18n-check: Addendum A rev 2 section 8. checkIdentical is a gate, see scripts/i18n-check.sh.
i18n-check: check-label
	$(RUN) sh scripts/i18n-check.sh

i18n-types: check-label
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

build: check-label
	$(PNPM) exec vite build

# e2e: Playwright (Chromium) against the built SPA served by `vite preview`, in the pinned
# Playwright image. Full Compose stack arrives with the screens. Heavy build: follow the VM rules.
e2e: check-label build
	$(RUN_PW) ./node_modules/.bin/playwright test
