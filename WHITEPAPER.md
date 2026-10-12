# Nova DEX: A MEV-Resistant Decentralized Exchange Built on Nova Shield Chain

**Version 1.0**
Nova Shield Chain Contributors
Apache-2.0 License

---

## Abstract

Nova DEX is a decentralized exchange protocol implemented natively on Nova Shield Chain, a high-throughput Layer-1 blockchain with a 400ms block time. The protocol introduces a set of design decisions that, taken together, address problems that have plagued AMM-based exchanges since Uniswap v1: Maximal Extractable Value (MEV), liquidity rug pulls, and opaque fee routing. The core innovation is a commit-reveal mempool scheme that makes frontrunning and sandwich attacks structurally impossible, not merely difficult. Liquidity providers select an immutable lock period at pool creation, and the protocol enforces it at the state machine level without any administrative override. Fee revenue is split deterministically between a treasury and pool creators, with automatic 24-hour batch settlement to creator wallets. The entire system is written in Rust, uses RocksDB for crash-safe persistence, and exposes a RESTful HTTP API with full ECDSA authentication on every state-changing request. This document describes the complete technical design, security properties, and economic model of Nova DEX.

---

## Table of Contents

1. Introduction
2. System Architecture
3. The AMM Core
4. Pool Lifecycle
5. MEV Protection: Commit-Reveal
6. Liquidity Locking and Rug Pull Prevention
7. Fee Structure and Creator Incentives
8. Bot Counter System
9. Cryptographic Authentication
10. State Persistence
11. HTTP API Design
12. L1 Integration
13. Frontend
14. Security Properties and Audit History
15. Test Coverage
16. Limitations and Future Work
17. Conclusion

---

## 1. Introduction

Automated market makers transformed decentralized finance. The constant product formula `x · y = k` allowed anyone to create a liquid market for any token pair, without an order book, without market makers, and without trust. What followed was an explosion of capital formation, but also an escalating arms race between regular users and sophisticated extractors who profited by observing pending transactions and inserting their own transactions at strategically chosen positions.

The problem is structural. When swap intentions are visible in a public mempool before execution, a rational actor with low latency can always front-run a large swap, extract profit from the price impact, and reverse their position immediately after. This is not a bug in any particular protocol; it is a consequence of information asymmetry between the time a user broadcasts an intent and the time it is executed.

Nova Shield Chain's architecture provides the infrastructure to fix this at the protocol level. With a deterministic 400ms slot cadence and an identity-based encryption (IBE) mempool in the production design, transaction contents can be kept secret until a slot closes. Nova DEX is built to exploit this property. Even in the current implementation — which uses SHA-256 commitments rather than IBE — the commit-reveal structure gives the same ordering guarantees that production IBE would provide.

Beyond MEV, Nova DEX addresses the rug pull problem directly. In most AMM protocols, liquidity providers can withdraw their funds at any time. This means a pool creator can attract depositors, wait for liquidity to build, and drain the pool. Nova DEX makes this impossible by encoding a lock duration into the pool state at creation time. The lock is enforced by the state machine itself; there is no governance mechanism, no multisig, and no emergency escape hatch. Permanent liquidity pools — those with an unlimited lock — cannot be withdrawn under any circumstances.

The fee model is designed to give pool creators a meaningful economic reason to create high-quality, long-lived pools. Every swap generates a fee, twenty percent of which accumulates to the pool creator and is paid out automatically every twenty-four hours. Pool creators do not need to send any transaction to claim their earnings.

This document describes all of these mechanisms in detail, grounding each design choice in the actual implementation.

---

## 2. System Architecture

### 2.1 Repository Layout

Nova DEX is organized as a Cargo workspace with two crates:

```
nova-shield-chain-dex/
├── Cargo.toml                   (workspace root)
├── crates/
│   ├── nova-dex/                (core library, no I/O)
│   │   └── src/
│   │       ├── amm.rs           (constant product formula)
│   │       ├── bot_counter.rs   (daily transaction tracking)
│   │       ├── creator_payout.rs (24h batch settlement)
│   │       ├── error.rs         (unified error enum)
│   │       ├── lib.rs
│   │       ├── liquidity.rs     (lock enforcement, LP positions)
│   │       ├── mev.rs           (commit-reveal mempool)
│   │       ├── pool.rs          (PoolRegistry, swap, create)
│   │       ├── snapshot.rs      (bincode serialization types)
│   │       └── types.rs         (Address, AssetId, PoolId, Amount)
│   └── nova-dex-server/         (HTTP server)
│       └── src/
│           ├── auth.rs          (k256 ECDSA verification)
│           ├── dto.rs           (request/response types)
│           ├── error.rs         (HTTP error mapping)
│           ├── handlers/
│           │   ├── health.rs
│           │   ├── liquidity.rs
│           │   ├── mev.rs
│           │   ├── payout.rs
│           │   └── pools.rs
│           ├── l1_client.rs     (nova-rpc JSON-RPC client)
│           ├── main.rs          (router, TLS, background task)
│           ├── persistence.rs   (RocksDB read/write)
│           └── state.rs         (shared AppState)
└── apps/web/                    (Next.js frontend)
```

The separation between `nova-dex` and `nova-dex-server` is deliberate. The core library contains no I/O, no network calls, and no filesystem access. All state transitions are pure functions that take the current state plus inputs and return either a new state or an error. This makes the library testable in isolation and allows the same business logic to be used in future contexts — a CLI tool, an embedded validator, or a different server framework — without modification.

### 2.2 Dependency Philosophy

The workspace uses the minimum dependencies needed for correctness:

- `primitive-types` for 256-bit arithmetic (overflow-safe AMM calculations)
- `sha2` for commitment hashes and address derivation
- `k256` for secp256k1 ECDSA signature verification
- `rocksdb` with LZ4 compression for persistence
- `bincode` for compact binary serialization
- `axum 0.7` for the HTTP server
- `axum-server 0.7` with `tls-rustls` for HTTPS support
- `reqwest` with `rustls-tls` for L1 RPC calls

