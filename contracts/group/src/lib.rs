use super::*;

pub trait GroupTrait: pallet_group::Config {}

#[pallet::config]
pub trait Config: pallet_group::Config {
    /// Because this pallet emits events, it depends on the runtime's definition of an event.
    type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;
    /// Treasury account.
    #[pallet::constant]
    type TreasuryAccount: Get<AccountIdOf<Self>>;
    /// Maximum number of groups that can be created.
    #[pallet::constant]
    type MaxGroups: Get<u32>;
    /// Maximum number of members per group.
    #[pallet::constant]
    type MaxMembers: Get<u32>;
}

#[pallet::error]
pub enum Error<T> {
    /// Maximum number of groups reached.
    MaxGroupsReached,
    /// Maximum number of members reached.
    MaxMembersReached,
    /// Group already exists.
    GroupAlreadyExists,
    /// Group does not exist.
    GroupDoesNotExist,
    /// Member already exists.
    MemberAlreadyExists,
    /// Member does not exist.
    MemberDoesNotExist,
    /// Contribution amount is invalid.
    InvalidContribution,
    /// Capacity is invalid.
    InvalidCapacity,
    /// Frequency is invalid.
    InvalidFrequency,
    /// Fee is invalid.
    InvalidFee,
    /// Token is invalid.
    InvalidToken,
}

#[pallet::event]
#[pallet::generate_deposit(pub(super) fn deposit_event)]
pub enum Event<T: Config> {
    /// A group has been initialized with the given configuration.
    /// [group_id, token, treasury, contribution_amount, capacity, frequency, fee]
    GroupInitialized {
        group_id: u32,
        token: T::CurrencyId,
        treasury: T::AccountId,
        contribution_amount: BalanceOf<T>,
        capacity: u32,
        frequency: u32,
        fee: BalanceOf<T>,
    },
    /// A group has been created.
    /// [group_id]
    GroupCreated {
        group_id: u32,
    },
    /// A member has joined a group.
    /// [group_id, member]
    MemberAdded {
        group_id: u32,
        member: T::AccountId,
    },
    /// A member has left a group.
    /// [group_id, member]
    MemberRemoved {
        group_id: u32,
        member: T::AccountId,
    },
}

#[pallet::pallet]
pub struct Pallet<T>(_);

#[pallet::storage]
#[pallet::getter(fn groups)]
pub type Groups<T: Config> = StorageMap<_, Twox64Concat, u32, GroupInfo<T>>;

#[pallet::storage]
#[pallet::getter(fn group_members)]
pub type GroupMembers<T: Config> = StorageDoubleMap<
    _,
    Twox64Concat, u32,
    Twox64Concat, T::AccountId,
    (),
>;

#[pallet::storage]
#[pallet::getter(fn next_group_id)]
pub type NextGroupId<T> = StorageValue<_, u32, ValueQuery>;

#[derive(codec::MaxEncodedLen)]
#[scale_info(skip_type_params(T))]
pub struct GroupInfo<T: Config> {
    pub token: T::CurrencyId,
    pub treasury: T::AccountId,
    pub contribution_amount: BalanceOf<T>,
    pub capacity: u32,
    pub frequency: u32,
    pub fee: BalanceOf<T>,
    pub created_at: u64,
}

impl<T: Config> GroupInfo<T> {
    pub fn new(
        token: T::CurrencyId,
        treasury: T::AccountId,
        contribution_amount: BalanceOf<T>,
        capacity: u32,
        frequency: u32,
        fee: BalanceOf<T>,
    ) -> Self {
        Self {
            token,
            treasury,
            contribution_amount,
            capacity,
            frequency,
            fee,
            created_at: frame_system::Pallet::<T>::block_number().saturated_into(),
        }
    }
}

impl<T: Config> GroupTrait for T {}

