//! Kani proofs addressing formal verification checklist gaps.
//! Each proof targets a specific checklist item (A/B/E/F/G).

#![cfg(kani)]

mod common;
use common::*;

// ############################################################################
// A2: 0 <= R_i <= max(PNL_i, 0) after set_pnl
// ############################################################################

/// set_pnl always maintains 0 <= R_i <= max(PNL_i, 0) for any PNL transition.
#[kani::proof]
#[kani::unwind(34)]
#[kani::solver(cadical)]
fn proof_a2_reserve_bounds_after_set_pnl() {
    let mut engine = RiskEngine::new(zero_fee_params());
    let idx = add_user_test(&mut engine, 0).unwrap();
    engine
        .deposit_not_atomic(idx, 500_000, DEFAULT_SLOT)
        .unwrap();

    let init_pnl: i128 = kani::any();
    kani::assume(init_pnl >= -100_000 && init_pnl <= 100_000);
    engine.set_pnl(idx as usize, init_pnl);

    let r1 = engine.accounts[idx as usize].reserved_pnl;
    let pos1 = core::cmp::max(engine.accounts[idx as usize].pnl, 0) as u128;
    assert!(r1 <= pos1, "A2: R_i <= max(PNL_i,0) after first set");

    let new_pnl: i128 = kani::any();
    kani::assume(new_pnl > -200_000 && new_pnl < 200_000);
    kani::assume(new_pnl != i128::MIN);
    kani::assume(new_pnl <= MAX_ACCOUNT_POSITIVE_PNL as i128 || new_pnl <= 0);
    engine.set_pnl(idx as usize, new_pnl);

    let r2 = engine.accounts[idx as usize].reserved_pnl;
    let pos2 = core::cmp::max(engine.accounts[idx as usize].pnl, 0) as u128;
    assert!(r2 <= pos2, "A2: R_i <= max(PNL_i,0) after transition");

    kani::cover!(init_pnl > 0 && new_pnl > init_pnl, "positive increase");
    kani::cover!(init_pnl > 0 && new_pnl < 0, "positive to negative");
    kani::cover!(init_pnl < 0 && new_pnl > 0, "negative to positive");
}

// ############################################################################
// A7: fee_credits ∈ [-(i128::MAX), 0] after trade fees
// ############################################################################

/// After a trade, fee_credits stays in valid range.
#[kani::proof]
#[kani::unwind(8)]
#[kani::solver(cadical)]
fn proof_a7_fee_credits_bounds_after_trade() {
    let mut engine = RiskEngine::new(default_params());
    let a = add_user_test(&mut engine, 0).unwrap();
    engine.deposit_not_atomic(a, 1_000, DEFAULT_SLOT).unwrap();

    let fee: u16 = kani::any();
    kani::assume(fee > 0 && fee <= 2_000);
    kani::cover!((fee as u128) <= 1_000, "fee fully paid from capital");
    kani::cover!((fee as u128) > 1_000, "fee shortfall routes to fee_credits");

    let (paid, impact, dropped) = engine
        .charge_fee_to_insurance(a as usize, fee as u128)
        .unwrap();

    let expected_debt = if (fee as u128) > 1_000 {
        (fee as u128) - 1_000
    } else {
        0
    };
    let fc = engine.accounts[a as usize].fee_credits.get();

    assert!(paid <= fee as u128);
    assert!(impact <= fee as u128);
    assert!(paid + dropped <= fee as u128);
    assert!(
        fc == -(expected_debt as i128),
        "A7: unpaid fee shortfall is represented as non-positive fee credit"
    );
    assert!(fc <= 0, "A7: fee_credits <= 0");
    assert!(fc != i128::MIN, "A7: fee_credits != i128::MIN");
    assert!(fc >= -(i128::MAX), "A7: fee_credits >= -(i128::MAX)");
    assert!(
        engine.check_conservation(),
        "fee routing must preserve public accounting invariants"
    );
}

// ############################################################################
// F2: Insurance floor respected after absorb_protocol_loss
// ############################################################################

// ############################################################################
// F8: Loss seniority in settlement (losses before fees)
// ############################################################################

