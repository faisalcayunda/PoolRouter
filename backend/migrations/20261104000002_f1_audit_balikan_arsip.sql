-- F1 schema, part 2: audit_akses extension, data balikan (D4), archive and restore (D6) (SHAA-333).
-- Source: data contract v0.9.5-final F1-5, F1-6, F1-7.4 (audit values). Additive; rollback in backend/db/rollback/.

-- ---------------------------------------------------------------- audit_akses (F1-6.4)
-- Append-only table: new columns carry constant defaults or NULL, so no existing row is rewritten (DQ-21 unaffected).
ALTER TABLE audit_akses
  ADD COLUMN sumber text NOT NULL DEFAULT 'online' CONSTRAINT chk_audit_sumber CHECK (sumber IN ('online', 'arsip')),
  ADD COLUMN target_pengguna_id uuid,
  ADD COLUMN pelaku_kedua_id uuid,
  ADD COLUMN target_restore_id uuid,
  ADD COLUMN kunci_hmac_versi integer CHECK (kunci_hmac_versi >= 1);

ALTER TABLE audit_akses DROP CONSTRAINT audit_akses_aksi_check;
ALTER TABLE audit_akses ADD CONSTRAINT chk_aksi CHECK (aksi IN ('lihat_log', 'cari_log', 'cari_nik', 'lihat_payload', 'ekspor',
  'lihat_audit', 'restore_arsip', 'ubah_peran', 'akses_ditolak',
  'minta_restore', 'terbit_kode_daftar', 'daftar_mfa', 'bootstrap_mfa', 'pakai_kode_pemulihan', 'reset_mfa', 'reset_mfa_darurat'));

-- A reason is required wherever the contract says so (minta_restore, terbit_kode_daftar, reset_mfa*, DQ-50 d).
ALTER TABLE audit_akses DROP CONSTRAINT chk_alasan_wajib;
ALTER TABLE audit_akses ADD CONSTRAINT chk_alasan_wajib CHECK (
  (tampilan_nik <> 'utuh' AND aksi NOT IN ('lihat_payload', 'ekspor', 'restore_arsip', 'minta_restore', 'terbit_kode_daftar',
                                           'reset_mfa', 'reset_mfa_darurat'))
  OR length(btrim(coalesce(alasan, ''))) > 0);

ALTER TABLE audit_akses
  ADD CONSTRAINT chk_audit_target_pengguna CHECK (
    (aksi IN ('terbit_kode_daftar', 'daftar_mfa', 'bootstrap_mfa', 'reset_mfa', 'reset_mfa_darurat')) = (target_pengguna_id IS NOT NULL)),
  ADD CONSTRAINT chk_audit_pelaku_kedua CHECK (
    (aksi = 'reset_mfa_darurat') = (pelaku_kedua_id IS NOT NULL) AND pelaku_kedua_id IS DISTINCT FROM pengguna_id),
  ADD CONSTRAINT chk_audit_target_restore CHECK (
    (aksi IN ('minta_restore', 'restore_arsip')) = (target_restore_id IS NOT NULL)),
  ADD CONSTRAINT chk_audit_sumber_arsip CHECK (sumber = 'online' OR target_restore_id IS NOT NULL),                       -- DQ-49 (d)
  ADD CONSTRAINT chk_audit_reset_beda CHECK (aksi <> 'reset_mfa' OR target_pengguna_id <> pengguna_id),                    -- DQ-50 (d)
  ADD CONSTRAINT chk_audit_penerbit_beda CHECK (aksi <> 'terbit_kode_daftar' OR target_pengguna_id <> pengguna_id),        -- DQ-50 (d)
  ADD CONSTRAINT chk_audit_kode_id CHECK (
    aksi NOT IN ('terbit_kode_daftar', 'pakai_kode_pemulihan') OR coalesce(filter_kueri ? 'kode_id', false));              -- MF-02, MF-10
CREATE INDEX audit_akses_target_pengguna_idx ON audit_akses (target_pengguna_id, terjadi_at) WHERE target_pengguna_id IS NOT NULL;
CREATE INDEX audit_akses_kode_idx ON audit_akses ((filter_kueri ->> 'kode_id')) WHERE aksi IN ('terbit_kode_daftar', 'pakai_kode_pemulihan');

