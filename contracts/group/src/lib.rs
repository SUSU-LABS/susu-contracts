// Copyright (c) SUSU-LABS
// SPDX-License-Identifier: Apache-2.0

module group::group {
    use std::string;
    use std::error;
    use sui::coin;
    use sui::call_frame;
    use sui::tx_context;
    use sui::address;
    use sui::table;
    use sui::vector;
    use sui::tx_context::{Self, TxContext};
    use sui::object;
    use sui::event;
    use sui::bag;
    use sui::table::{Self, Table, Entry};
    use sui::balance;
    use sui::sui::SUI;
    use sui::account_cap;
    use sui::authenticator;

    // === Error Codes ===
    const EAdminOnly: u64;
    const EGroupNotFound: u64;
    const EInvalidPayoutOrder: u64;
    const EAlreadyMember: u64;
    const ENotMember: u64;
    const EGroupPaused: u64;
    const EInvalidRound: u64;
    const EInsufficientContribution: u64;
    const EPayoutAlreadyExecuted: u64;
    const EInvalidGroupState: u64;

    // === Events ===
    /// Emitted when a group is created
    struct GroupCreated has copy, drop {
        group_id: address,
        admin: address,
        num_rounds: u64,
        period_secs: u64,
        contribution_amount: u64,
    }

    /// Emitted when a member joins a group
    struct MemberJoined has copy, drop {
        group_id: address,
        member: address,
        position: u64,
    }

    /// Emitted when a member leaves a group
    struct MemberLeft has copy, drop {
        group_id: address,
        member: address,
    }

    /// Emitted when a payout is executed
    struct PayoutExecuted has copy, drop {
        group_id: address,
        round: u64,
        recipient: address,
        amount: u64,
    }

    /// Emitted when a contribution is recorded
    struct ContributionRecorded has copy, drop {
        group_id: address,
        member: address,
        amount: u64,
    }

    // === Group Object ===
    /// The main group object. Contains all group state.
    struct Group has key {
        id: uid,
        /// The admin of the group
        admin: address,
        /// Whether the group is paused
        paused: bool,
        /// The members of the group, mapped by address -> position
        members: table.Table<address, u64>,
        /// The payout order, mapping round -> recipient address
        payout_order: table.Table<u64, address>,
        /// The current round
        current_round: u64,
        /// Total number of rounds
        num_rounds: u64,
        /// Period between rounds in seconds
        period_secs: u64,
        /// Contribution amount per member per round
        contribution_amount: u64,
        /// Total contributions collected this round
        contributions_collected: balance::Balance<SUI>,
    }

    // === Admin Cap ===
    /// Capability to administer the group
    struct GroupCap has key, store {
        id: uid,
        group_id: address,
    }

    // === Constants ===
    /// Minimum contribution amount
    const MIN_CONTRIBUTION: u64 = 100000000; // 0.1 SUI

    // === Public Functions ===

    /// Creates a new group with the specified parameters.
    /// Only the caller can be the initial admin.
    public fun create_group(
        num_rounds: u64,
        period_secs: u64,
        contribution_amount: u64,
        payout_order: vector<vector<u8>>,
        ctx: &mut TxContext
    ): (object::Object<Group>, object::Object<GroupCap>) {
        assert!(num_rounds > 0, error::permission_denied(EInvalidGroupState));
        assert!(period_secs > 0, error::permission_denied(EInvalidGroupState));
        assert!(contribution_amount >= MIN_CONTRIBUTION, error::permission_denied(EInvalidGroupState));
        assert!(payout_order.len() == num_rounds as usize, error::permission_denied(EInvalidPayoutOrder));

        let group = object::new(ctx);
        let group_id = object::id(&group);

        // Initialize tables
        let members = table::new<address, u64>();
        let payout_order_table = table::new<u64, address>();

        // Populate payout order table
        let i = 0;
        while (i < payout_order.len()) {
            let addr_bytes = payout_order[i];
            assert!(!addr_bytes.is_empty(), error::permission_denied(EInvalidPayoutOrder));
            let addr = @addr_bytes;
            table::add(&mut payout_order_table, i as u64, addr);
            i = i + 1;
        }

        // Transfer the group object to the sender's account
        let sender = tx_context::sender(ctx);
        object::transfer(group, sender);

        // Create and transfer the admin cap
        let group_cap = object::new(ctx);
        move_to(ctx, GroupCap {
            id: object::uid_instantiate<&mut UID>(ctx),
            group_id,
        });
        object::transfer(group_cap, sender);

        // Emit event
        event::emit(GroupCreated {
            group_id,
            admin: sender,
            num_rounds,
            period_secs,
            contribution_amount,
        });

        (group, group_cap)
    }

