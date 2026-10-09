use std::collections::HashMap;
use sui_sdk::types::base_types::SuiAddress;
use sui_sdk::types::object::ObjectID;
use sui_sdk::types::transaction::Effect;
use crate::effects::Effects;
use crate::token::{Token, TokenMode};

/// A token that can re-enter `execute_payout` during its own execution.
/// It ensures that re-entrant calls do not cause double payments across rounds.
#[derive(Clone)]
pub struct ReentrantToken {
    id: ObjectID,
    mode: TokenMode,
}

impl ReentrantToken {
    pub fn new(id: ObjectID) -> Self {
        Self {
            id,
            mode: TokenMode::RoundRobin,
        }
    }

    /// Execute payout, protecting against re-entrant double-pay.
    /// If called re-entrantly during an already-in-progress payout,
    /// the re-entrant call is a no-op.
    pub fn execute_payout(
        &mut self,
        effects: &mut Effects,
        rr_id: &ObjectID,
        recipients: &HashMap<SuiAddress, u64>,
    ) -> Result<(), String> {
        // If we are already inside a payout for this round, skip (re-entrant guard)
        if effects.is_payout_in_progress() {
            return Ok(());
        }

        effects.mark_payout_in_progress(true);
        let result = self.do_execute_payout(effects, rr_id, recipients);
        effects.mark_payout_in_progress(false);
        result
    }

    fn do_execute_payout(
        &mut self,
        effects: &mut Effects,
        rr_id: &ObjectID,
        recipients: &HashMap<SuiAddress, u64>,
    ) -> Result<(), String> {
        // Advance round before interactions (effects-before-interactions ordering)
        effects.advance_round();
        let round = effects.round();

        // Distribute to recipients in round-robin fashion
        let total: u64 = recipients.values().sum();
        if total == 0 {
            return Err("No funds to distribute".to_string());
        }

        let mut remaining = total;
        for (addr, amount) in recipients {
            if remaining >= *amount {
                effects.record_transfer(*addr, *amount);
                remaining -= amount;
            }
        }

        // Verify each recipient received exactly their share once
        for (addr, expected) in recipients {
            let actual = effects.get_balance_change(*addr);
            if actual != *expected {
                return Err(format!(
                    "Recipient {} received {} instead of {}",
                    addr, actual, expected
                ));
            }
        }

        Ok(())
    }

    /// Contribute funds to the token pool.
    pub fn contribute(
        &mut self,
        effects: &mut Effects,
        from: &SuiAddress,
        amount: u64,
    ) -> Result<(), String> {
        if effects.is_payout_in_progress() {
            return Err("Cannot contribute during active payout".to_string());
        }
        effects.record_contribution(*from, amount);
        Ok(())
    }
}
