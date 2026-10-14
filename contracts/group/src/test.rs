#![cfg(test)]

//! Tests for the Susu Group contract.
//!
//! Structure mirrors the specification's testing requirements (v3 §25.1, §25.2):
//! happy paths, every negative path, boundary values, the final round, and the
//! financial invariants. The canonical financial case is 3 members × 10 USDC:
//! pool 30 USDC, fee 0.15 USDC, recipient 29.85 USDC.
//!
//! Money is expressed in the token's smallest unit. USDC has 7 decimal places, so
//! 1 USDC = 10_000_000.

use super::*;
use soroban_sdk::{
    testutils::{Address as _, Events as _},
    token, Event, Vec,
};

/// 1 USDC in stroops (7 decimals).
const ONE_USDC: i128 = 10_000_000;

/// A week, in seconds. Informational on-chain; payouts are gated by contributions.
const ONE_WEEK: u64 = 604_800;

/// Test fixture: a registered group with a real Stellar Asset Contract token and
/// funded members.
struct Setup {
    env: Env,
    group_id: Address,
    token: Address,
    treasury: Address,
    members: Vec<Address>,
    amount: i128,
    capacity: u32,
}

impl Setup {
    fn client(&self) -> GroupContractClient<'_> {
        GroupContractClient::new(&self.env, &self.group_id)
    }

    fn member(&self, index: u32) -> Address {
        self.members.get(index).unwrap()
    }

    fn token_client(&self) -> token::Client<'_> {
        token::Client::new(&self.env, &self.token)
    }

    /// Joins every member, in order, producing the payout order.
    fn join_all(&self) {
        let client = self.client();
        let mut index = 0;
        while index < self.capacity {
            client.join(&self.member(index));
            index += 1;
        }
    }

    /// Contributes for every member in the given round.
    fn contribute_all(&self, round: u32) {
        let client = self.client();
        let mut index = 0;
        while index < self.capacity {
            client.contribute(&self.member(index), &self.amount, &round);
            index += 1;
        }
    }

    /// Runs the group from a fresh state through every round.
    fn run_to_completion(&self) {
        self.join_all();
        self.client().start();
        let mut round = 1;
        while round <= self.capacity {
            self.contribute_all(round);
            self.client().execute_payout();
            round += 1;
        }
    }
}

/// Builds a group with `capacity` funded members.
fn setup(capacity: u32, amount: i128, fee_bps: u32) -> Setup {
    let env = Env::default();
    env.mock_all_auths();

    let treasury = Address::generate(&env);
    let token_admin = Address::generate(&env);
    let token = env
        .register_stellar_asset_contract_v2(token_admin)
        .address();

    let group_id = env.register(
        GroupContract,
        (
            Address::generate(&env), // factory (informational)
            Address::generate(&env), // creator (informational, no authority)
            token.clone(),
            treasury.clone(),
            amount,
            capacity,
            ONE_WEEK,
            fee_bps,
        ),
    );

    // Fund every member well beyond their total obligation.
    let mut members = Vec::new(&env);
    let asset_client = token::StellarAssetClient::new(&env, &token);
    let mut index = 0;
    while index < capacity {
        let member = Address::generate(&env);
        asset_client.mint(&member, &(amount * (capacity as i128 + 1)));
        members.push_back(member);
        index += 1;
    }

    Setup {
        env,
        group_id,
        token,
        treasury,
        members,
        amount,
        capacity,
    }
}

/// A group that has joined all members and started.
fn setup_started(capacity: u32, amount: i128, fee_bps: u32) -> Setup {
    let setup = setup(capacity, amount, fee_bps);
    setup.join_all();
    setup.client().start();
    setup
}

// ---------------------------------------------------------------------------
// Construction and configuration
// ---------------------------------------------------------------------------

#[test]
fn constructor_stores_configuration_and_opens_the_group() {
    let setup = setup(3, 10 * ONE_USDC, MAX_FEE_BPS);
    let client = setup.client();
    let state = client.get_group();

    assert_eq!(state.config.token, setup.token);
    assert_eq!(state.config.treasury, setup.treasury);
    assert_eq!(state.config.contribution_amount, 10 * ONE_USDC);
    assert_eq!(state.config.member_capacity, 3);
    assert_eq!(state.config.frequency_seconds, ONE_WEEK);
    assert_eq!(state.config.fee_bps, MAX_FEE_BPS);
    assert_eq!(state.status, Status::Open);
    assert_eq!(state.current_round, 0);
    assert_eq!(state.member_count, 0);
    assert_eq!(client.get_token(), setup.token);
    assert_eq!(client.version(), 2);
}

#[test]
fn constructor_publishes_the_initial_configuration() {
    let env = Env::default();
    env.mock_all_auths();

    let factory = Address::generate(&env);
    let creator = Address::generate(&env);
    let token = env
        .register_stellar_asset_contract_v2(Address::generate(&env))
        .address();
    let treasury = Address::generate(&env);
    let amount = 10 * ONE_USDC;
    let capacity = 3u32;
    let fee_bps = MAX_FEE_BPS;

    let group_id = env.register(
        GroupContract,
        (
            factory.clone(),
            creator.clone(),
            token.clone(),
            treasury.clone(),
            amount,
            capacity,
            ONE_WEEK,
            fee_bps,
        ),
    );

    // The constructor is the only invocation so far, so `events().all()` is exactly
    // what it published. Assert exactly one event, carrying the full configuration,
    // so a duplicated emission would also fail.
    let emitted = env.events().all().filter_by_contract(&group_id);
    let expected = GroupInitialized {
        factory,
        creator,
        token,
        treasury,
        contribution_amount: amount,
        member_capacity: capacity,
        frequency_seconds: ONE_WEEK,
        fee_bps,
    }
    .to_xdr(&env, &group_id);

    assert_eq!(
        emitted
            .events()
            .iter()
            .filter(|event| **event == expected)
            .count(),
        1,
        "the constructor must publish exactly one GroupInitialized event"
    );
}

