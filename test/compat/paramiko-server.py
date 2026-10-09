#!/usr/bin/env python3

import errno
import fcntl
import os
import signal
import socket
import struct
import subprocess
import termios
import threading
import time
from contextlib import suppress

import paramiko


USERNAME = "portmate"
PASSWORD = "portmate"
HOME = "/home/portmate"
HOST_KEY_PATH = "/etc/portmate/ssh_host_key"
READ_SIZE = 32768


def sftp_error(error: OSError) -> int:
    return paramiko.SFTPServer.convert_errno(error.errno or errno.EIO)


class LocalSftpHandle(paramiko.SFTPHandle):
    def __init__(self, flags: int, path: str, file_object) -> None:
        super().__init__(flags)
        self.path = path
        self.readfile = file_object
        self.writefile = file_object

    def stat(self):
        try:
            return paramiko.SFTPAttributes.from_stat(os.fstat(self.readfile.fileno()))
        except OSError as error:
            return sftp_error(error)

    def chattr(self, attr):
        try:
            paramiko.SFTPServer.set_file_attr(self.path, attr)
            return paramiko.SFTP_OK
        except OSError as error:
            return sftp_error(error)


class LocalSftpServer(paramiko.SFTPServerInterface):
    @staticmethod
    def _path(path: str) -> str:
        if os.path.isabs(path):
            return os.path.normpath(path)
        return os.path.normpath(f"/{path}")

    def canonicalize(self, path: str) -> str:
        return self._path(path)

    def list_folder(self, path: str):
        try:
            entries = []
            for entry in os.scandir(self._path(path)):
                attr = paramiko.SFTPAttributes.from_stat(entry.stat())
                attr.filename = entry.name
                entries.append(attr)
            return entries
        except OSError as error:
            return sftp_error(error)

    def stat(self, path: str):
        try:
            return paramiko.SFTPAttributes.from_stat(os.stat(self._path(path)))
        except OSError as error:
            return sftp_error(error)

    def lstat(self, path: str):
        try:
            return paramiko.SFTPAttributes.from_stat(os.lstat(self._path(path)))
        except OSError as error:
            return sftp_error(error)

    def open(self, path: str, flags: int, attr):
        local_path = self._path(path)
        mode = getattr(attr, "st_mode", None) or 0o666
        try:
            descriptor = os.open(local_path, flags, mode)
            if flags & os.O_WRONLY:
                file_mode = "ab" if flags & os.O_APPEND else "wb"
            elif flags & os.O_RDWR:
                file_mode = "a+b" if flags & os.O_APPEND else "r+b"
            else:
                file_mode = "rb"
            file_object = os.fdopen(descriptor, file_mode, buffering=0)
        except OSError as error:
            with suppress(UnboundLocalError):
                os.close(descriptor)
            return sftp_error(error)

        try:
            if flags & os.O_CREAT and attr is not None:
                paramiko.SFTPServer.set_file_attr(local_path, attr)
        except OSError as error:
            file_object.close()
            return sftp_error(error)
        return LocalSftpHandle(flags, local_path, file_object)

    def remove(self, path: str):
        try:
            os.remove(self._path(path))
            return paramiko.SFTP_OK
        except OSError as error:
            return sftp_error(error)

    def rename(self, oldpath: str, newpath: str):
        target = self._path(newpath)
        if os.path.lexists(target):
            return paramiko.SFTP_FAILURE
        try:
            os.rename(self._path(oldpath), target)
            return paramiko.SFTP_OK
        except OSError as error:
            return sftp_error(error)

    def posix_rename(self, oldpath: str, newpath: str):
        try:
            os.replace(self._path(oldpath), self._path(newpath))
            return paramiko.SFTP_OK
        except OSError as error:
            return sftp_error(error)

    def mkdir(self, path: str, attr):
        mode = getattr(attr, "st_mode", None) or 0o777
        try:
            os.mkdir(self._path(path), mode)
            return paramiko.SFTP_OK
        except OSError as error:
            return sftp_error(error)

    def rmdir(self, path: str):
        try:
            os.rmdir(self._path(path))
            return paramiko.SFTP_OK
        except OSError as error:
            return sftp_error(error)

    def chattr(self, path: str, attr):
        try:
            paramiko.SFTPServer.set_file_attr(self._path(path), attr)
            return paramiko.SFTP_OK
        except OSError as error:
            return sftp_error(error)

    def symlink(self, target_path: str, path: str):
        try:
            os.symlink(target_path, self._path(path))
            return paramiko.SFTP_OK
        except OSError as error:
            return sftp_error(error)

    def readlink(self, path: str):
        try:
            return os.readlink(self._path(path))
        except OSError as error:
            return sftp_error(error)


