# Risk Engine Spec (Source of Truth) — v16.8.3 Realizable Full Shared Cross-Margin

**Design:** protected principal + junior profit claims + lazy A/K/F side indices, native 128-bit persistent state.
**Status:** implementation source of truth. Normative terms are **MUST**, **MUST NOT**, **SHOULD**, **MAY**.
**Scope:** one perpetual DEX risk engine for one quote-token vault.

This revision supersedes v16.8.2 for the product goal of Hyperliquid-like cross-margin UX in permissionless accounts while containing oracle/market failure by limiting usable PnL to realizable source-domain backing.

```text
RiskNotional_i = ceil(abs(effective_pos_q(i)) * oracle_price / POS_SCALE)
```

This ceiling is load-bearing. A nonzero fractional quote-notional position has nonzero risk notional and cannot evade maintenance by floor rounding. Floor oracle notional MAY be displayed or used by wrapper policy, but MUST NOT be used for margin.

- Trade fees use executed floor notional:

```text
trade_notional = floor(size_q * exec_price / POS_SCALE)
```

### 1.3 A/K/F scales

```text
ADL_ONE    = 1_000_000_000_000_000
FUNDING_DEN = 1_000_000_000
```

`A_side` is dimensionless and scaled by `ADL_ONE`. `K_side` has units `ADL scale * quote/base`. `F_side_num` has units `ADL scale * quote/base * FUNDING_DEN`.

### 1.4 Hard bounds

```text
MAX_VAULT_TVL                 = 10_000_000_000_000_000
MAX_ORACLE_PRICE              = 1_000_000_000_000
MAX_POSITION_ABS_Q            = 100_000_000_000_000
MAX_TRADE_SIZE_Q              = MAX_POSITION_ABS_Q
MAX_OI_SIDE_Q                 = 100_000_000_000_000
MAX_ACCOUNT_NOTIONAL          = 100_000_000_000_000_000_000
MAX_PROTOCOL_FEE_ABS          = 1_000_000_000_000_000_000_000_000_000_000_000_000
GLOBAL_MAX_ABS_FUNDING_E9_PER_SLOT = 10_000
MAX_TRADING_FEE_BPS           = 10_000
MAX_INITIAL_BPS               = 10_000
MAX_MAINTENANCE_BPS           = 10_000
MAX_LIQUIDATION_FEE_BPS       = 10_000
MAX_MATERIALIZED_ACCOUNTS     = 1_000_000
MIN_A_SIDE                    = 100_000_000_000_000
MAX_WARMUP_SLOTS              = 18_446_744_073_709_551_615
MAX_RESOLVE_PRICE_DEVIATION_BPS = 10_000
PRICE_MOVE_CONSUMPTION_SCALE  = 1_000_000_000
```

`MAX_ACTIVE_POSITIONS_PER_SIDE` MUST be finite and MUST NOT exceed `MAX_MATERIALIZED_ACCOUNTS`.

### 1.5 Immutable per-market configuration

The market stores immutable:

```text
cfg_h_min, cfg_h_max
cfg_maintenance_bps, cfg_initial_bps
cfg_trading_fee_bps
cfg_liquidation_fee_bps, cfg_liquidation_fee_cap, cfg_min_liquidation_abs
cfg_min_nonzero_mm_req, cfg_min_nonzero_im_req
cfg_resolve_price_deviation_bps
cfg_max_active_positions_per_side
cfg_max_accrual_dt_slots
cfg_max_abs_funding_e9_per_slot
cfg_max_price_move_bps_per_slot
cfg_min_funding_lifetime_slots
cfg_account_index_capacity
```

Initialization MUST require:

```text
0 < cfg_min_nonzero_mm_req < cfg_min_nonzero_im_req
0 <= cfg_maintenance_bps <= MAX_MAINTENANCE_BPS
cfg_maintenance_bps <= cfg_initial_bps <= MAX_INITIAL_BPS
0 <= cfg_trading_fee_bps <= MAX_TRADING_FEE_BPS
0 <= cfg_liquidation_fee_bps <= MAX_LIQUIDATION_FEE_BPS
0 <= cfg_min_liquidation_abs <= cfg_liquidation_fee_cap <= MAX_PROTOCOL_FEE_ABS
0 <= cfg_h_min <= cfg_h_max <= MAX_WARMUP_SLOTS
cfg_h_max > 0
0 <= cfg_resolve_price_deviation_bps <= MAX_RESOLVE_PRICE_DEVIATION_BPS
0 < cfg_account_index_capacity <= MAX_MATERIALIZED_ACCOUNTS
0 < cfg_max_active_positions_per_side <= MAX_ACTIVE_POSITIONS_PER_SIDE
cfg_max_active_positions_per_side <= cfg_account_index_capacity
0 < cfg_max_accrual_dt_slots <= MAX_WARMUP_SLOTS
0 <= cfg_max_abs_funding_e9_per_slot <= GLOBAL_MAX_ABS_FUNDING_E9_PER_SLOT
0 < cfg_max_price_move_bps_per_slot
```

Live admission pairs MUST satisfy:

```text
cfg_margin_mode == RealizableFullSharedCrossMargin
cfg_asset_support_weight(asset) == FULL_SUPPORT_WEIGHT for every Active asset
cfg_source_credit_mode == CounterpartyRealizableBackingCapped
cfg_credit_lien_mode == RequiredForRiskAndSettlementUse
cfg_claim_bound_mode == ExactScaledDecomposedReplaceable
cfg_backing_mode == ReservedFreshCounterpartyBacking
cfg_bankruptcy_mode == LegAttributedMarketSideB
cfg_insurance_mode in {DomainBudgeted, GlobalProtocolFirstLossWithCaps}
cfg_asset_set_lifecycle == MutableWithActivationProofs
cfg_instance_isolation == true
cfg_public_liveness_profile == CrankForward
cfg_permissionless_recovery_enabled == true
cfg_recovery_fallback_price_enabled == true
cfg_max_recovery_fallback_deviation_bps <= MAX_RECOVERY_FALLBACK_DEVIATION_BPS
cfg_recovery_fallback_envelope_enabled == true
cfg_owner_dead_leg_forfeit_enabled == true
cfg_full_refresh_required_for_favorable_actions == true
cfg_stale_certificate_penalty_enabled == true
cfg_deterministic_portfolio_liquidation_enabled == true
cfg_close_state_scope == AccountLocalWithPreemptibleDomainLocks
cfg_close_conflict_policy == DeterministicPreemptivePriority
cfg_no_global_B_index == true
cfg_no_cross_instance_socialization == true
cfg_asset_activation_cooldown_slots >= cfg_min_public_refresh_grace_slots
cfg_public_b_chunk_atoms > 0
cfg_max_account_b_settlement_chunks > 0
cfg_max_bankrupt_close_chunks > 0
cfg_max_bankrupt_close_lifetime_slots > 0
cfg_credit_lien_revalidation_required == true
cfg_backing_freshness_buckets == 1
cfg_pending_obligation_settlement_chunks > 0
cfg_close_drift_reserve_enabled == true
cfg_close_drift_anchor_mode == ImmutableReferenceSlot
```

For public or permissionless wrappers with untrusted live oracle or execution-price PnL, wrapper policy MUST additionally enforce `admit_h_min > 0`.

### 1.6 Funding and solvency-envelope validation

Initialization MUST validate, in exact wide arithmetic:

```text
ADL_ONE * MAX_ORACLE_PRICE * cfg_max_abs_funding_e9_per_slot * cfg_max_accrual_dt_slots <= i128::MAX
cfg_min_funding_lifetime_slots >= cfg_max_accrual_dt_slots
ADL_ONE * MAX_ORACLE_PRICE * cfg_max_abs_funding_e9_per_slot * cfg_min_funding_lifetime_slots <= i128::MAX
```

Initialization MUST also validate the exact per-risk-notional envelope below for every integer risk notional `N` with `1 <= N <= MAX_ACCOUNT_NOTIONAL`, by an exact bounded breakpoint/interval proof or by a stronger conservative sufficient proof. Unbounded runtime loops over all `N` are forbidden on constrained runtimes.

Let:

```text
price_budget_bps  = cfg_max_price_move_bps_per_slot * cfg_max_accrual_dt_slots
funding_budget_num = cfg_max_abs_funding_e9_per_slot * cfg_max_accrual_dt_slots * 10_000
loss_budget_num   = price_budget_bps * FUNDING_DEN + funding_budget_num
```

For each `N`:

```text
price_funding_loss_N = ceil(N * loss_budget_num / (10_000 * FUNDING_DEN))
worst_liq_notional_N = ceil(N * (10_000 + price_budget_bps) / 10_000)
liq_fee_raw_N        = ceil(worst_liq_notional_N * cfg_liquidation_fee_bps / 10_000)
liq_fee_N            = min(max(liq_fee_raw_N, cfg_min_liquidation_abs), cfg_liquidation_fee_cap)
mm_req_N             = max(floor(N * cfg_maintenance_bps / 10_000), cfg_min_nonzero_mm_req)
require price_funding_loss_N + liq_fee_N <= mm_req_N
```

