//! Making a mapped `CARRY4` chain something a 7-series slice can hold.
//!
//! # Why the mapped netlist is not enough
//!
//! Mapping writes a `CARRY4` the way the primitive's model reads:
//! `O = S ^ {CO[2:0], CI | CYINIT}` and `CO[i] = S[i] ? CO[i-1] : DI[i]`,
//! with `S` any signal at all. The slice is narrower than the model in two
//! ways, and both are in the database rather than in a datasheet:
//!
//! - **`S` is the lookup table's output.** `ppips_clbll_l.db` gives the
//!   propagate input no wire of its own — the only thing it records near
//!   it is `CLBLL_L_A.CLBLL_L_A1 hint`, the lookup table used as a wire —
//!   so bit `n`'s propagate is whatever the lookup table at position `n`
//!   of the same slice computes. Mapping `count + 1` leaves 25 of the 26
//!   propagate bits as a flip-flop's output with no lookup table at all,
//!   because `count[i] ^ 0` is folded to `count[i]`.
//! - **A constant generate input has one source.** `DI` comes from the
//!   slice's bypass input `<L>X` (the default, `CARRY4.<L>CY0` clear) or
//!   from the lookup table's `O5` (`<L>CY0` set). A bypass input that is
//!   not routed reads one, so a constant zero comes from `O5`, or from a
//!   ground the flow ties to the bypass input after routing, as Vivado ties
//!   it ([`CarryOptions::generates_on_the_bypass`]).
//!
//! [`legalise_carries`] closes both, and changes no logic doing it:
//!
//! - a propagate bit driven by anything but a lookup table it can have to
//!   itself gets a **buffer** lookup table, `O = I0`, in front of it — which
//!   is what Vivado calls a route-through, and makes with no cell, on `A6`
//!   (`O = I5` here, with [`CarryOptions::buffers_on_a6`]);
//! - a constant propagate bit gets a lookup table whose output is that
//!   constant;
//! - a constant generate bit makes its lane's lookup table one of these
//!   two built here, with the constant in the **lower half** of the truth
//!   table, which is what `O5` reads, and the function in the upper half,
//!   which is what `O6` reads with `A6` high. `I5` is tied to one in the
//!   netlist so the cell's own model reads the upper half too: the netlist
//!   says what the silicon does, and a simulator of it agrees.
//!
//! - a carry in from the fabric (`CYINIT`, on the slice's `AX`) beside a
//!   generate from the fabric on bit 0, which would want `AX` too, puts
//!   the generate on the lane's `O5`: its propagate's table and its
//!   generate become one `LUT6_2` (the propagate on `O6`, the generate on
//!   `O5`, `I5` tied high), which is how Vivado builds `a + b + c` with `c`
//!   on `CYINIT` (see `SharedAx`). When the two need more than five
//!   inputs between them the carry in goes on a `CARRY4` of its own below
//!   the chain instead, its top lane passing it up, which costs what
//!   writing it as a lane did.
//!
//! - a carry out the fabric reads from a lane whose sum it reads too, which
//!   the slice's one output for that lane cannot carry both of, is read off
//!   the lane above instead, as Vivado does (`move_carry_outs`); this one
//!   adds a lane, or a `CARRY4` above the chain, but changes no value.
//!
//! **A lane nothing reads is left alone**, and its two inputs become `x`.
//! A lane is read when its sum is, when a carry out at or above it is,
//! or when a sum above it is: the sum of lane `j` is `S ^ CO` of the lane
//! below, so every lane under a read sum is read through its carry.
//! Mapping pads the top `CARRY4` of a 26-bit adder with two lanes of
//! zeros; their sums and carries go nowhere, so whatever lookup table the
//! placer puts at their positions changes nothing anyone sees, and giving
//! them tables of their own would spend two lookup tables and two bits of
//! `<L>CY0` on silence.
//!
//! # What this does not do
//!
//! It does not place anything. That the propagate's lookup table lands in
//! the same slice at the same position, and that the next `CARRY4` lands
//! in the slice directly above, is the placer's business, and the placer
//! reads it off the routing graph rather than off this module: see
//! `place`'s dedicated connections.

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::fpga::device::{BelRole, Device};
use crate::fpga::place::{Netlist, Signal};
use crate::fpga::primitives::{
    Names, add_assign, add_cell, add_cell_in, add_net, add_net_in, const_expr, expr, net_expr,
    slice_expr,
};
use crate::ir::emit::{BitView, SigBit};
use crate::ir::{
    AttrValue, Bit, CellId, CellKind, Const, Design, ExprId, ExprKind, ModuleId, Name, PortDir,
    Type,
};

/// What [`legalise_carries`] did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CarryPacking {
    /// `CARRY4` instances looked at.
    pub carries: usize,
    /// Propagate bits already driven by a lookup table of their own.
    pub kept: usize,
    /// Buffer lookup tables added in front of a propagate bit.
    pub buffers: usize,
    /// Lookup tables added for a constant propagate bit.
    pub constants: usize,
    /// Of the added tables, those carrying a constant generate in their
    /// lower half for `O5`.
    pub generates: usize,
    /// Lanes whose outputs nothing reads.
    pub dead_lanes: usize,
    /// Chains whose carry in from the fabric shares `AX` with bit 0's
    /// generate, that generate put on the lane's `O5` (`SharedAx` in this module).
    pub generates_on_o5: usize,
    /// Of those, the ones whose two tables need more than five inputs, the
    /// carry in on a `CARRY4` of its own below the chain instead.
    pub carries_below: usize,
    /// Carry outs read from the lane above the one whose sum is read too,
    /// since the two would want one output of the slice
    /// (`move_carry_outs`).
    pub carries_out_moved: usize,
    /// Constant generates put on their propagate table's own `O5`
    /// ([`CarryOptions::generates_on_their_tables`]), with no buffer.
    pub generates_folded: usize,
    /// Constant generates left on the lane's bypass input, to be tied there
    /// after routing ([`CarryOptions::generates_on_the_bypass`]), with no
    /// buffer and nothing in the table.
    pub generates_on_bypass: usize,
}

impl CarryPacking {
    /// One line, for a report. The constant generates folded onto their
    /// propagate's table are named only when there are any, so that a build
    /// that does not ask for it reports what it always did.
    pub fn to_text(&self) -> String {
        let mut folded = if self.generates_folded > 0 {
            format!(
                ", {} constant generate(s) on their propagate's own table",
                self.generates_folded
            )
        } else {
            String::new()
        };
        if self.generates_on_bypass > 0 {
            folded.push_str(&format!(
                ", {} constant generate(s) tied on the lane's bypass input",
                self.generates_on_bypass
            ));
        }
        format!(
            "carry chains: {} CARRY4, {} propagate bit(s) on their own lookup table, \
             {} buffer and {} constant lookup table(s) added ({} feeding a constant \
             generate), {} lane(s) nothing reads, {} carry in(s) from the fabric beside a \
             generate on `O5` and {} on a `CARRY4` below, {} carry out(s) read a lane above \
             a sum read beside them{folded}\n",
            self.carries,
            self.kept,
            self.buffers,
            self.constants,
            self.generates,
            self.dead_lanes,
            self.generates_on_o5,
            self.carries_below,
            self.carries_out_moved
        )
    }
}

