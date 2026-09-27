use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};
use std::sync::Arc;

use portmate_core::{ConnectionConfig, SessionProfile};

pub(super) const TELNET_IAC: u8 = 255;
pub(super) const TELNET_SE: u8 = 240;
pub(super) const TELNET_SB: u8 = 250;
pub(super) const TELNET_WILL: u8 = 251;
pub(super) const TELNET_WONT: u8 = 252;
pub(super) const TELNET_DO: u8 = 253;
pub(super) const TELNET_DONT: u8 = 254;
pub(super) const TELNET_OPT_BINARY: u8 = 0;
pub(super) const TELNET_OPT_ECHO: u8 = 1;
const TELNET_OPT_SUPPRESS_GO_AHEAD: u8 = 3;
pub(super) const TELNET_OPT_TERMINAL_TYPE: u8 = 24;
pub(super) const TELNET_OPT_NAWS: u8 = 31;
pub(super) const TELNET_TTYPE_IS: u8 = 0;
pub(super) const TELNET_TTYPE_SEND: u8 = 1;

pub(super) struct TelnetRuntimeState {
    pub(super) protocol_lane: Arc<tokio::sync::Mutex<()>>,
    binary_enabled: bool,
    naws_enabled: bool,
    pub(super) local_binary: AtomicBool,
    pub(super) remote_binary: AtomicBool,
    pub(super) naws_negotiated: AtomicBool,
    pub(super) cols: AtomicU16,
    pub(super) rows: AtomicU16,
    terminal_type: String,
}

impl TelnetRuntimeState {
    pub(super) fn from_profile(profile: &SessionProfile) -> Option<Arc<Self>> {
        let ConnectionConfig::Telnet(tcp) = &profile.connection else {
            return None;
        };
        Some(Arc::new(Self {
            protocol_lane: Arc::new(tokio::sync::Mutex::new(())),
            binary_enabled: tcp.telnet_binary,
            naws_enabled: tcp.telnet_naws,
            local_binary: AtomicBool::new(false),
            remote_binary: AtomicBool::new(false),
            naws_negotiated: AtomicBool::new(false),
            cols: AtomicU16::new(profile.terminal.cols),
            rows: AtomicU16::new(profile.terminal.rows),
            terminal_type: profile.terminal.term.clone(),
        }))
    }
}

enum TelnetState {
    Data,
    Iac,
    Command(u8),
    Subnegotiation,
    SubnegotiationIac,
}

pub(super) struct TelnetNegotiator {
    state: TelnetState,
    pub(super) subnegotiation: Vec<u8>,
    after_cr: bool,
    local_options: [bool; 256],
    remote_options: [bool; 256],
    subnegotiation_overflow: bool,
    runtime: Arc<TelnetRuntimeState>,
}

impl TelnetNegotiator {
    pub(super) fn new(runtime: Arc<TelnetRuntimeState>) -> Self {
        Self {
            state: TelnetState::Data,
            subnegotiation: Vec::new(),
            after_cr: false,
            local_options: [false; 256],
            remote_options: [false; 256],
            subnegotiation_overflow: false,
            runtime,
        }
    }

    fn push_data_byte(&mut self, byte: u8, output: &mut Vec<u8>, remote_binary: bool) {
        if remote_binary {
            self.after_cr = false;
            output.push(byte);
            return;
        }
        if self.after_cr {
            self.after_cr = false;
            if byte == 0 {
                return;
            }
        }
        // Render CR immediately; only its optional NVT NUL padding is pending.
        // Telnet commands may be interleaved between the CR and its padding.
        self.after_cr = byte == b'\r';
        output.push(byte);
    }

    pub(super) fn finish(&mut self) -> Vec<u8> {
        self.after_cr = false;
        Vec::new()
    }

