-- Foundation: updated_at trigger function, tenancy and installations (DOM-009).
--
-- Conventions for every migration:
--   * Enums are text + CHECK (not PG ENUM), so expand/contract is one ALTER TABLE.
--   * Timestamps are timestamptz NOT NULL DEFAULT now(); ids are uuid DEFAULT gen_random_uuid()
--     (the application normally supplies UUIDv7).
--   * Every tenant table has organization_id and UNIQUE (id, organization_id); children use
--     composite FKs (parent_id, organization_id) so a row can never reference another tenant.
--   * Forward-only: applied migrations are never edited.

CREATE FUNCTION rg_set_updated_at() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  NEW.updated_at := now();
  RETURN NEW;
END
$$;

CREATE TABLE organizations (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  slug text NOT NULL UNIQUE CHECK (slug ~ '^[a-z0-9][a-z0-9-]{0,62}$'),
  display_name text NOT NULL CHECK (length(display_name) BETWEEN 1 AND 200),
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now()
);

-- Global identity: a user may belong to many organizations, so this is not a tenant table.
CREATE TABLE users (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  provider text NOT NULL CHECK (provider IN ('github')),
  provider_user_id text NOT NULL,
  login text NOT NULL,
  display_name text,
  email text,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (provider, provider_user_id)
);

CREATE TABLE memberships (
  organization_id uuid NOT NULL REFERENCES organizations (id) ON DELETE CASCADE,
  user_id uuid NOT NULL REFERENCES users (id) ON DELETE CASCADE,
  role text NOT NULL CHECK (role IN ('owner', 'admin', 'member', 'viewer')),
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (organization_id, user_id)
);
CREATE INDEX memberships_user_idx ON memberships (user_id);

-- No token columns, ever: installation credentials are minted on demand and never stored.
CREATE TABLE provider_installations (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  organization_id uuid NOT NULL REFERENCES organizations (id) ON DELETE CASCADE,
  provider text NOT NULL CHECK (provider IN ('github')),
  provider_installation_id bigint NOT NULL,
  account_login text NOT NULL,
  account_type text NOT NULL CHECK (account_type IN ('organization', 'user')),
  permissions jsonb NOT NULL DEFAULT '{}'::jsonb,
  suspended_at timestamptz,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (provider, provider_installation_id),
  UNIQUE (id, organization_id)
);
CREATE INDEX provider_installations_org_idx ON provider_installations (organization_id);

CREATE TRIGGER organizations_set_updated_at BEFORE UPDATE ON organizations
  FOR EACH ROW EXECUTE FUNCTION rg_set_updated_at();
CREATE TRIGGER users_set_updated_at BEFORE UPDATE ON users
  FOR EACH ROW EXECUTE FUNCTION rg_set_updated_at();
CREATE TRIGGER memberships_set_updated_at BEFORE UPDATE ON memberships
  FOR EACH ROW EXECUTE FUNCTION rg_set_updated_at();
CREATE TRIGGER provider_installations_set_updated_at BEFORE UPDATE ON provider_installations
  FOR EACH ROW EXECUTE FUNCTION rg_set_updated_at();
