//! The last mapping step: generic LUTs and flip-flops become the
//! family's own primitives.
//!
//! [`super::primitives`] maps what a device does in *hard* logic (block
//! RAM, DSP, carry, IO, clock buffers) and the technology mapper
//! ([`crate::synth::techmap`]) covers everything else with generic
//! [`CellKind::Lut`] cells. What is left after those two are cells that
//! the fabric implements but that still carry Reticle's own names: a
//! `lut` and a `dff` rather than an `SB_LUT4` and an `SB_DFFESR`. A
//! place-and-route tool knows nothing about the generic ones, so
//! [`map_cells`] rewrites them into the device's primitives, with the
//! parameters those primitives take, which is the last thing between a
//! synthesised design and nextpnr.
//!
//! Everything here is driven by the device database, never by the family
//! name:
//!
//! | Generic cell | Becomes | Parameters |
//! |--------------|---------|------------|
//! | [`CellKind::Lut`] | the [`BelRole::Lut`] primitive | its declared parameter, holding the truth table |
//! | [`CellKind::Dff`] | the [`BelRole::Ff`] primitive whose `mode` matches, one per bit | the parameters that `bel` line declares |
//!
//! # LUT init ordering
//!
//! A mapped LUT's `init` has one bit per input pattern, **bit `i` being
//! the output for the pattern whose bit `j` is the value of input `j`**,
//! input 0 the least significant (see [`crate::synth::techmap`]). Both
//! families Reticle ships use exactly that order — an `SB_LUT4`'s
//! `LUT_INIT` bit `i` is the output for `{I3,I2,I1,I0} == i`, and an ECP5
//! `LUT4`'s `INIT` bit `i` for `{D,C,B,A} == i` — so the truth table is
//! written out unchanged and the *j*-th declared input port (`I0`, or
//! `A`) is fed input `j`. A family that numbered its pins the other way
//! round would need its `.dev` file to list the ports in its own order,
//! which is why the order of the `i=...` list is part of the database.
//!
//! A LUT of fewer inputs than the device's is widened rather than left
//! partly connected: the unused inputs are tied to zero and the truth
//! table is repeated `2^(K-k)` times, so the function ignores them. That
//! keeps the primitive's parameter exactly as wide as the family expects
//! (16 bits for a 4-input family), which is what the tools assume.
//!
//! # Flip-flops
//!
//! A `dff` cell is as wide as the register it came from; a fabric
//! flip-flop is one bit. A wide cell therefore becomes one primitive per
//! bit, driving one new single-bit net each, with a concatenation
//! assigned to the original net so that everything reading it is
//! undisturbed.
//!
//! Which primitive a bit gets is decided by the [`FfVariant`] the device
//! declares: the clock edge, whether there is a clock enable, and the
//! kind and polarity of the set or reset. The *value* the reset loads
//! decides set versus reset per bit, so a register reset to `8'b00010000`
//! becomes seven resetting flip-flops and one setting one.
//!
//! # Polarity the family does not have
//!
//! Every `SB_DFF*` set and reset pin is active high, and
//! `always @(posedge clk or negedge rst_n)` is how nearly all real HDL is
//! written, so matching polarity exactly would refuse most designs on
//! iCE40. It is not refused: when the device declares the same flip-flop
//! with the *other* polarity, the net feeding the pin is inverted and
//! that primitive is used. The same is done for a clock enable whose
//! polarity the family lacks (`enable_low` in a `mode` clause).
//!
//! The inverter is one of the device's own LUTs, and there is **one per
//! net**, not one per flip-flop: a reset fanning out to two hundred flops
//! costs one LUT, which is what a vendor flow does too.
//! [`CellMapReport::inverted`] lists the nets and
//! [`CellMapReport::inverters`] counts them. This mirrors
//! [`crate::asic::library`], which inserts an inverter rather than
//! refusing a polarity no standard cell has.
//!
//! # A constant on a flip-flop's data input
//!
//! A lookup table can absorb a constant input into its truth table; a
//! flip-flop's data input cannot absorb anything, so a register bit that is
//! genuinely constant — `reg [2:0] stage` for four states, where `stage[2]`
//! is always zero — needs something to **drive** that constant. On a real
//! fabric there is nothing to leave it connected to: on an ECP5 the data
//! comes off the fabric's `M` wire, and an unrouted slice input on that
//! family reads as a **one**, so the bit comes up set and the design reads a
//! value it cannot produce. That cost eight rounds of investigation on a USB
//! device that would not enumerate; `docs/fpga-trellis.md` has it in full.
//!
//! So [`map_cells`] finishes by giving every such flip-flop a driver:
//! **one lookup table per constant**, shared by every flip-flop of the
//! module that wants that value, with a truth table that is all zeros or all
//! ones and therefore ignores every input. That is what a vendor flow does —
//! nextpnr's `pack_constants` builds a `$PACKER_GND` and a `$PACKER_VCC`
//! LUT4 with `INIT` 0 and 0xFFFF — and it is what Lattice's own bitstreams
//! for a Cynthion contain, measured rather than assumed: one `SLICEA.K0`
//! with `INIT` all zeros and one with all ones per design, every input tied
//! high, feeding eight to thirty-two flip-flops each across the whole die,
//! and not one flip-flop anywhere with nothing routed to its data wire.
//! `tests/fpga_trellis.rs`'s
//! `what_lattices_own_packer_writes_for_a_constant` is that measurement.
//!
//! Three things about it are deliberate:
//!
//! - **the constant is decided on the bit-level view**
//!   ([`crate::ir::emit::BitView`]), so a constant that reaches the pin
//!   through a continuous assignment, a slice or a concatenation counts as
//!   one. The netlist builder resolves the pin the same way, which is what
//!   makes this the same question the backend would have asked;
//! - **the driver is placed like any other LUT.** Nothing here chooses a
//!   site: the cell goes into the netlist and the placer allocates it, so it
//!   cannot collide with a LUT the placer has already used, and a design with
//!   no spare LUT is a placement error with the message placement errors
//!   have;
//! - **an `x` or a `z` is not built.** There is no wire value for either, so
//!   such a pin is left as it is, and the backend refuses the flip-flop by
//!   name and position rather than inventing a level for it.
//!
//! [`CellMapReport::constants`] lists the flip-flops; the LUTs themselves
//! are counted in [`CellMapReport::primitives`] like any other.
//!
//! What is *not* fixed this way is the **clock edge**. Inverting a clock
//! makes a second clock network with its own skew and duty-cycle
//! distortion, which is a physical-design decision and not a mapper's to
//! take, so a design wanting an edge the family does not declare is still
//! reported ([`NO_FF_VARIANT`]) and left generic — as is a polarity on a
//! device that declares no LUT to invert with. Leaving such a cell
//! generic rather than approximating it is deliberate: [`super::flow`]'s
//! netlist check then reports it as not a device primitive, and the user
//! sees one honest error instead of a netlist that simulates differently
//! from the design.
//!
//! [`BelRole::Lut`]: super::device::BelRole::Lut
//! [`BelRole::Ff`]: super::device::BelRole::Ff
//! [`FfVariant`]: super::device::FfVariant

use std::collections::BTreeMap;
use std::fmt::Write as _;

