//! Greedy selection by score per token, pinned items first. The token budget is never exceeded.

use super::resolve::BudgetLimits;
use crate::model::{ContextItem, ItemContent, ItemKind, OmitReason, OmittedItem};

/// A gated candidate with an optional cheaper compressed form (from CTX-007).
#[derive(Debug, Clone)]
pub struct Candidate {
    pub item: ContextItem,
    pub cheaper: Option<(ItemContent, u32)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Selection {
    pub selected: Vec<ContextItem>,
    pub omitted: Vec<OmittedItem>,
    pub tokens_used: u32,
}

#[derive(Clone, Copy)]
enum Category {
    Changed,
    Related,
    Tests,
    Configs,
    RuleDocs,
}

impl Category {
    fn of(kind: ItemKind) -> Self {
        match kind {
            ItemKind::ChangedSymbol | ItemKind::ClassSignature => Self::Changed,
            ItemKind::RelatedSymbol
            | ItemKind::ApiEndpoint
            | ItemKind::SchemaChange
            | ItemKind::DependencyChange => Self::Related,
            ItemKind::Test => Self::Tests,
            ItemKind::Config => Self::Configs,
            ItemKind::RuleDoc | ItemKind::Convention => Self::RuleDocs,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Changed => "changed",
            Self::Related => "related",
            Self::Tests => "tests",
            Self::Configs => "configs",
            Self::RuleDocs => "rule_docs",
        }
    }

    fn cap(self, l: &BudgetLimits) -> usize {
        usize::from(match self {
            Self::Changed => l.max_changed,
            Self::Related => l.max_related,
            Self::Tests => l.max_tests,
            Self::Configs => l.max_configs,
            Self::RuleDocs => l.max_rule_docs,
        })
    }

    fn index(self) -> usize {
        self as usize
    }
}

fn pinned_rank(kind: ItemKind) -> u8 {
    match kind {
        ItemKind::ChangedSymbol => 0,
        ItemKind::ClassSignature => 1,
        _ => 2,
    }
}

fn omit(item: &ContextItem, reason: OmitReason) -> OmittedItem {
    OmittedItem {
        node: item.node,
        title: item.title.clone(),
        kind: item.kind,
        score: item.score,
        reason,
    }
}

/// Picks items for one package. Every input candidate ends up either selected or omitted with a
/// reason, exactly once. `sum(selected.tokens) <= limits.max_tokens` always holds.
pub fn select(candidates: Vec<Candidate>, limits: &BudgetLimits) -> Selection {
    let (mut pinned, mut rest): (Vec<_>, Vec<_>) =
        candidates.into_iter().partition(|c| c.item.pinned);

    // Pinned: changed symbols by score (risk) desc, then signatures, then other pinned items.
    pinned.sort_by(|a, b| {
        pinned_rank(a.item.kind)
            .cmp(&pinned_rank(b.item.kind))
            .then_with(|| b.item.score.total_cmp(&a.item.score))
            .then_with(|| a.item.id.cmp(&b.item.id))
    });
    // Others: score per token desc, ties by score desc then id.
    let ratio = |c: &Candidate| c.item.score / c.item.tokens.max(1) as f32;
    rest.sort_by(|a, b| {
        ratio(b)
            .total_cmp(&ratio(a))
            .then_with(|| b.item.score.total_cmp(&a.item.score))
            .then_with(|| a.item.id.cmp(&b.item.id))
    });

    let mut selected = Vec::new();
    let mut omitted = Vec::new();
    let mut used: u32 = 0;
    let mut counts = [0usize; 5];

    for (is_pinned, group) in [(true, pinned), (false, rest)] {
        for cand in group {
            let cat = Category::of(cand.item.kind);
            if counts[cat.index()] >= cat.cap(limits) {
                omitted.push(omit(
                    &cand.item,
                    OmitReason::BudgetCount {
                        category: cat.name().to_string(),
                    },
                ));
                continue;
            }
            let room = limits.max_tokens.saturating_sub(used);
            let mut item = cand.item;
            let fits_default = item.tokens <= room;
            let take_cheaper = !fits_default
                && (is_pinned || item.score >= 0.5)
                && cand.cheaper.as_ref().is_some_and(|(_, t)| *t <= room);
            if fits_default {
                // keep as is
            } else if take_cheaper {
                if let Some((content, t)) = cand.cheaper {
                    item.content = content;
                    item.tokens = t;
                }
            } else {
                omitted.push(omit(&item, OmitReason::BudgetTokens));
                continue;
            }
            used = used.saturating_add(item.tokens);
            counts[cat.index()] += 1;
            selected.push(item);
        }
    }
    selected.sort_by(ContextItem::package_order);
    omitted.sort_by(OmittedItem::list_order);
    Selection {
        selected,
        omitted,
        tokens_used: used,
    }
}
