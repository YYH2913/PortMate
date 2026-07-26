use super::*;

#[tauri::command]
pub(crate) async fn create_tunnel(
    state: State<'_, AppState>,
    request: CreateTunnelRequest,
) -> Result<TunnelSpec, String> {
    create_tunnel_inner(state.inner(), request).await
}

#[tauri::command]
pub(crate) fn list_tunnels(
    state: State<'_, AppState>,
    session_id: Option<String>,
) -> Result<Vec<TunnelStatus>, String> {
    list_tunnels_inner(state.inner(), session_id.as_deref())
}

#[tauri::command]
pub(crate) async fn stop_tunnel(
    state: State<'_, AppState>,
    tunnel_id: String,
) -> Result<TunnelStatus, String> {
    stop_tunnel_inner(state.inner(), &tunnel_id).await
}
