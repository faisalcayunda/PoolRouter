-- F0 schema, part 2: request, status history, transaction log, webhook log, attempt counter,
-- access audit, data balikan (SHAA-193). Source: data contract v0.7.10 sections 3.1-3.6, 5, 8.2.
-- Timestamps are timestamptz (UTC instants); JSON is jsonb; VARCHAR domains are text + CHECK.

-- ---------------------------------------------------------------- request (3.1)
CREATE TABLE request (
  request_id uuid PRIMARY KEY,
  client_id uuid NOT NULL REFERENCES client (client_id),
  credential_id uuid NOT NULL REFERENCES client_kredensial (credential_id),
  downstream_endpoint_id uuid NOT NULL REFERENCES downstream_endpoint (downstream_endpoint_id),
  -- NULL list for unparseable bodies is empty (design review round 1, SHAA-193): downstream_pool_id,
  -- prioritas, prioritas_efektif derive from endpoint/pool configuration, never from the body, and are
  -- always NOT NULL via chk_pool_kolom_not_null. See docs/db/skema-f0.md section 2.
  downstream_pool_id uuid REFERENCES downstream_pool (downstream_pool_id),
  provider_id uuid NOT NULL REFERENCES provider (provider_id),
  upstream_pool_id uuid REFERENCES upstream_pool (upstream_pool_id),
  provider_config_version integer NOT NULL,
  endpoint_config_version integer NOT NULL,
  mode text NOT NULL CHECK (mode IN ('sync', 'async')),
  prioritas integer CHECK (prioritas >= 1),
  prioritas_efektif integer CHECK (prioritas_efektif >= 1),
  status text NOT NULL CHECK (status IN ('queued', 'dispatched', 'in_flight', 'done', 'failed', 'rejected', 'expired', 'cancelled')),
  reject_reason text CHECK (reject_reason IN ('validasi_gagal', 'metadata_tidak_valid', 'kuota_habis', 'porsi_sync_habis', 'objek_nonaktif')),
  reject_http_status integer CHECK (reject_http_status IN (400, 403, 422, 429, 503)),
  cancel_reason text CHECK (cancel_reason IN ('dibatalkan_client', 'client_nonaktif', 'kredensial_nonaktif', 'endpoint_nonaktif', 'pool_nonaktif', 'provider_nonaktif')),
  cancelled_by text,                                   -- C-02: 'client' | admin user id | 'sistem'
  failure_reason text CHECK (failure_reason IN ('retry_habis', 'provider_error_permanen', 'timeout_sync', 'objek_nonaktif')),
  idempotency_key varchar(128) CHECK (idempotency_key ~ '^[\x21-\x7e]+$'),
  idempotency_fingerprint varchar(64) CHECK (idempotency_fingerprint ~ '^[0-9a-f]{64}$'),
  referensi_client varchar(128),
  input_payload jsonb,
  metadata jsonb NOT NULL,
  payload_purged_at timestamptz,
  output_purged_at timestamptz,
  received_at timestamptz NOT NULL,
  ttl_expires_at timestamptz,
  dispatched_at timestamptz,
  not_before timestamptz,
  dispatch_key uuid,
  first_attempt_at timestamptz,
  completed_at timestamptz,
  attempt_count integer NOT NULL DEFAULT 0 CHECK (attempt_count >= 0),
  final_kode_transaksi uuid,                           -- FK added after transaction_log exists
  output_payload jsonb,
  sumber_hasil text CHECK (sumber_hasil IN ('provider', 'cache')),
  webhook_status text CHECK (webhook_status IN ('menunggu', 'terkirim', 'gagal', 'tidak_dikirim')),
  webhook_attempt_count integer NOT NULL DEFAULT 0 CHECK (webhook_attempt_count >= 0),
  webhook_last_at timestamptz,
  webhook_last_http_status integer,
  webhook_last_error varchar(200),
  source_ip varchar(45),

  -- reason/state consistency (DQ-08)
  CONSTRAINT chk_reject CHECK (
    (status = 'rejected') = (reject_reason IS NOT NULL) AND (status = 'rejected') = (reject_http_status IS NOT NULL)),
  CONSTRAINT chk_reject_http CHECK (reject_reason IS NULL OR (reject_reason, reject_http_status) IN (
    ('validasi_gagal', 400), ('validasi_gagal', 422), ('metadata_tidak_valid', 400), ('metadata_tidak_valid', 422),
    ('kuota_habis', 429), ('porsi_sync_habis', 429), ('objek_nonaktif', 403), ('objek_nonaktif', 503))),
  CONSTRAINT chk_cancel CHECK (
    (status = 'cancelled') = (cancel_reason IS NOT NULL) AND (status = 'cancelled') = (cancelled_by IS NOT NULL)),
  CONSTRAINT chk_cancelled_by CHECK (
    cancelled_by IS NULL OR (
      CASE WHEN cancel_reason = 'dibatalkan_client' THEN cancelled_by = 'client'
           ELSE cancelled_by = 'sistem' OR cancelled_by ~ '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$' END)),
  CONSTRAINT chk_failure CHECK ((status = 'failed') = (failure_reason IS NOT NULL)),
  CONSTRAINT chk_failure_mode CHECK (failure_reason IS NULL OR mode = 'async' OR failure_reason <> 'objek_nonaktif'),
  -- attempts (T-02a, DQ-07)
  CONSTRAINT chk_attempt CHECK (
    CASE WHEN status IN ('rejected', 'expired', 'cancelled') THEN attempt_count = 0
         WHEN status IN ('done', 'failed', 'in_flight') THEN attempt_count >= 1
         ELSE true END),
  CONSTRAINT chk_sync_tidak_antre CHECK (mode = 'async' OR status NOT IN ('queued', 'dispatched')),
  CONSTRAINT chk_upstream_rejected CHECK (status <> 'rejected' OR upstream_pool_id IS NULL),
  -- terminal states
  CONSTRAINT chk_completed CHECK (
    (status IN ('done', 'failed', 'rejected', 'expired', 'cancelled')) = (completed_at IS NOT NULL)),
  CONSTRAINT chk_final_log CHECK (
    status NOT IN ('done', 'failed') OR final_kode_transaksi IS NOT NULL),
  CONSTRAINT chk_final_log_only CHECK (status IN ('done', 'failed') OR final_kode_transaksi IS NULL),
  -- output only for done; may be purged afterwards (P-03)
  CONSTRAINT chk_output CHECK (
    CASE WHEN status = 'done' THEN output_payload IS NOT NULL OR output_purged_at IS NOT NULL
         ELSE output_payload IS NULL END),
  -- queue time limit (async only)
  CONSTRAINT chk_ttl_mode CHECK (mode = 'async' OR ttl_expires_at IS NULL),
  -- idempotency (async only, never on rejected)
  CONSTRAINT chk_idem CHECK (
    (idempotency_key IS NULL) = (idempotency_fingerprint IS NULL)
    AND (idempotency_key IS NULL OR (mode = 'async' AND status <> 'rejected'))),
  -- P-01: a rejected row never keeps input payload
  CONSTRAINT chk_p01 CHECK (status <> 'rejected' OR input_payload IS NULL),
  -- webhook (async terminal except rejected)
  CONSTRAINT chk_webhook CHECK (
    CASE WHEN mode = 'async' AND status IN ('done', 'failed', 'expired', 'cancelled') THEN webhook_status IS NOT NULL
         ELSE webhook_status IS NULL END),
  CONSTRAINT chk_dispatch_key CHECK (mode = 'async' OR dispatch_key IS NULL),
  CONSTRAINT chk_metadata_obj CHECK (jsonb_typeof(metadata) = 'object'),

  -- NULL list for bodies that cannot be parsed is empty (AC SHAA-193, design review round 1):
  -- downstream_pool_id, prioritas, prioritas_efektif and ttl_expires_at derive from endpoint/pool
  -- configuration, not from the body, so the pool columns are always NOT NULL; ttl follows mode
  -- (also on rejected async rows); metadata must be {} exactly on validasi_gagal/400 rows.
  CONSTRAINT chk_pool_kolom_not_null CHECK (downstream_pool_id IS NOT NULL AND prioritas IS NOT NULL AND prioritas_efektif IS NOT NULL),
  CONSTRAINT chk_null_body_tak_terbaca CHECK (
    NOT (reject_reason = 'validasi_gagal' AND reject_http_status = 400) OR metadata = '{}'::jsonb),
  CONSTRAINT chk_ttl_async CHECK ((mode = 'async') = (ttl_expires_at IS NOT NULL))
);

