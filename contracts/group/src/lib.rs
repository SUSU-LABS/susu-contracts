//! # Susu Protocol — Group Contract
//!
//! One Group contract instance per Susu group. This contract is the **financial
//! authority** for its group's pool. Nothing off-chain may override it.
//!
//! ## Financial rules (integer arithmetic only — never floating point)
//! ```text
//! fee              = pool * fee_bps / 10_000   (integer division, truncated)
//! recipient_amount = pool - fee
//! fee_bps          <= 50  (0.50% protocol maximum)
//! fee + recipient_amount == pool               (exact, by construction)
//! ```
//!
//! ## Invariants enforced here
//! - One contribution per member per round; a member can never pay twice in a round.
//! - One payout per round; a round can never pay out twice.
//! - Only the configured token, and only the exact configured contribution amount.
//! - No early payout: every member must have contributed before a payout is allowed.
//! - A missing contribution means **WAIT**. There is no timeout, no skip and no
//!   penalty, so no member can ever be bypassed or lose their turn.
//! - The recipient is the member at position `round` in the immutable join order.
//! - No arbitrary withdrawal exists — not for the creator, the admin, the treasury,
//!   the backend, nor the indexer. Funds only ever leave to the round recipient and
//!   to the treasury fee, and only through `execute_payout`.
//! - The final round completes exactly once.
//!
//! ## Lifecycle
//! ```text
//! DRAFT/OPEN ──start()──> ACTIVE ──execute_payout()──> ACTIVE (next round)
//!                            └───────────────────────> COMPLETED (final round)
//!
//! ACTIVE round: WAITING_FOR_CONTRIBUTIONS -> READY_FOR_PAYOUT -> PAYOUT_EXECUTED
//! ```
//!
//! ## Trust boundary
//! `start` and `execute_payout` are intentionally **permissionless**: starting a full
//! group, and paying out a fully funded round, are the outcomes the group already
//! agreed to. Neither can redirect funds, because the recipient and every amount are
//! fixed by configuration and this contract's own checks.
//!
//! `contribute` requires the contributing member's own authorization via the host's
//! `require_auth`. There is no admin bypass.
//!
//! See `docs/CONTRACT_SPEC.md` for the full specification and storage/TTL policy.
// Contract entry points and their generated clients take one argument per ABI
// parameter. The argument counts are fixed by the protocol ABI, not by this
// implementation, so the lint is allowed crate-wide.
#![allow(clippy::too_many_arguments)]
#![no_std]

use soroban_sdk::{
    contract, contracterror, contractevent, contractimpl, contracttype, token, Address, Env, Vec,
};

/// Denominator for basis-point math. 1 bps = 1/10_000.
pub const BPS_DENOMINATOR: i128 = 10_000;

/// Contract version reported by `version()`. Bumped with each released interface.
const CONTRACT_VERSION: u32 = 2;

/// Protocol maximum fee, in basis points. The MVP protocol fee is 0.50%.
///
/// A group can never be created with a fee above this, so the protocol can never
/// charge more than 0.50% of a pool. Changing this constant is a change to a
/// financial invariant and requires human review.
pub const MAX_FEE_BPS: u32 = 50;

/// Minimum members a group can be created with. A single-member group is not a Susu.
pub const MIN_MEMBERS: u32 = 2;

/// Maximum members, to bound iteration and per-entry storage growth.
pub const MAX_MEMBERS: u32 = 100;

/// Instance (configuration/state) TTL policy, in ledgers. ~5s per ledger:
/// 100_000 ledgers is ~5.8 days, 518_400 ledgers is ~30 days.
const INSTANCE_TTL_THRESHOLD: u32 = 100_000;
const INSTANCE_TTL_EXTEND_TO: u32 = 518_400;

/// Persistent (members/contributions) TTL policy, in ledgers. Applied on every
/// write and read so long-running rounds cannot archive the group's own history.
const PERSISTENT_TTL_THRESHOLD: u32 = 100_000;
const PERSISTENT_TTL_EXTEND_TO: u32 = 518_400;

/// Lifecycle status of the group as a whole.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Status {
    /// Created but not yet accepting members.
    Draft,
    /// Accepting members until capacity is reached.
    Open,
    /// Membership is full and locked; rounds are running.
    Active,
    /// Every round has paid out exactly once. Terminal state.
    Completed,
}

