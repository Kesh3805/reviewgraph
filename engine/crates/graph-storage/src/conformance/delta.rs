//! Delta-chain cases of the GS-001 suite (ADR-003 overlay semantics).

use super::*;
use crate::model::{flatten, GraphDelta};
use crate::types::SnapshotKind;

/// Fills `file_version_id` on a delta's file list, the way `with_file_versions` does for a
/// full graph.
async fn with_delta_files(f: &HarnessFixture, d: &GraphDelta) -> Result<GraphDelta, String> {
    let listing = Graph {
        schema_version: GRAPH_SCHEMA_VERSION,
        files: d.files.clone(),
        nodes: d.nodes_added.clone(),
        edges: Vec::new(),
        unresolved: Vec::new(),
    };
    let listing = with_file_versions(f, &listing).await?;
    let mut out = d.clone();
    out.files = listing.files;
    Ok(out)
}

/// Sorts every list of a delta, so comparisons never depend on storage order.
fn normalize_delta(d: &GraphDelta) -> GraphDelta {
    let mut out = d.clone();
    out.files.sort_by(|a, b| a.path.cmp(&b.path));
    out.files.dedup_by(|a, b| a.path == b.path);
    out.nodes_added.sort_by_key(|n| n.key());
    out.nodes_added.dedup_by_key(|n| n.key());
    out.nodes_removed.sort();
    out.nodes_removed.dedup();
    out.edges_added.sort();
    out.edges_added
        .dedup_by(|a, b| a.identity() == b.identity());
    out.edges_removed.sort();
    out.edges_removed
        .dedup_by(|a, b| a.identity() == b.identity());
    out.unresolved_replaced.sort_by(|a, b| a.0.cmp(&b.0));
    for (_, rows) in &mut out.unresolved_replaced {
        rows.sort_by_key(|r| r.ordinal);
        rows.dedup_by_key(|r| r.ordinal);
    }
    out.lineage.sort_by_key(|l| (l.from_key, l.to_key));
    out
}

/// One delta: a new file with a node and an edge into the base graph.
fn sample_delta() -> GraphDelta {
    GraphDelta {
        files: vec![file("src/d.ts", FileChange::Added)],
        nodes_added: vec![symbol(6, "src/d.ts", "deltaFn")],
        nodes_removed: Vec::new(),
        edges_added: vec![edge(6, EdgeKind::Calls, 3, 800)],
        edges_removed: Vec::new(),
        unresolved_replaced: Vec::new(),
        lineage: Vec::new(),
    }
}

/// A delta is read back exactly as written, and the chain materializes as `flatten`.
pub async fn write_delta_then_load_equals_overlay_flatten(h: &dyn Harness) -> Result<(), String> {
    let f = h.fresh().await.map_err(show)?;
    let base_g = with_file_versions(&f, &fixture_graph()).await?;
    let base = ready_full(&f, &base_g, sha('a')?, FINGERPRINT).await?;
    let d = with_delta_files(&f, &sample_delta()).await?;
    let delta_meta = ready_delta(&f, &d, sha('b')?, OTHER_FINGERPRINT, base.id).await?;

    let loaded = f
        .store
        .load_graph(&f.scope, delta_meta.id)
        .await
        .map_err(show)?;
    let expected = flatten(&base_g, std::slice::from_ref(&d));
    if loaded != expected {
        return Err(format!(
            "materialized graph differs from flatten: {}/{} vs {}/{} nodes/edges",
            loaded.nodes.len(),
            loaded.edges.len(),
            expected.nodes.len(),
            expected.edges.len()
        ));
    }

    let read_back = f
        .store
        .load_delta(&f.scope, delta_meta.id)
        .await
        .map_err(show)?;
    if normalize_delta(&read_back) != normalize_delta(&d) {
        return Err("load_delta did not return the rows that were written".to_owned());
    }
    Ok(())
}

