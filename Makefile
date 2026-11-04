# Entry point repo PoolRouter (ADR-0001 Addendum A §3.2, §4).
# Tidak ada `podman` atau `docker` yang ditulis langsung di sini: pilih engine lewat CONTAINER_ENGINE.
#
#   make compose-up  ENV=dev CONTAINER_ENGINE=podman
#   make compose-lint
#
# ENV_FILE default ke deploy/compose/env/<ENV>.env (di-gitignore). Untuk staging dan produksi,
# isi ENV_FILE dengan path file env di host target, di luar repo.

.NOTPARALLEL:

ENV ?= dev
CONTAINER_ENGINE ?= podman
# JOBS diteruskan ke build-arg image backend. Default 1 cocok untuk VM Podman 2 CPU / 3,7 GiB (SHAA-222). CI memberi JOBS=2 eksplisit.
JOBS ?= 1
PROJECT ?= poolrouter-$(ENV)
COMPOSE_DIR := deploy/compose
ENV_FILE ?= $(COMPOSE_DIR)/env/$(ENV).env
# EXTRA_COMPOSE_FILES: override tambahan, mis. label harness uji (deploy/compose/test/).
EXTRA_COMPOSE_FILES ?=
COMPOSE_FILES := -f $(COMPOSE_DIR)/compose.yaml -f $(COMPOSE_DIR)/compose.$(ENV).yaml $(addprefix -f ,$(EXTRA_COMPOSE_FILES))
# COMPOSE_PROFILES: dev default ke bundled-infra. Staging dan prod wajib diisi eksplisit (ADR-0001 Addendum §3.1).
COMPOSE_PROFILES ?=
EFF_PROFILES := $(if $(COMPOSE_PROFILES),$(COMPOSE_PROFILES),$(if $(filter dev,$(ENV)),bundled-infra,))
PROFILE_ARGS := $(addprefix --profile ,$(EFF_PROFILES))
# Provider P3 (Docker Compose v2 dari container). Variabel ini hanya dibaca oleh podman.
# Provider membaca file env dan direktori secret dari luar repo lewat mount read-only.
SECRETS_DIR := $(shell sed -n 's/^POOLROUTER_SECRETS_DIR=//p' "$(ENV_FILE)" 2>/dev/null | head -n 1)
EXTRA_MOUNTS := $(abspath $(dir $(ENV_FILE))):$(SECRETS_DIR)
COMPOSE_ENV := PODMAN_COMPOSE_PROVIDER=$(CURDIR)/$(COMPOSE_DIR)/bin/compose-provider POOLROUTER_EXTRA_MOUNTS=$(EXTRA_MOUNTS)
COMPOSE := $(COMPOSE_ENV) $(CONTAINER_ENGINE) compose -p $(PROJECT) $(COMPOSE_FILES) --env-file $(ENV_FILE)
LINT_ENV_FILES := $(COMPOSE_DIR)/env/dev.env.example
IMAGE_TAG ?= dev

.PHONY: check-env compose-up compose-down compose-ps compose-logs compose-config compose-migrate compose-lint compose-test-dind image

check-env:
	@case "$(ENV)" in dev|staging|prod) ;; *) echo "ENV harus dev, staging, atau prod" >&2; exit 2 ;; esac
	@case "$(CONTAINER_ENGINE)" in podman|docker) ;; *) echo "CONTAINER_ENGINE harus podman atau docker" >&2; exit 2 ;; esac
	@test -f "$(ENV_FILE)" || { echo "ENV_FILE $(ENV_FILE) tidak ada (lihat docs/runbook/deploy.md)" >&2; exit 2; }
	@if [ -z "$(EFF_PROFILES)" ]; then echo "COMPOSE_PROFILES wajib diisi eksplisit untuk ENV=$(ENV) (bundled-infra atau managed; lihat docs/runbook/deploy.md)" >&2; exit 2; fi

# compose-config: pemeriksaan wajib sebelum up. Merender dengan ENV_FILE yang akan dipakai.
compose-config: check-env
	$(COMPOSE) --profile '*' config -q

compose-up: check-env
	$(COMPOSE) $(PROFILE_ARGS) up -d --wait

# compose-migrate: jalankan setelah compose-up (bundled-infra sudah healthy). Tidak menaikkan app.
compose-migrate: check-env
	$(COMPOSE) $(PROFILE_ARGS) --profile migrate run --rm migrate

compose-down: check-env
	$(COMPOSE) $(PROFILE_ARGS) down

compose-ps: check-env
	$(COMPOSE) $(PROFILE_ARGS) ps

compose-logs: check-env
	$(COMPOSE) $(PROFILE_ARGS) logs --tail=200

# compose-lint: V1 (ADR-0001 Addendum A §5, §8). Skrip dan fixture di deploy/compose/lint/.
compose-lint:
	COMPOSE_CMD="$(COMPOSE_ENV) $(CONTAINER_ENGINE) compose" \
	COMPOSE_DIR="$(COMPOSE_DIR)" \
	sh $(COMPOSE_DIR)/lint/compose-lint.sh

# compose-test-dind: bukti V3 di Docker Engine sementara (harness DinD, butuh SHAA_ISSUE dan ENV_FILE).
compose-test-dind:
	SHAA_ISSUE="$(SHAA_ISSUE)" ENV_FILE="$(ENV_FILE)" PROFILES="$(EFF_PROFILES)" \
	sh $(COMPOSE_DIR)/test/dind.sh

# image: membangun image dari Containerfile dengan engine terpilih (Addendum §3.2).
# Compose tidak dipakai untuk build di staging dan produksi.
image:
	@test -f backend/Containerfile || { echo "backend/Containerfile belum ada (menunggu SHAA-175)" >&2; exit 2; }
	$(CONTAINER_ENGINE) build -f backend/Containerfile --build-arg JOBS=$(JOBS) -t localhost/poolrouter-backend:$(IMAGE_TAG) backend

# Repo gate (ADR-0003 section 5): `make setup test lint` from the repo root.
# Each stack keeps its own Makefile; these targets only delegate. frontend/ is
# included once it has a Makefile, so adding it needs no change here.
STACKS := backend $(if $(wildcard frontend/Makefile),frontend)

.PHONY: help setup test lint fmt clean push-main

help:
	@echo "targets: setup test lint fmt clean (gate repo, delegasi ke stack); push-main (gate penuh lalu push ke main, wajib CONTAINER_LABEL=shaal.issue=SHAA-<n>); compose-up compose-down compose-ps compose-logs compose-config compose-migrate compose-lint compose-test-dind image"

# push-main: satu-satunya jalur push ke main (CLAUDE.md). Gate penuh harus hijau pada hash yang di-push (SHAA-328).
push-main:
	@RUNNER="$(RUNNER)" FRONTEND_RUNNER="$(FRONTEND_RUNNER)" CONTAINER_LABEL="$(CONTAINER_LABEL)" MAKE="$(MAKE)" sh scripts/push-main.sh

setup test lint fmt clean:
	@set -e; for s in $(STACKS); do \
		echo "==> $$s: $@"; \
		$(MAKE) -C $$s $@; \
	done
