//! Flat clause arena.
//!
//! Every clause lives in one `Vec<Lit>`: a three-word header followed by the
//! literals. A [`ClauseRef`] is the header's offset. Keeping clauses
//! contiguous (rather than one heap allocation per clause) is what makes
//! propagation cache-friendly: the watch lists point into one array and the
//! literals of a clause are always adjacent.
//!
//! Header layout (each word is a `Lit` reinterpreted as a raw `u32`):
//!
//! | word | meaning                                                          |
//! |------|------------------------------------------------------------------|
//! | 0    | number of literals                                               |
//! | 1    | flags (`LEARNT`, `DELETED`, `RELOCATED`) and, above them, the LBD |
//! | 2    | activity as `f32` bits, or the forwarding offset once relocated   |
//!
//! Deleted clauses are only marked; their space is counted in `wasted` and
//! reclaimed by [`ClauseDb::compact`] once enough has accumulated. That is
//! MiniSat's `RegionAllocator` + `relocAll` scheme.

use super::Lit;

/// The offset of a clause header inside the arena.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) struct ClauseRef(u32);

impl ClauseRef {
    /// The absent clause reference (no reason, no forwarding address).
    pub(super) const NONE: ClauseRef = ClauseRef(u32::MAX);

    /// True when this is [`ClauseRef::NONE`].
    pub(super) fn is_none(self) -> bool {
        self == ClauseRef::NONE
    }

    fn offset(self) -> usize {
        self.0 as usize
    }
}

const HEADER: usize = 3;
const FLAG_LEARNT: u32 = 1;
const FLAG_DELETED: u32 = 2;
const FLAG_RELOCATED: u32 = 4;
const LBD_SHIFT: u32 = 3;
/// Largest storable LBD; a real LBD never gets anywhere near it.
const LBD_MAX: u32 = (1 << (32 - LBD_SHIFT)) - 1;

/// The arena holding every clause of a solver.
pub(super) struct ClauseDb {
    data: Vec<Lit>,
    wasted: usize,
}

impl ClauseDb {
    pub(super) fn new() -> ClauseDb {
        ClauseDb {
            data: Vec::new(),
            wasted: 0,
        }
    }

    fn with_capacity(words: usize) -> ClauseDb {
        ClauseDb {
            data: Vec::with_capacity(words),
            wasted: 0,
        }
    }

    /// Words in use, including deleted clauses.
    pub(super) fn used(&self) -> usize {
        self.data.len()
    }

    /// Words occupied by deleted or shrunk clauses.
    pub(super) fn wasted(&self) -> usize {
        self.wasted
    }

    fn next_offset(&self) -> u32 {
        let offset = u32::try_from(self.data.len()).unwrap_or(u32::MAX);
        assert!(offset < u32::MAX, "clause arena exceeds 2^32 words");
        offset
    }

    /// Appends a clause and returns its reference.
    pub(super) fn alloc(&mut self, lits: &[Lit], learnt: bool, lbd: u32) -> ClauseRef {
        let offset = self.next_offset();
        let len = u32::try_from(lits.len()).expect("clause exceeds 2^32 literals");
        let flags = if learnt { FLAG_LEARNT } else { 0 } | (lbd.min(LBD_MAX) << LBD_SHIFT);
        self.data.push(Lit::from_raw(len));
        self.data.push(Lit::from_raw(flags));
        self.data.push(Lit::from_raw(0f32.to_bits()));
        self.data.extend_from_slice(lits);
        ClauseRef(offset)
    }

    /// Number of literals in the clause.
    pub(super) fn len(&self, cr: ClauseRef) -> usize {
        self.data[cr.offset()].raw() as usize
    }

    /// The literals of the clause.
    pub(super) fn lits(&self, cr: ClauseRef) -> &[Lit] {
        let start = cr.offset() + HEADER;
        &self.data[start..start + self.len(cr)]
    }

    /// The literals of the clause, mutably (used to rotate watches).
    pub(super) fn lits_mut(&mut self, cr: ClauseRef) -> &mut [Lit] {
        let len = self.len(cr);
        let start = cr.offset() + HEADER;
        &mut self.data[start..start + len]
    }

    /// The literal at `i`.
    pub(super) fn lit(&self, cr: ClauseRef, i: usize) -> Lit {
        self.data[cr.offset() + HEADER + i]
    }

    fn flags(&self, cr: ClauseRef) -> u32 {
        self.data[cr.offset() + 1].raw()
    }

    fn set_flags(&mut self, cr: ClauseRef, flags: u32) {
        self.data[cr.offset() + 1] = Lit::from_raw(flags);
    }

    pub(super) fn is_learnt(&self, cr: ClauseRef) -> bool {
        self.flags(cr) & FLAG_LEARNT != 0
    }

    pub(super) fn is_deleted(&self, cr: ClauseRef) -> bool {
        self.flags(cr) & FLAG_DELETED != 0
    }

    /// The literal block distance recorded when the clause was learnt.
    pub(super) fn lbd(&self, cr: ClauseRef) -> u32 {
        self.flags(cr) >> LBD_SHIFT
    }

