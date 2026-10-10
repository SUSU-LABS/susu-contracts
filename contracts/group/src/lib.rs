//! SUSU Group Contract
//!
//! This module manages groups, members, and contribution rounds.

use concordium_contracts_common::*;
use std::collections::BTreeMap;

// --- Types ---

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct RoundInfo {
    /// The address receiving the payout.
    /// Documented as `None` for a round that does not exist yet.
    pub recipient: Option<Address>,
    /// The current phase of the round.
    pub phase: Phase,
    /// Total contributions made to this round.
    pub contributions: u64,
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq)]
pub enum Phase {
    WaitingForContributions,
    Active,
    Completed,
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct GroupInfo {
    /// Map of member index to address.
    pub members: BTreeMap<u32, Address>,
    /// Maximum number of members allowed.
    pub member_capacity: u32,
    /// Index of the current active or most recently completed round.
    pub current_round: u32,
}

// --- Storage Keys ---

enum DataKey {
    MemberAt(u32),
    GroupState,
}

impl DataKey {
    fn get<T: Serialize + Deserialize>(&self) -> Option<T> {
        match self {
            DataKey::MemberAt(idx) => {
                let key = (*idx).to_bytes();
                // In a real environment, this calls into the runtime storage
                // For this snippet, we assume the runtime handles the lookup
                // and returns the deserialized value if present.
                None // Placeholder
            }
            DataKey::GroupState => {
                let key = b"group_state";
                None // Placeholder
            }
        }
    }

    fn set<T: Serialize>(&self, value: &T) {
        match self {
            DataKey::MemberAt(idx) => {
                let key = (*idx).to_bytes();
                // Runtime storage write
            }
            DataKey::GroupState => {
                let key = b"group_state";
                // Runtime storage write
            }
        }
    }
}

// --- Global State ---

static mut GROUP_STATE: Option<GroupInfo> = None;

fn get_group_state() -> &'static mut GroupInfo {
    unsafe {
        GROUP_STATE.get_or_insert_with(|| GroupInfo {
            members: BTreeMap::new(),
            member_capacity: 0,
            current_round: 0,
        })
    }
}

// --- API Responses ---

#[derive(Serialize, Deserialize, Debug)]
pub struct GetRoundResponse {
    pub info: Option<RoundInfo>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct InitResponse {
    pub success: bool,
}

// --- Functions ---

/// Initializes the group.
#[concordium(init)]
fn init(_ctx: &InitContext, _sender: Address, _args: InitArgs) -> InitResult<InitResponse> {
    let group = get_group_state();
    // Initialization logic would go here
    Ok(InitResponse { success: true }.into_return_value())
}

struct InitArgs;

impl<'a> Deserialize<'a> for InitArgs {
    fn deserialize(_reader: &mut Reader<'a>) -> Result<Self, DeserializationError> {
        Ok(InitArgs)
    }
}

/// Adds a member to the group.
#[concordium(update)]
fn add_member(_ctx: &UpdateContext, _sender: Address, _args: AddMemberArgs) -> UpdateResult<AddMemberResponse> {
    // Logic omitted for brevity
    Ok(AddMemberResponse { success: true }.into_return_value())
}

struct AddMemberArgs {
    address: Address,
}

impl<'a> Deserialize<'a> for AddMemberArgs {
    fn deserialize(reader: &mut Reader<'a>) -> Result<Self, DeserializationError> {
        let address = reader.get()?;
        Ok(AddMemberArgs { address })
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct AddMemberResponse {
    pub success: bool,
}

/// Returns information about a specific round.
///
/// # Arguments
/// * `round` - The index of the round to query.
///
/// # Returns
/// * `GetRoundResponse` containing the `RoundInfo` if the round exists and is valid.
#[concordium(get_round)]
fn get_round(ctx: &ContractContext, _sender: Address) -> GetRoundResponse {
    let round: u32 = ctx.parameter_cursor().get();
    
    let group = get_group_state();
    
    // Ensure the round index is within the configured capacity
    if round > group.member_capacity {
        return GetRoundResponse {
            info: None,
        };
    }

    // Fetch the stored round info
    let round_info = match DataKey::MemberAt(round).get::<RoundInfo>() {
        Some(info) => info,
        None => {
            // If no data is stored for this round, it doesn't exist yet
            return GetRoundResponse {
                info: None,
            };
        }
    };

    // FIX: Apply the logic from the issue.
    // If the requested round is greater than the current round, it means the round
    // has not started yet (it is in the future). In this case, the recipient should be None,
    // even if data was pre-scheduled.
    let recipient = if round > group.current_round {
        None
    } else {
        round_info.recipient
    };

    GetRoundResponse {
        info: Some(RoundInfo {
            recipient,
            phase: round_info.phase,
            contributions: round_info.contributions,
        }),
    }
}

/// Contributes to the current round.
#[concordium(update)]
fn contribute(_ctx: &UpdateContext, _sender: Address, _args: ContributeArgs) -> UpdateResult<ContributeResponse> {
    // Logic omitted
    Ok(ContributeResponse { success: true }.into_return_value())
}

struct ContributeArgs {
    amount: u64,
}

impl<'a> Deserialize<'a> for ContributeArgs {
    fn deserialize(reader: &mut Reader<'a>) -> Result<Self, DeserializationError> {
        let amount = reader.get()?;
        Ok(ContributeArgs { amount })
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ContributeResponse {
    pub success: bool,
}

/// Ends the current round and starts the next one.
#[concordium(update)]
fn end_round(_ctx: &UpdateContext, _sender: Address) -> UpdateResult<EndRoundResponse> {
    // Logic omitted
    Ok(EndRoundResponse { success: true }.into_return_value())
}

#[derive(Serialize, Deserialize, Debug)]
pub struct EndRoundResponse {
    pub success: bool,
}
