#[cfg(test)]
pub(crate) fn migrate_profile_secrets_with_io<ReadSecret, WriteBatch, DeleteBatch, PersistStore>(
    store: &mut SessionStore,
    request: &ProfileSecretMigrationRequest,
    read_secret: ReadSecret,
    write_batch: WriteBatch,
    delete_batch: DeleteBatch,
    mut persist_store: PersistStore,
) -> Result<ProfileSecretMigrationResponse, String>
where
    ReadSecret: FnMut(&str) -> Result<String, String>,
    WriteBatch: FnMut(SecretStorage, &[PreparedProfileSecretMigration]) -> Result<bool, String>,
    DeleteBatch: FnMut(SecretStorage, &[String]) -> SecretBatchDeleteOutcome,
    PersistStore: FnMut(&SessionStore, &[String], &[String]) -> ProfileSecretStoreCommit,
{
    migrate_profile_secrets_with_journal_io(
        store,
        request,
        read_secret,
        write_batch,
        delete_batch,
        |next_store, affected_profile_ids, target_refs, _| {
            persist_store(next_store, affected_profile_ids, target_refs)
        },
        |_| Ok(()),
    )
}
