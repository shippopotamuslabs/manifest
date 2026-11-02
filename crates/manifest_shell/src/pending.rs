//! Pending approvals/questions: card state, reducer, and simple view.
//!
//! Cards are source-of-truth state fetched with `muse_pending`; stream
//! hints only trigger a fetch. This module owns the per-session card
//! set and the interaction state (pager, selections, synthetic Other
//! text, deny reason). The transcript holds only a `Pending{request_id}`
//! entry at the pause point; this state decides what that entry means.
//! Rendering is the inline transcript card at the pause point.

use std::collections::{HashMap, HashSet};

use iced::{Element, Length};
use manifest_domain::{
    AnswerValue, ApprovalDecision, ApprovalRequest, PendingAnswer, PendingKind, PendingRequest,
    PendingResolution, Question, QuestionRequest, SessionId,
};

use crate::kit;
use crate::theme::{
    DANGER, FONT_SIZE_BODY, FONT_SIZE_SMALL, SPACE_S, TEXT_PRIMARY, TEXT_SECONDARY,
};

/// Label of the shell-added synthetic option. Never in a payload; a
/// real option literally named `None` is a plain label and gets no
/// text box, while this label opens the free-text box.
pub const OTHER_LABEL: &str = "Other";

/// Where a card stands. Enums name the situations (UI contract §1):
/// no bool tuples can describe a settled card that is also awaiting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Phase {
    /// Awaiting the user's decision/answers; blocks the composer.
    Awaiting,
    /// A decide/answer call is in flight.
    Deciding,
    /// Settled; collapses to the one-line record.
    Settled(PendingResolution),
    /// Vanished from the fetch before a settlement was seen:
    /// approvals expire to deny, questions cancel on timeout.
    Expired,
    /// The decide/answer (or its reconciliation) failed; card stays
    /// with a Retry that re-fetches. Carries the failure message.
    Failed(String),
}

/// Whether the deny reason editor shows. An enum, not a bool pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DenyEditor {
    /// The reason editor is hidden.
    #[default]
    Hidden,
    /// The reason editor is showing.
    Editing,
}

/// Interaction state for one card, kept apart from the payload so a
/// fetch refresh updates the payload without losing the user's place.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Interaction {
    /// Current question index (pager); approvals stay at 0.
    pub pager_index: usize,
    /// Picked real option labels per question id, in pick order.
    pub selections: HashMap<String, Vec<String>>,
    /// Question ids where the synthetic Other radio is picked.
    pub other_selected: HashSet<String>,
    /// Free text per question id, for the synthetic Other box.
    pub other_text: HashMap<String, String>,
    /// Question ids the user skipped.
    pub skipped: HashSet<String>,
    /// Typed deny reason (approvals).
    pub deny_reason: String,
    /// Whether the deny reason editor shows.
    pub deny_editor: DenyEditor,
}

/// One card: the fetched payload plus its interaction and phase.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingCardState {
    /// The fetched request (source of truth payload).
    pub request: PendingRequest,
    /// Pager/selection/text state.
    pub interaction: Interaction,
    /// Lifecycle phase.
    pub phase: Phase,
}

impl PendingCardState {
    /// A fresh awaiting card for `request`.
    pub fn new(request: PendingRequest) -> Self {
        Self {
            request,
            interaction: Interaction::default(),
            phase: Phase::Awaiting,
        }
    }

    /// Session this card belongs to.
    pub fn session(&self) -> &SessionId {
        self.request.session()
    }

    /// Opaque request handle.
    pub fn request_id(&self) -> &str {
        self.request.request_id()
    }

    /// Whether the card still needs the user (blocks the composer).
    pub fn is_awaiting(&self) -> bool {
        matches!(self.phase, Phase::Awaiting)
    }

    /// Whether the card counts as pending for a session badge:
    /// anything not yet settled or expired.
    pub fn is_pending(&self) -> bool {
        matches!(
            self.phase,
            Phase::Awaiting | Phase::Deciding | Phase::Failed(_)
        )
    }

    /// The current question, for question cards.
    pub fn current_question(&self) -> Option<&Question> {
        let questions = self.request.questions()?;
        questions.get(self.interaction.pager_index)
    }

    /// One-line settled/expired record: asked, decided, by whom.
    /// Approvals name their subject; answers carry their summary.
    pub fn settled_line(&self) -> String {
        let subject = match &self.request {
            PendingRequest::Approval { approval, .. } => {
                if approval.subject_line.is_empty() {
                    approval.tool.clone()
                } else {
                    approval.subject_line.clone()
                }
            }
            PendingRequest::Question { question, .. } => question
                .questions
                .first()
                .map(|q| q.prompt.clone())
                .unwrap_or_default(),
        };
        match &self.phase {
            Phase::Settled(PendingResolution::Approved) => {
                format!("Approval · {subject} — Approved by you")
            }
            Phase::Settled(PendingResolution::Denied(reason)) => match reason {
                Some(reason) if !reason.trim().is_empty() => {
                    format!("Approval · {subject} — Denied by you: {reason}")
                }
                _ => format!("Approval · {subject} — Denied by you"),
            },
            Phase::Settled(PendingResolution::Answered(summary)) => {
                format!("Question · {subject} — Answered by you: {summary}")
            }
            Phase::Settled(PendingResolution::Cancelled) => {
                format!("Question · {subject} — Cancelled")
            }
            Phase::Settled(PendingResolution::Expired) | Phase::Expired => match &self.request {
                PendingRequest::Approval { .. } => {
                    format!("Approval · {subject} — Expired, denied automatically")
                }
                PendingRequest::Question { .. } => {
                    format!("Question · {subject} — Expired, cancelled")
                }
            },
            Phase::Awaiting | Phase::Deciding | Phase::Failed(_) => {
                String::from("Needs your input")
            }
        }
    }

