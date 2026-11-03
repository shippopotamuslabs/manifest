//! Transcript: streaming shell entries grouped into rows.
//!
//! The transcript renders shell entries, not domain items: stream events
//! fold into entries by host id, and rows derive from entries after every
//! mutation. Grouping, headlines, and heights are all state; the view only
//! reads rows (UI contract §§1-2). Live markdown parses incrementally into
//! [`iced::widget::markdown::Content`]; only the live entry re-renders
//! while it streams (UI contract §3).

use std::collections::HashSet;

use iced::widget::markdown;
use iced::{Element, Length};
use manifest_domain::{ItemStatus, SessionId, StreamItem, TranscriptItem};

use crate::kit;
use crate::pending;
use crate::theme::{
    ACCENT_BASE, DANGER, FONT_SIZE_BODY, FONT_SIZE_SMALL, SPACE_L, SPACE_M, SPACE_XL, TEXT_PRIMARY,
    TEXT_SECONDARY, TEXT_TERTIARY,
};

/// The text widgets' relative 1.3 line spacing. Confirmed against the
/// live build in LOGICAL px: a one-line bubble measures ~55 logical px
/// tall including its `2 * SPACE_L` vertical padding. Note screenshots
/// are physical px and this desktop scales ~1.2x, so divide physical
/// measurements by the scale factor before comparing.
const LINE_HEIGHT: f32 = FONT_SIZE_BODY * 1.3;
/// Mean glyph advance for the height estimator. Live ink extents measure
/// ~0.45em at `FONT_SIZE_BODY`; tune against the live build if the
/// sticky timing ever drifts from the real text.
const ESTIMATED_CHAR_WIDTH: f32 = FONT_SIZE_BODY * 0.46;
/// Height of a collapsed group header row in px. Matches the kit group
/// header: small text at [`LINE_HEIGHT`] plus its vertical padding.
/// Tune against the release build if the header drifts from the real row.
const GROUP_HEADER_HEIGHT: f32 = FONT_SIZE_SMALL * 1.3 + 2.0 * crate::theme::SPACE_XS;
/// Height of the Copy row under a completed assistant message in px.
/// Matches the fixed square of the kit icon button it now renders.
/// Tune against the release build if it drifts.
const COPY_ROW_HEIGHT: f32 = crate::theme::ICON_BUTTON_SIZE;
/// Approximate height of an inline pending card in px. Tune against
/// the release build if the card drifts from the real height.
pub const PENDING_CARD_HEIGHT: f32 = 340.0;
/// Separator between headline parts, e.g. `Thinking · 2 tool calls`.
const MIDDOT: &str = " · ";

/// Id of the transcript scrollable, so error appends can snap it to
/// the end via [`scroll_to_end`].
const SCROLL_ID: &str = "manifest-transcript";

/// Snaps the transcript scrollable to its end so a freshly appended
/// error is visible below the fold. Callers map the task into their
/// own message type.
pub fn scroll_to_end() -> iced::Task<Message> {
    iced::widget::operation::snap_to_end(SCROLL_ID)
}

/// One live shell entry: a stream item folded by host id plus local
/// echoes (optimistic user prompts and errors, which carry no host id).
#[derive(Debug)]
pub struct Entry {
    /// Host item handle; `None` for local echoes until the echo is
    /// claimed by its completed event.
    pub host_id: Option<String>,
    /// The entry payload.
    pub kind: EntryKind,
    /// Whether the host marked the item complete (or the entry is a
    /// finished local value).
    pub completed: bool,
}

/// Entry payloads: one variant per rendered shape.
#[derive(Debug)]
pub enum EntryKind {
    /// Something the user wrote.
    User {
        /// Prompt text.
        text: String,
    },
    /// Something the agent wrote: raw markdown plus its incrementally
    /// parsed form. The text is authoritative; the content is its cache.
    Assistant {
        /// Assistant markdown.
        text: String,
        /// Incrementally parsed markdown; reparsed wholesale on complete.
        markdown: markdown::Content,
    },
    /// The host's abstract of the turn's plan, one sentence per slot.
    Reasoning {
        /// Summary sentences; slots grow to fit `summary.N` deltas.
        summary: Vec<String>,
    },
    /// One tool invocation with its headline and result.
    Tool {
        /// Tool name; empty when the wire sent none.
        tool: String,
        /// One-line headline parsed from the args; none when no arg fit.
        headline: Option<String>,
        /// Result text: streamed deltas, replaced by the completed item
        /// when it carries output.
        output: String,
        /// How far the call got.
        status: ItemStatus,
        /// Machine-readable failure detail; present on failed calls.
        failure: Option<String>,
    },
    /// A note worth showing inline, e.g. a fixture tool summary or an
    /// unknown item kind the shell renders instead of dropping.
    Note {
        /// Note text.
        text: String,
    },
    /// An error shown as part of the conversation in the danger tone.
    Error {
        /// Error text.
        text: String,
    },
    /// A pending approval/question at its pause point. Only the
    /// request id lives here; card payload and phase live in
    /// `pending::State`. Settling collapses this entry in place to
    /// a one-line `Note` record.
    Pending {
        /// Opaque pending request handle.
        request_id: String,
    },
}

/// One rendered row: either a lone entry or a collapsible run of
/// reasoning/tool entries. Rows derive from entries after every
/// mutation ([`State::rebuild_rows`]); the view reads them as-is.
#[derive(Debug, Clone, PartialEq)]
pub enum Row {
    /// A single entry at this entry index.
    Single(usize),
    /// A maximal run of consecutive reasoning/tool entries. `first` and
    /// `last` are inclusive entry indices; `key` identifies the run for
    /// the expanded set; `headline` is precomputed for the header.
    Group {
        /// First entry index in the run, inclusive.
        first: usize,
        /// Last entry index in the run, inclusive.
        last: usize,
        /// Run identity: the first entry's host id, else `row-{first}`.
        key: String,
        /// Precomputed header text (UI contract §2).
        headline: String,
        /// Whether the run renders its members below the header.
        expanded: bool,
    },
}

/// Transcript state: shell entries, their derived rows, and
/// scroll-derived sticky data.
#[derive(Debug)]
pub struct State {
    /// Shell entries for the visible session.
    pub entries: Vec<Entry>,
    /// Rows derived from entries; rebuilt after every mutation.
    pub rows: Vec<Row>,
    /// Group keys currently expanded.
    expanded: HashSet<String>,
    /// Markdown rendering config, built once from theme tokens.
    settings: markdown::Settings,
    /// Absolute vertical scroll offset in px, from the last viewport.
    pub scroll_offset: f32,
    /// Scrollable content width in px, from the last viewport.
    pub content_width: f32,
    /// Index into [`rows`](State::rows) of the prompt stuck to the top.
    pub sticky: Option<usize>,
    /// Px the sticky bubble is pushed up by the incoming prompt.
    pub sticky_push: f32,
    /// Full height of the sticky bubble in px; sizes the overlay window.
    pub sticky_height: f32,
}

impl Default for State {
    /// Empty transcript: no entries, no rows, default scroll state.
    fn default() -> Self {
        State {
            entries: Vec::new(),
            rows: Vec::new(),
            expanded: HashSet::new(),
            settings: markdown_settings(),
            scroll_offset: 0.0,
            content_width: 0.0,
            sticky: None,
            sticky_push: 0.0,
            sticky_height: 0.0,
        }
    }
}

/// Transcript messages, enumerated before the view (UI contract §1).
/// `Viewport` is `Debug + Clone` but not `PartialEq`, so neither is
/// this enum.
#[derive(Debug, Clone)]
pub enum Message {
    /// The scrollable published a new viewport after scrolling or a
    /// bounds change.
    Scrolled(iced::widget::scrollable::Viewport),
    /// A group header was pressed; flips the run's expanded state.
    ToggleGroup(String),
    /// A Copy button was pressed; the app writes the text to the
    /// clipboard (a `Task`, which state cannot produce).
    Copy(String),
    /// A markdown link was clicked. No-op in this slice: link opening
    /// lands later, which will read the URI.
    #[allow(dead_code)]
    LinkClicked(String),
    /// An inline pending-card interaction, routed by the app exactly
    /// like `app::Message::Pending`.
    Pending(crate::pending::Message),
}

/// Element for the pending entry at its pause point: the full card
/// from `pending_state` for `session`, mapped to [`Message::Pending`].
pub fn pending_element<'a>(
    pending_state: &'a pending::State,
    session: Option<&'a SessionId>,
    request_id: &str,
) -> Option<Element<'a, Message>> {
    pending_state
        .card_element(session, request_id)
        .map(|element| element.map(Message::Pending))
}