    pub(super) fn filter(&mut self, input: &[u8]) -> (Vec<u8>, Vec<Vec<u8>>) {
        let mut output = Vec::with_capacity(input.len());
        let mut replies = Vec::new();
        let mut remote_binary = self.runtime.remote_binary.load(Ordering::SeqCst);
        for byte in input {
            match self.state {
                TelnetState::Data => {
                    if *byte == TELNET_IAC {
                        self.state = TelnetState::Iac;
                    } else {
                        self.push_data_byte(*byte, &mut output, remote_binary);
                    }
                }
                TelnetState::Iac => match *byte {
                    TELNET_IAC => {
                        self.push_data_byte(TELNET_IAC, &mut output, remote_binary);
                        self.state = TelnetState::Data;
                    }
                    TELNET_DO | TELNET_DONT | TELNET_WILL | TELNET_WONT => {
                        self.state = TelnetState::Command(*byte);
                    }
                    TELNET_SB => {
                        self.subnegotiation.clear();
                        self.subnegotiation_overflow = false;
                        self.state = TelnetState::Subnegotiation;
                    }
                    _ => {
                        self.state = TelnetState::Data;
                    }
                },
                TelnetState::Command(command) => {
                    replies.extend(telnet_option_replies(
                        command,
                        *byte,
                        &self.runtime,
                        &mut self.local_options,
                        &mut self.remote_options,
                    ));
                    remote_binary = self.runtime.remote_binary.load(Ordering::SeqCst);
                    self.state = TelnetState::Data;
                }
                TelnetState::Subnegotiation => {
                    if *byte == TELNET_IAC {
                        self.state = TelnetState::SubnegotiationIac;
                    } else {
                        self.push_subnegotiation(*byte);
                    }
                }
                TelnetState::SubnegotiationIac => {
                    if *byte == TELNET_SE {
                        if let Some(reply) = (!self.subnegotiation_overflow).then(|| telnet_subnegotiation_reply(
                            &self.subnegotiation,
                            &self.runtime.terminal_type,
                        )).flatten() {
                            replies.push(reply);
                        }
                        self.subnegotiation.clear();
                        self.state = TelnetState::Data;
                    } else if *byte == TELNET_IAC {
                        self.push_subnegotiation(TELNET_IAC);
                        self.state = TelnetState::Subnegotiation;
                    } else {
                        self.push_subnegotiation(TELNET_IAC);
                        self.push_subnegotiation(*byte);
                        self.state = TelnetState::Subnegotiation;
                    }
                }
            }
        }
        (output, replies)
    }

    fn push_subnegotiation(&mut self, byte: u8) {
        if self.subnegotiation.len() < 4096 && !self.subnegotiation_overflow {
            self.subnegotiation.push(byte);
        } else {
            self.subnegotiation.clear();
            self.subnegotiation_overflow = true;
        }
    }
}

