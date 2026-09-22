//! `src/ffi/reticle.h` and the exported symbols must not drift apart.
//!
//! The header is written by hand, which is the only way to get comments a
//! C programmer can read, so nothing stops it going stale except a test.
//! This one reads both sides — the declarations in the header and the
//! `#[unsafe(no_mangle)] pub extern "C" fn` definitions under `src/ffi/` —
//! and compares the two sets of names. Adding a function on either side
//! without the other fails here, naming the symbol.
//!
//! It also checks that the `RETICLE_*` constants agree in value, since a
//! status code that means one thing in Rust and another in C is worse than
//! a missing one.
#![cfg(feature = "ffi")]

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

/// The directory holding the FFI module and its header.
fn ffi_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src/ffi")
}

/// The header's text.
fn header() -> String {
    fs::read_to_string(ffi_dir().join("reticle.h")).expect("src/ffi/reticle.h is missing")
}

/// Every `.rs` file of the FFI module, concatenated.
///
/// The unit tests live in `tests.rs` and call the entry points rather than
/// define them, so including it costs nothing and keeps this from needing
/// a list of files to maintain.
fn sources() -> String {
    let mut names: Vec<PathBuf> = fs::read_dir(ffi_dir())
        .expect("src/ffi is missing")
        .map(|entry| entry.expect("readable directory entry").path())
        .filter(|path| path.extension().is_some_and(|e| e == "rs"))
        .collect();
    names.sort();
    assert!(!names.is_empty(), "src/ffi holds no Rust sources");
    names
        .iter()
        .map(|path| fs::read_to_string(path).expect("readable source"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The identifier that follows `prefix` in `line`, if any.
fn name_after<'a>(line: &'a str, prefix: &str) -> Option<&'a str> {
    let rest = line.split_once(prefix)?.1;
    let end = rest
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .unwrap_or(rest.len());
    let name = &rest[..end];
    (!name.is_empty()).then_some(name)
}

/// Every `reticle_*` function the Rust side exports.
fn exported_symbols(sources: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for line in sources.lines() {
        let line = line.trim();
        // A doc comment or a `//` comment may mention a function name; the
        // definitions are the only lines that start with `pub`.
        if !line.starts_with("pub ") {
            continue;
        }
        if let Some(name) = name_after(line, "extern \"C\" fn ")
            && name.starts_with("reticle_")
        {
            out.insert(name.to_owned());
        }
    }
    out
}

/// Every `reticle_*` function the header declares.
///
/// A declaration is a line at the left margin whose type is `int`, `void`
/// or `const char *`, which is the whole vocabulary of this ABI.
fn declared_symbols(header: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for line in header.lines() {
        // Indented lines are continuations of a parameter list, and the
        // typedefs are handled separately.
        if line.starts_with(char::is_whitespace) || line.starts_with("typedef") {
            continue;
        }
        for prefix in ["int ", "void ", "const char *"] {
            if let Some(name) = name_after(line, prefix)
                && name.starts_with("reticle_")
            {
                out.insert(name.to_owned());
            }
        }
    }
    out
}

/// Every `#define RETICLE_… <number>` in the header.
fn header_constants(header: &str) -> BTreeMap<String, i64> {
    let mut out = BTreeMap::new();
    for line in header.lines() {
        let Some(rest) = line.strip_prefix("#define RETICLE_") else {
            continue;
        };
        let mut parts = rest.split_whitespace();
        let (Some(name), Some(value)) = (parts.next(), parts.next()) else {
            continue;
        };
        if let Ok(value) = value.parse::<i64>() {
            out.insert(format!("RETICLE_{name}"), value);
        }
    }
    out
}

/// Every `pub const RETICLE_…: c_int = <number>;` on the Rust side.
fn rust_constants(sources: &str) -> BTreeMap<String, i64> {
    let mut out = BTreeMap::new();
    for line in sources.lines() {
        let Some(rest) = line.trim().strip_prefix("pub const RETICLE_") else {
            continue;
        };
        let Some((name, rest)) = rest.split_once(':') else {
            continue;
        };
        let Some((_, value)) = rest.split_once('=') else {
            continue;
        };
        if let Ok(value) = value.trim().trim_end_matches(';').parse::<i64>() {
            out.insert(format!("RETICLE_{}", name.trim()), value);
        }
    }
    out
}

#[test]
fn the_header_declares_exactly_the_exported_symbols() {
    let exported = exported_symbols(&sources());
    let declared = declared_symbols(&header());

    assert!(
        exported.len() > 40,
        "the scan found only {} exported symbols, which cannot be right",
        exported.len()
    );

    let missing: Vec<&String> = exported.difference(&declared).collect();
    assert!(
        missing.is_empty(),
        "exported but not declared in src/ffi/reticle.h: {missing:?}"
    );

    let extra: Vec<&String> = declared.difference(&exported).collect();
    assert!(
        extra.is_empty(),
        "declared in src/ffi/reticle.h but not exported: {extra:?}"
    );
}

#[test]
fn the_header_and_rust_agree_on_every_constant() {
    let from_header = header_constants(&header());
    let from_rust = rust_constants(&sources());

    assert!(
        from_header.len() >= 15,
        "the scan found only {} constants in the header",
        from_header.len()
    );

    for (name, value) in &from_rust {
        assert_eq!(
            from_header.get(name),
            Some(value),
            "`{name}` is {value} in Rust; the header disagrees"
        );
    }
    for name in from_header.keys() {
        assert!(
            from_rust.contains_key(name),
            "`{name}` is in the header but not in Rust"
        );
    }
}

#[test]
fn the_header_declares_the_three_opaque_handles() {
    let header = header();
    for name in ["reticle_design", "reticle_sim", "reticle_diagnostics"] {
        assert!(
            header.contains(&format!("typedef struct {name} {name};")),
            "`{name}` is not declared as an opaque handle"
        );
        // Every handle needs a way back out.
        assert!(
            header.contains(&format!("void {name}_free(")),
            "`{name}` has no `_free`"
        );
    }
}
