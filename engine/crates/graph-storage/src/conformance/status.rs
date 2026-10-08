//! Status-machine cases of the GS-001 suite.

use super::*;
use crate::model::GraphDelta;

/// Ten concurrent transitions from the same `from`: exactly one wins.
pub async fn status_cas_only_one_winner(h: &dyn Harness) -> Result<(), String> {
    let f = h.fresh().await.map_err(show)?;
    let meta = f
        .store
        .create_snapshot(NewSnapshot::full(
            f.scope,
            sha('a')?,
            SnapshotPurpose::Local,
            FINGERPRINT,
        ))
        .await
        .map_err(show)?;

    let winners = cas_round(
        &f,
        meta.id,
        SnapshotStatus::Pending,
        SnapshotStatus::Indexing,
    )
    .await?;
    if winners != 1 {
        return Err(format!(
            "Pending -> Indexing had {winners} winners, expected 1"
        ));
    }
    let losers = cas_round(
        &f,
        meta.id,
        SnapshotStatus::Pending,
        SnapshotStatus::Indexing,
    )
    .await?;
    if losers != 0 {
        return Err(format!("a stale CAS won {losers} times"));
    }
    let winners = cas_round(
        &f,
        meta.id,
        SnapshotStatus::Indexing,
        SnapshotStatus::Persisting,
    )
    .await?;
    if winners != 1 {
        return Err(format!("Indexing -> Persisting had {winners} winners"));
    }
    let meta = fetch(&f, meta.id).await?;
    if meta.status != SnapshotStatus::Persisting {
        return Err(format!("status is {}, expected persisting", meta.status));
    }
    Ok(())
}

async fn cas_round(
    f: &HarnessFixture,
    id: SnapshotId,
    from: SnapshotStatus,
    to: SnapshotStatus,
) -> Result<usize, String> {
    let attempts = (0..10).map(|_| f.store.transition(&f.scope, id, from, to, None));
    let results = futures::future::join_all(attempts).await;
    let mut winners = 0;
    for result in results {
        match result {
            Ok(true) => winners += 1,
            Ok(false) => {}
            Err(e) => {
                return Err(format!(
                    "transition errored instead of returning false: {e}"
                ))
            }
        }
    }
    Ok(winners)
}

/// Transitions outside the state machine return `false`, never an error, and change nothing.
pub async fn illegal_transition_returns_false(h: &dyn Harness) -> Result<(), String> {
    let f = h.fresh().await.map_err(show)?;
    let meta = f
        .store
        .create_snapshot(NewSnapshot::full(
            f.scope,
            sha('a')?,
            SnapshotPurpose::Local,
            FINGERPRINT,
        ))
        .await
        .map_err(show)?;

    let illegal = [
        (SnapshotStatus::Pending, SnapshotStatus::Ready),
        (SnapshotStatus::Pending, SnapshotStatus::Persisting),
        (SnapshotStatus::Pending, SnapshotStatus::Inconsistent),
        (SnapshotStatus::Pending, SnapshotStatus::Pending),
        (SnapshotStatus::Ready, SnapshotStatus::Indexing),
        (SnapshotStatus::Ready, SnapshotStatus::Failed),
        (SnapshotStatus::Ready, SnapshotStatus::Ready),
    ];
    for (from, to) in illegal {
        let moved = f
            .store
            .transition(&f.scope, meta.id, from, to, None)
            .await
            .map_err(show)?;
        if moved {
            return Err(format!("illegal transition {from} -> {to} was accepted"));
        }
    }
    if fetch(&f, meta.id).await?.status != SnapshotStatus::Pending {
        return Err("an illegal transition changed the status".to_owned());
    }

    // A legal path still works afterwards: the refusals did not wedge the row.
    to_persisting(&f, meta.id).await?;
    let g = with_file_versions(&f, &fixture_graph()).await?;
    f.store
        .write_full(&f.scope, meta.id, &g)
        .await
        .map_err(show)?;
    must_transition(
        &f,
        meta.id,
        SnapshotStatus::Persisting,
        SnapshotStatus::Ready,
    )
    .await?;
    let moved = f
        .store
        .transition(
            &f.scope,
            meta.id,
            SnapshotStatus::Ready,
            SnapshotStatus::Ready,
            None,
        )
        .await
        .map_err(show)?;
    if moved {
        return Err("a self-transition was accepted".to_owned());
    }
    if fetch(&f, meta.id).await?.status != SnapshotStatus::Ready {
        return Err("a refused transition moved a ready snapshot".to_owned());
    }
    Ok(())
}

