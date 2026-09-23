//! Mapping generic IR cells onto a device's primitives.
//!
//! [`map`] rewrites one module in place so that what a device does in
//! hard logic is expressed as that device's primitives, and everything
//! else is left for the technology mapper. Six things happen, in this
//! order, each of them optional ([`MapOptions`]) and each of them
//! reported ([`MapReport`]):
//!
//! | Step | What it recognises | What it emits |
//! |------|--------------------|---------------|
//! | Block RAM | a [`Memory`] with its `MemRdPort` / `MemWrPort` cells | one block per width x depth slice (per read port, when the block has too few), carrying its slice of the initial contents, plus address decoding and output muxing as cells; or, below the threshold, the memory built out of logic |
//! | DSP | a `Mul`, and a `Mul` feeding an `Add` | one multiplier or multiply-accumulate block |
//! | Carry | an `Add` at least `min_carry_width` bits wide | a chain of carry primitives plus `Xor` cells for the sums |
//! | IO buffers | every top-level port | one IO primitive per bit, carrying the constraints' `io_standard`, `drive`, `slew` and `pullup`; for a port with a `ddr` clock, one per *two* bits with both edges registered (in the buffer where the family's buffer does it, in a `ddr_in` / `ddr_out` register beside it where it does not); and an `iodelay` element where a delay is asked for |
//! | PLLs | a clock constraint on a net nothing drives | the device's PLL, its dividers solved by [`super::pll::solve`], fed from the clock constrained on an input port |
//! | Clock buffers | a net driving many flip-flop clock pins | a global buffer, with the clock pins moved onto it |
//!
//! Every primitive becomes a [`CellKind::Blackbox`] named after the
//! device's primitive, with its parameters in [`Cell::params`] and the
//! constraints that produced it in [`Cell::attrs`], so the result is still
//! ordinary IR: it validates, prints as `.rtl`, and emits as Verilog or
//! Yosys JSON like anything else.
//!
//! # Nothing is guessed
//!
//! Each step asks the database for what it needs — a [`BramShape`] whose
//! width modes fit, a [`DspShape`] wide enough, a primitive with the
//! ports the step must connect — and *declines with a note* when the
//! device does not describe it.
//!
//! # The memory fallback
//!
//! A memory that fits no block RAM is not left as a memory: a
//! place-and-route tool takes primitives only, so it is *built out of
//! logic* here, and which logic comes from the device file. A family
//! that declares a distributed RAM primitive with a usable port map (the
//! ECP5's `TRELLIS_DPR16X4`) gets one per bank, with its shape read off
//! that port map rather than written in Rust; a family that declares
//! none (iCE40) gets one flip-flop per bit, a write enable decoded per
//! word and a multiplexer per read port. Several read ports mean several
//! copies of a distributed RAM, which has one; one array of flip-flops
//! serves them all.
//!
//! The cases the fallback cannot take are *named* rather than
//! approximated: initial contents flip-flops cannot be preloaded with, a
//! write that is not clocked, write ports on different clocks, and a
//! memory over [`MapOptions::max_logic_bits`], where "it does not fit
//! this part" is the useful answer. [`BramFallback`] says which of these
//! happened and what was built.
//!
//! # Block RAM wiring and contents
//!
//! How a mode puts a word on the pins and in the contents is the
//! database's [`BramModeLayout`], not Rust: the data pins a narrow mode
//! uses (the iCE40 spreads 512x8 over pins 0, 2, .., 14), where the word
//! address starts on the address port and what the pins below it are
//! tied to (the ECP5 addresses in units of its narrowest mode, so its
//! 18-bit mode starts at pin 4, with the write byte enables tied high),
//! and where each bit of each word sits in the rows the initialisation
//! parameters hold.
//!
//! A memory with initial contents ([`Memory::init`]) gets them in every
//! block: each block the width and depth slices it holds, every copy of
//! a duplicated memory the same, `x` and `z` bits and words past the
//! end of the contents as 0. When the database cannot say where they go
//! — a primitive without the `init` flag, no `init_params`, a mode
//! without a layout — the memory is still mapped and the loss is a
//! warning ([`NO_BRAM_INIT`]), never a silence: the netlist's blocks
//! start blank, and a ROM in them would read zeros on the device.
//!
//! [`Memory::init`]: crate::ir::Memory::init
//! [`BramModeLayout`]: super::device::BramModeLayout
//!
//! [`Memory`]: crate::ir::Memory
//! [`DspShape`]: super::device::DspShape
//! [`Cell::params`]: crate::ir::Cell::params
//! [`Cell::attrs`]: crate::ir::Cell::attrs

use std::collections::{BTreeMap, HashSet};
use std::fmt::Write as _;

use super::constraints::{Constraints, IoAttrs};
use super::device::{
    BelKind, BelRole, BramInitLayout, BramInitParams, BramModeLayout, BramShape, Device,
    PllFeedback, PllShape,
};
use super::pll::PllSolution;
use crate::diag::{Diagnostic, Diagnostics};
use crate::ir::expr::operands;
use crate::ir::{
    Assign, AttrValue, Attrs, Bit, Cell, CellId, CellKind, Const, Design, Expr, ExprId, ExprKind,
    Lvalue, MemoryId, Module, ModuleId, Name, Net, NetId, NetKind, PortDir, Type, infer_type,
};
use crate::source::Span;

/// Diagnostic code for a memory that could not become a block RAM.
pub const NO_BLOCK_RAM: &str = "F0300";
/// Diagnostic code for a clock that could not get a global buffer.
pub const NO_GLOBAL_BUFFER: &str = "F0301";
/// Diagnostic code for an IO buffer Reticle could not wire completely.
pub const PARTIAL_IO: &str = "F0302";
/// Diagnostic code for a clock constraint no PLL of the device can meet.
pub const NO_PLL: &str = "F0303";
/// Diagnostic code for an IO register or delay the device cannot build.
pub const NO_IO_REGISTER: &str = "F0304";
/// Diagnostic code for a memory whose initial contents the block RAMs it
/// was mapped to cannot hold.
pub const NO_BRAM_INIT: &str = "F0305";

/// Which mapping steps run, and the thresholds they use.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MapOptions {
    /// Map memories onto block RAMs.
    pub infer_block_ram: bool,
    /// Memories smaller than this many bits stay in logic, unless
    /// `ram_style = "block"` says otherwise.
    pub min_bram_bits: u64,
    /// A memory of more than this many bits is not built out of logic
    /// when it misses a block RAM: it is left as a memory and reported,
    /// since some thousands of flip-flops are almost never what the
    /// designer meant and the honest answer is that the design does not
    /// fit the part.
    pub max_logic_bits: u64,
    /// Map multipliers onto DSP blocks.
    pub infer_dsp: bool,
    /// Insert an IO buffer at every top-level port.
    pub insert_io_buffers: bool,
    /// Instantiate a PLL for a clock constraint on a net nothing drives,
    /// from the clock constrained on an input port.
    pub infer_pll: bool,
    /// Put heavily used clocks on a global buffer.
    pub insert_clock_buffers: bool,
    /// How many flip-flop clock pins a net must drive to earn one.
    pub global_buffer_threshold: usize,
    /// Expand wide adders onto the carry chain.
    pub map_carry: bool,
    /// The narrowest adder worth a carry chain.
    pub min_carry_width: u32,
}

impl Default for MapOptions {
    fn default() -> Self {
        MapOptions {
            infer_block_ram: true,
            min_bram_bits: 256,
            max_logic_bits: 4096,
            infer_dsp: true,
            insert_io_buffers: true,
            infer_pll: true,
            insert_clock_buffers: true,
            global_buffer_threshold: 8,
            map_carry: true,
            min_carry_width: 4,
        }
    }
}

/// One memory turned into block RAMs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BramMapping {
    /// The memory's name.
    pub memory: String,
    /// The primitive used.
    pub primitive: String,
    /// Width of one element.
    pub data_width: u32,
    /// Number of elements.
    pub depth: u64,
    /// The width mode chosen, as `(data_width, depth)`.
    pub mode: (u32, u32),
    /// How many blocks side by side carry the width.
    pub wide: u32,
    /// How many blocks stacked carry the depth.
    pub deep: u32,
    /// How many copies of the contents there are: one, or one per read
    /// port when the memory wanted more ports than one block has, all
    /// written together from the memory's single write port (or, for a
    /// read-only memory, never written at all).
    pub copies: u32,
    /// The blocks carry the memory's initial contents in their
    /// initialisation parameters. False for a memory without any, and
    /// for one whose contents the blocks could not hold, which is then
    /// reported as a warning ([`NO_BRAM_INIT`]).
    pub initialised: bool,
}

impl BramMapping {
    /// Total number of blocks used.
    pub fn blocks(&self) -> u32 {
        self.wide * self.deep * self.copies
    }
}

/// What happened to a memory that did not become a block RAM.
///
/// The fallback is *performed*, not only decided: `style` says what the
/// memory was actually built from and `cells` how many of them it took.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BramFallback {
    /// The memory's name.
    pub memory: String,
    /// Why it was not mapped onto a block RAM.
    pub reason: String,
    /// What it was built from instead: `distributed LUT RAM` when the
    /// device declares a usable one, `flip-flops` when it does not,
    /// `logic` for a read-only memory that is nothing but multiplexers,
    /// and `a memory` when even this could not be done, which `reason`
    /// then explains.
    pub style: &'static str,
    /// The primitive the fallback instantiated, when it used one.
    pub primitive: Option<String>,
    /// How many storage elements it built — flip-flops, or distributed
    /// RAM primitives. Zero for a read-only memory and for one that was
    /// left in place.
    pub cells: u32,
    /// True when the memory really was lowered and no `memory` is left
    /// in the netlist. False is what [`super::check_nextpnr_json`]
    /// reports as "it did not become block RAM or logic".
    pub built: bool,
}

/// One multiplier turned into a DSP block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DspMapping {
    /// The name of the cell that was replaced.
    pub cell: String,
    /// The primitive used.
    pub primitive: String,
    /// Width of the first operand.
    pub a_width: u32,
    /// Width of the second operand.
    pub b_width: u32,
    /// Width of the result.
    pub p_width: u32,
    /// True when an adder was folded into the block.
    pub multiply_add: bool,
    /// True when that adder accumulates the block's own output.
    pub accumulate: bool,
}

/// One port given IO buffers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IoMapping {
    /// The port's name.
    pub port: String,
    /// The primitive used.
    pub primitive: String,
    /// How many buffers, one per pin: one per bit, or one per two bits
    /// for a double-data-rate port.
    pub bits: u32,
    /// The pin the constraints assign, if any.
    pub pin: Option<String>,
    /// The IO standard the constraints ask for, if any.
    pub io_standard: Option<String>,
    /// For a double-data-rate port, the clock of its registers and the
    /// primitive that registers both edges: the IO buffer itself on a
    /// family whose buffer does (iCE40 `SB_IO`), a register beside it on
    /// one that does not (ECP5 `IDDRX1F` / `ODDRX1F`).
    pub ddr: Option<(String, String)>,
    /// The delay put between the pin and the fabric, in the device's
    /// steps, and the primitive that provides it.
    pub delay: Option<(u32, String)>,
}

/// One clock net considered for a global buffer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClockMapping {
    /// The net carrying the clock.
    pub net: String,
    /// How many flip-flop clock pins it drives.
    pub fanout: usize,
    /// The buffer inserted, or `None` when the net stayed on local
    /// routing.
    pub primitive: Option<String>,
    /// Why it stayed, when it did.
    pub reason: Option<String>,
}

/// One PLL instantiated because a clock constraint asked for a frequency
/// the input clock does not have.
///
/// Frequencies are whole hertz so that the report compares exactly; the
/// solver's own [`super::pll::PllSolution`] keeps the unrounded values.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PllMapping {
    /// The net the PLL drives: the one the constraint named.
    pub net: String,
    /// The input clock it is generated from.
    pub source: String,
    /// The primitive used.
    pub primitive: String,
    /// The input clock's frequency.
    pub input_hz: u64,
    /// The frequency the constraint asked for.
    pub requested_hz: u64,
    /// The frequency the chosen dividers give.
    pub achieved_hz: u64,
    /// The VCO frequency of that setting.
    pub vco_hz: u64,
    /// Every parameter the setting fixes, with its value.
    pub params: Vec<(String, i64)>,
}

impl PllMapping {
    /// How far the achieved frequency is from the request, in parts per
    /// million; positive when it is above.
    pub fn error_ppm(&self) -> f64 {
        if self.requested_hz == 0 {
            return 0.0;
        }
        let achieved = hz_to_f64(self.achieved_hz);
        let requested = hz_to_f64(self.requested_hz);
        (achieved - requested) / requested * 1e6
    }
}

/// Hertz as a float. Clock frequencies are below 2^53 Hz by nine orders
/// of magnitude, where the conversion is exact.
fn hz_to_f64(hz: u64) -> f64 {
    let high = u32::try_from(hz >> 32).unwrap_or(u32::MAX);
    let low = u32::try_from(hz & 0xFFFF_FFFF).unwrap_or(0);
    f64::from(high) * 4_294_967_296.0 + f64::from(low)
}

/// Megahertz, for a report line.
fn mhz(hz: u64) -> String {
    format!("{:.3} MHz", hz_to_f64(hz) / 1e6)
}

/// One adder expanded onto the carry chain.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CarryMapping {
    /// The name of the cell that was replaced.
    pub cell: String,
    /// The primitive used.
    pub primitive: String,
    /// How many bits the adder had.
    pub width: u32,
    /// How many carry primitives the chain uses: one per bit except the
    /// last, whose carry-out nothing reads.
    pub primitives: u32,
}

/// What [`map`] did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MapReport {
    /// The device the module was mapped for.
    pub device: String,
    /// Memories turned into block RAMs.
    pub block_rams: Vec<BramMapping>,
    /// Memories left in logic.
    pub bram_fallbacks: Vec<BramFallback>,
    /// Multipliers turned into DSP blocks.
    pub dsps: Vec<DspMapping>,
    /// Ports given IO buffers.
    pub io_buffers: Vec<IoMapping>,
    /// Clock nets and what became of them.
    pub clocks: Vec<ClockMapping>,
    /// PLLs instantiated for clock constraints.
    pub plls: Vec<PllMapping>,
    /// Adders expanded onto the carry chain.
    pub carry_chains: Vec<CarryMapping>,
    /// Anything a step declined to do, in the order it was decided.
    pub notes: Vec<String>,
}

impl MapReport {
    /// True when nothing was mapped and nothing was declined.
    pub fn is_empty(&self) -> bool {
        self.block_rams.is_empty()
            && self.bram_fallbacks.is_empty()
            && self.dsps.is_empty()
            && self.io_buffers.is_empty()
            && self.clocks.is_empty()
            && self.plls.is_empty()
            && self.carry_chains.is_empty()
            && self.notes.is_empty()
    }

    /// Renders the report as plain text, one section per step.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "device: {}", self.device);
        if !self.block_rams.is_empty() || !self.bram_fallbacks.is_empty() {
            out.push_str("block RAM:\n");
            for item in &self.block_rams {
                let _ = write!(
                    out,
                    "  {} ({}x{}) -> {} x {} in {}x{} mode ({} wide, {} deep",
                    item.memory,
                    item.depth,
                    item.data_width,
                    item.blocks(),
                    item.primitive,
                    item.mode.0,
                    item.mode.1,
                    item.wide,
                    item.deep
                );
                if item.copies > 1 {
                    let _ = write!(out, ", {} copies, one per read port", item.copies);
                }
                if item.initialised {
                    out.push_str(", initialised");
                }
                out.push_str(")\n");
            }
            for item in &self.bram_fallbacks {
                let _ = write!(out, "  {} -> {}", item.memory, item.style);
                match (&item.primitive, item.cells) {
                    (Some(primitive), n) => {
                        let _ = write!(out, ", {n} x {primitive}");
                    }
                    (None, n) if n > 0 => {
                        let _ = write!(out, ", {n} flip-flop(s)");
                    }
                    (None, _) => {}
                }
                let _ = writeln!(out, " ({})", item.reason);
            }
        }
        if !self.dsps.is_empty() {
            out.push_str("dsp:\n");
            for item in &self.dsps {
                let shape = if item.accumulate {
                    "multiply-accumulate"
                } else if item.multiply_add {
                    "multiply-add"
                } else {
                    "multiply"
                };
                let _ = writeln!(
                    out,
                    "  {} ({}x{} -> {}) -> {} {}",
                    item.cell, item.a_width, item.b_width, item.p_width, item.primitive, shape
                );
            }
        }
        if !self.carry_chains.is_empty() {
            out.push_str("carry:\n");
            for item in &self.carry_chains {
                let _ = writeln!(
                    out,
                    "  {} ({} bits) -> {} x {}",
                    item.cell, item.width, item.primitives, item.primitive
                );
            }
        }
        if !self.io_buffers.is_empty() {
            out.push_str("io:\n");
            for item in &self.io_buffers {
                let _ = write!(
                    out,
                    "  {} ({} bits) -> {}",
                    item.port, item.bits, item.primitive
                );
                if let Some(pin) = &item.pin {
                    let _ = write!(out, " at pin {pin}");
                }
                if let Some(standard) = &item.io_standard {
                    let _ = write!(out, " as {standard}");
                }
                if let Some((clock, primitive)) = &item.ddr {
                    let _ = write!(out, ", double data rate on {clock} ({primitive})");
                }
                if let Some((steps, primitive)) = &item.delay {
                    let _ = write!(out, ", delayed {steps} step(s) ({primitive})");
                }
                out.push('\n');
            }
        }
        if !self.plls.is_empty() {
            out.push_str("pll:\n");
            for item in &self.plls {
                let params: Vec<String> = item
                    .params
                    .iter()
                    .map(|(key, value)| format!("{key}={value}"))
                    .collect();
                let _ = writeln!(
                    out,
                    "  {} -> {} from {} at {}: {} for {} asked ({:+.1} ppm), vco {}, {}",
                    item.net,
                    item.primitive,
                    item.source,
                    mhz(item.input_hz),
                    mhz(item.achieved_hz),
                    mhz(item.requested_hz),
                    // Rounded first, so that a tiny negative error does
                    // not print as `-0.0`.
                    (item.error_ppm() * 10.0).round() / 10.0 + 0.0,
                    mhz(item.vco_hz),
                    params.join(" ")
                );
            }
        }
        if !self.clocks.is_empty() {
            out.push_str("clocks:\n");
            for item in &self.clocks {
                match (&item.primitive, &item.reason) {
                    (Some(primitive), _) => {
                        let _ = writeln!(
                            out,
                            "  {} ({} flip-flops) -> {}",
                            item.net, item.fanout, primitive
                        );
                    }
                    (None, Some(reason)) => {
                        let _ = writeln!(
                            out,
                            "  {} ({} flip-flops) -> local routing ({reason})",
                            item.net, item.fanout
                        );
                    }
                    (None, None) => {
                        let _ = writeln!(
                            out,
                            "  {} ({} flip-flops) -> local routing",
                            item.net, item.fanout
                        );
                    }
                }
            }
        }
        if !self.notes.is_empty() {
            out.push_str("notes:\n");
            for note in &self.notes {
                let _ = writeln!(out, "  {note}");
            }
        }
        out
    }
}

/// Maps `module` onto `device`, in place.
///
/// `constraints` supplies the IO options carried onto the IO buffers;
/// pass an empty value when there are none. Problems that the user can do
/// something about are reported through `diags`; what was mapped is in the
/// returned [`MapReport`].
pub fn map(
    design: &mut Design,
    module: ModuleId,
    device: &Device,
    constraints: &Constraints,
    options: &MapOptions,
    diags: &mut Diagnostics,
) -> MapReport {
    let mut report = MapReport {
        device: device.name.clone(),
        ..MapReport::default()
    };
    let Some(module) = design.modules.get_mut(module) else {
        return report;
    };
    let mut mapper = Mapper {
        device,
        constraints,
        options,
        report: &mut report,
        diags,
    };
    if options.infer_block_ram {
        mapper.block_rams(module);
    }
    if options.infer_dsp {
        mapper.dsps(module);
    }
    if options.map_carry {
        mapper.carry_chains(module);
    }
    if options.insert_io_buffers {
        mapper.io_buffers(module);
    }
    if options.infer_pll {
        mapper.clock_generators(module);
    }
    if options.insert_clock_buffers {
        mapper.clock_buffers(module);
    }
    report
}

/// The state one [`map`] call carries between steps.
struct Mapper<'a> {
    device: &'a Device,
    constraints: &'a Constraints,
    options: &'a MapOptions,
    report: &'a mut MapReport,
    diags: &'a mut Diagnostics,
}

// --- small IR helpers -------------------------------------------------------
//
// The ones marked `pub(super)` are shared with [`super::techcells`],
// which builds the same kind of cells one step later in the flow.

/// Adds an expression node, typed by the operator rules.
///
/// Every node built here has operands the rules accept; an ill-typed one
/// would be a bug in this module, so it falls back to the first operand's
/// type and lets `ir::validate` report it with a span, exactly as
/// `ModuleBuilder` does.
pub(super) fn expr(module: &mut Module, kind: ExprKind, span: Span) -> ExprId {
    let ty = match infer_type(module, &kind) {
        Ok(ty) => ty,
        Err(_) => operands(&kind)
            .first()
            .and_then(|id| module.exprs.get(*id))
            .map_or(Type::bit(), |e| e.ty.clone()),
    };
    module.add_expr(Expr::new(kind, ty, span))
}

pub(super) fn net_expr(module: &mut Module, net: NetId, span: Span) -> ExprId {
    expr(module, ExprKind::Net(net), span)
}

pub(super) fn const_expr(module: &mut Module, value: Const, span: Span) -> ExprId {
    expr(module, ExprKind::Const(value), span)
}

/// `base[hi:lo]`, or `base` itself when the slice covers everything.
pub(super) fn slice_expr(
    module: &mut Module,
    base: ExprId,
    hi: u32,
    lo: u32,
    span: Span,
) -> ExprId {
    let width = module.exprs.get(base).and_then(|e| e.ty.width());
    if lo == 0 && width == Some(hi + 1) {
        return base;
    }
    expr(module, ExprKind::Slice { base, hi, lo }, span)
}

