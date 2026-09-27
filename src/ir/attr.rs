//! Attributes: ordered key/value metadata on every IR object.
//!
//! Attributes carry source annotations (`(* keep *)`, `(* ram_style =
//! "block" *)`, VHDL attribute specifications) and pass-to-pass hints
//! through the pipeline. They are stored as an ordered list so output is
//! deterministic and so the text format round-trips exactly; lookups are
//! linear, which is fine for the handful of attributes real objects carry.
//!
//! The same container is used for parameter overrides on instances and
//! cells, where the keys are parameter names.

use std::fmt;

use super::Name;
use super::types::Const;

/// The value of an attribute or parameter.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum AttrValue {
    /// A sized bit vector (`8'd3`), the usual result of an HDL expression.
    Const(Const),
    /// A string (`"block"`).
    String(String),
    /// An unsized integer, for attributes whose width is meaningless
    /// (`keep = 1`, `max_fanout = 16`).
    Int(i64),
}

impl AttrValue {
    /// The value as an integer, if it is an `Int` or a two-state `Const`
    /// that fits in 63 bits.
    pub fn as_int(&self) -> Option<i64> {
        match self {
            AttrValue::Int(v) => Some(*v),
            AttrValue::Const(c) => c.to_u64().and_then(|v| i64::try_from(v).ok()),
            AttrValue::String(_) => None,
        }
    }

    /// The value as a string slice, if it is a `String`.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            AttrValue::String(s) => Some(s),
            _ => None,
        }
    }

    /// True for a value that HDL attribute conventions treat as set:
    /// a non-zero integer or constant, or a string that does not spell a
    /// falsehood.
    ///
    /// The textual forms matter because the vendors that invented these
    /// flags write them as strings: `(* keep = "true" *)` and
    /// `(* keep = "false" *)` are both in the wild, and reading the second
    /// as set would keep exactly what the author asked to be dropped.
    /// `"0"`, `"false"`, `"no"`, `"off"` and an empty or blank string are
    /// false; every other string is true, since `ram_style = "block"` must
    /// still count as set.
    pub fn is_truthy(&self) -> bool {
        match self {
            AttrValue::Int(v) => *v != 0,
            AttrValue::Const(c) => !c.is_zero(),
            AttrValue::String(s) => !matches!(
                s.trim().to_ascii_lowercase().as_str(),
                "" | "0" | "false" | "no" | "off"
            ),
        }
    }
}

/// The attribute spellings that mean "keep this object in the netlist".
///
/// One intent, six vendors: `keep` is Yosys's and the one Reticle writes
/// itself, `dont_touch` and `mark_debug` are Vivado's, `preserve` and
/// `noprune` are Quartus's, and the `syn_` forms are Synplify's. They are
/// all honoured, because a user who wrote one of them and had the object
/// deleted anyway has been given a wrong answer in silence.
///
/// Matching ignores case and the separators the vendors disagree about, so
/// `KEEP`, `DONT_TOUCH` and `dont-touch` all count; see [`is_keep_attr`].
pub const KEEP_ATTRS: &[&str] = &[
    "keep",
    "dont_touch",
    "mark_debug",
    "noprune",
    "preserve",
    "syn_keep",
    "syn_noprune",
    "syn_preserve",
];

/// Attribute names that contain a word from [`KEEP_ATTRS`] but mean
/// something else, so [`looks_like_keep`] must not report them.
pub const KEEP_LOOKALIKES: &[&str] = &["keep_hierarchy"];

