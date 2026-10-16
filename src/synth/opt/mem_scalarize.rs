//! Memories every port of which names a constant word.
//!
//! An unpacked array written and read element by element from a
//! `generate` loop — `always @(posedge clk) c[i] <= ...` for each `i` —
//! elaborates to a memory with a write port and a read port per element,
//! each at a constant address. Nothing about it is a memory: it is
//! registers. Left as one, every later step treats it as one, and the
//! FPGA mapper's fallback builds each word from every write port and each
//! read from every word, which grows as the square of the ports and was
//! the reason NewAE's ChipWhisperer-Husky gateware (an `esad` with 256
//! counters, a dozen such arrays) did not finish mapping in an hour.
//!
//! [`MemScalarize`] turns such a memory into one net per word. A word's
//! writers become one flip-flop on their common clock, its `d` a chain of
//! `mux(en, data, ...)` ending in the word itself, latest port last so
//! that it wins, as it would in the memory; [`super::FfOpt`] lifts the
//! enable out of that afterwards. A combinational read becomes the word; a
//! clocked one, a flip-flop of it with the port's enable. A word nothing
//! writes holds its initial value. An address past the end reads `x` and
//! writes nothing, which is what the memory did.
//!
//! It is left alone when any port's address is not constant, a write port
//! is not clocked, the writers of one word disagree about their clock, or
//! anything reads the memory other than through a port.

use crate::diag::Diagnostics;
use crate::ir::{
    Assign, AttrValue, Attrs, CellId, CellKind, ExprId, ExprKind, Lvalue, MemoryId, Module, NetId,
};
use crate::synth::eval::eval_closed;
use crate::synth::util::{add_cell, add_wire, mk_const, mk_mux, mk_net, mk_undef};
use crate::synth::{Pass, PassStats};

/// The memory-scalarisation pass; see the module docs.
#[derive(Debug, Default, Clone, Copy)]
pub struct MemScalarize;

impl Pass for MemScalarize {
    fn name(&self) -> &'static str {
        "mem_scalarize"
    }

    fn run(&self, m: &mut Module, _diags: &mut Diagnostics) -> PassStats {
        let mut stats = PassStats::default();
        let memories: Vec<MemoryId> = m.memories.ids().collect();
        let mut gone = Vec::new();
        for mem in memories {
            if let Some(words) = scalarize(m, mem) {
                stats.bump("memories split into registers", 1);
                stats.bump("words", words);
                gone.push(mem);
            }
        }
        if !gone.is_empty() {
            // The memories' ports went with them, so only the ids of the
            // memories left need moving.
            let remap = m.memories.retain(|id, _| !gone.contains(&id));
            for (_, cell) in m.cells.iter_mut() {
                if let CellKind::MemRdPort { mem, .. } | CellKind::MemWrPort { mem, .. } =
                    &mut cell.kind
                    && let Some(Some(new)) = remap.get(mem.index())
                {
                    *mem = *new;
                }
            }
        }
        stats
    }
}

/// One port, with its address resolved and its pins read out.
struct Port {
    cell: CellId,
    /// The word, or `None` for an address past the end.
    word: Option<usize>,
    clk: Option<ExprId>,
    en: Option<ExprId>,
    /// The written value, for a write port.
    data: Option<ExprId>,
    /// The read value's net, for a read port.
    out: Option<NetId>,
    clocked: bool,
}

