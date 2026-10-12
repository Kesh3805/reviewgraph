//! Framework and graph signals (RISK-003): what a changed symbol reaches and what it calls.
//!
//! Call-based rules read the added and removed calls of the symbol (bound to `pkg:` targets by
//! the call binder); reachability and queue rules read the impact graph. Without an impact
//! graph the graph rules are skipped and the result is flagged `degraded`.

use codegraph::NodeKind;

use super::new_signal;
use super::policy::RiskPolicy;
use super::signal::{dedup_signals, RiskSignal, SignalEvidence, SignalSubject};
use crate::graph::{EntryKind, ImpactGraph, Relation, ResourceRole, SymbolImpact};
use crate::input::{facts, CallInput, ChangeSet, SymbolInput};

const HTTP_CLIENTS: &[&str] = &[
    "pkg:npm/axios",
    "pkg:npm/node-fetch",
    "pkg:npm/got",
    "pkg:npm/undici",
];
const FILESYSTEM: &[&str] = &["pkg:node/fs", "pkg:npm/fs-extra"];
const CRYPTO: &[&str] = &[
    "pkg:node/crypto",
    "pkg:npm/bcrypt",
    "pkg:npm/bcryptjs",
    "pkg:npm/argon2",
    "pkg:npm/jsonwebtoken",
    "pkg:npm/jose",
];
const JWT: &[&str] = &["pkg:npm/jsonwebtoken", "pkg:npm/jose"];
const SERIALIZERS: &[&str] = &["pkg:npm/class-transformer"];
const PAYMENT_CLIENTS: &[&str] = &["pkg:npm/stripe"];

/// The outcome of the graph rules.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GraphDetection {
    pub signals: Vec<RiskSignal>,
    /// No impact graph: reachability and queue rules were skipped.
    pub degraded: bool,
}

/// `id` is `package` or a sub-path of it (`pkg:node/fs` matches `pkg:node/fs/promises`).
fn pkg_is(id: &str, package: &str) -> bool {
    id == package
        || id
            .strip_prefix(package)
            .is_some_and(|rest| rest.starts_with('/'))
}

fn calls_package(call: &CallInput, packages: &[&str]) -> bool {
    call.target_id
        .as_deref()
        .is_some_and(|id| packages.iter().any(|package| pkg_is(id, package)))
}

fn add(
    out: &mut Vec<RiskSignal>,
    symbol: &SymbolInput,
    rule_id: &str,
    confidence: f32,
    detail: &str,
) {
    out.extend(new_signal(
        rule_id,
        SignalSubject::Symbol(symbol.key()),
        confidence,
        vec![SignalEvidence {
            file: Some(symbol.path().clone()),
            symbol: Some(symbol.key()),
            node_id: Some(symbol.id().to_owned()),
            detail: detail.to_owned(),
            ..SignalEvidence::default()
        }],
    ));
}

/// The strongest confidence among the calls of `packages`, or `None` when there is none.
fn best_confidence<'a>(
    calls: impl Iterator<Item = &'a CallInput>,
    packages: &[&str],
) -> Option<f32> {
    calls
        .filter(|call| calls_package(call, packages))
        .map(|call| call.confidence)
        .fold(None, |best: Option<f32>, c| {
            Some(best.map_or(c, |b| b.max(c)))
        })
}

fn graph_rules(out: &mut Vec<RiskSignal>, symbol: &SymbolInput, impact: Option<&SymbolImpact>) {
    let elements = impact.map_or(&[][..], |impact| impact.elements.as_slice());
    if elements.iter().any(|element| {
        element.relation == Relation::Endpoint
            && element
                .endpoint
                .as_ref()
                .is_some_and(|endpoint| endpoint.entry_kind == EntryKind::Http)
    }) {
        add(
            out,
            symbol,
            "endpoint.reachable",
            1.0,
            "an HTTP endpoint reaches this symbol",
        );
    }
    let produces = symbol.kind == Some(NodeKind::QueueProducer)
        || elements.iter().any(|element| {
            element.relation == Relation::QueueProducer
                && element
                    .resource
                    .is_some_and(|resource| resource.role == ResourceRole::Produces)
        });
    if produces {
        add(
            out,
            symbol,
            "queue.produce_changed",
            1.0,
            "a symbol producing to a queue changed",
        );
    }
    let consumes = matches!(
        symbol.kind,
        Some(NodeKind::JobHandler | NodeKind::QueueConsumer)
    ) || elements.iter().any(|element| {
        element.relation == Relation::QueueConsumer
            && element
                .resource
                .is_some_and(|resource| resource.role == ResourceRole::Consumes)
    });
    if consumes {
        add(
            out,
            symbol,
            "queue.consume_changed",
            1.0,
            "a queue consumer or job handler changed",
        );
    }
}

