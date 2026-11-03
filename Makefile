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
PROJECT ?= poolrouter-$(ENV)
COMPOSE_DIR := deploy/compose
ENV_FILE ?= $(COMPOSE_DIR)/env/$(ENV).env
# EXTRA_COMPOSE_FILES: override tambahan, mis. label harness uji (deploy/compose/test/).
EXTRA_COMPOSE_FILES ?=
COMPOSE_FILES := -f $(COMPOSE_DIR)/compose.yaml -f $(COMPOSE_DIR)/compose.$(ENV).yaml $(addprefix -f ,$(EXTRA_COMPOSE_FILES))
COMPOSE_PROFILES ?= bundled-infra
# Provider P3 (Docker Compose v2 dari container). Variabel ini hanya dibaca oleh podman.
# Provider membaca file env dan direktori secret dari luar repo lewat mount read-only.
SECRETS_DIR := $(shell sed -n 's/^POOLROUTER_SECRETS_DIR=//p' "$(ENV_FILE)" 2>/dev/null | head -n 1)
EXTRA_MOUNTS := $(abspath $(dir $(ENV_FILE))):$(SECRETS_DIR)
COMPOSE_ENV := PODMAN_COMPOSE_PROVIDER=$(CURDIR)/$(COMPOSE_DIR)/bin/compose-provider POOLROUTER_EXTRA_MOUNTS=$(EXTRA_MOUNTS)
COMPOSE := $(COMPOSE_ENV) $(CONTAINER_ENGINE) compose -p $(PROJECT) $(COMPOSE_FILES) --env-file $(ENV_FILE)
LINT_ENV_FILES := $(COMPOSE_DIR)/env/dev.env.example
IMAGE_TAG ?= dev

.PHONY: check-env compose-up compose-down compose-ps compose-logs compose-config compose-lint image

check-env:
	@case "$(ENV)" in dev|staging|prod) ;; *) echo "ENV harus dev, staging, atau prod" >&2; exit 2 ;; esac
	@case "$(CONTAINER_ENGINE)" in podman|docker) ;; *) echo "CONTAINER_ENGINE harus podman atau docker" >&2; exit 2 ;; esac
	@test -f "$(ENV_FILE)" || { echo "ENV_FILE $(ENV_FILE) tidak ada (lihat docs/runbook/deploy.md)" >&2; exit 2; }

compose-config: check-env
	$(COMPOSE) --profile '*' config -q

compose-up: check-env
	$(COMPOSE) $(addprefix --profile ,$(COMPOSE_PROFILES)) up -d --wait

compose-down: check-env
	$(COMPOSE) $(addprefix --profile ,$(COMPOSE_PROFILES)) down

compose-ps: check-env
	$(COMPOSE) $(addprefix --profile ,$(COMPOSE_PROFILES)) ps

compose-logs: check-env
	$(COMPOSE) $(addprefix --profile ,$(COMPOSE_PROFILES)) logs --tail=200

# compose-lint: V1 (ADR-0001 Addendum A §5, §8). Skrip dan fixture di deploy/compose/lint/.
compose-lint:
	COMPOSE_CMD="$(COMPOSE_ENV) $(CONTAINER_ENGINE) compose" \
	COMPOSE_DIR="$(COMPOSE_DIR)" \
	sh $(COMPOSE_DIR)/lint/compose-lint.sh

# image: membangun image dari Containerfile dengan engine terpilih (Addendum §3.2).
# Compose tidak dipakai untuk build di staging dan produksi.
image:
	@test -f backend/Containerfile || { echo "backend/Containerfile belum ada (menunggu SHAA-175)" >&2; exit 2; }
	$(CONTAINER_ENGINE) build -f backend/Containerfile -t localhost/poolrouter-backend:$(IMAGE_TAG) backend
