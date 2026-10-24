use std::path::PathBuf;

use protocol::{Failure, FailureCode};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("couldn't read or write {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{file} isn't valid: {reason}")]
    Invalid { file: &'static str, reason: String },
    #[error("{0}")]
    NotFound(String),
}

impl Error {
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Error::Io {
            path: path.into(),
            source,
        }
    }

    pub fn invalid(file: &'static str, reason: impl ToString) -> Self {
        Error::Invalid {
            file,
            reason: reason.to_string(),
        }
    }
}

impl From<Error> for Failure {
    fn from(err: Error) -> Self {
        let code = match err {
            Error::Io { .. } => FailureCode::Io,
            Error::Invalid { .. } => FailureCode::Invalid,
            Error::NotFound(_) => FailureCode::NotFound,
        };
        Failure {
            code,
            message: err.to_string(),
        }
    }
}