/// Markdown rendering config, built once from theme tokens (UI
/// contract §5): body text size, monospace code fonts, surface-2
/// inline-code wash, and the accent link color.
fn markdown_settings() -> markdown::Settings {
    let style = markdown::Style {
        font: iced::Font::default(),
        inline_code_highlight: markdown::Highlight {
            background: iced::Background::Color(crate::theme::SURFACE_2),
            border: iced::Border::default().rounded(4.0),
        },
        inline_code_padding: iced::padding::left(1).right(1),
        inline_code_color: TEXT_PRIMARY,
        inline_code_font: iced::Font::MONOSPACE,
        code_block_font: iced::Font::MONOSPACE,
        link_color: ACCENT_BASE,
    };
    markdown::Settings::with_text_size(FONT_SIZE_BODY, style)
}

/// Friendly note text naming an unknown item kind in words.
fn friendly_unknown(kind: &str) -> String {
    format!("Unhandled item kind: {kind}")
}

/// Friendly note text for a compaction item. The terminal outcome and
/// reason ride as values (failures after admission arrive as this
/// item's terminal event), so they render here, never as a drop. A
/// missing outcome means the item is still in progress.
fn friendly_compaction(
    outcome: &Option<String>,
    reason: &Option<String>,
    tokens_before: &Option<u64>,
    tokens_after: &Option<u64>,
) -> String {
    match outcome.as_deref() {
        Some("compacted") => match (tokens_before, tokens_after) {
            (Some(before), Some(after)) => {
                format!("Context compacted · {before} → {after} tokens")
            }
            _ => String::from("Context compacted"),
        },
        Some("noop") => match reason {
            Some(reason) => format!("Compaction not needed ({reason})"),
            None => String::from("Compaction not needed"),
        },
        Some("failed") => match reason {
            Some(reason) => format!("Compaction failed: {reason}"),
            None => String::from("Compaction failed"),
        },
        Some("cancelled") => String::from("Compaction cancelled"),
        Some(other) => format!("Compaction finished ({other})"),
        None => String::from("Compacting context…"),
    }
}

/// The summary sentences as one block for rendering and measuring.
fn reasoning_text(summary: &[String]) -> String {
    summary.join("\n")
}

/// Whether an entry joins a group run.
fn is_groupable(kind: &EntryKind) -> bool {
    matches!(kind, EntryKind::Reasoning { .. } | EntryKind::Tool { .. })
}

/// Whether an entry is a user prompt (the only sticky row kind).
fn is_user(entry: &Entry) -> bool {
    matches!(entry.kind, EntryKind::User { .. })
}

/// Whether an entry and a stream item describe the same variant.
fn entry_matches(entry: &Entry, item: &StreamItem) -> bool {
    matches!(
        (&entry.kind, item),
        (EntryKind::User { .. }, StreamItem::UserMessage { .. })
            | (EntryKind::Assistant { .. }, StreamItem::AgentMessage { .. })
            | (EntryKind::Reasoning { .. }, StreamItem::Reasoning { .. })
            | (EntryKind::Tool { .. }, StreamItem::ToolCall { .. })
            | (EntryKind::Note { .. }, StreamItem::Compaction { .. })
            | (EntryKind::Note { .. }, StreamItem::Unknown { .. })
    )
}

/// Builds an entry from a stream item. System items deliberately produce
/// none: host bookkeeping with no user-facing value is not rendered.
/// Returns the entry with `completed` set as given.
fn entry_from_item(item: &StreamItem, completed: bool) -> Option<Entry> {
    let host_id = Some(item.item_id().to_string());
    let kind = match item {
        StreamItem::UserMessage { text, .. } => EntryKind::User { text: text.clone() },
        StreamItem::AgentMessage { text, .. } => EntryKind::Assistant {
            markdown: markdown::Content::parse(text),
            text: text.clone(),
        },
        StreamItem::Reasoning { summary, .. } => EntryKind::Reasoning {
            summary: summary.clone(),
        },
        StreamItem::ToolCall {
            tool,
            headline,
            output,
            status,
            failure,
            ..
        } => EntryKind::Tool {
            tool: tool.clone(),
            headline: headline.clone(),
            output: output.clone().unwrap_or_default(),
            status: status.clone(),
            failure: failure.clone(),
        },
        StreamItem::System { .. } => return None,
        StreamItem::Compaction {
            outcome,
            reason,
            tokens_before,
            tokens_after,
            ..
        } => EntryKind::Note {
            text: friendly_compaction(outcome, reason, tokens_before, tokens_after),
        },
        StreamItem::Unknown { kind, .. } => EntryKind::Note {
            text: friendly_unknown(kind),
        },
    };
    Some(Entry {
        host_id,
        kind,
        completed,
    })
}

/// Builds an entry from a fixture domain item. All fixture entries are
/// complete. Fixture tool notes stay notes, matching prior rendering.
fn entry_from_transcript_item(item: TranscriptItem) -> Entry {
    match item {
        TranscriptItem::UserTurn { text } => Entry {
            host_id: None,
            kind: EntryKind::User { text },
            completed: true,
        },
        TranscriptItem::AssistantTurn { text } => Entry {
            host_id: None,
            kind: EntryKind::Assistant {
                markdown: markdown::Content::parse(&text),
                text,
            },
            completed: true,
        },
        TranscriptItem::ToolNote { text } => Entry {
            host_id: None,
            kind: EntryKind::Note { text },
            completed: true,
        },
        TranscriptItem::ErrorNote { text } => Entry {
            host_id: None,
            kind: EntryKind::Error { text },
            completed: true,
        },
    }
}

/// Precomputes a group headline from a run of entries (UI contract §2).
/// A lone tool run shows its tool name (or `Tool` when empty) plus its
/// headline after a middot when present; any other run joins `Thinking`
/// (when the run holds reasoning) with `1 tool call` / `N tool calls`
/// (when it holds tools). A working suffix marks an unfinished run and
/// a failed suffix marks a run holding a failed tool call.
fn group_headline(run: &[Entry]) -> String {
    if run.len() == 1
        && let EntryKind::Tool { tool, headline, .. } = &run[0].kind
    {
        let name = if tool.is_empty() {
            "Tool"
        } else {
            tool.as_str()
        };
        let mut headline_text = match headline {
            Some(line) if !line.is_empty() => format!("{name}{MIDDOT}{line}"),
            _ => name.to_string(),
        };
        if !run[0].completed {
            headline_text.push_str(&format!("{MIDDOT}working"));
        }
        if matches!(
            &run[0].kind,
            EntryKind::Tool {
                status: ItemStatus::Failed,
                ..
            }
        ) {
            headline_text.push_str(&format!("{MIDDOT}failed"));
        }
        return headline_text;
    }
    let mut parts = Vec::new();
    if run
        .iter()
        .any(|entry| matches!(entry.kind, EntryKind::Reasoning { .. }))
    {
        parts.push(String::from("Thinking"));
    }
    let tools = run
        .iter()
        .filter(|entry| matches!(entry.kind, EntryKind::Tool { .. }))
        .count();
    if tools == 1 {
        parts.push(String::from("1 tool call"));
    } else if tools > 1 {
        parts.push(format!("{tools} tool calls"));
    }
    let mut headline_text = parts.join(MIDDOT);
    if run.iter().any(|entry| !entry.completed) {
        headline_text.push_str(&format!("{MIDDOT}working"));
    }
    if run.iter().any(|entry| {
        matches!(
            entry.kind,
            EntryKind::Tool {
                status: ItemStatus::Failed,
                ..
            }
        )
    }) {
        headline_text.push_str(&format!("{MIDDOT}failed"));
    }
    headline_text
}

impl State {
    /// Appends an error entry, unless the last entry is already the
    /// same error (a flapping refresh must not stack duplicates).
    /// Returns true when an entry was appended, false when deduped.
    pub fn push_error(&mut self, text: String) -> bool {
        let duplicate = matches!(
            self.entries.last(),
            Some(Entry {
                kind: EntryKind::Error { text: last },
                ..
            }) if last == &text
        );
        if !duplicate {
            self.entries.push(Entry {
                host_id: None,
                kind: EntryKind::Error { text },
                completed: true,
            });
            self.rebuild_rows();
            true
        } else {
            false
        }
    }

    /// Appends an optimistic local user prompt. The echo carries no
    /// host id until the prompt's completed event claims it.
    pub fn push_user(&mut self, text: String) {
        self.entries.push(Entry {
            host_id: None,
            kind: EntryKind::User { text },
            completed: false,
        });
        self.rebuild_rows();
    }

    /// Inserts a pending entry at the end (the pause point).
    /// Dedupes by request id. Returns true when inserted.
    pub fn push_pending(&mut self, request_id: String) -> bool {
        if self.entries.iter().any(|entry| {
            matches!(&entry.kind, EntryKind::Pending { request_id: id } if id == &request_id)
        }) {
            return false;
        }
        self.entries.push(Entry {
            host_id: None,
            kind: EntryKind::Pending { request_id },
            completed: false,
        });
        self.rebuild_rows();
        true
    }