-- ---------------------------------------------------------------- D4: data balikan (DB-01..DB-06)
-- F0 provider_config_versi accepted config_schema_version = 1 only. Schema 2 carries the data_balikan block (F1-5.3);
-- its fields are validated by the admin API (V-05, 422), not here. Existing version-1 documents stay valid.
ALTER TABLE provider_config_versi DROP CONSTRAINT provider_config_versi_config_schema_version_check;
ALTER TABLE provider_config_versi ADD CONSTRAINT chk_config_schema_version CHECK (config_schema_version IN (1, 2));

ALTER TABLE data_balikan_batch
  ADD COLUMN kirim_berikutnya_at timestamptz,
  ADD COLUMN config_ref_version integer;
-- F0 never composed a batch (the job is F1), so no row is expected; if one exists it is tied to the provider's active version.
UPDATE data_balikan_batch b SET config_ref_version = coalesce((SELECT p.config_versi_aktif FROM provider p WHERE p.provider_id = b.provider_id), 1)
 WHERE config_ref_version IS NULL;
ALTER TABLE data_balikan_batch
  ALTER COLUMN config_ref_version SET NOT NULL,
  ADD CONSTRAINT chk_config_ref_version CHECK (config_ref_version >= 1),
  ADD CONSTRAINT chk_kirim_berikutnya CHECK ((status = 'gagal') = (kirim_berikutnya_at IS NOT NULL));                    -- DQ-47 (b)
CREATE INDEX data_balikan_batch_kirim_idx ON data_balikan_batch (kirim_berikutnya_at) WHERE status = 'gagal';

-- DB-01: one active item per request, enforced by the database (reference form from the contract).
ALTER TABLE data_balikan_item ADD COLUMN aktif boolean NOT NULL DEFAULT true;
CREATE UNIQUE INDEX data_balikan_item_aktif_uq ON data_balikan_item (request_id) WHERE aktif;

-- DQ-47 (a): an item is active exactly while its batch is not gagal_permanen. The batch transition deactivates its
-- items in the same transaction (SECURITY DEFINER, so the job role needs no extra rights); items cannot be
-- (de)activated against their batch's state.
CREATE FUNCTION tg_balikan_batch_gagal_permanen() RETURNS trigger LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
BEGIN
  UPDATE data_balikan_item SET aktif = false WHERE batch_id = NEW.batch_id AND aktif;
  RETURN NULL;
END $$;
CREATE TRIGGER data_balikan_batch_gagal_permanen AFTER UPDATE OF status ON data_balikan_batch
  FOR EACH ROW WHEN (NEW.status = 'gagal_permanen' AND OLD.status IS DISTINCT FROM 'gagal_permanen')
  EXECUTE FUNCTION tg_balikan_batch_gagal_permanen();

CREATE FUNCTION tg_balikan_item_sesuai_batch() RETURNS trigger LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
DECLARE
  v_status text;
BEGIN
  SELECT status INTO v_status FROM data_balikan_batch WHERE batch_id = NEW.batch_id;
  IF NEW.aktif = (v_status = 'gagal_permanen') THEN
    RAISE EXCEPTION 'data_balikan_item.aktif harus false tepat bila batch gagal_permanen (batch %, status %)', NEW.batch_id, v_status
      USING ERRCODE = 'check_violation';
  END IF;
  RETURN NEW;
END $$;
CREATE TRIGGER data_balikan_item_sesuai_batch BEFORE INSERT OR UPDATE OF aktif ON data_balikan_item
  FOR EACH ROW EXECUTE FUNCTION tg_balikan_item_sesuai_batch();

