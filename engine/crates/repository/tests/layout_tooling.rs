#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;

use repository::layout::{detect_layout, LayoutFacts, RootSource};
use repository::manifests::detect_manifests;
use repository::read::BoundedReader;
use repository::tooling::{detect_tooling, CiProvider, ToolingFacts};
use repository::tsconfig::detect_tsconfigs;
use repository::walk::{walk, WalkOptions};
use repository::workspaces::detect_workspaces;
use review_test_support::write_file;

fn run(root: &Path) -> (LayoutFacts, ToolingFacts, Vec<repository::InitWarning>) {
    let (inv, _) = walk(root, &WalkOptions::default()).unwrap();
    let reader = BoundedReader::new(&inv.root);
    let (manifests, _) = detect_manifests(&inv, &reader);
    let (ts, _) = detect_tsconfigs(&inv, &reader);
    let (ws, _) = detect_workspaces(&inv, &reader, &manifests);
    let (layout, mut warnings) = detect_layout(&inv, &reader, &ts, &ws);
    let (tooling, w2) = detect_tooling(&inv, &reader, &ts, &manifests);
    warnings.extend(w2);
    (layout, tooling, warnings)
}

#[test]
fn nest_cli_source_root_preferred() {
    let tmp = tempfile::tempdir().unwrap();
    write_file(
        tmp.path(),
        "nest-cli.json",
        r#"{"sourceRoot":"src","projects":{"worker":{"sourceRoot":"apps/worker/src"}}}"#,
    );
    write_file(
        tmp.path(),
        "tsconfig.json",
        r#"{"compilerOptions":{"rootDir":"./src"}}"#,
    );
    write_file(tmp.path(), "src/main.ts", "export {};\n");
    write_file(tmp.path(), "apps/worker/src/main.ts", "export {};\n");
    let (layout, _, _) = run(tmp.path());
    let src = layout
        .source_roots
        .iter()
        .find(|r| r.path.as_str() == "src")
        .unwrap();
    assert_eq!(src.source, RootSource::NestCliSourceRoot);
    assert_eq!(src.confidence, 0.95);
    assert!(layout
        .source_roots
        .iter()
        .any(|r| r.path.as_str() == "apps/worker/src"));
}

#[test]
fn test_roots_from_jest_json_key() {
    let tmp = tempfile::tempdir().unwrap();
    write_file(
        tmp.path(),
        "package.json",
        r#"{"name":"x","jest":{"roots":["<rootDir>/specs"],"testMatch":["**/*.check.ts"]}}"#,
    );
    write_file(tmp.path(), "specs/a.check.ts", "export {};\n");
    let (layout, _, _) = run(tmp.path());
    let root = layout
        .test_roots
        .iter()
        .find(|r| r.path.as_str() == "specs")
        .unwrap();
    assert_eq!(root.source, RootSource::JestConfig);
    assert_eq!(root.confidence, 0.9);
    assert!(layout.test_globs.contains(&"**/*.check.ts".to_owned()));
    assert!(layout.test_globs.iter().any(|g| g.contains("e2e-spec")));
}

#[test]
fn jest_config_ts_static_extraction_low_confidence() {
    let tmp = tempfile::tempdir().unwrap();
    write_file(
        tmp.path(),
        "jest.config.ts",
        "export default {\n  roots: ['<rootDir>/tests', \"<rootDir>/more\"],\n  testRegex: '.*\\\\.spec\\\\.ts$',\n};\nconsole.log(process.env.SECRET);\n",
    );
    let (layout, _, warnings) = run(tmp.path());
    let tests = layout
        .test_roots
        .iter()
        .find(|r| r.path.as_str() == "tests")
        .unwrap();
    assert_eq!(tests.confidence, 0.6);
    assert!(layout.test_roots.iter().any(|r| r.path.as_str() == "more"));
    assert_eq!(layout.test_regexes.len(), 1);
    assert!(warnings
        .iter()
        .any(|w| w.code == "config_static_extraction"));
}

#[test]
fn jest_e2e_json_testregex() {
    let tmp = tempfile::tempdir().unwrap();
    write_file(
        tmp.path(),
        "test/jest-e2e.json",
        r#"{"moduleFileExtensions":["js","ts"],"rootDir":".","testRegex":".e2e-spec.ts$"}"#,
    );
    write_file(tmp.path(), "test/app.e2e-spec.ts", "export {};\n");
    let (layout, _, _) = run(tmp.path());
    assert_eq!(layout.test_regexes, vec![".e2e-spec.ts$"]);
    assert!(layout.test_roots.iter().any(|r| r.path.as_str() == "test"));
}

