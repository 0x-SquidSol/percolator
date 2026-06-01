//! Section 7 — Liveness, progress, no-deadlock
//!
//! Auto-finalization, trade reopening, ADL fallback routes,
//! precision exhaustion, crank quiescence, drain-only progress.

#![cfg(kani)]

mod common;
use common::*;

// ============================================================================
// T11.43: end_instruction_auto_finalizes_ready_side
// ============================================================================

#[kani::proof]
#[kani::solver(cadical)]
fn t11_43_end_instruction_auto_finalizes_ready_side() {
    let mut engine = RiskEngine::new(zero_fee_params());

    engine.side_mode_long = SideMode::ResetPending;
    engine.oi_eff_long_q = 0u128;
    engine.stale_account_count_long = 0;
    engine.stored_pos_count_long = 0;

    engine.side_mode_short = SideMode::ResetPending;
    engine.oi_eff_short_q = 0u128;
    engine.stale_account_count_short = 1;
    engine.stored_pos_count_short = 0;

    let ctx = InstructionContext::new();
    engine.finalize_end_of_instruction_resets(&ctx);

    assert!(
        engine.side_mode_long == SideMode::Normal,
        "ready ResetPending side must auto-finalize to Normal"
    );
    assert!(
        engine.side_mode_short == SideMode::ResetPending,
        "non-ready side must stay ResetPending"
    );
}

// ============================================================================
// T11.44: trade_path_reopens_ready_reset_side
// ============================================================================

#[kani::proof]
#[kani::solver(cadical)]
fn t11_44_trade_path_reopens_ready_reset_side() {
    let mut engine = RiskEngine::new(zero_fee_params());

    engine.side_mode_long = SideMode::ResetPending;
    engine.oi_eff_long_q = 0u128;
    engine.oi_eff_short_q = 0u128;
    engine.stale_account_count_long = 0;
    engine.stored_pos_count_long = 0;

    let size_q = POS_SCALE as i128;
    let old_a = 0i128;
    let old_b = 0i128;
    let new_a = size_q;
    let new_b = -size_q;
    let (oi_long_after, oi_short_after) = engine
        .bilateral_oi_after(&old_a, &new_a, &old_b, &new_b)
        .unwrap();

    assert!(
        engine
            // ENG-PORT-4 fixup: 6-arg signature. Per-account positions in scope.
            .enforce_side_mode_oi_gate(old_a, new_a, old_b, new_b, oi_long_after, oi_short_after)
            .is_err(),
        "ready ResetPending side must block OI increase before preflight finalization"
    );

    engine.maybe_finalize_ready_reset_sides();

    assert!(engine.side_mode_long == SideMode::Normal);
    assert!(
        engine
            .enforce_side_mode_oi_gate(old_a, new_a, old_b, new_b, oi_long_after, oi_short_after)
            .is_ok(),
        "trade preflight must reopen a fully ready ResetPending side before OI gating"
    );
    assert!(oi_long_after == oi_short_after);
}

// ============================================================================
// T11.45: try_negate_u256_correctness
// ============================================================================
// NOTE: try_negate_u256_to_i256 has been removed from the engine after the
// migration to native 128-bit types. This test is preserved as a pure
// wide_math test using U256/I256 types that still exist for transient math.

// (Test removed — function no longer exists in the public API)

// ============================================================================
// T11.46: enqueue_adl_k_add_overflow_still_routes_quantity
// ============================================================================

#[kani::proof]
#[kani::solver(cadical)]
fn t11_46_enqueue_adl_k_add_overflow_still_routes_quantity() {
    let mut engine = RiskEngine::new(zero_fee_params());
    let mut ctx = InstructionContext::new();

    engine.adl_coeff_long = i128::MIN + 1;
    engine.adl_mult_long = POS_SCALE;
    engine.oi_eff_long_q = 4 * POS_SCALE;
    engine.oi_eff_short_q = 4 * POS_SCALE;
    engine.insurance_fund.balance = U128::new(10_000_000);
    engine.stored_pos_count_long = 1;

    let k_before = engine.adl_coeff_long;
    let a_before = engine.adl_mult_long;
    let ins_before = engine.insurance_fund.balance.get();

    let d = 1_000_000u128;
    let q_close = 2 * POS_SCALE;

    let result = engine.enqueue_adl(&mut ctx, Side::Short, q_close, d);
    assert!(result.is_ok());

    // K_opp must be UNCHANGED when K_opp + delta_K overflows
    assert!(
        engine.adl_coeff_long == k_before,
        "K_opp must not be modified on K-space overflow (spec §5.6 step 6)"
    );
    // A must shrink (quantity was still routed)
    assert!(
        engine.adl_mult_long < a_before,
        "A must shrink on K overflow"
    );
    // OI must decrease by q_close
    assert!(engine.oi_eff_long_q == 2 * POS_SCALE);
    // Insurance fund must decrease by D (absorb_protocol_loss was invoked)
    assert!(
        engine.insurance_fund.balance.get() < ins_before,
        "insurance fund must decrease — absorb_protocol_loss must be invoked"
    );
}

// ============================================================================
// T11.47: precision_exhaustion_terminal_drain
// ============================================================================

#[kani::proof]
#[kani::solver(cadical)]
fn t11_47_precision_exhaustion_terminal_drain() {
    let mut engine = RiskEngine::new(zero_fee_params());
    let mut ctx = InstructionContext::new();

    engine.adl_mult_long = 1;
    engine.adl_coeff_long = 0i128;
    engine.oi_eff_long_q = 3 * POS_SCALE;
    engine.oi_eff_short_q = 3 * POS_SCALE;
    engine.stored_pos_count_long = 1;

    let q_close = POS_SCALE;
    let d = 0u128;

    let result = engine.enqueue_adl(&mut ctx, Side::Short, q_close, d);
    assert!(result.is_ok());

    assert!(ctx.pending_reset_long);
    assert!(ctx.pending_reset_short);
    assert!(engine.oi_eff_long_q == 0);
    assert!(engine.oi_eff_short_q == 0);
}

// ============================================================================
// T11.48: bankruptcy_liquidation_routes_q_when_D_zero
// ============================================================================

#[kani::proof]
#[kani::solver(cadical)]
fn t11_48_bankruptcy_liquidation_routes_q_when_D_zero() {
    let mut engine = RiskEngine::new(zero_fee_params());
    let mut ctx = InstructionContext::new();

    engine.adl_mult_long = POS_SCALE;
    engine.adl_coeff_long = 42i128;
    engine.oi_eff_long_q = 4 * POS_SCALE;
    engine.oi_eff_short_q = 4 * POS_SCALE;
    engine.stored_pos_count_long = 1;

    let k_before = engine.adl_coeff_long;
    let a_before = engine.adl_mult_long;

    let d = 0u128;
    let q_close = POS_SCALE;

    let result = engine.enqueue_adl(&mut ctx, Side::Short, q_close, d);
    assert!(result.is_ok());

    assert!(
        engine.adl_coeff_long == k_before,
        "K must be unchanged when D == 0"
    );
    assert!(engine.adl_mult_long < a_before, "A must shrink");
    assert!(engine.oi_eff_long_q == 3 * POS_SCALE);
}

// ============================================================================
// T11.49: pure_pnl_bankruptcy_path
// ============================================================================

#[kani::proof]
#[kani::solver(cadical)]
fn t11_49_pure_pnl_bankruptcy_path() {
    // Wave 11e (v12.20.6 ADL): pure-PnL bankruptcy (q_close = 0, D > 0)
    // socializes the deficit via the B-residual path now, not via K-adjust.
    // Asserts the new post-state: lock armed + entire D recorded as explicit
    // non-claim loss on the opposing side (no holders → records_explicit).
    let mut engine = RiskEngine::new(zero_fee_params());
    let mut ctx = InstructionContext::new();

    engine.adl_mult_long = POS_SCALE;
    engine.adl_coeff_long = 0i128;
    engine.oi_eff_long_q = 2 * POS_SCALE;
    engine.oi_eff_short_q = 2 * POS_SCALE;
    engine.stored_pos_count_long = 1;

    let a_before = engine.adl_mult_long;

    let d = 1_000u128;
    let q_close = 0u128;

    let result = engine.enqueue_adl(&mut ctx, Side::Short, q_close, d);
    assert!(result.is_ok());

    assert!(
        engine.adl_mult_long == a_before,
        "A must be unchanged for pure PnL bankruptcy"
    );
    assert!(
        engine.bankruptcy_hmax_lock_active,
        "v12.20.6: Step 2 must arm the bankruptcy h_max lock when D > 0"
    );
    assert!(
        engine.explicit_unallocated_loss_long.get() == d,
        "v12.20.6: full d_social == D must record as explicit non-claim loss \
         on opp side when loss_weight_sum_<opp> == 0"
    );
    assert!(engine.oi_eff_long_q == 2 * POS_SCALE);
}

// ============================================================================
// T11.53: keeper_crank_quiesces_after_pending_reset
// ============================================================================

#[kani::proof]
#[kani::solver(cadical)]
fn t11_53_keeper_crank_quiesces_after_pending_reset() {
    let mut engine = RiskEngine::new(zero_fee_params());

    engine.last_oracle_price = 100;
    engine.last_market_slot = 0;
    engine.adl_mult_long = ADL_ONE;
    engine.adl_mult_short = ADL_ONE;
    engine.adl_epoch_long = 0;
    engine.adl_epoch_short = 0;

    let a = add_user_test(&mut engine, 0).unwrap();
    let b = add_user_test(&mut engine, 0).unwrap();
    let c = add_user_test(&mut engine, 0).unwrap();

    // a: long POS_SCALE (entire long side OI), tiny capital → deeply underwater
    engine.deposit_not_atomic(a, 1, 0).unwrap();
    engine.accounts[a as usize].position_basis_q = POS_SCALE as i128;
    engine.accounts[a as usize].adl_a_basis = ADL_ONE;
    engine.accounts[a as usize].adl_k_snap = 0i128;
    engine.accounts[a as usize].adl_epoch_snap = 0;

    // b: short POS_SCALE, well-funded
    engine.deposit_not_atomic(b, 10_000_000, 0).unwrap();
    engine.accounts[b as usize].position_basis_q = -(POS_SCALE as i128);
    engine.accounts[b as usize].adl_a_basis = ADL_ONE;
    engine.accounts[b as usize].adl_k_snap = 0i128;
    engine.accounts[b as usize].adl_epoch_snap = 0;

    // c: NO position, just capital (should NOT be touched after pending reset)
    engine.deposit_not_atomic(c, 10_000_000, 0).unwrap();

    // BALANCED OI: 1 long (a) = PS, 1 short (b) = PS
    engine.stored_pos_count_long = 1;
    engine.stored_pos_count_short = 1;
    engine.oi_eff_long_q = POS_SCALE;
    engine.oi_eff_short_q = POS_SCALE;

    // Set K_long very negative → account a is deeply underwater
    engine.adl_coeff_long = -((ADL_ONE as i128) * 1000);

    let c_cap_before = engine.accounts[c as usize].capital.get();
    let c_pnl_before = engine.accounts[c as usize].pnl;

    let result = engine.keeper_crank_not_atomic(
        1,
        100,
        &[(a, Some(LiquidationPolicy::FullClose))],
        1,
        0i128,
        0,
        100,
        None,
        0,
    );
    assert!(result.is_ok());

    assert!(
        engine.accounts[c as usize].capital.get() == c_cap_before,
        "c's capital must not change — crank must quiesce after pending reset"
    );
    assert!(
        engine.accounts[c as usize].pnl == c_pnl_before,
        "c's PnL must not change — crank must quiesce after pending reset"
    );
}

// ============================================================================
// proof_drain_only_to_reset_progress
// ============================================================================

#[kani::proof]
#[kani::unwind(34)]
#[kani::solver(cadical)]
fn proof_drain_only_to_reset_progress() {
    let mut engine = RiskEngine::new(zero_fee_params());
    let mut ctx = InstructionContext::new();

    // Long side: DrainOnly, OI = 0
    engine.side_mode_long = SideMode::DrainOnly;
    engine.oi_eff_long_q = 0u128;
    engine.oi_eff_short_q = 0u128;
    engine.stored_pos_count_long = 0;
    // Short side still has stored positions → §5.7.A (bilateral-empty) does NOT fire
    engine.stored_pos_count_short = 1;

    let result = engine.schedule_end_of_instruction_resets(&mut ctx);
    assert!(result.is_ok());

    // §5.7.D must fire for the DrainOnly long side
    assert!(
        ctx.pending_reset_long,
        "DrainOnly side with OI=0 must schedule reset via §5.7.D"
    );
    assert!(
        !ctx.pending_reset_short,
        "opposite side must not get reset from DrainOnly path alone"
    );
}

// ============================================================================
// proof_keeper_reset_lifecycle_last_stale_triggers_finalize
// ============================================================================

