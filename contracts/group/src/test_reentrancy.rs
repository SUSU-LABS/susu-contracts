#![cfg(test)]

//! Re-entrancy adversarial tests — issue #9.
//!
//! Every other test in this crate pays with the standard Stellar Asset
//! Contract (`test.rs:90-92`, `deploy_group.rs:50-52`). The Factory forwards
//! a caller-supplied token and the Group constructor accepts an arbitrary
//! token address, so a hostile token is a realistic, entirely untested
//! adversary.
//!
//! These tests register a test-only malicious token whose `transfer` fires a
//! re-entrant call into the Group mid-flight — once during a contribution
//! payment, once during a payout.
//!
//! Result: the nested invoke is refused by the platform before it reaches
//! the contract — soroban-env-host 27 rejects calls to a contract that is
//! already on the call stack (`ContractReentryMode::Prohibited`, surfaced as
//! `ScErrorType::Context` / `InvalidAction`, wrapped by the SDK as an
//! `InvokeError::Abort`) — and the ledger records exactly one contribution
//! and one payout, with exact final balances.
//!
//! The checks-effects-interactions ordering in `contribute` and
//! `execute_payout` — the contribution key and the round pointer are
//! committed before the external token transfer — is the second layer of
//! defense for a future where re-entry is permitted: it is exercised
//! directly by `contribute_rejects_a_duplicate_contribution_in_the_same_round`
//! and `execute_payout_cannot_execute_the_same_round_twice` in `test.rs`.

use super::*;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{contract, contractimpl, contracttype, Address, Env, Vec};

/// 1 USDC in stroops (7 decimals), matching `test.rs`.
const ONE_USDC: i128 = 10_000_000;

/// A week, in seconds. Informational on-chain; payouts are gated by
/// contributions.
const ONE_WEEK: u64 = 604_800;

/// The canonical documented case (3 members x 10 USDC, pool 30 USDC, fee
/// 0.15 USDC, recipient 29.85 USDC) uses 50 bps.
const CANONICAL_FEE_BPS: u32 = 50;

/// Fee on a full 3-member pool, in stroops: 30 USDC * 50 bps.
const CANONICAL_FEE: i128 = 1_500_000;

/// Recipient share of a full 3-member pool, in stroops: 30 USDC - fee.
const CANONICAL_RECIPIENT: i128 = 298_500_000;

/// Marker recorded when the nested invoke never reached the Group: the host
/// refused it (re-entry protection, or a conversion failure). This is the
/// expected outcome on soroban-env-host 27, where contract re-entry is
/// prohibited by default.
const HOST_REJECTED: u32 = u32::MAX;

/// Which re-entrant call the malicious token attempts.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Attack {
    /// Re-enter `contribute` for a member that has already contributed.
    Contribute,
    /// Re-enter `execute_payout` while a payout is being paid out.
    Payout,
}

/// What the malicious token observed about its re-entrant attempt.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttackReport {
    /// Number of transfers that carried an armed attack.
    pub attacks: u32,
    /// Re-entrant calls the Group rejected.
    pub rejected: u32,
    /// Re-entrant calls the Group accepted (must stay zero).
    pub succeeded: u32,
    /// `GroupError` discriminant of the last rejection: `HOST_REJECTED`
    /// when the nested invoke was refused before reaching the Group
    /// (the expected platform behavior), the guard's code when a nested
    /// call actually reached the Group, and 0 when it was accepted.
    pub last_error: u32,
}

#[contracttype]
enum TokenKey {
    Balance(Address),
    Group,
    Attacker,
    Round,
    Kind,
    Attacks,
    Rejected,
    Succeeded,
    LastError,
}

/// A token that follows the interface the Group actually calls (`transfer`)
/// but fires a re-entrant call into the Group on the first armed transfer.
///
/// Its ledger uses checked arithmetic, so an actual double-spend attempt
/// panics here instead of silently creating money.
#[contract]
struct ReentrantToken;

#[contractimpl]
impl ReentrantToken {
    pub fn __constructor(env: Env) {
        let instance = env.storage().instance();
        instance.set(&TokenKey::Attacks, &0u32);
        instance.set(&TokenKey::Rejected, &0u32);
        instance.set(&TokenKey::Succeeded, &0u32);
        instance.set(&TokenKey::LastError, &0u32);
    }

    pub fn mint(env: Env, to: Address, amount: i128) {
        let instance = env.storage().instance();
        let current: i128 = instance.get(&TokenKey::Balance(to.clone())).unwrap_or(0);
        instance.set(&TokenKey::Balance(to), &(current + amount));
    }