This law is the construction-level self-neutral-siphon boundary. It accounts for fractional funding, integer rounding, worst adverse post-move liquidation notional, bps fees, fee floors, and fee caps. Implementations MUST NOT substitute floor-funded bps budgeting, pre-move liquidation notional, floor risk notional, or a two-point small-notional shortcut unless accompanied by an exact proof covering every intervening and larger notional.

If a deployment defines `permissionless_resolve_stale_slots`, initialization MUST require:

```text
permissionless_resolve_stale_slots <= cfg_max_accrual_dt_slots
```

### 1.7 Wrapper-fed effective price and raw oracle target

Oracle normalization, source selection, target storage, and rate limiting are wrapper-owned. The engine only validates and accrues the effective `oracle_price` passed to it.

A compliant public wrapper SHOULD maintain distinct fields equivalent to:

```text
oracle_target_price      // latest validated normalized external target
oracle_target_publish_ts // target source timestamp or publish slot
last_effective_price     // last price actually fed into engine accrual, equal to engine P_last when synchronized
```

The wrapper MUST NOT overload `last_effective_price` as the raw target. If the external target jumps beyond the engine cap, the wrapper keeps the raw target and feeds a capped staircase of effective prices until caught up.

For an exposed live market (`OI_eff_long != 0 || OI_eff_short != 0`), the wrapper-fed next effective price SHOULD be computed by the deterministic clamp law:

```text
dt = now_slot - slot_last
if target == P_last or dt == 0:
    next_price = P_last
else:
    max_delta = floor(P_last * cfg_max_price_move_bps_per_slot * dt / 10_000)
    next_price = clamp_toward(P_last, target, max_delta)
```

The multiplication MUST use exact wide arithmetic; `max_delta` MAY be capped to the price type maximum after the exact quotient. `clamp_toward` moves toward `target` by at most `max_delta` and never overshoots. The result MUST satisfy the engine cap in §5.3.

Normative consequences:

- Same-slot exposed cranks (`dt == 0`) MUST pass `P_last`; price catch-up requires elapsed slots. They MAY still do Phase 1 liquidation checks and Phase 2 round-robin touches at the unchanged effective price.
- If exposed `target != P_last`, `dt > 0`, and the computed `max_delta == 0`, ordinary live catch-up cannot make progress at the deployed price scale/cap. The wrapper MUST treat this as `CatchupRequired` / recovery territory and MUST NOT advance `slot_last` by feeding the unchanged price merely to bypass the lag.
- If exposed `dt > cfg_max_accrual_dt_slots` and the target differs from `P_last`, ordinary one-step live catch-up is unavailable. The wrapper MUST use an explicit recovery path, privileged degenerate resolution, or a separately specified atomic multi-accrual procedure that preserves all §5.3 mutation-order and cap invariants.
- If both OI sides are zero, no live position can lose equity, so the wrapper MAY feed the raw target directly subject to ordinary price validity.
- Feeding a cap-violating raw target into exposed live accrual is non-compliant and should fail before engine state mutation.

While `oracle_target_price != P_last`, the market is intentionally using a lagged effective engine price. For public wrappers, keeper progress, liquidation attempts, settlement, and structural sweep MAY continue at the effective price, but user operations that are risk-increasing or extraction-sensitive MUST either be rejected or pass a conservative wrapper shadow policy using both the effective engine price and the raw target. At minimum, public wrappers MUST reject risk-increasing user trades during target/effective-price divergence unless they are priced and margin-checked under a stricter dual-price policy that removes the known-lag free option.

---

## 2. State

### 2.1 Account state

Each materialized account stores:

```text
C_i: u128                      protected principal
PNL_i: i128                    realized PnL claim
R_i: u128                      reserved positive PnL, 0 <= R_i <= max(PNL_i,0)
basis_pos_q_i: i128
a_basis_i: u128
k_snap_i: i128
f_snap_i: i128
epoch_snap_i: u64
fee_credits_i: i128            <= 0, never i128::MIN
last_fee_slot_i: u64
```

Live accounts additionally store at most one scheduled bucket and one pending bucket.

Scheduled bucket:

```text
sched_present_i: bool
sched_remaining_q_i: u128
sched_anchor_q_i: u128
sched_start_slot_i: u64
sched_horizon_i: u64
sched_release_q_i: u128
```

Pending bucket:

```text
pending_present_i: bool
pending_remaining_q_i: u128
pending_horizon_i: u64
```

For long-profit claims, use long-side best-case price/basis; for short-profit claims, use short-side best-case price/basis. B loss is nonnegative and excluded from positive upper bounds. Bucket members must remain inside stored basis/K/F ranges; otherwise the bucket is recomputed, split, hard-maxed, or the market fails closed. The claim bound MUST never understate true positive claims owed by the source domain.

### 2.2 Counterparty backing and insurance-credit reservations

A full account refresh computes, for every domain where the account currently owes loss, a deterministic `BackingReservationPlan`.

The plan MUST be built from every active leg before any newly observed K/F delta is applied. Its canonical order is `(negative phase first, source_domain ascending, canonical leg_slot ascending)`. A plan entry MAY cache the arithmetic delta computed while building that plan because the canonical account representation has at most one leg per asset and applying an earlier entry cannot mutate another asset's K/F target. Immediately before application, the implementation MUST reload the leg and asset, recompute the inexpensive live K/F target, and reject unless it matches the cached target; it MUST also recompute and match the phase and source domain encoded in the key. This is ordering, not netting: each negative delta reserves available capital in its own source domain, and a same-refresh positive delta cannot erase that loss before reservation. Positive face that existed before the refresh remains subject to the ordinary source-support and face-burn rules when a new loss is applied.

A backing reservation may be funded only by:
- senior capital `C_i`;
- already realized nonjunior quote gains;
- released cancel/deposit escrow;
- settled pending-obligation rebate;
- settlement-quality source credit from another domain whose backing is consumed or liened atomically.

Open positive PnL that is not converted into a source-credit lien is not backing. Circular backing is forbidden: a reservation chain MUST strictly consume or lien already available backing and MUST NOT return to a previously visited source domain without external senior capital.

Backing freshness is maintained with bounded buckets. The v16.8 public profile uses exactly one freshness bucket per source domain. Any future multi-bucket profile MUST either refill the bucket holding the consumed receivable or recompute the source aggregate before admitting new backing; it MUST NOT compare a source-wide receivable against an unrelated empty bucket.

```text
R_i = scheduled_remaining + pending_remaining
if sched_present: 0 < sched_remaining <= sched_anchor, cfg_h_min <= sched_horizon <= cfg_h_max, sched_release <= sched_anchor
if pending_present: 0 < pending_remaining, cfg_h_min <= pending_horizon <= cfg_h_max
if R_i == 0: both buckets absent
pending never matures while pending
```

If `basis_pos_q_i != 0`, then `a_basis_i > 0`. Any helper dividing by `a_basis_i` or `a_basis_i * POS_SCALE` MUST fail conservatively if the denominator is zero.

On resolved markets, reserve storage is inert and MUST be cleared by `prepare_account_for_resolved_touch` before mutating resolved PnL.

Wrapper-owned annotation fields MAY exist, but the engine MUST never read them to decide margin, liquidation, fee routing, admission, accrual, resolution, reset, reclamation, conservation, or authorization. They MUST be canonicalized on materialization and cleared on free-slot reset.

### 2.2 Global state

The engine stores:

```text
create_lien_from_counterparty_backing(bucket, amount):
    require bucket.status == Fresh
    require bucket.fresh_unliened_backing_num >= amount
    bucket.fresh_unliened_backing_num -= amount
    bucket.valid_liened_backing_num   += amount
    SourceCreditState.valid_liened_backing_num += amount
    // fresh_reserved_backing_num unchanged

consume_lien_backing(bucket, amount):
    require bucket.valid_liened_backing_num >= amount
    require amount % BOUND_SCALE == 0 when consumed for quote-atom residual cure
    cure_atoms = amount / BOUND_SCALE
    bucket.valid_liened_backing_num     -= amount
    bucket.consumed_liened_backing_num  += amount
    SourceCreditState.valid_liened_backing_num -= amount
    SourceCreditState.fresh_reserved_backing_num -= amount
    SourceCreditState.spent_backing_num += amount
    SourceCreditState.provider_receivable_num += amount
    reduce or finalize the locked source-domain claim in the same atomic step
    record only cure_atoms, never amount, in quote-atom close/support ledgers

add_fresh_counterparty_backing(bucket, amount):
    require amount > 0 and bucket accepts the target freshness epoch
    refill = min(amount, SourceCreditState.provider_receivable_num)
    require refill <= bucket.consumed_liened_backing_num
    bucket.consumed_liened_backing_num -= refill
    SourceCreditState.provider_receivable_num -= refill
    bucket.fresh_unliened_backing_num += amount
    SourceCreditState.fresh_reserved_backing_num += amount
    // spent_backing_num unchanged: it remains cumulative audit state

release_lien_backing(bucket, amount):
    require bucket.valid_liened_backing_num >= amount
    bucket.valid_liened_backing_num   -= amount
    bucket.fresh_unliened_backing_num += amount
    SourceCreditState.valid_liened_backing_num -= amount
    // fresh_reserved_backing_num unchanged

impair_lien_backing(bucket, amount):
    require bucket.valid_liened_backing_num >= amount
    bucket.valid_liened_backing_num    -= amount
    bucket.impaired_liened_backing_num += amount
    SourceCreditState.valid_liened_backing_num -= amount
    SourceCreditState.fresh_reserved_backing_num -= amount
    SourceCreditState.impaired_liened_backing_num += amount
```

