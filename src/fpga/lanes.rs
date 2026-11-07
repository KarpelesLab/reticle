//! Memories written part of an element at a time: their lanes, and the
//! memories of a lane each.
//!
//! A write of part of an element (`mem[a][15:8] <= d`,
//! [`Lvalue::MemPart`](crate::ir::Lvalue::MemPart)) reaches the cell form as
//! a write port whose enable has a bit per bit of the element: the write's
//! condition on the bits it writes, zero on the rest (synthesis builds it as
//! copies of the condition and constant zeros). The byte-write
//! loop is four such ports, one a byte.
//!
//! [`lanes`] reads that back: the groups of element bits that every write
//! port enables together, a byte each in the loop's case. [`split`] builds a
//! memory per lane, written by the ports that write it under the one enable
//! they give it and read by a read port per read port, the reads put back
//! together in the element's order. Nothing about the memory changes but
//! that it is several: a word read is the same bits, and a word written
//! writes the same bits, so it is exact. It is what a mapper without byte
//! enables builds from, and what one with them falls back to when the lanes
//! are not bytes.

use crate::ir::{
    AttrValue, Cell, CellId, CellKind, Const, ExprId, ExprKind, Memory, MemoryId, Module, Name,
    Type,
};
use crate::source::Span;
use crate::synth::membits::{EnBit, enable_bits, same_bit};
use crate::synth::util::{add_wire, fresh_cell_name, mk, mk_concat, mk_const, mk_net, mk_slice};

/// The attribute naming, on a memory [`split`] made, the memory it is a
/// lane of.
pub(crate) const LANE_OF: &str = "lane_of";

/// True when some write port of `mem` has an enable per bit.
pub(crate) fn has_lanes(m: &Module, mem: MemoryId) -> bool {
    m.cells.values().any(|c| {
        matches!(c.kind, CellKind::MemWrPort { mem: x, .. } if x == mem)
            && c.input("en")
                .and_then(|e| m.expr(e).ty.width())
                .is_some_and(|w| w > 1)
    })
}

/// The write ports of `mem`, in cell order.
fn write_ports(m: &Module, mem: MemoryId) -> Vec<CellId> {
    m.cells
        .iter()
        .filter(|(_, c)| matches!(c.kind, CellKind::MemWrPort { mem: x, .. } if x == mem))
        .map(|(id, _)| id)
        .collect()
}

/// The lanes of `mem`: the element's bits grouped so that every write
/// port enables the bits of a group alike, each group's bits in ascending
/// order and the groups by their lowest bit. One group of every bit when
/// no port writes part of an element.
pub(crate) fn lanes(m: &Module, mem: MemoryId) -> Vec<Vec<u32>> {
    let width = m.memories[mem].elem.width().unwrap_or(0);
    let ports: Vec<Vec<EnBit>> = write_ports(m, mem)
        .into_iter()
        .filter_map(|id| m.cells[id].input("en"))
        .map(|en| enable_bits(m, en, width))
        .collect();
    let mut groups: Vec<Vec<u32>> = Vec::new();
    for bit in 0..width {
        let b = bit as usize;
        let found = groups.iter_mut().find(|g| {
            let first = g[0] as usize;
            ports.iter().all(|p| same_bit(m, p[first], p[b]))
        });
        match found {
            Some(g) => g.push(bit),
            None => groups.push(vec![bit]),
        }
    }
    groups
}

/// The width of `mem`'s lanes when they are bytes a block RAM's byte write
/// enables can take: every lane the same number of adjacent bits, a
/// multiple of 9 or else of 8, the lanes in order from bit 0. A lane of
/// sixteen is two bytes under one enable, as Vivado drives `WEA` in pairs
/// for it (`bw_sdp_1024x32_b16`). Returns 9 or 8; `None` for any other
/// lanes, a nibble's (`bw_sdp_1024x16_b4`) for one.
pub(crate) fn byte_lanes(m: &Module, mem: MemoryId) -> Option<u32> {
    let groups = lanes(m, mem);
    let size = groups.first()?.len();
    let mut next = 0u32;
    for g in &groups {
        if g.len() != size || g.iter().zip(next..).any(|(b, want)| *b != want) {
            return None;
        }
        next += u32::try_from(g.len()).ok()?;
    }
    let size = u32::try_from(size).ok()?;
    if size % 9 == 0 {
        Some(9)
    } else if size % 8 == 0 {
        Some(8)
    } else {
        None
    }
}