    /// Collapses a pending entry in place to its one-line record.
    /// Returns true when an entry collapsed.
    pub fn collapse_pending(&mut self, request_id: &str, line: String) -> bool {
        let Some(entry) = self.entries.iter_mut().find(|entry| {
            matches!(&entry.kind, EntryKind::Pending { request_id: id } if id == request_id)
        }) else {
            return false;
        };
        entry.kind = EntryKind::Note { text: line };
        entry.completed = true;
        self.rebuild_rows();
        true
    }

    /// Whether a live (uncollapsed) pending entry names `request_id`.
    #[allow(dead_code)]
    pub fn has_pending(&self, request_id: &str) -> bool {
        self.entries.iter().any(|entry| {
            matches!(&entry.kind, EntryKind::Pending { request_id: id } if id == request_id)
        })
    }

    /// Builds transcript state from domain items. Scroll-derived fields
    /// start at zero / none, matching `Default`.
    pub fn from_items(items: Vec<TranscriptItem>) -> Self {
        let mut state = State {
            entries: items.into_iter().map(entry_from_transcript_item).collect(),
            ..State::default()
        };
        state.rebuild_rows();
        state
    }

    /// Host-id join: the position of the entry carrying this id.
    fn position_by_host(&self, host_id: &str) -> Option<usize> {
        self.entries
            .iter()
            .position(|entry| entry.host_id.as_deref() == Some(host_id))
    }

    /// Folds an item-started event into entries. System items produce
    /// no entry. Started is an upsert: a missing entry is created, and
    /// an existing one refreshes its payload — but empty started values
    /// never wipe streamed content a delta already filled.
    pub fn item_started(&mut self, item: StreamItem) {
        if matches!(item, StreamItem::System { .. }) {
            return;
        }
        let host_id = item.item_id().to_string();
        match self.position_by_host(&host_id) {
            Some(position) => refresh_entry(&mut self.entries[position], &item),
            None => {
                if let Some(entry) = entry_from_item(&item, false) {
                    self.entries.push(entry);
                }
            }
        }
        self.rebuild_rows();
    }

    /// Folds an item delta into the entry joined by host id. Only the
    /// live fields append: assistant `text`, reasoning `summary.N`
    /// (slots grow with empty strings to index `N`), tool `output`.
    /// Unknown items or fields are ignored: item-completed is
    /// authoritative, so a dropped path still converges.
    pub fn item_delta(&mut self, item_id: &str, field: &str, delta: &str) {
        // A delta with no joined entry has no kind to append to (deltas
        // carry no kind); it is ignored and the completed item fills in.
        let Some(position) = self.position_by_host(item_id) else {
            return;
        };
        match (&mut self.entries[position].kind, field) {
            (EntryKind::Assistant { text, markdown }, "text") => {
                text.push_str(delta);
                markdown.push_str(delta);
            }
            (EntryKind::Reasoning { summary }, field) => {
                if let Some(index) = field
                    .strip_prefix("summary.")
                    .and_then(|slot| slot.parse::<usize>().ok())
                {
                    while summary.len() <= index {
                        summary.push(String::new());
                    }
                    summary[index].push_str(delta);
                }
            }
            (EntryKind::Tool { output, .. }, "output") => {
                output.push_str(delta);
            }
            _ => {}
        }
        self.rebuild_rows();
    }

    /// Folds an item-completed event into entries, authoritatively:
    /// payloads replace wholesale, except a tool completion without
    /// output keeps the streamed buffer. A userMessage completion first
    /// tries to claim an equal-text optimistic echo instead of
    /// appending a duplicate; a completion without a started event
    /// otherwise creates its entry.
    pub fn item_completed(&mut self, item: StreamItem) {
        if matches!(item, StreamItem::System { .. }) {
            return;
        }
        let host_id = item.item_id().to_string();
        if self.position_by_host(&host_id).is_none()
            && let StreamItem::UserMessage { text, .. } = &item
            && let Some(position) = self.entries.iter().position(|entry| {
                entry.host_id.is_none()
                    && matches!(&entry.kind, EntryKind::User { text: echo } if echo == text)
            })
        {
            self.entries[position].host_id = Some(host_id);
            self.entries[position].completed = true;
            self.rebuild_rows();
            return;
        }
        match self.position_by_host(&host_id) {
            Some(position) => {
                let entry = &mut self.entries[position];
                match (&mut entry.kind, &item) {
                    (
                        EntryKind::Assistant { text, markdown },
                        StreamItem::AgentMessage { text: next, .. },
                    ) => {
                        *text = next.clone();
                        *markdown = markdown::Content::parse(next);
                    }
                    (
                        EntryKind::Tool {
                            tool,
                            headline,
                            output,
                            status,
                            failure,
                        },
                        StreamItem::ToolCall {
                            tool: next_tool,
                            headline: next_headline,
                            output: next_output,
                            status: next_status,
                            failure: next_failure,
                            ..
                        },
                    ) => {
                        *tool = next_tool.clone();
                        *headline = next_headline.clone();
                        if let Some(next_output) = next_output {
                            *output = next_output.clone();
                        }
                        *status = next_status.clone();
                        *failure = next_failure.clone();
                    }
                    (
                        EntryKind::Reasoning { summary },
                        StreamItem::Reasoning { summary: next, .. },
                    ) => {
                        *summary = next.clone();
                    }
                    (EntryKind::User { text }, StreamItem::UserMessage { text: next, .. }) => {
                        *text = next.clone();
                    }
                    (EntryKind::Note { text }, StreamItem::Unknown { kind, .. }) => {
                        *text = friendly_unknown(kind);
                    }
                    (
                        EntryKind::Note { text },
                        StreamItem::Compaction {
                            outcome,
                            reason,
                            tokens_before,
                            tokens_after,
                            ..
                        },
                    ) => {
                        *text = friendly_compaction(outcome, reason, tokens_before, tokens_after);
                    }
                    _ => {
                        if let Some(fresh) = entry_from_item(&item, true) {
                            entry.kind = fresh.kind;
                        }
                    }
                }
                entry.completed = true;
            }
            None => {
                if let Some(entry) = entry_from_item(&item, true) {
                    self.entries.push(entry);
                }
            }
        }
        self.rebuild_rows();
    }

    /// Seeds the transcript from stored history items, oldest first.
    ///
    /// History predates every live entry, so missing items prepend
    /// ahead of existing entries in host order. Items already present
    /// (by host id, from an earlier seed or a live stream event) are
    /// skipped, and system items produce no entry. Returns whether an
    /// entry was added. History is terminal, so seeded entries read
    /// completed.
    pub fn seed_history(&mut self, items: Vec<StreamItem>) -> bool {
        let mut present: HashSet<&str> = self
            .entries
            .iter()
            .filter_map(|entry| entry.host_id.as_deref())
            .collect();
        let mut fresh = Vec::new();
        for item in &items {
            let id = item.item_id();
            if present.contains(id) {
                continue;
            }
            if let Some(entry) = entry_from_item(item, true) {
                present.insert(id);
                fresh.push(entry);
            }
        }
        if fresh.is_empty() {
            return false;
        }
        let mut entries = std::mem::take(&mut self.entries);
        fresh.append(&mut entries);
        self.entries = fresh;
        self.rebuild_rows();
        true
    }

    /// Derives rows from entries: a maximal run of consecutive
    /// reasoning/tool entries forms one group (even a run of one);
    /// everything else stands alone. Expanded flags persist by key.
    fn rebuild_rows(&mut self) {
        let mut rows = Vec::new();
        let mut index = 0;
        while index < self.entries.len() {
            if !is_groupable(&self.entries[index].kind) {
                rows.push(Row::Single(index));
                index += 1;
                continue;
            }
            let mut last = index;
            while last + 1 < self.entries.len() && is_groupable(&self.entries[last + 1].kind) {
                last += 1;
            }
            let key = self.entries[index]
                .host_id
                .clone()
                .unwrap_or_else(|| format!("row-{index}"));
            let headline = group_headline(&self.entries[index..=last]);
            let expanded = self.expanded.contains(&key);
            rows.push(Row::Group {
                first: index,
                last,
                key,
                headline,
                expanded,
            });
            index = last + 1;
        }
        self.rows = rows;
    }

    /// Reducer: folds viewport samples and group toggles into state.
    /// Copy is intercepted by the app (clipboard is a `Task`); link
    /// clicks are a documented no-op in this slice.
    pub fn update(&mut self, message: Message) {
        match message {
            Message::Scrolled(viewport) => {
                let offset_y = viewport.absolute_offset().y;
                let width = viewport.content_bounds().width;
                self.on_viewport(offset_y, width);
            }
            Message::ToggleGroup(key) => {
                if !self.expanded.remove(&key) {
                    self.expanded.insert(key);
                }
                self.rebuild_rows();
            }
            Message::Copy(_) | Message::LinkClicked(_) | Message::Pending(_) => {}
        }
    }