def process_environment(term: str = "xterm-256color") -> dict[str, str]:
    return {
        "HOME": HOME,
        "LANG": "C.UTF-8",
        "LOGNAME": USERNAME,
        "PATH": "/usr/local/bin:/usr/bin:/bin",
        "SHELL": "/bin/sh",
        "TERM": term,
        "USER": USERNAME,
    }


def send_channel(channel: paramiko.Channel, data: bytes, stderr: bool = False) -> None:
    offset = 0
    while offset < len(data) and not channel.closed:
        sent = channel.send_stderr(data[offset:]) if stderr else channel.send(data[offset:])
        if sent <= 0:
            break
        offset += sent


def terminate_process(process: subprocess.Popen) -> None:
    if process.poll() is not None:
        return
    with suppress(ProcessLookupError):
        os.killpg(process.pid, signal.SIGTERM)
    try:
        process.wait(timeout=2)
        return
    except subprocess.TimeoutExpired:
        pass
    with suppress(ProcessLookupError):
        os.killpg(process.pid, signal.SIGKILL)
    with suppress(subprocess.TimeoutExpired):
        process.wait(timeout=2)


def pump_channel_to_file(channel: paramiko.Channel, target) -> None:
    try:
        while not channel.closed:
            data = channel.recv(READ_SIZE)
            if not data:
                break
            target.write(data)
            target.flush()
    except (BrokenPipeError, EOFError, OSError, socket.error):
        pass
    finally:
        with suppress(BrokenPipeError, OSError):
            target.close()


def pump_file_to_channel(channel: paramiko.Channel, source, stderr: bool = False) -> None:
    try:
        while not channel.closed:
            data = os.read(source.fileno(), READ_SIZE)
            if not data:
                break
            send_channel(channel, data, stderr)
    except (EOFError, OSError, socket.error):
        pass


def finish_process(channel: paramiko.Channel, process: subprocess.Popen, workers: list[threading.Thread]) -> None:
    try:
        while process.poll() is None and not channel.closed:
            time.sleep(0.05)
        if channel.closed:
            terminate_process(process)
        else:
            returncode = process.wait()
            for worker in workers:
                worker.join(timeout=1)
            with suppress(EOFError, OSError, socket.error):
                channel.send_exit_status(returncode if returncode >= 0 else 128 - returncode)
    finally:
        terminate_process(process)
        with suppress(EOFError, OSError, socket.error):
            channel.close()


def run_exec(channel: paramiko.Channel, command: bytes) -> None:
    try:
        process = subprocess.Popen(
            ["/bin/sh", "-c", command.decode("utf-8", "surrogateescape")],
            cwd=HOME,
            env=process_environment(),
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            start_new_session=True,
        )
    except (OSError, ValueError) as error:
        send_channel(channel, f"Command failed: {error}\n".encode(), stderr=True)
        channel.send_exit_status(127)
        channel.close()
        return

    assert process.stdin is not None
    assert process.stdout is not None
    assert process.stderr is not None
    workers = [
        threading.Thread(target=pump_channel_to_file, args=(channel, process.stdin), daemon=True),
        threading.Thread(target=pump_file_to_channel, args=(channel, process.stdout), daemon=True),
        threading.Thread(target=pump_file_to_channel, args=(channel, process.stderr, True), daemon=True),
    ]
    for worker in workers:
        worker.start()
    finish_process(channel, process, workers)


def resize_pty(master_fd: int, width: int, height: int) -> None:
    with suppress(OSError):
        fcntl.ioctl(master_fd, termios.TIOCSWINSZ, struct.pack("HHHH", height, width, 0, 0))


def pump_channel_to_pty(channel: paramiko.Channel, master_fd: int) -> None:
    try:
        while not channel.closed:
            data = channel.recv(READ_SIZE)
            if not data:
                break
            offset = 0
            while offset < len(data):
                offset += os.write(master_fd, data[offset:])
    except (EOFError, OSError, socket.error):
        pass


def pump_pty_to_channel(channel: paramiko.Channel, master_fd: int) -> None:
    try:
        while not channel.closed:
            data = os.read(master_fd, READ_SIZE)
            if not data:
                break
            send_channel(channel, data)
    except (EOFError, OSError, socket.error):
        pass


def run_shell(channel: paramiko.Channel, term: str, width: int, height: int, server) -> None:
    master_fd, slave_fd = os.openpty()
    resize_pty(master_fd, width, height)
    try:
        process = subprocess.Popen(
            ["/usr/bin/setsid", "--ctty", "--wait", "/bin/sh", "-l"],
            cwd=HOME,
            env=process_environment(term),
            stdin=slave_fd,
            stdout=slave_fd,
            stderr=slave_fd,
            close_fds=True,
        )
    except OSError as error:
        os.close(master_fd)
        send_channel(channel, f"Shell failed: {error}\n".encode(), stderr=True)
        channel.send_exit_status(127)
        channel.close()
        return
    finally:
        os.close(slave_fd)

    server.register_pty(channel, master_fd)
    workers = [
        threading.Thread(target=pump_channel_to_pty, args=(channel, master_fd), daemon=True),
        threading.Thread(target=pump_pty_to_channel, args=(channel, master_fd), daemon=True),
    ]
    for worker in workers:
        worker.start()
    try:
        finish_process(channel, process, workers)
    finally:
        server.unregister_pty(channel)
        with suppress(OSError):
            os.close(master_fd)


