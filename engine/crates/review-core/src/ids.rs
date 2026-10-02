//! Typed identifiers (DOM-001).
//!
//! Every entity ID is its own type, so passing a [`RepositoryId`] where an
//! [`OrganizationId`] is expected is a compile error rather than a tenant-isolation bug:
//!
//! ```compile_fail
//! use review_core::ids::{OrganizationId, RepositoryId};
//! fn takes_org(_: OrganizationId) {}
//! takes_org(RepositoryId::new());
//! ```
//!
//! `review-core` is sqlx-free. Persistence crates bind `id.as_uuid()` and decode rows with
//! `#[sqlx(try_from = "Uuid")]`, which the `From<Uuid>` impls make possible.

use std::fmt;
use std::str::FromStr;

use schemars::gen::SchemaGenerator;
use schemars::schema::Schema;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::CoreError;

macro_rules! uuid_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, JsonSchema)]
        #[serde(transparent)]
        pub struct $name(Uuid);

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let s = String::deserialize(deserializer)?;
                s.parse().map_err(serde::de::Error::custom)
            }
        }

        impl $name {
            /// Generates a new time-ordered (UUIDv7) identifier. Call once, at entity creation.
            #[allow(clippy::new_without_default)]
            pub fn new() -> Self {
                Self(Uuid::now_v7())
            }

            pub const fn from_uuid(u: Uuid) -> Self {
                Self(u)
            }

            pub const fn as_uuid(&self) -> &Uuid {
                &self.0
            }

            pub const fn into_uuid(self) -> Uuid {
                self.0
            }
        }

        impl From<Uuid> for $name {
            fn from(u: Uuid) -> Self {
                Self(u)
            }
        }

        impl From<$name> for Uuid {
            fn from(id: $name) -> Uuid {
                id.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.0.hyphenated())
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}({})", stringify!($name), self.0.hyphenated())
            }
        }

        impl FromStr for $name {
            type Err = CoreError;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                let invalid = |reason: String| CoreError::InvalidId {
                    kind: stringify!($name),
                    reason,
                };
                if s.len() != 36 {
                    return Err(invalid(format!(
                        "expected a hyphenated UUID of 36 characters, got {}",
                        s.len()
                    )));
                }
                Uuid::parse_str(s)
                    .map(Self)
                    .map_err(|e| invalid(e.to_string()))
            }
        }
    };
}

uuid_id!(
    /// Tenant boundary.
    OrganizationId
);
uuid_id!(
    /// A person or service account.
    UserId
);
uuid_id!(
    /// A registered source repository.
    RepositoryId
);
uuid_id!(
    /// An immutable repository snapshot at one commit.
    SnapshotId
);
uuid_id!(
    /// One version of one source file.
    FileVersionId
);
uuid_id!(
    /// A pull request as known to ReviewGraph.
    PullRequestId
);
uuid_id!(
    /// One review of one pull-request head.
    ReviewRunId
);
uuid_id!(
    /// One reviewer execution inside a review run.
    ReviewerRunId
);
uuid_id!(
    /// A finding proposed by a reviewer, before verification.
    CandidateFindingId
);
uuid_id!(
    /// A finding that passed verification.
    VerifiedFindingId
);
uuid_id!(
    /// A finding that was published to the provider.
    PublishedFindingId
);

/// Opaque canonical symbol identifier, for example
/// `ts:src/auth/auth.service#AuthService.authorize/method`.
///
/// There is deliberately no `FromStr`; SID-001 adds `SymbolId::parse`.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct SymbolId(String);

impl SymbolId {
    /// Wraps an already canonical string. Only SID-001's parser and storage decoding may call
    /// this; everything else must obtain a `SymbolId` from those.
    pub fn from_canonical_unchecked(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SymbolId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Debug for SymbolId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SymbolId({})", self.0)
    }
}

/// 128-bit storage key of a [`SymbolId`]: `blake3(symbol_id)[..16]` (ADR-005). Canonical text
/// form is 32 lowercase hex characters.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SymbolKey([u8; 16]);