/// What one lane's propagate input becomes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Propagate {
    /// Keep it: a lookup table drives it and drives no other propagate.
    Keep,
    /// A buffer of this slot.
    Buffer(usize),
    /// A constant.
    Constant(bool),
    /// Nobody reads the lane: `x`.
    Dead,
    /// The lookup table that drives it, rewritten with the lane's constant
    /// generate in its lower half for `O5`: it reads five signals or fewer,
    /// so `A6` is free to choose the half. Only with
    /// [`CarryOptions::generates_on_their_tables`].
    Fold,
    /// Keep the lookup table that drives it, and leave the lane's constant
    /// generate on the slice's bypass input `<L>X`, which is tied to the
    /// constant after routing as Vivado ties it (`GND_WIRE`, a `GFAN`, the
    /// lane's `BYP_ALT`, for a zero; nothing driven, for a one). Only with
    /// [`CarryOptions::generates_on_the_bypass`].
    Tie,
}

/// What [`legalise_carries_with`] may do beyond what a slice requires.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CarryOptions {
    /// Put a lane's constant generate on the `O5` of the lookup table that
    /// is already its propagate, when that table reads five signals or fewer
    /// and no other propagate, rather than on a buffer table of its own in
    /// front of it. The buffer is a level of logic and a connection on every
    /// path through the lane: an equality compared on the chain, whose
    /// generate is a constant zero, was a lane table, a wire and a buffer
    /// before the `CARRY4`. Vivado has no buffer there: it ties the generate
    /// to the slice's bypass input, `GND_WIRE` through a `GFAN` and a
    /// `BYP_ALT`, which this flow now does too
    /// ([`CarryOptions::generates_on_the_bypass`]), and this is what is left
    /// where it cannot: lane `A` beside a carry in from the fabric. Off,
    /// every constant generate takes a buffer as it always did; `reticle
    /// fpga` turns it on with the default `--synthesis vivado`.
    pub generates_on_their_tables: bool,
    /// Leave a lane's constant generate on the slice's bypass input `<L>X`,
    /// for the flow to tie there after routing, wherever its propagate is a
    /// lookup table of its own and the bypass is not the carry in's (lane
    /// `A` beside a carry in from the fabric falls back to
    /// [`CarryOptions::generates_on_their_tables`] or a buffer). This is
    /// Vivado's own way: its routed designs ground a constant generate as
    /// `GND_WIRE` -> `GFAN<k>` -> `BYP_ALT<n>` -> `BYP<n>` -> `<L>X`, with
    /// `<L>CY0` clear, and give a one the bypass undriven, which reads
    /// `VCC_WIRE` (`docs/fpga-vivado-parity.md`, "A ground on the bypass
    /// input"). The table is left as it is, so a lane may read six inputs.
    ///
    /// The lanes are marked on the `CARRY4` ([`BYPASS_GENERATES`]); the flow
    /// gives each such pin a signal nothing drives
    /// ([`tie_bypass_generates`]), so that the placer keeps every other
    /// signal off that bypass input, and ties it after routing. `reticle
    /// fpga` turns it on with the default `--synthesis vivado`.
    pub generates_on_the_bypass: bool,
    /// Put a buffer in front of a propagate on its table's `A6`, its truth
    /// table the upper half (`O6 = A6`), and keep it there
    /// ([`ROUTE_THROUGH`]). That is Vivado's way through the lane's table to
    /// the propagate, with no cell: its routed `carry_counter` takes each of
    /// its counter's propagates in on `<L>6`, through `<L>6LUT/A6` to
    /// `CARRY4/S<k>`, the table's `INIT` the upper half and nothing else of
    /// the slice set (`docs/fpga-vivado-parity.md`, "A way through to the
    /// propagate"). A buffer on another input is a table to Vivado, which
    /// given an empty table goes through it on `A6` whatever the route
    /// (`xray::vivado`). Not for a buffer whose lower half holds a constant
    /// generate, which needs `A6` high. `reticle fpga` turns it on with the
    /// default `--synthesis vivado`.
    pub buffers_on_a6: bool,
}

/// The attribute [`legalise_carries_with`] marks a buffer table with when it
/// puts it on `A6` ([`CarryOptions::buffers_on_a6`]): the router keeps the
/// signal there (`xray::lut_input_classes`), so that the table is a way through
/// on the input Vivado's is. An attribute, so that nothing writes it out.
pub const ROUTE_THROUGH: &str = "route_through";

/// The attribute [`legalise_carries_with`] marks a `CARRY4` with when some
/// of its lanes' constant generates are left on the bypass input
/// ([`CarryOptions::generates_on_the_bypass`]): an integer, bit `n` set for
/// lane `n`. An attribute rather than a parameter, so that nothing writes it
/// out: Vivado, given the constant on `DI`, makes the same tie itself.
pub const BYPASS_GENERATES: &str = "bypass_generates";

/// A function of at most a few signals, as a lookup table computes it:
/// `inputs` the distinct signals, by where each lives in the module, and
/// `truth[i]` the value when input `j` is bit `j` of `i`.
#[derive(Clone, Debug)]
struct Function {
    inputs: Vec<(usize, (crate::ir::NetId, u32))>,
    truth: Vec<bool>,
    /// The table it is read from, which the packed one replaces, and the
    /// net that table drives.
    table: Option<(CellId, crate::ir::NetId)>,
}

impl Function {
    /// `I0` of a table, passed through.
    fn buffer(slot: usize, owner: (crate::ir::NetId, u32)) -> Function {
        Function {
            inputs: vec![(slot, owner)],
            truth: vec![false, true],
            table: None,
        }
    }

    /// A constant.
    fn constant(value: bool) -> Function {
        Function {
            inputs: Vec::new(),
            truth: vec![value],
            table: None,
        }
    }

    /// Its value over `signals`, a superset of its inputs, at the
    /// assignment `x` gives them.
    fn at(&self, signals: &[usize], x: usize) -> bool {
        let mut i = 0usize;
        for (k, (slot, _)) in self.inputs.iter().enumerate() {
            let at = signals.iter().position(|s| s == slot).unwrap_or(0);
            if (x >> at) & 1 == 1 {
                i |= 1 << k;
            }
        }
        self.truth[i]
    }
}

/// A chain whose carry in comes from the fabric and whose bit 0's generate
/// does too: the two want the slice's one `AX`, since `CYINIT` is reached
/// from `AX` alone and a generate from the fabric by the bypass input.
///
/// Vivado puts the generate on `O5`: bit 0's propagate on `O6` and the
/// generate on `O5` of one `LUT6_2`, `CARRY4.ACY0` set (the shared adders,
/// `s ? c - d : a + b`, and `a + b + c` with `c` on `CYINIT`). That is
/// [`SharedAx::Pin`], when the two functions read five signals or fewer
/// between them: the generate's own table is folded in when nothing else
/// reads it, and it is passed through from `I0`-to-`I4` otherwise. Past
/// five, [`SharedAx::Below`]: a `CARRY4` of its own under the chain, whose
/// top lane's carry out is the carry in (propagate a constant zero, the
/// generate the signal on its bypass input), so that the chain's own
/// `CYINIT` is a zero and `AX` is the generate's.
#[derive(Clone, Debug)]
enum SharedAx {
    /// Both on one table: the propagate's function and the generate's.
    Pin { upper: Function, lower: Function },
    /// The carry in a `CARRY4` below.
    Below,
}

