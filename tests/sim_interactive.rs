//! Golden transcripts for the interactive session.
//!
//! Every `testdata/sim/interactive/<name>.txt` is a script:
//!
//! ```text
//! # a comment
//! design counter.rtl          the design, from testdata/sim
//! > run 20ns                  a command
//! ran to time 20000           the lines it must produce, in order
//! > print nowhere
//! ! unknown net `nowhere`     an error the command must fail with
//! ```
//!
//! A command with no expected lines must produce none. Expectations are
//! hand-written; a mismatch prints what the session actually said.

#![cfg(feature = "sim")]

use std::fs;
use std::path::{Path, PathBuf};

use reticle::ir::Design;
use reticle::sim::interactive::Session;
use reticle::sim::{SimOptions, Simulator};
use reticle::source::SourceMap;

fn cases() -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/sim/interactive");
    let mut paths: Vec<PathBuf> = fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()))
        .map(|entry| entry.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "txt"))
        .collect();
    paths.sort();
    paths
}

/// One command with the lines it must produce.
struct Step {
    line_no: usize,
    command: String,
    expected: Vec<String>,
}

/// Splits a transcript into its design name and its steps.
fn parse_transcript(text: &str) -> (String, Vec<Step>) {
    let mut design = String::new();
    let mut steps: Vec<Step> = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let trimmed = line.trim_end();
        if trimmed.trim().is_empty() || trimmed.trim_start().starts_with('#') {
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("design ") {
            design = rest.trim().to_owned();
            continue;
        }
        match trimmed.strip_prefix("> ") {
            Some(command) => steps.push(Step {
                line_no: i + 1,
                command: command.to_owned(),
                expected: Vec::new(),
            }),
            None => match steps.last_mut() {
                Some(step) => step.expected.push(trimmed.to_owned()),
                None => panic!("transcript line {} comes before any command", i + 1),
            },
        }
    }
    assert!(!design.is_empty(), "transcript has no `design` line");
    (design, steps)
}

fn run_case(path: &Path) -> Result<(), String> {
    let text = fs::read_to_string(path).map_err(|e| e.to_string())?;
    let (design_name, steps) = parse_transcript(&text);
    let dir = path.parent().expect("transcript directory");
    let design_path = dir.join("..").join(&design_name);
    let design_text = fs::read_to_string(&design_path)
        .map_err(|e| format!("cannot read {}: {e}", design_path.display()))?;
    let mut map = SourceMap::new();
    let file = map.add(design_name.clone(), design_text.clone()).unwrap();
    let design = Design::parse_text(&design_text, file).map_err(|d| d.render(&map))?;
    let sim = Simulator::new(&design, SimOptions::default()).map_err(|d| d.render(&map))?;
    let mut session = Session::new(sim);
    let mut failures = Vec::new();
    let mut quit = false;
    for step in &steps {
        let got: Vec<String> = match session.execute(&step.command) {
            Ok(response) => {
                quit = response.quit;
                response.lines
            }
            Err(e) => vec![format!("! {e}")],
        };
        if got != step.expected {
            failures.push(format!(
                "line {}: `{}`\n  expected:\n{}\n  actual:\n{}",
                step.line_no,
                step.command,
                indent(&step.expected),
                indent(&got)
            ));
        }
    }
    if !quit {
        failures.push("the transcript does not end with `quit`".to_owned());
    }
    // Completion must offer something for every net-taking command.
    let candidates = session.complete("print ");
    if candidates.is_empty() {
        failures.push("completion offered no net names".to_owned());
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("\n"))
    }
}

fn indent(lines: &[String]) -> String {
    if lines.is_empty() {
        return "    <nothing>".to_owned();
    }
    lines
        .iter()
        .map(|l| format!("    {l}"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn interactive_transcripts() {
    let paths = cases();
    assert!(!paths.is_empty(), "no interactive transcripts found");
    let mut failures = Vec::new();
    for path in paths {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if let Err(e) = run_case(&path) {
            failures.push(format!("{name}:\n{e}"));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n\n"));
}
