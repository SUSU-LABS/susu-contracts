//! Integration test: the Factory really deploys a working Group contract.
//!
//! This is the only test that exercises `create_group`'s deployment path, which
//! needs the compiled Group Wasm. It is gated behind the `wasm-integration` feature
//! so that a plain `cargo test` does not depend on build order.
//!
//! CI builds the Wasm first and then runs:
//! ```text
//! cargo test -p susu-factory --features wasm-integration
//! ```
//!
//! Locally: `./scripts/build-contracts.sh && cargo test -p susu-factory --features wasm-integration`

#![cfg(feature = "wasm-integration")]

use soroban_sdk::{
    testutils::{Address as _, Events as _},
    token, Address, Bytes, Env, Event as _,
};
use susu_factory::{FactoryContract, FactoryContractClient, FactoryError};
use susu_group::{GroupContractClient, Status};

/// The Group contract Wasm, built by `cargo build --target wasm32v1-none`.
const GROUP_WASM: &[u8] = include_bytes!("../../../target/wasm32v1-none/release/susu_group.wasm");

const ONE_USDC: i128 = 10_000_000;
const ONE_WEEK: u64 = 604_800;

struct Harness {
    env: Env,
    factory_id: Address,
    client: FactoryContractClient<'static>,
    token: Address,
    treasury: Address,
}

impl Harness {
    fn new() -> Self {
        let env = Env::default();
        env.mock_all_auths();

        let wasm_hash = env
            .deployer()
            .upload_contract_wasm(Bytes::from_slice(&env, GROUP_WASM));

        let admin = Address::generate(&env);
        let treasury = Address::generate(&env);
        let factory_id = env.register(FactoryContract, (admin, wasm_hash, treasury.clone(), 50u32));

        let token = env
            .register_stellar_asset_contract_v2(Address::generate(&env))
            .address();

        Self {
            client: FactoryContractClient::new(&env, &factory_id),
            env,
            factory_id,
            token,
            treasury,
        }
    }
}

#[test]
fn create_group_deploys_an_initialized_group() {
    let harness = Harness::new();
    let creator = Address::generate(&harness.env);

    let group_address =
        harness
            .client
            .create_group(&creator, &harness.token, &(10 * ONE_USDC), &3u32, &ONE_WEEK);

    // The group is registered and discoverable.
    assert_eq!(harness.client.get_group(&1u32), group_address);
    assert_eq!(harness.client.get_group_count(), 1);

    // The constructor ran with exactly the configuration the Factory holds.
    let group = GroupContractClient::new(&harness.env, &group_address);
    let state = group.get_group();

    assert_eq!(state.config.factory, harness.factory_id);
    assert_eq!(state.config.creator, creator);
    assert_eq!(state.config.token, harness.token);
    assert_eq!(state.config.treasury, harness.treasury);
    assert_eq!(state.config.contribution_amount, 10 * ONE_USDC);
    assert_eq!(state.config.member_capacity, 3);
    assert_eq!(state.config.frequency_seconds, ONE_WEEK);
    assert_eq!(state.config.fee_bps, 50);
    assert_eq!(state.status, Status::Open);
    assert_eq!(state.member_count, 0);

    // The created group is fully operational.
    let member = Address::generate(&harness.env);
    assert_eq!(group.join(&member), 1);
}

#[test]
fn create_group_assigns_distinct_addresses_and_monotonic_ids() {
    let harness = Harness::new();
    let creator = Address::generate(&harness.env);

    let first =
        harness
            .client
            .create_group(&creator, &harness.token, &(10 * ONE_USDC), &3u32, &ONE_WEEK);
    let second =
        harness
            .client
            .create_group(&creator, &harness.token, &(20 * ONE_USDC), &2u32, &ONE_WEEK);

    assert_ne!(first, second);
    assert_eq!(harness.client.get_group(&1u32), first);
    assert_eq!(harness.client.get_group(&2u32), second);
    assert_eq!(harness.client.get_group_count(), 2);

    // Each group holds its own configuration.
    let first_state = GroupContractClient::new(&harness.env, &first).get_group();
    let second_state = GroupContractClient::new(&harness.env, &second).get_group();
    assert_eq!(first_state.config.contribution_amount, 10 * ONE_USDC);
    assert_eq!(second_state.config.contribution_amount, 20 * ONE_USDC);
    assert_eq!(first_state.config.member_capacity, 3);
    assert_eq!(second_state.config.member_capacity, 2);
}

