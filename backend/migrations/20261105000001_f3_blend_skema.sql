-- F3 schema, part 1: blend configuration, blend parent/child, gate trail (SHAA-354).
-- Source: delta data contract F3 v0.3.2 (SHAA-314), ADR-0007 rev 4 (docs/adr/0007-lanjutan-f3.md).
-- Purely additive over F0/F1: new tables, nullable columns with constant defaults, new constraints (added NOT VALID and
-- validated on request so the check scan holds no ACCESS EXCLUSIVE lock). No F0/F1 constraint is relaxed, no F0 migration
-- is edited, no F0 row is rewritten (log tables stay append-only, R-03). `blend_id IS NULL` means an ordinary F0 request.
-- Runs as poolrouter_owner after the F1 migrations (SHAA-333). Rollback: backend/db/rollback/20261105000001_*.down.sql.
-- Rules that need the process environment or cross-row history stay in the application / pipeline checks (see
-- docs/db/skema-f3.md section 5): the gate flag (GB-01, GB-02), G-3 (GB-10), DQ-61 (a) recomputation, DQ-63 (a)(b)(d)(g)(h).

-- ---------------------------------------------------------------- provider marker (2.2, GB-03)
-- Not part of provider_config_versi (it does not affect template rendering and does not bump provider_config_version).
ALTER TABLE provider ADD COLUMN butuh_dasar_hukum_blend boolean NOT NULL DEFAULT false;

-- ---------------------------------------------------------------- audit_konfigurasi: migration actor, gate evidence, approval reference
-- Additive. pengguna_id becomes nullable only for the system actor 'migrasi' (GB-03: the initial marker values are the first audit entries).
ALTER TABLE audit_konfigurasi ALTER COLUMN pengguna_id DROP NOT NULL;
ALTER TABLE audit_konfigurasi
  ADD COLUMN aktor text NOT NULL DEFAULT 'pengguna' CONSTRAINT chk_audit_konfig_aktor CHECK (aktor IN ('pengguna', 'migrasi')),
  ADD COLUMN gerbang text CONSTRAINT chk_audit_konfig_gerbang CHECK (gerbang IN ('diizinkan', 'ditolak_gerbang')),   -- GB-02 evidence
  ADD COLUMN blend_terdampak jsonb CONSTRAINT chk_audit_konfig_blend_terdampak CHECK (jsonb_typeof(blend_terdampak) = 'array'),
  ADD COLUMN referensi_persetujuan_nomor varchar(128),
  ADD COLUMN referensi_persetujuan_tanggal date;
ALTER TABLE audit_konfigurasi
  ADD CONSTRAINT chk_audit_konfig_aktor_pengguna CHECK ((aktor = 'pengguna') = (pengguna_id IS NOT NULL)),
  -- a gate refusal is recorded as aksi 'ditolak' and nothing else is (GB-02)
  ADD CONSTRAINT chk_audit_konfig_gerbang_ditolak CHECK (gerbang IS NULL OR ((gerbang = 'ditolak_gerbang') = (aksi = 'ditolak'))),
  ADD CONSTRAINT chk_audit_konfig_gerbang_entitas CHECK (gerbang IS NULL OR entitas_jenis IN ('endpoint_blend', 'provider_penanda_blend')),
  -- DQ-63 (f), GB-03: marker true -> false needs the structured approval reference (the reason is already NOT NULL / non-blank)
  ADD CONSTRAINT chk_audit_penanda_turun CHECK (
    NOT (entitas_jenis = 'provider_penanda_blend' AND aksi = 'ubah'
         AND nilai_lama ->> 'butuh_dasar_hukum_blend' = 'true' AND nilai_baru ->> 'butuh_dasar_hukum_blend' = 'false')
    OR (referensi_persetujuan_nomor IS NOT NULL AND length(btrim(referensi_persetujuan_nomor)) > 0
        AND referensi_persetujuan_tanggal IS NOT NULL)),
  -- DQ-63 (e), GB-02 point 2: a successful false -> true change while blends are affected is not allowed
  ADD CONSTRAINT chk_audit_penanda_naik CHECK (
    NOT (entitas_jenis = 'provider_penanda_blend' AND aksi = 'ubah'
         AND nilai_lama ->> 'butuh_dasar_hukum_blend' = 'false' AND nilai_baru ->> 'butuh_dasar_hukum_blend' = 'true')
    OR coalesce(jsonb_array_length(blend_terdampak), 0) = 0);
ALTER TABLE audit_konfigurasi DROP CONSTRAINT audit_konfigurasi_entitas_jenis_check;
ALTER TABLE audit_konfigurasi ADD CONSTRAINT chk_audit_konfig_entitas_jenis CHECK (entitas_jenis IN (
  'client', 'kredensial', 'downstream_endpoint', 'downstream_pool', 'upstream_pool', 'provider',
  'client_webhook', 'webhook_domain_allowlist', 'provider_config', 'endpoint_config',
  'endpoint_metadata_field', 'konfigurasi_platform', 'admin_user',
  'endpoint_blend', 'provider_penanda_blend'));

-- Initial marker values (GB-03): one audit entry per provider, actor 'migrasi'.
-- G-3 / GB-10 invariant: a provider with wajib_log_nik = true (Dukcapil) carries the marker true. Mock and default
-- providers stay false. This is the fail-closed reading of the "explicit list" in GB-03; the platform admin confirms
-- or corrects the list in the PR review before the production run (docs/db/skema-f3.md section 4).
UPDATE provider SET butuh_dasar_hukum_blend = true WHERE wajib_log_nik;
INSERT INTO audit_konfigurasi (audit_konfigurasi_id, terjadi_at, pengguna_id, aktor, entitas_jenis, entitas_id, aksi,
                               nilai_lama, nilai_baru, alasan)
