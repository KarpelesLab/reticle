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
use ieee.fixed_pkg.all;

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
        rendered.contains("fixed_pkg"),
        "the diagnostic does not name the package:\n{rendered}"
    );
    assert!(
        rendered.contains("not bundled"),
        "the diagnostic does not say the package is missing:\n{rendered}"
    );
    // Exactly one error: the rest of the design still analyses.
    assert_eq!(diags.error_count(), 1, "expected one error:\n{rendered}");
}

/// The operands the operator tables below are written against, declared
/// once at the head of the generated package.
const PROBES: &str = "\
  constant u200 : unsigned(7 downto 0) := to_unsigned(200, 8);
  constant u100 : unsigned(7 downto 0) := to_unsigned(100, 8);
  constant u3   : unsigned(3 downto 0) := to_unsigned(3, 4);
  constant u15  : unsigned(3 downto 0) := to_unsigned(15, 4);
  constant sm7  : signed(7 downto 0) := to_signed(-7, 8);
  constant sp7  : signed(7 downto 0) := to_signed(7, 8);
  constant sp3  : signed(7 downto 0) := to_signed(3, 8);
  constant sm3  : signed(7 downto 0) := to_signed(-3, 8);
  constant sm8  : signed(7 downto 0) := to_signed(-8, 8);
  constant smin : signed(7 downto 0) := to_signed(-128, 8);
  constant sone : signed(7 downto 0) := to_signed(1, 8);
  constant sm1  : signed(3 downto 0) := to_signed(-1, 4);
";

/// Analyses one package holding [`PROBES`] and one constant per case, and
/// compares each folded value with the expected text.
///
/// Everything goes through one analysis, so a whole table costs one pass;
/// a mismatch names the expression that broke.
fn check_constants(package: &str, cases: &[(&str, &str, &str)]) {
    use reticle::vhdl::sema::DeclKind;

    let mut src = format!(
        "library ieee;\nuse ieee.std_logic_1164.all;\nuse {package}.all;\n\
         package cases is\n{PROBES}"
    );
    for (i, (ty, expr, _)) in cases.iter().enumerate() {
        src.push_str(&format!("  constant c{i} : {ty} := {expr};\n"));
    }
    src.push_str("end package cases;\n");

    let (map, diags, analysis) = analyze("cases.vhd", &src);
    assert!(
        !diags.has_errors(),
        "the probe package did not analyse:\n{}",
        diags.render(&map)
    );
    let work = analysis.interner.get_ci("work");
    let region = analysis
        .units
        .iter()
        .find(|u| Some(u.library) == work)
        .and_then(|u| u.region)
        .expect("the probe package has a region");
    let probe_count = PROBES.lines().count();
    let mut seen = 0;
    for (n, &d) in analysis.region(region).decls.iter().enumerate() {
        if n < probe_count {
            continue;
        }
        let decl = analysis.decl(d);
        let DeclKind::Object { ty, .. } = decl.kind else {
            continue;
        };
        let (_, expr, expected) = cases[seen];
        let actual = analysis
            .decl_value(d)
            .map(|v| analysis.describe_value(v, ty))
            .unwrap_or_else(|| "<not static>".to_owned());
        assert_eq!(&actual, expected, "`{expr}` folded to {actual}");
        seen += 1;
    }
    assert_eq!(seen, cases.len(), "not every case produced a constant");
}