use super::device::{BelKind, BelRole, Device, FfReset, FfVariant};
use super::primitives::{add_assign, add_cell, add_net, const_expr, expr, net_expr, slice_expr};
use crate::diag::{Diagnostic, Diagnostics};
use crate::ir::emit::{BitView, SigBit};
use crate::ir::{
    AttrValue, Bit, Cell, CellId, CellKind, Const, Design, ExprId, ExprKind, Module, ModuleId,
    Name, NetId, Type,
};

/// Diagnostic code for a flip-flop the device has no primitive for.
pub const NO_FF_VARIANT: &str = "F0310";
/// Diagnostic code for a device that cannot express a mapped LUT.
pub const NO_LUT_PRIMITIVE: &str = "F0311";

/// What [`map_cells`] did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CellMapReport {
    /// How many of each device primitive the rewrite produced, sorted by
    /// primitive name.
    pub primitives: Vec<(String, u32)>,
    /// The cells that stayed generic, as `(cell, why)`, in cell order.
    pub declined: Vec<(String, String)>,
    /// Nets given an inverter because the device declares no flip-flop of
    /// the polarity the design asked for, as `(net, which pin it feeds)`.
    ///
    /// One entry per net, however many flip-flops share it; the inverter
    /// itself is counted in `primitives` like any other LUT.
    pub inverted: Vec<(String, String)>,
    /// Flip-flops whose data input was a constant and now has a lookup
    /// table driving it, as `(cell, the constant)`, in cell order.
    ///
    /// One entry per flip-flop and **one LUT per constant**, shared by
    /// every flip-flop of the module that wants that value, which is what
    /// nextpnr's `pack_constants` does and what Lattice's own bitstreams
    /// for this board contain. The LUTs themselves are counted in
    /// `primitives` like any other.
    pub constants: Vec<(String, Bit)>,
}

impl CellMapReport {
    /// True when nothing was rewritten and nothing was declined.
    pub fn is_empty(&self) -> bool {
        self.primitives.is_empty()
            && self.declined.is_empty()
            && self.inverted.is_empty()
            && self.constants.is_empty()
    }

    /// How many flip-flops were given a constant driver, which is not the
    /// number of drivers: there is one LUT per constant, not per flop.
    pub fn constant_data_pins(&self) -> usize {
        self.constants.len()
    }

    /// How many polarity inverters were inserted, which is how many nets
    /// needed one.
    pub fn inverters(&self) -> usize {
        self.inverted.len()
    }

    /// How many instances of `primitive` were produced.
    pub fn count(&self, primitive: &str) -> u32 {
        self.primitives
            .iter()
            .find(|(name, _)| name == primitive)
            .map_or(0, |(_, n)| *n)
    }

    /// The total number of primitives produced.
    pub fn total(&self) -> u32 {
        self.primitives.iter().map(|(_, n)| *n).sum()
    }

    /// Renders the report as plain text, one line per primitive.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        for (name, count) in &self.primitives {
            let _ = writeln!(out, "  {count} x {name}");
        }
        for (net, pin) in &self.inverted {
            let _ = writeln!(out, "  {net} inverted for the {pin} the device has");
        }
        for (cell, value) in &self.constants {
            let _ = writeln!(
                out,
                "  {cell}'s data input is the constant {value}, now driven"
            );
        }
        for (cell, why) in &self.declined {
            let _ = writeln!(out, "  {cell} stays generic ({why})");
        }
        out
    }

    fn bump(&mut self, primitive: &str, n: u32) {
        match self
            .primitives
            .iter_mut()
            .find(|(name, _)| name == primitive)
        {
            Some((_, count)) => *count += n,
            None => {
                self.primitives.push((primitive.to_owned(), n));
                self.primitives.sort();
            }
        }
    }
}

/// Rewrites the generic LUTs and flip-flops of `module` into `device`'s
/// primitives, in place.
///
/// Cells that are already black boxes (what [`super::map`] produced) and
/// everything else are left alone. A cell the device cannot express stays
/// as it is and is reported through `diags`; see the module docs.
pub fn map_cells(
    design: &mut Design,
    module: ModuleId,
    device: &Device,
    diags: &mut Diagnostics,
) -> CellMapReport {
    let mut report = CellMapReport::default();
    let Some(module) = design.modules.get_mut(module) else {
        return report;
    };
    map_luts(module, device, &mut report, diags);
    map_flip_flops(module, device, &mut report, diags);
    drive_constant_data(module, device, &mut report, diags);
    report
}

// --- lookup tables ----------------------------------------------------------

/// The parameter a LUT primitive carries its truth table in, and how many
/// bits the family expects it to have.
///
/// The database states both: the primitive's first (and, for every family
/// Reticle ships, only) declared parameter is the init, and the width of
/// the default value it is declared with is the width the family expects.
fn lut_init_param(bel: &BelKind, inputs: u32) -> (Name, u32) {
    let default_width = 1u32 << inputs.min(31);
    match bel.params.first() {
        Some((name, AttrValue::Const(value))) => (Name::new(name.clone()), value.width()),
        Some((name, _)) => (Name::new(name.clone()), default_width),
        None => (Name::new("INIT"), default_width),
    }
}

fn map_luts(
    module: &mut Module,
    device: &Device,
    report: &mut CellMapReport,
    diags: &mut Diagnostics,
) {
    let luts: Vec<CellId> = module
        .cells
        .iter()
        .filter(|(_, c)| matches!(c.kind, CellKind::Lut { .. }))
        .map(|(id, _)| id)
        .collect();
    if luts.is_empty() {
        return;
    }
    let Some(bel) = device.bel(BelRole::Lut).cloned() else {
        let span = module.cells[luts[0]].span;
        diags.push(
            Diagnostic::error(format!(
                "`{}` declares no LUT primitive, so {} mapped LUT(s) stay generic",
                device.name,
                luts.len()
            ))
            .with_code(NO_LUT_PRIMITIVE)
            .with_span(span)
            .with_note("add a `bel <name> lut port i=... o=...` line to the device file"),
        );
        for id in luts {
            report.declined.push((
                module.cells[id].name.as_str().to_owned(),
                format!("`{}` declares no LUT primitive", device.name),
            ));
        }
        return;
    };
    for id in luts {
        map_lut(module, device, &bel, id, report, diags);
    }
}

