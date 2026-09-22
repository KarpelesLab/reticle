//! Ergonomic construction of modules from Rust.
//!
//! [`ModuleBuilder`] wraps a [`Module`] under construction: it creates
//! nets and ports, builds typed expressions with one call per node
//! (`b.and(x, y)`, `b.const_u64(8, 3)`), records continuous assignments,
//! instances, cells and memories, and hands out [`ProcessBuilder`]s for
//! statement bodies. Every object takes the builder's current [`Span`]
//! (`b.span`), which a frontend updates as it lowers each source construct.
//!
//! Expression helpers compute the node's cached type with the rules in
//! [`super::expr`]. When the operands do not satisfy a rule the node is
//! still created, typed after its first operand, so a frontend can keep
//! going and let [`super::validate`] report the problem with a span.
//!
//! ```
//! use reticle::ir::builder::ModuleBuilder;
//! use reticle::ir::{AssignKind, ProcessKind, Type};
//! use reticle::source::{SourceMap, Span};
//!
//! let mut map = SourceMap::new();
//! let file = map.add("counter.v", "").unwrap();
//! let mut b = ModuleBuilder::new("counter", Span::new(file, 0, 0));
//! let clk = b.input("clk", Type::bit());
//! let q = b.output("q", Type::bits(8));
//! let (qv, one) = (b.net(q), b.const_u64(8, 1));
//! let next = b.add(qv, one);
//! let mut p = b.process(None, ProcessKind::posedge(clk));
//! p.assign(q, next, AssignKind::NonBlocking);
//! b.end_process(p);
//! let module = b.finish();
//! assert_eq!(module.processes.len(), 1);
//! ```

use std::ops::{Deref, DerefMut};

use super::Name;
use super::attr::{AttrValue, Attrs};
use super::cell::{Cell, CellId, CellKind};
use super::design::{
    Assign, Instance, InstanceId, Memory, MemoryId, Module, ModuleRef, Net, NetId, NetKind, Param,
    Port, PortDir,
};
use super::expr::{BinaryOp, Expr, ExprId, ExprKind, UnaryOp, infer_type};
use super::process::{
    AssignKind, Block, CaseArm, CaseKind, CaseQualifier, Delay, Edge, Lvalue, Process, ProcessId,
    ProcessKind, ReportSeverity, Stmt, StmtKind, WaitKind,
};
use super::types::{Const, Type};
use crate::source::Span;

impl From<NetId> for Lvalue {
    fn from(net: NetId) -> Self {
        Lvalue::Net(net)
    }
}

/// Builds a [`Module`] incrementally.
pub struct ModuleBuilder {
    module: Module,
    /// The span given to every object created next. Frontends set it as
    /// they walk their AST.
    pub span: Span,
}

impl ModuleBuilder {
    /// Starts a module named `name`; `span` is both the module's span and
    /// the initial current span.
    pub fn new(name: impl Into<Name>, span: Span) -> Self {
        ModuleBuilder {
            module: Module::new(name, span),
            span,
        }
    }

    /// Wraps an existing module so more can be added to it.
    pub fn from_module(module: Module, span: Span) -> Self {
        ModuleBuilder { module, span }
    }

    /// Returns the finished module.
    pub fn finish(self) -> Module {
        self.module
    }

    /// The module under construction.
    pub fn module(&self) -> &Module {
        &self.module
    }

    /// Mutable access to the module under construction, for what the
    /// builder does not cover.
    pub fn module_mut(&mut self) -> &mut Module {
        &mut self.module
    }

    /// Marks the module as a black box.
    pub fn blackbox(&mut self) {
        self.module.blackbox = true;
    }

    /// Sets a module attribute.
    pub fn attr(&mut self, key: impl Into<Name>, value: impl Into<AttrValue>) {
        self.module.attrs.set(key, value);
    }

    /// Records a resolved parameter.
    pub fn param(&mut self, name: impl Into<Name>, value: impl Into<AttrValue>) {
        self.module.params.push(Param {
            name: name.into(),
            value: value.into(),
            attrs: Attrs::new(),
            span: self.span,
        });
    }

