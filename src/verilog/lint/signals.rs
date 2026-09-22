//! Rules about signals: unused, undriven, multiply driven and implicit.

use std::collections::BTreeSet;

use crate::diag::{Diagnostics, Severity};
use crate::source::Span;
use crate::verilog::ast::{Direction, ExprKind, ModuleKind, NetType, RangeKind, SourceFile};

use super::facts::{DeclId, DeclKind, ModuleFacts, ProcId, UseContext, Write};
use super::width::const_eval;
use super::{Level, Lint, LintContext};

/// Attributes that mark a signal as deliberately unused or kept.
const KEEP_ATTRS: &[&str] = &[
    "keep",
    "dont_touch",
    "unused",
    "maybe_unused",
    "keep_hierarchy",
    "mark_debug",
];

/// True when the module's signals cannot be analysed reliably: an
/// interface exports its signals, a primitive has a truth table rather
/// than a body, and `.*` connects by name behind the linter's back.
fn opaque(m: &ModuleFacts<'_>) -> bool {
    matches!(m.module.kind, ModuleKind::Interface | ModuleKind::Primitive)
        || m.instances.iter().any(|i| i.wildcard)
}

/// True when the declaration is one a signal rule should ignore.
fn skip_decl(m: &ModuleFacts<'_>, id: DeclId) -> bool {
    let d = m.decl(id);
    !d.is_signal()
        || d.local
        || d.name.starts_with('_')
        || KEEP_ATTRS.iter().any(|a| d.has_attr(a))
        || matches!(d.kind, DeclKind::Port(p) if p.interface)
}

/// `unused-signal`: a net or variable that nothing ever reads.
pub(super) struct UnusedSignal;

impl Lint for UnusedSignal {
    fn id(&self) -> &'static str {
        "L0001"
    }

    fn name(&self) -> &'static str {
        "unused-signal"
    }

    fn default_level(&self) -> Level {
        Level::WARN
    }

    fn check(&self, ctx: &LintContext<'_>, _file: &SourceFile, diags: &mut Diagnostics) {
        for m in &ctx.facts.modules {
            if opaque(m) {
                continue;
            }
            for id in 0..m.decls.len() {
                let d = m.decl(id);
                if skip_decl(m, id) || d.direction().is_some() || !d.reads.is_empty() {
                    continue;
                }
                ctx.report(d.span, format!("`{}` is never read", d.name))
                    .note(format!(
                        "declared in `{}`; remove it, or name it with a leading `_`",
                        m.name()
                    ))
                    .emit(diags);
            }
        }
    }
}

/// `unused-input`: an input port that the module never reads.
pub(super) struct UnusedInput;

impl Lint for UnusedInput {
    fn id(&self) -> &'static str {
        "L0002"
    }

    fn name(&self) -> &'static str {
        "unused-input"
    }

    fn default_level(&self) -> Level {
        Level::WARN
    }

    fn check(&self, ctx: &LintContext<'_>, _file: &SourceFile, diags: &mut Diagnostics) {
        for m in &ctx.facts.modules {
            if opaque(m) {
                continue;
            }
            for &id in &m.ports {
                let d = m.decl(id);
                if skip_decl(m, id) || d.direction() != Some(Direction::Input) {
                    continue;
                }
                if d.reads.is_empty() {
                    ctx.report(
                        d.span,
                        format!("input `{}` of `{}` is never read", d.name, m.name()),
                    )
                    .emit(diags);
                }
            }
        }
    }
}

/// True when the net type drives itself (`supply0`, `tri1`, ...).
fn self_driving(t: NetType) -> bool {
    matches!(
        t,
        NetType::Supply0 | NetType::Supply1 | NetType::Tri0 | NetType::Tri1 | NetType::Trireg
    )
}

