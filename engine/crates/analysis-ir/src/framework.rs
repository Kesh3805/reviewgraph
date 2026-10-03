//! Framework-neutral facts produced by framework adapters, and the detection signals that gate
//! the adapters.

use std::collections::BTreeMap;

use review_core::location::SourceRange;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::symbol::{AttrValue, LocalId};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum FrameworkFactKind {
    ModuleDeclaration,
    Controller,
    HttpRoute,
    HttpGlobalConfig,
    DiInjection,
    Middleware,
    MiddlewareBinding,
    GlobalProvider,
    RouteMetadata,
    MetadataDecoratorDefinition,
    OrmEntity,
    OrmColumn,
    OrmRelation,
    OrmAccess,
    QueueConsumer,
    QueueJobHandler,
    QueueProducer,
    QueueRegistration,
    TestSuite,
    TestCase,
    TestMock,
    ConfigRead,
    Custom(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct IrFrameworkFact {
    /// `"nestjs"`, `"typeorm"`, `"bullmq"`, `"jest"`, `"config"`, ...
    pub adapter: String,
    pub kind: FrameworkFactKind,
    pub symbol: Option<LocalId>,
    pub attrs: BTreeMap<String, AttrValue>,
    pub range: SourceRange,
    pub confidence: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FrameworkPresence {
    /// Package directories where the framework was detected (the root is the empty string).
    pub scope_dirs: Vec<String>,
    pub major: Option<u64>,
    pub confidence: f32,
}

/// Plain-data framework detection result. `repository` produces the same shape
/// (`FrameworkSignalsData`); the composition root converts field for field, because
/// `repository` and `analysis-ir` must not depend on each other.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
pub struct FrameworkSignals {
    pub frameworks: BTreeMap<String, FrameworkPresence>,
}

impl FrameworkSignals {
    pub fn has(&self, id: &str) -> bool {
        self.frameworks.contains_key(id)
    }

    pub fn major(&self, id: &str) -> Option<u64> {
        self.frameworks.get(id).and_then(|p| p.major)
    }
}
