//! Language detection and per-language statistics (INIT-003).

use std::collections::BTreeMap;

use rayon::prelude::*;
use review_core::language::Language;
use review_core::location::RepoPath;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::InitWarning;
use crate::read::BoundedReader;
use crate::walk::{FileClass, FileEntry, FileInventory};

/// Bytes read from an extensionless file to look for a shebang.
const SHEBANG_BYTES: usize = 256;

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Dialect {
    Ts,
    Tsx,
    Dts,
    Js,
    Jsx,
    Mjs,
    Cjs,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
pub struct LanguageTag {
    pub language: Language,
    pub dialect: Option<Dialect>,
}

impl LanguageTag {
    const fn plain(language: Language) -> Self {
        Self {
            language,
            dialect: None,
        }
    }

    const fn with(language: Language, dialect: Dialect) -> Self {
        Self {
            language,
            dialect: Some(dialect),
        }
    }
}

/// Extension table, longest suffix first so `.d.ts` beats `.ts`.
const EXTENSIONS: &[(&str, LanguageTag)] = &[
    (
        ".d.mts",
        LanguageTag::with(Language::Typescript, Dialect::Dts),
    ),
    (
        ".d.cts",
        LanguageTag::with(Language::Typescript, Dialect::Dts),
    ),
    (
        ".d.ts",
        LanguageTag::with(Language::Typescript, Dialect::Dts),
    ),
    (
        ".tsx",
        LanguageTag::with(Language::Typescript, Dialect::Tsx),
    ),
    (".mts", LanguageTag::with(Language::Typescript, Dialect::Ts)),
    (".cts", LanguageTag::with(Language::Typescript, Dialect::Ts)),
    (".ts", LanguageTag::with(Language::Typescript, Dialect::Ts)),
    (
        ".jsx",
        LanguageTag::with(Language::Javascript, Dialect::Jsx),
    ),
    (
        ".mjs",
        LanguageTag::with(Language::Javascript, Dialect::Mjs),
    ),
    (
        ".cjs",
        LanguageTag::with(Language::Javascript, Dialect::Cjs),
    ),
    (".js", LanguageTag::with(Language::Javascript, Dialect::Js)),
    (".pyi", LanguageTag::plain(Language::Python)),
    (".py", LanguageTag::plain(Language::Python)),
    (".go", LanguageTag::plain(Language::Go)),
    (".rs", LanguageTag::plain(Language::Rust)),
    (".java", LanguageTag::plain(Language::Java)),
    (".kts", LanguageTag::plain(Language::Kotlin)),
    (".kt", LanguageTag::plain(Language::Kotlin)),
    (".cs", LanguageTag::plain(Language::Csharp)),
    (".rb", LanguageTag::plain(Language::Ruby)),
    (".php", LanguageTag::plain(Language::Php)),
    (".bash", LanguageTag::plain(Language::Shell)),
    (".zsh", LanguageTag::plain(Language::Shell)),
    (".sh", LanguageTag::plain(Language::Shell)),
    (".sql", LanguageTag::plain(Language::Sql)),
    (".yaml", LanguageTag::plain(Language::Yaml)),
    (".yml", LanguageTag::plain(Language::Yaml)),
    (".jsonc", LanguageTag::plain(Language::Json)),
    (".json5", LanguageTag::plain(Language::Json)),
    (".json", LanguageTag::plain(Language::Json)),
    (".toml", LanguageTag::plain(Language::Toml)),
    (".mdx", LanguageTag::plain(Language::Markdown)),
    (".md", LanguageTag::plain(Language::Markdown)),
    (".html", LanguageTag::plain(Language::Html)),
    (".htm", LanguageTag::plain(Language::Html)),
    (".scss", LanguageTag::plain(Language::Css)),
    (".sass", LanguageTag::plain(Language::Css)),
    (".less", LanguageTag::plain(Language::Css)),
    (".css", LanguageTag::plain(Language::Css)),
    (".tfvars", LanguageTag::plain(Language::Terraform)),
    (".tf", LanguageTag::plain(Language::Terraform)),
    (".hcl", LanguageTag::plain(Language::Terraform)),
    (".proto", LanguageTag::plain(Language::Protobuf)),
    (".graphql", LanguageTag::plain(Language::Graphql)),
    (".gql", LanguageTag::plain(Language::Graphql)),
    (".dockerfile", LanguageTag::plain(Language::Dockerfile)),
    (".prisma", LanguageTag::plain(Language::Prisma)),
];

fn basename(path: &RepoPath) -> &str {
    path.as_str().rsplit('/').next().unwrap_or(path.as_str())
}

fn special_filename(name: &str) -> Option<LanguageTag> {
    if name == "Dockerfile" || name.starts_with("Dockerfile.") {
        return Some(LanguageTag::plain(Language::Dockerfile));
    }
    // `Makefile` and `Jenkinsfile` have no dedicated language; they are reported as Other.
    if matches!(name, "Makefile" | "GNUmakefile" | "Jenkinsfile") {
        return Some(LanguageTag::plain(Language::Other));
    }
    None
}

/// True when the file has no extension and no special name, so a shebang can decide.
pub fn needs_shebang_probe(path: &RepoPath) -> bool {
    let name = basename(path);
    special_filename(name).is_none() && path.extension().is_none()
}

fn shebang_language(prefix: &[u8]) -> Option<LanguageTag> {
    let end = prefix
        .iter()
        .position(|&b| b == b'\n')
        .unwrap_or(prefix.len())
        .min(SHEBANG_BYTES);
    let line = String::from_utf8_lossy(&prefix[..end]);
    let rest = line.strip_prefix("#!")?.trim();
    // Tokens of the interpreter command: `/usr/bin/env -S node --flag` or `/bin/bash`.
    let mut tokens = rest.split_whitespace();
    let mut program = tokens.next()?;
    if program.rsplit('/').next() == Some("env") {
        program = tokens.find(|t| !t.starts_with('-'))?;
    }
    let program = program.rsplit('/').next().unwrap_or(program);
    let tag = match program {
        "node" | "nodejs" | "bun" => LanguageTag::with(Language::Javascript, Dialect::Js),
        "ts-node" | "tsx" | "deno" => LanguageTag::with(Language::Typescript, Dialect::Ts),
        "bash" | "sh" | "zsh" | "dash" => LanguageTag::plain(Language::Shell),
        "ruby" => LanguageTag::plain(Language::Ruby),
        p if p.starts_with("python") => LanguageTag::plain(Language::Python),
        _ => return None,
    };
    Some(tag)
}

/// Language of a path. `prefix` is the start of the file content (only consulted for files
/// without an extension). Unknown extensions are `Other`; extensionless files that are not
/// recognizable by name or shebang are `None`.
pub fn detect_language(path: &RepoPath, prefix: &[u8]) -> Option<LanguageTag> {
    let name = basename(path);
    if let Some(tag) = special_filename(name) {
        return Some(tag);
    }
    let lower = name.to_ascii_lowercase();
    for (suffix, tag) in EXTENSIONS {
        if lower.len() > suffix.len() && lower.ends_with(suffix) {
            return Some(*tag);
        }
    }
    if path.extension().is_some() {
        return Some(LanguageTag::plain(Language::Other));
    }
    shebang_language(prefix)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LanguageStat {
    pub language: Language,
    pub files: u64,
    pub bytes: u64,
    pub lines: u64,
    /// A registered analyzer exists for this language.
    pub analyzable: bool,
    /// Filled by INIT-008 once generated code is classified.
    pub generated_files: u64,
}

struct PerFile {
    language: Language,
    bytes: u64,
    lines: u64,
    warning: Option<InitWarning>,
}

/// Language of every inventory entry that carries analyzable text (`Source`, `TooLarge`), for
/// callers that need it per file (the indexer routes files to analyzers with this).
pub fn language_of_entry(reader: &BoundedReader, entry: &FileEntry) -> Option<LanguageTag> {
    let prefix = if needs_shebang_probe(&entry.path) && entry.class == FileClass::Source {
        reader.read_prefix(entry, SHEBANG_BYTES).unwrap_or_default()
    } else {
        Vec::new()
    };
    detect_language(&entry.path, &prefix)
}

/// Per-language statistics, sorted by bytes (descending) then language name.
pub fn language_stats(
    inventory: &FileInventory,
    reader: &BoundedReader,
    analyzers: &[Language],
) -> (Vec<LanguageStat>, Vec<InitWarning>) {
    let span = tracing::info_span!(
        "init.languages",
        languages.count = tracing::field::Empty,
        languages.primary = tracing::field::Empty
    );
    let _guard = span.enter();

    let per_file: Vec<PerFile> = inventory
        .entries
        .par_iter()
        .filter(|e| matches!(e.class, FileClass::Source | FileClass::TooLarge))
        .map(|entry| {
            let language = language_of_entry(reader, entry)
                .map(|t| t.language)
                .unwrap_or(Language::Other);
            let mut lines = 0;
            let mut warning = None;
            if entry.class == FileClass::Source {
                match reader.read_prefix(entry, usize::try_from(entry.size).unwrap_or(usize::MAX)) {
                    Ok(bytes) => lines = bytes.iter().filter(|&&b| b == b'\n').count() as u64,
                    Err(_) => {
                        warning = Some(InitWarning::new(
                            "unreadable_entry",
                            Some(entry.path.clone()),
                            "file could not be read for line counting",
                        ));
                    }
                }
            }
            PerFile {
                language,
                bytes: entry.size,
                lines,
                warning,
            }
        })
        .collect();

    let mut by_language: BTreeMap<Language, LanguageStat> = BTreeMap::new();
    let mut warnings = Vec::new();
    for file in per_file {
        let stat = by_language
            .entry(file.language)
            .or_insert_with(|| LanguageStat {
                language: file.language,
                files: 0,
                bytes: 0,
                lines: 0,
                analyzable: analyzers.contains(&file.language),
                generated_files: 0,
            });
        stat.files += 1;
        stat.bytes += file.bytes;
        stat.lines += file.lines;
        warnings.extend(file.warning);
    }
    warnings.sort();
    let mut stats: Vec<LanguageStat> = by_language.into_values().collect();
    stats.sort_by(|a, b| {
        b.bytes
            .cmp(&a.bytes)
            .then_with(|| a.language.as_str().cmp(b.language.as_str()))
    });
    span.record("languages.count", stats.len());
    if let Some(primary) = primary_language(&stats) {
        span.record("languages.primary", primary.as_str());
    }
    (stats, warnings)
}

/// The programming language with the most bytes; ties go to the earlier enum variant.
pub fn primary_language(stats: &[LanguageStat]) -> Option<Language> {
    let mut best: Option<(&LanguageStat, Language)> = None;
    for stat in stats.iter().filter(|s| s.language.is_programming()) {
        let better = match best {
            None => true,
            Some((current, _)) => {
                stat.bytes > current.bytes
                    || (stat.bytes == current.bytes && stat.language < current.language)
            }
        };
        if better {
            best = Some((stat, stat.language));
        }
    }
    best.map(|(_, language)| language)
}
