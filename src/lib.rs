//! Percolator risk engine — v16.
//!
//! v16 keeps the account-local engine surface and adds source-domain realizable
//! credit accounting so positive PnL cannot be used beyond proven source-domain
//! backing.

#![no_std]
#![deny(unsafe_code)]

extern crate alloc;

#[cfg(kani)]
extern crate kani;

pub const POS_SCALE: u128 = 1_000_000;
pub const ADL_ONE: u128 = 1_000_000_000_000_000;
pub const MIN_A_SIDE: u128 = 100_000_000_000_000;
pub const MAX_ORACLE_PRICE: u64 = 1_000_000_000_000;
pub const FUNDING_DEN: u128 = 1_000_000_000;
pub const STRESS_CONSUMPTION_SCALE: u128 = 1_000_000_000;
pub const SOCIAL_WEIGHT_SCALE: u128 = ADL_ONE;
pub const SOCIAL_LOSS_DEN: u128 = 1_000_000_000_000_000_000_000;
pub const SUPPORT_WEIGHT_SCALE: u128 = 1_000_000;
pub const FULL_SUPPORT_WEIGHT: u128 = SUPPORT_WEIGHT_SCALE;
pub const BOUND_SCALE: u128 = 1_000_000_000_000;
pub const CREDIT_RATE_SCALE: u128 = 1_000_000_000_000;
pub const MAX_VAULT_TVL: u128 = 10_000_000_000_000_000;
pub const MAX_POSITION_ABS_Q: u128 = 100_000_000_000_000;
pub const MAX_ACCOUNT_NOTIONAL: u128 = 100_000_000_000_000_000_000;
pub const MAX_TRADE_SIZE_Q: u128 = MAX_POSITION_ABS_Q;
pub const MAX_OI_SIDE_Q: u128 = 100_000_000_000_000;
pub const MAX_TRADING_FEE_BPS: u64 = 10_000;
pub const MAX_MARGIN_BPS: u64 = 10_000;
pub const MAX_LIQUIDATION_FEE_BPS: u64 = 10_000;
pub const MAX_PROTOCOL_FEE_ABS: u128 = 1_000_000_000_000_000_000_000_000_000_000_000_000;
pub const MAX_WARMUP_SLOTS: u64 = u64::MAX;
pub const MAX_RESOLVE_PRICE_DEVIATION_BPS: u64 = 10_000;
pub const MAX_RECOVERY_FALLBACK_DEVIATION_BPS: u64 = MAX_RESOLVE_PRICE_DEVIATION_BPS;

#[cfg(kani)]
pub mod v16;
#[cfg(not(kani))]
mod v16;
#[cfg(kani)]
pub mod wide_math;
#[cfg(all(not(kani), feature = "fork-facade"))]
pub mod wide_math;
#[cfg(all(not(kani), not(feature = "fork-facade")))]
mod wide_math;

#[cfg(kani)]
pub use v16::*;
#[cfg(not(kani))]
pub use v16::{
    active_bitmap_count_ones, active_bitmap_empty, active_bitmap_get, active_bitmap_is_empty,
    backing_domain_fee_split_for_lien_delta_num, v16_domain_count_for_market_slots,
    v16_domain_pair_for_asset_index, AccrueAssetOutcomeV16, AssetLifecycleV16, AssetStateV16,
    AssetStateV16Account, BackingBucketStatusV16, BackingBucketV16, BackingBucketV16Account,
    BackingDomainFeeSplitV16, BatchTradeOutcomeV16, CloseProgressLedgerV16,
    CloseProgressLedgerV16Account, DeadLegForfeitOutcomeV16, EngineAssetSlotV16Account,
    HealthCertV16, HealthCertV16Account, InsuranceCreditReservationV16,
    InsuranceCreditReservationV16Account, LiquidationOutcomeV16, LiquidationRequestV16, Market,
    MarketGroupV16HeaderAccount, MarketGroupV16View, MarketGroupV16ViewMut, MarketModeV16,
    MarketSlotV16View, MarketSlotV16ViewMut, PermissionlessCrankActionV16,
    PermissionlessCrankRequestV16, PermissionlessProgressOutcomeV16,
    PermissionlessRecoveryReasonV16, PortfolioAccountV16Account, PortfolioLegV16,
    PortfolioLegV16Account, PortfolioSourceDomainV16Account, PortfolioV16View, PortfolioV16ViewMut,
    ProvenanceHeaderV16, ProvenanceHeaderV16Account, RebalanceOutcomeV16, RebalanceRequestV16,
    ResolvedCloseOutcomeV16, ResolvedPayoutLedgerV16, ResolvedPayoutLedgerV16Account,
    ResolvedPayoutReceiptV16, ResolvedPayoutReceiptV16Account, SideModeV16, SideV16,
    SourceCreditStateV16, SourceCreditStateV16Account, TradeRequestV16, V16ActiveBitmap, V16Config,
    V16ConfigAccount, V16Error, V16OptionalRecoveryReasonAccount, V16PodI128, V16PodU128,
    V16PodU16, V16PodU32, V16PodU64, V16Result, PORTFOLIO_SOURCE_DOMAIN_CAP, V16_ACCOUNT_VERSION,
    V16_EMPTY_ACTIVE_BITMAP, V16_LAYOUT_DISCRIMINATOR, V16_MAX_PORTFOLIO_ASSETS_N,
};

// kani_active_bitmap_set is gated #[cfg(any(kani, test, feature="fork-facade"))]
// in v16.rs; re-export it under fork-facade so the wrapper tests can call it.
#[cfg(all(not(kani), feature = "fork-facade"))]
pub use v16::kani_active_bitmap_set;

// ADL effective-quantity kernels for the fuzz targets (kani builds get them from the blanket re-export).
#[cfg(all(not(kani), feature = "fuzz"))]
pub use v16::{kani_adl_effective_quantity_ceil, kani_raw_basis_for_adl_effective_quantity};

// v17 fork-facade re-exports — present only when the fork-facade feature is enabled (the wrapper
// opts in on its engine dep). Keeps the production frozen surface minimal by default. Under kani the
// blanket `pub use v16::*` above already covers these.
#[cfg(all(not(kani), feature = "fork-facade"))]
pub use v16::fork_facade;
#[cfg(all(not(kani), feature = "fork-facade"))]
pub use v16::lp_vault;
#[cfg(all(not(kani), feature = "fork-facade"))]
pub use v16::FeePolicyUpdateV16;
#[cfg(all(not(kani), feature = "fork-facade"))]
pub use v16::STRESS_ENVELOPE_TRIGGER_BPS_E9;
// Matrix row 38: wide_math is made pub mod under fork-facade (see mod declaration above)
// so the wrapper can use percolator::wide_math::U256 in expected_source_credit_rate_num.
// The module is private by default (frozen surface); fork-facade elevates it to pub.
