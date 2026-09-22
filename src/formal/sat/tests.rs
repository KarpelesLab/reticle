//! Solver tests: API edge cases, structured instances, and randomised
//! cross-checks against a reference DPLL solver.

use super::*;

fn v(i: u32) -> Var {
    Var::new(i)
}

fn p(i: u32) -> Lit {
    Lit::pos(v(i))
}

fn n(i: u32) -> Lit {
    Lit::neg(v(i))
}

/// xorshift64*: tiny, deterministic, and good enough for test instances.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Rng {
        Rng(seed.max(1))
    }

    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: u64) -> u64 {
        (self.next() >> 11) % n
    }

    fn bool(&mut self) -> bool {
        self.next() >> 63 == 1
    }
}

/// A random 3-SAT instance: `m` clauses over `n` variables, three distinct
/// variables per clause, random signs.
fn random_3sat(rng: &mut Rng, n: u32, m: usize) -> Vec<Vec<Lit>> {
    (0..m)
        .map(|_| {
            let mut vars = Vec::with_capacity(3);
            while vars.len() < 3 {
                let x = u32::try_from(rng.below(u64::from(n))).unwrap();
                if !vars.contains(&x) {
                    vars.push(x);
                }
            }
            vars.into_iter()
                .map(|x| Lit::new(v(x), rng.bool()))
                .collect()
        })
        .collect()
}

fn satisfies(clauses: &[Vec<Lit>], model: &[Option<bool>]) -> bool {
    clauses
        .iter()
        .all(|c| c.iter().any(|l| model[l.var().idx()] == Some(l.is_pos())))
}

/// A reference DPLL solver with unit propagation, for small instances.
fn dpll(clauses: &[Vec<Lit>], n: usize) -> bool {
    fn go(clauses: &[Vec<Lit>], assign: &mut [Option<bool>]) -> bool {
        loop {
            let mut changed = false;
            for c in clauses {
                let mut unassigned = None;
                let mut count = 0;
                let mut satisfied = false;
                for l in c {
                    match assign[l.var().idx()] {
                        Some(val) if val == l.is_pos() => {
                            satisfied = true;
                            break;
                        }
                        Some(_) => {}
                        None => {
                            count += 1;
                            unassigned = Some(*l);
                        }
                    }
                }
                if satisfied {
                    continue;
                }
                match (count, unassigned) {
                    (0, _) => return false,
                    (1, Some(l)) => {
                        assign[l.var().idx()] = Some(l.is_pos());
                        changed = true;
                    }
                    _ => {}
                }
            }
            if !changed {
                break;
            }
        }
        let Some(x) = assign.iter().position(Option::is_none) else {
            return true;
        };
        for val in [false, true] {
            let mut copy = assign.to_owned();
            copy[x] = Some(val);
            if go(clauses, &mut copy) {
                return true;
            }
        }
        false
    }
    go(clauses, &mut vec![None; n])
}

fn solver_from(clauses: &[Vec<Lit>]) -> Solver {
    let mut s = Solver::new();
    for c in clauses {
        s.add_clause(c);
    }
    s
}

/// Pigeonhole: `pigeons` pigeons into `holes` holes, each pigeon in some
/// hole, no hole holding two pigeons. Variable `p * holes + h`.
fn pigeonhole(pigeons: u32, holes: u32) -> Vec<Vec<Lit>> {
    let var = |pg: u32, h: u32| p(pg * holes + h);
    let mut clauses = Vec::new();
    for pg in 0..pigeons {
        clauses.push((0..holes).map(|h| var(pg, h)).collect());
    }
    for h in 0..holes {
        for a in 0..pigeons {
            for b in a + 1..pigeons {
                clauses.push(vec![!var(a, h), !var(b, h)]);
            }
        }
    }
    clauses
}

// ----- literals and variables ----------------------------------------------

