//! Test-only malicious SEP-41-compatible token used to verify reentrancy
//! safety of `contribute` and `execute_payout`.
//!
//! Attack modes: 1 = re-enter `contribute`, 2/3 = re-enter `execute_payout`.
//! The nested outcome is recorded as a sentinel code retrievable via
//! `get_nested_result` (1 = nested success, 4 = contract error,
//! 200 = host abort).
//!
//! The token behaves like a normal ledgered token except that on the first
//! `transfer` / `transfer_from` it performs a single re-entrant call back
//! into the Group contract before completing the ledger update. A
//! recursion guard (`reentered` flag) keeps the attack to exactly one
//! nested call so tests stay deterministic and cannot infinitely recurse.
#![allow(clippy::too_many_arguments)]

use soroban_sdk::{contract, contractimpl, vec, Address, Env, IntoVal, Map, Symbol, Val, Vec};

#[contract]
pub struct ReentrantToken;

#[contractimpl]
impl ReentrantToken {
    /// Initialise balances, reentry target, and attack mode.
    /// `mode`: 1 = re-enter `contribute`, 2 = re-enter `execute_payout`.
    pub fn init(e: Env, admin: Address, balances: Map<Address, i128>, group: Address, mode: u32) {
        e.storage()
            .instance()
            .set(&Symbol::new(&e, "admin"), &admin);
        e.storage()
            .instance()
            .set(&Symbol::new(&e, "bals"), &balances);
        e.storage()
            .instance()
            .set(&Symbol::new(&e, "group"), &group);
        e.storage().instance().set(&Symbol::new(&e, "mode"), &mode);
        e.storage()
            .instance()
            .set(&Symbol::new(&e, "reentered"), &false);
    }

    pub fn set_mode(e: Env, mode: u32) {
        e.storage().instance().set(&Symbol::new(&e, "mode"), &mode);
        e.storage()
            .instance()
            .set(&Symbol::new(&e, "reentered"), &false);
    }

    pub fn balance(e: Env, addr: Address) -> i128 {
        let bals: Map<Address, i128> = e
            .storage()
            .instance()
            .get(&Symbol::new(&e, "bals"))
            .unwrap();
        bals.get(addr).unwrap_or(0)
    }

    fn debit(e: &Env, from: &Address, amount: i128) {
        let key = Symbol::new(e, "bals");
        let mut bals: Map<Address, i128> = e.storage().instance().get(&key).unwrap();
        let cur: i128 = bals.get(from.clone()).unwrap_or(0);
        assert!(cur >= amount, "insufficient balance");
        bals.set(from.clone(), cur - amount);
        e.storage().instance().set(&key, &bals);
    }

    fn credit(e: &Env, to: &Address, amount: i128) {
        let key = Symbol::new(e, "bals");
        let mut bals: Map<Address, i128> = e.storage().instance().get(&key).unwrap();
        let cur: i128 = bals.get(to.clone()).unwrap_or(0);
        bals.set(to.clone(), cur + amount);
        e.storage().instance().set(&key, &bals);
    }

    fn maybe_reenter(e: &Env, from: Address, amount: i128) {
        let rkey = Symbol::new(e, "reentered");
        let done: bool = e.storage().instance().get(&rkey).unwrap_or(false);
        if done {
            return;
        }
        e.storage().instance().set(&rkey, &true);
        let group: Address = e
            .storage()
            .instance()
            .get(&Symbol::new(e, "group"))
            .unwrap();
        let mode: u32 = e
            .storage()
            .instance()
            .get(&Symbol::new(e, "mode"))
            .unwrap_or(0);
        // Best-effort reentry: record the nested outcome; outer call must still
        // preserve single-accounting invariants.
        // Sentinel codes for get_nested_result():
        //   1 = nested call succeeded (double payout — the bug under test)
        //   4 = nested call cleanly rejected with a contract error
        // 200 = nested call aborted at the host level
        let args: Vec<Val>;
        let func: &str;
        if mode == 1 {
            func = "contribute";
            args = vec![e, from.into_val(e), amount.into_val(e)];
        } else if mode == 2 || mode == 3 {
            func = "execute_payout";
            args = vec![e];
        } else {
            return;
        }
        let code: u32 = match e.try_invoke_contract::<(), soroban_sdk::Error>(
            &group,
            &Symbol::new(e, func),
            args,
        ) {
            Ok(Ok(())) => 1,
            Ok(Err(_)) => 200,
            Err(Ok(_)) => 4,
            Err(Err(_)) => 200,
        };
        e.storage()
            .instance()
            .set(&Symbol::new(e, "nested_result"), &code);
    }

    /// Returns the sentinel code recorded by the last reentrant attempt:
    /// 1 = nested call succeeded, 4 = rejected with a contract error,
    /// 200 = aborted at host level, u32::MAX = no reentry attempted yet.
    pub fn get_nested_result(e: Env) -> u32 {
        e.storage()
            .instance()
            .get(&Symbol::new(&e, "nested_result"))
            .unwrap_or(u32::MAX)
    }

    pub fn transfer(e: Env, from: Address, to: Address, amount: i128) {
        from.require_auth();
        Self::maybe_reenter(&e, from.clone(), amount);
        Self::debit(&e, &from, amount);
        Self::credit(&e, &to, amount);
    }

    pub fn transfer_from(e: Env, spender: Address, from: Address, to: Address, amount: i128) {
        spender.require_auth();
        Self::maybe_reenter(&e, from.clone(), amount);
        Self::debit(&e, &from, amount);
        Self::credit(&e, &to, amount);
    }

    pub fn approve(e: Env, from: Address, spender: Address, amount: i128, _live_until: u32) {
        from.require_auth();
        let _ = (spender, amount);
        let _ = e;
    }
}
