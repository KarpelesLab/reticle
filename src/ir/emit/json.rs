//! Yosys JSON netlist emission.
//!
//! The output follows the shape `write_json` produces, so nextpnr,
//! netlistsvg and other consumers of Yosys netlists accept it:
//!
//! ```text
//! { "creator": "reticle",
//!   "modules": { "<name>": {
//!     "attributes": { ... },
//!     "parameter_default_values": { ... },
//!     "ports": { "<name>": { "direction": "input", "bits": [ 2, 3 ] } },
//!     "cells": { "<name>": { "hide_name": 0, "type": "$add", "parameters": { ... },
//!                            "attributes": { ... }, "port_directions": { ... },
//!                            "connections": { "A": [ 2, 3 ], ... } } },
//!     "memories": { "<name>": { "hide_name": 0, "attributes": { ... },
//!                               "width": 8, "start_offset": 0, "size": 16 } },
//!     "netnames": { "<name>": { "hide_name": 0, "bits": [ 2, 3 ], "attributes": { ... } } } } } }
//! ```
//!
//! Bits are global net-bit numbers starting at 2 (0 and 1 are left unused
//! as Yosys does), constants are the strings `"0"`, `"1"`, `"x"`, `"z"`.
//! Continuous assignments of structural expressions are folded into the
//! bit numbering (see [`super::BitView`]); anything else, and any process,
//! is an error: the JSON format is for synthesised designs.
//!
//! # Cell types
//!
//! | IR cell                     | Yosys type                                              | Parameters                                                |
//! |-----------------------------|---------------------------------------------------------|-----------------------------------------------------------|
//! | `Not`, `Buf`                | `$not`, `$pos`                                          | `A_SIGNED`, `A_WIDTH`, `Y_WIDTH`                          |
//! | `And` … `Ge`                | `$and` `$or` `$xor` `$add` `$sub` `$mul` `$div` `$mod` `$shl` `$shr` `$sshr` `$eq` `$ne` `$lt` `$le` `$gt` `$ge` | `A_SIGNED`, `B_SIGNED`, `A_WIDTH`, `B_WIDTH`, `Y_WIDTH` |
//! | `ReduceAnd` `ReduceOr` `ReduceXor` | `$reduce_and` `$reduce_or` `$reduce_xor`         | `A_SIGNED`, `A_WIDTH`, `Y_WIDTH`                          |
//! | `Mux`, `Pmux`               | `$mux`, `$pmux`                                         | `WIDTH` (+ `S_WIDTH`)                                     |
//! | `Dff`                       | `$dff` `$dffe` `$adff` `$adffe` `$sdff` `$sdffe`        | `WIDTH`, `CLK_POLARITY`, `EN_POLARITY`, `ARST_POLARITY` / `SRST_POLARITY`, `ARST_VALUE` / `SRST_VALUE` |
//! | `Dlatch`                    | `$dlatch`                                               | `WIDTH`, `EN_POLARITY`                                    |
//! | `MemRdPort`, `MemWrPort`    | `$memrd`, `$memwr`                                      | `MEMID`, `ABITS`, `WIDTH`, `CLK_ENABLE`, `CLK_POLARITY`, `TRANSPARENT` / `PRIORITY` |
//! | `Lut`                       | `$lut`                                                  | `WIDTH`, `LUT`                                            |
//! | `Tristate`                  | `$tribuf`                                               | `WIDTH`                                                   |
//! | `Blackbox(name)`, instances | the name                                                | the cell's / instance's parameters                        |
//!
//! Parameter and attribute values are numbers when they are two-state and
//! at most 32 bits wide, bit strings (`"0x1z"`) otherwise; a string value
//! that looks like a bit string gets a trailing space, as Yosys does.
//! Memory contents are not emitted.

use super::super::attr::{AttrValue, Attrs};
use super::super::cell::{Cell, CellKind};
use super::super::design::{Design, Module, ModuleRef, PortDir};
use super::super::expr::ExprId;
use super::super::types::Const;
use super::{BitView, EmitError, Out, SigBit, json_string, require_cell_form};
use crate::logic::Bit;