#[test]
fn eslint_flat_config_detected() {
    let tmp = tempfile::tempdir().unwrap();
    write_file(tmp.path(), "eslint.config.mjs", "export default [];\n");
    write_file(tmp.path(), ".prettierrc", "{}\n");
    write_file(tmp.path(), "biome.json", "{}\n");
    let (_, tooling, _) = run(tmp.path());
    let tools: Vec<&str> = tooling.lint.iter().map(|t| t.tool.as_str()).collect();
    assert!(tools.contains(&"eslint"));
    assert!(tools.contains(&"prettier"));
    assert!(tools.contains(&"biome"));
}

const WORKFLOW: &str = "name: CI/CD\non:\n  push:\n    branches: [main]\n  pull_request:\njobs:\n  test:\n    runs-on: ubuntu-latest\n    steps:\n      - uses: actions/checkout@v4\n      - run: npm ci\n      - run: |\n          pnpm test --token=sk_live_SUPERSECRET\n          docker build -t app .\n      - env:\n          TOKEN: ${{ secrets.DEPLOY_TOKEN }}\n        run: 'curl -H \"Authorization: Bearer abc123\" https://x.example'\n";

#[test]
fn github_workflow_jobs_and_first_tokens_only() {
    let tmp = tempfile::tempdir().unwrap();
    write_file(tmp.path(), ".github/workflows/ci-cd.yml", WORKFLOW);
    let (_, tooling, _) = run(tmp.path());
    let ci = &tooling.ci[0];
    assert_eq!(ci.provider, CiProvider::GithubActions);
    let wf = ci.workflow.as_ref().unwrap();
    assert_eq!(wf.name.as_deref(), Some("CI/CD"));
    assert_eq!(wf.triggers, vec!["pull_request", "push"]);
    assert_eq!(wf.jobs, vec!["test"]);
    assert_eq!(wf.actions, vec!["actions/checkout@v4"]);
    assert_eq!(wf.run_tools, vec!["curl", "docker", "npm", "pnpm"]);
}

#[test]
fn ci_run_command_full_text_not_stored() {
    let tmp = tempfile::tempdir().unwrap();
    write_file(tmp.path(), ".github/workflows/ci-cd.yml", WORKFLOW);
    let (_, tooling, _) = run(tmp.path());
    let json = serde_json::to_string(&tooling).unwrap();
    for secret in ["sk_live_SUPERSECRET", "abc123", "DEPLOY_TOKEN", "secrets."] {
        assert!(!json.contains(secret), "{secret} leaked");
    }
}

#[test]
fn husky_hooks_detected() {
    let tmp = tempfile::tempdir().unwrap();
    write_file(tmp.path(), ".husky/pre-commit", "npx lint-staged\n");
    write_file(tmp.path(), ".pre-commit-config.yaml", "repos: []\n");
    write_file(
        tmp.path(),
        "lint-staged.config.js",
        "module.exports = {};\n",
    );
    let (_, tooling, _) = run(tmp.path());
    let tools: Vec<&str> = tooling.hooks.iter().map(|t| t.tool.as_str()).collect();
    assert!(tools.contains(&"husky"));
    assert!(tools.contains(&"pre-commit"));
    assert!(tools.contains(&"lint-staged"));
}

#[test]
fn compiler_and_runtime_config_detected() {
    let tmp = tempfile::tempdir().unwrap();
    write_file(tmp.path(), ".nvmrc", "20.11.0\n");
    write_file(tmp.path(), ".swcrc", "{}\n");
    write_file(
        tmp.path(),
        "nest-cli.json",
        r#"{"compilerOptions":{"builder":"swc"}}"#,
    );
    write_file(tmp.path(), "tsconfig.json", "{}");
    write_file(tmp.path(), "package.json", r#"{"engines":{"node":">=20"}}"#);
    let (_, tooling, _) = run(tmp.path());
    let find = |tool: &str| {
        tooling
            .compiler
            .iter()
            .filter(|t| t.tool == tool)
            .collect::<Vec<_>>()
    };
    assert_eq!(find("node").len(), 2);
    assert!(find("node")
        .iter()
        .any(|t| t.detail.as_deref() == Some("20.11.0")));
    assert_eq!(find("nest-cli")[0].detail.as_deref(), Some("swc"));
    assert_eq!(find("tsconfig").len(), 1);
    assert_eq!(find("swc").len(), 1);
}

#[test]
fn conventional_roots_need_code() {
    let tmp = tempfile::tempdir().unwrap();
    write_file(tmp.path(), "src/readme.md", "x\n");
    write_file(tmp.path(), "lib/a.ts", "export {};\n");
    write_file(tmp.path(), "tests/a.test.ts", "export {};\n");
    let (layout, _, _) = run(tmp.path());
    let roots: Vec<&str> = layout
        .source_roots
        .iter()
        .map(|r| r.path.as_str())
        .collect();
    assert_eq!(roots, vec!["lib"]);
    assert!(layout.test_roots.iter().any(|r| r.path.as_str() == "tests"));
}
