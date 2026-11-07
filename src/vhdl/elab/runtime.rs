//! Values only the simulation knows: strings built at run time, the type
//! attributes of an object, and the predefined operations that have no
//! VHDL body.
//!
//! Most of what a VHDL testbench prints is built while it runs:
//! `report "n = " & integer'image(n)` has no value at elaboration, so it
//! cannot fold to a string constant the way `report "done"` does. The IR
//! has a string type and the simulator string values, so such an
//! expression lowers to [`crate::ir::ExprKind::Call`]s of the `$vhdl_`
//! functions the simulator implements (their table is in the simulator's
//! `vhdl_runtime` module):
//!
//! | VHDL | IR |
//! |------|----|
//! | `integer'image(n)`, `to_string(n)` | `$vhdl_image_int(n)` |
//! | `real'image(x)` | `$vhdl_image_real(x)` |
//! | `time'image(t)`, `to_string(now)` | `$vhdl_image_time(t)` |
//! | `state_t'image(s)` | `$vhdl_image_enum(s, "idle", "run", …)` |
//! | `std_logic'image(b)` | `$vhdl_concat("'", $vhdl_to_string(b), "'")` |
//! | `character'image(c)` | `$vhdl_concat("'", c, "'")` |
//! | `to_string(v)`, `to_hstring(v)`, `to_ostring(v)` | `$vhdl_to_string(v)`, `$vhdl_to_hstring(v, signed)`, `$vhdl_to_ostring(v, signed)` |
//! | `a & b` of strings and characters | `$vhdl_concat(a, b)` |
//! | `a = b`, `a /= b` of strings | `$vhdl_streq(a, b)` |
//!
//! Anything that *does* fold still becomes one string constant, so a
//! design whose messages are all static lowers exactly as before.
//!
//! The scalar attributes of an object, `t'pos(x)`, `t'val(n)`,
//! `t'succ(x)`, `t'pred(x)`, `t'leftof(x)` and `t'rightof(x)`, are
//! arithmetic on the position encoding [`super::types`] gives every
//! enumeration and integer type, and lower to an add, a subtract or a
//! resize. `std_ulogic` is the exception: its nine positions are folded
//! into four bits, so its position is not in the bits and those
//! attributes of a `std_logic` object are reported.
//!
//! `minimum` and `maximum`, declared with every scalar type, become a
//! comparison and a multiplexer.

use crate::ir::builder::BlockBuilder;
use crate::ir::{ExprId, ExprKind, NetId, Type};
use crate::logic::Logic;
use crate::source::Span;
use crate::vhdl::ast;
use crate::vhdl::sema::{DeclId, DeclKind, SubprogramBody, attrs::Predefined};

use super::codes;
use super::expr::{Actual, first_expr};
use super::lower::{Lowerer, Sink};
use super::types::{BitKind, Layout, LayoutKind};

