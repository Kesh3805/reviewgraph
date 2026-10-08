//! Safe YAML loading, unknown-key detection and semantic validation (POL-001).
//!
//! Validation never fails hard: every problem becomes a [`ConfigIssue`] with the dotted key
//! path it concerns, and the caller decides what an error means (POL-001 falls back to the
//! defaults and applies no suppressions).

use std::collections::BTreeSet;

use chrono::NaiveDate;
use globset::Glob;
use schemars::schema::{RootSchema, Schema, SchemaObject, SingleOrVec};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::schema::{
    ReviewConfigV1, SuppressionKind, CONFIG_SCHEMA_VERSION, INFERABLE_LAYER_NAMES,
    MAINTAINABILITY_FLOOR, MINIMUM_PUBLISH_FLOOR, REVIEWER_NAMES,
};

/// Largest accepted config file.
pub const MAX_CONFIG_BYTES: usize = 256 * 1024;
/// Deepest accepted nesting of the parsed document (after alias expansion).
pub const MAX_CONFIG_DEPTH: usize = 32;
/// Most nodes the parsed document may expand to (alias bombs blow up here first).
pub const MAX_CONFIG_NODES: usize = 100_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueSeverity {
    Warning,
    Error,
}

/// Closed set of problem kinds; the label of `config_validation_errors_total{kind}`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigIssueKind {
    TooLarge,
    Parse,
    YamlBomb,
    UnknownKey,
    InvalidValue,
    UnsupportedVersion,
    OutOfRange,
    InvalidGlob,
    PathEscape,
    UndefinedLayer,
    DuplicateId,
    MissingReason,
    ExpiredSuppression,
}

impl ConfigIssueKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::TooLarge => "too_large",
            Self::Parse => "parse",
            Self::YamlBomb => "yaml_bomb",
            Self::UnknownKey => "unknown_key",
            Self::InvalidValue => "invalid_value",
            Self::UnsupportedVersion => "unsupported_version",
            Self::OutOfRange => "out_of_range",
            Self::InvalidGlob => "invalid_glob",
            Self::PathEscape => "path_escape",
            Self::UndefinedLayer => "undefined_layer",
            Self::DuplicateId => "duplicate_id",
            Self::MissingReason => "missing_reason",
            Self::ExpiredSuppression => "expired_suppression",
        }
    }
}

/// One validation problem, located by its dotted key path (`rules.queue_job`,
/// `suppressions[0].expires`; empty for the whole document).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigIssue {
    pub severity: IssueSeverity,
    pub kind: ConfigIssueKind,
    pub path: String,
    pub message: String,
}

impl ConfigIssue {
    pub fn error(
        kind: ConfigIssueKind,
        path: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            severity: IssueSeverity::Error,
            kind,
            path: path.into(),
            message: message.into(),
        }
    }

    pub fn warning(
        kind: ConfigIssueKind,
        path: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            severity: IssueSeverity::Warning,
            kind,
            path: path.into(),
            message: message.into(),
        }
    }

    pub fn is_error(&self) -> bool {
        self.severity == IssueSeverity::Error
    }

    /// One doctor line: `error rules.queue_job: unknown key ...`.
    pub fn render(&self) -> String {
        let level = match self.severity {
            IssueSeverity::Error => "error",
            IssueSeverity::Warning => "warning",
        };
        if self.path.is_empty() {
            format!("{level}: {}", self.message)
        } else {
            format!("{level} {}: {}", self.path, self.message)
        }
    }
}

/// Parses YAML into a JSON value under the size, depth and node caps.
pub fn parse_yaml_safely(raw: &[u8]) -> Result<Value, ConfigIssue> {
    if raw.len() > MAX_CONFIG_BYTES {
        return Err(ConfigIssue::error(
            ConfigIssueKind::TooLarge,
            "",
            format!(
                "config is {} bytes; the limit is {MAX_CONFIG_BYTES}",
                raw.len()
            ),
        ));
    }
    let text = std::str::from_utf8(raw)
        .map_err(|_| ConfigIssue::error(ConfigIssueKind::Parse, "", "config is not UTF-8"))?;
    let yaml: serde_yaml::Value = serde_yaml::from_str(text).map_err(|e| {
        let message = e.to_string();
        let lowered = message.to_lowercase();
        let kind = if lowered.contains("repetition limit") || lowered.contains("recursion limit") {
            ConfigIssueKind::YamlBomb
        } else {
            ConfigIssueKind::Parse
        };
        ConfigIssue::error(kind, "", format!("YAML: {message}"))
    })?;
    let mut nodes = 0usize;
    check_shape(&yaml, 1, &mut nodes)?;
    let json = serde_json::to_value(&yaml).map_err(|e| {
        ConfigIssue::error(
            ConfigIssueKind::Parse,
            "",
            format!("YAML is not representable as JSON: {e}"),
        )
    })?;
    if json.is_null() {
        // An empty file is an empty mapping.
        return Ok(Value::Object(serde_json::Map::new()));
    }
    Ok(json)
}

