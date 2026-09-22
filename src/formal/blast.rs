//! Bit-blasting: from the IR's cell form to CNF, one literal per bit.
//!
//! A [`Blaster`] analyses one module once and can then encode any number of
//! *frames* (time steps) of it into a shared [`CnfBuilder`]. Every net bit
//! of a frame becomes a [`Lit`]; combinational logic becomes Tseitin gates;
//! flip-flops and memories become *state* whose next-frame value is a
//! function of the current frame. The result of one frame is a
//! [`BlastedFrame`]: the literals of the inputs, the state, the next state,
//! the outputs and the properties, which the unroller ([`super::unroll`])
//! chains into a transition relation for BMC, k-induction and equivalence
//! checking.
//!
//! # What is accepted
//!
//! The engines work on the *cell form* plus continuous assigns, which is
//! what synthesis produces and what hand-built netlists use:
//!
//! - Every combinational [`CellKind`] (`Not` … `ReduceXor`, `Lut`, `Buf`).
//! - `Dff` with optional enable and reset. An asynchronous reset is treated
//!   like a synchronous one: the transition relation samples `rst` once per
//!   frame, so a reset pulse shorter than a clock period is invisible. A
//!   warning (`F0011`) says so once per module.
//! - `MemRdPort` / `MemWrPort` on memories of at most
//!   [`BlastOptions::max_memory_bits`] bits, by blasting every word as state.
//!   A clocked read port is a register; an unclocked one is combinational.
//!   Several clocked write ports to one memory apply in cell order, so a
//!   later port wins on a same-address collision. An unclocked write port
//!   is a latch and is rejected.
//! - `Blackbox` cells and instances only when they carry the `formal_free`
//!   attribute, in which case their outputs are unconstrained inputs.
//! - Continuous assigns to nets, slices, variable indices and
//!   concatenations of those; transport delays are ignored.
//! - Every [`ExprKind`] over bit vectors, including `Mul`, `Div`, `Mod` and
//!   `Pow` as circuits (see below).
//!
//! Rejected with an error: processes (run synthesis first), `Dlatch`,
//! `Tristate`, unmarked black boxes, `inout` ports, nets and expressions
//! of non-bit-vector type, `Call` nodes, memory writes in assigns and
//! combinational loops. The diagnostic codes are listed in
//! [`super`](crate::formal).
//!
//! # Two-state semantics
//!
//! The engines are 2-state. An `x` bit in a constant, a reset value, a
//! memory initialiser or an out-of-range select is modelled as a *free
//! variable*: a proof then holds for every value the bit could take, and a
//! counter-example may pick whichever value exposes the bug. A `z` bit is
//! an error, since it needs a resolution the netlist does not express. This
//! is sound for `assert` (nothing that is provable under free `x` can fail
//! in a simulator that picks concrete values) and conservative for `cover`.
//!
//! Undriven internal nets are free inputs too (warning `F0015`), as are the
//! outputs of `formal_free` black boxes.
//!
//! Arithmetic follows `IEEE 1364-2005 §5` on fully known operands, so a
//! blasted expression agrees with [`Logic`] evaluation bit for bit (the unit
//! tests cross-check every operator at widths 1, 7, 8, 33, 64 and 65):
//!
//! - `Add`, `Sub`, `Mul` and `Pow` are modulo `2^width`; `Mul` uses the
//!   builder's shift-and-add circuit, `Pow` square-and-multiply with the
//!   special cases of Table 5-6 (`0 ** negative` is `x`, so free).
//! - `Div` and `Mod` use a restoring divider on magnitudes with the signs
//!   applied afterwards (truncating quotient, remainder with the sign of
//!   the dividend). Division by zero is `x` in the standard; the circuit
//!   follows the synthesis convention and yields an all-ones magnitude
//!   quotient and the dividend magnitude as remainder, with the usual sign
//!   fix-up. A proof that depends on the exact value is therefore a proof
//!   about the synthesised netlist, not about simulation.
//! - Shifts are barrel shifters; an amount at or above the width shifts
//!   everything out. `Sshr` fills with the sign bit only when the left
//!   operand is signed, as the standard prescribes.
//! - `Eq`/`Ne` and `CaseEq`/`CaseNe` coincide in 2-state. `WildEq` treats
//!   `x`/`z` bits of a *constant* right operand as don't-care; with a
//!   non-constant right operand it is plain equality.
//! - `Resize` sign-extends only when both the node asks for a signed
//!   result and the operand is signed, as [`ExprKind::Resize`] documents.
//! - A variable `Index` or `IndexedSlice` that reaches outside the operand
//!   reads `x`, so those bits are free.
//!
//! Clocks are not modelled: one frame is one edge of the single clock every
//! flip-flop shares. A module whose flip-flops name more than one clock net
//! gets warning `F0010` and is checked as if they were the same clock.
//! `Dff` polarity (`clk_pos`) is irrelevant for the same reason.
//!
//! # Properties
//!
//! Properties are single-bit nets. They come from two places:
//!
//! 1. Nets carrying the attributes `formal_assert` (must be 1 in every
//!    reachable state), `formal_assume` (constrains the inputs: only traces
//!    where it is 1 in every frame are considered) or `formal_cover` (the
//!    engine looks for a trace where it becomes 1). Synthesis lowers
//!    `assert` statements to such nets; hand-written `.rtl` sets the
//!    attribute directly:
//!
//!    ```text
//!    attr formal_assert = 1
//!    net %q_in_range u1 wire
//!    assign %q_in_range = lt(%q, 8'd200)
//!    ```
//!
//! 2. An explicit list in [`BlastOptions::properties`], for callers that
//!    build properties programmatically (the equivalence checker adds its
//!    miter outputs this way).
//!
//! Both are named after the net.
//!
//! # Probes
//!
//! Along the way the blaster records *probes*: the select of every `Mux`
//! and `Pmux` cell and `?:` expression, the enable of every `Dff`, and the
//! value of every net carrying an `fsm_state` attribute. The reachability
//! lint ([`super::reach`]) asks whether each can take every value.

use std::collections::HashMap;

use crate::diag::{Diagnostic, Diagnostics};
use crate::formal::cnf::CnfBuilder;
use crate::formal::sat::Lit;
use crate::ir::arena::Id;
use crate::ir::validate::validate_module;
use crate::ir::{
    AttrValue, BinaryOp, CellId, CellKind, ExprId, ExprKind, Lvalue, MemoryId, Module, ModuleRef,
    NetId, PortDir, Type, UnaryOp,
};
use crate::logic::{Bit, Logic};
use crate::source::Span;

/// What a property net demands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PropertyKind {
    /// The net must be 1 in every reachable state.
    Assert,
    /// Only traces in which the net is 1 in every frame are considered.
    Assume,
    /// A trace in which the net becomes 1 is sought.
    Cover,
}

impl PropertyKind {
    /// The attribute naming this kind of property on a net.
    pub fn attribute(self) -> &'static str {
        match self {
            PropertyKind::Assert => "formal_assert",
            PropertyKind::Assume => "formal_assume",
            PropertyKind::Cover => "formal_cover",
        }
    }

    /// The keyword used in reports.
    pub fn keyword(self) -> &'static str {
        match self {
            PropertyKind::Assert => "assert",
            PropertyKind::Assume => "assume",
            PropertyKind::Cover => "cover",
        }
    }
}

/// Options of the bit-blaster.
#[derive(Clone, Debug)]
pub struct BlastOptions {
    /// Memories larger than this many bits (words × width) are rejected
    /// with `F0008` instead of being blasted word by word.
    pub max_memory_bits: u64,
    /// Properties in addition to those declared through attributes.
    pub properties: Vec<(NetId, PropertyKind)>,
}

impl Default for BlastOptions {
    fn default() -> Self {
        BlastOptions {
            max_memory_bits: 1 << 16,
            properties: Vec::new(),
        }
    }
}

/// A named bit vector of one frame, LSB first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Signal {
    /// The name shown in traces: a port or net name, or `mem[i]` for a
    /// memory word.
    pub name: String,
    /// Where the object was declared.
    pub span: Span,
    /// True when the value is a signed bit vector.
    pub signed: bool,
    /// One literal per bit.
    pub lits: Vec<Lit>,
}

/// A property of one frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PropertyLit {
    /// The property net's name.
    pub name: String,
    /// Where the property net was declared.
    pub span: Span,
    /// The literal that is true when the property net is 1.
    pub lit: Lit,
}

/// What a [`Probe`] observes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ProbeKind {
    /// The select of a `Mux` cell; 1 picks `b`.
    MuxSelect,
    /// One select bit of a `Pmux` cell.
    PmuxSelect,
    /// The condition of a `?:` expression.
    TernaryCond,
    /// The enable of a `Dff` cell.
    DffEnable,
    /// The value of a net marked `fsm_state`.
    FsmState,
}

/// A value the reachability lint examines.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Probe {
    /// What is observed.
    pub kind: ProbeKind,
    /// The cell or net name; empty for a `?:` expression.
    pub name: String,
    /// Where the observed object is.
    pub span: Span,
    /// The observed bits, LSB first.
    pub lits: Vec<Lit>,
}

/// One time step of a module, encoded in a [`CnfBuilder`].
#[derive(Clone, Debug, Default)]
pub struct BlastedFrame {
    /// Input ports, free black-box outputs and undriven nets, in the order
    /// of [`Blaster::input_slots`].
    pub inputs: Vec<Signal>,
    /// The current state, in the order of [`Blaster::state_slots`].
    pub state: Vec<Signal>,
    /// The next state, parallel to `state`.
    pub next_state: Vec<Vec<Lit>>,
    /// Output ports, in the order of [`Blaster::output_slots`].
    pub outputs: Vec<Signal>,
    /// Every other net, in id order, for waveform dumps.
    pub internal: Vec<Signal>,
    /// The `assert` properties.
    pub asserts: Vec<PropertyLit>,
    /// The `assume` properties.
    pub assumes: Vec<PropertyLit>,
    /// The `cover` properties.
    pub covers: Vec<PropertyLit>,
    /// The values the reachability lint examines.
    pub probes: Vec<Probe>,
}

impl BlastedFrame {
    /// A literal that is true when some `assert` of this frame is violated;
    /// false when there are none.
    pub fn bad(&self, cnf: &mut CnfBuilder) -> Lit {
        let violated: Vec<Lit> = self.asserts.iter().map(|p| !p.lit).collect();
        cnf.or_n(&violated)
    }

    /// The state literals, one vector per slot.
    pub fn state_lits(&self) -> Vec<Vec<Lit>> {
        self.state.iter().map(|s| s.lits.clone()).collect()
    }
}

/// What a state slot stores.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StateKind {
    /// The `q` output of a `Dff` cell.
    Dff(CellId),
    /// One word of a memory.
    MemWord(MemoryId, u64),
    /// The registered `data` output of a clocked `MemRdPort`.
    RdPort(CellId),
}

/// The static description of one state element.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StateSlot {
    /// The name shown in traces.
    pub name: String,
    /// Where the element was declared.
    pub span: Span,
    /// Number of bits.
    pub width: u32,
    /// True for a signed value.
    pub signed: bool,
    /// The value at time zero when the design says (a reset value, an
    /// `init` attribute or a memory initialiser); `x` bits are free.
    pub init: Option<Logic>,
    /// What the slot stores.
    pub kind: StateKind,
}

/// The static description of an input.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputSlot {
    /// The port or net name.
    pub name: String,
    /// The net carrying the value.
    pub net: NetId,
    /// True for an input port, false for a free net (an undriven net or a
    /// black-box output).
    pub is_port: bool,
}

/// The static description of an output port.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutputSlot {
    /// The port name.
    pub name: String,
    /// The net carrying the value.
    pub net: NetId,
}

/// Who drives (part of) a net.
#[derive(Clone, Copy, Debug)]
enum Driver {
    /// `module.assigns[i]`.
    Assign(usize),
    /// Output port `port` of a combinational cell or an unclocked read
    /// port.
    Cell(CellId, usize),
}

/// One module, analysed and ready to be blasted frame by frame.
///
/// Build it with [`Blaster::new`], which validates the module and reports
/// everything the engines cannot handle; then call [`Blaster::blast_frame`]
/// once per time step.
#[derive(Debug)]
pub struct Blaster<'a> {
    module: &'a Module,
    /// Evaluation order of the nets: every net comes after the nets its
    /// drivers read.
    order: Vec<NetId>,
    drivers: Vec<Vec<Driver>>,
    /// Per net: the state slot it is the current value of, if any.
    state_of_net: Vec<Option<usize>>,
    state: Vec<StateSlot>,
    inputs: Vec<InputSlot>,
    outputs: Vec<OutputSlot>,
    properties: Vec<(NetId, PropertyKind)>,
    fsm_nets: Vec<NetId>,
    /// Per memory: the range of state slots holding its words.
    mem_slots: Vec<std::ops::Range<usize>>,
    diags: Diagnostics,
}

