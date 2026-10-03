-- Database roles for tenant isolation (API-003, ADR-014).
--
--   rg_migrator  owns the schema; used only by `sqlx migrate`. NOLOGIN here: deployments grant it
--                to the migration login.
--   rg_api       the control plane; RLS enforced.
--   rg_engine    the Rust worker/engine; RLS enforced, sets the tenant per job.
--   rg_ops       BYPASSRLS; the reaper and admin scripts only. Never handed to a request path.
--
-- Roles are cluster-wide objects, so creation is guarded. Logins are granted membership in these
-- roles by the deployment; none of them can log in by itself.

DO $$
BEGIN
  IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'rg_migrator') THEN
    CREATE ROLE rg_migrator NOLOGIN;
  END IF;
  IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'rg_api') THEN
    CREATE ROLE rg_api NOLOGIN NOBYPASSRLS;
  END IF;
  IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'rg_engine') THEN
    CREATE ROLE rg_engine NOLOGIN NOBYPASSRLS;
  END IF;
  IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'rg_ops') THEN
    CREATE ROLE rg_ops NOLOGIN BYPASSRLS;
  END IF;
END
$$;

GRANT USAGE ON SCHEMA public TO rg_api, rg_engine, rg_ops;
GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA public TO rg_api, rg_engine, rg_ops;
-- Tables created by later migrations (run as the same role) are readable and writable too; each
-- of those migrations must also enable RLS on its tenant tables.
ALTER DEFAULT PRIVILEGES IN SCHEMA public
  GRANT SELECT, INSERT, UPDATE, DELETE ON TABLES TO rg_api, rg_engine, rg_ops;
