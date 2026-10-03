#!/usr/bin/env bash
# Cross-target file handoff of the WASM SQLite host (OpenSpec change
# `add-wasm-sqlite-host`, tests#WASM SQLite Host#Cross-target handoff):
#
#   1. the native host (tm-rusqlite) writes the fixture to native.db;
#   2. the WebAssembly build (sqlite-wasm-rs, memory VFS, under Node.js) imports
#      it, checks every engine row against the native dump, commits one more
#      transaction and exports wasm.db;
#   3. the native host opens wasm.db, checks it row for row and continues.
#
# Needs the wasm32-unknown-unknown target, Node.js 22.3+ and the
# wasm-bindgen-test-runner matching Cargo.lock on PATH.
set -euo pipefail

cd "$(dirname "$0")/.."
dir="$(mktemp -d)"
trap 'rm -rf "$dir"' EXIT
export TM_WASM_INTEROP_DIR="$dir"

TM_WASM_INTEROP_STEP=write cargo test -p tm-wasm --test native_host -- --ignored --exact interop_native_side
cargo test -p tm-wasm --target wasm32-unknown-unknown --test wasm_host -- interop_wasm_side
test -s "$dir/wasm.db" || { echo "the wasm side wrote no wasm.db" >&2; exit 1; }
TM_WASM_INTEROP_STEP=check cargo test -p tm-wasm --test native_host -- --ignored --exact interop_native_side
echo "cross-target handoff: ok"