#[test]
#[should_panic]
fn constructor_rejects_non_positive_contribution_amount() {
    let env = Env::default();
    env.mock_all_auths();
    let token = env
        .register_stellar_asset_contract_v2(Address::generate(&env))
        .address();
    env.register(
        GroupContract,
        (
            Address::generate(&env),
            Address::generate(&env),
            token,
            Address::generate(&env),
            0i128,
            3u32,
            ONE_WEEK,
            MAX_FEE_BPS,
        ),
    );
}

#[test]
#[should_panic]
fn constructor_rejects_overflowing_pooled_contribution_amount() {
    let env = Env::default();
    env.mock_all_auths();
    let token = env
        .register_stellar_asset_contract_v2(Address::generate(&env))
        .address();
    env.register(
        GroupContract,
        (
            Address::generate(&env),
            Address::generate(&env),
            token,
            Address::generate(&env),
            i128::MAX,
            2u32,
            ONE_WEEK,
            MAX_FEE_BPS,
        ),
    );
}

#[test]
#[should_panic]
fn constructor_rejects_subtle_pooled_overflow() {
    let env = Env::default();
    env.mock_all_auths();
    let token = env
        .register_stellar_asset_contract_v2(Address::generate(&env))
        .address();
    env.register(
        GroupContract,
        (
            Address::generate(&env),
            Address::generate(&env),
            token,
            Address::generate(&env),
            (i128::MAX / 3) + 1,
            3u32,
            ONE_WEEK,
            MAX_FEE_BPS,
        ),
    );
}

#[test]
fn constructor_accepts_max_safe_pooled_contribution_amount() {
    let env = Env::default();
    env.mock_all_auths();
    let token = env
        .register_stellar_asset_contract_v2(Address::generate(&env))
        .address();
    env.register(
        GroupContract,
        (
            Address::generate(&env),
            Address::generate(&env),
            token,
            Address::generate(&env),
            i128::MAX / 3,
            3u32,
            ONE_WEEK,
            MAX_FEE_BPS,
        ),
    );
}

#[test]
#[should_panic]
fn constructor_rejects_zero_member_capacity() {
    let env = Env::default();
    env.mock_all_auths();
    let token = env
        .register_stellar_asset_contract_v2(Address::generate(&env))
        .address();
    env.register(
        GroupContract,
        (
            Address::generate(&env),
            Address::generate(&env),
            token,
            Address::generate(&env),
            ONE_USDC,
            0u32,
            ONE_WEEK,
            MAX_FEE_BPS,
        ),
    );
}

#[test]
#[should_panic]
fn constructor_rejects_a_single_member_group() {
    // A one-member group is not a rotating savings group.
    let env = Env::default();
    env.mock_all_auths();
    let token = env
        .register_stellar_asset_contract_v2(Address::generate(&env))
        .address();
    env.register(
        GroupContract,
        (
            Address::generate(&env),
            Address::generate(&env),
            token,
            Address::generate(&env),
            ONE_USDC,
            MIN_MEMBERS - 1,
            ONE_WEEK,
            MAX_FEE_BPS,
        ),
    );
}

#[test]
#[should_panic]
fn constructor_rejects_capacity_above_the_maximum() {
    let env = Env::default();
    env.mock_all_auths();
    let token = env
        .register_stellar_asset_contract_v2(Address::generate(&env))
        .address();
    env.register(
        GroupContract,
        (
            Address::generate(&env),
            Address::generate(&env),
            token,
            Address::generate(&env),
            ONE_USDC,
            MAX_MEMBERS + 1,
            ONE_WEEK,
            MAX_FEE_BPS,
        ),
    );
}

#[test]
fn constructor_accepts_the_capacity_boundaries() {
    // MIN_MEMBERS is the smallest valid group.
    let smallest = setup(MIN_MEMBERS, ONE_USDC, MAX_FEE_BPS);
    assert_eq!(
        smallest.client().get_group().config.member_capacity,
        MIN_MEMBERS
    );

    // MAX_MEMBERS is the largest valid group.
    let largest = setup(MAX_MEMBERS, ONE_USDC, MAX_FEE_BPS);
    assert_eq!(
        largest.client().get_group().config.member_capacity,
        MAX_MEMBERS
    );
}

#[test]
#[should_panic]
fn constructor_rejects_fee_above_the_protocol_maximum() {
    let env = Env::default();
    env.mock_all_auths();
    let token = env
        .register_stellar_asset_contract_v2(Address::generate(&env))
        .address();
    env.register(
        GroupContract,
        (
            Address::generate(&env),
            Address::generate(&env),
            token,
            Address::generate(&env),
            ONE_USDC,
            3u32,
            ONE_WEEK,
            MAX_FEE_BPS + 1,
        ),
    );
}

#[test]
#[should_panic]
fn constructor_rejects_zero_fee() {
    let env = Env::default();
    env.mock_all_auths();
    let token = env
        .register_stellar_asset_contract_v2(Address::generate(&env))
        .address();
    env.register(
        GroupContract,
        (
            Address::generate(&env),
            Address::generate(&env),
            token,
            Address::generate(&env),
            ONE_USDC,
            3u32,
            ONE_WEEK,
            0u32,
        ),
    );
}

