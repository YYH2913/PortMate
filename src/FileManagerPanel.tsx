import { t, tr, useLocale, localizeDiagnostic, formatUiDate, formatUiNumber } from "./i18n";
import { useEffect, useMemo, useRef, useState } from "react";
import type { DragEvent as ReactDragEvent, MouseEvent as ReactMouseEvent } from "react";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import {
  ArrowDown,
  ArrowUp,
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
import { exactNonBlankPathInput, fileJoinPath as joinFilePath, fileParentPath as parentPath, parseFilePermissionMode } from "./file-path-input";
import {
  createFileNavigationHistory,
  currentFileNavigationPath,
  fileNavigationTarget,
  recordFileNavigation,
  restoreFileNavigation,
} from "./file-navigation-state";
import type { FileNavigationHistory } from "./file-navigation-state";
import { updateFileSelection } from "./file-selection";
import {
  defaultFileSort,
  fileModifiedTimestamp,
  fileNameExtension,
  filePermissionDescription,
  fileSortKeys,
  knownFileSize,
  nextFileSort,
  sortFileEntries,
  summarizeFileEntries,
} from "./file-list-presentation";
import type { FileSort, FileSortKey } from "./file-list-presentation";
import "./file-manager-details.css";
import { KeyedRequestGate } from "./keyed-request-gate";
import TransferList from "./TransferList";
import { fileTransferProtocolsForProfile, transferProtocolLabel } from "./transfer-capabilities";
import type { FileTransferProtocol } from "./transfer-capabilities";
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
  /** Editable address, never an implicit destination for file mutations. */
  path: string;
  directory: string | null;
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
type FilePaneKey = "local" | "remote";

type FileOperationContext = {
  tokens: Array<{ key: FilePaneKey; token: number }>;
  requestSessionId: string | null;
  remoteSessionId: string;
  remoteConnectionKey: string;
};

type ExternalDropState = {
  remote: boolean;
  taskIds: string[];
  message: string;
  status: "planning" | "queued" | "completed" | "warning";
} | null;

export default function FileManagerPanel({
  active,
  transfers,
  dismissedTransferIds,
  onTransfer,
  onDismissTransfer,
  onNotice,
}: {
  active?: SessionSummary;
  transfers: TransferTask[];
  dismissedTransferIds: ReadonlySet<string>;
  onTransfer: (task: TransferTask) => void;
  onDismissTransfer: (transferId: string) => void;
  onNotice: (notice: NoticeState) => void;
}) {
  useLocale();
  const [localPanel, setLocalPanel] = useState<FilePanelState>(() => ({ path: defaultLocalPath(), directory: null, entries: [], selected: [], busy: false, error: "" }));
  const [remotePanel, setRemotePanel] = useState<FilePanelState>(() => ({ path: ".", directory: null, entries: [], selected: [], busy: false, error: "" }));
  const [fileSorts, setFileSorts] = useState<Record<FilePaneKey, FileSort>>(() => ({ local: defaultFileSort, remote: defaultFileSort }));
  const localEntries = useMemo(() => sortFileEntries(localPanel.entries, fileSorts.local), [localPanel.entries, fileSorts.local]);
  const remoteEntries = useMemo(() => sortFileEntries(remotePanel.entries, fileSorts.remote), [remotePanel.entries, fileSorts.remote]);
  const [localNavigation, setLocalNavigation] = useState<FileNavigationHistory>(() => createFileNavigationHistory(defaultLocalPath()));
  const [remoteNavigation, setRemoteNavigation] = useState<FileNavigationHistory>(() => createFileNavigationHistory("."));
  const [propertiesDialog, setPropertiesDialog] = useState<FilePropertiesDialogState>(null);
  const [draggedFile, setDraggedFile] = useState<FileDragState>(null);
  const [dropTarget, setDropTarget] = useState<boolean | null>(null);
  const [externalDrop, setExternalDrop] = useState<ExternalDropState>(null);
  const [conflictPolicy, setConflictPolicy] = useState<TransferConflictPolicy>("fail");
  const [selectedTransferProtocol, setSelectedTransferProtocol] = useState<FileTransferProtocol>("sftp");
  const selectionAnchors = useRef<{ local: string; remote: string }>({ local: "", remote: "" });
  const fileLoadEpochs = useRef({ local: 0, remote: 0 });
  const activeFileLoadEpochs = useRef({ local: 0, remote: 0 });
  const activeFileSessionIdRef = useRef("");
  const activeFileConnectionKeyRef = useRef("");
  const filePropertiesGate = useRef(new KeyedRequestGate<"properties">());
  const fileOperationGate = useRef(new KeyedRequestGate<FilePaneKey>());
  const activeFileOperationsRef = useRef<Set<FileOperationContext>>(new Set());
  const busyFileOperationKeysRef = useRef<Set<FilePaneKey>>(new Set());
  const [busyFileOperationKeys, setBusyFileOperationKeys] = useState<Set<FilePaneKey>>(() => new Set());
  const transferOperationGate = useRef(new KeyedRequestGate<string>());
  const [busyTransferIds, setBusyTransferIds] = useState<Set<string>>(() => new Set());
  const canRemote = Boolean(active && isSshLikeProfile(active.profile) && active.runtime.status === "connected");
  const remoteConnectionKey = canRemote ? JSON.stringify([active?.profile.id, active?.runtime.connectedSince]) : "";
  activeFileConnectionKeyRef.current = remoteConnectionKey;
  const fileTransferProtocols = active ? fileTransferProtocolsForProfile(active.profile) : [];
  const fileTransferProtocol = fileTransferProtocols.includes(selectedTransferProtocol)
    ? selectedTransferProtocol
    : fileTransferProtocols[0] ?? "";
  const canTransferFiles = canRemote
    && Boolean(fileTransferProtocol)
    && !localPanel.busy
    && !remotePanel.busy
    && localPanel.directory !== null
    && remotePanel.directory !== null
    && busyFileOperationKeys.size === 0;
  activeFileSessionIdRef.current = canRemote ? active?.profile.id ?? "" : "";

  useEffect(() => {
    void loadFiles(false, defaultLocalPath(), "reset");
  }, []);

  useEffect(() => () => {
    fileLoadEpochs.current.local += 1;
    fileLoadEpochs.current.remote += 1;
    activeFileLoadEpochs.current = { local: 0, remote: 0 };
    filePropertiesGate.current.invalidateAll();
    fileOperationGate.current.invalidateAll();
    activeFileOperationsRef.current.clear();
    transferOperationGate.current.invalidateAll();
  }, []);

  useEffect(() => {
    [...activeFileOperationsRef.current]
      .filter((operation) => operation.tokens.some(({ key }) => key === "remote"))
      .forEach(finishFileOperation);
    fileOperationGate.current.invalidate("remote");
    activeFileLoadEpochs.current.remote = 0;
    selectionAnchors.current.remote = "";
    setDraggedFile(null);
    setDropTarget(null);
    setExternalDrop(null);
    filePropertiesGate.current.invalidate("properties");
    setPropertiesDialog(null);
    if (canRemote) {
      setRemotePanel((current) => ({ ...current, path: ".", directory: null, entries: [], selected: [], error: "" }));
      setRemoteNavigation(createFileNavigationHistory("."));
      void loadFiles(true, ".", "reset");
    } else {
      fileLoadEpochs.current.remote += 1;
      setRemotePanel((current) => ({ ...current, directory: null, entries: [], selected: [], busy: false, error: "" }));
      setRemoteNavigation(createFileNavigationHistory("."));
    }
  }, [remoteConnectionKey]);

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
      if (!active || remote === null || (remote && (!canRemote || !fileTransferProtocol))) {
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
  }, [remoteConnectionKey, active?.profile.id, localPanel.directory, remotePanel.directory, conflictPolicy, fileTransferProtocol]);

  useEffect(() => {
    if (!externalDrop || externalDrop.status !== "queued" || !externalDrop.taskIds.length) return;
    const batchTasks = externalDrop.taskIds.map((taskId) => transfers.find((task) => task.id === taskId));
    if (batchTasks.some((task) => !task)) return;
    if (batchTasks.some((task) => task?.status === "queued" || task?.status === "running")) return;
    const failed = batchTasks.filter((task) => task?.status === "failed" || task?.status === "cancelled").length;
    const message = failed
      ? t("files-completed-failed-or-cancelled", [batchTasks.length - failed, batchTasks.length, failed])
      : t("files-transferred", [batchTasks.length]);
    setExternalDrop((current) => current ? { ...current, message, status: failed ? "warning" : "completed" } : null);
    void loadFiles(
      externalDrop.remote,
      (externalDrop.remote ? remotePanel.directory : localPanel.directory) ?? ".",
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

  function beginFileOperation(remotes: boolean[]): FileOperationContext | null {
    const keys = [...new Set(remotes.map(filePaneKey))].sort((left, right) => (
      left === right ? 0 : left === "local" ? -1 : 1
    ));
    const remoteSessionId = activeFileSessionIdRef.current;
    if (!keys.length
      || keys.some((key) => activeFileLoadEpochs.current[key] !== 0)
      || (keys.includes("remote") && (!canRemote || !remoteSessionId))) return null;

    const tokens: FileOperationContext["tokens"] = [];
    for (const key of keys) {
      const token = fileOperationGate.current.begin(key);
      if (token === null) {
        tokens.forEach((entry) => fileOperationGate.current.finish(entry.key, entry.token));
        return null;
      }
      tokens.push({ key, token });
    }

    const nextBusy = new Set(busyFileOperationKeysRef.current);
    keys.forEach((key) => nextBusy.add(key));
    busyFileOperationKeysRef.current = nextBusy;
    setBusyFileOperationKeys(nextBusy);
    const operation = {
      tokens,
      requestSessionId: active?.profile.id ?? null,
      remoteSessionId,
      remoteConnectionKey: activeFileConnectionKeyRef.current,
    };
    activeFileOperationsRef.current.add(operation);
    return operation;
  }

  function isFileOperationCurrent(operation: FileOperationContext) {
    return operation.tokens.every(({ key, token }) => fileOperationGate.current.isCurrent(key, token))
      && (!operation.tokens.some(({ key }) => key === "remote")
        || (activeFileSessionIdRef.current === operation.remoteSessionId
          && activeFileConnectionKeyRef.current === operation.remoteConnectionKey));
  }

  function finishFileOperation(operation: FileOperationContext) {
    activeFileOperationsRef.current.delete(operation);
    const released: FilePaneKey[] = [];
    operation.tokens.forEach(({ key, token }) => {
      if (fileOperationGate.current.finish(key, token)) released.push(key);
    });
    if (!released.length) return;
    const nextBusy = new Set(busyFileOperationKeysRef.current);
    released.forEach((key) => nextBusy.delete(key));
    busyFileOperationKeysRef.current = nextBusy;
    setBusyFileOperationKeys(nextBusy);
  }

  function releaseCurrentFileOperation(operation: FileOperationContext) {
    if (!isFileOperationCurrent(operation)) return false;
    finishFileOperation(operation);
    return true;
  }

  function selectFileEntry(remote: boolean, entry: FileEntry, event: FileSelectionModifiers) {
    const setter = remote ? setRemotePanel : setLocalPanel;
    const anchorKey = remote ? "remote" : "local";
    setter((current) => {
      const result = updateFileSelection(
        sortFileEntries(current.entries, fileSorts[anchorKey]),
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
      selected: current.selected.length === current.entries.length ? [] : sortFileEntries(current.entries, fileSorts[filePaneKey(remote)]),
    }));
  }

  function changeFileSort(remote: boolean, key: FileSortKey) {
    const pane = filePaneKey(remote);
    setFileSorts(current => ({ ...current, [pane]: nextFileSort(current[pane], key) }));
  }

  async function loadFiles(
    remote: boolean,
    nextPath = remote ? remotePanel.path : localPanel.path,
    navigation: FileLoadNavigation = "record",
  ) {
    const loadKey = filePaneKey(remote);
    if (busyFileOperationKeysRef.current.has(loadKey)) return;
    const sessionId = remote ? active?.profile.id ?? "" : "";
    const connectionKey = activeFileConnectionKeyRef.current;
    if (remote && (!canRemote || !sessionId)) return;
    const epoch = fileLoadEpochs.current[loadKey] + 1;
    fileLoadEpochs.current[loadKey] = epoch;
    activeFileLoadEpochs.current[loadKey] = epoch;
    updatePanel(remote, {
      ...(navigation === "preserve" ? {} : { path: nextPath }),
      selected: [], busy: true, error: "",
    });
    selectionAnchors.current[loadKey] = "";
    try {
      const nextEntries = await invokeBackend<FileEntry[]>("list_files", { request: { sessionId: sessionId || null, path: nextPath, remote } });
      if (fileLoadEpochs.current[loadKey] !== epoch
        || (remote && activeFileConnectionKeyRef.current !== connectionKey)) return;
      updatePanel(remote, { entries: nextEntries, directory: nextPath, selected: [] });
      updateNavigation(remote, nextPath, navigation);
      selectionAnchors.current[remote ? "remote" : "local"] = "";
    } catch (error) {
      if (fileLoadEpochs.current[loadKey] !== epoch
        || (remote && activeFileConnectionKeyRef.current !== connectionKey)) return;
      updatePanel(remote, { entries: [], selected: [], directory: null, error: formatError(error) });
    } finally {
      if (fileLoadEpochs.current[loadKey] === epoch
        && (!remote || activeFileConnectionKeyRef.current === connectionKey)) {
        activeFileLoadEpochs.current[loadKey] = 0;
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

  async function runFileMutation(
    remote: boolean,
    refreshPath: string,
    mutate: (sessionId: string | null) => Promise<boolean>,
  ) {
    const operation = beginFileOperation([remote]);
    if (!operation) return;
    try {
      if (!await mutate(operation.requestSessionId)) return;
      if (!releaseCurrentFileOperation(operation)) return;
      await loadFiles(remote, refreshPath, "preserve");
    } catch (error) {
      if (isFileOperationCurrent(operation)) updatePanel(remote, { error: formatError(error) });
    } finally {
      finishFileOperation(operation);
    }
  }

  async function createDir(remote: boolean) {
    const panel = remote ? remotePanel : localPanel;
    if (panel.directory === null) return;
    const directory = panel.directory;
    await runFileMutation(remote, directory, async (sessionId) => {
      const name = exactNonBlankPathInput(window.prompt(t("directory-name")));
      if (name === null) return false;
      const nextPath = joinFilePath(directory, name, remote);
      await invokeBackend("create_directory", { request: { sessionId, path: nextPath, remote } });
      return true;
    });
  }

  async function createFile(remote: boolean) {
    const panel = remote ? remotePanel : localPanel;
    if (panel.directory === null) return;
    const directory = panel.directory;
    await runFileMutation(remote, directory, async (sessionId) => {
      const name = exactNonBlankPathInput(window.prompt(t("file-name")));
      if (name === null) return false;
      const nextPath = joinFilePath(directory, name, remote);
      await invokeBackend("create_file", { request: { sessionId, path: nextPath, remote } });
      return true;
    });
  }

  async function deleteSelected(remote: boolean) {
    const panel = remote ? remotePanel : localPanel;
    if (!panel.selected.length || panel.directory === null) return;
    await runFileMutation(remote, panel.directory, async (sessionId) => {
      if (!window.confirm(t("delete-selected-items", [panel.selected.length]))) return false;
      await invokeBackend("delete_paths", {
        request: {
          sessionId,
          paths: panel.selected.map((entry) => entry.path),
          remote,
        },
      });
      return true;
    });
  }

  async function renameSelected(remote: boolean) {
    const panel = remote ? remotePanel : localPanel;
    const selected = panel.selected[0];
    if (panel.selected.length !== 1 || !selected || panel.directory === null) return;
    await runFileMutation(remote, panel.directory, async (sessionId) => {
      const nextName = exactNonBlankPathInput(window.prompt(t("new-name"), selected.name));
      if (nextName === null) return false;
      const nextPath = joinFilePath(parentPath(selected.path, remote), nextName, remote);
      await invokeBackend("rename_path", { request: { sessionId, oldPath: selected.path, newPath: nextPath, remote } });
      return true;
    });
  }

  async function moveSelected(remote: boolean) {
    const panel = remote ? remotePanel : localPanel;
    if (!panel.selected.length || panel.directory === null) return;
    const directory = panel.directory;
    await runFileMutation(remote, directory, async (sessionId) => {
      const suggestedDestination = parentPath(directory, remote);
      const destination = exactNonBlankPathInput(window.prompt(
        t("move-to-directory"),
        suggestedDestination === "/" || suggestedDestination === "." || suggestedDestination === "~" ? "" : suggestedDestination,
      ));
      if (destination === null) return false;
      await invokeBackend("move_paths", {
        request: {
          sessionId,
          paths: panel.selected.map((entry) => entry.path),
          destination,
          remote,
        },
      });
      return true;
    });
  }

  async function copySelectedPaths(remote: boolean) {
    const panel = remote ? remotePanel : localPanel;
    if (!panel.selected.length) return;
    try {
      if (!navigator.clipboard?.writeText) {
        throw new Error(t("writing-to-the-clipboard-is-unavailable-in-this-environment"));
      }
      await navigator.clipboard.writeText(panel.selected.map((entry) => entry.path).join("\n"));
    } catch (error) {
      updatePanel(remote, { error: formatError(error) });
    }
  }

  async function chmodSelected(remote: boolean) {
    const panel = remote ? remotePanel : localPanel;
    const selected = panel.selected[0];
    if (panel.selected.length !== 1 || !selected || panel.directory === null) return;
    await runFileMutation(remote, panel.directory, async (sessionId) => {
      const modeText = window.prompt(t("octal-permissions"), "0644");
      if (!modeText?.trim()) return false;
      const mode = parseFilePermissionMode(modeText);
      await invokeBackend("chmod_path", { request: { sessionId, path: selected.path, mode, remote } });
      return true;
    });
  }

  async function showProperties(remote: boolean, entry?: FileEntry) {
    const panel = remote ? remotePanel : localPanel;
    const selected = entry ?? panel.selected[0];
    if ((!entry && panel.selected.length !== 1) || !selected) return;
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
    if (!active || !canTransferFiles) return;
    const selected = upload ? localPanel.selected : remotePanel.selected;
    if (!selected.length) return;
    await queueFileBatch(
      !upload,
      selected,
      upload,
      (upload ? remotePanel.directory : localPanel.directory)!,
      upload ? t("batch-upload") : t("batch-download"),
    );
  }

  async function queueFileBatch(
    sourceRemote: boolean,
    entries: FileEntry[],
    destinationRemote: boolean,
    destination: string,
    title: string,
  ) {
    if (!active || !canRemote || !fileTransferProtocol || !entries.length) return;
    const operation = beginFileOperation([sourceRemote, destinationRemote]);
    if (!operation) return;
    if (!operation.requestSessionId) {
      finishFileOperation(operation);
      return;
    }
    setExternalDrop({
      remote: destinationRemote,
      taskIds: [],
      message: t("planning-selected-items", [entries.length]),
      status: "planning",
    });
    updatePanel(destinationRemote, { error: "" });
    try {
      const result = await invokeBackend<ExternalDropResult>("start_file_batch", {
        request: {
          sessionId: operation.requestSessionId,
          protocol: fileTransferProtocol,
          paths: entries.map((entry) => entry.path),
          sourceRemote,
          destination,
          destinationRemote,
          conflictPolicy,
        },
      });
      if (!isFileOperationCurrent(operation)) return;
      result.tasks.forEach(onTransfer);
      const parts = [
        t("files", [result.tasks.length]),
        formatBytes(result.totalBytes),
        t("new-directories", [result.directoriesPrepared]),
      ];
      if (result.skipped.length) parts.push(t("skipped-items", [result.skipped.length]));
      const message = parts.join(" · ");
      setExternalDrop({
        remote: destinationRemote,
        taskIds: result.tasks.map((task) => task.id),
        message,
        status: result.tasks.length ? "queued" : result.skipped.length ? "warning" : "completed",
      });
      onNotice({ title, message });
      if (!releaseCurrentFileOperation(operation)) return;
      if (!result.tasks.length) {
        await loadFiles(destinationRemote, destination, "preserve");
      }
    } catch (error) {
      if (!isFileOperationCurrent(operation)) return;
      const message = formatError(error);
      setExternalDrop(null);
      updatePanel(destinationRemote, { error: message });
      onNotice({ title: t("failed", [title]), message });
    } finally {
      finishFileOperation(operation);
    }
  }

  function startFileDrag(remote: boolean, entry: FileEntry, event: ReactDragEvent<HTMLElement>) {
    if (!canTransferFiles) return;
    const panel = remote ? remotePanel : localPanel;
    const entries = panel.selected.some((item) => item.path === entry.path) ? panel.selected : [entry];
    setDraggedFile({ remote, entries });
    event.dataTransfer.effectAllowed = "copy";
    event.dataTransfer.setData("application/x-portmate-file", JSON.stringify({ remote, paths: entries.map((item) => item.path) }));
  }

  function handleDragOver(remote: boolean, event: ReactDragEvent<HTMLElement>) {
    if (!canTransferFiles || !draggedFile || draggedFile.remote === remote) return;
    event.preventDefault();
    event.dataTransfer.dropEffect = "copy";
    setDropTarget(remote);
  }

  async function dropFile(remote: boolean, event: ReactDragEvent<HTMLElement>) {
    event.preventDefault();
    const dropped = draggedFile;
    setDropTarget(null);
    setDraggedFile(null);
    if (!active || !canTransferFiles || !dropped || dropped.remote === remote) return;
    const targetPanel = remote ? remotePanel : localPanel;
    if (targetPanel.directory !== null) await queueFileBatch(dropped.remote, dropped.entries, remote, targetPanel.directory, t("drag-and-drop-transfer"));
  }

  async function startExternalDrop(remote: boolean, paths: string[]) {
    if (!active || (remote && (!canRemote || !fileTransferProtocol)) || !paths.length) return;
    const panel = remote ? remotePanel : localPanel;
    if (panel.directory === null) return;
    const directory = panel.directory;
    const operation = beginFileOperation([remote]);
    if (!operation) return;
    if (!operation.requestSessionId) {
      finishFileOperation(operation);
      return;
    }
    setExternalDrop({
      remote,
      taskIds: [],
      message: t("analyzing-dropped-paths", [paths.length]),
      status: "planning",
    });
    updatePanel(remote, { error: "" });
    try {
      const result = await invokeBackend<ExternalDropResult>("start_external_drop", {
        request: {
          sessionId: operation.requestSessionId,
          protocol: remote ? fileTransferProtocol : "sftp",
          paths,
          destination: directory,
          remote,
          conflictPolicy,
        },
      });
      if (!isFileOperationCurrent(operation)) return;
      result.tasks.forEach(onTransfer);
      const parts = [
        t("files", [result.tasks.length]),
        formatBytes(result.totalBytes),
        t("directories", [result.directoriesPrepared]),
      ];
      if (result.skipped.length) parts.push(t("skipped-items", [result.skipped.length]));
      const message = parts.join(" · ");
      setExternalDrop({
        remote,
        taskIds: result.tasks.map((task) => task.id),
        message,
        status: result.tasks.length ? "queued" : result.skipped.length ? "warning" : "completed",
      });
      onNotice({ title: t("external-drop-processed"), message });
      if (!releaseCurrentFileOperation(operation)) return;
      if (!result.tasks.length) {
        await loadFiles(remote, directory, "preserve");
      }
    } catch (error) {
      if (!isFileOperationCurrent(operation)) return;
      const message = formatError(error);
      setExternalDrop(null);
      updatePanel(remote, { error: message });
      onNotice({ title: t("external-drop-failed"), message });
    } finally {
      finishFileOperation(operation);
    }
  }

  async function retryTransfer(task: TransferTask) {
    const token = beginTransferOperation(task.id);
    if (token === null) return;
    try {
      const retried = await invokeBackend<TransferTask>("retry_transfer", { transferId: task.id });
      if (!transferOperationGate.current.isCurrent(task.id, token)) return;
      onTransfer(retried);
      onNotice({ title: t("retry-transfer"), message: `${retried.protocol} ${retried.status}: ${retried.message ?? ""}` });
    } catch (error) {
      if (transferOperationGate.current.isCurrent(task.id, token)) {
        onNotice({ title: t("failed-to-retry-transfer"), message: formatError(error) });
      }
    } finally {
      finishTransferOperation(task.id, token);
    }
  }

  async function cancelTransfer(task: TransferTask) {
    const token = beginTransferOperation(task.id);
    if (token === null) return;
    try {
      const cancelled = await invokeBackend<TransferTask>("cancel_transfer", { transferId: task.id });
      if (!transferOperationGate.current.isCurrent(task.id, token)) return;
      onTransfer(cancelled);
      onNotice({ title: t("cancel-transfer"), message: `${cancelled.protocol} ${cancelled.status}: ${cancelled.message ?? ""}` });
    } catch (error) {
      if (transferOperationGate.current.isCurrent(task.id, token)) {
        onNotice({ title: t("failed-to-cancel-transfer"), message: formatError(error) });
      }
    } finally {
      finishTransferOperation(task.id, token);
    }
  }

  function beginTransferOperation(transferId: string): number | null {
    const token = transferOperationGate.current.begin(transferId);
    if (token !== null) setBusyTransferIds((current) => new Set(current).add(transferId));
    return token;
  }

  function finishTransferOperation(transferId: string, token: number) {
    if (!transferOperationGate.current.finish(transferId, token)) return;
    setBusyTransferIds((current) => {
      const next = new Set(current);
      next.delete(transferId);
      return next;
    });
  }

  return (
    <div className={canRemote ? "file-manager dual" : "file-manager"}>
      <div className="file-panels">
        <FileBrowserPane
          title={t("local")}
          remote={false}
          panel={localPanel}
          entries={localEntries}
          sort={fileSorts.local}
          onSort={(key) => changeFileSort(false, key)}
          onInspect={(entry) => void showProperties(false, entry)}
          operationBusy={busyFileOperationKeys.has("local")}
          canTransfer={canTransferFiles}
          transferLabel={t("upload")}
          onPathChange={(path) => setLocalPanel((current) => ({ ...current, path }))}
          canGoBack={fileNavigationTarget(localNavigation, -1) !== null}
          canGoForward={fileNavigationTarget(localNavigation, 1) !== null}
          onGoBack={() => navigateHistory(false, -1)}
          onGoForward={() => navigateHistory(false, 1)}
          onNavigate={(path) => void loadFiles(false, path)}
          onRefresh={() => void loadFiles(false, currentFileNavigationPath(localNavigation) ?? localPanel.path, "preserve")}
          conflictPolicy={conflictPolicy}
          transferProtocol={fileTransferProtocol}
          transferProtocols={fileTransferProtocols}
          onConflictPolicyChange={setConflictPolicy}
          onTransferProtocolChange={setSelectedTransferProtocol}
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
          onTransfer={() => void transferBetween(true)}
        />
        {canRemote ? (
          <FileBrowserPane
            title={t("remote")}
            remote
            panel={remotePanel}
            entries={remoteEntries}
            sort={fileSorts.remote}
            onSort={(key) => changeFileSort(true, key)}
            onInspect={(entry) => void showProperties(true, entry)}
            operationBusy={busyFileOperationKeys.has("remote")}
            canTransfer={canTransferFiles}
            transferLabel={t("download")}
            onPathChange={(path) => setRemotePanel((current) => ({ ...current, path }))}
            canGoBack={fileNavigationTarget(remoteNavigation, -1) !== null}
            canGoForward={fileNavigationTarget(remoteNavigation, 1) !== null}
            onGoBack={() => navigateHistory(true, -1)}
            onGoForward={() => navigateHistory(true, 1)}
            onNavigate={(path) => void loadFiles(true, path)}
            onRefresh={() => void loadFiles(true, currentFileNavigationPath(remoteNavigation) ?? remotePanel.path, "preserve")}
            conflictPolicy={conflictPolicy}
            transferProtocol={fileTransferProtocol}
            transferProtocols={fileTransferProtocols}
            onConflictPolicyChange={setConflictPolicy}
            onTransferProtocolChange={setSelectedTransferProtocol}
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
      <TransferList
        transfers={transfers.slice(-3)}
        dismissedTransferIds={dismissedTransferIds}
        busyTransferIds={busyTransferIds}
        onRetry={retryTransfer}
        onCancel={cancelTransfer}
        onDismiss={onDismissTransfer}
      />
      {propertiesDialog ? <FilePropertiesDialog state={propertiesDialog} onClose={closePropertiesDialog} /> : null}
    </div>
  );
}

function FileBrowserPane({
  title,
  remote,
  panel,
  entries,
  sort,
  onSort,
  onInspect,
  operationBusy,
  canTransfer,
  transferLabel,
  canGoBack,
  canGoForward,
  dropActive,
  dropStatus,
  conflictPolicy,
  transferProtocol,
  transferProtocols,
  onPathChange,
  onGoBack,
  onGoForward,
  onNavigate,
  onRefresh,
  onSelect,
  onSelectAll,
  onConflictPolicyChange,
  onTransferProtocolChange,
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
  entries: FileEntry[];
  sort: FileSort;
  onSort: (key: FileSortKey) => void;
  onInspect: (entry: FileEntry) => void;
  operationBusy: boolean;
  canTransfer: boolean;
  transferLabel: string;
  canGoBack: boolean;
  canGoForward: boolean;
  dropActive: boolean;
  dropStatus: ExternalDropState;
  conflictPolicy: TransferConflictPolicy;
  transferProtocol: FileTransferProtocol | "";
  transferProtocols: FileTransferProtocol[];
  onPathChange: (path: string) => void;
  onGoBack: () => void;
  onGoForward: () => void;
  onNavigate: (path: string) => void;
  onRefresh: () => void;
  onSelect: (entry: FileEntry, event: FileSelectionModifiers) => void;
  onSelectAll: () => void;
  onConflictPolicyChange: (policy: TransferConflictPolicy) => void;
  onTransferProtocolChange: (protocol: FileTransferProtocol) => void;
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
  const { locale } = useLocale();
  const locked = panel.busy || operationBusy;
  const selectedPaths = useMemo(() => new Set(panel.selected.map(entry => entry.path)), [panel.selected]);
  const summary = useMemo(() => summarizeFileEntries(entries), [entries]);
  const selectedSummary = useMemo(() => summarizeFileEntries(panel.selected), [panel.selected]);
  // A directory can contain thousands of rows; reuse ICU formatters across the entire pane.
  const modifiedFormatter = useMemo(() => new Intl.DateTimeFormat(locale, {
    year: "numeric", month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit",
  }), [locale]);
  const byteFormatter = useMemo(() => new Intl.NumberFormat(locale), [locale]);
  const listRef = useRef<HTMLDivElement>(null);
  const rightToLeft = locale === "ar";
  useEffect(() => {
    // Browser scroll offsets change sign in RTL; don't carry a previous direction's
    // offset into the new layout and leave the first column partially offscreen.
    if (listRef.current) listRef.current.scrollLeft = 0;
  }, [rightToLeft]);

  function closeOverflowAndRun(event: ReactMouseEvent<HTMLButtonElement>, action: () => void) {
    event.currentTarget.closest("details")?.removeAttribute("open");
    action();
  }

  return (
    <section
      className={dropActive ? "file-browser-pane drop-active" : "file-browser-pane"}
      data-file-pane={remote ? "remote" : "local"}
      data-file-directory={panel.directory ?? ""}
      aria-busy={locked}
      onDragOver={locked ? undefined : onDragOver}
      onDragLeave={locked ? undefined : onDragLeave}
      onDrop={locked ? undefined : onDrop}
    >
      <div className="file-toolbar">
        <strong>{title}</strong>
        <input className="file-path-input" aria-label={t("path", [title])} value={panel.path} disabled={operationBusy} onChange={(event) => onPathChange(event.target.value)} onKeyDown={(event) => {
          if (event.key === "Enter") {
            onNavigate(panel.path);
          }
        }} />
        <button type="button" title={t("back", [title])} aria-label={t("back", [title])} onClick={onGoBack} disabled={!canGoBack || locked}><ChevronLeft size={13} /></button>
        <button type="button" title={t("forward", [title])} aria-label={t("forward", [title])} onClick={onGoForward} disabled={!canGoForward || locked}><ChevronRight size={13} /></button>
        <button type="button" title={t("refresh-directory", [title])} aria-label={t("refresh-directory", [title])} onClick={onRefresh} disabled={locked}><RefreshCw size={13} /></button>
      </div>
      <div className="file-actions">
        <button type="button" title={panel.selected.length === panel.entries.length && panel.entries.length ? t("clear-selection") : t("select-all-2")} aria-label={panel.selected.length === panel.entries.length && panel.entries.length ? t("clear-selection") : t("select-all-2")} onClick={onSelectAll} disabled={locked}><ListChecks size={13} /></button>
        <button type="button" title={t("new-folder")} aria-label={t("new-folder")} onClick={onCreateDir} disabled={locked || panel.directory === null}><FolderPlus size={13} /></button>
        <button type="button" title={t("new-file")} aria-label={t("new-file")} onClick={onCreateFile} disabled={locked || panel.directory === null}><FilePlus size={13} /></button>
        <button type="button" title={t("delete")} aria-label={t("delete")} onClick={onDelete} disabled={locked || !panel.selected.length}><Trash2 size={13} /></button>
        <details className="file-action-overflow">
          <summary title={t("more-file-actions")} aria-label={t("more-file-actions")}><MoreHorizontal size={13} /></summary>
          <div className="file-action-overflow-menu">
            <button type="button" title={t("copy-path")} aria-label={t("copy-path")} onClick={(event) => closeOverflowAndRun(event, onCopyPaths)} disabled={locked || !panel.selected.length}><Copy size={13} /><span>{t("copy-path")}</span></button>
            <button type="button" title={t("move-to")} aria-label={t("move-to")} onClick={(event) => closeOverflowAndRun(event, onMove)} disabled={locked || !panel.selected.length}><FolderInput size={13} /><span>{t("move-to")}</span></button>
            <button type="button" title={t("rename")} aria-label={t("rename")} onClick={(event) => closeOverflowAndRun(event, onRename)} disabled={locked || panel.selected.length !== 1}><Pencil size={13} /><span>{t("rename")}</span></button>
            <button type="button" title={t("change-permissions")} aria-label={t("change-permissions")} onClick={(event) => closeOverflowAndRun(event, onChmod)} disabled={locked || panel.selected.length !== 1}><ShieldCheck size={13} /><span>{t("change-permissions")}</span></button>
            <button type="button" title={t("file-properties")} aria-label={t("file-properties")} onClick={(event) => closeOverflowAndRun(event, onProperties)} disabled={locked || panel.selected.length !== 1}><Info size={13} /><span>{t("file-properties")}</span></button>
          </div>
        </details>
        <select value={conflictPolicy} disabled={locked} onChange={(event) => onConflictPolicyChange(event.target.value as TransferConflictPolicy)} aria-label={t("file-conflict-policy")} title={t("file-conflict-policy")}>
          <option value="fail">{t("stop")}</option>
          <option value="overwrite">{t("overwrite")}</option>
          <option value="skip">{t("skip")}</option>
          <option value="rename">{t("rename")}</option>
        </select>
        {transferProtocols.length ? (
          <select
            value={transferProtocol}
            disabled={locked}
            onChange={(event) => onTransferProtocolChange(event.target.value as FileTransferProtocol)}
            aria-label={t("file-transfer-protocol")}
            title={t("file-transfer-protocol")}
          >
            {transferProtocols.map((protocol) => (
              <option key={protocol} value={protocol}>{transferProtocolLabel(protocol)}</option>
            ))}
          </select>
        ) : null}
        <button type="button" title={transferLabel} aria-label={`${transferLabel}${panel.selected.length > 1 ? t("items", [panel.selected.length]) : ""}`} onClick={onTransfer} disabled={locked || !panel.selected.length || !canTransfer}>
          {remote ? <Download size={13} /> : <Upload size={13} />}
        </button>
      </div>
      <div className="file-pane-messages">
        {panel.directory !== null && panel.path !== panel.directory ? (
          <div className="file-pane-status">{t("showing-press-enter-to-open-the-new-path", [panel.directory])}</div>
        ) : null}
        {panel.error ? (
          <div className="file-error" role="alert">{localizeDiagnostic(panel.error)}</div>
        ) : dropStatus ? (
          <div className={`file-pane-status ${dropStatus.status}`}>{dropStatus.message}</div>
        ) : null}
      </div>
      <div className="file-list file-details-list" ref={listRef}>
        <div className="file-list-header">
          <span />
          <span />
          {fileSortKeys.map(key => (
            <button
              type="button"
              key={key}
              data-file-sort={key}
              data-sort-direction={sort.key === key ? sort.direction : "none"}
              aria-pressed={sort.key === key}
              aria-label={`${t("file-sort-by", [t(key)])} · ${t(nextFileSort(sort, key).direction === "asc" ? "file-sort-ascending" : "file-sort-descending")}`}
              title={t("file-sort-by", [t(key)])}
              disabled={locked}
              onClick={() => onSort(key)}
            >
              <span>{t(key)}</span>
              {sort.key === key ? sort.direction === "asc" ? <ArrowUp size={12} /> : <ArrowDown size={12} /> : null}
            </button>
          ))}
        </div>
        <button type="button" className="file-row up" aria-label={t("file-parent-directory")} disabled={locked || panel.directory === null} onClick={() => panel.directory !== null && onNavigate(parentPath(panel.directory, remote))}>
          <span className="file-row-check" />
          <Folder size={13} />
          <span>..</span>
        </button>
        <div className="file-list-entries" role="listbox" aria-label={`${title} · ${t("file-manager")}`} aria-multiselectable="true">
        {entries.map((entry) => (
          <div
            key={entry.path}
            data-file-path={entry.path}
            className={selectedPaths.has(entry.path) ? "file-row active" : "file-row"}
            role="option"
            aria-selected={selectedPaths.has(entry.path)}
            aria-disabled={locked}
            aria-keyshortcuts="Alt+Enter"
            tabIndex={locked ? -1 : 0}
            draggable={canTransfer && !locked}
            onDragStart={(event) => {
              if (!locked) onDragStart(entry, event);
            }}
            onDragEnd={onDragEnd}
            onClick={(event) => {
              if (!locked) onSelect(entry, event);
            }}
            onKeyDown={(event) => {
              if (!locked && event.key === "Enter" && event.altKey) {
                event.preventDefault();
                onInspect(entry);
              } else if (!locked && (event.key === "Enter" || event.key === " ")) {
                event.preventDefault();
                onSelect(entry, event);
              }
            }}
            onDoubleClick={() => {
              if (!locked && entry.isDir) {
                onNavigate(entry.path);
              } else if (!locked) {
                onInspect(entry);
              }
            }}
          >
            <input type="checkbox" tabIndex={-1} readOnly disabled={locked} checked={selectedPaths.has(entry.path)} aria-label={t("select", [entry.name])} />
            {entry.isDir ? <Folder size={13} /> : <File size={13} />}
            <span className="file-entry-name" title={entry.path}>{entry.name}</span>
            <small className="file-entry-type" title={formatEntryType(entry)}>{formatEntryType(entry)}</small>
            <small className="file-entry-size" title={entry.isDir ? t("file-size-summary-hint") : knownFileSize(entry.size) === null ? t("unknown") : `${byteFormatter.format(entry.size)} B`}>
              {entry.isDir ? "—" : knownFileSize(entry.size) === null ? t("unknown") : formatBytes(entry.size)}
            </small>
            <small className="file-entry-modified" title={entry.modified ?? t("unknown")}>{formatDateTime(entry.modified, modifiedFormatter)}</small>
          </div>
        ))}
        </div>
        {!locked && panel.directory !== null && !entries.length ? <div className="file-list-empty">{t("file-empty-directory")}</div> : null}
      </div>
      <div className="file-list-summary" role="status" aria-live="polite" title={t("file-size-summary-hint")}>
        <span>{t("directories", [formatUiNumber(summary.directories)])} · {t("files", [formatUiNumber(summary.files)])} · {tr("file-listed-size", [<bdi dir={summary.unknownSizes ? "auto" : "ltr"}>{summary.unknownSizes ? t("unknown") : formatBytes(summary.bytes)}</bdi>])}</span>
        {selectedSummary.count ? <span>{t("selected", [formatUiNumber(selectedSummary.count)])} · {tr("file-selected-size", [<bdi dir={selectedSummary.unknownSizes ? "auto" : "ltr"}>{selectedSummary.unknownSizes ? t("unknown") : formatBytes(selectedSummary.bytes)}</bdi>])}</span> : null}
      </div>
    </section>
  );
}

function FilePropertiesDialog({ state, onClose }: { state: NonNullable<FilePropertiesDialogState>; onClose: () => void }) {
  useLocale();
  const properties = state.properties;
  return (
    <div className="dialog-backdrop utility-backdrop" onMouseDown={(event) => event.target === event.currentTarget && onClose()}>
      <div className="wind-dialog file-properties-dialog" role="dialog" aria-modal="true" aria-labelledby="file-properties-title">
        <header className="dialog-title">
          <span id="file-properties-title">{t("file-properties")}</span>
          <button type="button" onClick={onClose} aria-label={t("close")}><X size={20} /></button>
        </header>
        <div className="file-properties-content">
          {state.busy ? <div className="empty-pane top">{t("loading-2")}</div> : null}
          {state.error ? <div className="file-error">{localizeDiagnostic(state.error)}</div> : null}
          {properties ? (
            <dl className="property-grid">
              <dt>{t("name")}</dt>
              <dd className="file-property-raw">{properties.name}</dd>
              <dt>{t("path-2")}</dt>
              <dd className="file-property-raw" title={properties.path}>{properties.path}</dd>
              <dt>{t("location")}</dt>
              <dd>{properties.remote ? t("remote") : t("local")}</dd>
              <dt>{t("type")}</dt>
              <dd>{formatFileKind(properties)}</dd>
              <dt>{t("size")}</dt>
              <dd className="file-property-size">{properties.isFile ? knownFileSize(properties.size) === null ? t("unknown") : `${formatBytes(properties.size)} (${formatUiNumber(properties.size)} B)` : "—"}</dd>
              <dt>{t("permissions")}</dt>
              <dd className="file-property-raw">{filePermissionDescription(properties.permissions) ?? t("unknown")}</dd>
              <dt>{t("modified")}</dt>
              <dd title={properties.modified ?? undefined}>{formatDateTime(properties.modified)}</dd>
              <dt>{t("accessed")}</dt>
              <dd title={properties.accessed ?? undefined}>{formatDateTime(properties.accessed)}</dd>
              <dt>{t("created")}</dt>
              <dd title={properties.created ?? undefined}>{formatDateTime(properties.created)}</dd>
            </dl>
          ) : null}
        </div>
        <footer className="utility-actions">
          <button type="button" onClick={onClose}>{t("close")}</button>
        </footer>
      </div>
    </div>
  );
}

function isSshLikeProfile(profile: SessionProfile): profile is SessionProfile & { connection: Extract<ConnectionConfig, { kind: "ssh" | "tmux" }> } {
  return profile.connection.kind === "ssh" || profile.connection.kind === "tmux";
}

function defaultLocalPath() {
  return "~";
}

function filePaneKey(remote: boolean): FilePaneKey {
  return remote ? "remote" : "local";
}

function filePaneAtPhysicalPosition(x: number, y: number): boolean | null {
  const scale = window.devicePixelRatio || 1;
  const target = document.elementFromPoint(x / scale, y / scale);
  const pane = target?.closest<HTMLElement>("[data-file-pane]");
  if (pane?.dataset.filePane === "remote") return true;
  if (pane?.dataset.filePane === "local") return false;
  return null;
}

function formatEntryType(entry: FileEntry) {
  if (entry.isDir) return t("directory");
  const extension = fileNameExtension(entry.name);
  return extension ? t("file-extension-type", [extension.toUpperCase()]) : t("file-kind-file");
}

function formatFileKind(properties: FileProperties) {
  if (properties.isSymlink) return t("file-kind-symlink");
  if (properties.isDir) return t("directory");
  if (properties.isFile) return formatEntryType(properties);
  return t("file-kind-other");
}

function formatDateTime(value?: string | null, formatter?: Intl.DateTimeFormat) {
  const timestamp = fileModifiedTimestamp(value);
  if (timestamp === null) return t("unknown");
  return formatter ? formatter.format(timestamp) : formatUiDate(timestamp, { dateStyle: "medium", timeStyle: "long" });
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