    /// Whether the synthetic Other free-text box shows for `question_id`.
    pub fn other_field_visible(&self, question_id: &str) -> bool {
        self.interaction.other_selected.contains(question_id)
    }

    /// Whether `label` is picked for `question_id` (real options only).
    pub fn is_selected(&self, question_id: &str, label: &str) -> bool {
        self.interaction
            .selections
            .get(question_id)
            .is_some_and(|picked| picked.iter().any(|p| p == label))
    }

    /// Footer primary label: `Submit` on the last question, else `Next`.
    pub fn next_label(&self) -> &'static str {
        let total = self.request.questions().map_or(0, <[Question]>::len);
        if total == 0 || self.interaction.pager_index + 1 >= total {
            "Submit"
        } else {
            "Next"
        }
    }

    /// Assemble the full answer once, from selections/Other/skips.
    /// Every question gets exactly one entry (spike encoding: a skip
    /// is `AnswerValue::Skipped`, never empty free text).
    fn assemble_answer(&self) -> Option<PendingAnswer> {
        let PendingRequest::Question { question, .. } = &self.request else {
            return None;
        };
        let mut answers = Vec::with_capacity(question.questions.len());
        for q in &question.questions {
            let value = if self.interaction.skipped.contains(&q.id) {
                AnswerValue::Skipped
            } else if self.interaction.other_selected.contains(&q.id) {
                AnswerValue::free_text(
                    self.interaction
                        .other_text
                        .get(&q.id)
                        .map(String::as_str)
                        .unwrap_or(""),
                )
            } else {
                match self
                    .interaction
                    .selections
                    .get(&q.id)
                    .filter(|picked| !picked.is_empty())
                {
                    Some(picked) if q.multi => AnswerValue::Labels(picked.clone()),
                    Some(picked) => AnswerValue::Label(picked[0].clone()),
                    None => AnswerValue::Skipped,
                }
            };
            answers.push((q.id.clone(), value));
        }
        Some(PendingAnswer::new(answers))
    }
}

/// Pending messages, enumerated before the view (UI contract §1).
/// `request` is the opaque request id; cards are found by it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    /// Advance the question pager.
    PagerNext {
        /// Request id of the card.
        request: String,
    },
    /// Go back in the question pager.
    PagerPrev {
        /// Request id of the card.
        request: String,
    },
    /// Pick (or toggle, when multiple) an option. `label` is a real
    /// option label, or [`OTHER_LABEL`] for the synthetic Other.
    SelectOption {
        /// Request id of the card.
        request: String,
        /// Question id the option belongs to.
        question: String,
        /// Picked label.
        label: String,
    },
    /// The synthetic Other free-text box changed.
    OtherTextChanged {
        /// Request id of the card.
        request: String,
        /// Question id the text belongs to.
        question: String,
        /// New text.
        text: String,
    },
    /// Skip the current question: records a skip and advances.
    SkipQuestion {
        /// Request id of the card.
        request: String,
    },
    /// Submit all answers; assembles once on the last question.
    /// Before the last question it acts as Next.
    SubmitAnswers {
        /// Request id of the card.
        request: String,
    },
    /// Approve an approval card.
    Approve {
        /// Request id of the card.
        request: String,
    },
    /// Toggle the deny reason editor.
    DenyToggle {
        /// Request id of the card.
        request: String,
    },
    /// The deny reason text changed.
    DenyReasonChanged {
        /// Request id of the card.
        request: String,
        /// New reason text.
        text: String,
    },
    /// Confirm a deny, carrying the reason when one was typed.
    ConfirmDeny {
        /// Request id of the card.
        request: String,
    },
    /// Retry after a failure: the card stays, truth is re-fetched.
    RetryFetch {
        /// Request id of the failed card.
        request: String,
    },
}

/// What the reducer asks the App/engine to do. State cannot do I/O
/// (UI contract §2), so engine calls ride out as effects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// Nothing to do.
    None,
    /// Re-fetch the pending set (`muse_pending`).
    FetchPending,
    /// Decide an approval.
    Decide {
        /// Session owning the card.
        session: SessionId,
        /// Request id to decide.
        request_id: String,
        /// Approve or deny.
        decision: ApprovalDecision,
        /// Deny reason, when typed.
        reason: Option<String>,
    },
    /// Answer a question set, assembled once.
    Answer {
        /// Session owning the card.
        session: SessionId,
        /// Request id to answer.
        request_id: String,
        /// The assembled answers.
        answer: PendingAnswer,
    },
}

/// Outcome of replacing the card set from a fetch, for the App to
/// reconcile transcript entries with.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LoadedDiff {
    /// `(session, request_id)` cards seen for the first time: insert
    /// a transcript entry at the pause point (the end).
    pub inserted: Vec<(SessionId, String)>,
    /// `(session, request_id, one-line record)` cards that vanished
    /// unsettled: collapse their entry to the expired record.
    pub expired: Vec<(SessionId, String, String)>,
}

/// All pending cards, by session. Settled/expired cards stay so
/// their transcript record and counts remain derivable.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct State {
    /// Cards per session, in fetch order.
    pub cards: HashMap<SessionId, Vec<PendingCardState>>,
    /// `Requested` stream hints not yet satisfied by a successful
    /// fetch: `(session, request_id, kind)`. A failed fetch turns
    /// each hint with no loaded card into a placeholder card; a
    /// successful fetch (or a settlement) clears the hints it
    /// resolves, present or absent.
    pub hinted: Vec<(SessionId, String, PendingKind)>,
}

