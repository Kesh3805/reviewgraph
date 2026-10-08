//! SID-001: the canonical `SymbolId` grammar, its parser and the `SymbolKey` golden vectors.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::path::Path;

use proptest::prelude::*;
use review_core::language::Language;
use review_core::location::RepoPath;
use review_core::symbol::{ModulePath, SymbolKind};
use review_core::symbol_id::{
    module_path_collisions, module_path_for, module_paths, ModulePathCollision, SymbolIdError,
    SymbolIdParts, MAX_ID_BYTES, MAX_MODULE_PATH_BYTES, MAX_SEGMENT_BYTES, RESERVED_LANG_PREFIXES,
};

const GOLDEN_VECTORS: &str = "tests/data/symbol_key_vectors.json";

fn path(raw: &str) -> RepoPath {
    RepoPath::new(raw).unwrap()
}

fn parts(module: &str, qualified_name: &[&str], kind: SymbolKind) -> SymbolIdParts {
    SymbolIdParts::new(
        "ts",
        ModulePath::of(&path(module)),
        qualified_name.iter().map(|s| (*s).to_owned()).collect(),
        kind,
    )
}

fn format(module: &str, qualified_name: &[&str], kind: SymbolKind) -> String {
    parts(module, qualified_name, kind)
        .format()
        .unwrap()
        .as_str()
        .to_owned()
}

#[test]
fn example_from_adr_005_formats_exactly() {
    assert_eq!(
        format(
            "src/auth/auth.service.ts",
            &["AuthService", "authorize"],
            SymbolKind::Method
        ),
        "ts:src/auth/auth.service#AuthService.authorize/method"
    );
    assert_eq!(
        format("src/a.ts", &["handler"], SymbolKind::Function),
        "ts:src/a#handler/function"
    );
    // The language tag comes from the analyzed unit, so a JavaScript file yields `js:`, never
    // `ts:`, even when it shares a module path with a TypeScript file.
    let js = SymbolIdParts::new(
        Language::Javascript.id_prefix(),
        ModulePath::of(&path("src/a.js")),
        vec!["handler".to_owned()],
        SymbolKind::Function,
    )
    .format()
    .unwrap();
    assert_eq!(js.as_str(), "js:src/a#handler/function");
}

#[test]
fn module_symbol_id() {
    let module = SymbolIdParts::module("ts", ModulePath::of(&path("src/auth/auth.service.ts")))
        .format()
        .unwrap();
    assert_eq!(
        module.as_str(),
        "ts:src/auth/auth.service#__module__/module"
    );
    let unit_symbol = SymbolIdParts::new(
        "ts",
        module_path_for(&path("src/auth/auth.service.ts"), Language::Typescript),
        vec!["__module__".to_owned()],
        SymbolKind::Module,
    )
    .format()
    .unwrap();
    assert_eq!(unit_symbol, module);
}

#[test]
fn escape_special_chars_roundtrip() {
    let cases = [
        ("a.b", "ts:src/a#a%2Eb/function"),
        ("x#y", "ts:src/a#x%23y/function"),
        ("p/q", "ts:src/a#p%2Fq/function"),
        ("t~1", "ts:src/a#t%7E1/function"),
        ("100%", "ts:src/a#100%25/function"),
        ("a b", "ts:src/a#a%20b/function"),
        ("üñí", "ts:src/a#üñí/function"),
        ("#x", "ts:src/a#%23x/method"),
        ("<anonymous>", "ts:src/a#<anonymous>/function"),
    ];
    for (raw, canonical) in cases {
        let id = review_core::ids::SymbolId::parse(canonical)
            .unwrap_or_else(|e| panic!("{canonical:?} must parse: {e}"));
        assert_eq!(id.as_str(), canonical);
        let decoded = id.parts().unwrap();
        assert_eq!(decoded.qualified_name, vec![raw.to_owned()]);
        // Formatting the decoded parts reproduces the same canonical string.
        assert_eq!(decoded.format().unwrap(), id);
    }
    assert_eq!(
        format("src/a b.ts", &["f"], SymbolKind::Function),
        "ts:src/a%20b#f/function",
        "spaces in the module path are escaped too"
    );
    assert_eq!(
        format("src/a#b.ts", &["f"], SymbolKind::Function),
        "ts:src/a%23b#f/function",
        "a `#` in a file name cannot break the structure of the id"
    );
}

