//! `review init` orchestration (INIT-011): runs detectors INIT-001..010 in order, assembles
//! `RepositoryFacts`, and writes `.review/repository.json` atomically.

use std::collections::BTreeSet;
use std::path::PathBuf;

use review_core::language::Language;
use review_core::location::RepoPath;

use crate::build_systems::detect_build_systems;
use crate::docs_meta::detect_docs;
use crate::entrypoints::detect_entrypoints;
use crate::env_files::detect_env_files;
use crate::error::{InitError, InitWarning};
use crate::facts::{
    DeferredToIndex, GeneratedSummary, InventorySummary, RepositoryFacts, REPOSITORY_FACTS_SCHEMA,
};
use crate::frameworks::{auth_facts, detect_frameworks};
use crate::generated::{classify_generated, GitAttributesView};
use crate::git::{discover, tracked_among, GitOpenOptions};
use crate::infra::detect_infra;
use crate::language::{language_stats, primary_language};
use crate::layout::detect_layout;
use crate::manifests::detect_manifests;
use crate::migrations::detect_migrations;
use crate::read::BoundedReader;
use crate::review_dir::{self, ReviewLock};
use crate::tooling::detect_tooling;
use crate::tsconfig::detect_tsconfigs;
use crate::walk::{walk, FileClass, WalkOptions};
use crate::workspaces::detect_workspaces;

#[derive(Debug, Clone)]
pub struct InitOptions {
    pub root: PathBuf,
    /// Recompute even when `repository.json` is up to date. Never overwrites `config.yaml`.
    pub force: bool,
    pub allow_non_git: bool,
    pub provider_default_branch: Option<String>,
    pub walk: WalkOptions,
    pub analyzable_languages: Vec<Language>,
    /// False in worker mode: facts are computed but nothing is written.
    pub write_review_dir: bool,
}

