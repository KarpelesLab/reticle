//! Packing `SB_LUT4`, `SB_DFF*` and `SB_CARRY` cells into iCE40 logic
//! cells.
//!
//! An iCE40 logic cell is a four-input lookup table, a carry and a
//! flip-flop, and the parts are not independent: the flip-flop's data is
//! the lookup table's output and nothing else; the carry's two operands are
//! the lookup table's second and third inputs, the same two wires; the
//! carry in of cell *k* is the carry out of cell *k-1*. A placer that put
//! the three on separate sites would produce a placement the silicon cannot
//! hold, so the three are made one cell here, `ICESTORM_LC`, before the
//! placer sees them — the same thing nextpnr's `pack` does, by the same
//! name.
//!
//! # The rules
//!
//! 1. A **carry** takes the lookup table whose second and third inputs are
//!    its two operands, preferring one whose fourth input is its carry in
//!    (an adder's sum bit). With none, it gets a cell of its own with an
//!    empty table.
//! 2. A **flip-flop** takes the lookup table driving its data when nothing
//!    else reads that table's output. Otherwise it gets a cell whose table
//!    passes its first input through (`16'hAAAA`), or holds the constant its
//!    data is tied to.
//! 3. Every other lookup table is a cell of its own.
//! 4. A lookup table input **tied to a constant** is folded into the truth
//!    table and left unconnected, because an unrouted input reads as zero
//!    (`icebox_vlog`'s default, which is what the silicon does). The one
//!    exception is an input the carry shares and that must be a one: that
//!    is wired to a cell whose table is all ones.
//! 5. A carry chain whose carry in is a **constant** starts with one more
//!    cell, a *feed-in* whose carry operands are that constant, so that its
//!    carry out is the constant whatever is below it. A chain's last carry
//!    out read by anything but the chain is handed to the fabric by a
//!    *feed-out* cell above it, whose table passes its carry in through.
//! 6. The flip-flop's enable and set/reset, unconnected, read as one and
//!    zero; tied to those constants they are left unconnected.
//! 7. A flip-flop's enable and synchronous set/reset go onto the cell's
//!    `CEN` and `SR` as they are, because an `SB_DFFESR` means what the
//!    silicon does: the reset acts only on an enabled edge (the iCE
//!    Technology Library's `if (E) Q <= R ? 0 : D`; `icebox_vlog` decodes
//!    the cell as `if (cen) q <= sr ? set : d`). Making a reset-wins
//!    register out of one is flip-flop mapping's job, which widens the
//!    enable to `E | R` there (`enable_gates_reset` in
//!    `devices/ice40.dev`), so that the netlist handed to nextpnr and the
//!    one packed here are the same and correct. Packing used to do the
//!    widening itself, for this backend only; see `docs/fpga-icestorm.md`.
//!
//! 8. A register **declared to start at one** is refused, naming it: an
//!    iCE40 flip-flop starts at zero and nothing here inverts one.
//!
//! What it refuses, with a message naming the cell: a clock, an enable of
//! zero or a set/reset of one tied to a constant, a carry out read both by
//! the chain and by the fabric, and an initial one.

use std::collections::BTreeMap;

use crate::ir::emit::{BitView, SigBit};
use crate::ir::{
    AttrValue, Attrs, Bit, Cell, CellId, CellKind, Const, Expr, ExprId, ExprKind, Module, Name,
    Net, NetId, NetKind, Type,
};
use crate::source::Span;

/// The packed cell's primitive name.
pub const LC: &str = "ICESTORM_LC";
/// The packed cell's name when its flip-flop is unused, which goes on a
/// site with no clock, enable or set/reset pin (see `IceFabric`'s `lcc`
/// bels).
pub const LC_COMB: &str = "ICESTORM_LC_COMB";