CREATE UNIQUE INDEX request_idempotency_uq ON request (client_id, idempotency_key) WHERE idempotency_key IS NOT NULL;
-- dispatcher: priority, then random key (K-05); only requests that may be sent now
CREATE INDEX request_dispatch_idx ON request (downstream_pool_id, prioritas_efektif, dispatch_key) WHERE status = 'queued';
-- sweepers (T-02d, T-02e) and K-20 mass actions
CREATE INDEX request_ttl_idx ON request (ttl_expires_at) WHERE status IN ('queued', 'dispatched');
CREATE INDEX request_menunggu_credential_idx ON request (credential_id) WHERE status IN ('queued', 'dispatched');
CREATE INDEX request_menunggu_client_idx ON request (client_id) WHERE status IN ('queued', 'dispatched');
CREATE INDEX request_menunggu_endpoint_idx ON request (downstream_endpoint_id) WHERE status IN ('queued', 'dispatched');
CREATE INDEX request_menunggu_dpool_idx ON request (downstream_pool_id) WHERE status IN ('queued', 'dispatched');
CREATE INDEX request_menunggu_upool_idx ON request (upstream_pool_id) WHERE status IN ('queued', 'dispatched');
CREATE INDEX request_menunggu_provider_idx ON request (provider_id) WHERE status IN ('queued', 'dispatched');
CREATE INDEX request_inflight_idx ON request (request_id) WHERE status = 'in_flight';
-- webhook outbox
CREATE INDEX request_webhook_idx ON request (completed_at) WHERE webhook_status = 'menunggu';
-- polling window, retention (8.4), P-03
CREATE INDEX request_completed_idx ON request (completed_at) WHERE completed_at IS NOT NULL;
CREATE INDEX request_client_received_idx ON request (client_id, received_at);

