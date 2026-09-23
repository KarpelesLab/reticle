//! The `reticle` command-line tool.
//!
//! A thin wrapper over the library: it reads files, calls one stage and
//! prints the result. Everything it does is reachable from Rust through the
//! `reticle` crate, which is the supported way to script the compiler; this
//! binary exists for the common one-shot operations.
//!
//! The library is sans-I/O, so this is the only place that touches the
//! filesystem. Argument parsing is hand-written (see [`args`]) because the
//! crate ships no dependencies.
//!
//! Exit codes: 0 on success, 1 when errors were reported or a check failed,
//! 2 on usage errors.

mod args;
mod cache_store;

use std::path::Path;
use std::process::ExitCode;

use args::{ArgError, Args, Spec};

use reticle::diag::{Diagnostic, Diagnostics};
use reticle::ir::Design;
use reticle::ir::emit::{self, Format};
use reticle::source::{SourceId, SourceMap};

const USAGE: &str = "\
reticle: a VHDL / Verilog compiler

Usage: reticle <command> [options] [files...]

Commands:
  build    Resolve a project's IP dependencies and elaborate it
  search   Search an IP registry index
  add      Add an IP dependency to a project from a registry index
  check    Parse and check source files
  fmt      Format Verilog and VHDL source
  synth    Synthesise a design to a technology-independent netlist
  fpga     Synthesise and map for an FPGA, and export the place-and-route inputs
  asic     Map onto a Liberty standard-cell library and export for OpenROAD
  emit     Write a design out in another format
  sim      Simulate a design and print its output
  verify   Prove or refute a design's assertions
  timing   Report static timing and clock domain crossings
  lsp      Run the language server over standard input and output
  cache    Build incrementally against a cache of elaborated modules
  viewer   Write the design out as browsable HTML: schematics and reference
  help     Show this message, or `reticle help <command>`
  version  Show the version

`synth`, `emit`, `sim` and `verify` take Verilog (.v, .sv), VHDL (.vhd,
.vhdl) or a design already in the `.rtl` IR text format. Sources of one
language elaborate together, so a testbench and the modules it
instantiates go on one command line.

Run `reticle help <command>` for a command's options.
";

const FMT_USAGE: &str = "Usage: reticle fmt [options] [files...]

Formats Verilog (.v, .sv) and VHDL (.vhd, .vhdl) source. With no option
the formatted text goes to stdout; a file is only rewritten with --write.

Options:
  --write    Rewrite each file in place
  --check    Report which files would change and exit 1 if any would
  --diff     Print a unified diff instead of the formatted text
  --width <n>    Line width to aim for (default 100)
  --indent <n>   Spaces per level, or `tab` (default 2)
";

const BUILD_USAGE: &str = "\
Usage: reticle build [options] [reticle.proj]

Reads a project manifest, resolves its IP dependencies, and elaborates
the project and everything it depends on into one design. Defaults to
`reticle.proj` in the current directory.

Options:
  --output <file>  Write the elaborated design as .rtl
  --lock <file>    Write the resolved versions here (default reticle.lock)
  --no-lock        Do not write a lock file
  --synth          Synthesise the elaborated design
  --report         Print what was resolved and elaborated
  --quiet          Suppress the summary line
";

const CACHE_USAGE: &str = "\
Usage: reticle cache [options] [files...]

Builds a design incrementally. Every module is keyed on the source text
that produced it, the options, and its dependencies' keys, so a re-run
after a small edit only re-elaborates what changed. Entries live one per
file under --dir and are plain text with a short header.

Editing a file invalidates that module and every module above it in the
hierarchy; modules below it, and unrelated ones, are read back from the
store.

Options:
  --dir <d>       Cache directory (default .reticle-cache)
  --top <module>  Treat this module as the top
  --param <N=V>   Override a top-level parameter or generic; repeatable
  --output <f>    Write the resulting design here as .rtl
  --synth         Also synthesise, caching the netlists separately
  --only-top      Build just the top module, not every module
  --max-size <n>  Evict least-recently-used entries past n bytes
  --stats         Print what hit and what missed, module by module
  --verify        Check every entry against its content and exit
  --list          List the entries and exit
  --clear         Remove every entry and exit
  --quiet         Suppress the summary line
";

const SEARCH_USAGE: &str = "\
Usage: reticle search --index <path> <query>

Searches an IP registry index by package name and description. The index
is either one file or a directory tree of files, as a crates.io-style
index is laid out.

Options:
  --index <path>  The index file or directory
";

const ADD_USAGE: &str = "\
Usage: reticle add --index <path> <package> [requirement]

Adds a dependency on <package> to the project manifest, choosing the
newest release the requirement allows (default: any). The manifest is
rewritten in place with its comments and layout kept.

Options:
  --index <path>    The index file or directory
  --project <file>  The project manifest (default reticle.proj)
  --dry-run         Print the rewritten manifest instead of saving it
";

const ASIC_USAGE: &str = "\
Usage: reticle asic --liberty <file.lib> [options] <design>...

Synthesises, maps onto the standard cells of a Liberty library, and
reports area and the critical path.

Options:
  --liberty <file>  The Liberty library to map onto; required
  --top <module>    Treat this module as the top
  --param <N=V>     Override a top-level parameter or generic; repeatable
  --output <file>   Write the mapped netlist as .rtl
  --verilog <file>  Write the mapped netlist as structural Verilog
  --quiet           Suppress the summary line
";

const CHECK_USAGE: &str = "\
Usage: reticle check [files...]

Parses each file and reports diagnostics. `.rtl` files are additionally
validated against the IR's structural rules.

Options:
  --param <N=V>  Override a top-level parameter or generic; repeatable
  --quiet   Print diagnostics only, no summary line
";

const FPGA_USAGE: &str = "\
Usage: reticle fpga [options] <design.v|design.rtl>...

Runs the whole target flow: generic synthesis, then block-RAM, DSP,
carry, IO and clock-buffer mapping, then LUT covering, then a rewrite to
the device's own primitives. Writes the netlist and constraints that
nextpnr reads.

Options:
  --device <name>    Target device; required (see --list-devices)
  --list-devices     List the built-in devices and exit
  --constraints <f>  Read pin and placement constraints from an .rcf file
  --top <module>     Treat this module as the top
  --param <N=V>      Override a top-level parameter or generic; repeatable
  --output-dir <d>   Write <top>.json and the constraints here (default: .)
  --netlist <file>   Also write the mapped design in the .rtl text format
  --report           Print the mapping report to stderr
  --quiet            Suppress the summary line

Xilinx 7 series only, and only with a chip database:
  --chipdb <dir>     A Project X-Ray database (f4pga/prjxray-db). Also
                     read from RETICLE_CHIPDB. Reticle never fetches it;
                     see docs/fpga-xray.md for the command that does.
  --bitstream <file> Write a 7-series .bit here, from the real fabric.
                     NOTHING PRODUCED THIS WAY HAS BEEN LOADED INTO A
                     PART; what is established is that the container is
                     the one UG470 describes and that its frames are at
                     addresses the part really has.
  --region <box>     Which tiles of the fabric to load, as x0,y0,x1,y1 in
                     the database's grid coordinates. The default is a
                     box around the constrained pins, because the whole
                     die is 20 million pips and will not fit.
";

const SYNTH_USAGE: &str = "\
Usage: reticle synth [options] <design.rtl>

Lowers processes to cells, infers flip-flops, latches and memories, then
optimises. Writes the resulting netlist in the `.rtl` format.

Options:
  --output <file>     Write the netlist here (default: stdout)
  --top <module>      Treat this module as the top
  --param <N=V>       Override a top-level parameter or generic; repeatable
  --fsm <encoding>    auto, binary, one-hot, gray or none
  --max-iterations <n>  Optimisation-loop cap (default 8)
  --lut <k>           Also map the logic onto k-input lookup tables (2..8)
  --gates             Also map the logic onto the generic gate library
  --verify            Prove the optimised netlist equivalent to the
                      unoptimised lowering (slow)
  --report            Print the pass log and cell counts to stderr
  --quiet             Suppress the summary line
";

const EMIT_USAGE: &str = "\
Usage: reticle emit [options] <design.rtl>

Options:
  --format <fmt>   verilog, vhdl, json, blif or edif (default verilog)
  --output <file>  Write here (default: stdout)
  --param <N=V>    Override a top-level parameter or generic; repeatable
";

const SIM_USAGE: &str = "\
Usage: reticle sim [options] <design.rtl>

Runs the design until $finish, until no events remain, or until the time
limit. Text from $display and report statements goes to stdout.

