//! An adder after a multiply, taken into the `DSP48E1`'s own.
//!
//! A block adds after it multiplies, so a multiply feeding an adder needs no
//! carry chain: `p <= a * b + c` is one block with `C` on `Z`, an accumulator is
//! one block with `P` on `Z`, and a sum of products is one block each, every one
//! adding the one below over `PCOUT`.
//!
//! # Where this is from
//!
//! **Measured** (`testdata/fpga/vivado/dsp/inference.txt`): `f_add` is one block
//! with `OPMODE` `0110101`, `P` on the sum and `C` on the addend's register;
//! `f_macc` one with `OPMODE` `0100101`. `fir8`, a sum of eight products, is eight
//! blocks with `OPMODE` `0010101` joined by `PCOUT`, the lowest with `OPMODE`
//! `0000101` and `P` where the others have `M`, the highest with `P` for the
//! sum's register, and no carry chain at all.
//!
//! **Not Vivado's**: it also shares the filter's shift register through `ACOUT` and
//! `BCOUT`, which leaves `fir8` with 23 flip-flops; here that register stays in the
//! fabric. A subtraction, which `ALUMODE` could do, stays there too.
//!
//! # When an adder may go
//!
//! Only the low `width` bits of the sum are read, so the block's 48 bits are right
//! modulo them as long as the term it adds is too: a term is the product resized,
//! which is the product's low bits outright when it is cut down, and when it is
//! extended only if the product fits its net and the extension is the one the
//! block's sign makes.

use std::collections::HashSet;

use crate::ir::{CellId, CellKind, ExprId, ExprKind, Module, NetId};

/// One multiply a sum reads, on its way to a block.
#[derive(Clone, Copy, Debug)]
pub(super) struct Term {
    /// The `Mul` cell.
    pub mul: CellId,
}

/// What a sum adds that is not a product.
#[derive(Clone, Copy, Debug)]
pub(super) enum Addend {
    /// Nothing.
    None,
    /// A value, on `C`.
    Value(ExprId),
    /// What the sum's own register holds: `P`.
    Own,
}

/// A sum of products a chain of blocks can take.
#[derive(Clone, Debug)]
pub(super) struct Fold {
    /// The products, lowest block first.
    pub terms: Vec<Term>,
    /// The `Add` cells, which go.
    pub adds: Vec<CellId>,
    /// What the last of them drives.
    pub root: NetId,
    /// How many bits of the sum are read.
    pub width: u32,
    /// What else it adds, on the lowest block.
    pub addend: Addend,
    /// The clock the products' flip-flops are on, if they have any.
    pub clock: Option<ExprId>,
}

/// The net an operand is read from, and how it was resized on the way.
struct Read {
    net: NetId,
    /// `Some(signed)` when a resize widened it: by sign or by zeros.
    widened: Option<bool>,
}

fn read_of(module: &Module, e: ExprId) -> Option<Read> {
    match &module.exprs.get(e)?.kind {
        ExprKind::Net(net) => Some(Read {
            net: *net,
            widened: None,
        }),
        ExprKind::Resize {
            expr: inner,
            width,
            signed,
        } => {
            let ExprKind::Net(net) = module.exprs.get(*inner)?.kind else {
                return None;
            };
            let from = module.nets.get(net)?.ty.width()?;
            let sign = *signed && module.nets.get(net)?.ty.is_signed();
            Some(Read {
                net,
                widened: (*width > from).then_some(sign),
            })
        }
        _ => None,
    }
}

/// The multiply a net is, directly or through one flip-flop only it feeds, with
/// that flip-flop's clock. The flip-flop has to be one the block can hold: the
/// block's `P` will be the sum, so a product left for a flip-flop in the fabric
/// has nowhere to come from.
fn product_behind(
    module: &Module,
    uses: &[usize],
    driver: &std::collections::HashMap<NetId, CellId>,
    net: NetId,
) -> Option<(CellId, Option<ExprId>)> {
    if uses.get(net.index()).copied().unwrap_or(0) != 1 {
        return None;
    }
    let cell = *driver.get(&net)?;
    match module.cells.get(cell)?.kind {
        CellKind::Mul => Some((cell, None)),
        CellKind::Dff { .. } => {
            let control = super::dsp_absorb::control_of(module, cell)?;
            let d = module.cells.get(cell)?.input("d")?;
            let read = read_of(module, d)?;
            // A flip-flop that cuts the product down is the block's too, so
            // long as the net it reads is only the multiply.
            if read.widened.is_some() {
                return None;
            }
            if uses.get(read.net.index()).copied().unwrap_or(0) != 1 {
                return None;
            }
            let behind = *driver.get(&read.net)?;
            matches!(module.cells.get(behind)?.kind, CellKind::Mul)
                .then_some((behind, Some(control.clk)))
        }
        _ => None,
    }
}

/// A multiply's exact width and whether it can be negative, or `None` when it does
/// not go on one block.
pub(super) type ProductIs<'a> = &'a dyn Fn(&Module, CellId) -> Option<(u32, bool)>;

