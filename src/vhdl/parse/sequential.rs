//! Sequential statements.

use super::{PResult, Parser, Recover, is_sequential_keyword};
use crate::source::Span;
use crate::vhdl::ast::*;
use crate::vhdl::token::TokenKind;

/// Which keywords legitimately end the statement list being parsed.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum SeqContext {
    /// Only `end`.
    Plain,
    /// `end`, `elsif` and `else`.
    If,
    /// `end` and `when`.
    Case,
}

impl<'t, 'src> Parser<'t, 'src> {
    /// Sequential statements up to `end` (and the context's arm keywords),
    /// recovering after each failed one.
    pub(super) fn parse_sequential_statements(&mut self) -> Vec<SequentialStatement> {
        self.parse_sequential_statements_in(SeqContext::Plain)
    }

    pub(super) fn parse_sequential_statements_in(
        &mut self,
        ctx: SeqContext,
    ) -> Vec<SequentialStatement> {
        let mut stmts = Vec::new();
        loop {
            let k = self.kind();
            let ends = match k {
                TokenKind::End => {
                    if self.consume_stray_end() {
                        continue;
                    }
                    true
                }
                TokenKind::Eof => true,
                TokenKind::Elsif | TokenKind::Else => ctx == SeqContext::If,
                TokenKind::When => ctx == SeqContext::Case,
                _ => false,
            };
            if ends {
                break;
            }
            let start = self.pos;
            match self.parse_sequential_statement() {
                Ok(s) => stmts.push(s),
                Err(Recover) => {
                    self.recover(start, |k| is_sequential_keyword(k) || k == TokenKind::End)
                }
            }
        }
        stmts
    }

    /// One sequential statement, `;` included.
    fn parse_sequential_statement(&mut self) -> PResult<SequentialStatement> {
        let start = self.span();
        let label = self.parse_optional_label()?;
        let kind = match self.kind() {
            TokenKind::Wait => self.parse_wait()?,
            TokenKind::Assert => SequentialKind::Assertion(self.parse_assertion()?),
            TokenKind::Report => self.parse_report()?,
            TokenKind::If => SequentialKind::If(self.parse_if(label.as_ref())?),
            TokenKind::Case => SequentialKind::Case(self.parse_case(label.as_ref())?),
            TokenKind::For | TokenKind::While | TokenKind::Loop => {
                SequentialKind::Loop(self.parse_loop(label.as_ref())?)
            }
            TokenKind::Next | TokenKind::Exit => self.parse_next_exit()?,
            TokenKind::Return => {
                self.bump();
                let value = if self.at(TokenKind::Semi) {
                    None
                } else {
                    Some(self.parse_expr()?)
                };
                SequentialKind::Return(value)
            }
            TokenKind::Null => {
                self.bump();
                SequentialKind::Null
            }
            TokenKind::With => self.parse_selected_assignment()?,
            TokenKind::Elsif | TokenKind::Else | TokenKind::When => {
                let msg = format!("{} outside of the statement it belongs to", self.kind());
                self.error_here(msg);
                return Err(Recover);
            }
            _ => self.parse_assignment_or_call()?,
        };
        if !matches!(
            kind,
            SequentialKind::If(_) | SequentialKind::Case(_) | SequentialKind::Loop(_)
        ) {
            self.expect_semi()?;
        }
        Ok(SequentialStatement {
            label,
            kind,
            span: self.span_from(start),
        })
    }

    /// `wait [on names] [until cond] [for time]`
    fn parse_wait(&mut self) -> PResult<SequentialKind> {
        self.expect(TokenKind::Wait)?;
        let sensitivity = if self.eat(TokenKind::On).is_some() {
            Some(self.parse_sensitivity_names()?)
        } else {
            None
        };
        let condition = if self.eat(TokenKind::Until).is_some() {
            Some(self.parse_expr()?)
        } else {
            None
        };
        let timeout = if self.eat(TokenKind::For).is_some() {
            Some(self.parse_expr()?)
        } else {
            None
        };
        Ok(SequentialKind::Wait {
            sensitivity,
            condition,
            timeout,
        })
    }

