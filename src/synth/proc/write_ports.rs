//! Write ports of one address merged into one, where some write part of an
//! element.
//!
//! The byte-write loop, `for (i ..) if (we[i]) mem[a][i*8 +: 8] <= d[i*8 +:
//! 8]`, lowers to one write port per byte, each with an enable per bit that
//! is its byte's condition and zeros (`exec::Exec::mem_write_part`), and the
//! vendor's other form, a process per byte out of a `generate` loop, lowers
//! the same. A block RAM with byte write enables is one port with an enable
//! per byte, so the ports are merged here into the one port they describe:
//! its enable is each bit's from the port that writes it, and its data each
//! bit's from that port too.
//!
//! Only ports that follow one another among the memory's write ports, on one
//! clock, at one address (the same expression, node for node) are merged, and
//! only when one of them writes part of an element, so that a memory written
//! a whole word at a time is left exactly as it was. Where two of them write
//! one bit, the later one, as the cells are ordered and the processes were
//! written, wins it, which is what the simulation of the ports did.

use crate::ir::{CellId, CellKind, ExprId, MemoryId, Module};
use crate::synth::membits::{EnBit, bit_expr, enable_bits, enable_expr, same_bit};
use crate::synth::util::{mk_concat, mk_const, mk_mux, mk_or_tree, mk_slice, same_expr};

/// Merges the write ports of every memory as the module docs say; returns
/// how many ports went into another.
pub(super) fn merge_write_ports(m: &mut Module) -> u64 {
    let mut merged = 0;
    let memories: Vec<MemoryId> = m.memories.ids().collect();
    for mem in memories {
        while let Some(run) = next_run(m, mem) {
            merged += u64::try_from(run.len() - 1).unwrap_or(0);
            merge(m, mem, &run);
        }
        collapse(m, mem);
    }
    merged
}

/// A write port whose enable per bit is one condition on every bit is a
/// port of the whole word under that condition (`mem[a][31:0] <= d` of a
/// 32-bit memory), and one that enables no bit writes nothing: both are
/// put so, so that what is written a whole word at a time looks it.
fn collapse(m: &mut Module, mem: MemoryId) {
    let width = m.memories[mem].elem.width().unwrap_or(0);
    let mut idle = Vec::new();
    for cell in ports(m, mem) {
        if !wide(m, cell) {
            continue;
        }
        let Some(en) = m.cells[cell].input("en") else {
            continue;
        };
        let bits = enable_bits(m, en, width);
        let Some(first) = bits.first().copied() else {
            continue;
        };
        if matches!(first, EnBit::Bit(..)) || !bits.iter().all(|b| same_bit(m, *b, first)) {
            continue;
        }
        let span = m.cells[cell].span;
        let Some(one) = bit_expr(m, first, span) else {
            idle.push(cell);
            continue;
        };
        for (name, slot) in &mut m.cells[cell].inputs {
            if name.as_str() == "en" {
                *slot = one;
            }
        }
    }
    if !idle.is_empty() {
        m.cells.retain(|id, _| !idle.contains(&id));
    }
}

/// The write ports of `mem`, in cell order.
fn ports(m: &Module, mem: MemoryId) -> Vec<CellId> {
    m.cells
        .iter()
        .filter(|(_, c)| matches!(c.kind, CellKind::MemWrPort { mem: x, .. } if x == mem))
        .map(|(id, _)| id)
        .collect()
}

fn wide(m: &Module, cell: CellId) -> bool {
    m.cells[cell]
        .input("en")
        .and_then(|e| m.expr(e).ty.width())
        .is_some_and(|w| w > 1)
}

/// Whether two write ports are on one clock at one address.
fn together(m: &Module, a: CellId, b: CellId) -> bool {
    let (x, y) = (&m.cells[a], &m.cells[b]);
    let same = |port: &str| match (x.input(port), y.input(port)) {
        (Some(p), Some(q)) => same_expr(m, p, q, 32),
        (None, None) => true,
        _ => false,
    };
    x.kind == y.kind && same("addr") && same("clk")
}

/// The first run of two or more consecutive write ports of `mem` that
/// [`together`] holds for and one of which writes part of an element.
fn next_run(m: &Module, mem: MemoryId) -> Option<Vec<CellId>> {
    let all = ports(m, mem);
    let mut at = 0;
    while at < all.len() {
        let mut end = at + 1;
        while end < all.len() && together(m, all[at], all[end]) {
            end += 1;
        }
        let run = &all[at..end];
        if run.len() > 1 && run.iter().any(|c| wide(m, *c)) {
            return Some(run.to_vec());
        }
        at = end;
    }
    None
}