impl<'a> Lowerer<'a, '_> {
    /// The layout of an unconstrained `string`, the type of every value
    /// built at run time here.
    pub(crate) fn string_layout(&self) -> Layout {
        Layout {
            ty: self.a().builtins.string,
            kind: LayoutKind::Str,
            width: 0,
            signed: false,
        }
    }

    /// True when the layout is a one-dimensional array of `character`, a
    /// constrained `string` object.
    pub(crate) fn is_char_array(&self, l: &Layout) -> bool {
        l.array().is_some_and(|a| self.is_character(&a.elem))
    }

    /// A string value as the character codes of a vector `width` bits wide,
    /// the first character in the most significant byte.
    pub(crate) fn string_to_chars(&mut self, id: ExprId, width: u32, span: Span) -> ExprId {
        self.b.span = span;
        self.b.call("$vhdl_chars_of", vec![id], Type::bits(width))
    }

    /// The character codes of a constrained string object as a string.
    pub(crate) fn chars_to_string(&mut self, id: ExprId, span: Span) -> ExprId {
        self.string_call("$vhdl_string_of", vec![id], span)
    }

    /// True when the layout is the predefined `character` type.
    fn is_character(&self, l: &Layout) -> bool {
        matches!(l.kind, LayoutKind::Enum(_))
            && self.a().same_base(l.ty, self.a().builtins.character)
    }

    /// A string-valued call of one of the simulator's `$vhdl_` functions.
    fn string_call(&mut self, name: &str, args: Vec<ExprId>, span: Span) -> ExprId {
        self.b.span = span;
        self.b.call(name, args, Type::String)
    }

    /// The text `t'image(x)` has, for a value `id` of layout `l`; with
    /// `quoted` false, the text the predefined `to_string(x)` has, which
    /// writes a character literal without its quotes.
    pub(crate) fn image_of(
        &mut self,
        id: ExprId,
        l: &Layout,
        quoted: bool,
        span: Span,
    ) -> Option<ExprId> {
        let quote = |this: &mut Self, inner: ExprId| -> ExprId {
            if !quoted {
                return inner;
            }
            this.b.span = span;
            let q1 = this.b.string("'");
            let q2 = this.b.string("'");
            this.string_call("$vhdl_concat", vec![q1, inner, q2], span)
        };
        match &l.kind {
            LayoutKind::Int => Some(self.string_call("$vhdl_image_int", vec![id], span)),
            LayoutKind::Real => Some(self.string_call("$vhdl_image_real", vec![id], span)),
            LayoutKind::Physical => {
                if self.a().same_base(l.ty, self.a().builtins.time) {
                    return Some(self.string_call("$vhdl_image_time", vec![id], span));
                }
                let base = self.a().base_type(l.ty);
                let unit = match &self.a().ty(base).kind {
                    crate::vhdl::sema::TypeKind::Physical { units, .. } => units
                        .first()
                        .map(|u| self.a().decl(*u).spelling.to_lowercase())
                        .unwrap_or_default(),
                    _ => String::new(),
                };
                let n = self.string_call("$vhdl_image_int", vec![id], span);
                self.b.span = span;
                let u = self.b.string(format!(" {unit}"));
                Some(self.string_call("$vhdl_concat", vec![n, u], span))
            }
            LayoutKind::Bit(BitKind::Boolean) => {
                self.b.span = span;
                let f = self.b.string("false");
                let t = self.b.string("true");
                Some(self.string_call("$vhdl_image_enum", vec![id, f, t], span))
            }
            LayoutKind::Bit(_) => {
                let text = self.string_call("$vhdl_to_string", vec![id], span);
                Some(quote(self, text))
            }
            LayoutKind::Enum(_) if self.is_character(l) => {
                self.b.span = span;
                let text = self.string_call("$vhdl_concat", vec![id], span);
                Some(quote(self, text))
            }
            LayoutKind::Enum(names) => {
                let names = names.clone();
                let mut args = vec![id];
                for n in names {
                    let text = crate::vhdl::sema::attrs::literal_image(&n);
                    let text = if quoted {
                        text
                    } else {
                        match text.strip_prefix('\'').and_then(|t| t.strip_suffix('\'')) {
                            Some(inner) if !inner.is_empty() => inner.to_owned(),
                            _ => text,
                        }
                    };
                    self.b.span = span;
                    args.push(self.b.string(text));
                }
                Some(self.string_call("$vhdl_image_enum", args, span))
            }
            LayoutKind::Array(arr) if matches!(arr.elem.kind, LayoutKind::Bit(_)) => {
                Some(self.string_call("$vhdl_to_string", vec![id], span))
            }
            LayoutKind::Str => Some(id),
            _ => {
                self.unsupported(span, "the image of a value of this type");
                None
            }
        }
    }

    /// The shadow net of `signal` that keeps `kind` (`event`, `last_value`
    /// or `last_event`; see the simulator's VHDL runtime), made on first
    /// use.
    pub(crate) fn shadow(
        &mut self,
        signal: NetId,
        kind: &'static str,
        span: Span,
    ) -> Option<NetId> {
        if let Some(&s) = self.shadows.get(&(signal, kind)) {
            return Some(s);
        }
        let base_name = self.b.module().nets[signal].name.to_string();
        let layout = match kind {
            "event" => self.boolean_layout(),
            "last_event" => self.physical_layout(),
            _ => self.net_layout(signal),
        };
        let name = self.fresh(&format!("{base_name}${kind}"));
        let net = self.new_net(name, layout, true, span)?;
        self.b.module_mut().nets[net].kind = crate::ir::NetKind::Reg;
        self.b.net_attr(net, "vhdl_shadow", kind);
        self.b.net_attr(net, "vhdl_shadow_of", base_name);
        self.shadows.insert((signal, kind), net);
        self.shadow_base.insert(net, signal);
        Some(net)
    }

    /// The net of a name that denotes a whole signal, for its attributes.
    fn attribute_signal(&mut self, prefix: &'a ast::Name, span: Span) -> Option<NetId> {
        let net = self.signal_net(prefix);
        if net.is_none() {
            self.unsupported(
                span,
                "this attribute of something other than a whole signal",
            );
        }
        net
    }

    /// `rising_edge(s)` or `falling_edge(s)` as a value: `s'event`, with
    /// `s` now `'1'` and before `'0'` (or the other way round), as IEEE
    /// 1164 defines them. As the clock condition of a process they are
    /// recognised before this point and become the process's clock.
    pub(crate) fn edge_expr(
        &mut self,
        n: &'a ast::Name,
        rising: bool,
        span: Span,
    ) -> Option<(ExprId, Layout)> {
        let net = self.attribute_signal(n, span)?;
        let event = self.shadow(net, "event", span)?;
        let last = self.shadow(net, "last_value", span)?;
        self.b.span = span;
        let (now_bit, before_bit) = if rising { (true, false) } else { (false, true) };
        let cur = self.b.net(net);
        let cur = self.slice_bits(cur, 0, 0);
        let want_now = self.b.const_bit(now_bit);
        let is_now = self.b.binary(crate::ir::BinaryOp::CaseEq, cur, want_now);
        let old = self.b.net(last);
        let old = self.slice_bits(old, 0, 0);
        let want_before = self.b.const_bit(before_bit);
        let was = self.b.binary(crate::ir::BinaryOp::CaseEq, old, want_before);
        let ev = self.b.net(event);
        let both = self.b.land(is_now, was);
        let id = self.b.land(ev, both);
        Some((id, self.boolean_layout()))
    }

    /// A signal attribute read as a value: `s'event`, `s'active`,
    /// `s'last_value`, `s'last_event`, `s'stable` and `s'stable(t)`, and
    /// `s'delayed(t)`; the bound attributes of an object whose subtype was
    /// not static, `x'length`, are evaluated here too when they fold.
    pub(crate) fn signal_attribute(
        &mut self,
        prefix: &'a ast::Name,
        attribute: &str,
        arg: Option<&'a ast::Expr>,
        span: Span,
        sink: &mut Sink<'_>,
    ) -> Option<(ExprId, Layout)> {
        let _ = sink;
        let attr = attribute.to_ascii_lowercase();
        match attr.as_str() {
            // There are no transactions without a change here, so an
            // active signal is one with an event.
            "event" | "active" => {
                let net = self.attribute_signal(prefix, span)?;
                let ev = self.shadow(net, "event", span)?;
                self.b.span = span;
                Some((self.b.net(ev), self.boolean_layout()))
            }
            "last_value" => {
                let net = self.attribute_signal(prefix, span)?;
                let last = self.shadow(net, "last_value", span)?;
                let layout = self.net_layout(net);
                self.b.span = span;
                Some((self.b.net(last), layout))
            }
            "last_event" => {
                let net = self.attribute_signal(prefix, span)?;
                let at = self.shadow(net, "last_event", span)?;
                let now = self.now_expr(span);
                self.b.span = span;
                let at = self.b.net(at);
                Some((self.b.sub(now, at), self.physical_layout()))
            }
            "stable" | "quiet" => {
                let net = self.attribute_signal(prefix, span)?;
                let t = match arg {
                    Some(t) => self.eval_int(t).and_then(|v| i64::try_from(v).ok()),
                    None => Some(0),
                };
                let Some(t) = t else {
                    self.unsupported(span, "`'stable` of a time that is not static");
                    return None;
                };
                if t == 0 {
                    let ev = self.shadow(net, "event", span)?;
                    self.b.span = span;
                    let ev = self.b.net(ev);
                    return Some((self.b.lnot(ev), self.boolean_layout()));
                }
                // Stable for `t`: no event in this delta, and the last one
                // at least `t` ago. Evaluated where it is read, so a
                // signal assigned from it changes only when something it
                // reads does.
                let ev = self.shadow(net, "event", span)?;
                let at = self.shadow(net, "last_event", span)?;
                let now = self.now_expr(span);
                self.b.span = span;
                let at = self.b.net(at);
                let since = self.b.sub(now, at);
                let limit = self.b.constant(Logic::from_i64(t, 64).with_signed(true));
                let long = self.b.ge(since, limit);
                let ev = self.b.net(ev);
                let quiet = self.b.lnot(ev);
                Some((self.b.land(quiet, long), self.boolean_layout()))
            }
            "delayed" => self.delayed(prefix, arg, span),
            _ => {
                self.error(
                    codes::NOT_STATIC,
                    span,
                    format!("the attribute `{attribute}` cannot be lowered to the IR"),
                );
                None
            }
        }
    }

    /// `s'delayed(t)`: a signal that follows `s` `t` later, by transport
    /// (LRM 16.2.4), kept by a process of its own.
    fn delayed(
        &mut self,
        prefix: &'a ast::Name,
        arg: Option<&'a ast::Expr>,
        span: Span,
    ) -> Option<(ExprId, Layout)> {
        let net = self.attribute_signal(prefix, span)?;
        let fs = match arg {
            Some(t) => self.eval_int(t).and_then(|v| u64::try_from(v).ok()),
            None => Some(0),
        };
        let Some(fs) = fs else {
            self.unsupported(span, "`'delayed` of a time that is not static");
            return None;
        };
        if let Some(&d) = self.shadows.get(&(net, "delayed")) {
            let layout = self.net_layout(net);
            self.b.span = span;
            return Some((self.b.net(d), layout));
        }
        let layout = self.net_layout(net);
        let base = self.b.module().nets[net].name.to_string();
        let name = self.fresh(&format!("{base}$delayed"));
        let d = self.new_net(name, layout.clone(), true, span)?;
        self.b.net_attr(d, "vhdl_shadow", "start");
        self.b.net_attr(d, "vhdl_shadow_of", base);
        let index = self.b.module().processes.len();
        let outer = self.driver;
        self.driver = super::lower::DriverKey::Process(index);
        self.b.span = span;
        let mut p = self
            .b
            .process(None, crate::ir::ProcessKind::Sensitive(vec![net]));
        p.span = span;
        let v = self.b.net(net);
        if fs == 0 {
            p.assign(
                crate::ir::Lvalue::Net(d),
                v,
                crate::ir::AssignKind::NonBlocking,
            );
        } else {
            p.assign_after(
                crate::ir::Lvalue::Net(d),
                v,
                crate::ir::AssignKind::NonBlocking,
                super::stmt::delay_from_fs(fs),
            );
        }
        let pid = self.b.end_process(p);
        self.record_driver(d, super::lower::DriverSite::Process(pid), true, span);
        self.driver = outer;
        self.shadows.insert((net, "delayed"), d);
        self.b.span = span;
        Some((self.b.net(d), layout))
    }

    /// A predefined attribute with an argument whose value is only known
    /// at run time: `t'image(x)`, `t'pos(x)`, `t'val(n)`, `t'succ(x)`,
    /// `t'pred(x)`, `t'leftof(x)`, `t'rightof(x)`.
    pub(crate) fn runtime_attribute(
        &mut self,
        p: Predefined,
        attr_name: Option<&'a ast::Name>,
        args: &'a [ast::AssociationElement],
        span: Span,
        sink: &mut Sink<'_>,
    ) -> Option<(ExprId, Layout)> {
        let Some(arg) = first_expr(args) else {
            self.attribute_not_static(p, span);
            return None;
        };
        let result = self.layout_at(span);
        match p {
            Predefined::Image => {
                let (id, l) = self.build(arg, None, sink)?;
                let id = self.image_of(id, &l, true, span)?;
                Some((id, self.string_layout()))
            }
            Predefined::Pos | Predefined::Val => {
                let want = result?;
                let (id, l) = self.build(arg, None, sink)?;
                if matches!(l.kind, LayoutKind::Bit(BitKind::StdLogic))
                    || matches!(want.kind, LayoutKind::Bit(BitKind::StdLogic))
                {
                    self.unsupported(
                        span,
                        "the position of a `std_ulogic` value, which is not held in its bits",
                    );
                    return None;
                }
                self.b.span = span;
                let id = self.widen(id, want.width, l.signed);
                Some((id, want))
            }
            Predefined::Succ | Predefined::Pred | Predefined::LeftOf | Predefined::RightOf => {
                let (id, l) = self.build(arg, None, sink)?;
                if matches!(l.kind, LayoutKind::Bit(BitKind::StdLogic)) {
                    self.unsupported(
                        span,
                        "the successor of a `std_ulogic` value, which is not held in its bits",
                    );
                    return None;
                }
                let ascending = self
                    .a()
                    .scalar_range(l.ty)
                    .is_none_or(|b| b.dir == ast::Direction::To);
                let forward = match p {
                    Predefined::Succ => true,
                    Predefined::Pred => false,
                    Predefined::RightOf => ascending,
                    _ => !ascending,
                };
                self.b.span = span;
                let one = self
                    .b
                    .constant(Logic::from_u64(1, l.width.max(1)).with_signed(l.signed));
                let id = if forward {
                    self.b.add(id, one)
                } else {
                    self.b.sub(id, one)
                };
                Some((id, result.unwrap_or(l)))
            }
            Predefined::Stable | Predefined::Quiet | Predefined::Delayed => {
                let ast::Name::Attribute {
                    prefix, attribute, ..
                } = attr_name?
                else {
                    self.attribute_not_static(p, span);
                    return None;
                };
                self.signal_attribute(prefix, &attribute.name, Some(arg), span, sink)
            }
            _ => {
                self.attribute_not_static(p, span);
                None
            }
        }
    }

    /// A `real` constant: the IR has no real literal, so it is the
    /// simulator's `$vhdl_real` of the value's IEEE 754 bits.
    pub(crate) fn real_const(&mut self, r: f64, span: Span) -> ExprId {
        self.b.span = span;
        let bits = self.b.constant(Logic::from_u64(r.to_bits(), 64));
        self.b.call("$vhdl_real", vec![bits], Type::Real)
    }

    /// The layout of `real`.
    pub(crate) fn real_layout(&self) -> Layout {
        Layout {
            ty: self.a().builtins.real,
            kind: LayoutKind::Real,
            width: 0,
            signed: true,
        }
    }

    /// A value as a `real`: itself when it is one, else the conversion of
    /// an integer or physical value.
    pub(crate) fn as_real(&mut self, id: ExprId, l: &Layout, span: Span) -> ExprId {
        if matches!(l.kind, LayoutKind::Real) {
            return id;
        }
        self.b.span = span;
        self.b.call("$vhdl_int_to_real", vec![id], Type::Real)
    }

    /// A `real` rounded to an integer or physical value of `to`'s width.
    pub(crate) fn real_to_int(&mut self, id: ExprId, to: &Layout, span: Span) -> ExprId {
        self.b.span = span;
        self.b
            .call("$vhdl_real_to_int", vec![id], Type::sbits(to.width.max(1)))
    }

    /// A binary operator of which an operand is a `real`: the operands are
    /// converted to `real`, the IR's operator applied to them, and the
    /// result rounded when its type is not `real` (`period * 1.5` is a
    /// time). The IR has `real` arithmetic and comparisons; `**` and `mod`
    /// are the simulator's.
    pub(crate) fn real_binary(
        &mut self,
        sym: &str,
        (l, ll): (ExprId, &Layout),
        (r, rl): (ExprId, &Layout),
        result: &Layout,
        span: Span,
    ) -> Option<(ExprId, Layout)> {
        use crate::ir::BinaryOp as B;
        if sym == "**" {
            let x = self.as_real(l, ll, span);
            let y = self.as_real(r, rl, span);
            self.b.span = span;
            let f = self.b.string("**");
            let id = self.b.call("$vhdl_math", vec![f, x, y], Type::Real);
            return Some((id, self.real_layout()));
        }
        let x = self.as_real(l, ll, span);
        let y = self.as_real(r, rl, span);
        self.b.span = span;
        let op = match sym {
            "+" => B::Add,
            "-" => B::Sub,
            "*" => B::Mul,
            "/" => B::Div,
            "=" => B::Eq,
            "/=" => B::Ne,
            "<" => B::Lt,
            "<=" => B::Le,
            ">" => B::Gt,
            ">=" => B::Ge,
            "mod" | "rem" => {
                let f = self.b.string("mod");
                let id = self.b.call("$vhdl_math", vec![f, x, y], Type::Real);
                return Some((id, self.real_layout()));
            }
            other => {
                self.unsupported(span, &format!("the operator `{other}` of `real` operands"));
                return None;
            }
        };
        let id = self.b.binary(op, x, y);
        if op.is_predicate() {
            return Some((id, self.boolean_layout()));
        }
        if matches!(result.kind, LayoutKind::Real) {
            return Some((id, result.clone()));
        }
        // A physical or integer result, `t * 1.5`.
        Some((self.real_to_int(id, result, span), result.clone()))
    }

    /// A call to an `ieee.math_real` function, at run time.
    fn math_real(
        &mut self,
        name: &str,
        args: &[Actual<'a>],
        span: Span,
        sink: &mut Sink<'_>,
    ) -> Option<(ExprId, Layout)> {
        let mut ids = Vec::with_capacity(args.len() + 1);
        self.b.span = span;
        ids.push(self.b.string(name));
        for a in args {
            let e = a.expr()?;
            let (id, l) = self.build(e, None, sink)?;
            ids.push(self.as_real(id, &l, e.span()));
        }
        self.b.span = span;
        let id = self.b.call("$vhdl_math", ids, Type::Real);
        Some((id, self.real_layout()))
    }

    /// Reports an attribute that has no value here.
    pub(crate) fn attribute_not_static(&mut self, p: Predefined, span: Span) {
        self.error(
            codes::NOT_STATIC,
            span,
            format!("the attribute `{p:?}` is not static after elaboration"),
        );
    }

    /// `a & b` whose result is a string: every operand of the chain
    /// becomes an argument of one `$vhdl_concat`.
    pub(crate) fn string_concat(
        &mut self,
        lhs: &'a ast::Expr,
        rhs: &'a ast::Expr,
        span: Span,
        sink: &mut Sink<'_>,
    ) -> Option<(ExprId, Layout)> {
        let mut leaves = Vec::new();
        self.concat_leaves(lhs, &mut leaves);
        self.concat_leaves(rhs, &mut leaves);
        let mut parts = Vec::with_capacity(leaves.len());
        for leaf in leaves {
            let (id, l) = self.build(leaf, None, sink)?;
            if matches!(l.kind, LayoutKind::Str) || self.is_character(&l) {
                parts.push(id);
                continue;
            }
            if self.is_char_array(&l) {
                parts.push(self.chars_to_string(id, leaf.span()));
                continue;
            }
            self.error(
                codes::TYPE,
                leaf.span(),
                "this operand of a string concatenation is neither a string nor a character",
            );
            return None;
        }
        Some((
            self.string_call("$vhdl_concat", parts, span),
            self.string_layout(),
        ))
    }

    /// The operands of a chain of string concatenations, left to right.
    fn concat_leaves(&mut self, e: &'a ast::Expr, out: &mut Vec<&'a ast::Expr>) {
        match e {
            ast::Expr::Binary {
                op: ast::BinaryOp::Concat,
                lhs,
                rhs,
                span,
            } if self.eval(e).is_none()
                && self
                    .layout_at(*span)
                    .is_some_and(|l| matches!(l.kind, LayoutKind::Str)) =>
            {
                self.concat_leaves(lhs, out);
                self.concat_leaves(rhs, out);
            }
            ast::Expr::Paren { inner, .. } => self.concat_leaves(inner, out),
            other => out.push(other),
        }
    }

    /// `a = b` or `a /= b` of two strings.
    pub(crate) fn string_compare(
        &mut self,
        equal: bool,
        l: ExprId,
        r: ExprId,
        span: Span,
    ) -> (ExprId, Layout) {
        self.b.span = span;
        let mut id = self.b.call("$vhdl_streq", vec![l, r], Type::bit());
        if !equal {
            id = self.b.lnot(id);
        }
        (id, self.boolean_layout())
    }

    /// The bit-vector text conversions: `to_string`, `to_bstring`,
    /// `to_ostring` and `to_hstring` of a vector of single-bit elements.
    pub(crate) fn vector_text(
        &mut self,
        name: &str,
        arg: &'a ast::Expr,
        signed: bool,
        span: Span,
        sink: &mut Sink<'_>,
    ) -> Option<(ExprId, Layout)> {
        let (id, l) = self.build(arg, None, sink)?;
        if !l.is_bits() {
            self.error(codes::TYPE, arg.span(), "this value is not a bit vector");
            return None;
        }
        let func = match name {
            "to_hstring" => "$vhdl_to_hstring",
            "to_ostring" => "$vhdl_to_ostring",
            _ => "$vhdl_to_string",
        };
        self.b.span = span;
        let flag = self.b.const_bit(signed);
        let id = self.string_call(func, vec![id, flag], span);
        Some((id, self.string_layout()))
    }

    /// A call to a subprogram the language or a bundled package declares
    /// without a VHDL body, which elaboration implements: the predefined
    /// `minimum`, `maximum` and `to_string` of a type, `std.standard`'s
    /// `now` and `to_hstring` family. `None` when `d` is not one of them.
    pub(crate) fn native_function(
        &mut self,
        d: DeclId,
        args: &[Actual<'a>],
        span: Span,
        sink: &mut Sink<'_>,
    ) -> Option<Option<(ExprId, Layout)>> {
        let d = super::lower::resolve_alias(self.a(), d);
        let DeclKind::Subprogram { body, .. } = &self.a().decl(d).kind else {
            return None;
        };
        let predefined = matches!(body, SubprogramBody::Predefined);
        let name = self.a().decl(d).spelling.to_ascii_lowercase();
        let package = self.bundled_package_of(d);
        if package.as_deref() == Some("ieee.math_real") {
            let name = name.trim_matches('"').to_owned();
            return Some(self.math_real(&name, args, span, sink));
        }
        let standard = package.as_deref() == Some("std.standard");
        let first = args.first().and_then(|a| a.expr());
        if predefined {
            // Whatever happens to the arguments, the call is this one: a
            // failure below has been reported and must not fall through
            // to inlining, which would add "no body to inline".
            return Some(match name.as_str() {
                "to_string" => first.and_then(|arg| self.predefined_to_string(arg, span, sink)),
                "minimum" | "maximum" => self.extremum(&name, args, span, sink),
                _ => None,
            });
        }
        if !standard {
            return None;
        }
        match name.as_str() {
            "now" => {
                self.b.span = span;
                let t = self.b.call("$time", Vec::new(), Type::bits(64));
                let id = self.b.resize(t, 64, true);
                let layout = Layout {
                    kind: LayoutKind::Physical,
                    width: 64,
                    signed: true,
                    ty: self.a().builtins.time,
                };
                Some(Some((id, layout)))
            }
            "to_bstring" | "to_ostring" | "to_hstring" => {
                Some(first.and_then(|arg| self.vector_text(&name, arg, false, span, sink)))
            }
            "to_string" if args.len() == 2 => Some(self.two_argument_to_string(args, span, sink)),
            _ => None,
        }
    }

    /// `to_string(t, unit)`, `to_string(r, digits)` and `to_string(r,
    /// format)` of `std.standard`.
    fn two_argument_to_string(
        &mut self,
        args: &[Actual<'a>],
        span: Span,
        sink: &mut Sink<'_>,
    ) -> Option<(ExprId, Layout)> {
        let value = args.first().and_then(|a| a.expr())?;
        let second = args.get(1).and_then(|a| a.expr())?;
        let (v, vl) = self.build(value, None, sink)?;
        let (s, _) = self.build(second, None, sink)?;
        let func = match vl.kind {
            LayoutKind::Physical => "$vhdl_time_in",
            LayoutKind::Real => match self.a().type_of(second.span()) {
                Some(t) if self.a().is_string_type(t) => "$vhdl_real_format",
                _ => "$vhdl_real_digits",
            },
            _ => {
                self.unsupported(span, "this `to_string` overload");
                return None;
            }
        };
        let id = self.string_call(func, vec![v, s], span);
        Some((id, self.string_layout()))
    }

    /// The predefined `to_string(x)` of a scalar or of an array of a
    /// character-literal type.
    fn predefined_to_string(
        &mut self,
        arg: &'a ast::Expr,
        span: Span,
        sink: &mut Sink<'_>,
    ) -> Option<(ExprId, Layout)> {
        let (id, l) = self.build(arg, None, sink)?;
        let id = self.image_of(id, &l, false, span)?;
        Some((id, self.string_layout()))
    }

    /// A call to a procedure the simulator implements: `std.env.finish`
    /// and `std.env.stop` (LRM 16.5), with or without a status, which end
    /// and pause the simulation as `$finish` and `$stop` do. The status is
    /// not passed on. True when `d` is one of them and was lowered.
    pub(crate) fn native_procedure(
        &mut self,
        d: DeclId,
        span: Span,
        out: &mut BlockBuilder,
    ) -> bool {
        let d = super::lower::resolve_alias(self.a(), d);
        if self.bundled_package_of(d).as_deref() != Some("std.env") {
            return false;
        }
        let name = self.a().decl(d).spelling.to_ascii_lowercase();
        out.span = span;
        match name.as_str() {
            "finish" => out.finish_sim(),
            "stop" => out.stop(),
            _ => return false,
        }
        true
    }

    /// `minimum(l, r)` or `maximum(l, r)` of a scalar type.
    fn extremum(
        &mut self,
        name: &str,
        args: &[Actual<'a>],
        span: Span,
        sink: &mut Sink<'_>,
    ) -> Option<(ExprId, Layout)> {
        let l = args.first().and_then(|a| a.expr())?;
        let r = args.get(1).and_then(|a| a.expr())?;
        let (lid, ll) = self.build(l, None, sink)?;
        let rid = self.expr_in(r, &ll, sink);
        let result = self.layout_at(span).unwrap_or_else(|| ll.clone());
        self.b.span = span;
        let less = self.b.lt(lid, rid);
        let id = if name == "minimum" {
            self.b.mux(less, lid, rid)
        } else {
            self.b.mux(less, rid, lid)
        };
        let id = self.coerce(id, &ll, &result, span);
        Some((id, result))
    }

    /// The message arguments of an assertion or report: the string, passed
    /// to the simulator through a `%s` so that a `%` in it is printed
    /// rather than read as a format.
    pub(crate) fn message_args(&mut self, id: ExprId, span: Span) -> Vec<ExprId> {
        if let ExprKind::String(s) = &self.b.module().expr(id).kind
            && !s.contains('%')
        {
            return vec![id];
        }
        self.b.span = span;
        let fmt = self.b.string("%s");
        vec![fmt, id]
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{elab, rtl};

    /// Would catch a message that folds being lowered to run-time calls
    /// anyway, which would change the IR of every design whose messages
    /// are static; would not catch a wrong text, which the simulation
    /// fixtures hold.
    #[test]
    fn a_static_message_stays_one_string() {
        let out = rtl("entity e is end entity;\n\
             architecture a of e is begin\n\
               process begin report \"do\" & \"ne\"; wait; end process;\n\
             end architecture;");
        assert!(out.contains("\"done\""), "{out}");
        assert!(!out.contains("$vhdl_"), "{out}");
    }

    /// Would catch an explicit `to_string` of a user type clashing with
    /// the predefined one (`V0202`) or losing to it; would not catch a
    /// predefined operation hidden in a region where it should stay
    /// visible.
    #[test]
    fn an_explicit_to_string_hides_the_predefined_one() {
        let (design, diags) = elab(
            "entity e is end entity;\n\
             architecture a of e is\n\
               type state_t is (idle, run);\n\
               function to_string (s : state_t) return string is\n\
               begin return \"custom\"; end function;\n\
             begin\n\
               process variable s : state_t := run;\n\
               begin report to_string(run) & \"/\" & state_t'image(s); wait; end process;\n\
             end architecture;",
        );
        let out = design.unwrap_or_else(|| panic!("{diags}")).to_text();
        assert!(out.contains("\"custom/\""), "{out}");
    }

    /// Would catch a run-time image of an enumeration losing its literal
    /// list, or the literals keeping their source case; would not catch
    /// the simulator's rendering, which `tests/vhdl_xsim.rs` holds.
    #[test]
    fn an_enumeration_image_carries_its_literals() {
        let out = rtl("entity e is end entity;\n\
             architecture a of e is\n\
               type state_t is (Idle, Run);\n\
             begin\n\
               process variable s : state_t := Run;\n\
               begin report state_t'image(s); wait; end process;\n\
             end architecture;");
        assert!(out.contains("\"idle\", \"run\")"), "{out}");
        assert!(out.contains("$vhdl_image_enum"), "{out}");
    }
}