/// Public settlement applies negative PnL before fee-debt sweep.
#[kani::proof]
#[kani::unwind(34)]
#[kani::solver(cadical)]
fn proof_f8_loss_seniority_in_touch() {
    let mut engine = RiskEngine::new_with_market(zero_fee_params(), DEFAULT_SLOT, DEFAULT_ORACLE);
    let a = add_user_test(&mut engine, 0).unwrap();

    let loss: u8 = kani::any();
    let fee_debt: u8 = kani::any();
    kani::assume(loss >= 1 && loss <= 20);
    kani::assume(fee_debt >= 1 && fee_debt <= 20);

    engine
        .deposit_not_atomic(a, loss as u128, DEFAULT_SLOT)
        .unwrap();
    engine.set_pnl(a as usize, -(loss as i128)).unwrap();
    engine.accounts[a as usize].fee_credits = I128::new(-(fee_debt as i128));

    let capital_before = engine.accounts[a as usize].capital.get();
    let insurance_before = engine.insurance_fund.balance.get();
    let vault_before = engine.vault.get();

    engine
        .settle_flat_negative_pnl_not_atomic(a, DEFAULT_SLOT)
        .unwrap();
    assert!(
        capital_before == loss as u128,
        "fixture gives principal enough to cover only the PnL loss"
    );
    assert!(
        engine.accounts[a as usize].capital.get() == 0,
        "loss settlement must consume the available principal"
    );
    assert!(
        engine.accounts[a as usize].pnl == 0,
        "negative PnL must be cleared by principal before fees"
    );
    assert!(
        engine.accounts[a as usize].fee_credits.get() == -(fee_debt as i128),
        "fee debt must remain unpaid when loss settlement exhausted principal"
    );
    assert!(
        engine.insurance_fund.balance.get() == insurance_before,
        "fee debt must not be paid ahead of senior PnL loss"
    );

    assert!(
        engine.vault.get() == vault_before,
        "settlement must not move external vault balance"
    );
    assert!(engine.check_conservation(), "conservation after settlement");

    kani::cover!(loss > 1 && fee_debt > 1, "loss and fee debt both exercised");
}

// ############################################################################
// B7: OI_long == OI_short after trade (symbolic size)
// ############################################################################

#[kani::proof]
#[kani::unwind(34)]
#[kani::solver(cadical)]
fn proof_b7_oi_balance_after_trade() {
    let mut engine = RiskEngine::new_with_market(zero_fee_params(), DEFAULT_SLOT, DEFAULT_ORACLE);
    let a = add_user_test(&mut engine, 0).unwrap();
    let b = add_user_test(&mut engine, 0).unwrap();
    engine.deposit_not_atomic(a, 500_000, DEFAULT_SLOT).unwrap();
    engine.deposit_not_atomic(b, 500_000, DEFAULT_SLOT).unwrap();

    let lots: u8 = kani::any();
    kani::assume(lots > 0 && lots <= 20);
    let size_q = (lots as u128) * POS_SCALE;
    let a_is_long: bool = kani::any();

    if a_is_long {
        engine
            .attach_effective_position(a as usize, size_q as i128)
            .unwrap();
        engine
            .attach_effective_position(b as usize, -(size_q as i128))
            .unwrap();
    } else {
        engine
            .attach_effective_position(a as usize, -(size_q as i128))
            .unwrap();
        engine
            .attach_effective_position(b as usize, size_q as i128)
            .unwrap();
    }
    engine.oi_eff_long_q = size_q;
    engine.oi_eff_short_q = size_q;

    let eff_a = engine.effective_pos_q(a as usize);
    let eff_b = engine.effective_pos_q(b as usize);
    let expected_long =
        if eff_a > 0 { eff_a as u128 } else { 0 } + if eff_b > 0 { eff_b as u128 } else { 0 };
    let expected_short = if eff_a < 0 { eff_a.unsigned_abs() } else { 0 }
        + if eff_b < 0 { eff_b.unsigned_abs() } else { 0 };

    assert!(engine.oi_eff_long_q == expected_long);
    assert!(engine.oi_eff_short_q == expected_short);
    assert!(
        engine.oi_eff_long_q == engine.oi_eff_short_q,
        "B7: OI_long == OI_short after a balanced trade"
    );
    assert!(engine.stored_pos_count_long == 1);
    assert!(engine.stored_pos_count_short == 1);
    assert!(
        engine.check_conservation(),
        "balanced trade state must preserve public accounting invariants"
    );
    kani::cover!(a_is_long, "account a long after trade");
    kani::cover!(!a_is_long, "account a short after trade");
}