/// True when the net type resolves several drivers by design.
fn resolved_net(t: NetType) -> bool {
    matches!(
        t,
        NetType::Tri
            | NetType::Tri0
            | NetType::Tri1
            | NetType::Triand
            | NetType::Trior
            | NetType::Trireg
            | NetType::Wand
            | NetType::Wor
    )
}

/// True when the declaration needs a driver inside the module.
fn needs_driver(m: &ModuleFacts<'_>, id: DeclId) -> bool {
    let d = m.decl(id);
    if skip_decl(m, id) || !d.writes.is_empty() {
        return false;
    }
    if d.net_type().is_some_and(self_driving) {
        return false;
    }
    match d.direction() {
        Some(Direction::Input | Direction::Inout | Direction::Ref | Direction::ConstRef) => false,
        Some(Direction::Output) | None => true,
    }
}

/// `undriven-signal`: a net or variable that nothing ever assigns.
pub(super) struct UndrivenSignal;

impl Lint for UndrivenSignal {
    fn id(&self) -> &'static str {
        "L0003"
    }

    fn name(&self) -> &'static str {
        "undriven-signal"
    }

    fn default_level(&self) -> Level {
        Level::WARN
    }

    fn check(&self, ctx: &LintContext<'_>, _file: &SourceFile, diags: &mut Diagnostics) {
        for m in &ctx.facts.modules {
            if opaque(m) {
                continue;
            }
            for id in 0..m.decls.len() {
                let d = m.decl(id);
                if d.direction().is_some() || !needs_driver(m, id) {
                    continue;
                }
                ctx.report(d.span, format!("`{}` is never assigned", d.name))
                    .note("it will read as x (or z for a net) forever")
                    .emit(diags);
            }
        }
    }
}

/// `undriven-output`: an output port nothing in the module drives.
pub(super) struct UndrivenOutput;

impl Lint for UndrivenOutput {
    fn id(&self) -> &'static str {
        "L0004"
    }

    fn name(&self) -> &'static str {
        "undriven-output"
    }

    fn default_level(&self) -> Level {
        Level::WARN
    }

    fn check(&self, ctx: &LintContext<'_>, _file: &SourceFile, diags: &mut Diagnostics) {
        for m in &ctx.facts.modules {
            if opaque(m) {
                continue;
            }
            for &id in &m.ports {
                let d = m.decl(id);
                if d.direction() != Some(Direction::Output) || !needs_driver(m, id) {
                    continue;
                }
                ctx.report(
                    d.span,
                    format!("output `{}` of `{}` is never driven", d.name, m.name()),
                )
                .emit(diags);
            }
        }
    }
}

/// The bits one write covers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Slice {
    /// The whole declaration, or a selection that is not constant.
    Whole,
    /// A constant bit range, low to high.
    Bits(i128, i128),
}

impl Slice {
    fn overlaps(self, other: Slice) -> bool {
        match (self, other) {
            (Slice::Whole, _) | (_, Slice::Whole) => true,
            (Slice::Bits(al, ah), Slice::Bits(bl, bh)) => al <= bh && bl <= ah,
        }
    }
}

/// The bits a write covers, when they are constant.
fn write_slice(m: &ModuleFacts<'_>, w: &Write<'_>) -> Slice {
    if w.whole {
        return Slice::Whole;
    }
    let Some(lhs) = w.lhs else {
        return Slice::Whole;
    };
    match &lhs.kind {
        ExprKind::Index { base, index } if matches!(base.kind, ExprKind::Ident(_)) => {
            match const_eval(m, index) {
                Some(i) => Slice::Bits(i, i),
                None => Slice::Whole,
            }
        }
        ExprKind::Range {
            base,
            kind,
            left,
            right,
        } if matches!(base.kind, ExprKind::Ident(_)) => {
            let (l, r) = (const_eval(m, left), const_eval(m, right));
            match (kind, l, r) {
                (RangeKind::Fixed, Some(a), Some(b)) => Slice::Bits(a.min(b), a.max(b)),
                (RangeKind::IndexedUp, Some(a), Some(w)) => Slice::Bits(a, a + w - 1),
                (RangeKind::IndexedDown, Some(a), Some(w)) => Slice::Bits(a - w + 1, a),
                _ => Slice::Whole,
            }
        }
        _ => Slice::Whole,
    }
}

