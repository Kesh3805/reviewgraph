-- Graph per-file intelligence (GS-002): the content-addressed `file_versions`, the `symbols`
-- inside them and the snapshot-scoped `unresolved_refs` that record what could not be linked
-- (C6).
--
--   * `file_versions` is keyed by (repository, path, content_hash, analyzer_version) so the
--     same bytes are stored once and shared by every snapshot (ADR-003). GS-004 upserts with
--     ON CONFLICT DO NOTHING + re-select; that constraint is the concurrency guard.
--   * `symbols` repeats organization_id/repository_id (denormalized, no trigger) so RLS and the
--     PRD §103 index work without joining file_versions; a test asserts they match the parent.
--   * `unresolved_refs.snapshot_id` is a plain uuid here and gains its FK in GS-003, where
--     `snapshots` exists.
--   * Lookups (`node_kinds`) are global reference data, not tenant tables, so they carry no
--     organization_id and no RLS policy.
--
-- Roles and RLS follow the DOM-009 / API-009 pattern: ENABLE + FORCE + `tenant_isolation` on
-- `app.organization_id` (fail closed: a missing setting yields NULL and matches nothing).
-- `ALTER DEFAULT PRIVILEGES` from 20261002000006_db_roles.sql already grants the DML roles.

-- Node taxonomy mirrored by codegraph::ALL_NODE_KINDS (CG-001/CG-002). The ids are the Rust
-- discriminants; they are never reused.
CREATE TABLE node_kinds (
  id   smallint PRIMARY KEY,
  name text NOT NULL UNIQUE
);

INSERT INTO node_kinds (id, name) VALUES
  (0,   'Repository'),
  (1,   'Package'),
  (2,   'Module'),
  (3,   'Directory'),
  (4,   'File'),
  (10,  'Namespace'),
  (11,  'Class'),
  (12,  'Interface'),
  (13,  'Struct'),
  (14,  'Trait'),
  (15,  'Enum'),
  (16,  'TypeAlias'),
  (20,  'Function'),
  (21,  'Method'),
  (22,  'Constructor'),
  (23,  'Property'),
  (24,  'Field'),
  (25,  'Parameter'),
  (26,  'Variable'),
  (27,  'Constant'),
  (30,  'ApiEndpoint'),
  (31,  'Controller'),
  (32,  'Handler'),
  (33,  'Middleware'),
  (40,  'DatabaseEntity'),
  (41,  'DatabaseTable'),
  (42,  'DatabaseColumn'),
  (43,  'Migration'),
  (50,  'Queue'),
  (51,  'QueueProducer'),
  (52,  'QueueConsumer'),
  (53,  'JobHandler'),
  (60,  'Configuration'),
  (61,  'EnvironmentVariable'),
  (70,  'TestSuite'),
  (71,  'TestCase'),
  (72,  'Fixture'),
  (80,  'ExternalDependency'),
  (81,  'ExternalApi'),
  (90,  'BuildTarget'),
  (91,  'CliCommand'),
  (92,  'Worker'),
  (100, 'DocumentationRule'),
  (101, 'ArchitecturalBoundary');

CREATE TABLE file_versions (
  id               bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  organization_id  uuid    NOT NULL REFERENCES organizations (id),
  repository_id    uuid    NOT NULL,
  path             text    NOT NULL CHECK (path <> '' AND path !~ '(^/|\.\./|^\.\.$)'),
  content_hash     bytea   NOT NULL CHECK (octet_length(content_hash) = 32),
  language         text    NOT NULL,
  analyzer_version text    NOT NULL,
  parse_status     text    NOT NULL CHECK (parse_status IN ('ok', 'partial', 'failed', 'skipped')),
  size_bytes       integer NOT NULL CHECK (size_bytes >= 0),
  symbol_count     integer NOT NULL DEFAULT 0 CHECK (symbol_count >= 0),
  diagnostic_count integer NOT NULL DEFAULT 0 CHECK (diagnostic_count >= 0),
  created_at       timestamptz NOT NULL DEFAULT now(),
  CONSTRAINT file_versions_content_key
    UNIQUE (repository_id, path, content_hash, analyzer_version),
  CONSTRAINT file_versions_repo_fk
    FOREIGN KEY (repository_id, organization_id)
    REFERENCES repositories (id, organization_id) ON DELETE CASCADE,
  UNIQUE (id, organization_id)
);

-- PRD §103 (repository_id, file_id): scope by repository then walk the paths it owns.
CREATE INDEX file_versions_repo_path ON file_versions (repository_id, path);