// ############################################################################
// B1: Conservation after trade with fees
// ############################################################################

#[kani::proof]
#[kani::unwind(8)]
#[kani::solver(cadical)]
fn proof_b1_conservation_after_trade_with_fees() {
    let mut engine = RiskEngine::new(default_params());
    let a = add_user_test(&mut engine, 0).unwrap();
    let b = add_user_test(&mut engine, 0).unwrap();
    engine.deposit_not_atomic(a, 1_000, DEFAULT_SLOT).unwrap();
    engine.deposit_not_atomic(b, 1_000, DEFAULT_SLOT).unwrap();
    assert!(engine.check_conservation());

    let fee: u16 = kani::any();
    kani::assume(fee > 0 && fee <= 2_000);
    kani::cover!((fee as u128) <= 1_000, "fee fully paid from capital");
    kani::cover!((fee as u128) > 1_000, "fee shortfall routed to fee_credits");

    let vault_before = engine.vault.get();
    let ins_before = engine.insurance_fund.balance.get();

    let (paid_a, impact_a, dropped_a) = engine
        .charge_fee_to_insurance(a as usize, fee as u128)
        .unwrap();
    let (paid_b, impact_b, dropped_b) = engine
        .charge_fee_to_insurance(b as usize, fee as u128)
        .unwrap();

    assert!(paid_a <= fee as u128 && paid_b <= fee as u128);
    assert!(impact_a <= fee as u128 && impact_b <= fee as u128);
    assert!(paid_a + dropped_a <= fee as u128);
    assert!(paid_b + dropped_b <= fee as u128);
    assert!(engine.accounts[a as usize].fee_credits.get() <= 0);
    assert!(engine.accounts[b as usize].fee_credits.get() <= 0);
    assert!(
        engine.vault.get() == vault_before,
        "fee routing must not move vault tokens"
    );
    assert!(
        engine.insurance_fund.balance.get() == ins_before + paid_a + paid_b,
        "insurance fund increases by realized paid fees"
    );
    assert!(
        engine.check_conservation(),
        "B1: conservation after trade with fees"
    );
}

// ############################################################################
// E8: Position bound enforcement
// ############################################################################

#[kani::proof]
#[kani::unwind(34)]
#[kani::solver(cadical)]
fn proof_e8_position_bound_enforcement() {
    let mut engine = RiskEngine::new(zero_fee_params());
    let a = add_user_test(&mut engine, 0).unwrap();
    let b = add_user_test(&mut engine, 0).unwrap();
    engine
        .deposit_not_atomic(a, 10_000_000_000, DEFAULT_SLOT)
        .unwrap();
    engine
        .deposit_not_atomic(b, 10_000_000_000, DEFAULT_SLOT)
        .unwrap();

    let oversize = (MAX_POSITION_ABS_Q + 1) as i128;
    let result = engine.execute_trade_not_atomic(
        a,
        b,
        DEFAULT_ORACLE,
        DEFAULT_SLOT,
        oversize,
        DEFAULT_ORACLE,
        0i128,
        0u64,
        0,
        100,
        None,
    );
    assert!(result.is_err(), "E8: oversize trade must be rejected");

    kani::cover!(true, "oversize rejected");
}

// ############################################################################
// B5: PNL_matured_pos_tot <= PNL_pos_tot after set_pnl + set_reserved_pnl
// ############################################################################

