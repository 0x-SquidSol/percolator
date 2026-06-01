// End-to-end integration tests with realistic trading scenarios
// Tests complete user journeys with multiple participants

#[cfg(feature = "test")]
use percolator::i128::U128;
#[cfg(feature = "test")]
use percolator::*;

#[cfg(feature = "test")]
fn default_params() -> RiskParams {
    // v12.19 envelope: max_price_move * max_dt + funding_budget + liq_fee <= maint_bps.
    // E2E demos: allow dt up to 200 slots (for test_e2e_complete_user_journey's
    // slot advancement pattern) and per-slot cap of 14 bps:
    //   price_budget   = 14 * 200 = 2800
    //   funding_budget = 10_000 * 200 * 10_000 / 1e9 = 20
    //   total          = 2800 + 20 + 50 = 2870 <= maint 3000 ✓
    // Cap at dt=10, P=100: 14 * 10 * 100 = 14_000 units of abs_dp*10_000
    // → abs_dp <= 1 (= 1% move). Tests adapt their price-move magnitudes.
    RiskParams {
        maintenance_margin_bps: 3000,
        initial_margin_bps: 3500,
        max_trading_fee_bps: 10,
        max_accounts: 64,
        liquidation_fee_bps: 50,
        liquidation_fee_cap: U128::new(100_000),
        min_liquidation_abs: U128::new(0),
        min_nonzero_mm_req: 31,
        min_nonzero_im_req: 32,
        h_min: 0,
        h_max: 100,
        resolve_price_deviation_bps: 1000,
        max_accrual_dt_slots: 200,
        max_abs_funding_e9_per_slot: 10_000,
        min_funding_lifetime_slots: 10_000_000,
        max_active_positions_per_side: MAX_ACCOUNTS as u64,
        max_price_move_bps_per_slot: 14,
    }
}

/// Helper: allocate a user slot without moving capital. Uses `materialize_at`
/// (test-visible back-door). The public engine API materializes only via
/// deposit with amount >= min_initial_deposit — exercised separately.
#[cfg(feature = "test")]
fn add_user_test(engine: &mut RiskEngine, _fee_payment: u128) -> Result<u16> {
    let idx = engine.free_head;
    if idx == u16::MAX || (idx as usize) >= MAX_ACCOUNTS {
        return Err(RiskError::Overflow);
    }
    engine.materialize_at(idx, engine.current_slot)?;
    Ok(idx)
}

/// Helper: materialize an LP account (materialize as user then rewrite kind).
#[cfg(feature = "test")]
#[allow(dead_code)]
fn add_lp_test(
    engine: &mut RiskEngine,
    matcher_program: [u8; 32],
    matcher_context: [u8; 32],
    _fee_payment: u128,
) -> Result<u16> {
    let idx = add_user_test(engine, 0)?;
    engine.accounts[idx as usize].kind = Account::KIND_LP;
    engine.accounts[idx as usize].matcher_program = matcher_program;
    engine.accounts[idx as usize].matcher_context = matcher_context;
    Ok(idx)
}

/// Helper: create i128 position size from base quantity (scaled by POS_SCALE)
#[cfg(feature = "test")]
fn pos_q(qty: i64) -> i128 {
    let abs_val = (qty as i128).unsigned_abs();
    let scaled = abs_val.checked_mul(POS_SCALE).unwrap();
    if qty < 0 {
        -(scaled as i128)
    } else {
        scaled as i128
    }
}

/// Helper: crank to make trades/withdrawals work
#[cfg(feature = "test")]
fn crank(engine: &mut RiskEngine, slot: u64, oracle_price: u64) {
    let _ = engine.keeper_crank_not_atomic(slot, oracle_price, &[], 64, 0i128, 0, 100, None, 0);
}

// ============================================================================
// E2E Test 1: Complete User Journey
// ============================================================================

