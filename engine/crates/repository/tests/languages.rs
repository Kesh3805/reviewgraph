#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use repository::language::{
    detect_language, language_stats, primary_language, Dialect, LanguageStat, LanguageTag,
};
use repository::read::BoundedReader;
use repository::walk::{walk, WalkOptions};
use review_core::language::Language;
use review_core::location::RepoPath;
use review_test_support::{fixture_repo, write_file};

fn detect(path: &str, prefix: &[u8]) -> Option<LanguageTag> {
    detect_language(&RepoPath::new(path).unwrap(), prefix)
}

fn tag(language: Language, dialect: Option<Dialect>) -> Option<LanguageTag> {
    Some(LanguageTag { language, dialect })
}

fn stats_for(root: &std::path::Path, analyzers: &[Language]) -> Vec<LanguageStat> {
    let (inv, _) = walk(root, &WalkOptions::default()).unwrap();
    let reader = BoundedReader::new(&inv.root);
    language_stats(&inv, &reader, analyzers).0
}

fn stat(stats: &[LanguageStat], language: Language) -> &LanguageStat {
    stats
        .iter()
        .find(|s| s.language == language)
        .unwrap_or_else(|| panic!("no stats for {language}"))
}

#[test]
fn dts_beats_ts_suffix() {
    assert_eq!(
        detect("types/global.d.ts", b""),
        tag(Language::Typescript, Some(Dialect::Dts))
    );
    assert_eq!(
        detect("src/a.d.mts", b""),
        tag(Language::Typescript, Some(Dialect::Dts))
    );
    assert_eq!(
        detect("src/a.ts", b""),
        tag(Language::Typescript, Some(Dialect::Ts))
    );
}

#[test]
fn tsx_and_jsx_dialects() {
    assert_eq!(
        detect("a.tsx", b""),
        tag(Language::Typescript, Some(Dialect::Tsx))
    );
    assert_eq!(
        detect("a.jsx", b""),
        tag(Language::Javascript, Some(Dialect::Jsx))
    );
}

#[test]
fn mjs_cjs_dialects() {
    assert_eq!(
        detect("a.mjs", b""),
        tag(Language::Javascript, Some(Dialect::Mjs))
    );
    assert_eq!(
        detect("a.cjs", b""),
        tag(Language::Javascript, Some(Dialect::Cjs))
    );
    assert_eq!(
        detect("a.mts", b""),
        tag(Language::Typescript, Some(Dialect::Ts))
    );
}

#[test]
fn shebang_node_and_tsnode_detected() {
    assert_eq!(
        detect("bin/tool", b"#!/usr/bin/env node\nconsole.log(1)\n"),
        tag(Language::Javascript, Some(Dialect::Js))
    );
    assert_eq!(
        detect("bin/tool", b"#!/usr/bin/env -S ts-node --esm\n"),
        tag(Language::Typescript, Some(Dialect::Ts))
    );
    assert_eq!(
        detect("bin/run", b"#!/usr/bin/python3.11\n"),
        tag(Language::Python, None)
    );
    assert_eq!(
        detect("bin/run", b"#!/bin/bash\nset -e\n"),
        tag(Language::Shell, None)
    );
    assert_eq!(
        detect("bin/run", b"#!/usr/bin/env ruby\n"),
        tag(Language::Ruby, None)
    );
    assert_eq!(detect("bin/run", b"no shebang here"), None);
    assert_eq!(detect("bin/run", b"#!/usr/bin/perl\n"), None);
}

#[test]
fn shebang_ignored_when_extension_present() {
    assert_eq!(
        detect("scripts/data.txt", b"#!/usr/bin/env node\n"),
        tag(Language::Other, None)
    );
    assert_eq!(
        detect("scripts/a.py", b"#!/usr/bin/env node\n"),
        tag(Language::Python, None)
    );
}

