//! Balanced-ternary integer semantics and arithmetic (M-111; FR-M2).
//!
//! A [`Trit`] is a digit in `{−1, 0, +1}`. An `m`-trit balanced-ternary number with digits written
//! **most-significant-first** `⟨t₀ … t_{m-1}⟩` denotes the integer
//! `value(t) = Σⱼ digit(tⱼ)·3^(m-1-j)` (`docs/spec/swaps/binary-ternary.md` §1). This module is the
//! single home for the codec (`int ↔ trits`) and the digit-wise arithmetic; it is reused by the
//! reference interpreter's `trit.*` primitives (M-111) and by the binary↔ternary swap (M-120).
//!
//! Two identities the spec calls out (§1) hold by construction here and are oracle-tested:
//! **negation = digit-wise sign flip** ([`neg`]) and the symmetric range `[−(3^m−1)/2, (3^m−1)/2]`
//! ([`max_magnitude`]). Arithmetic is **fixed-width**: a result outside the range is an explicit
//! `None`/overflow — never a silent wrap (SC-3; G2).
//!
//! **Correction (CU-7 recon, 2026-07-08 — mitigation #14, verify against the codebase before
//! implementing): [`add`]/[`sub`]/[`mul`]/[`neg`] are NOT machine-integer-capped.** They are
//! digit-serial (ripple-carry add, shifted-accumulation multiply) directly over `&[Trit]`, with no
//! machine integer anywhere in the algorithm; overflow is detected **structurally** (a nonzero
//! final carry / nonzero high digits), so they are correct and never-silent at **any** width `m`.
//! The machine-integer cap belongs to the separate *conversion* utilities below —
//! [`max_magnitude`] (whose own `3^m` computation needs integer room) and
//! [`int_to_trits`]/[`trits_to_int`] (which round-trip a **value**, not a width) — used for
//! decimal-literal encoding and oracle tests, never by `add`/`sub`/`mul`/`neg` themselves. (A prior
//! revision of this comment conflated the two; corrected per VR-5 — see
//! `crates/mycelium-core/src/tests/ternary.rs` for the width-60/200 witness tests and
//! `mycelium-l1/tests/enablement.rs`'s width-80 three-way for the end-to-end confirmation.) The
//! **arbitrary-width** path that removes the *conversion* utilities' ceiling entirely (growing a
//! digit instead of ever needing a machine-integer-sized magnitude) lives in `big_ternary`
//! ([`BigTernary`]) — the bignum need the original cap anticipated (E20-1/M-756; RFC-0033 §4.2;
//! ADR-029). The shared balanced full-adder [`add_with_carry`] is the single never-silent digit
//! primitive both the fixed-width [`add`] and the growable [`BigTernary`] ripple (DRY).
//!
//! **E-W1 widening (M-1119, 2026-07-18 — W-1 §A.5 enablement item; mitigation #14 verify-first
//! finding).** The conversion utilities were originally documented as `i64`-capped at `m ≤ 40`, but
//! that figure was itself inaccurate: `max_magnitude`'s naive `3^m` computation (`pow =
//! pow.checked_mul(3)?` for `m` iterations) actually overflowed `i64` **one trit earlier**, at
//! `m = 40` (`3^40 ≈ 1.2158e19 > i64::MAX ≈ 9.223e18`, even though the *quotient* `(3^40−1)/2` would
//! itself have fit) — so the real pre-widening ceiling was `m ≤ 39`, matching
//! `mycelium-mlir::swap_codegen::MAX_TERNARY_WIDTH_I64 = 39`'s independently-documented figure, not
//! the `m ≤ 40` this module's own comment (and `mycelium-std-ternary`'s wrapper) had claimed. A
//! second, more serious gap: even where `int_to_trits`'s pure divide/mod loop never overflows at any
//! width, the OLD `i64` [`trits_to_int`] Horner accumulation (`acc·3 + digit`) could — for a
//! genuinely 41-trit-significant value (e.g. `i64::MIN` encoded at `m = 41`) — transiently exceed
//! `i64::MAX` mid-fold even though the final decoded value fits `i64`, which is exactly the kind of
//! decode-side overflow `mycelium-cert::ternary_to_binary` would have hit unconditionally once a
//! `Ternary{41}` value entered the system (a live G2 gap, not hypothetical — confirmed by direct
//! computation before this fix). All three conversion utilities now route through **`i128`**
//! (per the issue's sanctioned choice — full arbitrary-width stays [`BigTernary`]'s job, M-758
//! `PackedTernary` stays YAGNI): [`max_magnitude`]'s `3^m` fits `i128` through `m = 80`
//! (`3^80 ≈ 1.478e38 < i128::MAX ≈ 1.7014e38`; `3^81` overflows); [`int_to_trits`]'s divide/mod loop
//! never overflows at any width (unchanged shape, just wider); [`trits_to_int`]'s Horner fold stays
//! **infallible** (not `Option`, matching its existing contract) and is safe for every width whose
//! `max_magnitude` itself fits `i128` (`m ≲ 80`) — a caller decoding a wider, larger-magnitude trit
//! string still carries the same *kind* of undocumented-boundary risk this module always had, just
//! moved from `~40` to `~80` trits; [`BigTernary::to_i128`] remains the fully checked (`Option`)
//! alternative when that residual matters. This directly unblocks the W-1 canonical
//! `Binary{64} ↔ Ternary{41}` pair (`docs/spec/swaps/binary-ternary.md` §A.3/§A.5): `max_magnitude(41)`
//! now returns `Some`, and `mycelium-cert::legal_pair`/`mycelium-mlir::swap_codegen::legal_pair` (both
//! already `i128`-typed at their own call sites) pick the fix up directly.

