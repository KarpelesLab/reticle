//! Combinational and sequential equivalence checking.
//!
//! Two modules with the same interface are equivalent when, fed the same
//! inputs, they produce the same outputs. The classic construction is the
//! *miter*: both modules share their inputs, and an output-comparison
//! network flags any disagreement. [`Miter`] is that construction on
//! blasted frames, exposed as a [`Transition`] so the model-checking
//! engines can run on it unchanged:
//!
//! - **Combinational** (neither module has state): the miter has one
//!   frame; a satisfying assignment of "some output differs" is a
//!   distinguishing input vector. This is BMC at depth 0.
//! - **Sequential** (either module has state): the miter's state is the
//!   union of both states, started from matched reset states, and the
//!   comparison must hold in every frame. BMC to
//!   [`EquivOptions::depth`] looks for a distinguishing input sequence;
//!   if none exists, k-induction (up to [`EquivOptions::max_k`]) tries
//!   to prove that none ever will.
//!
//! Sequential equivalence is only as strong as its induction: two
//! implementations with different state encodings often *are* equivalent
//! without "outputs agree" being inductive. When the two share register
//! names, [`EquivOptions::match_state_by_name`] adds the equality of every
//! same-named, same-width state element as an extra property; the
//! conjunction is usually 1-inductive, which turns an `Unknown` into a
//! proof. A trace that violates only such a state property is still
//! reported as [`EquivOutcome::Different`], since the checker cannot tell
//! whether the outputs would have diverged later; the failing property's
//! name says which register.
//!
//! Ports are matched by name; a port missing on one side, or one whose
//! width or direction differs, is an error (`F0017`). `assume` properties
//! of both modules constrain the shared inputs. Their own `assert`
//! properties are not checked here.

use std::fmt::Write;

use super::blast::{BlastOptions, BlastedFrame, Blaster, PropertyLit, Signal, StateSlot};
use super::bmc::{BmcOptions, BmcOutcome, bmc_system};
use super::cnf::CnfBuilder;
use super::induct::{InductOptions, InductOutcome, induct_system};
use super::sat::Lit;
use super::trace::Trace;
use super::unroll::{InitMode, Transition};
use crate::diag::{Diagnostic, Diagnostics};
use crate::ir::{Design, ModuleId, PortDir};

/// Options of [`check_equivalent`].
#[derive(Clone, Debug)]
pub struct EquivOptions {
    /// Frames of bounded search for a distinguishing sequence.
    pub depth: u32,
    /// Largest induction depth tried for a proof.
    pub max_k: u32,
    /// How the initial states are constrained. With [`InitMode::Reset`],
    /// every state element must have a reset value (`F0018`).
    pub init: InitMode,
    /// Also require same-named state elements to stay equal.
    pub match_state_by_name: bool,
    /// Conflicts allowed per SAT call before giving up.
    pub conflict_limit: Option<u64>,
    /// Options of the bit-blaster.
    pub blast: BlastOptions,
}

impl Default for EquivOptions {
    fn default() -> Self {
        EquivOptions {
            depth: 20,
            max_k: 10,
            init: InitMode::Reset,
            match_state_by_name: true,
            conflict_limit: None,
            blast: BlastOptions::default(),
        }
    }
}

/// How equivalence was established.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EquivProof {
    /// Both modules are combinational; the outputs agree for every input.
    Combinational,
    /// The comparison is `k`-inductive from the matched reset states.
    Induction {
        /// The induction depth.
        k: u32,
    },
}

/// The verdict of an equivalence check.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EquivOutcome {
    /// The modules agree on every input (sequence).
    Equivalent(EquivProof),
    /// A distinguishing input (sequence) exists.
    Different {
        /// The frame in which an output (or matched state) differs.
        frame: u32,
        /// The names of the outputs or state elements that differ.
        properties: Vec<String>,
        /// The trace, with both modules' values.
        trace: Trace,
    },
    /// Not settled: no difference within the bound, no inductive proof,
    /// or a diagnostic prevented the check.
    Unknown {
        /// The bounded depth that found no difference, if the check ran.
        depth: Option<u32>,
    },
}

/// What [`check_equivalent`] returns.
#[derive(Clone, Debug)]
pub struct EquivReport {
    /// The verdict.
    pub outcome: EquivOutcome,
    /// Warnings, and errors when the check could not run.
    pub diags: Diagnostics,
}