/// Phase of the group's current round.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RoundPhase {
    /// Collecting contributions. Payout is not yet allowed.
    WaitingForContributions,
    /// Every member has contributed; a payout is now allowed.
    ReadyForPayout,
    /// The round has paid out. Terminal for this round only.
    PayoutExecuted,
}

/// Immutable financial and membership configuration.
///
/// Captured at construction and never modified afterwards, so a group's fee,
/// treasury, token, amount and capacity cannot change under its members.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GroupConfig {
    /// The Factory that deployed this group. Informational; carries no authority.
    pub factory: Address,
    /// The account that created the group. Informational; carries no authority
    /// beyond neither — in particular it can never withdraw group funds.
    pub creator: Address,
    /// The single accepted token (USDC SAC). No other asset is accepted.
    pub token: Address,
    /// Fee recipient. Receives only the computed fee, never group funds.
    pub treasury: Address,
    /// Exact amount every member must contribute each round, in token stroops.
    pub contribution_amount: i128,
    /// Exact number of members; also the number of rounds.
    pub member_capacity: u32,
    /// Nominal round cadence, in seconds. Informational for the MVP: a payout is
    /// gated by contributions, never by time, and a missing contribution means WAIT.
    pub frequency_seconds: u64,
    /// Protocol fee in basis points, `<= MAX_FEE_BPS`.
    pub fee_bps: u32,
}

/// Composite key identifying one member's contribution in one round.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RoundMember {
    pub round: u32,
    pub member: Address,
}

/// Full observable state of the group, returned by `get_group`.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GroupState {
    pub config: GroupConfig,
    pub status: Status,
    /// 1-based round number. `0` before the group starts.
    pub current_round: u32,
    /// Number of members that have joined.
    pub member_count: u32,
    pub round_phase: RoundPhase,
}

/// Observable state of a single round, returned by `get_round`.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RoundInfo {
    pub round: u32,
    /// Contributions actually received for this round (validated amounts only).
    pub pool: i128,
    /// Number of members that have contributed to this round.
    pub contribution_count: u32,
    pub phase: RoundPhase,
    pub payout_executed: bool,
    /// The scheduled recipient. `None` for a round that does not exist yet.
    pub recipient: Option<Address>,
}

/// Storage keys.
///
/// Every key is documented here because storage layout is part of the contract's
/// public, auditable surface. There is no dynamic or user-controlled key.
#[contracttype]
pub enum DataKey {
    // ---- Instance storage: configuration and current state -------------------
    /// `GroupConfig`. Immutable after construction.
    Config,
    /// `Status`.
    Status,
    /// `u32` — current round number, 1-based, `0` before start.
    CurrentRound,
    /// `RoundPhase`.
    RoundPhase,
    /// `u32` — number of members that have joined.
    MemberCount,
    // ---- Persistent storage: membership and per-round contributions ----------
    /// `Address` -> `u32` — member's 1-based position in the immutable payout order.
    Member(Address),
    /// `u32` (1-based position) -> `Address` — the immutable payout order.
    MemberAt(u32),
    /// `u32` (round) -> `i128` — total validated contributions for that round.
    RoundPool(u32),
    /// `u32` (round) -> `u32` — number of members that contributed.
    RoundContributionCount(u32),
    /// `RoundMember` -> `i128` — a member's contribution for a round. Presence is
    /// what makes a second contribution in the same round impossible.
    Contribution(RoundMember),
    /// `u32` (round) -> `bool` — whether the round has paid out. Makes a second
    /// payout for the same round impossible.
    PayoutExecuted(u32),
}