/// A three-deep delta chain materializes as the ordered overlay of all three.
pub async fn three_level_delta_chain_materializes(h: &dyn Harness) -> Result<(), String> {
    let f = h.fresh().await.map_err(show)?;
    let base_g = with_file_versions(&f, &fixture_graph()).await?;
    let base = ready_full(&f, &base_g, sha('a')?, FINGERPRINT).await?;

    let d1 = with_delta_files(&f, &sample_delta()).await?;
    let m1 = ready_delta(&f, &d1, sha('b')?, OTHER_FINGERPRINT, base.id).await?;

    let mut second = sample_delta();
    second.files = vec![file("src/e.ts", FileChange::Added)];
    second.nodes_added = vec![symbol(7, "src/e.ts", "second")];
    second.edges_added = vec![edge(7, EdgeKind::Calls, 6, 850)];
    let d2 = with_delta_files(&f, &second).await?;
    let m2 = ready_delta(&f, &d2, sha('c')?, FINGERPRINT, m1.id).await?;

    let mut third = sample_delta();
    third.files = vec![file("src/f.ts", FileChange::Added)];
    third.nodes_added = vec![symbol(8, "src/f.ts", "third")];
    third.edges_added = vec![edge(8, EdgeKind::Calls, 7, 870)];
    let d3 = with_delta_files(&f, &third).await?;
    let m3 = ready_delta(&f, &d3, sha('d')?, FINGERPRINT, m2.id).await?;

    let loaded = f.store.load_graph(&f.scope, m3.id).await.map_err(show)?;
    let expected = flatten(&base_g, &[d1.clone(), d2.clone(), d3.clone()]);
    if loaded != expected {
        return Err(format!(
            "chain materialization differs: {}/{} nodes/edges vs {}/{}",
            loaded.nodes.len(),
            loaded.edges.len(),
            expected.nodes.len(),
            expected.edges.len()
        ));
    }
    for want in ["src/d.ts", "src/e.ts", "src/f.ts"] {
        if !loaded.files.iter().any(|f| f.path == want) {
            return Err(format!(
                "file {want} is missing from the materialized chain"
            ));
        }
    }

    let partial = f.store.load_graph(&f.scope, m2.id).await.map_err(show)?;
    let expected_partial = flatten(&base_g, &[d1, d2]);
    if partial != expected_partial {
        return Err("an intermediate link did not materialize its own prefix".to_owned());
    }

    let chain = f.store.chain(&f.scope, m3.id).await.map_err(show)?;
    if chain.len() != 4 {
        return Err(format!(
            "expected 4 snapshots in the chain, got {}",
            chain.len()
        ));
    }
    if chain[0].id != base.id || chain[3].id != m3.id {
        return Err("chain is not ordered [full, …, head]".to_owned());
    }
    Ok(())
}

/// A tombstone removes exactly the edge it names.
pub async fn tombstone_removes_edge(h: &dyn Harness) -> Result<(), String> {
    let f = h.fresh().await.map_err(show)?;
    let base_g = with_file_versions(&f, &fixture_graph()).await?;
    let base = ready_full(&f, &base_g, sha('a')?, FINGERPRINT).await?;

    let victim = base_g
        .edges
        .iter()
        .find(|e| e.source == key(1) && e.kind == EdgeKind::Calls && e.target == key(3))
        .ok_or("fixture is missing the victim edge")?
        .clone();
    let d = GraphDelta {
        edges_removed: vec![victim.clone()],
        ..GraphDelta::default()
    };
    let m = ready_delta(&f, &d, sha('b')?, OTHER_FINGERPRINT, base.id).await?;

    let loaded = f.store.load_graph(&f.scope, m.id).await.map_err(show)?;
    if loaded
        .edges
        .iter()
        .any(|e| e.identity() == victim.identity())
    {
        return Err("tombstoned edge survived materialization".to_owned());
    }
    let survivors = [
        (1, EdgeKind::Imports, 3),
        (2, EdgeKind::References, 3),
        (1, EdgeKind::Calls, 4),
        (3, EdgeKind::Calls, 1),
        (5, EdgeKind::HandledBy, 3),
    ];
    for (from, kind, to) in survivors {
        if !loaded
            .edges
            .iter()
            .any(|e| e.source == key(from) && e.kind == kind && e.target == key(to))
        {
            return Err(format!(
                "edge {from} -{kind}-> {to} was removed as collateral damage"
            ));
        }
    }
    if loaded.nodes.len() != 5 {
        return Err("a tombstone must not touch nodes".to_owned());
    }
    Ok(())
}