/// A name no net and no cell of the module uses yet: `base`, or `base$1`,
/// `base$2` and so on until one is free.
///
/// The obvious way to write that is a lookup per candidate, and it is
/// why lowering a memory to logic used to take minutes: the hundreds of
/// cells one memory becomes all ask for the same base, so candidate `k`
/// costs `k` scans of every net and cell, and a module with a dozen
/// small memories spends its afternoon looking up names. One pass
/// collecting the suffixes already spoken for gives the same answer —
/// the *smallest* free suffix, so the names are the ones the old loop
/// produced — for one scan instead of `k` of them.
fn unique_name(module: &Module, base: &str) -> Name {
    let mut base_taken = false;
    let mut taken: HashSet<u32> = HashSet::new();
    {
        let mut note = |name: &str| {
            if name == base {
                base_taken = true;
            } else if let Some(suffix) = name
                .strip_prefix(base)
                .and_then(|rest| rest.strip_prefix('$'))
                .and_then(|digits| digits.parse::<u32>().ok())
            {
                taken.insert(suffix);
            }
        };
        for (_, net) in module.nets.iter() {
            note(net.name.as_str());
        }
        for (_, cell) in module.cells.iter() {
            note(cell.name.as_str());
        }
    }
    if !base_taken {
        return Name::new(base);
    }
    for suffix in 1u32.. {
        if !taken.contains(&suffix) {
            return Name::new(format!("{base}${suffix}"));
        }
    }
    unreachable!("a unique name always exists")
}

pub(super) fn add_net(module: &mut Module, base: &str, ty: Type, span: Span) -> NetId {
    let name = unique_name(module, base);
    module.nets.push(Net {
        name,
        ty,
        kind: NetKind::Wire,
        attrs: Attrs::new(),
        span,
    })
}

pub(super) fn add_cell(
    module: &mut Module,
    base: &str,
    kind: CellKind,
    inputs: Vec<(Name, ExprId)>,
    outputs: Vec<(Name, NetId)>,
    span: Span,
) -> CellId {
    let name = unique_name(module, base);
    module.cells.push(Cell {
        name,
        kind,
        inputs,
        outputs,
        params: Attrs::new(),
        attrs: Attrs::new(),
        span,
    })
}

pub(super) fn add_assign(module: &mut Module, target: NetId, value: ExprId, span: Span) {
    module.assigns.push(Assign {
        target: Lvalue::Net(target),
        value,
        delay: None,
        attrs: Attrs::new(),
        span,
    });
}

/// `value` on every bit of an `bits`-wide pin; `value` itself when the
/// pin is one bit, which is every family but a byte-enabled block RAM.
fn repeat_bits(module: &mut Module, value: ExprId, bits: u32, span: Span) -> ExprId {
    if bits <= 1 {
        return value;
    }
    let parts = vec![value; usize::try_from(bits).unwrap_or(1)];
    expr(module, ExprKind::Concat(parts), span)
}

/// Number of address bits needed to index `depth` elements.
fn addr_bits(depth: u64) -> u32 {
    if depth <= 1 {
        0
    } else {
        u64::BITS - (depth - 1).leading_zeros()
    }
}

/// `ceil(a / b)` for positive `b`.
fn div_ceil_u32(a: u32, b: u32) -> u32 {
    a.div_ceil(b)
}

/// The width of an expression, or 1 when it is not a bit vector.
fn expr_width(module: &Module, id: ExprId) -> u32 {
    module.exprs.get(id).and_then(|e| e.ty.width()).unwrap_or(1)
}

/// The width of a net, or 1 when it is not a bit vector.
fn net_width(module: &Module, id: NetId) -> u32 {
    module.nets.get(id).and_then(|n| n.ty.width()).unwrap_or(1)
}

/// How many times each net is read, counting every cell input, every
/// continuous assign and every statement.
fn net_use_counts(module: &Module) -> Vec<usize> {
    let mut counts = vec![0usize; module.nets.len()];
    let mut roots = Vec::new();
    module.for_each_root_expr(|id| roots.push(id));
    for id in roots {
        count_nets(module, id, &mut counts);
    }
    counts
}

/// Adds one to `counts` for every net `id` reads.
fn count_nets(module: &Module, id: ExprId, counts: &mut [usize]) {
    let mut stack = vec![id];
    while let Some(id) = stack.pop() {
        let Some(node) = module.exprs.get(id) else {
            continue;
        };
        if let ExprKind::Net(net) = node.kind {
            if let Some(slot) = counts.get_mut(net.index()) {
                *slot += 1;
            }
            continue;
        }
        stack.extend(operands(&node.kind));
    }
}

