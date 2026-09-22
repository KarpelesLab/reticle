//! Emitters: rendering a [`Design`] as Verilog, VHDL, Yosys JSON, BLIF or
//! EDIF text.
//!
//! Every emitter is a pure function from the IR to a `String`; nothing here
//! touches the filesystem. The HDL emitters render both forms of a module
//! (processes and cells), the netlist formats only the cell form and report
//! an [`EmitError`] pointing at the first construct they cannot express.
//!
//! | Format                    | Process form | Cell form | Hierarchy | Notes                                                     |
//! |---------------------------|--------------|-----------|-----------|-----------------------------------------------------------|
//! | [`verilog`] Verilog-2005  | yes          | yes       | yes       | `always` / `assign`; escaped identifiers; `(* attrs *)`   |
//! | [`vhdl`] VHDL-2008        | yes          | yes       | yes       | entity/architecture; `ieee.numeric_std`; `\extended\` ids |
//! | [`json`] Yosys JSON       | no           | yes       | yes       | Yosys internal cell types; nextpnr / netlistsvg           |
//! | [`blif`] BLIF             | no           | bit-level | yes       | `.names` truth tables and `.latch`; arithmetic rejected   |
//! | [`edif`] EDIF 2 0 0       | no           | yes       | yes       | external primitive library, single-bit ports and nets     |
//!
//! Output is deterministic: objects are rendered in arena order, which the
//! text format also uses, and nothing is iterated from a hash map.
//!
//! # Names
//!
//! Each target has its own identifier rules. The helpers in this module
//! ([`verilog_ident`], [`vhdl_ident`], [`edif_ident`]) escape a name that is
//! a keyword or is not a plain identifier of the target language, so any IR
//! name comes out as legal text; the emitters never rename objects
//! otherwise. [`json_string`] escapes text for JSON.
//!
//! # Netlist formats
//!
//! JSON, BLIF and EDIF see a module as bits: [`BitView`] numbers every bit
//! of every bit-vector net, evaluates *structural* expressions (constants,
//! nets, slices, concatenations, replications and resizes) to lists of bits,
//! and merges the bits that continuous assignments tie together, so
//! `assign y = {a, 2'b01}` becomes aliases and constants rather than logic.
//! Any other expression (an operator, a memory read, a call) is an error:
//! the netlist formats are for synthesised designs.

use std::error::Error;
use std::fmt;
use std::fmt::Write as _;

use super::design::{Design, Module, NetId, PortDir};
use super::expr::{ExprId, ExprKind};
use super::process::Lvalue;
use super::types::Bit;
use crate::source::Span;

pub mod blif;
pub mod edif;
pub mod json;
pub mod verilog;
pub mod vhdl;

pub use blif::{emit_blif, emit_blif_module};
pub use edif::emit_edif;
pub use json::emit_json;
pub use verilog::{VerilogOptions, emit_verilog, emit_verilog_module, emit_verilog_with};
pub use vhdl::{emit_vhdl, emit_vhdl_module};

/// A construct the target format cannot express.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EmitError {
    /// What could not be expressed and why.
    pub message: String,
    /// The offending object.
    pub span: Span,
}

impl EmitError {
    /// Builds an error.
    pub fn new(span: Span, message: impl Into<String>) -> Self {
        EmitError {
            message: message.into(),
            span,
        }
    }
}

impl fmt::Display for EmitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl Error for EmitError {}

/// An output format.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Format {
    /// Behavioural and structural Verilog-2005.
    Verilog,
    /// VHDL-2008.
    Vhdl,
    /// The Yosys JSON netlist format.
    Json,
    /// Berkeley Logic Interchange Format.
    Blif,
    /// EDIF 2 0 0.
    Edif,
}

impl Format {
    /// Every format, in a fixed order.
    pub const ALL: [Format; 5] = [
        Format::Verilog,
        Format::Vhdl,
        Format::Json,
        Format::Blif,
        Format::Edif,
    ];

    /// The conventional file extension, without the dot.
    pub fn extension(self) -> &'static str {
        match self {
            Format::Verilog => "v",
            Format::Vhdl => "vhd",
            Format::Json => "json",
            Format::Blif => "blif",
            Format::Edif => "edif",
        }
    }

    /// The format with the given extension or name (`v`, `verilog`, `sv`,
    /// `vhd`, `vhdl`, `json`, `blif`, `edif`, `edf`), case-insensitively.
    pub fn from_name(name: &str) -> Option<Format> {
        match name.to_ascii_lowercase().as_str() {
            "v" | "sv" | "verilog" => Some(Format::Verilog),
            "vhd" | "vhdl" => Some(Format::Vhdl),
            "json" => Some(Format::Json),
            "blif" => Some(Format::Blif),
            "edif" | "edf" => Some(Format::Edif),
            _ => None,
        }
    }
}