-- allowed transitions (3.2 diagram, T-05)
CREATE FUNCTION tg_request_transisi() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  IF TG_OP = 'INSERT' THEN
    IF NEW.status NOT IN ('rejected', 'queued', 'in_flight') THEN
      RAISE EXCEPTION 'status awal tidak sah: %', NEW.status USING ERRCODE = 'check_violation';
    END IF;
    RETURN NEW;
  END IF;
  IF NEW.status IS DISTINCT FROM OLD.status THEN
    IF NOT ((OLD.status, NEW.status) IN (
        ('queued', 'dispatched'), ('queued', 'expired'), ('queued', 'cancelled'), ('queued', 'failed'),
        ('dispatched', 'in_flight'), ('dispatched', 'cancelled'), ('dispatched', 'expired'),
        ('dispatched', 'failed'), ('dispatched', 'queued'),
        ('in_flight', 'queued'), ('in_flight', 'done'), ('in_flight', 'failed'))) THEN
      RAISE EXCEPTION 'transisi status tidak sah: % -> %', OLD.status, NEW.status USING ERRCODE = 'check_violation';
    END IF;
  END IF;
  -- terminal rows keep their outcome; only payload purge and webhook bookkeeping may change (T-05)
  IF OLD.status IN ('done', 'failed', 'rejected', 'expired', 'cancelled') AND (
       NEW.request_id <> OLD.request_id OR NEW.client_id <> OLD.client_id OR NEW.credential_id <> OLD.credential_id
       OR NEW.completed_at IS DISTINCT FROM OLD.completed_at OR NEW.attempt_count <> OLD.attempt_count
       OR NEW.final_kode_transaksi IS DISTINCT FROM OLD.final_kode_transaksi
       OR NEW.reject_reason IS DISTINCT FROM OLD.reject_reason OR NEW.cancel_reason IS DISTINCT FROM OLD.cancel_reason
       OR NEW.failure_reason IS DISTINCT FROM OLD.failure_reason OR NEW.cancelled_by IS DISTINCT FROM OLD.cancelled_by) THEN
    RAISE EXCEPTION 'status terminal tidak berubah (T-05)' USING ERRCODE = 'check_violation';
  END IF;
  IF NEW.request_id <> OLD.request_id OR NEW.client_id <> OLD.client_id OR NEW.received_at <> OLD.received_at
     OR NEW.mode <> OLD.mode THEN
    RAISE EXCEPTION 'kolom identitas request tidak boleh diubah' USING ERRCODE = 'check_violation';
  END IF;
  RETURN NEW;
