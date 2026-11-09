//! A lookup table's inputs as the router found them.
//!
//! A 7-series slice's six-input table computes one function of whichever
//! wires reach `A1`..`A6`, and each input's own multiplexer takes only some
//! of the tile's wires. A router held to the pin the mapper named reaches
//! the others by bouncing through a bypass or fan wire, and on NewAE's
//! ChipWhisperer-Husky those were the wires the route ran out of first.
//! Vivado permutes a table's inputs as it routes; [`lut_input_classes`]
//! tells the router which of them it may (see
//! [`RouteOptions::equivalent`](crate::fpga::route::RouteOptions)), and
//! [`apply_lut_moves`] rewrites each table it moved — its truth table, its
//! connections, and the netlist's pins — so that what is written is what
//! was routed.
//!
//! An input the router may move is one that carries a signal, or one the
//! truth table does not read. One tied to a constant the table does read
//! stays where it is: a `LUT6_2`'s `I5` is tied to one so that `O6` reads
//! the upper half, and is never routed, because an unrouted input reads
//! one on this fabric — moved, that one would land on a pin a signal had
//! left and the table would read the signal instead. Nor does the second
//! of two inputs carrying one signal: the router could end the first on
//! it, and the table would have two signals on one input and one on none
//! (see [`movable_inputs`]).
//!
//! Nor does a buffer in front of a carry propagate that `legalise_carries`
//! put on `A6` ([`super::carry::ROUTE_THROUGH`]): that is Vivado's way through
//! the table, and Vivado, given an empty table and a route to another input,
//! still goes in on `A6` (`xray::vivado`).

use std::collections::BTreeMap;

use crate::fpga::arch::{NodeId, RoutingGraph};
use crate::fpga::place::{Netlist, Placement};
use crate::fpga::route::Routing;
use crate::ir::{AttrValue, Const, Design, ModuleId};

/// The roles of a table's inputs, `i0` (`A1`) first.
const INPUTS: [&str; 6] = ["i0", "i1", "i2", "i3", "i4", "i5"];

/// The truth table of a placed lookup table, when it is the 64 bits a
/// six-input table's is: every table Reticle emits on this family is a
/// `LUT6` or a `LUT6_2` (`techcells` widens the narrower ones).
fn truth_table(
    design: &Design,
    top: ModuleId,
    netlist: &Netlist,
    instance: usize,
) -> Option<Const> {
    let cell = design.modules[top]
        .cells
        .get(netlist.instances[instance].cell)?;
    match cell.params.get("INIT") {
        Some(AttrValue::Const(init)) if init.width() == 64 => Some(init.clone()),
        _ => None,
    }
}

/// Whether `init` reads input `k`: whether some index's two halves along
/// `k` differ.
fn reads(init: &Const, k: u32) -> bool {
    (0..64u32)
        .filter(|i| i & (1 << k) == 0)
        .any(|i| init.bit(i) != init.bit(i | (1 << k)))
}

/// Which inputs of `instance` the router may move: one that carries a
/// signal, or one its truth table does not read.
fn movable(design: &Design, top: ModuleId, netlist: &Netlist, instance: usize) -> [bool; 6] {
    let Some(init) = truth_table(design, top, netlist, instance) else {
        return [false; 6];
    };
    let signals = INPUTS.map(|role| {
        netlist.instances[instance]
            .pins
            .iter()
            .map(|p| &netlist.pins[*p])
            .find(|p| !p.output && p.role == role)
            .map(|p| p.signal)
    });
    movable_inputs(&init, &signals)
}

/// [`movable`], of a table whose truth table is `init` and whose input `k`
/// carries `signals[k]`: `None` for no pin, `Some(None)` for a constant.
///
/// **A signal on two inputs moves on the first alone.** The router ends
/// each sink on a node of its class that no other signal holds and that is
/// not already in its own tree; the other input of the same signal, not yet
/// routed, is neither, so the router could end the first sink there, and
/// then route the second to its own node, already reached. Two sinks on one
/// input, and nothing for [`apply_lut_moves`] to follow (`sdp_65536x4`,
/// whose table read its read multiplexer's select twice). Kept where it is,
/// the second input is in no class and nothing ends on it.
fn movable_inputs(init: &Const, signals: &[Option<Option<usize>>; 6]) -> [bool; 6] {
    let mut out = [false; 6];
    for (k, signal) in signals.iter().enumerate() {
        out[k] = match signal {
            Some(Some(signal)) => !signals[..k].contains(&Some(Some(*signal))),
            Some(None) | None => !reads(init, u32::try_from(k).unwrap_or(0)),
        };
    }
    out
}

