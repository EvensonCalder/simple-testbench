use std::path::PathBuf;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum StbError {
    #[error("{0} is not implemented yet")]
    NotImplemented(&'static str),

    #[error("path does not exist: {0}")]
    MissingPath(PathBuf),

    #[error("invalid configuration: {0}")]
    InvalidConfig(String),

    #[error("no model instances matched the requested filters")]
    NoModelsSelected,
}
