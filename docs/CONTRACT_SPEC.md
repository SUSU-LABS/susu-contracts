# Contract Specification — susu-contracts

> **Phase 1 — implemented.** This document describes the contracts as they are
> implemented in `contracts/factory` and `contracts/group`. Any change to semantics —
> especially fees, custody, authorization, payout selection, or round transitions —
> requires human review before implementation.

## Constants

| Name | Value | Where |
|---|---|---|
| `MAX_FEE_BPS` | `50` | Factory + Group |
| `BPS_DENOMINATOR` | `10_000` | Group |
| `MIN_MEMBERS` | `2` | Factory + Group |
| `MAX_MEMBERS` | `100` | Factory + Group |
| `INSTANCE_TTL_THRESHOLD` | `100_000` ledgers | Factory + Group |
| `INSTANCE_TTL_EXTEND_TO` | `518_400` ledgers | Factory + Group |
| `PERSISTENT_TTL_THRESHOLD` | `100_000` ledgers | Group |
| `PERSISTENT_TTL_EXTEND_TO` | `518_400` ledgers | Group |
| `CONTRACT_VERSION` | `2` | Factory + Group |

```text
fee              = pool * fee_bps / 10_000    (integer division, truncated)
recipient_amount = pool - fee
fee + recipient_amount == pool                (exact, by construction)
```

Truncation always favours the recipient: the fee is truncated, never rounded up.
Floating point is forbidden in any financial calculation.

## Status

```text
DRAFT → OPEN → ACTIVE → COMPLETED
```

| Status | Meaning |
|---|---|
| `Draft` | Created but not accepting members. Defensive default only — see note. |
| `Open` | Accepting members until capacity is reached. Set by the constructor. |
| `Active` | Membership is full and locked; rounds are running. |
| `Completed` | Every round has paid out exactly once. Terminal. |

> **Note.** Construction sets `Open` directly, so a group is never observable in
> `Draft`. `Draft` remains the `unwrap_or` default when the status key is absent, so an
> unset key can never be misread as an active group.

Round sub-state while `ACTIVE`:

`WaitingForContributions → ReadyForPayout → PayoutExecuted → (next round | Completed)`

Missing contribution means **WAIT**. There is no timeout, no skip, and no penalty in
the MVP, so no member can be bypassed or lose their turn.

---

## Construction

Both contracts use a Soroban `__constructor` (deployed via `deploy_v2`) instead of the
`initialize(...)` entry point named in the Phase 0 draft. This is the SDK-idiomatic
equivalent with identical parameters and validation, and it removes the
initialization front-running window: the contract is never observable in an
uninitialized state, so its admin can never be claimed by a third party.

### Factory `__constructor(admin, group_wasm_hash, treasury, fee_bps)`

Sets the admin authority, the wasm hash used to instantiate groups, the treasury that
receives fees, and the fee in basis points.

- Rejects `fee_bps == 0` or `fee_bps > MAX_FEE_BPS` (`InvalidFeeBps`).
- Sets the group count to `0` and `paused` to `false`.

### Group `__constructor(factory, creator, token, treasury, contribution_amount, member_capacity, frequency_seconds, fee_bps)`

Sets the immutable group configuration. Called by the Factory during `create_group`,
but deployable by anyone — a group carries no privileges over the Factory.

- Rejects `contribution_amount <= 0`, capacity outside `[2, 100]`, `fee_bps` outside
  `(0, 50]`, and `frequency_seconds == 0`.
- `factory` and `creator` are informational and carry **no authority**.

---

## Factory

### `create_group(creator, token, contribution_amount, member_capacity, frequency_seconds) → Result<Address, FactoryError>`

Deploys a Group contract from `group_wasm_hash` with the Factory's current treasury and
fee, registers it under a monotonically increasing id, and emits `GroupCreated`.

- Requires `creator` authorization.
- Blocked while paused (`Paused`).
- Rejects invalid parameters (`InvalidContributionAmount`, `InvalidMemberCapacity`,
  `InvalidFrequency`).
- The deterministic salt is derived from the group id, so ids and addresses are
  one-to-one and monotonic.

### `set_fee(fee_bps) → Result<(), FactoryError>`

Updates the protocol fee. Requires admin authorization. Rejects out-of-range values.

> Applies to groups created **afterwards**. A deployed group's fee is part of its
> immutable configuration and cannot be changed — not by the admin, the Factory, or
> anyone else.

### `set_treasury(treasury) → Result<(), FactoryError>`

Updates the treasury used by **newly created** groups. Requires admin authorization.
Existing groups keep the treasury they were created with.

