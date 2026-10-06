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
use susu_factory::{FactoryContract, FactoryContractClient};
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
