import { useEffect, useMemo, useState } from "react";
import type { FormEvent, ReactNode } from "react";
import { X } from "lucide-react";
import { invokeBackend } from "./api";
import { deviceLoadEndpoint, isModemTransferProtocol, modemLoadCommand, transferProtocolLabel, transferProtocolsForProfile } from "./transfer-capabilities";
import type { TransferProtocol } from "./transfer-capabilities";
import { COMMON_SERIAL_BAUD_RATES } from "./serial-connection-settings";
import TransferList from "./TransferList";
import type { SessionSummary, TransferTask } from "./types";

export default function TransferDialog({
  session,
  transfers,
  dismissedTransferIds,
  onClose,
  onTask,
  onDismissTransfer,
  onNotice,
}: {
  session: SessionSummary;
  transfers: TransferTask[];
  dismissedTransferIds: ReadonlySet<string>;
  onClose: () => void;
  onTask: (task: TransferTask) => void;
  onDismissTransfer: (transferId: string) => void;
  onNotice: (message: string) => void;
}) {
  const protocols = useMemo(() => transferProtocolsForProfile(session.profile), [session.profile]);
  const [protocol, setProtocol] = useState<TransferProtocol | "">(() => protocols[0] ?? "");
  const [source, setSource] = useState("");
  const [destination, setDestination] = useState("");
  const [modemMode, setModemMode] = useState<"device-load" | "path">(() => session.profile.kind === "serial" ? "device-load" : "path");
  const [loadAddress, setLoadAddress] = useState("");
  const [loadBaudRate, setLoadBaudRate] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const sessionTransfers = transfers.filter((task) => task.sessionId === session.profile.id);
  const runningTransfers = sessionTransfers.filter((task) => task.status === "running");
  const retryableTransfers = sessionTransfers.filter((task) => task.status === "failed" || task.status === "cancelled");
  const connected = session.runtime.status === "connected";
  const modemProtocol = isModemTransferProtocol(protocol);
  const deviceLoadMode = modemProtocol && modemMode === "device-load";

  useEffect(() => {
    if (!protocol || !protocols.includes(protocol)) {
      setProtocol(protocols[0] ?? "");
    }
  }, [protocol, protocols]);

  async function submit(event: FormEvent) {
    event.preventDefault();
    setError("");
    if (!protocol) {
      setError("当前 Profile 未启用可用的传输协议。");
      return;
    }
    if (!connected) {
      setError("连接会话后才能开始传输。");
      return;
    }
    setBusy(true);
    try {
      const transferDestination = deviceLoadMode
        ? deviceLoadEndpoint(protocol, loadAddress, loadBaudRate)
        : destination;
      const task = await invokeBackend<TransferTask>("start_transfer", {
        request: { sessionId: session.profile.id, protocol, source, destination: transferDestination },
      });
      onTask(task);
      onNotice(`${task.protocol} ${task.status}: ${task.message ?? ""}`);
    } catch (error) {
      setError(formatTransferError(error));
    } finally {
      setBusy(false);
    }
  }

  async function retryTransfer(task: TransferTask) {
    try {
      const retried = await invokeBackend<TransferTask>("retry_transfer", { transferId: task.id });
      onTask(retried);
      onNotice(`${retried.protocol} ${retried.status}: ${retried.message ?? ""}`);
    } catch (error) {
      setError(formatTransferError(error));
    }
  }

  async function cancelTransfer(task: TransferTask) {
    try {
      const cancelled = await invokeBackend<TransferTask>("cancel_transfer", { transferId: task.id });
      onTask(cancelled);
      onNotice(`${cancelled.protocol} ${cancelled.status}: ${cancelled.message ?? ""}`);
    } catch (error) {
      setError(formatTransferError(error));
    }
  }

  async function cancelRunningTransfers() {
    for (const task of runningTransfers) await cancelTransfer(task);
  }

  async function retryFailedTransfers() {
    for (const task of retryableTransfers) await retryTransfer(task);
  }

  return (
    <div className="dialog-backdrop utility-backdrop" onMouseDown={(event) => event.target === event.currentTarget && onClose()}>
      <form className="wind-dialog utility-dialog transfer-dialog" onSubmit={submit}>
        <header className="dialog-title">
          <span className="app-icon" />
          <strong>传输任务</strong>
          <button type="button" onClick={onClose}><X size={20} /></button>
        </header>
        <section className="utility-content">
          <DialogField label="会话:"><input value={session.profile.name} readOnly /></DialogField>
          <DialogField label="协议:">
            <select value={protocol} disabled={!protocols.length} onChange={(event) => setProtocol(event.target.value as TransferProtocol)}>
              {!protocols.length ? <option value="">未启用传输协议</option> : null}
              {protocols.map((option) => <option key={option} value={option}>{transferProtocolLabel(option)}</option>)}
            </select>
          </DialogField>
          {modemProtocol ? (
            <DialogField label="接收端:">
              <div className="transfer-mode-switch" aria-label="Modem 接收端模式">
                <button type="button" aria-pressed={modemMode === "device-load"} onClick={() => setModemMode("device-load")}>自动 {modemLoadCommand(protocol)}</button>
                <button type="button" aria-pressed={modemMode === "path"} onClick={() => setModemMode("path")}>路径 / 已就绪</button>
              </div>
            </DialogField>
          ) : null}
          <DialogField label={deviceLoadMode ? "本地文件:" : "来源:"}><input value={source} onChange={(event) => setSource(event.target.value)} placeholder={deviceLoadMode ? "/local/firmware.bin" : "/local/file 或 remote:/remote/file"} /></DialogField>
          {deviceLoadMode ? (
            <>
              <DialogField label="加载地址:"><input value={loadAddress} onChange={(event) => setLoadAddress(event.target.value)} placeholder="可选，例如 0x80000000" spellCheck={false} /></DialogField>
              <DialogField label="传输波特率:">
                <input
                  type="number"
                  min={1}
                  max={4_294_967_295}
                  list="transfer-load-baud-rate-options"
                  value={loadBaudRate}
                  onChange={(event) => setLoadBaudRate(event.target.value)}
                  placeholder={session.profile.kind === "serial" ? "可选，留空使用当前波特率" : "仅串口会话可设置"}
                  disabled={session.profile.kind !== "serial"}
                />
                <datalist id="transfer-load-baud-rate-options">
                  {COMMON_SERIAL_BAUD_RATES.map((baudRate) => <option key={baudRate} value={baudRate} />)}
                </datalist>
              </DialogField>
            </>
          ) : (
            <DialogField label="目标:"><input value={destination} onChange={(event) => setDestination(event.target.value)} placeholder="/local/file 或 remote:/remote/file" /></DialogField>
          )}
          <div className="transfer-queue-panel">
            <header>
              <strong>队列</strong>
              <div>
                <button type="button" onClick={() => void retryFailedTransfers()} disabled={!retryableTransfers.length}>重试失败</button>
                <button type="button" onClick={() => void cancelRunningTransfers()} disabled={!runningTransfers.length}>取消运行中</button>
              </div>
            </header>
            <TransferList
              transfers={sessionTransfers}
              dismissedTransferIds={dismissedTransferIds}
              onRetry={(task) => void retryTransfer(task)}
              onCancel={(task) => void cancelTransfer(task)}
              onDismiss={onDismissTransfer}
            />
          </div>
          {!connected ? <div className="utility-status">当前会话未连接，只能查看和管理已有任务。</div> : null}
          {connected && !protocols.length ? <div className="utility-status">当前 Profile 未启用适用于此协议的传输方式。</div> : null}
          {error ? <div className="utility-error">{error}</div> : null}
        </section>
        <footer className="utility-actions">
          <button type="button" onClick={onClose}>取消</button>
          <button type="submit" disabled={busy || !connected || !protocol || !source.trim() || (!deviceLoadMode && !destination.trim()) || (deviceLoadMode && Boolean(loadBaudRate.trim()) && !loadAddress.trim())}>{busy ? "执行中" : "开始"}</button>
        </footer>
      </form>
    </div>
  );
}

function DialogField({ label, children }: { label: string; children: ReactNode }) {
  return (
    <label className="dialog-field">
      <span>{label}</span>
      {children}
    </label>
  );
}

function formatTransferError(error: unknown) {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  try {
    return JSON.stringify(error);
  } catch {
    return String(error);
  }
}
