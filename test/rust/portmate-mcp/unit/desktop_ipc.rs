use super::*;

#[test]
fn stale_loopback_attempts_leave_budget_to_reload_the_endpoint() {
    // A Windows refusal may consume its full timeout. Reserve enough budget
    // for every reload attempt plus the bounded backoff, not just the first.
    let backoff: Duration = (0..IPC_DISCOVERY_ATTEMPTS - 1)
        .map(|attempt| Duration::from_millis(50 << attempt.min(2)))
        .sum();
    assert!(IPC_CONNECT_TIMEOUT * IPC_DISCOVERY_ATTEMPTS as u32 + backoff < IPC_DISCOVERY_TIMEOUT);
    const { assert!(IPC_DISCOVERY_ATTEMPTS > 1) };
}
