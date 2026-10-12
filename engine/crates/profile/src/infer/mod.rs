//! Inference from the graph: roles, layers, modules (PROF-002).

pub mod architecture;
pub mod roles;

use codegraph::GraphQuery;

use crate::config::ReviewConfigV1;
use crate::facts::FactIndex;

/// A framework module declaration (for example a NestJS `@Module` class), from IR framework
/// facts: the graph has no node kind for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleDeclaration {
    pub name: String,
    /// Repository-relative file declaring it.
    pub file: String,
    /// Exported provider names.
    pub exports: Vec<String>,
}

/// Everything profile computation reads. All of it is immutable for the run.
#[derive(Clone, Copy)]
pub struct ProfileInputs<'a> {
    pub graph: &'a dyn GraphQuery,
    pub facts: &'a FactIndex,
    pub config: &'a ReviewConfigV1,
    pub module_declarations: &'a [ModuleDeclaration],
    /// Workspace package root directories (INIT-005).
    pub workspace_packages: &'a [String],
    /// Source roots (`src`, `apps/api/src`, ...). Empty means "infer".
    pub source_roots: &'a [String],
    /// Test frameworks detected by init (`jest`, `vitest`, ...).
    pub test_frameworks: &'a [String],
}

impl std::fmt::Debug for ProfileInputs<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProfileInputs")
            .field("nodes", &self.graph.node_count())
            .field("module_declarations", &self.module_declarations.len())
            .field("workspace_packages", &self.workspace_packages)
            .field("source_roots", &self.source_roots)
            .finish()
    }
}

impl<'a> ProfileInputs<'a> {
    /// Inputs with only a graph, empty facts and the default config.
    pub fn graph_only(graph: &'a dyn GraphQuery, facts: &'a FactIndex, config: &'a ReviewConfigV1) -> Self {
        Self {
            graph,
            facts,
            config,
            module_declarations: &[],
            workspace_packages: &[],
            source_roots: &[],
            test_frameworks: &[],
        }
    }
}
