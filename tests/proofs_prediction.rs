//! Refund-mode market resolution proofs.
//!
//! Formal verification of `resolve_market_refund_not_atomic` — the
//! engine entry point that transitions a Live binary-outcome market to
//! `Resolved` mode without applying a settlement price (the prediction-
//! market INVALID outcome path documented in proposal §2.6).
//!
//! Harnesses are organized by scenario complexity, starting with the
//! empty-market case and growing to multi-account scenarios as they
//! land:
//!
//! - empty market: zero used accounts; drain/finalize gates bypassed.
//! - single account, no open position: per-account loop runs but the
//!   helper is a no-op on that slot.
//! - single account with an open position: helper invokes the canonical
//!   decrement-only `attach_effective_position(_, 0)` path.
//! - two accounts bilateral: both sides of the book drain to zero.
//! - multi-account / edge cases: open-ended additional invariants.
//!
//! Precondition-rejection harnesses live alongside the positive harness
//! to verify each early-exit guard fires with the documented `Err`
//! variant and leaves engine state observably unchanged.
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
// Shared scaffolding: pre-state snapshot, symbolic-input builders, and
// baseline assertion helpers. Every refund-mode harness uses the same
// construction pattern, so we factor it once. Future harnesses (single-
// account, bilateral, multi-account) plug in here by adding their own
// pre-state shaping between `build_symbolic_live_engine()` and
// `RefundPreState::snapshot()`.
// ============================================================================

/// Snapshot of every engine field that refund-mode harnesses need to
/// compare against post-call. Captured before the call so it still
/// reflects the pre-call view even after the per-account loop has run.
struct RefundPreState {
    oracle: u64,
    pnl_pos_tot: u128,
    current_slot: u64,
    last_market_slot: u64,
    market_mode: MarketMode,
    pre_stored_long: u64,
    pre_stored_short: u64,
    side_mode_long: SideMode,
    side_mode_short: SideMode,
}

impl RefundPreState {
    fn snapshot(engine: &RiskEngine) -> Self {
        Self {
            oracle: engine.last_oracle_price,
            pnl_pos_tot: engine.pnl_pos_tot,
            current_slot: engine.current_slot,
            last_market_slot: engine.last_market_slot,
            market_mode: engine.market_mode,
            pre_stored_long: engine.stored_pos_count_long,
            pre_stored_short: engine.stored_pos_count_short,
            side_mode_long: engine.side_mode_long,
            side_mode_short: engine.side_mode_short,
        }
    }
}

/// Build a Live-mode engine with symbolic `last_oracle_price` and
/// symbolic `pnl_pos_tot`. Both inputs are bounded for CBMC tractability
/// (`oracle > 0 && oracle <= 1_000_000`; `pnl_pos_tot <= 1_000_000`);
/// the bounds span a non-trivial range and the engine's contract
/// behavior on these fields is value-agnostic over any positive range,
/// so widening them changes solver cost without changing what is proved.
fn build_symbolic_live_engine() -> RiskEngine {
    let oracle: u64 = kani::any();
    kani::assume(oracle > 0 && oracle <= 1_000_000);
    let mut engine = RiskEngine::new_with_market(refund_proof_params(), DEFAULT_SLOT, oracle);

    let pnl_pos_tot: u128 = kani::any();
    kani::assume(pnl_pos_tot <= 1_000_000);
    engine.pnl_pos_tot = pnl_pos_tot;

    engine
}

/// Return a symbolic `now_slot` constrained to satisfy the engine's
/// slot-monotonicity preconditions (`now_slot >= current_slot &&
/// now_slot >= last_market_slot`). Success harnesses use this directly;
/// rejection harnesses construct their own (violating) `now_slot`.
fn symbolic_monotone_slot(engine: &RiskEngine) -> u64 {
    let now_slot: u64 = kani::any();
    kani::assume(now_slot >= engine.current_slot);
    kani::assume(now_slot >= engine.last_market_slot);
    now_slot
}

