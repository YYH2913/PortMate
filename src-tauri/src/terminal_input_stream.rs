//! Desktop-only, bounded sequencing for pipelined keyboard IPC. These are not
//! MCP tools and do not open connections or bypass the shared outbound lane.
use super::*;

const MAX_STREAMS: usize = 1024;
const MAX_REORDERED_PACKETS: u64 = 32;
const MAX_PACKET_BYTES: usize = 16 * 1024;
// Match the desktop's eight-packet window even for multi-byte Unicode input.
const MAX_PENDING_BYTES: usize = 8 * MAX_PACKET_BYTES;
type StreamKey = (PathBuf, String, String);
static STREAMS: OnceLock<Mutex<HashMap<StreamKey, InputStream>>> = OnceLock::new();

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct TerminalInputOrder {
    pub stream_id: String,
    pub sequence: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TerminalInputBinding {
    pub stream_id: String,
}

struct InputPacket {
    text: Zeroizing<String>,
    coalesce: bool,
    sensitive: bool,
}

#[derive(Default)]
struct OrderedInput {
    next: u64,
    pending: BTreeMap<u64, InputPacket>,
    pending_bytes: usize,
    failed: bool,
}

impl OrderedInput {
    fn accept(
        &mut self,
        sequence: u64,
        packet: InputPacket,
        mut enqueue: impl FnMut(InputPacket) -> Result<(), String>,
    ) -> Result<(), String> {
        let result = self.accept_inner(sequence, packet, &mut enqueue);
        if result.is_err() {
            self.failed = true;
            self.pending.clear();
            self.pending_bytes = 0;
        }
        result
    }

    fn accept_inner(
        &mut self,
        sequence: u64,
        packet: InputPacket,
        enqueue: &mut impl FnMut(InputPacket) -> Result<(), String>,
    ) -> Result<(), String> {
        if self.failed {
            return Err("终端输入通道已失效，请重新发送输入".into());
        }
        // Tauri can retry via postMessage after losing a fetch response.
        // A retried sequence must never execute the same keystroke twice.
        if sequence < self.next || self.pending.contains_key(&sequence) {
            return Ok(());
        }
        if sequence > 9_007_199_254_740_991
            || sequence.saturating_sub(self.next) >= MAX_REORDERED_PACKETS
            || packet.text.len() > MAX_PACKET_BYTES
            || self.pending_bytes.saturating_add(packet.text.len()) > MAX_PENDING_BYTES
        {
            return Err("终端输入排序缓冲区超限，已停止该通道以避免乱序".into());
        }
        self.pending_bytes += packet.text.len();
        self.pending.insert(sequence, packet);
        while let Some(packet) = self.pending.remove(&self.next) {
            self.pending_bytes -= packet.text.len();
            enqueue(packet)?;
            self.next += 1;
        }
        Ok(())
    }
}

struct InputStream {
    id: String,
    runtime_id: String,
    ordered: OrderedInput,
}

fn stream_key(io: &SessionIo, session_id: &str, owner: &str) -> StreamKey {
    (
        io.store_path.clone(),
        session_id.to_string(),
        owner.to_string(),
    )
}

pub(super) fn begin_stream(
    io: &SessionIo,
    session_id: &str,
    owner: &str,
) -> Result<TerminalInputBinding, String> {
    let runtime_id = current_session_runtime_id(&io.runtimes, session_id)?
        .ok_or_else(|| "会话尚未连接，无法建立终端输入通道".to_string())?;
    let key = stream_key(io, session_id, owner);
    let mut streams = STREAMS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .map_err(|error| error.to_string())?;
    if !streams.contains_key(&key) && streams.len() >= MAX_STREAMS {
        return Err("终端输入通道数量已达上限".into());
    }
    let id = Uuid::new_v4().to_string();
    streams.insert(
        key,
        InputStream {
            id: id.clone(),
            runtime_id,
            ordered: OrderedInput::default(),
        },
    );
    Ok(TerminalInputBinding { stream_id: id })
}

pub(super) fn close_stream(io: &SessionIo, session_id: &str, owner: &str, stream_id: &str) {
    if let Some(streams) = STREAMS.get() {
        let mut streams = streams
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let key = stream_key(io, session_id, owner);
        if streams
            .get(&key)
            .is_some_and(|stream| stream.id == stream_id)
        {
            streams.remove(&key);
        }
    }
}

pub(super) fn clear_session_streams(path: &Path, session_id: &str) {
    if let Some(streams) = STREAMS.get() {
        streams
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .retain(|(store_path, id, _), _| store_path != path || id != session_id);
    }
}

pub(super) fn accept_text(
    io: SessionIo,
    session_id: String,
    owner: &str,
    order: TerminalInputOrder,
    text: String,
    coalesce: bool,
    sensitive: bool,
) -> Result<(), String> {
    let key = stream_key(&io, &session_id, owner);
    let mut streams = STREAMS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .map_err(|error| error.to_string())?;
    let stream = streams
        .get_mut(&key)
        .filter(|stream| stream.id == order.stream_id)
        .ok_or_else(|| "终端输入通道已关闭或被替换，已拒绝旧输入".to_string())?;
    let runtime_id = &stream.runtime_id;
    stream.ordered.accept(
        order.sequence,
        InputPacket {
            text: Zeroizing::new(text),
            coalesce,
            sensitive,
        },
        |packet| {
            enqueue_terminal_stream_text(
                io.clone(),
                session_id.clone(),
                runtime_id,
                packet.text.to_string(),
                packet.coalesce,
                packet.sensitive,
            )
        },
    )
}

#[tauri::command]
pub(crate) fn begin_terminal_input_stream(
    state: State<'_, AppState>,
    window: WebviewWindow,
    session_id: String,
) -> Result<TerminalInputBinding, String> {
    begin_stream(&state.session_io(), &session_id, window.label())
}

#[tauri::command]
pub(crate) fn close_terminal_input_stream(
    state: State<'_, AppState>,
    window: WebviewWindow,
    session_id: String,
    stream_id: String,
) {
    close_stream(&state.session_io(), &session_id, window.label(), &stream_id);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn packet(text: &str, sensitive: bool) -> InputPacket {
        InputPacket {
            text: Zeroizing::new(text.into()),
            coalesce: !text.contains('\r'),
            sensitive,
        }
    }

    #[test]
    fn restores_packet_order_without_waiting_for_ipc_responses() {
        let mut ordered = OrderedInput::default();
        let mut sent = Vec::new();
        for (sequence, text, sensitive) in [
            (1, "\x7f", true),
            (3, "\r", false),
            (0, "a", false),
            (2, "b", true),
        ] {
            ordered
                .accept(sequence, packet(text, sensitive), |packet| {
                    sent.push((packet.text.to_string(), packet.sensitive));
                    Ok(())
                })
                .unwrap();
        }
        assert_eq!(
            sent,
            [
                ("a".into(), false),
                ("\x7f".into(), true),
                ("b".into(), true),
                ("\r".into(), false)
            ]
        );
        assert!(ordered.pending.is_empty());
        assert_eq!(ordered.pending_bytes, 0);
    }

    #[test]
    fn retries_do_not_duplicate_pending_or_admitted_input() {
        let mut ordered = OrderedInput::default();
        let mut sent = String::new();
        for (sequence, text) in [(1, "b"), (1, "b"), (0, "a"), (0, "a"), (1, "b")] {
            ordered
                .accept(sequence, packet(text, false), |packet| {
                    sent.push_str(&packet.text);
                    Ok(())
                })
                .unwrap();
        }
        assert_eq!(sent, "ab");
    }

    #[test]
    fn admission_failure_cancels_buffered_suffix_instead_of_reordering_it() {
        let mut ordered = OrderedInput::default();
        ordered
            .accept(1, packet("later", true), |_| panic!("missing prefix"))
            .unwrap();
        assert!(ordered
            .accept(0, packet("first", false), |_| Err("queue full".into()))
            .is_err());
        assert!(ordered.pending.is_empty());
        assert!(ordered
            .accept(2, packet("never", false), |_| panic!(
                "failed stream resumed"
            ))
            .is_err());
    }

    #[test]
    fn bounds_sequence_window_and_buffered_bytes() {
        assert!(OrderedInput::default()
            .accept(32, packet("x", false), |_| Ok(()))
            .is_err());
        assert!(OrderedInput::default()
            .accept(0, packet(&"x".repeat(MAX_PACKET_BYTES + 1), false), |_| Ok(
                ()
            ))
            .is_err());
        let mut ordered = OrderedInput::default();
        for sequence in 1..=8 {
            ordered
                .accept(
                    sequence,
                    packet(&"x".repeat(MAX_PACKET_BYTES), true),
                    |_| panic!("missing prefix"),
                )
                .unwrap();
        }
        assert!(ordered.accept(9, packet("x", true), |_| Ok(())).is_err());
        assert!(ordered.pending.is_empty());
    }
}
