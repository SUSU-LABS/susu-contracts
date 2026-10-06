# Group

A Sui Move module for managing SUSU-style rotating savings groups.

## Overview

This module implements a group contract that allows users to create and join rotating savings groups (ROSCAs). Members contribute to a common pool, and each round one member receives the full pool as a payout.

## Features

- Create groups with configurable parameters
- Join and leave groups
- Contribute to the group pool
- Execute payouts in round-robin order
- Group administration via `GroupCap`

## Usage

### Creating a Group

```sui
use group::group;

// Create a new group
let (group, group_cap) = group::create_group(
    b"My Group",
    b"A test group",
    12, // 12 rounds
    604800, // 1 week period
    100000000, // 1 SUI contribution
    vector<[u128; 1]>, // payout order
    &mut ctx
);
```

### Joining a Group

```sui
use group::group;

// Join an existing group
group::join_group(group_cap, sender, &mut ctx);
```

### Contributing

```sui
use group::group;

// Contribute to the group
group::contribute(group_cap, coins, &mut ctx);
```

### Payout

```sui
use group::group;

// Execute payout for current round
group::execute_payout(group_cap, &mut ctx);
```

## Testing

Run tests with:
```bash
sui move test
