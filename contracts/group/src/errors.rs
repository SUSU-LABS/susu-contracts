// SPDX-License-Identifier: Apache-2.0
// Copyright 2023 SUSU Labs

use frame_support::dispatch::DispatchError;

#[derive(Debug, PartialEq)]
pub enum Error {
    GroupNotFound,
    MemberNotFound,
    MemberAlreadyExists,
    InvalidPosition,
    PayoutOrderIncomplete,
}

impl From<Error> for DispatchError {
    fn from(err: Error) -> DispatchError {
        match err {
            Error::GroupNotFound => DispatchError::Other("GroupNotFound"),
            Error::MemberNotFound => DispatchError::Other("MemberNotFound"),
            Error::MemberAlreadyExists => DispatchError::Other("MemberAlreadyExists"),
            Error::InvalidPosition => DispatchError::Other("InvalidPosition"),
            Error::PayoutOrderIncomplete => DispatchError::Other("PayoutOrderIncomplete"),
        }
    }
}
