//! The process form: structured behavioural code.
//!
//! A [`Process`] is a block of statements with a trigger ([`ProcessKind`]).
//! It is what the frontends produce for `always` / `initial` blocks and
//! VHDL processes, what the simulator executes, and what synthesis lowers
//! into cells. Statements are a small structured language: assignments to
//! [`Lvalue`]s, `if` / `case`, loops, waits, memory writes, assertions and
//! system calls. Expressions inside statements are [`ExprId`]s into the
//! owning module's arena.
//!
//! Time is expressed with [`Delay`] values in explicit [`TimeUnit`]s and a
//! per-module [`Timescale`], so mixed-language designs never have to guess
//! what a bare number means.

use super::Name;
use super::arena::define_id;
use super::attr::Attrs;
use super::design::{MemoryId, NetId};
use super::expr::ExprId;
use crate::source::Span;

define_id!(
    /// Identifies a process inside a module.
    ProcessId,
    "p"
);

/// A unit of simulated time.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum TimeUnit {
    /// Femtoseconds.
    Fs,
    /// Picoseconds.
    Ps,
    /// Nanoseconds.
    Ns,
    /// Microseconds.
    Us,
    /// Milliseconds.
    Ms,
    /// Seconds.
    S,
}

impl TimeUnit {
    /// Every unit, smallest first.
    pub const ALL: [TimeUnit; 6] = [
        TimeUnit::Fs,
        TimeUnit::Ps,
        TimeUnit::Ns,
        TimeUnit::Us,
        TimeUnit::Ms,
        TimeUnit::S,
    ];

    /// The unit's suffix as written in HDL and in the text format.
    pub fn name(self) -> &'static str {
        match self {
            TimeUnit::Fs => "fs",
            TimeUnit::Ps => "ps",
            TimeUnit::Ns => "ns",
            TimeUnit::Us => "us",
            TimeUnit::Ms => "ms",
            TimeUnit::S => "s",
        }
    }

    /// The unit with the given suffix.
    pub fn from_name(name: &str) -> Option<TimeUnit> {
        TimeUnit::ALL.into_iter().find(|u| u.name() == name)
    }

    /// The number of femtoseconds in one of this unit.
    pub fn in_fs(self) -> u64 {
        match self {
            TimeUnit::Fs => 1,
            TimeUnit::Ps => 1_000,
            TimeUnit::Ns => 1_000_000,
            TimeUnit::Us => 1_000_000_000,
            TimeUnit::Ms => 1_000_000_000_000,
            TimeUnit::S => 1_000_000_000_000_000,
        }
    }
}

/// A fixed duration.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Delay {
    /// Number of `unit`s.
    pub value: u64,
    /// The unit.
    pub unit: TimeUnit,
}

impl Delay {
    /// Builds a delay.
    pub fn new(value: u64, unit: TimeUnit) -> Self {
        Delay { value, unit }
    }

    /// The delay in femtoseconds, saturating on overflow.
    pub fn to_fs(self) -> u64 {
        self.value.saturating_mul(self.unit.in_fs())
    }
}

/// A module's time unit and precision (Verilog `` `timescale ``, VHDL
/// resolution limit).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Timescale {
    /// What a delay written without a unit means (`1 ns`).
    pub unit: Delay,
    /// The granularity delays are rounded to (`1 ps`).
    pub precision: Delay,
}

/// Which transition of a net a process or wait reacts to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Polarity {
    /// Rising edge (`posedge`, `rising_edge`).
    Pos,
    /// Falling edge (`negedge`, `falling_edge`).
    Neg,
    /// Any change of value (`'event`, plain sensitivity).
    Any,
}

/// A net together with the transition that triggers something.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Edge {
    /// The watched net.
    pub net: NetId,
    /// The transition.
    pub polarity: Polarity,
}

