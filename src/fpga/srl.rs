//! Turning a chain of flip-flops into a shift register.
//!
//! A slice's lookup table can hold thirty-two bits that shift, on a part where
//! thirty-two flip-flops take four slices, and a delay line is common enough
//! that [`extract`] looks for one after the mapper has finished and the design
//! is all primitives.
//!
//! # What Vivado does, which is what this does
//!
//! **Checked** against what Vivado 2026.1's synthesis made of each chain in
//! `testdata/fpga/vivado/slice_cells/chain*` (`tests/fpga_xray_slice_cells.rs`):
//!
//! - A chain is flip-flops with **one clock, one clock enable and no set or
//!   reset**, each reading only the one before, whose every stage but the last
//!   is read by nothing else. `chain8_rst`, `chain8_arst` and `chain8_set` stay
//!   flip-flops; `chain8_en`, `chain8_init` and `chain8_neg` do not. A stage read
//!   by something else ends a chain and another may start after it
//!   (`chain8_tap`); `chain8_tap_each` is all flip-flops.
//! - **The last stage stays a flip-flop**, and the others become the shift
//!   register, so three flip-flops are a two-bit `SRL16E` and a flip-flop. Fewer
//!   than three are left alone ([`DEFAULT_MIN_LENGTH`], `pure2`). **Unless what
//!   reads it is a flip-flop that could not be a stage**, one with another enable
//!   or clock edge: then that flip-flop registers the shift register's output and
//!   the last stage goes in with the rest (`chain8_en`, `chain8_neg`: all eight).
//! - **A stage reading a pad stays a flip-flop too**, which is where it may
//!   be packed into the pad's own register: `pure3` stays three flip-flops and
//!   `pure4` is one, a two-bit `SRL16E`, and one.
//! - **Up to 16 bits is an `SRL16E` and up to 32 an `SRLC32E`**; a longer one is
//!   as many 32-bit ones as it takes, each's `Q31` the next's `D`, and the rest,
//!   whose `Q` feeds the flip-flop. Vivado's rest of up to 16 is an `SRL16E` on the
//!   five-input half, which can read a cascade; the six-input form used here
//!   cannot, so here it is an `SRLC32E`.
//! - Each stage's `INIT` goes in the shift register's, so a chain with initial
//!   values may still be one (`chain8_init`).
//! - **`ASYNC_REG`, `shreg_extract = "no"`, `keep` and `dont_touch` keep a chain
//!   flip-flops**, and so does this, and so does a placement constraint that names
//!   the cell, which is not asked of Vivado. `srl_style = "register"` is a difference:
//!   Vivado 2026.1's synthesis made `chain8_srl_reg` a shift register all the same
//!   at the place it was written, on a `reg`, and this honours what it says.
//!   (An asynchronous reset named by a bit, `posedge sr[2]`, is an internal error of
//!   the Verilog front end, seen while writing `chain8_arst`, and not this
//!   module's; that fixture goes through a named wire.)
//!
//! Two things come first, because a chain cannot be seen without them. A
//! flip-flop nothing reads is gone (`prune`): a register whose top bits are
//! unused kept them until here, since mapping sees a register whole. And the same
//! flip-flop written twice, the same `D`, clock, enable and reset, is one flip-flop
//! (`merge`). Vivado does both, and `qor/srl_delay` is 2,081 flip-flops as
//! written and a chain of 96 with 32 taps once the copies are gone.
//!
//! # What this changes
//!
//! Every design with a chain, the Husky Plus included, gets a different netlist
//! and so a different placement, and whether that is better on a board is not
//! something a static tool can say. `reticle fpga --srl-min 0` turns the whole
//! thing off and gets back the netlist from before (`qor/`'s thirteen designs,
//! byte for byte, against the commit before this one;
//! `a_design_with_no_chain_is_unchanged` holds that a design with nothing to find
//! does not move). See `docs/fpga-vivado-parity.md`.
//!
//! **Not done**: a chain read at an address that moves, which Vivado makes an
//! `SRLC32E` with its `A` wired (`mem_shift`, `mem_shift16`). Here that is
//! flip-flops and a multiplexer, which is what it was before.

