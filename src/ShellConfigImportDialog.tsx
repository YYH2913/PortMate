import SessionConfigImportDialog from "./SessionConfigImportDialog";
import { SHELL_SESSION_IMPORT_MAX_SOURCE_CHARS, parseShellSessions } from "./shell-session-import";
import type { ShellSessionImportCandidate } from "./shell-session-import";
import type { ReactNode } from "react";
import type { SessionConfigImportSaveResult } from "./SessionConfigImportDialog";

const parseConfig = (source: string) => parseShellSessions(source);

export default function ShellConfigImportDialog({
  onImport,
  onClose,
  headerAddon,
}: {
  onImport: (candidates: ShellSessionImportCandidate[]) => Promise<SessionConfigImportSaveResult>;
  onClose: () => void;
  headerAddon?: (busy: boolean) => ReactNode;
}) {
  return <SessionConfigImportDialog
    title="导入本地 Shell"
    sourceLabel="/etc/shells"
    sourceAriaLabel="Shell 列表内容"
    sourcePlaceholder="/bin/zsh"
    emptyMessage="没有可导入的本地 Shell"
    maxSourceChars={SHELL_SESSION_IMPORT_MAX_SOURCE_CHARS}
    parse={parseConfig}
    candidateName={(candidate) => candidate.name}
    candidateTarget={(candidate) => candidate.program}
    candidateDetails={(candidate) => /[\\/]/.test(candidate.program) ? "" : "PATH"}
    headerAddon={headerAddon}
    onImport={onImport}
    onClose={onClose}
  />;
}