END $$;
CREATE TRIGGER request_transisi BEFORE INSERT OR UPDATE ON request
  FOR EACH ROW EXECUTE FUNCTION tg_request_transisi();

-- ---------------------------------------------------------------- request_status_history (3.5)
CREATE TABLE request_status_history (
  request_id uuid NOT NULL REFERENCES request (request_id),
  seq integer NOT NULL CHECK (seq >= 1),
  status_dari text CHECK (status_dari IN ('queued', 'dispatched', 'in_flight', 'done', 'failed', 'rejected', 'expired', 'cancelled')),
  status_ke text NOT NULL CHECK (status_ke IN ('queued', 'dispatched', 'in_flight', 'done', 'failed', 'rejected', 'expired', 'cancelled')),
  alasan text,
  aktor text NOT NULL CHECK (aktor IN ('webserver', 'controller', 'worker', 'client')
    OR aktor ~ '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$'),
  objek_pemicu_id uuid,
  terjadi_at timestamptz NOT NULL DEFAULT now(),       -- transaction time: shared by one mass-cancel (EN-08)
  PRIMARY KEY (request_id, seq),
  CONSTRAINT chk_seq_pertama CHECK ((seq = 1) = (status_dari IS NULL)),
  CONSTRAINT chk_pemicu_k20 CHECK (
     (coalesce(alasan IN ('client_nonaktif', 'kredensial_nonaktif', 'endpoint_nonaktif', 'pool_nonaktif', 'provider_nonaktif'), false)
     OR (status_ke = 'failed' AND alasan = 'objek_nonaktif')) = (objek_pemicu_id IS NOT NULL))
);
CREATE INDEX request_status_history_pemicu_idx ON request_status_history (objek_pemicu_id, terjadi_at) WHERE objek_pemicu_id IS NOT NULL;
CREATE TRIGGER request_status_history_append_only BEFORE UPDATE OR DELETE ON request_status_history
  FOR EACH ROW EXECUTE FUNCTION tg_append_only();

-- T-04: every status written to request has a history row in the same transaction (checked at commit).
-- Deferred AFTER triggers see the committed-state table, so the match must use the event's own row images:
-- INSERT -> first row (seq = 1, status_dari NULL); UPDATE -> status_dari = OLD.status. (SHAA-245)
CREATE FUNCTION tg_request_punya_history() RETURNS trigger LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
BEGIN
  IF NOT EXISTS (SELECT 1 FROM request_status_history h
                   WHERE h.request_id = NEW.request_id
                     AND CASE WHEN TG_OP = 'INSERT' THEN h.seq = 1 AND h.status_dari IS NULL
                              ELSE h.status_dari = OLD.status END
                     AND h.status_ke = NEW.status AND h.terjadi_at >= transaction_timestamp()) THEN
    RAISE EXCEPTION 'request % berstatus % tanpa baris request_status_history yang cocok (T-04)', NEW.request_id, NEW.status
      USING ERRCODE = 'integrity_constraint_violation';
  END IF;
  RETURN NULL;