#[test]
fn parse_rejects_noncanonical_escape() {
    for bad in [
        "ts:src/a#a%2fb/function",
        "ts:src/a#a%41/function",
        "ts:src/a#a b/function",
        "ts:src/a%2Fb#f/function",
    ] {
        assert!(
            matches!(
                review_core::ids::SymbolId::parse(bad),
                Err(SymbolIdError::NonCanonical(_))
            ),
            "{bad} is not canonical"
        );
    }
}

#[test]
fn parse_rejects_unknown_kind_and_lang() {
    assert!(matches!(
        review_core::ids::SymbolId::parse("ts:src/a#m/symbol"),
        Err(SymbolIdError::BadKind(_))
    ));
    assert!(matches!(
        review_core::ids::SymbolId::parse("ts:src/a#m/Method"),
        Err(SymbolIdError::BadKind(_))
    ));
    assert!(matches!(
        review_core::ids::SymbolId::parse("TS:src/a#m/function"),
        Err(SymbolIdError::BadLang(_))
    ));
    assert!(matches!(
        review_core::ids::SymbolId::parse("1ts:src/a#m/function"),
        Err(SymbolIdError::BadLang(_))
    ));
    assert!(matches!(
        review_core::ids::SymbolId::parse("t s:src/a#m/function"),
        Err(SymbolIdError::BadLang(_))
    ));
    let too_long_lang = format!("{}:src/a#m/function", "a".repeat(17));
    assert!(matches!(
        review_core::ids::SymbolId::parse(&too_long_lang),
        Err(SymbolIdError::TooLong { .. })
    ));
}

#[test]
fn parse_splits_on_first_hash_last_slash() {
    let parsed = review_core::ids::SymbolId::parse("ts:src/a#b%2Fc.d/function")
        .unwrap()
        .parts()
        .unwrap();
    assert_eq!(parsed.module_path.as_str(), "src/a");
    assert_eq!(
        parsed.qualified_name,
        vec!["b/c".to_owned(), "d".to_owned()]
    );
    assert_eq!(parsed.kind, SymbolKind::Function);
    // A raw `/` inside a name would move the kind separator.
    assert!(review_core::ids::SymbolId::parse("ts:src/a#b/c/function").is_err());
    // A second `#` is a raw structural byte inside a name segment.
    assert!(review_core::ids::SymbolId::parse("ts:src/a#b#c/function").is_err());
}

#[test]
fn ordinal_format_and_leading_zero_rejected() {
    let with_ordinal = parts("src/a.ts", &["f"], SymbolKind::Function).with_ordinal(1);
    assert_eq!(
        with_ordinal.format().unwrap().as_str(),
        "ts:src/a#f/function~1"
    );
    let big = parts("src/a.ts", &["f"], SymbolKind::Function).with_ordinal(u16::MAX);
    assert_eq!(
        big.format().unwrap().as_str(),
        format!("ts:src/a#f/function~{}", u16::MAX)
    );
    for bad in [
        "ts:src/a#f/function~0",
        "ts:src/a#f/function~01",
        "ts:src/a#f/function~",
        "ts:src/a#f/function~x",
        "ts:src/a#f/function~-1",
        "ts:src/a#f/function~65536",
    ] {
        assert!(
            matches!(
                review_core::ids::SymbolId::parse(bad),
                Err(SymbolIdError::BadOrdinal(_)) | Err(SymbolIdError::TooLong { .. })
            ),
            "{bad} must be rejected"
        );
    }
}