Options:
  --top <module>   Instantiate this module as the root
  --param <N=V>    Override a top-level parameter or generic; repeatable
  --until <ticks>  Stop at this time (in the design's precision)
  --seed <n>       Seed for $random (default 1)
  --vcd <file>     Write a VCD waveform here
  --fst <file>     Write a GTKWave FST waveform here
  --assert <expr>  Add a concurrent assertion; repeatable via a file with
                   --assert-file
  --assert-file <f>  Read one assertion per non-empty, non-# line
  --coverage <file>  Write a coverage report here (.info writes LCOV)
  --interactive    Read commands from standard input instead of running
                   to the end: run, step, break, force, print, watch, ...
                   (type `help` at the prompt)
  --quiet          Suppress the summary line
";

const TIMING_USAGE: &str = "\
Usage: reticle timing [options] <design.v|design.vhd|design.rtl>...

Reports setup and hold slack over the synthesised netlist, and the clock
domain crossings it can find. The design is synthesised first unless it
is already a netlist.

Options:
  --constraints <f>  Read create_clock and path exceptions from an .rcf file
  --period <ns>      Period for a clock no constraint names (default 10)
  --top <module>     Analyse this module
  --param <N=V>      Override a top-level parameter or generic; repeatable
  --paths <n>        Worst paths to report (default 10)
  --hold             Report hold instead of setup
  --cdc              Report clock domain crossings instead of timing
  --summary          Print only the per-clock summary
  --device <name>    Use the FPGA delay placeholders for this device family
";

const VIEWER_USAGE: &str = "\
Usage: reticle viewer [options] <design.v|design.vhd|design.rtl>...

Renders the design as a small static site: an index, a schematic page
per module and a reference page per module. Every page is
self-contained — no scripts, fonts or images are fetched — so it opens
from file:// and can be attached to a bug report.

The schematic is drawn from the cell form, so run it on a netlist, or
pass --synth to synthesise first; a module still in process form is
drawn one box per process instead.

Options:
  --output-dir <d>   Write the pages here (default: viewer)
  --top <module>     Treat this module as the top
  --param <N=V>      Override a top-level parameter or generic; repeatable
  --module <names>   Render only these modules (comma separated)
  --title <text>     Heading of the index page
  --synth            Synthesise first, so the schematic shows cells
  --max-nodes <n>    Skip a schematic with more boxes than this (default 1500)
  --no-schematic     Skip the schematic pages
  --no-doc           Skip the reference pages
  --no-source        Skip the source listings
  --quiet            Suppress the summary line
";

const LSP_USAGE: &str = "\
Usage: reticle lsp

Runs the Verilog and VHDL language server, speaking the Language Server
Protocol over standard input and output. An editor starts it; it is not
meant to be typed at. It takes no options.
";

const VERIFY_USAGE: &str = "\
Usage: reticle verify [options] <design.rtl>

Bounded model checking followed by k-induction over the nets marked with
the `formal_assert`, `formal_assume` and `formal_cover` attributes.

Options:
  --top <module>   Check this module (default: the design's top)
  --param <N=V>    Override a top-level parameter or generic; repeatable
  --depth <n>      Bounded-check depth (default 20)
  --max-k <n>      Largest induction depth to try (default 10)
  --init <mode>    reset, zero or free (default reset)
  --trace <file>   Write the counter-example as VCD here
";

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let Some(command) = argv.first().map(String::as_str) else {
        eprint!("{USAGE}");
        return ExitCode::from(2);
    };
    let rest = &argv[1..];
    match command {
        "help" | "-h" | "--help" => {
            print!("{}", help_text(rest.first().map(String::as_str)));
            ExitCode::SUCCESS
        }
        "version" | "-V" | "--version" => {
            println!("reticle {}", reticle::VERSION);
            ExitCode::SUCCESS
        }
        "build" => run(build, rest, BUILD_USAGE),
        "search" => run(search, rest, SEARCH_USAGE),
        "add" => run(add, rest, ADD_USAGE),
        "asic" => run(asic, rest, ASIC_USAGE),
        "check" => run(check, rest, CHECK_USAGE),
        "fmt" => run(fmt, rest, FMT_USAGE),
        "synth" => run(synth, rest, SYNTH_USAGE),
        "fpga" => run(fpga, rest, FPGA_USAGE),
        "emit" => run(emit_cmd, rest, EMIT_USAGE),
        "sim" => run(sim, rest, SIM_USAGE),
        "verify" => run(verify, rest, VERIFY_USAGE),
        "timing" => run(timing, rest, TIMING_USAGE),
        "lsp" => run(lsp, rest, LSP_USAGE),
        "cache" => run(cache_cmd, rest, CACHE_USAGE),
        "viewer" => run(viewer, rest, VIEWER_USAGE),
        other => {
            eprintln!("error: unknown command `{other}`\n");
            eprint!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

/// The help text for one command, or the overview.
fn help_text(command: Option<&str>) -> &'static str {
    match command {
        Some("build") => BUILD_USAGE,
        Some("search") => SEARCH_USAGE,
        Some("add") => ADD_USAGE,
        Some("asic") => ASIC_USAGE,
        Some("check") => CHECK_USAGE,
        Some("fmt") => FMT_USAGE,
        Some("synth") => SYNTH_USAGE,
        Some("fpga") => FPGA_USAGE,
        Some("emit") => EMIT_USAGE,
        Some("sim") => SIM_USAGE,
        Some("verify") => VERIFY_USAGE,
        Some("timing") => TIMING_USAGE,
        Some("lsp") => LSP_USAGE,
        Some("cache") => CACHE_USAGE,
        Some("viewer") => VIEWER_USAGE,
        _ => USAGE,
    }
}

/// What a command reports back.
enum Outcome {
    /// The command did its job.
    Ok,
    /// The command ran and found a problem in the user's design.
    Failed,
    /// The command was invoked wrongly; its usage text is printed.
    Usage(String),
}

/// Runs one command, turning its outcome into an exit code.
fn run(command: fn(&Args) -> Result<Outcome, ArgError>, argv: &[String], usage: &str) -> ExitCode {
    let spec = spec_for(usage);
    let args = match Args::parse(argv, &spec) {
        Ok(args) => args,
        Err(err) => {
            eprintln!("error: {err}\n");
            eprint!("{usage}");
            return ExitCode::from(2);
        }
    };
    match command(&args) {
        Ok(Outcome::Ok) => ExitCode::SUCCESS,
        Ok(Outcome::Failed) => ExitCode::from(1),
        Ok(Outcome::Usage(message)) => {
            eprintln!("error: {message}\n");
            eprint!("{usage}");
            ExitCode::from(2)
        }
        Err(err) => {
            eprintln!("error: {err}\n");
            eprint!("{usage}");
            ExitCode::from(2)
        }
    }
}

/// The accepted options for a command, keyed off its usage text so the two
/// can never drift apart.
fn spec_for(usage: &str) -> Spec {
    // Kept as one match rather than parsed out of the text: a static table
    // is checked by the compiler, a parsed one is not.
    if std::ptr::eq(usage, SEARCH_USAGE) {
        Spec {
            options: &["index"],
            flags: &[],
            repeated: &[],
        }
    } else if std::ptr::eq(usage, ADD_USAGE) {
        Spec {
            options: &["index", "project"],
            flags: &["dry-run"],
            repeated: &[],
        }
    } else if std::ptr::eq(usage, ASIC_USAGE) {
        Spec {
            options: &["liberty", "top", "output", "verilog"],
            flags: &["quiet"],
            repeated: &["param"],
        }
    } else if std::ptr::eq(usage, BUILD_USAGE) {
        Spec {
            options: &["output", "lock"],
            flags: &["no-lock", "synth", "report", "quiet"],
            repeated: &[],
        }
    } else if std::ptr::eq(usage, CACHE_USAGE) {
        Spec {
            options: &["dir", "top", "output", "max-size"],
            flags: &[
                "synth", "only-top", "stats", "verify", "list", "clear", "quiet",
            ],
            repeated: &["param"],
        }
    } else if std::ptr::eq(usage, CHECK_USAGE) {
        Spec {
            options: &[],
            flags: &["quiet"],
            repeated: &["param"],
        }
    } else if std::ptr::eq(usage, FMT_USAGE) {
        Spec {
            options: &["width", "indent"],
            flags: &["write", "check", "diff"],
            repeated: &[],
        }
    } else if std::ptr::eq(usage, SYNTH_USAGE) {
        Spec {
            options: &["output", "top", "fsm", "max-iterations", "lut"],
            flags: &["report", "quiet", "gates", "verify"],
            repeated: &["param"],
        }
    } else if std::ptr::eq(usage, FPGA_USAGE) {
        Spec {
            options: &[
                "device",
                "constraints",
                "top",
                "output-dir",
                "netlist",
                "chipdb",
                "bitstream",
                "region",
            ],
            flags: &["list-devices", "report", "quiet"],
            repeated: &["param"],
        }
    } else if std::ptr::eq(usage, LSP_USAGE) {
        Spec {
            options: &[],
            flags: &[],
            repeated: &[],
        }
    } else if std::ptr::eq(usage, TIMING_USAGE) {
        Spec {
            options: &["constraints", "period", "top", "paths", "device"],
            flags: &["hold", "cdc", "summary"],
            repeated: &["param"],
        }
    } else if std::ptr::eq(usage, VIEWER_USAGE) {
        Spec {
            options: &["output-dir", "top", "module", "title", "max-nodes"],
            flags: &["synth", "no-schematic", "no-doc", "no-source", "quiet"],
            repeated: &["param"],
        }
    } else if std::ptr::eq(usage, EMIT_USAGE) {
        Spec {
            options: &["format", "output"],
            flags: &[],
            repeated: &["param"],
        }
    } else if std::ptr::eq(usage, SIM_USAGE) {
        Spec {
            options: &[
                "top",
                "until",
                "seed",
                "vcd",
                "fst",
                "assert",
                "assert-file",
                "coverage",
            ],
            flags: &["quiet", "interactive"],
            repeated: &["param"],
        }
    } else {
        Spec {
            options: &["top", "depth", "max-k", "init", "trace"],
            flags: &[],
            repeated: &["param"],
        }
    }
}

/// Reads a file into the map, reporting failures as diagnostics.
fn load(map: &mut SourceMap, path: &str, diags: &mut Diagnostics) -> Option<SourceId> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) => {
            diags.push(Diagnostic::error(format!("cannot read `{path}`: {err}")));
            return None;
        }
    };
    match map.add(path.to_string(), text) {
        Ok(id) => Some(id),
        Err(err) => {
            diags.push(Diagnostic::error(format!("cannot load `{path}`: {err}")));
            None
        }
    }
}

/// Writes to a file, or to stdout when no path is given.
fn write_out(path: Option<&str>, text: &str) -> Result<(), String> {
    match path {
        None => {
            print!("{text}");
            Ok(())
        }
        Some(path) => {
            std::fs::write(path, text).map_err(|err| format!("cannot write `{path}`: {err}"))
        }
    }
}

/// Prints diagnostics and says whether any were errors.
fn report(diags: &mut Diagnostics, map: &SourceMap) -> bool {
    diags.sort();
    eprint!("{}", diags.render(map));
    diags.has_errors()
}

/// The lowercase extension of a path, if it has one.
fn extension(path: &str) -> Option<String> {
    Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
}

/// What kind of source the files on the command line are.
#[derive(PartialEq, Eq, Clone, Copy)]
enum Input {
    /// A design already in the `.rtl` IR text format.
    Rtl,
    /// Verilog or SystemVerilog, to be elaborated into the IR.
    Verilog,
    /// VHDL, which parses but does not reach the IR yet.
    Vhdl,
}

