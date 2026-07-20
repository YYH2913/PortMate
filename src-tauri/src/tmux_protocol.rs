use chrono::Utc;

use super::{
    shell_quote, TmuxMutationAction, TmuxMutationRequest, TmuxPaneInfo, TmuxSessionInfo,
    TmuxWindowInfo, TmuxWindowLayout,
};

pub(super) const MAX_TMUX_CONTROL_LINE_BYTES: usize = 64 * 1024;

pub(super) fn bounded_tmux_control_error(error: &str) -> String {
    const MAX_ERROR_CHARS: usize = 512;
    let mut value = error.chars().take(MAX_ERROR_CHARS).collect::<String>();
    if error.chars().count() > MAX_ERROR_CHARS {
        value.push_str("...");
    }
    value
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct TmuxControlParseResult {
    pub(super) changed: bool,
    pub(super) last_event: Option<&'static str>,
}

#[derive(Debug, Default)]
pub(super) struct TmuxControlLineParser {
    partial: Vec<u8>,
}

impl TmuxControlLineParser {
    pub(super) fn push(&mut self, data: &[u8]) -> Result<TmuxControlParseResult, String> {
        let mut parsed = TmuxControlParseResult::default();
        let mut start = 0;
        for (index, byte) in data.iter().enumerate() {
            if *byte != b'\n' {
                continue;
            }
            self.append_partial(&data[start..index])?;
            if let Some(kind) = tmux_control_event_kind(&self.partial) {
                parsed.changed = true;
                parsed.last_event = Some(kind);
            }
            self.partial.clear();
            start = index + 1;
        }
        self.append_partial(&data[start..])?;
        Ok(parsed)
    }

    fn append_partial(&mut self, data: &[u8]) -> Result<(), String> {
        let next_len = self
            .partial
            .len()
            .checked_add(data.len())
            .ok_or_else(|| "Tmux control-mode 行长度溢出".to_string())?;
        if next_len > MAX_TMUX_CONTROL_LINE_BYTES {
            return Err(format!(
                "Tmux control-mode 单行超过 {MAX_TMUX_CONTROL_LINE_BYTES} 字节上限"
            ));
        }
        self.partial.extend_from_slice(data);
        Ok(())
    }
}

pub(super) fn tmux_control_event_kind(line: &[u8]) -> Option<&'static str> {
    let line = line.strip_suffix(b"\r").unwrap_or(line);
    let name = line.split(|byte| *byte == b' ').next().unwrap_or_default();
    match name {
        b"%client-active-pane" => Some("client-active-pane"),
        b"%client-session-changed" => Some("client-session-changed"),
        b"%layout-change" => Some("layout-change"),
        b"%pane-exited" => Some("pane-exited"),
        b"%pane-mode-changed" => Some("pane-mode-changed"),
        b"%session-changed" => Some("session-changed"),
        b"%session-renamed" => Some("session-renamed"),
        b"%session-window-changed" => Some("session-window-changed"),
        b"%sessions-changed" => Some("sessions-changed"),
        b"%subscription-changed" => Some("subscription-changed"),
        b"%unlinked-window-add" => Some("unlinked-window-add"),
        b"%unlinked-window-close" => Some("unlinked-window-close"),
        b"%unlinked-window-renamed" => Some("unlinked-window-renamed"),
        b"%window-add" => Some("window-add"),
        b"%window-close" => Some("window-close"),
        b"%window-pane-changed" => Some("window-pane-changed"),
        b"%window-renamed" => Some("window-renamed"),
        _ => None,
    }
}

pub(super) fn normalize_tmux_target(target: &str) -> Result<&str, String> {
    let target = target.trim();
    if target.is_empty() {
        return Err("Tmux target 不能为空".to_string());
    }
    if target.chars().count() > 256 {
        return Err("Tmux target 不能超过 256 个字符".to_string());
    }
    if target.chars().any(char::is_control) {
        return Err("Tmux target 不能包含控制字符".to_string());
    }
    Ok(target)
}

fn normalize_tmux_name(name: &str) -> Result<&str, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Tmux 名称不能为空".to_string());
    }
    if name.chars().count() > 128 {
        return Err("Tmux 名称不能超过 128 个字符".to_string());
    }
    if name.chars().any(char::is_control) {
        return Err("Tmux 名称不能包含控制字符".to_string());
    }
    Ok(name)
}

