//! B6: the cross-margin `liquidation_lock` branch of `liquidate_account_not_atomic`.
//! Can an account be whittled to ZERO legs while still carrying a deficit, so that
//! `negative_pnl_account_count` never returns to 0 and the market-wide bankruptcy
//! h-lock can never be cleared?

use percolator::{
    AssetStateV16Account, AutoCrankWorkV16, EngineAssetSlotV16Account, LiquidationRequestV16,
    Market, MarketGroupV16HeaderAccount, MarketGroupV16ViewMut, PortfolioAccountV16Account,
    PortfolioV16ViewMut, ProvenanceHeaderV16, ProvenanceHeaderV16Account, TradeRequestV16,
    V16Config, V16Error, POS_SCALE,
};

const P0: u64 = 1_000_000;
const MARKET_ID: [u8; 32] = [7; 32];
const OWNER: [u8; 32] = [9; 32];

fn band_cfg(slots: u16) -> V16Config {
    let mut c = V16Config::public_user_fund_with_market_slots(slots, slots as u32, 0, 10);
    c.maintenance_margin_bps = 500;
    c.initial_margin_bps = 1_000;
    c.min_nonzero_mm_req = 10_000;
    c.min_nonzero_im_req = 20_000;
    c.liquidation_fee_bps = 50;
    c.liquidation_fee_cap = 1_000_000_000_000;
    c.min_liquidation_abs = 0;
    c.max_abs_funding_e9_per_slot = 111;
    c.max_price_move_bps_per_slot = 100;
    c.max_accrual_dt_slots = 3;
    c.min_funding_lifetime_slots = 3;
    c.max_trading_fee_bps = 100;
    c.rent_max_e9_per_slot = 23;
    c.max_bankrupt_close_lifetime_slots = 1_000_000;
    c
}

struct W2 {
    header: MarketGroupV16HeaderAccount,
    markets: Vec<Market<u64>>,
    accounts: Vec<PortfolioAccountV16Account>,
    now: u64,
}

impl W2 {
    fn new(cfg: V16Config, deposits: &[u128], n: usize) -> Self {
        cfg.validate_public_user_fund().expect("cfg validates");
        let mut header =
            MarketGroupV16HeaderAccount::new_dynamic(MARKET_ID, cfg, n as u32, 0).unwrap();
        let mut markets: Vec<Market<u64>> = (0..n)
            .map(|i| Market::new(i as u64, EngineAssetSlotV16Account::default()))
            .collect();
        for i in 0..n {
            header
                .activate_empty_asset_slot_not_atomic(
                    i as u32,
                    &mut markets[i].engine,
                    P0,
                    (i + 1) as u64,
                )
                .unwrap();
        }
        let mut accounts = Vec::new();
        for i in 0..deposits.len() {
            let mut k = [1u8; 32];
            k[0] = i as u8;
            k[1] = 1;
            let prov = ProvenanceHeaderV16Account::from_runtime(&ProvenanceHeaderV16::new(
                MARKET_ID, k, OWNER,
            ));
            let mut a = PortfolioAccountV16Account::default();
            a.init_empty_in_place(prov).unwrap();
            accounts.push(a);
        }
        let mut w = Self { header, markets, accounts, now: 1 };
        for (i, d) in deposits.iter().enumerate() {
            w.with(i, |m, a| m.deposit_not_atomic(a, *d)).unwrap();
        }
        w
    }

    fn with<R>(
        &mut self,
        i: usize,
        f: impl FnOnce(&mut MarketGroupV16ViewMut<'_, u64>, &mut PortfolioV16ViewMut<'_>) -> R,
    ) -> R {
        let mut m = MarketGroupV16ViewMut::new(&mut self.header, &mut self.markets);
        let mut a = PortfolioV16ViewMut::new(&mut self.accounts[i]);
        f(&mut m, &mut a)
    }

