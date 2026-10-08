//! Round-trip, lookup and tenant-isolation cases of the GS-001 suite.

use super::*;
use crate::types::{SnapshotKind, SnapshotQuery};

/// `write_full` then `load_graph` returns exactly what was written.
pub async fn write_full_then_load_roundtrip(h: &dyn Harness) -> Result<(), String> {
    let f = h.fresh().await.map_err(show)?;
    let g = with_file_versions(&f, &fixture_graph()).await?;
    let meta = ready_full(&f, &g, sha('a')?, FINGERPRINT).await?;

    let loaded = f.store.load_graph(&f.scope, meta.id).await.map_err(show)?;
    if loaded != g {
        return Err(format!(
            "round trip differs: wrote {} files/{} nodes/{} edges/{} unresolved, read {}/{}/{}/{}",
            g.files.len(),
            g.nodes.len(),
            g.edges.len(),
            g.unresolved.len(),
            loaded.files.len(),
            loaded.nodes.len(),
            loaded.edges.len(),
            loaded.unresolved.len()
        ));
    }

    let meta = fetch(&f, meta.id).await?;
    if meta.status != SnapshotStatus::Ready {
        return Err(format!("status is {}, expected ready", meta.status));
    }
    if meta.stats.files != 3 || meta.stats.nodes != 5 || meta.stats.edges != 6 {
        return Err(format!(
            "recorded stats are files={} nodes={} edges={}",
            meta.stats.files, meta.stats.nodes, meta.stats.edges
        ));
    }
    if meta.chain_depth != 0 || meta.base.is_some() {
        return Err("a full snapshot must have depth 0 and no base".to_owned());
    }
    Ok(())
}

/// `find_ready` resolves by commit, fingerprint, kind and purpose.
pub async fn find_ready_by_commit_and_fingerprint(h: &dyn Harness) -> Result<(), String> {
    let f = h.fresh().await.map_err(show)?;
    let commit = sha('a')?;
    let g = with_file_versions(&f, &fixture_graph()).await?;
    let meta = ready_full(&f, &g, commit.clone(), FINGERPRINT).await?;

    let by_commit = f
        .store
        .find_ready(&f.scope, SnapshotQuery::by_commit(commit.clone()))
        .await
        .map_err(show)?;
    if !matches!(by_commit, Some(m) if m.id == meta.id) {
        return Err("find_ready by commit did not return the snapshot".to_owned());
    }

    let by_fingerprint = f
        .store
        .find_ready(&f.scope, SnapshotQuery::by_fingerprint(FINGERPRINT))
        .await
        .map_err(show)?;
    if !matches!(by_fingerprint, Some(m) if m.id == meta.id) {
        return Err("find_ready by fingerprint did not return the snapshot".to_owned());
    }

    let wrong_commit = f
        .store
        .find_ready(&f.scope, SnapshotQuery::by_commit(sha('b')?))
        .await
        .map_err(show)?;
    if wrong_commit.is_some() {
        return Err("find_ready matched a snapshot at another commit".to_owned());
    }

    let wrong_fingerprint = f
        .store
        .find_ready(&f.scope, SnapshotQuery::by_fingerprint(OTHER_FINGERPRINT))
        .await
        .map_err(show)?;
    if wrong_fingerprint.is_some() {
        return Err("find_ready matched another fingerprint".to_owned());
    }

    let wrong_kind = f
        .store
        .find_ready(
            &f.scope,
            SnapshotQuery::by_fingerprint(FINGERPRINT).with_kind(SnapshotKind::Delta),
        )
        .await
        .map_err(show)?;
    if wrong_kind.is_some() {
        return Err("find_ready ignored the kind filter".to_owned());
    }

    let right_purpose = f
        .store
        .find_ready(
            &f.scope,
            SnapshotQuery::by_fingerprint(FINGERPRINT)
                .with_kind(SnapshotKind::Full)
                .with_purpose(SnapshotPurpose::DefaultBranch),
        )
        .await
        .map_err(show)?;
    if !matches!(right_purpose, Some(m) if m.id == meta.id) {
        return Err("find_ready dropped the purpose filter".to_owned());
    }

    let wrong_scope = f
        .store
        .find_ready(&f.foreign, SnapshotQuery::by_commit(commit))
        .await
        .map_err(show)?;
    if wrong_scope.is_some() {
        return Err("find_ready leaked a snapshot to another tenant".to_owned());
    }
    Ok(())
}

