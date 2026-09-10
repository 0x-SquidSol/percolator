//! test(#143) — a stressed junior winner cannot drain a funded senior.
//!
//! Issue #143 ("auto-minted provider backing → junior-profit seniority bypass")
//! describes a winner, paid out of a settled loser's recycled principal (which the
//! engine books as `source_fresh_backing`), extracting nominal profit ahead of the
//! insurance / socialized-loss stack under total stress (`residual == 0`).
//!
//! The mechanism is present — a winner with source claims realizes support via the
//! source path (`account_source_realizable_support`), which does not apply the
//! `residual()` haircut the non-source branch applies — but it is INERT, held closed
//! by two structural invariants:
//!
//!   1. `validate_header_aggregate_totals` checks `c_tot + insurance + earnings <=
//!      vault` FIRST, then adds `source_fresh_backing` as the most-junior claim — so
//!      the recycled loser principal is subordinate to insurance, at every mutator.
//!   2. a winner realizes only backing-limited support, so under `residual == 0` it
//!      cannot reach atoms that back a funded senior.
//!
//! These tests pin that property so a future change to the settlement / source-
//! backing / aggregate-total code that weakens either guard is caught here before it
//! merges. Everything is driven through REAL engine mutators — deposit, the matched
//! trade, the permissionless Refresh crank (which crashes the mark and settles
//! lazily), deposit_domain_insurance, resolve, and close_resolved. No pnl / capital /
//! backing / insurance field is hand-poked.
//!
//! Run: cargo test --features fork-facade --test no_seniority_bypass_143 -- --nocapture

use percolator::{
    EngineAssetSlotV16Account, Market, MarketGroupV16HeaderAccount, MarketGroupV16ViewMut,
    PermissionlessCrankActionV16, PermissionlessCrankRequestV16, PermissionlessProgressOutcomeV16,
    PortfolioAccountV16Account, PortfolioV16ViewMut, ProvenanceHeaderV16,
    ProvenanceHeaderV16Account, ResolvedCloseOutcomeV16, TradeRequestV16, V16Config,
};
use percolator::{BOUND_SCALE, POS_SCALE};

const PX0: u64 = 1_000_000;
const PX_CRASH: u64 = 100_000; // 90% crash — within max_price_move_bps_per_slot = 10_000 (100%/slot)
const DEP: u128 = 1_000_000;

fn market_id_bytes() -> [u8; 32] {
    [1; 32]
}

fn market_fixture(init_price: u64) -> (MarketGroupV16HeaderAccount, Vec<Market<u64>>) {
    let cfg = V16Config::public_user_fund_with_market_slots(1, 1, 0, 10);
    let mut header = MarketGroupV16HeaderAccount::new_dynamic(market_id_bytes(), cfg, 1, 0).unwrap();
    let mut markets = vec![Market::new(0u64, EngineAssetSlotV16Account::default())];
    header
        .activate_empty_asset_slot_not_atomic(0, &mut markets[0].engine, init_price, 1)
        .unwrap();
    {
        let view = MarketGroupV16ViewMut::new(&mut header, &mut markets);
        view.validate_shape().unwrap();
    }
    (header, markets)
}

fn account_fixture(seed: u8) -> PortfolioAccountV16Account {
    let header = ProvenanceHeaderV16Account::from_runtime(&ProvenanceHeaderV16::new(
        market_id_bytes(),
        [seed; 32],
        [9u8; 32],
    ));
    let mut account = PortfolioAccountV16Account::default();
    account.init_empty_in_place(header).unwrap();
    account
}

fn signed_q(q: u128) -> i128 {
    i128::try_from(q).unwrap()
}

/// residual() replicated from the engine: vault − (c_tot + insurance + earnings + source_fresh_backing).
fn residual(m: &MarketGroupV16ViewMut<'_, u64>) -> u128 {
    let senior = m.header.c_tot.get()
        + m.header.insurance.get()
        + m.header.backing_provider_earnings_total.get()
        + m.header.source_fresh_backing_total_num.get() / BOUND_SCALE;
    m.header.vault.get().saturating_sub(senior)
}

/// One permissionless Refresh crank at `price`, which crashes the mark and settles lazily.
fn refresh(
    m: &mut MarketGroupV16ViewMut<'_, u64>,
    a: &mut PortfolioV16ViewMut<'_>,
    slot: u64,
    price: u64,
) -> PermissionlessProgressOutcomeV16 {
    m.permissionless_crank_not_atomic(
        a,
        PermissionlessCrankRequestV16 {
            now_slot: slot,
            asset_index: 0,
            effective_price: price,
            funding_rate_e9: 0,
            action: PermissionlessCrankActionV16::Refresh,
        },
    )
    .unwrap_or_else(|e| panic!("refresh crank @price={price} slot={slot} failed: {e:?}"))
}

fn close_resolved_to_completion(
    m: &mut MarketGroupV16ViewMut<'_, u64>,
    a: &mut PortfolioV16ViewMut<'_>,
) -> u128 {
    for _ in 0..64 {
        match m.close_resolved_account_not_atomic(a, 0) {
            Ok(ResolvedCloseOutcomeV16::Closed { payout }) => return payout,
            Ok(ResolvedCloseOutcomeV16::ProgressOnly) => continue,
            Err(e) => panic!("close_resolved errored: {e:?}"),
        }
    }
    panic!("close_resolved did not finish in 64 iters");
}