SELECT gen_random_uuid(), now(), NULL, 'migrasi', 'provider_penanda_blend', p.provider_id, 'buat',
       NULL, jsonb_build_object('butuh_dasar_hukum_blend', p.butuh_dasar_hukum_blend),
       'migrasi F3: nilai awal penanda butuh_dasar_hukum_blend (GB-03, G-3)'
  FROM provider p;

-- ---------------------------------------------------------------- audit_akses.target_blend_id (2.6)
ALTER TABLE audit_akses ADD COLUMN target_blend_id uuid;
ALTER TABLE audit_akses ADD CONSTRAINT chk_audit_target_blend CHECK (target_blend_id IS NULL OR aksi IN ('lihat_payload', 'lihat_log'));
CREATE INDEX audit_akses_target_blend_idx ON audit_akses (target_blend_id, terjadi_at) WHERE target_blend_id IS NOT NULL;

-- ---------------------------------------------------------------- blend configuration (2.1, BC-01)
-- endpoint_blend holds identity and on/off state; every content change is a new immutable snapshot in
-- endpoint_blend_versi + endpoint_blend_komponen (BC-01, pattern V-03). The contract lists the content attributes on
-- "endpoint_blend"; physically they live on the version row so a snapshot can be referenced and never changes.
CREATE TABLE endpoint_blend (
  endpoint_blend_id uuid PRIMARY KEY,
  nama varchar(64) NOT NULL UNIQUE CHECK (nama ~ '^[a-z][a-z0-9_]*$'),                    -- path segment /v1/blend/{endpoint_blend}
  status text NOT NULL DEFAULT 'nonaktif' CHECK (status IN ('aktif', 'nonaktif')),
  config_versi_aktif integer CHECK (config_versi_aktif >= 1),
  dibuat_at timestamptz NOT NULL DEFAULT now(),
  diubah_at timestamptz NOT NULL DEFAULT now(),
  CONSTRAINT chk_blend_aktif_punya_versi CHECK (status = 'nonaktif' OR config_versi_aktif IS NOT NULL)
);

CREATE TABLE endpoint_blend_versi (
  endpoint_blend_id uuid NOT NULL REFERENCES endpoint_blend (endpoint_blend_id),
  config_version integer NOT NULL CHECK (config_version >= 1),
  config_schema_version integer NOT NULL CHECK (config_schema_version = 1),                -- F3 = 1
  mode_diizinkan text NOT NULL CHECK (mode_diizinkan IN ('sync', 'async', 'keduanya')),
  tata_letak text NOT NULL DEFAULT 'per_komponen' CHECK (tata_letak IN ('per_komponen', 'gabung_field')),
  aturan_konflik jsonb CHECK (jsonb_typeof(aturan_konflik) = 'object'),                    -- 'prioritas' / 'harus_sama' closed set: admin API (D3 butir 3)
  batas_waktu_detik integer CHECK (batas_waktu_detik > 0),
  dibuat_at timestamptz NOT NULL DEFAULT now(),
  dibuat_oleh uuid NOT NULL REFERENCES admin_user (pengguna_id),
  PRIMARY KEY (endpoint_blend_id, config_version),
  -- batas_waktu_detik: required when async is possible, NULL when sync only (admin API answers 422)
  CONSTRAINT chk_blend_batas_waktu CHECK ((mode_diizinkan = 'sync') = (batas_waktu_detik IS NULL)),
  CONSTRAINT chk_blend_konflik_tata_letak CHECK (aturan_konflik IS NULL OR tata_letak = 'gabung_field')
);
ALTER TABLE endpoint_blend
  ADD CONSTRAINT fk_endpoint_blend_versi_aktif FOREIGN KEY (endpoint_blend_id, config_versi_aktif)
    REFERENCES endpoint_blend_versi (endpoint_blend_id, config_version) DEFERRABLE INITIALLY DEFERRED;

CREATE TABLE endpoint_blend_komponen (
  endpoint_blend_id uuid NOT NULL,
  config_version integer NOT NULL,
  nama_komponen varchar(64) NOT NULL CHECK (nama_komponen ~ '^[a-z][a-z0-9_]*$'),
  downstream_endpoint_id uuid NOT NULL REFERENCES downstream_endpoint (downstream_endpoint_id),
  prioritas integer NOT NULL CHECK (prioritas >= 1),
  peta_input jsonb NOT NULL CHECK (jsonb_typeof(peta_input) = 'object'),
  PRIMARY KEY (endpoint_blend_id, config_version, nama_komponen),
  FOREIGN KEY (endpoint_blend_id, config_version) REFERENCES endpoint_blend_versi (endpoint_blend_id, config_version),
  UNIQUE (endpoint_blend_id, config_version, downstream_endpoint_id),                     -- the same endpoint never twice (BC-02, BL-03 b)
  UNIQUE (endpoint_blend_id, config_version, prioritas)                                   -- tie breaker / winner of the 'prioritas' rule
);
CREATE INDEX endpoint_blend_komponen_endpoint_idx ON endpoint_blend_komponen (downstream_endpoint_id);   -- K-20 / G-4 "affected blends" lookups

-- A new blend is always created 'nonaktif'; activation goes through the gate in the admin API (GB-02 point 1).
CREATE FUNCTION tg_endpoint_blend_baru_nonaktif() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  IF NEW.status <> 'nonaktif' THEN
    RAISE EXCEPTION 'endpoint_blend baru harus dibuat nonaktif; aktivasi lewat Admin API dengan gerbang F3-4 (GB-02)'
      USING ERRCODE = 'check_violation';
  END IF;
  RETURN NEW;
END $$;
CREATE TRIGGER endpoint_blend_baru_nonaktif BEFORE INSERT ON endpoint_blend
  FOR EACH ROW EXECUTE FUNCTION tg_endpoint_blend_baru_nonaktif();

-- BC-01: a snapshot never changes (V-03 pattern, tg_immutable from F0). A snapshot has at least two components
-- (ADR-0007 D1 butir 1), checked when the transaction commits because the components are inserted after the version row.
CREATE TRIGGER endpoint_blend_versi_immutable BEFORE UPDATE OR DELETE ON endpoint_blend_versi
  FOR EACH ROW EXECUTE FUNCTION tg_immutable();
