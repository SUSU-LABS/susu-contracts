// ... existing code ...

#[test]
fn test_group_constructor_invalid_capacity() {
    let owner = Address::from_str("0x1234567890123456789012345678901234567890").unwrap();
    let factory = Address::from_str("0x9876543210987654321098765432109876543210").unwrap();

    // Test with capacity below MIN_MEMBERS
    let result = Group::new(
        "Test Group".to_string(),
        "Description".to_string(),
        MIN_MEMBERS - 1,
        100,
        owner,
        factory,
    );
    assert!(matches!(result, Err(GroupError::InvalidMemberCapacity)));

    // Test with capacity above MAX_MEMBERS
    let result = Group::new(
        "Test Group".to_string(),
        "Description".to_string(),
        MAX_MEMBERS + 1,
        100,
        owner,
        factory,
    );
    assert!(matches!(result, Err(GroupError::InvalidMemberCapacity)));
}

// ... existing code ...
