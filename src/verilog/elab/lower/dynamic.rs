//! Dynamic arrays and queues (IEEE 1800-2017 §7.5, §7.10).
//!
//! The IR's memories have a fixed size, so `int d []` and `int q [$]` are a
//! memory of [`CAPACITY`] elements and a two-state `int` variable holding
//! how many of them the array has. `new[n]`, `new[n](d)`, `delete`,
//! `push_back`, `push_front`, `insert`, `pop_front` and `pop_back` are
//! statements and loops over the memory that keep the size right; a read
//! of an element the array does not have gives the element type's default
//! (`x` for a four-state element, `0` for a two-state one), as the standard
//! has it, whatever the memory still holds there; `q[$]` is the last
//! element. Growing past the capacity stops the simulation with a failure
//! that says so, rather than dropping elements.
//!
//! A whole dynamic array is assigned from `new`, from another array, or
//! from a pattern or concatenation of elements and arrays, the array itself
//! among them (`q = {q, 7, 8}` appends; `r = {q, r}` reads a copy of `r`
//! made first); `foreach` runs over the elements it has, and `%p` prints
//! them, built at run time.

use crate::ir::builder::BlockBuilder;
use crate::ir::{self, AssignKind, ExprId, MemoryId, NetId, Type};
use crate::logic::Logic;
use crate::source::Span;
use crate::verilog::ast::{self, Expr, ExprKind};

use super::super::codes;
use super::super::types::{Packed, Range, VType};
use super::Lowerer;
use super::expr::Sink;

/// How many elements a dynamic array or a queue can hold.
pub(super) const CAPACITY: u64 = 4096;

/// The type of a dynamic array's size: `int`.
pub(super) fn size_type() -> VType {
    VType::Packed(Packed {
        four_state: false,
        keyword: Some("int"),
        ..Packed::sbits(32)
    })
}

impl<'cx, 'ast> Lowerer<'cx, 'ast> {
    /// The memory and size variable of the dynamic array `e` names whole.
    pub(super) fn dynamic_of(&mut self, e: &'ast Expr) -> Option<(MemoryId, NetId)> {
        let (mem, _) = self.whole_array(e)?;
        let size = self.mems[mem.index()].dynamic?;
        Some((mem, size))
    }

    /// The value an element the array does not have reads as.
    pub(super) fn element_default(&mut self, mem: MemoryId) -> ExprId {
        let elem = self.mems[mem.index()].elem.clone();
        match elem {
            VType::Real => self.real_const(0.0),
            VType::String => self.b.string(""),
            _ => {
                let w = elem.packed().map_or(1, Packed::width);
                let value = if super::is_two_state(&elem) {
                    Logic::zero(w)
                } else {
                    Logic::x(w)
                };
                self.b.constant(value)
            }
        }
    }

    /// `d[i]` of a dynamic array: the element, or the default where the
    /// array has none; `$` is the last element.
    pub(super) fn dynamic_read(
        &mut self,
        mem: MemoryId,
        size: NetId,
        index: &'ast Expr,
        span: Span,
        sink: &mut Sink<'_>,
    ) -> ExprId {
        let i = self.dynamic_index(size, index, sink);
        self.b.span = span;
        let read = self.b.mem_read(mem, i);
        let n = self.b.net(size);
        let zero = self.int(0);
        let below = self.b.binary(ir::BinaryOp::Lt, i, n);
        let above = self.b.binary(ir::BinaryOp::Ge, i, zero);
        let inside = self.b.land(below, above);
        let default = self.element_default(mem);
        self.b.mux(inside, read, default)
    }

    /// An index of a dynamic array as a signed 32-bit value; `$` is the
    /// last element's.
    pub(super) fn dynamic_index(
        &mut self,
        size: NetId,
        index: &'ast Expr,
        sink: &mut Sink<'_>,
    ) -> ExprId {
        if matches!(index.kind, ExprKind::Literal(ast::Literal::Unbounded(_))) {
            self.b.span = index.span;
            let n = self.b.net(size);
            let one = self.int(1);
            return self.b.sub(n, one);
        }
        let v = self.expr(
            index,
            Some(super::Ctx {
                width: 32,
                signed: true,
            }),
            sink,
        );
        self.coerce(v, 32, true)
    }

    /// A signed 32-bit constant.
    fn int(&mut self, v: i64) -> ExprId {
        self.b.constant(Logic::from_i64(v, 32).as_signed())
    }