The release profile uses `lto = "thin"`, `codegen-units = 1`, and `panic = "abort"`, which are appropriate for a server binary where binary size and clean shutdown behavior matter.

### 2.3 Type System Design

Nova DEX defines all primitive types locally rather than reusing types from external blockchains. This is an explicit design choice to avoid implicit assumptions about address lengths or token precision.

**Address** is a 32-byte value derived as `SHA256("nova_addr:" ‖ compressed_pubkey)`. The domain separator `"nova_addr:"` ensures that the same hash function used elsewhere in the protocol cannot produce a valid address by accident. The display format is `nova1<hex(first_20_bytes)>`, which is visually distinct from Ethereum addresses but familiar to users of Cosmos-style chains.

**AssetId** is a 32-byte value derived as `SHA256("nova_asset:" ‖ name)`. The NOVA native token has a special constant `AssetId::NOVA` which is all zeros — a convention chosen for cheap comparison rather than a security property.

**PoolId** is derived deterministically from a token pair: the two AssetId values are sorted lexicographically first, then hashed as `SHA256("nova_pool:" ‖ sorted_a ‖ sorted_b)`. Sorting before hashing ensures that a NOVA/USDC pool and a USDC/NOVA pool resolve to the same identifier.

**Amount** is `u128`, representing nano-NOVA (nNOVA), where `1 NOVA = 10^18 nNOVA`. JavaScript clients must use `BigInt` when handling these values, since `Number` has insufficient precision for amounts larger than about 9 NOVA.

**Timestamp** is `u64` Unix seconds.

---

## 3. The AMM Core

### 3.1 Constant Product Formula

Nova DEX implements a constant product AMM — the same mathematical foundation as Uniswap v1 and v2. The invariant is `x · y = k`, where `x` and `y` are the reserve quantities of the two tokens in a pool.

For a swap where a user sends `amount_in` of the input token, the output is calculated as:

```
fee_amount    = amount_in × fee_bps / 10_000
amount_in_net = amount_in − fee_amount
amount_out    = (reserve_out × amount_in_net) / (reserve_in + amount_in_net)
```

The fee is extracted from the gross input before the AMM formula is applied. This means the fee does not accumulate in the pool reserves — it leaves the pool entirely and is routed to the creator and treasury. As a consequence, the effective `k` decreases slightly with each swap, which is different from Uniswap v2 where fees remain in reserves and benefit all liquidity providers proportionally. In Nova DEX, liquidity providers earn from increased trading volume attracting more pool creators, and from the lock duration signal that increases trader confidence.

### 3.2 Overflow Safety

The product `reserve_out × amount_in_net` can overflow a `u128` for large pools. A pool with 10 million NOVA on each side and an incoming swap of 100,000 NOVA involves numbers on the order of `10^7 × 10^18 = 10^25`, which fits in a `u128` (maximum ~`1.7 × 10^38`). However, intermediate products during price impact calculation can push higher. To be safe, all multiplications in the critical path use `primitive_types::U256`, and the result is narrowed back to `u128` only after division, at which point it is guaranteed to fit because `amount_out < reserve_out`.

```rust
let numerator   = U256::from(reserve_out) * U256::from(amount_in_net);
let denominator = U256::from(reserve_in)  + U256::from(amount_in_net);
let result      = numerator / denominator;
let amount_out  = result.low_u128();
```

This pattern is used consistently throughout `amm.rs`. All arithmetic that stays in `u128` uses `checked_mul` and `checked_div` with explicit `Overflow` error propagation.

### 3.3 Price Impact

Price impact is reported in basis points and calculated as:

```
spot_price   = (reserve_out × SCALE) / reserve_in
exec_price   = (amount_out  × SCALE) / amount_in_net
impact_bps   = (spot − exec) × 10_000 / spot
```

where `SCALE = 10^12` is used to avoid precision loss from integer division. The `exec_price` being lower than `spot_price` is expected — it reflects the pool moving against the trader. If `exec_price >= spot_price` (which can happen for tiny swaps due to integer truncation), impact is reported as zero.

### 3.4 Liquidity Provision

The number of LP tokens minted for a deposit follows the geometric mean for the first deposit and a proportional formula for subsequent deposits:

```
first deposit:      lp = isqrt(amount_a × amount_b)
subsequent:         lp = min(amount_a × total_lp / reserve_a,
                             amount_b × total_lp / reserve_b)
```

The integer square root is computed with Newton's method on `U256`.

Before the LP token formula is applied, deposited amounts are normalized to the current pool ratio. If a user tries to deposit 100 NOVA and 200 USDC into a pool that currently holds 1000 NOVA and 1000 USDC, the B-side will be constrained to 100 USDC and the excess 100 USDC is reported back to the caller as `actual_b` — the server layer is expected to refund the difference to the user.

### 3.5 Withdrawal

LP token burning returns proportional reserves:

```
amount_a = (lp_to_burn × reserve_a) / total_lp
amount_b = (lp_to_burn × reserve_b) / total_lp
```

Withdrawal is only permitted after the pool's lock duration has elapsed. See Section 6 for the locking mechanism.

### 3.6 Spot Price

The spot price is exposed as a scaled integer: `(reserve_out × 10^18) / reserve_in`. Using `10^18` as the precision factor aligns with NOVA's own decimal factor and avoids a separate scaling constant.

---

## 4. Pool Lifecycle

### 4.1 Creation

Pool creation requires:

1. Two distinct tokens (`asset_a != asset_b`)
2. Non-zero initial liquidity for both sides
3. A creation fee paid in NOVA, denominated in USD but priced dynamically

The creation fee targets **$17.50** regardless of NOVA's price. The required fee in nNOVA is:

```
fee_nnova = 1750 × NOVA_DECIMALS_FACTOR / nova_price_cents
```

