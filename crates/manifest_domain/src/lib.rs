//! Domain values for Manifest.
//!
//! `manifest_domain` is Manifest's domain crate and the innermost crate
//! of the workspace: pure session, transcript, and todo types, the engine
//! seam types the shell renders engine state through, plus a
//! deterministic [`fixture()`] the shell renders from. It performs no
//! I/O, owns no framework types, and depends on nothing
//! (distinct from external `agent_core`, shippopotamuslabs/agent_core).

mod branch;
mod engine;
mod fixture;

pub use branch::{parse_gitdir_pointer, parse_head, resolve_git_dir};
pub use engine::{
    AnswerValue, ApprovalChoiceSummary, ApprovalDecision, ApprovalRequest, BuiltinCommand,
    Connection, DraftAction, EffortChoice, EngineEvent, FREE_TEXT_LIMIT, FailedCommand, GoalInfo,
    GoalStatus, ItemStatus, ModelChoice, ModelInfo, MusePermission, PatchDocument, PatchFile,
    PatchHunk, PatchLine, PatchLineKind, PatchRef, PatchSummary, PendingAnswer, PendingDecision,
    PendingKind, PendingPhase, PendingRequest, PendingResolution, PermissionMode, Question,
    QuestionOption, QuestionRequest, SessionEngine, SkillInfo, SkillInvocation, SpawnChoices,
    StreamEvent, StreamItem, TurnId, TurnInput, UsageKind, UsageSnapshot, WireAnswerKind,
    WorktreeChoice, parse_slash_draft, truncate_free_text,
};
pub use fixture::{
    Fixture, ProjectId, ProjectSummary, SessionId, SessionStatus, SessionSummary, TodoItem,
    TodoStatus, TranscriptItem, assign_projects, compose_session_title, fixture, seed_projects,
    workspace_name,
};
