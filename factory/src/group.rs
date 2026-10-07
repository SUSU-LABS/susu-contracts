use borsh::{BorshDeserialize, BorshSerialize};
use near_sdk::{AccountId, BorshStorageKey};

#[derive(BorshSerialize, BorshDeserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct GroupConfig {
    pub max_holders: u64,
    pub transferable: bool,
    pub fees_bps: u16,
    pub royalty_fee_bps: u16,
    pub pause: bool,
}

#[derive(BorshSerialize, BorshDeserialize, Clone, Debug, PartialEq, Eq)]
pub struct Group {
    pub instance: AccountId,
    pub name: String,
    pub admin: AccountId,
    pub treasury: AccountId,
    pub nft_contract: AccountId,
    pub total_groups: u64,
    pub next_group_id: u64,
    pub config: GroupConfig,
}
