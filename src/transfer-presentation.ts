import { t } from "./i18n";
import type { TransferTask } from "./types";

const genericMessages = new Set(["queued", "running", "completed", "cancelled", "cancelling"]);

const statusLabels: Record<TransferTask["status"], string> = {
  queued: "queued",
  running: "transferring",
  completed: "completed",
  failed: "failed-2",
  cancelled: "cancelled",
};

export function transferStatusLabel(status: TransferTask["status"]) {
  return t(statusLabels[status]);
}

export function transferDisplayMessage(task: TransferTask) {
  const message = task.message?.trim() ?? "";
  if (message && !genericMessages.has(message.toLowerCase())) return message;
  if (task.status === "failed") return t("transfer-failed-the-remote-host-provided-no-details");
  return null;
}

export function transferDiagnosticText(task: TransferTask) {
  const lines = [
    `PortMate transfer ${task.id}`,
    `Status: ${transferStatusLabel(task.status)} (${task.status})`,
    `Protocol: ${task.protocol}`,
    `Source: ${task.source}`,
    `Destination: ${task.destination}`,
    `Progress: ${task.bytesDone} / ${task.bytesTotal || "unknown"} bytes`,
  ];
  if (task.startedAt) lines.push(`Started: ${task.startedAt}`);
  if (task.finishedAt) lines.push(`Finished: ${task.finishedAt}`);
  const message = transferDisplayMessage(task);
  if (message) lines.push(`Message: ${message}`);
  return lines.join("\n");
}