#[test]
fn lit_encoding() {
    let x = v(5);
    assert_eq!(x.index(), 5);
    assert_eq!(Lit::pos(x).var(), x);
    assert_eq!(Lit::neg(x).var(), x);
    assert!(Lit::pos(x).is_pos());
    assert!(Lit::neg(x).is_neg());
    assert_eq!(!Lit::pos(x), Lit::neg(x));
    assert_eq!(!!Lit::pos(x), Lit::pos(x));
    assert_eq!(Lit::new(x, false), Lit::pos(x));
    assert_eq!(Lit::new(x, true), Lit::neg(x));
    assert_eq!(Lit::pos(x).index(), 10);
    assert_eq!(Lit::neg(x).index(), 11);
    assert!(Lit::pos(x) < Lit::neg(x));
    assert!(Lit::neg(x) < Lit::pos(v(6)));
    assert_eq!(x.to_string(), "x5");
    assert_eq!(Lit::pos(x).to_string(), "x5");
    assert_eq!(Lit::neg(x).to_string(), "!x5");
}

// ----- API edge cases ------------------------------------------------------

#[test]
fn empty_problem_is_sat() {
    let mut s = Solver::new();
    assert_eq!(s.num_vars(), 0);
    assert_eq!(s.solve(), SolveResult::Sat);
    assert!(s.model().is_empty());
    let x = s.new_var();
    assert_eq!(s.solve(), SolveResult::Sat);
    assert!(s.value(x).is_some());
}

#[test]
fn empty_clause_is_unsat() {
    let mut s = Solver::new();
    assert!(!s.add_clause(&[]));
    assert!(!s.is_ok());
    assert_eq!(s.solve(), SolveResult::Unsat);
    assert!(s.conflict_core().is_empty());
    assert!(!s.add_clause(&[p(0)]), "nothing can be added after");
}

#[test]
fn unit_clauses() {
    let mut s = Solver::new();
    assert!(s.add_clause(&[p(0)]));
    assert!(s.add_clause(&[n(1)]));
    assert_eq!(s.num_vars(), 2, "variables are created on demand");
    assert_eq!(s.num_clauses(), 0, "units are assignments, not clauses");
    assert_eq!(s.solve(), SolveResult::Sat);
    assert_eq!(s.value(v(0)), Some(true));
    assert_eq!(s.value(v(1)), Some(false));
    assert!(s.add_clause(&[p(0), p(1)]), "satisfied clause is dropped");
    assert_eq!(s.num_clauses(), 0);
    assert!(!s.add_clause(&[n(0)]), "contradicting unit");
    assert_eq!(s.solve(), SolveResult::Unsat);
}

#[test]
fn unit_propagation_at_top_level() {
    let mut s = Solver::new();
    s.add_clause(&[p(0), p(1)]);
    s.add_clause(&[p(0), n(1)]);
    assert!(s.add_clause(&[n(0), p(2)]));
    // !x0 forces x1 and !x1: propagation finds the contradiction at once.
    assert!(!s.add_clause(&[n(0)]));
    assert!(!s.is_ok());
    assert_eq!(s.solve(), SolveResult::Unsat);
}

#[test]
fn duplicates_and_tautologies() {
    let mut s = Solver::new();
    assert!(s.add_clause(&[p(0), n(0)]));
    assert_eq!(s.num_clauses(), 0, "tautology dropped");
    assert!(s.add_clause(&[p(1), p(1), p(1)]));
    assert_eq!(s.num_clauses(), 0, "duplicates collapse to a unit");
    assert_eq!(s.solve(), SolveResult::Sat);
    assert_eq!(s.value(v(1)), Some(true));
    assert!(s.add_clause(&[p(2), p(3), p(2), n(3), p(2)]));
    assert_eq!(s.num_clauses(), 0, "tautology after dedup");
    assert!(s.add_clause(&[n(1), p(4), p(5), p(4)]));
    assert_eq!(s.num_clauses(), 1);
    assert_eq!(s.db.lits(s.clauses[0]), &[p(4), p(5)], "false x1 stripped");
}

