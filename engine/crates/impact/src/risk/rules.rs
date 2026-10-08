//! The versioned risk rule table (RISK-001): signal → category → weight → floor.
//!
//! Risk knowledge lives here, in one inspectable table, rather than in prompt checklists.
//! `docs/reviewers/risk-rules.md` is rendered from this module by [`render_markdown`]; the
//! `risk_rules_doc_up_to_date` test fails when the committed document drifts.

use std::fmt::Write as _;

use super::level::RiskLevel;
use super::signal::{RiskCategory, RuleSource};

/// Bumped whenever a rule, weight or floor changes. Part of the risk input hash.
pub const RISK_RULES_VERSION: u32 = 1;

/// One row of the rule table.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RiskRule {
    pub id: &'static str,
    pub category: RiskCategory,
    pub weight: f32,
    pub source: RuleSource,
    pub description: &'static str,
    pub floor_level: Option<RiskLevel>,
}

const fn rule(
    id: &'static str,
    category: RiskCategory,
    source: RuleSource,
    description: &'static str,
) -> RiskRule {
    RiskRule {
        id,
        category,
        weight: category.base_weight(),
        source,
        description,
        floor_level: None,
    }
}

const fn weighted(
    id: &'static str,
    category: RiskCategory,
    weight: f32,
    source: RuleSource,
    description: &'static str,
    floor_level: Option<RiskLevel>,
) -> RiskRule {
    RiskRule {
        id,
        category,
        weight,
        source,
        description,
        floor_level,
    }
}

use RiskCategory as C;
use RuleSource as S;

