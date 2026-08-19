// Included from lib.rs so the existing crate-root module paths stay stable.

#[cfg(feature = "desktop")]
mod app_bootstrap;
mod app_data_migration;
mod command_history_commands;
mod command_types;
mod custom_script_commands;
mod one_key_commands;
mod one_key_prompt;
mod one_key_runtime;
mod outbound_events;
mod profile_commands;
mod profile_normalization;
mod session_close;
mod session_commands;
mod session_events;
mod session_open;
mod session_profile_delete;
mod session_terminal;
mod state;
mod terminal_byte_events;
mod webkit_runtime;

use app_data_migration::*;
use custom_script_commands::{custom_script_for_session, run_custom_script_inner};
use one_key_prompt::*;
use one_key_runtime::*;
use outbound_events::*;
use profile_commands::merge_expected_json_value;
#[cfg(test)]
use profile_commands::{
    apply_proxy_password_update_with_io, merge_expected_profile_update,
    validate_expected_proxy_password, validate_profile_transport_change, validate_profile_tunnels,
};
use profile_normalization::*;
use serial_commands::serial_send_break_inner_with_validation;
#[cfg(test)]
use session_close::close_session_inner;
use session_close::{close_session_inner_with_validation, SessionCloseValidations};
#[cfg(test)]
use session_close::session_has_registered_runtime;
use session_commands::{mark_session_connected_with_events, profile_requires_runtime};
use session_events::*;
use session_events::{append_logging_error, append_logging_errors, sync_stored_event};
#[cfg(test)]
use session_open::{
    apply_session_open_profile_credentials, cancel_pending_session_opens,
    open_session_inner, register_session_open_cancellation, session_lifecycle_lane,
    spawn_session_prepare, wait_for_session_prepare,
};
use session_open::{open_session_inner_with_validation, SessionOpenCredentials};
#[cfg(any(feature = "desktop", test))]
use session_open::MAX_CONCURRENT_SESSION_OPENS;
#[cfg(test)]
use session_profile_delete::delete_session_profile_inner;
#[cfg(test)]
use session_terminal::{resize_session_inner, resize_session_profile_in_store};
use session_terminal::{terminal_key_sequence_for_protocol, terminate_command_for_protocol};
use state::*;
use terminal_byte_events::publish_terminal_bytes;
