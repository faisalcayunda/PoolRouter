-- F1 schema, part 3: MFA TOTP (D7) (SHAA-333).
-- Source: data contract v0.9.5-final F1-7 (MF-01..MF-10). Secrets are envelope-encrypted by the application; the
-- database only stores ciphertext and hashes. Rollback in backend/db/rollback/.

-- sesi in the contract is admin_session in F0.
ALTER TABLE admin_session ADD COLUMN mfa_terverifikasi_at timestamptz;

CREATE TABLE pengguna_mfa (
  pengguna_id uuid PRIMARY KEY REFERENCES admin_user (pengguna_id),                       -- MF-01: one row per user
  secret_ciphertext bytea NOT NULL,
  secret_dek_terbungkus bytea NOT NULL,
  secret_nonce bytea NOT NULL,
  kek_id varchar(100) NOT NULL,
  status text NOT NULL CHECK (status IN ('menunggu_konfirmasi', 'aktif')),
  langkah_terakhir bigint NOT NULL DEFAULT 0 CHECK (langkah_terakhir >= 0),
  didaftarkan_at timestamptz NOT NULL DEFAULT now(),
  dikonfirmasi_at timestamptz,
  CONSTRAINT chk_mfa_konfirmasi CHECK ((status = 'aktif') = (dikonfirmasi_at IS NOT NULL))
);
CREATE INDEX pengguna_mfa_menunggu_idx ON pengguna_mfa (didaftarkan_at) WHERE status = 'menunggu_konfirmasi';   -- MF-01 cleanup, DQ-50 (f)

-- MF-03: the last accepted step never goes down.
CREATE FUNCTION tg_mfa_langkah_naik() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  IF NEW.langkah_terakhir < OLD.langkah_terakhir THEN
    RAISE EXCEPTION 'langkah_terakhir tidak boleh turun (MF-03)' USING ERRCODE = 'check_violation';
  END IF;
  IF NEW.pengguna_id <> OLD.pengguna_id THEN
    RAISE EXCEPTION 'pengguna_id tidak boleh diubah' USING ERRCODE = 'check_violation';
  END IF;
  RETURN NEW;
END $$;
CREATE TRIGGER pengguna_mfa_langkah_naik BEFORE UPDATE ON pengguna_mfa
  FOR EACH ROW EXECUTE FUNCTION tg_mfa_langkah_naik();

CREATE TABLE kode_pemulihan_mfa (
  kode_id uuid PRIMARY KEY,
  pengguna_id uuid NOT NULL REFERENCES admin_user (pengguna_id),
  kode_hash varchar(255) NOT NULL CHECK (kode_hash LIKE '$argon2id$%'),                    -- PHC string; the code itself is never stored
  dibuat_at timestamptz NOT NULL DEFAULT now(),
  dipakai_at timestamptz
);
CREATE INDEX kode_pemulihan_mfa_pengguna_idx ON kode_pemulihan_mfa (pengguna_id) WHERE dipakai_at IS NULL;

-- MF-02: single use. Only dipakai_at may change, and only from NULL.
CREATE FUNCTION tg_kode_pemulihan_sekali() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  IF (NEW.kode_id, NEW.pengguna_id, NEW.kode_hash, NEW.dibuat_at) IS DISTINCT FROM (OLD.kode_id, OLD.pengguna_id, OLD.kode_hash, OLD.dibuat_at)
     OR OLD.dipakai_at IS NOT NULL THEN
    RAISE EXCEPTION 'kode pemulihan sekali pakai dan tidak dapat diubah (MF-02)' USING ERRCODE = 'integrity_constraint_violation';
  END IF;
  RETURN NEW;
END $$;
CREATE TRIGGER kode_pemulihan_mfa_sekali BEFORE UPDATE ON kode_pemulihan_mfa
  FOR EACH ROW EXECUTE FUNCTION tg_kode_pemulihan_sekali();