CREATE TRIGGER endpoint_blend_komponen_immutable BEFORE UPDATE OR DELETE ON endpoint_blend_komponen
  FOR EACH ROW EXECUTE FUNCTION tg_immutable();
CREATE FUNCTION tg_blend_versi_min_dua_komponen() RETURNS trigger LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
BEGIN
  IF (SELECT count(*) FROM endpoint_blend_komponen k
       WHERE k.endpoint_blend_id = NEW.endpoint_blend_id AND k.config_version = NEW.config_version) < 2 THEN
    RAISE EXCEPTION 'endpoint_blend % versi % punya kurang dari 2 komponen (ADR-0007 D1 butir 1)', NEW.endpoint_blend_id, NEW.config_version
      USING ERRCODE = 'integrity_constraint_violation';
  END IF;
  RETURN NULL;
END $$;
CREATE CONSTRAINT TRIGGER endpoint_blend_versi_min_dua_komponen AFTER INSERT ON endpoint_blend_versi
  DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION tg_blend_versi_min_dua_komponen();

-- ---------------------------------------------------------------- blend (2.3)
-- Pure function used by the ringkasan_komponen CHECK (DQ-62 f): an array of objects with exactly the allowed keys
-- (nama_komponen, request_id, status_api, failure_reason; the first three required) and no 16-digit run anywhere.
CREATE FUNCTION fn_ringkasan_komponen_valid(p jsonb) RETURNS boolean LANGUAGE sql IMMUTABLE PARALLEL SAFE AS $$
  SELECT CASE WHEN jsonb_typeof(p) = 'array' THEN
    NOT EXISTS (
      SELECT 1 FROM jsonb_array_elements(p) e
       WHERE jsonb_typeof(e) <> 'object'
          OR NOT (e ?& ARRAY['nama_komponen', 'request_id', 'status_api'])
          OR EXISTS (SELECT 1 FROM jsonb_object_keys(e) k WHERE k <> ALL (ARRAY['nama_komponen', 'request_id', 'status_api', 'failure_reason'])))
    AND p::text !~ '(^|[^0-9])[0-9]{16}([^0-9]|$)'
  ELSE false END
$$;

