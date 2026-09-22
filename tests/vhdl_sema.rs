//! Golden-file tests for VHDL semantic analysis.
//!
//! Every `testdata/vhdl/sema/<name>.vhd` is analysed against the bundled
//! `std` and `ieee` libraries and compared against:
//!
//! - `<name>.diag`: the rendered diagnostics, present only when there are
//!   any;
//! - `<name>.types`: the resolved type of every top-level signal, constant
//!   and port, present only when the file has any and analysis is clean.
//!
//! A file whose first line is `-- sema: vhdl93` is analysed as VHDL-93.
//! Files named `err_*` are the error cases and must produce at least one
//! error; every other file must analyse cleanly. Run with `UPDATE_EXPECT=1`
//! to rewrite the expectation files.

#![cfg(feature = "vhdl")]

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use reticle::diag::Diagnostics;
use reticle::source::SourceMap;
use reticle::vhdl::sema::{Design, LibraryUnitKind};
use reticle::vhdl::{Standard, stdlib};

fn corpus_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/vhdl/sema")
}

/// Compares `actual` with the golden file at `path`, where an empty
/// `actual` means the file must not exist. With `UPDATE_EXPECT=1` the file
/// is written (or removed) instead.
fn check_golden(path: &Path, actual: &str) -> Option<String> {
    let update = std::env::var_os("UPDATE_EXPECT").is_some_and(|v| v == "1");
    if update {
        if actual.is_empty() {
            let _ = fs::remove_file(path);
        } else {
            fs::write(path, actual).expect("write expectation");
        }
        return None;
    }
    let expected = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => panic!("reading {}: {e}", path.display()),
    };
    if expected == actual {
        return None;
    }
    Some(format!(
        "{} differs from the analyser output\n--- expected\n{expected}--- actual\n{actual}",
        path.display()
    ))
}

/// The standard selected by the file's first line.
fn standard_of(text: &str) -> Standard {
    if text.lines().next() == Some("-- sema: vhdl93") {
        Standard::Vhdl93
    } else {
        Standard::Vhdl2008
    }
}

/// Analyses one source as `work`, against the bundled libraries.
fn analyze(name: &str, text: &str) -> (SourceMap, Diagnostics, reticle::vhdl::Analysis) {
    let mut map = SourceMap::new();
    let standard = standard_of(text);
    let mut diags = Diagnostics::new();
    let mut design = Design::with_stdlib(&mut map, standard, &mut diags);
    assert!(
        !diags.has_errors(),
        "the bundled libraries failed to parse:\n{}",
        diags.render(&map)
    );
    let id = map.add(name.to_owned(), text.to_owned()).unwrap();
    design.add_source(&map, id, "work", &mut diags);
    let analysis = design.analyze(&map, &mut diags);
    diags.sort();
    (map, diags, analysis)
}

/// Renders the resolved types of the objects declared in `work`'s units,
/// one `unit.object : type` line each, in declaration order.
fn types_report(analysis: &reticle::vhdl::Analysis, map: &SourceMap) -> String {
    use reticle::vhdl::sema::{DeclKind, ObjectRole};

    let mut out = String::new();
    let work = analysis.interner.get_ci("work");
    for unit in &analysis.units {
        if Some(unit.library) != work {
            continue;
        }
        let Some(region) = unit.region else { continue };
        let unit_name = analysis.name(unit.name);
        let prefix = match unit.kind {
            LibraryUnitKind::Architecture => {
                let of = unit.primary.map(|p| analysis.name(p)).unwrap_or("?");
                format!("{of}({unit_name})")
            }
            _ => unit_name.to_owned(),
        };
        for &d in &analysis.region(region).decls {
            let decl = analysis.decl(d);
            let DeclKind::Object {
                ty, role, class, ..
            } = decl.kind
            else {
                continue;
            };
            if matches!(role, ObjectRole::Parameter | ObjectRole::External) {
                continue;
            }
            let _ = write!(
                out,
                "{prefix}.{} : {} {}",
                decl.spelling,
                class.as_str(),
                analysis.describe_type(ty, Some(map))
            );
            if let Some(v) = analysis.decl_value(d) {
                let _ = write!(out, " = {}", analysis.describe_value(v, ty));
            }
            out.push('\n');
        }
    }
    out
}

fn run_one(vhd: &Path) -> Vec<String> {
    let text = fs::read_to_string(vhd).expect("read corpus file");
    let name = vhd.file_name().unwrap().to_string_lossy().into_owned();
    let (map, diags, analysis) = analyze(&name, &text);
    let rendered = diags.render(&map);
    let types = if diags.has_errors() {
        String::new()
    } else {
        types_report(&analysis, &map)
    };

    let mut failures = Vec::new();
    for (ext, actual) in [("diag", rendered), ("types", types)] {
        if let Some(f) = check_golden(&vhd.with_extension(ext), &actual) {
            failures.push(f);
        }
    }
    failures
}

