//! Buyback parameters, gate-failure types, the market-exposure helper, and
//! the eligibility predicate.
//!
//! The handler's call sequence is:
//!
//! 1. Resolve and validate the target market's insurance fund and bound
//!    buyback accounts (handler crate concern; not in this module).
//! 2. [`market_exposure`] — computes a single market's risk-weighted
//!    exposure.
//! 3. [`buyback_eligible`] — runs the economic gates and returns the slice
//!    on success.
//!
//! All four buyback parameters are compile-time constants by design (see
//! PROPOSAL.md §4 and §7.5). Changing any of them requires a program
//! upgrade — there is no admin-tunable path.

// Explicit import of std's two-argument `Result` so this module's
// signatures are insulated from any single-argument `pub type Result<T>`
// alias declared at a downstream crate's lib root (a common convention
// where the crate-wide error type is folded into the `Result` name).
// Without this `use`, transferring the file into such a crate would
// fail to compile with E0107 — the parent's one-arg alias shadows the
// prelude's two-arg form, and `Result<T, BuybackBlocker>` becomes a
// wrong-arity instantiation. `Ok(_)` and `Err(_)` variant constructors
// are unaffected (they come via the prelude as values, not types).
use core::result::Result;

/// Numerator of the insurance-fund-to-exposure ratio threshold (1.5×).
///
/// Paired with [`BUYBACK_RATIO_THRESHOLD_DEN`]. The eligibility gate
/// compares `fund_balance × DEN` against `market_exposure × NUM` via
/// integer cross-multiplication; no fixed-point representation needed.
/// Equality passes (PROPOSAL.md §2.1: `≥`).
pub const BUYBACK_RATIO_THRESHOLD_NUM: u128 = 15;

/// Denominator of the insurance-fund-to-exposure ratio threshold (1.5×).
pub const BUYBACK_RATIO_THRESHOLD_DEN: u128 = 10;

/// Per-event withdrawal cap in basis points of insurance-fund balance
/// (0.1% per event — PROPOSAL.md §4 / §7.2).
pub const BUYBACK_PER_EVENT_BPS: u64 = 10;

/// Minimum spacing between buyback events, in seconds (24 hours).
pub const BUYBACK_COOLDOWN_SECS: i64 = 86_400;

/// Basis-points denominator. `value × bps / BPS_DENOMINATOR` converts a
/// bps fraction back into a value. Used by the per-market exposure formula
/// in PROPOSAL.md §3.1 (`maintenance_bps / 10_000`).
pub const BPS_DENOMINATOR: u128 = 10_000;

// Compile-time invariant: the ratio threshold must always exceed 1×.
// A misedit that produced NUM <= DEN would silently allow buybacks at
// any solvency ratio, including under-collateralized states. This
// const assertion fails the build rather than the tests.
const _: () = assert!(BUYBACK_RATIO_THRESHOLD_NUM > BUYBACK_RATIO_THRESHOLD_DEN);

// Compile-time invariant: the basis-points denominator must be non-zero
// so the per-market division `weighted / BPS_DENOMINATOR` cannot panic
// at runtime. Defense in depth alongside the existing runtime
// `bps_denominator_lock` test.
const _: () = assert!(BPS_DENOMINATOR > 0);

/// Reasons a buyback trigger may be blocked.
///
/// Each variant maps to a distinct failure mode returned by
/// [`market_exposure`] or [`buyback_eligible`]. Keeping these as an
/// enum (vs. string errors) lets callers distinguish steady-state cooldown
/// from anomalous gate failures without parsing.
///
/// Variants are ordered to match the runtime evaluation sequence: the four
/// economic gates run cheap-to-expensive inside [`buyback_eligible`].
/// `MathOverflow` sits last by convention because it is a cross-cutting
/// fail-closed bucket that any `checked_*` site can fire from, not a
/// sequenced gate.
///
/// The numeric discriminants are NOT frozen yet. They become append-only
/// — never reorder, never remove — only once this enum lands on-chain and
/// downstream services begin pinning the codes (the SDK mirror in
/// `errors/buyback.ts` carries that contract). Until transfer the order is
/// still being finalized here, so reordering — as in this staging tree —
/// is expected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuybackBlocker {
    /// Less than [`BUYBACK_COOLDOWN_SECS`] since the previous successful
    /// trigger.
    CooldownActive,
    /// Insurance fund balance is at or below the admin-set
    /// `insurance_floor`.
    BelowInsuranceFloor,
    /// The target market is currently paying haircut on positive PnL,
    /// indicating that market is in a stressed regime.
    HaircutsActive,
    /// `market_exposure_q` is zero — there is no measurable risk for the
    /// ratio gate to weigh the fund against, so "surplus relative to
    /// exposure" is undefined and the buyback must not fire. A non-zero
    /// magnitude floor, where desired, is enforced by the caller before
    /// this predicate runs.
    ExposureBelowMinimum,
    /// `fund × DEN` < `exposure × NUM`. The insurance fund is not
    /// over-collateralized enough relative to the market's current exposure.
    RatioBelowThreshold,
    /// A `checked_*` arithmetic operation returned `None` — either while
    /// computing the market's exposure or while running the cross-multiply
    /// ratio comparison. Treated as a fail-closed condition; should be
    /// unreachable in practice but defends against pathological input.
    /// Conventionally placed last as a cross-cutting bucket; do not
    /// re-sort alphabetically.
    MathOverflow,
}

