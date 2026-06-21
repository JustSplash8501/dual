use thiserror::Error;

#[derive(Debug, Error)]
pub enum DualError {
    #[error("dual.toml was not found in {0}. Run `dual init` first.")]
    MissingConfig(String),

    #[error("dual.toml is invalid: {0}")]
    InvalidConfig(String),

    #[error("task `{name}` is not defined{available}")]
    MissingTask { name: String, available: String },

    #[error("Dual environment support could not start: {0}")]
    BackendStart(String),

    #[error("Dual environment support exited with status {0}")]
    BackendFailed(String),
}

#[cfg(test)]
mod tests {
    use super::DualError;

    #[test]
    fn backend_errors_use_dual_terminology() {
        for error in [
            DualError::BackendStart("not installed".to_owned()),
            DualError::BackendFailed("1".to_owned()),
        ] {
            let message = error.to_string().to_ascii_lowercase();
            assert!(message.contains("dual environment support"));
            for forbidden in ["pixi", "conda", "environment engine"] {
                assert!(!message.contains(forbidden));
            }
        }
    }
}