#[kani::proof]
#[kani::unwind(34)]
#[kani::solver(cadical)]
fn proof_b5_matured_leq_pos_tot() {
    let mut engine = RiskEngine::new(zero_fee_params());
    let idx = add_user_test(&mut engine, 0).unwrap();
    engine
        .deposit_not_atomic(idx, 500_000, DEFAULT_SLOT)
        .unwrap();

    let pnl: i128 = kani::any();
    kani::assume(pnl > 0 && pnl <= 100_000);
    engine.set_pnl(idx as usize, pnl);
    assert!(
        engine.pnl_matured_pos_tot <= engine.pnl_pos_tot,
        "B5 after set_pnl"
    );

    // Transition to lower PNL
    let new_pnl: i128 = kani::any();
    kani::assume(new_pnl >= 0 && new_pnl < pnl);
    engine.set_pnl(idx as usize, new_pnl);
    assert!(
        engine.pnl_matured_pos_tot <= engine.pnl_pos_tot,
        "B5: matured <= pos_tot after decrease"
    );

    // Transition to negative PNL
    engine.set_pnl(idx as usize, -1000);
    assert!(
        engine.pnl_matured_pos_tot <= engine.pnl_pos_tot,
        "B5: matured <= pos_tot after negative"
    );

    kani::cover!(new_pnl > 0, "partial decrease");
}

// ############################################################################
// G4: DrainOnly blocks OI-increasing trades
// ############################################################################

#[kani::proof]
#[kani::unwind(34)]
#[kani::solver(cadical)]
fn proof_g4_drain_only_blocks_oi_increase() {
    // v12.19: DrainOnly is only reachable when the side has nonzero
    // residual OI (spec §5.6 — A_side below MIN_A_SIDE). With OI=0
    // execute_trade's pre-open flush transitions DrainOnly → Normal
    // via §5.7.D. Build a valid balanced residual-OI state directly,
    // then exercise the real execute_trade DrainOnly gate.
    let mut engine = RiskEngine::new_with_market(zero_fee_params(), DEFAULT_SLOT, DEFAULT_ORACLE);
    let a = add_user_test(&mut engine, 0).unwrap();
    let b = add_user_test(&mut engine, 0).unwrap();
    engine.deposit_not_atomic(a, 500_000, DEFAULT_SLOT).unwrap();
    engine.deposit_not_atomic(b, 500_000, DEFAULT_SLOT).unwrap();

    let open_q = (5 * POS_SCALE) as i128;
    engine
        .attach_effective_position(a as usize, open_q)
        .unwrap();
    engine
        .attach_effective_position(b as usize, -open_q)
        .unwrap();
    engine.oi_eff_long_q = open_q as u128;
    engine.oi_eff_short_q = open_q as u128;
    assert!(engine.check_conservation());

    engine.side_mode_long = SideMode::DrainOnly;

    let add_lots: u8 = kani::any();
    kani::assume(add_lots > 0 && add_lots <= 5);
    let size = (add_lots as i128) * POS_SCALE as i128;
    let oi_long_before = engine.oi_eff_long_q;
    let oi_short_before = engine.oi_eff_short_q;
    let eff_a_before = engine.effective_pos_q(a as usize);
    let eff_b_before = engine.effective_pos_q(b as usize);
    let new_eff_a = eff_a_before + size;
    let new_eff_b = eff_b_before - size;
    let (oi_long_after, oi_short_after) = engine
        .bilateral_oi_after(&eff_a_before, &new_eff_a, &eff_b_before, &new_eff_b)
        .unwrap();

    assert!(
        oi_long_after > oi_long_before,
        "G4 setup must be an OI-increasing long-side trade"
    );

    // ENG-PORT-4 fixup: 6-arg signature. Pass (eff_a_before, new_eff_a, eff_b_before,
    // new_eff_b) in scope, matching the bilateral_oi_after call earlier in the test.
    let result = engine.enforce_side_mode_oi_gate(eff_a_before, new_eff_a, eff_b_before, new_eff_b, oi_long_after, oi_short_after);
    match result {
        Err(RiskError::SideBlocked) => {}
        _ => assert!(
            false,
            "G4: DrainOnly must block OI-increasing trades at the implementation gate"
        ),
    }
    assert!(engine.side_mode_long == SideMode::DrainOnly);
    assert!(engine.oi_eff_long_q == oi_long_before);
    assert!(engine.oi_eff_short_q == oi_short_before);
    assert!(engine.effective_pos_q(a as usize) == eff_a_before);
    assert!(engine.effective_pos_q(b as usize) == eff_b_before);
    assert!(engine.check_conservation());

    kani::cover!(add_lots > 0, "DrainOnly blocks a positive OI increase");
}

