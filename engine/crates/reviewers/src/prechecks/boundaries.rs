//! REV-A-002: forbidden layer dependencies and new import cycles.
//!
//! Both checks are pure over graph edges. A *module* is the file path of a symbol id
//! (`ts:{path}#...`). The layer map comes from the repository profile (PROF-002) plus
//! `rules.layers`; this crate takes it as plain [`BoundaryConfig`].

use std::collections::{BTreeMap, BTreeSet};

use globset::{Glob, GlobSet, GlobSetBuilder};
use serde::{Deserialize, Serialize};

use super::{DeterministicFinding, PrecheckOutcome, SeverityHint};
use crate::context::{ChangedSymbol, NeighborEdge, ReviewContext};

pub const RULE_FORBIDDEN: &str = "architecture.forbidden_dependency";
pub const RULE_NEW_CYCLE: &str = "architecture.new_cycle";
/// Maximum modules in the projected module graph.
pub const MAX_MODULE_NODES: usize = 2_000;

/// Edge kinds that express a module dependency.
const DEPENDENCY_KINDS: &[&str] = &["IMPORTS", "DEPENDS_ON", "CALLS"];
/// Edge kinds projected onto the module graph for cycle detection.
const CYCLE_KINDS: &[&str] = &["IMPORTS", "DEPENDS_ON"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LayerRule {
    pub name: String,
    /// Path globs of the layer's files.
    pub globs: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoundaryConfig {
    pub layers: Vec<LayerRule>,
    /// `(from_layer, to_layer)` pairs that must not depend on each other.
    pub forbidden: Vec<(String, String)>,
}

/// The module (file path) of a symbol id: `ts:src/a.ts#A.f/method` gives `src/a.ts`.
pub fn module_of(symbol_id: &str) -> &str {
    let rest = symbol_id.split_once(':').map_or(symbol_id, |(_, r)| r);
    rest.split('#').next().unwrap_or(rest)
}

struct Layers {
    names: Vec<String>,
    sets: Vec<GlobSet>,
}

impl Layers {
    fn new(cfg: &BoundaryConfig) -> Self {
        let mut names = Vec::new();
        let mut sets = Vec::new();
        for l in &cfg.layers {
            let mut b = GlobSetBuilder::new();
            for g in &l.globs {
                if let Ok(g) = Glob::new(g) {
                    b.add(g);
                }
            }
            names.push(l.name.clone());
            sets.push(b.build().unwrap_or_else(|_| GlobSet::empty()));
        }
        Self { names, sets }
    }

    fn layer_of(&self, path: &str) -> Option<&str> {
        self.sets
            .iter()
            .position(|s| s.is_match(path))
            .map(|i| self.names[i].as_str())
    }
}

fn anchor_for<'a>(cx: &'a dyn ReviewContext, module: &str) -> Option<&'a ChangedSymbol> {
    cx.changed_symbols().iter().find(|s| s.path == module)
}

/// Flags every new dependency edge from a changed file that crosses a forbidden layer pair.
/// `base_edges` are the edges of the base graph; an edge present there is not new.
pub fn forbidden_dependency_precheck(
    cx: &dyn ReviewContext,
    base_edges: &[NeighborEdge],
    cfg: &BoundaryConfig,
) -> PrecheckOutcome {
    if cfg.layers.is_empty() {
        return PrecheckOutcome::NotExecuted {
            reason: "no layers defined or inferred".into(),
        };
    }
    let layers = Layers::new(cfg);
    let forbidden: BTreeSet<(&str, &str)> = cfg
        .forbidden
        .iter()
        .map(|(a, b)| (a.as_str(), b.as_str()))
        .collect();
    let existing: BTreeSet<(&str, &str, &str)> = base_edges
        .iter()
        .map(|e| (e.from.as_str(), e.to.as_str(), e.kind.as_str()))
        .collect();
    let mut seen: BTreeSet<(String, String)> = BTreeSet::new();
    let mut findings = Vec::new();
    for e in cx.edges() {
        if !DEPENDENCY_KINDS.contains(&e.kind.as_str()) {
            continue;
        }
        let (from, to) = (module_of(&e.from), module_of(&e.to));
        if from == to || existing.contains(&(e.from.as_str(), e.to.as_str(), e.kind.as_str())) {
            continue;
        }
        let Some(anchor) = anchor_for(cx, from) else {
            continue;
        };
        let (Some(lf), Some(lt)) = (layers.layer_of(from), layers.layer_of(to)) else {
            continue;
        };
        if lf == lt || !forbidden.contains(&(lf, lt)) || !seen.insert((from.into(), to.into())) {
            continue;
        }
        findings.push(DeterministicFinding {
            rule_id: RULE_FORBIDDEN.into(),
            reviewer: "deterministic:architecture".into(),
            symbol_key: anchor.symbol_id.clone(),
            path: anchor.path.clone(),
            anchor_range: anchor.range,
            severity_hint: SeverityHint::Medium,
            message: format!(
                "`{from}` ({lf}) depends on `{to}` ({lt}), which the layer rules forbid"
            ),
            absence: None,
            members: vec![from.to_owned(), to.to_owned()],
        });
    }
    PrecheckOutcome::Executed {
        findings,
        truncated: false,
    }
}

