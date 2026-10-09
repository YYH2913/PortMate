use super::*;

#[test]
fn ssh_reader_completion_closes_the_terminal_health_flag() {
    let terminal_channel_open = Arc::new(AtomicBool::new(true));
    let (sender, mut receiver) = tokio::sync::oneshot::channel();
    {
        let _guard = SshReaderCompletionGuard {
            terminal_channel_open: Arc::clone(&terminal_channel_open),
            reader_finished: Some(sender),
        };
    }
    assert!(!terminal_channel_open.load(Ordering::SeqCst));
    assert_eq!(receiver.try_recv(), Ok(()));
}