impl State {
    /// Cards for one session, empty when none.
    pub fn cards_for(&self, session: &SessionId) -> &[PendingCardState] {
        self.cards.get(session).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Find a card by request id, across sessions.
    pub fn card(&self, request_id: &str) -> Option<&PendingCardState> {
        self.cards
            .values()
            .flat_map(|cards| cards.iter())
            .find(|card| card.request_id() == request_id)
    }

    /// Mutable card by request id, across sessions.
    fn card_mut(&mut self, request_id: &str) -> Option<&mut PendingCardState> {
        self.cards
            .values_mut()
            .flat_map(|cards| cards.iter_mut())
            .find(|card| card.request_id() == request_id)
    }

    /// All non-zero pending counts by session, for the left-panel badge.
    pub fn pending_counts(&self) -> HashMap<SessionId, u64> {
        self.cards
            .keys()
            .filter_map(|session| {
                let count = self.pending_count(session) as u64;
                (count > 0).then(|| (session.clone(), count))
            })
            .collect()
    }

    /// First awaiting question card's request id for `session`.
    pub fn first_awaiting_question(&self, session: &SessionId) -> Option<String> {
        self.cards_for(session)
            .iter()
            .find(|card| card.is_awaiting() && card.request.questions().is_some())
            .map(|card| card.request_id().to_string())
    }

    /// Unsettled count for a background session's left-panel row.
    pub fn pending_count(&self, session: &SessionId) -> usize {
        self.cards_for(session)
            .iter()
            .filter(|card| card.is_pending())
            .count()
    }

    /// Whether `session` has a card awaiting the user (composer gate).
    pub fn has_awaiting(&self, session: &SessionId) -> bool {
        self.cards_for(session)
            .iter()
            .any(PendingCardState::is_awaiting)
    }

    /// Remember a `Requested` hint so a failed fetch can name the
    /// request it could not load. Deduped by `(session, request_id)`.
    pub fn record_hint(&mut self, session: SessionId, request_id: String, kind: PendingKind) {
        if !self
            .hinted
            .iter()
            .any(|(s, id, _)| s == &session && id == &request_id)
        {
            self.hinted.push((session, request_id, kind));
        }
    }

    /// Snapshot of the unsettled hinted requests, in hint order.
    pub fn hinted_requests(&self) -> Vec<(SessionId, String, PendingKind)> {
        self.hinted.clone()
    }

    /// Materialize a placeholder card for a hinted request whose
    /// fetch failed: minimal payload of the hinted kind, in
    /// [`Phase::Failed`] so the card renders the named error with a
    /// Retry that re-fetches. Returns true when a new card was
    /// inserted (the caller then pushes its transcript entry). An
    /// existing card is never replaced; an existing failed card
    /// only refreshes its failure message.
    pub fn mark_hint_failed(
        &mut self,
        session: &SessionId,
        request_id: &str,
        kind: PendingKind,
        message: String,
    ) -> bool {
        if let Some(card) = self
            .cards
            .get_mut(session)
            .and_then(|cards| cards.iter_mut().find(|c| c.request_id() == request_id))
        {
            if matches!(card.phase, Phase::Failed(_)) {
                card.phase = Phase::Failed(message);
            }
            return false;
        }
        let request = match kind {
            PendingKind::Approval => PendingRequest::Approval {
                session: session.clone(),
                request_id: String::from(request_id),
                approval: ApprovalRequest {
                    tool: String::new(),
                    subject_line: String::new(),
                    choices: Vec::new(),
                },
            },
            PendingKind::UserInput => PendingRequest::Question {
                session: session.clone(),
                request_id: String::from(request_id),
                question: QuestionRequest {
                    questions: Vec::new(),
                },
            },
        };
        let mut card = PendingCardState::new(request);
        card.phase = Phase::Failed(message);
        self.cards.entry(session.clone()).or_default().push(card);
        true
    }

    /// Replace the set wholesale from a fetch (source of truth).
    /// Present cards update their payload and keep interaction;
    /// a failed card that is present again returns to awaiting.
    /// Vanished unsettled cards become expired records, never a
    /// spinner and never a silent drop.
    pub fn apply_loaded(&mut self, requests: Vec<PendingRequest>) -> LoadedDiff {
        let mut diff = LoadedDiff::default();
        let incoming: HashSet<(SessionId, String)> = requests
            .iter()
            .map(|r| (r.session().clone(), r.request_id().to_string()))
            .collect();
        for request in requests {
            let session = request.session().clone();
            let request_id = request.request_id().to_string();
            let cards = self.cards.entry(session.clone()).or_default();
            match cards
                .iter_mut()
                .find(|card| card.request_id() == request_id)
            {
                Some(card) => {
                    card.request = request;
                    if matches!(card.phase, Phase::Failed(_)) {
                        card.phase = Phase::Awaiting;
                    }
                }
                None => {
                    cards.push(PendingCardState::new(request));
                    diff.inserted.push((session, request_id));
                }
            }
        }
        for (session, cards) in self.cards.iter_mut() {
            for card in cards.iter_mut() {
                let key = (session.clone(), card.request_id().to_string());
                if !incoming.contains(&key) && card.is_pending() {
                    card.phase = Phase::Expired;
                    diff.expired.push((
                        session.clone(),
                        card.request_id().to_string(),
                        card.settled_line(),
                    ));
                }
            }
        }
        // A successful fetch is the truth for every hint: present
        // hints are satisfied by their loaded (or revived) card,
        // absent hints settled elsewhere and need no placeholder.
        self.hinted.clear();
        diff
    }

    /// Settle one card; returns its one-line record for the
    /// transcript collapse, or `None` when the card is unknown.
    pub fn apply_settled(
        &mut self,
        session: &SessionId,
        request_id: &str,
        resolution: PendingResolution,
    ) -> Option<String> {
        self.hinted
            .retain(|(s, id, _)| !(s == session && id == request_id));
        let card = self
            .cards
            .get_mut(session)?
            .iter_mut()
            .find(|card| card.request_id() == request_id)?;
        card.phase = if matches!(resolution, PendingResolution::Expired) {
            Phase::Expired
        } else {
            Phase::Settled(resolution)
        };
        Some(card.settled_line())
    }

    /// Mark one card failed (decide/answer command failed). The card
    /// stays; Retry re-fetches. Returns whether the card was found.
    pub fn mark_failed(&mut self, session: &SessionId, request_id: &str, message: String) -> bool {
        match self
            .cards
            .get_mut(session)
            .and_then(|cards| cards.iter_mut().find(|c| c.request_id() == request_id))
        {
            Some(card) => {
                card.phase = Phase::Failed(message);
                true
            }
            None => false,
        }
    }

    /// Card for `request_id` owned by `session`, when a session is given.
    pub fn pending_card(
        &self,
        session: Option<&SessionId>,
        request_id: &str,
    ) -> Option<&PendingCardState> {
        let card = self.card(request_id)?;
        match session {
            Some(session) if card.session() != session => None,
            _ => Some(card),
        }
    }

    /// Interactive card element for one pending entry, when it is
    /// still pending for `session`.
    pub fn card_element<'a>(
        &'a self,
        session: Option<&SessionId>,
        request_id: &str,
    ) -> Option<Element<'a, Message>> {
        let card = self.pending_card(session, request_id)?;
        card.is_pending().then(|| card_view(card))
    }

    /// Reducer: folds one card interaction into state and reports
    /// the engine effect for the App to perform.
    pub fn update(&mut self, message: Message) -> Effect {
        // Deciding guard: interaction messages are no-ops unless the
        // card is awaiting. RetryFetch keeps its existing semantics.
        let interaction_request = match &message {
            Message::RetryFetch { .. } => None,
            Message::PagerNext { request }
            | Message::PagerPrev { request }
            | Message::SelectOption { request, .. }
            | Message::OtherTextChanged { request, .. }
            | Message::SkipQuestion { request }
            | Message::SubmitAnswers { request }
            | Message::Approve { request }
            | Message::DenyToggle { request }
            | Message::DenyReasonChanged { request, .. }
            | Message::ConfirmDeny { request } => Some(request.as_str()),
        };
        if let Some(request_id) = interaction_request
            && !matches!(
                self.card(request_id).map(|card| &card.phase),
                Some(Phase::Awaiting)
            )
        {
            return Effect::None;
        }
        match message {
            Message::PagerNext { request } => {
                if let Some(card) = self.card_mut(&request)
                    && let Some(questions) = card.request.questions()
                {
                    let last = questions.len().saturating_sub(1);
                    card.interaction.pager_index = (card.interaction.pager_index + 1).min(last);
                }
                Effect::None
            }
            Message::PagerPrev { request } => {
                if let Some(card) = self.card_mut(&request) {
                    card.interaction.pager_index = card.interaction.pager_index.saturating_sub(1);
                }
                Effect::None
            }
            Message::SelectOption {
                request,
                question,
                label,
            } => {
                if let Some(card) = self.card_mut(&request) {
                    card.interaction.skipped.remove(&question);
                    if label == OTHER_LABEL {
                        card.interaction.other_selected.insert(question.clone());
                        if !card_is_multi(card, &question) {
                            card.interaction.selections.remove(&question);
                        }
                    } else if card_is_multi(card, &question) {
                        card.interaction.other_selected.remove(&question);
                        let picked = card.interaction.selections.entry(question).or_default();
                        if let Some(at) = picked.iter().position(|p| p == &label) {
                            picked.remove(at);
                        } else {
                            picked.push(label);
                        }
                    } else {
                        card.interaction.other_selected.remove(&question);
                        card.interaction.selections.insert(question, vec![label]);
                    }
                }
                Effect::None
            }
            Message::OtherTextChanged {
                request,
                question,
                text,
            } => {
                if let Some(card) = self.card_mut(&request) {
                    card.interaction.other_selected.insert(question.clone());
                    card.interaction.other_text.insert(question, text);
                }
                Effect::None
            }
            Message::SkipQuestion { request } => {
                let Some(card) = self.card_mut(&request) else {
                    return Effect::None;
                };
                if !matches!(card.phase, Phase::Awaiting) {
                    return Effect::None;
                }
                let Some(current) = card.current_question().cloned() else {
                    return Effect::None;
                };
                card.interaction.skipped.insert(current.id.clone());
                card.interaction.selections.remove(&current.id);
                card.interaction.other_selected.remove(&current.id);
                let total = card.request.questions().map_or(0, <[Question]>::len);
                if total == 0 || card.interaction.pager_index + 1 < total {
                    card.interaction.pager_index += 1;
                    return Effect::None;
                }
                // Last question: Skip submits with that question skipped.
                let Some(answer) = card.assemble_answer() else {
                    return Effect::None;
                };
                let session = card.session().clone();
                let request_id = card.request_id().to_string();
                card.phase = Phase::Deciding;
                Effect::Answer {
                    session,
                    request_id,
                    answer,
                }
            }
            Message::SubmitAnswers { request } => {
                let Some(card) = self.card_mut(&request) else {
                    return Effect::None;
                };
                let total = card.request.questions().map_or(0, <[Question]>::len);
                if total == 0 || !matches!(card.phase, Phase::Awaiting) {
                    return Effect::None;
                }
                if card.interaction.pager_index + 1 < total {
                    card.interaction.pager_index += 1;
                    return Effect::None;
                }
                let Some(answer) = card.assemble_answer() else {
                    return Effect::None;
                };
                let session = card.session().clone();
                let request_id = card.request_id().to_string();
                card.phase = Phase::Deciding;
                Effect::Answer {
                    session,
                    request_id,
                    answer,
                }
            }
            Message::Approve { request } => {
                let Some(card) = self.card_mut(&request) else {
                    return Effect::None;
                };
                if !matches!(card.phase, Phase::Awaiting) {
                    return Effect::None;
                }
                let session = card.session().clone();
                let request_id = card.request_id().to_string();
                card.phase = Phase::Deciding;
                Effect::Decide {
                    session,
                    request_id,
                    decision: ApprovalDecision::Approve,
                    reason: None,
                }
            }
            Message::DenyToggle { request } => {
                if let Some(card) = self.card_mut(&request) {
                    card.interaction.deny_editor = match card.interaction.deny_editor {
                        DenyEditor::Hidden => DenyEditor::Editing,
                        DenyEditor::Editing => DenyEditor::Hidden,
                    };
                }
                Effect::None
            }
            Message::DenyReasonChanged { request, text } => {
                if let Some(card) = self.card_mut(&request) {
                    card.interaction.deny_reason = text;
                }
                Effect::None
            }
            Message::ConfirmDeny { request } => {
                let Some(card) = self.card_mut(&request) else {
                    return Effect::None;
                };
                if !matches!(card.phase, Phase::Awaiting) {
                    return Effect::None;
                }
                let reason = {
                    let trimmed = card.interaction.deny_reason.trim();
                    if trimmed.is_empty() {
                        None
                    } else {
                        Some(trimmed.to_string())
                    }
                };
                let session = card.session().clone();
                let request_id = card.request_id().to_string();
                card.phase = Phase::Deciding;
                Effect::Decide {
                    session,
                    request_id,
                    decision: ApprovalDecision::Deny,
                    reason,
                }
            }
            Message::RetryFetch { request } => {
                // Card stays in Failed until the fetch confirms truth.
                if self.card(&request).is_some() {
                    Effect::FetchPending
                } else {
                    Effect::None
                }
            }
        }
    }
}