impl Edge {
    /// A rising edge on `net`.
    pub fn pos(net: NetId) -> Self {
        Edge {
            net,
            polarity: Polarity::Pos,
        }
    }

    /// A falling edge on `net`.
    pub fn neg(net: NetId) -> Self {
        Edge {
            net,
            polarity: Polarity::Neg,
        }
    }

    /// Any change on `net`.
    pub fn any(net: NetId) -> Self {
        Edge {
            net,
            polarity: Polarity::Any,
        }
    }
}

/// What starts a process.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProcessKind {
    /// Combinational: re-evaluated whenever anything it reads changes
    /// (`always_comb`, `always @*`, VHDL processes with a complete
    /// sensitivity list once analysed as such).
    Comb,
    /// Clocked: runs on the listed clock edges and on the listed
    /// asynchronous control edges (`always_ff`, `always @(posedge clk or
    /// negedge rst_n)`).
    Sequential {
        /// Clock edges.
        clocks: Vec<Edge>,
        /// Asynchronous reset / set edges.
        resets: Vec<Edge>,
    },
    /// Runs once at time zero (`initial`).
    Initial,
    /// Runs whenever one of the listed nets changes (an explicit
    /// sensitivity list that has not been classified further).
    Sensitive(Vec<NetId>),
    /// Runs from time zero and controls its own timing with `wait`
    /// statements (`always` without sensitivity, VHDL processes without a
    /// sensitivity list, testbench code).
    Free,
}

impl ProcessKind {
    /// A sequential process with one rising-edge clock and no asynchronous
    /// control.
    pub fn posedge(clk: NetId) -> Self {
        ProcessKind::Sequential {
            clocks: vec![Edge::pos(clk)],
            resets: Vec::new(),
        }
    }

    /// The keyword naming this kind in the text format.
    pub fn keyword(&self) -> &'static str {
        match self {
            ProcessKind::Comb => "comb",
            ProcessKind::Sequential { .. } => "seq",
            ProcessKind::Initial => "initial",
            ProcessKind::Sensitive(_) => "sensitive",
            ProcessKind::Free => "free",
        }
    }
}

/// A block of statements with a trigger.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Process {
    /// Optional label; unique among the module's named processes.
    pub name: Option<Name>,
    /// What starts the process.
    pub kind: ProcessKind,
    /// The statements, executed in order.
    pub body: Block,
    /// Source attributes.
    pub attrs: Attrs,
    /// Where the process came from.
    pub span: Span,
}

/// A sequence of statements.
pub type Block = Vec<Stmt>;

/// Where an assignment writes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Lvalue {
    /// A whole net.
    Net(NetId),
    /// A constant part-select of a net, `net[hi:lo]`.
    Slice {
        /// The net.
        net: NetId,
        /// Most significant bit written.
        hi: u32,
        /// Least significant bit written.
        lo: u32,
    },
    /// A variable bit- or element-select of a net, `net[index]`.
    Index {
        /// The net.
        net: NetId,
        /// The index expression.
        index: ExprId,
    },
    /// Several targets written from one value; the first receives the most
    /// significant bits.
    Concat(Vec<Lvalue>),
    /// One memory element, `mem[addr]`.
    MemElem {
        /// The memory.
        mem: MemoryId,
        /// The element address.
        addr: ExprId,
    },
}

impl Lvalue {
    /// Every net written by this target, in order of appearance.
    pub fn nets(&self) -> Vec<NetId> {
        match self {
            Lvalue::Net(n) | Lvalue::Slice { net: n, .. } | Lvalue::Index { net: n, .. } => {
                vec![*n]
            }
            Lvalue::Concat(parts) => parts.iter().flat_map(Lvalue::nets).collect(),
            Lvalue::MemElem { .. } => Vec::new(),
        }
    }
}

/// Whether an assignment takes effect immediately or at the end of the
/// time step.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AssignKind {
    /// Verilog `=`, VHDL variable assignment.
    Blocking,
    /// Verilog `<=`, VHDL signal assignment.
    NonBlocking,
}