CREATE TABLE blend (
  blend_id uuid PRIMARY KEY,                                  -- UUIDv7 created at intake; the client's queue_id for a blend
  client_id uuid NOT NULL REFERENCES client (client_id),
  credential_id uuid NOT NULL REFERENCES client_kredensial (credential_id),
  endpoint_blend_id uuid NOT NULL,
  endpoint_blend_config_version integer NOT NULL CHECK (endpoint_blend_config_version >= 1),
  mode text NOT NULL CHECK (mode IN ('sync', 'async')),
  status text NOT NULL CHECK (status IN ('berjalan', 'done', 'failed', 'expired', 'cancelled', 'rejected')),
  reject_reason text CHECK (reject_reason IN ('validasi_gagal', 'metadata_tidak_valid', 'kuota_habis', 'porsi_sync_habis',
                                              'objek_nonaktif', 'gerbang_blend_tertutup')),
  reject_http_status integer CHECK (reject_http_status IN (400, 403, 422, 429, 503)),
  failure_reason text CHECK (failure_reason IN ('retry_habis', 'provider_error_permanen', 'timeout_sync', 'objek_nonaktif', 'fungsi_gagal')),
  anak_penentu_request_id uuid,
  cancel_reason text CHECK (cancel_reason IN ('dibatalkan_client', 'client_nonaktif', 'kredensial_nonaktif', 'endpoint_nonaktif', 'pool_nonaktif', 'provider_nonaktif')),
  cancelled_by text,
  gerbang_dasar_hukum text NOT NULL CHECK (gerbang_dasar_hukum IN ('tidak_berlaku', 'diizinkan', 'ditolak')),
  ringkasan_komponen jsonb,
  idempotency_key varchar(128) CHECK (idempotency_key ~ '^[\x21-\x7e]+$'),
  idempotency_fingerprint varchar(64) CHECK (idempotency_fingerprint ~ '^[0-9a-f]{64}$'),
  output_payload jsonb,                                       -- Rahasia (section 1)
  payload_purged_at timestamptz,
  ttl_polling_detik integer CHECK (ttl_polling_detik > 0),
  batas_waktu_at timestamptz NOT NULL,
  requested_at timestamptz NOT NULL,
  completed_at timestamptz,
  webhook_status text CHECK (webhook_status IN ('menunggu', 'terkirim', 'gagal', 'tidak_dikirim')),
  webhook_attempt_count integer NOT NULL DEFAULT 0 CHECK (webhook_attempt_count >= 0),
  webhook_last_at timestamptz,
  webhook_last_http_status integer,
  webhook_last_error varchar(200),
  webhook_not_before timestamptz,                             -- F1 WR-01 retry column, same meaning as request
  source_ip varchar(45),
  FOREIGN KEY (endpoint_blend_id, endpoint_blend_config_version) REFERENCES endpoint_blend_versi (endpoint_blend_id, config_version),
  -- deferred: the purge deletes the child before the parent in one transaction (BP-07)
  FOREIGN KEY (anak_penentu_request_id) REFERENCES request (request_id) DEFERRABLE INITIALLY DEFERRED,

  -- reject / failure / cancel pairing (DQ-62)
  CONSTRAINT chk_blend_reject CHECK (
    (status = 'rejected') = (reject_reason IS NOT NULL) AND (status = 'rejected') = (reject_http_status IS NOT NULL)),
  CONSTRAINT chk_blend_reject_http CHECK (reject_reason IS NULL OR (reject_reason, reject_http_status) IN (
    ('validasi_gagal', 400), ('validasi_gagal', 422), ('metadata_tidak_valid', 400), ('metadata_tidak_valid', 422),
    ('kuota_habis', 429), ('porsi_sync_habis', 429), ('objek_nonaktif', 403), ('objek_nonaktif', 503),
    ('gerbang_blend_tertutup', 403))),
  CONSTRAINT chk_blend_failure CHECK (
    (status = 'failed') = (failure_reason IS NOT NULL) AND (status = 'failed') = (anak_penentu_request_id IS NOT NULL)),
  CONSTRAINT chk_blend_cancel CHECK (
    (status = 'cancelled') = (cancel_reason IS NOT NULL) AND (status = 'cancelled') = (cancelled_by IS NOT NULL)),
  CONSTRAINT chk_blend_cancelled_by CHECK (
    cancelled_by IS NULL OR (
      CASE WHEN cancel_reason = 'dibatalkan_client' THEN cancelled_by = 'client'
           ELSE cancelled_by = 'sistem' OR cancelled_by ~ '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$' END)),
  -- gate evidence (GB-05, DQ-63 c): the rejection code, the evidence value and the 403 travel together
  CONSTRAINT chk_blend_gerbang CHECK (
    (reject_reason = 'gerbang_blend_tertutup') = (gerbang_dasar_hukum = 'ditolak')),
  -- terminal states
  CONSTRAINT chk_blend_completed CHECK ((status <> 'berjalan') = (completed_at IS NOT NULL)),
  CONSTRAINT chk_blend_batas CHECK (batas_waktu_at > requested_at),
  -- output only for done, and it may be purged afterwards (BP-01, BP-04, DQ-61 e)
  CONSTRAINT chk_blend_output CHECK (
    CASE WHEN status = 'done' THEN output_payload IS NOT NULL OR payload_purged_at IS NOT NULL
         ELSE output_payload IS NULL AND payload_purged_at IS NULL END),
  CONSTRAINT chk_blend_ringkasan CHECK (
    CASE WHEN status IN ('berjalan', 'rejected') THEN ringkasan_komponen IS NULL
         ELSE ringkasan_komponen IS NOT NULL AND fn_ringkasan_komponen_valid(ringkasan_komponen) END),
  -- the polling window is frozen at intake for accepted async blends. A rejected blend is never polled (no queue_id) and
  -- its pool TTL may not even have been read, so it carries none (Klaus, SHAA-354: async non-rejected NOT NULL, rest NULL).
  CONSTRAINT chk_blend_ttl_polling CHECK (
    CASE WHEN mode = 'async' AND status <> 'rejected' THEN ttl_polling_detik IS NOT NULL
         ELSE ttl_polling_detik IS NULL END),
  -- idempotency: async only, never on a rejected blend (BL-07)
  CONSTRAINT chk_blend_idem CHECK (
    (idempotency_key IS NULL) = (idempotency_fingerprint IS NULL)
    AND (idempotency_key IS NULL OR (mode = 'async' AND status <> 'rejected'))),
  -- webhook: one per terminal async blend except rejected; rejected and sync blends are 'tidak_dikirim'; running has none
  CONSTRAINT chk_blend_webhook CHECK (
    CASE WHEN status = 'berjalan' THEN webhook_status IS NULL
         WHEN status = 'rejected' OR mode = 'sync' THEN webhook_status = 'tidak_dikirim'
         ELSE webhook_status IS NOT NULL END),
  CONSTRAINT chk_blend_webhook_not_before CHECK (webhook_not_before IS NULL OR coalesce(webhook_status = 'menunggu', false))
);
CREATE UNIQUE INDEX blend_idempotency_uq ON blend (client_id, idempotency_key) WHERE idempotency_key IS NOT NULL;
CREATE INDEX blend_client_requested_idx ON blend (client_id, requested_at);
CREATE INDEX blend_endpoint_requested_idx ON blend (endpoint_blend_id, requested_at);
-- job penggabung and the sweeper for blends that ran past their deadline (DQ-61 f)
CREATE INDEX blend_berjalan_idx ON blend (batas_waktu_at) WHERE status = 'berjalan';
-- webhook outbox and retry (WR-01)
CREATE INDEX blend_webhook_idx ON blend (completed_at) WHERE webhook_status = 'menunggu';
CREATE INDEX blend_webhook_retry_idx ON blend (webhook_not_before) WHERE webhook_status = 'menunggu' AND webhook_not_before IS NOT NULL;
-- retention (8.4), purge of output_payload (BP-01, DQ-64)
CREATE INDEX blend_completed_idx ON blend (completed_at) WHERE completed_at IS NOT NULL;
CREATE INDEX blend_payload_idx ON blend (completed_at) WHERE output_payload IS NOT NULL;

