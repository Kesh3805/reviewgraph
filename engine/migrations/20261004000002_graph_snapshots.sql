-- Graph snapshots (GS-003): the snapshot rows themselves, the file membership, resolved edges,
-- synthetic nodes and symbol lineage they own, plus the `edge_kinds` / `resolved_by_kinds` /
-- `provenance_kinds` lookups and the deferred FK on `unresolved_refs.snapshot_id` (C6).
--
--   * Every graph query is snapshot-scoped, so the PRD §103 `(repository_id, …)` index prefixes
--     become `(snapshot_id, …)`; `symbols_repo_key` (GS-002) keeps covering (repository_id,
--     symbol_id).
--   * `graph_edges` omits repository_id (derivable from the snapshot) to save 16 B per row on
--     the largest table; RLS reads organization_id instead.
--   * Tenant tables carry organization_id and repeat the DOM-009 RLS block; the lookup tables
--     are global reference data without RLS.
--   * Composite foreign keys (id, organization_id) keep a child and its parent in the same
--     tenant, so a foreign organization can never be referenced even through a stale id.

CREATE TABLE edge_kinds (
  id   smallint PRIMARY KEY,
  name text NOT NULL UNIQUE
);

INSERT INTO edge_kinds (id, name) VALUES
  (0,  'CONTAINS'),
  (1,  'DECLARES'),
  (2,  'IMPORTS'),
  (3,  'EXPORTS'),
  (4,  'CALLS'),
  (5,  'READS'),
  (6,  'WRITES'),
  (7,  'IMPLEMENTS'),
  (8,  'EXTENDS'),
  (9,  'OVERRIDES'),
  (10, 'REFERENCES'),
  (11, 'USES_TYPE'),
  (12, 'RETURNS_TYPE'),
  (13, 'ACCEPTS_TYPE'),
  (14, 'ROUTES_TO'),
  (15, 'HANDLED_BY'),
  (16, 'TESTS'),
  (17, 'COVERS'),
  (18, 'PRODUCES_JOB'),
  (19, 'CONSUMES_JOB'),
  (20, 'READS_CONFIG'),
  (21, 'WRITES_CONFIG'),
  (22, 'READS_TABLE'),
  (23, 'WRITES_TABLE'),
  (24, 'DEPENDS_ON'),
  (25, 'THROWS'),
  (26, 'CATCHES'),
  (27, 'SERIALIZES'),
  (28, 'DESERIALIZES'),
  (29, 'VALIDATES'),
  (30, 'AUTHORIZES'),
  (31, 'PUBLISHES'),
  (32, 'SUBSCRIBES');

CREATE TABLE resolved_by_kinds (
  id   smallint PRIMARY KEY,
  name text NOT NULL UNIQUE
);

INSERT INTO resolved_by_kinds (id, name) VALUES
  (0, 'STRUCTURAL'),
  (1, 'IMPORT'),
  (2, 'THIS_MEMBER'),
  (3, 'DI_CONSTRUCTOR'),
  (4, 'TYPE_ANNOTATION'),
  (5, 'NAME_UNIQUE'),
  (6, 'NAME_AMBIGUOUS'),
  (7, 'FRAMEWORK'),
  (8, 'TYPE_CHECKER'),
  (9, 'HEURISTIC');

CREATE TABLE provenance_kinds (
  id   smallint PRIMARY KEY,
  name text NOT NULL UNIQUE
);

INSERT INTO provenance_kinds (id, name) VALUES
  (0, 'ANALYZER'),
  (1, 'FRAMEWORK'),
  (2, 'LINKER'),
  (3, 'TYPE_CHECKER'),
  (4, 'HEURISTIC'),
  (5, 'POLICY');

