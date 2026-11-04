-- F1 schema, part 1: aging (D1), allocation (D2), webhook retry (D3), platform settings (SHAA-333).
-- Source: data contract v0.9.5-final section 13 (F1-2, F1-3, F1-4); ADR-0006 rev 3 final (docs/adr/0006-delta-arsitektur-f1.md).
-- Purely additive over the F0 schema: new nullable columns, new constraints, new triggers. No F0 column is dropped,
-- retyped or re-keyed, and no F0 row is updated (log tables stay append-only, R-03). Rollback: backend/db/rollback/.
-- Runs as poolrouter_owner. Physical names follow F0 (text + CHECK, timestamptz, jsonb).

-- ---------------------------------------------------------------- D1: priority and aging (AG-01..AG-08)
-- Contract 2.1: aging_ambang_detik is a downstream-pool setting (default 600, 0 = aging off). The contract column list
-- in F1-9 does not enumerate it; 2.1 requires it, so it is a column on downstream_pool (reported in the PR).
ALTER TABLE downstream_pool
  ADD COLUMN aging_ambang_detik integer NOT NULL DEFAULT 600
    CONSTRAINT chk_aging_ambang_pool CHECK (aging_ambang_detik = 0 OR aging_ambang_detik BETWEEN 60 AND 86400);

ALTER TABLE request
  ADD COLUMN aging_ambang_detik integer
    CONSTRAINT chk_aging_ambang_rentang CHECK (aging_ambang_detik = 0 OR aging_ambang_detik BETWEEN 60 AND 86400),
  ADD COLUMN prioritas_diubah_at timestamptz,
  -- D2 (AL-07): scope of a kuota_habis rejection. NULL on F0 rows is read as 'upstream_pool'.
  ADD COLUMN reject_lingkup text
    CONSTRAINT chk_reject_lingkup_domain CHECK (reject_lingkup IN ('upstream_pool', 'anggaran')),
  -- D3 (WR-01): earliest next webhook claim. NULL = may go now (F0 behaviour).
  ADD COLUMN webhook_not_before timestamptz;

-- Aging columns exist only on async rows that were not rejected (sync, rejected and F0 rows keep NULL, F1-2.2).
-- "Wajib untuk async sejak aktivasi F1" cannot be a constraint without failing every F0 row on its next UPDATE, so
-- it is pipeline check DQ-40 (d) (docs/db/skema-f1.md section 6).
ALTER TABLE request
  ADD CONSTRAINT chk_aging_hanya_async CHECK (
    (mode = 'async' AND status <> 'rejected') OR (aging_ambang_detik IS NULL AND prioritas_diubah_at IS NULL)),
  ADD CONSTRAINT chk_prioritas_diubah_at CHECK (prioritas_diubah_at IS NULL OR prioritas_diubah_at >= received_at),   -- DQ-40 (c)
  ADD CONSTRAINT chk_reject_lingkup CHECK (reject_lingkup IS NULL OR coalesce(reject_reason = 'kuota_habis', false)),                 -- DQ-44 (a)
  ADD CONSTRAINT chk_webhook_not_before CHECK (webhook_not_before IS NULL OR coalesce(webhook_status = 'menunggu', false));            -- DQ-46 (c)

-- AG-01: 1 <= prioritas_efektif <= prioritas. The lower bound is F0. Added NOT VALID, then validated, so the check
-- scan holds only a SHARE UPDATE EXCLUSIVE lock; if an F0 row breaks the invariant the migration aborts untouched.
ALTER TABLE request ADD CONSTRAINT chk_prioritas_efektif_le_prioritas CHECK (prioritas_efektif <= prioritas) NOT VALID;
ALTER TABLE request VALIDATE CONSTRAINT chk_prioritas_efektif_le_prioritas;

-- Aging job: queued rows that can still climb, oldest step first (AG-03).
CREATE INDEX request_aging_idx ON request (prioritas_diubah_at)
  WHERE status = 'queued' AND prioritas_efektif > 1 AND aging_ambang_detik > 0;
-- Webhook scheduler: rows waiting for their retry time (WR-01, DQ-46 d).
CREATE INDEX request_webhook_retry_idx ON request (webhook_not_before)
  WHERE webhook_status = 'menunggu' AND webhook_not_before IS NOT NULL;