-- Allowed transitions and frozen columns (T-05, I-1). Compare-and-set itself is the application's job.
CREATE FUNCTION tg_blend_transisi() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  IF TG_OP = 'INSERT' THEN
    IF NEW.status NOT IN ('rejected', 'berjalan') THEN
      RAISE EXCEPTION 'status awal blend tidak sah: %', NEW.status USING ERRCODE = 'check_violation';
    END IF;
    RETURN NEW;
  END IF;
  IF NEW.status IS DISTINCT FROM OLD.status
     AND NOT (OLD.status = 'berjalan' AND NEW.status IN ('done', 'failed', 'expired', 'cancelled')) THEN
    RAISE EXCEPTION 'transisi status blend tidak sah: % -> %', OLD.status, NEW.status USING ERRCODE = 'check_violation';
  END IF;
  IF NEW.blend_id <> OLD.blend_id OR NEW.client_id <> OLD.client_id OR NEW.credential_id <> OLD.credential_id
     OR NEW.endpoint_blend_id <> OLD.endpoint_blend_id OR NEW.endpoint_blend_config_version <> OLD.endpoint_blend_config_version
     OR NEW.mode <> OLD.mode OR NEW.requested_at <> OLD.requested_at OR NEW.batas_waktu_at <> OLD.batas_waktu_at
     OR NEW.gerbang_dasar_hukum <> OLD.gerbang_dasar_hukum OR NEW.ttl_polling_detik IS DISTINCT FROM OLD.ttl_polling_detik
     OR NEW.idempotency_key IS DISTINCT FROM OLD.idempotency_key
     OR NEW.idempotency_fingerprint IS DISTINCT FROM OLD.idempotency_fingerprint THEN
    RAISE EXCEPTION 'kolom identitas blend tidak boleh diubah' USING ERRCODE = 'check_violation';
  END IF;
  IF OLD.status <> 'berjalan' AND (
       NEW.completed_at IS DISTINCT FROM OLD.completed_at OR NEW.reject_reason IS DISTINCT FROM OLD.reject_reason
       OR NEW.reject_http_status IS DISTINCT FROM OLD.reject_http_status OR NEW.failure_reason IS DISTINCT FROM OLD.failure_reason
       OR NEW.anak_penentu_request_id IS DISTINCT FROM OLD.anak_penentu_request_id OR NEW.cancel_reason IS DISTINCT FROM OLD.cancel_reason
       OR NEW.cancelled_by IS DISTINCT FROM OLD.cancelled_by OR NEW.ringkasan_komponen IS DISTINCT FROM OLD.ringkasan_komponen
       OR (OLD.output_payload IS NOT NULL AND NEW.output_payload IS NOT NULL AND NEW.output_payload IS DISTINCT FROM OLD.output_payload)
       OR (OLD.output_payload IS NULL AND NEW.output_payload IS NOT NULL)) THEN
    RAISE EXCEPTION 'status terminal blend tidak berubah (T-05); hanya purge payload dan pembukuan webhook' USING ERRCODE = 'check_violation';
  END IF;
  RETURN NEW;
END $$;
CREATE TRIGGER blend_transisi BEFORE INSERT OR UPDATE ON blend
  FOR EACH ROW EXECUTE FUNCTION tg_blend_transisi();

-- ---------------------------------------------------------------- blend_status_history (2.4)
CREATE TABLE blend_status_history (
  blend_id uuid NOT NULL REFERENCES blend (blend_id) ON DELETE CASCADE,   -- cascade only through the BP-07 purge (guard below)
  seq integer NOT NULL CHECK (seq >= 1),
  status_dari text CHECK (status_dari IN ('berjalan', 'done', 'failed', 'expired', 'cancelled', 'rejected')),
  status_ke text NOT NULL CHECK (status_ke IN ('berjalan', 'done', 'failed', 'expired', 'cancelled', 'rejected')),
  alasan text,                                                -- reject_reason, cancel_reason or failure_reason
  aktor text NOT NULL CHECK (aktor IN ('webserver', 'controller', 'client')
    OR aktor ~ '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$'),
  objek_pemicu_id uuid,
  terjadi_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (blend_id, seq),
  CONSTRAINT chk_blend_seq_pertama CHECK ((seq = 1) = (status_dari IS NULL)),
  CONSTRAINT chk_blend_transisi_riwayat CHECK (
    CASE WHEN status_dari IS NULL THEN status_ke IN ('berjalan', 'rejected')
         ELSE status_dari = 'berjalan' AND status_ke IN ('done', 'failed', 'expired', 'cancelled') END),
  -- the triggering object is recorded exactly when K-20 closes the blend through a child (BL-05 butir 3 and 5)
  CONSTRAINT chk_blend_pemicu_k20 CHECK (
    ((status_ke = 'cancelled' AND coalesce(alasan IN ('client_nonaktif', 'kredensial_nonaktif', 'endpoint_nonaktif', 'pool_nonaktif', 'provider_nonaktif'), false))
     OR (status_ke = 'failed' AND alasan = 'objek_nonaktif')) = (objek_pemicu_id IS NOT NULL))
);
CREATE INDEX blend_status_history_pemicu_idx ON blend_status_history (objek_pemicu_id, terjadi_at) WHERE objek_pemicu_id IS NOT NULL;

-- Append-only like request_status_history, except that rows follow their blend when the purge deletes it (BP-07).
-- A DELETE is accepted only when it is the referential cascade of a blend delete (trigger depth > 1) or comes from the archive job.
CREATE FUNCTION tg_blend_append_only() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  IF TG_OP = 'DELETE' AND (pg_trigger_depth() > 1 OR current_user = 'pr_job_arsip') THEN
    RETURN OLD;
  END IF;
  RAISE EXCEPTION '% bersifat append-only', TG_TABLE_NAME USING ERRCODE = 'integrity_constraint_violation';
END $$;
CREATE TRIGGER blend_status_history_append_only BEFORE UPDATE OR DELETE ON blend_status_history
  FOR EACH ROW EXECUTE FUNCTION tg_blend_append_only();
CREATE TRIGGER blend_status_history_no_truncate BEFORE TRUNCATE ON blend_status_history
  FOR EACH STATEMENT EXECUTE FUNCTION tg_append_only();

-- T-04: every blend status has its history row in the same transaction (checked at commit), reason included.
CREATE FUNCTION tg_blend_punya_history() RETURNS trigger LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
BEGIN
  IF NOT EXISTS (SELECT 1 FROM blend_status_history h
                  WHERE h.blend_id = NEW.blend_id
                    AND CASE WHEN TG_OP = 'INSERT' THEN h.seq = 1 AND h.status_dari IS NULL
                             ELSE h.status_dari = OLD.status END
                    AND h.status_ke = NEW.status
                    AND h.alasan IS NOT DISTINCT FROM coalesce(NEW.reject_reason, NEW.cancel_reason, NEW.failure_reason)
                    AND h.terjadi_at >= transaction_timestamp()) THEN
    RAISE EXCEPTION 'blend % berstatus % tanpa baris blend_status_history yang cocok (T-04)', NEW.blend_id, NEW.status
      USING ERRCODE = 'integrity_constraint_violation';
  END IF;
  RETURN NULL;