/// Whether the named question on `card` allows several picks.
fn card_is_multi(card: &PendingCardState, question_id: &str) -> bool {
    card.request
        .questions()
        .and_then(|qs| qs.iter().find(|q| q.id == question_id))
        .is_some_and(|q| q.multi)
}

/// One card panel: header with pager chrome, selection rows, and
/// the footer actions. All styling lives in `kit`.
fn card_view<'a>(card: &'a PendingCardState) -> Element<'a, Message> {
    let request = card.request_id().to_string();
    let interactive = matches!(card.phase, Phase::Awaiting);
    let press = |msg: Message| if interactive { Some(msg) } else { None };
    let mut column = iced::widget::column![].spacing(SPACE_S).width(Length::Fill);
    match &card.request {
        PendingRequest::Approval { approval, .. } => {
            let title = if approval.tool.is_empty() {
                "Approval"
            } else {
                approval.tool.as_str()
            };
            column = column.push(
                iced::widget::text(title)
                    .size(FONT_SIZE_BODY)
                    .color(TEXT_PRIMARY),
            );
            if !approval.subject_line.is_empty() {
                column = column.push(kit::code_block(approval.subject_line.as_str()));
            }
            let mut row = iced::widget::row![].spacing(SPACE_S);
            row = row.push(kit::pill_button(
                "Approve",
                kit::ButtonKind::Primary,
                press(Message::Approve {
                    request: request.clone(),
                }),
            ));
            row = row.push(kit::pill_button(
                "Deny",
                kit::ButtonKind::DangerGhost,
                press(Message::DenyToggle {
                    request: request.clone(),
                }),
            ));
            column = column.push(row);
            if card.interaction.deny_editor == DenyEditor::Editing {
                column = column.push(kit::text_field(
                    "Reason (optional)",
                    &card.interaction.deny_reason,
                    {
                        let request = request.clone();
                        move |text| Message::DenyReasonChanged {
                            request: request.clone(),
                            text,
                        }
                    },
                ));
                column = column.push(kit::pill_button(
                    "Confirm deny",
                    kit::ButtonKind::Danger,
                    press(Message::ConfirmDeny {
                        request: request.clone(),
                    }),
                ));
            }
        }
        PendingRequest::Question { question, .. } => {
            let total = question.questions.len();
            let index = card.interaction.pager_index.min(total.saturating_sub(1));
            if let Some(current) = question.questions.get(index) {
                // Header: prompt fills, pager chrome only when >1 question.
                let mut header = iced::widget::row![
                    iced::widget::container(
                        iced::widget::text(current.prompt.as_str())
                            .size(FONT_SIZE_BODY)
                            .color(TEXT_PRIMARY)
                    )
                    .width(Length::Fill)
                ]
                .spacing(SPACE_S)
                .align_y(iced::alignment::Vertical::Center)
                .width(Length::Fill);
                if kit::pager_visible(total) {
                    header = header.push(kit::icon_button(
                        lucide_icons::Icon::ChevronLeft,
                        kit::IconButtonKind::Subtle,
                        (index > 0 && interactive).then_some(Message::PagerPrev {
                            request: request.clone(),
                        }),
                    ));
                    header = header.push(
                        iced::widget::text(kit::pager_label(index, total))
                            .size(FONT_SIZE_SMALL)
                            .color(TEXT_SECONDARY),
                    );
                    header = header.push(kit::icon_button(
                        lucide_icons::Icon::ChevronRight,
                        kit::IconButtonKind::Subtle,
                        (index + 1 < total && interactive).then_some(Message::PagerNext {
                            request: request.clone(),
                        }),
                    ));
                }
                column = column.push(header);
                for option in &current.options {
                    column = column.push(kit::selection_row(
                        option.label.as_str(),
                        option.description.as_deref(),
                        card.is_selected(&current.id, &option.label),
                        current.multi,
                        Message::SelectOption {
                            request: request.clone(),
                            question: current.id.clone(),
                            label: option.label.clone(),
                        },
                    ));
                }
                // Synthetic Other: shell-added, never in the payload.
                column = column.push(kit::selection_row(
                    OTHER_LABEL,
                    None,
                    card.other_field_visible(&current.id),
                    current.multi,
                    Message::SelectOption {
                        request: request.clone(),
                        question: current.id.clone(),
                        label: String::from(OTHER_LABEL),
                    },
                ));
                if card.other_field_visible(&current.id) {
                    column = column.push(kit::text_field(
                        "Type your own answer here",
                        card.interaction
                            .other_text
                            .get(&current.id)
                            .map(String::as_str)
                            .unwrap_or(""),
                        {
                            let request = request.clone();
                            let question_id = current.id.clone();
                            move |text| Message::OtherTextChanged {
                                request: request.clone(),
                                question: question_id.clone(),
                                text,
                            }
                        },
                    ));
                }
                let mut row = iced::widget::row![].spacing(SPACE_S);
                row = row.push(kit::pill_button(
                    "Skip",
                    kit::ButtonKind::Ghost,
                    press(Message::SkipQuestion {
                        request: request.clone(),
                    }),
                ));
                row = row.push(kit::pill_button(
                    card.next_label(),
                    kit::ButtonKind::Primary,
                    press(Message::SubmitAnswers {
                        request: request.clone(),
                    }),
                ));
                column = column.push(row);
            }
        }
    }
    if let Phase::Failed(message) = &card.phase {
        let mut row = iced::widget::row![].spacing(SPACE_S);
        row = row.push(
            iced::widget::text(message.as_str())
                .size(FONT_SIZE_SMALL)
                .color(DANGER),
        );
        row = row.push(kit::pill_button(
            "Retry",
            kit::ButtonKind::Ghost,
            Some(Message::RetryFetch { request }),
        ));
        column = column.push(row);
    }
    kit::panel(column)
}

