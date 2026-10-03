//! Source languages known to ReviewGraph.

use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A language or file format. Wire form is the lowercase name. New variants are additive.
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
    Kotlin,
    Csharp,
    Ruby,
    Php,
    Shell,
    Sql,
    Yaml,
    Json,
    Toml,
    Markdown,
    Html,
    Css,
    Dockerfile,
    Terraform,
    Protobuf,
    Graphql,
    Prisma,
    Other,
}

impl Language {
    /// Every variant, in declaration (and therefore tie-break) order.
    pub const ALL: [Language; 24] = [
        Self::Typescript,
        Self::Javascript,
        Self::Python,
        Self::Java,
        Self::Go,
        Self::Rust,
        Self::Kotlin,
        Self::Csharp,
        Self::Ruby,
        Self::Php,
        Self::Shell,
        Self::Sql,
        Self::Yaml,
        Self::Json,
        Self::Toml,
        Self::Markdown,
        Self::Html,
        Self::Css,
        Self::Dockerfile,
        Self::Terraform,
        Self::Protobuf,
        Self::Graphql,
        Self::Prisma,
        Self::Other,
    ];

    /// Prefix used in `SymbolId` strings (ADR-005).
    pub const fn id_prefix(self) -> &'static str {
        match self {
            Self::Typescript => "ts",
            Self::Javascript => "js",
            Self::Python => "py",
            Self::Java => "java",
            Self::Go => "go",
            Self::Rust => "rs",
            Self::Kotlin => "kt",
            Self::Csharp => "cs",
            Self::Ruby => "rb",
            Self::Php => "php",
            Self::Shell => "sh",
            Self::Sql => "sql",
            Self::Yaml => "yaml",
            Self::Json => "json",
            Self::Toml => "toml",
            Self::Markdown => "md",
            Self::Html => "html",
            Self::Css => "css",
            Self::Dockerfile => "docker",
            Self::Terraform => "tf",
            Self::Protobuf => "proto",
            Self::Graphql => "graphql",
            Self::Prisma => "prisma",
            Self::Other => "other",
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
            Self::Kotlin => "kotlin",
            Self::Csharp => "csharp",
            Self::Ruby => "ruby",
            Self::Php => "php",
            Self::Shell => "shell",
            Self::Sql => "sql",
            Self::Yaml => "yaml",
            Self::Json => "json",
            Self::Toml => "toml",
            Self::Markdown => "markdown",
            Self::Html => "html",
            Self::Css => "css",
            Self::Dockerfile => "dockerfile",
            Self::Terraform => "terraform",
            Self::Protobuf => "protobuf",
            Self::Graphql => "graphql",
            Self::Prisma => "prisma",
            Self::Other => "other",
        }
    }

    /// True for languages that can hold executable program logic, as opposed to data and
    /// markup formats. Used to choose a repository's primary language.
    pub const fn is_programming(self) -> bool {
        !matches!(
            self,
            Self::Json
                | Self::Yaml
                | Self::Toml
                | Self::Markdown
                | Self::Html
                | Self::Css
                | Self::Sql
                | Self::Other
        )
    }
}

/// File dialect within a language: the grammar and module-kind hint for a path.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Dialect {
    Ts,
    Tsx,
    Dts,
    Js,
    Jsx,
    Mjs,
    Cjs,
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
            (Language::Kotlin, "kotlin", "kt"),
            (Language::Csharp, "csharp", "cs"),
            (Language::Ruby, "ruby", "rb"),
            (Language::Php, "php", "php"),
            (Language::Shell, "shell", "sh"),
            (Language::Sql, "sql", "sql"),
            (Language::Yaml, "yaml", "yaml"),
            (Language::Json, "json", "json"),
            (Language::Toml, "toml", "toml"),
            (Language::Markdown, "markdown", "md"),
            (Language::Html, "html", "html"),
            (Language::Css, "css", "css"),
            (Language::Dockerfile, "dockerfile", "docker"),
            (Language::Terraform, "terraform", "tf"),
            (Language::Protobuf, "protobuf", "proto"),
            (Language::Graphql, "graphql", "graphql"),
            (Language::Prisma, "prisma", "prisma"),
            (Language::Other, "other", "other"),
        ];
        assert_eq!(all.len(), Language::ALL.len());
        for (lang, name, prefix) in all {
            assert_eq!(serde_json::to_string(&lang).unwrap(), format!("\"{name}\""));
            assert_eq!(lang.to_string(), name);
            assert_eq!(lang.id_prefix(), prefix);
            assert_eq!(
                serde_json::from_str::<Language>(&format!("\"{name}\"")).unwrap(),
                lang
            );
        }
        let listed: Vec<Language> = all.iter().map(|(l, ..)| *l).collect();
        assert_eq!(listed, Language::ALL.to_vec());
    }

    #[test]
    fn data_formats_are_not_programming_languages() {
        assert!(Language::Typescript.is_programming());
        assert!(!Language::Json.is_programming());
        assert!(!Language::Markdown.is_programming());
        assert!(!Language::Other.is_programming());
    }
}