END $$;
CREATE CONSTRAINT TRIGGER blend_history_wajib AFTER INSERT OR UPDATE OF status ON blend
  DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION tg_blend_punya_history();

-- ---------------------------------------------------------------- blend_webhook_log (2.5)
-- Same columns and rules as webhook_log with blend_id instead of request_id; a separate table because
-- webhook_log.request_id stays NOT NULL (the F0 constraint is not relaxed). webhook-id on the wire = blend_id.
CREATE TABLE blend_webhook_log (
  webhook_delivery_id uuid PRIMARY KEY,
  blend_id uuid NOT NULL REFERENCES blend (blend_id) ON DELETE CASCADE,
  client_id uuid NOT NULL REFERENCES client (client_id),
  attempt_no integer NOT NULL CHECK (attempt_no >= 1),
  status_api text NOT NULL CHECK (status_api IN ('queued', 'processing', 'done', 'failed', 'expired', 'cancelled')),
  sent_at timestamptz NOT NULL,
  responded_at timestamptz,
  http_status integer,
  hasil text NOT NULL CHECK (hasil IN ('terkirim', 'gagal_http', 'gagal_jaringan', 'ditolak_allowlist', 'gagal_internal')),
  error_class varchar(64),
  duration_ms integer CHECK (duration_ms >= 0),
  secret_versi integer CHECK (secret_versi >= 1),
  UNIQUE (blend_id, attempt_no),
  CONSTRAINT chk_blend_secret_versi CHECK ((hasil = 'ditolak_allowlist') = (secret_versi IS NULL) OR hasil = 'gagal_internal'),
  CONSTRAINT chk_blend_allowlist_http CHECK (hasil <> 'ditolak_allowlist' OR http_status IS NULL)
);
CREATE INDEX blend_webhook_log_client_idx ON blend_webhook_log (client_id, sent_at);
-- immutable like webhook_log (UPDATE refused); DELETE only as the BP-07 cascade
CREATE TRIGGER blend_webhook_log_append_only BEFORE UPDATE OR DELETE ON blend_webhook_log
  FOR EACH ROW EXECUTE FUNCTION tg_blend_append_only();
CREATE TRIGGER blend_webhook_log_no_truncate BEFORE TRUNCATE ON blend_webhook_log
  FOR EACH STATEMENT EXECUTE FUNCTION tg_append_only();

-- ---------------------------------------------------------------- request.blend_id and the child rules (2.6, section 3)
ALTER TABLE request ADD COLUMN blend_id uuid;
ALTER TABLE request ADD CONSTRAINT fk_request_blend FOREIGN KEY (blend_id) REFERENCES blend (blend_id) NOT VALID;
ALTER TABLE request VALIDATE CONSTRAINT fk_request_blend;
ALTER TABLE request
  -- BL-03 c, BL-03 g (a child is never a rejected row), N-1 (a child never owns a webhook)
  ADD CONSTRAINT chk_request_anak_blend CHECK (
    blend_id IS NULL OR (
      idempotency_key IS NULL AND idempotency_fingerprint IS NULL AND status <> 'rejected'
      AND (webhook_status IS NULL OR (webhook_status = 'tidak_dikirim' AND webhook_attempt_count = 0)))) NOT VALID;
ALTER TABLE request VALIDATE CONSTRAINT chk_request_anak_blend;
-- BL-03 b / g, DQ-60 (g): one child per component of the parent; also the lookup index for the merge job
CREATE UNIQUE INDEX request_blend_komponen_uq ON request (blend_id, downstream_endpoint_id) WHERE blend_id IS NOT NULL;

-- BL-01: written once at insert, never changed (no NULL -> value, no replacement), and no backfill.
CREATE FUNCTION tg_request_f3_guard() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  IF NEW.blend_id IS DISTINCT FROM OLD.blend_id THEN
    RAISE EXCEPTION 'request.blend_id ditulis sekali saat insert dan tidak berubah (BL-01)' USING ERRCODE = 'check_violation';
  END IF;
  RETURN NEW;
END $$;
CREATE TRIGGER request_f3_guard BEFORE UPDATE OF blend_id ON request
  FOR EACH ROW EXECUTE FUNCTION tg_request_f3_guard();

-- BL-03 a, b, d, e at insert: the parent exists and is running (so a rejected blend never has children), client,
-- credential and mode match, the endpoint is a component of the parent's configuration version, the async queue
-- deadline equals the parent's, and received_at is within 5 s of the intake (one transaction, clock skew tolerated).
CREATE FUNCTION tg_request_anak_blend() RETURNS trigger LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
DECLARE
  p blend%ROWTYPE;
BEGIN
  SELECT * INTO p FROM blend WHERE blend_id = NEW.blend_id;
  IF NOT FOUND THEN
    RAISE EXCEPTION 'anak blend tanpa induk: %', NEW.blend_id USING ERRCODE = 'foreign_key_violation';
  END IF;
  IF p.status <> 'berjalan' THEN
    RAISE EXCEPTION 'blend % berstatus %, tidak boleh punya anak baru (BL-03 g)', p.blend_id, p.status USING ERRCODE = 'check_violation';
  END IF;
  IF NEW.client_id <> p.client_id OR NEW.credential_id <> p.credential_id OR NEW.mode <> p.mode THEN
    RAISE EXCEPTION 'client, credential atau mode anak berbeda dari induk blend % (BL-03 a)', p.blend_id USING ERRCODE = 'check_violation';
  END IF;
  IF NOT EXISTS (SELECT 1 FROM endpoint_blend_komponen k
                  WHERE k.endpoint_blend_id = p.endpoint_blend_id AND k.config_version = p.endpoint_blend_config_version
                    AND k.downstream_endpoint_id = NEW.downstream_endpoint_id) THEN
    RAISE EXCEPTION 'endpoint anak bukan komponen konfigurasi blend % (BL-03 b)', p.blend_id USING ERRCODE = 'check_violation';
  END IF;
  IF NEW.mode = 'async' AND NEW.ttl_expires_at IS DISTINCT FROM p.batas_waktu_at THEN
    RAISE EXCEPTION 'ttl_expires_at anak harus sama dengan batas_waktu_at blend (BL-03 d)' USING ERRCODE = 'check_violation';
  END IF;
  IF abs(extract(epoch FROM (NEW.received_at - p.requested_at))) > 5 THEN
    RAISE EXCEPTION 'received_at anak berselisih lebih dari 5 detik dari requested_at blend (BL-03 e)' USING ERRCODE = 'check_violation';
  END IF;
  RETURN NEW;