impl EquivReport {
    /// True when the modules were shown equivalent.
    pub fn equivalent(&self) -> bool {
        matches!(self.outcome, EquivOutcome::Equivalent(_))
    }

    /// Renders the verdict as text, with the distinguishing trace if any.
    /// Diagnostics are not included.
    pub fn render(&self, a: &str, b: &str) -> String {
        let mut out = String::new();
        let _ = write!(out, "equivalence {a} vs {b}: ");
        match &self.outcome {
            EquivOutcome::Equivalent(EquivProof::Combinational) => {
                let _ = writeln!(out, "EQUIVALENT (combinational)");
            }
            EquivOutcome::Equivalent(EquivProof::Induction { k }) => {
                let _ = writeln!(out, "EQUIVALENT ({k}-inductive from reset)");
            }
            EquivOutcome::Different {
                frame,
                properties,
                trace,
            } => {
                let list: Vec<String> = properties.iter().map(|p| format!("`{p}`")).collect();
                let _ = writeln!(out, "DIFFERENT at frame {frame} ({})", list.join(", "));
                out.push_str("  trace:\n");
                for line in trace.render().lines() {
                    let _ = writeln!(out, "    {line}");
                }
            }
            EquivOutcome::Unknown { depth: Some(d) } => {
                let _ = writeln!(
                    out,
                    "UNKNOWN (no difference to depth {d}, no inductive proof)"
                );
            }
            EquivOutcome::Unknown { depth: None } => {
                let _ = writeln!(out, "UNKNOWN");
            }
        }
        out
    }
}

/// The miter of two blasted modules: shared inputs, both states, and a
/// property per output pair (plus per matched state pair).
pub struct Miter<'a> {
    a: &'a Blaster<'a>,
    b: &'a Blaster<'a>,
    /// Shared input ports as `(name, index in a, index in b)`.
    shared: Vec<(String, usize, usize)>,
    /// Matched output ports as `(name, index in a, index in b)`.
    outputs: Vec<(String, usize, usize)>,
    /// Matched state slots as `(name, index in a, index in b)`.
    states: Vec<(String, usize, usize)>,
}

impl<'a> Miter<'a> {
    /// Builds the miter of `a` and `b`, reporting interface mismatches.
    /// State matching happens only when `match_state` is set.
    pub fn new(
        a: &'a Blaster<'a>,
        b: &'a Blaster<'a>,
        match_state: bool,
    ) -> Result<Miter<'a>, Diagnostics> {
        let mut diags = Diagnostics::new();
        let (ma, mb) = (a.module(), b.module());
        let mut shared = Vec::new();
        let mut outputs = Vec::new();
        for (pa_i, pa) in ma.ports.iter().enumerate() {
            let _ = pa_i;
            let Some(pb) = mb.port(pa.name.as_str()) else {
                diags.push(
                    Diagnostic::error(format!(
                        "port `{}` of `{}` has no counterpart in `{}`",
                        pa.name, ma.name, mb.name
                    ))
                    .with_code("F0017")
                    .with_span(pa.span),
                );
                continue;
            };
            let (wa, wb) = (ma.nets[pa.net].ty.width(), mb.nets[pb.net].ty.width());
            if pa.dir != pb.dir || wa != wb {
                diags.push(
                    Diagnostic::error(format!(
                        "port `{}` is `{}` {} bits in `{}` but `{}` {} bits in `{}`",
                        pa.name,
                        pa.dir.keyword(),
                        wa.unwrap_or(0),
                        ma.name,
                        pb.dir.keyword(),
                        wb.unwrap_or(0),
                        mb.name
                    ))
                    .with_code("F0017")
                    .with_label(pa.span, "first")
                    .with_secondary(pb.span, "second"),
                );
                continue;
            }
            match pa.dir {
                PortDir::In => {
                    let ia = a
                        .input_slots()
                        .iter()
                        .position(|s| s.is_port && s.net == pa.net);
                    let ib = b
                        .input_slots()
                        .iter()
                        .position(|s| s.is_port && s.net == pb.net);
                    if let (Some(ia), Some(ib)) = (ia, ib) {
                        shared.push((pa.name.to_string(), ia, ib));
                    }
                }
                PortDir::Out => {
                    let oa = a.output_slots().iter().position(|s| s.net == pa.net);
                    let ob = b.output_slots().iter().position(|s| s.net == pb.net);
                    if let (Some(oa), Some(ob)) = (oa, ob) {
                        outputs.push((pa.name.to_string(), oa, ob));
                    }
                }
                PortDir::InOut => {}
            }
        }
        for pb in &mb.ports {
            if ma.port(pb.name.as_str()).is_none() {
                diags.push(
                    Diagnostic::error(format!(
                        "port `{}` of `{}` has no counterpart in `{}`",
                        pb.name, mb.name, ma.name
                    ))
                    .with_code("F0017")
                    .with_span(pb.span),
                );
            }
        }
        if diags.has_errors() {
            return Err(diags);
        }
        let mut states = Vec::new();
        if match_state {
            for (ia, sa) in a.state_slots().iter().enumerate() {
                if let Some(ib) = b
                    .state_slots()
                    .iter()
                    .position(|sb| sb.name == sa.name && sb.width == sa.width)
                {
                    states.push((sa.name.clone(), ia, ib));
                }
            }
        }
        Ok(Miter {
            a,
            b,
            shared,
            outputs,
            states,
        })
    }

    /// True when either side has state.
    pub fn is_sequential(&self) -> bool {
        self.a.has_state() || self.b.has_state()
    }

    /// The state elements of `a` and `b` without an initial value under
    /// [`InitMode::Reset`], as `(module, name)`.
    pub fn uninitialised(&self) -> Vec<(String, String)> {
        let mut out = Vec::new();
        for (bl, m) in [(self.a, self.a.module()), (self.b, self.b.module())] {
            for s in bl.state_slots() {
                if s.init.is_none() {
                    out.push((m.name.to_string(), s.name.clone()));
                }
            }
        }
        out
    }

    fn prefixed(prefix: &str, mut s: Signal) -> Signal {
        s.name = format!("{prefix}.{}", s.name);
        s
    }
}

