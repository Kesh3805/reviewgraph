//! `package.json` parsing. Lenient: field-by-field extraction from a `serde_json::Value`.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use regex::Regex;
use review_core::location::RepoPath;
use serde_json::Value;

use super::{
    DepKind, DepSpec, Ecosystem, ManifestFact, ModuleType, NpmManifestExtras, ParseStatusLite,
};
use crate::error::InitWarning;

const MAX_EXPORTS_BYTES: usize = 64 * 1024;

fn hint_regexes() -> &'static Option<(Regex, Regex)> {
    static RE: OnceLock<Option<(Regex, Regex)>> = OnceLock::new();
    RE.get_or_init(|| {
        Some((
            Regex::new(r"^(\./)?[\w@./-]+\.(m?[jt]s|c[jt]s)$").ok()?,
            Regex::new(r"^(dist|build|src|lib)/[\w./-]+$").ok()?,
        ))
    })
}

/// Path-like tokens of a script value. Everything else (flags, env assignments, URLs, secrets)
/// is dropped.
pub(crate) fn script_entry_hints(script: &str) -> Vec<String> {
    let Some((file_re, dir_re)) = hint_regexes() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for token in script.split_whitespace() {
        let token = token.trim_matches(|c| c == '"' || c == '\'');
        if token.starts_with('-') || token.contains('=') {
            continue;
        }
        if (file_re.is_match(token) || dir_re.is_match(token)) && !out.iter().any(|t| t == token) {
            out.push(token.to_owned());
        }
    }
    out
}

pub(crate) fn parse(path: &RepoPath, text: &str) -> (ManifestFact, Vec<InitWarning>) {
    let mut warnings = Vec::new();
    let value: Value = match crate::jsonc::parse_jsonc(text.as_bytes()) {
        Ok(v) => v,
        Err(e) => {
            warnings.push(InitWarning::new(
                "manifest_parse",
                Some(path.clone()),
                format!("line {}, column {}", e.line, e.column),
            ));
            return (
                ManifestFact::failed(path.clone(), Ecosystem::Npm, "invalid JSON"),
                warnings,
            );
        }
    };
    let Some(root) = value.as_object() else {
        warnings.push(InitWarning::new(
            "manifest_parse",
            Some(path.clone()),
            "package.json is not a JSON object",
        ));
        return (
            ManifestFact::failed(path.clone(), Ecosystem::Npm, "not an object"),
            warnings,
        );
    };

    let mut fact = ManifestFact::new(path.clone(), Ecosystem::Npm);
    let mut partial = false;
    let mut type_error = |field: &str, warnings: &mut Vec<InitWarning>| {
        partial = true;
        warnings.push(InitWarning::new(
            "manifest_field_type",
            Some(path.clone()),
            format!("field `{field}` has an unexpected type"),
        ));
    };

    let string_field = |key: &str,
                        warnings: &mut Vec<InitWarning>,
                        on_err: &mut dyn FnMut(&str, &mut Vec<InitWarning>)|
     -> Option<String> {
        match root.get(key) {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) => Some(s.clone()),
            Some(_) => {
                on_err(key, warnings);
                None
            }
        }
    };

    fact.name = string_field("name", &mut warnings, &mut type_error);
    fact.version = string_field("version", &mut warnings, &mut type_error);
    fact.private = match root.get("private") {
        Some(Value::Bool(b)) => *b,
        None | Some(Value::Null) => false,
        Some(_) => {
            type_error("private", &mut warnings);
            false
        }
    };

    // Dependencies. Earlier groups win when a name appears in several.
    for (key, kind) in [
        ("dependencies", DepKind::Prod),
        ("optionalDependencies", DepKind::Optional),
        ("peerDependencies", DepKind::Peer),
        ("devDependencies", DepKind::Dev),
    ] {
        match root.get(key) {
            None | Some(Value::Null) => {}
            Some(Value::Object(map)) => {
                for (name, range) in map {
                    let Some(range) = range.as_str() else {
                        type_error(key, &mut warnings);
                        continue;
                    };
                    fact.dependencies.entry(name.clone()).or_insert(DepSpec {
                        range: range.to_owned(),
                        kind,
                        workspace_protocol: range.starts_with("workspace:"),
                    });
                }
            }
            Some(_) => type_error(key, &mut warnings),
        }
    }

    let mut extras = NpmManifestExtras {
        module_type: match root.get("type").and_then(Value::as_str) {
            Some("module") => Some(ModuleType::Module),
            Some("commonjs") => Some(ModuleType::Commonjs),
            _ => None,
        },
        main: string_field("main", &mut warnings, &mut type_error),
        module: string_field("module", &mut warnings, &mut type_error),
        types: string_field("types", &mut warnings, &mut type_error)
            .or_else(|| string_field("typings", &mut warnings, &mut type_error)),
        package_manager: string_field("packageManager", &mut warnings, &mut type_error),
        has_jest_key: root.contains_key("jest"),
        ..NpmManifestExtras::default()
    };

    match root.get("bin") {
        None | Some(Value::Null) => {}
        Some(Value::String(s)) => {
            let name = fact
                .name
                .as_deref()
                .map(|n| n.rsplit('/').next().unwrap_or(n).to_owned())
                .unwrap_or_else(|| "bin".to_owned());
            extras.bin.insert(name, s.clone());
        }
        Some(Value::Object(map)) => {
            for (k, v) in map {
                if let Some(v) = v.as_str() {
                    extras.bin.insert(k.clone(), v.to_owned());
                }
            }
        }
        Some(_) => type_error("bin", &mut warnings),
    }

    if let Some(exports) = root.get("exports") {
        let size = serde_json::to_string(exports).map(|s| s.len()).unwrap_or(0);
        if size <= MAX_EXPORTS_BYTES {
            extras.exports = Some(exports.clone());
        } else {
            warnings.push(InitWarning::new(
                "list_truncated",
                Some(path.clone()),
                "package.json `exports` is larger than 64 KiB and was not stored",
            ));
        }
    }

    match root.get("workspaces") {
        None | Some(Value::Null) => {}
        Some(Value::Array(items)) => {
            extras.workspaces = Some(
                items
                    .iter()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect(),
            );
        }
        Some(Value::Object(obj)) => match obj.get("packages") {
            Some(Value::Array(items)) => {
                extras.workspaces = Some(
                    items
                        .iter()
                        .filter_map(|v| v.as_str().map(str::to_owned))
                        .collect(),
                );
            }
            _ => extras.workspaces = Some(Vec::new()),
        },
        Some(_) => type_error("workspaces", &mut warnings),
    }

    match root.get("scripts") {
        None | Some(Value::Null) => {}
        Some(Value::Object(scripts)) => {
            let mut hints: BTreeMap<String, Vec<String>> = BTreeMap::new();
            for (name, command) in scripts {
                extras.script_names.push(name.clone());
                if let Some(command) = command.as_str() {
                    let found = script_entry_hints(command);
                    if !found.is_empty() {
                        hints.insert(name.clone(), found);
                    }
                }
            }
            extras.script_names.sort();
            extras.script_entry_hints = hints;
        }
        Some(_) => type_error("scripts", &mut warnings),
    }

    match root.get("engines") {
        None | Some(Value::Null) => {}
        Some(Value::Object(map)) => {
            for (k, v) in map {
                if let Some(v) = v.as_str() {
                    extras.engines.insert(k.clone(), v.to_owned());
                }
            }
        }
        Some(_) => type_error("engines", &mut warnings),
    }

    fact.npm = Some(extras);
    if partial {
        fact.parse_status = ParseStatusLite::Partial;
    }
    (fact, warnings)
}
