//! Elaboration: from the analysed VHDL tree to the unified IR.
//!
//! [`elaborate`] takes an [`Analysis`] — the parse tree plus the side
//! tables [`crate::vhdl::sema`] filled in — picks a top entity, binds every
//! instantiation, evaluates every generic, unrolls every generate
//! statement and lowers the whole hierarchy to a [`crate::ir::Design`] that
//! has passed [`crate::ir::validate`]. Nothing is re-analysed and no new
//! tree is built: elaboration walks the parser's tree and asks
//! `decl_of`, `type_of`, `value_of`, `call_of` and `range_of` as it goes.
//!
//! # Order of elaboration
//!
//! 1. **Index.** The design-unit index lists the entities, their
//!    architectures in analysis order, and the configurations; a second
//!    index builds the reverse maps from span to tree node that
//!    `Bound::Dynamic` and `SubprogramBody::Vhdl` need.
//! 2. **Top selection.** `ElabOptions::top` names the entity that becomes
//!    [`crate::ir::Design::top`]; without it, the first entity that no
//!    design unit instantiates is used. Every other root is elaborated
//!    too, so a file of independent entities lowers completely, exactly as
//!    the Verilog elaborator does.
//! 3. **Binding.** An entity uses the *last analysed* architecture
//!    (clause 13.3), unless a direct entity instantiation names one, or a
//!    configuration specification or configuration declaration chooses
//!    another. A component instantiation binds to the entity of the same
//!    name in the same library (the default binding of clause 7.3.3); a
//!    component that binds to nothing is `V0702`.
//! 4. **Generics.** Values are taken in this order: the generic's default,
//!    then the instantiation's generic map, then `ElabOptions::generics`
//!    for the top. Every `Bound::Dynamic` constraint of the entity and its
//!    architecture is then evaluated against them, so
//!    `std_logic_vector(WIDTH-1 downto 0)` has a width.
//! 5. **Per entity, one module.** Generics become [`crate::ir::Param`]s,
//!    ports become nets, then the entity's declarations and passive
//!    statements, then the architecture's declarations and statements.
//!    Generate statements and blocks are unrolled *into the same module*,
//!    under a hierarchical name prefix.
//! 6. **Drivers.** A signal with more than one driver gets explicit
//!    resolution logic (see below).
//! 7. **Validation.** The result is checked with [`crate::ir::validate`];
//!    a violation is a bug in this frontend and is reported as `V0711`.
//!
//! # Uniquification and naming
//!
//! An entity is elaborated once per distinct *(architecture, generic
//! values)* pair and each variant becomes its own IR module:
//!
//! - the variant whose generics are all at their defaults, bound to the
//!   default architecture, keeps the entity's name (`counter`);
//! - a variant with different generic values appends `$<GENERIC>_<value>`
//!   for each generic that differs (`fifo$WIDTH_8$DEPTH_4`);
//! - a variant bound to a non-default architecture appends
//!   `$arch_<name>`;
//! - values are sanitised to what a bare IR name allows: `-1` becomes
//!   `m1`, anything else that is not `[A-Za-z0-9_]` becomes `_`; a
//!   collision appends `$2`, `$3` and so on.
//!
//! Generate statements do not make modules; they are flattened into the
//! enclosing one with a hierarchical prefix on every name they declare:
//!
//! | Construct                          | Prefix        |
//! |------------------------------------|---------------|
//! | `g : for i in 0 to 3 generate`     | `g(0).` … `g(3).` |
//! | `g : if c generate`                | `g.`          |
//! | `g : if c generate` with a VHDL-2008 alternative label `a` | `g(a).` |
//! | `g : case s generate when a => …`  | `g(a).`       |
//! | `b : block`                        | `b.`          |
//!
//! So the signal `t` inside `g_bit : for i in 0 to 7 generate` is the net
//! `g_bit(3).t` and an instance `u` there is `g_bit(3).u`. Such a name is
//! not a bare IR name, so the `.rtl` text quotes it (`%"g_bit(3).t"`).
//!
//! # Lowering decisions
//!
//! **Types** are laid out by `elab::types`: one bit for `std_logic`,
//! `std_ulogic`, `bit` and `boolean`, a position-encoded vector for any
//! other enumeration (with the mapping written into the `enum_type` and
//! `enum_literals` attributes), `s32` for every integer type, `s64` for a
//! physical type, one wide vector for a record (first field in the most
//! significant bits) and for a one-dimensional array of single-bit
//! elements (leftmost element in the most significant bits), and an
//! [`crate::ir::Memory`] for an array of wider elements.
//!
//! **Objects.** A signal becomes an [`crate::ir::Net`]: `wire` when a
//! concurrent assignment drives it, `reg` when a process does. A variable
//! becomes a `var` net. Signal assignment lowers to a non-blocking
//! assignment and variable assignment to a blocking one, which is exactly
//! the VHDL rule. An initial value (`signal s : bit := '1'`) becomes an
//! `initial` process, since it is assigned once before simulation starts;
//! a subprogram's local variables are instead re-initialised on entry,
//! where the language elaborates them. A constant whose value is static
//! disappears into the expressions that use it; one that is not becomes a
//! net with a continuous driver.
//!
//! **Time.** Every VHDL module carries the timescale `1 fs / 1 fs`, which
//! is VHDL's own resolution limit, so a `wait for` — whose delay the IR
//! carries without a unit — is a plain count of femtoseconds. An `after`
//! clause keeps its unit, in the largest one that divides the delay
//! exactly.
//!
//! **Port maps** accept named, positional and `open` associations, and a
//! type conversion on either side: on the actual it is an ordinary
//! expression, on the formal (`std_logic_vector(q) => a`) it names the
//! port through the conversion. An `out` or `inout` port must be connected
//! to a signal, a slice or an aggregate of them, which is what the IR
//! requires; anything else is `V0708`. The width the actual is coerced to
//! is the one the target module's net has, so a port whose subtype
//! mentions the target's own generics needs no re-evaluation here.
//!
//! **Processes.** The shape of the body decides the
//! [`crate::ir::ProcessKind`]:
//!
//! | Source                                                    | Kind |
//! |-----------------------------------------------------------|------|
//! | `process (all)`                                           | `Comb` |
//! | `process (a, b)`                                          | `Sensitive([a, b])` |
//! | `process (clk) … if rising_edge(clk) then B end if;`      | `Sequential { clocks: [pos clk] }`, body `B` |
//! | `process (clk, rst) … if rst = '1' then R elsif rising_edge(clk) then B end if;` | `Sequential { clocks: [pos clk], resets: [pos rst] }`, body kept whole |
//! | `process … wait until rising_edge(clk); B`                | `Sequential { clocks: [pos clk] }`, body `B` |
//! | a process with any other `wait`                           | `Free` |
//!
//! `falling_edge` and `clk'event and clk = '1'` are recognised the same
//! way. A process that tests more than one clock edge is `V0701`.
//!
//! **Subprograms** are inlined at the call site, the way the Verilog
//! elaborator inlines functions and tasks: the formals become variable
//! nets, the body's statements are emitted into the caller (in a
//! continuous context, into a generated combinational process), and
//! `return` assigns the result net and sets a "has returned" flag that
//! guards the rest. An unconstrained formal (`a : bit_vector`) takes its
//! width from the actual, or from the context when the actual is itself
//! unconstrained. Recursion is `V0705`.
//!
//! **Operators** go through `Analysis::call_of`. A predefined operator
//! becomes the matching [`crate::ir::BinaryOp`] with an explicit `Resize`
//! on each operand, since the IR needs equal widths; `mod` gets the
//! correction that gives it the divisor's sign, which the IR's `Mod` does
//! not. The operators and conversions of `ieee.std_logic_1164` are
//! recognised by name and lowered to the same nodes rather than having
//! their nine-state lookup tables inlined; `is_x` becomes the IR's
//! `$isunknown` call. A call into a package Reticle does not bundle yet is
//! `V0710` — in practice `use ieee.numeric_std.all` is already stopped by
//! the analyser's `V0107`, which names the package and says it is not
//! bundled.
//!
//! # `std_logic` resolution
//!
//! The IR has no resolution node, so resolution is lowered to explicit
//! logic, and only where it is needed:
//!
//! - **One driver**, the overwhelming case, produces no extra logic at
//!   all: the assignment or process drives the net directly.
//! - **One driver that is conditionally `'Z'`** — `y <= d when en = '1'
//!   else 'Z';` — becomes a [`crate::ir::CellKind::Tristate`] cell, which
//!   is the shape a bus driver has in hardware.
//! - **Several drivers of a resolved signal** (a `std_logic`, or any
//!   subtype with a resolution function) each get their own net
//!   `<signal>$d0`, `<signal>$d1`, …, and the signal is driven by one
//!   assignment holding the resolution: a driver at `'Z'` yields to the
//!   others, two driving values conflict to `x`. The lowering is
//!   `mux(ceq(d0, 1'bz), d1, mux(ceq(d1, 1'bz), d0, 1'bx))` for two
//!   drivers, folded left for more.
//! - **Several drivers of an unresolved signal** (`bit`, `integer`,
//!   `std_ulogic`) is `V0706`, as it is in VHDL itself.
//!
//! # Diagnostics
//!
//! Every diagnostic carries a `V07xx` code; the table is in [`codes`].
//!
//! # Example
//!
//! ```
//! use reticle::diag::Diagnostics;
//! use reticle::source::SourceMap;
//! use reticle::vhdl::{ElabOptions, Standard, analyze_source, elaborate};
//!
//! let src = "\
//! entity inv is port (d : in bit; q : out bit); end entity;
//! architecture rtl of inv is begin q <= not d; end architecture;";
//! let mut map = SourceMap::new();
//! let id = map.add("inv.vhd", src).unwrap();
//! let mut diags = Diagnostics::new();
//! let analysis = analyze_source(&mut map, id, Standard::Vhdl2008, &mut diags);
//! assert!(!diags.has_errors(), "{}", diags.render(&map));
//! let design = elaborate(&analysis, &ElabOptions::default(), &mut diags).unwrap();
//! let top = design.top_module().unwrap();
//! assert_eq!(top.name, "inv");
//! assert_eq!(top.assigns.len(), 1);
//! ```