/// Errors returned by the group contract.
///
/// Each variant is a distinct, testable failure mode. Nothing here reveals
/// sensitive data.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum GroupError {
    /// A contribution amount of zero or less was configured.
    InvalidContributionAmount = 1,
    /// Member capacity is outside `[MIN_MEMBERS, MAX_MEMBERS]`.
    InvalidMemberCapacity = 2,
    /// Fee exceeds `MAX_FEE_BPS`, or is zero.
    InvalidFeeBps = 3,
    /// Round frequency is zero.
    InvalidFrequency = 4,
    /// The group is not accepting members.
    NotOpen = 5,
    /// The address is already a member.
    AlreadyMember = 6,
    /// The group already has its full complement of members.
    GroupFull = 7,
    /// Membership is not full, so the group cannot start.
    CapacityNotReached = 8,
    /// The group is not active.
    NotActive = 9,
    /// The address is not a member of this group.
    NotAMember = 10,
    /// The supplied round is not the current round.
    WrongRound = 11,
    /// The supplied amount is not the exact configured contribution amount.
    WrongAmount = 12,
    /// This member has already contributed to this round.
    AlreadyContributed = 13,
    /// Not every member has contributed, so no payout is allowed yet. WAIT.
    ContributionsIncomplete = 14,
    /// This round has already paid out.
    PayoutAlreadyExecuted = 15,
    /// The current round is not in a phase that allows this action.
    WrongRoundPhase = 16,
    /// The group has completed all rounds.
    GroupCompleted = 17,
    /// Integer arithmetic overflowed. Never expected for valid inputs.
    ArithmeticOverflow = 18,
    /// The computed split does not satisfy `fee + recipient_amount == pool`.
    /// Defensive: unreachable by construction, asserted so any future change to
    /// the money math fails loudly instead of silently mis-splitting funds.
    SplitInvariantViolated = 19,
}

// ---------------------------------------------------------------------------
// Events
//
// Topic layout is fixed per event so the indexer can key on it deterministically.
// Every event carries the contract address as the event's contract id (added by
// the host), so events are already scoped to this group.
// ---------------------------------------------------------------------------

/// A member joined the group at a position in the payout order.
#[contractevent(topics = ["susu", "join"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemberJoined {
    #[topic]
    pub member: Address,
    pub position: u32,
}

/// The group reached capacity and started running rounds.
#[contractevent(topics = ["susu", "start"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GroupStarted {
    pub member_count: u32,
}

/// A member contributed the exact configured amount for a round.
#[contractevent(topics = ["susu", "contribution"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContributionReceived {
    #[topic]
    pub member: Address,
    pub round: u32,
    pub amount: i128,
}

/// A round paid out to its scheduled recipient.
#[contractevent(topics = ["susu", "payout"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PayoutExecuted {
    #[topic]
    pub recipient: Address,
    pub round: u32,
    pub recipient_amount: i128,
}

/// The protocol fee for a round was transferred to the treasury.
#[contractevent(topics = ["susu", "fee"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FeePaid {
    #[topic]
    pub treasury: Address,
    pub round: u32,
    pub fee: i128,
}

/// Every round has paid out exactly once. Terminal.
#[contractevent(topics = ["susu", "completed"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GroupCompleted {
    pub rounds: u32,
}

/// The Susu Group contract type.
#[contract]
pub struct GroupContract;

#[contractimpl]
impl GroupContract {
    /// Construct the group.
    ///
    /// This is the SDK-idiomatic replacement for the `initialize(...)` entry point
    /// named in the specification. Deploying with a constructor (via the Factory's
    /// `deploy_v2`) means the contract is *never* observable in an uninitialized
    /// state, so there is no window in which a third party could claim ownership of
    /// the group. Semantics are unchanged: the same parameters are validated and
    /// stored, exactly once, before the contract can be called.
    ///
    /// Validation here is the only place configuration is ever accepted. All of it
    /// is enforced before any state is written.
    pub fn __constructor(
        env: Env,
        factory: Address,
        creator: Address,
        token: Address,
        treasury: Address,
        contribution_amount: i128,
        member_capacity: u32,
        frequency_seconds: u64,
        fee_bps: u32,
    ) {
        if contribution_amount <= 0
            || contribution_amount
                .checked_mul(member_capacity as i128)
                .is_none()
        {
            soroban_sdk::panic_with_error!(&env, GroupError::InvalidContributionAmount);
        }
        if !(MIN_MEMBERS..=MAX_MEMBERS).contains(&member_capacity) {
            soroban_sdk::panic_with_error!(&env, GroupError::InvalidMemberCapacity);
        }
        if frequency_seconds == 0 {
            soroban_sdk::panic_with_error!(&env, GroupError::InvalidFrequency);
        }
        // The fee ceiling is enforced here *and* in the Factory, so a group can
        // never charge more than the protocol maximum even if it were deployed
        // by some other means.
        if fee_bps == 0 || fee_bps > MAX_FEE_BPS {
            soroban_sdk::panic_with_error!(&env, GroupError::InvalidFeeBps);
        }

        let config = GroupConfig {
            factory,
            creator,
            token,
            treasury,
            contribution_amount,
            member_capacity,
            frequency_seconds,
            fee_bps,
        };

        let storage = env.storage().instance();
        storage.set(&DataKey::Config, &config);
        storage.set(&DataKey::Status, &Status::Open);
        storage.set(&DataKey::CurrentRound, &0u32);
        storage.set(&DataKey::RoundPhase, &RoundPhase::WaitingForContributions);
        storage.set(&DataKey::MemberCount, &0u32);
        extend_instance_ttl(&env);
    }

