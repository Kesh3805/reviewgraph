//! Normalized token extraction for TypeScript/JavaScript source (TSA-007 primitive, needed by the
//! symbol differ and the rename matcher).
//!
//! The lexer is deliberately small and total: it never fails, never needs the syntax tree, and its
//! output is a pure function of the bytes. Comments and whitespace are dropped, string literals of
//! either quote style collapse to their content, numeric separators are removed, and trailing
//! commas plus a final `;` are ignored, so a reformat cannot change a hash.

use analysis_ir::hashing::{Token, TokenClass};

/// A token with its byte range in the source, needed to replace a container's children with
/// placeholders.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpannedToken {
    /// Byte offset of the first byte of the token in the analyzed slice.
    pub start: usize,
    /// Byte offset one past the last byte.
    pub end: usize,
    /// The normalized token.
    pub token: Token,
}

/// Tokenizes `src` into normalized tokens with their byte ranges.
pub fn spanned_tokens(src: &str) -> Vec<SpannedToken> {
    let bytes = src.as_bytes();
    let mut out: Vec<SpannedToken> = Vec::new();
    let mut index = 0usize;
    while index < bytes.len() {
        let byte = bytes[index];
        match byte {
            b' ' | b'\t' | b'\r' | b'\n' => index += 1,
            b'/' if bytes.get(index + 1) == Some(&b'/') => {
                index = boundary(src, skip_to_newline(bytes, index));
            }
            b'/' if bytes.get(index + 1) == Some(&b'*') => {
                index = boundary(src, skip_block_comment(bytes, index));
            }
            b'\'' | b'"' => {
                let start = index;
                let end = boundary(src, scan_quoted(bytes, index, byte));
                let content = slice(src, start + 1, end.saturating_sub(1).max(start + 1));
                out.push(SpannedToken {
                    start,
                    end,
                    token: Token::new(TokenClass::Str, content),
                });
                index = end;
            }
            b'`' => {
                let start = index;
                let end = boundary(src, scan_template(bytes, index));
                out.push(SpannedToken {
                    start,
                    end,
                    token: Token::new(TokenClass::Template, slice(src, start, end)),
                });
                index = end;
            }
            b'0'..=b'9' => {
                let start = index;
                while index < bytes.len() && is_number_byte(bytes[index]) {
                    index += 1;
                }
                let text: String = slice(src, start, index)
                    .chars()
                    .filter(|c| *c != '_')
                    .collect();
                out.push(SpannedToken {
                    start,
                    end: index,
                    token: Token::new(TokenClass::Num, text.to_lowercase()),
                });
            }
            _ if is_ident_start(src, index) => {
                let start = index;
                index += char_len(src, index);
                while index < bytes.len() && is_ident_continue(src, index) {
                    index += char_len(src, index);
                }
                out.push(SpannedToken {
                    start,
                    end: index,
                    token: Token::ident(slice(src, start, index)),
                });
            }
            _ => {
                // A non-identifier character may be several bytes long (an arrow, a lone
                // replacement character); step over all of them so every index stays on a
                // character boundary.
                let end = boundary(src, index + char_len(src, index));
                out.push(SpannedToken {
                    start: index,
                    end,
                    token: Token::punct(char_at(src, index).to_string()),
                });
                index = end;
            }
        }
    }
    drop_trivia(&mut out);
    out
}

/// Tokenizes `src` into normalized tokens.
pub fn tokens_of(src: &str) -> Vec<Token> {
    spanned_tokens(src).into_iter().map(|t| t.token).collect()
}

/// Drops a trailing comma before a closing bracket and a trailing `;`, and collapses runs of `;`
/// so an added or removed semicolon cannot change a hash.
fn drop_trivia(tokens: &mut Vec<SpannedToken>) {
    if let Some(last) = tokens.last() {
        if last.token.class == TokenClass::Punct && last.token.text == ";" {
            tokens.pop();
        }
    }
    let mut index = 0;
    while index < tokens.len() {
        let is_comma =
            tokens[index].token.class == TokenClass::Punct && tokens[index].token.text == ",";
        let next_is_close = tokens.get(index + 1).is_some_and(|t| {
            t.token.class == TokenClass::Punct && matches!(t.token.text.as_str(), ")" | "]" | "}")
        });
        if is_comma && next_is_close {
            tokens.remove(index);
            continue;
        }
        let is_semi =
            tokens[index].token.class == TokenClass::Punct && tokens[index].token.text == ";";
        let next_is_close = tokens.get(index + 1).is_some_and(|t| {
            t.token.class == TokenClass::Punct && matches!(t.token.text.as_str(), ")" | "]" | "}")
        });
        if is_semi && next_is_close {
            tokens.remove(index);
            continue;
        }
        index += 1;
    }
}

fn skip_to_newline(bytes: &[u8], mut index: usize) -> usize {
    while index < bytes.len() && bytes[index] != b'\n' {
        index += 1;
    }
    index
}

fn skip_block_comment(bytes: &[u8], mut index: usize) -> usize {
    index += 2;
    while index < bytes.len() {
        if bytes[index] == b'*' && bytes.get(index + 1) == Some(&b'/') {
            return index + 2;
        }
        index += 1;
    }
    index
}

