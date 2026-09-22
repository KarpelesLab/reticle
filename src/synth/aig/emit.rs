//! Writing a mapped network back into a module.
//!
//! Every mapper produces a [`Netlist`]: a list of single-output nodes over
//! [`Signal`]s (constants, AIG inputs, or other nodes) with one cell kind
//! each, plus one signal per AIG output. [`write_back`] splices such a
//! netlist into the module the AIG came from, using the [`Mapping`] from
//! [`super::blast`]: the absorbed cells and assignments are removed, one
//! single-bit wire is created per node (except where a node directly
//! drives a single-bit output net, which it then drives without an
//! intermediate wire), every output net gets its driver (a cell, a
//! constant, an input bit, or a concatenation of node wires for a
//! multi-bit net), and nets nothing refers to any more are dropped. Ports
//! keep their nets.
//!
//! [`to_module`] is the identity mapper: it writes an AIG back as
//! single-bit `And` and `Not` cells, which is how an optimised AIG
//! round-trips into the IR without technology mapping.

use std::collections::{HashMap, HashSet};

use super::blast::{BitRef, Mapping};
use super::{Aig, Edge, NodeKind};
use crate::ir::attr::Attrs;
use crate::ir::cell::{Cell, CellKind};
use crate::ir::design::{Assign, Module, Net, NetId, NetKind};
use crate::ir::expr::{Expr, ExprId, ExprKind};
use crate::ir::process::Lvalue;
use crate::ir::types::{Const, Type};
use crate::ir::{CellId, Name};

/// A signal in a mapped network.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Signal {
    /// A constant.
    Const(bool),
    /// The `n`-th AIG input (see [`Mapping::inputs`]).
    Input(u32),
    /// The output of node `n` of the netlist.
    Node(u32),
}

/// One cell of a mapped network.
#[derive(Clone, Debug)]
pub struct NetNode {
    /// The cell to instantiate.
    pub kind: CellKind,
    /// Its input ports, in order; ignored for `Lut`, whose single port `a`
    /// takes every fanin packed with fanin 0 as the LSB.
    pub ports: Vec<Name>,
    /// The fanins, one per port (or per LUT input).
    pub fanins: Vec<Signal>,
    /// Attributes for the cell (`lib_cell` for gates).
    pub attrs: Attrs,
    /// A short prefix for generated names (`lut`, `g`, `and`).
    pub prefix: &'static str,
}

/// A mapped network over the AIG inputs.
#[derive(Clone, Debug, Default)]
pub struct Netlist {
    /// The cells, each single-output, fanins before fanouts.
    pub nodes: Vec<NetNode>,
    /// One signal per AIG output, in AIG output order.
    pub outputs: Vec<Signal>,
}

/// Writes an AIG back into `module` as `And` / `Not` cells.
pub fn to_module(aig: &Aig, mapping: &Mapping, module: &mut Module) {
    let netlist = aig_netlist(aig);
    write_back(&netlist, mapping, module);
}