/// How `case` items are matched.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CaseKind {
    /// Exact 4-state match (`case`).
    Plain,
    /// `z` (and `?`) bits in items are wildcards (`casez`).
    Z,
    /// `x` and `z` bits in items are wildcards (`casex`).
    X,
}

impl CaseKind {
    /// The keyword in the text format.
    pub fn keyword(self) -> &'static str {
        match self {
            CaseKind::Plain => "case",
            CaseKind::Z => "casez",
            CaseKind::X => "casex",
        }
    }
}

/// SystemVerilog `unique` / `priority` qualifiers on `case` and `if`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CaseQualifier {
    /// No qualifier.
    None,
    /// Exactly one item matches; synthesis may build parallel logic.
    Unique,
    /// Items are tested in order and at least one matches.
    Priority,
}

/// One `case` item: a list of values and the block they select.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaseArm {
    /// Values that select this arm; any match wins.
    pub values: Vec<ExprId>,
    /// Statements run when selected.
    pub body: Block,
}

/// What a `wait` statement waits for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WaitKind {
    /// A delay, in the module's timescale unit (`#expr`, `wait for`).
    Delay(ExprId),
    /// One of the listed transitions (`@(...)`, `wait on`).
    Event(Vec<Edge>),
    /// The condition becoming true (`wait(expr)`, `wait until`).
    Until(ExprId),
}

/// Severity of an assertion or report.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ReportSeverity {
    /// Informational.
    Note,
    /// Suspicious but continues.
    Warning,
    /// A failure; simulation continues by default.
    Error,
    /// A failure that stops simulation.
    Failure,
}

impl ReportSeverity {
    /// The keyword in the text format.
    pub fn keyword(self) -> &'static str {
        match self {
            ReportSeverity::Note => "note",
            ReportSeverity::Warning => "warning",
            ReportSeverity::Error => "error",
            ReportSeverity::Failure => "failure",
        }
    }

    /// The severity with the given keyword.
    pub fn from_keyword(name: &str) -> Option<ReportSeverity> {
        match name {
            "note" => Some(ReportSeverity::Note),
            "warning" => Some(ReportSeverity::Warning),
            "error" => Some(ReportSeverity::Error),
            "failure" => Some(ReportSeverity::Failure),
            _ => None,
        }
    }
}

/// Which memory file task a [`StmtKind::MemFile`] performs (IEEE
/// 1364-2005 §17.2.9, and `$writememh` / `$writememb` of IEEE 1800-2017
/// §21.5).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum MemFileOp {
    /// `$readmemh`: load hexadecimal words.
    ReadHex,
    /// `$readmemb`: load binary words.
    ReadBin,
    /// `$writememh`: save the contents as hexadecimal words.
    WriteHex,
    /// `$writememb`: save the contents as binary words.
    WriteBin,
}

impl MemFileOp {
    /// Every operation, in a fixed order.
    pub const ALL: [MemFileOp; 4] = [
        MemFileOp::ReadHex,
        MemFileOp::ReadBin,
        MemFileOp::WriteHex,
        MemFileOp::WriteBin,
    ];

    /// The keyword in the text format, which is also the Verilog task name
    /// without its `$`.
    pub fn keyword(self) -> &'static str {
        match self {
            MemFileOp::ReadHex => "readmemh",
            MemFileOp::ReadBin => "readmemb",
            MemFileOp::WriteHex => "writememh",
            MemFileOp::WriteBin => "writememb",
        }
    }

    /// The operation with the given keyword (without the `$`).
    pub fn from_keyword(name: &str) -> Option<MemFileOp> {
        MemFileOp::ALL.into_iter().find(|op| op.keyword() == name)
    }

    /// True for the two loads.
    pub fn is_read(self) -> bool {
        matches!(self, MemFileOp::ReadHex | MemFileOp::ReadBin)
    }

    /// True for the hexadecimal forms.
    pub fn is_hex(self) -> bool {
        matches!(self, MemFileOp::ReadHex | MemFileOp::WriteHex)
    }
}

