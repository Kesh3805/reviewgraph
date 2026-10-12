//! Path signals (RISK-002): configured `risk.paths`, the default path rules and payment paths.
//!
//! A path signal is about a file; scoring applies it to the symbols and clusters of that file.

use review_core::location::RepoPath;

use super::new_signal;
use super::policy::{default_path_rules, level_weight, RiskPolicy};
use super::signal::{dedup_signals, RiskSignal, SignalEvidence, SignalSubject};
use crate::input::{ChangeSet, FileDisposition};

fn evidence(path: &RepoPath, detail: String) -> Vec<SignalEvidence> {
    vec![SignalEvidence {
        file: Some(path.clone()),
        detail,
        ..SignalEvidence::default()
    }]
}

/// Path signals of every changed file. Generated and vendored files are skipped.
pub fn detect_path_signals(changes: &ChangeSet, policy: &RiskPolicy) -> Vec<RiskSignal> {
    let mut out = Vec::new();
    for file in &changes.files {
        if matches!(
            file.disposition,
            FileDisposition::Generated | FileDisposition::Vendored | FileDisposition::Minified
        ) || policy.generated_globs.is_match(file.path.as_str())
        {
            continue;
        }
        let subject = SignalSubject::File(file.path.clone());
        if let Some(rule) = policy.paths.iter().find(|rule| rule.matches(&file.path)) {
            let signal = new_signal(
                "path.configured",
                subject.clone(),
                1.0,
                evidence(
                    &file.path,
                    format!("matches risk path {} ({})", rule.glob, rule.level),
                ),
            );
            out.extend(signal.map(|signal| {
                signal
                    .with_weight(level_weight(rule.level))
                    .with_category(rule.effective_category())
                    .with_floor(Some(rule.level))
            }));
        } else if policy.paths.is_empty() {
            for default in default_path_rules() {
                if default.globs.is_match(file.path.as_str()) {
                    out.extend(new_signal(
                        default.rule_id,
                        subject.clone(),
                        1.0,
                        evidence(
                            &file.path,
                            format!("matches default rule {}", default.rule_id),
                        ),
                    ));
                }
            }
        }
        if policy.payment_paths.is_match(file.path.as_str()) {
            out.extend(new_signal(
                "path.payments",
                subject,
                1.0,
                evidence(&file.path, "under a payment or billing path".to_owned()),
            ));
        }
    }
    dedup_signals(out)
}