#[test]
fn model_is_reset_between_calls() {
    let mut s = Solver::new();
    s.add_clause(&[p(0), p(1)]);
    assert_eq!(s.solve(), SolveResult::Sat);
    assert_eq!(s.model().len(), 2);
    assert_eq!(s.solve_with_assumptions(&[n(0), n(1)]), SolveResult::Unsat);
    assert!(s.model().is_empty());
    assert_eq!(s.value(v(0)), None);
    let x = s.new_var();
    assert_eq!(s.solve(), SolveResult::Sat);
    assert!(s.value(x).is_some());
}

#[test]
fn assumptions_and_cores() {
    let mut s = Solver::new();
    s.add_clause(&[p(0), p(1)]);
    s.add_clause(&[n(0), p(1)]);
    assert_eq!(s.solve_with_assumptions(&[n(1)]), SolveResult::Unsat);
    assert_eq!(s.conflict_core(), &[n(1)]);
    assert!(s.is_ok(), "the clauses alone are still satisfiable");
    assert_eq!(s.solve_with_assumptions(&[p(1)]), SolveResult::Sat);
    assert_eq!(s.value(v(1)), Some(true));
    assert!(s.conflict_core().is_empty());

    // Only the assumptions involved end up in the core.
    let mut s = Solver::new();
    s.add_clause(&[n(0), n(1)]);
    s.add_clause(&[p(2), p(3)]);
    assert_eq!(
        s.solve_with_assumptions(&[p(4), p(0), p(2), p(1)]),
        SolveResult::Unsat
    );
    let mut core = s.conflict_core().to_vec();
    core.sort();
    assert_eq!(core, vec![p(0), p(1)]);

    // An assumption already false at level 0 is a core by itself.
    let mut s = Solver::new();
    s.add_clause(&[n(0)]);
    assert_eq!(s.solve_with_assumptions(&[p(1), p(0)]), SolveResult::Unsat);
    assert_eq!(s.conflict_core(), &[p(0)]);

    // Contradictory assumptions.
    let mut s = Solver::new();
    s.new_var();
    assert_eq!(s.solve_with_assumptions(&[p(0), n(0)]), SolveResult::Unsat);
    let mut core = s.conflict_core().to_vec();
    core.sort();
    assert_eq!(core, vec![p(0), n(0)]);

    // Redundant assumptions (same literal twice) are fine.
    assert_eq!(s.solve_with_assumptions(&[p(0), p(0)]), SolveResult::Sat);
}

#[test]
fn core_through_propagation_chain() {
    // a -> b -> c -> d, and !d; assuming a (plus unrelated e) gives core {a}.
    let mut s = Solver::new();
    s.add_clause(&[n(0), p(1)]);
    s.add_clause(&[n(1), p(2)]);
    s.add_clause(&[n(2), p(3)]);
    s.add_clause(&[n(3), p(5)]);
    s.add_clause(&[n(5), n(6)]);
    assert_eq!(
        s.solve_with_assumptions(&[p(4), p(0), p(6)]),
        SolveResult::Unsat
    );
    let mut core = s.conflict_core().to_vec();
    core.sort();
    assert_eq!(core, vec![p(0), p(6)]);
}

#[test]
fn incremental_solving() {
    let mut s = Solver::new();
    s.add_clause(&[p(0), p(1), p(2)]);
    assert_eq!(s.solve(), SolveResult::Sat);
    s.add_clause(&[n(0)]);
    assert_eq!(s.solve(), SolveResult::Sat);
    assert_eq!(s.value(v(0)), Some(false));
    s.add_clause(&[n(1), p(3)]);
    s.add_clause(&[n(3), n(2)]);
    assert_eq!(s.solve_with_assumptions(&[p(1)]), SolveResult::Sat);
    assert_eq!(s.value(v(2)), Some(false));
    s.add_clause(&[n(1)]);
    assert_eq!(s.solve(), SolveResult::Sat);
    assert_eq!(s.value(v(2)), Some(true));
    assert!(!s.add_clause(&[n(2)]));
    assert_eq!(s.solve(), SolveResult::Unsat);
    assert_eq!(s.solve(), SolveResult::Unsat);
}

