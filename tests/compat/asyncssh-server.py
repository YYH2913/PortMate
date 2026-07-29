#!/usr/bin/env python3

import asyncio
import os
import signal
import sys
from contextlib import suppress

import asyncssh


USERNAME = "portmate"
PASSWORD = "portmate"
HOME = "/home/portmate"


class PortMateSshServer(asyncssh.SSHServer):
    def begin_auth(self, username: str) -> bool:
        return True

    def password_auth_supported(self) -> bool:
        return True

    def validate_password(self, username: str, password: str) -> bool:
        return username == USERNAME and password == PASSWORD


async def terminate_child(child: asyncio.subprocess.Process) -> None:
    if child.returncode is not None:
        return

    with suppress(ProcessLookupError):
        os.killpg(child.pid, signal.SIGTERM)
    try:
        await asyncio.wait_for(child.wait(), timeout=2)
        return
    except asyncio.TimeoutError:
        pass

    with suppress(ProcessLookupError):
        os.killpg(child.pid, signal.SIGKILL)
    with suppress(asyncio.TimeoutError):
        await asyncio.wait_for(child.wait(), timeout=2)


async def handle_process(process: asyncssh.SSHServerProcess) -> None:
    if process.subsystem:
        process.stderr.write(f"Unsupported subsystem: {process.subsystem}\n".encode())
        process.exit(127)
        return

    command = process.command
    argv = ["/bin/sh", "-c", command] if command else ["/bin/sh", "-l"]
    environment = {
        "HOME": HOME,
        "LANG": "C.UTF-8",
        "LOGNAME": USERNAME,
        "PATH": "/usr/local/bin:/usr/bin:/bin",
        "SHELL": "/bin/sh",
        "TERM": process.term_type or "xterm-256color",
        "USER": USERNAME,
    }
    child = await asyncio.create_subprocess_exec(
        *argv,
        cwd=HOME,
        env=environment,
        stdin=asyncio.subprocess.PIPE,
        stdout=asyncio.subprocess.PIPE,
        stderr=asyncio.subprocess.PIPE,
        start_new_session=True,
    )
    assert child.stdin is not None
    assert child.stdout is not None
    assert child.stderr is not None

    try:
        await process.redirect(child.stdin, child.stdout, child.stderr)
        returncode = await child.wait()
        status = returncode if returncode >= 0 else 128 - returncode
        with suppress(asyncssh.Error, BrokenPipeError, ConnectionError):
            process.exit(status)
            await process.wait_closed()
    finally:
        await terminate_child(child)


async def run_server() -> None:
    server = await asyncssh.create_server(
        PortMateSshServer,
        "0.0.0.0",
        22,
        server_host_keys=["/etc/portmate/ssh_host_key"],
        process_factory=handle_process,
        encoding=None,
        line_editor=False,
        sftp_factory=lambda channel: asyncssh.SFTPServer(channel, chroot="/"),
        allow_scp=True,
        login_timeout=20,
        server_version=f"AsyncSSH_{asyncssh.__version__}",
    )
    print("PortMate AsyncSSH compatibility server listening on port 22", flush=True)
    await server.wait_closed()


try:
    asyncio.run(run_server())
except (OSError, asyncssh.Error) as error:
    sys.exit(f"AsyncSSH compatibility server failed: {error}")
