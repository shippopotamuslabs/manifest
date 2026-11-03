#!/usr/bin/env bash
# Install Manifest and its bundled muse-code-mcp server payload.
#
# Layout: the manifest binary goes to $PREFIX/bin/manifest and the
# server payload to $PREFIX/share/manifest/muse-code-mcp, so the
# release binary finds its server without any PATH setup. The script
# also provisions the checkout's bundled server (npm ci in
# bundled/muse-code-mcp) so `cargo run` from this checkout works too.
#
# Idempotent and non-destructive: it only writes under $PREFIX and
# the gitignored node_modules dirs, and recreates the payload dir
# from scratch on each run.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
PREFIX="${PREFIX:-$HOME/.local}"

fail() {
    printf 'install: %s\n' "$1" >&2
    exit 1
}

warn() {
    printf 'install: warning: %s\n' "$1" >&2
}

# Preflight: Node 20+ is the server runtime.
command -v node >/dev/null 2>&1 || fail "node not found on PATH; install Node 20+ first."
NODE_MAJOR="$(node -p 'process.versions.node.split(".")[0]')"
case "$NODE_MAJOR" in
    '' | *[!0-9]*)
        fail "could not determine the node version; install Node 20+ first."
        ;;
esac
[ "$NODE_MAJOR" -ge 20 ] || fail "node $NODE_MAJOR found; Node 20+ is required."

# Preflight: cargo builds the binary.
command -v cargo >/dev/null 2>&1 || fail "cargo not found on PATH; install the Rust toolchain first."

# Muse itself is the user's own install, not something bundled here.
command -v muse >/dev/null 2>&1 || warn "muse not found on PATH; server sessions will fail without it (install and log in to Muse separately)."

# Fresh clones need the submodule checked out, but exported trees and
# checkouts that already carry the bundled files must not require git.
if [ ! -f "$ROOT/bundled/muse-code-mcp/bin/muse-mcp" ]; then
    git -C "$ROOT" submodule update --init bundled/muse-code-mcp \
        || fail "could not check out the bundled muse-code-mcp server (run 'git submodule update --init' from the repo root)."
fi
[ -f "$ROOT/bundled/muse-code-mcp/bin/muse-mcp" ] \
    || fail "bundled muse-code-mcp server not found under bundled/ - run \`git submodule update --init\` from the repo root."

# Build the binary.
(cd "$ROOT" && cargo build --release -p manifest_shell)

# Install the binary.
install -Dm755 "$ROOT/target/release/manifest" "$PREFIX/bin/manifest"

# Install the server payload: recreate from the submodule checkout.
PAYLOAD="$PREFIX/share/manifest/muse-code-mcp"
rm -rf "$PAYLOAD"
mkdir -p "$PAYLOAD"
cp -r "$ROOT/bundled/muse-code-mcp/bin" "$PAYLOAD/bin"
cp -r "$ROOT/bundled/muse-code-mcp/src" "$PAYLOAD/src"
cp "$ROOT/bundled/muse-code-mcp/package.json" "$PAYLOAD/package.json"
cp "$ROOT/bundled/muse-code-mcp/package-lock.json" "$PAYLOAD/package-lock.json"
for doc in LICENSE README.md; do
    if [ -f "$ROOT/bundled/muse-code-mcp/$doc" ]; then
        cp "$ROOT/bundled/muse-code-mcp/$doc" "$PAYLOAD/$doc"
    fi
done
npm --prefix "$PAYLOAD" ci --omit=dev --no-audit --no-fund

# Provision the checkout's bundled server for `cargo run`.
npm --prefix "$ROOT/bundled/muse-code-mcp" ci --omit=dev --no-audit --no-fund

printf 'Installed manifest to %s\n' "$PREFIX/bin/manifest"
printf 'Installed muse-code-mcp payload to %s\n' "$PAYLOAD"
printf 'Run it: %s (ensure %s is on your PATH)\n' "$PREFIX/bin/manifest" "$PREFIX/bin"
printf 'Prerequisites: install and log in to Muse yourself (the muse CLI); set MANIFEST_MUSE_MCP_BIN to override the bundled server.\n'