    pub fn balance(env: Env, id: Address) -> i128 {
        env.storage()
            .instance()
            .get(&TokenKey::Balance(id))
            .unwrap_or(0)
    }

    /// Arm the token: the next `transfer` re-enters the Group before
    /// recording the payment.
    pub fn arm(env: Env, group: Address, attacker: Address, round: u32, kind: Attack) {
        let instance = env.storage().instance();
        instance.set(&TokenKey::Group, &group);
        instance.set(&TokenKey::Attacker, &attacker);
        instance.set(&TokenKey::Round, &round);
        instance.set(&TokenKey::Kind, &kind);
    }

    pub fn disarm(env: Env) {
        env.storage().instance().remove(&TokenKey::Group);
    }

    pub fn report(env: Env) -> AttackReport {
        let instance = env.storage().instance();
        AttackReport {
            attacks: instance.get(&TokenKey::Attacks).unwrap_or(0),
            rejected: instance.get(&TokenKey::Rejected).unwrap_or(0),
            succeeded: instance.get(&TokenKey::Succeeded).unwrap_or(0),
            last_error: instance.get(&TokenKey::LastError).unwrap_or(0),
        }
    }

    /// The token entry point the Group calls during contribution and payout.
    pub fn transfer(env: Env, from: Address, to: Address, amount: i128) {
        let instance = env.storage().instance();
        let attacks: u32 = instance.get(&TokenKey::Attacks).unwrap_or(0);
        let armed: Option<Address> = instance.get(&TokenKey::Group);
        if let Some(group) = armed {
            // Fire once. A nested call that somehow passed the guards would
            // itself transfer through here, and `attacks` is already 1, so
            // the recursion cannot loop.
            if attacks == 0 {
                instance.set(&TokenKey::Attacks, &1u32);
                let attacker: Address = instance.get(&TokenKey::Attacker).unwrap();
                let round: u32 = instance.get(&TokenKey::Round).unwrap();
                let kind: Attack = instance.get(&TokenKey::Kind).unwrap();

                let client = GroupContractClient::new(&env, &group);
                let (succeeded, code) = match kind {
                    Attack::Contribute => match client.try_contribute(&attacker, &amount, &round) {
                        // The nested Result distinguishes a clean success,
                        // a conversion failure, the Group's own error, and a
                        // host-level failure such as re-entry protection.
                        Ok(Ok(())) => (true, 0u32),
                        Ok(Err(_)) => (false, HOST_REJECTED),
                        Err(Ok(error)) => (false, error as u32),
                        Err(Err(_)) => (false, HOST_REJECTED),
                    },
                    Attack::Payout => match client.try_execute_payout() {
                        Ok(Ok(())) => (true, 0u32),
                        Ok(Err(_)) => (false, HOST_REJECTED),
                        Err(Ok(error)) => (false, error as u32),
                        Err(Err(_)) => (false, HOST_REJECTED),
                    },
                };

                let instance = env.storage().instance();
                instance.set(&TokenKey::LastError, &code);
                if succeeded {
                    let count: u32 = instance.get(&TokenKey::Succeeded).unwrap_or(0);
                    instance.set(&TokenKey::Succeeded, &(count + 1));
                } else {
                    let count: u32 = instance.get(&TokenKey::Rejected).unwrap_or(0);
                    instance.set(&TokenKey::Rejected, &(count + 1));
                }
            }
        }

        // Honest ledger with checked arithmetic: a transfer that exceeds the
        // payer's balance means someone attempted to spend money twice.
        let instance = env.storage().instance();
        let from_balance: i128 = instance.get(&TokenKey::Balance(from.clone())).unwrap_or(0);
        let to_balance: i128 = instance.get(&TokenKey::Balance(to.clone())).unwrap_or(0);
        if from_balance < amount {
            panic!("reentrant token: transfer exceeds balance (double spend)");
        }
        instance.set(&TokenKey::Balance(from), &(from_balance - amount));
        instance.set(&TokenKey::Balance(to), &(to_balance + amount));
    }
}

/// A group paid with the malicious token: `contribute` and `execute_payout`
/// hit the hostile `transfer`, not the standard Stellar Asset Contract.
struct Adversarial {
    env: Env,
    group_id: Address,
    token_id: Address,
    members: Vec<Address>,
    treasury: Address,
    amount: i128,
    capacity: u32,
}