impl<'a> Blaster<'a> {
    /// Analyses `module`. Errors (anything the engines cannot encode) are
    /// returned; warnings are kept and available through
    /// [`Blaster::diagnostics`].
    pub fn new(module: &'a Module, options: &BlastOptions) -> Result<Blaster<'a>, Diagnostics> {
        let mut diags = validate_module(module);
        if diags.has_errors() {
            return Err(diags);
        }
        let mut b = Blaster {
            module,
            order: Vec::new(),
            drivers: vec![Vec::new(); module.nets.len()],
            state_of_net: vec![None; module.nets.len()],
            state: Vec::new(),
            inputs: Vec::new(),
            outputs: Vec::new(),
            properties: Vec::new(),
            fsm_nets: Vec::new(),
            mem_slots: Vec::new(),
            diags: Diagnostics::new(),
        };
        b.analyse(options);
        diags.append(&mut b.diags);
        if diags.has_errors() {
            return Err(diags);
        }
        b.diags = diags;
        Ok(b)
    }

    /// The module being blasted.
    pub fn module(&self) -> &'a Module {
        self.module
    }

    /// Warnings recorded during analysis.
    pub fn diagnostics(&self) -> &Diagnostics {
        &self.diags
    }

    /// The state elements, in frame order.
    pub fn state_slots(&self) -> &[StateSlot] {
        &self.state
    }

    /// The inputs, in frame order.
    pub fn input_slots(&self) -> &[InputSlot] {
        &self.inputs
    }

    /// The output ports, in frame order.
    pub fn output_slots(&self) -> &[OutputSlot] {
        &self.outputs
    }

    /// The properties, as `(net, kind)`, attribute-declared ones first.
    pub fn properties(&self) -> &[(NetId, PropertyKind)] {
        &self.properties
    }

    /// True when the module has flip-flops or memories.
    pub fn has_state(&self) -> bool {
        !self.state.is_empty()
    }

    // ----- analysis -------------------------------------------------------

    fn error(&mut self, code: &'static str, span: Span, message: String) {
        self.diags
            .push(Diagnostic::error(message).with_code(code).with_span(span));
    }

    fn warning(&mut self, code: &'static str, span: Span, message: String) {
        self.diags
            .push(Diagnostic::warning(message).with_code(code).with_span(span));
    }

    fn analyse(&mut self, options: &BlastOptions) {
        let m = self.module;
        if m.blackbox {
            self.error(
                "F0006",
                m.span,
                format!("module `{}` is a black box and cannot be checked", m.name),
            );
            return;
        }
        for (_, p) in m.processes.iter() {
            self.error(
                "F0001",
                p.span,
                "process form is not supported by the formal engines: run synthesis first".into(),
            );
        }
        for (id, net) in m.nets.iter() {
            if !net.ty.is_bits() {
                self.error(
                    "F0002",
                    net.span,
                    format!(
                        "net `{}` has type `{}`, only bit vectors can be blasted",
                        net.name, net.ty
                    ),
                );
            }
            if net.attrs.contains("fsm_state") {
                self.fsm_nets.push(id);
            }
        }
        for port in &m.ports {
            match port.dir {
                PortDir::In => self.inputs.push(InputSlot {
                    name: port.name.to_string(),
                    net: port.net,
                    is_port: true,
                }),
                PortDir::Out => self.outputs.push(OutputSlot {
                    name: port.name.to_string(),
                    net: port.net,
                }),
                PortDir::InOut => self.error(
                    "F0003",
                    port.span,
                    format!("inout port `{}` is not supported", port.name),
                ),
            }
        }
        self.scan_exprs();
        self.scan_assigns();
        self.scan_cells();
        self.scan_instances();
        self.scan_memories(options);
        self.collect_properties(options);
        self.order_nets();
        if self.diags.has_errors() {
            return;
        }
        // Undriven nets are free inputs.
        let driven_as_input: Vec<NetId> = self.inputs.iter().map(|i| i.net).collect();
        for (id, net) in m.nets.iter() {
            if self.drivers[id.index()].is_empty()
                && self.state_of_net[id.index()].is_none()
                && !driven_as_input.contains(&id)
            {
                self.warning(
                    "F0015",
                    net.span,
                    format!(
                        "net `{}` has no driver and is treated as a free input",
                        net.name
                    ),
                );
                self.inputs.push(InputSlot {
                    name: net.name.to_string(),
                    net: id,
                    is_port: false,
                });
            }
        }
    }

    fn scan_exprs(&mut self) {
        let m = self.module;
        let mut problems = Vec::new();
        // The constant pattern of a wildcard comparison may hold `z`: it
        // means "don't care" there, not a tri-state value.
        let mut patterns = Vec::new();
        m.for_each_expr(|_, e| {
            if let ExprKind::Binary {
                op: BinaryOp::WildEq,
                rhs,
                ..
            } = &e.kind
                && m.expr(*rhs).as_const().is_some()
            {
                patterns.push(*rhs);
            }
        });
        m.for_each_expr(|id, e| match &e.kind {
            ExprKind::Const(c) => {
                if c.bits().contains(&Bit::Z) && !patterns.contains(&id) {
                    problems.push((
                        "F0009",
                        e.span,
                        format!(
                            "constant `{c}` has `z` bits, which the 2-state engines cannot model"
                        ),
                    ));
                }
            }
            ExprKind::String(_) => {
                problems.push((
                    "F0002",
                    e.span,
                    "string expressions cannot be blasted".to_string(),
                ));
            }
            ExprKind::Call { name, .. } => {
                problems.push((
                    "F0002",
                    e.span,
                    format!(
                        "call to `{name}` is not lowered; only bit-vector operators can be blasted"
                    ),
                ));
            }
            _ => {
                if !e.ty.is_bits() {
                    problems.push((
                        "F0002",
                        e.span,
                        format!("expression of type `{}` cannot be blasted", e.ty),
                    ));
                }
            }
        });
        for (code, span, msg) in problems {
            self.error(code, span, msg);
        }
    }

    fn scan_assigns(&mut self) {
        let m = self.module;
        for (i, a) in m.assigns.iter().enumerate() {
            let mut nets = Vec::new();
            let mut mem_write = false;
            lvalue_nets(&a.target, &mut nets, &mut mem_write);
            if mem_write {
                self.error(
                    "F0012",
                    a.span,
                    "memory write in a continuous assignment: use a `memwr` cell".into(),
                );
                continue;
            }
            for net in nets {
                self.drivers[net.index()].push(Driver::Assign(i));
            }
        }
    }

    fn scan_cells(&mut self) {
        let m = self.module;
        let mut clocks: Vec<Option<NetId>> = Vec::new();
        let mut async_warned = false;
        for (id, cell) in m.cells.iter() {
            match &cell.kind {
                CellKind::Dff { reset, .. } => {
                    let q = cell.output("q").expect("validated dff has q");
                    if let Some(e) = cell.input("clk") {
                        clocks.push(m.expr(e).as_net());
                    }
                    if let Some(r) = reset
                        && r.asynchronous
                        && !async_warned
                    {
                        async_warned = true;
                        self.warning(
                            "F0011",
                            cell.span,
                            format!(
                                "asynchronous reset of `{}` is checked as a synchronous one (sampled once per frame)",
                                cell.name
                            ),
                        );
                    }
                    let net = &m.nets[q];
                    let init = reset
                        .as_ref()
                        .map(|r| r.value.clone())
                        .or_else(|| init_attr(&net.attrs, width_of(&net.ty)));
                    self.push_state(
                        StateSlot {
                            name: net.name.to_string(),
                            span: cell.span,
                            width: width_of(&net.ty),
                            signed: net.ty.is_signed(),
                            init,
                            kind: StateKind::Dff(id),
                        },
                        Some(q),
                    );
                }
                CellKind::Dlatch => self.error(
                    "F0004",
                    cell.span,
                    format!(
                        "latch `{}` is not supported: the engines model edge-triggered logic only",
                        cell.name
                    ),
                ),
                CellKind::Tristate => self.error(
                    "F0005",
                    cell.span,
                    format!("tri-state driver `{}` is not supported", cell.name),
                ),
                CellKind::Blackbox(name) => {
                    if cell.attrs.is_set("formal_free") {
                        for (_, net) in &cell.outputs {
                            self.inputs.push(InputSlot {
                                name: m.nets[*net].name.to_string(),
                                net: *net,
                                is_port: false,
                            });
                        }
                    } else {
                        self.error(
                            "F0006",
                            cell.span,
                            format!(
                                "black box `{}` of type `{name}` has no model; mark it `formal_free` to treat its outputs as unconstrained inputs",
                                cell.name
                            ),
                        );
                    }
                }
                CellKind::MemRdPort { clocked, .. } => {
                    let data = cell.output("data").expect("validated memrd has data");
                    if *clocked {
                        if let Some(e) = cell.input("clk") {
                            clocks.push(m.expr(e).as_net());
                        }
                        let net = &m.nets[data];
                        let init = init_attr(&net.attrs, width_of(&net.ty));
                        self.push_state(
                            StateSlot {
                                name: net.name.to_string(),
                                span: cell.span,
                                width: width_of(&net.ty),
                                signed: net.ty.is_signed(),
                                init,
                                kind: StateKind::RdPort(id),
                            },
                            Some(data),
                        );
                    } else {
                        self.drivers[data.index()].push(Driver::Cell(id, 0));
                    }
                }
                CellKind::MemWrPort { clocked, .. } => {
                    if *clocked {
                        if let Some(e) = cell.input("clk") {
                            clocks.push(m.expr(e).as_net());
                        }
                    } else {
                        self.error(
                            "F0007",
                            cell.span,
                            format!(
                                "unclocked write port `{}` is a latch and is not supported",
                                cell.name
                            ),
                        );
                    }
                }
                _ => {
                    for (i, (_, net)) in cell.outputs.iter().enumerate() {
                        self.drivers[net.index()].push(Driver::Cell(id, i));
                    }
                }
            }
        }
        let mut distinct: Vec<Option<NetId>> = clocks.clone();
        distinct.sort();
        distinct.dedup();
        if distinct.len() > 1 {
            let names: Vec<String> = distinct
                .iter()
                .map(|c| match c {
                    Some(n) => format!("`{}`", m.nets[*n].name),
                    None => "<expression>".to_string(),
                })
                .collect();
            self.warning(
                "F0010",
                m.span,
                format!(
                    "flip-flops use {} different clocks ({}); they are checked as one clock domain",
                    distinct.len(),
                    names.join(", ")
                ),
            );
        }
    }

    fn scan_instances(&mut self) {
        let m = self.module;
        for (_, inst) in m.instances.iter() {
            if !inst.attrs.is_set("formal_free") {
                self.error(
                    "F0006",
                    inst.span,
                    format!(
                        "instance `{}` is not supported: flatten the design first, or mark it `formal_free` to treat its outputs as unconstrained inputs",
                        inst.name
                    ),
                );
                continue;
            }
            let ModuleRef::Resolved(_) = inst.module else {
                self.error(
                    "F0006",
                    inst.span,
                    format!("instance `{}` refers to a module outside the design, so its port directions are unknown", inst.name),
                );
                continue;
            };
            // Without the design we cannot see the target's port list; the
            // caller passes only a module. Treat every connection to a bare
            // net that nothing else drives as an output.
            for (_, e) in &inst.connections {
                if let Some(net) = m.expr(*e).as_net()
                    && self.drivers[net.index()].is_empty()
                    && m.port_of_net(net).is_none_or(|p| p.dir != PortDir::In)
                {
                    self.inputs.push(InputSlot {
                        name: m.nets[net].name.to_string(),
                        net,
                        is_port: false,
                    });
                }
            }
        }
    }

