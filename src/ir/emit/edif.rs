//! EDIF 2 0 0 netlist emission.
//!
//! The layout is the one Xilinx, Lattice and Yosys tools exchange:
//!
//! ```text
//! (edif top
//!   (edifVersion 2 0 0) (edifLevel 0) (keywordMap (keywordLevel 0))
//!   (status (written (timeStamp 1970 1 1 0 0 0) (program "reticle")))
//!   (external LIB (edifLevel 0) (technology (numberDefinition))
//!     (cell AND_8 (cellType GENERIC) (view VIEW_NETLIST (viewType NETLIST)
//!       (interface (port (rename A_0_ "A[0]") (direction INPUT)) ...))) ...)
//!   (library DESIGN (edifLevel 0) (technology (numberDefinition))
//!     (cell adder (cellType GENERIC) (view VIEW_NETLIST (viewType NETLIST)
//!       (interface (port (rename a_0_ "a[0]") (direction INPUT)) ...)
//!       (contents
//!         (instance add0 (viewRef VIEW_NETLIST (cellRef AND_8 (libraryRef LIB))))
//!         (net (rename y_0_ "y[0]") (joined (portRef Y_0_ (instanceRef add0)) (portRef y_0_)))))))
//!   (design top (cellRef top (libraryRef DESIGN))))
//! ```
//!
//! Buses are split into single-bit ports and nets named `bus[i]`; names
//! that are not EDIF identifiers are written through `(rename id "name")`.
//! Modules of the design go into the `DESIGN` library (leaves first) and
//! primitive cells into the external `LIB` library, one cell per kind and
//! width:
//!
//! | IR cell                          | External cell                        | Ports                                     |
//! |----------------------------------|--------------------------------------|-------------------------------------------|
//! | `Not` `Buf` `And` … `Ge` `Reduce*` | `NOT_W`, `BUF_W`, `AND_W`, `ADD_W`, `SHL_W_B`, `EQ_W`, `REDUCE_AND_W` … | `A[i]`, `B[i]`, `Y[i]` (or `Y`) |
//! | `Mux`, `Pmux`                    | `MUX_W`, `PMUX_W_N`                  | `A[i]`, `B[i]`, `S` / `S[i]`, `Y[i]`      |
//! | `Dff`                            | `DFF_W`, `DFFE_W`, `ADFF_W`, `ADFFE_W`, `SDFF_W`, `SDFFE_W` | `CLK`, `D[i]`, `Q[i]`, `EN`, `RST`; properties `CLK_POLARITY`, `RST_POLARITY`, `RST_VALUE` |
//! | `Dlatch`                         | `DLATCH_W`                           | `EN`, `D[i]`, `Q[i]`                      |
//! | `MemRdPort`, `MemWrPort`         | `MEMRD_W_A[_CLK]`, `MEMWR_W_A[_CLK]` | `ADDR[i]`, `DATA[i]`, `EN`, `CLK`; property `MEMID` |
//! | `Lut`                            | `LUT_K`                              | `A[i]`, `Y`; property `INIT`              |
//! | `Tristate`                       | `TRIBUF_W`                           | `A[i]`, `EN`, `Y[i]`                      |
//! | `Blackbox(name)`, unresolved instances | `name`                         | the connected ports                       |
//!
//! Constant bits are driven by `GND` / `VCC` instances (ports `G` / `P`);
//! `x` and `z` bits are left unconnected. Parameters and attributes become
//! `(property ...)` entries.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use super::super::attr::{AttrValue, Attrs};
use super::super::cell::{Cell, CellKind};
use super::super::design::{Design, Module, ModuleRef, PortDir};
use super::{BitView, EmitError, Out, SigBit, edif_ident, leaves_first, require_cell_form};
use crate::logic::Bit;