    /// Folds one viewport sample into scroll and sticky state. Tests in
    /// this module drive the reducer through this fn directly: a
    /// `Viewport` has no public constructor, so `update` itself is a
    /// trivial delegation that headless tests cannot feed.
    fn on_viewport(&mut self, offset_y: f32, content_width: f32) {
        self.scroll_offset = offset_y;
        self.content_width = content_width;
        if content_width <= 0.0 {
            self.sticky = None;
            self.sticky_push = 0.0;
            self.sticky_height = 0.0;
            return;
        }
        let tops = row_tops(&self.rows, &self.entries, content_width);
        // Rule: nothing sticks while the offset sits at or above the top
        // of the first user prompt. There the overlay would exactly cover
        // the natural bubble, so leaving it out is cleaner.
        let first_top = self.rows.iter().enumerate().find_map(|(row, item)| {
            if row_is_user(item, &self.entries) {
                Some(tops[row])
            } else {
                None
            }
        });
        let mut current = None;
        if first_top.is_none_or(|top| offset_y > top) {
            for (row, item) in self.rows.iter().enumerate() {
                if row_is_user(item, &self.entries) && tops[row] <= offset_y {
                    current = Some(row);
                }
            }
        }
        match current {
            None => {
                self.sticky = None;
                self.sticky_push = 0.0;
                self.sticky_height = 0.0;
            }
            Some(row) => {
                let height = row_height(&self.rows[row], &self.entries, content_width);
                let next_top =
                    self.rows
                        .iter()
                        .enumerate()
                        .skip(row + 1)
                        .find_map(|(next, item)| {
                            if row_is_user(item, &self.entries) {
                                Some(tops[next])
                            } else {
                                None
                            }
                        });
                let push = match next_top {
                    Some(top) => {
                        let gap = top - offset_y;
                        if gap < height {
                            (height - gap).clamp(0.0, height)
                        } else {
                            0.0
                        }
                    }
                    None => 0.0,
                };
                self.sticky = Some(row);
                self.sticky_push = push;
                self.sticky_height = height;
            }
        }
    }

    /// Renders the scrollable transcript with the sticky prompt overlay.
    /// User prompts render as filled bubbles; assistant markdown sits in
    /// fill-width containers so long lines wrap (UI contract §4). The
    /// overlay is display-only: containers and the pin forward events
    /// without capturing them, so wheel and pointer input still reach
    /// the scrollable beneath.
    #[allow(dead_code)]
    pub fn view(&self) -> Element<'_, Message> {
        self.view_impl(None, None)
    }

    /// Like [`view`](Self::view), with inline pending cards from
    /// `pending_state` for the visible `session`.
    pub fn view_with_pending<'a>(
        &'a self,
        pending_state: &'a pending::State,
        session: Option<&'a SessionId>,
    ) -> Element<'a, Message> {
        self.view_impl(Some(pending_state), session)
    }

    fn view_impl<'a>(
        &'a self,
        pending_state: Option<&'a pending::State>,
        session: Option<&'a SessionId>,
    ) -> Element<'a, Message> {
        // The stack root is unconditional so the scrollable keeps its widget state across sticky changes.
        let mut list = iced::widget::column![].width(Length::Fill);
        for (row, item) in self.rows.iter().enumerate() {
            // The column carries no spacing of its own; every inter-row
            // gap is an explicit spacer sized by `row_gap_between`, the
            // same fn `row_tops` accumulates, so view and estimator cannot
            // drift.
            if row > 0 {
                list = list.push(iced::widget::Space::new().height(Length::Fixed(
                    row_gap_between(&self.rows[row - 1], item, &self.entries),
                )));
            }
            list = list.push(render_row(
                item,
                &self.entries,
                self.settings,
                pending_state,
                session,
            ));
        }
        let base: Element<'_, Message> = iced::widget::scrollable(list)
            .id(SCROLL_ID)
            .width(Length::Fill)
            .height(Length::Fill)
            .on_scroll(Message::Scrolled)
            .into();
        let mut stack = iced::widget::Stack::new()
            .push(base)
            .width(Length::Fill)
            .height(Length::Fill);
        if let Some(Row::Single(index)) = self.sticky.and_then(|row| self.rows.get(row))
            && let EntryKind::User { text } = &self.entries[*index].kind
        {
            // The bubble is the window: it is sized to exactly the visible
            // slice (the gap to the incoming prompt while pushing, the full
            // height once settled) and its text slides up inside via
            // negative top padding, clipping at the window's top edge. A
            // styled container quad is never scissored to a clip rect, so
            // no styled bounds may extend past the clip window.
            let overlay: Element<'_, Message> = kit::sticky_bubble(
                text.as_str(),
                (self.sticky_height - self.sticky_push).max(0.0),
                self.sticky_push,
            );
            stack = stack.push(overlay);
        }
        stack.into()
    }
}

/// Refreshes an existing entry from a started item without wiping
/// streamed content: empty started values keep the buffer. A started
/// item whose variant no longer matches the entry replaces the entry
/// payload (the host reuses an id for one item; this is defensive).
fn refresh_entry(entry: &mut Entry, item: &StreamItem) {
    if !entry_matches(entry, item) {
        if let Some(fresh) = entry_from_item(item, false) {
            entry.kind = fresh.kind;
            entry.completed = false;
        }
        return;
    }
    match (&mut entry.kind, item) {
        (EntryKind::User { text }, StreamItem::UserMessage { text: next, .. }) => {
            if !next.is_empty() {
                *text = next.clone();
            }
        }
        (EntryKind::Assistant { text, markdown }, StreamItem::AgentMessage { text: next, .. }) => {
            if !next.is_empty() {
                *text = next.clone();
                *markdown = markdown::Content::parse(next);
            }
        }
        (EntryKind::Reasoning { summary }, StreamItem::Reasoning { summary: next, .. }) => {
            if !next.is_empty() {
                *summary = next.clone();
            }
        }
        (
            EntryKind::Note { text },
            StreamItem::Compaction {
                outcome,
                reason,
                tokens_before,
                tokens_after,
                ..
            },
        ) => {
            *text = friendly_compaction(outcome, reason, tokens_before, tokens_after);
        }
        (
            EntryKind::Tool {
                tool,
                headline,
                output,
                status,
                failure,
            },
            StreamItem::ToolCall {
                tool: next_tool,
                headline: next_headline,
                output: next_output,
                status: next_status,
                failure: next_failure,
                ..
            },
        ) => {
            if !next_tool.is_empty() {
                *tool = next_tool.clone();
            }
            if next_headline.is_some() {
                *headline = next_headline.clone();
            }
            if let Some(next_output) = next_output
                && !next_output.is_empty()
            {
                *output = next_output.clone();
            }
            *status = next_status.clone();
            *failure = next_failure.clone();
        }
        _ => {}
    }
}

/// Whether a row is a lone user prompt (the only sticky row kind).
fn row_is_user(row: &Row, entries: &[Entry]) -> bool {
    matches!(row, Row::Single(index) if is_user(&entries[*index]))
}

/// Greedy word wrap by character count. `max_chars` is the widest line
/// in characters; words longer than that occupy the ceiling of their
/// length over `max_chars`. Empty text is one line. Counts characters
/// with `.chars().count()`, never bytes.
fn wrapped_lines(text: &str, text_width: f32) -> usize {
    let max_chars = ((text_width / ESTIMATED_CHAR_WIDTH) as usize).max(1);
    if text.is_empty() {
        return 1;
    }
    let mut lines = 1;
    let mut used = 0;
    for word in text.split_whitespace() {
        let len = word.chars().count();
        if len > max_chars {
            if used > 0 {
                lines += 1;
            }
            lines += len.div_ceil(max_chars) - 1;
            used = match len % max_chars {
                0 => max_chars,
                rest => rest,
            };
        } else if used == 0 {
            used = len;
        } else if used + 1 + len <= max_chars {
            used += 1 + len;
        } else {
            lines += 1;
            used = len;
        }
    }
    lines
}

