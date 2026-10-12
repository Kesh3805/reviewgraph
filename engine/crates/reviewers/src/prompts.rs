//! Versioned prompt registry (REV-001).
//!
//! Prompts live at `prompts/{kind}/v{n}.md` with YAML front matter
//! `{id, version, schema, tier_default, focus_profiles}`. They are embedded with `include_str!`,
//! `prompt_sha = blake3(file bytes)`, and `prompt_version = "{kind}:v{n}:{sha[..8]}"`.
//! `prompts/registry.lock` lists `kind/vN sha`; a test fails when a file's sha differs from the
//! lock, so a versioned prompt is immutable: any edit needs a new version.
//!
//! A prompt body may contain one `{{FOCUS_SECTIONS}}` line, and the file may end with focus
//! sections introduced by `<!-- focus:<profile> -->` lines. Rendering replaces the placeholder
//! with the active sections in sorted profile-name order, so there are at most 2^n static system
//! texts per prompt, each cacheable.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::error::ReviewerError;
use crate::focus::FocusProfile;

/// Placeholder line replaced by the active focus sections.
pub const FOCUS_PLACEHOLDER: &str = "{{FOCUS_SECTIONS}}";
const FOCUS_MARKER: &str = "<!-- focus:";

/// One embedded prompt file.
#[derive(Debug, Clone, Copy)]
pub struct PromptFile {
    pub kind: &'static str,
    pub version: u32,
    pub text: &'static str,
}

/// Every versioned prompt shipped with this crate.
pub const PROMPTS: &[PromptFile] = &[
    PromptFile {
        kind: "correctness",
        version: 1,
        text: include_str!("../prompts/correctness/v1.md"),
    },
    PromptFile {
        kind: "security",
        version: 1,
        text: include_str!("../prompts/security/v1.md"),
    },
];

/// The committed lock file.
pub const REGISTRY_LOCK: &str = include_str!("../prompts/registry.lock");

/// Identifies a prompt: `{ kind, version, sha }`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptRef {
    pub kind: String,
    pub version: u32,
    pub sha: String,
}

impl PromptRef {
    /// `"{kind}:v{n}:{sha[..8]}"`.
    pub fn prompt_version(&self) -> String {
        let short: String = self.sha.chars().take(8).collect();
        format!("{}:v{}:{short}", self.kind, self.version)
    }

