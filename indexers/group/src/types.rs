use fuels::prelude::*;
use std::collections::HashMap;

/// Represents a GroupInitialized event emitted when a group is created.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupInitializedEvent {
    /// The unique identifier of the group
    pub group_id: u64,
    /// The token used for contributions
    pub token: String,
    /// The treasury account address
    pub treasury: String,
    /// The required contribution amount per period
    pub contribution_amount: U256,
    /// Maximum number of members allowed
    pub capacity: u32,
    /// Contribution frequency in blocks
    pub frequency: u32,
    /// Fee percentage (in basis points)
    pub fee: U256,
}

/// Represents different types of group events
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GroupEvent {
    GroupInitialized(GroupInitializedEvent),
}

/// Configuration for a group
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupConfig {
    pub token: String,
    pub treasury: String,
    pub contribution_amount: U256,
    pub capacity: u32,
    pub frequency: u32,
    pub fee: U256,
}

/// Indexer state for groups
#[derive(Debug, Clone, Default)]
pub struct GroupIndexerState {
    pub groups: HashMap<u64, GroupConfig>,
    pub events_processed: u64,
}

impl GroupIndexerState {
    /// Creates a new empty indexer state
    pub fn new() -> Self {
        Self::default()
    }

    /// Processes a GroupInitialized event
    pub fn process_group_initialized(&mut self, event: &GroupInitializedEvent) {
        self.groups.insert(
            event.group_id,
            GroupConfig {
                token: event.token.clone(),
                treasury: event.treasury.clone(),
                contribution_amount: event.contribution_amount.clone(),
                capacity: event.capacity,
                frequency: event.frequency,
                fee: event.fee.clone(),
            },
        );
        self.events_processed += 1;
    }

    /// Gets the configuration for a group
    pub fn get_group_config(&self, group_id: u64) -> Option<&GroupConfig> {
        self.groups.get(&group_id)
    }

    /// Gets all group IDs
    pub fn get_all_group_ids(&self) -> Vec<u64> {
        self.groups.keys().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_group_indexer_state_process_event() {
        let mut state = GroupIndexerState::new();
        
        let event = GroupInitializedEvent {
            group_id: 0,
            token: "DOT".to_string(),
            treasury: "0x1234...5678".to_string(),
            contribution_amount: U256::from(100),
            capacity: 10,
            frequency: 100,
            fee: U256::from(100),
        };
        
        state.process_group_initialized(&event);
        
        assert_eq!(state.groups.len(), 1);
        assert_eq!(state.events_processed, 1);
        
        let config = state.get_group_config(0).expect("Group config should exist");
        assert_eq!(config.token, "DOT");
        assert_eq!(config.capacity, 10);
        assert_eq!(config.frequency, 100);
    }
}
