//! Section 7 — v12.14.0 Spec Compliance Proofs
//!
//! Properties 46, 59-75: live funding, configuration immutability,
//! bilateral OI decomposition, partial liquidation, deposit guards, profit conversion.

#![cfg(kani)]

mod common;
use common::*;

// ############################################################################
// PROPERTY 46: Funding rate recomputation determinism and bound enforcement
// ############################################################################

/// accrue_market_to accepts funding_rate_e9 when |rate| <= MAX_ABS_FUNDING_E9_PER_SLOT.
/// v12.16.4: rate is passed directly to accrue, no stored field.
#[kani::proof]
#[kani::unwind(34)]
#[kani::solver(cadical)]
fn proof_funding_rate_accepted_in_accrue() {
    let mut engine = RiskEngine::new_with_market(zero_fee_params(), 0, DEFAULT_ORACLE);

    // Bound by the configured params cap (tighter than the global const).
    let rate: i32 = kani::any();
    kani::assume(rate.unsigned_abs() as u64 <= engine.params.max_abs_funding_e9_per_slot);

    let result = engine.accrue_market_to(0, 1, rate as i128);
    assert!(
        result.is_ok(),
        "in-bounds rate must be accepted by accrue_market_to"
    );
}

// ############################################################################
// PROPERTY 74: Funding rate bound enforcement
// ############################################################################

/// accrue_market_to returns Err for |rate| > MAX_ABS_FUNDING_E9_PER_SLOT.
/// v12.16.4: validation folded into accrue_market_to.
#[kani::proof]
#[kani::unwind(34)]
#[kani::solver(cadical)]
fn proof_funding_rate_bound_rejected() {
    let mut engine = RiskEngine::new(zero_fee_params());
    let rate: i128 = kani::any();
    kani::assume(rate.unsigned_abs() > MAX_ABS_FUNDING_E9_PER_SLOT as u128);
    let result = engine.accrue_market_to(0, 1, rate);
    assert!(result.is_err(), "out-of-bounds rate must return Err");
}

// ############################################################################
// PROPERTY 72: Funding sign and floor-direction correctness
// ############################################################################

/// When r_last > 0, K_long decreases and K_short increases (longs pay shorts).
/// When r_last < 0, K_long increases and K_short decreases (shorts pay longs).
/// fund_term uses floor division: positive quotients round down, negative round
/// toward negative infinity.
#[kani::proof]
#[kani::unwind(34)]
#[kani::solver(cadical)]
fn proof_funding_sign_and_floor() {
    let mut engine = RiskEngine::new(zero_fee_params());
    engine.adl_mult_long = ADL_ONE;
    engine.adl_mult_short = ADL_ONE;
    engine.oi_eff_long_q = POS_SCALE;
    engine.oi_eff_short_q = POS_SCALE;
    engine.last_oracle_price = DEFAULT_ORACLE;
    engine.fund_px_last = DEFAULT_ORACLE; // funding basis (v12.16.5)
    engine.last_market_slot = 0;

    // Symbolic rate bounded by params cap (zero_fee_params: 10^8 < 10^9 const).
    let rate: i32 = kani::any();
    kani::assume(rate != 0);
    kani::assume(rate.unsigned_abs() as u64 <= engine.params.max_abs_funding_e9_per_slot);

    let f_long_before = engine.f_long_num;
    let f_short_before = engine.f_short_num;

    // dt=1, same price → only funding changes F (v12.16.5: F-only, no K)
    let result = engine.accrue_market_to(1, DEFAULT_ORACLE, rate as i128);
    assert!(result.is_ok());

    if rate > 0 {
        // Longs pay shorts → F_long decreases, F_short increases
        assert!(
            engine.f_long_num <= f_long_before,
            "positive rate: F_long must not increase"
        );
        assert!(
            engine.f_short_num >= f_short_before,
            "positive rate: F_short must not decrease"
        );
    } else {
        assert!(
            engine.f_long_num >= f_long_before,
            "negative rate: F_long must not decrease"
        );
        assert!(
            engine.f_short_num <= f_short_before,
            "negative rate: F_short must not increase"
        );
    }
}

/// Explicit floor-direction test: rate=-1, price=1000, dt=1 produces
/// fund_num = -1000, fund_term = floor(-1000/10000) = floor(-0.1) = -1.
/// Truncation toward zero would give 0 (wrong). Floor toward -∞ gives -1.
/// This means longs gain and shorts lose even for tiny negative rates.
#[kani::proof]
#[kani::unwind(34)]
#[kani::solver(cadical)]
fn proof_funding_floor_not_truncation() {
    let mut engine = RiskEngine::new(zero_fee_params());
    engine.adl_mult_long = ADL_ONE;
    engine.adl_mult_short = ADL_ONE;
    engine.oi_eff_long_q = POS_SCALE;
    engine.oi_eff_short_q = POS_SCALE;
    engine.last_oracle_price = DEFAULT_ORACLE;
    engine.fund_px_last = DEFAULT_ORACLE; // funding basis (v12.16.5)
    engine.last_market_slot = 0;

    let f_long_before = engine.f_long_num;
    let f_short_before = engine.f_short_num;

    // tiny negative rate passed directly (v12.16.5: F-only, no K)
    let result = engine.accrue_market_to(1, DEFAULT_ORACLE, -1);
    assert!(result.is_ok());

    // fund_num_total = 1000 * (-1) * 1 = -1000 (one exact delta, no floor/substep)
    // F_long -= A_long * (-1000) = F_long + ADL_ONE * 1000
    // F_short += A_short * (-1000) = F_short - ADL_ONE * 1000
    let expected_f_delta = (ADL_ONE as i128) * 1000;
    assert_eq!(
        engine.f_long_num,
        f_long_before + expected_f_delta,
        "negative rate: F_long must increase by A_long * |fund_num_total|"
    );
    assert_eq!(
        engine.f_short_num,
        f_short_before - expected_f_delta,
        "negative rate: F_short must decrease by A_short * |fund_num_total|"
    );
}

// ############################################################################
// PROPERTY 73: Funding skip on zero OI
// ############################################################################