#[cfg(test)]
mod tests {
    use super::*;
    use manifest_domain::{ApprovalRequest, Question, QuestionOption, QuestionRequest};

    fn approval(session: &str, id: &str) -> PendingRequest {
        PendingRequest::Approval {
            session: SessionId::new(session),
            request_id: String::from(id),
            approval: ApprovalRequest {
                tool: String::from("bash"),
                subject_line: String::from("ls"),
                choices: Vec::new(),
            },
        }
    }

    fn questions(session: &str, id: &str) -> PendingRequest {
        PendingRequest::Question {
            session: SessionId::new(session),
            request_id: String::from(id),
            question: QuestionRequest {
                questions: vec![
                    Question {
                        id: String::from("q1"),
                        prompt: String::from("Colour?"),
                        options: vec![
                            QuestionOption {
                                label: String::from("Red"),
                                description: Some(String::from("warm")),
                            },
                            QuestionOption {
                                label: String::from("None"),
                                description: None,
                            },
                        ],
                        multi: false,
                    },
                    Question {
                        id: String::from("q2"),
                        prompt: String::from("Toppings?"),
                        options: vec![
                            QuestionOption {
                                label: String::from("Cheese"),
                                description: None,
                            },
                            QuestionOption {
                                label: String::from("Olives"),
                                description: None,
                            },
                        ],
                        multi: true,
                    },
                    Question {
                        id: String::from("q3"),
                        prompt: String::from("File?"),
                        options: vec![QuestionOption {
                            label: String::from("alpha"),
                            description: None,
                        }],
                        multi: false,
                    },
                ],
            },
        }
    }