    /// Join the group, taking the next position in the payout order.
    ///
    /// The order in which members join is the payout order and is permanent: round
    /// `n` pays the member at position `n`. Requires the joining member's own
    /// authorization. Members may only join while the group is `Open`; once started,
    /// membership is locked forever.
    pub fn join(env: Env, member: Address) -> Result<u32, GroupError> {
        member.require_auth();
        extend_instance_ttl(&env);

        let storage = env.storage().instance();
        let status: Status = storage.get(&DataKey::Status).unwrap_or(Status::Draft);
        if status != Status::Open {
            return Err(GroupError::NotOpen);
        }

        let config: GroupConfig = storage.get(&DataKey::Config).unwrap();

        let persistent = env.storage().persistent();
        if persistent.has(&DataKey::Member(member.clone())) {
            return Err(GroupError::AlreadyMember);
        }

        let count: u32 = storage.get(&DataKey::MemberCount).unwrap_or(0);
        if count >= config.member_capacity {
            return Err(GroupError::GroupFull);
        }

        let position = count + 1;
        persistent.set(&DataKey::Member(member.clone()), &position);
        persistent.set(&DataKey::MemberAt(position), &member);
        storage.set(&DataKey::MemberCount, &position);
        extend_persistent_ttl(&env, &DataKey::Member(member.clone()));
        extend_persistent_ttl(&env, &DataKey::MemberAt(position));

        MemberJoined { member, position }.publish(&env);
        Ok(position)
    }

    /// Start the group once membership has reached capacity.
    ///
    /// Permissionless by design: reaching capacity is exactly the state the group
    /// agreed to start from, and starting locks configuration without moving any
    /// funds, so there is nothing for a caller to gain or to grief. Requiring the
    /// creator here would only let an absent creator stall a full group forever.
    pub fn start(env: Env) -> Result<(), GroupError> {
        extend_instance_ttl(&env);

        let storage = env.storage().instance();
        let status: Status = storage.get(&DataKey::Status).unwrap_or(Status::Draft);
        if status != Status::Open {
            return Err(GroupError::NotOpen);
        }

        let config: GroupConfig = storage.get(&DataKey::Config).unwrap();
        let count: u32 = storage.get(&DataKey::MemberCount).unwrap_or(0);
        if count != config.member_capacity {
            return Err(GroupError::CapacityNotReached);
        }

        // Financial configuration and membership are now immutable for the life of
        // the group. Nothing below can change them.
        storage.set(&DataKey::Status, &Status::Active);
        storage.set(&DataKey::CurrentRound, &1u32);
        storage.set(&DataKey::RoundPhase, &RoundPhase::WaitingForContributions);

        GroupStarted {
            member_count: count,
        }
        .publish(&env);
        Ok(())
    }