/// `e`'s bits at `bits` (ascending), as one expression: runs of adjacent
/// bits as slices, most significant first.
pub(crate) fn gather(m: &mut Module, e: ExprId, bits: &[u32], span: Span) -> ExprId {
    let mut runs: Vec<(u32, u32)> = Vec::new();
    for &b in bits {
        match runs.last_mut() {
            Some((_, hi)) if *hi + 1 == b => *hi = b,
            _ => runs.push((b, b)),
        }
    }
    let parts: Vec<ExprId> = runs
        .iter()
        .rev()
        .map(|&(lo, hi)| mk_slice(m, e, hi, lo, span))
        .collect();
    mk_concat(m, parts, span)
}

/// What [`split`] made of a memory.
pub(crate) struct Split {
    /// The memories, one a lane, in lane order.
    pub lanes: Vec<MemoryId>,
    /// The memory's own ports, which nothing needs now: the caller removes
    /// them, with `mem` itself, when it removes what it has replaced, so
    /// that every id it holds stays valid until then.
    pub ports: Vec<CellId>,
}

/// Builds a memory per lane of `mem` ([`lanes`]), with their ports and the
/// reads put back together, or returns `None` having changed nothing when
/// the memory is read other than through a port. `mem` and its ports are
/// left for the caller to remove ([`Split::ports`]).
pub(crate) fn split(m: &mut Module, mem: MemoryId) -> Option<Split> {
    let read_directly = m
        .exprs
        .iter()
        .any(|(_, e)| matches!(e.kind, ExprKind::MemRead { mem: x, .. } if x == mem));
    if read_directly {
        return None;
    }
    let groups = lanes(m, mem);
    let original: Memory = m.memories[mem].clone();
    let width = original.elem.width()?;
    let span = original.span;
    let mut created = Vec::with_capacity(groups.len());
    for (k, bits) in groups.iter().enumerate() {
        let init = original.init.as_ref().map(|words| {
            words
                .iter()
                .map(|w| {
                    let w = w.resize(width);
                    let mut lane = Const::zero(u32::try_from(bits.len()).unwrap_or(0));
                    for (j, &b) in bits.iter().enumerate() {
                        lane.set_bit(u32::try_from(j).unwrap_or(0), w.bit(b));
                    }
                    lane
                })
                .collect()
        });
        // Which memory it is a lane of: a mapper deciding between block
        // and distributed RAM decides for the whole memory, not a lane.
        let mut attrs = original.attrs.clone();
        attrs.set(
            LANE_OF,
            AttrValue::String(original.name.as_str().to_owned()),
        );
        let name = fresh_memory_name(m, &format!("{}$lane{k}", original.name));
        created.push(m.memories.push(Memory {
            name,
            elem: Type::bits(u32::try_from(bits.len()).unwrap_or(0)),
            size: original.size,
            init,
            attrs,
            span,
        }));
    }
    let ports: Vec<CellId> = m
        .cells
        .iter()
        .filter(|(_, c)| {
            matches!(c.kind, CellKind::MemWrPort { mem: x, .. } | CellKind::MemRdPort { mem: x, .. } if x == mem)
        })
        .map(|(id, _)| id)
        .collect();
    let mut added: Vec<Cell> = Vec::new();
    for &id in &ports {
        let cell = m.cells[id].clone();
        match cell.kind {
            CellKind::MemWrPort { clocked, .. } => {
                let en = cell.input("en")?;
                let data = cell.input("data")?;
                let enables = enable_bits(m, en, width);
                for (k, bits) in groups.iter().enumerate() {
                    let lane_en = match enables[bits[0] as usize] {
                        EnBit::Zero => continue,
                        EnBit::One => mk_const(m, Const::ones(1), span),
                        EnBit::Cond(c) => c,
                        EnBit::Bit(e, j) => mk_slice(m, e, j, j, span),
                    };
                    let lane_data = gather(m, data, bits, span);
                    let mut c = cell.clone();
                    c.kind = CellKind::MemWrPort {
                        mem: created[k],
                        clocked,
                    };
                    set_input(&mut c, "en", lane_en);
                    set_input(&mut c, "data", lane_data);
                    c.name = Name::new(format!("{}$lane{k}", cell.name));
                    added.push(c);
                }
            }
            CellKind::MemRdPort { clocked, .. } => {
                let out = cell.output("data")?;
                let mut lane_nets = Vec::with_capacity(groups.len());
                for (k, bits) in groups.iter().enumerate() {
                    let ty = Type::bits(u32::try_from(bits.len()).unwrap_or(0));
                    let net = add_wire(m, &format!("{}$lane{k}", m.nets[out].name), ty, span);
                    lane_nets.push(net);
                    let mut c = cell.clone();
                    c.kind = CellKind::MemRdPort {
                        mem: created[k],
                        clocked,
                    };
                    c.outputs = vec![(Name::new("data"), net)];
                    c.name = Name::new(format!("{}$lane{k}", cell.name));
                    added.push(c);
                }
                // The word read, bit by bit from the lane that holds it,
                // most significant first.
                let mut parts = Vec::new();
                for bit in (0..width).rev() {
                    let (k, j) = groups
                        .iter()
                        .enumerate()
                        .find_map(|(k, g)| g.iter().position(|b| *b == bit).map(|j| (k, j)))?;
                    let j = u32::try_from(j).unwrap_or(0);
                    let lane = mk_net(m, lane_nets[k], span);
                    parts.push(mk_slice(m, lane, j, j, span));
                }
                let value = mk(m, ExprKind::Concat(parts), span);
                let value = crate::synth::opt::const_fold::simplify(m, value);
                m.assigns.push(crate::ir::Assign {
                    target: crate::ir::Lvalue::Net(out),
                    value,
                    delay: None,
                    attrs: crate::ir::Attrs::new(),
                    span: cell.span,
                });
            }
            _ => {}
        }
    }
    for mut c in added {
        c.name = fresh_cell_name(m, c.name.as_str());
        m.cells.push(c);
    }
    Some(Split {
        lanes: created,
        ports,
    })
}

