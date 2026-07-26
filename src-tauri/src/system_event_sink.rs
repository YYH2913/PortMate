use super::*;

pub(super) struct SystemEventSinkGuard {
    store: Weak<Mutex<SessionStore>>,
    shutdown: std::sync::mpsc::Sender<()>,
    worker: Mutex<Option<std::thread::JoinHandle<()>>>,
}

impl SystemEventSinkGuard {
    fn shutdown(&self) {
        if let Some(store) = self.store.upgrade() {
            if let Ok(mut store) = store.lock() {
                store.clear_system_event_notifier();
            }
        }
        let _ = self.shutdown.send(());
        if let Ok(mut worker) = self.worker.lock() {
            if let Some(worker) = worker.take() {
                let _ = worker.join();
            }
        }
    }
}

impl Drop for SystemEventSinkGuard {
    fn drop(&mut self) {
        self.shutdown();
    }
}

pub(super) fn install_system_event_sink(state: &AppState) -> Result<(), String> {
    if state
        .system_event_sink
        .lock()
        .map_err(|error| error.to_string())?
        .is_some()
    {
        return Err("system event sink is already installed".to_string());
    }

    let (notifier, wakeups) = std::sync::mpsc::sync_channel(1);
    let (shutdown, shutdown_rx) = std::sync::mpsc::channel();
    let store = Arc::downgrade(&state.store);
    let store_path = state.store_path.clone();
    let app_handle = state.app_handle.clone();
    let worker_store = store.clone();
    let worker = std::thread::Builder::new()
        .name("portmate-system-event-sink".to_string())
        .spawn(move || {
            run_system_event_sink(
                &worker_store,
                &store_path,
                app_handle.as_ref(),
                wakeups,
                shutdown_rx,
            );
        })
        .map_err(|error| format!("failed to start system event sink: {error}"))?;
    let guard = SystemEventSinkGuard {
        store,
        shutdown,
        worker: Mutex::new(Some(worker)),
    };
    let install_result = state
        .store
        .lock()
        .map_err(|error| error.to_string())?
        .set_system_event_notifier(notifier);
    if let Err(error) = install_result {
        guard.shutdown();
        return Err(error);
    }
    *state
        .system_event_sink
        .lock()
        .map_err(|error| error.to_string())? = Some(guard);
    Ok(())
}

pub(super) fn shutdown_system_event_sink(state: &AppState) {
    let guard = state
        .system_event_sink
        .lock()
        .ok()
        .and_then(|mut guard| guard.take());
    if let Some(guard) = guard {
        guard.shutdown();
    }
}

fn run_system_event_sink(
    store: &Weak<Mutex<SessionStore>>,
    store_path: &Path,
    app_handle: Option<&AppHandle>,
    wakeups: std::sync::mpsc::Receiver<()>,
    shutdown: std::sync::mpsc::Receiver<()>,
) {
    loop {
        if shutdown.try_recv().is_ok() {
            drain_system_event_outbox(store, store_path, app_handle);
            return;
        }
        match wakeups.recv_timeout(Duration::from_millis(100)) {
            Ok(()) => drain_system_event_outbox(store, store_path, app_handle),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                drain_system_event_outbox(store, store_path, app_handle);
                return;
            }
        }
    }
}

fn drain_system_event_outbox(
    store: &Weak<Mutex<SessionStore>>,
    store_path: &Path,
    app_handle: Option<&AppHandle>,
) {
    loop {
        let events = match store.upgrade() {
            Some(store) => match store.lock() {
                Ok(mut store) => store.drain_system_event_outbox(),
                Err(error) => {
                    eprintln!("PortMate: system event outbox lock failed: {error}");
                    return;
                }
            },
            None => return,
        };
        if events.is_empty() {
            return;
        }
        for (event, profile) in events {
            publish_system_event(store, store_path, app_handle, event, profile);
        }
    }
}
