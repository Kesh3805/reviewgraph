#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeSet;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use repository::entrypoints::{detect_entrypoints, EntrySource, EntrypointFact, EntrypointKind};
use repository::env_files::{detect_env_files, EnvFileFact, EnvFileKind};
use repository::git::tracked_among;
use repository::infra::{detect_infra, InfraKind};
use repository::manifests::detect_manifests;
use repository::migrations::{detect_migrations, MigrationNaming, MigrationTool};
use repository::read::{BoundedReader, FileSource, OsFiles};
use repository::tsconfig::detect_tsconfigs;
use repository::walk::{walk, walk_with, FileInventory};
use repository::{InitWarning, WarningSeverity};
use review_core::location::RepoPath;
use review_test_support::{edge_case_tree, fixture_copy, fixture_repo, git, write_file, CANARIES};

fn entrypoints(root: &Path) -> Vec<EntrypointFact> {
    let (inv, _) = walk(root, &Default::default()).unwrap();
    let reader = BoundedReader::new(&inv.root);
    let (manifests, _) = detect_manifests(&inv, &reader);
    let (ts, _) = detect_tsconfigs(&inv, &reader);
    detect_entrypoints(&inv, &reader, &manifests, &ts).0
}

fn find<'a>(facts: &'a [EntrypointFact], path: &str) -> Vec<&'a EntrypointFact> {
    facts.iter().filter(|f| f.path.as_str() == path).collect()
}

fn nest_tree() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    let r = tmp.path();
    write_file(
        r,
        "package.json",
        r#"{"name":"api","main":"dist/index.js","bin":{"api-cli":"dist/cli/main.js"},
            "scripts":{"start:prod":"node dist/main","worker":"node dist/worker.js","seed":"ts-node scripts/seed.ts --password=zz"}}"#,
    );
    write_file(
        r,
        "tsconfig.json",
        r#"{"compilerOptions":{"outDir":"./dist","rootDir":"./src"}}"#,
    );
    write_file(
        r,
        "src/main.ts",
        "import { NestFactory } from \"@nestjs/core\";\nNestFactory.create(AppModule);\n",
    );
    write_file(
        r,
        "src/worker.ts",
        "NestFactory.createApplicationContext(WorkerModule);\n",
    );
    write_file(r, "src/index.ts", "export * from \"./app\";\n");
    write_file(
        r,
        "src/cli/main.ts",
        "#!/usr/bin/env node\nconsole.log(1);\n",
    );
    write_file(r, "scripts/seed.ts", "console.log(\"seed\");\n");
    write_file(
        r,
        "src/queue/consumer.ts",
        "import { Worker } from \"bullmq\";\nnew Worker(\"q\", async () => {});\n",
    );
    write_file(
        r,
        "Dockerfile",
        "FROM node:20\nCMD [\"node\", \"dist/main.js\"]\n",
    );
    write_file(r, "docker-compose.yml", "services: {}\n");
    write_file(r, "infra/main.tf", "resource \"x\" \"y\" {}\n");
    write_file(
        r,
        "k8s/deploy.yaml",
        "apiVersion: apps/v1\nkind: Deployment\nmetadata:\n  name: x\n",
    );
    write_file(
        r,
        "schema/migrations/20260101_init.sql",
        "create table a();\n",
    );
    write_file(
        r,
        "schema/migrations/20260102_more.sql",
        "create table b();\n",
    );
    write_file(
        r,
        "schema/migrations/20260103_more.sql",
        "create table c();\n",
    );
    write_file(
        r,
        ".env.example",
        "API_KEY=changeme\nexport DB_URL=postgres://u:p@h/db\n# COMMENT=1\n",
    );
    tmp
}