    /// `"{kind}/v{n}"`, the lock key.
    pub fn lock_key(&self) -> String {
        format!("{}/v{}", self.kind, self.version)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrontMatter {
    pub id: String,
    pub version: u32,
    pub schema: String,
    pub tier_default: String,
    #[serde(default)]
    pub focus_profiles: Vec<String>,
}

/// A parsed prompt.
#[derive(Debug, Clone)]
pub struct Prompt {
    pub reference: PromptRef,
    pub front: FrontMatter,
    body: String,
    focus: BTreeMap<String, String>,
}

/// blake3 of the prompt bytes, lowercase hex.
pub fn sha_of(text: &str) -> String {
    blake3::hash(text.as_bytes()).to_hex().to_string()
}

fn split_front_matter(text: &str) -> Result<(&str, &str), String> {
    let rest = text
        .strip_prefix("---\n")
        .ok_or_else(|| "missing front matter".to_owned())?;
    let end = rest
        .find("\n---\n")
        .ok_or_else(|| "unterminated front matter".to_owned())?;
    Ok((&rest[..end], &rest[end + 5..]))
}

impl Prompt {
    pub fn parse(file: &PromptFile) -> Result<Self, ReviewerError> {
        let bad =
            |why: String| ReviewerError::Prompt(format!("{}/v{}: {why}", file.kind, file.version));
        let (front, after) = split_front_matter(file.text).map_err(bad)?;
        let front: FrontMatter =
            serde_yaml::from_str(front).map_err(|e| bad(format!("front matter: {e}")))?;
        if front.id != file.kind || front.version != file.version {
            return Err(bad("front matter id/version does not match the file".into()));
        }
        let mut body = String::new();
        let mut focus: BTreeMap<String, String> = BTreeMap::new();
        let mut current: Option<String> = None;
        for line in after.split_inclusive('\n') {
            if let Some(name) = line
                .trim_end()
                .strip_prefix(FOCUS_MARKER)
                .and_then(|r| r.strip_suffix("-->"))
            {
                let name = name.trim().to_owned();
                if FocusProfile::parse(&name).is_none() {
                    return Err(bad(format!("unknown focus profile `{name}`")));
                }
                focus.insert(name.clone(), String::new());
                current = Some(name);
                continue;
            }
            match &current {
                Some(name) => {
                    if let Some(section) = focus.get_mut(name) {
                        section.push_str(line);
                    }
                }
                None => body.push_str(line),
            }
        }
        Ok(Self {
            reference: PromptRef {
                kind: file.kind.to_owned(),
                version: file.version,
                sha: sha_of(file.text),
            },
            front,
            body,
            focus,
        })
    }

    /// The system text for a set of active focus profiles.
    pub fn render(&self, active: &[FocusProfile]) -> String {
        let mut names: Vec<&str> = active.iter().map(|p| p.as_str()).collect();
        names.sort_unstable();
        names.dedup();
        let sections: String = names
            .iter()
            .filter_map(|n| self.focus.get(*n))
            .map(|s| s.trim().to_owned() + "\n")
            .collect::<Vec<_>>()
            .join("\n");
        let mut out = String::new();
        for line in self.body.split_inclusive('\n') {
            if line.trim() == FOCUS_PLACEHOLDER {
                out.push_str(&sections);
            } else {
                out.push_str(line);
            }
        }
        let trimmed = out.trim().to_owned();
        trimmed + "\n"
    }

    /// Focus profile names that have a section in this prompt.
    pub fn focus_names(&self) -> Vec<&str> {
        self.focus.keys().map(String::as_str).collect()
    }
}

/// Looks up and parses an embedded prompt.
pub fn prompt(kind: &str, version: u32) -> Result<Prompt, ReviewerError> {
    let file = PROMPTS
        .iter()
        .find(|p| p.kind == kind && p.version == version)
        .ok_or_else(|| ReviewerError::Prompt(format!("no prompt {kind}/v{version}")))?;
    Prompt::parse(file)
}

/// Parses `registry.lock`: `kind/vN <sha>` per line; `#` starts a comment.
pub fn parse_lock(text: &str) -> BTreeMap<String, String> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| {
            let mut parts = l.split_whitespace();
            Some((parts.next()?.to_owned(), parts.next()?.to_owned()))
        })
        .collect()
}

/// Checks `files` against a lock: every file must be locked with its exact sha. Returns every
/// mismatch.
pub fn check_lock(files: &[PromptFile], lock: &str) -> Result<(), Vec<String>> {
    let entries = parse_lock(lock);
    let errors: Vec<String> = files
        .iter()
        .filter_map(|f| {
            let key = format!("{}/v{}", f.kind, f.version);
            let sha = sha_of(f.text);
            match entries.get(&key) {
                Some(locked) if *locked == sha => None,
                Some(locked) => Some(format!(
                    "{key}: sha {sha} differs from the lock ({locked}); versioned prompts are immutable, add v{} instead",
                    f.version + 1
                )),
                None => Some(format!("{key}: not in registry.lock (sha {sha})")),
            }
        })
        .collect();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

/// Startup self-test: every prompt parses and matches the lock.
pub fn self_test() -> Result<(), ReviewerError> {
    for f in PROMPTS {
        Prompt::parse(f)?;
    }
    check_lock(PROMPTS, REGISTRY_LOCK).map_err(|e| ReviewerError::Prompt(e.join("; ")))
}