/// Renders `design` in `format` with each emitter's default options.
pub fn emit(design: &Design, format: Format) -> Result<String, EmitError> {
    match format {
        Format::Verilog => emit_verilog(design),
        Format::Vhdl => emit_vhdl(design),
        Format::Json => emit_json(design),
        Format::Blif => emit_blif(design),
        Format::Edif => emit_edif(design),
    }
}

// ---------------------------------------------------------------------------
// Identifiers
// ---------------------------------------------------------------------------

/// The reserved words of IEEE 1364-2005 (Annex B) and the additional
/// reserved words of IEEE 1800-2017, so output is also safe for
/// SystemVerilog tools.
const VERILOG_KEYWORDS: &[&str] = &[
    "accept_on",
    "alias",
    "always",
    "always_comb",
    "always_ff",
    "always_latch",
    "and",
    "assert",
    "assign",
    "assume",
    "automatic",
    "before",
    "begin",
    "bind",
    "bins",
    "binsof",
    "bit",
    "break",
    "buf",
    "bufif0",
    "bufif1",
    "byte",
    "case",
    "casex",
    "casez",
    "cell",
    "chandle",
    "checker",
    "class",
    "clocking",
    "cmos",
    "config",
    "const",
    "constraint",
    "context",
    "continue",
    "cover",
    "covergroup",
    "coverpoint",
    "cross",
    "deassign",
    "default",
    "defparam",
    "design",
    "disable",
    "dist",
    "do",
    "edge",
    "else",
    "end",
    "endcase",
    "endchecker",
    "endclass",
    "endclocking",
    "endconfig",
    "endfunction",
    "endgenerate",
    "endgroup",
    "endinterface",
    "endmodule",
    "endpackage",
    "endprimitive",
    "endprogram",
    "endproperty",
    "endsequence",
    "endspecify",
    "endtable",
    "endtask",
    "enum",
    "event",
    "eventually",
    "expect",
    "export",
    "extends",
    "extern",
    "final",
    "first_match",
    "for",
    "force",
    "foreach",
    "forever",
    "fork",
    "forkjoin",
    "function",
    "generate",
    "genvar",
    "global",
    "highz0",
    "highz1",
    "if",
    "iff",
    "ifnone",
    "ignore_bins",
    "illegal_bins",
    "implements",
    "implies",
    "import",
    "incdir",
    "include",
    "initial",
    "inout",
    "input",
    "inside",
    "instance",
    "int",
    "integer",
    "interconnect",
    "interface",
    "intersect",
    "join",
    "join_any",
    "join_none",
    "large",
    "let",
    "liblist",
    "library",
    "local",
    "localparam",
    "logic",
    "longint",
    "macromodule",
    "matches",
    "medium",
    "modport",
    "module",
    "nand",
    "negedge",
    "nettype",
    "new",
    "nexttime",
    "nmos",
    "nor",
    "noshowcancelled",
    "not",
    "notif0",
    "notif1",
    "null",
    "or",
    "output",
    "package",
    "packed",
    "parameter",
    "pmos",
    "posedge",
    "primitive",
    "priority",
    "program",
    "property",
    "protected",
    "pull0",
    "pull1",
    "pulldown",
    "pullup",
    "pulsestyle_ondetect",
    "pulsestyle_onevent",
    "pure",
    "rand",
    "randc",
    "randcase",
    "randsequence",
    "rcmos",
    "real",
    "realtime",
    "ref",
    "reg",
    "reject_on",
    "release",
    "repeat",
    "restrict",
    "return",
    "rnmos",
    "rpmos",
    "rtran",
    "rtranif0",
    "rtranif1",
    "s_always",
    "s_eventually",
    "s_nexttime",
    "s_until",
    "s_until_with",
    "scalared",
    "sequence",
    "shortint",
    "shortreal",
    "showcancelled",
    "signed",
    "small",
    "soft",
    "solve",
    "specify",
    "specparam",
    "static",
    "string",
    "strong",
    "strong0",
    "strong1",
    "struct",
    "super",
    "supply0",
    "supply1",
    "sync_accept_on",
    "sync_reject_on",
    "table",
    "tagged",
    "task",
    "this",
    "throughout",
    "time",
    "timeprecision",
    "timeunit",
    "tran",
    "tranif0",
    "tranif1",
    "tri",
    "tri0",
    "tri1",
    "triand",
    "trior",
    "trireg",
    "type",
    "typedef",
    "union",
    "unique",
    "unique0",
    "unsigned",
    "until",
    "until_with",
    "untyped",
    "use",
    "uwire",
    "var",
    "vectored",
    "virtual",
    "void",
    "wait",
    "wait_order",
    "wand",
    "weak",
    "weak0",
    "weak1",
    "while",
    "wildcard",
    "wire",
    "with",
    "within",
    "wor",
    "xnor",
    "xor",
];

