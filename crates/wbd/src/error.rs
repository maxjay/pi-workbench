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

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (Error::NotFound(message)
        | Error::Io(message)
        | Error::Conflict(message)
        | Error::Agent(message)
        | Error::NotSignedIn(message)
        | Error::Dova(message)) = self;
        f.write_str(message)
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
