//! TypeORM adapter: entities, columns, relations (NEST-005) and repository access heuristics.

mod access;
mod entity;

use analysis_ir::AnalyzerConfig;

use super::FrameworkCtx;

pub const ADAPTER: &str = "typeorm";
pub const PACKAGE: &str = "typeorm";

/// TypeORM is in the repository signals, or the file imports `typeorm` / `@nestjs/typeorm`.
pub fn detect(ctx: &FrameworkCtx<'_>, cfg: &AnalyzerConfig) -> bool {
    cfg.frameworks.has(ADAPTER)
        || cfg.frameworks.has("@nestjs/typeorm")
        || ctx.imports_from(PACKAGE)
        || ctx.imports_from("@nestjs/typeorm")
}

pub fn run(ctx: &mut FrameworkCtx<'_>) {
    entity::collect(ctx);
    access::collect(ctx);
}

/// TypeORM's default table name for a class: `UserAccount` becomes `user_account`.
pub fn snake_case(name: &str) -> String {
    let chars: Vec<char> = name.chars().collect();
    let mut out = String::new();
    for (i, &c) in chars.iter().enumerate() {
        if c.is_uppercase() {
            if i > 0 {
                let prev_lower = chars[i - 1].is_lowercase();
                let next_lower = chars.get(i + 1).is_some_and(|n| n.is_lowercase());
                if prev_lower || next_lower {
                    out.push('_');
                }
            }
            out.extend(c.to_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::snake_case;

    #[test]
    fn snake_case_matches_typeorm() {
        assert_eq!(snake_case("User"), "user");
        assert_eq!(snake_case("UserAccount"), "user_account");
        assert_eq!(snake_case("XMLParser"), "xml_parser");
        assert_eq!(snake_case("AuditLog2"), "audit_log2");
    }
}