    #[test]
    fn loaded_inserts_and_counts_background() {
        let mut state = State::default();
        let diff = state.apply_loaded(vec![approval("s-bg", "r1"), questions("s-bg", "r2")]);
        assert_eq!(diff.inserted.len(), 2);
        assert_eq!(state.pending_count(&SessionId::new("s-bg")), 2);
        assert_eq!(state.pending_count(&SessionId::new("s-other")), 0);
        assert!(state.has_awaiting(&SessionId::new("s-bg")));
    }

    #[test]
    fn vanish_unsettled_becomes_expired_record() {
        let mut state = State::default();
        state.apply_loaded(vec![approval("s-1", "r1")]);
        let diff = state.apply_loaded(Vec::new());
        assert_eq!(diff.expired.len(), 1);
        assert!(diff.expired[0].2.contains("Expired"));
        assert_eq!(state.pending_count(&SessionId::new("s-1")), 0);
        assert!(!state.has_awaiting(&SessionId::new("s-1")));
    }

    #[test]
    fn approve_emits_decide_and_deciding_phase() {
        let mut state = State::default();
        state.apply_loaded(vec![approval("s-1", "r1")]);
        let effect = state.update(Message::Approve {
            request: String::from("r1"),
        });
        assert_eq!(
            effect,
            Effect::Decide {
                session: SessionId::new("s-1"),
                request_id: String::from("r1"),
                decision: ApprovalDecision::Approve,
                reason: None,
            }
        );
        assert_eq!(state.card("r1").unwrap().phase, Phase::Deciding);
    }

    #[test]
    fn deny_reason_flows_to_decide() {
        let mut state = State::default();
        state.apply_loaded(vec![approval("s-1", "r1")]);
        state.update(Message::DenyToggle {
            request: String::from("r1"),
        });
        state.update(Message::DenyReasonChanged {
            request: String::from("r1"),
            text: String::from("  too risky "),
        });
        let effect = state.update(Message::ConfirmDeny {
            request: String::from("r1"),
        });
        assert!(matches!(
            effect,
            Effect::Decide {
                decision: ApprovalDecision::Deny,
                reason: Some(ref r),
                ..
            } if r == "too risky"
        ));
    }