    fn market<R>(&mut self, f: impl FnOnce(&mut MarketGroupV16ViewMut<'_, u64>) -> R) -> R {
        let mut m = MarketGroupV16ViewMut::new(&mut self.header, &mut self.markets);
        f(&mut m)
    }

    fn asset(&self, i: usize) -> percolator::AssetStateV16 {
        self.markets[i].engine.asset.try_to_runtime().unwrap()
    }

    fn trade(&mut self, ai: usize, long: usize, short: usize, q: u128) -> Result<(), V16Error> {
        let price = self.asset(ai).effective_price;
        let (lo, hi) = if long < short { (long, short) } else { (short, long) };
        let (left, right) = self.accounts.split_at_mut(hi);
        let (a_lo, a_hi) = (&mut left[lo], &mut right[0]);
        let (l, s) = if long < short { (a_lo, a_hi) } else { (a_hi, a_lo) };
        let mut m = MarketGroupV16ViewMut::new(&mut self.header, &mut self.markets);
        let mut lv = PortfolioV16ViewMut::new(l);
        let mut sv = PortfolioV16ViewMut::new(s);
        m.execute_trade_with_fee_loss_stale_scoped_not_atomic(
            &mut lv,
            &mut sv,
            TradeRequestV16 { asset_index: ai, size_q: q as i128, exec_price: price, fee_bps: 0 },
            true,
        )
        .map(|_| ())
    }

    fn step_price(&mut self, ai: usize, target: u64) {
        self.market(|m| m.set_asset_raw_oracle_target_not_atomic(ai, target)).unwrap();
        let a = self.asset(ai);
        let cfg = self.header.config.try_to_runtime_shape().unwrap();
        let dt = (self.now - a.slot_last).min(cfg.max_accrual_dt_slots).max(1);
        let p = a.effective_price;
        let exposed = a.oi_eff_long_q != 0 || a.oi_eff_short_q != 0;
        let capped = if !exposed {
            target
        } else {
            let md =
                (p as u128 * cfg.max_price_move_bps_per_slot as u128 * dt as u128 / 10_000) as u64;
            if target > p { p + md.min(target - p) } else { p - md.min(p - target) }
        };
        let now = self.now;
        let r = self.market(|m| m.accrue_asset_to_not_atomic(ai, now, capped, 0, true));
        if r.is_err() {
            println!("    accrue a{ai} err {r:?}");
        }
        let mut a = self.asset(ai);
        a.raw_oracle_target_price = capped;
        self.markets[ai].engine.asset = AssetStateV16Account::from_runtime(&a);
    }

    fn refresh(&mut self, i: usize) -> Result<percolator::HealthCertV16, V16Error> {
        let mut last = Err(V16Error::Stale);
        for _ in 0..8 {
            last = self.with(i, |m, a| m.full_account_refresh_not_atomic(a));
            if last != Err(V16Error::Stale) {
                break;
            }
        }
        last
    }

    fn legs(&self, i: usize) -> Vec<(u32, bool)> {
        (0..4)
            .map(|s| {
                let l = self.accounts[i].legs[s].try_to_runtime().unwrap();
                (l.asset_index, l.active)
            })
            .collect()
    }
}


/// GH#292 regression. A cross-margin account in bad debt sets `liquidation_lock`,
/// and the locked branch of `liquidate_account_not_atomic` books nothing and
/// consumes no insurance -- correctly, since a multi-asset deficit has no safe
/// per-domain attribution. But before the fix it also never ESCALATED, so the
/// account could be whittled to zero legs with its deficit intact. Nothing writes
/// its PnL again, so `negative_pnl_account_count` never decrements and
/// `bankruptcy_hlock_active` -- a market-group-wide withdrawal gate -- never clears.
///
/// FAILS ON PRISTINE `3ce4cbd1`: the liquidation returns `Ok`, `mode` stays Live
/// and `recovery_reason` stays `None`.
#[test]
fn gh292_locked_liquidation_of_a_bankrupt_account_declares_recovery() {
    let q = POS_SCALE;
    let mut w = W2::new(band_cfg(2), &[260_000, 50_000_000, 50_000_000], 2);
    w.trade(0, 0, 1, q).expect("open a0");
    w.trade(1, 0, 2, q).expect("open a1");
    w.refresh(0).unwrap();

    // Walk both marks down until the account is in bad debt AND locked.
    let mut latched = false;
    for _ in 0..24 {
        w.now += 1;
        w.step_price(0, 1);
        w.step_price(1, 1);
        let _ = w.refresh(0);
        if w.accounts[0].pnl.get() < 0 && w.accounts[0].liquidation_lock != 0 {
            latched = true;
            break;
        }
    }
    assert!(latched, "precondition: account must reach bad debt with the lock set");
    assert_eq!(w.accounts[0].capital.get(), 0, "precondition: capital exhausted");
    assert_eq!(
        w.header.negative_pnl_account_count.get(),
        1,
        "precondition: the counter is armed"
    );
    assert_eq!(w.header.mode, 0, "precondition: market is still Live");
    assert!(
        w.header.recovery_reason.try_to_runtime().unwrap().is_none(),
        "precondition: no recovery declared yet"
    );

    // While the account still holds open risk, the locked branch must behave exactly
    // as before: reduce risk, charge nothing, declare nothing. This half is pinned
    // independently by v16_spec_tests.rs
    // (v16_liquidation_of_unattributed_deficit_reduces_risk_without_charging_the_surviving_domain)
    // and is asserted here so a future widening of the escalation is caught.
    let first = w
        .with(0, |m, a| {
            m.liquidate_account_not_atomic(a, LiquidationRequestV16 { asset_index: 0 })
        })
        .expect("a locked account with open risk remaining still liquidates");
    assert_eq!(first.insurance_used, 0, "the locked branch consumes no insurance");
    assert_eq!(first.residual_booked, 0, "the locked branch books no residual");
    assert_eq!(first.fee_charged, 0, "the locked branch charges no fee");
    assert_eq!(w.header.mode, 0, "no recovery while open risk remains");

    // The close that leaves the account FLAT is the one with no resolution path, so
    // it must escalate instead of silently succeeding.
    let out = w.with(0, |m, a| {
        m.liquidate_account_not_atomic(a, LiquidationRequestV16 { asset_index: 1 })
    });
    assert!(
        matches!(out, Err(V16Error::RecoveryRequired)),
        "the locked liquidation that strands a deficit on a flat account must return \
         RecoveryRequired, got {:?}",
        out.as_ref().map(|o| o.closed_q).map_err(|e| format!("{e:?}"))
    );
    assert!(
        w.legs(0).iter().all(|(_, active)| !*active),
        "precondition for the assertion above: the account must now be flat, legs={:?}",
        w.legs(0)
    );
    assert!(
        w.accounts[0].pnl.get() < 0,
        "and the deficit must still be outstanding, pnl={}",
        w.accounts[0].pnl.get()
    );
    assert_eq!(
        w.header.mode, 2,
        "the declaration must commit: market mode should be Recovery"
    );
    assert!(
        w.header.recovery_reason.try_to_runtime().unwrap().is_some(),
        "a complete Recovery marker must be left behind, or the caller's \
         kernel_commit_declared_liquidation_recovery will propagate the error"
    );
}

/// Negative control: the fix must not change the UNLOCKED path. A single-asset
/// bankrupt account has no `liquidation_lock`, so its deficit is resolved in place
/// (principal, then domain insurance, then the residual chunk) and the liquidation
/// still succeeds. Passes before and after.
#[test]
fn gh292_single_asset_bankruptcy_still_resolves_in_place() {
    let q = POS_SCALE;
    let mut w = W2::new(band_cfg(1), &[110_000, 50_000_000], 1);
    w.trade(0, 0, 1, q).expect("open a0");
    w.refresh(0).unwrap();
    // Walk until the account is actually liquidatable. Below the maintenance
    // breach the engine answers `NonProgress` (certified_liq_deficit == 0), which
    // would make this control vacuous.
    let mut liquidatable = false;
    for _ in 0..40 {
        w.now += 1;
        w.step_price(0, 1);
        if let Ok(c) = w.refresh(0) {
            if c.certified_liq_deficit != 0 {
                liquidatable = true;
                break;
            }
        }
    }
    assert!(
        liquidatable,
        "precondition: the control account must breach maintenance, else the \
         liquidation answers NonProgress and proves nothing"
    );
    assert_eq!(
        w.accounts[0].liquidation_lock, 0,
        "a single-leg account must never take the locked branch"
    );
    let out = w.with(0, |m, a| {
        m.liquidate_account_not_atomic(a, LiquidationRequestV16 { asset_index: 0 })
    });
    assert!(
        out.is_ok(),
        "the unlocked path must still resolve a single-asset bankruptcy in place, got {:?}",
        out.as_ref().map(|o| o.closed_q).map_err(|e| format!("{e:?}"))
    );
}
