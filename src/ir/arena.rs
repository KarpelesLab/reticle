//! Index-based arenas and the typed ids that address them.
//!
//! Every IR object lives in an [`Arena`] owned by its parent (nets in a
//! module, modules in a design) and is referred to by a small `Copy` id
//! rather than a reference. Passes therefore mutate freely: an id stays
//! valid across edits to other objects, and there are no lifetimes to
//! thread through the data structure.
//!
//! Ids are plain `u32` newtypes, one per object kind, so a `NetId` can never
//! be used where a `CellId` is expected. They are only meaningful relative
//! to the arena that produced them; the IR never mixes ids across modules.

use std::fmt;
use std::marker::PhantomData;
use std::ops::{Index, IndexMut};

/// Narrows an arena position to the `u32` stored in ids.
///
/// Arenas are bounded by memory long before `u32::MAX` objects, so a failure
/// here is an internal invariant violation rather than a user error.
pub(crate) fn narrow(n: usize) -> u32 {
    u32::try_from(n).expect("arena index exceeds u32")
}

/// Widens a `u32` id payload to a `usize` position.
pub(crate) fn widen(n: u32) -> usize {
    // `u32` always fits in `usize` on every supported target.
    n as usize
}

/// A typed index into an [`Arena`].
pub trait Id: Copy + Eq + Ord + fmt::Debug {
    /// Builds the id addressing position `index`.
    fn from_index(index: usize) -> Self;
    /// The position this id addresses.
    fn index(self) -> usize;
}

/// Defines a `Copy` `u32` newtype implementing [`Id`].
macro_rules! define_id {
    ($(#[$meta:meta])* $name:ident, $prefix:literal) => {
        $(#[$meta])*
        #[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(pub(crate) u32);

        impl $name {
            /// The raw index of this id inside its arena.
            pub fn index(self) -> usize {
                $crate::ir::arena::widen(self.0)
            }

            /// The raw `u32` payload, for compact tables keyed by id.
            pub fn raw(self) -> u32 {
                self.0
            }
        }

        impl $crate::ir::arena::Id for $name {
            fn from_index(index: usize) -> Self {
                $name($crate::ir::arena::narrow(index))
            }

            fn index(self) -> usize {
                self.index()
            }
        }

        impl ::std::fmt::Debug for $name {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                write!(f, "{}{}", $prefix, self.0)
            }
        }

        impl ::std::fmt::Display for $name {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                write!(f, "{}{}", $prefix, self.0)
            }
        }
    };
}

pub(crate) use define_id;

/// A growable, index-addressed container of `T` keyed by ids of type `I`.
///
/// Objects are pushed and never move; removal goes through [`Arena::retain`],
/// which compacts the arena and returns a remap table so the owner can fix
/// up every id it holds.
#[derive(Clone, Debug)]
pub struct Arena<I: Id, T> {
    items: Vec<T>,
    _id: PhantomData<I>,
}

impl<I: Id, T> Default for Arena<I, T> {
    fn default() -> Self {
        Arena {
            items: Vec::new(),
            _id: PhantomData,
        }
    }
}

impl<I: Id, T> Arena<I, T> {
    /// Creates an empty arena.
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends an object and returns its id.
    pub fn push(&mut self, item: T) -> I {
        let id = I::from_index(self.items.len());
        self.items.push(item);
        id
    }

    /// The object with the given id, or `None` when the id is out of range.
    pub fn get(&self, id: I) -> Option<&T> {
        self.items.get(id.index())
    }

    /// Mutable access to the object with the given id.
    pub fn get_mut(&mut self, id: I) -> Option<&mut T> {
        self.items.get_mut(id.index())
    }

    /// True when `id` addresses an object in this arena.
    pub fn contains(&self, id: I) -> bool {
        id.index() < self.items.len()
    }

