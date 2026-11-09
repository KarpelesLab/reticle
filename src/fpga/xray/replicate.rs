//! Copies of a flip-flop that drives a great many lookup tables, as Vivado's
//! synthesis makes them.
//!
//! **Measured** (Vivado 2026.1, `synth_design -mode out_of_context`,
//! `xc7a100tftg256-3`, docs/fpga-vivado-parity.md, "A wide fanout copied"):
//! a register selecting between two registers of `N` bits, so that it drives
//! `N` tables, is one flip-flop to `N` = 220 and copied from 222; the same at
//! clock periods of 1, 2, 5 and 100 ns, so it is not timing-driven. The
//! timing corpus's `widemux` has two such, the select's top two bits, each
//! read by 256 tables, and each is two flip-flops in Vivado's netlist, the
//! readers split by the output register they lead to: `q[0]` to `q[7]` on
//! one, `q[8]` to `q[15]` on the other. How many copies Vivado makes past
//! that is not one rule: 255 to 385 readers were three, 448 to 513 five, and
//! the shares were 108 and 144 readers in shapes this module does not try to
//! reproduce. [`copies`] is one flip-flop for each [`SHARE`] readers, which is
//! Vivado's count for `widemux` (257 readers, two) and from 222 to 250 and 300
//! to 385 readers.
//!
//! **Which readers share a copy.** Each reader is followed forward through
//! the lookup tables and wide multiplexers after it, a few cells deep, to the
//! first flip-flop it leads to; readers are ordered by that flip-flop's name,
//! its bit number read as a number, and cut into runs of equal size, a reader
//! group never split. The original keeps the first run.
//!
//! **Which flip-flops are copied.** One with more than [`MOST_READERS`]
//! readers, none of them a clock pin. Not one that is kept, or marked
//! `ASYNC_REG`, nor one whose data comes from a pad or a flip-flop on another
//! clock: two copies of a synchroniser's first stage could resolve a
//! metastable input differently.
//!
//! A copy is the same cell with the same parameters (its `INIT` among them)
//! and the same inputs, so it holds what the original holds on every cycle;
//! only readers whose input is that one bit move to it.

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::fpga::primitives::{add_net, net_expr};
use crate::ir::emit::{BitView, SigBit};
use crate::ir::{CellId, CellKind, ExprId, Module, Name, NetId, Type};

/// The most readers a flip-flop has and stays one: Vivado copies one read by
/// 222 tables and not one read by 220 (221 was not asked).
pub const MOST_READERS: usize = 220;

/// The readers each copy is given, at most: fitted, not read off a rule.
/// `widemux`'s select bits are each read by 256 tables and a flip-flop, and
/// Vivado made two of each; 222 to 250 readers were two, 300 to 385 three.
pub const SHARE: usize = 129;

/// How far forward a reader is followed to find the flip-flop it leads to.
const DEPTH: usize = 4;

/// What [`replicate_wide_fanout`] did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Replication {
    /// Flip-flops copied.
    pub registers: usize,
    /// Copies made.
    pub copies: usize,
    /// Reader pins moved onto a copy.
    pub moved: usize,
}

/// How many flip-flops, the original among them, `readers` readers are
/// shared over.
pub fn copies(readers: usize) -> usize {
    if readers > MOST_READERS {
        readers.div_ceil(SHARE)
    } else {
        1
    }
}

/// The flip-flop primitives.
fn is_flop(kind: &CellKind) -> bool {
    matches!(kind, CellKind::Blackbox(n) if matches!(
        n.as_str(),
        "FDRE" | "FDSE" | "FDCE" | "FDPE" | "FDRE_1" | "FDSE_1" | "FDCE_1" | "FDPE_1"
    ))
}

