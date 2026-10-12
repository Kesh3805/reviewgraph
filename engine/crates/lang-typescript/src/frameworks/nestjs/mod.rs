//! NestJS adapter: modules (NEST-001), controllers and routes (NEST-002), dependency injection
//! (NEST-003) and guards, interceptors, pipes (NEST-004).

mod di;
mod middleware;
mod module;
mod routes;
mod tokens;

use analysis_ir::AnalyzerConfig;

use super::FrameworkCtx;

pub use tokens::{injected_properties, InjectedMap, InjectedToken, TokenKind};

pub const ADAPTER: &str = "nestjs";
pub const COMMON: &str = "@nestjs/common";

/// The framework is present in the repository signals, or the file imports `@nestjs/*`.
pub fn detect(ctx: &FrameworkCtx<'_>, cfg: &AnalyzerConfig) -> bool {
    cfg.frameworks.has(ADAPTER)
        || cfg.frameworks.has("@nestjs/core")
        || cfg.frameworks.has(COMMON)
        || ctx.imports_from_scope("@nestjs/")
}

pub fn run(ctx: &mut FrameworkCtx<'_>) {
    module::collect(ctx);
    routes::collect(ctx);
    di::collect(ctx);
    middleware::collect(ctx);
}
