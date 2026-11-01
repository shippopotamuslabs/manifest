# AGENTS.md — Manifest contributor notes

Agents working in this repo start here. README.md describes the
app for human readers; this file is the governing agent contract.

## What Manifest is

Manifest is a desktop harness for Muse Code, written in Rust on
the Iced UI framework. It is a pure coding tool: it drives coding
agent sessions, renders their turns as a live stream, and puts
approvals and questions in front of the user as first-class UI.

The first engine is Muse, reached through the
`shippopotamuslabs/muse-code-mcp` server over MCP stdio. The
server owns session lifecycle, approval brokering, and resume;
Manifest renders and routes. Streaming follows the server's
contract: the client declares the `muse.stream` experimental
capability at initialize and receives `notifications/muse/event`
events. The server's README is the source of truth for that
contract.

Later engines (bring-your-own-key providers, OpenAI, xAI) plug
into the same shell through `agent_core`, behind an engine seam
in the domain layer. The seam exists from the first slice; the
second engine is a later phase, commissioned separately.

Enclave is a separate product and stays separate. Manifest and
Enclave share the `agent_core` library; they share no product
surface.

The credential rule is absolute: Manifest never reads, stores,
prints, or forwards Muse OAuth tokens or
`~/.config/muse/auth.json`. Muse and the MCP server own
Muse credentials. Provider API keys, when the BYOK phase lands,
live in the OS keyring; a key in a repo file or a plain config
file is a contract violation.

## Repo state

Greenfield. This scaffold carries the contracts and the ignore
rules; the Cargo workspace arrives with the first commissioned
slice, after the vision phase. The quality bar for every slice is
the owner's: production-grade, staff-engineer-level work.

## Gospels

The cross-repo canon lives in the `shippopotamuslabs/gospels`
repo, pinned in this repo as a submodule at `contracts/gospel/`.
Read the gospels there; bump the pin deliberately, and never edit
the submodule's contents from this repo.

- Architecture follows
  `contracts/gospel/RUST-ARCHITECTURE-GOSPEL.md`: dependencies
  point inward, framework types stay at the edges, and the crate
  layering is presentation / domain / data.
- Failure classification follows `contracts/gospel/CODE-STYLE-GOSPEL.md`
  §§10–11, cross-stack canon: expected vs exceptional failures,
  partial success as diagnostics carried in values, one handler
  per boundary. In this repo's Rust terms: expected failures are
  typed errors carried in `Result`; each boundary (the MCP wire,
  the filesystem, a child process) converts its failures exactly
  once; a degradation with no value and no log line is a bug.
- Tests follow `contracts/gospel/TESTING-GOSPEL.md`: the general
  sections apply in full; its Android-specific §6 does not apply
  here. The Iced reducer is a first-class test surface (see the
  UI contract, §8).
- Anything a person reads — README, docs, comments, plan
  write-ups, error text — follows
  `contracts/gospel/WUMBO-GOSPEL.md`.
- The Compose UI gospels (Palette, Compose, Glass Bars, Button
  Shape) govern the Android apps and do not bind this Iced
  surface. The UI law here is `contracts/ICED-UI-CONTRACT.md`:
  read it before touching presentation code.

## Failure handling

- **Expected failures** are the routine refusals: server not
  running, unknown or closed session, a workspace the server
  rejects, a stream that drops mid-turn. They are values the
  domain layer branches on, and the UI renders them as named
  states.
- **Exceptional failures** (wire protocol breakage, corrupt local
  state, a panicking task) surface at the app boundary with a
  message the user can act on. Nothing below the boundary logs an
  error it also propagates.
- **Partial success** rides as values: a session that resumes
  with dropped deltas says so (`stream_dropped_deltas` on the
  server's status), and the UI shows the count rather than hiding
  the gap.

## Tests

Once the workspace exists, a change passes these locally before
pushing:

- `cargo test` — reducer and domain tests run headless.
- `cargo clippy -- -D warnings`
- UI performance claims are measured in release builds only
  (UI contract, §6).

## Conventions

- Work state lives in `plans/` (Synthitect status files), which
  is gitignored in this repo: plans stay local, contracts and
  code are what ship.
- PR titles follow the org convention — `type(scope): imperative
  subject (KEY-123)` — with the ticket key when the work has one.
  Individual commit messages are not linted; write them for the
  humans and agents reading the branch.
- The name is Manifest. Muse and Muse Code appear in this
  repo only as descriptive references (what the app works with),
  never as part of this app's own branding, and Meta's logos are
  not used.