/// A name ordered as its text, with each run of digits read as a number, so
/// that `q$ff$ff10` comes after `q$ff$ff9`.
fn natural(name: &str) -> Vec<(String, u64)> {
    let mut out = Vec::new();
    let mut text = String::new();
    let mut digits = String::new();
    for ch in name.chars() {
        if ch.is_ascii_digit() {
            digits.push(ch);
        } else {
            if !digits.is_empty() {
                out.push((
                    std::mem::take(&mut text),
                    digits.parse().unwrap_or(u64::MAX),
                ));
                digits.clear();
            }
            text.push(ch);
        }
    }
    let number = if digits.is_empty() {
        0
    } else {
        digits.parse().unwrap_or(u64::MAX)
    };
    out.push((text, number));
    out
}

/// A reader pin: the cell and its input port.
type Pin = (CellId, Name);

/// Copies the flip-flops of `module` that drive more than [`MOST_READERS`]
/// cells; see the module documentation.
///
/// # Errors
///
/// A message when the module's assignments do not resolve to bits.
pub fn replicate_wide_fanout(module: &mut Module) -> Result<Replication, String> {
    let mut report = Replication::default();
    let plan: Vec<(CellId, NetId, Vec<Vec<Pin>>)> = {
        let m = &*module;
        let view = BitView::new(m).map_err(|e| e.to_string())?;
        // Every cell input bit's readers: (cell, port, the port is that bit).
        let mut readers: HashMap<SigBit, Vec<(CellId, Name, bool)>> = HashMap::new();
        let mut driver: HashMap<SigBit, CellId> = HashMap::new();
        let mut outputs: HashMap<CellId, Vec<SigBit>> = HashMap::new();
        for (id, cell) in m.cells.iter() {
            // An input this view cannot read bit by bit (a primitive's pin
            // still written `~clk` until the device cells say otherwise) moves
            // nothing and is not counted: what it reads stays on the original.
            for (port, e) in &cell.inputs {
                let Ok(bits) = view.expr_bits(*e) else {
                    continue;
                };
                let single = bits.len() == 1;
                for b in bits {
                    let b = view.canonical(b);
                    if matches!(b, SigBit::Slot(_)) {
                        readers
                            .entry(b)
                            .or_default()
                            .push((id, port.clone(), single));
                    }
                }
            }
            for (_, net) in &cell.outputs {
                let Ok(bits) = view.net_bits(*net, cell.span) else {
                    continue;
                };
                for b in &bits {
                    driver.insert(view.canonical(*b), id);
                }
                outputs
                    .entry(id)
                    .or_default()
                    .extend(bits.iter().map(|b| view.canonical(*b)));
            }
        }
        let bit_of = |e: ExprId| -> Option<SigBit> {
            match view.expr_bits(e).ok()?.as_slice() {
                [b] => Some(view.canonical(*b)),
                _ => None,
            }
        };
        // The flip-flop a reader leads to: itself if it is one, else the
        // first reached forward, the least by name of those at one depth.
        let leads_to = |start: CellId| -> Option<CellId> {
            let mut frontier = vec![start];
            let mut seen: HashSet<CellId> = HashSet::new();
            for _ in 0..DEPTH {
                let mut found: Vec<CellId> = Vec::new();
                let mut next = Vec::new();
                for id in frontier {
                    if !seen.insert(id) {
                        continue;
                    }
                    if is_flop(&m.cells[id].kind) {
                        found.push(id);
                        continue;
                    }
                    for b in outputs.get(&id).into_iter().flatten() {
                        for (r, _, _) in readers.get(b).into_iter().flatten() {
                            next.push(*r);
                        }
                    }
                }
                if let Some(first) = found
                    .into_iter()
                    .min_by_key(|id| natural(m.cells[*id].name.as_str()))
                {
                    return Some(first);
                }
                next.sort_unstable();
                next.dedup();
                frontier = next;
            }
            None
        };
        let mut plan = Vec::new();
        for (id, cell) in m.cells.iter() {
            if !is_flop(&cell.kind)
                || crate::synth::keep::cell_is_kept(m, cell)
                || cell.attrs.contains("ASYNC_REG")
                || cell.attrs.contains("async_reg")
            {
                continue;
            }
            let (Some(q), Some(d), Some(c)) = (cell.output("Q"), cell.input("D"), cell.input("C"))
            else {
                continue;
            };
            let Some(q_bit) = view
                .net_bits(q, cell.span)
                .ok()
                .and_then(|b| (b.len() == 1).then(|| view.canonical(b[0])))
            else {
                continue;
            };
            let Some(sinks) = readers.get(&q_bit) else {
                continue;
            };
            let count = copies(sinks.len());
            if count < 2
                || sinks
                    .iter()
                    .any(|(_, port, _)| port.as_str() == "C" || port.as_str().contains("CLK"))
            {
                continue;
            }
            let clock = bit_of(c);
            if let Some(d_bit) = bit_of(d)
                && let Some(from) = driver.get(&d_bit)
            {
                let source = &m.cells[*from];
                if matches!(&source.kind, CellKind::Blackbox(n) if n.as_str().starts_with("IBUF")) {
                    continue;
                }
                if is_flop(&source.kind) && source.input("C").and_then(bit_of) != clock {
                    continue;
                }
            }
            // The readers that can move, grouped by the flip-flop each leads
            // to, the groups in its order.
            let mut groups: BTreeMap<Vec<(String, u64)>, Vec<Pin>> = BTreeMap::new();
            for (reader, port, single) in sinks {
                if !*single {
                    continue;
                }
                let key = leads_to(*reader).map_or_else(
                    || natural(m.cells[*reader].name.as_str()),
                    |f| natural(m.cells[f].name.as_str()),
                );
                groups.entry(key).or_default().push((*reader, port.clone()));
            }
            let movable: usize = groups.values().map(Vec::len).sum();
            let share = movable.div_ceil(count).max(1);
            let mut runs: Vec<Vec<Pin>> = vec![Vec::new()];
            for pins in groups.into_values() {
                if runs.last().is_some_and(|r| r.len() >= share) && runs.len() < count {
                    runs.push(Vec::new());
                }
                if let Some(run) = runs.last_mut() {
                    run.extend(pins);
                }
            }
            // The original keeps the first run.
            let moved: Vec<Vec<Pin>> = runs.into_iter().skip(1).filter(|r| !r.is_empty()).collect();
            if !moved.is_empty() {
                plan.push((id, q, moved));
            }
        }
        plan
    };
    for (flop, q, runs) in plan {
        report.registers += 1;
        let template = module.cells[flop].clone();
        let ty: Type = module.nets[q].ty.clone();
        for (k, pins) in runs.into_iter().enumerate() {
            let net_label = format!("{}$rep{k}", module.nets[q].name.as_str());
            let net = add_net(module, &net_label, ty.clone(), template.span);
            let mut copy = template.clone();
            copy.name = Name::new(format!("{}$rep{k}", template.name.as_str()));
            for (port, out) in copy.outputs.iter_mut() {
                if port.as_str() == "Q" {
                    *out = net;
                }
            }
            module.cells.push(copy);
            let read = net_expr(module, net, template.span);
            for (reader, port) in pins {
                for (p, e) in module.cells[reader].inputs.iter_mut() {
                    if *p == port {
                        *e = read;
                        report.moved += 1;
                    }
                }
            }
            report.copies += 1;
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The count against what Vivado made, where the rule is meant to agree:
    /// one flip-flop to 220 readers, two from 222 to 250 and for `widemux`'s
    /// 257, three from 300 to 384. Would catch the threshold or the share
    /// moved. (Vivado's three at 255 to 257 and five at 448 to 513 are where
    /// it does not agree, and the module says so.)
    #[test]
    fn copies_as_vivado_counts_them() {
        for (readers, want) in [
            (200, 1),
            (220, 1),
            (222, 2),
            (240, 2),
            (250, 2),
            (257, 2),
            (300, 3),
            (340, 3),
            (384, 3),
        ] {
            assert_eq!(copies(readers), want, "{readers} readers");
        }
    }

    use crate::ir::builder::ModuleBuilder;
    use crate::source::{SourceMap, Span};

    fn span() -> Span {
        let mut map = SourceMap::new();
        let id = map.add("replicate", "").unwrap();
        Span::new(id, 0, 0)
    }

    /// One flip-flop primitive.
    fn fdre(b: &mut ModuleBuilder, name: &str, c: ExprId, d: ExprId, q: NetId) -> CellId {
        let ce = b.const_bit(true);
        let r = b.const_bit(false);
        b.cell(
            name,
            CellKind::Blackbox(Name::new("FDRE")),
            vec![
                (Name::new("C"), c),
                (Name::new("CE"), ce),
                (Name::new("R"), r),
                (Name::new("D"), d),
            ],
            vec![(Name::new("Q"), q)],
        )
    }

    /// A select flip-flop `sel_ff` read by `n` tables, each loading a
    /// flip-flop `q_ff<i>` of its own, as `widemux`'s select:
    /// `q<i> <= LUT2(sel, d<i>)`. The select's data comes from `from` (an
    /// `FDRE` on the same clock, or an `IBUF`).
    fn select(n: usize, from: &str, async_reg: bool) -> Module {
        let mut b = ModuleBuilder::new("t", span());
        let clk = b.input("clk", Type::bit());
        let si = b.input("si", Type::bit());
        let c = b.net(clk);
        let src = b.add_net("src", Type::bit());
        let si_e = b.net(si);
        if from == "IBUF" {
            b.cell(
                "buf",
                CellKind::Blackbox(Name::new("IBUF")),
                vec![(Name::new("I"), si_e)],
                vec![(Name::new("O"), src)],
            );
        } else {
            fdre(&mut b, "first", c, si_e, src);
        }
        let sel = b.add_net("sel", Type::bit());
        let d = b.net(src);
        let flop = fdre(&mut b, "sel_ff", c, d, sel);
        b.module_mut().cells[flop].params.set("INIT", 1i64);
        if async_reg {
            b.module_mut().cells[flop].attrs.set("ASYNC_REG", "TRUE");
        }
        for i in 0..n {
            let t = b.add_net(format!("t{i}"), Type::bit());
            let q = b.output(format!("q{i}"), Type::bit());
            let s_e = b.net(sel);
            let data = b.input(format!("d{i}"), Type::bit());
            let d_e = b.net(data);
            b.cell(
                format!("lut{i}"),
                CellKind::Blackbox(Name::new("LUT2")),
                vec![(Name::new("I0"), s_e), (Name::new("I1"), d_e)],
                vec![(Name::new("O"), t)],
            );
            let t_e = b.net(t);
            fdre(&mut b, &format!("q_ff{i}"), c, t_e, q);
        }
        b.finish()
    }

    /// Each flip-flop whose name starts `sel_ff`, and the tables reading its
    /// output, by table number.
    fn readers_of_copies(m: &Module) -> Vec<(String, Vec<usize>)> {
        let mut out = Vec::new();
        for (_, cell) in m.cells.iter() {
            if !cell.name.as_str().starts_with("sel_ff") {
                continue;
            }
            let Some(q) = cell.output("Q") else {
                continue;
            };
            let mut tables: Vec<usize> = m
                .cells
                .iter()
                .filter(|(_, c)| {
                    c.input("I0")
                        .is_some_and(|e| m.exprs[e].as_net() == Some(q))
                })
                .filter_map(|(_, c)| c.name.as_str().strip_prefix("lut")?.parse().ok())
                .collect();
            tables.sort_unstable();
            out.push((cell.name.as_str().to_owned(), tables));
        }
        out.sort();
        out
    }

    /// **A copy is its original again, and every reader reads one of them.**
    /// The select read by 230 tables is two flip-flops, the same cell with the
    /// same inputs (the same expressions) and parameters (`INIT` among them),
    /// so that both hold the same value on every cycle; the tables are split
    /// by the flip-flop each leads to, `q_ff0` to `q_ff114` on the original and
    /// `q_ff115` to `q_ff229` on the copy, in their numbers' order, none lost
    /// or read twice. Would catch a copy with an input or a parameter of its
    /// own, a reader dropped or left on a net nothing drives, the readers
    /// ordered as text (`q_ff100` before `q_ff11`), and the split uneven.
    #[test]
    fn a_copy_is_its_original_and_holds_half_the_readers() {
        let mut m = select(230, "FDRE", false);
        let report = replicate_wide_fanout(&mut m).expect("replicated");
        assert_eq!(
            report,
            Replication {
                registers: 1,
                copies: 1,
                moved: 115
            }
        );
        let flops: Vec<_> = m
            .cells
            .iter()
            .filter(|(_, c)| c.name.as_str().starts_with("sel_ff"))
            .map(|(_, c)| c.clone())
            .collect();
        assert_eq!(flops.len(), 2);
        assert_eq!(flops[0].inputs, flops[1].inputs, "the copy's inputs");
        assert_eq!(flops[0].params, flops[1].params, "the copy's parameters");
        assert_eq!(flops[0].kind, flops[1].kind);
        let readers = readers_of_copies(&m);
        assert_eq!(readers[0].1, (0..115).collect::<Vec<_>>(), "{readers:?}");
        assert_eq!(readers[1].1, (115..230).collect::<Vec<_>>(), "{readers:?}");
    }

    /// **What is not copied**: a flip-flop read by 220 tables, Vivado's most
    /// on one; one whose data comes from a pad; and one marked `ASYNC_REG`.
    /// Read by 300, three. Would catch the threshold moved down, or a
    /// synchroniser's first stage copied, which two copies could resolve
    /// differently.
    #[test]
    fn a_synchroniser_and_a_narrower_fanout_stay_one() {
        for (n, from, async_reg) in [
            (220, "FDRE", false),
            (300, "IBUF", false),
            (300, "FDRE", true),
        ] {
            let mut m = select(n, from, async_reg);
            let report = replicate_wide_fanout(&mut m).expect("replicated");
            assert_eq!(report, Replication::default(), "{n} {from} {async_reg}");
        }
        let mut m = select(300, "FDRE", false);
        let report = replicate_wide_fanout(&mut m).expect("replicated");
        assert_eq!((report.registers, report.copies), (1, 2), "{report:?}");
    }

    /// **A pin this pass cannot read bit by bit is no error, and stays.** A
    /// falling-edge flip-flop's clock reaches it as `~clk` before the device
    /// cells say otherwise; one more reader of the select through `~sel`
    /// stays on the original, and the rest is copied as before. Would catch
    /// such a pin refusing the whole flow, which `tests/fpga_dsp_vivado.rs`'s
    /// falling-edge designs did.
    #[test]
    fn an_inverted_pin_is_left_where_it_is() {
        let m = select(230, "FDRE", false);
        let sel = m
            .nets
            .iter()
            .find(|(_, n)| n.name.as_str() == "sel")
            .map(|(id, _)| id);
        let mut b = ModuleBuilder::from_module(m, span());
        let s_e = b.net(sel.expect("`sel`"));
        let inverted = b.not(s_e);
        let out = b.output("z", Type::bit());
        b.cell(
            "odd",
            CellKind::Blackbox(Name::new("LUT1")),
            vec![(Name::new("I0"), inverted)],
            vec![(Name::new("O"), out)],
        );
        let mut m = b.finish();
        let report = replicate_wide_fanout(&mut m).expect("replicated");
        assert_eq!((report.registers, report.copies), (1, 1), "{report:?}");
        let odd = m.cells.iter().find(|(_, c)| c.name.as_str() == "odd");
        let input = odd.and_then(|(_, c)| c.input("I0"));
        assert_eq!(input, Some(inverted), "the inverted reader moved");
    }

    /// Bit numbers order as numbers. Would catch the names ordered as text,
    /// which would put `q[10]`'s readers between `q[1]`'s and `q[2]`'s.
    #[test]
    fn names_order_by_their_numbers() {
        assert!(natural("q$ff$ff9") < natural("q$ff$ff10"));
        assert!(natural("a2b") < natural("a10a"));
        assert!(natural("q") < natural("r"));
    }
}
