//! White-box tests for [`crate::ternary`] — the core codec/arithmetic (extracted from
//! `ternary/mod.rs`'s and `ternary/big_ternary.rs`'s former inline `#[cfg(test)]` modules per the
//! house test-layout rule, as-touched by E-W1/M-1119), the [`BigTernary`] arbitrary-width bridge,
//! and the tests beyond the ~40-trit figure the *conversion* utilities
//! (`max_magnitude`/`trits_to_int`/`int_to_trits`) originally declared — CU-7 recon (mitigation
//! #14: verify against the codebase before implementing).
//!
//! **Finding (original CU-7 recon).** The trx2 kickoff notes described the runnable fixed-width
//! `trit.add`/`trit.sub`/`trit.mul`/`trit.neg` prims as capped at "~40 trits", attributed to
//! `mycelium_core::ternary` being "`i64`-internal". Reading [`crate::ternary::add`]/
//! [`crate::ternary::mul`] shows this is **not accurate for the arithmetic itself**: both are
//! digit-serial (ripple-carry add, shifted-accumulation multiply) over `&[Trit]`, with no machine
//! integer anywhere in the algorithm — overflow is detected *structurally* (a nonzero final carry /
//! nonzero high digits), never via an integer-range check. The **only** capped pieces are the
//! *conversion* utilities (`max_magnitude`'s `3^m` must fit the accumulator type;
//! `int_to_trits`/`trits_to_int` round-trip a **value**, not a width) — used for decimal-literal
//! encoding and this file's own test oracle, a genuinely different concern from RFC-0033 §4.2.2's
//! "arithmetic MUST be arbitrary-width" mandate.
//!
//! **E-W1/M-1119 widening (2026-07-18).** The conversion utilities now route through `i128`
//! (`crate::ternary::mod`'s doc comment carries the full ceiling correction and the concrete
//! decode-side overflow finding this widening fixes). The WIDE-width tests below still use a
//! small-magnitude oracle at `WIDE = 60` (`3^60` vastly exceeds even `i128::MAX`, so this file does
//! not re-litigate `max_magnitude`'s own ceiling); a dedicated `m = 41` section below exercises the
//! newly-unblocked W-1 canonical width directly.
//!
//! The corresponding end-to-end (surface `.myc` → L1-eval ≡ L0-interp ≡ AOT) witness at 80 trits
//! lives in `mycelium-l1/tests/enablement.rs` (`trit_add_beyond_the_claimed_40_trit_cap_three_way`
//! / `trit_mul_beyond_the_claimed_40_trit_cap_three_way`).

use crate::ternary::*;
use crate::value::Trit;

// ── Core codec / arithmetic (extracted from `ternary/mod.rs`) ─────────────────────────────────

/// **Exhaustive truth-table proof** of the shared balanced full-adder over all 27 inputs: the
/// digit identity `a + b + carry_in == digit_out + 3·carry_out` holds exactly. This is the
/// regression guard for the DRY extraction — both [`add`] and [`BigTernary`] ripple this one
/// primitive, so a broken row fails here immediately (alongside `add_matches_integer_oracle`).
#[test]
fn add_with_carry_is_exhaustively_correct() {
    for a in [Trit::Neg, Trit::Zero, Trit::Pos] {
        for b in [Trit::Neg, Trit::Zero, Trit::Pos] {
            for c in [Trit::Neg, Trit::Zero, Trit::Pos] {
                let (d, carry) = add_with_carry(a, b, c);
                assert_eq!(
                    digit(a) + digit(b) + digit(c),
                    digit(d) + 3 * digit(carry),
                    "full-adder identity for ({a:?}, {b:?}, {c:?})"
                );
            }
        }
    }
}

/// Walk every integer representable in `m` trits, paired with its codec encoding.
fn each_in_range(m: u32, mut f: impl FnMut(i128, Vec<Trit>)) {
    let max = max_magnitude(m).unwrap();
    for v in -max..=max {
        f(v, int_to_trits(v, m).expect("in range"));
    }
}

#[test]
fn worked_example_matches_spec() {
    // binary-ternary.md §5: −78 in 6 trits is ⟨0,−1,0,0,+1,0⟩.
    let t = int_to_trits(-78, 6).unwrap();
    assert_eq!(
        t,
        vec![
            Trit::Zero,
            Trit::Neg,
            Trit::Zero,
            Trit::Zero,
            Trit::Pos,
            Trit::Zero
        ]
    );
    assert_eq!(trits_to_int(&t), -78);
}

#[test]
fn range_is_symmetric() {
    assert_eq!(max_magnitude(1), Some(1));
    assert_eq!(max_magnitude(6), Some(364)); // (3^6−1)/2
    assert_eq!(int_to_trits(365, 6), None); // just past the max → out of range
    assert_eq!(int_to_trits(-365, 6), None);
}

