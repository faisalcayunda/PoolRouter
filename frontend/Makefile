# Frontend gate (ADR-0003 sections 3.2 and 5).
# Toolchain runs in a pinned Node container image, so nothing is installed natively.
# Use FRONTEND_RUNNER=local on a machine (or CI job) whose Node matches .nvmrc.

SHELL := /bin/sh
.DEFAULT_GOAL := help
.NOTPARALLEL:

FRONTEND_RUNNER ?= podman
# Must match .nvmrc major and package.json engines. Never `latest`.
NODE_IMAGE ?= docker.io/library/node:24.22.2-bookworm-slim@sha256:c86ecead9e55a5210ac1727de38a4ca840bf03dcaf34dc9749addbf383b1920c
# Playwright image: tag must match the @playwright/test version in package.json.
PLAYWRIGHT_IMAGE ?= mcr.microsoft.com/playwright:v1.64.1-noble
# Optional podman label, e.g. CONTAINER_LABEL=shaal.issue=SHAA-197.
CONTAINER_LABEL ?=
# pnpm is activated through corepack from the packageManager field.
PNPM_HOME_DIR := /work/.cache/corepack
# Keep the pnpm store and metadata cache in .cache/ (git- and lint-ignored) so retries are incremental.
CACHE_ENV := -e npm_config_store_dir=/work/.cache/pnpm-store -e npm_config_cache=/work/.cache/npm
PNPM_INSTALL_FLAGS ?= --network-concurrency 2

ifeq ($(FRONTEND_RUNNER),podman)
RUN = podman run --rm $(if $(CONTAINER_LABEL),--label $(CONTAINER_LABEL)) \
	-v "$(CURDIR)":/work -w /work \
	-e COREPACK_HOME=$(PNPM_HOME_DIR) -e COREPACK_ENABLE_DOWNLOAD_PROMPT=0 -e CI=1 $(CACHE_ENV) \
	$(NODE_IMAGE) sh -c 'corepack enable && exec "$$@"' sh
RUN_PW = podman run --rm --ipc=host $(if $(CONTAINER_LABEL),--label $(CONTAINER_LABEL)) \
	-v "$(CURDIR)":/work -w /work \
	-e COREPACK_HOME=$(PNPM_HOME_DIR) -e COREPACK_ENABLE_DOWNLOAD_PROMPT=0 -e CI=1 $(CACHE_ENV) \
	$(PLAYWRIGHT_IMAGE) sh -c 'corepack enable && exec "$$@"' sh
else
RUN = corepack enable &&
RUN_PW = corepack enable &&
endif

PNPM = $(RUN) pnpm

.PHONY: help setup test lint fmt clean gen-api gen-api-check build e2e i18n-check i18n-types

help:
	@echo "targets: setup test lint fmt clean gen-api gen-api-check build e2e i18n-check i18n-types"

setup:
	$(PNPM) install --frozen-lockfile $(PNPM_INSTALL_FLAGS)

test:
	$(PNPM) exec vitest run

# lint = ESLint + Prettier --check + tsc --noEmit + i18n-check (Addendum A section 8).
lint: i18n-check
	$(PNPM) exec eslint .
	$(PNPM) exec prettier --check .
	$(PNPM) exec tsc --noEmit

# i18n-check: Addendum A rev 2 section 8. checkIdentical is a gate, see scripts/i18n-check.sh.
i18n-check:
	$(RUN) sh scripts/i18n-check.sh

i18n-types:
	$(PNPM) exec i18next-cli types

fmt:
	$(PNPM) exec prettier --write .

clean:
	rm -rf dist coverage test-results playwright-report node_modules/.vite

# gen-api: ADR-0003 section 4.2. backend/openapi/admin-api.yaml is owned by the backend stack.
gen-api:
	@test -f ../backend/openapi/admin-api.yaml || { echo "gen-api: ../backend/openapi/admin-api.yaml does not exist yet (backend stack); nothing to generate" >&2; exit 0; }
	$(PNPM) exec openapi-typescript ../backend/openapi/admin-api.yaml -o src/api/schema.gen.ts

gen-api-check:
	@test -f ../backend/openapi/admin-api.yaml || { echo "gen-api-check: ../backend/openapi/admin-api.yaml does not exist yet; skipped" >&2; exit 0; }
	$(MAKE) gen-api
	git diff --exit-code -- src/api/schema.gen.ts

build:
	$(PNPM) exec vite build

# e2e: Playwright against the built SPA served by `vite preview` (full Compose stack arrives with the screens).
e2e: build
	$(RUN_PW) pnpm exec playwright test