END $$;
CREATE CONSTRAINT TRIGGER request_history_wajib AFTER INSERT OR UPDATE OF status ON request
  DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION tg_request_punya_history();

-- ---------------------------------------------------------------- transaction_log (3.3)
CREATE TABLE transaction_log (
  kode_transaksi uuid PRIMARY KEY,
  request_id uuid NOT NULL REFERENCES request (request_id),
  attempt_no integer NOT NULL CHECK (attempt_no >= 1),
  provider_id uuid NOT NULL REFERENCES provider (provider_id),
  upstream_pool_id uuid NOT NULL REFERENCES upstream_pool (upstream_pool_id),
  client_id uuid NOT NULL REFERENCES client (client_id),
  downstream_endpoint_id uuid NOT NULL REFERENCES downstream_endpoint (downstream_endpoint_id),
  provider_config_version integer NOT NULL,
  endpoint_config_version integer NOT NULL,
  nik char(16) CHECK (nik ~ '^[0-9]{16}$'),
  requested_at timestamptz NOT NULL,
  responded_at timestamptz,
  duration_ms integer CHECK (duration_ms >= 0),
  status_akses text CHECK (status_akses IN ('berhasil', 'gagal_provider', 'gagal_jaringan', 'tidak_diketahui')),
  provider_http_status integer,
  provider_response_code varchar(64),
  error_class text CHECK (error_class IN ('timeout', 'koneksi', 'tls', 'respons_tidak_valid', 'auth_ditolak',
    'kuota_provider_habis', 'rate_limit_provider', 'provider_sementara', 'lainnya')),
  log_mode text NOT NULL CHECK (log_mode IN ('metadata_only', 'full_payload')),
  request_payload jsonb,
  response_payload jsonb,
  response_hash varchar(64) CHECK (response_hash ~ '^[0-9a-f]{64}$'),
  worker_id text NOT NULL,
  archived_at timestamptz,
  UNIQUE (request_id, attempt_no),
  -- R-05
  CONSTRAINT chk_payload_mode CHECK (log_mode = 'full_payload' OR (request_payload IS NULL AND response_payload IS NULL)),
  -- R-08: legal (status_akses, error_class) pairs; DQ-31 (e)
  CONSTRAINT chk_status_error CHECK (
    CASE WHEN status_akses IS NULL THEN error_class IS NULL
         WHEN status_akses IN ('berhasil', 'tidak_diketahui') THEN error_class IS NULL
          WHEN status_akses = 'gagal_jaringan' THEN error_class IS NOT NULL AND error_class IN ('timeout', 'koneksi', 'tls')
          WHEN status_akses = 'gagal_provider' THEN error_class IS NOT NULL AND error_class IN ('respons_tidak_valid', 'auth_ditolak',
           'kuota_provider_habis', 'rate_limit_provider', 'provider_sementara', 'lainnya')
    END),
  -- R-07
  CONSTRAINT chk_tls CHECK (error_class IS DISTINCT FROM 'tls' OR (
    responded_at IS NULL AND provider_http_status IS NULL AND provider_response_code IS NULL
    AND response_payload IS NULL AND response_hash IS NULL)),
  CONSTRAINT chk_responded CHECK (responded_at IS NULL OR responded_at >= requested_at)
);
-- DQ-34 (d) rolling window and DQ-05 (v0.7.8 comment on this issue)
CREATE INDEX transaction_log_pool_requested_idx ON transaction_log (upstream_pool_id, requested_at);
CREATE INDEX transaction_log_client_requested_idx ON transaction_log (client_id, requested_at);
CREATE INDEX transaction_log_requested_idx ON transaction_log (requested_at, kode_transaksi);   -- export order E-04, retention
CREATE INDEX transaction_log_pending_idx ON transaction_log (requested_at) WHERE status_akses IS NULL;   -- reconciler R-02
CREATE INDEX transaction_log_nik_idx ON transaction_log (nik);   -- auditor NIK lookup (US-14), auditor function only

