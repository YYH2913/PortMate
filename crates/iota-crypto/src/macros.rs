// Copyright 2020 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

#![allow(unused_macros)]

macro_rules! assert_buffer {
    ($has:expr, $needs:expr, $expr:expr, $name:expr) => {{
        if !($expr) {
            return Err($crate::Error::BufferSize {
                name: $name,
                needs: $needs,
                has: $has,
            });
        }
    }};
}

macro_rules! assert_buffer_lte {
    ($length:expr, $maximum:expr, $name:expr) => {{
        assert_buffer!($length, $maximum, $length <= $maximum, $name)
    }};
}

macro_rules! assert_buffer_gte {
    ($length:expr, $minimum:expr, $name:expr) => {{
        assert_buffer!($length, $minimum, $length >= $minimum, $name)
    }};
}

#[cfg(test)]
#[path = "../../../test/rust/iota-crypto/unit/macros.rs"]
mod tests;
