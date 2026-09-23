#!/usr/bin/env bash
#
# Runs what CI runs, so a push does not discover it the slow way.
#
# The feature loop is the part that matters most: building only with
# `--all-features` hides a module that a feature forgot to pull in, which is
# exactly how `reticle fpga` reached master unable to compile in the default
# build.
#
# Usage: tools/check.sh [quick]
#   quick   skip the packaging and MSRV steps, which are the slow ones

set -euo pipefail

quick=${1:-}
cd "$(dirname "$0")/.."

step() { printf '\n\033[1m== %s\033[0m\n' "$1"; }

step "Format"
cargo fmt --all --check

step "Clippy (all features)"
cargo clippy --all-targets --all-features -- -D warnings

# `tests/ffi_c_example.rs` links `examples/ffi/demo.c` against a real
# static library, which is the only check that `src/ffi/reticle.h`
# describes the symbols a C compiler actually finds. Cargo does not build
# a staticlib during `cargo test` (the crate's `[lib]` is an rlib), and a
# nested cargo would block on this invocation's lock, so it is built here
# instead; without it the test skips with a message.
step "Static library for the C example"
cargo rustc --lib --all-features --crate-type staticlib

step "Tests (all features)"
cargo test --all-features

# Every stage must build alone. `--all-targets` is what catches an
# integration test missing its `#![cfg(feature = ...)]` guard, and building
# `cli` alone is what catches the binary using a module its feature does not
# enable.
step "Each feature alone"
for f in "" verilog vhdl sim synth formal fpga asic timing ip lsp cache viewer ffi wasm cli program; do
    printf '  features=%s\n' "${f:-none}"
    if [ -z "$f" ]; then
        cargo clippy --no-default-features --all-targets -- -D warnings
    else
        cargo clippy --no-default-features --features "$f" --all-targets -- -D warnings
    fi
done

step "Tests (no default features)"
cargo test --no-default-features

# The library must keep compiling for the browser playground. The target
# is not installed everywhere, so this is a skip rather than a failure;
# see docs/wasm.md for the command that produces the loadable module.
step "WebAssembly"
if rustup target list --installed | grep -qx wasm32-unknown-unknown; then
    cargo build --target wasm32-unknown-unknown --release \
        --no-default-features --features wasm,verilog,vhdl,sim,synth
else
    echo "  the wasm32-unknown-unknown target is not installed, skipping"
fi

step "Docs"
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features

if [ "$quick" = "quick" ]; then
    printf '\n\033[1mAll quick checks passed.\033[0m Packaging and MSRV were skipped.\n'
    exit 0
fi

step "Publishable crate"
cargo package

step "MSRV"
if rustup toolchain list | grep -q '^1\.89'; then
    cargo +1.89 check --all-features
else
    echo "  the 1.89 toolchain is not installed, skipping"
fi

printf '\n\033[1mAll checks passed.\033[0m\n'