    /// A loop variable of its own.
    fn loop_var(&mut self, span: Span) -> Option<NetId> {
        let name = self.fresh("dyn");
        let net = self.new_var(name, size_type(), span)?;
        self.record_driver(net, self.driver, true, span);
        Some(net)
    }

    /// Sets the size, stopping the simulation with a failure when it would
    /// pass the capacity.
    fn set_size(&mut self, size: NetId, value: ExprId, span: Span, out: &mut BlockBuilder) {
        self.b.span = span;
        let cap = self.int(i64::try_from(CAPACITY).unwrap_or(i64::MAX));
        let fits = self.b.binary(ir::BinaryOp::Le, value, cap);
        let message = self.b.string(format!(
            "a dynamic array or queue grew past {CAPACITY} elements, as many as Reticle holds"
        ));
        out.span = span;
        out.push(ir::StmtKind::Assert {
            cond: fits,
            severity: ir::ReportSeverity::Failure,
            message: vec![message],
        });
        out.assign(size, value, AssignKind::Blocking);
        self.record_driver(size, self.driver, true, span);
    }

    /// `for (i = from; i < to; i++) mem[i] = what(i)` where `what` gives
    /// the value of each element from the loop variable.
    fn fill(
        &mut self,
        mem: MemoryId,
        from: ExprId,
        to: ExprId,
        span: Span,
        out: &mut BlockBuilder,
        what: &mut dyn FnMut(&mut Self, ExprId) -> ExprId,
    ) {
        let Some(i) = self.loop_var(span) else { return };
        self.b.span = span;
        let iv = self.b.net(i);
        let value = what(self, iv);
        let mut body = self.b.block();
        body.span = span;
        body.assign(
            ir::Lvalue::MemElem { mem, addr: iv },
            value,
            AssignKind::Blocking,
        );
        let cond = self.b.binary(ir::BinaryOp::Lt, iv, to);
        let one = self.int(1);
        let next = self.b.add(iv, one);
        out.span = span;
        out.for_(
            Some((ir::Lvalue::Net(i), from)),
            Some(cond),
            Some((ir::Lvalue::Net(i), next)),
            body.finish(),
        );
    }

    /// Moves elements `from .. size` one place up (`up`) or `from + 1 ..
    /// size` one place down.
    fn shift(
        &mut self,
        mem: MemoryId,
        size: NetId,
        from: ExprId,
        up: bool,
        span: Span,
        out: &mut BlockBuilder,
    ) {
        let Some(i) = self.loop_var(span) else { return };
        self.b.span = span;
        let iv = self.b.net(i);
        let n = self.b.net(size);
        let one = self.int(1);
        let mut body = self.b.block();
        body.span = span;
        if up {
            // for (i = size; i > from; i--) m[i] = m[i - 1]
            let below = self.b.sub(iv, one);
            let read = self.b.mem_read(mem, below);
            body.assign(
                ir::Lvalue::MemElem { mem, addr: iv },
                read,
                AssignKind::Blocking,
            );
            let cond = self.b.binary(ir::BinaryOp::Gt, iv, from);
            let next = self.b.sub(iv, one);
            out.span = span;
            out.for_(
                Some((ir::Lvalue::Net(i), n)),
                Some(cond),
                Some((ir::Lvalue::Net(i), next)),
                body.finish(),
            );
        } else {
            // for (i = from; i < size - 1; i++) m[i] = m[i + 1]
            let above = self.b.add(iv, one);
            let read = self.b.mem_read(mem, above);
            body.assign(
                ir::Lvalue::MemElem { mem, addr: iv },
                read,
                AssignKind::Blocking,
            );
            let last = self.b.sub(n, one);
            let cond = self.b.binary(ir::BinaryOp::Lt, iv, last);
            let next = self.b.add(iv, one);
            out.span = span;
            out.for_(
                Some((ir::Lvalue::Net(i), from)),
                Some(cond),
                Some((ir::Lvalue::Net(i), next)),
                body.finish(),
            );
        }
    }

    /// An element value written to a dynamic array, as its element type.
    fn element_value(&mut self, mem: MemoryId, e: &'ast Expr, out: &mut BlockBuilder) -> ExprId {
        let elem = self.mems[mem.index()].elem.clone();
        self.value_as(e, &elem, &mut Sink::Proc(out))
    }