CREATE TABLE snapshots (
  id                   uuid PRIMARY KEY,
  organization_id      uuid NOT NULL REFERENCES organizations (id),
  repository_id        uuid NOT NULL,
  commit_sha           text NOT NULL CHECK (commit_sha ~ '^[0-9a-f]{40}([0-9a-f]{24})?$'),
  kind                 text NOT NULL CHECK (kind IN ('full', 'delta')),
  base_snapshot_id     uuid,
  chain_depth          smallint NOT NULL DEFAULT 0,
  purpose              text NOT NULL
    CHECK (purpose IN ('default_branch', 'pull_request', 'local', 'compaction')),
  status               text NOT NULL
    CHECK (status IN ('pending', 'indexing', 'persisting', 'ready', 'failed', 'inconsistent')),
  graph_schema_version integer NOT NULL CHECK (graph_schema_version > 0),
  analyzer_versions    jsonb NOT NULL,
  config_hash          bytea NOT NULL CHECK (octet_length(config_hash) = 32),
  config_components    jsonb NOT NULL DEFAULT '{}'::jsonb,
  fingerprint          bytea NOT NULL CHECK (octet_length(fingerprint) = 32),
  stats                jsonb NOT NULL DEFAULT '{}'::jsonb,
  error                text,
  created_at           timestamptz NOT NULL DEFAULT now(),
  updated_at           timestamptz NOT NULL DEFAULT now(),
  completed_at         timestamptz,
  CONSTRAINT snapshots_kind_base CHECK ((kind = 'full') = (base_snapshot_id IS NULL)),
  CONSTRAINT snapshots_depth
    CHECK ((kind = 'full' AND chain_depth = 0) OR (kind = 'delta' AND chain_depth >= 1)),
  CONSTRAINT snapshots_repository_fk
    FOREIGN KEY (repository_id, organization_id)
    REFERENCES repositories (id, organization_id) ON DELETE CASCADE,
  CONSTRAINT snapshots_base_fk
    FOREIGN KEY (base_snapshot_id, organization_id)
    REFERENCES snapshots (id, organization_id),
  UNIQUE (id, organization_id)
);

-- PRD §103/§104: find the snapshot(s) for a commit, newest status first.
CREATE INDEX snapshots_repo_commit ON snapshots (repository_id, commit_sha, status);
CREATE INDEX snapshots_base ON snapshots (base_snapshot_id) WHERE base_snapshot_id IS NOT NULL;
-- Two workers indexing the same fingerprint collide here at the final pending -> ready
-- transition; the loser marks itself failed and callers use the winner.
CREATE UNIQUE INDEX snapshots_ready_full
  ON snapshots (repository_id, fingerprint) WHERE status = 'ready' AND kind = 'full';
CREATE UNIQUE INDEX snapshots_ready_delta
  ON snapshots (repository_id, fingerprint, base_snapshot_id)
  WHERE status = 'ready' AND kind = 'delta';

CREATE TABLE snapshot_files (
  snapshot_id     uuid NOT NULL,
  organization_id uuid NOT NULL,
  path            text NOT NULL,
  file_version_id bigint,                                 -- NULL = deleted in this delta
  change          text NOT NULL
    CHECK (change IN ('present', 'added', 'modified', 'deleted', 'renamed', 'relinked')),
  old_path        text,
  PRIMARY KEY (snapshot_id, path),
  CONSTRAINT snapshot_files_snapshot_fk
    FOREIGN KEY (snapshot_id, organization_id)
    REFERENCES snapshots (id, organization_id) ON DELETE CASCADE,
  CONSTRAINT snapshot_files_file_version_fk
    FOREIGN KEY (file_version_id, organization_id)
    REFERENCES file_versions (id, organization_id)
);

CREATE INDEX snapshot_files_fv
  ON snapshot_files (file_version_id) WHERE file_version_id IS NOT NULL;

CREATE TABLE graph_edges (
  snapshot_id     uuid NOT NULL,
  organization_id uuid NOT NULL,
  source_key      bytea NOT NULL CHECK (octet_length(source_key) = 16),
  kind            smallint NOT NULL REFERENCES edge_kinds (id),
  target_key      bytea NOT NULL CHECK (octet_length(target_key) = 16),
  confidence      real NOT NULL CHECK (confidence >= 0 AND confidence <= 1),
  resolved_by     smallint NOT NULL REFERENCES resolved_by_kinds (id),
  provenance      smallint NOT NULL REFERENCES provenance_kinds (id),
  flags           smallint NOT NULL DEFAULT 0,
  occurrences     integer NOT NULL DEFAULT 1 CHECK (occurrences >= 1),
  origin_path     text,
  file_version_id bigint,
  line integer, col integer,
  removed         boolean NOT NULL DEFAULT false,
  PRIMARY KEY (snapshot_id, source_key, kind, target_key),
  CONSTRAINT graph_edges_snapshot_fk
    FOREIGN KEY (snapshot_id, organization_id)
    REFERENCES snapshots (id, organization_id) ON DELETE CASCADE,
  CONSTRAINT graph_edges_file_version_fk
    FOREIGN KEY (file_version_id, organization_id)
    REFERENCES file_versions (id, organization_id)
);