/// Replaces the expression on input `port`.
fn set_input(cell: &mut Cell, port: &str, value: ExprId) {
    if let Some(slot) = cell.inputs.iter_mut().find(|(n, _)| n.as_str() == port) {
        slot.1 = value;
    }
}

/// A memory name no memory of `m` has: `base`, then `base_2`, ...
fn fresh_memory_name(m: &Module, base: &str) -> Name {
    let taken = |n: &str| m.memories.values().any(|x| x.name.as_str() == n);
    if !taken(base) {
        return Name::new(base);
    }
    let mut i = 2u32;
    loop {
        let candidate = format!("{base}_{i}");
        if !taken(&candidate) {
            return Name::new(candidate);
        }
        i += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::Design;
    use crate::source::SourceMap;

    fn module(text: &str) -> Module {
        let mut map = SourceMap::new();
        let file = map.add("t.rtl", text).unwrap();
        let design = Design::parse_text(text, file).unwrap();
        let id = design.modules.ids().next().unwrap();
        design.modules[id].clone()
    }

    const BYTES: &str = "top t\n\
        module t\n\
        \x20 net %clk u1 wire\n\
        \x20 net %a u2 wire\n\
        \x20 net %d u16 wire\n\
        \x20 net %we u2 wire\n\
        \x20 net %q u16 wire\n\
        \x20 port clk in %clk\n\
        \x20 port a in %a\n\
        \x20 port d in %d\n\
        \x20 port we in %we\n\
        \x20 port q out %q\n\
        \x20 memory @m 4 x u16\n\
        \x20   init 16'd4660 16'd22136\n\
        \x20 cell w0 memwr @m clocked (addr=%a, data={8'd0, %d[7:0]}, en={8'd0, {8{%we[0:0]}}}, clk=%clk) -> ()\n\
        \x20 cell w1 memwr @m clocked (addr=%a, data={%d[15:8], 8'd0}, en={{8{%we[1:1]}}, 8'd0}, clk=%clk) -> ()\n\
        \x20 cell r memrd @m clocked (addr=%a, clk=%clk, en=1'd1) -> (data=%q)\n\
        end\n";

    /// **Two byte writes are two lanes of eight**, and each lane memory
    /// has its port, its share of the contents, and a read. Would catch
    /// the bits grouped by port rather than by enable, a lane given the
    /// other's contents, or a read put back in the wrong order (the
    /// assign is checked bit for bit by the contents test in
    /// `tests/sim_compiled.rs` and by the memarr simulations).
    #[test]
    fn two_byte_writes_are_two_lanes() {
        let mut m = module(BYTES);
        let mem = m.memory_by_name("m").unwrap();
        assert!(has_lanes(&m, mem));
        let groups = lanes(&m, mem);
        assert_eq!(
            groups,
            vec![(0..8).collect::<Vec<u32>>(), (8..16).collect()]
        );
        let made = split(&mut m, mem).unwrap();
        m.cells.retain(|id, _| !made.ports.contains(&id));
        let made = made.lanes;
        assert_eq!(made.len(), 2);
        let lo = &m.memories[made[0]];
        assert_eq!(lo.elem, Type::bits(8));
        let init: Vec<u64> = lo
            .init
            .as_ref()
            .unwrap()
            .iter()
            .map(|c| c.to_u64().unwrap())
            .collect();
        assert_eq!(init, vec![0x34, 0x78]);
        let hi = &m.memories[made[1]];
        let init: Vec<u64> = hi
            .init
            .as_ref()
            .unwrap()
            .iter()
            .map(|c| c.to_u64().unwrap())
            .collect();
        assert_eq!(init, vec![0x12, 0x56]);
        let writes = m
            .cells
            .values()
            .filter(|c| matches!(c.kind, CellKind::MemWrPort { .. }))
            .count();
        assert_eq!(
            writes, 2,
            "each lane is written by the one port that writes it"
        );
        assert!(!m.cells.values().any(|c| matches!(
            c.kind,
            CellKind::MemWrPort { mem: x, .. } | CellKind::MemRdPort { mem: x, .. } if x == mem
        )));
    }

    /// The lane memories of `text`'s memory `m`, the module before and
    /// after in one design, and whether a bounded search of `depth` cycles
    /// found the two to differ, starting from zero (and the contents).
    #[cfg(feature = "formal")]
    fn split_differs(text: &str, mutate: impl Fn(&mut Module), depth: u32) -> bool {
        use crate::formal::equiv::{EquivOptions, EquivOutcome, check_equivalent};
        use crate::formal::unroll::InitMode;
        let before = module(text);
        let mut after = before.clone();
        let mem = after.memory_by_name("m").unwrap();
        let made = split(&mut after, mem).unwrap();
        after.cells.retain(|id, _| !made.ports.contains(&id));
        mutate(&mut after);
        after.name = Name::new("t_split");
        let mut design = Design::new();
        let a = design.add_module(before);
        let b = design.add_module(after);
        let options = EquivOptions {
            depth,
            max_k: 2,
            init: InitMode::Zero,
            match_state_by_name: false,
            ..EquivOptions::default()
        };
        let report = check_equivalent(&design, a, b, &options);
        assert!(
            !matches!(report.outcome, EquivOutcome::Unknown { depth: None }),
            "the check ran: {:?}",
            report.diags
        );
        matches!(report.outcome, EquivOutcome::Different { .. })
    }

    /// **A memory split into its lanes computes what it did**, by a
    /// bounded search over every input sequence of eight cycles from the
    /// same contents, for two byte writes and a word write to four words.
    /// The second half is the check on the check: the same split with the
    /// two lanes' reads put back the wrong way round is found to differ,
    /// so a split that wired a lane wrongly would fail here. Would not
    /// catch what eight cycles cannot reach, which on four words is little.
    #[cfg(feature = "formal")]
    #[test]
    fn a_split_memory_computes_what_it_did() {
        let text = BYTES.replace(
            "  cell r memrd",
            "  cell w2 memwr @m clocked (addr=%a, data=not(%d), en=and(%we[0:0], %we[1:1]), clk=%clk) -> ()\n  cell r memrd",
        );
        assert!(!split_differs(&text, |_| {}, 8));
        let swapped = |m: &mut Module| {
            let assign = m.assigns.last_mut().unwrap();
            let value = assign.value;
            let ExprKind::Concat(parts) = m.exprs[value].kind.clone() else {
                // Folded to two slices of lane nets: swap them.
                panic!("the read is put back as a concatenation");
            };
            let mut parts = parts;
            parts.reverse();
            m.exprs[value].kind = ExprKind::Concat(parts);
        };
        assert!(split_differs(&text, swapped, 8));
    }

    /// **A whole-word write and a byte write make lanes of what the byte
    /// write splits**, and the whole-word port writes both. Would catch a
    /// lane missing the port that writes every bit.
    #[test]
    fn a_word_write_writes_every_lane() {
        let text = BYTES.replace(
            "cell w1 memwr @m clocked (addr=%a, data={%d[15:8], 8'd0}, en={{8{%we[1:1]}}, 8'd0}, clk=%clk) -> ()",
            "cell w1 memwr @m clocked (addr=%a, data=%d, en=%we[1:1], clk=%clk) -> ()",
        );
        let mut m = module(&text);
        let mem = m.memory_by_name("m").unwrap();
        let made = split(&mut m, mem).unwrap();
        m.cells.retain(|id, _| !made.ports.contains(&id));
        assert_eq!(made.lanes.len(), 2);
        let writes = m
            .cells
            .values()
            .filter(|c| matches!(c.kind, CellKind::MemWrPort { .. }))
            .count();
        assert_eq!(writes, 3);
    }
}
