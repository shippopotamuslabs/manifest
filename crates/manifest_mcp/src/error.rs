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
    /// A write (or flush) to the server's stdin failed.
    Write(std::io::Error),
    /// The reader thread hit an I/O error on the server's stdout.
    Read(String),
    /// A line from the server was not usable protocol.
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
            Self::Spawn { command, source } => {
                write!(f, "Could not start the MCP server `{command}`: {source}")
            }
            Self::Write(source) => write!(f, "Could not write to the MCP server: {source}"),
            Self::Read(detail) => write!(f, "{detail}"),
            Self::Malformed(detail) => write!(f, "MCP protocol error: {detail}"),
            Self::Rpc { message } => write!(f, "MCP initialize failed: {message}"),
            Self::Tool(message) => write!(f, "{message}"),
            Self::Closed {
                before_connect: true,
            } => {
                write!(f, "The MCP server exited before completing initialize")
            }
            Self::Closed {
                before_connect: false,
            } => write!(f, "The MCP server closed the connection"),
        }
    }
}