impl SymbolKey {
    pub fn of(id: &SymbolId) -> SymbolKey {
        let hash = blake3::hash(id.as_str().as_bytes());
        let mut key = [0u8; 16];
        key.copy_from_slice(&hash.as_bytes()[..16]);
        SymbolKey(key)
    }

    pub const fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    pub const fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }
}

impl fmt::Display for SymbolKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&hex::encode(self.0))
    }
}

impl fmt::Debug for SymbolKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SymbolKey({self})")
    }
}

impl FromStr for SymbolKey {
    type Err = CoreError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let invalid = |reason: &str| CoreError::InvalidId {
            kind: "SymbolKey",
            reason: reason.to_owned(),
        };
        if s.len() != 32 {
            return Err(invalid("expected 32 lowercase hex characters"));
        }
        if !s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
            return Err(invalid("only lowercase hex characters are allowed"));
        }
        let mut key = [0u8; 16];
        hex::decode_to_slice(s, &mut key).map_err(|e| invalid(&e.to_string()))?;
        Ok(Self(key))
    }
}

impl Serialize for SymbolKey {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for SymbolKey {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for SymbolKey {
    fn schema_name() -> String {
        "SymbolKey".to_owned()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        crate::schema::string_pattern("SymbolKey", "^[0-9a-f]{32}$")
    }
}

/// Git commit SHA: 40 (SHA-1) or 64 (SHA-256) hex characters, stored lowercase.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CommitSha(String);

impl CommitSha {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// First 12 characters, for logs and UI.
    pub fn short(&self) -> &str {
        &self.0[..12]
    }
}

impl FromStr for CommitSha {
    type Err = CoreError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let valid_len = s.len() == 40 || s.len() == 64;
        if !valid_len || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(CoreError::InvalidCommitSha(s.chars().take(80).collect()));
        }
        Ok(Self(s.to_ascii_lowercase()))
    }
}

impl fmt::Display for CommitSha {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Debug for CommitSha {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "CommitSha({})", self.0)
    }
}