    /// Holds `value` in a variable of its own, so that statements after
    /// this one do not change what it reads.
    fn hold(
        &mut self,
        mem: MemoryId,
        value: ExprId,
        span: Span,
        out: &mut BlockBuilder,
    ) -> Option<ExprId> {
        let elem = self.mems[mem.index()].elem.clone();
        let name = self.fresh("dyn");
        let net = self.new_var(name, elem, span)?;
        out.span = span;
        out.assign(net, value, AssignKind::Blocking);
        self.record_driver(net, self.driver, true, span);
        self.b.span = span;
        Some(self.b.net(net))
    }

    /// `q.size()`, `q.pop_front()` and the like used as a value, or `None`
    /// when `callee` is not a method of a dynamic array.
    pub(super) fn dynamic_method(
        &mut self,
        e: &'ast Expr,
        callee: &'ast Expr,
        args: &'ast [ast::Arg],
        sink: &mut Sink<'_>,
    ) -> Option<ExprId> {
        let ExprKind::Member { base, name } = &callee.kind else {
            return None;
        };
        let (mem, size) = self.dynamic_of(base)?;
        let method = name.name.as_str();
        match method {
            "size" | "num" => {
                self.b.span = e.span;
                Some(self.b.net(size))
            }
            "pop_front" | "pop_back" => {
                let Sink::Proc(out) = sink else {
                    self.env
                        .unsupported(e.span, &format!("`{method}` in a continuous assignment"));
                    return Some(self.element_default(mem));
                };
                Some(self.pop(mem, size, method == "pop_front", e.span, out))
            }
            _ => {
                let _ = args;
                self.env.unsupported(
                    name.span,
                    &format!("the method `{method}` of a dynamic array or queue used as a value"),
                );
                Some(self.element_default(mem))
            }
        }
    }

    /// Removes the first or last element and gives its value; the default
    /// when the queue is empty.
    fn pop(
        &mut self,
        mem: MemoryId,
        size: NetId,
        front: bool,
        span: Span,
        out: &mut BlockBuilder,
    ) -> ExprId {
        self.b.span = span;
        let n = self.b.net(size);
        let zero = self.int(0);
        let one = self.int(1);
        let some = self.b.binary(ir::BinaryOp::Gt, n, zero);
        let at = if front { zero } else { self.b.sub(n, one) };
        let read = self.b.mem_read(mem, at);
        let default = self.element_default(mem);
        let value = self.b.mux(some, read, default);
        let Some(held) = self.hold(mem, value, span, out) else {
            return default;
        };
        if front {
            self.shift(mem, size, zero, false, span, out);
        }
        self.b.span = span;
        let n = self.b.net(size);
        let less = self.b.sub(n, one);
        let new = self.b.mux(some, less, zero);
        out.span = span;
        out.assign(size, new, AssignKind::Blocking);
        self.record_driver(size, self.driver, true, span);
        held
    }