-- ---------------------------------------------------------------- D6: archive manifest (F1-6.2)
CREATE TABLE arsip_manifest (
  manifest_id uuid PRIMARY KEY,                                      -- UUIDv7, application generated
  tabel_sumber text NOT NULL CHECK (tabel_sumber IN ('transaction_log', 'request', 'request_status_history', 'webhook_log', 'audit_akses')),
  provider_id uuid REFERENCES provider (provider_id),
  tanggal_wib date NOT NULL,
  objek_kunci varchar(512) NOT NULL UNIQUE,
  format varchar(32) NOT NULL,
  versi_contract varchar(16) NOT NULL,
  jumlah_baris bigint NOT NULL CHECK (jumlah_baris >= 0),
  jumlah_dikecualikan_h02 integer NOT NULL DEFAULT 0 CHECK (jumlah_dikecualikan_h02 >= 0),
  kode_transaksi_min uuid,
  kode_transaksi_maks uuid,
  sha256 char(64) NOT NULL CHECK (sha256 ~ '^[0-9a-f]{64}$'),       -- every status is after the object was written
  ukuran_byte bigint NOT NULL CHECK (ukuran_byte >= 0),
  kunci_hmac_versi integer CHECK (kunci_hmac_versi >= 1),
  nik_mode text CHECK (nik_mode IN ('plaintext', 'enkripsi_aplikasi')),
  payload_mode text CHECK (payload_mode IN ('plaintext', 'enkripsi_aplikasi')),
  nik_kek_id varchar(100),                                           -- KEK identifier, never key material
  enkripsi_format_versi integer CHECK (enkripsi_format_versi >= 1),
  dek_terbungkus bytea,                                              -- Rahasia: DEK wrapped by the archive KEK
  status text NOT NULL CHECK (status IN ('ditulis', 'terverifikasi', 'online_dihapus', 'gagal_verifikasi', 'objek_dihapus')),
  legal_hold boolean NOT NULL DEFAULT false,
  ditulis_at timestamptz, diverifikasi_at timestamptz, online_dihapus_at timestamptz, objek_dihapus_at timestamptz,
  CONSTRAINT chk_manifest_provider CHECK ((tabel_sumber = 'audit_akses') = (provider_id IS NULL)),
  CONSTRAINT chk_manifest_h02 CHECK (jumlah_dikecualikan_h02 <= jumlah_baris),
  CONSTRAINT chk_manifest_rentang_kode CHECK (
    (tabel_sumber = 'transaction_log') = (kode_transaksi_min IS NOT NULL AND kode_transaksi_maks IS NOT NULL)
    AND (kode_transaksi_min IS NULL) = (kode_transaksi_maks IS NULL)
    AND (kode_transaksi_min IS NULL OR kode_transaksi_min <= kode_transaksi_maks)),
  CONSTRAINT chk_manifest_log_kolom CHECK (                           -- DQ-48 (c)
    tabel_sumber <> 'transaction_log' OR (kunci_hmac_versi IS NOT NULL AND nik_mode IS NOT NULL AND payload_mode IS NOT NULL)),
  CONSTRAINT chk_manifest_request_payload CHECK (tabel_sumber <> 'request' OR payload_mode IS NOT NULL),
  CONSTRAINT chk_manifest_enkripsi CHECK (                            -- DQ-48 (c)
    NOT (coalesce(nik_mode = 'enkripsi_aplikasi', false) OR coalesce(payload_mode = 'enkripsi_aplikasi', false))
    OR (nik_kek_id IS NOT NULL AND enkripsi_format_versi IS NOT NULL AND dek_terbungkus IS NOT NULL)),
  CONSTRAINT chk_manifest_waktu CHECK (
    (status NOT IN ('terverifikasi', 'online_dihapus') OR diverifikasi_at IS NOT NULL)
    AND (status <> 'online_dihapus' OR online_dihapus_at IS NOT NULL)
    AND (status <> 'objek_dihapus' OR objek_dihapus_at IS NOT NULL))
);
-- Unique per unit; NULL provider_id (audit_akses) counts as a value (contract 6.2).
CREATE UNIQUE INDEX arsip_manifest_satuan_uq ON arsip_manifest (tabel_sumber, provider_id, tanggal_wib) NULLS NOT DISTINCT;
CREATE INDEX arsip_manifest_status_idx ON arsip_manifest (status, tanggal_wib);

