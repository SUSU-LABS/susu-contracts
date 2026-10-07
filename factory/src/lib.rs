use borsh::{BorshDeserialize, BorshSerialize};
use near_sdk::borsh::{self};
use near_sdk::collections::LookupMap;
use near_sdk::serde::{Deserialize, Serialize};
use near_sdk::{env, log, near_bindgen, require, AccountId, Balance, NearToken, PanicOnDefault, Promise};
use near_sdk::{get_promise_error, schedule_async_callback, storage_context, setup_init_error, PromiseCreateEvent, PromiseResult};

use susu_macros::SUSU_CONTRACT;
use susu_utils::*;

mod config;
pub use config::*;
mod group;
pub use group::*;

use crate::config::FactoryConfig;
use crate::group::Group;

const INSTANCE_TTL_THRESHOLD: u64 = 60 * 60 * 24 * 30; // 30 days

#[near_bindgen]
#[derive(BorshDeserialize, BorshSerialize, PanicOnDefault, SUSU_CONTRACT)]
pub struct Factory {
    /// All groups created by the factory.
    pub groups: LookupMap<GroupKey, Group>,

    /// Configuration settings for the factory.
    pub config: FactoryConfig,

    /// Mapping of account_id -> instance_ttl (timestamp when the instance will be archived)
    pub instance_ttls: LookupMap<AccountId, u64>,
}

#[derive(BorshSerialize, BorshDeserialize, Clone, Debug, PartialEq, Eq, Hash)]
pub struct GroupKey {
    pub instance: AccountId,
    pub name: String,
}

impl GroupKey {
    pub fn new(instance: &str, name: &str) -> Self {
        Self {
            instance: instance.parse().unwrap(),
            name: name.to_string(),
        }
    }
}

impl Factory {
    pub fn require_admin(&self) {
        let caller = env::caller();
        require!(
            self.config.admins.contains(&caller),
            "Caller is not an admin"
        );
    }

    pub fn extend_instance_ttl(&mut self, instance: &AccountId) {
        let now = env::block_timestamp();
        let ttl = now + INSTANCE_TTL_THRESHOLD;
        self.instance_ttls.insert(instance.clone(), ttl);
    }

    pub fn extend_instance_ttl_on_read(&mut self, instance: &AccountId) {
        if self.instance_ttls.get(instance).is_some() {
            self.extend_instance_ttl(instance);
        }
    }
}

/// Factory events
#[derive(BorshSerialize, BorshDeserialize)]
pub enum FactoryEvent {
    /// A new group was created.
    #[inarbitrary(rename = "CREATED_GROUP")]
    CreatedGroup {
        group: Group,
        group_key: GroupKey,
        group_owner: AccountId,
        fees_bps: u16,
    },
    /// The factory config was updated.
    #[inarbitrary(rename = "FACTORY_CONFIG_UPDATED")]
    ConfigUpdated {
        fees_bps: u16,
        paused: bool,
        treasury: AccountId,
        admins: Vec<AccountId>,
    },
}

// =============================================
// Public view methods
// =============================================

impl Factory {
    #[inline]
    pub fn get_group(&self, instance: &str, group_name: &str) -> Option<Group> {
        let key = GroupKey::new(instance, group_name);
        self.groups.get(&key)
    }

    #[inline]
    pub fn get_group_count(&mut self, instance: &str) -> u64 {
        let instance_account_id: AccountId = instance.parse().expect("Invalid instance account");
        self.extend_instance_ttl_on_read(&instance_account_id);
        self.groups.iter_prefix(&instance_account_id.to_string()).count() as u64
    }

    pub fn get_all_group_names(&self, instance: &str) -> Vec<String> {
        self.groups
            .iter_prefix(&instance.parse::<AccountId>().expect("Invalid instance"))
            .map(|(_, group)| group.name.clone())
            .collect()
    }
}

/// Public factory read methods
#[near_bindgen]
impl Factory {
    #[handle_result]
    pub fn get_config(&self) -> Result<FactoryConfig, String> {
        let config = self.config.clone();
        Ok(config)
    }

    #[handle_result]
    pub fn get_group_count(&mut self, instance: String) -> Result<u64, String> {
        let group_count = self.get_group_count(&instance);
        Ok(group_count)
    }

    #[handle_result]
    pub fn get_group(
        &self,
        instance: String,
        group_name: String,
    ) -> Result<Option<Group>, String> {
        let group = self.get_group(&instance, &group_name);
        Ok(group)
    }

    #[handle_result]
    pub fn get_all_group_names(&self, instance: String) -> Result<Vec<String>, String> {
        let names = self.get_all_group_names(&instance);
        Ok(names)
    }
}

