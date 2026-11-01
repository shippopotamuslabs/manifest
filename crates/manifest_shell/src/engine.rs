//! Subscription adapter between `manifest_mcp`'s blocking API and Iced.
//!
//! The data crate is blocking and thread-based: [`manifest_mcp::connect`]
//! spawns a worker thread and reports through a
//! [`std::sync::mpsc`] channel. Iced subscriptions are async
//! [`futures::Stream`] values, so this module bridges the two: a
//! forwarder thread moves items from the blocking channel into an
//! unbounded async channel the subscription stream reads from.

use std::sync::Arc;

use iced::futures::StreamExt;
use manifest_domain::{EngineEvent, SessionEngine};

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
    /// Wraps a shared engine in a cloneable handle.
    pub fn new(engine: Arc<dyn SessionEngine>) -> Self {
        Self { inner: engine }
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
            let handle = EngineHandle::new(Arc::new(engine) as Arc<dyn SessionEngine>);
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
