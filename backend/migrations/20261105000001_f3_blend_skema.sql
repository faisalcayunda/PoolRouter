-- F3 schema, part 1: blending (blend parent + F0 children), blend configuration, provider legal-basis marker, gate trail (SHAA-354).
-- Source: delta data contract F3 v0.3.2 (SHAA-314 attachment) sections 2-6, 10; ADR-0007 rev 4 (docs/adr/0007-lanjutan-f3.md).
-- Purely additive over F0 + F1: new tables, nullable columns, new indexes and triggers. No F0 constraint is relaxed, no F0 column
-- is dropped or retyped, no F0/F1 migration is edited, no F0 row is rewritten (log tables stay append-only, R-03). The only
-- touched F0 objects: request (+blend_id), audit_akses (+target_blend_id), provider (+butuh_dasar_hukum_blend),
-- audit_konfigurasi (entitas_jenis domain extended by one value), fn_purge_p03 (narrowed, BP-05). Rollback: backend/db/rollback/.
-- Runs as poolrouter_owner, after the F1 migrations (SHAA-333). Physical names follow F0 (text + CHECK, timestamptz, jsonb).
-- Grants: 20261105000002_f3_grants.sql. Function EXECUTE is locked down by default since the F1 grants migration.

-- ---------------------------------------------------------------- 2.1 blend configuration (BC-01..BC-05)
-- endpoint_blend holds identity and status; every content change creates a new immutable endpoint_blend_versi with its
-- own endpoint_blend_komponen rows (BC-01 snapshot: components, peta_input, layout, conflict rules, time limit, mode).
-- The contract lists these attributes on endpoint_blend; the physical split follows endpoint_config_versi in F0 (V-03).
CREATE TABLE endpoint_blend (
  endpoint_blend_id uuid PRIMARY KEY,
  nama varchar(64) NOT NULL UNIQUE CHECK (nama ~ '^[a-z][a-z0-9_]*$'),   -- path segment /v1/blend/{endpoint_blend}
  status text NOT NULL DEFAULT 'nonaktif' CHECK (status IN ('aktif', 'nonaktif')),
  config_version integer NOT NULL DEFAULT 1 CHECK (config_version >= 1),   -- current version; FK below once the version table exists
  dibuat_at timestamptz NOT NULL DEFAULT now(),
  diubah_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE endpoint_blend_versi (
  endpoint_blend_id uuid NOT NULL REFERENCES endpoint_blend (endpoint_blend_id),
  config_version integer NOT NULL CHECK (config_version >= 1),
  config_schema_version integer NOT NULL CHECK (config_schema_version = 1),
  mode_diizinkan text NOT NULL CHECK (mode_diizinkan IN ('sync', 'async', 'keduanya')),
  tata_letak text NOT NULL DEFAULT 'per_komponen' CHECK (tata_letak IN ('per_komponen', 'gabung_field')),
  aturan_konflik jsonb CHECK (aturan_konflik IS NULL OR jsonb_typeof(aturan_konflik) = 'object'),
  batas_waktu_detik integer CHECK (batas_waktu_detik > 0),
  dibuat_at timestamptz NOT NULL DEFAULT now(),
  dibuat_oleh uuid NOT NULL REFERENCES admin_user (pengguna_id),
  PRIMARY KEY (endpoint_blend_id, config_version),
  -- the admin API answers 422 for a time limit on a sync-only blend; the database holds the same line
  CONSTRAINT chk_blend_batas_waktu CHECK ((mode_diizinkan = 'sync') = (batas_waktu_detik IS NULL))
);
ALTER TABLE endpoint_blend
  ADD CONSTRAINT endpoint_blend_versi_fk FOREIGN KEY (endpoint_blend_id, config_version)
    REFERENCES endpoint_blend_versi (endpoint_blend_id, config_version) DEFERRABLE INITIALLY DEFERRED;

CREATE TABLE endpoint_blend_komponen (
  endpoint_blend_id uuid NOT NULL,
  config_version integer NOT NULL,
  nama_komponen varchar(64) NOT NULL CHECK (nama_komponen ~ '^[a-z][a-z0-9_]*$'),   -- key in output_payload
  downstream_endpoint_id uuid NOT NULL REFERENCES downstream_endpoint (downstream_endpoint_id),
  prioritas integer NOT NULL CHECK (prioritas >= 1),
  peta_input jsonb NOT NULL CHECK (jsonb_typeof(peta_input) = 'object'),
  PRIMARY KEY (endpoint_blend_id, config_version, nama_komponen),
  FOREIGN KEY (endpoint_blend_id, config_version) REFERENCES endpoint_blend_versi (endpoint_blend_id, config_version),
  UNIQUE (endpoint_blend_id, config_version, downstream_endpoint_id),   -- same endpoint twice is 422 (BC-02) and BL-03 (b) needs it
  UNIQUE (endpoint_blend_id, config_version, prioritas)
);
CREATE INDEX endpoint_blend_komponen_ep_idx ON endpoint_blend_komponen (downstream_endpoint_id);

-- V-03 / BC-01: a configuration version never changes. Components are only written while no blend refers to the version.
CREATE TRIGGER endpoint_blend_versi_immutable BEFORE UPDATE OR DELETE ON endpoint_blend_versi
  FOR EACH ROW EXECUTE FUNCTION tg_immutable();
CREATE TRIGGER endpoint_blend_komponen_immutable BEFORE UPDATE OR DELETE ON endpoint_blend_komponen
  FOR EACH ROW EXECUTE FUNCTION tg_immutable();

-- ADR-0007 D1 butir 1: a version has at least two components (checked at commit, components follow the version row).
CREATE FUNCTION tg_blend_versi_min_komponen() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
BEGIN
  IF (SELECT count(*) FROM endpoint_blend_komponen k
       WHERE k.endpoint_blend_id = NEW.endpoint_blend_id AND k.config_version = NEW.config_version) < 2 THEN
    RAISE EXCEPTION 'endpoint_blend % versi % punya kurang dari 2 komponen (BC-02)', NEW.endpoint_blend_id, NEW.config_version
      USING ERRCODE = 'check_violation';
  END IF;
  RETURN NULL;
END $$;
CREATE CONSTRAINT TRIGGER endpoint_blend_versi_min_komponen AFTER INSERT ON endpoint_blend_versi
  DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION tg_blend_versi_min_komponen();

-- ---------------------------------------------------------------- 2.2 provider marker (GB-01..GB-03, GB-10)
ALTER TABLE provider ADD COLUMN butuh_dasar_hukum_blend boolean NOT NULL DEFAULT false;
-- GB-10 / G-3: a provider with wajib_log_nik = true (Dukcapil) reads as marked. The contract has the platform admin supply
-- the explicit list (Dukcapil true, mock false, default false); until it arrives the rule itself is applied so that no
-- NIK-logging provider starts unmarked (fail-closed). A provider the admin wants unmarked is a later audited change.
UPDATE provider SET butuh_dasar_hukum_blend = true WHERE wajib_log_nik;

-- GB-03 audit of the marker, as a typed append-only table. audit_konfigurasi.pengguna_id is NOT NULL (F0, not relaxed),
-- and the first entry per provider is written by the migration itself (aktor = migrasi). The admin API also writes its
-- usual audit_konfigurasi row (entitas_jenis = 'provider').
CREATE TABLE provider_penanda_audit (
  audit_penanda_id uuid PRIMARY KEY,
  provider_id uuid NOT NULL REFERENCES provider (provider_id),
  terjadi_at timestamptz NOT NULL DEFAULT now(),
  aktor text NOT NULL CHECK (aktor = 'migrasi' OR aktor = 'admin'),
  pengguna_id uuid REFERENCES admin_user (pengguna_id),
  nilai_lama boolean,                                   -- NULL only for the migration entry
  nilai_baru boolean NOT NULL,                          -- the requested value, also when the gate refused it
  hasil text NOT NULL DEFAULT 'berhasil' CHECK (hasil IN ('berhasil', 'ditolak_gerbang')),
  alasan text NOT NULL CHECK (length(btrim(alasan)) > 0),
  referensi_persetujuan_nomor text,
  referensi_persetujuan_tanggal date,
  blend_terdampak jsonb NOT NULL DEFAULT '[]'::jsonb CHECK (jsonb_typeof(blend_terdampak) = 'array'),   -- [{endpoint_blend_id, nama}]
  CONSTRAINT chk_penanda_aktor CHECK ((aktor = 'migrasi') = (pengguna_id IS NULL)),
  CONSTRAINT chk_penanda_lama CHECK ((nilai_lama IS NULL) = (aktor = 'migrasi')),
  -- DQ-63 (f): true -> false needs the written-approval reference (GB-03)
  CONSTRAINT chk_penanda_true_false CHECK (
    NOT (nilai_lama AND NOT nilai_baru AND hasil = 'berhasil')
    OR (length(btrim(coalesce(referensi_persetujuan_nomor, ''))) > 0 AND referensi_persetujuan_tanggal IS NOT NULL)),
  -- DQ-63 (e): false -> true cannot succeed while an active blend uses the provider (G-4)
  CONSTRAINT chk_penanda_false_true CHECK (
    NOT (nilai_lama = false AND nilai_baru AND hasil = 'berhasil') OR jsonb_array_length(blend_terdampak) = 0)
);
CREATE INDEX provider_penanda_audit_idx ON provider_penanda_audit (provider_id, terjadi_at);
CREATE TRIGGER provider_penanda_audit_append_only BEFORE UPDATE OR DELETE ON provider_penanda_audit
  FOR EACH ROW EXECUTE FUNCTION tg_append_only();
CREATE TRIGGER provider_penanda_audit_no_truncate BEFORE TRUNCATE ON provider_penanda_audit
  FOR EACH STATEMENT EXECUTE FUNCTION tg_append_only();
INSERT INTO provider_penanda_audit (audit_penanda_id, provider_id, aktor, nilai_lama, nilai_baru, alasan)
SELECT uuidv7(), p.provider_id, 'migrasi', NULL, p.butuh_dasar_hukum_blend,
       'nilai awal migrasi F3 (GB-03): wajib_log_nik menentukan penanda (GB-10)'
  FROM provider p;

-- G-4 second line (GB-02 point 2, independent of the environment flag): false -> true is refused while an active blend
-- (current version) has a component on this provider. The UPDATE already holds the provider row exclusively; the activation
-- trigger below takes it FOR SHARE, so the two orders cannot both succeed (GB-07, U-111).
CREATE FUNCTION tg_provider_penanda_guard() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
BEGIN
  IF NEW.butuh_dasar_hukum_blend AND NOT OLD.butuh_dasar_hukum_blend AND EXISTS (
       SELECT 1 FROM endpoint_blend b
         JOIN endpoint_blend_komponen k ON k.endpoint_blend_id = b.endpoint_blend_id AND k.config_version = b.config_version
         JOIN downstream_endpoint e ON e.downstream_endpoint_id = k.downstream_endpoint_id
        WHERE b.status = 'aktif' AND e.provider_id = NEW.provider_id) THEN
    RAISE EXCEPTION 'penanda_dipakai_blend_aktif: provider % dipakai blend aktif (G-4)', NEW.provider_id
      USING ERRCODE = 'check_violation';
  END IF;
  RETURN NEW;
END $$;
CREATE TRIGGER provider_penanda_guard BEFORE UPDATE OF butuh_dasar_hukum_blend ON provider
  FOR EACH ROW WHEN (OLD.butuh_dasar_hukum_blend IS DISTINCT FROM NEW.butuh_dasar_hukum_blend)
  EXECUTE FUNCTION tg_provider_penanda_guard();

-- G-1 second line (GB-02 point 1): a blend becomes active (or changes version while active) with a marked provider among its
-- components only when the application states the gate is open for its process: SET LOCAL poolrouter.gerbang_blend_terbuka = 'true'.
-- The flag itself stays in the environment (GB-08); anything else, including an unset variable, is closed (fail-closed, GB-01).
CREATE FUNCTION tg_endpoint_blend_aktivasi() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
DECLARE
  v_ada_penanda boolean;
BEGIN
  IF TG_OP = 'INSERT' THEN
    IF NEW.status <> 'nonaktif' THEN
      RAISE EXCEPTION 'endpoint_blend baru selalu nonaktif (BC-02)' USING ERRCODE = 'check_violation';
    END IF;
    RETURN NEW;
  END IF;
  IF NEW.status = 'aktif' AND (OLD.status <> 'aktif' OR NEW.config_version <> OLD.config_version) THEN
    -- FOR SHARE on the component providers, in id order (no deadlock between two activations), then read the marker
    PERFORM 1 FROM provider pr
      WHERE pr.provider_id IN (SELECT e.provider_id FROM endpoint_blend_komponen k
                                 JOIN downstream_endpoint e ON e.downstream_endpoint_id = k.downstream_endpoint_id
                                WHERE k.endpoint_blend_id = NEW.endpoint_blend_id AND k.config_version = NEW.config_version)
      ORDER BY pr.provider_id FOR SHARE;
    SELECT coalesce(bool_or(pr.butuh_dasar_hukum_blend), false) INTO v_ada_penanda
      FROM provider pr
     WHERE pr.provider_id IN (SELECT e.provider_id FROM endpoint_blend_komponen k
                                JOIN downstream_endpoint e ON e.downstream_endpoint_id = k.downstream_endpoint_id
                               WHERE k.endpoint_blend_id = NEW.endpoint_blend_id AND k.config_version = NEW.config_version);
    IF v_ada_penanda AND coalesce(current_setting('poolrouter.gerbang_blend_terbuka', true), '') <> 'true' THEN
      RAISE EXCEPTION 'komponen_butuh_dasar_hukum: gerbang F3-4 tertutup untuk endpoint_blend % (G-1)', NEW.endpoint_blend_id
        USING ERRCODE = 'check_violation';
    END IF;
  END IF;
  RETURN NEW;
END $$;
CREATE TRIGGER endpoint_blend_aktivasi BEFORE INSERT OR UPDATE ON endpoint_blend
  FOR EACH ROW EXECUTE FUNCTION tg_endpoint_blend_aktivasi();

-- BC-01: components are written only while no blend refers to the version.
CREATE FUNCTION tg_blend_komponen_beku() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
BEGIN
  IF EXISTS (SELECT 1 FROM blend b WHERE b.endpoint_blend_id = NEW.endpoint_blend_id AND b.endpoint_blend_config_version = NEW.config_version) THEN
    RAISE EXCEPTION 'versi konfigurasi blend sudah dirujuk blend (BC-01)' USING ERRCODE = 'integrity_constraint_violation';
  END IF;
  RETURN NEW;
END $$;
-- (trigger is created after the blend table exists, below)

-- ---------------------------------------------------------------- 2.3 blend
CREATE TABLE blend (
  blend_id uuid PRIMARY KEY,                            -- UUIDv7 made by PoolRouter at intake; queue_id for the client
  client_id uuid NOT NULL REFERENCES client (client_id),
  credential_id uuid NOT NULL REFERENCES client_kredensial (credential_id),
  endpoint_blend_id uuid NOT NULL,
  endpoint_blend_config_version integer NOT NULL,
  mode text NOT NULL CHECK (mode IN ('sync', 'async')),
  status text NOT NULL CHECK (status IN ('berjalan', 'done', 'failed', 'expired', 'cancelled', 'rejected')),
  reject_reason text CHECK (reject_reason IN ('validasi_gagal', 'metadata_tidak_valid', 'kuota_habis', 'porsi_sync_habis',
    'objek_nonaktif', 'gerbang_blend_tertutup')),       -- gerbang_blend_tertutup exists only here, never on request (F0 enum untouched)
  reject_http_status integer CHECK (reject_http_status IN (400, 403, 422, 429, 503)),
  failure_reason text CHECK (failure_reason IN ('retry_habis', 'provider_error_permanen', 'timeout_sync', 'objek_nonaktif', 'fungsi_gagal')),
  anak_penentu_request_id uuid REFERENCES request (request_id),
  cancel_reason text CHECK (cancel_reason IN ('dibatalkan_client', 'client_nonaktif', 'kredensial_nonaktif', 'endpoint_nonaktif', 'pool_nonaktif', 'provider_nonaktif')),
  cancelled_by text,
  gerbang_dasar_hukum text NOT NULL CHECK (gerbang_dasar_hukum IN ('tidak_berlaku', 'diizinkan', 'ditolak')),
  ringkasan_komponen jsonb,
  idempotency_key varchar(128) CHECK (idempotency_key ~ '^[\x21-\x7e]+$'),
  idempotency_fingerprint varchar(64) CHECK (idempotency_fingerprint ~ '^[0-9a-f]{64}$'),
  output_payload jsonb,                                 -- Rahasia; the only Rahasia column of blend (section 1)
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
  webhook_not_before timestamptz,                       -- F1 WR-01 retry column, same meaning as on request
  source_ip varchar(45),

  FOREIGN KEY (endpoint_blend_id, endpoint_blend_config_version) REFERENCES endpoint_blend_versi (endpoint_blend_id, config_version),
  -- reject (DQ-62, DQ-63 c)
  CONSTRAINT chk_blend_reject CHECK (
    (status = 'rejected') = (reject_reason IS NOT NULL) AND (status = 'rejected') = (reject_http_status IS NOT NULL)),
  CONSTRAINT chk_blend_reject_http CHECK (reject_reason IS NULL OR (reject_reason, reject_http_status) IN (
    ('validasi_gagal', 400), ('validasi_gagal', 422), ('metadata_tidak_valid', 400), ('metadata_tidak_valid', 422),
    ('kuota_habis', 429), ('porsi_sync_habis', 429), ('objek_nonaktif', 403), ('objek_nonaktif', 503),
    ('gerbang_blend_tertutup', 403))),
  CONSTRAINT chk_blend_gerbang CHECK (                  -- GB-05, DQ-63 (c): reject reason <=> proof 'ditolak'
    coalesce(reject_reason = 'gerbang_blend_tertutup', false) = (gerbang_dasar_hukum = 'ditolak')),
  -- failure (BL-05)
  CONSTRAINT chk_blend_failure CHECK (
    (status = 'failed') = (failure_reason IS NOT NULL) AND (status = 'failed') = (anak_penentu_request_id IS NOT NULL)),
  -- cancel
  CONSTRAINT chk_blend_cancel CHECK (
    (status = 'cancelled') = (cancel_reason IS NOT NULL) AND (status = 'cancelled') = (cancelled_by IS NOT NULL)),
  CONSTRAINT chk_blend_cancelled_by CHECK (
    cancelled_by IS NULL OR (
      CASE WHEN cancel_reason = 'dibatalkan_client' THEN cancelled_by = 'client'
           ELSE cancelled_by = 'sistem' OR cancelled_by ~ '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$' END)),
  CONSTRAINT chk_blend_failure_mode CHECK (failure_reason IS NULL OR mode = 'async' OR failure_reason <> 'objek_nonaktif'),
  -- terminal states
  CONSTRAINT chk_blend_completed CHECK ((status <> 'berjalan') = (completed_at IS NOT NULL)),
  CONSTRAINT chk_blend_completed_urut CHECK (completed_at IS NULL OR completed_at >= requested_at),
  -- DQ-62 (f): the summary is an array of {nama_komponen, request_id, status_api, failure_reason}; no numbers, no 16-digit strings.
  -- Inline jsonpath on purpose: a function in a CHECK would need EXECUTE for every role that writes the table.
  CONSTRAINT chk_blend_ringkasan CHECK (
    CASE WHEN status IN ('berjalan', 'rejected') THEN ringkasan_komponen IS NULL
         ELSE ringkasan_komponen IS NOT NULL AND jsonb_typeof(ringkasan_komponen) = 'array'
              AND NOT jsonb_path_exists(ringkasan_komponen, '$[*] ? (@.type() != "object")')
              AND NOT jsonb_path_exists(ringkasan_komponen, '$[*].keyvalue() ? (@.key != "nama_komponen" && @.key != "request_id" && @.key != "status_api" && @.key != "failure_reason")')
              AND NOT jsonb_path_exists(ringkasan_komponen, '$.** ? (@.type() == "number")')
              AND NOT jsonb_path_exists(ringkasan_komponen, '$.** ? (@ like_regex "[0-9]{16}")') END),
  -- Rahasia payload only for done; may be purged afterwards (BP-01, BP-04, DQ-61 e)
  CONSTRAINT chk_blend_output CHECK (
    CASE WHEN status = 'done' THEN output_payload IS NOT NULL OR payload_purged_at IS NOT NULL
         ELSE output_payload IS NULL AND payload_purged_at IS NULL END),
  -- idempotency: async only, never on a rejected blend (BC-04, BL-07)
  CONSTRAINT chk_blend_idem CHECK (
    (idempotency_key IS NULL) = (idempotency_fingerprint IS NULL)
    AND (idempotency_key IS NULL OR (mode = 'async' AND status <> 'rejected'))),
  -- the polling window is frozen at intake for accepted async blends (BP-01); a rejected blend has none
  CONSTRAINT chk_blend_ttl CHECK (
    CASE WHEN mode = 'sync' THEN ttl_polling_detik IS NULL
         WHEN status = 'rejected' THEN ttl_polling_detik IS NULL
         ELSE ttl_polling_detik IS NOT NULL END),
  -- webhook (one per async terminal blend except rejected; rejected and sync read 'tidak_dikirim')
  CONSTRAINT chk_blend_webhook CHECK (
    CASE WHEN status = 'berjalan' THEN webhook_status IS NULL
         WHEN status = 'rejected' OR mode = 'sync' THEN webhook_status = 'tidak_dikirim'
         ELSE webhook_status IS NOT NULL END),
  CONSTRAINT chk_blend_webhook_not_before CHECK (webhook_not_before IS NULL OR coalesce(webhook_status = 'menunggu', false)),
  CONSTRAINT chk_blend_batas CHECK (batas_waktu_at >= requested_at)
);
CREATE UNIQUE INDEX blend_idempotency_uq ON blend (client_id, idempotency_key) WHERE idempotency_key IS NOT NULL;   -- namespace apart from request (BL-07)
CREATE INDEX blend_client_requested_idx ON blend (client_id, requested_at);
CREATE INDEX blend_endpoint_requested_idx ON blend (endpoint_blend_id, requested_at);
CREATE INDEX blend_berjalan_idx ON blend (batas_waktu_at) WHERE status = 'berjalan';     -- merge job and DQ-61 (f)
CREATE INDEX blend_completed_idx ON blend (completed_at) WHERE completed_at IS NOT NULL; -- retention (8.4), BP-01
CREATE INDEX blend_webhook_idx ON blend (completed_at) WHERE webhook_status = 'menunggu';
CREATE INDEX blend_webhook_retry_idx ON blend (webhook_not_before) WHERE webhook_status = 'menunggu' AND webhook_not_before IS NOT NULL;
CREATE INDEX blend_payload_purge_idx ON blend (completed_at) WHERE output_payload IS NOT NULL;

CREATE TRIGGER blend_komponen_beku BEFORE INSERT ON endpoint_blend_komponen
  FOR EACH ROW EXECUTE FUNCTION tg_blend_komponen_beku();

-- allowed transitions (4.1 diagram, T-05). A terminal blend keeps its outcome; only payload purge and webhook bookkeeping change.
CREATE FUNCTION tg_blend_transisi() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  IF TG_OP = 'INSERT' THEN
    IF NEW.status NOT IN ('berjalan', 'rejected') THEN
      RAISE EXCEPTION 'status awal blend tidak sah: %', NEW.status USING ERRCODE = 'check_violation';
    END IF;
    RETURN NEW;
  END IF;
  IF NEW.status IS DISTINCT FROM OLD.status AND NOT (OLD.status = 'berjalan' AND NEW.status IN ('done', 'failed', 'expired', 'cancelled')) THEN
    RAISE EXCEPTION 'transisi status blend tidak sah: % -> %', OLD.status, NEW.status USING ERRCODE = 'check_violation';
  END IF;
  IF NEW.blend_id <> OLD.blend_id OR NEW.client_id <> OLD.client_id OR NEW.credential_id <> OLD.credential_id
     OR NEW.endpoint_blend_id <> OLD.endpoint_blend_id OR NEW.endpoint_blend_config_version <> OLD.endpoint_blend_config_version
     OR NEW.mode <> OLD.mode OR NEW.requested_at <> OLD.requested_at OR NEW.batas_waktu_at <> OLD.batas_waktu_at
     OR NEW.ttl_polling_detik IS DISTINCT FROM OLD.ttl_polling_detik OR NEW.gerbang_dasar_hukum <> OLD.gerbang_dasar_hukum
     OR NEW.idempotency_key IS DISTINCT FROM OLD.idempotency_key OR NEW.idempotency_fingerprint IS DISTINCT FROM OLD.idempotency_fingerprint THEN
    RAISE EXCEPTION 'kolom identitas blend tidak boleh diubah' USING ERRCODE = 'check_violation';
  END IF;
  IF OLD.status <> 'berjalan' AND (
       NEW.completed_at IS DISTINCT FROM OLD.completed_at OR NEW.reject_reason IS DISTINCT FROM OLD.reject_reason
       OR NEW.reject_http_status IS DISTINCT FROM OLD.reject_http_status OR NEW.failure_reason IS DISTINCT FROM OLD.failure_reason
       OR NEW.anak_penentu_request_id IS DISTINCT FROM OLD.anak_penentu_request_id OR NEW.cancel_reason IS DISTINCT FROM OLD.cancel_reason
       OR NEW.cancelled_by IS DISTINCT FROM OLD.cancelled_by OR NEW.ringkasan_komponen IS DISTINCT FROM OLD.ringkasan_komponen) THEN
    RAISE EXCEPTION 'status terminal blend tidak berubah (T-05)' USING ERRCODE = 'check_violation';
  END IF;
  RETURN NEW;
END $$;
CREATE TRIGGER blend_transisi BEFORE INSERT OR UPDATE ON blend
  FOR EACH ROW EXECUTE FUNCTION tg_blend_transisi();

-- ---------------------------------------------------------------- 2.4 blend_status_history
CREATE TABLE blend_status_history (
  blend_id uuid NOT NULL REFERENCES blend (blend_id),
  seq integer NOT NULL CHECK (seq >= 1),
  status_dari text CHECK (status_dari IN ('berjalan', 'done', 'failed', 'expired', 'cancelled', 'rejected')),
  status_ke text NOT NULL CHECK (status_ke IN ('berjalan', 'done', 'failed', 'expired', 'cancelled', 'rejected')),
  alasan text,                                          -- reject_reason | cancel_reason | failure_reason
  aktor text NOT NULL CHECK (aktor IN ('webserver', 'controller', 'client')
    OR aktor ~ '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$'),
  objek_pemicu_id uuid,
  terjadi_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (blend_id, seq),
  CONSTRAINT chk_blend_seq_pertama CHECK ((seq = 1) = (status_dari IS NULL)),
  CONSTRAINT chk_blend_pemicu_k20 CHECK (
    (coalesce(alasan IN ('client_nonaktif', 'kredensial_nonaktif', 'endpoint_nonaktif', 'pool_nonaktif', 'provider_nonaktif'), false)
     OR (status_ke = 'failed' AND alasan = 'objek_nonaktif')) = (objek_pemicu_id IS NOT NULL))
);
CREATE TRIGGER blend_status_history_append_only BEFORE UPDATE OR DELETE ON blend_status_history
  FOR EACH ROW EXECUTE FUNCTION tg_append_only();

-- T-04 for blend: every status written to blend has a history row in the same transaction (same technique as request).
CREATE FUNCTION tg_blend_punya_history() RETURNS trigger LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
BEGIN
  IF NOT EXISTS (SELECT 1 FROM blend_status_history h
                   WHERE h.blend_id = NEW.blend_id
                     AND CASE WHEN TG_OP = 'INSERT' THEN h.seq = 1 AND h.status_dari IS NULL
                              ELSE h.status_dari = OLD.status END
                     AND h.status_ke = NEW.status AND h.terjadi_at >= transaction_timestamp()) THEN
    RAISE EXCEPTION 'blend % berstatus % tanpa baris blend_status_history yang cocok (T-04)', NEW.blend_id, NEW.status
      USING ERRCODE = 'integrity_constraint_violation';
  END IF;
  RETURN NULL;
END $$;
CREATE CONSTRAINT TRIGGER blend_history_wajib AFTER INSERT OR UPDATE OF status ON blend
  DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION tg_blend_punya_history();

-- ---------------------------------------------------------------- 2.5 blend_webhook_log
-- Separate from webhook_log: its request_id stays NOT NULL (F0 constraint not relaxed). webhook-id on the message = blend_id.
CREATE TABLE blend_webhook_log (
  webhook_delivery_id uuid PRIMARY KEY,
  blend_id uuid NOT NULL REFERENCES blend (blend_id),
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
CREATE TRIGGER blend_webhook_log_no_update BEFORE UPDATE ON blend_webhook_log
  FOR EACH ROW EXECUTE FUNCTION tg_immutable();

-- ---------------------------------------------------------------- 2.6 nullable columns on F0 objects
ALTER TABLE request
  ADD COLUMN blend_id uuid REFERENCES blend (blend_id);
-- BL-03 (c) and (f): a child never carries an idempotency key and never sends a webhook (N-1). F0 rows have blend_id NULL.
ALTER TABLE request
  ADD CONSTRAINT chk_anak_blend_idem CHECK (blend_id IS NULL OR idempotency_key IS NULL),
  ADD CONSTRAINT chk_anak_blend_webhook CHECK (
    blend_id IS NULL OR (webhook_attempt_count = 0 AND webhook_status IS NOT DISTINCT FROM CASE WHEN webhook_status IS NULL THEN NULL ELSE 'tidak_dikirim' END));
CREATE INDEX request_blend_idx ON request (blend_id) WHERE blend_id IS NOT NULL;
CREATE UNIQUE INDEX request_blend_endpoint_uq ON request (blend_id, downstream_endpoint_id) WHERE blend_id IS NOT NULL;   -- DQ-60 (g)

ALTER TABLE audit_akses ADD COLUMN target_blend_id uuid;   -- no FK, like target_request_id: the row outlives the blend

-- audit_konfigurasi: the admin API audits endpoint_blend changes there. Only the domain of entitas_jenis grows.
ALTER TABLE audit_konfigurasi DROP CONSTRAINT audit_konfigurasi_entitas_jenis_check;
ALTER TABLE audit_konfigurasi ADD CONSTRAINT audit_konfigurasi_entitas_jenis_check CHECK (entitas_jenis IN (
  'client', 'kredensial', 'downstream_endpoint', 'downstream_pool', 'upstream_pool', 'provider',
  'client_webhook', 'webhook_domain_allowlist', 'provider_config', 'endpoint_config',
  'endpoint_metadata_field', 'konfigurasi_platform', 'admin_user', 'endpoint_blend'));

-- BL-01: blend_id is written once at insert and never changes (no NULL -> set, no re-pointing).
CREATE FUNCTION tg_request_blend_guard() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  IF NEW.blend_id IS DISTINCT FROM OLD.blend_id THEN
    RAISE EXCEPTION 'request.blend_id ditulis sekali dan tidak berubah (BL-01)' USING ERRCODE = 'check_violation';
  END IF;
  RETURN NEW;
END $$;
CREATE TRIGGER request_blend_guard BEFORE UPDATE OF blend_id ON request
  FOR EACH ROW EXECUTE FUNCTION tg_request_blend_guard();

-- BL-03 (a)(b)(d)(e)(g) at child insert: parent exists, is not rejected, matches client / credential / mode, the child's endpoint is
-- one of the components of the parent's config version, ttl equals batas_waktu_at (async), received_at within 5 s of requested_at.
CREATE FUNCTION tg_request_anak_blend() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
DECLARE
  b blend%ROWTYPE;
BEGIN
  SELECT * INTO b FROM blend WHERE blend_id = NEW.blend_id;
  IF NOT FOUND THEN
    RAISE EXCEPTION 'anak blend tanpa induk (BL-03 a)' USING ERRCODE = 'integrity_constraint_violation';
  END IF;
  IF b.status = 'rejected' THEN
    RAISE EXCEPTION 'blend rejected tidak punya anak (BL-03 g)' USING ERRCODE = 'check_violation';
  END IF;
  IF NEW.client_id <> b.client_id OR NEW.credential_id <> b.credential_id OR NEW.mode <> b.mode THEN
    RAISE EXCEPTION 'client_id, credential_id, dan mode anak harus sama dengan induk (BL-03 a)' USING ERRCODE = 'check_violation';
  END IF;
  IF NOT EXISTS (SELECT 1 FROM endpoint_blend_komponen k
                  WHERE k.endpoint_blend_id = b.endpoint_blend_id AND k.config_version = b.endpoint_blend_config_version
                    AND k.downstream_endpoint_id = NEW.downstream_endpoint_id) THEN
    RAISE EXCEPTION 'endpoint anak bukan komponen versi konfigurasi induk (BL-03 b)' USING ERRCODE = 'check_violation';
  END IF;
  IF b.mode = 'async' AND NEW.ttl_expires_at IS DISTINCT FROM b.batas_waktu_at THEN
    RAISE EXCEPTION 'ttl_expires_at anak harus sama dengan batas_waktu_at induk (BL-03 d)' USING ERRCODE = 'check_violation';
  END IF;
  IF abs(extract(epoch FROM (NEW.received_at - b.requested_at))) > 5 THEN
    RAISE EXCEPTION 'received_at anak berselisih lebih dari 5 detik dari requested_at induk (BL-03 e)' USING ERRCODE = 'check_violation';
  END IF;
  RETURN NEW;
END $$;
CREATE TRIGGER request_anak_blend BEFORE INSERT ON request
  FOR EACH ROW WHEN (NEW.blend_id IS NOT NULL) EXECUTE FUNCTION tg_request_anak_blend();

-- BL-03 (b), DQ-60 (b): a non-rejected blend has exactly as many children as components, created in the same transaction (BL-02).
CREATE FUNCTION tg_blend_jumlah_anak() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
DECLARE
  v_anak integer;
  v_komponen integer;
BEGIN
  SELECT count(*) INTO v_anak FROM request WHERE blend_id = NEW.blend_id;
  SELECT count(*) INTO v_komponen FROM endpoint_blend_komponen
   WHERE endpoint_blend_id = NEW.endpoint_blend_id AND config_version = NEW.endpoint_blend_config_version;
  IF v_anak <> v_komponen THEN
    RAISE EXCEPTION 'blend % punya % anak untuk % komponen (BL-02, BL-03 b)', NEW.blend_id, v_anak, v_komponen
      USING ERRCODE = 'integrity_constraint_violation';
  END IF;
  RETURN NULL;
END $$;
CREATE CONSTRAINT TRIGGER blend_anak_lengkap AFTER INSERT ON blend
  DEFERRABLE INITIALLY DEFERRED FOR EACH ROW WHEN (NEW.status <> 'rejected') EXECUTE FUNCTION tg_blend_jumlah_anak();

-- DQ-65 (c) / BL-03 (f): a child has no row in webhook_log (the blend sends the only webhook).
CREATE FUNCTION tg_webhook_log_bukan_anak() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
BEGIN
  IF EXISTS (SELECT 1 FROM request r WHERE r.request_id = NEW.request_id AND r.blend_id IS NOT NULL) THEN
    RAISE EXCEPTION 'anak blend tidak punya webhook_log (BL-03 f, N-1)' USING ERRCODE = 'check_violation';
  END IF;
  RETURN NEW;
END $$;
CREATE TRIGGER webhook_log_bukan_anak BEFORE INSERT ON webhook_log
  FOR EACH ROW EXECUTE FUNCTION tg_webhook_log_bukan_anak();

-- BL-05 / BL-06 / DQ-61 (a)(c)(d) as far as the database can see them when the blend closes: the closing state must be
-- consistent with the children at that moment. The full recomputation of BL-05 stays a pipeline check (DQ-61 a); the
-- explicit client cancel (BL-06) is held to the same attempt_count = 0 rule.
CREATE FUNCTION tg_blend_terminal_konsisten() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
DECLARE
  v_anak integer;
  v_percobaan bigint;
  v_done integer;
  v_failed integer;
  v_cancelled integer;
  v_expired integer;
  p request%ROWTYPE;
BEGIN
  SELECT count(*), coalesce(sum(attempt_count), 0), count(*) FILTER (WHERE status = 'done'), count(*) FILTER (WHERE status = 'failed'),
         count(*) FILTER (WHERE status = 'cancelled'), count(*) FILTER (WHERE status = 'expired')
    INTO v_anak, v_percobaan, v_done, v_failed, v_cancelled, v_expired
    FROM request WHERE blend_id = NEW.blend_id;
  IF NEW.status = 'done' AND v_done <> v_anak THEN
    RAISE EXCEPTION 'blend done tetapi ada anak yang tidak done (BL-05.1)' USING ERRCODE = 'integrity_constraint_violation';
  ELSIF NEW.status = 'cancelled' AND v_percobaan > 0 THEN
    RAISE EXCEPTION 'blend cancelled tetapi anak sudah punya percobaan (BL-06, DQ-61 c)' USING ERRCODE = 'integrity_constraint_violation';
  ELSIF NEW.status = 'expired' AND (v_percobaan > 0 OR v_failed + v_cancelled + v_done > 0 OR v_expired = 0) THEN
    RAISE EXCEPTION 'blend expired tidak sesuai keadaan anak (BL-05.4, DQ-61 d)' USING ERRCODE = 'integrity_constraint_violation';
  ELSIF NEW.status = 'failed' THEN
    SELECT * INTO p FROM request WHERE request_id = NEW.anak_penentu_request_id;
    IF NOT FOUND OR p.blend_id IS DISTINCT FROM NEW.blend_id THEN
      RAISE EXCEPTION 'anak_penentu_request_id bukan anak blend ini (BL-05)' USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    IF v_done = v_anak THEN
      RAISE EXCEPTION 'blend failed tetapi semua anak done (BL-05)' USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    IF p.status = 'failed' AND p.failure_reason IS DISTINCT FROM NEW.failure_reason THEN
      RAISE EXCEPTION 'failure_reason blend harus sama dengan anak penentu yang failed (BL-05)' USING ERRCODE = 'integrity_constraint_violation';
    END IF;
  END IF;
  RETURN NULL;
END $$;
CREATE CONSTRAINT TRIGGER blend_terminal_konsisten AFTER UPDATE OF status ON blend
  DEFERRABLE INITIALLY DEFERRED FOR EACH ROW WHEN (NEW.status IN ('done', 'failed', 'expired', 'cancelled')) EXECUTE FUNCTION tg_blend_terminal_konsisten();

-- ---------------------------------------------------------------- 5. gate trail (GB-09)
-- One row per change of the pair (effective state, dasar_hukum_ref), written under an advisory lock so instances that start
-- together write once (compare-and-set against the last event). The trigger holds the rule for any writer; fn_catat_gerbang_blend
-- is the entry point of the process start. DQ-63 (h)(i) read this table.
CREATE TABLE gerbang_blend_peristiwa (
  peristiwa_id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,   -- order of events
  terjadi_at timestamptz NOT NULL DEFAULT now(),
  jenis text NOT NULL CHECK (jenis IN ('gerbang_blend_dibuka', 'gerbang_blend_ditutup')),
  nilai_flag text,                                                -- raw value of ${POOLROUTER_BLEND_DATA_KEPENDUDUKAN_DIIZINKAN}
  dasar_hukum_ref varchar(200),                                   -- document ID, never its content; NULL when closed
  sebab_tertutup text CHECK (sebab_tertutup IN ('flag_bukan_true', 'rujukan_kosong')),
  instance varchar(128) NOT NULL,
  CONSTRAINT chk_gerbang_dibuka CHECK (
    jenis <> 'gerbang_blend_dibuka'
    OR (nilai_flag = 'true' AND length(btrim(coalesce(dasar_hukum_ref, ''))) > 0 AND sebab_tertutup IS NULL)),
  CONSTRAINT chk_gerbang_ditutup CHECK (
    jenis <> 'gerbang_blend_ditutup'
    OR (dasar_hukum_ref IS NULL AND sebab_tertutup IS NOT NULL
        AND (sebab_tertutup = 'rujukan_kosong') = coalesce(nilai_flag = 'true', false)))
);
CREATE FUNCTION tg_gerbang_blend_peristiwa() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
DECLARE
  v_terakhir gerbang_blend_peristiwa%ROWTYPE;
  v_lama_terbuka boolean := false;
  v_lama_ref text := '';
BEGIN
  PERFORM pg_advisory_xact_lock(hashtextextended('poolrouter.gerbang_blend_peristiwa', 0));
  SELECT * INTO v_terakhir FROM gerbang_blend_peristiwa ORDER BY peristiwa_id DESC LIMIT 1;
  IF FOUND THEN
    v_lama_terbuka := v_terakhir.jenis = 'gerbang_blend_dibuka';
    v_lama_ref := coalesce(v_terakhir.dasar_hukum_ref, '');
  END IF;
  -- no events yet counts as (closed, empty); the pair must change for a new event to be written (GB-09)
  IF v_lama_terbuka = (NEW.jenis = 'gerbang_blend_dibuka') AND v_lama_ref = coalesce(NEW.dasar_hukum_ref, '') THEN
    RAISE EXCEPTION 'peristiwa gerbang tidak mengubah pasangan (keadaan efektif, dasar_hukum_ref) (GB-09, DQ-63 i)'
      USING ERRCODE = 'unique_violation';
  END IF;
  RETURN NEW;
END $$;
CREATE TRIGGER gerbang_blend_peristiwa_cas BEFORE INSERT ON gerbang_blend_peristiwa
  FOR EACH ROW EXECUTE FUNCTION tg_gerbang_blend_peristiwa();
CREATE TRIGGER gerbang_blend_peristiwa_append_only BEFORE UPDATE OR DELETE ON gerbang_blend_peristiwa
  FOR EACH ROW EXECUTE FUNCTION tg_append_only();
CREATE TRIGGER gerbang_blend_peristiwa_no_truncate BEFORE TRUNCATE ON gerbang_blend_peristiwa
  FOR EACH STATEMENT EXECUTE FUNCTION tg_append_only();

-- Process start: derive the effective state (GB-01: open only for the string 'true' AND a non-empty reference) and write the
-- event when the pair changed. Returns the event written, or NULL when nothing changed.
CREATE FUNCTION fn_catat_gerbang_blend(p_nilai_flag text, p_dasar_hukum_ref text, p_instance text) RETURNS text
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
DECLARE
  v_ref text := nullif(btrim(coalesce(p_dasar_hukum_ref, '')), '');
  v_jenis text;
BEGIN
  IF p_nilai_flag IS NOT DISTINCT FROM 'true' AND v_ref IS NOT NULL THEN
    v_jenis := 'gerbang_blend_dibuka';
    INSERT INTO gerbang_blend_peristiwa (jenis, nilai_flag, dasar_hukum_ref, instance) VALUES (v_jenis, p_nilai_flag, v_ref, p_instance);
  ELSE
    v_jenis := 'gerbang_blend_ditutup';
    v_ref := NULL;
    INSERT INTO gerbang_blend_peristiwa (jenis, nilai_flag, dasar_hukum_ref, sebab_tertutup, instance)
      VALUES (v_jenis, p_nilai_flag, NULL, CASE WHEN p_nilai_flag IS NOT DISTINCT FROM 'true' THEN 'rujukan_kosong' ELSE 'flag_bukan_true' END, p_instance);
  END IF;
  RETURN v_jenis;
EXCEPTION WHEN unique_violation THEN
  RETURN NULL;   -- same pair as the last event: nothing to write
END $$;

-- ---------------------------------------------------------------- 6. payload retention (BP-01, BP-02, BP-05)
-- BP-01 / BP-02: blend.output_payload goes NULL (payload_purged_at set) after completed_at + ttl_polling_detik (async) or
-- right after the response (sync), whatever the log_mode of the children (BP-03). pr_controller has no SELECT on output_payload
-- (A-01), so it purges through this function.
CREATE FUNCTION fn_purge_blend_payload(p_batas integer DEFAULT 1000) RETURNS integer
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
DECLARE
  v_n integer;
BEGIN
  WITH kandidat AS (
    SELECT b.blend_id FROM blend b
     WHERE b.status = 'done' AND b.output_payload IS NOT NULL
       AND b.completed_at + CASE WHEN b.mode = 'sync' THEN interval '0' ELSE make_interval(secs => b.ttl_polling_detik) END <= now()
     ORDER BY b.completed_at LIMIT p_batas FOR UPDATE SKIP LOCKED)
  UPDATE blend b SET output_payload = NULL, payload_purged_at = now() FROM kandidat k WHERE b.blend_id = k.blend_id;
  GET DIAGNOSTICS v_n = ROW_COUNT;
  RETURN v_n;
END $$;

-- BP-05 / N-3: the output (and the input and metadata purged with it) of a child is not removed before its blend is terminal;
-- the merge function reads the children's output_payload. Same function as F0 P-03 with one extra predicate; rollback restores it.
CREATE OR REPLACE FUNCTION fn_purge_p03(p_batas integer DEFAULT 1000) RETURNS integer
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
DECLARE
  v_n integer;
BEGIN
  WITH kandidat AS (
    SELECT r.request_id
      FROM request r
      JOIN transaction_log l ON l.kode_transaksi = r.final_kode_transaksi
      LEFT JOIN downstream_pool d ON d.downstream_pool_id = r.downstream_pool_id
      WHERE r.status IN ('done', 'failed') AND r.output_purged_at IS NULL AND l.log_mode = 'metadata_only'
       AND r.completed_at + CASE WHEN r.mode = 'sync' THEN interval '0' ELSE make_interval(secs => coalesce(d.ttl_antrean_detik, 0)) END <= now()
       AND NOT fn_ditahan_data_balikan(r.request_id)
       AND (r.blend_id IS NULL OR EXISTS (SELECT 1 FROM blend b WHERE b.blend_id = r.blend_id AND b.status <> 'berjalan'))   -- BP-05 (N-3)
     ORDER BY r.completed_at
     LIMIT p_batas
     FOR UPDATE OF r SKIP LOCKED)
  UPDATE request r
     SET input_payload = NULL, output_payload = NULL, output_purged_at = now(),
         metadata = fn_metadata_tanpa_rahasia(r.downstream_endpoint_id, r.metadata),
         payload_purged_at = now()
    FROM kandidat k WHERE r.request_id = k.request_id;
  GET DIAGNOSTICS v_n = ROW_COUNT;
  RETURN v_n;
END $$;
