import SessionConfigImportDialog from "./SessionConfigImportDialog";
import { PUTTY_SESSION_IMPORT_MAX_SOURCE_CHARS, parsePuttySessions } from "./putty-session-import";
import type { PuttySessionImportCandidate } from "./putty-session-import";
import type { SessionConfigImportSaveResult } from "./SessionConfigImportDialog";

const parseConfig = (source: string, sourceName: string) => parsePuttySessions(source, sourceName);

export default function PuttyConfigImportDialog({
  onImport,
  onClose,
}: {
  onImport: (candidates: PuttySessionImportCandidate[]) => Promise<SessionConfigImportSaveResult>;
  onClose: () => void;
}) {
  return <SessionConfigImportDialog
    title="导入 PuTTY 会话"
    sourceLabel="PuTTY session"
    sourceAriaLabel="PuTTY 配置内容"
    sourcePlaceholder="HostName=server.example.test"
    emptyMessage="没有可导入的 PuTTY 会话"
    maxSourceChars={PUTTY_SESSION_IMPORT_MAX_SOURCE_CHARS}
    parse={parseConfig}
    candidateName={(candidate) => candidate.name}
    candidateTarget={formatTarget}
    candidateDetails={formatDetails}
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
  if (candidate.kind !== "serial" && candidate.proxy) details.push("代理");
  if (candidate.kind === "ssh" && candidate.tryAgent) details.push("Agent");
  return details.join(" ");
}
