//! Composer: the draft input cluster at the foot of the main column.
//!
//! The input grows as the user types, the attach and send buttons sit
//! inside the bar, and the sub-bar carries live state: the permission
//! posture, the combined model-effort picker, and the context meter.
//! The App owns the engine and the visible session, so it intercepts
//! the send, stop, attach-picker, and menu-selection messages; the
//! reducer here only folds popup toggles and local display state.

use std::ops::Range;

use iced::widget::text_editor;
use iced::{Element, Length};
use manifest_domain::{
    BuiltinCommand, EffortChoice, ModelChoice, ModelInfo, MusePermission, PermissionMode,
    SkillInfo, UsageSnapshot,
};

use crate::kit::{self, IconButtonKind};
use crate::theme::{
    ATTACH_POPUP_WIDTH, COMPOSER_MAX_LINES, CONTEXT_POPUP_WIDTH, EFFORT_POPUP_WIDTH,
    FONT_SIZE_PANEL, MENU_ROW_HEIGHT, MODEL_LIST_MAX_HEIGHT, MODEL_POPUP_WIDTH,
    PERMISSION_POPUP_WIDTH, SLASH_POPUP_WIDTH, SPACE_S, SPACE_XS, TEXT_FAINT, TEXT_TERTIARY,
};

/// The attach popup has two real situations, so it is an enum, not a
/// bool (UI contract §1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AttachPopup {
    /// No attach popup is showing.
    #[default]
    Closed,
    /// The Files / Photos attach popup is showing.
    Open,
}

/// The permission menu has two real situations, so it is an enum, not
/// a bool (UI contract §1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PermissionMenu {
    /// No permission menu is showing.
    #[default]
    Closed,
    /// The permission posture menu is showing.
    Open,
}

/// The model menu has three real situations, so it is an enum, not
/// a bool (UI contract §1): closed, open on its own, and open with
/// the effort side-panel showing beside it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ModelMenu {
    /// No model menu is showing.
    #[default]
    Closed,
    /// The model card is showing on its own.
    Open,
    /// The model card and its effort side-panel are both showing.
    EffortOpen,
}

/// The context popover has two real situations, so it is an enum, not
/// a bool (UI contract §1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ContextPopover {
    /// No context popover is showing.
    #[default]
    Closed,
    /// The usage-breakdown popover is showing.
    Open,
}

/// The model catalog has three real situations, so it is an enum, not
/// a tuple of options (UI contract §1).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ModelCatalog {
    /// The catalog was requested and has not arrived yet.
    #[default]
    Loading,
    /// The catalog arrived; the shell picks from these rows.
    Ready(Vec<ModelInfo>),
    /// The catalog request failed; carries the message. The App
    /// constructs this when a models-tagged `CommandFailed` arrives
    /// while a catalog request is pending (`models_pending` in `app.rs`).
    Failed(String),
}

/// The skill catalog has three real situations, so it is an enum, not
/// a tuple of options (UI contract §1), mirroring [`ModelCatalog`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum SkillCatalog {
    /// The catalog was requested and has not arrived yet.
    #[default]
    Loading,
    /// The catalog arrived; the slash menu picks from these rows.
    Ready(Vec<SkillInfo>),
    /// The catalog request failed; carries the message. The App
    /// constructs this when a skills-tagged `CommandFailed` arrives
    /// while a catalog request is pending (`skills_pending` in `app.rs`).
    Failed(String),
}

/// The slash menu has three real situations, so it is an enum, not a
/// bool (UI contract §1).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum SlashMenu {
    /// No slash menu is showing.
    #[default]
    Closed,
    /// The menu is showing; `selected` indexes [`State::slash_rows`].
    Open {
        /// Selected row, always in range while rows are non-empty.
        selected: usize,
    },
    /// Esc dismissed the menu for this exact draft text: the same text
    /// does not reopen it, and any edit re-arms it.
    Dismissed {
        /// Draft text at dismissal time.
        text: String,
    },
}

/// One slash-menu row's identity, carried by click messages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlashPick {
    /// A hard-coded builtin row.
    Builtin(BuiltinCommand),
    /// A catalog skill row, by selector.
    Skill(String),
}

/// One precomputed visible slash-menu row: builtins first, then the
/// filtered catalog rows. Computed once in `update()` and stored in
/// [`State::slash_rows`], so the view reads without filtering
/// (UI contract §2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlashRow {
    /// Row identity for click messages.
    pub pick: SlashPick,
    /// Slash-menu handle, without the leading `/`.
    pub selector: String,
    /// Precomputed `/selector` display text.
    pub display: String,
    /// Human name: the skill label or the builtin name.
    pub name: String,
    /// One-line account of the row.
    pub description: String,
    /// Catalog scope (`builtin`, `bundled`, `user`, `project`, or
    /// `plugin`); picks the row's source icon.
    pub source: String,
}

/// Messages the composer can receive, enumerated before the view.
#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    /// The editor performed an editing action.
    Edit(text_editor::Action),
    /// The attach button was pressed; toggles the attach popup.
    AttachPressed,
    /// The Files row was pressed; the App opens the OS picker.
    AttachFilesPressed,
    /// The OS file picker returned; `Some` inserts a path ref into
    /// the draft, `None` (dismissed) is a no-op.
    AttachPicked(Option<String>),
    /// A press landed outside the floating attach menu; closes it.
    AttachDismissed,
    /// The send button was pressed; the App sends, spawns, or steers.
    SendPressed,
    /// The stop button was pressed; the App cancels the running turn.
    StopPressed,
    /// The permission pill was pressed; toggles the permission menu.
    PermissionPressed,
    /// A permission posture was picked; closes the menu.
    PermissionSelected(MusePermission),
    /// A press landed outside the floating permission menu; closes it.
    PermissionDismissed,
    /// The model-effort pill was pressed; toggles the model menu.
    ModelPressed,
    /// A model row was picked; selects it and keeps the menu open.
    ModelSelected(ModelChoice),
    /// A row's Edit action was pressed; selects that model and opens
    /// the effort side-panel.
    ModelEditPressed(ModelChoice),
    /// An effort was picked; closes the whole menu.
    EffortSelected(EffortChoice),
    /// A press landed outside the floating model menu; closes it.
    ModelDismissed,
    /// The context meter was pressed; toggles the popover.
    ContextPressed,
    /// A press landed outside the floating context popup; closes it.
    ContextDismissed,
    /// The selection moved down one slash row (wraps); no-op while
    /// the menu is closed or has no rows.
    SlashNext,
    /// The selection moved up one slash row (wraps); no-op while the
    /// menu is closed or has no rows.
    SlashPrev,
    /// The selected slash row was accepted: the draft becomes
    /// `/selector ` and the menu closes. No-op without a selection.
    SlashAccept,
    /// A slash row was clicked: accepts that row's identity.
    SlashSelected(SlashPick),
    /// Esc closed the slash menu for the current draft text; the same
    /// text does not reopen it.
    SlashDismiss,
}

/// Composer state: the draft content plus the live picker state and
/// the derived values the view reads without computing them itself
/// (UI contract §2).
#[derive(Debug, Clone)]
pub struct State {
    /// Text the user has typed but not sent.
    pub content: text_editor::Content,
    /// Visible input lines, grown from the content and clamped to
    /// [`COMPOSER_MAX_LINES`].
    pub visible_lines: usize,
    /// Whether the content holds any text; gates the send button.
    pub has_text: bool,
    /// Whether the send button is live. Owned by the App, which
    /// rewrites it as connection, selection, and in-flight state
    /// change; defaults to true so the button works before the App
    /// runs its first gate refresh.
    pub send_enabled: bool,
    /// Whether the visible session has a turn in flight. Owned by the
    /// App (written by its gate refresh): while set, Send becomes
    /// Steer and the Stop control appears.
    pub running_visible: bool,
    /// Precomputed hint shown while a pending card gates send, e.g.
    /// "Answer the pending card to continue"; `None` hides it. Written
    /// by the App in its gate refresh (UI contract §2).
    pub pending_hint: Option<String>,
    /// Whether the attach popup is showing.
    pub attach_popup: AttachPopup,
    /// Whether the permission menu is showing.
    pub permission_menu: PermissionMenu,
    /// Whether the model menu (and its effort submenu) is showing.
    pub model_menu: ModelMenu,
    /// Whether the context popover is showing.
    pub context_popover: ContextPopover,
    /// The posture new sessions spawn with.
    pub permission: PermissionMode,
    /// The model new sessions spawn with.
    pub model: ModelChoice,
    /// The reasoning effort new sessions spawn with.
    pub effort: EffortChoice,
    /// The host model catalog.
    pub catalog: ModelCatalog,
    /// The host skill catalog.
    pub skill_catalog: SkillCatalog,
    /// Whether the slash menu is showing, and its selection.
    pub slash_menu: SlashMenu,
    /// Precomputed visible slash rows for the current draft query:
    /// builtins first, then filtered catalog rows. The view reads
    /// these without filtering (UI contract §2).
    pub slash_rows: Vec<SlashRow>,
    /// Selectors the input highlights as a leading `/selector` token:
    /// builtins plus the ready catalog's selectors, refreshed with
    /// the catalog. The view derives the editor's highlight settings
    /// from these plus the draft's live token validity, so the editor
    /// re-highlights when either changes.
    pub highlight_selectors: Vec<String>,
    /// Path refs attached to the draft, in pick order.
    pub attachments: Vec<String>,
    /// The visible session's latest usage snapshot, when reported.
    pub usage: Option<UsageSnapshot>,
    /// Precomputed "model · effort" pill text.
    pub model_effort_label: String,
    /// Precomputed context meter text ("used/window · %" or a
    /// no-report fallback).
    pub context_label: String,
    /// Precomputed meter fill in percent, when the window is known.
    pub context_percent: Option<f32>,
}

impl State {
    /// Builds composer state with empty content, one visible line,
    /// the send button enabled, and every menu closed.
    pub fn new() -> Self {
        let mut state = State {
            content: text_editor::Content::new(),
            visible_lines: 1,
            has_text: false,
            send_enabled: true,
            running_visible: false,
            pending_hint: None,
            attach_popup: AttachPopup::Closed,
            permission_menu: PermissionMenu::Closed,
            model_menu: ModelMenu::Closed,
            context_popover: ContextPopover::Closed,
            permission: PermissionMode::default(),
            model: ModelChoice::default(),
            effort: EffortChoice::default(),
            catalog: ModelCatalog::Loading,
            skill_catalog: SkillCatalog::Loading,
            slash_menu: SlashMenu::Closed,
            slash_rows: Vec::new(),
            highlight_selectors: builtin_selectors(),
            attachments: Vec::new(),
            usage: None,
            model_effort_label: String::new(),
            context_label: String::new(),
            context_percent: None,
        };
        state.refresh_derived();
        state
    }

    /// Clears the draft after a send: resets only the draft text,
    /// its derived line/text flags, the attachments, and the
    /// transient popups. The picker state (permission, model,
    /// effort, catalog, usage) and the App-owned
    /// [`send_enabled`](State::send_enabled) and
    /// [`running_visible`](State::running_visible) gates survive a
    /// send, so the user's choices and the context meter carry over
    /// to the next draft. Derived labels are recomputed from the
    /// preserved state.
    pub fn clear(&mut self) {
        self.content = text_editor::Content::new();
        self.visible_lines = 1;
        self.has_text = false;
        self.attachments.clear();
        self.attach_popup = AttachPopup::Closed;
        self.permission_menu = PermissionMenu::Closed;
        self.model_menu = ModelMenu::Closed;
        self.context_popover = ContextPopover::Closed;
        self.close_slash();
        self.refresh_derived();
    }