/// Classifies a path by extension.
fn classify(path: &str) -> Option<Input> {
    match extension(path).as_deref() {
        Some("rtl") => Some(Input::Rtl),
        Some("v" | "sv" | "vh" | "svh") => Some(Input::Verilog),
        Some("vhd" | "vhdl") => Some(Input::Vhdl),
        _ => None,
    }
}

/// Loads the design named on the command line.
///
/// Either one `.rtl` file, read straight into the IR, or one or more
/// Verilog sources, which are preprocessed, parsed and elaborated. Mixing
/// the two is rejected, since an `.rtl` file is already a whole design.
/// Splits each `--param NAME=VALUE` into its two halves.
///
/// The value is whatever follows the first `=`, so `--param S="a=b"`
/// keeps its own equals sign. The elaborator parses it as a literal of
/// the source language and keeps it as a string when it is not one, so
/// nothing here has to know the difference.
fn param_overrides(args: &Args) -> Result<Vec<(String, String)>, ArgError> {
    args.repeated("param")
        .iter()
        .map(|text| match text.split_once('=') {
            Some((name, value)) if !name.is_empty() => Ok((name.to_string(), value.to_string())),
            _ => Err(ArgError::BadValue {
                option: "param".to_string(),
                value: text.clone(),
                expected: "a `NAME=VALUE` pair",
            }),
        })
        .collect()
}

fn load_design(args: &Args) -> Result<Result<(Design, SourceMap), Outcome>, ArgError> {
    let paths = args.positionals();
    if paths.is_empty() {
        return Ok(Err(Outcome::Usage("no input file given".into())));
    }

    let mut kinds: Vec<Input> = Vec::new();
    for path in paths {
        match classify(path) {
            Some(kind) => kinds.push(kind),
            None => {
                return Ok(Err(Outcome::Usage(format!(
                    "`{path}` has no recognised extension; expected .rtl, .v, .sv, .vhd or .vhdl"
                ))));
            }
        }
    }
    if kinds.windows(2).any(|w| w[0] != w[1]) {
        return Ok(Err(Outcome::Usage(
            "every input must be the same kind: one .rtl design, or Verilog \
             sources, or VHDL sources"
                .into(),
        )));
    }

    let mut map = SourceMap::new();
    let mut diags = Diagnostics::new();

    if kinds[0] == Input::Rtl {
        if paths.len() != 1 {
            return Ok(Err(Outcome::Usage(format!(
                "expected one .rtl file, got {}",
                paths.len()
            ))));
        }
        let Some(id) = load(&mut map, &paths[0], &mut diags) else {
            report(&mut diags, &map);
            return Ok(Err(Outcome::Failed));
        };
        let text = map.file(id).text().to_string();
        return match Design::parse_text(&text, id) {
            Ok(design) => Ok(Ok((design, map))),
            Err(mut errors) => {
                report(&mut errors, &map);
                Ok(Err(Outcome::Failed))
            }
        };
    }

    if kinds[0] == Input::Vhdl {
        // VHDL analyses against the bundled std and ieee libraries, then
        // elaborates the analysed tree.
        use reticle::vhdl::sema::Design as VhdlDesign;

        let mut library =
            VhdlDesign::with_stdlib(&mut map, reticle::vhdl::Standard::default(), &mut diags);
        if report(&mut diags, &map) {
            eprintln!("error: the bundled VHDL libraries failed to parse");
            return Ok(Err(Outcome::Failed));
        }
        for path in paths {
            let Some(id) = load(&mut map, path, &mut diags) else {
                continue;
            };
            library.add_source(&map, id, "work", &mut diags);
        }
        if report(&mut diags, &map) {
            return Ok(Err(Outcome::Failed));
        }

        let mut diags = Diagnostics::new();
        let analysis = library.analyze(&map, &mut diags);
        if report(&mut diags, &map) {
            return Ok(Err(Outcome::Failed));
        }

        let mut options = reticle::vhdl::ElabOptions::new();
        options.top = args.option("top").map(str::to_string);
        options.generics = param_overrides(args)?;
        let mut diags = Diagnostics::new();
        let design = reticle::vhdl::elaborate(&analysis, &options, &mut diags);
        let failed = report(&mut diags, &map);
        return match design {
            Some(design) if !failed => Ok(Ok((design, map))),
            _ => Ok(Err(Outcome::Failed)),
        };
    }

    // Verilog: parse every file, then elaborate them together so a design
    // split across files resolves its instances.
    let mut files = Vec::new();
    for path in paths {
        let Some(id) = load(&mut map, path, &mut diags) else {
            continue;
        };
        let dialect = extension(path)
            .as_deref()
            .and_then(reticle::verilog::Dialect::for_extension)
            .unwrap_or_default();
        let mut resolver = IncludesFrom(path.clone());
        files.push(reticle::verilog::parse_source(
            &mut map,
            id,
            dialect,
            &mut resolver,
            &mut diags,
        ));
    }
    if report(&mut diags, &map) {
        return Ok(Err(Outcome::Failed));
    }

    let dialect = extension(&paths[0])
        .as_deref()
        .and_then(reticle::verilog::Dialect::for_extension)
        .unwrap_or_default();
    let mut options = reticle::verilog::ElabOptions::new(dialect);
    options.top = args.option("top").map(str::to_string);
    options.params = param_overrides(args)?;
    let refs: Vec<&reticle::verilog::ast::SourceFile> = files.iter().collect();
    let mut diags = Diagnostics::new();
    let design = reticle::verilog::elaborate(&refs, &options, &mut diags);
    let failed = report(&mut diags, &map);
    match design {
        Some(design) if !failed => Ok(Ok((design, map))),
        _ => Ok(Err(Outcome::Failed)),
    }
}

/// `reticle cache`: build incrementally against a store on disk.
fn cache_cmd(args: &Args) -> Result<Outcome, ArgError> {
    use reticle::cache::{BuildOptions, Cache, Language, SourceUnit, build};

    let dir = args.option("dir").unwrap_or(".reticle-cache");
    let mut storage = cache_store::FileStorage::new(dir);

    if args.flag("clear") {
        let mut cache = Cache::new(&mut storage);
        let removed = cache.len();
        cache.clear();
        if !args.flag("quiet") {
            println!("removed {removed} entries from {dir}");
        }
        return Ok(Outcome::Ok);
    }

    if args.flag("verify") {
        let cache = Cache::new(&mut storage);
        let bad = cache.verify();
        for corruption in &bad {
            eprintln!("error: {corruption}");
        }
        if !args.flag("quiet") {
            println!(
                "{} entries, {} bytes, {} damaged",
                cache.len(),
                cache.total_bytes(),
                bad.len()
            );
        }
        return Ok(if bad.is_empty() {
            Outcome::Ok
        } else {
            Outcome::Failed
        });
    }

    if args.flag("list") {
        let cache = Cache::new(&mut storage);
        for key in cache.keys() {
            match cache.peek(key) {
                Some(entry) => println!("{key}  {:>8}  {}", entry.size(), entry.producer),
                None => println!("{key}  {:>8}  <unreadable>", "?"),
            }
        }
        if !args.flag("quiet") {
            println!("{} entries, {} bytes", cache.len(), cache.total_bytes());
        }
        return Ok(Outcome::Ok);
    }

    let paths = args.positionals();
    if paths.is_empty() {
        return Ok(Outcome::Usage("no input file given".into()));
    }

    let mut units = Vec::with_capacity(paths.len());
    for path in paths {
        match std::fs::read_to_string(path) {
            Ok(text) => units.push(SourceUnit::new(path.clone(), text)),
            Err(err) => {
                eprintln!("error: cannot read {path}: {err}");
                return Ok(Outcome::Failed);
            }
        }
    }

    let mut options = BuildOptions::new(Language::Verilog);
    options.top = args.option("top").map(str::to_string);
    options.only_top = args.flag("only-top");
    options.capacity = args.u64_option("max-size")?;
    // The library has no clock; the CLI does, so entries get a timestamp.
    options.created = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    if args.flag("synth") {
        // `$readmemh` reads through this, like `reticle sim` does. The
        // cache records every file synthesis reads with a digest of its
        // contents and re-checks them on each hit, so editing only a
        // `.hex` file still rebuilds the netlist.
        options.synth = Some(synth_options(paths));
    }

    let mut diags = Diagnostics::new();
    let result = build(&units, &options, &mut storage, &mut diags);
    let failed = report(&mut diags, &result.sources);

    if args.flag("stats") {
        print!("{}", result.report());
    }

    let Some(design) = &result.design else {
        return Ok(Outcome::Failed);
    };
    if let Some(path) = args.option("output")
        && let Err(err) = write_out(Some(path), &design.to_text())
    {
        eprintln!("error: {err}");
        return Ok(Outcome::Failed);
    }

    if !args.flag("quiet") {
        eprintln!(
            "{}: {} modules, {} cache hits, {} misses",
            result.top.as_deref().unwrap_or("<no top>"),
            design.modules.len(),
            result.hits,
            result.misses
        );
    }
    Ok(if failed { Outcome::Failed } else { Outcome::Ok })
}

/// Reads a registry index from one file or a directory tree of files.
fn load_index(
    path: &str,
    map: &mut SourceMap,
    diags: &mut Diagnostics,
) -> Option<reticle::ip::registry::Index> {
    use reticle::ip::registry::Index;

    // A directory is walked in sorted order, so the same tree always
    // yields the same index regardless of how the filesystem lists it.
    fn files_under(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        let mut entries: Vec<_> = entries.filter_map(Result::ok).map(|e| e.path()).collect();
        entries.sort();
        for entry in entries {
            if entry.is_dir() {
                files_under(&entry, out);
            } else {
                out.push(entry);
            }
        }
    }

    let root = std::path::Path::new(path);
    let mut files = Vec::new();
    if root.is_dir() {
        files_under(root, &mut files);
    } else {
        files.push(root.to_path_buf());
    }
    if files.is_empty() {
        diags.push(Diagnostic::error(format!("`{path}` holds no index files")));
        return None;
    }

    let mut index = Index::new();
    for file in files {
        let name = file.to_string_lossy().into_owned();
        let Some(id) = load(map, &name, diags) else {
            continue;
        };
        let text = map.file(id).text().to_string();
        for entry in Index::parse(&text, id, diags).entries() {
            index.insert(entry.clone());
        }
    }
    Some(index)
}

