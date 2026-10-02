//! Source languages known to ReviewGraph.

use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A source language. Wire form is the lowercase name.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    Typescript,
    Javascript,
    Python,
    Java,
    Go,
    Rust,
}

impl Language {
    /// Prefix used in `SymbolId` strings (ADR-005).
    pub const fn id_prefix(self) -> &'static str {
        match self {
            Self::Typescript => "ts",
            Self::Javascript => "js",
            Self::Python => "py",
            Self::Java => "java",
            Self::Go => "go",
            Self::Rust => "rs",
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Typescript => "typescript",
            Self::Javascript => "javascript",
            Self::Python => "python",
            Self::Java => "java",
            Self::Go => "go",
            Self::Rust => "rust",
        }
    }
}

impl fmt::Display for Language {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_form_is_lowercase_and_prefixes_are_stable() {
        let all = [
            (Language::Typescript, "typescript", "ts"),
            (Language::Javascript, "javascript", "js"),
            (Language::Python, "python", "py"),
            (Language::Java, "java", "java"),
            (Language::Go, "go", "go"),
            (Language::Rust, "rust", "rs"),
        ];
        for (lang, name, prefix) in all {
            assert_eq!(serde_json::to_string(&lang).unwrap(), format!("\"{name}\""));
            assert_eq!(lang.to_string(), name);
            assert_eq!(lang.id_prefix(), prefix);
            assert_eq!(
                serde_json::from_str::<Language>(&format!("\"{name}\"")).unwrap(),
                lang
            );
        }
    }
}