/// The router's classes of interchangeable sink nodes
/// ([`RouteOptions::equivalent`](crate::fpga::route::RouteOptions)): each
/// placed lookup table's movable inputs, as the table's site index.
#[must_use]
pub fn lut_input_classes(
    design: &Design,
    top: ModuleId,
    netlist: &Netlist,
    graph: &RoutingGraph,
    placement: &Placement,
) -> Vec<u32> {
    let mut out = vec![u32::MAX; graph.nodes.len()];
    for (instance, inst) in netlist.instances.iter().enumerate() {
        if inst.kind != "lut" {
            continue;
        }
        let Some(site) = placement.site_of(instance) else {
            continue;
        };
        // A buffer kept on `A6`, a way through to a carry propagate as
        // Vivado's is: see `CarryOptions::buffers_on_a6`.
        if design.modules[top]
            .cells
            .get(inst.cell)
            .is_some_and(|c| c.attrs.get(super::carry::ROUTE_THROUGH).is_some())
        {
            continue;
        }
        let free = movable(design, top, netlist, instance);
        // A class of one is no choice.
        if free.iter().filter(|f| **f).count() < 2 {
            continue;
        }
        let class = u32::try_from(site).unwrap_or(u32::MAX);
        for (k, role) in INPUTS.iter().enumerate() {
            if free[k]
                && let Some(node) = graph.sites[site].pin(role)
            {
                out[node as usize] = class;
            }
        }
    }
    out
}

/// `init` with input `k` moved to input `to[k]`: the table that reads at
/// index `m` what `init` read at the index whose bit `k` is bit `to[k]` of
/// `m`.
fn permute(init: &Const, to: &[u32; 6]) -> Const {
    let mut out = Const::zero(64);
    for m in 0..64u32 {
        let mut n = 0u32;
        for (k, j) in to.iter().enumerate() {
            if m & (1 << j) != 0 {
                n |= 1 << k;
            }
        }
        out.set_bit(m, init.bit(n));
    }
    out
}