/// An attribute name reduced to its ASCII alphanumerics in lower case, so
/// that `DONT_TOUCH`, `dont-touch` and `donttouch` compare equal.
fn normalise(name: &str) -> String {
    name.chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// True when `name` is one of [`KEEP_ATTRS`], compared without case or
/// separators.
#[must_use]
pub fn is_keep_attr(name: &str) -> bool {
    let n = normalise(name);
    KEEP_ATTRS.iter().any(|k| normalise(k) == n)
}

/// True when `name` reads like a request to keep something but is not a
/// spelling this crate honours: `keep_signal`, `dont_touch_network`,
/// `syn_keep_hierarchy` and friends.
///
/// It is deliberately a containment test rather than an edit distance: the
/// point is to catch a plausible invention, and an attribute nobody
/// recognises is worth a word either way. Names in [`KEEP_LOOKALIKES`] are
/// real attributes with their own meaning and are never reported.
#[must_use]
pub fn looks_like_keep(name: &str) -> bool {
    if is_keep_attr(name) {
        return false;
    }
    let n = normalise(name);
    if KEEP_LOOKALIKES.iter().any(|k| normalise(k) == n) {
        return false;
    }
    ["keep", "donttouch", "markdebug", "noprune", "preserve"]
        .iter()
        .any(|word| n.contains(word))
}

impl From<i64> for AttrValue {
    fn from(v: i64) -> Self {
        AttrValue::Int(v)
    }
}

impl From<&str> for AttrValue {
    fn from(v: &str) -> Self {
        AttrValue::String(v.to_owned())
    }
}

impl From<String> for AttrValue {
    fn from(v: String) -> Self {
        AttrValue::String(v)
    }
}

impl From<Const> for AttrValue {
    fn from(v: Const) -> Self {
        AttrValue::Const(v)
    }
}

impl fmt::Display for AttrValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AttrValue::Const(c) => write!(f, "{c}"),
            AttrValue::String(s) => write!(f, "{s:?}"),
            AttrValue::Int(v) => write!(f, "{v}"),
        }
    }
}

/// An ordered set of attributes keyed by name.
///
/// Insertion order is preserved and keys are unique: [`Attrs::set`]
/// replaces an existing value in place.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Attrs {
    items: Vec<(Name, AttrValue)>,
}

impl Attrs {
    /// Creates an empty set.
    pub fn new() -> Self {
        Self::default()
    }

    /// The value stored under `key`, if any.
    pub fn get(&self, key: &str) -> Option<&AttrValue> {
        self.items
            .iter()
            .find(|(k, _)| k.as_str() == key)
            .map(|(_, v)| v)
    }

    /// True when `key` is present.
    pub fn contains(&self, key: &str) -> bool {
        self.get(key).is_some()
    }

    /// True when `key` is present with a truthy value (see
    /// [`AttrValue::is_truthy`]); the usual test for flags like `keep`.
    pub fn is_set(&self, key: &str) -> bool {
        self.get(key).is_some_and(AttrValue::is_truthy)
    }

    /// Stores `value` under `key`, replacing an existing entry in place or
    /// appending a new one.
    pub fn set(&mut self, key: impl Into<Name>, value: impl Into<AttrValue>) {
        let key = key.into();
        let value = value.into();
        match self.items.iter_mut().find(|(k, _)| *k == key) {
            Some(slot) => slot.1 = value,
            None => self.items.push((key, value)),
        }
    }

    /// Removes `key`, returning its value if it was present.
    pub fn remove(&mut self, key: &str) -> Option<AttrValue> {
        let pos = self.items.iter().position(|(k, _)| k.as_str() == key)?;
        Some(self.items.remove(pos).1)
    }

    /// Iterates over `(key, value)` pairs in insertion order.
    pub fn iter(&self) -> impl DoubleEndedIterator<Item = (&Name, &AttrValue)> + ExactSizeIterator {
        self.items.iter().map(|(k, v)| (k, v))
    }

    /// Number of attributes.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// True when no attribute is stored.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// The keep attribute this object carries, as the name that was
    /// written and whether it is on; `None` when it carries none.
    ///
    /// The written name is returned rather than a canonical one so a
    /// diagnostic can quote what the source said.
    pub fn keep_attr(&self) -> Option<(&Name, bool)> {
        self.items
            .iter()
            .find(|(k, _)| is_keep_attr(k.as_str()))
            .map(|(k, v)| (k, v.is_truthy()))
    }

    /// True when any spelling in [`KEEP_ATTRS`] is present and on: the
    /// object must survive optimisation.
    pub fn is_kept(&self) -> bool {
        self.keep_attr().is_some_and(|(_, on)| on)
    }

    /// Appends every attribute of `other`, overriding duplicates.
    pub fn extend_from(&mut self, other: &Attrs) {
        for (k, v) in other.iter() {
            self.set(k.clone(), v.clone());
        }
    }
}

