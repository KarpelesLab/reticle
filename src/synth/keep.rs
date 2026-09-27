//! What `keep` promises, and the one place that decides it.
//!
//! A user who writes `(* keep *)` is nearly always keeping something for
//! an observer the compiler cannot see: a testbench, a logic analyser, a
//! probe point, a downstream tool, or a measurement of the silicon itself.
//! Deleting it anyway is a wrong answer with no error message, so this
//! module fixes what the attribute means and every pass asks it rather
//! than testing an attribute by hand.
//!
//! The spellings are [`crate::ir::KEEP_ATTRS`] — `keep`, `dont_touch`,
//! `mark_debug`, `preserve`, `noprune` and the `syn_` forms — compared
//! without case or separators, with a textual `"false"` read as off.
//!
//! # Where the attribute lands
//!
//! Neither front end invents a place to put an attribute: it goes on the
//! IR object the annotated declaration became.
//!
//! | Written on | Lands on |
//! |---|---|
//! | a Verilog `reg`, `wire`, `logic` or port declaration | the **net** |
//! | a VHDL `attribute keep of s : signal is true` | the **net** |
//! | an unpacked array / VHDL array signal | the **memory** |
//! | an `always` or `always_ff` block, a VHDL process | the **process**, and from there onto the cells it infers |
//! | a continuous `assign` | the **assign** (or the `tristate` cell it became) |
//! | a module instance | the **instance** |
//! | a `module` or `entity` | the **module** |
//!
//! Nothing lands on a **cell** from source: cells are inferred, so a
//! register annotated in Verilog carries its `keep` on the net that is the
//! flip-flop's `q`, never on the `dff`. A pass that reads only
//! `cell.attrs` therefore sees nothing, which is exactly how a kept
//! register came to be deleted in silence. [`cell_is_kept`] is the test to
//! use instead.
//!
//! # What `keep` promises
//!
//! **A kept object is in the netlist that synthesis hands on.** In detail:
//!
//! - A kept **net** survives dead-code elimination even when nothing reads
//!   it, its value is never substituted into its readers, and it is never
//!   aliased into another net.
//! - **A kept net driven by a state element keeps that state element.** A
//!   flip-flop or latch whose `q` is kept is not folded to a constant, not
//!   removed for want of a reader, and not merged into another register.
//! - A kept **cell** is not removed, not folded into an assignment, and not
//!   merged with an identical one.
//! - A kept **assign** is not removed, not merged and not propagated into
//!   its readers.
//! - A kept **memory** is not removed.
//!
//! # What still applies
//!
//! `keep` is a promise about the object, not about everything that feeds
//! it, and not a request to stop compiling:
//!
//! - The expression cone driving a kept object is still constant-folded,
//!   narrowed and shared. Keeping a register does not freeze the logic
//!   computing its `d`.
//! - A kept flip-flop still has a constant enable dropped and an enable or
//!   synchronous reset lifted out of its `d` mux. Those rewrites leave the
//!   same flip-flop with the same behaviour on the same net; they are how
//!   it reaches a fabric's `CE` and `SR` pins at all.
//! - A **combinational** cell driving a kept net may still fold to a
//!   constant assignment, and technology mapping still covers it with
//!   lookup tables or library cells. The net keeps its name and its value,
//!   which is everything an observer of a wire can ask for.
//!
//! The asymmetry between the last two points is the whole of the rule.
//! Folding a combinational cell leaves the kept net with the value it
//! always had. Folding a flip-flop does not: a register's value before its
//! first clock edge is a property of the **fabric**, not of the IR — an
//! ECP5 releases every flip-flop into its `REGSET` state whatever the
//! Verilog initialiser said — so "its data is a constant, therefore it is
//! that constant" is a claim the compiler cannot honour on a real part.
//! That is the difference a probe measures, and `keep` is how a user asks
//! to measure it.
//!
//! # Diagnostics
//!
//! [`audit`] warns with `S0034` when a keep cannot be honoured where it was
//! written, or when an attribute reads like a keep this crate does not
//! know. Silence is what a misplaced `keep` used to get, and silence is
//! what cost the time.

use crate::diag::{Diagnostic, Diagnostics};
use crate::ir::{Attrs, Cell, Design, Module, ProcessKind, looks_like_keep};

/// `S0034`: a keep attribute synthesis cannot honour as written.
const KEEP_NOT_HONOURED: &str = "S0034";

/// True when the object must survive optimisation.
pub(crate) fn is_kept(attrs: &Attrs) -> bool {
    attrs.is_kept()
}