mod big_ternary;
pub use big_ternary::{checked_add_fixed, BigTernary, FixedWidthTrits};

use crate::value::Trit;

/// The signed value of a single trit.
#[must_use]
pub fn digit(t: Trit) -> i64 {
    match t {
        Trit::Neg => -1,
        Trit::Zero => 0,
        Trit::Pos => 1,
    }
}

fn from_digit(d: i64) -> Trit {
    // C1-05: every caller normalizes into the balanced-ternary digit domain `{−1, 0, +1}` before
    // reaching here — `int_to_trits` folds the `r == 2` carry to `−1`, and `add`'s `(s+1).rem_euclid(3) − 1`
    // is provably in `[−1, +1]`. So `_ => Zero` is never taken on a well-formed call; the
    // `debug_assert!` documents and (in debug builds) checks that domain invariant without a
    // release-build panic in the trusted kernel. A stray out-of-domain digit maps to `Zero`
    // (the additive identity) rather than wrapping silently — still sound, never undefined.
    match d {
        -1 => Trit::Neg,
        1 => Trit::Pos,
        0 => Trit::Zero,
        _ => {
            debug_assert!(false, "balanced-ternary digit out of range: {d}");
            Trit::Zero
        }
    }
}

/// Balanced full-adder over single trits: returns `(digit_out, carry_out)` with the exact invariant
/// `digit(a) + digit(b) + digit(carry_in) == digit(digit_out) + 3·digit(carry_out)`. The sum
/// `s = digit(a)+digit(b)+digit(carry_in) ∈ [−3, 3]`, and `(s+1).rem_euclid(3)−1` / `(s+1).div_euclid(3)`
/// are provably balanced trits, so both outputs are in `{−1, 0, +1}`. This is the **single**
/// never-silent digit primitive both the fixed-width [`add`] and the growable [`BigTernary`] ripple
/// (DRY); it is exhaustively oracle-tested over all 27 inputs (`add_with_carry_is_exhaustively_correct`).
/// Guarantee: **Exact** (C2).
#[must_use]
pub(crate) fn add_with_carry(a: Trit, b: Trit, carry_in: Trit) -> (Trit, Trit) {
    let s = digit(a) + digit(b) + digit(carry_in);
    let d = (s + 1).rem_euclid(3) - 1;
    let c = (s + 1).div_euclid(3);
    (from_digit(d), from_digit(c))
}

/// Per-trit negation (sign flip): `value(neg_trit t) = −value(t)` exactly. Total; always in range
/// (balanced ternary is sign-symmetric, §1).
#[must_use]
pub(crate) fn neg_trit(t: Trit) -> Trit {
    match t {
        Trit::Neg => Trit::Pos,
        Trit::Zero => Trit::Zero,
        Trit::Pos => Trit::Neg,
    }
}

/// `true` iff the trit is the additive identity `Zero`.
#[inline]
#[must_use]
pub(crate) fn is_zero(t: Trit) -> bool {
    matches!(t, Trit::Zero)
}

/// `true` iff the trit is non-zero (`Neg` or `Pos`).
#[inline]
#[must_use]
pub(crate) fn is_nonzero(t: Trit) -> bool {
    !is_zero(t)
}

/// The maximum representable magnitude in `m` trits: `(3^m − 1) / 2`. The range is the symmetric
/// `[−max, +max]`. Returns `None` if `3^m` would overflow `i128` (`m ≥ 81`; E-W1/M-1119 widened
/// this from the prior real ceiling of `m ≥ 40` — see the module doc comment).
#[must_use]
pub fn max_magnitude(m: u32) -> Option<i128> {
    let mut pow: i128 = 1;
    for _ in 0..m {
        pow = pow.checked_mul(3)?;
    }
    Some((pow - 1) / 2)
}

/// The integer denoted by an MSB-first trit string (`value(t)`, §1). The empty string is `0`.
///
/// Infallible (matches its pre-widening contract): safe for every width whose [`max_magnitude`]
/// itself fits `i128` (`m ≲ 80`, E-W1/M-1119); [`BigTernary::to_i128`] is the fully checked
/// (`Option`) alternative beyond that.
#[must_use]
pub fn trits_to_int(trits: &[Trit]) -> i128 {
    // Horner from the most-significant digit: v = v·3 + dⱼ.
    trits
        .iter()
        .fold(0i128, |acc, &t| acc * 3 + i128::from(digit(t)))
}