ALTER TABLE request_status_history
  ADD COLUMN prioritas_dari integer CHECK (prioritas_dari >= 1),
  ADD COLUMN prioritas_ke integer CHECK (prioritas_ke >= 1);
-- AG-04: an aging row is queued -> queued by the controller, one step, and carries both priorities; no other row does.
-- objek_pemicu_id stays NULL for aging through F0 chk_pemicu_k20.
ALTER TABLE request_status_history
  ADD CONSTRAINT chk_aging_prioritas_terisi CHECK (
    (coalesce(alasan = 'aging', false)) = (prioritas_dari IS NOT NULL AND prioritas_ke IS NOT NULL)
    AND (prioritas_dari IS NULL) = (prioritas_ke IS NULL)),
  ADD CONSTRAINT chk_aging_baris CHECK (
    alasan IS DISTINCT FROM 'aging'
    OR (status_dari = 'queued' AND status_ke = 'queued' AND aktor = 'controller' AND prioritas_dari - prioritas_ke = 1));

-- AG-01 / AG-02 / AG-03 on request: the snapshot never changes, the priority only climbs one step at a time, and every
-- step has its aging history row in the same transaction (deferred, same technique as T-04 in F0).
CREATE FUNCTION tg_request_f1_guard() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  IF NEW.prioritas IS DISTINCT FROM OLD.prioritas THEN
    RAISE EXCEPTION 'prioritas asal tidak boleh berubah (AG-01)' USING ERRCODE = 'check_violation';
  END IF;
  IF NEW.aging_ambang_detik IS DISTINCT FROM OLD.aging_ambang_detik THEN
    RAISE EXCEPTION 'aging_ambang_detik adalah snapshot intake dan tidak boleh berubah (AG-02)' USING ERRCODE = 'check_violation';
  END IF;
  IF NEW.prioritas_efektif IS DISTINCT FROM OLD.prioritas_efektif THEN
    IF NOT (OLD.status = 'queued' AND NEW.status = 'queued' AND NEW.prioritas_efektif = OLD.prioritas_efektif - 1
            AND NEW.prioritas_diubah_at IS NOT DISTINCT FROM now()) THEN
      RAISE EXCEPTION 'prioritas_efektif hanya turun satu tingkat pada request queued dan mengisi prioritas_diubah_at = now() (AG-03)'
        USING ERRCODE = 'check_violation';
    END IF;
  END IF;
  RETURN NEW;
END $$;
CREATE TRIGGER request_f1_guard BEFORE UPDATE ON request
  FOR EACH ROW EXECUTE FUNCTION tg_request_f1_guard();

CREATE FUNCTION tg_request_aging_punya_history() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
BEGIN
  IF NOT EXISTS (SELECT 1 FROM request_status_history h
                  WHERE h.request_id = NEW.request_id AND h.alasan = 'aging'
                    AND h.prioritas_dari = OLD.prioritas_efektif AND h.prioritas_ke = NEW.prioritas_efektif
                    AND h.terjadi_at = NEW.prioritas_diubah_at) THEN
    RAISE EXCEPTION 'request % naik prioritas tanpa baris aging yang cocok di request_status_history (AG-04)', NEW.request_id
      USING ERRCODE = 'integrity_constraint_violation';
  END IF;
  RETURN NULL;
END $$;
CREATE CONSTRAINT TRIGGER request_aging_history_wajib AFTER UPDATE OF prioritas_efektif ON request
  DEFERRABLE INITIALLY DEFERRED FOR EACH ROW
  WHEN (OLD.prioritas_efektif IS DISTINCT FROM NEW.prioritas_efektif)
  EXECUTE FUNCTION tg_request_aging_punya_history();

-- ---------------------------------------------------------------- D2: reserved / shareable allocation (AL-01..AL-08)
-- "Rute aktif" in the contract has no column of its own in F0; here a route is active when both its pools are active.
-- boleh_pinjam defaults to true so that existing F0 routes (no allocation) satisfy CHECK (ii); the admin API writes
-- false explicitly for an allocated route (contract 3.1).
ALTER TABLE downstream_pool_rute
  ADD COLUMN alokasi_jenis text CHECK (alokasi_jenis IN ('exact', 'persen')),
  ADD COLUMN alokasi_nilai numeric(12,2),
  ADD COLUMN sifat text CHECK (sifat IN ('reserved', 'shareable')),
  ADD COLUMN boleh_pinjam boolean NOT NULL DEFAULT true;