/// True when the cell must survive optimisation: it is annotated itself,
/// or it is a state element (or a black box) driving a net that is.
///
/// Only a non-combinational cell inherits the keep of the nets it drives,
/// because only a state element decides *when* its output takes a value;
/// see the module docs for why folding one is not the same as folding a
/// gate.
pub(crate) fn cell_is_kept(m: &Module, cell: &Cell) -> bool {
    is_kept(&cell.attrs)
        || (!cell.kind.is_combinational()
            && cell.outputs.iter().any(|(_, n)| is_kept(&m.nets[*n].attrs)))
}

/// Reports every keep attribute synthesis cannot honour where it was
/// written, and every attribute that reads like one it does not know.
///
/// Run once, before lowering: a `keep` on a process is gone by the time the
/// optimiser runs, and an attribute the front end could not place is worth
/// naming at its own span rather than never.
pub fn audit(design: &Design, diags: &mut Diagnostics) {
    for (_, m) in design.modules.iter() {
        // A black box has no logic to keep or to drop, so nothing about a
        // keep inside one is honoured or dishonoured.
        if m.blackbox {
            continue;
        }
        if let Some((name, true)) = m.attrs.keep_attr() {
            diags.push(
                Diagnostic::warning(format!(
                    "`{name}` on module `{}` keeps nothing",
                    m.name
                ))
                .with_code(KEEP_NOT_HONOURED)
                .with_span(m.span)
                .with_note(
                    "`keep` keeps an object inside a module; write `keep_hierarchy` to stop the module being inlined",
                ),
            );
        }
        for (_, p) in m.processes.iter() {
            if !matches!(p.kind, ProcessKind::Initial) {
                continue;
            }
            if let Some((name, true)) = p.attrs.keep_attr() {
                diags.push(
                    Diagnostic::warning(format!("`{name}` on an `initial` block keeps nothing"))
                        .with_code(KEEP_NOT_HONOURED)
                        .with_span(p.span)
                        .with_note(
                            "an `initial` block becomes initial values rather than logic; annotate the declaration of the register instead",
                        ),
                );
            }
        }
        for (_, inst) in m.instances.iter() {
            if let Some((name, true)) = inst.attrs.keep_attr() {
                diags.push(
                    Diagnostic::warning(format!(
                        "`{name}` on instance `{}` does not keep a hierarchy boundary",
                        inst.name
                    ))
                    .with_code(KEEP_NOT_HONOURED)
                    .with_span(inst.span)
                    .with_note(
                        "an instance is never optimised away, but it is inlined unless it or its module carries `keep_hierarchy`",
                    ),
                );
            }
        }
        unknown_spellings(m, diags);
    }
}

