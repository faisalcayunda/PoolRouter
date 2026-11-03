-- Privileged bootstrap for the PoolRouter database (SHAA-193, ADR-0001 rev 15 section 7.2).
--
-- Roles are cluster-wide, so they are NOT created by the sqlx migrations (those run as
-- poolrouter_owner, which has no CREATEROLE). Run this once per cluster as a superuser,
-- through backend/db/bootstrap.sh. It is idempotent: re-running rotates the passwords.
--
-- Passwords are read from secret files via psql backtick expansion, so they never appear
-- on a command line or in the repo. Required psql variable: dbname.
-- Required environment variable: POOLROUTER_SECRETS_DIR (one file per role, named like the role).

\set ON_ERROR_STOP on
\set QUIET on

\set pw_owner `cat "$POOLROUTER_SECRETS_DIR/poolrouter_owner"`
\set pw_api `cat "$POOLROUTER_SECRETS_DIR/pr_api"`
\set pw_admin_api `cat "$POOLROUTER_SECRETS_DIR/pr_admin_api"`
\set pw_controller `cat "$POOLROUTER_SECRETS_DIR/pr_controller"`
\set pw_worker `cat "$POOLROUTER_SECRETS_DIR/pr_worker"`
\set pw_webhook `cat "$POOLROUTER_SECRETS_DIR/pr_webhook"`

DO $$
DECLARE
  r text;
BEGIN
  FOREACH r IN ARRAY ARRAY['poolrouter_owner', 'pr_api', 'pr_admin_api', 'pr_controller', 'pr_worker', 'pr_webhook']
  LOOP
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = r) THEN
      EXECUTE format('CREATE ROLE %I LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS', r);
    END IF;
  END LOOP;
END
$$;

ALTER ROLE poolrouter_owner PASSWORD :'pw_owner';
ALTER ROLE pr_api PASSWORD :'pw_api';
ALTER ROLE pr_admin_api PASSWORD :'pw_admin_api';
ALTER ROLE pr_controller PASSWORD :'pw_controller';
ALTER ROLE pr_worker PASSWORD :'pw_worker';
ALTER ROLE pr_webhook PASSWORD :'pw_webhook';

-- Never let a statement from the application roles leak parameters into the server log
-- (ADR-0001 section 7.1, contract R-04). Two settings, both needed:
--  * log_parameter_max_length_on_error = 0: an ERROR must not print parameters (binds).
--  * log_parameter_max_length = 0: extended-protocol parameters are not logged when log_statement is 'all'/'mod',
--    so a NIK cannot reach the server log through that path either (see the note on server scope below).
ALTER ROLE pr_api SET log_parameter_max_length_on_error = 0;
ALTER ROLE pr_admin_api SET log_parameter_max_length_on_error = 0;
ALTER ROLE pr_controller SET log_parameter_max_length_on_error = 0;
ALTER ROLE pr_worker SET log_parameter_max_length_on_error = 0;
ALTER ROLE pr_webhook SET log_parameter_max_length_on_error = 0;
ALTER ROLE pr_api SET log_parameter_max_length = 0;
ALTER ROLE pr_admin_api SET log_parameter_max_length = 0;
ALTER ROLE pr_controller SET log_parameter_max_length = 0;
ALTER ROLE pr_worker SET log_parameter_max_length = 0;
ALTER ROLE pr_webhook SET log_parameter_max_length = 0;
-- Scope: log_parameter_max_length and its _on_error variant are user-settable, so the per-role settings above are
-- what actually applies to statements from these roles (the server default of log_parameter_max_length_on_error is
-- already 0; log_parameter_max_length defaults to -1, i.e. unlimited). `log_statement` is superuser-only, so the
-- question of whether parameters are logged at all belongs to the server configuration of the target environment
-- (ADR-0001 section 7.1), not to a migration that runs as poolrouter_owner.

-- The database and the public schema belong to poolrouter_owner; nobody else may create objects.
SELECT format('ALTER DATABASE %I OWNER TO poolrouter_owner', :'dbname') \gexec
SELECT format('REVOKE ALL ON DATABASE %I FROM PUBLIC', :'dbname') \gexec
SELECT format('GRANT CONNECT ON DATABASE %I TO pr_api, pr_admin_api, pr_controller, pr_worker, pr_webhook', :'dbname') \gexec

\connect :dbname
ALTER SCHEMA public OWNER TO poolrouter_owner;
REVOKE ALL ON SCHEMA public FROM PUBLIC;
GRANT USAGE ON SCHEMA public TO pr_api, pr_admin_api, pr_controller, pr_worker, pr_webhook;
