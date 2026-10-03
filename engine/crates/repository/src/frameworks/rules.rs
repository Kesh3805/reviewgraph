//! The framework rule table (INIT-006). One row per framework with the evidence it needs.

use super::FrameworkCategory;

/// Confidence calibration. Linker confidences live in `codegraph::confidence` (CG-003); these
/// are detection confidences only.
pub mod confidence {
    /// Runtime framework in `dependencies`.
    pub const RUNTIME_PROD: f32 = 0.9;
    /// Runtime framework only in `devDependencies`.
    pub const RUNTIME_DEV_ONLY: f32 = 0.6;
    /// Test framework in any dependency group.
    pub const TEST_DEP: f32 = 0.9;
    /// Config or schema file only.
    pub const CONFIG_ONLY: f32 = 0.7;
    /// Dependency plus config file.
    pub const DEP_AND_CONFIG: f32 = 1.0;
    /// Detected only through a wrapper package (`@nestjs/platform-express`).
    pub const VIA: f32 = 0.9;
}

#[derive(Debug)]
pub struct Rule {
    pub id: &'static str,
    pub category: FrameworkCategory,
    /// Dependency names that prove the framework.
    pub deps: &'static [&'static str],
    /// Config/schema globs relative to the package directory. A pattern without `/` matches a
    /// file directly in the package directory; `**/` patterns match at any depth.
    pub config_globs: &'static [&'static str],
    /// Schema files (evidence kind `SchemaFile`) rather than config files.
    pub schema_globs: &'static [&'static str],
    /// A runtime dependency: a devDependency hit is weaker evidence.
    pub runtime: bool,
}

const fn rule(
    id: &'static str,
    category: FrameworkCategory,
    deps: &'static [&'static str],
    config_globs: &'static [&'static str],
    runtime: bool,
) -> Rule {
    Rule {
        id,
        category,
        deps,
        config_globs,
        schema_globs: &[],
        runtime,
    }
}

pub const RULES: &[Rule] = &[
    // Web
    rule(
        "nestjs",
        FrameworkCategory::Web,
        &["@nestjs/core", "@nestjs/common"],
        &["nest-cli.json"],
        true,
    ),
    rule("express", FrameworkCategory::Web, &["express"], &[], true),
    rule("fastify", FrameworkCategory::Web, &["fastify"], &[], true),
    rule(
        "nextjs",
        FrameworkCategory::Web,
        &["next"],
        &["next.config.js", "next.config.mjs", "next.config.ts"],
        true,
    ),
    // UI
    rule("react", FrameworkCategory::Ui, &["react"], &[], true),
    rule("vue", FrameworkCategory::Ui, &["vue"], &[], true),
    // ORM
    rule(
        "typeorm",
        FrameworkCategory::Orm,
        &["typeorm", "@nestjs/typeorm"],
        &[
            "ormconfig.json",
            "ormconfig.js",
            "ormconfig.ts",
            "**/data-source.ts",
        ],
        true,
    ),
    Rule {
        id: "prisma",
        category: FrameworkCategory::Orm,
        deps: &["prisma", "@prisma/client"],
        config_globs: &[],
        schema_globs: &["**/schema.prisma"],
        runtime: true,
    },
    rule(
        "sequelize",
        FrameworkCategory::Orm,
        &["sequelize", "@nestjs/sequelize"],
        &[],
        true,
    ),
    rule("knex", FrameworkCategory::Orm, &["knex"], &[], true),
    rule(
        "mikroorm",
        FrameworkCategory::Orm,
        &["@mikro-orm/core", "@mikro-orm/nestjs"],
        &[],
        true,
    ),
    rule(
        "drizzle",
        FrameworkCategory::Orm,
        &["drizzle-orm"],
        &["drizzle.config.ts"],
        true,
    ),
    // Queues
    rule(
        "bullmq",
        FrameworkCategory::Queue,
        &["bullmq", "@nestjs/bullmq"],
        &[],
        true,
    ),
    rule(
        "bull",
        FrameworkCategory::Queue,
        &["bull", "@nestjs/bull"],
        &[],
        true,
    ),
    // Tests
    rule(
        "jest",
        FrameworkCategory::Test,
        &["jest", "ts-jest", "@types/jest"],
        &[
            "jest.config.js",
            "jest.config.ts",
            "jest.config.mjs",
            "jest.config.cjs",
            "jest.config.json",
        ],
        false,
    ),
    rule(
        "vitest",
        FrameworkCategory::Test,
        &["vitest"],
        &[
            "vitest.config.ts",
            "vitest.config.js",
            "vitest.config.mjs",
            "vitest.config.mts",
        ],
        false,
    ),
    rule(
        "mocha",
        FrameworkCategory::Test,
        &["mocha"],
        &[
            ".mocharc.js",
            ".mocharc.cjs",
            ".mocharc.json",
            ".mocharc.yml",
            ".mocharc.yaml",
        ],
        false,
    ),
    rule(
        "playwright",
        FrameworkCategory::Test,
        &["@playwright/test"],
        &["playwright.config.ts", "playwright.config.js"],
        false,
    ),
    rule(
        "cypress",
        FrameworkCategory::Test,
        &["cypress"],
        &["cypress.config.ts", "cypress.config.js"],
        false,
    ),
    // Config, docs, validation
    rule(
        "config",
        FrameworkCategory::Config,
        &["@nestjs/config", "dotenv"],
        &[],
        true,
    ),
    rule(
        "swagger",
        FrameworkCategory::Docs,
        &["@nestjs/swagger", "swagger-ui-express"],
        &[],
        true,
    ),
    rule(
        "class-validator",
        FrameworkCategory::Validation,
        &["class-validator"],
        &[],
        true,
    ),
    rule("zod", FrameworkCategory::Validation, &["zod"], &[], true),
    rule("joi", FrameworkCategory::Validation, &["joi"], &[], true),
    // Auth libraries
    rule(
        "passport",
        FrameworkCategory::Auth,
        &["passport", "@nestjs/passport"],
        &[],
        true,
    ),
    rule(
        "jwt",
        FrameworkCategory::Auth,
        &["@nestjs/jwt", "jsonwebtoken", "jose"],
        &[],
        true,
    ),
];

/// Wrapper packages: `(wrapper dependency, framework id it implies, host framework)`.
pub const VIA_RULES: &[(&str, &str, &str)] = &[
    ("@nestjs/platform-express", "express", "nestjs"),
    ("@nestjs/platform-fastify", "fastify", "nestjs"),
];
