//! # Susu Protocol — Factory Contract
//!
//! The Factory deploys and registers one dedicated **Group** contract per Susu group.
//!
//! ## Authority
//! The Factory is **not** a custodian:
//! - It never holds group funds and has no function that can move them.
//! - It cannot withdraw from, or spend on behalf of, any group.
//! - It has no authority over a group's members, rounds, recipient or payouts.
//! - Groups capture their own configuration at construction, so later Factory
//!   configuration changes (fee, treasury) **cannot** reach an existing group.
//!
//! Admin authority is limited to protocol configuration: the fee ceiling, the
//! treasury address used for future groups, and pausing *new* group creation. It can
//! never touch an existing group or its money.
//!
//! ## Deployment model
//! `create_group` derives a deterministic salt from the group id and deploys the
//! group wasm with `deploy_v2`, passing the constructor arguments. The group is
//! therefore fully initialized in the same transaction that creates it, and its
//! address is reproducible from `(factory address, group id)`.
//!
//! ## Status: Phase 1
//! See `docs/CONTRACT_SPEC.md` for the full specification.
// Contract entry points and their generated clients take one argument per ABI
// parameter. The argument counts are fixed by the protocol ABI, not by this
// implementation, so the lint is allowed crate-wide.
#![allow(clippy::too_many_arguments)]
#![no_std]

use soroban_sdk::{
    contract, contracterror, contractevent, contractimpl, contracttype, Address, BytesN, Env,
};

/// Contract version reported by `version()`. Bumped with each released interface.
const CONTRACT_VERSION: u32 = 3;

/// Protocol maximum fee, in basis points. The MVP protocol fee is 0.50%.
///
/// The Factory can never be configured above this, and it will never deploy a group
/// above this. Changing it is a change to a financial invariant and requires human
/// review.
pub const MAX_FEE_BPS: u32 = 50;

/// Minimum members a group can be created with. Mirrors the Group contract's bound.
pub const MIN_MEMBERS: u32 = 2;

/// Maximum members a group can be created with. Mirrors the Group contract's bound.
pub const MAX_MEMBERS: u32 = 100;

/// Protocol configuration. Applies to groups created *after* a change.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FactoryConfig {
    /// Protocol admin. May only configure the fee, treasury and pause state.
    pub admin: Address,
    /// Fee recipient for groups created from now on. Never holds group funds.
    pub treasury: Address,
    /// Fee in basis points applied to newly created groups, `<= MAX_FEE_BPS`.
    pub fee_bps: u32,
    /// Hash of the Group contract wasm that `create_group` deploys.
    pub group_wasm_hash: BytesN<32>,
    /// When true, new group creation is disabled. Existing groups are unaffected.
    pub paused: bool,
}

/// Storage keys.
#[contracttype]
pub enum DataKey {
    /// `FactoryConfig`.
    Config,
    /// `u32` — number of groups created so far. Also the most recent group id.
    GroupCount,
    /// `u32` (group id) -> `Address` — the deployed group contract.
    Group(u32),
}

/// Errors returned by the Factory contract.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum FactoryError {
    /// Fee exceeds `MAX_FEE_BPS`, or is zero.
    InvalidFeeBps = 1,
    /// Contribution amount is zero or less.
    InvalidContributionAmount = 2,
    /// Member capacity is outside `[MIN_MEMBERS, MAX_MEMBERS]`.
    InvalidMemberCapacity = 3,
    /// Round frequency is zero.
    InvalidFrequency = 4,
    /// New group creation is paused.
    Paused = 5,
    /// No group exists with that id.
    GroupNotFound = 6,
    /// Integer arithmetic overflowed. Never expected for valid inputs.
    ArithmeticOverflow = 7,
    /// Treasury address cannot be the admin or the factory itself.
    InvalidTreasury = 8,
    /// The Factory has no configuration entry. Reported instead of panicking so
    /// the admin can still reach `pause` and recover a contract whose instance
    /// entry has archived.
    NotInitialized = 9,
}

/// A new group contract was deployed and registered.
#[contractevent(topics = ["susu", "group_created"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GroupCreated {
    #[topic]
    pub creator: Address,
    #[topic]
    pub group: Address,
    pub group_id: u32,
    pub token: Address,
    pub contribution_amount: i128,
    pub member_capacity: u32,
    pub fee_bps: u32,
    pub treasury: Address,
    pub frequency_seconds: u64,
}

/// The protocol fee applied to newly created groups changed.
#[contractevent(topics = ["susu", "fee_updated"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FeeUpdated {
    pub fee_bps: u32,
}

/// The treasury for newly created groups changed.
#[contractevent(topics = ["susu", "treasury_updated"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TreasuryUpdated {
    #[topic]
    pub treasury: Address,
}

/// New group creation was paused or unpaused.
#[contractevent(topics = ["susu", "pause_updated"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PauseUpdated {
    pub paused: bool,
}

/// The Susu Factory contract type.
#[contract]
pub struct FactoryContract;