/// Baseline post-condition gate for every successful refund resolution.
/// Asserts the documented contract field-by-field. Future harnesses
/// (with-position, bilateral, multi-account) consume this baseline and
/// add their own per-account assertions on top.
fn assert_refund_success_postconditions(
    engine: &RiskEngine,
    pre: &RefundPreState,
    now_slot: u64,
) {
    // Mode + slot bookkeeping.
    assert!(matches!(engine.market_mode, MarketMode::Resolved));
    assert!(engine.resolved_slot == now_slot);
    assert!(engine.current_slot == now_slot);
    assert!(engine.last_market_slot == now_slot);

    // No-settlement-price sentinel: resolved_price / resolved_live_price
    // carry the pre-call oracle unchanged.
    assert!(engine.resolved_price == pre.oracle);
    assert!(engine.resolved_live_price == pre.oracle);
    assert!(engine.resolved_k_long_terminal_delta == 0);
    assert!(engine.resolved_k_short_terminal_delta == 0);

    // Resolved-payout snapshot fields are zero sentinels.
    assert!(engine.resolved_payout_h_num == 0);
    assert!(engine.resolved_payout_h_den == 0);
    assert!(engine.resolved_payout_ready == 0);

    // Both sides of OI drained to zero.
    assert!(engine.oi_eff_long_q == 0);
    assert!(engine.oi_eff_short_q == 0);

    // Positive-PnL maturation: pnl_matured_pos_tot equals the pre-call
    // pnl_pos_tot, and equals the post-call pnl_pos_tot (the per-account
    // loop on an empty market does not change pnl_pos_tot, and future
    // populated harnesses must preserve this identity).
    assert!(engine.pnl_matured_pos_tot == pre.pnl_pos_tot);
    assert!(engine.pnl_matured_pos_tot == engine.pnl_pos_tot);

    // Standard public-postcondition gate (the function calls this
    // internally before returning Ok; we re-call it for direct inspection).
    assert!(engine.assert_public_postconditions().is_ok());
}

/// Baseline post-condition gate for every rejected refund resolution.
/// Asserts the engine is observably unchanged on every field the
/// success path would have mutated. A future refactor that writes state
/// before returning Err from a precondition guard fails this gate.
fn assert_engine_unchanged_on_reject(engine: &RiskEngine, pre: &RefundPreState) {
    assert!(engine.market_mode == pre.market_mode);
    assert!(engine.last_oracle_price == pre.oracle);
    assert!(engine.pnl_pos_tot == pre.pnl_pos_tot);
    assert!(engine.current_slot == pre.current_slot);
    assert!(engine.last_market_slot == pre.last_market_slot);
    assert!(engine.stored_pos_count_long == pre.pre_stored_long);
    assert!(engine.stored_pos_count_short == pre.pre_stored_short);
    assert!(engine.side_mode_long == pre.side_mode_long);
    assert!(engine.side_mode_short == pre.side_mode_short);
}

// ============================================================================
// Empty-market success — refund-mode resolution yields the documented
// post-state when zero accounts are used. Symbolic oracle + symbolic
// pnl_pos_tot prove the maturation copy and the placeholder oracle copy
// are value-agnostic.
// ============================================================================

/// On a Live market with no used account slots, symbolic
/// `last_oracle_price`, and symbolic `pnl_pos_tot`,
/// `resolve_market_refund_not_atomic` succeeds and produces the
/// documented post-state. The per-account loop is a no-op (nothing in
/// the bitmap), drain/finalize gates are bypassed (`pre_stored_long`
/// and `pre_stored_short` are both zero), the placeholder copy of
/// `last_oracle_price` into `resolved_price` / `resolved_live_price`
/// is proved value-agnostic by the symbolic oracle, and the
/// `pnl_matured_pos_tot = pnl_pos_tot` assignment is non-vacuous via
/// the symbolic pre-call `pnl_pos_tot`.
#[kani::proof]
#[kani::unwind(5)]
#[kani::solver(cadical)]
fn proof_refund_empty_market_resolves() {
    let mut engine = build_symbolic_live_engine();
    let pre = RefundPreState::snapshot(&engine);
    let now_slot = symbolic_monotone_slot(&engine);

    assert!(engine.resolve_market_refund_not_atomic(now_slot).is_ok());
    assert_refund_success_postconditions(&engine, &pre, now_slot);
}

