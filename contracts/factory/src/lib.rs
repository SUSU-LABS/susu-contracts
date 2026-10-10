#![no_std]

use soroban_sdk::{contract, contractimpl, contracttype, symbol_short, Address, BytesN, Env, Symbol};

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    Admin,
    FeeBps,
    Treasury,
    SusuWasmHash,
}

#[contract]
pub struct SusuFactory;

#[contractimpl]
impl SusuFactory {
    pub fn initialize(
        env: Env,
        admin: Address,
        fee_bps: u32,
        treasury: Address,
        susu_wasm_hash: BytesN<32>,
    ) {
        if env.storage().instance().has(&DataKey::Admin) {
            panic!("already initialized");
        }
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::FeeBps, &fee_bps);
        env.storage().instance().set(&DataKey::Treasury, &treasury);
        env.storage().instance().set(&DataKey::SusuWasmHash, &susu_wasm_hash);
    }

    pub fn set_fee_bps(env: Env, fee_bps: u32) {
        let admin: Address = env.storage().instance().get(&DataKey::Admin).unwrap();
        admin.require_auth();

        if fee_bps > 10000 {
            panic!("fee_bps cannot exceed 10000");
        }

        let previous_fee_bps: u32 = env.storage().instance().get(&DataKey::FeeBps).unwrap_or(0);
        env.storage().instance().set(&DataKey::FeeBps, &fee_bps);

        env.events().publish(
            (Symbol::new(&env, "FeeUpdated"),),
            (previous_fee_bps, fee_bps),
        );
    }

    pub fn set_treasury(env: Env, treasury: Address) {
        let admin: Address = env.storage().instance().get(&DataKey::Admin).unwrap();
        admin.require_auth();

        let previous_treasury: Address = env.storage().instance().get(&DataKey::Treasury).unwrap();
        env.storage().instance().set(&DataKey::Treasury, &treasury);

        env.events().publish(
            (Symbol::new(&env, "TreasuryUpdated"),),
            (previous_treasury, treasury),
        );
    }

    pub fn fee_bps(env: Env) -> u32 {
        env.storage().instance().get(&DataKey::FeeBps).unwrap_or(0)
    }

    pub fn treasury(env: Env) -> Address {
        env.storage().instance().get(&DataKey::Treasury).unwrap()
    }

    pub fn admin(env: Env) -> Address {
        env.storage().instance().get(&DataKey::Admin).unwrap()
    }
}
