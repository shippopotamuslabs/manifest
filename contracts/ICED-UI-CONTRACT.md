# ICED-UI-CONTRACT.md — the Iced UI law for Manifest

The gospel suite has no Iced contract, so this file is the binding
UI contract for this repo. It sits under the gospels pinned at
`contracts/gospel/`: where this contract and a gospel both speak,
the gospel wins. This contract governs the Iced surface only.

## 1. The model is the app

Manifest is built on the Elm architecture Iced provides: one
state tree, one `Message` enum, `update()` folds a message into
state, `view()` renders state.

- Enumerate the messages a feature can receive before writing its
  view. The message list is the feature's real specification.
- Model state as enums that name the actual situations:
  `Load<T> { Idle, Loading, Ready(T), Failed(String) }`,
  `Connection { Disconnected, Connecting, Connected }`. A tuple of
  booleans and options that can describe impossible situations is
  a contract violation; fix the state, never wrestle the view.
- `view()` is a pure projection of state. If a view is hard to
  write, the state model is wrong.

## 2. `update()` and `view()` do no work

- `update()` returns promptly. I/O, process work, and anything
  that can block runs as a `Task` that completes into a message.
- `view()` formats nothing, sorts nothing, parses nothing, and
  clones no large buffers. Derived data is computed once in
  `update()` and stored in state, ready for the view to read.
- The view borrows state. Rendering reads `&self`; it never takes
  ownership of the data it draws.

## 3. Streams enter through subscriptions

- Every external event source, the MCP notification stream
  included, enters the app as a `Subscription` that yields
  messages. Widgets never touch a socket, a channel, or a child
  process directly.
- Connection lifecycle lives in state as an enum (see §1). A
  dropped connection is a state the model names and the view
  renders, with reconnect as a message like any other.
- High-frequency input is batched before it reaches the model.
  Text deltas accumulate in state and land as messages at a
  steady cadence; the muse-code-mcp stream already coalesces at
  30 ms, and the client keeps that discipline on its side of the
  wire.
- Only the live item re-renders while it streams. A completed
  item is parsed and laid out once, and its rendered form is
  cached in state; completed history costs the view a lookup.

## 4. Layout has bounds

- Text panes carry explicit widths. Text inside a scrollable with
  unbounded width will not wrap, and most Iced layout bugs trace
  to an unbounded dimension somewhere in the tree.
- Panes size with `Fill` and `FillPortion` ratios: sidebar and
  main surface split by proportion, and leaf widgets state their
  sizes where proportion cannot express them.

## 5. One theme, one widget kit

- Colors, spacing, and typography are defined once, in the theme
  module. A second definition of the same value is a violation.
- The app builds a small kit of its own widgets (buttons, panels,
  inputs, list rows) on top of Iced's, and screens compose from
  the kit. Style closures sprinkled at call sites are a violation:
  a visual decision is made once, in the kit, and reused.

## 6. Performance is judged in release builds

- Iced in a debug build is slow, text shaping most of all. That
  is a property of the framework, and it is never evidence about
  this app's performance. Feel and measure the app in release.
- A performance claim in a review, a plan, or a commit message
  names the build profile it was measured in.

## 7. Features compose as modules

- A screen or feature owns its state, its `Message` enum, its
  `update()`, and its `view()`. The parent holds the child state,
  routes messages down, and maps child messages up.
- Crate layering follows
  `contracts/gospel/RUST-ARCHITECTURE-GOSPEL.md`: presentation
  renders and routes, domain holds the session model, data owns
  the wire and the disk. The MCP client is a data-layer adapter;
  its types stop at that layer.

## 8. The reducer is the test surface

- `update()` is a reducer, and reducers test headless: drive the
  state with messages and assert the resulting state, per
  `contracts/gospel/TESTING-GOSPEL.md`. UI logic that can only be
  verified by opening a window is a design smell against §1.