/// The payload of a statement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StmtKind {
    /// `target = value` or `target <= value`, optionally delayed.
    Assign {
        /// Where the value goes.
        target: Lvalue,
        /// The value.
        value: ExprId,
        /// Blocking or non-blocking.
        kind: AssignKind,
        /// Transport delay before the target changes.
        delay: Option<Delay>,
    },
    /// Conditional execution.
    If {
        /// Single-bit condition.
        cond: ExprId,
        /// Run when the condition is 1.
        then_: Block,
        /// Run otherwise; empty when there is no `else`.
        else_: Block,
    },
    /// Multi-way branch on a value.
    Case {
        /// The value compared against each arm.
        subject: ExprId,
        /// How wildcards are treated.
        kind: CaseKind,
        /// `unique` / `priority`.
        qualifier: CaseQualifier,
        /// The arms, tested in order.
        arms: Vec<CaseArm>,
        /// Run when no arm matches.
        default: Option<Block>,
    },
    /// A counted loop; every part is optional, as in C.
    For {
        /// Assignment run once before the loop.
        init: Option<(Lvalue, ExprId)>,
        /// Loop continues while this is 1; absent means forever.
        cond: Option<ExprId>,
        /// Assignment run after each iteration.
        step: Option<(Lvalue, ExprId)>,
        /// The loop body.
        body: Block,
    },
    /// Loop while the condition holds.
    While {
        /// Single-bit condition, tested before each iteration.
        cond: ExprId,
        /// The loop body.
        body: Block,
    },
    /// Loop a computed number of times.
    Repeat {
        /// Iteration count.
        count: ExprId,
        /// The loop body.
        body: Block,
    },
    /// Loop until `break`, `finish` or the end of simulation.
    Forever {
        /// The loop body.
        body: Block,
    },
    /// A nested, optionally named block.
    Block {
        /// The label, used by `break`/`continue` diagnostics and waveform
        /// scopes.
        name: Option<Name>,
        /// The statements.
        body: Block,
    },
    /// Suspend the process.
    Wait(WaitKind),
    /// A system task or unlowered procedure call (`$display`, `$finish`
    /// variants with arguments, VHDL `report` without a condition).
    SysCall {
        /// The task name as written (`$display`).
        name: Name,
        /// Arguments; string literals are [`super::ExprKind::String`].
        args: Vec<ExprId>,
    },
    /// Load a whole memory from a file, or save it to one: `$readmemh`,
    /// `$readmemb`, `$writememh`, `$writememb`.
    ///
    /// The memory is named by id, as [`StmtKind::MemWrite`] names it,
    /// because the task works on the memory itself rather than on a word
    /// of it, and no expression denotes a whole memory. The file format
    /// and the address rules are in [`super::memfile`].
    MemFile {
        /// Which task.
        op: MemFileOp,
        /// The memory loaded or saved.
        mem: MemoryId,
        /// The file name: a string, or a bit vector holding one.
        file: ExprId,
        /// The first address, in the IR's zero-based element numbering;
        /// absent means the lowest.
        start: Option<ExprId>,
        /// The last address, in the same numbering; absent means the
        /// highest. Only present with `start`.
        end: Option<ExprId>,
        /// The address that `@hex` lines in the file give element 0: the
        /// declared lowest index of the memory, so 0 unless it was
        /// declared with another bound (`reg [7:0] m [16:31]` has 16).
        base: i64,
    },
    /// Write one memory element, optionally gated.
    MemWrite {
        /// The memory.
        mem: MemoryId,
        /// Element address.
        addr: ExprId,
        /// Value written.
        value: ExprId,
        /// Single-bit write enable; absent means always.
        enable: Option<ExprId>,
    },
    /// Check a condition, reporting when it fails.
    Assert {
        /// Single-bit condition that must hold.
        cond: ExprId,
        /// How serious a failure is.
        severity: ReportSeverity,
        /// Report arguments, formatted like a `SysCall`'s.
        message: Vec<ExprId>,
    },
    /// End the simulation (`$finish`).
    Finish,
    /// Pause the simulation (`$stop`).
    Stop,
    /// Leave the innermost loop.
    Break,
    /// Skip to the next iteration of the innermost loop.
    Continue,
}

