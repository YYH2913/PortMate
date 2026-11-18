use super::*;

#[test]
fn timed_out_blocking_workers_are_reaped() {
    tauri::async_runtime::block_on(async {
        let finished = Arc::new(AtomicBool::new(false));
        let finished_in_worker = Arc::clone(&finished);
        let worker = tokio::task::spawn_blocking(move || {
            std::thread::sleep(Duration::from_millis(40));
            finished_in_worker.store(true, Ordering::SeqCst);
            7_u8
        });

        assert_eq!(
            wait_reapable_blocking_worker(worker, Duration::from_millis(1), "test worker").await,
            Err(BlockingWorkerWaitError::TimedOut)
        );
        tokio::time::timeout(Duration::from_secs(1), async {
            while !finished.load(Ordering::SeqCst) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("timed-out worker was not reaped");
    });
}

#[test]
fn cancelled_wait_futures_keep_ownership_of_blocking_workers() {
    tauri::async_runtime::block_on(async {
        let finished = Arc::new(AtomicBool::new(false));
        let finished_in_worker = Arc::clone(&finished);
        let worker = tokio::task::spawn_blocking(move || {
            std::thread::sleep(Duration::from_millis(40));
            finished_in_worker.store(true, Ordering::SeqCst);
        });
        let waiting =
            wait_reapable_blocking_worker(worker, Duration::from_secs(1), "cancelled worker");
        tokio::pin!(waiting);
        tokio::select! {
            _ = &mut waiting => panic!("worker completed before cancellation"),
            _ = tokio::time::sleep(Duration::from_millis(1)) => {}
        }
        tokio::time::timeout(Duration::from_secs(1), async {
            while !finished.load(Ordering::SeqCst) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("cancelled worker was detached instead of reaped");
    });
}

#[test]
fn terminal_echo_poll_coalesces_writes_and_returns_to_idle() {
    tauri::async_runtime::block_on(async {
        let wake = Arc::new(tokio::sync::Notify::new());
        let mut poll = TerminalEchoPoll {
            wake: Arc::clone(&wake),
            delay: LIBSSH_CHANNEL_POLL_INTERVAL,
        };
        // Writes between the empty poll and registering the waiter are remembered,
        // but a paste/burst must not accumulate a wake for every byte.
        for _ in 0..32 {
            wake.notify_one();
        }
        poll.wait_after_empty_poll().await;
        assert_eq!(poll.delay, LIBSSH_ECHO_POLL_MIN);
        while poll.delay < LIBSSH_CHANNEL_POLL_INTERVAL {
            let previous = poll.delay;
            poll.wait_after_empty_poll().await;
            assert!(poll.delay > previous);
        }
        assert_eq!(poll.delay, LIBSSH_CHANNEL_POLL_INTERVAL);
    });
}
