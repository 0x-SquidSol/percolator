//! Shared helpers, constants, and param factories for proof files.

pub use percolator::i128::{I128, U128};
pub use percolator::wide_math::{
    ceil_div_positive_checked, fee_debt_u128_checked, floor_div_signed_conservative,
    floor_div_signed_conservative_i128, mul_div_ceil_u128, mul_div_ceil_u256, mul_div_floor_u128,
    mul_div_floor_u256, mul_div_floor_u256_with_rem, saturating_mul_u128_u64,
    saturating_mul_u256_u64, wide_mul_div_floor_u128, wide_signed_mul_div_floor,
    wide_signed_mul_div_floor_from_k_pair, I256, U256,
};
pub use percolator::*;

// ============================================================================
// Small-model constants
// ============================================================================

/// Small-model scale factors (minimal bit-widths for CBMC tractability).
/// All arithmetic stays within i32/u16 to avoid 64-bit SAT blowup.
pub const S_POS_SCALE: u16 = 4;
pub const S_ADL_ONE: u16 = 256;

// ============================================================================
// Engine constants
// ============================================================================

pub const DEFAULT_ORACLE: u64 = 1_000;
pub const DEFAULT_SLOT: u64 = 100;

// ============================================================================
// Small-model helpers
// ============================================================================

/// Small-model: eager PnL for one mark event (long).
pub fn eager_mark_pnl_long(q_base: i32, delta_p: i32) -> i32 {
    q_base * delta_p
}

/// Small-model: eager PnL for one mark event (short).
pub fn eager_mark_pnl_short(q_base: i32, delta_p: i32) -> i32 {
    -(q_base * delta_p)
}

/// Small-model: lazy PnL from K difference.
/// pnl_delta = floor(|basis_q| * (K_cur - k_snap) / (a_basis * POS_SCALE))
pub fn lazy_pnl(basis_q_abs: u16, k_diff: i32, a_basis: u16) -> i32 {
    let den = (a_basis as i32) * (S_POS_SCALE as i32);
    if den == 0 {
        return 0;
    }
    let num = (basis_q_abs as i32) * k_diff;
    if num >= 0 {
        num / den
    } else {
        let abs_num = -num;
        -((abs_num + den - 1) / den)
    }
}

/// Small-model: lazy effective quantity.
pub fn lazy_eff_q(basis_q_abs: u16, a_cur: u16, a_basis: u16) -> u16 {
    if a_basis == 0 {
        return 0;
    }
    let product = (basis_q_abs as i32) * (a_cur as i32);
    (product / (a_basis as i32)) as u16
}

/// Small-model: K update for mark event (long).
pub fn k_after_mark_long(k_before: i32, a_long: u16, delta_p: i32) -> i32 {
    k_before + (a_long as i32) * delta_p
}

/// Small-model: K update for mark event (short).
pub fn k_after_mark_short(k_before: i32, a_short: u16, delta_p: i32) -> i32 {
    k_before - (a_short as i32) * delta_p
}

/// Small-model: K update for funding event (long).
pub fn k_after_fund_long(k_before: i32, a_long: u16, delta_f: i32) -> i32 {
    k_before - (a_long as i32) * delta_f
}

/// Small-model: K update for funding event (short).
pub fn k_after_fund_short(k_before: i32, a_short: u16, delta_f: i32) -> i32 {
    k_before + (a_short as i32) * delta_f
}

/// Small-model: A update for ADL quantity shrink.
pub fn a_after_adl(a_old: u16, oi_post: u16, oi: u16) -> u16 {
    if oi == 0 {
        return a_old;
    }
    let product = (a_old as i32) * (oi_post as i32);
    (product / (oi as i32)) as u16
}

// ============================================================================
// Engine param helpers
// ============================================================================

pub fn zero_fee_params() -> RiskParams {
    // v12.19 envelope: max_price_move * max_dt + funding_budget + liq_fee <= maint_bps.
    // With maint=500, liq=0, max_rate=10_000, max_dt=100:
    //   funding_budget = 10_000 * 100 * 10_000 / 1e9 = 10 bps
    //   available for price = 490 bps
    //   max_price_move_bps_per_slot = 4 gives price_budget = 400 <= 490 ✓
    RiskParams {
        maintenance_margin_bps: 500,
        initial_margin_bps: 1000,
        max_trading_fee_bps: 0,
        max_accounts: MAX_ACCOUNTS as u64,
        liquidation_fee_bps: 0,
        liquidation_fee_cap: U128::ZERO,
        min_liquidation_abs: U128::ZERO,
        min_nonzero_mm_req: 5,
        min_nonzero_im_req: 6,
        h_min: 0,
        h_max: 100,
        resolve_price_deviation_bps: 1000,
        max_accrual_dt_slots: 100,
        max_abs_funding_e9_per_slot: 10_000,
        min_funding_lifetime_slots: 10_000_000,
        max_active_positions_per_side: MAX_ACCOUNTS as u64,
        max_price_move_bps_per_slot: 4,
    }
}