pub mod codes;
mod conc;
mod eval;
mod expr;
mod lower;
mod spans;
mod stmt;
mod types;
mod units;

use std::collections::{HashMap, HashSet};

use crate::diag::{Diagnostic, Diagnostics};
use crate::ir::{self, Design, ModuleId};
use crate::source::Span;
use crate::vhdl::sema::{Analysis, DeclId, DeclKind, ObjectRole, UnitId, Value};

use spans::AstIndex;
use units::Index;

/// How a design is elaborated.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ElabOptions {
    /// The entity to use as the top of the hierarchy. When absent, the
    /// first entity that no design unit instantiates is used.
    pub top: Option<String>,
    /// Generic overrides for the top entity, as `(name, value)` pairs
    /// whose value is parsed as a VHDL literal (`8`, `16#ff#`, `2.5`,
    /// `true`) and kept as a string when it is not one.
    pub generics: Vec<(String, String)>,
    /// The library the design was compiled into; `work` when empty.
    pub library: Option<String>,
}

impl ElabOptions {
    /// Options with no top and no overrides.
    pub fn new() -> Self {
        Self::default()
    }

    /// The same options with `top` as the top entity.
    pub fn with_top(mut self, top: impl Into<String>) -> Self {
        self.top = Some(top.into());
        self
    }