    /// `name {, name}` of a sensitivity clause.
    fn parse_sensitivity_names(&mut self) -> PResult<Sensitivity> {
        let mut names = vec![self.parse_name()?];
        while self.eat(TokenKind::Comma).is_some() {
            names.push(self.parse_name()?);
        }
        Ok(Sensitivity::Names(names))
    }

    /// `assert cond [report expr] [severity expr]`
    pub(super) fn parse_assertion(&mut self) -> PResult<Assertion> {
        let start = self.span();
        self.expect(TokenKind::Assert)?;
        let condition = self.parse_expr()?;
        let report = if self.eat(TokenKind::Report).is_some() {
            Some(self.parse_expr()?)
        } else {
            None
        };
        let severity = if self.eat(TokenKind::Severity).is_some() {
            Some(self.parse_expr()?)
        } else {
            None
        };
        Ok(Assertion {
            condition,
            report,
            severity,
            span: self.span_from(start),
        })
    }

    /// `report expr [severity expr]`
    fn parse_report(&mut self) -> PResult<SequentialKind> {
        self.expect(TokenKind::Report)?;
        let message = self.parse_expr()?;
        let severity = if self.eat(TokenKind::Severity).is_some() {
            Some(self.parse_expr()?)
        } else {
            None
        };
        Ok(SequentialKind::Report { message, severity })
    }

    /// `if cond then stmts {elsif cond then stmts} [else stmts] end if [label];`
    fn parse_if(&mut self, label: Option<&Ident>) -> PResult<IfStatement> {
        let start = self.span();
        self.expect(TokenKind::If)?;
        self.with_open(TokenKind::If, |p| {
            let mut arms = Vec::new();
            let mut else_statements = None;
            let mut arm_start = start;
            loop {
                let condition = p.parse_condition_until(TokenKind::Then)?;
                p.expect_or_skip_to(TokenKind::Then)?;
                let statements = p.parse_sequential_statements_in(SeqContext::If);
                arms.push(IfArm {
                    condition,
                    statements,
                    span: p.span_from(arm_start),
                });
                match p.kind() {
                    TokenKind::Elsif => {
                        arm_start = p.bump().span;
                    }
                    TokenKind::Else => {
                        p.bump();
                        else_statements = Some(p.parse_sequential_statements_in(SeqContext::Plain));
                        break;
                    }
                    _ => break,
                }
            }
            p.parse_end(&[TokenKind::If], false, label)?;
            Ok(IfStatement {
                arms,
                else_statements,
                span: p.span_from(start),
            })
        })
    }

    /// `case[?] expr is {when choices => stmts} end case[?] [label];`
    fn parse_case(&mut self, label: Option<&Ident>) -> PResult<CaseStatement> {
        let start = self.span();
        self.expect(TokenKind::Case)?;
        let matching = if let Some(t) = self.eat(TokenKind::Question) {
            self.require_2008(t.span, "`case?`");
            true
        } else {
            false
        };
        self.with_open(TokenKind::Case, |p| {
            let expr = p.parse_condition_until(TokenKind::Is)?;
            p.expect_or_skip_to(TokenKind::Is)?;
            let mut arms = Vec::new();
            while p.at(TokenKind::When) {
                let arm_start = p.bump().span;
                let choices = p.parse_choices()?;
                p.expect_or_skip_to(TokenKind::Arrow)?;
                let statements = p.parse_sequential_statements_in(SeqContext::Case);
                arms.push(CaseArm {
                    choices,
                    statements,
                    span: p.span_from(arm_start),
                });
            }
            if arms.is_empty() {
                p.error_here("a case statement needs at least one `when` alternative");
            }
            let kws: &[TokenKind] = if matching {
                &[TokenKind::Case, TokenKind::Question]
            } else {
                &[TokenKind::Case]
            };
            p.parse_end(kws, false, label)?;
            Ok(CaseStatement {
                expr,
                matching,
                arms,
                span: p.span_from(start),
            })
        })
    }

