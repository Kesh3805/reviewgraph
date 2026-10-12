//! The precedence resolver (POL-004, PRD §65): explicit policy > architecture documentation >
//! established convention > generic guidance.
//!
//! * The highest-ranked applicable source wins; every other applicable source is kept in
//!   `overridden` for the explainability trace (PRD §85).
//! * An explicit exception — a path in `conventions.exceptions`, or a `type: rule` suppression
//!   naming the statement — negates lower-ranked sources for that scope.
//! * A convention applies only when enforceable (confidence ≥ 0.9 and samples ≥ 10).
//! * Two sources of the same rank that disagree resolve to the stricter decision and set
//!   `conflict` (visible in `review doctor`).

use std::cmp::Reverse;

use globset::{Glob, GlobSet, GlobSetBuilder};
use serde::{Deserialize, Serialize};

use crate::config::{LoadedConfig, ReviewConfigV1, SuppressionKind};
use crate::conventions::builtin::{
    controller_repository, error_hierarchy, guard_usage, queues, transaction_wrapper,
};
use crate::conventions::is_enforceable;
use crate::infer::roles::Role;
use crate::knowledge::DocumentationRule;
use crate::model::RepositoryProfile;

/// Where a statement comes from, ordered by rank (4 → 1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PolicySource {
    Explicit { rule_id: String },
    Documented { doc_id: String },
    Convention { convention_id: String, confidence: f32, samples: u32 },
    Generic,
}

impl PolicySource {
    pub const fn rank(&self) -> u8 {
        match self {
            Self::Explicit { .. } => 4,
            Self::Documented { .. } => 3,
            Self::Convention { .. } => 2,
            Self::Generic => 1,
        }
    }

    /// The id a `type: rule` suppression names.
    pub fn id(&self) -> &str {
        match self {
            Self::Explicit { rule_id } => rule_id,
            Self::Documented { doc_id } => doc_id,
            Self::Convention { convention_id, .. } => convention_id,
            Self::Generic => "generic",
        }
    }
}

/// The closed set of topics a policy can speak about.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "topic", rename_all = "snake_case")]
pub enum PolicyTopic {
    LayerDependency { from: String, to: String },
    TransactionOnWrite,
    GuardOnEndpoint,
    JobIdDeterminism,
    TestsForApiChange,
    ErrorHierarchy,
    MigrationsOnly,
}

impl PolicyTopic {
    /// A layer dependency with role names normalized (`controllers` → `controller`).
    pub fn layer_dependency(from: &str, to: &str) -> Self {
        let norm = |name: &str| {
            Role::from_layer_name(name)
                .map(|r| r.as_str().to_owned())
                .unwrap_or_else(|| name.to_owned())
        };
        Self::LayerDependency {
            from: norm(from),
            to: norm(to),
        }
    }

    /// Parses a documentation `topic:` value.
    pub fn parse(topic: &str, from: Option<&str>, to: Option<&str>) -> Option<Self> {
        Some(match topic.trim().to_ascii_lowercase().as_str() {
            "layer_dependency" => Self::layer_dependency(from?, to?),
            "transaction_on_write" => Self::TransactionOnWrite,
            "guard_on_endpoint" => Self::GuardOnEndpoint,
            "job_id_determinism" => Self::JobIdDeterminism,
            "tests_for_api_change" => Self::TestsForApiChange,
            "error_hierarchy" => Self::ErrorHierarchy,
            "migrations_only" => Self::MigrationsOnly,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    Required,
    Forbidden,
    Allowed,
    NoPolicy,
}

impl Decision {
    /// Higher is stricter.
    pub const fn strictness(self) -> u8 {
        match self {
            Self::Forbidden => 3,
            Self::Required => 2,
            Self::Allowed => 1,
            Self::NoPolicy => 0,
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "required" | "must" => Some(Self::Required),
            "forbidden" | "must_not" => Some(Self::Forbidden),
            "allowed" | "may" => Some(Self::Allowed),
            _ => None,
        }
    }
}

/// One thing some source says about a topic.
#[derive(Debug, Clone)]
pub struct PolicyStatement {
    pub source: PolicySource,
    pub topic: PolicyTopic,
    pub decision: Decision,
    /// `None` applies everywhere.
    scope: Option<(String, GlobSet)>,
}

fn compile(globs: &[String]) -> Option<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    for glob in globs {
        builder.add(Glob::new(glob).ok()?);
    }
    builder.build().ok()
}

impl PolicyStatement {
    pub fn new(source: PolicySource, topic: PolicyTopic, decision: Decision) -> Self {
        Self {
            source,
            topic,
            decision,
            scope: None,
        }
    }

    /// Restricts the statement to paths matching any of `globs` (`**` or empty means everywhere).
    pub fn scoped(mut self, globs: &[String]) -> Self {
        let globs: Vec<String> = globs.iter().filter(|g| g.as_str() != "**").cloned().collect();
        if !globs.is_empty() {
            if let Some(set) = compile(&globs) {
                self.scope = Some((globs.join(","), set));
            }
        }
        self
    }

    fn applies_to(&self, path: &str) -> bool {
        self.scope.as_ref().is_none_or(|(_, set)| set.is_match(path))
    }

    fn usable(&self) -> bool {
        match &self.source {
            PolicySource::Convention {
                confidence, samples, ..
            } => is_enforceable(*confidence, *samples),
            _ => true,
        }
    }
}

/// What a file or symbol is being asked about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileOrSymbol<'a> {
    File(&'a str),
    Symbol { path: &'a str, id: &'a str },
}

impl FileOrSymbol<'_> {
    pub fn path(&self) -> &str {
        match self {
            Self::File(path) | Self::Symbol { path, .. } => path,
        }
    }
}

/// The resolved policy for one scope and topic.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EffectivePolicy {
    pub topic: PolicyTopic,
    pub decision: Decision,
    pub winner: PolicySource,
    /// Every other applicable source, highest rank first.
    pub overridden: Vec<PolicySource>,
    /// Same-rank sources disagreed; the stricter decision won.
    pub conflict: bool,
}

