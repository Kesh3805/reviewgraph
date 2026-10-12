//! Hand-built [`ParsedUnit`]s for symbol-mapping and change-classification tests (DIFF-006,
//! CHG-001..005), independent of any analyzer.

use std::collections::BTreeMap;

use analysis_ir::facts::{FactKind, SymbolFacts, SyntaxFact};
use analysis_ir::symbol::{AttrValue, IrDecorator, IrSymbol, LocalId, Modifiers};
use analysis_ir::unit::{AnalyzerId, ParseStatus, ParsedUnit, UnitStats};
use review_core::language::{Dialect, Language};
use review_core::location::{ContentHash, Position, RepoPath, SourceRange};
use review_core::symbol::{Hash128, ModulePath, SymbolKind};
use review_core::version::AnalyzerVersion;

/// A range covering whole lines `start..=end` (end column 1 so the end line counts).
pub fn lines(start: u32, end: u32) -> SourceRange {
    SourceRange {
        start: Position {
            line: start.max(1),
            column: 0,
        },
        end: Position {
            line: end.max(start).max(1),
            column: 1,
        },
    }
}

/// A [`ParsedUnit`] under construction; `symbols[0]` is the module symbol.
#[derive(Debug, Clone)]
pub struct UnitBuilder {
    unit: ParsedUnit,
}

impl UnitBuilder {
    /// A TypeScript unit at `path` with `line_count` lines.
    pub fn new(path: RepoPath, line_count: u32) -> Self {
        let module = IrSymbol::new(
            LocalId(0),
            SymbolKind::Module,
            "__module__",
            vec!["__module__".to_owned()],
            lines(1, line_count.max(1)),
        );
        let unit = ParsedUnit {
            ir_schema: analysis_ir::unit::IR_SCHEMA_VERSION,
            module_path: ModulePath::of(&path),
            content_hash: ContentHash::of(path.as_str().as_bytes()),
            file: path,
            language: Language::Typescript,
            dialect: Some(Dialect::Ts),
            analyzer: AnalyzerId {
                name: "testkit".to_owned(),
                version: AnalyzerVersion::new(0, 0, 1),
            },
            status: ParseStatus::Ok,
            symbols: vec![module],
            references: Vec::new(),
            imports: Vec::new(),
            exports: Vec::new(),
            framework: Vec::new(),
            facts: Vec::new(),
            diagnostics: Vec::new(),
            stats: UnitStats {
                bytes: 0,
                lines: line_count,
                parse_micros: 0,
            },
        };
        Self { unit }
    }

    /// Append a symbol spanning `start..=end`; its body starts on the line after `start`.
    /// Returns its local id.
    pub fn symbol(
        &mut self,
        kind: SymbolKind,
        name: &str,
        parent: Option<u32>,
        start: u32,
        end: u32,
    ) -> u32 {
        let local = self.unit.symbols.len() as u32;
        let mut qualified = parent
            .and_then(|p| self.unit.symbols.get(p as usize))
            .map(|p| p.qualified_name.clone())
            .unwrap_or_default();
        qualified.push(name.to_owned());
        let mut symbol = IrSymbol::new(LocalId(local), kind, name, qualified, lines(start, end));
        symbol.parent = parent.map(LocalId);
        if end > start {
            symbol.body_range = Some(lines(start + 1, end));
        }
        symbol.signature = Some(format!("{name}()"));
        symbol.signature_hash = Hash128::of("sig", name.as_bytes());
        symbol.body_hash = Hash128::of("body", name.as_bytes());
        symbol.attr_hash = Hash128::of("attr", b"");
        self.unit.symbols.push(symbol);
        local
    }

    /// Mutable access to a symbol.
    pub fn get_mut(&mut self, local: u32) -> Option<&mut IrSymbol> {
        self.unit.symbols.get_mut(local as usize)
    }

    /// Set the three hashes of a symbol from short strings.
    pub fn hashes(&mut self, local: u32, signature: &str, body: &str, attrs: &str) -> &mut Self {
        if let Some(s) = self.get_mut(local) {
            s.signature_hash = Hash128::of("sig", signature.as_bytes());
            s.body_hash = Hash128::of("body", body.as_bytes());
            s.attr_hash = Hash128::of("attr", attrs.as_bytes());
        }
        self
    }

    /// Add a decorator on `line` to a symbol.
    pub fn decorator(&mut self, local: u32, name: &str, line: u32) -> &mut Self {
        if let Some(s) = self.get_mut(local) {
            s.decorators.push(IrDecorator {
                name: name.to_owned(),
                args: Vec::new(),
                range: lines(line, line),
            });
        }
        self
    }

    /// Add modifiers to a symbol.
    pub fn modifiers(&mut self, local: u32, modifiers: Modifiers) -> &mut Self {
        if let Some(s) = self.get_mut(local) {
            s.modifiers.insert(modifiers);
        }
        self
    }

    /// Append a syntax fact to a symbol.
    pub fn fact(&mut self, local: u32, kind: FactKind, key: &str, line: u32) -> &mut Self {
        self.fact_with(local, kind, key, line, BTreeMap::new())
    }

    /// Append a syntax fact with detail attributes.
    pub fn fact_with(
        &mut self,
        local: u32,
        kind: FactKind,
        key: &str,
        line: u32,
        detail: BTreeMap<String, AttrValue>,
    ) -> &mut Self {
        let fact = SyntaxFact {
            kind,
            key: key.to_owned(),
            range: lines(line, line),
            detail,
        };
        match self
            .unit
            .facts
            .iter_mut()
            .find(|f| f.symbol == LocalId(local))
        {
            Some(entry) => entry.facts.push(fact),
            None => {
                self.unit.facts.push(SymbolFacts {
                    symbol: LocalId(local),
                    facts: vec![fact],
                });
                self.unit.facts.sort_by_key(|f| f.symbol);
            }
        }
        self
    }

    /// Mark the unit as failed to parse.
    pub fn failed(&mut self) -> &mut Self {
        self.unit.status = ParseStatus::Failed {
            reason: analysis_ir::unit::FailReason::Internal,
        };
        self
    }

    /// The finished unit.
    pub fn build(&self) -> ParsedUnit {
        self.unit.clone()
    }
}