#[test]
fn group_created_carries_the_terms_frozen_at_creation_not_the_current_config() {
    // The Factory's fee and treasury change later via `set_fee` / `set_treasury`, so
    // the creation event is the only time-stamped record of what each group froze.
    let harness = Harness::new();
    let creator = Address::generate(&harness.env);

    let first =
        harness
            .client
            .create_group(&creator, &harness.token, &(10 * ONE_USDC), &3u32, &ONE_WEEK);
    // `events().all()` only reflects the most recent invocation, so capture now.
    let first_events = harness
        .env
        .events()
        .all()
        .filter_by_contract(&harness.factory_id);

    let new_treasury = Address::generate(&harness.env);
    harness.client.set_fee(&25u32);
    harness.client.set_treasury(&new_treasury);

    let second = harness.client.create_group(
        &creator,
        &harness.token,
        &(20 * ONE_USDC),
        &2u32,
        &(2 * ONE_WEEK),
    );
    let second_events = harness
        .env
        .events()
        .all()
        .filter_by_contract(&harness.factory_id);

    let first_expected = susu_factory::GroupCreated {
        creator: creator.clone(),
        group: first.clone(),
        group_id: 1,
        token: harness.token.clone(),
        contribution_amount: 10 * ONE_USDC,
        member_capacity: 3,
        fee_bps: 50,
        treasury: harness.treasury.clone(),
        frequency_seconds: ONE_WEEK,
    }
    .to_xdr(&harness.env, &harness.factory_id);
    let second_expected = susu_factory::GroupCreated {
        creator,
        group: second.clone(),
        group_id: 2,
        token: harness.token.clone(),
        contribution_amount: 20 * ONE_USDC,
        member_capacity: 2,
        fee_bps: 25,
        treasury: new_treasury.clone(),
        frequency_seconds: 2 * ONE_WEEK,
    }
    .to_xdr(&harness.env, &harness.factory_id);
    assert!(first_events.events().contains(&first_expected));
    assert!(second_events.events().contains(&second_expected));

    // Each event agrees with what the group itself froze at construction.
    let first_config = GroupContractClient::new(&harness.env, &first)
        .get_group()
        .config;
    assert_eq!(first_config.fee_bps, 50);
    assert_eq!(first_config.treasury, harness.treasury);
    assert_eq!(first_config.frequency_seconds, ONE_WEEK);
    let second_config = GroupContractClient::new(&harness.env, &second)
        .get_group()
        .config;
    assert_eq!(second_config.fee_bps, 25);
    assert_eq!(second_config.treasury, new_treasury);
    assert_eq!(second_config.frequency_seconds, 2 * ONE_WEEK);
}

#[test]
fn a_deployed_group_runs_a_full_cycle() {
    // The Factory's job is to produce a group that is correct; verify one end to end.
    let harness = Harness::new();
    let creator = Address::generate(&harness.env);

    let group_address =
        harness
            .client
            .create_group(&creator, &harness.token, &(10 * ONE_USDC), &3u32, &ONE_WEEK);
    // Capture the Factory's events now: `events().all()` only reflects the most
    // recent invocation, and the rest of this test drives the deployed Group.
    let created_events = harness
        .env
        .events()
        .all()
        .filter_by_contract(&harness.factory_id);
    let group = GroupContractClient::new(&harness.env, &group_address);

    let mut members = soroban_sdk::Vec::new(&harness.env);
    let asset_client = token::StellarAssetClient::new(&harness.env, &harness.token);
    let mut index = 0;
    while index < 3 {
        let member = Address::generate(&harness.env);
        asset_client.mint(&member, &(100 * ONE_USDC));
        group.join(&member);
        members.push_back(member);
        index += 1;
    }
    group.start();

    let mut round = 1;
    while round <= 3 {
        let mut i = 0;
        while i < 3 {
            group.contribute(&members.get(i).unwrap(), &(10 * ONE_USDC), &round);
            i += 1;
        }
        group.execute_payout();
        round += 1;
    }

    assert_eq!(group.get_status(), Status::Completed);

    // Fee = 0.5% of 30 USDC = 0.15 USDC per round, three rounds.
    let token_client = token::Client::new(&harness.env, &harness.token);
    assert_eq!(token_client.balance(&harness.treasury), 3 * 1_500_000);
    assert_eq!(token_client.balance(&group_address), 0);

    // The group was created by the Factory, and the Factory emitted an event for it.
    let expected = susu_factory::GroupCreated {
        creator,
        group: group_address.clone(),
        group_id: 1,
        token: harness.token.clone(),
        contribution_amount: 10 * ONE_USDC,
        member_capacity: 3,
        fee_bps: 50,
        treasury: harness.treasury.clone(),
        frequency_seconds: ONE_WEEK,
    }
    .to_xdr(&harness.env, &harness.factory_id);
    assert!(created_events.events().contains(&expected));
}

