//! Name visibility (IEEE 1076-2008 clause 12).
//!
//! Every declarative part is a [`Region`](super::Region) whose parent is
//! the enclosing region (an architecture's parent is its entity, whose
//! parent is the unit's context clause, whose parent is the root holding
//! the library names and `std.standard`). Looking a designator up walks
//! that chain:
//!
//! 1. In each region, the *directly visible* declarations (clause 12.3)
//!    are consulted before the *potentially visible* ones made visible by
//!    `use` clauses (clause 12.4): a local declaration hides a `use`d one.
//! 2. A non-overloadable declaration (an object, type, package, label,
//!    ...) hides every declaration of the same designator in outer regions
//!    and ends the walk.
//! 3. Overloadable declarations (subprograms and enumeration literals,
//!    clause 12.3) accumulate across regions, so a `"+"` from a package
//!    and a user-defined `"+"` are both candidates; an inner declaration
//!    hides an outer *homograph* (one with the same parameter and result
//!    type profile), which [`lookup`] drops as it walks outwards.
//! 4. Two potentially visible non-overloadable declarations with the same
//!    designator from different packages make the name ambiguous rather
//!    than visible (clause 12.4: "none of them is visible"); the lookup
//!    reports this so the checker can name both packages.

use crate::intern::Symbol;

use super::{Analysis, DeclId, DeclKind, RegionId};

/// The result of looking a designator up.
#[derive(Clone, Debug, Default)]
pub struct Lookup {
    /// The visible declarations, innermost first. Empty when the name is
    /// unknown.
    pub decls: Vec<DeclId>,
    /// Non-overloadable declarations made potentially visible by different
    /// `use` clauses in the same region; the name is not visible at all.
    pub conflicting: Vec<DeclId>,
}

impl Lookup {
    /// True when nothing was found (conflicts included).
    pub fn is_empty(&self) -> bool {
        self.decls.is_empty() && self.conflicting.is_empty()
    }

    /// The single declaration, if exactly one was found.
    pub fn single(&self) -> Option<DeclId> {
        match self.decls.as_slice() {
            [d] => Some(*d),
            _ => None,
        }
    }
}

/// True for declarations that may share a designator (clause 12.3).
pub fn is_overloadable(kind: &DeclKind) -> bool {
    matches!(
        kind,
        DeclKind::Subprogram { .. } | DeclKind::EnumLiteral { .. }
    )
}

/// True when the two declarations are homographs: overloadable, same
/// designator, and the same parameter and result type profile
/// (clause 4.5.1). Aliases of subprograms are compared through their
/// targets.
pub fn is_homograph(a: &Analysis, x: DeclId, y: DeclId) -> bool {
    let (dx, dy) = (a.decl(x), a.decl(y));
    if dx.name != dy.name {
        return false;
    }
    match (&dx.kind, &dy.kind) {
        (DeclKind::EnumLiteral { ty: tx, .. }, DeclKind::EnumLiteral { ty: ty_, .. }) => {
            a.same_base(*tx, *ty_)
        }
        (DeclKind::Subprogram { sig: sx, .. }, DeclKind::Subprogram { sig: sy, .. }) => {
            sx.kind == sy.kind
                && sx.params.len() == sy.params.len()
                && sx
                    .params
                    .iter()
                    .zip(&sy.params)
                    .all(|(p, q)| a.same_base(p.ty, q.ty))
                && match (sx.ret, sy.ret) {
                    (None, None) => true,
                    (Some(r), Some(s)) => a.same_base(r, s),
                    _ => false,
                }
        }
        (DeclKind::EnumLiteral { ty, .. }, DeclKind::Subprogram { sig, .. })
        | (DeclKind::Subprogram { sig, .. }, DeclKind::EnumLiteral { ty, .. }) => {
            // An enumeration literal is a parameterless function
            // returning its type.
            sig.params.is_empty() && sig.ret.is_some_and(|r| a.same_base(r, *ty))
        }
        _ => false,
    }
}

/// Looks `name` up from `region` outwards.
pub fn lookup(a: &Analysis, region: RegionId, name: Symbol) -> Lookup {
    let mut result = Lookup::default();
    let mut cur = Some(region);
    while let Some(r) = cur {
        let reg = a.region(r);
        let direct = reg.direct(name);
        if !direct.is_empty() && push_all(a, &mut result.decls, direct) {
            return result;
        }
        let used = reg.potentially_visible(name);
        if !used.is_empty() {
            // Potentially visible declarations are hidden by any directly
            // visible homograph already found in this or an inner region;
            // for non-overloadable ones, an inner direct declaration ended
            // the walk before we got here, so only conflicts among used
            // declarations remain to be checked.
            let non_ov: Vec<DeclId> = used
                .iter()
                .copied()
                .filter(|d| !is_overloadable(&a.decl(*d).kind))
                .collect();
            if !non_ov.is_empty() {
                if result.decls.is_empty() {
                    if non_ov.len() > 1 {
                        result.conflicting = non_ov;
                        return result;
                    }
                    result.decls.push(non_ov[0]);
                    return result;
                }
                // An overloadable found earlier plus a used non-overloadable
                // name: the inner overloadable declarations win.
                return result;
            }
            push_all(a, &mut result.decls, used);
        }
        cur = reg.parent;
    }
    result
}