/// The reserved words of IEEE 1076-2008 (including the PSL words that are
/// reserved in VHDL-2008).
const VHDL_RESERVED: &[&str] = &[
    "abs",
    "access",
    "after",
    "alias",
    "all",
    "and",
    "architecture",
    "array",
    "assert",
    "assume",
    "assume_guarantee",
    "attribute",
    "begin",
    "block",
    "body",
    "buffer",
    "bus",
    "case",
    "component",
    "configuration",
    "constant",
    "context",
    "cover",
    "default",
    "disconnect",
    "downto",
    "else",
    "elsif",
    "end",
    "entity",
    "exit",
    "fairness",
    "file",
    "for",
    "force",
    "function",
    "generate",
    "generic",
    "group",
    "guarded",
    "if",
    "impure",
    "in",
    "inertial",
    "inout",
    "is",
    "label",
    "library",
    "linkage",
    "literal",
    "loop",
    "map",
    "mod",
    "nand",
    "new",
    "next",
    "nor",
    "not",
    "null",
    "of",
    "on",
    "open",
    "or",
    "others",
    "out",
    "package",
    "parameter",
    "port",
    "postponed",
    "procedure",
    "process",
    "property",
    "protected",
    "pure",
    "range",
    "record",
    "register",
    "reject",
    "release",
    "rem",
    "report",
    "restrict",
    "restrict_guarantee",
    "return",
    "rol",
    "ror",
    "select",
    "sequence",
    "severity",
    "shared",
    "signal",
    "sla",
    "sll",
    "sra",
    "srl",
    "strong",
    "subtype",
    "then",
    "to",
    "transport",
    "type",
    "unaffected",
    "units",
    "until",
    "use",
    "variable",
    "vmode",
    "vprop",
    "vunit",
    "wait",
    "when",
    "while",
    "with",
    "xnor",
    "xor",
];

/// True when `s` is a Verilog (or SystemVerilog) keyword.
pub fn is_verilog_keyword(s: &str) -> bool {
    VERILOG_KEYWORDS.binary_search(&s).is_ok()
}

/// True when `s` is a VHDL-2008 reserved word, compared case-insensitively.
pub fn is_vhdl_reserved(s: &str) -> bool {
    let lower = s.to_ascii_lowercase();
    VHDL_RESERVED.binary_search(&lower.as_str()).is_ok()
}

/// True when `s` is a simple Verilog identifier:
/// `[A-Za-z_][A-Za-z0-9_$]*` and not a keyword.
pub fn is_simple_verilog_ident(s: &str) -> bool {
    let mut chars = s.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
        && !is_verilog_keyword(s)
}

/// Renders a name as a Verilog identifier, escaping it (`\name `, with the
/// trailing space that closes an escaped identifier) when it is a keyword
/// or is not a simple identifier. Characters an escaped identifier cannot
/// hold (whitespace, control characters, non-ASCII) become `_`.
pub fn verilog_ident(name: &str) -> String {
    if is_simple_verilog_ident(name) {
        return name.to_owned();
    }
    let mut out = String::with_capacity(name.len() + 2);
    out.push('\\');
    for c in name.chars() {
        if c.is_ascii_graphic() {
            out.push(c);
        } else {
            out.push('_');
        }
    }
    if out.len() == 1 {
        out.push('_');
    }
    out.push(' ');
    out
}

/// True when `s` is a basic VHDL identifier: a letter followed by letters,
/// digits and single underscores, not ending in an underscore, and not a
/// reserved word.
pub fn is_basic_vhdl_ident(s: &str) -> bool {
    let bytes = s.as_bytes();
    let Some(&first) = bytes.first() else {
        return false;
    };
    if !first.is_ascii_alphabetic() || bytes.last() == Some(&b'_') {
        return false;
    }
    let mut prev_underscore = false;
    for &b in bytes {
        let ok = b.is_ascii_alphanumeric() || b == b'_';
        if !ok || (b == b'_' && prev_underscore) {
            return false;
        }
        prev_underscore = b == b'_';
    }
    !is_vhdl_reserved(s)
}

/// Renders a name as a VHDL identifier, using an extended identifier
/// (`\name\`, with backslashes doubled) when it is not a basic identifier.
/// Characters an extended identifier cannot hold become `_`.
pub fn vhdl_ident(name: &str) -> String {
    if is_basic_vhdl_ident(name) {
        return name.to_owned();
    }
    vhdl_extended(name)
}

/// Renders a name as a VHDL extended identifier (`\name\`) whether or not
/// it is a basic identifier, for names that must stay distinct from a
/// basic identifier of the same spelling.
pub fn vhdl_extended(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 2);
    out.push('\\');
    for c in name.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            c if c.is_ascii_graphic() || c == ' ' => out.push(c),
            _ => out.push('_'),
        }
    }
    if out.len() == 1 {
        out.push('_');
    }
    out.push('\\');
    out
}