/// How one process drives one signal.
struct Driver {
    proc: ProcId,
    span: Span,
    slices: Vec<Slice>,
}

impl Driver {
    fn overlaps(&self, other: &Driver) -> bool {
        self.slices
            .iter()
            .any(|a| other.slices.iter().any(|b| a.overlaps(*b)))
    }
}

/// The English name of a driving process, for the diagnostic.
fn driver_kind(m: &ModuleFacts<'_>, proc: ProcId) -> &'static str {
    use super::facts::ProcKind as P;
    match m.procs[proc].kind {
        P::Always(_) => "an always block",
        P::ContAssign => "a continuous assignment",
        P::NetInit => "a net initialiser",
        P::Instance => "an instance output",
        P::Gate => "a gate output",
        _ => "a process",
    }
}

/// `multiple-drivers`: two or more processes drive the same bits.
pub(super) struct MultipleDrivers;

impl Lint for MultipleDrivers {
    fn id(&self) -> &'static str {
        "L0005"
    }

    fn name(&self) -> &'static str {
        "multiple-drivers"
    }

    fn default_level(&self) -> Level {
        Level::WARN
    }

    fn check(&self, ctx: &LintContext<'_>, _file: &SourceFile, diags: &mut Diagnostics) {
        for m in &ctx.facts.modules {
            if m.module.kind == ModuleKind::Primitive {
                continue;
            }
            for id in 0..m.decls.len() {
                let d = m.decl(id);
                if !d.is_signal() || d.local || d.net_type().is_some_and(resolved_net) {
                    continue;
                }
                // Group the driving writes by process.
                let mut drivers: Vec<Driver> = Vec::new();
                for &wi in &d.writes {
                    let w = &m.writes[wi];
                    if !w.kind.is_driver() {
                        continue;
                    }
                    let slice = write_slice(m, w);
                    match drivers.iter_mut().find(|dr| dr.proc == w.proc) {
                        Some(dr) => dr.slices.push(slice),
                        None => drivers.push(Driver {
                            proc: w.proc,
                            span: w.span,
                            slices: vec![slice],
                        }),
                    }
                }
                if drivers.len() < 2 {
                    continue;
                }
                // The first driver that conflicts with an earlier one.
                let mut conflicts: Vec<(usize, usize)> = Vec::new();
                for i in 0..drivers.len() {
                    for j in 0..i {
                        if m.procs[drivers[i].proc].exclusive_with(&m.procs[drivers[j].proc]) {
                            continue;
                        }
                        if drivers[i].overlaps(&drivers[j]) {
                            conflicts.push((j, i));
                        }
                    }
                }
                let Some(&(first, second)) = conflicts.first() else {
                    continue;
                };
                let mut report = ctx
                    .report(
                        drivers[second].span,
                        format!("`{}` has more than one driver", d.name),
                    )
                    .secondary(
                        drivers[first].span,
                        format!("also driven by {}", driver_kind(m, drivers[first].proc)),
                    );
                let extra = conflicts.len() - 1;
                if extra > 0 {
                    report = report.note(format!("{extra} further conflicting driver(s) omitted"));
                }
                report
                    .note("a net or variable driven from two processes resolves to x")
                    .emit(diags);
            }
        }
    }
}

/// `implicit-net`: an undeclared name used where Verilog invents a net.
pub(super) struct ImplicitNet;