impl Adversarial {
    fn group(&self) -> GroupContractClient<'_> {
        GroupContractClient::new(&self.env, &self.group_id)
    }

    fn token(&self) -> ReentrantTokenClient<'_> {
        ReentrantTokenClient::new(&self.env, &self.token_id)
    }

    fn member(&self, index: u32) -> Address {
        self.members.get(index).unwrap()
    }

    fn join_all(&self) {
        let client = self.group();
        let mut index = 0;
        while index < self.capacity {
            client.join(&self.member(index));
            index += 1;
        }
    }

    fn contribute_all(&self, round: u32) {
        let client = self.group();
        let mut index = 0;
        while index < self.capacity {
            client.contribute(&self.member(index), &self.amount, &round);
            index += 1;
        }
    }
}

/// Builds a group of `capacity` funded members paid with the malicious token.
fn adversarial(capacity: u32, amount: i128, fee_bps: u32) -> Adversarial {
    let env = Env::default();
    env.mock_all_auths();

    let treasury = Address::generate(&env);
    let token_id = env.register(ReentrantToken, ());

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
            fee_bps,
        ),
    );

    // Fund every member well beyond their total obligation.
    let token = ReentrantTokenClient::new(&env, &token_id);
    let mut members = Vec::new(&env);
    let mut index = 0;
    while index < capacity {
        let member = Address::generate(&env);
        token.mint(&member, &(amount * (capacity as i128 + 1)));
        members.push_back(member);
        index += 1;
    }

    Adversarial {
        env,
        group_id,
        token_id,
        members,
        treasury,
        amount,
        capacity,
    }
}

// ---------------------------------------------------------------------------
// Positive control: the hostile fixture itself is sound
// ---------------------------------------------------------------------------

/// The malicious token, never armed, must not disturb a complete run. This
/// is the positive control for the fixture: it proves that any rejection
/// observed in the armed tests comes from the attack, not from a broken
/// token implementation.
#[test]
fn reentrant_token_unarmed_completes_a_full_run() {
    let a = adversarial(3, 10 * ONE_USDC, CANONICAL_FEE_BPS);
    a.join_all();
    a.group().start();

    let mut round = 1;
    while round <= a.capacity {
        a.contribute_all(round);
        a.group().execute_payout();
        round += 1;
    }

    // Every member: minted 40, contributed 30 across three rounds, received
    // 29.85 once (join order is the payout order).
    let token = a.token();
    let mut index = 0;
    while index < a.capacity {
        let final_balance = 4 * 10 * ONE_USDC - 30 * ONE_USDC + CANONICAL_RECIPIENT;
        assert_eq!(token.balance(&a.member(index)), final_balance);
        index += 1;
    }

    // The fee is protocol revenue; the group retains nothing.
    assert_eq!(token.balance(&a.treasury), 3 * CANONICAL_FEE);
    assert_eq!(token.balance(&a.group_id), 0);

    let report = token.report();
    assert_eq!(report.attacks, 0);
    assert_eq!(report.succeeded, 0);

    assert_eq!(a.group().get_status(), Status::Completed);
}

// ---------------------------------------------------------------------------
// Re-entrant contribution (issue #9, `contribute` path)
// ---------------------------------------------------------------------------

/// The token re-enters `contribute` for the member whose payment is in
/// flight. The host refuses the nested invoke (contract re-entry is
/// prohibited), so exactly one payment leaves the member's balance, and
/// the round records exactly one contribution. The `AlreadyContributed`
/// guard that would stop a re-entrant call is covered directly by
/// `contribute_rejects_a_duplicate_contribution_in_the_same_round`.
#[test]
fn reentrant_contribution_is_rejected() {
    let a = adversarial(3, 10 * ONE_USDC, CANONICAL_FEE_BPS);
    a.join_all();
    a.group().start();

    a.token()
        .arm(&a.group_id, &a.member(0), &1u32, &Attack::Contribute);
    a.group().contribute(&a.member(0), &(10 * ONE_USDC), &1u32);

    // The nested call was attempted exactly once and refused before it
    // reached the Group: contract re-entry is prohibited by the host.
    let report = a.token().report();
    assert_eq!(report.attacks, 1);
    assert_eq!(report.succeeded, 0);
    assert_eq!(report.rejected, 1);
    assert_eq!(report.last_error, HOST_REJECTED);

    // The round records one contribution, not two.
    let round = a.group().get_round(&1u32);
    assert_eq!(round.contribution_count, 1);
    assert_eq!(round.pool, 10 * ONE_USDC);
    assert!(!round.payout_executed);

    // Exactly one debit from the member, one credit to the group.
    let token = a.token();
    assert_eq!(
        token.balance(&a.member(0)),
        4 * 10 * ONE_USDC - 10 * ONE_USDC
    );
    assert_eq!(token.balance(&a.group_id), 10 * ONE_USDC);

    // The round still completes normally after the failed attack.
    a.group().contribute(&a.member(1), &a.amount, &1u32);
    a.group().contribute(&a.member(2), &a.amount, &1u32);
    a.group().execute_payout();

    // Member 0 is the round-1 recipient: paid exactly once.
    assert_eq!(
        token.balance(&a.member(0)),
        4 * 10 * ONE_USDC - 10 * ONE_USDC + CANONICAL_RECIPIENT
    );
    assert_eq!(token.balance(&a.treasury), CANONICAL_FEE);
    assert_eq!(token.balance(&a.group_id), 0);
    assert_eq!(a.group().get_current_round(), 2);
}

