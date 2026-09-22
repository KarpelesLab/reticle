//! Reachability lint: constant selects, dead branches and dead states.
//!
//! A multiplexer whose select never changes, a flip-flop whose enable is
//! always (or never) asserted, or an FSM state no input sequence reaches
//! are usually bugs, or at least dead logic worth a warning. Simulation
//! finds them only for the stimulus it runs; the SAT engines can decide
//! them exactly within a bound: for every value of interest, "is there a
//! trace of at most `depth` frames from the initial state in which this
//! signal takes it?" is a `cover` question, answered by the same
//! incremental unrolling as [`mod@super::bmc`].
//!
//! The probes come from the bit-blaster ([`super::blast`]): the select of
//! every `Mux` and `Pmux` cell and `?:` expression, the enable of every
//! `Dff`, and the value of every net carrying an `fsm_state` attribute
//! (for widths up to [`ReachOptions::max_fsm_bits`], every value of the
//! encoding is tried; synthesis sets the attribute on the state register
//! it extracted, and hand-written netlists can set it themselves).
//!
//! Every finding is a warning [`Diagnostic`] with the cell's or
//! expression's span:
//!
//! | Code    | Finding                                                     |
//! |---------|-------------------------------------------------------------|
//! | `F0021` | A mux select or `?:` condition is constant within the bound |
//! | `F0022` | A flip-flop enable is constant within the bound             |
//! | `F0023` | An FSM state value is unreachable within the bound          |
//!
//! A verdict is bounded: "constant within `depth` frames" may still change
//! later. A probe that can take no value at all (contradictory
//! assumptions) is not reported. A solve that hits the conflict limit
//! leaves its probe undecided and unreported.

use super::blast::{BlastOptions, Blaster, ProbeKind};
use super::sat::{Lit, SolveResult};
use super::unroll::{InitMode, Unrolling};
use crate::diag::{Diagnostic, Diagnostics};
use crate::ir::{Design, ModuleId};

/// Options of [`reach_lint`].
#[derive(Clone, Debug)]
pub struct ReachOptions {
    /// Frames explored from the initial state.
    pub depth: u32,
    /// How frame 0 is constrained.
    pub init: InitMode,
    /// FSM state nets wider than this are not enumerated.
    pub max_fsm_bits: u32,
    /// Conflicts allowed per SAT call; `None` for no limit.
    pub conflict_limit: Option<u64>,
    /// Options of the bit-blaster.
    pub blast: BlastOptions,
}

impl Default for ReachOptions {
    fn default() -> Self {
        ReachOptions {
            depth: 20,
            init: InitMode::Reset,
            max_fsm_bits: 6,
            conflict_limit: None,
            blast: BlastOptions::default(),
        }
    }
}

/// What [`reach_lint`] returns.
#[derive(Clone, Debug, Default)]
pub struct ReachReport {
    /// Number of probe values examined.
    pub checked: usize,
    /// Number of probe values found unreachable.
    pub unreachable: usize,
    /// The findings (warnings), plus blaster diagnostics.
    pub diags: Diagnostics,
}

impl ReachReport {
    /// One line summarising the run; the findings themselves are the
    /// diagnostics.
    pub fn render(&self, module: &str) -> String {
        format!(
            "reach lint {module}: {} probe values checked, {} unreachable\n",
            self.checked, self.unreachable
        )
    }
}

/// One value of one probe.
struct Target {
    probe: usize,
    /// The bit values demanded, LSB first; `None` for a bit not constrained.
    bits: Vec<Option<bool>>,
    /// The value as a number, for messages.
    value: u64,
    reached: bool,
    unknown: bool,
}

