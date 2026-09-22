//! The `reticle` command-line tool.
//!
//! A thin wrapper over the library: it loads files into a
//! [`reticle::source::SourceMap`], invokes the requested stage and prints
//! diagnostics. Argument parsing is hand-written to keep the crate free of
//! dependencies.
//!
//! Exit codes: 0 on success, 1 when errors were reported, 2 on usage errors.

use std::process::ExitCode;

use reticle::diag::{Diagnostic, Diagnostics};
use reticle::source::SourceMap;

const USAGE: &str = "\
reticle: a VHDL / Verilog compiler

Usage: reticle <command> [options] [files...]

Commands:
  check    Load the given source files and report diagnostics
  help     Show this message
  version  Show the version

Options:
  -h, --help     Show this message
  -V, --version  Show the version
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(command) = args.first().map(String::as_str) else {
        eprint!("{USAGE}");
        return ExitCode::from(2);
    };
    match command {
        "help" | "-h" | "--help" => {
            print!("{USAGE}");
            ExitCode::SUCCESS
        }
        "version" | "-V" | "--version" => {
            println!("reticle {}", reticle::VERSION);
            ExitCode::SUCCESS
        }
        "check" => check(&args[1..]),
        other => {
            eprintln!("error: unknown command `{other}`\n");
            eprint!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

/// Loads every named file and reports what can be reported at this stage.
///
/// The frontends are not implemented yet (see `ROADMAP.md`), so this only
/// exercises the I/O and diagnostic plumbing: missing files are errors, and
/// files with an extension no frontend claims are warnings.
fn check(paths: &[String]) -> ExitCode {
    if paths.is_empty() {
        eprintln!("error: `check` needs at least one source file\n");
        eprint!("{USAGE}");
        return ExitCode::from(2);
    }

    let mut map = SourceMap::new();
    let mut diags = Diagnostics::new();
    for path in paths {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(err) => {
                diags.push(Diagnostic::error(format!("cannot read `{path}`: {err}")));
                continue;
            }
        };
        if let Err(err) = map.add(path.clone(), text) {
            diags.push(Diagnostic::error(format!("cannot load `{path}`: {err}")));
            continue;
        }
        let known = matches!(
            std::path::Path::new(path)
                .extension()
                .and_then(|e| e.to_str())
                .map(str::to_ascii_lowercase)
                .as_deref(),
            Some("v" | "sv" | "vh" | "svh" | "vhd" | "vhdl")
        );
        if !known {
            diags.push(
                Diagnostic::warning(format!("`{path}` has no recognised HDL extension"))
                    .with_note("expected .v, .sv, .vh, .svh, .vhd or .vhdl"),
            );
        }
    }

    diags.sort();
    eprint!("{}", diags.render(&map));
    if diags.has_errors() {
        eprintln!(
            "error: could not check {} file(s); {} error(s) emitted",
            paths.len(),
            diags.error_count()
        );
        return ExitCode::from(1);
    }
    eprintln!(
        "note: loaded {} file(s); no frontend is implemented yet, see ROADMAP.md",
        map.len()
    );
    ExitCode::SUCCESS
}
