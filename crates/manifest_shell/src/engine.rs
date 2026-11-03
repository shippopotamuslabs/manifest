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
//! connection attempt, then delegates the engine reduction
//! ([`apply`]), the subscription gating ([`subscription_for`]), and
//! the connection view helpers ([`connection_label`],
//! [`connection_cluster`]) here.

use std::sync::Arc;

use iced::futures::StreamExt;
use manifest_domain::{Connection, EngineEvent, SessionEngine};

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
    /// One engine event from the worker.
    Event(EngineEvent),
}

/// Folds one engine input into the held engine/connection state.
///
/// Returns whether an error note was appended (the caller maps that to
/// a scroll-to-end task). `Ready` stores the handle;
/// `ConnectionChanged(Failed)` pushes an error note then sets the
/// connection; `Connected` triggers `list_sessions` on the held handle;
/// `Failed` clears the handle; `SessionsLoaded` replaces the session
/// list; `SessionsLoadFailed`/`Diagnostic` push error notes;
/// `Stream(Pending)` refreshes the list only while connected with a
/// handle.
pub fn apply(
    connection: &mut Connection,
    engine: &mut Option<EngineHandle>,
    sessions: &mut crate::sessions::State,
    transcript: &mut crate::transcript::State,
    input: Input,
) -> bool {
    let mut appended = false;
    match input {
        Input::Ready(handle) => {
            *engine = Some(handle);
        }
        Input::Event(EngineEvent::ConnectionChanged(next)) => {
            if let Connection::Failed(message) = &next {
                appended = transcript.push_error(message.clone());
            }
            *connection = next;
            if *connection == Connection::Connected {
                if let Some(engine) = &*engine {
                    engine.list_sessions();
                }
            } else if matches!(*connection, Connection::Failed(_)) {
                *engine = None;
            }
        }
        Input::Event(EngineEvent::SessionsLoaded {
            projects,
            sessions: loaded,
        }) => {
            sessions.replace(projects, loaded);
        }
        Input::Event(EngineEvent::SessionsLoadFailed(message)) => {
            appended = transcript.push_error(message);
        }
        Input::Event(EngineEvent::Diagnostic(message)) => {
            appended = transcript.push_error(message);
        }
        Input::Event(EngineEvent::Stream(manifest_domain::StreamEvent::Pending { .. })) => {
            if *connection == Connection::Connected
                && let Some(engine) = &*engine
            {
                engine.list_sessions();
            }
        }
        // M2 owns no transcript: other stream events are
        // consumed as no-ops until M3 renders them.
        Input::Event(EngineEvent::Stream(_)) => {}
    }
    appended
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
            for event in std_rx {
                if tx.unbounded_send(Input::Event(event)).is_err() {
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
