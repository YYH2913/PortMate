import type { SessionProfile, TransferTask } from "./types";

export type TransferProtocol = TransferTask["protocol"];
export type ModemTransferProtocol = Extract<TransferProtocol, "xmodem" | "ymodem" | "zmodem">;

const transferProtocolLabels: Record<TransferProtocol, string> = {
  sftp: "SFTP",
  scp: "SCP",
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
  return protocols;
}

export function transferProtocolLabel(protocol: TransferProtocol): string {
  return transferProtocolLabels[protocol];
}

export function isModemTransferProtocol(protocol: TransferProtocol | ""): protocol is ModemTransferProtocol {
  return protocol === "xmodem" || protocol === "ymodem" || protocol === "zmodem";
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
