-- F3 schema test (SHAA-354): every object of delta data contract F3 v0.3.2 section 2 exists with the contract type and
-- nullability, and the enums of section 4.3 are what the contract says. "N" = NOT NULL, "Y" = nullable.
\set ON_ERROR_STOP on

CREATE TEMP TABLE f3_kolom (tabel text, kolom text, tipe text, nullable text);
INSERT INTO f3_kolom VALUES
 ('endpoint_blend','endpoint_blend_id','uuid','N'),('endpoint_blend','nama','character varying(64)','N'),
 ('endpoint_blend','status','text','N'),('endpoint_blend','config_version','integer','N'),
 ('endpoint_blend_versi','mode_diizinkan','text','N'),('endpoint_blend_versi','tata_letak','text','N'),
 ('endpoint_blend_versi','aturan_konflik','jsonb','Y'),('endpoint_blend_versi','batas_waktu_detik','integer','Y'),
 ('endpoint_blend_versi','config_schema_version','integer','N'),
 ('endpoint_blend_komponen','nama_komponen','character varying(64)','N'),('endpoint_blend_komponen','downstream_endpoint_id','uuid','N'),
 ('endpoint_blend_komponen','prioritas','integer','N'),('endpoint_blend_komponen','peta_input','jsonb','N'),
 ('provider','butuh_dasar_hukum_blend','boolean','N'),
 ('blend','blend_id','uuid','N'),('blend','client_id','uuid','N'),('blend','credential_id','uuid','N'),('blend','endpoint_blend_id','uuid','N'),
 ('blend','endpoint_blend_config_version','integer','N'),('blend','mode','text','N'),('blend','status','text','N'),
 ('blend','reject_reason','text','Y'),('blend','reject_http_status','integer','Y'),('blend','failure_reason','text','Y'),
 ('blend','anak_penentu_request_id','uuid','Y'),('blend','cancel_reason','text','Y'),('blend','cancelled_by','text','Y'),
 ('blend','gerbang_dasar_hukum','text','N'),('blend','ringkasan_komponen','jsonb','Y'),('blend','idempotency_key','character varying(128)','Y'),
 ('blend','idempotency_fingerprint','character varying(64)','Y'),('blend','output_payload','jsonb','Y'),
 ('blend','payload_purged_at','timestamp with time zone','Y'),('blend','ttl_polling_detik','integer','Y'),
 ('blend','batas_waktu_at','timestamp with time zone','N'),('blend','requested_at','timestamp with time zone','N'),
 ('blend','completed_at','timestamp with time zone','Y'),('blend','webhook_status','text','Y'),('blend','webhook_attempt_count','integer','N'),
 ('blend','webhook_last_at','timestamp with time zone','Y'),('blend','webhook_last_http_status','integer','Y'),
 ('blend','webhook_last_error','character varying(200)','Y'),('blend','source_ip','character varying(45)','Y'),
 ('blend_status_history','blend_id','uuid','N'),('blend_status_history','seq','integer','N'),('blend_status_history','status_dari','text','Y'),
 ('blend_status_history','status_ke','text','N'),('blend_status_history','alasan','text','Y'),('blend_status_history','aktor','text','N'),
 ('blend_status_history','objek_pemicu_id','uuid','Y'),
 ('blend_webhook_log','blend_id','uuid','N'),('blend_webhook_log','attempt_no','integer','N'),('blend_webhook_log','hasil','text','N'),
 ('request','blend_id','uuid','Y'),('audit_akses','target_blend_id','uuid','Y'),
 ('gerbang_blend_peristiwa','jenis','text','N'),('gerbang_blend_peristiwa','nilai_flag','text','Y'),
 ('gerbang_blend_peristiwa','dasar_hukum_ref','character varying(200)','Y'),('gerbang_blend_peristiwa','sebab_tertutup','text','Y'),
 ('gerbang_blend_peristiwa','instance','character varying(128)','N');

