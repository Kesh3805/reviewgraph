//! Credential-free remote URLs (INIT-001).
//!
//! `RedactedUrl` has no constructor from a raw string except [`redact`], so a URL that still
//! carries a password or token cannot reach logs, spans or `repository.json`.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Hosting provider guessed from the remote host name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderHint {
    Github,
    Gitlab,
    Bitbucket,
    Azure,
    Other,
    Local,
}

/// A remote URL with user, password, query and fragment removed.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RedactedUrl(String);

impl RedactedUrl {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for RedactedUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "RedactedUrl({:?})", self.0)
    }
}

impl fmt::Display for RedactedUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The outcome of [`redact`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RedactedRemote {
    pub url: RedactedUrl,
    pub provider: ProviderHint,
    /// `host/owner/name`, `.git` stripped. `None` for local paths and unparseable URLs.
    pub slug: Option<String>,
}

fn provider_for(host: &str) -> ProviderHint {
    let host = host.to_ascii_lowercase();
    match host.as_str() {
        "github.com" | "www.github.com" => ProviderHint::Github,
        "gitlab.com" | "www.gitlab.com" => ProviderHint::Gitlab,
        "bitbucket.org" | "www.bitbucket.org" => ProviderHint::Bitbucket,
        "dev.azure.com" | "ssh.dev.azure.com" | "vs-ssh.visualstudio.com" => ProviderHint::Azure,
        _ if host.ends_with(".visualstudio.com") => ProviderHint::Azure,
        _ => ProviderHint::Other,
    }
}

fn clean_path(path: &str) -> &str {
    let path = path.split(['?', '#']).next().unwrap_or(path);
    path.trim_matches('/')
}

fn slug_of(host: &str, path: &str) -> Option<String> {
    let path = clean_path(path);
    let path = path.strip_suffix(".git").unwrap_or(path);
    if path.is_empty() || host.is_empty() {
        return None;
    }
    Some(format!("{}/{}", host.to_ascii_lowercase(), path))
}

/// Removes credentials from a remote URL and extracts the provider hint and slug.
///
/// Rules: http(s)/git URLs lose user and password; `ssh://` URLs keep the user but lose the
/// password; scp-like `git@host:path` keeps `git@`; query and fragment are always removed. A URL
/// that cannot be parsed is replaced by a fixed placeholder, never echoed.
pub fn redact(raw: &str) -> RedactedRemote {
    let raw = raw.trim();
    let Ok(parsed) = gix::url::parse(raw) else {
        return RedactedRemote {
            url: RedactedUrl("<unparseable>".to_owned()),
            provider: ProviderHint::Other,
            slug: None,
        };
    };
    let path = String::from_utf8_lossy(&parsed.path).into_owned();
    let scheme = parsed.scheme.as_str().to_owned();

    if scheme == "file" || parsed.host.is_none() {
        let path = path.split(['?', '#']).next().unwrap_or(&path).to_owned();
        return RedactedRemote {
            url: RedactedUrl(path),
            provider: ProviderHint::Local,
            slug: None,
        };
    }
    let host = parsed.host.clone().unwrap_or_default();
    let provider = provider_for(&host);
    let slug = slug_of(&host, &path);
    let is_http = scheme == "http" || scheme == "https";
    let clean = clean_path(&path);

    let url = if parsed.serialize_alternative_form && scheme == "ssh" {
        let user = parsed.user.as_deref().map(|u| format!("{u}@"));
        format!("{}{}:{}", user.unwrap_or_default(), host, clean)
    } else {
        let user = if is_http {
            String::new()
        } else {
            parsed
                .user
                .as_deref()
                .map(|u| format!("{u}@"))
                .unwrap_or_default()
        };
        let port = parsed.port.map(|p| format!(":{p}")).unwrap_or_default();
        format!("{scheme}://{user}{host}{port}/{clean}")
    };
    RedactedRemote {
        url: RedactedUrl(url),
        provider,
        slug,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Case {
        raw: &'static str,
        url: &'static str,
        provider: ProviderHint,
        slug: Option<&'static str>,
    }

    #[test]
    fn twelve_url_shapes() {
        let cases = [
            Case {
                raw: "https://github.com/acme/widgets.git",
                url: "https://github.com/acme/widgets.git",
                provider: ProviderHint::Github,
                slug: Some("github.com/acme/widgets"),
            },
            Case {
                raw: "https://user:s3cr3t@github.com/acme/widgets.git",
                url: "https://github.com/acme/widgets.git",
                provider: ProviderHint::Github,
                slug: Some("github.com/acme/widgets"),
            },
            Case {
                raw: "https://x-access-token:ghs_TOKEN123@github.com/acme/widgets",
                url: "https://github.com/acme/widgets",
                provider: ProviderHint::Github,
                slug: Some("github.com/acme/widgets"),
            },
            Case {
                raw: "git@github.com:acme/widgets.git",
                url: "git@github.com:acme/widgets.git",
                provider: ProviderHint::Github,
                slug: Some("github.com/acme/widgets"),
            },
            Case {
                raw: "ssh://git@gitlab.com/group/sub/proj.git",
                url: "ssh://git@gitlab.com/group/sub/proj.git",
                provider: ProviderHint::Gitlab,
                slug: Some("gitlab.com/group/sub/proj"),
            },
            Case {
                raw: "ssh://git:pw@bitbucket.org:7999/team/repo.git",
                url: "ssh://git@bitbucket.org:7999/team/repo.git",
                provider: ProviderHint::Bitbucket,
                slug: Some("bitbucket.org/team/repo"),
            },
            Case {
                raw: "https://oauth2:glpat-abc@gitlab.com/group/proj.git?token=zzz#frag",
                url: "https://gitlab.com/group/proj.git",
                provider: ProviderHint::Gitlab,
                slug: Some("gitlab.com/group/proj"),
            },
            Case {
                raw: "https://dev.azure.com/org/project/_git/repo",
                url: "https://dev.azure.com/org/project/_git/repo",
                provider: ProviderHint::Azure,
                slug: Some("dev.azure.com/org/project/_git/repo"),
            },
            Case {
                raw: "https://git.example.com:8443/team/repo.git",
                url: "https://git.example.com:8443/team/repo.git",
                provider: ProviderHint::Other,
                slug: Some("git.example.com/team/repo"),
            },
            Case {
                raw: "git://example.org/pub/repo.git",
                url: "git://example.org/pub/repo.git",
                provider: ProviderHint::Other,
                slug: Some("example.org/pub/repo"),
            },
            Case {
                raw: "/srv/git/repo.git",
                url: "/srv/git/repo.git",
                provider: ProviderHint::Local,
                slug: None,
            },
            Case {
                raw: "file:///srv/git/repo.git",
                url: "/srv/git/repo.git",
                provider: ProviderHint::Local,
                slug: None,
            },
        ];
        for case in cases {
            let got = redact(case.raw);
            assert_eq!(got.url.as_str(), case.url, "url for {}", case.raw);
            assert_eq!(got.provider, case.provider, "provider for {}", case.raw);
            assert_eq!(got.slug.as_deref(), case.slug, "slug for {}", case.raw);
            let rendered = format!("{got:?}");
            for secret in ["s3cr3t", "ghs_TOKEN123", "glpat-abc", "pw@", "token=zzz"] {
                assert!(
                    !rendered.contains(secret),
                    "{secret} leaked for {}",
                    case.raw
                );
            }
        }
    }

    #[test]
    fn unparseable_url_is_not_echoed() {
        let got = redact("https://user:secret@[::bad");
        assert_eq!(got.url.as_str(), "<unparseable>");
        assert!(!format!("{got:?}").contains("secret"));
    }
}
