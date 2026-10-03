/** Defaults point at the dev compose services; override with RG_TEST_DATABASE_URL / RG_TEST_REDIS_URL. */
process.env.RG_TEST_DATABASE_URL ??=
  'postgres://reviewgraph:reviewgraph-dev@127.0.0.1:25432/reviewgraph';
process.env.RG_TEST_REDIS_URL ??= 'redis://127.0.0.1:26379';
