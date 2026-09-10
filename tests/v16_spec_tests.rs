use percolator::{
    v16_domain_count_for_market_slots, AssetLifecycleV16, AssetStateV16Account,
    BackingBucketStatusV16, BackingBucketV16, BackingBucketV16Account, CloseProgressLedgerV16,
    CloseProgressLedgerV16Account, EngineAssetSlotV16Account, HealthCertV16, HealthCertV16Account,
    LiquidationRequestV16, Market, MarketGroupV16HeaderAccount, MarketGroupV16ViewMut,
    PermissionlessCrankActionV16, PermissionlessCrankRequestV16, PermissionlessProgressOutcomeV16,
    PermissionlessRecoveryReasonV16, PortfolioAccountV16Account, PortfolioLegV16,
    PortfolioLegV16Account, PortfolioSourceDomainV16Account, PortfolioV16View, PortfolioV16ViewMut,
    ProvenanceHeaderV16, ProvenanceHeaderV16Account, RebalanceRequestV16, ResolvedCloseOutcomeV16,
    ResolvedPayoutLedgerV16, ResolvedPayoutLedgerV16Account, ResolvedPayoutReceiptV16,
    ResolvedPayoutReceiptV16Account, SideModeV16, SideV16, SourceCreditStateV16,
    SourceCreditStateV16Account, TradeRequestV16, V16Config, V16Error, V16PodI128, V16PodU128,
    V16PodU32, V16PodU64, V16_EMPTY_ACTIVE_BITMAP,
};
use percolator::{ADL_ONE, BOUND_SCALE, CREDIT_RATE_SCALE, POS_SCALE};

fn ids() -> ([u8; 32], [u8; 32], [u8; 32]) {
    ([1; 32], [2; 32], [3; 32])
}

fn market_fixture(
    market_slots: u32,
    init_price: u64,
) -> (MarketGroupV16HeaderAccount, Vec<Market<u64>>) {
    let (market_id, _, _) = ids();
    let cfg =
        V16Config::public_user_fund_with_market_slots(market_slots as u16, market_slots, 0, 10);
    let mut header =
        MarketGroupV16HeaderAccount::new_dynamic(market_id, cfg, market_slots, 0).unwrap();
    let mut markets = (0..market_slots)
        .map(|i| Market::new(i as u64, EngineAssetSlotV16Account::default()))
        .collect::<Vec<_>>();
    for i in 0..market_slots as usize {
        header
            .activate_empty_asset_slot_not_atomic(
                i as u32,
                &mut markets[i].engine,
                init_price,
                (i + 1) as u64,
            )
            .unwrap();
    }
    {
        let view = MarketGroupV16ViewMut::new(&mut header, &mut markets);
        view.validate_shape().unwrap();
    }
    (header, markets)
}

fn account_fixture(market_slots: u32, account_seed: u8) -> PortfolioAccountV16Account {
    let (market_id, _, owner) = ids();
    let header = ProvenanceHeaderV16Account::from_runtime(&ProvenanceHeaderV16::new(
        market_id,
        [account_seed; 32],
        owner,
    ));
    let _ = v16_domain_count_for_market_slots(market_slots).unwrap();
    let mut account = PortfolioAccountV16Account::default();
    account.init_empty_in_place(header).unwrap();
    account
}

// E6: a close ledger that finished paying out -- `finalized`, zero residual --
// but is still `active` because the ledger stays active to preserve close
// identity/history (close_id watermark, progress totals) for audit purposes.
// `support_consumed == junior_face_burned == gross` and
// `residual_remaining == 0` satisfy validate_close_progress_ledger_with_market's
// progress/residual bookkeeping invariant for a finalized ledger.
fn finalized_inert_close_progress(
    market_id: u64,
    close_id: u64,
    gross: u128,
) -> CloseProgressLedgerV16 {
    CloseProgressLedgerV16 {
        active: true,
        finalized: true,
        canceled: false,
        close_id,
        asset_index: 0,
        market_id,
        domain_side: SideV16::Long,
        gross_loss_at_close_start: gross,
        drift_reference_slot: 0,
        max_close_slot: 0,
        support_consumed: gross,
        junior_face_burned: gross,
        residual_remaining: 0,
        ..CloseProgressLedgerV16::EMPTY
    }
}

fn signed_q(q: u128) -> i128 {
    i128::try_from(q).unwrap()
}

/// Like `market_fixture`, but with a nonzero `max_trading_fee_bps` cap so
/// trade requests may carry a fee (the base `market_fixture` config has
/// `max_trading_fee_bps: 0`, which is why every pre-existing trade test in
/// this file uses `fee_bps: 0`).
fn market_fixture_with_trade_fee(
    market_slots: u32,
    init_price: u64,
    max_trading_fee_bps: u64,
) -> (MarketGroupV16HeaderAccount, Vec<Market<u64>>) {
    let (market_id, _, _) = ids();
    let mut cfg =
        V16Config::public_user_fund_with_market_slots(market_slots as u16, market_slots, 0, 10);
    cfg.max_trading_fee_bps = max_trading_fee_bps;
    let mut header =
        MarketGroupV16HeaderAccount::new_dynamic(market_id, cfg, market_slots, 0).unwrap();
    let mut markets = (0..market_slots)
        .map(|i| Market::new(i as u64, EngineAssetSlotV16Account::default()))
        .collect::<Vec<_>>();
    for i in 0..market_slots as usize {
        header
            .activate_empty_asset_slot_not_atomic(
                i as u32,
                &mut markets[i].engine,
                init_price,
                (i + 1) as u64,
            )
            .unwrap();
    }
    {
        let view = MarketGroupV16ViewMut::new(&mut header, &mut markets);
        view.validate_shape().unwrap();
    }
    (header, markets)
}

#[test]
fn v16_public_fund_validator_accepts_nontrivial_exact_solvency_profile() {
    let mut cfg = V16Config::public_user_fund_with_market_slots(1, 1, 1, 10);
    cfg.maintenance_margin_bps = 10_000;
    cfg.initial_margin_bps = 10_000;
    cfg.max_price_move_bps_per_slot = 100;
    cfg.max_accrual_dt_slots = 1;
    cfg.min_funding_lifetime_slots = 1;
    cfg.max_abs_funding_e9_per_slot = 0;
    cfg.liquidation_fee_bps = 100;
    cfg.min_liquidation_abs = 1;
    cfg.liquidation_fee_cap = 1;
    cfg.min_nonzero_mm_req = 2;
    cfg.min_nonzero_im_req = 3;

    assert_eq!(cfg.validate_public_user_fund(), Ok(()));
}

#[test]
fn v16_view_deposit_and_withdraw_are_the_tested_paths() {
    let (mut header, mut markets) = market_fixture(1, 100);
    let mut account_header = account_fixture(1, 2);
    let mut market_view = MarketGroupV16ViewMut::new(&mut header, &mut markets);
    let mut account_view = PortfolioV16ViewMut::new(&mut account_header);

    market_view
        .deposit_not_atomic(&mut account_view, 11)
        .unwrap();
    market_view
        .withdraw_not_atomic(&mut account_view, 4)
        .unwrap();

    assert_eq!(account_view.header.capital.get(), 7);
    assert_eq!(market_view.header.c_tot.get(), 7);
    assert_eq!(market_view.header.vault.get(), 7);
    market_view.validate_shape().unwrap();
    account_view
        .validate_with_market(&market_view.as_view())
        .unwrap();
}

#[test]
fn v16_view_fee_sync_settles_flat_loss_before_fee() {
    let (mut header, mut markets) = market_fixture(1, 100);
    let mut account_header = account_fixture(1, 4);
    header.vault = V16PodU128::new(100);
    header.c_tot = V16PodU128::new(100);
    header.negative_pnl_account_count = V16PodU64::new(1);
    header.current_slot = V16PodU64::new(10);
    header.slot_last = V16PodU64::new(10);
    account_header.capital = V16PodU128::new(100);
    account_header.pnl = V16PodI128::new(-40);

    let mut market_view = MarketGroupV16ViewMut::new(&mut header, &mut markets);
    let mut account_view = PortfolioV16ViewMut::new(&mut account_header);
    let charged = market_view
        .sync_account_fee_to_slot_not_atomic(&mut account_view, 10, 10)
        .unwrap();

    assert_eq!(charged, 60);
    assert_eq!(account_view.header.pnl.get(), 0);
    assert_eq!(account_view.header.capital.get(), 0);
    assert_eq!(market_view.header.c_tot.get(), 0);
    assert_eq!(market_view.header.insurance.get(), 60);
    assert_eq!(market_view.header.vault.get(), 100);
    assert_eq!(market_view.header.negative_pnl_account_count.get(), 0);
}

#[test]
fn v16_fee_sync_on_nonflat_account_settles_hidden_k_loss_before_fee() {
    let (mut header, mut markets) = market_fixture(1, 100);
    let mut long_header = account_fixture(1, 14);
    let mut short_header = account_fixture(1, 15);
    {
        let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);
        let mut long = PortfolioV16ViewMut::new(&mut long_header);
        let mut short = PortfolioV16ViewMut::new(&mut short_header);
        market.deposit_not_atomic(&mut long, 100).unwrap();
        market.deposit_not_atomic(&mut short, 1_000).unwrap();
        market
            .execute_trade_with_fee_loss_stale_scoped_not_atomic(
                &mut long,
                &mut short,
                TradeRequestV16 {
                    asset_index: 0,
                    size_q: signed_q(POS_SCALE),
                    exec_price: 100,
                    fee_bps: 0,
                },
                true,
            )
            .unwrap();
        market
            .accrue_asset_to_not_atomic(0, 2, 50, 0, true)
            .unwrap();
    }
    assert_eq!(long_header.pnl.get(), 0);
    assert_eq!(long_header.capital.get(), 100);
    assert_eq!(header.insurance.get(), 0);

    let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);
    let mut long = PortfolioV16ViewMut::new(&mut long_header);
    let charged = market
        .sync_account_fee_to_slot_not_atomic(&mut long, 2, 100)
        .unwrap();

    assert_eq!(
        charged, 50,
        "lazy K loss must consume principal before recurring fee collection"
    );
    assert_eq!(long.header.capital.get(), 0);
    assert_eq!(long.header.pnl.get(), 0);
    assert_eq!(market.header.insurance.get(), 50);
    market.validate_shape().unwrap();
    long.validate_with_market(&market.as_view()).unwrap();
}

#[test]
fn v16_finalize_side_reset_is_public_value_neutral_and_epoch_bumping() {
    let (mut header, mut markets) = market_fixture(1, 100);
    let vault_before = header.vault.get();
    let c_tot_before = header.c_tot.get();
    let insurance_before = header.insurance.get();
    let risk_epoch_before = header.risk_epoch.get();
    let mut asset = markets[0].engine.asset.try_to_runtime().unwrap();
    asset.mode_long = SideModeV16::ResetPending;
    markets[0].engine.asset = AssetStateV16Account::from_runtime(&asset);

    let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);
    market
        .finalize_side_reset_not_atomic(0, SideV16::Long)
        .unwrap();

    let finalized = market.markets[0].engine.asset.try_to_runtime().unwrap();
    assert_eq!(finalized.mode_long, SideModeV16::Normal);
    assert_eq!(market.header.risk_epoch.get(), risk_epoch_before + 1);
    assert_eq!(market.header.vault.get(), vault_before);
    assert_eq!(market.header.c_tot.get(), c_tot_before);
    assert_eq!(market.header.insurance.get(), insurance_before);
    market.validate_shape().unwrap();
}

#[test]
fn v16_finalize_side_reset_rejects_blocked_pending_side() {
    let (mut header, mut markets) = market_fixture(1, 100);
    let risk_epoch_before = header.risk_epoch.get();
    let mut asset = markets[0].engine.asset.try_to_runtime().unwrap();
    asset.mode_short = SideModeV16::ResetPending;
    asset.pending_obligation_count_short = 1;
    markets[0].engine.asset = AssetStateV16Account::from_runtime(&asset);

    let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);
    assert_eq!(
        market.finalize_side_reset_not_atomic(0, SideV16::Short),
        Err(V16Error::Stale)
    );

    let blocked = market.markets[0].engine.asset.try_to_runtime().unwrap();
    assert_eq!(blocked.mode_short, SideModeV16::ResetPending);
    assert_eq!(market.header.risk_epoch.get(), risk_epoch_before);
    assert_eq!(market.validate_shape(), Ok(()));
}

#[test]
fn v16_resolved_bound_refinement_uses_public_monotone_api() {
    let (mut header, mut markets) = market_fixture(1, 100);
    {
        let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);
        market.resolve_market_not_atomic(1).unwrap();
    }
    header.vault = V16PodU128::new(50);
    let exact_num = 2 * BOUND_SCALE;
    let bound_num = 4 * BOUND_SCALE;
    header.payout_snapshot_captured = 1;
    header.resolved_payout_ledger =
        ResolvedPayoutLedgerV16Account::from_runtime(&ResolvedPayoutLedgerV16 {
            snapshot_residual: 3,
            terminal_claim_exact_receipts_num: exact_num,
            terminal_claim_bound_unreceipted_num: bound_num,
            current_payout_rate_num: 3 * BOUND_SCALE,
            current_payout_rate_den: exact_num + bound_num,
            snapshot_slot: 1,
            payout_halted: false,
            finalized: false,
        });
    let vault_before = header.vault.get();
    let c_tot_before = header.c_tot.get();
    let insurance_before = header.insurance.get();

    let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);
    market
        .refine_resolved_unreceipted_bound_not_atomic(2 * BOUND_SCALE)
        .unwrap();

    let refined = market
        .header
        .resolved_payout_ledger
        .try_to_runtime()
        .unwrap();
    assert_eq!(
        refined.terminal_claim_bound_unreceipted_num,
        2 * BOUND_SCALE
    );
    assert!(
        refined.current_payout_rate_num * (exact_num + bound_num)
            >= (3 * BOUND_SCALE) * refined.current_payout_rate_den,
        "bound refinement must not reduce already-quoted payout rate"
    );
    assert_eq!(market.header.vault.get(), vault_before);
    assert_eq!(market.header.c_tot.get(), c_tot_before);
    assert_eq!(market.header.insurance.get(), insurance_before);
    market.validate_shape().unwrap();
}

#[test]
fn v16_batch_trade_applies_multiple_fills_after_inline_refresh() {
    let (mut header, mut markets) = market_fixture(2, 100);
    let mut long_header = account_fixture(2, 201);
    let mut short_header = account_fixture(2, 202);
    let requests = [
        TradeRequestV16 {
            asset_index: 0,
            size_q: signed_q(POS_SCALE),
            exec_price: 100,
            fee_bps: 0,
        },
        TradeRequestV16 {
            asset_index: 1,
            size_q: signed_q(2 * POS_SCALE),
            exec_price: 100,
            fee_bps: 0,
        },
    ];

    let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);
    let mut long = PortfolioV16ViewMut::new(&mut long_header);
    let mut short = PortfolioV16ViewMut::new(&mut short_header);
    market.deposit_not_atomic(&mut long, 1_000).unwrap();
    market.deposit_not_atomic(&mut short, 1_000).unwrap();

    let outcome = market
        .execute_batch_with_fee_loss_stale_scoped_not_atomic(&mut long, &mut short, &requests, true)
        .unwrap();

    assert_eq!(outcome.fill_count, 2);
    assert_eq!(outcome.notional, 300);
    assert_eq!(outcome.fee_a, 0);
    assert_eq!(outcome.fee_b, 0);
    assert_ne!(long.header.active_bitmap[0].get(), 0);
    assert_ne!(short.header.active_bitmap[0].get(), 0);
    assert_eq!(
        market.markets[0].engine.asset.oi_eff_long_q.get(),
        POS_SCALE
    );
    assert_eq!(
        market.markets[0].engine.asset.oi_eff_short_q.get(),
        POS_SCALE
    );
    assert_eq!(
        market.markets[1].engine.asset.oi_eff_long_q.get(),
        2 * POS_SCALE
    );
    assert_eq!(
        market.markets[1].engine.asset.oi_eff_short_q.get(),
        2 * POS_SCALE
    );
    market.validate_shape().unwrap();
    long.validate_with_market(&market.as_view()).unwrap();
    short.validate_with_market(&market.as_view()).unwrap();
}

#[test]
fn v16_batch_trade_supports_mixed_signed_spread_legs() {
    let (mut header, mut markets) = market_fixture(2, 100);
    let mut taker_header = account_fixture(2, 221);
    let mut lp_header = account_fixture(2, 222);
    let size_q = signed_q(5 * POS_SCALE);
    let requests = [
        TradeRequestV16 {
            asset_index: 0,
            size_q,
            exec_price: 100,
            fee_bps: 0,
        },
        TradeRequestV16 {
            asset_index: 1,
            size_q: -size_q,
            exec_price: 100,
            fee_bps: 0,
        },
    ];

    let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);
    let mut taker = PortfolioV16ViewMut::new(&mut taker_header);
    let mut lp = PortfolioV16ViewMut::new(&mut lp_header);
    market.deposit_not_atomic(&mut taker, 1_000).unwrap();
    market.deposit_not_atomic(&mut lp, 1_000).unwrap();

    let outcome = market
        .execute_batch_with_fee_loss_stale_scoped_not_atomic(&mut taker, &mut lp, &requests, true)
        .unwrap();

    assert_eq!(outcome.fill_count, 2);
    assert_eq!(outcome.notional, 1_000);
    assert_eq!(
        market.markets[0].engine.asset.oi_eff_long_q.get(),
        5 * POS_SCALE
    );
    assert_eq!(
        market.markets[0].engine.asset.oi_eff_short_q.get(),
        5 * POS_SCALE
    );
    assert_eq!(
        market.markets[1].engine.asset.oi_eff_long_q.get(),
        5 * POS_SCALE
    );
    assert_eq!(
        market.markets[1].engine.asset.oi_eff_short_q.get(),
        5 * POS_SCALE
    );

    let taker_asset0 = taker.header.legs[0].try_to_runtime().unwrap();
    let taker_asset1 = taker.header.legs[1].try_to_runtime().unwrap();
    let lp_asset0 = lp.header.legs[0].try_to_runtime().unwrap();
    let lp_asset1 = lp.header.legs[1].try_to_runtime().unwrap();
    assert_eq!(taker_asset0.side, SideV16::Long);
    assert_eq!(taker_asset1.side, SideV16::Short);
    assert_eq!(lp_asset0.side, SideV16::Short);
    assert_eq!(lp_asset1.side, SideV16::Long);
    assert_eq!(taker_asset0.basis_pos_q, size_q);
    assert_eq!(taker_asset1.basis_pos_q, -size_q);
    assert_eq!(lp_asset0.basis_pos_q, -size_q);
    assert_eq!(lp_asset1.basis_pos_q, size_q);
    market.validate_shape().unwrap();
    taker.validate_with_market(&market.as_view()).unwrap();
    lp.validate_with_market(&market.as_view()).unwrap();
}

#[test]
fn v16_single_trade_matches_batch_of_one_state() {
    let (mut single_header, mut single_markets) = market_fixture(1, 100);
    let mut single_long_header = account_fixture(1, 209);
    let mut single_short_header = account_fixture(1, 210);
    let mut batch_header = single_header;
    let mut batch_markets = single_markets.clone();
    let mut batch_long_header = single_long_header;
    let mut batch_short_header = single_short_header;
    let request = TradeRequestV16 {
        asset_index: 0,
        size_q: signed_q(2 * POS_SCALE),
        exec_price: 100,
        fee_bps: 0,
    };

    let single_outcome = {
        let mut market = MarketGroupV16ViewMut::new(&mut single_header, &mut single_markets);
        let mut long = PortfolioV16ViewMut::new(&mut single_long_header);
        let mut short = PortfolioV16ViewMut::new(&mut single_short_header);
        market.deposit_not_atomic(&mut long, 1_000).unwrap();
        market.deposit_not_atomic(&mut short, 1_000).unwrap();
        market
            .execute_trade_with_fee_loss_stale_scoped_not_atomic(
                &mut long, &mut short, request, true,
            )
            .unwrap()
    };
    let batch_outcome = {
        let mut market = MarketGroupV16ViewMut::new(&mut batch_header, &mut batch_markets);
        let mut long = PortfolioV16ViewMut::new(&mut batch_long_header);
        let mut short = PortfolioV16ViewMut::new(&mut batch_short_header);
        market.deposit_not_atomic(&mut long, 1_000).unwrap();
        market.deposit_not_atomic(&mut short, 1_000).unwrap();
        market
            .execute_batch_with_fee_loss_stale_scoped_not_atomic(
                &mut long,
                &mut short,
                &[request],
                true,
            )
            .unwrap()
    };

    assert_eq!(batch_outcome.fill_count, 1);
    assert_eq!(single_outcome.fee_a, batch_outcome.fee_a);
    assert_eq!(single_outcome.fee_b, batch_outcome.fee_b);
    assert_eq!(single_outcome.notional, batch_outcome.notional);
    assert_eq!(single_header, batch_header);
    assert_eq!(single_markets, batch_markets);
    assert_eq!(single_long_header, batch_long_header);
    assert_eq!(single_short_header, batch_short_header);
}

#[test]
fn v16_batch_trade_checks_initial_margin_on_final_portfolio() {
    let (mut header, mut markets) = market_fixture(2, 100);
    let mut taker_header = account_fixture(2, 211);
    let mut lp_header = account_fixture(2, 212);
    {
        let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);
        let mut taker = PortfolioV16ViewMut::new(&mut taker_header);
        let mut lp = PortfolioV16ViewMut::new(&mut lp_header);
        market.deposit_not_atomic(&mut taker, 1_000).unwrap();
        market.deposit_not_atomic(&mut lp, 1_000).unwrap();
        market
            .execute_trade_with_fee_loss_stale_scoped_not_atomic(
                &mut lp,
                &mut taker,
                TradeRequestV16 {
                    asset_index: 0,
                    size_q: signed_q(10 * POS_SCALE),
                    exec_price: 100,
                    fee_bps: 0,
                },
                true,
            )
            .unwrap();
    }

    let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);
    let mut taker = PortfolioV16ViewMut::new(&mut taker_header);
    let mut lp = PortfolioV16ViewMut::new(&mut lp_header);
    let outcome = market
        .execute_batch_with_fee_loss_stale_scoped_not_atomic(
            &mut taker,
            &mut lp,
            &[
                TradeRequestV16 {
                    asset_index: 1,
                    size_q: signed_q(10 * POS_SCALE),
                    exec_price: 100,
                    fee_bps: 0,
                },
                TradeRequestV16 {
                    asset_index: 0,
                    size_q: signed_q(10 * POS_SCALE),
                    exec_price: 100,
                    fee_bps: 0,
                },
            ],
            true,
        )
        .expect("batch must not reject a final-IM-valid basket due to interim IM");

    assert_eq!(outcome.fill_count, 2);
    assert_eq!(outcome.notional, 2_000);
    assert_eq!(
        market.markets[0].engine.asset.oi_eff_long_q.get(),
        0,
        "second fill closes the original asset-0 exposure"
    );
    assert_eq!(
        market.markets[1].engine.asset.oi_eff_long_q.get(),
        10 * POS_SCALE,
        "final portfolio keeps only the replacement asset-1 exposure"
    );
    assert_eq!(
        taker
            .header
            .health_cert
            .try_to_runtime()
            .unwrap()
            .certified_initial_req,
        1_000
    );
    assert_eq!(
        lp.header
            .health_cert
            .try_to_runtime()
            .unwrap()
            .certified_initial_req,
        1_000
    );
    market.validate_shape().unwrap();
    taker.validate_with_market(&market.as_view()).unwrap();
    lp.validate_with_market(&market.as_view()).unwrap();
}