/// accrue_market_to applies no funding delta when short side OI is zero.
#[kani::proof]
#[kani::unwind(34)]
#[kani::solver(cadical)]
fn proof_funding_skip_zero_oi_short() {
    // Spec §3.1 requires bilateral OI at the public boundary, so a valid
    // state with short OI zero also has long OI zero. Spec §5.5 steps 6-8
    // then make funding inactive and allow idle-market fast-forward.
    let mut engine = RiskEngine::new(zero_fee_params());
    engine.adl_mult_long = ADL_ONE;
    engine.adl_mult_short = ADL_ONE;
    engine.last_oracle_price = DEFAULT_ORACLE;
    engine.last_market_slot = 0;

    engine.oi_eff_long_q = 0;
    engine.oi_eff_short_q = 0;
    assert!(engine.oi_eff_long_q == engine.oi_eff_short_q);

    let k_long_before = engine.adl_coeff_long;
    let k_short_before = engine.adl_coeff_short;
    let f_long_before = engine.f_long_num;
    let f_short_before = engine.f_short_num;

    let rate: i16 = kani::any(); // symbolic rate
    kani::assume((rate.unsigned_abs() as u64) <= engine.params.max_abs_funding_e9_per_slot);
    let dt: u8 = kani::any();
    kani::assume(dt > 0);
    kani::cover!(
        (dt as u64) > engine.params.max_accrual_dt_slots,
        "zero-OI inactive funding allows over-envelope dt"
    );

    let now_slot = dt as u64;
    let result = engine.accrue_market_to(now_slot, DEFAULT_ORACLE, rate as i128);
    assert!(result.is_ok(), "valid zero-OI accrual must succeed");
    assert_eq!(engine.adl_coeff_long, k_long_before);
    assert_eq!(engine.adl_coeff_short, k_short_before);
    assert_eq!(engine.f_long_num, f_long_before);
    assert_eq!(engine.f_short_num, f_short_before);
    assert_eq!(engine.last_market_slot, now_slot);
    assert_eq!(engine.current_slot, now_slot);
    assert_eq!(engine.last_oracle_price, DEFAULT_ORACLE);
    assert_eq!(engine.fund_px_last, DEFAULT_ORACLE);
}

/// accrue_market_to applies no funding delta when long side OI is zero.
#[kani::proof]
#[kani::unwind(34)]
#[kani::solver(cadical)]
fn proof_funding_skip_zero_oi_long() {
    // Spec §3.1 requires bilateral OI at the public boundary, so a valid
    // state with long OI zero also has short OI zero.
    let mut engine = RiskEngine::new(zero_fee_params());
    engine.adl_mult_long = ADL_ONE;
    engine.adl_mult_short = ADL_ONE;
    engine.last_oracle_price = DEFAULT_ORACLE;
    engine.last_market_slot = 0;

    engine.oi_eff_long_q = 0;
    engine.oi_eff_short_q = 0;
    assert!(engine.oi_eff_long_q == engine.oi_eff_short_q);

    let k_long_before = engine.adl_coeff_long;
    let k_short_before = engine.adl_coeff_short;
    let f_long_before = engine.f_long_num;
    let f_short_before = engine.f_short_num;

    let rate: i16 = kani::any();
    kani::assume((rate.unsigned_abs() as u64) <= engine.params.max_abs_funding_e9_per_slot);
    let dt: u8 = kani::any();
    kani::assume(dt > 0);
    kani::cover!(
        (dt as u64) > engine.params.max_accrual_dt_slots,
        "zero-OI inactive funding allows over-envelope dt"
    );

    let now_slot = dt as u64;
    let result = engine.accrue_market_to(now_slot, DEFAULT_ORACLE, rate as i128);
    assert!(result.is_ok(), "valid zero-OI accrual must succeed");
    assert_eq!(engine.adl_coeff_long, k_long_before);
    assert_eq!(engine.adl_coeff_short, k_short_before);
    assert_eq!(engine.f_long_num, f_long_before);
    assert_eq!(engine.f_short_num, f_short_before);
    assert_eq!(engine.last_market_slot, now_slot);
    assert_eq!(engine.current_slot, now_slot);
    assert_eq!(engine.last_oracle_price, DEFAULT_ORACLE);
    assert_eq!(engine.fund_px_last, DEFAULT_ORACLE);
}

/// accrue_market_to applies no funding K delta when both sides have zero OI.
#[kani::proof]
#[kani::unwind(34)]
#[kani::solver(cadical)]
fn proof_funding_skip_zero_oi_both() {
    let mut engine = RiskEngine::new(zero_fee_params());
    engine.adl_mult_long = ADL_ONE;
    engine.adl_mult_short = ADL_ONE;
    engine.last_oracle_price = DEFAULT_ORACLE;
    engine.last_market_slot = 0;

    engine.oi_eff_long_q = 0;
    engine.oi_eff_short_q = 0;

    let k_long_before = engine.adl_coeff_long;
    let k_short_before = engine.adl_coeff_short;
    let f_long_before = engine.f_long_num;
    let f_short_before = engine.f_short_num;

    let rate: i16 = kani::any();
    kani::assume((rate.unsigned_abs() as u64) <= engine.params.max_abs_funding_e9_per_slot);
    let dt: u8 = kani::any();
    kani::assume(dt > 0);
    kani::cover!(
        (dt as u64) > engine.params.max_accrual_dt_slots,
        "zero-OI inactive funding allows over-envelope dt"
    );

    let now_slot = dt as u64;
    let result = engine.accrue_market_to(now_slot, DEFAULT_ORACLE, rate as i128);
    assert!(result.is_ok(), "valid zero-OI accrual must succeed");
    assert_eq!(engine.adl_coeff_long, k_long_before);
    assert_eq!(engine.adl_coeff_short, k_short_before);
    assert_eq!(engine.f_long_num, f_long_before);
    assert_eq!(engine.f_short_num, f_short_before);
    assert_eq!(engine.last_market_slot, now_slot);
    assert_eq!(engine.current_slot, now_slot);
    assert_eq!(engine.last_oracle_price, DEFAULT_ORACLE);
    assert_eq!(engine.fund_px_last, DEFAULT_ORACLE);
}

// ############################################################################
// PROPERTY 71: Funding with large dt bounded by max_accrual_dt_slots
// ############################################################################