A consumed lien MUST NOT remain in `bucket.valid_liened_backing_num`. A released lien MUST NOT remain in `SourceCreditState.valid_liened_backing_num`. An impaired lien MUST NOT remain in fresh backing. These equalities are load-bearing invariants, not implementation suggestions.

Consumed counterparty backing is recoverable principal, not a fee. Refill MUST be deterministic and source-domain local: a future inflow for domain `D` refills `D`'s outstanding consumed backing before it can be interpreted as excess new backing for another domain or another instance. Refill does not resurrect a consumed claim, does not reduce cumulative `spent_backing_num`, and does not create token value; it only moves independently locked backing back into the Fresh bucket while lowering the outstanding receivable.

Backing fee schedules are wrapper/product policy: the wrapper may choose a fee rate from time, utilization, market profile, or provider terms. Any fee that changes account capital, provider accounting, insurance, vault stock, source credit, or backing availability MUST be charged through an engine transition with a balanced `TokenValueFlowProof` and the same source-domain freshness/lien checks. The wrapper MUST NOT hand-edit backing-fee ledger state outside the engine.

Unliened backing in an expired bucket contributes zero. Liened backing in an expiring bucket MUST NOT cause `available_backing_num` underflow or inflation. On expiry, the engine MUST do one of the following before any credit-rate read:
1. refresh and roll the bucket forward with a full account proof;
2. atomically expire the bucket:
   ```text
   expired_unliened = bucket.fresh_unliened_backing_num
   expired_liened   = bucket.valid_liened_backing_num
   expired_total    = expired_unliened + expired_liened

   SourceCreditState.fresh_reserved_backing_num -= expired_total
   SourceCreditState.valid_liened_backing_num   -= expired_liened
   SourceCreditState.impaired_liened_backing_num += expired_liened

   bucket.fresh_unliened_backing_num = 0
   bucket.valid_liened_backing_num = 0
   bucket.impaired_liened_backing_num += expired_liened
   bucket.status = Impaired if expired_liened > 0 else Expired

   // consumed_liened_backing_num is unchanged audit state and MUST already have
   // been removed from source fresh/valid aggregates at consumption time.
   ```
3. route the source domain to recovery.

The expiry/impairment transition is aggregate and bounded by bucket id. Individual liens referencing an impaired bucket become impaired by bucket status and settle later through bounded cranks. Impaired lien backing is not available for new credit and does not count toward `available_backing_num`.

The instruction MUST prove after expiry by independent recomputation from bucket state:
```text
fresh_reserved_backing_num_after = sum_FreshBuckets(fresh_unliened + valid_liened)
valid_liened_backing_num_after   = sum_FreshBuckets(valid_liened)
available_backing_num_after      = recomputed_fresh_unliened + available_insurance_credit
available_backing_num_after <= available_backing_num_before
credit_rate_num_after <= credit_rate_num_before unless positive_claim_bound_num also decreased by an independently valid bounded recomputation
```
A pure expiry transition that increases available backing or credit rate is invalid and MUST revert or recover. A before/after inequality alone is insufficient unless the recomputed aggregate equalities also hold.

Insurance may contribute to source credit only through a canonical live insurance-credit reservation recorded in `InsuranceLedger.source_credit_reserved_num[D]`. `SourceCreditState[D].insurance_credit_reserved_num` is a derived view of that canonical ledger entry, not a second writer.

```text
amount_from_bound_num_up(x_num) = ceil(x_num / BOUND_SCALE)

InsuranceCreditReservation[D] {
    insurance_credit_reserved_num       // canonical live reservation, scaled
    valid_liened_insurance_num          // valid liens funded from this reservation
    impaired_liened_insurance_num       // impaired liens still encumbering this reservation
    consumed_insurance_num              // cumulative audit, no longer live
    source_credit_epoch
}
```

Granting or increasing `insurance_credit_reserved_num[D]` MUST atomically reserve from the domain's unspent current insurance capacity. Source-credit insurance reservations MUST NOT be drawn from global protocol first-loss capacity unless the reservation is explicitly recorded in a separate global reservation field and included in the same live-encumbrance invariant. The same insurance atom cannot simultaneously be:
- a source-credit insurance reservation;
- staged residual insurance;
- spent residual insurance;
- available global protocol first-loss budget; or
- available domain insurance budget.

`insurance_ledger.total_available` is current unspent insurance capital in the vault. Cumulative spent insurance is reflected by a lower `I`/`total_available`; it MUST NOT also be counted as a live encumbrance.

Live insurance encumbrance:

```text
live_source_credit_insurance =
    sum_D amount_from_bound_num_up(source_credit_reserved_num[D])

live_domain_staged =
    sum_D staged_domain_insurance_debits[D]

live_global_staged =
    global_protocol_staged_debits

live_source_credit_insurance + live_domain_staged + live_global_staged
    <= insurance_ledger.total_available
```

Per-domain cap:

```text
domain_spent[D]
  + staged_domain_insurance_debits[D]
  + amount_from_bound_num_up(source_credit_reserved_num[D])
  <= domain_budget[D]
```

Insurance-backed lien lifecycle arithmetic mirrors counterparty-backed lien arithmetic. All amounts below are scaled insurance reservation numerators.

```text
create_lien_from_insurance(reservation, amount):
    require reservation.insurance_credit_reserved_num
        >= reservation.valid_liened_insurance_num
         + reservation.impaired_liened_insurance_num
         + amount
    reservation.valid_liened_insurance_num += amount
    SourceCreditState.valid_liened_insurance_num += amount
    // insurance_credit_reserved_num unchanged; available insurance credit decreases by amount

consume_lien_from_insurance(reservation, amount):
    require reservation.valid_liened_insurance_num >= amount
    spend_atoms = amount_from_bound_num_up(amount)

    reservation.valid_liened_insurance_num -= amount
    reservation.insurance_credit_reserved_num -= amount
    reservation.consumed_insurance_num += amount

    InsuranceLedger.source_credit_reserved_num[D] -= amount
    SourceCreditState.valid_liened_insurance_num -= amount
    // SourceCreditState.insurance_credit_reserved_num view decreases with the canonical ledger

    InsuranceLedger.domain_spent[D] += spend_atoms
    InsuranceLedger.total_available -= spend_atoms
    I -= spend_atoms
    if the consume instruction pays external quote tokens:
        V -= spend_atoms
        record external_insurance_payout in the TokenValueFlowProof
    else:
        record exactly one internal quote-value credit in the TokenValueFlowProof and close/payout state:
            - CloseProgressLedger.insurance_spent for residual cure; or
            - staged_domain_insurance_debit for staged close insurance; or
            - ResolvedPayoutLedger.paid_effective for resolved/recovery payout.
        The same consume MUST NOT increment consumed_counterparty_credit_lien_backing,
        support_consumed, or any generic source-credit-support term.

    reduce or finalize the locked source-domain claim in the same atomic step
    require all senior and quote-value and reservation-conservation invariants hold after the debit

release_lien_from_insurance(reservation, amount):
    require reservation.valid_liened_insurance_num >= amount
    reservation.valid_liened_insurance_num -= amount
    SourceCreditState.valid_liened_insurance_num -= amount
    // insurance_credit_reserved_num unchanged; available insurance credit increases by amount

impair_lien_from_insurance(reservation, amount):
    require reservation.valid_liened_insurance_num >= amount
    reservation.valid_liened_insurance_num -= amount
    reservation.impaired_liened_insurance_num += amount
    SourceCreditState.valid_liened_insurance_num -= amount
    SourceCreditState.impaired_liened_insurance_num += amount
    // insurance_credit_reserved_num unchanged; impaired amount remains encumbered and unavailable

recover_or_reconcile_impaired_insurance_lien(reservation, amount, outcome):
    require reservation.impaired_liened_insurance_num >= amount
    if outcome == Released:
        reservation.impaired_liened_insurance_num -= amount
        reservation.insurance_credit_reserved_num -= amount
        InsuranceLedger.source_credit_reserved_num[D] -= amount
        SourceCreditState.impaired_liened_insurance_num -= amount
    if outcome == Consumed:
        reservation.impaired_liened_insurance_num -= amount
        reservation.insurance_credit_reserved_num -= amount
        InsuranceLedger.source_credit_reserved_num[D] -= amount
        SourceCreditState.impaired_liened_insurance_num -= amount
        spend_atoms = amount_from_bound_num_up(amount)
        InsuranceLedger.domain_spent[D] += spend_atoms
        InsuranceLedger.total_available -= spend_atoms
        I -= spend_atoms
        if the recovery/settlement transfer pays external quote tokens:
            V -= spend_atoms
            record external_insurance_payout in the TokenValueFlowProof
        else:
            record exactly one internal recovery/close quote-value credit in the TokenValueFlowProof
        preserve senior and quote-value and reservation-conservation invariants
```