/// Appends `found` to `acc`, skipping homographs of what is already
/// there (inner declarations hide outer homographs). Returns true when a
/// non-overloadable declaration was pushed, which ends the walk.
fn push_all(a: &Analysis, acc: &mut Vec<DeclId>, found: &[DeclId]) -> bool {
    let mut stop = false;
    for &d in found {
        if !is_overloadable(&a.decl(d).kind) {
            if acc.is_empty() {
                acc.push(d);
            }
            stop = true;
            continue;
        }
        if acc.iter().any(|&e| e == d || is_homograph(a, e, d)) {
            continue;
        }
        acc.push(d);
    }
    stop
}

/// Every designator visible from `region`, for "did you mean"
/// suggestions. Deterministic: sorted, deduplicated.
pub fn visible_names(a: &Analysis, region: RegionId) -> Vec<String> {
    let mut names = Vec::new();
    let mut cur = Some(region);
    while let Some(r) = cur {
        let reg = a.region(r);
        for n in reg.names() {
            names.push(a.name(n).to_owned());
        }
        let mut used: Vec<Symbol> = reg.used.keys().copied().collect();
        used.sort();
        for n in used {
            names.push(a.name(n).to_owned());
        }
        cur = reg.parent;
    }
    names.sort();
    names.dedup();
    names
}

/// The packages (as `library.package`) that declare `name`, for the
/// "missing use clause" hint. Deterministic: in unit order.
pub fn packages_declaring(a: &Analysis, name: Symbol) -> Vec<String> {
    let mut out = Vec::new();
    for u in &a.units {
        if u.kind != super::LibraryUnitKind::Package {
            continue;
        }
        let Some(region) = u.region else { continue };
        if !a.region(region).direct(name).is_empty() {
            out.push(format!("{}.{}", a.name(u.library), a.name(u.name)));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intern::Interner;
    use crate::source::{SourceId, SourceMap, Span};
    use crate::vhdl::sema::{ObjectClass, ObjectRole, RegionKind, TypeKind};

    fn span() -> Span {
        let mut m = SourceMap::new();
        let id: SourceId = m.add("x", "").unwrap();
        Span::new(id, 0, 0)
    }

    #[test]
    fn inner_object_hides_outer_and_used() {
        let mut a = Analysis::new_empty(Interner::new());
        let root = a.root();
        let ty = a.add_type(
            TypeKind::Integer(super::super::Bounds::int(
                0,
                crate::vhdl::ast::Direction::To,
                1,
            )),
            None,
        );
        let x = a.interner.intern_ci("x");
        let obj = |a: &mut Analysis, r| {
            a.add_decl(
                r,
                x,
                "x",
                DeclKind::Object {
                    class: ObjectClass::Constant,
                    ty,
                    mode: None,
                    role: ObjectRole::Plain,
                    deferred: false,
                },
                span(),
            )
        };
        let outer = obj(&mut a, root);
        let pkg = a.add_region(RegionKind::Package, None);
        let in_pkg = obj(&mut a, pkg);
        let inner_region = a.add_region(RegionKind::Architecture, Some(root));
        // Only `use`d in the inner region: the direct root one is outer,
        // so the used one wins at the inner region.
        a.add_use(inner_region, x, in_pkg);
        assert_eq!(lookup(&a, inner_region, x).single(), Some(in_pkg));
        let inner = obj(&mut a, inner_region);
        assert_eq!(lookup(&a, inner_region, x).single(), Some(inner));
        assert_eq!(lookup(&a, root, x).single(), Some(outer));
        // Two used non-overloadables conflict.
        let pkg2 = a.add_region(RegionKind::Package, None);
        let in_pkg2 = obj(&mut a, pkg2);
        let r2 = a.add_region(RegionKind::Architecture, None);
        a.add_use(r2, x, in_pkg);
        a.add_use(r2, x, in_pkg2);
        let l = lookup(&a, r2, x);
        assert!(l.decls.is_empty());
        assert_eq!(l.conflicting, vec![in_pkg, in_pkg2]);
        assert_eq!(visible_names(&a, inner_region), vec!["x"]);
    }
}