/// Readers refuse every non-`Ready` snapshot with `InvalidStatus`.
pub async fn load_non_ready_snapshot_errors(h: &dyn Harness) -> Result<(), String> {
    let f = h.fresh().await.map_err(show)?;
    let meta = f
        .store
        .create_snapshot(NewSnapshot::full(
            f.scope,
            sha('a')?,
            SnapshotPurpose::Local,
            FINGERPRINT,
        ))
        .await
        .map_err(show)?;

    match f.store.load_graph(&f.scope, meta.id).await {
        Err(StoreError::InvalidStatus { id, found, .. }) => {
            if id != meta.id {
                return Err("InvalidStatus named the wrong snapshot".to_owned());
            }
            if found != SnapshotStatus::Pending {
                return Err(format!("expected pending, got {found}"));
            }
        }
        Err(e) => return Err(format!("load_graph: expected InvalidStatus, got {e}")),
        Ok(_) => return Err("load_graph read a pending snapshot".to_owned()),
    }

    match f.store.load_delta(&f.scope, meta.id).await {
        Err(StoreError::InvalidStatus { .. }) => {}
        Err(e) => return Err(format!("load_delta: expected InvalidStatus, got {e}")),
        Ok(_) => return Err("load_delta read a pending snapshot".to_owned()),
    }
    match f
        .store
        .neighbors(
            &f.scope,
            meta.id,
            key(1),
            Direction::Out,
            EdgeKindSet::ALL,
            Confidence::MIN,
            10,
            None,
        )
        .await
    {
        Err(StoreError::InvalidStatus { .. }) => {}
        Err(e) => return Err(format!("neighbors: expected InvalidStatus, got {e}")),
        Ok(_) => return Err("neighbors read a pending snapshot".to_owned()),
    }
    match f.store.nodes(&f.scope, meta.id, &[key(1)]).await {
        Err(StoreError::InvalidStatus { .. }) => {}
        Err(e) => return Err(format!("nodes: expected InvalidStatus, got {e}")),
        Ok(_) => return Err("nodes read a pending snapshot".to_owned()),
    }
    match f.store.chain(&f.scope, meta.id).await {
        Err(StoreError::InvalidStatus { .. }) => {}
        Err(e) => return Err(format!("chain: expected InvalidStatus, got {e}")),
        Ok(_) => return Err("chain returned a pending snapshot".to_owned()),
    }

    // A missing id is NotFound, not InvalidStatus: callers can tell the two apart.
    let absent = SnapshotId::new();
    match f.store.load_graph(&f.scope, absent).await {
        Err(StoreError::NotFound(id)) if id == absent => {}
        Err(e) => return Err(format!("expected NotFound, got {e}")),
        Ok(_) => return Err("loaded a snapshot that does not exist".to_owned()),
    }
    Ok(())
}

/// Writes to a `Ready` snapshot are refused; callers create a new snapshot instead.
pub async fn write_to_ready_snapshot_errors(h: &dyn Harness) -> Result<(), String> {
    let f = h.fresh().await.map_err(show)?;
    let g = with_file_versions(&f, &fixture_graph()).await?;
    let meta = ready_full(&f, &g, sha('a')?, FINGERPRINT).await?;

    match f.store.write_full(&f.scope, meta.id, &g).await {
        Err(StoreError::InvalidStatus {
            id,
            expected,
            found,
        }) => {
            if id != meta.id
                || expected != SnapshotStatus::Persisting
                || found != SnapshotStatus::Ready
            {
                return Err(format!(
                    "unexpected InvalidStatus {{ id: {id}, expected: {expected}, found: {found} }}"
                ));
            }
        }
        Err(e) => return Err(format!("write_full: expected InvalidStatus, got {e}")),
        Ok(_) => return Err("write_full overwrote a ready snapshot".to_owned()),
    }

    match f
        .store
        .write_delta(&f.scope, meta.id, &GraphDelta::default())
        .await
    {
        Err(StoreError::InvalidStatus { .. }) | Err(StoreError::InvalidRequest(_)) => {}
        Err(e) => return Err(format!("write_delta: expected a refusal, got {e}")),
        Ok(_) => return Err("write_delta overwrote a ready snapshot".to_owned()),
    }

    let after = fetch(&f, meta.id).await?;
    if after.status != SnapshotStatus::Ready {
        return Err(format!("status became {}", after.status));
    }
    let loaded = f.store.load_graph(&f.scope, meta.id).await.map_err(show)?;
    if loaded != g {
        return Err("the refused writes corrupted the stored graph".to_owned());
    }
    Ok(())
}