    fn scan_memories(&mut self, options: &BlastOptions) {
        let m = self.module;
        for (id, mem) in m.memories.iter() {
            let width = width_of(&mem.elem);
            let bits = mem.size.saturating_mul(u64::from(width));
            let start = self.state.len();
            if !mem.elem.is_bits() {
                self.error(
                    "F0002",
                    mem.span,
                    format!(
                        "memory `{}` has element type `{}`, only bit vectors can be blasted",
                        mem.name, mem.elem
                    ),
                );
            } else if bits > options.max_memory_bits {
                self.error(
                    "F0008",
                    mem.span,
                    format!(
                        "memory `{}` has {} × {} = {bits} bits, above the limit of {} for word-by-word blasting",
                        mem.name, mem.size, width, options.max_memory_bits
                    ),
                );
            } else {
                for i in 0..mem.size {
                    let init = mem
                        .init
                        .as_ref()
                        .and_then(|v| usize::try_from(i).ok().and_then(|i| v.get(i)).cloned());
                    self.push_state(
                        StateSlot {
                            name: format!("{}[{i}]", mem.name),
                            span: mem.span,
                            width,
                            signed: mem.elem.is_signed(),
                            init,
                            kind: StateKind::MemWord(id, i),
                        },
                        None,
                    );
                }
            }
            self.mem_slots.push(start..self.state.len());
        }
    }

    fn push_state(&mut self, slot: StateSlot, net: Option<NetId>) {
        if let Some(net) = net {
            self.state_of_net[net.index()] = Some(self.state.len());
        }
        self.state.push(slot);
    }

    fn collect_properties(&mut self, options: &BlastOptions) {
        let m = self.module;
        let mut props: Vec<(NetId, PropertyKind)> = Vec::new();
        for (id, net) in m.nets.iter() {
            for kind in [
                PropertyKind::Assert,
                PropertyKind::Assume,
                PropertyKind::Cover,
            ] {
                if net.attrs.is_set(kind.attribute()) {
                    props.push((id, kind));
                }
            }
        }
        for &(net, kind) in &options.properties {
            if !props.contains(&(net, kind)) {
                props.push((net, kind));
            }
        }
        for (net, kind) in props {
            let Some(n) = m.nets.get(net) else {
                self.error(
                    "F0014",
                    m.span,
                    format!("property net {net} does not exist"),
                );
                continue;
            };
            if !n.ty.is_bit() {
                self.error(
                    "F0014",
                    n.span,
                    format!(
                        "{} property `{}` must be one bit wide, it is `{}`",
                        kind.keyword(),
                        n.name,
                        n.ty
                    ),
                );
                continue;
            }
            self.properties.push((net, kind));
        }
    }

    /// The nets a driver reads in the same frame.
    fn driver_deps(&self, driver: Driver) -> Vec<NetId> {
        let m = self.module;
        let mut nets = Vec::new();
        match driver {
            Driver::Assign(i) => {
                let a = &m.assigns[i];
                expr_nets(m, a.value, &mut nets);
                let mut idx = Vec::new();
                lvalue_index_exprs(&a.target, &mut idx);
                for e in idx {
                    expr_nets(m, e, &mut nets);
                }
            }
            Driver::Cell(id, _) => {
                for (_, e) in &m.cells[id].inputs {
                    expr_nets(m, *e, &mut nets);
                }
            }
        }
        nets.sort();
        nets.dedup();
        nets
    }

    /// Topologically sorts the nets; reports combinational loops.
    fn order_nets(&mut self) {
        let n = self.module.nets.len();
        let deps: Vec<Vec<NetId>> = (0..n)
            .map(|i| {
                if self.state_of_net[i].is_some() {
                    return Vec::new();
                }
                let mut all = Vec::new();
                for &d in &self.drivers[i] {
                    all.extend(self.driver_deps(d));
                }
                all.sort();
                all.dedup();
                all
            })
            .collect();
        // Iterative DFS with colours: 0 new, 1 on the stack, 2 done.
        let mut colour = vec![0u8; n];
        let mut order = Vec::with_capacity(n);
        let mut reported = false;
        for root in 0..n {
            if colour[root] != 0 {
                continue;
            }
            let mut stack: Vec<(usize, usize)> = vec![(root, 0)];
            colour[root] = 1;
            while let Some(&mut (node, ref mut next)) = stack.last_mut() {
                if *next < deps[node].len() {
                    let child = deps[node][*next].index();
                    *next += 1;
                    match colour[child] {
                        0 => {
                            colour[child] = 1;
                            stack.push((child, 0));
                        }
                        1 if !reported => {
                            reported = true;
                            let net = &self.module.nets[NetId::from_index(child)];
                            let span = net.span;
                            let name = net.name.to_string();
                            self.error(
                                "F0013",
                                span,
                                format!("combinational loop through net `{name}`"),
                            );
                        }
                        _ => {}
                    }
                } else {
                    colour[node] = 2;
                    order.push(NetId::from_index(node));
                    stack.pop();
                }
            }
        }
        self.order = order;
    }

    // ----- blasting -------------------------------------------------------

    /// Encodes one frame into `cnf`.
    ///
    /// `state` supplies the current-state literals (one vector per
    /// [`StateSlot`], usually the previous frame's `next_state`); `None`
    /// creates fresh variables. `inputs` likewise supplies the input
    /// literals (one vector per [`InputSlot`]) or leaves them free.
    ///
    /// # Panics
    ///
    /// Panics if a supplied vector has the wrong count or width.
    pub fn blast_frame(
        &self,
        cnf: &mut CnfBuilder,
        state: Option<&[Vec<Lit>]>,
        inputs: Option<&[Vec<Lit>]>,
    ) -> BlastedFrame {
        let m = self.module;
        let mut ev = Eval {
            b: self,
            cnf,
            nets: vec![None; m.nets.len()],
            state_lits: Vec::new(),
            exprs: HashMap::new(),
            probes: Vec::new(),
        };
        let mut frame = BlastedFrame::default();

        // State.
        for (i, slot) in self.state.iter().enumerate() {
            let lits = match state {
                Some(s) => {
                    assert_eq!(s.len(), self.state.len(), "state slot count");
                    assert_eq!(s[i].len(), to_usize(slot.width), "state slot width");
                    s[i].clone()
                }
                None => ev.cnf.new_vars(to_usize(slot.width)),
            };
            if let StateKind::Dff(c) | StateKind::RdPort(c) = slot.kind {
                let net = m.cells[c]
                    .outputs
                    .first()
                    .map(|(_, n)| *n)
                    .expect("state cell has an output");
                ev.nets[net.index()] = Some(lits.clone());
            }
            ev.state_lits.push(lits.clone());
            frame.state.push(Signal {
                name: slot.name.clone(),
                span: slot.span,
                signed: slot.signed,
                lits,
            });
        }

        // Inputs.
        for (i, slot) in self.inputs.iter().enumerate() {
            let net = &m.nets[slot.net];
            let width = to_usize(width_of(&net.ty));
            let lits = match inputs {
                Some(s) => {
                    assert_eq!(s.len(), self.inputs.len(), "input slot count");
                    assert_eq!(s[i].len(), width, "input slot width");
                    s[i].clone()
                }
                None => ev.cnf.new_vars(width),
            };
            ev.nets[slot.net.index()] = Some(lits.clone());
            frame.inputs.push(Signal {
                name: slot.name.clone(),
                span: net.span,
                signed: net.ty.is_signed(),
                lits,
            });
        }

        // Combinational logic, in dependency order.
        for &net in &self.order {
            if ev.nets[net.index()].is_some() {
                continue;
            }
            let lits = ev.net_value(net);
            ev.nets[net.index()] = Some(lits);
        }

        // Next state.
        let mut next: Vec<Vec<Lit>> = frame.state.iter().map(|s| s.lits.clone()).collect();
        for (i, slot) in self.state.iter().enumerate() {
            match slot.kind {
                StateKind::Dff(c) => {
                    let cell = &m.cells[c];
                    let CellKind::Dff { reset, .. } = &cell.kind else {
                        unreachable!()
                    };
                    let q = next[i].clone();
                    let d = ev.expr(cell.input("d").expect("dff has d"));
                    let mut value = fit(ev.cnf, &d, q.len(), false);
                    if let Some(en) = cell.input("en") {
                        let en = ev.expr(en);
                        let en = en.first().copied().unwrap_or(ev.cnf.true_lit());
                        ev.probes.push(Probe {
                            kind: ProbeKind::DffEnable,
                            name: cell.name.to_string(),
                            span: cell.span,
                            lits: vec![en],
                        });
                        value = ite_vec(ev.cnf, en, &value, &q);
                    }
                    if let (Some(r), Some(rst)) = (reset, cell.input("rst")) {
                        let rst = ev.expr(rst);
                        let rst = rst.first().copied().unwrap_or(ev.cnf.false_lit());
                        let active = if r.active_high { rst } else { !rst };
                        let rv = const_lits(ev.cnf, &r.value);
                        let rv = fit(ev.cnf, &rv, q.len(), false);
                        value = ite_vec(ev.cnf, active, &rv, &value);
                    }
                    next[i] = value;
                }
                StateKind::RdPort(c) => {
                    let cell = &m.cells[c];
                    let CellKind::MemRdPort { mem, .. } = cell.kind else {
                        unreachable!()
                    };
                    let q = next[i].clone();
                    let addr = ev.expr(cell.input("addr").expect("memrd has addr"));
                    let read = ev.read_mem(mem, &addr);
                    let read = fit(ev.cnf, &read, q.len(), false);
                    let value = match cell.input("en") {
                        Some(en) => {
                            let en = ev.expr(en);
                            let en = en.first().copied().unwrap_or(ev.cnf.true_lit());
                            ite_vec(ev.cnf, en, &read, &q)
                        }
                        None => read,
                    };
                    next[i] = value;
                }
                StateKind::MemWord(..) => {}
            }
        }
        // Memory writes, in cell order.
        for (_, cell) in m.cells.iter() {
            let CellKind::MemWrPort { mem, clocked: true } = cell.kind else {
                continue;
            };
            let range = self.mem_slots[mem.index()].clone();
            if range.is_empty() {
                continue;
            }
            let addr = ev.expr(cell.input("addr").expect("memwr has addr"));
            let data = ev.expr(cell.input("data").expect("memwr has data"));
            let en = ev.expr(cell.input("en").expect("memwr has en"));
            let en = en.first().copied().unwrap_or(ev.cnf.true_lit());
            for (word, slot) in range.enumerate() {
                let hit = decode(ev.cnf, &addr, word_index(word));
                let we = ev.cnf.and(en, hit);
                let data = fit(ev.cnf, &data, next[slot].len(), false);
                next[slot] = ite_vec(ev.cnf, we, &data, &next[slot]);
            }
        }
        frame.next_state = next;

        // Outputs, properties, internal nets.
        for slot in &self.outputs {
            let net = &m.nets[slot.net];
            frame.outputs.push(Signal {
                name: slot.name.clone(),
                span: net.span,
                signed: net.ty.is_signed(),
                lits: ev.nets[slot.net.index()].clone().unwrap_or_default(),
            });
        }
        for &(net, kind) in &self.properties {
            let n = &m.nets[net];
            let lit = ev.nets[net.index()]
                .as_ref()
                .and_then(|l| l.first().copied())
                .unwrap_or(ev.cnf.true_lit());
            let p = PropertyLit {
                name: n.name.to_string(),
                span: n.span,
                lit,
            };
            match kind {
                PropertyKind::Assert => frame.asserts.push(p),
                PropertyKind::Assume => frame.assumes.push(p),
                PropertyKind::Cover => frame.covers.push(p),
            }
        }
        for &net in &self.fsm_nets {
            let n = &m.nets[net];
            ev.probes.push(Probe {
                kind: ProbeKind::FsmState,
                name: n.name.to_string(),
                span: n.span,
                lits: ev.nets[net.index()].clone().unwrap_or_default(),
            });
        }
        let mut named: Vec<NetId> = self.inputs.iter().map(|s| s.net).collect();
        named.extend(self.outputs.iter().map(|s| s.net));
        named.extend(
            self.state_of_net
                .iter()
                .enumerate()
                .filter_map(|(i, s)| s.map(|_| NetId::from_index(i))),
        );
        for (id, net) in m.nets.iter() {
            if named.contains(&id) {
                continue;
            }
            frame.internal.push(Signal {
                name: net.name.to_string(),
                span: net.span,
                signed: net.ty.is_signed(),
                lits: ev.nets[id.index()].clone().unwrap_or_default(),
            });
        }
        frame.probes = ev.probes;
        frame
    }
}

// ----- static helpers ------------------------------------------------------

fn width_of(ty: &Type) -> u32 {
    ty.width().unwrap_or(0)
}

fn to_usize(w: u32) -> usize {
    usize::try_from(w).expect("u32 fits in usize")
}

