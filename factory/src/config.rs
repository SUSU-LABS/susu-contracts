use borsh::{BorshDeserialize, BorshSerialize};
use near_sdk::{AccountId, BorshStorageKey};

use super::Factory;

#[derive(BorshSerialize, BorshDeserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct FactoryConfig {
    pub admins: Vec<AccountId>,
    pub treasury: AccountId,
    pub fees_bps: u16,
    pub paused: bool,
}

impl FactoryConfig {
    pub fn new(admins: Vec<AccountId>, treasury: AccountId, fees_bps: u16) -> Self {
        Self {
            admins,
            treasury,
            fees_bps,
            paused: false,
        }
    }
}

#[derive(BorshSerialize, BorshDeserialize, Clone, Debug, BorshStorageKey)]
pub enum StorageKey {
    Groups,
    InstanceTtls,
}
