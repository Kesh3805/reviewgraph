//! BullMQ adapter: queue registrations, `@Processor` consumers and job handlers, and producers
//! (NEST-006). Job payloads and Redis options are never stored, only option key names.

mod consumer;
mod producer;
mod registration;

use analysis_ir::{AnalyzerConfig, IrExpr};

use super::util::expr_text;
use super::{FrameworkCtx, CONF_BOUND, CONF_UNBOUND};

pub const ADAPTER: &str = "bullmq";

/// Packages that export the BullMQ / Bull API.
pub const PACKAGES: [&str; 4] = ["@nestjs/bullmq", "@nestjs/bull", "bullmq", "bull"];

/// BullMQ is in the repository signals, or the file imports one of its packages.
pub fn detect(ctx: &FrameworkCtx<'_>, cfg: &AnalyzerConfig) -> bool {
    PACKAGES
        .iter()
        .any(|p| cfg.frameworks.has(p) || ctx.imports_from(p))
}

pub fn run(ctx: &mut FrameworkCtx<'_>) {
    registration::collect(ctx);
    consumer::collect(ctx);
    producer::collect(ctx);
}

/// Confidence that `name` is the `wanted` export of one of the BullMQ packages. A default import
/// (`import Queue from 'bull'`) counts for the class it is named after.
pub fn origin(ctx: &FrameworkCtx<'_>, name: &str, wanted: &str) -> Option<f32> {
    let mut parts = name.split('.');
    let root = parts.next()?;
    let rest: Vec<&str> = parts.collect();
    match (ctx.binding(root), rest.as_slice()) {
        (Some(b), []) if PACKAGES.contains(&b.specifier.as_str()) => (b.imported == wanted
            || (b.imported == "default" && root == wanted))
            .then_some(CONF_BOUND),
        (Some(b), [last]) if PACKAGES.contains(&b.specifier.as_str()) && b.imported == "*" => {
            (*last == wanted).then_some(CONF_BOUND)
        }
        (None, []) if root == wanted => Some(CONF_UNBOUND),
        _ => None,
    }
}

/// A resolved queue name: `(name, dynamic, confidence)`. Same-file constants and enum members
/// resolve exactly; anything else becomes `{<expr>}` at confidence 0.5.
pub fn queue_name(ctx: &FrameworkCtx<'_>, expr: Option<&IrExpr>) -> (String, bool, f32) {
    match expr {
        None => ("{unknown}".to_owned(), true, 0.5),
        Some(e) => match ctx.const_str(e) {
            Some(name) => {
                let conf = if matches!(e, IrExpr::Str(_) | IrExpr::Template { .. }) {
                    0.95
                } else {
                    0.9
                };
                (name, false, conf)
            }
            None => (format!("{{{}}}", expr_text(e)), true, 0.5),
        },
    }
}