impl<K: Into<Name>, V: Into<AttrValue>> FromIterator<(K, V)> for Attrs {
    fn from_iter<I: IntoIterator<Item = (K, V)>>(iter: I) -> Self {
        let mut attrs = Attrs::new();
        for (k, v) in iter {
            attrs.set(k, v);
        }
        attrs
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keep_is_recognised_however_the_vendor_spells_it() {
        for name in [
            "keep",
            "KEEP",
            "dont_touch",
            "DONT_TOUCH",
            "dont-touch",
            "donttouch",
            "mark_debug",
            "preserve",
            "noprune",
            "syn_keep",
            "syn_noprune",
            "syn_preserve",
        ] {
            assert!(is_keep_attr(name), "{name}");
            assert!(!looks_like_keep(name), "{name}");
            let attrs: Attrs = [(name, 1)].into_iter().collect();
            assert!(attrs.is_kept(), "{name}");
            assert_eq!(
                attrs.keep_attr().map(|(k, on)| (k.as_str(), on)),
                Some((name, true))
            );
        }
    }

    #[test]
    fn a_keep_that_is_off_keeps_nothing() {
        for value in [
            AttrValue::Int(0),
            AttrValue::Const(Const::from_u64(0, 1)),
            AttrValue::String("false".into()),
            AttrValue::String("FALSE".into()),
            AttrValue::String("no".into()),
            AttrValue::String("off".into()),
            AttrValue::String("0".into()),
            AttrValue::String(" ".into()),
        ] {
            let mut attrs = Attrs::new();
            attrs.set("keep", value.clone());
            assert!(!attrs.is_kept(), "{value}");
            assert_eq!(attrs.keep_attr().map(|(_, on)| on), Some(false));
        }
        let mut attrs = Attrs::new();
        attrs.set("keep", "true");
        assert!(attrs.is_kept());
        // A non-boolean flag still counts as set from its text.
        attrs.set("ram_style", "block");
        assert!(attrs.is_set("ram_style"));
    }

    #[test]
    fn an_invented_keep_is_not_mistaken_for_one() {
        for name in [
            "keep_signal",
            "keeps",
            "dont_touch_network",
            "syn_keep_hierarchy",
            "PRESERVE_DRIVER",
            "no_prune_me",
        ] {
            assert!(!is_keep_attr(name), "{name}");
            assert!(looks_like_keep(name), "{name}");
        }
        for name in ["keep_hierarchy", "ram_style", "init", "fsm_encoding"] {
            assert!(!is_keep_attr(name), "{name}");
            assert!(!looks_like_keep(name), "{name}");
        }
        assert!(Attrs::new().keep_attr().is_none());
    }

    #[test]
    fn set_get_and_order() {
        let mut attrs = Attrs::new();
        attrs.set("keep", 1);
        attrs.set("ram_style", "block");
        attrs.set("init", Const::from_u64(9, 4));
        attrs.set("keep", 0);
        assert_eq!(attrs.len(), 3);
        let keys: Vec<_> = attrs.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(keys, ["keep", "ram_style", "init"]);
        assert_eq!(attrs.get("keep").and_then(AttrValue::as_int), Some(0));
        assert!(!attrs.is_set("keep"));
        assert!(attrs.is_set("ram_style"));
        assert_eq!(
            attrs.get("ram_style").and_then(AttrValue::as_str),
            Some("block")
        );
        assert_eq!(attrs.get("init").and_then(AttrValue::as_int), Some(9));
        assert_eq!(
            attrs.remove("ram_style"),
            Some(AttrValue::String("block".into()))
        );
        assert!(!attrs.contains("ram_style"));
        assert_eq!(attrs.remove("missing"), None);
        assert_eq!(AttrValue::from("x").to_string(), "\"x\"");
        assert_eq!(AttrValue::from(-3).to_string(), "-3");
    }

    #[test]
    fn from_iter_and_extend() {
        let a: Attrs = [("a", 1i64), ("b", 2)].into_iter().collect();
        let mut b: Attrs = [("b", 5i64), ("c", 3)].into_iter().collect();
        b.extend_from(&a);
        let items: Vec<_> = b.iter().map(|(k, v)| (k.as_str(), v.as_int())).collect();
        assert_eq!(items, [("b", Some(2)), ("c", Some(3)), ("a", Some(1))]);
        assert!(Attrs::new().is_empty());
    }
}