ALTER TABLE request ADD CONSTRAINT request_final_log_fk
  FOREIGN KEY (final_kode_transaksi) REFERENCES transaction_log (kode_transaksi) DEFERRABLE INITIALLY DEFERRED;

CREATE FUNCTION tg_log_insert() RETURNS trigger LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
DECLARE
  v_wajib boolean;
BEGIN
  SELECT wajib_log_nik INTO v_wajib FROM provider WHERE provider_id = NEW.provider_id;
  IF v_wajib AND NEW.nik IS NULL THEN
    RAISE EXCEPTION 'nik wajib untuk provider ini (C-01)' USING ERRCODE = 'not_null_violation';
  END IF;
  RETURN NEW;
END $$;
CREATE TRIGGER transaction_log_insert BEFORE INSERT ON transaction_log
  FOR EACH ROW EXECUTE FUNCTION tg_log_insert();

-- R-03: once complete, a row only gets archived_at; while pending only the result columns may be filled.
CREATE FUNCTION tg_log_update() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  IF OLD.status_akses IS NOT NULL THEN
    IF (to_jsonb(NEW) - 'archived_at') IS DISTINCT FROM (to_jsonb(OLD) - 'archived_at') THEN
      RAISE EXCEPTION 'baris transaction_log yang lengkap tidak berubah (R-03)' USING ERRCODE = 'integrity_constraint_violation';
    END IF;
  ELSIF NEW.kode_transaksi <> OLD.kode_transaksi OR NEW.request_id <> OLD.request_id OR NEW.attempt_no <> OLD.attempt_no
        OR NEW.nik IS DISTINCT FROM OLD.nik OR NEW.requested_at <> OLD.requested_at
        OR NEW.upstream_pool_id <> OLD.upstream_pool_id OR NEW.provider_id <> OLD.provider_id THEN
    RAISE EXCEPTION 'kolom identitas transaction_log tidak boleh diubah (R-03)' USING ERRCODE = 'integrity_constraint_violation';
  END IF;
  RETURN NEW;
END $$;
CREATE TRIGGER transaction_log_update BEFORE UPDATE ON transaction_log
  FOR EACH ROW EXECUTE FUNCTION tg_log_update();
CREATE TRIGGER transaction_log_no_delete BEFORE DELETE ON transaction_log
  FOR EACH ROW EXECUTE FUNCTION tg_append_only();

-- ---------------------------------------------------------------- upstream_attempt_daily (KP-06)
CREATE TABLE upstream_attempt_daily (
  upstream_pool_id uuid NOT NULL REFERENCES upstream_pool (upstream_pool_id),
  periode_mulai timestamptz NOT NULL,
  periode_selesai timestamptz NOT NULL,
  periode_transisi boolean NOT NULL DEFAULT false,
  tanggal date NOT NULL,                               -- label only, never a key
  jumlah bigint NOT NULL CHECK (jumlah >= 0),
  snapshot_at timestamptz NOT NULL,
  PRIMARY KEY (upstream_pool_id, periode_mulai),
  CONSTRAINT chk_periode CHECK (periode_selesai > periode_mulai),
  -- periods of one pool do not overlap
  CONSTRAINT upstream_attempt_daily_tanpa_tumpang_tindih
    EXCLUDE USING gist (upstream_pool_id WITH =, tstzrange(periode_mulai, periode_selesai) WITH &&)
);