    /// `q.push_back(v)`, `d.delete()` and the like as a statement. `None`
    /// when `callee` is not a method of a dynamic array.
    pub(super) fn dynamic_mutator(
        &mut self,
        callee: &'ast Expr,
        args: &'ast [ast::Arg],
        span: Span,
        out: &mut BlockBuilder,
    ) -> Option<()> {
        let ExprKind::Member { base, name } = &callee.kind else {
            return None;
        };
        let (mem, size) = self.dynamic_of(base)?;
        let values: Vec<&'ast Expr> = args.iter().filter_map(|a| a.value.as_ref()).collect();
        let method = name.name.as_str();
        let want = match method {
            "delete" => values.len().min(1),
            "push_back" | "push_front" => 1,
            "insert" => 2,
            "pop_front" | "pop_back" | "size" | "num" => 0,
            _ => {
                self.env.unsupported(
                    name.span,
                    &format!("the method `{method}` of a dynamic array or queue"),
                );
                return Some(());
            }
        };
        if values.len() != want {
            self.env.error(
                codes::ARGUMENTS,
                span,
                format!("`{method}` takes {want} argument(s), not {}", values.len()),
            );
            return Some(());
        }
        self.b.span = span;
        let one = self.int(1);
        match method {
            "delete" if values.is_empty() => {
                let zero = self.int(0);
                out.span = span;
                out.assign(size, zero, AssignKind::Blocking);
                self.record_driver(size, self.driver, true, span);
            }
            "delete" => {
                // `q.delete(i)`: the elements after it move down.
                let i = self.dynamic_index(size, values[0], &mut Sink::Proc(out));
                self.b.span = span;
                let n = self.b.net(size);
                let zero = self.int(0);
                let below = self.b.binary(ir::BinaryOp::Lt, i, n);
                let above = self.b.binary(ir::BinaryOp::Ge, i, zero);
                let inside = self.b.land(below, above);
                let mut then = self.b.block();
                then.span = span;
                self.shift(mem, size, i, false, span, &mut then);
                self.b.span = span;
                let n = self.b.net(size);
                let less = self.b.sub(n, one);
                then.assign(size, less, AssignKind::Blocking);
                self.record_driver(size, self.driver, true, span);
                out.span = span;
                out.if_(inside, then.finish(), self.b.block().finish());
            }
            "push_back" => {
                let v = self.element_value(mem, values[0], out);
                self.b.span = span;
                let n = self.b.net(size);
                out.span = span;
                out.assign(
                    ir::Lvalue::MemElem { mem, addr: n },
                    v,
                    AssignKind::Blocking,
                );
                let more = self.b.add(n, one);
                self.set_size(size, more, span, out);
            }
            "push_front" => {
                let v = self.element_value(mem, values[0], out);
                let Some(v) = self.hold(mem, v, span, out) else {
                    return Some(());
                };
                let zero = self.int(0);
                self.shift(mem, size, zero, true, span, out);
                out.span = span;
                out.assign(
                    ir::Lvalue::MemElem { mem, addr: zero },
                    v,
                    AssignKind::Blocking,
                );
                self.b.span = span;
                let n = self.b.net(size);
                let more = self.b.add(n, one);
                self.set_size(size, more, span, out);
            }
            "insert" => {
                let i = self.dynamic_index(size, values[0], &mut Sink::Proc(out));
                let v = self.element_value(mem, values[1], out);
                let Some(v) = self.hold(mem, v, span, out) else {
                    return Some(());
                };
                self.shift(mem, size, i, true, span, out);
                out.span = span;
                out.assign(
                    ir::Lvalue::MemElem { mem, addr: i },
                    v,
                    AssignKind::Blocking,
                );
                self.b.span = span;
                let n = self.b.net(size);
                let more = self.b.add(n, one);
                self.set_size(size, more, span, out);
            }
            "pop_front" | "pop_back" => {
                let _ = self.pop(mem, size, method == "pop_front", span, out);
            }
            _ => {}
        }
        Some(())
    }

    /// `d = new[n]`, `d = new[n](d)`, `q = other`, `q = '{..}` or
    /// `q = {q, 7, 8}` for a dynamic array `lhs`. `false` when `lhs` is not
    /// one.
    pub(super) fn dynamic_assign(
        &mut self,
        lhs: &'ast Expr,
        rhs: &'ast Expr,
        span: Span,
        out: &mut BlockBuilder,
    ) -> bool {
        let Some((mem, size)) = self.dynamic_of(lhs) else {
            return false;
        };
        match &rhs.kind {
            ExprKind::New(args) => self.dynamic_new(mem, size, args, span, out),
            ExprKind::Pattern(items) => {
                let values: Vec<&'ast Expr> = items.iter().map(|i| &i.value).collect();
                let keyed = items.iter().any(|i| i.key.is_some());
                if keyed {
                    self.env
                        .unsupported(rhs.span, "a keyed pattern assigned to a dynamic array");
                } else {
                    self.dynamic_items(lhs, mem, size, &values, span, out);
                }
            }
            ExprKind::Concat(parts) => {
                let values: Vec<&'ast Expr> = parts.iter().collect();
                self.dynamic_items(lhs, mem, size, &values, span, out);
            }
            _ => {
                let values = [rhs];
                if self.whole_array(rhs).is_some() {
                    self.dynamic_items(lhs, mem, size, &values, span, out);
                } else {
                    self.env.unsupported(
                        rhs.span,
                        "an assignment to a dynamic array from anything but `new`, an array, \
                         a pattern or a concatenation",
                    );
                }
            }
        }
        true
    }