/// One statement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stmt {
    /// What the statement does.
    pub kind: StmtKind,
    /// Where it came from.
    pub span: Span,
}

impl Stmt {
    /// Builds a statement.
    pub fn new(kind: StmtKind, span: Span) -> Self {
        Stmt { kind, span }
    }

    /// The blocks nested directly inside this statement, in a fixed order.
    pub fn blocks(&self) -> Vec<&Block> {
        match &self.kind {
            StmtKind::If { then_, else_, .. } => vec![then_, else_],
            StmtKind::Case { arms, default, .. } => {
                let mut blocks: Vec<&Block> = arms.iter().map(|a| &a.body).collect();
                if let Some(d) = default {
                    blocks.push(d);
                }
                blocks
            }
            StmtKind::For { body, .. }
            | StmtKind::While { body, .. }
            | StmtKind::Repeat { body, .. }
            | StmtKind::Forever { body }
            | StmtKind::Block { body, .. } => vec![body],
            _ => Vec::new(),
        }
    }

    /// Mutable access to the blocks nested directly inside this statement.
    pub fn blocks_mut(&mut self) -> Vec<&mut Block> {
        match &mut self.kind {
            StmtKind::If { then_, else_, .. } => vec![then_, else_],
            StmtKind::Case { arms, default, .. } => {
                let mut blocks: Vec<&mut Block> = arms.iter_mut().map(|a| &mut a.body).collect();
                if let Some(d) = default {
                    blocks.push(d);
                }
                blocks
            }
            StmtKind::For { body, .. }
            | StmtKind::While { body, .. }
            | StmtKind::Repeat { body, .. }
            | StmtKind::Forever { body }
            | StmtKind::Block { body, .. } => vec![body],
            _ => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_units() {
        for unit in TimeUnit::ALL {
            assert_eq!(TimeUnit::from_name(unit.name()), Some(unit));
        }
        assert_eq!(TimeUnit::from_name("min"), None);
        assert_eq!(Delay::new(3, TimeUnit::Ns).to_fs(), 3_000_000);
        assert_eq!(Delay::new(u64::MAX, TimeUnit::S).to_fs(), u64::MAX);
        assert!(TimeUnit::Fs < TimeUnit::S);
    }

    #[test]
    fn keywords() {
        assert_eq!(ProcessKind::Comb.keyword(), "comb");
        assert_eq!(ProcessKind::posedge(NetId(0)).keyword(), "seq");
        assert_eq!(CaseKind::Z.keyword(), "casez");
        for sev in [
            ReportSeverity::Note,
            ReportSeverity::Warning,
            ReportSeverity::Error,
            ReportSeverity::Failure,
        ] {
            assert_eq!(ReportSeverity::from_keyword(sev.keyword()), Some(sev));
        }
        assert_eq!(ReportSeverity::from_keyword("fatal"), None);
    }

    #[test]
    fn lvalue_nets() {
        let lv = Lvalue::Concat(vec![
            Lvalue::Net(NetId(1)),
            Lvalue::Slice {
                net: NetId(2),
                hi: 3,
                lo: 0,
            },
            Lvalue::MemElem {
                mem: MemoryId(0),
                addr: ExprId(0),
            },
        ]);
        assert_eq!(lv.nets(), [NetId(1), NetId(2)]);
    }
}