    #[test]
    fn answer_assembles_label_other_and_skip_on_last() {
        let mut state = State::default();
        state.apply_loaded(vec![questions("s-1", "r2")]);
        // q1: real None label (plain label, no Other textbox).
        state.update(Message::SelectOption {
            request: String::from("r2"),
            question: String::from("q1"),
            label: String::from("None"),
        });
        // Submit before last only advances the pager.
        assert_eq!(
            state.update(Message::SubmitAnswers {
                request: String::from("r2")
            }),
            Effect::None
        );
        assert_eq!(state.card("r2").unwrap().interaction.pager_index, 1);
        // q2: multi picks.
        state.update(Message::SelectOption {
            request: String::from("r2"),
            question: String::from("q2"),
            label: String::from("Cheese"),
        });
        state.update(Message::PagerNext {
            request: String::from("r2"),
        });
        // q3: synthetic Other with free text.
        state.update(Message::SelectOption {
            request: String::from("r2"),
            question: String::from("q3"),
            label: String::from(OTHER_LABEL),
        });
        state.update(Message::OtherTextChanged {
            request: String::from("r2"),
            question: String::from("q3"),
            text: String::from("gamma"),
        });
        let effect = state.update(Message::SubmitAnswers {
            request: String::from("r2"),
        });
        match effect {
            Effect::Answer { answer, .. } => {
                assert_eq!(
                    answer.get("q1"),
                    Some(&AnswerValue::Label(String::from("None")))
                );
                assert_eq!(
                    answer.get("q2"),
                    Some(&AnswerValue::Labels(vec![String::from("Cheese")]))
                );
                assert_eq!(
                    answer.get("q3"),
                    Some(&AnswerValue::FreeText(String::from("gamma")))
                );
            }
            other => panic!("expected Answer, got {other:?}"),
        }
        assert_eq!(state.card("r2").unwrap().phase, Phase::Deciding);
    }

    #[test]
    fn skip_encodes_skipped_not_empty_free_text() {
        let mut state = State::default();
        state.apply_loaded(vec![questions("s-1", "r2")]);
        state.update(Message::SkipQuestion {
            request: String::from("r2"),
        });
        assert_eq!(state.card("r2").unwrap().interaction.pager_index, 1);
        state.update(Message::PagerNext {
            request: String::from("r2"),
        });
        state.update(Message::PagerNext {
            request: String::from("r2"),
        });
        // Still on last (pager clamps); answer q3 then submit all.
        state.update(Message::SelectOption {
            request: String::from("r2"),
            question: String::from("q3"),
            label: String::from("alpha"),
        });
        match state.update(Message::SubmitAnswers {
            request: String::from("r2"),
        }) {
            Effect::Answer { answer, .. } => {
                assert_eq!(answer.get("q1"), Some(&AnswerValue::Skipped));
            }
            other => panic!("expected Answer, got {other:?}"),
        }
    }

    #[test]
    fn settled_and_failed_phases_and_retry() {
        let mut state = State::default();
        state.apply_loaded(vec![approval("s-1", "r1")]);
        let line = state.apply_settled(&SessionId::new("s-1"), "r1", PendingResolution::Approved);
        assert_eq!(line.as_deref(), Some("Approval · ls — Approved by you"));
        assert_eq!(state.pending_count(&SessionId::new("s-1")), 0);

        state.apply_loaded(vec![approval("s-1", "r9")]);
        assert!(state.mark_failed(&SessionId::new("s-1"), "r9", String::from("boom")));
        assert!(matches!(
            state.card("r9").unwrap().phase,
            Phase::Failed(ref m) if m == "boom"
        ));
        assert_eq!(
            state.update(Message::RetryFetch {
                request: String::from("r9")
            }),
            Effect::FetchPending
        );
        // Card stays failed until the fetch returns; then it revives.
        state.apply_loaded(vec![approval("s-1", "r9")]);
        assert_eq!(state.card("r9").unwrap().phase, Phase::Awaiting);
    }

    #[test]
    fn other_field_visible_only_when_other_selected() {
        let mut state = State::default();
        state.apply_loaded(vec![questions("s-1", "r2")]);
        assert!(!state.card("r2").unwrap().other_field_visible("q1"));
        // Real `None` option does not open the box.
        state.update(Message::SelectOption {
            request: String::from("r2"),
            question: String::from("q1"),
            label: String::from("None"),
        });
        assert!(!state.card("r2").unwrap().other_field_visible("q1"));
        state.update(Message::SelectOption {
            request: String::from("r2"),
            question: String::from("q1"),
            label: String::from(OTHER_LABEL),
        });
        assert!(state.card("r2").unwrap().other_field_visible("q1"));
        // Picking a real option again hides it.
        state.update(Message::SelectOption {
            request: String::from("r2"),
            question: String::from("q1"),
            label: String::from("Red"),
        });
        assert!(!state.card("r2").unwrap().other_field_visible("q1"));
    }

    #[test]
    fn skip_on_last_question_submits_with_skip() {
        let mut state = State::default();
        state.apply_loaded(vec![questions("s-1", "r2")]);
        state.update(Message::PagerNext {
            request: String::from("r2"),
        });
        state.update(Message::PagerNext {
            request: String::from("r2"),
        });
        assert_eq!(state.card("r2").unwrap().next_label(), "Submit");
        match state.update(Message::SkipQuestion {
            request: String::from("r2"),
        }) {
            Effect::Answer { answer, .. } => {
                assert_eq!(answer.get("q3"), Some(&AnswerValue::Skipped));
            }
            other => panic!("expected Answer, got {other:?}"),
        }
        assert_eq!(state.card("r2").unwrap().phase, Phase::Deciding);
    }

    #[test]
    fn pending_counts_map_only_nonzero() {
        let mut state = State::default();
        state.apply_loaded(vec![approval("s-bg", "r1"), questions("s-bg", "r2")]);
        let counts = state.pending_counts();
        assert_eq!(counts.get(&SessionId::new("s-bg")), Some(&2));
        assert_eq!(counts.get(&SessionId::new("s-other")), None);
        assert_eq!(
            state
                .first_awaiting_question(&SessionId::new("s-bg"))
                .as_deref(),
            Some("r2")
        );
    }

