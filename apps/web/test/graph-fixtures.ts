import type { SymbolDetail, SymbolSearchResult } from '../lib/api/pending';

export const AUTHORIZE: SymbolSearchResult = {
  key: 'k-authorize',
  name: 'authorize',
  qualified_name: 'AuthService.authorize',
  kind: 'method',
  path: 'src/auth/auth.service.ts',
  line: 10,
  in_degree: 4,
  out_degree: 2,
};

export const AUTHORIZE_DETAIL: SymbolDetail = {
  key: 'k-authorize',
  id: 'sym-123',
  name: 'authorize',
  qualified_name: 'AuthService.authorize',
  kind: 'method',
  path: 'src/auth/auth.service.ts',
  start_line: 10,
  end_line: 24,
  signature: 'authorize(user: User, action: Action): Promise<void>',
  visibility: 'public',
  framework_facts: [{ name: 'nest.injectable', value: 'AuthService' }],
  lineage: [],
  edges_in: { CALLS: 4 },
  edges_out: { CALLS: 2 },
  confidence_histogram: [0, 0, 0, 0, 0, 0, 1, 0, 2, 3],
  snapshot_id: 'snap-head',
};

export const intelligenceWithSnapshots = {
  snapshots: [
    {
      id: 'snap-main',
      kind: 'full',
      commit_sha: 'aaaaaaa111',
      branch: 'main',
      chain_length: 0,
      created_at: '2026-10-06T10:00:00Z',
      node_count: 10,
      edge_count: 20,
    },
  ],
  fingerprint: null,
  versions: { tool_version: null, facts_schema_version: null, analyzers: {} },
  stats: null,
  languages: [],
  frameworks: [],
  index_jobs: [],
};