ALTER TABLE downstream_pool_rute
  ADD CONSTRAINT chk_alokasi_lengkap CHECK (                         -- (i)
    (alokasi_jenis IS NULL) = (alokasi_nilai IS NULL) AND (alokasi_jenis IS NULL) = (sifat IS NULL)),
  ADD CONSTRAINT chk_alokasi_tanpa_alokasi_boleh_pinjam CHECK (alokasi_jenis IS NOT NULL OR boleh_pinjam),   -- (ii)
  ADD CONSTRAINT chk_alokasi_nilai CHECK (                           -- (iii)
    alokasi_jenis IS NULL
    OR (alokasi_jenis = 'exact' AND alokasi_nilai >= 1 AND alokasi_nilai = trunc(alokasi_nilai))
    OR (alokasi_jenis = 'persen' AND alokasi_nilai BETWEEN 0.01 AND 100.00));

-- AL-01: sum of allocations of the active routes of one upstream pool <= Q = floor(kuota_harian * (1 - margin_kuota)).
-- Returns the excess (0 when fine) so the admin API can report it in its 422 (U-64).
CREATE FUNCTION fn_alokasi_kelebihan(p_upstream_pool_id uuid) RETURNS bigint
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = pg_catalog, public AS $$
  WITH q AS (
    SELECT floor(u.kuota_harian * (1 - u.margin_kuota))::bigint AS q FROM upstream_pool u WHERE u.upstream_pool_id = p_upstream_pool_id
  ), a AS (
    SELECT coalesce(sum(CASE r.alokasi_jenis WHEN 'exact' THEN r.alokasi_nilai::bigint
                                              ELSE floor(r.alokasi_nilai / 100 * q.q)::bigint END), 0) AS jumlah
      FROM downstream_pool_rute r JOIN downstream_pool d ON d.downstream_pool_id = r.downstream_pool_id, q
     WHERE r.upstream_pool_id = p_upstream_pool_id AND r.alokasi_jenis IS NOT NULL AND d.aktif
  )
  SELECT greatest(a.jumlah - q.q, 0) FROM a, q;
$$;

CREATE FUNCTION tg_alokasi_dalam_kuota() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE
  v_upstream uuid[];
  v_id uuid;
  v_lebih bigint;
BEGIN
  IF TG_TABLE_NAME = 'downstream_pool_rute' THEN
    v_upstream := ARRAY[NEW.upstream_pool_id];
  ELSIF TG_TABLE_NAME = 'upstream_pool' THEN
    v_upstream := ARRAY[NEW.upstream_pool_id];
  ELSE   -- downstream_pool: re-activation counts as an allocation change (AL-01)
    SELECT coalesce(array_agg(DISTINCT upstream_pool_id), ARRAY[]::uuid[]) INTO v_upstream
      FROM downstream_pool_rute WHERE downstream_pool_id = NEW.downstream_pool_id AND alokasi_jenis IS NOT NULL;
  END IF;
  FOREACH v_id IN ARRAY v_upstream LOOP
    v_lebih := fn_alokasi_kelebihan(v_id);
    IF v_lebih > 0 THEN
      RAISE EXCEPTION 'jumlah alokasi melebihi kuota efektif upstream pool % sebesar % (AL-01)', v_id, v_lebih
        USING ERRCODE = 'check_violation';
    END IF;
  END LOOP;
  RETURN NULL;
END $$;
CREATE CONSTRAINT TRIGGER downstream_pool_rute_alokasi AFTER INSERT OR UPDATE ON downstream_pool_rute
  DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION tg_alokasi_dalam_kuota();
CREATE CONSTRAINT TRIGGER upstream_pool_alokasi AFTER UPDATE OF kuota_harian, margin_kuota ON upstream_pool
  DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION tg_alokasi_dalam_kuota();
CREATE CONSTRAINT TRIGGER downstream_pool_alokasi AFTER UPDATE OF aktif ON downstream_pool
  DEFERRABLE INITIALLY DEFERRED FOR EACH ROW WHEN (NEW.aktif AND NOT OLD.aktif) EXECUTE FUNCTION tg_alokasi_dalam_kuota();

