// Copyright 2020-2021 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

//! Split integers to access the high and low parts of an integer.

/// Represents a split integer into high and low parts.
pub(crate) trait SplitInteger: Copy {
    /// The type of the high part of the integer.
    type High;
    /// The type of the low part of the integer.
    type Low;

    /// Returns the high part of the integer.
    fn hi(self) -> Self::High;
    /// Returns the low part of the integer.
    fn lo(self) -> Self::Low;
}

impl SplitInteger for i64 {
    type High = i32;
    type Low = u32;

    #[allow(clippy::cast_possible_truncation)] // Truncation is used to remove the low part of the integer.
    fn hi(self) -> Self::High {
        (self >> 32) as Self::High
    }

    #[allow(clippy::cast_possible_truncation)] // Truncation is used to remove the high part of the integer.
    #[allow(clippy::cast_sign_loss)] // Sign loss is expected as `Self::Low` represents the lower part of the integer.
    fn lo(self) -> Self::Low {
        self as Self::Low
    }
}

impl SplitInteger for u64 {
    type High = u32;
    type Low = u32;

    #[allow(clippy::cast_possible_truncation)] // Truncation is used to remove the low part of the integer.
    fn hi(self) -> Self::High {
        (self >> 32) as Self::High
    }

    #[allow(clippy::cast_possible_truncation)] // Truncation is used to remove the high part of the integer.
    fn lo(self) -> Self::Low {
        self as Self::Low
    }
}

#[cfg(test)]
#[path = "../../../../../../../test/rust/iota-crypto/unit/hashes/ternary/kerl/bigint/split_integer.rs"]
mod tests;
