//! Packages, `import` / `export`, and `timeunit` / `timeprecision`.

use super::super::ast::{Ident, ItemKind, Literal, Package, PackageRef};
use super::super::token::{Keyword, Punct, TokenKind};
use super::{PResult, Parser};

impl Parser<'_> {
    /// `package [lifetime] name; items endpackage [: name]`, cursor on
    /// `package`.
    pub(super) fn parse_package(&mut self) -> PResult<Package> {
        self.expect_kw(Keyword::Package)?;
        let lifetime = self.parse_lifetime();
        let name = self.expect_ident()?;
        self.expect_semi()?;
        let items = self.parse_items_until(&[Keyword::Endpackage]);
        self.expect_end(Keyword::Endpackage);
        Ok(Package {
            lifetime,
            name,
            items,
        })
    }

    /// `pkg::name, pkg::*, ...` (also `*::*`), the list after `import` or
    /// `export`.
    pub(super) fn parse_package_refs(&mut self) -> PResult<Vec<PackageRef>> {
        let mut refs = Vec::new();
        loop {
            let package = if self.at_punct(Punct::Star) {
                let span = self.bump();
                Ident::new("*", span)
            } else {
                self.expect_ident()?
            };
            self.expect_punct(Punct::ColonColon)?;
            let item = if self.eat_punct(Punct::Star).is_some() {
                None
            } else {
                Some(self.expect_ident()?)
            };
            let span = self.span_from(package.span);
            refs.push(PackageRef {
                package,
                item,
                span,
            });
            if self.eat_punct(Punct::Comma).is_none() {
                break;
            }
        }
        Ok(refs)
    }

    /// `import pkg::*;`, `export pkg::name;` or `export *::*;`, cursor on
    /// the keyword. A DPI `import "DPI-C" ...` is reported as unsupported
    /// and skipped whole (`None`).
    pub(super) fn parse_import_or_export(&mut self) -> PResult<Option<ItemKind>> {
        let is_import = self.at_kw(Keyword::Import);
        let start = self.bump();
        if matches!(self.kind(), TokenKind::Str { .. }) {
            let what = if is_import { "imports" } else { "exports" };
            self.error_at(start, format!("DPI {what} are not supported"));
            self.skip_past_semi();
            return Ok(None);
        }
        let refs = self.parse_package_refs()?;
        self.expect_semi()?;
        Ok(Some(if is_import {
            ItemKind::Import(refs)
        } else {
            ItemKind::Export(refs)
        }))
    }

    /// `timeunit 1ns [/ 1ps];` or `timeprecision 1ps;`, cursor on the
    /// keyword.
    pub(super) fn parse_timeunit(&mut self) -> PResult<ItemKind> {
        let is_unit = self.at_kw(Keyword::Timeunit);
        self.bump();
        let unit = self.parse_time_literal()?;
        let kind = if is_unit {
            let precision = if self.eat_punct(Punct::Slash).is_some() {
                Some(self.parse_time_literal()?)
            } else {
                None
            };
            ItemKind::Timeunit { unit, precision }
        } else {
            ItemKind::Timeprecision(unit)
        };
        self.expect_semi()?;
        Ok(kind)
    }

    /// A time literal such as `1ns`, kept as text.
    fn parse_time_literal(&mut self) -> PResult<Literal> {
        match self.kind() {
            TokenKind::Number { text } => {
                let text = text.clone();
                let span = self.bump();
                Ok(Literal::Number { text, span })
            }
            _ => Err(self.expected("time literal")),
        }
    }
}