use std::collections::{BTreeMap, HashMap, HashSet};

use super::constraints::{Constraints, matches_glob};
use super::device::{BelKind, BelRole, Device};
use super::primitives::{add_assign, add_cell, add_net, const_expr, net_expr};
use crate::ir::emit::{BitView, SigBit};
use crate::ir::{AttrValue, CellId, CellKind, Const, Design, ExprId, Module, ModuleId, Name, Type};
use crate::logic::Bit;
use crate::source::Span;

/// How many flip-flops make a chain worth a shift register by default: Vivado's
/// number. `gray_cdc`'s three-stage synchronisers are one, and a two-stage one
/// (`pure2`) is left alone.
pub const DEFAULT_MIN_LENGTH: u32 = 3;

/// What [`extract`] did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SrlReport {
    /// Flip-flops nothing read, and went.
    pub unread: usize,
    /// Flip-flops that were the same flip-flop as another, and went.
    pub merged: usize,
    /// Chains that became shift registers.
    pub chains: usize,
    /// Flip-flops those chains took.
    pub flip_flops: usize,
    /// Shift registers they became.
    pub shift_registers: usize,
}

/// One flip-flop that could be a stage of a chain, on the bit-level view.
struct Stage {
    cell: CellId,
    /// `FDRE` or `FDRE_1`.
    primitive: String,
    /// The `D`, `C`, `CE` and `R` pins' bits, and `Q`'s.
    d: SigBit,
    clock: SigBit,
    enable: SigBit,
    reset: SigBit,
    q: SigBit,
    /// `INIT`'s one bit.
    init: bool,
    /// Its `D` is what a pad delivers.
    reads_pad: bool,
    /// Nothing marks it as staying a flip-flop.
    free: bool,
}

/// The attributes that keep a register's flip-flops flip-flops. Read with their
/// case folded, as a Verilog attribute's name is written either way.
fn says_no_shift_register(attrs: &crate::ir::Attrs) -> bool {
    attrs.iter().any(|(key, value)| {
        let key = key.as_str().to_ascii_lowercase();
        let value = value
            .as_str()
            .map(str::to_ascii_lowercase)
            .or_else(|| value.as_int().map(|v| v.to_string()))
            .unwrap_or_default();
        match key.as_str() {
            "async_reg" => value != "false" && value != "0",
            "shreg_extract" => value == "no" || value == "false" || value == "0",
            "srl_style" => value == "register",
            // Quoted from UG901 and not asked: registers a design duplicated
            // on purpose, for a pad or for its fanout.
            "iob" => value == "true" || value == "yes",
            "equivalent_register_removal" => value == "no" || value == "false",
            _ => false,
        }
    })
}

/// One bit of a pin of `cell`, canonical, or `None` if it has not exactly one.
fn one_bit(view: &BitView<'_>, expr: Option<ExprId>) -> Option<SigBit> {
    let bits = view.expr_bits(expr?).ok()?;
    match bits.as_slice() {
        [bit] => Some(view.canonical(*bit)),
        _ => None,
    }
}