fn check_shape(
    value: &serde_yaml::Value,
    depth: usize,
    nodes: &mut usize,
) -> Result<(), ConfigIssue> {
    *nodes += 1;
    if *nodes > MAX_CONFIG_NODES || depth > MAX_CONFIG_DEPTH {
        return Err(ConfigIssue::error(
            ConfigIssueKind::YamlBomb,
            "",
            format!(
                "config expands beyond {MAX_CONFIG_NODES} nodes or {MAX_CONFIG_DEPTH} levels \
                 (alias expansion is capped)"
            ),
        ));
    }
    match value {
        serde_yaml::Value::Sequence(items) => {
            for item in items {
                check_shape(item, depth + 1, nodes)?;
            }
        }
        serde_yaml::Value::Mapping(map) => {
            for (key, item) in map {
                check_shape(key, depth + 1, nodes)?;
                check_shape(item, depth + 1, nodes)?;
            }
        }
        serde_yaml::Value::Tagged(tagged) => check_shape(&tagged.value, depth + 1, nodes)?,
        _ => {}
    }
    Ok(())
}

/// Every key the schema does not declare, with a "did you mean" suggestion when one is close.
pub fn unknown_keys(value: &Value, schema: &RootSchema) -> Vec<ConfigIssue> {
    let mut out = Vec::new();
    walk(
        value,
        &Schema::Object(schema.schema.clone()),
        schema,
        "",
        &mut out,
    );
    out
}

fn join(path: &str, key: &str) -> String {
    if path.is_empty() {
        key.to_owned()
    } else {
        format!("{path}.{key}")
    }
}

/// The concrete schema objects behind `schema`: references resolved, `allOf`/`anyOf`/`oneOf`
/// flattened.
fn concrete<'a>(schema: &'a Schema, root: &'a RootSchema, depth: usize) -> Vec<&'a SchemaObject> {
    let Schema::Object(object) = schema else {
        return Vec::new();
    };
    if depth > 16 {
        return Vec::new();
    }
    let mut out = Vec::new();
    if let Some(reference) = &object.reference {
        let name = reference.rsplit('/').next().unwrap_or(reference);
        if let Some(target) = root.definitions.get(name) {
            out.extend(concrete(target, root, depth + 1));
        }
    }
    if let Some(sub) = &object.subschemas {
        for list in [&sub.all_of, &sub.any_of, &sub.one_of]
            .into_iter()
            .flatten()
        {
            for item in list {
                out.extend(concrete(item, root, depth + 1));
            }
        }
    }
    if object.object.is_some() || object.array.is_some() {
        out.push(object);
    }
    out
}

fn walk(value: &Value, schema: &Schema, root: &RootSchema, path: &str, out: &mut Vec<ConfigIssue>) {
    let candidates = concrete(schema, root, 0);
    match value {
        Value::Object(map) => {
            let Some(object) = candidates.iter().find_map(|c| c.object.as_deref()) else {
                return;
            };
            let closed = matches!(
                object.additional_properties.as_deref(),
                Some(Schema::Bool(false))
            );
            for (key, item) in map {
                let child_path = join(path, key);
                if let Some(child) = object.properties.get(key) {
                    walk(item, child, root, &child_path, out);
                } else if closed {
                    let known: Vec<&str> = object.properties.keys().map(String::as_str).collect();
                    out.push(unknown_key_issue(&child_path, key, &known));
                } else if let Some(additional) = object.additional_properties.as_deref() {
                    walk(item, additional, root, &child_path, out);
                }
            }
        }
        Value::Array(items) => {
            let Some(array) = candidates.iter().find_map(|c| c.array.as_deref()) else {
                return;
            };
            if let Some(SingleOrVec::Single(item_schema)) = &array.items {
                for (i, item) in items.iter().enumerate() {
                    walk(item, item_schema, root, &format!("{path}[{i}]"), out);
                }
            }
        }
        _ => {}
    }
}