pub(super) fn tmux_attach_command(target: &str) -> Result<String, String> {
    let target = normalize_tmux_target(target)?;
    let target = shell_quote(target);
    Ok(format!(
        "tmux switch-client -t {target} || tmux attach -t {target} || tmux new-session -A -s {target}\r"
    ))
}

pub(super) fn tmux_pane_sync_command(target: &str, enabled: bool) -> Result<String, String> {
    let target = normalize_tmux_target(target)?;
    Ok(format!(
        "tmux set-option -w -t {} synchronize-panes {}",
        shell_quote(target),
        if enabled { "on" } else { "off" }
    ))
}

pub(super) fn tmux_mutation_command(request: &TmuxMutationRequest) -> Result<String, String> {
    let target = shell_quote(normalize_tmux_target(&request.target)?);
    let name = request
        .name
        .as_deref()
        .map(normalize_tmux_name)
        .transpose()?;
    match request.action {
        TmuxMutationAction::RenameSession => Ok(format!(
            "tmux rename-session -t {target} {}",
            shell_quote(name.ok_or_else(|| "重命名 session 需要新名称".to_string())?)
        )),
        TmuxMutationAction::KillSession => Ok(format!("tmux kill-session -t {target}")),
        TmuxMutationAction::NewWindow => Ok(match name {
            Some(name) => format!("tmux new-window -t {target} -n {}", shell_quote(name)),
            None => format!("tmux new-window -t {target}"),
        }),
        TmuxMutationAction::RenameWindow => Ok(format!(
            "tmux rename-window -t {target} {}",
            shell_quote(name.ok_or_else(|| "重命名 window 需要新名称".to_string())?)
        )),
        TmuxMutationAction::KillWindow => Ok(format!("tmux kill-window -t {target}")),
        TmuxMutationAction::KillPane => Ok(format!("tmux kill-pane -t {target}")),
        TmuxMutationAction::SelectPane => Ok(format!("tmux select-pane -t {target}")),
        TmuxMutationAction::BreakPane => Ok(format!("tmux break-pane -d -s {target}")),
        TmuxMutationAction::MovePaneHorizontal | TmuxMutationAction::MovePaneVertical => {
            let destination = shell_quote(normalize_tmux_target(
                request
                    .destination
                    .as_deref()
                    .ok_or_else(|| "跨 window 移动 pane 需要目标 window".to_string())?,
            )?);
            Ok(format!(
                "tmux move-pane -d {} -s {target} -t {destination}",
                if request.action == TmuxMutationAction::MovePaneHorizontal {
                    "-h"
                } else {
                    "-v"
                }
            ))
        }
        TmuxMutationAction::SplitPaneHorizontal => Ok(format!("tmux split-window -h -t {target}")),
        TmuxMutationAction::SplitPaneVertical => Ok(format!("tmux split-window -v -t {target}")),
        TmuxMutationAction::SwapPanePrevious => Ok(format!("tmux swap-pane -t {target} -U")),
        TmuxMutationAction::SwapPaneNext => Ok(format!("tmux swap-pane -t {target} -D")),
        TmuxMutationAction::ResizePaneLeft => Ok(format!(
            "tmux resize-pane -t {target} -L {}",
            normalize_tmux_resize_amount(request.amount)?
        )),
        TmuxMutationAction::ResizePaneRight => Ok(format!(
            "tmux resize-pane -t {target} -R {}",
            normalize_tmux_resize_amount(request.amount)?
        )),
        TmuxMutationAction::ResizePaneUp => Ok(format!(
            "tmux resize-pane -t {target} -U {}",
            normalize_tmux_resize_amount(request.amount)?
        )),
        TmuxMutationAction::ResizePaneDown => Ok(format!(
            "tmux resize-pane -t {target} -D {}",
            normalize_tmux_resize_amount(request.amount)?
        )),
        TmuxMutationAction::SelectLayout => Ok(format!(
            "tmux select-layout -t {target} {}",
            tmux_window_layout_argument(
                request
                    .layout
                    .ok_or_else(|| "切换 window 布局需要 layout".to_string())?
            )
        )),
    }
}

pub(super) fn tmux_mutation_event_scope(request: &TmuxMutationRequest) -> Result<String, String> {
    let target = normalize_tmux_target(&request.target)?;
    match request.action {
        TmuxMutationAction::MovePaneHorizontal | TmuxMutationAction::MovePaneVertical => {
            Ok(format!(
                "{target} -> {}",
                normalize_tmux_target(
                    request
                        .destination
                        .as_deref()
                        .ok_or_else(|| "跨 window 移动 pane 需要目标 window".to_string())?
                )?
            ))
        }
        _ => Ok(target.to_string()),
    }
}