/// Reports attribute names that read like a keep this crate does not
/// honour, wherever they sit in `m`.
fn unknown_spellings(m: &Module, diags: &mut Diagnostics) {
    let mut report = |name: &str, what: &str, span| {
        diags.push(
            Diagnostic::warning(format!(
                "`{name}` on {what} is not an attribute Reticle honours"
            ))
            .with_code(KEEP_NOT_HONOURED)
            .with_span(span)
            .with_note(format!(
                "to keep an object out of the optimiser's reach write one of: {}",
                crate::ir::KEEP_ATTRS.join(", ")
            )),
        );
    };
    let odd = |attrs: &Attrs| -> Option<String> {
        attrs
            .iter()
            .map(|(k, _)| k.as_str())
            .find(|k| looks_like_keep(k))
            .map(str::to_owned)
    };
    if let Some(name) = odd(&m.attrs) {
        report(&name, &format!("module `{}`", m.name), m.span);
    }
    for (_, net) in m.nets.iter() {
        if let Some(name) = odd(&net.attrs) {
            report(&name, &format!("`{}`", net.name), net.span);
        }
    }
    for (_, mem) in m.memories.iter() {
        if let Some(name) = odd(&mem.attrs) {
            report(&name, &format!("`{}`", mem.name), mem.span);
        }
    }
    for (_, inst) in m.instances.iter() {
        if let Some(name) = odd(&inst.attrs) {
            report(&name, &format!("instance `{}`", inst.name), inst.span);
        }
    }
    for (_, p) in m.processes.iter() {
        if let Some(name) = odd(&p.attrs) {
            report(&name, "this process", p.span);
        }
    }
    for a in &m.assigns {
        if let Some(name) = odd(&a.attrs) {
            report(&name, "this assignment", a.span);
        }
    }
    for (_, c) in m.cells.iter() {
        if let Some(name) = odd(&c.attrs) {
            report(&name, &format!("cell `{}`", c.name), c.span);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::builder::ModuleBuilder;
    use crate::ir::{CellKind, Name, Type};
    use crate::source::{SourceMap, Span};

    fn span() -> Span {
        let mut map = SourceMap::new();
        let id = map.add("t", "").unwrap();
        Span::new(id, 0, 0)
    }

    /// The net of a flip-flop's `q` keeps the flip-flop; the net of a gate
    /// does not keep the gate.
    #[test]
    fn only_state_inherits_the_keep_of_its_output() {
        let span = span();
        let mut b = ModuleBuilder::new("m", span);
        let clk = b.input("clk", Type::bit());
        let q = b.add_reg("q", Type::bit());
        let y = b.add_net("y", Type::bit());
        b.net_attr(q, "dont_touch", 1);
        b.net_attr(y, "keep", 1);
        let clkn = b.net(clk);
        let zero = b.const_bit(false);
        let ff = b.cell(
            "q$ff",
            CellKind::Dff {
                clk_pos: true,
                has_enable: false,
                reset: None,
            },
            vec![(Name::new("clk"), clkn), (Name::new("d"), zero)],
            vec![(Name::new("q"), q)],
        );
        let gate = b.cell2("y$not", CellKind::Not, zero, zero, y);
        let m = b.finish();
        assert!(cell_is_kept(&m, &m.cells[ff]));
        assert!(!cell_is_kept(&m, &m.cells[gate]));
    }

    /// A keep nobody can honour, and a spelling nobody recognises, are both
    /// worth a warning: the alternative is what this whole module exists to
    /// stop, which is silence.
    #[test]
    fn a_keep_that_cannot_be_honoured_is_reported() {
        use crate::ir::{Design, ModuleRef, ProcessKind};

        let span = span();
        let mut b = ModuleBuilder::new("m", span);
        b.attr("keep", 1);
        let q = b.add_reg("q", Type::bit());
        b.net_attr(q, "keep_signal", 1);
        let zero = b.const_bit(false);
        let mut p = b.process(None, ProcessKind::Initial);
        p.attrs.set("dont_touch", 1);
        p.blocking(q, zero);
        b.end_process(p);
        let inst = b.instance("u0", ModuleRef::Unresolved(Name::new("leaf")), Vec::new());
        b.module_mut().instances[inst].attrs.set("preserve", 1);
        let mut design = Design::new();
        let id = design.add_module(b.finish());
        design.top = Some(id);

        let mut diags = crate::diag::Diagnostics::new();
        audit(&design, &mut diags);
        let messages: Vec<&str> = diags.iter().map(|d| d.message.as_str()).collect();
        assert_eq!(diags.iter().count(), 4, "{messages:?}");
        assert!(
            diags.iter().all(|d| d.code == Some("S0034")),
            "{messages:?}"
        );
        assert!(diags.iter().all(|d| !d.is_error()), "{messages:?}");
        assert!(
            messages
                .iter()
                .any(|m| m.contains("`keep` on module `m` keeps nothing")),
            "{messages:?}"
        );
        assert!(
            messages
                .iter()
                .any(|m| m.contains("`dont_touch` on an `initial` block keeps nothing")),
            "{messages:?}"
        );
        assert!(
            messages
                .iter()
                .any(|m| m.contains("does not keep a hierarchy boundary")),
            "{messages:?}"
        );
        assert!(
            messages
                .iter()
                .any(|m| m.contains("`keep_signal` on `q` is not an attribute Reticle honours")),
            "{messages:?}"
        );
    }

    /// The attribute in the place it works says nothing at all, and neither
    /// does a black box, whose contents are somebody else's.
    #[test]
    fn a_keep_that_works_is_silent() {
        use crate::ir::Design;

        let span = span();
        let mut b = ModuleBuilder::new("m", span);
        let q = b.add_reg("q", Type::bit());
        b.net_attr(q, "keep", 1);
        b.attr("keep_hierarchy", 1);
        let mut design = Design::new();
        design.top = Some(design.add_module(b.finish()));

        let mut boxed = ModuleBuilder::new("b", span);
        boxed.attr("dont_touch", 1);
        boxed.blackbox();
        design.add_module(boxed.finish());

        let mut diags = crate::diag::Diagnostics::new();
        audit(&design, &mut diags);
        assert_eq!(diags.iter().count(), 0, "{}", diags.iter().count());
    }
}