/// accrue_market_to applies one exact funding delta for any dt up to
/// `max_accrual_dt_slots` (no internal sub-stepping in v12.16.5+).
#[kani::proof]
#[kani::unwind(34)]
#[kani::solver(cadical)]
fn proof_funding_substep_large_dt() {
    let mut engine = RiskEngine::new(zero_fee_params());
    engine.adl_mult_long = ADL_ONE;
    engine.adl_mult_short = ADL_ONE;
    engine.oi_eff_long_q = POS_SCALE;
    engine.oi_eff_short_q = POS_SCALE;
    engine.last_oracle_price = DEFAULT_ORACLE;
    engine.fund_px_last = DEFAULT_ORACLE; // funding basis (v12.16.5)
    engine.last_market_slot = 0;

    // v12.16.5: one exact total delta, no substeps. Bounded by
    // max_accrual_dt_slots (zero_fee_params = 1000).
    let dt = engine.params.max_accrual_dt_slots;
    let result = engine.accrue_market_to(dt, DEFAULT_ORACLE, 100);
    assert!(result.is_ok());

    // fund_num_total = fund_px_last * rate * dt = 1000 * 100 * 1000
    // F_long -= A_long * fund_num_total; K must NOT change from funding (F-only).
    assert_eq!(
        engine.adl_coeff_long, 0,
        "K_long must not change from funding"
    );
    let expected_f: i128 = -((ADL_ONE as i128) * (DEFAULT_ORACLE as i128) * 100 * (dt as i128));
    assert_eq!(
        engine.f_long_num, expected_f,
        "F_long must reflect exact total funding delta"
    );
}

// ############################################################################
// PROPERTY 75: Funding price-basis timing
// ############################################################################

/// Funding uses fund_px_0 (start-of-call snapshot of fund_px_last), not the
/// current oracle_price. After the call, fund_px_last is updated to oracle_price.
#[kani::proof]
#[kani::unwind(34)]
#[kani::solver(cadical)]
fn proof_funding_price_basis_timing() {
    let mut engine = RiskEngine::new(zero_fee_params());
    engine.adl_mult_long = ADL_ONE;
    engine.adl_mult_short = ADL_ONE;
    engine.oi_eff_long_q = POS_SCALE;
    engine.oi_eff_short_q = POS_SCALE;
    engine.last_oracle_price = 500; // old price for mark basis
    engine.fund_px_last = 500; // old price for funding basis (v12.16.5)
    engine.last_market_slot = 0;

    // Move one price tick over five slots: at P_last=500 and cap=4 bps/slot,
    // abs_dp * 10_000 == cap * dt * P_last == 10_000, so the v12.19
    // price-move envelope is tight but valid.
    let rate: i128 = 10_000;
    let result = engine.accrue_market_to(5, 501, rate);
    assert!(result.is_ok());

    // v12.16.5: Funding goes to F, mark goes to K.
    // fund_px_0 = 500 (fund_px_last before this call)
    // fund_num_total = 500 * 10_000 * 5 = 25_000_000
    // F_long -= ADL_ONE * 25_000_000
    // K_long only has mark: ΔP = 501-500 = 1, K_long += ADL_ONE
    let expected_k_long = ADL_ONE as i128; // mark only
    assert_eq!(
        engine.adl_coeff_long, expected_k_long,
        "K_long must reflect mark only, not funding"
    );
    let expected_f_long = -((ADL_ONE as i128) * 25_000_000i128);
    assert_eq!(
        engine.f_long_num, expected_f_long,
        "F_long must use fund_px_0=500, not oracle=501"
    );

    // After call, last_oracle_price must be updated to oracle_price
    assert_eq!(
        engine.last_oracle_price, 501,
        "last_oracle_price must be updated to oracle_price for next interval"
    );
    assert_eq!(
        engine.fund_px_last, 501,
        "fund_px_last must be updated only after funding used the start snapshot"
    );
}

// ############################################################################
// Funding: zero rate produces no K change (regression from v11.31)
// ############################################################################

/// When r_last = 0, no funding transfer occurs regardless of dt or OI.
#[kani::proof]
#[kani::unwind(34)]
#[kani::solver(cadical)]
fn proof_accrue_no_funding_when_rate_zero() {
    let mut engine = RiskEngine::new(zero_fee_params());
    engine.adl_mult_long = ADL_ONE;
    engine.adl_mult_short = ADL_ONE;
    engine.oi_eff_long_q = POS_SCALE;
    engine.oi_eff_short_q = POS_SCALE;
    engine.last_oracle_price = DEFAULT_ORACLE;
    engine.last_market_slot = 0;

    let dt: u16 = kani::any();
    kani::assume(dt >= 1 && dt <= 1000);

    let k_long_before = engine.adl_coeff_long;
    let k_short_before = engine.adl_coeff_short;

    let result = engine.accrue_market_to(dt as u64, DEFAULT_ORACLE, 0);
    assert!(result.is_ok());

    assert_eq!(
        engine.adl_coeff_long, k_long_before,
        "zero rate: K_long unchanged"
    );
    assert_eq!(
        engine.adl_coeff_short, k_short_before,
        "zero rate: K_short unchanged"
    );
}

/// accrue_market_to still applies mark-to-market correctly.
#[kani::proof]
#[kani::unwind(34)]
#[kani::solver(cadical)]
fn proof_accrue_mark_still_works() {
    let mut engine = RiskEngine::new_with_market(zero_fee_params(), 0, DEFAULT_ORACLE);

    let a = add_user_test(&mut engine, 0).unwrap();
    let b = add_user_test(&mut engine, 0).unwrap();

    // Valid public OI state: one long and one short with matching OI.
    engine
        .attach_effective_position(a as usize, POS_SCALE as i128)
        .unwrap();
    engine
        .attach_effective_position(b as usize, -(POS_SCALE as i128))
        .unwrap();
    engine.oi_eff_long_q = POS_SCALE;
    engine.oi_eff_short_q = POS_SCALE;

    let new_price: u64 = kani::any();
    kani::assume(new_price >= DEFAULT_ORACLE - 4);
    kani::assume(new_price <= DEFAULT_ORACLE + 4);
    kani::assume(new_price != DEFAULT_ORACLE);

    let k_long_before = engine.adl_coeff_long;
    let k_short_before = engine.adl_coeff_short;

    let now_slot = 10;
    let result = engine.accrue_market_to(now_slot, new_price, 0);
    assert!(result.is_ok());

    // Mark must change K: K_long += A_long * ΔP, K_short -= A_short * ΔP
    let delta_p = (new_price as i128) - (DEFAULT_ORACLE as i128);
    let expected_k_long = k_long_before + (ADL_ONE as i128) * delta_p;
    let expected_k_short = k_short_before - (ADL_ONE as i128) * delta_p;

    assert!(
        engine.adl_coeff_long == expected_k_long,
        "K_long must reflect mark-to-market"
    );
    assert!(
        engine.adl_coeff_short == expected_k_short,
        "K_short must reflect mark-to-market"
    );
}