#[contractimpl]
impl FactoryContract {
    /// Construct the Factory.
    ///
    /// This is the SDK-idiomatic replacement for the `initialize(...)` entry point
    /// named in the specification: deploying with a constructor (via `deploy_v2`)
    /// means the Factory is never observable in an uninitialized state, so its admin
    /// cannot be claimed by a third party in a front-running transaction. The
    /// parameters and validation are otherwise exactly as specified.
    pub fn __constructor(
        env: Env,
        admin: Address,
        group_wasm_hash: BytesN<32>,
        treasury: Address,
        fee_bps: u32,
    ) {
        if fee_bps == 0 || fee_bps > MAX_FEE_BPS {
            soroban_sdk::panic_with_error!(&env, FactoryError::InvalidFeeBps);
        }
        if treasury == admin || treasury == env.current_contract_address() {
            soroban_sdk::panic_with_error!(&env, FactoryError::InvalidTreasury);
        }
        let config = FactoryConfig {
            admin,
            treasury,
            fee_bps,
            group_wasm_hash,
            paused: false,
        };
        env.storage().instance().set(&DataKey::Config, &config);
        env.storage().instance().set(&DataKey::GroupCount, &0u32);
    }

    /// Deploy and register a new group.
    ///
    /// The creator authorizes the call and becomes the group's recorded creator,
    /// which is informational only. They receive **no** special authority over the
    /// group: they cannot withdraw from it, cannot change its configuration, and
    /// cannot prevent a payout. The group's `fee_bps` and `treasury` are taken from
    /// the Factory's current configuration and are frozen into the group at
    /// construction.
    ///
    /// Returns the address of the deployed group contract.
    pub fn create_group(
        env: Env,
        creator: Address,
        token: Address,
        contribution_amount: i128,
        member_capacity: u32,
        frequency_seconds: u64,
    ) -> Result<Address, FactoryError> {
        creator.require_auth();
        extend_instance_ttl(&env);

        let storage = env.storage().instance();
        let config = load_config(&env)?;

        if config.paused {
            return Err(FactoryError::Paused);
        }
        if !(MIN_MEMBERS..=MAX_MEMBERS).contains(&member_capacity) {
            return Err(FactoryError::InvalidMemberCapacity);
        }
        if frequency_seconds == 0 {
            return Err(FactoryError::InvalidFrequency);
        }
        if contribution_amount <= 0
            || contribution_amount
                .checked_mul(member_capacity as i128)
                .is_none()
        {
            return Err(FactoryError::InvalidContributionAmount);
        }

        let group_id = match storage.get::<DataKey, u32>(&DataKey::GroupCount) {
            Some(count) => match count.checked_add(1) {
                Some(value) => value,
                None => return Err(FactoryError::ArithmeticOverflow),
            },
            None => 1,
        };

        // Deploy the group, passing its full configuration to its constructor so it
        // is initialized atomically and never exists in an uninitialized state.
        let salt = group_salt(&env, group_id);
        let factory_address = env.current_contract_address();
        let group = env.deployer().with_current_contract(salt).deploy_v2(
            config.group_wasm_hash.clone(),
            (
                factory_address,
                creator.clone(),
                token.clone(),
                config.treasury.clone(),
                contribution_amount,
                member_capacity,
                frequency_seconds,
                config.fee_bps,
            ),
        );

        // Register afterwards. If deployment reverts, nothing is recorded.
        storage.set(&DataKey::GroupCount, &group_id);
        env.storage()
            .persistent()
            .set(&DataKey::Group(group_id), &group);
        extend_persistent_ttl(&env, &DataKey::Group(group_id));

        GroupCreated {
            creator,
            group: group.clone(),
            group_id,
            token,
            contribution_amount,
            member_capacity,
            fee_bps: config.fee_bps,
            treasury: config.treasury,
            frequency_seconds,
        }
        .publish(&env);

        Ok(group)
    }

    // -----------------------------------------------------------------------
    // Admin configuration. Limited strictly to protocol configuration for
    // future groups; nothing here can reach an existing group or its funds.
    // -----------------------------------------------------------------------

    /// Update the fee applied to groups created from now on.
    ///
    /// Capped at `MAX_FEE_BPS`, so the protocol can never charge more than 0.50%.
    /// Existing groups are unaffected: they froze their fee at construction.
    pub fn set_fee(env: Env, fee_bps: u32) -> Result<(), FactoryError> {
        require_admin(&env)?;
        extend_instance_ttl(&env);

        if fee_bps == 0 || fee_bps > MAX_FEE_BPS {
            return Err(FactoryError::InvalidFeeBps);
        }

        let storage = env.storage().instance();
        let mut config = load_config(&env)?;
        config.fee_bps = fee_bps;
        storage.set(&DataKey::Config, &config);

        FeeUpdated { fee_bps }.publish(&env);
        Ok(())
    }