-- ---------------------------------------------------------------- D6: restore requests (F1-6.3)
CREATE TABLE arsip_restore_permintaan (
  restore_id uuid PRIMARY KEY,
  diminta_oleh uuid NOT NULL,                                        -- auditor; no FK, the row outlives users like audit_akses
  audit_id uuid NOT NULL,                                            -- existence and content checked by arsip_restore_audit_wajib (no FK: it would make audit_akses un-TRUNCATE-able with a different error)
  provider_id uuid NOT NULL REFERENCES provider (provider_id),
  rentang_mulai date NOT NULL,
  rentang_selesai date NOT NULL,
  nik_hmac_per_versi jsonb,                                          -- [{"versi": n, "hmac": "..."}], pseudonyms only
  nik_tersamar char(4) CHECK (nik_tersamar ~ '^[0-9]{4}$'),
  alasan varchar(500) NOT NULL CHECK (length(btrim(alasan)) > 0),
  status text NOT NULL CHECK (status IN ('antre', 'berjalan', 'siap', 'gagal', 'kedaluwarsa')),
  diminta_at timestamptz NOT NULL DEFAULT now(),
  mulai_at timestamptz, siap_at timestamptz, kedaluwarsa_at timestamptz,
  jumlah_baris_pulih bigint CHECK (jumlah_baris_pulih >= 0),
  error_terakhir varchar(200),
  CONSTRAINT chk_restore_rentang CHECK (rentang_selesai >= rentang_mulai),
  CONSTRAINT chk_restore_hmac_bentuk CHECK (nik_hmac_per_versi IS NULL OR jsonb_typeof(nik_hmac_per_versi) = 'array'),
  CONSTRAINT chk_restore_siap CHECK (status <> 'siap' OR (siap_at IS NOT NULL AND kedaluwarsa_at IS NOT NULL)),
  CONSTRAINT chk_restore_berjalan CHECK (status NOT IN ('berjalan', 'siap') OR mulai_at IS NOT NULL),
  -- AR-07 / DQ-49 (c): pseudonyms are emptied once the request expires.
  CONSTRAINT chk_restore_kedaluwarsa CHECK (status <> 'kedaluwarsa' OR (nik_hmac_per_versi IS NULL AND nik_tersamar IS NULL))
);
CREATE INDEX arsip_restore_status_idx ON arsip_restore_permintaan (status, kedaluwarsa_at);
CREATE INDEX arsip_restore_audit_idx ON arsip_restore_permintaan (audit_id);

-- AR-05 (range) and DQ-49 (a): with an HMAC the range is capped in months, without one in days (platform settings);
-- the matching audit row (aksi = minta_restore, same restore_id, with a reason) must exist at commit.
CREATE FUNCTION tg_restore_rentang() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  IF NEW.nik_hmac_per_versi IS NOT NULL THEN
    IF NEW.rentang_selesai > (NEW.rentang_mulai + make_interval(months => fn_konfigurasi_int('restore_rentang_dengan_nik_maks_bulan', 12)))::date THEN
      RAISE EXCEPTION 'rentang restore dengan NIK melebihi % bulan (AR-05)', fn_konfigurasi_int('restore_rentang_dengan_nik_maks_bulan', 12)
        USING ERRCODE = 'check_violation';
    END IF;
  ELSIF NEW.status <> 'kedaluwarsa' AND NEW.rentang_selesai - NEW.rentang_mulai + 1 > fn_konfigurasi_int('restore_rentang_tanpa_nik_maks_hari', 7) THEN
    RAISE EXCEPTION 'rentang restore tanpa NIK melebihi % hari (AR-05)', fn_konfigurasi_int('restore_rentang_tanpa_nik_maks_hari', 7)
      USING ERRCODE = 'check_violation';
  END IF;
  RETURN NEW;
END $$;
CREATE TRIGGER arsip_restore_rentang BEFORE INSERT ON arsip_restore_permintaan
  FOR EACH ROW EXECUTE FUNCTION tg_restore_rentang();