// ############################################################################
// PROPERTY 62: Pure deposit no-insurance-draw
// ############################################################################

/// deposit never calls absorb_protocol_loss, never decrements I (spec property 62).
/// settle_losses MAY pay from capital to reduce negative PNL (that's loss settlement,
/// not insurance draw), but resolve_flat_negative is NOT called.
#[kani::proof]
#[kani::unwind(34)]
#[kani::solver(cadical)]
fn proof_deposit_no_insurance_draw() {
    let mut engine = RiskEngine::new(zero_fee_params());

    let idx = add_user_test(&mut engine, 0).unwrap();
    // Start with zero capital
    engine.deposit_not_atomic(idx, 0, DEFAULT_SLOT).unwrap();

    // Set very large negative PNL (much more than any deposit)
    engine.set_pnl(idx as usize, -10_000_000i128);

    let ins_before = engine.insurance_fund.balance.get();

    // Deposit a small amount — capital insufficient to cover PNL
    let amount: u32 = kani::any();
    kani::assume(amount > 0 && amount <= 1_000_000);

    let result = engine.deposit_not_atomic(idx, amount as u128, DEFAULT_SLOT);
    assert!(result.is_ok());

    // Insurance fund must NOT decrease (no absorb_protocol_loss via resolve_flat_negative)
    assert!(
        engine.insurance_fund.balance.get() >= ins_before,
        "deposit must never decrement I"
    );

    // PNL must still be negative (settle_losses paid from capital but couldn't cover all)
    assert!(
        engine.accounts[idx as usize].pnl < 0,
        "negative PNL must survive deposit — resolve_flat_negative not called"
    );
}

// ############################################################################
// PROPERTY 66: Flat authoritative deposit sweep
// ############################################################################

/// deposit does NOT sweep fee debt when PNL < 0 persists after settle_losses.
/// Symbolic deposit amount — for any amount, if PNL stays negative, no sweep.
#[kani::proof]
#[kani::unwind(34)]
#[kani::solver(cadical)]
fn proof_deposit_sweep_pnl_guard() {
    let mut engine = RiskEngine::new(zero_fee_params());

    let idx = add_user_test(&mut engine, 0).unwrap();
    // Start with zero capital
    engine.deposit_not_atomic(idx, 0, DEFAULT_SLOT).unwrap();

    // Symbolic fee debt
    let debt: u16 = kani::any();
    kani::assume(debt >= 1 && debt <= 10_000);
    engine.accounts[idx as usize].fee_credits = I128::new(-(debt as i128));

    // Set large negative PNL that exceeds any deposit amount
    engine.set_pnl(idx as usize, -10_000_000i128);

    let fc_before = engine.accounts[idx as usize].fee_credits.get();

    // Symbolic deposit — always insufficient to cover PNL=-10M
    let amount: u32 = kani::any();
    kani::assume(amount >= 1 && amount <= 1_000_000);
    engine
        .deposit_not_atomic(idx, amount as u128, DEFAULT_SLOT)
        .unwrap();

    // After deposit: capital went to settle_losses (paid toward PNL=-10M)
    // PNL is still very negative, so sweep must NOT happen
    assert!(
        engine.accounts[idx as usize].fee_credits.get() == fc_before,
        "deposit must not sweep when PNL < 0 after settle_losses"
    );
    assert!(
        engine.accounts[idx as usize].pnl < 0,
        "PNL must still be negative — settle_losses can't cover full loss"
    );
}

/// deposit DOES sweep fee debt on flat state with PNL >= 0.
/// Symbolic deposit amount exercises sweep with varying capital levels.
#[kani::proof]
#[kani::unwind(34)]
#[kani::solver(cadical)]
fn proof_deposit_sweep_when_pnl_nonneg() {
    let mut engine = RiskEngine::new(zero_fee_params());

    let idx = add_user_test(&mut engine, 0).unwrap();
    // Symbolic initial capital — ensures fee_debt_sweep has capital to pay from
    let init_cap: u32 = kani::any();
    kani::assume(init_cap >= 10_000 && init_cap <= 1_000_000);
    engine
        .deposit_not_atomic(idx, init_cap as u128, DEFAULT_SLOT)
        .unwrap();

    // Give account fee debt
    engine.accounts[idx as usize].fee_credits = I128::new(-5000);

    // PNL = 0 (flat position, no losses)
    assert!(engine.accounts[idx as usize].pnl == 0);

    // Symbolic deposit amount
    let dep: u32 = kani::any();
    kani::assume(dep >= 1 && dep <= 100_000);
    engine
        .deposit_not_atomic(idx, dep as u128, DEFAULT_SLOT)
        .unwrap();

    // fee_credits must have improved (debt partially/fully paid)
    assert!(
        engine.accounts[idx as usize].fee_credits.get() > -5000,
        "deposit must sweep fee debt when flat with PNL >= 0"
    );
}

// ############################################################################
// PROPERTY 61: Insurance top-up bounded arithmetic + now_slot
// ############################################################################