    /// Update the treasury for groups created from now on.
    ///
    /// Existing groups are unaffected: they froze their treasury at construction, so
    /// a treasury change can never redirect fees already owed to a group.
    pub fn set_treasury(env: Env, treasury: Address) -> Result<(), FactoryError> {
        require_admin(&env)?;
        extend_instance_ttl(&env);

        let storage = env.storage().instance();
        let mut config = load_config(&env)?;
        if treasury == config.admin || treasury == env.current_contract_address() {
            return Err(FactoryError::InvalidTreasury);
        }
        config.treasury = treasury.clone();
        storage.set(&DataKey::Config, &config);

        TreasuryUpdated { treasury }.publish(&env);
        Ok(())
    }

    /// Disable creation of new groups.
    ///
    /// Emergency control only. It cannot affect existing groups: their members,
    /// rounds, payouts and funds are entirely outside the Factory's reach. It cannot
    /// block a contribution or a payout either.
    pub fn pause(env: Env) -> Result<(), FactoryError> {
        require_admin(&env)?;
        extend_instance_ttl(&env);

        let storage = env.storage().instance();
        let mut config = load_config(&env)?;
        config.paused = true;
        storage.set(&DataKey::Config, &config);

        PauseUpdated { paused: true }.publish(&env);
        Ok(())
    }

    /// Re-enable creation of new groups.
    pub fn unpause(env: Env) -> Result<(), FactoryError> {
        require_admin(&env)?;
        extend_instance_ttl(&env);

        let storage = env.storage().instance();
        let mut config = load_config(&env)?;
        config.paused = false;
        storage.set(&DataKey::Config, &config);

        PauseUpdated { paused: false }.publish(&env);
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Read-only views.
    // -----------------------------------------------------------------------

    /// The Factory's current configuration.
    ///
    /// Extends the instance TTL like the mutating paths do, so a Factory that is
    /// only ever read keeps its configuration. Returns `NotInitialized` rather
    /// than unwrapping, so a missing entry is a typed error and not a panic that
    /// would take the whole contract's interface down with it.
    pub fn get_config(env: Env) -> Result<FactoryConfig, FactoryError> {
        extend_instance_ttl(&env);
        load_config(&env)
    }

    /// The address of a group by id.
    pub fn get_group(env: Env, group_id: u32) -> Result<Address, FactoryError> {
        let key = DataKey::Group(group_id);
        match env.storage().persistent().get(&key) {
            Some(address) => {
                // Reading a group keeps it alive as well; otherwise a group that
                // is looked up but not otherwise touched could still archive.
                extend_persistent_ttl(&env, &key);
                Ok(address)
            }
            None => Err(FactoryError::GroupNotFound),
        }
    }

    /// The number of groups created so far.
    pub fn get_group_count(env: Env) -> u32 {
        extend_instance_ttl(&env);
        env.storage()
            .instance()
            .get(&DataKey::GroupCount)
            .unwrap_or(0)
    }

    /// Returns the ABI/interface version of this contract.
    ///
    /// Metadata only — carries no financial meaning.
    pub fn version(_env: Env) -> u32 {
        CONTRACT_VERSION
    }
}

/// The salt used for a group, derived from its id.
///
/// Deterministic and unique per group id, so a group's address can be reproduced
/// off-chain from `(factory address, group id)` without querying the Factory. Group
/// ids are assigned monotonically and never reused, so a salt is never reused.
pub fn group_salt(env: &Env, group_id: u32) -> BytesN<32> {
    let mut salt = [0u8; 32];
    salt[28..32].copy_from_slice(&group_id.to_be_bytes());
    BytesN::from_array(env, &salt)
}

/// Requires the protocol admin's authorization.
fn require_admin(env: &Env) -> Result<(), FactoryError> {
    let config = load_config(env)?;
    config.admin.require_auth();
    Ok(())
}

/// Reads the Factory configuration, or reports `NotInitialized`.
///
/// The constructor always writes `Config`, so an absent entry means the instance
/// has been archived. Returning a typed error instead of panicking is what lets
/// `pause` still be reached: a panic here would leave the admin unable to restore
/// the contract through its own interface.
fn load_config(env: &Env) -> Result<FactoryConfig, FactoryError> {
    env.storage()
        .instance()
        .get(&DataKey::Config)
        .ok_or(FactoryError::NotInitialized)
}

/// Extends the instance entry's TTL so the Factory never archives.
fn extend_instance_ttl(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(INSTANCE_TTL_THRESHOLD, INSTANCE_TTL_EXTEND_TO);
}

/// Extends a persistent entry's TTL.
fn extend_persistent_ttl(env: &Env, key: &DataKey) {
    env.storage()
        .persistent()
        .extend_ttl(key, PERSISTENT_TTL_THRESHOLD, PERSISTENT_TTL_EXTEND_TO);
}

/// Instance TTL policy, in ledgers (~5s per ledger).
const INSTANCE_TTL_THRESHOLD: u32 = 100_000;
const INSTANCE_TTL_EXTEND_TO: u32 = 518_400;

/// Persistent TTL policy, in ledgers.
const PERSISTENT_TTL_THRESHOLD: u32 = 100_000;
const PERSISTENT_TTL_EXTEND_TO: u32 = 518_400;

mod test;