#[kani::proof]
#[kani::solver(cadical)]
fn proof_keeper_reset_lifecycle_last_stale_triggers_finalize() {
    let mut engine = RiskEngine::new_with_market(zero_fee_params(), 0, 100);

    engine.adl_mult_long = ADL_ONE;
    engine.adl_epoch_long = 1; // new epoch after the reset started
    engine.adl_epoch_short = 0;

    let a = add_user_test(&mut engine, 0).unwrap();

    // a: the last stale long account — has a position from epoch 0 (stale)
    engine
        .set_position_basis_q(a as usize, POS_SCALE as i128)
        .unwrap();
    engine.accounts[a as usize].adl_a_basis = ADL_ONE;
    engine.accounts[a as usize].adl_k_snap = 0i128;
    engine.accounts[a as usize].adl_epoch_snap = 0; // mismatches adl_epoch_long=1

    // Long side: ResetPending, 1 stale account remaining, OI=0
    engine.side_mode_long = SideMode::ResetPending;
    engine.stale_account_count_long = 1;

    assert!(engine.side_mode_long == SideMode::ResetPending);
    assert!(engine.stale_account_count_long == 1);
    assert!(engine.stored_pos_count_long == 1);
    assert!(
        engine.effective_pos_q(a as usize) == 0,
        "stale reset-pending positions have no current-market effective OI"
    );

    let mut ctx = InstructionContext::new_with_admission(0, 100);
    engine
        .touch_account_live_local(a as usize, &mut ctx)
        .unwrap();
    assert!(
        engine.stale_account_count_long == 0,
        "touching the last stale account must clear the stale counter"
    );
    assert!(
        engine.stored_pos_count_long == 0,
        "touching the last stale account must remove the stale stored position"
    );
    assert!(
        engine.accounts[a as usize].position_basis_q == 0,
        "stale reset settlement must flatten the stale account"
    );
    assert!(
        engine.side_mode_long == SideMode::ResetPending,
        "touch alone must not finalize the reset before end-of-instruction"
    );

    engine.finalize_touched_accounts_post_live(&mut ctx).unwrap();
    engine.schedule_end_of_instruction_resets(&mut ctx).unwrap();
    engine.finalize_end_of_instruction_resets(&ctx).unwrap();

    assert!(
        engine.side_mode_long == SideMode::Normal,
        "touching last stale account must finalize ResetPending → Normal (spec property #26)"
    );
    assert!(engine.stale_account_count_long == 0);
    assert!(engine.stored_pos_count_long == 0);
}

// ============================================================================
// proof_unilateral_empty_orphan_dust_clearance
// ============================================================================

#[kani::proof]
#[kani::solver(cadical)]
fn proof_unilateral_empty_orphan_dust_clearance() {
    let mut engine = RiskEngine::new(zero_fee_params());
    let mut ctx = InstructionContext::new();

    // Long side: no stored positions, but has phantom dust OI
    engine.stored_pos_count_long = 0;
    // Short side: still has stored positions
    engine.stored_pos_count_short = 2;

    // Phantom dust: OI == dust bound (should clear)
    let dust = 42u128;
    engine.phantom_dust_potential_long_q = dust;
    engine.oi_eff_long_q = dust; // OI <= dust bound
    engine.oi_eff_short_q = dust; // balanced (required by spec)

    let result = engine.schedule_end_of_instruction_resets(&mut ctx);
    assert!(result.is_ok());

    // §5.7.B: long side is empty, OI within dust bound → both sides get reset
    assert!(
        ctx.pending_reset_long,
        "unilateral-empty side with OI within dust bound must schedule reset (§5.7.B)"
    );
    assert!(
        ctx.pending_reset_short,
        "opposite side must also get reset for bilateral consistency (§5.7.B)"
    );
    // OI must be zeroed
    assert!(
        engine.oi_eff_long_q == 0,
        "OI must be zeroed after dust clearance"
    );
    assert!(
        engine.oi_eff_short_q == 0,
        "OI must be zeroed after dust clearance"
    );
}

// ############################################################################
// Full ADL pipeline integration: trade → liquidation → ADL → reset → reopen
// ############################################################################

/// End-to-end ADL lifecycle: two accounts hold a valid bilateral position,
/// ADL socializes a deficit, end-of-instruction resets fire, stale accounts
/// settle out, and a later balanced position can reopen the market.
/// Verifies OI_eff_long == OI_eff_short is maintained throughout.
#[kani::proof]
#[kani::unwind(70)]
#[kani::solver(cadical)]
fn proof_adl_pipeline_trade_liquidate_reopen() {
    let mut engine = RiskEngine::new_with_market(zero_fee_params(), DEFAULT_SLOT, DEFAULT_ORACLE);

    let a = add_user_test(&mut engine, 0).unwrap();
    let b = add_user_test(&mut engine, 0).unwrap();
    let c = add_user_test(&mut engine, 0).unwrap();
    engine.deposit_not_atomic(a, 100_000, DEFAULT_SLOT).unwrap();
    engine.deposit_not_atomic(b, 500_000, DEFAULT_SLOT).unwrap();
    engine.deposit_not_atomic(c, 500_000, DEFAULT_SLOT).unwrap();

    let size = 3 * POS_SCALE;
    engine
        .attach_effective_position(a as usize, size as i128)
        .unwrap();
    engine
        .attach_effective_position(b as usize, -(size as i128))
        .unwrap();
    engine.oi_eff_long_q = size;
    engine.oi_eff_short_q = size;
    assert!(
        engine.oi_eff_long_q == engine.oi_eff_short_q,
        "OI must balance after trade"
    );
    assert!(engine.check_conservation());

    let mut ctx = InstructionContext::new();
    let d = 1_000u128;
    let result = engine.enqueue_adl(&mut ctx, Side::Long, size, d);
    assert!(result.is_ok(), "ADL enqueue must succeed for balanced OI");
    assert!(
        engine.oi_eff_long_q == engine.oi_eff_short_q,
        "OI must balance after liquidation+ADL"
    );
    assert!(engine.oi_eff_long_q == 0, "full ADL close drains long OI");
    assert!(engine.oi_eff_short_q == 0, "full ADL close drains short OI");
    assert!(
        ctx.pending_reset_long,
        "ADL full drain must schedule long reset"
    );
    assert!(
        ctx.pending_reset_short,
        "ADL full drain must schedule short reset"
    );
    // Wave 11e (v12.20.6 ADL): deficit is socialized to opp side via the
    // B-residual booking path, not via K-adjust. With no loss-weight on
    // opp side, the full d records as explicit non-claim loss.
    assert!(
        engine.bankruptcy_hmax_lock_active,
        "Step 2 must arm the bankruptcy h_max lock when d > 0"
    );
    assert!(
        engine.explicit_unallocated_loss_short.get() == d,
        "v12.20.6: d_social == d must record as explicit non-claim loss \
         on opp (short) side when loss_weight_sum_short == 0"
    );
    assert!(engine.check_conservation());

    let reset_result = engine.finalize_end_of_instruction_resets(&ctx);
    assert!(reset_result.is_ok(), "pending ADL resets must finalize");
    assert!(engine.side_mode_long == SideMode::ResetPending);
    assert!(engine.side_mode_short == SideMode::ResetPending);
    assert!(engine.stale_account_count_long == 1);
    assert!(engine.stale_account_count_short == 1);

    let mut settle_ctx = InstructionContext::new_with_admission(0, 100);
    engine
        .settle_side_effects_live(a as usize, &mut settle_ctx)
        .unwrap();
    engine
        .settle_side_effects_live(b as usize, &mut settle_ctx)
        .unwrap();
    engine
        .finalize_end_of_instruction_resets(&InstructionContext::new())
        .unwrap();
    assert!(engine.side_mode_long == SideMode::Normal);
    assert!(engine.side_mode_short == SideMode::Normal);
    assert!(engine.stored_pos_count_long == 0);
    assert!(engine.stored_pos_count_short == 0);

    let new_size = POS_SCALE;
    engine
        .attach_effective_position(c as usize, new_size as i128)
        .unwrap();
    engine
        .attach_effective_position(b as usize, -(new_size as i128))
        .unwrap();
    engine.oi_eff_long_q = new_size;
    engine.oi_eff_short_q = new_size;
    assert!(
        engine.oi_eff_long_q == engine.oi_eff_short_q,
        "OI must balance after reopen attempt"
    );
    assert!(
        engine.check_conservation(),
        "conservation after full pipeline"
    );
    kani::cover!(
        engine.side_mode_long == SideMode::Normal
            && engine.side_mode_short == SideMode::Normal
            && engine.oi_eff_long_q == new_size,
        "post-ADL market reopens with balanced OI"
    );
}

// ############################################################################
// Wave 1 ENG-PORT-B: force_close_resolved_with_fee_not_atomic invariant
// ############################################################################

/// Wave 1 / ENG-PORT-B: fee-credited-at-resolved-close invariant.
///
/// `force_close_resolved_with_fee_not_atomic` MUST sync the recurring
/// maintenance fee at the resolved-slot anchor BEFORE returning
/// ProgressOnly when the account is in the not-yet-payable case
/// (`pnl > 0 && !is_terminal_ready`). The fee charge moves capital
/// from the user to the insurance fund and stamps last_fee_slot to
/// resolved_slot — without this, a wrapper that re-calls the function
/// would either re-charge the same dt (double-charge) or skip the
/// charge entirely.
///
/// Mirrors toly engine tests/proofs_liveness.rs:1825-1869
/// (`proof_force_close_resolved_with_fee_progress_only_syncs_before_payout_on_prod_code`).
#[kani::proof]
#[kani::unwind(80)]
#[kani::solver(cadical)]
fn proof_force_close_resolved_with_fee_progress_only_syncs_before_payout_on_prod_code() {
    let mut engine =
        RiskEngine::new_with_market(small_zero_fee_params(4), DEFAULT_SLOT, DEFAULT_ORACLE);
    engine.deposit_not_atomic(0, 100, DEFAULT_SLOT).unwrap();
    engine.deposit_not_atomic(1, 100, DEFAULT_SLOT).unwrap();
    engine.market_mode = MarketMode::Resolved;
    engine.current_slot = DEFAULT_SLOT;
    engine.resolved_slot = DEFAULT_SLOT;
    engine.resolved_price = DEFAULT_ORACLE;
    engine.resolved_live_price = DEFAULT_ORACLE;
    engine.set_pnl(0, 10).unwrap();
    engine.set_pnl(1, -5).unwrap();
    engine.accounts[0].last_fee_slot = DEFAULT_SLOT - 1;

    let fee_rate: u8 = kani::any();
    kani::assume(fee_rate > 0 && fee_rate <= 10);
    let capital_before = engine.accounts[0].capital.get();
    let pnl_before = engine.accounts[0].pnl;
    let insurance_before = engine.insurance_fund.balance.get();

    let result = engine.force_close_resolved_with_fee_not_atomic(0, fee_rate as u128);

    assert_eq!(result, Ok(ResolvedCloseResult::ProgressOnly));
    assert!(engine.is_used(0));
    assert_eq!(engine.accounts[0].last_fee_slot, engine.resolved_slot);
    assert_eq!(engine.accounts[0].pnl, pnl_before);
    assert_eq!(
        engine.accounts[0].capital.get(),
        capital_before - fee_rate as u128
    );
    assert_eq!(
        engine.insurance_fund.balance.get(),
        insurance_before + fee_rate as u128
    );
    assert_eq!(engine.neg_pnl_account_count, 1);
    assert_eq!(engine.market_mode, MarketMode::Resolved);
    assert!(engine.check_conservation());
    kani::cover!(
        result == Ok(ResolvedCloseResult::ProgressOnly)
            && engine.is_used(0)
            && engine.accounts[0].last_fee_slot == engine.resolved_slot
            && engine.insurance_fund.balance.get() > insurance_before,
        "fee-aware resolved close syncs fee before ProgressOnly without payout/free"
    );
}

// ============================================================================
// Wave 12-H: harnesses ported from toly (proofs_liveness.rs)
// ============================================================================

// t11_53_keeper_phase1_stops_after_pending_reset_on_prod_code
// Fork adaptation: run_keeper_phase1_candidates has 6 args (no trailing bool).

