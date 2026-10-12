-- Registry of Qdrant collections, one per embedding space and epoch (SEM-004, ADR-008).
--
-- Global, not tenant-scoped: collections are shared and tenant isolation is by payload filter
-- (SEM-005), so this table holds no tenant data and has no RLS. At most one collection is
-- active; activation flips building -> active and the previous active -> retiring in one
-- transaction. state_changed_at drives retirement (`review semantic retire --older-than 7d`).

CREATE TABLE semantic_collections (
  name text PRIMARY KEY,
  space_id text NOT NULL,
  provider text NOT NULL,
  model text NOT NULL,
  dims int NOT NULL CHECK (dims > 0),
  version int NOT NULL CHECK (version > 0),
  state text NOT NULL CHECK (state IN ('building', 'active', 'retiring', 'retired')),
  created_at timestamptz NOT NULL DEFAULT now(),
  activated_at timestamptz,
  state_changed_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (space_id, version)
);

CREATE UNIQUE INDEX one_active_collection ON semantic_collections ((true)) WHERE state = 'active';