    /// Restores a failed send's prompt into the draft: the inverse of
    /// [`State::clear`], recomputing the derived values from the text
    /// while preserving the App-owned gates.
    pub fn restore(&mut self, text: String) {
        self.content = text_editor::Content::with_text(&text);
        self.has_text = !self.content.text().is_empty();
        self.visible_lines = self.content.line_count().clamp(1, COMPOSER_MAX_LINES);
        self.refresh_slash_menu();
    }

    /// Recomputes the derived display strings from the picker state.
    /// The single writer of the label fields, called from every fold
    /// that changes them, so the view reads without formatting
    /// (UI contract §2).
    fn refresh_derived(&mut self) {
        self.model_effort_label = format!("{} · {}", self.model_name(), self.effort_name());
        let (label, percent) = match &self.usage {
            None => (String::from("Context —"), None),
            Some(snapshot) => match snapshot.window {
                Some(window) if window > 0 => {
                    let ratio = snapshot.used as f32 / window as f32;
                    (
                        format!(
                            "{} / {} · {}%",
                            snapshot.used,
                            window,
                            (ratio * 100.0).round() as u64
                        ),
                        Some((ratio * 100.0).clamp(0.0, 100.0)),
                    )
                }
                _ => (format!("{} tokens", snapshot.used), None),
            },
        };
        self.context_label = label;
        self.context_percent = percent;
    }

    /// Human name of the selected model: the catalog label when the
    /// selection names a known row, else the choice display.
    fn model_name(&self) -> &str {
        match &self.model {
            ModelChoice::Default => "Default",
            ModelChoice::Named(id) => self
                .catalog_label(id)
                .unwrap_or_else(|| self.model.display()),
        }
    }

    /// Catalog label for a model id, when the catalog knows it.
    fn catalog_label(&self, id: &str) -> Option<&str> {
        match &self.catalog {
            ModelCatalog::Ready(models) => models
                .iter()
                .find(|model| model.id == id)
                .map(ModelInfo::label),
            ModelCatalog::Loading | ModelCatalog::Failed(_) => None,
        }
    }