    /// The same options with one more generic override.
    pub fn with_generic(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.generics.push((name.into(), value.into()));
        self
    }

    /// The working library's name.
    pub fn library_name(&self) -> &str {
        self.library.as_deref().unwrap_or("work")
    }
}

/// Elaborates an analysed design into an IR design.
///
/// Diagnostics are appended to `diags`; `None` means at least one error was
/// reported and no usable design was produced. The returned design has a
/// `top` when one could be selected, holds every root entity of the
/// sources, and satisfies [`crate::ir::validate`].
pub fn elaborate(
    analysis: &Analysis,
    opts: &ElabOptions,
    diags: &mut Diagnostics,
) -> Option<Design> {
    let mut cx = Elab::new(analysis, opts);
    let work = analysis.interner.get_ci(opts.library_name());

    let roots = cx.index.roots();
    let top = match &opts.top {
        Some(name) => {
            let sym = analysis.interner.get_ci(name);
            let found = sym.and_then(|s| {
                cx.index
                    .entities
                    .iter()
                    .map(|e| e.unit)
                    .find(|&u| analysis.units[u.index()].name == s)
            });
            match found {
                Some(u) => Some(u),
                None => {
                    let names: Vec<&str> = cx
                        .index
                        .entities
                        .iter()
                        .map(|e| analysis.name(analysis.units[e.unit.index()].name))
                        .collect();
                    cx.diags.push(
                        Diagnostic::error(format!("no entity named `{name}` to use as the top"))
                            .with_code(codes::TOP)
                            .with_note(if names.is_empty() {
                                "the design declares no entities".to_owned()
                            } else {
                                format!("the design declares: {}", names.join(", "))
                            }),
                    );
                    diags.append(&mut cx.diags);
                    return None;
                }
            }
        }
        None => roots
            .iter()
            .copied()
            .find(|&u| work.is_none_or(|w| analysis.units[u.index()].library == w))
            .or_else(|| roots.first().copied()),
    };

    if top.is_none() {
        cx.diags.push(
            Diagnostic::warning("the design declares no entity to elaborate")
                .with_code(codes::TOP)
                .with_note("elaboration produced an empty design"),
        );
    }

    let mut order: Vec<UnitId> = Vec::new();
    if let Some(t) = top {
        order.push(t);
    }
    for r in &roots {
        if !order.contains(r) && work.is_none_or(|w| analysis.units[r.index()].library == w) {
            order.push(*r);
        }
    }

    let mut top_id = None;
    for unit in order {
        let span = analysis.units[unit.index()].span;
        let overrides = if Some(unit) == top {
            cx.top_generics(unit)
        } else {
            Vec::new()
        };
        let id = cx.elaborate_entity(unit, None, overrides, span);
        if Some(unit) == top {
            top_id = id;
        }
    }
    cx.design.top = top_id;

    let mut design = cx.design;
    let mut out = cx.diags;
    if out.has_errors() {
        diags.append(&mut out);
        return None;
    }
    let problems = ir::validate::validate(&design);
    if !problems.is_empty() {
        for d in problems.iter() {
            out.push(
                Diagnostic::error(format!(
                    "internal error: lowered IR is invalid: {}",
                    d.message
                ))
                .with_code(codes::INTERNAL)
                .with_note("this is a bug in the VHDL frontend, not in the source"),
            );
        }
        diags.append(&mut out);
        return None;
    }
    diags.append(&mut out);
    design.top = top_id;
    Some(design)
}