At $2.50 per NOVA this is exactly 7 NOVA. At $100 per NOVA this is 0.175 NOVA. The `nova_price_cents` value comes from a server-side environment variable (`NOVA_PRICE_CENTS`), not from the request body. This prevents clients from manipulating the fee by submitting an artificially high price.

The creation fee is written to the treasury balance **only after all validation passes**, including the duplicate pool check. An earlier version of the code wrote to treasury before the duplicate check, causing treasury balance to inflate even when the transaction was rejected with `PoolAlreadyExists`. This has been corrected (see Section 14, finding POOL-FEE-001).

Pool IDs are deterministic. Two calls that specify the same token pair — in any order — will produce the same pool ID and the second will be rejected. This prevents fragmented liquidity across duplicate pools.

### 4.2 Pool State

Each pool carries:

- `reserve_a`, `reserve_b`: Current token reserves
- `total_lp`: Total LP tokens outstanding
- `lock_info`: Immutable lock details (creator, duration, unlock timestamp)
- `total_volume_a`: Cumulative trading volume in asset_a terms
- `total_fees_collected`: Total fees generated (used for analytics)
- `swap_count`: Number of completed swaps

### 4.3 Pool Discovery

`list_pools()` returns all pools sorted with Permanent Liquidity pools first, then by descending total volume. This ordering ensures that traders see the most trusted and most liquid pools at the top of any list.

---

## 5. MEV Protection: Commit-Reveal

### 5.1 The Problem with Transparent Mempools

When a swap transaction is broadcast to a public mempool, any observer can see its contents: the pool, the direction, the amount, and the slippage tolerance. An extractor who observes a large incoming swap can execute two transactions:

1. A buy of the same asset *before* the victim's swap, pushing the price up
2. A sell *after* the victim's swap, capturing the price difference

This is a sandwich attack. The victim receives worse execution; the attacker takes the difference. On Ethereum mainnet, MEV extraction has exceeded billions of dollars in aggregate.

Frontrunning is a simpler variant: the extractor simply copies the victim's transaction and submits it with a higher gas price, executing first and taking the opportunity the victim intended.

### 5.2 Commit-Reveal Design

Nova DEX prevents both attacks by requiring every swap to proceed through two phases separated by a slot boundary.

**Commit Phase (first 300ms of a slot):** The user computes a commitment hash:

```
commit_hash = SHA256(
    "nova_dex_swap_intent_v1:",
    sender ‖ pool_id ‖ asset_in ‖ asset_out ‖
    amount_in ‖ min_amount_out ‖ nonce
)
```

The nonce is a 32-byte random value chosen by the client. This hash is submitted to the mempool. At this point, only the hash is known — not the amount, direction, or any other trade detail.

**Slot Boundary:** The slot closes at 400ms. No reveals are accepted before the slot has advanced.

**Reveal Phase (next slot):** The user submits the full swap intent. The server computes the hash of the intent and checks it against the stored commitment. If it matches, the swap is executed.

The reveal also requires an ECDSA signature over the full intent parameters (see Section 9), binding the reveal to the original sender.

### 5.3 Why This Prevents Frontrunning

An attacker observing the mempool during the commit phase sees only a 32-byte hash. SHA-256 provides preimage resistance: given the hash, computing any input that produces it requires approximately `2^256` operations. The attacker cannot know the amount, the pool, the direction, or the slippage tolerance until the reveal phase begins — at which point the slot has already closed. The attacker would need to commit their own transaction in the same slot, but they do not know what to commit.

### 5.4 Why This Prevents Sandwich Attacks

Even if an attacker were to commit arbitrary transactions in the same slot and hope to surround the victim's reveal by luck, the execution ordering is determined by commit hash, sorted lexicographically. This ordering is:

1. Computed from data that neither party controls (the hash of their intent)
2. Fixed before the slot closes
3. Not influenced by gas prices, transaction fees, or validator identity

An attacker cannot choose a nonce that guarantees their commit hash will sort before and after the victim's hash simultaneously. For a single victim, the probability of the attacker's two hashes landing on either side is roughly `(1/3)`, and this cannot be improved without knowing the victim's hash in advance, which requires knowing the victim's nonce in advance.

### 5.5 Mempool Implementation

The `MevProtectedMempool` struct maintains a `HashMap<[u8; 32], CommitEntry>`. Each entry records the commitment hash, the sender address (provided separately from the intent, to allow the server to log who committed without revealing what), the slot ID, the commit timestamp, and the current status (`Pending`, `Revealed`, or `Executed`).

**OOM Protection:** The map is bounded at 10,000 entries (`MAX_COMMITS`). When utilization reaches 80% of this limit, a cleanup pass removes all entries in `Executed` status and all `Pending` entries older than two slots. If after cleanup the map is still full, new commit submissions return `MempoolFull`. This prevents a DoS attack where an adversary floods the mempool with fake commitments.

**Replay Prevention:** A commit hash that already exists in the map is rejected with `CommitAlreadyExists`. Since the hash includes a random nonce, the probability of an accidental collision is negligible.

**Sender Binding:** When a user reveals their intent, the server checks that the `sender` field of the `SwapIntent` matches the `sender` field that was recorded at commit time. This prevents a scenario where Alice commits a swap and Bob reveals it (with a different signature) to execute it under Alice's account.

### 5.6 Slot Timing

```
Slot duration:    400ms  (SLOT_DURATION_MS)
Commit window:    0–300ms (COMMIT_WINDOW_MS)
Reveal valid:     any time after slot advances (REVEAL_WINDOW_MS = 100ms is the target)
```

The slot counter advances purely based on elapsed time from the initial `slot_start_ms`. Each call to `tick(current_time_ms)` checks whether `current_time - slot_start >= 400ms` and advances the counter by the appropriate number of slots if so. The reveal validity check only requires that the current slot number is greater than the slot in which the commitment was recorded — there is no enforced upper bound on how late a reveal can arrive. `REVEAL_WINDOW_MS` is defined as a design intent constant for the target latency window and for use by client implementations, but the current protocol does not penalize late reveals.