#[test]
fn nest_bootstrap_http_server() {
    let tmp = nest_tree();
    let facts = entrypoints(tmp.path());
    let main = find(&facts, "src/main.ts");
    assert!(main
        .iter()
        .any(|f| f.kind == EntrypointKind::HttpServer && f.confidence >= 0.9));
    assert!(main.iter().any(|f| matches!(
        &f.source,
        EntrySource::Bootstrap { call } if call == "NestFactory.create"
    )));
    assert_eq!(main[0].package.as_deref(), Some("api"));
}

#[test]
fn application_context_is_worker() {
    let tmp = nest_tree();
    let facts = entrypoints(tmp.path());
    assert!(find(&facts, "src/worker.ts")
        .iter()
        .any(|f| f.kind == EntrypointKind::Worker));
    assert!(find(&facts, "src/queue/consumer.ts")
        .iter()
        .any(|f| f.kind == EntrypointKind::Worker));
}

#[test]
fn package_main_dist_maps_to_src() {
    let tmp = nest_tree();
    let facts = entrypoints(tmp.path());
    assert!(find(&facts, "src/index.ts")
        .iter()
        .any(|f| f.kind == EntrypointKind::Library && f.source == EntrySource::PackageMain));
    // `node dist/main` in a script maps back to src/main.ts (no bootstrap call to outrank it)
    let plain = tempfile::tempdir().unwrap();
    write_file(
        plain.path(),
        "package.json",
        r#"{"name":"p","scripts":{"start:prod":"node dist/main"}}"#,
    );
    write_file(
        plain.path(),
        "tsconfig.json",
        r#"{"compilerOptions":{"outDir":"./dist","rootDir":"./src"}}"#,
    );
    write_file(
        plain.path(),
        "src/main.ts",
        "export {};
",
    );
    let facts = entrypoints(plain.path());
    assert!(find(&facts, "src/main.ts")
        .iter()
        .any(|f| matches!(&f.source, EntrySource::Script { name } if name == "start:prod")));
}

#[test]
fn bin_entries_are_cli() {
    let tmp = nest_tree();
    let facts = entrypoints(tmp.path());
    assert!(find(&facts, "src/cli/main.ts")
        .iter()
        .any(|f| f.kind == EntrypointKind::Cli
            && matches!(&f.source, EntrySource::PackageBin { name } if name == "api-cli")));
    assert!(find(&facts, "scripts/seed.ts")
        .iter()
        .any(|f| f.kind == EntrypointKind::Script));
}