/// `reticle search`: find packages in a registry index.
fn search(args: &Args) -> Result<Outcome, ArgError> {
    let Some(index_path) = args.option("index") else {
        return Ok(Outcome::Usage("no index given; pass --index".into()));
    };
    let query = match args.positionals() {
        [one] => one.clone(),
        [] => return Ok(Outcome::Usage("no search query given".into())),
        many => many.join(" "),
    };
    let mut map = SourceMap::new();
    let mut diags = Diagnostics::new();
    let Some(index) = load_index(index_path, &mut map, &mut diags) else {
        report(&mut diags, &map);
        return Ok(Outcome::Failed);
    };
    if report(&mut diags, &map) {
        return Ok(Outcome::Failed);
    }
    let matches = index.search(&query);
    if matches.is_empty() {
        eprintln!("note: nothing in the index matches `{query}`");
        return Ok(Outcome::Ok);
    }
    for m in matches {
        let yanked = if m.yanked { "  (yanked)" } else { "" };
        match &m.description {
            Some(text) => println!("{} {}  {text}{yanked}", m.name, m.version),
            None => println!("{} {}{yanked}", m.name, m.version),
        }
    }
    Ok(Outcome::Ok)
}

/// `reticle add`: add a dependency to a project from a registry index.
fn add(args: &Args) -> Result<Outcome, ArgError> {
    use reticle::ip::manifest::VersionReq;
    use reticle::ip::registry::{ProjectFile, add as add_dependency};

    let Some(index_path) = args.option("index") else {
        return Ok(Outcome::Usage("no index given; pass --index".into()));
    };
    let (name, req_text) = match args.positionals() {
        [name] => (name.clone(), "*".to_string()),
        [name, req] => (name.clone(), req.clone()),
        [] => return Ok(Outcome::Usage("no package named".into())),
        _ => {
            return Ok(Outcome::Usage(
                "expected a package and at most one requirement".into(),
            ));
        }
    };
    let Some(req) = VersionReq::parse(&req_text) else {
        return Ok(Outcome::Usage(format!(
            "`{req_text}` is not a version requirement; use 1.2.3, ^1.2.3, >=1.2.3 or *"
        )));
    };

    let manifest = args.option("project").unwrap_or("reticle.proj").to_string();
    let text = match std::fs::read_to_string(&manifest) {
        Ok(text) => text,
        Err(err) => {
            eprintln!("error: cannot read `{manifest}`: {err}");
            return Ok(Outcome::Failed);
        }
    };

    let mut map = SourceMap::new();
    let mut diags = Diagnostics::new();
    let Some(project) = reticle::ip::load_project(&mut map, manifest.clone(), &text, &mut diags)
    else {
        report(&mut diags, &map);
        return Ok(Outcome::Failed);
    };
    let Some(index) = load_index(index_path, &mut map, &mut diags) else {
        report(&mut diags, &map);
        return Ok(Outcome::Failed);
    };
    if report(&mut diags, &map) {
        return Ok(Outcome::Failed);
    }

    let file = ProjectFile {
        project: &project,
        text: &text,
    };
    let added = match add_dependency(&file, &name, &req, &index) {
        Ok(added) => added,
        Err(err) => {
            let mut one = Diagnostics::new();
            one.push(err.diagnostic());
            report(&mut one, &map);
            return Ok(Outcome::Failed);
        }
    };

    if args.flag("dry-run") {
        print!("{}", added.text);
        return Ok(Outcome::Ok);
    }
    if let Err(err) = std::fs::write(&manifest, &added.text) {
        eprintln!("error: cannot write `{manifest}`: {err}");
        return Ok(Outcome::Failed);
    }
    eprintln!("note: added {name} {} to {manifest}", added.version);
    Ok(Outcome::Ok)
}

/// `reticle asic`: map onto a Liberty library and report area and timing.
fn asic(args: &Args) -> Result<Outcome, ArgError> {
    use reticle::asic::liberty::Library;
    use reticle::asic::{AsicOptions, synthesize_asic};

    let Some(lib_path) = args.option("liberty") else {
        return Ok(Outcome::Usage("no library given; pass --liberty".into()));
    };
    let (mut design, mut map) = match load_design(args)? {
        Ok(pair) => pair,
        Err(outcome) => return Ok(outcome),
    };

    let mut diags = Diagnostics::new();
    let Some(id) = load(&mut map, lib_path, &mut diags) else {
        report(&mut diags, &map);
        return Ok(Outcome::Failed);
    };
    let text = map.file(id).text().to_string();
    let Some(library) = Library::parse(&text, id, &mut diags) else {
        report(&mut diags, &map);
        return Ok(Outcome::Failed);
    };
    if report(&mut diags, &map) {
        return Ok(Outcome::Failed);
    }

    let top = match args.option("top") {
        Some(name) => match design.module_by_name(name) {
            Some(id) => id,
            None => return Ok(Outcome::Usage(format!("no module named `{name}`"))),
        },
        None => match design.top {
            Some(id) => id,
            None => {
                return Ok(Outcome::Usage(
                    "the design names no top module; pass --top".into(),
                ));
            }
        },
    };

    let mut diags = Diagnostics::new();
    let result = synthesize_asic(
        &mut design,
        top,
        &library,
        &AsicOptions {
            synth: synth_options(args.positionals()),
            ..AsicOptions::default()
        },
        &mut diags,
    );
    let failed = report(&mut diags, &map);
    let report_ = match result {
        Ok(report_) => report_,
        Err(err) => {
            eprintln!("error: {err}");
            return Ok(Outcome::Failed);
        }
    };
    print!("{}", report_.to_text());
    if failed {
        return Ok(Outcome::Failed);
    }

    if let Some(path) = args.option("output")
        && let Err(message) = write_out(Some(path), &design.to_text())
    {
        eprintln!("error: {message}");
        return Ok(Outcome::Failed);
    }
    if let Some(path) = args.option("verilog") {
        match reticle::ir::emit::emit(&design, reticle::ir::emit::Format::Verilog) {
            Ok(text) => {
                if let Err(message) = write_out(Some(path), &text) {
                    eprintln!("error: {message}");
                    return Ok(Outcome::Failed);
                }
            }
            Err(err) => {
                eprintln!("error: {err}");
                return Ok(Outcome::Failed);
            }
        }
    }
    Ok(Outcome::Ok)
}

/// `reticle build`: resolve a project's IP and elaborate it.
fn build(args: &Args) -> Result<Outcome, ArgError> {
    use reticle::ip::{PathProvider, elaborate, load_project, resolve};

    let manifest = match args.positionals() {
        [] => "reticle.proj".to_string(),
        [one] => one.clone(),
        many => {
            return Ok(Outcome::Usage(format!(
                "expected one project manifest, got {}",
                many.len()
            )));
        }
    };
    let text = match std::fs::read_to_string(&manifest) {
        Ok(text) => text,
        Err(err) => {
            eprintln!("error: cannot read `{manifest}`: {err}");
            return Ok(Outcome::Failed);
        }
    };

    // Paths in a manifest are relative to the directory holding it, so the
    // provider is rooted there rather than at the current directory.
    let root = std::path::Path::new(&manifest)
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(std::path::Path::new("."))
        .to_path_buf();

    let mut map = SourceMap::new();
    let mut diags = Diagnostics::new();
    let Some(project) = load_project(&mut map, manifest.clone(), &text, &mut diags) else {
        report(&mut diags, &map);
        return Ok(Outcome::Failed);
    };

    // The library never touches the filesystem: it asks through this
    // closure, and the CLI owns every read.
    let read_root = root.clone();
    let mut provider = PathProvider::new(".", move |path: &str| {
        std::fs::read_to_string(read_root.join(path)).ok()
    });

    let mut resolved = resolve(map, &project, &mut provider, &mut diags);
    let elaboration = elaborate(&project, &mut resolved, &mut diags);
    let map = resolved.source_map();
    let failed = report(&mut diags, map);

    if args.flag("report") {
        eprint!("{}", elaboration.report());
    }

    if !args.flag("no-lock") && resolved.is_complete() {
        let lock = args
            .option("lock")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| root.join("reticle.lock"));
        if let Err(err) = std::fs::write(&lock, resolved.lock.to_text()) {
            eprintln!("error: cannot write `{}`: {err}", lock.display());
            return Ok(Outcome::Failed);
        }
    }

    let Some(mut design) = elaboration.design else {
        if !failed {
            eprintln!("error: the project produced no design");
        }
        return Ok(Outcome::Failed);
    };
    if failed {
        return Ok(Outcome::Failed);
    }

    if args.flag("synth") {
        let mut diags = Diagnostics::new();
        let options = synth_options(std::slice::from_ref(&manifest));
        let stats = reticle::synth::run(&mut design, &options, &mut diags);
        if report(&mut diags, map) {
            return Ok(Outcome::Failed);
        }
        if args.flag("report") {
            eprint!("{}", stats.render(Some(map)));
        }
    }

    if let Some(path) = args.option("output")
        && let Err(message) = write_out(Some(path), &design.to_text())
    {
        eprintln!("error: {message}");
        return Ok(Outcome::Failed);
    }

    if !args.flag("quiet") {
        eprintln!(
            "note: built `{}`: {} module(s) from {} source(s)",
            project.name,
            design.modules.len(),
            elaboration.sources.len()
        );
    }
    Ok(Outcome::Ok)
}