/// The unique `m`-trit balanced representation of `value`, MSB-first — or `None` if `value` lies
/// outside the `m`-trit range (an explicit out-of-range result, never a silent truncation; §3.1).
///
/// The divide/mod loop below never overflows at any width `m` (E-W1/M-1119 module doc comment) —
/// only the *conversion* (not this codec) was ever machine-integer-limited.
#[must_use]
pub fn int_to_trits(value: i128, m: u32) -> Option<Vec<Trit>> {
    let mut v = value;
    let mut lsb_first = Vec::with_capacity(m as usize);
    for _ in 0..m {
        // Balanced remainder in {−1, 0, +1}: take r ∈ {0,1,2} then fold 2 ≡ −1 (carry up).
        let mut r = v.rem_euclid(3);
        v = v.div_euclid(3);
        if r == 2 {
            r = -1;
            v += 1; // borrow: 2 ≡ −1 (mod 3)
        }
        // `r` is folded into {−1, 0, +1} above — a lossless narrowing to `from_digit`'s `i64`
        // digit domain (never a truncation of the actual `i128` value, only of this bounded
        // per-digit residual).
        lsb_first.push(from_digit(r as i64));
    }
    if v != 0 {
        return None; // value did not fit in m trits — out of range
    }
    lsb_first.reverse(); // to MSB-first
    Some(lsb_first)
}

/// Digit-wise negation: `value(neg t) = −value(t)` exactly (balanced ternary is sign-symmetric, §1).
/// Width-preserving and always in range.
#[must_use]
pub fn neg(trits: &[Trit]) -> Vec<Trit> {
    trits
        .iter()
        .map(|&t| match t {
            Trit::Neg => Trit::Pos,
            Trit::Zero => Trit::Zero,
            Trit::Pos => Trit::Neg,
        })
        .collect()
}

/// Ripple-carry add over two equal-length MSB-first trit strings, fixed-width. Returns `None` on
/// overflow (a non-zero final carry), i.e. when the true sum leaves the `m`-trit range — explicit,
/// never a silent wrap.
#[must_use]
pub fn add(a: &[Trit], b: &[Trit]) -> Option<Vec<Trit>> {
    if a.len() != b.len() {
        return None;
    }
    let m = a.len();
    let mut out = vec![Trit::Zero; m];
    let mut carry = Trit::Zero;
    // Process least-significant first (the tail of an MSB-first string), rippling the shared
    // balanced full-adder. The carry stays a balanced trit throughout (always in {−1, 0, +1}).
    for i in (0..m).rev() {
        let (d, c) = add_with_carry(a[i], b[i], carry);
        out[i] = d;
        carry = c;
    }
    if carry != Trit::Zero {
        return None; // non-zero final carry ⇒ out of m-trit range (explicit, never silent)
    }
    Some(out)
}

/// Fixed-width subtraction `a − b` = `add(a, neg(b))`.
#[must_use]
pub fn sub(a: &[Trit], b: &[Trit]) -> Option<Vec<Trit>> {
    if a.len() != b.len() {
        return None;
    }
    add(a, &neg(b))
}

/// Fixed-width multiplication. Computes the full product by shifted accumulation (independent of
/// machine integer multiply) in a `2m`-trit buffer, then returns the low `m` trits iff the high
/// trits are all zero — otherwise `None` (overflow, explicit).
#[must_use]
pub fn mul(a: &[Trit], b: &[Trit]) -> Option<Vec<Trit>> {
    if a.len() != b.len() {
        return None;
    }
    let m = a.len();
    if m == 0 {
        return Some(Vec::new());
    }
    let wide = 2 * m;
    let mut acc = vec![Trit::Zero; wide];
    // For each digit of b (power k, counting from the LSB), add ±(a << k) into the accumulator.
    for (k, &bk) in b.iter().rev().enumerate() {
        let factor = digit(bk);
        if factor == 0 {
            continue;
        }
        // a, possibly negated, placed at positions [k, k+m) of an LSB-first buffer.
        let a_signed: Vec<Trit> = if factor < 0 { neg(a) } else { a.to_vec() };
        let mut partial_lsb = vec![Trit::Zero; wide];
        for (j, &t) in a_signed.iter().rev().enumerate() {
            partial_lsb[k + j] = t;
        }
        // Add partial (LSB-first) into acc (LSB-first) — reuse the MSB-first adder via reversal.
        let mut acc_msb: Vec<Trit> = acc.iter().rev().copied().collect();
        let partial_msb: Vec<Trit> = partial_lsb.iter().rev().copied().collect();
        // The 2m-wide sum cannot overflow 2m trits for m-trit operands, so add() is total here.
        acc_msb = add(&acc_msb, &partial_msb)?;
        acc = acc_msb.iter().rev().copied().collect();
    }
    // acc is LSB-first, width 2m. The product fits in m trits iff positions [m, 2m) are all zero.
    if acc[m..].iter().any(|&t| t != Trit::Zero) {
        return None; // overflow
    }
    let low_msb: Vec<Trit> = acc[..m].iter().rev().copied().collect();
    Some(low_msb)
}
