use std::path::PathBuf;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum StbError {
    #[error("{0} is not implemented yet")]
    NotImplemented(&'static str),

    #[error("path does not exist: {0}")]
    MissingPath(PathBuf),
}