impl Mapper<'_> {
    fn note(&mut self, note: impl Into<String>) {
        self.report.notes.push(note.into());
    }

    // --- block RAM ----------------------------------------------------------

    fn block_rams(&mut self, module: &mut Module) {
        let memories: Vec<MemoryId> = module.memories.ids().collect();
        let mut mapped: Vec<CellId> = Vec::new();
        let mut lowered: Vec<MemoryId> = Vec::new();
        for mem in memories {
            if let Some(used) = self.block_ram(module, mem) {
                mapped.extend(used);
            } else if let Some(used) = self.lower_memory(module, mem) {
                mapped.extend(used);
                lowered.push(mem);
            }
        }
        if !mapped.is_empty() {
            module.cells.retain(|id, _| !mapped.contains(&id));
        }
        if !lowered.is_empty() {
            // The memories are gone, so the ids of the ones left move.
            // Every port cell of a lowered memory went with it, so the
            // only references to fix are those of the survivors.
            let remap = module.memories.retain(|id, _| !lowered.contains(&id));
            for (_, cell) in module.cells.iter_mut() {
                if let CellKind::MemRdPort { mem, .. } | CellKind::MemWrPort { mem, .. } =
                    &mut cell.kind
                    && let Some(Some(new)) = remap.get(mem.index())
                {
                    *mem = *new;
                }
            }
        }
    }

    /// Records what happened to a memory the block RAM mapper declined.
    fn record_fallback(
        &mut self,
        memory: &str,
        style: &'static str,
        primitive: Option<String>,
        cells: u32,
    ) {
        if let Some(entry) = self
            .report
            .bram_fallbacks
            .iter_mut()
            .rev()
            .find(|f| f.memory == memory)
        {
            entry.style = style;
            entry.primitive = primitive;
            entry.cells = cells;
            entry.built = true;
        }
    }

    /// Maps one memory, returning the port cells it replaced.
    fn block_ram(&mut self, module: &mut Module, mem: MemoryId) -> Option<Vec<CellId>> {
        let memory = module.memories.get(mem)?;
        let name = memory.name.as_str().to_owned();
        let span = memory.span;
        let width = memory.elem.width().unwrap_or(0);
        let depth = memory.size;
        let style = memory
            .attrs
            .get("ram_style")
            .and_then(AttrValue::as_str)
            .unwrap_or("")
            .to_ascii_lowercase();
        let forced = matches!(style.as_str(), "block" | "bram" | "ram_block");
        let refused = matches!(
            style.as_str(),
            "distributed" | "logic" | "registers" | "ff" | "flops"
        );
        let fallback_style = if self.lutram_shape().is_some() {
            "distributed LUT RAM"
        } else {
            "flip-flops"
        };
        let give_up = |this: &mut Self, reason: String, report: bool| {
            if report {
                this.diags.push(
                    Diagnostic::warning(format!(
                        "memory `{name}` does not fit a block RAM of `{}`: {reason}",
                        this.device.name
                    ))
                    .with_code(NO_BLOCK_RAM)
                    .with_span(span)
                    .with_note(format!("it will be built from {fallback_style}")),
                );
            }
            this.report.bram_fallbacks.push(BramFallback {
                memory: name.clone(),
                reason,
                // Filled in by `lower_memory`, which runs next and says
                // what was really built.
                style: "a memory",
                primitive: None,
                cells: 0,
                built: false,
            });
            None::<Vec<CellId>>
        };

        if refused {
            return give_up(
                self,
                format!("`ram_style = \"{style}\"` asks for logic"),
                false,
            );
        }
        if width == 0 || depth == 0 {
            return give_up(self, "it is empty".to_owned(), false);
        }
        let bits = u64::from(width) * depth;
        if !forced && bits < self.options.min_bram_bits {
            return give_up(
                self,
                format!(
                    "{bits} bits is below the {} bit threshold",
                    self.options.min_bram_bits
                ),
                false,
            );
        }
        if self.device.block_rams.is_empty() {
            return give_up(
                self,
                format!("`{}` has no block RAM", self.device.name),
                forced,
            );
        }

        // The ports of this memory, in cell order.
        let mut reads: Vec<CellId> = Vec::new();
        let mut writes: Vec<CellId> = Vec::new();
        for (id, cell) in module.cells.iter() {
            match &cell.kind {
                CellKind::MemRdPort { mem: m, .. } if *m == mem => reads.push(id),
                CellKind::MemWrPort { mem: m, .. } if *m == mem => writes.push(id),
                _ => {}
            }
        }
        if reads.is_empty() && writes.is_empty() {
            return give_up(self, "it has no ports".to_owned(), false);
        }
        // A block RAM reads on a clock edge. Giving one a read port that
        // has no clock would leave the block's clock pin unconnected and
        // the netlist reading a cycle late: the honest answer is that
        // this memory is distributed RAM or flip-flops, which is exactly
        // where the fallback puts it.
        if reads.iter().any(|id| {
            matches!(
                module.cells[*id].kind,
                CellKind::MemRdPort { clocked: false, .. }
            )
        }) {
            return give_up(
                self,
                "a block RAM reads on a clock edge and this memory has an asynchronous read \
                 port"
                    .to_owned(),
                forced,
            );
        }

        let Some((shape_index, mode, wide, deep)) = self.choose_bram(width, depth) else {
            return give_up(
                self,
                format!(
                    "no width mode of `{}` fits {depth}x{width}",
                    self.device.name
                ),
                forced,
            );
        };
        let shape = &self.device.block_rams[shape_index];
        // One block, every port of the memory on a port of it. When that
        // does not fit, the answer a real flow gives a register file is
        // duplication: one copy of the contents per read port, each with
        // one read and one write port of its own, all written together
        // from the one write port. It costs read ports, not silicon
        // behaviour, so it is only right when there is at most one
        // writer — two writers would need the copies kept in step, which
        // the blocks cannot do between them. A ROM (no writer at all) is
        // the easy case: every copy holds the same initial contents and
        // nothing ever changes them.
        let groups = match allocate_ports(shape, reads.len(), writes.len()) {
            Some((read_slots, write_slots)) => vec![BramCopy {
                reads: reads.clone(),
                read_slots,
                write_slots,
                index: 0,
            }],
            None if writes.len() <= 1 && reads.len() > 1 => {
                match allocate_ports(shape, 1, writes.len()) {
                    Some((read_slots, write_slots)) => reads
                        .iter()
                        .enumerate()
                        .map(|(index, id)| BramCopy {
                            reads: vec![*id],
                            read_slots: read_slots.clone(),
                            write_slots: write_slots.clone(),
                            index,
                        })
                        .collect(),
                    None => {
                        let reason = port_shortfall(shape, reads.len(), writes.len());
                        return give_up(self, reason, true);
                    }
                }
            }
            None => {
                let reason = port_shortfall(shape, reads.len(), writes.len());
                return give_up(self, reason, true);
            }
        };
        let copies = u32::try_from(groups.len()).unwrap_or(1);

        let shape = shape.clone();
        let layout = shape.layout_for_mode(mode);
        if copies > 1 {
            let how = if writes.is_empty() {
                "each holding the same initial contents"
            } else {
                "written together"
            };
            self.note(format!(
                "memory `{name}` has {} read ports and one `{}` serves one, so its contents are \
                 held in {copies} copies {how}",
                reads.len(),
                shape.name
            ));
        }
        let initialise = self.bram_contents(module, mem, &shape, &layout, mode);
        for copy in &groups {
            let plan = BramPlan {
                mode,
                wide,
                deep,
                read_slots: copy.read_slots.clone(),
                write_slots: copy.write_slots.clone(),
                copy: copy.index,
                copies: groups.len(),
                layout: layout.clone(),
                initialise,
            };
            self.emit_bram(module, mem, &shape, &plan, &copy.reads, &writes);
        }
        self.report.block_rams.push(BramMapping {
            memory: name,
            primitive: shape.name.clone(),
            data_width: width,
            depth,
            mode,
            wide,
            deep,
            copies,
            initialised: initialise,
        });
        let mut replaced = reads;
        replaced.extend(writes);
        Some(replaced)
    }

    /// Decides whether the blocks of a memory mapped to `shape` in `mode`
    /// can carry its initial contents: true when the memory has some and
    /// the database says where they go. A memory with contents the blocks
    /// cannot hold is a warning, never a silence: its netlist's blocks
    /// start blank, so a ROM would read zeros on the board.
    fn bram_contents(
        &mut self,
        module: &Module,
        mem: MemoryId,
        shape: &BramShape,
        layout: &BramModeLayout,
        mode: (u32, u32),
    ) -> bool {
        let memory = &module.memories[mem];
        let Some(init) = &memory.init else {
            return false;
        };
        let (mode_width, mode_depth) = mode;
        let device = &self.device.name;
        let primitive = &shape.name;
        let problem = match (&shape.init_params, &layout.init) {
            _ if !shape.init_supported => Some(format!(
                "`{device}` declares no way to initialise `{primitive}` (no `init` flag)"
            )),
            (None, _) => Some(format!(
                "`{device}`'s database does not say which parameters hold `{primitive}`'s \
                 contents (no `init_params`)"
            )),
            (_, None) => Some(format!(
                "`{device}`'s database does not say where the words of `{primitive}`'s \
                 {mode_width}x{mode_depth} mode sit in its contents"
            )),
            (Some(params), Some(words)) => init_layout_problem(params, words, mode).map(|why| {
                format!(
                    "`{device}`'s layout for the {mode_width}x{mode_depth} mode of \
                     `{primitive}` does not fit its contents: {why}"
                )
            }),
        };
        let name = memory.name.as_str().to_owned();
        if let Some(problem) = problem {
            self.diags.push(
                Diagnostic::warning(format!(
                    "memory `{name}` has initial contents that its block RAMs will not hold: \
                     {problem}"
                ))
                .with_code(NO_BRAM_INIT)
                .with_span(memory.span)
                .with_note(format!(
                    "the `{primitive}` cells start blank, so on the device the memory reads \
                     zeros until it is written"
                )),
            );
            return false;
        }
        if init.iter().any(|word| !word.is_fully_known()) {
            self.note(format!(
                "memory `{name}` has `x` or `z` bits in its initial contents; the block RAMs \
                 hold them as 0"
            ));
        }
        true
    }

    /// Picks the shape and mode that uses fewest blocks, preferring the
    /// widest mode when several tie.
    fn choose_bram(&self, width: u32, depth: u64) -> Option<(usize, (u32, u32), u32, u32)> {
        let mut best: Option<(usize, (u32, u32), u32, u32)> = None;
        let mut best_key: Option<(u64, u32)> = None;
        for (index, shape) in self.device.block_rams.iter().enumerate() {
            for &(mode_width, mode_depth) in &shape.width_modes {
                if mode_width == 0 || mode_depth == 0 {
                    continue;
                }
                let wide = div_ceil_u32(width, mode_width);
                let Ok(deep) = u32::try_from(depth.div_ceil(u64::from(mode_depth))) else {
                    continue;
                };
                let blocks = u64::from(wide) * u64::from(deep);
                // Fewest blocks wins; the widest mode breaks the tie, so a
                // memory that fits either way keeps its addressing simple.
                let key = (blocks, u32::MAX - mode_width);
                if best_key.is_none_or(|b| key < b) {
                    best_key = Some(key);
                    best = Some((index, (mode_width, mode_depth), wide, deep));
                }
            }
        }
        best
    }

    /// Builds the blocks, the address decoding and the output muxing.
    fn emit_bram(
        &mut self,
        module: &mut Module,
        mem: MemoryId,
        shape: &BramShape,
        plan: &BramPlan,
        reads: &[CellId],
        writes: &[CellId],
    ) {
        let (mode, wide, deep) = (plan.mode, plan.wide, plan.deep);
        let memory = &module.memories[mem];
        let memory_name = memory.name.as_str().to_owned();
        // Two copies of one memory must not name their nets alike.
        let base = plan.tag(&memory_name);
        let span = memory.span;
        let width = memory.elem.width().unwrap_or(0);
        let size = memory.size;
        let (mode_width, mode_depth) = mode;
        // The memory's own address width decides the split: a block deeper
        // than the whole memory simply uses fewer of its address bits.
        let total_bits = addr_bits(size);
        let local_bits = addr_bits(u64::from(mode_depth)).min(total_bits);
        let high_bits = total_bits - local_bits;
        let prim_addr_bits = shape.addr_width();
        let mode_params: Vec<(Name, AttrValue)> = shape
            .params_for_mode(mode)
            .iter()
            .map(|(k, v)| (Name::new(k.clone()), v.clone()))
            .collect();
        let layout = &plan.layout;
        // The contents, when they go into the blocks: `bram_contents`
        // checked that the database describes where.
        let contents = match (&memory.init, &shape.init_params, &layout.init) {
            (Some(init), Some(params), Some(words)) if plan.initialise => {
                Some((init.clone(), params.clone(), words.clone()))
            }
            _ => None,
        };

        // One port description per read and write port of the memory,
        // resolved once so the per-block loop only wires things up.
        let read_ports: Vec<PortWiring> = reads
            .iter()
            .map(|id| self.port_wiring(module, *id, local_bits, high_bits, span))
            .collect();
        let write_ports: Vec<PortWiring> = writes
            .iter()
            .map(|id| self.port_wiring(module, *id, local_bits, high_bits, span))
            .collect();

        // The address a block sees does not depend on which block it is,
        // so it is built once per port.
        let read_addrs: Vec<ExprId> = read_ports
            .iter()
            .map(|port| self.block_addr(module, port, prim_addr_bits, layout, mode_depth, span))
            .collect();
        let write_addrs: Vec<ExprId> = write_ports
            .iter()
            .map(|port| self.block_addr(module, port, prim_addr_bits, layout, mode_depth, span))
            .collect();
        // How many data pins a block uses: as many as the mode is wide,
        // unless the mode spreads its bits, in which case up to the
        // highest pin it uses.
        let data_pins = layout
            .data_bits
            .iter()
            .max()
            .map_or(mode_width, |top| top + 1);

        // For each read port, the value read by each depth slice.
        let mut rows: Vec<Vec<ExprId>> = vec![Vec::new(); read_ports.len()];
        for d in 0..deep {
            // The address decoding is per port and per depth slice; the
            // blocks side by side that carry the width share it.
            let read_ens: Vec<ExprId> = read_ports
                .iter()
                .map(|port| self.block_enable(module, port, d, span))
                .collect();
            let write_ens: Vec<ExprId> = write_ports
                .iter()
                .map(|port| self.block_enable(module, port, d, span))
                .collect();
            let mut row_parts: Vec<Vec<ExprId>> = vec![Vec::new(); read_ports.len()];
            for w in 0..wide {
                let lo = w * mode_width;
                let hi = ((w + 1) * mode_width - 1).min(width.saturating_sub(1));
                let mut inputs: Vec<(Name, ExprId)> = Vec::new();
                let mut outputs: Vec<(Name, NetId)> = Vec::new();
                for (index, port) in read_ports.iter().enumerate() {
                    let Some(map) = plan
                        .read_slots
                        .get(index)
                        .and_then(|slot| shape.port_map.get(*slot))
                    else {
                        continue;
                    };
                    if let Some(name) = map.signal("addr") {
                        inputs.push((Name::new(name), read_addrs[index]));
                    }
                    if let (Some(name), Some(clk)) = (map.signal("clk"), port.clk) {
                        inputs.push((Name::new(name), clk));
                    }
                    if let Some(name) = map.signal("en") {
                        inputs.push((Name::new(name), read_ens[index]));
                    }
                    if let Some(name) = map.signal("ce") {
                        inputs.push((Name::new(name), read_ens[index]));
                    }
                    if let Some(name) = map.signal("dout") {
                        let net = add_net(
                            module,
                            &format!("{base}$rd{index}_w{w}_d{d}"),
                            Type::bits(data_pins),
                            span,
                        );
                        outputs.push((Name::new(name), net));
                        let value = net_expr(module, net, span);
                        let part = if layout.data_bits.is_empty() {
                            slice_expr(module, value, hi - lo, 0, span)
                        } else {
                            // Data bit `j` comes off the pin the mode puts
                            // it on; most significant first.
                            let bits: Vec<ExprId> = (0..=hi - lo)
                                .rev()
                                .map(|j| {
                                    let pin = layout.data_bit(j);
                                    slice_expr(module, value, pin, pin, span)
                                })
                                .collect();
                            if bits.len() == 1 {
                                bits[0]
                            } else {
                                expr(module, ExprKind::Concat(bits), span)
                            }
                        };
                        row_parts[index].push(part);
                    }
                }
                for (index, port) in write_ports.iter().enumerate() {
                    let Some(map) = plan
                        .write_slots
                        .get(index)
                        .and_then(|slot| shape.port_map.get(*slot))
                    else {
                        continue;
                    };
                    if let Some(name) = map.signal("addr") {
                        inputs.push((Name::new(name), write_addrs[index]));
                    }
                    if let (Some(name), Some(clk)) = (map.signal("clk"), port.clk) {
                        inputs.push((Name::new(name), clk));
                    }
                    if let Some(name) = map.signal("en") {
                        inputs.push((Name::new(name), write_ens[index]));
                    }
                    if let Some(name) = map.signal("we") {
                        // A family with byte write enables has a pin per
                        // byte; a memory written a whole word at a time
                        // drives every one of them alike. Leaving the
                        // upper bits off would write one byte and drop
                        // the rest, silently.
                        let bits = map.width("we");
                        let name = Name::new(name);
                        let value = repeat_bits(module, write_ens[index], bits, span);
                        inputs.push((name, value));
                    }
                    if let Some(name) = map.signal("ce") {
                        inputs.push((Name::new(name), write_ens[index]));
                    }
                    if let Some(name) = map.signal("din")
                        && let Some(data) = port.data
                    {
                        let value = if layout.data_bits.is_empty() {
                            let part = slice_expr(module, data, hi, lo, span);
                            self.resize(module, part, mode_width, span)
                        } else {
                            spread_data(module, data, layout, lo, hi, data_pins, span)
                        };
                        inputs.push((Name::new(name), value));
                    }
                }
                let cell = add_cell(
                    module,
                    &format!("{base}$ram_w{w}_d{d}"),
                    CellKind::Blackbox(Name::new(shape.name.clone())),
                    inputs,
                    outputs,
                    span,
                );
                for (key, value) in &mode_params {
                    module.cells[cell].params.set(key.clone(), value.clone());
                }
                if let Some((init, params, words)) = &contents {
                    // Every copy holds the whole contents; this block
                    // holds its width slice of its depth slice of them.
                    for (key, value) in block_contents(init, width, params, words, mode, w, d) {
                        module.cells[cell].params.set(key, value);
                    }
                }
                module.cells[cell].attrs.set("memory", memory_name.clone());
            }
            for (index, parts) in row_parts.into_iter().enumerate() {
                let mut parts = parts;
                parts.reverse();
                let value = if parts.len() == 1 {
                    parts[0]
                } else {
                    expr(module, ExprKind::Concat(parts), span)
                };
                rows[index].push(value);
            }
        }

        // Each read port's data net is the depth slices muxed on the high
        // address bits, registered when the port is clocked.
        for (index, port) in read_ports.iter().enumerate() {
            let Some(data_net) = port.out else {
                continue;
            };
            let parts = &rows[index];
            if parts.len() == 1 {
                let value = self.resize(module, parts[0], net_width(module, data_net), span);
                add_assign(module, data_net, value, span);
                continue;
            }
            let select = self.read_select(module, port, &base, index, span);
            let out = ReadOutput {
                base: &base,
                index,
                width: net_width(module, data_net),
                span,
            };
            let value = self.mux_tree(module, parts, &select, &out);
            add_assign(module, data_net, value, span);
        }

        module.memories[mem]
            .attrs
            .set("mapped_to", shape.name.clone());
    }

    /// The signals of one memory port, in the form the block wiring needs.
    fn port_wiring(
        &mut self,
        module: &mut Module,
        cell: CellId,
        local_bits: u32,
        high_bits: u32,
        span: Span,
    ) -> PortWiring {
        let cell = &module.cells[cell];
        let addr = cell.input("addr");
        let clk = cell.input("clk");
        let en = cell.input("en");
        let data = cell.input("data");
        let out = cell.output("data");
        // A port may carry fewer address bits than the memory needs; use
        // what it has rather than slicing past its end.
        let available = addr.map_or(0, |a| expr_width(module, a));
        let local = local_bits.min(available);
        let high_count = high_bits.min(available.saturating_sub(local));
        let low = match addr {
            Some(a) if local > 0 => Some(slice_expr(module, a, local - 1, 0, span)),
            _ => None,
        };
        let high = match addr {
            Some(a) if high_count > 0 => {
                Some(slice_expr(module, a, local + high_count - 1, local, span))
            }
            _ => None,
        };
        PortWiring {
            low,
            high,
            high_bits: high_count,
            clk,
            en,
            data,
            out,
        }
    }

    /// The address a block sees, zero-extended to the primitive's width.
    ///
    /// A mode whose word address starts above bit 0 of the address port
    /// ([`BramModeLayout::addr_low`]) gets the word address, as wide as
    /// the mode's depth needs, shifted up to there, with the bits below
    /// tied to the mode's pad.
    fn block_addr(
        &mut self,
        module: &mut Module,
        port: &PortWiring,
        prim_addr_bits: u32,
        layout: &BramModeLayout,
        mode_depth: u32,
        span: Span,
    ) -> ExprId {
        if layout.addr_low == 0 {
            return match port.low {
                Some(addr) => self.resize(module, addr, prim_addr_bits, span),
                None => const_expr(module, Const::zero(prim_addr_bits.max(1)), span),
            };
        }
        let word_bits = addr_bits(u64::from(mode_depth)).max(1);
        let word = match port.low {
            Some(addr) => self.resize(module, addr, word_bits, span),
            None => const_expr(module, Const::zero(word_bits), span),
        };
        let pad = layout
            .addr_pad
            .clone()
            .unwrap_or_else(|| Const::zero(layout.addr_low));
        let pad = const_expr(module, pad, span);
        let pad = self.resize(module, pad, layout.addr_low, span);
        let joined = expr(module, ExprKind::Concat(vec![word, pad]), span);
        self.resize(module, joined, prim_addr_bits, span)
    }

    /// The enable a block at depth slice `d` sees: the port's enable and
    /// the high address bits matching `d`.
    fn block_enable(
        &mut self,
        module: &mut Module,
        port: &PortWiring,
        d: u32,
        span: Span,
    ) -> ExprId {
        let enable = match port.en {
            Some(en) => en,
            None => const_expr(module, Const::ones(1), span),
        };
        let Some(high) = port.high else {
            return enable;
        };
        if port.high_bits == 0 {
            return enable;
        }
        let want = const_expr(module, Const::from_u64(u64::from(d), port.high_bits), span);
        let hit = add_net(module, "bram$sel", Type::bit(), span);
        add_cell(
            module,
            "bram$eq",
            CellKind::Eq,
            vec![(Name::new("a"), high), (Name::new("b"), want)],
            vec![(Name::new("y"), hit)],
            span,
        );
        let hit_expr = net_expr(module, hit, span);
        let gated = add_net(module, "bram$en", Type::bit(), span);
        add_cell(
            module,
            "bram$and",
            CellKind::And,
            vec![(Name::new("a"), enable), (Name::new("b"), hit_expr)],
            vec![(Name::new("y"), gated)],
            span,
        );
        net_expr(module, gated, span)
    }

    /// The select bits the output mux uses: the read port's high address
    /// bits, registered when the port is clocked so that the mux follows
    /// the data out of the block.
    fn read_select(
        &mut self,
        module: &mut Module,
        port: &PortWiring,
        base: &str,
        index: usize,
        span: Span,
    ) -> Vec<ExprId> {
        let Some(high) = port.high else {
            return Vec::new();
        };
        let high_bits = port.high_bits;
        let source = match port.clk {
            Some(clk) => {
                let q = add_net(
                    module,
                    &format!("{base}$rd{index}_sel"),
                    Type::bits(high_bits),
                    span,
                );
                let mut inputs = vec![(Name::new("clk"), clk), (Name::new("d"), high)];
                let has_enable = port.en.is_some();
                if let Some(en) = port.en {
                    inputs.push((Name::new("en"), en));
                }
                add_cell(
                    module,
                    &format!("{base}$rd{index}_selff"),
                    CellKind::Dff {
                        clk_pos: true,
                        has_enable,
                        reset: None,
                    },
                    inputs,
                    vec![(Name::new("q"), q)],
                    span,
                );
                net_expr(module, q, span)
            }
            None => high,
        };
        (0..high_bits)
            .map(|bit| slice_expr(module, source, bit, bit, span))
            .collect()
    }

    /// A balanced tree of `Mux` cells selecting one of `parts`.
    fn mux_tree(
        &mut self,
        module: &mut Module,
        parts: &[ExprId],
        select: &[ExprId],
        out: &ReadOutput<'_>,
    ) -> ExprId {
        let span = out.span;
        if parts.len() == 1 || select.is_empty() {
            return self.resize(module, parts[0], out.width, span);
        }
        let level = select.len() - 1;
        let half = 1usize << level;
        let low = self.mux_tree_part(module, parts, 0, half, &select[..level], out);
        let high = self.mux_tree_part(module, parts, half, parts.len(), &select[..level], out);
        let net = add_net(
            module,
            &format!("{}$rd{}_mux", out.base, out.index),
            Type::bits(out.width),
            span,
        );
        add_cell(
            module,
            &format!("{}$rd{}_muxcell", out.base, out.index),
            CellKind::Mux,
            vec![
                (Name::new("a"), low),
                (Name::new("b"), high),
                (Name::new("s"), select[level]),
            ],
            vec![(Name::new("y"), net)],
            span,
        );
        net_expr(module, net, span)
    }

    /// One half of [`Mapper::mux_tree`]; an absent half reads as `x`,
    /// which is what an out-of-range address gives.
    fn mux_tree_part(
        &mut self,
        module: &mut Module,
        parts: &[ExprId],
        start: usize,
        end: usize,
        select: &[ExprId],
        out: &ReadOutput<'_>,
    ) -> ExprId {
        if start >= parts.len() {
            return const_expr(module, Const::x(out.width), out.span);
        }
        let slice = &parts[start..end.min(parts.len())];
        self.mux_tree(module, slice, select, out)
    }

    /// Zero-extends or truncates `value` to `width`.
    fn resize(&mut self, module: &mut Module, value: ExprId, width: u32, span: Span) -> ExprId {
        if expr_width(module, value) == width {
            return value;
        }
        expr(
            module,
            ExprKind::Resize {
                expr: value,
                width,
                signed: false,
            },
            span,
        )
    }

    // --- memories that stay in logic ----------------------------------------

    /// The device's distributed RAM primitive and its shape, when it
    /// declares one Reticle can wire up.
    ///
    /// The shape is read off the port map rather than declared
    /// separately: the block is as wide as it has `dout` pins and as
    /// deep as its `raddr` pins address, which for an ECP5
    /// `TRELLIS_DPR16X4` is four bits wide and sixteen deep. A `lutram`
    /// line without those port names is a primitive the database records
    /// but cannot connect, and the fallback then uses flip-flops.
    fn lutram_shape(&self) -> Option<LutRamShape> {
        let bel = self.device.bel(BelRole::LutRam)?;
        if !bel.has_ports(&["wclk", "we", "waddr", "din", "raddr", "dout"]) {
            return None;
        }
        let width = u32::try_from(bel.port_names("dout").len()).ok()?;
        let addr_bits = u32::try_from(bel.port_names("raddr").len()).ok()?;
        if width == 0
            || addr_bits == 0
            || addr_bits >= 31
            || bel.port_names("din").len() != bel.port_names("dout").len()
            || bel.port_names("waddr").len() != bel.port_names("raddr").len()
        {
            return None;
        }
        Some(LutRamShape {
            bel: bel.clone(),
            width,
            addr_bits,
        })
    }

    /// Builds a memory the block RAM step turned down out of logic, and
    /// returns the port cells that became part of it.
    ///
    /// `None` leaves the memory alone, which only happens for the cases
    /// the report names: contents that flip-flops cannot be preloaded
    /// with, a write that is not clocked, two write ports on different
    /// clocks, or a memory so large that building it from logic would be
    /// a worse answer than saying it does not fit.
    fn lower_memory(&mut self, module: &mut Module, mem: MemoryId) -> Option<Vec<CellId>> {
        let memory = module.memories.get(mem)?;
        let name = memory.name.as_str().to_owned();
        let span = memory.span;
        let width = memory.elem.width().unwrap_or(0);
        let depth = memory.size;
        let has_init = memory.init.as_ref().is_some_and(|v| !v.is_empty());
        let init = memory.init.clone().unwrap_or_default();
        // A `ram_style` naming logic is an instruction, not a hint, so it
        // lifts the size limit the same way `ram_style = "block"` lifts
        // the block RAM threshold.
        let asked_for_logic = memory
            .attrs
            .get("ram_style")
            .and_then(AttrValue::as_str)
            .is_some_and(|style| {
                matches!(
                    style.to_ascii_lowercase().as_str(),
                    "distributed" | "logic" | "registers" | "ff" | "flops"
                )
            });

        let mut reads: Vec<CellId> = Vec::new();
        let mut writes: Vec<CellId> = Vec::new();
        for (id, cell) in module.cells.iter() {
            match &cell.kind {
                CellKind::MemRdPort { mem: m, .. } if *m == mem => reads.push(id),
                CellKind::MemWrPort { mem: m, .. } if *m == mem => writes.push(id),
                _ => {}
            }
        }
        // Nothing reads it, or it holds nothing: it has no effect, so it
        // goes away with its ports rather than being built.
        if reads.is_empty() || width == 0 || depth == 0 {
            self.record_fallback(&name, "logic", None, 0);
            let mut replaced = reads;
            replaced.extend(writes);
            return Some(replaced);
        }
        let Ok(rows_wanted) = usize::try_from(depth) else {
            self.decline_lowering(&name, "it has more words than this machine can count", span);
            return None;
        };
        if !asked_for_logic && u64::from(width) * depth > self.options.max_logic_bits {
            self.decline_lowering(
                &name,
                &format!(
                    "{} bits is over the {} bit limit for building a memory out of logic",
                    u64::from(width) * depth,
                    self.options.max_logic_bits
                ),
                span,
            );
            return None;
        }

        // The write side decides the storage: its clock drives every
        // flip-flop, and a distributed RAM has exactly one write port.
        let mut write_clk: Option<ExprId> = None;
        for id in &writes {
            let cell = &module.cells[*id];
            let Some(clk) = cell.input("clk") else {
                self.decline_lowering(
                    &name,
                    "a write port is not clocked, so it is a latch array rather than a register \
                     file",
                    span,
                );
                return None;
            };
            let same = write_clk.is_none_or(|first| {
                module.exprs.get(first).and_then(Expr::as_net)
                    == module.exprs.get(clk).and_then(Expr::as_net)
            });
            if !same {
                self.decline_lowering(
                    &name,
                    "its write ports are on different clocks, which one array of flip-flops \
                     cannot serve",
                    span,
                );
                return None;
            }
            write_clk = Some(clk);
        }
        if has_init && !writes.is_empty() {
            self.decline_lowering(
                &name,
                "it has initial contents, which flip-flops built from logic cannot be loaded \
                 with",
                span,
            );
            return None;
        }

        let shape = self.lutram_shape().filter(|_| writes.len() == 1);
        let plan = LogicPlan {
            name: name.clone(),
            span,
            width,
            rows_wanted,
            init,
        };
        let cells = match &shape {
            Some(shape) => self.emit_lutram(module, &plan, shape, &reads, &writes, write_clk),
            None => self.emit_registers(module, &plan, &reads, &writes, write_clk),
        };
        match &shape {
            Some(shape) => {
                self.record_fallback(
                    &name,
                    "distributed LUT RAM",
                    Some(shape.bel.name.clone()),
                    cells,
                );
            }
            None if writes.is_empty() => self.record_fallback(&name, "logic", None, 0),
            None => self.record_fallback(&name, "flip-flops", None, cells),
        }
        let mut replaced = reads;
        replaced.extend(writes);
        Some(replaced)
    }

    /// Records a memory the fallback could not build either, with the
    /// reason, so that the netlist check's complaint has an explanation
    /// beside it.
    fn decline_lowering(&mut self, name: &str, why: &str, span: Span) {
        self.diags.push(
            Diagnostic::warning(format!("memory `{name}` is left as a memory: {why}"))
                .with_code(NO_BLOCK_RAM)
                .with_span(span)
                .with_note(
                    "a place-and-route tool takes primitives only, so the design will not build \
                     as it stands",
                ),
        );
        if let Some(entry) = self
            .report
            .bram_fallbacks
            .iter_mut()
            .rev()
            .find(|f| f.memory == name)
        {
            entry.reason = format!("{}; {why}", entry.reason);
        }
        self.note(format!("memory `{name}` stays generic: {why}"));
    }

    /// The signals of one memory port, for the logic fallback.
    fn logic_port(&mut self, module: &Module, cell: CellId) -> LogicPort {
        let cell = &module.cells[cell];
        LogicPort {
            addr: cell.input("addr"),
            clk: cell.input("clk"),
            en: cell.input("en"),
            data: cell.input("data"),
            out: cell.output("data"),
        }
    }

    /// True when the port's address selects row `row`, and its enable is
    /// on.
    fn row_hit(
        &mut self,
        module: &mut Module,
        port: &LogicPort,
        row: u64,
        bits: u32,
        span: Span,
    ) -> ExprId {
        let enable = match port.en {
            Some(en) => en,
            None => const_expr(module, Const::ones(1), span),
        };
        if bits == 0 {
            return enable;
        }
        let Some(addr) = port.addr else { return enable };
        let want = const_expr(module, Const::from_u64(row, bits), span);
        let selected = slice_expr(module, addr, bits - 1, 0, span);
        let hit = add_net(module, "mem$sel", Type::bit(), span);
        add_cell(
            module,
            "mem$eq",
            CellKind::Eq,
            vec![(Name::new("a"), selected), (Name::new("b"), want)],
            vec![(Name::new("y"), hit)],
            span,
        );
        let hit = net_expr(module, hit, span);
        let gated = add_net(module, "mem$we", Type::bit(), span);
        add_cell(
            module,
            "mem$and",
            CellKind::And,
            vec![(Name::new("a"), enable), (Name::new("b"), hit)],
            vec![(Name::new("y"), gated)],
            span,
        );
        net_expr(module, gated, span)
    }

    /// Drives one read port's net with `value`, through an output
    /// register when the port is clocked.
    fn finish_read(
        &mut self,
        module: &mut Module,
        plan: &LogicPlan,
        port: &LogicPort,
        index: usize,
        value: ExprId,
    ) {
        let Some(out) = port.out else { return };
        let span = plan.span;
        let want = net_width(module, out);
        let value = self.resize(module, value, want, span);
        let Some(clk) = port.clk.filter(|_| port.out.is_some()) else {
            add_assign(module, out, value, span);
            return;
        };
        let mut inputs = vec![(Name::new("clk"), clk), (Name::new("d"), value)];
        let has_enable = port.en.is_some();
        if let Some(en) = port.en {
            inputs.push((Name::new("en"), en));
        }
        add_cell(
            module,
            &format!("{}$rd{index}_q", plan.name),
            CellKind::Dff {
                clk_pos: true,
                has_enable,
                reset: None,
            },
            inputs,
            vec![(Name::new("q"), out)],
            span,
        );
    }

    /// The multiplexer selecting one of `rows` on the low bits of a read
    /// port's address.
    fn row_mux(
        &mut self,
        module: &mut Module,
        plan: &LogicPlan,
        port: &LogicPort,
        index: usize,
        rows: &[ExprId],
        bits: u32,
    ) -> ExprId {
        let span = plan.span;
        if rows.len() == 1 || bits == 0 {
            return rows[0];
        }
        let select: Vec<ExprId> = match port.addr {
            Some(addr) => (0..bits)
                .map(|bit| slice_expr(module, addr, bit, bit, span))
                .collect(),
            None => Vec::new(),
        };
        let out = ReadOutput {
            base: &plan.name,
            index,
            width: plan.width,
            span,
        };
        self.mux_tree(module, rows, &select, &out)
    }

    /// One flip-flop array: a register per word, a decoded write enable
    /// and a read multiplexer.
    ///
    /// A memory with no write port at all is a ROM, and then every row
    /// is a constant from the memory's initial contents rather than a
    /// register, so nothing is stored and only the multiplexer is built.
    fn emit_registers(
        &mut self,
        module: &mut Module,
        plan: &LogicPlan,
        reads: &[CellId],
        writes: &[CellId],
        write_clk: Option<ExprId>,
    ) -> u32 {
        let span = plan.span;
        let bits = addr_bits(plan.rows_wanted as u64);
        let write_ports: Vec<LogicPort> = writes
            .iter()
            .map(|id| self.logic_port(module, *id))
            .collect();
        let read_ports: Vec<LogicPort> = reads
            .iter()
            .map(|id| self.logic_port(module, *id))
            .collect();

        let mut rows: Vec<ExprId> = Vec::with_capacity(plan.rows_wanted);
        let mut flops = 0u32;
        for row in 0..plan.rows_wanted {
            if write_ports.is_empty() {
                rows.push(plan.constant_row(module, row, span));
                continue;
            }
            let q = add_net(
                module,
                &format!("{}$w{row}", plan.name),
                Type::bits(plan.width),
                span,
            );
            let q_expr = net_expr(module, q, span);
            // The word a write leaves behind: the last port that hits
            // this row wins, as the last assignment of a process does.
            let mut value = q_expr;
            let mut enable: Option<ExprId> = None;
            for port in &write_ports {
                let hit = self.row_hit(module, port, row as u64, bits, span);
                if let Some(data) = port.data {
                    let data = self.resize(module, data, plan.width, span);
                    let next = add_net(
                        module,
                        &format!("{}$w{row}_d", plan.name),
                        Type::bits(plan.width),
                        span,
                    );
                    add_cell(
                        module,
                        &format!("{}$w{row}_mux", plan.name),
                        CellKind::Mux,
                        vec![
                            (Name::new("a"), value),
                            (Name::new("b"), data),
                            (Name::new("s"), hit),
                        ],
                        vec![(Name::new("y"), next)],
                        span,
                    );
                    value = net_expr(module, next, span);
                }
                enable = Some(match enable {
                    None => hit,
                    Some(previous) => {
                        let any = add_net(module, "mem$any", Type::bit(), span);
                        add_cell(
                            module,
                            "mem$or",
                            CellKind::Or,
                            vec![(Name::new("a"), previous), (Name::new("b"), hit)],
                            vec![(Name::new("y"), any)],
                            span,
                        );
                        net_expr(module, any, span)
                    }
                });
            }
            let mut inputs = vec![
                (
                    Name::new("clk"),
                    write_clk.unwrap_or_else(|| const_expr(module, Const::zero(1), span)),
                ),
                (Name::new("d"), value),
            ];
            let has_enable = enable.is_some();
            if let Some(enable) = enable {
                inputs.push((Name::new("en"), enable));
            }
            add_cell(
                module,
                &format!("{}$ff{row}", plan.name),
                CellKind::Dff {
                    clk_pos: true,
                    has_enable,
                    reset: None,
                },
                inputs,
                vec![(Name::new("q"), q)],
                span,
            );
            flops += plan.width;
            rows.push(q_expr);
        }
        for (index, port) in read_ports.iter().enumerate() {
            let value = self.row_mux(module, plan, port, index, &rows, bits);
            self.finish_read(module, plan, port, index, value);
        }
        flops
    }

    /// One array of the device's distributed RAM primitive per read
    /// port.
    ///
    /// The primitive has one write port and one asynchronous read port,
    /// so several readers means several copies of the contents, each
    /// written from the one write port — the same duplication a vendor
    /// flow applies, and the same one [`Mapper::block_ram`] applies to a
    /// register file.
    fn emit_lutram(
        &mut self,
        module: &mut Module,
        plan: &LogicPlan,
        shape: &LutRamShape,
        reads: &[CellId],
        writes: &[CellId],
        write_clk: Option<ExprId>,
    ) -> u32 {
        let span = plan.span;
        let write = self.logic_port(module, writes[0]);
        let read_ports: Vec<LogicPort> = reads
            .iter()
            .map(|id| self.logic_port(module, *id))
            .collect();
        let banks = div_ceil_u32(plan.width, shape.width);
        let total = addr_bits(plan.rows_wanted as u64);
        let low = shape.addr_bits.min(total);
        let high = total - low;
        let rows = 1usize << high;
        let waddr = write
            .addr
            .map(|addr| self.resize_low(module, addr, low, span));
        let clk = write_clk.unwrap_or_else(|| const_expr(module, Const::zero(1), span));

        let mut built = 0u32;
        for (index, port) in read_ports.iter().enumerate() {
            let raddr = port
                .addr
                .map(|addr| self.resize_low(module, addr, low, span));
            let mut row_values: Vec<ExprId> = Vec::with_capacity(rows);
            for row in 0..rows {
                let we = self.row_hit_high(module, &write, row as u64, high, low, span);
                let mut parts: Vec<ExprId> = Vec::with_capacity(banks as usize);
                for bank in 0..banks {
                    let lo = bank * shape.width;
                    let hi = ((bank + 1) * shape.width - 1).min(plan.width - 1);
                    let mut inputs: Vec<(Name, ExprId)> = Vec::new();
                    let mut outputs: Vec<(Name, NetId)> = Vec::new();
                    inputs.push((Name::new(shape.bel.port("wclk").unwrap_or("WCK")), clk));
                    inputs.push((Name::new(shape.bel.port("we").unwrap_or("WRE")), we));
                    for (bit, pin) in shape.bel.port_names("waddr").iter().enumerate() {
                        let value = self.addr_bit(module, waddr, bit, span);
                        inputs.push((Name::new(*pin), value));
                    }
                    for (bit, pin) in shape.bel.port_names("raddr").iter().enumerate() {
                        let value = self.addr_bit(module, raddr, bit, span);
                        inputs.push((Name::new(*pin), value));
                    }
                    let data = write.data.map(|data| {
                        let part = slice_expr(module, data, hi, lo, span);
                        self.resize(module, part, shape.width, span)
                    });
                    for (bit, pin) in shape.bel.port_names("din").iter().enumerate() {
                        let bit = u32::try_from(bit).unwrap_or(0);
                        let value = match data {
                            Some(data) => slice_expr(module, data, bit, bit, span),
                            None => const_expr(module, Const::zero(1), span),
                        };
                        inputs.push((Name::new(*pin), value));
                    }
                    let mut bits: Vec<ExprId> = Vec::new();
                    for (bit, pin) in shape.bel.port_names("dout").iter().enumerate() {
                        let net = add_net(
                            module,
                            &format!("{}$rd{index}_r{row}_b{bank}_{bit}", plan.name),
                            Type::bit(),
                            span,
                        );
                        outputs.push((Name::new(*pin), net));
                        bits.push(net_expr(module, net, span));
                    }
                    let cell = add_cell(
                        module,
                        &format!("{}$dpr{index}_{row}_{bank}", plan.name),
                        CellKind::Blackbox(Name::new(shape.bel.name.clone())),
                        inputs,
                        outputs,
                        span,
                    );
                    for (key, value) in &shape.bel.params {
                        module.cells[cell].params.set(key.clone(), value.clone());
                    }
                    module.cells[cell].attrs.set("memory", plan.name.clone());
                    built += 1;
                    bits.truncate(usize::try_from(hi - lo + 1).unwrap_or(0));
                    bits.reverse();
                    parts.push(expr(module, ExprKind::Concat(bits), span));
                }
                parts.reverse();
                let value = if parts.len() == 1 {
                    parts[0]
                } else {
                    expr(module, ExprKind::Concat(parts), span)
                };
                row_values.push(value);
            }
            let value = self.mux_high(module, plan, port, index, &row_values, (low, high));
            self.finish_read(module, plan, port, index, value);
        }
        built
    }

    /// Bit `bit` of an address that may be narrower than the pin list,
    /// or a zero when it is.
    fn addr_bit(
        &mut self,
        module: &mut Module,
        addr: Option<ExprId>,
        bit: usize,
        span: Span,
    ) -> ExprId {
        let bit = u32::try_from(bit).unwrap_or(0);
        match addr {
            Some(addr) if bit < expr_width(module, addr) => {
                slice_expr(module, addr, bit, bit, span)
            }
            _ => const_expr(module, Const::zero(1), span),
        }
    }

    /// The low `bits` of an address, or a zero when it has none.
    fn resize_low(&mut self, module: &mut Module, addr: ExprId, bits: u32, span: Span) -> ExprId {
        if bits == 0 {
            return const_expr(module, Const::zero(1), span);
        }
        let available = expr_width(module, addr);
        slice_expr(module, addr, bits.min(available) - 1, 0, span)
    }

    /// Like [`Mapper::row_hit`], but on the *high* address bits, which is
    /// what selects one bank of a distributed RAM array.
    fn row_hit_high(
        &mut self,
        module: &mut Module,
        port: &LogicPort,
        row: u64,
        high: u32,
        low: u32,
        span: Span,
    ) -> ExprId {
        let enable = match port.en {
            Some(en) => en,
            None => const_expr(module, Const::ones(1), span),
        };
        if high == 0 {
            return enable;
        }
        let Some(addr) = port.addr else { return enable };
        let available = expr_width(module, addr);
        if low + high > available {
            return enable;
        }
        let want = const_expr(module, Const::from_u64(row, high), span);
        let selected = slice_expr(module, addr, low + high - 1, low, span);
        let hit = add_net(module, "mem$bank", Type::bit(), span);
        add_cell(
            module,
            "mem$eq",
            CellKind::Eq,
            vec![(Name::new("a"), selected), (Name::new("b"), want)],
            vec![(Name::new("y"), hit)],
            span,
        );
        let hit = net_expr(module, hit, span);
        let gated = add_net(module, "mem$we", Type::bit(), span);
        add_cell(
            module,
            "mem$and",
            CellKind::And,
            vec![(Name::new("a"), enable), (Name::new("b"), hit)],
            vec![(Name::new("y"), gated)],
            span,
        );
        net_expr(module, gated, span)
    }

    /// The multiplexer over the banks of a distributed RAM array,
    /// selected by the address bits above the ones one block carries.
    fn mux_high(
        &mut self,
        module: &mut Module,
        plan: &LogicPlan,
        port: &LogicPort,
        index: usize,
        rows: &[ExprId],
        split: (u32, u32),
    ) -> ExprId {
        let (low, high) = split;
        let span = plan.span;
        if rows.len() == 1 || high == 0 {
            return rows[0];
        }
        let select: Vec<ExprId> = match port.addr {
            Some(addr) if low + high <= expr_width(module, addr) => (0..high)
                .map(|bit| slice_expr(module, addr, low + bit, low + bit, span))
                .collect(),
            _ => Vec::new(),
        };
        let out = ReadOutput {
            base: &plan.name,
            index,
            width: plan.width,
            span,
        };
        self.mux_tree(module, rows, &select, &out)
    }

    // --- DSP ---------------------------------------------------------------

    fn dsps(&mut self, module: &mut Module) {
        if self.device.dsps.is_empty() {
            return;
        }
        let mut removed: Vec<CellId> = Vec::new();
        let muls: Vec<CellId> = module
            .cells
            .iter()
            .filter(|(_, c)| c.kind == CellKind::Mul)
            .map(|(id, _)| id)
            .collect();
        for mul in muls {
            if let Some(used) = self.dsp(module, mul) {
                removed.extend(used);
            }
        }
        if !removed.is_empty() {
            module.cells.retain(|id, _| !removed.contains(&id));
        }
    }

    fn dsp(&mut self, module: &mut Module, mul: CellId) -> Option<Vec<CellId>> {
        let cell = module.cells.get(mul)?;
        let name = cell.name.as_str().to_owned();
        let span = cell.span;
        let a = cell.input("a")?;
        let b = cell.input("b")?;
        let y = cell.output("y")?;
        let (a_width, b_width) = (expr_width(module, a), expr_width(module, b));
        let y_width = net_width(module, y);

        // Fold a following adder in when the device has an accumulator and
        // the multiplier's result goes nowhere else.
        let uses = net_use_counts(module);
        let single_use = uses.get(y.index()).copied().unwrap_or(0) == 1;
        let add = if single_use {
            self.adder_fed_by(module, y)
        } else {
            None
        };

        let mut result_net = y;
        let mut result_width = y_width;
        let mut addend = None;
        let mut fused = None;
        if let Some((add_id, other)) = add {
            let add_cell = &module.cells[add_id];
            if let Some(out) = add_cell.output("y") {
                result_net = out;
                result_width = net_width(module, out);
                addend = Some(other);
                fused = Some(add_id);
            }
        }

        let shape = self.device.dsps.iter().find(|shape| {
            shape.fits_multiply(a_width, b_width, result_width)
                && (addend.is_none() || shape.has_accumulator)
                && shape.port("a").is_some()
                && shape.port("b").is_some()
                && shape.port("p").is_some()
        });
        let Some(shape) = shape.cloned() else {
            self.note(format!(
                "cell `{name}` ({a_width}x{b_width} -> {result_width}) fits no DSP block of `{}`",
                self.device.name
            ));
            return None;
        };
        if addend.is_some() && shape.port("c").is_none() {
            // The block accumulates but the database does not say through
            // which port, so map the multiplier alone.
            addend = None;
            fused = None;
            result_net = y;
            result_width = y_width;
        }

        let accumulate = addend.is_some_and(|_| self.feeds_back(module, result_net));
        let mut inputs = Vec::new();
        let a_value = self.resize(module, a, shape.a_width, span);
        let b_value = self.resize(module, b, shape.b_width, span);
        inputs.push((Name::new(shape.port("a").unwrap_or("a")), a_value));
        inputs.push((Name::new(shape.port("b").unwrap_or("b")), b_value));
        if let (Some(addend), Some(port)) = (addend, shape.port("c")) {
            let value = self.resize(module, addend, shape.p_width, span);
            inputs.push((Name::new(port), value));
        }
        let product = add_net(
            module,
            &format!("{name}$p"),
            Type::bits(shape.p_width),
            span,
        );
        let cell = add_cell(
            module,
            &format!("{name}$dsp"),
            CellKind::Blackbox(Name::new(shape.name.clone())),
            inputs,
            vec![(Name::new(shape.port("p").unwrap_or("p")), product)],
            span,
        );
        module.cells[cell].params.set("A_WIDTH", i64::from(a_width));
        module.cells[cell].params.set("B_WIDTH", i64::from(b_width));
        let value = net_expr(module, product, span);
        let sliced = slice_expr(module, value, result_width - 1, 0, span);
        add_assign(module, result_net, sliced, span);

        self.report.dsps.push(DspMapping {
            cell: name,
            primitive: shape.name.clone(),
            a_width,
            b_width,
            p_width: result_width,
            multiply_add: fused.is_some(),
            accumulate,
        });
        let mut removed = vec![mul];
        removed.extend(fused);
        Some(removed)
    }

    /// The `Add` cell whose only other operand is not `net`, when `net`
    /// feeds exactly one adder input directly.
    fn adder_fed_by(&self, module: &Module, net: NetId) -> Option<(CellId, ExprId)> {
        for (id, cell) in module.cells.iter() {
            if cell.kind != CellKind::Add {
                continue;
            }
            let (Some(a), Some(b)) = (cell.input("a"), cell.input("b")) else {
                continue;
            };
            let a_net = module.exprs.get(a).and_then(Expr::as_net);
            let b_net = module.exprs.get(b).and_then(Expr::as_net);
            if a_net == Some(net) {
                return Some((id, b));
            }
            if b_net == Some(net) {
                return Some((id, a));
            }
        }
        None
    }

    /// True when `net` comes back into the design through a flip-flop,
    /// which is what makes an adder an accumulator.
    fn feeds_back(&self, module: &Module, net: NetId) -> bool {
        module.cells.iter().any(|(_, cell)| {
            matches!(cell.kind, CellKind::Dff { .. })
                && cell
                    .input("d")
                    .and_then(|d| module.exprs.get(d))
                    .and_then(Expr::as_net)
                    == Some(net)
        })
    }

    // --- carry chains -------------------------------------------------------

    fn carry_chains(&mut self, module: &mut Module) {
        let Some(bel) = self.device.bel(BelRole::Carry).cloned() else {
            return;
        };
        if !bel.has_ports(&["ci", "i0", "i1", "co"]) {
            let adders = module
                .cells
                .iter()
                .filter(|(_, c)| c.kind == CellKind::Add)
                .count();
            if adders > 0 {
                self.note(format!(
                    "`{}` describes the carry element `{}` without a (ci, i0, i1, co) port map, so {adders} adder(s) stay generic",
                    self.device.name, bel.name
                ));
            }
            return;
        }
        let adders: Vec<CellId> = module
            .cells
            .iter()
            .filter(|(_, c)| c.kind == CellKind::Add)
            .map(|(id, _)| id)
            .collect();
        let mut removed = Vec::new();
        for add in adders {
            let cell = &module.cells[add];
            let Some(y) = cell.output("y") else {
                continue;
            };
            let width = net_width(module, y);
            if width < self.options.min_carry_width {
                continue;
            }
            let (Some(a), Some(b)) = (cell.input("a"), cell.input("b")) else {
                continue;
            };
            let name = cell.name.as_str().to_owned();
            let span = cell.span;
            self.emit_carry(module, &bel, &name, a, b, y, width, span);
            self.report.carry_chains.push(CarryMapping {
                cell: name,
                primitive: bel.name.clone(),
                width,
                primitives: width.saturating_sub(1),
            });
            removed.push(add);
        }
        if !removed.is_empty() {
            module.cells.retain(|id, _| !removed.contains(&id));
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn emit_carry(
        &mut self,
        module: &mut Module,
        bel: &BelKind,
        name: &str,
        a: ExprId,
        b: ExprId,
        y: NetId,
        width: u32,
        span: Span,
    ) {
        let (ci, i0, i1, co) = (
            bel.port("ci").unwrap_or("ci").to_owned(),
            bel.port("i0").unwrap_or("i0").to_owned(),
            bel.port("i1").unwrap_or("i1").to_owned(),
            bel.port("co").unwrap_or("co").to_owned(),
        );
        let mut carry = const_expr(module, Const::zero(1), span);
        let mut sums = Vec::with_capacity(usize::try_from(width).unwrap_or(0));
        for bit in 0..width {
            let a_bit = slice_expr(module, a, bit, bit, span);
            let b_bit = slice_expr(module, b, bit, bit, span);
            // sum = a ^ b ^ carry, in two XOR cells the LUT mapper folds.
            let half = add_net(module, &format!("{name}$h{bit}"), Type::bit(), span);
            add_cell(
                module,
                &format!("{name}$xor{bit}a"),
                CellKind::Xor,
                vec![(Name::new("a"), a_bit), (Name::new("b"), b_bit)],
                vec![(Name::new("y"), half)],
                span,
            );
            let half_expr = net_expr(module, half, span);
            let sum = add_net(module, &format!("{name}$s{bit}"), Type::bit(), span);
            add_cell(
                module,
                &format!("{name}$xor{bit}b"),
                CellKind::Xor,
                vec![(Name::new("a"), half_expr), (Name::new("b"), carry)],
                vec![(Name::new("y"), sum)],
                span,
            );
            sums.push(net_expr(module, sum, span));
            if bit + 1 < width {
                let next = add_net(module, &format!("{name}$c{bit}"), Type::bit(), span);
                add_cell(
                    module,
                    &format!("{name}$carry{bit}"),
                    CellKind::Blackbox(Name::new(bel.name.clone())),
                    vec![
                        (Name::new(ci.clone()), carry),
                        (Name::new(i0.clone()), a_bit),
                        (Name::new(i1.clone()), b_bit),
                    ],
                    vec![(Name::new(co.clone()), next)],
                    span,
                );
                carry = net_expr(module, next, span);
            }
        }
        sums.reverse();
        let value = expr(module, ExprKind::Concat(sums), span);
        add_assign(module, y, value, span);
    }

    // --- IO buffers ---------------------------------------------------------

    fn io_buffers(&mut self, module: &mut Module) {
        let Some(bel) = self.device.bel(BelRole::Io).cloned() else {
            self.note(format!(
                "`{}` describes no IO buffer, so the ports are left bare",
                self.device.name
            ));
            return;
        };
        if !bel.has_ports(&["pad"]) {
            self.note(format!(
                "the IO buffer `{}` of `{}` has no `pad` port, so the ports are left bare",
                bel.name, self.device.name
            ));
            return;
        }
        for index in 0..module.ports.len() {
            self.io_buffer(module, index);
        }
    }

    fn io_buffer(&mut self, module: &mut Module, index: usize) {
        let port = &module.ports[index];
        let name = port.name.as_str().to_owned();
        let dir = port.dir;
        // Which buffer serves this direction. A family with one
        // configurable buffer answers the same primitive every time; a
        // family with one primitive per direction (Xilinx's IBUF, OBUF
        // and IOBUF) answers a different one, with its own pin names.
        let direction = match dir {
            PortDir::In => "in",
            PortDir::Out => "out",
            PortDir::InOut => "inout",
        };
        let Some(bel) = self.device.io_bel(direction).cloned() else {
            self.note(format!(
                "`{}` describes no IO buffer for an `{direction}` port, so `{name}` is left bare",
                self.device.name
            ));
            return;
        };
        if !bel.has_ports(&["pad"]) {
            self.note(format!(
                "the IO buffer `{}` of `{}` has no `pad` port, so `{name}` is left bare",
                bel.name, self.device.name
            ));
            return;
        }
        let bel = &bel;
        let port = &module.ports[index];
        let core = port.net;
        let span = port.span;
        let Some(width) = module.nets.get(core).and_then(|n| n.ty.width()) else {
            self.note(format!(
                "port `{name}` is not a bit vector and gets no IO buffer"
            ));
            return;
        };
        if width == 0 {
            return;
        }
        let assignment = self.constraints.pin_of(&name, None);
        let io = assignment.map(|a| a.io.clone()).unwrap_or_default();
        let pin = assignment.map(|a| a.pin.clone()).filter(|p| !p.is_empty());
        // A double-data-rate register or a delay is a property of the
        // whole port, however it was stated: on the port, or on any one
        // of its bits.
        let stated = |pick: &dyn Fn(&IoAttrs) -> bool| {
            self.constraints
                .pins
                .iter()
                .filter(|p| p.port == name)
                .find(|p| pick(&p.io))
                .map(|p| p.io.clone())
        };
        let ddr_clock = stated(&|io| io.ddr.is_some()).and_then(|io| io.ddr);
        let delay_steps = stated(&|io| io.delay.is_some()).and_then(|io| io.delay);
        let ddr =
            ddr_clock.and_then(|clock| self.ddr_plan(module, bel, &name, dir, width, &clock, span));
        let delay = delay_steps.and_then(|steps| {
            // Zero steps is how a parameterised block says "no delay": it
            // has no other way to leave the attribute out. So it builds no
            // element and warns about nothing, rather than setting a delay
            // element to zero on every pin, or, on a family with none,
            // warning that a delay nobody wanted is not applied.
            if steps == 0 {
                return None;
            }
            if matches!(ddr, Some(DdrPlan::InBuffer { .. })) {
                self.io_warning(
                    &name,
                    span,
                    "a delay cannot sit between a pad and the IO buffer that registers it, so \
                     it is not applied",
                );
                return None;
            }
            self.delay_plan(&name, steps, span)
        });
        let pins = if ddr.is_some() { width / 2 } else { width };

        let pad = add_net(module, &format!("{name}$pad"), Type::bits(pins), span);
        module.ports[index].net = pad;
        let pad_value = net_expr(module, pad, span);
        let core_value = net_expr(module, core, span);

        let dir_key = match (dir, &ddr) {
            (PortDir::In, Some(DdrPlan::InBuffer { .. })) => "ddr_in",
            (PortDir::Out, Some(DdrPlan::InBuffer { .. })) => "ddr_out",
            (PortDir::In, _) => "in",
            (PortDir::Out, _) => "out",
            (PortDir::InOut, _) => "inout",
        };
        let site = IoSite {
            port: &name,
            pin: pin.as_deref(),
            io: &io,
            conditions: dir_key,
            span,
        };
        // What the fabric sees, least significant first: the rising-edge
        // half, then the falling-edge half for a DDR port.
        let mut low_bits = Vec::new();
        let mut high_bits = Vec::new();
        let mut pad_bits = Vec::new();
        for bit in 0..pins {
            let pad_bit = slice_expr(module, pad_value, bit, bit, span);
            let pad_port = Name::new(bel.port("pad").unwrap_or("pad"));
            match (dir, &ddr) {
                (PortDir::In, Some(DdrPlan::InBuffer { clk })) => {
                    let clk = net_expr(module, *clk, span);
                    let lo = add_net(module, &format!("{name}$in{bit}"), Type::bit(), span);
                    let hi = add_net(module, &format!("{name}$in{bit}_n"), Type::bit(), span);
                    let mut inputs = vec![(pad_port, pad_bit)];
                    inputs.push((Name::new(bel.port("iclk").unwrap_or("iclk")), clk));
                    if let Some(ce) = bel.port("ce") {
                        let one = const_expr(module, Const::ones(1), span);
                        inputs.push((Name::new(ce), one));
                    }
                    let outputs = vec![
                        (Name::new(bel.port("din").unwrap_or("din")), lo),
                        (Name::new(bel.port("din1").unwrap_or("din1")), hi),
                    ];
                    self.io_cell(module, bel, &site, bit, inputs, outputs);
                    low_bits.push(net_expr(module, lo, span));
                    high_bits.push(net_expr(module, hi, span));
                }
                (PortDir::Out, Some(DdrPlan::InBuffer { clk })) => {
                    let clk = net_expr(module, *clk, span);
                    let net = add_net(module, &format!("{name}$pin{bit}"), Type::bit(), span);
                    let lo = slice_expr(module, core_value, bit, bit, span);
                    let hi = slice_expr(module, core_value, bit + pins, bit + pins, span);
                    let mut inputs = vec![
                        (Name::new(bel.port("dout").unwrap_or("dout")), lo),
                        (Name::new(bel.port("dout1").unwrap_or("dout1")), hi),
                        (Name::new(bel.port("oclk").unwrap_or("oclk")), clk),
                    ];
                    if let Some(ce) = bel.port("ce") {
                        let one = const_expr(module, Const::ones(1), span);
                        inputs.push((Name::new(ce), one));
                    }
                    self.io_cell(module, bel, &site, bit, inputs, vec![(pad_port, net)]);
                    pad_bits.push(net_expr(module, net, span));
                }
                (PortDir::In, beside) => {
                    let mut outputs = Vec::new();
                    let mut from_pad = None;
                    if let Some(port_name) = bel.port("din") {
                        let net = add_net(module, &format!("{name}$in{bit}"), Type::bit(), span);
                        outputs.push((Name::new(port_name), net));
                        from_pad = Some(net_expr(module, net, span));
                    }
                    self.io_cell(module, bel, &site, bit, vec![(pad_port, pad_bit)], outputs);
                    let Some(value) = from_pad else { continue };
                    let value = match &delay {
                        Some(plan) => self.emit_delay(module, plan, value, &name, bit, span),
                        None => value,
                    };
                    match beside {
                        Some(DdrPlan::Beside { bel: reg, clk }) => {
                            let (lo, hi) =
                                self.emit_ddr_in(module, reg, *clk, value, &name, bit, span);
                            low_bits.push(lo);
                            high_bits.push(hi);
                        }
                        _ => low_bits.push(value),
                    }
                }
                (PortDir::Out | PortDir::InOut, beside) => {
                    let net = add_net(module, &format!("{name}$pin{bit}"), Type::bit(), span);
                    pad_bits.push(net_expr(module, net, span));
                    let mut value = match beside {
                        Some(DdrPlan::Beside { bel: reg, clk }) => {
                            let lo = slice_expr(module, core_value, bit, bit, span);
                            let hi = slice_expr(module, core_value, bit + pins, bit + pins, span);
                            self.emit_ddr_out(module, reg, *clk, (lo, hi), &name, bit, span)
                        }
                        _ => slice_expr(module, core_value, bit, bit, span),
                    };
                    if let Some(plan) = &delay {
                        value = self.emit_delay(module, plan, value, &name, bit, span);
                    }
                    let mut inputs = Vec::new();
                    if let Some(port_name) = bel.port("dout") {
                        inputs.push((Name::new(port_name), value));
                    }
                    if dir == PortDir::InOut
                        && let Some(port_name) = bel.port("oe")
                    {
                        let one = const_expr(module, Const::ones(1), span);
                        inputs.push((Name::new(port_name), one));
                    }
                    self.io_cell(module, bel, &site, bit, inputs, vec![(pad_port, net)]);
                }
            }
        }
        let mut fabric = low_bits;
        fabric.extend(high_bits);
        if !fabric.is_empty() {
            fabric.reverse();
            let value = if fabric.len() == 1 {
                fabric[0]
            } else {
                expr(module, ExprKind::Concat(fabric), span)
            };
            add_assign(module, core, value, span);
        }
        if !pad_bits.is_empty() {
            pad_bits.reverse();
            let value = if pad_bits.len() == 1 {
                pad_bits[0]
            } else {
                expr(module, ExprKind::Concat(pad_bits), span)
            };
            add_assign(module, pad, value, span);
        }
        if dir == PortDir::InOut {
            self.diags.push(
                Diagnostic::warning(format!(
                    "inout port `{name}` gets an output-only buffer"
                ))
                .with_code(PARTIAL_IO)
                .with_span(span)
                .with_note(
                    "the output enable is tied active and the input path is left unconnected: Reticle does not infer a tri-state enable for a port yet",
                ),
            );
        }
        let ddr_report = ddr.as_ref().map(|plan| match plan {
            DdrPlan::InBuffer { clk } => {
                (module.nets[*clk].name.as_str().to_owned(), bel.name.clone())
            }
            DdrPlan::Beside { bel: reg, clk } => {
                (module.nets[*clk].name.as_str().to_owned(), reg.name.clone())
            }
        });
        self.report.io_buffers.push(IoMapping {
            port: name,
            primitive: bel.name.clone(),
            bits: pins,
            pin,
            io_standard: io.io_standard,
            ddr: ddr_report,
            delay: delay.map(|plan| (plan.steps, plan.bel.name.clone())),
        });
    }

    /// One IO buffer cell, with the parameters its direction and options
    /// select and the constraints that produced it as attributes.
    fn io_cell(
        &mut self,
        module: &mut Module,
        bel: &BelKind,
        site: &IoSite<'_>,
        bit: u32,
        inputs: Vec<(Name, ExprId)>,
        outputs: Vec<(Name, NetId)>,
    ) -> CellId {
        let cell = add_cell(
            module,
            &format!("{}$io{bit}", site.port),
            CellKind::Blackbox(Name::new(bel.name.clone())),
            inputs,
            outputs,
            site.span,
        );
        for (key, value) in &bel.params {
            module.cells[cell].params.set(key.clone(), value.clone());
        }
        for (key, value) in bel.params_when(site.conditions) {
            module.cells[cell].params.set(key.clone(), value.clone());
        }
        let io = site.io;
        if io.pullup == Some(true) {
            for (key, value) in bel.params_when("pullup") {
                module.cells[cell].params.set(key.clone(), value.clone());
            }
        }
        let attrs = &mut module.cells[cell].attrs;
        attrs.set("port", site.port.to_owned());
        if let Some(pin) = site.pin {
            attrs.set("pin", pin.to_owned());
        }
        if let Some(standard) = &io.io_standard {
            attrs.set("io_standard", standard.clone());
        }
        if let Some(drive) = io.drive {
            attrs.set("drive", i64::from(drive));
        }
        if let Some(slew) = &io.slew {
            attrs.set("slew", slew.clone());
        }
        if let Some(pullup) = io.pullup {
            attrs.set("pullup", i64::from(pullup));
        }
        cell
    }

    /// A warning about an IO option that could not be honoured.
    fn io_warning(&mut self, port: &str, span: Span, why: &str) {
        self.diags.push(
            Diagnostic::warning(format!("port `{port}`: {why}"))
                .with_code(NO_IO_REGISTER)
                .with_span(span),
        );
        self.note(format!("port `{port}`: {why}"));
    }

    /// How a double-data-rate port is built on this device, or `None`
    /// (with an error saying why) when it cannot be, in which case the
    /// port is buffered as an ordinary one.
    #[allow(clippy::too_many_arguments)]
    fn ddr_plan(
        &mut self,
        module: &Module,
        bel: &BelKind,
        port: &str,
        dir: PortDir,
        width: u32,
        clock: &str,
        span: Span,
    ) -> Option<DdrPlan> {
        let refuse = |this: &mut Self, why: String| {
            this.diags.push(
                Diagnostic::error(format!(
                    "port `{port}` asks for double-data-rate registers, but {why}"
                ))
                .with_code(NO_IO_REGISTER)
                .with_span(span)
                .with_note("it is buffered as an ordinary port, one pin per bit"),
            );
            this.note(format!("port `{port}` is not double data rate: {why}"));
            None
        };
        if dir == PortDir::InOut {
            return refuse(self, "a bidirectional one is not supported".to_owned());
        }
        if !width.is_multiple_of(2) {
            return refuse(
                self,
                format!(
                    "it is {width} bits wide, and a DDR port carries two bits per pin: the \
                     rising-edge half, then the falling-edge half"
                ),
            );
        }
        let Some(clk) = module.net_by_name(clock) else {
            return refuse(self, format!("there is no clock net `{clock}`"));
        };
        let (own, conditions, beside_role, beside_ports): (&[&str], &str, BelRole, &[&str]) =
            if dir == PortDir::In {
                (
                    &["din", "din1", "iclk"],
                    "ddr_in",
                    BelRole::DdrIn,
                    &["d", "clk", "q0", "q1"],
                )
            } else {
                (
                    &["dout", "dout1", "oclk"],
                    "ddr_out",
                    BelRole::DdrOut,
                    &["d0", "d1", "clk", "q"],
                )
            };
        if bel.has_ports(own) && !bel.params_when(conditions).is_empty() {
            return Some(DdrPlan::InBuffer { clk });
        }
        if let Some(reg) = self.device.bel(beside_role)
            && reg.has_ports(beside_ports)
        {
            return Some(DdrPlan::Beside {
                bel: reg.clone(),
                clk,
            });
        }
        refuse(
            self,
            format!(
                "`{}` declares neither an IO buffer that registers both edges nor a `{}` register",
                self.device.name,
                beside_role.keyword()
            ),
        )
    }

    /// The delay element for a port, or `None` (with a warning) when the
    /// device has none or cannot delay that far.
    fn delay_plan(&mut self, port: &str, steps: u32, span: Span) -> Option<DelayPlan> {
        let Some(bel) = self.device.bel(BelRole::IoDelay).cloned() else {
            self.io_warning(
                port,
                span,
                &format!(
                    "`{}` has no programmable IO delay, so the {steps}-step delay asked for is \
                     not applied",
                    self.device.name
                ),
            );
            return None;
        };
        let Some((param, max)) = bel
            .params_when("value")
            .first()
            .map(|(key, value)| (key.clone(), value.as_int().unwrap_or(0)))
        else {
            self.io_warning(
                port,
                span,
                &format!(
                    "the database does not say which parameter of `{}` carries the delay",
                    bel.name
                ),
            );
            return None;
        };
        if !bel.has_ports(&["i", "o"]) {
            self.io_warning(
                port,
                span,
                &format!("`{}` has no (i, o) port map", bel.name),
            );
            return None;
        }
        if i64::from(steps) > max {
            self.io_warning(
                port,
                span,
                &format!(
                    "`{}` delays at most {max} steps and {steps} were asked for, so the delay is \
                     not applied",
                    bel.name
                ),
            );
            return None;
        }
        Some(DelayPlan { bel, param, steps })
    }

    /// One delay element on `value`, returning the delayed signal.
    fn emit_delay(
        &mut self,
        module: &mut Module,
        plan: &DelayPlan,
        value: ExprId,
        port: &str,
        bit: u32,
        span: Span,
    ) -> ExprId {
        let out = add_net(module, &format!("{port}$dly{bit}"), Type::bit(), span);
        let cell = add_cell(
            module,
            &format!("{port}$delay{bit}"),
            CellKind::Blackbox(Name::new(plan.bel.name.clone())),
            vec![(Name::new(plan.bel.port("i").unwrap_or("i")), value)],
            vec![(Name::new(plan.bel.port("o").unwrap_or("o")), out)],
            span,
        );
        for (key, value) in &plan.bel.params {
            module.cells[cell].params.set(key.clone(), value.clone());
        }
        module.cells[cell]
            .params
            .set(plan.param.clone(), AttrValue::Int(i64::from(plan.steps)));
        net_expr(module, out, span)
    }

    /// A DDR input register beside the IO buffer, returning the bits
    /// captured on the rising and on the falling edge.
    #[allow(clippy::too_many_arguments)]
    fn emit_ddr_in(
        &mut self,
        module: &mut Module,
        reg: &BelKind,
        clk: NetId,
        value: ExprId,
        port: &str,
        bit: u32,
        span: Span,
    ) -> (ExprId, ExprId) {
        let clock = net_expr(module, clk, span);
        let lo = add_net(module, &format!("{port}$q{bit}"), Type::bit(), span);
        let hi = add_net(module, &format!("{port}$q{bit}_n"), Type::bit(), span);
        let mut inputs = vec![
            (Name::new(reg.port("d").unwrap_or("d")), value),
            (Name::new(reg.port("clk").unwrap_or("clk")), clock),
        ];
        if let Some(rst) = reg.port("rst") {
            let zero = const_expr(module, Const::zero(1), span);
            inputs.push((Name::new(rst), zero));
        }
        let cell = add_cell(
            module,
            &format!("{port}$iddr{bit}"),
            CellKind::Blackbox(Name::new(reg.name.clone())),
            inputs,
            vec![
                (Name::new(reg.port("q0").unwrap_or("q0")), lo),
                (Name::new(reg.port("q1").unwrap_or("q1")), hi),
            ],
            span,
        );
        for (key, value) in &reg.params {
            module.cells[cell].params.set(key.clone(), value.clone());
        }
        (net_expr(module, lo, span), net_expr(module, hi, span))
    }

    /// A DDR output register beside the IO buffer, launching `bits.0` on
    /// the rising edge and `bits.1` on the falling one.
    #[allow(clippy::too_many_arguments)]
    fn emit_ddr_out(
        &mut self,
        module: &mut Module,
        reg: &BelKind,
        clk: NetId,
        bits: (ExprId, ExprId),
        port: &str,
        bit: u32,
        span: Span,
    ) -> ExprId {
        let clock = net_expr(module, clk, span);
        let out = add_net(module, &format!("{port}$d{bit}"), Type::bit(), span);
        let mut inputs = vec![
            (Name::new(reg.port("d0").unwrap_or("d0")), bits.0),
            (Name::new(reg.port("d1").unwrap_or("d1")), bits.1),
            (Name::new(reg.port("clk").unwrap_or("clk")), clock),
        ];
        if let Some(rst) = reg.port("rst") {
            let zero = const_expr(module, Const::zero(1), span);
            inputs.push((Name::new(rst), zero));
        }
        let cell = add_cell(
            module,
            &format!("{port}$oddr{bit}"),
            CellKind::Blackbox(Name::new(reg.name.clone())),
            inputs,
            vec![(Name::new(reg.port("q").unwrap_or("q")), out)],
            span,
        );
        for (key, value) in &reg.params {
            module.cells[cell].params.set(key.clone(), value.clone());
        }
        net_expr(module, out, span)
    }

    // --- clock generators ---------------------------------------------------

    /// Instantiates a PLL for every clock constraint that names a net
    /// nothing drives, generating it from a clock constrained on an input
    /// port.
    ///
    /// That is how a design asks for a frequency: it declares the clock
    /// net, uses it, and constrains it to the period it wants, next to
    /// the constraint that states what the board's oscillator provides.
    /// A constrained net that *is* driven — by a divider in logic, say —
    /// is a description of a clock that already exists and is left
    /// alone.
    fn clock_generators(&mut self, module: &mut Module) {
        if self.constraints.clocks.is_empty() {
            return;
        }
        let driven = super::constraints::driven_nets(module);
        let mut sources: Vec<(String, NetId, f64)> = Vec::new();
        let mut wanted: Vec<(String, NetId, f64, Span)> = Vec::new();
        for clock in &self.constraints.clocks {
            let Some(net) = module.net_by_name(&clock.net) else {
                continue;
            };
            let frequency = clock.frequency_mhz();
            if frequency <= 0.0 {
                continue;
            }
            let is_input = module
                .port(&clock.net)
                .is_some_and(|p| p.dir == PortDir::In);
            if is_input {
                sources.push((clock.net.clone(), net, frequency));
            } else if !driven.get(net.index()).copied().unwrap_or(true) {
                wanted.push((clock.net.clone(), net, frequency, clock.span));
            }
        }
        if wanted.is_empty() {
            return;
        }
        let shapes: Vec<PllShape> = self
            .device
            .clock_resources
            .plls
            .iter()
            .filter(|p| p.is_configurable())
            .cloned()
            .collect();
        // Several lines may describe one physical block used different
        // ways, so the budget is the largest count, not their sum.
        let budget = shapes.iter().filter_map(|p| p.count).max();
        let mut used = 0u32;
        for (name, net, frequency, span) in wanted {
            let refuse = |this: &mut Self, why: String| {
                this.diags.push(
                    Diagnostic::error(format!(
                        "clock `{name}` asks for {frequency:.3} MHz and nothing drives it, but no \
                         PLL can be built for it: {why}"
                    ))
                    .with_code(NO_PLL)
                    .with_span(span),
                );
                this.note(format!("clock `{name}` gets no PLL: {why}"));
            };
            if shapes.is_empty() {
                refuse(
                    self,
                    format!(
                        "`{}` describes no PLL with its dividers and ports",
                        self.device.name
                    ),
                );
                continue;
            }
            if budget.is_some_and(|b| used >= b) {
                refuse(
                    self,
                    format!(
                        "`{}` has {} PLL(s) and they are all in use",
                        self.device.name,
                        budget.unwrap_or(0)
                    ),
                );
                continue;
            }
            let mut best: Option<(usize, usize, PllSolution)> = None;
            let mut reasons: Vec<String> = Vec::new();
            for (source_index, (_, source, input)) in sources.iter().enumerate() {
                if *source == net {
                    continue;
                }
                for (shape_index, shape) in shapes.iter().enumerate() {
                    match super::pll::solve(shape, *input, frequency) {
                        Ok(solution) => {
                            let better = best.as_ref().is_none_or(|(_, _, b)| {
                                solution.error_mhz().abs() < b.error_mhz().abs()
                            });
                            if better {
                                best = Some((source_index, shape_index, solution));
                            }
                        }
                        Err(why) => reasons.push(why),
                    }
                }
            }
            let Some((source_index, shape_index, solution)) = best else {
                let why = if sources.is_empty() {
                    "no clock is constrained on an input port to generate it from".to_owned()
                } else {
                    reasons.join("; ")
                };
                refuse(self, why);
                continue;
            };
            used += 1;
            let (source_name, source, _) = &sources[source_index];
            let shape = &shapes[shape_index];
            self.emit_pll(module, shape, &solution, *source, net, span);
            let mapping = PllMapping {
                net: name.clone(),
                source: source_name.clone(),
                primitive: shape.name.clone(),
                input_hz: super::pll::to_hz(solution.input_mhz),
                requested_hz: super::pll::to_hz(solution.requested_mhz),
                achieved_hz: super::pll::to_hz(solution.achieved_mhz),
                vco_hz: super::pll::to_hz(solution.vco_mhz),
                params: solution.params.clone(),
            };
            // A PLL a percent off is still a PLL, and whether that is
            // close enough is the designer's call; past that it deserves
            // to be said out loud rather than found in a report.
            if solution.error_ppm().abs() > 10_000.0 {
                self.diags.push(
                    Diagnostic::warning(format!(
                        "clock `{name}` asks for {frequency:.3} MHz and the nearest `{}` can make \
                         from `{source_name}` is {:.3} MHz ({:+.1} %)",
                        shape.name,
                        solution.achieved_mhz,
                        solution.error_ppm() / 1e4
                    ))
                    .with_code(NO_PLL)
                    .with_span(span),
                );
            }
            self.report.plls.push(mapping);
        }
    }

    /// Instantiates one PLL, configured by `solution`, from `source` to
    /// `out`.
    fn emit_pll(
        &mut self,
        module: &mut Module,
        shape: &PllShape,
        solution: &PllSolution,
        source: NetId,
        out: NetId,
        span: Span,
    ) {
        let name = module.nets[out].name.as_str().to_owned();
        let reference = net_expr(module, source, span);
        let mut inputs = vec![(Name::new(shape.port("ref").unwrap_or("REF")), reference)];
        let mut outputs = vec![(Name::new(shape.port("out").unwrap_or("OUT")), out)];
        match (shape.port("fbout"), shape.port("fb")) {
            // A block that brings its feedback tap out on a pin of its
            // own and expects the loop closed outside it: the Xilinx
            // `PLLE2_BASE`, whose `CLKFBOUT` has to reach `CLKFBIN` or
            // the phase detector sees nothing. The net between them is
            // the whole loop, and the dividers the solver chose already
            // assume it is a plain wire.
            (Some(fbout), Some(fbin)) => {
                let net = add_net(module, &format!("{name}$fb"), Type::bit(), span);
                outputs.push((Name::new(fbout), net));
                inputs.push((Name::new(fbin), net_expr(module, net, span)));
            }
            // Feedback taken from the output is wired back from the
            // output, which is what the formula the solver used assumes.
            (None, Some(port)) if shape.feedback == PllFeedback::Output => {
                let feedback = net_expr(module, out, span);
                inputs.push((Name::new(port), feedback));
            }
            _ => {}
        }
        for (port, level) in &shape.ties {
            let value = const_expr(module, Const::from_bool(*level), span);
            inputs.push((Name::new(port.clone()), value));
        }
        let cell = add_cell(
            module,
            &format!("{name}$pll"),
            CellKind::Blackbox(Name::new(shape.name.clone())),
            inputs,
            outputs,
            span,
        );
        let target = &mut module.cells[cell];
        for (key, value) in &shape.params {
            target.params.set(Name::new(key.clone()), value.clone());
        }
        for (key, value) in &solution.params {
            target
                .params
                .set(Name::new(key.clone()), AttrValue::Int(*value));
        }
        target.attrs.set(
            "frequency_mhz",
            AttrValue::String(format!("{:.6}", solution.achieved_mhz)),
        );
    }

    // --- clock buffers ------------------------------------------------------

    fn clock_buffers(&mut self, module: &mut Module) {
        let Some(bel) = self.device.bel(BelRole::GlobalBuffer).cloned() else {
            return;
        };
        if !bel.has_ports(&["i", "o"]) {
            self.note(format!(
                "the global buffer `{}` of `{}` has no (i, o) port map",
                bel.name, self.device.name
            ));
            return;
        }
        // Which net each clock pin reads, how many pins that is, and which
        // cells would move onto the buffer. A flip-flop cell is as many
        // clock pins as it is bits wide, since that is what it becomes
        // once it is mapped; a memory port is one.
        let mut fanout: BTreeMap<String, (NetId, Vec<CellId>, usize)> = BTreeMap::new();
        for (id, cell) in module.cells.iter() {
            let pins = match &cell.kind {
                CellKind::Dff { .. } => cell.output("q").map_or(1, |q| {
                    usize::try_from(net_width(module, q)).unwrap_or(1).max(1)
                }),
                CellKind::MemRdPort { clocked, .. } | CellKind::MemWrPort { clocked, .. }
                    if *clocked =>
                {
                    1
                }
                _ => continue,
            };
            let Some(clk) = cell.input("clk") else {
                continue;
            };
            let Some(net) = module.exprs.get(clk).and_then(Expr::as_net) else {
                continue;
            };
            let name = module.nets[net].name.as_str().to_owned();
            let entry = fanout.entry(name).or_insert((net, Vec::new(), 0));
            entry.1.push(id);
            entry.2 += pins;
        }
        // Highest fanout first, ties broken by name so the choice is
        // reproducible.
        let mut candidates: Vec<(String, NetId, Vec<CellId>, usize)> = fanout
            .into_iter()
            .map(|(name, (net, cells, pins))| (name, net, cells, pins))
            .collect();
        candidates.sort_by(|a, b| b.3.cmp(&a.3).then_with(|| a.0.cmp(&b.0)));

        let available = self.device.clock_resources.global_buffers;
        let mut used = 0u32;
        for (name, net, cells, count) in candidates {
            if count < self.options.global_buffer_threshold {
                self.report.clocks.push(ClockMapping {
                    net: name,
                    fanout: count,
                    primitive: None,
                    reason: Some(format!(
                        "below the threshold of {}",
                        self.options.global_buffer_threshold
                    )),
                });
                continue;
            }
            if available > 0 && used >= available {
                self.diags.push(
                    Diagnostic::warning(format!(
                        "`{}` has only {available} global buffer(s), so clock `{name}` stays on local routing",
                        self.device.name
                    ))
                    .with_code(NO_GLOBAL_BUFFER)
                    .with_span(module.nets[net].span),
                );
                self.report.clocks.push(ClockMapping {
                    net: name,
                    fanout: count,
                    primitive: None,
                    reason: Some("no global buffer left".to_owned()),
                });
                continue;
            }
            used += 1;
            let span = module.nets[net].span;
            let buffered = add_net(
                module,
                &format!("{name}$gb"),
                module.nets[net].ty.clone(),
                span,
            );
            let source = net_expr(module, net, span);
            let mut inputs = vec![(Name::new(bel.port("i").unwrap_or("i")), source)];
            // A buffer with an enable (the ECP5's `DCCA` has a `CE`) is
            // always on: nothing gates a clock here, and leaving the pin
            // unconnected would let the tool decide what an unenabled
            // clock buffer does.
            if let Some(port) = bel.port("en") {
                let one = const_expr(module, Const::ones(1), span);
                inputs.push((Name::new(port), one));
            }
            add_cell(
                module,
                &format!("{name}$gbuf"),
                CellKind::Blackbox(Name::new(bel.name.clone())),
                inputs,
                vec![(Name::new(bel.port("o").unwrap_or("o")), buffered)],
                span,
            );
            let replacement = net_expr(module, buffered, span);
            for id in cells {
                if let Some(slot) = module.cells[id]
                    .inputs
                    .iter_mut()
                    .find(|(port, _)| port.as_str() == "clk")
                {
                    slot.1 = replacement;
                }
            }
            self.report.clocks.push(ClockMapping {
                net: name,
                fanout: count,
                primitive: Some(bel.name.clone()),
                reason: None,
            });
        }
    }
}

/// Where one read port's muxing output goes, and what to call the cells
/// that build it.
struct ReadOutput<'a> {
    /// The memory's name, which the new nets and cells are named after.
    base: &'a str,
    /// Which read port of the memory this is.
    index: usize,
    /// Width of the net the muxing drives.
    width: u32,
    /// The span the new objects carry.
    span: Span,
}