    /// Short human name of the selected effort.
    fn effort_name(&self) -> &'static str {
        match self.effort {
            EffortChoice::Default => "Default",
            EffortChoice::None => "None",
            EffortChoice::Minimal => "Minimal",
            EffortChoice::Low => "Low",
            EffortChoice::Medium => "Medium",
            EffortChoice::High => "High",
            EffortChoice::XHigh => "XHigh",
            EffortChoice::Max => "Max",
            EffortChoice::Ultra => "Ultra",
        }
    }

    /// Folds a message into the composer state. Editing actions apply
    /// to the content, then the derived values are recomputed and
    /// stored; the App intercepts `SendPressed`, `StopPressed`,
    /// `AttachFilesPressed`, and `AttachPicked` before they reach
    /// this reducer, so those arms stay unreachable here.
    pub fn update(&mut self, message: Message) {
        match message {
            Message::Edit(action) => {
                self.content.perform(action);
                self.has_text = !self.content.text().is_empty();
                self.visible_lines = self.content.line_count().clamp(1, COMPOSER_MAX_LINES);
                self.refresh_slash_menu();
            }
            Message::AttachPressed => {
                self.attach_popup = match self.attach_popup {
                    AttachPopup::Closed => {
                        self.permission_menu = PermissionMenu::Closed;
                        self.model_menu = ModelMenu::Closed;
                        self.context_popover = ContextPopover::Closed;
                        self.close_slash();
                        AttachPopup::Open
                    }
                    AttachPopup::Open => AttachPopup::Closed,
                };
            }
            Message::AttachDismissed => {
                self.attach_popup = AttachPopup::Closed;
            }
            Message::PermissionPressed => {
                self.permission_menu = match self.permission_menu {
                    PermissionMenu::Closed => {
                        self.model_menu = ModelMenu::Closed;
                        self.context_popover = ContextPopover::Closed;
                        self.attach_popup = AttachPopup::Closed;
                        self.close_slash();
                        PermissionMenu::Open
                    }
                    PermissionMenu::Open => PermissionMenu::Closed,
                };
            }
            Message::PermissionSelected(posture) => {
                self.permission = PermissionMode::Muse(posture);
                self.permission_menu = PermissionMenu::Closed;
            }
            Message::PermissionDismissed => {
                self.permission_menu = PermissionMenu::Closed;
            }
            Message::ModelPressed => {
                self.model_menu = match self.model_menu {
                    ModelMenu::Closed => {
                        self.permission_menu = PermissionMenu::Closed;
                        self.context_popover = ContextPopover::Closed;
                        self.attach_popup = AttachPopup::Closed;
                        self.close_slash();
                        ModelMenu::Open
                    }
                    ModelMenu::Open | ModelMenu::EffortOpen => ModelMenu::Closed,
                };
            }
            Message::ModelSelected(choice) => {
                self.model = choice;
                if self.model_menu == ModelMenu::EffortOpen {
                    self.model_menu = ModelMenu::Open;
                }
                self.refresh_derived();
            }
            Message::ModelEditPressed(choice) => {
                if self.model_menu != ModelMenu::Closed {
                    self.model = choice;
                    self.model_menu = ModelMenu::EffortOpen;
                    self.refresh_derived();
                }
            }
            Message::EffortSelected(effort) => {
                self.effort = effort;
                self.model_menu = ModelMenu::Closed;
                self.refresh_derived();
            }
            Message::ModelDismissed => {
                self.model_menu = ModelMenu::Closed;
            }
            Message::ContextPressed => {
                self.context_popover = match self.context_popover {
                    ContextPopover::Closed => {
                        self.permission_menu = PermissionMenu::Closed;
                        self.model_menu = ModelMenu::Closed;
                        self.attach_popup = AttachPopup::Closed;
                        self.close_slash();
                        ContextPopover::Open
                    }
                    ContextPopover::Open => ContextPopover::Closed,
                };
            }
            Message::ContextDismissed => {
                self.context_popover = ContextPopover::Closed;
            }
            Message::SlashNext => {
                if let SlashMenu::Open { selected } = &mut self.slash_menu
                    && !self.slash_rows.is_empty()
                {
                    *selected = (*selected + 1) % self.slash_rows.len();
                }
            }
            Message::SlashPrev => {
                if let SlashMenu::Open { selected } = &mut self.slash_menu
                    && !self.slash_rows.is_empty()
                {
                    let len = self.slash_rows.len();
                    *selected = (*selected + len - 1) % len;
                }
            }
            Message::SlashAccept => {
                if let SlashMenu::Open { selected } = &self.slash_menu
                    && let Some(row) = self.slash_rows.get(*selected)
                {
                    let pick = row.pick.clone();
                    self.accept_slash_pick(&pick);
                }
            }
            Message::SlashSelected(pick) => {
                if self.slash_rows.iter().any(|row| row.pick == pick) {
                    self.accept_slash_pick(&pick);
                }
            }
            Message::SlashDismiss => {
                if matches!(self.slash_menu, SlashMenu::Open { .. }) {
                    let text = self.content.text();
                    self.slash_menu = SlashMenu::Dismissed { text };
                    self.slash_rows.clear();
                }
            }
            Message::SendPressed
            | Message::StopPressed
            | Message::AttachFilesPressed
            | Message::AttachPicked(_) => {}
        }
    }

    /// Accepts one slash row: the draft becomes `/selector ` with the
    /// caret after it, and the menu closes. Accepting the model
    /// builtin also opens the model menu, matching what sending
    /// `/model` does. The replacement folds through the editor as an
    /// edit (select all, paste the `/selector ` text): the edit
    /// notifies the editor's highlighter, so the token styles on the
    /// next draw with no extra keystroke — even when the draft was
    /// already a valid token and validity alone would not flip. The
    /// trailing move lands the caret after the inserted text (and
    /// clears the select-all range) instead of leaving it at the
    /// buffer start, which a fresh value would do.
    fn accept_slash_pick(&mut self, pick: &SlashPick) {
        let selector = match pick {
            SlashPick::Builtin(builtin) => builtin.selector(),
            SlashPick::Skill(selector) => selector.as_str(),
        };
        self.content.perform(text_editor::Action::SelectAll);
        self.content
            .perform(text_editor::Action::Edit(text_editor::Edit::Paste(
                std::sync::Arc::new(format!("/{selector} ")),
            )));
        self.content
            .perform(text_editor::Action::Move(text_editor::Motion::DocumentEnd));
        self.has_text = true;
        self.visible_lines = self.content.line_count().clamp(1, COMPOSER_MAX_LINES);
        self.close_slash();
        if matches!(pick, SlashPick::Builtin(BuiltinCommand::Model)) {
            self.permission_menu = PermissionMenu::Closed;
            self.context_popover = ContextPopover::Closed;
            self.attach_popup = AttachPopup::Closed;
            self.model_menu = ModelMenu::Open;
        }
    }

    /// Closes the slash menu without dismissing it for the draft: the
    /// next edit re-arms it while the draft still triggers. Only one
    /// menu shows at a time, so the other menu toggles call this when
    /// they open.
    fn close_slash(&mut self) {
        self.slash_menu = SlashMenu::Closed;
        self.slash_rows.clear();
    }

    /// Recomputes the slash menu from the draft after every edit: a
    /// draft whose unterminated first token starts with `/` opens
    /// (or refilters) the menu, anything else closes it. A dismissed
    /// menu stays shut for its exact dismissal text; any edit
    /// re-arms it. The visible rows are precomputed here so the view
    /// reads without filtering (UI contract §2); the selection
    /// clamps into the new rows.
    fn refresh_slash_menu(&mut self) {
        let text = self.content.text();
        let Some(query) = slash_query(&text) else {
            self.close_slash();
            return;
        };
        if let SlashMenu::Dismissed { text: dismissed } = &self.slash_menu
            && dismissed == &text
        {
            self.slash_rows.clear();
            return;
        }
        let rows = filter_slash_rows(&query, &self.skill_catalog);
        let selected = match self.slash_menu {
            SlashMenu::Open { selected } => selected.min(rows.len().saturating_sub(1)),
            SlashMenu::Closed | SlashMenu::Dismissed { .. } => 0,
        };
        self.slash_rows = rows;
        self.slash_menu = SlashMenu::Open { selected };
    }

    /// Inserts a picked path ref into the draft and closes the popup.
    /// Called by the App, which owns the OS picker.
    pub fn insert_attachment(&mut self, path: String) {
        self.attachments.push(path);
        self.attach_popup = AttachPopup::Closed;
    }

    /// The prompt to send: the draft text with each attached path ref
    /// appended as a clearly-marked ref line. Without attachments this
    /// is the draft text unchanged.
    pub fn composed_prompt(&self) -> String {
        let text = self.content.text();
        if self.attachments.is_empty() {
            return text;
        }
        let mut composed = text;
        for path in &self.attachments {
            composed.push('\n');
            composed.push_str("[ref: ");
            composed.push_str(path);
            composed.push(']');
        }
        composed
    }

    /// Replaces the catalog and recomputes the model pill, so the view
    /// reads without lookup (UI contract §2).
    pub fn set_catalog(&mut self, catalog: ModelCatalog) {
        self.catalog = catalog;
        self.refresh_derived();
    }

    /// Replaces the skill catalog, refreshes the highlight selectors
    /// and the visible slash rows, so the view reads without lookup
    /// (UI contract §2).
    pub fn set_skill_catalog(&mut self, catalog: SkillCatalog) {
        self.skill_catalog = catalog;
        let mut selectors = builtin_selectors();
        if let SkillCatalog::Ready(skills) = &self.skill_catalog {
            selectors.extend(skills.iter().map(|skill| skill.selector.clone()));
        }
        self.highlight_selectors = selectors;
        self.refresh_slash_menu();
    }

    /// Replaces the usage snapshot and recomputes the meter strings,
    /// so the view reads without formatting (UI contract §2).
    pub fn set_usage(&mut self, usage: Option<UsageSnapshot>) {
        self.usage = usage;
        self.refresh_derived();
    }

    /// Renders the input bar with the inside-bar buttons, the
    /// attachment refs, and the live sub-bar. The attach, model,
    /// permission, and context popups are not part of this flow; they
    /// float via [`State::attach_menu_overlay`],
    /// [`State::model_menu_overlay`], [`State::permission_menu_overlay`],
    /// and [`State::context_popover_overlay`].
    pub fn view(&self) -> Element<'_, Message> {
        // While the slash menu is open, arrows/Tab/Enter/Esc fold into
        // menu messages through the editor's key binding, before the
        // editor can consume them (arrows must not move the caret, Tab
        // must not leave the input, Enter must accept). Every other
        // key falls back to the editor defaults inside the closure.
        let slash_active = matches!(self.slash_menu, SlashMenu::Open { .. });
        let has_rows = slash_active && !self.slash_rows.is_empty();
        let keys: Option<kit::ComposerKeys<'_, Message>> = slash_active.then(|| {
            Box::new(move |press: text_editor::KeyPress| slash_key_binding(press, has_rows))
                as kit::ComposerKeys<'_, Message>
        });
        // The highlight settings carry the draft's live token
        // validity, so the editor re-arms exactly when the first token
        // becomes (or stops being) an exact selector — typed, pasted,
        // or accepted — with no extra keystroke.
        let highlight =
            SlashHighlightSettings::for_draft(&self.highlight_selectors, &self.content.text());
        let mut row = iced::widget::row![
            kit::composer_icon_slot(kit::icon_button(
                lucide_icons::Icon::Plus,
                IconButtonKind::Subtle,
                Some(Message::AttachPressed),
            )),
            iced::widget::container(kit::composer_input(
                &self.content,
                self.visible_lines,
                Message::Edit,
                &highlight,
                keys,
            ))
            .width(Length::Fill),
        ]
        .spacing(SPACE_XS)
        .align_y(iced::alignment::Vertical::Bottom);
        if self.running_visible {
            if self.has_text {
                row = row.push(kit::composer_icon_slot(kit::icon_button(
                    lucide_icons::Icon::ArrowUp,
                    IconButtonKind::Accent,
                    self.send_enabled.then_some(Message::SendPressed),
                )));
            }
            row = row.push(kit::composer_icon_slot(kit::icon_button(
                lucide_icons::Icon::Square,
                IconButtonKind::Subtle,
                Some(Message::StopPressed),
            )));
        } else if self.has_text {
            row = row.push(kit::composer_icon_slot(kit::icon_button(
                lucide_icons::Icon::ArrowUp,
                IconButtonKind::Accent,
                self.send_enabled.then_some(Message::SendPressed),
            )));
        }
        let mut column = iced::widget::column![kit::input_bar(row)].spacing(SPACE_XS);
        if let Some(hint) = &self.pending_hint {
            column = column.push(
                iced::widget::text(hint.as_str())
                    .size(FONT_SIZE_PANEL)
                    .color(TEXT_TERTIARY),
            );
        }
        if !self.attachments.is_empty() {
            column = column.push(self.attachments_view());
        }
        column = column.push(self.sub_bar_view());
        column.width(Length::Fill).into()
    }

    /// The floating model menu for the App to overlay above the
    /// composer, or `None` when the menu is closed. The menu never
    /// sits in the composer's layout flow, so opening it cannot
    /// change the composer's height or push the transcript around;
    /// the App anchors this row at the bottom-right of the window,
    /// just above the sub-bar.
    pub(crate) fn model_menu_overlay(&self) -> Option<Element<'_, Message>> {
        if matches!(self.model_menu, ModelMenu::Closed) {
            None
        } else {
            Some(self.model_menu_view())
        }
    }

    /// The floating permission menu for the App to overlay above the
    /// composer, or `None` when the menu is closed. The menu never
    /// sits in the composer's layout flow, so opening it cannot
    /// change the composer's height or push the transcript around;
    /// the App anchors this card at the bottom-left of the window,
    /// just above the sub-bar.
    pub(crate) fn permission_menu_overlay(&self) -> Option<Element<'_, Message>> {
        if matches!(self.permission_menu, PermissionMenu::Closed) {
            None
        } else {
            Some(self.permission_menu_view())
        }
    }

    /// The floating attach menu for the App to overlay above the
    /// composer, or `None` when the menu is closed. The menu never
    /// sits in the composer's layout flow, so opening it cannot
    /// change the composer's height or push the transcript around;
    /// the App anchors this card at the bottom-left of the window,
    /// just above the input bar.
    pub(crate) fn attach_menu_overlay(&self) -> Option<Element<'_, Message>> {
        if matches!(self.attach_popup, AttachPopup::Closed) {
            None
        } else {
            Some(self.attach_menu_view())
        }
    }

    /// The floating context popup for the App to overlay above the
    /// composer, or `None` when the popover is closed. The popup never
    /// sits in the composer's layout flow, so opening it cannot
    /// change the composer's height or push the transcript around;
    /// the App anchors this card at the bottom-right of the window,
    /// just above the sub-bar.
    pub(crate) fn context_popover_overlay(&self) -> Option<Element<'_, Message>> {
        if matches!(self.context_popover, ContextPopover::Closed) {
            None
        } else {
            Some(self.context_popover_view())
        }
    }

    /// The floating slash menu for the App to overlay above the
    /// composer, or `None` when the menu is closed or dismissed. The
    /// menu never sits in the composer's layout flow, so opening it
    /// cannot change the composer's height or push the transcript
    /// around; the App anchors this card at the main column's left
    /// edge, just above the input bar.
    pub(crate) fn slash_menu_overlay(&self) -> Option<Element<'_, Message>> {
        if matches!(self.slash_menu, SlashMenu::Open { .. }) {
            Some(self.slash_menu_view())
        } else {
            None
        }
    }

    /// The attach menu: a Files row that opens the OS picker, and a
    /// disabled Photos row with a plain note. Photos stay disabled
    /// because the bundled engine reports no image probe; the note
    /// says so rather than inventing one.
    fn attach_menu_view(&self) -> Element<'_, Message> {
        let files = kit::action_row(
            "Files",
            Some(lucide_icons::Icon::Paperclip),
            Message::AttachFilesPressed,
        );
        let photos = iced::widget::container(
            iced::widget::column![
                iced::widget::row![
                    kit::icon(lucide_icons::Icon::Image, FONT_SIZE_PANEL, TEXT_FAINT),
                    iced::widget::text("Photos")
                        .size(FONT_SIZE_PANEL)
                        .color(TEXT_TERTIARY),
                ]
                .spacing(SPACE_XS)
                .align_y(iced::alignment::Vertical::Center),
                iced::widget::text("Photos aren't available with this engine yet.")
                    .size(crate::theme::FONT_SIZE_SMALL)
                    .color(TEXT_TERTIARY),
            ]
            .spacing(SPACE_XS)
            .width(Length::Fill),
        )
        .padding([SPACE_XS, SPACE_S])
        .width(Length::Fill);
        kit::popup_card(
            iced::widget::column![files, photos]
                .spacing(SPACE_XS)
                .width(Length::Fill),
            ATTACH_POPUP_WIDTH,
        )
    }

    /// The attached path refs, one fixture pill per path.
    fn attachments_view(&self) -> Element<'_, Message> {
        let mut refs = iced::widget::row![].spacing(SPACE_XS);
        for path in &self.attachments {
            refs = refs.push(kit::static_pill(path.as_str()));
        }
        refs.width(Length::Fill).into()
    }

    /// The live sub-bar: the permission text-button on the left, and
    /// the combined model-effort pill plus the context meter pinned
    /// to the right.
    fn sub_bar_view(&self) -> Element<'_, Message> {
        let permission_kind = if self.permission == PermissionMode::Muse(MusePermission::Yolo) {
            kit::ButtonKind::DangerGhost
        } else {
            kit::ButtonKind::Ghost
        };
        iced::widget::row![
            kit::pill_button(
                self.permission.label(),
                permission_kind,
                Some(Message::PermissionPressed),
            ),
            iced::widget::container(iced::widget::text("")).width(Length::Fill),
            kit::pill_button(
                self.model_effort_label.as_str(),
                kit::ButtonKind::Ghost,
                Some(Message::ModelPressed),
            ),
            kit::pill_button(
                self.context_label.as_str(),
                kit::ButtonKind::Ghost,
                Some(Message::ContextPressed),
            ),
        ]
        .spacing(SPACE_S)
        .width(Length::Fill)
        .align_y(iced::alignment::Vertical::Center)
        .into()
    }

    /// The permission menu: every posture with its one-line account.
    /// The selected posture reads Primary; the rest read Ghost. Yolo
    /// is the exception: selected reads Danger and unselected reads
    /// DangerGhost, so its label stays in the palette error red.
    fn permission_menu_view(&self) -> Element<'_, Message> {
        let mut menu = iced::widget::column![].spacing(SPACE_XS);
        for posture in MusePermission::all() {
            let selected = self.permission == PermissionMode::Muse(posture);
            let kind = match (posture, selected) {
                (MusePermission::Yolo, true) => kit::ButtonKind::Danger,
                (MusePermission::Yolo, false) => kit::ButtonKind::DangerGhost,
                (_, true) => kit::ButtonKind::Primary,
                (_, false) => kit::ButtonKind::Ghost,
            };
            let entry = iced::widget::row![
                kit::pill_button(
                    posture.label(),
                    kind,
                    Some(Message::PermissionSelected(posture))
                ),
                iced::widget::text(posture.description())
                    .size(crate::theme::FONT_SIZE_SMALL)
                    .color(crate::theme::TEXT_TERTIARY),
            ]
            .spacing(SPACE_S)
            .align_y(iced::alignment::Vertical::Center);
            menu = menu.push(entry);
        }
        kit::popup_card(menu.width(Length::Fill), PERMISSION_POPUP_WIDTH)
    }

    /// The model menu: a narrow card right-aligned above the sub-bar,
    /// with one row per model choice, and — when a row's Edit action
    /// has opened it — the effort card to its left.
    fn model_menu_view(&self) -> Element<'_, Message> {
        let mut rows = iced::widget::column![kit::menu_row(
            "Default",
            self.model == ModelChoice::Default,
            Message::ModelSelected(ModelChoice::Default),
            Message::ModelEditPressed(ModelChoice::Default),
        )]
        .spacing(SPACE_XS)
        .width(Length::Fill);
        match &self.catalog {
            ModelCatalog::Loading => {
                rows = rows.push(kit::popup_note("Loading models\u{2026}"));
            }
            ModelCatalog::Failed(_) => {
                rows = rows.push(kit::popup_note("Models unavailable"));
            }
            ModelCatalog::Ready(models) => {
                let mut list = iced::widget::column![]
                    .spacing(SPACE_XS)
                    .width(Length::Fill);
                for model in models {
                    let choice = ModelChoice::Named(model.id.clone());
                    list = list.push(kit::menu_row(
                        model.label(),
                        self.model == choice,
                        Message::ModelSelected(choice.clone()),
                        Message::ModelEditPressed(choice),
                    ));
                }
                rows = rows.push(
                    iced::widget::scrollable(list)
                        .height(Length::Fixed(popup_list_height(models.len())))
                        .width(Length::Fill),
                );
            }
        }
        let model_card = kit::popup_card(rows, MODEL_POPUP_WIDTH);
        let mut bar =
            iced::widget::row![iced::widget::container(iced::widget::text("")).width(Length::Fill)]
                .spacing(SPACE_S)
                .width(Length::Fill);
        if self.model_menu == ModelMenu::EffortOpen {
            bar = bar.push(self.effort_menu_view());
        }
        bar = bar.push(model_card);
        bar.into()
    }

    /// The effort side-panel: an "Effort" header over every reasoning
    /// level, with a check on the current one.
    fn effort_menu_view(&self) -> Element<'_, Message> {
        let mut rows = iced::widget::column![kit::popup_header("Effort")]
            .spacing(SPACE_XS)
            .width(Length::Fill);
        for effort in EffortChoice::all() {
            rows = rows.push(kit::choice_row(
                effort_label(effort),
                self.effort == effort,
                Message::EffortSelected(effort),
            ));
        }
        kit::popup_card(rows, EFFORT_POPUP_WIDTH)
    }

    /// The context popover: the meter plus the one-line breakdown, or
    /// the no-report fallback.
    fn context_popover_view(&self) -> Element<'_, Message> {
        let mut popover = iced::widget::column![].spacing(SPACE_XS);
        if let Some(percent) = self.context_percent {
            popover = popover.push(iced::widget::progress_bar(0.0..=100.0, percent));
        }
        let detail = self
            .usage
            .as_ref()
            .map(|snapshot| snapshot.detail.as_str())
            .unwrap_or("No usage reported yet.");
        popover = popover.push(
            iced::widget::text(detail)
                .size(crate::theme::FONT_SIZE_SMALL)
                .color(crate::theme::TEXT_SECONDARY),
        );
        kit::popup_card(popover.width(Length::Fill), CONTEXT_POPUP_WIDTH)
    }

    /// The slash menu: one row per precomputed visible row, newest
    /// selection highlighted. Catalog states without rows read as
    /// notes, never as a blank card: loading while the request is in
    /// flight, the failure message on error, an empty state for an
    /// empty catalog, and a no-match note for a filtered-out query.
    fn slash_menu_view(&self) -> Element<'_, Message> {
        let selected = match self.slash_menu {
            SlashMenu::Open { selected } => selected,
            SlashMenu::Closed | SlashMenu::Dismissed { .. } => 0,
        };
        let mut menu = iced::widget::column![].spacing(SPACE_XS);
        if self.slash_rows.is_empty() {
            let note = match &self.skill_catalog {
                SkillCatalog::Loading => "Loading skills…",
                SkillCatalog::Failed(message) => message.as_str(),
                SkillCatalog::Ready(_) => {
                    // A bare `/` (the empty query) means the catalog
                    // itself is empty; any other query filtered every
                    // row out. A `None` query never renders here: the
                    // menu is closed exactly when the query is `None`.
                    if slash_query(&self.content.text()).is_some_and(|query| query.is_empty()) {
                        "No skills available"
                    } else {
                        "No matching commands"
                    }
                }
            };
            menu = menu.push(kit::popup_note(note));
        } else {
            let mut list = iced::widget::column![]
                .spacing(SPACE_XS)
                .width(Length::Fill);
            for (index, row) in self.slash_rows.iter().enumerate() {
                list = list.push(kit::slash_row(
                    slash_source_icon(&row.source),
                    row.display.as_str(),
                    row.description.as_str(),
                    index == selected,
                    Message::SlashSelected(row.pick.clone()),
                ));
            }
            menu = menu.push(
                iced::widget::scrollable(list)
                    .height(Length::Fixed(popup_list_height(self.slash_rows.len())))
                    .width(Length::Fill),
            );
        }
        kit::popup_card(menu.width(Length::Fill), SLASH_POPUP_WIDTH)
    }
}

