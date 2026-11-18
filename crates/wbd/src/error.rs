//! Why wbd couldn't do what a call asked. Each becomes the failure the page shows.

use std::path::Path;

use protocol::{Failure, FailureKind};

#[derive(Debug)]
pub enum Error {
    NotFound(String),
    Io(String),
    /// A save refused because the file changed since it was read.
    Conflict(String),
    /// The agent couldn't be started, or went away.
    Agent(String),
    /// dova has no sign-in to Azure DevOps.
    NotSignedIn(String),
    /// dova couldn't be started, or failed.
    Dova(String),
}

impl Error {
    /// Reading or writing `path` failed.
    pub fn io(path: &Path, err: std::io::Error) -> Self {
        Error::Io(format!("{}: {err}", path.display()))
    }
}

impl From<Error> for Failure {
    fn from(err: Error) -> Self {
        match err {
            Error::NotFound(message) => Failure::new(FailureKind::NotFound, message),
            Error::Io(message) => Failure::new(FailureKind::Io, message),
            Error::Conflict(message) => Failure::new(FailureKind::Conflict, message),
            Error::Agent(message) => Failure::new(FailureKind::Agent, message),
            Error::NotSignedIn(message) => Failure::new(FailureKind::NotSignedIn, message),
            Error::Dova(message) => Failure::new(FailureKind::Dova, message),
        }
    }
}
