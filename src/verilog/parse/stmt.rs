//! Procedural statements.
//!
//! [`Parser::parse_stmt`] dispatches on the first token. Statements that
//! begin with a name (assignments, calls, `++`/`--`) are parsed by taking
//! the left side as a postfix expression and then looking at the operator;
//! declarations inside blocks are recognised by [`Parser::looks_like_decl`]
//! and wrapped as [`StmtKind::Decl`].
//!
//! Lists of statements ([`Parser::parse_stmts_until`]) recover from a
//! failed statement by skipping to the next `;` or block keyword, so a
//! block with one broken line yields one diagnostic and keeps its other
//! statements.

use super::super::ast::{
    AssertKind, AssertSpec, Assertion, Assign, Block, Case, CaseItem, CaseKind, Deferred, Delay,
    Edge, EventControl, EventControlKind, EventExpr, Expr, ExprKind, For, ForInit, Foreach, Ident,
    If, Item, ItemKind, JoinKind, Qualifier, Stmt, StmtKind, TimingControl, TimingKind, VarDecl,
};
use super::super::token::{Keyword, Punct, TokenKind};
use super::expr::assign_op;
use super::{PResult, Parser};

/// Keywords that end any statement list, so a missing `end` is caught at
/// the enclosing construct rather than swallowing the rest of the module.
/// Keywords that can only start a new item (`always`, `initial`,
/// `function`, `module`...) count too: reaching one inside a block means
/// the block was never closed.
fn ends_stmt_list(kind: &TokenKind) -> bool {
    use Keyword as K;
    matches!(
        kind,
        TokenKind::Keyword(
            K::End
                | K::Join
                | K::JoinAny
                | K::JoinNone
                | K::Endcase
                | K::Endfunction
                | K::Endtask
                | K::Endmodule
                | K::Endinterface
                | K::Endprogram
                | K::Endpackage
                | K::Endgenerate
                | K::Endprimitive
                | K::Endclocking
                | K::Endproperty
                | K::Endsequence
                | K::Endspecify
                | K::Endtable
                | K::Always
                | K::AlwaysComb
                | K::AlwaysFf
                | K::AlwaysLatch
                | K::Initial
                | K::Final
                | K::Function
                | K::Task
                | K::Module
                | K::Macromodule
                | K::Interface
                | K::Program
                | K::Package
                | K::Primitive
                | K::Generate
                | K::Modport
        )
    )
}

