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
//! becomes seven resetting flip-flops and one setting one. When the
//! device declares no variant matching a bit — an iCE40 flip-flop with an
//! active-low reset, say, since `SB_DFF*` set and reset pins are active
//! high — the cell is left generic and a diagnostic says exactly which
//! flip-flop was wanted and which ones the device has. Leaving it generic
//! rather than approximating it is deliberate: [`super::flow`]'s netlist
//! check then reports the cell as not a device primitive, and the user
//! sees one honest error instead of a netlist that simulates differently
//! from the design.
//!
//! [`BelRole::Lut`]: super::device::BelRole::Lut
//! [`BelRole::Ff`]: super::device::BelRole::Ff
//! [`FfVariant`]: super::device::FfVariant

use std::fmt::Write as _;

use super::device::{BelKind, BelRole, Device, FfReset, FfVariant};
use super::primitives::{add_assign, add_cell, add_net, const_expr, expr, net_expr, slice_expr};
use crate::diag::{Diagnostic, Diagnostics};
use crate::ir::{
    AttrValue, Bit, Cell, CellId, CellKind, Const, Design, ExprId, ExprKind, Module, ModuleId,
    Name, Type,
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
}

impl CellMapReport {
    /// True when nothing was rewritten and nothing was declined.
    pub fn is_empty(&self) -> bool {
        self.primitives.is_empty() && self.declined.is_empty()
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
    let mut replaced: Vec<CellId> = Vec::new();
    for id in ffs {
        if map_flip_flop(module, device, id, report, diags) {
            replaced.push(id);
        }
    }
    if !replaced.is_empty() {
        module.cells.retain(|id, _| !replaced.contains(&id));
    }
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
fn map_flip_flop(
    module: &mut Module,
    device: &Device,
    id: CellId,
    report: &mut CellMapReport,
    diags: &mut Diagnostics,
) -> bool {
    let cell = &module.cells[id];
    let kind = cell.kind.clone();
    let name = cell.name.as_str().to_owned();
    let span = cell.span;
    let (Some(clk), Some(d), Some(q)) = (cell.input("clk"), cell.input("d"), cell.output("q"))
    else {
        report
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
    let mut bels: Vec<BelKind> = Vec::with_capacity(usize::try_from(width).unwrap_or(0));
    for bit in 0..width {
        let variant = variant_of(&kind, bit);
        match device.ff_variant(variant) {
            Some(bel) => bels.push(bel.clone()),
            None => {
                diags.push(
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
                report.declined.push((
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
        let bel = &bels[0];
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
        report.bump(&bel.name, 1);
        return false;
    }

    let mut bits = Vec::with_capacity(usize::try_from(width).unwrap_or(0));
    for bit in 0..width {
        let bel = &bels[usize::try_from(bit).unwrap_or(0)];
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
        report.bump(&bel.name, 1);
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

    fn cells_named(design: &Design, top: ModuleId, primitive: &str) -> usize {
        design
            .module(top)
            .cells
            .iter()
            .filter(|(_, c)| matches!(&c.kind, CellKind::Blackbox(n) if n.as_str() == primitive))
            .count()
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
    fn a_flip_flop_the_device_cannot_build_is_reported() {
        // An iCE40 set/reset is active high, so an active-low one has no
        // primitive at all.
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
        let mut diags = Diagnostics::new();
        let device = target("ice40-hx1k-tq144").unwrap();
        let report = map_cells(&mut design, top, device, &mut diags);
        assert!(diags.has_errors());
        let text = diags.render(&map);
        assert!(
            text.contains(
                "has no flip-flop with a rising clock edge, an active-low asynchronous reset"
            ),
            "{text}"
        );
        assert!(text.contains("SB_DFFR"), "{text}");
        assert_eq!(report.declined.len(), 1);
        assert_eq!(report.count("SB_DFF"), 0);
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
