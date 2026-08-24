import { useEffect, useRef, useState } from "react";
import type { ReactNode } from "react";
import { KeyedRequestGate } from "./keyed-request-gate";
import OpenSshConfigImportDialog from "./OpenSshConfigImportDialog";
import PuttyConfigImportDialog from "./PuttyConfigImportDialog";
import ShellConfigImportDialog from "./ShellConfigImportDialog";
import PortMateProfileImportDialog from "./PortMateProfileImportDialog";
import type { OpenSshImportCandidate } from "./openssh-config-import";
import type { PuttySessionImportCandidate } from "./putty-session-import";
import type { ShellSessionImportCandidate } from "./shell-session-import";
import type { SessionConfigImportSaveResult } from "./SessionConfigImportDialog";
import type { SessionProfile } from "./types";

type SessionImportMode = "openssh" | "putty" | "shell" | "portmate";

const importModes: Array<{ id: SessionImportMode; label: string }> = [
  { id: "openssh", label: "OpenSSH" },
  { id: "putty", label: "PuTTY" },
  { id: "shell", label: "Shell" },
  { id: "portmate", label: "PortMate Profile" },
];

export default function SessionImportDialog({
  onImportOpenSsh,
  onImportPutty,
  onImportShell,
  onImportPortMate,
  onClose,
}: {
  onImportOpenSsh: (candidates: OpenSshImportCandidate[]) => Promise<SessionConfigImportSaveResult>;
  onImportPutty: (candidates: PuttySessionImportCandidate[]) => Promise<SessionConfigImportSaveResult>;
  onImportShell: (candidates: ShellSessionImportCandidate[]) => Promise<SessionConfigImportSaveResult>;
  onImportPortMate: (profiles: SessionProfile[]) => Promise<SessionConfigImportSaveResult>;
  onClose: () => void;
}) {
  const [mode, setMode] = useState<SessionImportMode>("openssh");
  const operationGate = useRef(new KeyedRequestGate<"operation">());
  const dirtyModes = useRef(new Set<SessionImportMode>());

  useEffect(() => () => operationGate.current.invalidateAll(), []);

  function setDraftDirty(dirty: boolean) {
    if (dirty) dirtyModes.current.add(mode);
    else dirtyModes.current.delete(mode);
  }

  function changeMode(nextMode: SessionImportMode) {
    if (nextMode === mode) return;
    const token = operationGate.current.begin("operation");
    if (token === null) return;
    try {
      if (dirtyModes.current.has(mode)
        && !window.confirm(`当前 ${importModeLabel(mode)} 导入内容尚未完成，切换格式将放弃这些内容。是否继续？`)) return;
      dirtyModes.current.delete(mode);
      setMode(nextMode);
    } finally {
      operationGate.current.finish("operation", token);
    }
  }

  function closeDialog() {
    const token = operationGate.current.begin("operation");
    if (token === null) return;
    try {
      if (dirtyModes.current.has(mode)
        && !window.confirm(`当前 ${importModeLabel(mode)} 导入内容尚未完成，关闭窗口将放弃这些内容。是否继续？`)) return;
      dirtyModes.current.delete(mode);
      onClose();
    } finally {
      operationGate.current.finish("operation", token);
    }
  }

  const headerAddon = (busy: boolean): ReactNode => (
    <div className="session-import-mode-switch" role="group" aria-label="导入格式">
      {importModes.map((option) => (
        <button
          key={option.id}
          type="button"
          aria-pressed={mode === option.id}
          disabled={busy}
          onClick={() => changeMode(option.id)}
        >
          {option.label}
        </button>
      ))}
    </div>
  );

  if (mode === "putty") {
    return <PuttyConfigImportDialog onImport={onImportPutty} onClose={closeDialog} headerAddon={headerAddon} operationGate={operationGate.current} onDraftDirtyChange={setDraftDirty} />;
  }
  if (mode === "shell") {
    return <ShellConfigImportDialog onImport={onImportShell} onClose={closeDialog} headerAddon={headerAddon} operationGate={operationGate.current} onDraftDirtyChange={setDraftDirty} />;
  }
  if (mode === "portmate") {
    return <PortMateProfileImportDialog onImport={onImportPortMate} onClose={closeDialog} headerAddon={headerAddon} operationGate={operationGate.current} onDraftDirtyChange={setDraftDirty} />;
  }
  return <OpenSshConfigImportDialog onImport={onImportOpenSsh} onClose={closeDialog} headerAddon={headerAddon} operationGate={operationGate.current} onDraftDirtyChange={setDraftDirty} />;
}

function importModeLabel(mode: SessionImportMode) {
  return importModes.find((option) => option.id === mode)?.label ?? mode;
}