impl Parser<'_> {
    /// Statements up to end of input, a keyword in `terminators`, or any
    /// block-ending keyword (left for the caller to check).
    pub(super) fn parse_stmts_until(&mut self, terminators: &[Keyword]) -> Vec<Stmt> {
        let mut stmts = Vec::new();
        loop {
            if self.at_eof() || ends_stmt_list(self.kind()) {
                break;
            }
            if let TokenKind::Keyword(kw) = self.kind()
                && terminators.contains(kw)
            {
                break;
            }
            let before = self.pos;
            match self.parse_stmt() {
                Ok(stmt) => stmts.push(stmt),
                Err(_) => self.recover_stmt(),
            }
            if self.pos == before {
                self.bump();
            }
        }
        stmts
    }

    /// One statement, including a lone `;`.
    pub(super) fn parse_stmt(&mut self) -> PResult<Stmt> {
        let start = self.span();
        let attrs = self.parse_attrs()?;
        let label = if self.at_ident() && self.nth_is_punct(1, Punct::Colon) {
            let label = self.expect_ident()?;
            self.bump();
            Some(label)
        } else {
            None
        };
        let kind = self.parse_stmt_kind()?;
        Ok(Stmt {
            label,
            attrs,
            kind,
            span: self.span_from(start),
        })
    }

    /// A statement, or a lone `;` (which is a [`StmtKind::Null`]).
    fn parse_stmt_or_null(&mut self) -> PResult<Stmt> {
        self.parse_stmt()
    }

    /// Dispatches on the first token of a statement.
    fn parse_stmt_kind(&mut self) -> PResult<StmtKind> {
        use Keyword as K;
        if self.looks_like_decl() {
            return self.parse_decl_stmt();
        }
        let kind = match self.kind() {
            TokenKind::Punct(Punct::Semi) => {
                self.bump();
                StmtKind::Null
            }
            TokenKind::Punct(Punct::Hash) => {
                let start = self.span();
                let delay = self.parse_delay()?;
                let control = TimingControl {
                    kind: TimingKind::Delay(delay),
                    span: self.span_from(start),
                };
                let body = self.parse_stmt_or_null()?;
                StmtKind::Timing(control, Box::new(body))
            }
            TokenKind::Punct(Punct::At) => {
                let event = self.parse_event_control()?;
                let span = event.span;
                let control = TimingControl {
                    kind: TimingKind::Event(event),
                    span,
                };
                let body = self.parse_stmt_or_null()?;
                StmtKind::Timing(control, Box::new(body))
            }
            TokenKind::Punct(Punct::Arrow | Punct::ArrowArrow) => {
                let nonblocking = self.at_punct(Punct::ArrowArrow);
                self.bump();
                let target = self.parse_lvalue()?;
                self.expect_semi()?;
                StmtKind::Trigger {
                    nonblocking,
                    target,
                }
            }
            TokenKind::Keyword(kw) => match *kw {
                K::Begin => StmtKind::Block(self.parse_block()?),
                K::Fork => {
                    let (block, join) = self.parse_fork()?;
                    StmtKind::Fork(block, join)
                }
                K::If => StmtKind::If(self.parse_if(None)?),
                K::Case | K::Casez | K::Casex => StmtKind::Case(self.parse_case(None)?),
                K::Unique | K::Unique0 | K::Priority => {
                    let qualifier = match *kw {
                        K::Unique => Qualifier::Unique,
                        K::Unique0 => Qualifier::Unique0,
                        _ => Qualifier::Priority,
                    };
                    self.bump();
                    if self.at_kw(K::If) {
                        StmtKind::If(self.parse_if(Some(qualifier))?)
                    } else {
                        StmtKind::Case(self.parse_case(Some(qualifier))?)
                    }
                }
                K::For => StmtKind::For(self.parse_for()?),
                K::While => {
                    self.bump();
                    self.expect_punct(Punct::LParen)?;
                    let cond = self.parse_expr()?;
                    self.expect_punct(Punct::RParen)?;
                    let body = self.parse_stmt_or_null()?;
                    StmtKind::While(cond, Box::new(body))
                }
                K::Do => {
                    self.bump();
                    let body = self.parse_stmt_or_null()?;
                    self.expect_kw(K::While)?;
                    self.expect_punct(Punct::LParen)?;
                    let cond = self.parse_expr()?;
                    self.expect_punct(Punct::RParen)?;
                    self.expect_semi()?;
                    StmtKind::DoWhile(Box::new(body), cond)
                }
                K::Repeat => {
                    self.bump();
                    self.expect_punct(Punct::LParen)?;
                    let count = self.parse_expr()?;
                    self.expect_punct(Punct::RParen)?;
                    let body = self.parse_stmt_or_null()?;
                    StmtKind::Repeat(count, Box::new(body))
                }
                K::Forever => {
                    self.bump();
                    let body = self.parse_stmt_or_null()?;
                    StmtKind::Forever(Box::new(body))
                }
                K::Foreach => StmtKind::Foreach(self.parse_foreach()?),
                K::Break => {
                    self.bump();
                    self.expect_semi()?;
                    StmtKind::Break
                }
                K::Continue => {
                    self.bump();
                    self.expect_semi()?;
                    StmtKind::Continue
                }
                K::Return => {
                    self.bump();
                    let value = if self.at_punct(Punct::Semi) {
                        None
                    } else {
                        Some(self.parse_expr()?)
                    };
                    self.expect_semi()?;
                    StmtKind::Return(value)
                }
                K::Disable => {
                    self.bump();
                    if self.eat_kw(K::Fork).is_some() {
                        self.expect_semi()?;
                        StmtKind::DisableFork
                    } else {
                        let target = self.parse_lvalue()?;
                        self.expect_semi()?;
                        StmtKind::Disable(target)
                    }
                }
                K::Wait => {
                    self.bump();
                    if self.eat_kw(K::Fork).is_some() {
                        self.expect_semi()?;
                        StmtKind::WaitFork
                    } else {
                        self.expect_punct(Punct::LParen)?;
                        let cond = self.parse_expr()?;
                        self.expect_punct(Punct::RParen)?;
                        let body = self.parse_stmt_or_null()?;
                        StmtKind::Wait(cond, Box::new(body))
                    }
                }
                K::Assign => {
                    self.bump();
                    let lhs = self.parse_lvalue()?;
                    self.expect_punct(Punct::Eq)?;
                    let rhs = self.parse_expr()?;
                    self.expect_semi()?;
                    StmtKind::ProcAssign(lhs, rhs)
                }
                K::Deassign => {
                    self.bump();
                    let lhs = self.parse_lvalue()?;
                    self.expect_semi()?;
                    StmtKind::Deassign(lhs)
                }
                K::Force => {
                    self.bump();
                    let lhs = self.parse_lvalue()?;
                    self.expect_punct(Punct::Eq)?;
                    let rhs = self.parse_expr()?;
                    self.expect_semi()?;
                    StmtKind::Force(lhs, rhs)
                }
                K::Release => {
                    self.bump();
                    let lhs = self.parse_lvalue()?;
                    self.expect_semi()?;
                    StmtKind::Release(lhs)
                }
                K::Assert | K::Assume | K::Cover | K::Restrict => {
                    StmtKind::Assert(self.parse_assertion(None)?)
                }
                K::Void | K::Signed | K::Unsigned | K::Const | K::New | K::This | K::Super => {
                    self.parse_expr_stmt()?
                }
                _ if self.at_data_type_keyword() => self.parse_expr_stmt()?,
                _ => return Err(self.expected("statement")),
            },
            TokenKind::Ident { .. }
            | TokenKind::EscapedIdent { .. }
            | TokenKind::SystemIdent { .. }
            | TokenKind::Punct(
                Punct::LBrace
                | Punct::ApostropheBrace
                | Punct::PlusPlus
                | Punct::MinusMinus
                | Punct::LParen,
            ) => self.parse_expr_stmt()?,
            _ => return Err(self.expected("statement")),
        };
        Ok(kind)
    }

    /// An assignment, call or other expression statement, up to and
    /// including its `;`.
    fn parse_expr_stmt(&mut self) -> PResult<StmtKind> {
        let lhs = if self.at_punct(Punct::PlusPlus) || self.at_punct(Punct::MinusMinus) {
            self.parse_expr()?
        } else {
            self.parse_lvalue()?
        };
        if let Some(op) = assign_op(self.kind()) {
            self.bump();
            let timing = self.parse_timing_control_opt()?;
            let rhs = self.parse_expr()?;
            self.expect_semi()?;
            return Ok(StmtKind::Assign(Box::new(Assign {
                lhs,
                op,
                timing,
                rhs,
            })));
        }
        self.expect_semi()?;
        Ok(StmtKind::Expr(lhs))
    }

    /// True when a declaration starts here: a declaration keyword, a
    /// data type keyword, or `name name` / `name [..] name` /
    /// `pkg::name name`.
    pub(super) fn looks_like_decl(&self) -> bool {
        use Keyword as K;
        match self.kind() {
            TokenKind::Keyword(kw) => match kw {
                K::Var
                | K::Const
                | K::Automatic
                | K::Static
                | K::Parameter
                | K::Localparam
                | K::Typedef
                | K::Import
                | K::Input
                | K::Output
                | K::Inout
                | K::Ref
                | K::Genvar => true,
                // `void'(f())` and `int'(x)` are casts, not declarations.
                K::Void => false,
                _ => {
                    self.at_data_type_keyword()
                        && !(self.nth_is_punct(1, Punct::Apostrophe)
                            || self.nth_is_punct(1, Punct::ApostropheBrace))
                }
            },
            _ => self.named_type_ahead(),
        }
    }

    /// A declaration in statement position, wrapped as [`StmtKind::Decl`].
    fn parse_decl_stmt(&mut self) -> PResult<StmtKind> {
        let start = self.span();
        let kind = if self.at_ident() {
            ItemKind::Var(self.parse_var_decl()?)
        } else {
            match self.parse_item_kind()? {
                Some(kind) => kind,
                None => return Err(self.expected("declaration")),
            }
        };
        Ok(StmtKind::Decl(Box::new(Item {
            attrs: Vec::new(),
            kind,
            span: self.span_from(start),
        })))
    }

    /// `begin [: name] stmts end [: name]`, cursor on `begin`.
    pub(super) fn parse_block(&mut self) -> PResult<Block> {
        let start = self.expect_kw(Keyword::Begin)?;
        let label = self.parse_block_label()?;
        let stmts = self.parse_stmts_until(&[Keyword::End]);
        self.expect_end(Keyword::End);
        Ok(Block {
            label,
            stmts,
            span: self.span_from(start),
        })
    }

    /// `fork [: name] stmts join* [: name]`, cursor on `fork`.
    fn parse_fork(&mut self) -> PResult<(Block, JoinKind)> {
        let start = self.expect_kw(Keyword::Fork)?;
        let label = self.parse_block_label()?;
        let stmts = self.parse_stmts_until(&[Keyword::Join, Keyword::JoinAny, Keyword::JoinNone]);
        let join = if self.eat_kw(Keyword::Join).is_some() {
            JoinKind::All
        } else if self.eat_kw(Keyword::JoinAny).is_some() {
            JoinKind::Any
        } else if self.eat_kw(Keyword::JoinNone).is_some() {
            JoinKind::None
        } else {
            // Reported, but the fork is kept like an unclosed `begin`.
            self.expected("`join`, `join_any` or `join_none`");
            JoinKind::All
        };
        self.eat_end_label();
        Ok((
            Block {
                label,
                stmts,
                span: self.span_from(start),
            },
            join,
        ))
    }

    /// The `: name` after `begin` or `fork`.
    fn parse_block_label(&mut self) -> PResult<Option<Ident>> {
        if self.eat_punct(Punct::Colon).is_some() {
            Ok(Some(self.expect_ident()?))
        } else {
            Ok(None)
        }
    }

    /// `if (cond) stmt [else stmt]`, cursor on `if`.
    fn parse_if(&mut self, qualifier: Option<Qualifier>) -> PResult<If> {
        self.expect_kw(Keyword::If)?;
        self.expect_punct(Punct::LParen)?;
        let cond = self.parse_expr()?;
        self.expect_punct(Punct::RParen)?;
        let then_stmt = Box::new(self.parse_stmt_or_null()?);
        let else_stmt = if self.eat_kw(Keyword::Else).is_some() {
            Some(Box::new(self.parse_stmt_or_null()?))
        } else {
            None
        };
        Ok(If {
            qualifier,
            cond,
            then_stmt,
            else_stmt,
        })
    }

    /// `case (expr) [inside] items endcase`, cursor on the case keyword.
    fn parse_case(&mut self, qualifier: Option<Qualifier>) -> PResult<Case> {
        let kind = match self.kind() {
            TokenKind::Keyword(Keyword::Casez) => CaseKind::Casez,
            TokenKind::Keyword(Keyword::Casex) => CaseKind::Casex,
            TokenKind::Keyword(Keyword::Case) => CaseKind::Case,
            _ => return Err(self.expected("`case`")),
        };
        self.bump();
        self.expect_punct(Punct::LParen)?;
        let expr = self.parse_expr()?;
        self.expect_punct(Punct::RParen)?;
        let inside = self.eat_kw(Keyword::Inside).is_some();
        let mut items = Vec::new();
        while !self.at_kw(Keyword::Endcase) && !self.at_eof() {
            let before = self.pos;
            match self.parse_case_item() {
                Ok(item) => items.push(item),
                Err(_) => {
                    self.recover_stmt();
                    if self.pos == before {
                        self.bump();
                    }
                }
            }
            if ends_stmt_list(self.kind()) && !self.at_kw(Keyword::Endcase) {
                break;
            }
        }
        self.expect_end(Keyword::Endcase);
        Ok(Case {
            qualifier,
            kind,
            inside,
            expr,
            items,
        })
    }

    /// `default [:] stmt` or `a, b: stmt`.
    fn parse_case_item(&mut self) -> PResult<CaseItem> {
        let start = self.span();
        let patterns = if self.eat_kw(Keyword::Default).is_some() {
            self.eat_punct(Punct::Colon);
            Vec::new()
        } else {
            let mut patterns = vec![self.parse_expr()?];
            while self.eat_punct(Punct::Comma).is_some() {
                patterns.push(self.parse_expr()?);
            }
            self.expect_punct(Punct::Colon)?;
            patterns
        };
        let body = self.parse_stmt_or_null()?;
        Ok(CaseItem {
            patterns,
            body,
            span: self.span_from(start),
        })
    }

    /// `for (init; cond; step) body`, cursor on `for`.
    fn parse_for(&mut self) -> PResult<For> {
        self.expect_kw(Keyword::For)?;
        self.expect_punct(Punct::LParen)?;
        let mut init: Vec<ForInit> = Vec::new();
        if !self.at_punct(Punct::Semi) {
            loop {
                if self.at_kw(Keyword::Var)
                    || self.at_data_type_keyword()
                    || self.named_type_ahead()
                {
                    let var = self.eat_kw(Keyword::Var).is_some();
                    let data_type = self.parse_data_type()?;
                    let decl = self.parse_for_declarator()?;
                    init.push(ForInit::Decl(VarDecl {
                        lifetime: None,
                        constant: false,
                        var,
                        data_type,
                        decls: vec![decl],
                    }));
                } else if let Some(ForInit::Decl(prev)) = init.last_mut() {
                    // `int i = 0, j = 0`: another variable of the same type.
                    let decl = self.parse_for_declarator()?;
                    prev.decls.push(decl);
                } else {
                    let lhs = self.parse_lvalue()?;
                    let Some(op) = assign_op(self.kind()) else {
                        return Err(self.expected("`=`"));
                    };
                    self.bump();
                    let rhs = self.parse_expr()?;
                    let span = lhs.span.to(rhs.span);
                    init.push(ForInit::Assign(Expr::new(
                        ExprKind::Assign {
                            lhs: Box::new(lhs),
                            op,
                            rhs: Box::new(rhs),
                        },
                        span,
                    )));
                }
                if self.eat_punct(Punct::Comma).is_none() {
                    break;
                }
            }
        }
        self.expect_semi()?;
        let cond = if self.at_punct(Punct::Semi) {
            None
        } else {
            Some(self.parse_expr()?)
        };
        self.expect_semi()?;
        let mut step = Vec::new();
        if !self.at_punct(Punct::RParen) {
            loop {
                step.push(self.parse_expr_or_assign()?);
                if self.eat_punct(Punct::Comma).is_none() {
                    break;
                }
            }
        }
        self.expect_punct(Punct::RParen)?;
        let body = Box::new(self.parse_stmt_or_null()?);
        Ok(For {
            init,
            cond,
            step,
            body,
        })
    }

    /// One `name [dims] = value` of a `for` variable declaration, where
    /// the value is mandatory.
    fn parse_for_declarator(&mut self) -> PResult<super::super::ast::Declarator> {
        let name = self.expect_ident()?;
        let dims = self.parse_dims()?;
        self.expect_punct(Punct::Eq)?;
        let init = Some(self.parse_expr()?);
        let span = self.span_from(name.span);
        Ok(super::super::ast::Declarator {
            name,
            dims,
            init,
            span,
        })
    }

    /// `foreach (array[i, j]) body`, cursor on `foreach`.
    ///
    /// The loop variables live in the last bracket group before `)`, so
    /// that group is located first and the array expression is parsed
    /// with the cursor limited to the tokens before it.
    fn parse_foreach(&mut self) -> PResult<Foreach> {
        self.expect_kw(Keyword::Foreach)?;
        let open = self.pos;
        self.expect_punct(Punct::LParen)?;
        let Some(close) = self.matching_close(open) else {
            return Err(self.expected("`)`"));
        };
        // Walk back from the `]` before `)` to its `[`.
        let mut vars_open = None;
        if close > open + 1 && self.kind_at(close - 1).is_punct(Punct::RBracket) {
            let mut depth = 0usize;
            let mut i = close - 1;
            while i > open {
                match self.kind_at(i) {
                    TokenKind::Punct(Punct::RBracket) => depth += 1,
                    TokenKind::Punct(Punct::LBracket) => {
                        depth -= 1;
                        if depth == 0 {
                            vars_open = Some(i);
                            break;
                        }
                    }
                    _ => {}
                }
                i -= 1;
            }
        }
        let Some(vars_open) = vars_open else {
            return Err(self.expected("`array[vars]`"));
        };
        let array = self.with_limit(vars_open, |p| p.parse_expr())?;
        if self.pos != vars_open {
            return Err(self.expected("`[`"));
        }
        self.expect_punct(Punct::LBracket)?;
        let mut vars = Vec::new();
        loop {
            if self.at_punct(Punct::Comma) || self.at_punct(Punct::RBracket) {
                vars.push(None);
            } else {
                vars.push(Some(self.expect_ident()?));
            }
            if self.eat_punct(Punct::Comma).is_none() {
                break;
            }
        }
        self.expect_punct(Punct::RBracket)?;
        self.expect_punct(Punct::RParen)?;
        let body = Box::new(self.parse_stmt_or_null()?);
        Ok(Foreach { array, vars, body })
    }

    /// An intra-assignment `#delay`, `@event` or `repeat (n) @event`, if
    /// present.
    fn parse_timing_control_opt(&mut self) -> PResult<Option<TimingControl>> {
        let start = self.span();
        let kind = if self.at_punct(Punct::Hash) {
            TimingKind::Delay(self.parse_delay()?)
        } else if self.at_punct(Punct::At) {
            TimingKind::Event(self.parse_event_control()?)
        } else if self.at_kw(Keyword::Repeat) {
            self.bump();
            self.expect_punct(Punct::LParen)?;
            let count = self.parse_expr()?;
            self.expect_punct(Punct::RParen)?;
            TimingKind::RepeatEvent(count, self.parse_event_control()?)
        } else {
            return Ok(None);
        };
        Ok(Some(TimingControl {
            kind,
            span: self.span_from(start),
        }))
    }

    /// `@*`, `@(*)`, `@(a or posedge b, negedge c iff d)`, `@name`,
    /// cursor on `@`.
    pub(super) fn parse_event_control(&mut self) -> PResult<EventControl> {
        let start = self.expect_punct(Punct::At)?;
        let kind = if self.eat_punct(Punct::Star).is_some() {
            EventControlKind::Any
        } else if self.at_punct(Punct::LParen) {
            self.bump();
            if self.at_punct(Punct::Star) && self.nth_is_punct(1, Punct::RParen) {
                self.bump();
                self.bump();
                EventControlKind::Any
            } else {
                let mut events = vec![self.parse_event_expr()?];
                while self.eat_punct(Punct::Comma).is_some() || self.eat_kw(Keyword::Or).is_some() {
                    events.push(self.parse_event_expr()?);
                }
                self.expect_punct(Punct::RParen)?;
                EventControlKind::List(events)
            }
        } else if self.at_ident() {
            let expr = self.parse_lvalue()?;
            let span = expr.span;
            EventControlKind::List(vec![EventExpr {
                edge: None,
                expr,
                iff: None,
                span,
            }])
        } else {
            return Err(self.expected("event expression"));
        };
        Ok(EventControl {
            kind,
            span: self.span_from(start),
        })
    }

    /// `[posedge|negedge|edge] expr [iff cond]`.
    fn parse_event_expr(&mut self) -> PResult<EventExpr> {
        let start = self.span();
        let edge = if self.eat_kw(Keyword::Posedge).is_some() {
            Some(Edge::Posedge)
        } else if self.eat_kw(Keyword::Negedge).is_some() {
            Some(Edge::Negedge)
        } else if self.eat_kw(Keyword::Edge).is_some() {
            Some(Edge::Edge)
        } else {
            None
        };
        let expr = self.parse_expr()?;
        let iff = if self.eat_kw(Keyword::Iff).is_some() {
            Some(self.parse_expr()?)
        } else {
            None
        };
        Ok(EventExpr {
            edge,
            expr,
            iff,
            span: self.span_from(start),
        })
    }

    /// `assert`, `assume`, `cover` or `restrict`, immediate or
    /// `property`/`sequence`, with its action block; cursor on the
    /// keyword. `label` is the `name :` an item-level assertion had.
    pub(super) fn parse_assertion(&mut self, label: Option<Ident>) -> PResult<Assertion> {
        let kind = match self.kind() {
            TokenKind::Keyword(Keyword::Assert) => AssertKind::Assert,
            TokenKind::Keyword(Keyword::Assume) => AssertKind::Assume,
            TokenKind::Keyword(Keyword::Cover) => AssertKind::Cover,
            TokenKind::Keyword(Keyword::Restrict) => AssertKind::Restrict,
            _ => return Err(self.expected("`assert`")),
        };
        self.bump();
        let mut deferred = None;
        let spec = if self.at_kw(Keyword::Property) || self.at_kw(Keyword::Sequence) {
            self.bump();
            let open = self.pos;
            self.expect_punct(Punct::LParen)?;
            let Some(close) = self.matching_close(open) else {
                return Err(self.expected("`)`"));
            };
            let raw = self.raw_tokens(open + 1, close);
            while self.pos < close {
                self.bump();
            }
            self.expect_punct(Punct::RParen)?;
            AssertSpec::Property(raw)
        } else {
            if self.at_punct(Punct::Hash) {
                self.bump();
                match self.kind() {
                    TokenKind::Number { text } if text == "0" => {
                        self.bump();
                        deferred = Some(Deferred::Observed);
                    }
                    _ => return Err(self.expected("`0`")),
                }
            } else if self.eat_kw(Keyword::Final).is_some() {
                deferred = Some(Deferred::Final);
            }
            self.expect_punct(Punct::LParen)?;
            let expr = self.parse_expr()?;
            self.expect_punct(Punct::RParen)?;
            AssertSpec::Expr(expr)
        };
        let (then_stmt, else_stmt) = if self.eat_punct(Punct::Semi).is_some() {
            (None, None)
        } else if self.eat_kw(Keyword::Else).is_some() {
            (None, Some(Box::new(self.parse_stmt_or_null()?)))
        } else {
            let then_stmt = Box::new(self.parse_stmt_or_null()?);
            let else_stmt = if self.eat_kw(Keyword::Else).is_some() {
                Some(Box::new(self.parse_stmt_or_null()?))
            } else {
                None
            };
            (Some(then_stmt), else_stmt)
        };
        Ok(Assertion {
            label,
            kind,
            deferred,
            spec,
            then_stmt,
            else_stmt,
        })
    }

    /// A delay used as a standalone item prefix (`assign #1 ...`,
    /// gates), if present.
    pub(super) fn parse_delay_opt(&mut self) -> PResult<Option<Delay>> {
        if self.at_punct(Punct::Hash) {
            Ok(Some(self.parse_delay()?))
        } else {
            Ok(None)
        }
    }
}
