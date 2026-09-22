//! Simulation golden tests driven by `.rtl` designs.
//!
//! Every `testdata/sim/<name>.rtl` has a companion `<name>.expect` script
//! with one command per line (`#` starts a comment):
//!
//! ```text
//! no-validate                  skip ir::validate (designs with deliberate multiple drivers)
//! file <name>                  provide a file for $readmem*; contents until `end-file`
//! vcd                          enable VCD capture before running
//! coverage                     collect line and toggle coverage
//! run <n><unit>                run for a delay (ns, ps, us, ...) or ticks when no unit
//! run                          run until $finish or no events
//! step                         one time slot
//! set <path> = <literal>       write a net
//! force <path> = <literal>     force a net
//! release <path>               release a net
//! expect <path> = <literal>    compare a net's value (width and bits)
//! expect-mem <path>[<i>] = <literal>
//! expect-time <n><unit>
//! expect-finished
//! expect-output                the $display text so far, lines until `end-output`
//! expect-message <substring>   some diagnostic contains the text
//! expect-vcd                   the captured VCD equals <name>.vcd
//! assert <directive>           add a concurrent assertion (SVA / PSL text)
//! expect-assert <name> k=v ... counts of one directive: attempts, passes,
//!                              vacuous, failures, disabled, incomplete, cycles
//! expect-coverage              the rendered coverage report, until `end-coverage`
//! expect-lcov                  the LCOV `.info` output, until `end-lcov`
//! ```
//!
//! Expectations are hand-written: there is no update mode. Any error
//! diagnostic not covered by an `expect-message` fails the test.

#![cfg(feature = "sim")]

use std::fs;
use std::path::{Path, PathBuf};

use reticle::ir::validate::validate;
use reticle::ir::{Delay, Design, TimeUnit};
use reticle::logic::Logic;
use reticle::sim::{MemoryFiles, SimOptions, Simulator};
use reticle::source::SourceMap;

fn cases() -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/sim");
    let mut paths: Vec<PathBuf> = fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()))
        .map(|entry| entry.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "rtl"))
        .collect();
    paths.sort();
    paths
}

/// Parses `<number><unit>` into a delay; a bare number is in ticks.
fn parse_time(text: &str, sim: &Simulator) -> u64 {
    let digits: String = text.chars().take_while(|c| c.is_ascii_digit()).collect();
    let unit = &text[digits.len()..];
    let value: u64 = digits
        .parse()
        .unwrap_or_else(|_| panic!("bad time `{text}`"));
    if unit.is_empty() {
        return value;
    }
    let unit = TimeUnit::from_name(unit).unwrap_or_else(|| panic!("bad time unit `{text}`"));
    sim.ticks(Delay::new(value, unit))
}

fn literal(text: &str) -> Logic {
    Logic::parse_verilog(text.trim()).unwrap_or_else(|e| panic!("bad literal `{text}`: {e}"))
}

fn same_bits(a: &Logic, b: &Logic) -> bool {
    a.clone().as_unsigned() == b.clone().as_unsigned()
}

fn first_difference(expected: &str, actual: &str) -> String {
    for (i, (e, a)) in expected.lines().zip(actual.lines()).enumerate() {
        if e != a {
            return format!("line {}:\n  expected: {e}\n  actual:   {a}", i + 1);
        }
    }
    format!(
        "expected {} lines, got {}",
        expected.lines().count(),
        actual.lines().count()
    )
}

struct Script {
    validate: bool,
    vcd: bool,
    coverage: bool,
    files: MemoryFiles,
    commands: Vec<(usize, String, Vec<String>)>,
}

