import { useEffect, useRef, useState } from "react";
import { Archive, Copy, FileText, Package, RefreshCw, Search, Trash2, X } from "lucide-react";
import { invokeBackend } from "./api";
import { formatBytes } from "./display-formatters";
import { KeyedRequestGate } from "./keyed-request-gate";
import { filterLogShards, selectVisibleLogShards, summarizeBundleAttachmentSelection } from "./log-shard-state";
import type {
  ArchiveLogShardsResult,
  DeleteLogShardsResult,
  ExportSessionBundleArchiveResult,
  LogShardInfo,
  LogShardPreview,
  LogShardSearchMatch,
  SearchLogShardsResult,
  SessionSummary,
} from "./types";

export default function LogManagerDialog({
  sessions,
  activeId,
  onClose,
  onNotice,
}: {
  sessions: SessionSummary[];
  activeId: string;
  onClose: () => void;
  onNotice: (message: string) => void;
}) {
  const [shards, setShards] = useState<LogShardInfo[]>([]);
  const [selected, setSelected] = useState<string[]>([]);
  const [preview, setPreview] = useState<LogShardPreview | null>(null);
  const [query, setQuery] = useState("");
  const [format, setFormat] = useState<LogShardInfo["format"] | "all">("all");
  const [shardBusy, setShardBusy] = useState(false);
  const [previewBusy, setPreviewBusy] = useState(false);
  const [error, setError] = useState("");
  const [bundleSessionId, setBundleSessionId] = useState(activeId || sessions[0]?.profile.id || "");
  const [bundleRedacted, setBundleRedacted] = useState(true);
  const [bundleRawLogs, setBundleRawLogs] = useState(false);
  const [bundleAttachments, setBundleAttachments] = useState(false);
  const [bundleBusy, setBundleBusy] = useState(false);
  const [bundleResult, setBundleResult] = useState<ExportSessionBundleArchiveResult | null>(null);
  const [contentQuery, setContentQuery] = useState("");
  const [searchBusy, setSearchBusy] = useState(false);
  const [searchResult, setSearchResult] = useState<SearchLogShardsResult | null>(null);
  const [activeSearchMatch, setActiveSearchMatch] = useState<LogShardSearchMatch | null>(null);
  const [archiveBusy, setArchiveBusy] = useState(false);
  const [archiveResult, setArchiveResult] = useState<ArchiveLogShardsResult | null>(null);
  const requestGate = useRef(new KeyedRequestGate<"shards" | "preview" | "search">());
  const mutationGate = useRef(new KeyedRequestGate<"write">());

  const filtered = filterLogShards(shards, query, format);
  const selectedPaths = new Set(selected);
  const totalBytes = shards.reduce((sum, shard) => sum + shard.size, 0);
  const bundleAttachmentSelection = summarizeBundleAttachmentSelection(shards, selected);
  const busy = shardBusy || previewBusy;
  const operationBusy = busy || archiveBusy || bundleBusy;

  async function refreshShards() {
    const token = requestGate.current.begin("shards");
    if (token === null) return;
    setShardBusy(true);
    setError("");
    try {
      const next = await invokeBackend<LogShardInfo[]>("list_log_shards", {});
      if (!requestGate.current.isCurrent("shards", token)) return;
      setShards(next);
      const paths = new Set(next.map((shard) => shard.path));
      setSelected((current) => current.filter((path) => paths.has(path)));
      setPreview((current) => current && paths.has(current.path) ? current : null);
      setSearchResult(null);
      setActiveSearchMatch(null);
    } catch (error) {
      if (requestGate.current.isCurrent("shards", token)) setError(formatLogError(error));
    } finally {
      if (requestGate.current.finish("shards", token)) setShardBusy(false);
    }
  }

  useEffect(() => {
    void refreshShards();
    return () => {
      requestGate.current.invalidateAll();
      mutationGate.current.invalidateAll();
    };
  }, []);

  useEffect(() => {
    if (!bundleAttachmentSelection.count || !bundleAttachmentSelection.withinLimits) setBundleAttachments(false);
  }, [bundleAttachmentSelection.count, bundleAttachmentSelection.withinLimits]);

  async function openPreview(path: string) {
    requestGate.current.invalidate("preview");
    const token = requestGate.current.begin("preview");
    if (token === null) return;
    setPreviewBusy(true);
    setError("");
    try {
      setActiveSearchMatch(null);
      const next = await invokeBackend<LogShardPreview>("read_log_shard", { path, maxBytes: 64 * 1024 });
      if (requestGate.current.isCurrent("preview", token)) setPreview(next);
    } catch (error) {
      if (requestGate.current.isCurrent("preview", token)) setError(formatLogError(error));
    } finally {
      if (requestGate.current.finish("preview", token)) setPreviewBusy(false);
    }
  }

  async function deleteSelected() {
    if (!selected.length) return;
    const selectedSet = new Set(selected);
    const bytes = shards.filter((shard) => selectedSet.has(shard.path)).reduce((sum, shard) => sum + shard.size, 0);
    const mutationToken = mutationGate.current.begin("write");
    if (mutationToken === null) return;
    if (!window.confirm(`删除 ${selected.length} 个日志分片（${formatBytes(bytes)}）?`)) {
      mutationGate.current.finish("write", mutationToken);
      return;
    }
    const token = requestGate.current.begin("shards");
    if (token === null) {
      mutationGate.current.finish("write", mutationToken);
      return;
    }
    requestGate.current.invalidate("preview");
    requestGate.current.invalidate("search");
    setShardBusy(true);
    setError("");
    try {
      const result = await invokeBackend<DeleteLogShardsResult>("delete_log_shards", { paths: selected });
      if (!isCurrentMutation(mutationToken) || !requestGate.current.isCurrent("shards", token)) return;
      setSelected([]);
      setPreview(null);
      setSearchResult(null);
      setActiveSearchMatch(null);
      onNotice(`已删除 ${result.deleted} 个分片，释放 ${formatBytes(result.bytesDeleted)}`);
      const next = await invokeBackend<LogShardInfo[]>("list_log_shards", {});
      if (isCurrentMutation(mutationToken) && requestGate.current.isCurrent("shards", token)) setShards(next);
    } catch (error) {
      if (isCurrentMutation(mutationToken) && requestGate.current.isCurrent("shards", token)) {
        setError(formatLogError(error));
      }
    } finally {
      const current = isCurrentMutation(mutationToken)
        && requestGate.current.isCurrent("shards", token);
      requestGate.current.finish("shards", token);
      mutationGate.current.finish("write", mutationToken);
      if (current) setShardBusy(false);
    }
  }

  function toggleSelected(path: string) {
    setSelected((current) => current.includes(path) ? current.filter((item) => item !== path) : [...current, path]);
  }

  async function exportBundle() {
    if (!bundleSessionId) return;
    const token = mutationGate.current.begin("write");
    if (token === null) return;
    setBundleBusy(true);
    setBundleResult(null);
    setError("");
    try {
      const result = await invokeBackend<ExportSessionBundleArchiveResult>("export_session_bundle_archive", {
        request: {
          sessionId: bundleSessionId,
          redactSecrets: bundleRedacted,
          includeRawLogs: bundleRawLogs,
          attachmentPaths: bundleAttachments ? selected : [],
        },
      });
      if (!isCurrentMutation(token)) return;
      setBundleResult(result);
      const warning = result.warnings.length ? ` · ${result.warnings.join(" · ")}` : "";
      onNotice(`会话包已导出：${result.path}${warning}`);
    } catch (error) {
      if (isCurrentMutation(token)) setError(formatLogError(error));
    } finally {
      if (mutationGate.current.finish("write", token)) setBundleBusy(false);
    }
  }

  async function archiveSelected() {
    if (!selected.length) return;
    const token = mutationGate.current.begin("write");
    if (token === null) return;
    setArchiveBusy(true);
    setArchiveResult(null);
    setError("");
    try {
      const result = await invokeBackend<ArchiveLogShardsResult>("archive_log_shards", {
        request: { paths: selected },
      });
      if (!isCurrentMutation(token)) return;
      setArchiveResult(result);
      onNotice(`已归档 ${result.shards} 个日志分片：${result.path}`);
    } catch (error) {
      if (isCurrentMutation(token)) setError(formatLogError(error));
    } finally {
      if (mutationGate.current.finish("write", token)) setArchiveBusy(false);
    }
  }

  function isCurrentMutation(token: number) {
    return mutationGate.current.isCurrent("write", token);
  }

  async function searchShardContent() {
    if (!contentQuery.trim()) return;
    const token = requestGate.current.begin("search");
    if (token === null) return;
    setSearchBusy(true);
    setError("");
    try {
      const result = await invokeBackend<SearchLogShardsResult>("search_log_shards", {
        request: { query: contentQuery, paths: selected, limit: 200 },
      });
      if (!requestGate.current.isCurrent("search", token)) return;
      setSearchResult(result);
      setActiveSearchMatch(result.matches[0] ?? null);
      setPreview(null);
    } catch (error) {
      if (requestGate.current.isCurrent("search", token)) setError(formatLogError(error));
    } finally {
      if (requestGate.current.finish("search", token)) setSearchBusy(false);
    }
  }

  function changeContentQuery(next: string) {
    requestGate.current.invalidate("search");
    setSearchBusy(false);
    setContentQuery(next);
    setSearchResult(null);
    setActiveSearchMatch(null);
  }

  function clearContentSearch() {
    requestGate.current.invalidate("search");
    setSearchBusy(false);
    setSearchResult(null);
    setActiveSearchMatch(null);
  }

  return (
    <div className="dialog-backdrop utility-backdrop" onMouseDown={(event) => event.target === event.currentTarget && onClose()}>
      <section className="wind-dialog log-manager-dialog">
        <header className="dialog-title">
          <FileText size={17} />
          <strong>日志管理</strong>
          <button type="button" onClick={onClose}><X size={20} /></button>
        </header>
        <div className="log-manager-content">
          <div className="log-manager-toolbar">
            <strong>{shards.length} 个分片 · {formatBytes(totalBytes)}</strong>
            <input value={query} onChange={(event) => setQuery(event.target.value)} placeholder="筛选路径" />
            <select value={format} onChange={(event) => setFormat(event.target.value as LogShardInfo["format"] | "all")} aria-label="日志格式">
              <option value="all">全部格式</option>
              <option value="raw">Raw</option>
              <option value="txt">Text</option>
              <option value="jsonl">JSONL</option>
            </select>
            <div className="log-manager-toolbar-actions">
              <button type="button" title="刷新日志分片" aria-label="刷新日志分片" onClick={() => void refreshShards()} disabled={busy}><RefreshCw size={15} /></button>
              <button type="button" title="归档选中分片" aria-label="归档选中分片" onClick={() => void archiveSelected()} disabled={operationBusy || !selected.length}><Archive size={15} /></button>
              <button className="danger" type="button" title="删除选中分片" aria-label="删除选中分片" onClick={() => void deleteSelected()} disabled={operationBusy || !selected.length}><Trash2 size={15} /></button>
            </div>
          </div>
          <div className="log-manager-selection">
            <span>{searchResult ? `${searchResult.matches.length} 条命中 · ${searchResult.filesScanned} 文件 · ${formatBytes(searchResult.bytesScanned)}${searchResult.truncated ? " · 已截断" : ""}` : `${filtered.length} 项 · 已选 ${selected.length}`}</span>
            {!searchResult ? <button type="button" onClick={() => setSelected(selectVisibleLogShards(selected, filtered))} disabled={!filtered.length}>全选结果</button> : null}
            {!searchResult ? <button type="button" onClick={() => setSelected([])} disabled={!selected.length}>清除</button> : null}
          </div>
          <div className="log-content-search">
            <Search size={14} />
            <input value={contentQuery} onChange={(event) => changeContentQuery(event.target.value)} onKeyDown={(event) => {
              if (event.key === "Enter") {
                event.preventDefault();
                void searchShardContent();
              }
            }} placeholder="搜索 Text / JSONL 内容" />
            <button type="button" onClick={() => void searchShardContent()} disabled={searchBusy || !contentQuery.trim()}>{searchBusy ? "搜索中" : selected.length ? "搜索选中" : "搜索全部"}</button>
            <button type="button" onClick={clearContentSearch} disabled={!searchResult}>返回分片</button>
          </div>
          <div className="log-bundle-panel">
            {archiveResult ? (
              <div className="log-bundle-result log-archive-result">
                <code title={archiveResult.path}>{archiveResult.path}</code>
                <span>{archiveResult.shards} 分片 · 源 {formatBytes(archiveResult.sourceBytes)} · 包 {formatBytes(archiveResult.size)} · SHA-256 {archiveResult.sha256.slice(0, 16)}...</span>
                <button type="button" title="复制归档信息" aria-label="复制归档信息" onClick={() => void navigator.clipboard?.writeText(`${archiveResult.path}\n${archiveResult.checksumPath}\nSHA-256 ${archiveResult.sha256}`).catch(() => {})}><Copy size={14} /></button>
              </div>
            ) : null}
            <div className="log-bundle-controls">
              <select value={bundleSessionId} onChange={(event) => setBundleSessionId(event.target.value)} aria-label="导出会话">
                {sessions.map((session) => <option key={session.profile.id} value={session.profile.id}>{session.profile.name}</option>)}
              </select>
              <label><input type="checkbox" checked={bundleRedacted} onChange={(event) => {
                setBundleRedacted(event.target.checked);
                if (event.target.checked) {
                  setBundleRawLogs(false);
                  setBundleAttachments(false);
                }
              }} />脱敏</label>
              <label className={bundleRedacted ? "disabled" : ""}><input type="checkbox" checked={bundleRawLogs} disabled={bundleRedacted} onChange={(event) => setBundleRawLogs(event.target.checked)} />Raw 片段</label>
              <label
                className={bundleRedacted || !bundleAttachmentSelection.count || !bundleAttachmentSelection.withinLimits ? "disabled" : ""}
                title={bundleRedacted ? "附件不会自动脱敏" : bundleAttachmentSelection.withinLimits ? "" : "附件最多 32 项，单项不超过 16 MiB，且合计不超过 32 MiB"}
              ><input
                type="checkbox"
                checked={bundleAttachments}
                disabled={bundleRedacted || !bundleAttachmentSelection.count || !bundleAttachmentSelection.withinLimits}
                onChange={(event) => setBundleAttachments(event.target.checked)}
              />附件 {bundleAttachmentSelection.count} · {formatBytes(bundleAttachmentSelection.bytes)}</label>
              <button type="button" onClick={() => void exportBundle()} disabled={operationBusy || !bundleSessionId}><Package size={15} />{bundleBusy ? "导出中" : "导出会话包"}</button>
            </div>
            {bundleResult ? (
              <div className="log-bundle-result">
                <code title={bundleResult.path}>{bundleResult.path}</code>
                <span>{formatBytes(bundleResult.size)} · {bundleResult.files} 文件 · 附件 {bundleResult.attachments} · Raw {bundleResult.rawLogSegments} · {bundleResult.signatureAlgorithm}</span>
                <button type="button" title="复制会话包信息" aria-label="复制会话包信息" onClick={() => void navigator.clipboard?.writeText(`${bundleResult.path}\n${bundleResult.checksumPath}\n${bundleResult.signaturePath}\nSHA-256 ${bundleResult.sha256}\nEd25519 ${bundleResult.signingPublicKey}`).catch(() => {})}><Copy size={14} /></button>
              </div>
            ) : null}
          </div>
          <div className="log-manager-main">
            <div className="log-shard-list">
              {searchResult ? searchResult.matches.map((match) => (
                <button key={`${match.path}-${match.byteOffset}`} className={`log-search-result ${activeSearchMatch?.path === match.path && activeSearchMatch.byteOffset === match.byteOffset ? "active" : ""}`} type="button" onClick={() => {
                  setActiveSearchMatch(match);
                  setPreview(null);
                }}>
                  <strong>{match.path}:{match.line}</strong>
                  <span>{match.text}</span>
                </button>
              )) : filtered.map((shard) => (
                <div key={shard.path} className={`log-shard-row ${preview?.path === shard.path ? "active" : ""}`}>
                  <input type="checkbox" checked={selectedPaths.has(shard.path)} onChange={() => toggleSelected(shard.path)} aria-label={`选择 ${shard.path}`} />
                  <button type="button" onClick={() => void openPreview(shard.path)} title={shard.path}>
                    <strong>{shard.path}</strong>
                    <span>{shard.format.toUpperCase()} · {formatBytes(shard.size)}{shard.modifiedAt ? ` · ${new Date(shard.modifiedAt).toLocaleString()}` : ""}</span>
                  </button>
                </div>
              ))}
              {searchResult && !searchResult.matches.length ? <div className="empty-pane top">没有内容命中</div> : null}
              {!searchResult && !filtered.length ? <div className="empty-pane top">没有日志分片</div> : null}
            </div>
            <div className="log-preview">
              <header>
                <strong>{activeSearchMatch ? `${activeSearchMatch.path}:${activeSearchMatch.line}` : preview?.path ?? "预览"}</strong>
                {activeSearchMatch ? <span>{activeSearchMatch.format.toUpperCase()} · offset {activeSearchMatch.byteOffset}</span> : preview ? <span>{preview.encoding.toUpperCase()} · {formatBytes(preview.bytesRead)}{preview.truncated ? " · 尾部" : ""}</span> : null}
              </header>
              <pre>{activeSearchMatch?.text ?? preview?.content ?? "选择日志分片查看内容"}</pre>
            </div>
          </div>
          {searchResult?.warnings.length ? <div className="utility-status">{searchResult.warnings.join(" · ")}</div> : null}
          {error ? <div className="utility-error">{error}</div> : null}
        </div>
        <footer className="utility-actions">
          <button type="button" onClick={onClose}>关闭</button>
        </footer>
      </section>
    </div>
  );
}

function formatLogError(error: unknown) {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  try {
    return JSON.stringify(error);
  } catch {
    return String(error);
  }
}
