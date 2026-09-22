//! Clock domain crossing analysis.
//!
//! [`analyze_cdc_with`] assigns every sequential cell to a clock domain
//! by tracing its clock pin back to a clock source, then looks for every
//! place where a flip-flop in one domain feeds a flip-flop in another.
//! With the `fpga` feature, [`analyze_cdc`] is the same thing with the
//! domains named by a [`crate::fpga::Constraints`] value's
//! `create_clock` statements.
//!
//! # What can be proved, and what cannot
//!
//! This is the important distinction, and the report carries it on every
//! finding as [`Crossing::proven`].
//!
//! **Provable from the netlist alone**, because they are structural
//! facts:
//!
//! - An **unsynchronised crossing** ([`CrossingKind::Unsynchronised`]):
//!   a flop in domain A feeds, through combinational logic or directly,
//!   a flop in domain B that is not the first stage of a synchroniser.
//!   This is the bug everyone is looking for, and finding it needs no
//!   guesswork.
//! - A **two-flop (or deeper) synchroniser**
//!   ([`CrossingKind::Synchroniser`]): the destination flop takes the
//!   crossing signal with no logic in between, its only load is a second
//!   flop in the same domain, again with no logic in between, and the
//!   intermediate flop drives nothing else. That shape is exactly what a
//!   synchroniser is, so recognising it is a proof.
//! - An **asynchronous FIFO's shape**
//!   ([`CrossingKind::AsyncFifo`]): a memory whose write port is clocked
//!   in one domain and whose read port is clocked in another. The
//!   memory, the two ports and the two domains are all in the IR.
//! - **Reconvergence** ([`Reconvergence`]): two or more separate
//!   synchronisers from the same source domain whose outputs are
//!   recombined by one cone of logic. The recombination is structural;
//!   whether it matters depends on the protocol, which is why it is
//!   reported as a warning rather than an error.
//!
//! **Not provable, and reported as unverified** (`proven: false`):
//!
//! A synchroniser is recognised whether it is written as separately
//! named flip-flops or as one shifting register
//! (`sync <= {sync[N-2:0], d};`), which infers a single N-bit flop.
//!
//! - A **gray-coded bus** ([`CrossingKind::GrayBus`]): several bits
//!   crossing together through their own synchronisers. Whether the
//!   source is gray-coded is a property of the *values* the source
//!   takes, not of its structure, and nothing in a netlist says so. The
//!   analysis looks for the usual generator (`gray = bin ^ (bin >> 1)`,
//!   an XOR of a value with a shifted copy of itself) and says whether
//!   it found one, but the finding stays unverified either way: only a
//!   formal proof that at most one bit changes per source clock would
//!   settle it, and [`crate::formal`] is where that would live.
//! - A **handshake** ([`CrossingKind::Handshake`]): synchronised
//!   crossings in both directions between one pair of domains look like
//!   a request / acknowledge pair, but the analysis cannot tell a
//!   handshake from two unrelated control signals that happen to go
//!   opposite ways.
//! - An async FIFO's **gray pointers**: the shape is proved, the pointer
//!   encoding is not, for the same reason as the gray bus.
//!
//! # Not modelled
//!
//! Generated clocks (a clock divided by a flip-flop is a domain of its
//! own here, named after the net it comes out on), asynchronous resets
//! crossing domains, clock gating, latch-based synchronisers, and
//! multi-cycle protocols where the destination is qualified by an enable
//! that the analysis cannot see is safe.
//!
//! Storage has to be in the IR's own primitives. A netlist already
//! mapped to library or vendor cells keeps its flip-flops in
//! [`crate::ir::CellKind::Blackbox`] cells, whose insides nothing here
//! knows, so run the crossing analysis before technology mapping — which
//! is where it belongs anyway, since a crossing is a property of the
//! design, not of the cells it ends up in. The report says so when it
//! finds a module in that state.

use std::fmt::Write as _;

use crate::diag::Severity;
use crate::ir::{
    Cell, CellId, CellKind, Expr, ExprKind, MemoryId, Module, NetId, expr::operands,
    walk::lvalue_exprs,
};
use crate::source::{SourceMap, Span};

/// One clock domain: a name and the net that carries it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Domain {
    /// The clock's name from the constraints, or the root net's name
    /// when no constraint named it.
    pub name: String,
    /// The net the clock enters the design on.
    pub net: String,
    /// True when no `create_clock` named this clock.
    pub inferred: bool,
    /// How many sequential cells are in the domain.
    pub cells: usize,
}

/// What kind of crossing was found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CrossingKind {
    /// A chain of `depth` flip-flops in the destination domain with no
    /// logic between them. Structural, and therefore proved.
    Synchroniser {
        /// How many flops are in the chain, two or more.
        depth: usize,
    },
    /// Several bits crossing together, each through its own
    /// synchroniser. Whether the source is gray-coded cannot be proved
    /// from the netlist.
    GrayBus {
        /// How many bits cross together.
        bits: usize,
        /// True when the source cone contains the usual gray-code
        /// generator, an XOR of a value with a shifted copy of itself.
        generator_found: bool,
    },
    /// Synchronised crossings in both directions between one pair of
    /// domains, which is what a request / acknowledge pair looks like.
    Handshake,
    /// A memory written in one domain and read in another.
    AsyncFifo {
        /// The memory.
        memory: String,
        /// True when synchronised multi-bit buses cross in both
        /// directions, which is what gray pointers look like.
        pointers_synchronised: bool,
    },
    /// A flip-flop in one domain feeding a flip-flop in another with no
    /// synchroniser. The bug.
    Unsynchronised,
}

impl CrossingKind {
    /// A short word for reports.
    pub fn as_str(&self) -> &'static str {
        match self {
            CrossingKind::Synchroniser { .. } => "synchroniser",
            CrossingKind::GrayBus { .. } => "gray bus",
            CrossingKind::Handshake => "handshake",
            CrossingKind::AsyncFifo { .. } => "async fifo",
            CrossingKind::Unsynchronised => "unsynchronised",
        }
    }
}

/// One clock domain crossing.
#[derive(Clone, Debug, PartialEq)]
pub struct Crossing {
    /// What it is.
    pub kind: CrossingKind,
    /// How seriously to take it.
    pub severity: Severity,
    /// True when the classification follows from the netlist alone;
    /// false when it is a guess the report is honest about.
    pub proven: bool,
    /// The domain the data leaves.
    pub from_domain: String,
    /// The domain it arrives in.
    pub to_domain: String,
    /// The launching cells, by name, sorted.
    pub source_cells: Vec<String>,
    /// The capturing cells, by name, sorted.
    pub dest_cells: Vec<String>,
    /// Spans of the cells involved, source cells first.
    pub spans: Vec<Span>,
    /// One line saying what was found and why.
    pub message: String,
}

/// Several synchronised bits from one domain recombined by one cone of
/// logic in another, which reintroduces the metastability the
/// synchronisers removed.
#[derive(Clone, Debug, PartialEq)]
pub struct Reconvergence {
    /// The domain the bits come from.
    pub from_domain: String,
    /// The domain they are recombined in.
    pub to_domain: String,
    /// The synchroniser output cells that reconverge, sorted.
    pub synchronisers: Vec<String>,
    /// The cell whose input cone they meet in.
    pub at_cell: String,
    /// The pin of that cell.
    pub at_pin: String,
    /// Where that cell came from.
    pub span: Span,
}

