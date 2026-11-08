//! `std.textio` writes to the standard output.
//!
//! A `line` is an access to a string, and the IR has neither access types
//! nor string variables, so a `line` object is held as a fixed vector of
//! [`LINE_CHARS`] character codes, the text right-aligned in it (the codes
//! of a shorter text are led by zeros, which read back as nothing). `write`
//! and its relatives append the text of a value to it through the
//! simulator's `$vhdl_line_append`, and `writeline(output, l)` prints it
//! with `$display` and empties it, which is what xsim prints: the line,
//! without a severity or a time.
//!
//! The texts are those of LRM 16.4 as xsim writes them, measured: a
//! `boolean` is `TRUE` or `FALSE`, a `time` is a number of the unit
//! argument (`ns` by default) — a fraction of one when it is not whole,
//! `1.5 ns`, where `to_string(t, ns)` truncates — a `real` is its image
//! unless `digits` asks
//! for a fixed number of decimals, `field` pads to a width on the side
//! `justified` does not name.
//!
//! Not done: reading (`readline`, `read`), a file other than `output`, a
//! line longer than [`LINE_CHARS`] characters (its first characters are
//! lost), and `l.all` or `l'length` of a line.

use crate::ir::builder::BlockBuilder;
use crate::ir::{AssignKind, ExprId, Lvalue, Type};
use crate::logic::Logic;
use crate::source::Span;
use crate::vhdl::ast;
use crate::vhdl::sema::DeclId;

use super::expr::Actual;
use super::lower::{Lowerer, Sink};
use super::types::{BitKind, Layout, LayoutKind};

/// The characters a `line` holds.
pub(crate) const LINE_CHARS: u32 = 1024;