-- ---------------------------------------------------------------- webhook_log (3.6)
CREATE TABLE webhook_log (
  webhook_delivery_id uuid PRIMARY KEY,
  request_id uuid NOT NULL REFERENCES request (request_id),
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
  UNIQUE (request_id, attempt_no),
  CONSTRAINT chk_secret_versi CHECK ((hasil = 'ditolak_allowlist') = (secret_versi IS NULL) OR hasil = 'gagal_internal'),
  CONSTRAINT chk_allowlist_http CHECK (hasil <> 'ditolak_allowlist' OR http_status IS NULL)
);
CREATE INDEX webhook_log_client_idx ON webhook_log (client_id, sent_at);
CREATE TRIGGER webhook_log_no_update BEFORE UPDATE ON webhook_log
  FOR EACH ROW EXECUTE FUNCTION tg_immutable();

-- ---------------------------------------------------------------- audit_akses (8.2)
CREATE TABLE audit_akses (
  audit_id uuid PRIMARY KEY,
  pengguna_id uuid NOT NULL,                           -- Web UI user id (admin_user) or client id for API; no FK (outlives users)
  peran text NOT NULL,
  aksi text NOT NULL CHECK (aksi IN ('lihat_log', 'cari_log', 'cari_nik', 'lihat_payload', 'ekspor', 'lihat_audit',
    'restore_arsip', 'ubah_peran', 'akses_ditolak')),
  tampilan_nik text NOT NULL CHECK (tampilan_nik IN ('utuh', 'tersamar', 'tidak_ada')),
  filter_kueri jsonb,
  nik_hmac varchar(64) CHECK (nik_hmac ~ '^[0-9a-f]{64}$'),
  nik_tersamar varchar(16) CHECK (nik_tersamar ~ '^\*{12}[0-9]{4}$'),
  target_request_id uuid,
  target_kode_transaksi uuid,
  jumlah_record integer NOT NULL DEFAULT 0 CHECK (jumlah_record >= 0),
  ekspor_checksum varchar(64),
  atas_permintaan_pengguna_id uuid,
  alasan varchar(500),
  hasil text NOT NULL CHECK (hasil IN ('diizinkan', 'ditolak')),
  sumber_ip varchar(45) NOT NULL,
  sesi_id varchar(64) NOT NULL,
  terjadi_at timestamptz NOT NULL DEFAULT now(),
  CONSTRAINT chk_alasan_wajib CHECK (
    (tampilan_nik <> 'utuh' AND aksi NOT IN ('lihat_payload', 'ekspor', 'restore_arsip')) OR length(btrim(coalesce(alasan, ''))) > 0),
  CONSTRAINT chk_filter_wajib CHECK (
    aksi NOT IN ('cari_log', 'cari_nik', 'lihat_log', 'ekspor', 'lihat_audit', 'akses_ditolak') OR filter_kueri IS NOT NULL),
  -- E-01..E-03, DQ-35 (a)(b)
  CONSTRAINT chk_ekspor_checksum CHECK (
    CASE WHEN aksi = 'ekspor' AND hasil = 'diizinkan' THEN ekspor_checksum IS NOT NULL AND ekspor_checksum ~ '^[0-9a-f]{64}$' AND nik_hmac IS NOT NULL
         ELSE ekspor_checksum IS NULL END)
);
CREATE INDEX audit_akses_terjadi_idx ON audit_akses (terjadi_at);
CREATE INDEX audit_akses_pengguna_idx ON audit_akses (pengguna_id, terjadi_at);
CREATE INDEX audit_akses_aksi_idx ON audit_akses (aksi, terjadi_at);
CREATE TRIGGER audit_akses_append_only BEFORE UPDATE OR DELETE ON audit_akses
  FOR EACH ROW EXECUTE FUNCTION tg_append_only();
CREATE TRIGGER audit_akses_no_truncate BEFORE TRUNCATE ON audit_akses
  FOR EACH STATEMENT EXECUTE FUNCTION tg_append_only();

