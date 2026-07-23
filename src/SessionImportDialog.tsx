import { useState } from "react";
import type { ReactNode } from "react";
import OpenSshConfigImportDialog from "./OpenSshConfigImportDialog";
import PuttyConfigImportDialog from "./PuttyConfigImportDialog";
import ShellConfigImportDialog from "./ShellConfigImportDialog";
import type { OpenSshImportCandidate } from "./openssh-config-import";
import type { PuttySessionImportCandidate } from "./putty-session-import";
import type { ShellSessionImportCandidate } from "./shell-session-import";
import type { SessionConfigImportSaveResult } from "./SessionConfigImportDialog";

type SessionImportMode = "openssh" | "putty" | "shell";

const importModes: Array<{ id: SessionImportMode; label: string }> = [
  { id: "openssh", label: "OpenSSH" },
  { id: "putty", label: "PuTTY" },
  { id: "shell", label: "Shell" },
];

export default function SessionImportDialog({
  onImportOpenSsh,
  onImportPutty,
  onImportShell,
  onClose,
}: {
  onImportOpenSsh: (candidates: OpenSshImportCandidate[]) => Promise<SessionConfigImportSaveResult>;
  onImportPutty: (candidates: PuttySessionImportCandidate[]) => Promise<SessionConfigImportSaveResult>;
  onImportShell: (candidates: ShellSessionImportCandidate[]) => Promise<SessionConfigImportSaveResult>;
  onClose: () => void;
}) {
  const [mode, setMode] = useState<SessionImportMode>("openssh");
  const headerAddon = (busy: boolean): ReactNode => (
    <div className="session-import-mode-switch" role="group" aria-label="导入格式">
      {importModes.map((option) => (
        <button
          key={option.id}
          type="button"
          aria-pressed={mode === option.id}
          disabled={busy}
          onClick={() => setMode(option.id)}
        >
          {option.label}
        </button>
      ))}
    </div>
  );

  if (mode === "putty") {
    return <PuttyConfigImportDialog onImport={onImportPutty} onClose={onClose} headerAddon={headerAddon} />;
  }
  if (mode === "shell") {
    return <ShellConfigImportDialog onImport={onImportShell} onClose={onClose} headerAddon={headerAddon} />;
  }
  return <OpenSshConfigImportDialog onImport={onImportOpenSsh} onClose={onClose} headerAddon={headerAddon} />;
}