/// How one memory is laid out over blocks.
struct BramPlan {
    /// The width mode chosen, as `(data_width, depth)`.
    mode: (u32, u32),
    /// How many blocks side by side carry the width.
    wide: u32,
    /// How many blocks stacked carry the depth.
    deep: u32,
    /// Which physical port of the block each read port uses.
    read_slots: Vec<usize>,
    /// Which physical port of the block each write port uses.
    write_slots: Vec<usize>,
    /// Which copy of the contents this is.
    copy: usize,
    /// How many copies there are in all.
    copies: usize,
    /// How the mode places a word on the pins and in the contents.
    layout: BramModeLayout,
    /// Write the memory's initial contents into every block's
    /// initialisation parameters; decided, and any loss reported, by
    /// [`Mapper::bram_contents`].
    initialise: bool,
}

impl BramPlan {
    /// The prefix a cell or net of this copy is named with.
    fn tag(&self, base: &str) -> String {
        if self.copies > 1 {
            format!("{base}$c{}", self.copy)
        } else {
            base.to_owned()
        }
    }
}

/// One copy of a memory's contents, with the read ports it serves.
///
/// There is one of these unless the memory wants more ports than a block
/// has, in which case there is one per read port; see
/// [`Mapper::block_ram`].
struct BramCopy {
    /// The memory's read port cells this copy answers.
    reads: Vec<CellId>,
    /// Which physical port of the block each of `reads` uses.
    read_slots: Vec<usize>,
    /// Which physical port of the block each write port uses.
    write_slots: Vec<usize>,
    /// Which copy this is.
    index: usize,
}