#[test]
#[should_panic]
fn constructor_rejects_zero_frequency() {
    let env = Env::default();
    env.mock_all_auths();
    let token = env
        .register_stellar_asset_contract_v2(Address::generate(&env))
        .address();
    env.register(
        GroupContract,
        (
            Address::generate(&env),
            Address::generate(&env),
            token,
            Address::generate(&env),
            ONE_USDC,
            3u32,
            0u64,
            MAX_FEE_BPS,
        ),
    );
}

// ---------------------------------------------------------------------------
// Joining
// ---------------------------------------------------------------------------

#[test]
fn join_assigns_sequential_positions_and_is_emitted() {
    let setup = setup(3, 10 * ONE_USDC, MAX_FEE_BPS);
    let client = setup.client();

    assert_eq!(client.join(&setup.member(0)), 1);
    let after_first_join = setup.env.events().all();
    assert_eq!(client.join(&setup.member(1)), 2);
    // `events().all()` reflects the most recent invocation, so capture the
    // emitting call's events before making any further contract call.
    let after_second_join = setup.env.events().all();
    assert_eq!(client.join(&setup.member(2)), 3);

    assert_eq!(client.get_member(&setup.member(0)), 1);
    assert_eq!(client.get_member(&setup.member(2)), 3);
    assert_eq!(client.get_member_count(), 3);
    assert_eq!(client.get_group().status, Status::Open);

    let expected_order = Vec::from_array(
        &setup.env,
        [setup.member(0), setup.member(1), setup.member(2)],
    );
    assert_eq!(client.get_payout_order(), expected_order);

    let expected = MemberJoined {
        member: setup.member(1),
        position: 2,
    }
    .to_xdr(&setup.env, &setup.group_id);
    assert!(
        after_second_join
            .filter_by_contract(&setup.group_id)
            .events()
            .contains(&expected),
        "join must emit an indexable event"
    );

    let first_expected = MemberJoined {
        member: setup.member(0),
        position: 1,
    }
    .to_xdr(&setup.env, &setup.group_id);
    assert!(after_first_join
        .filter_by_contract(&setup.group_id)
        .events()
        .contains(&first_expected));
}

#[test]
fn join_rejects_a_duplicate_member() {
    let setup = setup(3, 10 * ONE_USDC, MAX_FEE_BPS);
    let client = setup.client();
    client.join(&setup.member(0));

    assert_eq!(
        client.try_join(&setup.member(0)),
        Err(Ok(GroupError::AlreadyMember))
    );
}

#[test]
fn non_member_is_reported_as_position_zero() {
    let setup = setup(3, 10 * ONE_USDC, MAX_FEE_BPS);
    let stranger = Address::generate(&setup.env);
    assert_eq!(setup.client().get_member(&stranger), 0);
}

#[test]
fn join_rejects_members_beyond_capacity() {
    let setup = setup(3, 10 * ONE_USDC, MAX_FEE_BPS);
    let client = setup.client();
    setup.join_all();

    let extra = Address::generate(&setup.env);
    assert_eq!(client.try_join(&extra), Err(Ok(GroupError::GroupFull)));
}

#[test]
fn join_is_rejected_once_the_group_has_started() {
    let setup = setup_started(2, 10 * ONE_USDC, MAX_FEE_BPS);
    let late = Address::generate(&setup.env);

    assert_eq!(setup.client().try_join(&late), Err(Ok(GroupError::NotOpen)));
}

#[test]
fn join_requires_the_members_own_authorization() {
    let setup = setup(3, 10 * ONE_USDC, MAX_FEE_BPS);
    let client = setup.client();
    setup.env.set_auths(&[]);

    assert!(client.try_join(&setup.member(0)).is_err());
}

// ---------------------------------------------------------------------------
// Starting
// ---------------------------------------------------------------------------

#[test]
fn start_requires_full_capacity() {
    let setup = setup(3, 10 * ONE_USDC, MAX_FEE_BPS);
    let client = setup.client();

    assert_eq!(client.try_start(), Err(Ok(GroupError::CapacityNotReached)));

    client.join(&setup.member(0));
    assert_eq!(client.try_start(), Err(Ok(GroupError::CapacityNotReached)));
}

#[test]
fn start_activates_round_one_and_emits_an_event() {
    let setup = setup(2, 10 * ONE_USDC, MAX_FEE_BPS);
    setup.join_all();
    setup.client().start();
    let emitted = setup.env.events().all().filter_by_contract(&setup.group_id);

    let state = setup.client().get_group();
    assert_eq!(state.status, Status::Active);
    assert_eq!(state.current_round, 1);
    assert_eq!(state.round_phase, RoundPhase::WaitingForContributions);

    let expected = GroupStarted { member_count: 2 }.to_xdr(&setup.env, &setup.group_id);
    assert!(emitted.events().contains(&expected));
}

#[test]
fn start_cannot_be_repeated() {
    let setup = setup_started(2, 10 * ONE_USDC, MAX_FEE_BPS);
    assert_eq!(setup.client().try_start(), Err(Ok(GroupError::NotOpen)));
}

// ---------------------------------------------------------------------------
// Contributions
// ---------------------------------------------------------------------------

#[test]
fn contribute_requires_an_active_group() {
    let setup = setup(2, 10 * ONE_USDC, MAX_FEE_BPS);
    let client = setup.client();
    setup.join_all();

    // Not started yet: contributions are not open.
    assert_eq!(
        client.try_contribute(&setup.member(0), &setup.amount, &1u32),
        Err(Ok(GroupError::NotActive))
    );
}

#[test]
fn contribute_rejects_a_wrong_amount() {
    let setup = setup_started(2, 10 * ONE_USDC, MAX_FEE_BPS);
    let client = setup.client();

    for wrong in [setup.amount - 1, setup.amount + 1, 1, 0, -setup.amount] {
        assert_eq!(
            client.try_contribute(&setup.member(0), &wrong, &1u32),
            Err(Ok(GroupError::WrongAmount)),
            "amount {wrong} must be rejected: only the exact configured amount is accepted"
        );
    }
}