impl Serialize for CommitSha {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for CommitSha {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for CommitSha {
    fn schema_name() -> String {
        "CommitSha".to_owned()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        crate::schema::string_pattern("CommitSha", "^([0-9a-f]{40}|[0-9a-f]{64})$")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    macro_rules! roundtrip {
        ($($t:ident),+) => {$({
            let id = $t::new();
            let shown = id.to_string();
            assert_eq!(shown.len(), 36);
            assert_eq!(shown.parse::<$t>().unwrap(), id);
            let json = serde_json::to_string(&id).unwrap();
            assert_eq!(json, format!("\"{shown}\""));
            assert_eq!(serde_json::from_str::<$t>(&json).unwrap(), id);
            assert_eq!(format!("{id:?}"), format!("{}({shown})", stringify!($t)));
            assert_eq!(Uuid::from(id), *id.as_uuid());
            assert_eq!($t::from(id.into_uuid()), id);
        })+};
    }

    #[test]
    fn uuid_ids_roundtrip_display_fromstr_serde() {
        roundtrip!(
            OrganizationId,
            UserId,
            RepositoryId,
            SnapshotId,
            FileVersionId,
            PullRequestId,
            ReviewRunId,
            ReviewerRunId,
            CandidateFindingId,
            VerifiedFindingId,
            PublishedFindingId
        );
    }

    #[test]
    fn uuid_ids_reject_malformed() {
        for bad in [
            "",
            "not-a-uuid",
            "0190f0c2e8a77c8e8a1b2c3d4e5f6a7b",
            "{0190f0c2-e8a7-7c8e-8a1b-2c3d4e5f6a7b}",
            "0190f0c2-e8a7-7c8e-8a1b-2c3d4e5f6a7z",
        ] {
            assert!(bad.parse::<RepositoryId>().is_err(), "{bad}");
            assert!(
                serde_json::from_str::<RepositoryId>(&format!("\"{bad}\"")).is_err(),
                "{bad}"
            );
        }
        match "x".parse::<ReviewRunId>() {
            Err(CoreError::InvalidId { kind, .. }) => assert_eq!(kind, "ReviewRunId"),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn new_ids_are_time_ordered_and_unique() {
        let a = SnapshotId::new();
        let b = SnapshotId::new();
        assert_ne!(a, b);
        assert_eq!(a.as_uuid().get_version_num(), 7);
    }

    const GOLDEN_ID: &str = "ts:src/auth/auth.service#AuthService.authorize/method";

    #[test]
    fn symbol_key_golden() {
        let key = SymbolKey::of(&SymbolId::from_canonical_unchecked(GOLDEN_ID));
        assert_eq!(key.to_string(), "97ac70ec2191e38555c6678614fc4699");
    }

    #[test]
    fn symbol_key_rejects_uppercase_and_wrong_length() {
        let good = "00112233445566778899aabbccddeeff";
        assert!(good.parse::<SymbolKey>().is_ok());
        assert!(good.to_uppercase().parse::<SymbolKey>().is_err());
        assert!(good[..31].parse::<SymbolKey>().is_err());
        assert!(format!("{good}0").parse::<SymbolKey>().is_err());
        assert!("zz112233445566778899aabbccddeeff"
            .parse::<SymbolKey>()
            .is_err());
        assert!(serde_json::from_str::<SymbolKey>("\"00112233445566778899AABBCCDDEEFF\"").is_err());
    }

    #[test]
    fn symbol_id_serializes_transparently() {
        let id = SymbolId::from_canonical_unchecked(GOLDEN_ID);
        assert_eq!(
            serde_json::to_string(&id).unwrap(),
            format!("\"{GOLDEN_ID}\"")
        );
        assert_eq!(id.as_str(), GOLDEN_ID);
    }

    #[test]
    fn commit_sha_accepts_sha1_and_sha256_rejects_other() {
        let sha1 = "a".repeat(40);
        let sha256 = "0123456789abcdef".repeat(4);
        assert_eq!(sha1.parse::<CommitSha>().unwrap().short(), "aaaaaaaaaaaa");
        assert_eq!(sha256.parse::<CommitSha>().unwrap().as_str(), sha256);
        assert_eq!(
            "ABCDEF0123"
                .repeat(4)
                .parse::<CommitSha>()
                .unwrap()
                .as_str(),
            "abcdef0123".repeat(4)
        );
        for bad in [
            String::new(),
            "a".repeat(39),
            "a".repeat(41),
            "a".repeat(63),
            "g".repeat(40),
            format!("{} ", "a".repeat(39)),
        ] {
            assert!(bad.parse::<CommitSha>().is_err(), "{bad:?}");
        }
        assert!(serde_json::from_str::<CommitSha>("\"HEAD\"").is_err());
        assert_eq!(
            serde_json::to_string(&sha1.parse::<CommitSha>().unwrap()).unwrap(),
            format!("\"{sha1}\"")
        );
    }

    #[test]
    fn ids_are_send_sync() {
        fn assert_send_sync<T: Send + Sync + 'static>() {}
        assert_send_sync::<OrganizationId>();
        assert_send_sync::<ReviewRunId>();
        assert_send_sync::<PublishedFindingId>();
        assert_send_sync::<SymbolId>();
        assert_send_sync::<SymbolKey>();
        assert_send_sync::<CommitSha>();
        assert_send_sync::<CoreError>();
    }

    proptest! {
        #[test]
        fn symbol_key_display_parse_roundtrip(bytes in any::<[u8; 16]>()) {
            let key = SymbolKey::from_bytes(bytes);
            let shown = key.to_string();
            prop_assert_eq!(shown.len(), 32);
            prop_assert_eq!(shown.parse::<SymbolKey>().unwrap(), key);
            let json = serde_json::to_string(&key).unwrap();
            prop_assert_eq!(serde_json::from_str::<SymbolKey>(&json).unwrap(), key);
        }
    }
}