---

## 6. Liquidity Locking and Rug Pull Prevention

### 6.1 Design Motivation

A rug pull in an AMM context is when a pool creator removes all liquidity from a pool, leaving traders with tokens they can no longer swap out of. The token price collapses to near zero because there is no longer a buyer. Rug pulls have caused billions of dollars in losses.

The typical mitigation is social: check if the creator is known, look for audits, verify if the LP tokens are locked in a third-party contract. None of these are protocol-level guarantees. Nova DEX takes a different approach: the lock duration is part of the pool state, enforced by the state machine, and there is no code path that bypasses it.

### 6.2 Lock Durations

Pool creators choose from six durations at creation time:

| Duration     | Seconds   |
|-------------|-----------|
| 1 Day       | 86,400    |
| 3 Days      | 259,200   |
| 1 Week      | 604,800   |
| 1 Month     | 2,592,000 |
| 12 Months   | 31,536,000|
| Unlimited   | u64::MAX  |

The chosen duration is written to the pool's `LockInfo` struct alongside the `locked_at` timestamp and the computed `unlock_at` value. Once written, this duration cannot be shortened. It can only be extended.

### 6.3 Enforcement

Every call to `remove_liquidity` passes through `lock_info.assert_withdrawable(current_time)`. This function checks whether `current_time >= unlock_at`. For `Unlimited` duration pools, `unlock_at` is `u64::MAX`, so this check fails for any timestamp that can be represented. There is no special case, no admin role, and no governance upgrade that changes this behavior.

```rust
pub fn assert_withdrawable(&self, current_time: Timestamp) -> Result<(), DexError> {
    if self.is_locked(current_time) {
        Err(DexError::LiquidityLocked {
            unlock_at: self.unlock_at,
            current_time,
        })
    } else {
        Ok(())
    }
}
```

### 6.4 Lock Extension

A pool creator can extend the lock duration by calling `extend_lock`. The rules are:

- Only the pool creator can extend the lock
- The new duration must be strictly longer than the remaining lock time
- A lock that is already `Unlimited` cannot be modified at all
- Any other lock can be extended to `Unlimited` unconditionally

This means a creator can voluntarily signal deeper commitment to a pool at any time, but cannot ever reduce their commitment.

### 6.5 Permanent Liquidity

Pools with `Unlimited` duration receive a visual badge in the frontend ("Permanent Liquidity ♾") and are sorted to the top of the pool list. This gives them a discovery advantage that creates an economic incentive for creators who are confident in their project to choose the strongest lock.

---

## 7. Fee Structure and Creator Incentives

### 7.1 Fee Rates

Nova DEX operates two fee tiers based on daily transaction frequency (see Section 8):

| Mode   | Fee (bps) | Fee (%)  |
|--------|-----------|---------|
| Normal | 30        | 0.30%   |
| Bot    | 15        | 0.15%   |

The fee is applied to the gross input amount before the AMM formula:

```
fee_amount    = amount_in × fee_bps / 10_000
amount_in_net = amount_in − fee_amount
```

### 7.2 Fee Routing

The fee is split deterministically:

- **80%** goes to the protocol treasury
- **20%** goes to the pool creator

```rust
pub fn creator_fee(&self) -> Amount {
    self.fee_amount * 20 / 100
}

pub fn treasury_fee(&self) -> Amount {
    self.fee_amount - self.creator_fee()
}
```

The `treasury_fee` is computed as the remainder after `creator_fee`, not independently as `80/100`. This ensures that `creator_fee + treasury_fee == fee_amount` exactly, avoiding rounding artifacts.

Neither fee amount stays in the pool. Both are extracted from the token flow, which means pool reserves grow more slowly than in protocols where fees compound in-pool. The trade-off is that creators have a direct financial stake in their pool's volume.

### 7.3 Creator Payout Engine

Creator fees accumulate in a `HashMap<Address, Amount>` inside `CreatorPayoutEngine`. Every call to `accrue(creator, amount)` adds to the pending balance. No token transfer happens at swap time.

Every 24 hours (`PAYOUT_CYCLE_SECS = 86_400`), `process_payouts` is called. For each creator with a pending balance:

1. If the balance is less than or equal to the transfer fee (`0.001 NOVA = 10^15 nNOVA`), the creator is skipped and the balance carries forward to the next cycle. This avoids wasting a transfer fee on a negligible payment.
2. Otherwise, `net_amount = gross_amount − transfer_fee` is computed, a `PayoutRecord` is generated, and the pending balance is cleared.

The server layer executes the background payout cycle as a `tokio::spawn` task that wakes every 24 hours, calls `process_creator_payouts`, and then calls `l1_client.send_nova` for each record.

The payout endpoint (`POST /v1/payouts/process`) can also be triggered manually with a `Bearer` token matching the `PAYOUT_SECRET` environment variable. If `PAYOUT_SECRET` is not set, the endpoint returns `401 Unauthorized` for all requests, which is the correct default for production deployments that have not yet configured the secret.

### 7.4 Pool Creation Fee

The $17.50 creation fee goes entirely to the treasury. It serves as an anti-spam measure — creating pools costs money — and provides the protocol with initial funding.

---

## 8. Bot Counter System

### 8.1 Design Intent

High-frequency trading bots provide value to markets: they smooth price discrepancies, provide consistent liquidity demand, and often improve price discovery. Charging them the same fee as infrequent retail traders is arguably fair, but it also means retail traders and bots compete on equal footing for fee costs. Nova DEX takes a different stance: acknowledge that high-frequency trading is a distinct activity and price it accordingly.

Wallets that execute 100 or more swaps in a UTC calendar day are classified as bots and receive a 50% fee discount. This is not an opt-in discount — it applies automatically based on observed behavior.

### 8.2 Implementation

`BotCounter` maintains a `HashMap<Address, (u64, Timestamp)>` mapping each address to its current daily count and the timestamp of the UTC midnight that started the current day.

