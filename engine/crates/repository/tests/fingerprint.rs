#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::path::Path;

use repository::config_hash::compute_config_hash;
use repository::fingerprint::{
    commit_component, compute_fingerprint, local_repository_id, worktree_hash, CommitComponent,
    Fingerprint, FingerprintInputs, RepoIdentity,
};
use repository::git::{discover, GitOpenOptions};
use repository::init::{run, FingerprintParams, InitOptions};
use repository::manifests::detect_manifests;
use repository::read::BoundedReader;
use repository::tsconfig::detect_tsconfigs;
use repository::walk::{walk, WalkOptions};
use review_core::ids::{CommitSha, RepositoryId};
use review_core::language::Language;
use review_core::location::RepoPath;
use review_core::version::AnalyzerVersion;
use review_test_support::{add_origin, fixture_copy, fixture_copy_named, git, write_file};

const SHA_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const SHA_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

struct Base {
    repo: RepoIdentity,
    commit: CommitComponent,
    analyzers: BTreeMap<Language, AnalyzerVersion>,
    graph: u32,
    config: [u8; 32],
    parsers: Vec<(&'static str, &'static str)>,
    profile: u32,
}

impl Base {
    fn new() -> Self {
        let mut analyzers = BTreeMap::new();
        analyzers.insert(Language::Typescript, AnalyzerVersion::new(0, 1, 0));
        Self {
            repo: RepoIdentity::Local("0123456789abcdef0123456789abcdef".to_owned()),
            commit: CommitComponent::Clean(SHA_A.parse::<CommitSha>().unwrap()),
            analyzers,
            graph: 1,
            config: [7u8; 32],
            parsers: vec![
                ("tree-sitter", "0.25.0"),
                ("tree-sitter-typescript", "0.23.0"),
            ],
            profile: 1,
        }
    }

    fn fp(&self) -> Fingerprint {
        compute_fingerprint(&FingerprintInputs {
            repository_id: &self.repo,
            commit: &self.commit,
            analyzer_versions: &self.analyzers,
            graph_schema_version: self.graph,
            config_hash: self.config,
            parser_versions: &self.parsers,
            profile_version: self.profile,
        })
    }
}

#[test]
fn golden_vector_v1() {
    let fp = Base::new().fp().to_string();
    assert!(fp.starts_with("fp1:") && fp.len() == 68);
    assert_eq!(
        fp, GOLDEN,
        "the encoding changed: this needs an ADR-015 update and a v2 domain"
    );
    assert_eq!(fp.parse::<Fingerprint>().unwrap().to_string(), fp);
}

const GOLDEN: &str = "fp1:99117c037362963eb69fe197ce28460e27a630429061748d437a2ca2d0c575e3";

#[test]
fn each_input_changes_fingerprint() {
    let base = Base::new().fp();
    let mut cases: Vec<(&str, Fingerprint)> = Vec::new();

    let mut b = Base::new();
    b.repo = RepoIdentity::Hosted(RepositoryId::new());
    cases.push(("repository_id", b.fp()));

    let mut b = Base::new();
    b.commit = CommitComponent::Clean(SHA_B.parse().unwrap());
    cases.push(("commit", b.fp()));

    let mut b = Base::new();
    b.analyzers
        .insert(Language::Typescript, AnalyzerVersion::new(0, 2, 0));
    cases.push(("analyzer_versions", b.fp()));

    let mut b = Base::new();
    b.graph = 2;
    cases.push(("graph_schema_version", b.fp()));

    let mut b = Base::new();
    b.config = [8u8; 32];
    cases.push(("config_hash", b.fp()));

    let mut b = Base::new();
    b.parsers[0].1 = "0.25.1";
    cases.push(("parser_versions", b.fp()));

    let mut b = Base::new();
    b.profile = 2;
    cases.push(("profile_version", b.fp()));

    assert_eq!(cases.len(), 7);
    for (name, fp) in &cases {
        assert_ne!(*fp, base, "{name} must change the fingerprint");
    }
    let mut unique: Vec<_> = cases.iter().map(|(_, f)| *f).collect();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), 7);
}

#[test]
fn parser_and_analyzer_order_does_not_matter() {
    let mut a = Base::new();
    a.parsers = vec![("b", "1"), ("a", "2")];
    let mut b = Base::new();
    b.parsers = vec![("a", "2"), ("b", "1")];
    assert_eq!(a.fp(), b.fp());
}

