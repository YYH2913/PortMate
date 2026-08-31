import { describe, expect, it } from "vitest";
import { deviceLoadEndpoint, deviceTftpEndpoint, fileTransferProtocolsForProfile, isModemTransferProtocol, isTftpTransferProtocol, modemLoadCommand, transferProtocolLabel, transferProtocolsForProfile } from "./transfer-capabilities";
import type { TransferProtocol } from "./transfer-capabilities";
import type { SessionKind, SessionProfile } from "./types";

describe("transfer capabilities", () => {
  it("offers SSH file transfer and enabled modem protocols for SSH-like profiles", () => {
    expect(transferProtocolsForProfile(profile("ssh"))).toEqual(["sftp", "scp", "xmodem", "ymodem", "zmodem", "tftp"]);
    expect(transferProtocolsForProfile(profile("tmux", { scp: false, ymodem: false }))).toEqual(["sftp", "xmodem", "zmodem", "tftp"]);
  });

  it("does not expose SSH file transfer to non-SSH transports", () => {
    for (const kind of ["serial", "shell", "telnet", "tcp"] as const) {
      expect(transferProtocolsForProfile(profile(kind))).toEqual(["xmodem", "ymodem", "zmodem", "tftp"]);
    }
  });

  it("exposes only enabled SFTP and SCP protocols to file tools", () => {
    expect(fileTransferProtocolsForProfile(profile("ssh"))).toEqual(["sftp", "scp"]);
    expect(fileTransferProtocolsForProfile(profile("tmux", { sftp: false }))).toEqual(["scp"]);
    expect(fileTransferProtocolsForProfile(profile("ssh", { scp: false }))).toEqual(["sftp"]);
    expect(fileTransferProtocolsForProfile(profile("serial"))).toEqual([]);
  });

  it("honors every protocol enable flag", () => {
    expect(transferProtocolsForProfile(profile("ssh", {
      sftp: false,
      scp: false,
      tftp: false,
      xmodem: false,
      ymodem: false,
      zmodem: false,
    }))).toEqual([]);
  });

  it("uses stable operator-facing protocol labels", () => {
    expect((["sftp", "scp", "tftp", "xmodem", "ymodem", "zmodem"] as TransferProtocol[]).map(transferProtocolLabel)).toEqual([
      "SFTP",
      "SCP",
      "TFTP",
      "XModem",
      "YModem",
      "ZModem",
    ]);
  });

  it("builds constrained device load endpoints for modem uploads", () => {
    expect(isModemTransferProtocol("sftp")).toBe(false);
    expect(isModemTransferProtocol("xmodem")).toBe(true);
    expect(modemLoadCommand("ymodem")).toBe("loady");
    expect(deviceLoadEndpoint("zmodem", " 0x80000000 ", " 115200 ")).toBe(
      "load:loadz?address=0x80000000&baud=115200",
    );
    expect(deviceLoadEndpoint("xmodem", "", "")).toBe("load:loadx");
    expect(isTftpTransferProtocol("tftp")).toBe(true);
    expect(deviceTftpEndpoint({
      address: " 0x81800000 ",
      fileName: " firmware.bin ",
      deviceIp: " 192.168.255.1 ",
      serverIp: "192.168.255.2",
      bindHost: "0.0.0.0",
      bindPort: "69",
      timeoutSeconds: "60",
    })).toBe("load:tftpboot?address=0x81800000&fileName=firmware.bin&deviceIp=192.168.255.1&serverIp=192.168.255.2&bindHost=0.0.0.0&bindPort=69&timeoutSeconds=60");
  });
});

function profile(kind: SessionKind, patch: Partial<SessionProfile["transfer"]> = {}): SessionProfile {
  return {
    kind,
    transfer: {
      sftp: true,
      scp: true,
      tftp: true,
      xmodem: true,
      ymodem: true,
      zmodem: true,
      ...patch,
    },
  } as SessionProfile;
}