    // --- nets and ports ---------------------------------------------------

    /// Adds a wire.
    pub fn add_net(&mut self, name: impl Into<Name>, ty: Type) -> NetId {
        self.add_net_kind(name, ty, NetKind::Wire)
    }

    /// Adds a register (a net written by processes).
    pub fn add_reg(&mut self, name: impl Into<Name>, ty: Type) -> NetId {
        self.add_net_kind(name, ty, NetKind::Reg)
    }

    /// Adds a net of the given kind.
    pub fn add_net_kind(&mut self, name: impl Into<Name>, ty: Type, kind: NetKind) -> NetId {
        self.module.nets.push(Net {
            name: name.into(),
            ty,
            kind,
            attrs: Attrs::new(),
            span: self.span,
        })
    }

    /// Exposes an existing net as a port.
    pub fn add_port(&mut self, name: impl Into<Name>, dir: PortDir, net: NetId) {
        self.module.ports.push(Port {
            name: name.into(),
            dir,
            net,
            span: self.span,
        });
    }

    /// Adds a wire and exposes it as an input port of the same name.
    pub fn input(&mut self, name: impl Into<Name>, ty: Type) -> NetId {
        self.port_net(name, ty, PortDir::In, NetKind::Wire)
    }

    /// Adds a wire and exposes it as an output port of the same name.
    pub fn output(&mut self, name: impl Into<Name>, ty: Type) -> NetId {
        self.port_net(name, ty, PortDir::Out, NetKind::Wire)
    }

    /// Adds a register and exposes it as an output port of the same name.
    pub fn output_reg(&mut self, name: impl Into<Name>, ty: Type) -> NetId {
        self.port_net(name, ty, PortDir::Out, NetKind::Reg)
    }

    /// Adds a wire and exposes it as an inout port of the same name.
    pub fn inout(&mut self, name: impl Into<Name>, ty: Type) -> NetId {
        self.port_net(name, ty, PortDir::InOut, NetKind::Wire)
    }

    fn port_net(&mut self, name: impl Into<Name>, ty: Type, dir: PortDir, kind: NetKind) -> NetId {
        let name = name.into();
        let net = self.add_net_kind(name.clone(), ty, kind);
        self.add_port(name, dir, net);
        net
    }

    /// Sets an attribute on a net.
    pub fn net_attr(&mut self, net: NetId, key: impl Into<Name>, value: impl Into<AttrValue>) {
        self.module.nets[net].attrs.set(key, value);
    }

    /// Adds a memory of `size` elements of type `elem`.
    pub fn memory(&mut self, name: impl Into<Name>, elem: Type, size: u64) -> MemoryId {
        self.module.memories.push(Memory {
            name: name.into(),
            elem,
            size,
            init: None,
            attrs: Attrs::new(),
            span: self.span,
        })
    }

    // --- expressions --------------------------------------------------------

    /// Adds an expression node, inferring its type from the operands.
    pub fn expr(&mut self, kind: ExprKind) -> ExprId {
        let ty = match infer_type(&self.module, &kind) {
            Ok(ty) => ty,
            Err(_) => self.fallback_type(&kind),
        };
        self.module.add_expr(Expr::new(kind, ty, self.span))
    }

    /// Adds an expression node with an explicit type, for `Call` and for
    /// tests of the validator.
    pub fn expr_typed(&mut self, kind: ExprKind, ty: Type) -> ExprId {
        self.module.add_expr(Expr::new(kind, ty, self.span))
    }

    /// The type given to an ill-typed node: that of its first operand, or
    /// one bit when it has none.
    fn fallback_type(&self, kind: &ExprKind) -> Type {
        super::expr::operands(kind)
            .first()
            .and_then(|id| self.module.exprs.get(*id))
            .map_or(Type::bit(), |e| e.ty.clone())
    }

    /// A constant.
    pub fn constant(&mut self, value: Const) -> ExprId {
        self.expr(ExprKind::Const(value))
    }

