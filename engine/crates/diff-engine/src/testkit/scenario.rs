//! Pull-request fixture scenarios (DIFF-007): `fixtures/pull-requests/<name>/{base/, patch.diff}`.
//!
//! [`Scenario::load`] reads the base tree, applies `patch.diff` in-process (a strict unified-diff
//! applier: every context and deleted line must match) and commits both trees into a throwaway
//! repository with the fixed fixture identity and timestamp, so the SHAs are identical on every
//! machine. The engine under test never calls `git`; the patch and the `git diff` references
//! under `expected/` were produced once with git and are committed.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use review_core::ids::CommitSha;

use super::{FileSpec, FixtureRepo, RepoBuilder};
use crate::git::GitError;

/// Failures while loading a scenario.
#[derive(Debug, thiserror::Error)]
pub enum ScenarioError {
    /// A fixture file could not be read.
    #[error("fixture io {path}: {source}")]
    Io {
        /// The file.
        path: PathBuf,
        /// The cause.
        source: std::io::Error,
    },
    /// `patch.diff` does not apply to `base/`.
    #[error("patch does not apply: {0}")]
    Patch(String),
    /// Building the repository failed.
    #[error(transparent)]
    Git(#[from] GitError),
}

/// A built scenario: both trees and the two-commit repository.
#[derive(Debug)]
pub struct Scenario {
    /// Scenario directory name.
    pub name: String,
    /// The scenario directory.
    pub dir: PathBuf,
    /// Base tree: path to content.
    pub base_files: BTreeMap<String, String>,
    /// Head tree after applying the patch.
    pub head_files: BTreeMap<String, String>,
    /// The repository holding the base and head commits.
    pub repo: FixtureRepo,
    /// Base commit.
    pub base: CommitSha,
    /// Head commit (child of base).
    pub head: CommitSha,
}

/// `fixtures/pull-requests` at the repository root.
pub fn pull_requests_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../fixtures/pull-requests")
}

impl Scenario {
    /// Load `fixtures/pull-requests/<name>`.
    pub fn load(name: &str) -> Result<Self, ScenarioError> {
        let dir = pull_requests_dir().join(name);
        let mut base_files = BTreeMap::new();
        collect(&dir.join("base"), Path::new(""), &mut base_files)?;
        let patch_path = dir.join("patch.diff");
        let patch = std::fs::read_to_string(&patch_path).map_err(|source| ScenarioError::Io {
            path: patch_path.clone(),
            source,
        })?;
        let head_files = apply_patch(&base_files, &patch).map_err(ScenarioError::Patch)?;
        Self::from_trees(name, dir, base_files, head_files)
    }

    /// Build a scenario from two in-memory trees (used for programmatic edge cases).
    pub fn from_trees(
        name: &str,
        dir: PathBuf,
        base_files: BTreeMap<String, String>,
        head_files: BTreeMap<String, String>,
    ) -> Result<Self, ScenarioError> {
        let mut repo = RepoBuilder::worktree()?;
        let base_specs: Vec<FileSpec<'_>> = base_files
            .iter()
            .map(|(p, c)| FileSpec::file(p, c))
            .collect();
        let base = repo.commit("base", &[], &base_specs)?;
        let head_specs: Vec<FileSpec<'_>> = head_files
            .iter()
            .map(|(p, c)| FileSpec::file(p, c))
            .collect();
        let head = repo.commit("head", std::slice::from_ref(&base), &head_specs)?;
        Ok(Self {
            name: name.to_owned(),
            dir,
            base_files,
            head_files,
            repo,
            base,
            head,
        })
    }

    /// Read a file under the scenario's `expected/` directory.
    pub fn expected(&self, rel: &str) -> Result<String, ScenarioError> {
        let path = self.dir.join("expected").join(rel);
        std::fs::read_to_string(&path).map_err(|source| ScenarioError::Io { path, source })
    }
}