#[test]
fn dirty_differs_from_clean_same_head() {
    let clean = Base::new();
    let mut dirty = Base::new();
    dirty.commit = CommitComponent::Dirty {
        head: SHA_A.parse().unwrap(),
        worktree_hash: [1u8; 32],
    };
    assert_ne!(clean.fp(), dirty.fp());
    let mut nogit = Base::new();
    nogit.commit = CommitComponent::NoGit {
        tree_hash: [1u8; 32],
    };
    assert_ne!(dirty.fp(), nogit.fp());
}

fn dirty_state(root: &Path) -> (repository::walk::FileInventory, Vec<RepoPath>) {
    let discovered = discover(root, &GitOpenOptions::default()).unwrap();
    let all = discovered.git.unwrap().dirty.all_paths;
    let (inv, _) = walk(root, &WalkOptions::default()).unwrap();
    (inv, all)
}

#[test]
fn dirty_hash_changes_with_content() {
    let tmp = fixture_copy("init-basic");
    write_file(tmp.path(), "src/index.ts", "export const a = 1;\n");
    let (inv, changed) = dirty_state(tmp.path());
    assert_eq!(changed.len(), 1);
    let reader = BoundedReader::new(&inv.root);
    let (h1, _) = worktree_hash(&inv.root, &inv, &reader, &changed);
    write_file(tmp.path(), "src/index.ts", "export const a = 2;\n");
    let (inv, changed) = dirty_state(tmp.path());
    let reader = BoundedReader::new(&inv.root);
    let (h2, _) = worktree_hash(&inv.root, &inv, &reader, &changed);
    assert_ne!(h1, h2);
    let (h2b, _) = worktree_hash(&inv.root, &inv, &reader, &changed);
    assert_eq!(h2, h2b);
}

#[test]
fn deleted_files_are_hashed_distinctly() {
    let tmp = fixture_copy("init-basic");
    std::fs::remove_file(tmp.path().join("src/index.ts")).unwrap();
    let (inv, changed) = dirty_state(tmp.path());
    assert_eq!(changed[0].as_str(), "src/index.ts");
    let reader = BoundedReader::new(&inv.root);
    let (deleted, _) = worktree_hash(&inv.root, &inv, &reader, &changed);
    write_file(
        tmp.path(),
        "src/index.ts",
        "export function main(): string {\n  return 'hello';\n}\n",
    );
    let (inv2, changed2) = dirty_state(tmp.path());
    let reader2 = BoundedReader::new(&inv2.root);
    let (modified, _) = worktree_hash(&inv2.root, &inv2, &reader2, &changed2);
    assert_ne!(deleted, modified);
}

#[test]
fn sensitive_file_content_not_hashed() {
    let tmp = fixture_copy("init-basic");
    write_file(tmp.path(), ".env.local", "TOKEN=aaaaaaaa\n");
    let (inv, changed) = dirty_state(tmp.path());
    assert!(changed.iter().any(|p| p.as_str() == ".env.local"));
    let reader = BoundedReader::new(&inv.root);
    let (h1, _) = worktree_hash(&inv.root, &inv, &reader, &changed);
    // Same size, different content: the hash must not change.
    write_file(tmp.path(), ".env.local", "TOKEN=bbbbbbbb\n");
    let (inv, changed) = dirty_state(tmp.path());
    let reader = BoundedReader::new(&inv.root);
    let (h2, _) = worktree_hash(&inv.root, &inv, &reader, &changed);
    assert_eq!(h1, h2);
    // A different size does change it.
    write_file(tmp.path(), ".env.local", "TOKEN=bbbbbbbbbbbb\n");
    let (inv, changed) = dirty_state(tmp.path());
    let reader = BoundedReader::new(&inv.root);
    let (h3, _) = worktree_hash(&inv.root, &inv, &reader, &changed);
    assert_ne!(h1, h3);
}

fn config_hash(root: &Path) -> [u8; 32] {
    let (inv, _) = walk(root, &WalkOptions::default()).unwrap();
    let reader = BoundedReader::new(&inv.root);
    let (manifests, _) = detect_manifests(&inv, &reader);
    let (ts, _) = detect_tsconfigs(&inv, &reader);
    compute_config_hash(&inv.root, &inv, &reader, &ts, &manifests).0
}

#[test]
fn config_yaml_formatting_and_comments_ignored() {
    let tmp = tempfile::tempdir().unwrap();
    write_file(
        tmp.path(),
        ".review/config.yaml",
        "version: 1\nignore:\n  - a\n  - b\n",
    );
    let h1 = config_hash(tmp.path());
    write_file(
        tmp.path(),
        ".review/config.yaml",
        "# a comment\nignore: [a, b]   # trailing\nversion:   1\n",
    );
    assert_eq!(h1, config_hash(tmp.path()));
    write_file(
        tmp.path(),
        ".review/config.yaml",
        "version: 1\nignore: [a]\n",
    );
    assert_ne!(h1, config_hash(tmp.path()));
    // Absent differs from present.
    std::fs::remove_file(tmp.path().join(".review/config.yaml")).unwrap();
    assert_ne!(h1, config_hash(tmp.path()));
}

