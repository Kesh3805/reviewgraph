#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use analysis_ir::{
    AnalyzerConfig, FrameworkFactKind, LanguageAnalyzer, ParseStatus, ParsedUnit, SourceInput,
};
use lang_python::PythonAnalyzer;
use review_core::location::{ContentHash, RepoPath};
use review_core::symbol::{ModulePath, SymbolKind};

fn analyze(path: &str, source: &str) -> ParsedUnit {
    let path = RepoPath::new(path).unwrap();
    let input = SourceInput {
        module_path: ModulePath::of(&path),
        content_hash: ContentHash::of(source.as_bytes()),
        path,
        bytes: source.as_bytes(),
        is_generated: false,
    };
    let unit = PythonAnalyzer::new()
        .analyze(&input, &AnalyzerConfig::default())
        .unwrap();
    analysis_ir::validate(&unit).unwrap();
    unit
}

#[test]
fn golden_ir_class_with_decorators() {
    let unit = analyze(
        "app/models.py",
        "import dataclasses\n\n@dataclasses.dataclass\nclass User(Base):\n    name: str = 'x'\n\n    def greet(self, other):\n        return other\n",
    );
    let names: Vec<String> = unit
        .symbols
        .iter()
        .skip(1)
        .map(|s| format!("{} {}", s.kind.as_id_str(), s.qualified_name.join(".")))
        .collect();
    assert_eq!(
        names,
        vec!["class User", "field User.name", "method User.greet"]
    );
    assert_eq!(unit.symbols[1].decorators[0].name, "dataclasses.dataclass");
    assert_eq!(unit.symbols[1].heritage.extends, vec!["Base"]);
    assert_eq!(unit.symbols[3].kind, SymbolKind::Method);
}

#[test]
fn syntax_error_tolerated_partial_symbols() {
    let unit = analyze("a.py", "def ok():\n    return 1\n\ndef broken(:\n    pass\n");
    assert!(matches!(unit.status, ParseStatus::Partial { .. }));
    assert!(unit.symbols.iter().any(|s| s.name == "ok"));
}

#[test]
fn fastapi_route_extracted_as_http_route() {
    let unit = analyze(
        "api.py",
        "from fastapi import FastAPI\napp = FastAPI()\n\n@app.get('/users')\ndef list_users():\n    return []\n",
    );
    assert!(unit
        .framework
        .iter()
        .any(|f| f.kind == FrameworkFactKind::HttpRoute));
}

#[test]
fn dynamic_getattr_yields_low_confidence_edge() {
    let unit = analyze("d.py", "def f(o):\n    return getattr(o, 'x')\n");
    assert!(unit
        .references
        .iter()
        .any(|r| r.name == "getattr" && r.attrs.contains_key("name_ambiguous")));
}
