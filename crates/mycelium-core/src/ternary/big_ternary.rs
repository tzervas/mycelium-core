//! Arbitrary-width balanced-ternary **integer** arithmetic (E20-1 / M-756; RFC-0033 §4.2; ADR-029).
//!
//! # Why this file exists
//! The fixed-width path in [`super`] (M-111) is `i64`-internal and **already never-silent** about its
//! ~40-trit cap ([`super::max_magnitude`] returns `None` at `m ≥ 41`; [`super::add`]/[`super::mul`]
//! return `None` on overflow). This module **removes the cap** by adding a growable representation that
//! *grows a new trit* instead of returning `None` — the bignum need the fixed-width comment
//! anticipated. It is **not** a bug-fix in Mycelium's code: the silent-overflow defect that motivates
//! an arbitrary-width path is `embeddonator`'s `dimensional::Tryte::max_value` (a different upstream
//! codebase, on the do-not-lift list), never `core::ternary`.
//!
//! # Design (KC-3: TRUSTED)
//! [`BigTernary`] is a digit-serial `Vec<Trit>` (least-significant-first, canonicalized) — the
//! obviously-correct, never-overflowing reference. A limbed/packed perf path (`PackedTernary`,
//! ≥40 trits/u64) is an explicit YAGNI follow-on (M-758) gated on a benchmark and, if added, MUST be
//! differentially proven bit-exact against this reference (RFC-0033 §4.2.2).
//!
//! # Never-silent boundary (G2)
//! [`BigTernary`] arithmetic NEVER overflows (the carry out of the top digit becomes a new digit). The
//! boundary is the **fixed-width** [`FixedWidthTrits`] (the in-memory image of `Repr::Ternary{N}`):
//! [`BigTernary::checked_to_width`] and [`checked_add_fixed`] return `Option` and yield `None` exactly
//! when the true result needs more than `N` trits — never a wrap or truncation. [`BigTernary::to_i128`]
//! is likewise `Option` (overflow-checked).
//!
//! # Guarantee lattice
//! Every operation here is **Exact** (closed integer arithmetic; the balanced-ternary digit algebra is
//! an exact integer identity, Knuth 4.1 / `docs/spec/swaps/binary-ternary.md` §1). The binary↔ternary
//! swap is `LosslessWithinRange` — lossless for the growable path, range-bounded for fixed width
//! (RFC-0033 §6.1).
//!
//! # Endianness
//! [`BigTernary`] is **least-significant-first** (index 0 least significant); the fixed-width [`super`]
//! codec/arithmetic is **most-significant-first**. The two are reconciled **only through the integer
//! value** ([`BigTernary::to_i128`] / [`super::trits_to_int`]), never by comparing trit vectors.

use super::{add_with_carry, digit, is_nonzero, is_zero, neg_trit};
use crate::value::Trit;

/// Arbitrary-width balanced-ternary integer (digit-serial reference form).
///
/// Invariant (canonical form): `digits` has no trailing (most-significant) `Zero` trits, EXCEPT that
/// zero is the empty vector. Enforced by [`BigTernary::canonicalize`] after every constructor/op, so
/// each integer has exactly **one** representation (non-redundant ⇒ content-addressing is well-defined;
/// RFC-0033 §4.2.4).
#[derive(Clone, PartialEq, Eq, Default, Debug)]
pub struct BigTernary {
    /// Balanced trits, index 0 least significant. Canonical: no trailing `Zero`.
    digits: Vec<Trit>,
}

impl BigTernary {
    /// The additive identity (empty digit vector).
    #[inline]
    #[must_use]
    pub fn zero() -> Self {
        BigTernary { digits: Vec::new() }
    }

    /// `true` iff this is exactly zero.
    #[inline]
    #[must_use]
    pub fn is_zero(&self) -> bool {
        self.digits.is_empty()
    }

    /// Number of significant trits (0 for zero).
    #[inline]
    #[must_use]
    pub fn width(&self) -> usize {
        self.digits.len()
    }

    /// Borrow the canonical digit slice (least-significant-first).
    #[inline]
    #[must_use]
    pub fn digits(&self) -> &[Trit] {
        &self.digits
    }

    /// Build from raw least-significant-first trits (any non-canonical input is accepted and
    /// canonicalized). Total — there is no invalid `Trit`, so this never fails.
    pub fn from_trits_lsf(trits: impl IntoIterator<Item = Trit>) -> Self {
        let mut b = BigTernary {
            digits: trits.into_iter().collect(),
        };
        b.canonicalize();
        b
    }

    /// Drop trailing (most-significant) `Zero` trits; zero becomes empty.
    fn canonicalize(&mut self) {
        while matches!(self.digits.last(), Some(Trit::Zero)) {
            self.digits.pop();
        }
    }

    /// Negate (flip every trit). Canonical form is preserved.
    #[must_use]
    pub fn neg(&self) -> Self {
        BigTernary {
            digits: self.digits.iter().map(|&t| neg_trit(t)).collect(),
        }
    }