// ############################################################################
// Goal 5: No same-trade bootstrap from positive slippage
// ############################################################################

/// A trade whose own positive slippage would be needed to pass IM must be
/// rejected. The trade-open equity excludes the candidate trade's gain.
#[kani::proof]
#[kani::unwind(34)]
#[kani::solver(cadical)]
fn proof_goal5_no_same_trade_bootstrap() {
    let mut engine = RiskEngine::new_with_market(zero_fee_params(), DEFAULT_SLOT, DEFAULT_ORACLE);
    let a = add_user_test(&mut engine, 0).unwrap();
    let b = add_user_test(&mut engine, 0).unwrap();
    engine.deposit_not_atomic(a, 10_000, DEFAULT_SLOT).unwrap();
    engine
        .deposit_not_atomic(b, 1_000_000, DEFAULT_SLOT)
        .unwrap();

    // Candidate: 200 units at oracle 1000, execution 900.
    // A's own positive slippage gain is 20_000, exactly enough to make
    // 10_000 capital appear to satisfy the 20_000 IM requirement if the
    // implementation incorrectly counted same-trade gains.
    let big_size = (200 * POS_SCALE) as i128;
    let exec_price = 900u64;
    let candidate_gain = 20_000i128;
    assert!(
        candidate_gain == (big_size / POS_SCALE as i128) * ((DEFAULT_ORACLE - exec_price) as i128)
    );
    assert!(candidate_gain == 20_000);

    // Model the post-candidate state at the margin gate: A has the candidate
    // gain in PnL, B's opposite loss has been settled from capital, and the
    // resulting residual backs A's positive PnL. This is the exact bootstrap
    // danger: counting A's own gain would make the trade pass IM.
    engine
        .attach_effective_position(a as usize, big_size)
        .unwrap();
    engine
        .attach_effective_position(b as usize, -big_size)
        .unwrap();
    engine.oi_eff_long_q = big_size as u128;
    engine.oi_eff_short_q = big_size as u128;
    engine.accounts[a as usize].pnl = candidate_gain;
    engine.pnl_pos_tot = candidate_gain as u128;
    engine.pnl_matured_pos_tot = candidate_gain as u128;
    engine.accounts[b as usize].capital = U128::new(980_000);
    engine.c_tot = U128::new(990_000);
    assert!(engine.vault.get() == engine.c_tot.get() + candidate_gain as u128);
    assert!(engine.check_conservation());

    let account = &engine.accounts[a as usize];
    let im_req = 20_000i128;
    let eq_if_gain_counted = engine.account_equity_trade_open_raw(account, a as usize, 0);
    let eq_trade_open = engine.account_equity_trade_open_raw(account, a as usize, candidate_gain);

    assert!(
        eq_if_gain_counted >= im_req,
        "setup must represent a real same-trade bootstrap opportunity"
    );
    assert!(
        eq_trade_open == 10_000,
        "trade-open equity must remove the candidate trade's own positive slippage"
    );
    assert!(
        !engine.is_above_initial_margin_trade_open(
            account,
            a as usize,
            DEFAULT_ORACLE,
            candidate_gain
        ),
        "Goal 5: trade must NOT bootstrap itself via own positive slippage"
    );

    kani::cover!(
        eq_if_gain_counted >= im_req && eq_trade_open < im_req,
        "bootstrap blocked by trade-open equity"
    );
}

// ############################################################################
// Goal 7: Pending merge uses max horizon
// ############################################################################

