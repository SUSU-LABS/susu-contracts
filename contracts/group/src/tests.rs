use super::*;
use frame_support::{assert_ok, assert_err};

pub(crate) use crate::{Group, Pallet as GroupPallet};
pub(crate) use frame_system::{Config as SystemConfig, ExtBuilder as SystemExtBuilder};
pub(crate) use orml_tokens::{Config as TokensConfig, ExtBuilder as TokensExtBuilder};
pub(crate) use substrate_test_runner::TestXt;

frame_support::construct_runtime!(
    pub enum Test where
        Block = Block,
        NodeBlock = Block,
        UncheckedExtrinsic = UncheckedExtrinsic,
    {
        System: frame_system,
        Group: crate,
        Balances: pallet_balances,
        Tokens: orml_tokens,
    }
);

impl SystemConfig for Test {
    type BaseCallFilter = frame_support::traits::Everything;
    type BlockWeights = ();
    type BlockLength = ();
    type DbWeight = ();
    type RuntimeOrigin = RuntimeOrigin;
    type RuntimeCall = RuntimeCall;
    type Index = u64;
    type BlockNumber = u64;
    type Hash = sp_core::H256;
    type Hashing = sp_runtime::traits::BlakeTwo256;
    type AccountId = u64;
    type Lookup = sp_runtime::traits::IdentityLookup<Self::AccountId>;
    type Header = Header;
    type RuntimeEvent = RuntimeEvent;
    type BlockHashCount = ();
    type Version = ();
    type PalletInfo = PalletInfo;
    type AccountData = pallet_balances::AccountData<u64>;
    type OnNewAccount = ();
    type OnKilledAccount = ();
    type SystemWeightInfo = ();
    type SS58Prefix = ();
    type OnSetCode = ();
    type MaxConsumers = frame_support::traits::ConstU32<16>;
}

impl pallet_balances::Config for Test {
    type MaxLocks = ConstU32<50>;
    type MaxReserves = ();
    type ReserveIdentifier = [u8; 8];
    type RuntimeEvent = RuntimeEvent;
    type Balance = u64;
    type DustRemoval = ();
    type ExistentialDeposit = ConstU64<1>;
    type AccountStore = System;
    type WeightInfo = ();
}

impl TokensConfig for Test {
    type RuntimeEvent = RuntimeEvent;
    type Balance = u64;
    type Currency = Balances;
    type CurrencyMeta = ();
    type ExistentialDeposits = FrameSupport;
    type OnChargeToken = ();
    type WeightInfo = ();
    type DepositBalanceOf = ();
    type MaxLocks = ();
    type MaxReserves = ();
    type ReserveIdentifier = ();
    type HoldIdentifiers = ();
    type SudoOrigin = System;
}

impl crate::Config for Test {
    type RuntimeEvent = RuntimeEvent;
    type TreasuryAccount = TreasuryAccount;
    type MaxGroups = ConstU32<100>;
    type MaxMembers = ConstU32<100>;
}

pub struct TreasuryAccount;
impl Get<u64> for TreasuryAccount {
    fn get() -> u64 {
        999
    }
}

pub(crate) type Block = substrate_test_runner::system::TestBlock<System>;
pub(crate) type UncheckedExtrinsic = substrate_test_runner::system::TestUncheckedExtrinsic<System>;

frame_support::parameter_types! {
    pub const ProposeBudget: u64 = 10;
    pub const AnnounceBudget: u64 = 10;
    pub const MaxApprovals: u32 = 100;
    pub const MaxProposals: u32 = 100;
}

pub(crate) mod support {
    use super::*;

    pub(crate) fn new_test_ext() -> sp_io::TestExternalities {
        let mut t = frame_system::GenesisConfig::<Test>::default()
            .build_storage()
            .unwrap();

        orml_tokens::GenesisConfig::<Test> {
            balances: vec![
                (1, AssetId::DOT, 1000),
                (2, AssetId::DOT, 1000),
                (3, AssetId::DOT, 1000),
            ],
        }
        .assimilate_storage(&mut t)
        .unwrap();

        let mut ext = sp_io::TestExternalities::from(t);
        ext.execute_with(|| System::set_block_number(1));
        ext
    }
}

