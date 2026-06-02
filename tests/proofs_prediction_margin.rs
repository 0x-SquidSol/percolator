//! Side-aware notional proofs for `market_kind = 2` (Polymarket-perp).
//!
//! These harnesses formalize the bounded-domain margin invariant added
//! alongside the side-aware branch in `RiskEngine::risk_notional_from_eff_q`:
//!
//!   * For `params.market_kind != 2`, behaviour is **bit-identical** to
//!     the pre-change symmetric formula. Any drift would silently break
//!     every legacy perp/hyperp/native-prediction market on the deploy.
//!     `proof_kind0_notional_unchanged_after_dispatch_refactor` is the
//!     regression bound — if it fails, the refactor mutated behaviour
//!     on kind=0 markets.
//!
//!   * For `params.market_kind == 2`, notional reflects the
//!     side-conditioned max loss per unit:
//!         long  → `|eff| * p             / POS_SCALE`
//!         short → `|eff| * (POS_SCALE-p) / POS_SCALE`
//!     i.e. a long at `p = 0.10` carries notional `0.10 * |eff|` (max
//!     loss per unit is `p`), and a short at the same price carries
//!     notional `0.90 * |eff|` (max loss per unit is `1-p`). This is
//!     the central correctness property the symmetric formula was
//!     violating.
//!
//! Together the two proofs establish: kind=0 unchanged, kind=2
//! correctly side-aware.
//!
//! Engine context: `RiskEngine::risk_notional_from_eff_q` and the
//! `params.market_kind` field on `RiskParams`.

#![cfg(kani)]

mod common;
use common::*;
use percolator::{wide_math, POS_SCALE};

/// Build a fresh engine with `market_kind = 2` and an oracle that
/// quotes p_yes in `POS_SCALE` units. Kept tiny so the bounded-trace
/// scan is tractable.
fn build_kind2_engine(oracle_price_e6: u64) -> RiskEngine {
    let mut params = zero_fee_params();
    params.max_accounts = 4;
    params.market_kind = 2;
    let mut e = RiskEngine::new();
    e.init_in_place(params, DEFAULT_SLOT, oracle_price_e6)
        .expect("kind2 engine init");
    e
}

/// Build a fresh kind=0 engine for the regression proof.
fn build_kind0_engine(oracle_price: u64) -> RiskEngine {
    let mut params = zero_fee_params();
    params.max_accounts = 4;
    // market_kind already 0 from zero_fee_params; explicit for clarity.
    params.market_kind = 0;
    let mut e = RiskEngine::new();
    e.init_in_place(params, DEFAULT_SLOT, oracle_price)
        .expect("kind0 engine init");
    e
}

/// Reference notional for kind=2 — the spec form, recomputed here
/// without reading the engine, so the engine's branch is checked
/// against an independent calculation.
fn ref_kind2_notional(eff: i128, p_e6: u64) -> u128 {
    if eff == 0 {
        return 0;
    }
    let abs = eff.unsigned_abs();
    let p = (p_e6 as u128).min(POS_SCALE - 1);
    let factor = if eff < 0 { POS_SCALE - p } else { p };
    wide_math::mul_div_ceil_u128(abs, factor, POS_SCALE)
}

/// Reference notional for kind=0 — the pre-refactor symmetric form.
fn ref_kind0_notional(eff: i128, oracle_price: u64) -> u128 {
    if eff == 0 {
        return 0;
    }
    wide_math::mul_div_ceil_u128(eff.unsigned_abs(), oracle_price as u128, POS_SCALE)
}

// ============================================================================
// Proof 1: kind=2 side-aware notional matches the side-conditioned spec.
// ============================================================================

#[kani::proof]
fn proof_kind2_notional_matches_side_aware_spec() {
    // Bound `p` to the tradeable + clamped domain. The wrapper-side ring
    // writer enforces `[10_000, 990_000]` on every snapshot; widen
    // slightly to [1, 999_999] so the boundary is covered.
    let p_e6: u64 = kani::any();
    kani::assume(p_e6 >= 1 && p_e6 <= POS_SCALE as u64 - 1);

    // Bound effective position size to a representative dynamic range.
    // Production caps `|eff|` at `MAX_TRADE_SIZE_Q`-ish bounds; the
    // exact value is irrelevant for the formula, only that `eff` is
    // signed and arithmetic stays inside u128 after the divide.
    let eff: i128 = kani::any();
    kani::assume(eff != i128::MIN);
    kani::assume(eff.unsigned_abs() <= 1u128 << 60);

    let engine = build_kind2_engine(p_e6);
    let got = engine.risk_notional_from_eff_q(eff, p_e6);
    let expected = ref_kind2_notional(eff, p_e6);
    assert_eq!(got, expected);
}

// ============================================================================
// Proof 2: kind=0 notional unchanged after the dispatch refactor.
// ============================================================================

#[kani::proof]
fn proof_kind0_notional_unchanged_after_dispatch_refactor() {
    let oracle_price: u64 = kani::any();
    kani::assume(oracle_price > 0 && oracle_price <= MAX_ORACLE_PRICE);
    let eff: i128 = kani::any();
    kani::assume(eff != i128::MIN);
    kani::assume(eff.unsigned_abs() <= 1u128 << 60);

    let engine = build_kind0_engine(oracle_price);
    let got = engine.risk_notional_from_eff_q(eff, oracle_price);
    let expected = ref_kind0_notional(eff, oracle_price);
    assert_eq!(got, expected);
}

// ============================================================================
// Proof 3: short notional > long notional for the same |size| at p < 0.5.
// Captures the qualitative asymmetric-margin property without needing to
// materialize an account or run the IM gate.
// ============================================================================

#[kani::proof]
fn proof_kind2_short_strictly_heavier_when_p_below_half() {
    // Restrict to p ∈ [1, POS_SCALE/2 - 1] so 1-p > p strictly.
    let p_e6: u64 = kani::any();
    kani::assume(p_e6 >= 1 && p_e6 < (POS_SCALE / 2) as u64);

    let size_q: i128 = kani::any();
    kani::assume(size_q > 0 && size_q.unsigned_abs() <= 1u128 << 60);

    let engine = build_kind2_engine(p_e6);
    let long_notional = engine.risk_notional_from_eff_q(size_q, p_e6);
    let short_notional = engine.risk_notional_from_eff_q(-size_q, p_e6);

    // Short carries strictly more notional at p < 0.5. Equivalent to
    // "short max-loss-per-unit (1-p) > long max-loss-per-unit (p)".
    assert!(short_notional > long_notional);
}
