//! Deterministic offline embedding by feature hashing (SEM-002).
//!
//! Used in development, CI and replay, where no API key exists. Lexical, not semantic, but
//! identifier-aware: `authorizeUser` and `authorize_user` produce the same features.
//!
//! Features: sub-token unigrams (weight 1), adjacent sub-token bigrams (weight 1) and character
//! trigrams of each identifier's joined lowercase sub-tokens (weight 0.5). Each feature `f` maps to
//! index `xxh3_64(f, seed) % dims` with the sign taken from bit 63, and adds
//! `sign * weight * (1 + ln(tf))`. Features are visited in sorted order and summed in `f64`, so
//! the output is bit-exact across runs and threads.

use std::collections::BTreeMap;

use async_trait::async_trait;
use xxhash_rust::xxh3::xxh3_64_with_seed;

use super::wrap::l2_normalize;
use super::{estimate_tokens, ProviderName};
use super::{EmbedError, EmbedRequest, EmbedResponse, EmbeddingProvider, EmbeddingSpace};
use crate::error::Error;

pub const DEFAULT_DIMS: u16 = 768;
pub const MIN_DIMS: u16 = 256;
pub const MAX_DIMS: u16 = 4096;
/// `"REVIEW"` in ASCII.
pub const SEED: u64 = 0x5245_5649_4557;
const MAX_BATCH: usize = 1024;
const MAX_INPUT_TOKENS: usize = 8192;

/// Feature-hashing provider. Space `hash-fh{dims}-{dims}`.
#[derive(Debug, Clone)]
pub struct HashProvider {
    space: EmbeddingSpace,
}

impl HashProvider {
    pub fn new(dims: u16, version: u16) -> Result<Self, Error> {
        if !(MIN_DIMS..=MAX_DIMS).contains(&dims) {
            return Err(Error::Config(format!(
                "hash provider dims must be in {MIN_DIMS}..={MAX_DIMS}, got {dims}"
            )));
        }
        Ok(Self {
            space: EmbeddingSpace::new(ProviderName::Hash, format!("fh{dims}"), dims, version)?,
        })
    }

    /// 768 dims, version 1.
    pub fn default_space() -> Result<Self, Error> {
        Self::new(DEFAULT_DIMS, 1)
    }

    /// Embeds one text (unit length, or all zeros for text without features).
    pub fn embed_text(&self, text: &str) -> Vec<f32> {
        embed_text(text, self.space.dims)
    }
}

/// Splits identifiers on non-alphanumerics, then on camelCase / digit boundaries; lowercases.
/// Returns one entry per identifier with its sub-tokens.
pub fn identifiers(text: &str) -> Vec<Vec<String>> {
    text.split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|s| !s.is_empty())
        .map(split_identifier)
        .filter(|parts| !parts.is_empty())
        .collect()
}

fn split_identifier(ident: &str) -> Vec<String> {
    let mut parts = Vec::new();
    for piece in ident.split('_').filter(|p| !p.is_empty()) {
        let chars: Vec<char> = piece.chars().collect();
        let mut current = String::new();
        for (i, &c) in chars.iter().enumerate() {
            let prev = i.checked_sub(1).and_then(|p| chars.get(p)).copied();
            let next = chars.get(i + 1).copied();
            let boundary = match prev {
                None => false,
                Some(p) => {
                    (p.is_lowercase() && c.is_uppercase())
                        || (p.is_alphabetic() && c.is_numeric())
                        || (p.is_numeric() && c.is_alphabetic())
                        // "HTTPServer" -> "http", "server"
                        || (p.is_uppercase()
                            && c.is_uppercase()
                            && next.is_some_and(char::is_lowercase))
                }
            };
            if boundary && !current.is_empty() {
                parts.push(std::mem::take(&mut current));
            }
            current.extend(c.to_lowercase());
        }
        if !current.is_empty() {
            parts.push(current);
        }
    }
    parts
}

/// Weighted feature counts in sorted order.
fn features(text: &str) -> BTreeMap<String, (f64, u32)> {
    let mut out: BTreeMap<String, (f64, u32)> = BTreeMap::new();
    let mut add = |f: String, w: f64| {
        let e = out.entry(f).or_insert((w, 0));
        e.1 += 1;
    };
    let idents = identifiers(text);
    let flat: Vec<&String> = idents.iter().flatten().collect();
    for t in &flat {
        add(format!("u:{t}"), 1.0);
    }
    for pair in flat.windows(2) {
        if let [a, b] = pair {
            add(format!("b:{a} {b}"), 1.0);
        }
    }
    for ident in &idents {
        let joined: Vec<char> = ident.concat().chars().collect();
        for tri in joined.windows(3) {
            add(format!("t:{}", tri.iter().collect::<String>()), 0.5);
        }
    }
    out
}

/// The hashing embedding of `text` with `dims` dimensions.
pub fn embed_text(text: &str, dims: u16) -> Vec<f32> {
    let n = usize::from(dims.max(1));
    let mut acc = vec![0f64; n];
    for (feature, (weight, tf)) in features(text) {
        let h = xxh3_64_with_seed(feature.as_bytes(), SEED);
        let index = (h % n as u64) as usize;
        let sign = if h >> 63 == 1 { -1.0 } else { 1.0 };
        if let Some(slot) = acc.get_mut(index) {
            *slot += sign * weight * (1.0 + f64::from(tf).ln());
        }
    }
    let mut v: Vec<f32> = acc.into_iter().map(|x| x as f32).collect();
    l2_normalize(&mut v);
    v
}

#[async_trait]
impl EmbeddingProvider for HashProvider {
    fn space(&self) -> &EmbeddingSpace {
        &self.space
    }

    fn max_batch(&self) -> usize {
        MAX_BATCH
    }

    fn max_input_tokens(&self) -> usize {
        MAX_INPUT_TOKENS
    }

    async fn embed(&self, req: EmbedRequest<'_>) -> Result<EmbedResponse, EmbedError> {
        super::check_input_lengths(req.texts, MAX_INPUT_TOKENS)?;
        let tokens: usize = req.texts.iter().map(|t| estimate_tokens(t)).sum();
        Ok(EmbedResponse {
            vectors: req
                .texts
                .iter()
                .map(|t| embed_text(t, self.space.dims))
                .collect(),
            usage_tokens: u32::try_from(tokens).unwrap_or(u32::MAX),
            latency_ms: 0,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_camel_snake_and_acronyms() {
        assert_eq!(
            identifiers("authorizeUser authorize_user HTTPServer v2Api"),
            vec![
                vec!["authorize".to_owned(), "user".to_owned()],
                vec!["authorize".to_owned(), "user".to_owned()],
                vec!["http".to_owned(), "server".to_owned()],
                vec!["v".to_owned(), "2".to_owned(), "api".to_owned()],
            ]
        );
    }

    #[test]
    fn empty_text_is_zero_vector() {
        assert!(embed_text("  ...  ", 256).iter().all(|x| *x == 0.0));
    }
}