/// Public factory methods
#[near_bindgen]
impl Factory {
    #[handle_result]
    pub fn create_group(
        &mut self,
        instance: String,
        name: String,
        nft: AccountId,
    ) -> Result<(), String> {
        let instance_account_id: AccountId = instance.parse().expect("Invalid instance account");
        self.extend_instance_ttl(&instance_account_id);

        // Check if the factory is paused
        if self.config.paused {
            return Err("Factory is paused".to_string());
        }

        // Get admin status from caller
        let caller = env::caller();
        if !self.config.admins.contains(&caller) {
            return Err("Caller is not an admin".to_string());
        }

        // Check if group already exists
        let group_key = GroupKey {
            instance: instance_account_id.clone(),
            name: name.clone(),
        };
        if self.groups.contains_key(&group_key) {
            return Err("Group already exists".to_string());
        }

        // Create group with initial owner
        let group = Group {
            instance: instance_account_id.clone(),
            name: name.clone(),
            admin: caller.clone(),
            treasury: self.config.treasury.clone(),
            nft_contract: nft.clone(),
            total_groups: 0,
            next_group_id: 1,
            config: group::GroupConfig {
                max_holders: 1_000,
                transferable: true,
                fees_bps: self.config.fees_bps,
                royalty_fee_bps: 0,
                pause: false,
            },
        };

        // Store group
        self.groups.insert(&group_key, &group);

        // Emit event
        env::log_str(
            &serde_json::to_string(&FactoryEvent::CreatedGroup {
                group: group.clone(),
                group_key,
                group_owner: caller.clone(),
                fees_bps: self.config.fees_bps,
            })
            .unwrap(),
        );

        // Return the new group's ID (we don't have it yet, it's 0 at creation)
        Ok(())
    }

    #[handle_result]
    pub fn set_fee(&mut self, new_fee_bps: u16) -> Result<(), String> {
        self.require_admin();

        // Update fees
        self.config.fees_bps = new_fee_bps;

        // Emit event
        env::log_str(
            &serde_json::to_string(&FactoryEvent::ConfigUpdated {
                fees_bps: new_fee_bps,
                paused: self.config.paused,
                treasury: self.config.treasury.clone(),
                admins: self.config.admins.clone(),
            })
            .unwrap(),
        );

        Ok(())
    }

    #[handle_result]
    pub fn set_treasury(&mut self, new_treasury: AccountId) -> Result<(), String> {
        self.require_admin();

        // Update treasury
        self.config.treasury = new_treasury;

        // Emit event
        env::log_str(
            &serde_json::to_string(&FactoryEvent::ConfigUpdated {
                fees_bps: self.config.fees_bps,
                paused: self.config.paused,
                treasury: self.config.treasury.clone(),
                admins: self.config.admins.clone(),
            })
            .unwrap(),
        );

        Ok(())
    }

    #[handle_result]
    pub fn pause(&mut self) -> Result<(), String> {
        self.require_admin();
        self.config.paused = true;

        // Emit event
        env::log_str(
            &serde_json::to_string(&FactoryEvent::ConfigUpdated {
                fees_bps: self.config.fees_bps,
                paused: true,
                treasury: self.config.treasury.clone(),
                admins: self.config.admins.clone(),
            })
            .unwrap(),
        );

        Ok(())
    }

    #[handle_result]
    pub fn unpause(&mut self) -> Result<(), String> {
        self.require_admin();
        self.config.paused = false;

        // Emit event
        env::log_str(
            &serde_json::to_string(&FactoryEvent::ConfigUpdated {
                fees_bps: self.config.fees_bps,
                paused: false,
                treasury: self.config.treasury.clone(),
                admins: self.config.admins.clone(),
            })
            .unwrap(),
        );

        Ok(())
    }

    #[handle_result]
    pub fn add_admin(&mut self, admin: AccountId) -> Result<(), String> {
        self.require_admin();

        // Add admin
        self.config.admins.push(admin.clone());

        // Emit event
        env::log_str(
            &serde_json::to_string(&FactoryEvent::ConfigUpdated {
                fees_bps: self.config.fees_bps,
                paused: self.config.paused,
                treasury: self.config.treasury.clone(),
                admins: self.config.admins.clone(),
            })
            .unwrap(),
        );

        Ok(())
    }

    #[handle_result]
    pub fn remove_admin(&mut self, admin: AccountId) -> Result<(), String> {
        self.require_admin();

        // Remove admin - skip if not found
        self.config
            .admins
            .retain(|a| a != &admin);

        // Emit event
        env::log_str(
            &serde_json::to_string(&FactoryEvent::ConfigUpdated {
                fees_bps: self.config.fees_bps,
                paused: self.config.paused,
                treasury: self.config.treasury.clone(),
                admins: self.config.admins.clone(),
            })
            .unwrap(),
        );

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use near_sdk::testing_env;

    fn setup() -> Factory {
        let config = FactoryConfig {
            admins: vec!["admin.near".parse().unwrap()],
            treasury: "treasury.near".parse().unwrap(),
            fees_bps: 0,
            paused: false,
        };
        testing_env!({
            signer_account_id: "admin.near".parse().unwrap(),
            predecessor_account_id: "admin.near".parse().unwrap(),
            attached_deposit: NearToken::from_near(1),
        });
        Factory {
            config,
            groups: LookupMap::new(b"g"),
            instance_ttls: LookupMap::new(b"t"),
        }
    }

    #[test]
    fn test_get_config_returns_ok() {
        let factory = setup();
        let config = factory.get_config().unwrap();
        assert_eq!(config.fees_bps, 0);
    }

    #[test]
    fn test_get_group_count_returns_zero() {
        let mut factory = setup();
        let count = factory.get_group_count("group.near".to_string()).unwrap();
        assert_eq!(count, 0);
    }
}