fn map_lut(
    module: &mut Module,
    device: &Device,
    bel: &BelKind,
    id: CellId,
    report: &mut CellMapReport,
    diags: &mut Diagnostics,
) {
    let cell = &module.cells[id];
    let CellKind::Lut { k, init } = cell.kind.clone() else {
        return;
    };
    let name = cell.name.as_str().to_owned();
    let span = cell.span;
    let (Some(a), Some(y)) = (cell.input("a"), cell.output("y")) else {
        report
            .declined
            .push((name, "the cell is not wired".to_owned()));
        return;
    };
    // A port list of one name is a bus (the generic family's `i=I`); a
    // list of several is one pin per input, input 0 first.
    let pins: Vec<Name> = bel.port_names("i").into_iter().map(Name::new).collect();
    let width = if pins.len() > 1 {
        u32::try_from(pins.len()).unwrap_or(device.lut_size)
    } else {
        device.lut_size
    };
    // The second half of the condition is not a real family's problem:
    // a truth table has 2^width bits, so a file claiming a LUT wider
    // than 16 inputs is refused rather than built.
    if k > width || width > 16 {
        diags.push(
            Diagnostic::error(format!(
                "cell `{name}` is a {k}-input LUT, but `{}` has {width}-input LUTs",
                device.name
            ))
            .with_code(NO_LUT_PRIMITIVE)
            .with_span(span)
            .with_note("map the logic with the device's LUT size"),
        );
        report
            .declined
            .push((name, format!("{k} inputs do not fit a {width}-input LUT")));
        return;
    }
    // Widen the truth table to the primitive's: repeating it once per
    // value of each added input makes the function ignore them, and the
    // added inputs are tied low.
    let copies = 1u32 << (width - k);
    let init = init.replicate(copies);
    let (param, param_width) = lut_init_param(bel, width);
    let init = if init.width() == param_width {
        init
    } else {
        init.resize(param_width)
    };

    let mut inputs: Vec<(Name, ExprId)> = Vec::new();
    if pins.len() > 1 {
        for (index, pin) in pins.iter().enumerate() {
            let bit = u32::try_from(index).unwrap_or(0);
            let value = if bit < k {
                slice_expr(module, a, bit, bit, span)
            } else {
                const_expr(module, Const::zero(1), span)
            };
            inputs.push((pin.clone(), value));
        }
    } else {
        let pin = pins.first().cloned().unwrap_or_else(|| Name::new("I"));
        let value = if k == width {
            a
        } else {
            let mut parts = vec![const_expr(module, Const::zero(width - k), span)];
            parts.push(a);
            expr(module, ExprKind::Concat(parts), span)
        };
        inputs.push((pin, value));
    }
    let out = Name::new(bel.port("o").unwrap_or("O"));

    let cell = &mut module.cells[id];
    cell.kind = CellKind::Blackbox(Name::new(bel.name.clone()));
    cell.inputs = inputs;
    cell.outputs = vec![(out, y)];
    cell.params.set(param, AttrValue::Const(init));
    report.bump(&bel.name, 1);
}

// --- flip-flops -------------------------------------------------------------

fn map_flip_flops(
    module: &mut Module,
    device: &Device,
    report: &mut CellMapReport,
    diags: &mut Diagnostics,
) {
    let ffs: Vec<CellId> = module
        .cells
        .iter()
        .filter(|(_, c)| matches!(c.kind, CellKind::Dff { .. }))
        .map(|(id, _)| id)
        .collect();
    if ffs.is_empty() {
        return;
    }
    let mut ctx = FfCtx {
        device,
        lut: device.bel(BelRole::Lut).cloned(),
        inverted: BTreeMap::new(),
        report,
        diags,
    };
    let mut replaced: Vec<CellId> = Vec::new();
    for id in ffs {
        if map_flip_flop(module, &mut ctx, id) {
            replaced.push(id);
        }
    }
    if !replaced.is_empty() {
        module.cells.retain(|id, _| !replaced.contains(&id));
    }
}

/// What flip-flop mapping carries from one cell to the next.
///
/// The inverter cache is the whole reason this exists: a reset net that
/// two hundred flip-flops read is inverted once, not two hundred times.
struct FfCtx<'a> {
    /// The device being mapped onto.
    device: &'a Device,
    /// Its LUT primitive, which is what an inverter is built from;
    /// `None` when the device declares none, in which case no polarity
    /// can be fixed and a mismatch is reported as before.
    lut: Option<BelKind>,
    /// The complement of each net that has one, by net.
    inverted: BTreeMap<NetId, NetId>,
    /// Where the counts, the inversions and the declines go.
    report: &'a mut CellMapReport,
    /// Where the refusals go.
    diags: &'a mut Diagnostics,
}

impl FfCtx<'_> {
    /// The expression carrying the complement of `signal`, building the
    /// inverter the first time a net asks for one.
    ///
    /// The inverter is a one-input LUT of the device's own kind, mapped
    /// by the same path as any other LUT so that it picks up the
    /// family's parameter name, its width and its pin order with no
    /// special case. `pin` names what the complement feeds, for the
    /// report.
    fn invert(
        &mut self,
        module: &mut Module,
        signal: ExprId,
        pin: &str,
        span: crate::source::Span,
    ) -> ExprId {
        let bel = self
            .lut
            .clone()
            .expect("a polarity is only flipped when the device has a LUT");
        let key = module.expr(signal).as_net();
        if let Some(net) = key.and_then(|net| self.inverted.get(&net)) {
            return net_expr(module, *net, span);
        }
        let base = match key {
            Some(net) => format!("{}$n", module.nets[net].name),
            None => "ff$n".to_owned(),
        };
        let out = add_net(module, &base, Type::bit(), span);
        let cell = add_cell(
            module,
            &format!("{base}$inv"),
            CellKind::Lut {
                k: 1,
                // Bit `i` is the output for input pattern `i`: a one for
                // a zero in, a zero for a one in.
                init: Const::from_u64(0b01, 2),
            },
            vec![(Name::new("a"), signal)],
            vec![(Name::new("y"), out)],
            span,
        );
        map_lut(module, self.device, &bel, cell, self.report, self.diags);
        let named = match key {
            Some(net) => {
                self.inverted.insert(net, out);
                module.nets[net].name.as_str().to_owned()
            }
            None => base,
        };
        self.report.inverted.push((named, pin.to_owned()));
        net_expr(module, out, span)
    }

    /// The clock enable and set/reset one bit presents to its primitive,
    /// inverted where that primitive's pin has the other polarity.
    fn control(
        &mut self,
        module: &mut Module,
        matched: &FfMatch,
        signals: (Option<ExprId>, Option<ExprId>),
        span: crate::source::Span,
    ) -> (Option<ExprId>, Option<ExprId>) {
        let (en, rst) = signals;
        let en = match (matched.invert_enable, en) {
            (true, Some(e)) => Some(self.invert(module, e, "clock enable", span)),
            (_, other) => other,
        };
        let rst = match (matched.invert_reset, rst) {
            (true, Some(e)) => Some(self.invert(module, e, "set/reset", span)),
            (_, other) => other,
        };
        (en, rst)
    }
}

/// One device flip-flop that can implement what a bit asked for, and
/// which of its inputs has to be inverted on the way in.
struct FfMatch {
    /// The primitive to instantiate.
    bel: BelKind,
    /// True when the set/reset net must be inverted first.
    invert_reset: bool,
    /// True when the clock-enable net must be inverted first.
    invert_enable: bool,
}

/// The device flip-flop to use for `want`, inverting a set/reset or a
/// clock enable when that is what it takes.
///
/// Candidates are tried in order of cost: the exact variant first, then
/// one inversion, then two. The clock edge is never inverted; see the
/// module docs.
fn resolve_variant(device: &Device, want: FfVariant, can_invert: bool) -> Option<FfMatch> {
    let options: [(Option<FfVariant>, bool, bool); 4] = [
        (Some(want), false, false),
        (want.with_flipped_reset(), true, false),
        (want.with_flipped_enable(), false, true),
        (
            want.with_flipped_reset()
                .and_then(FfVariant::with_flipped_enable),
            true,
            true,
        ),
    ];
    for (variant, invert_reset, invert_enable) in options {
        let Some(variant) = variant else { continue };
        if (invert_reset || invert_enable) && !can_invert {
            continue;
        }
        if let Some(bel) = device.ff_variant(variant) {
            return Some(FfMatch {
                bel: bel.clone(),
                invert_reset,
                invert_enable,
            });
        }
    }
    None
}