    /// An unsigned constant of `width` bits holding `value`.
    pub fn const_u64(&mut self, width: u32, value: u64) -> ExprId {
        self.constant(Const::from_u64(value, width))
    }

    /// A signed constant of `width` bits holding `value`.
    pub fn const_i64(&mut self, width: u32, value: i64) -> ExprId {
        self.constant(Const::from_i64(value, width))
    }

    /// A single-bit constant.
    pub fn const_bit(&mut self, value: bool) -> ExprId {
        self.const_u64(1, u64::from(value))
    }

    /// A string literal.
    pub fn string(&mut self, value: impl Into<String>) -> ExprId {
        self.expr(ExprKind::String(value.into()))
    }

    /// The value of a net.
    pub fn net(&mut self, net: NetId) -> ExprId {
        self.expr(ExprKind::Net(net))
    }

    /// `base[hi:lo]`.
    pub fn slice(&mut self, base: ExprId, hi: u32, lo: u32) -> ExprId {
        self.expr(ExprKind::Slice { base, hi, lo })
    }

    /// `base[index]`.
    pub fn index(&mut self, base: ExprId, index: ExprId) -> ExprId {
        self.expr(ExprKind::Index { base, index })
    }

    /// `base[offset +: width]` when `up`, else `base[offset -: width]`.
    pub fn indexed_slice(&mut self, base: ExprId, offset: ExprId, width: u32, up: bool) -> ExprId {
        self.expr(ExprKind::IndexedSlice {
            base,
            offset,
            width,
            up,
        })
    }

    /// `{parts...}`, first part most significant.
    pub fn concat(&mut self, parts: Vec<ExprId>) -> ExprId {
        self.expr(ExprKind::Concat(parts))
    }

    /// `{count{expr}}`.
    pub fn replicate(&mut self, count: u32, expr: ExprId) -> ExprId {
        self.expr(ExprKind::Replicate { count, expr })
    }

    /// A unary operator.
    pub fn unary(&mut self, op: UnaryOp, expr: ExprId) -> ExprId {
        self.expr(ExprKind::Unary { op, expr })
    }

    /// A binary operator.
    pub fn binary(&mut self, op: BinaryOp, lhs: ExprId, rhs: ExprId) -> ExprId {
        self.expr(ExprKind::Binary { op, lhs, rhs })
    }

    /// `cond ? then_ : else_`.
    pub fn mux(&mut self, cond: ExprId, then_: ExprId, else_: ExprId) -> ExprId {
        self.expr(ExprKind::Ternary { cond, then_, else_ })
    }

    /// Resizes to `width`, sign extending when `signed`.
    pub fn resize(&mut self, expr: ExprId, width: u32, signed: bool) -> ExprId {
        self.expr(ExprKind::Resize {
            expr,
            width,
            signed,
        })
    }

    /// Zero-extends or truncates to `width`.
    pub fn zext(&mut self, expr: ExprId, width: u32) -> ExprId {
        self.resize(expr, width, false)
    }

    /// Sign-extends or truncates to `width`.
    pub fn sext(&mut self, expr: ExprId, width: u32) -> ExprId {
        self.resize(expr, width, true)
    }

    /// `mem[addr]`.
    pub fn mem_read(&mut self, mem: MemoryId, addr: ExprId) -> ExprId {
        self.expr(ExprKind::MemRead { mem, addr })
    }

    /// An unlowered call returning `ty`.
    pub fn call(&mut self, name: impl Into<Name>, args: Vec<ExprId>, ty: Type) -> ExprId {
        self.expr_typed(
            ExprKind::Call {
                name: name.into(),
                args,
            },
            ty,
        )
    }

    /// Bitwise complement.
    pub fn not(&mut self, e: ExprId) -> ExprId {
        self.unary(UnaryOp::Not, e)
    }

    /// Negation.
    pub fn neg(&mut self, e: ExprId) -> ExprId {
        self.unary(UnaryOp::Neg, e)
    }

