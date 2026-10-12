//! Per-snapshot lexical identifier index (CTX-004).

pub mod index;
pub mod tokenize;

pub use index::{
    LexicalDoc, LexicalError, LexicalHit, LexicalIndex, LexicalIndexCache, CACHE_CAPACITY,
    DF_CUTOFF, MAX_HITS, MAX_POSTINGS, MIN_SCORE, SUBTOKEN_WEIGHT,
};
pub use tokenize::{tokenize, Tokens, STOPWORDS};