/// The variant bit `bit` of `cell` needs.
fn variant_of(kind: &CellKind, bit: u32) -> FfVariant {
    let CellKind::Dff {
        clk_pos,
        has_enable,
        reset,
    } = kind
    else {
        return FfVariant::plain();
    };
    FfVariant {
        clk_pos: *clk_pos,
        has_enable: *has_enable,
        // The IR's clock enable is active high; a family whose only
        // enable pin is active low gets an inverter, not a refusal.
        enable_active_high: true,
        reset: reset.as_ref().map(|r| FfReset {
            asynchronous: r.asynchronous,
            // An `x` in the reset value is a don't-care; taking it as a
            // zero keeps the flip-flop the simpler of the two.
            sets: r.value.bit(bit) == Bit::One,
            active_high: r.active_high,
        }),
    }
}

/// Maps one flip-flop; returns true when the original cell is to be
/// removed because per-bit primitives replaced it.
fn map_flip_flop(module: &mut Module, ctx: &mut FfCtx<'_>, id: CellId) -> bool {
    let device = ctx.device;
    let cell = &module.cells[id];
    let kind = cell.kind.clone();
    let name = cell.name.as_str().to_owned();
    let span = cell.span;
    let (Some(clk), Some(d), Some(q)) = (cell.input("clk"), cell.input("d"), cell.output("q"))
    else {
        ctx.report
            .declined
            .push((name, "the cell is not wired".to_owned()));
        return false;
    };
    let en = cell.input("en");
    let rst = cell.input("rst");
    let width = module.nets.get(q).and_then(|n| n.ty.width()).unwrap_or(1);

    // Every bit must be mappable before anything is rewritten, so that a
    // register with one impossible bit is reported whole rather than left
    // half converted.
    let can_invert = ctx.lut.is_some();
    let mut matches: Vec<FfMatch> = Vec::with_capacity(usize::try_from(width).unwrap_or(0));
    for bit in 0..width {
        let variant = variant_of(&kind, bit);
        match resolve_variant(device, variant, can_invert) {
            Some(matched) => matches.push(matched),
            None => {
                ctx.diags.push(
                    Diagnostic::error(format!(
                        "`{}` has no flip-flop with {}",
                        device.name,
                        variant.describe()
                    ))
                    .with_code(NO_FF_VARIANT)
                    .with_span(span)
                    .with_note(format!(
                        "flip-flop `{name}`{} needs one, and stays a generic cell that a place-and-route tool will reject",
                        if width > 1 { format!(" (bit {bit})") } else { String::new() }
                    ))
                    .with_note(format!(
                        "`{}` declares {}",
                        device.name,
                        declared_variants(device)
                    )),
                );
                ctx.report.declined.push((
                    name,
                    format!(
                        "`{}` has no flip-flop with {}",
                        device.name,
                        variant.describe()
                    ),
                ));
                return false;
            }
        }
    }

    if width == 1 {
        let matched = &matches[0];
        let (en, rst) = ctx.control(module, matched, (en, rst), span);
        let bel = &matched.bel;
        let mut inputs = vec![
            (Name::new(bel.port("clk").unwrap_or("C")), clk),
            (Name::new(bel.port("d").unwrap_or("D")), d),
        ];
        connect_control(module, bel, &mut inputs, en, rst, span);
        let out = Name::new(bel.port("q").unwrap_or("Q"));
        let cell = &mut module.cells[id];
        cell.kind = CellKind::Blackbox(Name::new(bel.name.clone()));
        cell.inputs = inputs;
        cell.outputs = vec![(out, q)];
        set_params(cell, bel);
        ctx.report.bump(&bel.name, 1);
        return false;
    }

    let mut bits = Vec::with_capacity(usize::try_from(width).unwrap_or(0));
    for bit in 0..width {
        let matched = &matches[usize::try_from(bit).unwrap_or(0)];
        let (en, rst) = ctx.control(module, matched, (en, rst), span);
        let bel = &matched.bel;
        let net = add_net(module, &format!("{name}$q{bit}"), Type::bit(), span);
        let d_bit = slice_expr(module, d, bit, bit, span);
        let mut inputs = vec![
            (Name::new(bel.port("clk").unwrap_or("C")), clk),
            (Name::new(bel.port("d").unwrap_or("D")), d_bit),
        ];
        connect_control(module, bel, &mut inputs, en, rst, span);
        let new = add_cell(
            module,
            &format!("{name}$ff{bit}"),
            CellKind::Blackbox(Name::new(bel.name.clone())),
            inputs,
            vec![(Name::new(bel.port("q").unwrap_or("Q")), net)],
            span,
        );
        set_params(&mut module.cells[new], bel);
        ctx.report.bump(&bel.name, 1);
        bits.push(net_expr(module, net, span));
    }
    bits.reverse();
    let value = expr(module, ExprKind::Concat(bits), span);
    add_assign(module, q, value, span);
    true
}

/// Adds the enable and set/reset inputs a variant has, tying an input the
/// primitive has but the flip-flop does not to its inactive level.
fn connect_control(
    module: &mut Module,
    bel: &BelKind,
    inputs: &mut Vec<(Name, ExprId)>,
    en: Option<ExprId>,
    rst: Option<ExprId>,
    span: crate::source::Span,
) {
    let variant = bel.ff.unwrap_or_else(FfVariant::plain);
    if let Some(port) = bel.port("en") {
        let value = match (variant.has_enable, en) {
            (true, Some(en)) => Some(en),
            // The primitive has a clock-enable pin but this variant does
            // not use it (an ECP5 `TRELLIS_FF` with `CEMUX="1"`): hold it
            // high so the flop always captures.
            (false, _) => Some(const_expr(module, Const::ones(1), span)),
            (true, None) => None,
        };
        if let Some(value) = value {
            inputs.push((Name::new(port), value));
        }
    }
    if let Some(port) = bel.port("rst") {
        let value = match (variant.reset, rst) {
            (Some(_), Some(rst)) => Some(rst),
            // Likewise for an unused set/reset pin: drive it to the level
            // that does nothing, which is the inactive one.
            (None, _) => Some(const_expr(module, Const::zero(1), span)),
            (Some(_), None) => None,
        };
        if let Some(value) = value {
            inputs.push((Name::new(port), value));
        }
    }
}

/// Copies the parameters the device declares for a primitive onto a cell.
fn set_params(cell: &mut Cell, bel: &BelKind) {
    for (key, value) in &bel.params {
        cell.params.set(Name::new(key.clone()), value.clone());
    }
}

// --- a constant on a flip-flop's data input ---------------------------------

