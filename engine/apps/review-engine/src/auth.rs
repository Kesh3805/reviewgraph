//! ServiceAuth for the internal API (API-005, Rust half).
//!
//! The wire format is the one `apps/api/src/internal/service-token.ts` signs: an HS256 JWT of at
//! most 60 s, with the key chosen by `kid`. The golden vectors in
//! `packages/contracts/fixtures/service-token/` pin both languages to the same bytes.
//!
//! Every route except `health` runs behind [`require_service_auth`], which checks signature,
//! audience, expiry (30 s skew), lifetime, issuer, the `graph:read` scope and a one-time `jti`.
//! Handlers then compare the `repo` claim with the repository in the path ([`Claims::authorize_repo`]).

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex};

use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::Response;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use base64::Engine as _;
use hmac::{Hmac, Mac};
use serde::Deserialize;
use sha2::Sha256;
use uuid::Uuid;

use crate::error::ApiError;
use crate::state::AppState;

type HmacSha256 = Hmac<Sha256>;

/// The audience this service accepts.
pub const AUDIENCE: &str = "rg-engine";
/// The scope every route needs.
pub const SCOPE_GRAPH_READ: &str = "graph:read";
pub const MAX_TOKEN_LIFETIME_SECONDS: u64 = 60;
pub const CLOCK_SKEW_SECONDS: u64 = 30;
/// Replay-cache retention: lifetime plus skew on both sides, rounded up.
pub const JTI_TTL_SECONDS: u64 = 120;
pub const MIN_KEY_BYTES: usize = 32;
const ISSUERS: [&str; 3] = ["rg-api", "rg-worker", "rg-engine"];
const KNOWN_SCOPES: [&str; 2] = ["clone-credentials", "graph:read"];

/// Why a token was rejected. For logs and metrics only: the response never carries it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthFailure {
    MissingToken,
    Malformed,
    UnknownKid,
    BadSignature,
    Expired,
    NotYetValid,
    WrongAudience,
    WrongIssuer,
    LifetimeTooLong,
    InvalidClaims,
    Replayed,
}

impl AuthFailure {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MissingToken => "missing_token",
            Self::Malformed => "malformed",
            Self::UnknownKid => "unknown_kid",
            Self::BadSignature => "bad_signature",
            Self::Expired => "expired",
            Self::NotYetValid => "not_yet_valid",
            Self::WrongAudience => "wrong_audience",
            Self::WrongIssuer => "wrong_issuer",
            Self::LifetimeTooLong => "lifetime_too_long",
            Self::InvalidClaims => "invalid_claims",
            Self::Replayed => "replayed",
        }
    }
}

/// Ordered key ring: every key verifies, so a rotation can overlap two keys.
#[derive(Clone)]
pub struct ServiceKeyRing {
    keys: Vec<(String, Vec<u8>)>,
}

impl fmt::Debug for ServiceKeyRing {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Key material never reaches a log line.
        f.debug_struct("ServiceKeyRing")
            .field(
                "kids",
                &self.keys.iter().map(|(kid, _)| kid).collect::<Vec<_>>(),
            )
            .finish()
    }
}

impl ServiceKeyRing {
    /// Parses `kid1:base64,kid2:base64`. Error messages never echo key material.
    pub fn parse(spec: &str) -> Result<Self, String> {
        let mut keys: Vec<(String, Vec<u8>)> = Vec::new();
        for entry in spec.split(',') {
            let entry = entry.trim();
            if entry.is_empty() {
                continue;
            }
            let (kid, secret) = entry
                .split_once(':')
                .filter(|(kid, _)| !kid.is_empty())
                .ok_or_else(|| "service key entries must look like kid:base64".to_owned())?;
            if !kid
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
            {
                return Err("service key kid has invalid characters".to_owned());
            }
            let secret = STANDARD
                .decode(secret)
                .map_err(|_| format!("service key \"{kid}\" is not valid base64"))?;
            if secret.len() < MIN_KEY_BYTES {
                return Err(format!(
                    "service key \"{kid}\" must be at least {MIN_KEY_BYTES} bytes"
                ));
            }
            if keys.iter().any(|(existing, _)| existing == kid) {
                return Err(format!("duplicate service key kid \"{kid}\""));
            }
            keys.push((kid.to_owned(), secret));
        }
        if keys.is_empty() {
            return Err("at least one service key is required".to_owned());
        }
        Ok(Self { keys })
    }