/// Renders every module of `design`.
pub fn emit_edif(design: &Design) -> Result<String, EmitError> {
    let top_name = design
        .top_module()
        .or_else(|| design.modules.values().next())
        .map_or("design".to_owned(), |m| m.name.as_str().to_owned());
    let mut lib: BTreeMap<String, LibCell> = BTreeMap::new();
    let mut cells = Out::new("  ");
    cells.indent();
    cells.indent();
    for id in leaves_first(design) {
        let module = design.module(id);
        if module.blackbox {
            continue;
        }
        require_cell_form(module, "EDIF")?;
        let view = BitView::new(module)?;
        let mut printer = Printer {
            design,
            module,
            view: &view,
            lib: &mut lib,
            out: Out::new("  "),
            nets: BTreeMap::new(),
            consts: [false; 2],
            names: Namer::default(),
            port_names: Namer::default(),
        };
        printer.out.indent();
        printer.out.indent();
        printer.module()?;
        cells.raw(&printer.out.finish());
    }
    let mut out = Out::new("  ");
    out.line(&format!("(edif {}", name_form(&top_name)));
    out.indent();
    out.line("(edifVersion 2 0 0)");
    out.line("(edifLevel 0)");
    out.line("(keywordMap (keywordLevel 0))");
    out.line("(status (written (timeStamp 1970 1 1 0 0 0) (program \"reticle\")))");
    out.line("(external LIB");
    out.indent();
    out.line("(edifLevel 0)");
    out.line("(technology (numberDefinition))");
    for cell in lib.values() {
        out.line(&format!("(cell {} (cellType GENERIC)", cell.id));
        out.indent();
        out.line("(view VIEW_NETLIST (viewType NETLIST)");
        out.indent();
        out.line("(interface");
        out.indent();
        for (port, dir) in &cell.ports {
            out.line(&format!("(port {} (direction {dir}))", name_form(port)));
        }
        out.dedent();
        out.line(")");
        out.dedent();
        out.line(")");
        out.dedent();
        out.line(")");
    }
    out.dedent();
    out.line(")");
    out.line("(library DESIGN");
    out.indent();
    out.line("(edifLevel 0)");
    out.line("(technology (numberDefinition))");
    out.raw(&cells.finish());
    out.dedent();
    out.line(")");
    if let Some(top) = design.top_module() {
        out.line(&format!(
            "(design {} (cellRef {} (libraryRef DESIGN)))",
            name_form(top.name.as_str()),
            name_form(top.name.as_str())
        ));
    }
    out.dedent();
    out.line(")");
    Ok(out.finish())
}

/// A name as an EDIF name form: the identifier, or `(rename id "name")`.
fn name_form(name: &str) -> String {
    let (id, renamed) = edif_ident(name);
    if renamed {
        format!("(rename {id} {})", edif_string(name))
    } else {
        id
    }
}

fn edif_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("%34%"),
            '%' => out.push_str("%37%"),
            c if c.is_ascii_graphic() || c == ' ' => out.push(c),
            _ => out.push('_'),
        }
    }
    out.push('"');
    out
}

fn property(name: &str, value: &AttrValue) -> String {
    let value = match value {
        AttrValue::Int(i) => format!("(integer {i})"),
        AttrValue::String(s) => format!("(string {})", edif_string(s)),
        AttrValue::Const(c) => match c.to_u64() {
            Some(v) if c.width() <= 31 => format!("(integer {v})"),
            _ => format!("(string {})", edif_string(&c.to_binary_string())),
        },
    };
    format!("(property {} {value})", name_form(name))
}

/// Gives every name in one namespace a distinct EDIF identifier
/// (identifiers are case-insensitive).
#[derive(Default)]
struct Namer {
    used: BTreeSet<String>,
    map: BTreeMap<String, String>,
}

impl Namer {
    /// The name form of `name`, unique within this namespace.
    fn form(&mut self, name: &str) -> String {
        if let Some(form) = self.map.get(name) {
            return form.clone();
        }
        let (mut id, mut renamed) = edif_ident(name);
        let base = id.clone();
        let mut i = 1;
        while self.used.contains(&id.to_ascii_lowercase()) {
            id = format!("{base}_{i}");
            renamed = true;
            i += 1;
        }
        self.used.insert(id.to_ascii_lowercase());
        let form = if renamed {
            format!("(rename {id} {})", edif_string(name))
        } else {
            id
        };
        self.map.insert(name.to_owned(), form.clone());
        form
    }

    /// The bare identifier of a name already registered with [`form`].
    fn id(&self, name: &str) -> String {
        let form = &self.map[name];
        match form.strip_prefix("(rename ") {
            Some(rest) => rest.split(' ').next().unwrap_or(rest).to_owned(),
            None => form.clone(),
        }
    }
}

/// An external library cell: identifier and ports with directions.
struct LibCell {
    id: String,
    ports: Vec<(String, &'static str)>,
}

struct Printer<'a> {
    design: &'a Design,
    module: &'a Module,
    view: &'a BitView<'a>,
    lib: &'a mut BTreeMap<String, LibCell>,
    out: Out,
    /// Port references joined to each canonical bit.
    nets: BTreeMap<SigBit, Vec<String>>,
    /// Whether GND / VCC are used.
    consts: [bool; 2],
    names: Namer,
    port_names: Namer,
}

