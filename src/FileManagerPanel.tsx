import { useEffect, useRef, useState } from "react";
import type { DragEvent as ReactDragEvent, MouseEvent as ReactMouseEvent } from "react";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import {
  ChevronLeft,
  ChevronRight,
  Copy,
  Download,
  File,
  FilePlus,
  Folder,
  FolderInput,
  FolderPlus,
  Info,
  ListChecks,
  MoreHorizontal,
  Pencil,
  RefreshCw,
  ShieldCheck,
  Trash2,
  Upload,
  X,
} from "lucide-react";
import { invokeBackend, isBackendAvailable } from "./api";
import { formatBytes } from "./display-formatters";
import {
  createFileNavigationHistory,
  currentFileNavigationPath,
  fileNavigationTarget,
  recordFileNavigation,
  restoreFileNavigation,
} from "./file-navigation-state";
import type { FileNavigationHistory } from "./file-navigation-state";
import { updateFileSelection } from "./file-selection";
import { KeyedRequestGate } from "./keyed-request-gate";
import TransferList from "./TransferList";
import type {
  ConnectionConfig,
  ExternalDropResult,
  FileEntry,
  FileProperties,
  SessionProfile,
  SessionSummary,
  TransferTask,
} from "./types";

type NoticeState = { title: string; message: string } | null;

type FilePanelState = {
  path: string;
  entries: FileEntry[];
  selected: FileEntry[];
  busy: boolean;
  error: string;
};

type FilePropertiesDialogState = {
  remote: boolean;
  path: string;
  properties: FileProperties | null;
  busy: boolean;
  error: string;
} | null;

type FileDragState = {
  remote: boolean;
  entries: FileEntry[];
} | null;

type TransferConflictPolicy = "fail" | "overwrite" | "skip" | "rename";
type FileSelectionModifiers = { shiftKey: boolean; ctrlKey: boolean; metaKey: boolean };
type FileLoadNavigation = "record" | "preserve" | "reset" | { type: "restore"; index: number };

type ExternalDropState = {
  remote: boolean;
  taskIds: string[];
  message: string;
  status: "planning" | "queued" | "completed" | "warning";
} | null;