    /// A ring with one raw secret, the `SERVICE_JWT_SECRET` fallback (kid `default`).
    pub fn single(kid: &str, secret: &[u8]) -> Result<Self, String> {
        if secret.len() < MIN_KEY_BYTES {
            return Err(format!(
                "service key must be at least {MIN_KEY_BYTES} bytes"
            ));
        }
        Ok(Self {
            keys: vec![(kid.to_owned(), secret.to_vec())],
        })
    }

    fn find(&self, kid: &str) -> Option<&[u8]> {
        self.keys
            .iter()
            .find(|(k, _)| k == kid)
            .map(|(_, secret)| secret.as_slice())
    }

    fn signing(&self) -> Option<&(String, Vec<u8>)> {
        self.keys.first()
    }
}

#[derive(Debug, Deserialize)]
struct Header {
    alg: String,
    kid: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawClaims {
    iss: Option<String>,
    aud: Option<serde_json::Value>,
    sub: Option<String>,
    #[serde(default)]
    scope: Vec<String>,
    org: Option<String>,
    repo: Option<String>,
    iat: Option<u64>,
    exp: Option<u64>,
    jti: Option<String>,
}

/// The verified claims of a service token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Claims {
    pub iss: String,
    pub sub: String,
    pub scope: Vec<String>,
    pub org: Option<Uuid>,
    pub repo: Option<Uuid>,
    pub iat: u64,
    pub exp: u64,
    pub jti: String,
}

impl Claims {
    pub fn has_scope(&self, scope: &str) -> bool {
        self.scope.iter().any(|s| s == scope)
    }

    /// The tenant scope of the token: both claims are required for a graph query, and the
    /// repository in the path must be the one the token was minted for.
    ///
    /// # Errors
    ///
    /// [`ApiError::Forbidden`] when a claim is missing or the repository differs.
    pub fn authorize_repo(&self, repo: Uuid) -> Result<(Uuid, Uuid), ApiError> {
        match (self.org, self.repo) {
            (Some(org), Some(claimed)) if claimed == repo => Ok((org, repo)),
            (Some(_), Some(_)) => Err(ApiError::Forbidden("repo_claim_mismatch")),
            _ => Err(ApiError::Forbidden("missing_tenant_claims")),
        }
    }
}

fn b64_json<T: for<'de> Deserialize<'de>>(segment: &str) -> Result<T, AuthFailure> {
    let bytes = URL_SAFE_NO_PAD
        .decode(segment)
        .map_err(|_| AuthFailure::Malformed)?;
    serde_json::from_slice(&bytes).map_err(|_| AuthFailure::Malformed)
}

/// Verifies signature, audience, expiry (30 s skew), lifetime bound and claim shape.
///
/// `now` is Unix seconds. The replay check is separate ([`JtiCache::claim`]) because it has a
/// side effect.
///
/// # Errors
///
/// The [`AuthFailure`] describing the first check that failed.
pub fn verify_token(
    ring: &ServiceKeyRing,
    token: &str,
    audience: &str,
    now: u64,
) -> Result<Claims, AuthFailure> {
    let mut parts = token.split('.');
    let (Some(header_b64), Some(payload_b64), Some(signature_b64), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(AuthFailure::Malformed);
    };
    let header: Header = b64_json(header_b64)?;
    if header.alg != "HS256" {
        return Err(AuthFailure::Malformed);
    }
    let secret = header
        .kid
        .as_deref()
        .and_then(|kid| ring.find(kid))
        .ok_or(AuthFailure::UnknownKid)?;
    let signature = URL_SAFE_NO_PAD
        .decode(signature_b64)
        .map_err(|_| AuthFailure::Malformed)?;
    let mut mac = HmacSha256::new_from_slice(secret).map_err(|_| AuthFailure::Malformed)?;
    mac.update(header_b64.as_bytes());
    mac.update(b".");
    mac.update(payload_b64.as_bytes());
    // `verify_slice` compares in constant time.
    mac.verify_slice(&signature)
        .map_err(|_| AuthFailure::BadSignature)?;

    let raw: RawClaims = b64_json(payload_b64)?;
    let (Some(iss), Some(sub), Some(iat), Some(exp), Some(jti), Some(aud)) =
        (raw.iss, raw.sub, raw.iat, raw.exp, raw.jti, raw.aud)
    else {
        return Err(AuthFailure::InvalidClaims);
    };
    let audience_ok = match &aud {
        serde_json::Value::String(single) => single == audience,
        serde_json::Value::Array(many) => many.iter().any(|v| v.as_str() == Some(audience)),
        _ => false,
    };
    if !audience_ok {
        return Err(AuthFailure::WrongAudience);
    }
    if now > exp.saturating_add(CLOCK_SKEW_SECONDS) {
        return Err(AuthFailure::Expired);
    }
    if !ISSUERS.contains(&iss.as_str()) {
        return Err(AuthFailure::WrongIssuer);
    }
    if sub.is_empty()
        || jti.is_empty()
        || !raw.scope.iter().all(|s| KNOWN_SCOPES.contains(&s.as_str()))
    {
        return Err(AuthFailure::InvalidClaims);
    }
    let parse_uuid = |value: Option<String>| -> Result<Option<Uuid>, AuthFailure> {
        value
            .map(|v| Uuid::parse_str(&v).map_err(|_| AuthFailure::InvalidClaims))
            .transpose()
    };
    let org = parse_uuid(raw.org)?;
    let repo = parse_uuid(raw.repo)?;
    if exp.saturating_sub(iat) > MAX_TOKEN_LIFETIME_SECONDS {
        return Err(AuthFailure::LifetimeTooLong);
    }
    if iat > now.saturating_add(CLOCK_SKEW_SECONDS) {
        return Err(AuthFailure::NotYetValid);
    }
    Ok(Claims {
        iss,
        sub,
        scope: raw.scope,
        org,
        repo,
        iat,
        exp,
        jti,
    })
}