-- Log side (R-01: written at INSERT, never changed). AL-02 / AL-04 domain and permission stay pipeline checks
-- DQ-42: a hot-path trigger there could fail the log write after a token was granted and break M-03.
ALTER TABLE transaction_log
  ADD COLUMN sumber_anggaran varchar(36)
    CONSTRAINT chk_sumber_anggaran_bentuk CHECK (
      sumber_anggaran = 'sisa'
      OR sumber_anggaran ~ '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$'),
  ADD COLUMN downstream_pool_id uuid REFERENCES downstream_pool (downstream_pool_id);
CREATE INDEX transaction_log_sumber_idx ON transaction_log (upstream_pool_id, sumber_anggaran, requested_at);   -- AL-05, AL-06

-- DQ-44 (b), deterministic and cheap: a copy that disagrees with the request is a bug, not a race.
CREATE FUNCTION tg_log_f1_insert() RETURNS trigger LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
BEGIN
  IF NEW.downstream_pool_id IS NOT NULL
     AND NEW.downstream_pool_id IS DISTINCT FROM (SELECT r.downstream_pool_id FROM request r WHERE r.request_id = NEW.request_id) THEN
    RAISE EXCEPTION 'transaction_log.downstream_pool_id harus sama dengan request.downstream_pool_id (DQ-44 b)'
      USING ERRCODE = 'check_violation';
  END IF;
  RETURN NEW;
END $$;
CREATE TRIGGER transaction_log_f1_insert BEFORE INSERT ON transaction_log
  FOR EACH ROW EXECUTE FUNCTION tg_log_f1_insert();

-- F0 tg_log_update froze only the identity columns while a row is pending. The two F1 columns join that list; the
-- complete-row branch already froze everything except archived_at. Same body as F0 otherwise.
CREATE OR REPLACE FUNCTION tg_log_update() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  IF OLD.status_akses IS NOT NULL THEN
    IF (to_jsonb(NEW) - 'archived_at') IS DISTINCT FROM (to_jsonb(OLD) - 'archived_at') THEN
      RAISE EXCEPTION 'baris transaction_log yang lengkap tidak berubah (R-03)' USING ERRCODE = 'integrity_constraint_violation';
    END IF;
  ELSIF NEW.kode_transaksi <> OLD.kode_transaksi OR NEW.request_id <> OLD.request_id OR NEW.attempt_no <> OLD.attempt_no
        OR NEW.nik IS DISTINCT FROM OLD.nik OR NEW.requested_at <> OLD.requested_at
        OR NEW.upstream_pool_id <> OLD.upstream_pool_id OR NEW.provider_id <> OLD.provider_id
        OR NEW.sumber_anggaran IS DISTINCT FROM OLD.sumber_anggaran
        OR NEW.downstream_pool_id IS DISTINCT FROM OLD.downstream_pool_id THEN
    RAISE EXCEPTION 'kolom identitas transaction_log tidak boleh diubah (R-03)' USING ERRCODE = 'integrity_constraint_violation';
  END IF;
  RETURN NEW;
END $$;

