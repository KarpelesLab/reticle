//! The cell form: a netlist of generic primitives.
//!
//! After synthesis a module is a set of [`Cell`]s wired through nets. Each
//! cell has a [`CellKind`] from a small, fixed primitive set, named input
//! ports connected to expressions and named output ports driving nets.
//! Inputs are expressions rather than bare nets so a slice, constant or
//! concatenation can feed a cell without an intermediate net; outputs are
//! nets because a cell is the driver of what it produces.
//!
//! Widths are not stored on the cell: they follow from the types of the
//! connected expressions and nets, and the validator checks they agree with
//! the rules in the table below. Arithmetic and comparison cells are signed
//! when both operands are signed, as for the matching expression operators.
//!
//! # Primitive set
//!
//! | Kind          | Inputs               | Outputs | Rule                                                  |
//! |---------------|----------------------|---------|-------------------------------------------------------|
//! | `Not`, `Buf`  | `a`                  | `y`     | `y` and `a` same width                                |
//! | `And` `Or` `Xor` | `a`, `b`          | `y`     | all same width                                        |
//! | `Add` `Sub` `Mul` `Div` `Mod` | `a`, `b` | `y` | all same width                                     |
//! | `Shl` `Shr` `Sshr` | `a`, `b`        | `y`     | `y` and `a` same width; `b` any width                 |
//! | `Eq` `Ne` `Lt` `Le` `Gt` `Ge` | `a`, `b` | `y` | `a` and `b` same width; `y` one bit                |
//! | `ReduceAnd` `ReduceOr` `ReduceXor` | `a` | `y`  | `y` one bit                                           |
//! | `Mux`         | `a`, `b`, `s`        | `y`     | `a` `b` `y` same width; `s` one bit; `s = 1` picks `b`|
//! | `Pmux`        | `a`, `b`, `s`        | `y`     | `s` has n bits (one-hot), `b` is n × width(`y`), `a` is the default |
//! | `Dff`         | `clk`, `d`, \[`en`\], \[`rst`\] | `q` | `d` and `q` same width; `clk` `en` `rst` one bit; reset value width of `q` |
//! | `Dlatch`      | `en`, `d`            | `q`     | `d` and `q` same width; `en` one bit                  |
//! | `MemRdPort`   | `addr`, \[`clk`, `en`\] | `data` | `data` is the element type; `clk`/`en` when clocked |
//! | `MemWrPort`   | `addr`, `data`, `en`, \[`clk`\] | —  | `data` is the element type; `en` one bit           |
//! | `Lut`         | `a`                  | `y`     | `a` has `k` bits, `init` has 2^k bits, `y` one bit    |
//! | `Tristate`    | `a`, `en`            | `y`     | `y` and `a` same width; `en` one bit; `z` when disabled |
//! | `Blackbox`    | any                  | any     | not checked                                           |

use super::Name;
use super::arena::define_id;
use super::attr::Attrs;
use super::design::{MemoryId, NetId};
use super::expr::ExprId;
use super::types::Const;
use crate::source::Span;

define_id!(
    /// Identifies a cell inside a module.
    CellId,
    "c"
);

/// The reset of a [`CellKind::Dff`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reset {
    /// True for an asynchronous reset, false for a synchronous one.
    pub asynchronous: bool,
    /// True when the reset is active high.
    pub active_high: bool,
    /// The value loaded while reset is active.
    pub value: Const,
}

