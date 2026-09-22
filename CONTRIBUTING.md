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
- **Lints are clean.** Run `tools/check.sh` (or `tools/check.sh quick` to
  skip packaging and the MSRV check); it runs exactly what CI runs.
  `unsafe_code` is forbidden. Building only with `--all-features` is not
  enough: every stage must also compile on its own, which is what the
  feature loop in that script checks.
- **Tests with every feature.** Unit tests next to the code, golden-file
  tests under `testdata/` driven by `tests/`. A parser change comes with
  corpus additions.
- **Feature gates.** Stage modules stay behind their Cargo feature; shared
  types (`source`, `diag`, `intern`, `logic`, `ir`) are always compiled.
  An integration test that uses a gated module needs
  `#![cfg(feature = "...")]` at the top, and a feature that the `reticle`
  binary reaches for must be listed in the `cli` feature.
- **Deterministic output.** Sort before rendering; never iterate a
  `HashMap` into user-visible output. Golden files are compared byte for
  byte, so `.gitattributes` pins line endings to LF; the two `crlf`
  fixtures are exempt on purpose, since they test CRLF handling.
- **Commits** are self-contained: build, lint and tests pass at every
  commit. Messages follow Conventional Commits so release-plz can derive
  versions and changelogs: `feat(verilog): add lexer`, `fix(diag): ...`,
  `docs:`, `test:`, `refactor:`, `chore:`, `ci:`. One summary line, blank
  line, then why and what. While the version is `0.0.x` there is no
  stable API to break, so never use `!` or a `BREAKING CHANGE:` footer;
  those start meaning something once a `0.1.0` is cut.