/// Splits the script into its preamble (files, flags) and commands.
fn parse_script(text: &str) -> Script {
    let mut script = Script {
        validate: true,
        vcd: false,
        coverage: false,
        files: MemoryFiles::new(),
        commands: Vec::new(),
    };
    let lines: Vec<&str> = text.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i].trim_end();
        i += 1;
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let (cmd, rest) = trimmed.split_once(' ').unwrap_or((trimmed, ""));
        let mut block = Vec::new();
        let terminator = match cmd {
            "file" => Some("end-file"),
            "expect-output" => Some("end-output"),
            "expect-coverage" => Some("end-coverage"),
            "expect-lcov" => Some("end-lcov"),
            _ => None,
        };
        if let Some(end) = terminator {
            loop {
                assert!(i < lines.len(), "missing `{end}`");
                let l = lines[i];
                i += 1;
                if l.trim() == end {
                    break;
                }
                block.push(l.to_owned());
            }
        }
        match cmd {
            "no-validate" => script.validate = false,
            "vcd" => script.vcd = true,
            "coverage" => script.coverage = true,
            "file" => {
                let mut content = block.join("\n");
                content.push('\n');
                script.files.insert(rest.trim(), content);
            }
            _ => script
                .commands
                .push((i, format!("{cmd} {rest}").trim().to_owned(), block)),
        }
    }
    script
}