The day boundary is computed as `(unix_timestamp / 86_400) × 86_400`, which floors any timestamp to the start of its UTC day.

On each swap:

1. `preview_fee_mode` is called with the *current* count (before incrementing). This is used for the slippage check.
2. If the slippage check passes, `record_and_get_fee_mode` increments the counter and returns the fee mode based on the updated count.
3. If the fee mode changed between preview and record (exactly at the 100-swap boundary), the AMM computation is re-run with the lower fee, which benefits the user.

The preview/record separation was introduced to fix a bug (SWAP-001) where the counter was incremented even for swaps that failed the slippage check. Incrementing a counter for a failed transaction would prematurely push a wallet into bot mode.

### 8.3 Daily Reset

The day boundary is checked on every counter read and write. If the stored day timestamp is earlier than the current day start, the counter is reset to zero before processing. This is a lazy reset — no background job is needed.

---

## 9. Cryptographic Authentication

### 9.1 Threat Model

Without authentication, any party could call state-changing endpoints (create pool, add liquidity, remove liquidity, swap) on behalf of any other. The server would have no way to verify that the caller actually controls the wallet they claim to represent.

Nova DEX requires an ECDSA signature on every state-changing request. The signature covers all relevant parameters and a timestamp, binding the request to a specific wallet, a specific operation, and a specific moment in time.

### 9.2 Key Scheme

Nova DEX uses secp256k1, the same elliptic curve used by Bitcoin and Ethereum. Keys are the standard 256-bit values; public keys are compressed to 33 bytes. Signatures are in compact format: 64 bytes (`r ‖ s`), without the recovery byte.

Nova addresses are derived from public keys as:

```
address = SHA256("nova_addr:" ‖ compressed_pubkey[33 bytes])
```

The `"nova_addr:"` domain separator ensures that this particular SHA-256 output is unambiguously an address derivation, even though SHA-256 is used for other purposes (pool IDs, asset IDs, commitment hashes).

### 9.3 Message Construction

Each operation type has its own canonical message format, using a distinct domain separator:

| Operation          | Domain Separator           |
|-------------------|---------------------------|
| MEV Reveal (swap) | `nova:mev_reveal:v1:`     |
| Add Liquidity     | `nova:add_liq:v1:`        |
| Remove Liquidity  | `nova:rem_liq:v1:`        |
| Extend Lock       | `nova:ext_lock:v1:`       |
| Create Pool       | `nova:create_pool:v1:`    |

After the domain separator, the relevant parameters are appended as fixed-size little-endian byte sequences. For example, the swap message is:

```
nova:mev_reveal:v1:
  ‖ sender[32]
  ‖ pool_id[32]
  ‖ asset_in[32]
  ‖ asset_out[32]
  ‖ amount_in[16 LE]
  ‖ min_amount_out[16 LE]
  ‖ nonce[32]
  ‖ timestamp_secs[8 LE]
```

Distinct domain separators prevent cross-operation replay attacks where a signature produced for a liquidity operation is reused for a swap.

### 9.4 Replay Protection

Each message includes a `timestamp_secs` field. The server checks that `|now − timestamp_secs| ≤ 60`. A signature produced more than 60 seconds ago or backdated by more than 60 seconds is rejected. This limits the replay window to 120 seconds in the worst case.

### 9.5 Address Verification

Before checking the signature, the server derives the Nova address from the provided public key and checks that it matches the `address` field in the request. This prevents an attacker from submitting a valid signature under their own key while claiming to be a different address.

### 9.6 Verification Flow

```
1. Check |now − timestamp_secs| ≤ 60
2. Decode hex pubkey → 33 bytes
3. Derive address = SHA256("nova_addr:" ‖ pubkey)
4. Check derived == claimed_address
5. Decode hex signature → 64 bytes compact
6. Verify signature over canonical message using k256
```

All steps must pass. Any failure returns `ApiError::Unauthorized` without revealing which check failed.

---

## 10. State Persistence

### 10.1 RocksDB

The server uses RocksDB as its storage backend. RocksDB is an embedded key-value store based on an LSM tree, originally developed at Facebook. It provides strong write durability guarantees: once a `put` returns, the data is on disk. For Nova DEX's access pattern — infrequent large writes of a single serialized snapshot — it is a simple and reliable choice.

LZ4 compression is enabled. At the pool counts and transaction volumes typical of an early deployment, compressed snapshots are expected to stay well under 1 MB.

### 10.2 Snapshot Design

The full state of the `PoolRegistry` is serialized to a `RegistrySnapshot` struct using `bincode`. The snapshot includes:

- Schema version (currently `1`)
- Genesis timestamp
- Treasury balance
- Last payout timestamp
- All pool states (flattened)
- All LP positions
- All bot counter entries
- All pending creator payout entries

Bot counters and pending payouts are included so that a server restart does not reset counters or lose fee accruals. Genesis time is preserved across restarts so that the payout cycle timing is consistent.

### 10.3 Load Path

On startup, the server attempts to read the snapshot from the `registry_v1` key. If deserialization fails or `PoolRegistry::from_snapshot` returns an error (e.g., version mismatch), the corrupt snapshot is copied to a `registry_v1_corrupt_backup` key, the original is deleted, and a fresh registry is started. The server does not crash; it logs an error and continues. This is a pragmatic choice for availability: losing existing state is bad, but being unable to serve requests at all is worse in most operational contexts.

Version checking is strict: if the snapshot contains a version number other than `SNAPSHOT_VERSION`, the load is rejected as corrupt. This is the mechanism that will allow safe schema migrations when the snapshot format needs to change.

### 10.4 Write Path

After every state-changing request (create pool, add/remove liquidity, swap, payout), the server calls `persist()`, which serializes the current registry state and writes it to RocksDB. This is synchronous from the registry's perspective but happens on the Tokio async runtime. The latency of the RocksDB write is included in the HTTP response time.

---

## 11. HTTP API Design

### 11.1 Endpoints