    /// Logical negation.
    pub fn lnot(&mut self, e: ExprId) -> ExprId {
        self.unary(UnaryOp::LogicNot, e)
    }

    /// AND reduction.
    pub fn reduce_and(&mut self, e: ExprId) -> ExprId {
        self.unary(UnaryOp::ReduceAnd, e)
    }

    /// OR reduction.
    pub fn reduce_or(&mut self, e: ExprId) -> ExprId {
        self.unary(UnaryOp::ReduceOr, e)
    }

    /// XOR reduction.
    pub fn reduce_xor(&mut self, e: ExprId) -> ExprId {
        self.unary(UnaryOp::ReduceXor, e)
    }

    /// Bitwise AND.
    pub fn and(&mut self, a: ExprId, b: ExprId) -> ExprId {
        self.binary(BinaryOp::And, a, b)
    }

    /// Bitwise OR.
    pub fn or(&mut self, a: ExprId, b: ExprId) -> ExprId {
        self.binary(BinaryOp::Or, a, b)
    }

    /// Bitwise XOR.
    pub fn xor(&mut self, a: ExprId, b: ExprId) -> ExprId {
        self.binary(BinaryOp::Xor, a, b)
    }

    /// Logical AND.
    pub fn land(&mut self, a: ExprId, b: ExprId) -> ExprId {
        self.binary(BinaryOp::LogicAnd, a, b)
    }

    /// Logical OR.
    pub fn lor(&mut self, a: ExprId, b: ExprId) -> ExprId {
        self.binary(BinaryOp::LogicOr, a, b)
    }

    /// Addition.
    pub fn add(&mut self, a: ExprId, b: ExprId) -> ExprId {
        self.binary(BinaryOp::Add, a, b)
    }

    /// Subtraction.
    pub fn sub(&mut self, a: ExprId, b: ExprId) -> ExprId {
        self.binary(BinaryOp::Sub, a, b)
    }

    /// Multiplication.
    pub fn mul(&mut self, a: ExprId, b: ExprId) -> ExprId {
        self.binary(BinaryOp::Mul, a, b)
    }

    /// Shift left.
    pub fn shl(&mut self, a: ExprId, b: ExprId) -> ExprId {
        self.binary(BinaryOp::Shl, a, b)
    }

    /// Logical shift right.
    pub fn shr(&mut self, a: ExprId, b: ExprId) -> ExprId {
        self.binary(BinaryOp::Shr, a, b)
    }

    /// Equality.
    pub fn eq(&mut self, a: ExprId, b: ExprId) -> ExprId {
        self.binary(BinaryOp::Eq, a, b)
    }

    /// Inequality.
    pub fn ne(&mut self, a: ExprId, b: ExprId) -> ExprId {
        self.binary(BinaryOp::Ne, a, b)
    }

    /// Less than.
    pub fn lt(&mut self, a: ExprId, b: ExprId) -> ExprId {
        self.binary(BinaryOp::Lt, a, b)
    }

    /// Less than or equal.
    pub fn le(&mut self, a: ExprId, b: ExprId) -> ExprId {
        self.binary(BinaryOp::Le, a, b)
    }

    /// Greater than.
    pub fn gt(&mut self, a: ExprId, b: ExprId) -> ExprId {
        self.binary(BinaryOp::Gt, a, b)
    }

    /// Greater than or equal.
    pub fn ge(&mut self, a: ExprId, b: ExprId) -> ExprId {
        self.binary(BinaryOp::Ge, a, b)
    }

    // --- drivers and structure ---------------------------------------------

    /// Adds a continuous assignment.
    pub fn assign(&mut self, target: impl Into<Lvalue>, value: ExprId) {
        self.assign_after(target, value, None);
    }

    /// Adds a continuous assignment with an optional transport delay.
    pub fn assign_after(&mut self, target: impl Into<Lvalue>, value: ExprId, delay: Option<Delay>) {
        self.module.assigns.push(Assign {
            target: target.into(),
            value,
            delay,
            attrs: Attrs::new(),
            span: self.span,
        });
    }