fn run_case(path: &Path) -> Result<(), String> {
    let name = path.file_stem().unwrap().to_string_lossy().into_owned();
    let text = fs::read_to_string(path).map_err(|e| e.to_string())?;
    let script_text = fs::read_to_string(path.with_extension("expect"))
        .map_err(|e| format!("missing .expect: {e}"))?;
    let script = parse_script(&script_text);
    let mut map = SourceMap::new();
    let file = map.add(format!("{name}.rtl"), text.clone()).unwrap();
    let design = Design::parse_text(&text, file).map_err(|d| d.render(&map))?;
    if script.validate {
        let diags = validate(&design);
        if diags.has_errors() {
            return Err(format!("validation failed\n{}", diags.render(&map)));
        }
    }
    let options = SimOptions {
        files: Some(Box::new(script.files)),
        coverage: script.coverage,
        ..SimOptions::default()
    };
    let mut sim = Simulator::new(&design, options).map_err(|d| d.render(&map))?;
    if script.vcd {
        sim.enable_vcd();
    }
    let mut expected_messages: Vec<String> = Vec::new();
    let mut failures = Vec::new();
    for (line_no, command, block) in &script.commands {
        let fail = |msg: String| format!("expect line {line_no}: {msg}");
        let (cmd, rest) = command.split_once(' ').unwrap_or((command.as_str(), ""));
        let rest = rest.trim();
        match cmd {
            "run" => {
                if rest.is_empty() {
                    sim.run();
                } else {
                    let ticks = parse_time(rest, &sim);
                    sim.run_for(ticks);
                }
            }
            "step" => {
                sim.step();
            }
            "set" | "force" => {
                let (path, value) = rest.split_once('=').expect("set <path> = <value>");
                let net = sim
                    .net(path.trim())
                    .unwrap_or_else(|| panic!("unknown net `{}`", path.trim()));
                if cmd == "set" {
                    sim.set(net, literal(value));
                } else {
                    sim.force(net, literal(value));
                }
            }
            "release" => {
                let net = sim
                    .net(rest)
                    .unwrap_or_else(|| panic!("unknown net `{rest}`"));
                sim.release(net);
            }
            "expect" => {
                let (path, value) = rest.split_once('=').expect("expect <path> = <value>");
                let path = path.trim();
                let Some(net) = sim.net(path) else {
                    failures.push(fail(format!("unknown net `{path}`")));
                    continue;
                };
                let want = literal(value);
                let got = sim.get(net);
                if !same_bits(&want, &got) {
                    failures.push(fail(format!("{path} is {got}, expected {want}")));
                }
            }
            "expect-mem" => {
                let (lhs, value) = rest
                    .split_once('=')
                    .expect("expect-mem <path>[<i>] = <value>");
                let (path, index) = lhs.trim().trim_end_matches(']').split_once('[').unwrap();
                let index: u64 = index.parse().unwrap();
                let Some(mem) = sim.memory(path) else {
                    failures.push(fail(format!("unknown memory `{path}`")));
                    continue;
                };
                let want = literal(value);
                match sim.get_mem(mem, index) {
                    Some(got) if same_bits(&want, &got) => {}
                    got => {
                        failures.push(fail(format!("{path}[{index}] is {got:?}, expected {want}")))
                    }
                }
            }
            "expect-time" => {
                let want = parse_time(rest, &sim);
                if sim.time() != want {
                    failures.push(fail(format!("time is {}, expected {want}", sim.time())));
                }
            }
            "expect-finished" => {
                if !sim.finished() {
                    failures.push(fail("simulation did not finish".into()));
                }
            }
            "expect-output" => {
                let mut want = block.join("\n");
                if !block.is_empty() {
                    want.push('\n');
                }
                let got = sim.take_output();
                if got != want {
                    failures.push(fail(format!(
                        "output differs\n{}",
                        first_difference(&want, &got)
                    )));
                }
            }
            "expect-message" => {
                let hit = sim.messages().iter().any(|d| d.message.contains(rest));
                if !hit {
                    failures.push(fail(format!("no diagnostic contains `{rest}`")));
                }
                expected_messages.push(rest.to_owned());
            }
            "assert" => {
                let span = reticle::source::Span::new(file, 0, 0);
                match sim.add_assertion_text(rest, span) {
                    Ok(_) => {}
                    Err(d) => failures.push(fail(format!("bad assertion: {}", d.message))),
                }
            }
            "expect-assert" => {
                let (name, wanted) = rest.split_once(' ').unwrap_or((rest, ""));
                let results = sim.assertion_results();
                let Some(result) = results.iter().find(|r| r.name == name) else {
                    failures.push(fail(format!("no assertion named `{name}`")));
                    continue;
                };
                for item in wanted.split_whitespace() {
                    let (key, value) = item.split_once('=').expect("expect-assert key=value");
                    let want: u64 = value.parse().expect("expect-assert count");
                    let got = match key {
                        "attempts" => result.attempts,
                        "passes" => result.passes,
                        "vacuous" => result.vacuous,
                        "failures" => result.failures,
                        "disabled" => result.disabled,
                        "incomplete" => result.incomplete,
                        "cycles" => result.cycles,
                        other => panic!("unknown assertion count `{other}`"),
                    };
                    if got != want {
                        failures.push(fail(format!("{name}.{key} is {got}, expected {want}")));
                    }
                }
            }
            "expect-coverage" | "expect-lcov" => {
                let Some(report) = sim.coverage() else {
                    failures.push(fail("coverage was not collected".into()));
                    continue;
                };
                let got = if cmd == "expect-lcov" {
                    report.to_lcov(&map)
                } else {
                    report.render(&map)
                };
                let mut want = block.join("\n");
                if !block.is_empty() {
                    want.push('\n');
                }
                if got != want {
                    failures.push(fail(format!(
                        "{cmd} differs\n{}\n--- actual ---\n{got}",
                        first_difference(&want, &got)
                    )));
                }
            }
            "expect-vcd" => {
                let golden = fs::read_to_string(path.with_extension("vcd"))
                    .map_err(|e| format!("missing .vcd: {e}"))?;
                let got = sim.vcd().unwrap_or("");
                if got != golden {
                    failures.push(fail(format!(
                        "vcd differs\n{}",
                        first_difference(&golden, got)
                    )));
                }
            }
            other => panic!("unknown command `{other}` in {name}.expect"),
        }
    }
    for d in sim.messages().iter().filter(|d| d.is_error()) {
        if !expected_messages.iter().any(|m| d.message.contains(m)) {
            failures.push(format!("unexpected error: {}", d.message));
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("\n"))
    }
}

#[test]
fn simulation_golden_files() {
    let paths = cases();
    assert!(!paths.is_empty(), "no simulation cases found");
    let mut failures = Vec::new();
    for path in paths {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if let Err(e) = run_case(&path) {
            failures.push(format!("{name}:\n{e}"));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n\n"));
}