impl<T: Config> Pallet<T> {
    /// Initialize a new group with the given configuration.
    ///
    /// This function is typically called by the Factory contract to create a new group.
    /// It validates the configuration and emits a `GroupInitialized` event.
    ///
    /// # Arguments
    /// * `token` - The currency token used for contributions
    /// * `treasury` - The treasury account that receives contributions
    /// * `contribution_amount` - The required contribution amount per period
    /// * `capacity` - Maximum number of members allowed
    /// * `frequency` - Contribution frequency in blocks
    /// * `fee` - Fee percentage for the group (in basis points)
    ///
    /// # Returns
    /// * `Result<u32, DispatchError>` - The new group ID or an error
    pub fn initialize_group(
        token: T::CurrencyId,
        treasury: T::AccountId,
        contribution_amount: BalanceOf<T>,
        capacity: u32,
        frequency: u32,
        fee: BalanceOf<T>,
    ) -> Result<u32, DispatchError> {
        // Validate configuration
        ensure!(capacity > 0, Error<T>::InvalidCapacity);
        ensure!(frequency > 0, Error<T>::InvalidFrequency);
        ensure!(fee <= 10_000, Error<T>::InvalidFee);
        ensure!(contribution_amount > Zero::zero(), Error<T>::InvalidContribution);

        let group_id = NextGroupId::<T>::get();
        ensure!(
            group_id < T::MaxGroups::get(),
            Error<T>::MaxGroupsReached
        );

        let group_info = GroupInfo::new(
            token,
            treasury,
            contribution_amount,
            capacity,
            frequency,
            fee,
        );

        Groups::<T>::insert(group_id, group_info);
        NextGroupId::<T>::put(group_id + 1);

        // Emit the GroupInitialized event with the full configuration
        Self::deposit_event(Event::GroupInitialized {
            group_id,
            token,
            treasury,
            contribution_amount,
            capacity,
            frequency,
            fee,
        });

        Ok(group_id)
    }

    /// Add a member to a group.
    ///
    /// # Arguments
    /// * `group_id` - The ID of the group
    /// * `member` - The account to add as a member
    ///
    /// # Returns
    /// * `Result<(), DispatchError>`
    pub fn add_member(group_id: u32, member: T::AccountId) -> Result<(), DispatchError> {
        // Check group exists
        ensure!(Groups::<T>::contains_key(group_id), Error<T>::GroupDoesNotExist);

        // Check member doesn't already exist
        ensure!(
            !GroupMembers::<T>::contains_key(group_id, &member),
            Error<T>::MemberAlreadyExists
        );

        // Check capacity
        let member_count = GroupMembers::<T>::iter_key(group_id).count() as u32;
        let group = Groups::<T>::get(group_id).expect("checked above");
        ensure!(member_count < group.capacity, Error<T>::MaxMembersReached);

        GroupMembers::<T>::insert(group_id, &member, ());

        Self::deposit_event(Event::MemberAdded { group_id, member });

        Ok(())
    }

    /// Remove a member from a group.
    ///
    /// # Arguments
    /// * `group_id` - The ID of the group
    /// * `member` - The account to remove
    ///
    /// # Returns
    /// * `Result<(), DispatchError>`
    pub fn remove_member(group_id: u32, member: T::AccountId) -> Result<(), DispatchError> {
        ensure!(Groups::<T>::contains_key(group_id), Error<T>::GroupDoesNotExist);
        ensure!(
            GroupMembers::<T>::contains_key(group_id, &member),
            Error<T>::MemberDoesNotExist
        );

        GroupMembers::<T>::remove(group_id, &member);

        Self::deposit_event(Event::MemberRemoved { group_id, member });

        Ok(())
    }

    /// Get the group info for a given group ID.
    pub fn get_group(group_id: u32) -> Option<GroupInfo<T>> {
        Groups::<T>::get(group_id)
    }

    /// Get the members of a group.
    pub fn get_group_members(group_id: u32) -> Vec<T::AccountId> {
        GroupMembers::<T>::iter_key(group_id).collect()
    }

    /// Get the next group ID.
    pub fn get_next_group_id() -> u32 {
        NextGroupId::<T>::get()
    }
}