#[test]
fn base_tsconfig_change_changes_config_hash() {
    let (_tmp, root) = fixture_copy_named("monorepo-pnpm");
    let before = config_hash(&root);
    let base = std::fs::read_to_string(root.join("tsconfig.base.json")).unwrap();
    write_file(
        &root,
        "tsconfig.base.json",
        base.replace("ES2022", "ES2020"),
    );
    assert_ne!(before, config_hash(&root));
}

#[test]
fn reviewignore_change_changes_config_hash() {
    let tmp = tempfile::tempdir().unwrap();
    write_file(tmp.path(), ".reviewignore", "a/\n");
    let h1 = config_hash(tmp.path());
    write_file(tmp.path(), ".reviewignore", "a/\nb/\n");
    assert_ne!(h1, config_hash(tmp.path()));
}

#[test]
fn local_repository_id_stable_across_clone_locations() {
    let one = fixture_copy("init-basic");
    let two = fixture_copy("init-basic");
    for t in [&one, &two] {
        add_origin(
            t.path(),
            "https://user:secret@GitHub.com/Acme/Widgets.git",
            "main",
        );
    }
    let id = |p: &Path| {
        let g = discover(p, &GitOpenOptions::default()).unwrap();
        local_repository_id(g.git.as_ref(), &g.root)
    };
    let (a, wa) = id(one.path());
    let (b, wb) = id(two.path());
    assert_eq!(a, b);
    assert!(wa.is_none() && wb.is_none());
    match a {
        RepoIdentity::Local(hex) => assert_eq!(hex.len(), 32),
        other => panic!("{other:?}"),
    }
    // Without a remote the id comes from the path and a warning says so.
    let three = fixture_copy("init-basic");
    let four = fixture_copy("init-basic");
    let (c, wc) = id(three.path());
    let (d, _) = id(four.path());
    assert_ne!(c, d);
    assert_eq!(wc.unwrap().code, "repository_id_from_path");
}

#[test]
fn commit_component_follows_worktree_state() {
    let tmp = fixture_copy("init-basic");
    let discovered = discover(tmp.path(), &GitOpenOptions::default()).unwrap();
    let (inv, _) = walk(tmp.path(), &WalkOptions::default()).unwrap();
    let reader = BoundedReader::new(&inv.root);
    let (clean, _) = commit_component(discovered.git.as_ref(), &discovered.root, &inv, &reader);
    assert!(matches!(clean, CommitComponent::Clean(_)));
    write_file(tmp.path(), "src/x.ts", "export {};\n");
    let discovered = discover(tmp.path(), &GitOpenOptions::default()).unwrap();
    let (inv, _) = walk(tmp.path(), &WalkOptions::default()).unwrap();
    let reader = BoundedReader::new(&inv.root);
    let (dirty, _) = commit_component(discovered.git.as_ref(), &discovered.root, &inv, &reader);
    assert!(matches!(dirty, CommitComponent::Dirty { .. }));
    let (nogit, _) = commit_component(None, &discovered.root, &inv, &reader);
    assert!(matches!(nogit, CommitComponent::NoGit { .. }));
    let _ = git(tmp.path(), &["status", "--short"]);
}

#[test]
fn init_fills_a_stable_fingerprint() {
    let (_tmp, root) = fixture_copy_named("init-basic");
    let mut analyzers = BTreeMap::new();
    analyzers.insert(Language::Typescript, AnalyzerVersion::new(0, 1, 0));
    let params = FingerprintParams {
        analyzer_versions: analyzers,
        graph_schema_version: 1,
        parser_versions: vec![("tree-sitter".to_owned(), "0.25.0".to_owned())],
        profile_version: 1,
        repository_id: None,
    };
    let mut opts = InitOptions::new(&root);
    opts.force = true;
    opts.fingerprint = Some(params);
    let first = run(&opts).unwrap();
    let second = run(&opts).unwrap();
    let fp = first.facts().fingerprint.clone().unwrap();
    assert_eq!(fp.len(), 68);
    assert!(fp.starts_with("fp1:") && fp[4..].bytes().all(|b| b.is_ascii_hexdigit()));
    assert_eq!(Some(fp), second.facts().fingerprint.clone());
    // The fingerprint is not part of facts_hash.
    assert_eq!(first.facts().facts_hash, second.facts().facts_hash);
    assert!(first
        .facts()
        .warnings
        .iter()
        .any(|w| w.code == "repository_id_from_path"));
}
