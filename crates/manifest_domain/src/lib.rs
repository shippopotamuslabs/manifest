//! Domain values for Manifest.
//!
//! `manifest_domain` is Manifest's domain crate and the innermost crate
//! of the workspace: pure session, transcript, and todo types, the engine
//! seam types the shell renders engine state through, plus a
//! deterministic [`fixture()`] the shell renders from. It performs no
//! I/O, owns no framework types, and depends on nothing
//! (distinct from external `agent_core`, shippopotamuslabs/agent_core).

mod engine;
mod fixture;

pub use engine::{
    Connection, EngineEvent, ItemStatus, PendingKind, PendingPhase, SessionEngine, SpawnChoices,
    StreamEvent, StreamItem, TurnId, UsageKind, WorktreeChoice,
};
pub use fixture::{
    Fixture, ProjectId, ProjectSummary, SessionId, SessionStatus, SessionSummary, TodoItem,
    TranscriptItem, assign_projects, fixture, seed_projects, workspace_name,
};
