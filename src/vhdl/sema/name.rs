//! Names: classification, calls, attributes and aggregates
//! (IEEE 1076-2008 clause 8, 9.3.3, 9.3.4).
//!
//! `Checker::classify` is the single entry point that turns a [`Name`]
//! into a [`Prefix`]. The parser leaves `f(x)` as [`Name::Call`] because
//! the grammar cannot tell a function call from an array index, a type
//! conversion or an attribute argument; classification resolves the
//! prefix first and then decides:
//!
//! | Prefix classifies to | `p(args)` means |
//! |---|---|
//! | a type mark | a type conversion (clause 9.3.6) |
//! | overloaded subprograms | a function call (clause 9.3.4) |
//! | an object of array type | an index, or a slice for one range |
//! | an object of access-to-array type | an implicit dereference, then the above |
//! | a predefined attribute | the attribute's argument (`t'image(x)`) |
//!
//! Aggregates (clause 9.3.3) are resolved against the composite type the
//! context asks for: positional associations fill the index range in
//! order, choices are resolved against the index type (or matched to
//! record element names), and `others` covers the rest. When every
//! element is static the aggregate itself folds to a [`Value`], which is
//! what makes `(others => '0')` usable as a constant initialiser.

use crate::diag::Diagnostic;
use crate::source::Span;
use crate::vhdl::ast::{
    self, Actual, Aggregate, AssociationElement, Choice, Direction, Expr, Name, Suffix,
};

use super::attrs::{self, Arg, Predefined, PrefixKind};
use super::check::Checker;
use super::constant::{ArrayValue, Value};
use super::expr::{Mode, NameInfo, ObjInfo, Prefix};
use super::library::LibraryUnitKind;
use super::overload::{self, ArgInfo, Cand, Mismatch};
use super::scope;
use super::types::{Bounds, TypeClass, TypeId, TypeKind};
use super::{CallTarget, DeclId, DeclKind, ObjectClass, ObjectRole, RangeInfo};

