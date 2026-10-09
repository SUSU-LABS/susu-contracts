// ... existing code ...

impl Group {
    pub fn new(
        name: String,
        description: String,
        member_capacity: u32,
        contribution_amount: u64,
        owner: Address,
        factory: Address,
    ) -> Result<Self, GroupError> {
        // Check member_capacity range before other validations
        if member_capacity < MIN_MEMBERS || member_capacity > MAX_MEMBERS {
            return Err(GroupError::InvalidMemberCapacity);
        }

        // Check contribution_amount and overflow
        if contribution_amount <= 0 || checked_mul(member_capacity as i128).is_none() {
            return Err(GroupError::InvalidContributionAmount);
        }

        // ... rest of the constructor ...
    }
}

// ... existing code ...