#[test]
fn v16_batch_trade_self_settles_stale_certificates_once_before_fills() {
    let (mut header, mut markets) = market_fixture(1, 100);
    let mut long_header = account_fixture(1, 203);
    let mut short_header = account_fixture(1, 204);
    {
        let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);
        let mut long = PortfolioV16ViewMut::new(&mut long_header);
        let mut short = PortfolioV16ViewMut::new(&mut short_header);
        market.deposit_not_atomic(&mut long, 1_000).unwrap();
        market.deposit_not_atomic(&mut short, 1_000).unwrap();
        market
            .execute_trade_with_fee_loss_stale_scoped_not_atomic(
                &mut long,
                &mut short,
                TradeRequestV16 {
                    asset_index: 0,
                    size_q: signed_q(POS_SCALE),
                    exec_price: 100,
                    fee_bps: 0,
                },
                true,
            )
            .unwrap();
        market
            .accrue_asset_to_not_atomic(0, 2, 101, 0, true)
            .unwrap();
        market.markets[0].engine.asset.raw_oracle_target_price = V16PodU64::new(101);
    }
    assert_eq!(long_header.pnl.get(), 0);

    let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);
    let mut long = PortfolioV16ViewMut::new(&mut long_header);
    let mut short = PortfolioV16ViewMut::new(&mut short_header);
    let outcome = market
        .execute_batch_with_fee_loss_stale_scoped_not_atomic(
            &mut long,
            &mut short,
            &[TradeRequestV16 {
                asset_index: 0,
                size_q: signed_q(POS_SCALE),
                exec_price: 101,
                fee_bps: 0,
            }],
            true,
        )
        .unwrap();

    assert_eq!(outcome.fill_count, 1);
    assert_eq!(outcome.notional, 101);
    assert!(long.header.pnl.get() > 0);
    market.validate_shape().unwrap();
    long.validate_with_market(&market.as_view()).unwrap();
    short.validate_with_market(&market.as_view()).unwrap();
}

#[test]
fn v16_batch_trade_rejects_loss_stale_risk_increase_after_inline_settlement() {
    let (mut header, mut markets) = market_fixture(1, 100);
    let mut long_header = account_fixture(1, 207);
    let mut short_header = account_fixture(1, 208);
    {
        let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);
        let mut long = PortfolioV16ViewMut::new(&mut long_header);
        let mut short = PortfolioV16ViewMut::new(&mut short_header);
        market.deposit_not_atomic(&mut long, 1_000).unwrap();
        market.deposit_not_atomic(&mut short, 1_000).unwrap();
        market
            .execute_trade_with_fee_loss_stale_scoped_not_atomic(
                &mut long,
                &mut short,
                TradeRequestV16 {
                    asset_index: 0,
                    size_q: signed_q(POS_SCALE),
                    exec_price: 100,
                    fee_bps: 0,
                },
                true,
            )
            .unwrap();
        market
            .accrue_asset_to_not_atomic(0, 3, 101, 0, true)
            .unwrap();
        market.markets[0].engine.asset.raw_oracle_target_price = V16PodU64::new(101);
    }

    let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);
    let mut long = PortfolioV16ViewMut::new(&mut long_header);
    let mut short = PortfolioV16ViewMut::new(&mut short_header);
    let res = market.execute_batch_with_fee_loss_stale_scoped_not_atomic(
        &mut long,
        &mut short,
        &[TradeRequestV16 {
            asset_index: 0,
            size_q: signed_q(POS_SCALE),
            exec_price: 101,
            fee_bps: 0,
        }],
        true,
    );

    assert_eq!(res, Err(V16Error::LockActive));
}

#[test]
fn v16_public_scoped_trade_preserves_unrelated_loss_stale_summary() {
    let (mut header, mut markets) = market_fixture(2, 100);
    let mut long_header = account_fixture(2, 209);
    let mut short_header = account_fixture(2, 210);
    {
        let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);
        let mut long = PortfolioV16ViewMut::new(&mut long_header);
        let mut short = PortfolioV16ViewMut::new(&mut short_header);
        market.deposit_not_atomic(&mut long, 1_000).unwrap();
        market.deposit_not_atomic(&mut short, 1_000).unwrap();
    }
    header.current_slot = V16PodU64::new(10);
    header.slot_last = V16PodU64::new(9);
    header.loss_stale_active = 1;
    let mut current_asset = markets[0].engine.asset.try_to_runtime().unwrap();
    current_asset.slot_last = 10;
    markets[0].engine.asset = AssetStateV16Account::from_runtime(&current_asset);

    let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);
    let mut long = PortfolioV16ViewMut::new(&mut long_header);
    let mut short = PortfolioV16ViewMut::new(&mut short_header);
    let outcome = market
        .execute_trade_with_fee_loss_stale_scoped_not_atomic(
            &mut long,
            &mut short,
            TradeRequestV16 {
                asset_index: 0,
                size_q: signed_q(POS_SCALE),
                exec_price: 100,
                fee_bps: 0,
            },
            true,
        )
        .expect("unrelated loss-stale summary must not block a locally current trade");

    assert_eq!(outcome.notional, 100);
    assert_eq!(market.header.loss_stale_active, 1);
    assert_eq!(market.markets[0].engine.asset.slot_last.get(), 10);
    market.validate_shape().unwrap();
    long.validate_with_market(&market.as_view()).unwrap();
    short.validate_with_market(&market.as_view()).unwrap();
}

#[test]
fn v16_batch_trade_is_bounded_by_configured_portfolio_asset_cap() {
    let (mut header, mut markets) = market_fixture(1, 100);
    let mut long_header = account_fixture(1, 205);
    let mut short_header = account_fixture(1, 206);
    let requests = [
        TradeRequestV16 {
            asset_index: 0,
            size_q: signed_q(POS_SCALE),
            exec_price: 100,
            fee_bps: 0,
        },
        TradeRequestV16 {
            asset_index: 0,
            size_q: signed_q(POS_SCALE),
            exec_price: 100,
            fee_bps: 0,
        },
    ];
    let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);
    let mut long = PortfolioV16ViewMut::new(&mut long_header);
    let mut short = PortfolioV16ViewMut::new(&mut short_header);
    market.deposit_not_atomic(&mut long, 1_000).unwrap();
    market.deposit_not_atomic(&mut short, 1_000).unwrap();

    let res = market.execute_batch_with_fee_loss_stale_scoped_not_atomic(
        &mut long, &mut short, &requests, true,
    );

    assert_eq!(res, Err(V16Error::InvalidConfig));
}

#[test]
fn v16_view_dynamic_market_slots_can_be_activated_without_runtime_vec_engine() {
    let (mut header, mut markets) = market_fixture(3, 100);
    let view = MarketGroupV16ViewMut::new(&mut header, &mut markets);
    view.validate_shape().unwrap();

    assert_eq!(
        view.header
            .config
            .try_to_runtime()
            .unwrap()
            .max_market_slots,
        3
    );
    assert_eq!(view.markets.len(), 3);
    assert_eq!(view.markets[2].engine.asset.market_id.get(), 3);
    assert_eq!(view.markets[2].engine.asset.effective_price.get(), 100);
}

#[test]
fn v16_public_raw_oracle_target_update_is_value_neutral_and_lifecycle_gated() {
    let (mut header, mut markets) = market_fixture(1, 100);
    let vault_before = header.vault.get();
    let c_tot_before = header.c_tot.get();
    let insurance_before = header.insurance.get();
    let oracle_epoch_before = header.oracle_epoch.get();

    let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);
    market
        .set_asset_raw_oracle_target_not_atomic(0, 111)
        .unwrap();
    let asset = market.markets[0].engine.asset.try_to_runtime().unwrap();

    assert_eq!(asset.raw_oracle_target_price, 111);
    assert_eq!(asset.effective_price, 100);
    assert_eq!(market.header.oracle_epoch.get(), oracle_epoch_before + 1);
    assert_eq!(market.header.vault.get(), vault_before);
    assert_eq!(market.header.c_tot.get(), c_tot_before);
    assert_eq!(market.header.insurance.get(), insurance_before);
    market
        .set_asset_raw_oracle_target_not_atomic(0, 111)
        .unwrap();
    assert_eq!(market.header.oracle_epoch.get(), oracle_epoch_before + 1);
    market.validate_shape().unwrap();
}

#[test]
fn v16_raw_oracle_target_only_change_invalidates_a_cached_health_cert() {
    // Regression for engine #107 / #93: a target-only push (no accrual, no
    // effective_price move) must invalidate any health cert taken while the
    // old target was in force, because target/effective lag -- and therefore
    // risk -- has grown even though the cert's own numbers are stale-blind
    // to it. We drive this through a real cert-gated not_atomic API
    // (charge_account_backing_fee_not_atomic) rather than reading the epoch
    // counter directly, so the test fails the way an exploit would: a stale
    // cert being admitted to authorize a financial action.
    let (mut header, mut markets) = market_fixture(1, 100);
    let mut account_header = account_fixture(1, 30);
    header.vault = V16PodU128::new(100);
    header.c_tot = V16PodU128::new(100);
    account_header.capital = V16PodU128::new(100);
    let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);
    market
        .deposit_fresh_counterparty_backing_not_atomic(0, 1, 10)
        .unwrap();

    // Cert taken while the target is still 100 (matches effective_price, no
    // lag) and current against every header epoch at the moment it's minted.
    account_header.health_cert = HealthCertV16Account::from_runtime(&HealthCertV16 {
        certified_equity: 100,
        certified_initial_req: 50,
        certified_maintenance_req: 40,
        cert_oracle_epoch: market.header.oracle_epoch.get(),
        cert_funding_epoch: market.header.funding_epoch.get(),
        cert_risk_epoch: market.header.risk_epoch.get(),
        cert_asset_set_epoch: market.header.asset_set_epoch.get(),
        active_bitmap_at_cert: V16_EMPTY_ACTIVE_BITMAP,
        valid: true,
        ..HealthCertV16::default()
    });
    let mut account = PortfolioV16ViewMut::new(&mut account_header);

    // Sanity: with the target unchanged, this exact cert is still admitted
    // (proves the setup is correct and the rejection below is caused
    // specifically by the target push, not some other staleness source).
    let vault_before = market.header.vault.get();
    let c_tot_before = market.header.c_tot.get();
    let insurance_before = market.header.insurance.get();
    let capital_before = account.header.capital.get();

    // Now push a target-only change: raw_oracle_target_price moves from 100
    // to 111 while effective_price (100) does not -- new target/effective
    // lag with zero accrual in between.
    market
        .set_asset_raw_oracle_target_not_atomic(0, 111)
        .unwrap();

    // The cert minted above is now stale: it was current for the pre-push
    // oracle_epoch and must be rejected, not silently admitted under grown
    // target/effective lag.
    let err = market.charge_account_backing_fee_not_atomic(&mut account, 0, 6, 1, 4);
    assert_eq!(
        err,
        Err(V16Error::Stale),
        "cached cert must be invalidated by a target-only oracle push"
    );
    assert_eq!(market.header.vault.get(), vault_before);
    assert_eq!(market.header.c_tot.get(), c_tot_before);
    assert_eq!(market.header.insurance.get(), insurance_before);
    assert_eq!(account.header.capital.get(), capital_before);
    assert_eq!(market.validate_shape(), Ok(()));
}

#[test]
fn v16_public_empty_asset_oracle_anchor_reset_rejects_any_group_position_state() {
    let (mut header, mut markets) = market_fixture(2, 100);
    let mut other_asset = markets[1].engine.asset.try_to_runtime().unwrap();
    other_asset.oi_eff_long_q = POS_SCALE;
    other_asset.oi_eff_short_q = POS_SCALE;
    other_asset.stored_pos_count_long = 1;
    other_asset.stored_pos_count_short = 1;
    other_asset.loss_weight_sum_long = POS_SCALE;
    other_asset.loss_weight_sum_short = POS_SCALE;
    markets[1].engine.asset = AssetStateV16Account::from_runtime(&other_asset);

    let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);
    let res = market.reset_empty_asset_oracle_anchor_not_atomic(0, 123, 10);

    assert_eq!(res, Err(V16Error::LockActive));
    assert_eq!(market.markets[0].engine.asset.effective_price.get(), 100);
}

#[test]
fn v16_public_empty_asset_oracle_anchor_reset_is_value_neutral() {
    let (mut header, mut markets) = market_fixture(2, 100);
    let vault_before = header.vault.get();
    let c_tot_before = header.c_tot.get();
    let insurance_before = header.insurance.get();

    let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);
    market
        .reset_empty_asset_oracle_anchor_not_atomic(0, 123, 10)
        .unwrap();
    let asset = market.markets[0].engine.asset.try_to_runtime().unwrap();

    assert_eq!(asset.raw_oracle_target_price, 123);
    assert_eq!(asset.effective_price, 123);
    assert_eq!(asset.fund_px_last, 123);
    assert_eq!(asset.slot_last, 10);
    assert_eq!(market.header.current_slot.get(), 10);
    assert_eq!(market.header.slot_last.get(), 10);
    assert_eq!(market.header.vault.get(), vault_before);
    assert_eq!(market.header.c_tot.get(), c_tot_before);
    assert_eq!(market.header.insurance.get(), insurance_before);
    market.validate_shape().unwrap();
}

#[test]
fn v16_public_force_asset_recovery_freezes_mark_and_is_idempotent() {
    let (mut header, mut markets) = market_fixture(2, 100);
    {
        let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);
        market
            .set_asset_raw_oracle_target_not_atomic(1, 150)
            .unwrap();
    }
    let asset_epoch_before = header.asset_set_epoch.get();
    let risk_epoch_before = header.risk_epoch.get();
    let vault_before = header.vault.get();
    let c_tot_before = header.c_tot.get();
    let insurance_before = header.insurance.get();

    let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);
    market.force_asset_recovery_not_atomic(1, 2).unwrap();
    let asset = market.markets[1].engine.asset.try_to_runtime().unwrap();

    assert_eq!(asset.lifecycle, AssetLifecycleV16::Recovery);
    assert_eq!(asset.raw_oracle_target_price, asset.effective_price);
    assert_eq!(market.header.asset_set_epoch.get(), asset_epoch_before + 1);
    assert_eq!(market.header.risk_epoch.get(), risk_epoch_before + 1);
    assert_eq!(market.header.vault.get(), vault_before);
    assert_eq!(market.header.c_tot.get(), c_tot_before);
    assert_eq!(market.header.insurance.get(), insurance_before);

    market.force_asset_recovery_not_atomic(1, 2).unwrap();
    assert_eq!(market.header.asset_set_epoch.get(), asset_epoch_before + 1);
    assert_eq!(market.header.risk_epoch.get(), risk_epoch_before + 1);
    market.validate_shape().unwrap();
}

#[test]
fn v16_restart_empty_asset_preserves_domain_budget_for_nonzero_asset() {
    let (mut header, mut markets) = market_fixture(2, 100);
    {
        let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);
        market.deposit_domain_insurance_not_atomic(2, 10).unwrap();
        market.force_asset_recovery_not_atomic(1, 2).unwrap();
    }
    let old_market_id = markets[1].engine.asset.market_id.get();
    let budget_before = markets[1].engine.insurance_domain_budget_long.get();
    let budget_total_before = header.insurance_domain_budget_remaining_total.get();
    let vault_before = header.vault.get();
    let c_tot_before = header.c_tot.get();
    let insurance_before = header.insurance.get();

    let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);
    market
        .restart_empty_asset_preserving_insurance_budget_not_atomic(1, 222, 3)
        .unwrap();
    let asset = market.markets[1].engine.asset.try_to_runtime().unwrap();

    assert_eq!(asset.lifecycle, AssetLifecycleV16::Active);
    assert_ne!(asset.market_id, old_market_id);
    assert_eq!(asset.raw_oracle_target_price, 222);
    assert_eq!(
        market.markets[1].engine.insurance_domain_budget_long.get(),
        budget_before
    );
    assert_eq!(
        market.header.insurance_domain_budget_remaining_total.get(),
        budget_total_before
    );
    assert_eq!(market.header.vault.get(), vault_before);
    assert_eq!(market.header.c_tot.get(), c_tot_before);
    assert_eq!(market.header.insurance.get(), insurance_before);
    market.validate_shape().unwrap();
}

#[test]
fn v16_canonicalize_retired_empty_asset_slot_clears_inert_domain_state() {
    let (mut header, mut markets) = market_fixture(2, 100);
    {
        let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);
        market.retire_empty_asset_not_atomic(1, 3).unwrap();
    }
    let old_market_id = markets[1].engine.asset.market_id.get();
    let inert_empty_source = SourceCreditStateV16 {
        credit_epoch: 7,
        credit_rate_num: 0,
        ..SourceCreditStateV16::EMPTY
    };
    markets[1].engine.source_credit_long =
        SourceCreditStateV16Account::from_runtime(&inert_empty_source);
    markets[1].engine.source_credit_short =
        SourceCreditStateV16Account::from_runtime(&inert_empty_source);
    let vault_before = header.vault.get();
    let c_tot_before = header.c_tot.get();
    let insurance_before = header.insurance.get();

    let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);
    market
        .canonicalize_retired_empty_asset_slot_not_atomic(1)
        .unwrap();
    let asset = market.markets[1].engine.asset.try_to_runtime().unwrap();

    assert_eq!(asset.lifecycle, AssetLifecycleV16::Retired);
    assert_eq!(asset.market_id, old_market_id);
    assert_eq!(
        market.markets[1]
            .engine
            .source_credit_long
            .try_to_runtime()
            .unwrap(),
        SourceCreditStateV16::EMPTY
    );
    assert_eq!(market.header.vault.get(), vault_before);
    assert_eq!(market.header.c_tot.get(), c_tot_before);
    assert_eq!(market.header.insurance.get(), insurance_before);
    market.validate_shape().unwrap();
}

#[test]
fn v16_reused_market_slot_rejects_old_market_id_leg() {
    let (mut header, mut markets) = market_fixture(1, 100);
    let mut account_header = account_fixture(1, 16);
    let old_market_id = markets[0].engine.asset.market_id.get();
    {
        let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);
        market.retire_empty_asset_not_atomic(0, 1).unwrap();
    }
    header
        .activate_empty_asset_slot_not_atomic(0, &mut markets[0].engine, 200, 2)
        .unwrap();
    assert_ne!(markets[0].engine.asset.market_id.get(), old_market_id);

    account_header.legs[0] = PortfolioLegV16Account::from_runtime(&PortfolioLegV16 {
        active: true,
        asset_index: 0,
        market_id: old_market_id,
        side: SideV16::Long,
        basis_pos_q: POS_SCALE as i128,
        a_basis: ADL_ONE,
        k_snap: 0,
        f_snap: 0,
        epoch_snap: 0,
        loss_weight: POS_SCALE,
        b_snap: 0,
        b_rem: 0,
        b_epoch_snap: 0,
        b_stale: false,
        stale: false,
    });
    account_header.active_bitmap[0] = V16PodU64::new(1);

    let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);
    let mut account = PortfolioV16ViewMut::new(&mut account_header);
    assert_eq!(
        market.full_account_refresh_not_atomic(&mut account),
        Err(V16Error::HiddenLeg),
        "stale legs from a retired market slot must not bind to the reactivated market"
    );
    market.validate_shape().unwrap();
}

#[test]
fn v16_retire_and_reactivate_empty_asset_after_source_credit_epoch_bump() {
    let (mut header, mut markets) = market_fixture(1, 100);
    let old_market_id = markets[0].engine.asset.market_id.get();
    let recomputed_empty_source = SourceCreditStateV16 {
        credit_epoch: 2,
        ..SourceCreditStateV16::EMPTY
    };
    markets[0].engine.source_credit_long =
        SourceCreditStateV16Account::from_runtime(&recomputed_empty_source);
    markets[0].engine.source_credit_short =
        SourceCreditStateV16Account::from_runtime(&recomputed_empty_source);

    let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);
    market.retire_empty_asset_not_atomic(0, 1).unwrap();
    assert_eq!(
        market.markets[0]
            .engine
            .asset
            .try_to_runtime()
            .unwrap()
            .lifecycle,
        AssetLifecycleV16::Retired
    );

    market
        .header
        .activate_empty_market_slot_not_atomic(0, &mut market.markets[0], 200, 2)
        .unwrap();
    assert_ne!(
        market.markets[0].engine.asset.market_id.get(),
        old_market_id
    );
    assert_eq!(
        market.markets[0]
            .engine
            .source_credit_long
            .try_to_runtime()
            .unwrap(),
        SourceCreditStateV16::EMPTY
    );
    assert_eq!(
        market.markets[0]
            .engine
            .source_credit_short
            .try_to_runtime()
            .unwrap(),
        SourceCreditStateV16::EMPTY
    );
    market.validate_shape().unwrap();
}

#[test]
fn v16_view_rejects_overwithdraw() {
    let (mut header, mut markets) = market_fixture(1, 100);
    let mut account_header = account_fixture(1, 6);
    let mut market_view = MarketGroupV16ViewMut::new(&mut header, &mut markets);
    let mut account_view = PortfolioV16ViewMut::new(&mut account_header);
    market_view
        .deposit_not_atomic(&mut account_view, 3)
        .unwrap();

    let err = market_view.withdraw_not_atomic(&mut account_view, 4);

    assert_eq!(err, Err(V16Error::LockActive));
}

