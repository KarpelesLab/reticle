//! Cache keys: what a stored artefact is filed under.
//!
//! A key is the digest of a *unit of work*: everything that could change
//! the artefact, folded in a fixed order. Getting this wrong is the one
//! failure mode of a build cache that is genuinely hard to debug — a key
//! that misses an input serves a stale artefact for changed sources, and
//! the result looks like a miscompilation. So the inputs are enumerated
//! here, every one of them has a `changing_*` test in this module, and
//! nothing is folded in except through [`KeyBuilder`], whose every method
//! writes a tag before the value.
//!
//! # What goes into a key
//!
//! Every key, whatever it describes, starts with:
//!
//! 1. [`FORMAT`], the key-format version. Bump it whenever the composition
//!    below, the hash in [`super::hash`] or the artefact encoding changes;
//!    that retires every existing entry instead of misreading it.
//! 2. [`crate::VERSION`], the compiler version. A new compiler may
//!    elaborate the same source differently.
//! 3. The feature set ([`feature_set`]): the compiled-in Cargo features
//!    that can change an artefact. A build with `synth` off cannot produce
//!    the same artefacts as one with it on.
//! 4. The *kind* of unit ([`KIND_SCAN`], [`KIND_ELAB`], [`KIND_SYNTH`]), so
//!    two kinds can never alias.
//!
//! A **scan** key ([`KIND_SCAN`], see [`super::scan`]) then folds in:
//!
//! 5. The language and dialect the file is read in.
//! 6. The file's name and its full source text.
//!
//! An **elaborated module** key ([`KIND_ELAB`]) folds in:
//!
//! 5. The language and dialect (or VHDL standard) of the build.
//! 6. The elaboration options that reach the frontend: the working library
//!    name for VHDL, and whether this module is the build's designated top.
//! 7. The parameter set — the `(name, value)` overrides, in the order
//!    given, and only for the designated top, because that is the only
//!    module the frontends apply them to.
//! 8. The module's own name.
//! 9. The name and full text of every source file that defines part of it,
//!    in the order the build was given them.
//! 10. The name and full text of every *global* source file — one that
//!     defines no module of its own (a Verilog package or compilation-unit
//!     item, a VHDL package, context or configuration). These are handed to
//!     every elaboration, so they are folded into every key. That
//!     over-invalidates (editing a package rebuilds everything) and is
//!     deliberately the conservative direction.
//! 11. The keys of the module's direct dependencies, each with its name,
//!     sorted by name. This is what makes the dependency direction work:
//!     a dependency's key already covers its own sources and its own
//!     dependencies, so editing a leaf changes every key above it and
//!     nothing below it.
//!
//! A **synthesised module** key ([`KIND_SYNTH`]) folds in the elaborated
//! module's key and then every synthesis option that can change the
//! netlist; see [`mod@super::build`] for which ones and why one is left out.
//!
//! # What is deliberately *not* in a key
//!
//! - The source file's path on disk, its modification time and its size.
//!   The text is hashed instead, so moving a file or touching it changes
//!   nothing, and editing it behind the build's back cannot go unnoticed.
//!   Only the *name* the caller gave a file is folded in, because it
//!   reaches diagnostics and `` `line `` directives.
//! - Comments and whitespace. They are part of the source text, so a
//!   comment-only edit is a miss. Normalising them away would need the
//!   lexer and would make a key depend on a second parse; see `docs/cache.md`.
//! - Which modules a build was asked to produce. That selects work, it does
//!   not change an artefact.

use std::fmt;

use super::hash::{Hash128, Hasher128};

/// Version of the key composition and of the artefact encoding.
///
/// Bumping it makes every existing entry unreachable, which is the correct
/// response to a change in what a key means.
pub const FORMAT: u32 = 1;

/// Kind tag of a per-file dependency scan key (see [`super::scan`]).
pub const KIND_SCAN: &str = "scan";
/// Kind tag of an elaborated-module key.
pub const KIND_ELAB: &str = "elab";
/// Kind tag of a synthesised-module key.
pub const KIND_SYNTH: &str = "synth";

