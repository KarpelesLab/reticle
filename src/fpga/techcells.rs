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

use super::device::{BelKind, BelRole, BramShape, Device, FfReset, FfVariant};
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
    /// Primitive pins that were a constant and now have a lookup table
    /// driving them, as `(cell, port, the constant)`, in cell order.
    ///
    /// One entry per **bit** of one pin of one cell — a block RAM's
    /// fourteen-bit address contributes one entry per constant bit — and
    /// **one LUT per constant**, shared by every pin of the module that
    /// wants that value, which is what nextpnr's `pack_constants` does and
    /// what Lattice's own bitstreams for this board contain. The LUTs
    /// themselves are counted in `primitives` like any other.
    pub constants: Vec<(String, String, Bit)>,
}

impl CellMapReport {
    /// True when nothing was rewritten and nothing was declined.
    pub fn is_empty(&self) -> bool {
        self.primitives.is_empty()
            && self.declined.is_empty()
            && self.inverted.is_empty()
            && self.constants.is_empty()
    }

    /// How many primitive pin bits were given a constant driver, which is
    /// not the number of drivers: there is one LUT per constant, not one
    /// per pin.
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
        for (cell, port, value) in &self.constants {
            let _ = writeln!(out, "  {cell}'s {port} is the constant {value}, now driven");
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

// --- a constant on a primitive's input pin ----------------------------------

/// Whether a family *designs* this pin to take a constant, so leaving one
/// on it is correct rather than the defect [`drive_constant_data`] exists
/// to prevent.
///
/// This is the whole exception list, and every entry is a mechanism that
/// has been read somewhere rather than a guess. Anything not named here
/// gets a driver, which is the safe default: a pin nothing drives does not
/// read the constant it was given — on an ECP5 it reads a **one** and on
/// the 7 series the database says every `IMUX` reads `VCC_WIRE` — so
/// "needs a driver" is the answer to assume when nothing says otherwise.
///
/// | Pin | What absorbs the constant |
/// |---|---|
/// | any input of a `lut` | the **truth table**. [`map_lut`] widens a narrow table so the function ignores the tied inputs, and Lattice's own packer does the same thing from the other end — `SLICE<l>.<X><n>MUX = 1` on every unused input, with the value folded into `INIT`, in all 4262 carry halves of this board's own bitstreams |
/// | a `ff`'s `clk`, `en`, `rst` | a **mux bit the backend writes**. On an ECP5 `SLICE<l>.CEMUX = 1` and `LSR<c>.LSRMUX`; on the 7 series a cleared `CEUSEDMUX` ties `CE` to one and a cleared `SRUSEDMUX` ties `SR` to zero, which is exactly what a flip-flop with no enable and no reset wants. Only `d` has no such bit, and `d` is what this pass was written for |
/// | a `carry`'s `ci` | **dedicated metal** from the cell below. Nothing outside the chain may drive it, and a lookup table on one would describe a connection the fabric has not got |
/// | a `carry`'s `cyinit` | the pin a family designs as its way in: [`WideCarry`](super::device::WideCarry) is "the chain goes on `ci` and the constant on `init`" |
/// | a wide `carry`'s `p` and `di` | **the lane's own truth table.** These two roles exist only on the [`WideCarry`](super::device::WideCarry) shape — a 7-series `CARRY4`'s `S` and `DI` — and `src/fpga/xray/carry.rs` already puts a constant propagate into a lookup table of that lane and a constant generate into the *lower half* of it, because `DI` is reached from the slice's own `O5` and not from general routing. A driver elsewhere would cost a lookup table and a route to get to a pin the fabric wires short. The narrow shapes' operands (`a`, `b`, `i0`, `i1`) are **not** here: those are the pins that cost a board |
/// | an `io`'s `oe` / `oen` | the **direction recipe**. Which of `IBUF`/`OBUF`/`IOBUF` a port gets, and which `BASE_TYPE` value an ECP5 pad gets, is decided from this pin; the ECP5 then ties the wire itself (`CIB.JB0MUX`), and that tie is on every Cynthion bitstream this flow has put in a part |
/// | an `io`'s `pad` | it reaches the outside world, not the fabric, and `Netlist::off_fabric` already says so |
/// | any `clk` | a clock is not a data pin. A lookup table on one would be a clock arriving on general routing, which both fabric backends refuse by name — so a constant here is reported, not papered over |
/// | every pin of a `gb` | its `i` is a clock, and its `en` is a constant **this flow puts there on purpose**: [`super::primitives`] ties an ECP5 `DCCA`'s `CE` high because nothing gates a clock here, and leaving it unconnected would let the tool decide what an unenabled clock buffer does. A lookup table on it would ask the router for a path into a clock buffer's enable for no defect at all |
///
/// A `dsp` and a `pll` are not `bel` lines — they are shapes of their own
/// — so they never reach this list, and the pins they hold at a constant
/// are the ones their own `tie` clause names: a declaration that the
/// *silicon* drives them, which `src/fpga/xray/cmt.rs` then reads off the
/// pin to pick the site's inverter bits.
///
/// The table above is what every family has in common. A family with a
/// mechanism another family has not got names the pin in its own `.dev`
/// file instead, with an `absorbs` clause, and
/// [`BelKind::absorbs_constant`] is consulted first — which is how a
/// 7-series `OBUF`'s data pin gets a driver while an ECP5 `TRELLIS_IO`'s
/// does not.
///
/// **That one is a real disagreement with the vendor and is deliberate.**
/// `ecppack` writes `CIB.J<x>MUX` for **not one** of the 318 output pads
/// of `analyzer.bit`, `selftest.bit` and `facedancer.bit`, on the data
/// wire or the tristate wire: it routes its constant lookup table into
/// every one of them instead, which is what
/// `tests/fpga_trellis.rs`'s
/// `what_lattices_own_packer_writes_for_a_constant_on_a_pad` measures. The
/// tie this flow writes instead is in every bitstream it has put in a
/// Cynthion, including the one whose `R4` holds a constant zero and which
/// a USB host enumerates — so it is measured on **silicon**, where the
/// vendor's answer is measured in somebody else's file. Replacing
/// something that works on a part with something that works on a part is
/// churn, and the part is the better instrument. The 7 series has no such
/// field, so there the constant is built.
fn absorbs_constant(bel: &BelKind, port: &str) -> bool {
    if bel.absorbs_constant(port) {
        return true;
    }
    match bel.role {
        BelRole::Lut | BelRole::GlobalBuffer => true,
        BelRole::Ff => port != "d",
        BelRole::Carry => matches!(port, "ci" | "cyinit" | "p" | "di") || port.ends_with("clk"),
        _ => matches!(port, "oe" | "oen" | "pad") || port.ends_with("clk"),
    }
}

/// Whether a block RAM pin of abstract role `role` needs a driver on a
/// block of this shape.
///
/// A `bram` block is not a `bel` line — it is declared by its own block
/// with a `port` map of abstract roles (`clk`, `en`, `addr`, `din`,
/// `dout`, `we`, `rst`) — so it is asked separately, and the answer is
/// narrower than for a `bel`: **its address, on a family whose own `.dev`
/// file states what the bits below the word address must read.**
///
/// That clause is `pad` on a `mode` line, and the ECP5 is the family that
/// has one:
///
/// ```text
/// mode 18 1024 param DATA_WIDTH_A=18 DATA_WIDTH_B=18 addr 4 pad 4'b0011 …
/// ```
///
/// A `DP16KD` in 18-bit mode addresses its words through `AD[13:4]` and
/// **reads** `AD[3:0]`, which must be `0011`. Two of those four bits are a
/// constant zero, nothing routed them, and an unrouted input on that
/// family is a **one** — so every 18-bit block RAM this flow built
/// addressed its contents through `1111`. Driving them is the fix.
///
/// A family that states no `pad` is read as not reading those bits, and
/// **that is a reading rather than a measurement.** It is the right one
/// for the two families whose libraries say so — a `RAMB18E1` in 18-bit
/// mode uses `ADDRARDADDR[13:4]` and an `SB_RAM40_4K` in 8-bit mode
/// `ADDR[8:0]`, with the rest unused — but it is their documentation
/// speaking and not this project's instrument. What makes it safe to rely
/// on: nextpnr ties those bits to zero on both families, so a netlist that
/// goes out to nextpnr gets them driven there, and on the 7 series
/// `XrayFabric::configure_block_rams` now gives an unused address bit the
/// idle policy, so a contended ground fan can no longer fail a build over
/// one.
///
/// The other roles stay with the backends' own tie passes, which is what
/// those were written for, and the gap that leaves is named in
/// `tests/fpga_constants.rs`.
fn bram_pin_needs_a_driver(role: &str, shape: &BramShape) -> bool {
    role == "addr"
        && shape
            .mode_layouts
            .iter()
            .any(|layout| layout.addr_pad.is_some())
}

/// Every `(primitive, port)` of `device` that a constant may not be left
/// on, from the device's own declarations and from nothing else.
fn pins_needing_a_driver(device: &Device) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for bel in &device.bels {
        for (role, names) in &bel.ports {
            if absorbs_constant(bel, role) {
                continue;
            }
            for name in names.split(',').filter(|n| !n.is_empty()) {
                out.push((bel.name.clone(), name.to_owned()));
            }
        }
    }
    for bram in &device.block_rams {
        for port in &bram.port_map {
            for (role, name) in &port.signals {
                if bram_pin_needs_a_driver(role, bram) {
                    out.push((bram.name.clone(), name.clone()));
                }
            }
        }
    }
    // A family declares its one flip-flop once per parameter set — an ECP5
    // has thirty-six `TRELLIS_FF` lines, and three `bram` blocks per device
    // say the same two ports — so the same `(primitive, port)` pair arrives
    // many times, and a cell would be given a driver once per line.
    out.sort();
    out.dedup();
    out
}

/// Gives every pin that is a constant and cannot absorb one something that
/// drives it: one lookup table per constant, shared by every cell of the
/// module that asks for that value.
///
/// Which pins those are is [`absorbs_constant`]'s answer, read off the
/// device's own port maps: **every input of every primitive except the
/// ones a family designs to take a constant**. The default is the one that
/// is safe, because the fault this prevents is silent — a netlist has no
/// unrouted wires, so a constant pin evaluates to the constant in every
/// simulation, every exhaustive model check and every SAT proof in this
/// tree, and the bitstream decodes perfectly either way.
///
/// See the module docs for why a fabric flip-flop cannot absorb a constant
/// the way a lookup table can, and
/// `tests/fpga_trellis.rs`'s `what_lattices_own_packer_writes_for_a_constant`
/// for the vendor bitstreams this shape was read out of.
///
/// # The five pins that were added one defect at a time
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
/// A **carry** element's inputs were the third case and the expensive one.
/// `cnt + 1` is an adder whose second operand is a constant, so thirty-one
/// of its thirty-two `b` pins are a constant **zero** — and a constant on
/// a carry cell's operand pin is not a wire anything routes, so on an ECP5
/// every one of them read as a **one** and the chain computed `cnt - 1`.
/// Nothing off the part could see it: the design synthesised, placed,
/// routed, and every set bit of its bitstream decoded back through the
/// database into the arcs the router chose, while the exhaustive check in
/// `src/fpga/primitives.rs` and the SAT proof in `tests/fpga_carry.rs`
/// both passed. What found it was a Cynthion that stopped enumerating.
///
/// A **block RAM's address** and an **output buffer's data** are the
/// fourth and fifth, and they are the reason this is now a rule with a
/// short list of exceptions rather than a list of three roles:
///
/// - an ECP5 `DP16KD` in 18-bit mode wants `AD[3:0] = 0011`, which
///   `src/fpga/devices/ecp5.dev` states as `pad 4'b0011`. Two of those
///   four bits are a constant **zero**, nothing routed them, and an
///   unrouted input on that family reads as a one — so every 18-bit
///   block RAM this flow built addressed its contents through `AD[3:0]`
///   reading `1111`. That is the `lutram` fault again, in the hard block,
///   and no test of this flow's own output could have seen it. It is
///   driven now, and only on the family whose file states the value:
///   [`bram_pin_needs_a_driver`] says why;
/// - an output buffer's data pin had nothing anywhere on the 7 series.
///   `assign led = 1'b1` left the pad's input unrouted and the pad came
///   out **low** on a Basys 3 — the opposite symptom to the ECP5's, from
///   the same cause, with no diagnostic on either. It is built now.
///
/// The two families' answers differ and that is the point of the
/// `absorbs` clause: an ECP5 pad's data wire *can* be tied, in the `CIB`
/// tile beside it, and that tie is in every bitstream this flow has put in
/// a Cynthion — so `ecp5.dev` says `absorbs dout` and this pass leaves it
/// alone there. A 7-series `OBUF` has no such field.
///
/// The 7-series block RAM address is not fixed here at all, and that is
/// worth being exact about. Those bits are ones the block does **not**
/// read, and what went wrong was the post-route tie pass failing a build
/// over them: it hunts a free `GND_WIRE -> GFAN<n>` path, an interconnect
/// tile has exactly **two** such fans for its 48 inputs and the router
/// competes for both, so whether a build succeeded depended on where the
/// block landed. `examples/basys3/ssd1306_console.v` failed on `097a1d4`
/// (at one placement, though it was recorded as nine: see
/// `docs/fpga-xray.md`), and `selftest.v` is the design
/// `docs/fpga-xray.md` records building, then not, then building again. The
/// remedy there is `src/fpga/xray/bram.rs`'s `unused_address_bit`, which
/// gives such a bit the idle policy instead — no lookup table and no route
/// for a bit nothing reads.
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
    let data_ports = pins_needing_a_driver(device);
    if data_ports.is_empty() {
        return;
    }
    // Which pin asks for which constant, decided on the bit-level view so
    // that a constant reaching the pin through a continuous assignment, a
    // slice or a concatenation counts as one — and **per bit**, because a
    // block RAM's address arrives as one fourteen-bit pin of which four
    // bits are the constant.
    let mut wanted: Vec<(CellId, String, Vec<Option<Bit>>)> = Vec::new();
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
                    // An output port of the cell is not in `inputs`, which
                    // is how a role name that means an input on one
                    // primitive and an output on another is told apart: an
                    // `io`'s `dout` carries the design's value *to* the
                    // pad, a `lutram`'s `dout` is what it reads back.
                    let Some(d) = cell.input(port) else { continue };
                    let Ok(bits) = view.expr_bits(d) else {
                        continue;
                    };
                    let values: Vec<Option<Bit>> = bits
                        .iter()
                        .map(|bit| match view.canonical(*bit) {
                            // An `x` or a `z` is not built: there is no
                            // wire value for either, so the pin is left as
                            // it is and the backend names it.
                            SigBit::Const(value @ (Bit::Zero | Bit::One)) => Some(value),
                            _ => None,
                        })
                        .collect();
                    if values.iter().any(Option::is_some) {
                        wanted.push((id, port.clone(), values));
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
    for (id, port, values) in wanted {
        let span = module.cells[id].span;
        let name = module.cells[id].name.as_str().to_owned();
        let Some(old) = module.cells[id].input(&port) else {
            continue;
        };
        // A pin with some constant bits and some real ones keeps the real
        // ones, and they are taken off a net rather than off the original
        // expression: a block RAM's address arrives as a concatenation, and
        // slicing *that* ten times would repeat the whole concatenation ten
        // times in everything downstream reads.
        let width = u32::try_from(values.len()).unwrap_or(1);
        let kept = values.iter().any(Option::is_none).then(|| {
            let net = add_net(module, &format!("{name}${port}"), Type::bits(width), span);
            add_assign(module, net, old, span);
            net_expr(module, net, span)
        });
        // One expression per bit, least significant first: the shared
        // driver where the bit was a constant, the pin's own bit where it
        // was not.
        let mut parts: Vec<ExprId> = Vec::with_capacity(values.len());
        for (index, value) in values.iter().enumerate() {
            let bit = u32::try_from(index).unwrap_or(0);
            let Some(value) = value else {
                let base = kept.unwrap_or(old);
                parts.push(slice_expr(module, base, bit, bit, span));
                continue;
            };
            let held = if *value == Bit::One {
                &mut one
            } else {
                &mut zero
            };
            let net = match *held {
                Some(net) => net,
                None => {
                    let net = constant_lut(module, device, &lut, *value, span, report, diags);
                    *held = Some(net);
                    net
                }
            };
            parts.push(net_expr(module, net, span));
            report.constants.push((name.clone(), port.clone(), *value));
        }
        // `Concat` is most significant first.
        parts.reverse();
        let driven = if parts.len() == 1 {
            parts[0]
        } else {
            expr(module, ExprKind::Concat(parts), span)
        };
        if let Some((_, slot)) = module.cells[id]
            .inputs
            .iter_mut()
            .find(|(p, _)| p.as_str() == port)
        {
            *slot = driven;
        }
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
            vec![("r$ff1".to_owned(), "DI".to_owned(), Bit::Zero)],
            "one flip-flop asked for a constant, and it is the top bit"
        );
        assert_eq!(report.constant_data_pins(), 1);
        assert!(
            report
                .to_text()
                .contains("r$ff1's DI is the constant 0, now driven"),
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
        assert_eq!(
            report.constants,
            vec![("r$ff1".to_owned(), "DI".to_owned(), Bit::One)]
        );
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
