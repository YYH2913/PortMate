import type { SessionProfile, TransferTask } from "./types";

export type TransferProtocol = TransferTask["protocol"];

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
