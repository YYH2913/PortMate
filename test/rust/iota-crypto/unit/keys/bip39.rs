// Copyright 2020 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0
use super::*;

#[test]
fn test_encode_decode() {
    fn inc(e: u8, i: usize) -> u8 {
        ((e as usize + 0x9b17f203) * (i + 0x4792a0e2) + 7) as u8
    }

    let mut entropy = [0_u8; 32];
    for _ in 0..5 {
        entropy
            .iter_mut()
            .enumerate()
            .for_each(|(i, e)| *e = e.wrapping_add(inc(*e, i)));

        for i in 4..9 {
            let n = 4 * i;

            let mnemonic = wordlist::encode(&entropy[..n], &wordlist::ENGLISH).unwrap();
            let decoded_entropy = wordlist::decode(&mnemonic, &wordlist::ENGLISH).unwrap();
            assert_eq!(&entropy[..n], &decoded_entropy[..]);
        }
    }
}