/// Every method refuses a snapshot id that belongs to another tenant, always as
/// `NotFound` (never as "forbidden"), so ids cannot be probed.
pub async fn cross_repository_snapshot_is_not_found(h: &dyn Harness) -> Result<(), String> {
    let f = h.fresh().await.map_err(show)?;
    let g = with_file_versions(&f, &fixture_graph()).await?;
    let meta = ready_full(&f, &g, sha('a')?, FINGERPRINT).await?;

    let mut problems: Vec<String> = Vec::new();
    if f.store
        .snapshot(&f.foreign, meta.id)
        .await
        .map_err(show)?
        .is_some()
    {
        problems.push("snapshot() leaked to another scope".to_owned());
    }

    match f.store.load_graph(&f.foreign, meta.id).await {
        Err(StoreError::NotFound(_)) => {}
        Err(e) => problems.push(format!("load_graph: expected NotFound, got {e}")),
        Ok(_) => problems.push("load_graph: returned a foreign graph".to_owned()),
    }
    match f
        .store
        .transition(
            &f.foreign,
            meta.id,
            SnapshotStatus::Ready,
            SnapshotStatus::Inconsistent,
            None,
        )
        .await
    {
        Err(StoreError::NotFound(_)) => {}
        Err(e) => problems.push(format!("transition: expected NotFound, got {e}")),
        Ok(true) => problems.push("transition: mutated a foreign snapshot".to_owned()),
        Ok(false) => problems.push("transition: silently missed a foreign snapshot".to_owned()),
    }
    match f.store.chain(&f.foreign, meta.id).await {
        Err(StoreError::NotFound(_)) => {}
        Err(e) => problems.push(format!("chain: expected NotFound, got {e}")),
        Ok(_) => problems.push("chain: returned a foreign chain".to_owned()),
    }
    if !problems.is_empty() {
        return Err(problems.join("; "));
    }
    // The owner still sees everything.
    let owned = fetch(&f, meta.id).await?;
    if owned.id != meta.id {
        return Err("the owner lost access to its own snapshot".to_owned());
    }
    Ok(())
}

/// `nodes` returns exactly the graph's nodes, and `None` for keys that are not there.
pub async fn nodes_lookup_matches_graph(h: &dyn Harness) -> Result<(), String> {
    let f = h.fresh().await.map_err(show)?;
    let g = with_file_versions(&f, &fixture_graph()).await?;
    let meta = ready_full(&f, &g, sha('a')?, FINGERPRINT).await?;

    let keys: Vec<NodeKey> = g.nodes.iter().map(crate::model::Node::key).collect();
    let found = f
        .store
        .nodes(&f.scope, meta.id, &keys)
        .await
        .map_err(show)?;
    if found.len() != keys.len() {
        return Err(format!(
            "asked for {} keys, got {} results",
            keys.len(),
            found.len()
        ));
    }
    for (want, got) in keys.iter().zip(&found) {
        match got {
            Some(node) if node.key() == *want => {}
            Some(node) => return Err(format!("expected {want}, got {}", node.key())),
            None => return Err(format!("node {want} is missing")),
        }
    }

    let missing = f
        .store
        .nodes(&f.scope, meta.id, &[key(0xee), key(1), key(0xff)])
        .await
        .map_err(show)?;
    if missing.len() != 3 {
        return Err("mixed lookup changed the result count".to_owned());
    }
    if !matches!(missing.as_slice(), [None, Some(_), None]) {
        return Err(format!(
            "expected [None, Some, None], got {:?}",
            missing.iter().map(|n| n.is_some()).collect::<Vec<_>>()
        ));
    }
    Ok(())
}