/// Every rule, grouped by source.
pub static RULES: &[RiskRule] = &[
    // Path and configuration (RISK-002).
    weighted(
        "path.configured",
        C::Authentication,
        0.8,
        S::Path,
        "File matches a configured risk path; weight by level (critical 0.8, high 0.6, medium 0.4, low 0.1), category inferred from the glob, floor = configured level",
        None,
    ),
    weighted(
        "path.auth",
        C::Authentication,
        0.5,
        S::Path,
        "File under an auth/ or security/ directory (default rule)",
        None,
    ),
    rule(
        "path.migrations",
        C::DatabaseMigrations,
        S::Path,
        "File under a migrations/ directory (default rule)",
    ),
    rule(
        "path.payments",
        C::Payments,
        S::Path,
        "File under a payment or billing directory (default rule)",
    ),
    weighted(
        "path.infra",
        C::DependencyUpdates,
        0.3,
        S::Path,
        "Dockerfile or CI workflow changed (default rule)",
        None,
    ),
    // Manifests and schemas (RISK-002).
    weighted(
        "dep.added",
        C::DependencyUpdates,
        0.3,
        S::Manifest,
        "Dependency added",
        None,
    ),
    weighted(
        "dep.major_bump",
        C::DependencyUpdates,
        0.45,
        S::Manifest,
        "Dependency bumped across a major version",
        None,
    ),
    weighted(
        "dep.prerelease",
        C::DependencyUpdates,
        0.4,
        S::Manifest,
        "Dependency moved to a prerelease version",
        None,
    ),
    weighted(
        "dep.removed",
        C::DependencyUpdates,
        0.25,
        S::Manifest,
        "Dependency removed",
        None,
    ),
    weighted(
        "dep.lock_drift",
        C::DependencyUpdates,
        0.2,
        S::Manifest,
        "Lockfile changed without a manifest change",
        None,
    ),
    rule(
        "migration.added",
        C::DatabaseMigrations,
        S::Manifest,
        "Migration added",
    ),
    weighted(
        "migration.destructive",
        C::DatabaseMigrations,
        0.75,
        S::Manifest,
        "Destructive migration (drop, narrowing type change, NOT NULL without default, rename)",
        Some(RiskLevel::High),
    ),
    weighted(
        "migration.edited_existing",
        C::DatabaseMigrations,
        0.7,
        S::Manifest,
        "An existing migration was edited",
        None,
    ),
    rule(
        "schema.entity_changed",
        C::SchemaChanges,
        S::Manifest,
        "ORM entity columns or relations changed",
    ),
    // Secrets (RISK-002).
    weighted(
        "secrets.literal_added",
        C::Secrets,
        0.9,
        S::Secrets,
        "Secret-like literal added (key, token, private key, high-entropy string)",
        Some(RiskLevel::Critical),
    ),
    weighted(
        "secrets.env_file",
        C::Secrets,
        0.8,
        S::Secrets,
        "Environment file committed",
        None,
    ),
    // Change classes (RISK-003).
    weighted(
        "authz.call_removed",
        C::Authorization,
        0.85,
        S::Change,
        "An authorization call was removed",
        Some(RiskLevel::High),
    ),
    weighted(
        "authz.direct_role_check",
        C::Authorization,
        0.6,
        S::Change,
        "A direct role or permission comparison was added",
        None,
    ),
    weighted(
        "authz.guard_removed",
        C::Authorization,
        0.85,
        S::Change,
        "A guard was removed from an endpoint",
        Some(RiskLevel::High),
    ),
    weighted(
        "authz.guard_added",
        C::Authorization,
        0.4,
        S::Change,
        "A guard was added to an endpoint",
        None,
    ),
    rule(
        "authz.logic_changed",
        C::Authorization,
        S::Change,
        "Authorization logic changed",
    ),
    weighted(
        "validation.removed",
        C::Validation,
        0.65,
        S::Change,
        "Input validation was removed",
        None,
    ),
    weighted(
        "db.write_changed",
        C::SchemaChanges,
        0.4,
        S::Change,
        "A database write changed outside a transaction (persistent data mutation)",
        None,
    ),
    weighted(
        "db.write_changed_in_tx",
        C::TransactionBoundaries,
        0.45,
        S::Change,
        "A database write inside a transaction changed",
        None,
    ),
    weighted(
        "tx.boundary_changed",
        C::TransactionBoundaries,
        0.6,
        S::Change,
        "A transaction boundary changed",
        None,
    ),
    weighted(
        "api.breaking_change",
        C::PublicApiContracts,
        0.6,
        S::Change,
        "Breaking API contract change (removed endpoint, field or parameter, narrowed type)",
        None,
    ),
    weighted(
        "api.additive_change",
        C::PublicApiContracts,
        0.35,
        S::Change,
        "Additive API contract change",
        None,
    ),
    weighted(
        "async.floating_promise",
        C::Concurrency,
        0.5,
        S::Change,
        "A floating promise was introduced, or an await on a database write was removed",
        None,
    ),
    // Framework and graph facts (RISK-003).
    weighted(
        "endpoint.reachable",
        C::PublicApiContracts,
        0.3,
        S::Graph,
        "The change reaches a public HTTP endpoint (amplifies authorization, authentication and validation by 1.15)",
        None,
    ),
    weighted(
        "queue.produce_changed",
        C::Queues,
        0.45,
        S::Framework,
        "A queue producer changed",
        None,
    ),
    weighted(
        "queue.consume_changed",
        C::BackgroundJobs,
        0.45,
        S::Framework,
        "A queue consumer or job handler changed",
        None,
    ),
    weighted(
        "external.call_added",
        C::ExternalNetworkCalls,
        0.4,
        S::Framework,
        "An outbound network call was added",
        None,
    ),
    weighted(
        "fs.access_changed",
        C::FilesystemAccess,
        0.4,
        S::Framework,
        "Filesystem access changed",
        None,
    ),
    rule(
        "crypto.changed",
        C::Cryptography,
        S::Framework,
        "Cryptographic calls changed (crypto, bcrypt, argon2, jsonwebtoken, jose)",
    ),
    weighted(
        "auth.jwt_verify_removed",
        C::Authentication,
        0.8,
        S::Framework,
        "A JWT verification call was removed",
        Some(RiskLevel::High),
    ),
    weighted(
        "auth.authn_changed",
        C::Authentication,
        0.7,
        S::Framework,
        "Authentication strategy or guard changed",
        None,
    ),
    weighted(
        "serialization.changed",
        C::Serialization,
        0.4,
        S::Framework,
        "Serialization behaviour changed",
        None,
    ),
    rule(
        "payments.changed",
        C::Payments,
        S::Framework,
        "Payment code changed",
    ),
    rule(
        "concurrency.shared_state",
        C::Concurrency,
        S::Framework,
        "Shared mutable state written, or concurrent database writes",
    ),
];

/// The rule with this id.
pub fn rule_by_id(id: &str) -> Option<&'static RiskRule> {
    RULES.iter().find(|rule| rule.id == id)
}

/// Static prose sections of the generated document.
const SCORING: &str = "\
## Scoring (RISK-004)

* Per signal: `contribution = clamp(weight × confidence × amplifier, 0, 0.95)`. The amplifier is
  1.15 for authorization, authentication and validation signals of a symbol that has
  `endpoint.reachable`, and 1.0 otherwise.
* Within a category the maximum contribution counts, so many weak signals of one kind never
  stack. Across categories contributions combine by noisy-OR:
  `score = 1 − Π(1 − max_contribution(category))`.
* Levels: `low < 0.25 ≤ medium < 0.50 ≤ high < 0.75 ≤ critical`. A signal's floor raises the
  level (`level = max(threshold level, floors)`), and then the score to the level's minimum
  (`medium 0.25`, `high 0.50`, `critical 0.75`).
* File-subject signals (paths, manifests, secrets) apply to the symbols of that file. A symbol
  classified low-risk (RISK-006) has the confidence of its path signals multiplied by 0.3;
  change and graph signals are never dampened.