    /// `[while cond | for i in range] loop stmts end loop [label];`
    fn parse_loop(&mut self, label: Option<&Ident>) -> PResult<LoopStatement> {
        let start = self.span();
        let scheme = match self.kind() {
            TokenKind::While => {
                self.bump();
                Some(IterationScheme::While(
                    self.parse_condition_until(TokenKind::Loop)?,
                ))
            }
            TokenKind::For => {
                self.bump();
                let param = self.parse_ident()?;
                self.expect(TokenKind::In)?;
                let range = self.parse_discrete_range()?;
                Some(IterationScheme::For { param, range })
            }
            _ => None,
        };
        self.expect_or_skip_to(TokenKind::Loop)?;
        self.with_open(TokenKind::Loop, |p| {
            let statements = p.parse_sequential_statements_in(SeqContext::Plain);
            p.parse_end(&[TokenKind::Loop], false, label)?;
            Ok(LoopStatement {
                scheme,
                statements,
                span: p.span_from(start),
            })
        })
    }

    /// `next|exit [label] [when cond]`
    fn parse_next_exit(&mut self) -> PResult<SequentialKind> {
        let is_next = self.bump().kind == TokenKind::Next;
        let label = if self.at_any(&[TokenKind::Ident, TokenKind::ExtendedIdent]) {
            Some(self.parse_ident()?)
        } else {
            None
        };
        let condition = if self.eat(TokenKind::When).is_some() {
            Some(self.parse_expr()?)
        } else {
            None
        };
        Ok(if is_next {
            SequentialKind::Next { label, condition }
        } else {
            SequentialKind::Exit { label, condition }
        })
    }

    /// A signal assignment, variable assignment or procedure call, told
    /// apart by what follows the target.
    fn parse_assignment_or_call(&mut self) -> PResult<SequentialKind> {
        let start = self.span();
        let target = self.parse_target()?;
        match self.kind() {
            TokenKind::Le => {
                self.bump();
                let (_, assignment) = self.parse_signal_assignment_rhs(target, start, false)?;
                Ok(SequentialKind::SignalAssignment(assignment))
            }
            TokenKind::ColonEq => {
                self.bump();
                let rhs = self.parse_variable_assignment_rhs()?;
                Ok(SequentialKind::VariableAssignment(VariableAssignment {
                    target,
                    rhs,
                    span: self.span_from(start),
                }))
            }
            TokenKind::Semi => match target {
                Target::Name(n) => Ok(SequentialKind::ProcedureCall(n)),
                Target::Aggregate(_) => Err(self.expected("`<=` or `:=`")),
            },
            _ => Err(self.expected("`<=`, `:=` or `;`")),
        }
    }

    /// A name or an aggregate.
    pub(super) fn parse_target(&mut self) -> PResult<Target> {
        if self.at(TokenKind::LParen) {
            Ok(Target::Aggregate(self.parse_aggregate()?))
        } else if self.at_name_start() {
            Ok(Target::Name(self.parse_name()?))
        } else {
            Err(self.expected("a statement"))
        }
    }

    /// The right-hand side of a variable assignment: an expression or a
    /// VHDL-2008 conditional expression.
    fn parse_variable_assignment_rhs(&mut self) -> PResult<VariableAssignmentRhs> {
        let arms = self.parse_conditional_exprs()?;
        if arms.len() == 1 && arms[0].condition.is_none() {
            let arm = arms.into_iter().next().unwrap_or_else(|| unreachable!());
            Ok(VariableAssignmentRhs::Simple(arm.value))
        } else {
            self.require_2008(arms[0].span, "conditional variable assignments");
            Ok(VariableAssignmentRhs::Conditional(arms))
        }
    }