/// One generic with the value elaboration gave it.
#[derive(Clone, Debug)]
pub(crate) struct GenericValue {
    /// The generic's declaration in the entity or component.
    pub decl: DeclId,
    /// Its value.
    pub value: Value,
}

/// The state shared by every module being lowered.
pub(crate) struct Elab<'a> {
    /// The analysed design.
    pub a: &'a Analysis,
    /// Reverse maps from span to tree node.
    pub ast: AstIndex<'a>,
    /// The design-unit index.
    pub index: Index,
    /// The caller's options.
    pub opts: ElabOptions,
    /// The design under construction.
    pub design: Design,
    /// Diagnostics raised so far.
    pub diags: Diagnostics,
    /// Elaborated variants by key.
    cache: HashMap<String, ModuleId>,
    /// Module names already used.
    names: HashSet<String>,
    /// Variant keys currently being elaborated, for recursion detection.
    stack: Vec<String>,
    /// Declarations by the span of their designator.
    pub decl_at: HashMap<Span, Vec<DeclId>>,
    /// Every declaration of the design, region by region.
    pub all_decls: Vec<DeclId>,
    /// What each alias declaration aliases.
    pub alias_target: HashMap<DeclId, DeclId>,
    /// The type of each user-defined attribute, by designator.
    pub attr_types: HashMap<crate::intern::Symbol, crate::vhdl::sema::TypeId>,
}

