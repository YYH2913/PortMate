#[cfg(test)]
pub(super) fn commit_store_mutation_with<ResultValue, Mutate, Persist, VerifyAfterError>(
    store: &mut SessionStore,
    mutate: Mutate,
    persist: Persist,
    verify_after_error: VerifyAfterError,
) -> Result<ResultValue, String>
where
    Mutate: FnOnce(&mut SessionStore) -> Result<ResultValue, String>,
    Persist: FnOnce(&SessionStore) -> Result<(), String>,
    VerifyAfterError: FnOnce(&SessionStore) -> Result<bool, String>,
{
    commit_store_mutation_with_state(store, mutate, persist, verify_after_error)
        .map_err(|error| error.message)
}
