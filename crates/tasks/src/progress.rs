use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OperationPhase {
    Validating,
    Trashing,
    Restoring,
}

/// Local progress observation. Completion counts advance only after the recovery record is durable.
#[derive(Clone, Debug)]
pub struct OperationProgress {
    pub phase: OperationPhase,
    pub completed: usize,
    pub total: usize,
    pub current_path: Option<PathBuf>,
}