### `pause() → Result<(), FactoryError>` / `unpause() → Result<(), FactoryError>`

Emergency control over **new group creation only**. Requires admin authorization.
Never affects funds already held by existing Group contracts.

### `get_group(group_id) → Result<Address, FactoryError>`

Returns the address of a registered group, or `GroupNotFound`.

### `get_group_count() → u32`

Returns the number of groups created. Also the most recently assigned group id.

### `get_config() → Result<FactoryConfig, FactoryError>`

Returns the full protocol configuration, including the current pause state. Extends
the instance TTL, so a Factory that is only ever read does not archive. Returns
`NotInitialized` rather than panicking if the configuration entry is absent.

### `version() → u32`

Returns `CONTRACT_VERSION`.

### Errors (`FactoryError`)

`InvalidFeeBps = 1`, `InvalidContributionAmount = 2`, `InvalidMemberCapacity = 3`,
`InvalidFrequency = 4`, `Paused = 5`, `GroupNotFound = 6`, `ArithmeticOverflow = 7`,
`InvalidTreasury = 8`, `NotInitialized = 9`.

---

## Group

### `join(member) → Result<u32, GroupError>`

Adds an authenticated member and appends them to the immutable payout order. Returns
the member's 1-based position.

- Requires `member` authorization.
- Only while `Open` (`NotOpen`).
- Rejects duplicates (`AlreadyMember`) and overflow past capacity (`GroupFull`).
- Emits `MemberJoined`.

### `start() → Result<(), GroupError>`

Transitions `Open → Active`, sets the current round to `1`, and begins collecting
contributions. Requires membership to be exactly full (`CapacityNotReached`).

### `contribute(member, amount, round) → Result<(), GroupError>`

An authenticated member contributes the exact configured amount for the current round.

- Requires `member` authorization.
- Fails when the caller is not the member (implicit in auth), the address is not a
  member (`NotAMember`), the round is not the current round (`WrongRound`), the amount
  is not the exact configured amount (`WrongAmount`), or the member has already
  contributed this round (`AlreadyContributed`).
- Only the configured token is reachable: the transfer is from `member` to the group
  contract's own address using the configured token client. A transfer of any other
  asset cannot satisfy the call.
- Emits `ContributionReceived`.

### `execute_payout() → Result<(), GroupError>`

Permissionless once the current round is fully funded. Transfers `fee` to the treasury
and `recipient_amount` to the scheduled recipient, then advances the round exactly once,
emitting `PayoutExecuted` and `FeePaid`.

- Fails when the round is not fully funded — the **WAIT** case
  (`ContributionsIncomplete`) — a payout has already been executed for this round
  (`PayoutAlreadyExecuted`), the group is not `Active` (`NotActive`), or it has already
  completed (`GroupCompleted`).
- The recipient is the member at position `round` in the immutable join order.
- On the final round the group becomes `Completed` and emits `GroupCompleted`.
- A defensive check asserts `fee + recipient_amount == pool` and returns
  `SplitInvariantViolated` if it ever fails, so money math cannot silently mis-split.

### Read functions

| Function | Returns |
|---|---|
| `get_group()` | `GroupState` — config, status, round, member count, phase |
| `get_member(address)` | `u32` — 1-based position, or `0` if not a member |
| `get_member_count()` | `u32` |
| `get_payout_order()` | `Vec<Address>` — the immutable payout order |
| `get_round(round)` | `RoundInfo` — pool, contribution count, phase, recipient |
| `get_current_recipient()` | `Result<Address, GroupError>` |
| `get_current_round()` | `u32` — 1-based; `0` before start |
| `get_pool_balance()` | `i128` — actual token balance of the contract |
| `is_contribution_complete()` | `bool` |
| `get_status()` | `Status` |
| `get_token()` | `Address` |
| `version()` | `u32` |

### Errors (`GroupError`)

`InvalidContributionAmount = 1`, `InvalidMemberCapacity = 2`, `InvalidFeeBps = 3`,
`InvalidFrequency = 4`, `NotOpen = 5`, `AlreadyMember = 6`, `GroupFull = 7`,
`CapacityNotReached = 8`, `NotActive = 9`, `NotAMember = 10`, `WrongRound = 11`,
`WrongAmount = 12`, `AlreadyContributed = 13`, `ContributionsIncomplete = 14`,
`PayoutAlreadyExecuted = 15`, `WrongRoundPhase = 16`, `GroupCompleted = 17`,
`ArithmeticOverflow = 18`, `SplitInvariantViolated = 19`.

---

## Invariants