    pub(super) fn set_lbd(&mut self, cr: ClauseRef, lbd: u32) {
        let flags = self.flags(cr) & ((1 << LBD_SHIFT) - 1);
        self.set_flags(cr, flags | (lbd.min(LBD_MAX) << LBD_SHIFT));
    }

    pub(super) fn activity(&self, cr: ClauseRef) -> f32 {
        f32::from_bits(self.data[cr.offset() + 2].raw())
    }

    pub(super) fn set_activity(&mut self, cr: ClauseRef, activity: f32) {
        self.data[cr.offset() + 2] = Lit::from_raw(activity.to_bits());
    }

    /// Marks the clause deleted; its words are reclaimed at the next
    /// compaction.
    pub(super) fn mark_deleted(&mut self, cr: ClauseRef) {
        debug_assert!(!self.is_deleted(cr));
        self.set_flags(cr, self.flags(cr) | FLAG_DELETED);
        self.wasted += HEADER + self.len(cr);
    }

    /// Shortens the clause in place to its first `new_len` literals.
    pub(super) fn shrink(&mut self, cr: ClauseRef, new_len: usize) {
        let len = self.len(cr);
        debug_assert!(new_len <= len && new_len >= 2);
        self.wasted += len - new_len;
        let raw = u32::try_from(new_len).expect("clause exceeds 2^32 literals");
        self.data[cr.offset()] = Lit::from_raw(raw);
    }

    /// Copies a live clause into `to`, leaving a forwarding address behind,
    /// and returns the new reference. Calling it twice for the same clause
    /// returns the same address.
    fn relocate(&mut self, cr: ClauseRef, to: &mut ClauseDb) -> ClauseRef {
        let flags = self.flags(cr);
        if flags & FLAG_RELOCATED != 0 {
            return self.forwarded(cr);
        }
        debug_assert!(flags & FLAG_DELETED == 0);
        let offset = to.next_offset();
        let len = self.len(cr);
        let start = cr.offset();
        to.data
            .extend_from_slice(&self.data[start..start + HEADER + len]);
        self.set_flags(cr, flags | FLAG_RELOCATED);
        self.data[start + 2] = Lit::from_raw(offset);
        ClauseRef(offset)
    }

    /// The new address of a clause moved by [`ClauseDb::compact`].
    pub(super) fn forwarded(&self, cr: ClauseRef) -> ClauseRef {
        debug_assert!(self.flags(cr) & FLAG_RELOCATED != 0);
        ClauseRef(self.data[cr.offset() + 2].raw())
    }

    /// Compacts every clause listed in `lists` into a fresh arena. Each list
    /// entry is rewritten to the new address; other references must be
    /// translated with [`ClauseDb::forwarded`] before `self` is replaced.
    pub(super) fn compact(&mut self, lists: &mut [&mut Vec<ClauseRef>]) -> ClauseDb {
        let mut to = ClauseDb::with_capacity(self.data.len() - self.wasted);
        for list in lists.iter_mut() {
            for cr in list.iter_mut() {
                *cr = self.relocate(*cr, &mut to);
            }
        }
        to
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::formal::sat::Var;

    fn l(i: u32) -> Lit {
        Lit::pos(Var::new(i))
    }

    #[test]
    fn alloc_and_read_back() {
        let mut db = ClauseDb::new();
        let a = db.alloc(&[l(0), l(1), l(2)], false, 0);
        let b = db.alloc(&[l(3), l(4)], true, 7);
        assert_eq!(db.lits(a), &[l(0), l(1), l(2)]);
        assert_eq!(db.lits(b), &[l(3), l(4)]);
        assert_eq!(db.lit(b, 1), l(4));
        assert!(!db.is_learnt(a));
        assert!(db.is_learnt(b));
        assert_eq!(db.lbd(b), 7);
        db.set_lbd(b, 3);
        assert_eq!(db.lbd(b), 3);
        assert!(db.is_learnt(b));
        db.set_activity(b, 2.5);
        assert_eq!(db.activity(b), 2.5);
        assert_eq!(db.wasted(), 0);
    }

    #[test]
    fn delete_shrink_and_compact() {
        let mut db = ClauseDb::new();
        let a = db.alloc(&[l(0), l(1), l(2), l(3)], false, 0);
        let b = db.alloc(&[l(3), l(4)], true, 2);
        let c = db.alloc(&[l(5), l(6), l(7)], false, 0);
        db.mark_deleted(b);
        assert!(db.is_deleted(b));
        db.shrink(a, 3);
        assert_eq!(db.lits(a), &[l(0), l(1), l(2)]);
        assert_eq!(db.wasted(), HEADER + 2 + 1);

        let mut clauses = vec![a, c];
        let mut learnts: Vec<ClauseRef> = Vec::new();
        let to = db.compact(&mut [&mut clauses, &mut learnts]);
        assert_eq!(db.forwarded(a), clauses[0]);
        assert_eq!(db.forwarded(c), clauses[1]);
        assert_eq!(to.lits(clauses[0]), &[l(0), l(1), l(2)]);
        assert_eq!(to.lits(clauses[1]), &[l(5), l(6), l(7)]);
        assert_eq!(to.used(), 2 * HEADER + 6);
        assert_eq!(to.wasted(), 0);
    }
}