#[test]
fn codec_round_trips_exhaustively() {
    for m in 1..=5 {
        each_in_range(m, |v, t| {
            assert_eq!(t.len(), m as usize);
            assert_eq!(trits_to_int(&t), v, "round-trip at m={m}");
        });
    }
}

#[test]
fn neg_is_value_negation() {
    for m in 1..=5 {
        each_in_range(m, |v, t| {
            assert_eq!(trits_to_int(&neg(&t)), -v, "neg at m={m}");
        });
    }
}

/// **Oracle property test (add):** the digit-wise ripple-carry adder agrees with the integer
/// oracle for *every* pair at small widths — in range it equals the encoded sum, out of range
/// it is `None`.
#[test]
fn add_matches_integer_oracle() {
    for m in 1..=4 {
        let max = max_magnitude(m).unwrap();
        for x in -max..=max {
            for y in -max..=max {
                let a = int_to_trits(x, m).unwrap();
                let b = int_to_trits(y, m).unwrap();
                let got = add(&a, &b);
                let expected = x + y;
                if expected.abs() <= max {
                    assert_eq!(got, int_to_trits(expected, m), "add {x}+{y} at m={m}");
                } else {
                    assert_eq!(got, None, "add {x}+{y} should overflow at m={m}");
                }
            }
        }
    }
}

#[test]
fn sub_matches_integer_oracle() {
    for m in 1..=4 {
        let max = max_magnitude(m).unwrap();
        for x in -max..=max {
            for y in -max..=max {
                let a = int_to_trits(x, m).unwrap();
                let b = int_to_trits(y, m).unwrap();
                let got = sub(&a, &b);
                let expected = x - y;
                if expected.abs() <= max {
                    assert_eq!(got, int_to_trits(expected, m), "sub {x}-{y} at m={m}");
                } else {
                    assert_eq!(got, None, "sub {x}-{y} should overflow at m={m}");
                }
            }
        }
    }
}

/// **Oracle property test (mul):** the shifted-add multiplier agrees with the integer oracle for
/// every pair at small widths.
#[test]
fn mul_matches_integer_oracle() {
    for m in 1..=4 {
        let max = max_magnitude(m).unwrap();
        for x in -max..=max {
            for y in -max..=max {
                let a = int_to_trits(x, m).unwrap();
                let b = int_to_trits(y, m).unwrap();
                let got = mul(&a, &b);
                let expected = x * y;
                if expected.abs() <= max {
                    assert_eq!(got, int_to_trits(expected, m), "mul {x}*{y} at m={m}");
                } else {
                    assert_eq!(got, None, "mul {x}*{y} should overflow at m={m}");
                }
            }
        }
    }
}

#[test]
fn unequal_widths_are_rejected() {
    let a = int_to_trits(1, 2).unwrap();
    let b = int_to_trits(1, 3).unwrap();
    assert_eq!(add(&a, &b), None);
    assert_eq!(sub(&a, &b), None);
    assert_eq!(mul(&a, &b), None);
}

// ── E-W1/M-1119: the widened i128 ceiling, m = 41 (the W-1 canonical Ternary width) ───────────

/// **`max_magnitude(41)` now succeeds** (was the DoD's headline `None`→`Some` flip; mirrors the
/// pre-widening `max_magnitude_overflows_at_m41` in `mycelium-std-ternary`, now inverted). Also
/// pins the corrected pre-widening ceiling finding (m=40 already overflowed `i64`, one trit
/// earlier than the old doc comment claimed) by checking the widened function's headroom well
/// past both figures.
#[test]
fn max_magnitude_succeeds_at_m41_and_beyond() {
    assert_eq!(max_magnitude(39), Some(2_026_277_576_509_488_133));
    assert_eq!(max_magnitude(40), Some(6_078_832_729_528_464_400));
    assert_eq!(max_magnitude(41), Some(18_236_498_188_585_393_201));
    // i128 headroom continues to m = 80; m = 81 is the new explicit ceiling.
    assert!(max_magnitude(80).is_some());
    assert_eq!(max_magnitude(81), None);
}

