//! Scope narrowing and exception detection (PROF-003).

use globset::{Glob, GlobMatcher};

use super::confidence::{consistency, SCOPE_CONSISTENCY};
use super::miner::Sample;

/// A sample with its verdict (`true` = conforms). Not-applicable samples are dropped earlier.
pub type Judged = (Sample, bool);

fn matcher(glob: &str) -> Option<GlobMatcher> {
    Glob::new(glob).ok().map(|g| g.compile_matcher())
}

/// Candidate scopes from broadest to tightest: the source root, `src/modules/**`, then one
/// glob per module (sorted, deduplicated).
pub fn candidate_scopes(samples: &[Judged]) -> Vec<String> {
    let mut modules: Vec<&str> = samples.iter().map(|(s, _)| s.module.as_str()).collect();
    modules.sort_unstable();
    modules.dedup();
    let mut out = vec!["**".to_owned(), "src/modules/**".to_owned()];
    out.extend(
        modules
            .into_iter()
            .filter(|m| !m.is_empty())
            .map(|m| format!("{m}/**")),
    );
    out
}

/// Picks the broadest candidate scope whose samples reach [`SCOPE_CONSISTENCY`]; falls back to
/// the source root when none does.
pub fn narrow_scope(samples: &[Judged]) -> String {
    for scope in candidate_scopes(samples) {
        let Some(m) = matcher(&scope) else { continue };
        let (mut ok, mut bad) = (0u32, 0u32);
        for (s, conforms) in samples {
            if m.is_match(&s.path) {
                if *conforms {
                    ok += 1;
                } else {
                    bad += 1;
                }
            }
        }
        if ok + bad > 0 && consistency(ok, bad) >= SCOPE_CONSISTENCY {
            return scope;
        }
    }
    "**".to_owned()
}

/// Why a violating sample is an exception rather than a violation.
pub fn exception_reason(
    sample: &Sample,
    convention_id: &str,
    generated: &[GlobMatcher],
    configured: &[GlobMatcher],
) -> Option<&'static str> {
    if sample.ignore_annotations.iter().any(|a| a == convention_id) {
        Some("review-ignore annotation")
    } else if generated.iter().any(|g| g.is_match(&sample.path)) {
        Some("generated file")
    } else if configured.iter().any(|g| g.is_match(&sample.path)) {
        Some("configured exception")
    } else {
        None
    }
}

/// Compiles globs, skipping invalid ones (config validation reports them elsewhere).
pub fn compile_all(globs: &[String]) -> Vec<GlobMatcher> {
    globs.iter().filter_map(|g| matcher(g)).collect()
}