impl Transition for Miter<'_> {
    fn name(&self) -> String {
        format!("{} vs {}", self.a.module().name, self.b.module().name)
    }

    fn state_slots(&self) -> Vec<StateSlot> {
        let mut slots: Vec<StateSlot> = self
            .a
            .state_slots()
            .iter()
            .cloned()
            .map(|mut s| {
                s.name = format!("{}.{}", self.a.module().name, s.name);
                s
            })
            .collect();
        slots.extend(self.b.state_slots().iter().cloned().map(|mut s| {
            s.name = format!("{}.{}", self.b.module().name, s.name);
            s
        }));
        slots
    }

    fn blast_frame(&self, cnf: &mut CnfBuilder, state: Option<&[Vec<Lit>]>) -> BlastedFrame {
        let na = self.a.state_slots().len();
        let (sa, sb) = match state {
            Some(s) => (Some(&s[..na]), Some(&s[na..])),
            None => (None, None),
        };
        // Inputs: shared ports get one set of fresh variables; everything
        // else is free on its own side.
        let fresh = |cnf: &mut CnfBuilder, bl: &Blaster, i: usize| -> Vec<Lit> {
            let w = bl.module().nets[bl.input_slots()[i].net]
                .ty
                .width()
                .unwrap_or(0);
            cnf.new_vars(usize::try_from(w).expect("width fits usize"))
        };
        let ins_a: Vec<Vec<Lit>> = (0..self.a.input_slots().len())
            .map(|i| fresh(cnf, self.a, i))
            .collect();
        let mut ins_b: Vec<Vec<Lit>> = (0..self.b.input_slots().len())
            .map(|i| fresh(cnf, self.b, i))
            .collect();
        for &(_, ia, ib) in &self.shared {
            ins_b[ib] = ins_a[ia].clone();
        }
        let fa = self.a.blast_frame(cnf, sa, Some(&ins_a));
        let fb = self.b.blast_frame(cnf, sb, Some(&ins_b));
        let (pa, pb) = (self.a.module().name.as_str(), self.b.module().name.as_str());

        let mut frame = BlastedFrame::default();
        let shared_a: Vec<usize> = self.shared.iter().map(|s| s.1).collect();
        let shared_b: Vec<usize> = self.shared.iter().map(|s| s.2).collect();
        for (i, s) in fa.inputs.iter().enumerate() {
            if shared_a.contains(&i) {
                frame.inputs.push(s.clone());
            } else {
                frame.inputs.push(Self::prefixed(pa, s.clone()));
            }
        }
        for (i, s) in fb.inputs.iter().enumerate() {
            if !shared_b.contains(&i) {
                frame.inputs.push(Self::prefixed(pb, s.clone()));
            }
        }
        frame
            .state
            .extend(fa.state.iter().cloned().map(|s| Self::prefixed(pa, s)));
        frame
            .state
            .extend(fb.state.iter().cloned().map(|s| Self::prefixed(pb, s)));
        frame.next_state.extend(fa.next_state.iter().cloned());
        frame.next_state.extend(fb.next_state.iter().cloned());
        frame
            .outputs
            .extend(fa.outputs.iter().cloned().map(|s| Self::prefixed(pa, s)));
        frame
            .outputs
            .extend(fb.outputs.iter().cloned().map(|s| Self::prefixed(pb, s)));
        frame
            .internal
            .extend(fa.internal.iter().cloned().map(|s| Self::prefixed(pa, s)));
        frame
            .internal
            .extend(fb.internal.iter().cloned().map(|s| Self::prefixed(pb, s)));
        for (name, oa, ob) in &self.outputs {
            let (x, y) = (&fa.outputs[*oa], &fb.outputs[*ob]);
            let lit = cnf.eq_vec(&x.lits, &y.lits);
            frame.asserts.push(PropertyLit {
                name: name.clone(),
                span: x.span,
                lit,
            });
        }
        for (name, ia, ib) in &self.states {
            let (x, y) = (&fa.state[*ia], &fb.state[*ib]);
            let lit = cnf.eq_vec(&x.lits, &y.lits);
            frame.asserts.push(PropertyLit {
                name: format!("state {name}"),
                span: x.span,
                lit,
            });
        }
        frame.assumes.extend(fa.assumes.iter().cloned());
        frame.assumes.extend(fb.assumes.iter().cloned());
        frame
    }
}