/// Test helper: shrink the account tier of `zero_fee_params` for Kani proofs
/// that need to model a small market. Mirrors toly's helper of the same name
/// (Wave 1 ENG-PORT-A harness needs `small_zero_fee_params(4)` to bound the
/// state space without changing solvency-envelope calibration).
pub fn small_zero_fee_params(max_accounts: u64) -> RiskParams {
    let mut params = zero_fee_params();
    params.max_accounts = max_accounts;
    params.max_active_positions_per_side = max_accounts;
    params
}

/// Test helper: materialize a user account via deposit_not_atomic (spec §10.2).
///
/// v12.18.1 removed add_user / add_lp / materialize_with_fee. The sole
/// materialization path is deposit with amount >= cfg_min_initial_deposit.
/// This helper picks the head of the free list and deposits the minimum.
///
/// Accepts an unused `_fee_payment` argument for mechanical migration from the
/// old `add_user_test(&mut engine, fee)` API; the engine no longer charges a fee.
pub fn add_user_test(engine: &mut RiskEngine, _fee_payment: u128) -> Result<u16> {
    let idx = engine.free_head;
    if idx == u16::MAX || (idx as usize) >= MAX_ACCOUNTS {
        return Err(RiskError::Overflow);
    }
    // Use materialize_at (test-visible back-door) to allocate a slot without
    // moving capital/vault. The public engine API only materializes via
    // deposit_not_atomic(amount >= min_initial_deposit); that spec-strict
    // path is exercised in dedicated materialization tests.
    engine.materialize_at(idx, engine.current_slot)?;
    Ok(idx)
}