/// Gives every pin that is a constant and cannot absorb one something that
/// drives it: one lookup table per constant, shared by every cell of the
/// module that asks for that value.
///
/// See the module docs for why a fabric flip-flop cannot absorb a constant
/// the way a lookup table can, and
/// `tests/fpga_trellis.rs`'s `what_lattices_own_packer_writes_for_a_constant`
/// for the vendor bitstreams this shape was read out of.
///
/// A flip-flop's data input is the pin this was written for. A distributed
/// RAM's inputs are in exactly the same position and for a sharper reason:
/// an unrouted lookup-table input is *tied high* on this family
/// (`SLICE<l>.<X><n>MUX = 1`), so a write-data or address bit left as a
/// constant zero would read as a one, and a memory addressed one word off
/// is not something a structural check would notice. Every input of a
/// `lutram` primitive is therefore given a real driver, which is what
/// nextpnr's `pack_constants` does for all of them.
///
/// # And a carry element's operands, which cost a board to find
///
/// A **carry** element's inputs are the third case and they were the
/// expensive one. `cnt + 1` is an adder whose second operand is a
/// constant, so thirty-one of its thirty-two `b` pins are a constant
/// **zero** — and a constant on a carry cell's operand pin is not a wire
/// anything routes, so on an ECP5 every one of them read as a **one** and
/// the chain computed `cnt - 1`.
///
/// Nothing off the part could see it. The design synthesised, placed,
/// routed, and every set bit of its bitstream decoded back through the
/// database into the arcs the router chose; the exhaustive check in
/// `src/fpga/primitives.rs` and the SAT proof in `tests/fpga_carry.rs`
/// both passed, because in a netlist a constant pin evaluates to the
/// constant. What found it was a Cynthion: the USB device design of
/// `CLAUDE.md`'s own example stopped enumerating, and the build of the
/// same design with carry inference switched off was byte-identical to
/// the bitstream that works.
///
/// So this pass covers every input of a `carry` primitive too, with two
/// exclusions and both of them are the chain's own ends. `ci` is
/// dedicated metal from the cell below and nothing outside may drive it.
/// `cyinit` is the pin a family *designs* to take a constant — the
/// 7-series' own way in, which [`WideCarry`](super::device::WideCarry)
/// describes as "the chain goes on `ci` and the constant on `init`" — so
/// it can absorb one by construction, and giving it a lookup table would
/// cost one per chain for nothing.
///
/// A pin the element could absorb the constant into — an ECP5 `CCU2C`
/// lane whose `INIT` could be rewritten, which is what `ecppack` does —
/// would be cheaper by a net, and is not done: the driver is correct on
/// every family and the folding would be correct on one.
fn drive_constant_data(
    module: &mut Module,
    device: &Device,
    report: &mut CellMapReport,
    diags: &mut Diagnostics,
) {
    let Some(lut) = device.bel(BelRole::Lut).cloned() else {
        // No LUT to build a constant out of. The backend refuses such a
        // flip-flop rather than writing one that comes up wrong; see
        // `TrellisFabric::configure_registers`.
        return;
    };
    let mut data_ports: Vec<(String, String)> = device
        .bels
        .iter()
        .filter(|bel| bel.role == BelRole::Ff)
        .map(|bel| (bel.name.clone(), bel.port("d").unwrap_or("D").to_owned()))
        .collect();
    for bel in device
        .bels
        .iter()
        .filter(|b| matches!(b.role, BelRole::LutRam | BelRole::Carry))
    {
        for (role, names) in &bel.ports {
            // The outputs, which nothing drives from outside, and the
            // chain's own two ends, which are dedicated metal: a `ci` is
            // reached from the cell below or from nowhere, and a flow that
            // put a lookup table on one would be describing a connection
            // the fabric does not have.
            if matches!(role.as_str(), "dout" | "o" | "s" | "co" | "ci" | "cyinit") {
                continue;
            }
            for name in names.split(',') {
                data_ports.push((bel.name.clone(), name.to_owned()));
            }
        }
    }
    // A family declares its one flip-flop once per parameter set — an ECP5
    // has thirty-six `TRELLIS_FF` lines — so the same `(primitive, port)`
    // pair arrives many times, and a cell would be given a driver once per
    // line.
    data_ports.sort();
    data_ports.dedup();
    if data_ports.is_empty() {
        return;
    }
    // Which flip-flop asks for which constant, decided on the bit-level
    // view so that a constant reaching the pin through a continuous
    // assignment, a slice or a concatenation counts as one.
    let mut wanted: Vec<(CellId, String, Bit)> = Vec::new();
    match BitView::new(module) {
        Ok(view) => {
            for (id, cell) in module.cells.iter() {
                let CellKind::Blackbox(primitive) = &cell.kind else {
                    continue;
                };
                for (_, port) in data_ports
                    .iter()
                    .filter(|(name, _)| name == primitive.as_str())
                {
                    let Some(d) = cell.input(port) else { continue };
                    let Ok(bits) = view.expr_bits(d) else {
                        continue;
                    };
                    let [bit] = bits[..] else { continue };
                    if let SigBit::Const(value @ (Bit::Zero | Bit::One)) = view.canonical(bit) {
                        wanted.push((id, port.clone(), value));
                    }
                }
            }
        }
        // A module whose assignments do not resolve is not this pass's
        // problem to report: the netlist builder says so with a span.
        Err(_) => return,
    }

    let mut zero: Option<NetId> = None;
    let mut one: Option<NetId> = None;
    for (id, port, value) in wanted {
        let span = module.cells[id].span;
        let held = if value == Bit::One {
            &mut one
        } else {
            &mut zero
        };
        let net = match *held {
            Some(net) => net,
            None => {
                let net = constant_lut(module, device, &lut, value, span, report, diags);
                *held = Some(net);
                net
            }
        };
        let driven = net_expr(module, net, span);
        let cell = &mut module.cells[id];
        let name = cell.name.as_str().to_owned();
        if let Some((_, slot)) = cell.inputs.iter_mut().find(|(p, _)| p.as_str() == port) {
            *slot = driven;
        }
        report.constants.push((name, value));
    }
}

/// One lookup table of the device's own kind whose output is `value`
/// whatever its inputs do, and the net it drives.
///
/// A one-input LUT with both halves of its truth table equal to `value`:
/// after [`map_lut`] widens it to the family's size the whole table is
/// `value`, so the function ignores every input. That is what makes it
/// safe on a fabric that ties an unrouted LUT input high — the value does
/// not depend on what the inputs read — and it is the shape Lattice's own
/// packer writes, an `INIT` of all zeros or all ones with every input
/// tied.
fn constant_lut(
    module: &mut Module,
    device: &Device,
    bel: &BelKind,
    value: Bit,
    span: crate::source::Span,
    report: &mut CellMapReport,
    diags: &mut Diagnostics,
) -> NetId {
    let one = value == Bit::One;
    let base = if one { "const1" } else { "const0" };
    let out = add_net(module, base, Type::bit(), span);
    let init = if one { Const::ones(2) } else { Const::zero(2) };
    let a = const_expr(module, Const::zero(1), span);
    let cell = add_cell(
        module,
        &format!("{base}$lut"),
        CellKind::Lut { k: 1, init },
        vec![(Name::new("a"), a)],
        vec![(Name::new("y"), out)],
        span,
    );
    map_lut(module, device, bel, cell, report, diags);
    out
}