/// Bits `lo..=hi` of `data` placed on the data pins a mode spreads them
/// over ([`BramModeLayout::data_bits`]), `pins` wide, with every pin the
/// mode does not use tied to 0.
fn spread_data(
    module: &mut Module,
    data: ExprId,
    layout: &BramModeLayout,
    lo: u32,
    hi: u32,
    pins: u32,
    span: Span,
) -> ExprId {
    // Most significant pin first, runs of unused pins as one constant.
    let mut parts: Vec<ExprId> = Vec::new();
    let mut zeros = 0u32;
    for pin in (0..pins).rev() {
        let bit = layout
            .data_bits
            .iter()
            .position(|p| *p == pin)
            .and_then(|j| u32::try_from(j).ok())
            .and_then(|j| lo.checked_add(j))
            .filter(|bit| *bit <= hi);
        match bit {
            Some(bit) => {
                if zeros > 0 {
                    parts.push(const_expr(module, Const::zero(zeros), span));
                    zeros = 0;
                }
                parts.push(slice_expr(module, data, bit, bit, span));
            }
            None => zeros += 1,
        }
    }
    if zeros > 0 {
        parts.push(const_expr(module, Const::zero(zeros), span));
    }
    if parts.len() == 1 {
        parts[0]
    } else {
        expr(module, ExprKind::Concat(parts), span)
    }
}

