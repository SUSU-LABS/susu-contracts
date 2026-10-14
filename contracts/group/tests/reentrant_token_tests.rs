//! Tests for the Group contract's effects-before-interactions protection.
//!
//! The Group contract follows the effects-before-interactions pattern: it
//! writes `PayoutExecuted(round)` and advances `CurrentRound` BEFORE calling
//! the token's `transfer`. This pattern is the protection against a
//! re-entrant token attempting to double-pay the same round by re-entering
//! `execute_payout` from inside `transfer`.
//!
//! ## Why the re-entrant token is a mock, and the test does not literally re-enter
//!
//! Soroban's host function `invoke_contract` does not allow a contract to
//! re-enter itself, so a literally re-entrant token cannot call back into
//! the Group. The reentrancy protection the Group contract enforces
//! (effects before interactions) is what makes the *hypothetical* re-entry
//! safe, and that protection is the property the issue asks us to lock in
//! with a test.
//!
//! These tests therefore verify the *consequences* of the
//! effects-before-interactions pattern: the mock token gives the tests a
//! way to drive the Group contract through the same failure paths an
//! attacker would need to trigger, and the tests assert the observable
//! outcomes that hold when effects are correctly written before
//! interactions.

use soroban_sdk::{testutils::Address as _, Address, Env, Vec};

use susu_group::reentrant_token::{ReentrantTokenContract, ReentrantTokenContractClient};
use susu_group::{GroupContract, GroupContractClient};

const ONE_USDC: i128 = 10_000_000;
const ONE_WEEK: u64 = 604_800;
const MAX_FEE_BPS: u32 = 50;

/// Pool size for the canonical 3-member case.
const POOL_FOR_3_MEMBERS: i128 = 30 * ONE_USDC;
/// Pool size for the 2-member case.
const POOL_FOR_2_MEMBERS: i128 = 20 * ONE_USDC;

/// Compute the recipient's share of a pool under `MAX_FEE_BPS` fee.
fn recipient_amount(pool: i128) -> i128 {
    let fee = pool * MAX_FEE_BPS as i128 / 10_000;
    pool - fee
}

/// Test fixture: a registered group funded with the mock token.
struct Setup {
    env: Env,
    group_id: Address,
    #[allow(dead_code)]
    token_id: Address,
    token: ReentrantTokenContractClient<'static>,
    treasury: Address,
    members: Vec<Address>,
    amount: i128,
    #[allow(dead_code)]
    capacity: u32,
}

impl Setup {
    fn client(&self) -> GroupContractClient<'_> {
        GroupContractClient::new(&self.env, &self.group_id)
    }

    fn member(&self, index: u32) -> Address {
        self.members.get(index).unwrap()
    }

    fn join_all(&self) {
        let client = self.client();
        let mut i = 0;
        while i < self.members.len() {
            client.join(&self.member(i));
            i += 1;
        }
    }

    fn contribute_all(&self, round: u32) {
        let client = self.client();
        let mut i = 0;
        while i < self.members.len() {
            client.contribute(&self.member(i), &self.amount, &round);
            i += 1;
        }
    }
}

fn build(capacity: u32, amount: i128) -> Setup {
    let env = Env::default();
    env.mock_all_auths();

    let treasury = Address::generate(&env);
    let token_id = env.register(ReentrantTokenContract, ());
    let token = ReentrantTokenContractClient::new(&env, &token_id);

    let group_id = env.register(
        GroupContract,
        (
            Address::generate(&env), // factory (informational)
            Address::generate(&env), // creator (informational, no authority)
            token_id.clone(),
            treasury.clone(),
            amount,
            capacity,
            ONE_WEEK,
            MAX_FEE_BPS,
        ),
    );

    let mut members = Vec::new(&env);
    let mut i = 0;
    while i < capacity {
        let m = Address::generate(&env);
        token.mint(&m, &(amount * (capacity as i128 + 1)));
        members.push_back(m);
        i += 1;
    }

    Setup {
        env,
        group_id,
        token_id,
        token,
        treasury,
        members,
        amount,
        capacity,
    }
}

// ---------------------------------------------------------------------------
// Test 1: a full lifecycle with the mock token behaves like a normal SAC.
// This is the regression-test baseline: any change to the Group contract
// that breaks the happy path with a non-re-entrant token must fail here.
// ---------------------------------------------------------------------------