/// The AIG as a netlist of `And` and `Not` cells.
pub fn aig_netlist(aig: &Aig) -> Netlist {
    let live = aig.live_nodes();
    let mut netlist = Netlist::default();
    // Plain signal per AIG node and, lazily, an inverter per node.
    let mut plain: Vec<Option<Signal>> = vec![None; aig.len()];
    let mut inverted: Vec<Option<Signal>> = vec![None; aig.len()];
    plain[0] = Some(Signal::Const(false));
    inverted[0] = Some(Signal::Const(true));
    for (pos, &pi) in aig.inputs().iter().enumerate() {
        plain[pi as usize] = Some(Signal::Input(u32::try_from(pos).expect("input index")));
    }
    let signal_of = |netlist: &mut Netlist,
                     inverted: &mut Vec<Option<Signal>>,
                     plain: &[Option<Signal>],
                     e: Edge|
     -> Signal {
        let p = plain[e.index()].expect("fanin emitted");
        if !e.is_complement() {
            return p;
        }
        if let Some(s) = inverted[e.index()] {
            return s;
        }
        let s = match p {
            Signal::Const(c) => Signal::Const(!c),
            _ => {
                netlist.nodes.push(NetNode {
                    kind: CellKind::Not,
                    ports: vec![Name::new("a")],
                    fanins: vec![p],
                    attrs: Attrs::new(),
                    prefix: "not",
                });
                Signal::Node(u32::try_from(netlist.nodes.len() - 1).expect("node index"))
            }
        };
        inverted[e.index()] = Some(s);
        s
    };
    for id in 0..aig.len() {
        if !live[id] || aig.node(u32::try_from(id).expect("index")).kind != NodeKind::And {
            continue;
        }
        let (a, b) = aig.fanins(u32::try_from(id).expect("index"));
        let sa = signal_of(&mut netlist, &mut inverted, &plain, a);
        let sb = signal_of(&mut netlist, &mut inverted, &plain, b);
        netlist.nodes.push(NetNode {
            kind: CellKind::And,
            ports: vec![Name::new("a"), Name::new("b")],
            fanins: vec![sa, sb],
            attrs: Attrs::new(),
            prefix: "and",
        });
        plain[id] = Some(Signal::Node(
            u32::try_from(netlist.nodes.len() - 1).expect("node index"),
        ));
    }
    for &o in aig.outputs() {
        let s = signal_of(&mut netlist, &mut inverted, &plain, o);
        netlist.outputs.push(s);
    }
    netlist
}