/// What packing did, for the report.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PackReport {
    /// Logic cells made.
    pub cells: usize,
    /// Of those, cells holding a flip-flop whose data had to be passed
    /// through a table of its own.
    pub pass_through: usize,
    /// Feed-in cells starting a chain with a constant carry in.
    pub feed_in: usize,
    /// Feed-out cells taking a chain's carry out to the fabric.
    pub feed_out: usize,
    /// Whether a cell driving a constant one was made.
    pub vcc: bool,
}

/// Why packing failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackError(pub String);

impl std::fmt::Display for PackError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for PackError {}

/// A flip-flop's variant, from its name: `SB_DFF[N][E][S][R|S]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FfKind {
    neg: bool,
    set: bool,
    asynchronous: bool,
}

fn ff_kind(primitive: &str) -> Option<FfKind> {
    let rest = primitive.strip_prefix("SB_DFF")?;
    let rest = rest.strip_prefix('N').map_or((false, rest), |r| (true, r));
    let (neg, rest) = rest;
    let rest = rest.strip_prefix('E').unwrap_or(rest);
    let (set, asynchronous) = match rest {
        "" => (false, false),
        "SR" => (false, false),
        "R" => (false, true),
        "SS" => (true, false),
        "S" => (true, true),
        _ => return None,
    };
    Some(FfKind {
        neg,
        set,
        asynchronous,
    })
}

/// One logic cell being assembled: its four inputs, carry in, flip-flop
/// controls and outputs, as canonical bits.
#[derive(Clone, Debug)]
struct Lc {
    name: String,
    span: Span,
    inputs: [Option<SigBit>; 4],
    init: u16,
    cin: Option<SigBit>,
    clk: Option<SigBit>,
    en: Option<SigBit>,
    sr: Option<SigBit>,
    /// The net the cell's output drives.
    out: Option<NetId>,
    /// The net its carry out drives.
    cout: Option<NetId>,
    carry: bool,
    dff: bool,
    set: bool,
    asynchronous: bool,
    neg: bool,
}

impl Lc {
    fn new(span: Span) -> Lc {
        Lc {
            name: String::new(),
            span,
            inputs: [None; 4],
            init: 0,
            cin: None,
            clk: None,
            en: None,
            sr: None,
            out: None,
            cout: None,
            carry: false,
            dff: false,
            set: false,
            asynchronous: false,
            neg: false,
        }
    }
}

/// A wire to be made: a one-bit net, by its name, whose bit stands for
/// itself.
enum Pin {
    Bit(SigBit),
    New(usize),
}