/// Renders every module of `design`.
pub fn emit_json(design: &Design) -> Result<String, EmitError> {
    let mut out = Out::new("  ");
    out.line("{");
    out.indent();
    out.line("\"creator\": \"reticle\",");
    out.line("\"modules\": {");
    out.indent();
    let count = design.modules.len();
    for (i, (id, module)) in design.modules.iter().enumerate() {
        require_cell_form(module, "JSON")?;
        let view = BitView::new(module)?;
        let mut numbers = vec![None; view.slots()];
        let mut next = 2u64;
        for (slot, number) in numbers.iter_mut().enumerate() {
            if view.is_representative(slot) {
                *number = Some(next);
                next += 1;
            }
        }
        let printer = ModulePrinter {
            design,
            module,
            view: &view,
            numbers: &numbers,
        };
        out.line(&format!("{}: {{", json_string(module.name.as_str())));
        out.indent();
        printer.module(&mut out, design.top == Some(id))?;
        out.dedent();
        out.line(if i + 1 < count { "}," } else { "}" });
    }
    out.dedent();
    out.line("}");
    out.dedent();
    out.line("}");
    Ok(out.finish())
}

/// The JSON value of an attribute or parameter.
fn value_json(v: &AttrValue) -> String {
    match v {
        AttrValue::Int(i) => i.to_string(),
        AttrValue::String(s) => {
            let looks_binary = !s.is_empty()
                && s.chars()
                    .all(|c| matches!(c, '0' | '1' | 'x' | 'z' | 'X' | 'Z' | '-'));
            if looks_binary {
                json_string(&format!("{s} "))
            } else {
                json_string(s)
            }
        }
        AttrValue::Const(c) => const_json(c),
    }
}

fn const_json(c: &Const) -> String {
    match c.to_u64() {
        Some(v) if c.width() <= 32 => v.to_string(),
        _ => json_string(&c.to_binary_string()),
    }
}

fn attrs_json(out: &mut Out, key: &str, attrs: &Attrs, last: bool) {
    let comma = if last { "" } else { "," };
    if attrs.is_empty() {
        out.line(&format!("\"{key}\": {{}}{comma}"));
        return;
    }
    out.line(&format!("\"{key}\": {{"));
    out.indent();
    let n = attrs.len();
    for (i, (k, v)) in attrs.iter().enumerate() {
        let sep = if i + 1 < n { "," } else { "" };
        out.line(&format!(
            "{}: {}{sep}",
            json_string(k.as_str()),
            value_json(v)
        ));
    }
    out.dedent();
    out.line(&format!("}}{comma}"));
}

fn hide_name(name: &str) -> u8 {
    u8::from(name.starts_with('$'))
}

/// A cell's rendering: type, parameters, port directions and connections.
struct CellJson {
    ty: String,
    params: Vec<(String, String)>,
    /// (port, direction, bits); the direction is `None` when unknown.
    ports: Vec<(String, Option<&'static str>, Vec<SigBit>)>,
}

struct ModulePrinter<'a> {
    design: &'a Design,
    module: &'a Module,
    view: &'a BitView<'a>,
    numbers: &'a [Option<u64>],
}

