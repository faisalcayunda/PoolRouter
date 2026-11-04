-- Rollback of 20261105000002_f3_grants.sql (SHAA-354). Run as poolrouter_owner, before the schema rollback (reverse order).
-- Grants on the F3 tables disappear with the tables (down 1); only grants on F0 objects need revoking here.
REVOKE SELECT (blend_id) ON request FROM pr_api, pr_worker, pr_controller, pr_blend;
REVOKE UPDATE (butuh_dasar_hukum_blend) ON provider FROM pr_admin_api;
REVOKE ALL ON ALL TABLES IN SCHEMA public FROM pr_blend;
REVOKE EXECUTE ON FUNCTION fn_catat_gerbang_blend(text, text, text) FROM pr_api;
REVOKE EXECUTE ON FUNCTION fn_purge_blend_payload(integer) FROM pr_controller;