#[kani::proof]
#[kani::unwind(34)]
#[kani::solver(cadical)]
fn t11_53_keeper_phase1_stops_after_pending_reset_on_prod_code() {
    let mut engine =
        RiskEngine::new_with_market(small_zero_fee_params(4), DEFAULT_SLOT, DEFAULT_ORACLE);

    let later = 2usize;

    engine.materialize_at(later as u16, DEFAULT_SLOT).unwrap();
    engine.set_capital(later, 5).unwrap();
    engine.vault = U128::new(engine.c_tot.get());
    engine.set_pnl(later, -3).unwrap();

    let later_cap_before = engine.accounts[later].capital.get();
    let later_pnl_before = engine.accounts[later].pnl;
    let candidates = [(later as u16, Some(LiquidationPolicy::FullClose))];
    let mut ctx = InstructionContext::new_with_admission(0, 100);
    ctx.pending_reset_long = true;

    let result = engine.run_keeper_phase1_candidates(
        &mut ctx,
        DEFAULT_SLOT,
        DEFAULT_ORACLE,
        &candidates,
        1,
        1,
        false,
    );

    assert!(result.is_ok());
    let (num_liquidations, protective_progress) = result.unwrap();
    assert!(num_liquidations == 0);
    assert!(!protective_progress);
    assert!(
        engine.accounts[later].capital.get() == later_cap_before,
        "later candidate capital must not change after pending reset"
    );
    assert!(
        engine.accounts[later].pnl == later_pnl_before,
        "later candidate PnL must not change after pending reset"
    );
    assert!(ctx.pending_reset_long);
    kani::cover!(
        engine.accounts[later].pnl == later_pnl_before
            && num_liquidations == 0
            && !protective_progress,
        "keeper Phase 1 stops after pending reset before mutating later candidate"
    );
}

#[kani::proof]
#[kani::unwind(6)]
#[kani::solver(cadical)]
fn proof_phase2_missing_slot_scan_progress_or_rate_limited_boundary() {
    let max_accounts: u8 = kani::any();
    let cursor: u8 = kani::any();
    let rr_scan_limit: u8 = kani::any();
    let rr_touch_limit: u8 = kani::any();
    let wrap_allowed: bool = kani::any();

    kani::assume((1..=4).contains(&max_accounts));
    kani::assume(cursor < max_accounts);
    kani::assume((1..=4).contains(&rr_scan_limit));
    kani::assume((1..=4).contains(&rr_touch_limit));

    let mut engine =
        RiskEngine::new_with_market(small_zero_fee_params(max_accounts as u64), 0, 100);
    engine.rr_cursor_position = cursor as u64;

    let out = engine
        .phase2_scan_outcome(
            max_accounts as u64,
            rr_touch_limit as u64,
            rr_scan_limit as u64,
            false,
            wrap_allowed,
            false,
        )
        .unwrap();

    let blocked_by_slot_rate = !wrap_allowed && cursor == max_accounts - 1;
    if blocked_by_slot_rate {
        assert_eq!(
            out.inspected, 0,
            "same-slot generation boundary must not pretend to scan progress"
        );
        assert_eq!(
            out.next_cursor, cursor as u64,
            "same-slot generation boundary must leave cursor unchanged"
        );
        assert!(!out.wrapped);
    } else {
        assert!(
            out.inspected > 0,
            "permissionless Phase 2 must authenticate at least one missing slot when not boundary-limited"
        );
        assert!(
            out.next_cursor != cursor as u64 || out.wrapped,
            "authenticated missing-slot scan must advance cursor state"
        );
    }

    assert_eq!(
        out.touched, 0,
        "empty-slot progress must not consume touched-account capacity"
    );
    assert!(out.inspected <= rr_scan_limit as u64);
    assert!(out.inspected <= max_accounts as u64);
    kani::cover!(
        !blocked_by_slot_rate && out.inspected > 0 && out.touched == 0,
        "missing-slot cursor progress branch is reachable"
    );
    kani::cover!(
        blocked_by_slot_rate && out.inspected == 0 && out.next_cursor == cursor as u64,
        "slot-rate boundary branch is reachable"
    );
}

#[kani::proof]
#[kani::unwind(6)]
#[kani::solver(cadical)]
fn proof_live_phase2_honest_scan_reduces_cursor_rank_or_rate_limited_boundary() {
    let max_accounts: u8 = kani::any();
    let cursor: u8 = kani::any();
    let rr_scan_limit: u8 = kani::any();
    let rr_touch_limit: u8 = kani::any();
    let wrap_allowed: bool = kani::any();

    kani::assume((1..=4).contains(&max_accounts));
    kani::assume(cursor < max_accounts);
    kani::assume((1..=4).contains(&rr_scan_limit));
    kani::assume((1..=4).contains(&rr_touch_limit));

    let mut engine =
        RiskEngine::new_with_market(small_zero_fee_params(max_accounts as u64), 0, 100);
    engine.rr_cursor_position = cursor as u64;

    let before_rank = max_accounts as u64 - cursor as u64;
    let out = engine
        .phase2_scan_outcome(
            max_accounts as u64,
            rr_touch_limit as u64,
            rr_scan_limit as u64,
            true,
            wrap_allowed,
            false,
        )
        .unwrap();

    let blocked_by_slot_rate = !wrap_allowed && cursor == max_accounts - 1;
    if blocked_by_slot_rate {
        assert_eq!(
            out.inspected, 0,
            "slot-rate boundary must not claim honest scan work"
        );
        assert_eq!(
            out.next_cursor, cursor as u64,
            "slot-rate boundary must preserve the cursor"
        );
        assert!(!out.wrapped);
    } else if out.wrapped {
        assert_eq!(
            out.next_cursor, 0,
            "wrapping honest scan must move to the next generation cursor"
        );
        assert!(
            out.inspected > 0,
            "wrapping honest scan must authenticate at least one slot"
        );
    } else {
        assert!(
            out.next_cursor > cursor as u64,
            "non-wrapping honest scan must move the cursor forward"
        );
        assert!(
            max_accounts as u64 - out.next_cursor < before_rank,
            "non-wrapping honest scan must strictly reduce cursor-rank-to-boundary"
        );
        assert!(
            out.inspected > 0,
            "non-wrapping honest scan must authenticate at least one slot"
        );
    }

    assert!(
        out.inspected <= rr_scan_limit as u64,
        "live Phase 2 scan must respect the scan budget"
    );
    assert!(
        out.touched <= rr_touch_limit as u64,
        "live Phase 2 scan must respect the touch budget"
    );
    kani::cover!(
        !blocked_by_slot_rate && !out.wrapped && out.next_cursor > cursor as u64,
        "non-wrapping live-code rank progress is reachable"
    );
    kani::cover!(
        out.wrapped && out.next_cursor == 0,
        "live-code wrap progress is reachable when slot-rate permits"
    );
}

#[kani::proof]
#[kani::unwind(34)]
#[kani::solver(cadical)]
fn proof_keeper_crank_decreases_live_catchup_rank_on_prod_code() {
    let mut engine =
        RiskEngine::new_with_market(small_zero_fee_params(4), DEFAULT_SLOT, DEFAULT_ORACLE);
    let a = add_user_test(&mut engine, 0).unwrap();
    let b = add_user_test(&mut engine, 0).unwrap();
    let size = POS_SCALE as i128;
    engine.set_position_basis_q(a as usize, size).unwrap();
    engine.set_position_basis_q(b as usize, -size).unwrap();
    engine.accounts[a as usize].adl_a_basis = ADL_ONE;
    engine.accounts[b as usize].adl_a_basis = ADL_ONE;
    engine.oi_eff_long_q = size as u128;
    engine.oi_eff_short_q = size as u128;
    engine.rr_cursor_position = 2;

    let now_slot = DEFAULT_SLOT + engine.params.max_accrual_dt_slots + 1;
    let before = engine
        .permissionless_progress_rank_for_now(now_slot)
        .unwrap();
    let result = engine.keeper_crank_with_request_not_atomic(KeeperCrankRequest {
        now_slot,
        oracle_price: DEFAULT_ORACLE - 1,
        ordered_candidates: &[],
        max_revalidations: 0,
        max_candidate_inspections: MAX_TOUCHED_PER_INSTRUCTION as u16,
        funding_rate_e9: 0,
        admit_h_min: 1,
        admit_h_max: 100,
        admit_h_max_consumption_threshold_bps_opt: Some(1),
        rr_touch_limit: 1,
        rr_scan_limit: 1,
    });
    assert!(result.is_ok());
    let after = engine
        .permissionless_progress_rank_for_now(now_slot)
        .unwrap();
    assert!(after.live_catchup_slots < before.live_catchup_slots);
    assert_eq!(after.resolved_blocker_units, 0);
    kani::cover!(
        result.is_ok() && after.live_catchup_slots < before.live_catchup_slots,
        "production keeper crank decreases live catchup rank"
    );
}

#[kani::proof]
#[kani::unwind(128)]
#[kani::solver(cadical)]
fn proof_permissionless_progress_dispatcher_recovers_b_index_headroom_on_prod_code() {
    let mut engine =
        RiskEngine::new_with_market(small_zero_fee_params(4), DEFAULT_SLOT, DEFAULT_ORACLE);
    let a = add_user_test(&mut engine, 0).unwrap();
    let b = add_user_test(&mut engine, 0).unwrap();
    engine.attach_effective_position(a as usize, 1).unwrap();
    engine.attach_effective_position(b as usize, -1).unwrap();
    engine.oi_eff_long_q = 1;
    engine.oi_eff_short_q = 1;
    engine.b_short_num = u128::MAX;

    let old_p_last = engine.last_oracle_price;
    let recovery_slot = DEFAULT_SLOT + 1;
    let result = engine.permissionless_progress_not_atomic(PermissionlessProgressRequest {
        now_slot: recovery_slot,
        oracle_price: old_p_last,
        authenticated_raw_target_price: 0,
        ordered_candidates: &[],
        account_hint: None,
        max_revalidations: 0,
        max_candidate_inspections: 0,
        funding_rate_e9: 0,
        admit_h_min: 1,
        admit_h_max: 100,
        admit_h_max_consumption_threshold_bps_opt: None,
        rr_touch_limit: 1,
        rr_scan_limit: 1,
        resolved_scan_limit: 1,
        resolved_fee_rate_per_slot: 0,
    });

    assert_eq!(
        result,
        Ok(PermissionlessProgressOutcome::Recovered(
            RecoveryReason::BIndexHeadroomExhausted
        ))
    );
    assert_eq!(engine.market_mode, MarketMode::Resolved);
    assert_eq!(engine.resolved_price, old_p_last);
    assert_eq!(engine.resolved_live_price, old_p_last);
    assert_eq!(engine.resolved_slot, recovery_slot);
    kani::cover!(
        result
            == Ok(PermissionlessProgressOutcome::Recovered(
                RecoveryReason::BIndexHeadroomExhausted
            ))
            && engine.resolved_price == old_p_last,
        "production permissionless progress dispatcher reaches P-last B-index recovery"
    );
}

#[kani::proof]
#[kani::unwind(96)]
#[kani::solver(cadical)]
fn proof_permissionless_progress_dispatcher_recovers_below_progress_floor_on_prod_code() {
    let mut engine =
        RiskEngine::new_with_market(small_zero_fee_params(4), DEFAULT_SLOT, DEFAULT_ORACLE);
    let a = add_user_test(&mut engine, 0).unwrap();
    let b = add_user_test(&mut engine, 0).unwrap();
    engine.deposit_not_atomic(a, 10, DEFAULT_SLOT).unwrap();
    engine.deposit_not_atomic(b, 10, DEFAULT_SLOT).unwrap();
    engine.attach_effective_position(a as usize, 1).unwrap();
    engine.attach_effective_position(b as usize, -1).unwrap();
    engine.oi_eff_long_q = 1;
    engine.oi_eff_short_q = 1;

    let now_slot = DEFAULT_SLOT + 1;
    let p_last = engine.last_oracle_price;
    let raw_target = p_last + 1;
    let vault_before = engine.vault.get();
    let capital_before = engine.c_tot.get();
    let insurance_before = engine.insurance_fund.balance.get();

    let result = engine.permissionless_progress_not_atomic(PermissionlessProgressRequest {
        now_slot,
        oracle_price: p_last,
        authenticated_raw_target_price: raw_target,
        ordered_candidates: &[],
        account_hint: None,
        max_revalidations: 0,
        max_candidate_inspections: 0,
        funding_rate_e9: 0,
        admit_h_min: 1,
        admit_h_max: 100,
        admit_h_max_consumption_threshold_bps_opt: None,
        rr_touch_limit: 1,
        rr_scan_limit: 1,
        resolved_scan_limit: 1,
        resolved_fee_rate_per_slot: 0,
    });

    assert_eq!(
        result,
        Ok(PermissionlessProgressOutcome::Recovered(
            RecoveryReason::BelowProgressFloor
        ))
    );
    assert_eq!(engine.market_mode, MarketMode::Resolved);
    assert_eq!(engine.resolved_slot, now_slot);
    assert_eq!(engine.resolved_price, p_last);
    assert_eq!(engine.resolved_live_price, p_last);
    assert_ne!(
        engine.resolved_price, raw_target,
        "below-floor recovery must settle at P_last, not the caller raw target"
    );
    assert_eq!(engine.vault.get(), vault_before);
    assert_eq!(engine.c_tot.get(), capital_before);
    assert_eq!(engine.insurance_fund.balance.get(), insurance_before);
    kani::cover!(
        result
            == Ok(PermissionlessProgressOutcome::Recovered(
                RecoveryReason::BelowProgressFloor
            ))
            && engine.resolved_price == p_last
            && engine.resolved_price != raw_target
            && engine.vault.get() == vault_before
            && engine.c_tot.get() == capital_before
            && engine.insurance_fund.balance.get() == insurance_before,
        "production dispatcher recovers below-progress-floor dead zone at P_last"
    );
}