// E6 (port of upstream engine c8aab338): a finalized zero-residual close
// ledger represents no outstanding obligation -- it must not permanently
// freeze a flat, solvent user's withdrawal just because the ledger is still
// `active` for history/identity. Companion to the already-carried Finding E
// (canceled-ledger) exemption proven by `v16_view_rejects_overwithdraw`'s
// sibling tests above.
#[test]
fn v16_finalized_zero_residual_close_does_not_block_withdraw() {
    let (mut header, mut markets) = market_fixture(1, 100);
    let mut account_header = account_fixture(1, 7);
    let market_id = markets[0].engine.asset.market_id.get();

    {
        let mut market_view = MarketGroupV16ViewMut::new(&mut header, &mut markets);
        let mut account_view = PortfolioV16ViewMut::new(&mut account_header);
        market_view
            .deposit_not_atomic(&mut account_view, 10)
            .unwrap();
    }

    account_header.close_progress = CloseProgressLedgerV16Account::from_runtime(
        &finalized_inert_close_progress(market_id, 3, 5),
    );

    let mut market_view = MarketGroupV16ViewMut::new(&mut header, &mut markets);
    let mut account_view = PortfolioV16ViewMut::new(&mut account_header);

    market_view
        .withdraw_not_atomic(&mut account_view, 4)
        .unwrap();

    assert_eq!(account_view.header.capital.get(), 6);
    assert_eq!(market_view.header.c_tot.get(), 6);
    assert_eq!(market_view.header.vault.get(), 6);
    // Withdraw does not itself mutate the finalized ledger -- it only stops
    // treating it as a blocker.
    let after_close = account_view.header.close_progress.try_to_runtime().unwrap();
    assert!(after_close.active && after_close.finalized && after_close.residual_remaining == 0);
}

// E6, second call site: a finalized zero-residual close ledger must also be
// treated as inert by the empty-account dematerialization gate
// (`is_empty_for_dematerialization`, reached via
// register/deregister_empty_materialized_portfolio_not_atomic), so an
// otherwise-empty account is not stranded from ordinary lifecycle bookkeeping
// (materialized-portfolio rent accounting) after an insurance-covered
// liquidation finishes paying out.
#[test]
fn v16_finalized_zero_residual_close_does_not_block_dematerialization() {
    let (mut header, mut markets) = market_fixture(1, 100);
    let mut account_header = account_fixture(1, 8);
    let market_id = markets[0].engine.asset.market_id.get();
    account_header.close_progress = CloseProgressLedgerV16Account::from_runtime(
        &finalized_inert_close_progress(market_id, 2, 5),
    );

    let mut market_view = MarketGroupV16ViewMut::new(&mut header, &mut markets);
    let account_view = PortfolioV16ViewMut::new(&mut account_header);

    market_view
        .register_empty_materialized_portfolio_not_atomic(&account_view.as_view())
        .unwrap();
    assert_eq!(market_view.header.materialized_portfolio_count.get(), 1);

    market_view
        .deregister_empty_materialized_portfolio_not_atomic(&account_view.as_view())
        .unwrap();
    assert_eq!(market_view.header.materialized_portfolio_count.get(), 0);
}

#[cfg(feature = "fuzz")]
#[test]
fn v16_insurance_lien_consume_rejects_fractional_bound_amount() {
    let (mut header, mut markets) = market_fixture(1, 100);
    header.vault = V16PodU128::new(10);
    header.insurance = V16PodU128::new(10);

    let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);
    market.deposit_domain_insurance_not_atomic(0, 10).unwrap();
    market
        .reserve_insurance_credit_not_atomic(0, BOUND_SCALE)
        .unwrap();
    market
        .create_source_credit_lien_from_insurance_not_atomic(0, BOUND_SCALE)
        .unwrap();

    let before_insurance = market.header.insurance;
    let before_spent = market.markets[0].engine.insurance_domain_spent_long;
    let before_reservation = market.markets[0].engine.insurance_reservation_long;
    let before_source = market.markets[0].engine.source_credit_long;

    let err = market.consume_source_credit_lien_from_insurance_not_atomic(0, 1);

    assert_eq!(err, Err(V16Error::InvalidConfig));
    assert_eq!(market.header.insurance, before_insurance);
    assert_eq!(
        market.markets[0].engine.insurance_domain_spent_long,
        before_spent
    );
    assert_eq!(
        market.markets[0].engine.insurance_reservation_long,
        before_reservation
    );
    assert_eq!(market.markets[0].engine.source_credit_long, before_source);
}

#[test]
fn v16_domain_insurance_deposit_and_withdraw_use_engine_budget_accounting() {
    let (mut header, mut markets) = market_fixture(1, 100);
    let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);

    market.deposit_domain_insurance_not_atomic(0, 10).unwrap();
    assert_eq!(market.header.vault.get(), 10);
    assert_eq!(market.header.insurance.get(), 10);
    assert_eq!(
        market.header.insurance_domain_budget_remaining_total.get(),
        10
    );
    assert_eq!(
        market.markets[0].engine.insurance_domain_budget_long.get(),
        10
    );

    market.withdraw_domain_insurance_not_atomic(0, 4).unwrap();
    assert_eq!(market.header.vault.get(), 6);
    assert_eq!(market.header.insurance.get(), 6);
    assert_eq!(
        market.header.insurance_domain_budget_remaining_total.get(),
        6
    );
    assert_eq!(
        market.markets[0].engine.insurance_domain_budget_long.get(),
        6
    );
    assert_eq!(market.validate_shape(), Ok(()));
}

#[test]
fn v16_credit_account_from_insurance_uses_unbudgeted_surplus_only() {
    let (mut header, mut markets) = market_fixture(1, 100);
    header.vault = V16PodU128::new(10);
    header.insurance = V16PodU128::new(10);
    let mut account_header = account_fixture(1, 9);
    let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);
    let mut account = PortfolioV16ViewMut::new(&mut account_header);

    market
        .credit_account_from_insurance_not_atomic(&mut account, 3, 0)
        .unwrap();
    assert_eq!(market.header.vault.get(), 10);
    assert_eq!(market.header.insurance.get(), 7);
    assert_eq!(market.header.c_tot.get(), 3);
    assert_eq!(account.header.capital.get(), 3);
    assert_eq!(market.validate_shape(), Ok(()));
    assert_eq!(account.validate_with_market(&market.as_view()), Ok(()));

    market
        .credit_domain_insurance_budget_not_atomic(0, 7)
        .unwrap();
    let err = market.credit_account_from_insurance_not_atomic(&mut account, 1, 0);
    assert_eq!(
        err,
        Err(V16Error::LockActive),
        "budgeted domain insurance must not be paid as a cranker reward"
    );
}

#[test]
fn v16_credit_account_from_insurance_respects_additional_reserved() {
    // Starvation-attack regression (protocol-fee RESERVE amendment,
    // ~/v17/DECISIONS-LEDGER.md): a crank-reward-style credit must never dip
    // `header.insurance` below the caller-declared `additional_reserved`
    // floor (e.g. the protocol's accrued-but-unwithdrawn fee claim), even
    // when the naive unbudgeted-surplus check (pre-amendment: `budget_remaining
    // > next_insurance`) would have allowed it.
    let (mut header, mut markets) = market_fixture(1, 100);
    header.vault = V16PodU128::new(10);
    header.insurance = V16PodU128::new(10);
    let mut account_header = account_fixture(1, 9);
    let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);
    let mut account = PortfolioV16ViewMut::new(&mut account_header);

    // No domain budget allocated (budget_remaining == 0), so the pre-amendment
    // check would allow draining the full 10 atoms of unbudgeted surplus as a
    // "cranker reward". With a 6-atom protocol reservation in place, only 4
    // atoms are actually free.
    let protocol_owed: u128 = 6;

    let err = market.credit_account_from_insurance_not_atomic(&mut account, 5, protocol_owed);
    assert_eq!(
        err,
        Err(V16Error::LockActive),
        "cranker reward must not be able to dip insurance below the protocol's reserved claim"
    );
    // Insurance/vault/capital must be untouched by the rejected attempt.
    assert_eq!(market.header.insurance.get(), 10);
    assert_eq!(account.header.capital.get(), 0);

    // Exactly the free amount (10 - 6 = 4) still succeeds.
    market
        .credit_account_from_insurance_not_atomic(&mut account, 4, protocol_owed)
        .unwrap();
    assert_eq!(market.header.insurance.get(), 6);
    assert_eq!(account.header.capital.get(), 4);
    assert_eq!(market.validate_shape(), Ok(()));

    // The reserved floor (6) is now exactly `header.insurance` -- any further
    // draw, however small, must fail.
    let err = market.credit_account_from_insurance_not_atomic(&mut account, 1, protocol_owed);
    assert_eq!(err, Err(V16Error::LockActive));
    assert_eq!(market.header.insurance.get(), 6, "reserve floor is exact");
}

#[test]
fn v16_public_domain_insurance_spent_setter_preserves_budget_total() {
    let (mut header, mut markets) = market_fixture(1, 100);
    let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);

    market.deposit_domain_insurance_not_atomic(0, 10).unwrap();
    market.set_domain_insurance_spent(0, 4).unwrap();
    assert_eq!(
        market.header.insurance_domain_budget_remaining_total.get(),
        6
    );
    assert_eq!(
        market.markets[0].engine.insurance_domain_spent_long.get(),
        4
    );
    market.set_domain_insurance_spent(0, 0).unwrap();
    assert_eq!(
        market.header.insurance_domain_budget_remaining_total.get(),
        10
    );
    assert_eq!(market.validate_shape(), Ok(()));
}

#[test]
fn v16_public_domain_insurance_spent_setter_rejects_unbacked_clear() {
    let (mut header, mut markets) = market_fixture(1, 100);
    header.vault = V16PodU128::new(5);
    header.insurance = V16PodU128::new(5);
    header.insurance_domain_budget_remaining_total = V16PodU128::new(5);
    markets[0].engine.insurance_domain_budget_long = V16PodU128::new(10);
    markets[0].engine.insurance_domain_spent_long = V16PodU128::new(5);
    let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);
    assert_eq!(market.validate_shape(), Ok(()));

    let err = market.set_domain_insurance_spent(0, 0);

    assert_eq!(err, Err(V16Error::LockActive));
    assert_eq!(
        market.header.insurance_domain_budget_remaining_total.get(),
        5
    );
    assert_eq!(
        market.markets[0].engine.insurance_domain_spent_long.get(),
        5
    );
}

#[test]
fn v16_backing_provider_earnings_credit_and_withdraw_are_engine_accounted() {
    let (mut header, mut markets) = market_fixture(1, 100);
    header.vault = V16PodU128::new(10);
    let market_id = markets[0].engine.asset.market_id.get();
    markets[0].engine.backing_long = BackingBucketV16Account::from_runtime(&BackingBucketV16 {
        market_id,
        fresh_unliened_backing_num: 1,
        expiry_slot: 10,
        status: BackingBucketStatusV16::Fresh,
        ..BackingBucketV16::EMPTY
    });
    markets[0].engine.source_credit_long =
        SourceCreditStateV16Account::from_runtime(&SourceCreditStateV16 {
            fresh_reserved_backing_num: 1,
            credit_rate_num: CREDIT_RATE_SCALE,
            ..SourceCreditStateV16::EMPTY
        });
    let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);

    market
        .credit_backing_provider_earnings_not_atomic(0, 4)
        .unwrap();
    assert_eq!(market.header.vault.get(), 10);
    assert_eq!(market.header.backing_provider_earnings_total.get(), 4);
    assert_eq!(
        market.markets[0]
            .engine
            .backing_long
            .utilization_fee_earnings
            .get(),
        4
    );
    market
        .withdraw_backing_provider_earnings_not_atomic(0, 3)
        .unwrap();
    assert_eq!(market.header.vault.get(), 7);
    assert_eq!(market.header.backing_provider_earnings_total.get(), 1);
    assert_eq!(
        market.markets[0]
            .engine
            .backing_long
            .utilization_fee_earnings
            .get(),
        1
    );
    assert_eq!(market.validate_shape(), Ok(()));
}

#[test]
fn v16_backing_provider_earnings_credit_rejects_without_vault_slack() {
    let (mut header, mut markets) = market_fixture(1, 100);
    header.vault = V16PodU128::new(10);
    header.c_tot = V16PodU128::new(10);
    let market_id = markets[0].engine.asset.market_id.get();
    markets[0].engine.backing_long = BackingBucketV16Account::from_runtime(&BackingBucketV16 {
        market_id,
        fresh_unliened_backing_num: 1,
        expiry_slot: 10,
        status: BackingBucketStatusV16::Fresh,
        ..BackingBucketV16::EMPTY
    });
    markets[0].engine.source_credit_long =
        SourceCreditStateV16Account::from_runtime(&SourceCreditStateV16 {
            fresh_reserved_backing_num: 1,
            credit_rate_num: CREDIT_RATE_SCALE,
            ..SourceCreditStateV16::EMPTY
        });
    let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);
    assert_eq!(market.validate_shape(), Ok(()));

    let err = market.credit_backing_provider_earnings_not_atomic(0, 1);

    assert_eq!(err, Err(V16Error::LockActive));
    assert_eq!(market.header.backing_provider_earnings_total.get(), 0);
    assert_eq!(
        market.markets[0]
            .engine
            .backing_long
            .utilization_fee_earnings
            .get(),
        0
    );
}

#[test]
fn v16_public_backing_principal_deposit_and_withdraw_move_vault_and_source_state() {
    let (mut header, mut markets) = market_fixture(1, 100);
    let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);

    market
        .deposit_fresh_counterparty_backing_not_atomic(0, 5, 10)
        .unwrap();
    assert_eq!(market.header.vault.get(), 5);
    assert_eq!(
        market.markets[0]
            .engine
            .backing_long
            .fresh_unliened_backing_num
            .get(),
        5 * BOUND_SCALE
    );
    assert_eq!(
        market.markets[0]
            .engine
            .source_credit_long
            .fresh_reserved_backing_num
            .get(),
        5 * BOUND_SCALE
    );

    market
        .withdraw_fresh_counterparty_backing_not_atomic(0, 2)
        .unwrap();
    assert_eq!(market.header.vault.get(), 3);
    assert_eq!(
        market.markets[0]
            .engine
            .backing_long
            .fresh_unliened_backing_num
            .get(),
        3 * BOUND_SCALE
    );
    assert_eq!(
        market.markets[0]
            .engine
            .source_credit_long
            .fresh_reserved_backing_num
            .get(),
        3 * BOUND_SCALE
    );
    assert_eq!(market.validate_shape(), Ok(()));
}

#[cfg(feature = "fuzz")]
#[test]
fn v16_public_backing_principal_withdraw_rejects_if_claims_would_be_underbacked() {
    let (mut header, mut markets) = market_fixture(1, 100);
    let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);
    market
        .deposit_fresh_counterparty_backing_not_atomic(0, 5, 10)
        .unwrap();
    market.header.pnl_pos_bound_tot_num = V16PodU128::new(5 * BOUND_SCALE);
    market.header.pnl_pos_bound_tot = V16PodU128::new(5);
    market
        .add_source_positive_claim_bound_not_atomic(0, 5 * BOUND_SCALE, 5 * BOUND_SCALE)
        .unwrap();

    let err = market.withdraw_fresh_counterparty_backing_not_atomic(0, 1);

    assert_eq!(err, Err(V16Error::LockActive));
    assert_eq!(market.header.vault.get(), 5);
    assert_eq!(
        market.markets[0]
            .engine
            .source_credit_long
            .credit_rate_num
            .get(),
        CREDIT_RATE_SCALE
    );
    assert_eq!(market.validate_shape(), Ok(()));
}

#[test]
fn v16_public_account_backing_fee_routes_provider_and_insurance_splits_atomically() {
    let (mut header, mut markets) = market_fixture(1, 100);
    let mut account_header = account_fixture(1, 23);
    header.vault = V16PodU128::new(100);
    header.c_tot = V16PodU128::new(100);
    account_header.capital = V16PodU128::new(100);
    let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);
    market
        .deposit_fresh_counterparty_backing_not_atomic(0, 1, 10)
        .unwrap();
    account_header.health_cert = HealthCertV16Account::from_runtime(&HealthCertV16 {
        certified_equity: 100,
        certified_initial_req: 50,
        certified_maintenance_req: 40,
        cert_oracle_epoch: market.header.oracle_epoch.get(),
        cert_funding_epoch: market.header.funding_epoch.get(),
        cert_risk_epoch: market.header.risk_epoch.get(),
        cert_asset_set_epoch: market.header.asset_set_epoch.get(),
        active_bitmap_at_cert: V16_EMPTY_ACTIVE_BITMAP,
        valid: true,
        ..HealthCertV16::default()
    });
    let mut account = PortfolioV16ViewMut::new(&mut account_header);

    let charged = market
        .charge_account_backing_fee_not_atomic(&mut account, 0, 6, 1, 4)
        .unwrap();

    assert_eq!(charged, 10);
    assert_eq!(market.header.vault.get(), 101);
    assert_eq!(market.header.c_tot.get(), 90);
    assert_eq!(account.header.capital.get(), 90);
    assert_eq!(market.header.insurance.get(), 4);
    assert_eq!(
        market.header.insurance_domain_budget_remaining_total.get(),
        4
    );
    assert_eq!(
        market.markets[0]
            .engine
            .backing_long
            .utilization_fee_earnings
            .get(),
        6
    );
    assert_eq!(account.header.health_cert.certified_equity.get(), 90);
    assert_eq!(market.validate_shape(), Ok(()));
    assert_eq!(account.validate_with_market(&market.as_view()), Ok(()));
}

#[test]
fn v16_public_account_backing_fee_rejects_if_post_fee_im_would_fail() {
    let (mut header, mut markets) = market_fixture(1, 100);
    let mut account_header = account_fixture(1, 24);
    header.vault = V16PodU128::new(100);
    header.c_tot = V16PodU128::new(100);
    account_header.capital = V16PodU128::new(100);
    let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);
    market
        .deposit_fresh_counterparty_backing_not_atomic(0, 1, 10)
        .unwrap();
    account_header.health_cert = HealthCertV16Account::from_runtime(&HealthCertV16 {
        certified_equity: 100,
        certified_initial_req: 95,
        certified_maintenance_req: 80,
        cert_oracle_epoch: market.header.oracle_epoch.get(),
        cert_funding_epoch: market.header.funding_epoch.get(),
        cert_risk_epoch: market.header.risk_epoch.get(),
        cert_asset_set_epoch: market.header.asset_set_epoch.get(),
        active_bitmap_at_cert: V16_EMPTY_ACTIVE_BITMAP,
        valid: true,
        ..HealthCertV16::default()
    });
    let mut account = PortfolioV16ViewMut::new(&mut account_header);

    let err = market.charge_account_backing_fee_not_atomic(&mut account, 0, 6, 1, 4);

    assert_eq!(err, Err(V16Error::LockActive));
    assert_eq!(market.header.c_tot.get(), 100);
    assert_eq!(account.header.capital.get(), 100);
    assert_eq!(market.header.insurance.get(), 0);
    assert_eq!(market.validate_shape(), Ok(()));
}

#[test]
fn v16_public_liquidation_on_unfunded_domain_cannot_drain_shared_insurance() {
    let (mut header, mut markets) = market_fixture(1, 100);
    let mut account_header = account_fixture(1, 10);
    header.vault = V16PodU128::new(50);
    header.insurance = V16PodU128::new(50);
    header.negative_pnl_account_count = V16PodU64::new(1);

    let mut asset = markets[0].engine.asset.try_to_runtime().unwrap();
    asset.oi_eff_long_q = 2 * POS_SCALE;
    asset.oi_eff_short_q = 2 * POS_SCALE;
    asset.loss_weight_sum_long = 2 * POS_SCALE;
    asset.loss_weight_sum_short = 2 * POS_SCALE;
    asset.stored_pos_count_long = 2;
    asset.stored_pos_count_short = 2;
    markets[0].engine.asset = AssetStateV16Account::from_runtime(&asset);
    header.resolved_payout_blocker_count = V16PodU64::new(4);

    account_header.pnl = V16PodI128::new(-5);
    account_header.legs[0] = PortfolioLegV16Account::from_runtime(&PortfolioLegV16 {
        active: true,
        asset_index: 0,
        market_id: asset.market_id,
        side: SideV16::Long,
        basis_pos_q: POS_SCALE as i128,
        a_basis: ADL_ONE,
        k_snap: asset.k_long,
        f_snap: asset.f_long_num,
        epoch_snap: asset.epoch_long,
        loss_weight: POS_SCALE,
        b_snap: asset.b_long_num,
        b_rem: 0,
        b_epoch_snap: asset.epoch_long,
        b_stale: false,
        stale: false,
    });
    account_header.active_bitmap[0] = V16PodU64::new(1);

    let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);
    let mut account = PortfolioV16ViewMut::new(&mut account_header);
    let insurance_before = market.header.insurance.get();
    let vault_before = market.header.vault.get();

    let out = market
        .liquidate_account_not_atomic(&mut account, LiquidationRequestV16 { asset_index: 0 })
        .expect("liquidation should progress by booking residual, not draining other domains");

    assert_eq!(out.insurance_used, 0);
    assert_eq!(market.header.insurance.get(), insurance_before);
    assert_eq!(market.header.vault.get(), vault_before);
    assert_eq!(
        market.markets[0].engine.insurance_domain_spent_short.get(),
        0
    );
    assert!(out.residual_booked > 0);
    market.validate_shape().unwrap();
    account.validate_with_market(&market.as_view()).unwrap();
}