/// One lane's plan: what its propagate input becomes, and the constant
/// its generate input is when it is one.
#[derive(Clone, Copy, Debug)]
struct Lane {
    propagate: Propagate,
    generate: Option<bool>,
}

/// Rewrites every wide-carry instance of `module` so a 7-series slice can
/// hold it; see the module docs.
///
/// A device with no wide carry element, or a module with none of its
/// instances, is left exactly as it was.
///
/// # Errors
///
/// A message when the netlist cannot be read bit by bit, or when a device
/// with a carry in from the fabric beside a generate from it has no
/// `LUT6_2` to put the generate on `O5` with and no `CI` to bring the carry
/// in from below (this module's `SharedAx`).
pub fn legalise_carries(
    design: &mut Design,
    module: ModuleId,
    device: &Device,
) -> Result<CarryPacking, String> {
    legalise_carries_with(design, module, device, &CarryOptions::default())
}

/// [`legalise_carries`], with what [`CarryOptions`] allows beyond it.
///
/// # Errors
///
/// As [`legalise_carries`].
pub fn legalise_carries_with(
    design: &mut Design,
    module: ModuleId,
    device: &Device,
    options: &CarryOptions,
) -> Result<CarryPacking, String> {
    let mut report = CarryPacking::default();
    let Some(carry) = device.bel(BelRole::Carry).cloned() else {
        return Ok(report);
    };
    let Some(shape) = carry.wide_carry() else {
        return Ok(report);
    };
    let Some(lut) = device.bel(BelRole::Lut).cloned() else {
        return Ok(report);
    };
    let lut_inputs: Vec<String> = lut.port_names("i").into_iter().map(str::to_owned).collect();
    let lut_output = lut.port("o").unwrap_or("O").to_owned();
    let init_name = lut
        .params
        .first()
        .map_or_else(|| "INIT".to_owned(), |(n, _)| n.clone());
    // The construction below needs a sixth input to choose between the two
    // halves; a device whose lookup table is narrower has no `O5` to speak
    // of either, and is left alone.
    if lut_inputs.len() != 6 {
        return Ok(report);
    }
    report.carries_out_moved = move_carry_outs(design, module, &carry.name, &shape)?;
    let Some(m) = design.modules.get(module) else {
        return Err("the module is not part of the design".to_owned());
    };

    // The plan is made against an immutable view, then applied. Each plan
    // carries, per lane, the net and bit a buffer lane reads, found in the
    // same view its slot numbers came from: building a view of the whole
    // module again for every chain to ask it was a quadratic, 17 s of the
    // Husky's 1,681 chains.
    type Owners = Vec<Option<(crate::ir::NetId, u32)>>;
    // Per chain: its lanes, the net and bit each buffer lane reads, bit 0's
    // shared `AX`, and each folded lane's table.
    type Plan = (
        CellId,
        Vec<Lane>,
        Owners,
        Option<SharedAx>,
        Vec<Option<Function>>,
    );
    let mut plans: Vec<Plan> = Vec::new();
    {
        let view = BitView::new(m).map_err(|e| e.to_string())?;
        let bits = |id: ExprId| -> Result<Vec<SigBit>, String> {
            Ok(view
                .expr_bits(id)
                .map_err(|e| e.to_string())?
                .into_iter()
                .map(|b| view.canonical(b))
                .collect())
        };

        // Which bits anything reads, how often, and who drives each bit.
        let mut read: HashSet<usize> = HashSet::new();
        let mut reads: HashMap<usize, usize> = HashMap::new();
        let mut driver: HashMap<usize, (CellId, String)> = HashMap::new();
        for (id, cell) in m.cells.iter() {
            for (_, e) in &cell.inputs {
                for bit in bits(*e)? {
                    if let SigBit::Slot(s) = bit {
                        read.insert(s);
                        *reads.entry(s).or_default() += 1;
                    }
                }
            }
            for (port, net) in &cell.outputs {
                let out = view.net_bits(*net, cell.span).map_err(|e| e.to_string())?;
                for bit in out {
                    if let SigBit::Slot(s) = view.canonical(bit) {
                        driver.insert(s, (id, port.as_str().to_owned()));
                    }
                }
            }
        }
        for port in &m.ports {
            if port.dir == PortDir::In {
                continue;
            }
            for bit in view
                .net_bits(port.net, port.span)
                .map_err(|e| e.to_string())?
            {
                if let SigBit::Slot(s) = view.canonical(bit) {
                    read.insert(s);
                    *reads.entry(s).or_default() += 2;
                }
            }
        }
        // A lookup table's function, over the distinct signals it reads, its
        // constant pins folded in; `None` when it is not one this can read.
        let table_of = |id: CellId| -> Option<Function> {
            let cell = &m.cells[id];
            if !matches!(&cell.kind, CellKind::Blackbox(n) if n.as_str() == lut.name) {
                return None;
            }
            let Some(AttrValue::Const(init)) = cell.params.get(init_name.as_str()) else {
                return None;
            };
            let output = cell.output(&lut_output)?;
            let mut pins: Vec<SigBit> = Vec::with_capacity(6);
            for name in &lut_inputs {
                match cell.input(name) {
                    Some(e) => match bits(e).ok()?.as_slice() {
                        [b] => pins.push(*b),
                        _ => return None,
                    },
                    None => pins.push(SigBit::Const(Bit::Zero)),
                }
            }
            let mut slots: Vec<usize> = Vec::new();
            for pin in &pins {
                match pin {
                    SigBit::Slot(s) if !slots.contains(s) => slots.push(*s),
                    SigBit::Slot(_) => {}
                    SigBit::Const(b) if *b == Bit::Zero || *b == Bit::One => {}
                    SigBit::Const(_) => return None,
                }
            }
            let mut truth = vec![false; 1 << slots.len()];
            for (i, value) in truth.iter_mut().enumerate() {
                let mut index = 0u32;
                for (j, pin) in pins.iter().enumerate() {
                    let high = match pin {
                        SigBit::Const(b) => *b == Bit::One,
                        SigBit::Slot(s) => {
                            let k = slots.iter().position(|x| x == s).unwrap_or(0);
                            (i >> k) & 1 == 1
                        }
                    };
                    if high {
                        index |= 1 << j;
                    }
                }
                *value = init.bit(index) == Bit::One;
            }
            Some(Function {
                inputs: slots.into_iter().map(|s| (s, view.owner(s))).collect(),
                truth,
                table: Some((id, output)),
            })
        };

        // How many propagate pins each bit feeds, over every chain: a
        // lookup table can be the propagate of one lane of one slice.
        let carries: Vec<CellId> = m
            .cells
            .iter()
            .filter(|(_, c)| matches!(&c.kind, CellKind::Blackbox(n) if n.as_str() == carry.name))
            .map(|(id, _)| id)
            .collect();
        // A constant generate goes on its propagate's own table only when
        // asked, and only on a device with the `LUT6_2` it takes.
        let fold_here = options.generates_on_their_tables
            && device.bels.iter().any(|b| b.name == super::lutpack::DUAL);
        let mut propagate_uses: BTreeMap<usize, usize> = BTreeMap::new();
        for id in &carries {
            if let Some(e) = m.cells[*id].input(shape.propagate) {
                for bit in bits(e)? {
                    if let SigBit::Slot(s) = bit {
                        *propagate_uses.entry(s).or_default() += 1;
                    }
                }
            }
        }

        for id in carries {
            let cell = &m.cells[id];
            report.carries += 1;
            let lanes = shape.width as usize;
            let (Some(s_expr), Some(di_expr)) =
                (cell.input(shape.propagate), cell.input(shape.data))
            else {
                return Err(format!(
                    "`{}` has no `{}` or `{}`",
                    cell.name, shape.propagate, shape.data
                ));
            };
            let s_bits = bits(s_expr)?;
            let di_bits = bits(di_expr)?;
            let out_bits = |port: &str| -> Result<Vec<SigBit>, String> {
                match cell.output(port) {
                    Some(net) => Ok(view
                        .net_bits(net, cell.span)
                        .map_err(|e| e.to_string())?
                        .into_iter()
                        .map(|b| view.canonical(b))
                        .collect()),
                    None => Ok(Vec::new()),
                }
            };
            let sums = out_bits(shape.sum)?;
            let carries_out = out_bits(shape.carry_out)?;
            if s_bits.len() != lanes || di_bits.len() != lanes {
                return Err(format!("`{}` is not {lanes} bits wide", cell.name));
            }
            let is_read =
                |b: Option<&SigBit>| matches!(b, Some(SigBit::Slot(s)) if read.contains(s));

            // The carry in, and whether it needs `AX`.
            let fabric_init = shape
                .init
                .and_then(|p| cell.input(p))
                .map(bits)
                .transpose()?
                .is_some_and(|b| matches!(b.first(), Some(SigBit::Slot(_))));

            let mut plan = Vec::with_capacity(lanes);
            // Each `Propagate::Fold` lane's table, as read.
            let mut folds: Vec<Option<Function>> = vec![None; lanes];
            let mut shared_ax = false;
            for lane in 0..lanes {
                // Read itself, or through its carry: a carry out at or above
                // it, or a sum above it, which is `S ^ CO` of the lane
                // below. That last was left out, and a chain read at its
                // top sum alone (an equality compared on the chain, the
                // carry out of `{0, L} + 1`) lost every propagate below
                // that sum to whatever lookup table the placer seated there.
                let live = is_read(sums.get(lane))
                    || (lane..lanes).any(|j| is_read(carries_out.get(j)))
                    || (lane + 1..lanes).any(|j| is_read(sums.get(j)));
                if !live {
                    report.dead_lanes += 1;
                    plan.push(Lane {
                        propagate: Propagate::Dead,
                        generate: None,
                    });
                    continue;
                }
                let generate = match di_bits[lane] {
                    // The generate *is* the propagate, and a lookup table
                    // drives it: `CO = S ? CI : S`, which is `S ? CI : 0`.
                    // A slice cannot have it both ways — that table is the
                    // propagate's own, and the same signal on the bypass
                    // input would have to leave the slice and come back —
                    // so it is the constant it amounts to. `(a - b) + c`
                    // makes this lane for every bit `c` does not reach,
                    // and the placer refused every site for it. A counter's
                    // propagate is a flip-flop's output and gets a buffer
                    // table, so its generate stays on the bypass input.
                    SigBit::Slot(s)
                        if di_bits[lane] == s_bits[lane]
                            && driver.get(&s).is_some_and(|(d, port)| {
                                matches!(&m.cells[*d].kind, CellKind::Blackbox(n) if n.as_str() == lut.name)
                                    && *port == lut_output
                            }) =>
                    {
                        Some(false)
                    }
                    SigBit::Slot(_) => None,
                    SigBit::Const(b) => Some(b == Bit::One),
                };
                if generate.is_none() && lane == 0 && fabric_init {
                    shared_ax = true;
                }
                let propagate = match s_bits[lane] {
                    SigBit::Const(b) => Propagate::Constant(b == Bit::One),
                    SigBit::Slot(s) => {
                        let own_lut = driver.get(&s).is_some_and(|(d, port)| {
                            matches!(&m.cells[*d].kind, CellKind::Blackbox(n) if n.as_str() == lut.name)
                                && *port == lut_output
                        });
                        let alone = own_lut && propagate_uses.get(&s) == Some(&1);
                        if alone && generate.is_none() {
                            Propagate::Keep
                        } else if alone
                            && options.generates_on_the_bypass
                            && generate.is_some()
                            && !(lane == 0 && fabric_init)
                        {
                            Propagate::Tie
                        } else if let Some(table) = driver
                            .get(&s)
                            .filter(|_| alone && fold_here)
                            .and_then(|(d, _)| table_of(*d))
                            .filter(|f| f.inputs.len() <= 5)
                        {
                            folds[lane] = Some(table);
                            Propagate::Fold
                        } else {
                            Propagate::Buffer(s)
                        }
                    }
                };
                plan.push(Lane {
                    propagate,
                    generate,
                });
            }
            let owners = plan
                .iter()
                .map(|lane| match lane.propagate {
                    Propagate::Buffer(s) => Some(view.owner(s)),
                    _ => None,
                })
                .collect();
            // Bit 0's propagate and generate on one table, or the carry in
            // below: see `SharedAx`.
            let shared = if shared_ax {
                let upper = match plan[0].propagate {
                    Propagate::Keep => match s_bits[0] {
                        SigBit::Slot(s) => driver.get(&s).and_then(|(d, _)| table_of(*d)),
                        SigBit::Const(_) => None,
                    },
                    Propagate::Buffer(s) => Some(Function::buffer(s, view.owner(s))),
                    Propagate::Constant(v) => Some(Function::constant(v)),
                    // A fold or a tie has a constant generate, and this a
                    // signal one.
                    Propagate::Dead | Propagate::Fold | Propagate::Tie => None,
                };
                let lower = match di_bits[0] {
                    SigBit::Slot(g) => {
                        // Its own table, when the generate is all it is for
                        // and it is not the propagate's.
                        let own = driver
                            .get(&g)
                            .filter(|_| reads.get(&g) == Some(&1))
                            .and_then(|(d, _)| table_of(*d))
                            .filter(|f| {
                                f.table.map(|(c, _)| c)
                                    != upper.as_ref().and_then(|u| u.table.map(|(c, _)| c))
                            });
                        Some(own.unwrap_or_else(|| Function::buffer(g, view.owner(g))))
                    }
                    SigBit::Const(_) => None,
                };
                Some(match (upper, lower) {
                    (Some(upper), Some(lower))
                        if upper.inputs.len()
                            + lower
                                .inputs
                                .iter()
                                .filter(|(s, _)| !upper.inputs.iter().any(|(u, _)| u == s))
                                .count()
                            <= 5 =>
                    {
                        SharedAx::Pin { upper, lower }
                    }
                    _ => SharedAx::Below,
                })
            } else {
                None
            };
            plans.push((id, plan, owners, shared, folds));
        }
    }

    // Apply.
    let Some(m) = design.modules.get_mut(module) else {
        return Err("the module is not part of the design".to_owned());
    };
    let mut removed: Vec<CellId> = Vec::new();
    let dual = device.bels.iter().find(|b| b.name == super::lutpack::DUAL);
    // Every net and cell this adds takes its name from one index of the
    // module's names, and nothing is removed until the end: see `Names`.
    let mut names = Names::of(m);
    for (id, plan, view_owner, shared, folds) in plans {
        let span = m.cells[id].span;
        let name = m.cells[id].name.as_str().to_owned();
        // Bit 0's two tables as one, or the carry in below the chain.
        let mut lane0: Option<(ExprId, ExprId)> = None;
        match shared {
            Some(SharedAx::Pin { upper, lower }) => {
                let Some(dual) = dual else {
                    return Err(format!(
                        "`{name}` takes its carry in and bit 0's generate both from the \
                         fabric, and the device has no `{}` to put the generate on `O5`",
                        super::lutpack::DUAL
                    ));
                };
                let dual_inputs: Vec<String> = dual
                    .port_names("i")
                    .into_iter()
                    .map(str::to_owned)
                    .collect();
                let (Some(o6), Some(o5)) = (dual.port("o"), dual.port("o5")) else {
                    return Err(format!("`{}` has no `O6` and `O5`", dual.name));
                };
                let mut signals: Vec<usize> = upper.inputs.iter().map(|(s, _)| *s).collect();
                let mut owners: Vec<(crate::ir::NetId, u32)> =
                    upper.inputs.iter().map(|(_, o)| *o).collect();
                for (slot, owner) in &lower.inputs {
                    if !signals.contains(slot) {
                        signals.push(*slot);
                        owners.push(*owner);
                    }
                }
                let mut init = Const::zero(64);
                for x in 0..32u32 {
                    if upper.at(&signals, x as usize) {
                        init.set_bit(32 + x, Bit::One);
                    }
                    if lower.at(&signals, x as usize) {
                        init.set_bit(x, Bit::One);
                    }
                }
                let mut inputs: Vec<(Name, ExprId)> = Vec::with_capacity(6);
                for (k, pin) in dual_inputs.iter().enumerate() {
                    let value = match owners.get(k) {
                        Some((net, b)) if k < 5 => {
                            let whole = net_expr(m, *net, span);
                            slice_expr(m, whole, *b, *b, span)
                        }
                        // `I5` is `A6`, high so that `O6` reads the upper half.
                        _ if k == 5 => const_expr(m, Const::ones(1), span),
                        _ => const_expr(m, Const::zero(1), span),
                    };
                    inputs.push((Name::new(pin.clone()), value));
                }
                let out_of = |m: &mut crate::ir::Module,
                              names: &mut Names,
                              f: &Function,
                              tag: &str| match f.table {
                    Some((_, net)) => net,
                    None => add_net_in(m, names, &format!("{name}${tag}0"), Type::bit(), span),
                };
                let o6_net = out_of(m, &mut names, &upper, "s");
                let o5_net = out_of(m, &mut names, &lower, "g");
                let cell = add_cell_in(
                    m,
                    &mut names,
                    &format!("{name}$lane0"),
                    CellKind::Blackbox(Name::new(dual.name.clone())),
                    inputs,
                    vec![(Name::new(o6), o6_net), (Name::new(o5), o5_net)],
                    span,
                );
                m.cells[cell]
                    .params
                    .set(Name::new("INIT"), AttrValue::Const(init));
                removed.extend(upper.table.iter().chain(&lower.table).map(|(c, _)| *c));
                lane0 = Some((net_expr(m, o6_net, span), net_expr(m, o5_net, span)));
                report.generates_on_o5 += 1;
            }
            Some(SharedAx::Below) => {
                let (Some(ci), Some(init)) = (shape.carry_in, shape.init) else {
                    return Err(format!(
                        "`{name}` takes its carry in and bit 0's generate both from the \
                         fabric, and the carry has no `CI` to take it from below"
                    ));
                };
                let lanes = shape.width;
                let carry_in = m.cells[id].input(init).expect("checked in the plan");
                // The top lane's propagate: a table of zero.
                let zero = add_net_in(m, &mut names, &format!("{name}$cin$s"), Type::bit(), span);
                let mut table_inputs: Vec<(Name, ExprId)> = Vec::with_capacity(6);
                for pin in &lut_inputs {
                    table_inputs
                        .push((Name::new(pin.clone()), const_expr(m, Const::zero(1), span)));
                }
                let table = add_cell_in(
                    m,
                    &mut names,
                    &format!("{name}$cin$lut"),
                    CellKind::Blackbox(Name::new(lut.name.clone())),
                    table_inputs,
                    vec![(Name::new(lut_output.clone()), zero)],
                    span,
                );
                m.cells[table].params.set(
                    Name::new(init_name.clone()),
                    AttrValue::Const(lane_init(Some(false), None)),
                );
                report.constants += 1;
                // `Concat` is most significant first: the top lane, then
                // three that nothing reads.
                let x =
                    |m: &mut crate::ir::Module| const_expr(m, Const::from_bits(&[Bit::X]), span);
                let mut s_parts = vec![net_expr(m, zero, span)];
                let mut di_parts = vec![carry_in];
                for _ in 1..lanes {
                    s_parts.push(x(m));
                    di_parts.push(x(m));
                }
                let s_expr = expr(m, ExprKind::Concat(s_parts), span);
                let di_expr = expr(m, ExprKind::Concat(di_parts), span);
                let sum = add_net_in(
                    m,
                    &mut names,
                    &format!("{name}$cin$o"),
                    Type::bits(lanes),
                    span,
                );
                let carries = add_net_in(
                    m,
                    &mut names,
                    &format!("{name}$cin$co"),
                    Type::bits(lanes),
                    span,
                );
                let low = |m: &mut crate::ir::Module| const_expr(m, Const::zero(1), span);
                let inputs = vec![
                    (Name::new(shape.propagate), s_expr),
                    (Name::new(shape.data), di_expr),
                    (Name::new(ci), low(m)),
                    (Name::new(init), low(m)),
                ];
                add_cell_in(
                    m,
                    &mut names,
                    &format!("{name}$cin"),
                    CellKind::Blackbox(Name::new(carry.name.clone())),
                    inputs,
                    vec![
                        (Name::new(shape.sum), sum),
                        (Name::new(shape.carry_out), carries),
                    ],
                    span,
                );
                let whole = net_expr(m, carries, span);
                let top = slice_expr(m, whole, lanes - 1, lanes - 1, span);
                let zero_in = low(m);
                for (port, value) in m.cells[id].inputs.iter_mut() {
                    if port.as_str() == ci {
                        *value = top;
                    } else if port.as_str() == init {
                        *value = zero_in;
                    }
                }
                report.carries += 1;
                report.dead_lanes += usize::try_from(lanes - 1).unwrap_or(0);
                report.carries_below += 1;
            }
            None => {}
        }
        let old_s = m.cells[id]
            .input(shape.propagate)
            .expect("checked in the plan");
        let old_di = m.cells[id].input(shape.data).expect("checked in the plan");
        let mut s_parts: Vec<ExprId> = Vec::with_capacity(plan.len());
        let mut di_parts: Vec<ExprId> = Vec::with_capacity(plan.len());
        // The lanes whose generate is left on the bypass input.
        let mut bypass = 0i64;
        for (lane, step) in plan.iter().enumerate() {
            let bit = u32::try_from(lane).unwrap_or(0);
            let x = || Const::from_bits(&[Bit::X]);
            let (s, di) = match step.propagate {
                _ if lane == 0 && lane0.is_some() => lane0.expect("just checked"),
                Propagate::Dead => (const_expr(m, x(), span), const_expr(m, x(), span)),
                Propagate::Keep => (
                    slice_expr(m, old_s, bit, bit, span),
                    slice_expr(m, old_di, bit, bit, span),
                ),
                Propagate::Tie => {
                    // The propagate as it was and the generate the constant
                    // it is, which the flow ties on the lane's bypass input.
                    let Some(generate) = step.generate else {
                        return Err(format!("`{name}` lane {lane}: a tie with nothing to tie"));
                    };
                    bypass |= 1 << lane;
                    report.generates_on_bypass += 1;
                    (
                        slice_expr(m, old_s, bit, bit, span),
                        const_expr(m, Const::from_bits(&[bit_of(generate)]), span),
                    )
                }
                Propagate::Fold => {
                    // The propagate's own table made a `LUT6_2`: its function
                    // on `O6`, the upper half over `I0`..`I4` with `I5` tied
                    // high, for every reader of its output as before, and the
                    // constant generate on `O5`, the lower half, which the
                    // lane's generate now reads as a signal. As `SharedAx`
                    // builds bit 0's.
                    let (Some(table), Some(generate)) = (folds[lane].as_ref(), step.generate)
                    else {
                        return Err(format!("`{name}` lane {lane}: a fold with nothing to fold"));
                    };
                    let Some((old, out)) = table.table else {
                        return Err(format!("`{name}` lane {lane}: a fold of no table"));
                    };
                    let Some(dual) = dual else {
                        return Err(format!(
                            "`{name}` lane {lane}: the device has no `{}` to put the generate on",
                            super::lutpack::DUAL
                        ));
                    };
                    let (Some(o6), Some(o5)) = (dual.port("o"), dual.port("o5")) else {
                        return Err(format!("`{}` has no `O6` and `O5`", dual.name));
                    };
                    let dual_inputs: Vec<String> = dual
                        .port_names("i")
                        .into_iter()
                        .map(str::to_owned)
                        .collect();
                    let signals: Vec<usize> = table.inputs.iter().map(|(s, _)| *s).collect();
                    let mut init = Const::zero(64);
                    for x in 0..32u32 {
                        if table.at(&signals, x as usize) {
                            init.set_bit(32 + x, Bit::One);
                        }
                        if generate {
                            init.set_bit(x, Bit::One);
                        }
                    }
                    let mut inputs: Vec<(Name, ExprId)> = Vec::with_capacity(6);
                    for (k, pin) in dual_inputs.iter().enumerate() {
                        let value = match table.inputs.get(k) {
                            Some((_, (net, b))) if k < 5 => {
                                let whole = net_expr(m, *net, span);
                                slice_expr(m, whole, *b, *b, span)
                            }
                            _ if k == 5 => const_expr(m, Const::ones(1), span),
                            _ => const_expr(m, Const::zero(1), span),
                        };
                        inputs.push((Name::new(pin.clone()), value));
                    }
                    let g =
                        add_net_in(m, &mut names, &format!("{name}$g{lane}"), Type::bit(), span);
                    let cell = add_cell_in(
                        m,
                        &mut names,
                        &format!("{name}$fold{lane}"),
                        CellKind::Blackbox(Name::new(dual.name.clone())),
                        inputs,
                        vec![(Name::new(o6), out), (Name::new(o5), g)],
                        span,
                    );
                    m.cells[cell]
                        .params
                        .set(Name::new("INIT"), AttrValue::Const(init));
                    removed.push(old);
                    report.generates_folded += 1;
                    (slice_expr(m, old_s, bit, bit, span), net_expr(m, g, span))
                }
                Propagate::Buffer(_) | Propagate::Constant(_) => {
                    let input = view_owner[lane].map(|(net, b)| {
                        let whole = net_expr(m, net, span);
                        slice_expr(m, whole, b, b, span)
                    });
                    let constant = match step.propagate {
                        Propagate::Constant(v) => Some(v),
                        _ => None,
                    };
                    if input.is_some() {
                        report.buffers += 1;
                    } else {
                        report.constants += 1;
                    }
                    if step.generate.is_some() {
                        report.generates += 1;
                    }
                    let out =
                        add_net_in(m, &mut names, &format!("{name}$s{lane}"), Type::bit(), span);
                    let mut init = lane_init(constant, step.generate);
                    let split = step.generate.is_some_and(|d| {
                        // The halves differ unless the constant generate
                        // equals a constant propagate.
                        constant != Some(d)
                    });
                    // A buffer on `A6`, as Vivado's way through: see
                    // `CarryOptions::buffers_on_a6`.
                    let on_a6 = options.buffers_on_a6 && input.is_some() && !split;
                    if on_a6 {
                        init = Const::from_u64(0xFFFF_FFFF_0000_0000, 64);
                    }
                    let signal_at = if on_a6 { 5 } else { 0 };
                    let mut inputs: Vec<(Name, ExprId)> = Vec::with_capacity(6);
                    for (index, pin) in lut_inputs.iter().enumerate() {
                        let value = match (index, input) {
                            (i, Some(e)) if i == signal_at => e,
                            (5, _) if split => const_expr(m, Const::ones(1), span),
                            _ => const_expr(m, Const::zero(1), span),
                        };
                        inputs.push((Name::new(pin.clone()), value));
                    }
                    let cell = add_cell_in(
                        m,
                        &mut names,
                        &format!("{name}$lut{lane}"),
                        CellKind::Blackbox(Name::new(lut.name.clone())),
                        inputs,
                        vec![(Name::new(lut_output.clone()), out)],
                        span,
                    );
                    m.cells[cell]
                        .params
                        .set(Name::new(init_name.clone()), AttrValue::Const(init));
                    if on_a6 {
                        m.cells[cell].attrs.set(ROUTE_THROUGH, AttrValue::Int(1));
                    }
                    let di = match step.generate {
                        Some(d) => const_expr(m, Const::from_bits(&[bit_of(d)]), span),
                        None => slice_expr(m, old_di, bit, bit, span),
                    };
                    (net_expr(m, out, span), di)
                }
            };
            if step.propagate == Propagate::Keep && !(lane == 0 && lane0.is_some()) {
                report.kept += 1;
            }
            s_parts.push(s);
            di_parts.push(di);
        }
        // `Concat` is most significant first.
        s_parts.reverse();
        di_parts.reverse();
        let s = expr(m, ExprKind::Concat(s_parts), span);
        let di = expr(m, ExprKind::Concat(di_parts), span);
        let cell = &mut m.cells[id];
        for (port, value) in cell.inputs.iter_mut() {
            if port.as_str() == shape.propagate {
                *value = s;
            } else if port.as_str() == shape.data {
                *value = di;
            }
        }
        if bypass != 0 {
            cell.attrs.set(BYPASS_GENERATES, AttrValue::Int(bypass));
        }
    }
    // The tables folded into a lane's `LUT6_2`, taken out last: a cell's id
    // is its place, which a removal moves.
    if !removed.is_empty() {
        removed.sort_unstable();
        m.cells.retain(|id, _| removed.binary_search(&id).is_err());
    }
    Ok(report)
}