/// Splices `netlist` into `module` in place of the logic `mapping`
/// absorbed.
///
/// The module's nets and expressions are renumbered on the way out (the
/// ones the mapping made redundant are dropped), so `mapping` describes
/// the module as it was and must not be used against it again.
pub fn write_back(netlist: &Netlist, mapping: &Mapping, module: &mut Module) {
    let span = module.span;
    // 1. Remove the absorbed cells and assignments.
    let absorbed_cells: HashSet<CellId> = mapping.absorbed_cells.iter().copied().collect();
    if !absorbed_cells.is_empty() {
        module.cells.retain(|id, _| !absorbed_cells.contains(&id));
    }
    let absorbed_assigns: HashSet<usize> = mapping.absorbed_assigns.iter().copied().collect();
    if !absorbed_assigns.is_empty() {
        let mut index = 0usize;
        module.assigns.retain(|_| {
            let keep = !absorbed_assigns.contains(&index);
            index += 1;
            keep
        });
    }

    // 2. Group output bits by net.
    let mut per_net: Vec<(NetId, Vec<Option<Signal>>)> = Vec::new();
    let mut slot_of: HashMap<NetId, usize> = HashMap::new();
    for (i, bit) in mapping.outputs.iter().enumerate() {
        let slot = *slot_of.entry(bit.net).or_insert_with(|| {
            let width = module.nets[bit.net].ty.width().unwrap_or(0);
            per_net.push((bit.net, vec![None; width as usize]));
            per_net.len() - 1
        });
        if let Some(s) = per_net[slot].1.get_mut(bit.bit as usize) {
            *s = Some(netlist.outputs[i]);
        }
    }

    // 3. Decide which nodes directly drive a single-bit output net.
    let mut direct: Vec<Option<NetId>> = vec![None; netlist.nodes.len()];
    let mut claimed: HashSet<u32> = HashSet::new();
    for (net, bits) in &per_net {
        if let [Some(Signal::Node(n))] = bits.as_slice()
            && claimed.insert(*n)
        {
            direct[*n as usize] = Some(*net);
        }
    }

    // 4. Names.
    let mut namer = Namer::new(module);

    // 5. Create node nets and cells.
    let mut node_net: Vec<Option<NetId>> = vec![None; netlist.nodes.len()];
    let mut input_expr: HashMap<u32, ExprId> = HashMap::new();
    let mut net_expr: HashMap<NetId, ExprId> = HashMap::new();
    for (i, node) in netlist.nodes.iter().enumerate() {
        let net = match direct[i] {
            Some(net) => net,
            None => {
                let name = namer.net(node.prefix);
                module.nets.push(Net {
                    name,
                    ty: Type::bit(),
                    kind: NetKind::Wire,
                    attrs: Attrs::new(),
                    span,
                })
            }
        };
        node_net[i] = Some(net);
    }
    let mut signal_expr = |module: &mut Module, s: Signal| -> ExprId {
        match s {
            Signal::Const(c) => module.add_expr(Expr::new(
                ExprKind::Const(Const::from_bool(c)),
                Type::bit(),
                span,
            )),
            Signal::Input(pos) => *input_expr.entry(pos).or_insert_with(|| {
                let BitRef { net, bit } = mapping.inputs[pos as usize];
                bit_expr(module, net, bit, span)
            }),
            Signal::Node(n) => {
                let net = node_net[n as usize].expect("node net");
                *net_expr.entry(net).or_insert_with(|| {
                    module.add_expr(Expr::new(ExprKind::Net(net), Type::bit(), span))
                })
            }
        }
    };
    for (i, node) in netlist.nodes.iter().enumerate() {
        let out = node_net[i].expect("node net");
        let inputs: Vec<(Name, ExprId)> = if let CellKind::Lut { k, .. } = &node.kind {
            let parts: Vec<ExprId> = node
                .fanins
                .iter()
                .rev()
                .map(|&s| signal_expr(module, s))
                .collect();
            let packed = if parts.len() == 1 {
                parts[0]
            } else {
                module.add_expr(Expr::new(ExprKind::Concat(parts), Type::bits(*k), span))
            };
            vec![(Name::new("a"), packed)]
        } else {
            node.ports
                .iter()
                .zip(&node.fanins)
                .map(|(p, &s)| (p.clone(), signal_expr(module, s)))
                .collect()
        };
        let name = namer.cell(node.prefix);
        module.cells.push(Cell {
            name,
            kind: node.kind.clone(),
            inputs,
            outputs: vec![(Name::new("y"), out)],
            params: Attrs::new(),
            attrs: node.attrs.clone(),
            span,
        });
    }

    // 6. Drive the output nets that are not driven directly.
    for (net, bits) in per_net {
        if let [Some(Signal::Node(n))] = bits.as_slice()
            && direct[*n as usize] == Some(net)
        {
            continue;
        }
        let width = bits.len();
        let all_const = bits
            .iter()
            .all(|b| matches!(b, Some(Signal::Const(_)) | None));
        let value = if all_const {
            let mut c = Const::zero(u32::try_from(width).expect("width"));
            for (i, b) in bits.iter().enumerate() {
                if let Some(Signal::Const(true)) = b {
                    c.set_bit(u32::try_from(i).expect("bit"), crate::logic::Bit::One);
                }
            }
            module.add_expr(Expr::new(
                ExprKind::Const(c),
                Type::bits(u32::try_from(width).expect("width")),
                span,
            ))
        } else if width == 1 {
            signal_expr(module, bits[0].unwrap_or(Signal::Const(false)))
        } else {
            let parts: Vec<ExprId> = bits
                .iter()
                .rev()
                .map(|b| signal_expr(module, b.unwrap_or(Signal::Const(false))))
                .collect();
            module.add_expr(Expr::new(
                ExprKind::Concat(parts),
                Type::bits(u32::try_from(width).expect("width")),
                span,
            ))
        };
        module.assigns.push(Assign {
            target: Lvalue::Net(net),
            value,
            delay: None,
            attrs: Attrs::new(),
            span,
        });
    }

    // 7. Drop what vanished.
    module.remove_unused_nets();
    module.gc_exprs();
}

