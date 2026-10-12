//! Inference of the repository's structure from the graph (PROF-002): roles, layers, the
//! observed layer dependency matrix and module boundaries.

pub mod architecture;
pub mod roles;

pub use architecture::{infer_architecture, ArchitectureInference};
pub use roles::{classify, Role, RoleAssignment, RoleBasis};

/// The module a file belongs to: `src/modules/<name>` or `src/<name>` below the (first) `src`
/// directory, otherwise the first directory. Empty for a file at the repository root.
pub fn module_of(path: &str) -> String {
    let mut segments: Vec<&str> = path.split('/').collect();
    segments.pop();
    if segments.is_empty() {
        return String::new();
    }
    if let Some(i) = segments.iter().position(|s| *s == "src") {
        let rest = &segments[i + 1..];
        let take = match rest.first() {
            Some(&"modules") if rest.len() >= 2 => 2,
            Some(_) => 1,
            None => 0,
        };
        return segments[..=i + take].join("/");
    }
    segments[0].to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_of_table() {
        assert_eq!(module_of("src/users/users.controller.ts"), "src/users");
        assert_eq!(module_of("src/modules/users/a/b.ts"), "src/modules/users");
        assert_eq!(module_of("src/main.ts"), "src");
        assert_eq!(module_of("apps/api/src/orders/x.ts"), "apps/api/src/orders");
        assert_eq!(module_of("libs/core/a.ts"), "libs");
        assert_eq!(module_of("index.ts"), "");
    }
}
