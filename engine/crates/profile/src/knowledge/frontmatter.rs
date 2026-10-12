//! Markdown frontmatter (`---` YAML block at the top of a document).

use std::collections::BTreeMap;

use serde_yaml::Value;

/// A parsed document: frontmatter (if any), its body, and where the body starts.
#[derive(Debug, Clone, PartialEq)]
pub struct Parsed {
    pub frontmatter: Option<BTreeMap<String, Value>>,
    pub body: String,
    /// 1-based line of the body's first line.
    pub body_start_line: u32,
    pub total_lines: u32,
}

/// Splits frontmatter from the body. `Err` carries a diagnostic for malformed frontmatter (an
/// unterminated block or YAML that is not a mapping).
pub fn parse(text: &str) -> Result<Parsed, String> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let lines: Vec<&str> = text.lines().collect();
    let total_lines = u32::try_from(lines.len()).unwrap_or(u32::MAX);
    if lines.first().map(|l| l.trim_end()) != Some("---") {
        return Ok(Parsed {
            frontmatter: None,
            body: text.to_owned(),
            body_start_line: 1,
            total_lines,
        });
    }
    let Some(end) = lines
        .iter()
        .skip(1)
        .position(|l| matches!(l.trim_end(), "---" | "..."))
    else {
        return Err("frontmatter starts with --- but is never closed".to_owned());
    };
    let end = end + 1;
    let yaml = lines[1..end].join("\n");
    let value: Value = serde_yaml::from_str(&yaml).map_err(|e| format!("frontmatter: {e}"))?;
    let map = match value {
        Value::Mapping(map) => map
            .into_iter()
            .filter_map(|(k, v)| k.as_str().map(|k| (k.to_owned(), v)))
            .collect(),
        Value::Null => BTreeMap::new(),
        _ => return Err("frontmatter is not a mapping".to_owned()),
    };
    Ok(Parsed {
        frontmatter: Some(map),
        body: lines[end + 1..].join("\n"),
        body_start_line: u32::try_from(end + 2).unwrap_or(u32::MAX),
        total_lines,
    })
}

/// A string value.
pub fn string(map: &BTreeMap<String, Value>, key: &str) -> Option<String> {
    match map.get(key)? {
        Value::String(s) => Some(s.trim().to_owned()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// A string or a list of strings.
pub fn strings(map: &BTreeMap<String, Value>, key: &str) -> Vec<String> {
    match map.get(key) {
        Some(Value::String(s)) => vec![s.trim().to_owned()],
        Some(Value::Sequence(items)) => items
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.trim().to_owned()))
            .filter(|s| !s.is_empty())
            .collect(),
        _ => Vec::new(),
    }
}

/// The first `# Heading` of a body.
pub fn first_heading(body: &str) -> Option<String> {
    body.lines()
        .map(str::trim)
        .find_map(|l| l.strip_prefix("# "))
        .map(|h| h.trim().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_frontmatter() {
        let p = parse("---\nid: a\napplies_to: [\"src/**\"]\n---\n# Title\nBody\n").unwrap();
        let fm = p.frontmatter.unwrap();
        assert_eq!(string(&fm, "id").as_deref(), Some("a"));
        assert_eq!(strings(&fm, "applies_to"), vec!["src/**"]);
        assert_eq!(p.body_start_line, 5);
        assert_eq!(first_heading(&p.body).as_deref(), Some("Title"));
    }

    #[test]
    fn no_frontmatter() {
        let p = parse("# Just a doc\n").unwrap();
        assert!(p.frontmatter.is_none());
        assert_eq!(p.body_start_line, 1);
    }

    #[test]
    fn unterminated_is_an_error() {
        assert!(parse("---\nid: a\n# no end\n").is_err());
        assert!(parse("---\n- a\n- b\n---\nbody\n").is_err());
    }
}
