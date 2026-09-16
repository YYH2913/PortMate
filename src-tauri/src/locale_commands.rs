/// Use the OS locale rather than the WebView runtime's installation language.
#[tauri::command]
pub(crate) fn system_locale() -> Option<String> {
    sys_locale::get_locale()
}