impl Checker<'_> {
    // --- classification ----------------------------------------------------

    /// Classifies a name. In [`Mode::Infer`] nothing is reported and
    /// [`Prefix::Error`] is returned for anything unresolvable.
    pub(crate) fn classify(&mut self, n: &Name, mode: Mode, expected: Option<TypeId>) -> Prefix {
        match n {
            Name::Simple(i) => {
                let sym = self.ident_sym(i);
                self.classify_lookup(sym, &i.name, i.span, mode)
            }
            Name::Operator { symbol, span } => {
                let sym = self
                    .a
                    .interner
                    .intern(&format!("\"{}\"", symbol.to_lowercase()));
                self.classify_lookup(sym, &format!("\"{symbol}\""), *span, mode)
            }
            Name::Char { ch, span } => {
                let sym = self.a.interner.intern(&format!("'{ch}'"));
                self.classify_lookup(sym, &format!("'{ch}'"), *span, mode)
            }
            Name::Selected {
                prefix,
                suffix,
                span,
            } => self.classify_selected(prefix, suffix, *span, mode),
            Name::Call { prefix, args, span } => {
                self.classify_call(prefix, args, *span, mode, expected)
            }
            Name::Slice {
                prefix,
                range,
                span,
            } => {
                let p = self.classify(prefix, mode, None);
                let (obj, ty) = match p {
                    Prefix::Object(o, t) => (Some(o), t),
                    Prefix::Value(t) => (None, t),
                    Prefix::Error => return Prefix::Error,
                    _ => {
                        if mode == Mode::Commit {
                            self.error(
                                "V0206",
                                prefix.span(),
                                "only an array object can be sliced",
                            );
                        }
                        return Prefix::Error;
                    }
                };
                let ty = self.deref_if_access(ty);
                if self.a.class(ty) != TypeClass::Array {
                    if mode == Mode::Commit && !self.a.is_error(ty) {
                        let tn = self.ty_name(ty);
                        self.error(
                            "V0206",
                            *span,
                            format!("`{tn}` is not an array type, so it cannot be sliced"),
                        );
                    }
                    return Prefix::Error;
                }
                let idx = self.a.array_info(ty).and_then(|(i, _)| i.first().copied());
                let info = self.resolve_discrete_range_in(range, idx);
                let elem = self.a.element_type(ty).unwrap_or(self.a.builtins.error);
                let sliced = self.slice_subtype(ty, elem, info.bounds.clone());
                self.a.set_type(*span, sliced);
                self.check_slice_bounds(ty, &info, *span, mode);
                match obj {
                    Some(o) => Prefix::Object(o, sliced),
                    None => Prefix::Value(sliced),
                }
            }
            Name::Attribute {
                prefix,
                attribute,
                signature,
                span,
            } => {
                self.classify_attribute(prefix, attribute, signature.as_deref(), *span, mode, None)
            }
            Name::External(ext) => {
                self.require_2008(ext.span, "external names");
                let ty = self.resolve_subtype_indication(&ext.subtype);
                for el in &ext.path.elements {
                    if let Some(idx) = &el.index {
                        let ui = self.a.builtins.universal_integer;
                        self.resolve(idx, ui);
                    }
                }
                let class = match ext.class {
                    ast::ExternalClass::Constant => ObjectClass::Constant,
                    ast::ExternalClass::Signal => ObjectClass::Signal,
                    ast::ExternalClass::Variable => ObjectClass::Variable,
                };
                // External names have no declaration to point at; make a
                // synthetic one so the lowering pass has a handle.
                let sym = self.a.interner.intern_ci(&format!(
                    "<external {}>",
                    ext.path
                        .elements
                        .iter()
                        .map(|e| e.name.name.as_str())
                        .collect::<Vec<_>>()
                        .join(".")
                ));
                let spelling = self.text(ext.span).to_owned();
                let region = self.region;
                let decl = self.a.add_decl(
                    region,
                    sym,
                    spelling,
                    DeclKind::Object {
                        class,
                        ty,
                        mode: None,
                        role: ObjectRole::External,
                        deferred: false,
                    },
                    ext.span,
                );
                self.a.set_ref(ext.span, decl);
                Prefix::Object(
                    ObjInfo {
                        decl,
                        class,
                        mode: None,
                        role: ObjectRole::External,
                    },
                    ty,
                )
            }
        }
    }

    /// Looks a designator up and turns the result into a [`Prefix`].
    fn classify_lookup(
        &mut self,
        sym: crate::intern::Symbol,
        spelling: &str,
        span: Span,
        mode: Mode,
    ) -> Prefix {
        let lk = scope::lookup(self.a, self.region, sym);
        if !lk.conflicting.is_empty() {
            if mode == Mode::Commit {
                let mut d = Diagnostic::error(format!(
                    "`{spelling}` is made visible by more than one use clause, so it is not visible"
                ))
                .with_code("V0201")
                .with_span(span);
                for c in &lk.conflicting {
                    let dspan = self.a.decl(*c).span;
                    d = d.with_secondary(dspan, "one declaration here");
                }
                self.push(d.with_note("select it explicitly, as in `ieee.numeric_std.\"+\"`"));
            }
            return Prefix::Error;
        }
        if lk.decls.is_empty() {
            if mode == Mode::Commit {
                self.report_unknown(sym, spelling, span);
            }
            return Prefix::Error;
        }
        if lk.decls.len() > 1 || scope::is_overloadable(&self.a.decl(lk.decls[0]).kind) {
            return Prefix::Overloaded(lk.decls);
        }
        let d = lk.decls[0];
        self.a.set_ref(span, d);
        self.prefix_of_decl(d, span)
    }

    /// The [`Prefix`] a non-overloadable declaration denotes.
    fn prefix_of_decl(&mut self, d: DeclId, span: Span) -> Prefix {
        match self.a.decl(d).kind.clone() {
            DeclKind::Library => {
                let lib = self.a.decl(d).name;
                let lib = if lib == self.syms.work {
                    self.ctx.library
                } else {
                    lib
                };
                match self.library_regions.get(&lib).copied() {
                    Some(r) => Prefix::Region(d, r),
                    None => Prefix::Error,
                }
            }
            DeclKind::Unit { unit, region } => {
                match self.a.units.get(unit.index()).map(|u| u.kind) {
                    Some(LibraryUnitKind::Package)
                    | Some(LibraryUnitKind::PackageInstantiation)
                    | Some(LibraryUnitKind::Context) => Prefix::Region(d, region),
                    _ => Prefix::Unit(unit),
                }
            }
            DeclKind::Type(t) | DeclKind::Subtype(t) => {
                self.a.set_type(span, t);
                Prefix::Type(t)
            }
            DeclKind::Object {
                class,
                ty,
                mode,
                role,
                ..
            } => {
                self.a.set_type(span, ty);
                // Make a static constant's value visible on the name, so
                // that indexing or slicing it folds too.
                if let Some(v) = self.a.decl_value(d).cloned() {
                    self.a.set_value(span, v);
                }
                Prefix::Object(
                    ObjInfo {
                        decl: d,
                        class,
                        mode,
                        role,
                    },
                    ty,
                )
            }
            DeclKind::RecordElement(ty) => Prefix::Value(ty),
            DeclKind::Component { .. } => Prefix::Error,
            DeclKind::Alias(target) => self.prefix_of_decl(target, span),
            DeclKind::Attribute(t) => Prefix::Value(t),
            DeclKind::Label(_) | DeclKind::Group => Prefix::Error,
            DeclKind::PhysicalUnit { ty, .. } => Prefix::Value(ty),
            DeclKind::EnumLiteral { .. } | DeclKind::Subprogram { .. } => {
                Prefix::Overloaded(vec![d])
            }
            DeclKind::Error => Prefix::Error,
        }
    }

    /// Reports an unknown identifier with suggestions and a missing-`use`
    /// hint.
    pub(crate) fn report_unknown(
        &mut self,
        sym: crate::intern::Symbol,
        spelling: &str,
        span: Span,
    ) {
        let mut d = Diagnostic::error(format!("cannot find `{spelling}` in this scope"))
            .with_code("V0200")
            .with_label(span, "not found in this scope");
        let pkgs = scope::packages_declaring(self.a, sym);
        if let Some(p) = pkgs.first() {
            let lib = p.split('.').next().unwrap_or("ieee");
            if lib == "work" || lib == self.a.name(self.ctx.library) {
                d = d.with_note(format!(
                    "`{spelling}` is declared in `{p}`; add `use {p}.all;`"
                ));
            } else {
                d = d.with_note(format!(
                    "`{spelling}` is declared in `{p}`; add `library {lib}; use {p}.all;`"
                ));
            }
        } else {
            let names = scope::visible_names(self.a, self.region);
            if let Some(s) = super::suggest(spelling, names.iter().map(String::as_str)) {
                d = d.with_note(format!("did you mean `{s}`?"));
            }
        }
        self.push(d);
    }

    /// `prefix.suffix`.
    fn classify_selected(
        &mut self,
        prefix: &Name,
        suffix: &Suffix,
        span: Span,
        mode: Mode,
    ) -> Prefix {
        let p = self.classify(prefix, mode, None);
        let Suffix::Designator(des) = suffix else {
            if mode == Mode::Commit {
                self.error("V0206", span, "`all` is only allowed in a use clause");
            }
            return Prefix::Error;
        };
        let (sym, spelling) = self.designator_sym(des);
        match p {
            Prefix::Region(pd, region) => {
                let found: Vec<DeclId> = self.a.region(region).direct(sym).to_vec();
                if found.is_empty() {
                    if mode == Mode::Commit {
                        // A design unit missing from a *library* may be one
                        // of the standard packages Reticle does not bundle
                        // yet; say so plainly instead of offering a typo.
                        let is_library = matches!(self.a.decl(pd).kind, DeclKind::Library);
                        let lib = self.a.name(self.a.decl(pd).name).to_owned();
                        let note = if is_library {
                            crate::vhdl::stdlib::missing_package_note(&lib, &spelling)
                        } else {
                            None
                        };
                        if let Some(note) = note {
                            self.push(
                                Diagnostic::error(note)
                                    .with_code("V0107")
                                    .with_label(des.span(), "not available in this build")
                                    .with_note(
                                        "Reticle bundles `std.standard`, `std.textio`, `std.env`, `ieee.std_logic_1164`, `ieee.numeric_std`, `ieee.numeric_bit`, `ieee.math_real`, `ieee.std_logic_textio` and the Synopsys `std_logic_arith` / `std_logic_unsigned` / `std_logic_signed`",
                                    ),
                            );
                            return Prefix::Error;
                        }
                        let names = self.a.region(region).names();
                        let names: Vec<String> =
                            names.iter().map(|s| self.a.name(*s).to_owned()).collect();
                        let ptext = self.text(prefix.span()).to_owned();
                        let what = if is_library { "library" } else { "package" };
                        let mut d = Diagnostic::error(format!(
                            "`{spelling}` is not declared in {what} `{ptext}`"
                        ))
                        .with_code("V0200")
                        .with_span(des.span());
                        if let Some(s) = super::suggest(&spelling, names.iter().map(String::as_str))
                        {
                            d = d.with_note(format!("did you mean `{s}`?"));
                        }
                        self.push(d);
                    }
                    return Prefix::Error;
                }
                if found.len() > 1 || scope::is_overloadable(&self.a.decl(found[0]).kind) {
                    return Prefix::Overloaded(found);
                }
                self.a.set_ref(des.span(), found[0]);
                self.prefix_of_decl(found[0], span)
            }
            Prefix::Object(obj, ty) => {
                let ty = self.deref_if_access(ty);
                match self.a.field_type(ty, sym) {
                    Some(fty) => {
                        // Record the element as a declaration so the
                        // lowering pass can resolve `r.f` to a field.
                        if let Some(fields) = self.a.record_fields(ty)
                            && let Some(f) = fields.iter().find(|f| f.name == sym)
                        {
                            let (fspan, fty2) = (f.span, f.ty);
                            let region = self.region;
                            let fdecl = self.a.add_decl(
                                region,
                                sym,
                                spelling.clone(),
                                DeclKind::RecordElement(fty2),
                                fspan,
                            );
                            self.a.set_ref(des.span(), fdecl);
                        }
                        self.a.set_type(span, fty);
                        Prefix::Object(obj, fty)
                    }
                    None => {
                        if mode == Mode::Commit && !self.a.is_error(ty) {
                            let tn = self.ty_name(ty);
                            let mut d =
                                Diagnostic::error(format!("`{tn}` has no element `{spelling}`"))
                                    .with_code("V0202")
                                    .with_span(des.span());
                            if let Some(fields) = self.a.record_fields(ty) {
                                let names: Vec<String> = fields
                                    .iter()
                                    .map(|f| self.a.name(f.name).to_owned())
                                    .collect();
                                if let Some(s) =
                                    super::suggest(&spelling, names.iter().map(String::as_str))
                                {
                                    d = d.with_note(format!("did you mean `{s}`?"));
                                } else if !names.is_empty() {
                                    d = d.with_note(format!("elements: {}", names.join(", ")));
                                }
                            }
                            self.push(d);
                        }
                        Prefix::Error
                    }
                }
            }
            Prefix::Value(ty) => {
                let ty = self.deref_if_access(ty);
                match self.a.field_type(ty, sym) {
                    Some(fty) => {
                        self.a.set_type(span, fty);
                        Prefix::Value(fty)
                    }
                    None => {
                        if mode == Mode::Commit && !self.a.is_error(ty) {
                            let tn = self.ty_name(ty);
                            self.error(
                                "V0202",
                                des.span(),
                                format!("`{tn}` has no element `{spelling}`"),
                            );
                        }
                        Prefix::Error
                    }
                }
            }
            Prefix::Type(t) => {
                // A selected enumeration literal (`state_t.idle` is not
                // VHDL, but `work.pkg.idle` reaches here through Region).
                if mode == Mode::Commit {
                    let tn = self.ty_name(t);
                    self.error(
                        "V0206",
                        span,
                        format!("`{tn}` is a type; it has no selected names"),
                    );
                }
                Prefix::Error
            }
            Prefix::Unit(u) => {
                // `work.e.something` is not a name; but an entity used as
                // a prefix in an instantiation is handled elsewhere.
                let _ = u;
                if mode == Mode::Commit {
                    self.error(
                        "V0206",
                        span,
                        "a design unit that is not a package has no selected names",
                    );
                }
                Prefix::Error
            }
            Prefix::Overloaded(_) | Prefix::Values(_) => {
                if mode == Mode::Commit {
                    self.error("V0206", span, "the prefix of a selected name must be a package, a record object or an access value");
                }
                Prefix::Error
            }
            Prefix::Error => Prefix::Error,
        }
    }

    /// Dereferences an access type once (VHDL's implicit dereference of
    /// an access value used as a record or array prefix, clause 8.1).
    pub(crate) fn deref_if_access(&self, ty: TypeId) -> TypeId {
        self.a.designated_type(ty).unwrap_or(ty)
    }

    // --- calls, indexes and conversions ------------------------------------

    fn classify_call(
        &mut self,
        prefix: &Name,
        args: &[AssociationElement],
        span: Span,
        mode: Mode,
        expected: Option<TypeId>,
    ) -> Prefix {
        // `t'image(x)` and `x'range(1)`: the prefix is an attribute name.
        if let Name::Attribute {
            prefix: aprefix,
            attribute,
            signature,
            ..
        } = prefix
        {
            return self.classify_attribute(
                aprefix,
                attribute,
                signature.as_deref(),
                span,
                mode,
                Some(args),
            );
        }
        let p = self.classify(prefix, mode, None);
        match p {
            Prefix::Type(t) => {
                // A type conversion takes exactly one positional actual.
                if args.len() != 1 || args[0].formal.is_some() {
                    if mode == Mode::Commit {
                        self.error("V0502", span, "a type conversion takes one operand");
                    }
                    return Prefix::Error;
                }
                let Actual::Expr(operand) = &args[0].actual else {
                    if mode == Mode::Commit {
                        self.error("V0502", span, "a type conversion takes an expression");
                    }
                    return Prefix::Error;
                };
                if mode == Mode::Infer {
                    return Prefix::Value(t);
                }
                let from = self.resolve_conversion_operand(operand, t);
                if !self.a.is_error(from) && !overload::closely_related(self.a, from, t) {
                    let (f, tn) = (self.ty_name(from), self.ty_name(t));
                    self.push(
                        Diagnostic::error(format!("cannot convert `{f}` to `{tn}`"))
                            .with_code("V0309")
                            .with_label(span, format!("`{f}` and `{tn}` are not closely related"))
                            .with_note("a type conversion needs two numeric types, or two array types with the same element type and dimensionality"),
                    );
                }
                self.a.set_call(span, CallTarget::Conversion(t));
                self.a.set_type(span, t);
                self.fold_conversion(span, operand.span(), from, t);
                Prefix::Value(t)
            }
            Prefix::Overloaded(cands) => {
                self.resolve_call(prefix, &cands, args, span, mode, expected)
            }
            Prefix::Object(obj, ty) => {
                let ty = self.deref_if_access(ty);
                self.index_or_slice(Some(obj), ty, args, span, prefix.span(), mode)
            }
            Prefix::Value(ty) => {
                let ty = self.deref_if_access(ty);
                self.index_or_slice(None, ty, args, span, prefix.span(), mode)
            }
            Prefix::Error => Prefix::Error,
            _ => {
                if mode == Mode::Commit {
                    let t = self.text(prefix.span()).to_owned();
                    self.error("V0206", span, format!("`{t}` cannot be called or indexed"));
                }
                Prefix::Error
            }
        }
    }

    /// The operand of a type conversion: it has no expected type of its
    /// own, so it is resolved freely, except that a candidate closely
    /// related to the target is preferred.
    fn resolve_conversion_operand(&mut self, operand: &Expr, target: TypeId) -> TypeId {
        let cands = self.infer(operand);
        let concrete: Vec<TypeId> = cands
            .iter()
            .filter_map(|c| match c {
                Cand::Ty(t) => Some(*t),
                _ => None,
            })
            .collect();
        if concrete.len() > 1
            && let Some(&t) = concrete
                .iter()
                .find(|&&t| overload::closely_related(self.a, t, target))
        {
            return self.resolve(operand, t);
        }
        if concrete.is_empty()
            && cands
                .iter()
                .any(|c| matches!(c, Cand::StringLit | Cand::Aggregate))
        {
            // `std_logic_vector("1010")` has no independent type: resolve
            // the literal against the target.
            return self.resolve(operand, target);
        }
        self.resolve_free(operand)
    }

    /// Folds a static type conversion between numeric types and between
    /// closely related arrays.
    fn fold_conversion(&mut self, span: Span, operand: Span, from: TypeId, to: TypeId) {
        let Some(v) = self.a.value_of(operand).cloned() else {
            return;
        };
        let (cf, ct) = (self.a.class(from), self.a.class(to));
        let out = if ct.is_integer() && cf.is_real() {
            v.as_real()
                .and_then(super::check::round_to_i128)
                .map(Value::Int)
        } else if ct.is_real() && cf.is_integer() {
            v.as_int().map(|i| Value::Real(i as f64))
        } else if ct == TypeClass::Array {
            // Re-index onto the target subtype's bounds.
            v.as_array().map(|a| {
                let (left, dir) = match self.a.index_constraint(to).and_then(|c| c.first().cloned())
                {
                    Some(b) => (b.left.int().unwrap_or(a.left), b.dir),
                    None => (a.left, a.dir),
                };
                Value::Array(ArrayValue {
                    left,
                    dir,
                    elems: a.elems.clone(),
                })
            })
        } else {
            Some(v)
        };
        if let Some(out) = out {
            self.a.set_value(span, out);
        }
    }

    /// `obj(args)`: an index (or a slice when the single actual is a
    /// range).
    #[allow(clippy::too_many_arguments)]
    fn index_or_slice(
        &mut self,
        obj: Option<ObjInfo>,
        ty: TypeId,
        args: &[AssociationElement],
        span: Span,
        prefix_span: Span,
        mode: Mode,
    ) -> Prefix {
        if self.a.class(ty) != TypeClass::Array {
            if mode == Mode::Commit && !self.a.is_error(ty) {
                let tn = self.ty_name(ty);
                let mut d = Diagnostic::error(format!(
                    "`{tn}` is not an array type, so it cannot be indexed"
                ))
                .with_code("V0206")
                .with_span(span);
                if self.a.class(ty) == TypeClass::Record {
                    d = d.with_note("use `.element` to select a record element");
                }
                self.push(d);
            }
            return Prefix::Error;
        }
        let (indices, elem) = match self.a.array_info(ty) {
            Some((i, e)) => (i.to_vec(), e),
            None => return Prefix::Error,
        };
        let elem = self.a.element_type(ty).unwrap_or(elem);
        // One positional range: a slice.
        if args.len() == 1
            && args[0].formal.is_none()
            && let Actual::Range(r) = &args[0].actual
        {
            let info = self.resolve_discrete_range_in(r, indices.first().copied());
            let sliced = self.slice_subtype(ty, elem, info.bounds.clone());
            self.check_slice_bounds(ty, &info, span, mode);
            self.a.set_type(span, sliced);
            return match obj {
                Some(o) => Prefix::Object(o, sliced),
                None => Prefix::Value(sliced),
            };
        }
        if args.len() != indices.len() {
            if mode == Mode::Commit {
                let tn = self.ty_name(ty);
                self.error(
                    "V0502",
                    span,
                    format!(
                        "`{tn}` has {} dimension{}, but {} index{} given",
                        indices.len(),
                        if indices.len() == 1 { "" } else { "s" },
                        args.len(),
                        if args.len() == 1 { " is" } else { "es are" }
                    ),
                );
            }
            return Prefix::Error;
        }
        let mut static_idx = Vec::new();
        for (arg, idx_ty) in args.iter().zip(&indices) {
            match &arg.actual {
                Actual::Expr(e) => {
                    if mode == Mode::Commit {
                        self.resolve(e, *idx_ty);
                        self.check_index_bounds(ty, e, *idx_ty, span);
                        static_idx.push(self.a.value_of(e.span()).cloned());
                    }
                }
                other => {
                    if mode == Mode::Commit {
                        self.error("V0502", other.span(), "expected an index expression");
                    }
                }
            }
        }
        if mode == Mode::Commit {
            self.a.set_call(span, CallTarget::Index);
            self.a.set_type(span, elem);
            // A static index into a statically known array folds; the
            // prefix's value is looked up through its own span.
            if let (1, Some(Some(iv))) = (indices.len(), static_idx.first())
                && let Some(i) = iv.as_int()
                && let Some(av) = self
                    .a
                    .value_of(prefix_span)
                    .and_then(|v| v.as_array().cloned())
                && let Some(v) = av.get(i).cloned()
            {
                self.a.set_value(span, v);
            }
        }
        match obj {
            Some(o) => Prefix::Object(o, elem),
            None => Prefix::Value(elem),
        }
    }

    /// The anonymous subtype of a slice: the element type with the
    /// slice's bounds.
    fn slice_subtype(&mut self, array: TypeId, elem: TypeId, bounds: Bounds) -> TypeId {
        let base = self.a.base_type(array);
        let _ = elem;
        self.a.add_type(
            TypeKind::Subtype {
                parent: base,
                constraint: Some(super::types::Constraint::Index(vec![bounds], None)),
                resolution: None,
            },
            None,
        )
    }

    /// Reports a slice whose static bounds fall outside the object's
    /// static index range, or whose direction disagrees (clause 8.5).
    fn check_slice_bounds(&mut self, array: TypeId, info: &RangeInfo, span: Span, mode: Mode) {
        if mode != Mode::Commit {
            return;
        }
        let Some(obj) = self
            .a
            .index_constraint(array)
            .and_then(|c| c.first().cloned())
        else {
            return;
        };
        if info.bounds.length() == Some(0) {
            return;
        }
        if obj.is_static() && info.bounds.is_static() && obj.dir != info.bounds.dir {
            self.error(
                "V0307",
                span,
                format!(
                    "slice direction `{}` does not match the object's `{}`",
                    info.bounds.dir.as_str(),
                    obj.dir.as_str()
                ),
            );
            return;
        }
        let (Some((olo, ohi)), Some((slo, shi))) = (obj.low_high(), info.bounds.low_high()) else {
            return;
        };
        if slo < olo || shi > ohi {
            let tn = self.ty_name(array);
            self.error(
                "V0307",
                span,
                format!("slice {slo} to {shi} is outside the range of `{tn}` ({olo} to {ohi})"),
            );
        }
    }

    /// Reports a static index outside a static index range.
    fn check_index_bounds(&mut self, array: TypeId, e: &Expr, idx_ty: TypeId, _span: Span) {
        let Some(v) = self.a.value_of(e.span()).and_then(|v| v.as_int()) else {
            return;
        };
        let Some(c) = self
            .a
            .index_constraint(array)
            .and_then(|c| c.first().cloned())
        else {
            return;
        };
        if c.contains_int(v) == Some(false) {
            let tn = self.ty_name(array);
            let (lo, hi) = c.low_high().unwrap_or((0, 0));
            let show = |i: i128| self.a.describe_value(&Value::Int(i), idx_ty);
            let _ = show;
            self.error(
                "V0307",
                e.span(),
                format!("index {v} is outside the range of `{tn}` ({lo} to {hi})"),
            );
        }
    }

    /// Resolves a call to one of `cands`.
    fn resolve_call(
        &mut self,
        prefix: &Name,
        cands: &[DeclId],
        args: &[AssociationElement],
        span: Span,
        mode: Mode,
        expected: Option<TypeId>,
    ) -> Prefix {
        let mut infos = Vec::new();
        for a in args {
            let formal = match &a.formal {
                Some(Expr::Name(Name::Simple(i))) => Some(self.ident_sym(i)),
                _ => None,
            };
            let (cands, open) = match &a.actual {
                Actual::Expr(e) | Actual::Inertial(e) => (self.infer(e), false),
                Actual::Open(_) => (Vec::new(), true),
                Actual::Range(_) => (vec![Cand::Error], false),
            };
            infos.push(ArgInfo {
                formal,
                cands,
                span: a.span,
                open,
            });
        }
        let mut viable: Vec<(DeclId, Vec<Option<usize>>)> = Vec::new();
        let mut errors: Vec<(DeclId, Mismatch)> = Vec::new();
        for &d in cands {
            match &self.a.decl(d).kind {
                DeclKind::Subprogram { sig, .. } => {
                    if let Some(e) = expected
                        && let Some(ret) = sig.ret
                        && !overload::types_compatible(self.a, ret, e)
                    {
                        continue;
                    }
                    if expected.is_some() && sig.ret.is_none() {
                        continue;
                    }
                    let sig = sig.clone();
                    match overload::match_signature(self.a, &sig, &infos) {
                        Ok(assoc) => viable.push((d, assoc)),
                        Err(m) => errors.push((d, m)),
                    }
                }
                DeclKind::EnumLiteral { ty, .. } => {
                    // `'0'(...)` is not a call; an enumeration literal
                    // takes no arguments.
                    let _ = ty;
                }
                _ => {}
            }
        }
        if viable.is_empty() {
            if mode == Mode::Commit {
                self.report_no_call(prefix, cands, &errors, args, span, expected);
            }
            return Prefix::Error;
        }
        if viable.len() > 1 {
            // Prefer a candidate whose formals match the actuals exactly.
            let exact: Vec<_> = viable
                .iter()
                .filter(|(d, assoc)| self.call_is_exact(*d, assoc, &infos))
                .cloned()
                .collect();
            if exact.len() == 1 {
                let (d, assoc) = exact.into_iter().next().expect("one exact candidate");
                return self.commit_call(d, &assoc, args, span, mode);
            }
            if mode == Mode::Infer {
                let tys: Vec<TypeId> = viable
                    .iter()
                    .filter_map(|(d, _)| match &self.a.decl(*d).kind {
                        DeclKind::Subprogram { sig, .. } => sig.ret,
                        _ => None,
                    })
                    .collect();
                return Prefix::Values(tys);
            }
            let mut d =
                Diagnostic::error(format!("ambiguous call to `{}`", self.text(prefix.span())))
                    .with_code("V0302")
                    .with_label(span, "several visible subprograms match these arguments");
            for (c, _) in &viable {
                let profile = self.a.describe_subprogram(*c);
                let cspan = self.a.decl(*c).span;
                d = d.with_secondary(cspan, format!("candidate: {profile}"));
            }
            self.push(d);
            return Prefix::Error;
        }
        let (d, assoc) = viable.into_iter().next().expect("one candidate");
        self.commit_call(d, &assoc, args, span, mode)
    }

    /// True when every actual's type matches its formal's base exactly
    /// (no universal conversion, no literal wildcard).
    fn call_is_exact(&self, d: DeclId, assoc: &[Option<usize>], infos: &[ArgInfo]) -> bool {
        let DeclKind::Subprogram { sig, .. } = &self.a.decl(d).kind else {
            return false;
        };
        sig.params
            .iter()
            .enumerate()
            .all(|(p, param)| match assoc[p] {
                Some(i) => infos[i]
                    .cands
                    .iter()
                    .any(|c| overload::exact(self.a, *c, param.ty)),
                None => true,
            })
    }

    fn commit_call(
        &mut self,
        d: DeclId,
        assoc: &[Option<usize>],
        args: &[AssociationElement],
        span: Span,
        mode: Mode,
    ) -> Prefix {
        let DeclKind::Subprogram { sig, .. } = self.a.decl(d).kind.clone() else {
            return Prefix::Error;
        };
        if mode == Mode::Infer {
            return match sig.ret {
                Some(r) => Prefix::Value(r),
                None => Prefix::Error,
            };
        }
        self.a.set_ref(span, d);
        self.a.set_call(span, CallTarget::Subprogram(d));
        let mut arg_spans: Vec<Option<Span>> = Vec::new();
        for (p, param) in sig.params.iter().enumerate() {
            match assoc[p] {
                Some(i) => {
                    let a = &args[i];
                    if let Some(f) = &a.formal {
                        self.a.set_ref(f.span(), param.decl);
                    }
                    match &a.actual {
                        Actual::Expr(e) | Actual::Inertial(e) => {
                            self.resolve(e, param.ty);
                            self.check_static_range(e, param.ty);
                            self.check_actual_object(e, param, span);
                            arg_spans.push(Some(e.span()));
                        }
                        Actual::Open(s) => {
                            if !param.has_default {
                                self.error(
                                    "V0501",
                                    *s,
                                    "this parameter has no default, so it cannot be `open`",
                                );
                            }
                            arg_spans.push(None);
                        }
                        Actual::Range(r) => {
                            self.error(
                                "V0502",
                                r.span(),
                                "a subprogram parameter takes an expression, not a range",
                            );
                            arg_spans.push(None);
                        }
                    }
                }
                None => arg_spans.push(None),
            }
        }
        // Purity: a pure function may not call an impure one (clause 4.3).
        if let Some(sub) = self.ctx.subprogram
            && sub.pure
            && !sig.pure
            && sig.kind == ast::SubprogramKind::Function
        {
            let callee = self.a.decl(d).spelling.clone();
            let cspan = self.a.decl(d).span;
            self.push(
                Diagnostic::error(format!(
                    "pure function `{}` calls impure function `{callee}`",
                    self.a.decl(sub.decl).spelling
                ))
                .with_code("V0405")
                .with_label(span, "call to an impure function")
                .with_secondary(cspan, "declared `impure` here")
                .with_note("declare the caller `impure function` as well"),
            );
        }
        if let Some(ret) = sig.ret {
            self.a.set_type(span, ret);
            let spans: Vec<Span> = arg_spans.iter().flatten().copied().collect();
            if spans.len() == arg_spans.len() {
                self.fold_call(span, d, &spans, ret);
            }
            Prefix::Value(ret)
        } else {
            Prefix::Value(self.a.builtins.error)
        }
    }

    /// Checks the object rules of an actual (clause 4.2.2.3): a `signal`
    /// formal needs a signal actual, an `out`/`inout` formal needs a
    /// writable object.
    fn check_actual_object(&mut self, e: &Expr, param: &super::Param, _call: Span) {
        if param.class != ObjectClass::Signal
            && param.class != ObjectClass::Variable
            && param.class != ObjectClass::File
            && param.mode == ast::Mode::In
        {
            return;
        }
        let Expr::Name(n) = e else {
            if param.mode != ast::Mode::In || param.class == ObjectClass::Signal {
                let word = param.class.as_str();
                self.error(
                    "V0501",
                    e.span(),
                    format!("this parameter is a {word}, so the actual must be a {word} object"),
                );
            }
            return;
        };
        let info = self.name_info(n);
        let Some(obj) = info.obj else {
            if param.mode != ast::Mode::In {
                self.error(
                    "V0501",
                    e.span(),
                    "the actual for an `out` or `inout` parameter must be an object",
                );
            }
            return;
        };
        let want = param.class;
        let got = obj.class;
        let compatible = match want {
            ObjectClass::Signal => got == ObjectClass::Signal,
            ObjectClass::Variable => {
                matches!(got, ObjectClass::Variable | ObjectClass::SharedVariable)
            }
            ObjectClass::File => got == ObjectClass::File,
            ObjectClass::Constant => true,
            ObjectClass::SharedVariable => true,
        };
        if !compatible {
            let dspan = self.a.decl(obj.decl).span;
            self.push(
                Diagnostic::error(format!(
                    "this parameter is a {}, but the actual is a {}",
                    want.as_str(),
                    got.as_str()
                ))
                .with_code("V0501")
                .with_label(e.span(), format!("a {} is required here", want.as_str()))
                .with_secondary(dspan, format!("declared as a {} here", got.as_str())),
            );
            return;
        }
        if param.mode != ast::Mode::In {
            self.check_writable(&obj, e.span(), "passed to an `out` or `inout` parameter");
        }
    }

    fn report_no_call(
        &mut self,
        prefix: &Name,
        cands: &[DeclId],
        errors: &[(DeclId, Mismatch)],
        args: &[AssociationElement],
        span: Span,
        expected: Option<TypeId>,
    ) {
        let name = self.text(prefix.span()).to_owned();
        // A single candidate: explain precisely what does not fit.
        if let [(d, m)] = errors {
            let profile = self.a.describe_subprogram(*d);
            let dspan = self.a.decl(*d).span;
            let diag = match m {
                Mismatch::Type(i, p) => {
                    let DeclKind::Subprogram { sig, .. } = &self.a.decl(*d).kind else {
                        return;
                    };
                    let want = self.ty_name(sig.params[*p].ty);
                    let pname = self.a.decl(sig.params[*p].decl).spelling.clone();
                    let aspan = args[*i].actual.span();
                    let got = self.describe_actual(&args[*i]);
                    Diagnostic::error(format!("argument {} of `{name}` has the wrong type", i + 1))
                        .with_code("V0300")
                        .with_label(aspan, format!("this is {got}"))
                        .with_secondary(
                            dspan,
                            format!("`{pname}` is declared `{want}` in {profile}"),
                        )
                }
                Mismatch::TooMany => Diagnostic::error(format!(
                    "`{name}` takes {} argument{}, but {} were given",
                    self.param_count(*d),
                    if self.param_count(*d) == 1 { "" } else { "s" },
                    args.len()
                ))
                .with_code("V0502")
                .with_label(span, "too many arguments")
                .with_secondary(dspan, format!("`{profile}`")),
                Mismatch::Missing(p) => {
                    let pn = self.a.decl(*p).spelling.clone();
                    Diagnostic::error(format!("missing argument `{pn}` in call to `{name}`"))
                        .with_code("V0502")
                        .with_label(span, format!("`{pn}` has no default value"))
                        .with_secondary(dspan, format!("`{profile}`"))
                }
                Mismatch::UnknownFormal(f) => {
                    let fname = self.a.name(*f).to_owned();
                    let aspan = args
                        .iter()
                        .find(|a| matches!(&a.formal, Some(Expr::Name(Name::Simple(i))) if i.name.eq_ignore_ascii_case(&fname)))
                        .map_or(span, |a| a.span);
                    let params = self.param_names(*d);
                    let mut d2 = Diagnostic::error(format!("`{name}` has no parameter `{fname}`"))
                        .with_code("V0502")
                        .with_label(aspan, "no such parameter")
                        .with_secondary(dspan, format!("`{profile}`"));
                    if let Some(s) = super::suggest(&fname, params.iter().map(String::as_str)) {
                        d2 = d2.with_note(format!("did you mean `{s}`?"));
                    }
                    d2
                }
                Mismatch::Duplicate(p) => {
                    let DeclKind::Subprogram { sig, .. } = &self.a.decl(*d).kind else {
                        return;
                    };
                    let pn = self.a.decl(sig.params[*p].decl).spelling.clone();
                    Diagnostic::error(format!("parameter `{pn}` is associated twice"))
                        .with_code("V0502")
                        .with_span(span)
                }
                Mismatch::PositionalAfterNamed(i) => {
                    Diagnostic::error("a positional association cannot follow a named one")
                        .with_code("V0502")
                        .with_span(args[*i].span)
                }
            };
            self.push(diag);
            return;
        }
        let mut d = Diagnostic::error(format!("no visible `{name}` matches these arguments"))
            .with_code("V0303")
            .with_label(span, "no matching subprogram");
        if let Some(e) = expected {
            let en = self.ty_name(e);
            d = d.with_note(format!("the context requires `{en}`"));
        }
        let mut listed = 0;
        for &c in cands {
            if matches!(self.a.decl(c).kind, DeclKind::Subprogram { .. }) {
                let profile = self.a.describe_subprogram(c);
                d = d.with_note(format!("candidate: {profile}"));
                listed += 1;
                if listed == 6 {
                    d = d.with_note("(more candidates not shown)");
                    break;
                }
            }
        }
        self.push(d);
    }

    fn describe_actual(&mut self, a: &AssociationElement) -> String {
        let cands = match &a.actual {
            Actual::Expr(e) | Actual::Inertial(e) => self.infer(e),
            _ => Vec::new(),
        };
        let names: Vec<String> = cands
            .iter()
            .map(|c| match c {
                Cand::Ty(t) => format!("`{}`", self.ty_name(*t)),
                Cand::StringLit => "a string literal".into(),
                Cand::Aggregate => "an aggregate".into(),
                Cand::Null => "`null`".into(),
                Cand::Error => "of unknown type".into(),
            })
            .collect();
        if names.is_empty() {
            "of unknown type".into()
        } else {
            names.join(" or ")
        }
    }

    fn param_count(&self, d: DeclId) -> usize {
        match &self.a.decl(d).kind {
            DeclKind::Subprogram { sig, .. } => sig.params.len(),
            _ => 0,
        }
    }

    fn param_names(&self, d: DeclId) -> Vec<String> {
        match &self.a.decl(d).kind {
            DeclKind::Subprogram { sig, .. } => sig
                .params
                .iter()
                .map(|p| self.a.decl(p.decl).spelling.clone())
                .collect(),
            _ => Vec::new(),
        }
    }

    // --- attributes --------------------------------------------------------

    fn classify_attribute(
        &mut self,
        prefix: &Name,
        attribute: &ast::Ident,
        signature: Option<&ast::Signature>,
        span: Span,
        mode: Mode,
        args: Option<&[AssociationElement]>,
    ) -> Prefix {
        let _ = signature;
        let Some(attr) = Predefined::from_name(&attribute.name) else {
            return self.user_attribute(prefix, attribute, span, mode);
        };
        let p = self.classify(prefix, mode, None);
        let (ty, obj) = match &p {
            Prefix::Type(t) => (*t, None),
            Prefix::Object(o, t) => (*t, Some(*o)),
            Prefix::Value(t) => (*t, None),
            Prefix::Error => return Prefix::Error,
            _ => {
                if mode == Mode::Commit {
                    self.error(
                        "V0206",
                        prefix.span(),
                        format!(
                            "`'{}` needs a type, object or signal prefix",
                            attribute.name
                        ),
                    );
                }
                return Prefix::Error;
            }
        };
        let b = self.a.builtins;
        // Argument handling.
        let arg_exprs: Vec<&Expr> = args
            .map(|a| {
                a.iter()
                    .filter_map(|e| match &e.actual {
                        Actual::Expr(x) => Some(x),
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default();
        if attr.arg() == Arg::Required && arg_exprs.is_empty() {
            if mode == Mode::Commit {
                self.error(
                    "V0502",
                    span,
                    format!("`'{}` needs an argument", attr.name()),
                );
            }
            return Prefix::Error;
        }
        if attr.arg() == Arg::None && !arg_exprs.is_empty() && mode == Mode::Commit {
            self.error(
                "V0502",
                span,
                format!("`'{}` takes no argument", attr.name()),
            );
        }
        if attr.prefix() == PrefixKind::Signal {
            if mode == Mode::Commit {
                match obj {
                    Some(o) if o.class == ObjectClass::Signal => {}
                    Some(o) => {
                        let dspan = self.a.decl(o.decl).span;
                        self.push(
                            Diagnostic::error(format!(
                                "`'{}` applies to a signal, but this is a {}",
                                attr.name(),
                                o.class.as_str()
                            ))
                            .with_code("V0206")
                            .with_label(span, "not a signal")
                            .with_secondary(
                                dspan,
                                format!("declared as a {} here", o.class.as_str()),
                            ),
                        );
                    }
                    None => self.error(
                        "V0206",
                        span,
                        format!("`'{}` applies to a signal", attr.name()),
                    ),
                }
                // Reading a signal attribute inside a pure function is a
                // side-channel the standard forbids for `'delayed` and
                // friends only through the general signal rules; the
                // sensitivity of `'event` is handled by the process check.
            }
            let result = match attr {
                Predefined::Delayed | Predefined::Transaction => ty,
                Predefined::Stable
                | Predefined::Quiet
                | Predefined::Event
                | Predefined::Active
                | Predefined::Driving => b.boolean,
                Predefined::LastEvent | Predefined::LastActive => b.time,
                Predefined::LastValue | Predefined::DrivingValue => ty,
                _ => b.boolean,
            };
            if mode == Mode::Commit {
                for e in &arg_exprs {
                    self.resolve(e, b.time);
                }
                self.a.set_type(span, result);
                self.a.set_call(span, CallTarget::Attribute(attr));
            }
            return Prefix::Value(result);
        }
        if attr.prefix() == PrefixKind::Entity {
            if mode == Mode::Commit {
                self.a.set_type(span, b.string);
                self.a.set_call(span, CallTarget::Attribute(attr));
            }
            return Prefix::Value(b.string);
        }
        if attr.is_type_valued() {
            let t = match attr {
                Predefined::Base => self.a.base_type(ty),
                Predefined::Subtype => ty,
                Predefined::Element => {
                    self.require_2008(span, "the `'element` attribute");
                    match self.a.element_type(ty) {
                        Some(e) => e,
                        None => {
                            if mode == Mode::Commit && !self.a.is_error(ty) {
                                let tn = self.ty_name(ty);
                                self.error(
                                    "V0206",
                                    span,
                                    format!("`{tn}` is not an array type, so it has no `'element`"),
                                );
                            }
                            b.error
                        }
                    }
                }
                _ => ty,
            };
            if mode == Mode::Commit {
                self.a.set_type(span, t);
            }
            return Prefix::Type(t);
        }
        if attr.is_range() {
            if mode == Mode::Commit {
                self.error(
                    "V0304",
                    span,
                    format!(
                        "`'{}` denotes a range; use it where a range is expected",
                        attr.name()
                    ),
                );
            }
            return Prefix::Error;
        }
        // Value attributes on a type or array.
        let dim = if arg_exprs.len() == 1 && attr.arg() == Arg::Optional {
            self.static_dim(arg_exprs[0])
        } else {
            1
        };
        let is_array = self.a.class(ty) == TypeClass::Array;
        let result = match attr {
            Predefined::Length => b.universal_integer,
            Predefined::Ascending => b.boolean,
            Predefined::Image => b.string,
            Predefined::Value => ty,
            Predefined::Pos => b.universal_integer,
            Predefined::Val
            | Predefined::Succ
            | Predefined::Pred
            | Predefined::LeftOf
            | Predefined::RightOf => ty,
            Predefined::Left | Predefined::Right | Predefined::High | Predefined::Low => {
                if is_array {
                    attrs::array_attr_type(self.a, ty, attr, dim).unwrap_or(b.integer)
                } else {
                    ty
                }
            }
            _ => ty,
        };
        if mode == Mode::Infer {
            return Prefix::Value(result);
        }
        if !is_array
            && matches!(attr, Predefined::Length | Predefined::Ascending)
            && !self.a.is_scalar(ty)
            && !self.a.is_error(ty)
        {
            let tn = self.ty_name(ty);
            self.error(
                "V0206",
                span,
                format!(
                    "`'{}` needs an array or scalar prefix, but `{tn}` is neither",
                    attr.name()
                ),
            );
            return Prefix::Error;
        }
        if !is_array
            && matches!(
                attr,
                Predefined::Left | Predefined::Right | Predefined::High | Predefined::Low
            )
            && !self.a.is_scalar(ty)
            && !self.a.is_error(ty)
        {
            let tn = self.ty_name(ty);
            self.error(
                "V0206",
                span,
                format!(
                    "`'{}` needs a scalar or array prefix, but `{tn}` is neither",
                    attr.name()
                ),
            );
            return Prefix::Error;
        }
        // Resolve the argument and fold.
        let mut argv = None;
        if attr.arg() == Arg::Required && !arg_exprs.is_empty() {
            let want = match attr {
                Predefined::Image
                | Predefined::Pos
                | Predefined::Succ
                | Predefined::Pred
                | Predefined::LeftOf
                | Predefined::RightOf => ty,
                Predefined::Val => b.universal_integer,
                Predefined::Value => b.string,
                _ => ty,
            };
            self.resolve(arg_exprs[0], want);
            argv = self.a.value_of(arg_exprs[0].span()).cloned();
        }
        self.a.set_type(span, result);
        self.a.set_call(span, CallTarget::Attribute(attr));
        let folded = match (attr.arg(), &argv) {
            (Arg::Required, Some(v)) => attrs::eval_with_arg(self.a, ty, attr, v),
            _ if is_array => attrs::eval_array(self.a, ty, attr, dim),
            _ => attrs::eval_scalar(self.a, ty, attr),
        };
        if let Some(v) = folded {
            self.a.set_value(span, v);
        }
        Prefix::Value(result)
    }

    fn static_dim(&mut self, e: &Expr) -> usize {
        let ui = self.a.builtins.universal_integer;
        self.resolve(e, ui);
        self.a
            .value_of(e.span())
            .and_then(Value::as_int)
            .and_then(|v| usize::try_from(v).ok())
            .unwrap_or(1)
    }

    /// A user-defined attribute: `obj'attr`.
    fn user_attribute(
        &mut self,
        prefix: &Name,
        attribute: &ast::Ident,
        span: Span,
        mode: Mode,
    ) -> Prefix {
        let p = self.classify(prefix, mode, None);
        if matches!(p, Prefix::Error) {
            return Prefix::Error;
        }
        let sym = self.ident_sym(attribute);
        let lk = scope::lookup(self.a, self.region, sym);
        let found = lk
            .decls
            .iter()
            .copied()
            .find_map(|d| match self.a.decl(d).kind {
                DeclKind::Attribute(t) => Some((d, t)),
                _ => None,
            });
        match found {
            Some((d, t)) => {
                if mode == Mode::Commit {
                    self.a.set_ref(attribute.span, d);
                    self.a.set_type(span, t);
                    // A static attribute value is looked up through the
                    // prefix's declaration.
                    if let Prefix::Object(o, _) = &p
                        && let Some(v) = self.a.attribute_value(o.decl, sym).cloned()
                    {
                        self.a.set_value(span, v);
                    }
                }
                Prefix::Value(t)
            }
            None => {
                if mode == Mode::Commit {
                    let names = scope::visible_names(self.a, self.region);
                    let mut d =
                        Diagnostic::error(format!("unknown attribute `'{}`", attribute.name))
                            .with_code("V0200")
                            .with_span(attribute.span);
                    let known: Vec<&str> = [
                        "left",
                        "right",
                        "high",
                        "low",
                        "length",
                        "range",
                        "reverse_range",
                        "ascending",
                        "event",
                        "stable",
                        "last_value",
                        "image",
                        "value",
                        "pos",
                        "val",
                        "succ",
                        "pred",
                        "base",
                        "subtype",
                        "element",
                    ]
                    .into_iter()
                    .collect();
                    if let Some(s) = super::suggest(&attribute.name, known.into_iter()) {
                        d = d.with_note(format!("did you mean `'{s}`?"));
                    } else if let Some(s) =
                        super::suggest(&attribute.name, names.iter().map(String::as_str))
                    {
                        d = d.with_note(format!("did you mean `'{s}`?"));
                    } else {
                        d = d.with_note("user-defined attributes must be declared with `attribute name : type;`");
                    }
                    self.push(d);
                }
                Prefix::Error
            }
        }
    }

    // --- names as expressions ----------------------------------------------

    /// The candidate types of a name used as an expression.
    pub(crate) fn infer_name(&mut self, n: &Name) -> Vec<Cand> {
        match self.classify(n, Mode::Infer, None) {
            Prefix::Object(_, t) | Prefix::Value(t) => vec![Cand::Ty(t)],
            Prefix::Values(ts) => ts.into_iter().map(Cand::Ty).collect(),
            Prefix::Overloaded(ds) => ds
                .iter()
                .filter_map(|&d| match &self.a.decl(d).kind {
                    DeclKind::EnumLiteral { ty, .. } => Some(Cand::Ty(*ty)),
                    DeclKind::Subprogram { sig, .. }
                        if sig.params.iter().all(|p| p.has_default) =>
                    {
                        sig.ret.map(Cand::Ty)
                    }
                    _ => None,
                })
                .collect(),
            Prefix::Type(_) | Prefix::Region(..) | Prefix::Unit(_) | Prefix::Error => Vec::new(),
        }
    }

    /// Resolves a name used as an expression against `expected`.
    pub(crate) fn resolve_name_expr(&mut self, n: &Name, expected: TypeId, mode: Mode) -> TypeId {
        // An overloaded name (enumeration literal or parameterless
        // function) resolves against the expected type directly.
        if let Prefix::Overloaded(ds) = self.classify(n, Mode::Infer, Some(expected)) {
            let matches: Vec<DeclId> = ds
                .iter()
                .copied()
                .filter(|&d| match &self.a.decl(d).kind {
                    DeclKind::EnumLiteral { ty, .. } => self.a.same_base(*ty, expected),
                    DeclKind::Subprogram { sig, .. } => {
                        sig.params.iter().all(|p| p.has_default)
                            && sig
                                .ret
                                .is_some_and(|r| overload::types_compatible(self.a, r, expected))
                    }
                    _ => false,
                })
                .collect();
            match matches.as_slice() {
                [d] => {
                    let d = *d;
                    self.a.set_ref(n.span(), d);
                    return match self.a.decl(d).kind.clone() {
                        DeclKind::EnumLiteral { pos, .. } => {
                            self.a.set_value(n.span(), Value::Enum(pos));
                            expected
                        }
                        DeclKind::Subprogram { sig, .. } => {
                            self.a.set_call(n.span(), CallTarget::Subprogram(d));
                            self.fold_call(n.span(), d, &[], sig.ret.unwrap_or(expected));
                            sig.ret.unwrap_or(expected)
                        }
                        _ => expected,
                    };
                }
                [] => {
                    if mode == Mode::Commit {
                        let en = self.ty_name(expected);
                        let text = self.text(n.span()).to_owned();
                        let mut d = Diagnostic::error(format!("`{text}` is not a value of `{en}`"))
                            .with_code("V0300")
                            .with_span(n.span());
                        // List what the name does denote.
                        let mut kinds: Vec<String> = ds
                            .iter()
                            .filter_map(|&x| match &self.a.decl(x).kind {
                                DeclKind::EnumLiteral { ty, .. } => {
                                    Some(format!("a literal of `{}`", self.ty_name(*ty)))
                                }
                                DeclKind::Subprogram { sig, .. } => sig
                                    .ret
                                    .map(|r| format!("a function returning `{}`", self.ty_name(r))),
                                _ => None,
                            })
                            .collect();
                        kinds.sort();
                        kinds.dedup();
                        for k in kinds.into_iter().take(4) {
                            d = d.with_note(format!("`{text}` is {k}"));
                        }
                        self.push(d);
                    }
                    return self.a.builtins.error;
                }
                _ => {
                    if mode == Mode::Commit {
                        let text = self.text(n.span()).to_owned();
                        let mut d = Diagnostic::error(format!("`{text}` is ambiguous here"))
                            .with_code("V0302")
                            .with_span(n.span());
                        for &c in matches.iter().take(6) {
                            let dspan = self.a.decl(c).span;
                            d = d.with_secondary(dspan, "candidate declared here");
                        }
                        self.push(d);
                    }
                    return self.a.builtins.error;
                }
            }
        }
        match self.classify(n, mode, Some(expected)) {
            Prefix::Object(obj, t) => {
                if mode == Mode::Commit {
                    self.check_readable(&obj, n.span());
                }
                if mode == Mode::Commit
                    && !overload::types_compatible(self.a, t, expected)
                    && !self.a.is_error(t)
                    && !self.a.is_error(expected)
                {
                    let (g, e) = (self.ty_name(t), self.ty_name(expected));
                    let text = self.text(n.span()).to_owned();
                    let mut d =
                        Diagnostic::error(format!("type mismatch: expected `{e}`, found `{g}`"))
                            .with_code("V0300")
                            .with_label(n.span(), format!("`{text}` is `{g}`"));
                    if let Some(decl) = self.a.decl_of(n.span()) {
                        let dspan = self.a.decl(decl).span;
                        d = d.with_secondary(dspan, format!("declared as `{g}` here"));
                    }
                    if self.a.is_std_ulogic_array(t) && self.a.class(expected).is_integer() {
                        d = d.with_note(
                            "convert with `to_integer(unsigned(...))` from `ieee.numeric_std`",
                        );
                    }
                    if self.a.class(t).is_integer() && self.a.is_std_ulogic_array(expected) {
                        d = d.with_note("convert with `std_logic_vector(to_unsigned(x, n))` from `ieee.numeric_std`");
                    }
                    self.push(d);
                    return self.a.builtins.error;
                }
                self.propagate_constant(n.span());
                t
            }
            Prefix::Value(t) => {
                if mode == Mode::Commit
                    && !overload::types_compatible(self.a, t, expected)
                    && !self.a.is_error(t)
                    && !self.a.is_error(expected)
                {
                    let (g, e) = (self.ty_name(t), self.ty_name(expected));
                    let text = self.text(n.span()).to_owned();
                    self.push(
                        Diagnostic::error(format!("type mismatch: expected `{e}`, found `{g}`"))
                            .with_code("V0300")
                            .with_label(n.span(), format!("`{text}` is `{g}`")),
                    );
                    return self.a.builtins.error;
                }
                self.propagate_constant(n.span());
                t
            }
            Prefix::Values(_) => {
                if mode == Mode::Commit {
                    self.error("V0302", n.span(), "ambiguous call");
                }
                self.a.builtins.error
            }
            Prefix::Type(t) => {
                if mode == Mode::Commit {
                    let tn = self.ty_name(t);
                    self.error(
                        "V0206",
                        n.span(),
                        format!("`{tn}` is a type, not a value; write `{tn}'(...)` to qualify an expression"),
                    );
                }
                self.a.builtins.error
            }
            Prefix::Region(..) | Prefix::Unit(_) => {
                if mode == Mode::Commit {
                    let text = self.text(n.span()).to_owned();
                    self.error("V0206", n.span(), format!("`{text}` is not a value"));
                }
                self.a.builtins.error
            }
            Prefix::Overloaded(_) | Prefix::Error => self.a.builtins.error,
        }
    }

    /// Copies a constant declaration's static value onto the name that
    /// reads it, so `c + 1` folds when `c` is a static constant.
    fn propagate_constant(&mut self, span: Span) {
        if self.a.value_of(span).is_some() {
            return;
        }
        if let Some(decl) = self.a.decl_of(span)
            && let Some(v) = self.a.decl_value(decl).cloned()
        {
            self.a.set_value(span, v);
        }
    }

    /// Classifies a name in commit mode and reports what it denotes.
    pub(crate) fn commit_name(&mut self, n: &Name, expected: Option<TypeId>) -> NameInfo {
        match self.classify(n, Mode::Commit, expected) {
            Prefix::Object(o, t) => NameInfo {
                ty: Some(t),
                obj: Some(o),
            },
            Prefix::Value(t) => NameInfo {
                ty: Some(t),
                obj: None,
            },
            _ => NameInfo::default(),
        }
    }

    /// Classifies a name without reporting, for the object rules.
    pub(crate) fn name_info(&mut self, n: &Name) -> NameInfo {
        match self.classify(n, Mode::Infer, None) {
            Prefix::Object(o, t) => NameInfo {
                ty: Some(t),
                obj: Some(o),
            },
            Prefix::Value(t) => NameInfo {
                ty: Some(t),
                obj: None,
            },
            _ => NameInfo::default(),
        }
    }

    // --- aggregates --------------------------------------------------------

    /// Resolves an aggregate against the composite type `expected`.
    pub(crate) fn resolve_aggregate(
        &mut self,
        agg: &Aggregate,
        expected: TypeId,
        mode: Mode,
    ) -> TypeId {
        match self.a.class(expected) {
            TypeClass::Array => self.array_aggregate(agg, expected, mode),
            TypeClass::Record => self.record_aggregate(agg, expected, mode),
            _ => {
                if mode == Mode::Commit && !self.a.is_error(expected) {
                    let en = self.ty_name(expected);
                    self.error(
                        "V0300",
                        agg.span,
                        format!("`{en}` is not a composite type, so it takes no aggregate"),
                    );
                }
                self.a.builtins.error
            }
        }
    }

    fn array_aggregate(&mut self, agg: &Aggregate, expected: TypeId, mode: Mode) -> TypeId {
        self.array_aggregate_at(agg, expected, 0, mode)
    }

    /// Resolves an array aggregate that indexes dimension `dim` (0-based)
    /// of `expected`. An N-dimensional aggregate nests N levels: each
    /// element of the outer aggregate is itself an aggregate for the next
    /// dimension, and only the innermost level holds element values
    /// (clause 9.3.3.3).
    fn array_aggregate_at(
        &mut self,
        agg: &Aggregate,
        expected: TypeId,
        dim: usize,
        mode: Mode,
    ) -> TypeId {
        let dims = self.a.dimensions(expected);
        let elem = self
            .a
            .element_type(expected)
            .unwrap_or(self.a.builtins.error);
        let innermost = dim + 1 >= dims;
        let idx = self
            .a
            .array_info(expected)
            .and_then(|i| i.0.get(dim).copied())
            .unwrap_or(self.a.builtins.integer);
        let mut positional = 0usize;
        let mut has_others = false;
        let mut choice_count = 0usize;
        let mut elem_values: Vec<Option<Value>> = Vec::new();
        let mut named = false;
        let mut all_choices_simple = true;

        for (i, el) in agg.elements.iter().enumerate() {
            if el.choices.is_empty() {
                if named && mode == Mode::Commit {
                    self.error(
                        "V0310",
                        el.span,
                        "a positional element cannot follow a named one",
                    );
                }
                positional += 1;
            } else {
                named = true;
                for c in &el.choices {
                    match c {
                        Choice::Others(s) => {
                            has_others = true;
                            if i + 1 != agg.elements.len() && mode == Mode::Commit {
                                self.error(
                                    "V0310",
                                    *s,
                                    "`others` must be the last element of an aggregate",
                                );
                            }
                        }
                        Choice::Expr(e) => {
                            if mode == Mode::Commit {
                                self.resolve(e, idx);
                            }
                            choice_count += 1;
                            if self.a.value_of(e.span()).is_none() {
                                all_choices_simple = false;
                            }
                        }
                        Choice::Range(r) => {
                            if mode == Mode::Commit {
                                self.resolve_discrete_range_in(r, Some(idx));
                            }
                            match self.a.range_of(r.span()).and_then(|i| i.bounds.length()) {
                                Some(n) => {
                                    choice_count += usize::try_from(n).unwrap_or(0);
                                }
                                None => all_choices_simple = false,
                            }
                        }
                    }
                }
            }
            if mode != Mode::Commit {
                elem_values.push(None);
                continue;
            }
            if innermost {
                self.resolve(&el.value, elem);
                self.check_static_range(&el.value, elem);
                elem_values.push(self.a.value_of(el.value.span()).cloned());
            } else {
                // The next dimension down.
                match &el.value {
                    Expr::Aggregate(inner) => {
                        self.array_aggregate_at(inner, expected, dim + 1, mode);
                    }
                    other => {
                        self.error(
                            "V0310",
                            other.span(),
                            format!(
                                "expected an aggregate for dimension {} of this array",
                                dim + 2
                            ),
                        );
                    }
                }
                elem_values.push(None);
            }
        }

        if mode != Mode::Commit {
            return expected;
        }
        // Length check against a static index constraint on this dimension.
        if let Some(len) = self
            .a
            .index_constraint(expected)
            .and_then(|c| c.get(dim).and_then(|b| b.length()))
            && !has_others
            && all_choices_simple
        {
            let n = i128::try_from(if named { choice_count } else { positional }).unwrap_or(0);
            if n != len {
                let tn = self.ty_name(expected);
                let what = if dims > 1 {
                    format!("dimension {} of `{tn}`", dim + 1)
                } else {
                    format!("`{tn}`")
                };
                self.error(
                    "V0307",
                    agg.span,
                    format!("aggregate has {n} element(s) but {what} has {len}"),
                );
            }
        }
        // Only a one-dimensional aggregate folds to a value.
        if dims == 1
            && let Some(v) = self.fold_array_aggregate(agg, expected, &elem_values, has_others)
        {
            self.a.set_value(agg.span, v);
        }
        expected
    }

    /// Folds a one-dimensional array aggregate whose elements are all
    /// static: all-positional, all-`others`, or indexed choices with an
    /// `others` for the rest. Anything else yields `None`, which simply
    /// makes the aggregate non-static.
    fn fold_array_aggregate(
        &mut self,
        agg: &Aggregate,
        expected: TypeId,
        elem_values: &[Option<Value>],
        has_others: bool,
    ) -> Option<Value> {
        let bounds = self.a.index_constraint(expected)?.first().cloned()?;
        let len = bounds.length()?;
        let n = usize::try_from(len).ok()?;
        let left = bounds.left.int()?;
        let dir = bounds.dir;
        let at = |i: i128| -> Option<usize> {
            let off = match dir {
                Direction::To => i - left,
                Direction::Downto => left - i,
            };
            usize::try_from(off).ok().filter(|&o| o < n)
        };
        let mut slots: Vec<Option<Value>> = vec![None; n];
        let mut others: Option<Value> = None;
        let mut pos = 0usize;
        for (i, el) in agg.elements.iter().enumerate() {
            let v = elem_values.get(i)?.clone();
            if el.choices.is_empty() {
                let v = v?;
                if pos >= n {
                    return None;
                }
                slots[pos] = Some(v);
                pos += 1;
                continue;
            }
            for c in &el.choices {
                match c {
                    Choice::Others(_) => others = v.clone(),
                    Choice::Expr(e) => {
                        let idx = self.a.value_of(e.span())?.as_int()?;
                        slots[at(idx)?] = v.clone();
                    }
                    Choice::Range(r) => {
                        let info = self.a.range_of(r.span())?.clone();
                        let (lo, hi) = info.bounds.low_high()?;
                        for i in lo..=hi {
                            slots[at(i)?] = v.clone();
                        }
                    }
                }
            }
        }
        if has_others && others.is_none() {
            return None;
        }
        let elems: Option<Vec<Value>> = slots
            .into_iter()
            .map(|s| s.or_else(|| others.clone()))
            .collect();
        Some(Value::Array(ArrayValue {
            left,
            dir,
            elems: elems?,
        }))
    }

    fn record_aggregate(&mut self, agg: &Aggregate, expected: TypeId, mode: Mode) -> TypeId {
        let fields: Vec<super::Field> = match self.a.record_fields(expected) {
            Some(f) => f.to_vec(),
            None => return self.a.builtins.error,
        };
        let mut values: Vec<Option<Value>> = vec![None; fields.len()];
        let mut filled = vec![false; fields.len()];
        let mut pos = 0usize;
        let mut named = false;
        for el in &agg.elements {
            if el.choices.is_empty() {
                if named && mode == Mode::Commit {
                    self.error(
                        "V0310",
                        el.span,
                        "a positional element cannot follow a named one",
                    );
                }
                if pos >= fields.len() {
                    if mode == Mode::Commit {
                        let tn = self.ty_name(expected);
                        self.error(
                            "V0307",
                            el.span,
                            format!("`{tn}` has only {} element(s)", fields.len()),
                        );
                    }
                    break;
                }
                let fty = self
                    .a
                    .field_type(expected, fields[pos].name)
                    .unwrap_or(fields[pos].ty);
                if mode == Mode::Commit {
                    self.resolve(&el.value, fty);
                    values[pos] = self.a.value_of(el.value.span()).cloned();
                }
                filled[pos] = true;
                pos += 1;
                continue;
            }
            named = true;
            // Named: each choice is an element name or `others`.
            let mut targets: Vec<usize> = Vec::new();
            let mut others = false;
            for c in &el.choices {
                match c {
                    Choice::Others(_) => others = true,
                    Choice::Expr(Expr::Name(Name::Simple(i))) => {
                        let sym = self.ident_sym(i);
                        match fields.iter().position(|f| f.name == sym) {
                            Some(p) => {
                                targets.push(p);
                                self.a.set_type(i.span, fields[p].ty);
                            }
                            None => {
                                if mode == Mode::Commit {
                                    let tn = self.ty_name(expected);
                                    let names: Vec<String> = fields
                                        .iter()
                                        .map(|f| self.a.name(f.name).to_owned())
                                        .collect();
                                    let mut d = Diagnostic::error(format!(
                                        "`{tn}` has no element `{}`",
                                        i.name
                                    ))
                                    .with_code("V0202")
                                    .with_span(i.span);
                                    if let Some(s) =
                                        super::suggest(&i.name, names.iter().map(String::as_str))
                                    {
                                        d = d.with_note(format!("did you mean `{s}`?"));
                                    }
                                    self.push(d);
                                }
                            }
                        }
                    }
                    other => {
                        if mode == Mode::Commit {
                            self.error(
                                "V0310",
                                other.span(),
                                "a record aggregate is indexed by element names",
                            );
                        }
                    }
                }
            }
            if others {
                for (p, f) in filled.iter_mut().enumerate() {
                    if !*f {
                        targets.push(p);
                        *f = true;
                    }
                }
            }
            if mode == Mode::Commit {
                // Every target must have the same type for one value.
                if let Some(&first) = targets.first() {
                    let fty = self
                        .a
                        .field_type(expected, fields[first].name)
                        .unwrap_or(fields[first].ty);
                    self.resolve(&el.value, fty);
                    let v = self.a.value_of(el.value.span()).cloned();
                    for &t in &targets {
                        values[t] = v.clone();
                        filled[t] = true;
                    }
                }
            } else {
                for &t in &targets {
                    filled[t] = true;
                }
            }
        }
        if mode == Mode::Commit {
            let missing: Vec<String> = fields
                .iter()
                .zip(&filled)
                .filter(|&(_, &f)| !f)
                .map(|(f, _)| self.a.name(f.name).to_owned())
                .collect();
            if !missing.is_empty() {
                let tn = self.ty_name(expected);
                self.error(
                    "V0307",
                    agg.span,
                    format!(
                        "aggregate for `{tn}` is missing element{} {}",
                        if missing.len() == 1 { "" } else { "s" },
                        missing.join(", ")
                    ),
                );
            } else if values.iter().all(Option::is_some) {
                let v = Value::Record(values.into_iter().flatten().collect());
                self.a.set_value(agg.span, v);
            }
        }
        expected
    }

    /// Reports an attempt to write to an object that cannot be written.
    pub(crate) fn check_writable(&mut self, obj: &ObjInfo, span: Span, what: &str) {
        let dspan = self.a.decl(obj.decl).span;
        let spelling = self.a.decl(obj.decl).spelling.clone();
        match obj.class {
            ObjectClass::Constant => {
                let (word, hint) = match obj.role {
                    ObjectRole::Generic => ("generic", "generics are constants"),
                    ObjectRole::LoopParam => ("loop parameter", "a loop parameter is a constant"),
                    ObjectRole::GenerateParam => {
                        ("generate parameter", "a generate parameter is a constant")
                    }
                    ObjectRole::Parameter => ("`in` parameter", "`in` parameters are constants"),
                    _ => ("constant", "constants cannot be assigned"),
                };
                self.push(
                    Diagnostic::error(format!("cannot assign to {word} `{spelling}`"))
                        .with_code("V0401")
                        .with_label(span, format!("cannot be {what}"))
                        .with_secondary(dspan, format!("`{spelling}` is declared here"))
                        .with_note(hint),
                );
            }
            _ => {
                if let Some(m) = obj.mode
                    && m == ast::Mode::In
                    && matches!(obj.role, ObjectRole::Port | ObjectRole::Parameter)
                {
                    let word = if obj.role == ObjectRole::Port {
                        "port"
                    } else {
                        "parameter"
                    };
                    self.push(
                        Diagnostic::error(format!("cannot assign to `in` {word} `{spelling}`"))
                            .with_code("V0401")
                            .with_label(span, format!("`{spelling}` cannot be {what}"))
                            .with_secondary(dspan, "declared `in` here")
                            .with_note(format!(
                                "change the mode to `out` or `inout` to drive this {word}"
                            )),
                    );
                }
            }
        }
    }

    /// Reports an attempt to read an object that cannot be read.
    pub(crate) fn check_readable(&mut self, obj: &ObjInfo, span: Span) {
        let Some(m) = obj.mode else { return };
        if m != ast::Mode::Out || !matches!(obj.role, ObjectRole::Port | ObjectRole::Parameter) {
            return;
        }
        // VHDL-2008 allows reading an `out` port (clause 6.5.2).
        if self.v2008() && obj.role == ObjectRole::Port {
            return;
        }
        let decl = self.a.decl(obj.decl);
        let dspan = decl.span;
        let spelling = decl.spelling.clone();
        let word = if obj.role == ObjectRole::Port {
            "port"
        } else {
            "parameter"
        };
        let mut d = Diagnostic::error(format!("cannot read `out` {word} `{spelling}`"))
            .with_code("V0402")
            .with_label(span, "read here")
            .with_secondary(dspan, "declared `out` here");
        if obj.role == ObjectRole::Port {
            d = d.with_note("use mode `buffer`, or keep an internal signal and assign it to the port (VHDL-2008 allows reading `out` ports)");
        } else {
            d = d.with_note("use mode `inout` to read the parameter");
        }
        self.push(d);
    }

    /// Folds a call to one of the bundled library functions when every
    /// argument is static.
    pub(crate) fn fold_call(&mut self, span: Span, d: DeclId, args: &[Span], ret: TypeId) {
        if let Some(v) = self.builtin_call(d, args, ret) {
            self.a.set_value(span, v);
        }
    }
}