    /// `expr [when cond {else expr when cond} [else expr]]`
    pub(super) fn parse_conditional_exprs(&mut self) -> PResult<Vec<ConditionalExpr>> {
        let mut arms = Vec::new();
        loop {
            let start = self.span();
            let value = self.parse_expr()?;
            let condition = if self.eat(TokenKind::When).is_some() {
                Some(self.parse_expr()?)
            } else {
                None
            };
            let last = condition.is_none();
            arms.push(ConditionalExpr {
                value,
                condition,
                span: self.span_from(start),
            });
            if last || self.eat(TokenKind::Else).is_none() {
                break;
            }
        }
        Ok(arms)
    }

    /// The part of a signal assignment after `<=`: `[guarded] [delay]
    /// waveform [when ...]`, `force ...` or `release ...`. Returns whether
    /// `guarded` was present.
    pub(super) fn parse_signal_assignment_rhs(
        &mut self,
        target: Target,
        start: Span,
        allow_guarded: bool,
    ) -> PResult<(bool, SignalAssignment)> {
        let guarded = allow_guarded && self.eat(TokenKind::Guarded).is_some();
        let rhs = match self.kind() {
            TokenKind::Force => {
                let t = self.bump();
                self.require_2008(t.span, "`force`");
                let mode = self.parse_force_mode();
                let arms = self.parse_conditional_exprs()?;
                SignalAssignmentRhs::Force { mode, arms }
            }
            TokenKind::Release => {
                let t = self.bump();
                self.require_2008(t.span, "`release`");
                let mode = self.parse_force_mode();
                SignalAssignmentRhs::Release { mode }
            }
            _ => {
                let delay = self.parse_optional_delay()?;
                let waveform = self.parse_waveform()?;
                if self.at(TokenKind::When) {
                    let arms = self.parse_conditional_waveforms(waveform, start)?;
                    let assignment = SignalAssignment {
                        target,
                        delay,
                        rhs: SignalAssignmentRhs::Conditional(arms),
                        span: self.span_from(start),
                    };
                    return Ok((guarded, assignment));
                }
                let assignment = SignalAssignment {
                    target,
                    delay,
                    rhs: SignalAssignmentRhs::Simple(waveform),
                    span: self.span_from(start),
                };
                return Ok((guarded, assignment));
            }
        };
        Ok((
            guarded,
            SignalAssignment {
                target,
                delay: None,
                rhs,
                span: self.span_from(start),
            },
        ))
    }

    /// `[in | out]` after `force` or `release`.
    fn parse_force_mode(&mut self) -> Option<Mode> {
        match self.kind() {
            TokenKind::In => {
                self.bump();
                Some(Mode::In)
            }
            TokenKind::Out => {
                self.bump();
                Some(Mode::Out)
            }
            _ => None,
        }
    }

    /// `[transport | [reject expr] inertial]`
    fn parse_optional_delay(&mut self) -> PResult<Option<DelayMechanism>> {
        let start = self.span();
        match self.kind() {
            TokenKind::Transport => Ok(Some(DelayMechanism::Transport(self.bump().span))),
            TokenKind::Reject => {
                self.bump();
                let reject = Some(self.parse_expr()?);
                self.expect(TokenKind::Inertial)?;
                Ok(Some(DelayMechanism::Inertial {
                    reject,
                    span: self.span_from(start),
                }))
            }
            TokenKind::Inertial => {
                self.bump();
                Ok(Some(DelayMechanism::Inertial {
                    reject: None,
                    span: self.span_from(start),
                }))
            }
            _ => Ok(None),
        }
    }

    /// `unaffected | element {, element}`
    pub(super) fn parse_waveform(&mut self) -> PResult<Waveform> {
        if let Some(t) = self.eat(TokenKind::Unaffected) {
            return Ok(Waveform::Unaffected(t.span));
        }
        let mut elements = vec![self.parse_waveform_element()?];
        while self.eat(TokenKind::Comma).is_some() {
            elements.push(self.parse_waveform_element()?);
        }
        Ok(Waveform::Elements(elements))
    }

    /// `expr [after expr]`
    fn parse_waveform_element(&mut self) -> PResult<WaveformElement> {
        let start = self.span();
        let value = self.parse_expr()?;
        let after = if self.eat(TokenKind::After).is_some() {
            Some(self.parse_expr()?)
        } else {
            None
        };
        Ok(WaveformElement {
            value,
            after,
            span: self.span_from(start),
        })
    }