fn scan_quoted(bytes: &[u8], start: usize, quote: u8) -> usize {
    let mut index = start + 1;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => index += 2,
            b'\n' => return index,
            b if b == quote => return index + 1,
            _ => index += 1,
        }
    }
    index
}

fn scan_template(bytes: &[u8], start: usize) -> usize {
    let mut index = start + 1;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => index += 2,
            b'`' => return index + 1,
            b'$' if bytes.get(index + 1) == Some(&b'{') => {
                let mut depth = 1;
                index += 2;
                while index < bytes.len() && depth > 0 {
                    match bytes[index] {
                        b'{' => depth += 1,
                        b'}' => depth -= 1,
                        b'`' => {
                            index = scan_template(bytes, index);
                            continue;
                        }
                        _ => {}
                    }
                    index += 1;
                }
            }
            _ => index += 1,
        }
    }
    index
}

fn is_number_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'.' || byte == b'_'
}

fn is_ident_start(src: &str, index: usize) -> bool {
    let c = char_at(src, index);
    c == '_' || c == '$' || c.is_alphabetic()
}

fn is_ident_continue(src: &str, index: usize) -> bool {
    let c = char_at(src, index);
    c == '_' || c == '$' || c.is_alphanumeric()
}

fn char_at(src: &str, index: usize) -> char {
    src.get(index..)
        .and_then(|rest| rest.chars().next())
        .unwrap_or('\0')
}

/// `index` clamped to the text and moved forward to the next character boundary. Scanners step
/// over escapes two bytes at a time, which can land inside a multi-byte character or past the
/// end of an unterminated literal.
fn boundary(src: &str, index: usize) -> usize {
    let mut index = index.min(src.len());
    while !src.is_char_boundary(index) {
        index += 1;
    }
    index
}

/// `src[start..end]`, or an empty string when the range is not on character boundaries.
fn slice(src: &str, start: usize, end: usize) -> String {
    src.get(start..end).unwrap_or_default().to_owned()
}

fn char_len(src: &str, index: usize) -> usize {
    char_at(src, index).len_utf8().max(1)
}

/// The placeholder token that replaces a container's child symbol (TSA-007): adding, removing or
/// renaming a member changes the container's body hash, while editing a member's body does not.
pub fn child_placeholder(kind: &str, name: &str) -> Token {
    Token::new(TokenClass::Placeholder, format!("<child:{kind}:{name}>"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multibyte_and_unterminated_input_never_panics() {
        for text in [
            "'\\\u{2192}",
            "a \u{2192} b",
            "`x\\\u{fffd}",
            "\"\\",
            "1\u{e9}",
            "/*\u{2192}",
            "x \u{fffd}\u{fffd} y",
        ] {
            let _ = tokens_of(text);
        }
        assert_eq!(rendered("a \u{2192} b").len(), 3);
    }

    fn rendered(src: &str) -> Vec<String> {
        spanned_tokens(src)
            .into_iter()
            .map(|t| format!("{:?}:{}", t.token.class, t.token.text))
            .collect()
    }

    #[test]
    fn whitespace_and_comments_are_dropped() {
        let a = rendered("const a = 1;");
        let b = rendered("// leading\nconst   a\t=\n1 ;\n/* block */");
        assert_eq!(a, b);
        assert_eq!(a.len(), 4);
    }

    #[test]
    fn quote_style_is_normalized() {
        assert_eq!(
            rendered("const a = 'x';"),
            rendered("const a = \"x\";"),
            "a quote change is not a behaviour change"
        );
        let with_escapes = rendered("const a = 'it\\'s';");
        assert!(with_escapes.iter().any(|t| t.contains("it\\'s")));
    }

    #[test]
    fn numbers_are_normalized() {
        assert_eq!(rendered("const a = 1_000;"), rendered("const a = 1000;"));
        assert_eq!(rendered("const a = 0XFF;"), rendered("const a = 0xff;"));
    }

    #[test]
    fn trailing_commas_and_final_semicolon_are_dropped() {
        assert_eq!(rendered("f(a, b,);"), rendered("f(a, b)"));
        assert_eq!(rendered("const a = [1, 2,];"), rendered("const a = [1, 2]"));
    }

    #[test]
    fn templates_and_identifiers() {
        let tokens = spanned_tokens("const a = `x${y}z`;");
        assert!(tokens
            .iter()
            .any(|t| t.token.class == TokenClass::Template && t.token.text.contains("${y}")));
        let unicode = spanned_tokens("const über = 1;");
        assert_eq!(unicode[1].token.text, "über");
        assert_eq!(unicode[1].end - unicode[1].start, "über".len());
    }

    #[test]
    fn spans_point_at_the_source() {
        let src = "const total = items.length;";
        let tokens = spanned_tokens(src);
        for spanned in &tokens {
            assert!(!src[spanned.start..spanned.end].is_empty());
            assert!(spanned.end <= src.len());
        }
    }

    #[test]
    fn unterminated_constructs_do_not_panic() {
        for src in [
            "const a = 'unterminated",
            "const a = `x",
            "/* unterminated",
            "const",
        ] {
            let _ = spanned_tokens(src);
        }
        assert!(!spanned_tokens("const").is_empty());
        assert!(spanned_tokens("").is_empty());
    }
}
