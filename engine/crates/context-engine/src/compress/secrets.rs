//! Secret screening for excerpts (CTX-007). A segment that looks secret-bearing causes its whole
//! item to be dropped, never partially masked.

const ASSIGNMENT_KEYWORDS: [&str; 6] =
    ["password", "passwd", "secret", "api_key", "apikey", "token"];
/// `(prefix, minimum length of the token characters after it)`.
const TOKEN_PREFIXES: [(&str, usize); 6] = [
    ("AKIA", 16),
    ("ghp_", 30),
    ("gho_", 30),
    ("xoxb-", 20),
    ("xoxp-", 20),
    ("sk_live_", 16),
];

fn is_token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

fn has_prefixed_token(text: &str) -> bool {
    TOKEN_PREFIXES.iter().any(|(prefix, min)| {
        text.match_indices(prefix).any(|(pos, _)| {
            text[pos + prefix.len()..]
                .chars()
                .take_while(|c| is_token_char(*c))
                .count()
                >= *min
        })
    })
}

/// `password: "hunter2hunter2"` style assignments of a literal of at least 8 characters.
fn has_assigned_secret(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    ASSIGNMENT_KEYWORDS.iter().any(|kw| {
        lower.match_indices(kw).any(|(pos, _)| {
            let after = &line[pos + kw.len()..];
            let Some(sep) = after.find([':', '=']) else {
                return false;
            };
            if sep > 12 {
                return false;
            }
            let value = after[sep + 1..].trim_start_matches(['=', '>', ' ', '\t']);
            let Some(quote) = value
                .chars()
                .next()
                .filter(|c| matches!(c, '"' | '\'' | '`'))
            else {
                return false;
            };
            let inner = &value[quote.len_utf8()..];
            let literal = inner.split(quote).next().unwrap_or("");
            literal.chars().count() >= 8
                && !literal.contains(' ')
                && !literal.contains("${")
                && !literal.starts_with("process.env")
        })
    })
}

/// Whether `text` contains something that looks like a credential.
pub fn contains_secret(text: &str) -> bool {
    if text.contains("-----BEGIN") && text.contains("PRIVATE KEY") {
        return true;
    }
    has_prefixed_token(text) || text.lines().any(has_assigned_secret)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_common_shapes() {
        assert!(contains_secret("const password = 'hunter2hunter2';"));
        assert!(contains_secret("aws: AKIAABCDEFGHIJKLMNOP"));
        assert!(contains_secret(
            "-----BEGIN RSA PRIVATE KEY-----\nabc\n-----END"
        ));
    }

    #[test]
    fn ignores_plain_code() {
        assert!(!contains_secret("const password: string = input.password;"));
        assert!(!contains_secret("authorize(user, 'update')"));
        assert!(!contains_secret("token = `${prefix}-abcdefgh`"));
    }
}