SELECT t.ok(t.hitung($q$
  SELECT count(*) FROM f3_kolom k
   WHERE NOT EXISTS (
     SELECT 1 FROM pg_attribute a JOIN pg_class c ON c.oid = a.attrelid JOIN pg_namespace n ON n.oid = c.relnamespace
      WHERE n.nspname = 'public' AND c.relname = k.tabel AND a.attname = k.kolom AND a.attnum > 0 AND NOT a.attisdropped
        AND format_type(a.atttypid, a.atttypmod) = k.tipe AND (NOT a.attnotnull) = (k.nullable = 'Y'))$q$) = 0,
  'every F3 column exists with the contract type and nullability');

-- F0 objects keep their shape (no column dropped or retyped, primary keys unchanged)
SELECT t.ok(t.hitung($q$SELECT count(*) FROM t.kolom_contract k WHERE NOT EXISTS (SELECT 1 FROM information_schema.columns c
  WHERE c.table_schema = 'public' AND c.table_name = k.tabel AND c.column_name = k.kolom)$q$) = 0, 'F0 contract columns all still exist after F3');
SELECT t.ok((SELECT array_agg(a.attname::text ORDER BY a.attname) FROM pg_index i JOIN pg_attribute a ON a.attrelid = i.indrelid AND a.attnum = ANY (i.indkey)
              WHERE i.indrelid = 'public.request'::regclass AND i.indisprimary) = ARRAY['request_id'], 'request primary key unchanged');
SELECT t.ok((SELECT is_nullable = 'YES' AND column_default IS NULL FROM information_schema.columns
              WHERE table_name = 'request' AND column_name = 'blend_id'), 'request.blend_id is nullable without default (BL-01)');

-- section 4.3 enums. Blend sets are contained in the F0 sets (failure_reason also carries fungsi_gagal, which F2 adds to request).
CREATE TEMP TABLE f3_enum AS
SELECT c.conrelid::regclass::text AS tabel, c.conname,
       (SELECT array_agg(m[1] ORDER BY m[1]) FROM regexp_matches(pg_get_constraintdef(c.oid), '''([a-z_]+)''', 'g') m) AS nilai
  FROM pg_constraint c WHERE c.contype = 'c' AND c.conrelid IN ('public.request'::regclass, 'public.blend'::regclass);
SELECT t.ok((SELECT nilai FROM f3_enum WHERE tabel = 'blend' AND conname = 'blend_status_check') = ARRAY['berjalan', 'cancelled', 'done', 'expired', 'failed', 'rejected'],
  'blend.status has exactly the six values of the contract');
SELECT t.ok((SELECT nilai FROM f3_enum WHERE tabel = 'blend' AND conname = 'blend_cancel_reason_check') = (SELECT nilai FROM f3_enum WHERE tabel = 'request' AND conname = 'request_cancel_reason_check'),
  'blend.cancel_reason = the F0 set');
SELECT t.ok((SELECT nilai FROM f3_enum WHERE tabel = 'blend' AND conname = 'blend_failure_reason_check') <@ (SELECT nilai || ARRAY['fungsi_gagal'] FROM f3_enum WHERE tabel = 'request' AND conname = 'request_failure_reason_check'),
  'blend.failure_reason is within the F0 set (plus fungsi_gagal for F2)');
SELECT t.ok((SELECT nilai FROM f3_enum WHERE tabel = 'blend' AND conname = 'blend_reject_reason_check') @> (SELECT nilai FROM f3_enum WHERE tabel = 'request' AND conname = 'request_reject_reason_check')
        AND (SELECT nilai FROM f3_enum WHERE tabel = 'blend' AND conname = 'blend_reject_reason_check') <@ (SELECT nilai || ARRAY['gerbang_blend_tertutup'] FROM f3_enum WHERE tabel = 'request' AND conname = 'request_reject_reason_check'),
  'blend.reject_reason = the F0 set + gerbang_blend_tertutup only');
SELECT t.ok(NOT EXISTS (SELECT 1 FROM f3_enum WHERE tabel = 'request' AND 'gerbang_blend_tertutup' = ANY (nilai)),
  'gerbang_blend_tertutup appears in no request constraint (F0 enum untouched)');
SELECT t.ok(NOT ('gerbang_blend_tertutup' = ANY ((SELECT nilai FROM f3_enum WHERE tabel = 'blend' AND conname = 'blend_failure_reason_check'))),
  'gerbang_blend_tertutup is not a failure_reason');
