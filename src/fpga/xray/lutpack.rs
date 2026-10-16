//! Two functions in one 7-series lookup table.
//!
//! A slice's six-input lookup table is two five-input ones that share their
//! inputs: with `A6` high, `O6` reads the upper half of the 64-bit truth
//! table, and `O5` reads the lower half whatever `A6` is (UG474, the
//! `LUT6_2` primitive). Mapping writes one function per lookup table, and
//! NewAE's ChipWhisperer-Husky gateware mapped that way is 35 191 of them
//! on a die that has 32 600 — of which 13 000 use four inputs or fewer.
//!
//! [`pack_lut_pairs`] puts two such functions into one `LUT6_2` when their
//! inputs number five or fewer between them: the first on `O6`, the
//! second on `O5`, `INIT = {first, second}` over the shared inputs, and
//! `I5` tied to one — which is what an unrouted `A6` reads on this fabric
//! (the database's `IMUX` default is `VCC_WIRE`), so the netlist and the
//! silicon agree on it without a route.
//!
//! Left alone: a lookup table using all six inputs, one that drives a
//! carry chain's propagate or generate (the chain needs the table at its
//! own position, and its generate tables already use `O5`), and one marked
//! `keep`.

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::fpga::device::{BelRole, Device};
use crate::fpga::primitives::{add_cell, const_expr};
use crate::ir::emit::{BitView, SigBit};
use crate::ir::{AttrValue, Bit, CellId, CellKind, Const, Design, ExprId, ModuleId, NetId};

/// The primitive this pass emits; a device file without it is left alone.
pub(super) const DUAL: &str = "LUT6_2";

/// What [`pack_lut_pairs`] did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LutPairing {
    /// Lookup tables looked at.
    pub luts: usize,
    /// Of those, the ones that could share a site.
    pub candidates: usize,
    /// Pairs packed into one `LUT6_2` each: the sites saved.
    pub pairs: usize,
}

impl LutPairing {
    /// One line, for a report.
    #[must_use]
    pub fn to_text(&self) -> String {
        if self.luts == 0 {
            return String::new();
        }
        format!(
            "note: lookup tables: {} of {} share a site with another (`{DUAL}`), saving {} \
             site(s)\n",
            2 * self.pairs,
            self.luts,
            self.pairs
        )
    }
}

/// One lookup table as the pass sees it.
struct Table {
    cell: CellId,
    /// The distinct signals it reads, in input order.
    inputs: Vec<SigBit>,
    /// An expression for each of them, to wire the packed cell with.
    exprs: Vec<ExprId>,
    /// Its function over `inputs`: bit `i` is the output when input `j`
    /// is bit `j` of `i`.
    truth: Vec<bool>,
    output: NetId,
}