export default function FileManagerPanel({
  active,
  transfers,
  onTransfer,
  onNotice,
}: {
  active?: SessionSummary;
  transfers: TransferTask[];
  onTransfer: (task: TransferTask) => void;
  onNotice: (notice: NoticeState) => void;
}) {
  const [localPanel, setLocalPanel] = useState<FilePanelState>(() => ({ path: defaultLocalPath(), entries: [], selected: [], busy: false, error: "" }));
  const [remotePanel, setRemotePanel] = useState<FilePanelState>(() => ({ path: ".", entries: [], selected: [], busy: false, error: "" }));
  const [localNavigation, setLocalNavigation] = useState<FileNavigationHistory>(() => createFileNavigationHistory(defaultLocalPath()));
  const [remoteNavigation, setRemoteNavigation] = useState<FileNavigationHistory>(() => createFileNavigationHistory("."));
  const [propertiesDialog, setPropertiesDialog] = useState<FilePropertiesDialogState>(null);
  const [draggedFile, setDraggedFile] = useState<FileDragState>(null);
  const [dropTarget, setDropTarget] = useState<boolean | null>(null);
  const [externalDrop, setExternalDrop] = useState<ExternalDropState>(null);
  const [conflictPolicy, setConflictPolicy] = useState<TransferConflictPolicy>("fail");
  const selectionAnchors = useRef<{ local: string; remote: string }>({ local: "", remote: "" });
  const fileLoadEpochs = useRef({ local: 0, remote: 0 });
  const activeFileSessionIdRef = useRef("");
  const filePropertiesGate = useRef(new KeyedRequestGate<"properties">());
  const canRemote = Boolean(active && isSshLikeProfile(active.profile) && active.runtime.status === "connected");
  activeFileSessionIdRef.current = canRemote ? active?.profile.id ?? "" : "";

  useEffect(() => {
    void loadFiles(false, defaultLocalPath(), "reset");
  }, []);

  useEffect(() => {
    if (canRemote) {
      setRemotePanel((current) => ({ ...current, path: ".", entries: [], selected: [], error: "" }));
      setRemoteNavigation(createFileNavigationHistory("."));
      void loadFiles(true, ".", "reset");
    } else {
      fileLoadEpochs.current.remote += 1;
      setRemotePanel((current) => ({ ...current, entries: [], selected: [], error: "" }));
      setRemoteNavigation(createFileNavigationHistory("."));
    }
  }, [canRemote, active?.profile.id]);

  useEffect(() => {
    setDropTarget(null);
    setExternalDrop(null);
    filePropertiesGate.current.invalidate("properties");
    setPropertiesDialog(null);
  }, [active?.profile.id]);

  useEffect(() => {
    if (!isBackendAvailable()) return;
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void getCurrentWebview().onDragDropEvent((event) => {
      const payload = event.payload;
      if (payload.type === "leave") {
        setDropTarget(null);
        return;
      }
      const remote = filePaneAtPhysicalPosition(payload.position.x, payload.position.y);
      if (!active || remote === null || (remote && !canRemote)) {
        setDropTarget(null);
        return;
      }
      if (payload.type === "drop") {
        setDropTarget(null);
        void startExternalDrop(remote, payload.paths);
      } else {
        setDropTarget(remote);
      }
    }).then((stopListening) => {
      if (disposed) stopListening();
      else unlisten = stopListening;
    }).catch(() => {
      // Native file-drop events are unavailable in browser preview.
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [canRemote, active?.profile.id, localPanel.path, remotePanel.path, conflictPolicy]);

  useEffect(() => {
    if (!externalDrop || externalDrop.status !== "queued" || !externalDrop.taskIds.length) return;
    const batchTasks = externalDrop.taskIds.map((taskId) => transfers.find((task) => task.id === taskId));
    if (batchTasks.some((task) => !task)) return;
    if (batchTasks.some((task) => task?.status === "queued" || task?.status === "running")) return;
    const failed = batchTasks.filter((task) => task?.status === "failed" || task?.status === "cancelled").length;
    const message = failed
      ? `${batchTasks.length - failed}/${batchTasks.length} 个文件完成，${failed} 个失败或取消`
      : `${batchTasks.length} 个文件传输完成`;
    setExternalDrop((current) => current ? { ...current, message, status: failed ? "warning" : "completed" } : null);
    void loadFiles(
      externalDrop.remote,
      externalDrop.remote ? remotePanel.path : localPanel.path,
      "preserve",
    );
  }, [externalDrop, transfers]);

  function updatePanel(remote: boolean, patch: Partial<FilePanelState>) {
    const setter = remote ? setRemotePanel : setLocalPanel;
    setter((current) => ({ ...current, ...patch }));
  }

  function updateNavigation(remote: boolean, path: string, navigation: FileLoadNavigation) {
    const setter = remote ? setRemoteNavigation : setLocalNavigation;
    setter((current) => {
      if (navigation === "preserve") return current;
      if (navigation === "reset") return createFileNavigationHistory(path);
      if (navigation === "record") return recordFileNavigation(current, path);
      return restoreFileNavigation(current, navigation.index);
    });
  }

  function selectFileEntry(remote: boolean, entry: FileEntry, event: FileSelectionModifiers) {
    const setter = remote ? setRemotePanel : setLocalPanel;
    const anchorKey = remote ? "remote" : "local";
    setter((current) => {
      const result = updateFileSelection(
        current.entries,
        current.selected,
        entry,
        selectionAnchors.current[anchorKey],
        event,
      );
      selectionAnchors.current[anchorKey] = result.anchorPath;
      return { ...current, selected: result.selected };
    });
  }

  function selectAllFileEntries(remote: boolean) {
    const setter = remote ? setRemotePanel : setLocalPanel;
    setter((current) => ({
      ...current,
      selected: current.selected.length === current.entries.length ? [] : [...current.entries],
    }));
  }

  async function loadFiles(
    remote: boolean,
    nextPath = remote ? remotePanel.path : localPanel.path,
    navigation: FileLoadNavigation = "record",
  ) {
    const loadKey = remote ? "remote" : "local";
    const epoch = fileLoadEpochs.current[loadKey] + 1;
    fileLoadEpochs.current[loadKey] = epoch;
    const sessionId = remote ? active?.profile.id ?? "" : "";
    if (remote && (!canRemote || !sessionId)) return;
    updatePanel(remote, { busy: true, error: "" });
    try {
      const nextEntries = await invokeBackend<FileEntry[]>("list_files", { request: { sessionId: sessionId || null, path: nextPath, remote } });
      if (fileLoadEpochs.current[loadKey] !== epoch
        || (remote && activeFileSessionIdRef.current !== sessionId)) return;
      updatePanel(remote, { entries: nextEntries, path: nextPath, selected: [] });
      updateNavigation(remote, nextPath, navigation);
      selectionAnchors.current[remote ? "remote" : "local"] = "";
    } catch (error) {
      if (fileLoadEpochs.current[loadKey] !== epoch
        || (remote && activeFileSessionIdRef.current !== sessionId)) return;
      updatePanel(remote, { entries: [], error: formatError(error) });
    } finally {
      if (fileLoadEpochs.current[loadKey] === epoch
        && (!remote || activeFileSessionIdRef.current === sessionId)) {
        updatePanel(remote, { busy: false });
      }
    }
  }

  function navigateHistory(remote: boolean, offset: -1 | 1) {
    const history = remote ? remoteNavigation : localNavigation;
    const target = fileNavigationTarget(history, offset);
    if (!target) return;
    void loadFiles(remote, target.path, { type: "restore", index: target.index });
  }

  async function createDir(remote: boolean) {
    const panel = remote ? remotePanel : localPanel;
    const name = window.prompt("目录名");
    if (!name?.trim()) return;
    const nextPath = joinFilePath(panel.path, name.trim(), remote);
    try {
      await invokeBackend("create_directory", { request: { sessionId: active?.profile.id ?? null, path: nextPath, remote } });
      await loadFiles(remote, panel.path, "preserve");
    } catch (error) {
      updatePanel(remote, { error: formatError(error) });
    }
  }

  async function createFile(remote: boolean) {
    const panel = remote ? remotePanel : localPanel;
    const name = window.prompt("文件名");
    if (!name?.trim()) return;
    const nextPath = joinFilePath(panel.path, name.trim(), remote);
    try {
      await invokeBackend("create_file", { request: { sessionId: active?.profile.id ?? null, path: nextPath, remote } });
      await loadFiles(remote, panel.path, "preserve");
    } catch (error) {
      updatePanel(remote, { error: formatError(error) });
    }
  }

  async function deleteSelected(remote: boolean) {
    const panel = remote ? remotePanel : localPanel;
    if (!panel.selected.length) return;
    if (!window.confirm(`删除选中的 ${panel.selected.length} 项?`)) return;
    try {
      await invokeBackend("delete_paths", {
        request: {
          sessionId: active?.profile.id ?? null,
          paths: panel.selected.map((entry) => entry.path),
          remote,
        },
      });
      await loadFiles(remote, panel.path, "preserve");
    } catch (error) {
      updatePanel(remote, { error: formatError(error) });
    }
  }

  async function renameSelected(remote: boolean) {
    const panel = remote ? remotePanel : localPanel;
    const selected = panel.selected[0];
    if (panel.selected.length !== 1 || !selected) return;
    const nextName = window.prompt("新名称", selected.name);
    if (!nextName?.trim()) return;
    const nextPath = joinFilePath(parentPath(selected.path, remote), nextName.trim(), remote);
    try {
      await invokeBackend("rename_path", { request: { sessionId: active?.profile.id ?? null, oldPath: selected.path, newPath: nextPath, remote } });
      await loadFiles(remote, panel.path, "preserve");
    } catch (error) {
      updatePanel(remote, { error: formatError(error) });
    }
  }

  async function moveSelected(remote: boolean) {
    const panel = remote ? remotePanel : localPanel;
    if (!panel.selected.length) return;
    const suggestedDestination = parentPath(panel.path, remote);
    const destination = window.prompt(
      "移动到目录",
      suggestedDestination === "/" || suggestedDestination === "." ? "" : suggestedDestination,
    );
    if (!destination?.trim()) return;
    try {
      await invokeBackend("move_paths", {
        request: {
          sessionId: active?.profile.id ?? null,
          paths: panel.selected.map((entry) => entry.path),
          destination: destination.trim(),
          remote,
        },
      });
      await loadFiles(remote, panel.path, "preserve");
    } catch (error) {
      updatePanel(remote, { error: formatError(error) });
    }
  }

  async function copySelectedPaths(remote: boolean) {
    const panel = remote ? remotePanel : localPanel;
    if (!panel.selected.length) return;
    try {
      if (!navigator.clipboard?.writeText) {
        throw new Error("当前环境不支持写入剪贴板。");
      }
      await navigator.clipboard.writeText(panel.selected.map((entry) => entry.path).join("\n"));
    } catch (error) {
      updatePanel(remote, { error: formatError(error) });
    }
  }

  async function chmodSelected(remote: boolean) {
    const panel = remote ? remotePanel : localPanel;
    const selected = panel.selected[0];
    if (panel.selected.length !== 1 || !selected) return;
    const modeText = window.prompt("八进制权限", "0644");
    if (!modeText?.trim()) return;
    const mode = Number.parseInt(modeText.replace(/^0o/i, ""), 8);
    if (!Number.isFinite(mode)) return;
    try {
      await invokeBackend("chmod_path", { request: { sessionId: active?.profile.id ?? null, path: selected.path, mode, remote } });
      await loadFiles(remote, panel.path, "preserve");
    } catch (error) {
      updatePanel(remote, { error: formatError(error) });
    }
  }

  async function showProperties(remote: boolean) {
    const panel = remote ? remotePanel : localPanel;
    const selected = panel.selected[0];
    if (panel.selected.length !== 1 || !selected) return;
    const gate = filePropertiesGate.current;
    gate.invalidate("properties");
    const token = gate.begin("properties")!;
    const nextState: NonNullable<FilePropertiesDialogState> = { remote, path: selected.path, properties: null, busy: true, error: "" };
    setPropertiesDialog(nextState);
    try {
      const properties = await invokeBackend<FileProperties>("file_properties", { request: { sessionId: active?.profile.id ?? null, path: selected.path, remote } });
      if (!gate.isCurrent("properties", token)) return;
      setPropertiesDialog({ ...nextState, properties, busy: false });
    } catch (error) {
      if (!gate.isCurrent("properties", token)) return;
      setPropertiesDialog({ ...nextState, busy: false, error: formatError(error) });
    } finally {
      gate.finish("properties", token);
    }
  }

  function closePropertiesDialog() {
    filePropertiesGate.current.invalidate("properties");
    setPropertiesDialog(null);
  }

  async function transferBetween(upload: boolean) {
    if (!active || !canRemote) return;
    const selected = upload ? localPanel.selected : remotePanel.selected;
    if (!selected.length) return;
    await queueFileBatch(
      !upload,
      selected,
      upload,
      upload ? remotePanel.path : localPanel.path,
      upload ? "批量上传" : "批量下载",
    );
  }

  async function queueFileBatch(
    sourceRemote: boolean,
    entries: FileEntry[],
    destinationRemote: boolean,
    destination: string,
    title: string,
  ) {
    if (!active || !canRemote || !entries.length) return;
    setExternalDrop({
      remote: destinationRemote,
      taskIds: [],
      message: `正在规划 ${entries.length} 个选中项`,
      status: "planning",
    });
    updatePanel(destinationRemote, { busy: true, error: "" });
    try {
      const result = await invokeBackend<ExternalDropResult>("start_file_batch", {
        request: {
          sessionId: active.profile.id,
          paths: entries.map((entry) => entry.path),
          sourceRemote,
          destination,
          destinationRemote,
          conflictPolicy,
        },
      });
      result.tasks.forEach(onTransfer);
      const parts = [
        `${result.tasks.length} 个文件`,
        formatBytes(result.totalBytes),
        `${result.directoriesPrepared} 个新目录`,
      ];
      if (result.skipped.length) parts.push(`跳过 ${result.skipped.length} 项`);
      const message = parts.join(" · ");
      setExternalDrop({
        remote: destinationRemote,
        taskIds: result.tasks.map((task) => task.id),
        message,
        status: result.tasks.length ? "queued" : result.skipped.length ? "warning" : "completed",
      });
      onNotice({ title, message });
      if (!result.tasks.length) {
        await loadFiles(destinationRemote, destination, "preserve");
      }
    } catch (error) {
      const message = formatError(error);
      setExternalDrop(null);
      updatePanel(destinationRemote, { error: message });
      onNotice({ title: `${title}失败`, message });
    } finally {
      updatePanel(destinationRemote, { busy: false });
    }
  }

  function startFileDrag(remote: boolean, entry: FileEntry, event: ReactDragEvent<HTMLElement>) {
    if (!canRemote) return;
    const panel = remote ? remotePanel : localPanel;
    const entries = panel.selected.some((item) => item.path === entry.path) ? panel.selected : [entry];
    setDraggedFile({ remote, entries });
    event.dataTransfer.effectAllowed = "copy";
    event.dataTransfer.setData("application/x-portmate-file", JSON.stringify({ remote, paths: entries.map((item) => item.path) }));
  }

  function handleDragOver(remote: boolean, event: ReactDragEvent<HTMLElement>) {
    if (!canRemote || !draggedFile || draggedFile.remote === remote) return;
    event.preventDefault();
    event.dataTransfer.dropEffect = "copy";
    setDropTarget(remote);
  }

  async function dropFile(remote: boolean, event: ReactDragEvent<HTMLElement>) {
    event.preventDefault();
    const dropped = draggedFile;
    setDropTarget(null);
    setDraggedFile(null);
    if (!active || !canRemote || !dropped || dropped.remote === remote) return;
    const targetPanel = remote ? remotePanel : localPanel;
    await queueFileBatch(dropped.remote, dropped.entries, remote, targetPanel.path, "拖拽传输");
  }

  async function startExternalDrop(remote: boolean, paths: string[]) {
    if (!active || (remote && !canRemote) || !paths.length) return;
    const panel = remote ? remotePanel : localPanel;
    setExternalDrop({
      remote,
      taskIds: [],
      message: `正在分析 ${paths.length} 个拖放路径`,
      status: "planning",
    });
    updatePanel(remote, { busy: true, error: "" });
    try {
      const result = await invokeBackend<ExternalDropResult>("start_external_drop", {
        request: {
          sessionId: active.profile.id,
          paths,
          destination: panel.path,
          remote,
          conflictPolicy,
        },
      });
      result.tasks.forEach(onTransfer);
      const parts = [
        `${result.tasks.length} 个文件`,
        formatBytes(result.totalBytes),
        `${result.directoriesPrepared} 个目录`,
      ];
      if (result.skipped.length) parts.push(`跳过 ${result.skipped.length} 项`);
      const message = parts.join(" · ");
      setExternalDrop({
        remote,
        taskIds: result.tasks.map((task) => task.id),
        message,
        status: result.tasks.length ? "queued" : result.skipped.length ? "warning" : "completed",
      });
      onNotice({ title: "外部拖放已处理", message });
      if (!result.tasks.length) {
        await loadFiles(remote, panel.path, "preserve");
      }
    } catch (error) {
      const message = formatError(error);
      setExternalDrop(null);
      updatePanel(remote, { error: message });
      onNotice({ title: "外部拖放失败", message });
    } finally {
      updatePanel(remote, { busy: false });
    }
  }

  async function startPromptTransfer(remote: boolean) {
    if (!active) return;
    const panel = remote ? remotePanel : localPanel;
    const selected = panel.selected[0];
    if (!selected || selected.isDir) return;
    if (remote) {
      const destination = window.prompt("下载到本地路径", selected.name);
      if (!destination) return;
      const task = await invokeBackend<TransferTask>("start_transfer", {
        request: { sessionId: active.profile.id, protocol: "sftp", source: `remote:${selected.path}`, destination },
      });
      onTransfer(task);
      onNotice({ title: "传输任务", message: `${task.protocol} ${task.status}: ${task.message ?? ""}` });
      return;
    }
    const destination = window.prompt("上传到远端路径", `/tmp/${selected.name}`);
    if (!destination) return;
    const task = await invokeBackend<TransferTask>("start_transfer", {
      request: { sessionId: active.profile.id, protocol: "sftp", source: selected.path, destination: `remote:${destination}` },
    });
    onTransfer(task);
    onNotice({ title: "传输任务", message: `${task.protocol} ${task.status}: ${task.message ?? ""}` });
  }

  async function retryTransfer(task: TransferTask) {
    try {
      const retried = await invokeBackend<TransferTask>("retry_transfer", { transferId: task.id });
      onTransfer(retried);
      onNotice({ title: "重试传输", message: `${retried.protocol} ${retried.status}: ${retried.message ?? ""}` });
    } catch (error) {
      onNotice({ title: "重试传输失败", message: formatError(error) });
    }
  }

  async function cancelTransfer(task: TransferTask) {
    try {
      const cancelled = await invokeBackend<TransferTask>("cancel_transfer", { transferId: task.id });
      onTransfer(cancelled);
      onNotice({ title: "取消传输", message: `${cancelled.protocol} ${cancelled.status}: ${cancelled.message ?? ""}` });
    } catch (error) {
      onNotice({ title: "取消传输失败", message: formatError(error) });
    }
  }

  return (
    <div className={canRemote ? "file-manager dual" : "file-manager"}>
      <div className="file-panels">
        <FileBrowserPane
          title="本地"
          remote={false}
          panel={localPanel}
          canTransfer={canRemote}
          transferLabel="上传"
          onPathChange={(path) => setLocalPanel((current) => ({ ...current, path }))}
          canGoBack={fileNavigationTarget(localNavigation, -1) !== null}
          canGoForward={fileNavigationTarget(localNavigation, 1) !== null}
          onGoBack={() => navigateHistory(false, -1)}
          onGoForward={() => navigateHistory(false, 1)}
          onNavigate={(path) => void loadFiles(false, path)}
          onRefresh={() => void loadFiles(false, currentFileNavigationPath(localNavigation) ?? localPanel.path, "preserve")}
          conflictPolicy={conflictPolicy}
          onConflictPolicyChange={setConflictPolicy}
          onSelect={(entry, event) => selectFileEntry(false, entry, event)}
          onSelectAll={() => selectAllFileEntries(false)}
          dropActive={dropTarget === false}
          dropStatus={externalDrop?.remote === false ? externalDrop : null}
          onDragStart={(entry, event) => startFileDrag(false, entry, event)}
          onDragEnd={() => {
            setDraggedFile(null);
            setDropTarget(null);
          }}
          onDragOver={(event) => handleDragOver(false, event)}
          onDragLeave={() => setDropTarget((current) => (current === false ? null : current))}
          onDrop={(event) => void dropFile(false, event)}
          onCreateDir={() => void createDir(false)}
          onCreateFile={() => void createFile(false)}
          onDelete={() => void deleteSelected(false)}
          onRename={() => void renameSelected(false)}
          onMove={() => void moveSelected(false)}
          onCopyPaths={() => void copySelectedPaths(false)}
          onChmod={() => void chmodSelected(false)}
          onProperties={() => void showProperties(false)}
          onTransfer={() => void (canRemote ? transferBetween(true) : startPromptTransfer(false))}
        />
        {canRemote ? (
          <FileBrowserPane
            title="远端"
            remote
            panel={remotePanel}
            canTransfer={canRemote}
            transferLabel="下载"
            onPathChange={(path) => setRemotePanel((current) => ({ ...current, path }))}
            canGoBack={fileNavigationTarget(remoteNavigation, -1) !== null}
            canGoForward={fileNavigationTarget(remoteNavigation, 1) !== null}
            onGoBack={() => navigateHistory(true, -1)}
            onGoForward={() => navigateHistory(true, 1)}
            onNavigate={(path) => void loadFiles(true, path)}
            onRefresh={() => void loadFiles(true, currentFileNavigationPath(remoteNavigation) ?? remotePanel.path, "preserve")}
            conflictPolicy={conflictPolicy}
            onConflictPolicyChange={setConflictPolicy}
            onSelect={(entry, event) => selectFileEntry(true, entry, event)}
            onSelectAll={() => selectAllFileEntries(true)}
            dropActive={dropTarget === true}
            dropStatus={externalDrop?.remote === true ? externalDrop : null}
            onDragStart={(entry, event) => startFileDrag(true, entry, event)}
            onDragEnd={() => {
              setDraggedFile(null);
              setDropTarget(null);
            }}
            onDragOver={(event) => handleDragOver(true, event)}
            onDragLeave={() => setDropTarget((current) => (current === true ? null : current))}
            onDrop={(event) => void dropFile(true, event)}
            onCreateDir={() => void createDir(true)}
            onCreateFile={() => void createFile(true)}
            onDelete={() => void deleteSelected(true)}
            onRename={() => void renameSelected(true)}
            onMove={() => void moveSelected(true)}
            onCopyPaths={() => void copySelectedPaths(true)}
            onChmod={() => void chmodSelected(true)}
            onProperties={() => void showProperties(true)}
            onTransfer={() => void transferBetween(false)}
          />
        ) : null}
      </div>
      <TransferList transfers={transfers.slice(-3)} onRetry={(task) => void retryTransfer(task)} onCancel={(task) => void cancelTransfer(task)} />
      {propertiesDialog ? <FilePropertiesDialog state={propertiesDialog} onClose={closePropertiesDialog} /> : null}
    </div>
  );
}

function FileBrowserPane({
  title,
  remote,
  panel,
  canTransfer,
  transferLabel,
  canGoBack,
  canGoForward,
  dropActive,
  dropStatus,
  conflictPolicy,
  onPathChange,
  onGoBack,
  onGoForward,
  onNavigate,
  onRefresh,
  onSelect,
  onSelectAll,
  onConflictPolicyChange,
  onDragStart,
  onDragEnd,
  onDragOver,
  onDragLeave,
  onDrop,
  onCreateDir,
  onCreateFile,
  onDelete,
  onRename,
  onMove,
  onCopyPaths,
  onChmod,
  onProperties,
  onTransfer,
}: {
  title: string;
  remote: boolean;
  panel: FilePanelState;
  canTransfer: boolean;
  transferLabel: string;
  canGoBack: boolean;
  canGoForward: boolean;
  dropActive: boolean;
  dropStatus: ExternalDropState;
  conflictPolicy: TransferConflictPolicy;
  onPathChange: (path: string) => void;
  onGoBack: () => void;
  onGoForward: () => void;
  onNavigate: (path: string) => void;
  onRefresh: () => void;
  onSelect: (entry: FileEntry, event: FileSelectionModifiers) => void;
  onSelectAll: () => void;
  onConflictPolicyChange: (policy: TransferConflictPolicy) => void;
  onDragStart: (entry: FileEntry, event: ReactDragEvent<HTMLElement>) => void;
  onDragEnd: () => void;
  onDragOver: (event: ReactDragEvent<HTMLElement>) => void;
  onDragLeave: () => void;
  onDrop: (event: ReactDragEvent<HTMLElement>) => void;
  onCreateDir: () => void;
  onCreateFile: () => void;
  onDelete: () => void;
  onRename: () => void;
  onMove: () => void;
  onCopyPaths: () => void;
  onChmod: () => void;
  onProperties: () => void;
  onTransfer: () => void;
}) {
  function closeOverflowAndRun(event: ReactMouseEvent<HTMLButtonElement>, action: () => void) {
    event.currentTarget.closest("details")?.removeAttribute("open");
    action();
  }

  return (
    <section
      className={dropActive ? "file-browser-pane drop-active" : "file-browser-pane"}
      data-file-pane={remote ? "remote" : "local"}
      onDragOver={onDragOver}
      onDragLeave={onDragLeave}
      onDrop={onDrop}
    >
      <div className="file-toolbar">
        <strong>{title}</strong>
        <input aria-label={`${title}路径`} value={panel.path} onChange={(event) => onPathChange(event.target.value)} onKeyDown={(event) => {
          if (event.key === "Enter") {
            onNavigate(panel.path);
          }
        }} />
        <button type="button" title={`${title}后退`} aria-label={`${title}后退`} onClick={onGoBack} disabled={!canGoBack || panel.busy}><ChevronLeft size={13} /></button>
        <button type="button" title={`${title}前进`} aria-label={`${title}前进`} onClick={onGoForward} disabled={!canGoForward || panel.busy}><ChevronRight size={13} /></button>
        <button type="button" title={`刷新${title}目录`} aria-label={`刷新${title}目录`} onClick={onRefresh} disabled={panel.busy}><RefreshCw size={13} /></button>
      </div>
      <div className="file-actions">
        <button type="button" title={panel.selected.length === panel.entries.length && panel.entries.length ? "清除选择" : "全选"} aria-label={panel.selected.length === panel.entries.length && panel.entries.length ? "清除选择" : "全选"} onClick={onSelectAll}><ListChecks size={13} /></button>
        <button type="button" title="新建文件夹" aria-label="新建文件夹" onClick={onCreateDir}><FolderPlus size={13} /></button>
        <button type="button" title="新建文件" aria-label="新建文件" onClick={onCreateFile}><FilePlus size={13} /></button>
        <button type="button" title="删除" aria-label="删除" onClick={onDelete} disabled={!panel.selected.length}><Trash2 size={13} /></button>
        <details className="file-action-overflow">
          <summary title="更多文件操作" aria-label="更多文件操作"><MoreHorizontal size={13} /></summary>
          <div className="file-action-overflow-menu">
            <button type="button" title="复制路径" aria-label="复制路径" onClick={(event) => closeOverflowAndRun(event, onCopyPaths)} disabled={!panel.selected.length}><Copy size={13} /><span>复制路径</span></button>
            <button type="button" title="移动到..." aria-label="移动到..." onClick={(event) => closeOverflowAndRun(event, onMove)} disabled={!panel.selected.length}><FolderInput size={13} /><span>移动到...</span></button>
            <button type="button" title="重命名" aria-label="重命名" onClick={(event) => closeOverflowAndRun(event, onRename)} disabled={panel.selected.length !== 1}><Pencil size={13} /><span>重命名</span></button>
            <button type="button" title="修改权限" aria-label="修改权限" onClick={(event) => closeOverflowAndRun(event, onChmod)} disabled={panel.selected.length !== 1}><ShieldCheck size={13} /><span>修改权限</span></button>
            <button type="button" title="文件属性" aria-label="文件属性" onClick={(event) => closeOverflowAndRun(event, onProperties)} disabled={panel.selected.length !== 1}><Info size={13} /><span>文件属性</span></button>
          </div>
        </details>
        <select value={conflictPolicy} onChange={(event) => onConflictPolicyChange(event.target.value as TransferConflictPolicy)} aria-label="文件冲突策略" title="文件冲突策略">
          <option value="fail">停止</option>
          <option value="overwrite">覆盖</option>
          <option value="skip">跳过</option>
          <option value="rename">重命名</option>
        </select>
        <button type="button" title={transferLabel} aria-label={`${transferLabel}${panel.selected.length > 1 ? ` ${panel.selected.length} 项` : ""}`} onClick={onTransfer} disabled={!panel.selected.length || !canTransfer}>
          {remote ? <Download size={13} /> : <Upload size={13} />}
        </button>
      </div>
      {panel.error ? (
        <div className="file-error">{panel.error}</div>
      ) : dropStatus ? (
        <div className={`file-pane-status ${dropStatus.status}`}>{dropStatus.message}</div>
      ) : null}
      <div className="file-list" role="listbox" aria-multiselectable="true">
        <button className="file-row up" onClick={() => onNavigate(parentPath(panel.path, remote))}>
          <span className="file-row-check" />
          <Folder size={13} />
          <span>..</span>
          <small />
        </button>
        {panel.entries.map((entry) => (
          <div
            key={entry.path}
            className={panel.selected.some((item) => item.path === entry.path) ? "file-row active" : "file-row"}
            role="option"
            aria-selected={panel.selected.some((item) => item.path === entry.path)}
            tabIndex={0}
            draggable={canTransfer}
            onDragStart={(event) => onDragStart(entry, event)}
            onDragEnd={onDragEnd}
            onClick={(event) => onSelect(entry, event)}
            onKeyDown={(event) => {
              if (event.key === "Enter" || event.key === " ") {
                event.preventDefault();
                onSelect(entry, event);
              }
            }}
            onDoubleClick={() => {
              if (entry.isDir) {
                onNavigate(entry.path);
              }
            }}
          >
            <input type="checkbox" tabIndex={-1} readOnly checked={panel.selected.some((item) => item.path === entry.path)} aria-label={`选择 ${entry.name}`} />
            {entry.isDir ? <Folder size={13} /> : <File size={13} />}
            <span>{entry.name}</span>
            <small>{entry.isDir ? "dir" : formatBytes(entry.size)}</small>
          </div>
        ))}
      </div>
    </section>
  );
}

function FilePropertiesDialog({ state, onClose }: { state: NonNullable<FilePropertiesDialogState>; onClose: () => void }) {
  const properties = state.properties;
  return (
    <div className="dialog-backdrop utility-backdrop" onMouseDown={(event) => event.target === event.currentTarget && onClose()}>
      <div className="wind-dialog file-properties-dialog">
        <header className="dialog-title">
          <span>文件属性</span>
          <button onClick={onClose}><X size={20} /></button>
        </header>
        <div className="file-properties-content">
          {state.busy ? <div className="empty-pane top">读取中...</div> : null}
          {state.error ? <div className="file-error">{state.error}</div> : null}
          {properties ? (
            <dl className="property-grid">
              <dt>名称</dt>
              <dd>{properties.name}</dd>
              <dt>路径</dt>
              <dd title={properties.path}>{properties.path}</dd>
              <dt>位置</dt>
              <dd>{properties.remote ? "远端" : "本地"}</dd>
              <dt>类型</dt>
              <dd>{formatFileKind(properties)}</dd>
              <dt>大小</dt>
              <dd>{properties.isFile ? `${formatBytes(properties.size)} (${properties.size} B)` : "-"}</dd>
              <dt>权限</dt>
              <dd>{formatFileMode(properties.permissions)}</dd>
              <dt>修改时间</dt>
              <dd>{formatDateTime(properties.modified)}</dd>
              <dt>访问时间</dt>
              <dd>{formatDateTime(properties.accessed)}</dd>
              <dt>创建时间</dt>
              <dd>{formatDateTime(properties.created)}</dd>
            </dl>
          ) : null}
        </div>
        <footer className="utility-actions">
          <button type="button" onClick={onClose}>关闭</button>
        </footer>
      </div>
    </div>
  );
}

function isSshLikeProfile(profile: SessionProfile): profile is SessionProfile & { connection: Extract<ConnectionConfig, { kind: "ssh" | "tmux" }> } {
  return profile.connection.kind === "ssh" || profile.connection.kind === "tmux";
}

function defaultLocalPath() {
  return "/";
}

function joinFilePath(base: string, name: string, remote: boolean) {
  const separator = remote || base.includes("/") ? "/" : "\\";
  const cleanBase = base.endsWith("/") || base.endsWith("\\") ? base.slice(0, -1) : base;
  return cleanBase ? `${cleanBase}${separator}${name}` : name;
}

function filePaneAtPhysicalPosition(x: number, y: number): boolean | null {
  const scale = window.devicePixelRatio || 1;
  const target = document.elementFromPoint(x / scale, y / scale);
  const pane = target?.closest<HTMLElement>("[data-file-pane]");
  if (pane?.dataset.filePane === "remote") return true;
  if (pane?.dataset.filePane === "local") return false;
  return null;
}

function parentPath(path: string, remote: boolean) {
  const separator = remote || path.includes("/") ? "/" : "\\";
  const trimmed = path.replace(/[\\/]$/, "");
  const index = Math.max(trimmed.lastIndexOf("/"), trimmed.lastIndexOf("\\"));
  if (index <= 0) return separator === "/" ? "/" : trimmed;
  return trimmed.slice(0, index);
}

function formatFileMode(mode?: number | null) {
  if (mode == null) return "-";
  return `0${(mode & 0o7777).toString(8).padStart(3, "0")}`;
}

function formatFileKind(properties: FileProperties) {
  if (properties.isSymlink) return "symlink";
  if (properties.isDir) return "directory";
  if (properties.isFile) return "file";
  return properties.kind || "other";
}

function formatDateTime(value?: string | null) {
  if (!value) return "-";
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return date.toLocaleString();
}

function formatError(error: unknown) {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  try {
    return JSON.stringify(error);
  } catch {
    return String(error);
  }
}