END $$;
CREATE TRIGGER request_anak_blend BEFORE INSERT ON request
  FOR EACH ROW WHEN (NEW.blend_id IS NOT NULL) EXECUTE FUNCTION tg_request_anak_blend();

-- BL-02, BL-03 b, DQ-60 (b): an accepted blend has exactly one child per component, checked at commit
-- (zero children is a failed atomic creation, so the blend row cannot outlive it).
CREATE FUNCTION tg_blend_anak_lengkap() RETURNS trigger LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
DECLARE
  v_komponen integer;
  v_anak integer;
  v_cocok integer;
BEGIN
  SELECT count(*) INTO v_komponen FROM endpoint_blend_komponen k
   WHERE k.endpoint_blend_id = NEW.endpoint_blend_id AND k.config_version = NEW.endpoint_blend_config_version;
  SELECT count(*), count(*) FILTER (WHERE EXISTS (
           SELECT 1 FROM endpoint_blend_komponen k
            WHERE k.endpoint_blend_id = NEW.endpoint_blend_id AND k.config_version = NEW.endpoint_blend_config_version
              AND k.downstream_endpoint_id = r.downstream_endpoint_id))
    INTO v_anak, v_cocok FROM request r WHERE r.blend_id = NEW.blend_id;
  IF v_anak <> v_komponen OR v_cocok <> v_komponen THEN
    RAISE EXCEPTION 'blend % punya % anak (% cocok), konfigurasi punya % komponen (BL-03 b)', NEW.blend_id, v_anak, v_cocok, v_komponen
      USING ERRCODE = 'integrity_constraint_violation';
  END IF;
  RETURN NULL;
END $$;
CREATE CONSTRAINT TRIGGER blend_anak_lengkap AFTER INSERT ON blend
  DEFERRABLE INITIALLY DEFERRED FOR EACH ROW WHEN (NEW.status = 'berjalan') EXECUTE FUNCTION tg_blend_anak_lengkap();

-- DQ-61 (b)-(e) at the moment the blend closes (BL-05, BL-06): the parts that can be decided from the children's
-- current rows. The full recomputation of BL-05 from the history at completed_at (DQ-61 a) stays a pipeline check.
CREATE FUNCTION tg_blend_terminal_konsisten() RETURNS trigger LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
DECLARE
  v_anak integer; v_done integer; v_cancelled integer; v_expired integer; v_failed integer; v_percobaan bigint; v_log bigint;
BEGIN
  SELECT count(*), count(*) FILTER (WHERE status = 'done'), count(*) FILTER (WHERE status = 'cancelled'),
         count(*) FILTER (WHERE status = 'expired'), count(*) FILTER (WHERE status = 'failed'), coalesce(sum(attempt_count), 0)
    INTO v_anak, v_done, v_cancelled, v_expired, v_failed, v_percobaan
    FROM request WHERE blend_id = NEW.blend_id;
  IF NEW.status = 'done' AND (v_anak = 0 OR v_done <> v_anak) THEN
    RAISE EXCEPTION 'blend done harus punya semua anak done (BL-05 butir 1)' USING ERRCODE = 'integrity_constraint_violation';
  END IF;
  IF NEW.status = 'cancelled' AND (v_percobaan > 0 OR v_cancelled = 0) THEN
    RAISE EXCEPTION 'blend cancelled butuh semua anak tanpa percobaan dan minimal satu anak cancelled (BL-05 butir 3, BL-06, DQ-61 c)'
      USING ERRCODE = 'integrity_constraint_violation';
  END IF;
  IF NEW.status = 'expired' THEN
    SELECT count(*) INTO v_log FROM transaction_log l JOIN request r ON r.request_id = l.request_id WHERE r.blend_id = NEW.blend_id;
    IF v_percobaan > 0 OR v_log > 0 OR v_cancelled > 0 OR v_done > 0 OR v_failed > 0 OR v_expired = 0 THEN
      RAISE EXCEPTION 'blend expired butuh anak tanpa percobaan dan tanpa log, tanpa anak done, failed atau cancelled, minimal satu expired (BL-05 butir 4, DQ-61 d)'
        USING ERRCODE = 'integrity_constraint_violation';
    END IF;
  END IF;
  IF NEW.status = 'failed' THEN
    IF v_done = v_anak OR NOT EXISTS (SELECT 1 FROM request WHERE request_id = NEW.anak_penentu_request_id AND blend_id = NEW.blend_id) THEN
      RAISE EXCEPTION 'blend failed butuh minimal satu anak tidak sukses dan anak penentu milik blend ini (BL-05 butir 2 dan 5)'
        USING ERRCODE = 'integrity_constraint_violation';
    END IF;
  END IF;
  RETURN NULL;
END $$;
CREATE CONSTRAINT TRIGGER blend_terminal_konsisten AFTER UPDATE OF status ON blend
  DEFERRABLE INITIALLY DEFERRED FOR EACH ROW
  WHEN (OLD.status = 'berjalan' AND NEW.status IN ('done', 'failed', 'expired', 'cancelled'))
  EXECUTE FUNCTION tg_blend_terminal_konsisten();