    /// Adds an instance of `module` with the given port connections.
    pub fn instance(
        &mut self,
        name: impl Into<Name>,
        module: ModuleRef,
        connections: Vec<(Name, ExprId)>,
    ) -> InstanceId {
        self.module.instances.push(Instance {
            name: name.into(),
            module,
            connections,
            params: Attrs::new(),
            attrs: Attrs::new(),
            span: self.span,
        })
    }

    /// Adds a cell.
    pub fn cell(
        &mut self,
        name: impl Into<Name>,
        kind: CellKind,
        inputs: Vec<(Name, ExprId)>,
        outputs: Vec<(Name, NetId)>,
    ) -> CellId {
        self.module.cells.push(Cell {
            name: name.into(),
            kind,
            inputs,
            outputs,
            params: Attrs::new(),
            attrs: Attrs::new(),
            span: self.span,
        })
    }

    /// Adds a two-input cell `kind(a, b) -> y`, the common case.
    pub fn cell2(
        &mut self,
        name: impl Into<Name>,
        kind: CellKind,
        a: ExprId,
        b: ExprId,
        y: NetId,
    ) -> CellId {
        self.cell(
            name,
            kind,
            vec![(Name::new("a"), a), (Name::new("b"), b)],
            vec![(Name::new("y"), y)],
        )
    }

    /// Starts a process; finish it with [`ModuleBuilder::end_process`].
    pub fn process(&mut self, name: Option<&str>, kind: ProcessKind) -> ProcessBuilder {
        ProcessBuilder {
            name: name.map(Name::new),
            kind,
            attrs: Attrs::new(),
            body: BlockBuilder::new(self.span),
            span: self.span,
        }
    }

    /// Adds a finished process to the module.
    pub fn end_process(&mut self, process: ProcessBuilder) -> ProcessId {
        self.module.processes.push(Process {
            name: process.name,
            kind: process.kind,
            body: process.body.finish(),
            attrs: process.attrs,
            span: process.span,
        })
    }

    /// A statement block builder using the current span, for nested blocks.
    pub fn block(&self) -> BlockBuilder {
        BlockBuilder::new(self.span)
    }
}

/// Builds a [`Process`]; derefs to its body's [`BlockBuilder`].
pub struct ProcessBuilder {
    name: Option<Name>,
    kind: ProcessKind,
    /// Attributes of the process.
    pub attrs: Attrs,
    body: BlockBuilder,
    span: Span,
}

impl Deref for ProcessBuilder {
    type Target = BlockBuilder;

    fn deref(&self) -> &BlockBuilder {
        &self.body
    }
}

impl DerefMut for ProcessBuilder {
    fn deref_mut(&mut self) -> &mut BlockBuilder {
        &mut self.body
    }
}

/// Builds a [`Block`] of statements.
///
/// Expressions are created on the [`ModuleBuilder`] beforehand; the block
/// builder only records statements, so the two never borrow each other.
pub struct BlockBuilder {
    stmts: Block,
    /// The span given to every statement created next.
    pub span: Span,
}

impl BlockBuilder {
    /// Starts an empty block.
    pub fn new(span: Span) -> Self {
        BlockBuilder {
            stmts: Vec::new(),
            span,
        }
    }

    /// Returns the statements.
    pub fn finish(self) -> Block {
        self.stmts
    }

    /// Appends a statement kind with the current span.
    pub fn push(&mut self, kind: StmtKind) {
        self.stmts.push(Stmt::new(kind, self.span));
    }

    /// `target = value` or `target <= value`.
    pub fn assign(&mut self, target: impl Into<Lvalue>, value: ExprId, kind: AssignKind) {
        self.push(StmtKind::Assign {
            target: target.into(),
            value,
            kind,
            delay: None,
        });
    }

    /// A blocking assignment.
    pub fn blocking(&mut self, target: impl Into<Lvalue>, value: ExprId) {
        self.assign(target, value, AssignKind::Blocking);
    }

    /// A non-blocking assignment.
    pub fn nonblocking(&mut self, target: impl Into<Lvalue>, value: ExprId) {
        self.assign(target, value, AssignKind::NonBlocking);
    }