CREATE FUNCTION tg_restore_punya_audit() RETURNS trigger LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
BEGIN
  IF NOT EXISTS (SELECT 1 FROM audit_akses a WHERE a.audit_id = NEW.audit_id AND a.aksi = 'minta_restore'
                    AND a.target_restore_id = NEW.restore_id AND a.pengguna_id = NEW.diminta_oleh
                    AND length(btrim(coalesce(a.alasan, ''))) > 0) THEN
    RAISE EXCEPTION 'permintaan restore % tanpa baris audit_akses minta_restore yang cocok (DQ-49 a)', NEW.restore_id
      USING ERRCODE = 'integrity_constraint_violation';
  END IF;
  RETURN NULL;
END $$;
CREATE CONSTRAINT TRIGGER arsip_restore_audit_wajib AFTER INSERT ON arsip_restore_permintaan
  DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION tg_restore_punya_audit();

-- Identity of a request never changes; only status, times and counters do.
CREATE FUNCTION tg_restore_identitas() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  IF (NEW.restore_id, NEW.diminta_oleh, NEW.audit_id, NEW.provider_id, NEW.rentang_mulai, NEW.rentang_selesai, NEW.alasan, NEW.diminta_at)
     IS DISTINCT FROM (OLD.restore_id, OLD.diminta_oleh, OLD.audit_id, OLD.provider_id, OLD.rentang_mulai, OLD.rentang_selesai, OLD.alasan, OLD.diminta_at)
     OR (NEW.nik_hmac_per_versi IS NOT NULL AND NEW.nik_hmac_per_versi IS DISTINCT FROM OLD.nik_hmac_per_versi)
     OR (OLD.status = 'kedaluwarsa' AND NEW.status <> 'kedaluwarsa') THEN
    RAISE EXCEPTION 'identitas permintaan restore tidak boleh diubah' USING ERRCODE = 'integrity_constraint_violation';
  END IF;
  RETURN NEW;
END $$;
CREATE TRIGGER arsip_restore_identitas BEFORE UPDATE ON arsip_restore_permintaan
  FOR EACH ROW EXECUTE FUNCTION tg_restore_identitas();

-- ---------------------------------------------------------------- D6: arsip_pulih (F1-6.3 AR-06)
-- Temporary copy of restored transaction_log rows (Rahasia, at most 14 days). Same columns and CHECKs as the online table
-- at this migration, plus restore_id. nik follows the online type as of now; an R-24b change to transaction_log.nik
-- must be repeated here (contract AR-06).
CREATE SCHEMA arsip_pulih;
CREATE TABLE arsip_pulih.transaction_log (LIKE public.transaction_log INCLUDING DEFAULTS INCLUDING CONSTRAINTS);
ALTER TABLE arsip_pulih.transaction_log
  ADD COLUMN restore_id uuid NOT NULL REFERENCES public.arsip_restore_permintaan (restore_id),
  ADD PRIMARY KEY (restore_id, kode_transaksi);
CREATE INDEX arsip_pulih_requested_idx ON arsip_pulih.transaction_log (restore_id, requested_at);
REVOKE ALL ON SCHEMA arsip_pulih FROM PUBLIC;
REVOKE ALL ON ALL TABLES IN SCHEMA arsip_pulih FROM PUBLIC;

-- AR-07: expiry removes the restored rows and empties the request pseudonyms in one transaction. This is the only
-- deletion path pr_restore gets that uses a WHERE (a column-filtered DELETE would need SELECT on arsip_pulih).
CREATE FUNCTION fn_arsip_pulih_kedaluwarsa(p_restore_id uuid) RETURNS bigint
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
DECLARE
  v_hapus bigint;
BEGIN
  UPDATE arsip_restore_permintaan SET status = 'kedaluwarsa', nik_hmac_per_versi = NULL, nik_tersamar = NULL
   WHERE restore_id = p_restore_id AND ((status = 'siap' AND kedaluwarsa_at <= now()) OR status = 'gagal');
  IF NOT FOUND THEN
    RETURN 0;
  END IF;
  DELETE FROM arsip_pulih.transaction_log WHERE restore_id = p_restore_id;
  GET DIAGNOSTICS v_hapus = ROW_COUNT;
  RETURN v_hapus;
END $$;
REVOKE ALL ON FUNCTION fn_arsip_pulih_kedaluwarsa(uuid) FROM PUBLIC;
