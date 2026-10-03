//! Imports and exports of a module.

use review_core::location::SourceRange;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::symbol::{IrExpr, LocalId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum ImportKind {
    Esm,
    SideEffect,
    CjsRequire,
    Dynamic,
    ImportEquals,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum Imported {
    Named(String),
    Default,
    Namespace,
    CjsModule,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ImportBinding {
    pub local: String,
    pub imported: Imported,
    pub type_only: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct IrImport {
    pub specifier: String,
    pub kind: ImportKind,
    pub type_only: bool,
    pub bindings: Vec<ImportBinding>,
    pub range: SourceRange,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum IrExport {
    Local {
        symbol: LocalId,
        exported_as: String,
        type_only: bool,
    },
    Reexport {
        specifier: String,
        imported: Imported,
        exported_as: String,
        type_only: bool,
    },
    StarReexport {
        specifier: String,
        as_namespace: Option<String>,
        type_only: bool,
    },
    DefaultExpr {
        expr: IrExpr,
    },
    CjsModuleExports {
        symbol: Option<LocalId>,
        expr: IrExpr,
    },
    CjsExportsProperty {
        name: String,
        symbol: Option<LocalId>,
    },
    ExportAssignment {
        symbol: Option<LocalId>,
    },
}