#[test]
fn reserved_prefix_rejected_as_lang() {
    for prefix in RESERVED_LANG_PREFIXES {
        let raw = format!("{prefix}:src/a#m/function");
        assert!(
            matches!(
                review_core::ids::SymbolId::parse(&raw),
                Err(SymbolIdError::ReservedLang(_))
            ),
            "{prefix} is reserved for graph node ids"
        );
    }
    for lang in ["ts", "js", "py", "go", "rs", "java"] {
        let raw = format!("{lang}:src/a#m/function");
        assert!(review_core::ids::SymbolId::parse(&raw).is_ok(), "{lang}");
    }
}

#[test]
fn module_path_strips_extensions_and_dts() {
    let cases = [
        ("src/a.ts", "src/a"),
        ("src/a.tsx", "src/a"),
        ("src/a.mts", "src/a"),
        ("src/a.cts", "src/a"),
        ("src/a.d.ts", "src/a.d"),
        ("src/a.service.ts", "src/a.service"),
    ];
    for (raw, expected) in cases {
        assert_eq!(
            module_path_for(&path(raw), Language::Typescript).as_str(),
            expected,
            "{raw} as typescript"
        );
    }
    for (raw, expected) in [
        ("src/a.js", "src/a"),
        ("src/a.jsx", "src/a"),
        ("src/a.mjs", "src/a"),
        ("src/a.cjs", "src/a"),
        ("src/a.ts", "src/a.ts"),
    ] {
        assert_eq!(
            module_path_for(&path(raw), Language::Javascript).as_str(),
            expected,
            "{raw} as javascript"
        );
    }
    assert_ne!(
        module_path_for(&path("src/a.ts"), Language::Typescript),
        module_path_for(&path("src/a.d.ts"), Language::Typescript),
        "an implementation and its declaration file never share an id"
    );
}

#[test]
fn module_path_collision_tsx() {
    let ts = path("src/a.ts");
    let tsx = path("src/a.tsx");
    let dts = path("src/a.d.ts");
    let entries = [
        (Language::Typescript, ts.clone()),
        (Language::Typescript, tsx.clone()),
        (Language::Typescript, dts.clone()),
    ];
    let resolved = module_paths(&entries);
    assert_eq!(resolved[&ts].as_str(), "src/a");
    assert_eq!(
        resolved[&tsx].as_str(),
        "src/a.tsx",
        "the later path in byte order keeps its extension"
    );
    assert_eq!(resolved[&dts].as_str(), "src/a.d");
    let collisions: Vec<ModulePathCollision> = module_path_collisions(&entries);
    assert_eq!(collisions.len(), 1);
    assert_eq!(collisions[0].language, Language::Typescript);
    assert_eq!(collisions[0].module_path.as_str(), "src/a");
    assert_eq!(collisions[0].paths, vec![ts, tsx]);
    // Different languages never collide, because the tag is part of the id.
    let mixed = [
        (Language::Typescript, path("src/a.ts")),
        (Language::Python, path("src/a.py")),
    ];
    assert!(module_path_collisions(&mixed).is_empty());
    // Ids built from the resolved map are unique.
    let mut ids: BTreeMap<String, ()> = BTreeMap::new();
    for ((_, p), module) in mixed.iter().zip(module_paths(&mixed).values()) {
        let lang = if p.extension().is_some_and(|e| e == "py") {
            Language::Python
        } else {
            Language::Typescript
        };
        let id = SymbolIdParts::new(
            lang.id_prefix(),
            module.clone(),
            vec!["f".to_owned()],
            SymbolKind::Function,
        )
        .format()
        .unwrap();
        assert!(ids.insert(id.as_str().to_owned(), ()).is_none());
    }
}

