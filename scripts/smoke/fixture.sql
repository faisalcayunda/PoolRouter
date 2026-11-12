-- Fixture SQL smoke (ARC-378, spec smoke-test bagian 3 langkah 4; Q4). Sama polanya dengan
-- fixture sesi ARC-251 (scripts/e2e-api-asli.sh): sesi lewat baris admin_session langsung,
-- tanpa login UI, kecuali G1 yang menguji login + MFA sungguhan sekali lewat HTTP (spec
-- Testing Decisions). SQL ini dijalankan scripts/smoke.sh lewat sb_psql SESUDAH migrate +
-- seed sintetis (backend-cli seed pengguna, seed dukcapil), jadi enam pengguna seed di bawah
-- ini sudah ada (backend/README.md).
--
-- Placeholder __UPPER_SNAKE__ diisi scripts/smoke.sh lewat sed sebelum SQL ini dipipe ke psql
-- (sama pola dengan sb_render_env di scripts/stack-berlabel.sh), supaya token sesi dan kode
-- pendaftaran MFA acak per run, tidak pernah di-commit (Keamanan, spec bagian Keamanan).
--
-- Enam baris admin_session, satu per peran seed, dengan mfa_terverifikasi_at=now() (bypass
-- login UI untuk gelombang selain G1). Hash SHA-256 token dihitung sed di shell, bukan di SQL,
-- supaya token mentahnya hanya ada di memory shell (umask 077) dan env file Playwright.
INSERT INTO admin_session (sesi_id, token_sha256, pengguna_id, kedaluwarsa_at, user_agent, mfa_terverifikasi_at)
SELECT gen_random_uuid(), decode(v.hash_hex, 'hex'), u.pengguna_id, now() + interval '8 hours', 'smoke', now()
FROM admin_user u
JOIN (VALUES
  ('seed-platform-admin', '__HASH_PLATFORM_ADMIN__'),
  ('seed-penyetuju',      '__HASH_PENYETUJU__'),
  ('seed-operator',       '__HASH_OPERATOR__'),
  ('seed-auditor',        '__HASH_AUDITOR__'),
  ('seed-client-owner',   '__HASH_CLIENT_OWNER__'),
  ('seed-admin-auditor',  '__HASH_ADMIN_AUDITOR__')
) AS v (username, hash_hex) ON v.username = u.username
RETURNING 1;

-- Q4: kode_daftar_mfa untuk seed-penyetuju, diterbitkan seed-platform-admin, umur 24 jam persis
-- (constraint chk_kode_daftar_umur), supaya G1 bisa membuktikan mfaDaftar + mfaKonfirmasi
-- sungguhan lewat HTTP untuk peran yang wajib MFA (platform_admin/auditor, ADR-0006 §9).
INSERT INTO kode_daftar_mfa (kode_id, pengguna_id, kode_hash, diterbitkan_oleh, diterbitkan_at, kedaluwarsa_at)
SELECT gen_random_uuid(), u.pengguna_id, '__HASH_KODE_DAFTAR_MFA__', p.pengguna_id, now(), now() + interval '24 hours'
FROM admin_user u, admin_user p
WHERE u.username = 'seed-penyetuju' AND p.username = 'seed-platform-admin'
RETURNING 1;
