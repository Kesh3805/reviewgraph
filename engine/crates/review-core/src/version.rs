//! Version types for derived data (DOM-003, ADR-015).
//!
//! Concrete version values live with their owners (`codegraph::SCHEMA_VERSION`, the analyzers,
//! the verification stage); this module only defines how versions are represented, validated
//! and serialized.

use std::fmt;
use std::str::FromStr;

use schemars::gen::SchemaGenerator;
use schemars::schema::Schema;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::CoreError;

fn invalid(kind: &'static str, reason: impl Into<String>) -> CoreError {
    CoreError::InvalidVersion {
        kind,
        reason: reason.into(),
    }
}

/// Integer version of the code-graph schema. A change forces a full rebuild.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(transparent)]
pub struct GraphSchemaVersion(pub u32);

/// Integer version of a repository's review profile.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(transparent)]
pub struct ProfileVersion(pub u32);

/// Integer version of the verification stage.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(transparent)]
pub struct VerificationVersion(pub u32);

macro_rules! semver_newtype {
    ($(#[$meta:meta])* $name:ident, $kind:literal) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
        #[serde(transparent)]
        pub struct $name(semver::Version);

        impl $name {
            pub const fn new(major: u64, minor: u64, patch: u64) -> Self {
                Self(semver::Version::new(major, minor, patch))
            }

            pub const fn major(&self) -> u64 {
                self.0.major
            }

            pub fn as_semver(&self) -> &semver::Version {
                &self.0
            }
        }

        impl FromStr for $name {
            type Err = CoreError;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                semver::Version::parse(s)
                    .map(Self)
                    .map_err(|e| invalid($kind, e.to_string()))
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let s = String::deserialize(deserializer)?;
                s.parse().map_err(serde::de::Error::custom)
            }
        }

        impl JsonSchema for $name {
            fn schema_name() -> String {
                stringify!($name).to_owned()
            }

            fn json_schema(_: &mut SchemaGenerator) -> Schema {
                crate::schema::string_pattern(
                    stringify!($name),
                    r"^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(-[0-9A-Za-z.-]+)?(\+[0-9A-Za-z.-]+)?$",
                )
            }
        }
    };
}

semver_newtype!(
    /// Semantic version of a language analyzer.
    ///
    /// A **major** bump means the analyzer's IR output may differ, so that language is re-parsed.
    /// Minor and patch bumps must be IR-compatible by contract and never trigger a re-parse.
    AnalyzerVersion,
    "analyzer version"
);

semver_newtype!(
    /// Semantic version of a reviewer.
    ReviewerVersion,
    "reviewer version"
);

/// Name of a prompt, matching `^[a-z][a-z0-9_-]{0,47}$`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, JsonSchema)]
#[serde(transparent)]
pub struct PromptName(String);

impl PromptName {
    pub fn new(name: impl Into<String>) -> Result<Self, CoreError> {
        let name = name.into();
        let mut chars = name.chars();
        let first_ok = chars.next().is_some_and(|c| c.is_ascii_lowercase());
        let rest_ok =
            chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-');
        if !first_ok || !rest_ok || name.len() > 48 {
            return Err(invalid(
                "prompt name",
                format!("{name:?} does not match ^[a-z][a-z0-9_-]{{0,47}}$"),
            ));
        }
        Ok(Self(name))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PromptName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for PromptName {
    type Err = CoreError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

impl<'de> Deserialize<'de> for PromptName {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Self::new(s).map_err(serde::de::Error::custom)
    }
}

/// A prompt and its revision. Display and serde form: `name/vN`, for example `correctness/v3`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PromptVersion {
    pub prompt: PromptName,
    pub n: u32,
}

impl PromptVersion {
    pub fn new(prompt: PromptName, n: u32) -> Self {
        Self { prompt, n }
    }
}

impl fmt::Display for PromptVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/v{}", self.prompt, self.n)
    }
}

