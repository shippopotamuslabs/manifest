# Manifest

First you muse, then you manifest.

Manifest is a desktop harness for Muse Code, built with Rust and
Iced. It drives Muse sessions through the muse-code-mcp
server and renders each turn as a live stream, with approvals and
questions presented as first-class UI.

Manifest is under active development. The shell runs, drives
Muse sessions through the bundled muse-code-mcp server, and
streams turns live.

## Run it

Prerequisites: install Muse yourself and log in; install Node 20+.

Run `./scripts/install.sh`, then run `manifest` (with
`~/.local/bin` on your `PATH` under the default prefix). For
development, the install script also provisions the bundled
server so `cargo run` from this checkout works; set
`MANIFEST_MUSE_MCP_BIN` to point at a different server.

Your projects (folder plus display name) persist in a local
registry at `~/.local/share/manifest/projects.json` on Linux.
Removing a project in the panel only removes it from that list,
never the folder.

Manifest is an independent project. It is not affiliated with or
endorsed by Meta. Muse and Muse Code are trademarks of Meta
Platforms, Inc.

0BSD License. Do whatever you want with the code just don't sue me or expect it to work.