/// True when `read` of a net holding a multiply's result, into a sum of `width`
/// bits, is the product's low bits.
fn extension_fits(
    module: &Module,
    read: &Read,
    width: u32,
    product_is: ProductIs<'_>,
    mul: CellId,
) -> bool {
    let Some(held) = module.nets.get(read.net).and_then(|n| n.ty.width()) else {
        return false;
    };
    if width <= held {
        return true;
    }
    let Some(sign) = read.widened else {
        return false;
    };
    let Some((bits, signed)) = product_is(module, mul) else {
        return false;
    };
    // The net holds the exact product, and its extension copies a sign that
    // the block's own would: a signed one sign extends, and one of two operands
    // read unsigned is never negative, so zeros are its sign too.
    bits <= held && (sign || !signed)
}

/// Finds the sums the device's block can take.
///
/// A multiply `product_is` says does not go on one block is one no sum holding it
/// can take.
pub(super) fn find(module: &Module, uses: &[usize], product_is: ProductIs<'_>) -> Vec<Fold> {
    let mut driver = std::collections::HashMap::new();
    for (id, cell) in module.cells.iter() {
        for (_, net) in &cell.outputs {
            driver.insert(*net, id);
        }
    }
    let width_of = |net: NetId| module.nets.get(net).and_then(|n| n.ty.width());
    // An `Add` another `Add` reads is not a root.
    let mut inner: HashSet<CellId> = HashSet::new();
    for (_, cell) in module.cells.iter() {
        if cell.kind != CellKind::Add {
            continue;
        }
        for port in ["a", "b"] {
            if let Some(read) = cell.input(port).and_then(|e| read_of(module, e))
                && let Some(from) = driver.get(&read.net)
                && module
                    .cells
                    .get(*from)
                    .is_some_and(|c| c.kind == CellKind::Add)
                && read.widened.is_none()
                && uses.get(read.net.index()).copied().unwrap_or(0) == 1
            {
                inner.insert(*from);
            }
        }
    }
    let mut out = Vec::new();
    let mut taken: HashSet<CellId> = HashSet::new();
    for (id, cell) in module.cells.iter() {
        if cell.kind != CellKind::Add || inner.contains(&id) {
            continue;
        }
        let Some(root) = cell.output("y") else {
            continue;
        };
        let Some(width) = width_of(root) else {
            continue;
        };
        if width > 48 {
            continue;
        }
        let mut terms: Vec<Term> = Vec::new();
        let mut adds: Vec<CellId> = Vec::new();
        let mut addend = Addend::None;
        let mut extras = 0;
        let mut at = id;
        let mut fits = true;
        let mut clock: Option<ExprId> = None;
        'walk: loop {
            adds.push(at);
            let add = &module.cells[at];
            let (Some(a), Some(b)) = (add.input("a"), add.input("b")) else {
                fits = false;
                break;
            };
            let mut next: Option<CellId> = None;
            for operand in [a, b] {
                let read = read_of(module, operand);
                if let Some(read) = &read {
                    if inner.contains(driver.get(&read.net).unwrap_or(&id))
                        && read.widened.is_none()
                    {
                        if next.is_some() {
                            fits = false;
                            break 'walk;
                        }
                        next = driver.get(&read.net).copied();
                        continue;
                    }
                    if let Some((mul, stage_clock)) =
                        product_behind(module, uses, &driver, read.net)
                        && !taken.contains(&mul)
                        && extension_fits(module, read, width, product_is, mul)
                    {
                        // One clock for the whole chain, since each block has
                        // one: a sum of two clocks' products stays in the fabric.
                        if let (Some(known), Some(here)) = (clock, stage_clock)
                            && !super::dsp_absorb::same(module, known, here)
                        {
                            fits = false;
                            break 'walk;
                        }
                        clock = clock.or(stage_clock);
                        terms.push(Term { mul });
                        continue;
                    }
                }
                // Not a product: what the lowest block adds. Its own register
                // when it reads back what the sum's flip-flop holds, and a value
                // otherwise.
                extras += 1;
                let own = read.as_ref().is_some_and(|read| {
                    let Some(q) = driver.get(&read.net) else {
                        return false;
                    };
                    let Some(flop) = module.cells.get(*q) else {
                        return false;
                    };
                    matches!(flop.kind, CellKind::Dff { .. })
                        && flop
                            .input("d")
                            .and_then(|d| read_of(module, d))
                            .is_some_and(|d| d.net == root)
                        && read.widened.is_none()
                });
                addend = if own {
                    Addend::Own
                } else {
                    Addend::Value(operand)
                };
            }
            match next {
                Some(cell) => at = cell,
                None => break,
            }
        }
        if !fits || terms.is_empty() || extras > 1 {
            continue;
        }
        // A sum with its own register in it has only the one block for `P`.
        if matches!(addend, Addend::Own) && terms.len() != 1 {
            continue;
        }
        // Lowest block first: the products nearest the root were found first.
        terms.reverse();
        for term in &terms {
            taken.insert(term.mul);
        }
        out.push(Fold {
            terms,
            adds,
            root,
            width,
            addend,
            clock,
        });
    }
    out
}
