//! Elaboration: flattening the hierarchical design into simulation tables.
//!
//! The design is walked from the top module. Every instance gets an
//! [`InstanceState`] with a table from its module's `NetId`s to global
//! [`SigId`]s. A child's port net is *aliased* to the parent's signal when
//! the connection is a plain net of the same width, so the connection has
//! no run-time cost; any other connection becomes an implicit [`Driver`]
//! evaluated in the parent's context (an input reads the parent expression
//! into the child's port signal; an output copies the child's port signal
//! into the parent's slice or concatenation). Because instantiation goes
//! top-down and a port net can alias at most one parent net, a plain table
//! suffices where a union-find would otherwise be needed.
//!
//! Drivers are the continuous sources of a signal: assigns, combinational
//! cell outputs and implicit port connections. Each keeps its current
//! contribution per target signal so a wire with several drivers can be
//! re-resolved when any one of them changes.
//!
//! The fanout tables built here (per signal and per memory) list the
//! consumers to schedule on a change: drivers and combinational cells on
//! any change of a net they read, sequential cells and processes on the
//! edges they are sensitive to.

use std::collections::BTreeSet;

use crate::diag::{Diagnostic, Diagnostics};
use crate::ir::walk::{stmt_exprs, walk_block};
use crate::ir::{
    Block, Cell, CellKind, Design, Edge, ExprId, ExprKind, Lvalue, MemoryId, Module, ModuleId,
    ModuleRef, NetId, NetKind, Polarity, PortDir, ProcessKind, Type, expr,
};
use crate::logic::{Bit, Logic};

use super::process::{ProcState, ProcStatus};
use super::sched::Event;
use super::value::{Signal, elem_count, elem_width, flat_width};
use super::{SimOptions, Simulator, Status};

macro_rules! sim_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub(crate) struct $name(pub(crate) u32);

        impl $name {
            /// The index into the table this id addresses.
            pub(crate) fn idx(self) -> usize {
                // `u32` always fits `usize` on supported targets.
                self.0 as usize
            }

            /// The id of position `i`.
            pub(crate) fn at(i: usize) -> Self {
                $name(u32::try_from(i).expect("simulation table exceeds u32"))
            }
        }
    };
}

sim_id!(
    /// An instance in the flattened tree.
    InstId
);
sim_id!(
    /// A signal: the storage behind one or more aliased nets.
    SigId
);
sim_id!(
    /// A memory instance.
    MemId
);
sim_id!(
    /// A process instance.
    ProcId
);
sim_id!(
    /// A continuous driver.
    DriverId
);
sim_id!(
    /// A cell instance.
    CellRef
);

/// One instance of a module in the flattened hierarchy.
pub(crate) struct InstanceState<'d> {
    /// The instance name (the module name for the top).
    pub(crate) name: String,
    /// The dotted hierarchical path from the top.
    pub(crate) path: String,
    /// The module, borrowed from the design.
    pub(crate) m: &'d Module,
    /// Child instances in declaration order.
    pub(crate) children: Vec<InstId>,
    /// Signal of each net, indexed by `NetId::index()`.
    pub(crate) nets: Vec<SigId>,
    /// Memory of each memory, indexed by `MemoryId::index()`.
    pub(crate) mems: Vec<MemId>,
    /// Femtoseconds per time unit of this module.
    pub(crate) unit_fs: u64,
    /// Ticks per time unit of this module.
    pub(crate) unit_ticks: u64,
}

/// A memory instance.
pub(crate) struct MemState {
    /// Hierarchical name.
    pub(crate) name: String,
    /// Width of one element.
    pub(crate) elem_width: u32,
    /// True for an array of strings, whose elements each hold as many
    /// characters as they were given rather than `elem_width` bits.
    pub(crate) strings: bool,
    /// True for an array of two-state elements, which start at zero and
    /// hold a `0` for every `x` or `z` written.
    pub(crate) two_state: bool,
    /// For an array of elements only some of whose bits are two-state:
    /// those bits.
    pub(crate) two_state_bits: Option<Logic>,
    /// True for an array of `real`, whose elements are IEEE 754 doubles.
    pub(crate) real: bool,
    /// Contents; `x` where uninitialised.
    pub(crate) data: Vec<Logic>,
    /// Consumers to schedule on any write.
    pub(crate) fanout: Vec<Consumer>,
}

/// Something that reacts to a change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Consumer {
    /// Re-evaluate a driver.
    Driver(DriverId),
    /// Trigger a sequential cell.
    Cell(CellRef),
    /// Trigger a process.
    Proc(ProcId),
}

/// A fanout entry: a consumer and the change it reacts to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Fanout {
    /// What to schedule.
    pub(crate) consumer: Consumer,
    /// Which change schedules it (`Any` for any change).
    pub(crate) polarity: Polarity,
}

/// An assignment target with its signals resolved but its dynamic indices
/// still as expressions, evaluated at each assignment.
#[derive(Clone, Debug)]
pub(crate) enum Target {
    /// Bits `[lo, lo + width)` of a signal.
    Sig {
        /// The signal.
        sig: SigId,
        /// Lowest bit.
        lo: u32,
        /// Bit count.
        width: u32,
    },
    /// Element `index` of a signal, each element `elem` bits wide.
    Index {
        /// The signal.
        sig: SigId,
        /// The index expression, evaluated in the owning instance.
        index: ExprId,
        /// Element width.
        elem: u32,
        /// Number of elements.
        count: u64,
    },
    /// A memory element.
    Mem {
        /// The memory.
        mem: MemId,
        /// The address expression.
        addr: ExprId,
    },
    /// Some bits of a memory element, as [`Lvalue::MemPart`] has them.
    MemPart {
        /// The memory.
        mem: MemId,
        /// The address expression.
        addr: ExprId,
        /// The offset of the value's bit 0 in the element.
        offset: ExprId,
        /// The number of bits written.
        width: u32,
    },
    /// A concatenation, most significant part first.
    Concat(Vec<Target>),
}

impl Target {
    /// Every signal the target may write.
    pub(crate) fn sigs(&self, out: &mut Vec<SigId>) {
        match self {
            Target::Sig { sig, .. } | Target::Index { sig, .. } => out.push(*sig),
            Target::Mem { .. } | Target::MemPart { .. } => {}
            Target::Concat(parts) => parts.iter().for_each(|p| p.sigs(out)),
        }
    }