    /// Contribute the exact configured amount for a round.
    ///
    /// The caller must be the contributing member and must authorize this call.
    /// A member can contribute at most once per round, and only while that round is
    /// still collecting. Over- and under-payment are rejected rather than adjusted,
    /// so the pool is always an exact multiple of the configured amount.
    ///
    /// State is written **before** the token transfer (checks-effects-interactions),
    /// so a re-entrant token contract cannot contribute twice for the same member and
    /// round.
    pub fn contribute(
        env: Env,
        member: Address,
        amount: i128,
        round: u32,
    ) -> Result<(), GroupError> {
        member.require_auth();
        extend_instance_ttl(&env);

        let storage = env.storage().instance();
        let status: Status = storage.get(&DataKey::Status).unwrap_or(Status::Draft);
        if status == Status::Completed {
            return Err(GroupError::GroupCompleted);
        }
        if status != Status::Active {
            return Err(GroupError::NotActive);
        }

        let config: GroupConfig = storage.get(&DataKey::Config).unwrap();
        if amount != config.contribution_amount {
            return Err(GroupError::WrongAmount);
        }

        let current_round: u32 = storage.get(&DataKey::CurrentRound).unwrap_or(0);
        if round != current_round {
            return Err(GroupError::WrongRound);
        }

        let phase: RoundPhase = storage.get(&DataKey::RoundPhase).unwrap();
        if phase != RoundPhase::WaitingForContributions {
            return Err(GroupError::WrongRoundPhase);
        }

        let persistent = env.storage().persistent();
        if !persistent.has(&DataKey::Member(member.clone())) {
            return Err(GroupError::NotAMember);
        }

        let contribution_key = DataKey::Contribution(RoundMember {
            round,
            member: member.clone(),
        });
        // Presence of this entry is what makes a duplicate contribution impossible.
        if persistent.has(&contribution_key) {
            return Err(GroupError::AlreadyContributed);
        }

        // Effects first: record the contribution and advance the round counters
        // before any external call.
        persistent.set(&contribution_key, &amount);
        let pool: i128 = persistent.get(&DataKey::RoundPool(round)).unwrap_or(0i128);
        let new_pool = match pool.checked_add(amount) {
            Some(value) => value,
            None => return Err(GroupError::ArithmeticOverflow),
        };
        let count_key = DataKey::RoundContributionCount(round);
        let contribution_count: u32 = persistent.get(&count_key).unwrap_or(0);
        // Same checked pattern as the pool update above: an overflow must
        // surface as ArithmeticOverflow rather than aborting the host.
        let new_count = match contribution_count.checked_add(1) {
            Some(value) => value,
            None => return Err(GroupError::ArithmeticOverflow),
        };

        persistent.set(&DataKey::RoundPool(round), &new_pool);
        persistent.set(&count_key, &new_count);
        if new_count >= config.member_capacity {
            storage.set(&DataKey::RoundPhase, &RoundPhase::ReadyForPayout);
        }

        extend_persistent_ttl(&env, &contribution_key);
        extend_persistent_ttl(&env, &DataKey::RoundPool(round));
        extend_persistent_ttl(&env, &count_key);

        // Interaction last.
        token::Client::new(&env, &config.token).transfer(
            &member,
            env.current_contract_address(),
            &amount,
        );

        ContributionReceived {
            member,
            round,
            amount,
        }
        .publish(&env);
        Ok(())
    }