impl FromStr for PromptVersion {
    type Err = CoreError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (name, rev) = s
            .split_once('/')
            .ok_or_else(|| invalid("prompt version", format!("{s:?} is not name/vN")))?;
        let digits = rev
            .strip_prefix('v')
            .filter(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit()))
            .ok_or_else(|| invalid("prompt version", format!("{rev:?} is not vN")))?;
        let n = digits
            .parse::<u32>()
            .map_err(|e| invalid("prompt version", e.to_string()))?;
        Ok(Self {
            prompt: PromptName::new(name)?,
            n,
        })
    }
}

impl Serialize for PromptVersion {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for PromptVersion {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for PromptVersion {
    fn schema_name() -> String {
        "PromptVersion".to_owned()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        crate::schema::string_pattern("PromptVersion", r"^[a-z][a-z0-9_-]{0,47}/v[0-9]+$")
    }
}

/// An embedding provider, model and dimensionality. Vectors from different spaces are never
/// comparable, so each space has its own vector collection (ADR-008).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EmbeddingSpace {
    pub provider: String,
    pub model: String,
    pub dims: u32,
}

impl EmbeddingSpace {
    pub fn new(
        provider: impl Into<String>,
        model: impl Into<String>,
        dims: u32,
    ) -> Result<Self, CoreError> {
        let (provider, model) = (provider.into(), model.into());
        if provider.is_empty() || model.is_empty() {
            return Err(invalid(
                "embedding space",
                "provider and model must be non-empty",
            ));
        }
        if dims == 0 {
            return Err(invalid("embedding space", "dims must be greater than 0"));
        }
        Ok(Self {
            provider,
            model,
            dims,
        })
    }

    /// Qdrant collection name `rg_{provider}_{model}_{dims}_v{n}` (ADR-008). Provider and model
    /// are lowercased and every character outside `[a-z0-9]` becomes `_`, so free text can never
    /// inject path or URL syntax.
    pub fn collection_name(&self, n: u32) -> String {
        fn sanitize(s: &str) -> String {
            s.chars()
                .flat_map(char::to_lowercase)
                .map(|c| {
                    if c.is_ascii_lowercase() || c.is_ascii_digit() {
                        c
                    } else {
                        '_'
                    }
                })
                .collect()
        }
        format!(
            "rg_{}_{}_{}_v{}",
            sanitize(&self.provider),
            sanitize(&self.model),
            self.dims,
            n
        )
    }
}

impl<'de> Deserialize<'de> for EmbeddingSpace {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Raw {
            provider: String,
            model: String,
            dims: u32,
        }
        let raw = Raw::deserialize(deserializer)?;
        Self::new(raw.provider, raw.model, raw.dims).map_err(serde::de::Error::custom)
    }
}

/// Blake3 hash (32 bytes) of the normalized configuration. Wire form: 64 lowercase hex characters.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ConfigHash([u8; 32]);

impl ConfigHash {
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Hashes already-normalized configuration bytes.
    pub fn of(normalized: &[u8]) -> Self {
        Self(*blake3::hash(normalized).as_bytes())
    }

    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Display for ConfigHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&hex::encode(self.0))
    }
}

impl fmt::Debug for ConfigHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ConfigHash({self})")
    }
}

impl FromStr for ConfigHash {
    type Err = CoreError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.len() != 64 || !s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
            return Err(invalid(
                "config hash",
                "expected 64 lowercase hex characters",
            ));
        }
        let mut out = [0u8; 32];
        hex::decode_to_slice(s, &mut out).map_err(|e| invalid("config hash", e.to_string()))?;
        Ok(Self(out))
    }
}