#[test]
fn backslash_paths_normalized_or_rejected() {
    assert!(
        RepoPath::new("src\\a.ts").is_err(),
        "the repository path type rejects backslashes"
    );
    assert!(
        review_core::ids::SymbolId::parse("ts:src\\a#f/function").is_err(),
        "a Windows-style module path can never produce an id"
    );
    assert!(
        parts("src/a.ts", &["a\\b"], SymbolKind::Function)
            .format()
            .is_ok(),
        "a backslash inside a name is ordinary text"
    );
    let id = parts("src/a.ts", &["a\\b"], SymbolKind::Function)
        .format()
        .unwrap();
    assert_eq!(id.parts().unwrap().qualified_name, vec!["a\\b".to_owned()]);
}

#[test]
fn nfc_normalization_applied() {
    let decomposed = "Cafe\u{301}";
    let precomposed = "Caf\u{e9}";
    let a = parts("src/a.ts", &[decomposed], SymbolKind::Function)
        .format()
        .unwrap();
    let b = parts("src/a.ts", &[precomposed], SymbolKind::Function)
        .format()
        .unwrap();
    assert_eq!(a, b, "equivalent spellings share one id");
    assert_eq!(a.as_str(), "ts:src/a#Caf\u{e9}/function");
    let module = SymbolIdParts::new(
        "ts",
        module_path_for(&path("src/Cafe\u{301}/f.ts"), Language::Typescript),
        vec!["f".to_owned()],
        SymbolKind::Function,
    );
    assert_eq!(
        module.format().unwrap().as_str(),
        "ts:src/Caf\u{e9}/f#f/function"
    );
}

#[test]
fn too_long_rejected() {
    let long_segment = "x".repeat(MAX_SEGMENT_BYTES + 1);
    assert!(matches!(
        parts("src/a.ts", &[&long_segment], SymbolKind::Function).format(),
        Err(SymbolIdError::TooLong {
            field: "segment",
            ..
        })
    ));
    let long_module = "a".repeat(MAX_MODULE_PATH_BYTES + 1);
    let huge_module = SymbolIdParts::new(
        "ts",
        module_path_for(
            &path(&format!("src/{long_module}.ts")),
            Language::Typescript,
        ),
        vec!["f".to_owned()],
        SymbolKind::Function,
    );
    assert!(matches!(
        huge_module.format(),
        Err(SymbolIdError::TooLong { .. })
    ));
    let deep: Vec<String> = (0..40)
        .map(|i| format!("segment-{i}-{}", "y".repeat(40)))
        .collect();
    let refs: Vec<&str> = deep.iter().map(String::as_str).collect();
    let deep_id = parts("src/a.ts", &refs, SymbolKind::Function).format();
    assert!(matches!(
        deep_id,
        Err(SymbolIdError::TooLong {
            field: "symbol id",
            ..
        })
    ));
    // Nothing is ever silently truncated: the lossy form is explicit and deterministic.
    let lossy = parts("src/a.ts", &refs, SymbolKind::Function).format_lossy();
    assert!(lossy.as_str().len() <= MAX_ID_BYTES);
    assert_eq!(
        lossy,
        parts("src/a.ts", &refs, SymbolKind::Function).format_lossy(),
        "the degradation is deterministic"
    );
    assert!(review_core::ids::SymbolId::parse(lossy.as_str()).is_ok());
}