    /// An assignment with a transport delay.
    pub fn assign_after(
        &mut self,
        target: impl Into<Lvalue>,
        value: ExprId,
        kind: AssignKind,
        delay: Delay,
    ) {
        self.push(StmtKind::Assign {
            target: target.into(),
            value,
            kind,
            delay: Some(delay),
        });
    }

    /// `if cond { then_ } else { else_ }`.
    pub fn if_(&mut self, cond: ExprId, then_: Block, else_: Block) {
        self.push(StmtKind::If { cond, then_, else_ });
    }

    /// A `case` statement.
    pub fn case(
        &mut self,
        subject: ExprId,
        kind: CaseKind,
        qualifier: CaseQualifier,
        arms: Vec<CaseArm>,
        default: Option<Block>,
    ) {
        self.push(StmtKind::Case {
            subject,
            kind,
            qualifier,
            arms,
            default,
        });
    }

    /// A `for` loop.
    pub fn for_(
        &mut self,
        init: Option<(Lvalue, ExprId)>,
        cond: Option<ExprId>,
        step: Option<(Lvalue, ExprId)>,
        body: Block,
    ) {
        self.push(StmtKind::For {
            init,
            cond,
            step,
            body,
        });
    }

    /// A `while` loop.
    pub fn while_(&mut self, cond: ExprId, body: Block) {
        self.push(StmtKind::While { cond, body });
    }

    /// A `repeat` loop.
    pub fn repeat(&mut self, count: ExprId, body: Block) {
        self.push(StmtKind::Repeat { count, body });
    }

    /// A `forever` loop.
    pub fn forever(&mut self, body: Block) {
        self.push(StmtKind::Forever { body });
    }

    /// A nested block.
    pub fn nested(&mut self, name: Option<&str>, body: Block) {
        self.push(StmtKind::Block {
            name: name.map(Name::new),
            body,
        });
    }

    /// Wait for a delay.
    pub fn wait_delay(&mut self, delay: ExprId) {
        self.push(StmtKind::Wait(WaitKind::Delay(delay)));
    }

    /// Wait for one of the edges.
    pub fn wait_event(&mut self, edges: Vec<Edge>) {
        self.push(StmtKind::Wait(WaitKind::Event(edges)));
    }

    /// Wait until the condition holds.
    pub fn wait_until(&mut self, cond: ExprId) {
        self.push(StmtKind::Wait(WaitKind::Until(cond)));
    }

    /// A system task call.
    pub fn syscall(&mut self, name: impl Into<Name>, args: Vec<ExprId>) {
        self.push(StmtKind::SysCall {
            name: name.into(),
            args,
        });
    }

    /// A memory write.
    pub fn mem_write(
        &mut self,
        mem: MemoryId,
        addr: ExprId,
        value: ExprId,
        enable: Option<ExprId>,
    ) {
        self.push(StmtKind::MemWrite {
            mem,
            addr,
            value,
            enable,
        });
    }

    /// An assertion.
    pub fn assert(&mut self, cond: ExprId, severity: ReportSeverity, message: Vec<ExprId>) {
        self.push(StmtKind::Assert {
            cond,
            severity,
            message,
        });
    }

    /// `$finish`.
    pub fn finish_sim(&mut self) {
        self.push(StmtKind::Finish);
    }

    /// `$stop`.
    pub fn stop(&mut self) {
        self.push(StmtKind::Stop);
    }

    /// `break`.
    pub fn break_(&mut self) {
        self.push(StmtKind::Break);
    }

