//! Bits of one net registered by several clocked processes.
//!
//! Verilog lets each process drive its own bits of a vector:
//!
//! ```verilog
//! for (i = 0; i < 9; i = i + 1) begin : g
//!     always @(posedge clk) clip[i] <= ...;
//! end
//! ```
//!
//! NewAE's ChipWhisperer-Husky gateware does exactly this. Process
//! lowering builds one flip-flop per assigned net, holding every bit the
//! process does not write, so the second process found the net already
//! driven and was refused (`S0016`).
//!
//! This step runs first. When every process that writes a net is clocked,
//! writes it only with non-blocking assignments, writes only constant
//! bits of it, and no two of them write the same bit, each process gets
//! a net of its own (`clip$part`, `clip$part_2`, …) holding just the bits
//! it writes, and its writes move there. One continuous assignment per run
//! of bits then puts each part back into the net. A part as wide as the
//! net would hold every bit its process does not write as a flip-flop
//! feeding itself, which nothing removes: four processes on four bits
//! built sixteen.
//!
//! Reads stay on the original net: a non-blocking write is never read
//! back within the cycle, so every read meant the register's output,
//! which the net still is.
//!
//! Anything else is left alone and still refused: a combinational
//! process, a blocking write, a variable index, or two processes on one
//! bit.

use std::collections::{BTreeMap, BTreeSet};

use crate::ir::walk::walk_block;
use crate::ir::{
    Arena, Assign, AssignKind, Attrs, ExprKind, Lvalue, Module, Net, NetId, Process, ProcessId,
    ProcessKind, StmtKind, Type,
};
use crate::synth::util::{fresh_net_name, mk_net, mk_slice};

/// What one process writes of one net.
#[derive(Default)]
struct Writes {
    /// The bits written, or `None` once a write is not of constant bits.
    bits: Option<BTreeSet<u32>>,
    /// True when every write is non-blocking.
    nonblocking: bool,
}

/// One process's share of a split net: the part net, and which bit of
/// the net each of its bits is, lowest first.
pub(super) type Part = (NetId, Vec<u32>);

/// Splits the nets several processes drive bit by bit; returns how many
/// were split, and records the parts of each in `parts` so the caller can
/// carry an initial value over once the `initial` processes have run.
pub(super) fn split_shared_registers(
    m: &mut Module,
    processes: &mut Arena<ProcessId, Process>,
    parts: &mut Vec<(NetId, Vec<Part>)>,
) -> u64 {
    // Who writes what.
    let mut writers: BTreeMap<NetId, Vec<(usize, Writes)>> = BTreeMap::new();
    for (index, process) in processes.values().enumerate() {
        if matches!(process.kind, ProcessKind::Initial) {
            continue;
        }
        let clocked = matches!(process.kind, ProcessKind::Sequential { .. });
        let mut mine: BTreeMap<NetId, Writes> = BTreeMap::new();
        walk_block(&process.body, &mut |stmt| {
            let (targets, nonblocking): (Vec<&Lvalue>, bool) = match &stmt.kind {
                StmtKind::Assign { target, kind, .. } => {
                    (vec![target], *kind == AssignKind::NonBlocking)
                }
                StmtKind::For { init, step, .. } => (
                    init.iter().chain(step.iter()).map(|(lv, _)| lv).collect(),
                    false,
                ),
                _ => return,
            };
            for target in targets {
                record(m, target, clocked && nonblocking, &mut mine);
            }
        });
        for (net, writes) in mine {
            writers.entry(net).or_default().push((index, writes));
        }
    }

    let mut split = 0;
    let ids: Vec<ProcessId> = processes.ids().collect();
    for (net, by) in writers {
        let Some(width) = m.nets[net].ty.width() else {
            continue;
        };
        if by.len() < 2 {
            continue;
        }
        let mut claimed: BTreeSet<u32> = BTreeSet::new();
        let disjoint = by.iter().all(|(_, w)| {
            w.nonblocking
                && w.bits
                    .as_ref()
                    .is_some_and(|bits| bits.iter().all(|b| *b < width && claimed.insert(*b)))
        });
        // A continuous assignment or a cell already driving the whole net
        // is a conflict this does not resolve.
        let driven = m.assigns.iter().any(|a| a.target.nets().contains(&net))
            || m.cells
                .values()
                .any(|c| c.outputs.iter().any(|(_, n)| *n == net));
        if !disjoint || driven {
            continue;
        }
        let original = m.nets[net].clone();
        let mut made = Vec::new();
        for (index, writes) in &by {
            let order: Vec<u32> = writes.bits.iter().flatten().copied().collect();
            let width = u32::try_from(order.len()).unwrap_or(u32::MAX);
            let part = m.nets.push(Net {
                name: fresh_net_name(m, &format!("{}$part", original.name)),
                ty: Type::bits(width),
                kind: original.kind,
                attrs: original.attrs.clone(),
                span: original.span,
            });
            let at = |bit: u32| u32::try_from(order.partition_point(|b| *b < bit)).unwrap_or(0);
            let process = &mut processes[ids[*index]];
            let module: &Module = m;
            crate::ir::walk::walk_block_mut(&mut process.body, &mut |stmt| {
                if let StmtKind::Assign { target, .. } = &mut stmt.kind {
                    retarget(module, target, net, part, &at);
                }
            });
            let span = process.span;
            let read = mk_net(m, part, span);
            for (lo, hi) in runs(order.iter()) {
                let value = mk_slice(m, read, at(hi), at(lo), span);
                m.assigns.push(Assign {
                    target: Lvalue::Slice { net, hi, lo },
                    value,
                    delay: None,
                    attrs: Attrs::new(),
                    span,
                });
            }
            made.push((part, order));
        }
        parts.push((net, made));
        split += 1;
    }
    split
}

