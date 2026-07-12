import type { TransferTask } from "./types";

const genericMessages = new Set(["queued", "running", "completed", "cancelled", "cancelling"]);

const statusLabels: Record<TransferTask["status"], string> = {
  queued: "排队中",
  running: "传输中",
  completed: "已完成",
  failed: "失败",
  cancelled: "已取消",
};

export function transferStatusLabel(status: TransferTask["status"]) {
  return statusLabels[status];
}

export function transferDisplayMessage(task: TransferTask) {
  const message = task.message?.trim() ?? "";
  if (message && !genericMessages.has(message.toLowerCase())) return message;
  if (task.status === "failed") return "传输失败，远端未返回详细原因";
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
