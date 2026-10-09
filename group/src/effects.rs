use std::collections::HashMap;
use sui_sdk::types::base_types::SuiAddress;
use sui_sdk::types::object::ObjectID;

/// Tracks the state of effects across rounds.
#[derive(Clone)]
pub struct Effects {
    round: u64,
    payout_in_progress: bool,
    balance_changes: HashMap<SuiAddress, u64>,
}

impl Effects {
    pub fn new() -> Self {
        Self {
            round: 0,
            payout_in_progress: false,
            balance_changes: HashMap::new(),
        }
    }

    pub fn round(&self) -> u64 {
        self.round
    }

    pub fn is_payout_in_progress(&self) -> bool {
        self.payout_in_progress
    }

    pub fn mark_payout_in_progress(&mut self, value: bool) {
        self.payout_in_progress = value;
    }

    pub fn advance_round(&mut self) {
        self.round += 1;
    }

    pub fn record_transfer(&mut self, addr: SuiAddress, amount: u64) {
        let entry = self.balance_changes.entry(addr).or_insert(0);
        *entry += amount;
    }

    pub fn record_contribution(&mut self, addr: SuiAddress, amount: u64) {
        let entry = self.balance_changes.entry(addr).or_insert(0);
        *entry -= amount; // contribution reduces external balance
    }

    pub fn get_balance_change(&self, addr: SuiAddress) -> u64 {
        *self.balance_changes.get(&addr).unwrap_or(&0)
    }
}
