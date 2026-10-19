use super::*;
use std::io::Cursor;

struct Wire {
    incoming: Cursor<Vec<u8>>,
    outgoing: Vec<u8>,
}

impl Wire {
    fn new(version: &[u8], status: &[u8]) -> Self {
        let mut incoming = packet(version);
        incoming.extend(packet(status));
        Self {
            incoming: Cursor::new(incoming),
            outgoing: Vec::new(),
        }
    }
}

impl Read for Wire {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let length = buffer.len().min(2);
        self.incoming.read(&mut buffer[..length])
    }
}

impl Write for Wire {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let length = buffer.len().min(3);
        self.outgoing.extend_from_slice(&buffer[..length]);
        Ok(length)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn packet(payload: &[u8]) -> Vec<u8> {
    let mut packet = (payload.len() as u32).to_be_bytes().to_vec();
    packet.extend_from_slice(payload);
    packet
}

fn string(value: &[u8]) -> Vec<u8> {
    let mut field = (value.len() as u32).to_be_bytes().to_vec();
    field.extend_from_slice(value);
    field
}

fn version(extensions: &[(&[u8], &[u8])]) -> Vec<u8> {
    let mut payload = vec![2, 0, 0, 0, 3];
    for (name, value) in extensions {
        payload.extend(string(name));
        payload.extend(string(value));
    }
    payload
}

fn status(code: u32) -> Vec<u8> {
    let mut payload = vec![101, 0, 0, 0, 1];
    payload.extend_from_slice(&code.to_be_bytes());
    payload.extend(string(b"server diagnostic"));
    payload.extend(string(b""));
    payload
}

#[test]
fn compatibility_hardlink_handles_split_packets_and_preserves_exact_paths() {
    let mut wire = Wire::new(
        &version(&[(b"other", b"2"), (EXTENSION, b"1")]),
        &status(sys::SSH_FX_OK),
    );
    request(&mut wire, "old '界\\.part", "new \"界\\").unwrap();

    let mut outgoing = Cursor::new(wire.outgoing);
    assert_eq!(read_packet(&mut outgoing).unwrap(), [1, 0, 0, 0, 3]);
    let payload = read_packet(&mut outgoing).unwrap();
    assert_eq!(payload[0], 200); // EXTENDED, never RENAME.
    let mut fields = &payload[1..];
    assert_eq!(take_u32(&mut fields).unwrap(), 1);
    assert_eq!(take_string(&mut fields).unwrap(), b"hardlink@openssh.com");
    assert_eq!(
        take_string(&mut fields).unwrap(),
        "old '界\\.part".as_bytes()
    );
    assert_eq!(take_string(&mut fields).unwrap(), "new \"界\\".as_bytes());
    assert!(fields.is_empty());
    assert_eq!(outgoing.position() as usize, outgoing.get_ref().len());
}

#[test]
fn compatibility_hardlink_rejects_missing_or_unknown_extension_before_mutation() {
    for extensions in [vec![], vec![(EXTENSION, b"2".as_slice())]] {
        let mut wire = Wire::new(&version(&extensions), &status(sys::SSH_FX_OK));
        assert_eq!(
            request(&mut wire, "old", "new"),
            Err(Error::Sftp(SftpError(sys::SSH_FX_OP_UNSUPPORTED)))
        );
        assert_eq!(wire.outgoing, packet(&[1, 0, 0, 0, 3]));
    }
}

#[test]
fn compatibility_hardlink_preserves_server_rejections_including_existing_targets() {
    for code in [
        sys::SSH_FX_PERMISSION_DENIED,
        sys::SSH_FX_FAILURE,
        sys::SSH_FX_OP_UNSUPPORTED,
        sys::SSH_FX_FILE_ALREADY_EXISTS,
    ] {
        let mut wire = Wire::new(&version(&[(EXTENSION, b"1")]), &status(code));
        assert_eq!(
            request(&mut wire, "old", "existing"),
            Err(Error::Sftp(SftpError(code)))
        );
    }
}

#[test]
fn compatibility_hardlink_rejects_malformed_version_before_mutation() {
    for payload in [
        vec![101, 0, 0, 0, 3],
        vec![2, 0, 0, 0, 4],
        vec![2, 0, 0, 0, 3, 0],
        vec![2, 0, 0, 0, 3, 255, 255, 255, 255],
    ] {
        let mut wire = Wire::new(&payload, &status(sys::SSH_FX_OK));
        assert!(request(&mut wire, "old", "new").is_err());
        assert_eq!(wire.outgoing, packet(&[1, 0, 0, 0, 3]));
    }
}

#[test]
fn compatibility_hardlink_rejects_wrong_id_type_truncation_and_trailing_status() {
    let good = status(sys::SSH_FX_OK);
    let mut wrong_id = good.clone();
    wrong_id[4] = 2;
    let mut wrong_type = good.clone();
    wrong_type[0] = 201;
    let mut trailing = good.clone();
    trailing.push(0);
    for payload in [wrong_id, wrong_type, good[..9].to_vec(), trailing] {
        let mut wire = Wire::new(&version(&[(EXTENSION, b"1")]), &payload);
        assert!(request(&mut wire, "old", "new").is_err());
    }
}

#[test]
fn compatibility_hardlink_bounds_packet_allocation_before_reading_a_body() {
    for length in [0, 4, MAX_PACKET_BYTES as u32 + 1, u32::MAX] {
        let mut input = Cursor::new(length.to_be_bytes());
        assert!(read_packet(&mut input).is_err());
        assert_eq!(input.position(), 4);
    }
}

#[test]
fn compatibility_hardlink_rejects_oversized_or_nul_paths_before_any_io() {
    for (old, new) in [
        ("x".repeat(MAX_PACKET_BYTES), "new".to_string()),
        ("old".to_string(), "x".repeat(MAX_PACKET_BYTES)),
        ("old\0bad".to_string(), "new".to_string()),
        ("old".to_string(), "new\0bad".to_string()),
    ] {
        let mut wire = Wire::new(&version(&[(EXTENSION, b"1")]), &status(0));
        assert!(request(&mut wire, &old, &new).is_err());
        assert!(wire.outgoing.is_empty());
    }
}

#[test]
fn compatibility_hardlink_rejects_eof_before_a_complete_reply() {
    let mut wire = Wire::new(&version(&[(EXTENSION, b"1")]), &status(0));
    wire.incoming.get_mut().pop();
    assert!(request(&mut wire, "old", "new").is_err());
}

#[test]
fn compatibility_hardlink_propagates_transport_timeouts() {
    struct TimedOutWire;
    impl Read for TimedOutWire {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::new(io::ErrorKind::TimedOut, "reply deadline"))
        }
    }
    impl Write for TimedOutWire {
        fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
            Ok(buffer.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    assert!(request(&mut TimedOutWire, "old", "new")
        .unwrap_err()
        .to_string()
        .contains("reply deadline"));
}