    /// `continue`.
    pub fn continue_(&mut self) {
        self.push(StmtKind::Continue);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::types::Bit;
    use crate::source::SourceMap;

    fn span() -> Span {
        let mut map = SourceMap::new();
        let id = map.add("t", "").unwrap();
        Span::new(id, 0, 0)
    }

    #[test]
    fn builds_typed_expressions() {
        let mut b = ModuleBuilder::new("m", span());
        let a = b.input("a", Type::bits(8));
        let s = b.input("s", Type::sbits(8));
        let an = b.net(a);
        let sn = b.net(s);
        let m = b.module();
        assert_eq!(m.expr(an).ty, Type::bits(8));
        let sum = b.add(an, sn);
        assert_eq!(b.module().expr(sum).ty, Type::bits(8));
        let neg = b.neg(sn);
        assert_eq!(b.module().expr(neg).ty, Type::sbits(8));
        let cmp = b.lt(an, sn);
        assert!(b.module().expr(cmp).ty.is_bit());
        let sl = b.slice(an, 3, 0);
        assert_eq!(b.module().expr(sl).ty, Type::bits(4));
        let cat = b.concat(vec![sl, an]);
        assert_eq!(b.module().expr(cat).ty, Type::bits(12));
        let rep = b.replicate(3, sl);
        assert_eq!(b.module().expr(rep).ty, Type::bits(12));
        let ext = b.sext(sl, 16);
        assert_eq!(b.module().expr(ext).ty, Type::sbits(16));
        let z = b.zext(sl, 16);
        assert_eq!(b.module().expr(z).ty, Type::bits(16));
        let idx = b.index(an, sl);
        assert!(b.module().expr(idx).ty.is_bit());
        let is = b.indexed_slice(an, sl, 2, true);
        assert_eq!(b.module().expr(is).ty, Type::bits(2));
        let red = b.reduce_or(an);
        let mux = b.mux(red, an, sn);
        assert_eq!(b.module().expr(mux).ty, Type::bits(8));
        let call = b.call("$clog2", vec![an], Type::Integer);
        assert_eq!(b.module().expr(call).ty, Type::Integer);
        let st = b.string("hi");
        assert_eq!(b.module().expr(st).ty, Type::String);
        let c = b.const_i64(4, -1);
        assert_eq!(
            b.module().expr(c).as_const().unwrap(),
            &Const::from_i64(-1, 4)
        );
        let bit = b.const_bit(true);
        assert_eq!(b.module().expr(bit).as_const().unwrap().bits(), [Bit::One]);
        // Ill-typed nodes fall back to the first operand's type.
        let bad = b.and(an, sl);
        assert_eq!(b.module().expr(bad).ty, Type::bits(8));
        let bad_leaf = b.expr(ExprKind::Concat(vec![]));
        assert_eq!(b.module().expr(bad_leaf).ty, Type::bits(0));
        for op in UnaryOp::ALL {
            b.unary(op, an);
        }
        for op in BinaryOp::ALL {
            b.binary(op, an, an);
        }
        let x = b.not(an);
        let y = b.lnot(x);
        let _ = (b.reduce_and(y), b.reduce_xor(y));
        let _ = (b.or(an, an), b.xor(an, an), b.land(y, y), b.lor(y, y));
        let _ = (b.sub(an, an), b.mul(an, an), b.shl(an, sl), b.shr(an, sl));
        let _ = (
            b.eq(an, an),
            b.ne(an, an),
            b.le(an, an),
            b.gt(an, an),
            b.ge(an, an),
        );
    }

    #[test]
    fn builds_structure() {
        let span = span();
        let mut b = ModuleBuilder::new("m", span);
        b.attr("keep_hierarchy", 1);
        b.param("WIDTH", 8);
        let clk = b.input("clk", Type::bit());
        let d = b.input("d", Type::bits(8));
        let q = b.output_reg("q", Type::bits(8));
        let io = b.inout("io", Type::bit());
        b.net_attr(q, "keep", 1);
        let mem = b.memory("mem", Type::bits(8), 16);
        let t = b.add_reg("t", Type::bits(8));
        let dn = b.net(d);
        let zero = b.const_u64(4, 0);
        let rd = b.mem_read(mem, zero);
        b.assign(t, rd);
        b.assign_after(
            io,
            dn,
            Some(Delay::new(1, super::super::process::TimeUnit::Ns)),
        );
        let cond = b.reduce_or(dn);
        let mut p = b.process(Some("p"), ProcessKind::posedge(clk));
        let mut then = b.block();
        then.nonblocking(q, dn);
        then.mem_write(mem, zero, dn, Some(cond));
        let mut els = b.block();
        els.assign_after(
            q,
            rd,
            AssignKind::NonBlocking,
            Delay::new(2, super::super::process::TimeUnit::Ps),
        );
        p.if_(cond, then.finish(), els.finish());
        p.attrs.set("full_case", 1);
        b.end_process(p);
        let mut init = b.process(None, ProcessKind::Initial);
        let msg = b.string("start");
        init.syscall("$display", vec![msg]);
        init.assert(cond, ReportSeverity::Warning, vec![msg]);
        init.wait_delay(zero);
        init.wait_event(vec![Edge::pos(clk)]);
        init.wait_until(cond);
        init.case(
            dn,
            CaseKind::Z,
            CaseQualifier::Unique,
            vec![CaseArm {
                values: vec![dn],
                body: vec![],
            }],
            Some(vec![]),
        );
        init.for_(Some((Lvalue::Net(t), dn)), Some(cond), None, vec![]);
        init.while_(cond, vec![]);
        init.repeat(dn, vec![]);
        init.forever(vec![]);
        init.nested(Some("blk"), vec![]);
        init.blocking(t, dn);
        init.break_();
        init.continue_();
        init.stop();
        init.finish_sim();
        b.end_process(init);
        b.cell2("c0", CellKind::And, dn, dn, t);
        b.instance(
            "u0",
            ModuleRef::Unresolved(Name::new("leaf")),
            vec![(Name::new("a"), dn)],
        );
        b.blackbox();
        let m = b.finish();
        assert!(m.blackbox);
        assert_eq!(m.ports.len(), 4);
        assert_eq!(m.processes.len(), 2);
        assert_eq!(m.processes.values().nth(1).unwrap().body.len(), 16);
        assert_eq!(m.cells.len(), 1);
        assert_eq!(m.instances.len(), 1);
        assert_eq!(m.assigns.len(), 2);
        assert!(m.attrs.is_set("keep_hierarchy"));
        assert_eq!(m.param("WIDTH").and_then(|p| p.value.as_int()), Some(8));
        let b2 = ModuleBuilder::from_module(m, span);
        assert_eq!(b2.module().name, "m");
        let mut b2 = b2;
        b2.module_mut().blackbox = false;
        assert!(!b2.finish().blackbox);
    }

    /// The builder and the text format agree on the golden counter.
    #[test]
    fn builds_the_golden_counter() {
        use super::super::design::Design;
        use super::super::process::{TimeUnit, Timescale};

        let mut b = ModuleBuilder::new("counter", span());
        b.module_mut().timescale = Some(Timescale {
            unit: Delay::new(1, TimeUnit::Ns),
            precision: Delay::new(1, TimeUnit::Ps),
        });
        b.param("WIDTH", 8);
        let clk = b.input("clk", Type::bit());
        let rst = b.input("rst", Type::bit());
        let en = b.input("en", Type::bit());
        let q = b.output_reg("q", Type::bits(8));
        b.net_attr(q, "keep", 1);
        let (rstv, env, qv) = (b.net(rst), b.net(en), b.net(q));
        let zero = b.const_u64(8, 0);
        let one = b.const_u64(8, 1);
        let next = b.add(qv, one);
        let mut p = b.process(Some("count"), ProcessKind::posedge(clk));
        let mut reset = b.block();
        reset.nonblocking(q, zero);
        let mut count = b.block();
        count.nonblocking(q, next);
        let mut run = b.block();
        run.if_(env, count.finish(), Vec::new());
        p.if_(rstv, reset.finish(), run.finish());
        b.end_process(p);
        let mut design = Design::new();
        let id = design.add_module(b.finish());
        design.top = Some(id);
        assert_eq!(
            design.to_text(),
            include_str!("../../testdata/ir/counter.rtl")
        );
        assert!(crate::ir::validate::validate(&design).is_empty());
    }
}
