# Percolator

**EXPERIMENTAL RESEARCH PROJECT — NOT AUDITED. Do NOT use with real funds. This is experimental software provided for learning and research purposes only. Use at your own risk.**

Risk engine library for permissionless perpetual futures on Solana.

Current normative spec: [`spec.md`](spec.md), **v16.8.3**.

A predictable perpetual-futures risk engine built around backed exits, lazy
overhang clearing, and bounded cranks.

If you want the `xy = k` of perpetual futures risk engines -- something you can reason about, audit, and run without human intervention -- the cleanest move is simple: stop treating profit like money. Treat it like what it really is in a stressed exchange: a junior claim on a shared balance sheet.

> No user can ever withdraw more value than actually exists on the exchange balance sheet.

## Three Invariants

A stressed perp exchange has three jobs:

1. **Backed exits:** when the vault is stressed, nobody can extract more value than the balance sheet can pay.
2. **Fair overhang clearing:** when positions go bankrupt, the residual is absorbed pro rata instead of by a discretionary ADL queue.
3. **Bounded cranks:** when the oracle moves, the live book is repriced only inside the configured one-step risk budget.

Percolator composes three mechanisms:

- **H** (the haircut ratio) makes positive PnL a junior claim on residual value.
- **A/K/F** (lazy side indices) settles mark moves, funding, and ADL overhang without selecting individual losers.
- **The price/funding envelope** bounds every exposed accrual step before K/F/price/slot state can mutate.

---

## H: Backed Exits

Capital is senior. Profit is junior. A single global ratio determines how much
released positive PnL is actually backed.

```
Residual  = max(0, V - C_tot - I - E - F)

              min(Residual, PNL_matured_pos_tot)
    h     =  ----------------------------------
                    PNL_matured_pos_tot
```

`Residual` is what is left for junior claims after **every** senior claim, and
there are four of them, not two:

| term | what it is | why it is senior |
|---|---|---|
| `C_tot` | deposited capital | principal is never junior to profit |
| `I` | insurance | reserved against future losses |
| `E` | `backing_provider_earnings_total` | utilization fees already owed to LPs |
| `F` | `source_fresh_backing_total` | backing earmarked to a specific claim |

`E` and `F` are easy to overlook and both are load-bearing. Omitting either
over-states the junior pool and promises winners atoms that someone else can
already withdraw — see the Kani proofs
`proof_v16_residual_excludes_senior_backing_provider_earnings` and
`proof_v16_residual_excludes_recoverable_counterparty_backing_principal`.

### Two ways a junior claim gets paid

`h` above governs **unsecured** junior claims — those with no dedicated
backing. A claim that *is* source-backed is paid from its own backing instead,
and does not compete for `Residual`:

```
account has source claims   ->  paid from that claim's backing (backing-limited)
otherwise                   ->  paid from Residual, pro-rata via h
```

That is a seniority *structure*, not an exemption, and `F` is what keeps the two
consistent: atoms earmarked to a backed claim are removed from `Residual`, so the
same atom is never promised to both a backed and an unsecured claimant.

When a losing account's principal is consumed it becomes exactly this kind of
earmarked backing for its counterparty — capital and `c_tot` fall by precisely
the amount that appears as backing, and the vault does not move. Value is
reshaped, never created (`proof_v16_capital_backed_loss_reservation_is_value_neutral_and_capital_capped`).

If fully backed, `h = 1`. If stressed, `h < 1`. Every profitable account sees
the same fraction of its *released* positive PnL:

```
ReleasedPos_i   = max(PNL_i, 0) - R_i
effective_pnl_i = floor(ReleasedPos_i * h)
```

Fresh profit sits in a per-account reserve `R_i` and converts to released
(matured) profit through admission and warmup. Only admitted matured profit
enters the haircut denominator (`PNL_matured_pos_tot`) and per-account effective
PnL.

This is the core anti-oracle-manipulation defense. An attacker who spikes a
price sees live gain locked in reserve, excluded from both the ratio and their
withdrawable amount, until the instruction policy admits it. Public wrappers
using untrusted live oracle or execution-price PnL must use nonzero admission
warmup; stress-threshold gating is not a substitute.

No rankings, no queue priority, no first-come advantage. The floor rounding is conservative — the sum of all effective PnL never exceeds what exists in the vault.

When the system is stressed, `h` falls and less profit converts. When losses
settle or buffers recover, `h` rises. Self-healing.

Flat accounts are always protected — `h` only gates profit extraction, never touches deposited capital.

---

## A/K/F: Fair Overhang Clearing

When a leveraged account goes bankrupt, two things need to happen: remove the position quantity from open interest, and distribute any uncovered deficit across the opposing side.

Traditional ADL queues pick specific counterparties and force-close them.
Percolator replaces the queue with lazy side indices:

- **A** scales everyone's effective position equally.
- **K** accumulates mark and ADL overhang effects.
- **F** accumulates funding effects.

```
effective_pos(i) = floor(basis_i * A / a_basis_i)
pnl_delta(i)     =
    floor(|basis_i| * ((K - k_snap_i) * FUNDING_DEN + (F - f_snap_i))
          / (a_basis_i * POS_SCALE * FUNDING_DEN))
```