impl Default for State {
    fn default() -> Self {
        Self::new()
    }
}

/// Height of a scrollable popup list for `row_count` rows: fits the
/// rows (and the spacing between them) up to
/// [`MODEL_LIST_MAX_HEIGHT`], and is at least one row tall so an
/// empty list still occupies a single row. Shared by the model card
/// and the slash menu, whose rows share [`MENU_ROW_HEIGHT`].
fn popup_list_height(row_count: usize) -> f32 {
    let rows = row_count.max(1);
    let spacing = (rows - 1) as f32 * SPACE_XS;
    (rows as f32 * MENU_ROW_HEIGHT + spacing).min(MODEL_LIST_MAX_HEIGHT)
}

/// Selectors highlighted as a leading token before any catalog
/// arrives: the hard-coded builtins.
fn builtin_selectors() -> Vec<String> {
    BuiltinCommand::all()
        .iter()
        .map(|builtin| builtin.selector().to_string())
        .collect()
}

/// The slash query for one draft: the text after the leading `/` of
/// the first token, or `None` when the first token does not start
/// with `/` or is already terminated by whitespace (the user has
/// moved past the command word into arguments). An empty query (a
/// bare `/`) matches every row.
fn slash_query(text: &str) -> Option<String> {
    // Leading whitespace is skipped by finding the first token
    // start; the token must then run to the end of the draft. Any
    // trailing text starts with whitespace (the token is the maximal
    // non-whitespace run), so a length mismatch means the command
    // word is terminated and the menu stays shut.
    let rest = &text[text.find(|next: char| !next.is_whitespace())?..];
    let token = rest.split_whitespace().next()?;
    if rest.len() != token.len() {
        return None;
    }
    token.strip_prefix('/').map(str::to_string)
}

/// Whether one row matches the query: a case-insensitive substring
/// hit on the selector, the display name, or the description. An
/// empty query matches everything.
fn slash_matches(query: &str, selector: &str, name: &str, description: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    let query = query.to_lowercase();
    selector.to_lowercase().contains(&query)
        || name.to_lowercase().contains(&query)
        || description.to_lowercase().contains(&query)
}

/// One builtin slash row, precomputed for the menu.
fn builtin_row(builtin: BuiltinCommand) -> SlashRow {
    SlashRow {
        pick: SlashPick::Builtin(builtin),
        selector: builtin.selector().to_string(),
        display: format!("/{}", builtin.selector()),
        name: builtin.name().to_string(),
        description: builtin.description().to_string(),
        source: String::from("builtin"),
    }
}

/// One catalog skill slash row, precomputed for the menu.
fn skill_row(skill: &SkillInfo) -> SlashRow {
    SlashRow {
        pick: SlashPick::Skill(skill.selector.clone()),
        selector: skill.selector.clone(),
        display: format!("/{}", skill.selector),
        name: skill.label().to_string(),
        description: skill.description.clone(),
        source: skill.source.clone(),
    }
}

/// Visible slash rows for one query: builtins first, then the ready
/// catalog's matching skills. Loading, failed, and empty catalogs
/// contribute no skill rows; the view renders those states as notes.
fn filter_slash_rows(query: &str, catalog: &SkillCatalog) -> Vec<SlashRow> {
    let mut rows: Vec<SlashRow> = BuiltinCommand::all()
        .iter()
        .map(|builtin| builtin_row(*builtin))
        .filter(|row| slash_matches(query, &row.selector, &row.name, &row.description))
        .collect();
    if let SkillCatalog::Ready(skills) = catalog {
        rows.extend(
            skills
                .iter()
                .map(skill_row)
                .filter(|row| slash_matches(query, &row.selector, &row.name, &row.description)),
        );
    }
    rows
}

/// The Lucide source icon for one slash row: builtins read as a
/// terminal, catalog scopes as package (bundled), user, folder
/// (project), or puzzle (plugin). Unknown scopes read as sparkles,
/// never as a missing icon.
fn slash_source_icon(source: &str) -> lucide_icons::Icon {
    match source {
        "builtin" => lucide_icons::Icon::SquareTerminal,
        "bundled" => lucide_icons::Icon::Package,
        "user" => lucide_icons::Icon::User,
        "project" => lucide_icons::Icon::Folder,
        "plugin" => lucide_icons::Icon::Puzzle,
        _ => lucide_icons::Icon::Sparkles,
    }
}

/// Byte range of the draft's highlightable first token: the leading
/// `/selector` token when its selector is known, `None` otherwise.
/// The range covers the first token only, and only on an exact
/// selector match; the arguments region never highlights.
pub(crate) fn slash_token_range(line: &str, selectors: &[String]) -> Option<Range<usize>> {
    let start = line.find(|next: char| !next.is_whitespace())?;
    let token = line[start..].split_whitespace().next()?;
    let selector = token.strip_prefix('/')?;
    if selector.is_empty() {
        return None;
    }
    selectors
        .iter()
        .any(|known| known == selector)
        .then(|| start..start + token.len())
}

/// The slash-menu key binding: while the menu has rows, arrows move
/// the selection, Enter and Tab accept it, and Esc dismisses the
/// menu, all before the editor consumes them. Esc also dismisses a
/// row-less menu. Every other key (including modified Enter, so
/// Ctrl+Enter still reaches the pending shortcut) falls back to the
/// editor defaults.
fn slash_key_binding(
    press: text_editor::KeyPress,
    has_rows: bool,
) -> Option<text_editor::Binding<Message>> {
    use iced::keyboard::{Key, Modifiers, key::Named};
    if press.modifiers != Modifiers::empty() {
        return text_editor::Binding::from_key_press(press);
    }
    match &press.key {
        Key::Named(Named::Escape) => Some(text_editor::Binding::Custom(Message::SlashDismiss)),
        Key::Named(Named::ArrowUp) if has_rows => {
            Some(text_editor::Binding::Custom(Message::SlashPrev))
        }
        Key::Named(Named::ArrowDown) if has_rows => {
            Some(text_editor::Binding::Custom(Message::SlashNext))
        }
        Key::Named(Named::Enter) if has_rows => {
            Some(text_editor::Binding::Custom(Message::SlashAccept))
        }
        Key::Named(Named::Tab) if has_rows => {
            Some(text_editor::Binding::Custom(Message::SlashAccept))
        }
        _ => text_editor::Binding::from_key_press(press),
    }
}

/// The editor style for a matched slash token: the theme's strong
/// primary, regular weight, so the token reads distinct from body
/// text under any theme the app runs. Plain data (no editor highlight
/// types: the `iced` facade hides the highlight `Format` path, so
/// `kit` fills it by inference from this pair).
///
/// Weight is deliberately regular: a mid-line font change desyncs the
/// caret x from the shaped text in this Iced line (0.14), rendering
/// the caret a character-width off in opposite directions as styling
/// toggles. Color never affects advances, shaping, or caret geometry
/// on any backend, so color-only styling removes the desync by
/// construction. See `SlashHighlighter` for the protocol notes.
pub(crate) fn slash_token_style(theme: &iced::Theme) -> (iced::Color, iced::Font) {
    (
        theme.extended_palette().primary.strong.color,
        iced::Font::DEFAULT,
    )
}

/// The editor highlight value for a matched slash token: a unit
/// marker, since the token style is fixed in [`slash_token_style`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SlashHighlight;

