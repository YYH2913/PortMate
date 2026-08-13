// Included from lib.rs so the existing crate-root module paths stay stable.

mod mcp_authorization;
mod mcp_commands;
mod mcp_content_staging;
mod mcp_control;
mod mcp_execution;
mod mcp_http_runtime;
mod mcp_ipc;
mod sysmon_commands;
mod sysmon_linux_network;
mod sysmon_linux_network_fallback;
mod sysmon_local_command;
mod sysmon_metrics;
mod sysmon_network;
#[cfg(any(target_os = "linux", test))]
mod sysmon_network_io;
mod sysmon_remote_parsing;
mod sysmon_runtime;
mod trigger_runtime;

use mcp_authorization::*;
#[cfg(test)]
use mcp_commands::export_mcp_audit_inner;
use mcp_content_staging::*;
use mcp_control::*;
use mcp_execution::*;
use mcp_http_runtime::*;
use mcp_ipc::*;
use sysmon_linux_network::*;
use sysmon_linux_network_fallback::*;
use sysmon_local_command::*;
use sysmon_metrics::*;
use sysmon_network::*;
#[cfg(any(target_os = "linux", test))]
use sysmon_network_io::*;
use sysmon_remote_parsing::*;
use sysmon_runtime::*;
use trigger_runtime::*;