use support::*;

#[test]
fn test_initialize_group_emits_event() {
    new_test_ext().execute_with(|| {
        let token = AssetId::DOT;
        let treasury = 1u64;
        let contribution_amount = 100u64;
        let capacity = 10u32;
        let frequency = 100u32;
        let fee = 100u64;

        // Capture events
        System::record_event(RuntimeEvent::Group(crate::Event::GroupInitialized {
            group_id: 0,
            token,
            treasury,
            contribution_amount,
            capacity,
            frequency,
            fee,
        }));

        let result = GroupPallet::initialize_group(
            token,
            treasury,
            contribution_amount.into(),
            capacity,
            frequency,
            fee.into(),
        );

        assert_ok!(result);
        assert_eq!(result.unwrap(), 0);

        // Verify the event was emitted
        let events = System::events();
        let expected_event = RuntimeEvent::Group(crate::Event::GroupInitialized {
            group_id: 0,
            token,
            treasury,
            contribution_amount: contribution_amount.into(),
            capacity,
            frequency,
            fee: fee.into(),
        });

        assert!(events.iter().any(|record| record.event == expected_event));
    });
}

#[test]
fn test_initialize_group_validates_capacity() {
    new_test_ext().execute_with(|| {
        let result = GroupPallet::initialize_group(
            AssetId::DOT,
            1u64,
            100u64,
            0, // Invalid: capacity must be > 0
            100u32,
            100u64,
        );

        assert_err!(result, Error::<Test>::InvalidCapacity);
    });
}

#[test]
fn test_initialize_group_validates_frequency() {
    new_test_ext().execute_with(|| {
        let result = GroupPallet::initialize_group(
            AssetId::DOT,
            1u64,
            100u64,
            10u32,
            0, // Invalid: frequency must be > 0
            100u64,
        );

        assert_err!(result, Error::<Test>::InvalidFrequency);
    });
}

#[test]
fn test_initialize_group_validates_fee() {
    new_test_ext().execute_with(|| {
        let result = GroupPallet::initialize_group(
            AssetId::DOT,
            1u64,
            100u64,
            10u32,
            100u32,
            10_001, // Invalid: fee must be <= 10_000
        );

        assert_err!(result, Error::<Test>::InvalidFee);
    });
}

#[test]
fn test_initialize_group_validates_contribution() {
    new_test_ext().execute_with(|| {
        let result = GroupPallet::initialize_group(
            AssetId::DOT,
            1u64,
            0, // Invalid: contribution must be > 0
            10u32,
            100u32,
            100u64,
        );

        assert_err!(result, Error::<Test>::InvalidContribution);
    });
}

#[test]
fn test_initialize_group_emits_single_event() {
    new_test_ext().execute_with(|| {
        let token = AssetId::DOT;
        let treasury = 1u64;
        let contribution_amount = 100u64;
        let capacity = 10u32;
        let frequency = 100u32;
        let fee = 100u64;

        let _ = GroupPallet::initialize_group(
            token,
            treasury,
            contribution_amount.into(),
            capacity,
            frequency,
            fee.into(),
        );

        let events = System::events();
        let group_initialized_events: Vec<_> = events
            .iter()
            .filter(|record| matches!(record.event, RuntimeEvent::Group(crate::Event::GroupInitialized { .. })))
            .collect();

        assert_eq!(group_initialized_events.len(), 1);

        let event = &group_initialized_events[0];
        if let RuntimeEvent::Group(crate::Event::GroupInitialized {
            group_id,
            token: evt_token,
            treasury: evt_treasury,
            contribution_amount: evt_contribution,
            capacity: evt_capacity,
            frequency: evt_frequency,
            fee: evt_fee,
        }) = &event.event
        {
            assert_eq!(*group_id, 0);
            assert_eq!(*evt_token, token);
            assert_eq!(*evt_treasury, treasury);
            assert_eq!(*evt_contribution, contribution_amount.into());
            assert_eq!(*evt_capacity, capacity);
            assert_eq!(*evt_frequency, frequency);
            assert_eq!(*evt_fee, fee.into());
        } else {
            panic!("Expected GroupInitialized event");
        }
    });
}
