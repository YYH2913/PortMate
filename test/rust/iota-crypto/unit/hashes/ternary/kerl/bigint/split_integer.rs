// Copyright 2020-2023 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0
use super::SplitInteger;

macro_rules! test_split_integers {
        ( $( [$fname:ident, $src:expr, $dst:expr] ),+ $(,)? ) => {
            $(
                #[test]
                fn $fname() {
                    assert_eq!($src, $dst);
                }
            )+
        }
    }

test_split_integers!(
    // i64
    [split_i64_hi_one_is_zero, 1i64.hi(), 0i32],
    [split_i64_lo_one_is_one, 1i64.lo(), 1u32],
    [split_i64_hi_max_is_max, i64::MAX.hi(), i32::MAX],
    [split_i64_lo_max_is_max, i64::MAX.lo(), u32::MAX],
    [split_i64_hi_min_is_min, i64::MIN.hi(), i32::MIN],
    [split_i64_lo_min_is_zero, i64::MIN.lo(), 0u32],
    [split_i64_hi_neg_one_is_neg_one, (-1i64).hi(), -1i32],
    [split_i64_lo_neg_one_is_max, (-1i64).lo(), u32::MAX],
    // u64
    [split_u64_hi_one_is_zero, 1u64.hi(), 0u32],
    [split_u64_lo_one_is_one, 1u64.lo(), 1u32],
    [split_u64_hi_max_is_max, u64::MAX.hi(), u32::MAX],
    [split_u64_lo_max_is_max, u64::MAX.lo(), u32::MAX],
    [split_u64_hi_min_is_min, u64::MIN.hi(), 0u32],
    [split_u64_lo_min_is_zero, u64::MIN.lo(), 0u32],
);