/// `reticle fmt`: lay source back out in one house style.
fn fmt(args: &Args) -> Result<Outcome, ArgError> {
    use reticle::fmt_doc::{FormatOptions, Indent};

    let paths = args.positionals();
    if paths.is_empty() {
        return Ok(Outcome::Usage("no input file given".into()));
    }
    if args.flag("write") && args.flag("check") {
        return Ok(Outcome::Usage(
            "--write and --check do opposite things; pick one".into(),
        ));
    }

    let mut options = FormatOptions::default();
    if let Some(width) = args.u32_option("width")? {
        options.line_width = width as usize;
    }
    if let Some(indent) = args.option("indent") {
        options.indent = if indent.eq_ignore_ascii_case("tab") {
            Indent::Tabs
        } else {
            match indent.parse::<usize>() {
                Ok(n) => Indent::Spaces(n),
                Err(_) => {
                    return Ok(Outcome::Usage(format!(
                        "`--indent {indent}` is not a number of spaces or `tab`"
                    )));
                }
            }
        };
    }

    let mut map = SourceMap::new();
    let mut diags = Diagnostics::new();
    let mut would_change = Vec::new();
    let mut failed = false;

    for path in paths {
        let Some(kind) = classify(path) else {
            diags.push(
                Diagnostic::error(format!("`{path}` is not a Verilog or VHDL source"))
                    .with_note("expected .v, .sv, .vh, .svh, .vhd or .vhdl"),
            );
            failed = true;
            continue;
        };
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(err) => {
                diags.push(Diagnostic::error(format!("cannot read `{path}`: {err}")));
                failed = true;
                continue;
            }
        };

        let result = match kind {
            Input::Verilog => {
                let dialect = extension(path)
                    .as_deref()
                    .and_then(reticle::verilog::Dialect::for_extension)
                    .unwrap_or_default();
                reticle::verilog::format::format_check(&text, dialect, &options)
            }
            Input::Vhdl => reticle::vhdl::format::format_check(
                &text,
                reticle::vhdl::Standard::default(),
                &options,
            ),
            Input::Rtl => {
                diags.push(
                    Diagnostic::error(format!("`{path}` is an IR file, not HDL source"))
                        .with_note("the .rtl text format is already canonical"),
                );
                failed = true;
                continue;
            }
        };

        let check = match result {
            Ok(check) => check,
            Err(mut errors) => {
                // The file does not parse, so formatting it would risk
                // damaging it. Report and leave it alone.
                let id = match map.add(path.clone(), text) {
                    Ok(id) => id,
                    Err(err) => {
                        diags.push(Diagnostic::error(format!("cannot load `{path}`: {err}")));
                        failed = true;
                        continue;
                    }
                };
                let _ = id;
                diags.append(&mut errors);
                failed = true;
                continue;
            }
        };

        if check.changed {
            would_change.push(path.clone());
        }
        if args.flag("check") {
            if check.changed && args.flag("diff") {
                print!("{}", check.diff);
            }
        } else if args.flag("write") {
            if check.changed
                && let Err(err) = std::fs::write(path, &check.formatted)
            {
                diags.push(Diagnostic::error(format!("cannot write `{path}`: {err}")));
                failed = true;
            }
        } else if args.flag("diff") {
            print!("{}", check.diff);
        } else {
            print!("{}", check.formatted);
        }
    }

    // The diagnostics above carry no spans (they are file-level), so an
    // empty map renders them fine.
    if report(&mut diags, &map) {
        failed = true;
    }

    if args.flag("check") && !would_change.is_empty() {
        for path in &would_change {
            eprintln!("would reformat {path}");
        }
        eprintln!("{} file(s) would change", would_change.len());
        return Ok(Outcome::Failed);
    }
    if args.flag("write") && !would_change.is_empty() {
        eprintln!("reformatted {} file(s)", would_change.len());
    }
    Ok(if failed { Outcome::Failed } else { Outcome::Ok })
}

/// `reticle check`: parse every file and report what is wrong with it.
fn check(args: &Args) -> Result<Outcome, ArgError> {
    let paths = args.positionals();
    if paths.is_empty() {
        return Ok(Outcome::Usage("no input file given".into()));
    }
    let mut map = SourceMap::new();
    let mut diags = Diagnostics::new();
    let mut checked = 0usize;

    for path in paths {
        let Some(id) = load(&mut map, path, &mut diags) else {
            continue;
        };
        let text = map.file(id).text().to_string();
        match extension(path).as_deref() {
            Some("rtl") => match Design::parse_text(&text, id) {
                Ok(design) => {
                    let mut errors = reticle::ir::validate::validate(&design);
                    diags.append(&mut errors);
                    checked += 1;
                }
                Err(mut errors) => diags.append(&mut errors),
            },
            Some("v" | "sv" | "vh" | "svh") => {
                let mut resolver = IncludesFrom(path.clone());
                let dialect = extension(path)
                    .as_deref()
                    .and_then(reticle::verilog::Dialect::for_extension)
                    .unwrap_or_default();
                let file = reticle::verilog::parse_source(
                    &mut map,
                    id,
                    dialect,
                    &mut resolver,
                    &mut diags,
                );
                checked += file.items.len().min(1);
            }
            Some("vhd" | "vhdl") => {
                let file = reticle::vhdl::parse_source(
                    &map,
                    id,
                    reticle::vhdl::Standard::default(),
                    &mut diags,
                );
                checked += file.units.len().min(1);
            }
            _ => diags.push(
                Diagnostic::warning(format!("`{path}` has no recognised extension"))
                    .with_note("expected .rtl, .v, .sv, .vh, .svh, .vhd or .vhdl"),
            ),
        }
    }

    let failed = report(&mut diags, &map);
    if !args.flag("quiet") && !failed {
        eprintln!("note: checked {checked} file(s), no errors");
    }
    Ok(if failed { Outcome::Failed } else { Outcome::Ok })
}

/// Resolves `` `include `` relative to the including file's directory.
struct IncludesFrom(String);

impl reticle::verilog::IncludeResolver for IncludesFrom {
    fn resolve(&mut self, path: &str, _from: SourceId) -> Option<(String, String)> {
        let base = Path::new(&self.0).parent().unwrap_or(Path::new("."));
        let full = base.join(path);
        let text = std::fs::read_to_string(&full).ok()?;
        Some((full.to_string_lossy().into_owned(), text))
    }
}

/// `reticle synth`: processes to cells, then optimisation.
fn synth(args: &Args) -> Result<Outcome, ArgError> {
    use reticle::synth::{FsmEncoding, SynthOptions};

    // Validate the options before touching the filesystem, so a bad flag is
    // reported as a usage error rather than masked by a missing file.
    let lut = args.u32_option("lut")?;
    if let Some(k) = lut
        && !(2..=8).contains(&k)
    {
        return Ok(Outcome::Usage(format!(
            "`--lut {k}` is out of range; k must be 2 to 8"
        )));
    }
    if lut.is_some() && args.flag("gates") {
        return Ok(Outcome::Usage(
            "--lut and --gates are different targets; pick one".into(),
        ));
    }

    let (mut design, map) = match load_design(args)? {
        Ok(pair) => pair,
        Err(outcome) => return Ok(outcome),
    };

    let mut options = SynthOptions {
        ..synth_options(args.positionals())
    };
    if let Some(name) = args.option("fsm") {
        match FsmEncoding::from_attr(name) {
            Some(encoding) => options.fsm_encoding = encoding,
            None => {
                return Ok(Outcome::Usage(format!(
                    "`--fsm {name}` is not one of auto, binary, one-hot, gray, none"
                )));
            }
        }
    }
    if let Some(n) = args.u32_option("max-iterations")? {
        options.max_iterations = n;
    }
    options.verify_equivalence = args.flag("verify");
    if let Some(top) = args.option("top") {
        match design.module_by_name(top) {
            Some(id) => design.top = Some(id),
            None => return Ok(Outcome::Usage(format!("no module named `{top}`"))),
        }
    }

    let mut diags = Diagnostics::new();
    let stats = reticle::synth::run(&mut design, &options, &mut diags);
    let failed = report(&mut diags, &map);
    if args.flag("report") {
        eprint!("{}", stats.render(Some(&map)));
    }
    if failed {
        return Ok(Outcome::Failed);
    }

    // Technology mapping is a separate step: generic synthesis first, so
    // inference still sees arithmetic and memories, then the logic that is
    // left over is covered by LUTs or gates.
    if lut.is_some() || args.flag("gates") {
        use reticle::synth::cells::GateLibrary;
        use reticle::synth::techmap::{MapOptions, Target, map_module};

        let library = GateLibrary::generic();
        let target = match lut {
            Some(k) => Target::Lut(k),
            None => Target::Gates(&library),
        };
        let map_options = MapOptions {
            target,
            ..MapOptions::default()
        };
        let ids: Vec<_> = design.modules.iter().map(|(id, _)| id).collect();
        for id in ids {
            let stats = map_module(&mut design.modules[id], &map_options);
            if args.flag("report") {
                eprintln!(
                    "map {}: {} cells, depth {}, area {:.0} (aig {} -> {} nodes)",
                    design.modules[id].name,
                    stats.cells,
                    stats.depth,
                    stats.area,
                    stats.before.nodes,
                    stats.after.nodes
                );
            }
        }
    }
    if let Err(message) = write_out(args.option("output"), &design.to_text()) {
        eprintln!("error: {message}");
        return Ok(Outcome::Failed);
    }
    if !args.flag("quiet") {
        eprintln!("note: synthesised {} module(s)", design.modules.len());
    }
    Ok(Outcome::Ok)
}

