import { t, useLocale, localizeDiagnostic } from "./i18n";
import { useEffect, useMemo, useRef, useState } from "react";
import type { FormEvent, ReactNode } from "react";
import { X } from "lucide-react";
import { invokeBackend } from "./api";
import { deviceLoadEndpoint, deviceTftpEndpoint, isModemTransferProtocol, isTftpTransferProtocol, modemLoadCommand, transferProtocolLabel, transferProtocolsForProfile } from "./transfer-capabilities";
import type { TransferProtocol } from "./transfer-capabilities";
import { COMMON_SERIAL_BAUD_RATES } from "./serial-connection-settings";
import { KeyedRequestGate } from "./keyed-request-gate";
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
  useLocale();
  const protocols = useMemo(() => transferProtocolsForProfile(session.profile), [session.profile]);
  const [protocol, setProtocol] = useState<TransferProtocol | "">(() => protocols[0] ?? "");
  const [source, setSource] = useState("");
  const [destination, setDestination] = useState("");
  const [modemMode, setModemMode] = useState<"device-load" | "path">(() => session.profile.kind === "serial" ? "device-load" : "path");
  const [loadAddress, setLoadAddress] = useState("");
  const [loadBaudRate, setLoadBaudRate] = useState("");
  const [tftpFileName, setTftpFileName] = useState("");
  const [tftpDeviceIp, setTftpDeviceIp] = useState("");
  const [tftpServerIp, setTftpServerIp] = useState("");
  const [tftpBindHost, setTftpBindHost] = useState("");
  // Prefer an unprivileged ephemeral port. Port 69 is commonly occupied by a
  // system TFTP daemon and requires elevated privileges on Unix.
  const [tftpBindPort, setTftpBindPort] = useState("0");
  const [tftpTimeoutSeconds, setTftpTimeoutSeconds] = useState("60");
  const [busy, setBusy] = useState(false);
  const [batchBusy, setBatchBusy] = useState(false);
  const [busyTransferIds, setBusyTransferIds] = useState<Set<string>>(() => new Set());
  const [error, setError] = useState("");
  const startGateRef = useRef(new KeyedRequestGate<"start">());
  const batchOperationGateRef = useRef(new KeyedRequestGate<"batch">());
  const transferOperationGateRef = useRef(new KeyedRequestGate<string>());
  const sessionTransfers = transfers.filter((task) => task.sessionId === session.profile.id);
  const activeTransfers = sessionTransfers.filter((task) => task.status === "queued" || task.status === "running");
  const retryableTransfers = sessionTransfers.filter((task) => task.status === "failed" || task.status === "cancelled");
  const connected = session.runtime.status === "connected";
  const modemProtocol = isModemTransferProtocol(protocol);
  const tftpProtocol = isTftpTransferProtocol(protocol);
  const deviceLoadMode = modemProtocol && modemMode === "device-load";

  useEffect(() => {
    if (!protocol || !protocols.includes(protocol)) {
      setProtocol(protocols[0] ?? "");
    }
  }, [protocol, protocols]);

  useEffect(() => {
    startGateRef.current.invalidateAll();
    batchOperationGateRef.current.invalidateAll();
    transferOperationGateRef.current.invalidateAll();
    setBusy(false);
    setBatchBusy(false);
    setBusyTransferIds(new Set());
    return () => {
      startGateRef.current.invalidateAll();
      batchOperationGateRef.current.invalidateAll();
      transferOperationGateRef.current.invalidateAll();
    };
  }, [session.profile.id]);

  async function submit(event: FormEvent) {
    event.preventDefault();
    setError("");
    if (!protocol) {
      setError(t("no-supported-transfer-protocols-are-enabled-in-this-profile"));
      return;
    }
    if (!connected) {
      setError(t("connect-the-session-before-starting-a-transfer"));
      return;
    }
    const gate = startGateRef.current;
    const token = gate.begin("start");
    if (token === null) return;
    setBusy(true);
    try {
      const transferDestination = deviceLoadMode
        ? deviceLoadEndpoint(protocol, loadAddress, loadBaudRate)
        : tftpProtocol
          ? deviceTftpEndpoint({
              address: loadAddress,
              fileName: tftpFileName,
              deviceIp: tftpDeviceIp,
              serverIp: tftpServerIp,
              bindHost: tftpBindHost,
              bindPort: tftpBindPort,
              timeoutSeconds: tftpTimeoutSeconds,
            })
          : destination;
      const task = await invokeBackend<TransferTask>("start_transfer", {
        request: { sessionId: session.profile.id, protocol, source, destination: transferDestination },
      });
      if (!gate.isCurrent("start", token)) return;
      onTask(task);
      onNotice(`${task.protocol} ${task.status}: ${task.message ?? ""}`);
    } catch (error) {
      if (gate.isCurrent("start", token)) setError(formatTransferError(error));
    } finally {
      if (gate.finish("start", token)) setBusy(false);
    }
  }

  async function retryTransfer(task: TransferTask, batchToken?: number) {
    if (batchToken === undefined && batchOperationGateRef.current.isActive("batch")) return;
    if (batchToken !== undefined && !batchOperationGateRef.current.isCurrent("batch", batchToken)) return;
    const token = beginTransferOperation(task.id);
    if (token === null) return;
    try {
      const retried = await invokeBackend<TransferTask>("retry_transfer", { transferId: task.id });
      if (!transferOperationGateRef.current.isCurrent(task.id, token)) return;
      onTask(retried);
      onNotice(`${retried.protocol} ${retried.status}: ${retried.message ?? ""}`);
    } catch (error) {
      if (transferOperationGateRef.current.isCurrent(task.id, token)) setError(formatTransferError(error));
    } finally {
      finishTransferOperation(task.id, token);
    }
  }

  async function cancelTransfer(task: TransferTask, batchToken?: number) {
    if (batchToken === undefined && batchOperationGateRef.current.isActive("batch")) return;
    if (batchToken !== undefined && !batchOperationGateRef.current.isCurrent("batch", batchToken)) return;
    const token = beginTransferOperation(task.id);
    if (token === null) return;
    try {
      const cancelled = await invokeBackend<TransferTask>("cancel_transfer", { transferId: task.id });
      if (!transferOperationGateRef.current.isCurrent(task.id, token)) return;
      onTask(cancelled);
      onNotice(`${cancelled.protocol} ${cancelled.status}: ${cancelled.message ?? ""}`);
    } catch (error) {
      if (transferOperationGateRef.current.isCurrent(task.id, token)) setError(formatTransferError(error));
    } finally {
      finishTransferOperation(task.id, token);
    }
  }

  function beginTransferOperation(transferId: string): number | null {
    const token = transferOperationGateRef.current.begin(transferId);
    if (token !== null) setBusyTransferIds((current) => new Set(current).add(transferId));
    return token;
  }

  function finishTransferOperation(transferId: string, token: number) {
    if (!transferOperationGateRef.current.finish(transferId, token)) return;
    setBusyTransferIds((current) => {
      const next = new Set(current);
      next.delete(transferId);
      return next;
    });
  }

  async function cancelActiveTransfers() {
    if (activeTransfers.some((task) => transferOperationGateRef.current.isActive(task.id))) return;
    const token = batchOperationGateRef.current.begin("batch");
    if (token === null) return;
    setBatchBusy(true);
    try {
      for (const task of activeTransfers) {
        if (!batchOperationGateRef.current.isCurrent("batch", token)) return;
        await cancelTransfer(task, token);
      }
    } finally {
      if (batchOperationGateRef.current.finish("batch", token)) setBatchBusy(false);
    }
  }

  async function retryFailedTransfers() {
    if (retryableTransfers.some((task) => transferOperationGateRef.current.isActive(task.id))) return;
    const token = batchOperationGateRef.current.begin("batch");
    if (token === null) return;
    setBatchBusy(true);
    try {
      for (const task of retryableTransfers) {
        if (!batchOperationGateRef.current.isCurrent("batch", token)) return;
        await retryTransfer(task, token);
      }
    } finally {
      if (batchOperationGateRef.current.finish("batch", token)) setBatchBusy(false);
    }
  }

  return (
    <div className="dialog-backdrop utility-backdrop" onMouseDown={(event) => event.target === event.currentTarget && onClose()}>
      <form className="wind-dialog utility-dialog transfer-dialog" onSubmit={submit}>
        <header className="dialog-title">
          <span className="app-icon" />
          <strong>{t("transfer-tasks")}</strong>
          <button type="button" onClick={onClose}><X size={20} /></button>
        </header>
        <section className="utility-content">
          <DialogField label={t("session-3")}><input value={session.profile.name} readOnly /></DialogField>
          <DialogField label={t("protocol")}>
            <select value={protocol} disabled={!protocols.length} onChange={(event) => setProtocol(event.target.value as TransferProtocol)}>
              {!protocols.length ? <option value="">{t("no-transfer-protocol-enabled")}</option> : null}
              {protocols.map((option) => <option key={option} value={option}>{transferProtocolLabel(option)}</option>)}
            </select>
          </DialogField>
          {modemProtocol ? (
            <DialogField label={t("receiver")}>
              <div className="transfer-mode-switch" aria-label={t("modem-receiver-mode")}>
                <button type="button" aria-pressed={modemMode === "device-load"} onClick={() => setModemMode("device-load")}>{t("automatic-2", [modemLoadCommand(protocol)])}</button>
                <button type="button" aria-pressed={modemMode === "path"} onClick={() => setModemMode("path")}>{t("path-ready")}</button>
              </div>
            </DialogField>
          ) : null}
          <DialogField label={deviceLoadMode || tftpProtocol ? t("local-file-2") : t("source")}><input value={source} onChange={(event) => setSource(event.target.value)} placeholder={deviceLoadMode || tftpProtocol ? "/local/firmware.bin" : t("local-file-or-remote-remote-file")} /></DialogField>
          {deviceLoadMode ? (
            <>
              <DialogField label={t("load-address")}><input value={loadAddress} onChange={(event) => setLoadAddress(event.target.value)} placeholder={t("optional-e-g-0x80000000")} spellCheck={false} /></DialogField>
              <DialogField label={t("transfer-baud-rate")}>
                <input
                  type="number"
                  min={1}
                  max={4_294_967_295}
                  list="transfer-load-baud-rate-options"
                  value={loadBaudRate}
                  onChange={(event) => setLoadBaudRate(event.target.value)}
                  placeholder={session.profile.kind === "serial" ? t("optional-blank-uses-the-current-baud-rate") : t("only-configurable-for-serial-sessions")}
                  disabled={session.profile.kind !== "serial"}
                />
                <datalist id="transfer-load-baud-rate-options">
                  {COMMON_SERIAL_BAUD_RATES.map((baudRate) => <option key={baudRate} value={baudRate} />)}
                </datalist>
              </DialogField>
            </>
          ) : tftpProtocol ? (
            <>
              <DialogField label={t("device-ip")}><input value={tftpDeviceIp} onChange={(event) => setTftpDeviceIp(event.target.value)} placeholder={t("e-g-192-168-255-1")} spellCheck={false} /></DialogField>
              <DialogField label={t("server-ip")}><input value={tftpServerIp} onChange={(event) => setTftpServerIp(event.target.value)} placeholder={t("optional-inferred-from-the-route-to-the-device")} spellCheck={false} /></DialogField>
              <DialogField label={t("bind-address")}><input value={tftpBindHost} onChange={(event) => setTftpBindHost(event.target.value)} placeholder={t("optional-e-g-0-0-0-0")} spellCheck={false} /></DialogField>
              <DialogField label={t("listen-port")}><input type="number" min={0} max={65_535} value={tftpBindPort} onChange={(event) => setTftpBindPort(event.target.value)} placeholder={t("0-assigns-a-port-automatically-69-may-require-privileges")} /></DialogField>
              <DialogField label={t("load-address")}><input value={loadAddress} onChange={(event) => setLoadAddress(event.target.value)} placeholder={t("optional-default-loadaddr")} spellCheck={false} /></DialogField>
              <DialogField label={t("requested-file-name")}><input value={tftpFileName} onChange={(event) => setTftpFileName(event.target.value)} placeholder={t("optional-defaults-to-the-local-file-name")} spellCheck={false} /></DialogField>
              <DialogField label={t("total-timeout-seconds")}><input type="number" min={5} value={tftpTimeoutSeconds} onChange={(event) => setTftpTimeoutSeconds(event.target.value)} /></DialogField>
            </>
          ) : (
            <DialogField label={t("target-2")}><input value={destination} onChange={(event) => setDestination(event.target.value)} placeholder={modemProtocol ? t("remote-device-path") : t("local-file-or-remote-remote-file")} /></DialogField>
          )}
          <div className="transfer-queue-panel">
            <header>
              <strong>{t("queue")}</strong>
              <div>
                <button type="button" onClick={() => void retryFailedTransfers()} disabled={batchBusy || Boolean(busyTransferIds.size) || !retryableTransfers.length}>{t("retry-failed")}</button>
                <button type="button" onClick={() => void cancelActiveTransfers()} disabled={batchBusy || Boolean(busyTransferIds.size) || !activeTransfers.length}>{t("cancel-unfinished")}</button>
              </div>
            </header>
            <TransferList
              transfers={sessionTransfers}
              dismissedTransferIds={dismissedTransferIds}
              busyTransferIds={busyTransferIds}
              operationsLocked={batchBusy}
              onRetry={retryTransfer}
              onCancel={cancelTransfer}
              onDismiss={onDismissTransfer}
            />
          </div>
          {!connected ? <div className="utility-status">{t("the-session-is-disconnected-only-existing-tasks-can-be")}</div> : null}
          {connected && !protocols.length ? <div className="utility-status">{t("this-profile-has-no-enabled-transfer-methods-for-this")}</div> : null}
          {error ? <div className="utility-error">{localizeDiagnostic(error)}</div> : null}
        </section>
        <footer className="utility-actions">
          <button type="button" onClick={onClose}>{t("cancel")}</button>
          <button type="submit" disabled={busy || !connected || !protocol || !source.trim() || (!deviceLoadMode && !tftpProtocol && !destination.trim()) || (deviceLoadMode && Boolean(loadBaudRate.trim()) && !loadAddress.trim()) || (tftpProtocol && !tftpDeviceIp.trim())}>{busy ? t("running-2") : t("start-2")}</button>
        </footer>
      </form>
    </div>
  );
}

function DialogField({ label, children }: { label: string; children: ReactNode }) {
  useLocale();
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