fn word_index(i: usize) -> u64 {
    u64::try_from(i).expect("usize fits in u64")
}

/// The `init` attribute of a net as a value of `width` bits, if present.
fn init_attr(attrs: &crate::ir::Attrs, width: u32) -> Option<Logic> {
    match attrs.get("init")? {
        AttrValue::Const(c) => Some(c.resize(width)),
        AttrValue::Int(v) => Some(Logic::from_i64(*v, width).as_unsigned()),
        AttrValue::String(_) => None,
    }
}

/// The nets written by an lvalue; `mem_write` is set for a memory element.
fn lvalue_nets(lv: &Lvalue, out: &mut Vec<NetId>, mem_write: &mut bool) {
    match lv {
        Lvalue::Net(n) | Lvalue::Slice { net: n, .. } | Lvalue::Index { net: n, .. } => {
            out.push(*n);
        }
        Lvalue::Concat(parts) => {
            for p in parts {
                lvalue_nets(p, out, mem_write);
            }
        }
        Lvalue::MemElem { .. } => *mem_write = true,
    }
}

/// The index expressions inside an lvalue.
fn lvalue_index_exprs(lv: &Lvalue, out: &mut Vec<ExprId>) {
    match lv {
        Lvalue::Index { index, .. } => out.push(*index),
        Lvalue::Concat(parts) => {
            for p in parts {
                lvalue_index_exprs(p, out);
            }
        }
        Lvalue::MemElem { addr, .. } => out.push(*addr),
        Lvalue::Net(_) | Lvalue::Slice { .. } => {}
    }
}

/// The nets an expression reads.
fn expr_nets(m: &Module, id: ExprId, out: &mut Vec<NetId>) {
    let e = m.expr(id);
    if let ExprKind::Net(n) = e.kind {
        out.push(n);
    }
    for op in crate::ir::expr::operands(&e.kind) {
        expr_nets(m, op, out);
    }
}

/// The literals of a constant; `x` and `z` bits become fresh variables.
pub fn const_lits(cnf: &mut CnfBuilder, c: &Logic) -> Vec<Lit> {
    c.bits()
        .into_iter()
        .map(|b| match b {
            Bit::Zero => cnf.false_lit(),
            Bit::One => cnf.true_lit(),
            Bit::X | Bit::Z => cnf.new_var(),
        })
        .collect()
}

/// The literals of an unsigned constant of `width` bits.
fn u64_lits(cnf: &CnfBuilder, value: u64, width: usize) -> Vec<Lit> {
    (0..width)
        .map(|i| {
            if i < 64 && (value >> i) & 1 == 1 {
                cnf.true_lit()
            } else {
                cnf.false_lit()
            }
        })
        .collect()
}

/// Truncates or extends `v` to `width` bits.
fn fit(cnf: &CnfBuilder, v: &[Lit], width: usize, sign_extend: bool) -> Vec<Lit> {
    let mut out: Vec<Lit> = v.iter().take(width).copied().collect();
    let ext = if sign_extend {
        v.last().copied().unwrap_or(cnf.false_lit())
    } else {
        cnf.false_lit()
    };
    out.resize(width, ext);
    out
}

/// Per-bit `c ? t : e`.
fn ite_vec(cnf: &mut CnfBuilder, c: Lit, t: &[Lit], e: &[Lit]) -> Vec<Lit> {
    debug_assert_eq!(t.len(), e.len());
    t.iter().zip(e).map(|(&x, &y)| cnf.ite(c, x, y)).collect()
}

/// `a - b` as `(difference, no_borrow)`; `no_borrow` is `a >= b`.
fn sub_vec(cnf: &mut CnfBuilder, a: &[Lit], b: &[Lit]) -> (Vec<Lit>, Lit) {
    debug_assert_eq!(a.len(), b.len());
    let mut out = Vec::with_capacity(a.len());
    let mut carry = cnf.true_lit();
    for (&x, &y) in a.iter().zip(b) {
        let (s, c) = cnf.full_adder(x, !y, carry);
        out.push(s);
        carry = c;
    }
    (out, carry)
}

/// `a + b` modulo the width.
fn add_vec(cnf: &mut CnfBuilder, a: &[Lit], b: &[Lit]) -> Vec<Lit> {
    let mut s = cnf.add(a, b);
    s.truncate(a.len());
    s
}

/// Two's complement negation.
fn neg_vec(cnf: &mut CnfBuilder, a: &[Lit]) -> Vec<Lit> {
    let zero = vec![cnf.false_lit(); a.len()];
    sub_vec(cnf, &zero, a).0
}

/// `a * b` modulo the width.
fn mul_vec(cnf: &mut CnfBuilder, a: &[Lit], b: &[Lit]) -> Vec<Lit> {
    if a.is_empty() {
        return Vec::new();
    }
    let mut p = cnf.mul(a, b);
    p.truncate(a.len());
    p
}

/// Unsigned restoring division: `(quotient, remainder)`. Division by zero
/// yields an all-ones quotient and the dividend as remainder.
fn udivmod(cnf: &mut CnfBuilder, a: &[Lit], b: &[Lit]) -> (Vec<Lit>, Vec<Lit>) {
    let w = a.len();
    let f = cnf.false_lit();
    let mut q = vec![f; w];
    let mut r = vec![f; w + 1];
    let mut wide_b = b.to_vec();
    wide_b.push(f);
    for i in (0..w).rev() {
        // r = (r << 1) | a[i], keeping w + 1 bits.
        let mut shifted = vec![a[i]];
        shifted.extend_from_slice(&r[..w]);
        let (diff, ge) = sub_vec(cnf, &shifted, &wide_b);
        q[i] = ge;
        r = ite_vec(cnf, ge, &diff, &shifted);
    }
    r.truncate(w);
    (q, r)
}

/// Signed-aware division and remainder (truncating; remainder takes the
/// sign of the dividend).
fn divmod(cnf: &mut CnfBuilder, a: &[Lit], b: &[Lit], signed: bool) -> (Vec<Lit>, Vec<Lit>) {
    if a.is_empty() {
        return (Vec::new(), Vec::new());
    }
    if !signed {
        return udivmod(cnf, a, b);
    }
    let a_neg = a[a.len() - 1];
    let b_neg = b[b.len() - 1];
    let na = neg_vec(cnf, a);
    let nb = neg_vec(cnf, b);
    let abs_a = ite_vec(cnf, a_neg, &na, a);
    let abs_b = ite_vec(cnf, b_neg, &nb, b);
    let (q, r) = udivmod(cnf, &abs_a, &abs_b);
    let flip = cnf.xor(a_neg, b_neg);
    let nq = neg_vec(cnf, &q);
    let nr = neg_vec(cnf, &r);
    let q = ite_vec(cnf, flip, &nq, &q);
    let r = ite_vec(cnf, a_neg, &nr, &r);
    (q, r)
}

/// `a < b`, signed or unsigned.
fn lt_vec(cnf: &mut CnfBuilder, a: &[Lit], b: &[Lit], signed: bool) -> Lit {
    if a.is_empty() {
        return cnf.false_lit();
    }
    let (a, b) = if signed {
        let mut a = a.to_vec();
        let mut b = b.to_vec();
        let n = a.len() - 1;
        a[n] = !a[n];
        b[n] = !b[n];
        (a, b)
    } else {
        (a.to_vec(), b.to_vec())
    };
    let (_, ge) = sub_vec(cnf, &a, &b);
    !ge
}

/// A barrel shifter; `left` selects the direction and `fill` the bit
/// shifted in. Amounts at or above the width shift everything out.
fn shift(cnf: &mut CnfBuilder, a: &[Lit], amount: &[Lit], left: bool, fill: Lit) -> Vec<Lit> {
    let w = a.len();
    let mut cur = a.to_vec();
    let mut overflow = Vec::new();
    for (i, &bit) in amount.iter().enumerate() {
        if i >= 31 || (1usize << i) >= w {
            overflow.push(bit);
            continue;
        }
        let sh = 1usize << i;
        let shifted: Vec<Lit> = (0..w)
            .map(|j| {
                if left {
                    if j >= sh { cur[j - sh] } else { fill }
                } else if j + sh < w {
                    cur[j + sh]
                } else {
                    fill
                }
            })
            .collect();
        cur = ite_vec(cnf, bit, &shifted, &cur);
    }
    if !overflow.is_empty() {
        let any = cnf.or_n(&overflow);
        let all_fill = vec![fill; w];
        cur = ite_vec(cnf, any, &all_fill, &cur);
    }
    cur
}

/// `addr == index`, false when `index` does not fit in the address width.
fn decode(cnf: &mut CnfBuilder, addr: &[Lit], index: u64) -> Lit {
    if addr.len() < 64 && index >> addr.len() != 0 {
        return cnf.false_lit();
    }
    let k = u64_lits(cnf, index, addr.len());
    cnf.eq_vec(addr, &k)
}

/// `words[index]` through a mux tree; an out-of-range index reads free
/// bits. Every word has the same width.
fn select(cnf: &mut CnfBuilder, words: &[Vec<Lit>], index: &[Lit]) -> Vec<Lit> {
    let size = words.len();
    let width = words.first().map_or(0, Vec::len);
    if size == 0 {
        return cnf.new_vars(width);
    }
    let needed = usize::BITS - (size - 1).leading_zeros();
    let needed = to_usize(needed);
    let used = needed.min(index.len());
    let reachable = if used >= 63 {
        size
    } else {
        size.min(1 << used)
    };
    let f = cnf.false_lit();
    let mut level: Vec<Vec<Lit>> = words[..reachable].to_vec();
    level.resize(1 << used, vec![f; width]);
    for &bit in &index[..used] {
        level = level
            .chunks(2)
            .map(|pair| ite_vec(cnf, bit, &pair[1], &pair[0]))
            .collect();
    }
    let mut out = level.into_iter().next().unwrap_or_default();
    // In range: index < size, unless every index value is a valid word.
    let always = index.len() < 64 && (1u64 << index.len()) <= word_index(size);
    if !always {
        let bound = u64_lits(cnf, word_index(size), index.len());
        let in_range = lt_vec(cnf, index, &bound, false);
        let free = cnf.new_vars(width);
        out = ite_vec(cnf, in_range, &out, &free);
    }
    out
}

/// `base ** exp` modulo the width, per IEEE 1364-2005 Table 5-6.
fn pow_vec(
    cnf: &mut CnfBuilder,
    base: &[Lit],
    exp: &[Lit],
    signed: bool,
    exp_signed: bool,
) -> Vec<Lit> {
    let w = base.len();
    if w == 0 {
        return Vec::new();
    }
    let one = u64_lits(cnf, 1, w);
    let mut acc = one.clone();
    for &bit in exp.iter().rev() {
        acc = mul_vec(cnf, &acc, &acc);
        let times = mul_vec(cnf, &acc, base);
        acc = ite_vec(cnf, bit, &times, &acc);
    }
    if !exp_signed {
        return acc;
    }
    let exp_neg = exp[exp.len() - 1];
    let zero = vec![cnf.false_lit(); w];
    let base_zero = cnf.eq_vec(base, &zero);
    let base_one = cnf.eq_vec(base, &one);
    let ones = vec![cnf.true_lit(); w];
    let base_minus_one = if signed {
        cnf.eq_vec(base, &ones)
    } else {
        cnf.false_lit()
    };
    let keep = cnf.or(base_one, base_minus_one);
    let free = cnf.new_vars(w);
    let neg_case = ite_vec(cnf, keep, &acc, &zero);
    let neg_case = ite_vec(cnf, base_zero, &free, &neg_case);
    ite_vec(cnf, exp_neg, &neg_case, &acc)
}

// ----- per-frame evaluation ------------------------------------------------

struct Eval<'b, 'a> {
    b: &'b Blaster<'a>,
    cnf: &'b mut CnfBuilder,
    nets: Vec<Option<Vec<Lit>>>,
    /// Current-state literals, one vector per slot.
    state_lits: Vec<Vec<Lit>>,
    exprs: HashMap<ExprId, Vec<Lit>>,
    probes: Vec<Probe>,
}

