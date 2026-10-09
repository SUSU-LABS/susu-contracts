use cosmwasm_std::{Addr, DepsMut, Response, SubMsg};
use cw_multi_test::{App, Contract, ContractWrapper, Executor};

// Import the factory contract modules
// Note: Adjust import paths if the actual crate name differs (e.g., if it's just 'factory')
// Based on standard naming conventions for susu-labs, it's likely 'susu_factory'
use susu_factory::contract::{execute, instantiate};
use susu_factory::msg::{InstantiateMsg, SetTreasuryMsg};
use susu_factory::error::FactoryError;

fn mock_app() -> App {
    App::default()
}

fn factory_contract() -> Box<dyn Contract<cosmwasm_std::Empty>> {
    let code = ContractWrapper::new(execute, instantiate, None);
    Box::new(code)
}

#[test]
fn test_set_treasury_rejects_admin() {
    let mut app = mock_app();
    let owner = Addr::unchecked("owner");
    
    let factory_code_id = app.store_code(factory_contract());
    let factory_addr = app
        .instantiate_contract(
            factory_code_id,
            owner.clone(),
            &InstantiateMsg {},
            &[],
            "Factory",
            None,
        )
        .unwrap();

    // Attempt to set treasury to admin (owner)
    let res = app.execute_contract(
        owner.clone(),
        factory_addr.clone(),
        &SetTreasuryMsg {
            new_treasury: owner.clone().into(),
        },
        &[],
    );

    assert_eq!(
        res.unwrap_err().root_cause().to_string(),
        FactoryError::InvalidTreasury {}.to_string()
    );
}

#[test]
fn test_set_treasury_rejects_self() {
    let mut app = mock_app();
    let owner = Addr::unchecked("owner");
    
    let factory_code_id = app.store_code(factory_contract());
    let factory_addr = app
        .instantiate_contract(
            factory_code_id,
            owner.clone(),
            &InstantiateMsg {},
            &[],
            "Factory",
            None,
        )
        .unwrap();

    // Attempt to set treasury to factory address itself
    let res = app.execute_contract(
        owner.clone(),
        factory_addr.clone(),
        &SetTreasuryMsg {
            new_treasury: factory_addr.clone().into(),
        },
        &[],
    );

    assert_eq!(
        res.unwrap_err().root_cause().to_string(),
        FactoryError::InvalidTreasury {}.to_string()
    );
}

#[test]
fn test_set_treasury_valid_success() {
    let mut app = mock_app();
    let owner = Addr::unchecked("owner");
    let new_treasury = Addr::unchecked("new_treasury");
    
    let factory_code_id = app.store_code(factory_contract());
    let factory_addr = app
        .instantiate_contract(
            factory_code_id,
            owner.clone(),
            &InstantiateMsg {},
            &[],
            "Factory",
            None,
        )
        .unwrap();

    // Attempt to set treasury to a valid new address
    let res = app.execute_contract(
        owner.clone(),
        factory_addr.clone(),
        &SetTreasuryMsg {
            new_treasury: new_treasury.into(),
        },
        &[],
    );

    // Should succeed without error
    assert!(res.is_ok());
}