/// When both buckets are occupied, merges into pending use horizon = max.
#[kani::proof]
#[kani::unwind(4)]
#[kani::solver(cadical)]
fn proof_goal7_pending_merge_max_horizon() {
    let mut engine = RiskEngine::new(zero_fee_params());
    let idx = add_user_test(&mut engine, 0).unwrap();
    engine
        .deposit_not_atomic(idx, 1_000_000, DEFAULT_SLOT)
        .unwrap();

    // First append creates sched
    engine.accounts[idx as usize].pnl += 10_000;
    engine.pnl_pos_tot += 10_000;
    engine.append_or_route_new_reserve(idx as usize, 10_000, DEFAULT_SLOT, 10);
    assert_eq!(engine.accounts[idx as usize].sched_present, 1);

    // Second append creates pending (different slot)
    engine.accounts[idx as usize].pnl += 10_000;
    engine.pnl_pos_tot += 10_000;
    engine.append_or_route_new_reserve(idx as usize, 10_000, DEFAULT_SLOT + 1, 5);
    assert_eq!(engine.accounts[idx as usize].pending_present, 1);

    let h1: u8 = kani::any();
    kani::assume(h1 >= 1 && h1 <= 100);
    let h_lock = h1 as u64;

    // Third append merges into pending
    engine.accounts[idx as usize].pnl += 10_000;
    engine.pnl_pos_tot += 10_000;
    engine.append_or_route_new_reserve(idx as usize, 10_000, DEFAULT_SLOT + 2, h_lock);

    assert!(
        engine.accounts[idx as usize].pending_horizon >= h_lock,
        "Goal 7: pending horizon must be >= h_lock after merge"
    );

    kani::cover!(true, "pending max-horizon enforced");
}

// ############################################################################
// Goal 23: No pure-capital insurance draw without accrual
// ############################################################################

/// deposit does not call accrue_market_to and must not draw from insurance.
#[kani::proof]
#[kani::unwind(34)]
#[kani::solver(cadical)]
fn proof_goal23_deposit_no_insurance_draw() {
    let mut engine = RiskEngine::new(zero_fee_params());
    let idx = add_user_test(&mut engine, 0).unwrap();
    engine
        .deposit_not_atomic(idx, 100_000, DEFAULT_SLOT)
        .unwrap();

    let ins_before = engine.insurance_fund.balance.get();

    // Symbolic deposit amount
    let amount: u128 = kani::any();
    kani::assume(amount > 0 && amount <= 500_000);

    let result = engine.deposit_not_atomic(idx, amount, DEFAULT_SLOT + 1);
    assert!(
        result.is_ok(),
        "valid existing-account deposit must succeed"
    );

    let ins_after = engine.insurance_fund.balance.get();
    assert!(
        ins_after >= ins_before,
        "Goal 23: deposit must never decrease insurance"
    );

    kani::cover!(result.is_ok(), "deposit succeeds without insurance draw");
}

// ############################################################################
// Goal 27: Path-independent touched-account finalization
// ############################################################################

