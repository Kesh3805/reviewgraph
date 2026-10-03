//! Infrastructure configuration files (INIT-009).

use rayon::prelude::*;
use review_core::location::RepoPath;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::InitWarning;
use crate::read::BoundedReader;
use crate::walk::{FileClass, FileInventory};

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum InfraKind {
    Dockerfile,
    Compose,
    Terraform,
    Helm,
    Kubernetes,
    Serverless,
    CloudBuild,
    AppEngine,
    Fly,
    Vercel,
    Netlify,
    DockerIgnore,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct InfraFact {
    pub path: RepoPath,
    pub kind: InfraKind,
}

fn by_name(path: &str) -> Option<InfraKind> {
    let name = path.rsplit('/').next().unwrap_or(path);
    let lower = name.to_ascii_lowercase();
    if lower == "dockerfile" || lower.starts_with("dockerfile.") || lower.ends_with(".dockerfile") {
        return Some(InfraKind::Dockerfile);
    }
    if (lower.starts_with("docker-compose") || lower.starts_with("compose."))
        && (lower.ends_with(".yml") || lower.ends_with(".yaml"))
    {
        return Some(InfraKind::Compose);
    }
    if lower.ends_with(".tf") {
        return Some(InfraKind::Terraform);
    }
    match lower.as_str() {
        "chart.yaml" => Some(InfraKind::Helm),
        "serverless.yml" | "serverless.yaml" => Some(InfraKind::Serverless),
        "cloudbuild.yml" | "cloudbuild.yaml" => Some(InfraKind::CloudBuild),
        "app.yaml" => Some(InfraKind::AppEngine),
        "fly.toml" => Some(InfraKind::Fly),
        "vercel.json" => Some(InfraKind::Vercel),
        "netlify.toml" => Some(InfraKind::Netlify),
        ".dockerignore" => Some(InfraKind::DockerIgnore),
        _ => None,
    }
}

/// Infrastructure files, sorted by path. YAML files carrying both `apiVersion:` and `kind:` in
/// their first KiB are Kubernetes manifests.
pub fn detect_infra(
    inv: &FileInventory,
    reader: &BoundedReader,
) -> (Vec<InfraFact>, Vec<InitWarning>) {
    let mut facts: Vec<InfraFact> = inv
        .entries
        .par_iter()
        .filter_map(|entry| {
            let path = entry.path.as_str();
            if let Some(kind) = by_name(path) {
                return Some(InfraFact {
                    path: entry.path.clone(),
                    kind,
                });
            }
            let is_yaml = path.ends_with(".yaml") || path.ends_with(".yml");
            if is_yaml && entry.class == FileClass::Source && !path.starts_with(".github/") {
                let head = reader.read_text(entry, 1024).ok()?;
                if head.contains("apiVersion:") && head.contains("kind:") {
                    return Some(InfraFact {
                        path: entry.path.clone(),
                        kind: InfraKind::Kubernetes,
                    });
                }
            }
            None
        })
        .collect();
    facts.sort_by(|a, b| a.path.cmp(&b.path));
    (facts, Vec::new())
}