* A cluster combines the category maxima of its members by the same noisy-OR. The pull request
  score is the maximum over clusters (a large PR is not riskier per se), plus 0.1 when more
  than 50 files changed. No signal means score 0.0, level low.
* Worked example (PRD §151): `authz.call_removed` 0.85 × 1.15 → 0.95 (clamped),
  `path.configured` (`src/auth/**: critical`) 0.8, `endpoint.reachable` 0.3:
  `1 − 0.05 × 0.2 × 0.7 = 0.993` → critical.
";

const EFFECTS: &str = "\
## Effects (RISK-005)

| Level | Reviewers (if enabled) | Depth | Context × | Impact × | Model tier | Verification | Threshold Δ | Test inspection |
|---|---|---|---|---|---|---|---|---|
| low | correctness (non-cosmetic only) | light | 0.5 | 0.5 | FAST_REASONER | structural (stages 1–4) | +0.05 | no |
| medium | correctness | standard | 1.0 | 1.0 | REVIEW_REASONER | standard (1–7, deterministic contradiction search) | 0 | if untested |
| high | correctness + category reviewers | deep | 1.25 | 1.5 | REVIEW_REASONER | full (1–8, verifier) | 0 | yes |
| critical | correctness + security + category reviewers | deep | 1.5 | 2.0 | REVIEW_REASONER (+ DEEP_REASONER when enabled) | full | −0.03 (never below 0.55) | yes |

Category reviewers: authorization, authentication, cryptography, permissions, secrets,
validation and payments → `security`; database migrations, schema changes and transaction
boundaries → `correctness` with the `database_safety` focus; concurrency, queues and background
jobs → `correctness` + `performance`; public API contracts → `architecture`; untested changed
symbols → `test`. A reviewer disabled by configuration is removed and listed in
`disabled_by_configuration`. Authorization and secrets signals force full verification whatever
the configuration says. Intent (CHG-009) is not an input.
";

const LOW_RISK: &str = "\
## Low-risk changes (RISK-006, PRD §39)

| Kind | Detection |
|---|---|
| formatting_or_comments | cosmetic symbol (hashes equal) or whitespace/comment-only file |
| pure_rename | rename with similarity ≥ 0.95 and an unchanged body, or a file rename with similarity 1.0 |
| generated | generated or vendored file |
| import_sort | only import lines changed and the multiset of specifiers and bindings is equal |
| simple_constant | only the literal initializer of a constant or variable changed |
| snapshot_update | only `*.snap` / `__snapshots__/` files changed |

Overrides (any one makes the change not low-risk): an exported symbol renamed while callers
outside the PR exist, or an API contract change; any change class on the symbol; a simple
constant read by authorization, validation, cryptography or payment code, or used as a
configuration default; a file under a `high` or `critical` risk path; an import reordering that
moves side-effect imports. A degraded parse is never low-risk.
";

const ERROR_HANDLING: &str = "\
## Error handling (decision)

PRD §37 has no error-handling category. `exception_handling_changed` (including swallowed
exceptions) therefore produces **no risk-score signal**; it influences reviewer routing through
the change class (REV-002) instead.
";

/// Renders `docs/reviewers/risk-rules.md`.
pub fn render_markdown() -> String {
    let mut out = String::new();
    out.push_str("# Risk rules\n\n");
    out.push_str(
        "Generated from `engine/crates/impact/src/risk/rules.rs` by\n\
         `impact::risk::rules::render_markdown()`; do not edit by hand. The\n\
         `risk_rules_doc_up_to_date` test fails when this file drifts.\n\n",
    );
    let _ = writeln!(out, "Rules version: {RISK_RULES_VERSION}\n");
    out.push_str("## Categories (PRD §37)\n\n");
    out.push_str("| Category | Base weight |\n|---|---|\n");
    for category in RiskCategory::ALL {
        let _ = writeln!(
            out,
            "| {} | {:.2} |",
            category.as_str(),
            category.base_weight()
        );
    }
    out.push_str("\n## Rules\n\n");
    out.push_str(
        "| Rule | Category | Source | Weight | Floor | Description |\n|---|---|---|---|---|---|\n",
    );
    for rule in RULES {
        let floor = rule.floor_level.map_or("—", RiskLevel::as_str);
        let _ = writeln!(
            out,
            "| `{}` | {} | {} | {:.2} | {} | {} |",
            rule.id,
            rule.category.as_str(),
            rule.source.as_str(),
            rule.weight,
            floor,
            rule.description
        );
    }
    out.push('\n');
    out.push_str(SCORING);
    out.push('\n');
    out.push_str(EFFECTS);
    out.push('\n');
    out.push_str(LOW_RISK);
    out.push('\n');
    out.push_str(ERROR_HANDLING);
    out
}