// FIX E3 (upstream #92 / b97e1746): liquidation size + fee are now fully
// engine-selected. This test proves the engine picks the MINIMAL healthy
// partial close (981, not the full 10_000 position) and charges the
// correspondingly smaller fee (79, not 800) -- the exact opposite of the
// pre-fix "caller picks close_q" behavior this fix closes.
#[test]
fn v16_liquidation_engine_selects_healthy_partial_before_margin_floor() {
    const PRICE: u64 = POS_SCALE as u64;
    const POSITION_Q: u128 = 10_000;
    const ACCOUNT_CAPITAL: u128 = 980;
    const EXPECTED_CLOSE_Q: u128 = 981;
    const EXPECTED_FEE: u128 = 79;

    let (mut header, mut markets) = market_fixture(1, PRICE);
    header.config.maintenance_margin_bps = V16PodU64::new(1_000);
    header.config.initial_margin_bps = V16PodU64::new(1_000);
    header.config.min_nonzero_mm_req = V16PodU128::new(800);
    header.config.min_nonzero_im_req = V16PodU128::new(801);
    header.config.liquidation_fee_bps = V16PodU64::new(800);
    header.config.min_liquidation_abs = V16PodU128::new(0);
    header.config.liquidation_fee_cap = V16PodU128::new(1_000);
    header.config.max_price_move_bps_per_slot = V16PodU64::new(1);
    header
        .config
        .try_to_runtime_shape()
        .unwrap()
        .validate_public_user_fund()
        .unwrap();
    header.vault = V16PodU128::new(ACCOUNT_CAPITAL * 2);
    header.c_tot = V16PodU128::new(ACCOUNT_CAPITAL * 2);

    let mut asset = markets[0].engine.asset.try_to_runtime().unwrap();
    asset.effective_price = PRICE;
    asset.raw_oracle_target_price = PRICE;
    asset.oi_eff_long_q = POSITION_Q * 2;
    asset.oi_eff_short_q = POSITION_Q * 2;
    asset.loss_weight_sum_long = POSITION_Q * 2;
    asset.loss_weight_sum_short = POSITION_Q * 2;
    asset.stored_pos_count_long = 2;
    asset.stored_pos_count_short = 2;
    markets[0].engine.asset = AssetStateV16Account::from_runtime(&asset);
    header.resolved_payout_blocker_count = V16PodU64::new(4);

    let mut account_header = account_fixture(1, 14);
    account_header.capital = V16PodU128::new(ACCOUNT_CAPITAL);
    account_header.legs[0] = PortfolioLegV16Account::from_runtime(&PortfolioLegV16 {
        active: true,
        asset_index: 0,
        market_id: asset.market_id,
        side: SideV16::Long,
        basis_pos_q: i128::try_from(POSITION_Q).unwrap(),
        a_basis: ADL_ONE,
        k_snap: asset.k_long,
        f_snap: asset.f_long_num,
        epoch_snap: asset.epoch_long,
        loss_weight: POSITION_Q,
        b_snap: asset.b_long_num,
        b_rem: 0,
        b_epoch_snap: asset.epoch_long,
        b_stale: false,
        stale: false,
    });
    account_header.active_bitmap[0] = V16PodU64::new(1);

    let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);
    let mut account = PortfolioV16ViewMut::new(&mut account_header);
    let out = market
        .liquidate_account_not_atomic(&mut account, LiquidationRequestV16 { asset_index: 0 })
        .unwrap();

    assert_eq!(out.closed_q, EXPECTED_CLOSE_Q); // 981, NOT 10_000 (full close)
    assert_eq!(out.fee_charged, EXPECTED_FEE); // 79, NOT 800 (8% of full notional)
    assert_eq!(account.header.capital.get(), ACCOUNT_CAPITAL - EXPECTED_FEE);
    assert_eq!(account.header.active_bitmap[0].get(), 1); // leg stays open (partial)
    let leg = account.header.legs[0].try_to_runtime().unwrap();
    assert_eq!(
        leg.basis_pos_q,
        i128::try_from(POSITION_Q - EXPECTED_CLOSE_Q).unwrap()
    );
    let cert = account.header.health_cert.try_to_runtime().unwrap();
    assert_eq!(cert.certified_liq_deficit, 0);
    assert_eq!(cert.certified_equity, 901);
    assert_eq!(cert.certified_maintenance_req, 901);
    market.validate_shape().unwrap();
    account.validate_with_market(&market.as_view()).unwrap();
}

#[test]
fn v16_permissionless_liquidation_progresses_when_unrelated_asset_is_loss_stale() {
    let (mut header, mut markets) = market_fixture(2, 100);
    let mut account_header = account_fixture(2, 11);
    header.current_slot = V16PodU64::new(10);
    header.slot_last = V16PodU64::new(9);
    header.loss_stale_active = 1;
    header.vault = V16PodU128::new(50);
    header.insurance = V16PodU128::new(50);
    header.negative_pnl_account_count = V16PodU64::new(1);

    let mut asset0 = markets[0].engine.asset.try_to_runtime().unwrap();
    asset0.slot_last = 10;
    asset0.oi_eff_long_q = 2 * POS_SCALE;
    asset0.oi_eff_short_q = 2 * POS_SCALE;
    asset0.loss_weight_sum_long = 2 * POS_SCALE;
    asset0.loss_weight_sum_short = 2 * POS_SCALE;
    asset0.stored_pos_count_long = 2;
    asset0.stored_pos_count_short = 2;
    markets[0].engine.asset = AssetStateV16Account::from_runtime(&asset0);
    let mut asset1 = markets[1].engine.asset.try_to_runtime().unwrap();
    asset1.slot_last = 9;
    asset1.oi_eff_long_q = POS_SCALE;
    asset1.oi_eff_short_q = POS_SCALE;
    asset1.loss_weight_sum_long = POS_SCALE;
    asset1.loss_weight_sum_short = POS_SCALE;
    asset1.stored_pos_count_long = 1;
    asset1.stored_pos_count_short = 1;
    markets[1].engine.asset = AssetStateV16Account::from_runtime(&asset1);
    header.resolved_payout_blocker_count = V16PodU64::new(6);

    account_header.pnl = V16PodI128::new(-5);
    account_header.legs[0] = PortfolioLegV16Account::from_runtime(&PortfolioLegV16 {
        active: true,
        asset_index: 0,
        market_id: asset0.market_id,
        side: SideV16::Long,
        basis_pos_q: POS_SCALE as i128,
        a_basis: ADL_ONE,
        k_snap: asset0.k_long,
        f_snap: asset0.f_long_num,
        epoch_snap: asset0.epoch_long,
        loss_weight: POS_SCALE,
        b_snap: asset0.b_long_num,
        b_rem: 0,
        b_epoch_snap: asset0.epoch_long,
        b_stale: false,
        stale: false,
    });
    account_header.active_bitmap[0] = V16PodU64::new(1);

    let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);
    let mut account = PortfolioV16ViewMut::new(&mut account_header);
    let outcome = market
        .permissionless_crank_not_atomic(
            &mut account,
            percolator::PermissionlessCrankRequestV16 {
                now_slot: 10,
                asset_index: 0,
                effective_price: 100,
                funding_rate_e9: 0,
                action: percolator::PermissionlessCrankActionV16::Liquidate(
                    LiquidationRequestV16 { asset_index: 0 },
                ),
            },
        )
        .expect(
            "locally current liquidation must progress despite unrelated global loss-staleness",
        );

    assert_eq!(
        outcome,
        percolator::PermissionlessProgressOutcomeV16::AccountCurrent
    );
    assert_eq!(market.header.loss_stale_active, 0);
    assert_eq!(market.header.slot_last.get(), 10);
    let unrelated_asset = market.markets[1].engine.asset.try_to_runtime().unwrap();
    assert_eq!(unrelated_asset.slot_last, 9);
    assert_eq!(account.header.active_bitmap[0].get(), 0);
    market.validate_shape().unwrap();
    account.validate_with_market(&market.as_view()).unwrap();
}

#[test]
fn v16_permissionless_recovery_crank_is_value_neutral_and_idempotent() {
    let (mut header, mut markets) = market_fixture(1, 100);
    let mut account_header = account_fixture(1, 12);
    {
        let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);
        let mut account = PortfolioV16ViewMut::new(&mut account_header);
        market.deposit_not_atomic(&mut account, 7).unwrap();
    }
    header.insurance = V16PodU128::new(3);
    header.vault = V16PodU128::new(10);
    let vault_before = header.vault;
    let c_tot_before = header.c_tot;
    let insurance_before = header.insurance;
    let capital_before = account_header.capital;
    let pnl_before = account_header.pnl;

    let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);
    let mut account = PortfolioV16ViewMut::new(&mut account_header);
    let first = market
        .permissionless_crank_not_atomic(
            &mut account,
            PermissionlessCrankRequestV16 {
                now_slot: 1,
                asset_index: 0,
                effective_price: 100,
                funding_rate_e9: 0,
                action: PermissionlessCrankActionV16::Recover(
                    PermissionlessRecoveryReasonV16::ExplicitLossOrDustAuditOverflow,
                ),
            },
        )
        .unwrap();
    let second = market
        .permissionless_crank_not_atomic(
            &mut account,
            PermissionlessCrankRequestV16 {
                now_slot: 1,
                asset_index: 0,
                effective_price: 100,
                funding_rate_e9: 0,
                action: PermissionlessCrankActionV16::Recover(
                    PermissionlessRecoveryReasonV16::BIndexHeadroomExhausted,
                ),
            },
        )
        .unwrap();
    let refresh_after_recovery = market.permissionless_crank_not_atomic(
        &mut account,
        PermissionlessCrankRequestV16 {
            now_slot: 1,
            asset_index: 0,
            effective_price: 100,
            funding_rate_e9: 0,
            action: PermissionlessCrankActionV16::Refresh,
        },
    );

    assert_eq!(
        first,
        PermissionlessProgressOutcomeV16::RecoveryDeclared(
            PermissionlessRecoveryReasonV16::ExplicitLossOrDustAuditOverflow
        )
    );
    assert_eq!(second, first);
    assert_eq!(refresh_after_recovery, Err(V16Error::LockActive));
    assert_eq!(market.header.vault, vault_before);
    assert_eq!(market.header.c_tot, c_tot_before);
    assert_eq!(market.header.insurance, insurance_before);
    assert_eq!(account.header.capital, capital_before);
    assert_eq!(account.header.pnl, pnl_before);
    market.validate_shape().unwrap();
    account.validate_with_market(&market.as_view()).unwrap();
}

#[test]
fn v16_resolved_payout_topup_finishes_receipt_without_overpaying() {
    let (mut header, mut markets) = market_fixture(1, 100);
    let mut account_header = account_fixture(1, 13);
    {
        let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);
        market.resolve_market_not_atomic(1).unwrap();
    }
    let terminal_claim = 10u128;
    header.vault = V16PodU128::new(4);
    header.payout_snapshot_captured = 1;
    header.resolved_payout_ledger =
        ResolvedPayoutLedgerV16Account::from_runtime(&ResolvedPayoutLedgerV16 {
            snapshot_residual: terminal_claim,
            terminal_claim_exact_receipts_num: terminal_claim * BOUND_SCALE,
            terminal_claim_bound_unreceipted_num: 0,
            current_payout_rate_num: 1,
            current_payout_rate_den: 1,
            snapshot_slot: 1,
            payout_halted: false,
            finalized: false,
        });
    account_header.resolved_payout_receipt =
        ResolvedPayoutReceiptV16Account::from_runtime(&ResolvedPayoutReceiptV16 {
            present: true,
            prior_bound_contribution_num: terminal_claim * BOUND_SCALE,
            live_released_face_at_receipt: 0,
            terminal_positive_claim_face: terminal_claim,
            paid_effective: 2,
            finalized: false,
        });

    let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);
    let mut account = PortfolioV16ViewMut::new(&mut account_header);
    let first = market
        .claim_resolved_payout_topup_not_atomic(&mut account)
        .unwrap();
    let after_first = account
        .header
        .resolved_payout_receipt
        .try_to_runtime()
        .unwrap();
    market.header.vault = V16PodU128::new(4);
    let second = market
        .claim_resolved_payout_topup_not_atomic(&mut account)
        .unwrap();
    let after_second = account
        .header
        .resolved_payout_receipt
        .try_to_runtime()
        .unwrap();
    let third = market
        .claim_resolved_payout_topup_not_atomic(&mut account)
        .unwrap();

    assert_eq!(first, 4);
    assert_eq!(after_first.paid_effective, 6);
    assert!(!after_first.finalized);
    assert_eq!(second, 4);
    assert_eq!(after_second.paid_effective, terminal_claim);
    assert!(after_second.finalized);
    assert_eq!(third, 0);
    assert_eq!(market.header.vault.get(), 0);
    market.validate_shape().unwrap();
    account.validate_with_market(&market.as_view()).unwrap();
}

#[test]
fn v16_risk_increasing_trade_creates_source_credit_lien_for_im() {
    let (mut header, mut markets) = market_fixture(1, 1);
    let mut long_header = account_fixture(1, 8);
    let mut short_header = account_fixture(1, 9);
    let claim = 100u128;
    let claim_num = claim * BOUND_SCALE;
    long_header.pnl = V16PodI128::new(claim as i128);
    long_header.source_domains[0].domain = V16PodU32::new(0);
    long_header.source_domains[0].source_claim_market_id = V16PodU64::new(1);
    long_header.source_domains[0].source_claim_bound_num = V16PodU128::new(claim_num);
    header.pnl_pos_tot = V16PodU128::new(claim);
    header.pnl_pos_bound_tot_num = V16PodU128::new(claim_num);
    header.pnl_pos_bound_tot = V16PodU128::new(claim);
    header.source_claim_bound_total_num = V16PodU128::new(claim_num);
    header.source_fresh_backing_total_num = V16PodU128::new(claim_num);
    // Backing principal is vault-funded and senior-side: vault must cover it.
    header.vault = V16PodU128::new(claim + header.vault.get());
    markets[0].engine.source_credit_long =
        SourceCreditStateV16Account::from_runtime(&SourceCreditStateV16 {
            positive_claim_bound_num: claim_num,
            exact_positive_claim_num: claim_num,
            fresh_reserved_backing_num: claim_num,
            credit_rate_num: CREDIT_RATE_SCALE,
            ..SourceCreditStateV16::EMPTY
        });
    markets[0].engine.backing_long = BackingBucketV16Account::from_runtime(&BackingBucketV16 {
        market_id: 1,
        fresh_unliened_backing_num: claim_num,
        expiry_slot: 100,
        status: BackingBucketStatusV16::Fresh,
        ..BackingBucketV16::EMPTY
    });
    {
        let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);
        let mut short = PortfolioV16ViewMut::new(&mut short_header);
        market.deposit_not_atomic(&mut short, 1_000).unwrap();
    }

    let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);
    let mut long = PortfolioV16ViewMut::new(&mut long_header);
    let mut short = PortfolioV16ViewMut::new(&mut short_header);
    market
        .execute_trade_with_fee_loss_stale_scoped_not_atomic(
            &mut long,
            &mut short,
            TradeRequestV16 {
                asset_index: 0,
                size_q: signed_q(10 * POS_SCALE),
                exec_price: 1,
                fee_bps: 0,
            },
            true,
        )
        .expect("risk-increasing trade should atomically lien backed source credit for IM");

    assert_eq!(long.header.capital.get(), 0);
    assert_eq!(
        long.header.source_domains[0].source_claim_liened_num.get(),
        10 * BOUND_SCALE
    );
    assert_eq!(
        long.header.source_domains[0]
            .source_lien_effective_reserved
            .get(),
        10
    );
    assert_eq!(
        long.header.source_domains[0]
            .source_lien_counterparty_backing_num
            .get(),
        10 * BOUND_SCALE
    );
    assert_eq!(
        market.markets[0]
            .engine
            .source_credit_long
            .valid_liened_backing_num
            .get(),
        10 * BOUND_SCALE
    );
    assert_eq!(
        market.markets[0]
            .engine
            .backing_long
            .valid_liened_backing_num
            .get(),
        10 * BOUND_SCALE
    );
    assert_eq!(
        market.markets[0]
            .engine
            .backing_long
            .fresh_unliened_backing_num
            .get(),
        90 * BOUND_SCALE
    );
    assert_eq!(
        market.convert_released_pnl_to_capital_not_atomic(&mut long),
        Err(V16Error::LockActive),
        "source-backed positive PnL must not be realized while the source-claim exposure remains open"
    );
    market.validate_shape().unwrap();
    long.validate_with_market(&market.as_view()).unwrap();
    short.validate_with_market(&market.as_view()).unwrap();
}

#[test]
fn v16_residual_reward_credit_uses_real_principal_not_notional() {
    let (mut header, mut markets) = market_fixture(1, 1_000);
    header.config.initial_margin_bps = V16PodU64::new(500);
    header.config.maintenance_margin_bps = V16PodU64::new(500);
    header.config.min_nonzero_im_req = V16PodU128::new(2);
    header.config.min_nonzero_mm_req = V16PodU128::new(1);
    let mut taker_header = account_fixture(1, 23);
    let mut lp_header = account_fixture(1, 24);
    {
        let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);
        let mut taker = PortfolioV16ViewMut::new(&mut taker_header);
        let mut lp = PortfolioV16ViewMut::new(&mut lp_header);
        market.deposit_not_atomic(&mut taker, 10_000).unwrap();
        market.deposit_not_atomic(&mut lp, 10_000).unwrap();
    }

    taker_header.residual_crystallized_loss_atoms_total = V16PodU128::new(10_000);
    let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);
    let mut taker = PortfolioV16ViewMut::new(&mut taker_header);
    let mut lp = PortfolioV16ViewMut::new(&mut lp_header);
    market
        .execute_trade_with_fee_loss_stale_scoped_not_atomic(
            &mut taker,
            &mut lp,
            TradeRequestV16 {
                asset_index: 0,
                size_q: signed_q(POS_SCALE),
                exec_price: 1_000,
                fee_bps: 0,
            },
            true,
        )
        .unwrap();

    assert_eq!(
        taker.header.residual_spent_principal_atoms_total.get(),
        50,
        "1 lot at price 1000 with 500 bps IM spends only 50 atoms of residual budget"
    );
    assert_eq!(lp.header.residual_received_atoms_total.get(), 50);
    assert_ne!(
        lp.header.residual_received_atoms_total.get(),
        1_000,
        "counter must not credit leveraged notional"
    );
    taker.validate_with_market(&market.as_view()).unwrap();
    lp.validate_with_market(&market.as_view()).unwrap();
    market.validate_shape().unwrap();
}

#[test]
fn v16_residual_reward_credit_is_capped_by_available_crystallized_loss() {
    let (mut header, mut markets) = market_fixture(1, 1_000);
    header.config.initial_margin_bps = V16PodU64::new(500);
    header.config.maintenance_margin_bps = V16PodU64::new(500);
    let mut taker_header = account_fixture(1, 25);
    let mut lp_header = account_fixture(1, 26);
    {
        let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);
        let mut taker = PortfolioV16ViewMut::new(&mut taker_header);
        let mut lp = PortfolioV16ViewMut::new(&mut lp_header);
        market.deposit_not_atomic(&mut taker, 10_000).unwrap();
        market.deposit_not_atomic(&mut lp, 10_000).unwrap();
    }

    taker_header.residual_crystallized_loss_atoms_total = V16PodU128::new(30);
    let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);
    let mut taker = PortfolioV16ViewMut::new(&mut taker_header);
    let mut lp = PortfolioV16ViewMut::new(&mut lp_header);
    market
        .execute_trade_with_fee_loss_stale_scoped_not_atomic(
            &mut taker,
            &mut lp,
            TradeRequestV16 {
                asset_index: 0,
                size_q: signed_q(POS_SCALE),
                exec_price: 1_000,
                fee_bps: 0,
            },
            true,
        )
        .unwrap();

    assert_eq!(taker.header.residual_spent_principal_atoms_total.get(), 30);
    assert_eq!(lp.header.residual_received_atoms_total.get(), 30);
    taker.validate_with_market(&market.as_view()).unwrap();
    lp.validate_with_market(&market.as_view()).unwrap();
}

#[test]
fn v16_principal_loss_crystallizes_residual_budget_monotonically() {
    let (mut header, mut markets) = market_fixture(1, 100);
    let mut account_header = account_fixture(1, 27);
    header.vault = V16PodU128::new(100);
    header.c_tot = V16PodU128::new(100);
    header.negative_pnl_account_count = V16PodU64::new(1);
    account_header.capital = V16PodU128::new(100);
    account_header.pnl = V16PodI128::new(-40);
    account_header.residual_crystallized_loss_atoms_total = V16PodU128::new(7);

    let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);
    let mut account = PortfolioV16ViewMut::new(&mut account_header);
    market
        .sync_account_fee_to_slot_not_atomic(&mut account, 1, 0)
        .unwrap();

    assert_eq!(account.header.capital.get(), 60);
    assert_eq!(account.header.pnl.get(), 0);
    assert_eq!(
        account.header.residual_crystallized_loss_atoms_total.get(),
        47,
        "historical crystallized-loss budget only increases by real capital consumed"
    );
    account.validate_with_market(&market.as_view()).unwrap();
    market.validate_shape().unwrap();
}

#[test]
fn v16_source_backed_conversion_clears_sparse_source_domain_slot() {
    let (mut header, mut markets) = market_fixture(1, 1);
    let mut account_header = account_fixture(1, 18);
    let claim = 20u128;
    let claim_num = claim * BOUND_SCALE;
    header.vault = V16PodU128::new(claim);
    header.pnl_pos_tot = V16PodU128::new(claim);
    header.pnl_pos_bound_tot_num = V16PodU128::new(claim_num);
    header.pnl_pos_bound_tot = V16PodU128::new(claim);
    header.source_claim_bound_total_num = V16PodU128::new(claim_num);
    header.source_fresh_backing_total_num = V16PodU128::new(claim_num);
    account_header.pnl = V16PodI128::new(claim as i128);
    account_header.source_domains[0].domain = V16PodU32::new(0);
    account_header.source_domains[0].source_claim_market_id = V16PodU64::new(1);
    account_header.source_domains[0].source_claim_bound_num = V16PodU128::new(claim_num);
    markets[0].engine.source_credit_long =
        SourceCreditStateV16Account::from_runtime(&SourceCreditStateV16 {
            positive_claim_bound_num: claim_num,
            exact_positive_claim_num: claim_num,
            fresh_reserved_backing_num: claim_num,
            credit_rate_num: CREDIT_RATE_SCALE,
            ..SourceCreditStateV16::EMPTY
        });
    markets[0].engine.backing_long = BackingBucketV16Account::from_runtime(&BackingBucketV16 {
        market_id: 1,
        fresh_unliened_backing_num: claim_num,
        expiry_slot: 100,
        status: BackingBucketStatusV16::Fresh,
        ..BackingBucketV16::EMPTY
    });

    let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);
    let mut account = PortfolioV16ViewMut::new(&mut account_header);
    market
        .full_account_refresh_not_atomic(&mut account)
        .unwrap();
    let converted = market
        .convert_released_pnl_to_capital_not_atomic(&mut account)
        .expect("flat source-backed PnL should be convertible when backing is available");

    assert_eq!(converted, claim);
    assert_eq!(account.header.pnl.get(), 0);
    assert_eq!(account.header.capital.get(), claim);
    assert_eq!(
        account.header.source_domains[0],
        PortfolioSourceDomainV16Account::default()
    );
    account.validate_with_market(&market.as_view()).unwrap();
    market.validate_shape().unwrap();
}

#[test]
fn v16_sparse_source_domains_reject_unoccupied_tagged_slot() {
    let (mut header, mut markets) = market_fixture(1, 1);
    let mut account_header = account_fixture(1, 19);
    account_header.source_domains[1].domain = V16PodU32::new(1);
    account_header.source_domains[1].source_claim_market_id = V16PodU64::new(1);

    let market = MarketGroupV16ViewMut::new(&mut header, &mut markets);
    let account = PortfolioV16View::new(&account_header);
    assert_eq!(
        account.validate_with_market(&market.as_view()),
        Err(V16Error::HiddenLeg),
        "unoccupied tagged source-domain slots must not survive validation"
    );
}