#[test]
fn contribute_rejects_a_wrong_round() {
    let setup = setup_started(2, 10 * ONE_USDC, MAX_FEE_BPS);
    let client = setup.client();

    for wrong_round in [0u32, 2, 3, u32::MAX] {
        assert_eq!(
            client.try_contribute(&setup.member(0), &setup.amount, &wrong_round),
            Err(Ok(GroupError::WrongRound))
        );
    }
}

#[test]
fn contribute_rejects_a_non_member() {
    let setup = setup_started(2, 10 * ONE_USDC, MAX_FEE_BPS);
    let stranger = Address::generate(&setup.env);

    assert_eq!(
        setup
            .client()
            .try_contribute(&stranger, &setup.amount, &1u32),
        Err(Ok(GroupError::NotAMember))
    );
}

#[test]
fn contribute_rejects_a_duplicate_contribution_in_the_same_round() {
    let setup = setup_started(2, 10 * ONE_USDC, MAX_FEE_BPS);
    let client = setup.client();
    client.contribute(&setup.member(0), &setup.amount, &1u32);

    assert_eq!(
        client.try_contribute(&setup.member(0), &setup.amount, &1u32),
        Err(Ok(GroupError::AlreadyContributed)),
        "one contribution per member per round"
    );
}

#[test]
fn contribute_requires_the_members_own_authorization() {
    let setup = setup_started(2, 10 * ONE_USDC, MAX_FEE_BPS);
    let client = setup.client();
    setup.env.set_auths(&[]);

    assert!(client
        .try_contribute(&setup.member(0), &setup.amount, &1u32)
        .is_err());
}

#[test]
fn contribute_moves_funds_into_the_pool_and_tracks_completion() {
    let setup = setup_started(3, 10 * ONE_USDC, MAX_FEE_BPS);
    let client = setup.client();
    let token_client = setup.token_client();
    let before = token_client.balance(&setup.member(0));

    assert!(!client.is_contribution_complete());

    client.contribute(&setup.member(0), &setup.amount, &1u32);
    assert_eq!(
        token_client.balance(&setup.member(0)),
        before - setup.amount
    );
    assert_eq!(token_client.balance(&setup.group_id), setup.amount);
    assert_eq!(client.get_pool_balance(), 10 * ONE_USDC);
    assert!(!client.is_contribution_complete(), "2 of 3 contributed");
    assert_eq!(
        client.get_group().round_phase,
        RoundPhase::WaitingForContributions
    );

    client.contribute(&setup.member(1), &setup.amount, &1u32);
    assert!(!client.is_contribution_complete());

    client.contribute(&setup.member(2), &setup.amount, &1u32);
    let emitted = setup.env.events().all().filter_by_contract(&setup.group_id);
    assert!(client.is_contribution_complete());
    assert_eq!(client.get_group().round_phase, RoundPhase::ReadyForPayout);
    assert_eq!(client.get_pool_balance(), 30 * ONE_USDC);

    let round = client.get_round(&1u32);
    assert_eq!(round.pool, 30 * ONE_USDC);
    assert_eq!(round.contribution_count, 3);
    assert_eq!(round.recipient, Some(setup.member(0)));
    assert!(!round.payout_executed);

    let expected = ContributionReceived {
        member: setup.member(2),
        round: 1,
        amount: setup.amount,
    }
    .to_xdr(&setup.env, &setup.group_id);
    assert!(emitted.events().contains(&expected));
}

#[test]
fn contribute_fails_when_the_member_cannot_pay() {
    // Demonstrates that only the configured token is ever used: a member holding a
    // different asset has no balance in this group's token, so the transfer fails
    // and no contribution is recorded.
    let setup = setup_started(2, 10 * ONE_USDC, MAX_FEE_BPS);
    let client = setup.client();
    let payer = Address::generate(&setup.env);

    // Fund them in an unrelated token. The group must not accept it.
    let other_token = setup
        .env
        .register_stellar_asset_contract_v2(Address::generate(&setup.env))
        .address();
    token::StellarAssetClient::new(&setup.env, &other_token).mint(&payer, &(1000 * ONE_USDC));

    assert_eq!(client.get_token(), setup.token);
    // Not a member yet, so this fails on membership before it can fail on funds.
    assert_eq!(
        client.try_contribute(&payer, &setup.amount, &1u32),
        Err(Ok(GroupError::NotAMember))
    );
}

// ---------------------------------------------------------------------------
// Payouts — including the WAIT behaviour
// ---------------------------------------------------------------------------

#[test]
fn execute_payout_waits_until_every_member_has_contributed() {
    let setup = setup_started(3, 10 * ONE_USDC, MAX_FEE_BPS);
    let client = setup.client();

    // Nobody has contributed: WAIT.
    assert_eq!(
        client.try_execute_payout(),
        Err(Ok(GroupError::ContributionsIncomplete))
    );

    // Partially funded: still WAIT. A missing contribution never skips anyone.
    client.contribute(&setup.member(0), &setup.amount, &1u32);
    assert_eq!(
        client.try_execute_payout(),
        Err(Ok(GroupError::ContributionsIncomplete))
    );

    client.contribute(&setup.member(1), &setup.amount, &1u32);
    assert_eq!(
        client.try_execute_payout(),
        Err(Ok(GroupError::ContributionsIncomplete))
    );

    // Fully funded: the payout is now permitted.
    client.contribute(&setup.member(2), &setup.amount, &1u32);
    client.execute_payout();

    let round = client.get_round(&1u32);
    assert!(round.payout_executed);
    assert_eq!(round.phase, RoundPhase::PayoutExecuted);
}

