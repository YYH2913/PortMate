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
    pending_cr: bool,
    runtime: Arc<TelnetRuntimeState>,
}

impl TelnetNegotiator {
    pub(super) fn new(runtime: Arc<TelnetRuntimeState>) -> Self {
        Self {
            state: TelnetState::Data,
            subnegotiation: Vec::new(),
            pending_cr: false,
            runtime,
        }
    }

    fn push_data_byte(&mut self, byte: u8, output: &mut Vec<u8>, remote_binary: bool) {
        if remote_binary {
            self.flush_pending_cr(output);
            output.push(byte);
            return;
        }
        if self.pending_cr {
            output.push(b'\r');
            self.pending_cr = false;
            if byte == 0 {
                return;
            }
        }
        if byte == b'\r' {
            self.pending_cr = true;
        } else {
            output.push(byte);
        }
    }

    fn flush_pending_cr(&mut self, output: &mut Vec<u8>) {
        if self.pending_cr {
            output.push(b'\r');
            self.pending_cr = false;
        }
    }

    pub(super) fn finish(&mut self) -> Vec<u8> {
        let mut output = Vec::new();
        self.flush_pending_cr(&mut output);
        output
    }

    pub(super) fn filter(&mut self, input: &[u8]) -> (Vec<u8>, Vec<Vec<u8>>) {
        let mut output = Vec::with_capacity(input.len());
        let mut replies = Vec::new();
        let mut remote_binary = self.runtime.remote_binary.load(Ordering::SeqCst);
        for byte in input {
            match self.state {
                TelnetState::Data => {
                    if *byte == TELNET_IAC {
                        self.flush_pending_cr(&mut output);
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
                        self.state = TelnetState::Subnegotiation;
                    }
                    _ => {
                        self.state = TelnetState::Data;
                    }
                },
                TelnetState::Command(command) => {
                    replies.extend(telnet_option_replies(command, *byte, &self.runtime));
                    remote_binary = self.runtime.remote_binary.load(Ordering::SeqCst);
                    self.state = TelnetState::Data;
                }
                TelnetState::Subnegotiation => {
                    if *byte == TELNET_IAC {
                        self.state = TelnetState::SubnegotiationIac;
                    } else {
                        self.subnegotiation.push(*byte);
                    }
                }
                TelnetState::SubnegotiationIac => {
                    if *byte == TELNET_SE {
                        if let Some(reply) = telnet_subnegotiation_reply(
                            &self.subnegotiation,
                            &self.runtime.terminal_type,
                        ) {
                            replies.push(reply);
                        }
                        self.subnegotiation.clear();
                        self.state = TelnetState::Data;
                    } else if *byte == TELNET_IAC {
                        self.subnegotiation.push(TELNET_IAC);
                        self.state = TelnetState::Subnegotiation;
                    } else {
                        self.subnegotiation.push(TELNET_IAC);
                        self.subnegotiation.push(*byte);
                        self.state = TelnetState::Subnegotiation;
                    }
                }
            }
        }
        (output, replies)
    }
}

fn telnet_option_replies(command: u8, option: u8, runtime: &TelnetRuntimeState) -> Vec<Vec<u8>> {
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