/// Every flip-flop of `module` that may be a stage: an `FDRE` or `FDRE_1`, marked
/// `free` unless an attribute says otherwise.
fn stages(
    module: &Module,
    device: &Device,
    view: &BitView<'_>,
    constraints: &Constraints,
) -> Vec<Stage> {
    // A flip-flop a placement constraint names stays what it names: it is the
    // design's say-so where that cell goes, and a shift register has no such name.
    let named = |cell: &str| {
        constraints
            .region_assignments
            .iter()
            .map(|assignment| assignment.pattern.as_str())
            .chain(constraints.rlocs.iter().map(|rloc| rloc.pattern.as_str()))
            .any(|pattern| matches_glob(pattern, cell))
    };
    let mut marked: HashSet<SigBit> = HashSet::new();
    // What a pad delivers: a port read directly, or an input buffer's output.
    let mut pads: HashSet<SigBit> = HashSet::new();
    for port in &module.ports {
        if port.dir != crate::ir::PortDir::Out
            && let Ok(bits) = view.net_bits(port.net, port.span)
        {
            pads.extend(bits.iter().map(|b| view.canonical(*b)));
        }
    }
    for (_, cell) in module.cells.iter() {
        if let CellKind::Blackbox(name) = &cell.kind
            && device
                .bels
                .iter()
                .any(|b| b.name == name.as_str() && b.role == BelRole::Io)
        {
            for (_, net) in &cell.outputs {
                if let Ok(bits) = view.net_bits(*net, cell.span) {
                    pads.extend(bits.iter().map(|b| view.canonical(*b)));
                }
            }
        }
    }
    for (id, net) in module.nets.iter() {
        if (says_no_shift_register(&net.attrs) || crate::synth::keep::is_kept(&net.attrs))
            && let Ok(bits) = view.net_bits(id, net.span)
        {
            marked.extend(bits.iter().map(|b| view.canonical(*b)));
        }
    }
    let mut out = Vec::new();
    for (id, cell) in module.cells.iter() {
        let CellKind::Blackbox(name) = &cell.kind else {
            continue;
        };
        if !matches!(name.as_str(), "FDRE" | "FDRE_1") {
            continue;
        }
        let Some(bel) = device.bels.iter().find(|b| b.name == name.as_str()) else {
            continue;
        };
        let pin = |role: &str| bel.port(role).and_then(|port| cell.input(port));
        let (Some(d), Some(clock), Some(enable), Some(reset)) = (
            one_bit(view, pin("d")),
            one_bit(view, pin("clk")),
            one_bit(view, pin("en")),
            one_bit(view, pin("rst")),
        ) else {
            continue;
        };
        let q = bel
            .port("q")
            .and_then(|port| cell.output(port))
            .and_then(|net| view.net_bits(net, cell.span).ok())
            .and_then(|bits| match bits.as_slice() {
                [bit] => Some(view.canonical(*bit)),
                _ => None,
            });
        let Some(q) = q else { continue };
        let init = matches!(
            cell.params.get("INIT"),
            Some(AttrValue::Const(c)) if c.bit(0) == Bit::One
        ) || matches!(cell.params.get("INIT").and_then(AttrValue::as_int), Some(v) if v & 1 == 1);
        out.push(Stage {
            cell: id,
            primitive: name.as_str().to_owned(),
            d,
            clock,
            enable,
            reset,
            q,
            init,
            reads_pad: pads.contains(&d),
            free: !crate::synth::keep::cell_is_kept(module, cell)
                && !marked.contains(&q)
                && !named(cell.name.as_str()),
        });
    }
    out
}

/// How many things read each bit: every cell's inputs, an output port, and an
/// instance's connections.
pub(crate) fn read_counts(module: &Module, view: &BitView<'_>) -> HashMap<SigBit, usize> {
    let mut reads: HashMap<SigBit, usize> = HashMap::new();
    let mut count = |expr: ExprId| {
        if let Ok(bits) = view.expr_bits(expr) {
            for bit in bits {
                *reads.entry(view.canonical(bit)).or_default() += 1;
            }
        }
    };
    for (_, cell) in module.cells.iter() {
        for (_, expr) in &cell.inputs {
            count(*expr);
        }
    }
    for (_, inst) in module.instances.iter() {
        for (_, expr) in &inst.connections {
            count(*expr);
        }
    }
    for port in &module.ports {
        if port.dir != crate::ir::PortDir::In
            && let Ok(bits) = view.net_bits(port.net, port.span)
        {
            for bit in bits {
                *reads.entry(view.canonical(bit)).or_default() += 1;
            }
        }
    }
    reads
}

