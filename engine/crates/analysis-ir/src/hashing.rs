//! Language-neutral token hashing (TSA-007 primitives, consumed by SID-004 and SID-005).
//!
//! A hash must change when behaviour changes and not when formatting, comments, quote style or
//! trailing commas change. Analyzers therefore normalize source into a token stream and hand it to
//! [`TokenHasher`], which frames every token as `class || len_le_u32 || bytes` under a domain
//! separator and truncates the BLAKE3 digest to 128 bits.

use review_core::symbol::{Hash128, ShingleSet};

/// Domain of [`HashKind::Body`].
pub const HASH_DOMAIN_BODY: &str = "rg.body.v1";
/// Domain of [`HashKind::Signature`].
pub const HASH_DOMAIN_SIGNATURE: &str = "rg.sig.v1";
/// Domain of [`HashKind::Attributes`].
pub const HASH_DOMAIN_ATTR: &str = "rg.attr.v1";
/// Domain of the shingle stream.
pub const HASH_DOMAIN_SHINGLE: &str = "rg.shingle.v1";

/// Which hash a token stream feeds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HashKind {
    /// Body tokens.
    Body,
    /// Signature tokens (the symbol's own name is excluded).
    Signature,
    /// Decorators and export/visibility flags.
    Attributes,
}

impl HashKind {
    /// The domain separator of this hash family.
    pub const fn domain(self) -> &'static str {
        match self {
            Self::Body => HASH_DOMAIN_BODY,
            Self::Signature => HASH_DOMAIN_SIGNATURE,
            Self::Attributes => HASH_DOMAIN_ATTR,
        }
    }
}

/// Lexical class of a token. Part of the hashed stream, so `Str("a")` and `Ident("a")` differ.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u8)]
pub enum TokenClass {
    /// Identifier or keyword (keywords are identifiers to the hasher: the class is the same).
    Ident = 0,
    /// String literal, already normalized to its content.
    Str = 1,
    /// Number, already normalized.
    Num = 2,
    /// Punctuation and operators.
    Punct = 3,
    /// Template chunk.
    Template = 4,
    /// Regular expression literal.
    Regex = 5,
    /// Placeholder standing in for a child symbol of a container.
    Placeholder = 6,
}

impl TokenClass {
    /// Stable byte used in the hash framing.
    pub const fn tag(self) -> u8 {
        self as u8
    }
}

/// One normalized token.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Token {
    /// Lexical class.
    pub class: TokenClass,
    /// Normalized text.
    pub text: String,
}

impl Token {
    /// A token of the given class.
    pub fn new(class: TokenClass, text: impl Into<String>) -> Self {
        Self {
            class,
            text: text.into(),
        }
    }

    /// An identifier token.
    pub fn ident(text: impl Into<String>) -> Self {
        Self::new(TokenClass::Ident, text)
    }

    /// A punctuation token.
    pub fn punct(text: impl Into<String>) -> Self {
        Self::new(TokenClass::Punct, text)
    }

    /// A string-literal token holding its content.
    pub fn string(text: impl Into<String>) -> Self {
        Self::new(TokenClass::Str, text)
    }
}

/// Streaming hasher over a normalized token stream.
#[derive(Debug)]
pub struct TokenHasher {
    domain: &'static str,
    inner: blake3::Hasher,
    count: u32,
}

impl TokenHasher {
    /// A hasher for one hash family.
    pub fn new(kind: HashKind) -> Self {
        Self::with_domain(kind.domain())
    }

    /// A hasher with an explicit domain, for tests and for the shingle stream.
    pub fn with_domain(domain: &'static str) -> Self {
        let mut inner = blake3::Hasher::new();
        inner.update(domain.as_bytes());
        inner.update(&[0]);
        Self {
            domain,
            inner,
            count: 0,
        }
    }

