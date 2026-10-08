-- GS-004: a delta removes a node by key alone (`GraphDelta::nodes_removed` is a list of
-- keys), so a synthetic-node tombstone row carries only `node_key` and `removed = true`;
-- `node_id`, `kind` and `attrs` are meaningless for it. Additions keep the full payload
-- (removed = false), and readers require the payload exactly when `removed = false`.
--
-- The key + `removed` flag is the whole tombstone payload; `load_delta` returns the keys of
-- every `removed = true` row of one snapshot as `nodes_removed`, which is what `flatten`
-- needs to drop the node.

ALTER TABLE synthetic_nodes ALTER COLUMN node_id DROP NOT NULL;
ALTER TABLE synthetic_nodes ALTER COLUMN kind DROP NOT NULL;