At all times:

```text
insurance_credit_reserved_num
    >= valid_liened_insurance_num + impaired_liened_insurance_num

InsuranceLedger.source_credit_reserved_num[D]
    == InsuranceCreditReservation[D].insurance_credit_reserved_num
    == SourceCreditState[D].insurance_credit_reserved_num
```

`impaired_liened_insurance_num` is a live encumbrance and MUST be subtracted from available insurance credit. It is unavailable for new liens until explicitly released or consumed by recovery. A transition that moves an insurance-backed lien between valid, impaired, consumed, and released states MUST independently recompute:

```text
available_insurance_credit_num =
    insurance_credit_reserved_num
  - valid_liened_insurance_num
  - impaired_liened_insurance_num
```

and MUST NOT increase available insurance credit or credit rate unless the transition is a genuine release or a new insurance reservation is added.

`SourceCreditLien` records the backing source:

```text
SourceCreditLien {
    account_id
    source_domain
    face_claim_locked
    effective_credit_reserved
    backing_reserved
    backing_source in {CounterpartyBucket, InsuranceReservation}
    backing_bucket_id optional
    insurance_reservation_id optional
    credit_rate_num_at_creation
    credit_epoch
    status in {Valid, Impaired, Consumed, Released}
    purpose in {Risk, Withdrawal, Conversion, Fee, ResidualCure, Payout}
}
```

Creating a lien atomically:
1. verifies the account has un-liened positive claim face in that source domain;
2. computes required face and backing;
3. requires `credit_rate_num > 0`;
4. requires `required_backing <= available_backing_num`;
5. locks face claim so it cannot be reused for soft credit, another lien, or another instance;
6. chooses a deterministic backing source:
   - if `CounterpartyBucket`, call `create_lien_from_counterparty_backing` and record `backing_bucket_id`;
   - if `InsuranceReservation`, call `create_lien_from_insurance` and record `insurance_reservation_id`;
7. records credit epoch and purpose.

For effective credit `E` measured in quote atoms:

```text
required_face_num = ceil(E * BOUND_SCALE * CREDIT_RATE_SCALE / credit_rate_num)
required_backing_num = E * BOUND_SCALE
```

A lien can be released only by reversing the dependent risk, consuming it into settlement, or recovery reconciliation. If a counterparty backing bucket expires or insurance backing becomes impaired, the lien becomes `Impaired`. Insurance backing has no time-expiry bucket; it becomes impaired only by deterministic events: source-domain Recovery, market-group Recovery, insurance-reservation invariant failure, domain/global insurance cap exhaustion affecting the reservation, or governance-declared insurance impairment routed through recovery. Recovery MUST call `impair_lien_from_insurance` or `recover_or_reconcile_impaired_insurance_lien` for affected insurance-backed liens before any favorable action can use that source domain. An impaired lien cannot support new risk or payout and adds an impaired-lien penalty to the owning account until it deleverages, liquidates, ADLs, refreshes with new backing, or recovers.

Locked face claim MUST be excluded from soft maintenance credit and from any further lien calculation. This prevents the same positive PnL from being counted once as soft equity and again as liened equity.

Close/support classification for source-credit liens:

```text
if backing_source == CounterpartyBucket and purpose == ResidualCure:
    consumed scaled backing amount is converted to cure_atoms = amount / BOUND_SCALE
    cure_atoms is recorded as consumed_counterparty_credit_lien_backing
    and MUST NOT be recorded as insurance_spent.

if backing_source == InsuranceReservation and purpose == ResidualCure:
    consumed value is recorded as insurance_spent
    and MUST NOT be recorded as consumed_counterparty_credit_lien_backing,
    support_consumed, or generic source-credit support.

if backing_source == InsuranceReservation and purpose in {Withdrawal, Conversion, Fee, Payout}:
    consumed value is recorded as external insurance-backed payout/spend
    with the matching V/I/insurance-ledger debit.
```

Every consumed lien MUST be classified by `backing_source` before it mutates any close ledger. A lien consumption that would increment two residual-cure categories, or none, is an invariant failure and MUST revert or route to recovery.

Insurance-backed lien impairment triggers:
- source domain enters `Recovery` or `DrainOnly` with the reservation not proven usable;
- the insurance reservation is invalidated, suspended, or no longer within domain/global caps;
- market group enters `Recovery` and the reservation is not explicitly preserved;
- recovery marks the backing unavailable.

Insurance does not expire by time unless an explicit configured expiry policy exists. If such a policy exists, expiry MUST call `impair_lien_from_insurance` or release/consume the lien in the same bounded step.

### 2.4 Soft maintenance credit

Maintenance may use soft source credit without reserving a lien:

```text
soft_leg_credit =
    floor(leg_local_positive_value * credit_rate_num[source_domain]
          / CREDIT_RATE_SCALE)
```

Soft credit is recomputed on every full refresh and every favorable action. It creates no payout right and no durable support. If the source rate falls, health falls immediately.

Trade approval that increases risk MUST create liens for any positive credit beyond no-positive-credit equity. Purely risk-reducing trades may use soft credit only for validation.

-------------------------------------------------------------------------------
3. Asset lifecycle
-------------------------------------------------------------------------------

Asset slots are bounded by `N`:

```text
Disabled -> PendingActivation -> Active -> DrainOnly -> Retired
                                      \-> Recovery -> Retired
```

Activation requires:
- slot Disabled or Retired;
- no remaining OI, weights, B, K/F, claims, backing, liens, pending barriers, pending obligations, close ledgers, or stale accounts in the slot;
- oracle, price, funding, B-headroom, claim-bound, backing, close-progress, and portfolio-envelope proofs pass for the whole instance;
- support weight exactly `FULL_SUPPORT_WEIGHT`;
- activation cooldown satisfied;
- `config_hash`, `risk_epoch`, and `asset_set_epoch` incremented;
- certificates fail closed unless their schema explicitly excludes the new asset.

DrainOnly blocks risk increase and new attaches. Retired requires zero OI, zero stored positions, no pending barriers, no obligations, no liens, all close ledgers finalized/canceled, and all prior-epoch stale accounts settled/migrated/recovered. A `ResetPending` side cannot reset again until all prior-epoch stale accounts are settled, migrated, or recovered.

-------------------------------------------------------------------------------
4. State
-------------------------------------------------------------------------------

```text
MarketGroup {
    instance_id
    V, I, C_tot
    materialized_portfolio_count_unbounded_counter

    risk_epoch
    oracle_epoch
    funding_epoch
    asset_set_epoch
    current_slot

    assets[0..N)
    source_credit_ledger[(asset, side)]
    source_credit_liens
    domain_locks[(asset, side)]
    insurance_ledger
    close_progress_ledger
    pending_domain_loss_barriers[(asset, side)]
    pending_obligation_aggregates[(barrier_id)]
    pending_obligation_ledger
    resolved_payout_ledger optional
    global_stale_penalty_params
    mode in {Live, Resolved, Recovery}
}
```


```text
InsuranceLedger {
    total_available                         // current unspent insurance capital in the vault
    domain_budget[(asset, side)]            // per-domain cap
    domain_spent[(asset, side)]             // cumulative spent for cap/audit only
    domain_global_cap[(asset, side)]
    domain_global_spent[(asset, side)]      // cumulative global first-loss spend by domain
    staged_domain_insurance_debits[(asset, side)]
    global_protocol_budget
    global_protocol_spent                   // cumulative spent for cap/audit only
    global_protocol_staged_debits
    source_credit_reserved_num[(asset, side)]   // canonical live source-credit insurance reservation
}
```

Insurance-credit invariants:

```text
live_source_credit_insurance =
    sum_D amount_from_bound_num_up(source_credit_reserved_num[D])

live_domain_staged =
    sum_D staged_domain_insurance_debits[D]

live_source_credit_insurance + live_domain_staged + global_protocol_staged_debits
    <= total_available

for every D:
    domain_spent[D]
  + staged_domain_insurance_debits[D]
  + amount_from_bound_num_up(source_credit_reserved_num[D])
  <= domain_budget[D]

global_protocol_spent + global_protocol_staged_debits
    <= global_protocol_budget
```

`InsuranceLedger.source_credit_reserved_num[D]` is canonical. `SourceCreditState[D].insurance_credit_reserved_num` MUST be read as a derived view or updated only by the same helper that mutates the insurance ledger. A desynchronized duplicate value is an invariant failure.


```text
Asset {
    lifecycle
    raw_oracle_target_price
    effective_price
    fund_px_last
    slot_last

    A_long, A_short
    K_long, K_short
    F_long_num, F_short_num

    B_long_num, B_short_num
    B_epoch_start_long_num, B_epoch_start_short_num
    K_epoch_start_long, K_epoch_start_short
    F_epoch_start_long_num, F_epoch_start_short_num
    A_epoch_start_long, A_epoch_start_short

    OI_eff_long, OI_eff_short
    stored_pos_count_long, stored_pos_count_short
    stale_account_count_long, stale_account_count_short

    loss_weight_sum_long, loss_weight_sum_short
    social_loss_remainder_long_num, social_loss_remainder_short_num
    social_loss_dust_long_num, social_loss_dust_short_num
    explicit_unallocated_loss_long, explicit_unallocated_loss_short

    support_weight = FULL_SUPPORT_WEIGHT when Active
    recovery_reference_price
    fallback_recovery_price
    recovery_fallback_deviation_bps
    epoch_long, epoch_short
    mode_long, mode_short in {Normal, DrainOnly, ResetPending}
}
```