-- ---------------------------------------------------------------- audit_login (ADR-0003 3.6)
-- Admin login/logout trail, deliberately separate from audit_akses: audit_akses tracks NIK data access and its
-- pengguna_id is NOT NULL, but a failed login for an unknown username has no user. The typed username is never
-- stored (it may hold a mistyped password or personal data); passwords are never stored. Limited, append-only,
-- INSERT only for pr_admin_api; only an auditor reads it, through a SECURITY DEFINER function that writes an
-- audit_akses lihat_audit row. Same retention as audit_akses.
CREATE TABLE audit_login (
  audit_id uuid PRIMARY KEY,
  pengguna_id uuid,                                    -- NULL when the username is not recognised
  aksi text NOT NULL CHECK (aksi IN ('login', 'logout')),
  hasil text NOT NULL CHECK (hasil IN ('berhasil', 'gagal', 'terkunci')),
  sesi_id varchar(64),
  sumber_ip varchar(45) NOT NULL,
  user_agent varchar(512),
  terjadi_at timestamptz NOT NULL DEFAULT now(),
  -- a successful login and any logout belong to a session; a failed or locked-out attempt does not
  CONSTRAINT chk_sesi_wajib CHECK (((aksi = 'logout' OR hasil = 'berhasil') AND sesi_id IS NOT NULL)
                                    OR (aksi = 'login' AND hasil IN ('gagal', 'terkunci') AND sesi_id IS NULL))
);
CREATE INDEX audit_login_terjadi_idx ON audit_login (terjadi_at);
CREATE INDEX audit_login_pengguna_idx ON audit_login (pengguna_id, terjadi_at);
CREATE TRIGGER audit_login_append_only BEFORE UPDATE OR DELETE ON audit_login
  FOR EACH ROW EXECUTE FUNCTION tg_append_only();
CREATE TRIGGER audit_login_no_truncate BEFORE TRUNCATE ON audit_login
  FOR EACH STATEMENT EXECUTE FUNCTION tg_append_only();

-- ---------------------------------------------------------------- data balikan (5, F1 jobs; tables exist so H-01/H-02 can be written)
CREATE TABLE data_balikan_batch (
  batch_id uuid PRIMARY KEY,
  provider_id uuid NOT NULL REFERENCES provider (provider_id),
  pemicu text NOT NULL CHECK (pemicu IN ('jumlah_n', 'periode')),
  periode_mulai_at timestamptz NOT NULL,
  periode_selesai_at timestamptz NOT NULL,
  jumlah_item integer NOT NULL CHECK (jumlah_item >= 0),
  status text NOT NULL CHECK (status IN ('disusun', 'dikirim', 'diterima', 'gagal', 'gagal_permanen')),
  jadwal_kirim_at timestamptz NOT NULL,
  attempt_count integer NOT NULL DEFAULT 0,
  format_version varchar(50) NOT NULL DEFAULT 'belum_ditetapkan',
  payload_hash varchar(64) CHECK (payload_hash ~ '^[0-9a-f]{64}$'),
  provider_ack_ref varchar(200),
  disusun_at timestamptz, dikirim_at timestamptz, diterima_at timestamptz,
  error_terakhir varchar(500),
  CONSTRAINT chk_hash_setelah_kirim CHECK (status IN ('disusun') OR payload_hash IS NOT NULL OR status IN ('gagal', 'gagal_permanen'))
);
CREATE TABLE data_balikan_item (
  batch_id uuid NOT NULL REFERENCES data_balikan_batch (batch_id),
  request_id uuid NOT NULL REFERENCES request (request_id),
  kode_transaksi uuid NOT NULL REFERENCES transaction_log (kode_transaksi),
  nik char(16) NOT NULL CHECK (nik ~ '^[0-9]{16}$'),
  field_input jsonb NOT NULL,
  field_metadata jsonb NOT NULL,
  requested_at timestamptz NOT NULL,
  PRIMARY KEY (batch_id, request_id)
);
CREATE INDEX data_balikan_item_request_idx ON data_balikan_item (request_id);
