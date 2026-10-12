//! Parse diagnostics of file versions (IDX-003).
//!
//! Rows carry a severity, a stable code and an optional source range. There is deliberately no
//! message column: diagnostics must never carry source text. At most
//! [`MAX_DIAGNOSTICS_PER_FILE`] rows are stored per file version; the true count lives in
//! `file_versions.diagnostic_count`.

use std::collections::BTreeMap;
use std::sync::Mutex;

use async_trait::async_trait;
use repository::store::RepoScope;
use uuid::Uuid;

use crate::StoreError;

/// Most diagnostic rows stored for one file version.
pub const MAX_DIAGNOSTICS_PER_FILE: usize = 50;

/// One stored diagnostic. `severity`: 0 info, 1 warning, 2 error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseDiagnosticRow {
    pub severity: u8,
    pub code: String,
    pub line: Option<u32>,
    pub col: Option<u32>,
    pub end_line: Option<u32>,
    pub end_col: Option<u32>,
}

/// Persistence of diagnostics, keyed by the adapter-allocated file version id.
#[async_trait]
pub trait ParseDiagnosticsStore: Send + Sync {
    /// Writes the rows of each file version (truncated to [`MAX_DIAGNOSTICS_PER_FILE`], ordinals
    /// assigned in order). A file version that already has rows keeps them. Returns the number
    /// of rows written.
    async fn write(
        &self,
        scope: &RepoScope,
        files: &[(i64, Vec<ParseDiagnosticRow>)],
    ) -> Result<u64, StoreError>;

    /// The stored rows of one file version, in ordinal order.
    async fn read(
        &self,
        scope: &RepoScope,
        file_version_id: i64,
    ) -> Result<Vec<ParseDiagnosticRow>, StoreError>;
}

/// In-memory store for tests and dry runs.
#[derive(Debug, Default)]
pub struct MemParseDiagnostics {
    rows: Mutex<BTreeMap<(Uuid, i64), Vec<ParseDiagnosticRow>>>,
}

impl MemParseDiagnostics {
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of file versions with stored rows.
    pub fn file_versions(&self) -> usize {
        self.rows.lock().map_or(0, |r| r.len())
    }
}

#[async_trait]
impl ParseDiagnosticsStore for MemParseDiagnostics {
    async fn write(
        &self,
        scope: &RepoScope,
        files: &[(i64, Vec<ParseDiagnosticRow>)],
    ) -> Result<u64, StoreError> {
        let org = *scope.organization_id.as_uuid();
        let mut rows = self
            .rows
            .lock()
            .map_err(|_| StoreError::backend_msg("diagnostics lock poisoned"))?;
        let mut written = 0u64;
        for (id, list) in files {
            if list.is_empty() || rows.contains_key(&(org, *id)) {
                continue;
            }
            let kept: Vec<ParseDiagnosticRow> = list
                .iter()
                .take(MAX_DIAGNOSTICS_PER_FILE)
                .cloned()
                .collect();
            written += kept.len() as u64;
            rows.insert((org, *id), kept);
        }
        Ok(written)
    }

    async fn read(
        &self,
        scope: &RepoScope,
        file_version_id: i64,
    ) -> Result<Vec<ParseDiagnosticRow>, StoreError> {
        let org = *scope.organization_id.as_uuid();
        let rows = self
            .rows
            .lock()
            .map_err(|_| StoreError::backend_msg("diagnostics lock poisoned"))?;
        Ok(rows
            .get(&(org, file_version_id))
            .cloned()
            .unwrap_or_default())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use review_core::ids::{OrganizationId, RepositoryId};

    fn scope() -> RepoScope {
        RepoScope {
            organization_id: OrganizationId::new(),
            repository_id: RepositoryId::new(),
        }
    }

    fn row(code: &str) -> ParseDiagnosticRow {
        ParseDiagnosticRow {
            severity: 2,
            code: code.to_owned(),
            line: Some(1),
            col: Some(2),
            end_line: Some(1),
            end_col: Some(3),
        }
    }

    #[tokio::test]
    async fn rows_are_capped_and_written_once() {
        let store = MemParseDiagnostics::new();
        let scope = scope();
        let many: Vec<ParseDiagnosticRow> = (0..80).map(|_| row("syntax_error")).collect();
        let written = store.write(&scope, &[(1, many.clone())]).await.unwrap();
        assert_eq!(written, MAX_DIAGNOSTICS_PER_FILE as u64);
        assert_eq!(store.read(&scope, 1).await.unwrap().len(), 50);
        // A second write for the same file version is a no-op.
        assert_eq!(store.write(&scope, &[(1, many)]).await.unwrap(), 0);
    }

    #[tokio::test]
    async fn tenants_do_not_see_each_others_rows() {
        let store = MemParseDiagnostics::new();
        let (a, b) = (scope(), scope());
        store.write(&a, &[(7, vec![row("panic")])]).await.unwrap();
        assert_eq!(store.read(&a, 7).await.unwrap().len(), 1);
        assert!(store.read(&b, 7).await.unwrap().is_empty());
    }
}