/// Test helper: materialize an LP account. The engine has no LP-specific
/// materialization path under v12.18.1, so this helper materializes via
/// deposit then rewrites `kind` + matcher fields post-hoc.
pub fn add_lp_test(
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

/// Test helper: set PnL to any value, Live-mode compatible.
///
/// `RiskEngine::set_pnl` uses `ImmediateReleaseResolvedOnly` and errs for positive
/// increases in Live mode. This helper picks the right mode: UseAdmissionPair in
/// Live (routes positive PnL via admission), ImmediateRelease otherwise.
pub fn set_pnl_test(engine: &mut RiskEngine, idx: usize, new_pnl: i128) -> Result<()> {
    if new_pnl == i128::MIN {
        return engine.set_pnl(idx, new_pnl); // preserve i128::MIN rejection semantics
    }
    let old_pnl = engine.accounts[idx].pnl;
    let old_pos: u128 = if old_pnl > 0 { old_pnl as u128 } else { 0 };
    let new_pos: u128 = if new_pnl > 0 { new_pnl as u128 } else { 0 };
    let h_max = engine.params.h_max;
    if new_pos > old_pos && engine.market_mode == MarketMode::Live {
        let mut ctx = InstructionContext::new_with_admission(0, h_max);
        engine.set_pnl_with_reserve(
            idx,
            new_pnl,
            ReserveMode::UseAdmissionPair(0, h_max),
            Some(&mut ctx),
        )
    } else {
        engine.set_pnl(idx, new_pnl)
    }
}

/// Wave 12-H: seed an active stress envelope for Kani fixtures.
/// Mirrors toly `seed_active_stress_envelope` (tests/common/mod.rs:29-44).
pub fn seed_active_stress_envelope(
    engine: &mut RiskEngine,
    consumed_bps_e9: u128,
    start_slot: u64,
    remaining_indices: u64,
) {
    assert!(consumed_bps_e9 > 0);
    assert!(remaining_indices <= engine.params.max_accounts);
    if engine.current_slot < start_slot {
        engine.current_slot = start_slot;
    }
    engine.stress_consumed_bps_e9_since_envelope = consumed_bps_e9;
    engine.stress_envelope_remaining_indices = remaining_indices;
    engine.stress_envelope_start_slot = start_slot;
    engine.stress_envelope_start_generation = engine.sweep_generation;
}

/// Wave 12-H: install a canonical stored position for Kani fixtures that need
/// exact A/K/epoch snapshots not reachable through `attach_effective_position`.
/// Mirrors toly `install_position_test` (tests/common/mod.rs:222-307).
pub fn install_position_test(
    engine: &mut RiskEngine,
    idx: usize,
    basis: i128,
    a_basis: u128,
    k_snap: i128,
    epoch_snap: u64,
) -> Result<()> {
    if idx >= MAX_ACCOUNTS || !engine.is_used(idx) || basis == 0 || a_basis == 0 {
        return Err(RiskError::CorruptState);
    }
    if engine.accounts[idx].position_basis_q != 0 {
        return Err(RiskError::CorruptState);
    }
    let side = if basis > 0 { Side::Long } else { Side::Short };
    let abs_basis = basis.unsigned_abs();
    let product = abs_basis
        .checked_mul(SOCIAL_WEIGHT_SCALE)
        .ok_or(RiskError::Overflow)?;
    let q = product / a_basis;
    let r = product % a_basis;
    let weight = if r == 0 {
        q
    } else {
        q.checked_add(1).ok_or(RiskError::Overflow)?
    };
    if weight == 0 || weight > SOCIAL_LOSS_DEN {
        return Err(RiskError::Overflow);
    }

    let (epoch_side, b_current, b_epoch_start, f_side) = match side {
        Side::Long => (
            engine.adl_epoch_long,
            engine.b_long_num,
            engine.b_epoch_start_long_num,
            engine.f_long_num,
        ),
        Side::Short => (
            engine.adl_epoch_short,
            engine.b_short_num,
            engine.b_epoch_start_short_num,
            engine.f_short_num,
        ),
    };

    engine.accounts[idx].position_basis_q = basis;
    engine.accounts[idx].adl_a_basis = a_basis;
    engine.accounts[idx].adl_k_snap = k_snap;
    engine.accounts[idx].f_snap = f_side;
    engine.accounts[idx].adl_epoch_snap = epoch_snap;
    engine.accounts[idx].loss_weight = weight;
    engine.accounts[idx].b_snap = if epoch_snap == epoch_side {
        b_current
    } else {
        b_epoch_start
    };
    engine.accounts[idx].b_rem = 0;
    engine.accounts[idx].b_epoch_snap = epoch_snap;

    match side {
        Side::Long => {
            engine.stored_pos_count_long = engine
                .stored_pos_count_long
                .checked_add(1)
                .ok_or(RiskError::Overflow)?;
            if epoch_snap == engine.adl_epoch_long {
                engine.loss_weight_sum_long = engine
                    .loss_weight_sum_long
                    .checked_add(weight)
                    .ok_or(RiskError::Overflow)?;
            }
        }
        Side::Short => {
            engine.stored_pos_count_short = engine
                .stored_pos_count_short
                .checked_add(1)
                .ok_or(RiskError::Overflow)?;
            if epoch_snap == engine.adl_epoch_short {
                engine.loss_weight_sum_short = engine
                    .loss_weight_sum_short
                    .checked_add(weight)
                    .ok_or(RiskError::Overflow)?;
            }
        }
    }
    Ok(())
}

pub fn default_params() -> RiskParams {
    // v12.19 envelope: with maint=500, liq=100, max_rate=10_000, max_dt=100:
    //   funding_budget = 10_000 * 100 * 10_000 / 1e9 = 10 bps
    //   available for price = 500 - 100 - 10 = 390 bps
    //   max_price_move_bps_per_slot = 3 → price_budget = 300 <= 390 ✓
    RiskParams {
        maintenance_margin_bps: 500,
        initial_margin_bps: 1000,
        max_trading_fee_bps: 10,
        max_accounts: MAX_ACCOUNTS as u64,
        liquidation_fee_bps: 100,
        liquidation_fee_cap: U128::new(1_000_000),
        min_liquidation_abs: U128::new(0),
        min_nonzero_mm_req: 10,
        min_nonzero_im_req: 11,
        h_min: 0,
        h_max: 100,
        resolve_price_deviation_bps: 1000,
        max_accrual_dt_slots: 100,
        max_abs_funding_e9_per_slot: 10_000,
        min_funding_lifetime_slots: 10_000_000,
        max_active_positions_per_side: MAX_ACCOUNTS as u64,
        max_price_move_bps_per_slot: 3,
    }
}