```text
PortfolioAccount {
    owner
    instance_id
    market_group_id
    config_hash_at_open

    C_i
    PNL_i
    R_i                         // live released positive PnL face
    fee_credits_i <= 0 and != i128::MIN

    active_bitmap
    legs[0..N)
    account_claim_bound_contributions
    source_credit_lien_keys[0..bounded]

    health_cert
    stale_state
    positive_credit_lock
    rebalance_lock
    liquidation_lock
    cancel_deposit_escrow
    portfolio_close_state optional
}
```

Each account has at most one canonical signed net leg per asset. Same-asset opposite exposure MUST net into that leg.

-------------------------------------------------------------------------------
5. Global invariants
-------------------------------------------------------------------------------

```text
C_tot <= V <= MAX_VAULT_TVL
I <= V
V >= C_tot + I
0 <= neg_pnl_account_count <= materialized_account_count <= cfg_account_index_capacity <= MAX_MATERIALIZED_ACCOUNTS
0 <= rr_cursor_position < cfg_account_index_capacity
slot_last <= current_slot
F_long_num and F_short_num fit i128
if Live: PNL_matured_pos_tot <= PNL_pos_tot <= MAX_PNL_POS_TOT_LIVE and resolved fields are zero
if Resolved: resolved_price > 0, resolved_live_price > 0, PNL_matured_pos_tot <= PNL_pos_tot
if snapshot not ready: resolved_payout_h_num = resolved_payout_h_den = 0
if snapshot ready: resolved_payout_h_num <= resolved_payout_h_den
```

### 2.3 Account materialization and freeing

Every external index MUST satisfy `i < cfg_account_index_capacity`. Missing/materialized status MUST come from authenticated engine state; omitted account data is not proof of missingness.

Only `deposit(i, amount > 0, now_slot)` may materialize a missing account. `materialize_account(i, materialize_slot)` initializes all fields to zero/canonical defaults, sets `last_fee_slot_i = materialize_slot`, and increments `materialized_account_count`.

`free_empty_account_slot(i)` is the only canonical free path. Preconditions:

```text
account materialized
C_i = 0, PNL_i = 0, R_i = 0
both buckets absent
basis_pos_q_i = 0
fee_credits_i <= 0
```

Effects: forgive fee debt by setting `fee_credits_i = 0`, reset local fields to canonical zero-position defaults, clear reserves and wrapper annotations, set `last_fee_slot_i = 0`, mark the slot missing/reusable in authenticated state, and decrement `materialized_account_count`. `neg_pnl_account_count` is unchanged.

### 2.4 Side reset lifecycle

For every materialized account with nonzero basis on side `s`, exactly one holds:

```text
epoch_snap_i == epoch_s
or mode_s == ResetPending and epoch_snap_i + 1 == epoch_s
```

`begin_full_drain_reset(side)` requires `OI_eff_side == 0` and then snapshots `K_side`/`F_side_num` to epoch-start fields, zeros live `K_side`/`F_side_num`, increments `epoch_side`, sets `A_side = ADL_ONE`, sets `stale_account_count_side = stored_pos_count_side`, clears phantom dust for that side, and enters `ResetPending`.

When a side reset or leg clear combines two valid sub-atom social-loss carries,
the sum is normalized modulo `SOCIAL_LOSS_DEN`. Crossing the denominator adds
one to the side-local `explicit_unallocated_loss` audit counter; that counter
saturates and never creates payout capacity. Remainder, dust, and explicit-loss
audit fields remain durable while the asset has any live economic obligation.
An otherwise empty asset may clear only those inert fields inside the single
retirement transition so historical rounding cannot permanently block terminal
progress; callers do not need a preparatory cleanup transition.
After every source claim, provider receivable, backing amount, lien, and insurance
reservation is zero, cumulative `spent_backing_num` is likewise audit-only and is
cleared atomically with retirement. A nonzero provider receivable or consumed backing
remains a hard retirement blocker.
After every position, effective-OI atom, pending obligation, and social-loss weight is
also zero, historical K/F indices and their prior-epoch baselines have no remaining
claimant and are audit-only. Retirement clears them atomically. Restart performs the
same terminal normalization, including spent-only domain budgets and source/social
audit, before assigning the fresh market generation; any live obligation or nonzero
remaining insurance budget rejects the complete restart transition.

`finalize_side_reset(side)` requires `ResetPending`, zero OI, zero stale count, and zero stored position count, then sets mode to `Normal`.

Before any OI-increasing operation rejects on `ResetPending`, it MUST call `maybe_finalize_ready_reset_sides_before_oi_increase`.

---

## 3. Claims, haircuts, and equity

Let:

```text
Residual = V - (C_tot + I)   // checked, and invariant guarantees nonnegative
PosPNL_i = max(PNL_i, 0)
ReleasedPos_i = PosPNL_i - R_i on Live
ReleasedPos_i = PosPNL_i on Resolved
PendingWarmupTot = PNL_pos_tot - PNL_matured_pos_tot = sum R_i on Live
```

Canonical haircut pairs:

```text
if PNL_matured_pos_tot == 0: h = (1, 1)
else h = (min(Residual, PNL_matured_pos_tot), PNL_matured_pos_tot)

if PNL_pos_tot == 0: g = (1, 1)
else g = (min(Residual, PNL_pos_tot), PNL_pos_tot)
```

Then:

```text
PNL_eff_matured_i = floor(ReleasedPos_i * h.num / h.den)
PNL_eff_trade_i   = floor(PosPNL_i     * g.num / g.den)
```

Equity lanes, all exact wide signed:

```text
Eq_withdraw_raw_i = C_i + min(PNL_i,0) + PNL_eff_matured_i - FeeDebt_i
Eq_trade_raw_i    = C_i + min(PNL_i,0) + PNL_eff_trade_i   - FeeDebt_i
Eq_maint_raw_i    = C_i + PNL_i                            - FeeDebt_i
Eq_net_i          = max(0, Eq_maint_raw_i)
```

Candidate trade approval MUST neutralize that trade’s own positive slippage:

```text
TradeGain_i_candidate = max(candidate_trade_pnl_i, 0)
PNL_trade_open_i      = PNL_i - TradeGain_i_candidate
PosPNL_trade_open_i   = max(PNL_trade_open_i, 0)
PNL_pos_tot_trade_open_i = PNL_pos_tot - PosPNL_i + PosPNL_trade_open_i
compute g_open from PNL_pos_tot_trade_open_i and Residual
Eq_trade_open_raw_i = C_i + min(PNL_trade_open_i,0) + floor(PosPNL_trade_open_i*g_open.num/g_open.den) - FeeDebt_i
```

`Eq_trade_open_raw_i` is the only compliant risk-increasing trade approval metric.

---

## 4. Reserve, PnL, fee, and insurance helpers

### 4.1 Capital and position setters

`set_capital(i, new_C)` updates `C_tot` by the exact signed delta, then writes `C_i`.

`set_position_basis_q(i, new_basis)` updates long/short stored position counts exactly once according to old/new sign flags, enforcing `cfg_max_active_positions_per_side` on any increment, then writes `basis_pos_q_i`. All position-zeroing settlement branches MUST use this helper or an exactly equivalent path.

### 4.2 Reserve bucket operations

`promote_pending_to_scheduled(i)` does nothing if scheduled exists or pending absent. Otherwise it creates a scheduled bucket from pending with `sched_start_slot = current_slot`, `sched_anchor_q = sched_remaining_q = pending_remaining_q`, `sched_horizon = pending_horizon`, `sched_release_q = 0`, and clears pending. It MUST NOT change `R_i`.

`append_new_reserve(i, reserve_add, admitted_h_eff)` requires positive amount and positive horizon. If no scheduled bucket exists but pending exists, first promote pending. Then:

1. if scheduled absent, create scheduled at `current_slot`;
2. else if pending absent and `sched_start_slot == current_slot`, `sched_horizon == admitted_h_eff`, and `sched_release_q == 0`, merge into scheduled;
3. else if pending absent, create pending;
4. else merge into pending and set `pending_horizon = max(pending_horizon, admitted_h_eff)`.

Finally increase `R_i` by `reserve_add`.

`apply_reserve_loss_newest_first(i, reserve_loss)` consumes pending before scheduled, decrements `R_i`, and clears empty buckets.

`advance_profit_warmup(i)` promotes pending if needed, computes:

```text
elapsed = current_slot - sched_start_slot
effective_elapsed = min(elapsed, sched_horizon)
sched_total = floor(sched_anchor_q * effective_elapsed / sched_horizon)
sched_increment = sched_total - sched_release_q
release = min(sched_remaining_q, sched_increment)
```