fn collect(
    root: &Path,
    rel: &Path,
    out: &mut BTreeMap<String, String>,
) -> Result<(), ScenarioError> {
    let dir = root.join(rel);
    let entries = std::fs::read_dir(&dir).map_err(|source| ScenarioError::Io {
        path: dir.clone(),
        source,
    })?;
    for entry in entries {
        let entry = entry.map_err(|source| ScenarioError::Io {
            path: dir.clone(),
            source,
        })?;
        let rel_path = rel.join(entry.file_name());
        let file_type = entry.file_type().map_err(|source| ScenarioError::Io {
            path: entry.path(),
            source,
        })?;
        if file_type.is_dir() {
            collect(root, &rel_path, out)?;
        } else {
            let content =
                std::fs::read_to_string(entry.path()).map_err(|source| ScenarioError::Io {
                    path: entry.path(),
                    source,
                })?;
            out.insert(rel_path.to_string_lossy().replace('\\', "/"), content);
        }
    }
    Ok(())
}

/// One file section of a unified diff.
#[derive(Debug, Default)]
struct FilePatch {
    old: Option<String>,
    new: Option<String>,
    hunks: Vec<PatchHunk>,
}

#[derive(Debug, Default)]
struct PatchHunk {
    old_start: usize,
    lines: Vec<(char, String, bool)>,
}

/// Apply a git-style unified diff (text only) to `base`, returning the new tree. Supports
/// modifications, additions (`--- /dev/null`), deletions (`+++ /dev/null`), renames
/// (`rename from`/`rename to`, with or without hunks) and `\ No newline at end of file`.
pub fn apply_patch(
    base: &BTreeMap<String, String>,
    patch: &str,
) -> Result<BTreeMap<String, String>, String> {
    let mut out = base.clone();
    for fp in parse_files(patch)? {
        let source = fp
            .old
            .as_ref()
            .map(|p| {
                base.get(p)
                    .cloned()
                    .ok_or_else(|| format!("patched file {p} missing from base"))
            })
            .transpose()?
            .unwrap_or_default();
        if let Some(old) = &fp.old {
            out.remove(old);
        }
        if let Some(new) = &fp.new {
            let content = apply_hunks(&source, &fp.hunks).map_err(|e| format!("{new}: {e}"))?;
            out.insert(new.clone(), content);
        }
    }
    Ok(out)
}

fn strip_prefix(path: &str) -> Option<String> {
    let path = path.trim_end_matches('\t').trim();
    if path == "/dev/null" {
        return None;
    }
    Some(
        path.strip_prefix("a/")
            .or_else(|| path.strip_prefix("b/"))
            .unwrap_or(path)
            .to_owned(),
    )
}

fn parse_files(patch: &str) -> Result<Vec<FilePatch>, String> {
    let mut files: Vec<FilePatch> = Vec::new();
    let mut current: Option<FilePatch> = None;
    let mut lines = patch.split_inclusive('\n').peekable();
    while let Some(raw) = lines.next() {
        let line = raw.strip_suffix('\n').unwrap_or(raw);
        if let Some(rest) = line.strip_prefix("diff --git ") {
            if let Some(done) = current.take() {
                files.push(done);
            }
            let mut fp = FilePatch::default();
            let mut parts = rest.split(' ');
            fp.old = parts.next().and_then(strip_prefix);
            fp.new = parts.next().and_then(strip_prefix);
            current = Some(fp);
            continue;
        }
        let Some(fp) = current.as_mut() else {
            continue;
        };
        if line.starts_with("new file mode") {
            fp.old = None;
        } else if line.starts_with("deleted file mode") {
            fp.new = None;
        } else if let Some(p) = line.strip_prefix("rename from ") {
            fp.old = Some(p.to_owned());
        } else if let Some(p) = line.strip_prefix("rename to ") {
            fp.new = Some(p.to_owned());
        } else if let Some(p) = line.strip_prefix("--- ") {
            fp.old = strip_prefix(p);
        } else if let Some(p) = line.strip_prefix("+++ ") {
            fp.new = strip_prefix(p);
        } else if line.starts_with("GIT binary patch") {
            return Err("binary patches are not supported by the fixture applier".to_owned());
        } else if line.starts_with("@@ ") {
            let old_start = parse_old_start(line)?;
            let mut hunk = PatchHunk {
                old_start,
                lines: Vec::new(),
            };
            while let Some(next) = lines.peek() {
                let body = next.strip_suffix('\n').unwrap_or(next);
                if body.starts_with("@@ ") || body.starts_with("diff --git ") {
                    break;
                }
                let body_owned = body.to_owned();
                lines.next();
                if body_owned.starts_with('\\') {
                    if let Some(last) = hunk.lines.last_mut() {
                        last.2 = true;
                    }
                    continue;
                }
                let mut chars = body_owned.chars();
                let tag = chars.next().unwrap_or(' ');
                if !matches!(tag, ' ' | '+' | '-') {
                    return Err(format!("unexpected hunk line {body_owned:?}"));
                }
                hunk.lines.push((tag, chars.as_str().to_owned(), false));
            }
            fp.hunks.push(hunk);
        }
    }
    if let Some(done) = current.take() {
        files.push(done);
    }
    Ok(files)
}