/// The Cargo features that can change an artefact, in a fixed order.
///
/// Only features a cached artefact could depend on are listed: the two
/// frontends decide whether a source can be elaborated at all, `synth`
/// decides whether a netlist can be produced, and `formal` changes what
/// `synth` reports when equivalence checking is asked for.
const FEATURES: &[(&str, bool)] = &[
    ("verilog", cfg!(feature = "verilog")),
    ("vhdl", cfg!(feature = "vhdl")),
    ("synth", cfg!(feature = "synth")),
    ("formal", cfg!(feature = "formal")),
];

/// The compiled-in feature set, as the string folded into every key.
///
/// For example `"verilog+vhdl"` in a frontends-only build. The order is
/// fixed, so the string is the same on every platform.
pub fn feature_set() -> String {
    let mut out = String::new();
    for (name, on) in FEATURES {
        if *on {
            if !out.is_empty() {
                out.push('+');
            }
            out.push_str(name);
        }
    }
    if out.is_empty() {
        out.push_str("none");
    }
    out
}

/// What a cached artefact is filed under.
///
/// A key is a [`Hash128`] with a tag saying it came from [`KeyBuilder`];
/// it prints as 32 hexadecimal digits, which is also the name a
/// filesystem-backed [`Storage`](super::Storage) would give the entry.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CacheKey(Hash128);

impl CacheKey {
    /// Wraps a digest that was computed elsewhere.
    ///
    /// Only a store reading a key back from its own text form should need
    /// this; a key that describes a unit of work comes from [`KeyBuilder`].
    pub fn from_hash(hash: Hash128) -> CacheKey {
        CacheKey(hash)
    }

    /// The underlying digest.
    pub fn hash(self) -> Hash128 {
        self.0
    }

    /// The key as 32 lowercase hexadecimal digits.
    pub fn to_hex(self) -> String {
        self.0.to_hex()
    }

    /// Parses the form [`CacheKey::to_hex`] writes.
    pub fn from_hex(text: &str) -> Option<CacheKey> {
        Hash128::from_hex(text).map(CacheKey)
    }
}

impl fmt::Display for CacheKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

/// Builds a [`CacheKey`] by folding in one named input at a time.
///
/// Every method writes its tag before its value, so an input can never be
/// mistaken for another one, and every string is length-framed (see
/// [`Hasher128`]), so no concatenation of two inputs can look like a
/// different pair.
#[derive(Clone, Debug)]
pub struct KeyBuilder {
    hasher: Hasher128,
}

impl KeyBuilder {
    /// Starts a key of the given kind, folding in the format version, the
    /// compiler version and the feature set.
    pub fn new(kind: &str) -> KeyBuilder {
        let mut hasher = Hasher128::new();
        hasher.write_str("reticle-cache");
        hasher.write_u64(u64::from(FORMAT));
        hasher.write_str(crate::VERSION);
        hasher.write_str(&feature_set());
        hasher.write_str(kind);
        KeyBuilder { hasher }
    }

    /// Folds in the module, entity or file the unit of work is about.
    pub fn subject(&mut self, name: &str) -> &mut KeyBuilder {
        self.hasher.write_str("subject");
        self.hasher.write_str(name);
        self
    }

    /// Folds in a named option whose value is text.
    pub fn option(&mut self, name: &str, value: &str) -> &mut KeyBuilder {
        self.hasher.write_str("option");
        self.hasher.write_str(name);
        self.hasher.write_str(value);
        self
    }

    /// Folds in a named option whose value is a flag.
    pub fn flag(&mut self, name: &str, value: bool) -> &mut KeyBuilder {
        self.hasher.write_str("flag");
        self.hasher.write_str(name);
        self.hasher.write_bool(value);
        self
    }

    /// Folds in a named option whose value is a number.
    pub fn number(&mut self, name: &str, value: u64) -> &mut KeyBuilder {
        self.hasher.write_str("number");
        self.hasher.write_str(name);
        self.hasher.write_u64(value);
        self
    }