/// A (long, loser) and B (short, winner) each deposit `DEP`; optionally fund a genuine
/// senior with `insurance` domain-insurance atoms; open a matched 1-lot; crash the mark
/// via one Refresh per account. Returns the settled market and both accounts.
fn build_settled_pair(
    insurance: u128,
) -> (
    MarketGroupV16HeaderAccount,
    Vec<Market<u64>>,
    PortfolioAccountV16Account,
    PortfolioAccountV16Account,
) {
    let (mut header, mut markets) = market_fixture(PX0);
    let mut a_h = account_fixture(91); // A = long = loser
    let mut b_h = account_fixture(92); // B = short = winner
    {
        let mut m = MarketGroupV16ViewMut::new(&mut header, &mut markets);
        {
            let mut a = PortfolioV16ViewMut::new(&mut a_h);
            m.deposit_not_atomic(&mut a, DEP).unwrap();
        }
        {
            let mut b = PortfolioV16ViewMut::new(&mut b_h);
            m.deposit_not_atomic(&mut b, DEP).unwrap();
        }
        if insurance > 0 {
            m.deposit_domain_insurance_not_atomic(0, insurance).unwrap();
        }
        {
            let mut a = PortfolioV16ViewMut::new(&mut a_h);
            let mut b = PortfolioV16ViewMut::new(&mut b_h);
            m.execute_trade_with_fee_loss_stale_scoped_not_atomic(
                &mut a,
                &mut b,
                TradeRequestV16 {
                    asset_index: 0,
                    size_q: signed_q(POS_SCALE),
                    exec_price: PX0,
                    fee_bps: 0,
                },
                true,
            )
            .expect("A long / B short matched trade");
        }
        {
            let mut a = PortfolioV16ViewMut::new(&mut a_h);
            refresh(&mut m, &mut a, 2, PX_CRASH);
        }
        {
            let mut b = PortfolioV16ViewMut::new(&mut b_h);
            refresh(&mut m, &mut b, 3, PX_CRASH);
        }
    }
    (header, markets, a_h, b_h)
}

/// #143's precondition, as a sanity anchor: the crash settles the loser from principal
/// and the winner realizes a nominal gain with NO warmup, and the loser's principal is
/// recycled into `source_fresh_backing` at its close. If this stops holding the two
/// invariant tests below no longer exercise #143's mechanism — so pin it.
#[test]
fn matched_settle_recycles_loser_principal_as_source_backing() {
    let (mut header, mut markets, mut a_h, mut b_h) = build_settled_pair(0);
    let mut m = MarketGroupV16ViewMut::new(&mut header, &mut markets);

    {
        let a = PortfolioV16ViewMut::new(&mut a_h);
        let b = PortfolioV16ViewMut::new(&mut b_h);
        assert_eq!(a.header.pnl.get(), 0, "loser loss settled from principal");
        assert_eq!(b.header.pnl.get(), 900_000, "winner nominal gain realized");
        assert_eq!(b.header.reserved_pnl.get(), 0, "no warmup engaged on the winner's claim");
    }
    assert_eq!(residual(&m), 0, "the book is at total stress: residual == 0");

    m.resolve_market_not_atomic(4).unwrap();
    let _ = close_resolved_to_completion(&mut m, &mut PortfolioV16ViewMut::new(&mut a_h));
    assert_eq!(
        m.header.source_fresh_backing_total_num.get() / BOUND_SCALE,
        900_000,
        "loser principal recycled into source_fresh_backing (the #143 auto-mint)"
    );
}

/// INVARIANT 1 — under total stress with a funded insurance senior, a junior winner's
/// full extraction drains the senior by exactly 0. This is the #143 property.
#[test]
fn stressed_winner_cannot_drain_funded_senior() {
    const INSURANCE: u128 = 500_000;
    let (mut header, mut markets, mut a_h, mut b_h) = build_settled_pair(INSURANCE);
    let mut m = MarketGroupV16ViewMut::new(&mut header, &mut markets);

    assert_eq!(residual(&m), 0, "precondition: total stress (residual == 0) with the senior present");

    m.resolve_market_not_atomic(4).unwrap();
    let _ = close_resolved_to_completion(&mut m, &mut PortfolioV16ViewMut::new(&mut a_h));

    let insurance_before = m.header.insurance.get();
    let _ = close_resolved_to_completion(&mut m, &mut PortfolioV16ViewMut::new(&mut b_h));
    let insurance_after = m.header.insurance.get();

    assert_eq!(
        insurance_before.saturating_sub(insurance_after),
        0,
        "#143: a junior winner must not extract any atoms from the funded senior"
    );
    assert_eq!(m.header.insurance.get(), INSURANCE, "the senior is left byte-for-byte intact");
    assert_eq!(m.header.vault.get(), INSURANCE, "the only atoms left in the vault are the senior's");
    m.validate_shape().expect("aggregate conservation holds");
}

/// INVARIANT 2 — the winner's junior profit is inert in Live: it cannot be converted
/// to capital or withdrawn before the loser is closed (the recycling that would back
/// the claim only happens at the loser's close, and validate_with_market gates the
/// open-leg account). So there is no Live route to reach the senior either.
#[test]
fn winner_junior_profit_is_inert_in_live_before_loser_close() {
    let (mut header, mut markets, _a_h, mut b_h) = build_settled_pair(500_000);
    let mut m = MarketGroupV16ViewMut::new(&mut header, &mut markets);

    let mut b = PortfolioV16ViewMut::new(&mut b_h);
    assert_eq!(b.header.pnl.get(), 900_000, "winner has a realized nominal gain in Live");
    assert!(
        m.convert_released_pnl_to_capital_not_atomic(&mut b).is_err(),
        "junior profit is not convertible to capital in Live before the loser closes"
    );
    assert!(
        m.withdraw_not_atomic(&mut b, 1).is_err(),
        "the account cannot withdraw while its leg is open / unsettled"
    );
    assert_eq!(m.header.insurance.get(), 500_000, "senior untouched by the Live attempts");
}
