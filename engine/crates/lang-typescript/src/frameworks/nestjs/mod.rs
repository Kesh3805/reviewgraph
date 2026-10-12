//! NestJS adapter: modules (NEST-001); further facts are added by NEST-002..007.

mod module;

use analysis_ir::AnalyzerConfig;

use super::FrameworkCtx;

pub const ADAPTER: &str = "nestjs";
pub const COMMON: &str = "@nestjs/common";

/// The framework is present in the repository signals, or the file imports `@nestjs/*`.
pub fn detect(ctx: &FrameworkCtx<'_>, cfg: &AnalyzerConfig) -> bool {
    cfg.frameworks.has(ADAPTER)
        || cfg.frameworks.has("@nestjs/core")
        || cfg.frameworks.has(COMMON)
        || ctx.bindings.imports_from_scope("@nestjs/")
}

pub fn run(ctx: &mut FrameworkCtx<'_>) {
    module::collect(ctx);
}