/// `reticle fpga`: the whole target flow, ending in nextpnr's inputs.
fn fpga(args: &Args) -> Result<Outcome, ArgError> {
    use reticle::fpga::{
        Constraints, FpgaOptions, PnrRoute, builtin_devices, check_nextpnr_json, export_nextpnr,
        export_vendor, pnr_route, synthesize_for, target,
    };

    if args.flag("list-devices") {
        for device in builtin_devices().devices().iter() {
            println!(
                "{:<16} {:<10} {}-LUT",
                device.name, device.family, device.lut_size
            );
        }
        return Ok(Outcome::Ok);
    }

    let Some(device_name) = args.option("device") else {
        return Ok(Outcome::Usage(
            "no device given; pass --device, or --list-devices to see them".into(),
        ));
    };
    let Some(device) = target(device_name) else {
        let known: Vec<&str> = builtin_devices()
            .devices()
            .iter()
            .map(|d| d.name.as_str())
            .collect();
        return Ok(Outcome::Usage(format!(
            "unknown device `{device_name}`; known devices are {}",
            known.join(", ")
        )));
    };

    let (mut design, mut map) = match load_design(args)? {
        Ok(pair) => pair,
        Err(outcome) => return Ok(outcome),
    };
    let Some(top) = design.top else {
        return Ok(Outcome::Usage(
            "the design names no top module; pass --top".into(),
        ));
    };

    // Constraints come from the file first, then from attributes in the
    // source, so an attribute cannot silently override an explicit pin.
    let mut diags = Diagnostics::new();
    let mut constraints = Constraints::new();
    if let Some(path) = args.option("constraints") {
        let Some(id) = load(&mut map, path, &mut diags) else {
            report(&mut diags, &map);
            return Ok(Outcome::Failed);
        };
        let text = map.file(id).text().to_string();
        constraints = Constraints::parse(&text, id, &mut diags);
    }
    constraints.merge_attrs(&design, top, &mut diags);
    constraints.check(&design, device, &mut diags);
    if report(&mut diags, &map) {
        return Ok(Outcome::Failed);
    }

    let options = FpgaOptions {
        synth: synth_options(args.positionals()),
        ..FpgaOptions::default()
    };
    let mut diags = Diagnostics::new();
    let flow = match synthesize_for(&mut design, top, device, &constraints, &options, &mut diags) {
        Ok(report) => report,
        Err(err) => {
            report(&mut diags, &map);
            eprintln!("error: {err}");
            return Ok(Outcome::Failed);
        }
    };
    let failed = report(&mut diags, &map);
    if args.flag("report") {
        eprint!("{}", flow.to_text());
    }
    if failed {
        return Ok(Outcome::Failed);
    }

    // Check the netlist against the device before writing it, so a file
    // nextpnr would reject never reaches the disk unnoticed.
    let problems = check_nextpnr_json(&design, top, device, &constraints);
    if !problems.is_empty() {
        for problem in &problems {
            eprintln!("error: {problem}");
        }
        return Ok(Outcome::Failed);
    }

    let dir = std::path::Path::new(args.option("output-dir").unwrap_or("."));
    let top_name = design.modules[top].name.as_str().to_string();

    // Which files to write is the one part of a family that is not data:
    // the open families go to nextpnr, the 7 series to Vivado. A family
    // with no route at all falls through to the nextpnr export, whose
    // error names it.
    let files: Vec<(std::path::PathBuf, String)>;
    let command: Vec<String>;
    match pnr_route(&device.family) {
        Some(PnrRoute::Vendor) => {
            let inputs = match export_vendor(&design, top, device, &constraints) {
                Ok(inputs) => inputs,
                Err(err) => {
                    eprintln!("error: {err}");
                    return Ok(Outcome::Failed);
                }
            };
            // The file names come from the export, not from `top_name`:
            // a module specialised by `--param` is called something like
            // `soc_top$CLK_DIV_868`, and the export flattens that into a
            // name a shell and a file system can both hold.
            let stem = inputs.stem;
            files = vec![
                (dir.join(format!("{stem}.v")), inputs.verilog),
                (dir.join(format!("{stem}.xdc")), inputs.xdc),
                (dir.join(format!("{stem}.tcl")), inputs.script),
            ];
            command = inputs.args;
        }
        _ => {
            let inputs = match export_nextpnr(&design, top, device, &constraints) {
                Ok(inputs) => inputs,
                Err(err) => {
                    eprintln!("error: {err}");
                    return Ok(Outcome::Failed);
                }
            };
            files = vec![
                (dir.join(format!("{top_name}.json")), inputs.json),
                (dir.join(&inputs.constraints_name), inputs.pcf_or_lpf),
            ];
            command = inputs.args;
        }
    }
    for (path, text) in &files {
        if let Err(err) = std::fs::write(path, text) {
            eprintln!("error: cannot write `{}`: {err}", path.display());
            return Ok(Outcome::Failed);
        }
    }
    if let Some(path) = args.option("netlist")
        && let Err(message) = write_out(Some(path), &design.to_text())
    {
        eprintln!("error: {message}");
        return Ok(Outcome::Failed);
    }

    // The 7-series bitstream, which needs a chip database the user
    // supplies. Everything above this point is the same flow every
    // family takes.
    if let Some(path) = args.option("bitstream") {
        match write_xc7_bitstream(args, &design, top, device, &constraints, path) {
            Ok(note) => {
                if !args.flag("quiet") {
                    eprint!("{note}");
                }
            }
            Err(message) => {
                eprintln!("error: {message}");
                return Ok(Outcome::Failed);
            }
        }
    }

    if !args.flag("quiet") {
        let names: Vec<String> = files
            .iter()
            .map(|(path, _)| path.display().to_string())
            .collect();
        eprintln!("note: wrote {}", names.join(", "));
        eprintln!("note: place and route with: {}", command.join(" "));
    }
    Ok(Outcome::Ok)
}

/// Writes a Xilinx 7-series `.bit` from a real chip database.
///
/// # NOTHING PRODUCED HERE HAS BEEN LOADED INTO A PART
///
/// What this establishes is that the container is the one UG470
/// describes, that its IDCODE is the one the device file and the
/// database agree on, and that every frame it writes is at an address
/// the part really has. It does not establish that the frames configure
/// anything, and the note it returns says so when the design is not
/// routed.
///
/// The database never comes from the library: it is read here, through
/// a `FileProvider`, from a directory the user named.
fn write_xc7_bitstream(
    args: &Args,
    design: &reticle::ir::Design,
    top: reticle::ir::ModuleId,
    device: &reticle::fpga::Device,
    constraints: &reticle::fpga::Constraints,
    path: &str,
) -> Result<String, String> {
    use reticle::fpga::place::{PlaceOptions, place};
    use reticle::fpga::route::{RouteOptions, route};
    use reticle::fpga::xc7::{self, BitHeader};
    use reticle::fpga::xray::{GridRegion, XrayDatabase, XrayOptions};
    use reticle::fpga::{Netlist, bitstream};

    let root = args
        .option("chipdb")
        .map(str::to_owned)
        .or_else(|| std::env::var("RETICLE_CHIPDB").ok())
        .ok_or_else(|| {
            "`--bitstream` needs a chip database: pass --chipdb <dir> or set RETICLE_CHIPDB. \
             Reticle never fetches one; see docs/fpga-xray.md"
                .to_owned()
        })?;

    let files = DiskFiles::for_sources(&[]);
    let mut options = XrayOptions::new();
    if let Some(text) = args.option("region") {
        let numbers: Result<Vec<u32>, _> = text.split(',').map(str::parse::<u32>).collect();
        match numbers {
            Ok(n) if n.len() == 4 => {
                options.region = Some(GridRegion::new(n[0], n[1], n[2], n[3]));
            }
            _ => return Err(format!("`--region {text}` is not `x0,y0,x1,y1`")),
        }
    }

    let db =
        XrayDatabase::open(&files, &root, &device.name, &options).map_err(|e| e.to_string())?;

    // Without an explicit region, load the fabric around the pins the
    // constraints name. The whole die does not fit; the loader would
    // refuse it with the numbers, which is not a useful default.
    if options.region.is_none() {
        let pins: Vec<String> = constraints.pins.iter().map(|p| p.pin.clone()).collect();
        options.region = db
            .region_for_pins(&files, &pins, 12)
            .map_err(|e| e.to_string())?;
        if options.region.is_none() {
            return Err(
                "no constrained pin names a site this database has, so there is nowhere to \
                 load the fabric around; pass --region x0,y0,x1,y1"
                    .to_owned(),
            );
        }
    }

    let fabric = db.load(&files, &options).map_err(|e| e.to_string())?;

    // Refuse before writing anything if the database is for another die.
    if let Some(idcode) = device.idcode {
        fabric.check_idcode(idcode).map_err(|e| e.to_string())?;
    }

    let graph = fabric.arch.build_graph();
    let netlist = Netlist::build(design, top, device, &graph).map_err(|e| e.to_string())?;
    let (placement, placement_report) = place(
        &netlist,
        &fabric.arch,
        &graph,
        constraints,
        &PlaceOptions::default(),
    )
    .map_err(|e| e.to_string())?;

    // Routing on this fabric needs bel pins, and the database does not
    // ship them. Attempt it, and say plainly when it does not happen
    // rather than writing a file that implies it did.
    let (routing, routed) = match route(&netlist, &graph, &placement, &RouteOptions::default()) {
        Ok((routing, report)) => (routing, report.signals),
        Err(_) => (reticle::fpga::Routing::new(netlist.signals.len()), 0),
    };

    let tiles = bitstream::generate(
        design,
        top,
        &fabric.arch,
        &graph,
        &netlist,
        &placement,
        &routing,
    )
    .map_err(|e| e.to_string())?;
    let frames = xc7::frames_from_bitstream(&fabric.part, &tiles, &fabric.frames)
        .map_err(|e| e.to_string())?;

    let name = design.modules[top].name.as_str();
    let header = BitHeader::new(
        format!("{name};UserID=0XFFFFFFFF;Version=reticle"),
        fabric
            .part
            .name
            .trim_start_matches("xc")
            .trim_end_matches("-1"),
    );
    let bytes = xc7::write_bit(&header, &fabric.part, &frames).map_err(|e| e.to_string())?;
    std::fs::write(path, &bytes).map_err(|e| format!("cannot write `{path}`: {e}"))?;

    let mut note = String::new();
    note.push_str(&fabric.stats.to_text());
    note.push_str(&format!(
        "note: wrote {path}, {} byte(s), {} frame(s), {} configuration bit(s) set\n",
        bytes.len(),
        fabric.part.layout.frames(),
        frames.ones()
    ));
    note.push_str(&format!(
        "note: {} instance(s) placed on real sites, {} held at a package pin, \
         {} pin(s) the fabric gives no wire\n",
        netlist.instances.len(),
        placement_report.fixed,
        placement_report.off_fabric
    ));
    note.push_str(&format!(
        "note: {routed} of {} signal(s) routed\n",
        netlist.signals.len()
    ));
    if routed < netlist.signals.len() || placement_report.off_fabric > 0 {
        note.push_str(
            "warning: the design is NOT FULLY ROUTED. The chip database ships bit positions but \
             not the tile-type wire lists that say which wire a bel pin reaches, so a bel has no \
             pins and a signal has no path to one. What was written is a structurally valid \
             bitstream that configures nothing.\n",
        );
    }
    note.push_str(
        "warning: NOTHING PRODUCED BY THIS FLOW HAS BEEN LOADED INTO A PART. See \
         docs/fpga-xray.md for what is and is not established.\n",
    );
    Ok(note)
}

