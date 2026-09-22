//! BLIF (Berkeley Logic Interchange Format) emission.
//!
//! One `.model` per module, the top module first. BLIF is a bit-level
//! format, so every cell is expanded to per-bit `.names` truth tables and
//! `.latch` lines; bits of a bus net are named `net[i]`, one-bit nets keep
//! their name. Constants use the conventional `$false`, `$true` and
//! `$undef` (for `x` and `z`) nets, defined at the top of the model.
//!
//! | IR cell                        | Rendering                                                  |
//! |--------------------------------|------------------------------------------------------------|
//! | `Not` `Buf` `And` `Or` `Xor`   | one `.names` per bit                                       |
//! | `Mux`, `Pmux`                  | one `.names` per bit (one-hot select for `Pmux`)           |
//! | `ReduceAnd` `ReduceOr`         | one `.names`                                               |
//! | `ReduceXor`                    | a chain of two-input `.names`                              |
//! | `Lut`                          | one `.names` listing the minterms of `init`                |
//! | `Dff`                          | `.latch d q re|fe clk init`; a synchronous reset or enable becomes a `.names` in front of the latch; asynchronous resets are an error |
//! | `Dlatch`                       | `.latch d q ah en init`                                    |
//! | `Blackbox`, instances          | `.subckt name port[i]=bit ...`                             |
//! | arithmetic, comparisons, shifts, memory ports, `Tristate` | error: map to gates or LUTs first |
//!
//! The initial value of a latch comes from an `init` attribute on the
//! output net when it is a constant, and is `2` (don't care) otherwise.
//! Structural continuous assignments are folded into the bit naming; an
//! output port whose bits are aliases of other nets gets `.names` buffers
//! so the port keeps its name.

use std::fmt::Write as _;

use super::super::attr::AttrValue;
use super::super::cell::{Cell, CellKind};
use super::super::design::{Design, Module, ModuleId, ModuleRef, PortDir};
use super::{BitView, EmitError, Out, SigBit, require_cell_form};
use crate::logic::Bit;

/// Renders every module of `design`, the top module first.
pub fn emit_blif(design: &Design) -> Result<String, EmitError> {
    let mut order: Vec<ModuleId> = Vec::new();
    if let Some(top) = design.top {
        order.push(top);
    }
    order.extend(design.modules.ids().filter(|id| Some(*id) != design.top));
    let mut out = String::new();
    for id in order {
        if design.module(id).blackbox {
            continue;
        }
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&emit_blif_module(design, id)?);
    }
    Ok(out)
}

/// Renders one module as a `.model`.
pub fn emit_blif_module(design: &Design, module: ModuleId) -> Result<String, EmitError> {
    let module = design.module(module);
    require_cell_form(module, "BLIF")?;
    let view = BitView::new(module)?;
    let mut printer = Printer {
        design,
        module,
        view: &view,
        out: Out::new(""),
        consts: [false; 3],
        temps: 0,
    };
    printer.model()?;
    Ok(printer.finish())
}

/// A BLIF token: whitespace, `#`, `=` and `\` cannot appear in a name.
fn token(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_whitespace() || matches!(c, '#' | '=' | '\\') {
                '_'
            } else {
                c
            }
        })
        .collect()
}

struct Printer<'a> {
    design: &'a Design,
    module: &'a Module,
    view: &'a BitView<'a>,
    out: Out,
    /// Which of `$false`, `$true`, `$undef` are used.
    consts: [bool; 3],
    temps: u32,
}