-- ---------------------------------------------------------------- platform settings with ranges (F1-2.1, F1-4.2, F1-6.3)
-- Stored in konfigurasi_platform (F0 key/value). Defaults marked [asumsi, D-10] in the contract are seeded as given.
-- Range checks live here as a second line behind the admin API 422; changes are audited by the API (audit_konfigurasi).
CREATE FUNCTION fn_konfigurasi_int(p_kunci text, p_default integer) RETURNS integer
LANGUAGE sql STABLE SET search_path = pg_catalog, public AS $$
  SELECT coalesce((SELECT (nilai #>> '{}')::integer FROM konfigurasi_platform WHERE kunci = p_kunci), p_default);
$$;

CREATE FUNCTION tg_konfigurasi_f1() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE
  v integer;
  v_min integer;
  v_maks integer;
BEGIN
  IF NEW.kunci NOT IN ('prioritas_maks', 'webhook_batas_percobaan', 'webhook_backoff_base_detik', 'webhook_backoff_cap_detik',
                       'webhook_jitter_persen', 'restore_masa_berlaku_hari', 'restore_rentang_dengan_nik_maks_bulan',
                       'restore_rentang_tanpa_nik_maks_hari') THEN
    RETURN NEW;
  END IF;
  IF jsonb_typeof(NEW.nilai) <> 'number' OR (NEW.nilai #>> '{}') !~ '^[0-9]+$' THEN
    RAISE EXCEPTION '% harus bilangan bulat tak negatif', NEW.kunci USING ERRCODE = 'check_violation';
  END IF;
  v := (NEW.nilai #>> '{}')::integer;
  SELECT lo, hi INTO v_min, v_maks FROM (VALUES
    ('prioritas_maks', 2, 10), ('webhook_batas_percobaan', 1, 10), ('webhook_backoff_base_detik', 1, 600),
    ('webhook_backoff_cap_detik', 1, 21600), ('webhook_jitter_persen', 0, 50), ('restore_masa_berlaku_hari', 1, 30),
    ('restore_rentang_dengan_nik_maks_bulan', 1, 36), ('restore_rentang_tanpa_nik_maks_hari', 1, 31)) k(nama, lo, hi)
   WHERE nama = NEW.kunci;
  IF v < v_min OR v > v_maks THEN
    RAISE EXCEPTION '% di luar rentang sah % - % (nilai %)', NEW.kunci, v_min, v_maks, v USING ERRCODE = 'check_violation';
  END IF;
  IF NEW.kunci = 'webhook_backoff_base_detik' AND v > fn_konfigurasi_int('webhook_backoff_cap_detik', 7200) THEN
    RAISE EXCEPTION 'webhook_backoff_base_detik tidak boleh melebihi webhook_backoff_cap_detik' USING ERRCODE = 'check_violation';
  END IF;
  IF NEW.kunci = 'webhook_backoff_cap_detik' AND v < fn_konfigurasi_int('webhook_backoff_base_detik', 30) THEN
    RAISE EXCEPTION 'webhook_backoff_cap_detik tidak boleh kurang dari webhook_backoff_base_detik' USING ERRCODE = 'check_violation';
  END IF;
  -- Lowering the number of levels below a pool's priority is rejected (F1-2.1).
  IF NEW.kunci = 'prioritas_maks' AND EXISTS (SELECT 1 FROM downstream_pool WHERE prioritas > v) THEN
    RAISE EXCEPTION 'prioritas_maks % lebih kecil dari prioritas sebuah downstream pool', v USING ERRCODE = 'check_violation';
  END IF;
  RETURN NEW;
END $$;
CREATE TRIGGER konfigurasi_platform_f1 BEFORE INSERT OR UPDATE ON konfigurasi_platform
  FOR EACH ROW EXECUTE FUNCTION tg_konfigurasi_f1();

INSERT INTO konfigurasi_platform (kunci, nilai) VALUES
  ('prioritas_maks', '5'::jsonb),                          -- [asumsi, D-10]
  ('webhook_batas_percobaan', '6'::jsonb),
  ('webhook_backoff_base_detik', '30'::jsonb),
  ('webhook_backoff_cap_detik', '7200'::jsonb),
  ('webhook_jitter_persen', '20'::jsonb),
  ('restore_masa_berlaku_hari', '14'::jsonb),
  ('restore_rentang_dengan_nik_maks_bulan', '12'::jsonb),
  ('restore_rentang_tanpa_nik_maks_hari', '7'::jsonb);     -- [asumsi]

-- A pool priority cannot exceed prioritas_maks (1..prioritas_maks, F1-2.1). Fires on priority writes only, so an F0
-- pool above the new maximum is not blocked from unrelated updates.
CREATE FUNCTION tg_downstream_pool_prioritas() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  IF NEW.prioritas > fn_konfigurasi_int('prioritas_maks', 5) THEN
    RAISE EXCEPTION 'prioritas % melebihi prioritas_maks %', NEW.prioritas, fn_konfigurasi_int('prioritas_maks', 5)
      USING ERRCODE = 'check_violation';
  END IF;
  RETURN NEW;
END $$;
CREATE TRIGGER downstream_pool_prioritas BEFORE INSERT OR UPDATE OF prioritas ON downstream_pool
  FOR EACH ROW EXECUTE FUNCTION tg_downstream_pool_prioritas();