CREATE TABLE symbols (
  file_version_id bigint  NOT NULL,
  organization_id uuid    NOT NULL,
  repository_id   uuid    NOT NULL,
  symbol_key      bytea   NOT NULL CHECK (octet_length(symbol_key) = 16),
  symbol_id       text    NOT NULL,
  kind            smallint NOT NULL REFERENCES node_kinds (id),
  name            text    NOT NULL,
  qualified_name  text    NOT NULL,
  signature       text,
  start_line integer NOT NULL CHECK (start_line >= 1),
  start_col  integer NOT NULL CHECK (start_col >= 1),
  end_line   integer NOT NULL CHECK (end_line >= 1),
  end_col    integer NOT NULL CHECK (end_col >= 1),
  body_hash      bytea CHECK (body_hash IS NULL OR octet_length(body_hash) = 16),
  signature_hash bytea CHECK (signature_hash IS NULL OR octet_length(signature_hash) = 16),
  parent_key     bytea CHECK (parent_key IS NULL OR octet_length(parent_key) = 16),
  visibility     smallint NOT NULL DEFAULT 0,
  is_exported    boolean  NOT NULL DEFAULT false,
  is_generated   boolean  NOT NULL DEFAULT false,
  attrs          jsonb    NOT NULL DEFAULT '{}'::jsonb,
  PRIMARY KEY (file_version_id, symbol_key),
  CONSTRAINT symbols_file_version_fk
    FOREIGN KEY (file_version_id, organization_id)
    REFERENCES file_versions (id, organization_id) ON DELETE CASCADE,
  CONSTRAINT symbols_repository_fk
    FOREIGN KEY (repository_id, organization_id)
    REFERENCES repositories (id, organization_id) ON DELETE CASCADE
);

-- PRD §103 (repository_id, symbol_id): resolve one key inside one repository.
CREATE INDEX symbols_repo_key ON symbols (repository_id, symbol_key);
-- Name lookups (API search, INC-004 cold path).
CREATE INDEX symbols_repo_name ON symbols (repository_id, name);

CREATE TABLE unresolved_refs (
  snapshot_id      uuid    NOT NULL,          -- FK added by GS-003 (C6)
  organization_id  uuid    NOT NULL,
  repository_id    uuid    NOT NULL,
  file_version_id  bigint  NOT NULL,
  ordinal          integer NOT NULL CHECK (ordinal >= 0),
  from_symbol_key  bytea CHECK (from_symbol_key IS NULL OR octet_length(from_symbol_key) = 16),
  name             text    NOT NULL,
  ref_kind         smallint NOT NULL,
  import_specifier text,
  reason           smallint NOT NULL,
  candidate_count  smallint NOT NULL DEFAULT 0,
  line integer NOT NULL, col integer NOT NULL,
  PRIMARY KEY (snapshot_id, file_version_id, ordinal),
  CONSTRAINT unresolved_refs_file_version_fk
    FOREIGN KEY (file_version_id, organization_id)
    REFERENCES file_versions (id, organization_id) ON DELETE CASCADE,
  CONSTRAINT unresolved_refs_repository_fk
    FOREIGN KEY (repository_id, organization_id)
    REFERENCES repositories (id, organization_id) ON DELETE CASCADE
);

CREATE INDEX unresolved_refs_name ON unresolved_refs (snapshot_id, name);
CREATE INDEX unresolved_refs_spec
  ON unresolved_refs (snapshot_id, import_specifier) WHERE import_specifier IS NOT NULL;

ALTER TABLE file_versions ENABLE ROW LEVEL SECURITY;
ALTER TABLE file_versions FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON file_versions
  USING (organization_id = NULLIF(current_setting('app.organization_id', true), '')::uuid)
  WITH CHECK (organization_id = NULLIF(current_setting('app.organization_id', true), '')::uuid);

ALTER TABLE symbols ENABLE ROW LEVEL SECURITY;
ALTER TABLE symbols FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON symbols
  USING (organization_id = NULLIF(current_setting('app.organization_id', true), '')::uuid)
  WITH CHECK (organization_id = NULLIF(current_setting('app.organization_id', true), '')::uuid);

ALTER TABLE unresolved_refs ENABLE ROW LEVEL SECURITY;
ALTER TABLE unresolved_refs FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON unresolved_refs
  USING (organization_id = NULLIF(current_setting('app.organization_id', true), '')::uuid)
  WITH CHECK (organization_id = NULLIF(current_setting('app.organization_id', true), '')::uuid);
