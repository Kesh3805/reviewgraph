use std::time::Duration;

/// The six budgeted resources.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum BudgetKind {
    Symbols,
    GraphExpansion,
    ModelTokens,
    Candidates,
    ModelCalls,
    Latency,
}

impl BudgetKind {
    pub const ALL: [BudgetKind; 6] = [
        Self::Symbols,
        Self::GraphExpansion,
        Self::ModelTokens,
        Self::Candidates,
        Self::ModelCalls,
        Self::Latency,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Symbols => "symbols",
            Self::GraphExpansion => "graph_expansion",
            Self::ModelTokens => "model_tokens",
            Self::Candidates => "candidates",
            Self::ModelCalls => "model_calls",
            Self::Latency => "latency",
        }
    }

    pub(crate) const fn index(self) -> usize {
        self as usize
    }
}

/// Per-run limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReviewBudget {
    pub max_reviewed_symbols: u32,
    pub max_graph_expansion_nodes: u32,
    pub max_model_tokens: u64,
    pub max_candidate_findings: u32,
    pub max_model_calls: u32,
    pub max_review_latency: Duration,
}

impl ReviewBudget {
    /// Deployment hard caps; repository overrides can only go below these.
    pub const HARD_CAPS: ReviewBudget = ReviewBudget {
        max_reviewed_symbols: 400,
        max_graph_expansion_nodes: 5_000,
        max_model_tokens: 400_000,
        max_candidate_findings: 50,
        max_model_calls: 40,
        max_review_latency: Duration::from_secs(300),
    };

    /// Plan defaults (equal to the hard caps in v1).
    pub const DEFAULT: ReviewBudget = Self::HARD_CAPS;

    /// Clamp a repository override to `caps`. Returns the clamped budget and the names of the
    /// fields that were clamped (the caller logs a config warning, not an error).
    pub fn clamp_to(self, caps: &ReviewBudget) -> (ReviewBudget, Vec<&'static str>) {
        let mut clamped = Vec::new();
        macro_rules! c {
            ($f:ident) => {
                if self.$f > caps.$f {
                    clamped.push(stringify!($f));
                }
            };
        }
        c!(max_reviewed_symbols);
        c!(max_graph_expansion_nodes);
        c!(max_model_tokens);
        c!(max_candidate_findings);
        c!(max_model_calls);
        c!(max_review_latency);
        let out = ReviewBudget {
            max_reviewed_symbols: self.max_reviewed_symbols.min(caps.max_reviewed_symbols),
            max_graph_expansion_nodes: self
                .max_graph_expansion_nodes
                .min(caps.max_graph_expansion_nodes),
            max_model_tokens: self.max_model_tokens.min(caps.max_model_tokens),
            max_candidate_findings: self.max_candidate_findings.min(caps.max_candidate_findings),
            max_model_calls: self.max_model_calls.min(caps.max_model_calls),
            max_review_latency: self.max_review_latency.min(caps.max_review_latency),
        };
        (out, clamped)
    }

    pub(crate) fn limit(&self, kind: BudgetKind) -> u64 {
        match kind {
            BudgetKind::Symbols => self.max_reviewed_symbols.into(),
            BudgetKind::GraphExpansion => self.max_graph_expansion_nodes.into(),
            BudgetKind::ModelTokens => self.max_model_tokens,
            BudgetKind::Candidates => self.max_candidate_findings.into(),
            BudgetKind::ModelCalls => self.max_model_calls.into(),
            BudgetKind::Latency => self.max_review_latency.as_millis() as u64,
        }
    }
}
