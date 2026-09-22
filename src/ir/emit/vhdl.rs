//! VHDL-2008 emission.
//!
//! Every module becomes an `entity` / `architecture rtl` pair. Bit vectors
//! are `unsigned` / `signed` from `ieee.numeric_std` (one-bit vectors are
//! `std_logic`), so arithmetic, comparison and shift operators map directly
//! and the 4-state semantics of the IR survive: comparisons use the
//! VHDL-2008 matching operators (`?=`, `?<`, ...), which return
//! `std_logic`, and `x` / `z` constants become `'X'` / `'Z'`.
//!
//! # Processes
//!
//! | Kind                       | Rendering                                                        |
//! |----------------------------|------------------------------------------------------------------|
//! | `Comb`                     | `process (all)`                                                  |
//! | `Sequential`               | `process (clk, rst)` with `rising_edge` / `falling_edge`; a body that is `if rst ... else ...` on an asynchronous reset becomes the `if rst then ... elsif rising_edge(clk)` idiom |
//! | `Initial`                  | `process begin ... wait; end process;`                           |
//! | `Sensitive`                | `process (a, b)`                                                 |
//! | `Free`                     | `process begin ... end process;`                                 |
//!
//! Blocking assignments have no VHDL signal equivalent, so every net a
//! process assigns with `=` gets a *shadow variable* inside that process:
//! loaded from the signal when the process starts, read and written in
//! place of the signal, and written back to the signal at the end of the
//! process and around every `wait`. Non-blocking assignments are signal
//! assignments (`<=`, with `after` for delays). `case` on constant items
//! becomes `case` / `case?` (for `casez` / `casex`, with `-` wildcards),
//! on variable items an `if` chain. Loops with `break` / `continue` use
//! labelled `exit`. `$display`-style calls become `report` with the format
//! string converted to `to_string` / `to_hstring` concatenations;
//! `$finish` / `$stop` become `std.env.finish` / `std.env.stop`.
//!
//! # Cells
//!
//! Combinational cells become concurrent signal assignments, `Dff`,
//! `Dlatch` and memory ports become processes, `Lut` a constant indexed by
//! its input, `Tristate` a conditional assignment to `'Z'`, `Blackbox` a
//! component instantiation with `generic map`. Instances of modules in the
//! design use `entity work.name`; instances of black-box or unresolved
//! modules use a component declared from the connections.
//!
//! # Helpers
//!
//! Constructs VHDL cannot write inline (selecting bits of an expression,
//! `?:`, replication of a vector, `pmux`, `**` on vectors) call functions
//! of a small package `reticle_pkg` that is emitted at the top of the file
//! whenever it is used.
//!
//! # Names
//!
//! Names that are not basic identifiers, are reserved words, collide
//! case-insensitively with another name of the module, or would hide an
//! `ieee` or helper name the emitter relies on are written as extended
//! identifiers (`\name\`).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use super::super::attr::{AttrValue, Attrs};
use super::super::cell::{Cell, CellKind};
use super::super::design::{
    Design, Instance, InstanceId, Module, ModuleId, ModuleRef, NetId, PortDir,
};
use super::super::expr::{BinaryOp, ExprId, ExprKind, UnaryOp};
use super::super::process::{
    AssignKind, Block, CaseKind, Delay, Edge, Lvalue, Polarity, Process, ProcessKind,
    ReportSeverity, Stmt, StmtKind, TimeUnit, WaitKind,
};
use super::super::types::{Bit, Const, Type};
use super::{EmitError, Out, is_basic_vhdl_ident, vhdl_extended, vhdl_ident, vhdl_string};
use crate::source::Span;

/// Renders every module of `design`, preceded by the helper package when
/// any module needs it.
pub fn emit_vhdl(design: &Design) -> Result<String, EmitError> {
    let names = ModuleNames::new(design);
    let mut units = Vec::new();
    let mut uses_pkg = false;
    for (id, module) in design.modules.iter() {
        if module.blackbox {
            continue;
        }
        let mut printer = Printer::new(design, id, &names);
        printer.module()?;
        uses_pkg |= printer.uses_pkg;
        units.push(printer.finish());
    }
    Ok(assemble(units, uses_pkg))
}

/// Renders one module (with the helper package when it needs it).
pub fn emit_vhdl_module(design: &Design, module: ModuleId) -> Result<String, EmitError> {
    let names = ModuleNames::new(design);
    let mut printer = Printer::new(design, module, &names);
    printer.module()?;
    let uses_pkg = printer.uses_pkg;
    Ok(assemble(vec![printer.finish()], uses_pkg))
}

fn assemble(units: Vec<String>, uses_pkg: bool) -> String {
    let mut out = String::new();
    if uses_pkg {
        out.push_str(PACKAGE);
    }
    for unit in units {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str("library ieee;\nuse ieee.std_logic_1164.all;\nuse ieee.numeric_std.all;\n");
        if uses_pkg {
            out.push_str("use work.reticle_pkg.all;\n");
        }
        out.push('\n');
        out.push_str(&unit);
    }
    out
}

/// The helper package.
const PACKAGE: &str = r#"library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

package reticle_pkg is
  function to_sl(v : unsigned) return std_logic;
  function to_sl(b : boolean) return std_logic;
  function idx(v : unsigned; i : integer) return std_logic;
  function slc(v : unsigned; hi, lo : integer) return unsigned;
  function rep(v : unsigned; n : natural) return unsigned;
  function mux(s : std_logic; t, f : std_logic) return std_logic;
  function mux(s : std_logic; t, f : unsigned) return unsigned;
  function mux(s : std_logic; t, f : signed) return signed;
  function mux(s : std_logic; t, f : integer) return integer;
  function mux(s : std_logic; t, f : real) return real;
  function pmux(a, b, s : unsigned) return unsigned;
  function pow(a, b : unsigned) return unsigned;
  function pow(a, b : signed) return signed;
end package reticle_pkg;