#[test]
#[cfg(feature = "test")]
fn test_e2e_complete_user_journey() {
    // Scenario: Alice and Bob trade, experience PNL, warmup, withdrawal

    let mut engine = Box::new(RiskEngine::new(default_params()));

    // Initialize insurance fund
    let _ = engine.top_up_insurance_fund(50_000, 0);

    // Add two users with capital
    let alice = add_user_test(&mut engine, 0).unwrap();
    let bob = add_user_test(&mut engine, 0).unwrap();

    let oracle_price: u64 = 100; // 100 quote per base

    // Users deposit principal
    engine.deposit_not_atomic(alice, 100_000, 0).unwrap();
    engine.deposit_not_atomic(bob, 150_000, 0).unwrap();

    // Make crank fresh
    crank(&mut engine, 0, oracle_price);

    // === Phase 1: Trading ===

    // Alice goes long 50 base, Bob takes the other side (short)
    engine
        .execute_trade_not_atomic(
            alice,
            bob,
            oracle_price,
            0,
            pos_q(50),
            oracle_price,
            0i128,
            0u64,
            0,
            100,
            None,
        )
        .unwrap();

    // Check effective positions
    let alice_eff = engine.effective_pos_q(alice as usize);
    let bob_eff = engine.effective_pos_q(bob as usize);
    assert!(alice_eff > 0, "Alice should be long");
    assert!(bob_eff < 0, "Bob should be short");

    // Conservation should hold
    assert!(engine.check_conservation(), "Conservation after trade");

    // === Phase 2: Price Movement ===

    // v12.19: price cap = 14 bps/slot × 10 slots × P=100 → abs_dp <= 1 (1%).
    let new_price: u64 = 101;

    // Accrue market to new price
    engine.advance_slot(10);
    let slot = engine.current_slot;
    engine.accrue_market_to(slot, new_price, 0).unwrap();

    // Settle side effects for Alice (should have positive PnL from long)
    {
        let mut _ctx = InstructionContext::new_with_admission(0, 100);
        engine.settle_side_effects_live(alice as usize, &mut _ctx)
    }
    .unwrap();

    let alice_pnl = engine.accounts[alice as usize].pnl;
    // Long position + price up = positive PnL
    assert!(
        alice_pnl > 0,
        "Alice should have positive PnL after price increase"
    );

    // === Phase 3: PNL Warmup ===

    // Advance some slots
    engine.advance_slot(50);

    // Touch to settle and convert warmup
    let slot = engine.current_slot;
    {
        let mut ctx = InstructionContext::new_with_admission(0, 100);
        engine.accrue_market_to(slot, new_price, 0).unwrap();
        engine.current_slot = slot;
        engine
            .touch_account_live_local(alice as usize, &mut ctx)
            .unwrap();
        engine.finalize_touched_accounts_post_live(&mut ctx);
    }

    // The key invariant is conservation
    assert!(engine.check_conservation(), "Conservation after warmup");

    // === Phase 4: Close positions and withdraw_not_atomic ===

    let slot = engine.current_slot;
    crank(&mut engine, slot, new_price);

    // Alice closes her position (sell)
    let alice_pos = engine.effective_pos_q(alice as usize);
    if alice_pos != 0 {
        let abs_pos = alice_pos.unsigned_abs() as i128;
        let slot = engine.current_slot;
        // alice_pos > 0 (long), so closing means b buys from a (swap a,b with positive size)
        engine
            .execute_trade_not_atomic(
                bob, alice, new_price, slot, abs_pos, new_price, 0i128, 0u64, 0, 100, None,
            )
            .unwrap();
    }

    // Advance for full warmup
    engine.advance_slot(200);
    let slot = engine.current_slot;
    {
        let mut ctx = InstructionContext::new_with_admission(0, 100);
        engine.accrue_market_to(slot, new_price, 0).unwrap();
        engine.current_slot = slot;
        engine
            .touch_account_live_local(alice as usize, &mut ctx)
            .unwrap();
        engine.finalize_touched_accounts_post_live(&mut ctx);
    }

    // Alice withdraws some capital
    let slot = engine.current_slot;
    crank(&mut engine, slot, new_price);
    let alice_cap = engine.accounts[alice as usize].capital.get();
    if alice_cap > 1000 {
        let slot = engine.current_slot;
        engine
            .withdraw_not_atomic(alice, 1000, new_price, slot, 0i128, 0, 100, None)
            .unwrap();
    }

    assert!(engine.check_conservation(), "Conservation after withdrawal");
}

// ============================================================================
// E2E Test 2: Funding Complete Cycle
// ============================================================================

