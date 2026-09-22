//! Corpus tests for the SAT solver on DIMACS files.
//!
//! Every `testdata/sat/*.cnf` is solved; a `c expect: SAT` or `c expect:
//! UNSAT` comment line in the file states the expected answer. A `SAT`
//! answer is additionally checked by evaluating every clause under the
//! returned model, and the model is round-tripped through the `v ... 0`
//! writer.

#![cfg(feature = "formal")]

use std::fs;
use std::path::PathBuf;

use reticle::formal::sat::dimacs::{parse_dimacs, write_model};
use reticle::formal::sat::{Lit, SolveResult, Solver};

fn corpus_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("testdata")
        .join("sat")
}

fn expected(text: &str) -> SolveResult {
    let line = text
        .lines()
        .find_map(|l| l.strip_prefix("c expect:"))
        .expect("file declares `c expect:`");
    match line.trim() {
        "SAT" => SolveResult::Sat,
        "UNSAT" => SolveResult::Unsat,
        other => panic!("unknown expectation `{other}`"),
    }
}

/// Parses `v` lines back into a model.
fn read_model(text: &str) -> Vec<Option<bool>> {
    let mut model = Vec::new();
    for line in text.lines() {
        let body = line.strip_prefix('v').expect("model line starts with v");
        for tok in body.split_whitespace() {
            let n: i32 = tok.parse().expect("literal");
            if let Some(l) = Lit::from_dimacs(n) {
                let i = l.var().index() as usize;
                if model.len() <= i {
                    model.resize(i + 1, None);
                }
                model[i] = Some(l.is_pos());
            }
        }
    }
    model
}

#[test]
fn dimacs_corpus() {
    let mut paths: Vec<PathBuf> = fs::read_dir(corpus_dir())
        .expect("testdata/sat exists")
        .map(|e| e.expect("dir entry").path())
        .filter(|p| p.extension().is_some_and(|e| e == "cnf"))
        .collect();
    paths.sort();
    assert!(!paths.is_empty(), "corpus is not empty");

    for path in paths {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let text = fs::read_to_string(&path).expect("read cnf");
        let want = expected(&text);
        let cnf = parse_dimacs(&text).unwrap_or_else(|e| panic!("{name}: {e}"));
        let mut solver = Solver::from_dimacs(&text).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(solver.num_vars(), cnf.num_vars as usize, "{name}");
        let got = solver.solve();
        assert_eq!(got, want, "{name}");
        if got == SolveResult::Sat {
            let model = read_model(&write_model(solver.model()));
            assert_eq!(model.len(), solver.model().len(), "{name}");
            for clause in &cnf.clauses {
                assert!(
                    clause
                        .iter()
                        .any(|l| model[l.var().index() as usize] == Some(l.is_pos())),
                    "{name}: clause {clause:?} unsatisfied by model"
                );
            }
        }
    }
}
