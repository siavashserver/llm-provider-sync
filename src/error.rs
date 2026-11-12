//! Typed errors. Each variant maps to a process exit code (see [`Error::exit_code`]).

use std::{io, path::PathBuf};

use thiserror::Error;

use crate::probe::ProbeError;

#[derive(Debug, Error)]
pub enum Error {
    #[error("no {agent} config file found (looked in: {searched})")]
    ConfigNotFound {
        agent: &'static str,
        searched: String,
    },

    #[error("cannot access {}: {source}", .path.display())]
    Io { path: PathBuf, source: io::Error },

    #[error("invalid config {}: {message}", .path.display())]
    Parse { path: PathBuf, message: String },

    #[error("config {} must contain a JSON object", .0.display())]
    NotObject(PathBuf),

    #[error("invalid provider id: {0}")]
    InvalidId(String),

    #[error("invalid base URL: {0}")]
    InvalidUrl(String),

    #[error("provider `{0}` not found")]
    ProviderNotFound(String),

    #[error("provider `{0}` exists but is not OpenAI-compatible")]
    NotCompatible(String),

    #[error("provider `{0}` already exists")]
    ProviderExists(String),

    #[error(transparent)]
    Probe(#[from] ProbeError),

    #[error("{0} provider(s) failed to sync")]
    SyncFailed(usize),

    #[error("{0}")]
    Usage(String),

    #[error("cancelled")]
    Cancelled,

    #[error("terminal error: {0}")]
    Ui(#[source] io::Error),
}

impl Error {
    /// Process exit code for this error.
    pub fn exit_code(&self) -> u8 {
        match self {
            Self::Usage(_) => 2,
            Self::ConfigNotFound { .. }
            | Self::Io { .. }
            | Self::Parse { .. }
            | Self::NotObject(_) => 3,
            Self::InvalidId(_) | Self::InvalidUrl(_) | Self::Probe(_) | Self::SyncFailed(_) => 4,
            Self::ProviderNotFound(_) | Self::NotCompatible(_) | Self::ProviderExists(_) => 5,
            Self::Ui(_) => 1,
            Self::Cancelled => 130,
        }
    }
}
