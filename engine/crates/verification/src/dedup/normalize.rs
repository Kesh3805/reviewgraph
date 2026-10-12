//! Deterministic claim normalization (DED-001).

/// Normalized claim: sorted, de-duplicated tokens.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NormalizedClaim {
    tokens: Vec<String>,
}

impl NormalizedClaim {
    pub fn tokens(&self) -> &[String] {
        &self.tokens
    }

    pub fn is_empty(&self) -> bool {
        self.tokens.is_empty()
    }

    /// Canonical single-string form (stored in `candidate_findings.claim_normalized`).
    pub fn as_text(&self) -> String {
        self.tokens.join(" ")
    }

    /// Jaccard similarity over token sets (used by DED-002 rule R3).
    pub fn jaccard(&self, other: &Self) -> f32 {
        if self.tokens.is_empty() && other.tokens.is_empty() {
            return 1.0;
        }
        let inter = self
            .tokens
            .iter()
            .filter(|t| other.tokens.binary_search(t).is_ok())
            .count();
        let union = self.tokens.len() + other.tokens.len() - inter;
        inter as f32 / union as f32
    }
}

const STOP_WORDS: &[&str] = &[
    "a", "an", "the", "is", "are", "was", "were", "be", "been", "of", "to", "in", "on", "at",
    "for", "and", "or", "it", "its", "this", "that", "with", "by", "as", "from", "which", "can",
    "may", "will", "would", "should", "could",
];

const LIT: &str = "<lit>";

fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Normalize a free-text claim: lowercase, strip markdown/backticks, replace double-quoted
/// literals with `<lit>`, drop line-number tokens, punctuation and stop words. Identifiers
/// (camelCase, snake_case) are kept intact.
///
/// Unicode folding is limited to `to_lowercase`; full NFKC needs the `unicode-normalization`
/// dependency and is a follow-up.
pub fn normalize_claim(input: &str) -> NormalizedClaim {
    let mut tokens: Vec<String> = Vec::new();
    let mut word = String::new();
    let mut in_quote = false;
    let mut quoted = false;
    let mut raw: Vec<String> = Vec::new();

    let flush = |word: &mut String, raw: &mut Vec<String>| {
        if !word.is_empty() {
            raw.push(std::mem::take(word));
        }
    };

    for c in input.chars() {
        if c == '"' {
            if in_quote {
                in_quote = false;
                if quoted {
                    raw.push(LIT.to_string());
                }
                quoted = false;
            } else {
                flush(&mut word, &mut raw);
                in_quote = true;
            }
            continue;
        }
        if in_quote {
            quoted = true;
            continue;
        }
        if is_ident_char(c) {
            word.extend(c.to_lowercase());
        } else {
            flush(&mut word, &mut raw);
        }
    }
    flush(&mut word, &mut raw);

    let mut i = 0;
    while i < raw.len() {
        let t = &raw[i];
        let is_num = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit());
        if t == "line" && raw.get(i + 1).is_some_and(|n| is_num(n)) {
            i += 2;
            continue;
        }
        if t.len() > 1 && t.starts_with('l') && is_num(&t[1..]) {
            i += 1;
            continue;
        }
        if is_num(t) || STOP_WORDS.contains(&t.as_str()) {
            i += 1;
            continue;
        }
        tokens.push(t.clone());
        i += 1;
    }
    tokens.sort();
    tokens.dedup();
    NormalizedClaim { tokens }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removes_line_numbers_and_literals() {
        let a = normalize_claim("The check at line 42 reads \"secret\" in `checkAuth`: L7");
        assert_eq!(a.as_text(), "<lit> check checkauth reads");
    }

    #[test]
    fn whitespace_and_case_invariant() {
        assert_eq!(
            normalize_claim("Auth   CHECK removed"),
            normalize_claim("auth check\n removed")
        );
    }

    #[test]
    fn identifiers_kept_intact() {
        assert!(normalize_claim("calls get_user_id")
            .tokens()
            .contains(&"get_user_id".to_string()));
    }
}
