use super::*;
use std::future::Future;
use std::pin::pin;
use std::task::{Context, Poll, Waker};

#[test]
fn a_checkpoint_lock_does_not_block_the_query_caller_or_async_executor() {
    tauri::async_runtime::block_on(async {
        let store = Arc::new(Mutex::new(SessionStore::default()));
        let held = Arc::clone(&store);
        let (locked_tx, locked_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let holder = std::thread::spawn(move || {
            let _guard = held.lock().unwrap();
            locked_tx.send(()).unwrap();
            // A broken implementation fails after a bounded wait, not a test
            // deadlock. The correct future yields before we release the lock.
            let _ = release_rx.recv_timeout(Duration::from_secs(2));
        });
        locked_rx.recv().unwrap();
        let caller = std::thread::current().id();
        let mut query = pin!(query_desktop_store(Arc::clone(&store), move |store| {
            (std::thread::current().id(), store.summaries())
        }));
        let first_poll = query.as_mut().poll(&mut Context::from_waker(Waker::noop()));
        let _ = release_tx.send(());
        holder.join().unwrap();
        assert!(
            matches!(first_poll, Poll::Pending),
            "Store lock blocked the caller instead of yielding"
        );
        let (worker, summaries) = query.await.unwrap();
        assert_ne!(
            worker, caller,
            "query ran on the caller's event-loop thread"
        );
        assert!(summaries.is_empty());
        assert!(store.try_lock().is_ok(), "result retained the Store guard");
    });
}

#[test]
fn poisoned_store_is_reported_without_panicking_the_desktop_command() {
    let store = Arc::new(Mutex::new(SessionStore::default()));
    let poison = Arc::clone(&store);
    let _ = std::thread::spawn(move || {
        let _guard = poison.lock().unwrap();
        panic!("fixture: poisoned checkpoint lock");
    })
    .join();
    let result =
        tauri::async_runtime::block_on(query_desktop_store(store, SessionStore::summaries));
    assert!(result.is_err());
}
