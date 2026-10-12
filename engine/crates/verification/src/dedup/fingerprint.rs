//! Root-cause fingerprint v2 (DED-001): `symbol + category + normalized claim`.
//!
//! The input struct deliberately has no severity, confidence, reviewer, title, line or
//! evidence fields, so including them in the hash is a compile error.

use std::fmt;
use std::str::FromStr;

use super::normalize::normalize_claim;

/// Structured predicate (REV-C-003). `kind == "other"` falls back to claim tokens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Predicate {
    pub kind: String,
    pub subject: String,
}

/// Everything the fingerprint may depend on.
#[derive(Debug, Clone)]
pub struct RootCauseInput<'a> {
    pub category: &'a str,
    pub anchor_symbol_key: Option<&'a str>,
    pub anchor_path: Option<&'a str>,
    pub predicate: Option<Predicate>,
    pub claim: &'a str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FingerprintBasis {
    Predicate,
    Claim,
    PathFallback,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum DedupError {
    #[error("candidate has neither an anchor symbol nor an anchor path")]
    Unanchorable,
    #[error("invalid v2 fingerprint: {0}")]
    InvalidFingerprint(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RootCauseFingerprint(String);

impl RootCauseFingerprint {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for RootCauseFingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for RootCauseFingerprint {
    type Err = DedupError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let hex = s
            .strip_prefix("v2:")
            .ok_or_else(|| DedupError::InvalidFingerprint(s.to_string()))?;
        if hex.len() == 32 && hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
            Ok(Self(s.to_string()))
        } else {
            Err(DedupError::InvalidFingerprint(s.to_string()))
        }
    }
}

/// Claim signature (also used by DED-003 identity keys).
pub fn claim_signature(input: &RootCauseInput<'_>) -> (String, FingerprintBasis) {
    match &input.predicate {
        Some(p) if p.kind != "other" => {
            let subject = normalize_claim(&p.subject).as_text();
            (format!("p:{}\0{}", p.kind, subject), FingerprintBasis::Predicate)
        }
        _ => (
            format!("c:{}", normalize_claim(input.claim).as_text()),
            FingerprintBasis::Claim,
        ),
    }
}

pub fn root_cause_fingerprint(
    input: &RootCauseInput<'_>,
) -> Result<(RootCauseFingerprint, FingerprintBasis), DedupError> {
    let (anchor, path_fallback) = match (input.anchor_symbol_key, input.anchor_path) {
        (Some(k), _) => (k.to_string(), false),
        (None, Some(p)) => (format!("path:{p}"), true),
        (None, None) => return Err(DedupError::Unanchorable),
    };
    let (sig, mut basis) = claim_signature(input);
    if path_fallback {
        basis = FingerprintBasis::PathFallback;
    }
    let preimage = format!("v2\0{}\0{}\0{}", input.category, anchor, sig);
    let hash = blake3::hash(preimage.as_bytes());
    let hex: String = hash.as_bytes()[..16].iter().map(|b| format!("{b:02x}")).collect();
    Ok((RootCauseFingerprint(format!("v2:{hex}")), basis))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input<'a>(claim: &'a str, sym: &'a str) -> RootCauseInput<'a> {
        RootCauseInput {
            category: "security",
            anchor_symbol_key: Some(sym),
            anchor_path: Some("a.ts"),
            predicate: Some(Predicate {
                kind: "auth_check_removed".into(),
                subject: "handler".into(),
            }),
            claim,
        }
    }

    #[test]
    fn stable_under_paraphrase_with_same_predicate() {
        let a = root_cause_fingerprint(&input("authorization removed", "s1")).unwrap().0;
        let b = root_cause_fingerprint(&input("permission validation gone", "s1")).unwrap().0;
        assert_eq!(a, b);
        assert_eq!(a.as_str().len(), 35);
    }

    #[test]
    fn differs_for_symbol_and_category() {
        let a = root_cause_fingerprint(&input("x", "s1")).unwrap().0;
        let b = root_cause_fingerprint(&input("x", "s2")).unwrap().0;
        let mut i = input("x", "s1");
        i.category = "correctness";
        let c = root_cause_fingerprint(&i).unwrap().0;
        assert_ne!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn claim_basis_when_predicate_other() {
        let mut i = input("Claim Text", "s1");
        i.predicate = None;
        let (_, basis) = root_cause_fingerprint(&i).unwrap();
        assert_eq!(basis, FingerprintBasis::Claim);
    }

    #[test]
    fn unanchorable_errors() {
        let mut i = input("x", "s1");
        i.anchor_symbol_key = None;
        i.anchor_path = None;
        assert_eq!(root_cause_fingerprint(&i).unwrap_err(), DedupError::Unanchorable);
    }

    #[test]
    fn parse_rejects_v1_prefix() {
        assert!("v1:0123456789abcdef0123456789abcdef".parse::<RootCauseFingerprint>().is_err());
        assert!("v2:0123456789abcdef0123456789abcdef".parse::<RootCauseFingerprint>().is_ok());
    }
}