fn normalize_tmux_resize_amount(amount: Option<u16>) -> Result<u16, String> {
    let amount = amount.unwrap_or(5);
    if !(1..=100).contains(&amount) {
        return Err("Tmux pane 调整步长必须在 1..=100 之间".to_string());
    }
    Ok(amount)
}

pub(super) fn tmux_window_layout_argument(layout: TmuxWindowLayout) -> &'static str {
    match layout {
        TmuxWindowLayout::EvenHorizontal => "even-horizontal",
        TmuxWindowLayout::EvenVertical => "even-vertical",
        TmuxWindowLayout::MainHorizontal => "main-horizontal",
        TmuxWindowLayout::MainVertical => "main-vertical",
        TmuxWindowLayout::Tiled => "tiled",
    }
}

pub(super) fn tmux_mutation_label(action: TmuxMutationAction) -> &'static str {
    match action {
        TmuxMutationAction::RenameSession => "session renamed",
        TmuxMutationAction::KillSession => "session closed",
        TmuxMutationAction::NewWindow => "window created",
        TmuxMutationAction::RenameWindow => "window renamed",
        TmuxMutationAction::KillWindow => "window closed",
        TmuxMutationAction::KillPane => "pane closed",
        TmuxMutationAction::SelectPane => "pane selected",
        TmuxMutationAction::BreakPane => "pane broken into window",
        TmuxMutationAction::MovePaneHorizontal => "pane moved horizontally",
        TmuxMutationAction::MovePaneVertical => "pane moved vertically",
        TmuxMutationAction::SplitPaneHorizontal => "pane split horizontally",
        TmuxMutationAction::SplitPaneVertical => "pane split vertically",
        TmuxMutationAction::SwapPanePrevious => "pane swapped with previous",
        TmuxMutationAction::SwapPaneNext => "pane swapped with next",
        TmuxMutationAction::ResizePaneLeft => "pane resized left",
        TmuxMutationAction::ResizePaneRight => "pane resized right",
        TmuxMutationAction::ResizePaneUp => "pane resized up",
        TmuxMutationAction::ResizePaneDown => "pane resized down",
        TmuxMutationAction::SelectLayout => "window layout selected",
    }
}

pub(super) fn parse_tmux_session(line: &str) -> Option<TmuxSessionInfo> {
    let mut parts = line.split('\t');
    let name = parts.next()?.to_string();
    if name.is_empty() {
        return None;
    }
    let windows = parts
        .next()
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or_default();
    let attached = parts
        .next()
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or_default();
    let created = parts
        .next()
        .and_then(|value| value.parse::<i64>().ok())
        .and_then(|timestamp| chrono::DateTime::<Utc>::from_timestamp(timestamp, 0))
        .map(|value| value.to_rfc3339());
    Some(TmuxSessionInfo {
        name,
        windows,
        attached,
        created,
    })
}

pub(super) fn parse_tmux_window(line: &str) -> Option<TmuxWindowInfo> {
    let mut parts = line.split('\t');
    let session = parts.next()?.to_string();
    if session.is_empty() {
        return None;
    }
    Some(TmuxWindowInfo {
        session,
        window_index: parts
            .next()
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or_default(),
        window_id: parts.next().unwrap_or_default().to_string(),
        name: parts.next().unwrap_or_default().to_string(),
        panes: parts
            .next()
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or_default(),
        active: parts.next().unwrap_or_default() == "1",
        synchronized: false,
    })
}

pub(super) fn parse_tmux_pane(line: &str) -> Option<TmuxPaneInfo> {
    let mut parts = line.split('\t');
    let session = parts.next()?.to_string();
    if session.is_empty() {
        return None;
    }
    Some(TmuxPaneInfo {
        session,
        window_index: parts
            .next()
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or_default(),
        pane_index: parts
            .next()
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or_default(),
        pane_id: parts.next().unwrap_or_default().to_string(),
        active: parts.next().unwrap_or_default() == "1",
        command: parts.next().unwrap_or_default().to_string(),
        title: parts.next().unwrap_or_default().to_string(),
        synchronized: parts.next().unwrap_or_default() == "1",
    })
}