/// Rewrites every lookup table a sink of `routing` ended on another input
/// of ([`Route::moved`](crate::fpga::route::Route)): its truth table and
/// its connections in `design`, and its pins' roles in `netlist`, so that
/// the pin each signal is named on is the pin it was routed to. Returns
/// how many tables it rewrote.
///
/// # Errors
///
/// When a move lands somewhere [`lut_input_classes`] did not offer, or two
/// signals land on one input: the routing does not say what the table
/// computes, and nothing should be written from it.
pub fn apply_lut_moves(
    design: &mut Design,
    top: ModuleId,
    netlist: &mut Netlist,
    graph: &RoutingGraph,
    placement: &Placement,
    routing: &Routing,
) -> Result<usize, String> {
    // Per table, each moved input's new position.
    let mut moves: BTreeMap<usize, Vec<(usize, u32)>> = BTreeMap::new();
    for signal in 0..netlist.signals.len() {
        let Some(route) = routing.route(signal) else {
            continue;
        };
        for (pin, node) in &route.moved {
            let instance = netlist.pins[*pin].instance;
            let to = placement.site_of(instance).and_then(|site| {
                graph.sites[site]
                    .pins
                    .iter()
                    .find(|(_, n)| n == node)
                    .and_then(|(role, _)| INPUTS.iter().position(|r| r == role))
            });
            let Some(to) = to else {
                return Err(format!(
                    "`{}` was routed to {}, which is not one of its inputs",
                    netlist.instances[instance].name,
                    graph.wire(*node as NodeId).full_name()
                ));
            };
            moves
                .entry(instance)
                .or_default()
                .push((*pin, u32::try_from(to).unwrap_or(0)));
        }
    }
    for (instance, moved) in &moves {
        let name = netlist.instances[*instance].name.clone();
        let free = movable(design, top, netlist, *instance);
        let Some(init) = truth_table(design, top, netlist, *instance) else {
            return Err(format!(
                "`{name}` was given other inputs but has no truth table to follow them"
            ));
        };
        // Input k of the table as mapped goes to input to[k]: a moved
        // signal where it went, an unmoved one and a fixed one where it
        // was, and the free inputs left over onto the inputs left over.
        let mut to: [Option<u32>; 6] = [None; 6];
        let pins: Vec<usize> = netlist.instances[*instance]
            .pins
            .iter()
            .copied()
            .filter(|p| !netlist.pins[*p].output)
            .collect();
        for pin in &pins {
            let Some(k) = INPUTS.iter().position(|r| *r == netlist.pins[*pin].role) else {
                continue;
            };
            if let Some((_, j)) = moved.iter().find(|(p, _)| p == pin) {
                to[k] = Some(*j);
            } else if netlist.pins[*pin].signal.is_some() || !free[k] {
                to[k] = Some(u32::try_from(k).unwrap_or(0));
            }
        }
        for (k, slot) in to.iter_mut().enumerate() {
            if !free[k] {
                *slot = Some(u32::try_from(k).unwrap_or(0));
            }
        }
        let mut taken = [false; 6];
        for j in to.iter().flatten() {
            if std::mem::replace(&mut taken[*j as usize], true) {
                return Err(format!("`{name}` has two signals routed to one input"));
            }
        }
        for slot in &mut to {
            if slot.is_none() {
                let j = (0..6).find(|j| !taken[*j]).unwrap_or(0);
                taken[j] = true;
                *slot = Some(u32::try_from(j).unwrap_or(0));
            }
        }
        let to: [u32; 6] = to.map(|j| j.unwrap_or(0));

        // The design: the truth table, and each connection on its new port.
        let cell_id = netlist.instances[*instance].cell;
        let cell = &mut design.modules[top].cells[cell_id];
        let ports: Vec<String> = (0..6).map(|k| format!("I{k}")).collect();
        let mut inputs = Vec::with_capacity(cell.inputs.len());
        for (port, expr) in &cell.inputs {
            match ports.iter().position(|p| p == port.as_str()) {
                Some(k) => inputs.push((crate::ir::Name::new(&ports[to[k] as usize]), *expr)),
                None => inputs.push((port.clone(), *expr)),
            }
        }
        cell.inputs = inputs;
        cell.params
            .set("INIT", AttrValue::Const(permute(&init, &to)));
        // The netlist: each input pin under its new role and port.
        for pin in &pins {
            if let Some(k) = INPUTS.iter().position(|r| *r == netlist.pins[*pin].role) {
                let j = to[k] as usize;
                netlist.pins[*pin].role = INPUTS[j].to_owned();
                netlist.pins[*pin].port = ports[j].clone();
            }
        }
    }
    Ok(moves.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::Bit;

    /// **Moving inputs moves the truth table with them.** `a & ~b` on
    /// `I0`, `I1`, with `I0` routed to `A4` and `I1` to `A1`: the new
    /// table must compute the same function of the same signals at their
    /// new inputs, at every one of the 64 indices, and an input it does
    /// not read stays unread. Would catch the permutation applied
    /// backwards (`init[to[m]]` for `init[from[m]]`), which a symmetric
    /// function such as `a & b` would not.
    #[test]
    fn a_permuted_table_computes_the_same_function_of_the_moved_inputs() {
        let mut init = Const::zero(64);
        for i in 0..64u32 {
            if i & 1 == 1 && i & 2 == 0 {
                init.set_bit(i, Bit::One);
            }
        }
        // I0 -> A4 (index 3), I1 -> A1 (index 0), the rest closing up.
        let to = [3, 0, 1, 2, 4, 5];
        let moved = permute(&init, &to);
        for m in 0..64u32 {
            let a = m & (1 << 3) != 0;
            let b = m & 1 != 0;
            assert_eq!(moved.bit(m) == Bit::One, a && !b, "at index {m}");
        }
        assert!(reads(&moved, 3) && reads(&moved, 0));
        assert!(!reads(&moved, 1) && !reads(&moved, 5));
    }

    /// **A signal on two inputs is offered to the router on the first
    /// alone.** `sdp_65536x4`'s table, as it reached the router before
    /// `srl` folded it (`0xf00f99990ff06666`, the read multiplexer's select
    /// on `I4` and `I5` both): `I5` must stay where it is, every other
    /// signal move, and a constant the table reads stay too. Would catch
    /// the second input offered, with which the router once ended `I4` on
    /// `A6` and `I5` on its own `A6` (`` `$lut4` has two signals routed to
    /// one input ``); would not catch the router doing that to inputs of
    /// two different signals, which its own rule (a node another signal
    /// holds) is for.
    #[test]
    fn a_signal_on_two_inputs_moves_on_the_first_alone() {
        let mut init = Const::zero(64);
        let table: u64 = 0xf00f_9999_0ff0_6666;
        for i in 0..64u32 {
            if table >> i & 1 == 1 {
                init.set_bit(i, Bit::One);
            }
        }
        let signals = [0, 1, 2, 3, 4, 4].map(|s| Some(Some(s)));
        assert_eq!(
            movable_inputs(&init, &signals),
            [true, true, true, true, true, false]
        );
        // The same signal first on I1 and again on I3: I3 stays.
        let signals = [0, 7, 2, 7, 4, 5].map(|s| Some(Some(s)));
        assert_eq!(
            movable_inputs(&init, &signals),
            [true, true, true, false, true, true]
        );
        // A constant the table reads stays; one on no pin it does not read
        // moves.
        let mut signals = [0, 1, 2, 3, 4, 5].map(|s| Some(Some(s)));
        signals[2] = Some(None);
        assert_eq!(
            movable_inputs(&init, &signals),
            [true, true, false, true, true, true]
        );
        let mut and = Const::zero(64);
        for i in (0..64u32).filter(|i| i & 3 == 3) {
            and.set_bit(i, Bit::One);
        }
        let signals = [Some(Some(0)), Some(Some(1)), None, None, None, None];
        assert_eq!(
            movable_inputs(&and, &signals),
            [true, true, true, true, true, true]
        );
    }
}
