use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TmuxSessionInfo {
    pub name: String,
    pub windows: u32,
    pub attached: u32,
    pub created: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TmuxState {
    pub sessions: Vec<TmuxSessionInfo>,
    pub windows: Vec<TmuxWindowInfo>,
    pub panes: Vec<TmuxPaneInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TmuxControlStatus {
    pub session_id: String,
    pub target: String,
    pub active: bool,
    #[serde(default)]
    pub runtime_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TmuxControlEvent {
    pub session_id: String,
    pub target: String,
    pub kind: String,
    pub active: bool,
    pub runtime_id: String,
    #[serde(default)]
    pub protocol_event: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum TmuxWindowLayout {
    EvenHorizontal,
    EvenVertical,
    MainHorizontal,
    MainVertical,
    Tiled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TmuxMutationRequest {
    pub session_id: String,
    pub action: TmuxMutationAction,
    pub target: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub destination: Option<String>,
    #[serde(default)]
    pub layout: Option<TmuxWindowLayout>,
    #[serde(default)]
    pub amount: Option<u16>,
}