/// Removes every flip-flop nothing reads, and what only it read: a round for
/// each stage of an unread chain. Returns how many went.
fn prune(module: &mut Module, device: &Device, constraints: &Constraints) -> Result<usize, String> {
    let gone: HashSet<CellId> = {
        let view = BitView::new(module).map_err(|e| e.to_string())?;
        let found = stages(module, device, &view, constraints);
        let mut reads = read_counts(module, &view);
        let mut gone = HashSet::new();
        loop {
            let mut changed = false;
            for stage in &found {
                if gone.contains(&stage.cell)
                    || !stage.free
                    || reads.get(&stage.q).copied().unwrap_or(0) != 0
                {
                    continue;
                }
                gone.insert(stage.cell);
                changed = true;
                for bit in [stage.d, stage.clock, stage.enable, stage.reset] {
                    if let Some(count) = reads.get_mut(&bit) {
                        *count = count.saturating_sub(1);
                    }
                }
            }
            if !changed {
                break;
            }
        }
        gone
    };
    if !gone.is_empty() {
        module.cells.retain(|id, _| !gone.contains(&id));
    }
    Ok(gone.len())
}

/// Removes every flip-flop that is the same as another: same primitive, same
/// `D`, clock, enable and reset, same `INIT`. Its output becomes the first one's.
///
/// That can make two more the same, one stage on: `srl_delay`'s 64 words of 32
/// bits are one bit delayed, and a round per stage finds it. A round looks at
/// each flip-flop once, so this is the design's size times its longest run of
/// copies, and a design with none pays for one round. Returns how many went.
fn merge(module: &mut Module, device: &Device, constraints: &Constraints) -> Result<usize, String> {
    let replaced: Vec<(CellId, CellId)> = {
        let view = BitView::new(module).map_err(|e| e.to_string())?;
        let found = stages(module, device, &view, constraints);
        let mut rep: HashMap<SigBit, SigBit> = HashMap::new();
        let lookup = |rep: &HashMap<SigBit, SigBit>, bit: SigBit| {
            let mut bit = bit;
            while let Some(next) = rep.get(&bit) {
                bit = *next;
            }
            bit
        };
        let mut gone: Vec<(CellId, CellId)> = Vec::new();
        let mut dead = vec![false; found.len()];
        loop {
            let mut first: BTreeMap<String, usize> = BTreeMap::new();
            let mut changed = false;
            for (index, stage) in found.iter().enumerate() {
                if dead[index] || !stage.free {
                    continue;
                }
                let key = format!(
                    "{} {:?} {:?} {:?} {:?} {}",
                    stage.primitive,
                    lookup(&rep, stage.d),
                    lookup(&rep, stage.clock),
                    lookup(&rep, stage.enable),
                    lookup(&rep, stage.reset),
                    stage.init
                );
                match first.get(&key) {
                    Some(&keep) => {
                        rep.insert(stage.q, found[keep].q);
                        gone.push((stage.cell, found[keep].cell));
                        dead[index] = true;
                        changed = true;
                    }
                    None => {
                        first.insert(key, index);
                    }
                }
            }
            if !changed {
                break;
            }
        }
        gone
    };
    let count = replaced.len();
    if count == 0 {
        return Ok(0);
    }
    for (cell, keep) in &replaced {
        let span = module.cells[*cell].span;
        let (Some((_, from)), Some((_, to))) = (
            module.cells[*keep].outputs.first().cloned(),
            module.cells[*cell].outputs.first().cloned(),
        ) else {
            continue;
        };
        let value = net_expr(module, from, span);
        add_assign(module, to, value, span);
    }
    let gone: HashSet<CellId> = replaced.iter().map(|(cell, _)| *cell).collect();
    module.cells.retain(|id, _| !gone.contains(&id));
    Ok(count)
}

/// One chain, its stages in order.
struct Chain {
    stages: Vec<usize>,
    /// What reads the last stage is a flip-flop of another control set, which
    /// registers the shift register's output, so the last stage goes in too.
    registered: bool,
}