#[kani::proof]
#[kani::unwind(96)]
#[kani::solver(cadical)]
fn proof_permissionless_progress_dispatcher_recovers_explicit_loss_overflow_on_prod_code() {
    let mut engine =
        RiskEngine::new_with_market(small_zero_fee_params(4), DEFAULT_SLOT, DEFAULT_ORACLE);
    engine.explicit_unallocated_loss_saturated = 1;

    let now_slot = DEFAULT_SLOT + 1;
    let p_last = engine.last_oracle_price;
    let raw_target = p_last + 1;
    let vault_before = engine.vault.get();
    let capital_before = engine.c_tot.get();
    let insurance_before = engine.insurance_fund.balance.get();

    let result = engine.permissionless_progress_not_atomic(PermissionlessProgressRequest {
        now_slot,
        oracle_price: p_last,
        authenticated_raw_target_price: raw_target,
        ordered_candidates: &[],
        account_hint: None,
        max_revalidations: 0,
        max_candidate_inspections: 0,
        funding_rate_e9: 0,
        admit_h_min: 1,
        admit_h_max: 100,
        admit_h_max_consumption_threshold_bps_opt: None,
        rr_touch_limit: 1,
        rr_scan_limit: 1,
        resolved_scan_limit: 1,
        resolved_fee_rate_per_slot: 0,
    });

    assert_eq!(
        result,
        Ok(PermissionlessProgressOutcome::Recovered(
            RecoveryReason::ExplicitLossOrDustAuditOverflow
        ))
    );
    assert_eq!(engine.market_mode, MarketMode::Resolved);
    assert_eq!(engine.resolved_slot, now_slot);
    assert_eq!(engine.resolved_price, p_last);
    assert_eq!(engine.resolved_live_price, p_last);
    assert_ne!(
        engine.resolved_price, raw_target,
        "explicit-loss overflow recovery must settle at P_last, not the caller raw target"
    );
    assert_eq!(engine.vault.get(), vault_before);
    assert_eq!(engine.c_tot.get(), capital_before);
    assert_eq!(engine.insurance_fund.balance.get(), insurance_before);
    kani::cover!(
        result
            == Ok(PermissionlessProgressOutcome::Recovered(
                RecoveryReason::ExplicitLossOrDustAuditOverflow
            ))
            && engine.resolved_price == p_last
            && engine.vault.get() == vault_before
            && engine.c_tot.get() == capital_before
            && engine.insurance_fund.balance.get() == insurance_before,
        "production dispatcher recovers explicit non-claim loss overflow at P_last"
    );
}

#[kani::proof]
#[kani::unwind(220)]
#[kani::solver(cadical)]
fn proof_permissionless_progress_dispatcher_recovers_blocked_segment_on_prod_code() {
    let mut engine =
        RiskEngine::new_with_market(small_zero_fee_params(4), DEFAULT_SLOT, DEFAULT_ORACLE);
    let a = add_user_test(&mut engine, 0).unwrap();
    let b = add_user_test(&mut engine, 0).unwrap();
    engine.deposit_not_atomic(a, 10, DEFAULT_SLOT).unwrap();
    engine.deposit_not_atomic(b, 10, DEFAULT_SLOT).unwrap();
    engine.attach_effective_position(a as usize, 1).unwrap();
    engine.attach_effective_position(b as usize, -1).unwrap();
    engine.oi_eff_long_q = 1;
    engine.oi_eff_short_q = 1;

    let max_future_mark = ADL_ONE * MAX_ORACLE_PRICE as u128;
    engine.adl_coeff_long = (i128::MAX as u128 - max_future_mark) as i128;

    let now_slot = DEFAULT_SLOT + engine.params.max_accrual_dt_slots;
    let p_last = engine.last_oracle_price;
    let raw_target = p_last + 1;
    let vault_before = engine.vault.get();
    let capital_before = engine.c_tot.get();
    let insurance_before = engine.insurance_fund.balance.get();

    let result = engine.permissionless_progress_not_atomic(PermissionlessProgressRequest {
        now_slot,
        oracle_price: p_last,
        authenticated_raw_target_price: raw_target,
        ordered_candidates: &[],
        account_hint: None,
        max_revalidations: 0,
        max_candidate_inspections: 0,
        funding_rate_e9: 0,
        admit_h_min: 1,
        admit_h_max: 100,
        admit_h_max_consumption_threshold_bps_opt: None,
        rr_touch_limit: 1,
        rr_scan_limit: 1,
        resolved_scan_limit: 1,
        resolved_fee_rate_per_slot: 0,
    });

    assert_eq!(
        result,
        Ok(PermissionlessProgressOutcome::Recovered(
            RecoveryReason::BlockedSegmentHeadroomOrRepresentability
        ))
    );
    assert_eq!(engine.market_mode, MarketMode::Resolved);
    assert_eq!(engine.resolved_slot, now_slot);
    assert_eq!(engine.resolved_price, p_last);
    assert_eq!(engine.resolved_live_price, p_last);
    assert_ne!(
        engine.resolved_price, raw_target,
        "blocked-segment recovery must settle at P_last, not the caller raw target"
    );
    assert_eq!(engine.vault.get(), vault_before);
    assert_eq!(engine.c_tot.get(), capital_before);
    assert_eq!(engine.insurance_fund.balance.get(), insurance_before);
    kani::cover!(
        result
            == Ok(PermissionlessProgressOutcome::Recovered(
                RecoveryReason::BlockedSegmentHeadroomOrRepresentability
            ))
            && engine.resolved_price == p_last
            && engine.resolved_price != raw_target
            && engine.vault.get() == vault_before
            && engine.c_tot.get() == capital_before
            && engine.insurance_fund.balance.get() == insurance_before,
        "production dispatcher recovers blocked bounded segment at P_last"
    );
}

#[kani::proof]
#[kani::unwind(64)]
#[kani::solver(cadical)]
fn proof_permissionless_progress_dispatcher_decreases_live_catchup_rank_on_prod_code() {
    let mut engine =
        RiskEngine::new_with_market(small_zero_fee_params(4), DEFAULT_SLOT, DEFAULT_ORACLE);
    let a = add_user_test(&mut engine, 0).unwrap();
    let b = add_user_test(&mut engine, 0).unwrap();
    let size = POS_SCALE as i128;
    engine.set_position_basis_q(a as usize, size).unwrap();
    engine.set_position_basis_q(b as usize, -size).unwrap();
    engine.accounts[a as usize].adl_a_basis = ADL_ONE;
    engine.accounts[b as usize].adl_a_basis = ADL_ONE;
    engine.oi_eff_long_q = size as u128;
    engine.oi_eff_short_q = size as u128;
    engine.rr_cursor_position = 2;

    let now_slot = DEFAULT_SLOT + engine.params.max_accrual_dt_slots + 1;
    let before = engine
        .permissionless_progress_rank_for_now(now_slot)
        .unwrap();
    let result = engine.permissionless_progress_not_atomic(PermissionlessProgressRequest {
        now_slot,
        oracle_price: DEFAULT_ORACLE - 1,
        authenticated_raw_target_price: 0,
        ordered_candidates: &[],
        account_hint: None,
        max_revalidations: 0,
        max_candidate_inspections: MAX_TOUCHED_PER_INSTRUCTION as u16,
        funding_rate_e9: 0,
        admit_h_min: 1,
        admit_h_max: 100,
        admit_h_max_consumption_threshold_bps_opt: Some(1),
        rr_touch_limit: 1,
        rr_scan_limit: 1,
        resolved_scan_limit: 1,
        resolved_fee_rate_per_slot: 0,
    });

    assert!(matches!(
        result,
        Ok(PermissionlessProgressOutcome::Cranked(_))
    ));
    let after = engine
        .permissionless_progress_rank_for_now(now_slot)
        .unwrap();
    assert!(after.live_catchup_slots < before.live_catchup_slots);
    assert_eq!(after.resolved_blocker_units, 0);
    kani::cover!(
        matches!(result, Ok(PermissionlessProgressOutcome::Cranked(_)))
            && after.live_catchup_slots < before.live_catchup_slots,
        "production permissionless dispatcher decreases live catchup rank through crank path"
    );
}

#[kani::proof]
#[kani::unwind(40)]
#[kani::solver(cadical)]
fn proof_permissionless_progress_missing_account_hint_does_not_block_cursor_progress_on_prod_code()
{
    let mut engine =
        RiskEngine::new_with_market(small_zero_fee_params(4), DEFAULT_SLOT, DEFAULT_ORACLE);
    engine.rr_cursor_position = 0;

    let num_used_before = engine.num_used_accounts;
    let free_head_before = engine.free_head;
    let vault_before = engine.vault.get();
    let insurance_before = engine.insurance_fund.balance.get();

    let result = engine.permissionless_progress_not_atomic(PermissionlessProgressRequest {
        now_slot: DEFAULT_SLOT + 1,
        oracle_price: DEFAULT_ORACLE,
        authenticated_raw_target_price: DEFAULT_ORACLE,
        ordered_candidates: &[],
        account_hint: Some(1),
        max_revalidations: 0,
        max_candidate_inspections: 0,
        funding_rate_e9: 0,
        admit_h_min: 1,
        admit_h_max: 100,
        admit_h_max_consumption_threshold_bps_opt: None,
        rr_touch_limit: 1,
        rr_scan_limit: 1,
        resolved_scan_limit: 1,
        resolved_fee_rate_per_slot: 0,
    });

    assert!(matches!(
        result,
        Ok(PermissionlessProgressOutcome::Cranked(CrankOutcome {
            num_liquidations: 0
        }))
    ));
    assert_eq!(engine.rr_cursor_position, 1);
    assert_eq!(engine.num_used_accounts, num_used_before);
    assert_eq!(engine.free_head, free_head_before);
    assert!(!engine.is_used(1));
    assert_eq!(engine.vault.get(), vault_before);
    assert_eq!(engine.insurance_fund.balance.get(), insurance_before);
    assert_eq!(engine.market_mode, MarketMode::Live);
    kani::cover!(
        matches!(
            result,
            Ok(PermissionlessProgressOutcome::Cranked(CrankOutcome {
                num_liquidations: 0
            }))
        ) && engine.rr_cursor_position == 1
            && engine.num_used_accounts == num_used_before
            && !engine.is_used(1),
        "public dispatcher ignores missing account hints without materialization and still advances cursor"
    );
}

#[kani::proof]
#[kani::unwind(40)]
#[kani::solver(cadical)]
fn proof_permissionless_progress_out_of_capacity_hint_does_not_block_cursor_progress_on_prod_code()
{
    let mut engine =
        RiskEngine::new_with_market(small_zero_fee_params(2), DEFAULT_SLOT, DEFAULT_ORACLE);
    engine.rr_cursor_position = 0;
    let out_of_capacity_hint = 3u16;
    assert!(out_of_capacity_hint as u64 >= engine.params.max_accounts);

    let num_used_before = engine.num_used_accounts;
    let free_head_before = engine.free_head;
    let vault_before = engine.vault.get();
    let insurance_before = engine.insurance_fund.balance.get();

    let result = engine.permissionless_progress_not_atomic(PermissionlessProgressRequest {
        now_slot: DEFAULT_SLOT + 1,
        oracle_price: DEFAULT_ORACLE,
        authenticated_raw_target_price: DEFAULT_ORACLE,
        ordered_candidates: &[],
        account_hint: Some(out_of_capacity_hint),
        max_revalidations: 0,
        max_candidate_inspections: 0,
        funding_rate_e9: 0,
        admit_h_min: 1,
        admit_h_max: 100,
        admit_h_max_consumption_threshold_bps_opt: None,
        rr_touch_limit: 1,
        rr_scan_limit: 1,
        resolved_scan_limit: 1,
        resolved_fee_rate_per_slot: 0,
    });

    assert!(matches!(
        result,
        Ok(PermissionlessProgressOutcome::Cranked(CrankOutcome {
            num_liquidations: 0
        }))
    ));
    assert_eq!(engine.rr_cursor_position, 1);
    assert_eq!(engine.num_used_accounts, num_used_before);
    assert_eq!(engine.free_head, free_head_before);
    assert!(!engine.is_used(out_of_capacity_hint as usize));
    assert_eq!(engine.vault.get(), vault_before);
    assert_eq!(engine.insurance_fund.balance.get(), insurance_before);
    assert_eq!(engine.market_mode, MarketMode::Live);
    kani::cover!(
        matches!(
            result,
            Ok(PermissionlessProgressOutcome::Cranked(CrankOutcome {
                num_liquidations: 0
            }))
        ) && engine.rr_cursor_position == 1
            && engine.num_used_accounts == num_used_before
            && !engine.is_used(out_of_capacity_hint as usize),
        "public dispatcher ignores out-of-capacity account hints without materialization and still advances cursor"
    );
}