impl Printer<'_> {
    fn finish(self) -> String {
        self.out.finish()
    }

    /// The name of a raw bit (not canonicalised).
    fn raw_name(&self, slot: usize) -> String {
        let (net, i) = self.view.owner(slot);
        let net = &self.module.nets[net];
        if net.ty.width() == Some(1) {
            token(net.name.as_str())
        } else {
            format!("{}[{i}]", token(net.name.as_str()))
        }
    }

    /// The name of the canonical form of a bit.
    fn name(&mut self, bit: SigBit) -> String {
        match self.view.canonical(bit) {
            SigBit::Const(Bit::Zero) => {
                self.consts[0] = true;
                "$false".to_owned()
            }
            SigBit::Const(Bit::One) => {
                self.consts[1] = true;
                "$true".to_owned()
            }
            SigBit::Const(_) => {
                self.consts[2] = true;
                "$undef".to_owned()
            }
            SigBit::Slot(s) => self.raw_name(s),
        }
    }

    fn names(&mut self, bits: &[SigBit]) -> Vec<String> {
        bits.iter().map(|b| self.name(*b)).collect()
    }

    fn temp(&mut self, cell: &Cell, what: &str) -> String {
        self.temps += 1;
        format!("{}.{what}{}", token(cell.name.as_str()), self.temps)
    }

    fn model(&mut self) -> Result<(), EmitError> {
        let m = self.module;
        self.out.line(&format!(".model {}", token(m.name.as_str())));
        let mut inputs = Vec::new();
        let mut outputs = Vec::new();
        for port in &m.ports {
            let slots = self.view.net_slots(port.net, port.span)?;
            let names: Vec<String> = slots.iter().map(|s| self.raw_name(*s)).collect();
            match port.dir {
                PortDir::In => inputs.extend(names),
                PortDir::Out => outputs.extend(names),
                PortDir::InOut => {
                    inputs.extend(names.iter().cloned());
                    outputs.extend(names);
                }
            }
        }
        if !inputs.is_empty() {
            self.out.line(&format!(".inputs {}", inputs.join(" ")));
        }
        if !outputs.is_empty() {
            self.out.line(&format!(".outputs {}", outputs.join(" ")));
        }
        // Body into a buffer so the constant definitions can precede it.
        let mut body = Out::new("");
        std::mem::swap(&mut self.out, &mut body);
        for (_, cell) in m.cells.iter() {
            self.cell(cell)?;
        }
        for (_, inst) in m.instances.iter() {
            let target = match &inst.module {
                ModuleRef::Resolved(id) => self.design.module(*id).name.as_str(),
                ModuleRef::Unresolved(name) => name.as_str(),
            };
            let mut line = format!(".subckt {}", token(target));
            for (port, e) in &inst.connections {
                let bits = self.view.expr_bits(*e)?;
                self.connections(&mut line, port.as_str(), &bits);
            }
            self.out.line(&line);
        }
        // Output ports that alias other bits or constants need buffers.
        for port in &m.ports {
            if port.dir == PortDir::In {
                continue;
            }
            for slot in self.view.net_slots(port.net, port.span)? {
                let canonical = self.view.canonical(SigBit::Slot(slot));
                if canonical != SigBit::Slot(slot) {
                    let from = self.name(canonical);
                    let to = self.raw_name(slot);
                    self.out.line(&format!(".names {from} {to}"));
                    self.out.line("1 1");
                }
            }
        }
        std::mem::swap(&mut self.out, &mut body);
        if self.consts[0] {
            self.out.line(".names $false");
        }
        if self.consts[1] {
            self.out.line(".names $true");
            self.out.line("1");
        }
        if self.consts[2] {
            self.out.line(".names $undef");
        }
        self.out.raw(&body.finish());
        self.out.line(".end");
        Ok(())
    }

    /// Appends `port=bit` (or `port[i]=bit`) pairs to a `.subckt` line.
    fn connections(&mut self, line: &mut String, port: &str, bits: &[SigBit]) {
        let port = token(port);
        if bits.len() == 1 {
            let name = self.name(bits[0]);
            let _ = write!(line, " {port}={name}");
            return;
        }
        for (i, bit) in bits.iter().enumerate() {
            let name = self.name(*bit);
            let _ = write!(line, " {port}[{i}]={name}");
        }
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

    /// The raw slots of an output port; outputs are named by their own
    /// bits since the cell drives them.
    fn output(&self, cell: &Cell, port: &str) -> Result<Vec<String>, EmitError> {
        match cell.output(port) {
            Some(net) => Ok(self
                .view
                .net_slots(net, cell.span)?
                .into_iter()
                .map(|s| self.raw_name(s))
                .collect()),
            None => Err(EmitError::new(
                cell.span,
                format!("cell `{}` has no output `{port}`", cell.name),
            )),
        }
    }

    /// Emits `.names inputs... output` followed by `rows`.
    fn table(&mut self, inputs: &[String], output: &str, rows: &[String]) {
        let mut line = String::from(".names");
        for i in inputs {
            let _ = write!(line, " {i}");
        }
        let _ = write!(line, " {output}");
        self.out.line(&line);
        for row in rows {
            self.out.line(row);
        }
    }

    fn cell(&mut self, cell: &Cell) -> Result<(), EmitError> {
        match &cell.kind {
            CellKind::Not | CellKind::Buf => {
                let a = self.input(cell, "a")?;
                let y = self.output(cell, "y")?;
                let row = if matches!(cell.kind, CellKind::Not) {
                    "0 1"
                } else {
                    "1 1"
                };
                for (a, y) in a.iter().zip(&y) {
                    let a = self.name(*a);
                    self.table(&[a], y, &[row.to_owned()]);
                }
            }
            CellKind::And | CellKind::Or | CellKind::Xor => {
                let a = self.input(cell, "a")?;
                let b = self.input(cell, "b")?;
                let y = self.output(cell, "y")?;
                let rows: Vec<String> = match cell.kind {
                    CellKind::And => vec!["11 1".into()],
                    CellKind::Or => vec!["1- 1".into(), "-1 1".into()],
                    _ => vec!["10 1".into(), "01 1".into()],
                };
                for ((a, b), y) in a.iter().zip(&b).zip(&y) {
                    let a = self.name(*a);
                    let b = self.name(*b);
                    self.table(&[a, b], y, &rows);
                }
            }
            CellKind::Mux => {
                let a = self.input(cell, "a")?;
                let b = self.input(cell, "b")?;
                let s = self.input(cell, "s")?;
                let y = self.output(cell, "y")?;
                let s = self.name(s[0]);
                for ((a, b), y) in a.iter().zip(&b).zip(&y) {
                    let a = self.name(*a);
                    let b = self.name(*b);
                    self.table(&[a, b, s.clone()], y, &["1-0 1".into(), "-11 1".into()]);
                }
            }
            CellKind::Pmux => {
                let a = self.input(cell, "a")?;
                let b = self.input(cell, "b")?;
                let s = self.input(cell, "s")?;
                let y = self.output(cell, "y")?;
                let n = s.len();
                let w = y.len();
                let s_names = self.names(&s);
                for (i, y) in y.iter().enumerate() {
                    let mut inputs = vec![self.name(a[i])];
                    for j in 0..n {
                        let bit = b.get(j * w + i).copied().unwrap_or(SigBit::Const(Bit::X));
                        inputs.push(self.name(bit));
                    }
                    inputs.extend(s_names.iter().cloned());
                    let mut rows = vec![format!("1{}{} 1", "-".repeat(n), "0".repeat(n))];
                    for j in 0..n {
                        let one_hot = |k: usize| if k == j { '1' } else { '-' };
                        let sel: String = (0..n).map(one_hot).collect();
                        rows.push(format!("-{sel}{sel} 1"));
                    }
                    self.table(&inputs, y, &rows);
                }
            }
            CellKind::ReduceAnd | CellKind::ReduceOr => {
                let a = self.input(cell, "a")?;
                let y = self.output(cell, "y")?;
                let inputs = self.names(&a);
                let rows: Vec<String> = if matches!(cell.kind, CellKind::ReduceAnd) {
                    vec![format!("{} 1", "1".repeat(inputs.len()))]
                } else {
                    (0..inputs.len())
                        .map(|i| {
                            let row: String = (0..inputs.len())
                                .map(|k| if k == i { '1' } else { '-' })
                                .collect();
                            format!("{row} 1")
                        })
                        .collect()
                };
                self.table(&inputs, &y[0], &rows);
            }
            CellKind::ReduceXor => {
                let a = self.input(cell, "a")?;
                let y = self.output(cell, "y")?;
                let inputs = self.names(&a);
                match inputs.as_slice() {
                    [] => {
                        self.consts[0] = true;
                        self.table(&["$false".to_owned()], &y[0], &["1 1".into()]);
                    }
                    [single] => self.table(std::slice::from_ref(single), &y[0], &["1 1".into()]),
                    _ => {
                        let mut acc = inputs[0].clone();
                        for (i, next) in inputs[1..].iter().enumerate() {
                            let out = if i + 2 == inputs.len() {
                                y[0].clone()
                            } else {
                                self.temp(cell, "x")
                            };
                            self.table(
                                &[acc.clone(), next.clone()],
                                &out,
                                &["10 1".into(), "01 1".into()],
                            );
                            acc = out;
                        }
                    }
                }
            }
            CellKind::Lut { k, init } => {
                let a = self.input(cell, "a")?;
                let y = self.output(cell, "y")?;
                let inputs = self.names(&a);
                let mut rows = Vec::new();
                for i in 0..init.width() {
                    if init.bit(i) == Bit::One {
                        let row: String = (0..*k)
                            .map(|j| if (i >> j) & 1 == 1 { '1' } else { '0' })
                            .collect();
                        rows.push(format!("{row} 1"));
                    }
                }
                self.table(&inputs, &y[0], &rows);
            }
            CellKind::Dff {
                clk_pos,
                has_enable,
                reset,
            } => {
                if reset.as_ref().is_some_and(|r| r.asynchronous) {
                    return Err(EmitError::new(
                        cell.span,
                        format!(
                            "cell `{}`: BLIF latches have no asynchronous reset",
                            cell.name
                        ),
                    ));
                }
                let clk = self.input(cell, "clk")?;
                let clk = self.name(clk[0]);
                let d = self.input(cell, "d")?;
                let q = self.output(cell, "q")?;
                let q_net = cell.output("q").expect("checked");
                let init = self.module.nets[q_net].attrs.get("init").cloned();
                let rst = match reset {
                    Some(_) => Some(self.input(cell, "rst")?),
                    None => None,
                };
                let en = if *has_enable {
                    Some(self.input(cell, "en")?)
                } else {
                    None
                };
                let edge = if *clk_pos { "re" } else { "fe" };
                for (i, q) in q.iter().enumerate() {
                    let d_bit = self.name(d[i]);
                    let init_bit = match &init {
                        Some(AttrValue::Const(c))
                            if c.get(u32::try_from(i).unwrap_or(u32::MAX)).is_some() =>
                        {
                            match c.bit(u32::try_from(i).unwrap_or(0)) {
                                Bit::Zero => '0',
                                Bit::One => '1',
                                _ => '2',
                            }
                        }
                        Some(AttrValue::Int(v)) => {
                            if (v >> i.min(63)) & 1 == 1 {
                                '1'
                            } else {
                                '0'
                            }
                        }
                        _ => '2',
                    };
                    let next = match (&rst, &en) {
                        (None, None) => d_bit,
                        _ => {
                            let mut inputs = Vec::new();
                            let mut rows = Vec::new();
                            let reset = reset.as_ref();
                            let active =
                                reset.map_or('1', |r| if r.active_high { '1' } else { '0' });
                            let inactive = if active == '1' { '0' } else { '1' };
                            let value_one = reset.is_some_and(|r| {
                                r.value.bit(u32::try_from(i).unwrap_or(0)) == Bit::One
                            });
                            if let Some(rst) = &rst {
                                inputs.push(self.name(rst[0]));
                            }
                            if let Some(en) = &en {
                                inputs.push(self.name(en[0]));
                            }
                            inputs.push(d_bit);
                            inputs.push(q.clone());
                            match (rst.is_some(), en.is_some()) {
                                (true, true) => {
                                    if value_one {
                                        rows.push(format!("{active}--- 1"));
                                    }
                                    rows.push(format!("{inactive}11- 1"));
                                    rows.push(format!("{inactive}0-1 1"));
                                }
                                (true, false) => {
                                    if value_one {
                                        rows.push(format!("{active}-- 1"));
                                    }
                                    rows.push(format!("{inactive}1- 1"));
                                }
                                _ => {
                                    rows.push("11- 1".into());
                                    rows.push("0-1 1".into());
                                }
                            }
                            let next = self.temp(cell, "d");
                            self.table(&inputs, &next, &rows);
                            next
                        }
                    };
                    self.out
                        .line(&format!(".latch {next} {q} {edge} {clk} {init_bit}"));
                }
            }
            CellKind::Dlatch => {
                let en = self.input(cell, "en")?;
                let en = self.name(en[0]);
                let d = self.input(cell, "d")?;
                let q = self.output(cell, "q")?;
                for (d, q) in d.iter().zip(&q) {
                    let d = self.name(*d);
                    self.out.line(&format!(".latch {d} {q} ah {en} 2"));
                }
            }
            CellKind::Blackbox(name) => {
                let mut line = format!(".subckt {}", token(name.as_str()));
                for (port, e) in &cell.inputs {
                    let bits = self.view.expr_bits(*e)?;
                    self.connections(&mut line, port.as_str(), &bits);
                }
                for (port, net) in &cell.outputs {
                    let slots = self.view.net_slots(*net, cell.span)?;
                    let bits: Vec<SigBit> = slots.into_iter().map(SigBit::Slot).collect();
                    self.connections(&mut line, port.as_str(), &bits);
                }
                self.out.line(&line);
            }
            other => {
                return Err(EmitError::new(
                    cell.span,
                    format!(
                        "cell `{}`: BLIF cannot express `{}`; map the design to gates or LUTs first",
                        cell.name,
                        other.keyword()
                    ),
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens() {
        assert_eq!(token("a b"), "a_b");
        assert_eq!(token("x#y=z\\"), "x_y_z_");
        assert_eq!(token("q[3]"), "q[3]");
    }
}