/// Adds what `target` writes to `mine`.
fn record(m: &Module, target: &Lvalue, nonblocking: bool, mine: &mut BTreeMap<NetId, Writes>) {
    let (net, bits) = match target {
        Lvalue::Concat(parts) => {
            for part in parts {
                record(m, part, nonblocking, mine);
            }
            return;
        }
        Lvalue::MemElem { .. } => return,
        Lvalue::Net(net) => (*net, None),
        Lvalue::Slice { net, hi, lo } => (*net, Some((*lo..=*hi).collect::<Vec<_>>())),
        Lvalue::Index { net, index } => {
            let bit = match &m.expr(*index).kind {
                ExprKind::Const(c) => c.to_u64().and_then(|b| u32::try_from(b).ok()),
                _ => None,
            };
            (*net, bit.map(|b| vec![b]))
        }
    };
    let first = !mine.contains_key(&net);
    let entry = mine.entry(net).or_default();
    if first {
        entry.bits = Some(BTreeSet::new());
        entry.nonblocking = true;
    }
    entry.nonblocking &= nonblocking;
    match (bits, entry.bits.as_mut()) {
        (Some(bits), Some(set)) => set.extend(bits),
        _ => entry.bits = None,
    }
}

/// Moves every write of `from` in `target` to the same bits of `to`,
/// where bit `b` of `from` is bit `at(b)`. Only constant bits reach here.
fn retarget(m: &Module, target: &mut Lvalue, from: NetId, to: NetId, at: &dyn Fn(u32) -> u32) {
    match target {
        Lvalue::Slice { net, hi, lo } if *net == from => {
            *target = Lvalue::Slice {
                net: to,
                hi: at(*hi),
                lo: at(*lo),
            };
        }
        Lvalue::Index { net, index } if *net == from => {
            let bit = m
                .expr(*index)
                .as_const()
                .and_then(crate::ir::Const::to_u64)
                .and_then(|b| u32::try_from(b).ok())
                .unwrap_or(0);
            *target = Lvalue::Slice {
                net: to,
                hi: at(bit),
                lo: at(bit),
            };
        }
        Lvalue::Concat(parts) => parts.iter_mut().for_each(|p| retarget(m, p, from, to, at)),
        _ => {}
    }
}

/// The runs of consecutive bits in an ascending sequence, as `(lo, hi)`.
fn runs<'a>(bits: impl Iterator<Item = &'a u32>) -> Vec<(u32, u32)> {
    let mut out: Vec<(u32, u32)> = Vec::new();
    for &bit in bits {
        match out.last_mut() {
            Some((_, hi)) if *hi + 1 == bit => *hi = bit,
            _ => out.push((bit, bit)),
        }
    }
    out
}
