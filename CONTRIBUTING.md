# Contributing to Reticle

Rules that every change must follow. CI enforces most of them.

- **No third-party crates.** Everything is written in-house. If you think
  you need a dependency, you don't; write the piece you need.
- **Sans-I/O library.** Nothing under `src/` except `src/bin/` touches the
  filesystem, environment or network. Sources come in through
  `source::SourceMap`; results are returned as values.
- **Diagnostics, not panics.** User-facing problems go through
  `diag::Diagnostics` with a span. Panics are for internal invariants only.
- **Every object has a span.** AST nodes and IR objects carry a
  `source::Span` so later stages can report precisely.
- **Docs on everything public.** `missing_docs` is on. Module docs explain
  the design, not just the API.
- **Lints are clean.** `cargo fmt --all --check`,
  `cargo clippy --all-targets --all-features -- -D warnings`, and
  `cargo clippy --no-default-features -- -D warnings` must pass.
  `unsafe_code` is forbidden.
- **Tests with every feature.** Unit tests next to the code, golden-file
  tests under `testdata/` driven by `tests/`. A parser change comes with
  corpus additions.
- **Feature gates.** Stage modules stay behind their Cargo feature; shared
  types (`source`, `diag`, `intern`, `logic`, `ir`) are always compiled.
- **Deterministic output.** Sort before rendering; never iterate a
  `HashMap` into user-visible output.
- **Commits** are self-contained: build, lint and tests pass at every
  commit. Message: one summary line, blank line, why and what.