#[test]
fn conventional_main_only_when_no_stronger_source() {
    let tmp = tempfile::tempdir().unwrap();
    write_file(tmp.path(), "package.json", r#"{"name":"lib"}"#);
    write_file(tmp.path(), "src/index.ts", "export {};\n");
    let facts = entrypoints(tmp.path());
    assert_eq!(facts.len(), 1);
    assert_eq!(facts[0].source, EntrySource::Conventional);
    assert_eq!(facts[0].confidence, 0.5);

    let tmp = nest_tree();
    let facts = entrypoints(tmp.path());
    assert!(find(&facts, "src/index.ts")
        .iter()
        .all(|f| f.source != EntrySource::Conventional));
}

#[test]
fn dockerfile_cmd_mapped() {
    let tmp = tempfile::tempdir().unwrap();
    write_file(tmp.path(), "package.json", r#"{"name":"p"}"#);
    write_file(
        tmp.path(),
        "tsconfig.json",
        r#"{"compilerOptions":{"outDir":"./dist","rootDir":"./src"}}"#,
    );
    write_file(
        tmp.path(),
        "src/main.ts",
        "export {};
",
    );
    write_file(
        tmp.path(),
        "Dockerfile",
        "FROM node:20
CMD [\"node\", \"dist/main.js\"]
",
    );
    let facts = entrypoints(tmp.path());
    assert!(find(&facts, "src/main.ts")
        .iter()
        .any(|f| f.source == EntrySource::Dockerfile && f.kind == EntrypointKind::HttpServer));
}

#[test]
fn monorepo_entrypoints_map_per_package() {
    let repo = fixture_repo("monorepo-pnpm");
    let facts = entrypoints(&repo);
    let main = find(&facts, "apps/api/src/main.ts");
    assert!(main.iter().any(|f| f.kind == EntrypointKind::HttpServer));
    assert_eq!(main[0].package.as_deref(), Some("@acme/api"));
}

#[test]
fn migration_dir_schema_migrations_timestamped() {
    let tmp = nest_tree();
    let (inv, _) = walk(tmp.path(), &Default::default()).unwrap();
    let facts = detect_migrations(&inv);
    let dir = facts
        .dirs
        .iter()
        .find(|d| d.dir.as_str() == "schema/migrations")
        .unwrap();
    assert_eq!(dir.naming, MigrationNaming::TimestampPrefix);
    assert_eq!(dir.files, 3);
    assert_eq!(dir.tool_hint, Some(MigrationTool::RawSql));
}

#[test]
fn prisma_migrations_tool_hint() {
    let tmp = tempfile::tempdir().unwrap();
    write_file(
        tmp.path(),
        "prisma/schema.prisma",
        "model A { id Int @id }\n",
    );
    write_file(
        tmp.path(),
        "prisma/migrations/20260101000000_init/migration.sql",
        "x\n",
    );
    write_file(
        tmp.path(),
        "prisma/migrations/20260102000000_more/migration.sql",
        "x\n",
    );
    let (inv, _) = walk(tmp.path(), &Default::default()).unwrap();
    let facts = detect_migrations(&inv);
    assert_eq!(facts.dirs.len(), 1);
    assert_eq!(facts.dirs[0].tool_hint, Some(MigrationTool::Prisma));
    assert_eq!(facts.dirs[0].files, 2);
    assert_eq!(facts.schema_files.len(), 1);
}

#[test]
fn infra_kinds_detected() {
    let tmp = nest_tree();
    write_file(tmp.path(), ".dockerignore", "node_modules\n");
    write_file(tmp.path(), "charts/app/Chart.yaml", "name: app\n");
    write_file(tmp.path(), "fly.toml", "app = \"x\"\n");
    let (inv, _) = walk(tmp.path(), &Default::default()).unwrap();
    let reader = BoundedReader::new(&inv.root);
    let (facts, _) = detect_infra(&inv, &reader);
    let kind_of = |p: &str| facts.iter().find(|f| f.path.as_str() == p).map(|f| f.kind);
    assert_eq!(kind_of("Dockerfile"), Some(InfraKind::Dockerfile));
    assert_eq!(kind_of("docker-compose.yml"), Some(InfraKind::Compose));
    assert_eq!(kind_of("infra/main.tf"), Some(InfraKind::Terraform));
    assert_eq!(kind_of("charts/app/Chart.yaml"), Some(InfraKind::Helm));
    assert_eq!(kind_of("fly.toml"), Some(InfraKind::Fly));
    assert_eq!(kind_of(".dockerignore"), Some(InfraKind::DockerIgnore));
}

#[test]
fn kubernetes_yaml_by_header() {
    let tmp = nest_tree();
    let (inv, _) = walk(tmp.path(), &Default::default()).unwrap();
    let reader = BoundedReader::new(&inv.root);
    let (facts, _) = detect_infra(&inv, &reader);
    let k8s = facts
        .iter()
        .find(|f| f.path.as_str() == "k8s/deploy.yaml")
        .unwrap();
    assert_eq!(k8s.kind, InfraKind::Kubernetes);
    assert!(facts
        .iter()
        .all(|f| f.path.as_str() != "docker-compose.yml" || f.kind == InfraKind::Compose));
}

#[test]
fn env_template_keys_only() {
    let tmp = nest_tree();
    let (inv, _) = walk(tmp.path(), &Default::default()).unwrap();
    let reader = BoundedReader::new(&inv.root);
    let (facts, _) = detect_env_files(&inv, &reader, &BTreeSet::new());
    assert_eq!(facts.len(), 1);
    assert_eq!(facts[0].kind, EnvFileKind::Template);
    assert_eq!(facts[0].variable_names, vec!["API_KEY", "DB_URL"]);
    let json = serde_json::to_string(&facts).unwrap();
    assert!(!json.contains("changeme"));
    assert!(!json.contains("postgres://"));
}

#[derive(Debug, Default)]
struct CountingFiles {
    opened: Mutex<Vec<PathBuf>>,
}

impl FileSource for CountingFiles {
    fn read_prefix(&self, absolute: &Path, max: usize) -> io::Result<Vec<u8>> {
        self.opened.lock().unwrap().push(absolute.to_path_buf());
        OsFiles.read_prefix(absolute, max)
    }
}

fn all_facts_json(
    inv: &FileInventory,
    reader: &BoundedReader,
    tracked: &BTreeSet<RepoPath>,
) -> (Vec<EnvFileFact>, Vec<InitWarning>, String) {
    let (env, warnings) = detect_env_files(inv, reader, tracked);
    let (manifests, _) = detect_manifests(inv, reader);
    let (ts, _) = detect_tsconfigs(inv, reader);
    let (entry, _) = detect_entrypoints(inv, reader, &manifests, &ts);
    let (infra, _) = detect_infra(inv, reader);
    let migrations = detect_migrations(inv);
    let json = serde_json::to_string(&(&env, &warnings, entry, infra, migrations)).unwrap();
    (env, warnings, json)
}

#[test]
fn real_env_file_never_opened() {
    let tmp = edge_case_tree();
    let counting = Arc::new(CountingFiles::default());
    let (inv, _) = walk_with(tmp.path(), &Default::default(), counting.clone()).unwrap();
    let reader = BoundedReader::with_source(&inv.root, counting.clone());
    let (env, _, json) = all_facts_json(&inv, &reader, &BTreeSet::new());
    let real = env.iter().find(|e| e.path.as_str() == ".env.test").unwrap();
    assert_eq!(real.kind, EnvFileKind::Real);
    assert!(real.variable_names.is_empty());
    let opened = counting.opened.lock().unwrap();
    assert!(opened
        .iter()
        .all(|p| !p.ends_with(".env.test") && !p.ends_with(".env.production")));
    for canary in CANARIES {
        assert!(!json.contains(canary));
    }
}

#[test]
fn committed_env_file_warns() {
    let tmp = fixture_copy("init-basic");
    write_file(tmp.path(), ".env.test", "TOKEN=RG_CANARY_ENV_77\n");
    git(tmp.path(), &["add", "-f", ".env.test"]);
    let (inv, _) = walk(tmp.path(), &Default::default()).unwrap();
    let reader = BoundedReader::new(&inv.root);
    let candidates: Vec<RepoPath> = inv.entries.iter().map(|e| e.path.clone()).collect();
    let tracked = tracked_among(&inv.root, &candidates);
    assert!(tracked.contains(&RepoPath::new(".env.test").unwrap()));
    assert!(tracked.contains(&RepoPath::new("package.json").unwrap()));
    let (env, warnings, json) = all_facts_json(&inv, &reader, &tracked);
    assert!(env[0].tracked_in_git);
    let w = warnings
        .iter()
        .find(|w| w.code == "env_file_committed")
        .unwrap();
    assert_eq!(w.severity, WarningSeverity::High);
    assert!(!json.contains("RG_CANARY_ENV_77"));
}

#[test]
#[ignore = "needs RG_REFERENCE_REPO_PATH"]
fn reference_entrypoints() {
    let root = std::env::var("RG_REFERENCE_REPO_PATH").unwrap();
    let facts = entrypoints(Path::new(&root));
    assert!(find(&facts, "src/main.ts")
        .iter()
        .any(|f| f.kind == EntrypointKind::HttpServer));
}