/// top_up_insurance_fund uses checked addition, enforces MAX_VAULT_TVL,
/// accepts monotone zero-OI idle fast-forward, rejects exposed over-envelope
/// no-accrual time jumps, and is validate-then-mutate on rejection.
#[kani::proof]
#[kani::unwind(34)]
#[kani::solver(cadical)]
fn proof_top_up_insurance_now_slot() {
    let mut engine = RiskEngine::new(zero_fee_params());
    engine.current_slot = 50;

    let exposed: bool = kani::any();
    if exposed {
        let a = add_user_test(&mut engine, 0).unwrap();
        let b = add_user_test(&mut engine, 0).unwrap();
        engine
            .attach_effective_position(a as usize, POS_SCALE as i128)
            .unwrap();
        engine
            .attach_effective_position(b as usize, -(POS_SCALE as i128))
            .unwrap();
        engine.oi_eff_long_q = POS_SCALE;
        engine.oi_eff_short_q = POS_SCALE;
    }

    let amount: u32 = kani::any();
    kani::assume(amount > 0 && amount <= 1_000_000);

    let now_slot: u64 = kani::any();
    kani::assume(now_slot >= 50 && now_slot <= 200);

    let v_before = engine.vault.get();
    let i_before = engine.insurance_fund.balance.get();
    let current_before = engine.current_slot;
    let envelope_top = engine
        .last_market_slot
        .checked_add(engine.params.max_accrual_dt_slots)
        .expect("envelope top");

    let result = engine.top_up_insurance_fund(amount as u128, now_slot);
    let should_accept = now_slot >= current_before && (!exposed || now_slot <= envelope_top);

    assert!(
        result.is_ok() == should_accept,
        "top_up acceptance must match no-accrual public path guard"
    );
    if should_accept {
        assert!(
            engine.current_slot == now_slot,
            "current_slot must be updated"
        );
        assert!(
            engine.vault.get() == v_before + amount as u128,
            "V must increase by amount"
        );
        assert!(
            engine.insurance_fund.balance.get() == i_before + amount as u128,
            "I must increase by amount"
        );
    } else {
        assert!(
            engine.current_slot == current_before,
            "rejected top_up must not advance current_slot"
        );
        assert!(
            engine.vault.get() == v_before,
            "rejected top_up must not mutate V"
        );
        assert!(
            engine.insurance_fund.balance.get() == i_before,
            "rejected top_up must not mutate I"
        );
    }
    assert!(engine.check_conservation());

    kani::cover!(
        !exposed && now_slot > envelope_top && result.is_ok(),
        "zero-OI top_up may fast-forward outside envelope"
    );
    kani::cover!(
        exposed && now_slot <= envelope_top && result.is_ok(),
        "exposed top_up accepted inside accrual envelope"
    );
    kani::cover!(
        exposed && now_slot > envelope_top && result.is_err(),
        "exposed top_up rejected outside accrual envelope"
    );
}

/// top_up_insurance_fund rejects now_slot < current_slot.
#[kani::proof]
#[kani::unwind(34)]
#[kani::solver(cadical)]
fn proof_top_up_insurance_rejects_stale_slot() {
    let mut engine = RiskEngine::new(zero_fee_params());
    engine.current_slot = 100;

    let result = engine.top_up_insurance_fund(1000, 50);
    assert!(result.is_err(), "must reject now_slot < current_slot");
}

// ############################################################################
// PROPERTY 69: Positive conversion denominator
// ############################################################################

/// Whenever flat auto-conversion consumes x > 0 released profit,
/// pnl_matured_pos_tot > 0 and h_den > 0.
/// We verify this by setting up a state with released profit and checking
/// that the haircut denominator is positive.
#[kani::proof]
#[kani::unwind(34)]
#[kani::solver(cadical)]
fn proof_positive_conversion_denominator() {
    let mut engine = RiskEngine::new(zero_fee_params());

    let idx = add_user_test(&mut engine, 0).unwrap();
    engine
        .deposit_not_atomic(idx, 1_000_000, DEFAULT_SLOT)
        .unwrap();

    // Set up matured positive PNL
    let pnl_val: u32 = kani::any();
    kani::assume(pnl_val > 0 && pnl_val <= 100_000);
    let pnl = pnl_val as i128;

    engine.set_pnl(idx as usize, pnl);
    // For released_pos to be > 0, the account must have matured PnL.
    // released_pos = pnl_matured_pos_tot contribution from this account.
    // In a flat account, after warmup, the released portion is positive.
    // We directly verify the haircut ratio:
    engine.pnl_matured_pos_tot = pnl_val as u128;

    let (h_num, h_den) = engine.haircut_ratio();
    // When pnl_matured_pos_tot > 0, h_den == pnl_matured_pos_tot > 0
    assert!(
        h_den > 0,
        "h_den must be positive when pnl_matured_pos_tot > 0"
    );
    assert!(h_num <= h_den, "h_num must not exceed h_den");
}

// ############################################################################
// PROPERTY 64: Exact trade OI decomposition
// ############################################################################

/// A valid bilateral post-trade state must decompose exactly into long and
/// short effective OI.
#[kani::proof]
#[kani::unwind(34)]
#[kani::solver(cadical)]
fn proof_bilateral_oi_decomposition() {
    let mut engine = RiskEngine::new_with_market(zero_fee_params(), DEFAULT_SLOT, DEFAULT_ORACLE);

    let a = add_user_test(&mut engine, 0).unwrap();
    let b = add_user_test(&mut engine, 0).unwrap();
    engine
        .deposit_not_atomic(a, 5_000_000, DEFAULT_SLOT)
        .unwrap();
    engine
        .deposit_not_atomic(b, 5_000_000, DEFAULT_SLOT)
        .unwrap();

    let lots: u8 = kani::any();
    kani::assume(lots > 0 && lots <= 3);
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

    // OI_long should be the sum of positive positions.
    let expected_long =
        if eff_a > 0 { eff_a as u128 } else { 0 } + if eff_b > 0 { eff_b as u128 } else { 0 };
    let expected_short = if eff_a < 0 { eff_a.unsigned_abs() } else { 0 }
        + if eff_b < 0 { eff_b.unsigned_abs() } else { 0 };

    assert!(
        engine.oi_eff_long_q == expected_long,
        "OI_long must match bilateral decomposition"
    );
    assert!(
        engine.oi_eff_short_q == expected_short,
        "OI_short must match bilateral decomposition"
    );
    assert!(
        engine.oi_eff_long_q == engine.oi_eff_short_q,
        "OI_long must equal OI_short"
    );
    assert!(engine.stored_pos_count_long == 1);
    assert!(engine.stored_pos_count_short == 1);
    assert!(engine.check_conservation());
    kani::cover!(a_is_long, "a-long bilateral state reachable");
    kani::cover!(!a_is_long, "a-short bilateral state reachable");
}