/// Settings for [`SlashHighlighter`]: the known selectors plus
/// whether the draft's first line currently opens with an exact
/// `/selector` token.
///
/// The token bit exists so the editor re-arms the highlighter through
/// its own settings path exactly when validity flips — typed, pasted,
/// or accepted — instead of relying solely on per-edit `change_line`
/// notifications. The widget compares settings by value on every
/// layout (`iced_widget` 0.14.2 `text_editor.rs`): a flip reaches
/// `Highlighter::update`, which resets to line 0 so the next draw
/// feeds the first line again. Selector catalog changes re-arm the
/// same way, as before.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SlashHighlightSettings {
    /// Known selectors: builtins plus the ready catalog's.
    selectors: Vec<String>,
    /// Whether the draft's first line opens with an exact token.
    token: bool,
}

impl SlashHighlightSettings {
    /// Settings for one draft snapshot: the selectors plus the live
    /// validity of the draft's first line. The view derives this per
    /// rebuild, so any edit that flips validity changes the settings
    /// value the widget compares.
    pub(crate) fn for_draft(selectors: &[String], draft: &str) -> Self {
        let first = draft.lines().next().unwrap_or("");
        let token = slash_token_range(first, selectors).is_some();
        Self {
            selectors: selectors.to_vec(),
            token,
        }
    }

    /// Whether the draft's first line opens with an exact token.
    /// Test-only observability for the validity bit the widget
    /// compares opaquely; production reads it through `PartialEq`.
    #[cfg(test)]
    pub(crate) fn token_valid(&self) -> bool {
        self.token
    }
}

/// Highlights the draft's leading `/selector` token.
///
/// The editor feeds lines in order from the last `change_line`
/// notification (`iced_graphics` 0.14.0 `text/editor.rs`
/// `update`/`highlight`: every edit reports its topmost changed line,
/// and each draw feeds `highlight_line` for lines
/// `current_line..=last_visible` unless the highlighter is already
/// past them), so this tracks its position: only position 0 (the
/// draft's first line) can highlight, and each fed line advances the
/// position by one. Selector and token-validity changes arrive
/// through `update`, which re-arms from line 0 so the next draw
/// re-highlights.
pub(crate) struct SlashHighlighter {
    /// Known selectors: builtins plus the ready catalog's.
    selectors: Vec<String>,
    /// Line index of the next fed line.
    line: usize,
}

impl iced::widget::text::Highlighter for SlashHighlighter {
    type Settings = SlashHighlightSettings;
    type Highlight = SlashHighlight;
    type Iterator<'a> = std::option::IntoIter<(Range<usize>, SlashHighlight)>;

    fn new(settings: &SlashHighlightSettings) -> Self {
        Self {
            selectors: settings.selectors.clone(),
            line: 0,
        }
    }

    fn update(&mut self, new_settings: &SlashHighlightSettings) {
        self.selectors = new_settings.selectors.clone();
        self.line = 0;
    }

    fn change_line(&mut self, line: usize) {
        self.line = line;
    }

    fn highlight_line(&mut self, line: &str) -> Self::Iterator<'_> {
        let at_first = self.line == 0;
        self.line = self.line.saturating_add(1);
        if at_first {
            slash_token_range(line, &self.selectors)
                .map(|range| (range, SlashHighlight))
                .into_iter()
        } else {
            None.into_iter()
        }
    }

    fn current_line(&self) -> usize {
        self.line
    }
}