    /// Addition. NEVER overflows — the final carry becomes a new digit. Digit-serial ripple of the
    /// shared [`super::add_with_carry`]; `O(max(width))`.
    #[must_use]
    pub fn add(&self, other: &Self) -> Self {
        let n = self.digits.len().max(other.digits.len());
        let mut out = Vec::with_capacity(n + 1);
        let mut carry = Trit::Zero;
        for i in 0..n {
            let a = self.digits.get(i).copied().unwrap_or(Trit::Zero);
            let b = other.digits.get(i).copied().unwrap_or(Trit::Zero);
            let (sum, c) = add_with_carry(a, b, carry);
            out.push(sum);
            carry = c;
        }
        if is_nonzero(carry) {
            out.push(carry);
        }
        let mut r = BigTernary { digits: out };
        r.canonicalize();
        r
    }

    /// Subtraction: `self + (−other)`.
    #[must_use]
    pub fn sub(&self, other: &Self) -> Self {
        self.add(&other.neg())
    }

    /// Multiplication (schoolbook over balanced trits): each `b_i ∈ {−1, 0, +1}`, so the partial
    /// product is `±self` shifted left by `i`, accumulated with [`add`](Self::add).
    /// `O(width(self) · width(other))`. A Karatsuba/Toom fast path is a YAGNI follow-on (M-759),
    /// equivalence-tested against this if added.
    #[must_use]
    pub fn mul(&self, other: &Self) -> Self {
        let mut acc = BigTernary::zero();
        for (i, &b) in other.digits.iter().enumerate() {
            if is_zero(b) {
                continue;
            }
            // partial = (±self) << i  (i leading zero trits, then the signed digits)
            let mut shifted = vec![Trit::Zero; i];
            let signed = if matches!(b, Trit::Neg) {
                self.neg()
            } else {
                self.clone()
            };
            shifted.extend_from_slice(&signed.digits);
            acc = acc.add(&BigTernary { digits: shifted });
        }
        acc.canonicalize();
        acc
    }

    // ---- bridges to/from machine integers (never-silent) ----

    /// Exact construction from `i128`.
    #[must_use]
    pub fn from_i128(mut value: i128) -> Self {
        let mut digits = Vec::new();
        while value != 0 {
            // Balanced residue in {−1, 0, +1}: `rem_euclid` gives 0,1,2; map 2 → −1. `value - rem` is
            // divisible by 3 (rem ≡ value mod 3), so the quotient is exact and applies the borrow.
            let m = value.rem_euclid(3);
            let rem: i128 = if m == 2 { -1 } else { m };
            value = (value - rem) / 3;
            digits.push(match rem {
                -1 => Trit::Neg,
                0 => Trit::Zero,
                1 => Trit::Pos,
                _ => unreachable!("balanced residue is in {{-1, 0, 1}}"),
            });
        }
        let mut b = BigTernary { digits };
        b.canonicalize();
        b
    }

    /// NEVER-SILENT conversion to `i128`: `None` if the value does not fit (overflow-checked Horner).
    #[must_use]
    pub fn to_i128(&self) -> Option<i128> {
        let mut acc: i128 = 0;
        let mut pow: i128 = 1;
        for (i, &t) in self.digits.iter().enumerate() {
            let term = i128::from(digit(t)).checked_mul(pow)?;
            acc = acc.checked_add(term)?;
            if i + 1 < self.digits.len() {
                pow = pow.checked_mul(3)?;
            }
        }
        Some(acc)
    }

    // ---- the fixed-width / never-silent boundary ----

    /// NEVER-SILENT narrowing to a fixed width of `n` trits: `Some` iff `width() ≤ n`; `None`
    /// otherwise. The single honest definition of "out of range" for `Repr::Ternary{trits:n}`.
    #[must_use]
    pub fn checked_to_width(&self, n: u32) -> Option<FixedWidthTrits> {
        if self.width() > n as usize {
            return None;
        }
        let mut digits = self.digits.clone();
        digits.resize(n as usize, Trit::Zero);
        Some(FixedWidthTrits { trits: digits })
    }
}

/// A balanced-ternary value pinned to exactly `trits.len()` trits — the in-memory image of
/// `Repr::Ternary{trits:N}`. Padding trits are `Zero`. Arithmetic that could overflow the width is
/// never-silent (see [`checked_add_fixed`]).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct FixedWidthTrits {
    /// Exactly `N` trits, least-significant-first, `Zero`-padded.
    pub trits: Vec<Trit>,
}

impl FixedWidthTrits {
    /// Promote to the growable form (always exact).
    #[must_use]
    pub fn to_big(&self) -> BigTernary {
        BigTernary::from_trits_lsf(self.trits.iter().copied())
    }
}

/// NEVER-SILENT fixed-width addition: ripples the shared [`super::add_with_carry`] across `n` trits and
/// returns `None` iff the carry out of the top trit is non-zero (the true sum needs trit `n+1`). No
/// wrap, no truncation. Both inputs MUST be the same width (debug-asserted; a mismatch is a caller bug,
/// not a runtime value condition).
#[must_use]
pub fn checked_add_fixed(a: &FixedWidthTrits, b: &FixedWidthTrits) -> Option<FixedWidthTrits> {
    debug_assert_eq!(
        a.trits.len(),
        b.trits.len(),
        "width mismatch is a caller bug"
    );
    let n = a.trits.len();
    let mut out = Vec::with_capacity(n);
    let mut carry = Trit::Zero;
    for i in 0..n {
        let (sum, c) = add_with_carry(a.trits[i], b.trits[i], carry);
        out.push(sum);
        carry = c;
    }
    if is_nonzero(carry) {
        None // overflow — explicit, never silent
    } else {
        Some(FixedWidthTrits { trits: out })
    }
}
