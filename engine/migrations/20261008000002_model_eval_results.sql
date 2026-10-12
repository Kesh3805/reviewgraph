-- Model evaluation results (EVAL-005, ADR-010).
--
-- Global benchmark data: one row per (eval run, tier, provider, model). There are no tenant rows,
-- so the table has no organization_id and no RLS. Only the engine and ops roles may write it;
-- the API may read it (router provenance). Inserts are idempotent through the UNIQUE key with
-- ON CONFLICT DO NOTHING.

CREATE TABLE model_eval_results (
  id uuid PRIMARY KEY,
  eval_run_id text NOT NULL,
  config_name text NOT NULL,
  config_hash text NOT NULL,
  corpus_version text NOT NULL,
  corpus_hash text NOT NULL,
  engine_git_sha text NOT NULL,
  routing_table_hash text NOT NULL,
  tier text NOT NULL,
  provider text NOT NULL,
  model text NOT NULL,
  prompt_versions jsonb NOT NULL,
  reviewer_versions jsonb NOT NULL,
  verification_version text NOT NULL,
  synthetic_fixtures boolean NOT NULL,
  cases_total integer NOT NULL,
  repetitions integer NOT NULL,
  precision numeric(5, 4),
  recall numeric(5, 4),
  fp_rate numeric(5, 4),
  trap_fp_rate numeric(5, 4),
  safe_change_fp_rate numeric(5, 4),
  structured_output_success numeric(5, 4),
  verification_success numeric(5, 4),
  suppression_precision numeric(5, 4),
  latency_p50_ms integer,
  latency_p95_ms integer,
  input_tokens bigint,
  output_tokens bigint,
  cached_tokens bigint,
  cost_usd_micros bigint,
  metrics jsonb NOT NULL,
  report_path text NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (eval_run_id, tier, provider, model)
);

REVOKE INSERT, UPDATE, DELETE ON model_eval_results FROM rg_api;
GRANT SELECT ON model_eval_results TO rg_api;
GRANT SELECT, INSERT ON model_eval_results TO rg_engine, rg_ops;