/// A delta may override an edge by re-adding its identity with new attributes.
pub async fn edge_override_in_delta(h: &dyn Harness) -> Result<(), String> {
    let f = h.fresh().await.map_err(show)?;
    let base_g = with_file_versions(&f, &fixture_graph()).await?;
    let base = ready_full(&f, &base_g, sha('a')?, FINGERPRINT).await?;

    let mut upgraded = base_g
        .edges
        .iter()
        .find(|e| e.source == key(1) && e.kind == EdgeKind::Calls && e.target == key(3))
        .ok_or("fixture is missing the override target")?
        .clone();
    upgraded.confidence = Confidence::from_permille(1000);
    let d = GraphDelta {
        edges_added: vec![upgraded.clone()],
        ..GraphDelta::default()
    };
    let m = ready_delta(&f, &d, sha('b')?, OTHER_FINGERPRINT, base.id).await?;

    let loaded = f.store.load_graph(&f.scope, m.id).await.map_err(show)?;
    let edge = loaded
        .edges
        .iter()
        .find(|e| e.identity() == upgraded.identity())
        .ok_or("overridden edge disappeared")?;
    if edge.confidence != upgraded.confidence {
        return Err(format!(
            "expected confidence {}, got {}",
            upgraded.confidence, edge.confidence
        ));
    }
    if loaded.edges.len() != base_g.edges.len() {
        return Err("an override must not add a duplicate identity".to_owned());
    }
    Ok(())
}

/// Deleting a file drops its node and its unresolved references.
pub async fn deleted_file_removes_nodes_and_unresolved(h: &dyn Harness) -> Result<(), String> {
    let f = h.fresh().await.map_err(show)?;
    let base_g = with_file_versions(&f, &fixture_graph()).await?;
    let base = ready_full(&f, &base_g, sha('a')?, FINGERPRINT).await?;

    let victim = "src/b.ts";
    let victim_key = key(3);
    let tombstones: Vec<GraphEdge> = base_g
        .edges
        .iter()
        .filter(|e| e.source == victim_key || e.target == victim_key)
        .cloned()
        .collect();
    if tombstones.is_empty() {
        return Err("fixture is missing edges to tombstone".to_owned());
    }
    let d = GraphDelta {
        files: vec![file(victim, FileChange::Deleted)],
        nodes_removed: vec![victim_key],
        edges_removed: tombstones,
        ..GraphDelta::default()
    };
    let m = ready_delta(&f, &d, sha('b')?, OTHER_FINGERPRINT, base.id).await?;

    let loaded = f.store.load_graph(&f.scope, m.id).await.map_err(show)?;
    if loaded.files.iter().any(|f| f.path == victim) {
        return Err("deleted file is still listed".to_owned());
    }
    if loaded.nodes.iter().any(|n| n.key() == victim_key) {
        return Err("deleted file's node survived".to_owned());
    }
    if loaded.unresolved.iter().any(|u| u.file == victim) {
        return Err("deleted file's unresolved references survived".to_owned());
    }
    if loaded.nodes.iter().any(|n| n.file() == Some(victim)) {
        return Err("another node still claims the deleted file".to_owned());
    }
    if !loaded.files.iter().any(|f| f.path == "src/a.ts") {
        return Err("an unrelated file was dropped with the deleted one".to_owned());
    }
    Ok(())
}