/// Estimated entry height in px. A user bubble is narrower by its
/// horizontal padding (see [`kit::prompt_bubble`]) and taller by its
/// vertical padding; assistant text spans the full content width with a
/// Copy row under completed messages; tool entries stack one headline
/// line over wrapped output and failure lines. These constants must
/// match the kit constructors and the column spacing.
fn entry_height(entry: &Entry, content_width: f32) -> f32 {
    let full_width = content_width.max(ESTIMATED_CHAR_WIDTH);
    match &entry.kind {
        EntryKind::User { text } => {
            let text_width = (content_width - 2.0 * SPACE_XL).max(ESTIMATED_CHAR_WIDTH);
            wrapped_lines(text, text_width) as f32 * LINE_HEIGHT + 2.0 * SPACE_L
        }
        EntryKind::Assistant { text, .. } => {
            let copy = if entry.completed {
                COPY_ROW_HEIGHT
            } else {
                0.0
            };
            wrapped_lines(text, full_width) as f32 * LINE_HEIGHT + copy
        }
        EntryKind::Reasoning { summary } => {
            wrapped_lines(&reasoning_text(summary), full_width) as f32 * LINE_HEIGHT
        }
        EntryKind::Tool {
            output, failure, ..
        } => {
            let mut height = LINE_HEIGHT;
            if !output.is_empty() {
                height += wrapped_lines(output, full_width) as f32 * LINE_HEIGHT;
            }
            if let Some(failure) = failure
                && !failure.is_empty()
            {
                height += wrapped_lines(failure, full_width) as f32 * LINE_HEIGHT;
            }
            height
        }
        EntryKind::Note { text } | EntryKind::Error { text } => {
            wrapped_lines(text, full_width) as f32 * LINE_HEIGHT
        }
        EntryKind::Pending { .. } => PENDING_CARD_HEIGHT,
    }
}

/// Estimated row height in px. A collapsed group is one header row;
/// an expanded group stacks its header over its members with inner
/// single-spacing gaps above each member.
fn row_height(row: &Row, entries: &[Entry], content_width: f32) -> f32 {
    match row {
        Row::Single(index) => entry_height(&entries[*index], content_width),
        Row::Group {
            first,
            last,
            expanded,
            ..
        } => {
            if !expanded {
                return GROUP_HEADER_HEIGHT;
            }
            let members: f32 = (*first..=*last)
                .map(|index| entry_height(&entries[index], content_width))
                .sum();
            GROUP_HEADER_HEIGHT + (last - first + 1) as f32 * SPACE_M + members
        }
    }
}

/// Gap between two adjacent rows: double column spacing around a lone
/// user prompt (above and below it), single spacing otherwise.
/// Single source of truth for the view's extra spacer and `row_tops`.
fn row_gap_between(previous: &Row, current: &Row, entries: &[Entry]) -> f32 {
    if row_is_user(previous, entries) || row_is_user(current, entries) {
        2.0 * SPACE_M
    } else {
        SPACE_M
    }
}

/// Top edge of every row in px. Rows stack with [`row_gap_between`] of
/// the neighboring pair between them, no spacing before the first, and
/// the content carries no other padding.
fn row_tops(rows: &[Row], entries: &[Entry], content_width: f32) -> Vec<f32> {
    let mut tops = Vec::with_capacity(rows.len());
    let mut top = 0.0;
    for (index, row) in rows.iter().enumerate() {
        if index > 0 {
            top += row_gap_between(&rows[index - 1], row, entries);
        }
        tops.push(top);
        top += row_height(row, entries, content_width);
    }
    tops
}

/// Renders one row: a lone entry, or a group header with its members
/// below it while expanded.
fn render_row<'a>(
    row: &'a Row,
    entries: &'a [Entry],
    settings: markdown::Settings,
    pending_state: Option<&'a pending::State>,
    session: Option<&'a SessionId>,
) -> Element<'a, Message> {
    match row {
        Row::Single(index) => render_entry(&entries[*index], settings, pending_state, session),
        Row::Group {
            first,
            last,
            key,
            headline,
            expanded,
        } => {
            let mut column = iced::widget::column![kit::group_header(
                headline,
                *expanded,
                Message::ToggleGroup(key.clone())
            )]
            .width(Length::Fill);
            if *expanded {
                for entry in entries.iter().take(*last + 1).skip(*first) {
                    column = column.push(render_entry(entry, settings, pending_state, session));
                }
            }
            column.into()
        }
    }
}

/// Renders one entry, distinct by kind. Body text is borrowed, never
/// cloned, except the owned copies message payloads carry (Elm messages
/// own their data): Copy buttons clone the text they will write.
fn render_entry<'a>(
    entry: &'a Entry,
    settings: markdown::Settings,
    pending_state: Option<&'a pending::State>,
    session: Option<&'a SessionId>,
) -> Element<'a, Message> {
    match &entry.kind {
        EntryKind::User { text } => kit::prompt_bubble(text.as_str()),
        EntryKind::Assistant { text, markdown } => {
            let mut column =
                iced::widget::column![markdown::view_with(markdown.items(), settings, &Viewer)]
                    .width(Length::Fill);
            if entry.completed {
                column = column.push(kit::tooltip_icon_button(
                    lucide_icons::Icon::Copy,
                    kit::IconButtonKind::Subtle,
                    "Copy",
                    Some(Message::Copy(text.clone())),
                ));
            }
            iced::widget::container(column).width(Length::Fill).into()
        }
        EntryKind::Reasoning { summary } => iced::widget::container(
            iced::widget::text(reasoning_text(summary))
                .size(FONT_SIZE_BODY)
                .color(TEXT_SECONDARY),
        )
        .width(Length::Fill)
        .into(),
        EntryKind::Tool {
            tool,
            headline,
            output,
            failure,
            ..
        } => {
            let name = if tool.is_empty() {
                "Tool"
            } else {
                tool.as_str()
            };
            let header = match headline {
                Some(line) if !line.is_empty() => format!("{name}{MIDDOT}{line}"),
                _ => name.to_string(),
            };
            let mut column = iced::widget::column![
                iced::widget::text(header)
                    .size(FONT_SIZE_BODY)
                    .color(TEXT_PRIMARY),
            ]
            .width(Length::Fill);
            if !output.is_empty() {
                column = column.push(
                    iced::widget::container(
                        iced::widget::text(output.as_str())
                            .size(FONT_SIZE_BODY)
                            .color(TEXT_PRIMARY)
                            .font(iced::Font::MONOSPACE),
                    )
                    .width(Length::Fill),
                );
            }
            if let Some(failure) = failure
                && !failure.is_empty()
            {
                column = column.push(
                    iced::widget::container(
                        iced::widget::text(failure.as_str())
                            .size(FONT_SIZE_BODY)
                            .color(DANGER),
                    )
                    .width(Length::Fill),
                );
            }
            column.into()
        }
        EntryKind::Note { text } => iced::widget::container(
            iced::widget::text(text.as_str())
                .size(FONT_SIZE_BODY)
                .color(TEXT_TERTIARY),
        )
        .width(Length::Fill)
        .into(),
        EntryKind::Error { text } => iced::widget::container(
            iced::widget::text(text.as_str())
                .size(FONT_SIZE_BODY)
                .color(DANGER),
        )
        .width(Length::Fill)
        .into(),
        EntryKind::Pending { request_id } => {
            match pending_state.and_then(|state| pending_element(state, session, request_id)) {
                Some(element) => element,
                None => kit::static_pill("Needs your input"),
            }
        }
    }
}

/// Markdown viewer for transcript messages: links report their URI and
/// code blocks stack a header row (language label when present plus a
/// Copy button carrying the raw code) over the default rendering.
struct Viewer;

impl<'a> markdown::Viewer<'a, Message> for Viewer {
    fn on_link_click(url: markdown::Uri) -> Message {
        Message::LinkClicked(url)
    }