fn corpus_files() -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = fs::read_dir(corpus_dir())
        .expect("testdata/vhdl/sema exists")
        .map(|e| e.expect("dir entry").path())
        .filter(|p| p.extension().is_some_and(|e| e == "vhd"))
        .collect();
    files.sort();
    files
}

#[test]
fn golden_corpus() {
    let files = corpus_files();
    assert!(files.len() >= 30, "the sema corpus is too small");

    let mut failures = Vec::new();
    for f in &files {
        failures.extend(run_one(f));
    }
    if !failures.is_empty() {
        panic!(
            "{} golden mismatch(es); run with UPDATE_EXPECT=1 to accept:\n\n{}",
            failures.len(),
            failures.join("\n")
        );
    }
}

/// Every corpus file that is not an error case must analyse cleanly, and
/// every error case must produce at least one error.
#[test]
fn error_cases_are_the_only_ones_with_errors() {
    for vhd in corpus_files() {
        let text = fs::read_to_string(&vhd).unwrap();
        let name = vhd.file_name().unwrap().to_string_lossy().into_owned();
        let is_error_case = name.starts_with("err_");
        let (map, diags, _) = analyze(&name, &text);
        assert_eq!(
            diags.has_errors(),
            is_error_case,
            "{name}: unexpected error status\n{}",
            diags.render(&map)
        );
    }
}

/// The bundled libraries must parse and analyse with no diagnostics at
/// all: not even a warning, since user code inherits whatever they
/// declare.
#[test]
fn bundled_libraries_analyse_cleanly() {
    let mut map = SourceMap::new();
    let mut diags = Diagnostics::new();
    let design = Design::with_stdlib(&mut map, Standard::Vhdl2008, &mut diags);
    let analysis = design.analyze(&map, &mut diags);
    diags.sort();
    assert!(
        diags.is_empty(),
        "the bundled libraries are not clean:\n{}",
        diags.render(&map)
    );
    // Every bundled source produced at least one analysed unit.
    assert_eq!(analysis.files.len(), stdlib::SOURCES.len());
    for u in &analysis.units {
        assert!(u.analyzed, "{} was not analysed", analysis.name(u.name));
    }
    // The predefined types are all bound.
    let b = analysis.builtins;
    for (name, ty) in [
        ("boolean", b.boolean),
        ("bit", b.bit),
        ("character", b.character),
        ("severity_level", b.severity_level),
        ("integer", b.integer),
        ("real", b.real),
        ("time", b.time),
        ("natural", b.natural),
        ("positive", b.positive),
        ("string", b.string),
        ("bit_vector", b.bit_vector),
        ("boolean_vector", b.boolean_vector),
        ("file_open_kind", b.file_open_kind),
        ("file_open_status", b.file_open_status),
    ] {
        assert!(!analysis.is_error(ty), "`{name}` was not bound");
    }
    // `std_logic_1164` is there, with its nine-state type.
    assert!(analysis.unit("ieee", "std_logic_1164").is_some());
    let su = analysis.std_ulogic().expect("std_ulogic is declared");
    assert!(analysis.is_character_type(su));
}

/// Naming a package Reticle does not bundle yet gives one clear
/// diagnostic naming it, not a cascade of unknown identifiers.
#[test]
fn missing_package_is_reported_once() {
    let src = "\
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity t is
  port (clk : in std_logic);
end entity;

architecture a of t is
  signal s : std_logic;
begin
  s <= clk;
end architecture;
";
    let (map, diags, _) = analyze("t.vhd", src);
    let rendered = diags.render(&map);
    assert!(
        rendered.contains("numeric_std"),
        "the diagnostic does not name the package:\n{rendered}"
    );
    assert!(
        rendered.contains("not bundled"),
        "the diagnostic does not say the package is missing:\n{rendered}"
    );
    // Exactly one error: the rest of the design still analyses.
    assert_eq!(diags.error_count(), 1, "expected one error:\n{rendered}");
}

/// A package Reticle has never heard of is a plain unknown-unit error.
#[test]
fn unknown_package_is_reported() {
    let src = "\
library ieee;
use ieee.no_such_package.all;
entity t is end entity;
";
    let (map, diags, _) = analyze("t.vhd", src);
    let rendered = diags.render(&map);
    assert!(diags.has_errors(), "expected an error");
    assert!(
        rendered.contains("no_such_package"),
        "the diagnostic does not name the package:\n{rendered}"
    );
}