/// Renders a name as an EDIF identifier: letters, digits and underscores,
/// starting with a letter (or `&`), at most 255 characters. Returns the
/// identifier and whether it differs from `name`, in which case the caller
/// wraps it in `(rename ident "name")`.
pub fn edif_ident(name: &str) -> (String, bool) {
    let mut out = String::with_capacity(name.len() + 1);
    for c in name.chars() {
        if c.is_ascii_alphanumeric() || c == '_' {
            out.push(c);
        } else {
            out.push('_');
        }
    }
    if !out.starts_with(|c: char| c.is_ascii_alphabetic()) {
        out.insert(0, '&');
    }
    out.truncate(255);
    let renamed = out != name;
    (out, renamed)
}

/// Renders `s` as a JSON string literal with the escapes JSON requires.
pub fn json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Renders `s` as a Verilog string literal.
pub(super) fn verilog_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\{:03o}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Renders `s` as a VHDL string literal (quotes doubled). Control
/// characters are not representable inside a string literal and are
/// replaced by a space.
pub(super) fn vhdl_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\"\""),
            c if (c as u32) < 0x20 => out.push(' '),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

// ---------------------------------------------------------------------------
// Indented output
// ---------------------------------------------------------------------------

/// A line-oriented text buffer with an indentation level.
#[derive(Debug, Default)]
pub(super) struct Out {
    buf: String,
    depth: usize,
    unit: &'static str,
}

impl Out {
    /// A buffer indenting with `unit` per level.
    pub(super) fn new(unit: &'static str) -> Self {
        Out {
            buf: String::new(),
            depth: 0,
            unit,
        }
    }

    /// Appends one indented line.
    pub(super) fn line(&mut self, text: &str) {
        for _ in 0..self.depth {
            self.buf.push_str(self.unit);
        }
        self.buf.push_str(text);
        self.buf.push('\n');
    }

    /// Appends an empty line.
    pub(super) fn blank(&mut self) {
        self.buf.push('\n');
    }

    /// Appends text without indentation or newline.
    pub(super) fn raw(&mut self, text: &str) {
        self.buf.push_str(text);
    }

    /// Increases the indentation.
    pub(super) fn indent(&mut self) {
        self.depth += 1;
    }

    /// Decreases the indentation.
    pub(super) fn dedent(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    /// The text so far.
    pub(super) fn finish(self) -> String {
        self.buf
    }
}

// ---------------------------------------------------------------------------
// Bit-level view of the cell form
// ---------------------------------------------------------------------------

/// One bit of a signal, either a constant or a numbered net bit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SigBit {
    /// A constant bit.
    Const(Bit),
    /// A bit of a net: the slot number handed out by [`BitView`].
    Slot(usize),
}

impl SigBit {
    /// A total order: constants (`0 1 x z`) before slots, slots by number.
    fn rank(self) -> (u8, usize) {
        match self {
            SigBit::Const(Bit::Zero) => (0, 0),
            SigBit::Const(Bit::One) => (1, 0),
            SigBit::Const(Bit::X) => (2, 0),
            SigBit::Const(Bit::Z) => (3, 0),
            SigBit::Slot(s) => (4, s),
        }
    }
}

impl PartialOrd for SigBit {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for SigBit {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.rank().cmp(&other.rank())
    }
}

/// A per-bit view of a module for the netlist emitters.
///
/// Every bit of every bit-vector net gets a *slot*; continuous assignments
/// with structural right-hand sides union the slots they connect (or bind a
/// slot to a constant), and [`BitView::canonical`] maps a raw bit to the
/// representative of its class: the constant, or the lowest slot in the
/// class. Slots are numbered in net order, bit 0 first, so representatives
/// are deterministic.
pub struct BitView<'a> {
    module: &'a Module,
    /// First slot of each net, by net index; `None` for non-bit-vector nets.
    net_base: Vec<Option<usize>>,
    /// The net and bit position owning each slot.
    owner: Vec<(NetId, u32)>,
    /// Union-find parent links.
    parent: Vec<usize>,
    /// The constant bound to a class, kept on its root.
    konst: Vec<Option<Bit>>,
    /// The smallest slot of a class, kept on its root.
    lowest: Vec<usize>,
}