impl<'a> Lowerer<'a, '_> {
    /// A call to a `std.textio` procedure, or to the textio overloads of
    /// `ieee.std_logic_1164`, lowered here. `None` when `d` is not one;
    /// `Some(false)` when it is one this does not support, which has been
    /// reported.
    pub(crate) fn textio_procedure(
        &mut self,
        d: DeclId,
        args: &[Actual<'a>],
        span: Span,
        out: &mut BlockBuilder,
    ) -> Option<bool> {
        let package = self.bundled_package_of(d)?;
        if package != "std.textio" && package != "ieee.std_logic_1164" {
            return None;
        }
        let name = self.a().decl(d).spelling.to_ascii_lowercase();
        match name.as_str() {
            "write" | "swrite" | "hwrite" | "owrite" | "bwrite" => {
                Some(self.textio_write(&name, args, span, out))
            }
            "writeline" => Some(self.writeline(args, span, out)),
            "deallocate" => Some(self.clear_line(args, span, out)),
            "read" | "readline" | "hread" | "oread" | "bread" | "sread" | "tee" => {
                self.unsupported(span, &format!("`{name}` of `std.textio`"));
                Some(false)
            }
            _ => None,
        }
    }

    /// The actual of formal `name`, or of position `index`.
    fn textio_arg(&self, args: &[Actual<'a>], index: usize, name: &str) -> Option<&'a ast::Expr> {
        for a in args {
            if let Some(ast::Expr::Name(ast::Name::Simple(f))) = a.formal()
                && f.name.eq_ignore_ascii_case(name)
            {
                return a.expr();
            }
        }
        let positional: Vec<&Actual<'a>> = args.iter().filter(|a| a.formal().is_none()).collect();
        positional.get(index).and_then(|a| a.expr())
    }

    /// `write(l, value, justified, field, digits|unit)` and the radix
    /// forms: the value's text appended to the line.
    fn textio_write(
        &mut self,
        name: &str,
        args: &[Actual<'a>],
        span: Span,
        out: &mut BlockBuilder,
    ) -> bool {
        let (Some(line), Some(value)) = (
            self.textio_arg(args, 0, "l"),
            self.textio_arg(args, 1, "value"),
        ) else {
            return false;
        };
        let Some((target, line_layout)) = self.line_target(line, out) else {
            return false;
        };
        let Some(text) = self.write_text(name, value, args, span, out) else {
            return false;
        };
        let text = self.justify(text, args, span, out);
        self.b.span = span;
        let old = self.lvalue_read(&target, &line_layout);
        let new = self.b.call(
            "$vhdl_line_append",
            vec![old, text],
            Type::bits(line_layout.width),
        );
        out.span = span;
        out.assign(target, new, AssignKind::Blocking);
        true
    }

    /// The text `write` appends for `value`.
    fn write_text(
        &mut self,
        name: &str,
        value: &'a ast::Expr,
        args: &[Actual<'a>],
        span: Span,
        out: &mut BlockBuilder,
    ) -> Option<ExprId> {
        let (id, l) = self.build(value, None, &mut Sink::Proc(out))?;
        let func = match name {
            "hwrite" => Some("$vhdl_to_hstring"),
            "owrite" => Some("$vhdl_to_ostring"),
            "bwrite" => Some("$vhdl_to_string"),
            _ => None,
        };
        if let Some(f) = func {
            self.b.span = span;
            let flag = self.b.const_bit(false);
            return Some(self.b.call(f, vec![id, flag], Type::String));
        }
        Some(match &l.kind {
            LayoutKind::Str => id,
            LayoutKind::Bit(BitKind::Boolean) => {
                self.b.span = span;
                let f = self.b.string("FALSE");
                let t = self.b.string("TRUE");
                self.b
                    .call("$vhdl_image_enum", vec![id, f, t], Type::String)
            }
            LayoutKind::Physical => {
                let unit = match self.textio_arg(args, 4, "unit") {
                    Some(u) => self.expr_in(u, &l, &mut Sink::Proc(out)),
                    None => {
                        self.b.span = span;
                        self.b
                            .constant(Logic::from_i64(1_000_000, 64).with_signed(true))
                    }
                };
                self.b.span = span;
                self.b
                    .call("$vhdl_time_write", vec![id, unit], Type::String)
            }
            LayoutKind::Real => match self.textio_arg(args, 4, "digits") {
                Some(dg) => {
                    let int = self.int_layout();
                    let dg = self.expr_in(dg, &int, &mut Sink::Proc(out));
                    self.b.span = span;
                    self.b.call("$vhdl_real_digits", vec![id, dg], Type::String)
                }
                None => self.image_of(id, &l, true, span)?,
            },
            _ if self.is_char_array(&l) => self.chars_to_string(id, span),
            // A character, a bit, a vector of bits, an integer: the
            // predefined `to_string`'s text.
            _ => self.image_of(id, &l, false, span)?,
        })
    }

    /// Pads a text to the `field` argument's width, on the side the
    /// `justified` argument does not name (right by default).
    fn justify(
        &mut self,
        text: ExprId,
        args: &[Actual<'a>],
        span: Span,
        out: &mut BlockBuilder,
    ) -> ExprId {
        let Some(field) = self.textio_arg(args, 3, "field") else {
            return text;
        };
        let int = self.int_layout();
        let width = self.expr_in(field, &int, &mut Sink::Proc(out));
        let left = match self.textio_arg(args, 2, "justified") {
            Some(j) => self.cond(j, &mut Sink::Proc(out)),
            None => {
                self.b.span = span;
                self.b.const_bit(false)
            }
        };
        self.b.span = span;
        self.b
            .call("$vhdl_justify", vec![text, left, width], Type::String)
    }

    /// `writeline(output, l)`: the line printed, then emptied.
    fn writeline(&mut self, args: &[Actual<'a>], span: Span, out: &mut BlockBuilder) -> bool {
        let (Some(file), Some(line)) =
            (self.textio_arg(args, 0, "f"), self.textio_arg(args, 1, "l"))
        else {
            return false;
        };
        let to_output = matches!(file, ast::Expr::Name(n)
        if self.a().decl_of(n.span()).is_some_and(|d| {
            self.a().decl(d).spelling.eq_ignore_ascii_case("output")
                && self.bundled_package_of(d).as_deref() == Some("std.textio")
        }));
        if !to_output {
            self.unsupported(span, "`writeline` to a file other than `output`");
            return false;
        }
        let Some((target, layout)) = self.line_target(line, out) else {
            return false;
        };
        self.b.span = span;
        let bits = self.lvalue_read(&target, &layout);
        let text = self.b.call("$vhdl_line_text", vec![bits], Type::String);
        let fmt = self.b.string("%s");
        out.span = span;
        out.syscall("$display", vec![fmt, text]);
        self.empty_line(target, &layout, span, out);
        true
    }

    /// `deallocate(l)`: the line emptied.
    fn clear_line(&mut self, args: &[Actual<'a>], span: Span, out: &mut BlockBuilder) -> bool {
        let Some(line) = self.textio_arg(args, 0, "p") else {
            return false;
        };
        let Some((target, layout)) = self.line_target(line, out) else {
            return false;
        };
        self.empty_line(target, &layout, span, out);
        true
    }

    fn empty_line(&mut self, target: Lvalue, layout: &Layout, span: Span, out: &mut BlockBuilder) {
        self.b.span = span;
        let zero = self.b.constant(Logic::zero(layout.width.max(1)));
        out.span = span;
        out.assign(target, zero, AssignKind::Blocking);
    }

    /// The target a `line` argument names.
    fn line_target(
        &mut self,
        e: &'a ast::Expr,
        out: &mut BlockBuilder,
    ) -> Option<(Lvalue, Layout)> {
        let ast::Expr::Name(n) = e else {
            self.unsupported(e.span(), "a `line` that is not a variable");
            return None;
        };
        self.lvalue_name(n, &mut Sink::Proc(out))
    }

    /// The value a target holds now.
    fn lvalue_read(&mut self, target: &Lvalue, layout: &Layout) -> ExprId {
        match target {
            Lvalue::Net(n) => self.b.net(*n),
            Lvalue::Slice { net, hi, lo } => {
                let base = self.b.net(*net);
                self.slice_bits(base, *hi, *lo)
            }
            _ => self.zero_of(layout),
        }
    }
}