/// An expression reading one bit of a net.
fn bit_expr(module: &mut Module, net: NetId, bit: u32, span: crate::source::Span) -> ExprId {
    let ty = module.nets[net].ty.clone();
    let base = module.add_expr(Expr::new(ExprKind::Net(net), ty.clone(), span));
    if ty.width() == Some(1) {
        return base;
    }
    module.add_expr(Expr::new(
        ExprKind::Slice {
            base,
            hi: bit,
            lo: bit,
        },
        Type::bit(),
        span,
    ))
}

/// Generates names that do not collide with the module's.
struct Namer {
    nets: HashSet<String>,
    cells: HashSet<String>,
    counter: HashMap<&'static str, u32>,
}

impl Namer {
    fn new(module: &Module) -> Namer {
        Namer {
            nets: module
                .nets
                .values()
                .map(|n| n.name.as_str().to_owned())
                .collect(),
            cells: module
                .cells
                .values()
                .map(|c| c.name.as_str().to_owned())
                .collect(),
            counter: HashMap::new(),
        }
    }

    fn fresh(&mut self, prefix: &'static str, kind: char) -> Name {
        loop {
            let n = self.counter.entry(prefix).or_insert(0);
            let name = format!("${prefix}{n}");
            *n += 1;
            let set = if kind == 'n' {
                &mut self.nets
            } else {
                &mut self.cells
            };
            if set.insert(name.clone()) {
                return Name::new(name);
            }
        }
    }

    fn net(&mut self, prefix: &'static str) -> Name {
        self.fresh(prefix, 'n')
    }

    fn cell(&mut self, prefix: &'static str) -> Name {
        self.fresh(prefix, 'c')
    }
}

#[cfg(test)]
mod tests {
    use super::super::blast::from_module;
    use super::*;
    use crate::ir::builder::ModuleBuilder;
    use crate::ir::validate::validate_module;
    use crate::source::{SourceMap, Span};

    fn span() -> Span {
        let mut map = SourceMap::new();
        let id = map.add("t", "").unwrap();
        Span::new(id, 0, 0)
    }

    #[test]
    fn round_trips_as_and_not_cells() {
        let mut b = ModuleBuilder::new("m", span());
        let a = b.input("a", Type::bits(2));
        let c = b.input("c", Type::bit());
        let y = b.output("y", Type::bits(2));
        let z = b.output("z", Type::bit());
        let k = b.output("k", Type::bits(2));
        let (an, cn) = (b.net(a), b.net(c));
        let rep = b.replicate(2, cn);
        let x = b.xor(an, rep);
        b.assign(y, x);
        let r = b.reduce_and(an);
        b.assign(z, r);
        let one = b.const_u64(2, 2);
        b.assign(k, one);
        let m = b.finish();
        let (aig, mapping) = from_module(&m);
        let mut mapped = m.clone();
        to_module(&aig, &mapping, &mut mapped);
        assert!(validate_module(&mapped).is_empty(), "{}", {
            let mut d = crate::ir::Design::new();
            d.add_module(mapped.clone());
            d.to_text()
        });
        assert!(
            mapped
                .cells
                .values()
                .all(|c| matches!(c.kind, CellKind::And | CellKind::Not))
        );
        assert_eq!(mapped.ports.len(), 5);
        // `z` is single-bit and driven directly by a cell; `y` by a concat.
        let z_net = mapped.net_by_name("z").unwrap();
        assert!(mapped.cells.values().any(|c| c.output("y") == Some(z_net)));
        assert!(
            mapped
                .assigns
                .iter()
                .any(|a| a.target == Lvalue::Net(mapped.net_by_name("y").unwrap()))
        );
        let k_net = mapped.net_by_name("k").unwrap();
        let k_assign = mapped
            .assigns
            .iter()
            .find(|a| a.target == Lvalue::Net(k_net))
            .unwrap();
        assert_eq!(
            mapped.exprs[k_assign.value].as_const(),
            Some(&Const::from_u64(2, 2))
        );
    }
}
