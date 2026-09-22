//! Concurrent statements: processes, blocks, concurrent assignments and
//! calls, instantiations and generates.

use super::{PResult, Parser, Recover, is_concurrent_keyword};
use crate::diag::Diagnostic;
use crate::vhdl::ast::*;
use crate::vhdl::token::TokenKind;

/// Which keywords legitimately end the statement list being parsed.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ConcContext {
    /// Only `end`.
    Plain,
    /// `end`, `elsif` and `else` (if-generate arms).
    IfGenerate,
    /// `end` and `when` (case-generate arms).
    CaseGenerate,
}

impl<'t, 'src> Parser<'t, 'src> {
    /// Concurrent statements up to `end`, recovering after each failed one.
    pub(super) fn parse_concurrent_statements(&mut self) -> Vec<ConcurrentStatement> {
        self.parse_concurrent_statements_in(ConcContext::Plain)
    }

    fn parse_concurrent_statements_in(&mut self, ctx: ConcContext) -> Vec<ConcurrentStatement> {
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
                TokenKind::Elsif | TokenKind::Else => ctx == ConcContext::IfGenerate,
                TokenKind::When => ctx == ConcContext::CaseGenerate,
                _ => false,
            };
            if ends {
                break;
            }
            let start = self.pos;
            match self.parse_concurrent_statement() {
                Ok(Some(s)) => stmts.push(s),
                Ok(None) => {}
                Err(Recover) => self.recover(start, |k| {
                    is_concurrent_keyword(k) || matches!(k, TokenKind::End | TokenKind::Begin)
                }),
            }
        }
        stmts
    }

    /// One concurrent statement, `;` included. Returns `None` for a PSL
    /// directive, which is reported and skipped.
    fn parse_concurrent_statement(&mut self) -> PResult<Option<ConcurrentStatement>> {
        let start = self.span();
        let label = self.parse_optional_label()?;
        let postponed = if self.at(TokenKind::Postponed) && self.kind_at(1) != TokenKind::Process {
            self.bump();
            true
        } else {
            false
        };
        let kind = match self.kind() {
            TokenKind::Process | TokenKind::Postponed => {
                ConcurrentKind::Process(self.parse_process(label.as_ref())?)
            }
            TokenKind::Block => ConcurrentKind::Block(self.parse_block(label.as_ref())?),
            TokenKind::For => {
                self.require_generate_label(label.as_ref());
                ConcurrentKind::ForGenerate(self.parse_for_generate(label.as_ref())?)
            }
            TokenKind::If => {
                self.require_generate_label(label.as_ref());
                ConcurrentKind::IfGenerate(self.parse_if_generate(label.as_ref())?)
            }
            TokenKind::Case => {
                self.require_generate_label(label.as_ref());
                ConcurrentKind::CaseGenerate(self.parse_case_generate(label.as_ref())?)
            }
            TokenKind::Assert if self.at_psl_assert() => return self.skip_psl(start),
            TokenKind::Assume
            | TokenKind::AssumeGuarantee
            | TokenKind::Cover
            | TokenKind::Restrict
            | TokenKind::RestrictGuarantee
            | TokenKind::Fairness
            | TokenKind::Sequence
            | TokenKind::Property
            | TokenKind::Default
            | TokenKind::Vunit
            | TokenKind::Vprop
            | TokenKind::Vmode => return self.skip_psl(start),
            TokenKind::Assert => ConcurrentKind::Assertion {
                postponed,
                assertion: self.parse_assertion()?,
            },
            TokenKind::With => {
                let (selector, matching) = self.parse_selector()?;
                let target_start = self.span();
                let target = self.parse_target()?;
                self.expect(TokenKind::Le)?;
                let (guarded, assignment) =
                    self.parse_selected_signal_rhs(target, target_start, selector, matching, true)?;
                ConcurrentKind::SignalAssignment(ConcurrentSignalAssignment {
                    postponed,
                    guarded,
                    assignment,
                })
            }
            TokenKind::Component | TokenKind::Entity | TokenKind::Configuration => {
                ConcurrentKind::Instantiation(self.parse_instantiation(None)?)
            }
            TokenKind::LParen => {
                let target_start = self.span();
                let target = Target::Aggregate(self.parse_aggregate()?);
                self.expect(TokenKind::Le)?;
                let (guarded, assignment) =
                    self.parse_signal_assignment_rhs(target, target_start, true)?;
                ConcurrentKind::SignalAssignment(ConcurrentSignalAssignment {
                    postponed,
                    guarded,
                    assignment,
                })
            }
            _ if self.at_name_start() => {
                let target_start = self.span();
                let name = self.parse_name()?;
                match self.kind() {
                    TokenKind::Le => {
                        self.bump();
                        let (guarded, assignment) = self.parse_signal_assignment_rhs(
                            Target::Name(name),
                            target_start,
                            true,
                        )?;
                        ConcurrentKind::SignalAssignment(ConcurrentSignalAssignment {
                            postponed,
                            guarded,
                            assignment,
                        })
                    }
                    TokenKind::Generic | TokenKind::Port => {
                        ConcurrentKind::Instantiation(self.parse_instantiation(Some(name))?)
                    }
                    TokenKind::Semi
                        if label.is_some()
                            && !postponed
                            && matches!(name, Name::Simple(_) | Name::Selected { .. }) =>
                    {
                        ConcurrentKind::Instantiation(Instantiation {
                            span: name.span(),
                            unit: InstantiatedUnit::Component(name),
                            generic_map: None,
                            port_map: None,
                        })
                    }
                    TokenKind::Semi => ConcurrentKind::ProcedureCall {
                        postponed,
                        call: name,
                    },
                    TokenKind::ColonEq => {
                        self.error_here(
                            "`:=` is not a concurrent statement; use `<=` for a signal",
                        );
                        return Err(Recover);
                    }
                    _ => return Err(self.expected("`<=`, `port map`, `generic map` or `;`")),
                }
            }
            TokenKind::Elsif | TokenKind::Else | TokenKind::When => {
                let msg = format!("{} outside of a generate statement", self.kind());
                self.error_here(msg);
                return Err(Recover);
            }
            _ => return Err(self.expected("a concurrent statement")),
        };
        if !matches!(
            kind,
            ConcurrentKind::Process(_)
                | ConcurrentKind::Block(_)
                | ConcurrentKind::ForGenerate(_)
                | ConcurrentKind::IfGenerate(_)
                | ConcurrentKind::CaseGenerate(_)
        ) {
            self.expect_semi()?;
        }
        Ok(Some(ConcurrentStatement {
            label,
            kind,
            span: self.span_from(start),
        }))
    }

    /// True when `assert` starts a PSL directive: it is followed by a PSL
    /// temporal keyword, which VHDL lexes as an identifier.
    fn at_psl_assert(&self) -> bool {
        let t = &self.tokens[(self.pos + 1).min(self.tokens.len() - 1)];
        t.kind == TokenKind::Ident
            && matches!(
                t.text().to_ascii_lowercase().as_str(),
                "always" | "never" | "eventually!" | "eventually"
            )
    }

    /// Reports and skips a PSL directive up to its `;`.
    fn skip_psl(&mut self, start: crate::source::Span) -> PResult<Option<ConcurrentStatement>> {
        let kw = self.bump();
        self.report(
            Diagnostic::warning("PSL directives are not supported; skipped")
                .with_label(kw.span, "PSL directive starts here"),
        );
        while !self.at_any(&[TokenKind::Semi, TokenKind::Eof]) {
            self.bump();
        }
        self.eat(TokenKind::Semi);
        let _ = start;
        Ok(None)
    }

    /// Generate statements need a label; report when it is missing.
    fn require_generate_label(&mut self, label: Option<&Ident>) {
        if label.is_none() {
            let span = self.span();
            self.report(Diagnostic::error("a generate statement requires a label").with_span(span));
        }
    }

    /// `[postponed] process [(all | names)] [is] decls begin stmts end
    /// [postponed] process [label];`
    fn parse_process(&mut self, label: Option<&Ident>) -> PResult<ProcessStatement> {
        let start = self.span();
        let postponed = self.eat(TokenKind::Postponed).is_some();
        self.expect(TokenKind::Process)?;
        let sensitivity = if self.eat(TokenKind::LParen).is_some() {
            let s = if let Some(t) = self.eat(TokenKind::All) {
                self.require_2008(t.span, "`process (all)`");
                Sensitivity::All(t.span)
            } else {
                let mut names = vec![self.parse_name()?];
                while self.eat(TokenKind::Comma).is_some() {
                    names.push(self.parse_name()?);
                }
                Sensitivity::Names(names)
            };
            self.expect(TokenKind::RParen)?;
            Some(s)
        } else {
            None
        };
        self.eat(TokenKind::Is);
        self.with_open(TokenKind::Process, |p| {
            let decls = p.parse_declarative_part();
            p.expect_or_skip_to(TokenKind::Begin)?;
            let statements = p.parse_sequential_statements();
            p.parse_end(&[TokenKind::Process], false, label)?;
            Ok(ProcessStatement {
                postponed,
                sensitivity,
                decls,
                statements,
                span: p.span_from(start),
            })
        })
    }

    /// `block [(guard)] [is] [generic ...] [port ...] decls begin stmts end
    /// block [label];`
    fn parse_block(&mut self, label: Option<&Ident>) -> PResult<BlockStatement> {
        let start = self.span();
        self.expect(TokenKind::Block)?;
        let guard = if self.eat(TokenKind::LParen).is_some() {
            let g = self.parse_expr()?;
            self.expect(TokenKind::RParen)?;
            Some(g)
        } else {
            None
        };
        self.eat(TokenKind::Is);
        self.with_open(TokenKind::Block, |p| {
            let generics = p.parse_optional_generic_clause()?;
            let generic_map = if !generics.is_empty() && p.at(TokenKind::Generic) {
                let m = p.parse_generic_map_aspect()?;
                p.expect_semi()?;
                Some(m)
            } else {
                None
            };
            let ports = p.parse_optional_port_clause()?;
            let port_map = if !ports.is_empty() && p.at(TokenKind::Port) {
                let m = p.parse_port_map_aspect()?;
                p.expect_semi()?;
                Some(m)
            } else {
                None
            };
            let decls = p.parse_declarative_part();
            p.expect_or_skip_to(TokenKind::Begin)?;
            let statements = p.parse_concurrent_statements();
            p.parse_end(&[TokenKind::Block], false, label)?;
            Ok(BlockStatement {
                guard,
                generics,
                generic_map,
                ports,
                port_map,
                decls,
                statements,
                span: p.span_from(start),
            })
        })
    }

    /// `[component] name | entity name [(arch)] | configuration name`
    /// followed by the map aspects. `name` is given when the caller has
    /// already parsed a component name.
    fn parse_instantiation(&mut self, name: Option<Name>) -> PResult<Instantiation> {
        let start = name.as_ref().map_or(self.span(), Name::span);
        let unit = match name {
            Some(n) => InstantiatedUnit::Component(n),
            None => match self.kind() {
                TokenKind::Component => {
                    self.bump();
                    InstantiatedUnit::Component(self.parse_name()?)
                }
                TokenKind::Entity => {
                    self.bump();
                    let name = self.parse_name_no_call()?;
                    let architecture = if self.eat(TokenKind::LParen).is_some() {
                        let a = self.parse_ident()?;
                        self.expect(TokenKind::RParen)?;
                        Some(a)
                    } else {
                        None
                    };
                    InstantiatedUnit::Entity { name, architecture }
                }
                _ => {
                    self.expect(TokenKind::Configuration)?;
                    InstantiatedUnit::Configuration(self.parse_name()?)
                }
            },
        };
        let generic_map = if self.at(TokenKind::Generic) {
            Some(self.parse_generic_map_aspect()?)
        } else {
            None
        };
        let port_map = if self.at(TokenKind::Port) {
            Some(self.parse_port_map_aspect()?)
        } else {
            None
        };
        Ok(Instantiation {
            unit,
            generic_map,
            port_map,
            span: self.span_from(start),
        })
    }

    /// `for param in range generate body end generate [label];`
    fn parse_for_generate(&mut self, label: Option<&Ident>) -> PResult<ForGenerate> {
        let start = self.span();
        self.expect(TokenKind::For)?;
        let param = self.parse_ident()?;
        self.expect(TokenKind::In)?;
        let range = self.parse_discrete_range()?;
        self.expect_or_skip_to(TokenKind::Generate)?;
        self.with_open(TokenKind::Generate, |p| {
            let body = p.parse_generate_body(None, ConcContext::Plain)?;
            p.parse_end(&[TokenKind::Generate], false, label)?;
            Ok(ForGenerate {
                param,
                range,
                body,
                span: p.span_from(start),
            })
        })
    }

    /// `if [alt:] cond generate body {elsif [alt:] cond generate body}
    /// [else [alt:] generate body] end generate [label];`
    fn parse_if_generate(&mut self, label: Option<&Ident>) -> PResult<IfGenerate> {
        let start = self.span();
        self.expect(TokenKind::If)?;
        self.with_open(TokenKind::Generate, |p| {
            let mut arms = Vec::new();
            let mut else_arm = None;
            let mut arm_start = start;
            loop {
                let alt = p.parse_optional_label()?;
                let condition = p.parse_condition_until(TokenKind::Generate)?;
                p.expect_or_skip_to(TokenKind::Generate)?;
                let body = p.parse_generate_body(alt, ConcContext::IfGenerate)?;
                arms.push(IfGenerateArm {
                    condition,
                    body,
                    span: p.span_from(arm_start),
                });
                match p.kind() {
                    TokenKind::Elsif => {
                        arm_start = p.bump().span;
                        p.require_2008(arm_start, "`elsif` in generate statements");
                    }
                    TokenKind::Else => {
                        let t = p.bump();
                        p.require_2008(t.span, "`else` in generate statements");
                        let alt = p.parse_optional_label()?;
                        p.expect_or_skip_to(TokenKind::Generate)?;
                        else_arm = Some(p.parse_generate_body(alt, ConcContext::Plain)?);
                        break;
                    }
                    _ => break,
                }
            }
            p.parse_end(&[TokenKind::Generate], false, label)?;
            Ok(IfGenerate {
                arms,
                else_arm,
                span: p.span_from(start),
            })
        })
    }

    /// `case expr generate {when [alt:] choices => body} end generate;`
    fn parse_case_generate(&mut self, label: Option<&Ident>) -> PResult<CaseGenerate> {
        let start = self.span();
        self.expect(TokenKind::Case)?;
        self.require_2008(start, "case generate statements");
        self.with_open(TokenKind::Generate, |p| {
            let expr = p.parse_condition_until(TokenKind::Generate)?;
            p.expect_or_skip_to(TokenKind::Generate)?;
            let mut arms = Vec::new();
            while p.at(TokenKind::When) {
                let arm_start = p.bump().span;
                let alt = p.parse_optional_label()?;
                let choices = p.parse_choices()?;
                p.expect_or_skip_to(TokenKind::Arrow)?;
                let body = p.parse_generate_body(alt, ConcContext::CaseGenerate)?;
                arms.push(CaseGenerateArm {
                    choices,
                    body,
                    span: p.span_from(arm_start),
                });
            }
            p.parse_end(&[TokenKind::Generate], false, label)?;
            Ok(CaseGenerate {
                expr,
                arms,
                span: p.span_from(start),
            })
        })
    }

    /// `[decls begin] stmts [end [alt_label];]`
    fn parse_generate_body(
        &mut self,
        label: Option<Ident>,
        ctx: ConcContext,
    ) -> PResult<GenerateBody> {
        let start = self.span();
        let decls = if self.at_declaration_start() {
            let d = self.parse_declarative_part();
            self.expect_or_skip_to(TokenKind::Begin)?;
            d
        } else {
            self.eat(TokenKind::Begin);
            Vec::new()
        };
        let statements = self.parse_concurrent_statements_in(ctx);
        // `end;` or `end alt_label;` closes the body itself, as opposed to
        // the `end generate` of the statement.
        let body_end = self.at(TokenKind::End)
            && (self.kind_at(1) == TokenKind::Semi
                || (matches!(self.kind_at(1), TokenKind::Ident | TokenKind::ExtendedIdent)
                    && self.kind_at(2) == TokenKind::Semi));
        if body_end {
            self.bump();
            self.parse_optional_end_name(label.as_ref())?;
            self.expect_semi()?;
        }
        Ok(GenerateBody {
            label,
            decls,
            statements,
            span: self.span_from(start),
        })
    }
}
