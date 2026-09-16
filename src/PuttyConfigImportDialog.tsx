import { t, useLocale } from "./i18n";
import SessionConfigImportDialog from "./SessionConfigImportDialog";
import { PUTTY_SESSION_IMPORT_MAX_SOURCE_CHARS, parsePuttySessions } from "./putty-session-import";
import type { PuttySessionImportCandidate } from "./putty-session-import";
import type { ReactNode } from "react";
import type { SessionConfigImportSaveResult } from "./SessionConfigImportDialog";
import type { KeyedRequestGate } from "./keyed-request-gate";

const parseConfig = (source: string, sourceName: string) => parsePuttySessions(source, sourceName);

export default function PuttyConfigImportDialog({
  onImport,
  onClose,
  headerAddon,
  operationGate,
  onDraftDirtyChange,
}: {
  onImport: (candidates: PuttySessionImportCandidate[]) => Promise<SessionConfigImportSaveResult>;
  onClose: () => void;
  headerAddon?: (busy: boolean) => ReactNode;
  operationGate: KeyedRequestGate<"operation">;
  onDraftDirtyChange: (dirty: boolean) => void;
}) {
  useLocale();
  return <SessionConfigImportDialog
    title={t("import-putty-sessions")}
    sourceLabel="PuTTY session"
    sourceAriaLabel={t("putty-configuration")}
    sourcePlaceholder="HostName=server.example.test"
    emptyMessage={t("no-putty-sessions-available-to-import")}
    maxSourceChars={PUTTY_SESSION_IMPORT_MAX_SOURCE_CHARS}
    parse={parseConfig}
    candidateName={(candidate) => candidate.name}
    candidateTarget={formatTarget}
    candidateDetails={formatDetails}
    headerAddon={headerAddon}
    operationGate={operationGate}
    onDraftDirtyChange={onDraftDirtyChange}
    onImport={onImport}
    onClose={onClose}
  />;
}

function formatTarget(candidate: PuttySessionImportCandidate) {
  if (candidate.kind === "serial") {
    return candidate.serial.baudRate ? `${candidate.serial.port} @ ${candidate.serial.baudRate}` : candidate.serial.port;
  }
  const host = candidate.host.includes(":") ? `[${candidate.host}]` : candidate.host;
  return candidate.username ? `${candidate.username}@${host}:${candidate.port}` : `${host}:${candidate.port}`;
}

function formatDetails(candidate: PuttySessionImportCandidate) {
  const protocol = candidate.kind === "tcp" ? "Raw TCP" : candidate.kind === "serial" ? "Serial" : candidate.kind.toUpperCase();
  const details = [protocol];
  if (candidate.kind !== "serial" && candidate.proxy) details.push(t("proxy"));
  if (candidate.kind === "ssh" && candidate.tryAgent) details.push("Agent");
  if (candidate.terminal) details.push(t("terminal"));
  if (candidate.kind === "ssh" && candidate.forwards?.length) details.push(t("forwards", [candidate.forwards.length]));
  return details.join(" ");
}
