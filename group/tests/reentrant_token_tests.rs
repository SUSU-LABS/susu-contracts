use susu_contracts::reentrant_token::{ReentrantToken, Mode};
use susu_contracts::effects::Effects;
use susu_contracts::token::Token;
use sui_sdk::types::base_types::SuiAddress;
use sui_sdk::types::object::ObjectID;
use sui_test_framework::{get_signer, initialize_balances, register_object, set_balance};
use std::collections::HashMap;

fn setup() -> (ReentrantToken, Effects) {
    let id = ObjectID::random();
    let mut token = ReentrantToken::new(id);
    let effects = Effects::new();
    (token, effects)
}

fn make_recipients() -> HashMap<SuiAddress, u64> {
    let mut map = HashMap::new();
    let addr1 = get_signer(0).object_address();
    let addr2 = get_signer(1).object_address();
    map.insert(addr1, 100);
    map.insert(addr2, 200);
    map
}

#[sui_test_framework::test]
fn test_reentrant_token_double_pay_across_rounds() {
    let (mut token, mut effects) = setup();

    // Register two round-robins with different payouts
    let rr1_id = register_object(&mut token, &mut effects, Mode::RoundRobin);
    let rr2_id = register_object(&mut token, &mut effects, Mode::RoundRobin);

    let recipients = make_recipients();
    let addr1 = recipients.keys().next().unwrap().to_owned();
    let addr2 = recipients.values().next().unwrap().to_owned();

    // Seed some SUI into the token
    set_balance(addr1, 1000);
    set_balance(addr2, 1000);
    initialize_balances(&mut token, &mut effects, &recipients);

    // Round 1: normal payout
    token.execute_payout(&mut effects, &rr1_id, &recipients).unwrap();
    assert_eq!(effects.round(), 1);

    // Verify balances after round 1
    let balance1_after_r1 = get_balance(addr1);
    let balance2_after_r1 = get_balance(addr2);

    // Now advance round and attempt a second payout using the SAME round-robin
    // The re-entrant token should NOT be able to double-pay across rounds.
    token.execute_payout(&mut effects, &rr1_id, &recipients).unwrap();
    assert_eq!(effects.round(), 2);

    // Verify that the balances did NOT change from round 1 to round 2
    // (no double payment occurred)
    assert_eq!(get_balance(addr1), balance1_after_r1);
    assert_eq!(get_balance(addr2), balance2_after_r1);

    // Contribute after round advance should still work correctly
    let contribution = 500u64;
    token.contribute(&mut effects, &addr1, contribution).unwrap();
    assert_eq!(effects.round(), 2);

    // Final payout should distribute exactly once per recipient
    token.execute_payout(&mut effects, &rr2_id, &recipients).unwrap();
    assert_eq!(effects.round(), 3);
}

fn get_balance(addr: SuiAddress) -> u64 {
    // Return the balance of the address
    sui_sdk::framework::sui_system::get_balance(addr)
}