/// Takes each carry out that the fabric reads beside a sum it reads too
/// off the lane above instead, and returns how many it moved.
///
/// A slice has one output per lane for both: lane `k`'s sum and its carry
/// out leave by `<L>MUX` alone (`<L>OUTMUX.XOR` or `<L>OUTMUX.CY`), so when
/// the fabric wants both, one of the two has no way out. The router went a
/// hundred passes over that one wire and gave up, and the flow wrote a
/// bitstream with nothing routed: the top bit of `{0, a} + {0, b}` is the
/// carry out (`MapOptions::carry_ends`) and its top sum is the bit below,
/// and a second adder reading both (`a + b + c`, `vh_adder`'s `gadd`) is all
/// it takes.
///
/// Vivado never reads the two off one lane, and what it does instead was
/// measured (2026.1, `{co, s} = a + b` for 6, 7 and 8 bits, placed and
/// routed): the carry out comes off the lane above the top sum, that lane's
/// propagate a constant one and its generate zero, so that its carry out is
/// the carry into it. For six bits that is `CO[2]` of the second `CARRY4`
/// beside `O[1]`; for eight, `CO[0]` of a third `CARRY4` whose `CI` is the
/// second's `CO[3]` — which Vivado does even when the sums are registered,
/// though the flip-flop beside the lane could have taken one of them.
/// This does the same, with the lane above when nothing reads it and with a
/// `CARRY4` of its own above the chain when the carry out is its top lane's
/// and nothing continues the chain. A lane above that something reads, or
/// a chain that goes on, is left as it was: neither has been seen.
///
/// The carry out's readers are not touched: the instance's carry outs are
/// given a net of their own and the old net is assigned from it, the moved
/// bit from the lane above.
fn move_carry_outs(
    design: &mut Design,
    module: ModuleId,
    carry: &str,
    shape: &crate::fpga::device::WideCarry<'_>,
) -> Result<usize, String> {
    let lanes = shape.width as usize;
    // Per instance: the lane whose carry out moves, and whether it moves
    // into a `CARRY4` of its own above.
    let mut moves: Vec<(CellId, usize, bool)> = Vec::new();
    {
        let Some(m) = design.modules.get(module) else {
            return Err("the module is not part of the design".to_owned());
        };
        let view = BitView::new(m).map_err(|e| e.to_string())?;
        let bits = |id: ExprId| -> Result<Vec<SigBit>, String> {
            Ok(view
                .expr_bits(id)
                .map_err(|e| e.to_string())?
                .into_iter()
                .map(|b| view.canonical(b))
                .collect())
        };
        // Who reads each bit: a cell's pin, or `None` for a port of the
        // module.
        let mut readers: HashMap<usize, Vec<Option<(CellId, String)>>> = HashMap::new();
        for (id, cell) in m.cells.iter() {
            for (port, e) in &cell.inputs {
                for bit in bits(*e)? {
                    if let SigBit::Slot(s) = bit {
                        readers
                            .entry(s)
                            .or_default()
                            .push(Some((id, port.as_str().to_owned())));
                    }
                }
            }
        }
        for port in &m.ports {
            if port.dir == PortDir::In {
                continue;
            }
            for bit in view
                .net_bits(port.net, port.span)
                .map_err(|e| e.to_string())?
            {
                if let SigBit::Slot(s) = view.canonical(bit) {
                    readers.entry(s).or_default().push(None);
                }
            }
        }
        let is_carry =
            |id: CellId| matches!(&m.cells[id].kind, CellKind::Blackbox(n) if n.as_str() == carry);
        for (id, cell) in m.cells.iter() {
            if !is_carry(id) {
                continue;
            }
            let out_bits = |port: &str| -> Result<Vec<SigBit>, String> {
                match cell.output(port) {
                    Some(net) => Ok(view
                        .net_bits(net, cell.span)
                        .map_err(|e| e.to_string())?
                        .into_iter()
                        .map(|b| view.canonical(b))
                        .collect()),
                    None => Ok(Vec::new()),
                }
            };
            let sums = out_bits(shape.sum)?;
            let carries_out = out_bits(shape.carry_out)?;
            if sums.len() != lanes || carries_out.len() != lanes {
                continue;
            }
            let read_by = |b: &SigBit| -> &[Option<(CellId, String)>] {
                match b {
                    SigBit::Slot(s) => readers.get(s).map_or(&[], Vec::as_slice),
                    SigBit::Const(_) => &[],
                }
            };
            // The top lane's carry out into the next instance's `CI` is
            // `COUT`, a wire of its own; anything else reads it through
            // the lane's output.
            let chained = |r: &Option<(CellId, String)>| matches!(r, Some((c, p)) if is_carry(*c) && Some(p.as_str()) == shape.carry_in);
            for k in 0..lanes {
                let fabric = read_by(&carries_out[k])
                    .iter()
                    .any(|r| !(k + 1 == lanes && chained(r)));
                if !fabric || read_by(&sums[k]).is_empty() {
                    continue;
                }
                if k + 1 < lanes {
                    let above_free = (k + 1..lanes).all(|j| {
                        read_by(&sums[j]).is_empty() && read_by(&carries_out[j]).is_empty()
                    });
                    if above_free {
                        moves.push((id, k, false));
                    }
                } else if !read_by(&carries_out[k]).iter().any(chained) {
                    moves.push((id, k, true));
                }
                // One lane per instance can be the top sum's; a second is
                // not a shape the mapper writes.
                break;
            }
        }
    }

    let Some(m) = design.modules.get_mut(module) else {
        return Err("the module is not part of the design".to_owned());
    };
    let width = shape.width;
    for &(id, k, above) in &moves {
        let span = m.cells[id].span;
        let name = m.cells[id].name.as_str().to_owned();
        let Some(old) = m.cells[id].output(shape.carry_out) else {
            continue;
        };
        let new = add_net(m, &format!("{name}$co"), Type::bits(width), span);
        for (port, net) in m.cells[id].outputs.iter_mut() {
            if port.as_str() == shape.carry_out {
                *net = new;
            }
        }
        let lane = u32::try_from(k).unwrap_or(0);
        let one = |m: &mut crate::ir::Module| const_expr(m, Const::ones(1), span);
        let zero = |m: &mut crate::ir::Module| const_expr(m, Const::zero(1), span);
        let x = |m: &mut crate::ir::Module| const_expr(m, Const::from_bits(&[Bit::X]), span);
        // The value the moved bit takes: the carry out of the lane above.
        let moved = if above {
            // A `CARRY4` above whose bottom lane passes its carry in up:
            // propagate one, generate zero, and the other lanes nothing.
            let (Some(ci), init) = (shape.carry_in, shape.init) else {
                continue;
            };
            let mut s_parts = Vec::with_capacity(lanes);
            let mut di_parts = Vec::with_capacity(lanes);
            for _ in 1..lanes {
                s_parts.push(x(m));
                di_parts.push(x(m));
            }
            s_parts.push(one(m));
            di_parts.push(zero(m));
            let s = expr(m, ExprKind::Concat(s_parts), span);
            let di = expr(m, ExprKind::Concat(di_parts), span);
            let whole = net_expr(m, new, span);
            let top = slice_expr(m, whole, width - 1, width - 1, span);
            let mut inputs = vec![
                (Name::new(shape.propagate), s),
                (Name::new(shape.data), di),
                (Name::new(ci), top),
            ];
            if let Some(init) = init {
                inputs.push((Name::new(init), zero(m)));
            }
            let sum = add_net(m, &format!("{name}$cout$o"), Type::bits(width), span);
            let carries = add_net(m, &format!("{name}$cout$co"), Type::bits(width), span);
            add_cell(
                m,
                &format!("{name}$cout"),
                CellKind::Blackbox(Name::new(carry)),
                inputs,
                vec![
                    (Name::new(shape.sum), sum),
                    (Name::new(shape.carry_out), carries),
                ],
                span,
            );
            let whole = net_expr(m, carries, span);
            slice_expr(m, whole, 0, 0, span)
        } else {
            // The lane above, nothing reads, made the same pass-through.
            let (Some(old_s), Some(old_di)) = (
                m.cells[id].input(shape.propagate),
                m.cells[id].input(shape.data),
            ) else {
                continue;
            };
            let mut s_parts = Vec::with_capacity(lanes);
            let mut di_parts = Vec::with_capacity(lanes);
            for j in (0..width).rev() {
                if j == lane + 1 {
                    s_parts.push(one(m));
                    di_parts.push(zero(m));
                } else {
                    s_parts.push(slice_expr(m, old_s, j, j, span));
                    di_parts.push(slice_expr(m, old_di, j, j, span));
                }
            }
            let s = expr(m, ExprKind::Concat(s_parts), span);
            let di = expr(m, ExprKind::Concat(di_parts), span);
            for (port, value) in m.cells[id].inputs.iter_mut() {
                if port.as_str() == shape.propagate {
                    *value = s;
                } else if port.as_str() == shape.data {
                    *value = di;
                }
            }
            let whole = net_expr(m, new, span);
            slice_expr(m, whole, lane + 1, lane + 1, span)
        };
        // The old net, most significant first, the moved bit from above.
        let mut parts = Vec::with_capacity(lanes);
        for j in (0..width).rev() {
            if j == lane {
                parts.push(moved);
            } else {
                let whole = net_expr(m, new, span);
                parts.push(slice_expr(m, whole, j, j, span));
            }
        }
        let value = expr(m, ExprKind::Concat(parts), span);
        add_assign(m, old, value, span);
    }
    Ok(moves.len())
}