It releases `release` to `PNL_matured_pos_tot`. If the scheduled bucket empties, it is cleared completely including `sched_release_q = 0`, and pending is promoted if present. A non-empty bucket MUST NOT persist with an over-advanced release cursor.

### 4.3 Admission

`admit_fresh_reserve_h_lock(i, fresh_positive_pnl_i, ctx, admit_h_min, admit_h_max) -> admitted_h_eff` requires a live materialized account and valid admission pair. Let:

```text
Residual_now = V - (C_tot + I)
matured_plus_fresh = PNL_matured_pos_tot + fresh_positive_pnl_i
threshold_opt = ctx.admit_h_max_consumption_threshold_bps_opt_shared
```

Law:

1. if `i` is in `ctx.h_max_sticky_accounts`, return `admit_h_max`;
2. if `threshold_opt = Some(threshold_bps)`, compute `threshold_e9 = threshold_bps * PRICE_MOVE_CONSUMPTION_SCALE`; if `price_move_consumed_bps_e9_this_generation >= threshold_e9`, choose `admit_h_max`;
3. otherwise choose `admit_h_min` iff `matured_plus_fresh <= Residual_now`, else `admit_h_max`;
4. if `admit_h_max` was chosen, insert `i` into the sticky set.

`None` disables the stress gate. `Some(0)` is invalid. The engine enforces only the supplied policy; public-wrapper nonzero-warmup requirements are wrapper obligations.

`admit_outstanding_reserve_on_touch(i, ctx)` accelerates all outstanding reserve only when all hold:

```text
reserve_total > 0
ctx.admit_h_min_shared == 0
stress threshold is absent or inactive
PNL_matured_pos_tot + reserve_total <= Residual_now
```

If so it moves the entire reserve into `PNL_matured_pos_tot`, clears both buckets, and sets `R_i = 0`. Otherwise it leaves reserve unchanged. It never extends or resets a horizon.

### 4.4 PnL mutation

Every persistent `PNL_i` mutation after materialization MUST use `set_pnl`, except `consume_released_pnl`.

`set_pnl(i, new_PNL, reserve_mode[, ctx])` where reserve mode is:

```text
UseAdmissionPair(admit_h_min, admit_h_max)
ImmediateReleaseResolvedOnly
NoPositiveIncreaseAllowed
```

It updates `PNL_pos_tot`, `PNL_matured_pos_tot`, `R_i`, reserve buckets, and `neg_pnl_account_count` atomically.

For positive increases:

- `NoPositiveIncreaseAllowed` fails;
- `ImmediateReleaseResolvedOnly` requires `Resolved`, increases `PNL_matured_pos_tot`, and does not reserve;
- `UseAdmissionPair` requires `Live`, obtains `admitted_h_eff`, immediately matures iff `admitted_h_eff == 0`, otherwise appends reserve.

For non-increases it consumes reserve loss newest-first, then matured loss, updates aggregates and sign count, and requires no reserve remains when live positive PnL becomes zero.

`consume_released_pnl(i, x)` requires live `0 < x <= ReleasedPos_i`, decreases `PNL_i`, `PNL_pos_tot`, and `PNL_matured_pos_tot` by `x`, and leaves reserve unchanged.

### 4.5 Fees

Trading fee:

```text
fee = 0 if cfg_trading_fee_bps == 0 or trade_notional == 0
else ceil(trade_notional * cfg_trading_fee_bps / 10_000)
```

Liquidation fee for `q_close_q`:

```text
if q_close_q == 0: liq_fee = 0
else:
  closed_notional = floor(q_close_q * oracle_price / POS_SCALE)
  liq_fee_raw = ceil(closed_notional * cfg_liquidation_fee_bps / 10_000)
  liq_fee = min(max(liq_fee_raw, cfg_min_liquidation_abs), cfg_liquidation_fee_cap)
```

`charge_fee_to_insurance(i, fee_abs)` requires `fee_abs <= MAX_PROTOCOL_FEE_ABS`. It computes collectible headroom from capital plus fee-credit headroom, pays as much as possible from `C_i` into `I`, records any collectible shortfall as negative `fee_credits_i`, and drops the uncollectible tail. It MUST NOT mutate PnL, reserves, positive-PnL aggregates, or K/F indices.

`sync_account_fee_to_slot(i, anchor, rate)` charges recurring wrapper-owned fees exactly once over `[last_fee_slot_i, anchor]`, caps `rate * dt` at `MAX_PROTOCOL_FEE_ABS` without failing on raw-product overflow, routes the capped amount through `charge_fee_to_insurance`, and advances `last_fee_slot_i = anchor`. Live anchors must be `<= current_slot`; resolved anchors must be `<= resolved_slot`.

`fee_debt_sweep(i)` pays fee debt from available `C_i` into `I`. This preserves `Residual` because it is a pure `C -> I` reclassification.

### 4.6 Insurance loss

`use_insurance_buffer(loss_abs)` MUST spend exactly `pay = min(loss_abs, I)`, set `I -= pay`, and return `loss_abs - pay`. It MUST NOT drain the full insurance fund when the loss is smaller.

`record_uninsured_protocol_loss(loss_abs)` may record telemetry but MUST NOT inflate `D`, `C_tot`, `PNL_pos_tot`, `PNL_matured_pos_tot`, `V`, or `I`. The loss remains represented by junior haircuts.

`absorb_protocol_loss(loss_abs)` calls `use_insurance_buffer` and records only the returned nonzero remainder.

---

## 5. A/K/F, accrual, ADL, and resets

### 5.1 Effective position

For account `i` with nonzero basis on side `s`:

```text
if epoch_snap_i != epoch_s: effective_pos_q(i) = 0
else effective_abs_pos_q = floor(abs(basis_pos_q_i) * A_s / a_basis_i)
effective_pos_q = sign(basis_pos_q_i) * effective_abs_pos_q
```

The exact bilateral trade OI after-values are:

```text
OI_long_after  = OI_eff_long  - old_long_a  - old_long_b  + new_long_a  + new_long_b
OI_short_after = OI_eff_short - old_short_a - old_short_b + new_short_a + new_short_b
```

They MUST be used for both gating and writeback.

### 5.2 Settlement of side effects

Live touch settlement:

1. if basis is zero, return;
2. require `a_basis_i > 0` and compute `den = a_basis_i * POS_SCALE` exactly;
3. if current epoch, compute effective quantity and `pnl_delta` with `wide_signed_mul_div_floor_from_kf_pair(abs_basis, k_snap, K_s, f_snap, F_s_num, den)`;
4. apply `set_pnl(..., UseAdmissionPair(ctx...))`;
5. if effective quantity floors to zero, increment the side phantom-dust bound by exactly one q-unit, clear basis through `set_position_basis_q(i,0)`, and reset snapshots; otherwise update snapshots.

Epoch-mismatch settlement requires `mode_s == ResetPending`, `epoch_snap_i + 1 == epoch_s`, and positive stale count. It settles against `K_epoch_start_s` / `F_epoch_start_s_num`, applies PnL through admission, clears basis through `set_position_basis_q(i,0)`, decrements stale count, and resets snapshots.

Resolved settlement first calls `prepare_account_for_resolved_touch`, then settles stale one-epoch-lag basis against:

```text
k_terminal_s_exact = K_epoch_start_s + resolved_k_terminal_delta_s
f_terminal_s_exact = F_epoch_start_s_num
```

using `ImmediateReleaseResolvedOnly`, then clears basis through `set_position_basis_q` and decrements stale count.

### 5.3 Accrual

`accrue_market_to(now_slot, oracle_price, funding_rate_e9_per_slot)` requires live mode, trusted `now_slot >= slot_last`, valid oracle price, and funding-rate magnitude within config.

Let:

```text
dt = now_slot - slot_last
funding_active = funding_rate != 0 && OI_eff_long != 0 && OI_eff_short != 0 && fund_px_last > 0
price_move_active = P_last > 0 && oracle_price != P_last && (OI_eff_long != 0 || OI_eff_short != 0)
```

If either active branch is true, require `dt <= cfg_max_accrual_dt_slots`.

If `price_move_active`, before mutating any K/F/price/slot/consumption state, require exactly:

```text
abs(oracle_price - P_last) * 10_000 <= cfg_max_price_move_bps_per_slot * dt * P_last
```

Then update stress consumption:

```text
consumed = floor(abs_delta_price * 10_000 * PRICE_MOVE_CONSUMPTION_SCALE / P_last)
price_move_consumed_bps_e9_this_generation = saturating_add(price_move_consumed_bps_e9_this_generation, consumed)
```

The accumulator is a stress signal, not a conservation quantity; overflow MUST saturate at `u128::MAX` and force slow-lane admission for finite thresholds until generation reset.

Mark-to-market once:

```text
ΔP = oracle_price - P_last
if OI_long_0  > 0: K_long  += A_long  * ΔP
if OI_short_0 > 0: K_short -= A_short * ΔP
```

Funding, if active, uses one exact total:

```text
fund_num_total = fund_px_last * funding_rate_e9_per_slot * dt
F_long_num  -= A_long  * fund_num_total
F_short_num += A_short * fund_num_total
```

Persistent K/F overflow fails conservatively. Finally set `slot_last = now_slot`, `P_last = oracle_price`, and `fund_px_last = oracle_price`.

