//! Domain values for Manifest.
//!
//! `manifest_domain` is Manifest's domain crate and the innermost crate
//! of the workspace: pure session, transcript, and todo types plus a
//! deterministic [`fixture()`] the shell renders from. It performs no
//! I/O, owns no framework types, and depends on nothing
//! (distinct from external `agent_core`, shippopotamuslabs/agent_core).

mod fixture;

pub use fixture::{Fixture, SessionId, SessionStatus, SessionSummary, TodoItem, TranscriptItem, fixture};