/// Gives each generate [`legalise_carries_with`] left on a `CARRY4`'s bypass
/// input ([`BYPASS_GENERATES`]) a signal of its own that nothing drives, and
/// returns how many it gave one.
///
/// The pin keeps its constant, which is what the flow ties the bypass input to
/// after routing (`XrayFabric::configure_block_rams`). The signal is what the
/// placer reads: a pin with a signal draws on the wires that can reach it,
/// and a signal that the lane's table does not drive can reach a generate by
/// the bypass input alone, so no flip-flop of the lane whose data comes from
/// elsewhere may be put on that input beside it, as for a generate from the
/// fabric. Each pin has its own signal, so a placer that pulls a signal's pins
/// together pulls nothing. Nothing routes a signal nothing drives, the
/// bitstream does not take the pin for a constant on `O5` (`<L>CY0` stays
/// clear, the bypass's own setting), and the Vivado export, which reads the
/// cell, writes the constant. Called after the netlist's own check that every
/// signal read is driven, so that afterwards a `CARRY4` generate whose signal
/// nothing drives is one of these and nothing else ([`bypass_tie`]).
pub fn tie_bypass_generates(design: &Design, module: ModuleId, netlist: &mut Netlist) -> usize {
    let Some(m) = design.modules.get(module) else {
        return 0;
    };
    let mut given = 0;
    for index in 0..netlist.instances.len() {
        let instance = &netlist.instances[index];
        if instance.primitive != "CARRY4" {
            continue;
        }
        let Some(lanes) = m
            .cells
            .get(instance.cell)
            .and_then(|cell| cell.attrs.get(BYPASS_GENERATES))
            .and_then(AttrValue::as_int)
        else {
            continue;
        };
        let name = instance.name.clone();
        let pins = instance.pins.clone();
        for lane in 0..4 {
            if lanes & (1 << lane) == 0 {
                continue;
            }
            let role = format!("di{lane}");
            let Some(pin) = pins.iter().copied().find(|p| {
                let pin = &netlist.pins[*p];
                pin.role == role
                    && pin.signal.is_none()
                    && matches!(pin.constant, Some(Bit::Zero | Bit::One))
            }) else {
                continue;
            };
            let signal = netlist.signals.len();
            netlist.signals.push(Signal {
                name: format!("{name}.DI[{lane}] on the bypass input"),
                driver: None,
                sinks: vec![pin],
            });
            netlist.pins[pin].signal = Some(signal);
            given += 1;
        }
    }
    given
}

