import type { SessionProfile, TransferTask } from "./types";

export type TransferProtocol = TransferTask["protocol"];
export type ModemTransferProtocol = Extract<TransferProtocol, "xmodem" | "ymodem" | "zmodem">;

const transferProtocolLabels: Record<TransferProtocol, string> = {
  sftp: "SFTP",
  scp: "SCP",
  tftp: "TFTP",
  xmodem: "XModem",
  ymodem: "YModem",
  zmodem: "ZModem",
};

export function transferProtocolsForProfile(profile: SessionProfile): TransferProtocol[] {
  const protocols: TransferProtocol[] = [];
  const sshLike = profile.kind === "ssh" || profile.kind === "tmux";

  if (sshLike && profile.transfer.sftp) protocols.push("sftp");
  if (sshLike && profile.transfer.scp) protocols.push("scp");
  if (profile.transfer.xmodem) protocols.push("xmodem");
  if (profile.transfer.ymodem) protocols.push("ymodem");
  if (profile.transfer.zmodem) protocols.push("zmodem");
  if (profile.transfer.tftp !== false) protocols.push("tftp");
  return protocols;
}

export function transferProtocolLabel(protocol: TransferProtocol): string {
  return transferProtocolLabels[protocol];
}

export function isModemTransferProtocol(protocol: TransferProtocol | ""): protocol is ModemTransferProtocol {
  return protocol === "xmodem" || protocol === "ymodem" || protocol === "zmodem";
}

export function isTftpTransferProtocol(protocol: TransferProtocol | ""): protocol is "tftp" {
  return protocol === "tftp";
}

export function modemLoadCommand(protocol: ModemTransferProtocol): "loadx" | "loady" | "loadz" {
  if (protocol === "xmodem") return "loadx";
  if (protocol === "ymodem") return "loady";
  return "loadz";
}

export function deviceLoadEndpoint(
  protocol: ModemTransferProtocol,
  address: string,
  baudRate: string,
): string {
  const query = new URLSearchParams();
  const normalizedAddress = address.trim();
  const normalizedBaudRate = baudRate.trim();
  if (normalizedAddress) query.set("address", normalizedAddress);
  if (normalizedBaudRate) query.set("baud", normalizedBaudRate);
  const encoded = query.toString();
  return `load:${modemLoadCommand(protocol)}${encoded ? `?${encoded}` : ""}`;
}

export interface DeviceTftpEndpointOptions {
  address: string;
  fileName: string;
  deviceIp: string;
  serverIp: string;
  bindHost: string;
  bindPort: string;
  timeoutSeconds: string;
}

export function deviceTftpEndpoint(options: DeviceTftpEndpointOptions): string {
  const query = new URLSearchParams();
  for (const [name, value] of Object.entries(options)) {
    const normalized = value.trim();
    if (normalized) query.set(name, normalized);
  }
  return `load:tftpboot?${query.toString()}`;
}