    fn code_block(
        &self,
        settings: markdown::Settings,
        language: Option<&'a str>,
        code: &'a str,
        lines: &'a [markdown::Text],
    ) -> Element<'a, Message> {
        let mut header = iced::widget::row![].spacing(SPACE_M).width(Length::Fill);
        if let Some(language) = language
            && !language.is_empty()
        {
            header = header.push(
                iced::widget::text(language)
                    .size(FONT_SIZE_SMALL)
                    .color(TEXT_TERTIARY),
            );
        }
        header = header.push(
            iced::widget::container(iced::widget::text("").size(FONT_SIZE_SMALL))
                .width(Length::Fill),
        );
        header = header.push(kit::tooltip_icon_button(
            lucide_icons::Icon::Copy,
            kit::IconButtonKind::Subtle,
            "Copy",
            Some(Message::Copy(code.to_string())),
        ));
        iced::widget::column![
            header,
            markdown::code_block(settings, lines, Self::on_link_click),
        ]
        .spacing(SPACE_M)
        .width(Length::Fill)
        .into()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ESTIMATED_CHAR_WIDTH, Entry, EntryKind, Row, State, friendly_unknown, group_headline,
        row_gap_between, row_height, row_tops, wrapped_lines,
    };
    use crate::theme::{SPACE_L, SPACE_M};
    use manifest_domain::{ItemStatus, StreamItem, fixture};

    /// Indices of the single user rows in display order.
    fn user_rows(state: &State) -> Vec<usize> {
        state
            .rows
            .iter()
            .enumerate()
            .filter(|(_, row)| {
                matches!(row, Row::Single(index) if matches!(state.entries[*index].kind, EntryKind::User { .. }))
            })
            .map(|(row, _)| row)
            .collect()
    }

    fn user_item(id: &str, text: &str) -> StreamItem {
        StreamItem::UserMessage {
            item_id: String::from(id),
            text: String::from(text),
        }
    }

    fn agent_item(id: &str, text: &str) -> StreamItem {
        StreamItem::AgentMessage {
            item_id: String::from(id),
            text: String::from(text),
            phase: None,
        }
    }

    fn reasoning_item(id: &str, summary: &[&str]) -> StreamItem {
        StreamItem::Reasoning {
            item_id: String::from(id),
            summary: summary.iter().map(|line| String::from(*line)).collect(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn tool_item(
        id: &str,
        tool: &str,
        headline: Option<&str>,
        output: Option<&str>,
        status: ItemStatus,
        failure: Option<&str>,
    ) -> StreamItem {
        StreamItem::ToolCall {
            item_id: String::from(id),
            tool: String::from(tool),
            headline: headline.map(String::from),
            args_json: String::new(),
            todos: None,
            status,
            output: output.map(String::from),
            failure: failure.map(String::from),
        }
    }

    fn system_item(id: &str) -> StreamItem {
        StreamItem::System {
            item_id: String::from(id),
            kind: String::from("reminderChild"),
        }
    }

    fn unknown_item(id: &str) -> StreamItem {
        StreamItem::Unknown {
            item_id: String::from(id),
            kind: String::from("from-the-future"),
        }
    }

    fn compaction_item(
        id: &str,
        outcome: Option<&str>,
        reason: Option<&str>,
        tokens_before: Option<u64>,
        tokens_after: Option<u64>,
    ) -> StreamItem {
        StreamItem::Compaction {
            item_id: String::from(id),
            outcome: outcome.map(String::from),
            reason: reason.map(String::from),
            tokens_before,
            tokens_after,
        }
    }

    fn note_text(state: &State) -> &str {
        match &state.entries[0].kind {
            EntryKind::Note { text } => text,
            other => panic!("expected a note entry, got {other:?}"),
        }
    }

    #[test]
    fn short_text_stays_on_one_line() {
        assert_eq!(wrapped_lines("Hello", 600.0), 1);
    }

    #[test]
    fn wrapping_grows_lines_as_width_shrinks() {
        let text = "The refresh now runs inside the request path";
        assert_eq!(wrapped_lines(text, 600.0), 1);
        assert!(wrapped_lines(text, 100.0) > 1);
    }

    #[test]
    fn overlong_word_splits_across_lines() {
        let word = "a".repeat(50);
        assert_eq!(wrapped_lines(&word, 600.0), 1);
        let max_chars = (100.0 / ESTIMATED_CHAR_WIDTH) as usize;
        let expected = 50_usize.div_ceil(max_chars);
        assert_eq!(wrapped_lines(&word, 100.0), expected);
    }

    #[test]
    fn empty_text_is_one_line() {
        assert_eq!(wrapped_lines("", 600.0), 1);
    }

    #[test]
    fn user_bubble_is_taller_by_vertical_padding() {
        let user = Entry {
            host_id: None,
            kind: EntryKind::User {
                text: String::from("Short line"),
            },
            completed: true,
        };
        let note = Entry {
            host_id: None,
            kind: EntryKind::Note {
                text: String::from("Short line"),
            },
            completed: true,
        };
        let width = 600.0;
        let gap = super::entry_height(&user, width) - super::entry_height(&note, width);
        assert!((gap - 2.0 * SPACE_L).abs() < 0.001);
    }

    #[test]
    fn tops_accumulate_height_plus_spacing() {
        let state = State::from_items(fixture().transcript);
        let tops = row_tops(&state.rows, &state.entries, 600.0);
        assert_eq!(tops.len(), state.rows.len());
        assert_eq!(tops[0], 0.0);
        for (index, top) in tops.iter().enumerate().skip(1) {
            let want = tops[index - 1]
                + row_height(&state.rows[index - 1], &state.entries, 600.0)
                + row_gap_between(&state.rows[index - 1], &state.rows[index], &state.entries);
            assert!((top - want).abs() < 0.001);
        }
    }

    #[test]
    fn gaps_are_double_around_user_rows_and_single_otherwise() {
        let user = Entry {
            host_id: None,
            kind: EntryKind::User {
                text: String::from("prompt"),
            },
            completed: true,
        };
        let note = Entry {
            host_id: None,
            kind: EntryKind::Note {
                text: String::from("reply"),
            },
            completed: true,
        };
        let rows = [
            Row::Single(0),
            Row::Single(1),
            Row::Single(2),
            Row::Single(3),
        ];
        let entries = [
            user,
            note,
            Entry {
                host_id: None,
                kind: EntryKind::Note {
                    text: String::from("more"),
                },
                completed: true,
            },
            Entry {
                host_id: None,
                kind: EntryKind::User {
                    text: String::from("again"),
                },
                completed: true,
            },
        ];
        // Double below and above a user row.
        assert_eq!(row_gap_between(&rows[0], &rows[1], &entries), 2.0 * SPACE_M);
        assert_eq!(row_gap_between(&rows[2], &rows[3], &entries), 2.0 * SPACE_M);
        // Single between two non-user rows.
        assert_eq!(row_gap_between(&rows[1], &rows[2], &entries), SPACE_M);
    }

    #[test]
    fn offset_zero_shows_no_sticky_for_leading_prompt() {
        let mut state = State::from_items(fixture().transcript);
        state.on_viewport(0.0, 600.0);
        assert_eq!(state.scroll_offset, 0.0);
        assert_eq!(state.content_width, 600.0);
        assert_eq!(state.sticky, None);
        assert_eq!(state.sticky_push, 0.0);
        assert_eq!(state.sticky_height, 0.0);
    }

    #[test]
    fn scrolled_past_first_prompt_sticks_it() {
        let mut state = State::from_items(fixture().transcript);
        let tops = row_tops(&state.rows, &state.entries, 600.0);
        let users = user_rows(&state);
        assert!(users.len() >= 2);
        state.on_viewport(tops[users[0]] + 1.0, 600.0);
        assert_eq!(state.sticky, Some(users[0]));
        assert_eq!(state.sticky_push, 0.0);
        assert!(state.sticky_height > 0.0);
    }

    #[test]
    fn incoming_prompt_pushes_sticky_up() {
        let mut state = State::from_items(fixture().transcript);
        let tops = row_tops(&state.rows, &state.entries, 600.0);
        let users = user_rows(&state);
        let height = row_height(&state.rows[users[0]], &state.entries, 600.0);
        let offset = tops[users[1]] - height / 2.0;
        state.on_viewport(offset, 600.0);
        assert_eq!(state.sticky, Some(users[0]));
        assert!(state.sticky_push > 0.0 && state.sticky_push < state.sticky_height);
    }

    #[test]
    fn arriving_at_next_prompt_switches_sticky() {
        let mut state = State::from_items(fixture().transcript);
        let tops = row_tops(&state.rows, &state.entries, 600.0);
        let users = user_rows(&state);
        state.on_viewport(tops[users[1]], 600.0);
        assert_eq!(state.sticky, Some(users[1]));
        assert_eq!(state.sticky_push, 0.0);
    }

    #[test]
    fn narrower_width_recomputes_sticky() {
        let mut state = State::from_items(fixture().transcript);
        let users = user_rows(&state);
        let tops_wide = row_tops(&state.rows, &state.entries, 600.0);
        let height_wide = row_height(&state.rows[users[0]], &state.entries, 600.0);
        let offset = tops_wide[users[1]] - height_wide / 2.0;
        state.on_viewport(offset, 600.0);
        assert_eq!(state.sticky, Some(users[0]));
        state.on_viewport(offset, 200.0);
        assert_eq!(state.content_width, 200.0);
        assert_eq!(state.sticky, Some(users[0]));
        assert!(state.sticky_height > height_wide);
    }

    #[test]
    fn zero_width_clears_sticky() {
        let mut state = State::from_items(fixture().transcript);
        state.on_viewport(50.0, 600.0);
        assert!(state.sticky.is_some());
        state.on_viewport(50.0, 0.0);
        assert_eq!(state.sticky, None);
        assert_eq!(state.sticky_push, 0.0);
        assert_eq!(state.sticky_height, 0.0);
    }

    #[test]
    fn empty_transcript_never_sticks() {
        let mut state = State::from_items(vec![]);
        state.on_viewport(100.0, 600.0);
        assert_eq!(state.sticky, None);
        assert_eq!(state.sticky_push, 0.0);
        assert_eq!(state.sticky_height, 0.0);
    }

    #[test]
    fn push_error_dedupes_consecutive_duplicates() {
        let mut state = State::from_items(vec![]);
        assert!(state.push_error(String::from("boom")));
        assert!(!state.push_error(String::from("boom")));
        assert_eq!(state.entries.len(), 1);
        assert!(state.push_error(String::from("other")));
        assert_eq!(state.entries.len(), 2);
        assert!(matches!(
            state.entries.last(),
            Some(Entry {
                kind: EntryKind::Error { text },
                ..
            }) if text == "other"
        ));
    }

    #[test]
    fn pending_entry_renders_inline_card_and_uses_card_height() {
        use manifest_domain::{ApprovalRequest, PendingRequest, SessionId};
        let mut pending_state = crate::pending::State::default();
        pending_state.apply_loaded(vec![PendingRequest::Approval {
            session: SessionId::new("s-1"),
            request_id: String::from("r1"),
            approval: ApprovalRequest {
                tool: String::from("bash"),
                subject_line: String::from("ls"),
                choices: Vec::new(),
            },
        }]);
        let mut state = State::from_items(vec![]);
        assert!(state.push_pending(String::from("r1")));
        let session = SessionId::new("s-1");
        assert!(super::pending_element(&pending_state, Some(&session), "r1").is_some());
        assert!(
            super::pending_element(&pending_state, Some(&SessionId::new("s-2")), "r1").is_none()
        );
        assert!(super::pending_element(&pending_state, Some(&session), "missing").is_none());
        let entry = &state.entries[0];
        assert_eq!(
            super::entry_height(entry, 600.0),
            super::PENDING_CARD_HEIGHT
        );
        // The full view builds with the inline card present.
        let _ = state.view_with_pending(&pending_state, Some(&session));
        let _ = state.view();
    }

    #[test]
    fn pending_entry_inserts_dedupes_and_collapses_in_place() {
        let mut state = State::from_items(vec![]);
        assert!(state.push_pending(String::from("r1")));
        assert!(!state.push_pending(String::from("r1")));
        assert_eq!(state.entries.len(), 1);
        assert!(state.has_pending("r1"));
        assert!(state.collapse_pending("r1", String::from("Approval · ls — Approved by you")));
        assert!(!state.has_pending("r1"));
        assert!(matches!(
            &state.entries[0].kind,
            EntryKind::Note { text } if text.contains("Approved by you")
        ));
        assert!(state.entries[0].completed);
        assert!(!state.collapse_pending("missing", String::from("x")));
    }

    #[test]
    fn started_deltas_completed_replaces_buffer_with_authoritative_text() {
        let mut state = State::default();
        state.item_started(agent_item("a-1", ""));
        state.item_delta("a-1", "text", "strea");
        state.item_delta("a-1", "text", "ming");
        assert!(matches!(
            &state.entries[0].kind,
            EntryKind::Assistant { text, .. } if text == "streaming"
        ));
        assert!(!state.entries[0].completed);
        state.item_completed(agent_item("a-1", "final **answer**"));
        assert!(matches!(
            &state.entries[0].kind,
            EntryKind::Assistant { text, .. } if text == "final **answer**"
        ));
        assert!(state.entries[0].completed);
        assert_eq!(state.entries.len(), 1);
    }

    #[test]
    fn empty_started_text_keeps_streamed_buffer() {
        let mut state = State::default();
        // A duplicate started event with empty text must not erase the
        // buffer deltas already filled.
        state.item_started(agent_item("a-1", ""));
        state.item_delta("a-1", "text", "partial");
        state.item_started(agent_item("a-1", ""));
        assert!(matches!(
            &state.entries[0].kind,
            EntryKind::Assistant { text, .. } if text == "partial"
        ));
        assert_eq!(state.entries.len(), 1);
    }

    #[test]
    fn reasoning_summary_grows_to_indexed_slot() {
        let mut state = State::default();
        state.item_started(reasoning_item("r-1", &[]));
        state.item_delta("r-1", "summary.1", "second");
        assert!(matches!(
            &state.entries[0].kind,
            EntryKind::Reasoning { summary }
                if summary.len() == 2 && summary[0].is_empty() && summary[1] == "second"
        ));
        state.item_completed(reasoning_item("r-1", &["only"]));
        assert!(matches!(
            &state.entries[0].kind,
            EntryKind::Reasoning { summary } if summary == &vec![String::from("only")]
        ));
    }

    #[test]
    fn tool_output_delta_then_completed_with_output_overrides() {
        let mut state = State::default();
        state.item_started(tool_item(
            "t-1",
            "bash",
            Some("ls"),
            None,
            ItemStatus::InProgress,
            None,
        ));
        state.item_delta("t-1", "output", "preview");
        assert!(matches!(
            &state.entries[0].kind,
            EntryKind::Tool { output, .. } if output == "preview"
        ));
        state.item_completed(tool_item(
            "t-1",
            "bash",
            Some("ls"),
            Some("full output"),
            ItemStatus::Completed,
            None,
        ));
        assert!(matches!(
            &state.entries[0].kind,
            EntryKind::Tool { output, status, .. }
                if output == "full output" && *status == ItemStatus::Completed
        ));
        assert!(state.entries[0].completed);
    }

    #[test]
    fn tool_completed_without_output_keeps_buffer() {
        let mut state = State::default();
        state.item_started(tool_item(
            "t-1",
            "bash",
            Some("ls"),
            None,
            ItemStatus::InProgress,
            None,
        ));
        state.item_delta("t-1", "output", "preview");
        state.item_completed(tool_item(
            "t-1",
            "bash",
            Some("ls"),
            None,
            ItemStatus::Completed,
            None,
        ));
        assert!(matches!(
            &state.entries[0].kind,
            EntryKind::Tool { output, .. } if output == "preview"
        ));
    }

    #[test]
    fn user_message_echo_claims_optimistic_entry() {
        let mut state = State::default();
        state.push_user(String::from("hello"));
        state.item_completed(user_item("u-1", "hello"));
        assert_eq!(state.entries.len(), 1);
        assert_eq!(state.entries[0].host_id, Some(String::from("u-1")));
        assert!(state.entries[0].completed);
    }

    #[test]
    fn user_message_without_echo_appends() {
        let mut state = State::default();
        state.push_user(String::from("other"));
        state.item_completed(user_item("u-1", "hello"));
        assert_eq!(state.entries.len(), 2);
        assert!(matches!(
            &state.entries[1].kind,
            EntryKind::User { text } if text == "hello"
        ));
    }

    #[test]
    fn completed_without_started_creates_entry() {
        let mut state = State::default();
        state.item_completed(agent_item("a-9", "late arrival"));
        assert_eq!(state.entries.len(), 1);
        assert_eq!(state.entries[0].host_id, Some(String::from("a-9")));
        assert!(state.entries[0].completed);
    }

    #[test]
    fn system_items_produce_no_entry() {
        let mut state = State::default();
        state.item_started(system_item("s-1"));
        state.item_completed(system_item("s-1"));
        assert!(state.entries.is_empty());
        assert!(state.rows.is_empty());
    }

    #[test]
    fn seed_history_appends_in_order_completed() {
        let mut state = State::default();
        assert!(state.seed_history(vec![
            user_item("h-u1", "First question"),
            agent_item("h-a1", "First answer."),
        ]));
        assert_eq!(state.entries.len(), 2);
        assert!(matches!(
            &state.entries[0].kind,
            EntryKind::User { text } if text == "First question"
        ));
        assert!(matches!(
            &state.entries[1].kind,
            EntryKind::Assistant { text, .. } if text == "First answer."
        ));
        assert!(state.entries.iter().all(|entry| entry.completed));
        assert_eq!(state.entries[0].host_id, Some(String::from("h-u1")));
    }

    #[test]
    fn seed_history_prepends_ahead_of_live_entries() {
        let mut state = State::default();
        state.item_completed(agent_item("live-1", "Live answer."));
        assert!(state.seed_history(vec![
            user_item("h-u1", "First question"),
            agent_item("h-a1", "First answer."),
        ]));
        assert_eq!(
            state
                .entries
                .iter()
                .map(|entry| entry.host_id.clone())
                .collect::<Vec<_>>(),
            vec![
                Some(String::from("h-u1")),
                Some(String::from("h-a1")),
                Some(String::from("live-1")),
            ]
        );
    }

    #[test]
    fn seed_history_skips_present_ids_and_system_items() {
        let mut state = State::default();
        state.item_completed(agent_item("h-a1", "Streamed answer."));
        assert!(state.seed_history(vec![
            agent_item("h-a1", "Seeded answer."),
            system_item("h-s1"),
            user_item("h-u1", "First question"),
        ]));
        // The live entry keeps its streamed text; the seed fills only
        // the gap, ahead of it.
        assert_eq!(
            state
                .entries
                .iter()
                .map(|entry| entry.host_id.clone())
                .collect::<Vec<_>>(),
            vec![Some(String::from("h-u1")), Some(String::from("h-a1"))]
        );
        assert!(matches!(
            &state.entries[1].kind,
            EntryKind::Assistant { text, .. } if text == "Streamed answer."
        ));
        // A repeat seed finds everything present and adds nothing.
        assert!(!state.seed_history(vec![
            user_item("h-u1", "First question"),
            agent_item("h-a1", "Seeded answer."),
        ]));
        assert_eq!(state.entries.len(), 2);
        // System-only seeds add nothing.
        assert!(!state.seed_history(vec![system_item("h-s2")]));
    }

    #[test]
    fn unknown_items_produce_a_named_note() {
        let mut state = State::default();
        state.item_started(unknown_item("x-1"));
        assert!(matches!(
            &state.entries[0].kind,
            EntryKind::Note { text } if text == &friendly_unknown("from-the-future")
        ));
        assert!(!state.entries[0].completed);
        state.item_completed(unknown_item("x-1"));
        assert_eq!(state.entries.len(), 1);
        assert!(state.entries[0].completed);
    }

    #[test]
    fn compaction_compacted_renders_token_counts() {
        let mut state = State::default();
        state.item_started(compaction_item("c-1", None, None, None, None));
        assert_eq!(note_text(&state), "Compacting context…");
        assert!(!state.entries[0].completed);
        state.item_completed(compaction_item(
            "c-1",
            Some("compacted"),
            None,
            Some(48210),
            Some(3912),
        ));
        assert_eq!(state.entries.len(), 1);
        assert_eq!(note_text(&state), "Context compacted · 48210 → 3912 tokens");
        assert!(state.entries[0].completed);
    }

    #[test]
    fn compaction_failed_renders_reason() {
        let mut state = State::default();
        state.item_completed(compaction_item(
            "c-2",
            Some("failed"),
            Some("summarizer_failed"),
            None,
            None,
        ));
        assert_eq!(note_text(&state), "Compaction failed: summarizer_failed");
        assert!(state.entries[0].completed);
    }

    #[test]
    fn compaction_noop_cancelled_and_open_outcomes_render() {
        for (id, item, want) in [
            (
                "c-3",
                compaction_item(
                    "c-3",
                    Some("noop"),
                    Some("no_compactable_history"),
                    None,
                    None,
                ),
                "Compaction not needed (no_compactable_history)",
            ),
            (
                "c-4",
                compaction_item("c-4", Some("cancelled"), None, None, None),
                "Compaction cancelled",
            ),
            (
                "c-5",
                compaction_item("c-5", Some("pruned"), None, None, None),
                "Compaction finished (pruned)",
            ),
        ] {
            let mut state = State::default();
            state.item_completed(item);
            assert_eq!(note_text(&state), want, "outcome note for {id}");
        }
    }

    #[test]
    fn unknown_delta_field_is_ignored() {
        let mut state = State::default();
        state.item_started(agent_item("a-1", ""));
        state.item_delta("a-1", "bogus", "zzz");
        state.item_delta("missing", "text", "zzz");
        assert!(matches!(
            &state.entries[0].kind,
            EntryKind::Assistant { text, .. } if text.is_empty()
        ));
    }

    #[test]
    fn consecutive_reasoning_and_tools_group_with_pinned_headline() {
        let mut state = State::default();
        state.item_started(reasoning_item("r-1", &["plan"]));
        state.item_started(tool_item(
            "t-1",
            "read_file",
            Some("note.txt"),
            None,
            ItemStatus::InProgress,
            None,
        ));
        state.item_started(tool_item(
            "t-2",
            "bash",
            Some("ls"),
            None,
            ItemStatus::InProgress,
            None,
        ));
        state.item_completed(reasoning_item("r-1", &["plan"]));
        assert_eq!(state.rows.len(), 1);
        assert!(matches!(
            &state.rows[0],
            Row::Group { first: 0, last: 2, headline, expanded: false, .. }
                if headline == "Thinking · 2 tool calls · working"
        ));
        // Completing the tools drops the working suffix.
        state.item_completed(tool_item(
            "t-1",
            "read_file",
            Some("note.txt"),
            Some("out"),
            ItemStatus::Completed,
            None,
        ));
        state.item_completed(tool_item(
            "t-2",
            "bash",
            Some("ls"),
            Some("out"),
            ItemStatus::Completed,
            None,
        ));
        assert!(matches!(
            &state.rows[0],
            Row::Group { headline, .. } if headline == "Thinking · 2 tool calls"
        ));
        assert_eq!(
            group_headline(&state.entries[0..3]),
            "Thinking · 2 tool calls"
        );
    }

    #[test]
    fn single_tool_headline_uses_tool_and_headline() {
        let mut state = State::default();
        state.item_started(tool_item(
            "t-1",
            "read_file",
            Some("note.txt"),
            None,
            ItemStatus::InProgress,
            None,
        ));
        assert!(matches!(
            &state.rows[0],
            Row::Group { first: 0, last: 0, headline, .. }
                if headline == "read_file · note.txt · working"
        ));
        // An empty tool name falls back to the word Tool.
        let mut bare = State::default();
        bare.item_started(tool_item(
            "t-1",
            "",
            None,
            None,
            ItemStatus::InProgress,
            None,
        ));
        assert!(matches!(
            &bare.rows[0],
            Row::Group { headline, .. } if headline == "Tool · working"
        ));
    }

    #[test]
    fn failed_tool_marks_headline() {
        let mut state = State::default();
        state.item_started(tool_item(
            "t-1",
            "read_file",
            Some("gone.txt"),
            None,
            ItemStatus::InProgress,
            None,
        ));
        state.item_completed(tool_item(
            "t-1",
            "read_file",
            Some("gone.txt"),
            Some("tool failed: boom"),
            ItemStatus::Failed,
            Some("boom"),
        ));
        assert!(matches!(
            &state.rows[0],
            Row::Group { headline, .. } if headline == "read_file · gone.txt · failed"
        ));
    }

    #[test]
    fn lone_reasoning_run_headline_is_thinking() {
        let entries = vec![Entry {
            host_id: Some(String::from("r-1")),
            kind: EntryKind::Reasoning {
                summary: vec![String::from("plan")],
            },
            completed: true,
        }];
        assert_eq!(group_headline(&entries), "Thinking");
    }

    #[test]
    fn toggle_group_flips_expanded_in_rows() {
        let mut state = State::default();
        state.item_started(reasoning_item("r-1", &["plan"]));
        state.item_started(tool_item(
            "t-1",
            "bash",
            Some("ls"),
            None,
            ItemStatus::InProgress,
            None,
        ));
        let key = match &state.rows[0] {
            Row::Group { key, expanded, .. } => {
                assert!(!expanded);
                key.clone()
            }
            Row::Single(_) => panic!("reasoning plus tool must group"),
        };
        state.update(super::Message::ToggleGroup(key.clone()));
        assert!(matches!(&state.rows[0], Row::Group { expanded: true, .. }));
        state.update(super::Message::ToggleGroup(key));
        assert!(matches!(
            &state.rows[0],
            Row::Group {
                expanded: false,
                ..
            }
        ));
    }

    #[test]
    fn collapsed_group_row_is_header_height_and_expanded_grows() {
        let mut state = State::default();
        state.item_started(reasoning_item("r-1", &["plan"]));
        state.item_started(tool_item(
            "t-1",
            "bash",
            Some("ls"),
            None,
            ItemStatus::InProgress,
            None,
        ));
        let width = 600.0;
        let collapsed = row_height(&state.rows[0], &state.entries, width);
        assert!((collapsed - super::GROUP_HEADER_HEIGHT).abs() < 0.001);
        let key = match &state.rows[0] {
            Row::Group { key, .. } => key.clone(),
            Row::Single(_) => panic!("reasoning plus tool must group"),
        };
        state.update(super::Message::ToggleGroup(key));
        let expanded = row_height(&state.rows[0], &state.entries, width);
        assert!(expanded > collapsed);
        let tops = row_tops(&state.rows, &state.entries, width);
        assert_eq!(tops[0], 0.0);
    }

    #[test]
    fn sticky_selects_user_rows_across_groups() {
        let mut state = State::default();
        state.push_user(String::from("first"));
        state.item_started(reasoning_item("r-1", &["plan"]));
        state.item_completed(reasoning_item("r-1", &["plan"]));
        state.push_user(String::from("second"));
        // Rows: user, group, user.
        assert_eq!(state.rows.len(), 3);
        let tops = row_tops(&state.rows, &state.entries, 600.0);
        state.on_viewport(tops[2], 600.0);
        assert_eq!(state.sticky, Some(2));
        state.on_viewport(tops[0] + 1.0, 600.0);
        assert_eq!(state.sticky, Some(0));
    }
}