    /// Folds in a parameter or generic override set, in the order given.
    ///
    /// The order is kept rather than sorted because the frontends apply
    /// overrides in order, so two orders can mean two designs.
    pub fn params(&mut self, params: &[(String, String)]) -> &mut KeyBuilder {
        self.hasher.write_str("params");
        self.hasher.write_len(params.len());
        for (name, value) in params {
            self.hasher.write_str(name);
            self.hasher.write_str(value);
        }
        self
    }

    /// Folds in one source file: the name it was given and its full text.
    pub fn source(&mut self, name: &str, text: &str) -> &mut KeyBuilder {
        self.hasher.write_str("source");
        self.hasher.write_str(name);
        self.hasher.write_str(text);
        self
    }

    /// Folds in one dependency's name and key.
    ///
    /// Callers must present dependencies in a deterministic order; see
    /// [`mod@super::build`], which sorts them by name.
    pub fn dependency(&mut self, name: &str, key: CacheKey) -> &mut KeyBuilder {
        self.hasher.write_str("dependency");
        self.hasher.write_str(name);
        self.hasher.write_hash(key.hash());
        self
    }

    /// Folds in the key of the artefact this one is derived from.
    pub fn derived_from(&mut self, key: CacheKey) -> &mut KeyBuilder {
        self.hasher.write_str("derived-from");
        self.hasher.write_hash(key.hash());
        self
    }

