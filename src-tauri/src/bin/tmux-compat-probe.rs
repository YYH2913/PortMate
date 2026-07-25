use serde::{Deserialize, Serialize};
use std::io::{self, Read};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TmuxSessionInfo {
    pub name: String,
    pub windows: u32,
    pub attached: u32,
    pub created: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TmuxPaneInfo {
    pub session: String,
    pub window_index: u32,
    pub pane_index: u32,
    pub pane_id: String,
    pub active: bool,
    pub synchronized: bool,
    pub command: String,
    pub title: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TmuxWindowInfo {
    pub session: String,
    pub window_index: u32,
    pub window_id: String,
    pub name: String,
    pub panes: u32,
    pub active: bool,
    pub synchronized: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TmuxMutationAction {
    RenameSession,
    KillSession,
    NewWindow,
    RenameWindow,
    KillWindow,
    KillPane,
    SelectPane,
    BreakPane,
    MovePaneHorizontal,
    MovePaneVertical,
    SplitPaneHorizontal,
    SplitPaneVertical,
    SwapPanePrevious,
    SwapPaneNext,
    ResizePaneLeft,
    ResizePaneRight,
    ResizePaneUp,
    ResizePaneDown,
    SelectLayout,
}

#[derive(Debug, Clone, Copy)]
pub enum TmuxWindowLayout {
    EvenHorizontal,
    EvenVertical,
    MainHorizontal,
    MainVertical,
    Tiled,
}

#[derive(Debug, Clone)]
pub struct TmuxMutationRequest {
    pub session_id: String,
    pub action: TmuxMutationAction,
    pub target: String,
    pub name: Option<String>,
    pub destination: Option<String>,
    pub layout: Option<TmuxWindowLayout>,
    pub amount: Option<u16>,
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[path = "../tmux_protocol.rs"]
mod tmux_protocol;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProbeInput {
    sessions: String,
    windows: String,
    panes: String,
    control: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProbeOutput {
    sessions: Vec<TmuxSessionInfo>,
    windows: Vec<TmuxWindowInfo>,
    panes: Vec<TmuxPaneInfo>,
    control_changed: bool,
    last_control_event: Option<&'static str>,
    protocol_command_count: usize,
    bounded_error_characters: usize,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut input = String::new();
    io::stdin().read_to_string(&mut input)?;
    let input: ProbeInput = serde_json::from_str(&input)?;
    let sessions = input
        .sessions
        .lines()
        .filter_map(tmux_protocol::parse_tmux_session)
        .collect::<Vec<_>>();
    let mut windows = input
        .windows
        .lines()
        .filter_map(tmux_protocol::parse_tmux_window)
        .collect::<Vec<_>>();
    let panes = input
        .panes
        .lines()
        .filter_map(tmux_protocol::parse_tmux_pane)
        .collect::<Vec<_>>();
    for window in &mut windows {
        let matching = panes.iter().filter(|pane| {
            pane.session == window.session && pane.window_index == window.window_index
        });
        let matching = matching.collect::<Vec<_>>();
        window.synchronized = !matching.is_empty() && matching.iter().all(|pane| pane.synchronized);
    }

    let mut parser = tmux_protocol::TmuxControlLineParser::default();
    let mut control_changed = false;
    let mut last_control_event = None;
    for chunk in input.control.as_bytes().chunks(7) {
        let parsed = parser.push(chunk)?;
        control_changed |= parsed.changed;
        if parsed.last_event.is_some() {
            last_control_event = parsed.last_event;
        }
    }
    let protocol_command_count = exercise_protocol_commands()?;
    let bounded_error_characters = tmux_protocol::bounded_tmux_control_error(&"故".repeat(600))
        .chars()
        .count();
    println!(
        "{}",
        serde_json::to_string(&ProbeOutput {
            sessions,
            windows,
            panes,
            control_changed,
            last_control_event,
            protocol_command_count,
            bounded_error_characters,
        })?
    );
    Ok(())
}

fn exercise_protocol_commands() -> Result<usize, String> {
    use TmuxMutationAction::*;
    let actions = [
        RenameSession,
        KillSession,
        NewWindow,
        RenameWindow,
        KillWindow,
        KillPane,
        SelectPane,
        BreakPane,
        MovePaneHorizontal,
        MovePaneVertical,
        SplitPaneHorizontal,
        SplitPaneVertical,
        SwapPanePrevious,
        SwapPaneNext,
        ResizePaneLeft,
        ResizePaneRight,
        ResizePaneUp,
        ResizePaneDown,
        SelectLayout,
    ];
    let mut commands = vec![
        tmux_protocol::tmux_attach_command("lab-renamed")?,
        tmux_protocol::tmux_pane_sync_command("lab-renamed:0", true)?,
    ];
    for action in actions {
        let request = TmuxMutationRequest {
            session_id: "compat".to_string(),
            action,
            target: if matches!(action, RenameSession | KillSession | NewWindow) {
                "lab-renamed".to_string()
            } else {
                "lab-renamed:0.0".to_string()
            },
            name: matches!(action, RenameSession | NewWindow | RenameWindow)
                .then(|| "compat name".to_string()),
            destination: matches!(action, MovePaneHorizontal | MovePaneVertical)
                .then(|| "lab-renamed:1".to_string()),
            layout: (action == SelectLayout).then_some(TmuxWindowLayout::Tiled),
            amount: matches!(
                action,
                ResizePaneLeft | ResizePaneRight | ResizePaneUp | ResizePaneDown
            )
            .then_some(1),
        };
        commands.push(tmux_protocol::tmux_mutation_command(&request)?);
        let _ = tmux_protocol::tmux_mutation_event_scope(&request)?;
        let _ = tmux_protocol::tmux_mutation_label(action);
    }
    Ok(commands.len())
}
