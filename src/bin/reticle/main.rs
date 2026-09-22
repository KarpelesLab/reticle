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
  check    Parse and check source files
  fmt      Format Verilog and VHDL source
  synth    Synthesise a design to a technology-independent netlist
  fpga     Synthesise and map for an FPGA, and export the place-and-route inputs
  emit     Write a design out in another format
  sim      Simulate a design and print its output
  verify   Prove or refute a design's assertions
  help     Show this message, or `reticle help <command>`
  version  Show the version

`synth`, `emit`, `sim` and `verify` take either Verilog sources (.v, .sv,
elaborated to the IR) or a design already in the `.rtl` IR text format.
VHDL parses and checks today; its path to the IR is still in progress.

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

const CHECK_USAGE: &str = "\
Usage: reticle check [files...]

Parses each file and reports diagnostics. `.rtl` files are additionally
validated against the IR's structural rules.

Options:
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
  --output-dir <d>   Write <top>.json and the constraints here (default: .)
  --netlist <file>   Also write the mapped design in the .rtl text format
  --report           Print the mapping report to stderr
  --quiet            Suppress the summary line
";

const SYNTH_USAGE: &str = "\
Usage: reticle synth [options] <design.rtl>

Lowers processes to cells, infers flip-flops, latches and memories, then
optimises. Writes the resulting netlist in the `.rtl` format.

Options:
  --output <file>     Write the netlist here (default: stdout)
  --top <module>      Treat this module as the top
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
";

const SIM_USAGE: &str = "\
Usage: reticle sim [options] <design.rtl>

Runs the design until $finish, until no events remain, or until the time
limit. Text from $display and report statements goes to stdout.

Options:
  --top <module>   Instantiate this module as the root
  --until <ticks>  Stop at this time (in the design's precision)
  --seed <n>       Seed for $random (default 1)
  --vcd <file>     Write a VCD waveform here
  --fst <file>     Write a GTKWave FST waveform here
  --quiet          Suppress the summary line
";

const VERIFY_USAGE: &str = "\
Usage: reticle verify [options] <design.rtl>

Bounded model checking followed by k-induction over the nets marked with
the `formal_assert`, `formal_assume` and `formal_cover` attributes.

Options:
  --top <module>   Check this module (default: the design's top)
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
        "check" => run(check, rest, CHECK_USAGE),
        "fmt" => run(fmt, rest, FMT_USAGE),
        "synth" => run(synth, rest, SYNTH_USAGE),
        "fpga" => run(fpga, rest, FPGA_USAGE),
        "emit" => run(emit_cmd, rest, EMIT_USAGE),
        "sim" => run(sim, rest, SIM_USAGE),
        "verify" => run(verify, rest, VERIFY_USAGE),
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
        Some("check") => CHECK_USAGE,
        Some("fmt") => FMT_USAGE,
        Some("synth") => SYNTH_USAGE,
        Some("fpga") => FPGA_USAGE,
        Some("emit") => EMIT_USAGE,
        Some("sim") => SIM_USAGE,
        Some("verify") => VERIFY_USAGE,
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
    if std::ptr::eq(usage, CHECK_USAGE) {
        Spec {
            options: &[],
            flags: &["quiet"],
        }
    } else if std::ptr::eq(usage, FMT_USAGE) {
        Spec {
            options: &["width", "indent"],
            flags: &["write", "check", "diff"],
        }
    } else if std::ptr::eq(usage, SYNTH_USAGE) {
        Spec {
            options: &["output", "top", "fsm", "max-iterations", "lut"],
            flags: &["report", "quiet", "gates", "verify"],
        }
    } else if std::ptr::eq(usage, FPGA_USAGE) {
        Spec {
            options: &["device", "constraints", "top", "output-dir", "netlist"],
            flags: &["list-devices", "report", "quiet"],
        }
    } else if std::ptr::eq(usage, EMIT_USAGE) {
        Spec {
            options: &["format", "output"],
            flags: &[],
        }
    } else if std::ptr::eq(usage, SIM_USAGE) {
        Spec {
            options: &["top", "until", "seed", "vcd", "fst"],
            flags: &["quiet"],
        }
    } else {
        Spec {
            options: &["top", "depth", "max-k", "init", "trace"],
            flags: &[],
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
    if kinds.contains(&Input::Vhdl) {
        return Ok(Err(Outcome::Usage(
            "VHDL does not reach the IR yet; use `reticle check` on it for now".into(),
        )));
    }
    if kinds.windows(2).any(|w| w[0] != w[1]) {
        return Ok(Err(Outcome::Usage(
            "cannot mix .rtl with Verilog sources; an .rtl file is already a whole design".into(),
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
    let refs: Vec<&reticle::verilog::ast::SourceFile> = files.iter().collect();
    let mut diags = Diagnostics::new();
    let design = reticle::verilog::elaborate(&refs, &options, &mut diags);
    let failed = report(&mut diags, &map);
    match design {
        Some(design) if !failed => Ok(Ok((design, map))),
        _ => Ok(Err(Outcome::Failed)),
    }
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

    let mut options = SynthOptions::default();
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
        Constraints, FpgaOptions, builtin_devices, check_nextpnr_json, export_nextpnr,
        synthesize_for, target,
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

    let options = FpgaOptions::default();
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

    let inputs = match export_nextpnr(&design, top, device, &constraints) {
        Ok(inputs) => inputs,
        Err(err) => {
            eprintln!("error: {err}");
            return Ok(Outcome::Failed);
        }
    };

    let dir = std::path::Path::new(args.option("output-dir").unwrap_or("."));
    let top_name = design.modules[top].name.as_str().to_string();
    let json_path = dir.join(format!("{top_name}.json"));
    let constraints_path = dir.join(&inputs.constraints_name);
    for (path, text) in [
        (&json_path, &inputs.json),
        (&constraints_path, &inputs.pcf_or_lpf),
    ] {
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

    if !args.flag("quiet") {
        eprintln!(
            "note: wrote {} and {}",
            json_path.display(),
            constraints_path.display()
        );
        eprintln!("note: place and route with: {}", inputs.args.join(" "));
    }
    Ok(Outcome::Ok)
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
    let (design, map) = match load_design(args)? {
        Ok(pair) => pair,
        Err(outcome) => return Ok(outcome),
    };

    let mut options = SimOptions {
        top: args.option("top").map(str::to_string),
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
    let failed = report(&mut messages, &map);

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