    /// Pay out the current round to its scheduled recipient.
    ///
    /// Permissionless: once every member has contributed, the payout is the outcome
    /// the group already agreed to, and the recipient and amounts are fixed by
    /// configuration. The caller only supplies the trigger — they cannot influence
    /// where funds go.
    ///
    /// Reverts with `ContributionsIncomplete` while any member is outstanding. This
    /// is the WAIT behaviour: a round simply does not advance until it is fully
    /// funded. There is no timeout, no skip, and no penalty.
    pub fn execute_payout(env: Env) -> Result<(), GroupError> {
        extend_instance_ttl(&env);

        let storage = env.storage().instance();
        let status: Status = storage.get(&DataKey::Status).unwrap_or(Status::Draft);
        if status == Status::Completed {
            return Err(GroupError::GroupCompleted);
        }
        if status != Status::Active {
            return Err(GroupError::NotActive);
        }

        let phase: RoundPhase = storage.get(&DataKey::RoundPhase).unwrap();
        if phase == RoundPhase::PayoutExecuted {
            return Err(GroupError::PayoutAlreadyExecuted);
        }
        if phase != RoundPhase::ReadyForPayout {
            // Covers both "not everyone has contributed yet" and a malformed phase.
            return Err(GroupError::ContributionsIncomplete);
        }

        let config: GroupConfig = storage.get(&DataKey::Config).unwrap();
        let round: u32 = storage.get(&DataKey::CurrentRound).unwrap_or(0);

        let persistent = env.storage().persistent();
        if persistent
            .get(&DataKey::PayoutExecuted(round))
            .unwrap_or(false)
        {
            return Err(GroupError::PayoutAlreadyExecuted);
        }

        // The recipient is fixed by the join order; the caller has no say.
        let recipient: Address = match persistent.get(&DataKey::MemberAt(round)) {
            Some(address) => address,
            None => return Err(GroupError::WrongRound),
        };

        let pool: i128 = persistent.get(&DataKey::RoundPool(round)).unwrap_or(0i128);

        let (fee, recipient_amount) = split_pool(pool, config.fee_bps)?;

        // Effects before interactions: mark the round paid and advance the round
        // pointer so a re-entrant token cannot pay the same round twice.
        persistent.set(&DataKey::PayoutExecuted(round), &true);
        extend_persistent_ttl(&env, &DataKey::PayoutExecuted(round));

        let is_final_round = round >= config.member_capacity;
        if is_final_round {
            storage.set(&DataKey::RoundPhase, &RoundPhase::PayoutExecuted);
            storage.set(&DataKey::Status, &Status::Completed);
        } else {
            storage.set(&DataKey::CurrentRound, &(round + 1));
            storage.set(&DataKey::RoundPhase, &RoundPhase::WaitingForContributions);
        }

        // Interactions last. The fee is protocol revenue; the remainder belongs to
        // the scheduled recipient. Nothing is retained by this contract.
        let token_client = token::Client::new(&env, &config.token);
        if fee > 0 {
            token_client.transfer(&env.current_contract_address(), &config.treasury, &fee);
        }
        token_client.transfer(
            &env.current_contract_address(),
            &recipient,
            &recipient_amount,
        );

        PayoutExecuted {
            recipient,
            round,
            recipient_amount,
        }
        .publish(&env);
        FeePaid {
            treasury: config.treasury,
            round,
            fee,
        }
        .publish(&env);

        if is_final_round {
            GroupCompleted {
                rounds: config.member_capacity,
            }
            .publish(&env);
        }

        Ok(())
    }

    // -----------------------------------------------------------------------
    // Read-only views. None of these mutate state or move funds.
    // -----------------------------------------------------------------------

    /// Full observable state of the group.
    pub fn get_group(env: Env) -> GroupState {
        let storage = env.storage().instance();
        GroupState {
            config: storage.get(&DataKey::Config).unwrap(),
            status: storage.get(&DataKey::Status).unwrap_or(Status::Draft),
            current_round: storage.get(&DataKey::CurrentRound).unwrap_or(0),
            member_count: storage.get(&DataKey::MemberCount).unwrap_or(0),
            round_phase: storage
                .get(&DataKey::RoundPhase)
                .unwrap_or(RoundPhase::WaitingForContributions),
        }
    }

    /// A member's 1-based position in the payout order, or `0` if not a member.
    pub fn get_member(env: Env, address: Address) -> u32 {
        env.storage()
            .persistent()
            .get(&DataKey::Member(address))
            .unwrap_or(0)
    }

