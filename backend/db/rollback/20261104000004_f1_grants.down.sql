-- Rollback of 20261104000004_f1_grants.sql (SHAA-333). Run as poolrouter_owner, in reverse order (4, 3, 2, 1).
-- Column grants on the new columns disappear with the columns (down 1..3); the table grants of the new tables with the tables.
REVOKE ALL ON ALL TABLES IN SCHEMA public FROM pr_job_balikan, pr_job_arsip, pr_restore;
-- Back to F0: the controller owns archived_at again (contract 8.1 row "Job arsip" before F1).
GRANT UPDATE (archived_at) ON transaction_log TO pr_controller;
-- Back to the F0 default-privilege state (only the schema-level entry remains).
ALTER DEFAULT PRIVILEGES FOR ROLE poolrouter_owner GRANT EXECUTE ON FUNCTIONS TO PUBLIC;