/// Every chain of at least `min_length` stages.
fn chains(module: &Module, found: &[Stage], view: &BitView<'_>, min_length: u32) -> Vec<Chain> {
    let reads = read_counts(module, view);
    let by_q: HashMap<SigBit, usize> = found
        .iter()
        .enumerate()
        .filter(|(_, stage)| stage.free)
        .map(|(index, stage)| (stage.q, index))
        .collect();
    // The stage each one follows, when it can follow it in a chain.
    let mut previous: Vec<Option<usize>> = vec![None; found.len()];
    let mut next: Vec<Option<usize>> = vec![None; found.len()];
    for (index, stage) in found.iter().enumerate() {
        if !stage.free || stage.reset != SigBit::Const(Bit::Zero) {
            continue;
        }
        let Some(&before) = by_q.get(&stage.d) else {
            continue;
        };
        let other = &found[before];
        if before != index
            && !other.reads_pad
            && reads.get(&other.q).copied().unwrap_or(0) == 1
            && other.reset == SigBit::Const(Bit::Zero)
            && other.primitive == stage.primitive
            && other.clock == stage.clock
            && other.enable == stage.enable
        {
            previous[index] = Some(before);
            next[before] = Some(index);
        }
    }
    let mut out = Vec::new();
    for start in 0..found.len() {
        if previous[start].is_some() || next[start].is_none() {
            continue;
        }
        let mut chain = vec![start];
        let mut at = start;
        while let Some(following) = next[at] {
            chain.push(following);
            at = following;
        }
        if u32::try_from(chain.len()).unwrap_or(u32::MAX) >= min_length {
            let tail = &found[chain[chain.len() - 1]];
            let registered = reads.get(&tail.q).copied().unwrap_or(0) == 1
                && found.iter().any(|stage| stage.d == tail.q);
            out.push(Chain {
                stages: chain,
                registered,
            });
        }
    }
    out
}

/// What one shift register's inputs are, apart from its depth and contents.
struct Source {
    data: ExprId,
    clock: ExprId,
    enable: ExprId,
    inverted: bool,
    name: String,
    span: Span,
}

/// One shift register of `depth` bits whose stored bits are `init`, first stage
/// first, and the net its output is on: its `Q` at that depth, or its `Q31` when
/// `cascades`, for the next one's `D`. `short` says it may be the sixteen-bit form,
/// whose data is not a cascade's.
fn emit(
    module: &mut Module,
    source: &Source,
    index: usize,
    (depth, short, cascades): (u32, bool, bool),
    init: &[bool],
    bels: (&BelKind, &BelKind),
) -> crate::ir::NetId {
    let (long_bel, short_bel) = bels;
    let bel = if short && depth <= 16 {
        short_bel
    } else {
        long_bel
    };
    let wide = std::ptr::eq(bel, long_bel);
    let span = source.span;
    let net = add_net(
        module,
        &format!("{}$srl{index}", source.name),
        Type::bit(),
        span,
    );
    let mut inputs = vec![
        (Name::new(bel.port("d").unwrap_or("D")), source.data),
        (Name::new(bel.port("clk").unwrap_or("CLK")), source.clock),
        (Name::new(bel.port("ce").unwrap_or("CE")), source.enable),
    ];
    let address = u64::from(depth - 1);
    let names = bel.port_names("a");
    if let [one] = names.as_slice() {
        let e = const_expr(module, Const::from_u64(address, 5), span);
        inputs.push((Name::new(*one), e));
    } else {
        for (bit, name) in names.iter().enumerate() {
            let value = (address >> bit) & 1;
            let e = const_expr(module, Const::from_u64(value, 1), span);
            inputs.push((Name::new(*name), e));
        }
    }
    let out = if cascades { "q31" } else { "q" };
    let id = add_cell(
        module,
        &format!("{}$srl{index}", source.name),
        CellKind::Blackbox(Name::new(bel.name.clone())),
        inputs,
        vec![(Name::new(bel.port(out).unwrap_or("Q")), net)],
        span,
    );
    let width = if wide { 32 } else { 16 };
    let mut value = 0u64;
    for (bit, set) in init.iter().enumerate() {
        if *set {
            value |= 1 << bit;
        }
    }
    module.cells[id].params.set(
        Name::new("INIT"),
        AttrValue::Const(Const::from_u64(value, width)),
    );
    if source.inverted {
        module.cells[id]
            .params
            .set(Name::new("IS_CLK_INVERTED"), 1i64);
    }
    net
}