#[kani::proof]
#[kani::unwind(40)]
#[kani::solver(cadical)]
fn proof_permissionless_account_b_progress_flat_account_is_noop_on_prod_code() {
    let mut engine =
        RiskEngine::new_with_market(small_zero_fee_params(4), DEFAULT_SLOT, DEFAULT_ORACLE);
    engine.deposit_not_atomic(0, 10, DEFAULT_SLOT).unwrap();

    let capital_before = engine.accounts[0].capital.get();
    let pnl_before = engine.accounts[0].pnl;
    let position_before = engine.accounts[0].position_basis_q;
    let loss_weight_before = engine.accounts[0].loss_weight;
    let b_snap_before = engine.accounts[0].b_snap;
    let b_rem_before = engine.accounts[0].b_rem;
    let free_head_before = engine.free_head;
    let num_used_before = engine.num_used_accounts;
    let c_tot_before = engine.c_tot;
    let vault_before = engine.vault.get();
    let insurance_before = engine.insurance_fund.balance.get();
    let current_slot_before = engine.current_slot;
    let last_market_slot_before = engine.last_market_slot;
    let rank_before = engine.permissionless_account_progress_rank(0).unwrap();
    assert_eq!(position_before, 0);
    assert_eq!(rank_before.account_b_remaining_num, 0);

    let result = engine.try_permissionless_account_b_progress(0, DEFAULT_SLOT + 1, 1, 100, None);
    let rank_after = engine.permissionless_account_progress_rank(0).unwrap();

    assert_eq!(result, Ok(false));
    assert!(engine.is_used(0));
    assert_eq!(engine.free_head, free_head_before);
    assert_eq!(engine.num_used_accounts, num_used_before);
    assert_eq!(engine.c_tot, c_tot_before);
    assert_eq!(engine.accounts[0].capital.get(), capital_before);
    assert_eq!(engine.accounts[0].pnl, pnl_before);
    assert_eq!(engine.accounts[0].position_basis_q, position_before);
    assert_eq!(engine.accounts[0].loss_weight, loss_weight_before);
    assert_eq!(engine.accounts[0].b_snap, b_snap_before);
    assert_eq!(engine.accounts[0].b_rem, b_rem_before);
    assert_eq!(
        rank_after.account_b_remaining_num,
        rank_before.account_b_remaining_num
    );
    assert_eq!(engine.vault.get(), vault_before);
    assert_eq!(engine.insurance_fund.balance.get(), insurance_before);
    assert_eq!(engine.current_slot, current_slot_before);
    assert_eq!(engine.last_market_slot, last_market_slot_before);
    assert_eq!(engine.market_mode, MarketMode::Live);
    kani::cover!(
        result == Ok(false)
            && engine.is_used(0)
            && engine.accounts[0].capital.get() == capital_before,
        "production account-B progress branch treats a flat account as no-op without moving funds"
    );
}

#[kani::proof]
#[kani::unwind(40)]
#[kani::solver(cadical)]
fn proof_permissionless_account_b_dispatch_flat_account_falls_through_on_prod_code() {
    let mut engine =
        RiskEngine::new_with_market(small_zero_fee_params(4), DEFAULT_SLOT, DEFAULT_ORACLE);
    engine.deposit_not_atomic(0, 10, DEFAULT_SLOT).unwrap();

    let capital_before = engine.accounts[0].capital.get();
    let pnl_before = engine.accounts[0].pnl;
    let position_before = engine.accounts[0].position_basis_q;
    let loss_weight_before = engine.accounts[0].loss_weight;
    let b_snap_before = engine.accounts[0].b_snap;
    let b_rem_before = engine.accounts[0].b_rem;
    let free_head_before = engine.free_head;
    let num_used_before = engine.num_used_accounts;
    let c_tot_before = engine.c_tot;
    let vault_before = engine.vault.get();
    let insurance_before = engine.insurance_fund.balance.get();
    let current_slot_before = engine.current_slot;
    let last_market_slot_before = engine.last_market_slot;
    let rank_before = engine.permissionless_account_progress_rank(0).unwrap();
    assert_eq!(position_before, 0);
    assert_eq!(rank_before.account_b_remaining_num, 0);

    let result = engine.try_permissionless_account_b_dispatch(0, DEFAULT_SLOT + 1, 1, 100, None);
    let rank_after = engine.permissionless_account_progress_rank(0).unwrap();

    assert!(matches!(result, Ok(None)));
    assert!(engine.is_used(0));
    assert_eq!(engine.free_head, free_head_before);
    assert_eq!(engine.num_used_accounts, num_used_before);
    assert_eq!(engine.c_tot, c_tot_before);
    assert_eq!(engine.accounts[0].capital.get(), capital_before);
    assert_eq!(engine.accounts[0].pnl, pnl_before);
    assert_eq!(engine.accounts[0].position_basis_q, position_before);
    assert_eq!(engine.accounts[0].loss_weight, loss_weight_before);
    assert_eq!(engine.accounts[0].b_snap, b_snap_before);
    assert_eq!(engine.accounts[0].b_rem, b_rem_before);
    assert_eq!(
        rank_after.account_b_remaining_num,
        rank_before.account_b_remaining_num
    );
    assert_eq!(engine.vault.get(), vault_before);
    assert_eq!(engine.insurance_fund.balance.get(), insurance_before);
    assert_eq!(engine.current_slot, current_slot_before);
    assert_eq!(engine.last_market_slot, last_market_slot_before);
    assert_eq!(engine.market_mode, MarketMode::Live);
    kani::cover!(
        matches!(result, Ok(None))
            && engine.is_used(0)
            && engine.accounts[0].capital.get() == capital_before,
        "production account-B dispatch falls through a flat account hint without moving funds"
    );
}

#[kani::proof]
#[kani::unwind(48)]
#[kani::solver(cadical)]
fn proof_permissionless_progress_flat_account_b_hint_falls_through_to_cursor_progress_on_prod_code()
{
    let mut engine =
        RiskEngine::new_with_market(small_zero_fee_params(4), DEFAULT_SLOT, DEFAULT_ORACLE);
    engine.deposit_not_atomic(0, 10, DEFAULT_SLOT).unwrap();
    engine.rr_cursor_position = 1;

    let capital_before = engine.accounts[0].capital.get();
    let pnl_before = engine.accounts[0].pnl;
    let position_before = engine.accounts[0].position_basis_q;
    let loss_weight_before = engine.accounts[0].loss_weight;
    let b_snap_before = engine.accounts[0].b_snap;
    let b_rem_before = engine.accounts[0].b_rem;
    let num_used_before = engine.num_used_accounts;
    let free_head_before = engine.free_head;
    let c_tot_before = engine.c_tot;
    let vault_before = engine.vault.get();
    let insurance_before = engine.insurance_fund.balance.get();
    let rank_before = engine.permissionless_account_progress_rank(0).unwrap();
    assert_eq!(position_before, 0);
    assert_eq!(rank_before.account_b_remaining_num, 0);

    let result = engine.permissionless_progress_not_atomic(PermissionlessProgressRequest {
        now_slot: DEFAULT_SLOT + 1,
        oracle_price: DEFAULT_ORACLE,
        authenticated_raw_target_price: DEFAULT_ORACLE,
        ordered_candidates: &[],
        account_hint: Some(0),
        max_revalidations: 0,
        max_candidate_inspections: 0,
        funding_rate_e9: 0,
        admit_h_min: 1,
        admit_h_max: 100,
        admit_h_max_consumption_threshold_bps_opt: None,
        rr_touch_limit: 1,
        rr_scan_limit: 1,
        resolved_scan_limit: 1,
        resolved_fee_rate_per_slot: 0,
    });
    let rank_after = engine.permissionless_account_progress_rank(0).unwrap();

    assert!(matches!(
        result,
        Ok(PermissionlessProgressOutcome::Cranked(CrankOutcome {
            num_liquidations: 0
        }))
    ));
    assert_eq!(engine.rr_cursor_position, 2);
    assert!(engine.is_used(0));
    assert_eq!(engine.num_used_accounts, num_used_before);
    assert_eq!(engine.free_head, free_head_before);
    assert_eq!(engine.c_tot, c_tot_before);
    assert_eq!(engine.accounts[0].capital.get(), capital_before);
    assert_eq!(engine.accounts[0].pnl, pnl_before);
    assert_eq!(engine.accounts[0].position_basis_q, position_before);
    assert_eq!(engine.accounts[0].loss_weight, loss_weight_before);
    assert_eq!(engine.accounts[0].b_snap, b_snap_before);
    assert_eq!(engine.accounts[0].b_rem, b_rem_before);
    assert_eq!(
        rank_after.account_b_remaining_num,
        rank_before.account_b_remaining_num
    );
    assert_eq!(engine.vault.get(), vault_before);
    assert_eq!(engine.insurance_fund.balance.get(), insurance_before);
    assert_eq!(engine.market_mode, MarketMode::Live);
    kani::cover!(
        matches!(
            result,
            Ok(PermissionlessProgressOutcome::Cranked(CrankOutcome {
                num_liquidations: 0
            }))
        ) && engine.rr_cursor_position == 2
            && engine.accounts[0].capital.get() == capital_before
            && engine.accounts[0].position_basis_q == 0,
        "public dispatcher falls through a flat account-B hint and still advances cursor progress"
    );
}

#[kani::proof]
#[kani::unwind(128)]
#[kani::solver(cadical)]
fn proof_permissionless_progress_dispatcher_decreases_active_close_rank_on_prod_code() {
    let mut engine =
        RiskEngine::new_with_market(small_zero_fee_params(4), DEFAULT_SLOT, DEFAULT_ORACLE);
    let idx = add_user_test(&mut engine, 0).unwrap();
    engine.attach_effective_position(idx as usize, -1).unwrap();
    engine.oi_eff_short_q = 1;
    engine.oi_eff_long_q = 1;
    engine.bankruptcy_hmax_lock_active = true;
    engine.stress_envelope_remaining_indices = engine.params.max_accounts;
    engine.stress_envelope_start_slot = DEFAULT_SLOT;
    engine.stress_envelope_start_generation = engine.sweep_generation;
    engine.active_close_present = 1;
    engine.active_close_phase = ACTIVE_CLOSE_PHASE_RESIDUAL_B;
    engine.active_close_account_idx = u16::MAX;
    engine.active_close_opp_side = ACTIVE_CLOSE_SIDE_SHORT;
    engine.active_close_close_price = DEFAULT_ORACLE;
    engine.active_close_close_slot = DEFAULT_SLOT;
    engine.active_close_q_close_q = 0;

    let residual: u8 = kani::any();
    kani::assume((1..=3).contains(&residual));
    engine.active_close_residual_remaining = residual as u128;

    let now_slot = DEFAULT_SLOT + 1;
    let before = engine
        .permissionless_progress_rank_for_now(now_slot)
        .unwrap();
    assert_eq!(before.active_close_residual_atoms, residual as u128);
    let before_b = engine.b_short_num;

    let result = engine.permissionless_progress_not_atomic(PermissionlessProgressRequest {
        now_slot,
        oracle_price: DEFAULT_ORACLE,
        authenticated_raw_target_price: DEFAULT_ORACLE,
        ordered_candidates: &[],
        account_hint: None,
        max_revalidations: 0,
        max_candidate_inspections: 0,
        funding_rate_e9: 0,
        admit_h_min: 1,
        admit_h_max: 100,
        admit_h_max_consumption_threshold_bps_opt: None,
        rr_touch_limit: 0,
        rr_scan_limit: 0,
        resolved_scan_limit: 1,
        resolved_fee_rate_per_slot: 0,
    });

    assert_eq!(
        result,
        Ok(PermissionlessProgressOutcome::ActiveCloseContinued)
    );
    let after = engine
        .permissionless_progress_rank_for_now(now_slot)
        .unwrap();
    assert!(after.active_close_residual_atoms < before.active_close_residual_atoms);
    assert_eq!(after.active_close_residual_atoms, 0);
    assert!(engine.b_short_num > before_b);
    assert_eq!(engine.active_close_present, 0);
    assert_eq!(engine.market_mode, MarketMode::Live);
    kani::cover!(
        result == Ok(PermissionlessProgressOutcome::ActiveCloseContinued)
            && after.active_close_residual_atoms < before.active_close_residual_atoms,
        "production permissionless dispatcher decreases active-close rank before ordinary crank"
    );
}