impl<'a> BitView<'a> {
    /// Numbers the bits of `module` and applies its continuous assignments.
    ///
    /// Port nets get the lowest slots (inputs first, in port order), so a
    /// class that contains a port bit is represented by that port bit and
    /// the port's name survives in the output.
    pub fn new(module: &'a Module) -> Result<Self, EmitError> {
        let mut net_base = vec![None; module.nets.len()];
        let mut owner = Vec::new();
        let inputs = module.ports.iter().filter(|p| p.dir == PortDir::In);
        let others = module.ports.iter().filter(|p| p.dir != PortDir::In);
        let port_nets = inputs.chain(others).map(|p| p.net);
        let order = port_nets.chain(module.nets.ids());
        for id in order {
            if net_base[id.index()].is_some() {
                continue;
            }
            let net = &module.nets[id];
            if let Some(w) = net.ty.width() {
                net_base[id.index()] = Some(owner.len());
                owner.extend((0..w).map(|i| (id, i)));
            }
        }
        let n = owner.len();
        let mut view = BitView {
            module,
            net_base,
            owner,
            parent: (0..n).collect(),
            konst: vec![None; n],
            lowest: (0..n).collect(),
        };
        for assign in &module.assigns {
            let targets = view.lvalue_slots(&assign.target, assign.span)?;
            let values = view.expr_bits(assign.value)?;
            if targets.len() != values.len() {
                return Err(EmitError::new(
                    assign.span,
                    format!(
                        "assignment connects {} bits to {} bits",
                        values.len(),
                        targets.len()
                    ),
                ));
            }
            for (slot, value) in targets.into_iter().zip(values) {
                match value {
                    SigBit::Const(bit) => view.bind(slot, bit),
                    SigBit::Slot(other) => view.union(slot, other),
                }
            }
        }
        Ok(view)
    }

