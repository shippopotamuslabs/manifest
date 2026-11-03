//! Store values for the project-registry boundary.
//!
//! Every failure of the disk boundary (creating parent directories,
//! writing the file, renaming the temp file into place, reading or
//! parsing the file) converts into one of these values exactly once.
//! Nothing in this crate logs an error it also propagates.

use std::fmt::{Display, Formatter, Result as FmtResult};
use std::path::PathBuf;

/// One way the project-registry disk boundary can fail.
#[derive(Debug)]
pub enum Error {
    /// A read, write, rename, or directory-creation step failed for a
    /// reason other than the file being absent on load. The source is
    /// kept so `source()` reports the operating-system detail; the
    /// user-facing text is friendly and names the path.
    Io {
        /// The file being operated on when the failure happened.
        path: PathBuf,
        /// The underlying operating-system failure.
        source: std::io::Error,
    },
    /// The file exists but is not a valid registry: bad JSON, a
    /// missing or blank field, or a duplicated workspace.
    Corrupt {
        /// The registry file that failed to parse or validate.
        path: PathBuf,
        /// What was wrong with the file.
        detail: String,
    },
    /// The file's `version` field is not one this Manifest build reads.
    UnsupportedVersion {
        /// The registry file carrying the unknown version.
        path: PathBuf,
        /// The version number found in the file.
        found: u32,
    },
    /// `save` was given a project it cannot persist. The file is left
    /// untouched; fix the project and try again.
    Invalid {
        /// Which project was rejected and why.
        detail: String,
    },
}

impl Display for Error {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        match self {
            Self::Io { path, source } => write!(
                f,
                "Couldn't read or write the project list at {}: {source}",
                path.display(),
            ),
            Self::Corrupt { path, detail } => write!(
                f,
                "The project list at {} is not a valid registry: {detail}",
                path.display(),
            ),
            Self::UnsupportedVersion { path, found } => write!(
                f,
                "The project list at {} uses version {found}, which this version of Manifest cannot read.",
                path.display(),
            ),
            Self::Invalid { detail } => {
                write!(f, "Couldn't save the project list: {detail}")
            }
        }
    }
}

impl std::error::Error for Error {
    /// Report the operating-system failure behind [`Error::Io`].
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Corrupt { .. } | Self::UnsupportedVersion { .. } | Self::Invalid { .. } => None,
        }
    }
}