    /// Every expression evaluated when the target is resolved.
    pub(crate) fn exprs(&self, out: &mut Vec<ExprId>) {
        match self {
            Target::Sig { .. } => {}
            Target::Index { index, .. } => out.push(*index),
            Target::Mem { addr, .. } => out.push(*addr),
            Target::MemPart { addr, offset, .. } => {
                out.push(*addr);
                out.push(*offset);
            }
            Target::Concat(parts) => parts.iter().for_each(|p| p.exprs(out)),
        }
    }
}

/// What a driver computes.
#[derive(Clone, Copy, Debug)]
pub(crate) enum DriverSource {
    /// An expression evaluated in the driver's instance.
    Expr(ExprId),
    /// A copy of another signal (implicit output port connection).
    Sig(SigId),
    /// The function of a combinational cell.
    Cell(CellRef),
    /// A `force` of `sig` to an expression, which the signal follows while
    /// it is forced (IEEE 1364-2005 §9.3.2): `active` until a `release`.
    Force {
        /// The expression forced.
        value: ExprId,
        /// The signal forced.
        sig: SigId,
        /// False once released, until forced again.
        active: bool,
    },
    /// Bits `lo .. lo + width` of what every driver of `sig` but `except`
    /// puts on it: one side of an `inout` port joined to a part of a
    /// net, which has to see what the other side drives without seeing
    /// its own drive come back. See `join_inout`.
    Others {
        /// The signal whose drivers are resolved.
        sig: SigId,
        /// Its lowest bit taken.
        lo: u32,
        /// How many bits.
        width: u32,
        /// The driver left out: the other half of the join.
        except: DriverId,
    },
}

/// A continuous driver of one or more signals.
pub(crate) struct Driver {
    /// The instance whose expressions the driver evaluates.
    pub(crate) inst: InstId,
    /// What it computes.
    pub(crate) source: DriverSource,
    /// Where the value goes.
    pub(crate) target: Target,
    /// Transport delay in ticks.
    pub(crate) delay: u64,
    /// The current contribution to each wire it drives.
    pub(crate) contrib: Vec<(SigId, Logic)>,
    /// True while an evaluation is queued in the active region.
    pub(crate) scheduled: bool,
    /// A weak driver (`ir::WEAK_ATTR`), which a wire takes only where its
    /// other drivers leave it at `z`.
    pub(crate) weak: bool,
    /// The fall and turn-off delays in ticks, when they are not `delay`
    /// (`ir::FALL_DELAY_ATTR`, `ir::TURNOFF_DELAY_ATTR`).
    pub(crate) fall: Option<u64>,
    pub(crate) turnoff: Option<u64>,
}

/// A cell instance.
pub(crate) struct CellState<'d> {
    /// The owning instance.
    pub(crate) inst: InstId,
    /// The cell.
    pub(crate) cell: &'d Cell,
    /// The last sampled clock (or enable) for edge detection.
    pub(crate) last_clk: Bit,
    /// True while an evaluation is queued.
    pub(crate) scheduled: bool,
}

/// Collects the nets and memories an expression reads, recursively.
pub(crate) fn expr_reads(
    m: &Module,
    e: ExprId,
    nets: &mut BTreeSet<NetId>,
    mems: &mut BTreeSet<MemoryId>,
) {
    let Some(node) = m.exprs.get(e) else {
        return;
    };
    match &node.kind {
        ExprKind::Net(n) => {
            nets.insert(*n);
        }
        ExprKind::MemRead { mem, .. } => {
            mems.insert(*mem);
        }
        _ => {}
    }
    for op in expr::operands(&node.kind) {
        expr_reads(m, op, nets, mems);
    }
}

/// Collects everything a block reads: every expression of every statement,
/// including lvalue indices.
pub(crate) fn block_reads(
    m: &Module,
    block: &Block,
    nets: &mut BTreeSet<NetId>,
    mems: &mut BTreeSet<MemoryId>,
) {
    walk_block(block, &mut |stmt| {
        stmt_exprs(stmt, &mut |e| expr_reads(m, e, nets, mems));
    });
}

/// Ticks for a delay given in femtoseconds, rounded to nearest and at
/// least one tick for a non-zero delay.
pub(crate) fn fs_to_ticks(fs: u64, precision_fs: u64) -> u64 {
    if fs == 0 {
        return 0;
    }
    let ticks = (fs + precision_fs / 2) / precision_fs;
    ticks.max(1)
}