/// The primitive a [`Cell`] implements; see the table in the module docs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CellKind {
    /// Bitwise complement.
    Not,
    /// Bitwise AND.
    And,
    /// Bitwise OR.
    Or,
    /// Bitwise XOR.
    Xor,
    /// Two-way multiplexer.
    Mux,
    /// Parallel (one-hot) multiplexer.
    Pmux,
    /// Adder.
    Add,
    /// Subtractor.
    Sub,
    /// Multiplier.
    Mul,
    /// Divider.
    Div,
    /// Remainder.
    Mod,
    /// Shift left.
    Shl,
    /// Logical shift right.
    Shr,
    /// Arithmetic shift right.
    Sshr,
    /// Equality comparator.
    Eq,
    /// Inequality comparator.
    Ne,
    /// Less-than comparator.
    Lt,
    /// Less-or-equal comparator.
    Le,
    /// Greater-than comparator.
    Gt,
    /// Greater-or-equal comparator.
    Ge,
    /// AND reduction.
    ReduceAnd,
    /// OR reduction.
    ReduceOr,
    /// XOR reduction.
    ReduceXor,
    /// Edge-triggered flip-flop with optional enable and reset.
    Dff {
        /// True when the flop captures on the rising clock edge.
        clk_pos: bool,
        /// True when the `en` input exists.
        has_enable: bool,
        /// The reset, if any.
        reset: Option<Reset>,
    },
    /// Level-sensitive latch, transparent while `en` is high.
    Dlatch,
    /// A read port of a memory.
    MemRdPort {
        /// The memory read.
        mem: MemoryId,
        /// True when the read is registered on `clk` (with `en`).
        clocked: bool,
    },
    /// A write port of a memory.
    MemWrPort {
        /// The memory written.
        mem: MemoryId,
        /// True when the write happens on `clk`; otherwise whenever `en` is
        /// high.
        clocked: bool,
    },
    /// A k-input lookup table; `init` bit `i` is the output for input `i`.
    Lut {
        /// Number of inputs.
        k: u32,
        /// The truth table, 2^k bits.
        init: Const,
    },
    /// A buffer (identity).
    Buf,
    /// A tri-state driver.
    Tristate,
    /// A cell defined outside the design (a vendor primitive or an opaque
    /// IP core), identified by name; its ports are not checked.
    Blackbox(Name),
}

impl CellKind {
    /// The keyword introducing this kind in the text format.
    pub fn keyword(&self) -> &'static str {
        match self {
            CellKind::Not => "not",
            CellKind::And => "and",
            CellKind::Or => "or",
            CellKind::Xor => "xor",
            CellKind::Mux => "mux",
            CellKind::Pmux => "pmux",
            CellKind::Add => "add",
            CellKind::Sub => "sub",
            CellKind::Mul => "mul",
            CellKind::Div => "div",
            CellKind::Mod => "mod",
            CellKind::Shl => "shl",
            CellKind::Shr => "shr",
            CellKind::Sshr => "sshr",
            CellKind::Eq => "eq",
            CellKind::Ne => "ne",
            CellKind::Lt => "lt",
            CellKind::Le => "le",
            CellKind::Gt => "gt",
            CellKind::Ge => "ge",
            CellKind::ReduceAnd => "rand",
            CellKind::ReduceOr => "ror",
            CellKind::ReduceXor => "rxor",
            CellKind::Dff { .. } => "dff",
            CellKind::Dlatch => "dlatch",
            CellKind::MemRdPort { .. } => "memrd",
            CellKind::MemWrPort { .. } => "memwr",
            CellKind::Lut { .. } => "lut",
            CellKind::Buf => "buf",
            CellKind::Tristate => "tristate",
            CellKind::Blackbox(_) => "blackbox",
        }
    }

    /// The kind with the given keyword, for kinds that carry no data.
    pub fn simple_from_keyword(name: &str) -> Option<CellKind> {
        Some(match name {
            "not" => CellKind::Not,
            "and" => CellKind::And,
            "or" => CellKind::Or,
            "xor" => CellKind::Xor,
            "mux" => CellKind::Mux,
            "pmux" => CellKind::Pmux,
            "add" => CellKind::Add,
            "sub" => CellKind::Sub,
            "mul" => CellKind::Mul,
            "div" => CellKind::Div,
            "mod" => CellKind::Mod,
            "shl" => CellKind::Shl,
            "shr" => CellKind::Shr,
            "sshr" => CellKind::Sshr,
            "eq" => CellKind::Eq,
            "ne" => CellKind::Ne,
            "lt" => CellKind::Lt,
            "le" => CellKind::Le,
            "gt" => CellKind::Gt,
            "ge" => CellKind::Ge,
            "rand" => CellKind::ReduceAnd,
            "ror" => CellKind::ReduceOr,
            "rxor" => CellKind::ReduceXor,
            "dlatch" => CellKind::Dlatch,
            "buf" => CellKind::Buf,
            "tristate" => CellKind::Tristate,
            _ => return None,
        })
    }

    /// The input port names this kind requires, in canonical order.
    /// Empty for `Blackbox`, whose ports are free.
    pub fn input_ports(&self) -> Vec<&'static str> {
        match self {
            CellKind::Not | CellKind::Buf | CellKind::Lut { .. } => vec!["a"],
            CellKind::ReduceAnd | CellKind::ReduceOr | CellKind::ReduceXor => vec!["a"],
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
            | CellKind::Ge => vec!["a", "b"],
            CellKind::Mux | CellKind::Pmux => vec!["a", "b", "s"],
            CellKind::Dff {
                has_enable, reset, ..
            } => {
                let mut ports = vec!["clk", "d"];
                if *has_enable {
                    ports.push("en");
                }
                if reset.is_some() {
                    ports.push("rst");
                }
                ports
            }
            CellKind::Dlatch => vec!["en", "d"],
            CellKind::MemRdPort { clocked, .. } => {
                if *clocked {
                    vec!["addr", "clk", "en"]
                } else {
                    vec!["addr"]
                }
            }
            CellKind::MemWrPort { clocked, .. } => {
                if *clocked {
                    vec!["addr", "data", "en", "clk"]
                } else {
                    vec!["addr", "data", "en"]
                }
            }
            CellKind::Tristate => vec!["a", "en"],
            CellKind::Blackbox(_) => Vec::new(),
        }
    }

    /// The output port names this kind requires. Empty for `Blackbox` and
    /// `MemWrPort`.
    pub fn output_ports(&self) -> Vec<&'static str> {
        match self {
            CellKind::Dff { .. } | CellKind::Dlatch => vec!["q"],
            CellKind::MemRdPort { .. } => vec!["data"],
            CellKind::MemWrPort { .. } | CellKind::Blackbox(_) => Vec::new(),
            _ => vec!["y"],
        }
    }

    /// True for cells whose outputs depend only on their current inputs.
    pub fn is_combinational(&self) -> bool {
        !matches!(
            self,
            CellKind::Dff { .. }
                | CellKind::Dlatch
                | CellKind::MemRdPort { .. }
                | CellKind::MemWrPort { .. }
                | CellKind::Blackbox(_)
        )
    }

    /// True when the cell's result is one bit regardless of operand width.
    pub fn is_predicate(&self) -> bool {
        matches!(
            self,
            CellKind::Eq
                | CellKind::Ne
                | CellKind::Lt
                | CellKind::Le
                | CellKind::Gt
                | CellKind::Ge
                | CellKind::ReduceAnd
                | CellKind::ReduceOr
                | CellKind::ReduceXor
                | CellKind::Lut { .. }
        )
    }
}