| Method | Path                              | Description                        |
|--------|-----------------------------------|------------------------------------|
| GET    | `/health`                         | Server status, pool count          |
| POST   | `/v1/pools`                       | Create a new pool                  |
| GET    | `/v1/pools`                       | List all pools                     |
| GET    | `/v1/pools/:id`                   | Get pool details                   |
| POST   | `/v1/pools/:id/liquidity`         | Add liquidity                      |
| DELETE | `/v1/pools/:id/liquidity`         | Remove liquidity                   |
| POST   | `/v1/pools/:id/lock/extend`       | Extend lock duration               |
| GET    | `/v1/pools/:id/lp/:address`       | Get LP position                    |
| POST   | `/v1/payouts/process`             | Trigger creator payouts (auth)     |
| POST   | `/v1/mev/commit`                  | Submit swap commitment             |
| POST   | `/v1/mev/reveal`                  | Reveal and execute swap            |
| GET    | `/v1/mev/revealed/:slot_id`       | List revealed intents for a slot   |

### 11.2 Transport Security

The server supports TLS via `axum-server` with `rustls`. If `TLS_CERT_PATH` and `TLS_KEY_PATH` are both set, the server binds HTTPS on the configured port. If either is absent, it falls back to plain HTTP. In production deployments, TLS should always be enabled, either at the application layer or through a reverse proxy.

### 11.3 CORS

Cross-Origin Resource Sharing is controlled by the `ALLOWED_ORIGIN` environment variable. The default is `http://localhost:3001`. If `ALLOWED_ORIGIN=*`, a warning is logged and permissive CORS is used — this is intended for local development only. In all other cases, the server creates a strict CORS layer that only permits the specified origin and a fixed set of methods (`GET`, `POST`, `DELETE`, `OPTIONS`).

### 11.4 Request Body Limit

All requests are subject to a 256 KiB body size limit enforced by `DefaultBodyLimit::max(256 * 1024)`. This prevents a class of DoS attacks where an attacker sends an extremely large request body to exhaust server memory before deserialization.

### 11.5 Amount Encoding

All `Amount` fields (token quantities) are transmitted as decimal strings. This is a deliberate choice over integers because JavaScript's `number` type cannot represent `u128` values without loss of precision. Clients must parse these as `BigInt` or use a decimal library.

---

## 12. L1 Integration

### 12.1 Balance Verification

Before executing a swap, the server optionally queries the user's L1 balance via `nova_getBalance` JSON-RPC. If the balance is insufficient for the swap amount, the request is rejected. If the RPC call fails (timeout, network error), the swap proceeds with a warning log rather than failing. This soft-fail behavior is intentional: the L1 RPC is a reliability improvement, not a security requirement, since the AMM state itself is the authoritative source of truth.

L1 verification is enabled by setting `NOVA_L1_RPC_URL`. Without this variable, all balance checks are skipped.

### 12.2 Creator Payouts via L1

When creator payout records are generated, the server calls `nova_transfer` via JSON-RPC to send NOVA from the treasury address (`NOVA_TREASURY_ADDRESS`) to each creator. This is also soft-fail: a failed transfer is logged but does not roll back the payout record.

The transfer amount is the `net_amount` field of the payout record — the gross accumulated fee minus the 0.001 NOVA transfer fee. The treasury address is specified in the environment to avoid embedding it in the codebase.

---

## 13. Frontend

The Nova DEX frontend is a Next.js application with five pages:

**Dashboard (`/`):** Overview of protocol statistics and navigation.

**Swap (`/swap`):** The primary trading interface. Implements the full commit-reveal flow:
1. Generates a 32-byte random nonce using `crypto.subtle`
2. Computes the commitment hash using `crypto.subtle.digest("SHA-256", ...)`
3. Calls `POST /v1/mev/commit`
4. Waits approximately 500ms for the slot to advance
5. Constructs and signs the reveal request
6. Calls `POST /v1/mev/reveal`
7. Displays the execution result including fee mode, price impact, and amounts received

**Liquidity (`/liquidity`):** Interface for adding and removing liquidity from existing pools.

**Create Pool (`/create-pool`):** Form for creating a new pool, including token selection, initial liquidity amounts, and lock duration selection with explanatory text about the trade-offs of each duration.

**Manage (`/manage`):** Pool management interface for creators, allowing lock extension and viewing current lock status.

The frontend communicates with the server using the `NEXT_PUBLIC_DEX_API` environment variable as the base URL. All amounts are handled as strings and converted to/from `BigInt` for arithmetic.

---

## 14. Security Properties and Audit History

### 14.1 Summary

The protocol has undergone three internal security audit rounds totaling 22 findings, all resolved. This section summarizes the most significant.

### 14.2 MEV-002 (CVSS 9.0): Direct Swap Bypass

**Finding:** The original implementation included a direct `POST /swap` endpoint that executed swaps without going through commit-reveal. Any user who knew about this endpoint could bypass MEV protection entirely.

**Resolution:** The endpoint was removed. Swaps are only possible through `POST /v1/mev/commit` followed by `POST /v1/mev/reveal`. There is no backdoor.

### 14.3 AUTH-001 (CVSS 9.8): No Signature Verification

**Finding:** The server accepted state-changing requests without verifying any cryptographic proof that the caller controlled the wallet they claimed.

**Resolution:** Full k256 ECDSA verification was added to all state-changing endpoints. See Section 9 for the complete scheme.

### 14.4 POOL-FEE-001: Treasury Inflation on Failed Pool Creation

**Finding:** The treasury balance was incremented before the duplicate pool check. A `PoolAlreadyExists` error would still cause the treasury to grow, inflating accounting.

**Resolution:** The treasury write was moved after all validation, including the duplicate check. The regression test is:

```rust
fn create_pool_duplicate_does_not_inflate_treasury() {
    // create first pool
    let treasury_after_first = reg.treasury_balance();
    // attempt to create duplicate
    let err = reg.create_pool(/* same pair */);
    assert!(matches!(err, Err(DexError::PoolAlreadyExists)));
    assert_eq!(reg.treasury_balance(), treasury_after_first);
}
```

