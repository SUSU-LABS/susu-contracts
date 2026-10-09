pub mod decoder;
pub mod error;
pub mod types;

pub use decoder::GroupDecoder;
pub use types::{GroupEvent, GroupInitializedEvent, GroupIndexerState};

/// Indexer for SUSU Group contracts
#[derive(Debug, Clone)]
pub struct GroupIndexer {
    decoder: GroupDecoder,
    state: GroupIndexerState,
}

impl GroupIndexer {
    /// Creates a new GroupIndexer with the given configuration
    pub fn new(group_initialized_topic: String) -> Self {
        Self {
            decoder: GroupDecoder::new(group_initialized_topic),
            state: GroupIndexerState::new(),
        }
    }

    /// Processes a transaction receipt and updates the indexer state
    pub fn process_receipt(&mut self, receipt: &TxReceipt) -> Result<Vec<GroupEvent>, decoder::error::DecodeError> {
        let events = self.decoder.decode_events(receipt)?;
        
        for event in &events {
            if let GroupEvent::GroupInitialized(event_data) = event {
                self.state.process_group_initialized(event_data);
            }
        }
        
        Ok(events)
    }

    /// Gets the current indexer state
    pub fn state(&self) -> &GroupIndexerState {
        &self.state
    }

    /// Gets a mutable reference to the indexer state
    pub fn state_mut(&mut self) -> &mut GroupIndexerState {
        &mut self.state
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_group_indexer_new() {
        let indexer = GroupIndexer::new("test_topic".to_string());
        assert_eq!(indexer.state().groups.len(), 0);
        assert_eq!(indexer.state().events_processed, 0);
    }
}