/// One primitive instance in the cell form.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cell {
    /// Unique among the module's cells.
    pub name: Name,
    /// The primitive.
    pub kind: CellKind,
    /// Input ports and the expressions feeding them, in port order.
    pub inputs: Vec<(Name, ExprId)>,
    /// Output ports and the nets they drive, in port order.
    pub outputs: Vec<(Name, NetId)>,
    /// Parameter overrides, only meaningful for `Blackbox` cells.
    pub params: Attrs,
    /// Source attributes.
    pub attrs: Attrs,
    /// Where the cell came from.
    pub span: Span,
}

impl Cell {
    /// The expression connected to input port `port`.
    pub fn input(&self, port: &str) -> Option<ExprId> {
        self.inputs
            .iter()
            .find(|(n, _)| n.as_str() == port)
            .map(|(_, e)| *e)
    }

    /// The net driven by output port `port`.
    pub fn output(&self, port: &str) -> Option<NetId> {
        self.outputs
            .iter()
            .find(|(n, _)| n.as_str() == port)
            .map(|(_, n)| *n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keywords_and_ports() {
        for kw in [
            "not", "and", "or", "xor", "mux", "pmux", "add", "sub", "mul", "div", "mod", "shl",
            "shr", "sshr", "eq", "ne", "lt", "le", "gt", "ge", "rand", "ror", "rxor", "dlatch",
            "buf", "tristate",
        ] {
            let kind = CellKind::simple_from_keyword(kw).unwrap();
            assert_eq!(kind.keyword(), kw);
            assert!(!kind.output_ports().is_empty());
        }
        assert_eq!(CellKind::simple_from_keyword("dff"), None);
        let dff = CellKind::Dff {
            clk_pos: true,
            has_enable: true,
            reset: Some(Reset {
                asynchronous: false,
                active_high: true,
                value: Const::zero(1),
            }),
        };
        assert_eq!(dff.input_ports(), ["clk", "d", "en", "rst"]);
        assert_eq!(dff.output_ports(), ["q"]);
        assert!(!dff.is_combinational());
        assert!(CellKind::Eq.is_predicate());
        assert!(CellKind::Add.is_combinational());
        assert_eq!(
            CellKind::MemWrPort {
                mem: MemoryId(0),
                clocked: true
            }
            .input_ports(),
            ["addr", "data", "en", "clk"]
        );
        assert!(
            CellKind::MemWrPort {
                mem: MemoryId(0),
                clocked: false
            }
            .output_ports()
            .is_empty()
        );
        assert!(CellKind::Blackbox(Name::new("X")).input_ports().is_empty());
    }
}
