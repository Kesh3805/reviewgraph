#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;

use repository::codeowners::parse_codeowners;
use repository::docs_meta::{detect_docs, DocsFacts, VaultKind};
use repository::read::BoundedReader;
use repository::walk::{walk, WalkOptions};
use review_core::location::RepoPath;
use review_test_support::write_file;

fn docs(root: &Path) -> DocsFacts {
    let (inv, _) = walk(root, &WalkOptions::default()).unwrap();
    let reader = BoundedReader::new(&inv.root);
    detect_docs(&inv, &reader).0
}

fn tree() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    let r = tmp.path();
    write_file(
        r,
        "docs/adr/0001-use-nestjs.md",
        "# Use NestJS\n\nStatus: Accepted\n\nWe must use it.\n",
    );
    write_file(
        r,
        "docs/adr/0002-use-typeorm.md",
        "# Use TypeORM\n\n**Status:** Proposed\n",
    );
    write_file(r, "docs/adr/README.md", "# index\n");
    write_file(r, "docs/architecture.md", "# Architecture\n");
    write_file(r, "docs/architecture/layers.md", "# Layers\n");
    write_file(r, "ARCHITECTURE.md", "# Top\n");
    write_file(r, "CONTRIBUTING.md", "# Contributing\n");
    write_file(r, ".github/CODEOWNERS", "# owners\n* @acme/platform\n/src/billing/ @alice bob@example.com\nbroken-line-without-owner\n/docs oops-not-an-owner\n");
    write_file(r, "CODEOWNERS", "* @root-owner\n");
    write_file(
        r,
        "AGENTS.md",
        "# Agents\nYou must always run tests. Never push. Do not skip hooks.\n",
    );
    write_file(r, "CLAUDE.md", "# Claude\n");
    write_file(r, ".github/pull_request_template.md", "## Summary\n");
    write_file(r, "SECURITY.md", "# Security\n");
    write_file(r, ".agent/acme/index.md", "# Index\n");
    write_file(
        r,
        ".agent/acme/security.md",
        "# Security rules\nAlways validate. Never trust. You must sanitize.\n",
    );
    write_file(
        r,
        ".agent/acme/.obsidian/app.json",
        "{\"RG_CANARY_5f1c\":true}\n",
    );
    write_file(r, ".claude/settings.json", "{\"RG_CANARY_5f1c\":true}\n");
    write_file(
        r,
        ".claude/settings.local.json",
        "{\"RG_CANARY_5f1c\":true}\n",
    );
    write_file(r, ".claude/notes.md", "# notes\n");
    write_file(
        r,
        "rules/coding-style.md",
        "# Style\nAlways format. Never skip lint. Must pass CI.\n",
    );
    write_file(r, "notes/random.md", "# nothing\n");
    tmp
}

#[test]
fn adr_number_title_status_extracted() {
    let facts = docs(tree().path());
    assert_eq!(facts.adrs.len(), 2);
    assert_eq!(facts.adrs[0].number, Some(1));
    assert_eq!(facts.adrs[0].title.as_deref(), Some("Use NestJS"));
    assert_eq!(facts.adrs[0].status.as_deref(), Some("Accepted"));
    assert_eq!(facts.adrs[1].status.as_deref(), Some("Proposed"));
}

#[test]
fn adr_dir_inferred_from_numbered_files() {
    let tmp = tempfile::tempdir().unwrap();
    write_file(tmp.path(), "design/records/0001-a.md", "# A\n");
    write_file(tmp.path(), "design/records/0002-b.md", "# B\n");
    write_file(tmp.path(), "design/single/0001-only.md", "# C\n");
    let facts = docs(tmp.path());
    let paths: Vec<&str> = facts.adrs.iter().map(|a| a.path.as_str()).collect();
    assert_eq!(
        paths,
        vec!["design/records/0001-a.md", "design/records/0002-b.md"]
    );
}

#[test]
fn architecture_docs_found() {
    let facts = docs(tree().path());
    let paths: Vec<&str> = facts.architecture.iter().map(|p| p.as_str()).collect();
    assert!(paths.contains(&"docs/architecture.md"));
    assert!(paths.contains(&"docs/architecture/layers.md"));
    assert!(paths.contains(&"ARCHITECTURE.md"));
    assert_eq!(facts.contributing.len(), 1);
    assert_eq!(facts.security_policy.len(), 1);
    assert_eq!(facts.pr_templates.len(), 1);
}