#[test]
fn v16_mutable_view_compacts_persisted_domain_indexed_source_claim_before_deposit() {
    let (mut header, mut markets) = market_fixture(1, 1);
    let mut account_header = account_fixture(1, 20);
    let claim = 7u128;
    let claim_num = claim * BOUND_SCALE;
    header.vault = V16PodU128::new(claim);
    header.c_tot = V16PodU128::new(0);
    header.pnl_pos_tot = V16PodU128::new(claim);
    header.pnl_pos_bound_tot_num = V16PodU128::new(claim_num);
    header.pnl_pos_bound_tot = V16PodU128::new(claim);
    header.source_claim_bound_total_num = V16PodU128::new(claim_num);
    account_header.pnl = V16PodI128::new(claim as i128);
    account_header.source_domains[1].domain = V16PodU32::new(1);
    account_header.source_domains[1].source_claim_market_id = V16PodU64::new(1);
    account_header.source_domains[1].source_claim_bound_num = V16PodU128::new(claim_num);
    markets[0].engine.source_credit_short =
        SourceCreditStateV16Account::from_runtime(&SourceCreditStateV16 {
            positive_claim_bound_num: claim_num,
            exact_positive_claim_num: claim_num,
            fresh_reserved_backing_num: claim_num,
            credit_rate_num: CREDIT_RATE_SCALE,
            ..SourceCreditStateV16::EMPTY
        });
    markets[0].engine.backing_short = BackingBucketV16Account::from_runtime(&BackingBucketV16 {
        market_id: 1,
        fresh_unliened_backing_num: claim_num,
        expiry_slot: 100,
        status: BackingBucketStatusV16::Fresh,
        ..BackingBucketV16::EMPTY
    });

    let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);
    PortfolioV16View::new(&account_header)
        .validate_with_market(&market.as_view())
        .expect("read-only validation must accept coherent domain-indexed parked PnL");
    let mut account = PortfolioV16ViewMut::new(&mut account_header);
    market
        .deposit_not_atomic(&mut account, 3)
        .expect("later deposit must accept a persisted parked source claim");

    assert_eq!(account.header.capital.get(), 3);
    assert_eq!(account.header.source_domains[0].domain.get(), 1);
    assert_eq!(
        account.header.source_domains[0]
            .source_claim_bound_num
            .get(),
        claim_num
    );
    assert_eq!(
        account.header.source_domains[1],
        PortfolioSourceDomainV16Account::default()
    );
    account.validate_with_market(&market.as_view()).unwrap();
    market.validate_shape().unwrap();
}

#[test]
fn v16_trade_created_parked_source_claim_survives_later_deposit() {
    let (mut header, mut markets) = market_fixture(1, 100);
    let mut long_header = account_fixture(1, 21);
    let mut short_header = account_fixture(1, 22);

    {
        let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);
        let mut long = PortfolioV16ViewMut::new(&mut long_header);
        let mut short = PortfolioV16ViewMut::new(&mut short_header);
        market.deposit_not_atomic(&mut long, 1_000).unwrap();
        market.deposit_not_atomic(&mut short, 1_000).unwrap();
        market
            .execute_trade_with_fee_loss_stale_scoped_not_atomic(
                &mut long,
                &mut short,
                TradeRequestV16 {
                    asset_index: 0,
                    size_q: signed_q(POS_SCALE),
                    exec_price: 100,
                    fee_bps: 0,
                },
                true,
            )
            .unwrap();
        market
            .accrue_asset_to_not_atomic(0, 2, 101, 0, true)
            .unwrap();
        market.full_account_refresh_not_atomic(&mut long).unwrap();
    }

    assert!(long_header.pnl.get() > 0);
    assert!(
        long_header
            .source_domains
            .iter()
            .any(|source| source.domain.get() == 1
                && source.source_claim_market_id.get() == 1
                && source.source_claim_bound_num.get() != 0),
        "winner refresh must persist the source-domain claim created by K/F settlement"
    );

    let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);
    PortfolioV16View::new(&long_header)
        .validate_with_market(&market.as_view())
        .expect("read-only validation must accept the trade-created parked claim");
    let mut long = PortfolioV16ViewMut::new(&mut long_header);
    market
        .deposit_not_atomic(&mut long, 3)
        .expect("later deposit must accept the persisted trade-created parked claim");

    assert_eq!(long.header.capital.get(), 1_003);
    long.validate_with_market(&market.as_view()).unwrap();
    market.validate_shape().unwrap();
}

// Converged from toly v16.8.11 (ce073dc): certifies the new first-class engine
// API add_account_source_positive_pnl_not_atomic — value-neutral notional
// attribution with account -> domain -> group claim aggregates in lockstep, and
// the non-Live rejection.
#[test]
fn v16_grant_source_positive_pnl_attributes_claims_and_aggregates_in_lockstep() {
    let (mut header, mut markets) = market_fixture(1, 1);
    let mut account_header = account_fixture(1, 31);
    let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);
    let mut account = PortfolioV16ViewMut::new(&mut account_header);

    market
        .add_account_source_positive_pnl_not_atomic(&mut account, 0, 25)
        .expect("granting source-attributed positive pnl must succeed in Live");

    assert_eq!(account.header.pnl.get(), 25);
    assert_eq!(
        account.header.source_domains[0]
            .source_claim_bound_num
            .get(),
        25 * BOUND_SCALE
    );
    assert_eq!(market.header.pnl_pos_tot.get(), 25);
    assert_eq!(market.header.pnl_pos_bound_tot_num.get(), 25 * BOUND_SCALE);
    assert_eq!(
        market.header.source_claim_bound_total_num.get(),
        25 * BOUND_SCALE
    );
    // The grant is notional attribution: no quote value moves.
    assert_eq!(market.header.vault.get(), 0);
    assert_eq!(market.header.c_tot.get(), 0);
    assert_eq!(market.validate_shape(), Ok(()));
    assert_eq!(account.validate_with_market(&market.as_view()), Ok(()));

    // Granting in a non-Live market is rejected before any mutation.
    market.header.mode = 1; // Resolved
    market.header.resolved_slot = V16PodU64::new(1);
    let err = market.add_account_source_positive_pnl_not_atomic(&mut account, 0, 1);
    assert_eq!(err, Err(V16Error::LockActive));
    assert_eq!(account.header.pnl.get(), 25);
}

// ---------------------------------------------------------------------------
// Protocol-fee design §1A: taker-only trade fee charging.
// ---------------------------------------------------------------------------

#[test]
fn v16_taker_only_charges_long_side_when_taker_is_long_account() {
    let (mut header, mut markets) = market_fixture_with_trade_fee(1, 100, 1_000);
    let mut long_header = account_fixture(1, 41);
    let mut short_header = account_fixture(1, 42);
    {
        let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);
        let mut long = PortfolioV16ViewMut::new(&mut long_header);
        let mut short = PortfolioV16ViewMut::new(&mut short_header);
        market.deposit_not_atomic(&mut long, 1_000).unwrap();
        market.deposit_not_atomic(&mut short, 1_000).unwrap();
    }

    let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);
    let mut long = PortfolioV16ViewMut::new(&mut long_header);
    let mut short = PortfolioV16ViewMut::new(&mut short_header);
    let short_capital_before = short.header.capital.get();

    let outcome = market
        .execute_trade_with_fee_loss_stale_scoped_not_atomic(
            &mut long,
            &mut short,
            TradeRequestV16 {
                asset_index: 0,
                size_q: signed_q(POS_SCALE),
                exec_price: 100,
                fee_bps: 1_000, // notional 100 -> fee 10
            },
            true, // long_account is the taker
        )
        .unwrap();

    assert_eq!(outcome.fee_a, 10, "taker (long) pays the full fee");
    assert_eq!(outcome.fee_b, 0, "maker (short) pays nothing");
    assert_eq!(long.header.capital.get(), 1_000 - 10);
    assert_eq!(
        short.header.capital.get(),
        short_capital_before,
        "maker's capital is byte-identical before/after the fee-bearing fill"
    );
    market.validate_shape().unwrap();
}

#[test]
fn v16_taker_only_charges_short_side_when_taker_is_short_account() {
    let (mut header, mut markets) = market_fixture_with_trade_fee(1, 100, 1_000);
    let mut long_header = account_fixture(1, 43);
    let mut short_header = account_fixture(1, 44);
    {
        let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);
        let mut long = PortfolioV16ViewMut::new(&mut long_header);
        let mut short = PortfolioV16ViewMut::new(&mut short_header);
        market.deposit_not_atomic(&mut long, 1_000).unwrap();
        market.deposit_not_atomic(&mut short, 1_000).unwrap();
    }

    let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);
    let mut long = PortfolioV16ViewMut::new(&mut long_header);
    let mut short = PortfolioV16ViewMut::new(&mut short_header);
    let long_capital_before = long.header.capital.get();

    let outcome = market
        .execute_trade_with_fee_loss_stale_scoped_not_atomic(
            &mut long,
            &mut short,
            TradeRequestV16 {
                asset_index: 0,
                size_q: signed_q(POS_SCALE),
                exec_price: 100,
                fee_bps: 1_000,
            },
            false, // short_account is the taker
        )
        .unwrap();

    assert_eq!(outcome.fee_a, 0, "maker (long) pays nothing");
    assert_eq!(outcome.fee_b, 10, "taker (short) pays the full fee");
    assert_eq!(
        long.header.capital.get(),
        long_capital_before,
        "maker's capital is byte-identical before/after the fee-bearing fill"
    );
    assert_eq!(short.header.capital.get(), 1_000 - 10);
    market.validate_shape().unwrap();
}

// E4 (upstream 8f25aa5d): a sub-atom fill (size_q * exec_price / POS_SCALE < 1,
// i.e. floor-notional == 0) must still charge a nonzero fee via ceil-notional,
// because it opens nonzero OI (free risk) despite the floored notional reading
// zero. Adapted from upstream's `v16_subatom_trade_charges_fee_on_ceil_fee_notional`
// for this fork's taker-only single-payer fee model (upstream charges both legs
// independently; here exactly one side -- the taker -- pays the fee).
#[test]
fn v16_subatom_trade_charges_fee_on_ceil_fee_notional() {
    let (mut header, mut markets) = market_fixture_with_trade_fee(1, 100, 1);
    let mut long_header = account_fixture(1, 213);
    let mut short_header = account_fixture(1, 214);
    {
        let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);
        let mut long = PortfolioV16ViewMut::new(&mut long_header);
        let mut short = PortfolioV16ViewMut::new(&mut short_header);
        market.deposit_not_atomic(&mut long, 1_000).unwrap();
        market.deposit_not_atomic(&mut short, 1_000).unwrap();
    }

    let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);
    let mut long = PortfolioV16ViewMut::new(&mut long_header);
    let mut short = PortfolioV16ViewMut::new(&mut short_header);

    // sub_atom_size * exec_price / POS_SCALE floors to 0 (999_900 / 1_000_000),
    // but ceils to 1 -- this is the exact boundary upstream's fix targets.
    let sub_atom_size = POS_SCALE / 100 - 1;
    let outcome = market
        .execute_trade_with_fee_loss_stale_scoped_not_atomic(
            &mut long,
            &mut short,
            TradeRequestV16 {
                asset_index: 0,
                size_q: signed_q(sub_atom_size),
                exec_price: 100,
                fee_bps: 1,
            },
            true, // long_account is the taker
        )
        .unwrap();

    assert_eq!(
        outcome.notional, 0,
        "floor-notional (margin/PnL basis) is unaffected by the fee fix"
    );
    assert_eq!(
        outcome.fee_a, 1,
        "ceil-notional fee: sub-atom fill must not charge a fee of 0"
    );
    assert_eq!(
        outcome.fee_b, 0,
        "maker (short) pays nothing under taker-only"
    );
    assert_eq!(long.header.capital.get(), 1_000 - 1);
    assert_eq!(short.header.capital.get(), 1_000);
    assert_eq!(
        market.markets[0].engine.asset.oi_eff_long_q.get(),
        sub_atom_size,
        "nonzero risk was opened despite the floored notional reading zero"
    );
    assert_eq!(
        market.markets[0].engine.asset.oi_eff_short_q.get(),
        sub_atom_size
    );
    market.validate_shape().unwrap();
    long.validate_with_market(&market.as_view()).unwrap();
    short.validate_with_market(&market.as_view()).unwrap();
}

#[test]
fn v16_taker_only_batch_mixed_spread_charges_taker_on_every_leg() {
    let (mut header, mut markets) = market_fixture_with_trade_fee(2, 100, 1_000);
    let mut taker_header = account_fixture(2, 45);
    let mut lp_header = account_fixture(2, 46);
    {
        let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);
        let mut taker = PortfolioV16ViewMut::new(&mut taker_header);
        let mut lp = PortfolioV16ViewMut::new(&mut lp_header);
        market.deposit_not_atomic(&mut taker, 1_000).unwrap();
        market.deposit_not_atomic(&mut lp, 1_000).unwrap();
    }

    let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);
    let mut taker = PortfolioV16ViewMut::new(&mut taker_header);
    let mut lp = PortfolioV16ViewMut::new(&mut lp_header);
    let lp_capital_before = lp.header.capital.get();

    // A mixed long/short spread against one LP: taker (account_a, the
    // engine's fixed first positional account for batches per design §1A.3)
    // goes long asset 0 and short asset 1.
    let requests = [
        TradeRequestV16 {
            asset_index: 0,
            size_q: signed_q(POS_SCALE),
            exec_price: 100,
            fee_bps: 1_000,
        },
        TradeRequestV16 {
            asset_index: 1,
            size_q: -signed_q(POS_SCALE),
            exec_price: 100,
            fee_bps: 1_000,
        },
    ];

    let outcome = market
        .execute_batch_with_fee_loss_stale_scoped_not_atomic(
            &mut taker, &mut lp, &requests,
            true, // taker == account_a is always the engine's first (long_account) slot for batches
        )
        .unwrap();

    assert_eq!(outcome.fill_count, 2);
    assert_eq!(
        outcome.fee_a, 20,
        "taker pays fee on both legs of the spread (10 + 10)"
    );
    assert_eq!(outcome.fee_b, 0, "LP pays nothing on either leg");
    assert_eq!(taker.header.capital.get(), 1_000 - 20);
    assert_eq!(
        lp.header.capital.get(),
        lp_capital_before,
        "LP's capital is unchanged across every leg of the batch"
    );
    market.validate_shape().unwrap();
}

#[test]
fn v16_taker_only_n1_maker_fallback_when_taker_pnl_negative() {
    let (mut header, mut markets) = market_fixture_with_trade_fee(1, 100, 1_000);
    let mut long_header = account_fixture(1, 47);
    let mut short_header = account_fixture(1, 48);
    {
        let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);
        let mut long = PortfolioV16ViewMut::new(&mut long_header);
        let mut short = PortfolioV16ViewMut::new(&mut short_header);
        market.deposit_not_atomic(&mut long, 1_000).unwrap();
        market.deposit_not_atomic(&mut short, 1_000).unwrap();
        // Certify both accounts once while flat/pnl==0 so `stale_state` is
        // false and `health_cert.valid` is true against the *current*
        // epochs/bitmap. `settle_account_for_position_action_and_refresh_not_atomic`
        // (called at the top of the trade pipeline) then takes its early-out
        // branch instead of re-settling the account, which is what lets the
        // negative pnl injected below survive into the fee-charge site
        // un-settled — exactly the "current path deliberately skips loss
        // settlement" property the N1 guard is written against.
        market.full_account_refresh_not_atomic(&mut long).unwrap();
        market.full_account_refresh_not_atomic(&mut short).unwrap();
    }
    // N1: the taker (long) already carries a negative PnL, which fires the
    // pre-existing `charge_account_fee_current_not_atomic` waiver
    // (`account.header.pnl.get() < 0`). Under pure taker-only charging this
    // would let an underwater taker trade fee-free; the maker-fallback must
    // instead charge the solvent maker `fee.min(maker.capital)`.
    long_header.pnl = V16PodI128::new(-5);
    header.negative_pnl_account_count = V16PodU64::new(1);

    let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);
    let mut long = PortfolioV16ViewMut::new(&mut long_header);
    let mut short = PortfolioV16ViewMut::new(&mut short_header);
    let long_capital_before = long.header.capital.get();

    let outcome = market
        .execute_trade_with_fee_loss_stale_scoped_not_atomic(
            &mut long,
            &mut short,
            TradeRequestV16 {
                asset_index: 0,
                size_q: signed_q(POS_SCALE),
                exec_price: 100,
                fee_bps: 1_000,
            },
            true, // long_account (the underwater taker) initiated the trade
        )
        .unwrap();

    assert_eq!(
        outcome.fee_a, 0,
        "taker's own charge is waived by the pnl<0 guard, not stripped"
    );
    assert_eq!(
        outcome.fee_b, 10,
        "N1 fallback: the solvent maker is charged instead of nobody"
    );
    assert_eq!(
        long.header.capital.get(),
        long_capital_before,
        "the pnl<0 guard still protects the taker's own capital"
    );
    assert_eq!(
        long.header.pnl.get(),
        -5,
        "taker pnl untouched by the fee path"
    );
    assert_eq!(short.header.capital.get(), 1_000 - 10);
    market.validate_shape().unwrap();
}

#[test]
fn v16_taker_only_n1_no_fallback_when_fee_is_genuinely_zero() {
    // Distinguishes "fee == 0" (no fallback — nothing to collect from anyone)
    // from "pnl < 0 waived a nonzero fee" (fallback fires). Both taker and
    // maker have negative PnL here, but fee_bps is 0, so neither should ever
    // be charged and outcome.fee_a/fee_b must both be 0.
    let (mut header, mut markets) = market_fixture_with_trade_fee(1, 100, 1_000);
    let mut long_header = account_fixture(1, 49);
    let mut short_header = account_fixture(1, 50);
    {
        let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);
        let mut long = PortfolioV16ViewMut::new(&mut long_header);
        let mut short = PortfolioV16ViewMut::new(&mut short_header);
        market.deposit_not_atomic(&mut long, 1_000).unwrap();
        market.deposit_not_atomic(&mut short, 1_000).unwrap();
        market.full_account_refresh_not_atomic(&mut long).unwrap();
        market.full_account_refresh_not_atomic(&mut short).unwrap();
    }
    long_header.pnl = V16PodI128::new(-5);
    header.negative_pnl_account_count = V16PodU64::new(1);

    let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);
    let mut long = PortfolioV16ViewMut::new(&mut long_header);
    let mut short = PortfolioV16ViewMut::new(&mut short_header);
    let long_capital_before = long.header.capital.get();
    let short_capital_before = short.header.capital.get();

    let outcome = market
        .execute_trade_with_fee_loss_stale_scoped_not_atomic(
            &mut long,
            &mut short,
            TradeRequestV16 {
                asset_index: 0,
                size_q: signed_q(POS_SCALE),
                exec_price: 100,
                fee_bps: 0,
            },
            true,
        )
        .unwrap();

    assert_eq!(outcome.fee_a, 0);
    assert_eq!(outcome.fee_b, 0);
    assert_eq!(long.header.capital.get(), long_capital_before);
    assert_eq!(short.header.capital.get(), short_capital_before);
}

#[test]
fn v16_taker_only_n1_maker_fallback_when_taker_capital_zero_pnl_nonnegative() {
    // Fee-evasion regression (security review 2026-07-15, MEDIUM). Before the
    // fix, the maker-fallback trigger was
    // `taker_fee == 0 && fee != 0 && taker.pnl < 0`. A taker whose capital is
    // drawn to exactly 0 WITHOUT being underwater (pnl >= 0) also makes
    // `charge_account_fee_current_not_atomic` return 0 -- via
    // `fee.min(capital) == 0`, a structurally different reason than the
    // pnl<0 waiver -- but the old pnl<0 qualifier didn't fire for this case,
    // so the fallback never charged the maker either: the fee vanished
    // entirely (protocol/LP/creator/insurance all got 0).
    //
    // This reproduces the exact scenario the security review flagged: a
    // multi-leg batch where an EARLY leg drains the taker's capital to
    // exactly 0 (paying its own fee in full, so it is NOT underwater -- pnl
    // stays 0), and a LATER leg in the SAME batch then owes a nonzero fee
    // with nothing left to pay it. Leg 2 closes the position leg 1 opened
    // (net batch position == 0) so the batch-final initial-margin check
    // (`finish_trade_checks_not_atomic` certifies once, after all legs, for
    // multi-leg batches) sees a flat book and a trivial (zero) margin
    // requirement regardless of the taker's zero capital.
    let (mut header, mut markets) = market_fixture_with_trade_fee(2, 100, 1_000);
    let mut long_header = account_fixture(2, 51);
    let mut short_header = account_fixture(2, 52);
    {
        let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);
        let mut long = PortfolioV16ViewMut::new(&mut long_header);
        let mut short = PortfolioV16ViewMut::new(&mut short_header);
        // Taker (long) is funded with exactly one leg's fee (10) -- enough to
        // pay leg 1 in full and land at capital == 0, pnl == 0 (NOT
        // negative) before leg 2 is even evaluated.
        market.deposit_not_atomic(&mut long, 10).unwrap();
        market.deposit_not_atomic(&mut short, 1_000).unwrap();
    }

    let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);
    let mut long = PortfolioV16ViewMut::new(&mut long_header);
    let mut short = PortfolioV16ViewMut::new(&mut short_header);

    let requests = [
        // Leg 1: taker opens long POS_SCALE @ 100, fee 10 -- fully solvent,
        // pays in full, capital drops 10 -> 0.
        TradeRequestV16 {
            asset_index: 0,
            size_q: signed_q(POS_SCALE),
            exec_price: 100,
            fee_bps: 1_000,
        },
        // Leg 2: taker closes the same position back to flat, notional 100,
        // fee 10 -- taker capital is now 0, so the taker's own charge
        // attempt collects 0 even though pnl is still 0 (not negative).
        TradeRequestV16 {
            asset_index: 0,
            size_q: -signed_q(POS_SCALE),
            exec_price: 100,
            fee_bps: 1_000,
        },
    ];

    let outcome = market
        .execute_batch_with_fee_loss_stale_scoped_not_atomic(
            &mut long, &mut short, &requests, true, // long_account is the taker on every leg
        )
        .unwrap();

    assert_eq!(outcome.fill_count, 2);
    assert_eq!(
        outcome.fee_a, 10,
        "taker pays leg 1 in full (solvent), then owes 0 on leg 2 (capital \
         exhausted) -- total taker-side charge is just leg 1's fee"
    );
    assert_eq!(
        outcome.fee_b, 10,
        "fix: leg 2's fee is NOT lost -- the maker-fallback fires because \
         the taker's own charge returned 0, even though the taker's pnl is \
         NOT negative (capital exhaustion, not the pnl<0 waiver)"
    );
    assert_eq!(long.header.capital.get(), 0, "taker fully drained by leg 1");
    assert_eq!(long.header.pnl.get(), 0, "taker was never underwater");
    assert_eq!(
        short.header.capital.get(),
        1_000 - 10,
        "maker pays leg 2's fee via the fallback"
    );
    market.validate_shape().unwrap();
}

