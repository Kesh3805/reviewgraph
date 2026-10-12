//! `rules.database.migrations_only` (POL-003): a schema-affecting change outside the migration
//! paths is a violation. Three shapes are recognized:
//!
//! 1. A changed entity column (a `DatabaseColumn`, or a field of a `DatabaseEntity`) when the
//!    pull request adds or changes no migration.
//! 2. Raw DDL (`CREATE TABLE`, `ALTER TABLE`, ...) added in non-migration code.
//! 3. `synchronize: true` added in non-test code (schema sync bypasses migrations).
//!
//! Without configured `migration_paths` and with no detectable migrations the rule reports
//! `NOT_EXECUTED`.

use std::collections::{BTreeMap, BTreeSet};

use codegraph::{GraphQuery, NodeKind, NodeRef};
use globset::{Glob, GlobSet, GlobSetBuilder};
use review_core::evidence::EvidenceKind;

use super::change::SymbolChange;
use super::evaluator::{FileRange, RuleContext, RuleEvaluator, RuleOutcome, RuleViolation};
use super::evidence::{deterministic, location, symbol_ref};
use super::templates::render;
use crate::config::{ReviewConfigV1, RuleSeverity};
use crate::infer::roles::{is_test_path, role_from_path, Role};

pub const RULE_KEY: &str = "migrations_only";
pub const RULE_ID: &str = "rules.database.migrations_only";

const DDL: &[&str] = &[
    "CREATE TABLE",
    "ALTER TABLE",
    "DROP TABLE",
    "CREATE INDEX",
    "DROP INDEX",
    "ADD COLUMN",
    "DROP COLUMN",
    "RENAME COLUMN",
    "TRUNCATE TABLE",
    "CREATE SCHEMA",
];

const CODE_EXTENSIONS: &[&str] = &[
    "ts", "tsx", "js", "jsx", "mjs", "cjs", "mts", "cts", "py", "go", "rs", "java", "kt", "rb",
    "cs", "php",
];

#[derive(Debug, Clone, Copy, Default)]
pub struct MigrationsOnly;

fn is_code(path: &str) -> bool {
    path.rsplit_once('.')
        .is_some_and(|(_, ext)| CODE_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()))
}

/// Configured migration globs, or globs over detected migration directories.
fn migration_globs(config: &ReviewConfigV1, head: &dyn GraphQuery) -> Option<(GlobSet, Vec<String>)> {
    let mut globs: Vec<String> = config.rules.database.migration_paths.clone();
    if globs.is_empty() {
        let mut dirs = BTreeSet::new();
        head.for_each_node(&mut |node| {
            let Some(path) = node.file else {
                return;
            };
            let in_migrations_dir = path
                .split('/')
                .any(|s| s == "migrations" || s == "migration");
            if node.kind == NodeKind::Migration || in_migrations_dir {
                if let Some((dir, _)) = path.rsplit_once('/') {
                    dirs.insert(format!("{dir}/**"));
                }
            }
        });
        globs = dirs.into_iter().collect();
    }
    if globs.is_empty() {
        return None;
    }
    let mut builder = GlobSetBuilder::new();
    for glob in &globs {
        if let Ok(glob) = Glob::new(glob) {
            builder.add(glob);
        }
    }
    Some((builder.build().ok()?, globs))
}

/// A column of an entity: a `DatabaseColumn`, or a field/property whose parent is an entity or
/// that lives in an entity file.
fn is_entity_column(graph: &dyn GraphQuery, node: &NodeRef<'_>) -> bool {
    if node.kind == NodeKind::DatabaseColumn {
        return true;
    }
    if !matches!(node.kind, NodeKind::Property | NodeKind::Field) {
        return false;
    }
    let parent_is_entity = node
        .attrs
        .parent
        .and_then(|p| graph.node(p))
        .is_some_and(|p| p.kind == NodeKind::DatabaseEntity);
    parent_is_entity
        || node
            .file
            .and_then(role_from_path)
            .is_some_and(|a| a.role == Role::Entity)
}