impl<'a> Elab<'a> {
    fn new(a: &'a Analysis, opts: &ElabOptions) -> Elab<'a> {
        let mut decl_at: HashMap<Span, Vec<DeclId>> = HashMap::new();
        let mut all_decls = Vec::new();
        for region in &a.regions {
            for &id in &region.decls {
                let slot = decl_at.entry(a.decl(id).span).or_default();
                if !slot.contains(&id) {
                    slot.push(id);
                }
                all_decls.push(id);
            }
        }
        let ast = AstIndex::build(a);
        let mut alias_target = HashMap::new();
        for &(designator, target) in ast.aliases() {
            if let Some(from) = decl_at.get(&designator).and_then(|v| v.first().copied())
                && let Some(to) = a.decl_of(target)
                && from != to
            {
                alias_target.insert(from, to);
            }
        }
        let mut attr_types = HashMap::new();
        for &id in &all_decls {
            if let DeclKind::Attribute(t) = a.decl(id).kind {
                attr_types.entry(a.decl(id).name).or_insert(t);
            }
        }
        Elab {
            decl_at,
            all_decls,
            alias_target,
            attr_types,
            a,
            ast,
            index: Index::build(a),
            opts: opts.clone(),
            design: Design::new(),
            diags: Diagnostics::new(),
            cache: HashMap::new(),
            names: HashSet::new(),
            stack: Vec::new(),
        }
    }

    /// Reports a diagnostic.
    pub(crate) fn report(&mut self, d: Diagnostic) {
        self.diags.push(d);
    }

    /// Reports an error with a code and a span.
    pub(crate) fn error(&mut self, code: &'static str, span: Span, msg: impl Into<String>) {
        self.diags
            .push(Diagnostic::error(msg).with_code(code).with_span(span));
    }

    /// The generics of an entity, in declaration order.
    pub(crate) fn generics_of(&self, unit: UnitId) -> Vec<DeclId> {
        self.role_decls(unit, ObjectRole::Generic)
    }

    /// The ports of an entity, in declaration order.
    pub(crate) fn ports_of(&self, unit: UnitId) -> Vec<DeclId> {
        self.role_decls(unit, ObjectRole::Port)
    }

    fn role_decls(&self, unit: UnitId, want: ObjectRole) -> Vec<DeclId> {
        let Some(region) = self.a.units[unit.index()].region else {
            return Vec::new();
        };
        self.a
            .region(region)
            .decls
            .iter()
            .copied()
            .filter(
                |&d| matches!(self.a.decl(d).kind, DeclKind::Object { role, .. } if role == want),
            )
            .collect()
    }

    /// The `--top` generic overrides, matched to the top entity's
    /// generics by name.
    fn top_generics(&mut self, unit: UnitId) -> Vec<GenericValue> {
        let mut out = Vec::new();
        if self.opts.generics.is_empty() {
            return out;
        }
        let span = self.a.units[unit.index()].span;
        let generics = self.generics_of(unit);
        let pairs = self.opts.generics.clone();
        for (name, text) in pairs {
            let found = generics.iter().copied().find(|&d| {
                self.a
                    .interner
                    .get_ci(&name)
                    .is_some_and(|s| self.a.decl(d).name == s)
            });
            let Some(decl) = found else {
                self.report(
                    Diagnostic::error(format!("the top entity has no generic named `{name}`"))
                        .with_code(codes::GENERIC)
                        .with_span(span),
                );
                continue;
            };
            let ty = self.a.decl_type(decl);
            match parse_generic_text(&text, self.a, ty) {
                Some(v) => out.push(GenericValue { decl, value: v }),
                None => self.report(
                    Diagnostic::error(format!("`{text}` is not a value for generic `{name}`"))
                        .with_code(codes::GENERIC)
                        .with_span(span),
                ),
            }
        }
        out
    }

    /// Elaborates one entity with one architecture and one generic set,
    /// reusing an existing variant.
    pub(crate) fn elaborate_entity(
        &mut self,
        entity: UnitId,
        architecture: Option<UnitId>,
        generics: Vec<GenericValue>,
        span: Span,
    ) -> Option<ModuleId> {
        let default_arch = self.index.architecture_of(entity);
        let arch = architecture.or(default_arch);
        let Some(arch) = arch else {
            let name = self.a.name(self.a.units[entity.index()].name).to_owned();
            self.report(
                Diagnostic::error(format!("entity `{name}` has no architecture"))
                    .with_code(codes::NO_ARCHITECTURE)
                    .with_span(span),
            );
            return None;
        };
        let key = self.variant_key(entity, arch, &generics);
        if let Some(&id) = self.cache.get(&key) {
            return Some(id);
        }
        if self.stack.contains(&key) {
            let name = self.a.name(self.a.units[entity.index()].name).to_owned();
            self.report(
                Diagnostic::error(format!("entity `{name}` instantiates itself"))
                    .with_code(codes::HIERARCHY)
                    .with_label(span, "recursive instantiation"),
            );
            return None;
        }
        self.stack.push(key.clone());
        let id = lower::lower_entity(self, entity, arch, generics, span, &key);
        self.stack.retain(|k| k != &key);
        id
    }

    /// The key identifying one variant of one entity.
    fn variant_key(&self, entity: UnitId, arch: UnitId, generics: &[GenericValue]) -> String {
        let a = self.a;
        let lib = a.name(a.units[entity.index()].library);
        let ent = a.name(a.units[entity.index()].name);
        let arch_name = a.name(a.units[arch.index()].name);
        let mut parts: Vec<String> = generics
            .iter()
            .map(|g| format!("{}={}", a.decl(g.decl).spelling, g.value))
            .collect();
        parts.sort();
        format!("{lib}.{ent}({arch_name})#{}", parts.join(","))
    }

    /// Registers a finished module under a name derived from the entity,
    /// the architecture and the generics that differ from their defaults.
    pub(crate) fn module_name(
        &mut self,
        entity: UnitId,
        arch: UnitId,
        applied: &[(String, String)],
    ) -> String {
        let a = self.a;
        let mut name = a.name(a.units[entity.index()].name).to_owned();
        if self.index.architecture_of(entity) != Some(arch) {
            name.push_str("$arch_");
            name.push_str(&sanitise(a.name(a.units[arch.index()].name)));
        }
        for (g, v) in applied {
            name.push('$');
            name.push_str(&sanitise(g));
            name.push('_');
            name.push_str(&sanitise(v));
        }
        if !self.names.contains(&name) {
            return name;
        }
        for n in 2.. {
            let candidate = format!("{name}${n}");
            if !self.names.contains(&candidate) {
                return candidate;
            }
        }
        name
    }

    /// Records a finished module under its variant key.
    pub(crate) fn register(&mut self, key: &str, name: String, module: ir::Module) -> ModuleId {
        self.names.insert(name);
        let id = self.design.add_module(module);
        self.cache.insert(key.to_owned(), id);
        id
    }
}

/// Maps a generic value's text to the characters a bare IR name allows.
pub(crate) fn sanitise(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for (i, c) in text.chars().enumerate() {
        match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '_' => out.push(c),
            '-' if i == 0 => out.push('m'),
            _ => out.push('_'),
        }
    }
    out
}