impl<'d> Simulator<'d> {
    /// Builds a simulator: selects the top, flattens the hierarchy, builds
    /// the tables and queues the time-0 work.
    pub(crate) fn elaborate(
        design: &'d Design,
        options: SimOptions,
    ) -> Result<Simulator<'d>, Diagnostics> {
        let mut diags = Diagnostics::new();
        let top = select_top(design, &options, &mut diags)?;
        let precision_fs = design
            .modules
            .values()
            .filter_map(|m| m.timescale.map(|t| t.precision.to_fs()))
            .filter(|&fs| fs > 0)
            .min()
            .unwrap_or(1_000);
        let options_seed = options.seed;
        let rng = if options.seed == 0 {
            0x9E37_79B9_7F4A_7C15
        } else {
            options.seed
        };
        let mut sim = Simulator {
            design,
            options,
            precision_fs,
            instances: Vec::new(),
            signals: Vec::new(),
            sig_fanout: Vec::new(),
            sig_drivers: Vec::new(),
            sig_others: Vec::new(),
            sig_waiters: Vec::new(),
            sig_callbacks: Vec::new(),
            callbacks: Vec::new(),
            memories: Vec::new(),
            drivers: Vec::new(),
            cells: Vec::new(),
            procs: Vec::new(),
            now: 0,
            active: Default::default(),
            inactive: Default::default(),
            nba: Vec::new(),
            timed: Default::default(),
            strobes: Vec::new(),
            monitor: None,
            status: Status::Running,
            finals_run: false,
            output: String::new(),
            messages: Diagnostics::new(),
            vcd: None,
            rng,
            // The low 32 bits, as a C `long` seed would hold them.
            #[allow(clippy::cast_possible_truncation)]
            random_seed: options_seed as i32,
            time_format: super::sys::TimeFormat::for_precision(precision_fs),
            warned_calls: Vec::new(),
            written: std::collections::BTreeMap::new(),
            assertions: Vec::new(),
            assert_watch: Vec::new(),
            assert_sample: Vec::new(),
            assert_clocks: Vec::new(),
            coverage: None,
            breaks: Vec::new(),
            break_hits: Vec::new(),
            next_break: 0,
            vhdl: Default::default(),
            inertial: Default::default(),
        };
        let top_name = design.modules[top].name.to_string();
        let mut chain = Vec::new();
        sim.instantiate(top, top_name, None, Vec::new(), &mut chain, &mut diags);
        sim.bind_hier_refs(&mut diags);
        for i in 0..sim.instances.len() {
            sim.elaborate_contents(InstId::at(i), &mut diags);
        }
        if diags.has_errors() {
            return Err(diags);
        }
        sim.messages = diags;
        if sim.options.coverage {
            sim.init_coverage();
        }
        sim.vhdl_prepare();
        sim.schedule_time_zero();
        Ok(sim)
    }

    /// Gives each hierarchical reference's net the signal of the net it
    /// names, in the instance its path leads to (see
    /// [`crate::ir::HierRef`]), so reading, waiting on and forcing the
    /// reference act on the target. Runs once every instance exists and
    /// before any fanout is built from the nets' signals.
    fn bind_hier_refs(&mut self, diags: &mut Diagnostics) {
        for i in 0..self.instances.len() {
            let m = self.instances[i].m;
            for href in &m.hier_refs {
                let mut at = i;
                let mut missing = None;
                for name in &href.path {
                    match self.instances[at]
                        .children
                        .iter()
                        .find(|c| self.instances[c.idx()].name == name.as_str())
                    {
                        Some(child) => at = child.idx(),
                        None => {
                            missing = Some(format!("no instance `{name}`"));
                            break;
                        }
                    }
                }
                let target = self.instances[at]
                    .m
                    .nets
                    .iter()
                    .find(|(_, n)| n.name == href.net)
                    .map(|(id, _)| id.index());
                match (missing, target) {
                    (None, Some(index)) => {
                        let sig = self.instances[at].nets[index];
                        if let Some(slot) = self.instances[i].nets.get_mut(href.local.index()) {
                            *slot = sig;
                        }
                    }
                    (missing, _) => {
                        let why = missing.unwrap_or_else(|| format!("no net `{}`", href.net));
                        diags.push(
                            Diagnostic::error(format!(
                                "the hierarchical reference `{}` in `{}` leads nowhere: {why}",
                                m.nets[href.local].name, self.instances[i].path
                            ))
                            .with_span(m.nets[href.local].span),
                        );
                    }
                }
            }
        }
    }

    /// Creates the instance of `module` and, recursively, its children.
    ///
    /// `aliases` maps the module's nets to signals already created by the
    /// parent (port connections to plain nets); `chain` is the stack of
    /// modules being instantiated, for recursion detection.
    fn instantiate(
        &mut self,
        module: ModuleId,
        name: String,
        parent: Option<InstId>,
        aliases: Vec<Option<SigId>>,
        chain: &mut Vec<ModuleId>,
        diags: &mut Diagnostics,
    ) -> InstId {
        let design = self.design;
        let m = &design.modules[module];
        let path = match parent {
            Some(p) => format!("{}.{}", self.instances[p.idx()].path, name),
            None => name.clone(),
        };
        let id = InstId::at(self.instances.len());
        let unit_fs = m
            .timescale
            .map(|t| t.unit.to_fs())
            .filter(|&fs| fs > 0)
            .unwrap_or(1_000_000);
        let unit_ticks = fs_to_ticks(unit_fs, self.precision_fs);
        let mut nets = Vec::with_capacity(m.nets.len());
        for (nid, net) in m.nets.iter() {
            let sig = match aliases.get(nid.index()).copied().flatten() {
                // A variable a parent's wire stands for starts as a variable
                // does, at `x`: an output `reg q` of a primitive read through
                // the wire it drives used to read `z` until first written.
                Some(sig) => {
                    let s = &mut self.signals[sig.idx()];
                    if net.kind != NetKind::Wire
                        && !s.two_state
                        && s.value.width() > 0
                        && s.value == Logic::z(s.value.width())
                    {
                        s.value = Logic::x(s.value.width()).with_signed(s.value.is_signed());
                    }
                    sig
                }
                None => {
                    let full = format!("{path}.{}", net.name);
                    let sig = self.new_signal(full, &net.ty, net.kind, m.name.clone(), net.span);
                    self.signals[sig.idx()].resolution = match net.attrs.get(crate::ir::WIRED_ATTR)
                    {
                        Some(crate::ir::AttrValue::String(w)) if w == "and" => {
                            super::value::Resolution::And
                        }
                        Some(crate::ir::AttrValue::String(w)) if w == "or" => {
                            super::value::Resolution::Or
                        }
                        _ => super::value::Resolution::Wire,
                    };
                    match net.attrs.get(crate::ir::TWO_STATE_ATTR) {
                        Some(crate::ir::AttrValue::Const(mask)) => {
                            let s = &mut self.signals[sig.idx()];
                            s.value = s.value.two_state_where(mask);
                            s.two_state_bits = Some(mask.clone());
                        }
                        Some(_) => {
                            let s = &mut self.signals[sig.idx()];
                            s.two_state = true;
                            s.value = s.value.two_state();
                        }
                        None => {}
                    }
                    sig
                }
            };
            nets.push(sig);
        }
        let mut mems = Vec::with_capacity(m.memories.len());
        for (_, mem) in m.memories.iter() {
            let width = flat_width(&mem.elem);
            let size = usize::try_from(mem.size).unwrap_or(usize::MAX);
            let strings = mem.elem == crate::ir::Type::String;
            let (two_state, two_state_bits) = match mem.attrs.get(crate::ir::TWO_STATE_ATTR) {
                Some(crate::ir::AttrValue::Const(mask)) => (false, Some(mask.clone())),
                Some(_) => (true, None),
                None => (false, None),
            };
            let blank = if strings {
                Logic::zero(0)
            } else if two_state || mem.elem == crate::ir::Type::Real {
                // A real starts at 0.0, whose encoding is all zeros.
                Logic::zero(width)
            } else if let Some(mask) = &two_state_bits {
                Logic::x(width).two_state_where(mask)
            } else {
                Logic::x(width)
            };
            let mut data = vec![blank; size];
            if let Some(init) = &mem.init {
                for (slot, value) in data.iter_mut().zip(init) {
                    *slot = value.resize(width).with_signed(mem.elem.is_signed());
                }
            }
            let mid = MemId::at(self.memories.len());
            self.memories.push(MemState {
                name: format!("{path}.{}", mem.name),
                elem_width: width,
                strings,
                two_state,
                two_state_bits,
                real: mem.elem == crate::ir::Type::Real,
                data,
                fanout: Vec::new(),
            });
            mems.push(mid);
        }
        self.instances.push(InstanceState {
            name,
            path,
            m,
            children: Vec::new(),
            nets,
            mems,
            unit_fs,
            unit_ticks,
        });
        chain.push(module);
        for (_, inst) in m.instances.iter() {
            let child_module = match &inst.module {
                ModuleRef::Resolved(cm) => *cm,
                ModuleRef::Unresolved(n) => {
                    // A black box the design *declares* is a resolved
                    // reference to a `Module` with `blackbox` set, and is
                    // warned about below. Landing here means nothing
                    // declares the module at all, which would simulate as
                    // a box whose outputs stay `x`: a run whose result is
                    // a fiction, and the sort of fiction a testbench
                    // reports as a failure of the design. Refuse instead.
                    if design.module_by_name(n.as_str()).is_none() {
                        diags.push(
                            Diagnostic::error(format!(
                                "no module named `{n}` is defined, so instance `{}` cannot be \
                                 simulated",
                                inst.name
                            ))
                            .with_span(inst.span)
                            .with_note(format!(
                                "Reticle has no library search path: add the file that defines \
                                 `{n}` to this run"
                            ))
                            .with_note(
                                "a black box the design declares does simulate, as a box whose \
                                 outputs stay undriven; a module nothing declares does not",
                            ),
                        );
                        continue;
                    }
                    diags.push(
                        Diagnostic::warning(format!(
                            "instance `{}` refers to unbound module `{n}`; its outputs stay undriven",
                            inst.name
                        ))
                        .with_span(inst.span)
                        .with_note(
                            "the design holds a module of that name; `Design::resolve_instances` \
                             binds the reference to it",
                        ),
                    );
                    continue;
                }
            };
            let Some(cm) = design.modules.get(child_module) else {
                diags.push(
                    Diagnostic::error(format!(
                        "instance `{}` refers to a module id that is not in the design",
                        inst.name
                    ))
                    .with_span(inst.span),
                );
                continue;
            };
            if chain.contains(&child_module) {
                diags.push(
                    Diagnostic::error(format!(
                        "module `{}` instantiates itself through `{}`",
                        cm.name, inst.name
                    ))
                    .with_span(inst.span),
                );
                continue;
            }
            if cm.blackbox {
                diags.push(
                    Diagnostic::warning(format!(
                        "instance `{}` of black box `{}` has no behaviour; its outputs stay undriven",
                        inst.name, cm.name
                    ))
                    .with_span(inst.span),
                );
            }
            let mut child_aliases: Vec<Option<SigId>> = vec![None; cm.nets.len()];
            let mut implicit = Vec::new();
            for (pname, e) in &inst.connections {
                let Some(port) = cm.port(pname.as_str()) else {
                    diags.push(
                        Diagnostic::warning(format!(
                            "instance `{}` connects port `{pname}`, which `{}` does not have",
                            inst.name, cm.name
                        ))
                        .with_span(inst.span),
                    );
                    continue;
                };
                let Some(child_net) = cm.nets.get(port.net) else {
                    continue;
                };
                let child_w = flat_width(&child_net.ty);
                let expr_node = &m.exprs[*e];
                if let ExprKind::Net(n) = expr_node.kind
                    && let Some(parent_net) = m.nets.get(n)
                    && flat_width(&parent_net.ty) == child_w
                    && child_aliases[port.net.index()].is_none()
                {
                    child_aliases[port.net.index()] =
                        Some(self.instances[id.idx()].nets[n.index()]);
                } else {
                    if flat_width(&expr_node.ty) != child_w {
                        diags.push(
                            Diagnostic::warning(format!(
                                "port `{pname}` of instance `{}` is {child_w} bits but its connection is {} bits",
                                inst.name,
                                flat_width(&expr_node.ty)
                            ))
                            .with_span(expr_node.span),
                        );
                    }
                    implicit.push((port.dir, port.net, *e, expr_node.span));
                }
            }
            let child = self.instantiate(
                child_module,
                inst.name.to_string(),
                Some(id),
                child_aliases,
                chain,
                diags,
            );
            self.instances[id.idx()].children.push(child);
            for (dir, pnet, e, span) in implicit {
                let child_sig = self.instances[child.idx()].nets[pnet.index()];
                let width = self.signals[child_sig.idx()].width();
                match dir {
                    PortDir::In => self.drivers.push(Driver {
                        inst: id,
                        source: DriverSource::Expr(e),
                        target: Target::Sig {
                            sig: child_sig,
                            lo: 0,
                            width,
                        },
                        delay: 0,
                        contrib: Vec::new(),
                        scheduled: false,
                        weak: false,
                        fall: None,
                        turnoff: None,
                    }),
                    PortDir::InOut if self.join_inout(id, child_sig, width, e) => {}
                    PortDir::Out | PortDir::InOut => match self.target_from_expr(id, e) {
                        Some(target) => self.drivers.push(Driver {
                            inst: id,
                            source: DriverSource::Sig(child_sig),
                            target,
                            delay: 0,
                            contrib: Vec::new(),
                            scheduled: false,
                            weak: false,
                            fall: None,
                            turnoff: None,
                        }),
                        None => diags.push(
                            Diagnostic::warning(format!(
                                "output port of instance `{}` is connected to an expression that is not a net, slice or concatenation; the connection is ignored",
                                inst.name
                            ))
                            .with_span(span),
                        ),
                    },
                }
            }
        }
        chain.pop();
        id
    }

    /// Joins an `inout` port of a child, `child` of `width` bits, to the
    /// part of a net of `inst` that `e` selects, both ways, and returns
    /// whether it did.
    ///
    /// A port connected to a whole net of its own width shares the
    /// parent's signal and needs nothing. One connected to a part of a net
    /// — `.IO(bus[3])`, which is how every buffer of a mapped netlist's
    /// bidirectional bus is connected — has a signal of its own, and used
    /// to get only a driver from it into the parent: the child's drive
    /// went out, and nothing driven from outside ever came in, so a
    /// buffer read back `z` from a bus a testbench was driving. A pair of
    /// copies cannot fix that, because each would carry the other's last
    /// value back and hold it after the drive is released. Each side here
    /// is driven with what every *other* driver of the far side puts on
    /// it, which is the resolution a `tran` makes. A connection that is
    /// not one part of one net keeps the one-way driver.
    fn join_inout(&mut self, inst: InstId, child: SigId, width: u32, e: ExprId) -> bool {
        let Some(Target::Sig {
            sig,
            lo,
            width: part,
        }) = self.target_from_expr(inst, e)
        else {
            return false;
        };
        if part != width || sig == child {
            return false;
        }
        let up = DriverId::at(self.drivers.len());
        let down = DriverId::at(self.drivers.len() + 1);
        self.drivers.push(Driver {
            inst,
            source: DriverSource::Others {
                sig: child,
                lo: 0,
                width,
                except: down,
            },
            target: Target::Sig { sig, lo, width },
            delay: 0,
            contrib: Vec::new(),
            scheduled: false,
            weak: false,
            fall: None,
            turnoff: None,
        });
        self.drivers.push(Driver {
            inst,
            source: DriverSource::Others {
                sig,
                lo,
                width,
                except: up,
            },
            target: Target::Sig {
                sig: child,
                lo: 0,
                width,
            },
            delay: 0,
            contrib: Vec::new(),
            scheduled: false,
            weak: false,
            fall: None,
            turnoff: None,
        });
        true
    }

    /// Allocates a signal. Wires start at `z` (undriven), registers and
    /// variables at `x`.
    fn new_signal(
        &mut self,
        name: String,
        ty: &Type,
        kind: NetKind,
        module: crate::ir::Name,
        span: crate::source::Span,
    ) -> SigId {
        let width = flat_width(ty);
        let value = match kind {
            NetKind::Wire => Logic::z(width),
            NetKind::Reg | NetKind::Variable => Logic::x(width),
        };
        let id = SigId::at(self.signals.len());
        self.signals.push(Signal {
            name,
            ty: ty.clone(),
            module,
            span,
            kind,
            value: value.with_signed(ty.is_signed()),
            forced: None,
            two_state: false,
            two_state_bits: None,
            resolution: super::value::Resolution::Wire,
        });
        self.sig_fanout.push(Vec::new());
        self.sig_drivers.push(Vec::new());
        self.sig_others.push(Vec::new());
        self.sig_waiters.push(Vec::new());
        self.sig_callbacks.push(Vec::new());
        id
    }

    /// The target for an lvalue of instance `inst`.
    pub(crate) fn target_from_lvalue(&self, inst: InstId, lv: &Lvalue) -> Target {
        let state = &self.instances[inst.idx()];
        let m = state.m;
        match lv {
            Lvalue::Net(n) => {
                let sig = state.nets[n.index()];
                Target::Sig {
                    sig,
                    lo: 0,
                    width: self.signals[sig.idx()].width(),
                }
            }
            Lvalue::Slice { net, hi, lo } => {
                let ew = elem_width(&m.nets[*net].ty);
                Target::Sig {
                    sig: state.nets[net.index()],
                    lo: lo.saturating_mul(ew),
                    width: hi.saturating_sub(*lo).saturating_add(1).saturating_mul(ew),
                }
            }
            Lvalue::Index { net, index } => {
                let ty = &m.nets[*net].ty;
                Target::Index {
                    sig: state.nets[net.index()],
                    index: *index,
                    elem: elem_width(ty),
                    count: elem_count(ty),
                }
            }
            Lvalue::Concat(parts) => Target::Concat(
                parts
                    .iter()
                    .map(|p| self.target_from_lvalue(inst, p))
                    .collect(),
            ),
            Lvalue::MemElem { mem, addr } => Target::Mem {
                mem: state.mems[mem.index()],
                addr: *addr,
            },
            Lvalue::MemPart {
                mem,
                addr,
                offset,
                width,
            } => Target::MemPart {
                mem: state.mems[mem.index()],
                addr: *addr,
                offset: *offset,
                width: *width,
            },
        }
    }

    /// The target for an expression used as a connection sink: a net, a
    /// constant slice or index of a net, or a concatenation of those.
    fn target_from_expr(&self, inst: InstId, e: ExprId) -> Option<Target> {
        let state = &self.instances[inst.idx()];
        let m = state.m;
        let node = m.exprs.get(e)?;
        match &node.kind {
            ExprKind::Net(n) => Some(self.target_from_lvalue(inst, &Lvalue::Net(*n))),
            ExprKind::Slice { base, hi, lo } => {
                let net = m.exprs.get(*base)?.as_net()?;
                Some(self.target_from_lvalue(
                    inst,
                    &Lvalue::Slice {
                        net,
                        hi: *hi,
                        lo: *lo,
                    },
                ))
            }
            ExprKind::Index { base, index } => {
                let net = m.exprs.get(*base)?.as_net()?;
                Some(self.target_from_lvalue(inst, &Lvalue::Index { net, index: *index }))
            }
            ExprKind::Concat(parts) => {
                let parts: Option<Vec<Target>> = parts
                    .iter()
                    .map(|p| self.target_from_expr(inst, *p))
                    .collect();
                Some(Target::Concat(parts?))
            }
            _ => None,
        }
    }

    /// Ticks for an IR delay.
    pub(crate) fn delay_ticks(&self, delay: crate::ir::Delay) -> u64 {
        fs_to_ticks(delay.to_fs(), self.precision_fs)
    }

    /// Adds a fanout entry.
    fn add_fanout(&mut self, sig: SigId, consumer: Consumer, polarity: Polarity) {
        let entry = Fanout { consumer, polarity };
        let list = &mut self.sig_fanout[sig.idx()];
        if !list.contains(&entry) {
            list.push(entry);
        }
    }

    /// Registers `consumer` on every net and memory `e` reads.
    /// Wakes the `force` driver `d` on a change of what `value` reads.
    pub(crate) fn fanout_force(&mut self, inst: InstId, value: ExprId, d: DriverId) {
        self.fanout_expr(inst, value, Consumer::Driver(d));
    }

    fn fanout_expr(&mut self, inst: InstId, e: ExprId, consumer: Consumer) {
        let m = self.instances[inst.idx()].m;
        let mut nets = BTreeSet::new();
        let mut mems = BTreeSet::new();
        expr_reads(m, e, &mut nets, &mut mems);
        self.fanout_sets(inst, &nets, &mems, consumer, Polarity::Any);
    }

    /// Registers `consumer` on the given nets (with `polarity`) and
    /// memories of `inst`.
    fn fanout_sets(
        &mut self,
        inst: InstId,
        nets: &BTreeSet<NetId>,
        mems: &BTreeSet<MemoryId>,
        consumer: Consumer,
        polarity: Polarity,
    ) {
        for n in nets {
            let sig = self.instances[inst.idx()].nets[n.index()];
            self.add_fanout(sig, consumer, polarity);
        }
        for mem in mems {
            let mid = self.instances[inst.idx()].mems[mem.index()];
            let list = &mut self.memories[mid.idx()].fanout;
            if !list.contains(&consumer) {
                list.push(consumer);
            }
        }
    }

    /// Builds the drivers, cells and processes of one instance.
    fn elaborate_contents(&mut self, inst: InstId, diags: &mut Diagnostics) {
        let m = self.instances[inst.idx()].m;
        for assign in &m.assigns {
            let target = self.target_from_lvalue(inst, &assign.target);
            let delay = assign.delay.map_or(0, |d| self.delay_ticks(d));
            let other = |key: &str| {
                assign
                    .attrs
                    .get(key)
                    .and_then(crate::ir::AttrValue::as_int)
                    .and_then(|fs| u64::try_from(fs).ok())
                    .map(|fs| fs_to_ticks(fs, self.precision_fs))
            };
            let (fall, turnoff) = (
                other(crate::ir::FALL_DELAY_ATTR),
                other(crate::ir::TURNOFF_DELAY_ATTR),
            );
            self.drivers.push(Driver {
                inst,
                source: DriverSource::Expr(assign.value),
                target,
                delay,
                contrib: Vec::new(),
                scheduled: false,
                weak: assign.attrs.is_set(crate::ir::WEAK_ATTR),
                fall,
                turnoff,
            });
            if assign.delay.is_some_and(|d| d.inertial) {
                self.inertial
                    .mark_driver(DriverId::at(self.drivers.len() - 1));
            }
        }
        for (_, cell) in m.cells.iter() {
            self.elaborate_cell(inst, cell, diags);
        }
        for (pid, process) in m.processes.iter() {
            let name = match &process.name {
                Some(n) => format!("{}.{n}", self.instances[inst.idx()].path),
                None => format!("{}.{}", self.instances[inst.idx()].path, pid),
            };
            let id = ProcId::at(self.procs.len());
            self.procs.push(ProcState::new(inst, name, process));
            let mut nets = BTreeSet::new();
            let mut mems = BTreeSet::new();
            match &process.kind {
                ProcessKind::Comb => {
                    block_reads(m, &process.body, &mut nets, &mut mems);
                    self.fanout_sets(inst, &nets, &mems, Consumer::Proc(id), Polarity::Any);
                }
                ProcessKind::Sensitive(list) => {
                    nets.extend(list.iter().copied());
                    self.fanout_sets(inst, &nets, &mems, Consumer::Proc(id), Polarity::Any);
                }
                ProcessKind::Sequential { clocks, resets } => {
                    for Edge { net, polarity } in clocks.iter().chain(resets) {
                        let sig = self.instances[inst.idx()].nets[net.index()];
                        self.add_fanout(sig, Consumer::Proc(id), *polarity);
                    }
                }
                ProcessKind::Initial | ProcessKind::Free => {}
            }
        }
        // Drivers created for this instance (assigns, cells) and for its
        // children's ports all evaluate in `inst`; register their reads.
        for d in 0..self.drivers.len() {
            let did = DriverId::at(d);
            if self.drivers[d].inst != inst {
                continue;
            }
            let source = self.drivers[d].source;
            let mut target_exprs = Vec::new();
            self.drivers[d].target.exprs(&mut target_exprs);
            let mut sigs = Vec::new();
            self.drivers[d].target.sigs(&mut sigs);
            for sig in sigs {
                if !self.sig_drivers[sig.idx()].contains(&did) {
                    self.sig_drivers[sig.idx()].push(did);
                }
            }
            for e in target_exprs {
                self.fanout_expr(inst, e, Consumer::Driver(did));
            }
            match source {
                DriverSource::Expr(e) | DriverSource::Force { value: e, .. } => {
                    self.fanout_expr(inst, e, Consumer::Driver(did));
                }
                DriverSource::Sig(sig) => {
                    self.add_fanout(sig, Consumer::Driver(did), Polarity::Any)
                }
                DriverSource::Others { sig, .. } => {
                    // A change of what one driver contributes need not
                    // change the resolved value, and this source can change
                    // all the same: `apply_driver` schedules it on every
                    // contribution, and a register's value on a write.
                    self.add_fanout(sig, Consumer::Driver(did), Polarity::Any);
                    if !self.sig_others[sig.idx()].contains(&did) {
                        self.sig_others[sig.idx()].push(did);
                    }
                }
                DriverSource::Cell(c) => {
                    let cell = self.cells[c.idx()].cell;
                    for (_, e) in &cell.inputs {
                        self.fanout_expr(inst, *e, Consumer::Driver(did));
                    }
                    if let CellKind::MemRdPort { mem, .. } = &cell.kind {
                        let mid = self.instances[inst.idx()].mems[mem.index()];
                        let list = &mut self.memories[mid.idx()].fanout;
                        if !list.contains(&Consumer::Driver(did)) {
                            list.push(Consumer::Driver(did));
                        }
                    }
                }
            }
        }
    }

    /// Creates the state (and, for combinational kinds, the driver) of a
    /// cell and registers its triggers.
    fn elaborate_cell(&mut self, inst: InstId, cell: &'d Cell, diags: &mut Diagnostics) {
        let cref = CellRef::at(self.cells.len());
        self.cells.push(CellState {
            inst,
            cell,
            last_clk: Bit::X,
            scheduled: false,
        });
        let output_target = |sim: &Self, port: &str| -> Option<Target> {
            let net = cell.output(port)?;
            Some(sim.target_from_lvalue(inst, &Lvalue::Net(net)))
        };
        let push_driver = |sim: &mut Self, target: Target| {
            sim.drivers.push(Driver {
                inst,
                source: DriverSource::Cell(cref),
                target,
                delay: 0,
                contrib: Vec::new(),
                scheduled: false,
                weak: false,
                fall: None,
                turnoff: None,
            });
        };
        match &cell.kind {
            CellKind::Dff { reset, .. } => {
                if let Some(e) = cell.input("clk") {
                    self.fanout_expr(inst, e, Consumer::Cell(cref));
                }
                if reset.as_ref().is_some_and(|r| r.asynchronous)
                    && let Some(e) = cell.input("rst")
                {
                    self.fanout_expr(inst, e, Consumer::Cell(cref));
                }
            }
            CellKind::MemRdPort { clocked: true, .. }
            | CellKind::MemWrPort { clocked: true, .. } => {
                if let Some(e) = cell.input("clk") {
                    self.fanout_expr(inst, e, Consumer::Cell(cref));
                }
            }
            CellKind::MemWrPort { clocked: false, .. } => {
                for (_, e) in &cell.inputs {
                    self.fanout_expr(inst, *e, Consumer::Cell(cref));
                }
            }
            CellKind::Blackbox(name) => {
                diags.push(
                    Diagnostic::warning(format!(
                        "cell `{}` is black box `{name}`; its outputs stay undriven",
                        cell.name
                    ))
                    .with_span(cell.span),
                );
            }
            CellKind::Dlatch => {
                if let Some(t) = output_target(self, "q") {
                    push_driver(self, t);
                }
            }
            CellKind::MemRdPort { clocked: false, .. } => {
                if let Some(t) = output_target(self, "data") {
                    push_driver(self, t);
                }
            }
            _ => {
                if let Some(t) = output_target(self, "y") {
                    push_driver(self, t);
                } else {
                    diags.push(
                        Diagnostic::warning(format!(
                            "cell `{}` has no output `y`; it is not simulated",
                            cell.name
                        ))
                        .with_span(cell.span),
                    );
                }
            }
        }
    }

    /// Queues the time-0 work: every driver, then every process that runs
    /// without a trigger.
    fn schedule_time_zero(&mut self) {
        // A wire something drives reads `x` until its drivers' values reach
        // it, which for a delayed one is not at time zero (IEEE 1364-2005
        // §6.1.3: a continuous assignment's driver starts unknown). It used
        // to read `z`, as a wire nothing drives does.
        for (s, drivers) in self.sig_drivers.iter().enumerate() {
            let sig = &mut self.signals[s];
            if sig.kind == NetKind::Wire && !drivers.is_empty() && sig.ty.is_bits() {
                let w = sig.value.width();
                sig.value = Logic::x(w).with_signed(sig.value.is_signed());
            }
        }
        for d in 0..self.drivers.len() {
            self.drivers[d].scheduled = true;
            self.active.push_back(Event::Driver(DriverId::at(d)));
        }
        // A SystemVerilog variable's initialiser sets the variable before
        // anything runs, and without an event (`ir::INITIALISER_ATTR`).
        for p in 0..self.procs.len() {
            if self.procs[p]
                .process
                .attrs
                .is_set(crate::ir::INITIALISER_ATTR)
                && self.initialise_silently(ProcId::at(p))
            {
                self.procs[p].status = ProcStatus::Dead;
            }
        }
        // `always_comb` and VHDL processes run once at time zero after the
        // `initial` and `always` blocks have started (IEEE 1800-2017
        // §9.2.2.2.2): those are queued first. They used to be queued in
        // declaration order, so an `always_comb` above an `initial` ran
        // first.
        // Within each group the deepest instances' processes go first: xsim
        // runs a submodule's `initial` before its parent's (both orders are
        // the standard's).
        let mut order: Vec<usize> = (0..self.procs.len()).collect();
        order.sort_by_key(|p| {
            let depth = self.instances[self.procs[*p].inst.idx()]
                .path
                .matches('.')
                .count();
            (
                matches!(
                    self.procs[*p].process.kind,
                    ProcessKind::Comb | ProcessKind::Sensitive(_)
                ),
                std::cmp::Reverse(depth),
            )
        });
        for p in order {
            let process = self.procs[p].process;
            if self.procs[p].status == ProcStatus::Dead {
                continue;
            }
            // `always @(a)` waits for `a` first (`ir::WAITS_FIRST_ATTR`),
            // and a `final` block for the end (`ir::FINAL_ATTR`).
            let waits = (matches!(process.kind, ProcessKind::Comb | ProcessKind::Sensitive(_))
                && process.attrs.is_set(crate::ir::WAITS_FIRST_ATTR))
                || process.attrs.is_set(crate::ir::FINAL_ATTR);
            if !waits
                && matches!(
                    process.kind,
                    ProcessKind::Initial
                        | ProcessKind::Free
                        | ProcessKind::Comb
                        | ProcessKind::Sensitive(_)
                )
            {
                self.procs[p].status = ProcStatus::Queued;
                self.active.push_back(Event::Proc(ProcId::at(p)));
            }
        }
    }

    /// Runs an initialiser's assignments in place, before time zero: each
    /// value written straight into its variable, so that no process sees
    /// an event. `false`, having done nothing, for a body that is anything
    /// but undelayed blocking assignments to variables; it then runs at
    /// time zero as an `initial` block would.
    fn initialise_silently(&mut self, pid: ProcId) -> bool {
        let inst = self.procs[pid.idx()].inst;
        let body = &self.procs[pid.idx()].process.body;
        let simple = body.iter().all(|s| {
            matches!(
                &s.kind,
                crate::ir::StmtKind::Assign {
                    kind: crate::ir::AssignKind::Blocking,
                    delay: None,
                    ..
                }
            )
        });
        if !simple {
            return false;
        }
        for stmt in body {
            let crate::ir::StmtKind::Assign { target, value, .. } = &stmt.kind else {
                continue;
            };
            let v = self.eval(inst, *value);
            let target = self.target_from_lvalue(inst, target);
            let v = self.logic_for_target(&target, v);
            let stores = self.resolve_target(inst, &target);
            self.store_silently(&stores, &v);
        }
        true
    }
}