    /// The module viewed.
    pub fn module(&self) -> &'a Module {
        self.module
    }

    /// Number of slots.
    pub fn slots(&self) -> usize {
        self.owner.len()
    }

    /// The net and bit position that own `slot`.
    pub fn owner(&self, slot: usize) -> (NetId, u32) {
        self.owner[slot]
    }

    fn find(&self, mut slot: usize) -> usize {
        while self.parent[slot] != slot {
            slot = self.parent[slot];
        }
        slot
    }

    fn union(&mut self, a: usize, b: usize) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra == rb {
            return;
        }
        // Keep the root with the lowest slot so representatives are stable.
        let (keep, drop) = if self.lowest[ra] <= self.lowest[rb] {
            (ra, rb)
        } else {
            (rb, ra)
        };
        self.parent[drop] = keep;
        if self.konst[keep].is_none() {
            self.konst[keep] = self.konst[drop];
        }
    }

    fn bind(&mut self, slot: usize, bit: Bit) {
        let root = self.find(slot);
        if self.konst[root].is_none() {
            self.konst[root] = Some(bit);
        }
    }

    /// The representative of `bit`'s class.
    pub fn canonical(&self, bit: SigBit) -> SigBit {
        match bit {
            SigBit::Const(_) => bit,
            SigBit::Slot(slot) => {
                let root = self.find(slot);
                match self.konst[root] {
                    Some(c) => SigBit::Const(c),
                    None => SigBit::Slot(self.lowest[root]),
                }
            }
        }
    }

    /// True when `slot` is the representative of its class (and the class
    /// is not bound to a constant).
    pub fn is_representative(&self, slot: usize) -> bool {
        self.canonical(SigBit::Slot(slot)) == SigBit::Slot(slot)
    }

    /// The raw slots of a net, bit 0 first, or an error when the net is not
    /// a bit vector.
    pub fn net_slots(&self, net: NetId, span: Span) -> Result<Vec<usize>, EmitError> {
        let n = &self.module.nets[net];
        match (self.net_base[net.index()], n.ty.width()) {
            (Some(base), Some(w)) => Ok((base..base + w as usize).collect()),
            _ => Err(EmitError::new(
                span,
                format!(
                    "net `{}` has type `{}`, which a netlist cannot carry",
                    n.name, n.ty
                ),
            )),
        }
    }

    /// The canonical bits of a net, bit 0 first.
    pub fn net_bits(&self, net: NetId, span: Span) -> Result<Vec<SigBit>, EmitError> {
        Ok(self
            .net_slots(net, span)?
            .into_iter()
            .map(|s| self.canonical(SigBit::Slot(s)))
            .collect())
    }

    /// The raw slots written by an lvalue, bit 0 first.
    pub fn lvalue_slots(&self, lv: &Lvalue, span: Span) -> Result<Vec<usize>, EmitError> {
        match lv {
            Lvalue::Net(net) => self.net_slots(*net, span),
            Lvalue::Slice { net, hi, lo } => {
                let slots = self.net_slots(*net, span)?;
                let (lo, hi) = (*lo as usize, *hi as usize);
                if hi < lo || hi >= slots.len() {
                    return Err(EmitError::new(span, "slice is outside the net"));
                }
                Ok(slots[lo..=hi].to_vec())
            }
            Lvalue::Index { net, index } => {
                let slots = self.net_slots(*net, span)?;
                let i = self.const_index(*index, span)?;
                slots
                    .get(i)
                    .map(|s| vec![*s])
                    .ok_or_else(|| EmitError::new(span, "index is outside the net"))
            }
            Lvalue::Concat(parts) => {
                let mut out = Vec::new();
                for part in parts.iter().rev() {
                    out.extend(self.lvalue_slots(part, span)?);
                }
                Ok(out)
            }
            Lvalue::MemElem { .. } => Err(EmitError::new(
                span,
                "a memory element cannot be a netlist connection",
            )),
        }
    }

    fn const_index(&self, index: ExprId, span: Span) -> Result<usize, EmitError> {
        let expr = &self.module.exprs[index];
        match expr.as_const().and_then(|c| c.to_u64()) {
            Some(v) => usize::try_from(v)
                .map_err(|_| EmitError::new(expr.span, "index does not fit in memory")),
            None => Err(EmitError::new(
                if expr.span.is_empty() {
                    span
                } else {
                    expr.span
                },
                "a netlist connection needs a constant index; run synthesis first",
            )),
        }
    }

    /// The raw bits of a structural expression, bit 0 first.
    ///
    /// Structural expressions are constants, nets, constant slices and
    /// indices, concatenations, replications and resizes of structural
    /// expressions. Anything else is an error pointing at the expression.
    pub fn expr_bits(&self, id: ExprId) -> Result<Vec<SigBit>, EmitError> {
        let expr = &self.module.exprs[id];
        let span = expr.span;
        match &expr.kind {
            ExprKind::Const(c) => Ok((0..c.width()).map(|i| SigBit::Const(c.bit(i))).collect()),
            ExprKind::Net(net) => Ok(self
                .net_slots(*net, span)?
                .into_iter()
                .map(SigBit::Slot)
                .collect()),
            ExprKind::Slice { base, hi, lo } => {
                if !self.module.exprs[*base].ty.is_bits() {
                    return Err(EmitError::new(
                        span,
                        "an array slice cannot be a netlist connection",
                    ));
                }
                let bits = self.expr_bits(*base)?;
                let (lo, hi) = (*lo as usize, *hi as usize);
                if hi < lo || hi >= bits.len() {
                    return Err(EmitError::new(span, "slice is outside its operand"));
                }
                Ok(bits[lo..=hi].to_vec())
            }
            ExprKind::Index { base, index } => {
                if !self.module.exprs[*base].ty.is_bits() {
                    return Err(EmitError::new(
                        span,
                        "an array element cannot be a netlist connection",
                    ));
                }
                let bits = self.expr_bits(*base)?;
                let i = self.const_index(*index, span)?;
                Ok(vec![bits.get(i).copied().unwrap_or(SigBit::Const(Bit::X))])
            }
            ExprKind::IndexedSlice {
                base,
                offset,
                width,
                up,
            } => {
                let bits = self.expr_bits(*base)?;
                let off = self.const_index(*offset, span)?;
                let w = *width as usize;
                let lo = if *up {
                    off
                } else {
                    (off + 1).saturating_sub(w)
                };
                Ok((lo..lo + w)
                    .map(|i| bits.get(i).copied().unwrap_or(SigBit::Const(Bit::X)))
                    .collect())
            }
            ExprKind::Concat(parts) => {
                let mut out = Vec::new();
                for part in parts.iter().rev() {
                    out.extend(self.expr_bits(*part)?);
                }
                Ok(out)
            }
            ExprKind::Replicate { count, expr } => {
                let bits = self.expr_bits(*expr)?;
                Ok(bits.repeat(*count as usize))
            }
            ExprKind::Resize {
                expr: inner,
                width,
                signed,
            } => {
                let operand = &self.module.exprs[*inner];
                if !operand.ty.is_bits() {
                    return Err(EmitError::new(
                        span,
                        "a resize of a non-vector cannot be a netlist connection",
                    ));
                }
                let mut bits = self.expr_bits(*inner)?;
                let w = *width as usize;
                let fill = if *signed && operand.ty.is_signed() {
                    bits.last().copied().unwrap_or(SigBit::Const(Bit::Zero))
                } else {
                    SigBit::Const(Bit::Zero)
                };
                bits.resize(w, fill);
                Ok(bits)
            }
            ExprKind::String(_) => Err(EmitError::new(
                span,
                "a string cannot be a netlist connection",
            )),
            ExprKind::Unary { op, .. } => Err(not_structural(span, op.name())),
            ExprKind::Binary { op, .. } => Err(not_structural(span, op.name())),
            ExprKind::Ternary { .. } => Err(not_structural(span, "mux")),
            ExprKind::MemRead { .. } => Err(not_structural(span, "memory read")),
            ExprKind::Call { name, .. } => Err(not_structural(span, name.as_str())),
        }
    }

    /// The canonical bits of a structural expression, bit 0 first.
    pub fn expr_canonical(&self, id: ExprId) -> Result<Vec<SigBit>, EmitError> {
        Ok(self
            .expr_bits(id)?
            .into_iter()
            .map(|b| self.canonical(b))
            .collect())
    }
}

fn not_structural(span: Span, what: &str) -> EmitError {
    EmitError::new(
        span,
        format!("`{what}` is not a netlist connection; synthesise the design to cells first"),
    )
}