/// Signs a token with the ring's first key (HS256). The engine only verifies in production; this
/// exists for tests and for the cross-language golden vectors.
///
/// # Errors
///
/// `Err` when the ring is empty or the claims do not serialize.
pub fn sign_token(ring: &ServiceKeyRing, claims: &serde_json::Value) -> Result<String, String> {
    let (kid, secret) = ring.signing().ok_or("empty key ring")?;
    let header = serde_json::json!({ "alg": "HS256", "typ": "JWT", "kid": kid });
    let encode = |value: &serde_json::Value| -> Result<String, String> {
        let bytes = serde_json::to_vec(value).map_err(|e| e.to_string())?;
        Ok(URL_SAFE_NO_PAD.encode(bytes))
    };
    let signing_input = format!("{}.{}", encode(&header)?, encode(claims)?);
    let mut mac = HmacSha256::new_from_slice(secret).map_err(|e| e.to_string())?;
    mac.update(signing_input.as_bytes());
    let signature = URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes());
    Ok(format!("{signing_input}.{signature}"))
}

/// One-time `jti` cache: an in-memory, size-bounded replay guard.
#[derive(Debug)]
pub struct JtiCache {
    capacity: usize,
    seen: Mutex<HashMap<String, u64>>,
}

impl JtiCache {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            seen: Mutex::new(HashMap::new()),
        }
    }

    /// Resolves `false` when `jti` was already seen and has not expired.
    pub fn claim(&self, jti: &str, now: u64) -> bool {
        let Ok(mut seen) = self.seen.lock() else {
            // A poisoned lock means a panic elsewhere: fail closed.
            return false;
        };
        if let Some(expiry) = seen.get(jti) {
            if *expiry > now {
                return false;
            }
        }
        if seen.len() >= self.capacity {
            seen.retain(|_, expiry| *expiry > now);
            if seen.len() >= self.capacity {
                // Still full of live ids: drop the entry closest to expiry.
                if let Some(oldest) = seen
                    .iter()
                    .min_by_key(|(_, expiry)| **expiry)
                    .map(|(k, _)| k.clone())
                {
                    seen.remove(&oldest);
                }
            }
        }
        seen.insert(jti.to_owned(), now.saturating_add(JTI_TTL_SECONDS));
        true
    }
}