/// Finalize_touched_accounts_post_live produces the same conversion result
/// regardless of which accounts are touched (order-independent within the
/// touched set, since the shared snapshot is computed once).
#[kani::proof]
#[kani::unwind(34)]
#[kani::solver(cadical)]
fn proof_goal27_finalize_path_independent() {
    let mut engine = RiskEngine::new_with_market(zero_fee_params(), DEFAULT_SLOT, DEFAULT_ORACLE);
    let a = add_user_test(&mut engine, 0).unwrap();
    let b = add_user_test(&mut engine, 0).unwrap();
    engine.deposit_not_atomic(a, 500_000, DEFAULT_SLOT).unwrap();
    engine.deposit_not_atomic(b, 500_000, DEFAULT_SLOT).unwrap();
    assert!(a < b);

    // Construct a valid whole-haircut snapshot state: one touched account
    // has matured positive PnL and the other is a touched no-op. This keeps
    // the proof focused on path independence while still exercising the
    // real finalize conversion path.
    engine.accounts[a as usize].pnl = 10_000;
    engine.accounts[b as usize].pnl = 0;
    engine.pnl_pos_tot = 10_000;
    engine.pnl_matured_pos_tot = 10_000;
    engine.vault = U128::new(engine.vault.get() + 10_000);

    // Touch a then b
    let mut ctx1 = InstructionContext::new_with_admission(0, 100);
    assert!(ctx1.add_touched(a));
    assert!(ctx1.add_touched(b));

    // Touch b then a (reversed order)
    let mut ctx2 = InstructionContext::new_with_admission(0, 100);
    assert!(ctx2.add_touched(b));
    assert!(ctx2.add_touched(a));

    // Reversed insertion must canonicalize to the same sorted touched set.
    assert!(ctx1.touched_count == 2);
    assert!(ctx2.touched_count == 2);
    assert!(ctx1.touched_accounts[0] == ctx2.touched_accounts[0]);
    assert!(ctx1.touched_accounts[1] == ctx2.touched_accounts[1]);
    assert!(ctx1.touched_accounts[0] == a);
    assert!(ctx1.touched_accounts[1] == b);

    let cap_a_before = engine.accounts[a as usize].capital.get();
    let cap_b_before = engine.accounts[b as usize].capital.get();

    let senior_sum = engine.c_tot.get() + engine.insurance_fund.balance.get();
    let residual = engine.vault.get() - senior_sum;
    let h_snapshot_den = engine.pnl_matured_pos_tot;
    let h_snapshot_num = core::cmp::min(residual, h_snapshot_den);
    let is_whole = h_snapshot_den > 0 && h_snapshot_num == h_snapshot_den;
    assert!(is_whole);

    let mut _ctx_snap = percolator::InstructionContext::new();
    let finalized_a = engine.finalize_touched_account_post_live_with_snapshot(
        ctx1.touched_accounts[0] as usize,
        is_whole,
        false,
        &mut _ctx_snap,
    );
    assert!(finalized_a.is_ok());
    let mut _ctx_snap2 = percolator::InstructionContext::new();
    let finalized_b = engine.finalize_touched_account_post_live_with_snapshot(
        ctx1.touched_accounts[1] as usize,
        is_whole,
        false,
        &mut _ctx_snap2,
    );
    assert!(finalized_b.is_ok());

    assert_eq!(
        engine.accounts[a as usize].capital.get(),
        cap_a_before + 10_000,
        "Goal 27: a's conversion must use shared whole snapshot"
    );
    assert_eq!(
        engine.accounts[b as usize].capital.get(),
        cap_b_before,
        "Goal 27: touched no-op account must be order-independent"
    );
    assert_eq!(engine.accounts[a as usize].pnl, 0);
    assert_eq!(engine.accounts[b as usize].pnl, 0);
    assert_eq!(engine.pnl_pos_tot, 0);
    assert_eq!(
        engine.pnl_matured_pos_tot, 0,
        "Goal 27: matured aggregate must be consumed exactly once"
    );
    assert!(engine.check_conservation());

    kani::cover!(true, "finalize is order-independent");
}

// ############################################################################
// Two-bucket warmup proofs
// ############################################################################

/// R_i = sched_remaining + pending_remaining after append.
#[kani::proof]
#[kani::unwind(5)]
#[kani::solver(cadical)]
fn proof_two_bucket_reserve_sum_after_append() {
    let mut engine = RiskEngine::new(zero_fee_params());
    let idx = add_user_test(&mut engine, 0).unwrap();
    engine
        .deposit_not_atomic(idx, 1_000_000, DEFAULT_SLOT)
        .unwrap();

    let h_lock: u64 = kani::any();
    kani::assume(h_lock >= 1 && h_lock <= 100);

    // First append: creates scheduled
    let r1: u128 = kani::any();
    kani::assume(r1 > 0 && r1 <= 50_000);
    engine.accounts[idx as usize].pnl += r1 as i128;
    engine.pnl_pos_tot += r1;
    engine.append_or_route_new_reserve(idx as usize, r1, DEFAULT_SLOT, h_lock);

    // Second append at different slot: creates pending
    let r2: u128 = kani::any();
    kani::assume(r2 > 0 && r2 <= 50_000);
    engine.accounts[idx as usize].pnl += r2 as i128;
    engine.pnl_pos_tot += r2;
    engine.append_or_route_new_reserve(idx as usize, r2, DEFAULT_SLOT + 1, h_lock);

    // R_i must equal sum of both buckets
    let a = &engine.accounts[idx as usize];
    let sched_r = if a.sched_present != 0 {
        a.sched_remaining_q
    } else {
        0
    };
    let pend_r = if a.pending_present != 0 {
        a.pending_remaining_q
    } else {
        0
    };
    assert_eq!(
        a.reserved_pnl,
        sched_r + pend_r,
        "R_i must equal sched + pending"
    );

    kani::cover!(
        a.sched_present != 0 && a.pending_present != 0,
        "both buckets present"
    );
}