/// **The W-1 canonical pair's exact round-trip.** `Binary{64}`'s full range (`i64::MIN..=i64::MAX`)
/// encodes and decodes exactly at `Ternary{41}` — including `i64::MIN`, the concrete value whose
/// decode-side Horner accumulation would have transiently overflowed the OLD `i64`
/// `trits_to_int` (confirmed by direct computation before this fix; `ternary/mod.rs`'s doc
/// comment records the finding). `docs/spec/swaps/binary-ternary.md` §A.3 names this pair
/// canonical; `3^40 < 2^64 ≤ 3^41` is exactly why 40 trits do not suffice but 41 do.
#[test]
fn m41_round_trip_covers_the_full_binary64_range() {
    let m = 41u32;
    for v in [
        i64::MIN,
        i64::MIN + 1,
        -1,
        0,
        1,
        i64::MAX - 1,
        i64::MAX,
        // Also the values that need the 41st (most-significant) trit — 40 trits alone
        // (max_magnitude(40) ≈ 6.079e18) cannot represent these, only 41 can.
        7_000_000_000_000_000_000,
        -7_000_000_000_000_000_000,
    ] {
        let value = i128::from(v);
        let trits = int_to_trits(value, m).unwrap_or_else(|| panic!("{v} must fit in {m} trits"));
        assert_eq!(trits.len(), m as usize);
        assert_eq!(trits_to_int(&trits), value, "round-trip at m={m} for v={v}");
    }
}

/// The all-`+` 41-trit value (`max_magnitude(41)`) exceeds `i64::MAX` — the exact magnitude that
/// distinguishes `Ternary{41}`'s range from `Binary{64}`'s (`T_41` is strictly larger than `B_64`,
/// per `binary-ternary.md` §2: a total bijection is impossible, the inverse is partial). Decoding
/// it is `trits_to_int`'s widened-but-still-`i128`-bounded honest domain — no panic, no silent
/// wrap, matches the module doc comment's disclosed residual.
#[test]
fn m41_all_plus_exceeds_binary64_range() {
    let all_plus = vec![Trit::Pos; 41];
    let decoded = trits_to_int(&all_plus);
    assert_eq!(decoded, max_magnitude(41).unwrap());
    assert!(
        decoded > i128::from(i64::MAX),
        "the all-+ 41-trit value must exceed Binary{{64}}'s range (partial right-inverse, §4 P2)"
    );
}

// ── WIDE-width arithmetic (well past the conversion-utility ceiling) ──────────────────────────

/// Well past the "~40-trit" figure the recon corrected; still an arbitrary, non-special width.
const WIDE: u32 = 60;

#[test]
fn add_matches_the_integer_oracle_at_wide_width() {
    for x in [-1000i128, -500, -1, 0, 1, 500, 1000] {
        for y in [-1000i128, -500, -1, 0, 1, 500, 1000] {
            let a = int_to_trits(x, WIDE).expect("small value fits WIDE trits");
            let b = int_to_trits(y, WIDE).expect("small value fits WIDE trits");
            let got = add(&a, &b).expect("small sums stay well within WIDE trits' range");
            assert_eq!(
                trits_to_int(&got),
                x + y,
                "add({x}, {y}) at width {WIDE} must match the integer oracle"
            );
        }
    }
}

#[test]
fn mul_matches_the_integer_oracle_at_wide_width() {
    for x in -50i128..=50 {
        for y in [-50i128, -10, -1, 0, 1, 10, 50] {
            let a = int_to_trits(x, WIDE).expect("small value fits WIDE trits");
            let b = int_to_trits(y, WIDE).expect("small value fits WIDE trits");
            let got = mul(&a, &b).expect("small products stay well within WIDE trits' range");
            assert_eq!(
                trits_to_int(&got),
                x * y,
                "mul({x}, {y}) at width {WIDE} must match the integer oracle"
            );
        }
    }
}

#[test]
fn neg_matches_the_integer_oracle_at_wide_width() {
    for x in [-1000i128, -500, -1, 0, 1, 500, 1000] {
        let a = int_to_trits(x, WIDE).expect("small value fits WIDE trits");
        let got = neg(&a);
        assert_eq!(
            trits_to_int(&got),
            -x,
            "neg({x}) at width {WIDE} must match the oracle"
        );
    }
}

/// `add` at 200 trits — far past any oracle's reach (`3^200` vastly exceeds even `i128::MAX`), so
/// this checks the algorithm's *shape* directly (low digits carry the value, high digits stay
/// zero) rather than via `trits_to_int` on the full width. Confirms nothing in `add` depends on a
/// width ceiling tied to a machine integer (there is none in the algorithm — see the module note).
#[test]
fn add_operates_structurally_at_200_trits_far_past_any_oracle() {
    let n = 200usize;
    let a = vec![Trit::Zero; n]; // 0
    let mut b = vec![Trit::Zero; n];
    *b.last_mut().expect("n > 0") = Trit::Pos; // 1
    let sum = add(&a, &b).expect("0 + 1 must be in range at any width");
    assert_eq!(sum.len(), n, "add must preserve width");
    assert_eq!(
        trits_to_int(&sum[(n - 10)..]),
        1,
        "the low 10 digits, read on their own, must equal 1"
    );
    assert!(
        sum[..(n - 10)].iter().all(|&t| t == Trit::Zero),
        "every digit above the low 10 must stay Zero"
    );
}