/// The wall clock in Unix seconds.
pub fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// axum middleware: authenticates the bearer token and stores the [`Claims`] in the request
/// extensions. A rejected token is a bare 401 (the reason is only logged); a valid token without
/// the `graph:read` scope is a 403.
pub async fn require_service_auth(
    State(state): State<AppState>,
    mut request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let token = request
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let Some(token) = token else {
        tracing::warn!(
            reason = AuthFailure::MissingToken.as_str(),
            "service auth rejected"
        );
        return Err(ApiError::Unauthorized);
    };
    let now = unix_now();
    let claims = verify_token(state.keys(), token, AUDIENCE, now).map_err(|reason| {
        tracing::warn!(reason = reason.as_str(), "service auth rejected");
        ApiError::Unauthorized
    })?;
    if !state.jti().claim(&claims.jti, now) {
        tracing::warn!(
            reason = AuthFailure::Replayed.as_str(),
            "service auth rejected"
        );
        return Err(ApiError::Unauthorized);
    }
    if !claims.has_scope(SCOPE_GRAPH_READ) {
        return Err(ApiError::Forbidden("missing_scope"));
    }
    request.extensions_mut().insert(Arc::new(claims));
    Ok(next.run(request).await)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &[u8] = b"golden-vector-secret-0123456789ab";

    fn ring() -> ServiceKeyRing {
        ServiceKeyRing::single("k1", SECRET).unwrap()
    }

    fn claims(now: u64) -> serde_json::Value {
        serde_json::json!({
            "iss": "rg-api", "aud": AUDIENCE, "sub": "api-1", "scope": ["graph:read"],
            "org": "0197a1c2-3d4e-7f50-8a6b-7c8d9e0f1a2b",
            "repo": "0197a1c2-3d4e-7f50-8a6b-7c8d9e0f1a2c",
            "iat": now, "exp": now + 60, "jti": "j-1",
        })
    }

    #[test]
    fn round_trip_and_golden_vector() {
        let token = sign_token(&ring(), &claims(1_700_000_000)).unwrap();
        let verified = verify_token(&ring(), &token, AUDIENCE, 1_700_000_000).unwrap();
        assert_eq!(verified.sub, "api-1");

        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../packages/contracts/fixtures/service-token/ts-signed.json"
        ))
        .unwrap();
        let token = fixture["token"].as_str().unwrap();
        let now = fixture["now"].as_u64().unwrap();
        let claims = verify_token(&ring(), token, "rg-api", now).unwrap();
        assert_eq!(claims.jti, "golden-jti-0001");
        assert_eq!(claims.scope, vec!["clone-credentials".to_owned()]);

        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../packages/contracts/fixtures/service-token/rust-signed.json"
        ))
        .unwrap();
        let claims = verify_token(
            &ring(),
            fixture["token"].as_str().unwrap(),
            "rg-api",
            fixture["now"].as_u64().unwrap(),
        )
        .unwrap();
        assert_eq!(claims.sub, "worker-1");
    }

    #[test]
    fn rejects_bad_tokens() {
        let now = 1_700_000_000;
        let good = sign_token(&ring(), &claims(now)).unwrap();
        let other = ServiceKeyRing::single("k1", b"another-secret-0123456789abcdefgh").unwrap();
        assert_eq!(
            verify_token(&other, &good, AUDIENCE, now),
            Err(AuthFailure::BadSignature)
        );
        assert_eq!(
            verify_token(&ring(), &good, "rg-api", now),
            Err(AuthFailure::WrongAudience)
        );
        assert_eq!(
            verify_token(&ring(), &good, AUDIENCE, now + 61 + CLOCK_SKEW_SECONDS),
            Err(AuthFailure::Expired)
        );
        assert_eq!(
            verify_token(&ring(), &good, AUDIENCE, now - 31),
            Err(AuthFailure::NotYetValid)
        );
        assert_eq!(
            verify_token(&ring(), "a.b", AUDIENCE, now),
            Err(AuthFailure::Malformed)
        );
        let mut long = claims(now);
        long["exp"] = serde_json::json!(now + 61);
        let token = sign_token(&ring(), &long).unwrap();
        assert_eq!(
            verify_token(&ring(), &token, AUDIENCE, now),
            Err(AuthFailure::LifetimeTooLong)
        );
        let mut tampered = good.clone();
        tampered.push('x');
        assert!(verify_token(&ring(), &tampered, AUDIENCE, now).is_err());
    }

    #[test]
    fn jti_is_single_use_until_expiry() {
        let cache = JtiCache::new(2);
        assert!(cache.claim("a", 100));
        assert!(!cache.claim("a", 101));
        assert!(cache.claim("a", 100 + JTI_TTL_SECONDS));
        assert!(cache.claim("b", 100));
        assert!(cache.claim("c", 100));
        assert!(cache.seen.lock().unwrap().len() <= 2);
    }

    #[test]
    fn key_ring_parsing() {
        let b64 = STANDARD.encode(SECRET);
        assert!(ServiceKeyRing::parse(&format!("k1:{b64},k2:{b64}")).is_ok());
        assert!(ServiceKeyRing::parse(&format!("k1:{b64},k1:{b64}")).is_err());
        assert!(ServiceKeyRing::parse("k1:c2hvcnQ=").is_err());
        assert!(ServiceKeyRing::parse("").is_err());
        let debug = format!("{:?}", ring());
        assert!(!debug.contains("golden"));
    }

    #[test]
    fn repo_claim_must_match_the_path() {
        let claims = verify_token(
            &ring(),
            &sign_token(&ring(), &claims(1_700_000_000)).unwrap(),
            AUDIENCE,
            1_700_000_000,
        )
        .unwrap();
        let repo = claims.repo.unwrap();
        assert!(claims.authorize_repo(repo).is_ok());
        assert!(claims.authorize_repo(Uuid::nil()).is_err());
    }
}