impl ModulePrinter<'_> {
    fn bits_json(&self, bits: &[SigBit]) -> String {
        let items: Vec<String> = bits
            .iter()
            .map(|b| match self.view.canonical(*b) {
                SigBit::Const(Bit::Zero) => "\"0\"".to_owned(),
                SigBit::Const(Bit::One) => "\"1\"".to_owned(),
                SigBit::Const(Bit::X) => "\"x\"".to_owned(),
                SigBit::Const(Bit::Z) => "\"z\"".to_owned(),
                SigBit::Slot(s) => self.numbers[s].map_or("\"x\"".to_owned(), |n| n.to_string()),
            })
            .collect();
        if items.is_empty() {
            "[]".to_owned()
        } else {
            format!("[ {} ]", items.join(", "))
        }
    }

    fn module(&self, out: &mut Out, top: bool) -> Result<(), EmitError> {
        let m = self.module;
        // Attributes, with the design-level flags Yosys consumers expect.
        let mut attrs = m.attrs.clone();
        if top {
            attrs.set("top", 1);
        }
        if m.blackbox {
            attrs.set("blackbox", 1);
        }
        attrs_json(out, "attributes", &attrs, false);
        if !m.params.is_empty() {
            let params: Attrs = m
                .params
                .iter()
                .map(|p| (p.name.clone(), p.value.clone()))
                .collect();
            attrs_json(out, "parameter_default_values", &params, false);
        }
        // Ports.
        out.line("\"ports\": {");
        out.indent();
        let n = m.ports.len();
        for (i, port) in m.ports.iter().enumerate() {
            let dir = match port.dir {
                PortDir::In => "input",
                PortDir::Out => "output",
                PortDir::InOut => "inout",
            };
            let bits = self.view.net_bits(port.net, port.span)?;
            let sep = if i + 1 < n { "," } else { "" };
            out.line(&format!(
                "{}: {{ \"direction\": \"{dir}\", \"bits\": {} }}{sep}",
                json_string(port.name.as_str()),
                self.bits_json(&bits)
            ));
        }
        out.dedent();
        out.line("},");
        // Cells and instances.
        out.line("\"cells\": {");
        out.indent();
        let total = m.cells.len() + m.instances.len();
        let mut index = 0;
        for (_, cell) in m.cells.iter() {
            index += 1;
            let rendered = self.cell(cell, index)?;
            self.cell_json(
                out,
                cell.name.as_str(),
                &cell.attrs,
                rendered,
                index == total,
            );
        }
        for (_, inst) in m.instances.iter() {
            index += 1;
            let rendered = self.instance(inst)?;
            self.cell_json(
                out,
                inst.name.as_str(),
                &inst.attrs,
                rendered,
                index == total,
            );
        }
        out.dedent();
        if total > 0 {
            out.line("},");
        }
        // Memories.
        if !m.memories.is_empty() {
            out.line("\"memories\": {");
            out.indent();
            let n = m.memories.len();
            for (i, (_, mem)) in m.memories.iter().enumerate() {
                out.line(&format!("{}: {{", json_string(mem.name.as_str())));
                out.indent();
                out.line(&format!("\"hide_name\": {},", hide_name(mem.name.as_str())));
                attrs_json(out, "attributes", &mem.attrs, false);
                out.line(&format!("\"width\": {},", mem.elem.width().unwrap_or(0)));
                out.line("\"start_offset\": 0,");
                out.line(&format!("\"size\": {}", mem.size));
                out.dedent();
                out.line(if i + 1 < n { "}," } else { "}" });
            }
            out.dedent();
            out.line("},");
        }
        // Net names.
        out.line("\"netnames\": {");
        out.indent();
        let nets: Vec<_> = m.nets.iter().filter(|(_, n)| n.ty.is_bits()).collect();
        let n = nets.len();
        for (i, (id, net)) in nets.into_iter().enumerate() {
            let bits = self.view.net_bits(id, net.span)?;
            out.line(&format!("{}: {{", json_string(net.name.as_str())));
            out.indent();
            out.line(&format!("\"hide_name\": {},", hide_name(net.name.as_str())));
            out.line(&format!("\"bits\": {},", self.bits_json(&bits)));
            attrs_json(out, "attributes", &net.attrs, true);
            out.dedent();
            out.line(if i + 1 < n { "}," } else { "}" });
        }
        out.dedent();
        out.line("}");
        Ok(())
    }

    fn cell_json(&self, out: &mut Out, name: &str, attrs: &Attrs, cell: CellJson, last: bool) {
        out.line(&format!("{}: {{", json_string(name)));
        out.indent();
        out.line(&format!("\"hide_name\": {},", hide_name(name)));
        out.line(&format!("\"type\": {},", json_string(&cell.ty)));
        if cell.params.is_empty() {
            out.line("\"parameters\": {},");
        } else {
            out.line("\"parameters\": {");
            out.indent();
            let n = cell.params.len();
            for (i, (k, v)) in cell.params.iter().enumerate() {
                let sep = if i + 1 < n { "," } else { "" };
                out.line(&format!("{}: {v}{sep}", json_string(k)));
            }
            out.dedent();
            out.line("},");
        }
        attrs_json(out, "attributes", attrs, false);
        let dirs: Vec<&(String, Option<&'static str>, Vec<SigBit>)> =
            cell.ports.iter().filter(|(_, d, _)| d.is_some()).collect();
        if dirs.is_empty() {
            out.line("\"port_directions\": {},");
        } else {
            out.line("\"port_directions\": {");
            out.indent();
            let n = dirs.len();
            for (i, (port, dir, _)) in dirs.iter().enumerate() {
                let sep = if i + 1 < n { "," } else { "" };
                out.line(&format!(
                    "{}: \"{}\"{sep}",
                    json_string(port),
                    dir.expect("filtered")
                ));
            }
            out.dedent();
            out.line("},");
        }
        if cell.ports.is_empty() {
            out.line("\"connections\": {}");
        } else {
            out.line("\"connections\": {");
            out.indent();
            let n = cell.ports.len();
            for (i, (port, _, bits)) in cell.ports.iter().enumerate() {
                let sep = if i + 1 < n { "," } else { "" };
                out.line(&format!(
                    "{}: {}{sep}",
                    json_string(port),
                    self.bits_json(bits)
                ));
            }
            out.dedent();
            out.line("}");
        }
        out.dedent();
        out.line(if last { "}" } else { "}," });
    }

    fn input(&self, cell: &Cell, port: &str) -> Result<Vec<SigBit>, EmitError> {
        match cell.input(port) {
            Some(id) => self.view.expr_bits(id),
            None => Err(EmitError::new(
                cell.span,
                format!("cell `{}` has no input `{port}`", cell.name),
            )),
        }
    }

    fn output(&self, cell: &Cell, port: &str) -> Result<Vec<SigBit>, EmitError> {
        match cell.output(port) {
            Some(net) => self.view.net_bits(net, cell.span),
            None => Err(EmitError::new(
                cell.span,
                format!("cell `{}` has no output `{port}`", cell.name),
            )),
        }
    }

    fn input_signed(&self, cell: &Cell, port: &str) -> u8 {
        u8::from(
            cell.input(port)
                .is_some_and(|id| self.module.exprs[id].ty.is_signed()),
        )
    }

    fn cell(&self, cell: &Cell, index: usize) -> Result<CellJson, EmitError> {
        let num = |n: usize| n.to_string();
        let mut params: Vec<(String, String)> = Vec::new();
        let mut ports: Vec<(String, Option<&'static str>, Vec<SigBit>)> = Vec::new();
        let mut inp = |name: &str, bits: Vec<SigBit>| {
            ports.push((name.to_owned(), Some("input"), bits));
        };
        let ty = match &cell.kind {
            CellKind::Not
            | CellKind::Buf
            | CellKind::ReduceAnd
            | CellKind::ReduceOr
            | CellKind::ReduceXor => {
                let a = self.input(cell, "a")?;
                let y = self.output(cell, "y")?;
                params.push(("A_SIGNED".into(), self.input_signed(cell, "a").to_string()));
                params.push(("A_WIDTH".into(), num(a.len())));
                params.push(("Y_WIDTH".into(), num(y.len())));
                inp("A", a);
                ports.push(("Y".into(), Some("output"), y));
                match cell.kind {
                    CellKind::Not => "$not",
                    CellKind::Buf => "$pos",
                    CellKind::ReduceAnd => "$reduce_and",
                    CellKind::ReduceOr => "$reduce_or",
                    _ => "$reduce_xor",
                }
                .to_owned()
            }
            CellKind::And
            | CellKind::Or
            | CellKind::Xor
            | CellKind::Add
            | CellKind::Sub
            | CellKind::Mul
            | CellKind::Div
            | CellKind::Mod
            | CellKind::Shl
            | CellKind::Shr
            | CellKind::Sshr
            | CellKind::Eq
            | CellKind::Ne
            | CellKind::Lt
            | CellKind::Le
            | CellKind::Gt
            | CellKind::Ge => {
                let a = self.input(cell, "a")?;
                let b = self.input(cell, "b")?;
                let y = self.output(cell, "y")?;
                params.push(("A_SIGNED".into(), self.input_signed(cell, "a").to_string()));
                params.push(("B_SIGNED".into(), self.input_signed(cell, "b").to_string()));
                params.push(("A_WIDTH".into(), num(a.len())));
                params.push(("B_WIDTH".into(), num(b.len())));
                params.push(("Y_WIDTH".into(), num(y.len())));
                inp("A", a);
                inp("B", b);
                ports.push(("Y".into(), Some("output"), y));
                format!("${}", cell.kind.keyword())
            }
            CellKind::Mux => {
                let a = self.input(cell, "a")?;
                let b = self.input(cell, "b")?;
                let s = self.input(cell, "s")?;
                let y = self.output(cell, "y")?;
                params.push(("WIDTH".into(), num(y.len())));
                inp("A", a);
                inp("B", b);
                inp("S", s);
                ports.push(("Y".into(), Some("output"), y));
                "$mux".to_owned()
            }
            CellKind::Pmux => {
                let a = self.input(cell, "a")?;
                let b = self.input(cell, "b")?;
                let s = self.input(cell, "s")?;
                let y = self.output(cell, "y")?;
                params.push(("WIDTH".into(), num(y.len())));
                params.push(("S_WIDTH".into(), num(s.len())));
                inp("A", a);
                inp("B", b);
                inp("S", s);
                ports.push(("Y".into(), Some("output"), y));
                "$pmux".to_owned()
            }
            CellKind::Dff {
                clk_pos,
                has_enable,
                reset,
            } => {
                let clk = self.input(cell, "clk")?;
                let d = self.input(cell, "d")?;
                let q = self.output(cell, "q")?;
                params.push(("WIDTH".into(), num(q.len())));
                params.push(("CLK_POLARITY".into(), u8::from(*clk_pos).to_string()));
                let mut ty = String::from("$");
                inp("CLK", clk);
                match reset {
                    Some(r) if r.asynchronous => {
                        ty.push_str("adff");
                        params.push(("ARST_POLARITY".into(), u8::from(r.active_high).to_string()));
                        params.push(("ARST_VALUE".into(), const_json(&r.value)));
                        let rst = self.input(cell, "rst")?;
                        inp("ARST", rst);
                    }
                    Some(r) => {
                        ty.push_str("sdff");
                        params.push(("SRST_POLARITY".into(), u8::from(r.active_high).to_string()));
                        params.push(("SRST_VALUE".into(), const_json(&r.value)));
                        let rst = self.input(cell, "rst")?;
                        inp("SRST", rst);
                    }
                    None => ty.push_str("dff"),
                }
                if *has_enable {
                    ty.push('e');
                    params.push(("EN_POLARITY".into(), "1".into()));
                    let en = self.input(cell, "en")?;
                    inp("EN", en);
                }
                inp("D", d);
                ports.push(("Q".into(), Some("output"), q));
                ty
            }
            CellKind::Dlatch => {
                let en = self.input(cell, "en")?;
                let d = self.input(cell, "d")?;
                let q = self.output(cell, "q")?;
                params.push(("WIDTH".into(), num(q.len())));
                params.push(("EN_POLARITY".into(), "1".into()));
                inp("EN", en);
                inp("D", d);
                ports.push(("Q".into(), Some("output"), q));
                "$dlatch".to_owned()
            }
            CellKind::MemRdPort { mem, clocked } => {
                let memory = &self.module.memories[*mem];
                let addr = self.input(cell, "addr")?;
                let data = self.output(cell, "data")?;
                params.push(("MEMID".into(), json_string(&format!("\\{}", memory.name))));
                params.push(("ABITS".into(), num(addr.len())));
                params.push(("WIDTH".into(), num(data.len())));
                params.push(("CLK_ENABLE".into(), u8::from(*clocked).to_string()));
                params.push(("CLK_POLARITY".into(), "1".into()));
                params.push(("TRANSPARENT".into(), "0".into()));
                if *clocked {
                    let clk = self.input(cell, "clk")?;
                    let en = self.input(cell, "en")?;
                    inp("CLK", clk);
                    inp("EN", en);
                } else {
                    inp("CLK", vec![SigBit::Const(Bit::X)]);
                    inp("EN", vec![SigBit::Const(Bit::One)]);
                }
                inp("ADDR", addr);
                ports.push(("DATA".into(), Some("output"), data));
                "$memrd".to_owned()
            }
            CellKind::MemWrPort { mem, clocked } => {
                let memory = &self.module.memories[*mem];
                let addr = self.input(cell, "addr")?;
                let data = self.input(cell, "data")?;
                let en = self.input(cell, "en")?;
                params.push(("MEMID".into(), json_string(&format!("\\{}", memory.name))));
                params.push(("ABITS".into(), num(addr.len())));
                params.push(("WIDTH".into(), num(data.len())));
                params.push(("CLK_ENABLE".into(), u8::from(*clocked).to_string()));
                params.push(("CLK_POLARITY".into(), "1".into()));
                params.push(("PRIORITY".into(), num(index)));
                if *clocked {
                    let clk = self.input(cell, "clk")?;
                    inp("CLK", clk);
                } else {
                    inp("CLK", vec![SigBit::Const(Bit::X)]);
                }
                // Yosys write enables are per bit.
                let en_bit = en.first().copied().unwrap_or(SigBit::Const(Bit::One));
                inp("EN", vec![en_bit; data.len()]);
                inp("ADDR", addr);
                inp("DATA", data);
                "$memwr".to_owned()
            }
            CellKind::Lut { k, init } => {
                let a = self.input(cell, "a")?;
                let y = self.output(cell, "y")?;
                params.push(("WIDTH".into(), k.to_string()));
                params.push(("LUT".into(), const_json(init)));
                inp("A", a);
                ports.push(("Y".into(), Some("output"), y));
                "$lut".to_owned()
            }
            CellKind::Tristate => {
                let a = self.input(cell, "a")?;
                let en = self.input(cell, "en")?;
                let y = self.output(cell, "y")?;
                params.push(("WIDTH".into(), num(y.len())));
                inp("A", a);
                inp("EN", en);
                ports.push(("Y".into(), Some("output"), y));
                "$tribuf".to_owned()
            }
            CellKind::Blackbox(name) => {
                for (k, v) in cell.params.iter() {
                    params.push((k.as_str().to_owned(), value_json(v)));
                }
                for (port, e) in &cell.inputs {
                    let bits = self.view.expr_bits(*e)?;
                    inp(port.as_str(), bits);
                }
                for (port, net) in &cell.outputs {
                    let bits = self.view.net_bits(*net, cell.span)?;
                    ports.push((port.as_str().to_owned(), Some("output"), bits));
                }
                name.as_str().to_owned()
            }
        };
        Ok(CellJson { ty, params, ports })
    }

    fn instance(&self, inst: &super::super::design::Instance) -> Result<CellJson, EmitError> {
        let target = inst.module.id().map(|id| self.design.module(id));
        let ty = match &inst.module {
            ModuleRef::Resolved(id) => self.design.module(*id).name.as_str().to_owned(),
            ModuleRef::Unresolved(name) => name.as_str().to_owned(),
        };
        let params = inst
            .params
            .iter()
            .map(|(k, v)| (k.as_str().to_owned(), value_json(v)))
            .collect();
        let mut ports = Vec::new();
        for (port, e) in &inst.connections {
            let dir = target
                .and_then(|t| t.port(port.as_str()))
                .map(|p| match p.dir {
                    PortDir::In => "input",
                    PortDir::Out => "output",
                    PortDir::InOut => "inout",
                });
            let bits = self.bits_of(*e)?;
            ports.push((port.as_str().to_owned(), dir, bits));
        }
        Ok(CellJson { ty, params, ports })
    }

    fn bits_of(&self, e: ExprId) -> Result<Vec<SigBit>, EmitError> {
        self.view.expr_bits(e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values() {
        assert_eq!(value_json(&AttrValue::Int(3)), "3");
        assert_eq!(value_json(&AttrValue::from("block")), "\"block\"");
        assert_eq!(value_json(&AttrValue::from("0101")), "\"0101 \"");
        assert_eq!(value_json(&AttrValue::Const(Const::from_u64(9, 4))), "9");
        assert_eq!(
            value_json(&AttrValue::Const(Const::parse_verilog("4'b10xz").unwrap())),
            "\"10xz\""
        );
        assert_eq!(
            value_json(&AttrValue::Const(Const::from_u64(1, 40))),
            "\"0000000000000000000000000000000000000001\""
        );
        assert_eq!(hide_name("$auto$1"), 1);
        assert_eq!(hide_name("q"), 0);
    }
}