    /// Adds a member to the group at the specified position.
    /// Only the admin can add members.
    public fun add_member(
        group_cap: &mut GroupCap,
        member: address,
        position: u64,
        ctx: &mut TxContext
    ) {
        assert!(group_cap_admin_of(group_id(group_cap)) == tx_context::sender(ctx), error::permission_denied(EAdminOnly));
        
        let group = object::from<&mut object::Object<Group>>(group_id(group_cap));
        assert!(!table::contains(&group.members, &member), error::already_exists(EAlreadyMember));
        assert!(position < group.num_rounds, error::invalid_arguments(EInvalidRound));

        table::add(&mut group.members, member, position);

        event::emit(MemberJoined {
            group_id: group_id(group_cap),
            member,
            position,
        });
    }

    /// Removes a member from the group.
    /// Only the admin can remove members.
    public fun remove_member(
        group_cap: &mut GroupCap,
        member: address,
        ctx: &mut TxContext
    ) {
        assert!(group_cap_admin_of(group_id(group_cap)) == tx_context::sender(ctx), error::permission_denied(EAdminOnly));
        
        let group = object::from<&mut object::Object<Group>>(group_id(group_cap));
        assert!(table::contains(&group.members, &member), error::not_found(ENotMember));

        table::remove(&mut group.members, &member);

        event::emit(MemberLeft {
            group_id: group_id(group_cap),
            member,
        });
    }

    /// Pauses the group, preventing further contributions and payouts.
    public fun pause_group(
        group_cap: &mut GroupCap,
        ctx: &mut TxContext
    ) {
        assert!(group_cap_admin_of(group_id(group_cap)) == tx_context::sender(ctx), error::permission_denied(EAdminOnly));
        
        let group = object::from<&mut object::Object<Group>>(group_id(group_cap));
        group.paused = true;
    }

    /// Unpauses the group, allowing contributions and payouts again.
    public fun unpause_group(
        group_cap: &mut GroupCap,
        ctx: &mut TxContext
    ) {
        assert!(group_cap_admin_of(group_id(group_cap)) == tx_context::sender(ctx), error::permission_denied(EAdminOnly));
        
        let group = object::from<&mut object::Object<Group>>(group_id(group_cap));
        group.paused = false;
    }

    /// Makes a contribution to the group.
    public fun contribute(
        group_cap: &mut GroupCap,
        contribution: coin::Coin<SUI>,
        ctx: &mut TxContext
    ) {
        let sender = tx_context::sender(ctx);
        let group_id = group_id(group_cap);
        let group = object::from<&mut object::Object<Group>>(group_id);

        // Check if group is paused
        assert!(!group.paused, error::permission_denied(EGroupPaused));

        // Check if sender is a member
        assert!(table::contains(&group.members, &sender), error::not_found(ENotMember));

        // Check contribution amount
        assert!(coin::value(&contribution) == group.contribution_amount, error::invalid_arguments(EInsufficientContribution));

        // Record contribution
        table::add(
            &mut object::from<&mut object::Object<Group>>(group_id).contributions_collected,
            coin::into_balance(contribution)
        );

        event::emit(ContributionRecorded {
            group_id,
            member: sender,
            amount: coin::value(&contribution),
        });
    }

    /// Executes a payout to the current round's recipient.
    public fun execute_payout(
        group_cap: &mut GroupCap,
        ctx: &mut TxContext
    ): coin::Coin<SUI> {
        let group_id = group_id(group_cap);
        let group = object::from<&mut object::Object<Group>>(group_id);

        // Check if group is paused
        assert!(!group.paused, error::permission_denied(EGroupPaused));

        // Get the current round's recipient
        let current_round = group.current_round;
        assert!(table::contains(&group.payout_order, &current_round), error::not_found(EInvalidRound));
        
        let recipient = table::borrow(&group.payout_order, &current_round);
        
        // Extend persistent storage for recipient to prevent archival
        call_frame::persistent::extend(&recipient, 518_400);
        
        // Extend persistent storage for payout order entry
        call_frame::persistent::extend(&current_round, 518_400);

        // Transfer the accumulated contributions to the recipient
        let balance = table::remove(
            &mut object::from<&mut object::Object<Group>>(group_id).contributions_collected,
        );
        
        let coins = coin::from_balance(balance);

        // Advance to next round
        group.current_round = current_round + 1;

        event::emit(PayoutExecuted {
            group_id,
            round: current_round,
            recipient,
            amount: coin::value(&coins),
        });

        coins
    }