/// Every `ieee.numeric_std` operator that yields a vector, against a
/// hand-computed answer, through the real analyser.
///
/// The golden corpus covers the same ground in bulk; this table exists so
/// a regression names the operator that broke, and so the edge cases that
/// catch real bugs live in one place: mixed operand widths, wrapping at
/// the width, the two remainder signs, and narrowing a signed value.
#[test]
fn numeric_std_vector_operators_fold() {
    let u8t = "unsigned(7 downto 0)";
    let u16t = "unsigned(15 downto 0)";
    let u4t = "unsigned(3 downto 0)";
    let s8t = "signed(7 downto 0)";
    let s16t = "signed(15 downto 0)";
    let s4t = "signed(3 downto 0)";
    let s1t = "signed(0 downto 0)";
    check_constants(
        "ieee.numeric_std",
        &[
            // Conversions, including an integer too big for the width.
            (u8t, "to_unsigned(200, 8)", "\"11001000\""),
            (s8t, "to_signed(-7, 8)", "\"11111001\""),
            (u8t, "to_unsigned(300, 8)", "\"00101100\""),
            // Addition and subtraction, wrapping at the width.
            (u8t, "u200 + u100", "\"00101100\""),
            (u8t, "u100 - u200", "\"10011100\""),
            (s8t, "sm7 + sp3", "\"11111100\""),
            (s8t, "smin - sone", "\"01111111\""),
            // Mixed widths: the shorter operand is extended, not cut.
            (u8t, "u200 + u3", "\"11001011\""),
            (u8t, "u3 + u200", "\"11001011\""),
            (s8t, "sm7 + sm1", "\"11111000\""),
            // An integer operand takes the vector's length.
            (u8t, "u200 + 55", "\"11111111\""),
            (u8t, "55 + u200", "\"11111111\""),
            (s8t, "sm7 - 1", "\"11111000\""),
            // Multiplication is as wide as both operands together.
            (u16t, "u200 * u100", "\"0100111000100000\""),
            (s16t, "sm7 * sp3", "\"1111111111101011\""),
            (s16t, "smin * smin", "\"0100000000000000\""),
            // Division truncates towards zero; `rem` takes the sign of
            // the dividend and `mod` the sign of the divisor.
            (s8t, "sm7 / sp3", "\"11111110\""),
            (s8t, "sp7 / sm3", "\"11111110\""),
            (s8t, "sm7 rem sp3", "\"11111111\""),
            (s8t, "sm7 mod sp3", "\"00000010\""),
            (s8t, "sp7 rem sm3", "\"00000001\""),
            (s8t, "sp7 mod sm3", "\"11111110\""),
            (u8t, "u200 rem u100", "\"00000000\""),
            (u8t, "u200 / u100", "\"00000010\""),
            // Sign and magnitude. The most negative value has no
            // positive counterpart, so `abs` of it is itself.
            (s8t, "-sm7", "\"00000111\""),
            (s8t, "abs(sm7)", "\"00000111\""),
            (s8t, "abs(smin)", "\"10000000\""),
            // Resizing: growing extends, narrowing an unsigned truncates
            // and narrowing a signed keeps the sign bit.
            (u16t, "resize(u200, 16)", "\"0000000011001000\""),
            (s16t, "resize(sm7, 16)", "\"1111111111111001\""),
            (u4t, "resize(u200, 4)", "\"1000\""),
            (s4t, "resize(sm7, 4)", "\"1001\""),
            (s1t, "resize(sm7, 1)", "\"1\""),
            // Shifts. A right shift of a signed value fills with the
            // sign bit, except `srl`, which is always logical.
            (u8t, "shift_left(u200, 1)", "\"10010000\""),
            (u8t, "shift_right(u200, 4)", "\"00001100\""),
            (s8t, "shift_right(sm8, 1)", "\"11111100\""),
            (s8t, "sm8 srl 1", "\"01111100\""),
            (u8t, "u200 sll 1", "\"10010000\""),
            (u8t, "u200 srl 9", "\"00000000\""),
            // A rotate keeps every bit, and by the width is a no-op.
            (u8t, "rotate_left(u200, 3)", "\"01000110\""),
            (u8t, "rotate_right(u200, 3)", "\"00011001\""),
            (u8t, "rotate_left(u200, 8)", "\"11001000\""),
            // Extrema, and the element-wise logic.
            (u8t, "maximum(u200, u100)", "\"11001000\""),
            (s8t, "minimum(sm7, sp3)", "\"11111001\""),
            (u8t, "u200 and u100", "\"01000000\""),
            (u8t, "u200 xor u100", "\"10101100\""),
            (u8t, "not u200", "\"00110111\""),
        ],
    );
}