#[test]
fn incremental_after_learning() {
    // Learn clauses on a random instance, then keep adding units and check
    // the answers against the reference throughout.
    let mut rng = Rng::new(0xC0FFEE);
    let n = 18;
    let mut clauses = random_3sat(&mut rng, n, 70);
    let mut s = solver_from(&clauses);
    for i in 0..n {
        let expect = dpll(&clauses, n as usize);
        let got = s.solve();
        assert_eq!(got == SolveResult::Sat, expect, "after {i} units");
        if expect {
            assert!(satisfies(&clauses, s.model()));
        } else {
            break;
        }
        let unit = vec![Lit::new(v(i), rng.bool())];
        clauses.push(unit.clone());
        s.add_clause(&unit);
    }
}

#[test]
fn limits_yield_unknown() {
    let clauses = pigeonhole(9, 8);
    let mut s = solver_from(&clauses);
    s.set_conflict_limit(Some(20));
    assert_eq!(s.solve(), SolveResult::Unknown);
    assert!(s.stats().conflicts >= 20);
    assert!(s.is_ok(), "an Unknown answer decides nothing");
    assert!(s.model().is_empty());
    let before = s.stats().conflicts;
    s.set_conflict_limit(None);
    s.set_propagation_limit(Some(500));
    assert_eq!(s.solve(), SolveResult::Unknown);
    assert!(s.stats().conflicts > before, "limits are per call");
    s.set_propagation_limit(None);

    // The same solver finishes an easier job once the limits are lifted.
    let mut s = solver_from(&pigeonhole(5, 4));
    s.set_conflict_limit(Some(1));
    assert_eq!(s.solve(), SolveResult::Unknown);
    s.set_conflict_limit(None);
    assert_eq!(s.solve(), SolveResult::Unsat);
    assert!(!s.is_ok());

    // A limit that is never reached does not change the answer.
    let mut s = solver_from(&pigeonhole(4, 4));
    s.set_conflict_limit(Some(1_000_000));
    s.set_propagation_limit(Some(1_000_000));
    assert_eq!(s.solve(), SolveResult::Sat);
}

#[test]
fn stats_and_restarts() {
    let mut s = solver_from(&pigeonhole(7, 6));
    assert_eq!(s.stats(), Stats::default());
    assert_eq!(s.solve(), SolveResult::Unsat);
    let st = s.stats();
    assert!(st.conflicts > 100);
    assert!(st.decisions > 0);
    assert!(st.propagations > st.decisions);
    assert!(st.learnts > 0);
    assert!(
        st.restarts > 0,
        "Luby restarts every 100 conflicts at first"
    );
    assert!(st.tot_literals <= st.max_literals);
    assert!(
        st.tot_literals < st.max_literals,
        "minimisation removes literals"
    );
}

#[test]
fn deterministic() {
    let mut rng = Rng::new(7);
    let clauses = random_3sat(&mut rng, 120, 510);
    let mut a = solver_from(&clauses);
    let mut b = solver_from(&clauses);
    let ra = a.solve();
    let rb = b.solve();
    assert_eq!(ra, rb);
    assert_eq!(a.stats(), b.stats());
    assert_eq!(a.model(), b.model());
}

// ----- structured instances -----------------------------------------------

#[test]
fn pigeonhole_instances() {
    let mut s = solver_from(&pigeonhole(6, 6));
    assert_eq!(s.solve(), SolveResult::Sat);
    let m = s.model().to_vec();
    assert!(satisfies(&pigeonhole(6, 6), &m));
    assert_eq!(solver_from(&pigeonhole(6, 5)).solve(), SolveResult::Unsat);
    assert_eq!(solver_from(&pigeonhole(7, 6)).solve(), SolveResult::Unsat);
}

#[test]
fn pigeonhole_with_glucose_restarts() {
    let mut s = solver_from(&pigeonhole(7, 6));
    s.set_restart_policy(RestartPolicy::Glucose);
    assert_eq!(s.restart_policy(), RestartPolicy::Glucose);
    assert_eq!(s.solve(), SolveResult::Unsat);
}

