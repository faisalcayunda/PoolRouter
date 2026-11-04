-- Rollback of 20261105000002_f3_grants.sql (SHAA-354). Run as poolrouter_owner, before 20261105000001.down.
-- Grants on the F3 tables/columns disappear with them (down 1); only grants on pre-existing objects are listed.
REVOKE UPDATE (butuh_dasar_hukum_blend) ON provider FROM pr_admin_api;
REVOKE ALL ON ALL TABLES IN SCHEMA public FROM pr_blend;