// ============================================================================
// Precondition-rejection harnesses — each of the four guards at the head
// of `resolve_market_refund_not_atomic` must reject with the documented
// `Err` variant and leave the engine observably unchanged. The
// bankrupt-close gate has two independent in-flight signals
// (`active_close_present` and `bankruptcy_hmax_lock_active`); each gets
// its own harness so a regression that removes either gate fails its
// own proof.
// ============================================================================

/// Guard 1: `market_mode != Live` rejects with `Unauthorized`.
#[kani::proof]
#[kani::unwind(5)]
#[kani::solver(cadical)]
fn proof_refund_rejects_when_mode_not_live() {
    let mut engine = build_symbolic_live_engine();
    engine.market_mode = MarketMode::Resolved;
    let pre = RefundPreState::snapshot(&engine);
    let now_slot = symbolic_monotone_slot(&engine);

    let result = engine.resolve_market_refund_not_atomic(now_slot);
    assert!(matches!(result, Err(RiskError::Unauthorized)));
    assert_engine_unchanged_on_reject(&engine, &pre);
}

/// Guard 2a: `active_close_present != 0` rejects with `RecoveryRequired`
/// via `ensure_no_active_bankrupt_close`.
#[kani::proof]
#[kani::unwind(5)]
#[kani::solver(cadical)]
fn proof_refund_rejects_when_active_close_present() {
    let mut engine = build_symbolic_live_engine();
    engine.active_close_present = 1;
    let pre = RefundPreState::snapshot(&engine);
    let now_slot = symbolic_monotone_slot(&engine);

    let result = engine.resolve_market_refund_not_atomic(now_slot);
    assert!(matches!(result, Err(RiskError::RecoveryRequired)));
    assert_engine_unchanged_on_reject(&engine, &pre);
}

/// Guard 2b: `bankruptcy_hmax_lock_active = true` rejects with
/// `RecoveryRequired` via the same gate. Covers the other in-flight
/// signal independently of `active_close_present`.
#[kani::proof]
#[kani::unwind(5)]
#[kani::solver(cadical)]
fn proof_refund_rejects_when_bankruptcy_hmax_lock_active() {
    let mut engine = build_symbolic_live_engine();
    engine.bankruptcy_hmax_lock_active = true;
    let pre = RefundPreState::snapshot(&engine);
    let now_slot = symbolic_monotone_slot(&engine);

    let result = engine.resolve_market_refund_not_atomic(now_slot);
    assert!(matches!(result, Err(RiskError::RecoveryRequired)));
    assert_engine_unchanged_on_reject(&engine, &pre);
}

/// Guard 3: `now_slot < current_slot` rejects with `Overflow`
/// (slot monotonicity).
#[kani::proof]
#[kani::unwind(5)]
#[kani::solver(cadical)]
fn proof_refund_rejects_when_now_slot_below_current_slot() {
    let mut engine = build_symbolic_live_engine();
    kani::assume(engine.current_slot >= 1);
    let pre = RefundPreState::snapshot(&engine);
    let now_slot: u64 = kani::any();
    kani::assume(now_slot < engine.current_slot);

    let result = engine.resolve_market_refund_not_atomic(now_slot);
    assert!(matches!(result, Err(RiskError::Overflow)));
    assert_engine_unchanged_on_reject(&engine, &pre);
}

/// Guard 4: `now_slot < last_market_slot` rejects with `Overflow`.
/// Exercised independently of guard 3 by raising `last_market_slot`
/// above `current_slot` so a `now_slot` in
/// `[current_slot, last_market_slot)` trips guard 4 without tripping
/// guard 3.
#[kani::proof]
#[kani::unwind(5)]
#[kani::solver(cadical)]
fn proof_refund_rejects_when_now_slot_below_last_market_slot() {
    let mut engine = build_symbolic_live_engine();
    let bump: u64 = kani::any();
    kani::assume(bump >= 1 && bump <= 8);
    engine.last_market_slot = engine.current_slot + bump;
    let pre = RefundPreState::snapshot(&engine);
    let now_slot: u64 = kani::any();
    kani::assume(now_slot >= engine.current_slot);
    kani::assume(now_slot < engine.last_market_slot);

    let result = engine.resolve_market_refund_not_atomic(now_slot);
    assert!(matches!(result, Err(RiskError::Overflow)));
    assert_engine_unchanged_on_reject(&engine, &pre);
}