/// Everything the crossing analysis found.
#[derive(Clone, Debug, PartialEq)]
pub struct CdcReport {
    /// The module analysed.
    pub module: String,
    /// The clock domains, sorted by name.
    pub domains: Vec<Domain>,
    /// The crossings, worst first then by domain pair and cell name.
    pub crossings: Vec<Crossing>,
    /// Reconvergent synchronisers, sorted by destination cell.
    pub reconvergences: Vec<Reconvergence>,
    /// Anything worth saying, in a stable order.
    pub notes: Vec<String>,
}

impl CdcReport {
    /// How many crossings have the given severity.
    pub fn count(&self, severity: Severity) -> usize {
        self.crossings
            .iter()
            .filter(|c| c.severity == severity)
            .count()
    }

    /// True when anything was reported as an error.
    pub fn has_errors(&self) -> bool {
        self.count(Severity::Error) > 0
    }

    /// The crossings whose destination is the named cell.
    pub fn crossings_into(&self, cell: &str) -> impl Iterator<Item = &Crossing> {
        self.crossings
            .iter()
            .filter(move |c| c.dest_cells.iter().any(|d| d == cell))
    }

    /// A readable report.
    pub fn render(&self) -> String {
        self.render_inner(None)
    }

    /// A readable report with `file:line:col` locations resolved
    /// through `sources`.
    pub fn render_sources(&self, sources: &SourceMap) -> String {
        self.render_inner(Some(sources))
    }

    fn render_inner(&self, sources: Option<&SourceMap>) -> String {
        let mut out = String::new();
        let _ = writeln!(
            out,
            "clock domain crossings in module `{}`: {} domain(s), {} crossing(s)",
            self.module,
            self.domains.len(),
            self.crossings.len()
        );
        if !self.domains.is_empty() {
            out.push_str("  domain           clock net        cells\n");
            for domain in &self.domains {
                let _ = writeln!(
                    out,
                    "  {:<16} {:<16} {:>5}{}",
                    domain.name,
                    domain.net,
                    domain.cells,
                    if domain.inferred { "  (inferred)" } else { "" }
                );
            }
        }
        for crossing in &self.crossings {
            let _ = writeln!(out);
            let _ = writeln!(
                out,
                "{}: {} {} -> {}{}",
                crossing.severity,
                crossing.kind.as_str(),
                crossing.from_domain,
                crossing.to_domain,
                if crossing.proven { "" } else { " (unverified)" }
            );
            let _ = writeln!(out, "  {}", crossing.message);
            if !crossing.source_cells.is_empty() {
                let _ = writeln!(out, "  source: {}", crossing.source_cells.join(", "));
            }
            if !crossing.dest_cells.is_empty() {
                let _ = writeln!(out, "  dest:   {}", crossing.dest_cells.join(", "));
            }
            if let Some(map) = sources {
                for span in &crossing.spans {
                    let (file, loc) = map.locate(*span);
                    let _ = writeln!(out, "  at {}:{}:{}", file, loc.line, loc.col);
                }
            }
        }
        for r in &self.reconvergences {
            let _ = writeln!(out);
            let _ = writeln!(
                out,
                "{}: reconvergence {} -> {}",
                Severity::Warning,
                r.from_domain,
                r.to_domain
            );
            let _ = writeln!(
                out,
                "  {} synchronised bits ({}) are recombined at `{}`",
                r.synchronisers.len(),
                r.synchronisers.join(", "),
                r.at_pin
            );
            if let Some(map) = sources {
                let (file, loc) = map.locate(r.span);
                let _ = writeln!(out, "  at {}:{}:{}", file, loc.line, loc.col);
            }
        }
        if !self.notes.is_empty() {
            let _ = writeln!(out);
            for note in &self.notes {
                let _ = writeln!(out, "note: {note}");
            }
        }
        out
    }
}

/// Runs the crossing analysis with the clocks named by a constraints
/// value.
#[cfg(feature = "fpga")]
pub fn analyze_cdc(module: &Module, constraints: &crate::fpga::Constraints) -> CdcReport {
    let spec = super::sta::TimingSpec::from_constraints(constraints);
    analyze_cdc_with(module, &spec)
}

/// Runs the crossing analysis, naming domains after `spec`'s clocks
/// where one matches and after the clock net otherwise.
pub fn analyze_cdc_with(module: &Module, spec: &super::sta::TimingSpec) -> CdcReport {
    Cdc::new(module, spec).run()
}

// ---------------------------------------------------------------------------
// Implementation

/// A sequential cell as `collect` first sees it: its id, span, clock
/// expression, output nets and name.
type RawFlop = (CellId, Span, Option<crate::ir::ExprId>, Vec<NetId>, String);

/// A memory port: the domain it is clocked in, the cell's name and its
/// span.
type MemPort = (usize, String, Span);

/// One sequential cell and what is known about it.
struct Flop {
    id: CellId,
    name: String,
    /// Index into `domains`.
    domain: usize,
    span: Span,
    /// The nets its outputs drive.
    outputs: Vec<NetId>,
}

/// Where a `d` pin's data comes from.
struct Fanin {
    /// The flops whose outputs reach the pin.
    sources: Vec<usize>,
    /// True when anything other than a buffer or an assignment sits in
    /// between.
    through_logic: bool,
}

struct Cdc<'a> {
    module: &'a Module,
    spec: &'a super::sta::TimingSpec,
    domains: Vec<Domain>,
    flops: Vec<Flop>,
    /// The flop driving each net, by net index.
    driver: Vec<Option<usize>>,
    /// The combinational cell driving each net, by net index.
    comb_driver: Vec<Option<CellId>>,
    /// The assignment driving each net, by net index.
    assign_driver: Vec<Option<usize>>,
    notes: Vec<String>,
}