package body reticle_pkg is
  function to_sl(v : unsigned) return std_logic is
    variable n : unsigned(v'length - 1 downto 0) := v;
  begin
    return n(0);
  end function;
  function to_sl(b : boolean) return std_logic is
  begin
    if b then return '1'; else return '0'; end if;
  end function;
  function idx(v : unsigned; i : integer) return std_logic is
    variable n : unsigned(v'length - 1 downto 0) := v;
  begin
    if i < 0 or i >= n'length then return 'X'; end if;
    return n(i);
  end function;
  function slc(v : unsigned; hi, lo : integer) return unsigned is
    variable n : unsigned(v'length - 1 downto 0) := v;
    variable r : unsigned(hi - lo downto 0) := (others => 'X');
  begin
    for k in 0 to hi - lo loop
      if lo + k >= 0 and lo + k < n'length then r(k) := n(lo + k); end if;
    end loop;
    return r;
  end function;
  function rep(v : unsigned; n : natural) return unsigned is
    variable r : unsigned(v'length * n - 1 downto 0);
  begin
    for k in 0 to n - 1 loop
      r((k + 1) * v'length - 1 downto k * v'length) := v;
    end loop;
    return r;
  end function;
  function mux(s : std_logic; t, f : std_logic) return std_logic is
  begin
    if s = '1' then return t; else return f; end if;
  end function;
  function mux(s : std_logic; t, f : unsigned) return unsigned is
  begin
    if s = '1' then return t; else return f; end if;
  end function;
  function mux(s : std_logic; t, f : signed) return signed is
  begin
    if s = '1' then return t; else return f; end if;
  end function;
  function mux(s : std_logic; t, f : integer) return integer is
  begin
    if s = '1' then return t; else return f; end if;
  end function;
  function mux(s : std_logic; t, f : real) return real is
  begin
    if s = '1' then return t; else return f; end if;
  end function;
  function pmux(a, b, s : unsigned) return unsigned is
    variable r : unsigned(a'length - 1 downto 0) := a;
    variable bn : unsigned(b'length - 1 downto 0) := b;
    variable sn : unsigned(s'length - 1 downto 0) := s;
  begin
    for k in 0 to sn'length - 1 loop
      if sn(k) = '1' then r := bn((k + 1) * r'length - 1 downto k * r'length); end if;
    end loop;
    return r;
  end function;
  function pow(a, b : unsigned) return unsigned is
    variable r : unsigned(a'length - 1 downto 0) := (others => '0');
    variable base : unsigned(a'length - 1 downto 0) := a;
    variable e : unsigned(b'length - 1 downto 0) := b;
  begin
    r(0) := '1';
    for k in 0 to e'length - 1 loop
      if e(k) = '1' then r := resize(r * base, r'length); end if;
      base := resize(base * base, base'length);
    end loop;
    return r;
  end function;
  function pow(a, b : signed) return signed is
  begin
    return signed(pow(unsigned(a), unsigned(b)));
  end function;
end package body reticle_pkg;
"#;

/// Names the emitter relies on from `ieee`, `std` and its own package;
/// user objects with these names (case-insensitively) are escaped so they
/// cannot hide them.
const AVOID: &[&str] = &[
    "bit",
    "bit_vector",
    "boolean",
    "character",
    "env",
    "falling_edge",
    "idx",
    "ieee",
    "integer",
    "mux",
    "natural",
    "now",
    "numeric_std",
    "pmux",
    "pow",
    "real",
    "rep",
    "resize",
    "reticle_pkg",
    "rising_edge",
    "rtl",
    "shift_left",
    "shift_right",
    "signed",
    "slc",
    "std",
    "std_logic",
    "std_logic_1164",
    "std_logic_vector",
    "std_match",
    "std_ulogic",
    "std_ulogic_vector",
    "string",
    "time",
    "to_hstring",
    "to_integer",
    "to_ostring",
    "to_signed",
    "to_sl",
    "to_string",
    "to_unsigned",
    "unsigned",
    "work",
];

/// How a value is represented in VHDL.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum K {
    /// `std_logic`: every one-bit vector.
    Sl,
    /// `unsigned`.
    U,
    /// `signed`.
    S,
    /// `integer`.
    Int,
    /// `real`.
    Real,
    /// `string`.
    Str,
    /// A named array type.
    Arr,
}

fn kind_of(ty: &Type) -> K {
    match ty {
        Type::Bits { width: 1, .. } => K::Sl,
        Type::Bits { signed: true, .. } => K::S,
        Type::Bits { .. } => K::U,
        Type::Integer => K::Int,
        Type::Real => K::Real,
        Type::String => K::Str,
        Type::Array { .. } => K::Arr,
    }
}

/// A rendered value with its representation.
type V = (String, K);

/// Whether `text` can stand alone as an operand (a name, literal or
/// parenthesised / called form).
fn is_atomic(text: &str) -> bool {
    text.ends_with(')')
        || text.ends_with('"')
        || text.ends_with('\'')
        || text.ends_with('\\')
        || text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '\''))
}

fn paren(text: &str) -> String {
    if is_atomic(text) {
        text.to_owned()
    } else {
        format!("({text})")
    }
}

/// The bit-string literal of a constant, MSB first, for `unsigned` /
/// `signed` contexts; `wild` turns `x` and `z` into `-`.
fn bits_literal(c: &Const, wild: Option<CaseKind>) -> String {
    let mut s = String::with_capacity(c.width() as usize + 2);
    s.push('"');
    for i in (0..c.width()).rev() {
        s.push(bit_char(c.bit(i), wild));
    }
    s.push('"');
    s
}

fn bit_char(b: Bit, wild: Option<CaseKind>) -> char {
    match (b, wild) {
        (Bit::Zero, _) => '0',
        (Bit::One, _) => '1',
        (Bit::Z, Some(_)) | (Bit::X, Some(CaseKind::X)) => '-',
        (Bit::X, _) => 'X',
        (Bit::Z, _) => 'Z',
    }
}

/// Renders a constant in the representation of its own type.
fn const_value(c: &Const, wild: Option<CaseKind>) -> V {
    if c.width() == 1 {
        return (format!("'{}'", bit_char(c.bit(0), wild)), K::Sl);
    }
    let k = if c.is_signed() { K::S } else { K::U };
    let ty = if c.is_signed() { "signed" } else { "unsigned" };
    (format!("{ty}'({})", bits_literal(c, wild)), k)
}

fn severity(s: ReportSeverity) -> &'static str {
    match s {
        ReportSeverity::Note => "note",
        ReportSeverity::Warning => "warning",
        ReportSeverity::Error => "error",
        ReportSeverity::Failure => "failure",
    }
}

/// Design-level names: entity names, unique case-insensitively.
struct ModuleNames {
    names: Vec<String>,
}

impl ModuleNames {
    fn new(design: &Design) -> Self {
        let raw: Vec<&str> = design.modules.values().map(|m| m.name.as_str()).collect();
        let namer = Namer::new(raw.iter().copied());
        ModuleNames {
            names: raw.iter().map(|n| namer.name(n)).collect(),
        }
    }

    fn of(&self, id: ModuleId) -> &str {
        &self.names[id.index()]
    }
}

/// Decides which names of one namespace can be basic identifiers.
struct Namer {
    /// Lower-cased basic names that occur more than once.
    clashes: BTreeSet<String>,
    /// Lower-cased names in use (basic and extended), to derive fresh ones.
    used: BTreeSet<String>,
}

impl Namer {
    fn new<'a>(names: impl Iterator<Item = &'a str>) -> Self {
        let mut counts: BTreeMap<String, usize> = BTreeMap::new();
        let mut used = BTreeSet::new();
        for n in names {
            let lower = n.to_ascii_lowercase();
            *counts.entry(lower.clone()).or_default() += 1;
            used.insert(lower);
        }
        Namer {
            clashes: counts
                .into_iter()
                .filter(|(_, c)| *c > 1)
                .map(|(n, _)| n)
                .collect(),
            used,
        }
    }

    fn name(&self, n: &str) -> String {
        let lower = n.to_ascii_lowercase();
        if is_basic_vhdl_ident(n)
            && !self.clashes.contains(&lower)
            && AVOID.binary_search(&lower.as_str()).is_err()
            && !lower.starts_with("reticle_")
        {
            n.to_owned()
        } else {
            vhdl_extended(n)
        }
    }

    /// A basic identifier derived from `base` that no name uses.
    fn fresh(&mut self, base: &str) -> String {
        let mut candidate = base.to_owned();
        let mut i = 1;
        while self.used.contains(&candidate.to_ascii_lowercase()) {
            candidate = format!("{base}{i}");
            i += 1;
        }
        self.used.insert(candidate.to_ascii_lowercase());
        candidate
    }
}

/// A component declaration derived from an instance or black-box cell.
struct Component {
    name: String,
    generics: Vec<(String, String, String)>,
    ports: Vec<(String, &'static str, String)>,
}

impl Component {
    fn generic(&mut self, name: String, ty: String, value: String) {
        if !self.generics.iter().any(|(n, _, _)| *n == name) {
            self.generics.push((name, ty, value));
        }
    }

    fn port(&mut self, name: String, dir: &'static str, ty: String) {
        if !self.ports.iter().any(|(n, _, _)| *n == name) {
            self.ports.push((name, dir, ty));
        }
    }
}

/// Attribute declarations and specifications of one declarative region.
#[derive(Default)]
struct AttrSet {
    decls: BTreeMap<String, &'static str>,
    specs: Vec<String>,
}

struct Printer<'a> {
    design: &'a Design,
    module: &'a Module,
    module_id: ModuleId,
    module_names: &'a ModuleNames,
    namer: Namer,
    out: Out,
    uses_pkg: bool,
    /// Rendered net names, by net index.
    net_names: Vec<String>,
    /// Rendered memory names.
    mem_names: Vec<String>,
    /// Named array types, keyed by the IR type's text.
    array_types: BTreeMap<String, String>,
    /// Declarations that go before `begin`: types, constants, extra
    /// signals, components, attributes.
    type_decls: Vec<String>,
    decls: Vec<String>,
    components: BTreeMap<String, Component>,
    /// Attributes of the entity and its ports.
    entity_attrs: AttrSet,
    /// Attributes of everything declared in the architecture.
    arch_attrs: AttrSet,
    /// Shadow variables of the process being printed: net -> variable.
    shadow: BTreeMap<NetId, String>,
    /// Temporaries of the process being printed: (name, type).
    temps: Vec<(String, String)>,
    /// Labels of the enclosing loops: (break target, continue target).
    loops: Vec<(String, Option<String>)>,
    next_label: u32,
    /// The unit of `wait for` delays.
    unit: Delay,
}

impl<'a> Printer<'a> {
    fn new(design: &'a Design, module_id: ModuleId, module_names: &'a ModuleNames) -> Self {
        let module = design.module(module_id);
        let mut all: Vec<&str> = Vec::new();
        all.extend(module.ports.iter().map(|p| p.name.as_str()));
        all.extend(
            module
                .nets
                .values()
                .filter(|n| module.ports.iter().all(|p| p.name != n.name))
                .map(|n| n.name.as_str()),
        );
        all.extend(module.memories.values().map(|m| m.name.as_str()));
        all.extend(module.instances.values().map(|i| i.name.as_str()));
        all.extend(module.cells.values().map(|c| c.name.as_str()));
        all.extend(
            module
                .processes
                .values()
                .filter_map(|p| p.name.as_ref().map(|n| n.as_str())),
        );
        all.extend(module.params.iter().map(|p| p.name.as_str()));
        let namer = Namer::new(all.into_iter());
        let net_names = module
            .nets
            .values()
            .map(|n| namer.name(n.name.as_str()))
            .collect();
        let mem_names = module
            .memories
            .values()
            .map(|m| namer.name(m.name.as_str()))
            .collect();
        let unit = module
            .timescale
            .map_or(Delay::new(1, TimeUnit::Ns), |ts| ts.unit);
        Printer {
            design,
            module,
            module_id,
            module_names,
            namer,
            out: Out::new("  "),
            uses_pkg: false,
            net_names,
            mem_names,
            array_types: BTreeMap::new(),
            type_decls: Vec::new(),
            decls: Vec::new(),
            components: BTreeMap::new(),
            entity_attrs: AttrSet::default(),
            arch_attrs: AttrSet::default(),
            shadow: BTreeMap::new(),
            temps: Vec::new(),
            loops: Vec::new(),
            next_label: 0,
            unit,
        }
    }

    fn finish(self) -> String {
        self.out.finish()
    }

    fn err(&self, span: Span, message: impl Into<String>) -> EmitError {
        EmitError::new(span, message)
    }

    fn pkg(&mut self, text: String) -> String {
        self.uses_pkg = true;
        text
    }

    fn net_name(&self, net: NetId) -> String {
        match self.shadow.get(&net) {
            Some(var) => var.clone(),
            None => self.net_names[net.index()].clone(),
        }
    }

    fn net_kind(&self, net: NetId) -> K {
        kind_of(&self.module.nets[net].ty)
    }

    // --- types ----------------------------------------------------------------

    fn type_text(&mut self, ty: &Type) -> Result<String, EmitError> {
        Ok(match ty {
            Type::Bits { width: 1, .. } => "std_logic".to_owned(),
            Type::Bits { width, signed } => format!(
                "{}({} downto 0)",
                if *signed { "signed" } else { "unsigned" },
                width.saturating_sub(1)
            ),
            Type::Integer => "integer".to_owned(),
            Type::Real => "real".to_owned(),
            Type::String => "string".to_owned(),
            Type::Array { elem, len } => {
                let key = ty.to_string();
                if let Some(name) = self.array_types.get(&key) {
                    return Ok(name.clone());
                }
                let elem_text = self.type_text(elem)?;
                let name = self.namer.fresh("reticle_arr_t");
                self.type_decls.push(format!(
                    "type {name} is array (0 to {}) of {elem_text};",
                    len.saturating_sub(1)
                ));
                self.array_types.insert(key, name.clone());
                name
            }
        })
    }

    /// The type of a generic holding `value`, and the value's text.
    fn generic_of(&self, value: &AttrValue) -> (String, String) {
        match value {
            AttrValue::Int(i) => ("integer".to_owned(), i.to_string()),
            AttrValue::String(s) => ("string".to_owned(), vhdl_string(s)),
            AttrValue::Const(c) => match c.to_u64() {
                Some(v) if c.width() <= 31 => ("integer".to_owned(), v.to_string()),
                _ => (
                    format!("unsigned({} downto 0)", c.width().saturating_sub(1)),
                    bits_literal(c, None),
                ),
            },
        }
    }

    // --- attributes ---------------------------------------------------------

    /// Records the attributes of `object` (a `signal`, `label` or `entity`)
    /// in the entity's or the architecture's declarative region.
    fn attributes(&mut self, attrs: &Attrs, object: &str, class: &str, in_entity: bool) {
        for (key, value) in attrs.iter() {
            let name = self.namer.name(key.as_str());
            let (ty, text) = match value {
                AttrValue::Int(i) => ("integer", i.to_string()),
                AttrValue::String(s) => ("string", vhdl_string(s)),
                AttrValue::Const(c) => match c.to_u64() {
                    Some(v) if c.width() <= 31 => ("integer", v.to_string()),
                    _ => ("string", format!("\"{}\"", c.to_binary_string())),
                },
            };
            let set = if in_entity {
                &mut self.entity_attrs
            } else {
                &mut self.arch_attrs
            };
            match set.decls.get(&name) {
                // The same attribute name with another value type cannot be
                // declared twice in one region; keep the first.
                Some(existing) if *existing != ty => continue,
                Some(_) => {}
                None => {
                    set.decls.insert(name.clone(), ty);
                }
            }
            set.specs
                .push(format!("attribute {name} of {object} : {class} is {text};"));
        }
    }

    // --- module -------------------------------------------------------------

    fn module(&mut self) -> Result<(), EmitError> {
        let m = self.module;
        let entity = self.module_names.of(self.module_id).to_owned();
        // Render the body first: it discovers components, helper signals
        // and array types that must be declared before `begin`.
        let mut body = Out::new("  ");
        body.indent();
        std::mem::swap(&mut self.out, &mut body);
        self.body()?;
        std::mem::swap(&mut self.out, &mut body);
        let body = body.finish();

        self.out.line(&format!("entity {entity} is"));
        self.out.indent();
        if !m.params.is_empty() {
            self.out.line("generic (");
            self.out.indent();
            let count = m.params.len();
            for (i, p) in m.params.iter().enumerate() {
                let (ty, value) = self.generic_of(&p.value);
                let sep = if i + 1 < count { ";" } else { "" };
                self.out.line(&format!(
                    "{} : {ty} := {value}{sep}",
                    self.namer.name(p.name.as_str())
                ));
            }
            self.out.dedent();
            self.out.line(");");
        }
        if !m.ports.is_empty() {
            self.out.line("port (");
            self.out.indent();
            let count = m.ports.len();
            for (i, p) in m.ports.iter().enumerate() {
                let net = &m.nets[p.net];
                let ty = self.type_text(&net.ty)?;
                let dir = match p.dir {
                    PortDir::In => "in",
                    PortDir::Out => "out",
                    PortDir::InOut => "inout",
                };
                let sep = if i + 1 < count { ";" } else { "" };
                self.out.line(&format!(
                    "{} : {dir} {ty}{sep}",
                    self.namer.name(p.name.as_str())
                ));
            }
            self.out.dedent();
            self.out.line(");");
        }
        self.attributes(&m.attrs, &entity, "entity", true);
        let entity_attrs = std::mem::take(&mut self.entity_attrs);
        for (name, ty) in &entity_attrs.decls {
            self.out.line(&format!("attribute {name} : {ty};"));
        }
        for spec in &entity_attrs.specs {
            self.out.line(spec);
        }
        self.out.dedent();
        self.out.line(&format!("end entity {entity};"));
        self.out.blank();
        self.out.line(&format!("architecture rtl of {entity} is"));
        self.out.indent();
        for t in std::mem::take(&mut self.type_decls) {
            self.out.line(&t);
        }
        let arch_attrs = std::mem::take(&mut self.arch_attrs);
        for (name, ty) in &arch_attrs.decls {
            self.out.line(&format!("attribute {name} : {ty};"));
        }
        for d in std::mem::take(&mut self.decls) {
            self.out.line(&d);
        }
        for spec in &arch_attrs.specs {
            self.out.line(spec);
        }
        let components = std::mem::take(&mut self.components);
        for comp in components.values() {
            self.component(comp);
        }
        self.out.dedent();
        self.out.line("begin");
        self.out.raw(&body);
        self.out.line("end architecture rtl;");
        Ok(())
    }

    fn component(&mut self, comp: &Component) {
        self.out.line(&format!("component {} is", comp.name));
        self.out.indent();
        if !comp.generics.is_empty() {
            self.out.line("generic (");
            self.out.indent();
            let n = comp.generics.len();
            for (i, (name, ty, value)) in comp.generics.iter().enumerate() {
                let sep = if i + 1 < n { ";" } else { "" };
                self.out.line(&format!("{name} : {ty} := {value}{sep}"));
            }
            self.out.dedent();
            self.out.line(");");
        }
        if !comp.ports.is_empty() {
            self.out.line("port (");
            self.out.indent();
            let n = comp.ports.len();
            for (i, (name, dir, ty)) in comp.ports.iter().enumerate() {
                let sep = if i + 1 < n { ";" } else { "" };
                self.out.line(&format!("{name} : {dir} {ty}{sep}"));
            }
            self.out.dedent();
            self.out.line(");");
        }
        self.out.dedent();
        self.out.line("end component;");
    }

    /// Declares the nets that are not ports, the memories, and renders
    /// every concurrent statement.
    fn body(&mut self) -> Result<(), EmitError> {
        let m = self.module;
        // Ports whose net has a different name: alias the net to the port.
        for p in &m.ports {
            let net = &m.nets[p.net];
            if p.name != net.name {
                let ty = self.type_text(&net.ty)?;
                let decl = format!(
                    "alias {} : {ty} is {};",
                    self.net_names[p.net.index()],
                    self.namer.name(p.name.as_str())
                );
                self.decls.push(decl);
            }
        }
        for (id, net) in m.nets.iter() {
            let object = self.net_names[id.index()].clone();
            match m.port_of_net(id) {
                Some(port) => {
                    let name = self.namer.name(port.name.as_str());
                    self.attributes(&net.attrs, &name, "signal", true);
                }
                None => {
                    let ty = self.type_text(&net.ty)?;
                    self.decls.push(format!("signal {object} : {ty};"));
                    self.attributes(&net.attrs, &object, "signal", false);
                }
            }
        }
        for (id, mem) in m.memories.iter() {
            let elem = self.type_text(&mem.elem)?;
            let name = self.mem_names[id.index()].clone();
            let ty = self.namer.fresh(&format!("{}_t", name.trim_matches('\\')));
            self.type_decls.push(format!(
                "type {ty} is array (0 to {}) of {elem};",
                mem.size.saturating_sub(1)
            ));
            let init = match &mem.init {
                Some(values) if !values.is_empty() && mem.elem.is_bits() => {
                    let items: Vec<String> = values
                        .iter()
                        .map(|c| const_value(c, None).0)
                        .map(|t| match t.split_once("'(") {
                            Some((_, rest)) => rest.trim_end_matches(')').to_owned(),
                            None => t,
                        })
                        .collect();
                    if u64::try_from(values.len()).unwrap_or(u64::MAX) == mem.size {
                        format!(" := ({})", items.join(", "))
                    } else {
                        let named: Vec<String> = items
                            .iter()
                            .enumerate()
                            .map(|(i, v)| format!("{i} => {v}"))
                            .collect();
                        let rest = if mem.elem.is_bit() {
                            "'U'"
                        } else {
                            "(others => 'U')"
                        };
                        format!(" := ({}, others => {rest})", named.join(", "))
                    }
                }
                _ => String::new(),
            };
            self.decls.push(format!("signal {name} : {ty}{init};"));
            self.attributes(&mem.attrs, &name, "signal", false);
        }
        for assign in &m.assigns {
            let (value, k) = self.expr(assign.value)?;
            let target = self.lvalue_text(&assign.target, true)?;
            let text = self.conv((value, k), target.1);
            let after = match assign.delay {
                Some(d) => format!(" after {}", self.delay(d)),
                None => String::new(),
            };
            self.out.line(&format!("{} <= {text}{after};", target.0));
        }
        for (_, process) in m.processes.iter() {
            self.process(process)?;
        }
        for (_, cell) in m.cells.iter() {
            self.cell(cell)?;
        }
        for (id, inst) in m.instances.iter() {
            self.instance(id, inst)?;
        }
        Ok(())
    }

    fn delay(&self, d: Delay) -> String {
        let fs = d.to_fs();
        for unit in TimeUnit::ALL.iter().rev() {
            if fs.is_multiple_of(unit.in_fs()) {
                return format!("{} {}", fs / unit.in_fs(), unit.name());
            }
        }
        format!("{fs} fs")
    }

    // --- instances ----------------------------------------------------------

    /// True when something in this module drives `net`, or an instance
    /// before `before` connects it: the guess for the direction of a port
    /// of an unresolved module is "output" for the first instance that
    /// touches an otherwise undriven net.
    fn driven(&self, net: NetId, before: InstanceId) -> bool {
        let m = self.module;
        if m.port_of_net(net).is_some_and(|p| p.dir != PortDir::Out) {
            return true;
        }
        if m.assigns.iter().any(|a| a.target.nets().contains(&net)) {
            return true;
        }
        if m.cells
            .values()
            .any(|c| c.outputs.iter().any(|(_, n)| *n == net))
        {
            return true;
        }
        if m.instances.iter().any(|(id, inst)| {
            id < before
                && inst
                    .connections
                    .iter()
                    .any(|(_, e)| self.expr_net(*e) == Some(net))
        }) {
            return true;
        }
        let mut written = false;
        m.for_each_stmt(|s| {
            if let StmtKind::Assign { target, .. } = &s.kind
                && target.nets().contains(&net)
            {
                written = true;
            }
        });
        written
    }

    /// The net an expression names, when it is one net (or a slice of one).
    fn expr_net(&self, id: ExprId) -> Option<NetId> {
        match &self.module.exprs[id].kind {
            ExprKind::Net(n) => Some(*n),
            ExprKind::Slice { base, .. } | ExprKind::Index { base, .. } => self.expr_net(*base),
            _ => None,
        }
    }

    fn instance(&mut self, inst_id: InstanceId, inst: &Instance) -> Result<(), EmitError> {
        let label = self.namer.name(inst.name.as_str());
        let target = inst.module.id().map(|id| self.design.module(id));
        // (port name, direction, formal kind)
        let mut formals: Vec<(String, PortDir, K, Type)> = Vec::new();
        let mut head;
        match target {
            Some(t) if !t.blackbox => {
                head = format!(
                    "entity work.{}",
                    self.module_names.of(inst.module.id().expect("resolved"))
                );
                for (port, _) in &inst.connections {
                    let p = t.port(port.as_str()).ok_or_else(|| {
                        self.err(
                            inst.span,
                            format!("instance `{}` connects unknown port `{port}`", inst.name),
                        )
                    })?;
                    let ty = t.nets[p.net].ty.clone();
                    formals.push((vhdl_ident(port.as_str()), p.dir, kind_of(&ty), ty));
                }
            }
            _ => {
                let name = match &inst.module {
                    ModuleRef::Resolved(id) => self.design.module(*id).name.as_str(),
                    ModuleRef::Unresolved(n) => n.as_str(),
                };
                let comp_name = vhdl_ident(name);
                head = comp_name.clone();
                let mut comp = self
                    .components
                    .remove(&comp_name)
                    .unwrap_or_else(|| Component {
                        name: comp_name.clone(),
                        generics: Vec::new(),
                        ports: Vec::new(),
                    });
                match target {
                    Some(t) => {
                        for p in &t.params {
                            let (ty, value) = self.generic_of(&p.value);
                            comp.generic(vhdl_ident(p.name.as_str()), ty, value);
                        }
                        for p in &t.ports {
                            let net = &t.nets[p.net];
                            let ty = self.type_text(&net.ty)?;
                            let dir = match p.dir {
                                PortDir::In => "in",
                                PortDir::Out => "out",
                                PortDir::InOut => "inout",
                            };
                            comp.port(vhdl_ident(p.name.as_str()), dir, ty);
                        }
                        for (port, _) in &inst.connections {
                            let p = t.port(port.as_str()).ok_or_else(|| {
                                self.err(
                                    inst.span,
                                    format!(
                                        "instance `{}` connects unknown port `{port}`",
                                        inst.name
                                    ),
                                )
                            })?;
                            let ty = t.nets[p.net].ty.clone();
                            formals.push((vhdl_ident(port.as_str()), p.dir, kind_of(&ty), ty));
                        }
                    }
                    None => {
                        for (port, e) in &inst.connections {
                            let ty = self.module.exprs[*e].ty.clone();
                            let dir = match self.expr_net(*e) {
                                Some(n) if !self.driven(n, inst_id) => PortDir::Out,
                                _ => PortDir::In,
                            };
                            let ty_text = self.type_text(&ty)?;
                            let dir_text = if dir == PortDir::Out { "out" } else { "in" };
                            comp.port(vhdl_ident(port.as_str()), dir_text, ty_text);
                            formals.push((vhdl_ident(port.as_str()), dir, kind_of(&ty), ty));
                        }
                    }
                }
                for (name, value) in inst.params.iter() {
                    let (ty, value) = self.generic_of(value);
                    comp.generic(vhdl_ident(name.as_str()), ty, value);
                }
                self.components.insert(comp_name, comp);
            }
        }
        if !inst.params.is_empty() {
            let items: Vec<String> = inst
                .params
                .iter()
                .map(|(k, v)| format!("{} => {}", vhdl_ident(k.as_str()), self.generic_of(v).1))
                .collect();
            let _ = write!(head, " generic map ({})", items.join(", "));
        }
        let mut assocs = Vec::new();
        for ((port, dir, fk, _), (_, e)) in formals.iter().zip(&inst.connections) {
            let actual = self.actual(*e, *dir, *fk, &label, port)?;
            assocs.push(format!("{port} => {actual}"));
        }
        self.attributes(&inst.attrs, &label, "label", false);
        if assocs.is_empty() {
            self.out.line(&format!("{label} : {head};"));
        } else {
            self.out.line(&format!("{label} : {head} port map ("));
            self.out.indent();
            let n = assocs.len();
            for (i, a) in assocs.iter().enumerate() {
                let sep = if i + 1 < n { "," } else { "" };
                self.out.line(&format!("{a}{sep}"));
            }
            self.out.dedent();
            self.out.line(");");
        }
        Ok(())
    }

    /// The actual of a port association, converted to the formal's
    /// representation; a concatenation driven by an output goes through a
    /// helper signal that is split afterwards.
    fn actual(
        &mut self,
        e: ExprId,
        dir: PortDir,
        fk: K,
        label: &str,
        port: &str,
    ) -> Result<String, EmitError> {
        let expr = &self.module.exprs[e];
        if dir != PortDir::In
            && let ExprKind::Concat(parts) = &expr.kind
        {
            let parts = parts.clone();
            let ty = expr.ty.clone();
            let tmp = self.namer.fresh(&format!(
                "reticle_{}_{}",
                label.trim_matches('\\'),
                port.trim_matches('\\')
            ));
            let ty_text = self.type_text(&ty)?;
            self.decls.push(format!("signal {tmp} : {ty_text};"));
            let mut hi = ty.width().unwrap_or(0);
            for part in parts {
                let w = self.module.exprs[part].ty.width().unwrap_or(0);
                let lo = hi - w;
                let (target, tk) = self.expr(part)?;
                let piece = if w == 1 {
                    (format!("{tmp}({lo})"), K::Sl)
                } else {
                    (format!("{tmp}({} downto {lo})", hi - 1), K::U)
                };
                let value = self.conv(piece, tk);
                self.out.line(&format!("{target} <= {value};"));
                hi = lo;
            }
            return Ok(tmp);
        }
        let (text, k) = self.expr(e)?;
        Ok(match (dir, k == fk) {
            (_, true) => text,
            (PortDir::In, false) => self.conv((text, k), fk),
            // For an output the conversion applies to the formal side; the
            // widths match, so the actual keeps its own type.
            (_, false) => text,
        })
    }

    // --- processes ----------------------------------------------------------

    /// Nets assigned with blocking assignments inside `block`.
    fn blocking_nets(block: &Block, out: &mut BTreeSet<NetId>) {
        for stmt in block {
            match &stmt.kind {
                StmtKind::Assign {
                    target,
                    kind: AssignKind::Blocking,
                    ..
                } => out.extend(target.nets()),
                StmtKind::For { init, step, .. } => {
                    if let Some((lv, _)) = init {
                        out.extend(lv.nets());
                    }
                    if let Some((lv, _)) = step {
                        out.extend(lv.nets());
                    }
                }
                _ => {}
            }
            for b in stmt.blocks() {
                Self::blocking_nets(b, out);
            }
        }
    }

    fn edge_text(&self, e: &Edge) -> String {
        let name = self.net_names[e.net.index()].clone();
        match e.polarity {
            Polarity::Pos => format!("rising_edge({name})"),
            Polarity::Neg => format!("falling_edge({name})"),
            Polarity::Any => format!("{name}'event"),
        }
    }

    /// Recognises `if rst ... else ...` on an asynchronous reset net and
    /// returns the VHDL condition.
    fn reset_condition(&self, cond: ExprId, resets: &[Edge]) -> Option<String> {
        let expr = &self.module.exprs[cond];
        let (net, active_high) = match &expr.kind {
            ExprKind::Net(n) => (*n, true),
            ExprKind::Unary {
                op: UnaryOp::Not | UnaryOp::LogicNot,
                expr,
            } => (self.module.exprs[*expr].as_net()?, false),
            _ => return None,
        };
        let edge = resets.iter().find(|e| e.net == net)?;
        let ok = match edge.polarity {
            Polarity::Pos => active_high,
            Polarity::Neg => !active_high,
            Polarity::Any => true,
        };
        ok.then(|| {
            format!(
                "{} = '{}'",
                self.net_names[net.index()],
                if active_high { '1' } else { '0' }
            )
        })
    }

    fn process(&mut self, process: &Process) -> Result<(), EmitError> {
        let mut blocking = BTreeSet::new();
        Self::blocking_nets(&process.body, &mut blocking);
        self.shadow.clear();
        self.temps.clear();
        for net in blocking {
            let base = format!("{}_v", self.net_names[net.index()].trim_matches('\\'));
            let var = self.namer.fresh(&base);
            self.shadow.insert(net, var);
        }
        let label = process.name.as_ref().map(|n| self.namer.name(n.as_str()));
        let head = match &process.kind {
            ProcessKind::Comb => "process (all)".to_owned(),
            ProcessKind::Sequential { clocks, resets } => {
                let names: Vec<String> = clocks
                    .iter()
                    .chain(resets)
                    .map(|e| self.net_names[e.net.index()].clone())
                    .collect();
                format!("process ({})", names.join(", "))
            }
            ProcessKind::Initial | ProcessKind::Free => "process".to_owned(),
            ProcessKind::Sensitive(nets) => {
                if nets.is_empty() {
                    "process (all)".to_owned()
                } else {
                    let names: Vec<String> = nets
                        .iter()
                        .map(|n| self.net_names[n.index()].clone())
                        .collect();
                    format!("process ({})", names.join(", "))
                }
            }
        };
        // Body first, so temporaries are known before the declarations.
        let mut body = Out::new("  ");
        body.indent();
        body.indent();
        std::mem::swap(&mut self.out, &mut body);
        self.load_shadows();
        match &process.kind {
            ProcessKind::Sequential { clocks, resets } => {
                let clock_cond = clocks
                    .iter()
                    .map(|e| self.edge_text(e))
                    .collect::<Vec<_>>()
                    .join(" or ");
                let async_edges: Vec<String> = resets.iter().map(|e| self.edge_text(e)).collect();
                let reset = match process.body.as_slice() {
                    [
                        Stmt {
                            kind: StmtKind::If { cond, then_, else_ },
                            ..
                        },
                    ] => self
                        .reset_condition(*cond, resets)
                        .map(|c| (c, then_, else_)),
                    _ => None,
                };
                match reset {
                    Some((cond, then_, else_)) => {
                        self.out.line(&format!("if {cond} then"));
                        self.out.indent();
                        self.block(then_)?;
                        self.out.dedent();
                        self.out.line(&format!("elsif {clock_cond} then"));
                        self.out.indent();
                        self.block(else_)?;
                        self.out.dedent();
                        self.out.line("end if;");
                    }
                    None => {
                        let mut cond = clock_cond;
                        for e in async_edges {
                            let _ = write!(cond, " or {e}");
                        }
                        self.out.line(&format!("if {cond} then"));
                        self.out.indent();
                        self.block(&process.body)?;
                        self.out.dedent();
                        self.out.line("end if;");
                    }
                }
            }
            _ => {
                self.block(&process.body)?;
            }
        }
        self.store_shadows();
        if matches!(process.kind, ProcessKind::Initial) {
            self.out.line("wait;");
        }
        std::mem::swap(&mut self.out, &mut body);
        let body = body.finish();

        if let Some(label) = &label {
            self.attributes(&process.attrs, label, "label", false);
        }
        let label_text = label.map(|l| format!("{l} : ")).unwrap_or_default();
        let shadows: Vec<(NetId, String)> =
            self.shadow.iter().map(|(n, v)| (*n, v.clone())).collect();
        let temps = std::mem::take(&mut self.temps);
        if shadows.is_empty() && temps.is_empty() {
            self.out.line(&format!("{label_text}{head}"));
        } else {
            self.out.line(&format!("{label_text}{head}"));
            self.out.indent();
            for (net, var) in &shadows {
                let ty = self.module.nets[*net].ty.clone();
                let ty = self.type_text(&ty)?;
                self.out.line(&format!("variable {var} : {ty};"));
            }
            for (name, ty) in &temps {
                self.out.line(&format!("variable {name} : {ty};"));
            }
            self.out.dedent();
        }
        self.out.line("begin");
        self.out.raw(&body);
        self.out.line("end process;");
        self.shadow.clear();
        Ok(())
    }

    fn load_shadows(&mut self) {
        for (net, var) in self.shadow.clone() {
            self.out
                .line(&format!("{var} := {};", self.net_names[net.index()]));
        }
    }

    fn store_shadows(&mut self) {
        for (net, var) in self.shadow.clone() {
            self.out
                .line(&format!("{} <= {var};", self.net_names[net.index()]));
        }
    }

    fn temp(&mut self, ty: String) -> String {
        let name = self.namer.fresh("reticle_tmp");
        self.temps.push((name.clone(), ty));
        name
    }

    fn block(&mut self, block: &Block) -> Result<(), EmitError> {
        if block.is_empty() {
            self.out.line("null;");
        }
        for stmt in block {
            self.stmt(stmt)?;
        }
        Ok(())
    }

    /// `cond = '1'` for a single-bit condition.
    fn condition(&mut self, cond: ExprId) -> Result<String, EmitError> {
        let (text, k) = self.expr(cond)?;
        Ok(match k {
            K::Sl => format!("{} = '1'", paren(&text)),
            K::U | K::S => format!("{} /= 0", paren(&text)),
            K::Int | K::Real => format!("{} /= 0", paren(&text)),
            K::Str | K::Arr => format!("{} = '1'", paren(&text)),
        })
    }

    /// The target of an assignment and its representation; `signal`
    /// bypasses the process's shadow variables (non-blocking assignments
    /// always write the signal).
    fn lvalue_text(&mut self, lv: &Lvalue, signal: bool) -> Result<V, EmitError> {
        let name_of = |this: &Self, net: NetId| {
            if signal {
                this.net_names[net.index()].clone()
            } else {
                this.net_name(net)
            }
        };
        Ok(match lv {
            Lvalue::Net(net) => (name_of(self, *net), self.net_kind(*net)),
            Lvalue::Slice { net, hi, lo } => {
                let name = name_of(self, *net);
                let base_k = self.net_kind(*net);
                if hi == lo {
                    if base_k == K::Arr {
                        (format!("{name}({lo})"), self.elem_kind(*net))
                    } else {
                        (format!("{name}({lo})"), K::Sl)
                    }
                } else if base_k == K::Arr {
                    (format!("{name}({lo} to {hi})"), K::Arr)
                } else {
                    (format!("{name}({hi} downto {lo})"), base_k)
                }
            }
            Lvalue::Index { net, index } => {
                let name = name_of(self, *net);
                let base_k = self.net_kind(*net);
                let idx = self.index_text(*index)?;
                let k = if base_k == K::Arr {
                    self.elem_kind(*net)
                } else {
                    K::Sl
                };
                (format!("{name}({idx})"), k)
            }
            Lvalue::Concat(_) => {
                return Err(self.err(
                    Span::new(self.module.span.file, 0, 0),
                    "concatenation targets are split by the caller",
                ));
            }
            Lvalue::MemElem { mem, addr } => {
                let name = self.mem_names[mem.index()].clone();
                let idx = self.index_text(*addr)?;
                (
                    format!("{name}({idx})"),
                    kind_of(&self.module.memories[*mem].elem),
                )
            }
        })
    }

    fn elem_kind(&self, net: NetId) -> K {
        match &self.module.nets[net].ty {
            Type::Array { elem, .. } => kind_of(elem),
            other => kind_of(other),
        }
    }

    /// Assigns `value` to `target` (splitting concatenation targets).
    fn assign(
        &mut self,
        target: &Lvalue,
        value: V,
        blocking: bool,
        delay: Option<Delay>,
    ) -> Result<(), EmitError> {
        if let Lvalue::Concat(parts) = target {
            let (text, k) = value;
            let width: u32 = parts.iter().map(|p| self.lvalue_width(p)).sum();
            let ty = if width == 1 {
                "std_logic".to_owned()
            } else {
                format!("unsigned({} downto 0)", width - 1)
            };
            let tmp = self.temp(ty);
            let value = self.conv((text, k), if width == 1 { K::Sl } else { K::U });
            self.out.line(&format!("{tmp} := {value};"));
            let mut hi = width;
            for part in parts {
                let w = self.lvalue_width(part);
                let lo = hi - w;
                let piece = if width == 1 {
                    (tmp.clone(), K::Sl)
                } else if w == 1 {
                    (format!("{tmp}({lo})"), K::Sl)
                } else {
                    (format!("{tmp}({} downto {lo})", hi - 1), K::U)
                };
                self.assign(part, piece, blocking, delay)?;
                hi = lo;
            }
            return Ok(());
        }
        let shadowed = matches!(target, Lvalue::Net(n) | Lvalue::Slice { net: n, .. } | Lvalue::Index { net: n, .. } if self.shadow.contains_key(n));
        let (tname, tk) = self.lvalue_text(target, !blocking)?;
        let text = self.conv(value, tk);
        if blocking && shadowed {
            match delay {
                Some(d) => {
                    let ty = self.kind_type_text(tk, &tname);
                    let tmp = self.temp(ty);
                    self.out.line(&format!("{tmp} := {text};"));
                    self.out.line(&format!("wait for {};", self.delay(d)));
                    self.out.line(&format!("{tname} := {tmp};"));
                }
                None => self.out.line(&format!("{tname} := {text};")),
            }
        } else {
            match delay {
                Some(d) => self.out.line(&format!(
                    "{tname} <= transport {text} after {};",
                    self.delay(d)
                )),
                None => self.out.line(&format!("{tname} <= {text};")),
            }
        }
        Ok(())
    }

    /// A subtype text for a temporary holding the value of `target`.
    fn kind_type_text(&self, k: K, target: &str) -> String {
        match k {
            K::Sl => "std_logic".to_owned(),
            K::Int => "integer".to_owned(),
            K::Real => "real".to_owned(),
            K::Str => "string".to_owned(),
            K::U | K::S | K::Arr => format!("{target}'subtype"),
        }
    }

    fn lvalue_width(&self, lv: &Lvalue) -> u32 {
        match lv {
            Lvalue::Net(n) => self.module.nets[*n].ty.width().unwrap_or(0),
            Lvalue::Slice { hi, lo, .. } => hi - lo + 1,
            Lvalue::Index { .. } => 1,
            Lvalue::Concat(parts) => parts.iter().map(|p| self.lvalue_width(p)).sum(),
            Lvalue::MemElem { mem, .. } => self.module.memories[*mem].elem.width().unwrap_or(0),
        }
    }

    fn stmt(&mut self, stmt: &Stmt) -> Result<(), EmitError> {
        match &stmt.kind {
            StmtKind::Assign {
                target,
                value,
                kind,
                delay,
            } => {
                let v = self.expr(*value)?;
                self.assign(target, v, *kind == AssignKind::Blocking, *delay)?;
            }
            StmtKind::If { cond, then_, else_ } => {
                let cond = self.condition(*cond)?;
                self.if_chain(&format!("if {cond} then"), then_, else_)?;
            }
            StmtKind::Case {
                subject,
                kind,
                arms,
                default,
                ..
            } => self.case(*subject, *kind, arms, default.as_ref())?,
            StmtKind::For {
                init,
                cond,
                step,
                body,
            } => {
                if let Some((lv, e)) = init {
                    let v = self.expr(*e)?;
                    self.assign(lv, v, true, None)?;
                }
                let head = match cond {
                    Some(c) => format!("while {} loop", self.condition(*c)?),
                    None => "loop".to_owned(),
                };
                self.loop_body(&head, body, step.as_ref())?;
            }
            StmtKind::While { cond, body } => {
                let head = format!("while {} loop", self.condition(*cond)?);
                self.loop_body(&head, body, None)?;
            }
            StmtKind::Repeat { count, body } => {
                let count = self.index_text(*count)?;
                self.next_label += 1;
                let var = format!("reticle_i_{}", self.next_label);
                self.loop_body(&format!("for {var} in 1 to {count} loop"), body, None)?;
            }
            StmtKind::Forever { body } => self.loop_body("loop", body, None)?,
            StmtKind::Block { name, body } => {
                if let Some(name) = name {
                    self.out.line(&format!("-- block {name}"));
                }
                for s in body {
                    self.stmt(s)?;
                }
            }
            StmtKind::Wait(kind) => {
                self.store_shadows();
                match kind {
                    WaitKind::Delay(e) => {
                        let expr = &self.module.exprs[*e];
                        let text = match expr.as_const().and_then(|c| c.to_u64()) {
                            Some(n) => self.delay(Delay::new(
                                n.saturating_mul(self.unit.value),
                                self.unit.unit,
                            )),
                            None => {
                                let n = self.index_text(*e)?;
                                format!("{n} * {} {}", self.unit.value, self.unit.unit.name())
                            }
                        };
                        self.out.line(&format!("wait for {text};"));
                    }
                    WaitKind::Event(edges) => {
                        if edges.iter().all(|e| e.polarity == Polarity::Any) {
                            let names: Vec<String> = edges
                                .iter()
                                .map(|e| self.net_names[e.net.index()].clone())
                                .collect();
                            self.out.line(&format!("wait on {};", names.join(", ")));
                        } else {
                            let conds: Vec<String> =
                                edges.iter().map(|e| self.edge_text(e)).collect();
                            self.out
                                .line(&format!("wait until {};", conds.join(" or ")));
                        }
                    }
                    WaitKind::Until(e) => {
                        let cond = self.condition(*e)?;
                        self.out
                            .line(&format!("if not ({cond}) then wait until {cond}; end if;"));
                    }
                }
                self.load_shadows();
            }
            StmtKind::SysCall { name, args } => self.syscall(name.as_str(), args)?,
            StmtKind::MemFile { op, .. } => {
                let name = op.keyword();
                self.out
                    .line(&format!("null; -- ${name} has no VHDL equivalent"));
            }
            StmtKind::MemWrite {
                mem,
                addr,
                value,
                enable,
            } => {
                let name = self.mem_names[mem.index()].clone();
                let idx = self.index_text(*addr)?;
                let v = self.expr(*value)?;
                let text = self.conv(v, kind_of(&self.module.memories[*mem].elem));
                let line = format!("{name}({idx}) <= {text};");
                match enable {
                    Some(en) => {
                        let cond = self.condition(*en)?;
                        self.out.line(&format!("if {cond} then {line} end if;"));
                    }
                    None => self.out.line(&line),
                }
            }
            StmtKind::Assert {
                cond,
                severity: sev,
                message,
            } => {
                let cond = self.condition(*cond)?;
                let report = if message.is_empty() {
                    String::new()
                } else {
                    format!(" report {}", self.format_args(message)?)
                };
                self.out.line(&format!(
                    "assert {cond}{report} severity {};",
                    severity(*sev)
                ));
            }
            StmtKind::Finish => self.out.line("std.env.finish;"),
            StmtKind::Stop => self.out.line("std.env.stop;"),
            StmtKind::Break => match self.loops.last() {
                Some((label, _)) => self.out.line(&format!("exit {label};")),
                None => return Err(self.err(stmt.span, "`break` outside a loop")),
            },
            StmtKind::Continue => match self.loops.last() {
                Some((_, Some(label))) => self.out.line(&format!("exit {label};")),
                Some((label, None)) => self.out.line(&format!("next {label};")),
                None => return Err(self.err(stmt.span, "`continue` outside a loop")),
            },
        }
        Ok(())
    }

    fn if_chain(&mut self, head: &str, then_: &Block, else_: &Block) -> Result<(), EmitError> {
        self.out.line(head);
        self.out.indent();
        self.block(then_)?;
        self.out.dedent();
        if else_.is_empty() {
            self.out.line("end if;");
            return Ok(());
        }
        if let [
            Stmt {
                kind: StmtKind::If { cond, then_, else_ },
                ..
            },
        ] = else_.as_slice()
        {
            let cond = self.condition(*cond)?;
            return self.if_chain(&format!("elsif {cond} then"), then_, else_);
        }
        self.out.line("else");
        self.out.indent();
        self.block(else_)?;
        self.out.dedent();
        self.out.line("end if;");
        Ok(())
    }

    fn case(
        &mut self,
        subject: ExprId,
        kind: CaseKind,
        arms: &[super::super::process::CaseArm],
        default: Option<&Block>,
    ) -> Result<(), EmitError> {
        let all_const = arms
            .iter()
            .flat_map(|a| a.values.iter())
            .all(|v| self.module.exprs[*v].as_const().is_some());
        let (subject_text, sk) = self.expr(subject)?;
        if !all_const {
            // An `if` chain; wildcards only apply to constant items.
            let mut first = true;
            for arm in arms {
                let mut conds = Vec::new();
                for v in &arm.values {
                    let c = self.match_condition(&subject_text, sk, *v, kind)?;
                    conds.push(c);
                }
                let keyword = if first { "if" } else { "elsif" };
                first = false;
                self.out
                    .line(&format!("{keyword} {} then", conds.join(" or ")));
                self.out.indent();
                self.block(&arm.body)?;
                self.out.dedent();
            }
            if let Some(default) = default {
                if first {
                    self.block(default)?;
                    return Ok(());
                }
                self.out.line("else");
                self.out.indent();
                self.block(default)?;
                self.out.dedent();
            }
            if !first {
                self.out.line("end if;");
            }
            return Ok(());
        }
        // `case` needs a subject with a locally static subtype: a signal
        // or variable name, or a temporary.
        let is_name = matches!(
            self.module.exprs[subject].kind,
            ExprKind::Net(_) | ExprKind::Slice { .. }
        );
        let subject_text = if is_name || matches!(sk, K::Sl | K::Int) {
            subject_text
        } else {
            let ty = match &self.module.exprs[subject].ty {
                Type::Bits { width, signed } => format!(
                    "{}({} downto 0)",
                    if *signed { "signed" } else { "unsigned" },
                    width.saturating_sub(1)
                ),
                other => other.to_string(),
            };
            let tmp = self.temp(ty);
            self.out.line(&format!("{tmp} := {subject_text};"));
            tmp
        };
        let wild = match kind {
            CaseKind::Plain => None,
            _ => Some(kind),
        };
        let keyword = if wild.is_some() && sk != K::Sl && sk != K::Int {
            "case?"
        } else {
            "case"
        };
        self.out.line(&format!("{keyword} {subject_text} is"));
        self.out.indent();
        for arm in arms {
            let choices: Vec<String> = arm
                .values
                .iter()
                .map(|v| {
                    let c = self.module.exprs[*v].as_const().expect("constant");
                    match sk {
                        K::Sl => format!("'{}'", bit_char(c.bit(0), wild)),
                        K::Int => c.to_i64().unwrap_or(0).to_string(),
                        _ => bits_literal(c, wild),
                    }
                })
                .collect();
            self.out.line(&format!("when {} =>", choices.join(" | ")));
            self.out.indent();
            self.block(&arm.body)?;
            self.out.dedent();
        }
        self.out.line("when others =>");
        self.out.indent();
        match default {
            Some(d) => self.block(d)?,
            None => self.out.line("null;"),
        }
        self.out.dedent();
        self.out.dedent();
        self.out.line(&format!("end {keyword};"));
        Ok(())
    }

    /// One `case` item as a condition on the subject.
    fn match_condition(
        &mut self,
        subject: &str,
        sk: K,
        value: ExprId,
        kind: CaseKind,
    ) -> Result<String, EmitError> {
        let expr = &self.module.exprs[value];
        if let (Some(c), CaseKind::Z | CaseKind::X) = (expr.as_const(), kind)
            && c.width() > 1
        {
            let lit = bits_literal(c, Some(kind));
            let s = self.conv((subject.to_owned(), sk), K::U);
            return Ok(format!("std_match({s}, {lit})"));
        }
        let v = self.expr(value)?;
        let v = self.conv(v, sk);
        Ok(format!("{} = {}", paren(subject), paren(&v)))
    }

    fn loop_body(
        &mut self,
        head: &str,
        body: &Block,
        step: Option<&(Lvalue, ExprId)>,
    ) -> Result<(), EmitError> {
        let needs_continue = contains_direct(body, &|k| matches!(k, StmtKind::Continue));
        self.next_label += 1;
        let n = self.next_label;
        let outer = format!("reticle_loop_{n}");
        let inner = (needs_continue && step.is_some()).then(|| format!("reticle_body_{n}"));
        self.out.line(&format!("{outer} : {head}"));
        self.out.indent();
        self.loops.push((outer.clone(), inner.clone()));
        if let Some(inner) = &inner {
            self.out.line(&format!("{inner} : loop"));
            self.out.indent();
            self.block(body)?;
            self.out.line(&format!("exit {inner};"));
            self.out.dedent();
            self.out.line(&format!("end loop {inner};"));
        } else {
            self.block(body)?;
        }
        if let Some((lv, e)) = step {
            let v = self.expr(*e)?;
            self.assign(lv, v, true, None)?;
        }
        self.loops.pop();
        self.out.dedent();
        self.out.line(&format!("end loop {outer};"));
        Ok(())
    }

    fn syscall(&mut self, name: &str, args: &[ExprId]) -> Result<(), EmitError> {
        match name {
            "$display" | "$write" | "$strobe" | "$monitor" | "$displayb" | "$displayh"
            | "$displayo" | "$writeb" | "$writeh" | "$writeo" | "$info" | "report" => {
                let text = if args.is_empty() {
                    "\"\"".to_owned()
                } else {
                    self.format_args(args)?
                };
                self.out.line(&format!("report {text};"));
            }
            "$warning" | "$error" | "$fatal" => {
                let text = if args.is_empty() {
                    "\"\"".to_owned()
                } else {
                    self.format_args(args)?
                };
                let sev = match name {
                    "$warning" => "warning",
                    "$error" => "error",
                    _ => "failure",
                };
                self.out.line(&format!("report {text} severity {sev};"));
            }
            "$finish" => self.out.line("std.env.finish;"),
            "$stop" => self.out.line("std.env.stop;"),
            _ => {
                self.out
                    .line(&format!("null; -- {name} has no VHDL equivalent"));
            }
        }
        Ok(())
    }

    /// Converts `$display`-style arguments to a string expression.
    fn format_args(&mut self, args: &[ExprId]) -> Result<String, EmitError> {
        let mut pieces: Vec<String> = Vec::new();
        let mut rest = args.iter();
        let first = rest.next().expect("non-empty");
        let fmt = match &self.module.exprs[*first].kind {
            ExprKind::String(s) => Some(s.clone()),
            _ => None,
        };
        let Some(fmt) = fmt else {
            for a in args {
                let s = self.arg_string(*a, 'd')?;
                pieces.push(s);
            }
            return Ok(pieces.join(" & \" \" & "));
        };
        let mut literal = String::new();
        let mut chars = fmt.trim_end_matches('\n').chars().peekable();
        let flush = |literal: &mut String, pieces: &mut Vec<String>| {
            if !literal.is_empty() {
                pieces.push(vhdl_string(literal));
                literal.clear();
            }
        };
        while let Some(c) = chars.next() {
            if c == '\n' {
                flush(&mut literal, &mut pieces);
                pieces.push("LF".to_owned());
                continue;
            }
            if c != '%' {
                literal.push(c);
                continue;
            }
            // Skip flags and width.
            while chars
                .peek()
                .is_some_and(|c| c.is_ascii_digit() || *c == '-' || *c == '.')
            {
                chars.next();
            }
            let Some(spec) = chars.next() else {
                literal.push('%');
                break;
            };
            match spec.to_ascii_lowercase() {
                '%' => literal.push('%'),
                'm' => {}
                's' | 'd' | 'h' | 'x' | 'b' | 'o' | 'c' | 't' | 'f' | 'e' | 'g' | 'u' => {
                    flush(&mut literal, &mut pieces);
                    match rest.next() {
                        Some(a) => {
                            let s = self.arg_string(*a, spec.to_ascii_lowercase())?;
                            pieces.push(s);
                        }
                        None => pieces.push("\"\"".to_owned()),
                    }
                }
                other => {
                    literal.push('%');
                    literal.push(other);
                }
            }
        }
        flush(&mut literal, &mut pieces);
        for a in rest {
            let s = self.arg_string(*a, 'd')?;
            pieces.push(s);
        }
        if pieces.is_empty() {
            pieces.push("\"\"".to_owned());
        }
        Ok(pieces.join(" & "))
    }

    fn arg_string(&mut self, arg: ExprId, spec: char) -> Result<String, EmitError> {
        let (text, k) = self.expr(arg)?;
        Ok(match k {
            K::Str => text,
            K::Sl => format!("to_string({text})"),
            K::Int => format!("integer'image({text})"),
            K::Real => format!("real'image({text})"),
            K::U | K::S => match spec {
                'h' | 'x' => format!("to_hstring({text})"),
                'b' => format!("to_string({text})"),
                'o' => format!("to_ostring({text})"),
                _ => format!("integer'image(to_integer({text}))"),
            },
            K::Arr => format!("to_string({text})"),
        })
    }

    // --- cells --------------------------------------------------------------

    fn cell_in(&mut self, cell: &Cell, port: &str) -> Result<V, EmitError> {
        match cell.input(port) {
            Some(id) => self.expr(id),
            None => Err(self.err(
                cell.span,
                format!("cell `{}` has no input `{port}`", cell.name),
            )),
        }
    }

    fn cell_out(&self, cell: &Cell, port: &str) -> Result<(String, K, NetId), EmitError> {
        match cell.output(port) {
            Some(net) => Ok((self.net_names[net.index()].clone(), self.net_kind(net), net)),
            None => Err(self.err(
                cell.span,
                format!("cell `{}` has no output `{port}`", cell.name),
            )),
        }
    }

    /// A signal name for a cell input that must be a signal (a clock): the
    /// net itself, or a helper signal driven by the expression.
    fn signal_input(&mut self, cell: &Cell, port: &str) -> Result<String, EmitError> {
        let id = cell.input(port).ok_or_else(|| {
            self.err(
                cell.span,
                format!("cell `{}` has no input `{port}`", cell.name),
            )
        })?;
        if let Some(net) = self.module.exprs[id].as_net() {
            return Ok(self.net_names[net.index()].clone());
        }
        let (text, k) = self.expr(id)?;
        let name = self.namer.fresh(&format!(
            "reticle_{}_{port}",
            cell.name.as_str().trim_matches('\\')
        ));
        let ty = self.kind_type_text(k, &text);
        self.decls.push(format!("signal {name} : {ty};"));
        self.out.line(&format!("{name} <= {text};"));
        Ok(name)
    }

    fn cell(&mut self, cell: &Cell) -> Result<(), EmitError> {
        let label = self.namer.name(cell.name.as_str());
        self.attributes(&cell.attrs, &label, "label", false);
        let binary = |kind: &CellKind| -> Option<BinaryOp> {
            Some(match kind {
                CellKind::And => BinaryOp::And,
                CellKind::Or => BinaryOp::Or,
                CellKind::Xor => BinaryOp::Xor,
                CellKind::Add => BinaryOp::Add,
                CellKind::Sub => BinaryOp::Sub,
                CellKind::Mul => BinaryOp::Mul,
                CellKind::Div => BinaryOp::Div,
                CellKind::Mod => BinaryOp::Mod,
                CellKind::Shl => BinaryOp::Shl,
                CellKind::Shr => BinaryOp::Shr,
                CellKind::Sshr => BinaryOp::Sshr,
                CellKind::Eq => BinaryOp::Eq,
                CellKind::Ne => BinaryOp::Ne,
                CellKind::Lt => BinaryOp::Lt,
                CellKind::Le => BinaryOp::Le,
                CellKind::Gt => BinaryOp::Gt,
                CellKind::Ge => BinaryOp::Ge,
                _ => return None,
            })
        };
        if let Some(op) = binary(&cell.kind) {
            let a = self.cell_in(cell, "a")?;
            let b = self.cell_in(cell, "b")?;
            let (y, yk, _) = self.cell_out(cell, "y")?;
            let width = self.module.nets[cell.output("y").expect("checked")]
                .ty
                .width()
                .unwrap_or(1);
            let v = self.binary(op, a, b, yk, width);
            let text = self.conv(v, yk);
            self.out.line(&format!("{y} <= {text};"));
            return Ok(());
        }
        match &cell.kind {
            CellKind::Not | CellKind::ReduceAnd | CellKind::ReduceOr | CellKind::ReduceXor => {
                let op = match cell.kind {
                    CellKind::Not => UnaryOp::Not,
                    CellKind::ReduceAnd => UnaryOp::ReduceAnd,
                    CellKind::ReduceOr => UnaryOp::ReduceOr,
                    _ => UnaryOp::ReduceXor,
                };
                let a = self.cell_in(cell, "a")?;
                let (y, yk, _) = self.cell_out(cell, "y")?;
                let v = self.unary(op, a);
                let text = self.conv(v, yk);
                self.out.line(&format!("{y} <= {text};"));
            }
            CellKind::Buf => {
                let a = self.cell_in(cell, "a")?;
                let (y, yk, _) = self.cell_out(cell, "y")?;
                let text = self.conv(a, yk);
                self.out.line(&format!("{y} <= {text};"));
            }
            CellKind::Mux => {
                let a = self.cell_in(cell, "a")?;
                let b = self.cell_in(cell, "b")?;
                let s = self.cell_in(cell, "s")?;
                let (y, yk, _) = self.cell_out(cell, "y")?;
                let s = self.conv(s, K::Sl);
                let a = self.conv(a, yk);
                let b = self.conv(b, yk);
                self.out
                    .line(&format!("{y} <= {b} when {} = '1' else {a};", paren(&s)));
            }
            CellKind::Pmux => {
                let a = self.cell_in(cell, "a")?;
                let b = self.cell_in(cell, "b")?;
                let s = self.cell_in(cell, "s")?;
                let (y, yk, _) = self.cell_out(cell, "y")?;
                let a = self.conv(a, K::U);
                let b = self.conv(b, K::U);
                let s = self.conv(s, K::U);
                let v = self.pkg(format!("pmux({a}, {b}, {s})"));
                let text = self.conv((v, K::U), yk);
                self.out.line(&format!("{y} <= {text};"));
            }
            CellKind::Dff {
                clk_pos,
                has_enable,
                reset,
            } => {
                let clk = self.signal_input(cell, "clk")?;
                let d = self.cell_in(cell, "d")?;
                let (q, qk, _) = self.cell_out(cell, "q")?;
                let d = self.conv(d, qk);
                let edge = if *clk_pos {
                    format!("rising_edge({clk})")
                } else {
                    format!("falling_edge({clk})")
                };
                let mut sens = vec![clk];
                let reset_line = match reset {
                    Some(r) => {
                        let rst = self.cell_in(cell, "rst")?;
                        let rst = self.conv(rst, K::Sl);
                        if r.asynchronous
                            && let Some(net) = cell
                                .input("rst")
                                .and_then(|id| self.module.exprs[id].as_net())
                        {
                            sens.push(self.net_names[net.index()].clone());
                        }
                        let value = self.conv(const_value(&r.value, None), qk);
                        Some((
                            r.asynchronous,
                            format!(
                                "{} = '{}'",
                                paren(&rst),
                                if r.active_high { '1' } else { '0' }
                            ),
                            format!("{q} <= {value};"),
                        ))
                    }
                    None => None,
                };
                let load = if *has_enable {
                    let en = self.cell_in(cell, "en")?;
                    let en = self.conv(en, K::Sl);
                    format!("if {} = '1' then {q} <= {d}; end if;", paren(&en))
                } else {
                    format!("{q} <= {d};")
                };
                self.out
                    .line(&format!("{label} : process ({})", sens.join(", ")));
                self.out.line("begin");
                self.out.indent();
                match reset_line {
                    Some((true, cond, action)) => {
                        self.out.line(&format!("if {cond} then"));
                        self.out.indent();
                        self.out.line(&action);
                        self.out.dedent();
                        self.out.line(&format!("elsif {edge} then"));
                        self.out.indent();
                        self.out.line(&load);
                        self.out.dedent();
                        self.out.line("end if;");
                    }
                    Some((false, cond, action)) => {
                        self.out.line(&format!("if {edge} then"));
                        self.out.indent();
                        self.out.line(&format!("if {cond} then"));
                        self.out.indent();
                        self.out.line(&action);
                        self.out.dedent();
                        self.out.line("else");
                        self.out.indent();
                        self.out.line(&load);
                        self.out.dedent();
                        self.out.line("end if;");
                        self.out.dedent();
                        self.out.line("end if;");
                    }
                    None => {
                        self.out.line(&format!("if {edge} then"));
                        self.out.indent();
                        self.out.line(&load);
                        self.out.dedent();
                        self.out.line("end if;");
                    }
                }
                self.out.dedent();
                self.out.line("end process;");
            }
            CellKind::Dlatch => {
                let en = self.cell_in(cell, "en")?;
                let d = self.cell_in(cell, "d")?;
                let (q, qk, _) = self.cell_out(cell, "q")?;
                let en = self.conv(en, K::Sl);
                let d = self.conv(d, qk);
                self.out.line(&format!("{label} : process (all)"));
                self.out.line("begin");
                self.out.indent();
                self.out
                    .line(&format!("if {} = '1' then {q} <= {d}; end if;", paren(&en)));
                self.out.dedent();
                self.out.line("end process;");
            }
            CellKind::MemRdPort { mem, clocked } => {
                let name = self.mem_names[mem.index()].clone();
                let addr = cell.input("addr").expect("checked");
                let idx = self.index_text(addr)?;
                let (data, dk, _) = self.cell_out(cell, "data")?;
                let elem_k = kind_of(&self.module.memories[*mem].elem);
                let value = self.conv((format!("{name}({idx})"), elem_k), dk);
                if *clocked {
                    let clk = self.signal_input(cell, "clk")?;
                    let en = self.cell_in(cell, "en")?;
                    let en = self.conv(en, K::Sl);
                    self.out.line(&format!("{label} : process ({clk})"));
                    self.out.line("begin");
                    self.out.indent();
                    self.out.line(&format!(
                        "if rising_edge({clk}) and {} = '1' then {data} <= {value}; end if;",
                        paren(&en)
                    ));
                    self.out.dedent();
                    self.out.line("end process;");
                } else {
                    self.out.line(&format!("{data} <= {value};"));
                }
            }
            CellKind::MemWrPort { mem, clocked } => {
                let name = self.mem_names[mem.index()].clone();
                let addr = cell.input("addr").expect("checked");
                let idx = self.index_text(addr)?;
                let data = self.cell_in(cell, "data")?;
                let elem_k = kind_of(&self.module.memories[*mem].elem);
                let data = self.conv(data, elem_k);
                let en = self.cell_in(cell, "en")?;
                let en = self.conv(en, K::Sl);
                let write = format!("{name}({idx}) <= {data};");
                if *clocked {
                    let clk = self.signal_input(cell, "clk")?;
                    self.out.line(&format!("{label} : process ({clk})"));
                    self.out.line("begin");
                    self.out.indent();
                    self.out.line(&format!(
                        "if rising_edge({clk}) and {} = '1' then {write} end if;",
                        paren(&en)
                    ));
                } else {
                    self.out.line(&format!("{label} : process (all)"));
                    self.out.line("begin");
                    self.out.indent();
                    self.out
                        .line(&format!("if {} = '1' then {write} end if;", paren(&en)));
                }
                self.out.dedent();
                self.out.line("end process;");
            }
            CellKind::Lut { init, .. } => {
                let a = cell.input("a").expect("checked");
                let idx = self.index_text(a)?;
                let (y, _, _) = self.cell_out(cell, "y")?;
                let constant = self
                    .namer
                    .fresh(&format!("{}_init", cell.name.as_str().trim_matches('\\')));
                self.decls.push(format!(
                    "constant {constant} : unsigned({} downto 0) := {};",
                    init.width().saturating_sub(1),
                    bits_literal(init, None)
                ));
                self.out.line(&format!("{y} <= {constant}({idx});"));
            }
            CellKind::Tristate => {
                let a = self.cell_in(cell, "a")?;
                let en = self.cell_in(cell, "en")?;
                let (y, yk, _) = self.cell_out(cell, "y")?;
                let a = self.conv(a, yk);
                let en = self.conv(en, K::Sl);
                let z = if yk == K::Sl {
                    "'Z'"
                } else {
                    "(others => 'Z')"
                };
                self.out
                    .line(&format!("{y} <= {a} when {} = '1' else {z};", paren(&en)));
            }
            CellKind::Blackbox(target) => {
                let comp_name = vhdl_ident(target.as_str());
                let mut comp = self
                    .components
                    .remove(&comp_name)
                    .unwrap_or_else(|| Component {
                        name: comp_name.clone(),
                        generics: Vec::new(),
                        ports: Vec::new(),
                    });
                for (name, value) in cell.params.iter() {
                    let (ty, value) = self.generic_of(value);
                    comp.generic(vhdl_ident(name.as_str()), ty, value);
                }
                let mut assocs = Vec::new();
                for (port, e) in &cell.inputs {
                    let ty = self.module.exprs[*e].ty.clone();
                    let ty_text = self.type_text(&ty)?;
                    let pname = vhdl_ident(port.as_str());
                    comp.port(pname.clone(), "in", ty_text);
                    let (text, _) = self.expr(*e)?;
                    assocs.push(format!("{pname} => {text}"));
                }
                for (port, net) in &cell.outputs {
                    let ty = self.module.nets[*net].ty.clone();
                    let ty_text = self.type_text(&ty)?;
                    let pname = vhdl_ident(port.as_str());
                    comp.port(pname.clone(), "out", ty_text);
                    assocs.push(format!("{pname} => {}", self.net_names[net.index()]));
                }
                self.components.insert(comp_name.clone(), comp);
                let mut head = comp_name;
                if !cell.params.is_empty() {
                    let items: Vec<String> = cell
                        .params
                        .iter()
                        .map(|(k, v)| {
                            format!("{} => {}", vhdl_ident(k.as_str()), self.generic_of(v).1)
                        })
                        .collect();
                    let _ = write!(head, " generic map ({})", items.join(", "));
                }
                if assocs.is_empty() {
                    self.out.line(&format!("{label} : {head};"));
                } else {
                    self.out.line(&format!("{label} : {head} port map ("));
                    self.out.indent();
                    let n = assocs.len();
                    for (i, a) in assocs.iter().enumerate() {
                        let sep = if i + 1 < n { "," } else { "" };
                        self.out.line(&format!("{a}{sep}"));
                    }
                    self.out.dedent();
                    self.out.line(");");
                }
            }
            _ => unreachable!("binary cells handled above"),
        }
        Ok(())
    }

    // --- expressions --------------------------------------------------------

    /// Converts a value to another representation.
    fn conv(&mut self, v: V, to: K) -> String {
        let (text, from) = v;
        if from == to {
            return text;
        }
        match (from, to) {
            (K::Sl, K::U) => format!("unsigned'(0 => {text})"),
            (K::Sl, K::S) => format!("signed'(0 => {text})"),
            (K::S, K::U) => format!("unsigned({})", paren(&text)),
            (K::U, K::S) => format!("signed({})", paren(&text)),
            (K::U, K::Sl) => self.pkg(format!("to_sl({text})")),
            (K::S, K::Sl) => self.pkg(format!("to_sl(unsigned({}))", paren(&text))),
            (K::Int, K::U) => format!("to_unsigned({text}, 32)"),
            (K::Int, K::S) => format!("to_signed({text}, 32)"),
            (K::Int, K::Sl) => self.pkg(format!("to_sl(to_unsigned({text}, 1))")),
            (K::U | K::S, K::Int) => format!("to_integer({text})"),
            (K::Sl, K::Int) => format!("to_integer(unsigned'(0 => {text}))"),
            _ => text,
        }
    }

    /// An integer-valued text for an index or count expression.
    fn index_text(&mut self, id: ExprId) -> Result<String, EmitError> {
        let v = self.expr(id)?;
        Ok(self.conv(v, K::Int))
    }

    /// `name(...)`-style access to a bit-vector expression that is a net,
    /// a slice of one, an array element or a memory element: the text
    /// and the offset of bit 0 inside the named object, with the object's
    /// representation.
    fn select_base(&mut self, id: ExprId) -> Result<Option<(String, u32, K)>, EmitError> {
        let expr = &self.module.exprs[id];
        Ok(match &expr.kind {
            ExprKind::Net(net) if self.module.nets[*net].ty.is_bits() => {
                let k = self.net_kind(*net);
                if k == K::Sl {
                    None
                } else {
                    Some((self.net_name(*net), 0, k))
                }
            }
            ExprKind::Slice { base, lo, .. } if self.module.exprs[*base].ty.is_bits() => {
                let (base, lo) = (*base, *lo);
                self.select_base(base)?
                    .map(|(name, off, k)| (name, off + lo, k))
            }
            ExprKind::Index { base, index } => {
                let (base, index) = (*base, *index);
                match &self.module.exprs[base].ty {
                    Type::Array { elem, .. } if elem.is_bits() && !elem.is_bit() => {
                        let k = kind_of(elem);
                        let name = self.array_name(base)?;
                        let idx = self.index_text(index)?;
                        Some((format!("{name}({idx})"), 0, k))
                    }
                    _ => None,
                }
            }
            ExprKind::MemRead { mem, addr } => {
                let (mem, addr) = (*mem, *addr);
                let elem = &self.module.memories[mem].elem;
                if elem.is_bits() && !elem.is_bit() {
                    let k = kind_of(elem);
                    let name = self.mem_names[mem.index()].clone();
                    let idx = self.index_text(addr)?;
                    Some((format!("{name}({idx})"), 0, k))
                } else {
                    None
                }
            }
            _ => None,
        })
    }

    fn array_name(&mut self, id: ExprId) -> Result<String, EmitError> {
        let expr = &self.module.exprs[id];
        match &expr.kind {
            ExprKind::Net(net) => Ok(self.net_name(*net)),
            ExprKind::Index { base, index } => {
                let (base, index) = (*base, *index);
                let name = self.array_name(base)?;
                let idx = self.index_text(index)?;
                Ok(format!("{name}({idx})"))
            }
            _ => Err(self.err(expr.span, "VHDL can only select elements of array objects")),
        }
    }

    fn expr(&mut self, id: ExprId) -> Result<V, EmitError> {
        let expr = &self.module.exprs[id];
        let k = kind_of(&expr.ty);
        let span = expr.span;
        match &expr.kind {
            ExprKind::Const(c) => Ok(const_value(c, None)),
            ExprKind::String(s) => Ok((vhdl_string(s), K::Str)),
            ExprKind::Net(net) => Ok((self.net_name(*net), self.net_kind(*net))),
            ExprKind::Slice { base, hi, lo } => {
                let (base, hi, lo) = (*base, *hi, *lo);
                if !self.module.exprs[base].ty.is_bits() {
                    let name = self.array_name(base)?;
                    return Ok((format!("{name}({lo} to {hi})"), K::Arr));
                }
                if let Some((name, off, bk)) = self.select_base(base)? {
                    if hi == lo {
                        return Ok((format!("{name}({})", off + lo), K::Sl));
                    }
                    let text = format!("{name}({} downto {})", off + hi, off + lo);
                    return Ok((self.conv((text, bk), k), k));
                }
                let inner = self.expr(base)?;
                let inner = self.conv(inner, K::U);
                if hi == lo {
                    Ok((self.pkg(format!("idx({inner}, {lo})")), K::Sl))
                } else {
                    Ok((self.pkg(format!("slc({inner}, {hi}, {lo})")), K::U))
                }
            }
            ExprKind::Index { base, index } => {
                let (base, index) = (*base, *index);
                if !self.module.exprs[base].ty.is_bits() {
                    let name = self.array_name(base)?;
                    let idx = self.index_text(index)?;
                    return Ok((format!("{name}({idx})"), k));
                }
                let idx = self.index_text(index)?;
                if let Some((name, off, _)) = self.select_base(base)? {
                    return Ok((
                        if off == 0 {
                            format!("{name}({idx})")
                        } else {
                            format!("{name}({off} + {idx})")
                        },
                        K::Sl,
                    ));
                }
                let inner = self.expr(base)?;
                match inner.1 {
                    K::Sl => Ok((
                        self.pkg(format!("idx(unsigned'(0 => {}), {idx})", inner.0)),
                        K::Sl,
                    )),
                    _ => {
                        let inner = self.conv(inner, K::U);
                        Ok((self.pkg(format!("idx({inner}, {idx})")), K::Sl))
                    }
                }
            }
            ExprKind::IndexedSlice {
                base,
                offset,
                width,
                up,
            } => {
                let (base, offset, width, up) = (*base, *offset, *width, *up);
                let off = self.index_text(offset)?;
                let (hi, lo) = if up {
                    (format!("{off} + {}", width.saturating_sub(1)), off.clone())
                } else {
                    (off.clone(), format!("{off} - {}", width.saturating_sub(1)))
                };
                if let Some((name, base_off, bk)) = self.select_base(base)? {
                    let (hi, lo) = if base_off == 0 {
                        (hi, lo)
                    } else {
                        (format!("{base_off} + {hi}"), format!("{base_off} + {lo}"))
                    };
                    if width == 1 {
                        return Ok((format!("{name}({lo})"), K::Sl));
                    }
                    let text = format!("{name}({hi} downto {lo})");
                    return Ok((self.conv((text, bk), k), k));
                }
                let inner = self.expr(base)?;
                let inner = self.conv(inner, K::U);
                if width == 1 {
                    Ok((self.pkg(format!("idx({inner}, {lo})")), K::Sl))
                } else {
                    Ok((self.pkg(format!("slc({inner}, {hi}, {lo})")), K::U))
                }
            }
            ExprKind::Concat(parts) => {
                let parts = parts.clone();
                let mut items = Vec::new();
                for p in parts {
                    let (text, pk) = self.expr(p)?;
                    items.push(match pk {
                        K::Sl | K::U => text,
                        _ => self.conv((text, pk), K::U),
                    });
                }
                if items.len() == 1 && k == K::Sl {
                    return Ok((items.remove(0), K::Sl));
                }
                let text = format!("unsigned'({})", items.join(" & "));
                Ok((self.conv((text, K::U), k), k))
            }
            ExprKind::Replicate { count, expr } => {
                let (count, expr) = (*count, *expr);
                let (text, ik) = self.expr(expr)?;
                if count == 1 {
                    return Ok((self.conv((text, ik), k), k));
                }
                let text = match ik {
                    K::Sl => format!("unsigned'(0 to {} => {text})", count.saturating_sub(1)),
                    _ => {
                        let inner = self.conv((text, ik), K::U);
                        self.pkg(format!("rep({inner}, {count})"))
                    }
                };
                Ok((self.conv((text, K::U), k), k))
            }
            ExprKind::Unary { op, expr } => {
                let (op, expr) = (*op, *expr);
                let v = self.expr(expr)?;
                let v = self.unary(op, v);
                Ok((self.conv(v, k), k))
            }
            ExprKind::Binary { op, lhs, rhs } => {
                let (op, lhs, rhs) = (*op, *lhs, *rhs);
                let width = expr.ty.width().unwrap_or(1);
                let l = self.expr(lhs)?;
                let r = match (op, self.module.exprs[rhs].as_const()) {
                    // `x` and `z` in a constant pattern are don't-cares.
                    (BinaryOp::WildEq, Some(c)) => const_value(c, Some(CaseKind::X)),
                    _ => self.expr(rhs)?,
                };
                let v = self.binary(op, l, r, k, width);
                Ok((self.conv(v, k), k))
            }
            ExprKind::Ternary { cond, then_, else_ } => {
                let (cond, then_, else_) = (*cond, *then_, *else_);
                let c = self.expr(cond)?;
                let c = self.conv(c, K::Sl);
                let t = self.expr(then_)?;
                let e = self.expr(else_)?;
                match k {
                    K::Str | K::Arr => Err(self.err(
                        span,
                        "VHDL has no conditional expression for strings or arrays",
                    )),
                    _ => {
                        let t = self.conv(t, k);
                        let e = self.conv(e, k);
                        Ok((self.pkg(format!("mux({c}, {t}, {e})")), k))
                    }
                }
            }
            ExprKind::Resize {
                expr,
                width,
                signed,
            } => {
                let (inner, width, signed) = (*expr, *width, *signed);
                self.resize(inner, width, signed, k)
            }
            ExprKind::MemRead { mem, addr } => {
                let (mem, addr) = (*mem, *addr);
                let name = self.mem_names[mem.index()].clone();
                let idx = self.index_text(addr)?;
                Ok((format!("{name}({idx})"), k))
            }
            ExprKind::Call { name, args } => {
                let args = args.clone();
                let mut items = Vec::new();
                for a in args {
                    items.push(self.expr(a)?.0);
                }
                Ok((
                    format!("{}({})", vhdl_ident(name.as_str()), items.join(", ")),
                    k,
                ))
            }
        }
    }

    /// Renders a unary operator; the result keeps the operand's
    /// representation for `Not` and `Neg` and is `Sl` for reductions.
    fn unary(&mut self, op: UnaryOp, v: V) -> V {
        let (text, k) = v;
        match (op, k) {
            (UnaryOp::Not, _) => (format!("(not {})", paren(&text)), k),
            (UnaryOp::Neg, K::S | K::Int | K::Real) => (format!("(-{})", paren(&text)), k),
            (UnaryOp::Neg, K::U) => (format!("unsigned(-signed({text}))"), k),
            (UnaryOp::Neg, _) => (text, k),
            (UnaryOp::ReduceAnd | UnaryOp::ReduceOr | UnaryOp::ReduceXor, K::Sl) => (text, K::Sl),
            (
                UnaryOp::ReduceNand | UnaryOp::ReduceNor | UnaryOp::ReduceXnor | UnaryOp::LogicNot,
                K::Sl,
            ) => (format!("(not {})", paren(&text)), K::Sl),
            (op, _) => {
                let inner = self.conv((text, k), K::U);
                let word = match op {
                    UnaryOp::ReduceAnd => "and",
                    UnaryOp::ReduceOr => "or",
                    UnaryOp::ReduceXor => "xor",
                    UnaryOp::ReduceNand => "nand",
                    UnaryOp::ReduceNor | UnaryOp::LogicNot => "nor",
                    UnaryOp::ReduceXnor => "xnor",
                    UnaryOp::Not | UnaryOp::Neg => unreachable!(),
                };
                (format!("({word} {inner})"), K::Sl)
            }
        }
    }

    /// Renders a binary operator; the result is in representation `out`
    /// for arithmetic and bitwise operators, `Sl` for predicates.
    fn binary(&mut self, op: BinaryOp, l: V, r: V, out: K, width: u32) -> V {
        use BinaryOp as B;
        let lk = l.1;
        let rk = r.1;
        // Single-bit operands of vector operators are promoted.
        let vec_kind = match (lk, rk) {
            (K::S, K::S) => K::S,
            _ => K::U,
        };
        match op {
            B::And | B::Or | B::Xor | B::Xnor | B::LogicAnd | B::LogicOr => {
                let word = match op {
                    B::And | B::LogicAnd => "and",
                    B::Or | B::LogicOr => "or",
                    B::Xor => "xor",
                    _ => "xnor",
                };
                let k = if lk == K::Sl && rk == K::Sl {
                    K::Sl
                } else {
                    out
                };
                let a = self.conv(l, k);
                let b = self.conv(r, k);
                (format!("({} {word} {})", paren(&a), paren(&b)), k)
            }
            B::Add | B::Sub if lk == K::Sl && rk == K::Sl => {
                (format!("({} xor {})", paren(&l.0), paren(&r.0)), K::Sl)
            }
            B::Add | B::Sub | B::Div | B::Mod if matches!(lk, K::Int | K::Real) => {
                let sym = match op {
                    B::Add => "+",
                    B::Sub => "-",
                    B::Div => "/",
                    _ => "rem",
                };
                (format!("({} {sym} {})", paren(&l.0), paren(&r.0)), lk)
            }
            B::Add | B::Sub | B::Div | B::Mod => {
                let k = vec_kind;
                let a = self.conv(l, k);
                let b = self.conv(r, k);
                let sym = match op {
                    B::Add => "+",
                    B::Sub => "-",
                    B::Div => "/",
                    _ => "rem",
                };
                (format!("({} {sym} {})", paren(&a), paren(&b)), k)
            }
            B::Mul if matches!(lk, K::Int | K::Real) => {
                (format!("({} * {})", paren(&l.0), paren(&r.0)), lk)
            }
            B::Mul => {
                let a = self.conv(l, K::U);
                let b = self.conv(r, K::U);
                (
                    format!("resize({} * {}, {width})", paren(&a), paren(&b)),
                    K::U,
                )
            }
            B::Pow if matches!(lk, K::Int | K::Real) => {
                (format!("({} ** {})", paren(&l.0), paren(&r.0)), lk)
            }
            B::Pow => {
                let k = vec_kind;
                let a = self.conv(l, k);
                let b = self.conv(r, k);
                (self.pkg(format!("pow({a}, {b})")), k)
            }
            B::Shl | B::Shr | B::Sshr => {
                let n = self.conv(r, K::Int);
                let k = match lk {
                    K::S => K::S,
                    _ => K::U,
                };
                let a = self.conv(l, k);
                let text = match (op, k) {
                    (B::Shl, _) => format!("shift_left({a}, {n})"),
                    (B::Shr, K::U) | (B::Sshr, K::S) => format!("shift_right({a}, {n})"),
                    (B::Shr, _) => format!("signed(shift_right(unsigned({a}), {n}))"),
                    (_, _) => format!("unsigned(shift_right(signed({a}), {n}))"),
                };
                (text, k)
            }
            B::Eq | B::Ne | B::Lt | B::Le | B::Gt | B::Ge => {
                let sym = match op {
                    B::Eq => "?=",
                    B::Ne => "?/=",
                    B::Lt => "?<",
                    B::Le => "?<=",
                    B::Gt => "?>",
                    _ => "?>=",
                };
                match (lk, rk) {
                    (K::Int | K::Real | K::Str | K::Arr, _) => {
                        let plain = &sym[1..];
                        (
                            self.pkg(format!("to_sl({} {plain} {})", paren(&l.0), paren(&r.0))),
                            K::Sl,
                        )
                    }
                    (K::Sl, K::Sl) if matches!(op, B::Eq | B::Ne) => {
                        (format!("({} {sym} {})", paren(&l.0), paren(&r.0)), K::Sl)
                    }
                    _ => {
                        let k = vec_kind;
                        let a = self.conv(l, k);
                        let b = self.conv(r, k);
                        (format!("({} {sym} {})", paren(&a), paren(&b)), K::Sl)
                    }
                }
            }
            B::CaseEq | B::CaseNe => {
                let sym = if op == B::CaseEq { "=" } else { "/=" };
                let scalar = (lk == K::Sl && rk == K::Sl)
                    || matches!(lk, K::Int | K::Real | K::Str | K::Arr);
                let (a, b) = if scalar {
                    (l.0, r.0)
                } else {
                    let k = vec_kind;
                    (self.conv(l, k), self.conv(r, k))
                };
                (
                    self.pkg(format!("to_sl({} {sym} {})", paren(&a), paren(&b))),
                    K::Sl,
                )
            }
            B::WildEq => {
                let (a, b) = if lk == K::Sl && rk == K::Sl {
                    (l.0, r.0)
                } else {
                    (self.conv(l, K::U), self.conv(r, K::U))
                };
                (self.pkg(format!("to_sl(std_match({a}, {b}))")), K::Sl)
            }
        }
    }

    fn resize(&mut self, inner: ExprId, width: u32, signed: bool, out: K) -> Result<V, EmitError> {
        let operand = &self.module.exprs[inner];
        let (w, operand_signed, is_int) = match &operand.ty {
            Type::Bits { width, signed } => (*width, *signed, false),
            Type::Integer => (32, true, true),
            other => {
                return Err(self.err(
                    operand.span,
                    format!("cannot resize a value of type `{other}` in VHDL"),
                ));
            }
        };
        let v = self.expr(inner)?;
        if is_int {
            let text = if signed {
                format!("to_signed({}, {width})", v.0)
            } else {
                format!("unsigned(to_signed({}, {width}))", v.0)
            };
            let k = if signed { K::S } else { K::U };
            return Ok((self.conv((text, k), out), out));
        }
        let sext = signed && operand_signed;
        if width == w {
            return Ok((self.conv(v, out), out));
        }
        // Widening or narrowing goes through `resize`; numeric_std keeps
        // the sign bit when narrowing a signed value, so narrowing always
        // uses unsigned to keep the low bits as the IR requires.
        let text = if sext && width > w {
            let s = self.conv(v, K::S);
            (format!("resize({s}, {width})"), K::S)
        } else {
            let u = self.conv(v, K::U);
            (format!("resize({u}, {width})"), K::U)
        };
        Ok((self.conv(text, out), out))
    }
}

/// True when `block` contains a statement matching `pred` outside any
/// nested loop.
fn contains_direct(block: &Block, pred: &dyn Fn(&StmtKind) -> bool) -> bool {
    block.iter().any(|stmt| {
        if pred(&stmt.kind) {
            return true;
        }
        match &stmt.kind {
            StmtKind::For { .. }
            | StmtKind::While { .. }
            | StmtKind::Repeat { .. }
            | StmtKind::Forever { .. } => false,
            _ => stmt.blocks().iter().any(|b| contains_direct(b, pred)),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::builder::ModuleBuilder;
    use crate::source::SourceMap;

    fn span() -> Span {
        let mut map = SourceMap::new();
        let id = map.add("t", "").unwrap();
        Span::new(id, 0, 0)
    }

    fn render(build: impl FnOnce(&mut ModuleBuilder, [ExprId; 5]) -> ExprId) -> (String, bool) {
        let span = span();
        let mut b = ModuleBuilder::new("t", span);
        let a = b.input("a", Type::bits(8));
        let bb = b.input("b", Type::bits(8));
        let c = b.input("c", Type::bit());
        let sa = b.input("sa", Type::sbits(8));
        let sb = b.input("sb", Type::sbits(8));
        let nets = [b.net(a), b.net(bb), b.net(c), b.net(sa), b.net(sb)];
        let e = build(&mut b, nets);
        let module = b.finish();
        let mut design = Design::new();
        design.add_module(module);
        let names = ModuleNames::new(&design);
        let mut p = Printer::new(&design, ModuleId(0), &names);
        let text = p.expr(e).unwrap().0;
        (text, p.uses_pkg)
    }

    #[test]
    fn expressions() {
        assert_eq!(render(|b, [a, bb, ..]| b.add(a, bb)).0, "(a + b)");
        assert_eq!(
            render(|b, [_, _, _, sa, sb]| b.mul(sa, sb)).0,
            "signed(resize(unsigned(sa) * unsigned(sb), 8))"
        );
        assert_eq!(
            render(|b, [a, _, _, sa, _]| b.add(a, sa)).0,
            "(a + unsigned(sa))"
        );
        assert_eq!(render(|b, [_, _, _, sa, sb]| b.lt(sa, sb)).0, "(sa ?< sb)");
        assert_eq!(render(|b, [a, ..]| b.slice(a, 3, 3)).0, "a(3)");
        assert_eq!(
            render(|b, [_, _, _, sa, _]| b.slice(sa, 6, 2)).0,
            "unsigned(sa(6 downto 2))"
        );
        assert_eq!(
            render(|b, [a, bb, ..]| {
                let s = b.add(a, bb);
                b.slice(s, 3, 1)
            }),
            ("slc((a + b), 3, 1)".to_owned(), true)
        );
        assert_eq!(
            render(|b, [a, _, c, ..]| b.concat(vec![c, a])).0,
            "unsigned'(c & a)"
        );
        assert_eq!(
            render(|b, [_, _, c, ..]| b.replicate(4, c)).0,
            "unsigned'(0 to 3 => c)"
        );
        assert_eq!(render(|b, [a, ..]| b.reduce_or(a)).0, "(or a)");
        assert_eq!(render(|b, [_, _, c, ..]| b.lnot(c)).0, "(not c)");
        assert_eq!(render(|b, [a, ..]| b.neg(a)).0, "unsigned(-signed(a))");
        assert_eq!(
            render(|b, [a, _, c, ..]| {
                let one = b.const_u64(8, 1);
                b.mux(c, a, one)
            }),
            ("mux(c, a, unsigned'(\"00000001\"))".to_owned(), true)
        );
        assert_eq!(
            render(|b, [a, bb, ..]| b.shr(a, bb)).0,
            "shift_right(a, to_integer(b))"
        );
        assert_eq!(
            render(|b, [_, _, _, sa, sb]| b.shr(sa, sb)).0,
            "signed(shift_right(unsigned(sa), to_integer(sb)))"
        );
        assert_eq!(
            render(|b, [a, bb, ..]| b.binary(BinaryOp::Sshr, a, bb)).0,
            "unsigned(shift_right(signed(a), to_integer(b)))"
        );
        assert_eq!(
            render(|b, [_, _, c, ..]| {
                let one = b.const_bit(true);
                b.add(c, one)
            })
            .0,
            "(c xor '1')"
        );
        assert_eq!(
            render(|b, [a, _, c, ..]| b.index(a, c)).0,
            "a(to_integer(unsigned'(0 => c)))"
        );
    }

    #[test]
    fn resize_forms() {
        assert_eq!(render(|b, [a, ..]| b.zext(a, 12)).0, "resize(a, 12)");
        assert_eq!(
            render(|b, [_, _, _, sa, _]| b.sext(sa, 12)).0,
            "resize(sa, 12)"
        );
        assert_eq!(
            render(|b, [_, _, _, sa, _]| b.sext(sa, 4)).0,
            "signed(resize(unsigned(sa), 4))"
        );
        assert_eq!(render(|b, [a, ..]| b.sext(a, 8)).0, "signed(a)");
        assert_eq!(
            render(|b, [a, ..]| b.zext(a, 1)),
            ("to_sl(resize(a, 1))".to_owned(), true)
        );
        assert_eq!(
            render(|b, [_, _, c, ..]| b.zext(c, 4)).0,
            "resize(unsigned'(0 => c), 4)"
        );
    }

    #[test]
    fn helpers() {
        assert_eq!(
            bits_literal(&Const::parse_verilog("4'b1x0z").unwrap(), None),
            "\"1X0Z\""
        );
        assert_eq!(
            bits_literal(&Const::parse_verilog("4'b1x0z").unwrap(), Some(CaseKind::Z)),
            "\"1X0-\""
        );
        assert_eq!(
            bits_literal(&Const::parse_verilog("4'b1x0z").unwrap(), Some(CaseKind::X)),
            "\"1-0-\""
        );
        assert_eq!(
            const_value(&Const::from_u64(1, 1), None),
            ("'1'".to_owned(), K::Sl)
        );
        assert!(is_atomic("a(3)"));
        assert!(is_atomic("x'length"));
        assert!(!is_atomic("a + b"));
        let namer = Namer::new(["Foo", "foo", "bar", "entity", "resize"].into_iter());
        assert_eq!(namer.name("Foo"), "\\Foo\\");
        assert_eq!(namer.name("foo"), "\\foo\\");
        assert_eq!(namer.name("bar"), "bar");
        assert_eq!(namer.name("entity"), "\\entity\\");
        assert_eq!(namer.name("resize"), "\\resize\\");
        let mut namer = namer;
        assert_eq!(namer.fresh("bar"), "bar1");
        assert_eq!(namer.fresh("bar"), "bar2");
        assert_eq!(namer.fresh("baz"), "baz");
    }
}