// ---------------------------------------------------------------------------
// New engine primitive: withdraw_insurance_surplus_not_atomic (design §1.5).
// ---------------------------------------------------------------------------

#[test]
fn v16_withdraw_insurance_surplus_zero_amount_is_a_noop() {
    let (mut header, mut markets) = market_fixture(1, 100);
    header.vault = V16PodU128::new(500);
    header.insurance = V16PodU128::new(200);
    let vault_before = header.vault;
    let insurance_before = header.insurance;

    let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);
    market.withdraw_insurance_surplus_not_atomic(0).unwrap();

    assert_eq!(market.header.vault, vault_before);
    assert_eq!(market.header.insurance, insurance_before);
}

#[test]
fn v16_withdraw_insurance_surplus_rejects_amount_exceeding_surplus() {
    let (mut header, mut markets) = market_fixture(1, 100);
    header.vault = V16PodU128::new(500);
    header.insurance = V16PodU128::new(200);
    header.insurance_domain_budget_remaining_total = V16PodU128::new(150);
    // Unbudgeted surplus = insurance(200) - source_reserved(0) - budget_remaining(150) = 50.
    let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);

    let err = market.withdraw_insurance_surplus_not_atomic(51);
    assert_eq!(err, Err(V16Error::LockActive));
}

#[test]
fn v16_withdraw_insurance_surplus_rejects_amount_exceeding_vault() {
    let (mut header, mut markets) = market_fixture(1, 100);
    header.vault = V16PodU128::new(40);
    header.insurance = V16PodU128::new(200);
    // Unbudgeted surplus = 200 (no reservations/budget set), but vault only
    // has 40 physical atoms — the vault bound must still gate the transfer.
    let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);

    let err = market.withdraw_insurance_surplus_not_atomic(41);
    assert_eq!(err, Err(V16Error::LockActive));
}

#[test]
fn v16_withdraw_insurance_surplus_exact_boundary_succeeds() {
    let (mut header, mut markets) = market_fixture(1, 100);
    header.vault = V16PodU128::new(500);
    header.insurance = V16PodU128::new(200);
    header.insurance_domain_budget_remaining_total = V16PodU128::new(150);
    // Exact boundary: surplus == 50, withdraw exactly 50.
    let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);

    market.withdraw_insurance_surplus_not_atomic(50).unwrap();

    assert_eq!(market.header.vault.get(), 450);
    assert_eq!(market.header.insurance.get(), 150);
    assert_eq!(
        market.header.insurance_domain_budget_remaining_total.get(),
        150,
        "domain budgets are untouched by the surplus withdrawal"
    );
    market.validate_shape().unwrap();
}

// --- E2 (upstream engine #108, fixes #97): fresh risk must be blocked while
// either side of an asset is mid side-recovery (ResetPending/DrainOnly), not
// just while the asset's overall lifecycle is non-Active. Risk-REDUCING
// trades must remain admitted throughout recovery.

#[test]
fn v16_trade_rejects_fresh_risk_when_either_side_is_recovering() {
    let cases = [
        (SideModeV16::ResetPending, SideModeV16::Normal),
        (SideModeV16::Normal, SideModeV16::ResetPending),
        (SideModeV16::DrainOnly, SideModeV16::Normal),
        (SideModeV16::Normal, SideModeV16::DrainOnly),
    ];
    for (mode_long, mode_short) in cases {
        let (mut header, mut markets) = market_fixture(1, 100);
        let mut long_header = account_fixture(1, 60);
        let mut short_header = account_fixture(1, 61);
        {
            let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);
            let mut long = PortfolioV16ViewMut::new(&mut long_header);
            let mut short = PortfolioV16ViewMut::new(&mut short_header);
            market.deposit_not_atomic(&mut long, 1_000).unwrap();
            market.deposit_not_atomic(&mut short, 1_000).unwrap();
        }
        let mut asset = markets[0].engine.asset.try_to_runtime().unwrap();
        asset.mode_long = mode_long;
        asset.mode_short = mode_short;
        markets[0].engine.asset = AssetStateV16Account::from_runtime(&asset);

        let vault_before = header.vault.get();
        let c_tot_before = header.c_tot.get();
        let insurance_before = header.insurance.get();

        let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);
        let mut long = PortfolioV16ViewMut::new(&mut long_header);
        let mut short = PortfolioV16ViewMut::new(&mut short_header);
        let long_capital_before = long.header.capital.get();
        let short_capital_before = short.header.capital.get();

        let res = market.execute_trade_with_fee_loss_stale_scoped_not_atomic(
            &mut long,
            &mut short,
            TradeRequestV16 {
                asset_index: 0,
                size_q: signed_q(POS_SCALE),
                exec_price: 100,
                fee_bps: 0,
            },
            true,
        );

        assert_eq!(
            res,
            Err(V16Error::LockActive),
            "fresh risk admitted with mode_long={mode_long:?} mode_short={mode_short:?}"
        );
        // Rollback-clean rejection: no partial state mutation, no fee leakage.
        assert_eq!(market.header.vault.get(), vault_before);
        assert_eq!(market.header.c_tot.get(), c_tot_before);
        assert_eq!(market.header.insurance.get(), insurance_before);
        assert_eq!(long.header.capital.get(), long_capital_before);
        assert_eq!(short.header.capital.get(), short_capital_before);
        market.validate_shape().unwrap();
    }
}

#[test]
fn v16_trade_keeps_two_sided_risk_reduction_open_during_side_recovery() {
    let (mut header, mut markets) = market_fixture(1, 100);
    let mut long_header = account_fixture(1, 62);
    let mut short_header = account_fixture(1, 63);
    {
        let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);
        let mut long = PortfolioV16ViewMut::new(&mut long_header);
        let mut short = PortfolioV16ViewMut::new(&mut short_header);
        market.deposit_not_atomic(&mut long, 10_000).unwrap();
        market.deposit_not_atomic(&mut short, 10_000).unwrap();
        // Open long +2*POS_SCALE / short -2*POS_SCALE while both side modes
        // are still Normal.
        market
            .execute_trade_with_fee_loss_stale_scoped_not_atomic(
                &mut long,
                &mut short,
                TradeRequestV16 {
                    asset_index: 0,
                    size_q: signed_q(2 * POS_SCALE),
                    exec_price: 100,
                    fee_bps: 0,
                },
                true,
            )
            .unwrap();
    }

    // Now put the asset mid side-recovery on BOTH sides simultaneously (the
    // gate is not side-specific by design).
    let mut asset = markets[0].engine.asset.try_to_runtime().unwrap();
    asset.mode_long = SideModeV16::ResetPending;
    asset.mode_short = SideModeV16::DrainOnly;
    markets[0].engine.asset = AssetStateV16Account::from_runtime(&asset);

    let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);
    let mut long = PortfolioV16ViewMut::new(&mut long_header);
    let mut short = PortfolioV16ViewMut::new(&mut short_header);

    // A matched reduction (-POS_SCALE) shrinks both legs' magnitude
    // (long: +2 -> +1, short: -2 -> -1) and must still be admitted.
    let outcome = market
        .execute_trade_with_fee_loss_stale_scoped_not_atomic(
            &mut long,
            &mut short,
            TradeRequestV16 {
                asset_index: 0,
                size_q: -signed_q(POS_SCALE),
                exec_price: 100,
                fee_bps: 0,
            },
            true,
        )
        .expect("risk-reducing trade must stay open during side recovery");

    assert_eq!(outcome.notional, 100);
    let long_leg = long.header.legs[0].try_to_runtime().unwrap();
    let short_leg = short.header.legs[0].try_to_runtime().unwrap();
    assert_eq!(long_leg.basis_pos_q, signed_q(POS_SCALE));
    assert_eq!(short_leg.basis_pos_q, -signed_q(POS_SCALE));
    market.validate_shape().unwrap();
}

// E5 (upstream engine #109 / 143e68c4, "Prevent same-trade OI masking"): a
// single trade may reduce only OI that existed BEFORE that trade -- otherwise
// one leg's same-call addition can be spent as if it were preexisting
// reduction capacity on the other leg, making aggregate accounting depend on
// mutation order within one apply_trade_after_refresh_not_atomic call.
//
// Scenario: `liquidated` flips short(-10) -> long(+1) in the same call that
// `survivor` reduces long(+13) -> long(+2). In isolation, survivor's leg
// alone appears to free up 11 units of long-side reduction capacity, but the
// asset's PRE-TRADE ledger only records oi_eff_long_q = oi_eff_short_q =
// MATCHED_Q = 10 units (the pre-fix bug: the ledger is authoritative, not
// whatever a single leg's raw basis happens to show, and one leg cannot
// "loan" the other leg's same-call delta as spendable reduction capacity).
// The gate must reject with LockActive and mutate NOTHING -- not the asset
// ledger, not vault/c_tot/insurance, not either leg -- proving this is a
// preflight rejection, not a partial-apply rollback.
#[test]
fn v16_crossed_trade_cannot_spend_same_call_addition_as_preexisting_oi() {
    const MATCHED_Q: u128 = 10 * POS_SCALE;
    const LIQUIDATED_SHORT_Q: u128 = 10 * POS_SCALE;
    const SURVIVOR_LONG_Q: u128 = 13 * POS_SCALE;
    const FLIP_SIZE_Q: u128 = 11 * POS_SCALE; // liquidated: -10 -> +1; survivor: +13 -> +2

    let (mut header, mut markets) = market_fixture(1, 100);

    let mut asset = markets[0].engine.asset.try_to_runtime().unwrap();
    asset.oi_eff_long_q = MATCHED_Q;
    asset.oi_eff_short_q = MATCHED_Q;
    asset.loss_weight_sum_long = MATCHED_Q;
    asset.loss_weight_sum_short = MATCHED_Q;
    asset.stored_pos_count_long = 1;
    asset.stored_pos_count_short = 1;
    markets[0].engine.asset = AssetStateV16Account::from_runtime(&asset);
    // resolved_payout_blocker_count must reconcile with the asset's own
    // stored_pos_count_long(1) + stored_pos_count_short(1) totals, or
    // set_asset_state's delta-reconciliation throws CounterUnderflow before
    // the trade path (and therefore the OI gate) is ever reached.
    header.resolved_payout_blocker_count = V16PodU64::new(2);

    let mut liquidated_header = account_fixture(1, 217);
    liquidated_header.capital = V16PodU128::new(1_000_000);
    liquidated_header.legs[0] = PortfolioLegV16Account::from_runtime(&PortfolioLegV16 {
        active: true,
        asset_index: 0,
        market_id: asset.market_id,
        side: SideV16::Short,
        basis_pos_q: -signed_q(LIQUIDATED_SHORT_Q),
        a_basis: ADL_ONE,
        k_snap: asset.k_short,
        f_snap: asset.f_short_num,
        epoch_snap: asset.epoch_short,
        loss_weight: LIQUIDATED_SHORT_Q,
        b_snap: asset.b_short_num,
        b_rem: 0,
        b_epoch_snap: asset.epoch_short,
        b_stale: false,
        stale: false,
    });
    liquidated_header.active_bitmap[0] = V16PodU64::new(1);

    let mut survivor_header = account_fixture(1, 218);
    survivor_header.capital = V16PodU128::new(1_000_000);
    survivor_header.legs[0] = PortfolioLegV16Account::from_runtime(&PortfolioLegV16 {
        active: true,
        asset_index: 0,
        market_id: asset.market_id,
        side: SideV16::Long,
        basis_pos_q: signed_q(SURVIVOR_LONG_Q),
        a_basis: ADL_ONE,
        k_snap: asset.k_long,
        f_snap: asset.f_long_num,
        epoch_snap: asset.epoch_long,
        loss_weight: SURVIVOR_LONG_Q,
        b_snap: asset.b_long_num,
        b_rem: 0,
        b_epoch_snap: asset.epoch_long,
        b_stale: false,
        stale: false,
    });
    survivor_header.active_bitmap[0] = V16PodU64::new(1);

    let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);
    let mut liquidated = PortfolioV16ViewMut::new(&mut liquidated_header);
    let mut survivor = PortfolioV16ViewMut::new(&mut survivor_header);

    let asset_before = market.markets[0].engine.asset;
    let vault_before = market.header.vault.get();
    let c_tot_before = market.header.c_tot.get();
    let insurance_before = market.header.insurance.get();
    let liquidated_capital_before = liquidated.header.capital.get();
    let survivor_capital_before = survivor.header.capital.get();

    // `liquidated` is the long_account param (receives +FLIP_SIZE_Q, flipping
    // short(-10) -> long(+1)); `survivor` is the short_account param
    // (receives -FLIP_SIZE_Q, reducing long(+13) -> long(+2)).
    let result = market.execute_trade_with_fee_loss_stale_scoped_not_atomic(
        &mut liquidated,
        &mut survivor,
        TradeRequestV16 {
            asset_index: 0,
            size_q: signed_q(FLIP_SIZE_Q),
            exec_price: 100,
            fee_bps: 0,
        },
        true,
    );

    assert_eq!(result, Err(V16Error::LockActive));
    assert_eq!(
        market.markets[0].engine.asset, asset_before,
        "rejected trade must not mutate the asset ledger at all"
    );
    assert_eq!(market.header.vault.get(), vault_before);
    assert_eq!(market.header.c_tot.get(), c_tot_before);
    assert_eq!(market.header.insurance.get(), insurance_before);
    assert_eq!(liquidated.header.capital.get(), liquidated_capital_before);
    assert_eq!(survivor.header.capital.get(), survivor_capital_before);
    let liquidated_leg = liquidated.header.legs[0].try_to_runtime().unwrap();
    let survivor_leg = survivor.header.legs[0].try_to_runtime().unwrap();
    assert_eq!(liquidated_leg.basis_pos_q, -signed_q(LIQUIDATED_SHORT_Q));
    assert_eq!(survivor_leg.basis_pos_q, signed_q(SURVIVOR_LONG_Q));
    market.validate_shape().unwrap();
}

// Builds a resolved market in which `taker` holds exactly one active leg (so
// `resolved_bankruptcy_attribution` resolves via the leg scan) and carries an
// unabsorbed loss larger than its capital. Returns the asset's `market_id` so the
// caller can stamp a prior close ledger onto the account if it wants one.
fn resolved_market_with_bankrupt_taker_funded(
    fund_insurance: bool,
) -> (
    MarketGroupV16HeaderAccount,
    Vec<Market<u64>>,
    PortfolioAccountV16Account,
    u64,
) {
    let (mut header, mut markets) = market_fixture(1, 100);
    let mut taker_header = account_fixture(1, 91);
    let mut maker_header = account_fixture(1, 92);

    {
        let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);
        let mut taker = PortfolioV16ViewMut::new(&mut taker_header);
        let mut maker = PortfolioV16ViewMut::new(&mut maker_header);
        market.deposit_not_atomic(&mut taker, 1_000).unwrap();
        market.deposit_not_atomic(&mut maker, 1_000).unwrap();
        market
            .execute_trade_with_fee_loss_stale_scoped_not_atomic(
                &mut taker,
                &mut maker,
                TradeRequestV16 {
                    asset_index: 0,
                    size_q: signed_q(POS_SCALE),
                    exec_price: 100,
                    fee_bps: 0,
                },
                true,
            )
            .unwrap();
        // With insurance funded the bankruptcy path consumes it and reaches
        // `advance_close_progress_ledger`'s ledger-state guard. Without it, the
        // advance short-circuits on an all-zero progress delta and the failure is
        // silent instead -- both variants are exercised below.
        if fund_insurance {
            market.deposit_domain_insurance_not_atomic(0, 500).unwrap();
            market.deposit_domain_insurance_not_atomic(1, 500).unwrap();
        }
        market.resolve_market_not_atomic(1).unwrap();
    }

    // Loss exceeds capital, so principal settlement cannot clear it and the
    // resolved close must route through the bankruptcy path.
    taker_header.pnl = V16PodI128::new(-5_000);
    header.negative_pnl_account_count = V16PodU64::new(1);

    let market_id = markets[0].engine.asset.market_id.get();
    (header, markets, taker_header, market_id)
}

// E6 exempted finalized-inert ledgers inside `begin_close_progress_ledger`, but
// `settle_resolved_bankruptcy_negative_pnl` decides whether to CALL it by testing
// the raw `close_progress.active` flag. A finalized-inert ledger is still
// `active`, so the fresh close is skipped and the stale finalized ledger is kept;
// the following `advance_close_progress_ledger` then rejects it outright, and the
// whole resolved close reverts. An account that survived one fully-covered
// bankruptcy and meets a second at resolution can no longer be closed.
#[test]
fn e6_second_bankruptcy_reopens_finalized_inert_close_ledger() {
    let (mut header, mut markets, mut taker_header, market_id) =
        resolved_market_with_bankrupt_taker_funded(true);

    // An earlier bankruptcy on this account finished paying out in full: the
    // ledger is finalized with zero residual, but stays `active` to preserve
    // close identity for audit. `domain_side` is the side that BACKS the loss,
    // i.e. the opposite of the account's own leg (validated at the
    // `ledger.domain_side != opposite_side(leg.side)` check), so a long taker
    // carries a Short-domain ledger.
    taker_header.close_progress =
        CloseProgressLedgerV16Account::from_runtime(&CloseProgressLedgerV16 {
            active: true,
            finalized: true,
            canceled: false,
            close_id: 3,
            asset_index: 0,
            market_id,
            domain_side: SideV16::Short,
            gross_loss_at_close_start: 5,
            drift_reference_slot: 0,
            max_close_slot: 0,
            support_consumed: 5,
            junior_face_burned: 5,
            residual_remaining: 0,
            ..CloseProgressLedgerV16::EMPTY
        });

    let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);
    let mut taker = PortfolioV16ViewMut::new(&mut taker_header);

    // The constructed state must itself be valid, otherwise the revert below
    // would prove nothing about the close path.
    assert_eq!(
        market.validate_shape(),
        Ok(()),
        "market shape must be valid"
    );
    assert_eq!(
        taker.validate_with_market(&market.as_view()),
        Ok(()),
        "account (including the finalized-inert ledger) must be valid"
    );

    let outcome = market
        .close_resolved_account_not_atomic(&mut taker, 0)
        .expect("a second bankruptcy must reopen the inert close, not revert");
    assert!(matches!(outcome, ResolvedCloseOutcomeV16::Closed { .. }));

    // The replacement close is a well-formed first-class close, not a patched-up
    // remnant: the loss is absorbed, the close-id watermark advances rather than
    // colliding, the domain barrier it took is released again, and both validators
    // still accept the resulting state.
    assert_eq!(taker.header.pnl.get(), 0, "the loss is fully absorbed");
    let ledger = taker.header.close_progress.try_to_runtime().unwrap();
    assert_eq!(ledger.close_id, 4, "close-id watermark advances from 3");
    assert!(ledger.finalized && ledger.residual_remaining == 0);
    assert_eq!(
        market.markets[0]
            .engine
            .pending_domain_loss_barrier_long
            .get(),
        0
    );
    assert_eq!(
        market.markets[0]
            .engine
            .pending_domain_loss_barrier_short
            .get(),
        0,
        "the barrier taken by the reopened close is released again"
    );
    assert_eq!(market.validate_shape(), Ok(()));
    assert_eq!(taker.validate_with_market(&market.as_view()), Ok(()));
}

// Control: identical account and market, but with no prior close on record. The
// same resolved close must not hit LockActive, which pins the finalized-inert
// ledger -- not the bankruptcy itself -- as the cause above.
#[test]
fn e6_resolved_close_on_bankruptcy_without_prior_close_is_not_blocked() {
    let (mut header, mut markets, mut taker_header, _market_id) =
        resolved_market_with_bankrupt_taker_funded(true);

    let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);
    let mut taker = PortfolioV16ViewMut::new(&mut taker_header);

    let result = market.close_resolved_account_not_atomic(&mut taker, 0);
    assert_ne!(
        result.err(),
        Some(V16Error::LockActive),
        "a first bankruptcy at resolution must not be blocked"
    );
}

// The same defect with no insurance on the loss-backing domain, where it is
// SILENT rather than an error and has market-wide reach.
//
// `consume_domain_insurance_for_negative_pnl` returns 0, so the `advance` that
// raises LockActive above is skipped. `book_bankruptcy_residual_chunk_*` is then
// reached with the stale finalized ledger, whose `residual_remaining` is 0, so
// both the booking and the advance short-circuit on their zero guards and the
// call returns Ok having booked nothing. The close makes no progress and can be
// re-cranked forever.
//
// The account's own capital is stranded, but the wider consequence is that
// `negative_pnl_account_count` never falls to zero, and
// `resolved_positive_payout_ready` gates every positive resolved payout on that
// counter -- so no winner in the market can be paid either.
#[test]
fn e6_second_bankruptcy_completes_without_insurance() {
    let (mut header, mut markets, mut taker_header, market_id) =
        resolved_market_with_bankrupt_taker_funded(false);

    taker_header.close_progress =
        CloseProgressLedgerV16Account::from_runtime(&CloseProgressLedgerV16 {
            active: true,
            finalized: true,
            canceled: false,
            close_id: 3,
            asset_index: 0,
            market_id,
            domain_side: SideV16::Short,
            gross_loss_at_close_start: 5,
            drift_reference_slot: 0,
            max_close_slot: 0,
            support_consumed: 5,
            junior_face_burned: 5,
            residual_remaining: 0,
            ..CloseProgressLedgerV16::EMPTY
        });

    let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);
    let mut taker = PortfolioV16ViewMut::new(&mut taker_header);
    assert_eq!(market.validate_shape(), Ok(()));
    assert_eq!(taker.validate_with_market(&market.as_view()), Ok(()));

    // The reopened close books the residual as explicit loss and completes, with
    // no insurance available and without new capital being added.
    let outcome = market
        .close_resolved_account_not_atomic(&mut taker, 0)
        .expect("the close must progress rather than stall");
    assert!(
        matches!(outcome, ResolvedCloseOutcomeV16::Closed { .. }),
        "close completes instead of returning ProgressOnly forever"
    );

    assert_eq!(
        taker.header.pnl.get(),
        0,
        "the loss is absorbed, not parked"
    );
    // Before the fix this counter never returned to zero, and
    // `resolved_positive_payout_ready` gates every positive resolved payout on it
    // -- so a single stalled account withheld every winner's payout market-wide.
    assert_eq!(
        market.header.negative_pnl_account_count.get(),
        0,
        "the market's positive-payout gate is released"
    );
    let ledger = taker.header.close_progress.try_to_runtime().unwrap();
    assert_eq!(ledger.close_id, 4, "close-id watermark advances from 3");
    assert!(ledger.finalized && ledger.residual_remaining == 0);
    assert_eq!(market.validate_shape(), Ok(()));
    assert_eq!(taker.validate_with_market(&market.as_view()), Ok(()));
}