-- PRD §103 (…, target_node_id, edge_type): the reverse hop of neighbour lookups.
CREATE INDEX graph_edges_target ON graph_edges (snapshot_id, target_key, kind);
CREATE INDEX graph_edges_origin
  ON graph_edges (snapshot_id, origin_path) WHERE origin_path IS NOT NULL;

CREATE TABLE synthetic_nodes (
  snapshot_id     uuid NOT NULL,
  organization_id uuid NOT NULL,
  node_key        bytea NOT NULL CHECK (octet_length(node_key) = 16),
  node_id         text NOT NULL,
  kind            smallint NOT NULL REFERENCES node_kinds (id),
  attrs           jsonb NOT NULL DEFAULT '{}'::jsonb,
  removed         boolean NOT NULL DEFAULT false,
  PRIMARY KEY (snapshot_id, node_key),
  CONSTRAINT synthetic_nodes_snapshot_fk
    FOREIGN KEY (snapshot_id, organization_id)
    REFERENCES snapshots (id, organization_id) ON DELETE CASCADE
);

CREATE TABLE symbol_lineage (
  id              bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  organization_id uuid NOT NULL,
  repository_id   uuid NOT NULL,
  from_snapshot_id uuid NOT NULL,
  to_snapshot_id   uuid NOT NULL,
  from_key bytea NOT NULL CHECK (octet_length(from_key) = 16),
  to_key   bytea NOT NULL CHECK (octet_length(to_key) = 16),
  transition text NOT NULL
    CHECK (transition IN ('renamed', 'moved', 'renamed_moved', 'signature_changed_moved')),
  similarity real NOT NULL CHECK (similarity >= 0 AND similarity <= 1),
  CONSTRAINT symbol_lineage_repository_fk
    FOREIGN KEY (repository_id, organization_id)
    REFERENCES repositories (id, organization_id) ON DELETE CASCADE,
  CONSTRAINT symbol_lineage_from_fk
    FOREIGN KEY (from_snapshot_id, organization_id)
    REFERENCES snapshots (id, organization_id),
  CONSTRAINT symbol_lineage_to_fk
    FOREIGN KEY (to_snapshot_id, organization_id)
    REFERENCES snapshots (id, organization_id) ON DELETE CASCADE,
  UNIQUE (to_snapshot_id, from_key, to_key)
);

CREATE INDEX symbol_lineage_from ON symbol_lineage (repository_id, from_key);
CREATE INDEX symbol_lineage_to ON symbol_lineage (repository_id, to_key);

-- The FK GS-002 deliberately deferred: unresolved refs belong to one snapshot (C6), and a
-- delta writes the complete set of every file it lists.
ALTER TABLE unresolved_refs ADD CONSTRAINT unresolved_refs_snapshot_fk
  FOREIGN KEY (snapshot_id, organization_id)
  REFERENCES snapshots (id, organization_id) ON DELETE CASCADE;

ALTER TABLE snapshots ENABLE ROW LEVEL SECURITY;
ALTER TABLE snapshots FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON snapshots
  USING (organization_id = NULLIF(current_setting('app.organization_id', true), '')::uuid)
  WITH CHECK (organization_id = NULLIF(current_setting('app.organization_id', true), '')::uuid);

ALTER TABLE snapshot_files ENABLE ROW LEVEL SECURITY;
ALTER TABLE snapshot_files FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON snapshot_files
  USING (organization_id = NULLIF(current_setting('app.organization_id', true), '')::uuid)
  WITH CHECK (organization_id = NULLIF(current_setting('app.organization_id', true), '')::uuid);

ALTER TABLE graph_edges ENABLE ROW LEVEL SECURITY;
ALTER TABLE graph_edges FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON graph_edges
  USING (organization_id = NULLIF(current_setting('app.organization_id', true), '')::uuid)
  WITH CHECK (organization_id = NULLIF(current_setting('app.organization_id', true), '')::uuid);

ALTER TABLE synthetic_nodes ENABLE ROW LEVEL SECURITY;
ALTER TABLE synthetic_nodes FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON synthetic_nodes
  USING (organization_id = NULLIF(current_setting('app.organization_id', true), '')::uuid)
  WITH CHECK (organization_id = NULLIF(current_setting('app.organization_id', true), '')::uuid);

ALTER TABLE symbol_lineage ENABLE ROW LEVEL SECURITY;
ALTER TABLE symbol_lineage FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON symbol_lineage
  USING (organization_id = NULLIF(current_setting('app.organization_id', true), '')::uuid)
  WITH CHECK (organization_id = NULLIF(current_setting('app.organization_id', true), '')::uuid);
