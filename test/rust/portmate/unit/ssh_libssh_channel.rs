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
