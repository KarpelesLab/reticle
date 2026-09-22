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

step "Tests (all features)"
cargo test --all-features

# Every stage must build alone. `--all-targets` is what catches an
# integration test missing its `#![cfg(feature = ...)]` guard, and building
# `cli` alone is what catches the binary using a module its feature does not
# enable.
step "Each feature alone"
for f in "" verilog vhdl sim synth formal fpga asic lsp cli; do
    printf '  features=%s\n' "${f:-none}"
    if [ -z "$f" ]; then
        cargo clippy --no-default-features --all-targets -- -D warnings
    else
        cargo clippy --no-default-features --features "$f" --all-targets -- -D warnings
    fi
done

step "Tests (no default features)"
cargo test --no-default-features

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