#[test]
#[cfg(feature = "test")]
fn test_e2e_funding_complete_cycle() {
    // Scenario: Users trade, positive funding rate accrues (longs pay shorts),
    // then positions flip. Verifies funding actually changes account PnL.

    let mut engine = Box::new(RiskEngine::new(default_params()));
    let _ = engine.top_up_insurance_fund(50_000, 0);

    let alice = add_user_test(&mut engine, 0).unwrap();
    let bob = add_user_test(&mut engine, 0).unwrap();

    let oracle_price: u64 = 100;

    engine.deposit_not_atomic(alice, 200_000, 0).unwrap();
    engine.deposit_not_atomic(bob, 200_000, 0).unwrap();

    crank(&mut engine, 0, oracle_price);

    // Alice goes long, Bob goes short
    engine
        .execute_trade_not_atomic(
            alice,
            bob,
            oracle_price,
            0,
            pos_q(100),
            oracle_price,
            0i128,
            0u64,
            0,
            100,
            None,
        )
        .unwrap();

    // Record capital before funding (settle_losses converts PnL to capital changes,
    // so we track capital, not PnL directly)
    let alice_cap_before = engine.accounts[alice as usize].capital.get();
    let bob_cap_before = engine.accounts[bob as usize].capital.get();

    // Apply a positive funding rate: longs pay shorts.
    // Use rr_touch_limit=64 so the crank can make protective progress (required
    // when equity_active_accrual=true under the toly-correct bounded-accrual path).
    engine.advance_slot(1);
    let slot1 = engine.current_slot;
    engine
        .keeper_crank_not_atomic(slot1, oracle_price, &[], 64, 5_000i128, 0, 100, None, 64)
        .unwrap();

    // Advance time so next accrue_market_to applies funding.
    engine.advance_slot(20);
    let slot2 = engine.current_slot;

    // This crank accrues the market (which applies 20 slots of funding at rate 5000)
    // then touches both accounts (settle_side_effects realizes the K delta into PnL,
    // then settle_losses transfers negative PnL from capital).
    engine
        .keeper_crank_not_atomic(
            slot2,
            oracle_price,
            &[(alice, None), (bob, None)],
            64,
            5_000i128,
            0,
            100,
            None,
            64,
        )
        .unwrap();

    let alice_cap_after = engine.accounts[alice as usize].capital.get();
    let bob_cap_after = engine.accounts[bob as usize].capital.get();

    // Alice (long) paid funding → capital decreased (loss settled from principal)
    assert!(
        alice_cap_after < alice_cap_before,
        "positive rate: long capital must decrease from funding (before={}, after={})",
        alice_cap_before,
        alice_cap_after
    );

    // Bob (short) received funding → PnL positive, but it goes to reserved_pnl
    // (warmup). Bob's capital stays the same but PnL + reserved goes up.
    // Check that bob didn't lose capital like alice did.
    assert!(
        bob_cap_after >= bob_cap_before,
        "positive rate: short capital must not decrease from funding (before={}, after={})",
        bob_cap_before,
        bob_cap_after
    );

    // Net check: alice lost more capital than bob (funding is zero-sum at K level,
    // but floor rounding means payers lose weakly more than receivers gain)
    let alice_loss = alice_cap_before - alice_cap_after;
    assert!(alice_loss > 0, "alice must have lost capital from funding");

    assert!(engine.check_conservation(), "Conservation after funding");

    // === Positions Flip ===
    let slot = engine.current_slot;

    // Alice closes long and opens short (total -200 base)
    engine
        .execute_trade_not_atomic(
            bob,
            alice,
            oracle_price,
            slot,
            pos_q(200),
            oracle_price,
            0i128,
            0u64,
            0,
            100,
            None,
        )
        .unwrap();

    // Now Alice is short and Bob is long
    let alice_eff = engine.effective_pos_q(alice as usize);
    let bob_eff = engine.effective_pos_q(bob as usize);
    assert!(alice_eff < 0, "Alice should now be short");
    assert!(bob_eff > 0, "Bob should now be long");

    assert!(
        engine.check_conservation(),
        "Conservation after position flip"
    );
}

#[test]
#[cfg(feature = "test")]
fn test_e2e_negative_funding_rate() {
    // Negative funding rate: shorts pay longs

    let mut engine = Box::new(RiskEngine::new(default_params()));
    let _ = engine.top_up_insurance_fund(50_000, 0);

    let alice = add_user_test(&mut engine, 0).unwrap();
    let bob = add_user_test(&mut engine, 0).unwrap();

    let oracle_price: u64 = 100;

    engine.deposit_not_atomic(alice, 200_000, 0).unwrap();
    engine.deposit_not_atomic(bob, 200_000, 0).unwrap();

    crank(&mut engine, 0, oracle_price);

    // Alice long, Bob short
    engine
        .execute_trade_not_atomic(
            alice,
            bob,
            oracle_price,
            0,
            pos_q(100),
            oracle_price,
            0i128,
            0u64,
            0,
            100,
            None,
        )
        .unwrap();

    let alice_cap_before = engine.accounts[alice as usize].capital.get();
    let bob_cap_before = engine.accounts[bob as usize].capital.get();

    // Apply negative rate: shorts pay longs (-5000 bps/slot).
    // Use rr_touch_limit=64 so the crank can make protective progress (required
    // when equity_active_accrual=true under the toly-correct bounded-accrual path).
    engine.advance_slot(1);
    let slot1 = engine.current_slot;
    engine
        .keeper_crank_not_atomic(slot1, oracle_price, &[], 64, -5_000i128, 0, 100, None, 64)
        .unwrap();

    // Advance and settle
    engine.advance_slot(20);
    let slot2 = engine.current_slot;
    engine
        .keeper_crank_not_atomic(
            slot2,
            oracle_price,
            &[(alice, None), (bob, None)],
            64,
            -5_000i128,
            0,
            100,
            None,
            64,
        )
        .unwrap();

    let alice_cap_after = engine.accounts[alice as usize].capital.get();
    let bob_cap_after = engine.accounts[bob as usize].capital.get();

    // Negative rate: shorts pay, longs receive
    // Bob (short) paid funding → capital decreased (loss settled from principal)
    assert!(
        bob_cap_after < bob_cap_before,
        "negative rate: short capital must decrease (before={}, after={})",
        bob_cap_before,
        bob_cap_after
    );

    // Alice (long) received → capital must not decrease
    assert!(
        alice_cap_after >= alice_cap_before,
        "negative rate: long capital must not decrease (before={}, after={})",
        alice_cap_before,
        alice_cap_after
    );

    let bob_loss = bob_cap_before - bob_cap_after;
    assert!(
        bob_loss > 0,
        "bob must have lost capital from negative funding"
    );

    assert!(
        engine.check_conservation(),
        "Conservation with negative funding"
    );
}