fn telnet_option_replies(
    command: u8,
    option: u8,
    runtime: &TelnetRuntimeState,
    local_options: &mut [bool; 256],
    remote_options: &mut [bool; 256],
) -> Vec<Vec<u8>> {
    // We only respond to peer-initiated negotiation. RFC 854 requires silence
    // for requests for an already active mode and negative acknowledgements
    // of an already disabled mode, otherwise peers can negotiate forever.
    let enabled = match command {
        TELNET_DO | TELNET_DONT => &mut local_options[usize::from(option)],
        TELNET_WILL | TELNET_WONT => &mut remote_options[usize::from(option)],
        _ => return Vec::new(),
    };
    if matches!(command, TELNET_DO | TELNET_WILL) == *enabled {
        return Vec::new();
    }
    let response = match command {
        TELNET_DO => match option {
            TELNET_OPT_BINARY if runtime.binary_enabled => {
                runtime.local_binary.store(true, Ordering::SeqCst);
                TELNET_WILL
            }
            TELNET_OPT_BINARY => {
                runtime.local_binary.store(false, Ordering::SeqCst);
                TELNET_WONT
            }
            TELNET_OPT_NAWS if runtime.naws_enabled => {
                runtime.naws_negotiated.store(true, Ordering::SeqCst);
                TELNET_WILL
            }
            TELNET_OPT_NAWS => {
                runtime.naws_negotiated.store(false, Ordering::SeqCst);
                TELNET_WONT
            }
            TELNET_OPT_SUPPRESS_GO_AHEAD | TELNET_OPT_TERMINAL_TYPE => TELNET_WILL,
            _ => TELNET_WONT,
        },
        TELNET_DONT => {
            if option == TELNET_OPT_BINARY {
                runtime.local_binary.store(false, Ordering::SeqCst);
            } else if option == TELNET_OPT_NAWS {
                runtime.naws_negotiated.store(false, Ordering::SeqCst);
            }
            TELNET_WONT
        }
        TELNET_WILL => match option {
            TELNET_OPT_BINARY if runtime.binary_enabled => {
                runtime.remote_binary.store(true, Ordering::SeqCst);
                TELNET_DO
            }
            TELNET_OPT_BINARY => {
                runtime.remote_binary.store(false, Ordering::SeqCst);
                TELNET_DONT
            }
            TELNET_OPT_ECHO | TELNET_OPT_SUPPRESS_GO_AHEAD => TELNET_DO,
            _ => TELNET_DONT,
        },
        TELNET_WONT => {
            if option == TELNET_OPT_BINARY {
                runtime.remote_binary.store(false, Ordering::SeqCst);
            }
            TELNET_DONT
        }
        _ => return Vec::new(),
    };
    *enabled = matches!(response, TELNET_WILL | TELNET_DO);
    let mut replies = vec![vec![TELNET_IAC, response, option]];
    if command == TELNET_DO && option == TELNET_OPT_NAWS && response == TELNET_WILL {
        replies.push(telnet_naws_message(
            runtime.cols.load(Ordering::SeqCst),
            runtime.rows.load(Ordering::SeqCst),
        ));
    }
    replies
}

fn telnet_subnegotiation_reply(payload: &[u8], terminal_type: &str) -> Option<Vec<u8>> {
    if payload.first().copied() == Some(TELNET_OPT_TERMINAL_TYPE)
        && payload.get(1).copied() == Some(TELNET_TTYPE_SEND)
    {
        let mut reply = vec![
            TELNET_IAC,
            TELNET_SB,
            TELNET_OPT_TERMINAL_TYPE,
            TELNET_TTYPE_IS,
        ];
        append_telnet_subnegotiation_payload(&mut reply, terminal_type.as_bytes());
        reply.extend_from_slice(&[TELNET_IAC, TELNET_SE]);
        return Some(reply);
    }
    None
}

fn append_telnet_subnegotiation_payload(output: &mut Vec<u8>, payload: &[u8]) {
    for byte in payload {
        output.push(*byte);
        if *byte == TELNET_IAC {
            output.push(*byte);
        }
    }
}

pub(super) fn telnet_naws_message(cols: u16, rows: u16) -> Vec<u8> {
    let mut message = vec![TELNET_IAC, TELNET_SB, TELNET_OPT_NAWS];
    append_telnet_subnegotiation_payload(&mut message, &cols.to_be_bytes());
    append_telnet_subnegotiation_payload(&mut message, &rows.to_be_bytes());
    message.extend_from_slice(&[TELNET_IAC, TELNET_SE]);
    message
}

pub(super) fn encode_telnet_outbound_text(text: &str, local_binary: bool) -> String {
    if local_binary {
        return text.to_string();
    }
    let mut output = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\r' => {
                output.push('\r');
                if chars.peek().copied() == Some('\n') {
                    chars.next();
                    output.push('\n');
                } else {
                    output.push('\0');
                }
            }
            '\n' => output.push_str("\r\n"),
            _ => output.push(ch),
        }
    }
    output
}

pub(super) fn encode_telnet_outbound_bytes(bytes: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(bytes.len());
    for byte in bytes {
        output.push(*byte);
        if *byte == TELNET_IAC {
            output.push(*byte);
        }
    }
    output
}
