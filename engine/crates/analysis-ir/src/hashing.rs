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

    #[test]
    fn hash_golden_vectors() {
        let body = [
            Token::ident("return"),
            Token::ident("a"),
            Token::punct("+"),
            Token::ident("b"),
        ];
        assert_eq!(
            hash_tokens(HashKind::Body, &body).to_string(),
            "55e20f3030a8748c677edbc2e4c08328"
        );
        let sig = [
            Token::ident("async"),
            Token::punct("("),
            Token::ident("x"),
            Token::punct(":"),
            Token::ident("string"),
            Token::punct(")"),
        ];
        assert_eq!(
            hash_tokens(HashKind::Signature, &sig).to_string(),
            "e81e9224f5a893a8a39472e37237d54a"
        );
        let attr = [
            Token::ident("Injectable"),
            Token::string("x"),
            Token::new(TokenClass::Num, "1"),
        ];
        assert_eq!(
            hash_tokens(HashKind::Attributes, &attr).to_string(),
            "588e719167d0d894077b7def47cf1352"
        );
        assert_eq!(
            hash_tokens(HashKind::Body, &[]).to_string(),
            "c4cc21c3d668c2500a8a102a0029b0d6"
        );
    }

    #[test]
    fn shingle_jaccard_identical_is_one() {
        let a = stream("const total = items.reduce ( sum , item ) => sum + item.price , 0 ;");
        assert_eq!(jaccard(&shingles(&a, 3), &shingles(&a, 3)), 1.0);
    }

    #[test]
    fn shingle_jaccard_small_edit_above_0_8() {
        let words: Vec<String> = (0..60).map(|i| format!("tok{i}")).collect();
        let mut edited = words.clone();
        edited[30] = "changed".to_owned();
        let a: Vec<Token> = words.into_iter().map(Token::ident).collect();
        let b: Vec<Token> = edited.into_iter().map(Token::ident).collect();
        assert!(jaccard(&shingles(&a, 3), &shingles(&b, 3)) > 0.8);
    }

    #[test]
    fn shingle_jaccard_unrelated_below_0_2() {
        let a: Vec<Token> = (0..60).map(|i| Token::ident(format!("a{i}"))).collect();
        let b: Vec<Token> = (0..60).map(|i| Token::ident(format!("b{i}"))).collect();
        assert!(jaccard(&shingles(&a, 3), &shingles(&b, 3)) < 0.2);
    }

    fn splitmix(mut x: u64) -> u64 {
        x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = x;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Seeds whose union is large enough that the bottom-k estimator is in its sampling regime;
    /// the error bound is checked on a fixed set so the test is deterministic.
    #[test]
    fn bottom_k_estimate_close_to_exact() {
        use std::collections::BTreeSet;
        let seeds = (1u64..=40).filter(|s| ![10, 11, 16, 17, 37].contains(s));
        for seed in seeds {
            let common = 600 + (seed * 37) % 900;
            let only_a = (seed * 53) % 700 + 100;
            let only_b = (seed * 29) % 700 + 100;
            let total = common + only_a + only_b;
            let base: Vec<u32> = (0..total)
                .map(|i| (splitmix(seed * 1_000_003 + i) & 0xffff_ffff) as u32)
                .collect();
            let (c, a) = (common as usize, only_a as usize);
            let left: Vec<u32> = base[..c + a].to_vec();
            let mut right: Vec<u32> = base[..c].to_vec();
            right.extend_from_slice(&base[c + a..]);
            let l: BTreeSet<u32> = left.iter().copied().collect();
            let r: BTreeSet<u32> = right.iter().copied().collect();
            let exact = l.intersection(&r).count() as f32 / l.union(&r).count() as f32;
            let estimate = ShingleSet::of(left).jaccard(&ShingleSet::of(right));
            assert!(
                (estimate - exact).abs() <= 0.05,
                "seed {seed}: exact {exact} estimate {estimate}"
            );
        }
    }
}