/// Checks whether modules `a` and `b` of `design` are equivalent.
pub fn check_equivalent(
    design: &Design,
    a: ModuleId,
    b: ModuleId,
    options: &EquivOptions,
) -> EquivReport {
    let mut diags = Diagnostics::new();
    let unknown = |diags: Diagnostics| EquivReport {
        outcome: EquivOutcome::Unknown { depth: None },
        diags,
    };
    let ba = match Blaster::new(design.module(a), &options.blast) {
        Ok(b) => b,
        Err(d) => return unknown(d),
    };
    let bb = match Blaster::new(design.module(b), &options.blast) {
        Ok(b) => b,
        Err(d) => return unknown(d),
    };
    diags.append(&mut ba.diagnostics().clone());
    diags.append(&mut bb.diagnostics().clone());
    let miter = match Miter::new(&ba, &bb, options.match_state_by_name) {
        Ok(m) => m,
        Err(mut d) => {
            diags.append(&mut d);
            return unknown(diags);
        }
    };
    let (ma, mb) = (design.module(a), design.module(b));

    if !miter.is_sequential() {
        let opts = BmcOptions {
            depth: 0,
            init: InitMode::Free,
            conflict_limit: options.conflict_limit,
            blast: options.blast.clone(),
        };
        let mut r = bmc_system(&miter, &opts);
        diags.append(&mut r.diags);
        let outcome = match r.outcome {
            BmcOutcome::Safe { .. } => EquivOutcome::Equivalent(EquivProof::Combinational),
            BmcOutcome::Failed {
                frame,
                properties,
                trace,
            } => EquivOutcome::Different {
                frame,
                properties,
                trace,
            },
            BmcOutcome::Unknown { .. } => EquivOutcome::Unknown { depth: None },
        };
        return EquivReport { outcome, diags };
    }

    if options.init == InitMode::Reset {
        let missing = miter.uninitialised();
        if !missing.is_empty() {
            let list: Vec<String> = missing.iter().map(|(m, s)| format!("`{m}.{s}`")).collect();
            diags.push(
                Diagnostic::error(format!(
                    "sequential equivalence needs a reset value for every state element; missing for {}",
                    list.join(", ")
                ))
                .with_code("F0018")
                .with_label(ma.span, "first module")
                .with_secondary(mb.span, "second module")
                .with_note("use `InitMode::Zero` or `InitMode::Free` to check from other initial states"),
            );
            return unknown(diags);
        }
    }

    let opts = BmcOptions {
        depth: options.depth,
        init: options.init,
        conflict_limit: options.conflict_limit,
        blast: options.blast.clone(),
    };
    let mut r = bmc_system(&miter, &opts);
    diags.append(&mut r.diags);
    match r.outcome {
        BmcOutcome::Failed {
            frame,
            properties,
            trace,
        } => {
            return EquivReport {
                outcome: EquivOutcome::Different {
                    frame,
                    properties,
                    trace,
                },
                diags,
            };
        }
        BmcOutcome::Unknown { .. } => return unknown(diags),
        BmcOutcome::Safe { .. } => {}
    }
    let opts = InductOptions {
        max_k: options.max_k,
        init: options.init,
        unique_states: true,
        conflict_limit: options.conflict_limit,
        blast: options.blast.clone(),
    };
    let mut r = induct_system(&miter, &opts);
    diags.append(&mut r.diags);
    let outcome = match r.outcome {
        InductOutcome::Proved { k } => EquivOutcome::Equivalent(EquivProof::Induction { k }),
        InductOutcome::Failed {
            frame,
            properties,
            trace,
        } => EquivOutcome::Different {
            frame,
            properties,
            trace,
        },
        InductOutcome::Unknown { .. } => EquivOutcome::Unknown {
            depth: Some(options.depth),
        },
    };
    EquivReport { outcome, diags }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::builder::ModuleBuilder;
    use crate::ir::{CellKind, Module, Name, Reset, Type};
    use crate::logic::Logic;
    use crate::source::{SourceMap, Span};

    fn span() -> Span {
        let mut map = SourceMap::new();
        let id = map.add("t", "").unwrap();
        Span::new(id, 0, 0)
    }

    /// A 4-bit adder: `y = a + b`, either as one `add` or bit by bit.
    fn adder(name: &str, ripple: bool, bug: bool) -> Module {
        let mut b = ModuleBuilder::new(name, span());
        let a = b.input("a", Type::bits(4));
        let bn = b.input("b", Type::bits(4));
        let y = b.output("y", Type::bits(4));
        let (av, bv) = (b.net(a), b.net(bn));
        if !ripple {
            let s = b.add(av, bv);
            b.assign(y, s);
        } else {
            let mut carry = b.const_bit(false);
            let mut bits = Vec::new();
            for i in 0..4 {
                let ai = b.slice(av, i, i);
                let bi = b.slice(bv, i, i);
                let x = b.xor(ai, bi);
                let s = b.xor(x, carry);
                let c1 = b.and(ai, bi);
                let c2 = b.and(x, carry);
                let c = if bug && i == 2 { c1 } else { b.or(c1, c2) };
                bits.push(s);
                carry = c;
            }
            bits.reverse();
            let cat = b.concat(bits);
            b.assign(y, cat);
        }
        b.finish()
    }

    fn design(mods: Vec<Module>) -> (Design, Vec<ModuleId>) {
        let mut d = Design::new();
        let ids = mods.into_iter().map(|m| d.add_module(m)).collect();
        (d, ids)
    }

    #[test]
    fn combinational_equivalence() {
        let (d, ids) = design(vec![adder("a1", false, false), adder("a2", true, false)]);
        let r = check_equivalent(&d, ids[0], ids[1], &EquivOptions::default());
        assert_eq!(
            r.outcome,
            EquivOutcome::Equivalent(EquivProof::Combinational)
        );
        let (d, ids) = design(vec![adder("a1", false, false), adder("a3", true, true)]);
        let r = check_equivalent(&d, ids[0], ids[1], &EquivOptions::default());
        let EquivOutcome::Different {
            frame,
            properties,
            trace,
        } = r.outcome
        else {
            panic!("{:?}", r.outcome);
        };
        assert_eq!(frame, 0);
        assert_eq!(properties, ["y"]);
        let f = &trace.frames[0];
        let a = f.inputs[0].1.to_u64().unwrap();
        let b = f.inputs[1].1.to_u64().unwrap();
        assert_eq!(f.outputs[0].0, "a1.y");
        assert_eq!(f.outputs[0].1.to_u64(), Some((a + b) & 15));
        assert_ne!(f.outputs[1].1.to_u64(), Some((a + b) & 15));
    }

    #[test]
    fn port_mismatch_is_an_error() {
        let mut b = ModuleBuilder::new("other", span());
        let a = b.input("a", Type::bits(4));
        let y = b.output("y", Type::bits(5));
        let av = b.net(a);
        let e = b.zext(av, 5);
        b.assign(y, e);
        let (d, ids) = design(vec![adder("a1", false, false), b.finish()]);
        let r = check_equivalent(&d, ids[0], ids[1], &EquivOptions::default());
        assert_eq!(r.outcome, EquivOutcome::Unknown { depth: None });
        let codes: Vec<_> = r.diags.iter().filter_map(|d| d.code).collect();
        assert_eq!(codes, ["F0017", "F0017"]);
    }

    /// A counter with enable; `variant` picks the implementation and
    /// `reset` whether the flop has a reset value.
    fn counter(name: &str, variant: u8, reset: bool) -> Module {
        let mut b = ModuleBuilder::new(name, span());
        let clk = b.input("clk", Type::bit());
        let en = b.input("en", Type::bit());
        let q = b.output("q", Type::bits(4));
        let (qv, env, clkv) = (b.net(q), b.net(en), b.net(clk));
        let d = match variant {
            0 => {
                let one = b.const_u64(4, 1);
                let inc = b.add(qv, one);
                b.mux(env, inc, qv)
            }
            1 => {
                let e4 = b.zext(env, 4);
                b.add(qv, e4)
            }
            _ => {
                // Buggy: counts by two once q is 8 or more.
                let one = b.const_u64(4, 1);
                let two = b.const_u64(4, 2);
                let eight = b.const_u64(4, 8);
                let big = b.ge(qv, eight);
                let step = b.mux(big, two, one);
                let inc = b.add(qv, step);
                b.mux(env, inc, qv)
            }
        };
        let f = b.const_bit(false);
        let inputs = if reset {
            vec![
                (Name::new("clk"), clkv),
                (Name::new("d"), d),
                (Name::new("rst"), f),
            ]
        } else {
            vec![(Name::new("clk"), clkv), (Name::new("d"), d)]
        };
        b.cell(
            "ff",
            CellKind::Dff {
                clk_pos: true,
                has_enable: false,
                reset: reset.then(|| Reset {
                    asynchronous: false,
                    active_high: true,
                    value: Logic::zero(4),
                }),
            },
            inputs,
            vec![(Name::new("q"), q)],
        );
        b.finish()
    }

    #[test]
    fn sequential_equivalence() {
        let (d, ids) = design(vec![counter("c1", 0, true), counter("c2", 1, true)]);
        let r = check_equivalent(&d, ids[0], ids[1], &EquivOptions::default());
        // Equal states imply equal next states: 1-inductive.
        assert_eq!(
            r.outcome,
            EquivOutcome::Equivalent(EquivProof::Induction { k: 1 }),
            "{:?}",
            r.diags
        );
        // Without state matching the outputs alone are still inductive
        // here (q is the output).
        let opts = EquivOptions {
            match_state_by_name: false,
            ..EquivOptions::default()
        };
        let r = check_equivalent(&d, ids[0], ids[1], &opts);
        assert!(matches!(
            r.outcome,
            EquivOutcome::Equivalent(EquivProof::Induction { .. })
        ));

        let (d, ids) = design(vec![counter("c1", 0, true), counter("c3", 2, true)]);
        let r = check_equivalent(&d, ids[0], ids[1], &EquivOptions::default());
        let EquivOutcome::Different {
            frame,
            properties,
            trace,
        } = r.outcome
        else {
            panic!("{:?}", r.outcome);
        };
        // q reaches 8 after eight enabled frames; the next step differs.
        assert_eq!(frame, 9);
        assert!(properties.contains(&"q".to_string()));
        assert_eq!(trace.len(), 10);
        assert_eq!(trace.frames[9].outputs[0].1.to_u64(), Some(9));
        assert_eq!(trace.frames[9].outputs[1].1.to_u64(), Some(10));
    }

    #[test]
    fn missing_reset_values_are_reported() {
        let (d, ids) = design(vec![counter("c1", 0, true), counter("c2", 1, false)]);
        let r = check_equivalent(&d, ids[0], ids[1], &EquivOptions::default());
        assert_eq!(r.outcome, EquivOutcome::Unknown { depth: None });
        assert!(r.diags.iter().any(|d| d.code == Some("F0018")));
        // From a zero initial state the check goes through.
        let opts = EquivOptions {
            init: InitMode::Zero,
            ..EquivOptions::default()
        };
        let r = check_equivalent(&d, ids[0], ids[1], &opts);
        assert!(
            matches!(r.outcome, EquivOutcome::Equivalent(_)),
            "{:?}",
            r.outcome
        );
        // From free initial states the counters can start apart.
        let opts = EquivOptions {
            init: InitMode::Free,
            ..EquivOptions::default()
        };
        let r = check_equivalent(&d, ids[0], ids[1], &opts);
        assert!(
            matches!(r.outcome, EquivOutcome::Different { frame: 0, .. }),
            "{:?}",
            r.outcome
        );
    }
}
