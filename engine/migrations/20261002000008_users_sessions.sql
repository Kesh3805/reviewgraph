-- Dashboard sessions and GitHub-derived membership sync (API-004).
--
-- `users` already exists (DOM-009: provider, provider_user_id, login, display_name, email);
-- this adds the avatar. Sessions are global to a user (not tenant scoped), like `users`, and
-- are looked up by session id only. The GitHub OAuth token is never stored: the callback uses
-- it to list installations and drops it.

ALTER TABLE users ADD COLUMN IF NOT EXISTS avatar_url text;

CREATE TABLE sessions (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  user_id uuid NOT NULL REFERENCES users (id) ON DELETE CASCADE,
  created_at timestamptz NOT NULL DEFAULT now(),
  expires_at timestamptz NOT NULL,
  revoked_at timestamptz,
  -- SHA-256 of the User-Agent: enough to recognise a device in a list, not a fingerprint store.
  user_agent_hash text CHECK (user_agent_hash ~ '^[0-9a-f]{64}$'),
  CHECK (expires_at > created_at)
);
CREATE INDEX sessions_user_idx ON sessions (user_id);
CREATE INDEX sessions_live_idx ON sessions (expires_at) WHERE revoked_at IS NULL;

GRANT SELECT, INSERT, UPDATE, DELETE ON sessions TO rg_api, rg_engine, rg_ops;

-- Replaces a user's memberships with the set derived from the installations GitHub says they
-- can access (called on every login, so access revoked on GitHub is revoked here too).
-- Memberships and organizations are tenant scoped, so this runs as the function owner (rg_ops)
-- with a pinned search_path. Roles are limited to what the provider can prove: no `admin`.
CREATE OR REPLACE FUNCTION rg_sync_user_memberships(
  p_user_id uuid, p_org_ids uuid[], p_roles text[]
) RETURNS void
LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS $$
BEGIN
  IF coalesce(array_length(p_org_ids, 1), 0) <> coalesce(array_length(p_roles, 1), 0) THEN
    RAISE EXCEPTION 'org and role arrays differ in length';
  END IF;
  DELETE FROM memberships
  WHERE user_id = p_user_id AND NOT (organization_id = ANY (p_org_ids));
  INSERT INTO memberships (organization_id, user_id, role)
  SELECT o, p_user_id, r FROM unnest(p_org_ids, p_roles) AS t (o, r)
  ON CONFLICT (organization_id, user_id) DO UPDATE
    -- Never downgrade an admin/owner assigned in the dashboard; only upgrade.
    SET role = CASE
      WHEN memberships.role IN ('admin', 'owner') THEN memberships.role
      ELSE EXCLUDED.role
    END;
END
$$;

REVOKE ALL ON FUNCTION rg_sync_user_memberships(uuid, uuid[], text[]) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION rg_sync_user_memberships(uuid, uuid[], text[]) TO rg_api, rg_ops;
ALTER FUNCTION rg_sync_user_memberships(uuid, uuid[], text[]) OWNER TO rg_ops;