impl<'a> Cdc<'a> {
    fn new(module: &'a Module, spec: &'a super::sta::TimingSpec) -> Cdc<'a> {
        Cdc {
            module,
            spec,
            domains: Vec::new(),
            flops: Vec::new(),
            driver: vec![None; module.nets.len()],
            comb_driver: vec![None; module.nets.len()],
            assign_driver: vec![None; module.nets.len()],
            notes: Vec::new(),
        }
    }

    /// The nets an expression reads, without duplicates.
    fn expr_nets(&self, root: crate::ir::ExprId) -> Vec<NetId> {
        let mut out = Vec::new();
        let mut stack = vec![root];
        let mut seen = vec![false; self.module.exprs.len()];
        while let Some(id) = stack.pop() {
            let Some(expr) = self.module.exprs.get(id) else {
                continue;
            };
            if seen[id.index()] {
                continue;
            }
            seen[id.index()] = true;
            if let ExprKind::Net(net) = expr.kind
                && !out.contains(&net)
            {
                out.push(net);
            }
            stack.extend(operands(&expr.kind));
        }
        out
    }

    /// Follows a clock net back through buffers and assignments to the
    /// net it originates on.
    fn clock_root(&self, mut net: NetId) -> NetId {
        for _ in 0..64 {
            if let Some(cell) = self.comb_driver[net.index()]
                && matches!(self.module.cells[cell].kind, CellKind::Buf | CellKind::Not)
                && let Some(expr) = self.module.cells[cell].input("a")
                && let [only] = self.expr_nets(expr).as_slice()
            {
                net = *only;
                continue;
            }
            if let Some(index) = self.assign_driver[net.index()]
                && let [only] = self.expr_nets(self.module.assigns[index].value).as_slice()
            {
                net = *only;
                continue;
            }
            break;
        }
        net
    }

    fn domain_of_net(&mut self, net: NetId) -> usize {
        let root = self.clock_root(net);
        let net_name = self.module.nets[root].name.as_str().to_owned();
        let clock = self.spec.clocks.iter().find(|c| c.net == net_name);
        let name = clock.map_or_else(|| net_name.clone(), |c| c.name.clone());
        if let Some(at) = self.domains.iter().position(|d| d.name == name) {
            self.domains[at].cells += 1;
            return at;
        }
        self.domains.push(Domain {
            name,
            net: net_name,
            inferred: clock.is_none(),
            cells: 1,
        });
        self.domains.len() - 1
    }

    /// Collects drivers, flops and their domains.
    fn collect(&mut self) {
        for (id, cell) in self.module.cells.iter() {
            for (_, net) in &cell.outputs {
                if cell.kind.is_combinational() {
                    self.comb_driver[net.index()] = Some(id);
                }
            }
        }
        for (i, assign) in self.module.assigns.iter().enumerate() {
            for net in assign.target.nets() {
                self.assign_driver[net.index()] = Some(i);
            }
        }
        let sequential: Vec<RawFlop> = self
            .module
            .cells
            .iter()
            .filter(|(_, c)| {
                matches!(
                    c.kind,
                    CellKind::Dff { .. } | CellKind::Dlatch | CellKind::MemRdPort { .. }
                )
            })
            .map(|(id, c)| {
                let clock_port = match c.kind {
                    CellKind::Dlatch => "en",
                    _ => "clk",
                };
                (
                    id,
                    c.span,
                    c.input(clock_port),
                    c.outputs.iter().map(|(_, n)| *n).collect(),
                    c.name.as_str().to_owned(),
                )
            })
            .collect();
        for (id, span, clock, outputs, name) in sequential {
            let Some(clock) = clock else {
                continue;
            };
            let Some(net) = self.expr_nets(clock).first().copied() else {
                continue;
            };
            let domain = self.domain_of_net(net);
            let index = self.flops.len();
            for net in &outputs {
                self.driver[net.index()] = Some(index);
            }
            self.flops.push(Flop {
                id,
                name,
                domain,
                span,
                outputs,
            });
        }
        // A write port has no output, so it is not a flop, but its
        // domain still has to exist for the memory crossings below.
        let write_clocks: Vec<crate::ir::ExprId> = self
            .module
            .cells
            .values()
            .filter(|c| matches!(c.kind, CellKind::MemWrPort { clocked: true, .. }))
            .filter_map(|c| c.input("clk"))
            .collect();
        for clock in write_clocks {
            if let Some(net) = self.expr_nets(clock).first().copied() {
                self.domain_of_net(net);
            }
        }
    }

    /// The flops whose outputs reach `nets` through combinational logic,
    /// and whether any real logic was crossed.
    fn fanin_of(&self, nets: &[NetId]) -> Fanin {
        let mut sources = Vec::new();
        let mut through_logic = false;
        let mut seen = vec![false; self.module.nets.len()];
        let mut stack: Vec<NetId> = nets.to_vec();
        while let Some(net) = stack.pop() {
            if seen[net.index()] {
                continue;
            }
            seen[net.index()] = true;
            if let Some(flop) = self.driver[net.index()] {
                if !sources.contains(&flop) {
                    sources.push(flop);
                }
                continue;
            }
            if let Some(cell) = self.comb_driver[net.index()] {
                let cell = &self.module.cells[cell];
                if !matches!(cell.kind, CellKind::Buf) {
                    through_logic = true;
                }
                for (_, expr) in &cell.inputs {
                    stack.extend(self.expr_nets(*expr));
                }
                continue;
            }
            if let Some(index) = self.assign_driver[net.index()] {
                let assign = &self.module.assigns[index];
                stack.extend(self.expr_nets(assign.value));
                lvalue_exprs(&assign.target, &mut |e| stack.extend(self.expr_nets(e)));
                continue;
            }
        }
        sources.sort_unstable();
        Fanin {
            sources,
            through_logic,
        }
    }

    /// The pins that read `net`, as `(cell index, port)` pairs plus a
    /// count of everything else that reads it (ports, assignments).
    fn loads_of(&self, net: NetId) -> (Vec<(CellId, String)>, usize) {
        let mut cells = Vec::new();
        let mut other = 0;
        for (id, cell) in self.module.cells.iter() {
            for (port, expr) in &cell.inputs {
                if self.expr_nets(*expr).contains(&net) {
                    cells.push((id, port.as_str().to_owned()));
                }
            }
        }
        for assign in &self.module.assigns {
            if self.expr_nets(assign.value).contains(&net) {
                other += 1;
            }
        }
        if self.module.ports.iter().any(|p| p.net == net) {
            other += 1;
        }
        (cells, other)
    }

    /// True when `flop` is the first stage of a synchroniser chain of at
    /// least two flops: its only load is another flop in the same
    /// domain, on that flop's `d` pin, with nothing in between.
    ///
    /// Returns the depth of the chain.
    /// The stage count when `flop` is a synchroniser written as a single
    /// shifting register rather than as separately named flip-flops.
    ///
    /// The idiom `sync <= {sync[N-2:0], d};` infers one N-bit flip-flop
    /// whose `d` is a concatenation of its own output shifted by one and
    /// the incoming bit, so the flop-to-flop walk in
    /// [`Self::synchroniser_depth`] never sees a second stage and the
    /// crossing was reported as unsynchronised. That is the most common
    /// way the thing is written, so the false positive mattered more than
    /// most.
    ///
    /// Only the exact shape counts: two concatenated parts, one of them a
    /// slice of this flop's own output covering every bit but one, the
    /// other a single bit. Anything else falls through to the general
    /// walk, since a partial match is not evidence of anything.
    fn shift_register_depth(&self, flop: usize) -> Option<usize> {
        let cell = &self.module.cells[self.flops[flop].id];
        let q = *self.flops[flop].outputs.first()?;
        let width = self.module.nets[q].ty.width()?;
        if width < 2 {
            return None;
        }
        let d = cell
            .inputs
            .iter()
            .find(|(port, _)| port.as_str() == "d")
            .map(|(_, expr)| *expr)?;
        let ExprKind::Concat(parts) = &self.module.exprs.get(d)?.kind else {
            return None;
        };
        if parts.len() != 2 {
            return None;
        }

        // Either shift direction: new data at the bottom with the old
        // bits above it, or the mirror image.
        let shapes = [(parts[0], parts[1]), (parts[1], parts[0])];
        for (kept, incoming) in shapes {
            let Some(kept_expr) = self.module.exprs.get(kept) else {
                continue;
            };
            let ExprKind::Slice { base, hi, lo } = kept_expr.kind else {
                continue;
            };
            let keeps_all_but_one = (hi == width - 2 && lo == 0) || (hi == width - 1 && lo == 1);
            if !keeps_all_but_one {
                continue;
            }
            if self.module.exprs.get(base).and_then(Expr::as_net) != Some(q) {
                continue;
            }
            if self.module.exprs.get(incoming)?.ty.width() != Some(1) {
                continue;
            }
            return usize::try_from(width).ok();
        }
        None
    }

    fn synchroniser_depth(&self, flop: usize) -> usize {
        let mut depth = 1;
        let mut at = flop;
        for _ in 0..8 {
            let Some(net) = self.flops[at].outputs.first().copied() else {
                break;
            };
            let (loads, other) = self.loads_of(net);
            if other > 0 || loads.len() != 1 {
                break;
            }
            let (cell, port) = &loads[0];
            if port != "d" {
                break;
            }
            let Some(next) = self.flops.iter().position(|f| f.id == *cell) else {
                break;
            };
            if self.flops[next].domain != self.flops[flop].domain {
                break;
            }
            depth += 1;
            at = next;
        }
        depth
    }

    /// The nets reached from `nets` through operations that only move
    /// bits about: continuous assignments, buffers and shifts. This is
    /// what makes `bin` and `bin >> 1` come back as the same source.
    fn transparent_sources(&self, nets: &[NetId]) -> Vec<NetId> {
        let mut out = Vec::new();
        let mut seen = vec![false; self.module.nets.len()];
        let mut stack: Vec<NetId> = nets.to_vec();
        while let Some(net) = stack.pop() {
            if seen[net.index()] {
                continue;
            }
            seen[net.index()] = true;
            if let Some(index) = self.assign_driver[net.index()] {
                stack.extend(self.expr_nets(self.module.assigns[index].value));
                continue;
            }
            if let Some(cell) = self.comb_driver[net.index()]
                && matches!(
                    self.module.cells[cell].kind,
                    CellKind::Buf | CellKind::Shl | CellKind::Shr | CellKind::Sshr
                )
            {
                for (_, expr) in &self.module.cells[cell].inputs {
                    stack.extend(self.expr_nets(*expr));
                }
                continue;
            }
            out.push(net);
        }
        out.sort_unstable();
        out
    }

    /// True when the cone feeding `nets` contains the usual gray-code
    /// generator: an XOR whose two operands come from a common source
    /// once shifts and wiring are seen through, which is what
    /// `bin ^ (bin >> 1)` looks like as cells.
    fn has_gray_generator(&self, nets: &[NetId]) -> bool {
        let mut seen = vec![false; self.module.nets.len()];
        let mut stack: Vec<NetId> = nets.to_vec();
        while let Some(net) = stack.pop() {
            if seen[net.index()] {
                continue;
            }
            seen[net.index()] = true;
            if let Some(cell) = self.comb_driver[net.index()] {
                let cell = &self.module.cells[cell];
                if matches!(cell.kind, CellKind::Xor)
                    && let (Some(a), Some(b)) = (cell.input("a"), cell.input("b"))
                {
                    let an = self.transparent_sources(&self.expr_nets(a));
                    let bn = self.transparent_sources(&self.expr_nets(b));
                    if an.iter().any(|n| bn.contains(n)) {
                        return true;
                    }
                }
                for (_, expr) in &cell.inputs {
                    stack.extend(self.expr_nets(*expr));
                }
            } else if let Some(index) = self.assign_driver[net.index()] {
                stack.extend(self.expr_nets(self.module.assigns[index].value));
            }
        }
        false
    }

    /// The data cone of a flop's `d` pin.
    fn data_nets(&self, cell: &Cell) -> Vec<NetId> {
        let mut nets = Vec::new();
        for (port, expr) in &cell.inputs {
            if port.as_str() == "clk" {
                continue;
            }
            nets.extend(self.expr_nets(*expr));
        }
        nets
    }

    fn run(mut self) -> CdcReport {
        self.collect();

        // --- flop-to-flop crossings ---------------------------------
        let mut crossings: Vec<Crossing> = Vec::new();
        // (source domain, dest domain, dest flop, depth) for every
        // recognised synchroniser, used to find buses and handshakes.
        let mut syncs: Vec<(usize, usize, usize, usize)> = Vec::new();
        for dest in 0..self.flops.len() {
            let cell = &self.module.cells[self.flops[dest].id];
            let nets = self.data_nets(cell);
            let fanin = self.fanin_of(&nets);
            let to_domain = self.flops[dest].domain;
            let foreign: Vec<usize> = fanin
                .sources
                .iter()
                .copied()
                .filter(|s| self.flops[*s].domain != to_domain)
                .collect();
            if foreign.is_empty() {
                continue;
            }
            // A shifting synchroniser's `d` is a concatenation, which the
            // general fan-in walk counts as logic between the domains. The
            // shape check above has already proved there is none, so it
            // stands in for that test rather than being added to it.
            let shifting = self.shift_register_depth(dest);
            let depth = shifting.unwrap_or_else(|| self.synchroniser_depth(dest));
            let synchronised =
                foreign.len() == 1 && depth >= 2 && (shifting.is_some() || !fanin.through_logic);
            // Group the foreign sources by their domain so one crossing
            // is reported per (source domain, destination flop).
            let mut by_domain: Vec<(usize, Vec<usize>)> = Vec::new();
            for source in foreign {
                let d = self.flops[source].domain;
                match by_domain.iter_mut().find(|(x, _)| *x == d) {
                    Some((_, v)) => v.push(source),
                    None => by_domain.push((d, vec![source])),
                }
            }
            by_domain.sort_by_key(|(d, _)| self.domains[*d].name.clone());
            for (from_domain, sources) in by_domain {
                let mut source_cells: Vec<String> = sources
                    .iter()
                    .map(|s| self.flops[*s].name.clone())
                    .collect();
                source_cells.sort();
                let mut spans: Vec<Span> = sources.iter().map(|s| self.flops[*s].span).collect();
                spans.push(self.flops[dest].span);
                if synchronised {
                    syncs.push((from_domain, to_domain, dest, depth));
                    crossings.push(Crossing {
                        kind: CrossingKind::Synchroniser { depth },
                        severity: Severity::Note,
                        proven: true,
                        from_domain: self.domains[from_domain].name.clone(),
                        to_domain: self.domains[to_domain].name.clone(),
                        source_cells,
                        dest_cells: vec![self.flops[dest].name.clone()],
                        spans,
                        message: format!(
                            "`{}` starts a {depth}-flop synchroniser with no logic in the chain",
                            self.flops[dest].name
                        ),
                    });
                } else {
                    let why = if fanin.through_logic {
                        "combinational logic sits between the domains"
                    } else if depth < 2 {
                        "the capturing flip-flop is not followed by a second one in its own domain"
                    } else {
                        "several source flip-flops feed one capturing flip-flop"
                    };
                    crossings.push(Crossing {
                        kind: CrossingKind::Unsynchronised,
                        severity: Severity::Error,
                        proven: true,
                        from_domain: self.domains[from_domain].name.clone(),
                        to_domain: self.domains[to_domain].name.clone(),
                        source_cells,
                        dest_cells: vec![self.flops[dest].name.clone()],
                        spans,
                        message: format!(
                            "`{}` captures data from another clock domain and {why}",
                            self.flops[dest].name
                        ),
                    });
                }
            }
        }

        // --- buses and handshakes -----------------------------------
        let mut pairs: Vec<(usize, usize)> = Vec::new();
        for (from, to, _, _) in &syncs {
            if !pairs.contains(&(*from, *to)) {
                pairs.push((*from, *to));
            }
        }
        pairs.sort_by_key(|(a, b)| (self.domains[*a].name.clone(), self.domains[*b].name.clone()));
        for (from, to) in pairs.clone() {
            let members: Vec<usize> = syncs
                .iter()
                .filter(|(f, t, _, _)| *f == from && *t == to)
                .map(|(_, _, dest, _)| *dest)
                .collect();
            // Count bits, not cells: a netlist may keep a bus in one
            // wide flip-flop or split it into one per bit, and both are
            // the same crossing. A shifting synchroniser is the exception
            // and counts as one bit however deep it is, because its width
            // is stages rather than data.
            let bits: usize = members
                .iter()
                .map(|d| {
                    if self.shift_register_depth(*d).is_some() {
                        return 1;
                    }
                    self.flops[*d]
                        .outputs
                        .iter()
                        .map(|n| {
                            usize::try_from(self.module.nets[*n].ty.width().unwrap_or(1))
                                .unwrap_or(1)
                        })
                        .sum::<usize>()
                })
                .sum();
            if bits < 2 {
                continue;
            }
            let mut dest_cells: Vec<String> = members
                .iter()
                .map(|d| self.flops[*d].name.clone())
                .collect();
            dest_cells.sort();
            let mut source_cells = Vec::new();
            let mut cone = Vec::new();
            for dest in &members {
                let cell = &self.module.cells[self.flops[*dest].id];
                let nets = self.data_nets(cell);
                for source in self.fanin_of(&nets).sources {
                    if self.flops[source].domain == from {
                        source_cells.push(self.flops[source].name.clone());
                        let cell = &self.module.cells[self.flops[source].id];
                        cone.extend(self.data_nets(cell));
                    }
                }
            }
            source_cells.sort();
            source_cells.dedup();
            let generator_found = self.has_gray_generator(&cone);
            let mut spans: Vec<Span> = members.iter().map(|d| self.flops[*d].span).collect();
            spans.sort_by_key(|s| (s.start, s.end));
            crossings.push(Crossing {
                kind: CrossingKind::GrayBus {
                    bits,
                    generator_found,
                },
                severity: Severity::Warning,
                proven: false,
                from_domain: self.domains[from].name.clone(),
                to_domain: self.domains[to].name.clone(),
                source_cells,
                dest_cells,
                spans,
                message: format!(
                    "{bits} bits cross together through separate synchronisers; {}. \
                     Synchronising a multi-bit bus is only safe if at most one bit changes per \
                     source clock, which this analysis cannot prove",
                    if generator_found {
                        "the source cone does contain an XOR of a value with a shifted copy of \
                         itself, the usual gray-code generator"
                    } else {
                        "no gray-code generator was found in the source cone"
                    }
                ),
            });
            // Crossings in both directions between one pair of domains
            // look like a request / acknowledge handshake.
            if pairs.contains(&(to, from)) && from < to {
                crossings.push(Crossing {
                    kind: CrossingKind::Handshake,
                    severity: Severity::Note,
                    proven: false,
                    from_domain: self.domains[from].name.clone(),
                    to_domain: self.domains[to].name.clone(),
                    source_cells: Vec::new(),
                    dest_cells: Vec::new(),
                    spans: Vec::new(),
                    message: "synchronised signals cross in both directions, which is the shape \
                              of a request / acknowledge handshake; whether the two are actually \
                              a protocol cannot be told from the netlist"
                        .to_owned(),
                });
            }
        }

        // --- asynchronous FIFOs -------------------------------------
        crossings.extend(self.memory_crossings(&pairs));

        // --- reconvergence ------------------------------------------
        let reconvergences = self.reconvergences(&syncs);

        crossings.sort_by(|a, b| {
            b.severity
                .cmp(&a.severity)
                .then_with(|| a.from_domain.cmp(&b.from_domain))
                .then_with(|| a.to_domain.cmp(&b.to_domain))
                .then_with(|| a.kind.as_str().cmp(b.kind.as_str()))
                .then_with(|| a.dest_cells.cmp(&b.dest_cells))
                .then_with(|| a.source_cells.cmp(&b.source_cells))
        });
        let mut domains = self.domains;
        domains.sort_by(|a, b| a.name.cmp(&b.name));
        if self.flops.is_empty() {
            let boxes = self
                .module
                .cells
                .values()
                .filter(|c| matches!(c.kind, CellKind::Blackbox(_)))
                .count();
            if boxes > 0 {
                self.notes.push(format!(
                    "the module has no sequential primitives and {boxes} black-box cell(s); \
                     storage inside a black box is not recognised, so run the crossing analysis \
                     before technology mapping"
                ));
            }
        }
        if domains.len() < 2 && crossings.is_empty() && self.notes.is_empty() {
            self.notes
                .push("the design has one clock domain; nothing crosses".to_owned());
        }
        let mut notes = self.notes;
        notes.dedup();
        CdcReport {
            module: self.module.name.as_str().to_owned(),
            domains,
            crossings,
            reconvergences,
            notes,
        }
    }

    /// A memory written in one domain and read in another.
    fn memory_crossings(&self, pairs: &[(usize, usize)]) -> Vec<Crossing> {
        let mut out = Vec::new();
        let mut memories: Vec<(MemoryId, Vec<MemPort>, Vec<MemPort>)> = Vec::new();
        for (_, cell) in self.module.cells.iter() {
            let (mem, writing) = match cell.kind {
                CellKind::MemRdPort { mem, clocked: true } => (mem, false),
                CellKind::MemWrPort { mem, clocked: true } => (mem, true),
                _ => continue,
            };
            let Some(clock) = cell.input("clk") else {
                continue;
            };
            let Some(net) = self.expr_nets(clock).first().copied() else {
                continue;
            };
            let root = self.clock_root(net);
            let name = self.module.nets[root].name.as_str().to_owned();
            let domain_name = self
                .spec
                .clocks
                .iter()
                .find(|c| c.net == name)
                .map_or(name, |c| c.name.clone());
            let Some(domain) = self.domains.iter().position(|d| d.name == domain_name) else {
                continue;
            };
            let entry = match memories.iter_mut().find(|(m, _, _)| *m == mem) {
                Some(e) => e,
                None => {
                    memories.push((mem, Vec::new(), Vec::new()));
                    memories.last_mut().expect("just pushed")
                }
            };
            let port = (domain, cell.name.as_str().to_owned(), cell.span);
            if writing {
                entry.1.push(port);
            } else {
                entry.2.push(port);
            }
        }
        memories.sort_by_key(|(m, _, _)| m.index());
        for (mem, writes, reads) in memories {
            let name = self.module.memories[mem].name.as_str().to_owned();
            for (wd, wname, wspan) in &writes {
                for (rd, rname, rspan) in &reads {
                    if wd == rd {
                        continue;
                    }
                    let pointers_synchronised =
                        pairs.contains(&(*wd, *rd)) && pairs.contains(&(*rd, *wd));
                    out.push(Crossing {
                        kind: CrossingKind::AsyncFifo {
                            memory: name.clone(),
                            pointers_synchronised,
                        },
                        severity: if pointers_synchronised {
                            Severity::Warning
                        } else {
                            Severity::Error
                        },
                        proven: false,
                        from_domain: self.domains[*wd].name.clone(),
                        to_domain: self.domains[*rd].name.clone(),
                        source_cells: vec![wname.clone()],
                        dest_cells: vec![rname.clone()],
                        spans: vec![*wspan, *rspan],
                        message: if pointers_synchronised {
                            format!(
                                "memory `{name}` is written in one domain and read in another, \
                                 and synchronised buses cross in both directions, which is what \
                                 an asynchronous FIFO with gray pointers looks like; that the \
                                 pointers are really gray-coded cannot be proved here"
                            )
                        } else {
                            format!(
                                "memory `{name}` is written in one domain and read in another \
                                 with no synchronised pointer buses crossing between them"
                            )
                        },
                    });
                }
            }
        }
        out
    }

    /// Two or more synchroniser outputs from one source domain meeting
    /// in one cone of logic.
    ///
    /// The report names the cell where they *first* meet: a cell counts
    /// only if no single one of its input pins already carries two of
    /// them, since in that case the merge happened further back and the
    /// cell that did it is the one worth naming.
    fn reconvergences(&self, syncs: &[(usize, usize, usize, usize)]) -> Vec<Reconvergence> {
        // The last flop of each recognised synchroniser chain is what
        // the rest of the destination domain sees.
        // (source domain, destination domain, chain tail)
        let mut tails: Vec<(usize, usize, usize)> = Vec::new();
        for (from, to, dest, _) in syncs {
            let mut at = *dest;
            for _ in 0..8 {
                let Some(net) = self.flops[at].outputs.first().copied() else {
                    break;
                };
                let (loads, other) = self.loads_of(net);
                if other > 0 || loads.len() != 1 {
                    break;
                }
                let (cell, port) = &loads[0];
                if port != "d" {
                    break;
                }
                match self.flops.iter().position(|f| f.id == *cell) {
                    Some(next) if self.flops[next].domain == self.flops[at].domain => at = next,
                    _ => break,
                }
            }
            if !tails.contains(&(*from, *to, at)) {
                tails.push((*from, *to, at));
            }
        }
        let mut out = Vec::new();
        for (_, cell) in self.module.cells.iter() {
            // Which chains each input pin brings in, by source and
            // destination domain.
            let mut per_pin: Vec<Vec<((usize, usize), String)>> = Vec::new();
            for (port, expr) in &cell.inputs {
                if port.as_str() == "clk" {
                    per_pin.push(Vec::new());
                    continue;
                }
                let reached = self.fanin_of(&self.expr_nets(*expr)).sources;
                per_pin.push(
                    tails
                        .iter()
                        .filter(|(_, _, tail)| reached.contains(tail))
                        .map(|(from, to, tail)| ((*from, *to), self.flops[*tail].name.clone()))
                        .collect(),
                );
            }
            let mut merged: Vec<((usize, usize), Vec<String>)> = Vec::new();
            for pin in &per_pin {
                let mut here: Vec<((usize, usize), usize)> = Vec::new();
                for (key, name) in pin {
                    match merged.iter_mut().find(|(k, _)| k == key) {
                        Some((_, v)) => {
                            if !v.contains(name) {
                                v.push(name.clone());
                            }
                        }
                        None => merged.push((*key, vec![name.clone()])),
                    }
                    match here.iter_mut().find(|(k, _)| k == key) {
                        Some((_, n)) => *n += 1,
                        None => here.push((*key, 1)),
                    }
                }
                // This pin already carries the merge, so an earlier cell
                // is the one to name.
                if here.iter().any(|(_, n)| *n > 1) {
                    merged.clear();
                    break;
                }
            }
            let output = cell
                .outputs
                .first()
                .map_or_else(|| "?".to_owned(), |(p, _)| p.as_str().to_owned());
            for ((from, to), mut names) in merged {
                if names.len() < 2 {
                    continue;
                }
                names.sort();
                out.push(Reconvergence {
                    from_domain: self.domains[from].name.clone(),
                    to_domain: self.domains[to].name.clone(),
                    synchronisers: names,
                    at_cell: cell.name.as_str().to_owned(),
                    at_pin: format!("{}/{output}", cell.name),
                    span: cell.span,
                });
            }
        }
        out.sort_by(|a, b| {
            a.at_pin
                .cmp(&b.at_pin)
                .then_with(|| a.from_domain.cmp(&b.from_domain))
        });
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::builder::ModuleBuilder;
    use crate::ir::types::Type;
    use crate::ir::{Const, NetId as Net};
    use crate::source::SourceMap;
    use crate::timing::sta::{ClockSpec, TimingSpec};

    fn span() -> Span {
        let mut map = SourceMap::new();
        let id = map.add("cdc-test", "").unwrap();
        Span::new(id, 0, 0)
    }

    fn dff() -> CellKind {
        CellKind::Dff {
            clk_pos: true,
            has_enable: false,
            reset: None,
        }
    }

    /// Two clocks, `a` and `b`, named by constraints.
    fn two_clocks() -> TimingSpec {
        TimingSpec::new()
            .with_clock(ClockSpec::new("aclk", "a", 10.0))
            .with_clock(ClockSpec::new("bclk", "b", 7.0))
    }

    /// Adds a flip-flop `name` on clock net `clk` capturing `data`.
    fn flop(b: &mut ModuleBuilder, name: &str, clk: Net, data: crate::ir::ExprId, q: Net) {
        let ce = b.net(clk);
        b.cell(
            name,
            dff(),
            vec![("clk".into(), ce), ("d".into(), data)],
            vec![("q".into(), q)],
        );
    }

    #[test]
    fn an_unsynchronised_crossing_is_an_error() {
        let mut b = ModuleBuilder::new("bad", span());
        let a = b.input("a", Type::bit());
        let bclk = b.input("b", Type::bit());
        let d = b.input("d", Type::bit());
        let out = b.output("out", Type::bit());
        let mid = b.add_net("mid", Type::bit());
        let de = b.net(d);
        flop(&mut b, "src", a, de, mid);
        let me = b.net(mid);
        flop(&mut b, "dst", bclk, me, out);
        let m = b.finish();
        let report = analyze_cdc_with(&m, &two_clocks());
        assert_eq!(report.domains.len(), 2);
        assert_eq!(report.crossings.len(), 1);
        let crossing = &report.crossings[0];
        assert_eq!(crossing.kind, CrossingKind::Unsynchronised);
        assert_eq!(crossing.severity, Severity::Error);
        assert!(crossing.proven);
        assert_eq!(crossing.from_domain, "aclk");
        assert_eq!(crossing.to_domain, "bclk");
        assert_eq!(crossing.source_cells, ["src"]);
        assert_eq!(crossing.dest_cells, ["dst"]);
        assert!(report.has_errors());
        assert_eq!(report.count(Severity::Error), 1);
        assert_eq!(report.crossings_into("dst").count(), 1);
        assert!(report.render().contains("unsynchronised aclk -> bclk"));
    }

    #[test]
    fn logic_between_the_domains_is_still_unsynchronised() {
        let mut b = ModuleBuilder::new("logic", span());
        let a = b.input("a", Type::bit());
        let bclk = b.input("b", Type::bit());
        let d = b.input("d", Type::bit());
        let en = b.input("en", Type::bit());
        let out = b.output("out", Type::bit());
        let out2 = b.output("out2", Type::bit());
        let mid = b.add_net("mid", Type::bit());
        let gated = b.add_net("gated", Type::bit());
        let s2 = b.add_net("s2", Type::bit());
        let de = b.net(d);
        flop(&mut b, "src", a, de, mid);
        let (me, ee) = (b.net(mid), b.net(en));
        b.cell2("g", CellKind::And, me, ee, gated);
        // A proper two-flop chain, but with logic in front of it.
        let ge = b.net(gated);
        flop(&mut b, "s1", bclk, ge, s2);
        let se = b.net(s2);
        flop(&mut b, "s2", bclk, se, out);
        let oe = b.net(out);
        flop(&mut b, "s3", bclk, oe, out2);
        let m = b.finish();
        let report = analyze_cdc_with(&m, &two_clocks());
        assert_eq!(report.crossings.len(), 1);
        assert_eq!(report.crossings[0].kind, CrossingKind::Unsynchronised);
        assert!(
            report.crossings[0]
                .message
                .contains("combinational logic sits between")
        );
    }

    /// Builds `count` two-flop synchronisers from domain `a` to `b`,
    /// each fed by its own source flop; returns the module.
    fn synchronisers(count: usize, gray: bool) -> crate::ir::Module {
        let mut b = ModuleBuilder::new("sync", span());
        let a = b.input("a", Type::bit());
        let bclk = b.input("b", Type::bit());
        let d = b.input("d", Type::bits(4));
        for i in 0..count {
            let src = b.add_net(format!("src{i}"), Type::bit());
            let s1 = b.add_net(format!("s1_{i}"), Type::bit());
            let out = b.output(format!("out{i}"), Type::bit());
            let de = b.net(d);
            let bit = b.slice(de, u32::try_from(i).unwrap(), u32::try_from(i).unwrap());
            let value = if gray {
                // `gray = bin ^ (bin >> 1)`: an XOR of a value with a
                // shifted copy of itself.
                let shifted = b.add_net(format!("sh{i}"), Type::bits(4));
                let one = b.constant(Const::from_u64(4, 1));
                let shr = b.shr(de, one);
                b.assign(shifted, shr);
                let se = b.net(shifted);
                let sbit = b.slice(se, u32::try_from(i).unwrap(), u32::try_from(i).unwrap());
                let x = b.add_net(format!("x{i}"), Type::bit());
                b.cell2(format!("xor{i}"), CellKind::Xor, bit, sbit, x);
                b.net(x)
            } else {
                bit
            };
            flop(&mut b, &format!("srcff{i}"), a, value, src);
            let se = b.net(src);
            flop(&mut b, &format!("sync{i}a"), bclk, se, s1);
            let s1e = b.net(s1);
            flop(&mut b, &format!("sync{i}b"), bclk, s1e, out);
        }
        b.finish()
    }

    #[test]
    fn a_two_flop_synchroniser_is_recognised_and_proved() {
        let m = synchronisers(1, false);
        let report = analyze_cdc_with(&m, &two_clocks());
        assert_eq!(report.crossings.len(), 1);
        let crossing = &report.crossings[0];
        assert_eq!(crossing.kind, CrossingKind::Synchroniser { depth: 2 });
        assert_eq!(crossing.severity, Severity::Note);
        assert!(crossing.proven);
        assert_eq!(crossing.dest_cells, ["sync0a"]);
        assert!(!report.has_errors());
        assert!(report.reconvergences.is_empty());
    }

    #[test]
    fn a_bus_of_synchronisers_is_reported_as_unverified() {
        let m = synchronisers(3, false);
        let report = analyze_cdc_with(&m, &two_clocks());
        let bus = report
            .crossings
            .iter()
            .find(|c| matches!(c.kind, CrossingKind::GrayBus { .. }))
            .expect("a bus is reported");
        assert_eq!(
            bus.kind,
            CrossingKind::GrayBus {
                bits: 3,
                generator_found: false,
            }
        );
        // The shape is recognised; the encoding is not proved.
        assert!(!bus.proven);
        assert_eq!(bus.severity, Severity::Warning);
        assert!(bus.message.contains("cannot prove"));
        assert!(report.render().contains("(unverified)"));
        // The generator is found when the source really is gray coded.
        let m = synchronisers(3, true);
        let report = analyze_cdc_with(&m, &two_clocks());
        let bus = report
            .crossings
            .iter()
            .find(|c| matches!(c.kind, CrossingKind::GrayBus { .. }))
            .expect("a bus is reported");
        assert_eq!(
            bus.kind,
            CrossingKind::GrayBus {
                bits: 3,
                generator_found: true,
            }
        );
        // Even then it stays unverified: the structure is suggestive,
        // not a proof about the values.
        assert!(!bus.proven);
    }

    #[test]
    fn reconvergent_synchronisers_are_reported() {
        // Two synchronised bits recombined by one AND in the
        // destination domain: the classic reconvergence bug.
        let mut b = ModuleBuilder::new("recon", span());
        let a = b.input("a", Type::bit());
        let bclk = b.input("b", Type::bit());
        let d0 = b.input("d0", Type::bit());
        let d1 = b.input("d1", Type::bit());
        let out = b.output("out", Type::bit());
        let mut tails = Vec::new();
        for (i, d) in [d0, d1].into_iter().enumerate() {
            let src = b.add_net(format!("src{i}"), Type::bit());
            let s1 = b.add_net(format!("s1_{i}"), Type::bit());
            let s2 = b.add_net(format!("s2_{i}"), Type::bit());
            let de = b.net(d);
            flop(&mut b, &format!("srcff{i}"), a, de, src);
            let se = b.net(src);
            flop(&mut b, &format!("sync{i}a"), bclk, se, s1);
            let s1e = b.net(s1);
            flop(&mut b, &format!("sync{i}b"), bclk, s1e, s2);
            tails.push(s2);
        }
        let merged = b.add_net("merged", Type::bit());
        let (t0, t1) = (b.net(tails[0]), b.net(tails[1]));
        b.cell2("recombine", CellKind::And, t0, t1, merged);
        let me = b.net(merged);
        flop(&mut b, "cap", bclk, me, out);
        let m = b.finish();
        let report = analyze_cdc_with(&m, &two_clocks());
        assert_eq!(report.reconvergences.len(), 1);
        let r = &report.reconvergences[0];
        assert_eq!(r.from_domain, "aclk");
        assert_eq!(r.to_domain, "bclk");
        assert_eq!(r.synchronisers, ["sync0b", "sync1b"]);
        assert_eq!(r.at_cell, "recombine");
        assert_eq!(r.at_pin, "recombine/y");
        assert!(report.render().contains("reconvergence aclk -> bclk"));
        // The capturing flop is not reported as well: the merge already
        // happened at the AND.
        assert!(!report.reconvergences.iter().any(|r| r.at_cell == "cap"));
    }

    #[test]
    fn an_async_fifo_is_recognised_by_its_ports() {
        let mut b = ModuleBuilder::new("fifo", span());
        let a = b.input("a", Type::bit());
        let bclk = b.input("b", Type::bit());
        let waddr = b.input("waddr", Type::bits(4));
        let wdata = b.input("wdata", Type::bits(8));
        let wen = b.input("wen", Type::bit());
        let raddr = b.input("raddr", Type::bits(4));
        let ren = b.input("ren", Type::bit());
        let rdata = b.output("rdata", Type::bits(8));
        let mem = b.memory("ram", Type::bits(8), 16);
        let (ae, be) = (b.net(a), b.net(bclk));
        let (wa, wd, we) = (b.net(waddr), b.net(wdata), b.net(wen));
        b.cell(
            "wr",
            CellKind::MemWrPort { mem, clocked: true },
            vec![
                ("addr".into(), wa),
                ("data".into(), wd),
                ("en".into(), we),
                ("clk".into(), ae),
            ],
            vec![],
        );
        let (ra, re) = (b.net(raddr), b.net(ren));
        b.cell(
            "rd",
            CellKind::MemRdPort { mem, clocked: true },
            vec![("addr".into(), ra), ("clk".into(), be), ("en".into(), re)],
            vec![("data".into(), rdata)],
        );
        let m = b.finish();
        let report = analyze_cdc_with(&m, &two_clocks());
        let fifo = report
            .crossings
            .iter()
            .find(|c| matches!(c.kind, CrossingKind::AsyncFifo { .. }))
            .expect("the memory crossing is found");
        assert_eq!(
            fifo.kind,
            CrossingKind::AsyncFifo {
                memory: "ram".to_owned(),
                pointers_synchronised: false,
            }
        );
        // No synchronised pointer buses: this is the dangerous shape.
        assert_eq!(fifo.severity, Severity::Error);
        assert!(!fifo.proven);
        assert_eq!(fifo.from_domain, "aclk");
        assert_eq!(fifo.to_domain, "bclk");
        assert!(report.render().contains("async fifo"));
    }

    #[test]
    fn a_single_domain_design_has_nothing_to_report() {
        let mut b = ModuleBuilder::new("one", span());
        let clk = b.input("clk", Type::bit());
        let d = b.input("d", Type::bit());
        let q = b.output("q", Type::bit());
        let de = b.net(d);
        flop(&mut b, "ff", clk, de, q);
        let m = b.finish();
        let report = analyze_cdc_with(&m, &TimingSpec::new());
        assert_eq!(report.domains.len(), 1);
        // No `create_clock` named it, so the domain takes the net's name.
        assert_eq!(report.domains[0].name, "clk");
        assert!(report.domains[0].inferred);
        assert_eq!(report.domains[0].cells, 1);
        assert!(report.crossings.is_empty());
        assert!(report.notes.iter().any(|n| n.contains("one clock domain")));
    }

    #[test]
    fn a_clock_is_followed_through_buffers() {
        let mut b = ModuleBuilder::new("buffered", span());
        let a = b.input("a", Type::bit());
        let d = b.input("d", Type::bit());
        let q = b.output("q", Type::bit());
        let buffered = b.add_net("abuf", Type::bit());
        let ae = b.net(a);
        b.cell(
            "cbuf",
            CellKind::Buf,
            vec![("a".into(), ae)],
            vec![("y".into(), buffered)],
        );
        let de = b.net(d);
        flop(&mut b, "ff", buffered, de, q);
        let m = b.finish();
        let report = analyze_cdc_with(&m, &two_clocks());
        // The buffer does not make a domain of its own.
        assert_eq!(report.domains.len(), 1);
        assert_eq!(report.domains[0].name, "aclk");
    }

    #[test]
    fn a_shared_intermediate_flop_is_not_a_synchroniser() {
        // The first stage drives something else as well, so its output
        // is not metastability-free where that other thing reads it.
        let mut b = ModuleBuilder::new("shared", span());
        let a = b.input("a", Type::bit());
        let bclk = b.input("b", Type::bit());
        let d = b.input("d", Type::bit());
        let out = b.output("out", Type::bit());
        let leak = b.output("leak", Type::bit());
        let src = b.add_net("src", Type::bit());
        let s1 = b.add_net("s1", Type::bit());
        let de = b.net(d);
        flop(&mut b, "srcff", a, de, src);
        let se = b.net(src);
        flop(&mut b, "s1", bclk, se, s1);
        let s1e = b.net(s1);
        flop(&mut b, "s2", bclk, s1e, out);
        let s1e2 = b.net(s1);
        b.assign(leak, s1e2);
        let m = b.finish();
        let report = analyze_cdc_with(&m, &two_clocks());
        assert_eq!(report.crossings.len(), 1);
        assert_eq!(report.crossings[0].kind, CrossingKind::Unsynchronised);
        assert!(
            report.crossings[0]
                .message
                .contains("not followed by a second one")
        );
    }

    #[test]
    fn rendering_is_stable_and_carries_spans() {
        let m = synchronisers(2, false);
        let report = analyze_cdc_with(&m, &two_clocks());
        let text = report.render();
        assert_eq!(text, report.render());
        assert!(text.starts_with("clock domain crossings in module `sync`"));
        let mut map = SourceMap::new();
        let _ = map.add("cdc-test", "");
        assert!(report.render_sources(&map).contains("at cdc-test:1:1"));
        assert_eq!(CrossingKind::Handshake.as_str(), "handshake");
    }

    #[test]
    fn crossings_in_both_directions_look_like_a_handshake() {
        // Two bits each way between the same pair of domains.
        let mut b = ModuleBuilder::new("hs", span());
        let a = b.input("a", Type::bit());
        let bclk = b.input("b", Type::bit());
        let req = b.input("req", Type::bits(2));
        let ack = b.input("ack", Type::bits(2));
        for (i, (from, to, source)) in [(a, bclk, req), (bclk, a, ack)].into_iter().enumerate() {
            for bit in 0..2u32 {
                let src = b.add_net(format!("src{i}_{bit}"), Type::bit());
                let s1 = b.add_net(format!("s1_{i}_{bit}"), Type::bit());
                let out = b.output(format!("out{i}_{bit}"), Type::bit());
                let se = b.net(source);
                let value = b.slice(se, bit, bit);
                flop(&mut b, &format!("srcff{i}_{bit}"), from, value, src);
                let sv = b.net(src);
                flop(&mut b, &format!("sync{i}_{bit}a"), to, sv, s1);
                let s1e = b.net(s1);
                flop(&mut b, &format!("sync{i}_{bit}b"), to, s1e, out);
            }
        }
        let m = b.finish();
        let report = analyze_cdc_with(&m, &two_clocks());
        let handshake = report
            .crossings
            .iter()
            .find(|c| c.kind == CrossingKind::Handshake)
            .expect("a handshake is guessed at");
        assert!(!handshake.proven);
        assert_eq!(handshake.severity, Severity::Note);
    }

    #[cfg(feature = "fpga")]
    #[test]
    fn constraints_name_the_domains() {
        use crate::diag::Diagnostics;
        let mut map = SourceMap::new();
        let file = map
            .add(
                "c.rcf",
                "create_clock -name aclk -period 10 a\ncreate_clock -name bclk -period 7 b\n",
            )
            .unwrap();
        let mut diags = Diagnostics::new();
        let text = map.file(file).text().to_owned();
        let constraints = crate::fpga::Constraints::parse(&text, file, &mut diags);
        assert!(!diags.has_errors(), "{}", diags.render(&map));
        let m = synchronisers(1, false);
        let report = analyze_cdc(&m, &constraints);
        assert_eq!(report.domains.len(), 2);
        assert!(report.domains.iter().all(|d| !d.inferred));
        assert_eq!(
            report.render(),
            analyze_cdc_with(&m, &two_clocks()).render()
        );
    }
}