/// Re-linking a file replaces its whole unresolved set (C6).
pub async fn relinked_file_replaces_unresolved_rows(h: &dyn Harness) -> Result<(), String> {
    let f = h.fresh().await.map_err(show)?;
    let base_g = with_file_versions(&f, &fixture_graph()).await?;
    let base = ready_full(&f, &base_g, sha('a')?, FINGERPRINT).await?;

    let target = "src/b.ts";
    let version = base_g
        .files
        .iter()
        .find(|f| f.path == target)
        .and_then(|f| f.file_version_id)
        .ok_or("fixture file has no file version")?;

    let mut listing = file(target, FileChange::Relinked);
    listing.file_version_id = Some(version);
    let d = GraphDelta {
        files: vec![listing],
        // The file was re-parsed, so its symbols are re-added after `flatten` dropped them.
        nodes_added: vec![symbol(3, target, "beta")],
        unresolved_replaced: vec![(
            target.to_owned(),
            vec![unresolved(target, 0, "stillMissing")],
        )],
        ..GraphDelta::default()
    };
    let m = ready_delta(&f, &d, sha('b')?, OTHER_FINGERPRINT, base.id).await?;

    let loaded = f.store.load_graph(&f.scope, m.id).await.map_err(show)?;
    let rows: Vec<&UnresolvedRef> = loaded
        .unresolved
        .iter()
        .filter(|u| u.file == target)
        .collect();
    if rows.len() != 1 {
        return Err(format!(
            "expected exactly one replaced row, got {}",
            rows.len()
        ));
    }
    if rows[0].name != "stillMissing" {
        return Err(format!("expected the delta's row, got {}", rows[0].name));
    }
    if !loaded.nodes.iter().any(|n| n.key() == key(3)) {
        return Err("the re-linked file lost its node".to_owned());
    }
    Ok(())
}

/// A delta cannot be written on a base that is not `Ready`, and a full snapshot cannot be
/// written as a delta.
pub async fn delta_on_non_ready_base_rejected(h: &dyn Harness) -> Result<(), String> {
    let f = h.fresh().await.map_err(show)?;
    let full = pending_full(&f, sha('a')?, SnapshotPurpose::DefaultBranch, FINGERPRINT).await?;
    let delta_meta = pending_delta(
        &f,
        sha('b')?,
        SnapshotPurpose::DefaultBranch,
        FINGERPRINT,
        full.id,
    )
    .await?;

    let empty = GraphDelta::default();
    match f.store.write_delta(&f.scope, delta_meta.id, &empty).await {
        Err(StoreError::InvalidStatus { id, found, .. }) if id == full.id => {
            if found.is_readable() {
                return Err("the base was already readable".to_owned());
            }
        }
        Err(e) => return Err(format!("expected InvalidStatus, got {e}")),
        Ok(_) => return Err("write_delta accepted a base that is not ready".to_owned()),
    }

    // …and a delta written to a *ready* full snapshot is a shape error, not a status one.
    let base_g = with_file_versions(&f, &fixture_graph()).await?;
    let ready = ready_full(&f, &base_g, sha('c')?, OTHER_FINGERPRINT).await?;
    match f.store.write_delta(&f.scope, ready.id, &empty).await {
        Err(StoreError::InvalidRequest(_)) | Err(StoreError::InvalidStatus { .. }) => {}
        Err(e) => return Err(format!("expected a rejection, got {e}")),
        Ok(_) => return Err("a full snapshot accepted a delta payload".to_owned()),
    }
    let kind = fetch(&f, full.id).await?.kind;
    if kind != SnapshotKind::Full {
        return Err("snapshot kind changed unexpectedly".to_owned());
    }
    let still_pending = fetch(&f, delta_meta.id).await?;
    if !still_pending.status.is_writable() {
        return Err("a rejected write changed the status".to_owned());
    }
    Ok(())
}