#[test]
fn codeowners_github_location_precedence() {
    let facts = docs(tree().path());
    let co = facts.codeowners.unwrap();
    assert_eq!(co.path.as_str(), ".github/CODEOWNERS");
}

#[test]
fn codeowners_invalid_lines_recorded() {
    let facts = docs(tree().path());
    let co = facts.codeowners.unwrap();
    assert_eq!(co.rules.len(), 2);
    assert_eq!(co.rules[1].owners, vec!["@alice", "bob@example.com"]);
    assert_eq!(co.invalid_lines, vec![4, 5]);
    let parsed = parse_codeowners(RepoPath::new("CODEOWNERS").unwrap(), "* @a/b # trailing\n");
    assert_eq!(parsed.rules[0].owners, vec!["@a/b"]);
}

#[test]
fn agent_instruction_files_listed() {
    let facts = docs(tree().path());
    let paths: Vec<&str> = facts
        .agent_instructions
        .iter()
        .map(|p| p.as_str())
        .collect();
    assert_eq!(paths, vec!["AGENTS.md", "CLAUDE.md"]);
}

#[test]
fn rule_doc_scoring_and_cap() {
    let facts = docs(tree().path());
    let by_path = |p: &str| facts.rule_docs.iter().find(|r| r.path.as_str() == p);
    let agents = by_path("AGENTS.md").unwrap();
    assert_eq!(agents.score, 0.7);
    assert!(by_path("rules/coding-style.md").unwrap().score >= 0.8);
    assert!(by_path("notes/random.md").is_none());
    let mut sorted = facts.rule_docs.clone();
    sorted.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap()
            .then_with(|| a.path.cmp(&b.path))
    });
    assert_eq!(sorted, facts.rule_docs);

    let tmp = tempfile::tempdir().unwrap();
    for i in 0..250 {
        write_file(tmp.path(), &format!("rules/security-{i:03}.md"), "# s\n");
    }
    let (inv, _) = walk(tmp.path(), &WalkOptions::default()).unwrap();
    let reader = BoundedReader::new(&inv.root);
    let (facts, warnings) = detect_docs(&inv, &reader);
    assert_eq!(facts.rule_docs.len(), 200);
    assert!(warnings.iter().any(|w| w.code == "list_truncated"));
}

#[test]
fn agent_vault_and_obsidian_vault_detected() {
    let facts = docs(tree().path());
    let find = |kind| facts.knowledge_vaults.iter().find(|v| v.kind == kind);
    let agent = find(VaultKind::AgentVault).unwrap();
    assert_eq!(agent.root.as_str(), ".agent/acme");
    assert_eq!(agent.markdown_files, 2);
    let obsidian = find(VaultKind::Obsidian).unwrap();
    assert_eq!(obsidian.root.as_str(), ".agent/acme");
    let claude = find(VaultKind::ClaudeDir).unwrap();
    assert_eq!(claude.markdown_files, 1);
}

#[test]
fn claude_settings_not_read() {
    let tmp = tree();
    let (inv, _) = walk(tmp.path(), &WalkOptions::default()).unwrap();
    let reader = BoundedReader::new(&inv.root);
    let (facts, warnings) = detect_docs(&inv, &reader);
    let json = serde_json::to_string(&(&facts, &warnings)).unwrap();
    assert!(!json.contains("RG_CANARY_5f1c"));
}

#[test]
#[ignore = "needs RG_REFERENCE_REPO_PATH"]
fn reference_docs() {
    let root = std::env::var("RG_REFERENCE_REPO_PATH").unwrap();
    let facts = docs(Path::new(&root));
    assert!(facts
        .knowledge_vaults
        .iter()
        .any(|v| v.root.as_str() == ".agent/knowledge" && v.kind == VaultKind::AgentVault));
    assert!(facts
        .architecture
        .iter()
        .any(|p| p.as_str() == "docs/architecture.md"));
}