#[test]
fn chain_of_implications_backjumps() {
    // A long implication chain hanging off each decision: the learnt
    // clauses must be asserting at the right level for propagation to
    // pick them up (exercises the watch positions of learnt clauses).
    let mut s = Solver::new();
    let len = 200;
    for i in 0..len {
        s.add_clause(&[n(i), p(i + 1)]);
    }
    s.add_clause(&[n(len), n(len + 1)]);
    s.add_clause(&[p(0), p(len + 1)]);
    s.add_clause(&[p(0), n(len + 1)]);
    assert_eq!(s.solve(), SolveResult::Sat);
    assert_eq!(s.value(v(0)), Some(true));
    assert_eq!(s.value(v(len + 1)), Some(false));
}

// ----- database maintenance ------------------------------------------------

#[test]
fn reduce_db_and_garbage_collection() {
    let mut rng = Rng::new(99);
    let clauses = random_3sat(&mut rng, 150, 640);
    let mut plain = solver_from(&clauses);
    let expect = plain.solve();

    let mut s = solver_from(&clauses);
    s.next_reduce = 20; // reduce early and often
    assert_eq!(s.solve(), expect);
    let st = s.stats();
    assert!(st.reductions > 0, "reductions: {}", st.reductions);
    assert!(st.removed_learnts > 0);
    assert!(
        s.db.wasted() * 5 <= s.db.used(),
        "garbage stays under a fifth of the arena"
    );
    if expect == SolveResult::Sat {
        assert!(satisfies(&clauses, s.model()));
    }
    // Every watcher points at a live clause and the watched literals are
    // the clause's first two.
    for (i, ws) in s.watches.iter().enumerate() {
        for w in ws {
            assert!(!s.db.is_deleted(w.cref));
            let c = s.db.lits(w.cref);
            let watched = !Lit::from_raw(u32::try_from(i).unwrap());
            assert!(c[0] == watched || c[1] == watched);
        }
    }
    for &cr in s.clauses.iter().chain(&s.learnts) {
        assert!(!s.db.is_deleted(cr));
    }
}

#[test]
fn top_level_simplification_removes_clauses() {
    let mut s = Solver::new();
    s.add_clause(&[p(0), p(1), p(2)]);
    s.add_clause(&[p(0), n(1), p(3)]);
    s.add_clause(&[n(4), p(5), p(6)]);
    assert_eq!(s.num_clauses(), 3);
    s.add_clause(&[p(0)]);
    // Adding a unit propagates but does not simplify until a solve.
    assert_eq!(s.num_clauses(), 3);
    assert_eq!(s.solve(), SolveResult::Sat);
    assert_eq!(s.num_clauses(), 1, "clauses satisfied by x0 are gone");
    s.add_clause(&[p(4)]);
    s.add_clause(&[n(5)]);
    assert_eq!(s.solve(), SolveResult::Sat);
    assert_eq!(s.num_clauses(), 0, "the last clause became a unit");
    assert_eq!(s.value(v(6)), Some(true));
}

// ----- randomised cross-checks ---------------------------------------------

#[test]
fn random_3sat_matches_reference() {
    let mut rng = Rng::new(0x5EED);
    let mut sat = 0;
    let mut unsat = 0;
    for &n in &[8u32, 12, 16, 20] {
        // Clause/variable ratios in hundredths: 3.00, 4.26 (the phase
        // transition), 5.50.
        for &ratio in &[300u32, 426, 550] {
            let m = (n * ratio).div_ceil(100) as usize;
            for _ in 0..10 {
                let clauses = random_3sat(&mut rng, n, m);
                let expect = dpll(&clauses, n as usize);
                let mut s = solver_from(&clauses);
                let got = s.solve();
                assert_ne!(got, SolveResult::Unknown);
                assert_eq!(got == SolveResult::Sat, expect, "n={n} m={m}");
                if expect {
                    assert!(satisfies(&clauses, s.model()));
                    sat += 1;
                } else {
                    assert!(s.conflict_core().is_empty());
                    unsat += 1;
                }
            }
        }
    }
    assert!(sat > 20 && unsat > 20, "sat={sat} unsat={unsat}");
}