class PortMateServer(paramiko.ServerInterface):
    def __init__(self) -> None:
        self._lock = threading.Lock()
        self._requests: dict[int, tuple[str, int, int]] = {}
        self._ptys: dict[int, int] = {}

    def check_auth_password(self, username: str, password: str):
        if username == USERNAME and password == PASSWORD:
            return paramiko.AUTH_SUCCESSFUL
        return paramiko.AUTH_FAILED

    def get_allowed_auths(self, username: str) -> str:
        return "password"

    def check_channel_request(self, kind: str, chanid: int):
        if kind == "session":
            return paramiko.OPEN_SUCCEEDED
        return paramiko.OPEN_FAILED_ADMINISTRATIVELY_PROHIBITED

    def check_channel_pty_request(
        self,
        channel: paramiko.Channel,
        term: bytes,
        width: int,
        height: int,
        pixelwidth: int,
        pixelheight: int,
        modes: bytes,
    ) -> bool:
        del pixelwidth, pixelheight, modes
        terminal = term.decode("ascii", "replace") if isinstance(term, bytes) else term
        with self._lock:
            self._requests[channel.get_id()] = (terminal, width, height)
        return True

    def check_channel_window_change_request(
        self,
        channel: paramiko.Channel,
        width: int,
        height: int,
        pixelwidth: int,
        pixelheight: int,
    ) -> bool:
        del pixelwidth, pixelheight
        with self._lock:
            master_fd = self._ptys.get(channel.get_id())
            request = self._requests.get(channel.get_id())
            if request is not None:
                self._requests[channel.get_id()] = (request[0], width, height)
        if master_fd is not None:
            resize_pty(master_fd, width, height)
        return True

    def check_channel_shell_request(self, channel: paramiko.Channel) -> bool:
        with self._lock:
            term, width, height = self._requests.get(
                channel.get_id(), ("xterm-256color", 80, 24)
            )
        threading.Thread(
            target=run_shell,
            args=(channel, term, width, height, self),
            daemon=True,
        ).start()
        return True

    def check_channel_exec_request(self, channel: paramiko.Channel, command: bytes) -> bool:
        threading.Thread(target=run_exec, args=(channel, command), daemon=True).start()
        return True

    def register_pty(self, channel: paramiko.Channel, master_fd: int) -> None:
        with self._lock:
            self._ptys[channel.get_id()] = master_fd
            request = self._requests.get(channel.get_id())
        if request is not None:
            resize_pty(master_fd, request[1], request[2])

    def unregister_pty(self, channel: paramiko.Channel) -> None:
        with self._lock:
            self._ptys.pop(channel.get_id(), None)
            self._requests.pop(channel.get_id(), None)


def serve_connection(client: socket.socket, host_key: paramiko.PKey) -> None:
    transport = paramiko.Transport(client)
    transport.local_version = f"SSH-2.0-paramiko_{paramiko.__version__}"
    transport.add_server_key(host_key)
    transport.set_subsystem_handler("sftp", paramiko.SFTPServer, LocalSftpServer)
    channels: list[paramiko.Channel] = []
    try:
        transport.start_server(server=PortMateServer())
        while transport.is_active():
            channel = transport.accept(timeout=1)
            if channel is not None:
                channels.append(channel)
            channels = [active for active in channels if not active.closed]
    except (EOFError, OSError, paramiko.SSHException) as error:
        if transport.is_active():
            print(f"Paramiko connection failed: {error}", flush=True)
    finally:
        transport.close()
        client.close()


def main() -> None:
    stop = threading.Event()
    listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    listener.bind(("0.0.0.0", 22))
    listener.listen(64)
    listener.settimeout(1)
    host_key = paramiko.Ed25519Key.from_private_key_file(HOST_KEY_PATH)

    def request_stop(signum, frame) -> None:
        del signum, frame
        stop.set()

    signal.signal(signal.SIGINT, request_stop)
    signal.signal(signal.SIGTERM, request_stop)
    print("PortMate Paramiko compatibility server listening on port 22", flush=True)
    try:
        while not stop.is_set():
            try:
                client, _ = listener.accept()
            except socket.timeout:
                continue
            threading.Thread(target=serve_connection, args=(client, host_key), daemon=True).start()
    finally:
        listener.close()


if __name__ == "__main__":
    main()