/// Runs the lint on `module` of `design`.
pub fn reach_lint(design: &Design, module: ModuleId, options: &ReachOptions) -> ReachReport {
    let m = design.module(module);
    let blaster = match Blaster::new(m, &options.blast) {
        Ok(b) => b,
        Err(diags) => {
            return ReachReport {
                diags,
                ..ReachReport::default()
            };
        }
    };
    let mut diags = blaster.diagnostics().clone();
    let mut unrolling = Unrolling::new(&blaster, options.init);
    unrolling.set_conflict_limit(options.conflict_limit);

    // Enumerate the targets from frame 0's probes.
    let probes = unrolling.frames()[0].probes.clone();
    let mut targets: Vec<Target> = Vec::new();
    for (i, p) in probes.iter().enumerate() {
        let width = u32::try_from(p.lits.len()).expect("probe width fits u32");
        match p.kind {
            ProbeKind::MuxSelect | ProbeKind::TernaryCond | ProbeKind::DffEnable => {
                for v in [false, true] {
                    targets.push(Target {
                        probe: i,
                        bits: vec![Some(v)],
                        value: u64::from(v),
                        reached: false,
                        unknown: false,
                    });
                }
            }
            ProbeKind::PmuxSelect => {
                for (bit, _) in p.lits.iter().enumerate() {
                    for v in [false, true] {
                        let mut bits = vec![None; p.lits.len()];
                        bits[bit] = Some(v);
                        targets.push(Target {
                            probe: i,
                            bits,
                            value: u64::try_from(bit).expect("fits") * 2 + u64::from(v),
                            reached: false,
                            unknown: false,
                        });
                    }
                }
            }
            ProbeKind::FsmState => {
                if width == 0 || width > options.max_fsm_bits {
                    continue;
                }
                for value in 0..(1u64 << width) {
                    let bits = (0..width).map(|b| Some((value >> b) & 1 == 1)).collect();
                    targets.push(Target {
                        probe: i,
                        bits,
                        value,
                        reached: false,
                        unknown: false,
                    });
                }
            }
        }
    }

    for k in 0..=options.depth {
        let ku = usize::try_from(k).expect("depth fits usize");
        if k > 0 {
            unrolling.extend();
        }
        for t in targets.iter_mut().filter(|t| !t.reached) {
            let lits: Vec<Lit> = unrolling.frames()[ku].probes[t.probe]
                .lits
                .iter()
                .zip(&t.bits)
                .filter_map(|(&l, b)| b.map(|b| if b { l } else { !l }))
                .collect();
            let want = unrolling.cnf().and_n(&lits);
            match unrolling.solve(&[want]) {
                SolveResult::Sat => t.reached = true,
                SolveResult::Unsat => {}
                SolveResult::Unknown => t.unknown = true,
            }
        }
        if targets.iter().all(|t| t.reached) {
            break;
        }
    }

    let within = format!("within {} frames", options.depth + 1);
    let mut unreachable = 0;
    for (i, p) in probes.iter().enumerate() {
        let mine: Vec<&Target> = targets.iter().filter(|t| t.probe == i).collect();
        if mine.is_empty() {
            continue;
        }
        let decided = |t: &&&Target| !t.unknown;
        match p.kind {
            ProbeKind::MuxSelect | ProbeKind::TernaryCond | ProbeKind::DffEnable => {
                let zero = mine.iter().find(|t| t.value == 0).filter(decided);
                let one = mine.iter().find(|t| t.value == 1).filter(decided);
                let (Some(zero), Some(one)) = (zero, one) else {
                    continue;
                };
                if zero.reached == one.reached {
                    continue;
                }
                unreachable += 1;
                let constant = u8::from(one.reached);
                let (code, message) = match p.kind {
                    ProbeKind::MuxSelect => (
                        "F0021",
                        format!(
                            "select of mux `{}` is constant {constant} {within}: input `{}` is never chosen",
                            p.name,
                            if one.reached { "a" } else { "b" }
                        ),
                    ),
                    ProbeKind::TernaryCond => (
                        "F0021",
                        format!(
                            "condition of this `?:` is constant {constant} {within}: the {} branch is never taken",
                            if one.reached { "else" } else { "then" }
                        ),
                    ),
                    _ => (
                        "F0022",
                        if one.reached {
                            format!(
                                "enable of flip-flop `{}` is always 1 {within}: the enable is redundant",
                                p.name
                            )
                        } else {
                            format!(
                                "enable of flip-flop `{}` is never 1 {within}: the register never loads",
                                p.name
                            )
                        },
                    ),
                };
                diags.push(
                    Diagnostic::warning(message)
                        .with_code(code)
                        .with_span(p.span),
                );
            }
            ProbeKind::PmuxSelect => {
                for bit in 0..p.lits.len() {
                    let b64 = u64::try_from(bit).expect("fits");
                    let zero = mine.iter().find(|t| t.value == b64 * 2).filter(decided);
                    let one = mine.iter().find(|t| t.value == b64 * 2 + 1).filter(decided);
                    let (Some(zero), Some(one)) = (zero, one) else {
                        continue;
                    };
                    if zero.reached == one.reached {
                        continue;
                    }
                    unreachable += 1;
                    let message = if one.reached {
                        format!(
                            "select bit {bit} of pmux `{}` is always 1 {within}: every other case is dead",
                            p.name
                        )
                    } else {
                        format!(
                            "select bit {bit} of pmux `{}` is never 1 {within}: case {bit} is dead",
                            p.name
                        )
                    };
                    diags.push(
                        Diagnostic::warning(message)
                            .with_code("F0021")
                            .with_span(p.span),
                    );
                }
            }
            ProbeKind::FsmState => {
                let dead: Vec<String> = mine
                    .iter()
                    .filter(|t| !t.reached && !t.unknown)
                    .map(|t| t.value.to_string())
                    .collect();
                if dead.is_empty() || dead.len() == mine.len() {
                    continue;
                }
                unreachable += dead.len();
                diags.push(
                    Diagnostic::warning(format!(
                        "FSM state `{}` never takes value{} {} {within}",
                        p.name,
                        if dead.len() == 1 { "" } else { "s" },
                        dead.join(", ")
                    ))
                    .with_code("F0023")
                    .with_span(p.span),
                );
            }
        }
    }
    ReachReport {
        checked: targets.len(),
        unreachable,
        diags,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::builder::ModuleBuilder;
    use crate::ir::{CellKind, Name, Reset, Type};
    use crate::logic::Logic;
    use crate::source::{SourceMap, Span};

    fn span() -> Span {
        let mut map = SourceMap::new();
        let id = map.add("t", "").unwrap();
        Span::new(id, 0, 0)
    }

    /// A 2-bit state register cycling 0 -> 1 -> 2 -> 0 with a mux whose
    /// select is `q == 3`, a dead enable, and a live mux on `in`.
    fn fsm() -> (Design, ModuleId) {
        let mut b = ModuleBuilder::new("fsm", span());
        let clk = b.input("clk", Type::bit());
        let inp = b.input("in", Type::bit());
        let q = b.output_reg("q", Type::bits(2));
        b.net_attr(q, "fsm_state", 1);
        let y = b.output("y", Type::bits(2));
        let z = b.output("z", Type::bits(2));
        let held = b.output("held", Type::bit());
        let (qv, clkv, inv) = (b.net(q), b.net(clk), b.net(inp));
        let one = b.const_u64(2, 1);
        let two = b.const_u64(2, 2);
        let zero = b.const_u64(2, 0);
        let three = b.const_u64(2, 3);
        let inc = b.add(qv, one);
        let at_two = b.eq(qv, two);
        let next = b.mux(at_two, zero, inc);
        let f = b.const_bit(false);
        b.cell(
            "state",
            CellKind::Dff {
                clk_pos: true,
                has_enable: false,
                reset: Some(Reset {
                    asynchronous: false,
                    active_high: true,
                    value: Logic::zero(2),
                }),
            },
            vec![
                (Name::new("clk"), clkv),
                (Name::new("d"), next),
                (Name::new("rst"), f),
            ],
            vec![(Name::new("q"), q)],
        );
        let dead_sel = b.eq(qv, three);
        let w = b.output("w", Type::bits(2));
        let dead_tern = b.mux(dead_sel, one, qv);
        b.assign(w, dead_tern);
        b.cell(
            "dead_mux",
            CellKind::Mux,
            vec![
                (Name::new("a"), qv),
                (Name::new("b"), zero),
                (Name::new("s"), dead_sel),
            ],
            vec![(Name::new("y"), y)],
        );
        b.cell(
            "live_mux",
            CellKind::Mux,
            vec![
                (Name::new("a"), qv),
                (Name::new("b"), zero),
                (Name::new("s"), inv),
            ],
            vec![(Name::new("y"), z)],
        );
        b.cell(
            "never",
            CellKind::Dff {
                clk_pos: true,
                has_enable: true,
                reset: None,
            },
            vec![
                (Name::new("clk"), clkv),
                (Name::new("d"), inv),
                (Name::new("en"), dead_sel),
            ],
            vec![(Name::new("q"), held)],
        );
        let mut d = Design::new();
        let id = d.add_module(b.finish());
        (d, id)
    }

    #[test]
    fn reports_dead_select_enable_and_state() {
        let (d, id) = fsm();
        let report = reach_lint(&d, id, &ReachOptions::default());
        let mut msgs: Vec<String> = report
            .diags
            .iter()
            .map(|d| format!("{} {}", d.code.unwrap(), d.message))
            .collect();
        msgs.sort();
        assert_eq!(
            msgs,
            [
                "F0021 condition of this `?:` is constant 0 within 21 frames: the then branch is never taken",
                "F0021 select of mux `dead_mux` is constant 0 within 21 frames: input `b` is never chosen",
                "F0022 enable of flip-flop `never` is never 1 within 21 frames: the register never loads",
                "F0023 FSM state `q` never takes value 3 within 21 frames",
            ]
        );
        // The `?:` computing `next` is live (q reaches 2) and so is
        // `live_mux`; neither is reported.
        assert_eq!(report.unreachable, 4);
        // Two `?:`, two muxes, one enable (two values each), one 2-bit FSM
        // state (four values).
        assert_eq!(report.checked, 2 * 5 + 4);
    }

    #[test]
    fn errors_come_back_as_diagnostics() {
        let mut b = ModuleBuilder::new("bad", span());
        b.inout("io", Type::bit());
        let mut d = Design::new();
        let id = d.add_module(b.finish());
        let report = reach_lint(&d, id, &ReachOptions::default());
        assert!(report.diags.has_errors());
        assert_eq!(report.checked, 0);
    }
}