#[test]
fn execute_payout_cannot_run_before_the_group_starts() {
    let setup = setup(2, 10 * ONE_USDC, MAX_FEE_BPS);
    setup.join_all();

    assert_eq!(
        setup.client().try_execute_payout(),
        Err(Ok(GroupError::NotActive))
    );
}

#[test]
fn execute_payout_cannot_execute_the_same_round_twice() {
    let setup = setup_started(3, 10 * ONE_USDC, MAX_FEE_BPS);
    let client = setup.client();
    setup.contribute_all(1);

    client.execute_payout();
    // Round 2 has begun, so a second payout is not "already executed" for round 1 —
    // it is simply a payout for a round that is not funded yet.
    assert_eq!(
        client.try_execute_payout(),
        Err(Ok(GroupError::ContributionsIncomplete)),
        "advancing the round exactly once prevents a double payout"
    );
    assert_eq!(client.get_current_round(), 2);
}

#[test]
fn execute_payout_splits_the_pool_exactly_and_advances_the_round() {
    // The canonical case from the specification: 3 × 10 USDC = 30 USDC.
    let setup = setup_started(3, 10 * ONE_USDC, MAX_FEE_BPS);
    let client = setup.client();
    let token_client = setup.token_client();
    setup.contribute_all(1);

    let recipient = setup.member(0);
    let recipient_before = token_client.balance(&recipient);
    let treasury_before = token_client.balance(&setup.treasury);

    client.execute_payout();
    let emitted = setup.env.events().all().filter_by_contract(&setup.group_id);

    // fee = 30 USDC * 50 / 10_000 = 0.15 USDC; recipient = 29.85 USDC.
    let pool = 30 * ONE_USDC;
    let fee = 1_500_000; // 0.15 USDC at 7 decimals
    let recipient_amount = 298_500_000; // 29.85 USDC at 7 decimals

    assert_eq!(fee + recipient_amount, pool, "fee + recipient == pool");
    assert_eq!(
        token_client.balance(&recipient),
        recipient_before + recipient_amount
    );
    assert_eq!(token_client.balance(&setup.treasury), treasury_before + fee);
    assert_eq!(
        token_client.balance(&setup.group_id),
        0,
        "the contract retains nothing"
    );

    assert_eq!(client.get_current_round(), 2);
    assert_eq!(
        client.get_group().round_phase,
        RoundPhase::WaitingForContributions
    );
    assert_eq!(client.get_pool_balance(), 0);

    let expected_payout = PayoutExecuted {
        recipient: recipient.clone(),
        round: 1,
        recipient_amount,
    }
    .to_xdr(&setup.env, &setup.group_id);
    let expected_fee = FeePaid {
        treasury: setup.treasury.clone(),
        round: 1,
        fee,
    }
    .to_xdr(&setup.env, &setup.group_id);
    assert!(emitted.events().contains(&expected_payout));
    assert!(emitted.events().contains(&expected_fee));
}

#[test]
fn execute_payout_pays_the_recipient_from_the_immutable_order() {
    let setup = setup_started(3, 10 * ONE_USDC, MAX_FEE_BPS);
    let client = setup.client();
    let token_client = setup.token_client();

    // Round 1 pays the first joiner, round 2 the second, round 3 the third.
    let mut round = 1;
    while round <= 3 {
        let expected_recipient = setup.member(round - 1);
        assert_eq!(client.get_current_recipient(), expected_recipient);
        assert_eq!(
            client.get_round(&round).recipient,
            Some(expected_recipient.clone())
        );

        let before = token_client.balance(&expected_recipient);
        setup.contribute_all(round);
        client.execute_payout();
        assert_eq!(
            token_client.balance(&expected_recipient),
            before - setup.amount + 298_500_000,
            "round {round} must pay its scheduled recipient"
        );
        round += 1;
    }
}

#[test]
fn final_round_completes_the_group_exactly_once() {
    let setup = setup_started(3, 10 * ONE_USDC, MAX_FEE_BPS);
    let client = setup.client();
    setup.contribute_all(1);
    client.execute_payout();
    setup.contribute_all(2);
    client.execute_payout();
    setup.contribute_all(3);
    client.execute_payout();
    let emitted = setup.env.events().all().filter_by_contract(&setup.group_id);

    let state = client.get_group();
    assert_eq!(state.status, Status::Completed);
    assert_eq!(state.round_phase, RoundPhase::PayoutExecuted);
    assert_eq!(state.current_round, 3);

    // 3 members, 3 rounds: exactly 3 payouts were executed.
    let completed = GroupCompleted { rounds: 3 }.to_xdr(&setup.env, &setup.group_id);
    assert!(emitted.events().contains(&completed));

    // A completed group accepts no further contributions and no further payouts.
    assert_eq!(
        client.try_execute_payout(),
        Err(Ok(GroupError::GroupCompleted))
    );
    assert_eq!(
        client.try_contribute(&setup.member(0), &setup.amount, &3u32),
        Err(Ok(GroupError::GroupCompleted))
    );
}

// ---------------------------------------------------------------------------
// Financial end-to-end (v3 §25.2)
// ---------------------------------------------------------------------------