pub(crate) fn unknown_key_issue(path: &str, key: &str, known: &[&str]) -> ConfigIssue {
    let message = match suggest(key, known) {
        Some(best) => format!("unknown key `{key}` (did you mean {best})"),
        None => format!("unknown key `{key}`; expected one of: {}", known.join(", ")),
    };
    ConfigIssue::error(ConfigIssueKind::UnknownKey, path, message)
}

/// The closest known name within edit distance 3 (or a prefix match).
pub fn suggest<'a>(key: &str, known: &[&'a str]) -> Option<&'a str> {
    known
        .iter()
        .map(|k| (levenshtein(key, k), *k))
        .filter(|(d, k)| *d <= 3 || k.starts_with(key) || key.starts_with(k))
        .min()
        .map(|(_, k)| k)
}

fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            cur[j + 1] = (prev[j] + cost).min(prev[j + 1] + 1).min(cur[j] + 1);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

/// True when `path` stays inside the repository root: relative, no `..` segment, no drive.
pub fn is_confined(path: &str) -> bool {
    if path.starts_with('/') || path.starts_with('\\') || path.starts_with('~') {
        return false;
    }
    let bytes = path.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
        return false;
    }
    !path.split(['/', '\\']).any(|segment| segment == "..")
}

fn check_glob(path: &str, glob: &str, out: &mut Vec<ConfigIssue>) {
    if !is_confined(glob) {
        out.push(ConfigIssue::error(
            ConfigIssueKind::PathEscape,
            path,
            format!("`{glob}` escapes the repository root"),
        ));
        return;
    }
    if let Err(e) = Glob::new(glob) {
        out.push(ConfigIssue::error(
            ConfigIssueKind::InvalidGlob,
            path,
            format!("`{glob}` is not a valid glob: {e}"),
        ));
    }
}

fn check_globs(path: &str, globs: &[String], out: &mut Vec<ConfigIssue>) {
    for (i, glob) in globs.iter().enumerate() {
        check_glob(&format!("{path}[{i}]"), glob, out);
    }
}

fn check_threshold(path: &str, value: f64, floor: f64, out: &mut Vec<ConfigIssue>) {
    if !(floor..=1.0).contains(&value) {
        out.push(ConfigIssue::error(
            ConfigIssueKind::OutOfRange,
            path,
            format!("{value} is outside [{floor}, 1]"),
        ));
    }
}

fn check_unique<'a>(path: &str, ids: impl Iterator<Item = &'a str>, out: &mut Vec<ConfigIssue>) {
    let mut seen = BTreeSet::new();
    for (i, id) in ids.enumerate() {
        if id.trim().is_empty() {
            out.push(ConfigIssue::error(
                ConfigIssueKind::InvalidValue,
                format!("{path}[{i}].id"),
                "id must not be empty",
            ));
        } else if !seen.insert(id) {
            out.push(ConfigIssue::error(
                ConfigIssueKind::DuplicateId,
                format!("{path}[{i}].id"),
                format!("duplicate id `{id}`"),
            ));
        }
    }
}