type Graph = BTreeMap<String, BTreeSet<String>>;

fn module_graph(edges: &[NeighborEdge]) -> Graph {
    let mut g = Graph::new();
    for e in edges {
        if !CYCLE_KINDS.contains(&e.kind.as_str()) {
            continue;
        }
        let (from, to) = (module_of(&e.from), module_of(&e.to));
        if from == to {
            continue;
        }
        g.entry(from.to_owned()).or_default().insert(to.to_owned());
        g.entry(to.to_owned()).or_default();
    }
    g
}

struct Tarjan<'g> {
    g: &'g Graph,
    index: BTreeMap<&'g str, usize>,
    low: BTreeMap<&'g str, usize>,
    on_stack: BTreeSet<&'g str>,
    stack: Vec<&'g str>,
    next: usize,
    sccs: Vec<Vec<String>>,
}

impl<'g> Tarjan<'g> {
    fn visit(&mut self, v: &'g str) {
        self.index.insert(v, self.next);
        self.low.insert(v, self.next);
        self.next += 1;
        self.stack.push(v);
        self.on_stack.insert(v);
        let g: &'g Graph = self.g;
        if let Some(succ) = g.get(v) {
            for w in succ {
                let w = w.as_str();
                if !self.index.contains_key(w) {
                    self.visit(w);
                    let lw = self.low[w];
                    let lv = self.low[v];
                    self.low.insert(v, lv.min(lw));
                } else if self.on_stack.contains(w) {
                    let iw = self.index[w];
                    let lv = self.low[v];
                    self.low.insert(v, lv.min(iw));
                }
            }
        }
        if self.low[v] == self.index[v] {
            let mut comp = Vec::new();
            while let Some(w) = self.stack.pop() {
                self.on_stack.remove(w);
                comp.push(w.to_owned());
                if w == v {
                    break;
                }
            }
            if comp.len() > 1 {
                comp.sort();
                self.sccs.push(comp);
            }
        }
    }
}

/// Strongly connected components with more than one module, members sorted.
fn cycles(g: &Graph) -> BTreeSet<Vec<String>> {
    let mut t = Tarjan {
        g,
        index: BTreeMap::new(),
        low: BTreeMap::new(),
        on_stack: BTreeSet::new(),
        stack: Vec::new(),
        next: 0,
        sccs: Vec::new(),
    };
    for v in g.keys() {
        if !t.index.contains_key(v.as_str()) {
            t.visit(v.as_str());
        }
    }
    t.sccs.into_iter().collect()
}

/// Reports import cycles that touch a changed module and are absent in the base graph.
pub fn new_cycle_precheck(cx: &dyn ReviewContext, base_edges: &[NeighborEdge]) -> PrecheckOutcome {
    let head = module_graph(cx.edges());
    let truncated = head.len() > MAX_MODULE_NODES;
    let base_cycles = cycles(&module_graph(base_edges));
    let touched: BTreeSet<&str> = cx
        .changed_symbols()
        .iter()
        .map(|s| s.path.as_str())
        .collect();
    let mut findings = Vec::new();
    for members in cycles(&head) {
        if base_cycles.contains(&members) {
            continue;
        }
        let Some(anchor) = members
            .iter()
            .find(|m| touched.contains(m.as_str()))
            .and_then(|m| anchor_for(cx, m))
        else {
            continue;
        };
        findings.push(DeterministicFinding {
            rule_id: RULE_NEW_CYCLE.into(),
            reviewer: "deterministic:architecture".into(),
            symbol_key: anchor.symbol_id.clone(),
            path: anchor.path.clone(),
            anchor_range: anchor.range,
            severity_hint: SeverityHint::Medium,
            message: format!("import cycle between {}", members.join(", ")),
            absence: None,
            members,
        });
    }
    PrecheckOutcome::Executed {
        findings,
        truncated,
    }
}