impl Lint for ImplicitNet {
    fn id(&self) -> &'static str {
        "L0006"
    }

    fn name(&self) -> &'static str {
        "implicit-net"
    }

    fn default_level(&self) -> Level {
        Level::WARN
    }

    fn check(&self, ctx: &LintContext<'_>, _file: &SourceFile, diags: &mut Diagnostics) {
        for m in &ctx.facts.modules {
            if m.wildcard_import {
                continue;
            }
            let mut seen: BTreeSet<&str> = BTreeSet::new();
            for u in &m.undeclared {
                if !seen.insert(&u.ident.name) {
                    continue;
                }
                let what = match u.context {
                    UseContext::PortConn => "a port connection",
                    UseContext::AssignLhs => "a continuous assignment",
                };
                let severity = if m.default_nettype_none {
                    Severity::Error
                } else {
                    Severity::Warning
                };
                let note = if m.default_nettype_none {
                    "`default_nettype none` is in effect, so no implicit net is created"
                } else {
                    "Verilog creates an implicit 1-bit wire; declare it, or use `default_nettype none`"
                };
                ctx.report_as(
                    severity,
                    u.ident.span,
                    format!(
                        "`{}` is used in {what} without being declared",
                        u.ident.name
                    ),
                )
                .note(note)
                .emit(diags);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::LintConfig;
    use super::super::tests::lint;

    #[test]
    fn reports_unused_and_undriven() {
        let out = lint(
            "module m(input a, input b, output y, output z);\n\
             wire t;\n\
             wire u = a;\n\
             assign y = a;\n\
             endmodule\n",
            &LintConfig::new(),
        );
        assert!(out.contains("input `b` of `m` is never read"), "{out}");
        assert!(out.contains("`t` is never read"), "{out}");
        assert!(out.contains("`t` is never assigned"), "{out}");
        assert!(out.contains("`u` is never read"), "{out}");
        assert!(out.contains("output `z` of `m` is never driven"), "{out}");
        assert!(!out.contains("`y`"), "{out}");
        assert!(!out.contains("`a`"), "{out}");
    }

    #[test]
    fn keep_attributes_and_underscore_names_are_exempt() {
        let out = lint(
            "module m(input a);\n(* keep *) wire t = a;\nwire _spare;\nendmodule\n",
            &LintConfig::new(),
        );
        assert_eq!(out, "");
    }

    #[test]
    fn multiple_drivers_respects_slices_and_generate_arms() {
        let out = lint(
            "module m #(parameter P = 1) (input a, output [3:0] y, output [1:0] z);\n\
             assign y[1:0] = {2{a}};\n\
             assign y[3:2] = {2{~a}};\n\
             generate if (P) begin assign z = 2'b01; end else begin assign z = 2'b10; end endgenerate\n\
             endmodule\n",
            &LintConfig::new(),
        );
        assert_eq!(out, "", "{out}");

        let out = lint(
            "module m(input a, output y, output w);\n\
             assign y = a;\nassign y = ~a;\n\
             wor w;\nassign w = a;\nassign w = ~a;\n\
             endmodule\n",
            &LintConfig::new(),
        );
        assert!(out.contains("`y` has more than one driver"), "{out}");
        assert!(!out.contains("`w` has more than one driver"), "{out}");
    }

    #[test]
    fn implicit_nets_are_errors_under_default_nettype_none() {
        let out = lint(
            "module leaf(input i, output o);\nassign o = i;\nendmodule\n\
             module m(input a, output y);\nleaf u0 (.i(a), .o(mid));\nassign y = mid;\nendmodule\n",
            &LintConfig::new(),
        );
        assert!(
            out.contains("warning[L0006]: `mid` is used in a port connection"),
            "{out}"
        );

        let out = lint(
            "`default_nettype none\nmodule leaf(input i, output o);\nassign o = i;\nendmodule\n\
             module m(input a, output y);\nleaf u0 (.i(a), .o(mid));\nassign y = mid;\nendmodule\n",
            &LintConfig::new(),
        );
        assert!(out.contains("error[L0006]: `mid`"), "{out}");
    }
}
