#!/usr/bin/env python3

import fcntl
import os
import struct
import termios

from zope.interface import implementer

from twisted.conch import telnet
from twisted.internet import protocol, reactor
from twisted.internet.error import ProcessExitedAlready


TERMINAL_TYPE = b"\x18"
TERMINAL_TYPE_IS = b"\x00"
TERMINAL_TYPE_SEND = b"\x01"


class ShellProcessProtocol(protocol.ProcessProtocol):
    def __init__(self, owner):
        self.owner = owner

    def connectionMade(self):
        self.owner.process_transport = self.transport
        self.owner.apply_terminal_size()
        if self.owner.pending_input:
            self.transport.write(bytes(self.owner.pending_input))
            self.owner.pending_input.clear()

    def outReceived(self, data):
        self.owner.write_to_client(data)

    def errReceived(self, data):
        self.owner.write_to_client(data)

    def processEnded(self, _reason):
        self.owner.process_transport = None
        if self.owner.network_connected:
            self.owner.transport.loseConnection()


@implementer(telnet.ITelnetProtocol)
class ShellProtocol(protocol.Protocol):
    def __init__(self):
        self.network_connected = False
        self.process_transport = None
        self.pending_input = bytearray()
        self.cols = 80
        self.rows = 24
        self.terminal_type = "xterm"

    def connectionMade(self):
        self.network_connected = True
        self.transport.negotiationMap[telnet.NAWS] = self.telnet_naws
        self.transport.negotiationMap[TERMINAL_TYPE] = self.telnet_terminal_type

        self.transport.do(telnet.NAWS).addErrback(self.ignore_negotiation_failure)
        terminal_type = self.transport.do(TERMINAL_TYPE)
        terminal_type.addCallback(
            lambda _result: self.transport.requestNegotiation(
                TERMINAL_TYPE, TERMINAL_TYPE_SEND
            )
        )
        terminal_type.addErrback(self.ignore_negotiation_failure)
        self.transport.will(telnet.ECHO).addErrback(self.ignore_negotiation_failure)
        self.transport.will(telnet.SGA).addErrback(self.ignore_negotiation_failure)

        environment = os.environ.copy()
        environment.update(
            {
                "HOME": "/root",
                "LANG": "C.UTF-8",
                "PS1": "$ ",
                "TERM": "xterm-256color",
            }
        )
        reactor.spawnProcess(
            ShellProcessProtocol(self),
            "/bin/sh",
            ["/bin/sh", "-i"],
            env=environment,
            usePTY=True,
        )

    def connectionLost(self, _reason):
        self.network_connected = False
        process_transport = self.process_transport
        self.process_transport = None
        if process_transport is not None:
            try:
                process_transport.signalProcess("TERM")
            except ProcessExitedAlready:
                pass

    def dataReceived(self, data):
        if self.process_transport is None:
            self.pending_input.extend(data)
        else:
            self.process_transport.write(data)

    def write_to_client(self, data):
        if self.network_connected:
            self.transport.write(data)

    def enableLocal(self, option):
        return option in {telnet.ECHO, telnet.SGA}

    def enableRemote(self, option):
        return option in {telnet.NAWS, telnet.SGA, TERMINAL_TYPE}

    def disableLocal(self, _option):
        pass

    def disableRemote(self, _option):
        pass

    def unhandledCommand(self, _command, _argument):
        pass

    def unhandledSubnegotiation(self, _command, _data):
        pass

    def telnet_naws(self, data):
        payload = b"".join(data)
        if len(payload) != 4:
            return
        self.cols, self.rows = struct.unpack("!HH", payload)
        self.apply_terminal_size()

    def telnet_terminal_type(self, data):
        payload = b"".join(data)
        if payload[:1] == TERMINAL_TYPE_IS:
            self.terminal_type = payload[1:].decode("ascii", errors="replace")[:128]

    def apply_terminal_size(self):
        process_transport = self.process_transport
        if process_transport is None:
            return
        descriptor = getattr(process_transport, "fd", None)
        if descriptor is None:
            return
        dimensions = struct.pack("HHHH", self.rows, self.cols, 0, 0)
        try:
            fcntl.ioctl(descriptor, termios.TIOCSWINSZ, dimensions)
        except OSError:
            pass

    @staticmethod
    def ignore_negotiation_failure(_failure):
        return None


def build_protocol():
    return telnet.TelnetTransport(ShellProtocol)


factory = protocol.Factory()
factory.protocol = build_protocol
reactor.listenTCP(23, factory, backlog=64, interface="0.0.0.0")
reactor.run()
