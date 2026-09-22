//! Generate constructs: `if`, `case` and `for` at item level, and the
//! `begin : label ... end` blocks they contain. The `generate` /
//! `endgenerate` region itself is handled by the item loop, since it is
//! only a wrapper.

use super::super::ast::{GenBlock, GenCase, GenCaseItem, GenFor, GenIf, Item};
use super::super::token::{Keyword, Punct};
use super::{PResult, Parser};

impl Parser<'_> {
    /// `if (cond) block [else block]`, cursor on `if`.
    pub(super) fn parse_gen_if(&mut self) -> PResult<GenIf> {
        self.expect_kw(Keyword::If)?;
        self.expect_punct(Punct::LParen)?;
        let cond = self.parse_expr()?;
        self.expect_punct(Punct::RParen)?;
        let then_block = self.parse_gen_block_or_item()?;
        let else_block = if self.eat_kw(Keyword::Else).is_some() {
            Some(self.parse_gen_block_or_item()?)
        } else {
            None
        };
        Ok(GenIf {
            cond,
            then_block,
            else_block,
        })
    }

    /// `case (expr) values: block ... default: block endcase`, cursor on
    /// `case`.
    pub(super) fn parse_gen_case(&mut self) -> PResult<GenCase> {
        self.expect_kw(Keyword::Case)?;
        self.expect_punct(Punct::LParen)?;
        let expr = self.parse_expr()?;
        self.expect_punct(Punct::RParen)?;
        let mut items = Vec::new();
        while !self.at_kw(Keyword::Endcase) && !self.at_eof() {
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
            let block = self.parse_gen_block_or_item()?;
            items.push(GenCaseItem {
                patterns,
                block,
                span: self.span_from(start),
            });
        }
        self.expect_end(Keyword::Endcase);
        Ok(GenCase { expr, items })
    }

    /// `for ([genvar] i = init; cond; step) block`, cursor on `for`.
    pub(super) fn parse_gen_for(&mut self) -> PResult<GenFor> {
        self.expect_kw(Keyword::For)?;
        self.expect_punct(Punct::LParen)?;
        let genvar = self.eat_kw(Keyword::Genvar).is_some();
        let var = self.expect_ident()?;
        self.expect_punct(Punct::Eq)?;
        let init = self.parse_expr()?;
        self.expect_semi()?;
        let cond = self.parse_expr()?;
        self.expect_semi()?;
        let step = self.parse_expr_or_assign()?;
        self.expect_punct(Punct::RParen)?;
        let body = self.parse_gen_block_or_item()?;
        Ok(GenFor {
            genvar,
            var,
            init,
            cond,
            step,
            body,
        })
    }

    /// `begin [: label] items end [: label]`, cursor on `begin`.
    pub(super) fn parse_gen_block(&mut self) -> PResult<GenBlock> {
        let start = self.expect_kw(Keyword::Begin)?;
        let label = if self.eat_punct(Punct::Colon).is_some() {
            Some(self.expect_ident()?)
        } else {
            None
        };
        let items = self.parse_items_until(&[Keyword::End]);
        self.expect_end(Keyword::End);
        Ok(GenBlock {
            label,
            items,
            span: self.span_from(start),
        })
    }

    /// A `begin ... end` block, a lone `;`, or a single item, as the body
    /// of a generate construct. A SystemVerilog `label : begin` is also
    /// accepted.
    fn parse_gen_block_or_item(&mut self) -> PResult<GenBlock> {
        let start = self.span();
        if self.at_ident()
            && self.nth_is_punct(1, Punct::Colon)
            && self.nth_is_kw(2, Keyword::Begin)
        {
            let label = self.expect_ident()?;
            self.bump();
            let mut block = self.parse_gen_block()?;
            if block.label.is_none() {
                block.label = Some(label);
            }
            block.span = self.span_from(start);
            return Ok(block);
        }
        if self.at_kw(Keyword::Begin) {
            return self.parse_gen_block();
        }
        if self.eat_punct(Punct::Semi).is_some() {
            return Ok(GenBlock {
                label: None,
                items: Vec::new(),
                span: self.span_from(start),
            });
        }
        let items: Vec<Item> = self.parse_item()?.into_iter().collect();
        Ok(GenBlock {
            label: None,
            items,
            span: self.span_from(start),
        })
    }
}
