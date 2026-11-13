// Copyright 2020 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

/// Fill the buffer `bs` with cryptographically strong entropy from the operating systems
/// recommended entropy source
pub fn fill(bs: &mut [u8]) -> crate::Result<()> {
    getrandom::getrandom(bs).map_err(|e| crate::Error::SystemError {
        call: "getrandom::getrandom",
        raw_os_error: e.raw_os_error(),
    })
}

/// Generate a cryptographically strong random value of a `Sized` type `T`
///
/// # Safety
/// This function fills the memory of the returned type with random values, there are no guarantees
/// that the type's invariants hold and so may lead to undefined behavior if used inappropriately.
pub unsafe fn gen<T: Sized + Copy>() -> crate::Result<T> {
    let mut t = core::mem::MaybeUninit::uninit();
    fill(core::slice::from_raw_parts_mut(
        t.as_mut_ptr() as *mut u8,
        core::mem::size_of::<T>(),
    ))?;
    Ok(t.assume_init())
}

#[cfg(test)]
#[path = "../../../../test/rust/iota-crypto/unit/utils/rand.rs"]
mod tests;