/// Why a mode's contents layout cannot be used with `params`, or `None`
/// when it can: the words of a row times the rows must be the mode's
/// depth, and every row bit must fall inside a slot.
fn init_layout_problem(
    params: &BramInitParams,
    layout: &BramInitLayout,
    mode: (u32, u32),
) -> Option<String> {
    let (mode_width, mode_depth) = mode;
    let per_row = u64::try_from(layout.words.len()).unwrap_or(u64::MAX);
    let rows = params.total_rows();
    if per_row.saturating_mul(rows) != u64::from(mode_depth) {
        return Some(format!(
            "{per_row} word(s) per row over {rows} rows is not {mode_depth} words"
        ));
    }
    for word in &layout.words {
        if u32::try_from(word.len()).ok() != Some(mode_width) {
            return Some(format!(
                "a word lists {} bits, not {mode_width}",
                word.len()
            ));
        }
        if let Some(bit) = word.iter().find(|bit| **bit >= params.slot) {
            return Some(format!(
                "row bit {bit} is outside a {}-bit row",
                params.slot
            ));
        }
    }
    None
}

/// The initialisation parameters of the block that holds bits
/// `w * mode_width ..` of words `d * mode_depth ..` of a memory `width`
/// bits wide with initial contents `init`.
///
/// Word `a` of the block (counted from its first) keeps its data bit `j`
/// where [`BramInitLayout::locate`] says; a bit that is not a known `1`,
/// and every word past the end of `init`, is 0. Every parameter is
/// written, so the block's contents are stated in full.
fn block_contents(
    init: &[Const],
    width: u32,
    params: &BramInitParams,
    layout: &BramInitLayout,
    mode: (u32, u32),
    w: u32,
    d: u32,
) -> Vec<(Name, AttrValue)> {
    let (mode_width, mode_depth) = mode;
    let rows = params.total_rows();
    let rows_per_param = u64::from(params.rows.max(1));
    let mut values: Vec<Const> = (0..params.count)
        .map(|_| Const::zero(params.param_width()))
        .collect();
    let first = u64::from(d) * u64::from(mode_depth);
    for local in 0..u64::from(mode_depth) {
        let Some(word) = usize::try_from(first + local)
            .ok()
            .and_then(|index| init.get(index))
        else {
            break;
        };
        for j in 0..mode_width {
            let Some(bit) = w
                .checked_mul(mode_width)
                .and_then(|base| base.checked_add(j))
                .filter(|bit| *bit < width)
            else {
                break;
            };
            if word.get(bit) != Some(Bit::One) {
                continue;
            }
            let Some((row, at)) = layout.locate(local, j, rows) else {
                continue;
            };
            let within = u32::try_from(row % rows_per_param).unwrap_or(0);
            let Some(value) = usize::try_from(row / rows_per_param)
                .ok()
                .and_then(|index| values.get_mut(index))
            else {
                continue;
            };
            let position = within * params.slot + at;
            if position < value.width() {
                value.set_bit(position, Bit::One);
            }
        }
    }
    values
        .into_iter()
        .zip(0u32..)
        .map(|(value, index)| (Name::new(params.name(index)), AttrValue::Const(value)))
        .collect()
}

/// Why a block RAM shape cannot serve a memory's ports.
///
/// A port serves one access, read or write, so the constraint is the
/// total, not the read and write counts separately. Reporting those
/// separately produced a message where both halves looked satisfiable: a
/// 2-read, 1-write register file against a shape with 2 readable and 2
/// writable ports reads as though it fits, when in fact it wants three.
fn port_shortfall(shape: &BramShape, reads: usize, writes: usize) -> String {
    let usable = shape
        .port_map
        .iter()
        .filter(|p| !p.signals.is_empty())
        .count();
    format!(
        "`{}` has {usable} port(s) and each serves either a read or a write, but the memory \
         needs {} ({reads} read, {writes} write)",
        shape.name,
        reads + writes
    )
}

/// Gives every read and every write port of a memory a physical port of
/// the block, or `None` when the block does not have enough.
///
/// A port is used once: a true dual-port block serves one reader and one
/// writer with its two independent ports rather than piling both onto the
/// first one. Readers are placed first because a block is more often
/// short of read ports than of write ports.
fn allocate_ports(
    shape: &BramShape,
    reads: usize,
    writes: usize,
) -> Option<(Vec<usize>, Vec<usize>)> {
    let mut taken = vec![false; shape.port_map.len()];
    let pick = |wants_read: bool, taken: &mut Vec<bool>| {
        let found = shape
            .port_map
            .iter()
            .enumerate()
            .position(|(index, port)| {
                !taken[index]
                    && !port.signals.is_empty()
                    && if wants_read {
                        port.role.reads()
                    } else {
                        port.role.writes()
                    }
            })?;
        taken[found] = true;
        Some(found)
    };
    let mut read_slots = Vec::with_capacity(reads);
    for _ in 0..reads {
        read_slots.push(pick(true, &mut taken)?);
    }
    let mut write_slots = Vec::with_capacity(writes);
    for _ in 0..writes {
        write_slots.push(pick(false, &mut taken)?);
    }
    Some((read_slots, write_slots))
}

/// How one double-data-rate port is built.
enum DdrPlan {
    /// The IO buffer registers both edges itself (iCE40 `SB_IO`), on
    /// this clock.
    InBuffer {
        /// The clock net.
        clk: NetId,
    },
    /// A register beside the buffer does (ECP5 `IDDRX1F` / `ODDRX1F`).
    Beside {
        /// The register primitive.
        bel: BelKind,
        /// The clock net.
        clk: NetId,
    },
}

/// The delay element one port gets.
struct DelayPlan {
    /// The primitive.
    bel: BelKind,
    /// The parameter carrying the number of steps.
    param: String,
    /// How many steps.
    steps: u32,
}

/// What every IO buffer cell of one port shares.
struct IoSite<'a> {
    /// The port's name, which the cells are named after.
    port: &'a str,
    /// The package pin, if one is assigned.
    pin: Option<&'a str>,
    /// The electrical options.
    io: &'a IoAttrs,
    /// The condition selecting the buffer's direction parameters (`in`,
    /// `out`, `inout`, `ddr_in`, `ddr_out`).
    conditions: &'a str,
    /// The span the new objects carry.
    span: Span,
}

/// What the database says the device's distributed RAM primitive is.
///
/// Read off its port map rather than declared: `width` is how many
/// `dout` pins it has, `addr_bits` how many `raddr` pins, and `depth`
/// what those address.
struct LutRamShape {
    /// The primitive.
    bel: BelKind,
    /// Bits per word.
    width: u32,
    /// Address bits one block takes, so it holds `2^addr_bits` words.
    addr_bits: u32,
}

/// The shape of one memory being built out of logic.
struct LogicPlan {
    /// The memory's name, which the new nets and cells are named after.
    name: String,
    /// The span the new objects carry.
    span: Span,
    /// Bits per word.
    width: u32,
    /// Words.
    rows_wanted: usize,
    /// The initial contents, element 0 first; empty when there are none.
    init: Vec<Const>,
}

impl LogicPlan {
    /// Word `row` of the initial contents as a constant, or an `x` word
    /// when the memory does not state one, which is what reading an
    /// uninitialised memory gives.
    fn constant_row(&self, module: &mut Module, row: usize, span: Span) -> ExprId {
        match self.init.get(row) {
            Some(value) => {
                let value = value.clone().resize(self.width);
                const_expr(module, value, span)
            }
            None => const_expr(module, Const::x(self.width), span),
        }
    }
}

/// The signals of one memory port, for the logic fallback.
struct LogicPort {
    /// The address.
    addr: Option<ExprId>,
    /// The clock, for a clocked port.
    clk: Option<ExprId>,
    /// The enable, if the port has one.
    en: Option<ExprId>,
    /// The data written, for a write port.
    data: Option<ExprId>,
    /// The net read, for a read port.
    out: Option<NetId>,
}

/// The signals of one memory port, resolved for the block wiring.
struct PortWiring {
    /// The address bits inside one block.
    low: Option<ExprId>,
    /// The address bits selecting which block.
    high: Option<ExprId>,
    /// How many bits `high` has.
    high_bits: u32,
    /// The clock, for a clocked port.
    clk: Option<ExprId>,
    /// The enable, if the port has one.
    en: Option<ExprId>,
    /// The data written, for a write port.
    data: Option<ExprId>,
    /// The net read, for a read port.
    out: Option<NetId>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fpga::target;
    use crate::ir::builder::ModuleBuilder;
    use crate::ir::validate::validate;
    use crate::ir::{ProcessKind, Reset};
    use crate::source::{SourceMap, Span};

    fn span() -> (SourceMap, Span) {
        let mut map = SourceMap::new();
        let file = map.add("top.v", "").unwrap();
        let span = Span::new(file, 0, 0);
        (map, span)
    }

    /// A design with one memory of `depth` elements of `width` bits, one
    /// clocked write port and one clocked read port.
    fn memory_design(width: u32, depth: u64) -> (Design, ModuleId, SourceMap) {
        let (map, span) = span();
        let mut b = ModuleBuilder::new("top", span);
        let clk = b.input("clk", Type::bit());
        let we = b.input("we", Type::bit());
        let addr_width = addr_bits(depth).max(1);
        let waddr = b.input("waddr", Type::bits(addr_width));
        let raddr = b.input("raddr", Type::bits(addr_width));
        let wdata = b.input("wdata", Type::bits(width));
        let rdata = b.output("rdata", Type::bits(width));
        let mem = b.memory("ram", Type::bits(width), depth);
        let (clk_e, we_e) = (b.net(clk), b.net(we));
        let (waddr_e, wdata_e) = (b.net(waddr), b.net(wdata));
        let raddr_e = b.net(raddr);
        let one = b.const_bit(true);
        b.cell(
            "wr",
            CellKind::MemWrPort { mem, clocked: true },
            vec![
                (Name::new("addr"), waddr_e),
                (Name::new("data"), wdata_e),
                (Name::new("en"), we_e),
                (Name::new("clk"), clk_e),
            ],
            vec![],
        );
        b.cell(
            "rd",
            CellKind::MemRdPort { mem, clocked: true },
            vec![
                (Name::new("addr"), raddr_e),
                (Name::new("clk"), clk_e),
                (Name::new("en"), one),
            ],
            vec![(Name::new("data"), rdata)],
        );
        let mut design = Design::new();
        let top = design.add_module(b.finish());
        design.top = Some(top);
        (design, top, map)
    }

