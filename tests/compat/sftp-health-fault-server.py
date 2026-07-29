#!/usr/bin/env python3

import struct
import sys
import time


SSH_FXP_INIT = 1
SSH_FXP_VERSION = 2
SSH_FXP_OPENDIR = 11
SSH_FXP_READDIR = 12
SSH_FXP_REALPATH = 16
SSH_FXP_STATUS = 101
SSH_FXP_HANDLE = 102
SSH_FXP_NAME = 104
MAX_PACKET_LENGTH = 1024 * 1024
CLIENT_PACKET_LENGTH_LIMIT = 256 * 1024


def read_exact(length):
    data = bytearray()
    while len(data) < length:
        chunk = sys.stdin.buffer.read(length - len(data))
        if not chunk:
            raise EOFError("SFTP client closed the channel")
        data.extend(chunk)
    return bytes(data)


def read_packet(expected_type):
    length = struct.unpack(">I", read_exact(4))[0]
    if length < 1 or length > MAX_PACKET_LENGTH:
        raise ValueError(f"invalid SFTP packet length: {length}")
    packet = read_exact(length)
    packet_type = packet[0]
    if packet_type != expected_type:
        raise ValueError(
            f"expected SFTP packet type {expected_type}, received {packet_type}"
        )
    return packet[1:]


def write_packet(packet_type, payload):
    packet = bytes([packet_type]) + payload
    sys.stdout.buffer.write(struct.pack(">I", len(packet)) + packet)
    sys.stdout.buffer.flush()


def request_id(payload):
    if len(payload) < 4:
        raise ValueError("SFTP request omitted its request id")
    return struct.unpack(">I", payload[:4])[0]


def encode_string(value):
    return struct.pack(">I", len(value)) + value


def stall():
    time.sleep(30)


def reject_status_requests(status_code, message):
    while True:
        length = struct.unpack(">I", read_exact(4))[0]
        if length < 5 or length > MAX_PACKET_LENGTH:
            raise ValueError(f"invalid SFTP request packet length: {length}")
        packet = read_exact(length)
        write_packet(
            SSH_FXP_STATUS,
            struct.pack(">II", request_id(packet[1:]), status_code)
            + encode_string(message)
            + encode_string(b""),
        )


def main():
    if len(sys.argv) != 2:
        raise ValueError("expected exactly one SFTP fault mode")
    mode = sys.argv[1]
    fixed_modes = {
        "init",
        "canonicalize",
        "malformed-packet",
        "opendir",
        "oversized-packet",
        "readdir",
        "truncated-packet",
        "wrong-request-id",
        "no-space",
        "quota-exceeded",
        "unknown-status",
    }
    status_code = None
    if mode.startswith("status-"):
        encoded_status = mode.removeprefix("status-")
        if not encoded_status.isascii() or not encoded_status.isdecimal():
            raise ValueError(f"invalid numeric SFTP status fault mode: {mode}")
        status_code = int(encoded_status)
        if status_code < 0 or status_code > 0xFFFFFFFF:
            raise ValueError(f"SFTP status fault code is out of range: {status_code}")
    elif mode not in fixed_modes:
        raise ValueError(
            "expected init, canonicalize, opendir, readdir, no-space, quota-exceeded, "
            "unknown-status, malformed-packet, oversized-packet, truncated-packet, "
            "wrong-request-id, or status-N fault mode"
        )

    init = read_packet(SSH_FXP_INIT)
    if len(init) < 4:
        raise ValueError("SFTP init packet omitted its version")
    if mode == "init":
        stall()
        return
    write_packet(SSH_FXP_VERSION, struct.pack(">I", 3))
    if mode in {
        "malformed-packet",
        "oversized-packet",
        "truncated-packet",
        "wrong-request-id",
    }:
        length = struct.unpack(">I", read_exact(4))[0]
        if length < 5 or length > MAX_PACKET_LENGTH:
            raise ValueError(f"invalid SFTP request packet length: {length}")
        packet = read_exact(length)
        if mode == "malformed-packet":
            write_packet(255, b"")
        elif mode == "wrong-request-id":
            write_packet(
                SSH_FXP_STATUS,
                struct.pack(">II", (request_id(packet[1:]) + 1) & 0xFFFFFFFF, 4)
                + encode_string(b"wrong request id injected by PortMate fault server")
                + encode_string(b""),
            )
        elif mode == "truncated-packet":
            sys.stdout.buffer.write(struct.pack(">I", 9) + bytes([SSH_FXP_STATUS]))
            sys.stdout.buffer.flush()
        else:
            sys.stdout.buffer.write(
                struct.pack(">I", CLIENT_PACKET_LENGTH_LIMIT + 1)
            )
            sys.stdout.buffer.flush()
            stall()
        return
    status_faults = {
        "no-space": (14, b"no space injected by PortMate fault server"),
        "quota-exceeded": (15, b"quota exceeded injected by PortMate fault server"),
        "unknown-status": (99, b"unknown status injected by PortMate fault server"),
    }
    if status_code is not None:
        reject_status_requests(
            status_code, f"status {status_code} injected by PortMate fault server".encode()
        )
        return
    if mode in status_faults:
        reject_status_requests(*status_faults[mode])
        return

    realpath = read_packet(SSH_FXP_REALPATH)
    if mode == "canonicalize":
        stall()
        return
    realpath_id = request_id(realpath)
    canonical_path = b"/home/portmate"
    write_packet(
        SSH_FXP_NAME,
        struct.pack(">II", realpath_id, 1)
        + encode_string(canonical_path)
        + encode_string(canonical_path)
        + struct.pack(">I", 0),
    )

    opendir = read_packet(SSH_FXP_OPENDIR)
    if mode == "opendir":
        stall()
        return
    opendir_id = request_id(opendir)
    write_packet(SSH_FXP_HANDLE, struct.pack(">I", opendir_id) + encode_string(b"health"))

    read_packet(SSH_FXP_READDIR)
    stall()


if __name__ == "__main__":
    try:
        main()
    except (EOFError, ValueError) as error:
        print(error, file=sys.stderr)
        sys.exit(65)