impl Eval<'_, '_> {
    /// The value of a combinational net: its drivers applied in order,
    /// undriven bits free.
    fn net_value(&mut self, net: NetId) -> Vec<Lit> {
        let m = self.b.module;
        let width = to_usize(width_of(&m.nets[net].ty));
        let mut bits: Vec<Option<Lit>> = vec![None; width];
        let drivers = self.b.drivers[net.index()].clone();
        for d in drivers {
            match d {
                Driver::Assign(i) => {
                    let a = &m.assigns[i];
                    let value = self.expr(a.value);
                    let total = self.lvalue_width(&a.target);
                    let value = fit(self.cnf, &value, total, false);
                    // Walk the lvalue from the MSB end.
                    let mut offset = total;
                    self.apply_lvalue(&a.target, &value, &mut offset, net, &mut bits);
                }
                Driver::Cell(c, port) => {
                    let value = self.cell_output(c, port);
                    for (i, slot) in bits.iter_mut().enumerate() {
                        if let Some(&l) = value.get(i) {
                            *slot = Some(l);
                        }
                    }
                }
            }
        }
        bits.into_iter()
            .map(|b| b.unwrap_or_else(|| self.cnf.new_var()))
            .collect()
    }

    fn lvalue_width(&self, lv: &Lvalue) -> usize {
        let m = self.b.module;
        match lv {
            Lvalue::Net(n) => to_usize(width_of(&m.nets[*n].ty)),
            Lvalue::Slice { hi, lo, .. } => to_usize(hi - lo + 1),
            Lvalue::Index { .. } => 1,
            Lvalue::Concat(parts) => parts.iter().map(|p| self.lvalue_width(p)).sum(),
            Lvalue::MemElem { .. } => 0,
        }
    }