/// The constant a `CARRY4` generate pin is tied to on its lane's bypass input,
/// when it is one [`tie_bypass_generates`] gave a signal nothing drives.
pub fn bypass_tie(netlist: &Netlist, pin: usize) -> Option<Bit> {
    let p = netlist.pins.get(pin)?;
    if !p.role.starts_with("di") || netlist.instances[p.instance].primitive != "CARRY4" {
        return None;
    }
    let signal = p.signal?;
    if netlist.signals[signal].driver.is_some() {
        return None;
    }
    p.constant.filter(|b| matches!(b, Bit::Zero | Bit::One))
}

fn bit_of(value: bool) -> Bit {
    if value { Bit::One } else { Bit::Zero }
}

/// The 64-bit truth table of a lane's own lookup table.
///
/// Index `i` is the output for `{I5..I0} == i`. The **upper half**
/// (`I5` = 1) is the propagate — a buffer of `I0` when `constant` is
/// `None`, else the constant — and is what `O6` reads with `A6` high. The
/// **lower half** is what `O5` reads whatever `A6` is, so it carries the
/// constant generate when there is one and repeats the upper half when
/// there is not, which makes the table independent of `I5` altogether.
///
/// Every half is independent of `I1`..`I4`, which are tied in the
/// netlist and read one in the silicon (an unrouted interconnect input of
/// this family reads `VCC_WIRE`; `ppips_int_l.db` records it as the
/// `default` source of every `IMUX`), so nothing depends on which.
fn lane_init(constant: Option<bool>, generate: Option<bool>) -> Const {
    let mut value = 0u64;
    for index in 0..64u64 {
        let upper = match constant {
            Some(v) => v,
            None => index & 1 == 1,
        };
        let bit = if index & 32 != 0 {
            upper
        } else {
            generate.unwrap_or(upper)
        };
        if bit {
            value |= 1 << index;
        }
    }
    Const::from_u64(value, 64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_buffer_reads_its_first_input_in_both_halves() {
        assert_eq!(
            lane_init(None, None),
            Const::from_u64(0xAAAA_AAAA_AAAA_AAAA, 64)
        );
    }

    #[test]
    fn a_constant_generate_lives_in_the_lower_half_only() {
        // O6 (upper, A6 high) is the buffer; O5 (lower) is the zero.
        assert_eq!(
            lane_init(None, Some(false)),
            Const::from_u64(0xAAAA_AAAA_0000_0000, 64)
        );
        assert_eq!(
            lane_init(Some(false), Some(true)),
            Const::from_u64(0x0000_0000_FFFF_FFFF, 64)
        );
        assert_eq!(lane_init(Some(true), None), Const::from_u64(u64::MAX, 64));
    }
}