1. `fee = pool * fee_bps / 10_000`
2. `recipient_amount = pool - fee`
3. `fee + recipient_amount == pool`
4. Exactly one contribution per member per round.
5. Exactly one payout per round.
6. Configured token only; exact configured amount only.
7. No early payout.
8. Recipient derives from the immutable payout order.
9. `payouts + fees <= valid contributions`
10. No arbitrary withdrawal, by any actor — creator, admin, backend, indexer, or
    treasury. Funds only ever leave via `execute_payout`, to the round recipient and the
    treasury fee. Nothing is retained by the Group contract at completion.
11. The final round completes exactly once.

## Storage & TTL

Instance storage holds configuration and current state and is extended on every mutating
entry point; persistent storage holds membership and per-round contributions and is
extended when the specific key is touched.

**Group — instance**

| Key | Type | Notes |
|---|---|---|
| `Config` | `GroupConfig` | Immutable after construction |
| `Status` | `Status` | |
| `CurrentRound` | `u32` | 1-based; `0` before start |
| `RoundPhase` | `RoundPhase` | |
| `MemberCount` | `u32` | |

**Group — persistent**

| Key | Type | Notes |
|---|---|---|
| `Member(Address)` | `u32` | Position in the payout order |
| `MemberAt(u32)` | `Address` | Inverse of `Member` |
| `RoundPool(u32)` | `i128` | Total validated contributions for the round |
| `RoundContributionCount(u32)` | `u32` | Members that contributed |
| `Contribution(RoundMember)` | `i128` | A member's contribution for a round; presence prevents a second |
| `PayoutExecuted(u32)` | `bool` | One key per round |

There is no dynamic or user-controlled storage key.

**Factory — instance**

| Key | Type | Notes |
|---|---|---|
| `Config` | `FactoryConfig` | Admin, treasury, fee, wasm hash, paused |
| `GroupCount` | `u32` | Number of groups created |
| `Group(u32)` | `Address` | group id → deployed address |

Both thresholds are `100_000` ledgers and extend to `518_400` ledgers (~30 days at
5-second ledgers). Extension is best-effort and never gates an entry point's success:
the Factory extends its instance entry on every mutating call **and** on the read
paths (`get_config`, `get_group_count`), and extends a group's persistent entry when
`get_group` reads it. No entry point unwraps `Config`; a missing configuration is the
typed `NotInitialized` error instead.

## Events

All events use the `["susu", "<name>"]` topic prefix so the indexer can key on them
deterministically. Address-bearing fields are marked `#[topic]` where the indexer needs
to filter by them.

**Factory**

| Event | Topics | Data |
|---|---|---|
| `GroupCreated` | `susu`, `group_created`, `creator`, `group` | `group_id`, `token`, `contribution_amount`, `member_capacity`, `fee_bps`, `treasury`, `frequency_seconds` |
| `FeeUpdated` | `susu`, `fee_updated` | `previous_fee_bps`, `fee_bps` |
| `TreasuryUpdated` | `susu`, `treasury_updated`, `treasury` | `previous_treasury` |
| `PauseUpdated` | `susu`, `pause_updated` | `paused` |

**Group**

| Event | Topics | Data |
|---|---|---|
| `GroupInitialized` | `susu`, `group_initialized` | `factory`, `creator`, `token`, `treasury`, `contribution_amount`, `member_capacity`, `frequency_seconds`, `fee_bps` |
| `MemberJoined` | `susu`, `join` | `member`, `position` |
| `GroupStarted` | `susu`, `start` | `member_count` |
| `ContributionReceived` | `susu`, `contribution` | `member`, `round`, `amount` |
| `PayoutExecuted` | `susu`, `payout` | `recipient`, `round`, `recipient_amount` |
| `FeePaid` | `susu`, `fee` | `treasury`, `round`, `fee` |
| `GroupCompleted` | `susu`, `completed` | `rounds` |

Event shapes are stable, idempotently indexable identities — the indexer deduplicates on
chain-derived event identity (ledger, transaction, and event index).

## Testing

`cargo test --workspace` covers construction and validation boundaries, authorization,
duplicate and capacity rules, round transitions, the WAIT-on-miss path, payout split
exactness across many pools and fee values, the final round, storage isolation from stray
transfers, and TTL extension.

The Factory additionally has a deployment integration test
(`contracts/factory/tests/deploy_group.rs`, gated behind the `wasm-integration` feature)
that loads the compiled Group Wasm, deploys it through `create_group`, and runs a full
three-member, three-round lifecycle with real token balances. Run it with:

```bash
./scripts/build-contracts.sh
cargo test -p susu-factory --features wasm-integration
```

CI builds the Wasm and runs this test, so it is enforced rather than silently skipped.
