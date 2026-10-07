use near_sdk::test_utils::{accounts, VMContextBuilder};
use near_sdk::testing_env;
use susu_factory::Factory;
use susu_factory::config::FactoryConfig;
use near_sdk::AccountId;
use near_sdk::NearToken;

fn setup() -> Factory {
    let builder = VMContextBuilder::new();
    let context = builder
        .signer_account_id(accounts(0))
        .current_account_id(accounts(0))
        .attached_deposit(NearToken::from_near(100))
        .build();
    testing_env!(context);

    Factory {
        config: FactoryConfig {
            admins: vec![accounts(0)],
            treasury: accounts(1),
            fees_bps: 100,
            paused: false,
        },
        groups: near_sdk::collections::LookupMap::new(b"g"),
        instance_ttls: near_sdk::collections::LookupMap::new(b"t"),
    }
}

#[test]
fn test_get_config_after_ttl_threshold() {
    let mut factory = setup();
    
    // Set an old TTL to simulate archive condition
    let instance = accounts(2);
    factory.instance_ttls.insert(
        instance.clone(),
        0, // timestamp in the past
    );
    
    // get_config should still work even with old TTL
    let config = factory.get_config().unwrap();
    assert_eq!(config.fees_bps, 100);
}

#[test]
fn test_get_group_count_extends_ttl() {
    let mut factory = setup();
    
    let instance = accounts(2);
    factory.instance_ttls.insert(
        instance.clone(),
        1000,
    );
    
    // This should extend the TTL
    let count = factory.get_group_count(instance.to_string()).unwrap();
    assert_eq!(count, 0);
    
    // TTL should have been extended
    let new_ttl = factory.instance_ttls.get(&instance).unwrap();
    assert!(new_ttl > 1000);
}

#[test]
fn test_pause_before_authorization_does_not_panic() {
    let mut factory = setup();
    
    // Non-admin caller
    let non_admin = accounts(3);
    testing_env!(VMContextBuilder::new()
        .signer_account_id(non_admin.clone())
        .current_account_id(accounts(0))
        .attached_deposit(NearToken::from_near(0))
        .build());
    
    // Should return error, not panic
    let result = factory.pause();
    assert!(result.is_err());
}