#[test]
fn dockerfile_variants() {
    for path in [
        "Dockerfile",
        "docker/Dockerfile.prod",
        "services/api.dockerfile",
        "Dockerfile.dev",
    ] {
        assert_eq!(detect(path, b""), tag(Language::Dockerfile, None), "{path}");
    }
    assert_eq!(detect("Makefile", b""), tag(Language::Other, None));
    assert_eq!(detect("Jenkinsfile", b""), tag(Language::Other, None));
}

#[test]
fn table_of_sixty_paths() {
    let cases: &[(&str, Language)] = &[
        ("a.ts", Language::Typescript),
        ("a.TS", Language::Typescript),
        ("src/a.service.ts", Language::Typescript),
        ("a.cts", Language::Typescript),
        ("a.js", Language::Javascript),
        ("a.min.js", Language::Javascript),
        ("a.py", Language::Python),
        ("stubs/a.pyi", Language::Python),
        ("main.go", Language::Go),
        ("lib.rs", Language::Rust),
        ("A.java", Language::Java),
        ("a.kt", Language::Kotlin),
        ("build.gradle.kts", Language::Kotlin),
        ("A.cs", Language::Csharp),
        ("a.rb", Language::Ruby),
        ("index.php", Language::Php),
        ("run.sh", Language::Shell),
        ("run.bash", Language::Shell),
        ("run.zsh", Language::Shell),
        ("q.sql", Language::Sql),
        ("c.yml", Language::Yaml),
        ("c.yaml", Language::Yaml),
        ("c.json", Language::Json),
        ("tsconfig.jsonc", Language::Json),
        ("c.json5", Language::Json),
        ("Cargo.toml", Language::Toml),
        ("README.md", Language::Markdown),
        ("doc.mdx", Language::Markdown),
        ("index.html", Language::Html),
        ("index.htm", Language::Html),
        ("a.css", Language::Css),
        ("a.scss", Language::Css),
        ("a.sass", Language::Css),
        ("a.less", Language::Css),
        ("main.tf", Language::Terraform),
        ("vars.tfvars", Language::Terraform),
        ("x.hcl", Language::Terraform),
        ("api.proto", Language::Protobuf),
        ("schema.graphql", Language::Graphql),
        ("q.gql", Language::Graphql),
        ("schema.prisma", Language::Prisma),
        ("Dockerfile", Language::Dockerfile),
        ("image.png", Language::Other),
        ("a.unknownext", Language::Other),
        ("deep/a/b/c/d/e.go", Language::Go),
        ("a.b.c.py", Language::Python),
        (".eslintrc.json", Language::Json),
        (".github/workflows/ci.yml", Language::Yaml),
        ("docker-compose.yaml", Language::Yaml),
        ("package.json", Language::Json),
        ("src/app.module.ts", Language::Typescript),
        ("src/app.e2e-spec.ts", Language::Typescript),
        ("types/index.d.ts", Language::Typescript),
        ("app/page.tsx", Language::Typescript),
        ("app/page.jsx", Language::Javascript),
        ("jest.config.mjs", Language::Javascript),
        ("jest.config.cjs", Language::Javascript),
        ("pom.xml", Language::Other),
        ("data.csv", Language::Other),
        ("migrations/001_init.sql", Language::Sql),
        ("Makefile", Language::Other),
    ];
    assert!(cases.len() >= 60, "{} cases", cases.len());
    for (path, expected) in cases {
        let got = detect(path, b"").map(|t| t.language);
        assert_eq!(got, Some(*expected), "{path}");
    }
}

fn polyglot(root: &std::path::Path) {
    write_file(
        root,
        "src/a.ts",
        "export const a = 1;\nexport const b = 2;\n",
    );
    write_file(root, "src/b.js", "module.exports = 1;\n");
    write_file(root, "py/app.py", "print(1)\nprint(2)\nprint(3)\n");
    write_file(root, "go/main.go", "package main\n");
    write_file(root, "rs/lib.rs", "fn main() {}\n");
    write_file(root, "java/A.java", "class A {}\n");
    write_file(root, "kt/B.kt", "class B\n");
    write_file(
        root,
        "README.md",
        "# readme\n\nlots of documentation text here to be big\n",
    );
    write_file(root, "data.json", "{}\n");
}