/// Packs every `SB_LUT4`, `SB_DFF*` and `SB_CARRY` of `module` into
/// [`LC`] cells, in place.
///
/// # Errors
///
/// [`PackError`] for what the module documentation lists as refused.
pub fn pack(module: &mut Module) -> Result<PackReport, PackError> {
    let Some(span) = module.cells.iter().next().map(|(_, c)| c.span) else {
        return Ok(PackReport::default());
    };
    let view = BitView::new(module).map_err(|e| PackError(e.to_string()))?;
    let bits_of = |e: ExprId| -> Result<Vec<SigBit>, PackError> {
        view.expr_bits(e).map_err(|err| PackError(err.to_string()))
    };
    // Every cell's connections, as canonical bits.
    struct Seen {
        id: CellId,
        primitive: String,
        inputs: BTreeMap<String, SigBit>,
        outputs: BTreeMap<String, (NetId, SigBit)>,
    }
    let mut seen: Vec<Seen> = Vec::new();
    // How many readers each slot has, counting a top-level output port.
    let mut readers: BTreeMap<usize, usize> = BTreeMap::new();
    for (id, cell) in module.cells.iter() {
        let CellKind::Blackbox(p) = &cell.kind else {
            continue;
        };
        let mut inputs = BTreeMap::new();
        for (port, e) in &cell.inputs {
            let bits = bits_of(*e)?;
            for b in &bits {
                if let SigBit::Slot(s) = view.canonical(*b) {
                    *readers.entry(s).or_default() += 1;
                }
            }
            if let [b] = bits.as_slice() {
                inputs.insert(port.as_str().to_owned(), view.canonical(*b));
            }
        }
        let mut outputs = BTreeMap::new();
        for (port, net) in &cell.outputs {
            let bits = view
                .net_bits(*net, cell.span)
                .map_err(|e| PackError(e.to_string()))?;
            if let [b] = bits.as_slice() {
                outputs.insert(port.as_str().to_owned(), (*net, view.canonical(*b)));
            }
        }
        seen.push(Seen {
            id,
            primitive: p.as_str().to_owned(),
            inputs,
            outputs,
        });
    }
    for port in &module.ports {
        if port.dir != crate::ir::PortDir::In
            && let Ok(bits) = view.net_bits(port.net, port.span)
        {
            for b in bits {
                if let SigBit::Slot(s) = view.canonical(b) {
                    *readers.entry(s).or_default() += 1;
                }
            }
        }
    }

    // Inputs of the other cells (the IO buffers, the RGB driver, the
    // oscillators) tied to a one: an unrouted input reads zero, so these are
    // wired to a cell that drives a one.
    let tied_high: Vec<(CellId, String)> = seen
        .iter()
        .filter(|s| {
            !matches!(s.primitive.as_str(), "SB_LUT4" | "SB_CARRY")
                && ff_kind(&s.primitive).is_none()
        })
        .flat_map(|s| {
            s.inputs
                .iter()
                .filter(|(port, bit)| {
                    **bit == SigBit::Const(Bit::One) && port.as_str() != "PACKAGE_PIN"
                })
                .map(|(port, _)| (s.id, port.clone()))
                .collect::<Vec<_>>()
        })
        .collect();
    // Rule 8: a flip-flop declared to start at one. An iCE40 flip-flop
    // starts at zero, and nothing in this flow inverts one to make it start
    // at one, so such a register would come up wrong and say nothing. It is
    // refused by name instead.
    let ff_q: BTreeMap<SigBit, String> = seen
        .iter()
        .filter(|s| ff_kind(&s.primitive).is_some())
        .filter_map(|s| {
            s.outputs
                .get("Q")
                .map(|(_, b)| (*b, module.cells[s.id].name.as_str().to_owned()))
        })
        .collect();
    for (net_id, net) in module.nets.iter() {
        let Some(AttrValue::Const(init)) = net.attrs.get("init") else {
            continue;
        };
        let Ok(slots) = view.net_slots(net_id, net.span) else {
            continue;
        };
        for (bit, slot) in slots.iter().enumerate() {
            let Ok(index) = u32::try_from(bit) else {
                continue;
            };
            if index >= init.width() || init.bit(index) != Bit::One {
                continue;
            }
            if let Some(cell) = ff_q.get(&view.canonical(SigBit::Slot(*slot))) {
                return Err(PackError(format!(
                    "`{}[{bit}]` is declared to start at one, and its flip-flop `{cell}` would \
                     start at zero: an iCE40 flip-flop always does, and this flow does not invert \
                     one to change that. Declare the register's complement instead",
                    net.name
                )));
            }
        }
    }
    let is = |s: &Seen, p: &str| s.primitive == p;
    let luts: Vec<usize> = (0..seen.len())
        .filter(|&i| is(&seen[i], "SB_LUT4"))
        .collect();
    let carries: Vec<usize> = (0..seen.len())
        .filter(|&i| is(&seen[i], "SB_CARRY"))
        .collect();
    let ffs: Vec<usize> = (0..seen.len())
        .filter(|&i| ff_kind(&seen[i].primitive).is_some())
        .collect();
    let lut_out: BTreeMap<SigBit, usize> = luts
        .iter()
        .filter_map(|&i| seen[i].outputs.get("O").map(|(_, b)| (*b, i)))
        .collect();
    let input = |i: usize, port: &str| seen[i].inputs.get(port).copied();
    let zero = SigBit::Const(Bit::Zero);
    let init_of = |i: usize| -> u16 {
        let cell = &module.cells[seen[i].id];
        cell.params
            .get("LUT_INIT")
            .and_then(AttrValue::as_int)
            .map_or(0, |v| u16::try_from(v & 0xFFFF).unwrap_or(0))
    };

    let mut taken_lut: BTreeMap<usize, usize> = BTreeMap::new();
    // Each cell being assembled, with the indices into `seen` of its lookup
    // table, its flip-flop and its carry.
    type Parts = (Lc, Option<usize>, Option<usize>, Option<usize>);
    let mut lcs: Vec<Parts> = Vec::new();

    // Rule 1: carries.
    for &c in &carries {
        let (i0, i1, ci) = (
            input(c, "I0").unwrap_or(zero),
            input(c, "I1").unwrap_or(zero),
            input(c, "CI").unwrap_or(zero),
        );
        // A table that reads nothing but the carry's operands and its carry
        // in can share the cell, its inputs rearranged to the carry's
        // wires; a sum bit, which reads the carry in, is preferred.
        let operands: Vec<SigBit> = [i0, i1, ci]
            .into_iter()
            .filter(|b| matches!(b, SigBit::Slot(_)))
            .collect();
        let reads = |l: usize| -> Vec<SigBit> {
            ["I0", "I1", "I2", "I3"]
                .iter()
                .filter_map(|p| input(l, p))
                .filter(|b| matches!(b, SigBit::Slot(_)))
                .collect()
        };
        let candidates: Vec<usize> = luts
            .iter()
            .copied()
            .filter(|l| !taken_lut.contains_key(l))
            .filter(|&l| {
                let r = reads(l);
                !r.is_empty() && r.iter().all(|b| operands.contains(b))
            })
            .collect();
        let lut = candidates
            .iter()
            .copied()
            .find(|&l| reads(l).contains(&ci))
            .or_else(|| candidates.first().copied());
        let index = lcs.len();
        if let Some(l) = lut {
            taken_lut.insert(l, index);
        }
        lcs.push((Lc::new(span), lut, None, Some(c)));
    }
    // Rule 2: flip-flops.
    for &f in &ffs {
        let d = input(f, "D").unwrap_or(zero);
        let lut = lut_out.get(&d).copied().filter(|l| {
            let SigBit::Slot(s) = d else { return false };
            readers.get(&s).copied().unwrap_or(0) == 1
                && taken_lut.get(l).is_none_or(|&at| lcs[at].2.is_none())
        });
        match lut.and_then(|l| taken_lut.get(&l).copied().map(|at| (l, at))) {
            Some((_, at)) => lcs[at].2 = Some(f),
            None => {
                let index = lcs.len();
                if let Some(l) = lut {
                    taken_lut.insert(l, index);
                }
                lcs.push((Lc::new(span), lut, Some(f), None));
            }
        }
    }
    // Rule 3: the rest.
    for &l in &luts {
        if let std::collections::btree_map::Entry::Vacant(slot) = taken_lut.entry(l) {
            slot.insert(lcs.len());
            lcs.push((Lc::new(span), Some(l), None, None));
        }
    }

    // Now each cell's connections.
    let mut report = PackReport::default();
    let mut built: Vec<Lc> = Vec::new();
    for (mut lc, lut, ff, carry) in lcs {
        let anchor = carry.or(lut).or(ff).unwrap_or(0);
        lc.name = module.cells[seen[anchor].id].name.as_str().to_owned();
        lc.span = module.cells[seen[anchor].id].span;
        if let Some(l) = lut {
            lc.inputs = [
                input(l, "I0"),
                input(l, "I1"),
                input(l, "I2"),
                input(l, "I3"),
            ];
            lc.init = init_of(l);
            lc.out = seen[l].outputs.get("O").map(|(n, _)| *n);
        }
        if let Some(c) = carry {
            lc.carry = true;
            let (i0, i1, ci) = (
                input(c, "I0").unwrap_or(zero),
                input(c, "I1").unwrap_or(zero),
                input(c, "CI").unwrap_or(zero),
            );
            if lut.is_some() {
                // The table's inputs onto the carry's wires: operand A on
                // the second input, B on the third, the carry in on the
                // fourth, which a pip can take from the carry chain.
                lc.init = retarget(lc.init, lc.inputs, &[(i0, 1), (i1, 2), (ci, 3)]);
                lc.inputs = [None, None, None, Some(ci)];
            }
            lc.inputs[1] = Some(i0);
            lc.inputs[2] = Some(i1);
            lc.cin = Some(ci);
            lc.cout = seen[c].outputs.get("CO").map(|(n, _)| *n);
        }
        if let Some(f) = ff {
            let kind = ff_kind(&seen[f].primitive).unwrap_or(FfKind {
                neg: false,
                set: false,
                asynchronous: false,
            });
            let fname = module.cells[seen[f].id].name.as_str().to_owned();
            if lut.is_none() {
                report.pass_through += 1;
                let d = input(f, "D").unwrap_or(zero);
                lc.inputs = [Some(d), None, None, None];
                lc.init = 0xAAAA;
            }
            lc.dff = true;
            lc.set = kind.set;
            lc.asynchronous = kind.asynchronous;
            lc.neg = kind.neg;
            lc.out = seen[f].outputs.get("Q").map(|(n, _)| *n);
            lc.clk = input(f, "C");
            if matches!(lc.clk, Some(SigBit::Const(_)) | None) {
                return Err(PackError(format!(
                    "flip-flop `{fname}` has a constant clock, which an iCE40 logic cell cannot \
                     be given"
                )));
            }
            lc.en = match input(f, "E") {
                None | Some(SigBit::Const(Bit::One)) => None,
                Some(SigBit::Const(_)) => {
                    return Err(PackError(format!(
                        "flip-flop `{fname}` has its enable tied low, so it never loads; this \
                         flow does not build that"
                    )));
                }
                other => other,
            };
            let sr_port = if kind.set { "S" } else { "R" };
            lc.sr = match input(f, sr_port) {
                None | Some(SigBit::Const(Bit::Zero)) => None,
                Some(SigBit::Const(_)) => {
                    return Err(PackError(format!(
                        "flip-flop `{fname}` has its set/reset tied high; this flow does not \
                         build that"
                    )));
                }
                other => other,
            };
        }
        built.push(lc);
    }

    // Rule 4, 5: constants, feed-ins, feed-outs.
    let mut new_nets: Vec<String> = Vec::new();
    let mut extra: Vec<Lc> = Vec::new();
    let mut vcc: Option<usize> = None;
    let mut want_vcc = |new_nets: &mut Vec<String>| -> usize {
        *vcc.get_or_insert_with(|| {
            new_nets.push("$ice40$vcc".to_owned());
            new_nets.len() - 1
        })
    };
    // Which cell's carry in each carry out feeds, and who else reads it.
    let cout_bit = |lc: &Lc| -> Option<SigBit> {
        let net = lc.cout?;
        view.net_bits(net, lc.span)
            .ok()
            .and_then(|b| b.first().map(|b| view.canonical(*b)))
    };
    let mut pin_override: BTreeMap<(usize, usize), Pin> = BTreeMap::new();
    let mut cin_override: BTreeMap<usize, Pin> = BTreeMap::new();
    let mut cout_override: BTreeMap<usize, usize> = BTreeMap::new();
    if !tied_high.is_empty() {
        want_vcc(&mut new_nets);
    }
    // Rule 4: a lookup table input tied to a constant.
    for (i, lc) in built.iter_mut().enumerate() {
        for j in 0..4 {
            let Some(SigBit::Const(c)) = lc.inputs[j] else {
                continue;
            };
            let one = c == Bit::One;
            if lc.carry && (j == 1 || j == 2) && one {
                let v = want_vcc(&mut new_nets);
                pin_override.insert((i, j), Pin::New(v));
                lc.inputs[j] = None;
                continue;
            }
            let mut folded = 0u16;
            for idx in 0..16u16 {
                let from = if one { idx | 1 << j } else { idx & !(1 << j) };
                if lc.init >> from & 1 != 0 {
                    folded |= 1 << idx;
                }
            }
            lc.init = folded;
            lc.inputs[j] = None;
        }
    }
    // Rule 5.
    let carry_of_cin: BTreeMap<SigBit, usize> = built
        .iter()
        .enumerate()
        .filter(|(_, lc)| lc.carry)
        .filter_map(|(i, lc)| lc.cin.map(|b| (b, i)))
        .collect();
    for i in 0..built.len() {
        if !built[i].carry {
            continue;
        }
        if let Some(SigBit::Const(c)) = built[i].cin {
            report.feed_in += 1;
            new_nets.push(format!("{}$ci", built[i].name));
            let net = new_nets.len() - 1;
            let feed = Lc {
                name: format!("{}$feed_in", built[i].name),
                span: built[i].span,
                carry: true,
                ..Lc::new(span)
            };
            let one = c == Bit::One;
            let v = one.then(|| want_vcc(&mut new_nets));
            extra.push(feed);
            let at = built.len() + extra.len() - 1;
            if let Some(v) = v {
                pin_override.insert((at, 1), Pin::New(v));
                pin_override.insert((at, 2), Pin::New(v));
            }
            cout_override.insert(at, net);
            cin_override.insert(i, Pin::New(net));
        }
        // Rule 5, second half: a carry out read outside the chain.
        let Some(co) = cout_bit(&built[i]) else {
            continue;
        };
        let SigBit::Slot(slot) = co else { continue };
        let next = carry_of_cin.get(&co).copied();
        let inside = next.map_or(0, |n| {
            1 + built[n].inputs.iter().filter(|b| **b == Some(co)).count()
        });
        let total = readers.get(&slot).copied().unwrap_or(0);
        if total > inside {
            if next.is_some() {
                return Err(PackError(format!(
                    "the carry out of `{}` is read by the next carry and by other logic; an \
                     iCE40 carry out reaches only the cell above it, so this flow does not build \
                     that",
                    built[i].name
                )));
            }
            // The carry gets a fresh net; the feed-out drives the old one.
            report.feed_out += 1;
            new_nets.push(format!("{}$co", built[i].name));
            let raw = new_nets.len() - 1;
            let old = built[i].cout;
            cout_override.insert(i, raw);
            extra.push(Lc {
                name: format!("{}$feed_out", built[i].name),
                span: built[i].span,
                init: 0xFF00,
                out: old,
                ..Lc::new(span)
            });
            let at = built.len() + extra.len() - 1;
            cin_override.insert(at, Pin::New(raw));
            pin_override.insert((at, 3), Pin::New(raw));
        }
    }
    if vcc.is_some() {
        report.vcc = true;
        extra.push(Lc {
            name: "$ice40$vcc".to_owned(),
            init: 0xFFFF,
            ..Lc::new(span)
        });
    }
    built.extend(extra);
    let vcc_cell = vcc.map(|_| built.len() - 1);

    let owners: Vec<(usize, (NetId, u32))> =
        (0..view.slots()).map(|s| (s, view.owner(s))).collect();
    // Build the new cells. First the new nets.
    let mut new_net_ids: Vec<NetId> = Vec::new();
    for name in &new_nets {
        let id = module.nets.push(Net {
            name: unique(module, name),
            ty: Type::bit(),
            kind: NetKind::Wire,
            attrs: Attrs::new(),
            span,
        });
        new_net_ids.push(id);
    }
    let mut slot_expr: BTreeMap<usize, ExprId> = BTreeMap::new();
    let mut expr_for = |module: &mut Module, pin: &Pin| -> ExprId {
        match pin {
            Pin::New(n) => net_bit(module, new_net_ids[*n], None, span),
            Pin::Bit(SigBit::Const(c)) => module.add_expr(Expr::new(
                ExprKind::Const(Const::from_bits(&[*c])),
                Type::bit(),
                span,
            )),
            Pin::Bit(SigBit::Slot(s)) => *slot_expr.entry(*s).or_insert_with(|| {
                let (net, bit) = owners[*s].1;
                let width = module.nets[net].ty.width().unwrap_or(1);
                net_bit(module, net, (width > 1).then_some(bit), span)
            }),
        }
    };
    let tied_names: Vec<(String, String)> = tied_high
        .iter()
        .map(|(id, port)| (module.cells[*id].name.as_str().to_owned(), port.clone()))
        .collect();
    let old: Vec<CellId> = luts
        .iter()
        .chain(&carries)
        .chain(&ffs)
        .map(|&i| seen[i].id)
        .collect();
    module.cells.retain(|id, _| !old.contains(&id));
    for (i, lc) in built.iter().enumerate() {
        let mut inputs: Vec<(Name, ExprId)> = Vec::new();
        for (j, input) in lc.inputs.iter().enumerate() {
            let pin = match pin_override.remove(&(i, j)) {
                Some(p) => Some(p),
                None => input.map(Pin::Bit),
            };
            if let Some(pin) = pin {
                let e = expr_for(module, &pin);
                inputs.push((Name::new(format!("I{j}")), e));
            }
        }
        let cin = match cin_override.remove(&i) {
            Some(p) => Some(p),
            None => lc.cin.map(Pin::Bit),
        };
        if lc.carry
            && let Some(pin) = cin
        {
            let e = expr_for(module, &pin);
            inputs.push((Name::new("CIN"), e));
        } else if let Some(Pin::New(n)) = cin {
            // A feed-out: its carry in is only there to place it.
            let e = expr_for(module, &Pin::New(n));
            inputs.push((Name::new("CIN"), e));
        }
        for (port, bit) in [("CLK", lc.clk), ("CEN", lc.en), ("SR", lc.sr)] {
            if let Some(pin) = bit.map(Pin::Bit) {
                let e = expr_for(module, &pin);
                inputs.push((Name::new(port), e));
            }
        }
        let mut outputs: Vec<(Name, NetId)> = Vec::new();
        let out = if Some(i) == vcc_cell {
            vcc.map(|v| new_net_ids[v])
        } else {
            lc.out
        };
        if let Some(net) = out {
            outputs.push((Name::new("O"), net));
        }
        let cout = cout_override.get(&i).map(|n| new_net_ids[*n]).or(lc.cout);
        if lc.carry
            && let Some(net) = cout
        {
            outputs.push((Name::new("COUT"), net));
        }
        let mut params = Attrs::new();
        params.set(
            Name::new("LUT_INIT"),
            AttrValue::Const(Const::from_u64(u64::from(lc.init), 16)),
        );
        for (key, on) in [
            ("CARRY_ENABLE", lc.carry),
            ("DFF_ENABLE", lc.dff),
            ("SET_NORESET", lc.set),
            ("ASYNC_SR", lc.asynchronous),
            ("NEG_CLK", lc.neg),
        ] {
            params.set(
                Name::new(key),
                AttrValue::Const(Const::from_u64(u64::from(on), 1)),
            );
        }
        let name = unique(module, &format!("{}$lc", lc.name));
        module.cells.push(Cell {
            name,
            kind: CellKind::Blackbox(Name::new(if lc.dff { LC } else { LC_COMB })),
            inputs,
            outputs,
            params,
            attrs: Attrs::new(),
            span: lc.span,
        });
    }
    if let Some(v) = vcc {
        // `retain` renumbers the cells it keeps, so the tied cells are found
        // again by name.
        for (name, port) in &tied_names {
            let e = net_bit(module, new_net_ids[v], None, span);
            if let Some((_, c)) = module
                .cells
                .iter_mut()
                .find(|(_, c)| c.name.as_str() == name.as_str())
            {
                for (p, input) in &mut c.inputs {
                    if p.as_str() == port.as_str() {
                        *input = e;
                    }
                }
            }
        }
    }
    report.cells = built.len();
    Ok(report)
}

