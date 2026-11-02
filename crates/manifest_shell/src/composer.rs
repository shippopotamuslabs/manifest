//! Composer: the draft input cluster at the foot of the main column.
//!
//! The input grows as the user types, the attach and send buttons sit
//! inside the bar, and the sub-bar carries live state: the permission
//! posture, the combined model-effort picker, and the context meter.
//! The App owns the engine and the visible session, so it intercepts
//! the send, stop, attach-picker, and menu-selection messages; the
//! reducer here only folds popup toggles and local display state.

use iced::widget::text_editor;
use iced::{Element, Length};
use manifest_domain::{
    EffortChoice, ModelChoice, ModelInfo, MusePermission, PermissionMode, UsageSnapshot,
};

use crate::kit::{self, IconButtonKind};
use crate::theme::{
    ATTACH_POPUP_WIDTH, COMPOSER_MAX_LINES, CONTEXT_POPUP_WIDTH, EFFORT_POPUP_WIDTH,
    FONT_SIZE_PANEL, MENU_ROW_HEIGHT, MODEL_LIST_MAX_HEIGHT, MODEL_POPUP_WIDTH,
    PERMISSION_POPUP_WIDTH, SPACE_S, SPACE_XS, TEXT_FAINT, TEXT_TERTIARY,
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
            attach_popup: AttachPopup::Closed,
            permission_menu: PermissionMenu::Closed,
            model_menu: ModelMenu::Closed,
            context_popover: ContextPopover::Closed,
            permission: PermissionMode::default(),
            model: ModelChoice::default(),
            effort: EffortChoice::default(),
            catalog: ModelCatalog::Loading,
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
        self.refresh_derived();
    }

    /// Restores a failed send's prompt into the draft: the inverse of
    /// [`State::clear`], recomputing the derived values from the text
    /// while preserving the App-owned gates.
    pub fn restore(&mut self, text: String) {
        self.content = text_editor::Content::with_text(&text);
        self.has_text = !self.content.text().is_empty();
        self.visible_lines = self.content.line_count().clamp(1, COMPOSER_MAX_LINES);
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
            }
            Message::AttachPressed => {
                self.attach_popup = match self.attach_popup {
                    AttachPopup::Closed => {
                        self.permission_menu = PermissionMenu::Closed;
                        self.model_menu = ModelMenu::Closed;
                        self.context_popover = ContextPopover::Closed;
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
                        ContextPopover::Open
                    }
                    ContextPopover::Open => ContextPopover::Closed,
                };
            }
            Message::ContextDismissed => {
                self.context_popover = ContextPopover::Closed;
            }
            Message::SendPressed
            | Message::StopPressed
            | Message::AttachFilesPressed
            | Message::AttachPicked(_) => {}
        }
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
                        .height(Length::Fixed(model_list_height(models.len())))
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
}

impl Default for State {
    fn default() -> Self {
        Self::new()
    }
}

/// Height of the scrollable catalog list for `row_count` models:
/// fits the rows (and the spacing between them) up to
/// [`MODEL_LIST_MAX_HEIGHT`], and is at least one row tall so an
/// empty catalog still occupies a single row.
fn model_list_height(row_count: usize) -> f32 {
    let rows = row_count.max(1);
    let spacing = (rows - 1) as f32 * SPACE_XS;
    (rows as f32 * MENU_ROW_HEIGHT + spacing).min(MODEL_LIST_MAX_HEIGHT)
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
    fn model_list_height_fits_content_up_to_the_max() {
        // One row (and an empty catalog) fit exactly one row tall.
        assert_eq!(model_list_height(0), MENU_ROW_HEIGHT);
        assert_eq!(model_list_height(1), MENU_ROW_HEIGHT);
        // Two rows include the spacing between them.
        assert_eq!(model_list_height(2), 2.0 * MENU_ROW_HEIGHT + SPACE_XS);
        // Many rows cap at the max height.
        assert_eq!(model_list_height(100), MODEL_LIST_MAX_HEIGHT);
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
}