// ── BigTernary (arbitrary-width; extracted from `ternary/big_ternary.rs`) ─────────────────────

fn bt(v: i128) -> BigTernary {
    BigTernary::from_i128(v)
}

#[test]
fn roundtrip_i128() {
    for v in [-1_000_000i128, -42, -1, 0, 1, 13, 14, 364, 365, 9_999_999] {
        assert_eq!(bt(v).to_i128(), Some(v), "roundtrip {v}");
    }
}

#[test]
fn big_ternary_add_matches_integer() {
    for a in [-50i128, -1, 0, 7, 121] {
        for b in [-121i128, -7, 0, 1, 50] {
            assert_eq!(bt(a).add(&bt(b)).to_i128(), Some(a + b), "{a}+{b}");
        }
    }
}

#[test]
fn big_ternary_sub_matches_integer() {
    for a in [-50i128, -1, 0, 7, 121] {
        for b in [-121i128, -7, 0, 1, 50] {
            assert_eq!(bt(a).sub(&bt(b)).to_i128(), Some(a - b), "{a}-{b}");
        }
    }
}

#[test]
fn big_ternary_mul_matches_integer() {
    for a in [-40i128, -3, 0, 1, 27] {
        for b in [-27i128, -1, 0, 3, 40] {
            assert_eq!(bt(a).mul(&bt(b)).to_i128(), Some(a * b), "{a}*{b}");
        }
    }
}

#[test]
fn big_ternary_negation_round_trips() {
    for v in [-9_999i128, -7, -1, 0, 1, 42, 365] {
        assert_eq!(bt(v).neg().to_i128(), Some(-v), "neg {v}");
        assert_eq!(bt(v).neg().neg(), bt(v), "double-neg {v}");
    }
}

#[test]
fn big_ternary_zero_is_canonical_empty() {
    assert!(BigTernary::zero().is_zero());
    assert_eq!(BigTernary::zero().width(), 0);
    assert_eq!(bt(0), BigTernary::zero());
    assert_eq!(bt(0).to_i128(), Some(0));
    // 1 + (−1) canonicalizes back to the empty zero, not a padded form.
    assert_eq!(bt(1).add(&bt(-1)), BigTernary::zero());
}

#[test]
fn big_ternary_beyond_40_trits_is_exact_not_silent() {
    // 3^41 exceeds the fixed-width i64 path's ORIGINAL range (which is never-silent there);
    // BigTernary grows to width 42 and stays exact. The headline "removes the cap" witness —
    // unaffected by E-W1's i128 widening of the fixed-width conversion utilities above (this is
    // the fully arbitrary-width path, M-756).
    let mut x = BigTernary::from_i128(1);
    let three = BigTernary::from_i128(3);
    for _ in 0..41 {
        x = x.mul(&three);
    }
    assert_eq!(x.width(), 42);
    assert_eq!(x.to_i128(), Some(3i128.pow(41)));
}

#[test]
fn big_ternary_fixed_width_overflow_is_none() {
    // width 3 holds [−13, 13]. 13 + 1 = 14 overflows → None.
    let a = bt(13).checked_to_width(3).unwrap();
    let one = bt(1).checked_to_width(3).unwrap();
    assert_eq!(checked_add_fixed(&a, &one), None);
    // 6 + 6 = 12 still fits width 3.
    let six = bt(6).checked_to_width(3).unwrap();
    let r = checked_add_fixed(&six, &six).unwrap();
    assert_eq!(r.to_big().to_i128(), Some(12));
}

#[test]
fn big_ternary_narrowing_is_never_silent() {
    assert!(bt(13).checked_to_width(3).is_some());
    assert!(bt(14).checked_to_width(3).is_none()); // 14 needs 4 trits
}

/// Cross-check the growable type against the fixed-width [`add`] within range — two independent
/// implementations must agree. Bridged **only through the integer value** (most-significant-first
/// `add` vs least-significant-first `BigTernary`), never by comparing trit vectors directly — the
/// honest way to reconcile the two endiannesses.
#[test]
fn big_agrees_with_fixed_width_add_in_range() {
    let m = 4u32;
    let max = max_magnitude(m).unwrap();
    for x in -max..=max {
        for y in -max..=max {
            let big = BigTernary::from_i128(x + y);
            let a = int_to_trits(x, m).unwrap();
            let b = int_to_trits(y, m).unwrap();
            match add(&a, &b) {
                // in range ⇒ both agree on the integer value
                Some(s) => {
                    assert_eq!(big.to_i128(), Some(trits_to_int(&s)), "{x}+{y}");
                }
                // fixed-width overflow ⇒ the value needs > m trits ⇒ BigTernary is wider
                None => assert!(big.width() > m as usize, "{x}+{y} should exceed width {m}"),
            }
        }
    }
}