CREATE TABLE kode_daftar_mfa (
  kode_id uuid PRIMARY KEY,
  pengguna_id uuid NOT NULL REFERENCES admin_user (pengguna_id),
  kode_hash char(64) NOT NULL CHECK (kode_hash ~ '^[0-9a-f]{64}$'),                        -- SHA-256 hex of the 128-bit code
  diterbitkan_oleh uuid NOT NULL REFERENCES admin_user (pengguna_id),
  diterbitkan_at timestamptz NOT NULL DEFAULT now(),
  kedaluwarsa_at timestamptz NOT NULL,
  gagal_count integer NOT NULL DEFAULT 0 CHECK (gagal_count BETWEEN 0 AND 5),
  dipakai_at timestamptz,
  gugur_at timestamptz,
  gugur_alasan text CHECK (gugur_alasan IN ('gagal_5x', 'diganti')),
  CONSTRAINT chk_kode_daftar_penerbit CHECK (diterbitkan_oleh <> pengguna_id),             -- MF-10 (ii)
  CONSTRAINT chk_kode_daftar_umur CHECK (kedaluwarsa_at - diterbitkan_at = interval '24 hours'),
  CONSTRAINT chk_kode_daftar_gugur CHECK ((gugur_at IS NULL) = (gugur_alasan IS NULL)),
  CONSTRAINT chk_kode_daftar_5x CHECK (gagal_count < 5 OR gugur_at IS NOT NULL),
  CONSTRAINT chk_kode_daftar_gagal5 CHECK (gugur_alasan IS DISTINCT FROM 'gagal_5x' OR gagal_count = 5),
  CONSTRAINT chk_kode_daftar_terpakai CHECK (dipakai_at IS NULL OR (gugur_at IS NULL AND dipakai_at <= kedaluwarsa_at))
);
-- At most one active code per user (MF-10 i, U-83). A code past kedaluwarsa_at is inactive by time, but stays in the
-- index until a new issue marks it 'diganti' in the same transaction, as the contract specifies.
CREATE UNIQUE INDEX kode_daftar_mfa_aktif_uq ON kode_daftar_mfa (pengguna_id) WHERE dipakai_at IS NULL AND gugur_at IS NULL;
CREATE INDEX kode_daftar_mfa_pengguna_idx ON kode_daftar_mfa (pengguna_id, diterbitkan_at);

CREATE FUNCTION tg_kode_daftar_beku() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  IF (NEW.kode_id, NEW.pengguna_id, NEW.kode_hash, NEW.diterbitkan_oleh, NEW.diterbitkan_at, NEW.kedaluwarsa_at)
       IS DISTINCT FROM (OLD.kode_id, OLD.pengguna_id, OLD.kode_hash, OLD.diterbitkan_oleh, OLD.diterbitkan_at, OLD.kedaluwarsa_at)
     OR NEW.gagal_count < OLD.gagal_count
     OR (OLD.dipakai_at IS NOT NULL AND NEW.dipakai_at IS DISTINCT FROM OLD.dipakai_at)
     OR (OLD.gugur_at IS NOT NULL AND (NEW.gugur_at IS DISTINCT FROM OLD.gugur_at OR NEW.gugur_alasan IS DISTINCT FROM OLD.gugur_alasan)) THEN
    RAISE EXCEPTION 'kode_daftar_mfa: identitas, hash, dan keadaan akhir tidak boleh diubah (MF-10)' USING ERRCODE = 'integrity_constraint_violation';
  END IF;
  RETURN NEW;
END $$;
CREATE TRIGGER kode_daftar_mfa_beku BEFORE UPDATE ON kode_daftar_mfa
  FOR EACH ROW EXECUTE FUNCTION tg_kode_daftar_beku();

-- MF-07 / AC US-24 item 8: activating F1 revokes every session of an auditor or platform admin, so each of them
-- passes the MFA step on the next login. Sessions are 8-hour rows that cannot be restored; the rollback does not
-- bring them back (users log in again).
DELETE FROM admin_session s
 WHERE EXISTS (SELECT 1 FROM admin_user_peran p
                WHERE p.pengguna_id = s.pengguna_id AND p.peran IN ('auditor', 'platform_admin') AND p.dicabut_at IS NULL);