// ############################################################################
// PROPERTY 68: Partial liquidation remainder nonzero
// ############################################################################

/// Partial liquidation with 0 < q_close < abs(eff) produces nonzero remainder.
/// The proof validates the spec transition directly: a keeper-approved partial
/// close must satisfy 0 < q_close < abs(eff), restore maintenance health, keep
/// bilateral OI balanced, and leave a nonzero live remainder.
#[kani::proof]
#[kani::unwind(34)]
#[kani::solver(cadical)]
fn proof_partial_liquidation_remainder_nonzero() {
    let mut engine = RiskEngine::new_with_market(zero_fee_params(), DEFAULT_SLOT, DEFAULT_ORACLE);

    let a = add_user_test(&mut engine, 0).unwrap();
    let b = add_user_test(&mut engine, 0).unwrap();

    // Before partial: notional = 500k, MM = 25k, equity = 10k -> liquidatable.
    // After closing 400 units: remaining notional = 100k, MM = 5k, equity = 10k -> healthy.
    engine.deposit_not_atomic(a, 10_000, DEFAULT_SLOT).unwrap();

    let size = (500 * POS_SCALE) as i128;
    engine.set_position_basis_q(a as usize, size).unwrap();
    engine.set_position_basis_q(b as usize, -size).unwrap();
    engine.oi_eff_long_q = size as u128;
    engine.oi_eff_short_q = size as u128;
    assert!(
        !engine.is_above_maintenance_margin(
            &engine.accounts[a as usize],
            a as usize,
            DEFAULT_ORACLE
        ),
        "pre-partial fixture must be liquidatable"
    );

    let q_close = (400 * POS_SCALE) as u128;
    let abs_eff = engine.effective_pos_q(a as usize).unsigned_abs();
    assert!(
        q_close > 0 && q_close < abs_eff,
        "ExactPartial must be strictly smaller than the live position"
    );

    let hint = Some(LiquidationPolicy::ExactPartial(q_close));
    let validated = engine
        .validate_keeper_hint(a, size, &hint, DEFAULT_ORACLE)
        .unwrap();
    assert!(
        matches!(validated, Some(LiquidationPolicy::ExactPartial(q)) if q == q_close),
        "keeper pre-flight must approve a health-restoring partial close"
    );

    let remaining = size - q_close as i128;
    let mut post = engine.clone();
    post.attach_effective_position(a as usize, remaining)
        .unwrap();
    post.attach_effective_position(b as usize, -remaining)
        .unwrap();
    post.oi_eff_long_q = remaining as u128;
    post.oi_eff_short_q = remaining as u128;

    assert!(
        remaining != 0,
        "valid ExactPartial must leave a nonzero remainder"
    );
    assert!(
        post.effective_pos_q(a as usize) != 0,
        "post-partial effective position must remain live"
    );
    assert!(
        post.enforce_partial_liq_post_health(a as usize, DEFAULT_ORACLE)
            .is_ok(),
        "post-partial health check must pass for the selected q_close"
    );
    assert!(post.oi_eff_long_q == post.oi_eff_short_q, "OI balance");
    assert!(post.check_conservation());
}

// ############################################################################
// PROPERTY 65: Liquidation policy determinism
// ############################################################################

/// liquidate accepts only FullClose or ExactPartial; ExactPartial with
/// q_close_q == 0 or q_close_q >= abs(eff) is rejected.
#[kani::proof]
#[kani::unwind(34)]
#[kani::solver(cadical)]
fn proof_liquidation_policy_validity() {
    let mut engine = RiskEngine::new_with_market(zero_fee_params(), DEFAULT_SLOT, DEFAULT_ORACLE);

    let a = add_user_test(&mut engine, 0).unwrap();
    let b = add_user_test(&mut engine, 0).unwrap();

    let size_q = (400 * POS_SCALE) as i128;
    engine.set_position_basis_q(a as usize, size_q).unwrap();
    engine.set_position_basis_q(b as usize, -size_q).unwrap();
    engine.oi_eff_long_q = size_q as u128;
    engine.oi_eff_short_q = size_q as u128;

    let abs_eff = engine.effective_pos_q(a as usize).unsigned_abs();
    assert!(
        abs_eff == size_q as u128,
        "test account must have the configured live position"
    );
    assert!(
        !engine.is_above_maintenance_margin(
            &engine.accounts[a as usize],
            a as usize,
            DEFAULT_ORACLE
        ),
        "zero-capital positioned account must be liquidatable so policy validation is non-vacuous"
    );

    // ExactPartial(0) must fail
    let mut zero_close_engine = engine.clone();
    let r1 = zero_close_engine.liquidate_at_oracle_not_atomic(
        a,
        DEFAULT_SLOT,
        DEFAULT_ORACLE,
        LiquidationPolicy::ExactPartial(0),
        0i128,
        0,
        100,
        None,
    );
    assert!(
        r1.is_err(),
        "ExactPartial(0) must be rejected for a liquidatable account"
    );

    // ExactPartial(abs_eff) would be a full close disguised as partial and must fail.
    let mut full_size_partial_engine = engine.clone();
    let r2 = full_size_partial_engine.liquidate_at_oracle_not_atomic(
        a,
        DEFAULT_SLOT,
        DEFAULT_ORACLE,
        LiquidationPolicy::ExactPartial(abs_eff),
        0i128,
        0,
        100,
        None,
    );
    assert!(
        r2.is_err(),
        "ExactPartial(abs_eff) must be rejected; use FullClose instead"
    );

    assert!(
        engine.check_conservation(),
        "initial balanced liquidation-policy fixture is conserved"
    );
}

// ############################################################################
// PROPERTY 60: Direct fee-credit repayment cap
// ############################################################################

