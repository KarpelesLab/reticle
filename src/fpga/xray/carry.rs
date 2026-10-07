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
//!   not routed reads one, so a constant zero has to come from `O5`.
//!
//! [`legalise_carries`] closes both, and changes no logic doing it:
//!
//! - a propagate bit driven by anything but a lookup table it can have to
//!   itself gets a **buffer** lookup table, `O = I0`, in front of it — which
//!   is what Vivado calls a route-through;
//! - a constant propagate bit gets a lookup table whose output is that
//!   constant;
//! - a constant generate bit makes its lane's lookup table one of these
//!   two built here, with the constant in the **lower half** of the truth
//!   table, which is what `O5` reads, and the function in the upper half,
//!   which is what `O6` reads with `A6` high. `I5` is tied to one in the
//!   netlist so the cell's own model reads the upper half too: the netlist
//!   says what the silicon does, and a simulator of it agrees.
//!
//! **A lane nothing reads is left alone**, and its two inputs become `x`.
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
use crate::fpga::primitives::{add_cell, add_net, const_expr, expr, net_expr, slice_expr};
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
}

impl CarryPacking {
    /// One line, for a report.
    pub fn to_text(&self) -> String {
        format!(
            "carry chains: {} CARRY4, {} propagate bit(s) on their own lookup table, \
             {} buffer and {} constant lookup table(s) added ({} feeding a constant \
             generate), {} lane(s) nothing reads\n",
            self.carries, self.kept, self.buffers, self.constants, self.generates, self.dead_lanes
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
/// A message when the netlist cannot be read bit by bit, or when a
/// `CARRY4` asks for something one slice cannot give: a carry in from the
/// fabric *and* a fabric generate on bit 0, which would both need the
/// slice's one `AX`.
pub fn legalise_carries(
    design: &mut Design,
    module: ModuleId,
    device: &Device,
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
    let Some(m) = design.modules.get(module) else {
        return Err("the module is not part of the design".to_owned());
    };

    // The plan is made against an immutable view, then applied.
    let mut plans: Vec<(CellId, Vec<Lane>)> = Vec::new();
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

        // Which bits anything reads, and who drives each bit.
        let mut read: HashSet<usize> = HashSet::new();
        let mut driver: HashMap<usize, (CellId, String)> = HashMap::new();
        for (id, cell) in m.cells.iter() {
            for (_, e) in &cell.inputs {
                for bit in bits(*e)? {
                    if let SigBit::Slot(s) = bit {
                        read.insert(s);
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
                }
            }
        }

        // How many propagate pins each bit feeds, over every chain: a
        // lookup table can be the propagate of one lane of one slice.
        let carries: Vec<CellId> = m
            .cells
            .iter()
            .filter(|(_, c)| matches!(&c.kind, CellKind::Blackbox(n) if n.as_str() == carry.name))
            .map(|(id, _)| id)
            .collect();
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
            for lane in 0..lanes {
                let live =
                    is_read(sums.get(lane)) || (lane..lanes).any(|j| is_read(carries_out.get(j)));
                if !live {
                    report.dead_lanes += 1;
                    plan.push(Lane {
                        propagate: Propagate::Dead,
                        generate: None,
                    });
                    continue;
                }
                let generate = match di_bits[lane] {
                    SigBit::Slot(_) => None,
                    SigBit::Const(b) => Some(b == Bit::One),
                };
                if generate.is_none() && lane == 0 && fabric_init {
                    return Err(format!(
                        "`{}` takes its carry in and bit 0's generate both from the fabric, \
                         and a slice has one `AX` for the two",
                        cell.name
                    ));
                }
                let propagate = match s_bits[lane] {
                    SigBit::Const(b) => Propagate::Constant(b == Bit::One),
                    SigBit::Slot(s) => {
                        let own_lut = driver.get(&s).is_some_and(|(d, port)| {
                            matches!(&m.cells[*d].kind, CellKind::Blackbox(n) if n.as_str() == lut.name)
                                && *port == lut_output
                        });
                        if own_lut && propagate_uses.get(&s) == Some(&1) && generate.is_none() {
                            Propagate::Keep
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
            plans.push((id, plan));
        }
    }

    // Apply.
    let Some(m) = design.modules.get_mut(module) else {
        return Err("the module is not part of the design".to_owned());
    };
    for (id, plan) in plans {
        let span = m.cells[id].span;
        let name = m.cells[id].name.as_str().to_owned();
        let view_owner: Vec<Option<(crate::ir::NetId, u32)>> = {
            let view = BitView::new(m).map_err(|e| e.to_string())?;
            plan.iter()
                .map(|lane| match lane.propagate {
                    Propagate::Buffer(s) => Some(view.owner(s)),
                    _ => None,
                })
                .collect()
        };
        let old_s = m.cells[id]
            .input(shape.propagate)
            .expect("checked in the plan");
        let old_di = m.cells[id].input(shape.data).expect("checked in the plan");
        let mut s_parts: Vec<ExprId> = Vec::with_capacity(plan.len());
        let mut di_parts: Vec<ExprId> = Vec::with_capacity(plan.len());
        for (lane, step) in plan.iter().enumerate() {
            let bit = u32::try_from(lane).unwrap_or(0);
            let x = || Const::from_bits(&[Bit::X]);
            let (s, di) = match step.propagate {
                Propagate::Dead => (const_expr(m, x(), span), const_expr(m, x(), span)),
                Propagate::Keep => (
                    slice_expr(m, old_s, bit, bit, span),
                    slice_expr(m, old_di, bit, bit, span),
                ),
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
                    let out = add_net(m, &format!("{name}$s{lane}"), Type::bit(), span);
                    let init = lane_init(constant, step.generate);
                    let split = step.generate.is_some_and(|d| {
                        // The halves differ unless the constant generate
                        // equals a constant propagate.
                        constant != Some(d)
                    });
                    let mut inputs: Vec<(Name, ExprId)> = Vec::with_capacity(6);
                    for (index, pin) in lut_inputs.iter().enumerate() {
                        let value = match (index, input) {
                            (0, Some(e)) => e,
                            (5, _) if split => const_expr(m, Const::ones(1), span),
                            _ => const_expr(m, Const::zero(1), span),
                        };
                        inputs.push((Name::new(pin.clone()), value));
                    }
                    let cell = add_cell(
                        m,
                        &format!("{name}$lut{lane}"),
                        CellKind::Blackbox(Name::new(lut.name.clone())),
                        inputs,
                        vec![(Name::new(lut_output.clone()), out)],
                        span,
                    );
                    m.cells[cell]
                        .params
                        .set(Name::new(init_name.clone()), AttrValue::Const(init));
                    let di = match step.generate {
                        Some(d) => const_expr(m, Const::from_bits(&[bit_of(d)]), span),
                        None => slice_expr(m, old_di, bit, bit, span),
                    };
                    (net_expr(m, out, span), di)
                }
            };
            if step.propagate == Propagate::Keep {
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
    }
    Ok(report)
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
