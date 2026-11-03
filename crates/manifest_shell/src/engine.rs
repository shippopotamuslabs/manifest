//! Engine plumbing: subscription adapter plus engine reduction/view helpers.
//!
//! The data crate is blocking and thread-based: [`manifest_mcp::connect`]
//! spawns a worker thread and reports through a
//! [`std::sync::mpsc`] channel. Iced subscriptions are async
//! [`futures::Stream`] values, so this module bridges the two: a
//! forwarder thread moves items from the blocking channel into an
//! unbounded async channel the subscription stream reads from.
//!
//! The root [`App`](crate::app::App) stays thin: it guards on the
//! connection attempt, then uses the subscription gating
//! ([`subscription_for`]) and the connection view helpers
//! ([`connection_label`], [`connection_cluster`]) here.

use std::sync::Arc;

use iced::futures::StreamExt;
use manifest_domain::{Connection, EngineEvent, SessionEngine, SessionId, SpawnChoices, StreamEvent};

/// Folds `next` into `into` when both are item deltas for the same
/// session, item, and field: the delta texts concatenate and the first
/// event's other fields (turn, cursor) are kept. Returns whether the
/// merge happened; a refusal leaves both events untouched so the caller
/// can emit `into` and retry from `next`, preserving order.
fn try_merge_delta(into: &mut EngineEvent, next: &EngineEvent) -> bool {
    match (into, next) {
        (
            EngineEvent::Stream(StreamEvent::ItemDelta {
                session: into_session,
                item_id: into_item,
                field: into_field,
                delta: into_delta,
                ..
            }),
            EngineEvent::Stream(StreamEvent::ItemDelta {
                session: next_session,
                item_id: next_item,
                field: next_field,
                delta: next_delta,
                ..
            }),
        ) if into_session == next_session && into_item == next_item && into_field == next_field => {
            into_delta.push_str(next_delta);
            true
        }
        _ => false,
    }
}

/// Cloneable handle to the session engine behind the subscription.
///
/// The inner trait object is shared, so clones are cheap; dropping every
/// clone (in practice the one [`App`](crate::app::App) holds) lets the
/// worker shut down and kills the child server process.
#[derive(Clone)]
pub struct EngineHandle {
    /// Shared engine; commands send through it without I/O on the caller.
    inner: Arc<dyn SessionEngine>,
}

impl EngineHandle {
    /// Wraps an engine in a cloneable handle; the sharing is internal.
    pub fn new(engine: impl SessionEngine + 'static) -> Self {
        Self {
            inner: Arc::new(engine),
        }
    }

    /// Requests a session-list refresh; results arrive as engine events.
    pub fn list_sessions(&self) {
        self.inner.list_sessions();
    }

    /// Spawns a session; success arrives as `SessionSpawned`.
    pub fn spawn_session(&self, workspace: String, prompt: String, choices: SpawnChoices) {
        self.inner.spawn_session(workspace, prompt, choices);
    }

    /// Sends a follow-up turn; success arrives as `TurnAccepted`.
    pub fn send_turn(&self, session: SessionId, prompt: String) {
        self.inner.send_turn(session, prompt);
    }

    /// Resumes a session; success arrives as `SessionResumed`.
    pub fn resume_session(&self, session: SessionId) {
        self.inner.resume_session(session);
    }
}

impl std::fmt::Debug for EngineHandle {
    /// Prints without requiring `Debug` on the trait object.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "EngineHandle(..)")
    }
}

/// One item from the engine stream: either the handle (sent first, so the
/// app holds it before any event can ask for a command) or one event.
#[derive(Debug, Clone)]
pub enum Input {
    /// The engine handle; the forwarder sends this before any event.
    Ready(EngineHandle),
    /// One engine event from the worker, boxed so the hot app message
    /// enum stays small.
    Event(Box<EngineEvent>),
}

impl Input {
    /// Wraps one worker event for the subscription stream.
    pub fn event(event: EngineEvent) -> Self {
        Self::Event(Box::new(event))
    }
}

/// Gates the engine stream: it runs only while connecting or connected
/// with a config. A finished stream whose recipe is still returned gets
/// restarted, which would respawn against a dead server in a loop, so
/// failure and fixture mode return no engine recipe until the user
/// retries.
pub fn subscription_for(
    attempt: u64,
    config: Option<&manifest_mcp::ServerConfig>,
    connection: &Connection,
) -> iced::Subscription<(u64, Input)> {
    match (config, connection) {
        (Some(config), Connection::Connecting | Connection::Connected) => {
            subscription(attempt, config)
        }
        _ => iced::Subscription::none(),
    }
}