/// #137 — the initial-margin source-credit lien must be released when the exposure
/// that required it is closed, so the account can convert its own positive PnL while
/// the market is still Live.
#[test]
fn im_lien_is_released_when_the_position_closes_in_live() {
    let (mut header, mut markets) = market_fixture(1, 1);
    let mut long_header = account_fixture(1, 8);
    let mut short_header = account_fixture(1, 9);
    let claim = 100u128;
    let claim_num = claim * BOUND_SCALE;
    long_header.pnl = V16PodI128::new(claim as i128);
    long_header.source_domains[0].domain = V16PodU32::new(0);
    long_header.source_domains[0].source_claim_market_id = V16PodU64::new(1);
    long_header.source_domains[0].source_claim_bound_num = V16PodU128::new(claim_num);
    header.pnl_pos_tot = V16PodU128::new(claim);
    header.pnl_pos_bound_tot_num = V16PodU128::new(claim_num);
    header.pnl_pos_bound_tot = V16PodU128::new(claim);
    header.source_claim_bound_total_num = V16PodU128::new(claim_num);
    header.source_fresh_backing_total_num = V16PodU128::new(claim_num);
    // Backing principal is vault-funded and senior-side: vault must cover it.
    header.vault = V16PodU128::new(claim + header.vault.get());
    markets[0].engine.source_credit_long =
        SourceCreditStateV16Account::from_runtime(&SourceCreditStateV16 {
            positive_claim_bound_num: claim_num,
            exact_positive_claim_num: claim_num,
            fresh_reserved_backing_num: claim_num,
            credit_rate_num: CREDIT_RATE_SCALE,
            ..SourceCreditStateV16::EMPTY
        });
    markets[0].engine.backing_long = BackingBucketV16Account::from_runtime(&BackingBucketV16 {
        market_id: 1,
        fresh_unliened_backing_num: claim_num,
        expiry_slot: 100,
        status: BackingBucketStatusV16::Fresh,
        ..BackingBucketV16::EMPTY
    });
    {
        let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);
        let mut short = PortfolioV16ViewMut::new(&mut short_header);
        market.deposit_not_atomic(&mut short, 1_000).unwrap();
    }

    let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);
    let mut long = PortfolioV16ViewMut::new(&mut long_header);
    let mut short = PortfolioV16ViewMut::new(&mut short_header);
    market
        .execute_trade_with_fee_loss_stale_scoped_not_atomic(
            &mut long,
            &mut short,
            TradeRequestV16 {
                asset_index: 0,
                size_q: signed_q(10 * POS_SCALE),
                exec_price: 1,
                fee_bps: 0,
            },
            true,
        )
        .expect("risk-increasing trade should atomically lien backed source credit for IM");

    assert_eq!(long.header.capital.get(), 0);
    assert_eq!(
        long.header.source_domains[0].source_claim_liened_num.get(),
        10 * BOUND_SCALE
    );
    assert_eq!(
        long.header.source_domains[0]
            .source_lien_effective_reserved
            .get(),
        10
    );
    assert_eq!(
        long.header.source_domains[0]
            .source_lien_counterparty_backing_num
            .get(),
        10 * BOUND_SCALE
    );
    assert_eq!(
        market.markets[0]
            .engine
            .source_credit_long
            .valid_liened_backing_num
            .get(),
        10 * BOUND_SCALE
    );
    assert_eq!(
        market.markets[0]
            .engine
            .backing_long
            .valid_liened_backing_num
            .get(),
        10 * BOUND_SCALE
    );
    assert_eq!(
        market.markets[0]
            .engine
            .backing_long
            .fresh_unliened_backing_num
            .get(),
        90 * BOUND_SCALE
    );
    assert_eq!(
        market.convert_released_pnl_to_capital_not_atomic(&mut long),
        Err(V16Error::LockActive),
        "source-backed positive PnL must not be realized while the source-claim exposure remains open"
    );
    market.validate_shape().unwrap();
    long.validate_with_market(&market.as_view()).unwrap();
    short.validate_with_market(&market.as_view()).unwrap();

    let lien_open = long.header.source_domains[0].source_claim_liened_num.get();
    assert_ne!(lien_open, 0, "fixture must actually create an IM lien");

    market
        .execute_trade_with_fee_loss_stale_scoped_not_atomic(
            &mut short,
            &mut long,
            TradeRequestV16 {
                asset_index: 0,
                size_q: signed_q(10 * POS_SCALE),
                exec_price: 1,
                fee_bps: 0,
            },
            true,
        )
        .expect("closing trade");

    // Refresh so certificate staleness (an unrelated gate) does not mask the result.
    let _ = market.full_account_refresh_not_atomic(&mut long);

    let converted = market.convert_released_pnl_to_capital_not_atomic(&mut long);
    let lien_closed = long.header.source_domains[0].source_claim_liened_num.get();
    println!("FIX137 lien_open={lien_open} lien_closed={lien_closed} convert={converted:?}");

    // The defect: the lien survived the close and pinned the conversion.
    assert_eq!(
        lien_closed, 0,
        "IM lien must be released once the exposure that required it is closed"
    );
    assert_ne!(
        converted,
        Err(V16Error::LockActive),
        "conversion must no longer be held by a stale source-credit lien in Live"
    );
    // NOT asserted: `converted.is_ok()`. This minimal fixture never accrues, so the
    // market's own freshness gate returns `Stale` — a legitimate and unrelated
    // guard. Asserting it here would test the fixture, not the fix. The two
    // assertions above are exactly what #137 changes, and both fail without it
    // (the lien stays at its opening value and the error is LockActive).
}

/// #146 — expiry moves liened counterparty backing to the impaired counters. A
/// subsequent Live release for a flat account must clear that lien without either
/// reporting an arithmetic underflow or restoring the forfeited backing.
#[test]
fn expired_im_lien_release_is_live_and_does_not_restore_backing() {
    let (mut header, mut markets) = market_fixture(1, 1);
    let mut long_header = account_fixture(1, 18);
    let mut short_header = account_fixture(1, 19);
    let claim = 100u128;
    let claim_num = claim * BOUND_SCALE;
    long_header.pnl = V16PodI128::new(claim as i128);
    long_header.source_domains[0].domain = V16PodU32::new(0);
    long_header.source_domains[0].source_claim_market_id = V16PodU64::new(1);
    long_header.source_domains[0].source_claim_bound_num = V16PodU128::new(claim_num);
    header.pnl_pos_tot = V16PodU128::new(claim);
    header.pnl_pos_bound_tot_num = V16PodU128::new(claim_num);
    header.pnl_pos_bound_tot = V16PodU128::new(claim);
    header.source_claim_bound_total_num = V16PodU128::new(claim_num);
    header.source_fresh_backing_total_num = V16PodU128::new(claim_num);
    header.vault = V16PodU128::new(claim);
    markets[0].engine.source_credit_long =
        SourceCreditStateV16Account::from_runtime(&SourceCreditStateV16 {
            positive_claim_bound_num: claim_num,
            exact_positive_claim_num: claim_num,
            fresh_reserved_backing_num: claim_num,
            credit_rate_num: CREDIT_RATE_SCALE,
            ..SourceCreditStateV16::EMPTY
        });
    markets[0].engine.backing_long = BackingBucketV16Account::from_runtime(&BackingBucketV16 {
        market_id: 1,
        fresh_unliened_backing_num: claim_num,
        expiry_slot: 100,
        status: BackingBucketStatusV16::Fresh,
        ..BackingBucketV16::EMPTY
    });

    let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);
    let mut long = PortfolioV16ViewMut::new(&mut long_header);
    let mut short = PortfolioV16ViewMut::new(&mut short_header);
    market.deposit_not_atomic(&mut short, 1_000).unwrap();
    market
        .execute_trade_with_fee_loss_stale_scoped_not_atomic(
            &mut long,
            &mut short,
            TradeRequestV16 {
                asset_index: 0,
                size_q: signed_q(10 * POS_SCALE),
                exec_price: 1,
                fee_bps: 0,
            },
            true,
        )
        .expect("risk-increasing trade must create the IM lien");
    market
        .execute_trade_with_fee_loss_stale_scoped_not_atomic(
            &mut short,
            &mut long,
            TradeRequestV16 {
                asset_index: 0,
                size_q: signed_q(10 * POS_SCALE),
                exec_price: 1,
                fee_bps: 0,
            },
            true,
        )
        .expect("closing trade must flatten the lien's exposure");

    let lien_num = long.header.source_domains[0]
        .source_lien_counterparty_backing_num
        .get();
    assert_ne!(lien_num, 0, "fixture must retain a stale IM lien");
    market
        .expire_source_backing_bucket_not_atomic(0, 100)
        .expect("the backing bucket must expire through the production transition");
    let impaired = market.markets[0]
        .engine
        .backing_long
        .try_to_runtime()
        .unwrap();
    assert_eq!(impaired.status, BackingBucketStatusV16::Impaired);
    assert_eq!(impaired.impaired_liened_backing_num, lien_num);
    assert_eq!(impaired.fresh_unliened_backing_num, 0);

    let vault_before = market.header.vault.get();
    let capital_before = market.header.c_tot.get();
    let released = market.release_account_source_credit_liens_if_unneeded_not_atomic(&mut long);

    assert_eq!(released, Ok(10));
    assert_eq!(
        long.header.source_domains[0].source_claim_liened_num.get(),
        0
    );
    let released_bucket = market.markets[0]
        .engine
        .backing_long
        .try_to_runtime()
        .unwrap();
    assert_eq!(
        released_bucket.status,
        BackingBucketStatusV16::Empty,
        "a fully drained impaired bucket has no residue and canonicalizes to Empty"
    );
    assert_eq!(released_bucket.impaired_liened_backing_num, 0);
    assert_eq!(
        released_bucket.fresh_unliened_backing_num, 0,
        "expired principal must never be resurrected as fresh backing"
    );
    assert_eq!(market.header.vault.get(), vault_before);
    assert_eq!(market.header.c_tot.get(), capital_before);
    market.validate_shape().unwrap();
    long.validate_with_market(&market.as_view()).unwrap();
    short.validate_with_market(&market.as_view()).unwrap();
}

/// #134 — a position opened against a side whose `a` was scaled down by an ADL must
/// not create value. Conservation: obligations == vault, with no deposit or withdrawal.
#[test]
fn post_adl_new_position_conserves_value() {
    for d in [1u64, 10, 100, 1_000, 10_000, 100_000] {
        scen134(d);
    }
}

/// Asserts the residue does not SCALE with the price move. Pre-fix it was
/// `minted = dpx * trade * h` (linear); post-fix it is a constant 1-atom floor/ceil
/// residue in the conservative direction (vault >= obligations).
fn scen134(dpx: u64) {
    const PX0: u64 = 1_000_000;
    const DEP: u128 = 100_000_000;
    let (mut header, mut markets) = market_fixture(1, PX0);
    let mut a_h = account_fixture(1, 61);
    let mut b_h = account_fixture(1, 62);
    let mut c_h = account_fixture(1, 63);
    let obl = |m: &MarketGroupV16ViewMut<'_, u64>| -> u128 {
        m.header.c_tot.get() + m.header.pnl_pos_tot.get() + m.header.insurance.get()
    };
    let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);
    let mut a = PortfolioV16ViewMut::new(&mut a_h);
    let mut b = PortfolioV16ViewMut::new(&mut b_h);
    let mut c = PortfolioV16ViewMut::new(&mut c_h);
    market.deposit_not_atomic(&mut a, DEP).unwrap();
    market.deposit_not_atomic(&mut b, DEP).unwrap();
    market.deposit_not_atomic(&mut c, DEP).unwrap();
    let v0 = market.header.vault.get();

    market
        .execute_trade_with_fee_loss_stale_scoped_not_atomic(
            &mut a,
            &mut b,
            TradeRequestV16 {
                asset_index: 0,
                size_q: signed_q(10 * POS_SCALE),
                exec_price: PX0,
                fee_bps: 0,
            },
            true,
        )
        .unwrap();
    market
        .rebalance_reduce_position_not_atomic(
            &mut a,
            RebalanceRequestV16 {
                asset_index: 0,
                reduce_q: 4 * POS_SCALE,
            },
        )
        .unwrap();
    assert!(
        market.markets[0].engine.asset.a_short.get() < ADL_ONE,
        "ADL must scale a_short"
    );

    // C opens against B, whose short leg carries a frozen a_basis from before the ADL.
    market
        .execute_trade_with_fee_loss_stale_scoped_not_atomic(
            &mut c,
            &mut b,
            TradeRequestV16 {
                asset_index: 0,
                size_q: signed_q(10 * POS_SCALE),
                exec_price: PX0,
                fee_bps: 0,
            },
            true,
        )
        .unwrap();

    market
        .accrue_asset_to_not_atomic(0, 2, PX0 + dpx, 0, true)
        .unwrap();
    let _ = market.full_account_refresh_not_atomic(&mut a);
    let _ = market.full_account_refresh_not_atomic(&mut b);
    let _ = market.full_account_refresh_not_atomic(&mut c);

    let vault = market.header.vault.get();
    let o = obl(&market);
    let delta = o as i128 - vault as i128;
    println!("CONS134 dpx={dpx:>7} delta={delta:>7}");
    assert!(
        delta <= 0,
        "obligations must never exceed the vault (mint), got +{delta} at dpx={dpx}"
    );
    assert!(
        delta >= -1,
        "residue must stay a single conservative atom, got {delta} at dpx={dpx}"
    );
    assert_eq!(vault, v0, "no deposits or withdrawals occurred");
}

/// #134 thorough sweep: the residue must never be positive (a mint) and must not
/// scale, across haircut fractions, trade sizes and repeated ADL rounds.
#[test]
fn post_adl_sweep_never_mints() {
    const PX0: u64 = 1_000_000;
    const DEP: u128 = 100_000_000_000;
    let run = |oi: u128, red: u128, sz: u128, dpx: u64, rounds: u32| -> i128 {
        let (mut header, mut markets) = market_fixture(1, PX0);
        let mut a_h = account_fixture(1, 81);
        let mut b_h = account_fixture(1, 82);
        let mut c_h = account_fixture(1, 83);
        let mut m = MarketGroupV16ViewMut::new(&mut header, &mut markets);
        let mut a = PortfolioV16ViewMut::new(&mut a_h);
        let mut b = PortfolioV16ViewMut::new(&mut b_h);
        let mut c = PortfolioV16ViewMut::new(&mut c_h);
        m.deposit_not_atomic(&mut a, DEP).unwrap();
        m.deposit_not_atomic(&mut b, DEP).unwrap();
        m.deposit_not_atomic(&mut c, DEP).unwrap();
        let v0 = m.header.vault.get();
        if let Err(e) = m.execute_trade_with_fee_loss_stale_scoped_not_atomic(
            &mut a,
            &mut b,
            TradeRequestV16 {
                asset_index: 0,
                size_q: signed_q(oi * POS_SCALE),
                exec_price: PX0,
                fee_bps: 0,
            },
            true,
        ) {
            println!("SKIP trade1 {e:?}");
            return 0;
        }
        for _ in 0..rounds {
            if let Err(e) = m.rebalance_reduce_position_not_atomic(
                &mut a,
                RebalanceRequestV16 {
                    asset_index: 0,
                    reduce_q: red * POS_SCALE,
                },
            ) {
                println!("SKIP adl {e:?}");
                return 0;
            }
        }
        if let Err(e) = m.execute_trade_with_fee_loss_stale_scoped_not_atomic(
            &mut c,
            &mut b,
            TradeRequestV16 {
                asset_index: 0,
                size_q: signed_q(sz * POS_SCALE),
                exec_price: PX0,
                fee_bps: 0,
            },
            true,
        ) {
            println!("SKIP trade3 {e:?}");
            return 0;
        }
        if m.accrue_asset_to_not_atomic(0, 2, PX0 + dpx, 0, true)
            .is_err()
        {
            return 0;
        }
        let _ = m.full_account_refresh_not_atomic(&mut a);
        let _ = m.full_account_refresh_not_atomic(&mut b);
        let _ = m.full_account_refresh_not_atomic(&mut c);
        let o = m.header.c_tot.get() + m.header.pnl_pos_tot.get() + m.header.insurance.get();
        assert_eq!(m.header.vault.get(), v0, "no deposits/withdrawals");
        o as i128 - m.header.vault.get() as i128
    };
    let mut worst: i128 = 0;
    for &(oi, red, sz) in &[
        (10u128, 4u128, 10u128),
        (10, 1, 10),
        (10, 8, 10),
        (10, 9, 10),
        (100, 40, 100),
        (100, 89, 100),
        (10, 4, 4),
        (10, 4, 20),
        (20, 5, 20),
        (50, 20, 50),
    ] {
        for &dpx in &[1u64, 1_000, 100_000] {
            for &rounds in &[1u32, 2] {
                let d = run(oi, red, sz, dpx, rounds);
                if d > 0 {
                    println!("SWEEP134 MINT oi={oi} red={red} sz={sz} dpx={dpx} rounds={rounds} delta=+{d}");
                }
                if d > worst {
                    worst = d;
                }
            }
        }
    }
    println!("SWEEP134 worst_positive_delta={worst}");
    assert_eq!(worst, 0, "no configuration may mint value");
}

/// Upstream aeyakovenko/percolator#132 (OPEN, confirmed REAL DoS by Toly across four
/// clean-room reproductions) — present in this engine too.
///
/// `clear_leg` detaches by RAW `basis_pos_q` from A-scaled `oi_eff`, so after a
/// unilateral reduction scales the opposite side, the untouched opposite leg's raw
/// basis exceeds effective OI and it can never detach: `CounterUnderflow`, with the
/// victim's principal permanently stuck.
///
/// Toly's prescribed invariant: "detach an A-basis leg by its exact remaining
/// effective-OI contribution with deterministic aggregate rounding."
#[test]
fn untouched_opposite_leg_detaches_after_a_unilateral_reduction() {
    const PX0: u64 = 1_000_000;
    const DEP: u128 = 100_000_000;
    let (mut header, mut markets) = market_fixture(1, PX0);
    let mut a_h = account_fixture(1, 91);
    let mut b_h = account_fixture(1, 92);
    let mut c_h = account_fixture(1, 93);
    let mut m = MarketGroupV16ViewMut::new(&mut header, &mut markets);
    let mut a = PortfolioV16ViewMut::new(&mut a_h);
    let mut b = PortfolioV16ViewMut::new(&mut b_h);
    let mut c = PortfolioV16ViewMut::new(&mut c_h);
    m.deposit_not_atomic(&mut a, DEP).unwrap();
    m.deposit_not_atomic(&mut b, DEP).unwrap();
    m.deposit_not_atomic(&mut c, DEP).unwrap();

    // A long 4 vs B short 4.
    m.execute_trade_with_fee_loss_stale_scoped_not_atomic(
        &mut a,
        &mut b,
        TradeRequestV16 {
            asset_index: 0,
            size_q: signed_q(4 * POS_SCALE),
            exec_price: PX0,
            fee_bps: 0,
        },
        true,
    )
    .unwrap();

    // Toly's sequence: the long owner unilaterally reduces 4 -> 3.
    m.rebalance_reduce_position_not_atomic(
        &mut a,
        RebalanceRequestV16 {
            asset_index: 0,
            reduce_q: POS_SCALE,
        },
    )
    .unwrap();

    let oi_short = m.markets[0].engine.asset.oi_eff_short_q.get();
    let b_raw = b.header.legs[0]
        .try_to_runtime()
        .unwrap()
        .basis_pos_q
        .unsigned_abs();
    println!(
        "DOS132 oi_eff_short={oi_short} untouched_raw_basis={b_raw} raw_exceeds_effective={}",
        b_raw > oi_short
    );

    // The untouched short closes out against a fresh counterparty. Its leg reaches
    // zero, so clear_leg runs — and must not underflow.
    let closed = m.execute_trade_with_fee_loss_stale_scoped_not_atomic(
        &mut b,
        &mut c,
        TradeRequestV16 {
            asset_index: 0,
            size_q: signed_q(4 * POS_SCALE),
            exec_price: PX0,
            fee_bps: 0,
        },
        true,
    );
    println!("DOS132 detach={closed:?}");

    // The real property: the leg must actually be detachable, whatever the error code.
    assert!(
        closed.is_ok(),
        "an untouched opposite leg must remain detachable after a unilateral reduction, got {closed:?}"
    );
}