// ---------------------------------------------------------------------------
// Re-entrant payout, non-final round (issue #9, `execute_payout` path)
// ---------------------------------------------------------------------------

/// The token re-enters `execute_payout` while the round-2 payout is being
/// paid out. The host refuses the nested invoke, so the recipient is paid
/// exactly once. The round-advance guard that would stop a re-entrant
/// payout is covered directly by
/// `execute_payout_cannot_execute_the_same_round_twice`.
#[test]
fn reentrant_payout_is_rejected_mid_round() {
    let a = adversarial(3, 10 * ONE_USDC, CANONICAL_FEE_BPS);
    a.join_all();
    a.group().start();

    // Round 1 completes normally.
    a.contribute_all(1);
    a.group().execute_payout();

    // Round 2 fills, and the payout becomes the attack surface.
    a.contribute_all(2);
    a.token()
        .arm(&a.group_id, &a.member(1), &2u32, &Attack::Payout);
    a.group().execute_payout();

    // The nested payout was attempted exactly once and refused before it
    // reached the Group: contract re-entry is prohibited by the host.
    let report = a.token().report();
    assert_eq!(report.attacks, 1);
    assert_eq!(report.succeeded, 0);
    assert_eq!(report.rejected, 1);
    assert_eq!(report.last_error, HOST_REJECTED);

    // Member 1 is the round-2 recipient: contributed twice, paid exactly
    // once.
    let token = a.token();
    assert_eq!(
        token.balance(&a.member(1)),
        4 * 10 * ONE_USDC - 2 * 10 * ONE_USDC + CANONICAL_RECIPIENT
    );
    assert_eq!(token.balance(&a.treasury), 2 * CANONICAL_FEE);
    assert_eq!(token.balance(&a.group_id), 0);

    // The executed round is closed exactly once.
    let round = a.group().get_round(&2u32);
    assert!(round.payout_executed);
    assert_eq!(a.group().get_current_round(), 3);
    assert_eq!(a.group().get_status(), Status::Active);
}

// ---------------------------------------------------------------------------
// Re-entrant payout, final round (issue #9, `execute_payout` path)
// ---------------------------------------------------------------------------

/// On the final round the token re-enters `execute_payout` during the
/// closing transfers. The host refuses the nested invoke, so the closing
/// accounting is exact: every member is paid exactly once and the group
/// retains nothing. The `PayoutAlreadyExecuted` guard that would stop a
/// re-entrant call is covered directly by
/// `execute_payout_cannot_execute_the_same_round_twice`.
#[test]
fn reentrant_payout_is_rejected_on_final_round() {
    let a = adversarial(3, 10 * ONE_USDC, CANONICAL_FEE_BPS);
    a.join_all();
    a.group().start();

    // Rounds 1 and 2 complete normally.
    a.contribute_all(1);
    a.group().execute_payout();
    a.contribute_all(2);
    a.group().execute_payout();

    // The final round fills, and its payout becomes the attack surface.
    a.contribute_all(3);
    a.token()
        .arm(&a.group_id, &a.member(2), &3u32, &Attack::Payout);
    a.group().execute_payout();

    let report = a.token().report();
    assert_eq!(report.attacks, 1);
    assert_eq!(report.succeeded, 0);
    assert_eq!(report.rejected, 1);
    assert_eq!(report.last_error, HOST_REJECTED);

    // Closing accounting: every member paid exactly once, fees collected
    // exactly once per round, nothing retained by the group.
    let token = a.token();
    let mut index = 0;
    while index < a.capacity {
        let final_balance = 4 * 10 * ONE_USDC - 30 * ONE_USDC + CANONICAL_RECIPIENT;
        assert_eq!(token.balance(&a.member(index)), final_balance);
        index += 1;
    }
    assert_eq!(token.balance(&a.treasury), 3 * CANONICAL_FEE);
    assert_eq!(token.balance(&a.group_id), 0);

    assert_eq!(a.group().get_status(), Status::Completed);

    // No residual pending state: the group is closed.
    let round = a.group().get_round(&3u32);
    assert!(round.payout_executed);
}