/// The flip-flop primitives a device declares, as a sentence.
fn declared_variants(device: &Device) -> String {
    let mut names: Vec<&str> = Vec::new();
    for (bel, _) in device.ff_variants() {
        if !names.contains(&bel.name.as_str()) {
            names.push(bel.name.as_str());
        }
    }
    if names.is_empty() {
        return "no flip-flop at all".to_owned();
    }
    format!(
        "{} ({} variant(s))",
        names.join(", "),
        device.ff_variants().count()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fpga::target;
    use crate::ir::Reset;
    use crate::ir::builder::ModuleBuilder;
    use crate::ir::validate::validate;
    use crate::source::{SourceMap, Span};

    fn span() -> (SourceMap, Span) {
        let mut map = SourceMap::new();
        let file = map.add("top.v", "").unwrap();
        let span = Span::new(file, 0, 0);
        (map, span)
    }

    /// A module with one LUT and one flip-flop of the given shape.
    fn design_with(kind: CellKind, width: u32) -> (Design, ModuleId, SourceMap) {
        let (map, span) = span();
        let mut b = ModuleBuilder::new("top", span);
        let clk = b.input("clk", Type::bit());
        let rst = b.input("rst", Type::bit());
        let en = b.input("en", Type::bit());
        let a = b.input("a", Type::bits(2));
        let q = b.output("q", Type::bits(width));
        let lut_out = b.add_net("lut_out", Type::bit());
        let a_e = b.net(a);
        b.cell(
            "l",
            CellKind::Lut {
                k: 2,
                // a & !b: pattern 01 only.
                init: Const::from_u64(0b0010, 4),
            },
            vec![(Name::new("a"), a_e)],
            vec![(Name::new("y"), lut_out)],
        );
        let lut_e = b.net(lut_out);
        let d = b.replicate(width, lut_e);
        let mut inputs = vec![(Name::new("clk"), b.net(clk)), (Name::new("d"), d)];
        if let CellKind::Dff {
            has_enable, reset, ..
        } = &kind
        {
            if *has_enable {
                inputs.push((Name::new("en"), b.net(en)));
            }
            if reset.is_some() {
                inputs.push((Name::new("rst"), b.net(rst)));
            }
        }
        b.cell("r", kind, inputs, vec![(Name::new("q"), q)]);
        let mut design = Design::new();
        let top = design.add_module(b.finish());
        design.top = Some(top);
        (design, top, map)
    }

    /// A module whose register's top bits are a constant: one LUT feeds bit
    /// 0 and `value` is concatenated above it, so bits 1.. are a flip-flop
    /// whose data input is a constant and nothing else.
    fn design_with_constant_bits(width: u32, value: Bit) -> (Design, ModuleId, SourceMap) {
        let (map, span) = span();
        let mut b = ModuleBuilder::new("top", span);
        let clk = b.input("clk", Type::bit());
        let a = b.input("a", Type::bits(2));
        let q = b.output("q", Type::bits(width));
        let lut_out = b.add_net("lut_out", Type::bit());
        let a_e = b.net(a);
        b.cell(
            "l",
            CellKind::Lut {
                k: 2,
                init: Const::from_u64(0b0010, 4),
            },
            vec![(Name::new("a"), a_e)],
            vec![(Name::new("y"), lut_out)],
        );
        let lut_e = b.net(lut_out);
        // A concatenation is most significant part first, so the constants
        // are the top bits and the LUT is bit 0.
        let top_bits = if value == Bit::One {
            b.constant(Const::ones(width - 1))
        } else {
            b.constant(Const::zero(width - 1))
        };
        let d = b.concat(vec![top_bits, lut_e]);
        let clk_e = b.net(clk);
        b.cell(
            "r",
            CellKind::Dff {
                clk_pos: true,
                has_enable: false,
                reset: None,
            },
            vec![(Name::new("clk"), clk_e), (Name::new("d"), d)],
            vec![(Name::new("q"), q)],
        );
        let mut design = Design::new();
        let top = design.add_module(b.finish());
        design.top = Some(top);
        (design, top, map)
    }

    /// The net a cell's port is connected to, or `None` when it is not a
    /// plain net.
    fn port_net(design: &Design, top: ModuleId, cell: &str, port: &str) -> Option<String> {
        let module = design.module(top);
        let (_, cell) = module.cells.iter().find(|(_, c)| c.name.as_str() == cell)?;
        let e = cell.input(port)?;
        let net = module.expr(e).as_net()?;
        Some(module.nets[net].name.as_str().to_owned())
    }

    fn cells_named(design: &Design, top: ModuleId, primitive: &str) -> usize {
        design
            .module(top)
            .cells
            .iter()
            .filter(|(_, c)| matches!(&c.kind, CellKind::Blackbox(n) if n.as_str() == primitive))
            .count()
    }

    /// A flip-flop whose data input is the constant zero gets a lookup table
    /// that makes the zero, because an unrouted slice input on an ECP5 reads
    /// as a **one** and the flop would otherwise come up set. This is the
    /// defect that kept a USB device from enumerating for eight rounds; see
    /// `docs/fpga-trellis.md`.
    #[test]
    fn a_constant_on_a_flip_flops_data_input_gets_a_driver() {
        let (mut design, top, _map) = design_with_constant_bits(2, Bit::Zero);
        let mut diags = Diagnostics::new();
        let device = target("ecp5-45f-CABGA381").unwrap();
        let report = map_cells(&mut design, top, device, &mut diags);
        assert_eq!(diags.len(), 0, "{:?}", diags.iter().next());
        assert!(!validate(&design).has_errors());

        // Two flip-flops, and the mapped LUT plus one for the constant.
        assert_eq!(cells_named(&design, top, "TRELLIS_FF"), 2);
        assert_eq!(report.count("LUT4"), 2);
        assert_eq!(
            report.constants,
            vec![("r$ff1".to_owned(), Bit::Zero)],
            "one flip-flop asked for a constant, and it is the top bit"
        );
        assert_eq!(report.constant_data_pins(), 1);
        assert!(
            report
                .to_text()
                .contains("r$ff1's data input is the constant 0, now driven"),
            "{}",
            report.to_text()
        );

        // The flop's data comes off the driver's net, and that driver is a
        // LUT4 whose truth table is all zeros however its inputs read.
        assert_eq!(
            port_net(&design, top, "r$ff1", "DI").as_deref(),
            Some("const0")
        );
        let driver = design
            .module(top)
            .cells
            .iter()
            .find(|(_, c)| c.name.as_str() == "const0$lut")
            .map(|(_, c)| c.clone())
            .expect("the constant driver is in the netlist");
        let Some(AttrValue::Const(init)) = driver.params.get("INIT") else {
            panic!("no INIT on the constant driver");
        };
        assert_eq!(init.width(), 16);
        assert_eq!(init.to_u64(), Some(0x0000));
        assert_eq!(
            driver
                .inputs
                .iter()
                .map(|(n, _)| n.as_str())
                .collect::<Vec<_>>(),
            ["A", "B", "C", "D"],
            "every input is connected, so nothing is left for the fabric to decide"
        );
    }

    /// The constant one is the same shape with the other truth table — and
    /// on an ECP5 it costs no `INIT` bits at all, since all ones is what the
    /// field defaults to. It is built anyway, because *why* an undriven wire
    /// reads as a one is a property of the silicon and not something a
    /// netlist should depend on.
    #[test]
    fn a_constant_one_gets_a_driver_of_its_own() {
        let (mut design, top, _map) = design_with_constant_bits(2, Bit::One);
        let mut diags = Diagnostics::new();
        let device = target("ecp5-45f-CABGA381").unwrap();
        let report = map_cells(&mut design, top, device, &mut diags);
        assert_eq!(diags.len(), 0, "{:?}", diags.iter().next());
        assert!(!validate(&design).has_errors());
        assert_eq!(report.constants, vec![("r$ff1".to_owned(), Bit::One)]);
        assert_eq!(
            port_net(&design, top, "r$ff1", "DI").as_deref(),
            Some("const1")
        );
        let driver = design
            .module(top)
            .cells
            .iter()
            .find(|(_, c)| c.name.as_str() == "const1$lut")
            .map(|(_, c)| c.clone())
            .expect("the constant driver is in the netlist");
        let Some(AttrValue::Const(init)) = driver.params.get("INIT") else {
            panic!("no INIT on the constant driver");
        };
        assert_eq!(init.to_u64(), Some(0xFFFF));
    }

    /// One driver per constant, not one per flip-flop, which is what
    /// nextpnr's `pack_constants` does and what Lattice's own bitstreams for
    /// a Cynthion contain: one `SLICEA.K0` feeding eight, twelve, eighteen
    /// or thirty-two flip-flops across the whole die.
    #[test]
    fn every_flip_flop_wanting_the_same_constant_shares_one_driver() {
        let (mut design, top, _map) = design_with_constant_bits(5, Bit::Zero);
        let mut diags = Diagnostics::new();
        let device = target("ecp5-45f-CABGA381").unwrap();
        let report = map_cells(&mut design, top, device, &mut diags);
        assert_eq!(diags.len(), 0, "{:?}", diags.iter().next());
        assert!(!validate(&design).has_errors());
        assert_eq!(report.constant_data_pins(), 4, "bits 1 to 4 are the zero");
        assert_eq!(report.count("LUT4"), 2, "the mapped one and one constant");
        assert_eq!(
            cells_named(&design, top, "LUT4"),
            2,
            "and no second copy of the constant in the module either"
        );
        for bit in 1..5 {
            assert_eq!(
                port_net(&design, top, &format!("r$ff{bit}"), "DI").as_deref(),
                Some("const0"),
                "bit {bit}"
            );
        }
    }

    /// A device whose file declares no LUT cannot have a constant built for
    /// it, and the pass says nothing rather than half-doing it. The backend
    /// is what refuses such a flip-flop, by name and position.
    #[test]
    fn a_device_with_no_lut_gets_no_constant_driver() {
        let (mut design, top, _map) = design_with_constant_bits(2, Bit::Zero);
        let mut diags = Diagnostics::new();
        let mut device = target("ecp5-45f-CABGA381").unwrap().clone();
        device.bels.retain(|bel| bel.role != BelRole::Lut);
        let report = map_cells(&mut design, top, &device, &mut diags);
        assert!(report.constants.is_empty());
        assert_eq!(
            port_net(&design, top, "r$ff1", "DI"),
            None,
            "the data input is still the constant it was"
        );
    }

    #[test]
    fn a_lut_becomes_the_family_primitive() {
        let (mut design, top, _map) = design_with(
            CellKind::Dff {
                clk_pos: true,
                has_enable: false,
                reset: None,
            },
            1,
        );
        let mut diags = Diagnostics::new();
        let device = target("ice40-hx1k-tq144").unwrap();
        let report = map_cells(&mut design, top, device, &mut diags);
        assert_eq!(diags.len(), 0);
        assert_eq!(cells_named(&design, top, "SB_LUT4"), 1);
        assert_eq!(report.count("SB_LUT4"), 1);
        assert!(!validate(&design).has_errors());

        let cell = design
            .module(top)
            .cells
            .iter()
            .find(|(_, c)| matches!(&c.kind, CellKind::Blackbox(n) if n.as_str() == "SB_LUT4"))
            .map(|(_, c)| c.clone())
            .unwrap();
        // Four pins, the two unused ones tied low, in pin order.
        let pins: Vec<&str> = cell.inputs.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(pins, ["I0", "I1", "I2", "I3"]);
        // `a & !b` over two inputs is 4'b0010; repeated four times it is
        // 16'b0010_0010_0010_0010 = 0x2222, which is what a four-input
        // SB_LUT4 ignoring I2 and I3 computes.
        let init = cell.params.get("LUT_INIT").unwrap();
        let AttrValue::Const(init) = init else {
            panic!("LUT_INIT is not a constant: {init:?}")
        };
        assert_eq!(init.width(), 16);
        assert_eq!(init.to_u64(), Some(0x2222));
    }

    #[test]
    fn a_wide_flip_flop_becomes_one_primitive_per_bit() {
        let (mut design, top, _map) = design_with(
            CellKind::Dff {
                clk_pos: true,
                has_enable: true,
                reset: Some(Reset {
                    asynchronous: false,
                    active_high: true,
                    // Bit 1 is set, the others are reset.
                    value: Const::from_u64(0b0010, 4),
                }),
            },
            4,
        );
        let mut diags = Diagnostics::new();
        let device = target("ice40-hx1k-tq144").unwrap();
        let report = map_cells(&mut design, top, device, &mut diags);
        assert_eq!(diags.len(), 0, "{:?}", diags.iter().next());
        assert_eq!(cells_named(&design, top, "SB_DFFESR"), 3);
        assert_eq!(cells_named(&design, top, "SB_DFFESS"), 1);
        assert_eq!(report.count("SB_DFFESR"), 3);
        assert!(!validate(&design).has_errors());
        assert!(report.to_text().contains("3 x SB_DFFESR"));
    }

    #[test]
    fn an_ecp5_flip_flop_carries_its_muxes() {
        let (mut design, top, _map) = design_with(
            CellKind::Dff {
                clk_pos: false,
                has_enable: false,
                reset: Some(Reset {
                    asynchronous: true,
                    active_high: false,
                    value: Const::zero(1),
                }),
            },
            1,
        );
        let mut diags = Diagnostics::new();
        let device = target("ecp5-45f-CABGA381").unwrap();
        map_cells(&mut design, top, device, &mut diags);
        assert_eq!(diags.len(), 0);
        let cell = design
            .module(top)
            .cells
            .iter()
            .find(|(_, c)| matches!(&c.kind, CellKind::Blackbox(n) if n.as_str() == "TRELLIS_FF"))
            .map(|(_, c)| c.clone())
            .unwrap();
        assert_eq!(cell.params.get("CLKMUX").unwrap().as_str(), Some("INV"));
        assert_eq!(cell.params.get("LSRMUX").unwrap().as_str(), Some("INV"));
        assert_eq!(cell.params.get("SRMODE").unwrap().as_str(), Some("ASYNC"));
        assert_eq!(cell.params.get("REGSET").unwrap().as_str(), Some("RESET"));
        // The unused clock enable is tied high, not left dangling.
        let ce = cell
            .inputs
            .iter()
            .find(|(n, _)| n.as_str() == "CE")
            .unwrap();
        let value = design.module(top).exprs[ce.1].as_const().unwrap();
        assert_eq!(value.to_u64(), Some(1));
        assert_eq!(cells_named(&design, top, "LUT4"), 1);
    }

    #[test]
    fn an_active_low_reset_is_inverted_onto_the_polarity_ice40_has() {
        // Every iCE40 set/reset is active high, and `negedge rst_n` is
        // how nearly all HDL is written, so the mapper inverts the net
        // and uses `SB_DFFR` rather than refusing.
        let (mut design, top, _map) = design_with(
            CellKind::Dff {
                clk_pos: true,
                has_enable: false,
                reset: Some(Reset {
                    asynchronous: true,
                    active_high: false,
                    value: Const::zero(1),
                }),
            },
            1,
        );
        let mut diags = Diagnostics::new();
        let device = target("ice40-hx1k-tq144").unwrap();
        let report = map_cells(&mut design, top, device, &mut diags);
        assert_eq!(diags.len(), 0, "{:?}", diags.iter().next());
        assert_eq!(cells_named(&design, top, "SB_DFFR"), 1);
        assert!(report.declined.is_empty());
        // One inverter, named after the net it complements, and it is an
        // ordinary LUT of the family: the design's own LUT plus this one.
        assert_eq!(report.inverters(), 1);
        assert_eq!(
            report.inverted,
            [("rst".to_owned(), "set/reset".to_owned())]
        );
        assert_eq!(cells_named(&design, top, "SB_LUT4"), 2);
        assert!(report.to_text().contains("rst inverted for the set/reset"));
        assert!(!validate(&design).has_errors());

        // The inverter really computes a complement: bit `i` of the
        // truth table is the output for input pattern `i`, and with I1,
        // I2 and I3 tied low that is 16'h5555.
        let inv = design
            .module(top)
            .cells
            .iter()
            .find(|(_, c)| c.name.as_str() == "rst$n$inv")
            .map(|(_, c)| c.clone())
            .expect("the inverter");
        let AttrValue::Const(init) = inv.params.get("LUT_INIT").unwrap() else {
            panic!("LUT_INIT is not a constant")
        };
        assert_eq!(init.to_u64(), Some(0x5555));
    }

    #[test]
    fn one_inverter_serves_every_flip_flop_on_the_net() {
        // A four-bit register is four `SB_DFFR`s, and the reset they
        // share costs one LUT between them, not four.
        let (mut design, top, _map) = design_with(
            CellKind::Dff {
                clk_pos: true,
                has_enable: false,
                reset: Some(Reset {
                    asynchronous: true,
                    active_high: false,
                    value: Const::zero(4),
                }),
            },
            4,
        );
        let mut diags = Diagnostics::new();
        let device = target("ice40-hx1k-tq144").unwrap();
        let report = map_cells(&mut design, top, device, &mut diags);
        assert_eq!(diags.len(), 0, "{:?}", diags.iter().next());
        assert_eq!(cells_named(&design, top, "SB_DFFR"), 4);
        assert_eq!(report.inverters(), 1);
        assert_eq!(cells_named(&design, top, "SB_LUT4"), 2);
        assert!(!validate(&design).has_errors());
    }

    #[test]
    fn a_clock_edge_the_device_lacks_is_still_reported() {
        // Inverting a clock would make a second clock network, so it is
        // not something the mapper does: a family with no falling-edge
        // flip-flop is reported, not approximated.
        let (mut design, top, map) = design_with(
            CellKind::Dff {
                clk_pos: false,
                has_enable: false,
                reset: None,
            },
            1,
        );
        let mut device = target("ice40-hx1k-tq144").unwrap().clone();
        device.bels.retain(|b| b.ff.is_none_or(|v| v.clk_pos));
        let mut diags = Diagnostics::new();
        let report = map_cells(&mut design, top, &device, &mut diags);
        assert!(diags.has_errors());
        let text = diags.render(&map);
        assert!(
            text.contains("has no flip-flop with a falling clock edge"),
            "{text}"
        );
        assert_eq!(report.declined.len(), 1);
        assert_eq!(report.inverters(), 0);
        // The cell is untouched, so nothing silently changed meaning.
        assert!(
            design
                .module(top)
                .cells
                .iter()
                .any(|(_, c)| matches!(c.kind, CellKind::Dff { .. }))
        );
    }

    #[test]
    fn a_device_with_no_lut_cannot_fix_a_polarity() {
        // The inverter is one of the device's own LUTs, so a family that
        // declares none is back to an exact match or a refusal.
        let (mut design, top, map) = design_with(
            CellKind::Dff {
                clk_pos: true,
                has_enable: false,
                reset: Some(Reset {
                    asynchronous: true,
                    active_high: false,
                    value: Const::zero(1),
                }),
            },
            1,
        );
        let mut device = target("ice40-hx1k-tq144").unwrap().clone();
        device.bels.retain(|b| b.role != BelRole::Lut);
        let mut diags = Diagnostics::new();
        let report = map_cells(&mut design, top, &device, &mut diags);
        assert!(diags.has_errors());
        let text = diags.render(&map);
        assert!(
            text.contains(
                "has no flip-flop with a rising clock edge, an active-low asynchronous reset"
            ),
            "{text}"
        );
        assert!(text.contains("SB_DFFR"), "{text}");
        assert_eq!(report.inverters(), 0);
        assert_eq!(report.declined.len(), 2, "the LUT is declined too");
    }

    #[test]
    fn an_enable_polarity_the_device_lacks_is_inverted_too() {
        // No shipped family has an active-low clock enable, so this is a
        // device whose `SB_DFFE` has been re-declared with one: the
        // enable net is inverted exactly as a reset would be.
        let (mut design, top, _map) = design_with(
            CellKind::Dff {
                clk_pos: true,
                has_enable: true,
                reset: None,
            },
            1,
        );
        let mut device = target("ice40-hx1k-tq144").unwrap().clone();
        for bel in &mut device.bels {
            if let Some(variant) = &mut bel.ff
                && variant.has_enable
            {
                variant.enable_active_high = false;
            }
        }
        let mut diags = Diagnostics::new();
        let report = map_cells(&mut design, top, &device, &mut diags);
        assert_eq!(diags.len(), 0, "{:?}", diags.iter().next());
        assert_eq!(cells_named(&design, top, "SB_DFFE"), 1);
        assert_eq!(
            report.inverted,
            [("en".to_owned(), "clock enable".to_owned())]
        );
        assert!(!validate(&design).has_errors());
    }

    #[test]
    fn a_six_input_family_widens_the_truth_table() {
        let (mut design, top, _map) = design_with(
            CellKind::Dff {
                clk_pos: true,
                has_enable: false,
                reset: None,
            },
            1,
        );
        let mut diags = Diagnostics::new();
        let device = target("generic-k6").unwrap();
        map_cells(&mut design, top, device, &mut diags);
        assert_eq!(diags.len(), 0);
        let cell = design
            .module(top)
            .cells
            .iter()
            .find(|(_, c)| matches!(&c.kind, CellKind::Blackbox(n) if n.as_str() == "LUT"))
            .map(|(_, c)| c.clone())
            .unwrap();
        // One bus port, not six pins, because the family declares `i=I`.
        assert_eq!(cell.inputs.len(), 1);
        assert_eq!(cell.inputs[0].0.as_str(), "I");
        let AttrValue::Const(init) = cell.params.get("INIT").unwrap() else {
            panic!("INIT is not a constant")
        };
        assert_eq!(init.width(), 64);
        assert_eq!(init.to_u64(), Some(0x2222_2222_2222_2222));
        assert!(!validate(&design).has_errors());
    }
}