    /// Number of objects.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// True when no object has been pushed.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Iterates over `(id, object)` pairs in id order.
    pub fn iter(&self) -> impl DoubleEndedIterator<Item = (I, &T)> + ExactSizeIterator {
        self.items
            .iter()
            .enumerate()
            .map(|(i, t)| (I::from_index(i), t))
    }

    /// Iterates mutably over `(id, object)` pairs in id order.
    pub fn iter_mut(&mut self) -> impl DoubleEndedIterator<Item = (I, &mut T)> + ExactSizeIterator {
        self.items
            .iter_mut()
            .enumerate()
            .map(|(i, t)| (I::from_index(i), t))
    }

    /// Iterates over every id in order.
    pub fn ids(&self) -> impl DoubleEndedIterator<Item = I> + ExactSizeIterator + use<I, T> {
        (0..self.items.len()).map(I::from_index)
    }

    /// Iterates over the objects in id order.
    pub fn values(&self) -> std::slice::Iter<'_, T> {
        self.items.iter()
    }

    /// Finds the id of the first object satisfying `pred`.
    pub fn find(&self, pred: impl FnMut(&T) -> bool) -> Option<I> {
        self.items.iter().position(pred).map(I::from_index)
    }

    /// Keeps only the objects for which `keep` returns true, compacting the
    /// arena, and returns the remap table: `remap[old.index()]` is the new
    /// id of a kept object or `None` for a removed one.
    pub fn retain(&mut self, mut keep: impl FnMut(I, &T) -> bool) -> Vec<Option<I>> {
        let mut remap = Vec::with_capacity(self.items.len());
        let mut next = 0usize;
        let mut index = 0usize;
        self.items.retain(|t| {
            let kept = keep(I::from_index(index), t);
            index += 1;
            if kept {
                remap.push(Some(I::from_index(next)));
                next += 1;
            } else {
                remap.push(None);
            }
            kept
        });
        remap
    }
}

impl<I: Id, T> Index<I> for Arena<I, T> {
    type Output = T;

    fn index(&self, id: I) -> &T {
        &self.items[id.index()]
    }
}

impl<I: Id, T> IndexMut<I> for Arena<I, T> {
    fn index_mut(&mut self, id: I) -> &mut T {
        &mut self.items[id.index()]
    }
}

impl<'a, I: Id, T> IntoIterator for &'a Arena<I, T> {
    type Item = (I, &'a T);
    type IntoIter = Box<dyn DoubleEndedIterator<Item = (I, &'a T)> + 'a>;

    fn into_iter(self) -> Self::IntoIter {
        Box::new(self.iter())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::design::NetId as TestId;

    #[test]
    fn push_get_and_index() {
        let mut arena: Arena<TestId, &str> = Arena::new();
        let a = arena.push("a");
        let b = arena.push("b");
        assert_eq!(a.index(), 0);
        assert_eq!(b.raw(), 1);
        assert_eq!(arena[a], "a");
        assert_eq!(arena.get(b), Some(&"b"));
        assert_eq!(arena.get(TestId(7)), None);
        assert!(arena.contains(b));
        assert_eq!(arena.len(), 2);
        assert_eq!(format!("{a:?} {b}"), "n0 n1");
        arena[a] = "z";
        assert_eq!(arena.find(|s| *s == "z"), Some(a));
        let ids: Vec<_> = arena.ids().collect();
        assert_eq!(ids, [a, b]);
    }

    #[test]
    fn retain_compacts_and_remaps() {
        let mut arena: Arena<TestId, u32> = Arena::new();
        for i in 0..5 {
            arena.push(i);
        }
        let remap = arena.retain(|_, v| v % 2 == 0);
        assert_eq!(arena.len(), 3);
        assert_eq!(
            remap,
            [
                Some(TestId(0)),
                None,
                Some(TestId(1)),
                None,
                Some(TestId(2))
            ]
        );
        let values: Vec<_> = arena.values().copied().collect();
        assert_eq!(values, [0, 2, 4]);
    }
}