/// deposit_fee_credits applies exactly `min(amount, debt)`, never makes
/// fee_credits positive, and increases V and I by exactly the applied amount.
/// The wrapper must reject or refund any caller transfer above the returned
/// `pay`, so the engine must never book the excess as insurance.
#[kani::proof]
#[kani::unwind(34)]
#[kani::solver(cadical)]
fn proof_deposit_fee_credits_cap() {
    let mut engine = RiskEngine::new(zero_fee_params());

    let idx = add_user_test(&mut engine, 0).unwrap();
    engine
        .deposit_not_atomic(idx, 100_000, DEFAULT_SLOT)
        .unwrap();

    // Give fee debt
    engine.accounts[idx as usize].fee_credits = I128::new(-5000);

    let v_before = engine.vault.get();
    let i_before = engine.insurance_fund.balance.get();
    let fc_before = engine.accounts[idx as usize].fee_credits.get();

    let amount: u32 = kani::any();
    kani::assume(amount > 0 && amount <= 100_000);

    let pay = engine
        .deposit_fee_credits(idx, amount as u128, DEFAULT_SLOT)
        .expect("valid fee-credit deposit");
    let expected_pay = if amount as u128 > 5000 {
        5000
    } else {
        amount as u128
    };

    assert!(pay == expected_pay, "pay must equal min(amount, debt)");
    assert!(
        pay <= amount as u128,
        "engine must not book more than caller amount"
    );
    assert!(
        pay <= 5000,
        "engine must not book more than outstanding debt"
    );
    assert!(
        engine.accounts[idx as usize].fee_credits.get() == fc_before + pay as i128,
        "fee_credits must improve exactly by pay"
    );
    assert!(
        engine.accounts[idx as usize].fee_credits.get() <= 0,
        "fee_credits must never become positive"
    );
    assert!(
        engine.vault.get() == v_before + pay,
        "V must increase by exactly pay"
    );
    assert!(
        engine.insurance_fund.balance.get() == i_before + pay,
        "I must increase by exactly pay"
    );
    if amount as u128 > 5000 {
        assert!(
            engine.accounts[idx as usize].fee_credits.get() == 0,
            "over-deposit must clear only outstanding debt"
        );
        assert!(
            engine.vault.get() < v_before + amount as u128,
            "excess over debt must not be booked into V"
        );
        assert!(
            engine.insurance_fund.balance.get() < i_before + amount as u128,
            "excess over debt must not be booked into I"
        );
    }
}

// ############################################################################
// PROPERTY 70: Partial liquidation health check survives reset scheduling
// ############################################################################

/// Partial liquidation that closes a tiny amount MUST be rejected by the
/// mandatory post-partial health check (§9.4 step 14). Closing 1 unit out
/// of a large position at a crash price cannot restore health.
/// This proves enforcement: the health check rejects insufficient partials.
#[kani::proof]
#[kani::unwind(34)]
#[kani::solver(cadical)]
fn proof_partial_liq_health_check_mandatory() {
    let mut engine = RiskEngine::new_with_market(zero_fee_params(), DEFAULT_SLOT, DEFAULT_ORACLE);

    let a = add_user_test(&mut engine, 0).unwrap();
    let b = add_user_test(&mut engine, 0).unwrap();

    let size_q = (400 * POS_SCALE) as i128;
    engine.set_position_basis_q(a as usize, size_q).unwrap();
    engine.set_position_basis_q(b as usize, -size_q).unwrap();
    engine.oi_eff_long_q = size_q as u128;
    engine.oi_eff_short_q = size_q as u128;
    assert!(
        !engine.is_above_maintenance_margin(
            &engine.accounts[a as usize],
            a as usize,
            DEFAULT_ORACLE
        ),
        "zero-capital account with a large position must be liquidatable"
    );

    // Concrete tiny close amount. It is valid but far too small to restore
    // maintenance health, so the post-partial step-14 state must reject.
    let tiny_close = 1u128;
    assert!(tiny_close > 0 && tiny_close < engine.effective_pos_q(a as usize).unsigned_abs());
    let remaining = size_q - tiny_close as i128;
    engine
        .attach_effective_position(a as usize, remaining)
        .unwrap();
    engine
        .attach_effective_position(b as usize, -remaining)
        .unwrap();
    engine.oi_eff_long_q = remaining as u128;
    engine.oi_eff_short_q = remaining as u128;
    assert!(
        !engine.is_above_maintenance_margin(
            &engine.accounts[a as usize],
            a as usize,
            DEFAULT_ORACLE
        ),
        "post-partial remainder must still be under maintenance"
    );

    let basis_before = engine.accounts[a as usize].position_basis_q;
    let result = engine.enforce_partial_liq_post_health(a as usize, DEFAULT_ORACLE);

    // Health check at step 14 MUST reject: closing a few units out of 400M
    // position cannot restore maintenance margin.
    // Result is Err(Undercollateralized) — NOT Ok(true).
    assert!(
        matches!(result, Err(RiskError::Undercollateralized)),
        "tiny partial must be rejected by health check — remainder still unhealthy"
    );
    assert!(
        engine.accounts[a as usize].position_basis_q == basis_before,
        "post-health validation is read-only"
    );
}

// ############################################################################
// PROPERTY 42: Post-reset funding recomputation stores exactly 0
// ############################################################################

/// keeper_crank_not_atomic passes the supplied funding_rate directly to accrue_market_to.
/// v12.16.4: no stored rate field; rate is consumed directly per call.
#[kani::proof]
#[kani::unwind(34)]
#[kani::solver(cadical)]
fn proof_keeper_crank_r_last_stores_supplied_rate() {
    let mut engine = RiskEngine::new(zero_fee_params());

    let idx = add_user_test(&mut engine, 0).unwrap();
    engine
        .deposit_not_atomic(idx, 1_000_000, DEFAULT_SLOT)
        .unwrap();

    // Symbolic supplied rate bounded by the engine's configured params cap
    // (zero_fee_params sets max_abs_funding_e9_per_slot = 10^8, tighter than
    // the global MAX_ABS_FUNDING_E9_PER_SLOT = 10^9).
    let supplied_rate: i32 = kani::any();
    kani::assume(supplied_rate.unsigned_abs() as u64 <= engine.params.max_abs_funding_e9_per_slot);

    // v12.16.4: rate passed directly to accrue_market_to via keeper_crank_not_atomic
    let result = engine.keeper_crank_not_atomic(
        DEFAULT_SLOT + 1,
        DEFAULT_ORACLE,
        &[(idx, None)],
        64,
        supplied_rate as i128,
        0,
        100,
        None,
        0,
    );
    assert!(result.is_ok());
}