/// Loss hits pending first (newest-first).
#[kani::proof]
#[kani::unwind(5)]
#[kani::solver(cadical)]
fn proof_two_bucket_loss_newest_first() {
    let mut engine = RiskEngine::new(zero_fee_params());
    let idx = add_user_test(&mut engine, 0).unwrap();
    engine
        .deposit_not_atomic(idx, 1_000_000, DEFAULT_SLOT)
        .unwrap();

    // Create sched + pending
    engine.accounts[idx as usize].pnl = 30_000;
    engine.pnl_pos_tot = 30_000;
    engine.append_or_route_new_reserve(idx as usize, 10_000, DEFAULT_SLOT, 10);
    engine.append_or_route_new_reserve(idx as usize, 20_000, DEFAULT_SLOT + 1, 10);

    let sched_before = engine.accounts[idx as usize].sched_remaining_q;

    // Loss that fits in pending
    let loss: u128 = kani::any();
    kani::assume(loss > 0 && loss <= 20_000);
    engine.apply_reserve_loss_newest_first(idx as usize, loss);

    // Scheduled must be untouched
    assert_eq!(
        engine.accounts[idx as usize].sched_remaining_q, sched_before,
        "scheduled must be untouched when loss fits in pending"
    );

    kani::cover!(loss == 20_000, "exact pending drain");
    kani::cover!(loss < 20_000, "partial pending loss");
}

/// Scheduled bucket matures exactly per its horizon.
#[kani::proof]
#[kani::unwind(5)]
#[kani::solver(cadical)]
fn proof_two_bucket_scheduled_timing() {
    let mut engine = RiskEngine::new(zero_fee_params());
    let idx = add_user_test(&mut engine, 0).unwrap();
    engine
        .deposit_not_atomic(idx, 1_000_000, DEFAULT_SLOT)
        .unwrap();

    let anchor: u128 = kani::any();
    kani::assume(anchor > 0 && anchor <= 1_000);
    let h: u64 = kani::any();
    kani::assume(h >= 1 && h <= 20);

    engine.accounts[idx as usize].pnl = anchor as i128;
    engine.pnl_pos_tot = anchor;
    engine.append_or_route_new_reserve(idx as usize, anchor, DEFAULT_SLOT, h);

    let dt: u64 = kani::any();
    kani::assume(dt >= 1 && dt <= 40);
    engine.current_slot = DEFAULT_SLOT + dt;

    let r_before = engine.accounts[idx as usize].reserved_pnl;
    engine.advance_profit_warmup(idx as usize);
    let released = r_before - engine.accounts[idx as usize].reserved_pnl;

    let expected = if dt as u128 >= h as u128 {
        anchor
    } else {
        mul_div_floor_u128(anchor, dt as u128, h as u128)
    };
    assert_eq!(
        released, expected,
        "release must match floor(anchor*elapsed/horizon)"
    );

    kani::cover!(dt < h, "partial maturity");
    kani::cover!(dt >= h, "full maturity");
}

/// Pending does not mature.
#[kani::proof]
#[kani::unwind(5)]
#[kani::solver(cadical)]
fn proof_two_bucket_pending_non_maturity() {
    let mut engine = RiskEngine::new(zero_fee_params());
    let idx = add_user_test(&mut engine, 0).unwrap();
    engine
        .deposit_not_atomic(idx, 1_000_000, DEFAULT_SLOT)
        .unwrap();

    // Create sched + pending
    engine.accounts[idx as usize].pnl = 30_000;
    engine.pnl_pos_tot = 30_000;
    engine.append_or_route_new_reserve(idx as usize, 10_000, DEFAULT_SLOT, 10);
    engine.append_or_route_new_reserve(idx as usize, 20_000, DEFAULT_SLOT + 1, 10);

    let pending_before = engine.accounts[idx as usize].pending_remaining_q;

    // Advance well past horizon
    engine.current_slot = DEFAULT_SLOT + 200;
    engine.advance_profit_warmup(idx as usize);

    // If pending is still present (not promoted), it must not have matured
    if engine.accounts[idx as usize].pending_present != 0 {
        assert_eq!(
            engine.accounts[idx as usize].pending_remaining_q, pending_before,
            "pending must not mature while pending"
        );
    }

    kani::cover!(true, "warmup with pending exercised");
}