    /// `new[n]` or `new[n](src)` into the dynamic array `mem`.
    fn dynamic_new(
        &mut self,
        mem: MemoryId,
        size: NetId,
        args: &'ast [Expr],
        span: Span,
        out: &mut BlockBuilder,
    ) {
        let Some(count) = args.first() else {
            self.env.error(
                codes::TYPE,
                span,
                "a dynamic array is made with `new[size]`",
            );
            return;
        };
        let n = self.expr(
            count,
            Some(super::Ctx {
                width: 32,
                signed: true,
            }),
            &mut Sink::Proc(out),
        );
        let n = self.coerce(n, 32, true);
        let Some(n_held) = self.loop_var(span) else {
            return;
        };
        out.span = span;
        out.assign(n_held, n, AssignKind::Blocking);
        self.b.span = span;
        let n = self.b.net(n_held);
        let zero = self.int(0);
        match args.get(1) {
            // `new[n](d)`: the elements of `d` it has, the default after.
            Some(src) => {
                let Some((from, from_size)) = self.dynamic_of(src) else {
                    self.env.unsupported(
                        src.span,
                        "`new[n](..)` from anything but a dynamic array or queue",
                    );
                    return;
                };
                if from == mem {
                    let old = self.b.net(size);
                    self.fill(mem, old, n, span, out, &mut |me, _| me.element_default(mem));
                } else {
                    self.fill(mem, zero, n, span, out, &mut |me, i| {
                        let had = me.b.net(from_size);
                        let inside = me.b.binary(ir::BinaryOp::Lt, i, had);
                        let read = me.b.mem_read(from, i);
                        let default = me.element_default(mem);
                        me.b.mux(inside, read, default)
                    });
                }
            }
            None => {
                self.fill(mem, zero, n, span, out, &mut |me, _| {
                    me.element_default(mem)
                });
            }
        }
        self.set_size(size, n, span, out);
    }