    /// Writes the part of `value` that lands on `target` into `bits`.
    /// `offset` is the number of value bits not yet consumed, counting
    /// from the MSB end.
    fn apply_lvalue(
        &mut self,
        lv: &Lvalue,
        value: &[Lit],
        offset: &mut usize,
        target: NetId,
        bits: &mut [Option<Lit>],
    ) {
        match lv {
            Lvalue::Concat(parts) => {
                for p in parts {
                    self.apply_lvalue(p, value, offset, target, bits);
                }
            }
            Lvalue::MemElem { .. } => {}
            _ => {
                let w = self.lvalue_width(lv);
                *offset -= w;
                let part = &value[*offset..*offset + w];
                match lv {
                    Lvalue::Net(n) if *n == target => {
                        for (slot, &l) in bits.iter_mut().zip(part) {
                            *slot = Some(l);
                        }
                    }
                    Lvalue::Slice { net, lo, .. } if *net == target => {
                        for (i, &l) in part.iter().enumerate() {
                            if let Some(slot) = bits.get_mut(to_usize(*lo) + i) {
                                *slot = Some(l);
                            }
                        }
                    }
                    Lvalue::Index { net, index } if *net == target => {
                        let idx = self.expr(*index);
                        let v = part[0];
                        for (i, slot) in bits.iter_mut().enumerate() {
                            let hit = decode(self.cnf, &idx, word_index(i));
                            let prev = slot.unwrap_or_else(|| self.cnf.new_var());
                            *slot = Some(self.cnf.ite(hit, v, prev));
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    /// The value of a net already evaluated in this frame.
    fn net_lits(&mut self, net: NetId) -> Vec<Lit> {
        match &self.nets[net.index()] {
            Some(l) => l.clone(),
            None => {
                // Only reachable through a loop the analysis rejected;
                // keep going with free bits rather than panic.
                let w = to_usize(width_of(&self.b.module.nets[net].ty));
                let lits = self.cnf.new_vars(w);
                self.nets[net.index()] = Some(lits.clone());
                lits
            }
        }
    }

    /// The current contents of a memory, one vector per word.
    fn mem_words(&self, mem: MemoryId) -> Vec<Vec<Lit>> {
        let range = self.b.mem_slots[mem.index()].clone();
        self.state_lits[range].to_vec()
    }

    /// `mem[addr]` in the current frame.
    fn read_mem(&mut self, mem: MemoryId, addr: &[Lit]) -> Vec<Lit> {
        let words = self.mem_words(mem);
        if words.is_empty() {
            let w = to_usize(width_of(&self.b.module.memories[mem].elem));
            return self.cnf.new_vars(w);
        }
        select(self.cnf, &words, addr)
    }

    /// The literals of an expression, memoised per frame.
    fn expr(&mut self, id: ExprId) -> Vec<Lit> {
        if let Some(v) = self.exprs.get(&id) {
            return v.clone();
        }
        let v = self.eval(id);
        self.exprs.insert(id, v.clone());
        v
    }

    fn eval(&mut self, id: ExprId) -> Vec<Lit> {
        let m = self.b.module;
        let e = m.expr(id);
        let width = to_usize(width_of(&e.ty));
        match &e.kind {
            ExprKind::Const(c) => const_lits(self.cnf, c),
            ExprKind::String(_) | ExprKind::Call { .. } => self.cnf.new_vars(width),
            ExprKind::Net(n) => self.net_lits(*n),
            ExprKind::Slice { base, hi, lo } => {
                let b = self.expr(*base);
                let (lo, hi) = (to_usize(*lo), to_usize(*hi));
                b.get(lo..=hi)
                    .map(<[Lit]>::to_vec)
                    .unwrap_or_else(|| self.cnf.new_vars(width))
            }
            ExprKind::Index { base, index } => {
                let b = self.expr(*base);
                let i = self.expr(*index);
                let words: Vec<Vec<Lit>> = b.iter().map(|&l| vec![l]).collect();
                select(self.cnf, &words, &i)
            }
            ExprKind::IndexedSlice {
                base,
                offset,
                width: w,
                up,
            } => {
                let b = self.expr(*base);
                let off = self.expr(*offset);
                let w = to_usize(*w);
                self.indexed_slice(&b, &off, w, *up)
            }
            ExprKind::Concat(parts) => {
                let mut out = Vec::with_capacity(width);
                for p in parts.iter().rev() {
                    out.extend(self.expr(*p));
                }
                out
            }
            ExprKind::Replicate { count, expr } => {
                let v = self.expr(*expr);
                let mut out = Vec::with_capacity(width);
                for _ in 0..*count {
                    out.extend_from_slice(&v);
                }
                out
            }
            ExprKind::Unary { op, expr } => {
                let a = self.expr(*expr);
                match op {
                    UnaryOp::Not => a.iter().map(|&l| !l).collect(),
                    UnaryOp::Neg => neg_vec(self.cnf, &a),
                    UnaryOp::ReduceAnd => vec![self.cnf.and_n(&a)],
                    UnaryOp::ReduceOr => vec![self.cnf.or_n(&a)],
                    UnaryOp::ReduceXor => vec![self.cnf.xor_n(&a)],
                    UnaryOp::ReduceNand => vec![!self.cnf.and_n(&a)],
                    UnaryOp::ReduceNor | UnaryOp::LogicNot => vec![!self.cnf.or_n(&a)],
                    UnaryOp::ReduceXnor => vec![!self.cnf.xor_n(&a)],
                }
            }
            ExprKind::Binary { op, lhs, rhs } => {
                let a = self.expr(*lhs);
                let b = self.expr(*rhs);
                let ls = m.expr(*lhs).ty.is_signed();
                let rs = m.expr(*rhs).ty.is_signed();
                let pattern = if *op == BinaryOp::WildEq {
                    m.expr(*rhs).as_const().cloned()
                } else {
                    None
                };
                self.binop(*op, &a, &b, ls, rs, pattern.as_ref())
            }
            ExprKind::Ternary { cond, then_, else_ } => {
                let c = self.expr(*cond);
                let c = c.first().copied().unwrap_or(self.cnf.false_lit());
                let t = self.expr(*then_);
                let f = self.expr(*else_);
                self.probes.push(Probe {
                    kind: ProbeKind::TernaryCond,
                    name: String::new(),
                    span: e.span,
                    lits: vec![c],
                });
                let t = fit(self.cnf, &t, width, false);
                let f = fit(self.cnf, &f, width, false);
                ite_vec(self.cnf, c, &t, &f)
            }
            ExprKind::Resize { expr, signed, .. } => {
                let a = self.expr(*expr);
                let sign = *signed && m.expr(*expr).ty.is_signed();
                fit(self.cnf, &a, width, sign)
            }
            ExprKind::MemRead { mem, addr } => {
                let a = self.expr(*addr);
                self.read_mem(*mem, &a)
            }
        }
    }

    /// `base[offset +: width]` or `base[offset -: width]`; bits outside
    /// `base` are free.
    fn indexed_slice(&mut self, base: &[Lit], offset: &[Lit], width: usize, up: bool) -> Vec<Lit> {
        let bw = base.len();
        let f = self.cnf.false_lit();
        // Result bit j is base[offset + j - pad] with pad = width - 1 for a
        // downward select, so shift a zero-padded copy of base right by
        // the offset and check each bit's index against [pad, bw + pad).
        let pad = if up { 0 } else { width.saturating_sub(1) };
        let mut ext = vec![f; pad];
        ext.extend_from_slice(base);
        let shifted = shift(self.cnf, &ext, offset, false, f);
        // The index arithmetic must hold both offset + j and the upper
        // bound without wrapping.
        let bound = word_index(bw + pad + width);
        let n = (offset.len() + 1).max(to_usize(u64::BITS - bound.leading_zeros()));
        let mut off = offset.to_vec();
        off.resize(n, f);
        (0..width)
            .map(|j| {
                let jl = u64_lits(self.cnf, word_index(j), off.len());
                let t = add_vec(self.cnf, &off, &jl);
                let lo = u64_lits(self.cnf, word_index(pad), off.len());
                let hi = u64_lits(self.cnf, word_index(bw + pad), off.len());
                let below = lt_vec(self.cnf, &t, &lo, false);
                let above = !lt_vec(self.cnf, &t, &hi, false);
                let out = self.cnf.or(below, above);
                let bit = shifted.get(j).copied().unwrap_or(f);
                let free = self.cnf.new_var();
                self.cnf.ite(out, free, bit)
            })
            .collect()
    }

    /// A binary operator on two vectors; `pattern` is the constant right
    /// operand of a `WildEq`.
    fn binop(
        &mut self,
        op: BinaryOp,
        a: &[Lit],
        b: &[Lit],
        a_signed: bool,
        b_signed: bool,
        pattern: Option<&Logic>,
    ) -> Vec<Lit> {
        let cnf = &mut *self.cnf;
        let signed = a_signed && b_signed;
        match op {
            BinaryOp::And => a.iter().zip(b).map(|(&x, &y)| cnf.and(x, y)).collect(),
            BinaryOp::Or => a.iter().zip(b).map(|(&x, &y)| cnf.or(x, y)).collect(),
            BinaryOp::Xor => a.iter().zip(b).map(|(&x, &y)| cnf.xor(x, y)).collect(),
            BinaryOp::Xnor => a.iter().zip(b).map(|(&x, &y)| cnf.eq(x, y)).collect(),
            BinaryOp::LogicAnd => {
                let x = cnf.or_n(a);
                let y = cnf.or_n(b);
                vec![cnf.and(x, y)]
            }
            BinaryOp::LogicOr => {
                let x = cnf.or_n(a);
                let y = cnf.or_n(b);
                vec![cnf.or(x, y)]
            }
            BinaryOp::Add => add_vec(cnf, a, b),
            BinaryOp::Sub => sub_vec(cnf, a, b).0,
            BinaryOp::Mul => mul_vec(cnf, a, b),
            BinaryOp::Div => divmod(cnf, a, b, signed).0,
            BinaryOp::Mod => divmod(cnf, a, b, signed).1,
            BinaryOp::Pow => pow_vec(cnf, a, b, signed, b_signed),
            BinaryOp::Shl => {
                let f = cnf.false_lit();
                shift(cnf, a, b, true, f)
            }
            BinaryOp::Shr => {
                let f = cnf.false_lit();
                shift(cnf, a, b, false, f)
            }
            BinaryOp::Sshr => {
                let fill = if a_signed {
                    a.last().copied().unwrap_or(cnf.false_lit())
                } else {
                    cnf.false_lit()
                };
                shift(cnf, a, b, false, fill)
            }
            BinaryOp::Eq | BinaryOp::CaseEq => vec![cnf.eq_vec(a, b)],
            BinaryOp::Ne | BinaryOp::CaseNe => vec![!cnf.eq_vec(a, b)],
            BinaryOp::WildEq => {
                let bits: Vec<Lit> = a
                    .iter()
                    .zip(b)
                    .enumerate()
                    .filter(|(i, _)| {
                        pattern.is_none_or(|p| {
                            u32::try_from(*i)
                                .ok()
                                .and_then(|i| p.get(i))
                                .is_some_and(Bit::is_known)
                        })
                    })
                    .map(|(_, (&x, &y))| cnf.eq(x, y))
                    .collect();
                vec![cnf.and_n(&bits)]
            }
            BinaryOp::Lt => vec![lt_vec(cnf, a, b, signed)],
            BinaryOp::Gt => vec![lt_vec(cnf, b, a, signed)],
            BinaryOp::Le => vec![!lt_vec(cnf, b, a, signed)],
            BinaryOp::Ge => vec![!lt_vec(cnf, a, b, signed)],
        }
    }

    /// The value of output `port` of a combinational cell or an unclocked
    /// read port.
    fn cell_output(&mut self, id: CellId, port: usize) -> Vec<Lit> {
        let m = self.b.module;
        let cell = &m.cells[id];
        let out_net = cell.outputs[port].1;
        let width = to_usize(width_of(&m.nets[out_net].ty));
        let input = |ev: &mut Self, name: &str| -> Vec<Lit> {
            cell.input(name).map(|e| ev.expr(e)).unwrap_or_default()
        };
        let signed_in =
            |name: &str| -> bool { cell.input(name).is_some_and(|e| m.expr(e).ty.is_signed()) };
        let binary = |ev: &mut Self, op: BinaryOp| -> Vec<Lit> {
            let a = input(ev, "a");
            let b = input(ev, "b");
            let (sa, sb) = (signed_in("a"), signed_in("b"));
            let b = if op.is_shift() {
                b
            } else {
                fit(ev.cnf, &b, a.len(), false)
            };
            ev.binop(op, &a, &b, sa, sb, None)
        };
        let v = match &cell.kind {
            CellKind::Not => input(self, "a").iter().map(|&l| !l).collect(),
            CellKind::Buf => input(self, "a"),
            CellKind::And => binary(self, BinaryOp::And),
            CellKind::Or => binary(self, BinaryOp::Or),
            CellKind::Xor => binary(self, BinaryOp::Xor),
            CellKind::Add => binary(self, BinaryOp::Add),
            CellKind::Sub => binary(self, BinaryOp::Sub),
            CellKind::Mul => binary(self, BinaryOp::Mul),
            CellKind::Div => binary(self, BinaryOp::Div),
            CellKind::Mod => binary(self, BinaryOp::Mod),
            CellKind::Shl => binary(self, BinaryOp::Shl),
            CellKind::Shr => binary(self, BinaryOp::Shr),
            CellKind::Sshr => binary(self, BinaryOp::Sshr),
            CellKind::Eq => binary(self, BinaryOp::Eq),
            CellKind::Ne => binary(self, BinaryOp::Ne),
            CellKind::Lt => binary(self, BinaryOp::Lt),
            CellKind::Le => binary(self, BinaryOp::Le),
            CellKind::Gt => binary(self, BinaryOp::Gt),
            CellKind::Ge => binary(self, BinaryOp::Ge),
            CellKind::ReduceAnd => {
                let a = input(self, "a");
                vec![self.cnf.and_n(&a)]
            }
            CellKind::ReduceOr => {
                let a = input(self, "a");
                vec![self.cnf.or_n(&a)]
            }
            CellKind::ReduceXor => {
                let a = input(self, "a");
                vec![self.cnf.xor_n(&a)]
            }
            CellKind::Mux => {
                let a = input(self, "a");
                let b = input(self, "b");
                let s = input(self, "s");
                let s = s.first().copied().unwrap_or(self.cnf.false_lit());
                self.probes.push(Probe {
                    kind: ProbeKind::MuxSelect,
                    name: cell.name.to_string(),
                    span: cell.span,
                    lits: vec![s],
                });
                let a = fit(self.cnf, &a, width, false);
                let b = fit(self.cnf, &b, width, false);
                ite_vec(self.cnf, s, &b, &a)
            }
            CellKind::Pmux => {
                let a = input(self, "a");
                let b = input(self, "b");
                let s = input(self, "s");
                self.probes.push(Probe {
                    kind: ProbeKind::PmuxSelect,
                    name: cell.name.to_string(),
                    span: cell.span,
                    lits: s.clone(),
                });
                let mut y = fit(self.cnf, &a, width, false);
                // Lowest select bit has the highest priority.
                for (i, &sel) in s.iter().enumerate().rev() {
                    let part: Vec<Lit> = b
                        .get(i * width..(i + 1) * width)
                        .map(<[Lit]>::to_vec)
                        .unwrap_or_else(|| vec![self.cnf.false_lit(); width]);
                    y = ite_vec(self.cnf, sel, &part, &y);
                }
                y
            }
            CellKind::Lut { init, .. } => {
                let a = input(self, "a");
                let table: Vec<Vec<Lit>> = init
                    .bits()
                    .into_iter()
                    .map(|b| match b {
                        Bit::One => vec![self.cnf.true_lit()],
                        Bit::Zero => vec![self.cnf.false_lit()],
                        _ => vec![self.cnf.new_var()],
                    })
                    .collect();
                select(self.cnf, &table, &a)
            }
            CellKind::MemRdPort {
                mem,
                clocked: false,
            } => {
                let addr = input(self, "addr");
                self.read_mem(*mem, &addr)
            }
            _ => self.cnf.new_vars(width),
        };
        fit(self.cnf, &v, width, false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::formal::sat::{SolveResult, Solver};
    use crate::ir::builder::ModuleBuilder;
    use crate::ir::{Name, Reset};
    use crate::source::SourceMap;

    fn span() -> Span {
        let mut map = SourceMap::new();
        let id = map.add("t", "").unwrap();
        Span::new(id, 0, 0)
    }

    /// A small deterministic generator (xorshift64*).
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
        }

        fn logic(&mut self, width: u32, signed: bool) -> Logic {
            let bits: Vec<Bit> = (0..width)
                .map(|i| {
                    // Bias towards small and extreme values.
                    let r = self.next();
                    let bit = match r % 4 {
                        0 => i < 3 && r >> 8 & 1 == 1,
                        1 => i + 3 >= width && r >> 8 & 1 == 1,
                        _ => r >> 8 & 1 == 1,
                    };
                    Bit::from_bool(bit)
                })
                .collect();
            Logic::from_bits(&bits).with_signed(signed)
        }
    }

    /// Reads a vector from the model; unassigned variables read `x`.
    fn read(s: &Solver, lits: &[Lit]) -> Logic {
        let bits: Vec<Bit> = lits
            .iter()
            .map(|l| match s.value(l.var()) {
                Some(v) => Bit::from_bool(v ^ l.is_neg()),
                None => Bit::X,
            })
            .collect();
        Logic::from_bits(&bits)
    }

    /// Assumptions fixing `lits` to `value` (an `x` bit is left free).
    fn fix(lits: &[Lit], value: &Logic) -> Vec<Lit> {
        lits.iter()
            .enumerate()
            .filter_map(|(i, &l)| {
                value
                    .bit(u32::try_from(i).unwrap())
                    .to_bool()
                    .map(|b| if b { l } else { !l })
            })
            .collect()
    }

    /// True when every known bit of `expected` matches `actual`.
    fn agrees(expected: &Logic, actual: &Logic) -> bool {
        expected.width() == actual.width()
            && expected
                .bits()
                .iter()
                .zip(actual.bits())
                .all(|(e, a)| e.to_bool().is_none_or(|e| Some(e) == a.to_bool()))
    }

    /// Evaluates an expression on `Logic` values with nets bound by `env`.
    fn eval_logic(m: &Module, id: ExprId, env: &[(NetId, Logic)]) -> Logic {
        let e = m.expr(id);
        let get = |x: ExprId| eval_logic(m, x, env);
        match &e.kind {
            ExprKind::Const(c) => c.clone(),
            ExprKind::Net(n) => env.iter().find(|(id, _)| id == n).unwrap().1.clone(),
            ExprKind::Slice { base, hi, lo } => get(*base).slice(*hi, *lo),
            ExprKind::Index { base, index } => {
                let b = get(*base);
                let i = get(*index);
                match i.to_u64().and_then(|i| u32::try_from(i).ok()) {
                    Some(i) => Logic::from_bit(b.get(i).unwrap_or(Bit::X)),
                    None => Logic::x(1),
                }
            }
            ExprKind::IndexedSlice {
                base,
                offset,
                width,
                up,
            } => {
                let b = get(*base);
                let o = get(*offset).to_u64().and_then(|o| i64::try_from(o).ok());
                let bits: Vec<Bit> = (0..*width)
                    .map(|j| {
                        let idx = o.map(|o| {
                            if *up {
                                o + i64::from(j)
                            } else {
                                o - i64::from(*width) + 1 + i64::from(j)
                            }
                        });
                        idx.and_then(|i| u32::try_from(i).ok())
                            .and_then(|i| b.get(i))
                            .unwrap_or(Bit::X)
                    })
                    .collect();
                Logic::from_bits(&bits)
            }
            ExprKind::Concat(parts) => {
                let vals: Vec<Logic> = parts.iter().map(|p| get(*p)).collect();
                Logic::concat_all(&vals)
            }
            ExprKind::Replicate { count, expr } => get(*expr).replicate(*count),
            ExprKind::Unary { op, expr } => {
                let a = get(*expr);
                match op {
                    UnaryOp::Not => a.not(),
                    UnaryOp::Neg => a.neg(),
                    UnaryOp::ReduceAnd => a.reduce_and(),
                    UnaryOp::ReduceOr => a.reduce_or(),
                    UnaryOp::ReduceXor => a.reduce_xor(),
                    UnaryOp::ReduceNand => a.reduce_nand(),
                    UnaryOp::ReduceNor => a.reduce_nor(),
                    UnaryOp::ReduceXnor => a.reduce_xnor(),
                    UnaryOp::LogicNot => a.logical_not(),
                }
            }
            ExprKind::Binary { op, lhs, rhs } => {
                let a = get(*lhs);
                let b = get(*rhs);
                match op {
                    BinaryOp::And => a.and(&b),
                    BinaryOp::Or => a.or(&b),
                    BinaryOp::Xor => a.xor(&b),
                    BinaryOp::Xnor => a.xnor(&b),
                    BinaryOp::LogicAnd => a.logical_and(&b),
                    BinaryOp::LogicOr => a.logical_or(&b),
                    BinaryOp::Add => a.add(&b),
                    BinaryOp::Sub => a.sub(&b),
                    BinaryOp::Mul => a.mul(&b),
                    BinaryOp::Div => a.div(&b),
                    BinaryOp::Mod => a.rem(&b),
                    BinaryOp::Pow => a.pow(&b),
                    BinaryOp::Shl => a.shl_by(&b),
                    BinaryOp::Shr => a.shr_by(&b),
                    BinaryOp::Sshr => a.sshr_by(&b),
                    BinaryOp::Eq => a.eq(&b),
                    BinaryOp::Ne => a.ne(&b),
                    BinaryOp::CaseEq => a.case_eq(&b),
                    BinaryOp::CaseNe => a.case_ne(&b),
                    BinaryOp::WildEq => a.wildcard_eq(&b),
                    BinaryOp::Lt => a.lt(&b),
                    BinaryOp::Le => a.le(&b),
                    BinaryOp::Gt => a.gt(&b),
                    BinaryOp::Ge => a.ge(&b),
                }
            }
            ExprKind::Ternary { cond, then_, else_ } => match get(*cond).bit(0) {
                Bit::One => get(*then_),
                Bit::Zero => get(*else_),
                _ => Logic::x(e.ty.width().unwrap()),
            },
            ExprKind::Resize {
                expr,
                width,
                signed,
            } => {
                let a = get(*expr);
                let a = if *signed { a } else { a.as_unsigned() };
                a.resize(*width).with_signed(*signed)
            }
            _ => unreachable!("not used in tests"),
        }
    }

    const WIDTHS: [u32; 6] = [1, 7, 8, 33, 64, 65];

    /// Builds a module with inputs `a`, `b` (width `w`), `c`, `d` (1 bit)
    /// and one output per operator; returns it with the output nets and
    /// the expressions behind them.
    fn operator_module(w: u32, signed: bool) -> (Module, Vec<(NetId, ExprId)>) {
        let mut b = ModuleBuilder::new("ops", span());
        let ty = Type::Bits { width: w, signed };
        let a_n = b.input("a", ty.clone());
        let b_n = b.input("b", ty.clone());
        let c_n = b.input("c", Type::bit());
        let d_n = b.input("d", Type::bit());
        let (a, bb, c, d) = (b.net(a_n), b.net(b_n), b.net(c_n), b.net(d_n));
        let mut outs = Vec::new();
        let mut exprs = Vec::new();
        for op in BinaryOp::ALL {
            if op == BinaryOp::Pow && w > 8 {
                continue;
            }
            let e = if op.is_logical() {
                b.binary(op, c, d)
            } else {
                b.binary(op, a, bb)
            };
            exprs.push((format!("bin_{}", op.name()), e));
        }
        for op in UnaryOp::ALL {
            exprs.push((format!("un_{}", op.name()), b.unary(op, a)));
        }
        exprs.push(("ternary".into(), b.mux(c, a, bb)));
        exprs.push(("zext".into(), b.zext(a, w + 3)));
        exprs.push(("sext".into(), b.sext(a, w + 3)));
        exprs.push(("trunc".into(), b.resize(a, w.max(2) - 1, signed)));
        exprs.push(("slice".into(), b.slice(a, w - 1, 0)));
        exprs.push(("concat".into(), b.concat(vec![a, bb, c])));
        exprs.push(("rep".into(), b.replicate(3, bb)));
        exprs.push(("index_wide".into(), b.index(a, bb)));
        let small = b.zext(bb, 3);
        exprs.push(("index_small".into(), b.index(a, small)));
        let off = b.zext(bb, 4);
        let part = w.min(3);
        exprs.push(("islice_up".into(), b.indexed_slice(a, off, part, true)));
        exprs.push(("islice_down".into(), b.indexed_slice(a, off, part, false)));
        let pattern = b.constant(Logic::parse_verilog("4'b1x0z").unwrap());
        let a4 = b.zext(a, 4);
        exprs.push(("wild".into(), b.binary(BinaryOp::WildEq, a4, pattern)));
        for (name, e) in exprs {
            let ty = b.module().expr(e).ty.clone();
            let n = b.output(name, ty);
            b.assign(n, e);
            outs.push((n, e));
        }
        (b.finish(), outs)
    }

    #[test]
    fn expressions_match_logic_evaluation() {
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        for &w in &WIDTHS {
            for signed in [false, true] {
                let (m, outs) = operator_module(w, signed);
                let blaster =
                    Blaster::new(&m, &BlastOptions::default()).unwrap_or_else(|d| panic!("{d:?}"));
                let mut cnf = CnfBuilder::new();
                let frame = blaster.blast_frame(&mut cnf, None, None);
                let mut solver = cnf.into_solver();
                let inputs: Vec<(NetId, &Signal)> = blaster
                    .input_slots()
                    .iter()
                    .zip(&frame.inputs)
                    .map(|(s, sig)| (s.net, sig))
                    .collect();
                for _ in 0..5 {
                    let mut env = Vec::new();
                    let mut assumptions = Vec::new();
                    for (net, sig) in &inputs {
                        let width = u32::try_from(sig.lits.len()).unwrap();
                        let v = rng.logic(width, sig.signed);
                        assumptions.extend(fix(&sig.lits, &v));
                        env.push((*net, v));
                    }
                    assert_eq!(
                        solver.solve_with_assumptions(&assumptions),
                        SolveResult::Sat
                    );
                    for ((net, e), sig) in outs.iter().zip(&frame.outputs) {
                        let expected = eval_logic(&m, *e, &env);
                        let actual = read(&solver, &sig.lits);
                        assert!(
                            agrees(&expected, &actual),
                            "width {w} signed {signed} output `{}`: expected {expected} got {actual} (env {:?})",
                            m.nets[*net].name,
                            env.iter().map(|(_, v)| v.to_string()).collect::<Vec<_>>()
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn division_by_zero_follows_synthesis_convention() {
        for signed in [false, true] {
            let (m, outs) = operator_module(8, signed);
            let blaster = Blaster::new(&m, &BlastOptions::default()).unwrap();
            let mut cnf = CnfBuilder::new();
            let frame = blaster.blast_frame(&mut cnf, None, None);
            let mut solver = cnf.into_solver();
            let a = Logic::from_u64(0x93, 8).with_signed(signed);
            let zero = Logic::zero(8).with_signed(signed);
            let mut assumptions = fix(&frame.inputs[0].lits, &a);
            assumptions.extend(fix(&frame.inputs[1].lits, &zero));
            assert_eq!(
                solver.solve_with_assumptions(&assumptions),
                SolveResult::Sat
            );
            let find = |name: &str| {
                let i = outs
                    .iter()
                    .position(|(n, _)| m.nets[*n].name.as_str() == name)
                    .unwrap();
                read(&solver, &frame.outputs[i].lits).to_u64().unwrap()
            };
            if signed {
                // |a| = 0x6d, |0| = 0: the magnitude quotient is all ones,
                // negated because the signs differ: 0x01; the magnitude
                // remainder is |a|, negated back: a itself.
                assert_eq!(find("bin_div"), 0x01);
                assert_eq!(find("bin_mod"), 0x93);
            } else {
                assert_eq!(find("bin_div"), 0xff);
                assert_eq!(find("bin_mod"), 0x93);
            }
        }
    }

    /// Builds a module with one cell of each combinational kind.
    fn cell_module(w: u32, signed: bool) -> (Module, Vec<(NetId, ExprId)>) {
        let mut b = ModuleBuilder::new("cells", span());
        let ty = Type::Bits { width: w, signed };
        let a_n = b.input("a", ty.clone());
        let b_n = b.input("b", ty.clone());
        let s_n = b.input("s", Type::bit());
        let sel_n = b.input("sel", Type::bits(3));
        let (a, bb, s, sel) = (b.net(a_n), b.net(b_n), b.net(s_n), b.net(sel_n));
        let mut outs = Vec::new();
        let two_input = [
            (CellKind::And, BinaryOp::And),
            (CellKind::Or, BinaryOp::Or),
            (CellKind::Xor, BinaryOp::Xor),
            (CellKind::Add, BinaryOp::Add),
            (CellKind::Sub, BinaryOp::Sub),
            (CellKind::Mul, BinaryOp::Mul),
            (CellKind::Div, BinaryOp::Div),
            (CellKind::Mod, BinaryOp::Mod),
            (CellKind::Shl, BinaryOp::Shl),
            (CellKind::Shr, BinaryOp::Shr),
            (CellKind::Sshr, BinaryOp::Sshr),
            (CellKind::Eq, BinaryOp::Eq),
            (CellKind::Ne, BinaryOp::Ne),
            (CellKind::Lt, BinaryOp::Lt),
            (CellKind::Le, BinaryOp::Le),
            (CellKind::Gt, BinaryOp::Gt),
            (CellKind::Ge, BinaryOp::Ge),
        ];
        for (kind, op) in two_input {
            let name = format!("c_{}", kind.keyword());
            let reference = b.binary(op, a, bb);
            let out_ty = b.module().expr(reference).ty.clone();
            let y = b.output(name.clone(), out_ty);
            b.cell2(name, kind, a, bb, y);
            outs.push((y, reference));
        }
        let one_input = [
            (CellKind::Not, UnaryOp::Not),
            (CellKind::ReduceAnd, UnaryOp::ReduceAnd),
            (CellKind::ReduceOr, UnaryOp::ReduceOr),
            (CellKind::ReduceXor, UnaryOp::ReduceXor),
        ];
        for (kind, op) in one_input {
            let name = format!("c_{}", kind.keyword());
            let reference = b.unary(op, a);
            let out_ty = b.module().expr(reference).ty.clone();
            let y = b.output(name.clone(), out_ty);
            b.cell(
                name,
                kind,
                vec![(Name::new("a"), a)],
                vec![(Name::new("y"), y)],
            );
            outs.push((y, reference));
        }
        let y = b.output("c_buf", ty.clone());
        b.cell(
            "c_buf",
            CellKind::Buf,
            vec![(Name::new("a"), a)],
            vec![(Name::new("y"), y)],
        );
        outs.push((y, a));
        // mux: s = 1 picks b.
        let reference = b.mux(s, bb, a);
        let y = b.output("c_mux", ty.clone());
        b.cell(
            "c_mux",
            CellKind::Mux,
            vec![
                (Name::new("a"), a),
                (Name::new("b"), bb),
                (Name::new("s"), s),
            ],
            vec![(Name::new("y"), y)],
        );
        outs.push((y, reference));
        // pmux with three one-hot selects over {~a, b, a}; default a.
        let na = b.not(a);
        let cat = b.concat(vec![na, bb, a]);
        let s0 = b.slice(sel, 0, 0);
        let s1 = b.slice(sel, 1, 1);
        let s2 = b.slice(sel, 2, 2);
        let r2 = b.mux(s2, na, a);
        let r1 = b.mux(s1, bb, r2);
        let reference = b.mux(s0, a, r1);
        let y = b.output(
            "c_pmux",
            Type::Bits {
                width: w,
                signed: false,
            },
        );
        b.cell(
            "c_pmux",
            CellKind::Pmux,
            vec![
                (Name::new("a"), a),
                (Name::new("b"), cat),
                (Name::new("s"), sel),
            ],
            vec![(Name::new("y"), y)],
        );
        outs.push((y, reference));
        // lut: 3-input majority.
        let y = b.output("c_lut", Type::bit());
        b.cell(
            "c_lut",
            CellKind::Lut {
                k: 3,
                init: Logic::from_u64(0b1110_1000, 8),
            },
            vec![(Name::new("a"), sel)],
            vec![(Name::new("y"), y)],
        );
        let t01 = b.and(s0, s1);
        let t02 = b.and(s0, s2);
        let t12 = b.and(s1, s2);
        let t = b.or(t01, t02);
        let reference = b.or(t, t12);
        outs.push((y, reference));
        (b.finish(), outs)
    }

    #[test]
    fn cells_match_logic_evaluation() {
        let mut rng = Rng(0xD1B5_4A32_D192_ED03);
        for &w in &WIDTHS {
            for signed in [false, true] {
                let (m, outs) = cell_module(w, signed);
                let blaster =
                    Blaster::new(&m, &BlastOptions::default()).unwrap_or_else(|d| panic!("{d:?}"));
                let mut cnf = CnfBuilder::new();
                let frame = blaster.blast_frame(&mut cnf, None, None);
                let mut solver = cnf.into_solver();
                for _ in 0..4 {
                    let mut env = Vec::new();
                    let mut assumptions = Vec::new();
                    for (slot, sig) in blaster.input_slots().iter().zip(&frame.inputs) {
                        let width = u32::try_from(sig.lits.len()).unwrap();
                        let mut v = rng.logic(width, sig.signed);
                        if slot.name == "sel" {
                            // One-hot or zero, as pmux requires.
                            let k = rng.next() % 4;
                            v = Logic::from_u64(if k == 3 { 0 } else { 1 << k }, 3);
                        }
                        assumptions.extend(fix(&sig.lits, &v));
                        env.push((slot.net, v));
                    }
                    assert_eq!(
                        solver.solve_with_assumptions(&assumptions),
                        SolveResult::Sat
                    );
                    for ((net, e), sig) in outs.iter().zip(&frame.outputs) {
                        let expected = eval_logic(&m, *e, &env);
                        let actual = read(&solver, &sig.lits);
                        assert!(
                            agrees(&expected, &actual),
                            "width {w} signed {signed} cell `{}`: expected {expected} got {actual}",
                            m.nets[*net].name
                        );
                    }
                }
            }
        }
    }

    /// A counter with enable and synchronous reset next to a 4 x 8 memory
    /// with one write port, one clocked and one unclocked read port.
    fn sequential_module() -> Module {
        let mut b = ModuleBuilder::new("seq", span());
        let clk = b.input("clk", Type::bit());
        let rst = b.input("rst", Type::bit());
        let en = b.input("en", Type::bit());
        let we = b.input("we", Type::bit());
        let waddr = b.input("waddr", Type::bits(2));
        let wdata = b.input("wdata", Type::bits(8));
        let q = b.output("q", Type::bits(4));
        let rd = b.output("rd", Type::bits(8));
        let rdq = b.output("rdq", Type::bits(8));
        let inc = b.add_net("inc", Type::bits(4));
        let mem = b.memory("m", Type::bits(8), 4);
        let (qv, one) = (b.net(q), b.const_u64(4, 1));
        let sum = b.add(qv, one);
        b.assign(inc, sum);
        let (clkv, rstv, env, incv) = (b.net(clk), b.net(rst), b.net(en), b.net(inc));
        b.cell(
            "ff",
            CellKind::Dff {
                clk_pos: true,
                has_enable: true,
                reset: Some(Reset {
                    asynchronous: false,
                    active_high: true,
                    value: Logic::from_u64(3, 4),
                }),
            },
            vec![
                (Name::new("clk"), clkv),
                (Name::new("d"), incv),
                (Name::new("en"), env),
                (Name::new("rst"), rstv),
            ],
            vec![(Name::new("q"), q)],
        );
        let (wev, wav, wdv) = (b.net(we), b.net(waddr), b.net(wdata));
        b.cell(
            "wr",
            CellKind::MemWrPort { mem, clocked: true },
            vec![
                (Name::new("addr"), wav),
                (Name::new("data"), wdv),
                (Name::new("en"), wev),
                (Name::new("clk"), clkv),
            ],
            vec![],
        );
        let raddr = b.slice(qv, 1, 0);
        b.cell(
            "rd",
            CellKind::MemRdPort {
                mem,
                clocked: false,
            },
            vec![(Name::new("addr"), raddr)],
            vec![(Name::new("data"), rd)],
        );
        let t = b.const_bit(true);
        b.cell(
            "rdq",
            CellKind::MemRdPort { mem, clocked: true },
            vec![
                (Name::new("addr"), raddr),
                (Name::new("clk"), clkv),
                (Name::new("en"), t),
            ],
            vec![(Name::new("data"), rdq)],
        );
        b.finish()
    }

    #[test]
    fn state_elements_and_memories() {
        let m = sequential_module();
        let blaster =
            Blaster::new(&m, &BlastOptions::default()).unwrap_or_else(|d| panic!("{d:?}"));
        let names: Vec<&str> = blaster
            .state_slots()
            .iter()
            .map(|s| s.name.as_str())
            .collect();
        assert_eq!(names, ["q", "rdq", "m[0]", "m[1]", "m[2]", "m[3]"]);
        assert_eq!(blaster.state_slots()[0].init, Some(Logic::from_u64(3, 4)));
        assert_eq!(blaster.state_slots()[1].init, None);
        assert!(blaster.has_state());
        let mut cnf = CnfBuilder::new();
        let f0 = blaster.blast_frame(&mut cnf, None, None);
        let f1 = blaster.blast_frame(&mut cnf, Some(&f0.next_state), None);
        let f2 = blaster.blast_frame(&mut cnf, Some(&f1.next_state), None);
        let mut solver = cnf.into_solver();
        let input = |f: &BlastedFrame, name: &str| -> Vec<Lit> {
            f.inputs
                .iter()
                .find(|s| s.name == name)
                .unwrap()
                .lits
                .clone()
        };
        let output = |f: &BlastedFrame, name: &str| -> Vec<Lit> {
            f.outputs
                .iter()
                .find(|s| s.name == name)
                .unwrap()
                .lits
                .clone()
        };
        let one = Logic::from_u64(1, 1);
        let zero = Logic::from_u64(0, 1);
        let mut a = Vec::new();
        // Frame 0: q = 5 in the state, reset asserted; write 0xAB at 1.
        a.extend(fix(&f0.state[0].lits, &Logic::from_u64(5, 4)));
        a.extend(fix(&input(&f0, "rst"), &one));
        a.extend(fix(&input(&f0, "en"), &one));
        a.extend(fix(&input(&f0, "we"), &one));
        a.extend(fix(&input(&f0, "waddr"), &Logic::from_u64(1, 2)));
        a.extend(fix(&input(&f0, "wdata"), &Logic::from_u64(0xAB, 8)));
        // Frame 1: count (q = 3 after reset -> 4); no write.
        a.extend(fix(&input(&f1, "rst"), &zero));
        a.extend(fix(&input(&f1, "en"), &one));
        a.extend(fix(&input(&f1, "we"), &zero));
        // Frame 2: hold.
        a.extend(fix(&input(&f2, "rst"), &zero));
        a.extend(fix(&input(&f2, "en"), &zero));
        a.extend(fix(&input(&f2, "we"), &zero));
        assert_eq!(solver.solve_with_assumptions(&a), SolveResult::Sat);
        assert_eq!(read(&solver, &output(&f0, "q")).to_u64(), Some(5));
        assert_eq!(read(&solver, &output(&f1, "q")).to_u64(), Some(3));
        assert_eq!(read(&solver, &output(&f2, "q")).to_u64(), Some(4));
        // The write lands in word 1 from frame 1 on.
        assert_eq!(read(&solver, &f1.state[3].lits).to_u64(), Some(0xAB));
        assert_eq!(read(&solver, &f2.state[3].lits).to_u64(), Some(0xAB));
        // Frame 0 reads word 1 (q[1:0] = 1) asynchronously; the clocked
        // port delivers the same word one frame later.
        let w1 = read(&solver, &f0.state[3].lits);
        assert!(agrees(&w1, &read(&solver, &output(&f0, "rd"))));
        assert!(agrees(&w1, &read(&solver, &output(&f1, "rdq"))));
        // Frame 2 reads word 0 (q = 4), which nothing wrote: it is free.
        let rd2 = output(&f2, "rd");
        let mut b = a.clone();
        b.push(rd2[0]);
        assert_eq!(solver.solve_with_assumptions(&b), SolveResult::Sat);
        b.pop();
        b.push(!rd2[0]);
        assert_eq!(solver.solve_with_assumptions(&b), SolveResult::Sat);
    }

    #[test]
    fn lvalue_slices_concats_and_indices() {
        let mut b = ModuleBuilder::new("lv", span());
        let a = b.input("a", Type::bits(4));
        let i = b.input("i", Type::bits(2));
        let y = b.output("y", Type::bits(8));
        let z = b.output("z", Type::bits(4));
        let w = b.output("w", Type::bits(3));
        let (av, iv) = (b.net(a), b.net(i));
        let na = b.not(av);
        // y[7:4] = ~a, y[3:0] = a through a concatenation target.
        let cat = b.concat(vec![na, av]);
        b.assign(
            Lvalue::Concat(vec![
                Lvalue::Slice {
                    net: y,
                    hi: 7,
                    lo: 4,
                },
                Lvalue::Slice {
                    net: y,
                    hi: 3,
                    lo: 0,
                },
            ]),
            cat,
        );
        // z[i] = a[0]; the other bits are free.
        let a0 = b.slice(av, 0, 0);
        b.assign(Lvalue::Index { net: z, index: iv }, a0);
        // w[2:1] = a[1:0]; bit 0 undriven.
        let a10 = b.slice(av, 1, 0);
        b.assign(
            Lvalue::Slice {
                net: w,
                hi: 2,
                lo: 1,
            },
            a10,
        );
        let m = b.finish();
        let blaster =
            Blaster::new(&m, &BlastOptions::default()).unwrap_or_else(|d| panic!("{d:?}"));
        let mut cnf = CnfBuilder::new();
        let f = blaster.blast_frame(&mut cnf, None, None);
        let mut solver = cnf.into_solver();
        let mut asm = fix(&f.inputs[0].lits, &Logic::from_u64(0b1011, 4));
        asm.extend(fix(&f.inputs[1].lits, &Logic::from_u64(2, 2)));
        // Pin the free bits of z low so the read is deterministic.
        for (k, l) in f.outputs[1].lits.iter().enumerate() {
            if k != 2 {
                asm.push(!*l);
            }
        }
        assert_eq!(solver.solve_with_assumptions(&asm), SolveResult::Sat);
        assert_eq!(
            read(&solver, &f.outputs[0].lits).to_u64(),
            Some(0b0100_1011)
        );
        assert_eq!(read(&solver, &f.outputs[1].lits).to_u64(), Some(0b0100));
        let w = read(&solver, &f.outputs[2].lits);
        assert_eq!(w.slice(2, 1).to_u64(), Some(0b11));
        assert_eq!(
            solver.solve_with_assumptions(&[f.outputs[2].lits[0]]),
            SolveResult::Sat
        );
        assert_eq!(
            solver.solve_with_assumptions(&[!f.outputs[2].lits[0]]),
            SolveResult::Sat
        );
    }

    #[test]
    fn properties_from_attributes_and_options() {
        let mut b = ModuleBuilder::new("props", span());
        let a = b.input("a", Type::bit());
        let p = b.add_net("p", Type::bit());
        b.net_attr(p, "formal_assert", 1);
        let q = b.add_net("q", Type::bit());
        b.net_attr(q, "formal_cover", 1);
        let r = b.add_net("r", Type::bit());
        let av = b.net(a);
        b.assign(p, av);
        b.assign(q, av);
        b.assign(r, av);
        let m = b.finish();
        let opts = BlastOptions {
            properties: vec![(r, PropertyKind::Assume)],
            ..BlastOptions::default()
        };
        let blaster = Blaster::new(&m, &opts).unwrap();
        assert_eq!(
            blaster.properties(),
            [
                (p, PropertyKind::Assert),
                (q, PropertyKind::Cover),
                (r, PropertyKind::Assume)
            ]
        );
        let mut cnf = CnfBuilder::new();
        let f = blaster.blast_frame(&mut cnf, None, None);
        assert_eq!(f.asserts.len(), 1);
        assert_eq!(f.covers.len(), 1);
        assert_eq!(f.assumes.len(), 1);
        assert_eq!(f.asserts[0].name, "p");
        let bad = f.bad(&mut cnf);
        assert_eq!(bad, !f.asserts[0].lit);
    }

    #[test]
    fn rejects_what_it_cannot_model() {
        let mut b = ModuleBuilder::new("bad", span());
        let a = b.input("a", Type::bit());
        let y = b.output("y", Type::bit());
        b.inout("io", Type::bit());
        let av = b.net(a);
        b.cell(
            "l",
            CellKind::Dlatch,
            vec![(Name::new("en"), av), (Name::new("d"), av)],
            vec![(Name::new("q"), y)],
        );
        let z = b.constant(Logic::z(1));
        let zn = b.add_net("zn", Type::bit());
        b.assign(zn, z);
        let m = b.finish();
        let err = Blaster::new(&m, &BlastOptions::default()).unwrap_err();
        let codes: Vec<&str> = err.iter().filter_map(|d| d.code).collect();
        assert!(codes.contains(&"F0003"), "{codes:?}");
        assert!(codes.contains(&"F0004"), "{codes:?}");
        assert!(codes.contains(&"F0009"), "{codes:?}");

        // A combinational loop.
        let mut b = ModuleBuilder::new("loop", span());
        let x = b.add_net("x", Type::bit());
        let xv = b.net(x);
        let nx = b.not(xv);
        b.assign(x, nx);
        let m = b.finish();
        let err = Blaster::new(&m, &BlastOptions::default()).unwrap_err();
        assert!(err.iter().any(|d| d.code == Some("F0013")));

        // A process.
        let mut b = ModuleBuilder::new("proc", span());
        let p = b.process(None, crate::ir::ProcessKind::Comb);
        b.end_process(p);
        let m = b.finish();
        let err = Blaster::new(&m, &BlastOptions::default()).unwrap_err();
        assert!(err.iter().any(|d| d.code == Some("F0001")));

        // A memory over the limit.
        let mut b = ModuleBuilder::new("mem", span());
        b.memory("big", Type::bits(8), 1024);
        let m = b.finish();
        let opts = BlastOptions {
            max_memory_bits: 4096,
            ..BlastOptions::default()
        };
        let err = Blaster::new(&m, &opts).unwrap_err();
        assert!(err.iter().any(|d| d.code == Some("F0008")));
    }

    #[test]
    fn warnings_for_free_nets_and_async_reset() {
        let mut b = ModuleBuilder::new("warn", span());
        let clk = b.input("clk", Type::bit());
        let rst = b.input("rst", Type::bit());
        let q = b.output("q", Type::bit());
        let free = b.add_net("free", Type::bit());
        let (clkv, rstv, fv) = (b.net(clk), b.net(rst), b.net(free));
        b.cell(
            "ff",
            CellKind::Dff {
                clk_pos: true,
                has_enable: false,
                reset: Some(Reset {
                    asynchronous: true,
                    active_high: false,
                    value: Logic::zero(1),
                }),
            },
            vec![
                (Name::new("clk"), clkv),
                (Name::new("d"), fv),
                (Name::new("rst"), rstv),
            ],
            vec![(Name::new("q"), q)],
        );
        let m = b.finish();
        let blaster = Blaster::new(&m, &BlastOptions::default()).unwrap();
        let codes: Vec<&str> = blaster
            .diagnostics()
            .iter()
            .filter_map(|d| d.code)
            .collect();
        assert_eq!(codes, ["F0011", "F0015"]);
        assert_eq!(blaster.input_slots().len(), 3);
        assert!(!blaster.input_slots()[2].is_port);
    }
}