#[test]
fn financial_e2e_three_members_ten_usdc_each() {
    let setup = setup(3, 10 * ONE_USDC, MAX_FEE_BPS);
    let token_client = setup.token_client();

    let start_balances: Vec<i128> = {
        let mut balances = Vec::new(&setup.env);
        let mut index = 0;
        while index < 3 {
            balances.push_back(token_client.balance(&setup.member(index)));
            index += 1;
        }
        balances
    };

    setup.run_to_completion();

    // Every member pays 3 × 10 USDC and receives exactly one 29.85 USDC payout.
    let mut index = 0;
    while index < 3 {
        let expected = start_balances.get(index).unwrap() - 30 * ONE_USDC + 298_500_000;
        assert_eq!(
            token_client.balance(&setup.member(index)),
            expected,
            "member {index} must end with exactly one payout and three contributions"
        );
        index += 1;
    }

    // The treasury receives 0.5% of each of the 3 rounds, and nothing is retained.
    assert_eq!(token_client.balance(&setup.treasury), 3 * 1_500_000);
    assert_eq!(token_client.balance(&setup.group_id), 0);
    assert_eq!(setup.client().get_status(), Status::Completed);
}

#[test]
fn every_member_receives_exactly_one_payout() {
    let setup = setup(4, 25 * ONE_USDC, MAX_FEE_BPS);
    setup.run_to_completion();

    // Each round pays a distinct member, in join order, exactly once.
    let client = setup.client();
    let mut index = 0;
    while index < 4 {
        assert_eq!(
            client.get_round(&(index + 1)).recipient,
            Some(setup.member(index))
        );
        index += 1;
    }
    assert_eq!(client.get_status(), Status::Completed);
}

/// The fee split must satisfy `fee + recipient == pool` exactly, with no rounding
/// drift, across a wide range of pools and fee values. Truncation always favours
/// the recipient (the fee is truncated, never rounded up).
#[test]
fn fee_split_invariant_holds_across_many_pools() {
    let mut fee_bps = 1;
    while fee_bps <= MAX_FEE_BPS {
        // Pools that divide evenly, and pools that produce truncating fractions.
        let pools: [i128; 7] = [0, 1, 3, 7, 9_999, 100_000_000, 123_456_789_012_345];
        let mut pool_index = 0;
        while pool_index < pools.len() {
            let pool = pools[pool_index];
            let (fee, recipient_amount) = split_pool(pool, fee_bps).unwrap();
            assert_eq!(
                fee + recipient_amount,
                pool,
                "fee + recipient must equal pool (pool={pool}, fee_bps={fee_bps})"
            );
            assert!(fee >= 0, "fee must never be negative");
            assert!(
                recipient_amount <= pool,
                "recipient amount must never exceed the pool"
            );
            assert_eq!(
                fee,
                pool * (fee_bps as i128) / BPS_DENOMINATOR,
                "fee must be the truncated bps share"
            );
            pool_index += 1;
        }
        fee_bps += 1;
    }
}

#[test]
fn fee_split_truncates_toward_the_recipient() {
    // 1 stroop at 50 bps = 0.005 stroops, which truncates to a zero fee.
    let (fee, recipient_amount) = split_pool(1, MAX_FEE_BPS).unwrap();
    assert_eq!(fee, 0);
    assert_eq!(recipient_amount, 1);
}

/// PRNG for deterministic property and fuzz testing without external dependencies.
struct FuzzRng(u64);

impl FuzzRng {
    const fn new(seed: u64) -> Self {
        Self(if seed == 0 { 0xdeadbeefcafebabe } else { seed })
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545F4914F6CDD1D)
    }

    fn next_u128(&mut self) -> u128 {
        ((self.next_u64() as u128) << 64) | (self.next_u64() as u128)
    }
}

/// Property test iterating pools (including i128 extremes and fuzz distributions)
/// and all valid fee_bps values, asserting:
/// 1. `fee + recipient_amount == pool` holds exactly for every pair.
/// 2. `fee` is always the integer floor of the exact ratio `pool * fee_bps / 10_000`.
/// 3. The fee never exceeds `pool * fee_bps / 10_000`.
/// 4. Remainder is non-negative and strictly less than `BPS_DENOMINATOR`.
#[test]
fn fee_split_property_fuzz_across_pools_and_all_valid_fee_bps() {
    let mut rng = FuzzRng::new(0x1337c0de5eed);

    for fee_bps in 1..=MAX_FEE_BPS {
        let bps = fee_bps as i128;
        let max_safe_pool = i128::MAX / bps;

        // Structured representative pools covering zero, small amounts, currency units,
        // intermediate values, and upper bounds up to max_safe_pool.
        let fixed_pools: [i128; 39] = [
            0,
            1,
            2,
            3,
            5,
            7,
            9,
            10,
            11,
            42,
            99,
            100,
            101,
            9_999,
            10_000,
            10_001,
            19_999,
            20_000,
            20_001,
            99_999,
            100_000,
            100_001,
            ONE_USDC,
            10 * ONE_USDC,
            30 * ONE_USDC,
            100 * ONE_USDC,
            100_000_000,
            1_000_000_000,
            123_456_789_012_345,
            1_000_000_000_000_000_000,
            max_safe_pool / 4,
            max_safe_pool / 3,
            max_safe_pool / 2,
            (max_safe_pool / 3) * 2,
            (max_safe_pool / 4) * 3,
            (max_safe_pool / 10) * 9,
            max_safe_pool - 10_000,
            max_safe_pool - 1,
            max_safe_pool,
        ];

        let test_pool = |pool: i128| {
            let (fee, recipient_amount) = split_pool(pool, fee_bps)
                .unwrap_or_else(|e| panic!("split_pool({pool}, {fee_bps}) failed: {e:?}"));

            // Invariant: fee + recipient_amount == pool
            assert_eq!(
                fee + recipient_amount,
                pool,
                "fee + recipient_amount must equal pool (pool={pool}, fee_bps={fee_bps})"
            );

            assert!(
                fee >= 0,
                "fee must not be negative (pool={pool}, fee_bps={fee_bps})"
            );
            assert!(
                recipient_amount <= pool,
                "recipient_amount must not exceed pool (pool={pool}, fee_bps={fee_bps})"
            );
            assert!(
                recipient_amount >= 0,
                "recipient_amount must not be negative (pool={pool}, fee_bps={fee_bps})"
            );

            let numerator = pool * bps;
            let expected_fee = numerator / BPS_DENOMINATOR;

            // Invariant: fee is always the floor of the exact ratio pool * fee_bps / 10_000
            assert_eq!(
                fee, expected_fee,
                "fee must match floor of exact ratio (pool={pool}, fee_bps={fee_bps})"
            );

            // Fee never exceeds pool * fee_bps / 10_000
            assert!(
                fee * BPS_DENOMINATOR <= numerator,
                "fee must never exceed pool * fee_bps / 10_000 (pool={pool}, fee_bps={fee_bps})"
            );

            // Remainder in [0, 10_000), proving exact floor truncation
            let remainder = numerator - fee * BPS_DENOMINATOR;
            assert!(
                (0..BPS_DENOMINATOR).contains(&remainder),
                "remainder {remainder} must be in [0, 10_000) (pool={pool}, fee_bps={fee_bps})"
            );
        };

        for &pool in &fixed_pools {
            test_pool(pool);
        }

        // Powers of 2 and offsets up to max_safe_pool
        for shift in 1..127 {
            if let Some(p) = 1i128.checked_shl(shift) {
                if p <= max_safe_pool {
                    test_pool(p);
                    test_pool(p - 1);
                    if p < max_safe_pool {
                        test_pool(p + 1);
                    }
                }
            }
        }

        // Powers of 10 and offsets up to max_safe_pool
        let mut pow10: i128 = 1;
        while let Some(next) = pow10.checked_mul(10) {
            if next > max_safe_pool {
                break;
            }
            pow10 = next;
            test_pool(pow10);
            test_pool(pow10 - 1);
            if pow10 < max_safe_pool {
                test_pool(pow10 + 1);
            }
        }

        // Property fuzzing: 200 pseudo-random pools distributed up to max_safe_pool
        for _ in 0..200 {
            let rand_val = (rng.next_u128() % (max_safe_pool as u128 + 1)) as i128;
            test_pool(rand_val);
        }
    }
}

