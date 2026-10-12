//! `GET .../source?path&start&end`: a redacted excerpt of at most 200 lines.

use axum::extract::{Path, Query, State};
use axum::Json;
use review_core::ids::SnapshotId;
use uuid::Uuid;

use super::{scope_of, Authed};
use crate::dto::{SourceQuery, SourceResponse, MAX_EXCERPT_LINES};
use crate::error::ApiError;
use crate::state::AppState;

/// A repository-relative path with no traversal, no absolute form and no control characters.
fn valid_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 1000
        && !path.starts_with('/')
        && !path.contains('\\')
        && !path.chars().any(char::is_control)
        && path
            .split('/')
            .all(|segment| !segment.is_empty() && segment != "..")
}

pub async fn source(
    State(state): State<AppState>,
    claims: Authed,
    Path((repo, snap)): Path<(Uuid, Uuid)>,
    Query(query): Query<SourceQuery>,
) -> Result<Json<SourceResponse>, ApiError> {
    let scope = scope_of(&claims.0, repo)?;
    if !valid_path(&query.path) {
        return Err(ApiError::BadRequest(
            "path must be repository-relative".to_owned(),
        ));
    }
    if query.start == 0 || query.end < query.start {
        return Err(ApiError::BadRequest(
            "start must be at least 1 and end must not be before start".to_owned(),
        ));
    }
    let snapshot = SnapshotId::from_uuid(snap);
    // The commit lookup is scoped, so a snapshot of another repository is a 404 here.
    let commit = state.backend().snapshot_commit(scope, snapshot).await?;
    let capped_end = query
        .end
        .min(query.start.saturating_add(MAX_EXCERPT_LINES - 1));
    let text = state
        .backend()
        .read_source(scope, &commit, &query.path)
        .await?;

    let wanted = (capped_end - query.start + 1) as usize;
    let lines: Vec<&str> = text
        .lines()
        .skip(query.start as usize - 1)
        .take(wanted)
        .collect();
    if lines.is_empty() {
        return Err(ApiError::NotFound);
    }
    let end = query.start + lines.len() as u32 - 1;
    let (redacted_text, counts) = model_gateway::redact::redact_str(&lines.join("\n"));
    Ok(Json(SourceResponse {
        snapshot_id: snap.to_string(),
        path: query.path,
        start: query.start,
        end,
        text: redacted_text,
        redacted: !counts.is_empty(),
        truncated: capped_end < query.end,
    }))
}

#[cfg(test)]
mod tests {
    use super::valid_path;

    #[test]
    fn rejects_traversal_and_absolute_paths() {
        assert!(valid_path("src/a.ts"));
        assert!(!valid_path("../etc/passwd"));
        assert!(!valid_path("src/../../x"));
        assert!(!valid_path("/etc/passwd"));
        assert!(!valid_path("src\\a.ts"));
        assert!(!valid_path("src//a.ts"));
        assert!(!valid_path(""));
    }
}