fn parse_old_start(header: &str) -> Result<usize, String> {
    let old = header
        .split(' ')
        .nth(1)
        .and_then(|s| s.strip_prefix('-'))
        .ok_or_else(|| format!("bad hunk header {header:?}"))?;
    let start = old.split(',').next().unwrap_or("0");
    start
        .parse::<usize>()
        .map_err(|_| format!("bad hunk header {header:?}"))
}

fn apply_hunks(source: &str, hunks: &[PatchHunk]) -> Result<String, String> {
    // (text without terminator, has terminator)
    let old: Vec<(&str, bool)> = source
        .split_inclusive('\n')
        .map(|l| match l.strip_suffix('\n') {
            Some(text) => (text, true),
            None => (l, false),
        })
        .collect();
    let mut out = String::with_capacity(source.len());
    let mut next = 0usize;
    for hunk in hunks {
        let has_old = hunk.lines.iter().any(|(t, _, _)| *t != '+');
        let start = if has_old {
            hunk.old_start.saturating_sub(1)
        } else {
            hunk.old_start
        };
        if start < next || start > old.len() {
            return Err(format!("hunk at line {} out of order", hunk.old_start));
        }
        for (text, eol) in &old[next..start] {
            push(&mut out, text, *eol);
        }
        next = start;
        for (tag, text, no_eol) in &hunk.lines {
            match tag {
                '+' => push(&mut out, text, !no_eol),
                _ => {
                    let Some((have, eol)) = old.get(next) else {
                        return Err(format!("hunk at line {} runs past the end", hunk.old_start));
                    };
                    if have != text || *eol == *no_eol {
                        return Err(format!(
                            "line {} does not match: {have:?} vs {text:?}",
                            next + 1
                        ));
                    }
                    if *tag == ' ' {
                        push(&mut out, text, !no_eol);
                    }
                    next += 1;
                }
            }
        }
    }
    for (text, eol) in &old[next.min(old.len())..] {
        push(&mut out, text, *eol);
    }
    Ok(out)
}

fn push(out: &mut String, text: &str, eol: bool) {
    out.push_str(text);
    if eol {
        out.push('\n');
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn applies_modification_addition_and_deletion() {
        let mut base = BTreeMap::new();
        base.insert("a.txt".to_owned(), "1\n2\n3\n".to_owned());
        base.insert("gone.txt".to_owned(), "x\n".to_owned());
        let patch = "diff --git a/a.txt b/a.txt\n--- a/a.txt\n+++ b/a.txt\n@@ -1,3 +1,3 @@\n 1\n-2\n+two\n 3\n\
diff --git a/gone.txt b/gone.txt\ndeleted file mode 100644\n--- a/gone.txt\n+++ /dev/null\n@@ -1 +0,0 @@\n-x\n\
diff --git a/new.txt b/new.txt\nnew file mode 100644\n--- /dev/null\n+++ b/new.txt\n@@ -0,0 +1 @@\n+n\n\\ No newline at end of file\n";
        let head = apply_patch(&base, patch).unwrap_or_default();
        assert_eq!(head.get("a.txt").map(String::as_str), Some("1\ntwo\n3\n"));
        assert!(!head.contains_key("gone.txt"));
        assert_eq!(head.get("new.txt").map(String::as_str), Some("n"));
    }
}