/// The first process of `module` as an error, for the formats that only
/// accept the cell form.
pub(super) fn require_cell_form(module: &Module, format: &str) -> Result<(), EmitError> {
    if let Some((_, process)) = module.processes.iter().next() {
        let what = match &process.name {
            Some(name) => format!("process `{name}`"),
            None => "a process".to_owned(),
        };
        return Err(EmitError::new(
            process.span,
            format!(
                "module `{}` still has {what}; {format} takes the cell form only, synthesise first",
                module.name
            ),
        ));
    }
    Ok(())
}

/// The modules of a design with instantiated modules before the modules
/// that instantiate them, falling back to arena order when the hierarchy
/// is cyclic.
pub(super) fn leaves_first(design: &Design) -> Vec<super::design::ModuleId> {
    design
        .topological_order()
        .unwrap_or_else(|_| design.modules.ids().collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::Type;
    use crate::ir::builder::ModuleBuilder;
    use crate::ir::process::Lvalue;
    use crate::logic::Logic;
    use crate::source::SourceMap;

    fn span() -> Span {
        let mut map = SourceMap::new();
        let id = map.add("t", "").unwrap();
        Span::new(id, 0, 0)
    }

    #[test]
    fn keyword_tables_are_sorted() {
        assert!(VERILOG_KEYWORDS.windows(2).all(|w| w[0] < w[1]));
        assert!(VHDL_RESERVED.windows(2).all(|w| w[0] < w[1]));
        assert!(is_verilog_keyword("module"));
        assert!(is_verilog_keyword("logic"));
        assert!(!is_verilog_keyword("modules"));
        assert!(is_vhdl_reserved("Entity"));
        assert!(!is_vhdl_reserved("entities"));
    }

    #[test]
    fn verilog_escaping() {
        assert_eq!(verilog_ident("clk"), "clk");
        assert_eq!(verilog_ident("a$b_9"), "a$b_9");
        assert_eq!(verilog_ident("module"), "\\module ");
        assert_eq!(verilog_ident("9a"), "\\9a ");
        assert_eq!(verilog_ident("a[0]"), "\\a[0] ");
        assert_eq!(verilog_ident("a b"), "\\a_b ");
        assert_eq!(verilog_ident("é"), "\\_ ");
        assert_eq!(verilog_ident(""), "\\_ ");
        assert_eq!(verilog_string("a\"b\\\n"), "\"a\\\"b\\\\\\n\"");
    }

    #[test]
    fn vhdl_escaping() {
        assert_eq!(vhdl_ident("clk"), "clk");
        assert_eq!(vhdl_ident("Clk_1"), "Clk_1");
        assert_eq!(vhdl_ident("entity"), "\\entity\\");
        assert_eq!(vhdl_ident("ENTITY"), "\\ENTITY\\");
        assert_eq!(vhdl_ident("_a"), "\\_a\\");
        assert_eq!(vhdl_ident("a_"), "\\a_\\");
        assert_eq!(vhdl_ident("a__b"), "\\a__b\\");
        assert_eq!(vhdl_ident("9a"), "\\9a\\");
        assert_eq!(vhdl_ident("a$b"), "\\a$b\\");
        assert_eq!(vhdl_ident("a\\b"), "\\a\\\\b\\");
        assert_eq!(vhdl_ident("a b"), "\\a b\\");
        assert_eq!(vhdl_ident(""), "\\_\\");
        assert_eq!(vhdl_extended("clk"), "\\clk\\");
        assert_eq!(vhdl_string("say \"hi\""), "\"say \"\"hi\"\"\"");
    }

    #[test]
    fn edif_escaping() {
        assert_eq!(edif_ident("clk"), ("clk".to_owned(), false));
        assert_eq!(edif_ident("a[0]"), ("a_0_".to_owned(), true));
        assert_eq!(edif_ident("_x"), ("&_x".to_owned(), true));
        assert_eq!(edif_ident("9"), ("&9".to_owned(), true));
        assert_eq!(edif_ident("$abc"), ("&_abc".to_owned(), true));
    }

    #[test]
    fn json_escaping() {
        assert_eq!(json_string("plain"), "\"plain\"");
        assert_eq!(json_string("a\"b"), "\"a\\\"b\"");
        assert_eq!(json_string("a\\b"), "\"a\\\\b\"");
        assert_eq!(json_string("a\nb\tc\r"), "\"a\\nb\\tc\\r\"");
        assert_eq!(json_string("\u{01}"), "\"\\u0001\"");
        assert_eq!(json_string("\u{08}\u{0c}"), "\"\\b\\f\"");
        assert_eq!(json_string("ünïcode"), "\"ünïcode\"");
    }

    #[test]
    fn format_names() {
        for f in Format::ALL {
            assert_eq!(Format::from_name(f.extension()), Some(f));
        }
        assert_eq!(Format::from_name("VHDL"), Some(Format::Vhdl));
        assert_eq!(Format::from_name("edf"), Some(Format::Edif));
        assert_eq!(Format::from_name("txt"), None);
        let e = EmitError::new(span(), "nope");
        assert_eq!(e.to_string(), "nope");
    }

    #[test]
    fn out_indents() {
        let mut out = Out::new("  ");
        out.line("a");
        out.indent();
        out.line("b");
        out.dedent();
        out.dedent();
        out.blank();
        out.raw("c");
        assert_eq!(out.finish(), "a\n  b\n\nc");
    }

    #[test]
    fn bit_view_unions_assigns() {
        let span = span();
        let mut b = ModuleBuilder::new("m", span);
        let a = b.input("a", Type::bits(4));
        let y = b.output("y", Type::bits(8));
        let z = b.output("z", Type::bits(2));
        let av = b.net(a);
        let hi = b.slice(av, 3, 2);
        let k = b.constant(Logic::parse_verilog("2'b1z").unwrap());
        let cat = b.concat(vec![k, hi, av]);
        b.assign(y, cat);
        let yv = b.net(y);
        let ys = b.slice(yv, 7, 6);
        b.assign(z, ys);
        let module = b.finish();
        let view = BitView::new(&module).unwrap();
        assert_eq!(view.slots(), 14);
        assert_eq!(view.owner(5), (y, 1));
        // y[3:0] aliases a, y[5:4] aliases a[3:2], y[6] is z, y[7] is 1,
        // and z aliases y[7:6].
        let bits = view.net_bits(y, span).unwrap();
        assert_eq!(bits[0], SigBit::Slot(0));
        assert_eq!(bits[3], SigBit::Slot(3));
        assert_eq!(bits[4], SigBit::Slot(2));
        assert_eq!(bits[5], SigBit::Slot(3));
        assert_eq!(bits[6], SigBit::Const(Bit::Z));
        assert_eq!(bits[7], SigBit::Const(Bit::One));
        let zb = view.net_bits(z, span).unwrap();
        assert_eq!(zb, [SigBit::Const(Bit::Z), SigBit::Const(Bit::One)]);
        assert!(SigBit::Const(Bit::Z) < SigBit::Slot(0));
        assert!(SigBit::Slot(1) < SigBit::Slot(2));
        assert!(view.is_representative(0));
        assert!(!view.is_representative(8));
        assert!(!view.is_representative(10));
        assert_eq!(view.module().name, "m");
    }

    #[test]
    fn bit_view_rejects_logic() {
        let span = span();
        let mut b = ModuleBuilder::new("m", span);
        let a = b.input("a", Type::bits(4));
        let y = b.output("y", Type::bits(4));
        let av = b.net(a);
        let n = b.not(av);
        b.assign(y, n);
        let module = b.finish();
        let err = BitView::new(&module).err().unwrap();
        assert!(err.message.contains("`not`"));

        let mut b = ModuleBuilder::new("m", span);
        let i = b.add_net("i", Type::Integer);
        let y = b.output("y", Type::bits(4));
        let iv = b.net(i);
        let r = b.resize(iv, 4, false);
        b.assign(y, r);
        let module = b.finish();
        assert!(BitView::new(&module).is_err());

        let mut b = ModuleBuilder::new("m", span);
        let m = b.memory("m", Type::bits(4), 4);
        let k = b.const_u64(2, 0);
        let v = b.const_u64(4, 0);
        b.assign(Lvalue::MemElem { mem: m, addr: k }, v);
        let module = b.finish();
        assert!(BitView::new(&module).is_err());
    }

    #[test]
    fn bit_view_structural_forms() {
        let span = span();
        let mut b = ModuleBuilder::new("m", span);
        let a = b.input("a", Type::sbits(2));
        let y = b.output("y", Type::bits(9));
        let av = b.net(a);
        let one = b.const_u64(1, 1);
        let idx = b.index(av, one);
        let sext = b.sext(av, 4);
        let rep = b.replicate(2, idx);
        let sl = b.indexed_slice(av, one, 2, false);
        let cat = b.concat(vec![sext, rep, sl, idx]);
        b.assign(y, cat);
        let module = b.finish();
        let view = BitView::new(&module).unwrap();
        let bits = view.net_bits(y, span).unwrap();
        assert_eq!(bits[0], SigBit::Slot(1)); // a[1]
        assert_eq!(bits[1], SigBit::Slot(0)); // a[1 -: 2] = a[1:0]
        assert_eq!(bits[2], SigBit::Slot(1));
        assert_eq!(bits[3], SigBit::Slot(1)); // {2{a[1]}}
        assert_eq!(bits[4], SigBit::Slot(1));
        assert_eq!(bits[5], SigBit::Slot(0)); // sext: a[0], a[1], a[1], a[1]
        assert_eq!(bits[8], SigBit::Slot(1));
        assert!(require_cell_form(&module, "x").is_ok());
        assert_eq!(leaves_first(&Design::new()).len(), 0);
    }
}