/// Short human name of an effort choice, for the effort submenu.
/// Pure data the update stores and the view reads; kept beside the
/// composer because the domain choice carries wire values only.
fn effort_label(effort: EffortChoice) -> &'static str {
    match effort {
        EffortChoice::Default => "Default",
        EffortChoice::None => "None",
        EffortChoice::Minimal => "Minimal",
        EffortChoice::Low => "Low",
        EffortChoice::Medium => "Medium",
        EffortChoice::High => "High",
        EffortChoice::XHigh => "XHigh",
        EffortChoice::Max => "Max",
        EffortChoice::Ultra => "Ultra",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Types text through edit actions, mapping newlines to Enter.
    fn type_text(state: &mut State, text: &str) {
        for next in text.chars() {
            let edit = if next == '\n' {
                text_editor::Edit::Enter
            } else {
                text_editor::Edit::Insert(next)
            };
            state.update(Message::Edit(text_editor::Action::Edit(edit)));
        }
    }

    #[test]
    fn default_matches_new() {
        let got = State::default();
        let want = State::new();
        assert_eq!(got.visible_lines, 1);
        assert_eq!(got.visible_lines, want.visible_lines);
        assert_eq!(got.has_text, want.has_text);
        assert!(!got.has_text);
        assert_eq!(got.send_enabled, want.send_enabled);
        assert!(got.send_enabled);
        assert_eq!(got.content.text(), want.content.text());
        assert_eq!(got.content.text(), "");
    }

    #[test]
    fn new_starts_with_menus_closed_and_default_picks() {
        let got = State::new();
        assert_eq!(got.attach_popup, AttachPopup::Closed);
        assert_eq!(got.permission_menu, PermissionMenu::Closed);
        assert_eq!(got.model_menu, ModelMenu::Closed);
        assert_eq!(got.context_popover, ContextPopover::Closed);
        assert_eq!(got.permission, PermissionMode::default());
        assert_eq!(got.model, ModelChoice::Default);
        assert_eq!(got.effort, EffortChoice::Default);
        assert_eq!(got.catalog, ModelCatalog::Loading);
        assert!(got.attachments.is_empty());
        assert_eq!(got.usage, None);
    }

    #[test]
    fn clear_resets_draft_but_preserves_send_enabled() {
        let mut got = State::new();
        type_text(&mut got, "draft\ntext");
        got.send_enabled = false;
        got.running_visible = true;
        got.update(Message::PermissionSelected(MusePermission::Judge));
        got.clear();
        assert_eq!(got.content.text(), "");
        assert!(!got.has_text);
        assert_eq!(got.visible_lines, 1);
        assert!(!got.send_enabled);
        assert!(got.running_visible);
        // Picker choices survive a send; only the draft resets.
        assert_eq!(got.permission, PermissionMode::Muse(MusePermission::Judge));
        assert!(got.attachments.is_empty());
    }

    #[test]
    fn clear_preserves_picker_state_and_derived_labels() {
        let mut got = catalog_state();
        got.update(Message::ModelSelected(ModelChoice::Named(String::from(
            "muse-spark",
        ))));
        got.update(Message::EffortSelected(EffortChoice::High));
        got.update(Message::PermissionSelected(MusePermission::Judge));
        got.set_usage(UsageSnapshot::parse(
            r#"{"prompt":57064,"output":16,"total":57080,"limit":200000}"#,
        ));
        let label_before = got.model_effort_label.clone();
        let context_before = got.context_label.clone();
        let percent_before = got.context_percent;
        type_text(&mut got, "draft text");
        got.insert_attachment(String::from("/work/notes.md"));
        // Reopen a menu so clear() must close it.
        got.update(Message::ModelPressed);
        assert_eq!(got.model_menu, ModelMenu::Open);
        got.send_enabled = false;
        got.running_visible = true;
        got.clear();
        assert_eq!(got.content.text(), "");
        assert!(!got.has_text);
        assert_eq!(got.visible_lines, 1);
        assert!(got.attachments.is_empty());
        assert_eq!(got.attach_popup, AttachPopup::Closed);
        assert_eq!(got.permission_menu, PermissionMenu::Closed);
        assert_eq!(got.model_menu, ModelMenu::Closed);
        assert_eq!(got.context_popover, ContextPopover::Closed);
        assert_eq!(got.model, ModelChoice::Named(String::from("muse-spark")));
        assert_eq!(got.effort, EffortChoice::High);
        assert_eq!(got.permission, PermissionMode::Muse(MusePermission::Judge));
        assert!(matches!(got.catalog, ModelCatalog::Ready(_)));
        assert!(got.usage.is_some());
        assert_eq!(got.model_effort_label, label_before);
        assert_eq!(got.context_label, context_before);
        assert_eq!(got.context_percent, percent_before);
        assert!(!got.send_enabled);
        assert!(got.running_visible);
    }

    #[test]
    fn typing_inserts_text_and_shows_send() {
        let mut got = State::new();
        assert!(!got.has_text);
        type_text(&mut got, "hi");
        assert_eq!(got.content.text(), "hi");
        assert!(got.has_text);
    }

    #[test]
    fn newlines_grow_visible_lines() {
        let mut got = State::new();
        assert_eq!(got.visible_lines, 1);
        type_text(&mut got, "a\nb");
        assert_eq!(got.visible_lines, 2);
        type_text(&mut got, "\nc");
        assert_eq!(got.visible_lines, 3);
    }

    #[test]
    fn visible_lines_clamp_at_max() {
        let mut got = State::new();
        type_text(&mut got, "\n\n\n\n\n\n");
        assert_eq!(got.visible_lines, COMPOSER_MAX_LINES);
    }

    #[test]
    fn deleting_all_text_resets_derived_state() {
        let mut got = State::new();
        type_text(&mut got, "hello\nworld");
        assert!(got.has_text);
        got.update(Message::Edit(text_editor::Action::SelectAll));
        got.update(Message::Edit(text_editor::Action::Edit(
            text_editor::Edit::Backspace,
        )));
        assert_eq!(got.content.text(), "");
        assert!(!got.has_text);
        assert_eq!(got.visible_lines, 1);
    }

    #[test]
    fn restore_sets_text_and_derived_state_preserving_send_enabled() {
        let mut got = State::new();
        got.send_enabled = false;
        got.restore(String::from("hello\nworld"));
        assert_eq!(got.content.text(), "hello\nworld");
        assert!(got.has_text);
        assert_eq!(got.visible_lines, 2);
        assert!(!got.send_enabled);
        got.send_enabled = true;
        got.restore(String::new());
        assert_eq!(got.content.text(), "");
        assert!(!got.has_text);
        assert_eq!(got.visible_lines, 1);
        assert!(got.send_enabled);
    }

    #[test]
    fn button_presses_leave_state_unchanged() {
        let mut got = State::new();
        type_text(&mut got, "draft\ntext");
        let text = got.content.text();
        let lines = got.visible_lines;
        let has_text = got.has_text;
        got.update(Message::SendPressed);
        got.update(Message::StopPressed);
        got.update(Message::AttachFilesPressed);
        got.update(Message::AttachPicked(Some(String::from("/x"))));
        assert_eq!(got.content.text(), text);
        assert_eq!(got.visible_lines, lines);
        assert_eq!(got.has_text, has_text);
    }

    #[test]
    fn attach_popup_toggles() {
        let mut got = State::new();
        got.update(Message::AttachPressed);
        assert_eq!(got.attach_popup, AttachPopup::Open);
        got.update(Message::AttachPressed);
        assert_eq!(got.attach_popup, AttachPopup::Closed);
    }

    #[test]
    fn attach_dismissed_closes_menu_and_is_harmless_when_closed() {
        let mut got = State::new();
        got.update(Message::AttachDismissed);
        assert_eq!(got.attach_popup, AttachPopup::Closed);
        got.update(Message::AttachPressed);
        assert_eq!(got.attach_popup, AttachPopup::Open);
        got.update(Message::AttachDismissed);
        assert_eq!(got.attach_popup, AttachPopup::Closed);
    }

    #[test]
    fn attach_menu_overlay_follows_popup_state() {
        let mut got = State::new();
        assert!(got.attach_menu_overlay().is_none());
        got.update(Message::AttachPressed);
        assert!(got.attach_menu_overlay().is_some());
        got.update(Message::AttachDismissed);
        assert!(got.attach_menu_overlay().is_none());
    }

    #[test]
    fn opening_attach_closes_permission_and_model_menus() {
        let mut got = State::new();
        got.update(Message::PermissionPressed);
        assert_eq!(got.permission_menu, PermissionMenu::Open);
        got.update(Message::AttachPressed);
        assert_eq!(got.attach_popup, AttachPopup::Open);
        assert_eq!(got.permission_menu, PermissionMenu::Closed);
        got.update(Message::ModelPressed);
        assert_eq!(got.model_menu, ModelMenu::Open);
        assert_eq!(got.attach_popup, AttachPopup::Closed);
        got.update(Message::AttachPressed);
        assert_eq!(got.attach_popup, AttachPopup::Open);
        assert_eq!(got.model_menu, ModelMenu::Closed);
    }

    #[test]
    fn insert_attachment_records_path_and_closes_popup() {
        let mut got = State::new();
        got.update(Message::AttachPressed);
        got.insert_attachment(String::from("/work/notes.md"));
        assert_eq!(got.attachments, vec![String::from("/work/notes.md")]);
        assert_eq!(got.attach_popup, AttachPopup::Closed);
    }

    #[test]
    fn composed_prompt_returns_text_unchanged_without_attachments() {
        let mut got = State::new();
        type_text(&mut got, "hello");
        assert_eq!(got.composed_prompt(), "hello");
    }

    #[test]
    fn composed_prompt_appends_ref_lines_in_pick_order() {
        let mut got = State::new();
        type_text(&mut got, "hello");
        got.insert_attachment(String::from("/work/a.md"));
        got.insert_attachment(String::from("/work/b.md"));
        assert_eq!(
            got.composed_prompt(),
            "hello\n[ref: /work/a.md]\n[ref: /work/b.md]"
        );
        // The draft text itself is untouched; only the send prompt carries
        // the refs.
        assert_eq!(got.content.text(), "hello");
    }

    #[test]
    fn permission_menu_selects_posture_and_closes() {
        let mut got = State::new();
        got.update(Message::PermissionPressed);
        assert_eq!(got.permission_menu, PermissionMenu::Open);
        got.update(Message::PermissionSelected(MusePermission::OnRequest));
        assert_eq!(
            got.permission,
            PermissionMode::Muse(MusePermission::OnRequest)
        );
        assert_eq!(got.permission_menu, PermissionMenu::Closed);
    }

    /// Builds a state with a two-row catalog ready.
    fn catalog_state() -> State {
        let mut got = State::new();
        got.set_catalog(ModelCatalog::Ready(vec![
            ModelInfo {
                id: String::from("muse-spark"),
                display_label: String::from("Muse Spark"),
                is_default: true,
                is_active: false,
            },
            ModelInfo {
                id: String::from("muse-mini"),
                display_label: String::from("Muse Mini"),
                is_default: false,
                is_active: false,
            },
        ]));
        got
    }

    /// Composer state with a ready skill catalog of two rows.
    fn skill_state() -> State {
        let mut got = State::new();
        got.set_skill_catalog(SkillCatalog::Ready(vec![
            SkillInfo {
                selector: String::from("review"),
                display_name: String::from("Code Review"),
                description: String::from("Reviews pull requests"),
                source: String::from("user"),
                plugin_id: None,
                argument_hint: None,
            },
            SkillInfo {
                selector: String::from("tests"),
                display_name: String::from("Tests"),
                description: String::from("Runs the suite"),
                source: String::from("bundled"),
                plugin_id: None,
                argument_hint: None,
            },
        ]));
        got
    }

    /// Selectors of the visible slash rows, in order.
    fn row_selectors(state: &State) -> Vec<&str> {
        state
            .slash_rows
            .iter()
            .map(|row| row.selector.as_str())
            .collect()
    }

    /// One unmodified key press for `named`.
    fn key_press(named: iced::keyboard::key::Named) -> text_editor::KeyPress {
        text_editor::KeyPress {
            key: iced::keyboard::Key::Named(named),
            modified_key: iced::keyboard::Key::Named(named),
            physical_key: iced::keyboard::key::Physical::Unidentified(
                iced::keyboard::key::NativeCode::Unidentified,
            ),
            modifiers: iced::keyboard::Modifiers::empty(),
            text: None,
            status: text_editor::Status::Focused { is_hovered: true },
        }
    }

    #[test]
    fn model_menu_selects_catalog_row_and_effort() {
        let mut got = catalog_state();
        // The catalog arrives without changing the selection.
        assert_eq!(got.model_effort_label, "Default · Default");
        got.update(Message::ModelPressed);
        assert_eq!(got.model_menu, ModelMenu::Open);
        got.update(Message::ModelEditPressed(ModelChoice::Default));
        assert_eq!(got.model_menu, ModelMenu::EffortOpen);
        got.update(Message::EffortSelected(EffortChoice::High));
        assert_eq!(got.effort, EffortChoice::High);
        assert_eq!(got.model_menu, ModelMenu::Closed);
        assert_eq!(got.model_effort_label, "Default · High");
        got.update(Message::ModelPressed);
        got.update(Message::ModelSelected(ModelChoice::Named(String::from(
            "muse-spark",
        ))));
        // Selecting a model keeps the menu open for its Edit action.
        assert_eq!(got.model_menu, ModelMenu::Open);
        assert_eq!(got.model_effort_label, "Muse Spark · High");
    }

    #[test]
    fn model_menu_effort_toggle_needs_open_menu() {
        // Edit is the effort-panel opener now; with no menu open it
        // selects nothing and opens nothing.
        let mut got = State::new();
        got.update(Message::ModelEditPressed(ModelChoice::Named(String::from(
            "muse-spark",
        ))));
        assert_eq!(got.model_menu, ModelMenu::Closed);
        assert_eq!(got.model, ModelChoice::Default);
    }

    #[test]
    fn model_edit_selects_target_and_opens_effort() {
        let mut got = catalog_state();
        got.update(Message::ModelPressed);
        got.update(Message::ModelEditPressed(ModelChoice::Named(String::from(
            "muse-spark",
        ))));
        assert_eq!(got.model, ModelChoice::Named(String::from("muse-spark")));
        assert_eq!(got.model_menu, ModelMenu::EffortOpen);
        assert_eq!(got.model_effort_label, "Muse Spark · Default");
    }

    #[test]
    fn model_edit_on_another_row_switches_and_keeps_effort_open() {
        let mut got = catalog_state();
        got.update(Message::ModelPressed);
        got.update(Message::ModelEditPressed(ModelChoice::Named(String::from(
            "muse-spark",
        ))));
        got.update(Message::ModelEditPressed(ModelChoice::Named(String::from(
            "muse-mini",
        ))));
        assert_eq!(got.model, ModelChoice::Named(String::from("muse-mini")));
        assert_eq!(got.model_menu, ModelMenu::EffortOpen);
        assert_eq!(got.model_effort_label, "Muse Mini · Default");
    }

    #[test]
    fn model_select_keeps_menu_open_and_closes_effort_panel() {
        let mut got = catalog_state();
        got.update(Message::ModelPressed);
        got.update(Message::ModelEditPressed(ModelChoice::Named(String::from(
            "muse-spark",
        ))));
        assert_eq!(got.model_menu, ModelMenu::EffortOpen);
        got.update(Message::ModelSelected(ModelChoice::Named(String::from(
            "muse-mini",
        ))));
        assert_eq!(got.model, ModelChoice::Named(String::from("muse-mini")));
        assert_eq!(got.model_menu, ModelMenu::Open);
    }

    #[test]
    fn effort_select_closes_whole_menu() {
        let mut got = catalog_state();
        got.update(Message::ModelPressed);
        got.update(Message::ModelEditPressed(ModelChoice::Named(String::from(
            "muse-spark",
        ))));
        got.update(Message::EffortSelected(EffortChoice::Low));
        assert_eq!(got.effort, EffortChoice::Low);
        assert_eq!(got.model_menu, ModelMenu::Closed);
        assert_eq!(got.model_effort_label, "Muse Spark · Low");
    }

    #[test]
    fn model_dismissed_closes_menu_and_keeps_picks() {
        let mut got = catalog_state();
        // Dismissing a closed menu is a no-op.
        got.update(Message::ModelDismissed);
        assert_eq!(got.model_menu, ModelMenu::Closed);
        got.update(Message::ModelPressed);
        got.update(Message::ModelEditPressed(ModelChoice::Named(String::from(
            "muse-spark",
        ))));
        assert_eq!(got.model_menu, ModelMenu::EffortOpen);
        got.update(Message::ModelDismissed);
        assert_eq!(got.model_menu, ModelMenu::Closed);
        // The picks made before the dismissal stand.
        assert_eq!(got.model, ModelChoice::Named(String::from("muse-spark")));
        assert_eq!(got.model_effort_label, "Muse Spark · Default");
    }

    #[test]
    fn permission_dismissed_closes_menu_and_keeps_pick() {
        let mut got = State::new();
        // Dismissing a closed menu is a no-op.
        got.update(Message::PermissionDismissed);
        assert_eq!(got.permission_menu, PermissionMenu::Closed);
        got.update(Message::PermissionPressed);
        assert_eq!(got.permission_menu, PermissionMenu::Open);
        got.update(Message::PermissionDismissed);
        assert_eq!(got.permission_menu, PermissionMenu::Closed);
        // The current posture stands.
        assert_eq!(got.permission, PermissionMode::default());
    }

    #[test]
    fn permission_menu_overlay_follows_menu_state() {
        let mut got = State::new();
        assert!(got.permission_menu_overlay().is_none());
        got.update(Message::PermissionPressed);
        assert!(got.permission_menu_overlay().is_some());
        got.update(Message::PermissionDismissed);
        assert!(got.permission_menu_overlay().is_none());
    }

    #[test]
    fn model_menu_overlay_follows_menu_state() {
        let mut got = catalog_state();
        assert!(got.model_menu_overlay().is_none());
        got.update(Message::ModelPressed);
        assert!(got.model_menu_overlay().is_some());
        got.update(Message::ModelEditPressed(ModelChoice::Default));
        assert_eq!(got.model_menu, ModelMenu::EffortOpen);
        assert!(got.model_menu_overlay().is_some());
        got.update(Message::ModelDismissed);
        assert!(got.model_menu_overlay().is_none());
    }

    #[test]
    fn opening_model_menu_closes_other_sub_bar_popups() {
        let mut got = State::new();
        got.update(Message::PermissionPressed);
        assert_eq!(got.permission_menu, PermissionMenu::Open);
        got.update(Message::ModelPressed);
        assert_eq!(got.model_menu, ModelMenu::Open);
        assert_eq!(got.permission_menu, PermissionMenu::Closed);
        got.update(Message::ContextPressed);
        assert_eq!(got.context_popover, ContextPopover::Open);
        assert_eq!(got.model_menu, ModelMenu::Closed);
        got.update(Message::ModelPressed);
        assert_eq!(got.model_menu, ModelMenu::Open);
        assert_eq!(got.context_popover, ContextPopover::Closed);
        got.update(Message::AttachPressed);
        assert_eq!(got.attach_popup, AttachPopup::Open);
        assert_eq!(got.model_menu, ModelMenu::Closed);
        got.update(Message::ModelPressed);
        assert_eq!(got.model_menu, ModelMenu::Open);
        assert_eq!(got.attach_popup, AttachPopup::Closed);
        // The pill toggles the whole menu closed, effort panel included.
        got.update(Message::ModelEditPressed(ModelChoice::Default));
        assert_eq!(got.model_menu, ModelMenu::EffortOpen);
        got.update(Message::ModelPressed);
        assert_eq!(got.model_menu, ModelMenu::Closed);
        // Opening the permission menu likewise closes the model menu.
        got.update(Message::ModelPressed);
        got.update(Message::PermissionPressed);
        assert_eq!(got.permission_menu, PermissionMenu::Open);
        assert_eq!(got.model_menu, ModelMenu::Closed);
    }

    #[test]
    fn failed_catalog_renders_its_message() {
        let mut got = State::new();
        got.set_catalog(ModelCatalog::Failed(String::from("models down")));
        assert_eq!(
            got.catalog,
            ModelCatalog::Failed(String::from("models down"))
        );
        // The pill still names the default choice without a catalog.
        // The menu itself renders a short generic line, never this raw
        // server text (see model_menu_view).
        assert_eq!(got.model_effort_label, "Default · Default");
    }

    #[test]
    fn unknown_model_id_falls_back_to_the_id() {
        let mut got = State::new();
        got.set_catalog(ModelCatalog::Ready(Vec::new()));
        got.update(Message::ModelSelected(ModelChoice::Named(String::from(
            "future-model",
        ))));
        assert_eq!(got.model_effort_label, "future-model · Default");
    }

    #[test]
    fn popup_list_height_fits_content_up_to_the_max() {
        // One row (and an empty catalog) fit exactly one row tall.
        assert_eq!(popup_list_height(0), MENU_ROW_HEIGHT);
        assert_eq!(popup_list_height(1), MENU_ROW_HEIGHT);
        // Two rows include the spacing between them.
        assert_eq!(popup_list_height(2), 2.0 * MENU_ROW_HEIGHT + SPACE_XS);
        // Many rows cap at the max height.
        assert_eq!(popup_list_height(100), MODEL_LIST_MAX_HEIGHT);
    }

    #[test]
    fn context_popover_toggles_and_labels_usage() {
        let mut got = State::new();
        assert_eq!(got.context_label, "Context —");
        assert_eq!(got.context_percent, None);
        got.update(Message::ContextPressed);
        assert_eq!(got.context_popover, ContextPopover::Open);
        got.set_usage(UsageSnapshot::parse(
            r#"{"prompt":57064,"output":16,"total":57080,"limit":200000}"#,
        ));
        assert_eq!(got.context_label, "57080 / 200000 · 29%");
        let percent = got.context_percent.expect("windowed usage meters");
        assert!((percent - 28.54).abs() < 0.01);
        got.update(Message::ContextPressed);
        assert_eq!(got.context_popover, ContextPopover::Closed);
    }

    #[test]
    fn context_dismissed_closes_popover_and_is_harmless_when_closed() {
        let mut got = State::new();
        got.update(Message::ContextDismissed);
        assert_eq!(got.context_popover, ContextPopover::Closed);
        got.update(Message::ContextPressed);
        assert_eq!(got.context_popover, ContextPopover::Open);
        got.update(Message::ContextDismissed);
        assert_eq!(got.context_popover, ContextPopover::Closed);
    }

    #[test]
    fn context_popover_overlay_follows_popover_state() {
        let mut got = State::new();
        assert!(got.context_popover_overlay().is_none());
        got.update(Message::ContextPressed);
        assert!(got.context_popover_overlay().is_some());
        got.update(Message::ContextDismissed);
        assert!(got.context_popover_overlay().is_none());
    }

    #[test]
    fn usage_without_window_has_no_meter() {
        let mut got = State::new();
        got.set_usage(UsageSnapshot::parse(r#"{"total": 42}"#));
        assert_eq!(got.context_label, "42 tokens");
        assert_eq!(got.context_percent, None);
        got.set_usage(None);
        assert_eq!(got.context_label, "Context —");
    }

    #[test]
    fn slash_opens_on_leading_slash_with_builtins_first() {
        let mut got = State::new();
        assert_eq!(got.slash_menu, SlashMenu::Closed);
        type_text(&mut got, "/");
        assert_eq!(got.slash_menu, SlashMenu::Open { selected: 0 });
        // Loading catalog: builtins still list.
        assert_eq!(row_selectors(&got), vec!["model", "compact", "goal"]);
        assert!(got.slash_menu_overlay().is_some());
    }

    #[test]
    fn slash_stays_closed_without_leading_slash() {
        let mut got = skill_state();
        type_text(&mut got, "hello /review");
        assert_eq!(got.slash_menu, SlashMenu::Closed);
        assert!(got.slash_rows.is_empty());
        assert!(got.slash_menu_overlay().is_none());
    }

    #[test]
    fn slash_space_after_command_closes_menu() {
        let mut got = skill_state();
        type_text(&mut got, "/compact");
        assert_eq!(got.slash_menu, SlashMenu::Open { selected: 0 });
        type_text(&mut got, " ");
        assert_eq!(got.slash_menu, SlashMenu::Closed);
        assert!(got.slash_rows.is_empty());
        assert!(got.slash_menu_overlay().is_none());
    }

    #[test]
    fn slash_args_close_menu_and_deleting_back_reopens() {
        let mut got = skill_state();
        type_text(&mut got, "/compact fix");
        assert_eq!(got.content.text(), "/compact fix");
        assert_eq!(got.slash_menu, SlashMenu::Closed);
        assert!(got.slash_rows.is_empty());
        for _ in 0.." fix".len() {
            got.update(Message::Edit(text_editor::Action::Edit(
                text_editor::Edit::Backspace,
            )));
        }
        assert_eq!(got.content.text(), "/compact");
        assert_eq!(got.slash_menu, SlashMenu::Open { selected: 0 });
        assert!(!got.slash_rows.is_empty());
    }

    #[test]
    fn slash_query_ignores_terminated_first_token() {
        // Terminated first tokens (the user is past the command
        // word) carry no query, so the menu closes.
        assert_eq!(slash_query("/compact "), None);
        assert_eq!(slash_query("/compact fix it"), None);
        assert_eq!(slash_query("/compact\nfix"), None);
        assert_eq!(slash_query("  /compact "), None);
        // Unterminated first tokens behave as before.
        assert_eq!(slash_query("/compact"), Some(String::from("compact")));
        assert_eq!(slash_query("/"), Some(String::new()));
        assert_eq!(slash_query("  /comp"), Some(String::from("comp")));
        assert_eq!(slash_query(""), None);
        assert_eq!(slash_query("   "), None);
        assert_eq!(slash_query("hello"), None);
        assert_eq!(slash_query("hello /review"), None);
    }

    #[test]
    fn slash_filter_matches_selector_name_and_description() {
        let mut got = skill_state();
        type_text(&mut got, "/rev");
        // Builtins carry no "rev"; only the review skill lists.
        assert_eq!(row_selectors(&got), vec!["review"]);
        let mut named = skill_state();
        type_text(&mut named, "/code");
        assert_eq!(row_selectors(&named), vec!["review"]);
        let mut described = skill_state();
        type_text(&mut described, "/pull");
        assert_eq!(row_selectors(&described), vec!["review"]);
        let mut upper = skill_state();
        type_text(&mut upper, "/REV");
        assert_eq!(row_selectors(&upper), vec!["review"]);
    }

    #[test]
    fn slash_arrows_move_and_wrap() {
        let mut got = skill_state();
        type_text(&mut got, "/");
        // Builtins first, then the catalog rows.
        assert_eq!(
            row_selectors(&got),
            vec!["model", "compact", "goal", "review", "tests"]
        );
        got.update(Message::SlashNext);
        assert_eq!(got.slash_menu, SlashMenu::Open { selected: 1 });
        got.update(Message::SlashPrev);
        assert_eq!(got.slash_menu, SlashMenu::Open { selected: 0 });
        // Wraps both directions.
        got.update(Message::SlashPrev);
        assert_eq!(got.slash_menu, SlashMenu::Open { selected: 4 });
        got.update(Message::SlashNext);
        assert_eq!(got.slash_menu, SlashMenu::Open { selected: 0 });
    }

    #[test]
    fn slash_arrows_are_noops_without_rows() {
        let mut got = skill_state();
        type_text(&mut got, "/zzz");
        assert!(got.slash_rows.is_empty());
        assert_eq!(got.slash_menu, SlashMenu::Open { selected: 0 });
        got.update(Message::SlashNext);
        got.update(Message::SlashPrev);
        got.update(Message::SlashAccept);
        assert_eq!(got.slash_menu, SlashMenu::Open { selected: 0 });
        assert_eq!(got.content.text(), "/zzz");
    }

    #[test]
    fn slash_accept_inserts_selector_and_closes() {
        let mut got = skill_state();
        type_text(&mut got, "/rev");
        got.update(Message::SlashNext);
        got.update(Message::SlashPrev);
        got.update(Message::SlashAccept);
        assert_eq!(got.content.text(), "/review ");
        assert_eq!(got.slash_menu, SlashMenu::Closed);
        assert!(got.slash_rows.is_empty());
        assert!(got.slash_menu_overlay().is_none());
    }

    #[test]
    fn slash_click_accepts_row_identity() {
        let mut got = skill_state();
        type_text(&mut got, "/");
        got.update(Message::SlashSelected(SlashPick::Skill(String::from(
            "tests",
        ))));
        assert_eq!(got.content.text(), "/tests ");
        assert_eq!(got.slash_menu, SlashMenu::Closed);
        // An unknown pick is a no-op: the draft and menu survive it.
        let mut unknown = skill_state();
        type_text(&mut unknown, "/");
        unknown.update(Message::SlashSelected(SlashPick::Skill(String::from(
            "nope",
        ))));
        assert_eq!(unknown.content.text(), "/");
        assert_eq!(unknown.slash_menu, SlashMenu::Open { selected: 0 });
    }

    #[test]
    fn slash_esc_dismisses_and_editing_reopens() {
        let mut got = skill_state();
        type_text(&mut got, "/rev");
        assert!(matches!(got.slash_menu, SlashMenu::Open { .. }));
        got.update(Message::SlashDismiss);
        assert_eq!(
            got.slash_menu,
            SlashMenu::Dismissed {
                text: String::from("/rev"),
            }
        );
        assert!(got.slash_menu_overlay().is_none());
        // The same text does not reopen: a refresh with no edit is
        // a no-op.
        got.refresh_slash_menu();
        assert_eq!(
            got.slash_menu,
            SlashMenu::Dismissed {
                text: String::from("/rev"),
            }
        );
        // Any edit re-arms the menu.
        type_text(&mut got, "i");
        assert_eq!(got.slash_menu, SlashMenu::Open { selected: 0 });
        assert_eq!(row_selectors(&got), vec!["review"]);
    }

    #[test]
    fn slash_empty_catalog_lists_builtins_only() {
        let mut got = State::new();
        got.set_skill_catalog(SkillCatalog::Ready(Vec::new()));
        type_text(&mut got, "/");
        assert_eq!(row_selectors(&got), vec!["model", "compact", "goal"]);
        type_text(&mut got, "zzz");
        assert!(got.slash_rows.is_empty());
    }

    #[test]
    fn slash_failed_catalog_keeps_composer_usable() {
        let mut got = State::new();
        got.set_skill_catalog(SkillCatalog::Failed(String::from("boom")));
        type_text(&mut got, "/zzz");
        // No rows, but the draft and the menu shell survive.
        assert!(got.slash_rows.is_empty());
        assert_eq!(got.slash_menu, SlashMenu::Open { selected: 0 });
        assert!(got.slash_menu_overlay().is_some());
        assert_eq!(got.content.text(), "/zzz");
    }

    #[test]
    fn slash_builtin_model_accept_opens_model_menu() {
        let mut got = skill_state();
        type_text(&mut got, "/mod");
        assert_eq!(row_selectors(&got), vec!["model"]);
        got.update(Message::SlashAccept);
        assert_eq!(got.content.text(), "/model ");
        assert_eq!(got.slash_menu, SlashMenu::Closed);
        assert_eq!(got.model_menu, ModelMenu::Open);
    }

    #[test]
    fn slash_selection_clamps_when_filter_shrinks() {
        let mut got = skill_state();
        type_text(&mut got, "/");
        got.update(Message::SlashNext);
        got.update(Message::SlashNext);
        got.update(Message::SlashNext);
        got.update(Message::SlashNext);
        assert_eq!(got.slash_menu, SlashMenu::Open { selected: 4 });
        // Narrowing the filter clamps the selection into range.
        type_text(&mut got, "zzz");
        assert_eq!(got.slash_menu, SlashMenu::Open { selected: 0 });
    }

    #[test]
    fn slash_clear_closes_menu() {
        let mut got = skill_state();
        type_text(&mut got, "/rev");
        assert!(matches!(got.slash_menu, SlashMenu::Open { .. }));
        got.clear();
        assert_eq!(got.slash_menu, SlashMenu::Closed);
        assert!(got.slash_rows.is_empty());
    }

    #[test]
    fn slash_source_icons_cover_every_scope() {
        // `Icon` carries no `PartialEq`, so the mapping asserts by
        // pattern: every known scope and the unknown-scope fallback.
        use lucide_icons::Icon;
        assert!(matches!(slash_source_icon("builtin"), Icon::SquareTerminal));
        assert!(matches!(slash_source_icon("bundled"), Icon::Package));
        assert!(matches!(slash_source_icon("user"), Icon::User));
        assert!(matches!(slash_source_icon("project"), Icon::Folder));
        assert!(matches!(slash_source_icon("plugin"), Icon::Puzzle));
        assert!(matches!(slash_source_icon(""), Icon::Sparkles));
        assert!(matches!(slash_source_icon("mystery"), Icon::Sparkles));
    }

    #[test]
    fn slash_token_range_highlights_exact_first_token_only() {
        let selectors = vec![String::from("review"), String::from("goal")];
        assert_eq!(
            slash_token_range("/review", &selectors),
            Some(0..7),
            "exact match highlights the token"
        );
        assert_eq!(
            slash_token_range("/review fix it", &selectors),
            Some(0..7),
            "arguments never highlight"
        );
        assert_eq!(
            slash_token_range("/rev", &selectors),
            None,
            "prefix match does not highlight"
        );
        assert_eq!(
            slash_token_range("/nope", &selectors),
            None,
            "unknown selector does not highlight"
        );
        assert_eq!(
            slash_token_range("hello", &selectors),
            None,
            "plain text does not highlight"
        );
        assert_eq!(slash_token_range("/", &selectors), None);
        assert_eq!(slash_token_range("", &selectors), None);
        assert_eq!(
            slash_token_range("  /goal x", &selectors),
            Some(2..7),
            "leading whitespace still anchors the first token"
        );
    }

    #[test]
    fn slash_highlighter_marks_first_line_only() {
        use iced::widget::text::Highlighter;
        let selectors = vec![String::from("review")];
        let mut got = SlashHighlighter::new(&SlashHighlightSettings::for_draft(
            &selectors,
            "/review fix",
        ));
        let first: Vec<_> = got.highlight_line("/review fix").collect();
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].0, 0..7);
        // The second fed line never highlights, even for a match.
        let second: Vec<_> = got.highlight_line("/review").collect();
        assert!(second.is_empty());
        // A change notification re-arms the position.
        got.change_line(0);
        let again: Vec<_> = got.highlight_line("/review").collect();
        assert_eq!(again.len(), 1);
        // New settings re-arm from line 0 through update, whether
        // the selectors changed or only the token validity flipped.
        got.update(&SlashHighlightSettings::for_draft(
            &[String::from("tests")],
            "/review",
        ));
        assert_eq!(got.current_line(), 0);
        let switched: Vec<_> = got.highlight_line("/review").collect();
        assert!(switched.is_empty());
        let matched: Vec<_> = {
            got.change_line(0);
            got.highlight_line("/tests x").collect()
        };
        assert_eq!(matched.len(), 1);
        assert_eq!(matched[0].0, 0..6);
    }

    #[test]
    fn slash_keys_map_menu_gestures_before_the_editor() {
        use iced::keyboard::key::Named;
        let accept = |named| slash_key_binding(key_press(named), true);
        assert_eq!(
            accept(Named::ArrowDown),
            Some(text_editor::Binding::Custom(Message::SlashNext))
        );
        assert_eq!(
            accept(Named::ArrowUp),
            Some(text_editor::Binding::Custom(Message::SlashPrev))
        );
        assert_eq!(
            accept(Named::Enter),
            Some(text_editor::Binding::Custom(Message::SlashAccept))
        );
        assert_eq!(
            accept(Named::Tab),
            Some(text_editor::Binding::Custom(Message::SlashAccept))
        );
        assert_eq!(
            accept(Named::Escape),
            Some(text_editor::Binding::Custom(Message::SlashDismiss))
        );
        // Without rows, navigation and accept fall back to the editor
        // defaults; Esc still dismisses.
        let bare = |named| slash_key_binding(key_press(named), false);
        assert_eq!(
            bare(Named::Escape),
            Some(text_editor::Binding::Custom(Message::SlashDismiss))
        );
        assert_eq!(
            bare(Named::ArrowDown),
            text_editor::Binding::from_key_press(key_press(Named::ArrowDown))
        );
        assert_eq!(
            bare(Named::Enter),
            text_editor::Binding::from_key_press(key_press(Named::Enter))
        );
    }

    #[test]
    fn slash_modified_enter_falls_back_to_editor_defaults() {
        use iced::keyboard::key::Named;
        let mut press = key_press(Named::Enter);
        press.modifiers = iced::keyboard::Modifiers::CTRL;
        // Ctrl+Enter keeps its pending-shortcut meaning: the editor
        // default applies, never an accept.
        assert_eq!(
            slash_key_binding(press.clone(), true),
            text_editor::Binding::from_key_press(press)
        );
    }

    #[test]
    fn slash_token_style_reads_regular_primary_from_the_theme() {
        // Color-only by contract: a mid-line weight change desyncs the
        // caret from the shaped text, while color cannot affect caret
        // geometry on any backend.
        let (color, font) = slash_token_style(&crate::theme::manifest_theme());
        let expected = crate::theme::manifest_theme()
            .extended_palette()
            .primary
            .strong
            .color;
        assert_eq!(color, expected);
        assert_eq!(font, iced::Font::DEFAULT);
    }

    #[test]
    fn slash_fresh_content_starts_caret_at_buffer_start() {
        // Premise the accept path works around: a fresh value starts
        // its caret at 0,0, so replacing the draft outright would park
        // the caret at the start of the inserted command.
        let fresh = text_editor::Content::<iced::Renderer>::with_text("/review ");
        assert_eq!(fresh.cursor().position.line, 0);
        assert_eq!(fresh.cursor().position.column, 0);
    }

    #[test]
    fn slash_accept_lands_caret_after_inserted_token() {
        let mut got = skill_state();
        type_text(&mut got, "/rev");
        got.update(Message::SlashAccept);
        assert_eq!(got.content.text(), "/review ");
        let cursor = got.content.cursor();
        assert_eq!(cursor.position.line, 0);
        assert_eq!(
            cursor.position.column,
            "/review ".len(),
            "caret lands directly after the inserted `/selector `"
        );
        assert_eq!(cursor.selection, None, "accept leaves no selection");
    }

    #[test]
    fn slash_click_accept_lands_caret_after_inserted_token() {
        let mut got = skill_state();
        type_text(&mut got, "/");
        got.update(Message::SlashSelected(SlashPick::Skill(String::from(
            "tests",
        ))));
        assert_eq!(got.content.text(), "/tests ");
        let cursor = got.content.cursor();
        assert_eq!(cursor.position.line, 0);
        assert_eq!(
            cursor.position.column,
            "/tests ".len(),
            "mouse accept lands the caret like keyboard accept"
        );
        assert_eq!(cursor.selection, None, "accept leaves no selection");
    }

    #[test]
    fn slash_highlight_settings_flip_with_token_validity() {
        let mut got = skill_state();
        let settings_for = |state: &State| {
            crate::composer::SlashHighlightSettings::for_draft(
                &state.highlight_selectors,
                &state.content.text(),
            )
        };
        // A partial token is not valid: no re-arm.
        type_text(&mut got, "/revie");
        let partial = settings_for(&got);
        assert!(!partial.token_valid());
        // Completing the exact selector flips validity, so the next
        // layout re-arms the highlighter with no extra keystroke.
        type_text(&mut got, "w");
        let exact = settings_for(&got);
        assert!(exact.token_valid());
        assert_ne!(
            partial, exact,
            "validity flip must change settings or the widget never re-highlights"
        );
        // Deleting back to a prefix flips validity off again, so the
        // span clears on the next draw.
        got.update(Message::Edit(text_editor::Action::Edit(
            text_editor::Edit::Backspace,
        )));
        let reverted = settings_for(&got);
        assert!(!reverted.token_valid());
        assert_ne!(exact, reverted);
        // Typing the trailing space keeps the token valid: the token
        // itself still matches, so the style must survive the space.
        type_text(&mut got, "w ");
        assert!(settings_for(&got).token_valid());
    }

    /// Mirror of the vendored editor protocol the widget runs
    /// (iced_widget 0.14.2 `text_editor.rs` layout/draw with
    /// iced_graphics 0.14.0 `text/editor.rs` update/highlight): each
    /// edit notifies `change_line` with the topmost changed line, and
    /// each draw feeds `highlight_line` for lines
    /// `current_line..=last_visible` unless the highlighter is already
    /// past the visible lines.
    struct ProtocolEditor {
        highlighter: SlashHighlighter,
        line: String,
    }

    impl ProtocolEditor {
        fn new(settings: &SlashHighlightSettings) -> Self {
            use iced::widget::text::Highlighter;
            Self {
                highlighter: SlashHighlighter::new(settings),
                line: String::new(),
            }
        }

        /// One keystroke: the buffer changes, then layout consumes the
        /// edit notification exactly once.
        fn edit(&mut self, line: &str) {
            use iced::widget::text::Highlighter;
            self.line = line.to_string();
            self.highlighter.change_line(0);
        }

        /// One draw over a single visible line.
        fn draw(&mut self) -> Vec<(Range<usize>, SlashHighlight)> {
            use iced::widget::text::Highlighter;
            if self.highlighter.current_line() > 0 {
                return Vec::new();
            }
            self.highlighter.highlight_line(&self.line).collect()
        }
    }

    #[test]
    fn slash_highlighter_styles_exact_token_under_editor_protocol() {
        let selectors = vec![String::from("compact")];
        let mut editor = ProtocolEditor::new(&SlashHighlightSettings::for_draft(&selectors, ""));
        // Typing a prefix never styles, draw after draw.
        for prefix in ["/", "/c", "/compa"] {
            editor.edit(prefix);
            assert!(editor.draw().is_empty(), "prefix {prefix} must not style");
        }
        // Typing the final character styles on that draw: no trailing
        // space required. The edit notification alone re-arms the
        // highlighter; the settings flip in the widget covers the same
        // transition through its own update path.
        editor.edit("/compact");
        let styled = editor.draw();
        assert_eq!(styled.len(), 1, "exact token styles immediately");
        assert_eq!(styled[0].0, 0..8);
        // Deleting one character reverts on the next draw.
        editor.edit("/compac");
        assert!(
            editor.draw().is_empty(),
            "deleting out of the exact token clears the span"
        );
    }
}