### 5.4 ADL / bankrupt liquidation socialization

`enqueue_adl(ctx, liq_side, q_close_q, D)`:

1. decrements liquidated-side OI by `q_close_q`;
2. spends insurance exactly with `use_insurance_buffer(D)`;
3. if opposing OI is zero, records any remainder as uninsured and schedules reset if both sides zero;
4. if opposing stored position count is zero, reduces opposing OI by `q_close_q`, records remainder, and schedules reset if both sides zero;
5. otherwise computes opposing quantity decay and optional K loss.

For `D_rem > 0`, compute:

```text
delta_K_abs = ceil(D_rem * A_old * POS_SCALE / OI_before)
delta_K_exact = -delta_K_abs
```

If representability, `K_opp + delta_K_exact`, or future mark headroom `|K_candidate| + A_old * MAX_ORACLE_PRICE <= i128::MAX` fails, route `D_rem` to uninsured loss while still continuing quantity socialization.

Then:

```text
OI_post = OI_before - q_close_q
A_candidate = floor(A_old * OI_post / OI_before)
```

If `OI_post == 0`, zero opposing OI and schedule reset. If `A_candidate > 0`, set `A_opp`, set `OI_eff_opp`, add the exact ADL dust bound, and enter `DrainOnly` if `A_opp < MIN_A_SIDE`. If `A_candidate == 0` while `OI_post > 0`, zero both OI sides and schedule both resets.

### 5.5 End-of-instruction reset scheduling

At the end of every top-level instruction that can touch accounts, mutate side state, liquidate, or resolved-close, call `schedule_end_of_instruction_resets(ctx)` exactly once, except for the additional explicit pre-open dust/reset flush inside `execute_trade`.

If both stored side counts are zero, compute `clear_bound = checked_add(phantom_dust_bound_long_q, phantom_dust_bound_short_q)`. If residual OI or dust exists, require OI symmetry and clear both OI sides only if both are within `clear_bound`; otherwise fail conservatively.

If exactly one stored side is zero, require OI symmetry and clear both sides only if the empty side’s OI is within that side’s phantom-dust bound; otherwise fail conservatively.

If a side is `DrainOnly` and its OI is zero, set that side’s pending reset flag.

`finalize_end_of_instruction_resets(ctx)` begins pending resets and finalizes any ready `ResetPending` side.

---

## 6. Live local touch and finalization

`touch_account_live_local(i, ctx)`:

1. requires live materialized account;
2. adds `i` to `ctx.touched_accounts` or fails on capacity;
3. calls `admit_outstanding_reserve_on_touch(i, ctx)`;
4. advances warmup;
5. settles A/K/F side effects;
6. settles negative PnL from principal;
7. if now authoritative flat and still negative, calls `absorb_protocol_loss` and sets PnL to zero;
8. MUST NOT auto-convert or sweep fee debt.

`finalize_touched_accounts_post_live(ctx)` computes one shared whole-haircut snapshot after all live local work. It then iterates touched accounts in ascending storage-index order. If an account is flat, has released positive PnL, and the snapshot has `h = 1`, it uses `consume_released_pnl` followed by `set_capital(C_i + released)`. It then calls `fee_debt_sweep`.

---

## 7. Margin and liquidation

After authoritative live touch:

```text
RiskNotional_i = 0 if effective_pos_q(i) == 0
else ceil(abs(effective_pos_q(i)) * oracle_price / POS_SCALE)

MM_req_i = 0 if flat else max(floor(RiskNotional_i * cfg_maintenance_bps / 10_000), cfg_min_nonzero_mm_req)
IM_req_i = 0 if flat else max(floor(RiskNotional_i * cfg_initial_bps / 10_000), cfg_min_nonzero_im_req)
```

Maintenance healthy iff `Eq_net_i > MM_req_i`. Withdrawal healthy iff `Eq_withdraw_raw_i >= IM_req_i`. Risk-increasing trade approval healthy iff `Eq_trade_open_raw_i >= IM_req_post_i`.

A trade is risk-increasing if it increases absolute effective position, flips sign, or opens from flat. It is strictly risk-reducing if same sign, nonzero before/after, and absolute position decreases.

An account is liquidatable iff after full authoritative live touch it has nonzero effective position and `Eq_net_i <= MM_req_i`. If recurring fees are enabled, the account MUST be fee-current first.

Partial liquidation requires `0 < q_close_q < abs(old_eff_pos_q_i)`. It closes synthetically at oracle price, attaches the remaining position, settles losses from principal, charges liquidation fee, invokes `enqueue_adl(ctx, liq_side, q_close_q, 0)`, and requires the remaining nonzero position to be maintenance healthy after the step.

Full-close liquidation closes the whole effective position at oracle price, attaches flat, settles losses from principal, charges liquidation fee, sets `D = max(-PNL_i, 0)`, invokes `enqueue_adl` if `q_close_q > 0 || D > 0`, then sets negative PnL to zero with `NoPositiveIncreaseAllowed` if `D > 0`.

---

## 8. External operations

### 8.1 Standard live lifecycle

Live instructions that depend on current market state execute:

1. validate slots, effective oracle price, funding-rate bound, admission pair, optional threshold (`None` disables; `Some(t)` requires `0 < t <= floor(u128::MAX / PRICE_MOVE_CONSUMPTION_SCALE)`), and endpoint inputs;
2. initialize fresh `ctx`;
3. call `accrue_market_to` exactly once;
4. set `current_slot = now_slot`;
5. sync recurring fees for touched accounts before health-sensitive checks;
6. run endpoint logic;
7. call `finalize_touched_accounts_post_live(ctx)` exactly once if live local touches were used;
8. schedule and finalize resets exactly once;
9. assert OI symmetry for side-mutating/live-exposure instructions;
10. require `V >= C_tot + I`.

Any early no-op return after state mutation or fee sync MUST still perform the final applicable invariant checks.

### 8.2 No-accrual public path guard

Pure public live paths that advance `current_slot` without calling `accrue_market_to` MUST call:

```text
require_no_accrual_public_path_within_envelope(now_slot):
  require market_mode == Live
  require now_slot >= current_slot
  require slot_last <= current_slot
  if OI_eff_long == 0 && OI_eff_short == 0: return
  dt = now_slot - slot_last    // checked subtraction
  require dt <= cfg_max_accrual_dt_slots
```

This avoids overflow-prone `slot_last + cfg_max_accrual_dt_slots` arithmetic and permits zero-OI idle fast-forward.

### 8.3 Pure capital / fee operations

`deposit(i, amount, now_slot)` is live-only, no-accrual, and may materialize missing `i` only if `amount > 0`. It increases `V`, increases `C_i`, settles realized losses from principal, MUST NOT absorb flat negative loss through insurance, and sweeps fee debt only if the account is flat and nonnegative.

`deposit_fee_credits(i, amount, now_slot)` pays `min(amount, FeeDebt_i)` into `V` and `I`, increases `fee_credits_i` by that amount, and never makes fee credits positive.

`top_up_insurance_fund(amount, now_slot)` increases `V` and `I` by `amount`.

`charge_account_fee(i, fee_abs, now_slot)` routes `fee_abs` through `charge_fee_to_insurance` and performs no margin check by itself.

`settle_flat_negative_pnl(i, now_slot[, fee_rate])` is live-only, no-accrual, requires flat account with no reserve, syncs fee if enabled, settles losses from principal, then absorbs any remaining negative PnL through insurance/uninsured loss and sets PnL to zero.

`reclaim_empty_account(i, now_slot[, fee_rate])` is live-only, no-accrual, syncs fees if enabled, then requires the §2.3 free-slot preconditions and calls `free_empty_account_slot`.

### 8.4 User value-moving current-state operations

`settle_account` runs the standard live lifecycle, touches one account, and finalizes.

`withdraw` touches and finalizes first. It then requires `amount <= C_i`; if the account is nonflat, it requires withdrawal health under the hypothetical state where both `V` and `C_tot` decrease by `amount`; then it pays out by decreasing `C_i` and `V`.

`convert_released_pnl` touches first, requires `0 < x_req <= ReleasedPos_i`, computes current `h`, and for flat accounts requires `x_req <= max_safe_flat_conversion_released`. It consumes released PnL, adds `floor(x_req * h.num / h.den)` to capital, sweeps fee debt, and if still nonflat requires maintenance health.

`close_account` touches and finalizes first. It requires flat, zero PnL, no reserve, and no fee debt, pays out all capital by decreasing `C_i` and `V`, then calls `free_empty_account_slot`.

### 8.5 Trade

`execute_trade(a,b, ..., size_q, exec_price)` requires distinct materialized accounts, valid execution price, positive size, computed `trade_notional <= MAX_ACCOUNT_NOTIONAL`, and standard live lifecycle.

It syncs fees if enabled, touches both accounts in deterministic ascending storage-index order, then runs a pre-open dust/reset flush using a separate reset-only context. It captures pre-trade positions and maintenance state, finalizes ready reset sides, computes candidate positions and exact bilateral OI after-values, enforces position/OI bounds and side-mode gating, applies execution-slippage PnL before fees, attaches positions, writes OI after-values, settles losses, charges trade fees, computes post-trade risk notional and approval metrics, and approves each account independently:

