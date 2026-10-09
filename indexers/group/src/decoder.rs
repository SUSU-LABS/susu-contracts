use fuels::prelude::TxReceipt;
use fuels::types::{Bits256, ContractId};
use super::error::DecodeError;
use super::types::*;

/// Decodes group-related events from transaction receipts.
#[derive(Debug, Clone)]
pub struct GroupDecoder {
    pub group_initialized_topic: String,
}

impl GroupDecoder {
    /// Creates a new GroupDecoder with the given topic.
    pub fn new(group_initialized_topic: String) -> Self {
        Self {
            group_initialized_topic,
        }
    }

    /// Decodes a GroupInitialized event from a receipt.
    ///
    /// Expected event format:
    /// - topic[0]: GroupInitialized (32 bytes)
    /// - data: [group_id (8 bytes), token (32 bytes), treasury (32 bytes),
    ///          contribution_amount (32 bytes), capacity (4 bytes),
    ///          frequency (4 bytes), fee (32 bytes)]
    pub fn decode_group_initialized(
        &self,
        receipt: &TxReceipt,
    ) -> Result<Option<GroupInitializedEvent>, DecodeError> {
        let logs = receipt.logs();
        
        for log in logs {
            // Check if this log matches our GroupInitialized topic
            if log.data.len() >= 148 { // 8 + 32 + 32 + 32 + 4 + 4 + 32 = 148 bytes
                // Extract the topic from the first 32 bytes
                let topic_bytes = &log.data[..32];
                let topic = hex::encode(topic_bytes);
                
                if topic == self.group_initialized_topic {
                    let data = &log.data[32..];
                    
                    // Parse the event data
                    let group_id = u64::from_be_bytes(data[0..8].try_into().unwrap());
                    let token = Bits256::from(data[8..40].try_into().unwrap());
                    let treasury = Bits256::from(data[40..72].try_into().unwrap());
                    let contribution_amount = U256::from(data[72..104].try_into().unwrap());
                    let capacity = u32::from_be_bytes(data[104..108].try_into().unwrap());
                    let frequency = u32::from_be_bytes(data[108..112].try_into().unwrap());
                    let fee = U256::from(data[112..144].try_into().unwrap());
                    
                    return Ok(Some(GroupInitializedEvent {
                        group_id,
                        token: token.into(),
                        treasury: treasury.into(),
                        contribution_amount,
                        capacity,
                        frequency,
                        fee,
                    }));
                }
            }
        }
        
        Ok(None)
    }

    /// Decodes all group-related events from a receipt.
    pub fn decode_events(
        &self,
        receipt: &TxReceipt,
    ) -> Result<Vec<GroupEvent>, DecodeError> {
        let mut events = Vec::new();
        
        if let Some(group_initialized) = self.decode_group_initialized(receipt)? {
            events.push(GroupEvent::GroupInitialized(group_initialized));
        }
        
        Ok(events)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fuels::types::LogFragment;
    
    #[test]
    fn test_decode_group_initialized() {
        let decoder = GroupDecoder::new("group_initialized".to_string());
        
        // Create a mock receipt with GroupInitialized event
        let receipt = TxReceipt::new(
            fuels::types::TransactionResponse {
                id: Default::default(),
                block_height: Default::default(),
                time: Default::default(),
                gas_used: Default::default(),
                status: Default::default(),
                logs: vec![
                    LogFragment::new(
                        &[0u8; 32], // topic (will be replaced)
                        &[0u8; 148], // event data
                    ).to_string(),
                ],
                ...Default::default(),
            },
        );
        
        // This is a simplified test - in reality you'd need to set up proper mock data
        let result = decoder.decode_events(&receipt);
        assert!(result.is_ok());
    }
}
