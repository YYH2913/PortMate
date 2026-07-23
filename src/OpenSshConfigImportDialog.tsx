import SessionConfigImportDialog from "./SessionConfigImportDialog";
import {
  OPENSSH_CONFIG_IMPORT_MAX_SOURCE_CHARS,
  parseOpenSshConfig,
} from "./openssh-config-import";
import type { OpenSshImportCandidate } from "./openssh-config-import";
import type { SessionConfigImportSaveResult } from "./SessionConfigImportDialog";

export type OpenSshConfigImportSaveResult = SessionConfigImportSaveResult;

const parseConfig = (source: string) => parseOpenSshConfig(source);

export default function OpenSshConfigImportDialog({
  onImport,
  onClose,
}: {
  onImport: (candidates: OpenSshImportCandidate[]) => Promise<OpenSshConfigImportSaveResult>;
  onClose: () => void;
}) {
  return <SessionConfigImportDialog
    title="导入 OpenSSH 会话"
    sourceLabel="OpenSSH config"
    sourceAriaLabel="OpenSSH 配置内容"
    sourcePlaceholder="Host name"
    emptyMessage="没有可导入的字面 Host 条目"
    maxSourceChars={OPENSSH_CONFIG_IMPORT_MAX_SOURCE_CHARS}
    parse={parseConfig}
    candidateName={(candidate) => candidate.hostAlias}
    candidateTarget={formatEndpoint}
    candidateDetails={(candidate) => [
      candidate.identityFiles.length ? `${candidate.identityFiles.length} 个密钥` : "",
      candidate.jumps.length ? `${candidate.jumps.length} 个跳板` : "",
    ].filter(Boolean).join(" ")}
    onImport={onImport}
    onClose={onClose}
  />;
}

function formatEndpoint(candidate: OpenSshImportCandidate) {
  const host = candidate.host.includes(":") ? `[${candidate.host}]` : candidate.host;
  return candidate.username ? `${candidate.username}@${host}:${candidate.port}` : `${host}:${candidate.port}`;
}