/// Packs pairs of lookup tables into `LUT6_2` cells; see the module docs.
///
/// # Errors
///
/// A message when the module cannot be read bit by bit.
pub fn pack_lut_pairs(
    design: &mut Design,
    module: ModuleId,
    device: &Device,
) -> Result<LutPairing, String> {
    let mut report = LutPairing::default();
    let (Some(dual), Some(lut)) = (
        device.bels.iter().find(|b| b.name == DUAL).cloned(),
        device.bel(BelRole::Lut).cloned(),
    ) else {
        return Ok(report);
    };
    let inputs: Vec<String> = lut.port_names("i").into_iter().map(str::to_owned).collect();
    let dual_inputs: Vec<String> = dual
        .port_names("i")
        .into_iter()
        .map(str::to_owned)
        .collect();
    let (Some(out), Some(o6), Some(o5)) = (lut.port("o"), dual.port("o"), dual.port("o5")) else {
        return Ok(report);
    };
    if inputs.len() != 6 || dual_inputs.len() != 6 {
        return Ok(report);
    }
    let (out, o6, o5) = (out.to_owned(), o6.to_owned(), o5.to_owned());
    let Some(m) = design.modules.get(module) else {
        return Err("the module is not part of the design".to_owned());
    };

    let mut tables: Vec<Table> = Vec::new();
    {
        let view = BitView::new(m).map_err(|e| e.to_string())?;
        let bit = |e: ExprId| -> Option<SigBit> {
            let bits = view.expr_bits(e).ok()?;
            match bits.as_slice() {
                [b] => Some(view.canonical(*b)),
                _ => None,
            }
        };
        // What the carry chains read through their propagate and generate:
        // those tables stay where the chain needs them.
        let mut chained: HashSet<SigBit> = HashSet::new();
        for (_, cell) in m.cells.iter() {
            if !matches!(&cell.kind, CellKind::Blackbox(n) if device
                .bel(BelRole::Carry)
                .is_some_and(|c| c.name == n.as_str()))
            {
                continue;
            }
            for (_, e) in &cell.inputs {
                for b in view.expr_bits(*e).map_err(|e| e.to_string())? {
                    chained.insert(view.canonical(b));
                }
            }
        }
        for (id, cell) in m.cells.iter() {
            if !matches!(&cell.kind, CellKind::Blackbox(n) if n.as_str() == lut.name) {
                continue;
            }
            report.luts += 1;
            if crate::synth::keep::cell_is_kept(m, cell) {
                continue;
            }
            let Some(output) = cell.output(&out) else {
                continue;
            };
            let Ok(out_bits) = view.net_bits(output, cell.span) else {
                continue;
            };
            if out_bits
                .iter()
                .any(|b| chained.contains(&view.canonical(*b)))
            {
                continue;
            }
            let Some(AttrValue::Const(init)) = cell.params.get("INIT") else {
                continue;
            };
            // Each pin: a signal, or a constant the truth table sees.
            let mut pins: Vec<SigBit> = Vec::with_capacity(6);
            let mut pin_exprs: Vec<Option<ExprId>> = Vec::with_capacity(6);
            for name in &inputs {
                match cell.input(name) {
                    Some(e) => {
                        let Some(b) = bit(e) else { break };
                        pins.push(b);
                        pin_exprs.push(Some(e));
                    }
                    None => {
                        pins.push(SigBit::Const(Bit::Zero));
                        pin_exprs.push(None);
                    }
                }
            }
            if pins.len() != 6
                || pins
                    .iter()
                    .any(|p| matches!(p, SigBit::Const(b) if *b != Bit::Zero && *b != Bit::One))
            {
                continue;
            }
            let mut signals: Vec<SigBit> = Vec::new();
            let mut exprs: Vec<ExprId> = Vec::new();
            for (pin, e) in pins.iter().zip(&pin_exprs) {
                if let (SigBit::Slot(_), Some(e)) = (pin, e)
                    && !signals.contains(pin)
                {
                    signals.push(*pin);
                    exprs.push(*e);
                }
            }
            if signals.len() > 5 {
                continue;
            }
            // The function over the distinct signals, the constants
            // folded in as the netlist gives them.
            let mut truth = vec![false; 1 << signals.len()];
            for (i, slot) in truth.iter_mut().enumerate() {
                let mut index = 0usize;
                for (j, pin) in pins.iter().enumerate() {
                    let value = match pin {
                        SigBit::Const(b) => *b == Bit::One,
                        SigBit::Slot(_) => {
                            let k = signals.iter().position(|s| s == pin).unwrap_or(0);
                            (i >> k) & 1 == 1
                        }
                    };
                    if value {
                        index |= 1 << j;
                    }
                }
                *slot = init.bit(u32::try_from(index).unwrap_or(0)) == Bit::One;
            }
            report.candidates += 1;
            tables.push(Table {
                cell: id,
                inputs: signals,
                exprs,
                truth,
                output,
            });
        }
    }

    // Pairs, greedily: the widest tables first, each with the partner that
    // shares most of its inputs; a table no partner shares anything with
    // takes the narrowest one left.
    let mut by_signal: HashMap<SigBit, Vec<usize>> = HashMap::new();
    for (i, t) in tables.iter().enumerate() {
        for s in &t.inputs {
            by_signal.entry(*s).or_default().push(i);
        }
    }
    let mut order: Vec<usize> = (0..tables.len()).collect();
    order.sort_by_key(|i| (std::cmp::Reverse(tables[*i].inputs.len()), *i));
    let mut narrow: BTreeMap<(usize, usize), ()> = BTreeMap::new();
    for (i, t) in tables.iter().enumerate() {
        narrow.insert((t.inputs.len(), i), ());
    }
    let mut taken = vec![false; tables.len()];
    let mut pairs: Vec<(usize, usize)> = Vec::new();
    let union = |a: &Table, b: &Table| -> usize {
        a.inputs.len() + b.inputs.iter().filter(|s| !a.inputs.contains(s)).count()
    };
    for &i in &order {
        if taken[i] {
            continue;
        }
        taken[i] = true;
        narrow.remove(&(tables[i].inputs.len(), i));
        let mut best: Option<(usize, usize)> = None;
        for s in &tables[i].inputs {
            for &j in by_signal.get(s).into_iter().flatten().take(64) {
                if taken[j] || union(&tables[i], &tables[j]) > 5 {
                    continue;
                }
                let shared = tables[j]
                    .inputs
                    .iter()
                    .filter(|x| tables[i].inputs.contains(x))
                    .count();
                if best.is_none_or(|(_, b)| shared > b) {
                    best = Some((j, shared));
                }
            }
        }
        let partner = best.map(|(j, _)| j).or_else(|| {
            narrow
                .keys()
                .take(8)
                .map(|(_, j)| *j)
                .find(|j| union(&tables[i], &tables[*j]) <= 5)
        });
        if let Some(j) = partner {
            taken[j] = true;
            narrow.remove(&(tables[j].inputs.len(), j));
            pairs.push((i, j));
        } else {
            // Nothing fits beside it; it stays a table of its own.
            taken[i] = true;
        }
    }

    // The packed cells.
    let m = &mut design.modules[module];
    let mut removed: Vec<CellId> = Vec::new();
    for (i, j) in pairs {
        let (a, b) = (&tables[i], &tables[j]);
        let mut signals = a.inputs.clone();
        let mut exprs = a.exprs.clone();
        for (s, e) in b.inputs.iter().zip(&b.exprs) {
            if !signals.contains(s) {
                signals.push(*s);
                exprs.push(*e);
            }
        }
        // Over the shared inputs: bit `x` of a half is the function at the
        // assignment `x` gives them.
        let half = |t: &Table| -> Vec<bool> {
            (0..32usize)
                .map(|x| {
                    let mut i = 0usize;
                    for (k, s) in t.inputs.iter().enumerate() {
                        let at = signals.iter().position(|v| v == s).unwrap_or(0);
                        if (x >> at) & 1 == 1 {
                            i |= 1 << k;
                        }
                    }
                    t.truth[i]
                })
                .collect()
        };
        let (upper, lower) = (half(a), half(b));
        let mut init = Const::zero(64);
        for x in 0..32u32 {
            if upper[x as usize] {
                init.set_bit(32 + x, Bit::One);
            }
            if lower[x as usize] {
                init.set_bit(x, Bit::One);
            }
        }
        let span = m.cells[a.cell].span;
        let mut cell_inputs: Vec<(crate::ir::Name, ExprId)> = Vec::with_capacity(6);
        for (k, name) in dual_inputs.iter().enumerate() {
            let e = match exprs.get(k) {
                Some(e) if k < 5 => *e,
                // `I5` is `A6`, high so that `O6` reads the upper half; an
                // unused input below it is a zero the halves ignore.
                _ if k == 5 => const_expr(m, Const::ones(1), span),
                _ => const_expr(m, Const::zero(1), span),
            };
            cell_inputs.push((crate::ir::Name::new(name.as_str()), e));
        }
        let name = format!("{}$dual", m.cells[a.cell].name);
        let cell = add_cell(
            m,
            &name,
            CellKind::Blackbox(crate::ir::Name::new(DUAL)),
            cell_inputs,
            vec![
                (crate::ir::Name::new(o6.as_str()), a.output),
                (crate::ir::Name::new(o5.as_str()), b.output),
            ],
            span,
        );
        m.cells[cell].params.set("INIT", AttrValue::Const(init));
        removed.push(a.cell);
        removed.push(b.cell);
        report.pairs += 1;
    }
    if !removed.is_empty() {
        m.cells.retain(|id, _| !removed.contains(&id));
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fpga::target;
    use crate::ir::builder::ModuleBuilder;
    use crate::ir::{Name, Type};
    use crate::source::{SourceMap, Span};

    /// Evaluates every lookup table of `m`, single or dual, on one
    /// assignment of the module's inputs, and answers each output port.
    fn eval(m: &crate::ir::Module, inputs: &[(NetId, bool)]) -> Vec<bool> {
        let mut value: HashMap<NetId, bool> = inputs.iter().copied().collect();
        let read = |e: ExprId, value: &HashMap<NetId, bool>| -> Option<bool> {
            let expr = m.expr(e);
            match &expr.kind {
                crate::ir::ExprKind::Net(n) => value.get(n).copied(),
                crate::ir::ExprKind::Const(c) => Some(c.bit(0) == Bit::One),
                _ => None,
            }
        };
        // Tables in any order: repeat until nothing changes.
        for _ in 0..8 {
            for (_, cell) in m.cells.iter() {
                let CellKind::Blackbox(name) = &cell.kind else {
                    continue;
                };
                let Some(AttrValue::Const(init)) = cell.params.get("INIT") else {
                    continue;
                };
                let mut index = 0u32;
                let mut known = true;
                for (k, port) in ["I0", "I1", "I2", "I3", "I4", "I5"].iter().enumerate() {
                    match cell.input(port).and_then(|e| read(e, &value)) {
                        Some(true) => index |= 1 << k,
                        Some(false) => {}
                        None => known = false,
                    }
                }
                if !known {
                    continue;
                }
                if name.as_str() == DUAL {
                    value.insert(cell.output("O6").unwrap(), init.bit(index) == Bit::One);
                    value.insert(cell.output("O5").unwrap(), init.bit(index & 31) == Bit::One);
                } else {
                    value.insert(cell.output("O").unwrap(), init.bit(index) == Bit::One);
                }
            }
        }
        m.ports
            .iter()
            .filter(|p| p.dir == crate::ir::PortDir::Out)
            .map(|p| value.get(&p.net).copied().unwrap_or(false))
            .collect()
    }

    /// Six tables over five inputs, with constants on some pins as mapping
    /// leaves them, packed: every output agrees with the unpacked module on
    /// all 32 assignments, and the tables did pair.
    ///
    /// Would catch a half written to the wrong end of `INIT`, an input
    /// order that differs between the two halves, a constant folded the
    /// wrong way, and `I5` not tied high. Would not catch `O5` reading
    /// something other than the lower half on silicon, which is UG474's
    /// statement, quoted.
    #[test]
    fn two_tables_in_one_answer_as_they_did_apart() {
        let mut map = SourceMap::new();
        let file = map.add("t", "").unwrap();
        let mut b = ModuleBuilder::new("pairs", Span::new(file, 0, 0));
        let ins: Vec<NetId> = (0..5)
            .map(|i| b.input(format!("x{i}"), Type::bit()))
            .collect();
        // Each table: its INIT and which input (or constant) each pin reads.
        let specs: [(u64, [i32; 6]); 6] = [
            (0x6996_9669_9669_6996, [0, 1, 2, -1, -1, -1]),
            (0xFE01_FE01_FE01_FE01, [3, 4, -1, -1, -1, -1]),
            (0x8000_0000_8000_0000, [0, 2, 4, 1, 3, -1]),
            (0x1234_5678_9ABC_DEF0, [4, 3, 2, 1, 0, -1]),
            (0x0F0F_F0F0_3C3C_C3C3, [1, -2, 3, -1, 0, -1]),
            (0xAAAA_5555_CCCC_3333, [2, 0, -1, -2, -1, -1]),
        ];
        for (n, (_, pins)) in specs.iter().enumerate() {
            let y = b.output(format!("y{n}"), Type::bit());
            let mut inputs = Vec::new();
            for (k, pin) in pins.iter().enumerate() {
                let e = match pin {
                    -1 => b.const_bit(false),
                    -2 => b.const_bit(true),
                    i => b.net(ins[usize::try_from(*i).unwrap()]),
                };
                inputs.push((Name::new(format!("I{k}")), e));
            }
            b.cell(
                format!("t{n}"),
                CellKind::Blackbox(Name::new("LUT6")),
                inputs,
                vec![(Name::new("O"), y)],
            );
        }
        let mut module = b.finish();
        for (id, cell) in module.cells.iter_mut() {
            let n = id.index();
            cell.params
                .set("INIT", AttrValue::Const(Const::from_u64(specs[n].0, 64)));
        }
        let mut design = Design::new();
        let top = design.add_module(module);
        design.top = Some(top);
        let before = design.module(top).clone();

        let device = target("xc7a35t-ftg256").unwrap();
        let report = pack_lut_pairs(&mut design, top, device).unwrap();
        assert!(report.pairs >= 2, "{report:?}");
        let after = design.module(top);
        assert!(
            after
                .cells
                .iter()
                .filter(|(_, c)| matches!(&c.kind, CellKind::Blackbox(n) if n.as_str() == DUAL))
                .all(|(_, c)| {
                    c.input("I5")
                        .and_then(|e| after.expr(e).as_const().cloned())
                        .is_some_and(|k| k.bit(0) == Bit::One)
                }),
            "a `LUT6_2` whose `I5` is not tied high"
        );
        for assignment in 0..32u32 {
            let values: Vec<(NetId, bool)> = ins
                .iter()
                .enumerate()
                .map(|(i, n)| (*n, (assignment >> i) & 1 == 1))
                .collect();
            assert_eq!(
                eval(after, &values),
                eval(&before, &values),
                "assignment {assignment:05b}"
            );
        }
    }
}
