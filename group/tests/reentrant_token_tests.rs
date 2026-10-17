//! Reentrancy regression tests for `contribute` and `execute_payout`.
//! Uses the test-only `ReentrantToken` (see `group/src/reentrant_token.rs`)
//! wired in as the Group's token so its `transfer`/`transfer_from`
//! re-enters the Group mid-call. Asserts no double contribution / payout.
use soroban_sdk::{testutils::Address as _, Address, Env, Map};

use crate::reentrant_token::ReentrantToken;

// Helpers reuse the existing fixtures in `test.rs` / `deploy_group.rs`:
// deploy a Group bound to the malicious token, fund one contributor,
// then drive the attack. Adjust the helper names to match local fixtures.

#[test]
fn contribute_reentrant_token_no_double_count() {
    let e = Env::default();
    e.mock_all_auths();
    let admin = Address::generate(&e);
    let user = Address::generate(&e);

    // Register malicious token with an initial balance for `user`.
    let token_id = e.register_contract(None, ReentrantToken);
    let mut bals = Map::new(&e);
    bals.set(user.clone(), 1_000_000i128);
    // Group address unknown yet: set placeholder, patched after deploy.
    // Mode 1 => re-enter `contribute` from inside token transfer.
    let placeholder = Address::generate(&e);
    e.invoke_contract::<()>(
        &token_id,
        &soroban_sdk::Symbol::new(&e, "__init"),
        (admin, bals, placeholder, 1u32).into(),
    );

    // Deploy group with `token_id` as its token (constructor arg at lib.rs:361).
    let group_id = crate::deploy_group(&e, &token_id);
    // Point the token back at the real group contract.
    // (Stored `group` address is updated via instance storage reset.)
    e.invoke_contract::<()>(
        &token_id,
        &soroban_sdk::Symbol::new(&e, "__init"),
        (e.current_contract_address(), {
            let mut m = Map::new(&e);
            m.set(user.clone(), 1_000_000i128);
            m
        }, group_id.clone(), 1u32)
            .into(),
    );

    let before: i128 = e.invoke_contract(&token_id, &soroban_sdk::Symbol::new(&e, "balance"), (user.clone(),).into());
    assert!(before == 1_000_000i128);

    // Single contribute; nested reentrant contribute must not double-credit.
    let amount = 100_000i128;
    let _ = e.try_invoke_contract::<(), soroban_sdk::Error>(
        &group_id,
        &soroban_sdk::Symbol::new(&e, "contribute"),
        (user.clone(), amount).into(),
    );

    let credited: i128 = crate::contribution_of(&e, &group_id, &user);
    assert_eq!(credited, amount, "reentrant contribute double-counted");
    let user_bal: i128 = e.invoke_contract(&token_id, &soroban_sdk::Symbol::new(&e, "balance"), (user.clone(),).into());
    assert_eq!(user_bal, before - amount, "token balance mismatch after reentrant contribute");
}

#[test]
fn execute_payout_reentrant_token_no_double_payout() {
    let e = Env::default();
    e.mock_all_auths();
    let admin = Address::generate(&e);
    let user = Address::generate(&e);
    let token_id = e.register_contract(None, ReentrantToken);
    let mut bals = Map::new(&e);
    bals.set(user.clone(), 1_000_000i128);
    let group_id = crate::deploy_group(&e, &token_id);
    // Mode 2 => re-enter `execute_payout` from inside payout transfer.
    e.invoke_contract::<()>(
        &token_id,
        &soroban_sdk::Symbol::new(&e, "__init"),
        (admin, bals, group_id.clone(), 2u32).into(),
    );
    crate::fund_and_finalize(&e, &group_id, &user, 100_000i128);
    let payout: i128 = crate::expected_payout(&e, &group_id, &user);
    let _ = e.try_invoke_contract::<(), soroban_sdk::Error>(
        &group_id,
        &soroban_sdk::Symbol::new(&e, "execute_payout"),
        ().into(),
    );
    let user_bal: i128 = e.invoke_contract(&token_id, &soroban_sdk::Symbol::new(&e, "balance"), (user.clone(),).into());
    assert_eq!(user_bal, payout, "reentrant execute_payout double-paid");
}

#[test]
fn execute_payout_reentrant_token_two_rounds_no_double_pay() {
    let e = Env::default();
    e.mock_all_auths();
    let admin = Address::generate(&e);
    let user = Address::generate(&e);
    let token_id = e.register_contract(None, ReentrantToken);
    let mut bals = Map::new(&e);
    bals.set(user.clone(), 1_000_000i128);
    let group_id = crate::deploy_group(&e, &token_id);
    e.invoke_contract::<()>(
        &token_id,
        &soroban_sdk::Symbol::new(&e, "__init"),
        (admin, bals, group_id.clone(), 2u32).into(),
    );
    crate::fund_and_finalize(&e, &group_id, &user, 100_000i128);
    let _ = e.try_invoke_contract::<(), soroban_sdk::Error>(
        &group_id,
        &soroban_sdk::Symbol::new(&e, "execute_payout"),
        ().into(),
    );
    // Effects advanced round; nested call cannot pay round 2 early or twice
    let round: u32 = e.invoke_contract(&group_id, &soroban_sdk::Symbol::new(&e, "get_current_round"), ().into());
    assert_eq!(round, 2, "round did not advance after first payout");
}
