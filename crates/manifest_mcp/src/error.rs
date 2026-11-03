//! Crate-private error values for the MCP stdio boundary.
//!
//! Every failure of the wire boundary (spawning the child, writing its
//! stdin, reading its stdout, speaking the protocol) converts into one
//! of these values exactly once, and converts into a user-actionable
//! `String` exactly once more at the [`EngineEvent`] boundary. Nothing
//! in this crate logs an error it also propagates.

use std::fmt::{Display, Formatter, Result as FmtResult};

/// One way the MCP stdio boundary can fail.
#[derive(Debug)]
pub(crate) enum Error {
    /// The server process could not be started.
    Spawn {
        command: String,
        source: std::io::Error,
    },
    /// A write (or flush) to the server's stdin failed. The source is
    /// kept for debugger inspection; the user-facing text is friendly
    /// and never carries the raw I/O detail.
    #[allow(dead_code)]
    Write(std::io::Error),
    /// The reader thread hit an I/O error on the server's stdout. The
    /// detail is kept for debugger inspection; the user-facing text is
    /// friendly and never carries the raw I/O detail.
    #[allow(dead_code)]
    Read(String),
    /// A line from the server was not usable protocol. The detail is
    /// kept for debugger inspection; the user-facing text is friendly
    /// and never carries the raw protocol detail.
    #[allow(dead_code)]
    Malformed(String),
    /// The server answered `initialize` with an error.
    Rpc { message: String },
    /// A `muse_status` round trip failed or its result did not parse.
    Tool(String),
    /// The server exited or closed stdout.
    Closed {
        /// True when `initialize` had not completed yet.
        before_connect: bool,
    },
}

impl Display for Error {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        match self {
            Self::Spawn { command, source } => match source.kind() {
                std::io::ErrorKind::NotFound => write!(
                    f,
                    "Couldn't find the Muse MCP server (`{command}`). \
                     Make sure muse-code-mcp is installed, or point \
                     MANIFEST_MUSE_MCP_BIN at its binary."
                ),
                std::io::ErrorKind::PermissionDenied => write!(
                    f,
                    "Couldn't run the Muse MCP server (`{command}`). \
                     Check that the file is executable."
                ),
                _ => write!(f, "Couldn't start the Muse MCP server (`{command}`)."),
            },
            Self::Write(_) => write!(
                f,
                "Couldn't send a message to the Muse MCP server. It may have stopped running."
            ),
            Self::Read(_) => write!(
                f,
                "Couldn't read from the Muse MCP server. It may have stopped running."
            ),
            Self::Malformed(_) => write!(
                f,
                "The Muse MCP server sent something Manifest couldn't understand."
            ),
            Self::Rpc { message } => {
                write!(f, "The Muse MCP server refused to start: {message}")
            }
            Self::Tool(message) => write!(f, "{message}"),
            Self::Closed {
                before_connect: true,
            } => {
                write!(
                    f,
                    "The Muse MCP server stopped before it finished starting up."
                )
            }
            Self::Closed {
                before_connect: false,
            } => write!(
                f,
                "Lost the connection to the Muse MCP server. It may have stopped running."
            ),
        }
    }
}