/// Built once per run from the config, the base profile and the documentation rules.
#[derive(Debug, Clone, Default)]
pub struct PolicyResolver {
    statements: Vec<PolicyStatement>,
    exceptions: Option<GlobSet>,
    /// `(suppression id, named statement id)` of `type: rule` suppressions.
    rule_suppressions: Vec<(String, String)>,
}

impl PolicyResolver {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, statement: PolicyStatement) {
        self.statements.push(statement);
    }

    /// The explicit exceptions: config `conventions.exceptions` globs and `type: rule`
    /// suppressions (only from a valid config).
    pub fn with_exceptions(mut self, loaded: &LoadedConfig) -> Self {
        let globs = &loaded.config.conventions.exceptions;
        if !globs.is_empty() {
            self.exceptions = compile(globs);
        }
        self.rule_suppressions = loaded
            .applicable_suppressions()
            .iter()
            .filter(|s| s.kind == SuppressionKind::Rule)
            .map(|s| (s.id.clone(), s.value.clone()))
            .collect();
        self
    }

    /// Statements from the config's rules, the profile's conventions and documented rules that
    /// declare a `topic`.
    pub fn from_inputs(
        loaded: &LoadedConfig,
        profile: Option<&RepositoryProfile>,
        docs: &[DocumentationRule],
    ) -> Self {
        let mut resolver = Self::new().with_exceptions(loaded);
        explicit_statements(&loaded.config, &mut resolver);
        if let Some(profile) = profile {
            for c in &profile.conventions {
                let Some((topic, decision)) = convention_topic(&c.id) else {
                    continue;
                };
                resolver.add(
                    PolicyStatement::new(
                        PolicySource::Convention {
                            convention_id: c.id.clone(),
                            confidence: c.confidence,
                            samples: c.samples,
                        },
                        topic,
                        decision,
                    )
                    .scoped(std::slice::from_ref(&c.scope)),
                );
            }
        }
        for doc in docs {
            let Some(policy) = &doc.policy else {
                continue;
            };
            let Some(topic) =
                PolicyTopic::parse(&policy.topic, policy.from.as_deref(), policy.to.as_deref())
            else {
                continue;
            };
            let decision = Decision::parse(&policy.decision).unwrap_or(Decision::Required);
            let globs: Vec<String> = doc
                .applies_to
                .iter()
                .filter(|a| !a.contains('#'))
                .cloned()
                .collect();
            resolver.add(
                PolicyStatement::new(
                    PolicySource::Documented {
                        doc_id: doc.node_id.clone(),
                    },
                    topic,
                    decision,
                )
                .scoped(&globs),
            );
        }
        resolver
    }

    /// The effective policy for `scope` on `topic`.
    pub fn effective(&self, scope: &FileOrSymbol<'_>, topic: &PolicyTopic) -> EffectivePolicy {
        let path = scope.path();
        let mut applicable: Vec<&PolicyStatement> = self
            .statements
            .iter()
            .filter(|s| &s.topic == topic && s.applies_to(path) && s.usable())
            .collect();
        applicable.sort_by_key(|s| Reverse(s.source.rank()));
        let mut overridden: Vec<PolicySource> = Vec::new();

        // A `type: rule` suppression removes the statements it names.
        let mut negated_by: Option<String> = None;
        applicable.retain(|s| {
            match self
                .rule_suppressions
                .iter()
                .find(|(_, named)| named == s.source.id())
            {
                Some((sid, _)) => {
                    negated_by = Some(format!("suppression:{sid}"));
                    overridden.push(s.source.clone());
                    false
                }
                None => true,
            }
        });
        // `conventions.exceptions` negates every source ranked below explicit policy.
        let excepted = self.exceptions.as_ref().is_some_and(|set| set.is_match(path));
        if excepted {
            applicable.retain(|s| {
                if s.source.rank() < 4 {
                    overridden.push(s.source.clone());
                    false
                } else {
                    true
                }
            });
            if applicable.is_empty() {
                negated_by = Some("conventions.exceptions".to_owned());
            }
        }

        let Some(top) = applicable.first() else {
            return EffectivePolicy {
                topic: topic.clone(),
                decision: if negated_by.is_some() {
                    Decision::Allowed
                } else {
                    Decision::NoPolicy
                },
                winner: match negated_by {
                    Some(rule_id) => PolicySource::Explicit { rule_id },
                    None => PolicySource::Generic,
                },
                overridden,
                conflict: false,
            };
        };
        let rank = top.source.rank();
        let same_rank: Vec<&&PolicyStatement> =
            applicable.iter().filter(|s| s.source.rank() == rank).collect();
        let conflict = same_rank.iter().any(|s| s.decision != top.decision);
        let winner = same_rank
            .iter()
            .max_by(|a, b| {
                a.decision
                    .strictness()
                    .cmp(&b.decision.strictness())
                    .then(b.source.id().cmp(a.source.id()))
            })
            .map_or(*top, |s| **s);
        if conflict {
            crate::metrics::policy_conflict();
        }
        for s in &applicable {
            if !std::ptr::eq(*s, winner) {
                overridden.push(s.source.clone());
            }
        }
        overridden.sort_by_key(|s| Reverse(s.rank()));
        EffectivePolicy {
            topic: topic.clone(),
            decision: winner.decision,
            winner: winner.source.clone(),
            overridden,
            conflict,
        }
    }

    /// Every topic some statement speaks about, sorted.
    pub fn topics(&self) -> Vec<PolicyTopic> {
        let mut out: Vec<PolicyTopic> = self.statements.iter().map(|s| s.topic.clone()).collect();
        out.sort();
        out.dedup();
        out
    }
}