/// The `ieee.numeric_std` operations that answer with something other
/// than a vector: the comparisons, the tests and `to_integer`.
#[test]
fn numeric_std_predicates_fold() {
    let b = "boolean";
    let i = "integer";
    let l = "std_ulogic";
    check_constants(
        "ieee.numeric_std",
        &[
            (b, "u200 > u100", "true"),
            (b, "u200 < u100", "false"),
            // Comparing different widths extends rather than truncates,
            // so 15 in four bits does not beat 200 in eight.
            (b, "u3 < u200", "true"),
            (b, "u200 >= u15", "true"),
            (b, "sm1 > sm7", "true"),
            (b, "sm7 < sp3", "true"),
            (b, "u200 > 100", "true"),
            (b, "sm7 < 0", "true"),
            (b, "u200 = 200", "true"),
            (b, "sm7 /= -8", "true"),
            // The matching operators answer with a logic value.
            (l, "u200 ?= u200", "'1'"),
            (l, "u200 ?/= u100", "'1'"),
            (l, "u100 ?< u200", "'1'"),
            (l, "u200 ?>= u100", "'1'"),
            // Don't-cares in the pattern match anything.
            (b, "std_match(u200, unsigned'(\"11--1000\"))", "true"),
            (b, "std_match(u200, unsigned'(\"10--1000\"))", "false"),
            (b, "is_x(u200)", "false"),
            (i, "find_leftmost(u200, '1')", "7"),
            (i, "find_rightmost(u200, '1')", "3"),
            (i, "find_leftmost(to_unsigned(0, 8), '1')", "-1"),
            (i, "to_integer(u200)", "200"),
            (i, "to_integer(sm7)", "-7"),
            (i, "to_integer(resize(u200, 16))", "200"),
            (i, "to_integer(resize(sm7, 4))", "-7"),
            // The 2008 reductions.
            (l, "or u200", "'1'"),
            (l, "and u200", "'0'"),
            (l, "xor u200", "'1'"),
        ],
    );
}

/// `ieee.numeric_bit` is the same core over `bit`, so the same table of
/// answers holds with the two-state element encoding.
#[test]
fn numeric_bit_shares_the_core() {
    let u8t = "unsigned(7 downto 0)";
    let s8t = "signed(7 downto 0)";
    let b = "boolean";
    let i = "integer";
    check_constants(
        "ieee.numeric_bit",
        &[
            (u8t, "u200 + u100", "\"00101100\""),
            (u8t, "u200 + u3", "\"11001011\""),
            (s8t, "sm7 rem sp3", "\"11111111\""),
            (s8t, "sm7 mod sp3", "\"00000010\""),
            (s8t, "abs(sm7)", "\"00000111\""),
            (s8t, "shift_right(sm8, 1)", "\"11111100\""),
            ("signed(3 downto 0)", "resize(sm7, 4)", "\"1001\""),
            (b, "sm7 < sp3", "true"),
            (i, "to_integer(sm7)", "-7"),
            (i, "find_leftmost(u200, '1')", "7"),
            ("bit", "xor u200", "'1'"),
        ],
    );
}

/// Every arithmetic package that landed is usable: a `use` clause naming
/// one, and the types and functions it declares, analyse without error.
#[test]
fn bundled_arithmetic_packages_are_usable() {
    for (pkg, body) in [
        (
            "numeric_std",
            "constant c : unsigned(7 downto 0) := to_unsigned(5, 8);",
        ),
        (
            "numeric_bit",
            "constant c : signed(7 downto 0) := to_signed(-5, 8);",
        ),
        ("math_real", "constant c : real := sqrt(2.0) * math_pi;"),
        (
            "std_logic_arith",
            "constant c : unsigned(3 downto 0) := conv_unsigned(9, 4);",
        ),
        (
            "std_logic_unsigned",
            "constant c : integer := conv_integer(std_logic_vector'(\"1010\"));",
        ),
        (
            "std_logic_signed",
            "constant c : integer := conv_integer(std_logic_vector'(\"1010\"));",
        ),
    ] {
        let src = format!(
            "library ieee;\n\
             use ieee.std_logic_1164.all;\n\
             use ieee.{pkg}.all;\n\
             package p is\n  {body}\nend package;\n"
        );
        let (map, diags, _) = analyze("t.vhd", &src);
        assert!(
            !diags.has_errors(),
            "`ieee.{pkg}` is not usable:\n{}",
            diags.render(&map)
        );
    }
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
