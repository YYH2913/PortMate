import { t, useLocale } from "./i18n";
import SessionConfigImportDialog from "./SessionConfigImportDialog";
import { SHELL_SESSION_IMPORT_MAX_SOURCE_CHARS, parseShellSessions } from "./shell-session-import";
import type { ShellSessionImportCandidate } from "./shell-session-import";
import type { ReactNode } from "react";
import type { SessionConfigImportSaveResult } from "./SessionConfigImportDialog";
import type { KeyedRequestGate } from "./keyed-request-gate";

const parseConfig = (source: string) => parseShellSessions(source);

export default function ShellConfigImportDialog({
  onImport,
  onClose,
  headerAddon,
  operationGate,
  onDraftDirtyChange,
}: {
  onImport: (candidates: ShellSessionImportCandidate[]) => Promise<SessionConfigImportSaveResult>;
  onClose: () => void;
  headerAddon?: (busy: boolean) => ReactNode;
  operationGate: KeyedRequestGate<"operation">;
  onDraftDirtyChange: (dirty: boolean) => void;
}) {
  useLocale();
  return <SessionConfigImportDialog
    title={t("import-local-shells")}
    sourceLabel="/etc/shells"
    sourceAriaLabel={t("shell-list-content")}
    sourcePlaceholder="/bin/zsh"
    emptyMessage={t("no-local-shells-available-to-import")}
    maxSourceChars={SHELL_SESSION_IMPORT_MAX_SOURCE_CHARS}
    parse={parseConfig}
    candidateName={(candidate) => candidate.name}
    candidateTarget={(candidate) => candidate.program}
    candidateDetails={(candidate) => /[\\/]/.test(candidate.program) ? "" : "PATH"}
    headerAddon={headerAddon}
    operationGate={operationGate}
    onDraftDirtyChange={onDraftDirtyChange}
    onImport={onImport}
    onClose={onClose}
  />;
}