    fn run(design: &mut Design, top: ModuleId, device: &str, options: &MapOptions) -> MapReport {
        let mut diags = Diagnostics::new();
        let report = map(
            design,
            top,
            target(device).unwrap(),
            &Constraints::new(),
            options,
            &mut diags,
        );
        let problems = validate(design);
        assert!(
            !problems.has_errors(),
            "mapped design is invalid:\n{:?}",
            problems.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
        report
    }

    fn cells_named(design: &Design, top: ModuleId, primitive: &str) -> usize {
        design
            .module(top)
            .cells
            .iter()
            .filter(|(_, c)| matches!(&c.kind, CellKind::Blackbox(n) if n.as_str() == primitive))
            .count()
    }

    #[test]
    fn splits_a_wide_memory_across_blocks() {
        // 1024 x 32 bits is 32 kbit: eight 4 kbit blocks, two wide and
        // four deep in the 16-bit mode.
        let (mut design, top, _map) = memory_design(32, 1024);
        let options = MapOptions {
            insert_io_buffers: false,
            insert_clock_buffers: false,
            ..MapOptions::default()
        };
        let report = run(&mut design, top, "ice40-hx1k-tq144", &options);
        let item = &report.block_rams[0];
        assert_eq!(item.primitive, "SB_RAM40_4K");
        assert_eq!(item.mode, (16, 256));
        assert_eq!((item.wide, item.deep), (2, 4));
        assert_eq!(item.blocks(), 8);
        assert_eq!(cells_named(&design, top, "SB_RAM40_4K"), 8);
        // Four depth slices need two select bits, so three mux cells and
        // one register for the read address.
        let module = design.module(top);
        assert_eq!(
            module
                .cells
                .iter()
                .filter(|(_, c)| c.kind == CellKind::Mux)
                .count(),
            3
        );
        assert_eq!(
            module
                .cells
                .iter()
                .filter(|(_, c)| c.kind == CellKind::Eq)
                .count(),
            8,
            "one address comparison per block row and port"
        );
        assert!(
            module
                .memories
                .iter()
                .all(|(_, m)| m.attrs.contains("mapped_to"))
        );
        assert!(report.to_text().contains("2 wide, 4 deep"));
    }

    #[test]
    fn a_small_memory_fits_one_block() {
        let (mut design, top, _map) = memory_design(8, 64);
        let options = MapOptions {
            insert_io_buffers: false,
            insert_clock_buffers: false,
            min_bram_bits: 256,
            ..MapOptions::default()
        };
        let report = run(&mut design, top, "ice40-hx1k-tq144", &options);
        let item = &report.block_rams[0];
        assert_eq!(item.blocks(), 1);
        assert_eq!(item.mode, (16, 256), "the widest mode that fits wins");
        assert_eq!(cells_named(&design, top, "SB_RAM40_4K"), 1);
        // One block needs no decoding and no muxing.
        let module = design.module(top);
        assert_eq!(
            module
                .cells
                .iter()
                .filter(|(_, c)| matches!(c.kind, CellKind::Mux | CellKind::Eq))
                .count(),
            0
        );
    }

    #[test]
    fn a_deep_memory_splits_by_address() {
        // 4096 x 1 is 4 kbit: one block in the 2048x2 mode would waste
        // half of it, so it takes two blocks stacked.
        let (mut design, top, _map) = memory_design(1, 4096);
        let options = MapOptions {
            insert_io_buffers: false,
            insert_clock_buffers: false,
            ..MapOptions::default()
        };
        let report = run(&mut design, top, "ice40-hx1k-tq144", &options);
        let item = &report.block_rams[0];
        assert_eq!(item.mode, (2, 2048));
        assert_eq!((item.wide, item.deep), (1, 2));
        assert_eq!(cells_named(&design, top, "SB_RAM40_4K"), 2);
        // The 2x2048 mode reads and writes on data pins 3 and 11, which
        // the database says, so there is nothing left to warn about.
        assert!(
            !report.notes.iter().any(|n| n.contains("narrow")),
            "{:?}",
            report.notes
        );
        // One select bit: one mux and one address register.
        let module = design.module(top);
        assert_eq!(
            module
                .cells
                .iter()
                .filter(|(_, c)| c.kind == CellKind::Mux)
                .count(),
            1
        );
        assert_eq!(
            module
                .cells
                .iter()
                .filter(|(_, c)| matches!(c.kind, CellKind::Dff { .. }))
                .count(),
            1
        );
    }

    #[test]
    fn small_memories_and_unknown_devices_fall_back() {
        let (mut design, top, _map) = memory_design(4, 16);
        let options = MapOptions {
            insert_io_buffers: false,
            insert_clock_buffers: false,
            ..MapOptions::default()
        };
        let report = run(&mut design, top, "ice40-hx1k-tq144", &options);
        assert!(report.block_rams.is_empty());
        let fallback = &report.bram_fallbacks[0];
        assert!(fallback.reason.contains("below the 256 bit threshold"));
        assert_eq!(fallback.style, "flip-flops");
        assert!(report.to_text().contains("ram -> flip-flops"));

        // `ram_style = "block"` overrides the threshold.
        let (mut design, top, _map) = memory_design(4, 16);
        let mem = design.module(top).memories.ids().next().unwrap();
        design.module_mut(top).memories[mem]
            .attrs
            .set("ram_style", "block");
        let report = run(&mut design, top, "ice40-hx1k-tq144", &options);
        assert_eq!(report.block_rams.len(), 1);

        // `ram_style = "distributed"` keeps it in logic, silently.
        let (mut design, top, _map) = memory_design(32, 1024);
        let mem = design.module(top).memories.ids().next().unwrap();
        design.module_mut(top).memories[mem]
            .attrs
            .set("ram_style", "distributed");
        let report = run(&mut design, top, "ecp5-25f-CABGA381", &options);
        assert!(report.block_rams.is_empty());
        assert_eq!(report.bram_fallbacks[0].style, "distributed LUT RAM");
    }

    #[test]
    fn a_small_memory_becomes_flip_flops_on_a_family_without_lut_ram() {
        // 16 x 4 is 64 bits, under the threshold, and the iCE40 database
        // declares no distributed RAM: one flip-flop per bit, a decoded
        // write enable and a read multiplexer.
        let (mut design, top, _map) = memory_design(4, 16);
        let options = MapOptions {
            insert_io_buffers: false,
            insert_clock_buffers: false,
            ..MapOptions::default()
        };
        let report = run(&mut design, top, "ice40-hx1k-tq144", &options);
        let fallback = &report.bram_fallbacks[0];
        assert_eq!(fallback.style, "flip-flops");
        assert!(fallback.built);
        assert_eq!(fallback.cells, 64);
        assert_eq!(fallback.primitive, None);
        assert!(
            report
                .to_text()
                .contains("ram -> flip-flops, 64 flip-flop(s)")
        );

        // Nothing is left of the memory for the netlist check to find.
        let module = design.module(top);
        assert_eq!(module.memories.len(), 0);
        assert!(module.cells.iter().all(|(_, c)| !matches!(
            c.kind,
            CellKind::MemRdPort { .. } | CellKind::MemWrPort { .. }
        )));
        // Sixteen word registers plus the clocked read port's own.
        assert_eq!(
            module
                .cells
                .iter()
                .filter(|(_, c)| matches!(c.kind, CellKind::Dff { .. }))
                .count(),
            17
        );
        // A 16:1 multiplexer is fifteen two-input ones.
        assert_eq!(
            module
                .cells
                .iter()
                .filter(|(_, c)| c.kind == CellKind::Mux)
                .count(),
            15 + 16,
            "fifteen to select the word, one per word for the write"
        );
        assert!(!validate(&design).has_errors());
    }

    #[test]
    fn a_small_memory_becomes_lut_ram_where_the_device_has_one() {
        // The ECP5 declares TRELLIS_DPR16X4, four bits by sixteen words,
        // so a 16 x 8 memory is two of them side by side and no
        // multiplexer at all.
        let (mut design, top, _map) = memory_design(8, 16);
        let options = MapOptions {
            insert_io_buffers: false,
            insert_clock_buffers: false,
            ..MapOptions::default()
        };
        let report = run(&mut design, top, "ecp5-45f-CABGA381", &options);
        let fallback = &report.bram_fallbacks[0];
        assert_eq!(fallback.style, "distributed LUT RAM");
        assert_eq!(fallback.primitive.as_deref(), Some("TRELLIS_DPR16X4"));
        assert_eq!(fallback.cells, 2);
        assert!(fallback.built);
        assert_eq!(cells_named(&design, top, "TRELLIS_DPR16X4"), 2);
        assert_eq!(design.module(top).memories.len(), 0);
        assert!(!validate(&design).has_errors());

        // Every pin the cells connect is one the device declares; the
        // whole netlist is checked by `tests/fpga_flow.rs`, which runs
        // the rest of the flow too.
        let device = target("ecp5-45f-CABGA381").unwrap();
        let declared = device.primitive_ports("TRELLIS_DPR16X4").unwrap();
        for (_, cell) in design.module(top).cells.iter() {
            if !matches!(&cell.kind, CellKind::Blackbox(n) if n.as_str() == "TRELLIS_DPR16X4") {
                continue;
            }
            // Sixteen pins wired: WCK, WRE, four each of WAD, DI, RAD
            // and DO.
            assert_eq!(cell.inputs.len() + cell.outputs.len(), 18);
            let connected = cell
                .inputs
                .iter()
                .map(|(port, _)| port)
                .chain(cell.outputs.iter().map(|(port, _)| port));
            for port in connected {
                assert!(
                    declared.iter().any(|p| p == port.as_str()),
                    "`{port}` is not a TRELLIS_DPR16X4 pin"
                );
            }
        }
    }

    #[test]
    fn a_memory_too_big_for_logic_is_reported_rather_than_built() {
        // Eight kilobits of flip-flops is not what anyone meant, so the
        // memory stays and the report says why.
        let (mut design, top, sources) = memory_design(8, 1024);
        let options = MapOptions {
            insert_io_buffers: false,
            insert_clock_buffers: false,
            // No block RAM to fall into.
            infer_block_ram: true,
            max_logic_bits: 256,
            ..MapOptions::default()
        };
        let mut device = target("ice40-hx1k-tq144").unwrap().clone();
        device.block_rams.clear();
        let mut diags = Diagnostics::new();
        let report = map(
            &mut design,
            top,
            &device,
            &Constraints::new(),
            &options,
            &mut diags,
        );
        let fallback = &report.bram_fallbacks[0];
        assert!(!fallback.built);
        assert_eq!(fallback.style, "a memory");
        assert!(
            fallback.reason.contains("over the 256 bit limit"),
            "{fallback:?}"
        );
        assert_eq!(design.module(top).memories.len(), 1);
        let text = diags.render(&sources);
        assert!(text.contains("is left as a memory"), "{text}");
    }

    /// Adds a second read port to the memory of [`memory_design`].
    fn add_second_read(design: &mut Design, top: ModuleId, clocked: bool) {
        let module = design.module_mut(top);
        let mem = module.memories.ids().next().unwrap();
        let span = module.span;
        let addr = module.nets.find(|n| n.name.as_str() == "raddr").unwrap();
        let clk = module.nets.find(|n| n.name.as_str() == "clk").unwrap();
        let out = add_net(module, "rdata2", Type::bits(8), span);
        let addr_e = net_expr(module, addr, span);
        let mut inputs = vec![(Name::new("addr"), addr_e)];
        if clocked {
            let clk_e = net_expr(module, clk, span);
            let one = const_expr(module, Const::ones(1), span);
            inputs.push((Name::new("clk"), clk_e));
            inputs.push((Name::new("en"), one));
        }
        add_cell(
            module,
            "rd2",
            CellKind::MemRdPort { mem, clocked },
            inputs,
            vec![(Name::new("data"), out)],
            span,
        );
    }

    #[test]
    fn a_second_read_port_duplicates_the_block_ram() {
        // One `SB_RAM40_4K` has one read port and one write port, and
        // this memory wants two reads and a write. The answer a real
        // flow gives a register file is duplication: two blocks holding
        // the same contents, each serving one reader, both written
        // together from the one writer.
        let (mut design, top, _sources) = memory_design(8, 512);
        add_second_read(&mut design, top, true);
        let mut diags = Diagnostics::new();
        let report = map(
            &mut design,
            top,
            target("ice40-hx1k-tq144").unwrap(),
            &Constraints::new(),
            &MapOptions::default(),
            &mut diags,
        );
        assert!(report.bram_fallbacks.is_empty(), "{report:?}");
        let item = &report.block_rams[0];
        assert_eq!(item.copies, 2);
        assert_eq!(item.blocks(), 2);
        assert_eq!(cells_named(&design, top, "SB_RAM40_4K"), 2);
        assert!(report.to_text().contains("2 copies, one per read port"));
        assert!(
            report
                .notes
                .iter()
                .any(|n| n.contains("held in 2 copies written together")),
            "{:?}",
            report.notes
        );
        // Both copies see the same write, and each answers one reader.
        let module = design.module(top);
        for name in ["rdata", "rdata2"] {
            let net = module.nets.find(|n| n.name.as_str() == name).unwrap();
            assert!(
                module.assigns.iter().any(|a| a.target == Lvalue::Net(net)),
                "`{name}` is not driven"
            );
        }
        assert!(!validate(&design).has_errors());
    }

    #[test]
    fn a_memory_with_too_many_ports_falls_back() {
        // Two readers and two writers: duplication cannot help, since
        // the copies would have to be kept in step between them.
        let (mut design, top, sources) = memory_design(8, 512);
        add_second_read(&mut design, top, true);
        let module = design.module_mut(top);
        let mem = module.memories.ids().next().unwrap();
        let span = module.span;
        let addr = module.nets.find(|n| n.name.as_str() == "waddr").unwrap();
        let data = module.nets.find(|n| n.name.as_str() == "wdata").unwrap();
        let clk = module.nets.find(|n| n.name.as_str() == "clk").unwrap();
        let (addr_e, data_e, clk_e) = (
            net_expr(module, addr, span),
            net_expr(module, data, span),
            net_expr(module, clk, span),
        );
        let one = const_expr(module, Const::ones(1), span);
        add_cell(
            module,
            "wr2",
            CellKind::MemWrPort { mem, clocked: true },
            vec![
                (Name::new("addr"), addr_e),
                (Name::new("data"), data_e),
                (Name::new("en"), one),
                (Name::new("clk"), clk_e),
            ],
            vec![],
            span,
        );
        // The logic fallback is switched off here: what this test is
        // about is the reason, and four thousand flip-flops take a
        // while to build.
        let options = MapOptions {
            max_logic_bits: 0,
            insert_io_buffers: false,
            insert_clock_buffers: false,
            ..MapOptions::default()
        };
        let mut diags = Diagnostics::new();
        let report = map(
            &mut design,
            top,
            target("ice40-hx1k-tq144").unwrap(),
            &Constraints::new(),
            &options,
            &mut diags,
        );
        assert!(report.block_rams.is_empty());
        // The reason names the constraint that applies: a port serves one
        // access, so two reads and two writes want four of them.
        assert!(
            report.bram_fallbacks[0]
                .reason
                .contains("each serves either a read or a write"),
            "{:?}",
            report.bram_fallbacks
        );
        assert!(
            report.bram_fallbacks[0]
                .reason
                .contains("needs 4 (2 read, 2 write)"),
            "{:?}",
            report.bram_fallbacks
        );
        let text = diags.render(&sources);
        assert!(text.contains("does not fit a block RAM"), "{text}");
    }

    #[test]
    fn an_asynchronous_read_port_never_becomes_a_block_ram() {
        // A block RAM reads on a clock edge. A memory read
        // combinationally is a different circuit, so it goes to logic
        // however well it would otherwise fit.
        let (mut design, top, _sources) = memory_design(8, 512);
        add_second_read(&mut design, top, false);
        let options = MapOptions {
            max_logic_bits: 0,
            insert_io_buffers: false,
            insert_clock_buffers: false,
            ..MapOptions::default()
        };
        let mut diags = Diagnostics::new();
        let report = map(
            &mut design,
            top,
            target("ice40-hx1k-tq144").unwrap(),
            &Constraints::new(),
            &options,
            &mut diags,
        );
        assert!(report.block_rams.is_empty());
        assert!(
            report.bram_fallbacks[0]
                .reason
                .contains("asynchronous read port"),
            "{:?}",
            report.bram_fallbacks
        );
    }

    /// Gives the memory of a [`memory_design`] the contents `word(a)`.
    fn preload(design: &mut Design, top: ModuleId, word: impl Fn(u64) -> u64) {
        let module = design.module_mut(top);
        let mem = module.memories.ids().next().unwrap();
        let memory = &mut module.memories[mem];
        let width = memory.elem.width().unwrap();
        memory.init = Some(
            (0..memory.size)
                .map(|a| Const::from_u64(word(a) & ((1 << width) - 1), width))
                .collect(),
        );
    }

    /// The block RAM cell called `name`.
    fn bram_cell<'a>(design: &'a Design, top: ModuleId, name: &str) -> &'a Cell {
        design
            .module(top)
            .cells
            .iter()
            .map(|(_, c)| c)
            .find(|c| c.name.as_str() == name)
            .unwrap_or_else(|| panic!("no cell {name}"))
    }

    /// Bit `bit` of initialisation parameter `param` of `cell`.
    fn init_bit(cell: &Cell, param: &str, bit: u32) -> bool {
        match cell.params.get(param) {
            Some(AttrValue::Const(c)) => c.bit(bit) == Bit::One,
            other => panic!("{param} of {} is {other:?}", cell.name),
        }
    }

    #[test]
    fn ice40_narrow_modes_spread_their_bits_and_their_contents() {
        // 512x8 on SB_RAM40_4K: the row is address bits 7..0, address
        // bit 8 picks the even or the odd bits of the row, and data bit
        // j is on data pin 2j.
        let (mut design, top, _map) = memory_design(8, 512);
        preload(&mut design, top, |a| a * 37 + 11);
        let options = MapOptions {
            insert_io_buffers: false,
            insert_clock_buffers: false,
            ..MapOptions::default()
        };
        let report = run(&mut design, top, "ice40-hx1k-tq144", &options);
        let item = &report.block_rams[0];
        assert_eq!(item.mode, (8, 512));
        assert!(item.initialised);
        let cell = bram_cell(&design, top, "ram$ram_w0_d0");
        for a in 0..512u32 {
            let word = (u64::from(a) * 37 + 11) & 0xff;
            let (row, sub) = (a % 256, a / 256);
            for j in 0..8 {
                let param = format!("INIT_{:X}", row / 16);
                let at = (row % 16) * 16 + 2 * j + sub;
                assert_eq!(init_bit(cell, &param, at), (word >> j) & 1 == 1, "{a}.{j}");
            }
        }
        // Pins 0, 2, .., 14 carry the data both ways, the odd ones
        // written as 0.
        let module = design.module(top);
        let wdata = cell.input("WDATA").unwrap();
        assert_eq!(expr_width(module, wdata), 15);
        let rdata = cell.output("RDATA").unwrap();
        assert_eq!(net_width(module, rdata), 15);
        let text = design.to_text();
        assert!(
            text.contains(
                "WDATA={%wdata[7:7], 1'd0, %wdata[6:6], 1'd0, %wdata[5:5], 1'd0, %wdata[4:4], \
                 1'd0, %wdata[3:3], 1'd0, %wdata[2:2], 1'd0, %wdata[1:1], 1'd0, %wdata[0:0]}"
            ),
            "{text}"
        );
        assert!(
            text.contains("{%ram$rd0_w0_d0[14:14], %ram$rd0_w0_d0[12:12]"),
            "{text}"
        );

        // 4096x1 in the 2048x2 mode, two blocks deep: the second holds
        // words 2048 and up, word `l` of a block in row l % 256, row bit
        // l / 256.
        let (mut design, top, _map) = memory_design(1, 4096);
        preload(&mut design, top, |a| u64::from(a % 3 == 0));
        let report = run(&mut design, top, "ice40-hx1k-tq144", &options);
        assert_eq!(report.block_rams[0].mode, (2, 2048));
        for d in 0..2u32 {
            let cell = bram_cell(&design, top, &format!("ram$ram_w0_d{d}"));
            for local in 0..2048u32 {
                let a = d * 2048 + local;
                let (row, sub) = (local % 256, local / 256);
                let param = format!("INIT_{:X}", row / 16);
                let at = (row % 16) * 16 + sub;
                assert_eq!(init_bit(cell, &param, at), a % 3 == 0, "word {a}");
            }
        }
    }

    #[test]
    fn ecp5_addresses_and_contents_follow_the_database() {
        // 2048x9 on DP16KD: the word address starts at address pin 3,
        // two words share a row (bits 8..0 and 17..9), and each
        // INITVAL_nn holds sixteen rows in 20-bit slots.
        let (mut design, top, _map) = memory_design(9, 2048);
        preload(&mut design, top, |a| a ^ (a >> 3));
        let options = MapOptions {
            insert_io_buffers: false,
            insert_clock_buffers: false,
            ..MapOptions::default()
        };
        let report = run(&mut design, top, "ecp5-45f-CABGA381", &options);
        let item = &report.block_rams[0];
        assert_eq!((item.mode, item.blocks()), ((9, 2048), 1));
        assert!(item.initialised);
        let cell = bram_cell(&design, top, "ram$ram_w0_d0");
        match cell.params.get("INITVAL_3F") {
            Some(AttrValue::Const(c)) => assert_eq!(c.width(), 320),
            other => panic!("{other:?}"),
        }
        for a in 0..2048u32 {
            let word = (u64::from(a) ^ (u64::from(a) >> 3)) & 0x1ff;
            let (row, sub) = (a / 2, a % 2);
            for j in 0..9 {
                let param = format!("INITVAL_{:02X}", row / 16);
                let at = (row % 16) * 20 + 9 * sub + j;
                assert_eq!(init_bit(cell, &param, at), (word >> j) & 1 == 1, "{a}.{j}");
            }
        }
        let text = design.to_text();
        assert!(text.contains("ADA={%raddr, 3'd0}"), "{text}");
        assert!(text.contains("ADB={%waddr, 3'd0}"), "{text}");

        // 1024x18 ties the two pins below the address, the write byte
        // enables, high.
        let (mut design, top, _map) = memory_design(18, 1024);
        run(&mut design, top, "ecp5-45f-CABGA381", &options);
        let text = design.to_text();
        assert!(text.contains("ADB={%waddr, 4'd3}"), "{text}");
    }

    #[test]
    fn contents_a_block_cannot_hold_are_reported() {
        // The generic RAMB says it can be initialised and not how: the
        // memory still becomes block RAM, blank, and the loss is a
        // warning rather than a silence.
        let (mut design, top, _map) = memory_design(8, 512);
        preload(&mut design, top, |a| a);
        let mut diags = Diagnostics::new();
        let report = map(
            &mut design,
            top,
            target("generic").unwrap(),
            &Constraints::new(),
            &MapOptions {
                insert_io_buffers: false,
                ..MapOptions::default()
            },
            &mut diags,
        );
        assert_eq!(report.block_rams.len(), 1);
        assert!(!report.block_rams[0].initialised);
        let lost: Vec<_> = diags
            .iter()
            .filter(|d| d.code == Some(NO_BRAM_INIT))
            .collect();
        assert_eq!(lost.len(), 1);
        assert_eq!(lost[0].severity, crate::diag::Severity::Warning);
        assert!(
            lost[0].message.contains("no `init_params`"),
            "{}",
            lost[0].message
        );
        assert!(cells_named(&design, top, "RAMB") == 1);
        let cell = design
            .module(top)
            .cells
            .iter()
            .map(|(_, c)| c)
            .find(|c| matches!(&c.kind, CellKind::Blackbox(n) if n.as_str() == "RAMB"))
            .unwrap();
        assert!(
            cell.params
                .iter()
                .all(|(k, _)| !k.as_str().starts_with("INIT"))
        );
    }

    #[test]
    fn a_rom_read_twice_gets_one_initialised_copy_per_read() {
        // No write port, two read ports, one read port per SB_RAM40_4K:
        // two copies, each holding every word.
        let (mut design, top, _map) = memory_design(8, 256);
        {
            let module = design.module_mut(top);
            let wr = module
                .cells
                .iter()
                .find(|(_, c)| c.name.as_str() == "wr")
                .map(|(id, _)| id)
                .unwrap();
            module.cells.retain(|id, _| id != wr);
        }
        add_second_read(&mut design, top, true);
        preload(&mut design, top, |a| 255 - a);
        let options = MapOptions {
            insert_io_buffers: false,
            insert_clock_buffers: false,
            ..MapOptions::default()
        };
        let report = run(&mut design, top, "ice40-hx1k-tq144", &options);
        let item = &report.block_rams[0];
        assert_eq!((item.copies, item.blocks()), (2, 2));
        assert!(item.initialised);
        assert!(
            report
                .notes
                .iter()
                .any(|n| n.contains("same initial contents")),
            "{:?}",
            report.notes
        );
        for copy in 0..2 {
            let cell = bram_cell(&design, top, &format!("ram$c{copy}$ram_w0_d0"));
            assert!(cell.input("WE").is_none() && cell.input("WDATA").is_none());
            for a in 0..256u32 {
                let param = format!("INIT_{:X}", a / 16);
                for j in 0..8 {
                    let at = (a % 16) * 16 + j;
                    let want = ((255 - a) >> j) & 1 == 1;
                    assert_eq!(init_bit(cell, &param, at), want, "copy {copy} word {a}");
                }
            }
        }
    }

    /// A design with `q = a * b` and, optionally, an accumulator.
    fn multiplier_design(width: u32, accumulate: bool) -> (Design, ModuleId, SourceMap) {
        let (map, span) = span();
        let mut b = ModuleBuilder::new("top", span);
        let clk = b.input("clk", Type::bit());
        let x = b.input("x", Type::bits(width));
        let y = b.input("y", Type::bits(width));
        let q = b.output("q", Type::bits(width * 2));
        let product = b.add_net("product", Type::bits(width * 2));
        let (x_e, y_e) = (b.net(x), b.net(y));
        let x_r = b.zext(x_e, width * 2);
        let y_r = b.zext(y_e, width * 2);
        b.cell2("mul", CellKind::Mul, x_r, y_r, product);
        if accumulate {
            let sum = b.add_net("sum", Type::bits(width * 2));
            let (p_e, q_e) = (b.net(product), b.net(q));
            b.cell2("acc", CellKind::Add, p_e, q_e, sum);
            let (clk_e, sum_e) = (b.net(clk), b.net(sum));
            b.cell(
                "areg",
                CellKind::Dff {
                    clk_pos: true,
                    has_enable: false,
                    reset: None,
                },
                vec![(Name::new("clk"), clk_e), (Name::new("d"), sum_e)],
                vec![(Name::new("q"), q)],
            );
        } else {
            let p_e = b.net(product);
            b.assign(q, p_e);
        }
        let mut design = Design::new();
        let top = design.add_module(b.finish());
        design.top = Some(top);
        (design, top, map)
    }

    #[test]
    fn recognises_multipliers_and_accumulators() {
        let options = MapOptions {
            insert_io_buffers: false,
            insert_clock_buffers: false,
            map_carry: false,
            ..MapOptions::default()
        };
        let (mut design, top, _map) = multiplier_design(8, false);
        let report = run(&mut design, top, "ecp5-25f-CABGA381", &options);
        let item = &report.dsps[0];
        assert_eq!(item.primitive, "MULT18X18D");
        assert!(!item.multiply_add && !item.accumulate);
        assert_eq!(cells_named(&design, top, "MULT18X18D"), 1);
        assert!(report.to_text().contains("MULT18X18D multiply\n"));

        let (mut design, top, _map) = multiplier_design(8, true);
        let report = run(&mut design, top, "ecp5-25f-CABGA381", &options);
        let item = &report.dsps[0];
        assert!(item.multiply_add && item.accumulate);
        assert_eq!(
            design
                .module(top)
                .cells
                .iter()
                .filter(|(_, c)| c.kind == CellKind::Add)
                .count(),
            0,
            "the adder is folded into the block"
        );
        assert!(report.to_text().contains("multiply-accumulate"));
    }

    #[test]
    fn declines_multipliers_that_do_not_fit() {
        let options = MapOptions {
            insert_io_buffers: false,
            insert_clock_buffers: false,
            map_carry: false,
            ..MapOptions::default()
        };
        // 24x24 is wider than an 18x18 block.
        let (mut design, top, _map) = multiplier_design(24, false);
        let report = run(&mut design, top, "ecp5-25f-CABGA381", &options);
        assert!(report.dsps.is_empty());
        assert!(
            report.notes[0].contains("fits no DSP block"),
            "{:?}",
            report.notes
        );
        // A device without DSP blocks says nothing at all.
        let (mut design, top, _map) = multiplier_design(8, false);
        let report = run(&mut design, top, "ice40-hx1k-tq144", &options);
        assert!(report.dsps.is_empty() && report.notes.is_empty());
    }

    /// A design with one `Add` of the given width.
    fn adder_design(width: u32) -> (Design, ModuleId, SourceMap) {
        let (map, span) = span();
        let mut b = ModuleBuilder::new("top", span);
        let x = b.input("x", Type::bits(width));
        let y = b.input("y", Type::bits(width));
        let q = b.output("q", Type::bits(width));
        let (x_e, y_e) = (b.net(x), b.net(y));
        b.cell2("sum", CellKind::Add, x_e, y_e, q);
        let mut design = Design::new();
        let top = design.add_module(b.finish());
        design.top = Some(top);
        (design, top, map)
    }

    #[test]
    fn maps_adders_onto_the_carry_chain() {
        let options = MapOptions {
            insert_io_buffers: false,
            insert_clock_buffers: false,
            ..MapOptions::default()
        };
        let (mut design, top, _map) = adder_design(8);
        let report = run(&mut design, top, "ice40-hx1k-tq144", &options);
        assert_eq!(report.carry_chains[0].primitive, "SB_CARRY");
        assert_eq!(report.carry_chains[0].width, 8);
        assert_eq!(report.carry_chains[0].primitives, 7);
        // Seven carries (the top bit needs none) and two XORs per bit.
        assert_eq!(cells_named(&design, top, "SB_CARRY"), 7);
        assert_eq!(
            design
                .module(top)
                .cells
                .iter()
                .filter(|(_, c)| c.kind == CellKind::Xor)
                .count(),
            16
        );
        assert!(report.to_text().contains("(8 bits) -> 7 x SB_CARRY"));

        // A narrow adder is left alone.
        let (mut design, top, _map) = adder_design(2);
        let report = run(&mut design, top, "ice40-hx1k-tq144", &options);
        assert!(report.carry_chains.is_empty());
        assert_eq!(cells_named(&design, top, "SB_CARRY"), 0);

        // A family whose carry element has no port map declines loudly.
        let (mut design, top, _map) = adder_design(8);
        let report = run(&mut design, top, "ecp5-25f-CABGA381", &options);
        assert!(report.carry_chains.is_empty());
        assert!(report.notes[0].contains("without a (ci, i0, i1, co) port map"));
    }

    #[test]
    fn ddr_ports_that_cannot_be_built_are_named() {
        // A three-bit DDR port has no pin for its odd bit; a clock that
        // does not exist cannot clock anything; the iCE40 has no IO
        // delay; and the ECP5's goes to 127 steps, not 500.
        let (mut sources, span) = span();
        let mut b = ModuleBuilder::new("top", span);
        let _clk = b.input("clk", Type::bit());
        let odd = b.input("odd", Type::bits(3));
        let lost = b.input("lost", Type::bits(2));
        let slow = b.input("slow", Type::bits(1));
        let q = b.output("q", Type::bits(6));
        let parts = vec![b.net(odd), b.net(lost), b.net(slow)];
        let all = b.concat(parts);
        b.assign(q, all);
        let mut design = Design::new();
        let top = design.add_module(b.finish());
        design.top = Some(top);
        let rcf = concat!(
            "set_io -ddr clk odd 8\n",
            "set_io -ddr nowhere lost 9\n",
            "set_io -delay 500 slow 78\n",
        );
        let file = sources.add("top.rcf", rcf).unwrap();
        let mut diags = Diagnostics::new();
        let constraints = Constraints::parse(rcf, file, &mut diags);
        assert!(!diags.has_errors(), "{}", diags.render(&sources));
        let report = map_with(
            &mut design,
            top,
            "ice40-hx1k-tq144",
            &constraints,
            &mut diags,
        );
        let text = diags.render(&sources);
        assert!(text.contains("it is 3 bits wide"), "{text}");
        assert!(text.contains("there is no clock net `nowhere`"), "{text}");
        assert!(text.contains("has no programmable IO delay"), "{text}");
        // Every one of them is still buffered, as an ordinary port.
        for port in ["odd", "lost", "slow"] {
            let item = report.io_buffers.iter().find(|i| i.port == port).unwrap();
            assert_eq!(item.ddr, None, "{port}");
            assert_eq!(item.delay, None, "{port}");
        }
        assert!(!validate(&design).has_errors());

        let (mut design, top, _) = generated_clocks(&[]);
        let rcf = "set_io -delay 500 d G2\n";
        let file = sources.add("ecp5.rcf", rcf).unwrap();
        let mut diags = Diagnostics::new();
        let constraints = Constraints::parse(rcf, file, &mut diags);
        map_with(
            &mut design,
            top,
            "ecp5-45f-CABGA381",
            &constraints,
            &mut diags,
        );
        let text = diags.render(&sources);
        assert!(text.contains("delays at most 127 steps"), "{text}");
    }

    #[test]
    fn inserts_io_buffers_with_constraints() {
        let (mut sources, span) = span();
        let mut b = ModuleBuilder::new("top", span);
        let d = b.input("d", Type::bits(2));
        let q = b.output("q", Type::bit());
        let d_e = b.net(d);
        let bit = b.slice(d_e, 0, 0);
        b.assign(q, bit);
        let mut design = Design::new();
        let top = design.add_module(b.finish());
        design.top = Some(top);

        let rcf = "set_io -io_standard LVCMOS33 -pullup yes q 99\n";
        let file = sources.add("top.rcf", rcf).unwrap();
        let mut diags = Diagnostics::new();
        let constraints = Constraints::parse(rcf, file, &mut diags);
        let report = map_with(
            &mut design,
            top,
            "ice40-hx1k-tq144",
            &constraints,
            &mut diags,
        );
        let _ = &sources;
        assert!(!diags.has_errors());
        assert_eq!(cells_named(&design, top, "SB_IO"), 3);
        let module = design.module(top);
        // The port now points at the pad net, and the core net is driven
        // by the buffers.
        assert_eq!(module.port("d").map(|p| p.net), module.net_by_name("d$pad"));
        assert!(
            validate(&design).is_empty(),
            "{:?}",
            validate(&design).len()
        );

        let item = report.io_buffers.iter().find(|i| i.port == "q").unwrap();
        assert_eq!(item.pin.as_deref(), Some("99"));
        assert_eq!(item.io_standard.as_deref(), Some("LVCMOS33"));
        let module = design.module(top);
        let cell = module.cell_by_name("q$io0").unwrap();
        let cell = &module.cells[cell];
        assert_eq!(
            cell.params.get("PIN_TYPE").and_then(AttrValue::as_int),
            Some(0b011001)
        );
        assert_eq!(
            cell.params.get("PULLUP").and_then(AttrValue::as_int),
            Some(1)
        );
        assert_eq!(
            cell.attrs.get("io_standard").and_then(AttrValue::as_str),
            Some("LVCMOS33")
        );
        let d_cell = module.cell_by_name("d$io1").unwrap();
        assert_eq!(
            module.cells[d_cell]
                .params
                .get("PIN_TYPE")
                .and_then(AttrValue::as_int),
            Some(0b000001)
        );
        assert!(
            report
                .to_text()
                .contains("q (1 bits) -> SB_IO at pin 99 as LVCMOS33")
        );
    }

    fn map_with(
        design: &mut Design,
        top: ModuleId,
        device: &str,
        constraints: &Constraints,
        diags: &mut Diagnostics,
    ) -> MapReport {
        map(
            design,
            top,
            target(device).unwrap(),
            constraints,
            &MapOptions::default(),
            diags,
        )
    }

    /// A design with `count` flip-flops on one clock.
    fn flops(count: u32) -> (Design, ModuleId, SourceMap) {
        let (map, span) = span();
        let mut b = ModuleBuilder::new("top", span);
        let clk = b.input("clk", Type::bit());
        let d = b.input("d", Type::bit());
        let (clk_e, d_e) = (b.net(clk), b.net(d));
        for i in 0..count {
            let q = b.add_net(format!("q{i}"), Type::bit());
            b.cell(
                format!("ff{i}"),
                CellKind::Dff {
                    clk_pos: true,
                    has_enable: false,
                    reset: Some(Reset {
                        asynchronous: false,
                        active_high: true,
                        value: Const::zero(1),
                    }),
                },
                vec![
                    (Name::new("clk"), clk_e),
                    (Name::new("d"), d_e),
                    (Name::new("rst"), d_e),
                ],
                vec![(Name::new("q"), q)],
            );
        }
        let mut design = Design::new();
        let top = design.add_module(b.finish());
        design.top = Some(top);
        (design, top, map)
    }

    /// A design with an input clock `clk` and one flip-flop on each of
    /// `generated`, internal nets that nothing drives.
    fn generated_clocks(generated: &[&str]) -> (Design, ModuleId, SourceMap) {
        let (map, span) = span();
        let mut b = ModuleBuilder::new("top", span);
        let _clk = b.input("clk", Type::bit());
        let d = b.input("d", Type::bit());
        let d_e = b.net(d);
        for (i, name) in generated.iter().enumerate() {
            let net = b.add_net(*name, Type::bit());
            let net_e = b.net(net);
            let q = b.add_net(format!("q{i}"), Type::bit());
            b.cell(
                format!("ff{i}"),
                CellKind::Dff {
                    clk_pos: true,
                    has_enable: false,
                    reset: None,
                },
                vec![(Name::new("clk"), net_e), (Name::new("d"), d_e)],
                vec![(Name::new("q"), q)],
            );
        }
        let mut design = Design::new();
        let top = design.add_module(b.finish());
        design.top = Some(top);
        (design, top, map)
    }

    /// Clock constraints, each `(net, MHz)`.
    fn clocks(clocks: &[(&str, f64)]) -> Constraints {
        let (_map, span) = span();
        let mut constraints = Constraints::new();
        for (net, mhz) in clocks {
            constraints
                .clocks
                .push(super::super::constraints::ClockDef {
                    name: (*net).to_owned(),
                    net: (*net).to_owned(),
                    period_ns: 1000.0 / mhz,
                    span,
                    origin: super::super::constraints::Origin::File,
                });
        }
        constraints
    }

    fn pll_options() -> MapOptions {
        MapOptions {
            insert_io_buffers: false,
            insert_clock_buffers: false,
            ..MapOptions::default()
        }
    }

    #[test]
    fn a_clock_constraint_on_an_undriven_net_instantiates_a_pll() {
        let (mut design, top, _map) = generated_clocks(&["sys"]);
        let constraints = clocks(&[("clk", 12.0), ("sys", 48.0)]);
        let mut diags = Diagnostics::new();
        let report = map(
            &mut design,
            top,
            target("ice40-hx1k-tq144").unwrap(),
            &constraints,
            &pll_options(),
            &mut diags,
        );
        assert_eq!(diags.len(), 0, "{:?}", diags.iter().next());
        let pll = &report.plls[0];
        assert_eq!(pll.net, "sys");
        assert_eq!(pll.source, "clk");
        assert_eq!(pll.primitive, "SB_PLL40_CORE");
        assert_eq!(pll.achieved_hz, 48_000_000);
        assert_eq!(pll.error_ppm(), 0.0);
        assert_eq!(cells_named(&design, top, "SB_PLL40_CORE"), 1);
        assert!(report.to_text().contains("sys -> SB_PLL40_CORE from clk"));
        assert!(!validate(&design).has_errors());
    }

    #[test]
    fn a_driven_clock_is_described_not_generated() {
        // `sys` is driven here, by an assignment from `clk`: the
        // constraint describes a clock that exists, so no PLL.
        let (mut design, top, _map) = generated_clocks(&["sys"]);
        let module = design.module_mut(top);
        let span = module.span;
        let (clk, sys) = (
            module.net_by_name("clk").unwrap(),
            module.net_by_name("sys").unwrap(),
        );
        let value = net_expr(module, clk, span);
        add_assign(module, sys, value, span);
        let constraints = clocks(&[("clk", 12.0), ("sys", 48.0)]);
        let mut diags = Diagnostics::new();
        let report = map(
            &mut design,
            top,
            target("ice40-hx1k-tq144").unwrap(),
            &constraints,
            &pll_options(),
            &mut diags,
        );
        assert!(report.plls.is_empty());
        assert_eq!(diags.len(), 0);
    }

    #[test]
    fn a_pll_that_cannot_be_built_is_reported() {
        // No input clock to generate it from.
        let (mut design, top, sources) = generated_clocks(&["sys"]);
        let mut diags = Diagnostics::new();
        let report = map(
            &mut design,
            top,
            target("ice40-hx1k-tq144").unwrap(),
            &clocks(&[("sys", 48.0)]),
            &pll_options(),
            &mut diags,
        );
        assert!(report.plls.is_empty());
        let text = diags.render(&sources);
        assert!(
            text.contains("no clock is constrained on an input port"),
            "{text}"
        );
        assert!(text.contains(NO_PLL), "{text}");

        // A device that describes no PLL.
        let (mut design, top, sources) = generated_clocks(&["sys"]);
        let mut diags = Diagnostics::new();
        map(
            &mut design,
            top,
            target("generic").unwrap(),
            &clocks(&[("clk", 12.0), ("sys", 48.0)]),
            &pll_options(),
            &mut diags,
        );
        let text = diags.render(&sources);
        assert!(text.contains("describes no PLL"), "{text}");

        // An HX1K has one PLL, so the second generated clock is refused.
        let (mut design, top, sources) = generated_clocks(&["a", "b"]);
        let mut diags = Diagnostics::new();
        let report = map(
            &mut design,
            top,
            target("ice40-hx1k-tq144").unwrap(),
            &clocks(&[("clk", 12.0), ("a", 48.0), ("b", 36.0)]),
            &pll_options(),
            &mut diags,
        );
        assert_eq!(report.plls.len(), 1);
        let text = diags.render(&sources);
        assert!(
            text.contains("has 1 PLL(s) and they are all in use"),
            "{text}"
        );
    }

    #[test]
    fn a_pll_far_off_the_request_is_warned_about() {
        // 25 MHz to 74.25 MHz on the ECP5 comes out at 75 MHz, one per
        // cent off: built, and said out loud.
        let (mut design, top, sources) = generated_clocks(&["pix"]);
        let mut diags = Diagnostics::new();
        let report = map(
            &mut design,
            top,
            target("ecp5-45f-CABGA381").unwrap(),
            &clocks(&[("clk", 25.0), ("pix", 74.25)]),
            &pll_options(),
            &mut diags,
        );
        assert_eq!(report.plls[0].achieved_hz, 75_000_000);
        let text = diags.render(&sources);
        assert!(text.contains("the nearest `EHXPLLL` can make"), "{text}");
        // The feedback is wired from the output, as FEEDBK_PATH says.
        let module = design.module(top);
        let (_, cell) = module
            .cells
            .iter()
            .find(|(_, c)| matches!(&c.kind, CellKind::Blackbox(n) if n.as_str() == "EHXPLLL"))
            .unwrap();
        let fb = cell.input("CLKFB").unwrap();
        assert_eq!(
            module.exprs[fb].as_net(),
            module.net_by_name("pix"),
            "CLKFB is wired to CLKOP"
        );
    }

    #[test]
    fn promotes_busy_clocks_onto_a_global_buffer() {
        let options = MapOptions {
            insert_io_buffers: false,
            global_buffer_threshold: 4,
            ..MapOptions::default()
        };
        let (mut design, top, _map) = flops(6);
        let report = run(&mut design, top, "ice40-hx1k-tq144", &options);
        assert_eq!(cells_named(&design, top, "SB_GB"), 1);
        let item = &report.clocks[0];
        assert_eq!(item.fanout, 6);
        assert_eq!(item.primitive.as_deref(), Some("SB_GB"));
        // Every flip-flop now reads the buffered net.
        let module = design.module(top);
        let buffered = module.net_by_name("clk$gb").unwrap();
        for (_, cell) in module.cells.iter() {
            if matches!(cell.kind, CellKind::Dff { .. }) {
                let clk = cell.input("clk").unwrap();
                assert_eq!(module.exprs[clk].as_net(), Some(buffered));
            }
        }
        assert!(report.to_text().contains("clk (6 flip-flops) -> SB_GB"));

        // Below the threshold the clock stays put.
        let (mut design, top, _map) = flops(2);
        let report = run(&mut design, top, "ice40-hx1k-tq144", &options);
        assert_eq!(cells_named(&design, top, "SB_GB"), 0);
        assert_eq!(report.clocks[0].primitive, None);
        assert!(
            report.clocks[0]
                .reason
                .as_deref()
                .unwrap()
                .contains("threshold")
        );
    }

    #[test]
    fn respects_the_number_of_global_buffers() {
        // A device with a single global buffer and two busy clocks.
        let (mut sources, span) = span();
        let mut b = ModuleBuilder::new("top", span);
        let clk_a = b.input("clk_a", Type::bit());
        let clk_b = b.input("clk_b", Type::bit());
        let d = b.input("d", Type::bit());
        let d_e = b.net(d);
        for (index, clk) in [clk_a, clk_b].into_iter().enumerate() {
            let clk_e = b.net(clk);
            for i in 0..4 {
                let q = b.add_net(format!("q{index}_{i}"), Type::bit());
                b.cell(
                    format!("ff{index}_{i}"),
                    CellKind::Dff {
                        clk_pos: true,
                        has_enable: false,
                        reset: None,
                    },
                    vec![(Name::new("clk"), clk_e), (Name::new("d"), d_e)],
                    vec![(Name::new("q"), q)],
                );
            }
        }
        let mut design = Design::new();
        let top = design.add_module(b.finish());
        design.top = Some(top);

        let mut device = target("ice40-hx1k-tq144").unwrap().clone();
        device.clock_resources.global_buffers = 1;
        let _ = &mut sources;
        let mut diags = Diagnostics::new();
        let options = MapOptions {
            insert_io_buffers: false,
            global_buffer_threshold: 2,
            ..MapOptions::default()
        };
        let report = map(
            &mut design,
            top,
            &device,
            &Constraints::new(),
            &options,
            &mut diags,
        );
        assert_eq!(cells_named(&design, top, "SB_GB"), 1);
        assert_eq!(report.clocks[0].primitive.as_deref(), Some("SB_GB"));
        assert_eq!(report.clocks[1].primitive, None);
        let text = diags.render(&sources);
        assert!(text.contains("has only 1 global buffer(s)"), "{text}");
    }

    #[test]
    fn a_module_without_a_device_feature_is_left_alone() {
        let mut device = Device::new("bare", "bare");
        device.lut_size = 4;
        let (mut design, top, _map) = adder_design(8);
        let mut diags = Diagnostics::new();
        let report = map(
            &mut design,
            top,
            &device,
            &Constraints::new(),
            &MapOptions::default(),
            &mut diags,
        );
        assert!(report.carry_chains.is_empty());
        assert!(report.io_buffers.is_empty());
        assert!(
            report
                .notes
                .iter()
                .any(|n| n.contains("describes no IO buffer"))
        );
        assert!(!report.is_empty());
        assert!(MapReport::default().is_empty());
    }

    #[test]
    fn options_switch_every_step_off() {
        let options = MapOptions {
            infer_block_ram: false,
            infer_dsp: false,
            insert_io_buffers: false,
            insert_clock_buffers: false,
            map_carry: false,
            ..MapOptions::default()
        };
        let (mut design, top, _map) = memory_design(8, 512);
        let report = run(&mut design, top, "ice40-hx1k-tq144", &options);
        assert!(report.is_empty());
        assert_eq!(design.module(top).memories.len(), 1);
    }

    #[test]
    fn processes_are_not_disturbed() {
        // A module still in process form keeps its process; only the ports
        // gain buffers.
        let (sources, span) = span();
        let mut b = ModuleBuilder::new("top", span);
        let clk = b.input("clk", Type::bit());
        let q = b.output_reg("q", Type::bit());
        let one = b.const_bit(true);
        let mut p = b.process(None, ProcessKind::posedge(clk));
        p.nonblocking(q, one);
        b.end_process(p);
        let mut design = Design::new();
        let top = design.add_module(b.finish());
        design.top = Some(top);
        let mut diags = Diagnostics::new();
        let report = map_with(
            &mut design,
            top,
            "ice40-hx1k-tq144",
            &Constraints::new(),
            &mut diags,
        );
        assert_eq!(design.module(top).processes.len(), 1);
        assert_eq!(report.io_buffers.len(), 2);
        let _ = sources;
    }

    #[test]
    fn helpers_do_their_arithmetic() {
        assert_eq!(addr_bits(0), 0);
        assert_eq!(addr_bits(1), 0);
        assert_eq!(addr_bits(2), 1);
        assert_eq!(addr_bits(256), 8);
        assert_eq!(addr_bits(257), 9);
        assert_eq!(div_ceil_u32(7, 4), 2);
        assert_eq!(div_ceil_u32(8, 4), 2);
    }
}
