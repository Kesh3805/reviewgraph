//! Analyzer lookup by path.

use std::fmt;
use std::sync::Arc;

use review_core::location::RepoPath;

use crate::traits::LanguageAnalyzer;

/// Analyzers in registration order, which the composition root fixes. The first analyzer that
/// supports a path wins.
#[derive(Clone, Default)]
pub struct AnalyzerRegistry {
    analyzers: Vec<Arc<dyn LanguageAnalyzer>>,
}

impl fmt::Debug for AnalyzerRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list()
            .entries(self.analyzers.iter().map(|a| a.id().name))
            .finish()
    }
}

impl AnalyzerRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, analyzer: Arc<dyn LanguageAnalyzer>) {
        self.analyzers.push(analyzer);
    }

    pub fn for_path(&self, path: &RepoPath) -> Option<&dyn LanguageAnalyzer> {
        self.analyzers
            .iter()
            .find(|a| a.supports(path))
            .map(|a| a.as_ref())
    }

    pub fn analyzers(&self) -> &[Arc<dyn LanguageAnalyzer>] {
        &self.analyzers
    }

    pub fn len(&self) -> usize {
        self.analyzers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.analyzers.is_empty()
    }
}