When a liquidation reduces OI, `A` decreases -- every account on that side
shrinks by the same ratio. When a deficit is socialized, `K` shifts -- every
account absorbs the same per-unit loss. Funding moves through `F` the same way:
accounts settle against their snapshots when touched.

No account is singled out. Settlement is O(1) per account and order-independent.

### Markets Return to Healthy

A/K/F guarantees forward progress through a deterministic cycle:

**DrainOnly** — when `A` drops below a precision threshold, no new OI can be added. Positions can only close.

**ResetPending** — when OI reaches zero, the engine snapshots `K`, increments the epoch, and resets `A` back to 1. Remaining accounts settle their residual PnL exactly once when next touched.

**Normal** — once all stale accounts have settled and OI is confirmed zero, the side reopens for trading with full precision.

No admin intervention. No governance vote. The state machine always makes progress.

---

## Price/Funding Envelope

The third invariant is a system bound: an exposed market cannot be cranked
through an arbitrary oracle or funding jump in one step.

For any crank that advances the engine price while open interest exists, the
allowed price move is capped by elapsed slots:

```
abs(P_new - P_last) * 10_000
    <= max_price_move_bps_per_slot * dt * P_last
```

Equivalently, the normalized move is bounded by
`max_price_move_bps_per_slot * dt / 10_000`.

At a high level, the maximum price movement between exposed cranks is bounded by
the system's risk budget. If the market is configured around `L` times leverage,
the safe one-step move is roughly on the order of `1 / L`, with room reserved
for funding, liquidation fees, integer rounding, and fee floors/caps.

This turns "crank often enough" into a hard solvency boundary rather than an
operator preference. A stale or fast-moving oracle target must be fed into the
engine as a capped staircase of effective prices. Same-slot exposed cranks use
the previous price; they cannot mark live OI through a zero-time jump.

Active price or funding accrual also has a maximum elapsed-slot window; beyond
that, ordinary live catch-up fails closed and the wrapper must use recovery or
resolution.

Initialization proves a per-risk-notional envelope for the worst allowed
price/funding step plus liquidation fees. At runtime, before any K/F/price/slot
mutation, the engine checks that the next effective step stays inside that
envelope. If it does not fit, the crank fails closed instead of moving the
market into an unbudgeted state.

---

## How They Compose

| | H | A/K/F | Price/funding envelope |
|---|---|---|---|
| **Solves** | Backed exits | Bankrupt overhang clearing | Bounded live repricing |
| **Math** | Pro-rata profit scaling | Pro-rata position, mark, funding, and deficit scaling | Exact per-risk-notional loss budget |
| **Triggered by** | Withdrawal, conversion, settlement | Mark, funding, liquidation, reset | Live accrual/crank |
| **Failure mode** | Less profit is released | Side drains and resets | Crank fails closed or wrapper stair-steps |

Together:
- No user can withdraw more than exists.
- No user is singled out for forced closure.
- Flat accounts keep their deposits.
- Risk-increasing trades cannot count their own favorable execution slippage as margin.
- Markets recover through deterministic side resets.
- Exposed cranks are bounded to the configured price/funding budget.
- Raw oracle targets are wrapper-owned; the engine only sees capped effective prices.

A/K/F fairness is exact for open-position economics. H fairness is exact for the
currently stored realized claim set, not for the economically "true" claim set
you would get after globally touching every account.

The engine is not the whole public protocol by itself. A compliant wrapper must
enforce authorization, source and clamp oracle/funding inputs, use nonzero live
PnL admission for untrusted public flows, sync recurring fees when enabled, and
reject extraction-sensitive actions while raw oracle target and effective engine
price diverge.

---

## Features

- **v12.17 two-bucket warmup** — unrealized profit sits in a scheduled then pending reserve before entering the matured haircut denominator, bounding oracle-manipulation exposure
- **Per-side funding** — long and short funding indices (F coefficients) are tracked independently, enabling asymmetric funding rates
- **ADL via A/K coefficients** — position overhang is cleared lazily without singling out counterparties; O(1) per account, order-independent
- **Three-phase side reset** — `DrainOnly` → `ResetPending` → `Normal` guarantees markets always recover without admin intervention
- **No external dependencies** — pure `no_std` compatible Rust library; no CPI, no token transfers, no signer checks

## Build and Test

```bash
# Run the full test suite (uses MAX_ACCOUNTS=64 for speed)
cargo test --features test

# Run property tests and edge-case harnesses
cargo test --features test -- --include-ignored

# Run Kani formal verification proofs (one-time setup required)
cargo install --locked kani-verifier
cargo kani setup
cargo kani

# 471 Kani proof harnesses, 1,265 tests, 0 failures
```

## Security

See [THREAT_MODEL.md](THREAT_MODEL.md) for the full trust model, known deferred findings, and deployment checklist.

## Specification

The normative spec is in [spec.md](spec.md). It covers the H haircut ratio, A/K coefficient mechanics, two-bucket warmup math, funding computation, and all state machine transitions.

## Open Source

Fork it, test it, send bug reports. Percolator is open research under Apache-2.0.

## References

- Tarun Chitra, *Autodeleveraging: Impossibilities and Optimization*, arXiv:2512.01112, 2025. https://arxiv.org/abs/2512.01112
