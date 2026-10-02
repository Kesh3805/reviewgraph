//! Provenance of derived data and the ADR-015 invalidation rules.

use std::collections::{BTreeMap, BTreeSet};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::ids::CommitSha;
use crate::language::Language;
use crate::version::{
    AnalyzerVersion, ConfigHash, EmbeddingSpace, GraphSchemaVersion, ProfileVersion, PromptVersion,
    ReviewerVersion, VerificationVersion,
};

/// Recorded with every derived artifact. Serialization is deterministic (`BTreeMap`, no floats),
/// so it can be hashed into cache keys and the repository fingerprint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Provenance {
    pub commit_sha: CommitSha,
    pub graph_schema_version: GraphSchemaVersion,
    pub analyzer_versions: BTreeMap<Language, AnalyzerVersion>,
    pub config_hash: ConfigHash,
    pub profile_version: ProfileVersion,
    pub model: Option<ModelProvenance>,
}

/// Extra provenance for model-derived artifacts.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ModelProvenance {
    pub embedding_space: Option<EmbeddingSpace>,
    pub reviewer_version: Option<ReviewerVersion>,
    pub prompt_version: Option<PromptVersion>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub verification_version: Option<VerificationVersion>,
}

/// What must be recomputed when moving from a previous [`Provenance`] to the current one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Invalidation {
    /// Rebuild the whole graph.
    pub full_rebuild: bool,
    /// Re-parse only the files of these languages.
    pub reparse_languages: BTreeSet<Language>,
    /// Recompute the repository profile.
    pub profile: bool,
    /// Recompute model-derived layers (embeddings, reviews, verification).
    pub model_layers: bool,
}

impl Invalidation {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

impl Provenance {
    /// Applies the ADR-015 table. A pure function; a changed `commit_sha` alone invalidates
    /// nothing because incremental analysis handles it.
    pub fn invalidation_against(&self, previous: &Provenance) -> Invalidation {
        let mut out = Invalidation {
            full_rebuild: self.graph_schema_version != previous.graph_schema_version
                || self.config_hash != previous.config_hash,
            profile: self.profile_version != previous.profile_version,
            model_layers: self.model != previous.model,
            ..Invalidation::default()
        };
        let languages: BTreeSet<Language> = self
            .analyzer_versions
            .keys()
            .chain(previous.analyzer_versions.keys())
            .copied()
            .collect();
        for lang in languages {
            let reparse = match (
                self.analyzer_versions.get(&lang),
                previous.analyzer_versions.get(&lang),
            ) {
                (Some(now), Some(before)) => now.major() != before.major(),
                _ => true,
            };
            if reparse {
                out.reparse_languages.insert(lang);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> Provenance {
        Provenance {
            commit_sha: "a".repeat(40).parse().unwrap(),
            graph_schema_version: GraphSchemaVersion(1),
            analyzer_versions: BTreeMap::from([
                (Language::Typescript, AnalyzerVersion::new(1, 2, 3)),
                (Language::Python, AnalyzerVersion::new(0, 4, 0)),
            ]),
            config_hash: ConfigHash::of(b"config"),
            profile_version: ProfileVersion(1),
            model: Some(ModelProvenance {
                embedding_space: Some(
                    EmbeddingSpace::new("voyage", "voyage-code-3", 1024).unwrap(),
                ),
                reviewer_version: Some(ReviewerVersion::new(1, 0, 0)),
                prompt_version: Some("correctness/v3".parse().unwrap()),
                provider: Some("anthropic".into()),
                model: Some("sonnet".into()),
                verification_version: Some(VerificationVersion(1)),
            }),
        }
    }

    #[test]
    fn provenance_serialization_is_deterministic() {
        let a = base();
        let mut b = base();
        // Rebuild the map in the opposite insertion order.
        b.analyzer_versions = BTreeMap::new();
        b.analyzer_versions
            .insert(Language::Python, AnalyzerVersion::new(0, 4, 0));
        b.analyzer_versions
            .insert(Language::Typescript, AnalyzerVersion::new(1, 2, 3));
        let ja = serde_json::to_string(&a).unwrap();
        assert_eq!(ja, serde_json::to_string(&b).unwrap());
        insta::assert_snapshot!(ja);
        assert_eq!(serde_json::from_str::<Provenance>(&ja).unwrap(), a);
    }

    #[test]
    fn schema_bump_forces_full_rebuild() {
        let prev = base();
        let mut now = base();
        now.graph_schema_version = GraphSchemaVersion(2);
        let inv = now.invalidation_against(&prev);
        assert!(inv.full_rebuild);
        assert!(!inv.profile && !inv.model_layers && inv.reparse_languages.is_empty());
    }

    #[test]
    fn config_hash_change_forces_full_rebuild() {
        let prev = base();
        let mut now = base();
        now.config_hash = ConfigHash::of(b"other");
        assert!(now.invalidation_against(&prev).full_rebuild);
    }

    #[test]
    fn analyzer_major_bump_reparses_only_that_language() {
        let prev = base();
        let mut now = base();
        now.analyzer_versions
            .insert(Language::Typescript, AnalyzerVersion::new(2, 0, 0));
        let inv = now.invalidation_against(&prev);
        assert_eq!(
            inv.reparse_languages,
            BTreeSet::from([Language::Typescript])
        );
        assert!(!inv.full_rebuild && !inv.profile && !inv.model_layers);
    }

    #[test]
    fn analyzer_minor_bump_no_reparse() {
        let prev = base();
        let mut now = base();
        now.analyzer_versions
            .insert(Language::Typescript, AnalyzerVersion::new(1, 9, 0));
        now.analyzer_versions
            .insert(Language::Python, AnalyzerVersion::new(0, 4, 7));
        assert!(now.invalidation_against(&prev).is_empty());
    }

    #[test]
    fn language_added_or_removed_reparses_it() {
        let prev = base();
        let mut added = base();
        added
            .analyzer_versions
            .insert(Language::Go, AnalyzerVersion::new(0, 1, 0));
        assert_eq!(
            added.invalidation_against(&prev).reparse_languages,
            BTreeSet::from([Language::Go])
        );
        let mut removed = base();
        removed.analyzer_versions.remove(&Language::Python);
        assert_eq!(
            removed.invalidation_against(&prev).reparse_languages,
            BTreeSet::from([Language::Python])
        );
    }

    #[test]
    fn profile_change_sets_only_profile() {
        let prev = base();
        let mut now = base();
        now.profile_version = ProfileVersion(2);
        let inv = now.invalidation_against(&prev);
        assert!(inv.profile && !inv.full_rebuild && !inv.model_layers);
    }

    #[test]
    fn model_change_invalidates_only_model_layers() {
        let prev = base();
        let mut now = base();
        if let Some(m) = now.model.as_mut() {
            m.prompt_version = Some("correctness/v4".parse().unwrap());
        }
        let inv = now.invalidation_against(&prev);
        assert!(inv.model_layers);
        assert!(!inv.full_rebuild && !inv.profile && inv.reparse_languages.is_empty());

        let mut none = base();
        none.model = None;
        assert!(none.invalidation_against(&prev).model_layers);
    }

    #[test]
    fn commit_change_alone_invalidates_nothing() {
        let prev = base();
        let mut now = base();
        now.commit_sha = "b".repeat(40).parse().unwrap();
        assert!(now.invalidation_against(&prev).is_empty());
    }
}
