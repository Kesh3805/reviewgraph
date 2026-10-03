//! JSON with comments and trailing commas (tsconfig, eslintrc, devcontainer ...), INIT-004.

use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid JSON at line {line}, column {column}: {message}")]
pub struct JsoncError {
    pub message: String,
    pub line: usize,
    pub column: usize,
}

/// Removes `//` and `/* */` comments (outside strings) and commas that directly precede `}` or
/// `]`. Whitespace inside removed comments is replaced so line numbers stay stable.
pub fn strip_jsonc(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    let mut in_string = false;
    while i < bytes.len() {
        let b = bytes[i];
        if in_string {
            out.push(b);
            if b == b'\\' && i + 1 < bytes.len() {
                out.push(bytes[i + 1]);
                i += 2;
                continue;
            }
            if b == b'"' {
                in_string = false;
            }
            i += 1;
            continue;
        }
        match b {
            b'"' => {
                in_string = true;
                out.push(b);
                i += 1;
            }
            b'/' if bytes.get(i + 1) == Some(&b'/') => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                i += 2;
                while i < bytes.len() && !(bytes[i] == b'*' && bytes.get(i + 1) == Some(&b'/')) {
                    if bytes[i] == b'\n' {
                        out.push(b'\n');
                    }
                    i += 1;
                }
                i = (i + 2).min(bytes.len());
                out.push(b' ');
            }
            _ => {
                out.push(b);
                i += 1;
            }
        }
    }
    remove_trailing_commas(&out)
}

fn remove_trailing_commas(input: &[u8]) -> String {
    let mut out: Vec<u8> = Vec::with_capacity(input.len());
    let mut in_string = false;
    let mut i = 0;
    while i < input.len() {
        let b = input[i];
        if in_string {
            out.push(b);
            if b == b'\\' && i + 1 < input.len() {
                out.push(input[i + 1]);
                i += 2;
                continue;
            }
            if b == b'"' {
                in_string = false;
            }
            i += 1;
            continue;
        }
        if b == b'"' {
            in_string = true;
            out.push(b);
        } else if b == b',' {
            let mut j = i + 1;
            while j < input.len() && input[j].is_ascii_whitespace() {
                j += 1;
            }
            if !matches!(input.get(j), Some(b'}') | Some(b']')) {
                out.push(b);
            }
        } else {
            out.push(b);
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Parses JSON that may contain comments, trailing commas and a leading BOM.
pub fn parse_jsonc(bytes: &[u8]) -> Result<Value, JsoncError> {
    let text = String::from_utf8_lossy(bytes);
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
    let cleaned = strip_jsonc(text);
    serde_json::from_str(&cleaned).map_err(|e| JsoncError {
        message: e.to_string(),
        line: e.line(),
        column: e.column(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn jsonc_strips_line_and_block_comments() {
        let text = r#"{
  // line comment
  "a": 1, /* block
  comment */ "b": 2
}"#;
        let v = parse_jsonc(text.as_bytes()).unwrap();
        assert_eq!(v["a"], 1);
        assert_eq!(v["b"], 2);
    }

    #[test]
    fn jsonc_keeps_comment_like_text_in_strings() {
        let text = r#"{"url": "http://example.com/a//b", "glob": "/* not a comment */", "esc": "quote \" // still string"}"#;
        let v = parse_jsonc(text.as_bytes()).unwrap();
        assert_eq!(v["url"], "http://example.com/a//b");
        assert_eq!(v["glob"], "/* not a comment */");
        assert_eq!(v["esc"], "quote \" // still string");
    }

    #[test]
    fn jsonc_trailing_commas() {
        let text = "{\"a\": [1, 2, 3,], \"b\": {\"c\": 1,},}";
        let v = parse_jsonc(text.as_bytes()).unwrap();
        assert_eq!(v["a"], serde_json::json!([1, 2, 3]));
        assert_eq!(v["b"]["c"], 1);
        // a comma inside a string before a bracket is preserved
        let v = parse_jsonc(b"{\"s\": \"a,]\"}").unwrap();
        assert_eq!(v["s"], "a,]");
    }

    #[test]
    fn jsonc_handles_bom_and_reports_position() {
        let v = parse_jsonc("\u{feff}{\"a\": 1}".as_bytes()).unwrap();
        assert_eq!(v["a"], 1);
        let err = parse_jsonc(b"{\n  \"a\": nope\n}").unwrap_err();
        assert_eq!(err.line, 2);
    }

    fn arb_json() -> impl Strategy<Value = serde_json::Value> {
        let leaf = prop_oneof![
            Just(serde_json::Value::Null),
            any::<bool>().prop_map(serde_json::Value::Bool),
            any::<i32>().prop_map(|n| serde_json::json!(n)),
            "[ -~]{0,12}".prop_map(serde_json::Value::String),
        ];
        leaf.prop_recursive(3, 24, 4, |inner| {
            prop_oneof![
                prop::collection::vec(inner.clone(), 0..4).prop_map(serde_json::Value::Array),
                prop::collection::btree_map("[a-z/]{0,6}", inner, 0..4)
                    .prop_map(|m| serde_json::Value::Object(m.into_iter().collect())),
            ]
        })
    }

    proptest! {
        #[test]
        fn jsonc_equals_serde_for_plain_json(value in arb_json()) {
            let text = serde_json::to_string_pretty(&value).unwrap();
            prop_assert_eq!(parse_jsonc(text.as_bytes()).unwrap(), value);
        }
    }
}