/// Per-market inputs to [`market_exposure`].
///
/// Each field mirrors a name documented in PROPOSAL.md §3.1. Callers
/// resolve every field upstream — the math crate does not import oracle
/// or risk-engine code. The struct is `Copy` so callers can pass slices
/// of values without lifetime gymnastics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MarketView {
    /// Effective long open interest (Q-format), per PROPOSAL.md §3.1
    /// (`oi_eff_long_q`).
    pub oi_eff_long_q: u128,
    /// Effective short open interest (Q-format), per PROPOSAL.md §3.1
    /// (`oi_eff_short_q`).
    pub oi_eff_short_q: u128,
    /// Oracle price scaled by 1e6, pre-resolved by the caller via the
    /// matcher's per-market dispatch (Hyperp / Pyth — PROPOSAL.md §11
    /// "Oracle source"). The math crate is oracle-agnostic.
    pub oracle_price_e6: u128,
    /// Maintenance margin requirement in basis points, read directly
    /// from the slab's risk parameters (PROPOSAL.md §11
    /// "Maintenance bps source": per-market and immutable post-init).
    /// Width matches the on-chain `RiskParams::maintenance_margin_bps:
    /// u64` so the handler can pass the field through without a
    /// narrowing adapter — an `as u16` truncation could wrap a
    /// corrupted upstream value below 10 000 and silently pass the
    /// runtime check below. The `<= BPS_DENOMINATOR` invariant is
    /// enforced at runtime by [`market_exposure`] via
    /// [`BuybackBlocker::MathOverflow`].
    pub maintenance_bps: u64,
}

/// Computes a single market's risk-weighted exposure.
///
/// Implements the formula in PROPOSAL.md §3.1:
/// `(oi_eff_long_q + oi_eff_short_q) × oracle_price_e6 × maintenance_bps
/// / BPS_DENOMINATOR`. Long and short open interest are summed (not
/// netted) — a balanced book still represents real risk against the
/// market's insurance fund.
///
/// Per PROPOSAL.md §3.2 each market is evaluated independently against
/// its own insurance fund and its own exposure; there is no cross-market
/// aggregation. The handler calls this once per buyback check with the
/// target market's view — never a protocol-wide roll-up — so the input
/// is a single `MarketView`, not a slice.
///
/// All arithmetic is checked. Any overflow short-circuits with
/// [`BuybackBlocker::MathOverflow`] rather than wrapping or saturating.
/// The same variant is also returned for two precondition violations on
/// `MarketView`: (a) `maintenance_bps` exceeds [`BPS_DENOMINATOR`]
/// (10 000) — an out-of-range bps could silently inflate exposure; and
/// (b) `oracle_price_e6 == 0` — a missing or stuck oracle reading could
/// silently zero out the market's exposure, masking real risk. Both
/// preconditions are folded into the same fail-closed bucket. The
/// eligibility predicate that consumes this value treats `MathOverflow`
/// as a gate failure, so the buyback does not fire when the computation
/// lost precision or an upstream caller supplied an invalid `MarketView`.
pub fn market_exposure(market: MarketView) -> Result<u128, BuybackBlocker> {
    if market.maintenance_bps as u128 > BPS_DENOMINATOR {
        return Err(BuybackBlocker::MathOverflow);
    }
    if market.oracle_price_e6 == 0 {
        return Err(BuybackBlocker::MathOverflow);
    }
    let oi_sum = market
        .oi_eff_long_q
        .checked_add(market.oi_eff_short_q)
        .ok_or(BuybackBlocker::MathOverflow)?;
    let notional = oi_sum
        .checked_mul(market.oracle_price_e6)
        .ok_or(BuybackBlocker::MathOverflow)?;
    let weighted = notional
        .checked_mul(market.maintenance_bps as u128)
        .ok_or(BuybackBlocker::MathOverflow)?;
    Ok(weighted / BPS_DENOMINATOR)
}