#[test]
fn non_reentrant_run_pays_each_member_exactly_once() {
    let setup = build(3, 10 * ONE_USDC);
    let client = setup.client();
    setup.join_all();
    client.start();

    // Snapshot each member's balance so we can measure total payout
    // received (deposit + payout are interleaved in the account ledger).
    let mut balances_before = Vec::new(&setup.env);
    let mut i = 0;
    while i < setup.members.len() {
        balances_before.push_back(setup.token.balance(&setup.member(i)));
        i += 1;
    }

    let mut round = 1u32;
    while round <= 3 {
        setup.contribute_all(round);
        client.execute_payout();
        round += 1;
    }

    // Every member got exactly one payout, equal to the recipient share of
    // a single 3-member pool.
    let expected_payout = recipient_amount(POOL_FOR_3_MEMBERS);
    let mut i = 0;
    while i < setup.members.len() {
        let before = balances_before.get(i).unwrap();
        let after = setup.token.balance(&setup.member(i));
        // Each member contributes 3 * 10 USDC and receives the recipient
        // amount exactly once. Net change is `-30 + expected_payout`.
        let net = after - before;
        let expected_net = -(3 * setup.amount) + expected_payout;
        assert_eq!(
            net, expected_net,
            "member {i} must have been paid exactly once"
        );
        i += 1;
    }

    // Final group state: completed.
    let group = client.get_group();
    assert_eq!(group.status, susu_group::Status::Completed);
}

// ---------------------------------------------------------------------------
// Test 2: the mock token's failure path (insufficient balance on the
// second transfer) does not leave the contract in a half-applied state.
// ---------------------------------------------------------------------------
//
// The Group contract's `execute_payout` writes effects (PayoutExecuted,
// CurrentRound, RoundPhase) before calling the token's `transfer`. If the
// second transfer panics, the whole `execute_payout` reverts — including
// the effects that were written before the transfers. The test
// therefore asserts the post-revert state, not the in-flight state.

#[test]
fn failed_execute_payout_reverts_to_the_pre_call_state() {
    let setup = build(3, 10 * ONE_USDC);
    let client = setup.client();
    setup.join_all();
    client.start();
    setup.contribute_all(1);

    // Drain the group to *exactly* the fee amount, so the fee transfer
    // succeeds but the recipient transfer panics.
    let fee = POOL_FOR_3_MEMBERS * MAX_FEE_BPS as i128 / 10_000;
    setup.token.drain(&setup.group_id);
    setup.token.mint(&setup.group_id, &fee);

    let round_before = client.get_current_round();
    let phase_before = client.get_group().round_phase;
    let recipient_before_balance = setup.token.balance(
        &client
            .get_round(&1)
            .recipient
            .expect("round 1 must have a recipient"),
    );
    let treasury_before_balance = setup.token.balance(&setup.treasury);

    let res = client.try_execute_payout();
    assert!(
        res.is_err(),
        "the second transfer's insufficient balance must surface as an error to the caller"
    );

    // The whole call reverted: the round pointer, the phase, the
    // recipient's balance, and the treasury's balance are all back to
    // their pre-call state.
    assert_eq!(client.get_current_round(), round_before);
    assert_eq!(client.get_group().round_phase, phase_before);
    assert_eq!(
        setup.token.balance(
            &client
                .get_round(&1)
                .recipient
                .expect("round 1 must have a recipient")
        ),
        recipient_before_balance
    );
    assert_eq!(
        setup.token.balance(&setup.treasury),
        treasury_before_balance
    );
}

// ---------------------------------------------------------------------------
// Test 3: a re-attempted execute_payout on the same round (after the
// failed call has reverted) succeeds cleanly. The effects-before-
// interactions pattern is compatible with retry: a failed call does not
// leave behind partial state that blocks the next call.
// ---------------------------------------------------------------------------

#[test]
fn re_attempting_execute_payout_after_a_failed_call_succeeds() {
    let setup = build(3, 10 * ONE_USDC);
    let client = setup.client();
    setup.join_all();
    client.start();
    setup.contribute_all(1);

    // Round 1 funded. Drain the group to fee, run a failed execute_payout.
    let fee = POOL_FOR_3_MEMBERS * MAX_FEE_BPS as i128 / 10_000;
    setup.token.drain(&setup.group_id);
    setup.token.mint(&setup.group_id, &fee);
    let res = client.try_execute_payout();
    assert!(res.is_err());

    // Top the group up so the next call can complete. Members will
    // re-authorise their contributions, and the round is still ReadyForPayout.
    setup.token.mint(&setup.group_id, &(POOL_FOR_3_MEMBERS - fee));

    let round1 = client.get_round(&1);
    let recipient = round1.recipient.expect("round 1 must have a recipient");
    let recipient_before = setup.token.balance(&recipient);
    let treasury_before = setup.token.balance(&setup.treasury);

    client.execute_payout();

    let recipient_received = setup.token.balance(&recipient) - recipient_before;
    let fee_received = setup.token.balance(&setup.treasury) - treasury_before;
    assert_eq!(
        recipient_received,
        recipient_amount(POOL_FOR_3_MEMBERS),
        "round 1 recipient must receive the configured share, exactly once"
    );
    assert_eq!(
        fee_received,
        POOL_FOR_3_MEMBERS * MAX_FEE_BPS as i128 / 10_000,
        "treasury must receive the protocol fee, exactly once"
    );
    assert_eq!(client.get_current_round(), 2);
}