/// Replaces `mem` with registers when every port names a constant word;
/// returns the number of words, or `None` having changed nothing.
fn scalarize(m: &mut Module, mem: MemoryId) -> Option<u64> {
    let memory = &m.memories[mem];
    let ty = memory.elem.clone();
    ty.width()?;
    let size = usize::try_from(memory.size).ok()?;
    // Arrays this large are memories in earnest.
    if size > 4096 {
        return None;
    }
    let base = memory.name.as_str().to_owned();
    let init = memory.init.clone().unwrap_or_default();
    let span = memory.span;

    // Nothing but ports may read it.
    let read_directly = m
        .exprs
        .iter()
        .any(|(_, e)| matches!(e.kind, ExprKind::MemRead { mem: x, .. } if x == mem));
    let in_process = !m.processes.is_empty();
    if read_directly || in_process {
        return None;
    }

    // Every port, checked before anything changes.
    let mut reads = Vec::new();
    let mut writes = Vec::new();
    for (id, cell) in m.cells.iter() {
        let (is_write, clocked) = match &cell.kind {
            CellKind::MemRdPort { mem: x, clocked } if *x == mem => (false, *clocked),
            CellKind::MemWrPort { mem: x, clocked } if *x == mem => (true, *clocked),
            _ => continue,
        };
        let addr = eval_closed(m, cell.input("addr")?)?.to_u64();
        let word = addr
            .and_then(|a| usize::try_from(a).ok())
            .filter(|a| *a < size);
        let port = Port {
            cell: id,
            word,
            clk: cell.input("clk"),
            en: cell.input("en"),
            data: cell.input("data"),
            out: cell.output("data"),
            clocked,
        };
        if clocked && port.clk.is_none() {
            return None;
        }
        if is_write {
            if !clocked || port.en.is_none() || port.data.is_none() {
                return None;
            }
            writes.push(port);
        } else {
            port.out?;
            reads.push(port);
        }
    }
    // One clock per word.
    for word in 0..size {
        let mut clock: Option<NetId> = None;
        for port in writes.iter().filter(|p| p.word == Some(word)) {
            let net = m.expr(port.clk?).as_net()?;
            if *clock.get_or_insert(net) != net {
                return None;
            }
        }
    }

    // The words.
    let words: Vec<NetId> = (0..size)
        .map(|k| add_wire(m, &format!("{base}${k}"), ty.clone(), span))
        .collect();
    for (k, net) in words.iter().enumerate() {
        if let Some(value) = init.get(k) {
            m.nets[*net]
                .attrs
                .set("init", AttrValue::Const(value.clone()));
        }
        let writers: Vec<&Port> = writes.iter().filter(|p| p.word == Some(k)).collect();
        if writers.is_empty() {
            let value = match init.get(k) {
                Some(c) => mk_const(m, c.clone(), span),
                None => mk_undef(m, &ty, span),
            };
            m.assigns.push(Assign {
                target: Lvalue::Net(*net),
                value,
                delay: None,
                attrs: Attrs::new(),
                span,
            });
            continue;
        }
        // The latest port is the outermost choice, so it wins.
        let mut d = mk_net(m, *net, span);
        let mut clk = None;
        for port in writers {
            if let (Some(en), Some(data)) = (port.en, port.data) {
                d = mk_mux(m, en, data, d, span);
            }
            clk = port.clk;
        }
        let Some(clk) = clk else { continue };
        add_cell(
            m,
            &format!("{base}${k}$ff"),
            CellKind::Dff {
                clk_pos: true,
                has_enable: false,
                reset: None,
            },
            vec![("clk", clk), ("d", d)],
            vec![("q", *net)],
            span,
        );
    }

    // The reads.
    for port in &reads {
        let Some(out) = port.out else { continue };
        let value = match port.word {
            Some(k) => mk_net(m, words[k], span),
            None => mk_undef(m, &ty, span),
        };
        if let (true, Some(clk)) = (port.clocked, port.clk) {
            let mut inputs = vec![("clk", clk), ("d", value)];
            if let Some(en) = port.en {
                inputs.push(("en", en));
            }
            add_cell(
                m,
                &format!("{base}$rd$ff"),
                CellKind::Dff {
                    clk_pos: true,
                    has_enable: port.en.is_some(),
                    reset: None,
                },
                inputs,
                vec![("q", out)],
                span,
            );
        } else {
            m.assigns.push(Assign {
                target: Lvalue::Net(out),
                value,
                delay: None,
                attrs: Attrs::new(),
                span,
            });
        }
    }
    let ports: Vec<CellId> = reads.iter().chain(&writes).map(|p| p.cell).collect();
    m.cells.retain(|id, _| !ports.contains(&id));
    u64::try_from(size).ok()
}
