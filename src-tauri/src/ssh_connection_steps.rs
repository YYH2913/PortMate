use super::*;

#[derive(Debug, PartialEq, Eq)]
pub(super) enum BoundedConnectionStepError {
    TimedOut,
    Failed(String),
}

pub(super) async fn bounded_connection_step<T, E, F>(
    operation: F,
    timeout: Duration,
) -> Result<T, BoundedConnectionStepError>
where
    E: std::fmt::Display,
    F: std::future::Future<Output = Result<T, E>>,
{
    match tokio::time::timeout(timeout, operation).await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(error)) => Err(BoundedConnectionStepError::Failed(error.to_string())),
        Err(_) => Err(BoundedConnectionStepError::TimedOut),
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum DirectTcpipOpenError {
    TimedOut {
        timeout_ms: u128,
        cleanup_warning: Option<String>,
    },
    Failed(String),
}

pub(super) async fn open_direct_tcpip_with_timeout<H: client::Handler>(
    handle: &client::Handle<H>,
    target_host: String,
    target_port: u16,
    originator_address: String,
    originator_port: u16,
    timeout: Duration,
    disconnect_description: &str,
) -> Result<Channel<client::Msg>, DirectTcpipOpenError> {
    match bounded_connection_step(
        handle.channel_open_direct_tcpip(
            target_host,
            u32::from(target_port),
            originator_address,
            u32::from(originator_port),
        ),
        timeout,
    )
    .await
    {
        Ok(channel) => Ok(channel),
        Err(BoundedConnectionStepError::Failed(error)) => Err(DirectTcpipOpenError::Failed(error)),
        Err(BoundedConnectionStepError::TimedOut) => {
            // Cancelling russh's confirmation wait can orphan its channel entry.
            let cleanup_warning =
                request_ssh_disconnect_with_timeout(handle, disconnect_description).await;
            Err(DirectTcpipOpenError::TimedOut {
                timeout_ms: timeout.as_millis(),
                cleanup_warning,
            })
        }
    }
}