-- ---------------------------------------------------------------- gate trail (GB-09, DQ-63 h, i)
-- One row per change of the pair (effective state, dasar_hukum_ref). Written only through fn_gerbang_blend_catat.
-- Kept in its own typed table rather than as jsonb in audit_konfigurasi (physical form is Engineering's call, delta 0);
-- same class (Terbatas), same retention as audit_akses (section 6).
CREATE TABLE audit_gerbang_blend (
  urutan bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,     -- monotonic order for compare-and-set (timestamps can tie)
  terjadi_at timestamptz NOT NULL DEFAULT clock_timestamp(),
  peristiwa text NOT NULL CHECK (peristiwa IN ('gerbang_blend_dibuka', 'gerbang_blend_ditutup')),
  nilai_flag varchar(64) NOT NULL,                            -- raw value of ${POOLROUTER_BLEND_DATA_KEPENDUDUKAN_DIIZINKAN}
  dasar_hukum_ref varchar(128),                               -- document id, NULL when closed
  sebab_tertutup text CHECK (sebab_tertutup IN ('flag_bukan_true', 'rujukan_kosong')),
  instance varchar(64) NOT NULL,
  CONSTRAINT chk_gerbang_dibuka CHECK (
    peristiwa <> 'gerbang_blend_dibuka'
    OR (nilai_flag = 'true' AND dasar_hukum_ref IS NOT NULL AND length(btrim(dasar_hukum_ref)) > 0 AND sebab_tertutup IS NULL)),
  CONSTRAINT chk_gerbang_ditutup CHECK (
    peristiwa <> 'gerbang_blend_ditutup'
    OR (dasar_hukum_ref IS NULL AND sebab_tertutup IS NOT NULL
        AND ((sebab_tertutup = 'rujukan_kosong') = (nilai_flag = 'true'))))
);
CREATE INDEX audit_gerbang_blend_terjadi_idx ON audit_gerbang_blend (terjadi_at);
CREATE TRIGGER audit_gerbang_blend_append_only BEFORE UPDATE OR DELETE ON audit_gerbang_blend
  FOR EACH ROW EXECUTE FUNCTION tg_append_only();
CREATE TRIGGER audit_gerbang_blend_no_truncate BEFORE TRUNCATE ON audit_gerbang_blend
  FOR EACH STATEMENT EXECUTE FUNCTION tg_append_only();

-- GB-09: the caller passes the effective state it computed at start (GB-01) and the raw flag value. The function
-- compares the pair with the last event under an advisory lock (compare-and-set, once per change not per instance)
-- and returns the event it wrote, or NULL when nothing changed. No event: still closed, whatever sebab_tertutup or raw flag did.
CREATE FUNCTION fn_gerbang_blend_catat(p_terbuka boolean, p_nilai_flag text, p_dasar_hukum_ref text, p_instance text)
RETURNS text LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
DECLARE
  v_ref text := nullif(btrim(coalesce(p_dasar_hukum_ref, '')), '');
  v_flag text := left(coalesce(p_nilai_flag, ''), 64);
  v_terakhir audit_gerbang_blend%ROWTYPE;
  v_terbuka_lama boolean;
  v_ref_lama text;
BEGIN
  IF p_terbuka AND (v_flag <> 'true' OR v_ref IS NULL) THEN
    RAISE EXCEPTION 'gerbang efektif terbuka hanya bila flag persis true dan rujukan tidak kosong (GB-01)' USING ERRCODE = 'check_violation';
  END IF;
  PERFORM pg_advisory_xact_lock(hashtextextended('poolrouter.audit_gerbang_blend', 0));
  SELECT * INTO v_terakhir FROM audit_gerbang_blend ORDER BY urutan DESC LIMIT 1;
  IF FOUND THEN
    v_terbuka_lama := v_terakhir.peristiwa = 'gerbang_blend_dibuka';
    v_ref_lama := v_terakhir.dasar_hukum_ref;
  ELSE
    v_terbuka_lama := false; v_ref_lama := NULL;           -- no event yet: (closed, empty)
  END IF;
  IF p_terbuka THEN
    IF v_terbuka_lama AND v_ref_lama IS NOT DISTINCT FROM v_ref THEN RETURN NULL; END IF;
    INSERT INTO audit_gerbang_blend (peristiwa, nilai_flag, dasar_hukum_ref, instance)
      VALUES ('gerbang_blend_dibuka', v_flag, v_ref, left(p_instance, 64));
    RETURN 'gerbang_blend_dibuka';
  END IF;
  IF NOT v_terbuka_lama THEN RETURN NULL; END IF;           -- still closed
  INSERT INTO audit_gerbang_blend (peristiwa, nilai_flag, sebab_tertutup, instance)
    VALUES ('gerbang_blend_ditutup', v_flag, CASE WHEN v_flag = 'true' THEN 'rujukan_kosong' ELSE 'flag_bukan_true' END, left(p_instance, 64));
  RETURN 'gerbang_blend_ditutup';
END $$;
REVOKE ALL ON FUNCTION fn_gerbang_blend_catat(boolean, text, text, text) FROM PUBLIC;

-- ---------------------------------------------------------------- retention (8.4, BP-07)
-- Deletes blend rows (history and webhook log follow by cascade) older than p_batas whose children are all gone.
-- A blend whose last child is still online (H-02 hold, or not yet purged) is skipped: request.blend_id is a foreign key
-- without cascade, so a blend can never disappear before its children. Returns the number of blends deleted.
CREATE FUNCTION fn_blend_hapus_online(p_batas timestamptz) RETURNS bigint
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
DECLARE
  n bigint;
BEGIN
  DELETE FROM blend b
   WHERE b.status <> 'berjalan' AND b.completed_at < p_batas
     AND NOT EXISTS (SELECT 1 FROM request r WHERE r.blend_id = b.blend_id);
  GET DIAGNOSTICS n = ROW_COUNT;
  RETURN n;
END $$;
REVOKE ALL ON FUNCTION fn_blend_hapus_online(timestamptz) FROM PUBLIC;