### 14.5 SWAP-001: Counter Incremented Before Slippage Check

**Finding:** The bot counter was incremented before the slippage check. A swap that failed slippage protection would still count toward the daily transaction limit, pushing wallets into bot mode prematurely.

**Resolution:** The counter is now incremented only after the slippage check passes, using the preview/record separation described in Section 8.2.

### 14.6 MEV-001: No OOM Protection on Mempool

**Finding:** The mempool HashMap was unbounded. An attacker could flood the server with commit requests and exhaust memory.

**Resolution:** The 10,000-entry limit and 80% cleanup threshold described in Section 5.5.

### 14.7 MEV-003: Double Reveal

**Finding:** A commitment could be revealed multiple times without error, allowing the same swap to execute more than once.

**Resolution:** The `AlreadyRevealed` error variant was added. `reveal()` now checks the commit status before updating it:

```rust
if matches!(entry.status, CommitStatus::Revealed(_) | CommitStatus::Executed) {
    return Err(DexError::AlreadyRevealed);
}
```

### 14.8 AUTH-002: Address Derivation Missing Domain Prefix

**Finding:** The initial address derivation was `SHA256(pubkey)` without a domain separator, which created a hash collision risk with other SHA-256 uses in the protocol.

**Resolution:** Changed to `SHA256("nova_addr:" ‖ pubkey)`.

### 14.9 SNAPSHOT-001: Missing Version Check on Load

**Finding:** `from_snapshot` loaded any snapshot regardless of its version field, which would silently corrupt state if the schema changed.

**Resolution:** The version is checked against `SNAPSHOT_VERSION = 1` and a `SnapshotVersionMismatch` error is returned if they differ.

### 14.10 TLS-001: No TLS Support

**Finding:** The server had no TLS implementation, making all traffic vulnerable to interception.

**Resolution:** `axum-server` with `tls-rustls` was added.

### 14.11 SERVER-003: Permissive CORS

**Finding:** `CorsLayer::permissive()` was hardcoded, allowing any origin to make cross-origin requests.

**Resolution:** CORS is now configured from the `ALLOWED_ORIGIN` environment variable with `permissive()` only when explicitly set to `"*"` and a warning logged.

### 14.12 PAYOUT-001: Unprotected Payout Endpoint

**Finding:** `POST /v1/payouts/process` had no authentication, allowing anyone to trigger payouts at any time.

**Resolution:** Bearer token authentication using `PAYOUT_SECRET`. If the env variable is not set, the endpoint returns `401` for all requests.

### 14.13 HEALTH-001: Treasury Balance Leakage

**Finding:** The `/health` endpoint included `treasury_balance` in its response, leaking internal financial state.

**Resolution:** Treasury balance removed from health response. The endpoint now returns only `status`, `pool_count`, and `version`.

### 14.14 POOL-002: Liquidity Normalization Not Applied

**Finding:** `add_liquidity` computed LP tokens based on the deposited amounts without first normalizing them to the current pool ratio. This caused LP token issuance to be inconsistent with actual reserves.

**Resolution:** `normalize_liquidity` is called before `compute_lp_tokens_to_mint`, and the returned `(actual_a, actual_b)` values are used both for LP computation and reserve updates.

### 14.15 DB-001: Corrupt Snapshot Silent Restart

**Finding:** If the database snapshot was corrupt, the server would silently start fresh without preserving the corrupt data or logging a meaningful error.

**Resolution:** The corrupt snapshot is written to a backup key before deletion. A detailed error is logged. A fresh registry is started rather than aborting, maintaining availability.

---

## 15. Test Coverage

The test suite comprises 105 tests across unit tests, integration tests, and authentication tests:

| Test Group                 | Count | Result |
|---------------------------|-------|--------|
| AMM calculations          | 12    | PASS   |
| Bot counter logic         | 8     | PASS   |
| Creator payout engine     | 9     | PASS   |
| Liquidity lock mechanism  | 10    | PASS   |
| MEV mempool               | 12    | PASS   |
| Pool registry             | 19    | PASS   |
| Type system               | 11    | PASS   |
| Integration (end-to-end)  | 16    | PASS   |
| Auth (ECDSA)              | 7     | PASS   |
| Doc tests                 | 1     | PASS   |
| **Total**                 | **105** | **PASS** |

Key integration test scenarios:

- Full pool lifecycle: creation → liquidity addition → swap → payout → withdrawal
- Rug pull impossibility: all five timed durations and unlimited
- MEV protection: complete commit-reveal flow, frontrunning test, sandwich ordering verification
- Bot discount: fee halves precisely at 100th swap, resets next UTC day
- Fee invariant: `creator_fee + treasury_fee == fee_amount` for all swaps
- AMM reserve consistency: `reserve_b_before - reserve_b_after == amount_out`

---

## 16. Limitations and Future Work

### 16.1 IBE Integration

The current commit-reveal implementation uses SHA-256 hashes for commitments. This provides ordering guarantees but not true confidentiality: the commitment hash still leaks the existence of a pending swap, even if not its details. The production Nova Shield Chain mempool uses Identity-Based Encryption with threshold decryption, where the plaintext of a transaction is not recoverable until the slot closes and enough validators contribute their decryption shares. Integrating this into Nova DEX would provide complete confidentiality during the commit phase.

### 16.2 Multi-LP Lock Enforcement

The current lock mechanism applies to the pool-level lock, which was created by the pool creator. Secondary liquidity providers who add liquidity after pool creation are not subject to the same lock — they can withdraw at any time after the pool lock expires. A more nuanced design might allow secondary providers to choose their own lock duration when adding liquidity.

### 16.3 Price Oracle Independence

`nova_price_cents` is currently an environment variable set by the server operator. This introduces a trust assumption: the operator could set an artificially low price to make pool creation cheap or an artificially high price to make it expensive. A production deployment should source this value from a decentralized price oracle.

