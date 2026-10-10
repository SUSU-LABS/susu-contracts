#![cfg(test)]

use super::*;
use soroban_sdk::{testutils::{Address as _, Events}, Address, BytesN, Env, IntoVal};

#[test]
fn test_factory_events() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let treasury = Address::generate(&env);
    let new_treasury = Address::generate(&env);
    let susu_wasm_hash = BytesN::from_array(&env, &[0; 32]);

    let factory_id = env.register_contract(None, SusuFactory);
    let client = SusuFactoryClient::new(&env, &factory_id);

    client.initialize(&admin, &500, &treasury, &susu_wasm_hash);

    // Test FeeUpdated event
    client.set_fee_bps(&1000);
    let events = env.events().all();
    let last_event = events.last().unwrap();
    assert_eq!(
        last_event,
        (
            factory_id.clone(),
            (Symbol::new(&env, "FeeUpdated"),).into_val(&env),
            (500u32, 1000u32).into_val(&env)
        )
    );

    // Test TreasuryUpdated event
    client.set_treasury(&new_treasury);
    let events = env.events().all();
    let last_event = events.last().unwrap();
    assert_eq!(
        last_event,
        (
            factory_id.clone(),
            (Symbol::new(&env, "TreasuryUpdated"),).into_val(&env),
            (treasury, new_treasury).into_val(&env)
        )
    );
}