// ############################################################################
// PROPERTY 44: Deposit true-flat guard and latent-loss seniority
// ############################################################################

/// A deposit into an account with basis_pos_q != 0 neither routes unresolved
/// negative PnL through §7.3 nor sweeps fee debt.
/// Symbolic deposit amount and fee debt prove this for all combinations.
#[kani::proof]
#[kani::unwind(34)]
#[kani::solver(cadical)]
fn proof_deposit_nonflat_no_sweep_no_resolve() {
    let mut engine = RiskEngine::new_with_market(zero_fee_params(), DEFAULT_SLOT, DEFAULT_ORACLE);

    let a = add_user_test(&mut engine, 0).unwrap();
    let b = add_user_test(&mut engine, 0).unwrap();
    engine
        .deposit_not_atomic(a, 5_000_000, DEFAULT_SLOT)
        .unwrap();
    engine
        .deposit_not_atomic(b, 5_000_000, DEFAULT_SLOT)
        .unwrap();

    // Build the post-trade non-flat shape directly. This proof targets the
    // deposit gate, not trade matching.
    let size_q = (100 * POS_SCALE) as i128;
    engine
        .attach_effective_position(a as usize, size_q)
        .unwrap();
    engine
        .attach_effective_position(b as usize, -size_q)
        .unwrap();
    engine.oi_eff_long_q = size_q as u128;
    engine.oi_eff_short_q = size_q as u128;
    assert!(engine.accounts[a as usize].position_basis_q != 0);

    // Symbolic fee debt
    let debt: u16 = kani::any();
    kani::assume(debt >= 1 && debt <= 10_000);
    engine.accounts[a as usize].fee_credits = I128::new(-(debt as i128));
    engine.set_pnl(a as usize, -500i128).unwrap();

    let fc_before = engine.accounts[a as usize].fee_credits.get();
    let ins_before = engine.insurance_fund.balance.get();
    let vault_before = engine.vault.get();
    let c_tot_before = engine.c_tot.get();
    let cap_before = engine.accounts[a as usize].capital.get();
    let eff_before = engine.effective_pos_q(a as usize);

    // Symbolic deposit into account with open position (basis != 0)
    let dep_amount: u32 = kani::any();
    kani::assume(dep_amount >= 1 && dep_amount <= 1_000_000);
    engine
        .deposit_not_atomic(a, dep_amount as u128, DEFAULT_SLOT)
        .unwrap();

    // Deposit books the external capital transfer and settles senior losses
    // from principal, but the non-flat guard must defer fee sweep and
    // resolve_flat_negative.
    assert!(
        engine.vault.get() == vault_before + dep_amount as u128,
        "deposit must book exactly the external transfer"
    );
    assert!(
        engine.accounts[a as usize].capital.get() == cap_before + dep_amount as u128 - 500,
        "negative PnL must settle from principal"
    );
    assert!(
        engine.c_tot.get() == c_tot_before + dep_amount as u128 - 500,
        "C_tot tracks principal settlement"
    );
    assert!(
        engine.accounts[a as usize].pnl == 0,
        "loss is settled from principal, not insurance"
    );

    // fee_credits unchanged (no sweep on non-flat account)
    assert!(
        engine.accounts[a as usize].fee_credits.get() == fc_before,
        "deposit must not sweep fee debt when basis != 0"
    );

    // Insurance must not move (no resolve_flat_negative when not flat).
    assert!(
        engine.insurance_fund.balance.get() == ins_before,
        "deposit must not decrement insurance on non-flat account"
    );
    assert!(
        engine.effective_pos_q(a as usize) == eff_before,
        "deposit must not mutate effective position"
    );
    assert!(engine.check_conservation());
}

// ############################################################################
// Wave 12-M — keeper-crank funding-rate boundary harnesses (toly upstream port)
// ############################################################################

/// keeper_crank_not_atomic accepts the configured positive funding-rate
/// boundary at the production crank boundary.
#[kani::proof]
#[kani::unwind(8)]
#[kani::solver(cadical)]
fn proof_keeper_crank_accepts_positive_boundary_funding_rate_on_prod_code() {
    let mut engine = RiskEngine::new_with_market(zero_fee_params(), DEFAULT_SLOT, DEFAULT_ORACLE);
    let supplied_rate = engine.params.max_abs_funding_e9_per_slot as i128;

    let result = engine.keeper_crank_not_atomic(
        DEFAULT_SLOT + 1,
        DEFAULT_ORACLE,
        &[],
        0,
        supplied_rate,
        0,
        100,
        None,
        0,
    );
    assert!(result.is_ok());
    assert_eq!(engine.last_market_slot, DEFAULT_SLOT + 1);
    kani::cover!(
        result.is_ok() && engine.last_market_slot == DEFAULT_SLOT + 1,
        "keeper accepts positive configured funding-rate boundary"
    );
}

/// keeper_crank_not_atomic accepts the configured negative funding-rate
/// boundary at the production crank boundary.
#[kani::proof]
#[kani::unwind(8)]
#[kani::solver(cadical)]
fn proof_keeper_crank_accepts_negative_boundary_funding_rate_on_prod_code() {
    let mut engine = RiskEngine::new_with_market(zero_fee_params(), DEFAULT_SLOT, DEFAULT_ORACLE);
    let supplied_rate = -(engine.params.max_abs_funding_e9_per_slot as i128);

    let result = engine.keeper_crank_not_atomic(
        DEFAULT_SLOT + 1,
        DEFAULT_ORACLE,
        &[],
        0,
        supplied_rate,
        0,
        100,
        None,
        0,
    );
    assert!(result.is_ok());
    assert_eq!(engine.last_market_slot, DEFAULT_SLOT + 1);
    kani::cover!(
        result.is_ok() && engine.last_market_slot == DEFAULT_SLOT + 1,
        "keeper accepts negative configured funding-rate boundary"
    );
}