/// Semantic checks over a structurally valid config. `today` decides suppression expiry.
pub fn validate_semantics(config: &ReviewConfigV1, today: NaiveDate) -> Vec<ConfigIssue> {
    let mut out = Vec::new();
    if config.version != CONFIG_SCHEMA_VERSION {
        out.push(ConfigIssue::error(
            ConfigIssueKind::UnsupportedVersion,
            "version",
            format!(
                "version {} is not supported (expected {CONFIG_SCHEMA_VERSION})",
                config.version
            ),
        ));
    }

    check_globs("ignore", &config.ignore, &mut out);
    check_globs("generated.include", &config.generated.include, &mut out);
    check_globs("generated.exclude", &config.generated.exclude, &mut out);
    if let Some(ratio) = config.index.tolerance.max_failed_ratio {
        if !(0.0..=1.0).contains(&ratio) {
            out.push(ConfigIssue::error(
                ConfigIssueKind::OutOfRange,
                "index.tolerance.max_failed_ratio",
                format!("{ratio} is outside [0, 1]"),
            ));
        }
    }

    let review = &config.review;
    check_threshold(
        "review.confidence.minimum_publish",
        review.confidence.minimum_publish,
        MINIMUM_PUBLISH_FLOOR,
        &mut out,
    );
    for (name, value) in &review.confidence.per_reviewer {
        let path = format!("review.confidence.per_reviewer.{name}");
        if !REVIEWER_NAMES.contains(&name.as_str()) {
            out.push(unknown_key_issue(&path, name, &REVIEWER_NAMES));
            continue;
        }
        let floor = if name == "maintainability" {
            MAINTAINABILITY_FLOOR
        } else {
            MINIMUM_PUBLISH_FLOOR
        };
        check_threshold(&path, *value, floor, &mut out);
    }
    check_globs(
        "review.generated.ignore",
        &review.generated.ignore,
        &mut out,
    );
    for glob in review.risk.paths.keys() {
        check_glob(&format!("review.risk.paths.{glob}"), glob, &mut out);
    }

    let mut layers: BTreeSet<&str> = INFERABLE_LAYER_NAMES.iter().copied().collect();
    for (name, globs) in &config.architecture.layers {
        layers.insert(name.as_str());
        check_globs(&format!("architecture.layers.{name}"), globs, &mut out);
    }

    let rules = &config.rules;
    check_unique(
        "rules.forbidden_dependencies",
        rules.forbidden_dependencies.iter().map(|r| r.id.as_str()),
        &mut out,
    );
    for (i, rule) in rules.forbidden_dependencies.iter().enumerate() {
        for (field, layer) in [("from", &rule.from), ("to", &rule.to)] {
            if !layers.contains(layer.as_str()) {
                let known: Vec<&str> = layers.iter().copied().collect();
                let hint = suggest(layer, &known)
                    .map(|s| format!(" (did you mean {s})"))
                    .unwrap_or_default();
                out.push(ConfigIssue::error(
                    ConfigIssueKind::UndefinedLayer,
                    format!("rules.forbidden_dependencies[{i}].{field}"),
                    format!(
                        "layer `{layer}` is neither declared in architecture.layers nor \
                         inferable{hint}"
                    ),
                ));
            }
        }
        if rule.reason.trim().is_empty() {
            out.push(ConfigIssue::error(
                ConfigIssueKind::MissingReason,
                format!("rules.forbidden_dependencies[{i}].reason"),
                "a forbidden dependency needs a reason",
            ));
        }
    }
    check_globs(
        "rules.database.migration_paths",
        &rules.database.migration_paths,
        &mut out,
    );
    check_globs(
        "conventions.exceptions",
        &config.conventions.exceptions,
        &mut out,
    );

    check_unique(
        "suppressions",
        config.suppressions.iter().map(|s| s.id.as_str()),
        &mut out,
    );
    for (i, suppression) in config.suppressions.iter().enumerate() {
        let path = format!("suppressions[{i}]");
        if suppression.reason.trim().is_empty() {
            out.push(ConfigIssue::error(
                ConfigIssueKind::MissingReason,
                format!("{path}.reason"),
                "every suppression needs a reason",
            ));
        }
        if suppression.value.trim().is_empty() {
            out.push(ConfigIssue::error(
                ConfigIssueKind::InvalidValue,
                format!("{path}.value"),
                "value must not be empty",
            ));
        }
        if suppression.kind == SuppressionKind::Path {
            check_glob(&format!("{path}.value"), &suppression.value, &mut out);
        }
        if let Some(expires) = suppression.expires {
            if expires <= today {
                out.push(ConfigIssue::warning(
                    ConfigIssueKind::ExpiredSuppression,
                    format!("{path}.expires"),
                    format!("suppression `{}` expired on {expires}", suppression.id),
                ));
            }
        }
    }

    check_unique(
        "knowledge_sources",
        config.knowledge_sources.iter().map(|k| k.id.as_str()),
        &mut out,
    );
    for (i, source) in config.knowledge_sources.iter().enumerate() {
        if !is_confined(&source.path) {
            out.push(ConfigIssue::error(
                ConfigIssueKind::PathEscape,
                format!("knowledge_sources[{i}].path"),
                format!("`{}` escapes the repository root", source.path),
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levenshtein_basics() {
        assert_eq!(levenshtein("queue_job", "queue_jobs"), 1);
        assert_eq!(levenshtein("", "abc"), 3);
        assert_eq!(levenshtein("same", "same"), 0);
    }

    #[test]
    fn confinement() {
        assert!(is_confined("src/**/*.ts"));
        assert!(is_confined(".agent/knowledge"));
        assert!(!is_confined("../outside"));
        assert!(!is_confined("src/../../etc"));
        assert!(!is_confined("/etc/passwd"));
        assert!(!is_confined("C:/Windows"));
        assert!(!is_confined("~/secrets"));
    }
}