- flat result: fee-neutral negative-shortfall comparison must not worsen;
- risk-increasing: require `Eq_trade_open_raw_i >= IM_req_post_i`;
- already maintenance healthy: allow;
- strictly risk-reducing while unhealthy: allow only if fee-neutral maintenance shortfall strictly improves and fee-neutral negative equity does not worsen;
- otherwise reject.

### 8.6 Liquidate

`liquidate(i, ..., policy)` runs standard live lifecycle, syncs fees if enabled, touches the account, requires liquidation eligibility, executes `FullClose` or `ExactPartial(q_close_q)`, finalizes, schedules/finalizes resets, and checks conservation.

### 8.7 Keeper crank

`keeper_crank(now_slot, oracle_price, funding_rate, admit_h_min, admit_h_max, threshold_opt, ordered_candidates[], max_revalidations, rr_window_size[, fee_fn])` is live-only and accrues exactly once before both phases.

Phase 1 processes keeper-supplied candidates in supplied order until `max_revalidations` is exhausted or a pending reset is scheduled. Authenticated missing-account skips do not count. If a candidate slot is materialized, its account state MUST be available; omission/unreadability fails conservatively. Liquidation is Phase 1 only.

Phase 2 always runs, even if Phase 1 stopped on pending reset. It does not count against `max_revalidations`, does not liquidate, and does not stop on pending reset. Let:

```text
sweep_limit = cfg_account_index_capacity
remaining = sweep_limit - rr_cursor_position
rr_advance = min(rr_window_size, remaining)
sweep_end = rr_cursor_position + rr_advance
```

For each index in `[rr_cursor_position, sweep_end)`, skip only if authenticated engine state proves missing; otherwise require account data and call `touch_account_live_local`. Then set `rr_cursor_position = sweep_end`. If it reaches `sweep_limit`, wrap to `0`, increment `sweep_generation`, and reset `price_move_consumed_bps_e9_this_generation = 0` atomically.

### 8.8 Resolution and resolved close

`resolve_market(resolve_mode, resolved_price, live_oracle_price, now_slot, funding_rate)` is privileged. Branch selection is explicit; value-detected branch selection is forbidden.

Ordinary branch calls `accrue_market_to(now_slot, live_oracle_price, funding_rate)`, sets `current_slot`, and requires the resolved price to be inside the configured deviation band around the trusted live-sync price. On this branch, `live_oracle_price` is the effective live-sync price supplied to the engine; if the raw external target is beyond the live cap, feeding it directly will fail and the wrapper must first catch up through valid capped accruals or choose an explicit recovery path.

Degenerate branch requires `live_oracle_price == P_last` and `funding_rate == 0`, sets `current_slot = slot_last = now_slot`, uses `P_last` as the resolved live price, and skips the ordinary band. It is a privileged recovery path only.

Both branches compute terminal K deltas exactly, store them separately from live K, enter `Resolved`, set `resolved_slot`, clear payout snapshot state, set `PNL_matured_pos_tot = PNL_pos_tot`, zero both OI sides, begin/finalize side resets as applicable, and require conservation.

`force_close_resolved(i)` is permissionless and takes no caller slot. It requires `current_slot == resolved_slot`, prepares the account for resolved touch, settles resolved side effects, settles/absorbs losses, finalizes ready reset sides, then:

- if `PNL_i == 0`, fee-sweeps, forgives remaining fee debt, pays out capital, and frees the slot;
- if `PNL_i > 0` and the market is not positive-payout ready, returns `ProgressOnly`;
- if positive-payout ready, captures the shared payout snapshot if needed, pays `floor(PNL_i * snapshot_num / snapshot_den)`, fee-sweeps, pays out capital, and frees the slot.

A zero payout MUST NOT be the only encoding of progress-only.

---

## 9. Wrapper obligations

1. Public wrappers MUST NOT expose arbitrary caller-controlled `admit_h_min`, `admit_h_max`, threshold, or funding-rate inputs.
2. Public or permissionless wrappers with untrusted live oracle or execution-price PnL MUST use `admit_h_min > 0` for instructions that can create or accelerate live positive PnL. `admit_h_min = 0` is reserved for trusted/private immediate-release deployments.
3. Stress threshold gating is optional engine machinery. It is a reconciliation/UX stress signal, not a substitute for warmup.
4. Resolution is privileged. Wrappers MUST source trusted live and settlement prices, funding rate, and explicit `resolve_mode`.
5. Wrappers MUST monitor accrual envelopes and K/F headroom, and crank or resolve before exposed markets exceed live envelopes.
6. Public wrappers MUST separate raw oracle target state from effective engine price state and MUST feed capped staircase prices, not cap-violating raw jumps, into exposed live accrual. Same-slot exposed cranks MUST pass the unchanged engine price. If exposed catch-up would have `target != P_last`, `dt > 0`, and `max_delta == 0`, the wrapper MUST enter recovery or wait for enough elapsed slots; it MUST NOT advance `slot_last` with the unchanged price as a silent bypass.
7. While raw target and effective engine price differ, public wrappers MUST reject or conservatively shadow-check extraction-sensitive user actions (`withdraw`, `convert_released_pnl`, user-triggered settlement/finalization that can release or convert positive PnL, and any close path whose payout depends on lagged PnL) and MUST reject risk-increasing user trades unless a stricter dual-price policy prices and margin-checks the trade against the lag.
8. Public wrappers using the sweep-generation stress gate MUST pass nonzero `rr_window_size` on normal keeper cranks and ensure `max_revalidations + rr_window_size` fits touched-account capacity and compute budget. `rr_window_size = 0` is reserved for trusted/private compatibility or explicit recovery flows.
9. Public wrappers SHOULD enforce execution-price admissibility, e.g. bounded deviation from effective engine price and, during oracle catch-up lag, from the raw target as well.
10. User value-moving operations must be account-authorized. Intended permissionless paths are settlement, liquidation, reclaim, flat-negative cleanup, resolved close, and keeper crank.
11. If recurring fees are enabled, wrappers MUST sync fee-current state before health-sensitive checks, reclaim checks, and resolved terminal close, and MUST use `resolved_slot` on resolved markets.
12. Wrappers own account-materialization anti-spam economics: minimum deposit, recurring fees, and reclaim incentives.
13. Runtime configuration MUST bound `max_revalidations + rr_window_size` to fit actual context capacity and compute budget.
---

## 10. Required test coverage

Implementations and public wrappers MUST test at least:

1. conservation `V >= C_tot + I` across all paths;
2. PnL aggregate and `neg_pnl_account_count` consistency;
3. reserve admission, sticky `admit_h_max`, pending/scheduled behavior, reserve loss ordering, and no stale release cursor;
4. public-wrapper policy tests that `admit_h_min = 0` is not used for untrusted public live PnL;
5. outstanding reserve acceleration blocked by nonzero `admit_h_min` or active threshold;
6. exact candidate-trade positive-slippage neutralization;
7. fee-debt sweep residual neutrality and actual-fee-impact comparisons;
8. `RiskNotional` ceil margin including fractional-notional dust;
9. exact per-risk-notional init envelope including funding fractions, post-move liquidation notional, fee floor, fee cap, and rounded notionals;
10. price-move cap rejection before any K/F/price/slot/consumption mutation;
11. wrapper oracle catch-up clamp: raw target is stored separately, next effective price moves toward target by at most `floor(P_last * cap * dt / 10_000)`, and same-slot exposed cranks pass `P_last`;
12. target/effective-price divergence policy: public risk-increasing trades and extraction-sensitive actions are rejected or pass a stricter dual-price shadow check;
13. zero-OI no-accrual fast-forward and exposed-market no-accrual envelope rejection using checked subtraction near `u64::MAX`;
14. exact insurance spending `min(loss_abs, I)`;
15. stress accumulator floor-at-scaled-bps precision, saturating addition, threshold activation, and reset only on generation advance;
16. deterministic Phase 2 cursor arithmetic over `cfg_account_index_capacity`, authenticated missing-slot skips, and failure on omitted materialized account data;
17. public keeper wrappers using the stress gate pass nonzero `rr_window_size` on normal cranks and enforce touched-account budget;
18. deterministic ascending trade touch order and pre-open dust/reset flush;
19. all position zeroing through `set_position_basis_q` and all frees through `free_empty_account_slot`;
20. resolved payout readiness, shared snapshot stability, and explicit progress-vs-close outcome;
21. degenerate resolution requires explicit mode and exact degenerate inputs; ordinary resolution never value-detects into degenerate mode;
22. ADL exact K deficit computation, overflow fallback to uninsured loss while quantity socialization continues, and phantom-dust clearance bounds;
23. self-neutral insurance/oracle-siphon scenarios across multiple valid accrual envelopes;
24. exposed `target != P_last`, `dt > 0`, `max_delta == 0` cannot advance `slot_last` by feeding `P_last`; it must wait, reject as catch-up-required, or enter explicit recovery;
25. raw target jumps beyond the cap are never fed directly to exposed live engine accrual except in an explicit recovery/resolution test that confirms conservative failure or privileged recovery semantics.