#[test]
fn factory_config_changes_only_affect_future_groups() {
    let harness = Harness::new();
    let creator = Address::generate(&harness.env);

    // 1. Create Group A with the factory's initial fee (50 bps) and initial treasury.
    let group_a_address =
        harness
            .client
            .create_group(&creator, &harness.token, &(10 * ONE_USDC), &3u32, &ONE_WEEK);
    let group_a = GroupContractClient::new(&harness.env, &group_a_address);
    assert_eq!(group_a.get_group().config.fee_bps, 50);
    assert_eq!(group_a.get_group().config.treasury, harness.treasury);

    // 2. Admin updates fee to 10 bps and updates the treasury address.
    let new_treasury = Address::generate(&harness.env);
    harness.client.set_fee(&10u32);
    harness.client.set_treasury(&new_treasury);

    // 3. Create Group B after configuration changes.
    let group_b_address =
        harness
            .client
            .create_group(&creator, &harness.token, &(10 * ONE_USDC), &2u32, &ONE_WEEK);
    let group_b = GroupContractClient::new(&harness.env, &group_b_address);

    // 4. Assert Group A's frozen terms are preserved and Group B received the new terms.
    let state_a = group_a.get_group();
    assert_eq!(state_a.config.fee_bps, 50);
    assert_eq!(state_a.config.treasury, harness.treasury);

    let state_b = group_b.get_group();
    assert_eq!(state_b.config.fee_bps, 10);
    assert_eq!(state_b.config.treasury, new_treasury);

    // 5. Verify financial execution honors the frozen terms independently.
    let asset_client = token::StellarAssetClient::new(&harness.env, &harness.token);
    let token_client = token::Client::new(&harness.env, &harness.token);

    // Join and run round 1 of Group A (3 members x 10 USDC = 30 USDC, 50 bps fee = 0.15 USDC)
    let a_m0 = Address::generate(&harness.env);
    let a_m1 = Address::generate(&harness.env);
    let a_m2 = Address::generate(&harness.env);
    for m in [&a_m0, &a_m1, &a_m2] {
        asset_client.mint(m, &(100 * ONE_USDC));
        group_a.join(m);
    }
    group_a.start();
    for m in [&a_m0, &a_m1, &a_m2] {
        group_a.contribute(m, &(10 * ONE_USDC), &1u32);
    }
    group_a.execute_payout();
    assert_eq!(token_client.balance(&harness.treasury), 1_500_000);

    // Join and run round 1 of Group B (2 members x 10 USDC = 20 USDC, 10 bps fee = 0.02 USDC)
    let b_m0 = Address::generate(&harness.env);
    let b_m1 = Address::generate(&harness.env);
    for m in [&b_m0, &b_m1] {
        asset_client.mint(m, &(100 * ONE_USDC));
        group_b.join(m);
    }
    group_b.start();
    for m in [&b_m0, &b_m1] {
        group_b.contribute(m, &(10 * ONE_USDC), &1u32);
    }
    group_b.execute_payout();
    assert_eq!(token_client.balance(&new_treasury), 200_000);
}

#[test]
fn factory_pause_does_not_affect_existing_groups() {
    let harness = Harness::new();
    let creator = Address::generate(&harness.env);

    // 1. Deploy and initialize group while factory is running normally.
    let group_address =
        harness
            .client
            .create_group(&creator, &harness.token, &(10 * ONE_USDC), &2u32, &ONE_WEEK);
    let group = GroupContractClient::new(&harness.env, &group_address);

    let m1 = Address::generate(&harness.env);
    let m2 = Address::generate(&harness.env);
    let asset_client = token::StellarAssetClient::new(&harness.env, &harness.token);
    asset_client.mint(&m1, &(50 * ONE_USDC));
    asset_client.mint(&m2, &(50 * ONE_USDC));

    group.join(&m1);
    group.join(&m2);
    group.start();

    // 2. Factory is paused by admin.
    harness.client.pause();
    assert!(harness.client.get_config().paused);

    // 3. New group creation is refused.
    assert_eq!(
        harness.client.try_create_group(
            &creator,
            &harness.token,
            &(10 * ONE_USDC),
            &2u32,
            &ONE_WEEK,
        ),
        Err(Ok(FactoryError::Paused))
    );

    // 4. Existing group continues its lifecycle uninterrupted: contribute and payout.
    group.contribute(&m1, &(10 * ONE_USDC), &1u32);
    group.contribute(&m2, &(10 * ONE_USDC), &1u32);
    group.execute_payout();

    // Round 2
    group.contribute(&m1, &(10 * ONE_USDC), &2u32);
    group.contribute(&m2, &(10 * ONE_USDC), &2u32);
    group.execute_payout();

    assert_eq!(group.get_status(), Status::Completed);
}

#[test]
fn create_group_rejects_token_equal_to_factory() {
    let harness = Harness::new();
    let creator = Address::generate(&harness.env);

    assert_eq!(
        harness.client.try_create_group(
            &creator,
            &harness.factory_id,
            &(10 * ONE_USDC),
            &3u32,
            &ONE_WEEK,
        ),
        Err(Ok(FactoryError::InvalidToken))
    );
}
