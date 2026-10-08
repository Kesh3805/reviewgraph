//! Single-hop neighbour cases of the GS-001 suite.

use super::*;

/// `neighbors` must agree with the in-memory `Graph::neighbors` for every node, both
/// directions, with kind and confidence filters — and paginate without losing a row.
pub async fn neighbors_match_in_memory_query(h: &dyn Harness) -> Result<(), String> {
    let f = h.fresh().await.map_err(show)?;
    let g = with_file_versions(&f, &fixture_graph()).await?;
    let meta = ready_full(&f, &g, sha('a')?, FINGERPRINT).await?;
    let loaded = f.store.load_graph(&f.scope, meta.id).await.map_err(show)?;

    let filters = [
        (EdgeKindSet::ALL, Confidence::MIN),
        (EdgeKindSet::ALL, Confidence::from_permille(900)),
        (EdgeKindSet::of(EdgeKind::Calls), Confidence::MIN),
        (
            EdgeKindSet::of(EdgeKind::Calls),
            Confidence::from_permille(850),
        ),
        (EdgeKindSet::of(EdgeKind::Imports), Confidence::MIN),
        (EdgeKindSet::of(EdgeKind::HandledBy), Confidence::MIN),
    ];
    let directions = [Direction::Out, Direction::In];

    for node in &loaded.nodes {
        for dir in directions {
            for (kinds, min_confidence) in filters {
                let want: Vec<crate::model::GraphEdge> = loaded
                    .neighbors(node.key(), dir, kinds, min_confidence, 1000, None)
                    .into_iter()
                    .cloned()
                    .collect();
                let page = f
                    .store
                    .neighbors(
                        &f.scope,
                        meta.id,
                        node.key(),
                        dir,
                        kinds,
                        min_confidence,
                        1000,
                        None,
                    )
                    .await
                    .map_err(show)?;
                if page.edges != want {
                    return Err(format!(
                        "neighbors({:?}, {dir:?}, {kinds}) for {} returned {} edges, expected {}",
                        node.key(),
                        node.key(),
                        page.edges.len(),
                        want.len()
                    ));
                }
                if page.next_cursor.is_some() {
                    return Err("a page of 1000 claims there is a next page".to_owned());
                }
            }
        }
    }

    // Paginated reads concatenate to the unpaged result, in the same order.
    for dir in directions {
        let want: Vec<crate::model::GraphEdge> = loaded
            .neighbors(key(1), dir, EdgeKindSet::ALL, Confidence::MIN, 1000, None)
            .into_iter()
            .cloned()
            .collect();
        if want.is_empty() {
            return Err(format!("the fixture has no {dir:?} edges out of node 1"));
        }
        let mut seen: Vec<crate::model::GraphEdge> = Vec::new();
        let mut cursor = None;
        for _ in 0..32 {
            let page = f
                .store
                .neighbors(
                    &f.scope,
                    meta.id,
                    key(1),
                    dir,
                    EdgeKindSet::ALL,
                    Confidence::MIN,
                    1,
                    cursor,
                )
                .await
                .map_err(show)?;
            let count = page.edges.len();
            seen.extend(page.edges);
            match page.next_cursor {
                Some(next) => cursor = Some(next),
                None => break,
            }
            if count == 0 {
                return Err("a next cursor was returned with an empty page".to_owned());
            }
        }
        if seen != want {
            return Err(format!(
                "paginated {dir:?} neighbours returned {}/{} edges in a different order",
                seen.len(),
                want.len()
            ));
        }
    }
    Ok(())
}