    /// The key of everything folded in so far.
    pub fn finish(&self) -> CacheKey {
        CacheKey(self.hasher.finish())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A key with one of everything, as a baseline the tests perturb.
    fn baseline() -> KeyBuilder {
        let mut b = KeyBuilder::new(KIND_ELAB);
        b.subject("top")
            .option("language", "verilog")
            .option("dialect", "verilog2005")
            .flag("is-top", true)
            .number("depth", 3)
            .params(&[("W".to_owned(), "8".to_owned())])
            .source("top.v", "module top; mid u(); endmodule\n")
            .source("pkg.vh", "// shared\n")
            .dependency("mid", CacheKey::from_hash(Hash128 { hi: 1, lo: 2 }));
        b
    }

    #[test]
    fn the_same_inputs_give_the_same_key() {
        assert_eq!(baseline().finish(), baseline().finish());
    }

    #[test]
    fn changing_the_kind_changes_the_key() {
        assert_ne!(
            KeyBuilder::new(KIND_ELAB).finish(),
            KeyBuilder::new(KIND_SYNTH).finish()
        );
        assert_ne!(
            KeyBuilder::new(KIND_SCAN).finish(),
            KeyBuilder::new(KIND_ELAB).finish()
        );
    }

    #[test]
    fn changing_the_subject_changes_the_key() {
        let one = |name: &str| {
            let mut b = KeyBuilder::new(KIND_ELAB);
            b.subject(name);
            b.finish()
        };
        assert_ne!(one("top"), one("mid"));
    }

    #[test]
    fn changing_an_option_changes_the_key() {
        let mut a = KeyBuilder::new(KIND_ELAB);
        a.option("dialect", "verilog2005");
        let mut b = KeyBuilder::new(KIND_ELAB);
        b.option("dialect", "systemverilog");
        assert_ne!(a.finish(), b.finish());

        // The option's *name* matters too, not just its value.
        let mut c = KeyBuilder::new(KIND_ELAB);
        c.option("standard", "verilog2005");
        assert_ne!(a.finish(), c.finish());
    }

    #[test]
    fn changing_a_flag_changes_the_key() {
        let mut a = KeyBuilder::new(KIND_ELAB);
        a.flag("is-top", true);
        let mut b = KeyBuilder::new(KIND_ELAB);
        b.flag("is-top", false);
        assert_ne!(a.finish(), b.finish());
    }

    #[test]
    fn changing_a_number_changes_the_key() {
        let mut a = KeyBuilder::new(KIND_SYNTH);
        a.number("max-iterations", 8);
        let mut b = KeyBuilder::new(KIND_SYNTH);
        b.number("max-iterations", 9);
        assert_ne!(a.finish(), b.finish());
    }

    #[test]
    fn changing_the_parameter_set_changes_the_key() {
        let one = |params: &[(&str, &str)]| {
            let owned: Vec<(String, String)> = params
                .iter()
                .map(|(a, b)| ((*a).to_owned(), (*b).to_owned()))
                .collect();
            let mut b = KeyBuilder::new(KIND_ELAB);
            b.params(&owned);
            b.finish()
        };
        assert_ne!(one(&[("W", "8")]), one(&[("W", "16")]));
        assert_ne!(one(&[("W", "8")]), one(&[("N", "8")]));
        assert_ne!(one(&[]), one(&[("W", "8")]));
        // Order is meaningful: overrides are applied in order.
        assert_ne!(
            one(&[("W", "8"), ("N", "2")]),
            one(&[("N", "2"), ("W", "8")])
        );
    }

    #[test]
    fn changing_a_source_text_changes_the_key() {
        let one = |text: &str| {
            let mut b = KeyBuilder::new(KIND_ELAB);
            b.source("top.v", text);
            b.finish()
        };
        assert_ne!(
            one("module top; endmodule\n"),
            one("module top; endmodule ")
        );
        // Including a comment-only edit: the text is what is hashed.
        assert_ne!(
            one("module top; endmodule\n"),
            one("// c\nmodule top; endmodule\n")
        );
    }

    #[test]
    fn changing_a_source_name_changes_the_key() {
        let one = |name: &str| {
            let mut b = KeyBuilder::new(KIND_ELAB);
            b.source(name, "module top; endmodule\n");
            b.finish()
        };
        assert_ne!(one("top.v"), one("top.sv"));
    }

    #[test]
    fn adding_a_source_changes_the_key() {
        let mut a = KeyBuilder::new(KIND_ELAB);
        a.source("top.v", "module top; endmodule\n");
        let mut b = KeyBuilder::new(KIND_ELAB);
        b.source("top.v", "module top; endmodule\n");
        b.source("pkg.vh", "");
        assert_ne!(a.finish(), b.finish());
    }

    #[test]
    fn splitting_a_source_differently_changes_the_key() {
        // The framing in `Hasher128` is what stops `("ab", "")` and
        // `("a", "b")` colliding.
        let mut a = KeyBuilder::new(KIND_ELAB);
        a.source("a.v", "ab");
        let mut b = KeyBuilder::new(KIND_ELAB);
        b.source("a.v", "a");
        b.source("a.v", "b");
        assert_ne!(a.finish(), b.finish());
    }

    #[test]
    fn changing_a_dependency_key_changes_the_key() {
        let one = |lo: u64| {
            let mut b = KeyBuilder::new(KIND_ELAB);
            b.dependency("mid", CacheKey::from_hash(Hash128 { hi: 1, lo }));
            b.finish()
        };
        assert_ne!(one(2), one(3));
    }

    #[test]
    fn changing_a_dependency_name_changes_the_key() {
        let one = |name: &str| {
            let mut b = KeyBuilder::new(KIND_ELAB);
            b.dependency(name, CacheKey::from_hash(Hash128 { hi: 1, lo: 2 }));
            b.finish()
        };
        assert_ne!(one("mid"), one("other"));
    }

    #[test]
    fn changing_the_parent_key_changes_the_key() {
        let one = |lo: u64| {
            let mut b = KeyBuilder::new(KIND_SYNTH);
            b.derived_from(CacheKey::from_hash(Hash128 { hi: 0, lo }));
            b.finish()
        };
        assert_ne!(one(1), one(2));
    }

    #[test]
    fn the_feature_set_is_deterministic_and_non_empty() {
        assert_eq!(feature_set(), feature_set());
        assert!(!feature_set().is_empty());
        // The string is built from a fixed table, so it never contains a
        // feature twice or in a host-dependent order.
        let set = feature_set();
        let names: Vec<&str> = set.split('+').collect();
        let mut sorted = names.clone();
        sorted.dedup();
        assert_eq!(names.len(), sorted.len());
    }

    #[test]
    fn hex_round_trips() {
        let key = baseline().finish();
        assert_eq!(CacheKey::from_hex(&key.to_hex()), Some(key));
        assert_eq!(key.to_string(), key.to_hex());
        assert_eq!(CacheKey::from_hex("not a key"), None);
    }
}
