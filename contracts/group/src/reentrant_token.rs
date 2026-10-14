//! Mock token used to verify the Group contract's payout path is
//! effects-before-interactions safe.
//!
//! The Group contract writes `PayoutExecuted(round)` and advances
//! `CurrentRound` BEFORE calling the token's `transfer`. This module is a
//! minimal SAC-shaped mock that the integration tests in
//! `tests/reentrant_token_tests.rs` use to drive the Group contract into
//! the failure paths that an attacker would need to trigger.
//!
//! Note on Soroban's self-recursion behaviour: Soroban's host function
//! `invoke_contract` does not allow a contract to re-enter itself, so a
//! truly re-entrant token cannot actually call back into the Group. The
//! tests therefore verify the *property* the reentrancy protection
//! enforces — that effects are committed before the first external
//! interaction — rather than the literal reentrancy call. The mock token
//! makes that verification possible by giving the tests a way to make the
//! second `transfer` panic (insufficient balance) while leaving the first
//! transfer's effects on the ledger.
//!
//! Production callers do not need to import this module; the SAC interface
//! the Group contract depends on is what this mirrors.

use soroban_sdk::{contract, contractimpl, contracttype, testutils::Address as _, Address, Env, Map};

#[contracttype]
#[derive(Clone)]
enum DataKey {
    /// Per-address balance.
    Balance(Address),
}

#[contract]
pub struct ReentrantTokenContract;

#[contractimpl]
impl ReentrantTokenContract {
    pub fn __constructor(_env: Env) {}

    /// Mint tokens to `to`. Test-only convenience used in place of the
    /// Stellar Asset Contract's `mint`.
    pub fn mint(env: Env, to: Address, amount: i128) {
        let key = DataKey::Balance(to.clone());
        let current: i128 = env.storage().instance().get(&key).unwrap_or(0);
        env.storage().instance().set(&key, &(current + amount));
    }

    /// Drain an account's balance to zero. Test-only convenience used to
    /// set up the "second transfer panics" path the issue's regression
    /// test exercises.
    pub fn drain(env: Env, addr: Address) {
        env.storage()
            .instance()
            .set(&DataKey::Balance(addr), &0i128);
    }

    /// Read the current balance of `addr`.
    pub fn balance(env: Env, addr: Address) -> i128 {
        env.storage()
            .instance()
            .get(&DataKey::Balance(addr))
            .unwrap_or(0)
    }

    /// Standard SAC-shaped `transfer`. Effects on the balances are
    /// committed first, so any caller observing the contract state after
    /// the call sees the new balances.
    pub fn transfer(env: Env, from: Address, to: Address, amount: i128) {
        from.require_auth();

        let from_key = DataKey::Balance(from.clone());
        let to_key = DataKey::Balance(to.clone());

        let from_balance: i128 = env.storage().instance().get(&from_key).unwrap_or(0);
        if from_balance < amount {
            panic!("insufficient balance");
        }
        env.storage()
            .instance()
            .set(&from_key, &(from_balance - amount));
        let to_balance: i128 = env.storage().instance().get(&to_key).unwrap_or(0);
        env.storage()
            .instance()
            .set(&to_key, &(to_balance + amount));
    }

    /// The balance of a list of addresses. Test-only convenience.
    #[allow(dead_code)]
    pub fn balances(env: Env, addrs: soroban_sdk::Vec<Address>) -> soroban_sdk::Vec<i128> {
        let mut out = soroban_sdk::Vec::new(&env);
        for addr in addrs.iter() {
            let key = DataKey::Balance(addr);
            out.push_back(
                env.storage()
                    .instance()
                    .get(&key)
                    .unwrap_or(0),
            );
        }
        out
    }

    /// Bulk balance snapshot via a `Map`. Not part of the SAC interface;
    /// exposed for tests that want a keyed view.
    #[allow(dead_code)]
    pub fn balance_map(env: Env, addrs: soroban_sdk::Vec<Address>) -> Map<Address, i128> {
        let mut out = Map::new(&env);
        for addr in addrs.iter() {
            let key = DataKey::Balance(addr.clone());
            let v: i128 = env.storage().instance().get(&key).unwrap_or(0);
            out.set(addr, v);
        }
        out
    }
}