/// A truth table re-expressed for new input positions: `inputs` are the
/// table's present inputs, and each signal among them moves to the position
/// `target` gives it. A constant input keeps its constant; the table comes
/// out not depending on any position no signal moved to.
fn retarget(init: u16, inputs: [Option<SigBit>; 4], target: &[(SigBit, usize)]) -> u16 {
    let mut out = 0u16;
    for new_idx in 0..16u16 {
        let mut old_idx = 0u16;
        for (j, input) in inputs.iter().enumerate() {
            let bit = match input {
                Some(SigBit::Const(c)) => *c == Bit::One,
                Some(signal) => target
                    .iter()
                    .find(|(s, _)| s == signal)
                    .is_some_and(|(_, p)| new_idx >> p & 1 != 0),
                None => false,
            };
            if bit {
                old_idx |= 1 << j;
            }
        }
        if init >> old_idx & 1 != 0 {
            out |= 1 << new_idx;
        }
    }
    out
}

/// `net[bit]`, or `net` when it is one bit wide.
fn net_bit(module: &mut Module, net: NetId, bit: Option<u32>, span: Span) -> ExprId {
    let ty = module.nets[net].ty.clone();
    let base = module.add_expr(Expr::new(ExprKind::Net(net), ty, span));
    match bit {
        None => base,
        Some(b) => module.add_expr(Expr::new(
            ExprKind::Slice { base, hi: b, lo: b },
            Type::bit(),
            span,
        )),
    }
}

