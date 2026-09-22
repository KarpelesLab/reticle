//! DIMACS CNF reading and writing.
//!
//! The format is the lingua franca of SAT solvers:
//!
//! ```text
//! c a comment
//! p cnf <variables> <clauses>
//! 1 -3 0
//! 2 3 -1 0
//! ```
//!
//! Each clause is a whitespace-separated list of non-zero integers ended by
//! `0`; a positive `n` is variable `n` (1-based), a negative one its
//! negation. The parser is lenient in the ways solvers usually are: the
//! `p` line is optional, its counts are not enforced (the larger of the
//! declared and the observed variable count wins), clauses may span lines,
//! and a trailing `%` line (SATLIB style) ends the input.
//!
//! Models are written in the competition output format: lines starting
//! with `v` listing every variable's literal, terminated by `0`.

use std::fmt;

use super::{Lit, Solver};

/// A parsed DIMACS CNF file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Cnf {
    /// Number of variables: the maximum of the `p cnf` declaration and the
    /// largest variable used.
    pub num_vars: u32,
    /// The clauses, in file order.
    pub clauses: Vec<Vec<Lit>>,
}

/// A DIMACS syntax error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DimacsError {
    /// 1-based line of the offending token.
    pub line: usize,
    /// What went wrong.
    pub message: String,
}

impl fmt::Display for DimacsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

impl std::error::Error for DimacsError {}

/// Parses DIMACS CNF text.
pub fn parse_dimacs(text: &str) -> Result<Cnf, DimacsError> {
    let mut cnf = Cnf::default();
    let mut current: Vec<Lit> = Vec::new();
    let mut max_var: u32 = 0;
    let mut open_line = 0;
    let err = |line: usize, message: String| DimacsError { line, message };

    for (i, raw) in text.lines().enumerate() {
        let line = i + 1;
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.starts_with('c') {
            continue;
        }
        if trimmed == "%" {
            break;
        }
        if let Some(rest) = trimmed.strip_prefix('p') {
            let mut fields = rest.split_whitespace();
            if fields.next() != Some("cnf") {
                return Err(err(line, "expected `p cnf <vars> <clauses>`".into()));
            }
            let vars = fields
                .next()
                .and_then(|s| s.parse::<u32>().ok())
                .ok_or_else(|| err(line, "bad variable count in `p cnf` line".into()))?;
            fields
                .next()
                .and_then(|s| s.parse::<u64>().ok())
                .ok_or_else(|| err(line, "bad clause count in `p cnf` line".into()))?;
            cnf.num_vars = cnf.num_vars.max(vars);
            continue;
        }
        for tok in trimmed.split_whitespace() {
            let n: i32 = tok
                .parse()
                .map_err(|_| err(line, format!("expected a literal, found `{tok}`")))?;
            if n == 0 {
                cnf.clauses.push(std::mem::take(&mut current));
                continue;
            }
            if current.is_empty() {
                open_line = line;
            }
            let lit = Lit::from_dimacs(n).expect("non-zero");
            max_var = max_var.max(lit.var().index() + 1);
            current.push(lit);
        }
    }
    if !current.is_empty() {
        return Err(err(open_line, "clause not terminated by 0".into()));
    }
    cnf.num_vars = cnf.num_vars.max(max_var);
    Ok(cnf)
}

/// Renders clauses as DIMACS CNF text with a `p cnf` header.
pub fn write_dimacs<'a, I>(num_vars: u32, clauses: I) -> String
where
    I: IntoIterator<Item = &'a [Lit]>,
    I::IntoIter: ExactSizeIterator,
{
    use std::fmt::Write as _;
    let clauses = clauses.into_iter();
    let mut out = String::new();
    let _ = writeln!(out, "p cnf {num_vars} {}", clauses.len());
    for clause in clauses {
        for l in clause {
            let _ = write!(out, "{} ", l.to_dimacs());
        }
        out.push_str("0\n");
    }
    out
}

/// Renders a model as competition-style `v` lines, ending with `0`.
///
/// Unassigned variables are printed positive, since any value satisfies.
pub fn write_model(model: &[Option<bool>]) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    let mut col = 0;
    for (i, value) in model.iter().enumerate() {
        if col == 0 {
            out.push('v');
        }
        let n = i + 1;
        let _ = if value == &Some(false) {
            write!(out, " -{n}")
        } else {
            write!(out, " {n}")
        };
        col += 1;
        if col == 20 {
            out.push('\n');
            col = 0;
        }
    }
    if col == 0 {
        out.push('v');
    }
    out.push_str(" 0\n");
    out
}

