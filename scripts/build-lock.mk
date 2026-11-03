# Kunci build berat bersama (SHAA-194, K-41). Dipakai backend/Makefile dan frontend/Makefile
# lewat `include ../scripts/build-lock.mk`. Jangan menulis ulang logika kunci di stack mana pun.
#
# Pemakaian di stack:
#   HEAVY = $(BUILD_LOCK_SH) run -- <perintah podman run dengan BUILD_LOCK_OPTS>   (RUNNER=podman)
#   HEAVY = $(RUN)                                                               (RUNNER=native, tanpa kunci)
# Target compile memakai $(HEAVY); setup, fmt, clean, dan db-test tidak.
#
# Dua hal wajib di setiap run kunci: nama tetap (BUILD_LOCK_NAME) dan label shaal.build=heavy.
# Keduanya dipakai script untuk mengenali kunci dan untuk membersihkan kunci basi.

BUILD_LOCK_DIR := $(dir $(lastword $(MAKEFILE_LIST)))
BUILD_LOCK_SH := sh $(BUILD_LOCK_DIR)build-lock.sh
BUILD_LOCK_NAME ?= shaal-build-heavy
BUILD_LOCK_OPTS = --name $(BUILD_LOCK_NAME) --label shaal.build=heavy

.PHONY: build-lock-status build-lock-clean

build-lock-status:
	@$(BUILD_LOCK_SH) status

# Hanya menghapus kunci basi (container berhenti). Kunci yang masih jalan tidak disentuh.
build-lock-clean:
	@$(BUILD_LOCK_SH) clean