#[test]
fn stats_sorted_and_summed() {
    let tmp = tempfile::tempdir().unwrap();
    polyglot(tmp.path());
    let stats = stats_for(tmp.path(), &[Language::Typescript, Language::Javascript]);
    for language in [
        Language::Typescript,
        Language::Python,
        Language::Go,
        Language::Rust,
        Language::Java,
        Language::Kotlin,
    ] {
        assert_eq!(stat(&stats, language).files, 1, "{language}");
    }
    let ts = stat(&stats, Language::Typescript);
    assert_eq!(ts.lines, 2);
    assert_eq!(ts.bytes, 40);
    // sorted by bytes desc, then language name
    for pair in stats.windows(2) {
        assert!(
            pair[0].bytes > pair[1].bytes
                || (pair[0].bytes == pair[1].bytes
                    && pair[0].language.as_str() <= pair[1].language.as_str()),
            "{pair:?}"
        );
    }
}

#[test]
fn analyzable_flag_follows_registered_analyzers() {
    let tmp = tempfile::tempdir().unwrap();
    polyglot(tmp.path());
    let stats = stats_for(tmp.path(), &[Language::Typescript, Language::Javascript]);
    for s in &stats {
        let expected = matches!(s.language, Language::Typescript | Language::Javascript);
        assert_eq!(s.analyzable, expected, "{}", s.language);
    }
}

#[test]
fn too_large_counts_bytes_not_lines() {
    let tmp = tempfile::tempdir().unwrap();
    write_file(
        tmp.path(),
        "big.ts",
        "a
b
c
d
e
f
",
    );
    let opts = WalkOptions {
        max_analyze_bytes: 4,
        ..WalkOptions::default()
    };
    let (inv, _) = walk(tmp.path(), &opts).unwrap();
    let reader = BoundedReader::new(&inv.root);
    let (stats, _) = language_stats(&inv, &reader, &[]);
    let ts = stat(&stats, Language::Typescript);
    assert_eq!(ts.bytes, 12);
    assert_eq!(ts.lines, 0);
    assert_eq!(ts.files, 1);
}

#[test]
fn primary_language_excludes_data_formats() {
    let tmp = tempfile::tempdir().unwrap();
    write_file(tmp.path(), "big.json", "x".repeat(10_000));
    write_file(tmp.path(), "big.md", "y".repeat(10_000));
    write_file(tmp.path(), "src/a.ts", "export const a = 1;\n");
    let stats = stats_for(tmp.path(), &[Language::Typescript]);
    assert_eq!(primary_language(&stats), Some(Language::Typescript));
    assert!(matches!(
        stats[0].language,
        Language::Json | Language::Markdown
    ));
}

#[test]
fn primary_language_ties_break_by_enum_order() {
    let tmp = tempfile::tempdir().unwrap();
    write_file(tmp.path(), "a.py", "12345");
    write_file(tmp.path(), "b.ts", "12345");
    let stats = stats_for(tmp.path(), &[]);
    assert_eq!(primary_language(&stats), Some(Language::Typescript));
}

#[test]
fn init_basic_is_typescript() {
    let repo = fixture_repo("init-basic");
    let stats = stats_for(&repo, &[Language::Typescript, Language::Javascript]);
    assert_eq!(primary_language(&stats), Some(Language::Typescript));
    assert!(stat(&stats, Language::Typescript).analyzable);
}

#[test]
fn extensionless_shebang_files_are_counted_by_language() {
    let tmp = tempfile::tempdir().unwrap();
    write_file(
        tmp.path(),
        "bin/tool",
        "#!/usr/bin/env node\nconsole.log(1);\n",
    );
    let stats = stats_for(tmp.path(), &[Language::Javascript]);
    assert_eq!(stat(&stats, Language::Javascript).files, 1);
}
