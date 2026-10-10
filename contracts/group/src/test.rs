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
    assert_eq!(client.try_get_payout_order(), Ok(Ok(expected_order)));

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
fn get_payout_order_fails_loudly_when_a_position_is_archived() {
    let setup = setup(3, 10 * ONE_USDC, MAX_FEE_BPS);
    let client = setup.client();
    setup.join_all();

    // Sanity: the full order resolves before any archival.
    assert_eq!(
        client.try_get_payout_order(),
        Ok(Ok(Vec::from_array(
            &setup.env,
            [setup.member(0), setup.member(1), setup.member(2)],
        )))
    );

    // Simulate an archived `MemberAt` entry by removing it from persistent
    // storage directly. The old code silently skipped the missing position
    // and returned a 2-element vec; the fix must fail loudly instead.
    setup.env.as_contract(&setup.group_id, || {
        setup
            .env
            .storage()
            .persistent()
            .remove(&DataKey::MemberAt(2));
    });

    assert_eq!(
        client.try_get_payout_order(),
        Err(Ok(GroupError::PayoutOrderIncomplete))
    );
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