impl Solver {
    /// Builds a solver from DIMACS CNF text.
    ///
    /// Clauses are added in order, so a top-level contradiction leaves the
    /// solver in its "not ok" state (every `solve` returns `Unsat`); that
    /// is not a parse error.
    pub fn from_dimacs(text: &str) -> Result<Solver, DimacsError> {
        let cnf = parse_dimacs(text)?;
        let mut solver = Solver::new();
        while solver.num_vars() < cnf.num_vars as usize {
            solver.new_var();
        }
        for clause in &cnf.clauses {
            if !solver.add_clause(clause) {
                break;
            }
        }
        Ok(solver)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::formal::sat::{SolveResult, Var};

    fn l(n: i32) -> Lit {
        Lit::from_dimacs(n).unwrap()
    }

    #[test]
    fn parse_basic() {
        let cnf = parse_dimacs("c hello\np cnf 3 2\n1 -3 0\n2 3\n -1 0\n").unwrap();
        assert_eq!(cnf.num_vars, 3);
        assert_eq!(
            cnf.clauses,
            vec![vec![l(1), l(-3)], vec![l(2), l(3), l(-1)]]
        );
    }

    #[test]
    fn parse_without_header_and_with_percent() {
        let cnf = parse_dimacs("1 5 0\n-2 0\n%\n0\n").unwrap();
        assert_eq!(cnf.num_vars, 5);
        assert_eq!(cnf.clauses.len(), 2);
        let cnf = parse_dimacs("p cnf 9 1\n1 2 0\n").unwrap();
        assert_eq!(cnf.num_vars, 9, "declared count wins when larger");
        let cnf = parse_dimacs("p cnf 0 0\n").unwrap();
        assert_eq!(cnf.num_vars, 0);
        assert!(cnf.clauses.is_empty());
    }

    #[test]
    fn parse_errors() {
        let e = parse_dimacs("p cnf x 1\n").unwrap_err();
        assert_eq!(e.line, 1);
        let e = parse_dimacs("p dnf 1 1\n").unwrap_err();
        assert!(e.message.contains("p cnf"));
        let e = parse_dimacs("1 2 0\n\n3 foo 0\n").unwrap_err();
        assert_eq!(e.line, 3);
        assert!(e.message.contains("foo"));
        let e = parse_dimacs("1 2 0\n3 4\n").unwrap_err();
        assert_eq!(e.line, 2);
        assert_eq!(e.to_string(), "line 2: clause not terminated by 0");
    }

    #[test]
    fn dimacs_literal_round_trip() {
        for n in [1, -1, 7, -42] {
            assert_eq!(l(n).to_dimacs(), n);
        }
        assert_eq!(Lit::from_dimacs(0), None);
        assert_eq!(l(1), Lit::pos(Var::new(0)));
        assert_eq!(l(-1), Lit::neg(Var::new(0)));
    }

    #[test]
    fn write_and_reparse() {
        let clauses: Vec<Vec<Lit>> = vec![vec![l(1), l(-2)], vec![l(2), l(3)]];
        let text = write_dimacs(3, clauses.iter().map(Vec::as_slice));
        assert_eq!(text, "p cnf 3 2\n1 -2 0\n2 3 0\n");
        assert_eq!(parse_dimacs(&text).unwrap().clauses, clauses);
    }

    #[test]
    fn write_model_format() {
        assert_eq!(write_model(&[]), "v 0\n");
        assert_eq!(
            write_model(&[Some(true), Some(false), None]),
            "v 1 -2 3 0\n"
        );
        let model: Vec<Option<bool>> = (0..21).map(|i| Some(i % 2 == 0)).collect();
        let text = write_model(&model);
        assert_eq!(text.lines().count(), 2);
        assert!(text.ends_with(" 21 0\n"));
        assert!(text.lines().all(|line| line.starts_with("v ")));
    }

    #[test]
    fn from_dimacs_solves() {
        let mut s = Solver::from_dimacs("p cnf 2 3\n1 2 0\n-1 2 0\n1 -2 0\n").unwrap();
        assert_eq!(s.solve(), SolveResult::Sat);
        assert_eq!(s.model(), &[Some(true), Some(true)]);
        assert_eq!(write_model(s.model()), "v 1 2 0\n");
        let mut s = Solver::from_dimacs("1 0\n-1 0\n").unwrap();
        assert!(!s.is_ok());
        assert_eq!(s.solve(), SolveResult::Unsat);
        assert!(Solver::from_dimacs("1 0\n2").is_err());
    }
}