impl Printer<'_> {
    fn bus_names(port: &str, width: usize) -> Vec<String> {
        if width == 1 {
            vec![port.to_owned()]
        } else {
            (0..width).map(|i| format!("{port}[{i}]")).collect()
        }
    }

    /// Registers an external library cell, merging ports across uses.
    fn lib_cell(&mut self, name: &str, ports: &[(String, &'static str)]) -> String {
        let entry = self.lib.entry(name.to_owned()).or_insert_with(|| LibCell {
            id: edif_ident(name).0,
            ports: Vec::new(),
        });
        for (port, dir) in ports {
            if !entry.ports.iter().any(|(p, _)| p == port) {
                entry.ports.push((port.clone(), dir));
            }
        }
        entry.id.clone()
    }

    /// Joins `bit` to a port reference.
    fn join(&mut self, bit: SigBit, port_ref: String) {
        let canonical = self.view.canonical(bit);
        match canonical {
            SigBit::Const(Bit::X | Bit::Z) => {}
            SigBit::Const(Bit::Zero) => self.consts[0] = true,
            SigBit::Const(Bit::One) => self.consts[1] = true,
            SigBit::Slot(_) => {}
        }
        if !matches!(canonical, SigBit::Const(Bit::X | Bit::Z)) {
            self.nets.entry(canonical).or_default().push(port_ref);
        }
    }

    fn module(&mut self) -> Result<(), EmitError> {
        let m = self.module;
        let cell_name = name_form(m.name.as_str());
        self.out
            .line(&format!("(cell {cell_name} (cellType GENERIC)"));
        self.out.indent();
        for (k, v) in m.attrs.iter() {
            self.out.line(&property(k.as_str(), v));
        }
        self.out.line("(view VIEW_NETLIST (viewType NETLIST)");
        self.out.indent();
        self.out.line("(interface");
        self.out.indent();
        for port in &m.ports {
            let slots = self.view.net_slots(port.net, port.span)?;
            let dir = match port.dir {
                PortDir::In => "INPUT",
                PortDir::Out => "OUTPUT",
                PortDir::InOut => "INOUT",
            };
            for (name, slot) in Self::bus_names(port.name.as_str(), slots.len())
                .into_iter()
                .zip(slots)
            {
                let form = self.port_names.form(&name);
                self.out.line(&format!("(port {form} (direction {dir}))"));
                let id = self.port_names.id(&name);
                self.join(SigBit::Slot(slot), format!("(portRef {id})"));
            }
        }
        self.out.dedent();
        self.out.line(")");
        self.out.line("(contents");
        self.out.indent();
        // Reserve net identifiers for representative slots first so
        // instances cannot steal a net's name.
        for (_, cell) in m.cells.iter() {
            self.cell(cell)?;
        }
        for (_, inst) in m.instances.iter() {
            self.instance(inst)?;
        }
        if self.consts[0] {
            let id = self.lib_cell("GND", &[("G".to_owned(), "OUTPUT")]);
            let inst = self.names.form("GND_INST");
            self.out.line(&format!(
                "(instance {inst} (viewRef VIEW_NETLIST (cellRef {id} (libraryRef LIB))))"
            ));
        }
        if self.consts[1] {
            let id = self.lib_cell("VCC", &[("P".to_owned(), "OUTPUT")]);
            let inst = self.names.form("VCC_INST");
            self.out.line(&format!(
                "(instance {inst} (viewRef VIEW_NETLIST (cellRef {id} (libraryRef LIB))))"
            ));
        }
        let nets = std::mem::take(&mut self.nets);
        for (bit, mut refs) in nets {
            let name = match bit {
                SigBit::Const(Bit::Zero) => {
                    let gnd = self.names.id("GND_INST");
                    refs.insert(0, format!("(portRef G (instanceRef {gnd}))"));
                    "GND_NET".to_owned()
                }
                SigBit::Const(Bit::One) => {
                    let vcc = self.names.id("VCC_INST");
                    refs.insert(0, format!("(portRef P (instanceRef {vcc}))"));
                    "VCC_NET".to_owned()
                }
                SigBit::Const(_) => continue,
                SigBit::Slot(slot) => {
                    let (net, i) = self.view.owner(slot);
                    let net = &m.nets[net];
                    if net.ty.width() == Some(1) {
                        net.name.as_str().to_owned()
                    } else {
                        format!("{}[{i}]", net.name)
                    }
                }
            };
            let form = self.names.form(&name);
            self.out.line(&format!("(net {form} (joined"));
            self.out.indent();
            for r in refs {
                self.out.line(&r);
            }
            self.out.dedent();
            self.out.line("))");
        }
        self.out.dedent();
        self.out.line(")");
        self.out.dedent();
        self.out.line(")");
        self.out.dedent();
        self.out.line(")");
        Ok(())
    }

    /// Emits an instance of a library or design cell with its pins.
    fn emit_instance(
        &mut self,
        name: &str,
        cell_ref: &str,
        library: &str,
        props: &[String],
        attrs: &Attrs,
        pins: Vec<(String, SigBit)>,
    ) {
        let form = self.names.form(name);
        let id = self.names.id(name);
        let mut line = format!(
            "(instance {form} (viewRef VIEW_NETLIST (cellRef {cell_ref} (libraryRef {library})))"
        );
        for p in props {
            let _ = write!(line, " {p}");
        }
        for (k, v) in attrs.iter() {
            let _ = write!(line, " {}", property(k.as_str(), v));
        }
        line.push(')');
        self.out.line(&line);
        for (port, bit) in pins {
            let pid = edif_ident(&port).0;
            self.join(bit, format!("(portRef {pid} (instanceRef {id}))"));
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

    fn output(&self, cell: &Cell, port: &str) -> Result<Vec<SigBit>, EmitError> {
        match cell.output(port) {
            Some(net) => Ok(self
                .view
                .net_slots(net, cell.span)?
                .into_iter()
                .map(SigBit::Slot)
                .collect()),
            None => Err(EmitError::new(
                cell.span,
                format!("cell `{}` has no output `{port}`", cell.name),
            )),
        }
    }

    fn cell(&mut self, cell: &Cell) -> Result<(), EmitError> {
        // (port, direction, bits)
        let mut ports: Vec<(&str, &'static str, Vec<SigBit>)> = Vec::new();
        let mut props: Vec<String> = Vec::new();
        let lib_name: String = match &cell.kind {
            CellKind::Not
            | CellKind::Buf
            | CellKind::ReduceAnd
            | CellKind::ReduceOr
            | CellKind::ReduceXor => {
                let a = self.input(cell, "a")?;
                let y = self.output(cell, "y")?;
                let name = format!("{}_{}", cell.kind.keyword().to_ascii_uppercase(), a.len());
                let name = name
                    .replace("RAND_", "REDUCE_AND_")
                    .replace("ROR_", "REDUCE_OR_")
                    .replace("RXOR_", "REDUCE_XOR_");
                ports.push(("A", "INPUT", a));
                ports.push(("Y", "OUTPUT", y));
                name
            }
            CellKind::Shl | CellKind::Shr | CellKind::Sshr => {
                let a = self.input(cell, "a")?;
                let b = self.input(cell, "b")?;
                let y = self.output(cell, "y")?;
                let name = format!(
                    "{}_{}_{}",
                    cell.kind.keyword().to_ascii_uppercase(),
                    a.len(),
                    b.len()
                );
                ports.push(("A", "INPUT", a));
                ports.push(("B", "INPUT", b));
                ports.push(("Y", "OUTPUT", y));
                name
            }
            CellKind::And
            | CellKind::Or
            | CellKind::Xor
            | CellKind::Add
            | CellKind::Sub
            | CellKind::Mul
            | CellKind::Div
            | CellKind::Mod
            | CellKind::Eq
            | CellKind::Ne
            | CellKind::Lt
            | CellKind::Le
            | CellKind::Gt
            | CellKind::Ge => {
                let a = self.input(cell, "a")?;
                let b = self.input(cell, "b")?;
                let y = self.output(cell, "y")?;
                let name = format!("{}_{}", cell.kind.keyword().to_ascii_uppercase(), a.len());
                let signed = cell
                    .input("a")
                    .is_some_and(|id| self.module.exprs[id].ty.is_signed())
                    && cell
                        .input("b")
                        .is_some_and(|id| self.module.exprs[id].ty.is_signed());
                props.push(property("SIGNED", &AttrValue::Int(i64::from(signed))));
                ports.push(("A", "INPUT", a));
                ports.push(("B", "INPUT", b));
                ports.push(("Y", "OUTPUT", y));
                name
            }
            CellKind::Mux => {
                let a = self.input(cell, "a")?;
                let b = self.input(cell, "b")?;
                let s = self.input(cell, "s")?;
                let y = self.output(cell, "y")?;
                let name = format!("MUX_{}", y.len());
                ports.push(("A", "INPUT", a));
                ports.push(("B", "INPUT", b));
                ports.push(("S", "INPUT", s));
                ports.push(("Y", "OUTPUT", y));
                name
            }
            CellKind::Pmux => {
                let a = self.input(cell, "a")?;
                let b = self.input(cell, "b")?;
                let s = self.input(cell, "s")?;
                let y = self.output(cell, "y")?;
                let name = format!("PMUX_{}_{}", y.len(), s.len());
                ports.push(("A", "INPUT", a));
                ports.push(("B", "INPUT", b));
                ports.push(("S", "INPUT", s));
                ports.push(("Y", "OUTPUT", y));
                name
            }
            CellKind::Dff {
                clk_pos,
                has_enable,
                reset,
            } => {
                let clk = self.input(cell, "clk")?;
                let d = self.input(cell, "d")?;
                let q = self.output(cell, "q")?;
                let mut name = match reset {
                    Some(r) if r.asynchronous => "ADFF".to_owned(),
                    Some(_) => "SDFF".to_owned(),
                    None => "DFF".to_owned(),
                };
                if *has_enable {
                    name.push('E');
                }
                let _ = write!(name, "_{}", q.len());
                props.push(property(
                    "CLK_POLARITY",
                    &AttrValue::Int(i64::from(*clk_pos)),
                ));
                ports.push(("CLK", "INPUT", clk));
                if let Some(r) = reset {
                    let rst = self.input(cell, "rst")?;
                    props.push(property(
                        "RST_POLARITY",
                        &AttrValue::Int(i64::from(r.active_high)),
                    ));
                    props.push(property(
                        "RST_VALUE",
                        &AttrValue::String(r.value.to_binary_string()),
                    ));
                    ports.push(("RST", "INPUT", rst));
                }
                if *has_enable {
                    let en = self.input(cell, "en")?;
                    ports.push(("EN", "INPUT", en));
                }
                ports.push(("D", "INPUT", d));
                ports.push(("Q", "OUTPUT", q));
                name
            }
            CellKind::Dlatch => {
                let en = self.input(cell, "en")?;
                let d = self.input(cell, "d")?;
                let q = self.output(cell, "q")?;
                let name = format!("DLATCH_{}", q.len());
                ports.push(("EN", "INPUT", en));
                ports.push(("D", "INPUT", d));
                ports.push(("Q", "OUTPUT", q));
                name
            }
            CellKind::MemRdPort { mem, clocked } => {
                let addr = self.input(cell, "addr")?;
                let data = self.output(cell, "data")?;
                let mut name = format!("MEMRD_{}_{}", data.len(), addr.len());
                if *clocked {
                    name.push_str("_CLK");
                    let clk = self.input(cell, "clk")?;
                    let en = self.input(cell, "en")?;
                    ports.push(("CLK", "INPUT", clk));
                    ports.push(("EN", "INPUT", en));
                }
                props.push(property(
                    "MEMID",
                    &AttrValue::String(self.module.memories[*mem].name.as_str().to_owned()),
                ));
                ports.push(("ADDR", "INPUT", addr));
                ports.push(("DATA", "OUTPUT", data));
                name
            }
            CellKind::MemWrPort { mem, clocked } => {
                let addr = self.input(cell, "addr")?;
                let data = self.input(cell, "data")?;
                let en = self.input(cell, "en")?;
                let mut name = format!("MEMWR_{}_{}", data.len(), addr.len());
                if *clocked {
                    name.push_str("_CLK");
                    let clk = self.input(cell, "clk")?;
                    ports.push(("CLK", "INPUT", clk));
                }
                props.push(property(
                    "MEMID",
                    &AttrValue::String(self.module.memories[*mem].name.as_str().to_owned()),
                ));
                ports.push(("ADDR", "INPUT", addr));
                ports.push(("DATA", "INPUT", data));
                ports.push(("EN", "INPUT", en));
                name
            }
            CellKind::Lut { k, init } => {
                let a = self.input(cell, "a")?;
                let y = self.output(cell, "y")?;
                props.push(property(
                    "INIT",
                    &AttrValue::String(init.to_binary_string()),
                ));
                ports.push(("A", "INPUT", a));
                ports.push(("Y", "OUTPUT", y));
                format!("LUT_{k}")
            }
            CellKind::Tristate => {
                let a = self.input(cell, "a")?;
                let en = self.input(cell, "en")?;
                let y = self.output(cell, "y")?;
                let name = format!("TRIBUF_{}", y.len());
                ports.push(("A", "INPUT", a));
                ports.push(("EN", "INPUT", en));
                ports.push(("Y", "OUTPUT", y));
                name
            }
            CellKind::Blackbox(name) => {
                for (k, v) in cell.params.iter() {
                    props.push(property(k.as_str(), v));
                }
                for (port, e) in &cell.inputs {
                    ports.push((port.as_str(), "INPUT", self.view.expr_bits(*e)?));
                }
                for (port, net) in &cell.outputs {
                    let bits = self
                        .view
                        .net_slots(*net, cell.span)?
                        .into_iter()
                        .map(SigBit::Slot)
                        .collect();
                    ports.push((port.as_str(), "OUTPUT", bits));
                }
                name.as_str().to_owned()
            }
        };
        let mut lib_ports = Vec::new();
        let mut pins = Vec::new();
        for (port, dir, bits) in &ports {
            for (name, bit) in Self::bus_names(port, bits.len()).into_iter().zip(bits) {
                lib_ports.push((name.clone(), *dir));
                pins.push((name, *bit));
            }
        }
        let id = self.lib_cell(&lib_name, &lib_ports);
        self.emit_instance(cell.name.as_str(), &id, "LIB", &props, &cell.attrs, pins);
        Ok(())
    }

    fn instance(&mut self, inst: &super::super::design::Instance) -> Result<(), EmitError> {
        let props: Vec<String> = inst
            .params
            .iter()
            .map(|(k, v)| property(k.as_str(), v))
            .collect();
        let mut pins = Vec::new();
        let target = inst.module.id().map(|id| self.design.module(id));
        match target {
            Some(t) if !t.blackbox => {
                for (port, e) in &inst.connections {
                    let bits = self.view.expr_bits(*e)?;
                    for (name, bit) in Self::bus_names(port.as_str(), bits.len())
                        .into_iter()
                        .zip(bits)
                    {
                        pins.push((name, bit));
                    }
                }
                let id = edif_ident(t.name.as_str()).0;
                self.emit_instance(inst.name.as_str(), &id, "DESIGN", &props, &inst.attrs, pins);
            }
            _ => {
                let name = match &inst.module {
                    ModuleRef::Resolved(id) => self.design.module(*id).name.as_str(),
                    ModuleRef::Unresolved(n) => n.as_str(),
                };
                let mut lib_ports = Vec::new();
                for (port, e) in &inst.connections {
                    let bits = self.view.expr_bits(*e)?;
                    let dir =
                        target
                            .and_then(|t| t.port(port.as_str()))
                            .map_or("INPUT", |p| match p.dir {
                                PortDir::In => "INPUT",
                                PortDir::Out => "OUTPUT",
                                PortDir::InOut => "INOUT",
                            });
                    for (name, bit) in Self::bus_names(port.as_str(), bits.len())
                        .into_iter()
                        .zip(bits)
                    {
                        lib_ports.push((name.clone(), dir));
                        pins.push((name, bit));
                    }
                }
                let id = self.lib_cell(name, &lib_ports);
                self.emit_instance(inst.name.as_str(), &id, "LIB", &props, &inst.attrs, pins);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_properties() {
        assert_eq!(name_form("clk"), "clk");
        assert_eq!(name_form("a[0]"), "(rename a_0_ \"a[0]\")");
        assert_eq!(edif_string("say \"hi\" 100%"), "\"say %34%hi%34% 100%37%\"");
        assert_eq!(
            property("N", &AttrValue::Int(3)),
            "(property N (integer 3))"
        );
        assert_eq!(
            property("S", &AttrValue::from("x")),
            "(property S (string \"x\"))"
        );
        let mut namer = Namer::default();
        assert_eq!(namer.form("A"), "A");
        assert_eq!(namer.form("a"), "(rename a_1 \"a\")");
        assert_eq!(namer.form("A"), "A");
        assert_eq!(namer.id("a"), "a_1");
        assert_eq!(namer.id("A"), "A");
    }
}
