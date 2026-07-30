use super::*;

pub(super) const TRANSFER_CANCEL_POLL_INTERVAL: Duration = Duration::from_millis(100);
pub(super) const TRANSFER_CANCELLED_MESSAGE: &str = "transfer cancelled";

#[derive(Clone)]
pub(super) struct TransferProgressContext {
    pub(super) state: AppState,
    pub(super) task_id: String,
    pub(super) cancel: Arc<AtomicBool>,
    pub(super) last_emit: Arc<Mutex<Instant>>,
    pub(super) started: Instant,
    pub(super) rate_baseline_bytes: Arc<AtomicU64>,
    pub(super) rate_limit_bytes_per_second: Option<u64>,
}

pub(super) fn transfer_rate_limit_bytes_per_second(
    state: &AppState,
    session_id: &str,
) -> Option<u64> {
    state
        .store
        .lock()
        .ok()
        .and_then(|store| store.profile(session_id))
        .and_then(|profile| profile.transfer.rate_limit_bytes_per_second)
        .filter(|limit| *limit > 0)
}

pub(super) fn transfer_throttle_delay(
    rate_limit_bytes_per_second: Option<u64>,
    bytes_done: u64,
    elapsed: Duration,
) -> Option<Duration> {
    let limit = rate_limit_bytes_per_second.filter(|limit| *limit > 0)?;
    if bytes_done == 0 {
        return None;
    }
    Duration::from_secs_f64(bytes_done as f64 / limit as f64)
        .checked_sub(elapsed)
        .filter(|delay| !delay.is_zero())
}

pub(super) fn transfer_average_bps(task: &TransferTask) -> Option<f64> {
    let started = task.started_at?;
    let finished = task.finished_at?;
    let elapsed_ms = (finished - started).num_milliseconds().max(1) as f64;
    if task.bytes_done == 0 {
        return None;
    }
    Some((task.bytes_done as f64) * 1000.0 / elapsed_ms)
}

pub(super) fn record_applied_transfer_progress_with<Persist, VerifyAfterError>(
    store: &mut SessionStore,
    task_id: &str,
    bytes_done: u64,
    bytes_total: u64,
    persist: Persist,
    verify_after_error: VerifyAfterError,
) -> Result<TransferTask, String>
where
    Persist: FnOnce(&SessionStore) -> Result<(), String>,
    VerifyAfterError: FnOnce(&SessionStore) -> Result<bool, String>,
{
    let task = store
        .transfers
        .iter_mut()
        .find(|task| task.id == task_id)
        .ok_or_else(|| format!("unknown transfer: {task_id}"))?;
    task.bytes_done = bytes_done;
    if bytes_total > 0 {
        task.bytes_total = bytes_total;
    }
    task.message = Some("running".to_string());
    let task = task.clone();
    persist_applied_store_with(store, "transfer progress", persist, verify_after_error)?;
    Ok(task)
}

impl TransferProgressContext {
    pub(super) fn check_cancelled(&self) -> Result<(), String> {
        if self.cancel.load(Ordering::SeqCst) {
            Err(TRANSFER_CANCELLED_MESSAGE.to_string())
        } else {
            Ok(())
        }
    }

    pub(super) fn set_rate_baseline(&self, bytes_done: u64) {
        self.rate_baseline_bytes.store(bytes_done, Ordering::SeqCst);
    }

    pub(super) async fn throttle(&self, bytes_done: u64) -> Result<(), String> {
        let transferred_this_run =
            bytes_done.saturating_sub(self.rate_baseline_bytes.load(Ordering::SeqCst));
        if let Some(delay) = transfer_throttle_delay(
            self.rate_limit_bytes_per_second,
            transferred_this_run,
            self.started.elapsed(),
        ) {
            let started = Instant::now();
            loop {
                self.check_cancelled()?;
                let remaining = delay.saturating_sub(started.elapsed());
                if remaining.is_zero() {
                    break;
                }
                tokio::time::sleep(remaining.min(TRANSFER_CANCEL_POLL_INTERVAL)).await;
            }
        }
        Ok(())
    }

    pub(super) async fn update(&self, bytes_done: u64, bytes_total: u64) -> Result<(), String> {
        self.check_cancelled()?;
        self.throttle(bytes_done).await?;
        let should_emit = {
            let mut last_emit = self.last_emit.lock().map_err(|error| error.to_string())?;
            if last_emit.elapsed() < Duration::from_millis(300) && bytes_done < bytes_total {
                false
            } else {
                *last_emit = Instant::now();
                true
            }
        };
        if !should_emit {
            return Ok(());
        }
        let task = {
            let mut store = self.state.store.lock().map_err(|error| error.to_string())?;
            record_applied_transfer_progress_with(
                &mut store,
                &self.task_id,
                bytes_done,
                bytes_total,
                |next_store| save_store(&self.state.store_path, next_store),
                |next_store| verify_persisted_store_commit(&self.state.store_path, next_store),
            )?
        };
        emit_transfer_task(&self.state, &task);
        Ok(())
    }
}
