# ADR-006 — Language analyzer protocol

**Status:** Accepted · 2026-10-02

## Context
The graph must stay language-neutral (Invariant 5, PRD §95), and languages are added over time.

## Decision
```rust
trait LanguageAnalyzer: Send + Sync {
    fn language(&self) -> Language;
    fn version(&self) -> AnalyzerVersion;          // bump => re-parse that language only
    fn supports(&self, path: &RepoPath) -> bool;
    fn analyze(&self, input: &SourceInput, cfg: &AnalyzerConfig) -> Result<ParsedUnit, AnalyzeError>;
}
trait FrameworkAdapter: Send + Sync {               // runs over the syntax tree inside an analyzer
    fn name(&self) -> &'static str;
    fn detect(&self, repo: &RepoFacts) -> bool;
    fn extract(&self, ctx: &mut FrameworkCtx);      // emits IrFrameworkFact
}
trait ModuleResolver { fn resolve(&self, from: &RepoPath, specifier: &str) -> Resolution; }
trait SemanticProvider { fn resolve_ambiguous(&self, refs: &[AmbiguousRef]) -> Vec<SemanticResolution>; }
```

Rules:
- **Analyzers are pure and work on one file at a time.** They do no I/O beyond their input, so `ParsedUnit` can be cached by `(path, content_hash, analyzer_version)`.
- **The IR in `analysis-ir` is language-neutral.** Framework facts appear only as `IrFrameworkFact { kind, attrs }`. The linker maps these onto generic node and edge kinds such as APIEndpoint, HANDLED_BY and PRODUCES_JOB.
- **Parse errors are tolerated.** tree-sitter error nodes become `ParseDiagnostic` values, and partial symbols are still emitted.
- **Compiler helpers run out of process.** Helpers such as the TypeScript compiler, gopls, rust-analyzer or JDT speak JSON lines over stdio. Each starts with a version handshake, runs under a timeout, and is always optional.

## Alternatives
| Option | Rejected because |
|---|---|
| LSP as the primary protocol | Too chatty and stateful for batch indexing, and its semantics vary between servers. |
| Language logic inside the graph core | Violates Invariant 5. |

## Consequences
Adding a language means three things:
- a new crate that implements the trait
- fixture repositories
- golden IR tests
