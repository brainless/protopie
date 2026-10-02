#!/usr/bin/env bash
# Build server + GUI, then run the GUI. The GUI spawns the server as a separate process.
set -euo pipefail
cd "$(dirname "$0")"
cargo build -p protopie-server -p protopie-gui
exec ./target/debug/protopie-gui "$@"