/// `base`, or `base$<n>` for the first `n` no net or cell has.
fn unique(module: &Module, base: &str) -> Name {
    let taken = |name: &str| {
        module.nets.iter().any(|(_, n)| n.name.as_str() == name)
            || module.cells.iter().any(|(_, c)| c.name.as_str() == name)
    };
    if !taken(base) {
        return Name::new(base);
    }
    (1..)
        .map(|n| format!("{base}${n}"))
        .find(|c| !taken(c))
        .map(Name::new)
        .unwrap_or_else(|| Name::new(base))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A table moved to new positions computes the same function of the
    /// same signals: an exclusive-or of a carry in on input 0 and an
    /// operand on input 1 becomes one of input 3 and input 1.
    #[test]
    fn a_table_follows_its_signals() {
        let (a, ci) = (SigBit::Slot(7), SigBit::Slot(9));
        // I0 = ci, I1 = a, I2 = I3 = 0: XOR of I0 and I1 is 0x6666.
        let init = retarget(
            0x6666,
            [Some(ci), Some(a), Some(SigBit::Const(Bit::Zero)), None],
            &[(a, 1), (SigBit::Const(Bit::Zero), 2), (ci, 3)],
        );
        for idx in 0..16u16 {
            let (i1, i3) = (idx >> 1 & 1, idx >> 3 & 1);
            assert_eq!(init >> idx & 1, i1 ^ i3, "index {idx}");
        }
    }

    #[test]
    fn flip_flop_names_say_what_they_are() {
        let plain = ff_kind("SB_DFF").unwrap();
        assert!(!plain.neg && !plain.set && !plain.asynchronous);
        let ner = ff_kind("SB_DFFNER").unwrap();
        assert!(ner.neg && !ner.set && ner.asynchronous);
        let ess = ff_kind("SB_DFFESS").unwrap();
        assert!(!ess.neg && ess.set && !ess.asynchronous);
        let s = ff_kind("SB_DFFS").unwrap();
        assert!(s.set && s.asynchronous);
        assert!(ff_kind("SB_LUT4").is_none());
        assert!(ff_kind("SB_DFFX").is_none());
    }
}