    /// The items of a pattern or a concatenation, each an element or an
    /// array, as the new contents of `mem`. The first may be the array
    /// itself, which then keeps its elements and has the rest appended.
    fn dynamic_items(
        &mut self,
        lhs: &'ast Expr,
        mem: MemoryId,
        size: NetId,
        items: &[&'ast Expr],
        span: Span,
        out: &mut BlockBuilder,
    ) {
        let lhs_name = super::path_components(lhs);
        let names_lhs = |e: &Expr| lhs_name.is_some() && super::path_components(e) == lhs_name;
        let start = usize::from(items.first().is_some_and(|e| names_lhs(e)));
        // `r = {q, r}`: the array as it was, copied aside before it changes.
        let snapshot = if items[start..].iter().any(|e| names_lhs(e)) {
            let Some(copy) = self.dynamic_copy(mem, size, span, out) else {
                return;
            };
            Some(copy)
        } else {
            None
        };
        if start == 0 {
            let zero = self.int(0);
            out.span = span;
            out.assign(size, zero, AssignKind::Blocking);
            self.record_driver(size, self.driver, true, span);
        }
        for item in &items[start..] {
            let source = if names_lhs(item) {
                snapshot
            } else {
                self.dynamic_of(item)
            };
            if let Some((from, from_size)) = source {
                // Append every element of another dynamic array.
                self.b.span = span;
                let base = self.b.net(size);
                let Some(base_held) = self.loop_var(span) else {
                    return;
                };
                out.span = span;
                out.assign(base_held, base, AssignKind::Blocking);
                let Some(i) = self.loop_var(span) else { return };
                self.b.span = span;
                let iv = self.b.net(i);
                let bv = self.b.net(base_held);
                let at = self.b.add(bv, iv);
                let read = self.b.mem_read(from, iv);
                let mut body = self.b.block();
                body.span = span;
                body.assign(
                    ir::Lvalue::MemElem { mem, addr: at },
                    read,
                    AssignKind::Blocking,
                );
                let had = self.b.net(from_size);
                let cond = self.b.binary(ir::BinaryOp::Lt, iv, had);
                let one = self.int(1);
                let next = self.b.add(iv, one);
                let zero = self.int(0);
                out.span = span;
                out.for_(
                    Some((ir::Lvalue::Net(i), zero)),
                    Some(cond),
                    Some((ir::Lvalue::Net(i), next)),
                    body.finish(),
                );
                self.b.span = span;
                let bv = self.b.net(base_held);
                let had = self.b.net(from_size);
                let total = self.b.add(bv, had);
                self.set_size(size, total, span, out);
                continue;
            }
            if let Some(view) = self.array_view(item) {
                let elem = self.mems[view.mem.index()].elem.clone();
                let values = self.read_elements(&view, &elem, span);
                for v in values {
                    self.append(mem, size, v, span, out);
                }
                continue;
            }
            let v = self.element_value(mem, item, out);
            self.append(mem, size, v, span, out);
        }
    }

    /// A dynamic array of its own holding what `mem` holds now.
    fn dynamic_copy(
        &mut self,
        mem: MemoryId,
        size: NetId,
        span: Span,
        out: &mut BlockBuilder,
    ) -> Option<(MemoryId, NetId)> {
        let elem = self.mems[mem.index()].elem.clone();
        let ty = VType::Unpacked {
            elem: Box::new(elem),
            range: Range::dynamic(),
        };
        let name = self.fresh("dyn");
        let copy = self.new_memory(name, &ty, span)?;
        let copy_size = self.mems[copy.index()].dynamic?;
        self.b.span = span;
        let zero = self.int(0);
        let n = self.b.net(size);
        self.fill(copy, zero, n, span, out, &mut |me, i| me.b.mem_read(mem, i));
        self.b.span = span;
        let n = self.b.net(size);
        out.span = span;
        out.assign(copy_size, n, AssignKind::Blocking);
        self.record_driver(copy_size, self.driver, true, span);
        Some((copy, copy_size))
    }

    /// `mem[size] = v; size = size + 1`.
    fn append(
        &mut self,
        mem: MemoryId,
        size: NetId,
        v: ExprId,
        span: Span,
        out: &mut BlockBuilder,
    ) {
        self.b.span = span;
        let n = self.b.net(size);
        out.span = span;
        out.assign(
            ir::Lvalue::MemElem { mem, addr: n },
            v,
            AssignKind::Blocking,
        );
        let one = self.int(1);
        let more = self.b.add(n, one);
        self.set_size(size, more, span, out);
    }

    /// What `%p` prints of a dynamic array, built when it runs: `'{` and
    /// each element as `%p` prints a plain value, `,` between, `}`.
    pub(super) fn dynamic_p_string(
        &mut self,
        mem: MemoryId,
        size: NetId,
        span: Span,
        out: &mut BlockBuilder,
    ) -> ExprId {
        let name = self.fresh("dyn");
        self.b.span = span;
        let Some(s) = self.new_var(name, VType::String, span) else {
            return self.b.string("");
        };
        self.record_driver(s, self.driver, true, span);
        let open = self.b.string("'{");
        out.span = span;
        out.assign(s, open, AssignKind::Blocking);
        let Some(i) = self.loop_var(span) else {
            return self.b.net(s);
        };
        self.b.span = span;
        let iv = self.b.net(i);
        let sv = self.b.net(s);
        let read = self.b.mem_read(mem, iv);
        let zero = self.int(0);
        let first = self.b.binary(ir::BinaryOp::Eq, iv, zero);
        let plain = self.b.string("%s%p");
        let comma = self.b.string("%s,%p");
        let fmt = self.b.mux(first, plain, comma);
        let next_s = self.b.call("$sformatf", vec![fmt, sv, read], Type::String);
        let mut body = self.b.block();
        body.span = span;
        body.assign(s, next_s, AssignKind::Blocking);
        let n = self.b.net(size);
        let cond = self.b.binary(ir::BinaryOp::Lt, iv, n);
        let one = self.int(1);
        let next = self.b.add(iv, one);
        out.span = span;
        out.for_(
            Some((ir::Lvalue::Net(i), zero)),
            Some(cond),
            Some((ir::Lvalue::Net(i), next)),
            body.finish(),
        );
        self.b.span = span;
        let sv = self.b.net(s);
        let close = self.b.string("%s}");
        let closed = self.b.call("$sformatf", vec![close, sv], Type::String);
        out.span = span;
        out.assign(s, closed, AssignKind::Blocking);
        self.b.span = span;
        self.b.net(s)
    }

    /// The type a dynamic array's memory has: its dimension the capacity.
    pub(super) fn capacity_type(elem: VType) -> VType {
        VType::Unpacked {
            elem: Box::new(elem),
            range: Range::new(0, i64::try_from(CAPACITY).unwrap_or(i64::MAX) - 1),
        }
    }
}
