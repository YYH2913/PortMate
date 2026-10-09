// Included from lib.rs so the existing crate-root module paths stay stable.

mod archive_support;
mod bound_directory;
mod bundle_export;
mod external_drop_execution;
mod external_drop_planning;
mod file_batch;
mod file_batch_planning;
mod file_commands;
mod file_create_delete;
mod file_delete;
mod file_metadata;
mod file_operation_paths;
mod file_operations;
mod file_transfer;
mod log_bytes_ref;
mod log_commands;
mod log_query;
mod log_retention;
mod log_storage;
mod migration_diagnostics;
mod migration_journal_store;
mod migration_planning;
mod migration_recovery;
mod migration_runtime;
mod migration_types;
mod remote_safe_tree;
mod safe_sqlite;
mod sqlite_mirror;
mod sqlite_schema;
mod sqlite_store;
mod state_snapshot;
mod store_compatibility;
mod store_normalization;
mod store_persistence;
mod store_transactions;
mod system_event_sink;
mod terminal_export_commands;
mod transfer_commands;
mod transfer_progress;
mod transfer_request;
mod transfer_runtime;

use archive_support::*;
use bound_directory::*;
use bundle_export::*;
use external_drop_execution::*;
use external_drop_planning::*;
use file_batch::*;
use file_batch_planning::*;
use file_create_delete::*;
use file_delete::*;
use file_metadata::*;
use file_operation_paths::*;
use file_operations::*;
use file_transfer::*;
use log_bytes_ref::*;
use log_commands::bounded_log_query_limit;
use log_query::*;
use log_retention::*;
use log_storage::*;
use migration_diagnostics::*;
use migration_journal_store::*;
use migration_planning::*;
use migration_recovery::*;
use migration_runtime::*;
use migration_types::*;
use remote_safe_tree::*;
use safe_sqlite::*;
use sqlite_schema::*;
use sqlite_store::*;
use state_snapshot::*;
use store_compatibility::*;
use store_normalization::*;
use store_persistence::*;
use store_transactions::*;
use system_event_sink::*;

use transfer_progress::*;
use transfer_request::*;
use transfer_runtime::*;

#[cfg(test)]
include!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../test/rust/portmate/support/backend_storage.rs"
));
