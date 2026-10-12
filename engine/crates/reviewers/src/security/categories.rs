//! The twelve PRD §43 security responsibilities as a closed category set (REV-S-001).

use std::fmt;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecurityCategory {
    AuthzBypass,
    AuthnRegression,
    InputValidation,
    Injection,
    DataExposure,
    UnsafeDeserialization,
    TrustBoundary,
    CredentialHandling,
    PathTraversal,
    Ssrf,
    InsecureConfig,
    PrivilegeEscalation,
}

impl SecurityCategory {
    pub const ALL: [SecurityCategory; 12] = [
        Self::AuthzBypass,
        Self::AuthnRegression,
        Self::InputValidation,
        Self::Injection,
        Self::DataExposure,
        Self::UnsafeDeserialization,
        Self::TrustBoundary,
        Self::CredentialHandling,
        Self::PathTraversal,
        Self::Ssrf,
        Self::InsecureConfig,
        Self::PrivilegeEscalation,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AuthzBypass => "authz_bypass",
            Self::AuthnRegression => "authn_regression",
            Self::InputValidation => "input_validation",
            Self::Injection => "injection",
            Self::DataExposure => "data_exposure",
            Self::UnsafeDeserialization => "unsafe_deserialization",
            Self::TrustBoundary => "trust_boundary",
            Self::CredentialHandling => "credential_handling",
            Self::PathTraversal => "path_traversal",
            Self::Ssrf => "ssrf",
            Self::InsecureConfig => "insecure_config",
            Self::PrivilegeEscalation => "privilege_escalation",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.as_str() == s)
    }
}

impl fmt::Display for SecurityCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Where untrusted data enters (`trust_boundary.source`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrustSource {
    HttpBody,
    HttpQuery,
    HttpHeader,
    QueuePayload,
    Env,
    Db,
    ExternalApi,
}

/// The control a finding says is missing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MissingControl {
    Guard,
    ValidationPipe,
    Parameterization,
    Encoding,
    Allowlist,
    OwnershipCheck,
}
