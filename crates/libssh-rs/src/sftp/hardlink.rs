use super::{Error, SftpError, SshResult};
use crate::Channel;
use libssh_rs_sys as sys;
use std::io::{self, Read, Write};
use std::time::Instant;

const MAX_PACKET_BYTES: usize = 256 * 1024;
const EXTENSION: &[u8] = b"hardlink@openssh.com";
const REQUEST_ID: u32 = 1;

pub(super) fn request_on_channel(
    channel: &Channel,
    old_path: &str,
    new_path: &str,
    deadline: Instant,
) -> SshResult<()> {
    request(
        &mut DeadlineChannel { channel, deadline },
        old_path,
        new_path,
    )
}

struct DeadlineChannel<'a> {
    channel: &'a Channel,
    deadline: Instant,
}

impl Read for DeadlineChannel<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let remaining = self.channel.set_session_timeout_until(self.deadline)?;
        let read = self.channel.read_timeout(buffer, false, Some(remaining))?;
        if read == 0 && !buffer.is_empty() && !self.channel.is_eof() {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "SFTP hardlink reply timed out",
            ));
        }
        Ok(read)
    }
}

impl Write for DeadlineChannel<'_> {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.channel.set_session_timeout_until(self.deadline)?;
        self.channel.stdin().write(buffer)
    }

    fn flush(&mut self) -> io::Result<()> {
        let remaining = self.channel.set_session_timeout_until(self.deadline)?;
        Ok(self.channel.flush(Some(remaining))?)
    }
}

fn request(io: &mut (impl Read + Write), old_path: &str, new_path: &str) -> SshResult<()> {
    let payload = request_payload(old_path, new_path)?;
    write_packet(io, &[sys::SSH_FXP_INIT as u8, 0, 0, 0, 3])?;
    let version = read_packet(io)?;
    if !supports_hardlink(&version)? {
        return Err(Error::Sftp(SftpError(sys::SSH_FX_OP_UNSUPPORTED)));
    }
    write_packet(io, &payload)?;
    parse_status(&read_packet(io)?)
}

fn request_payload(old_path: &str, new_path: &str) -> SshResult<Vec<u8>> {
    let length = 1_usize
        .checked_add(4 + 12 + EXTENSION.len())
        .and_then(|length| length.checked_add(old_path.len()))
        .and_then(|length| length.checked_add(new_path.len()))
        .filter(|length| *length <= MAX_PACKET_BYTES)
        .ok_or_else(|| Error::fatal("SFTP hardlink request exceeds its packet limit"))?;
    if old_path.contains('\0') || new_path.contains('\0') {
        return Err(Error::fatal("SFTP hardlink path contains a NUL byte"));
    }
    let mut payload = Vec::with_capacity(length);
    payload.push(sys::SSH_FXP_EXTENDED as u8);
    payload.extend_from_slice(&REQUEST_ID.to_be_bytes());
    for value in [EXTENSION, old_path.as_bytes(), new_path.as_bytes()] {
        payload.extend_from_slice(&(value.len() as u32).to_be_bytes());
        payload.extend_from_slice(value);
    }
    Ok(payload)
}

fn write_packet(io: &mut impl Write, payload: &[u8]) -> SshResult<()> {
    let mut packet = Vec::with_capacity(4 + payload.len());
    packet.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    packet.extend_from_slice(payload);
    io.write_all(&packet)?;
    io.flush()?;
    Ok(())
}

fn read_packet(io: &mut impl Read) -> SshResult<Vec<u8>> {
    let mut length = [0; 4];
    io.read_exact(&mut length)?;
    let length = u32::from_be_bytes(length) as usize;
    if !(5..=MAX_PACKET_BYTES).contains(&length) {
        return Err(Error::fatal("invalid SFTP hardlink reply length"));
    }
    let mut packet = vec![0; length];
    io.read_exact(&mut packet)?;
    Ok(packet)
}

fn supports_hardlink(packet: &[u8]) -> SshResult<bool> {
    let mut fields = packet_body(packet, sys::SSH_FXP_VERSION)?;
    if take_u32(&mut fields)? != 3 {
        return Err(Error::fatal("SFTP hardlink requires protocol version 3"));
    }
    let mut supported = false;
    while !fields.is_empty() {
        let name = take_string(&mut fields)?;
        let version = take_string(&mut fields)?;
        if name == EXTENSION && version == b"1" {
            supported = true;
        }
    }
    Ok(supported)
}

fn parse_status(packet: &[u8]) -> SshResult<()> {
    let mut fields = packet_body(packet, sys::SSH_FXP_STATUS)?;
    if take_u32(&mut fields)? != REQUEST_ID {
        return Err(Error::fatal(
            "SFTP hardlink reply has an unexpected request ID",
        ));
    }
    let status = take_u32(&mut fields)?;
    take_string(&mut fields)?; // Server diagnostic.
    take_string(&mut fields)?; // Language tag.
    if !fields.is_empty() {
        return Err(Error::fatal("SFTP hardlink status has trailing data"));
    }
    if status == sys::SSH_FX_OK {
        Ok(())
    } else {
        Err(Error::Sftp(SftpError(status)))
    }
}

fn packet_body(packet: &[u8], expected: u32) -> SshResult<&[u8]> {
    match packet.split_first() {
        Some((&kind, fields)) if u32::from(kind) == expected => Ok(fields),
        _ => Err(Error::fatal("unexpected SFTP hardlink reply type")),
    }
}

fn take_u32(fields: &mut &[u8]) -> SshResult<u32> {
    let bytes = fields
        .get(..4)
        .ok_or_else(|| Error::fatal("truncated SFTP hardlink reply"))?;
    let value = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    *fields = &fields[4..];
    Ok(value)
}

fn take_string<'a>(fields: &mut &'a [u8]) -> SshResult<&'a [u8]> {
    let length = take_u32(fields)? as usize;
    let value = fields
        .get(..length)
        .ok_or_else(|| Error::fatal("truncated SFTP hardlink string"))?;
    *fields = &fields[length..];
    Ok(value)
}

#[cfg(test)]
#[path = "../../../../test/rust/libssh-rs/unit/hardlink.rs"]
mod tests;