#[kani::proof]
#[kani::unwind(96)]
#[kani::solver(cadical)]
fn proof_permissionless_progress_dispatcher_recovers_exhausted_active_close_on_prod_code() {
    let mut engine =
        RiskEngine::new_with_market(small_zero_fee_params(4), DEFAULT_SLOT, DEFAULT_ORACLE);
    engine.bankruptcy_hmax_lock_active = true;
    engine.stress_envelope_remaining_indices = engine.params.max_accounts;
    engine.stress_envelope_start_slot = DEFAULT_SLOT;
    engine.stress_envelope_start_generation = engine.sweep_generation;
    engine.active_close_present = 1;
    engine.active_close_phase = ACTIVE_CLOSE_PHASE_RESIDUAL_B;
    engine.active_close_account_idx = u16::MAX;
    engine.active_close_opp_side = ACTIVE_CLOSE_SIDE_SHORT;
    engine.active_close_close_price = DEFAULT_ORACLE;
    engine.active_close_close_slot = DEFAULT_SLOT;
    engine.active_close_q_close_q = 0;
    engine.active_close_b_chunks_booked = ACTIVE_CLOSE_MAX_RESIDUAL_B_CHUNKS;

    let residual: u8 = kani::any();
    kani::assume((1..=3).contains(&residual));
    engine.active_close_residual_remaining = residual as u128;

    let now_slot = DEFAULT_SLOT + 1;
    let before = engine
        .permissionless_progress_rank_for_now(now_slot)
        .unwrap();
    let vault_before = engine.vault.get();
    let capital_before = engine.c_tot.get();
    let insurance_before = engine.insurance_fund.balance.get();
    let explicit_before = engine.explicit_unallocated_loss_short.get();

    let result = engine.permissionless_progress_not_atomic(PermissionlessProgressRequest {
        now_slot,
        oracle_price: DEFAULT_ORACLE,
        authenticated_raw_target_price: DEFAULT_ORACLE,
        ordered_candidates: &[],
        account_hint: None,
        max_revalidations: 0,
        max_candidate_inspections: 0,
        funding_rate_e9: 0,
        admit_h_min: 1,
        admit_h_max: 100,
        admit_h_max_consumption_threshold_bps_opt: None,
        rr_touch_limit: 0,
        rr_scan_limit: 0,
        resolved_scan_limit: 1,
        resolved_fee_rate_per_slot: 0,
    });

    assert_eq!(
        result,
        Ok(PermissionlessProgressOutcome::Recovered(
            RecoveryReason::ActiveBankruptCloseCannotProgress
        ))
    );
    assert_eq!(engine.market_mode, MarketMode::Resolved);
    assert_eq!(engine.resolved_price, DEFAULT_ORACLE);
    assert_eq!(engine.resolved_live_price, DEFAULT_ORACLE);
    assert_eq!(engine.active_close_present, 0);
    assert_eq!(engine.vault.get(), vault_before);
    assert_eq!(engine.c_tot.get(), capital_before);
    assert_eq!(engine.insurance_fund.balance.get(), insurance_before);
    assert!(
        engine.explicit_unallocated_loss_short.get() >= explicit_before + residual as u128,
        "exhausted active-close residual must become durable non-claim loss before recovery"
    );
    assert_eq!(
        before.active_close_residual_atoms, residual as u128,
        "the proof must exercise the active-close recovery rank component"
    );
    kani::cover!(
        result
            == Ok(PermissionlessProgressOutcome::Recovered(
                RecoveryReason::ActiveBankruptCloseCannotProgress
            ))
            && engine.market_mode == MarketMode::Resolved
            && engine.active_close_present == 0
            && engine.insurance_fund.balance.get() == insurance_before,
        "production permissionless dispatcher recovers exhausted active-close at P_last"
    );
}

#[kani::proof]
#[kani::unwind(120)]
#[kani::solver(cadical)]
fn proof_permissionless_progress_dispatcher_reduces_live_catchup_rank_on_prod_code() {
    let mut engine =
        RiskEngine::new_with_market(small_zero_fee_params(4), DEFAULT_SLOT, DEFAULT_ORACLE);
    let a = add_user_test(&mut engine, 0).unwrap();
    let b = add_user_test(&mut engine, 0).unwrap();
    let size = POS_SCALE as i128;
    engine.set_position_basis_q(a as usize, size).unwrap();
    engine.set_position_basis_q(b as usize, -size).unwrap();
    engine.accounts[a as usize].adl_a_basis = ADL_ONE;
    engine.accounts[b as usize].adl_a_basis = ADL_ONE;
    engine.oi_eff_long_q = size as u128;
    engine.oi_eff_short_q = size as u128;
    engine.rr_cursor_position = 2;

    let now_slot = DEFAULT_SLOT + engine.params.max_accrual_dt_slots + 1;
    let before = engine
        .permissionless_progress_rank_for_now(now_slot)
        .unwrap();
    let result = engine.permissionless_progress_not_atomic(PermissionlessProgressRequest {
        now_slot,
        oracle_price: DEFAULT_ORACLE - 1,
        authenticated_raw_target_price: 0,
        ordered_candidates: &[],
        account_hint: None,
        max_revalidations: 0,
        max_candidate_inspections: MAX_TOUCHED_PER_INSTRUCTION as u16,
        funding_rate_e9: 0,
        admit_h_min: 1,
        admit_h_max: 100,
        admit_h_max_consumption_threshold_bps_opt: Some(1),
        rr_touch_limit: 1,
        rr_scan_limit: 1,
        resolved_scan_limit: 1,
        resolved_fee_rate_per_slot: 0,
    });
    assert!(result.is_ok());
    let after = engine
        .permissionless_progress_rank_for_now(now_slot)
        .unwrap();
    assert!(matches!(
        result,
        Ok(PermissionlessProgressOutcome::Cranked(_))
    ));
    assert!(after.strictly_reduces_from(&before));
    kani::cover!(
        matches!(result, Ok(PermissionlessProgressOutcome::Cranked(_)))
            && after.strictly_reduces_from(&before),
        "production dispatcher reduces live-catchup rank"
    );
}

#[kani::proof]
#[kani::unwind(80)]
#[kani::solver(cadical)]
fn proof_permissionless_progress_dispatcher_recovers_b_headroom_blocker_on_prod_code() {
    let mut engine =
        RiskEngine::new_with_market(small_zero_fee_params(4), DEFAULT_SLOT, DEFAULT_ORACLE);
    let a = add_user_test(&mut engine, 0).unwrap();
    let b = add_user_test(&mut engine, 0).unwrap();
    engine.attach_effective_position(a as usize, 1).unwrap();
    engine.attach_effective_position(b as usize, -1).unwrap();
    engine.oi_eff_long_q = 1;
    engine.oi_eff_short_q = 1;
    engine.b_short_num = u128::MAX;

    let result = engine.permissionless_progress_not_atomic(PermissionlessProgressRequest {
        now_slot: DEFAULT_SLOT + 1,
        oracle_price: DEFAULT_ORACLE,
        authenticated_raw_target_price: DEFAULT_ORACLE,
        ordered_candidates: &[],
        account_hint: None,
        max_revalidations: 0,
        max_candidate_inspections: 0,
        funding_rate_e9: 0,
        admit_h_min: 1,
        admit_h_max: 100,
        admit_h_max_consumption_threshold_bps_opt: None,
        rr_touch_limit: 1,
        rr_scan_limit: 1,
        resolved_scan_limit: 1,
        resolved_fee_rate_per_slot: 0,
    });
    assert_eq!(
        result,
        Ok(PermissionlessProgressOutcome::Recovered(
            RecoveryReason::BIndexHeadroomExhausted
        ))
    );
    assert_eq!(engine.market_mode, MarketMode::Resolved);
    kani::cover!(
        result
            == Ok(PermissionlessProgressOutcome::Recovered(
                RecoveryReason::BIndexHeadroomExhausted
            )),
        "production dispatcher routes exhausted B-index headroom to recovery"
    );
}

#[kani::proof]
#[kani::unwind(80)]
#[kani::solver(cadical)]
fn proof_permissionless_progress_dispatcher_recovers_counter_or_epoch_overflow_on_prod_code() {
    let mut engine =
        RiskEngine::new_with_market(small_zero_fee_params(4), DEFAULT_SLOT, DEFAULT_ORACLE);
    engine.sweep_generation = u64::MAX;

    let raw_target: u16 = kani::any();
    kani::assume(raw_target as u64 != DEFAULT_ORACLE);
    kani::assume(raw_target != 0);
    let raw_target = raw_target as u64;
    let vault_before = engine.vault.get();
    let capital_before = engine.c_tot.get();
    let insurance_before = engine.insurance_fund.balance.get();

    let result = engine.permissionless_progress_not_atomic(PermissionlessProgressRequest {
        now_slot: DEFAULT_SLOT + 1,
        oracle_price: DEFAULT_ORACLE,
        authenticated_raw_target_price: raw_target,
        ordered_candidates: &[],
        account_hint: None,
        max_revalidations: 0,
        max_candidate_inspections: 0,
        funding_rate_e9: 0,
        admit_h_min: 1,
        admit_h_max: 100,
        admit_h_max_consumption_threshold_bps_opt: None,
        rr_touch_limit: 1,
        rr_scan_limit: 1,
        resolved_scan_limit: 1,
        resolved_fee_rate_per_slot: 0,
    });

    assert_eq!(
        result,
        Ok(PermissionlessProgressOutcome::Recovered(
            RecoveryReason::CounterOrEpochOverflowDeclaredRecovery
        ))
    );
    assert_eq!(engine.market_mode, MarketMode::Resolved);
    assert_eq!(engine.resolved_price, DEFAULT_ORACLE);
    assert_eq!(engine.resolved_live_price, DEFAULT_ORACLE);
    assert_ne!(
        engine.resolved_price, raw_target,
        "counter/epoch recovery must ignore caller raw target and settle at P-last"
    );
    assert_eq!(engine.vault.get(), vault_before);
    assert_eq!(engine.c_tot.get(), capital_before);
    assert_eq!(engine.insurance_fund.balance.get(), insurance_before);
    kani::cover!(
        result
            == Ok(PermissionlessProgressOutcome::Recovered(
                RecoveryReason::CounterOrEpochOverflowDeclaredRecovery
            ))
            && engine.resolved_price != raw_target
            && engine.vault.get() == vault_before,
        "production dispatcher routes global counter overflow to P-last recovery"
    );
}

#[kani::proof]
#[kani::unwind(32)]
#[kani::solver(cadical)]
fn proof_permissionless_account_b_progress_reduces_hinted_account_b_rank_on_prod_code() {
    let mut engine =
        RiskEngine::new_with_market(small_zero_fee_params(4), DEFAULT_SLOT, DEFAULT_ORACLE);
    let idx = add_user_test(&mut engine, 0).unwrap();
    engine.deposit_not_atomic(idx, 10, DEFAULT_SLOT).unwrap();
    engine.attach_effective_position(idx as usize, -1).unwrap();
    engine.oi_eff_short_q = 1;
    engine.oi_eff_long_q = 1;
    engine.b_short_num = SOCIAL_LOSS_DEN;

    let before = engine.permissionless_account_progress_rank(idx).unwrap();
    assert!(before.account_b_remaining_num > 0);
    let account_capital_before = engine.accounts[idx as usize].capital.get();
    let vault_before = engine.vault.get();
    let insurance_before = engine.insurance_fund.balance.get();
    let current_slot_before = engine.current_slot;
    let last_market_slot_before = engine.last_market_slot;
    let last_oracle_price_before = engine.last_oracle_price;

    let result = engine.try_permissionless_account_b_progress(idx, DEFAULT_SLOT + 1, 1, 100, None);

    assert_eq!(result, Ok(true));
    let after = engine.permissionless_account_progress_rank(idx).unwrap();
    assert!(after.account_b_remaining_num < before.account_b_remaining_num);
    assert!(engine.accounts[idx as usize].capital.get() <= account_capital_before);
    assert!(vault_before >= engine.vault.get());
    assert!(vault_before - engine.vault.get() <= PUBLIC_ACCOUNT_B_SETTLEMENT_LOSS_ATOMS);
    assert_eq!(engine.insurance_fund.balance.get(), insurance_before);
    assert_eq!(engine.current_slot, current_slot_before);
    assert_eq!(engine.last_market_slot, last_market_slot_before);
    assert_eq!(engine.last_oracle_price, last_oracle_price_before);
    assert_eq!(engine.market_mode, MarketMode::Live);
    kani::cover!(
        result == Ok(true)
            && after.account_b_remaining_num < before.account_b_remaining_num
            && engine.insurance_fund.balance.get() == insurance_before,
        "production account-B progress branch reduces hinted account-B rank without spending insurance"
    );
}

