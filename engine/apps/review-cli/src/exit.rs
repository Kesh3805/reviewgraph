//! Process exit codes (CLI-001).

/// Exit codes of the `review` binary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum ExitCode {
    Ok = 0,
    FindingsAtOrAboveThreshold = 1,
    Usage = 2,
    NotInitialized = 3,
    GraphStale = 4,
    ProviderOrAuth = 5,
    Internal = 10,
}

impl ExitCode {
    pub fn code(self) -> i32 {
        self as i32
    }

    /// Stable machine-readable error code used in the JSON error envelope.
    pub fn name(self) -> &'static str {
        match self {
            ExitCode::Ok => "ok",
            ExitCode::FindingsAtOrAboveThreshold => "findings",
            ExitCode::Usage => "usage",
            ExitCode::NotInitialized => "not_initialized",
            ExitCode::GraphStale => "graph_stale",
            ExitCode::ProviderOrAuth => "provider_or_auth",
            ExitCode::Internal => "internal",
        }
    }
}

/// An error that carries the exit code and an optional hint.
#[derive(Debug)]
pub struct CliError {
    pub exit: ExitCode,
    pub message: String,
    pub hint: Option<String>,
}

impl CliError {
    pub fn new(exit: ExitCode, message: impl Into<String>) -> Self {
        Self {
            exit,
            message: message.into(),
            hint: None,
        }
    }

    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    pub fn not_initialized() -> Self {
        Self::new(ExitCode::NotInitialized, "repository is not initialized")
            .with_hint("run `review init`")
    }
}

impl std::fmt::Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for CliError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_code_not_initialized_3() {
        assert_eq!(CliError::not_initialized().exit.code(), 3);
        assert_eq!(ExitCode::Internal.code(), 10);
    }
}