/// Boundary pools near and above overflow threshold fail with ArithmeticOverflow, not a panic.
#[test]
fn boundary_pools_near_overflow_fail_with_arithmetic_overflow() {
    for fee_bps in 1..=MAX_FEE_BPS {
        let bps = fee_bps as i128;
        let max_safe_pool = i128::MAX / bps;

        // Boundary: highest non-overflowing pool succeeds
        let (fee, recipient) = split_pool(max_safe_pool, fee_bps)
            .expect("max_safe_pool must succeed without overflow");
        assert_eq!(fee + recipient, max_safe_pool);
        assert_eq!(fee, (max_safe_pool * bps) / BPS_DENOMINATOR);

        // When fee_bps > 1, max_safe_pool < i128::MAX; any pool above it overflows checked_mul
        if fee_bps > 1 {
            let overflow_boundary_pools = [
                max_safe_pool + 1,
                max_safe_pool + 2,
                max_safe_pool + 10_000,
                i128::MAX - 1,
                i128::MAX,
            ];

            for &overflow_pool in &overflow_boundary_pools {
                assert_eq!(
                    split_pool(overflow_pool, fee_bps),
                    Err(GroupError::ArithmeticOverflow),
                    "pool {overflow_pool} with fee_bps {fee_bps} must return ArithmeticOverflow, not panic"
                );
            }
        } else {
            // At 1 bps, max_safe_pool is i128::MAX
            assert_eq!(max_safe_pool, i128::MAX);
        }

        // Negative boundary checks near i128::MIN
        let min_safe_pool = i128::MIN / bps;
        if fee_bps > 1 {
            let min_overflow_pools = [
                min_safe_pool - 1,
                min_safe_pool - 2,
                min_safe_pool - 10_000,
                i128::MIN,
            ];

            for &min_overflow in &min_overflow_pools {
                assert_eq!(
                    split_pool(min_overflow, fee_bps),
                    Err(GroupError::ArithmeticOverflow),
                    "negative pool {min_overflow} with fee_bps {fee_bps} must return ArithmeticOverflow"
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Storage, TTL and views
// ---------------------------------------------------------------------------

#[test]
fn ttl_is_extended_by_an_interaction() {
    use soroban_sdk::testutils::Deployer as _;
    let setup = setup(2, 10 * ONE_USDC, MAX_FEE_BPS);
    setup.join_all();

    let ttl = setup
        .env
        .deployer()
        .get_contract_instance_ttl(&setup.group_id);
    assert!(
        ttl > INSTANCE_TTL_THRESHOLD,
        "instance TTL must be extended so an active group never archives (ttl={ttl})"
    );
}

#[test]
fn read_view_extends_group_instance_ttl() {
    use soroban_sdk::testutils::{Deployer as _, Ledger as _};
    let setup = setup(2, 10 * ONE_USDC, MAX_FEE_BPS);

    // Advance ledger past threshold
    setup
        .env
        .ledger()
        .set_sequence_number(INSTANCE_TTL_THRESHOLD + 10);

    // get_group read extends instance TTL
    let state = setup.client().get_group();
    assert_eq!(state.status, Status::Open);

    let ttl = setup
        .env
        .deployer()
        .get_contract_instance_ttl(&setup.group_id);
    assert!(
        ttl > INSTANCE_TTL_THRESHOLD,
        "get_group read must extend instance TTL (ttl={ttl})"
    );

    // get_member_count also extends instance TTL
    assert_eq!(setup.client().get_member_count(), 0);
}

#[test]
fn persistent_entries_survive_past_threshold_via_read_path() {
    use soroban_sdk::testutils::Ledger as _;
    let setup = setup_started(2, 10 * ONE_USDC, MAX_FEE_BPS);
    let client = setup.client();

    // Advance sequence number past PERSISTENT_TTL_THRESHOLD
    setup
        .env
        .ledger()
        .set_sequence_number(PERSISTENT_TTL_THRESHOLD + 10);

    // Reading member extends persistent Member key and returns valid position
    let pos = client.get_member(&setup.member(0));
    assert_eq!(pos, 1);
}

#[test]
fn get_round_reports_an_unstarted_round_safely() {
    let setup = setup(3, 10 * ONE_USDC, MAX_FEE_BPS);
    let round = setup.client().get_round(&5u32);

    assert_eq!(round.round, 5);
    assert_eq!(round.pool, 0);
    assert_eq!(round.contribution_count, 0);
    assert!(!round.payout_executed);
    assert_eq!(round.recipient, None);
}

#[test]
fn get_round_returns_no_recipient_for_future_rounds() {
    let setup = setup_started(3, 10 * ONE_USDC, MAX_FEE_BPS);
    let client = setup.client();

    // Round 1 is the current round: its scheduled recipient is observable.
    assert_eq!(client.get_round(&1u32).recipient, Some(setup.member(0)));

    // Rounds 2 and 3 have not started yet: no observable recipient, even
    // though the payout order is already fixed at join time.
    let round2 = client.get_round(&2u32);
    assert_eq!(round2.recipient, None);
    assert_eq!(round2.phase, RoundPhase::WaitingForContributions);
    assert!(!round2.payout_executed);

    let round3 = client.get_round(&3u32);
    assert_eq!(round3.recipient, None);
    assert_eq!(round3.phase, RoundPhase::WaitingForContributions);
    assert!(!round3.payout_executed);
}

#[test]
fn get_round_keeps_recipient_for_completed_and_current_rounds() {
    let setup = setup_started(3, 10 * ONE_USDC, MAX_FEE_BPS);
    let client = setup.client();

    // Complete round 1.
    setup.contribute_all(1);
    client.execute_payout();

    // The completed round keeps its recipient for auditability.
    let round1 = client.get_round(&1u32);
    assert!(round1.payout_executed);
    assert_eq!(round1.recipient, Some(setup.member(0)));

    // Round 2 is now current: its recipient is observable.
    let round2 = client.get_round(&2u32);
    assert!(!round2.payout_executed);
    assert_eq!(round2.recipient, Some(setup.member(1)));

    // Round 3 is still in the future: no recipient.
    assert_eq!(client.get_round(&3u32).recipient, None);
}

#[test]
fn get_pool_balance_is_zero_before_the_group_starts() {
    let setup = setup(3, 10 * ONE_USDC, MAX_FEE_BPS);
    assert_eq!(setup.client().get_pool_balance(), 0);
    assert_eq!(setup.client().get_current_round(), 0);
}

#[test]
fn get_current_recipient_fails_before_the_group_starts() {
    let setup = setup(3, 10 * ONE_USDC, MAX_FEE_BPS);
    assert_eq!(
        setup.client().try_get_current_recipient(),
        Err(Ok(GroupError::NotActive))
    );
}

#[test]
fn pool_balance_ignores_stray_transfers() {
    // A payout is derived from validated contributions, not from the raw token
    // balance, so a stray transfer cannot inflate a payout.
    let setup = setup_started(2, 10 * ONE_USDC, MAX_FEE_BPS);
    let client = setup.client();
    let donor = Address::generate(&setup.env);
    token::StellarAssetClient::new(&setup.env, &setup.token).mint(&donor, &(100 * ONE_USDC));
    setup
        .token_client()
        .transfer(&donor, &setup.group_id, &(100 * ONE_USDC));

    assert_eq!(
        client.get_pool_balance(),
        0,
        "stray funds are not part of the round pool"
    );
    assert_eq!(
        client.try_execute_payout(),
        Err(Ok(GroupError::ContributionsIncomplete)),
        "stray funds cannot trigger or inflate a payout"
    );
}

#[test]
#[should_panic]
fn constructor_rejects_treasury_equal_to_group() {
    let env = Env::default();
    env.mock_all_auths();
    let group_id = Address::generate(&env);
    let factory = Address::generate(&env);
    let creator = Address::generate(&env);
    let token = Address::generate(&env);

    // Registering with treasury = group_id should panic
    env.register_at(
        &group_id,
        GroupContract,
        (
            factory,
            creator,
            token,
            group_id.clone(),
            10_000_000i128,
            3u32,
            604_800u64,
            50u32,
        ),
    );
}

#[test]
#[should_panic]
fn constructor_rejects_token_equal_to_group() {
    let env = Env::default();
    env.mock_all_auths();
    let group_id = Address::generate(&env);
    let factory = Address::generate(&env);
    let creator = Address::generate(&env);
    let treasury = Address::generate(&env);

    // Registering with token = group_id should panic
    env.register_at(
        &group_id,
        GroupContract,
        (
            factory,
            creator,
            group_id.clone(),
            treasury,
            10_000_000i128,
            3u32,
            604_800u64,
            50u32,
        ),
    );
}

#[test]
fn contribute_returns_arithmetic_overflow_when_contribution_count_overflows() {
    let setup = setup_started(3, 10 * ONE_USDC, MAX_FEE_BPS);
    let client = setup.client();

    setup.env.as_contract(&setup.group_id, || {
        setup
            .env
            .storage()
            .persistent()
            .set(&DataKey::RoundContributionCount(1), &u32::MAX);
    });

    assert_eq!(
        client.try_contribute(&setup.member(0), &setup.amount, &1u32),
        Err(Ok(GroupError::ArithmeticOverflow))
    );
}