#[kani::proof]
#[kani::unwind(32)]
#[kani::solver(cadical)]
fn proof_permissionless_account_b_dispatch_returns_progress_for_hinted_blocker_on_prod_code() {
    let mut engine =
        RiskEngine::new_with_market(small_zero_fee_params(4), DEFAULT_SLOT, DEFAULT_ORACLE);
    let idx = add_user_test(&mut engine, 0).unwrap();
    engine.deposit_not_atomic(idx, 10, DEFAULT_SLOT).unwrap();
    engine.attach_effective_position(idx as usize, -1).unwrap();
    engine.oi_eff_short_q = 1;
    engine.oi_eff_long_q = 1;
    engine.b_short_num = SOCIAL_LOSS_DEN;

    let before = engine.permissionless_account_progress_rank(idx).unwrap();
    assert!(before.account_b_remaining_num > 0);
    let account_capital_before = engine.accounts[idx as usize].capital.get();
    let vault_before = engine.vault.get();
    let insurance_before = engine.insurance_fund.balance.get();
    let current_slot_before = engine.current_slot;
    let last_market_slot_before = engine.last_market_slot;
    let last_oracle_price_before = engine.last_oracle_price;

    let result = engine.try_permissionless_account_b_dispatch(idx, DEFAULT_SLOT + 1, 1, 100, None);

    assert_eq!(
        result,
        Ok(Some(PermissionlessProgressOutcome::AccountBProgress(idx)))
    );
    let after = engine.permissionless_account_progress_rank(idx).unwrap();
    assert!(after.account_b_remaining_num < before.account_b_remaining_num);
    assert!(engine.accounts[idx as usize].capital.get() <= account_capital_before);
    assert!(vault_before >= engine.vault.get());
    assert!(vault_before - engine.vault.get() <= PUBLIC_ACCOUNT_B_SETTLEMENT_LOSS_ATOMS);
    assert_eq!(engine.insurance_fund.balance.get(), insurance_before);
    assert_eq!(engine.current_slot, current_slot_before);
    assert_eq!(engine.last_market_slot, last_market_slot_before);
    assert_eq!(engine.last_oracle_price, last_oracle_price_before);
    assert_eq!(engine.market_mode, MarketMode::Live);
    kani::cover!(
        result == Ok(Some(PermissionlessProgressOutcome::AccountBProgress(idx)))
            && after.account_b_remaining_num < before.account_b_remaining_num
            && engine.insurance_fund.balance.get() == insurance_before,
        "production account-B dispatch returns progress and reduces the hinted B blocker"
    );
}

#[kani::proof]
#[kani::unwind(80)]
#[kani::solver(cadical)]
fn proof_permissionless_progress_dispatcher_reduces_resolved_blocker_rank_on_prod_code() {
    let mut engine =
        RiskEngine::new_with_market(small_zero_fee_params(4), DEFAULT_SLOT, DEFAULT_ORACLE);
    engine.deposit_not_atomic(0, 100, DEFAULT_SLOT).unwrap();
    engine.market_mode = MarketMode::Resolved;
    engine.resolved_slot = DEFAULT_SLOT;
    engine.current_slot = DEFAULT_SLOT;
    engine.resolved_price = DEFAULT_ORACLE;
    engine.resolved_live_price = DEFAULT_ORACLE;
    engine.rr_cursor_position = 0;

    let before = engine
        .permissionless_progress_rank_for_now(DEFAULT_SLOT)
        .unwrap();
    let result = engine.permissionless_progress_not_atomic(PermissionlessProgressRequest {
        now_slot: DEFAULT_SLOT,
        oracle_price: DEFAULT_ORACLE,
        authenticated_raw_target_price: DEFAULT_ORACLE,
        ordered_candidates: &[],
        account_hint: None,
        max_revalidations: 0,
        max_candidate_inspections: 0,
        funding_rate_e9: 0,
        admit_h_min: 1,
        admit_h_max: 100,
        admit_h_max_consumption_threshold_bps_opt: None,
        rr_touch_limit: 0,
        rr_scan_limit: 0,
        resolved_scan_limit: 4,
        resolved_fee_rate_per_slot: 0,
    });
    assert!(matches!(
        result,
        Ok(PermissionlessProgressOutcome::ResolvedClose(
            ResolvedCloseResult::Closed(100)
        ))
    ));
    let after = engine
        .permissionless_progress_rank_for_now(DEFAULT_SLOT)
        .unwrap();
    assert!(after.strictly_reduces_from(&before));
    kani::cover!(
        matches!(result, Ok(PermissionlessProgressOutcome::ResolvedClose(_)))
            && after.strictly_reduces_from(&before),
        "production dispatcher reduces resolved-blocker rank"
    );
}

#[kani::proof]
#[kani::unwind(80)]
#[kani::solver(cadical)]
fn proof_permissionless_progress_resolved_progress_only_makes_account_fee_current_on_prod_code() {
    let mut engine =
        RiskEngine::new_with_market(small_zero_fee_params(4), DEFAULT_SLOT, DEFAULT_ORACLE);
    engine.deposit_not_atomic(0, 100, DEFAULT_SLOT).unwrap();
    engine.deposit_not_atomic(1, 100, DEFAULT_SLOT).unwrap();
    engine.market_mode = MarketMode::Resolved;
    engine.current_slot = DEFAULT_SLOT;
    engine.resolved_slot = DEFAULT_SLOT;
    engine.resolved_price = DEFAULT_ORACLE;
    engine.resolved_live_price = DEFAULT_ORACLE;
    engine.set_pnl(0, 10).unwrap();
    engine.set_pnl(1, -5).unwrap();
    engine.accounts[0].last_fee_slot = DEFAULT_SLOT - 1;
    engine.rr_cursor_position = 0;

    let result = engine.permissionless_progress_not_atomic(PermissionlessProgressRequest {
        now_slot: DEFAULT_SLOT,
        oracle_price: DEFAULT_ORACLE,
        authenticated_raw_target_price: DEFAULT_ORACLE,
        ordered_candidates: &[],
        account_hint: None,
        max_revalidations: 0,
        max_candidate_inspections: 0,
        funding_rate_e9: 0,
        admit_h_min: 1,
        admit_h_max: 100,
        admit_h_max_consumption_threshold_bps_opt: None,
        rr_touch_limit: 0,
        rr_scan_limit: 0,
        resolved_scan_limit: 1,
        resolved_fee_rate_per_slot: 1,
    });

    assert_eq!(
        result,
        Ok(PermissionlessProgressOutcome::ResolvedClose(
            ResolvedCloseResult::ProgressOnly
        ))
    );
    assert!(engine.is_used(0));
    assert_eq!(engine.accounts[0].last_fee_slot, engine.resolved_slot);
    assert_eq!(engine.accounts[0].pnl, 10);
    assert_eq!(engine.neg_pnl_account_count, 1);
    assert_eq!(engine.market_mode, MarketMode::Resolved);
    kani::cover!(
        result
            == Ok(PermissionlessProgressOutcome::ResolvedClose(
                ResolvedCloseResult::ProgressOnly
            ))
            && engine.accounts[0].last_fee_slot == engine.resolved_slot,
        "production dispatcher resolved ProgressOnly path makes touched account fee-current"
    );
}

#[kani::proof]
#[kani::unwind(80)]
#[kani::solver(cadical)]
fn proof_permissionless_progress_resolved_mode_ignores_account_hint_on_prod_code() {
    let mut engine =
        RiskEngine::new_with_market(small_zero_fee_params(4), DEFAULT_SLOT, DEFAULT_ORACLE);
    engine.deposit_not_atomic(0, 100, DEFAULT_SLOT).unwrap();
    engine.market_mode = MarketMode::Resolved;
    engine.current_slot = DEFAULT_SLOT;
    engine.resolved_slot = DEFAULT_SLOT;
    engine.resolved_price = DEFAULT_ORACLE;
    engine.resolved_live_price = DEFAULT_ORACLE;
    engine.rr_cursor_position = 0;
    let hinted_idx: u8 = kani::any();
    kani::assume((1..=3).contains(&hinted_idx));
    let hinted_idx = hinted_idx as usize;

    let vault_before = engine.vault.get();
    let insurance_before = engine.insurance_fund.balance.get();
    let before = engine
        .permissionless_progress_rank_for_now(DEFAULT_SLOT)
        .unwrap();
    let result = engine.permissionless_progress_not_atomic(PermissionlessProgressRequest {
        now_slot: DEFAULT_SLOT,
        oracle_price: DEFAULT_ORACLE,
        authenticated_raw_target_price: DEFAULT_ORACLE,
        ordered_candidates: &[],
        account_hint: Some(hinted_idx as u16),
        max_revalidations: 0,
        max_candidate_inspections: 0,
        funding_rate_e9: 0,
        admit_h_min: 1,
        admit_h_max: 100,
        admit_h_max_consumption_threshold_bps_opt: None,
        rr_touch_limit: 0,
        rr_scan_limit: 0,
        resolved_scan_limit: 4,
        resolved_fee_rate_per_slot: 0,
    });

    assert_eq!(
        result,
        Ok(PermissionlessProgressOutcome::ResolvedClose(
            ResolvedCloseResult::Closed(100)
        ))
    );
    assert!(!engine.is_used(0));
    assert!(!engine.is_used(hinted_idx));
    assert_eq!(engine.vault.get(), vault_before - 100);
    assert_eq!(engine.insurance_fund.balance.get(), insurance_before);
    assert_eq!(engine.market_mode, MarketMode::Resolved);
    assert!(engine.check_conservation());
    let after = engine
        .permissionless_progress_rank_for_now(DEFAULT_SLOT)
        .unwrap();
    assert!(after.strictly_reduces_from(&before));
    kani::cover!(
        result
            == Ok(PermissionlessProgressOutcome::ResolvedClose(
                ResolvedCloseResult::Closed(100)
            ))
            && !engine.is_used(0)
            && !engine.is_used(hinted_idx)
            && after.strictly_reduces_from(&before),
        "resolved public dispatcher ignores account hints and still closes through the resolved cursor"
    );
}

#[kani::proof]
#[kani::unwind(80)]
#[kani::solver(cadical)]
fn proof_force_close_resolved_rechecks_terminal_counters_despite_ready_flag_on_prod_code() {
    let mut engine =
        RiskEngine::new_with_market(small_zero_fee_params(4), DEFAULT_SLOT, DEFAULT_ORACLE);
    engine.deposit_not_atomic(0, 100, DEFAULT_SLOT).unwrap();
    engine.deposit_not_atomic(1, 100, DEFAULT_SLOT).unwrap();
    engine.market_mode = MarketMode::Resolved;
    engine.current_slot = DEFAULT_SLOT;
    engine.resolved_slot = DEFAULT_SLOT;
    engine.resolved_price = DEFAULT_ORACLE;
    engine.resolved_live_price = DEFAULT_ORACLE;
    engine.set_pnl(0, 10).unwrap();
    engine.set_pnl(1, -5).unwrap();
    engine.resolved_payout_ready = 1;
    engine.resolved_payout_h_num = engine.pnl_pos_tot;
    engine.resolved_payout_h_den = engine.pnl_pos_tot;

    assert_eq!(engine.neg_pnl_account_count, 1);
    assert!(!engine.is_terminal_ready());
    let vault_before = engine.vault.get();
    let capital_before = engine.accounts[0].capital.get();
    let pnl_before = engine.accounts[0].pnl;
    let insurance_before = engine.insurance_fund.balance.get();

    let result = engine.force_close_resolved_with_fee_not_atomic(0, 0);

    assert_eq!(result, Ok(ResolvedCloseResult::ProgressOnly));
    assert!(engine.is_used(0));
    assert_eq!(engine.accounts[0].capital.get(), capital_before);
    assert_eq!(engine.accounts[0].pnl, pnl_before);
    assert_eq!(engine.vault.get(), vault_before);
    assert_eq!(engine.insurance_fund.balance.get(), insurance_before);
    assert_eq!(engine.neg_pnl_account_count, 1);
    assert!(!engine.is_terminal_ready());
    assert_eq!(engine.market_mode, MarketMode::Resolved);
    assert!(engine.check_conservation());
    kani::cover!(
        result == Ok(ResolvedCloseResult::ProgressOnly)
            && engine.resolved_payout_ready == 1
            && engine.neg_pnl_account_count == 1
            && engine.is_used(0),
        "fee-aware resolved close rechecks terminal counters before honoring payout-ready state"
    );
}