impl InitOptions {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            force: false,
            allow_non_git: false,
            provider_default_branch: None,
            walk: WalkOptions::default(),
            analyzable_languages: vec![Language::Typescript, Language::Javascript],
            write_review_dir: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum InitOutcome {
    Written {
        facts: Box<RepositoryFacts>,
        path: PathBuf,
    },
    UpToDate {
        facts: Box<RepositoryFacts>,
    },
    Computed {
        facts: Box<RepositoryFacts>,
    },
}

impl InitOutcome {
    pub fn facts(&self) -> &RepositoryFacts {
        match self {
            Self::Written { facts, .. } | Self::UpToDate { facts } | Self::Computed { facts } => {
                facts
            }
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Written { .. } => "written",
            Self::UpToDate { .. } => "up_to_date",
            Self::Computed { .. } => "computed",
        }
    }
}

/// Runs the full initialization.
pub fn run(opts: &InitOptions) -> Result<InitOutcome, InitError> {
    let span = tracing::info_span!(
        "repository_init",
        init.outcome = tracing::field::Empty,
        init.warnings = tracing::field::Empty,
        init.facts_hash = tracing::field::Empty,
        repository.primary_language = tracing::field::Empty
    );
    let _guard = span.enter();
    let started = std::time::Instant::now();

    let mut warnings: Vec<InitWarning> = Vec::new();
    let discovered = discover(
        &opts.root,
        &GitOpenOptions {
            allow_non_git: opts.allow_non_git,
            provider_default_branch: opts.provider_default_branch.clone(),
        },
    )?;
    warnings.extend(discovered.warnings.clone());
    let root = discovered.root.clone();

    let _lock = if opts.write_review_dir {
        let dir = review_dir::ensure_layout(&root)?;
        Some(ReviewLock::acquire(&dir)?)
    } else {
        None
    };

    // Up to date: same schema and tool version, same clean HEAD.
    if !opts.force {
        if let (Some(existing_json), Some(git)) =
            (review_dir::read_facts_json(&root), discovered.git.as_ref())
        {
            match serde_json::from_str::<RepositoryFacts>(&existing_json) {
                Ok(existing) => {
                    let same_head = existing
                        .git
                        .as_ref()
                        .and_then(|g| g.head.sha())
                        .zip(git.head.sha())
                        .is_some_and(|(a, b)| a == b);
                    let existing_clean = existing.git.as_ref().is_some_and(|g| !g.dirty.is_dirty);
                    if existing.schema_version == REPOSITORY_FACTS_SCHEMA
                        && existing.tool_version == env!("CARGO_PKG_VERSION")
                        && same_head
                        && existing_clean
                        && !git.dirty.is_dirty
                    {
                        span.record("init.outcome", "up_to_date");
                        return Ok(InitOutcome::UpToDate {
                            facts: Box::new(existing),
                        });
                    }
                }
                Err(_) => warnings.push(InitWarning::new(
                    "repository_json_unreadable",
                    None,
                    "existing repository.json could not be parsed and will be rewritten",
                )),
            }
        }
    }

    let (config, config_warnings) = review_dir::read_config(&root);
    warnings.extend(config_warnings);

    let mut walk_opts = opts.walk.clone();
    walk_opts.extra_ignore_globs.extend(config.ignore.clone());
    let (inventory, walk_warnings) = walk(&root, &walk_opts)?;
    warnings.extend(walk_warnings);
    let reader = BoundedReader::new(&inventory.root);

    let (languages, w) = language_stats(&inventory, &reader, &opts.analyzable_languages);
    warnings.extend(w);
    let (manifest_facts, w) = detect_manifests(&inventory, &reader);
    warnings.extend(w);
    let build_systems = detect_build_systems(&inventory);
    let (workspaces, w) = detect_workspaces(&inventory, &reader, &manifest_facts);
    warnings.extend(w);
    let (frameworks, w) = detect_frameworks(&inventory, &manifest_facts);
    warnings.extend(w);
    let auth = auth_facts(&frameworks);
    let (tsconfigs, w) = detect_tsconfigs(&inventory, &reader);
    warnings.extend(w);
    let (layout, w) = detect_layout(&inventory, &reader, &tsconfigs, &workspaces);
    warnings.extend(w);
    let (tooling, w) = detect_tooling(&inventory, &reader, &tsconfigs, &manifest_facts);
    warnings.extend(w);
    let attrs = GitAttributesView::from_inventory(&inventory, &reader);
    let (generated_facts, w) = classify_generated(
        &inventory,
        &tsconfigs,
        &attrs,
        &config.generated_config(),
        &reader,
    );
    warnings.extend(w);
    let (entrypoints, w) = detect_entrypoints(&inventory, &reader, &manifest_facts, &tsconfigs);
    warnings.extend(w);
    let migration_facts = detect_migrations(&inventory);
    let (infra, w) = detect_infra(&inventory, &reader);
    warnings.extend(w);
    let env_candidates: Vec<RepoPath> = inventory
        .entries
        .iter()
        .filter(|e| {
            let name = e
                .path
                .as_str()
                .rsplit('/')
                .next()
                .unwrap_or("")
                .to_ascii_lowercase();
            name == ".env"
                || name.starts_with(".env.")
                || name.ends_with(".env")
                || name == ".envrc"
        })
        .map(|e| e.path.clone())
        .collect();
    let tracked: BTreeSet<RepoPath> = if discovered.git.is_some() {
        tracked_among(&root, &env_candidates)
    } else {
        BTreeSet::new()
    };
    let (env_files, w) = detect_env_files(&inventory, &reader, &tracked);
    warnings.extend(w);
    let (docs, w) = detect_docs(&inventory, &reader);
    warnings.extend(w);

    let sensitive_files: Vec<RepoPath> = inventory
        .entries
        .iter()
        .filter(|e| e.class == FileClass::Sensitive)
        .map(|e| e.path.clone())
        .collect();

    // Generated counts per language.
    let mut languages = languages;
    for stat in &mut languages {
        stat.generated_files = generated_facts
            .files
            .keys()
            .filter(|path| {
                inventory.find(path.as_str()).is_some()
                    && crate::language::detect_language(path, &[])
                        .map(|t| t.language)
                        .unwrap_or(Language::Other)
                        == stat.language
            })
            .count() as u64;
    }

    let mut facts = RepositoryFacts {
        schema_version: REPOSITORY_FACTS_SCHEMA,
        tool_version: env!("CARGO_PKG_VERSION").to_owned(),
        detected_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        root_name: root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        git: discovered.git.clone(),
        inventory: InventorySummary::of(&inventory),
        primary_language: primary_language(&languages),
        languages,
        package_managers: manifest_facts.package_managers.clone(),
        manifests: manifest_facts.manifests.clone(),
        build_systems,
        workspaces,
        frameworks,
        auth,
        layout,
        tsconfigs: tsconfigs.configs.clone(),
        tooling,
        generated: GeneratedSummary::of(&generated_facts),
        entrypoints,
        migrations: migration_facts.dirs,
        schema_files: migration_facts.schema_files,
        infra,
        env_files,
        sensitive_files,
        docs,
        api_routes: DeferredToIndex::default(),
        fingerprint: None,
        facts_hash: String::new(),
        warnings,
        warnings_truncated: 0,
    };
    facts.apply_caps();
    facts.facts_hash = facts.compute_hash()?;
    span.record("init.warnings", facts.warnings.len());
    span.record("init.facts_hash", facts.facts_hash.as_str());
    if let Some(primary) = facts.primary_language {
        span.record("repository.primary_language", primary.as_str());
    }

    let outcome = if opts.write_review_dir {
        let dir = review_dir::review_dir(&root);
        let json = facts.to_pretty_json()?;
        let path = review_dir::write_facts_atomic(&dir, &json)?;
        let generated_json = serde_json::to_string_pretty(&generated_facts.files)
            .map_err(|e| InitError::Serde(e.to_string()))?;
        review_dir::write_cache_file(&dir, "generated.json", &generated_json)?;
        InitOutcome::Written {
            facts: Box::new(facts),
            path,
        }
    } else {
        InitOutcome::Computed {
            facts: Box::new(facts),
        }
    };
    span.record("init.outcome", outcome.label());
    tracing::debug!(
        target: "repository.init",
        elapsed_ms = started.elapsed().as_millis() as u64,
        outcome = outcome.label(),
        "init finished"
    );
    Ok(outcome)
}
