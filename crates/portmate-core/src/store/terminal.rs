use super::SessionStore;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub(super) struct TerminalScreen {
    parser: Arc<Mutex<vt100::Parser>>,
    reset_at: chrono::DateTime<chrono::Utc>,
}

impl std::fmt::Debug for TerminalScreen {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TerminalScreen")
    }
}

impl SessionStore {
    pub fn reset_terminal_screen(&mut self, session_id: &str) {
        if let Some(profile) = self.profile(session_id) {
            self.terminal_screens.insert(
                session_id.to_string(),
                TerminalScreen {
                    parser: Arc::new(Mutex::new(vt100::Parser::new(
                        profile.terminal.rows.clamp(1, 512),
                        profile.terminal.cols.clamp(1, 1024),
                        0,
                    ))),
                    reset_at: chrono::Utc::now(),
                },
            );
        }
    }

    pub fn resize_terminal_screen(&mut self, session_id: &str, rows: u16, cols: u16) {
        if let Some(screen) = self.terminal_screens.get(session_id) {
            if let Ok(mut parser) = screen.parser.lock() {
                parser
                    .screen_mut()
                    .set_size(rows.clamp(1, 512), cols.clamp(1, 1024));
            }
        }
    }

    pub(super) fn process_terminal_output(
        &mut self,
        session_id: &str,
        text: &str,
        ts: chrono::DateTime<chrono::Utc>,
    ) {
        if !self.terminal_screens.contains_key(session_id) {
            self.reset_terminal_screen(session_id);
            if let Some(screen) = self.terminal_screens.get_mut(session_id) {
                screen.reset_at = ts;
            }
        }
        if let Some(screen) = self.terminal_screens.get(session_id) {
            // Persisting old queued logs after reconnect must not paint the new PTY.
            if ts < screen.reset_at {
                return;
            }
            if let Ok(mut parser) = screen.parser.lock() {
                parser.process(text.as_bytes());
            }
        }
    }

    pub(super) fn terminal_screen(&self, session_id: &str) -> Option<String> {
        self.terminal_screens.get(session_id).and_then(|screen| {
            screen
                .parser
                .lock()
                .ok()
                .map(|parser| parser.screen().contents())
        })
    }
}