    /// The domain separator this hasher mixes in, for diagnostics and metrics labels.
    pub const fn domain(&self) -> &'static str {
        self.domain
    }

    /// Feeds one token.
    pub fn push(&mut self, token: &Token) -> &mut Self {
        let bytes = token.text.as_bytes();
        self.inner.update(&[token.class.tag()]);
        self.inner.update(&(bytes.len() as u32).to_le_bytes());
        self.inner.update(bytes);
        self.count = self.count.saturating_add(1);
        self
    }

    /// Feeds a whole stream.
    pub fn extend<'a>(&mut self, tokens: impl IntoIterator<Item = &'a Token>) -> &mut Self {
        for token in tokens {
            self.push(token);
        }
        self
    }

    /// Number of tokens fed so far.
    pub fn token_count(&self) -> u32 {
        self.count
    }

    /// Finishes the digest. An empty stream still hashes to a stable non-zero value, so "no body"
    /// is distinguishable from the unset [`Hash128::ZERO`].
    pub fn finish(self) -> Hash128 {
        let digest = self.inner.finalize();
        let mut out = [0u8; 16];
        out.copy_from_slice(&digest.as_bytes()[..16]);
        Hash128::from_bytes(out)
    }
}

/// Hashes a whole stream in one call.
pub fn hash_tokens(kind: HashKind, tokens: &[Token]) -> Hash128 {
    let mut hasher = TokenHasher::new(kind);
    hasher.extend(tokens);
    hasher.finish()
}

/// Body shingles: hashed token n-grams, de-duplicated into a bottom-k sketch (TSA-007). Bodies
/// with fewer than three tokens get a 1-gram set so tiny bodies are still comparable.
pub fn shingles(tokens: &[Token], n: usize) -> ShingleSet {
    let width = if tokens.len() >= 3 { n.max(3) } else { 1 };
    if tokens.is_empty() {
        return ShingleSet::default();
    }
    if tokens.len() < width {
        return ShingleSet::of([shingle_hash(&tokens[..1])]);
    }
    ShingleSet::of(tokens.windows(width).map(shingle_hash))
}

/// Jaccard similarity of two body token streams, from their shingles.
pub fn jaccard(a: &ShingleSet, b: &ShingleSet) -> f32 {
    a.jaccard(b)
}

fn shingle_hash(window: &[Token]) -> u32 {
    let mut hasher = TokenHasher::with_domain(HASH_DOMAIN_SHINGLE);
    for token in window {
        hasher.push(token);
    }
    let digest = hasher.finish();
    let bytes = digest.as_bytes();
    u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stream(source: &str) -> Vec<Token> {
        source
            .split_whitespace()
            .map(|t| Token::ident(t.to_owned()))
            .collect()
    }

    #[test]
    fn domains_separate_families() {
        let tokens = stream("const a = 1;");
        let body = hash_tokens(HashKind::Body, &tokens);
        let sig = hash_tokens(HashKind::Signature, &tokens);
        let attr = hash_tokens(HashKind::Attributes, &tokens);
        assert_ne!(body, sig);
        assert_ne!(body, attr);
        assert!(!body.is_zero());
        assert_eq!(HashKind::Body.domain(), "rg.body.v1");
    }

    #[test]
    fn empty_stream_is_stable_and_non_zero() {
        let empty = hash_tokens(HashKind::Body, &[]);
        assert!(
            !empty.is_zero(),
            "an absent body must differ from the unset hash"
        );
        assert_eq!(empty, hash_tokens(HashKind::Body, &[]));
        assert_ne!(empty, Hash128::ZERO);
    }

    #[test]
    fn token_count_and_framing() {
        let tokens = stream("a b c");
        let mut hasher = TokenHasher::new(HashKind::Body);
        hasher.extend(&tokens);
        assert_eq!(hasher.token_count(), 3);
        assert_eq!(hasher.finish(), hash_tokens(HashKind::Body, &tokens));
    }

    #[test]
    fn shingles_similarity() {
        let a = stream("const total = items.reduce((sum, item) => sum + item.price, 0);");
        let b = stream("const total = items.reduce((sum, item) => sum + item.price, 0);");
        let c = stream("return user.profile.displayName.trim().toLowerCase();");
        let exact = shingles(&a, 3);
        assert_eq!(exact.jaccard(&shingles(&b, 3)), 1.0);
        assert!(shingles(&a, 3).jaccard(&shingles(&c, 3)) < 0.2);
        assert!(shingles(&[], 3).is_empty());
        assert!(!shingles(&stream("x"), 3).is_empty());
    }

    #[test]
    fn classes_are_part_of_the_stream() {
        let a = hash_tokens(HashKind::Body, &[Token::ident("x"), Token::punct("=")]);
        let b = hash_tokens(HashKind::Body, &[Token::punct("="), Token::ident("x")]);
        assert_ne!(a, b, "order and class both matter");
    }
}