#[test]
fn random_3sat_with_assumptions_matches_reference() {
    let mut rng = Rng::new(0xA55);
    for round in 0..40 {
        let n = 16;
        let clauses = random_3sat(&mut rng, n, 60);
        let mut s = solver_from(&clauses);
        for _ in 0..3 {
            let k = 1 + usize::try_from(rng.below(4)).unwrap();
            let mut assumptions = Vec::new();
            while assumptions.len() < k {
                let x = u32::try_from(rng.below(u64::from(n))).unwrap();
                let l = Lit::new(v(x), rng.bool());
                if !assumptions.contains(&l) && !assumptions.contains(&!l) {
                    assumptions.push(l);
                }
            }
            let mut with = clauses.clone();
            with.extend(assumptions.iter().map(|&l| vec![l]));
            let expect = dpll(&with, n as usize);
            let got = s.solve_with_assumptions(&assumptions);
            assert_eq!(got == SolveResult::Sat, expect, "round {round}");
            if expect {
                assert!(satisfies(&with, s.model()));
            } else {
                // The core is a subset of the assumptions and is itself
                // enough to refute.
                let core = s.conflict_core().to_vec();
                assert!(core.iter().all(|l| assumptions.contains(l)));
                let mut with_core = clauses.clone();
                with_core.extend(core.iter().map(|&l| vec![l]));
                assert!(!dpll(&with_core, n as usize), "core refutes: {core:?}");
            }
        }
    }
}

#[test]
fn random_3sat_larger_models_verify() {
    let mut rng = Rng::new(0xBEEF);
    let mut sat = 0;
    for &(n, ratio) in &[(100u32, 400u32), (150, 426), (200, 410)] {
        for _ in 0..3 {
            let m = (n * ratio / 100) as usize;
            let clauses = random_3sat(&mut rng, n, m);
            let mut s = solver_from(&clauses);
            let got = s.solve();
            assert_ne!(got, SolveResult::Unknown);
            if got == SolveResult::Sat {
                assert!(satisfies(&clauses, s.model()));
                sat += 1;
            }
            // Both policies must agree.
            let mut g = solver_from(&clauses);
            g.set_restart_policy(RestartPolicy::Glucose);
            assert_eq!(g.solve(), got);
        }
    }
    assert!(sat > 0);
}

/// Performance smoke test on random 3-SAT at the phase transition and on
/// pigeonhole. Not a correctness test; run it by hand with
/// `cargo test --release --all-features -- --ignored --nocapture bench`.
#[test]
#[ignore = "benchmark; run manually in release mode"]
fn bench_random_3sat() {
    use std::time::Instant;
    let mut rng = Rng::new(0x1234_5678);
    for &n in &[200u32, 250, 300] {
        let m = (n * 426 / 100) as usize;
        let mut total_ms = 0.0;
        let mut conflicts = 0;
        let mut sat = 0;
        let runs: u32 = 5;
        for _ in 0..runs {
            let clauses = random_3sat(&mut rng, n, m);
            let mut s = solver_from(&clauses);
            let start = Instant::now();
            let got = s.solve();
            total_ms += start.elapsed().as_secs_f64() * 1e3;
            conflicts += s.stats().conflicts;
            if got == SolveResult::Sat {
                assert!(satisfies(&clauses, s.model()));
                sat += 1;
            }
        }
        println!(
            "3-SAT n={n} m={m}: {runs} runs, {sat} sat, avg {:.1} ms, avg {} conflicts",
            total_ms / f64::from(runs),
            conflicts / u64::from(runs)
        );
    }
    for &(pigeons, holes) in &[(8u32, 7u32), (9, 8), (10, 9)] {
        let mut s = solver_from(&pigeonhole(pigeons, holes));
        let start = Instant::now();
        let got = s.solve();
        println!(
            "PHP {pigeons}->{holes}: {got:?} in {:.1} ms, {} conflicts",
            start.elapsed().as_secs_f64() * 1e3,
            s.stats().conflicts
        );
    }
}