fn has_ddl(text: &str) -> Option<&'static str> {
    let upper = text.to_ascii_uppercase();
    let squashed = upper.split_whitespace().collect::<Vec<_>>().join(" ");
    DDL.iter().copied().find(|ddl| squashed.contains(ddl))
}

fn enables_synchronize(text: &str) -> bool {
    let lowered = text.to_ascii_lowercase();
    let Some(at) = lowered.find("synchronize") else {
        return false;
    };
    let rest = lowered[at + "synchronize".len()..].trim_start();
    let rest = rest.strip_prefix(['"', '\'']).unwrap_or(rest).trim_start();
    rest.strip_prefix(':')
        .or_else(|| rest.strip_prefix('='))
        .is_some_and(|v| v.trim_start().starts_with("true"))
}

impl RuleEvaluator for MigrationsOnly {
    fn rule_key(&self) -> &'static str {
        RULE_KEY
    }

    fn enabled(&self, config: &ReviewConfigV1) -> bool {
        config.rules.database.migrations_only
    }

    fn evaluate(&self, cx: &RuleContext<'_>) -> RuleOutcome {
        let Some((migrations, globs)) = migration_globs(cx.config, cx.graphs.head) else {
            return RuleOutcome::not_executed(
                RULE_KEY,
                "no migration_paths configured and no migrations detected",
            );
        };
        let paths = globs.join(", ");
        let is_migration = |p: &str| migrations.is_match(p);
        let pr_has_migration = cx.change.live_files().any(is_migration);
        let mut violations = Vec::new();
        let mut push = |path: &str, line: u32, symbol: Option<(&str, &str)>, what: String| {
            let mut params = BTreeMap::new();
            params.insert("rule_id".to_owned(), RULE_ID.to_owned());
            params.insert("what".to_owned(), what.clone());
            params.insert("migration_paths".to_owned(), paths.clone());
            let message = render(RULE_KEY, &params);
            let evidence = deterministic(
                "policy:migrations_only",
                EvidenceKind::DatabaseSchema,
                &message,
                location(path, line, line),
                symbol.and_then(|(id, name)| symbol_ref(id, name)).into_iter().collect(),
            )
            .into_iter()
            .collect();
            violations.push(RuleViolation {
                rule_id: RULE_ID.to_owned(),
                rule_key: RULE_KEY.to_owned(),
                anchor: FileRange {
                    path: path.to_owned(),
                    start_line: line,
                    end_line: line,
                },
                symbol_key: symbol.map(|(id, _)| id.to_owned()),
                evidence,
                severity: RuleSeverity::High,
                params,
                message,
            });
        };

        if !pr_has_migration {
            for symbol in &cx.change.symbols {
                let graph = if symbol.change == SymbolChange::Removed {
                    cx.graphs.base
                } else {
                    cx.graphs.head
                };
                let Some(node) = graph.node(symbol.key) else {
                    continue;
                };
                if is_migration(&symbol.path) || !is_entity_column(graph, &node) {
                    continue;
                }
                push(
                    &symbol.path,
                    symbol.line,
                    Some((node.id, node.qualified_name)),
                    format!("Entity column `{}` changed without a migration", node.qualified_name),
                );
            }
        }
        for added in &cx.change.added_lines {
            if is_migration(&added.path) || is_test_path(&added.path) {
                continue;
            }
            if is_code(&added.path) {
                if let Some(ddl) = has_ddl(&added.text) {
                    push(
                        &added.path,
                        added.line,
                        None,
                        format!("Raw DDL (`{ddl}`) in application code"),
                    );
                    continue;
                }
            }
            if enables_synchronize(&added.text) {
                push(
                    &added.path,
                    added.line,
                    None,
                    "`synchronize: true` lets the ORM change the schema".to_owned(),
                );
            }
        }
        RuleOutcome::executed(RULE_KEY, violations)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ddl_and_synchronize_detection() {
        assert_eq!(has_ddl("await q.query(`alter   table users add column x int`)"), Some("ALTER TABLE"));
        assert_eq!(has_ddl("const tableName = 'users'"), None);
        assert!(enables_synchronize("  synchronize: true,"));
        assert!(enables_synchronize("\"synchronize\": true"));
        assert!(!enables_synchronize("synchronize: false"));
    }
}