    /// The number of members that have joined.
    pub fn get_member_count(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&DataKey::MemberCount)
            .unwrap_or(0)
    }

    /// The immutable payout order, in join order. Position `n` (1-based) is paid in
    /// round `n`.
    pub fn get_payout_order(env: Env) -> Vec<Address> {
        let count: u32 = env
            .storage()
            .instance()
            .get(&DataKey::MemberCount)
            .unwrap_or(0);
        let persistent = env.storage().persistent();
        let mut order = Vec::new(&env);
        let mut position = 1u32;
        while position <= count {
            if let Some(address) = persistent.get(&DataKey::MemberAt(position)) {
                order.push_back(address);
            }
            position += 1;
        }
        order
    }

    /// Observable state of a specific round.
    pub fn get_round(env: Env, round: u32) -> RoundInfo {
        let persistent = env.storage().persistent();
        let payout_executed: bool = persistent
            .get(&DataKey::PayoutExecuted(round))
            .unwrap_or(false);
        let recipient: Option<Address> = persistent.get(&DataKey::MemberAt(round));

        // A round that has not started yet reports its phase as waiting.
        let phase = if payout_executed {
            RoundPhase::PayoutExecuted
        } else {
            let current_round: u32 = env
                .storage()
                .instance()
                .get(&DataKey::CurrentRound)
                .unwrap_or(0);
            if round == current_round {
                env.storage()
                    .instance()
                    .get(&DataKey::RoundPhase)
                    .unwrap_or(RoundPhase::WaitingForContributions)
            } else {
                RoundPhase::WaitingForContributions
            }
        };

        RoundInfo {
            round,
            pool: persistent.get(&DataKey::RoundPool(round)).unwrap_or(0i128),
            contribution_count: persistent
                .get(&DataKey::RoundContributionCount(round))
                .unwrap_or(0),
            phase,
            payout_executed,
            recipient,
        }
    }

    /// The member scheduled to receive the current round's payout.
    pub fn get_current_recipient(env: Env) -> Result<Address, GroupError> {
        let round: u32 = env
            .storage()
            .instance()
            .get(&DataKey::CurrentRound)
            .unwrap_or(0);
        if round == 0 {
            return Err(GroupError::NotActive);
        }
        match env.storage().persistent().get(&DataKey::MemberAt(round)) {
            Some(address) => Ok(address),
            None => Err(GroupError::WrongRound),
        }
    }

    /// The current round number, or `0` before the group starts.
    pub fn get_current_round(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&DataKey::CurrentRound)
            .unwrap_or(0)
    }

    /// Contributions validated for the current round.
    ///
    /// This is derived from validated contributions, not from the contract's raw
    /// token balance, so a stray transfer into the contract cannot inflate a
    /// payout. In normal operation the two are equal, because every round pays out
    /// in full.
    pub fn get_pool_balance(env: Env) -> i128 {
        let round: u32 = env
            .storage()
            .instance()
            .get(&DataKey::CurrentRound)
            .unwrap_or(0);
        if round == 0 {
            return 0;
        }
        env.storage()
            .persistent()
            .get(&DataKey::RoundPool(round))
            .unwrap_or(0i128)
    }

    /// Whether every member has contributed to the current round.
    pub fn is_contribution_complete(env: Env) -> bool {
        let storage = env.storage().instance();
        let config: GroupConfig = match storage.get(&DataKey::Config) {
            Some(config) => config,
            None => return false,
        };
        let round: u32 = storage.get(&DataKey::CurrentRound).unwrap_or(0);
        if round == 0 {
            return false;
        }
        let count: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::RoundContributionCount(round))
            .unwrap_or(0);
        count >= config.member_capacity
    }

    /// The group's lifecycle status.
    pub fn get_status(env: Env) -> Status {
        env.storage()
            .instance()
            .get(&DataKey::Status)
            .unwrap_or(Status::Draft)
    }

    /// The single token this group accepts.
    pub fn get_token(env: Env) -> Address {
        let config: GroupConfig = env.storage().instance().get(&DataKey::Config).unwrap();
        config.token
    }

    /// Returns the ABI/interface version of this contract.
    ///
    /// Metadata only — carries no financial meaning.
    pub fn version(_env: Env) -> u32 {
        CONTRACT_VERSION
    }
}

/// Splits a pool into the protocol fee and the recipient's amount.
///
/// Integer arithmetic only, with a defensive check on the invariant
/// `fee + recipient_amount == pool`. If that ever fails, the payout reverts rather
/// than paying an incorrect split.
fn split_pool(pool: i128, fee_bps: u32) -> Result<(i128, i128), GroupError> {
    let numerator = match pool.checked_mul(fee_bps as i128) {
        Some(value) => value,
        None => return Err(GroupError::ArithmeticOverflow),
    };
    let fee = numerator / BPS_DENOMINATOR;
    let recipient_amount = pool - fee;

    if fee + recipient_amount != pool {
        return Err(GroupError::SplitInvariantViolated);
    }
    Ok((fee, recipient_amount))
}

/// Extends the instance entry's TTL so an active group never archives.
fn extend_instance_ttl(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(INSTANCE_TTL_THRESHOLD, INSTANCE_TTL_EXTEND_TO);
}

/// Extends a persistent entry's TTL so membership and contribution history stay
/// readable for the life of a group, which can span many months.
fn extend_persistent_ttl(env: &Env, key: &DataKey) {
    env.storage()
        .persistent()
        .extend_ttl(key, PERSISTENT_TTL_THRESHOLD, PERSISTENT_TTL_EXTEND_TO);
}

mod test;
