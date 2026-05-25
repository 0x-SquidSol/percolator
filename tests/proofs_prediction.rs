//! Refund-mode market resolution proofs.
//!
//! Formal verification of `resolve_market_refund_not_atomic` — the
//! engine entry point that transitions a Live binary-outcome market to
//! `Resolved` mode without applying a settlement price (the prediction-
//! market INVALID outcome path documented in proposal §2.6).
//!
//! Harnesses are organized by scenario complexity, starting with the
//! empty-market case below and growing to multi-account scenarios as
//! they land:
//!
//! - empty market: zero used accounts; drain/finalize gates bypassed.
//! - single account, no open position: per-account loop runs but the
//!   helper is a no-op on that slot.
//! - single account with an open position: helper invokes the canonical
//!   decrement-only `attach_effective_position(_, 0)` path.
//! - two accounts bilateral: both sides of the book drain to zero.
//! - multi-account / edge cases: open-ended additional invariants.
//!
//! Engine-side context is in `src/percolator.rs` next to the
//! `resolve_market_refund_not_atomic` definition.

#![cfg(kani)]

mod common;
use common::*;

/// Small `max_accounts` keeps the per-account loop unwind bound
/// tractable for CBMC. The production cap is `MAX_ACCOUNTS`; for proof
/// purposes the function's logic is the same whether the loop iterates
/// 4 times or 4096.
fn refund_proof_params() -> RiskParams {
    let mut p = zero_fee_params();
    p.max_accounts = 4;
    p
}

// ============================================================================
// Empty market — refund-mode resolution succeeds and yields the documented
// post-state when zero accounts are used.
// ============================================================================

/// On a Live market with no used account slots,
/// `resolve_market_refund_not_atomic` succeeds and transitions the engine
/// to Resolved with the documented sentinel values intact:
///
/// - `oi_eff_long_q` and `oi_eff_short_q` both zero,
/// - `resolved_payout_h_num` / `_h_den` / `_ready` all zero,
/// - `resolved_price` and `resolved_live_price` set to the pre-call oracle,
/// - `pnl_matured_pos_tot` set to `pnl_pos_tot`,
/// - the standard `assert_public_postconditions` gate passing.
///
/// The per-account loop is a no-op (nothing in the bitmap), and the
/// drain/finalize gates are bypassed because both `pre_stored_long` and
/// `pre_stored_short` are zero.
#[kani::proof]
#[kani::unwind(5)]
#[kani::solver(cadical)]
fn proof_refund_empty_market_succeeds_and_satisfies_postconditions() {
    let mut engine =
        RiskEngine::new_with_market(refund_proof_params(), DEFAULT_SLOT, DEFAULT_ORACLE);

    // Harness preconditions — Live market, no positions, oracle set.
    assert!(matches!(engine.market_mode, MarketMode::Live));
    let pre_oracle = engine.last_oracle_price;
    let pre_pnl_pos_tot = engine.pnl_pos_tot;

    // Symbolic `now_slot` constrained to satisfy slot monotonicity.
    let now_slot: u64 = kani::any();
    kani::assume(now_slot >= engine.current_slot);
    kani::assume(now_slot >= engine.last_market_slot);

    let result = engine.resolve_market_refund_not_atomic(now_slot);
    assert!(
        result.is_ok(),
        "refund-mode resolve must succeed on empty market"
    );

    // Mode transition.
    assert!(
        matches!(engine.market_mode, MarketMode::Resolved),
        "market_mode must transition to Resolved"
    );

    // Slot bookkeeping.
    assert!(engine.resolved_slot == now_slot);
    assert!(engine.current_slot == now_slot);
    assert!(engine.last_market_slot == now_slot);

    // Sentinel values for the no-settlement-price branch.
    assert!(engine.resolved_price == pre_oracle);
    assert!(engine.resolved_live_price == pre_oracle);
    assert!(engine.resolved_k_long_terminal_delta == 0);
    assert!(engine.resolved_k_short_terminal_delta == 0);

    // Resolved-payout snapshot fields are zero sentinels.
    assert!(engine.resolved_payout_h_num == 0);
    assert!(engine.resolved_payout_h_den == 0);
    assert!(engine.resolved_payout_ready == 0);

    // Both sides of OI drained to zero.
    assert!(engine.oi_eff_long_q == 0);
    assert!(engine.oi_eff_short_q == 0);

    // Positive-PnL maturation.
    assert!(engine.pnl_matured_pos_tot == pre_pnl_pos_tot);
    assert!(engine.pnl_matured_pos_tot == engine.pnl_pos_tot);

    // Standard public-postcondition gate. The function calls this
    // internally before returning Ok; this is a redundant defense
    // verifying the post-state stays coherent under direct inspection.
    assert!(engine.assert_public_postconditions().is_ok());
}