/// Eligibility gate for a buyback trigger.
///
/// Runs the economic gates from PROPOSAL.md §2 in cheap-to-expensive order
/// — cooldown, insurance floor, market stress, then the exposure ratio
/// (guarded by a non-zero-exposure precondition) — and returns the slice
/// size. Eligibility is evaluated per market against that market's own
/// insurance fund and exposure.
///
/// On success, the returned slice has two regimes:
///
/// - **Proportional**: [`BUYBACK_PER_EVENT_BPS`] (10 bps) of `fund_balance`.
/// - **Clamped**: `fund_balance - insurance_floor` when the proportional
///   value would breach the floor.
///
/// Note the floor's strict-inequality asymmetry: `fund_balance ==
/// insurance_floor` fails the floor gate (PROPOSAL.md §2.3 specifies
/// `fund_balance > insurance_floor`), while `fund_balance ==
/// insurance_floor + 1` passes and may produce a slice of 0 or 1
/// depending on the proportional arm's integer rounding.
///
/// **`Ok(0)` IS A CALLER CORRECTNESS CONTRACT.** When the slice rounds
/// to zero (fund just above floor, proportional truncates), the caller
/// MUST short-circuit without stamping the cooldown timestamp.
/// PROPOSAL.md §5.1 mandates this: "no point burning a 24h slot on a
/// zero-byte event." A handler that forgets this contract will burn a
/// 24h cooldown for no economic effect.
///
/// Returns `Err(BuybackBlocker::MathOverflow)` from any `checked_*`
/// arithmetic failure. At realistic input scales this is defense-in-depth
/// — practical inputs do not approach the u128 boundary — but the
/// explicit channel keeps pathological input observable in operator logs
/// alongside the existing overflow handling in [`market_exposure`].
///
/// Trust assumptions on parameters:
///
/// - `haircut_active`: the target market's own haircut state. The math
///   crate trusts the boolean as that market's stress signal; the handler
///   passes the market's own haircut status, so a healthy market is not
///   blocked by another market's stress (the gate is per-market, matching
///   the per-market insurance fund and exposure).
/// - `now`, `last_buyback_ts`: caller supplies via Solana `Clock`. No
///   defensive sign checks; Solana timestamps post-genesis are
///   non-negative by construction.
/// - `market_exposure_q`: the target market's exposure, computed by
///   [`market_exposure`]. The Q-format suffix matches the field naming
///   in [`MarketView`].
pub fn buyback_eligible(
    fund_balance: u64,
    market_exposure_q: u128,
    last_buyback_ts: i64,
    now: i64,
    haircut_active: bool,
    insurance_floor: u64,
) -> Result<u64, BuybackBlocker> {
    // Gate 1: Cooldown — `now >= last_buyback_ts + BUYBACK_COOLDOWN_SECS`.
    let next_eligible_ts = last_buyback_ts
        .checked_add(BUYBACK_COOLDOWN_SECS)
        .ok_or(BuybackBlocker::MathOverflow)?;
    if now < next_eligible_ts {
        return Err(BuybackBlocker::CooldownActive);
    }

    // Gate 2: Floor — strict `fund_balance > insurance_floor`.
    if fund_balance <= insurance_floor {
        return Err(BuybackBlocker::BelowInsuranceFloor);
    }

    // Gate 3: Stress — no haircut active on this market.
    if haircut_active {
        return Err(BuybackBlocker::HaircutsActive);
    }

    // Gate 4: Ratio — cross-multiplied form of `fund / exposure >= 1.5`.
    // Equality passes (PROPOSAL.md §2.1: `>=`).
    //
    // Exposure precondition: a zero exposure makes the ratio degenerate —
    // `fund × DEN >= 0 × NUM` holds for any fund — which would let the
    // buyback fire on a market carrying no live risk. Require a non-zero
    // exposure so the ratio gate is meaningful.
    if market_exposure_q == 0 {
        return Err(BuybackBlocker::ExposureBelowMinimum);
    }

    // The lhs `checked_mul` is defense-in-depth; with `fund_balance: u64`
    // widened to u128 and multiplied by 10, the result cannot exceed
    // ~1.84e20, far below `u128::MAX`. The check is retained against any
    // future signature change. The rhs `checked_mul` IS reachable — a
    // pathologically large `market_exposure_q` near `u128::MAX` overflows
    // when multiplied by 15.
    let lhs = (fund_balance as u128)
        .checked_mul(BUYBACK_RATIO_THRESHOLD_DEN)
        .ok_or(BuybackBlocker::MathOverflow)?;
    let rhs = market_exposure_q
        .checked_mul(BUYBACK_RATIO_THRESHOLD_NUM)
        .ok_or(BuybackBlocker::MathOverflow)?;
    if lhs < rhs {
        return Err(BuybackBlocker::RatioBelowThreshold);
    }

    // Slice computation. The proportional arm uses `checked_mul` to bound
    // u64 overflow at the source; the clamped arm uses `saturating_sub`
    // safely because the floor gate has already established
    // `fund_balance > insurance_floor`.
    let slice_proportional = fund_balance
        .checked_mul(BUYBACK_PER_EVENT_BPS)
        .ok_or(BuybackBlocker::MathOverflow)?
        / (BPS_DENOMINATOR as u64);
    let slice_clamped = fund_balance.saturating_sub(insurance_floor);
    Ok(slice_proportional.min(slice_clamped))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Locks the cross-multiplication convention against accidental future
    /// edits. If this test ever fails, the gate's comparison direction may
    /// have silently inverted. The `NUM > DEN` invariant is locked
    /// separately by a compile-time `const _: () = assert!(...)` above.
    #[test]
    fn ratio_threshold_constants_lock() {
        assert_eq!(BUYBACK_RATIO_THRESHOLD_NUM, 15);
        assert_eq!(BUYBACK_RATIO_THRESHOLD_DEN, 10);
    }

    /// Locks the basis-points denominator at exactly 10 000. A misedit
    /// would silently scale every exposure and slice computation by
    /// orders of magnitude.
    #[test]
    fn bps_denominator_lock() {
        assert_eq!(BPS_DENOMINATOR, 10_000);
    }

    #[test]
    fn cooldown_is_24h() {
        assert_eq!(BUYBACK_COOLDOWN_SECS, 86_400);
        assert_eq!(BUYBACK_COOLDOWN_SECS / 3600, 24);
    }

    #[test]
    fn per_event_bps_is_10() {
        assert_eq!(BUYBACK_PER_EVENT_BPS, 10);
    }

    fn sample_market(long: u128, short: u128, price: u128, bps: u64) -> MarketView {
        MarketView {
            oi_eff_long_q: long,
            oi_eff_short_q: short,
            oracle_price_e6: price,
            maintenance_bps: bps,
        }
    }

    #[test]
    fn exposure_single_market_exact_value() {
        // (1_000 + 500) × 100 × 500 / 10_000 = 7_500
        let m = sample_market(1_000, 500, 100, 500);
        assert_eq!(market_exposure(m), Ok(7_500));
    }

    #[test]
    fn exposure_long_only_market_works() {
        // (10_000 + 0) × 1 × 500 / 10_000 = 500
        let m = sample_market(10_000, 0, 1, 500);
        assert_eq!(market_exposure(m), Ok(500));
    }

    #[test]
    fn exposure_zero_maintenance_bps_yields_zero() {
        // bps = 0 ⇒ zero exposure even with large OI and non-zero price.
        let m_zero_bps = sample_market(1_000_000, 1_000_000, 1_000, 0);
        assert_eq!(market_exposure(m_zero_bps), Ok(0));
    }

    #[test]
    fn exposure_overflow_returns_math_overflow_err() {
        // Path 1: checked_add overflow on (long + short).
        let m_add_overflow = sample_market(u128::MAX, 1, 1, 1);
        assert_eq!(
            market_exposure(m_add_overflow),
            Err(BuybackBlocker::MathOverflow),
        );

        // Path 2: checked_mul overflow on (oi_sum × price).
        // oi_sum = (u128::MAX / 2) + 0 = u128::MAX / 2.
        // price = 4 ⇒ oi_sum × 4 overflows.
        let m_mul_overflow = sample_market(u128::MAX / 2, 0, 4, 1);
        assert_eq!(
            market_exposure(m_mul_overflow),
            Err(BuybackBlocker::MathOverflow),
        );

        // Path 3: checked_mul overflow on (notional × maintenance_bps).
        // long = u128::MAX / 9_999, short = 0, price = 1, bps = 10_000.
        // oi_sum = u128::MAX / 9_999. notional = oi_sum × 1 = oi_sum.
        // notional × 10_000 > u128::MAX since (u128::MAX / 9_999) × 10_000
        // exceeds u128::MAX.
        let m_bps_overflow = sample_market(u128::MAX / 9_999, 0, 1, 10_000);
        assert_eq!(
            market_exposure(m_bps_overflow),
            Err(BuybackBlocker::MathOverflow),
        );
    }

    #[test]
    fn exposure_out_of_range_maintenance_bps_returns_err() {
        // bps = 10_001 violates the BPS_DENOMINATOR upper bound. The
        // function rejects it before any arithmetic runs, returning
        // MathOverflow as a fail-closed precondition violation.
        let m = sample_market(1_000, 1_000, 100, 10_001);
        assert_eq!(market_exposure(m), Err(BuybackBlocker::MathOverflow));

        // Boundary: bps = BPS_DENOMINATOR (10_000) is accepted.
        let m_boundary = sample_market(1_000, 0, 1, 10_000);
        // 1_000 × 1 × 10_000 / 10_000 = 1_000.
        assert_eq!(market_exposure(m_boundary), Ok(1_000));
    }

    #[test]
    fn exposure_zero_oracle_price_returns_err() {
        // oracle_price_e6 = 0 violates the non-zero precondition. The
        // function rejects it before any arithmetic runs, returning
        // MathOverflow as a fail-closed precondition violation. A
        // missing or stuck oracle would otherwise silently zero out
        // the market's exposure, masking real risk.
        let m = sample_market(1_000, 0, 0, 500);
        assert_eq!(market_exposure(m), Err(BuybackBlocker::MathOverflow));
    }

    // ---------------- buyback_eligible — happy paths ----------------

    #[test]
    fn predicate_all_gates_pass_proportional_slice() {
        // fund=1_000_000, exposure=500_000 → ratio = 2.0 (≥ 1.5).
        // proportional = 1_000_000 × 10 / 10_000 = 1_000.
        // clamped = 1_000_000 - 100_000 = 900_000.
        // min = 1_000.
        assert_eq!(
            buyback_eligible(1_000_000, 500_000, 0, 100_000, false, 100_000),
            Ok(1_000),
        );
    }

    #[test]
    fn predicate_clamped_slice_arm() {
        // fund just above floor; clamped < proportional.
        // fund = 100_005, floor = 100_000.
        // proportional = 100_005 × 10 / 10_000 = 100.
        // clamped = 5.
        // min = 5.
        // exposure = 50_000 → ratio = 2.0001 (≥ 1.5).
        assert_eq!(
            buyback_eligible(100_005, 50_000, 0, 100_000, false, 100_000),
            Ok(5),
        );
    }

    #[test]
    fn predicate_zero_slice_when_proportional_rounds_to_zero() {
        // fund × BPS / DENOM rounds to 0 when fund × BPS < DENOM.
        // fund = 100, floor = 99 → proportional = 0, clamped = 1, min = 0.
        // exposure = 60 → ratio = 1.667 (≥ 1.5).
        assert_eq!(buyback_eligible(100, 60, 0, 100_000, false, 99), Ok(0),);
    }

    // ---------------- buyback_eligible — gate failures ----------------

    #[test]
    fn predicate_cooldown_active() {
        let last_ts: i64 = 1_000_000;
        let now: i64 = last_ts + BUYBACK_COOLDOWN_SECS - 1;
        assert_eq!(
            buyback_eligible(1_000_000, 500_000, last_ts, now, false, 100_000),
            Err(BuybackBlocker::CooldownActive),
        );
    }

    #[test]
    fn predicate_cooldown_at_boundary_passes() {
        // now == last + COOLDOWN_SECS exactly → passes (≥ not >).
        let last_ts: i64 = 1_000_000;
        let now: i64 = last_ts + BUYBACK_COOLDOWN_SECS;
        assert_eq!(
            buyback_eligible(1_000_000, 500_000, last_ts, now, false, 100_000),
            Ok(1_000),
        );
    }

    #[test]
    fn predicate_last_ts_zero_passes_cooldown() {
        // last_ts = 0 means never fired. Cooldown trivially passes since
        // 0 + 86_400 = 86_400, far below realistic Solana clock.
        assert_eq!(
            buyback_eligible(1_000_000, 500_000, 0, 100_000, false, 100_000),
            Ok(1_000),
        );
    }

    #[test]
    fn predicate_floor_equal_blocked() {
        // fund == floor → fails (strict `>` per PROPOSAL.md §2.3).
        assert_eq!(
            buyback_eligible(100_000, 500_000, 0, 100_000, false, 100_000),
            Err(BuybackBlocker::BelowInsuranceFloor),
        );
    }

    #[test]
    fn predicate_floor_plus_one_passes() {
        // fund = floor + 1 → passes; slice clamped to 1.
        // proportional = 100_001 × 10 / 10_000 = 100.
        // clamped = 1.
        // min = 1.
        assert_eq!(
            buyback_eligible(100_001, 50_000, 0, 100_000, false, 100_000),
            Ok(1),
        );
    }

    #[test]
    fn predicate_haircut_active_blocked() {
        assert_eq!(
            buyback_eligible(1_000_000, 500_000, 0, 100_000, true, 100_000),
            Err(BuybackBlocker::HaircutsActive),
        );
    }

    #[test]
    fn predicate_ratio_below_threshold() {
        // fund=1_000_000, exposure=700_000 → ratio ≈ 1.43 (< 1.5).
        // 1_000_000 × 10 = 10_000_000.
        // 700_000 × 15 = 10_500_000.
        // lhs < rhs → fail.
        assert_eq!(
            buyback_eligible(1_000_000, 700_000, 0, 100_000, false, 100_000),
            Err(BuybackBlocker::RatioBelowThreshold),
        );
    }

    #[test]
    fn predicate_ratio_exactly_at_threshold_passes() {
        // fund × 10 == exposure × 15 → ratio = 1.5 exactly. Equality
        // passes (PROPOSAL.md §2.1: `≥`).
        // fund = 15_000, exposure = 10_000.
        // 15_000 × 10 = 150_000 = 10_000 × 15. lhs ≥ rhs.
        // proportional = 15_000 × 10 / 10_000 = 15.
        // clamped = 15_000 - 100 = 14_900.
        // min = 15.
        assert_eq!(
            buyback_eligible(15_000, 10_000, 0, 100_000, false, 100),
            Ok(15),
        );
    }

    #[test]
    fn predicate_zero_exposure_blocked() {
        // exposure = 0 → no measurable risk; the ratio gate would be
        // degenerate, so the buyback must not fire.
        assert_eq!(
            buyback_eligible(1_000_000, 0, 0, 100_000, false, 100_000),
            Err(BuybackBlocker::ExposureBelowMinimum),
        );
    }

    #[test]
    fn predicate_minimal_nonzero_exposure_passes() {
        // exposure = 1 (minimal non-zero) clears the precondition; with a
        // large fund the ratio gate still passes.
        // proportional = 1_000_000 × 10 / 10_000 = 1_000.
        assert_eq!(
            buyback_eligible(1_000_000, 1, 0, 100_000, false, 100_000),
            Ok(1_000),
        );
    }

    #[test]
    fn predicate_gate_ordering_haircut_before_exposure() {
        // The haircut gate fires before the exposure precondition: with
        // exposure = 0 and haircut active, the returned variant must be
        // HaircutsActive, not ExposureBelowMinimum.
        assert_eq!(
            buyback_eligible(1_000_000, 0, 0, 100_000, true, 100_000),
            Err(BuybackBlocker::HaircutsActive),
        );
    }

    // ---------------- buyback_eligible — overflow paths ----------------

    #[test]
    fn predicate_cooldown_addition_overflow() {
        // last_buyback_ts at i64::MAX → checked_add(COOLDOWN_SECS) overflows.
        // The function returns MathOverflow before any other check.
        assert_eq!(
            buyback_eligible(1, 0, i64::MAX, 0, false, 0),
            Err(BuybackBlocker::MathOverflow),
        );
    }

    #[test]
    fn predicate_rhs_cross_multiply_overflow() {
        // exposure × 15 overflows when exposure > u128::MAX / 15.
        // u128::MAX / 14 × 15 > u128::MAX → overflows on rhs checked_mul.
        // (lhs path is unreachable from u64 fund_balance — see function
        // doc-comment.)
        assert_eq!(
            buyback_eligible(1_000_000, u128::MAX / 14, 0, 100_000, false, 100_000,),
            Err(BuybackBlocker::MathOverflow),
        );
    }

    #[test]
    fn predicate_slice_multiply_overflow() {
        // fund × BPS_PER_EVENT (10) overflows u64 when fund > u64::MAX/10.
        // fund = u64::MAX, exposure small enough that ratio passes.
        // u64::MAX × 10 wraps in u64 → checked_mul returns None.
        assert_eq!(
            buyback_eligible(u64::MAX, 1_000, 0, 100_000, false, 0),
            Err(BuybackBlocker::MathOverflow),
        );
    }

    // ---------------- buyback_eligible — gate ordering ----------------

    #[test]
    fn predicate_gate_ordering_returns_cheapest_failure() {
        // Both cooldown AND ratio fail. Cooldown is checked first
        // (cheap-to-expensive ordering per PROPOSAL.md §2), so the
        // returned error must be CooldownActive, not RatioBelowThreshold.
        let last_ts: i64 = 1_000_000;
        let now: i64 = last_ts + BUYBACK_COOLDOWN_SECS - 1; // cooldown fails
                                                            // exposure high enough that ratio also fails: 1_000_000 / 700_000 ≈ 1.43
        assert_eq!(
            buyback_eligible(1_000_000, 700_000, last_ts, now, false, 100_000),
            Err(BuybackBlocker::CooldownActive),
        );
    }

    #[test]
    fn predicate_negative_last_ts_passes() {
        // Defensive coverage: Solana Clock timestamps are non-negative
        // by construction, but the i64 type allows negatives. Cooldown
        // arithmetic via checked_add handles them without panic.
        // last_ts = -1_000, now = 100_000.
        // -1_000 + 86_400 = 85_400. now > 85_400 → cooldown passes.
        // proportional = 1_000_000 × 10 / 10_000 = 1_000.
        // clamped = 900_000. min = 1_000.
        assert_eq!(
            buyback_eligible(1_000_000, 500_000, -1_000, 100_000, false, 100_000),
            Ok(1_000),
        );
    }

    #[test]
    fn predicate_now_before_last_ts_blocked() {
        // Defensive coverage: caller's "now" is earlier than
        // last_buyback_ts (clock drift / replayed state).
        // last_ts = 100_000, now = 50_000.
        // last + 86_400 = 186_400. 50_000 < 186_400 → CooldownActive.
        assert_eq!(
            buyback_eligible(1_000_000, 500_000, 100_000, 50_000, false, 100_000),
            Err(BuybackBlocker::CooldownActive),
        );
    }

    #[test]
    fn predicate_gate_ordering_floor_before_ratio() {
        // Cheap-to-expensive ordering: floor gate fires before ratio
        // gate. fund == floor → BelowInsuranceFloor (strict > per §2.3).
        // exposure = 700_000 would also fail the ratio check if reached.
        assert_eq!(
            buyback_eligible(100_000, 700_000, 0, 100_000, false, 100_000),
            Err(BuybackBlocker::BelowInsuranceFloor),
        );
    }

    #[test]
    fn predicate_gate_ordering_haircut_before_ratio() {
        // Cheap-to-expensive ordering: haircut gate fires before ratio
        // gate. haircut_active = true → HaircutsActive.
        // exposure = 700_000 would also fail the ratio check if reached.
        assert_eq!(
            buyback_eligible(1_000_000, 700_000, 0, 100_000, true, 100_000),
            Err(BuybackBlocker::HaircutsActive),
        );
    }

    #[test]
    fn predicate_gate_ordering_floor_before_haircut() {
        // Cheap-to-expensive ordering: floor gate fires before haircut
        // gate. Both would fail simultaneously: fund == floor and
        // haircut_active = true. Floor is checked first, so the
        // returned variant must be BelowInsuranceFloor, not HaircutsActive.
        // (Cooldown trivially passes: 0 + 86_400 < 100_000.)
        assert_eq!(
            buyback_eligible(100_000, 500_000, 0, 100_000, true, 100_000),
            Err(BuybackBlocker::BelowInsuranceFloor),
        );
    }
}