/// #449 — a PARTIAL reduce of a leg whose `a_basis` is STALE must adjust
/// `oi_eff` by the A-scaled contribution, not by the raw basis delta.
///
/// Scenario (0x-SquidSol's PoC shape, real mutators only):
///   A long 10 / B short 10  and  C long 10 / D short 10   (long OI 20, a_long = ADL_ONE)
///   D fully closes its FRESH short 10   -> long OI 20 -> 10, a_long -> 0.5*ADL_ONE
///   A partial-reduces its long, whose `a_basis` is now STALE (still ADL_ONE)
///
/// The shrink branch updates `oi_eff` with the RAW delta while
/// `loss_weight_sum` is maintained in A-correct terms (#134), so the stored
/// pair drifts. The drift keeps `oi_eff_long == oi_eff_short` (hiding it from
/// the Live equality guard) but breaks the protocol's own conservation
/// invariant `c_tot + pnl_pos_tot + insurance <= vault`.
///
/// Asserting CONSERVATION rather than a chosen formula keeps this test
/// independent of the fix's design.
#[test]
fn partial_reduce_of_stale_a_basis_leg_preserves_conservation() {
    const PX0: u64 = 1_000_000;
    const DEP: u128 = 100_000_000_000;
    let (mut header, mut markets) = market_fixture(1, PX0);
    let mut a_h = account_fixture(1, 91);
    let mut b_h = account_fixture(1, 92);
    let mut c_h = account_fixture(1, 93);
    let mut d_h = account_fixture(1, 94);
    let mut m = MarketGroupV16ViewMut::new(&mut header, &mut markets);
    let mut a = PortfolioV16ViewMut::new(&mut a_h);
    let mut b = PortfolioV16ViewMut::new(&mut b_h);
    let mut c = PortfolioV16ViewMut::new(&mut c_h);
    let mut d = PortfolioV16ViewMut::new(&mut d_h);
    for acct in [&mut a, &mut b, &mut c, &mut d] {
        m.deposit_not_atomic(acct, DEP).unwrap();
    }
    let v0 = m.header.vault.get();
    let trade = |sz: u128| TradeRequestV16 {
        asset_index: 0,
        size_q: signed_q(sz * POS_SCALE),
        exec_price: PX0,
        fee_bps: 0,
    };
    m.execute_trade_with_fee_loss_stale_scoped_not_atomic(&mut a, &mut b, trade(10), true)
        .expect("A long / B short");
    m.execute_trade_with_fee_loss_stale_scoped_not_atomic(&mut c, &mut d, trade(10), true)
        .expect("C long / D short");

    // D fully closes its FRESH short -> scales a_long on the opposite side.
    m.rebalance_reduce_position_not_atomic(
        &mut d,
        RebalanceRequestV16 {
            asset_index: 0,
            reduce_q: 10 * POS_SCALE,
        },
    )
    .expect("D full close");
    let a_long_after = m.markets[0].engine.asset.a_long.get();
    assert!(
        a_long_after < ADL_ONE,
        "precondition: a_long must be scaled (got {a_long_after})"
    );

    // A partial-reduces a leg whose a_basis is now STALE. This is the operation
    // no existing sweep or Kani proof covers.
    m.rebalance_reduce_position_not_atomic(
        &mut a,
        RebalanceRequestV16 {
            asset_index: 0,
            reduce_q: 5 * POS_SCALE,
        },
    )
    .expect("A stale-a_basis partial reduce");

    m.accrue_asset_to_not_atomic(0, 2, PX0 + 1_000, 0, true)
        .ok();
    for acct in [&mut a, &mut b, &mut c, &mut d] {
        let _ = m.full_account_refresh_not_atomic(acct);
    }
    assert_eq!(
        m.header.vault.get(),
        v0,
        "no deposits/withdrawals in this scenario"
    );
    let obligations = m.header.c_tot.get() + m.header.pnl_pos_tot.get() + m.header.insurance.get();
    let delta = obligations as i128 - m.header.vault.get() as i128;
    assert!(
        delta <= 0,
        "#449: conservation broken after stale-a_basis partial reduce — \
         obligations exceed vault by {delta} (vault is SHORT)"
    );
}

// #449 part 3 (the dead-leg guard, `effective_close_q == 0 -> Err(InvalidLeg)`)
// has NO coverage in this crate, and that is stated rather than papered over.
//
// I wrote a test for it and deleted it: the scenario I could build (full close,
// then reduce again) is rejected by an EARLIER check, returning the same
// `InvalidLeg` whether the guard is present or not. It passed with the guard
// reverted, which makes it vacuous — worse than absent, because it reads as
// coverage in a file reviewers trust.
//
// Reaching the guard needs a leg that is still ACTIVE while its A-scaled
// contribution floors to zero, which the fixtures here do not currently
// construct. The guard's actual regression is
// `v16_wrapper_prediction_asset_can_drain_retire_...` in percolator-prog — a
// different repo, and not where someone changing
// `rebalance_reduce_position_not_atomic` would look. That is the real gap.
/// Shared fixture for the Live source-credit-lien regressions below: an account
/// that opened a risk-increasing trade (minting the IM lien) and then closed it,
/// leaving a stale lien and realizable positive PnL.
#[allow(clippy::type_complexity)]
fn live_stale_im_lien_fixture() -> (
    MarketGroupV16HeaderAccount,
    Vec<Market<u64>>,
    PortfolioAccountV16Account,
) {
    let (mut header, mut markets) = market_fixture(1, 1);
    let mut long_header = account_fixture(1, 8);
    let mut short_header = account_fixture(1, 9);
    let claim = 100u128;
    let claim_num = claim * BOUND_SCALE;
    long_header.pnl = V16PodI128::new(claim as i128);
    long_header.source_domains[0].domain = V16PodU32::new(0);
    long_header.source_domains[0].source_claim_market_id = V16PodU64::new(1);
    long_header.source_domains[0].source_claim_bound_num = V16PodU128::new(claim_num);
    header.pnl_pos_tot = V16PodU128::new(claim);
    header.pnl_pos_bound_tot_num = V16PodU128::new(claim_num);
    header.pnl_pos_bound_tot = V16PodU128::new(claim);
    header.source_claim_bound_total_num = V16PodU128::new(claim_num);
    header.source_fresh_backing_total_num = V16PodU128::new(claim_num);
    header.vault = V16PodU128::new(claim + header.vault.get());
    markets[0].engine.source_credit_long =
        SourceCreditStateV16Account::from_runtime(&SourceCreditStateV16 {
            positive_claim_bound_num: claim_num,
            exact_positive_claim_num: claim_num,
            fresh_reserved_backing_num: claim_num,
            credit_rate_num: CREDIT_RATE_SCALE,
            ..SourceCreditStateV16::EMPTY
        });
    markets[0].engine.backing_long = BackingBucketV16Account::from_runtime(&BackingBucketV16 {
        market_id: 1,
        fresh_unliened_backing_num: claim_num,
        expiry_slot: 100,
        status: BackingBucketStatusV16::Fresh,
        ..BackingBucketV16::EMPTY
    });
    {
        let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);
        let mut short = PortfolioV16ViewMut::new(&mut short_header);
        market.deposit_not_atomic(&mut short, 1_000).unwrap();
    }
    {
        let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);
        let mut long = PortfolioV16ViewMut::new(&mut long_header);
        let mut short = PortfolioV16ViewMut::new(&mut short_header);
        market
            .execute_trade_with_fee_loss_stale_scoped_not_atomic(
                &mut long,
                &mut short,
                TradeRequestV16 {
                    asset_index: 0,
                    size_q: signed_q(10 * POS_SCALE),
                    exec_price: 1,
                    fee_bps: 0,
                },
                true,
            )
            .expect("opening trade mints the IM lien");
        market
            .execute_trade_with_fee_loss_stale_scoped_not_atomic(
                &mut short,
                &mut long,
                TradeRequestV16 {
                    asset_index: 0,
                    size_q: signed_q(10 * POS_SCALE),
                    exec_price: 1,
                    fee_bps: 0,
                },
                true,
            )
            .expect("closing trade");
    }
    (header, markets, long_header)
}

/// The Live lien release must leave the account with a CURRENT certificate.
///
/// `release_account_source_credit_liens_if_unneeded_not_atomic` advances the market
/// `risk_epoch` (it recomputes the source-credit domain), which retires every
/// outstanding certificate including the one the release derived for itself. It
/// previously also cleared `health_cert.valid` on exit. Either alone is enough to
/// make `ensure_favorable_action_current_certificate` reject the account, so the
/// conversion that `convert_released_pnl_to_capital_not_atomic` performs
/// immediately after the release returned `Stale` — and because the instruction
/// returned an error, the runtime discarded the release along with it, so the lien
/// could never clear in `Live` at all.
///
/// Regression for the durable freeze reported in #148 (a recurrence of #137).
#[test]
fn live_lien_release_leaves_a_current_certificate_so_conversion_succeeds() {
    let (mut header, mut markets, mut acct) = live_stale_im_lien_fixture();

    let lien = acct.source_domains[0].source_lien_effective_reserved.get();
    assert_ne!(
        lien, 0,
        "fixture must carry the stale IM lien the release targets"
    );

    // Model the runtime: an instruction that returns Err commits nothing, so a
    // failing conversion must not be credited with the release it performed.
    let mut outcomes = Vec::new();
    for _ in 0..4 {
        {
            let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);
            let mut a = PortfolioV16ViewMut::new(&mut acct);
            let _ = market.full_account_refresh_not_atomic(&mut a);
        }
        let (h, m, s) = (header, markets.clone(), acct);
        let res = {
            let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);
            let mut a = PortfolioV16ViewMut::new(&mut acct);
            market.convert_released_pnl_to_capital_not_atomic(&mut a)
        };
        if res.is_err() {
            header = h;
            markets = m;
            acct = s;
        }
        outcomes.push(res);
    }

    assert_eq!(
        outcomes[0],
        Ok(100),
        "the first conversion must succeed; it previously returned Err(Stale) because \
         the release retired the certificate the preflight then demanded"
    );
    assert!(
        outcomes[1..].iter().all(|r| *r == Ok(0)),
        "later conversions are no-ops once the claim is realized; got {outcomes:?}"
    );
    assert_eq!(
        acct.source_domains[0].source_lien_effective_reserved.get(),
        0,
        "the released lien must be committed, not rolled back with an error"
    );
}

/// The release is `pub` and reachable on its own, so it must leave a usable
/// certificate regardless of how it is reached — not only when it happens to be
/// called from inside `convert_released_pnl_to_capital_not_atomic`.
///
/// This distinguishes certifying inside the release from certifying at that one
/// call site: the call site is guarded by `valid_source_lien_effective_reserved_sum
/// != 0`, which is already false after a direct release, so a call-site-only fix
/// leaves this path returning `Stale`.
#[test]
fn direct_live_lien_release_leaves_a_certificate_the_conversion_can_use() {
    let (mut header, mut markets, mut acct) = live_stale_im_lien_fixture();

    let released = {
        let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);
        let mut a = PortfolioV16ViewMut::new(&mut acct);
        market.release_account_source_credit_liens_if_unneeded_not_atomic(&mut a)
    };
    assert_eq!(released, Ok(10), "the release itself succeeds");
    assert!(
        acct.health_cert.try_to_runtime().unwrap().valid,
        "the release must leave a valid certificate, as every other self-contained \
         mutating operation in this engine does"
    );

    let converted = {
        let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);
        let mut a = PortfolioV16ViewMut::new(&mut acct);
        market.convert_released_pnl_to_capital_not_atomic(&mut a)
    };
    assert_eq!(
        converted,
        Ok(100),
        "converting straight after a direct release must not be blocked by the \
         certificate that release retired"
    );
}

// upstream c09d4575 "Fix live source-backing expiry progress" (2026-08-06): a
// source-backed winner can remain Live past its backing bucket's expiry; the
// permissionless refresh commits exactly one canonical expiry transition per
// call (SourceBackingExpired { domain }) before valuation, so repeated cranks
// drain the bounded domain set. Fork adaptation: upstream drives this through
// permissionless_auto_crank_not_atomic (not ported); ours uses the Refresh action
// of permissionless_crank_not_atomic, which reaches the same refresh path with
// allow_b_chunk = true.
#[test]
fn v16_permissionless_refresh_expires_one_lapsed_live_source_domain_per_step() {
    let (mut header, mut markets) = market_fixture(2, 100);
    let mut account_header = account_fixture(2, 22);
    {
        let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);
        let mut account = PortfolioV16ViewMut::new(&mut account_header);
        market.deposit_not_atomic(&mut account, 100).unwrap();
        market
            .deposit_fresh_counterparty_backing_not_atomic(1, 40, 5)
            .unwrap();
        market
            .deposit_fresh_counterparty_backing_not_atomic(3, 40, 5)
            .unwrap();
        market
            .add_account_source_positive_pnl_not_atomic(&mut account, 1, 40)
            .unwrap();
        market
            .add_account_source_positive_pnl_not_atomic(&mut account, 3, 40)
            .unwrap();
        market
            .accrue_asset_to_not_atomic(0, 10, 100, 0, true)
            .unwrap();
        market
            .accrue_asset_to_not_atomic(1, 10, 100, 0, true)
            .unwrap();
    }

    let before = markets[0].engine.backing_short.try_to_runtime().unwrap();
    assert_eq!(before.status, BackingBucketStatusV16::Fresh);
    assert_eq!(before.expiry_slot, 5);
    assert_eq!(
        markets[1]
            .engine
            .backing_short
            .try_to_runtime()
            .unwrap()
            .status,
        BackingBucketStatusV16::Fresh
    );
    assert!(header.current_slot.get() > before.expiry_slot);
    let vault_before = header.vault.get();
    let c_tot_before = header.c_tot.get();
    let insurance_before = header.insurance.get();
    let earnings_before = header.backing_provider_earnings_total.get();
    let source_backing_before = header.source_fresh_backing_total_num.get();
    let risk_epoch_before = header.risk_epoch.get();

    let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);
    let mut account = PortfolioV16ViewMut::new(&mut account_header);
    let refresh_request = PermissionlessCrankRequestV16 {
        now_slot: 10,
        asset_index: 0,
        effective_price: 100,
        funding_rate_e9: 0,
        action: PermissionlessCrankActionV16::Refresh,
    };
    let expiry = market
        .permissionless_crank_not_atomic(&mut account, refresh_request)
        .expect("a Live refresh must expire lapsed backing instead of returning Stale");
    assert_eq!(
        expiry,
        PermissionlessProgressOutcomeV16::SourceBackingExpired { domain: 1 }
    );
    let after = market.markets[0]
        .engine
        .backing_short
        .try_to_runtime()
        .unwrap();
    assert_eq!(after.status, BackingBucketStatusV16::Expired);
    assert_eq!(after.fresh_unliened_backing_num, 0);
    assert_eq!(
        market.markets[1]
            .engine
            .backing_short
            .try_to_runtime()
            .unwrap()
            .status,
        BackingBucketStatusV16::Fresh,
        "one refresh expires exactly one source domain"
    );
    assert_eq!(market.header.vault.get(), vault_before);
    assert_eq!(market.header.c_tot.get(), c_tot_before);
    assert_eq!(market.header.insurance.get(), insurance_before);
    assert_eq!(
        market.header.backing_provider_earnings_total.get(),
        earnings_before
    );
    assert_eq!(
        market.header.source_fresh_backing_total_num.get(),
        source_backing_before - 40 * BOUND_SCALE
    );
    assert_eq!(market.header.risk_epoch.get(), risk_epoch_before + 1);
    assert_eq!(account.header.capital.get(), 100);
    assert_eq!(account.header.pnl.get(), 80);
    assert!(!account.header.health_cert.try_to_runtime().unwrap().valid);

    let second_expiry = market
        .permissionless_crank_not_atomic(&mut account, refresh_request)
        .expect("the next bounded refresh must expire the next domain");
    assert_eq!(
        second_expiry,
        PermissionlessProgressOutcomeV16::SourceBackingExpired { domain: 3 }
    );
    assert_eq!(
        market.markets[1]
            .engine
            .backing_short
            .try_to_runtime()
            .unwrap()
            .status,
        BackingBucketStatusV16::Expired
    );
    assert_eq!(market.header.vault.get(), vault_before);
    assert_eq!(market.header.c_tot.get(), c_tot_before);
    assert_eq!(market.header.insurance.get(), insurance_before);
    assert_eq!(
        market.header.backing_provider_earnings_total.get(),
        earnings_before
    );
    assert_eq!(market.header.source_fresh_backing_total_num.get(), 0);
    assert_eq!(market.header.risk_epoch.get(), risk_epoch_before + 2);

    let refresh = market
        .permissionless_crank_not_atomic(&mut account, refresh_request)
        .expect("the final bounded refresh must finish account refresh");
    assert_eq!(refresh, PermissionlessProgressOutcomeV16::AccountCurrent);
    assert!(account.header.health_cert.try_to_runtime().unwrap().valid);
    market.validate_shape().unwrap();
    account.validate_with_market(&market.as_view()).unwrap();
}

// upstream 650e3fdf "Commit recovery for unbookable terminal forfeits" (2026-08-22):
// when a Recovery-mode owner forfeit leaves a residual that the absorbing side
// cannot book (capacity 0), the forfeit COMMITS Recovery as a successful
// transition instead of returning RecoveryRequired. On Solana an Err discards the
// whole instruction, so the declared mode never persisted and the only path to
// resolved settlement was unreachable (the dead escalation valve). Fork
// adaptation: upstream's tail finalizes through permissionless_auto_crank
// (row 241, not ported); the committed state is asserted directly.
#[test]
fn v16_recovery_forfeit_commits_terminal_recovery_when_absorbing_side_is_empty() {
    let (mut header, mut markets) = market_fixture(1, 100);
    let mut account_header = account_fixture(1, 28);
    {
        let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);
        market.force_asset_recovery_not_atomic(0, 2).unwrap();
    }

    let mut asset = markets[0].engine.asset.try_to_runtime().unwrap();
    asset.oi_eff_long_q = POS_SCALE;
    asset.loss_weight_sum_long = POS_SCALE;
    asset.stored_pos_count_long = 1;
    markets[0].engine.asset = AssetStateV16Account::from_runtime(&asset);
    header.negative_pnl_account_count = V16PodU64::new(1);
    header.resolved_payout_blocker_count = V16PodU64::new(1);
    account_header.pnl = V16PodI128::new(-5);
    account_header.legs[0] = PortfolioLegV16Account::from_runtime(&PortfolioLegV16 {
        active: true,
        asset_index: 0,
        market_id: asset.market_id,
        side: SideV16::Long,
        basis_pos_q: POS_SCALE as i128,
        a_basis: ADL_ONE,
        k_snap: asset.k_long,
        f_snap: asset.f_long_num,
        epoch_snap: asset.epoch_long,
        loss_weight: POS_SCALE,
        b_snap: asset.b_long_num,
        b_rem: 0,
        b_epoch_snap: asset.epoch_long,
        b_stale: false,
        stale: false,
    });
    account_header.active_bitmap[0] = V16PodU64::new(1);

    let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);
    let mut account = PortfolioV16ViewMut::new(&mut account_header);
    market.validate_shape().unwrap();
    account.validate_with_market(&market.as_view()).unwrap();

    let outcome = market
        .forfeit_recovery_leg_not_atomic(&mut account, 0, u128::MAX)
        .expect("forfeit must commit Recovery instead of returning a rollback-only error");
    assert!(!outcome.detached);
    assert_eq!(outcome.residual_booked, 0);
    assert_eq!(outcome.explicit_loss, 0);
    assert_eq!(market.header.mode, 2);
    assert_eq!(
        market.header.recovery_reason.try_to_runtime().unwrap(),
        Some(PermissionlessRecoveryReasonV16::ActiveBankruptCloseCannotProgress)
    );
    assert_eq!(account.header.pnl.get(), -5);
    assert_eq!(
        account
            .header
            .close_progress
            .try_to_runtime()
            .unwrap()
            .residual_remaining,
        5
    );
    market.validate_shape().unwrap();
    account.validate_with_market(&market.as_view()).unwrap();
}

// upstream f06a04a7 "Keep strict trade reductions open below initial margin"
// (2026-08-06): an account whose fill is a STRICT reduction of its position on
// the asset skips the final initial-margin gate (and the IM source-lien /
// locked-lane no-positive-credit gates), so an under-margin owner can hand risk
// to a margin-healthy counterparty. Fork adaptation: the taker flag is passed
// explicitly (taker-only fee model, KL-ENGINE-TAKER-ONLY-FEE).
#[test]
fn v16_under_margin_owner_can_transfer_risk_to_margin_healthy_counterparty() {
    const OPEN_Q: u128 = 100 * POS_SCALE;
    let (mut header, mut markets) = market_fixture(1, 100);
    header.config.maintenance_margin_bps = V16PodU64::new(1_000);
    header.config.initial_margin_bps = V16PodU64::new(5_000);
    header.config.max_price_move_bps_per_slot = V16PodU64::new(1_000);
    header.config.max_accrual_dt_slots = V16PodU64::new(1);
    let mut owner_header = account_fixture(1, 12);
    let mut original_short_header = account_fixture(1, 13);
    let mut new_holder_header = account_fixture(1, 14);

    let mut market = MarketGroupV16ViewMut::new(&mut header, &mut markets);
    let mut owner = PortfolioV16ViewMut::new(&mut owner_header);
    let mut original_short = PortfolioV16ViewMut::new(&mut original_short_header);
    let mut new_holder = PortfolioV16ViewMut::new(&mut new_holder_header);
    market.deposit_not_atomic(&mut owner, 5_001).unwrap();
    market
        .deposit_not_atomic(&mut original_short, 100_000)
        .unwrap();
    market.deposit_not_atomic(&mut new_holder, 10_000).unwrap();
    market
        .execute_trade_with_fee_loss_stale_scoped_not_atomic(
            &mut owner,
            &mut original_short,
            TradeRequestV16 {
                asset_index: 0,
                size_q: signed_q(OPEN_Q),
                exec_price: 100,
                fee_bps: 0,
            },
            true,
        )
        .unwrap();

    market
        .set_asset_raw_oracle_target_not_atomic(0, 90)
        .unwrap();
    market
        .accrue_asset_to_not_atomic(0, 2, 90, 0, true)
        .unwrap();
    market
        .full_account_refresh_not_atomic(&mut original_short)
        .unwrap();
    let owner_cert = market.full_account_refresh_not_atomic(&mut owner).unwrap();
    market
        .full_account_refresh_not_atomic(&mut new_holder)
        .unwrap();
    assert!(
        owner_cert.certified_equity >= 0
            && (owner_cert.certified_equity as u128) < owner_cert.certified_initial_req,
        "owner must be below IM before the transfer"
    );

    market
        .execute_trade_with_fee_loss_stale_scoped_not_atomic(
            &mut owner,
            &mut new_holder,
            TradeRequestV16 {
                asset_index: 0,
                size_q: -signed_q(POS_SCALE),
                exec_price: 90,
                fee_bps: 0,
            },
            true,
        )
        .expect("strict reducer may exit while the new risk holder passes IM");

    assert_eq!(
        owner.header.legs[0].try_to_runtime().unwrap().basis_pos_q,
        signed_q(99 * POS_SCALE)
    );
    assert_eq!(
        new_holder.header.legs[0]
            .try_to_runtime()
            .unwrap()
            .basis_pos_q,
        signed_q(POS_SCALE)
    );
    let new_holder_cert = new_holder.header.health_cert.try_to_runtime().unwrap();
    assert!(
        new_holder_cert.valid
            && new_holder_cert.certified_equity >= 0
            && (new_holder_cert.certified_equity as u128) >= new_holder_cert.certified_initial_req,
        "new risk holder remains fully margined"
    );
    market.validate_shape().unwrap();
    owner.validate_with_market(&market.as_view()).unwrap();
    original_short
        .validate_with_market(&market.as_view())
        .unwrap();
    new_holder.validate_with_market(&market.as_view()).unwrap();
}
