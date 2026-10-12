//! Identifier tokenization for the lexical index (CTX-004).

/// Sub-tokens that carry no information on their own.
pub const STOPWORDS: [&str; 11] = [
    "get",
    "set",
    "id",
    "data",
    "value",
    "service",
    "controller",
    "module",
    "dto",
    "impl",
    "handler",
];
/// Sub-tokens shorter than this are dropped.
pub const MIN_SUBTOKEN_LEN: usize = 3;
/// Literal identifiers longer than this are never indexed (they are values, not names).
pub const MAX_LITERAL_LEN: usize = 64;

/// The tokens of one identifier: the full lowercased identifier and its camel/snake sub-tokens.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Tokens {
    pub full: String,
    pub subs: Vec<String>,
}

/// Splits on non-alphanumerics, lower-to-upper transitions and acronym ends (`HTTPServer` is
/// `http`, `server`).
fn split_words(ident: &str) -> Vec<String> {
    let chars: Vec<char> = ident.chars().collect();
    let mut words: Vec<String> = Vec::new();
    let mut cur = String::new();
    for (i, &ch) in chars.iter().enumerate() {
        if !ch.is_alphanumeric() {
            if !cur.is_empty() {
                words.push(std::mem::take(&mut cur));
            }
            continue;
        }
        let prev = i.checked_sub(1).and_then(|p| chars.get(p)).copied();
        let next = chars.get(i + 1).copied();
        let boundary = match prev {
            Some(p) if p.is_alphanumeric() => {
                (p.is_lowercase() && ch.is_uppercase())
                    || (p.is_uppercase()
                        && ch.is_uppercase()
                        && next.is_some_and(char::is_lowercase))
                    || (p.is_alphabetic() && ch.is_ascii_digit())
                    || (p.is_ascii_digit() && ch.is_alphabetic())
            }
            _ => false,
        };
        if boundary && !cur.is_empty() {
            words.push(std::mem::take(&mut cur));
        }
        cur.push(ch);
    }
    if !cur.is_empty() {
        words.push(cur);
    }
    words.into_iter().map(|w| w.to_lowercase()).collect()
}

pub fn tokenize(ident: &str) -> Tokens {
    let full = ident.trim().to_lowercase();
    if full.is_empty() {
        return Tokens::default();
    }
    let mut subs: Vec<String> = Vec::new();
    for word in split_words(ident) {
        if word.chars().count() >= MIN_SUBTOKEN_LEN
            && !STOPWORDS.contains(&word.as_str())
            && word != full
            && !subs.contains(&word)
        {
            subs.push(word);
        }
    }
    Tokens { full, subs }
}

/// Values that look like secrets (long unbroken alphanumeric runs mixing letters and digits) or
/// are too long to be identifiers are never indexed.
pub fn skip_literal(text: &str) -> bool {
    if text.chars().count() > MAX_LITERAL_LEN {
        return true;
    }
    let long_run = text.len() >= 32
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '=' | '-' | '_'))
        && text.chars().any(|c| c.is_ascii_digit())
        && text.chars().any(|c| c.is_ascii_alphabetic());
    let lower = text.to_ascii_lowercase();
    long_run
        || lower.starts_with("sk_")
        || lower.starts_with("ghp_")
        || lower.starts_with("akia")
        || lower.contains("-----begin")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camel_and_snake_split() {
        let t = tokenize("updateUserPermissions");
        assert_eq!(t.full, "updateuserpermissions");
        assert_eq!(t.subs, vec!["update", "user", "permissions"]);
        let t = tokenize("PERMISSION_SERVICE");
        assert_eq!(t.full, "permission_service");
        assert_eq!(t.subs, vec!["permission"]);
    }

    #[test]
    fn dotted_literals_split() {
        let t = tokenize("user.updated");
        assert_eq!(t.full, "user.updated");
        assert_eq!(t.subs, vec!["user", "updated"]);
    }

    #[test]
    fn acronyms_stay_together() {
        assert_eq!(tokenize("HTTPServer").subs, vec!["http", "server"]);
    }

    #[test]
    fn secrets_and_long_values_skipped() {
        assert!(skip_literal(&"a".repeat(65)));
        assert!(skip_literal("AbCdEf0123456789AbCdEf0123456789xyz"));
        assert!(!skip_literal("user.updated"));
    }
}