/// `reticle emit`: write the design in another format.
fn emit_cmd(args: &Args) -> Result<Outcome, ArgError> {
    let name = args.option("format").unwrap_or("verilog");
    let Some(format) = Format::from_name(name) else {
        return Ok(Outcome::Usage(format!(
            "`--format {name}` is not one of verilog, vhdl, json, blif, edif"
        )));
    };
    let (design, _map) = match load_design(args)? {
        Ok(pair) => pair,
        Err(outcome) => return Ok(outcome),
    };
    match emit::emit(&design, format) {
        Ok(text) => {
            if let Err(message) = write_out(args.option("output"), &text) {
                eprintln!("error: {message}");
                return Ok(Outcome::Failed);
            }
            Ok(Outcome::Ok)
        }
        Err(err) => {
            eprintln!("error: {err}");
            Ok(Outcome::Failed)
        }
    }
}

/// `reticle sim`: run the design and print what it says.
fn sim(args: &Args) -> Result<Outcome, ArgError> {
    use reticle::sim::{SimOptions, Simulator, Status};

    let until = args.u64_option("until")?;
    let seed = args.u64_option("seed")?;
    let (design, mut map) = match load_design(args)? {
        Ok(pair) => pair,
        Err(outcome) => return Ok(outcome),
    };

    let mut options = SimOptions {
        top: args.option("top").map(str::to_string),
        coverage: args.option("coverage").is_some(),
        // Without this every `$readmemh` fails: the simulator performs no
        // I/O of its own and reads files only through a provider.
        files: Some(Box::new(DiskFiles::for_sources(args.positionals()))),
        ..SimOptions::default()
    };
    if let Some(seed) = seed {
        options.seed = seed;
    }

    let mut sim = match Simulator::new(&design, options) {
        Ok(sim) => sim,
        Err(mut errors) => {
            report(&mut errors, &map);
            return Ok(Outcome::Failed);
        }
    };
    // Assertions are added before the run so they see every clock edge.
    let mut assertions: Vec<String> = Vec::new();
    if let Some(text) = args.option("assert") {
        assertions.push(text.to_string());
    }
    if let Some(path) = args.option("assert-file") {
        match std::fs::read_to_string(path) {
            Ok(text) => assertions.extend(
                text.lines()
                    .map(str::trim)
                    .filter(|line| !line.is_empty() && !line.starts_with('#'))
                    .map(str::to_string),
            ),
            Err(err) => {
                eprintln!("error: cannot read `{path}`: {err}");
                return Ok(Outcome::Failed);
            }
        }
    }
    if !assertions.is_empty() {
        // The text came from the command line, not a file in the map, so
        // give it a span into a synthetic source so errors still render.
        let mut added = Diagnostics::new();
        let joined = assertions.join("\n");
        let id = match map.add("<--assert>".to_string(), joined.clone()) {
            Ok(id) => id,
            Err(err) => {
                eprintln!("error: cannot hold the assertion text: {err}");
                return Ok(Outcome::Failed);
            }
        };
        let mut at = 0u32;
        for text in &assertions {
            let len = u32::try_from(text.len()).unwrap_or(0);
            let span = reticle::source::Span::new(id, at, at + len);
            at += len + 1;
            if let Err(diag) = sim.add_assertion_text(text, span) {
                added.push(diag);
            }
        }
        if report(&mut added, &map) {
            return Ok(Outcome::Failed);
        }
    }

    if args.flag("interactive") {
        return Ok(interactive(sim));
    }

    let dump = args.option("vcd");
    if dump.is_some() {
        sim.enable_vcd();
    }
    // The FST capture records through the change callbacks, so it has to be
    // installed before the run and handed back at dump time.
    let fst = args.option("fst").map(|path| (path, sim.enable_fst()));

    match until {
        Some(time) => sim.run_until(time),
        None => sim.run(),
    }

    print!("{}", sim.output());
    let mut messages = sim.take_messages();
    let mut failed = report(&mut messages, &map);

    if sim.assertion_count() > 0 {
        let mut broken = 0usize;
        for result in sim.assertion_results() {
            let verdict = if result.kind == reticle::sim::assertion::property::DirectiveKind::Cover
            {
                format!("{} match(es)", result.passes)
            } else if result.failures > 0 {
                broken += 1;
                format!("FAILED {} time(s)", result.failures)
            } else {
                format!("held over {} attempt(s)", result.attempts)
            };
            eprintln!("  {}: {verdict}", result.name);
        }
        if broken > 0 {
            failed = true;
        }
    }

    if let Some(path) = args.option("coverage") {
        match sim.coverage() {
            Some(report_) => {
                let text = if path.ends_with(".info") {
                    report_.to_lcov(&map)
                } else {
                    report_.render(&map)
                };
                if let Err(message) = write_out(Some(path), &text) {
                    eprintln!("error: {message}");
                    return Ok(Outcome::Failed);
                }
            }
            None => eprintln!("warning: the run collected no coverage"),
        }
    }

    if let Some(path) = dump {
        let mut text = String::new();
        if sim.dump_vcd(&mut text).is_err() {
            eprintln!("error: could not render the waveform");
            return Ok(Outcome::Failed);
        }
        if let Err(message) = write_out(Some(path), &text) {
            eprintln!("error: {message}");
            return Ok(Outcome::Failed);
        }
    }

    if let Some((path, capture)) = fst {
        let mut bytes = Vec::new();
        if sim.dump_fst(&capture, &mut bytes).is_err() {
            eprintln!("error: could not render the FST waveform");
            return Ok(Outcome::Failed);
        }
        if let Err(err) = std::fs::write(path, &bytes) {
            eprintln!("error: cannot write `{path}`: {err}");
            return Ok(Outcome::Failed);
        }
    }

    if !args.flag("quiet") {
        let reason = match (sim.status(), until) {
            (Status::Finished, _) => "finished",
            (Status::Stopped, _) => "stopped at $stop",
            (Status::Running, Some(_)) => "reached the time limit",
            (Status::Running, None) => "ran out of events",
        };
        eprintln!("note: {reason} at time {}", sim.time());
    }
    Ok(if failed { Outcome::Failed } else { Outcome::Ok })
}

/// `reticle timing`: slack over the netlist, and crossing analysis.
fn timing(args: &Args) -> Result<Outcome, ArgError> {
    use reticle::fpga::Constraints;
    use reticle::timing::cdc::analyze_cdc_with;
    use reticle::timing::delay::{DelayModel, FpgaModel, UnitModel};
    use reticle::timing::graph::flatten_for_timing;
    use reticle::timing::sta::{TimingOptions, TimingSpec, analyze_with};

    let paths = args.u32_option("paths")?;
    let period = match args.option("period") {
        None => 10.0_f64,
        Some(text) => match text.parse::<f64>() {
            Ok(value) if value > 0.0 => value,
            _ => {
                return Ok(Outcome::Usage(format!(
                    "`--period {text}` is not a positive number of nanoseconds"
                )));
            }
        },
    };

    let (mut design, mut map) = match load_design(args)? {
        Ok(pair) => pair,
        Err(outcome) => return Ok(outcome),
    };

    // Timing needs the cell form, so synthesise unless the design already
    // has one. A design still holding processes has no arcs to walk.
    let mut diags = Diagnostics::new();
    let has_processes = design.modules.iter().any(|(_, m)| !m.processes.is_empty());
    if has_processes {
        let options = synth_options(args.positionals());
        reticle::synth::run(&mut design, &options, &mut diags);
        if report(&mut diags, &map) {
            return Ok(Outcome::Failed);
        }
    }

    let top = match args.option("top") {
        Some(name) => match design.module_by_name(name) {
            Some(id) => id,
            None => return Ok(Outcome::Usage(format!("no module named `{name}`"))),
        },
        None => match design.top {
            Some(id) => id,
            None => {
                return Ok(Outcome::Usage(
                    "the design names no top module; pass --top".into(),
                ));
            }
        },
    };

    let module = match flatten_for_timing(&design, top) {
        Ok(module) => module,
        Err(mut errors) => {
            report(&mut errors, &map);
            return Ok(Outcome::Failed);
        }
    };

    let mut diags = Diagnostics::new();
    let mut constraints = Constraints::new();
    if let Some(path) = args.option("constraints") {
        let Some(id) = load(&mut map, path, &mut diags) else {
            report(&mut diags, &map);
            return Ok(Outcome::Failed);
        };
        let text = map.file(id).text().to_string();
        constraints = Constraints::parse(&text, id, &mut diags);
        if report(&mut diags, &map) {
            return Ok(Outcome::Failed);
        }
    }
    let spec = TimingSpec::from_constraints(&constraints);

    if args.flag("cdc") {
        let report_ = analyze_cdc_with(&module, &spec);
        print!("{}", report_.render_sources(&map));
        return Ok(if report_.has_errors() {
            Outcome::Failed
        } else {
            Outcome::Ok
        });
    }

    let mut options = TimingOptions {
        check_setup: !args.flag("hold"),
        check_hold: args.flag("hold"),
        default_period: period,
        ..TimingOptions::default()
    };
    if let Some(n) = paths {
        options.max_paths = n as usize;
    }

    // The delay numbers for a device family are placeholders, which the
    // report says; the unit model is honest about being a unit model.
    let fpga_model;
    let unit_model;
    let model: &dyn DelayModel = match args.option("device") {
        Some(_) => {
            fpga_model = FpgaModel::placeholders();
            &fpga_model
        }
        None => {
            unit_model = UnitModel::new();
            &unit_model
        }
    };

    let report_ = analyze_with(&module, &options, model, &spec);
    if args.flag("summary") {
        print!("{}", report_.render_summary());
    } else {
        print!("{}", report_.render_sources(&map));
    }
    Ok(if report_.has_violations() {
        Outcome::Failed
    } else {
        Outcome::Ok
    })
}

