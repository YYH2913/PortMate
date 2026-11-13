use super::*;

/// Polling/terminal reads must not wait for checkpoint/disk locks on the Tauri
/// event loop (or block an async executor thread). The result stays owned so
/// serialization also happens after the Store guard is released.
pub(super) async fn query_desktop_store<T: Send + 'static>(
    store: Arc<Mutex<SessionStore>>,
    query: impl FnOnce(&SessionStore) -> T + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let store = store.lock().map_err(|error| error.to_string())?;
        Ok(query(&store))
    })
    .await
    .map_err(|error| format!("后台会话查询失败: {error}"))?
}

#[cfg(test)]
#[path = "../../test/rust/portmate/unit/desktop_store_query.rs"]
mod tests;