/// One bit of the merged data: a bit of one port's data, or an expression.
#[derive(Clone, Copy)]
enum DataBit {
    Zero,
    Port(usize, u32),
    Expr(ExprId),
}

/// Merges `run` into its first port and removes the others.
fn merge(m: &mut Module, mem: MemoryId, run: &[CellId]) {
    let width = m.memories[mem].elem.width().unwrap_or(0);
    let span = m.cells[run[0]].span;
    let data: Vec<ExprId> = run
        .iter()
        .map(|c| m.cells[*c].input("data").expect("a write port has data"))
        .collect();
    let enables: Vec<Vec<EnBit>> = run
        .iter()
        .map(|c| {
            let en = m.cells[*c].input("en").expect("a write port has an enable");
            enable_bits(m, en, width)
        })
        .collect();
    let mut en_bits = Vec::with_capacity(width as usize);
    let mut data_bits = Vec::with_capacity(width as usize);
    for b in 0..width {
        let bi = b as usize;
        let writers: Vec<usize> = (0..run.len())
            .filter(|p| !matches!(enables[*p][bi], EnBit::Zero))
            .collect();
        match writers.as_slice() {
            [] => {
                en_bits.push(EnBit::Zero);
                data_bits.push(DataBit::Zero);
            }
            [p] => {
                en_bits.push(enables[*p][bi]);
                data_bits.push(DataBit::Port(*p, b));
            }
            _ => {
                // Several write the bit: any of them enables it, and the
                // last one enabled gives its value.
                let first = enables[writers[0]][bi];
                let en = if writers.iter().all(|p| same_bit(m, enables[*p][bi], first)) {
                    first
                } else if writers
                    .iter()
                    .any(|p| matches!(enables[*p][bi], EnBit::One))
                {
                    EnBit::One
                } else {
                    let terms: Vec<ExprId> = writers
                        .iter()
                        .filter_map(|p| bit_expr(m, enables[*p][bi], span))
                        .collect();
                    EnBit::Cond(mk_or_tree(m, &terms, span))
                };
                let mut value = mk_slice(m, data[writers[0]], b, b, span);
                for &p in &writers[1..] {
                    let d = mk_slice(m, data[p], b, b, span);
                    value = match bit_expr(m, enables[p][bi], span) {
                        Some(c) if !matches!(enables[p][bi], EnBit::One) => {
                            mk_mux(m, c, d, value, span)
                        }
                        _ => d,
                    };
                }
                en_bits.push(en);
                data_bits.push(DataBit::Expr(value));
            }
        }
    }
    let en = enable_expr(m, &en_bits, span);
    let value = data_expr(m, &data, &data_bits, span);
    let en = crate::synth::opt::const_fold::simplify(m, en);
    let value = crate::synth::opt::const_fold::simplify(m, value);
    let cell = &mut m.cells[run[0]];
    for (name, slot) in &mut cell.inputs {
        match name.as_str() {
            "en" => *slot = en,
            "data" => *slot = value,
            _ => {}
        }
    }
    let gone = &run[1..];
    m.cells.retain(|id, _| !gone.contains(&id));
}

/// The merged data, most significant first: runs of one port's bits as a
/// slice of its data.
fn data_expr(
    m: &mut Module,
    data: &[ExprId],
    bits: &[DataBit],
    span: crate::source::Span,
) -> ExprId {
    let mut parts: Vec<ExprId> = Vec::new();
    let mut at = 0;
    while at < bits.len() {
        let mut end = at + 1;
        match bits[at] {
            DataBit::Zero => {
                while end < bits.len() && matches!(bits[end], DataBit::Zero) {
                    end += 1;
                }
                let n = u32::try_from(end - at).unwrap_or(u32::MAX);
                parts.push(mk_const(m, crate::ir::Const::zero(n), span));
            }
            DataBit::Port(p, b) => {
                let mut hi = b;
                while end < bits.len()
                    && matches!(bits[end], DataBit::Port(q, c) if q == p && c == hi + 1)
                {
                    hi += 1;
                    end += 1;
                }
                parts.push(mk_slice(m, data[p], hi, b, span));
            }
            DataBit::Expr(e) => parts.push(e),
        }
        at = end;
    }
    parts.reverse();
    mk_concat(m, parts, span)
}

#[cfg(test)]
mod tests {
    use crate::ir::{CellKind, Design, Module};
    use crate::source::SourceMap;

    fn module(text: &str) -> Module {
        let mut map = SourceMap::new();
        let file = map.add("t.rtl", text).unwrap();
        let design = Design::parse_text(text, file).unwrap();
        let id = design.modules.ids().next().unwrap();
        design.modules[id].clone()
    }