/// Parses a `--generic NAME=VALUE` override into a static value of `ty`.
fn parse_generic_text(
    text: &str,
    a: &Analysis,
    ty: Option<crate::vhdl::sema::TypeId>,
) -> Option<Value> {
    use crate::vhdl::sema::constant::{parse_integer_literal, parse_real_literal};
    let text = text.trim();
    if let Some(ty) = ty {
        if a.is_boolean(ty) {
            match text.to_ascii_lowercase().as_str() {
                "true" => return Some(Value::from_bool(true)),
                "false" => return Some(Value::from_bool(false)),
                _ => {}
            }
        }
        if a.is_string_type(ty) {
            let s = text.trim_matches('"');
            return Some(Value::string(s.chars().map(|c| u32::from(c as u8))));
        }
    }
    if let Some(i) = parse_integer_literal(text) {
        return Some(Value::Int(i));
    }
    if let Some(r) = parse_real_literal(text) {
        return Some(Value::Real(r));
    }
    match text.to_ascii_lowercase().as_str() {
        "true" => Some(Value::from_bool(true)),
        "false" => Some(Value::from_bool(false)),
        _ => Some(Value::string(
            text.trim_matches('"').chars().map(|c| u32::from(c as u8)),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::SourceMap;
    use crate::vhdl::{Standard, analyze_source};

    /// Analyses and elaborates one snippet, returning the design (if any)
    /// and the rendered diagnostics.
    pub(crate) fn elab(src: &str) -> (Option<Design>, String) {
        elab_with(src, &ElabOptions::default())
    }

    /// Like [`elab`] with explicit options.
    pub(crate) fn elab_with(src: &str, opts: &ElabOptions) -> (Option<Design>, String) {
        let mut map = SourceMap::new();
        let id = map.add("t.vhd", src).unwrap();
        let mut diags = Diagnostics::new();
        let analysis = analyze_source(&mut map, id, Standard::Vhdl2008, &mut diags);
        assert!(
            !diags.has_errors(),
            "the snippet must analyse:\n{}",
            diags.render(&map)
        );
        let design = elaborate(&analysis, opts, &mut diags);
        diags.sort();
        (design, diags.render(&map))
    }

    /// The `.rtl` text of an elaborated snippet.
    pub(crate) fn rtl(src: &str) -> String {
        let (design, diags) = elab(src);
        let design = design.unwrap_or_else(|| panic!("elaboration failed:\n{diags}"));
        design.to_text()
    }

    #[test]
    fn elaborates_an_inverter() {
        let out = rtl(
            "entity inv is port (d : in bit; q : out bit); end entity;\n\
             architecture rtl of inv is begin q <= not d; end architecture;",
        );
        assert!(out.contains("top inv"), "{out}");
        assert!(out.contains("assign %q = not(%d)"), "{out}");
    }

    #[test]
    fn options_build() {
        let o = ElabOptions::new().with_top("top").with_generic("W", "8");
        assert_eq!(o.top.as_deref(), Some("top"));
        assert_eq!(o.generics, [("W".to_owned(), "8".to_owned())]);
        assert_eq!(o.library_name(), "work");
    }

    #[test]
    fn reports_a_missing_top() {
        let opts = ElabOptions::new().with_top("nope");
        let (design, diags) = elab_with("entity e is end entity;", &opts);
        assert!(design.is_none());
        assert!(diags.contains("V0700"), "{diags}");
    }

    #[test]
    fn sanitises_names() {
        assert_eq!(sanitise("-1"), "m1");
        assert_eq!(sanitise("a b"), "a_b");
        assert_eq!(sanitise("W8"), "W8");
    }

    #[test]
    fn a_generic_overrides_a_default_and_renames_the_module() {
        let src = "entity g is generic (W : positive := 4);\n\
                   port (d : in bit_vector(W - 1 downto 0);\n\
                         q : out bit_vector(W - 1 downto 0)); end entity;\n\
                   architecture rtl of g is begin q <= d; end architecture;";
        assert!(rtl(src).contains("net %d u4 wire"));
        let opts = ElabOptions::new().with_generic("W", "16");
        let (design, diags) = elab_with(src, &opts);
        let out = design.expect(&diags).to_text();
        assert!(out.contains("module g$W_16"), "{out}");
        assert!(out.contains("net %d u16 wire"), "{out}");
    }

    #[test]
    fn an_enumeration_is_encoded_by_position() {
        let out = rtl("entity e is port (q : out bit); end entity;\n\
             architecture rtl of e is\n\
               type t is (a, b, c);\n\
               signal s : t := c;\n\
             begin\n\
               q <= '1' when s = b else '0';\n\
             end architecture;");
        assert!(out.contains("attr enum_literals = \"a,b,c\""), "{out}");
        assert!(out.contains("net %s u2 reg"), "{out}");
        assert!(out.contains("eq(%s, 2'd1)"), "{out}");
        assert!(out.contains("%s = 2'd2"), "{out}");
    }

    #[test]
    fn a_generate_names_every_copy() {
        let out = rtl("entity g is port (d : in bit_vector(1 downto 0);\n\
                               q : out bit_vector(1 downto 0)); end entity;\n\
             architecture rtl of g is begin\n\
               b : for i in 0 to 1 generate\n\
                 signal t : bit;\n\
               begin\n\
                 t <= not d(i);\n\
                 q(i) <= t;\n\
               end generate;\n\
             end architecture;");
        assert!(out.contains("net %\"b(0).t\" u1 wire"), "{out}");
        assert!(out.contains("net %\"b(1).t\" u1 wire"), "{out}");
    }

    #[test]
    fn several_drivers_of_a_resolved_signal_get_resolution_logic() {
        let out = rtl("library ieee; use ieee.std_logic_1164.all;\n\
             entity r is port (a, b, ea, eb : in std_logic; y : out std_logic); end entity;\n\
             architecture rtl of r is signal s : std_logic; begin\n\
               s <= a when ea = '1' else 'Z';\n\
               s <= b when eb = '1' else 'Z';\n\
               y <= s;\n\
             end architecture;");
        assert!(out.contains("net %s$d0 u1 wire"), "{out}");
        assert!(out.contains("net %s$d1 u1 wire"), "{out}");
        assert!(out.contains("ceq(%s$d0, 1'bz)"), "{out}");
        assert!(out.contains("tristate"), "{out}");
    }

    #[test]
    fn a_single_driver_needs_no_resolution() {
        let out = rtl("library ieee; use ieee.std_logic_1164.all;\n\
             entity r is port (a : in std_logic; y : out std_logic); end entity;\n\
             architecture rtl of r is begin y <= a; end architecture;");
        assert!(out.contains("assign %y = %a"), "{out}");
        assert!(!out.contains("$d0"), "{out}");
        assert!(!out.contains("ceq"), "{out}");
    }

    #[test]
    fn a_wide_element_array_becomes_a_memory() {
        let out = rtl(
            "entity m is port (i : in integer range 0 to 3; q : out bit_vector(3 downto 0));\n\
             end entity;\n\
             architecture rtl of m is\n\
               type ram_t is array (0 to 3) of bit_vector(3 downto 0);\n\
               signal ram : ram_t;\n\
             begin\n\
               q <= ram(i);\n\
             end architecture;",
        );
        assert!(out.contains("memory @ram 4 x u4"), "{out}");
        assert!(out.contains("@ram[%i]"), "{out}");
    }

    #[test]
    fn an_unresolved_signal_with_two_drivers_is_reported() {
        let (design, diags) = elab(
            "entity d is port (a, b : in bit; y : out bit); end entity;\n\
             architecture rtl of d is signal s : bit; begin\n\
               s <= a; s <= b; y <= s;\n\
             end architecture;",
        );
        assert!(design.is_none());
        assert!(diags.contains("V0706"), "{diags}");
    }
}