    /// The arms of a conditional waveform, the first waveform having been
    /// parsed and the cursor being at `when`.
    fn parse_conditional_waveforms(
        &mut self,
        first: Waveform,
        start: Span,
    ) -> PResult<Vec<ConditionalWaveform>> {
        let mut arms = Vec::new();
        let mut waveform = first;
        let mut arm_start = start;
        loop {
            let condition = if self.eat(TokenKind::When).is_some() {
                Some(self.parse_expr()?)
            } else {
                None
            };
            let last = condition.is_none();
            arms.push(ConditionalWaveform {
                waveform,
                condition,
                span: self.span_from(arm_start),
            });
            if last || self.eat(TokenKind::Else).is_none() {
                break;
            }
            arm_start = self.span();
            waveform = self.parse_waveform()?;
        }
        Ok(arms)
    }

    /// `with expr select [?] target <=|:= ... when choices {, ...}` in a
    /// sequential context (VHDL-2008).
    fn parse_selected_assignment(&mut self) -> PResult<SequentialKind> {
        let start = self.span();
        let (selector, matching) = self.parse_selector()?;
        let target_start = self.span();
        let target = self.parse_target()?;
        match self.kind() {
            TokenKind::Le => {
                self.bump();
                let (_, assignment) = self.parse_selected_signal_rhs(
                    target,
                    target_start,
                    selector,
                    matching,
                    false,
                )?;
                self.require_2008(start, "selected signal assignments in sequential code");
                Ok(SequentialKind::SignalAssignment(assignment))
            }
            TokenKind::ColonEq => {
                self.bump();
                self.require_2008(start, "selected variable assignments");
                let mut arms = Vec::new();
                loop {
                    let arm_start = self.span();
                    let value = self.parse_expr()?;
                    self.expect(TokenKind::When)?;
                    let choices = self.parse_choices()?;
                    arms.push(SelectedExpr {
                        value,
                        choices,
                        span: self.span_from(arm_start),
                    });
                    if self.eat(TokenKind::Comma).is_none() {
                        break;
                    }
                }
                Ok(SequentialKind::VariableAssignment(VariableAssignment {
                    target,
                    rhs: VariableAssignmentRhs::Selected {
                        selector,
                        matching,
                        arms,
                    },
                    span: self.span_from(target_start),
                }))
            }
            _ => Err(self.expected("`<=` or `:=`")),
        }
    }

    /// `with expr select [?]`
    pub(super) fn parse_selector(&mut self) -> PResult<(Expr, bool)> {
        self.expect(TokenKind::With)?;
        let selector = self.parse_expr()?;
        self.expect_or_skip_to(TokenKind::Select)?;
        let matching = if let Some(t) = self.eat(TokenKind::Question) {
            self.require_2008(t.span, "`select?`");
            true
        } else {
            false
        };
        Ok((selector, matching))
    }

    /// The part of a selected signal assignment after `<=`.
    pub(super) fn parse_selected_signal_rhs(
        &mut self,
        target: Target,
        start: Span,
        selector: Expr,
        matching: bool,
        allow_guarded: bool,
    ) -> PResult<(bool, SignalAssignment)> {
        let guarded = allow_guarded && self.eat(TokenKind::Guarded).is_some();
        let delay = self.parse_optional_delay()?;
        let mut arms = Vec::new();
        loop {
            let arm_start = self.span();
            let waveform = self.parse_waveform()?;
            self.expect(TokenKind::When)?;
            let choices = self.parse_choices()?;
            arms.push(SelectedWaveform {
                waveform,
                choices,
                span: self.span_from(arm_start),
            });
            if self.eat(TokenKind::Comma).is_none() {
                break;
            }
        }
        Ok((
            guarded,
            SignalAssignment {
                target,
                delay,
                rhs: SignalAssignmentRhs::Selected {
                    selector,
                    matching,
                    arms,
                },
                span: self.span_from(start),
            },
        ))
    }
}