    #[test]
    fn interactions_are_noops_unless_awaiting() {
        let mut state = State::default();
        state.apply_loaded(vec![approval("s-1", "r1"), questions("s-1", "r2")]);
        // Move r1 to Deciding.
        assert!(matches!(
            state.update(Message::Approve {
                request: String::from("r1")
            }),
            Effect::Decide { .. }
        ));
        assert_eq!(state.card("r1").unwrap().phase, Phase::Deciding);
        // Every interaction on the deciding card is a no-op.
        assert_eq!(
            state.update(Message::DenyToggle {
                request: String::from("r1")
            }),
            Effect::None
        );
        assert_eq!(
            state.card("r1").unwrap().interaction.deny_editor,
            DenyEditor::Hidden
        );
        assert_eq!(
            state.update(Message::DenyReasonChanged {
                request: String::from("r1"),
                text: String::from("no")
            }),
            Effect::None
        );
        assert!(state.card("r1").unwrap().interaction.deny_reason.is_empty());
        assert_eq!(
            state.update(Message::ConfirmDeny {
                request: String::from("r1")
            }),
            Effect::None
        );
        assert_eq!(
            state.update(Message::Approve {
                request: String::from("r1")
            }),
            Effect::None
        );
        assert_eq!(state.card("r1").unwrap().phase, Phase::Deciding);
        // Failed cards also reject interaction; only RetryFetch acts.
        assert!(state.mark_failed(&SessionId::new("s-1"), "r2", String::from("boom")));
        assert_eq!(
            state.update(Message::PagerNext {
                request: String::from("r2")
            }),
            Effect::None
        );
        assert_eq!(state.card("r2").unwrap().interaction.pager_index, 0);
        assert_eq!(
            state.update(Message::SelectOption {
                request: String::from("r2"),
                question: String::from("q1"),
                label: String::from("Red")
            }),
            Effect::None
        );
        assert_eq!(
            state.update(Message::OtherTextChanged {
                request: String::from("r2"),
                question: String::from("q1"),
                text: String::from("x")
            }),
            Effect::None
        );
        assert_eq!(
            state.update(Message::SkipQuestion {
                request: String::from("r2")
            }),
            Effect::None
        );
        assert_eq!(
            state.update(Message::SubmitAnswers {
                request: String::from("r2")
            }),
            Effect::None
        );
        assert_eq!(
            state.update(Message::RetryFetch {
                request: String::from("r2")
            }),
            Effect::FetchPending
        );
        // Settled cards reject interaction too.
        let _ = state.apply_settled(&SessionId::new("s-1"), "r1", PendingResolution::Approved);
        assert_eq!(
            state.update(Message::PagerPrev {
                request: String::from("r1")
            }),
            Effect::None
        );
    }

    #[test]
    fn hint_failed_materializes_placeholder_and_load_clears_hint() {
        let mut state = State::default();
        state.record_hint(
            SessionId::new("s-1"),
            String::from("r1"),
            PendingKind::UserInput,
        );
        // Settled-style duplicate hint for the same request dedupes.
        state.record_hint(
            SessionId::new("s-1"),
            String::from("r1"),
            PendingKind::UserInput,
        );
        assert_eq!(state.hinted_requests().len(), 1);
        assert!(state.mark_hint_failed(
            &SessionId::new("s-1"),
            "r1",
            PendingKind::UserInput,
            String::from("write EPIPE"),
        ));
        let card = state.card("r1").unwrap();
        assert!(matches!(
            card.phase,
            Phase::Failed(ref m) if m == "write EPIPE"
        ));
        assert_eq!(card.request.questions(), Some(&[][..]));
        assert!(card.is_pending());
        assert!(!state.has_awaiting(&SessionId::new("s-1")));
        // The Failed card still renders an element (named error + Retry).
        assert!(
            state
                .card_element(Some(&SessionId::new("s-1")), "r1")
                .is_some()
        );
        // A second failure only refreshes the message, no new card.
        assert!(!state.mark_hint_failed(
            &SessionId::new("s-1"),
            "r1",
            PendingKind::UserInput,
            String::from("write EPIPE"),
        ));
        assert_eq!(state.cards_for(&SessionId::new("s-1")).len(), 1);
        // A successful fetch with the request absent clears the hint
        // without inserting a card; the placeholder expires instead.
        state.record_hint(
            SessionId::new("s-1"),
            String::from("r2"),
            PendingKind::Approval,
        );
        let diff = state.apply_loaded(Vec::new());
        assert!(diff.inserted.is_empty());
        assert!(state.hinted_requests().is_empty());
        assert_eq!(
            state.card("r1").unwrap().phase,
            Phase::Expired,
            "absent placeholder expires, never silently drops"
        );
    }

    #[test]
    fn pending_card_lookup_scopes_to_session() {
        let mut state = State::default();
        state.apply_loaded(vec![approval("s-1", "r1")]);
        assert!(
            state
                .pending_card(Some(&SessionId::new("s-1")), "r1")
                .is_some()
        );
        assert!(
            state
                .pending_card(Some(&SessionId::new("s-2")), "r1")
                .is_none()
        );
        assert!(state.pending_card(None, "r1").is_some());
        assert!(
            state
                .card_element(Some(&SessionId::new("s-1")), "r1")
                .is_some()
        );
        assert!(
            state
                .card_element(Some(&SessionId::new("s-2")), "r1")
                .is_none()
        );
    }

    #[test]
    fn present_on_reload_preserves_pager() {
        let mut state = State::default();
        state.apply_loaded(vec![questions("s-1", "r2")]);
        state.update(Message::PagerNext {
            request: String::from("r2"),
        });
        let diff = state.apply_loaded(vec![questions("s-1", "r2")]);
        assert!(diff.inserted.is_empty());
        assert_eq!(state.card("r2").unwrap().interaction.pager_index, 1);
    }
}