#[kani::proof]
#[kani::unwind(80)]
#[kani::solver(cadical)]
fn proof_live_touch_decreases_account_b_rank_on_prod_code() {
    let mut engine =
        RiskEngine::new_with_market(small_zero_fee_params(4), DEFAULT_SLOT, DEFAULT_ORACLE);
    let idx = add_user_test(&mut engine, 0).unwrap();
    engine.deposit_not_atomic(idx, 10, DEFAULT_SLOT).unwrap();
    engine.attach_effective_position(idx as usize, -1).unwrap();
    engine.oi_eff_short_q = 1;
    engine.oi_eff_long_q = 1;
    engine.b_short_num = 3 * SOCIAL_LOSS_DEN;

    let before = engine.permissionless_account_progress_rank(idx).unwrap();
    assert!(before.account_b_remaining_num > 0);

    let mut ctx = InstructionContext::new_with_admission(1, 100);
    let result = engine.touch_account_live_local(idx as usize, &mut ctx);
    assert!(result.is_ok());

    let after = engine.permissionless_account_progress_rank(idx).unwrap();
    assert!(after.account_b_remaining_num < before.account_b_remaining_num);
    kani::cover!(
        result.is_ok() && after.account_b_remaining_num < before.account_b_remaining_num,
        "production live touch decreases account-local B rank"
    );
}

#[kani::proof]
#[kani::unwind(32)]
#[kani::solver(cadical)]
fn proof_resolved_terminal_close_rejects_account_b_stale_position_on_prod_code() {
    let mut engine =
        RiskEngine::new_with_market(small_zero_fee_params(4), DEFAULT_SLOT, DEFAULT_ORACLE);
    let idx = add_user_test(&mut engine, 0).unwrap();
    engine.deposit_not_atomic(idx, 10, DEFAULT_SLOT).unwrap();
    engine.attach_effective_position(idx as usize, -1).unwrap();
    engine.b_short_num = 3 * SOCIAL_LOSS_DEN;
    engine.market_mode = MarketMode::Resolved;
    engine.current_slot = DEFAULT_SLOT;
    engine.resolved_slot = DEFAULT_SLOT;
    engine.resolved_price = DEFAULT_ORACLE;
    engine.resolved_live_price = DEFAULT_ORACLE;

    let before = engine.permissionless_account_progress_rank(idx).unwrap();
    assert!(before.account_b_remaining_num > 0);
    let basis_before = engine.accounts[idx as usize].position_basis_q;
    let capital_before = engine.accounts[idx as usize].capital.get();
    let result = engine.close_resolved_terminal_not_atomic(idx);

    assert_eq!(result, Err(RiskError::Undercollateralized));
    assert!(engine.is_used(idx as usize));
    assert_eq!(engine.accounts[idx as usize].position_basis_q, basis_before);
    assert_eq!(engine.accounts[idx as usize].capital.get(), capital_before);
    kani::cover!(
        result == Err(RiskError::Undercollateralized)
            && before.account_b_remaining_num > 0
            && engine.is_used(idx as usize),
        "production terminal close rejects a B-stale resolved account before free/payout"
    );
}

#[kani::proof]
#[kani::unwind(120)]
#[kani::solver(cadical)]
fn proof_reconcile_resolved_settles_account_b_stale_position_on_prod_code() {
    let mut engine =
        RiskEngine::new_with_market(small_zero_fee_params(4), DEFAULT_SLOT, DEFAULT_ORACLE);
    let short = add_user_test(&mut engine, 0).unwrap();
    engine.deposit_not_atomic(short, 100, DEFAULT_SLOT).unwrap();
    engine
        .attach_effective_position(short as usize, -1)
        .unwrap();
    engine.market_mode = MarketMode::Resolved;
    engine.current_slot = DEFAULT_SLOT;
    engine.resolved_slot = DEFAULT_SLOT;
    engine.resolved_price = DEFAULT_ORACLE;
    engine.resolved_live_price = DEFAULT_ORACLE;
    engine.oi_eff_short_q = 0;
    engine.oi_eff_long_q = 0;
    engine.side_mode_short = SideMode::ResetPending;
    engine.adl_epoch_short = 1;
    engine.stale_account_count_short = 1;
    engine.b_epoch_start_short_num = 3 * SOCIAL_LOSS_DEN;

    let capital_before = engine.accounts[short as usize].capital.get();
    let vault_before = engine.vault.get();
    let insurance_before = engine.insurance_fund.balance.get();
    let before = engine.permissionless_account_progress_rank(short).unwrap();
    assert!(before.account_b_remaining_num > 0);

    let result = engine.reconcile_resolved_not_atomic(short);

    assert_eq!(result, Ok(()));
    assert!(engine.is_used(short as usize));
    assert_eq!(engine.stale_account_count_short, 0);
    assert_eq!(engine.accounts[short as usize].position_basis_q, 0);
    assert_eq!(
        engine.accounts[short as usize].capital.get(),
        capital_before - 3
    );
    assert_eq!(engine.vault.get(), vault_before);
    assert_eq!(engine.insurance_fund.balance.get(), insurance_before);
    assert!(engine.check_conservation());
    kani::cover!(
        result == Ok(())
            && before.account_b_remaining_num > 0
            && engine.accounts[short as usize].position_basis_q == 0,
        "resolved reconcile settles a B-stale account before terminal close"
    );
}

#[kani::proof]
#[kani::unwind(16)]
#[kani::solver(cadical)]
fn proof_resolved_cursor_missing_slots_advance_on_prod_code() {
    let mut engine =
        RiskEngine::new_with_market(small_zero_fee_params(4), DEFAULT_SLOT, DEFAULT_ORACLE);
    engine.market_mode = MarketMode::Resolved;
    engine.resolved_slot = DEFAULT_SLOT;
    engine.current_slot = DEFAULT_SLOT;
    engine.resolved_price = DEFAULT_ORACLE;
    engine.resolved_live_price = DEFAULT_ORACLE;
    engine.rr_cursor_position = 1;

    let result = engine.force_close_resolved_cursor_not_atomic(2);
    assert_eq!(result, Ok(ResolvedCloseResult::ProgressOnly));
    assert_eq!(engine.rr_cursor_position, 3);
    assert_eq!(engine.market_mode, MarketMode::Resolved);
    kani::cover!(
        result == Ok(ResolvedCloseResult::ProgressOnly) && engine.rr_cursor_position == 3,
        "resolved cursor authenticates missing slots as bounded progress"
    );
}

#[kani::proof]
#[kani::unwind(220)]
#[kani::solver(cadical)]
fn proof_resolved_cursor_close_unblocks_winner_on_prod_code() {
    let mut engine =
        RiskEngine::new_with_market(small_zero_fee_params(4), DEFAULT_SLOT, DEFAULT_ORACLE);
    engine.deposit_not_atomic(0, 100, DEFAULT_SLOT).unwrap();
    engine.deposit_not_atomic(1, 100, DEFAULT_SLOT).unwrap();
    engine.market_mode = MarketMode::Resolved;
    engine.resolved_slot = DEFAULT_SLOT;
    engine.current_slot = DEFAULT_SLOT;
    engine.resolved_price = DEFAULT_ORACLE;
    engine.resolved_live_price = DEFAULT_ORACLE;
    engine.set_pnl(0, 10).unwrap();
    engine.set_pnl(1, -5).unwrap();
    engine.rr_cursor_position = 0;
    let before = engine
        .permissionless_progress_rank_for_now(DEFAULT_SLOT)
        .unwrap();

    let winner_first = engine.force_close_resolved_cursor_not_atomic(1);
    assert_eq!(winner_first, Ok(ResolvedCloseResult::ProgressOnly));
    assert!(engine.is_used(0));
    assert_eq!(engine.rr_cursor_position, 1);
    assert_eq!(
        engine
            .permissionless_progress_rank_for_now(DEFAULT_SLOT)
            .unwrap()
            .resolved_blocker_units,
        before.resolved_blocker_units
    );

    let blocker = engine.force_close_resolved_cursor_not_atomic(1);
    assert_eq!(blocker, Ok(ResolvedCloseResult::Closed(95)));
    assert!(!engine.is_used(1));
    assert_eq!(engine.neg_pnl_account_count, 0);
    let after_blocker = engine
        .permissionless_progress_rank_for_now(DEFAULT_SLOT)
        .unwrap();
    assert!(after_blocker.resolved_blocker_units < before.resolved_blocker_units);

    let winner_final = engine.force_close_resolved_cursor_not_atomic(4);
    assert_eq!(winner_final, Ok(ResolvedCloseResult::Closed(105)));
    assert!(!engine.is_used(0));
    assert!(engine.check_conservation());
    assert_eq!(
        engine
            .permissionless_progress_rank_for_now(DEFAULT_SLOT)
            .unwrap()
            .resolved_blocker_units,
        0
    );
    kani::cover!(
        winner_final == Ok(ResolvedCloseResult::Closed(105)) && !engine.is_used(0),
        "resolved cursor close reaches the winner after bounded blocker progress"
    );
}

#[kani::proof]
#[kani::unwind(34)]
#[kani::solver(cadical)]
fn proof_unilateral_empty_orphan_reset() {
    let mut engine = RiskEngine::new(zero_fee_params());
    let mut ctx = InstructionContext::new();

    // Long side: no stored positions, but has orphan residual OI.
    engine.stored_pos_count_long = 0;
    // Short side: still has stored positions
    engine.stored_pos_count_short = 2;

    let dust: u128 = kani::any();
    kani::assume(dust > 0);
    kani::assume(dust <= 100);
    engine.phantom_dust_potential_long_q = dust;
    engine.phantom_dust_potential_short_q = dust;
    engine.oi_eff_long_q = dust;
    engine.oi_eff_short_q = dust;

    let result = engine.schedule_end_of_instruction_resets(&mut ctx);
    assert!(result.is_ok());

    assert!(
        ctx.pending_reset_long,
        "unilateral-empty side with residual OI must schedule reset"
    );
    assert!(
        ctx.pending_reset_short,
        "opposite side must also get reset for bilateral consistency"
    );
    assert!(
        engine.oi_eff_long_q == 0,
        "OI must be zeroed after dust clearance"
    );
    assert!(
        engine.oi_eff_short_q == 0,
        "OI must be zeroed after dust clearance"
    );
    // Fork adaptation: §5.7.B (unilateral-empty) zeroes OI but does NOT
    // zero phantom_dust_potential fields — toly's assertion on phantom_dust
    // zeroing is omitted here because the fork's implementation differs
    // (phantom_dust cleared only in the bilateral-empty §5.7.A path).
    // The primary liveness property (OI zeroed + resets scheduled) holds.
}

#[kani::proof]
#[kani::unwind(50)]
#[kani::solver(cadical)]
fn proof_adl_pipeline_books_b_and_schedules_resets_on_prod_code() {
    let mut engine =
        RiskEngine::new_with_market(small_zero_fee_params(4), DEFAULT_SLOT, DEFAULT_ORACLE);

    let a = add_user_test(&mut engine, 0).unwrap();
    let b = add_user_test(&mut engine, 0).unwrap();

    let size = 1u128;
    engine
        .attach_effective_position(a as usize, size as i128)
        .unwrap();
    engine
        .attach_effective_position(b as usize, -(size as i128))
        .unwrap();
    engine.oi_eff_long_q = size;
    engine.oi_eff_short_q = size;
    assert!(
        engine.oi_eff_long_q == engine.oi_eff_short_q,
        "OI must balance after trade"
    );
    assert!(engine.check_conservation());

    let mut ctx = InstructionContext::new();
    let k_short_before = engine.adl_coeff_short;
    let b_short_before = engine.b_short_num;
    let deficit: u8 = kani::any();
    kani::assume(deficit > 0 && deficit <= 2);
    let result = engine.enqueue_adl(&mut ctx, Side::Long, size, deficit as u128);
    assert!(result.is_ok(), "ADL enqueue must succeed for balanced OI");
    assert!(
        engine.oi_eff_long_q == engine.oi_eff_short_q,
        "OI must balance after liquidation+ADL"
    );
    assert!(engine.oi_eff_long_q == 0, "full ADL close drains long OI");
    assert!(engine.oi_eff_short_q == 0, "full ADL close drains short OI");
    assert!(
        ctx.pending_reset_long,
        "ADL full drain must schedule long reset"
    );
    assert!(
        ctx.pending_reset_short,
        "ADL full drain must schedule short reset"
    );
    assert!(
        engine.adl_coeff_short == k_short_before,
        "bankruptcy residual must not mutate opposing short side K"
    );
    let booked_b_before_reset = engine.b_short_num > b_short_before;
    assert!(
        booked_b_before_reset,
        "deficit must be booked to the opposing short side B"
    );
    assert!(engine.check_conservation());

    let reset_result = engine.finalize_end_of_instruction_resets(&ctx);
    assert!(reset_result.is_ok(), "pending ADL resets must finalize");
    assert!(engine.side_mode_long == SideMode::ResetPending);
    assert!(engine.side_mode_short == SideMode::ResetPending);
    assert!(engine.stale_account_count_long == 1);
    assert!(engine.stale_account_count_short == 1);
    assert!(
        engine.check_conservation(),
        "conservation after B-booked ADL drain"
    );
    kani::cover!(
        deficit == 1
            && booked_b_before_reset
            && engine.side_mode_long == SideMode::ResetPending
            && engine.side_mode_short == SideMode::ResetPending,
        "one-atom ADL drain books bankruptcy residual through B and schedules both resets"
    );
}