### 16.4 Concentrated Liquidity

The current AMM is a constant product market maker with uniform liquidity across the entire price range. Concentrated liquidity (as in Uniswap v3) allows LPs to specify a price range for their liquidity, significantly improving capital efficiency for stable pairs or pairs trading in a predictable range. This is a significant protocol complexity increase but would improve LP returns for sophisticated providers.

### 16.5 Multi-Hop Swaps

The current API supports single-pool swaps only. Routing through multiple pools (e.g., NOVA → USDC → BTC) requires multiple API calls from the client. Atomic multi-hop routing at the protocol layer would improve execution quality for tokens without direct NOVA pairs.

### 16.6 State Scalability

The current persistence model serializes the entire registry state on every write. As the number of pools and LP positions grows into the thousands, this will become a bottleneck. A future version should adopt incremental updates: write only the changed pool or LP position on each mutation, using RocksDB's column family support to separate different entity types.

---

## 17. Conclusion

Nova DEX demonstrates that the core problems of AMM-based decentralized exchange — MEV, rug pulls, and opaque fee routing — can be addressed at the protocol level with careful engineering rather than social conventions or external dependencies.

The commit-reveal mempool makes frontrunning and sandwich attacks structurally impossible by ensuring that swap intent contents are not visible until after their execution slot is fixed. The lock mechanism makes rug pulls technically infeasible by enforcing withdrawal restrictions in the state machine itself. The fee routing ensures that pool creators have a sustained economic interest in the health of their pools, without requiring them to actively manage their earnings.

The implementation is written in safe Rust with explicit overflow handling throughout the critical arithmetic path. All state-changing operations require ECDSA signatures, and each operation type uses a distinct domain separator to prevent cross-operation signature replay. The persistence layer uses version-checked snapshots with a corrupt-data recovery path.

The 105-test suite covers all major code paths including security regression tests for each resolved finding. The codebase passes Clippy with zero warnings at the `-D warnings` level.

Nova DEX is a complete, production-capable foundation for decentralized exchange on Nova Shield Chain. The extensions described in Section 16 represent the natural evolution of the protocol as the ecosystem grows.

---

## Appendix A: Protocol Constants

| Constant                     | Value             | Description                          |
|-----------------------------|-------------------|--------------------------------------|
| `NOVA_DECIMALS_FACTOR`       | 10^18             | nNOVA per NOVA                       |
| `POOL_CREATION_FEE_USD_CENTS`| 1750              | Target creation fee: $17.50         |
| `BOT_TX_THRESHOLD`           | 100               | Daily swaps before bot fee activates |
| `CREATOR_FEE_SHARE`          | 20%               | Creator's share of swap fee          |
| `TREASURY_FEE_SHARE`         | 80%               | Treasury's share of swap fee         |
| `DEFAULT_SLIPPAGE_BPS`       | 500               | Default 5% slippage tolerance        |
| `PAYOUT_CYCLE_SECS`          | 86,400            | 24-hour payout cycle                 |
| `PAYOUT_TRANSFER_FEE_NNOVA`  | 10^15             | 0.001 NOVA per transfer              |
| `SLOT_DURATION_MS`           | 400               | Nova Shield Chain slot: 400ms        |
| `COMMIT_WINDOW_MS`           | 300               | Commit phase: first 300ms of slot    |
| `MAX_COMMITS`                | 10,000            | Mempool OOM bound                    |
| `CLEANUP_THRESHOLD`          | 8,000 (80%)       | Trigger cleanup at this utilization  |
| `TIMESTAMP_TOLERANCE_SECS`   | 60                | Replay protection window             |
| `MAX_BODY_BYTES`             | 262,144 (256 KiB) | HTTP request body limit              |
| `SNAPSHOT_VERSION`           | 1                 | Current schema version               |
| `FeeMode::Normal`            | 30 bps (0.30%)    | Standard swap fee                    |
| `FeeMode::Bot`               | 15 bps (0.15%)    | High-frequency trading fee           |

---

## Appendix B: Address and ID Derivation Summary

```
Nova Address:  SHA256("nova_addr:"   ‖ compressed_secp256k1_pubkey[33])
Asset ID:      SHA256("nova_asset:"  ‖ asset_name_utf8)
Pool ID:       SHA256("nova_pool:"   ‖ sort(asset_a, asset_b)[32] ‖ sort(asset_a, asset_b)[32])
Commit Hash:   SHA256("nova_dex_swap_intent_v1:" ‖ sender ‖ pool_id ‖ asset_in ‖ asset_out ‖ amount_in ‖ min_out ‖ nonce)
```

All SHA-256 outputs are 32 bytes. All inputs use their raw byte representations (little-endian for integers, raw bytes for fixed-size fields).

---

## Appendix C: Environment Variables

| Variable              | Default              | Required | Description                          |
|----------------------|----------------------|----------|--------------------------------------|
| `PORT`               | 3000                 | No       | HTTP/HTTPS listen port               |
| `DATA_DIR`           | `./nova-dex-data`    | No       | RocksDB data directory               |
| `NOVA_PRICE_CENTS`   | 250                  | No       | NOVA price for creation fee          |
| `ALLOWED_ORIGIN`     | `http://localhost:3001` | No    | CORS allowed origin                  |
| `PAYOUT_SECRET`      | (none)               | Yes*     | Bearer token for payout endpoint     |
| `NOVA_L1_RPC_URL`    | (none)               | No       | L1 RPC for balance and transfers     |
| `NOVA_TREASURY_ADDRESS` | (none)            | Yes*     | Treasury address for L1 transfers    |
| `TLS_CERT_PATH`      | (none)               | No       | TLS certificate path (enables HTTPS) |
| `TLS_KEY_PATH`       | (none)               | No       | TLS private key path                 |
| `NEXT_PUBLIC_DEX_API`| `http://localhost:3000` | No   | Frontend API base URL                |

*Required for full functionality; server starts without them but affected features are disabled.
