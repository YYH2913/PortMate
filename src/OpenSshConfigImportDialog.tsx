import { t, useLocale } from "./i18n";
import SessionConfigImportDialog from "./SessionConfigImportDialog";
import {
  OPENSSH_CONFIG_IMPORT_MAX_SOURCE_CHARS,
  parseOpenSshConfig,
} from "./openssh-config-import";
import type { OpenSshImportCandidate } from "./openssh-config-import";
import type { ReactNode } from "react";
import type { SessionConfigImportSaveResult } from "./SessionConfigImportDialog";
import type { KeyedRequestGate } from "./keyed-request-gate";

export type OpenSshConfigImportSaveResult = SessionConfigImportSaveResult;

const parseConfig = (source: string) => parseOpenSshConfig(source);

export default function OpenSshConfigImportDialog({
  onImport,
  onClose,
  headerAddon,
  operationGate,
  onDraftDirtyChange,
}: {
  onImport: (candidates: OpenSshImportCandidate[]) => Promise<OpenSshConfigImportSaveResult>;
  onClose: () => void;
  headerAddon?: (busy: boolean) => ReactNode;
  operationGate: KeyedRequestGate<"operation">;
  onDraftDirtyChange: (dirty: boolean) => void;
}) {
  useLocale();
  return <SessionConfigImportDialog
    title={t("import-openssh-sessions")}
    sourceLabel="OpenSSH config"
    sourceAriaLabel={t("openssh-configuration")}
    sourcePlaceholder="Host name"
    emptyMessage={t("no-literal-host-entries-available-to-import")}
    maxSourceChars={OPENSSH_CONFIG_IMPORT_MAX_SOURCE_CHARS}
    parse={parseConfig}
    candidateName={(candidate) => candidate.hostAlias}
    candidateTarget={formatEndpoint}
    candidateDetails={(candidate) => [
      candidate.identityFiles.length ? t("key-count", [candidate.identityFiles.length]) : "",
      candidate.jumps.length ? t("jump-count", [candidate.jumps.length]) : "",
      candidate.forwards.length ? t("forward-count", [candidate.forwards.length]) : "",
    ].filter(Boolean).join(" ")}
    headerAddon={headerAddon}
    operationGate={operationGate}
    onDraftDirtyChange={onDraftDirtyChange}
    onImport={onImport}
    onClose={onClose}
  />;
}

function formatEndpoint(candidate: OpenSshImportCandidate) {
  const host = candidate.host.includes(":") ? `[${candidate.host}]` : candidate.host;
  return candidate.username ? `${candidate.username}@${host}:${candidate.port}` : `${host}:${candidate.port}`;
}
