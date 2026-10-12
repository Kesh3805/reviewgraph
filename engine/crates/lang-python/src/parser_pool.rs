//! Thread-local tree-sitter parser. A parser is expensive to build, so each thread keeps one and
//! reuses it. A parser is never shared across threads.

use std::cell::RefCell;

use analysis_ir::AnalyzeError;
use tree_sitter::Parser;

thread_local! {
    static PARSER: RefCell<Option<Parser>> = const { RefCell::new(None) };
}

/// Runs `f` with this thread's parser, creating it on first use.
///
/// `Err(AnalyzeError::GrammarLoad)` only when `set_language` fails, which means the grammar ABI
/// does not match the runtime: a deployment bug that must be surfaced loudly.
pub fn with_parser<T>(f: impl FnOnce(&mut Parser) -> T) -> Result<T, AnalyzeError> {
    PARSER.with(|slot| {
        let mut slot = slot.borrow_mut();
        if slot.is_none() {
            let mut parser = Parser::new();
            parser
                .set_language(&tree_sitter_python::LANGUAGE.into())
                .map_err(|e| AnalyzeError::GrammarLoad(e.to_string()))?;
            *slot = Some(parser);
        }
        match slot.as_mut() {
            Some(parser) => Ok(f(parser)),
            None => Err(AnalyzeError::GrammarLoad("parser slot is empty".to_owned())),
        }
    })
}