/// Subscribes to the engine described by `config`, tagging every item with
/// `attempt` so the app can ignore streams from superseded reconnects.
///
/// The subscription data is the `(attempt, command, args, envs)` tuple (all
/// [`std::hash::Hash`]), so one recipe identifies one connection attempt.
/// The builder spawns a single forwarder thread that connects through
/// [`manifest_mcp::connect`], moves the sole [`EngineHandle`] to the app
/// (the forwarder keeps no clone, so dropping the app's handle shuts the
/// worker down), then forwards events until the subscription goes away.
///
/// Ordering guarantee: the forwarder sends [`Input::Ready`] before
/// forwarding any event, so the app holds the handle before
/// [`Connection::Connected`](manifest_domain::Connection::Connected) can
/// arrive and ask for a session list.
pub fn subscription(
    attempt: u64,
    config: &manifest_mcp::ServerConfig,
) -> iced::Subscription<(u64, Input)> {
    let data = (
        attempt,
        config.command.clone(),
        config.args.clone(),
        config.envs.clone(),
    );
    iced::Subscription::run_with(data, |data| {
        let (attempt, command, args, envs) =
            (data.0, data.1.clone(), data.2.clone(), data.3.clone());
        let (tx, rx) = iced::futures::channel::mpsc::unbounded();
        std::thread::spawn(move || {
            let (std_tx, std_rx) = std::sync::mpsc::channel();
            let engine = manifest_mcp::connect(
                manifest_mcp::ServerConfig {
                    command,
                    args,
                    envs,
                },
                std_tx,
            );
            let handle = EngineHandle::new(engine);
            if tx.unbounded_send(Input::Ready(handle)).is_err() {
                return;
            }
            // A disconnected channel ends the loop; an empty channel ends
            // the current coalescing pass, keeping the discipline of one
            // UI message per burst without timers (UI contract §3).
            let mut stash: Option<EngineEvent> = None;
            loop {
                let first = match stash.take() {
                    Some(event) => event,
                    None => match std_rx.recv() {
                        Ok(event) => event,
                        Err(_) => break,
                    },
                };
                let mut event = first;
                if matches!(event, EngineEvent::Stream(StreamEvent::ItemDelta { .. })) {
                    while let Ok(next) = std_rx.try_recv() {
                        if !try_merge_delta(&mut event, &next) {
                            stash = Some(next);
                            break;
                        }
                    }
                }
                if tx.unbounded_send(Input::event(event)).is_err() {
                    break;
                }
            }
        });
        rx.map(move |input| (attempt, input))
    })
}

/// Static label for the connection state. The view performs no
/// formatting: this match returns a static label, keeping string
/// building out of the view (UI contract §2).
pub fn connection_label(connection: &Connection) -> &'static str {
    match connection {
        Connection::Disconnected => "Offline",
        Connection::Connecting => "Connecting...",
        Connection::Connected => "Connected",
        Connection::Failed(_) => "Connection failed",
    }
}

/// The live-mode connection cluster: the connection label plus a Retry
/// button while failed. A plain helper on the connection state — no
/// state of its own.
pub fn connection_cluster<'a, M: Clone + 'a>(
    connection: &Connection,
    on_reconnect: M,
) -> iced::Element<'a, M> {
    let mut cluster = iced::widget::row![
        iced::widget::text(connection_label(connection))
            .size(crate::theme::FONT_SIZE_SMALL)
            .color(crate::theme::TEXT_TERTIARY),
    ];
    if matches!(connection, Connection::Failed(_)) {
        cluster = cluster.push(crate::kit::pill_button(
            "Retry",
            crate::kit::ButtonKind::Ghost,
            Some(on_reconnect),
        ));
    }
    cluster
        .spacing(crate::theme::SPACE_S)
        .align_y(iced::alignment::Vertical::Center)
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn delta(session: &str, item: &str, field: &str, text: &str) -> EngineEvent {
        EngineEvent::Stream(StreamEvent::ItemDelta {
            session: SessionId::new(session),
            turn: None,
            view_cursor: Some(String::from("cursor-first")),
            item_id: String::from(item),
            field: String::from(field),
            delta: String::from(text),
        })
    }

    #[test]
    fn merges_same_item_and_field_concatenating_text() {
        let mut into = delta("s-1", "a-1", "text", "foo");
        let next = delta("s-1", "a-1", "text", "bar");
        assert!(try_merge_delta(&mut into, &next));
        assert!(matches!(
            &into,
            EngineEvent::Stream(StreamEvent::ItemDelta { delta, view_cursor, .. })
                if delta == "foobar" && view_cursor.as_deref() == Some("cursor-first")
        ));
    }

    #[test]
    fn refuses_different_field_item_or_session() {
        let mut into = delta("s-1", "a-1", "text", "foo");
        assert!(!try_merge_delta(
            &mut into,
            &delta("s-1", "a-1", "output", "bar")
        ));
        assert!(!try_merge_delta(
            &mut into,
            &delta("s-1", "a-2", "text", "bar")
        ));
        assert!(!try_merge_delta(
            &mut into,
            &delta("s-2", "a-1", "text", "bar")
        ));
        // A refusal leaves the first event untouched.
        assert!(matches!(
            &into,
            EngineEvent::Stream(StreamEvent::ItemDelta { delta, .. }) if delta == "foo"
        ));
    }

    #[test]
    fn refuses_non_delta_events() {
        let mut into = delta("s-1", "a-1", "text", "foo");
        let other = EngineEvent::Diagnostic(String::from("note"));
        assert!(!try_merge_delta(&mut into, &other));
        assert!(!try_merge_delta(
            &mut EngineEvent::Diagnostic(String::from("note")),
            &into
        ));
    }

    /// Records engine commands: the handle must forward without I/O.
    /// Shared so clones observe the same calls.
    #[derive(Clone)]
    struct RecordingEngine {
        resumed: std::sync::Arc<std::sync::Mutex<Vec<SessionId>>>,
    }

    impl SessionEngine for RecordingEngine {
        fn list_sessions(&self) {}

        fn spawn_session(
            &self,
            _workspace: String,
            _prompt: String,
            _choices: manifest_domain::SpawnChoices,
        ) {
        }

        fn send_turn(&self, _session: SessionId, _prompt: String) {}

        fn resume_session(&self, session: SessionId) {
            self.resumed.lock().unwrap().push(session);
        }
    }

    #[test]
    fn resume_passthrough_forwards_session() {
        let engine = RecordingEngine {
            resumed: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
        };
        EngineHandle::new(engine.clone()).resume_session(SessionId::new("s-1"));
        assert_eq!(*engine.resumed.lock().unwrap(), vec![SessionId::new("s-1")]);
    }
}