    // === View Functions ===

    /// Returns the admin address of the group
    public fun group_admin(group_id: address): address {
        let group = object::from<&object::Object<Group>>(group_id);
        group.admin
    }

    /// Returns whether the group is paused
    public fun group_is_paused(group_id: address): bool {
        let group = object::from<&object::Object<Group>>(group_id);
        group.paused
    }

    /// Returns whether an address is a member of the group
    public fun is_member(group_id: address, member: address): bool {
        let group = object::from<&object::Object<Group>>(group_id);
        table::contains(&group.members, &member)
    }

    /// Returns the position of a member in the payout order
    public fun member_position(group_id: address, member: address): Option<u64> {
        let group = object::from<&object::Object<Group>>(group_id);
        if table::contains(&group.members, &member) {
            option::some(table::borrow(&group.members, &member))
        } else {
            option::none()
        }
    }

    /// Returns the total number of members
    public fun member_count(group_id: address): u64 {
        let group = object::from<&object::Object<Group>>(group_id);
        table::length(&group.members)
    }

    /// Returns the current round
    public fun current_round(group_id: address): u64 {
        let group = object::from<&object::Object<Group>>(group_id);
        group.current_round
    }

    /// Returns the total number of rounds
    public fun num_rounds(group_id: address): u64 {
        let group = object::from<&object::Object<Group>>(group_id);
        group.num_rounds
    }

    /// Returns the period in seconds
    public fun period_secs(group_id: address): u64 {
        let group = object::from<&object::Object<Group>>(group_id);
        group.period_secs
    }

    /// Returns the contribution amount
    public fun contribution_amount(group_id: address): u64 {
        let group = object::from<&object::Object<Group>>(group_id);
        group.contribution_amount
    }

    // === Internal Functions ===

    fun group_id(group_cap: &GroupCap): address {
        group_cap.group_id
    }

    fun group_cap_admin_of(group_id: address): address {
        let group = object::from<&object::Object<Group>>(group_id);
        group.admin
    }

    // === Tests ===
    #[test_only]
    module group_tests {
        use super::*;
        use sui::test_utils;
        use sui::tx_context::{Self, TxContext};

        fun test_create_group(ctx: &mut TxContext) {
            let (group, group_cap) = create_group(
                12,
                604800, // 1 week
                100000000, // 0.1 SUI
                vector![vector![1], vector![2], vector![3]],
                ctx
            );

            assert!(current_round(object::id(&group)) == 0, 0);
            assert!(num_rounds(object::id(&group)) == 12, 0);
            assert!(period_secs(object::id(&group)) == 604800, 0);
            assert!(contribution_amount(object::id(&group)) == 100000000, 0);
        }

        fun test_add_member(ctx: &mut TxContext) {
            let (group, mut group_cap) = create_group(
                3,
                604800,
                100000000,
                vector![vector![1], vector![2], vector![3]],
                ctx
            );

            let member = @1;
            add_member(&mut group_cap, member, 0, ctx);
            
            assert!(is_member(object::id(&group), member), 0);
            assert!(member_position(object::id(&group), member) == option::some(0), 0);
        }

        fun test_cannot_add_duplicate_member(ctx: &mut TxContext) {
            let (group, mut group_cap) = create_group(
                3,
                604800,
                100000000,
                vector![vector![1], vector![2], vector![3]],
                ctx
            );

            let member = @1;
            add_member(&mut group_cap, member, 0, ctx);
            
            // Should fail - already a member
            test_utils::assert_aborted_with(
                &extract(group),
                &mut group_cap,
                EAlreadyMember,
                |g, cap| add_member(cap, member, 0, ctx)
            );
        }
    }
}
