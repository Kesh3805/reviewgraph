//! [`reconcile`]: local diff vs provider file list (DIFF-005).

use std::collections::BTreeMap;

use review_core::location::RepoPath;

use super::lineset::LineSet;
use super::patch::parse_patch;
use super::{AnchorMap, AnchorSource, Discrepancy, FileAnchors, ProviderFileDiff, Reconciled};
use crate::hunks::LineKind;
use crate::metrics;
use crate::model::{DiffModel, FileDiff};

/// Files without a provider patch fall back to local hunks only below this many changed lines.
pub const LOCAL_FALLBACK_MAX_LINES: u32 = 3_000;

/// Compare the local diff with the provider's file list and build per-file anchor sets.
pub fn reconcile(local: &DiffModel, provider: &[ProviderFileDiff]) -> Reconciled {
    let span = tracing::info_span!(
        "diff.reconcile",
        files_local = local.files.len(),
        files_provider = provider.len(),
        discrepancies = tracing::field::Empty,
        anchorable_files = tracing::field::Empty,
    );
    let _enter = span.enter();

    let local_by_path: BTreeMap<&str, &FileDiff> = local
        .files
        .iter()
        .map(|f| (f.file.path.as_str(), f))
        .collect();
    let truncated = provider.iter().any(|p| p.truncated_list);
    let mut anchors = AnchorMap::default();
    let mut discrepancies = Vec::new();
    let mut seen: Vec<&str> = Vec::with_capacity(provider.len());

    for pf in provider {
        let local_file = local_by_path.get(pf.path.as_str()).copied();
        seen.push(pf.path.as_str());
        match local_file {
            None => discrepancies.push(Discrepancy::ProviderOnly {
                path: pf.path.clone(),
            }),
            Some(lf) => {
                if lf.file.status != pf.status {
                    discrepancies.push(Discrepancy::StatusMismatch {
                        path: pf.path.clone(),
                        local: lf.file.status,
                        provider: pf.status,
                    });
                }
                if let Some(lines) = lf.lines {
                    let l = (lines.additions, lines.deletions);
                    let p = (pf.additions, pf.deletions);
                    if l != p {
                        discrepancies.push(Discrepancy::StatsMismatch {
                            path: pf.path.clone(),
                            local: l,
                            provider: p,
                        });
                    }
                }
            }
        }
        let old_path = pf
            .old_path
            .clone()
            .or_else(|| local_file.and_then(|f| f.file.old_path.clone()));
        let parsed = pf.patch.as_deref().map(parse_patch);
        let file_anchors = match parsed {
            Some(Ok(patch)) => {
                if let Some(lf) = local_file {
                    if lf.disposition.is_analyze() && !lf.hunks.is_empty() {
                        let (right, left) = local_sets(lf);
                        if right != patch.right || left != patch.left {
                            discrepancies.push(Discrepancy::HunkBoundaryDiffers {
                                path: pf.path.clone(),
                            });
                        }
                    }
                }
                FileAnchors {
                    right: patch.right,
                    left: patch.left,
                    old_path,
                    source: AnchorSource::ProviderPatch,
                    anchorable: true,
                }
            }
            Some(Err(_)) | None => {
                discrepancies.push(Discrepancy::PatchMissing {
                    path: pf.path.clone(),
                });
                let small = pf.additions.saturating_add(pf.deletions) < LOCAL_FALLBACK_MAX_LINES;
                match local_file {
                    Some(lf) if small && lf.disposition.is_analyze() && !lf.hunks.is_empty() => {
                        let (right, left) = local_sets(lf);
                        FileAnchors {
                            right,
                            left,
                            old_path,
                            source: AnchorSource::LocalHunks,
                            anchorable: true,
                        }
                    }
                    _ => FileAnchors::unanchorable(old_path),
                }
            }
        };
        anchors.files.insert(pf.path.clone(), file_anchors);
    }

    seen.sort_unstable();
    for lf in &local.files {
        if seen.binary_search(&lf.file.path.as_str()).is_ok() {
            continue;
        }
        if !truncated {
            discrepancies.push(Discrepancy::LocalOnly {
                path: lf.file.path.clone(),
            });
        }
        anchors.files.insert(
            lf.file.path.clone(),
            FileAnchors::unanchorable(lf.file.old_path.clone()),
        );
    }
    if truncated {
        discrepancies.push(Discrepancy::ProviderListTruncated);
    }

    discrepancies.sort_by(|a, b| sort_key(a).cmp(&sort_key(b)));
    for d in &discrepancies {
        metrics::record_discrepancy(d.label());
    }
    let anchorable = anchors.files.values().filter(|a| a.anchorable).count();
    metrics::record_unanchorable((anchors.files.len() - anchorable) as u64);
    span.record("discrepancies", discrepancies.len());
    span.record("anchorable_files", anchorable);
    Reconciled {
        anchors,
        discrepancies,
    }
}

fn sort_key(d: &Discrepancy) -> (&str, &'static str) {
    (d.path().map_or("", RepoPath::as_str), d.label())
}

/// RIGHT and LEFT sets of the local hunks (context + added, context + deleted).
pub fn local_sets(file: &FileDiff) -> (LineSet, LineSet) {
    let mut right = Vec::new();
    let mut left = Vec::new();
    for hunk in &file.hunks {
        for line in &hunk.lines {
            match line.kind {
                LineKind::Context => {
                    right.extend(line.new_no);
                    left.extend(line.old_no);
                }
                LineKind::Add => right.extend(line.new_no),
                LineKind::Del => left.extend(line.old_no),
            }
        }
    }
    (LineSet::from_lines(right), LineSet::from_lines(left))
}