fn explicit_statements(config: &ReviewConfigV1, resolver: &mut PolicyResolver) {
    let explicit = |id: &str| PolicySource::Explicit {
        rule_id: id.to_owned(),
    };
    for rule in &config.rules.forbidden_dependencies {
        resolver.add(PolicyStatement::new(
            explicit(&rule.id),
            PolicyTopic::layer_dependency(&rule.from, &rule.to),
            Decision::Forbidden,
        ));
    }
    let rules = &config.rules;
    if rules.database.migrations_only {
        resolver.add(PolicyStatement::new(
            explicit(super::migrations_only::RULE_ID),
            PolicyTopic::MigrationsOnly,
            Decision::Required,
        ));
    }
    if rules.tests.public_api_changes_require_tests {
        resolver.add(PolicyStatement::new(
            explicit(super::api_tests::RULE_ID),
            PolicyTopic::TestsForApiChange,
            Decision::Required,
        ));
    }
    if rules.queue_jobs.require_deterministic_id {
        resolver.add(PolicyStatement::new(
            explicit(super::queue_ids::RULE_ID),
            PolicyTopic::JobIdDeterminism,
            Decision::Required,
        ));
    }
}

/// The topic a convention speaks about.
pub fn convention_topic(id: &str) -> Option<(PolicyTopic, Decision)> {
    if let Some(pair) = id.strip_prefix("layer_dependency:") {
        let (from, to) = pair.split_once("->")?;
        return Some((PolicyTopic::layer_dependency(from, to), Decision::Forbidden));
    }
    Some(match id {
        controller_repository::ID => (
            PolicyTopic::layer_dependency("controller", "repository"),
            Decision::Forbidden,
        ),
        transaction_wrapper::ID => (PolicyTopic::TransactionOnWrite, Decision::Required),
        guard_usage::ID => (PolicyTopic::GuardOnEndpoint, Decision::Required),
        queues::ID => (PolicyTopic::JobIdDeterminism, Decision::Required),
        error_hierarchy::ID => (PolicyTopic::ErrorHierarchy, Decision::Required),
        _ => return None,
    })
}