/// Merges the copies of a flip-flop and makes every chain of `min_length`
/// flip-flops or more a shift register and a flip-flop; `min_length` of 0 does
/// nothing at all, for a build that wants the netlist from before.
///
/// A device that declares no shift register has nothing done to it.
///
/// # Errors
///
/// A message when the module's assignments do not resolve to bits.
pub fn extract(
    design: &mut Design,
    module: ModuleId,
    device: &Device,
    constraints: &Constraints,
    min_length: u32,
) -> Result<SrlReport, String> {
    let mut report = SrlReport::default();
    if min_length == 0 {
        return Ok(report);
    }
    let (Some(long), Some(short)) = (
        device.bel(BelRole::Srl32).cloned(),
        device.bel(BelRole::Srl16).cloned(),
    ) else {
        return Ok(report);
    };
    let Some(m) = design.modules.get_mut(module) else {
        return Ok(report);
    };
    report.unread = prune(m, device, constraints)?;
    report.merged = merge(m, device, constraints)?;
    let (planned, cells): (Vec<Chain>, Vec<(CellId, bool)>) = {
        let view = BitView::new(m).map_err(|e| e.to_string())?;
        let found = stages(m, device, &view, constraints);
        let planned = chains(m, &found, &view, min_length);
        let cells = found.iter().map(|s| (s.cell, s.init)).collect();
        (planned, cells)
    };
    let mut gone: HashSet<CellId> = HashSet::new();
    for planned in &planned {
        let chain = &planned.stages;
        let last = chain[chain.len() - 1];
        let (first_cell, last_cell) = (cells[chain[0]].0, cells[last].0);
        let cell = &m.cells[first_cell];
        let CellKind::Blackbox(primitive) = &cell.kind else {
            continue;
        };
        let inverted = primitive.as_str() == "FDRE_1";
        let Some(ff) = device.bels.iter().find(|b| b.name == primitive.as_str()) else {
            continue;
        };
        let pin = |role: &str| ff.port(role).and_then(|port| cell.input(port));
        let (Some(data), Some(clock), Some(enable)) = (pin("d"), pin("clk"), pin("en")) else {
            continue;
        };
        let d_port = Name::new(ff.port("d").unwrap_or("D"));
        let mut source = Source {
            data,
            clock,
            enable,
            inverted,
            name: cell.name.as_str().to_owned(),
            span: cell.span,
        };
        // Every stage but the last unless a flip-flop of another kind reads it,
        // 32 bits at a time and the rest at the end.
        let stages_in = if planned.registered {
            &chain[..]
        } else {
            &chain[..chain.len() - 1]
        };
        let mut index = 0;
        let mut at = 0;
        while at < stages_in.len() {
            let left = stages_in.len() - at;
            let depth = if left > 32 { 32 } else { left };
            let init: Vec<bool> = stages_in[at..at + depth]
                .iter()
                .map(|stage| cells[*stage].1)
                .collect();
            let net = emit(
                m,
                &source,
                index,
                (
                    u32::try_from(depth).unwrap_or(32),
                    index == 0,
                    at + depth < stages_in.len(),
                ),
                &init,
                (&long, &short),
            );
            source.data = net_expr(m, net, source.span);
            index += 1;
            at += depth;
            report.shift_registers += 1;
        }
        if planned.registered {
            // The last stage is gone, and what read it reads the shift register:
            // its `Q` net is driven by the last shift register instead.
            let Some((_, q)) = m.cells[last_cell].outputs.first().cloned() else {
                continue;
            };
            add_assign(m, q, source.data, source.span);
        } else if let Some(slot) = m.cells[last_cell]
            .inputs
            .iter_mut()
            .find(|(name, _)| *name == d_port)
        {
            // The last stage stays, and reads the last shift register.
            slot.1 = source.data;
        }
        gone.extend(stages_in.iter().map(|stage| cells[*stage].0));
        report.chains += 1;
        report.flip_flops += stages_in.len();
    }
    if !gone.is_empty() {
        m.cells.retain(|id, _| !gone.contains(&id));
    }
    Ok(report)
}