fn call_rules(out: &mut Vec<RiskSignal>, symbol: &SymbolInput, policy: &RiskPolicy) {
    let all_calls = || symbol.added_calls.iter().chain(symbol.removed_calls.iter());

    let http_text = symbol.added_calls.iter().any(|call| {
        call.callee_text.contains("HttpService") || call.callee_text.contains("httpService")
    });
    if let Some(confidence) = best_confidence(symbol.added_calls.iter(), HTTP_CLIENTS) {
        add(
            out,
            symbol,
            "external.call_added",
            confidence,
            "an outbound HTTP call was added",
        );
    } else if http_text {
        add(
            out,
            symbol,
            "external.call_added",
            1.0,
            "an outbound HTTP call was added",
        );
    }

    if let Some(confidence) = best_confidence(all_calls(), FILESYSTEM) {
        add(
            out,
            symbol,
            "fs.access_changed",
            confidence,
            "filesystem calls were added or removed",
        );
    } else if symbol.facts.contains(facts::REQUEST_DERIVED_PATH) {
        add(
            out,
            symbol,
            "fs.access_changed",
            1.0,
            "a filesystem path derives from the request",
        );
    }

    if let Some(confidence) = best_confidence(all_calls(), CRYPTO) {
        add(
            out,
            symbol,
            "crypto.changed",
            confidence,
            "cryptographic calls were added or removed",
        );
    }
    let jwt_verify_removed = symbol
        .removed_calls
        .iter()
        .filter(|call| calls_package(call, JWT))
        .map(|call| call.callee_text.to_ascii_lowercase())
        .any(|text| text.contains("verify"));
    if jwt_verify_removed {
        add(
            out,
            symbol,
            "auth.jwt_verify_removed",
            1.0,
            "a JWT verification call was removed",
        );
    }

    if symbol.facts.contains(facts::JSON_PARSE_EXTERNAL)
        || best_confidence(all_calls(), SERIALIZERS).is_some()
    {
        add(
            out,
            symbol,
            "serialization.changed",
            1.0,
            "serialization of external data changed",
        );
    }

    if symbol.facts.contains(facts::AUTHN_STRATEGY) {
        add(
            out,
            symbol,
            "auth.authn_changed",
            1.0,
            "an authentication strategy or guard changed",
        );
    }

    if policy.payment_paths.is_match(symbol.path().as_str())
        || best_confidence(all_calls(), PAYMENT_CLIENTS).is_some()
    {
        add(out, symbol, "payments.changed", 1.0, "payment code changed");
    }

    if symbol.facts.contains(facts::SHARED_STATE_WRITE)
        || symbol.facts.contains(facts::PROMISE_ALL_DB_WRITES)
    {
        add(
            out,
            symbol,
            "concurrency.shared_state",
            1.0,
            "shared mutable state or concurrent database writes",
        );
    }
}

/// Framework and graph signals of every changed, non-cosmetic, non-test symbol.
pub fn detect_graph_signals(
    changes: &ChangeSet,
    impact: Option<&ImpactGraph>,
    policy: &RiskPolicy,
) -> GraphDetection {
    let mut out = Vec::new();
    for symbol in &changes.symbols {
        if symbol.is_skipped() || symbol.test {
            continue;
        }
        let symbol_impact = impact.and_then(|graph| graph.symbol(symbol.key()));
        graph_rules(&mut out, symbol, symbol_impact);
        call_rules(&mut out, symbol, policy);
    }
    GraphDetection {
        signals: dedup_signals(out),
        degraded: impact.is_none(),
    }
}