/// Picks the top module: the option, the design's top, or the only module
/// nothing instantiates.
fn select_top(
    design: &Design,
    options: &SimOptions,
    diags: &mut Diagnostics,
) -> Result<ModuleId, Diagnostics> {
    if let Some(name) = &options.top {
        return design.module_by_name(name).ok_or_else(|| {
            diags.push(Diagnostic::error(format!(
                "top module `{name}` is not in the design"
            )));
            std::mem::take(diags)
        });
    }
    if let Some(top) = design.top
        && design.modules.contains(top)
    {
        return Ok(top);
    }
    let mut instantiated = vec![false; design.modules.len()];
    for (_, m) in design.modules.iter() {
        for (_, inst) in m.instances.iter() {
            if let Some(id) = inst.module.id()
                && let Some(slot) = instantiated.get_mut(id.index())
            {
                *slot = true;
            }
        }
    }
    let roots: Vec<ModuleId> = design
        .modules
        .iter()
        .filter(|(id, m)| !instantiated[id.index()] && !m.blackbox)
        .map(|(id, _)| id)
        .collect();
    match roots.as_slice() {
        [one] => Ok(*one),
        [] => {
            diags.push(Diagnostic::error(
                "the design has no module to simulate; select a top",
            ));
            Err(std::mem::take(diags))
        }
        many => {
            let names: Vec<String> = many
                .iter()
                .map(|id| design.modules[*id].name.to_string())
                .collect();
            diags.push(
                Diagnostic::error("the design has several root modules; select a top")
                    .with_note(format!("candidates: {}", names.join(", "))),
            );
            Err(std::mem::take(diags))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::builder::ModuleBuilder;
    use crate::source::{SourceMap, Span};

    fn span() -> Span {
        let mut map = SourceMap::new();
        let id = map.add("t", "").unwrap();
        Span::new(id, 0, 0)
    }

    #[test]
    fn ticks_round_to_precision() {
        assert_eq!(fs_to_ticks(0, 1000), 0);
        assert_eq!(fs_to_ticks(1, 1000), 1);
        assert_eq!(fs_to_ticks(1499, 1000), 1);
        assert_eq!(fs_to_ticks(1500, 1000), 2);
        assert_eq!(fs_to_ticks(5_000_000, 1000), 5000);
    }

    #[test]
    fn top_selection() {
        let span = span();
        let mut design = Design::new();
        let leaf = design.add_module(ModuleBuilder::new("leaf", span).finish());
        let mut b = ModuleBuilder::new("top", span);
        b.instance("u0", ModuleRef::Resolved(leaf), Vec::new());
        let top = design.add_module(b.finish());
        let mut diags = Diagnostics::new();
        assert_eq!(
            select_top(&design, &SimOptions::default(), &mut diags).ok(),
            Some(top)
        );
        design.top = Some(leaf);
        assert_eq!(
            select_top(&design, &SimOptions::default(), &mut diags).ok(),
            Some(leaf)
        );
        let opts = SimOptions {
            top: Some("leaf".into()),
            ..SimOptions::default()
        };
        assert_eq!(select_top(&design, &opts, &mut diags).ok(), Some(leaf));
        let opts = SimOptions {
            top: Some("nope".into()),
            ..SimOptions::default()
        };
        assert!(select_top(&design, &opts, &mut diags).is_err());
        design.top = None;
        design.add_module(ModuleBuilder::new("other", span).finish());
        let err = select_top(&design, &SimOptions::default(), &mut diags).unwrap_err();
        assert!(err.has_errors());
        let empty = Design::new();
        let mut diags = Diagnostics::new();
        assert!(select_top(&empty, &SimOptions::default(), &mut diags).is_err());
    }

    #[test]
    fn port_aliasing_shares_signals() {
        let span = span();
        let mut design = Design::new();
        let mut leaf = ModuleBuilder::new("leaf", span);
        let a = leaf.input("a", Type::bits(4));
        let y = leaf.output("y", Type::bits(4));
        let av = leaf.net(a);
        leaf.assign(y, av);
        let leaf_id = design.add_module(leaf.finish());
        let mut top = ModuleBuilder::new("top", span);
        let x = top.input("x", Type::bits(4));
        let z = top.output("z", Type::bits(8));
        let w = top.add_net("w", Type::bits(4));
        let xv = top.net(x);
        let wv = top.net(w);
        let lo = top.slice(xv, 1, 0);
        let hi = top.slice(xv, 3, 2);
        let swapped = top.concat(vec![lo, hi]);
        let zv = top.net(z);
        let zlo = top.slice(zv, 3, 0);
        top.instance(
            "u0",
            ModuleRef::Resolved(leaf_id),
            vec![("a".into(), xv), ("y".into(), wv)],
        );
        top.instance(
            "u1",
            ModuleRef::Resolved(leaf_id),
            vec![("a".into(), swapped), ("y".into(), zlo)],
        );
        design.add_module(top.finish());
        let sim = Simulator::elaborate(&design, SimOptions::default()).unwrap();
        assert_eq!(sim.instances.len(), 3);
        // u0.a aliases top.x and u0.y aliases top.w.
        let u0 = &sim.instances[1];
        assert_eq!(u0.path, "top.u0");
        assert_eq!(u0.nets[0], sim.instances[0].nets[0]);
        assert_eq!(u0.nets[1], sim.instances[0].nets[2]);
        // u1's ports are implicit drivers: 2 from leaves + 2 implicit.
        assert_eq!(sim.drivers.len(), 4);
        assert!(sim.messages.is_empty());
    }

    /// A module nothing declares cannot be simulated: the box would hold
    /// `x`, and a testbench reading that reports a failure of a design
    /// that is in fact fine. So it is an error, named at the
    /// instantiation, and so is a recursive hierarchy.
    #[test]
    fn an_undeclared_module_and_a_recursive_one_are_errors() {
        let span = span();
        let mut design = Design::new();
        let mut top = ModuleBuilder::new("top", span);
        top.instance("u0", ModuleRef::Unresolved("ghost".into()), Vec::new());
        let top_id = design.add_module(top.finish());
        design
            .module_mut(top_id)
            .instances
            .push(crate::ir::Instance {
                name: "me".into(),
                module: ModuleRef::Resolved(top_id),
                connections: Vec::new(),
                params: Default::default(),
                attrs: Default::default(),
                span,
            });
        design.top = Some(top_id);
        let err = Simulator::elaborate(&design, SimOptions::default()).unwrap_err();
        assert_eq!(err.error_count(), 2);
        assert_eq!(err.warning_count(), 0);
        let text = err
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            text.contains("no module named `ghost` is defined, so instance `u0` cannot be"),
            "{text}"
        );
    }

    /// A black box the design *declares* does simulate: the outputs stay
    /// undriven and one warning says so. That is the line between a
    /// deliberate hole and a file left off the build.
    #[test]
    fn a_declared_black_box_simulates_with_a_warning() {
        let span = span();
        let mut design = Design::new();
        let mut stub = ModuleBuilder::new("vendor_core", span);
        stub.input("a", Type::bit());
        stub.output("y", Type::bit());
        let mut stub = stub.finish();
        stub.blackbox = true;
        let stub_id = design.add_module(stub);

        let mut top = ModuleBuilder::new("top", span);
        let a = top.input("a", Type::bit());
        let y = top.output("y", Type::bit());
        let (av, yv) = (top.net(a), top.net(y));
        top.instance(
            "u0",
            ModuleRef::Resolved(stub_id),
            vec![("a".into(), av), ("y".into(), yv)],
        );
        design.top = Some(design.add_module(top.finish()));
        let sim = Simulator::elaborate(&design, SimOptions::default())
            .expect("a declared black box is not an error");
        assert_eq!(sim.messages.warning_count(), 1);
        assert!(
            sim.messages.iter().any(|d| d
                .message
                .contains("of black box `vendor_core` has no behaviour")),
            "{}",
            sim.messages.len()
        );
    }
}