impl Serialize for ConfigHash {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for ConfigHash {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for ConfigHash {
    fn schema_name() -> String {
        "ConfigHash".to_owned()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        crate::schema::string_pattern("ConfigHash", "^[0-9a-f]{64}$")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_version_roundtrip_and_rejects_bad_names() {
        let v: PromptVersion = "correctness/v3".parse().unwrap();
        assert_eq!(v.prompt.as_str(), "correctness");
        assert_eq!(v.n, 3);
        assert_eq!(v.to_string(), "correctness/v3");
        assert_eq!(serde_json::to_string(&v).unwrap(), "\"correctness/v3\"");
        assert_eq!(
            serde_json::from_str::<PromptVersion>("\"correctness/v3\"").unwrap(),
            v
        );

        for bad in [
            "",
            "correctness",
            "correctness/3",
            "correctness/v",
            "correctness/v-1",
            "correctness/v3/x",
            "Correctness/v3",
            "1abc/v1",
            "has space/v1",
            "/v1",
            &format!("{}/v1", "a".repeat(49)),
            "a/v99999999999",
        ] {
            assert!(bad.parse::<PromptVersion>().is_err(), "{bad}");
        }
        assert!(PromptName::new("a".repeat(48)).is_ok());
        assert!(PromptName::new("a-b_c9").is_ok());
        assert!(serde_json::from_str::<PromptName>("\"Bad\"").is_err());
    }

    #[test]
    fn collection_name_sanitizes() {
        let space = EmbeddingSpace::new("Open AI/../x", "text-embedding:3 Large", 3072).unwrap();
        let name = space.collection_name(2);
        assert_eq!(name, "rg_open_ai____x_text_embedding_3_large_3072_v2");
        assert!(name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'));
    }

    #[test]
    fn collection_name_matches_adr_008_example() {
        let space = EmbeddingSpace::new("voyage", "voyage-code-3", 1024).unwrap();
        assert_eq!(space.collection_name(1), "rg_voyage_voyage_code_3_1024_v1");
    }

    #[test]
    fn embedding_space_validates() {
        assert!(EmbeddingSpace::new("", "m", 1).is_err());
        assert!(EmbeddingSpace::new("p", "", 1).is_err());
        assert!(EmbeddingSpace::new("p", "m", 0).is_err());
        assert!(
            serde_json::from_str::<EmbeddingSpace>(r#"{"provider":"p","model":"m","dims":0}"#)
                .is_err()
        );
        assert!(serde_json::from_str::<EmbeddingSpace>(
            r#"{"provider":"p","model":"m","dims":1,"x":1}"#
        )
        .is_err());
        let ok: EmbeddingSpace =
            serde_json::from_str(r#"{"provider":"p","model":"m","dims":8}"#).unwrap();
        assert_eq!(ok.dims, 8);
    }

    #[test]
    fn semver_wrappers_roundtrip() {
        let v: AnalyzerVersion = "1.2.3".parse().unwrap();
        assert_eq!(v, AnalyzerVersion::new(1, 2, 3));
        assert_eq!(v.major(), 1);
        assert_eq!(serde_json::to_string(&v).unwrap(), "\"1.2.3\"");
        assert!("1.2".parse::<AnalyzerVersion>().is_err());
        assert!(serde_json::from_str::<ReviewerVersion>("\"nope\"").is_err());
        assert_eq!(ReviewerVersion::new(0, 1, 0).to_string(), "0.1.0");
    }

    #[test]
    fn config_hash_wire_form() {
        let h = ConfigHash::of(b"normalized config");
        let shown = h.to_string();
        assert_eq!(shown.len(), 64);
        assert_eq!(shown.parse::<ConfigHash>().unwrap(), h);
        assert_eq!(serde_json::to_string(&h).unwrap(), format!("\"{shown}\""));
        assert!(shown.to_uppercase().parse::<ConfigHash>().is_err());
        assert!(shown[..63].parse::<ConfigHash>().is_err());
    }

    #[test]
    fn integer_versions_are_transparent() {
        assert_eq!(serde_json::to_string(&GraphSchemaVersion(7)).unwrap(), "7");
        assert_eq!(
            serde_json::from_str::<ProfileVersion>("2").unwrap(),
            ProfileVersion(2)
        );
        assert_eq!(serde_json::to_string(&VerificationVersion(1)).unwrap(), "1");
    }
}
