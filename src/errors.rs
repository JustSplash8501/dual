use thiserror::Error;

#[derive(Debug, Error)]
pub enum DualError {
    #[error("dual.toml was not found in {0}. Run `dual init` first.")]
    MissingConfig(String),

    #[error("dual.toml is invalid: {0}")]
    InvalidConfig(String),

    #[error("task `{name}` is not defined{available}")]
    MissingTask { name: String, available: String },

    #[error("the environment engine could not start: {0}")]
    BackendStart(String),

    #[error("the environment engine exited with status {0}")]
    BackendFailed(String),
}
