//! The classifier implementations, one module per class family.

pub mod condition;
pub mod control_flow;
pub mod loops;
pub mod returns;

use super::classify::Classifier;

/// Every built-in classifier, in the order they run.
pub fn registry() -> Vec<Box<dyn Classifier>> {
    vec![
        Box::new(control_flow::ControlFlowClassifier),
        Box::new(condition::ConditionClassifier),
        Box::new(loops::LoopClassifier),
        Box::new(returns::ReturnClassifier),
    ]
}