/// Serves `$readmemh` and friends from the filesystem.
///
/// The library never touches the disk, so it asks for files through this.
/// A relative path is tried against each of the directories the design's
/// sources came from, in order, and then the current directory, which is
/// how a simulator resolves `$readmemh("prog.hex")` written next to the
/// testbench that loads it.
struct DiskFiles {
    roots: Vec<std::path::PathBuf>,
}

impl DiskFiles {
    /// A provider rooted at the directories of `sources`.
    fn for_sources(sources: &[String]) -> DiskFiles {
        let mut roots: Vec<std::path::PathBuf> = Vec::new();
        for source in sources {
            let dir = Path::new(source)
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new("."))
                .to_path_buf();
            if !roots.contains(&dir) {
                roots.push(dir);
            }
        }
        roots.push(std::path::PathBuf::from("."));
        DiskFiles { roots }
    }
}

impl reticle::sim::FileProvider for DiskFiles {
    fn read_file(&self, path: &str) -> Option<String> {
        let wanted = Path::new(path);
        if wanted.is_absolute() {
            return std::fs::read_to_string(wanted).ok();
        }
        self.roots
            .iter()
            .find_map(|root| std::fs::read_to_string(root.join(wanted)).ok())
    }
}

/// Synthesis options that can read the files a design names.
///
/// Synthesis turns `$readmemh` into a memory's initial contents, and like
/// the simulator it reads files only through a provider it is handed.
/// Without one a ROM loaded that way comes out of synthesis empty, with a
/// warning; every command that synthesises goes through here so none of
/// them forgets.
fn synth_options(sources: &[String]) -> reticle::synth::SynthOptions {
    reticle::synth::SynthOptions {
        files: Some(std::rc::Rc::new(DiskFiles::for_sources(sources))),
        ..reticle::synth::SynthOptions::default()
    }
}

/// `reticle lsp`: the language server over standard input and output.
fn lsp(args: &Args) -> Result<Outcome, ArgError> {
    if !args.positionals().is_empty() {
        return Ok(Outcome::Usage(
            "`lsp` takes no files; the editor opens them".into(),
        ));
    }
    let stdin = std::io::stdin();
    let mut input = stdin.lock();
    let stdout = std::io::stdout();
    let mut output = stdout.lock();
    match reticle::lsp::stdio::serve(&mut input, &mut output) {
        Ok(()) => Ok(Outcome::Ok),
        Err(err) => {
            eprintln!("error: the language server stopped: {err}");
            Ok(Outcome::Failed)
        }
    }
}

/// Drives an interactive session from standard input.
///
/// Each line is one command. A prompt is written only when standard input
/// is a terminal, so the same command works from a script or a pipe
/// without prompts interleaved into the transcript.
fn interactive(sim: reticle::sim::Simulator<'_>) -> Outcome {
    use std::io::{BufRead, IsTerminal, Write};

    let mut session = reticle::sim::interactive::Session::new(sim);
    let prompt = std::io::stdin().is_terminal();
    let stdin = std::io::stdin();
    let mut failed = false;
    loop {
        if prompt {
            print!("reticle> ");
            let _ = std::io::stdout().flush();
        }
        let mut line = String::new();
        match stdin.lock().read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {}
            Err(err) => {
                eprintln!("error: cannot read a command: {err}");
                return Outcome::Failed;
            }
        }
        match session.execute(&line) {
            Ok(response) => {
                let text = response.text();
                let text = text.trim_end_matches('\n');
                if !text.is_empty() {
                    println!("{text}");
                }
                if response.quit {
                    break;
                }
            }
            Err(err) => {
                // A mistyped command is not fatal at a prompt, but a script
                // that contained one should not report success.
                eprintln!("error: {err}");
                failed = true;
            }
        }
    }
    if failed { Outcome::Failed } else { Outcome::Ok }
}

/// `reticle verify`: bounded model checking, then induction.
fn verify(args: &Args) -> Result<Outcome, ArgError> {
    use reticle::formal::{InitMode, Verdict, VerifyOptions};

    let depth = args.u32_option("depth")?;
    let max_k = args.u32_option("max-k")?;
    let init = match args.option("init") {
        None => InitMode::Reset,
        Some("reset") => InitMode::Reset,
        Some("zero") => InitMode::Zero,
        Some("free") => InitMode::Free,
        Some(other) => {
            return Ok(Outcome::Usage(format!(
                "`--init {other}` is not one of reset, zero, free"
            )));
        }
    };

    let (design, map) = match load_design(args)? {
        Ok(pair) => pair,
        Err(outcome) => return Ok(outcome),
    };

    let module = match args.option("top") {
        Some(name) => match design.module_by_name(name) {
            Some(id) => id,
            None => return Ok(Outcome::Usage(format!("no module named `{name}`"))),
        },
        None => match design.top {
            Some(id) => id,
            None => {
                return Ok(Outcome::Usage(
                    "the design names no top module; pass --top".into(),
                ));
            }
        },
    };

    let mut options = VerifyOptions {
        init,
        ..VerifyOptions::default()
    };
    if let Some(depth) = depth {
        options.depth = depth;
    }
    if let Some(k) = max_k {
        options.max_k = k;
    }

    let mut report_ = reticle::formal::verify(&design, module, &options);
    print!("{}", report_.render());
    let failed = report(&mut report_.diags, &map);

    if let (Some(path), Some(trace)) = (args.option("trace"), report_.trace.as_ref()) {
        let vcd = trace.to_vcd(design.modules[module].name.as_str());
        if let Err(message) = write_out(Some(path), &vcd) {
            eprintln!("error: {message}");
            return Ok(Outcome::Failed);
        }
    }

    let refuted = matches!(report_.verdict, Verdict::Failed { .. } | Verdict::Unknown);
    Ok(if failed || refuted {
        Outcome::Failed
    } else {
        Outcome::Ok
    })
}

/// `reticle viewer`: render the design as a browsable static site.
fn viewer(args: &Args) -> Result<Outcome, ArgError> {
    use reticle::viewer::{ViewerOptions, render};

    let max_nodes = args.u64_option("max-nodes")?;
    let (mut design, map) = match load_design(args)? {
        Ok(pair) => pair,
        Err(outcome) => return Ok(outcome),
    };
    if let Some(top) = args.option("top") {
        match design.module_by_name(top) {
            Some(id) => design.top = Some(id),
            None => return Ok(Outcome::Usage(format!("no module named `{top}`"))),
        }
    }
    if args.flag("synth") {
        let mut diags = Diagnostics::new();
        let options = synth_options(args.positionals());
        reticle::synth::run(&mut design, &options, &mut diags);
        if report(&mut diags, &map) {
            return Ok(Outcome::Failed);
        }
    }

    let mut options = ViewerOptions {
        comments: collect_comments(&map),
        schematics: !args.flag("no-schematic"),
        docs: !args.flag("no-doc"),
        sources: !args.flag("no-source"),
        ..ViewerOptions::default()
    };
    options.title = match (args.option("title"), design.top_module()) {
        (Some(title), _) => title.to_string(),
        (None, Some(top)) => format!("{} \u{2014} design reference", top.name),
        (None, None) => "Design reference".to_string(),
    };
    if let Some(names) = args.option("module") {
        options.only = names
            .split(',')
            .map(|name| name.trim().to_string())
            .filter(|name| !name.is_empty())
            .collect();
    }
    if let Some(n) = max_nodes {
        options.max_nodes = usize::try_from(n).unwrap_or(usize::MAX);
    }

    let site = render(&design, &map, &options);
    let dir = Path::new(args.option("output-dir").unwrap_or("viewer"));
    for (path, contents) in &site.files {
        let target = dir.join(path);
        if let Some(parent) = target.parent()
            && let Err(err) = std::fs::create_dir_all(parent)
        {
            eprintln!("error: cannot create `{}`: {err}", parent.display());
            return Ok(Outcome::Failed);
        }
        if let Err(err) = std::fs::write(&target, contents) {
            eprintln!("error: cannot write `{}`: {err}", target.display());
            return Ok(Outcome::Failed);
        }
    }
    if !args.flag("quiet") {
        eprintln!(
            "wrote {} page{}; open {}",
            site.len(),
            if site.len() == 1 { "" } else { "s" },
            dir.join("index.html").display()
        );
    }
    Ok(Outcome::Ok)
}

/// Lexes every HDL source in the map for its comments.
///
/// Neither frontend attaches comments to its tree; both keep them in a
/// side table the lexer fills, which is what the documentation view reads.
/// Re-lexing is cheap next to elaboration, and it keeps `load_design`
/// unchanged for every other command.
fn collect_comments(map: &SourceMap) -> reticle::viewer::Comments {
    let mut comments = reticle::viewer::Comments::new();
    for (id, file) in map.files() {
        match classify(file.name()) {
            Some(Input::Verilog) => {
                let dialect = extension(file.name())
                    .as_deref()
                    .and_then(reticle::verilog::Dialect::for_extension)
                    .unwrap_or_default();
                let mut diags = Diagnostics::new();
                let lexed =
                    reticle::verilog::Lexer::new(file.text(), id, dialect, &mut diags).run();
                comments.add_verilog(&lexed);
            }
            Some(Input::Vhdl) => {
                let mut diags = Diagnostics::new();
                let standard = reticle::vhdl::Standard::default();
                let lexed = reticle::vhdl::Lexer::new(file.text(), id, standard).lex(&mut diags);
                comments.add_vhdl(&lexed);
            }
            Some(Input::Rtl) | None => {}
        }
    }
    comments
}