#[test]
fn symbol_key_matches_blake3_prefix_golden_vectors() {
    let raw = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(GOLDEN_VECTORS))
        .unwrap_or_else(|e| panic!("read {GOLDEN_VECTORS}: {e}"));
    let document: serde_json::Value = serde_json::from_str(&raw).unwrap();
    let vectors = document["vectors"].as_array().expect("vectors array");
    assert!(
        !vectors.is_empty(),
        "the golden vector file must not be empty"
    );
    for vector in vectors {
        let id = vector["id"].as_str().expect("id");
        let key = vector["key"].as_str().expect("key");
        let parsed = review_core::ids::SymbolId::parse(id)
            .unwrap_or_else(|e| panic!("golden id {id:?} must be canonical: {e}"));
        assert_eq!(
            review_core::ids::SymbolKey::of(&parsed).to_string(),
            key,
            "the stored key of {id} changed"
        );
        assert_eq!(
            key.parse::<review_core::ids::SymbolKey>()
                .unwrap()
                .to_string(),
            key
        );
        // The key is the first 16 bytes of the BLAKE3 digest of the canonical string.
        let digest = blake3::hash(id.as_bytes());
        assert_eq!(hex::encode(&digest.as_bytes()[..16]), key);
    }
}

#[test]
fn symbol_key_hex_is_32_lowercase() {
    for (module, qualified, kind) in [
        ("src/a.ts", vec!["A", "b"], SymbolKind::Method),
        ("src/b b.ts", vec!["100%"], SymbolKind::Function),
        ("src/üñí.ts", vec!["üñí"], SymbolKind::Constant),
    ] {
        let id = parts(module, &qualified, kind).format().unwrap();
        let key = review_core::ids::SymbolKey::of(&id);
        let shown = key.to_string();
        assert_eq!(shown.len(), 32);
        assert!(shown
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')));
        assert_eq!(shown.parse::<review_core::ids::SymbolKey>().unwrap(), key);
        let json = serde_json::to_string(&key).unwrap();
        assert_eq!(json, format!("\"{shown}\""));
        assert_eq!(
            serde_json::from_str::<review_core::ids::SymbolKey>(&json).unwrap(),
            key
        );
    }
}

fn language() -> impl Strategy<Value = String> {
    "[a-z][a-z0-9_]{0,10}".prop_filter("not reserved", |s| {
        !RESERVED_LANG_PREFIXES.contains(&s.as_str())
    })
}

fn plain_segment() -> impl Strategy<Value = String> {
    "[a-zA-Z0-9_]{1,12}"
}

fn special_segment() -> impl Strategy<Value = String> {
    prop_oneof![
        "[ %#/~.\\-]{1,6}",
        "[a-z]{0,4}[.][a-z]{0,4}",
        "[a-z]{0,3}[~][a-z]{0,3}",
        "üñí",
    ]
}

proptest! {
    #[test]
    fn parse_format_roundtrip_property(
        lang in language(),
        dir in prop::collection::vec(plain_segment(), 1..3),
        file in plain_segment(),
        qualified in prop::collection::vec(prop_oneof![plain_segment(), special_segment()], 1..4),
        kind in prop::sample::select(SymbolKind::ALL.to_vec()),
        ordinal in prop::option::of(1u16..),
    ) {
        let module = format!("{}/{}.ts", dir.join("/"), file);
        let module_path = ModulePath::of(&path(&module));
        let qualified: Vec<String> = qualified;
        let p = SymbolIdParts::new(lang, module_path.clone(), qualified.clone(), kind);
        let p = match ordinal { Some(n) => p.with_ordinal(n), None => p };
        let id = p.format().unwrap();

        let back = review_core::ids::SymbolId::parse(id.as_str()).unwrap().parts().unwrap();
        prop_assert_eq!(back.qualified_name.clone(), qualified);
        prop_assert_eq!(back.kind, kind);
        prop_assert_eq!(back.ordinal, ordinal.unwrap_or(0));
        prop_assert_eq!(&back.module_path, &module_path);
        prop_assert_eq!(back.format().unwrap(), id.clone());
        prop_assert_eq!(review_core::ids::SymbolId::from_parts(&back).unwrap(), id.clone());
        prop_assert_eq!(id.as_str().parse::<review_core::ids::SymbolId>().unwrap(), id.clone());
        prop_assert_eq!(id.parts().unwrap(), back);
        prop_assert_eq!(
            review_core::ids::SymbolKey::of(&id).to_string().len(),
            32
        );
    }
}