    fn writes(m: &Module) -> Vec<String> {
        m.to_text()
            .lines()
            .filter(|l| l.contains(" memwr "))
            .map(|l| l.trim().to_owned())
            .collect()
    }

    const HEAD: &str = "top t\n\
        module t\n\
        \x20 net %clk u1 wire\n\
        \x20 net %a u2 wire\n\
        \x20 net %b u2 wire\n\
        \x20 net %d u16 wire\n\
        \x20 net %e u16 wire\n\
        \x20 net %we u2 wire\n\
        \x20 port clk in %clk\n\
        \x20 port a in %a\n\
        \x20 port b in %b\n\
        \x20 port d in %d\n\
        \x20 port e in %e\n\
        \x20 port we in %we\n\
        \x20 memory @m 4 x u16\n";

    const LOW: &str = "data={8'd0, %d[7:0]}, en={8'd0, {8{%we[0:0]}}}";
    const HIGH: &str = "data={%d[15:8], 8'd0}, en={{8{%we[1:1]}}, 8'd0}";

    fn design(cells: &[String]) -> Module {
        let mut text = HEAD.to_owned();
        for c in cells {
            text.push_str("  ");
            text.push_str(c);
            text.push('\n');
        }
        text.push_str("end\n");
        module(&text)
    }

    fn port(name: &str, addr: &str, rest: &str) -> String {
        format!("cell {name} memwr @m clocked (addr={addr}, {rest}, clk=%clk) -> ()")
    }

    /// **Byte writes of one address merge into one port**, its enable a
    /// condition a byte and its data each byte's: what the block's byte
    /// write enables take. Would catch the lanes' enables or data swapped,
    /// or the ports left apart.
    #[test]
    fn byte_writes_of_one_address_merge() {
        let mut m = design(&[port("w0", "%a", LOW), port("w1", "%a", HIGH)]);
        assert_eq!(super::merge_write_ports(&mut m), 1);
        let w = writes(&m);
        assert_eq!(w.len(), 1, "{w:?}");
        // The data is each port's own byte (slices the folding after
        // proc lowering takes to `%d`).
        assert!(
            w[0].contains("data={{%d[15:8], 8'd0}[15:8], {8'd0, %d[7:0]}[7:0]}")
                && w[0].contains("en={{8{%we[1:1]}}, {8{%we[0:0]}}}"),
            "{w:?}"
        );
    }

    /// **A port at another address between them keeps them apart**, since
    /// merging would move one past it, and the order of two ports is which
    /// wins a word they both write. Would catch a merge across it.
    #[test]
    fn a_port_between_keeps_them_apart() {
        let mut m = design(&[
            port("w0", "%a", LOW),
            port("x", "%b", "data=%e, en=1'd1"),
            port("w1", "%a", HIGH),
        ]);
        assert_eq!(super::merge_write_ports(&mut m), 0);
        assert_eq!(writes(&m).len(), 3);
    }

    /// **Where two merged ports write one bit, the later wins**: a whole
    /// word under one enable and then a byte under another give the low
    /// byte the byte's data where its enable holds, and an enable that is
    /// either's. Would catch the earlier port winning.
    #[test]
    fn the_later_port_wins_a_bit_both_write() {
        let mut m = design(&[
            port("w0", "%a", "data=%e, en=%we[1:1]"),
            port("w1", "%a", LOW),
        ]);
        assert_eq!(super::merge_write_ports(&mut m), 1);
        let w = writes(&m);
        assert_eq!(w.len(), 1, "{w:?}");
        assert!(
            w[0].contains("data={%e[15:8], mux(%we[0:0], {8'd0, %d[7:0]}[7:7], %e[7:7])"),
            "{w:?}"
        );
        assert!(
            w[0].contains("en={{8{%we[1:1]}}, {8{or(%we[1:1], %we[0:0])}}}"),
            "{w:?}"
        );
    }

    /// **A write of every bit under one condition is a write of the word**:
    /// its enable per bit becomes the condition, and one that enables no
    /// bit goes. Would catch either left as an enable per bit, which would
    /// send a memory written a word at a time to the byte-lane mapping.
    #[test]
    fn a_whole_word_written_in_part_is_a_word_written() {
        let mut m = design(&[
            port("w0", "%a", "data=%d, en={16{%we[0:0]}}"),
            port("w1", "%b", "data=%e, en=16'd0"),
        ]);
        super::merge_write_ports(&mut m);
        let w = writes(&m);
        assert_eq!(w.len(), 1, "{w:?}");
        assert!(w[0].contains("en=%we[0:0]"), "{w:?}");
        assert!(
            m.cells
                .values()
                .all(|c| !matches!(c.kind, CellKind::MemWrPort { .. }) || c.name.as_str() == "w0")
        );
    }
}
