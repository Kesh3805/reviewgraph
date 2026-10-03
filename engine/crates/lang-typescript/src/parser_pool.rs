//! Thread-local tree-sitter parsers. Parser objects are expensive, so each thread keeps one per
//! grammar and reuses it. A parser is never shared across threads.

use std::cell::RefCell;

use analysis_ir::AnalyzeError;
use review_core::language::Dialect;
use tree_sitter::{Language, Parser};

/// Which grammar parses a file. JavaScript uses the TSX grammar: JS is a syntactic subset of
/// TS, files named `.js` often contain JSX, and `<T>expr` casts never occur in JS.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Grammar {
    Typescript,
    Tsx,
}

impl Grammar {
    pub fn for_dialect(dialect: Dialect) -> Self {
        match dialect {
            Dialect::Ts | Dialect::Dts => Self::Typescript,
            Dialect::Tsx | Dialect::Js | Dialect::Jsx | Dialect::Mjs | Dialect::Cjs => Self::Tsx,
        }
    }

    pub fn language(self) -> Language {
        match self {
            Self::Typescript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            Self::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
        }
    }
}

#[derive(Default)]
struct ParserPool {
    typescript: Option<Parser>,
    tsx: Option<Parser>,
}

impl ParserPool {
    fn slot(&mut self, grammar: Grammar) -> &mut Option<Parser> {
        match grammar {
            Grammar::Typescript => &mut self.typescript,
            Grammar::Tsx => &mut self.tsx,
        }
    }
}

thread_local! {
    static PARSERS: RefCell<ParserPool> = RefCell::new(ParserPool::default());
}

/// Runs `f` with this thread's parser for `grammar`, creating it on first use.
///
/// `Err(AnalyzeError::GrammarLoad)` only when `set_language` fails, which means the grammar ABI
/// does not match the runtime: a deployment bug that must be surfaced loudly.
pub fn with_parser<T>(
    grammar: Grammar,
    f: impl FnOnce(&mut Parser) -> T,
) -> Result<T, AnalyzeError> {
    PARSERS.with(|pool| {
        let mut pool = pool.borrow_mut();
        let slot = pool.slot(grammar);
        if slot.is_none() {
            let mut parser = Parser::new();
            parser
                .set_language(&grammar.language())
                .map_err(|e| AnalyzeError::GrammarLoad(e.to_string()))?;
            *slot = Some(parser);
        }
        match slot.as_mut() {
            Some(parser) => Ok(f(parser)),
            None => Err(AnalyzeError::GrammarLoad("parser slot is empty".to_owned())),
        }
    })
}
