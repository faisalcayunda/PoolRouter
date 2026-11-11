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
# JOBS diteruskan ke build-arg image backend. Default 1 cocok untuk VM Podman 2 CPU / 3,7 GiB (ARC-222). CI memberi JOBS=2 eksplisit.
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

.PHONY: check-env compose-up compose-down compose-ps compose-logs compose-config compose-migrate compose-seed compose-lint compose-test-dind image image-frontend image-postgres

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

# compose-seed: seed sintetis di compose dev (ARC-237). Jalankan setelah compose-migrate. Profil `cli`
# memakai image backend yang memuat backend-cli. Hanya ENV=dev; di luar itu backend-cli menolak juga.
compose-seed: check-env
	@test "$(ENV)" = dev || { echo "compose-seed hanya untuk ENV=dev" >&2; exit 2; }
	$(COMPOSE) $(PROFILE_ARGS) --profile cli run --rm backend-cli seed pengguna
	$(COMPOSE) $(PROFILE_ARGS) --profile cli run --rm backend-cli seed dukcapil

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

# compose-test-dind: bukti V3 di Docker Engine sementara (harness DinD, butuh ARC_ISSUE dan ENV_FILE).
compose-test-dind:
	ARC_ISSUE="$(ARC_ISSUE)" ENV_FILE="$(ENV_FILE)" PROFILES="$(EFF_PROFILES)" \
	sh $(COMPOSE_DIR)/test/dind.sh

# image: membangun image dari Containerfile dengan engine terpilih (Addendum §3.2).
# Compose tidak dipakai untuk build di staging dan produksi.
image:
	@test -f backend/Containerfile || { echo "backend/Containerfile belum ada (menunggu ARC-175)" >&2; exit 2; }
	$(CONTAINER_ENGINE) build -f backend/Containerfile --build-arg JOBS=$(JOBS) -t localhost/poolrouter-backend:$(IMAGE_TAG) backend

# image-frontend: image nginx + build statis frontend (ADR-0003 §3.9), konteks frontend/.
image-frontend:
	@test -f frontend/Containerfile || { echo "frontend/Containerfile tidak ada" >&2; exit 2; }
	$(CONTAINER_ENGINE) build -f frontend/Containerfile -t localhost/poolrouter-frontend:$(IMAGE_TAG) frontend

# image-postgres: image PostgreSQL 17 + pgBackRest (deploy/compose/backup/), konteks folder itu.
# ARC_ISSUE (mis. ARC-215) memberi label arctiva.issue pada image.
image-postgres:
	$(CONTAINER_ENGINE) build $(if $(ARC_ISSUE),--label arctiva.issue=$(ARC_ISSUE)) -f $(COMPOSE_DIR)/backup/Containerfile -t localhost/poolrouter-postgres:$(IMAGE_TAG) $(COMPOSE_DIR)/backup

# Repo gate (ADR-0003 section 5): `make setup test lint` from the repo root.
# Each stack keeps its own Makefile; these targets only delegate. frontend/ is
# included once it has a Makefile, so adding it needs no change here.
# Lihat backend/Makefile: resep dengan efek samping memakai $(SUBMAKE), bukan $(MAKE), supaya `make -n` aman (ARC-398).
SUBMAKE = $(MAKE)
STACKS := backend $(if $(wildcard frontend/Makefile),frontend)

.PHONY: help setup test lint fmt clean push-main test-dry-run test-label test-atribut test-image-cli

help:
	@echo "gate resmi: make setup test lint RUNNER=<podman|docker> FRONTEND_RUNNER=<podman|docker> CONTAINER_LABEL=arctiva.issue=ARC-<n>"
	@echo "runner: podman (Mac Operator) atau docker bila ada; pakai runner kontainer yang tersedia."
	@echo "targets: setup test lint fmt clean (gate repo, delegasi ke stack); test-dry-run (uji regresi make -n, tanpa Docker); test-image-cli (biner di image backend); compose-up compose-down compose-ps compose-logs compose-config compose-migrate compose-seed compose-lint compose-test-dind image image-frontend"
	@echo "Push: tidak ada push langsung ke main. Branch agent/<ID>-<slug> -> PR ke main -> merge Operator (CLAUDE.md)."

# push-main: sengaja menolak. Aturan repo tidak mengizinkan push langsung ke main, dan jalur push
# otomatis ke main tidak boleh dikembalikan. Pesan ini menjelaskan alur yang benar, bukan
# "No rule to make target" (AC-3, ARC-2).
push-main:
	@echo "make push-main tidak dipakai lagi: push langsung ke main dilarang aturan repo (CLAUDE.md)." >&2
	@echo "Alur yang benar:" >&2
	@echo "  1. git checkout -b agent/<ID>-<slug> main" >&2
	@echo "  2. make setup test lint RUNNER=<podman|docker> FRONTEND_RUNNER=<podman|docker> CONTAINER_LABEL=arctiva.issue=ARC-<n>" >&2
	@echo "  3. git push -u origin agent/<ID>-<slug> lalu buka PR ke main (gh pr create --base main)" >&2
	@echo "  4. merge hanya oleh Operator setelah gate hijau; agen tidak merge dan tidak push ke main." >&2
	@exit 2

# test-dry-run: bukti `make -n` tidak menyentuh engine kontainer (stub docker/podman di PATH, ARC-398).
test-dry-run:
	@sh scripts/test-dry-run.sh

# test-label: uji regresi kontrak label gate arctiva.issue=ARC-<n> (AC-2, ARC-2).
test-label:
	@sh scripts/test-label.sh

# test-atribut: uji regresi aturan repo (push-main hanya penjaga, CLAUDE.md, tanpa label Shaal).
test-atribut:
	@sh scripts/test-atribut.sh

# test-image-cli: image backend memuat webserver, controller, worker, dan backend-cli (ARC-236).
# Tanpa IMAGE hanya memeriksa Containerfile (statis). Dengan IMAGE (hasil `make image`, tag $(IMAGE_TAG)) juga
# menjalankan `backend-cli --help` di dalam image: make image test-image-cli IMAGE=localhost/poolrouter-backend:$(IMAGE_TAG)
test-image-cli:
	@IMAGE="$(IMAGE)" CONTAINER_ENGINE="$(CONTAINER_ENGINE)" CONTAINER_LABEL="$(CONTAINER_LABEL)" sh scripts/test-image-cli.sh

setup test lint fmt clean:
	@set -e; for s in $(STACKS); do \
		echo "==> $$s: $@"; \
		$(MAKE) -C $$s $@; \
	done
