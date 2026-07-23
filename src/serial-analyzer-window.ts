import { WebviewWindow } from "@tauri-apps/api/webviewWindow";
import { isBackendAvailable } from "./api";
import { buildSerialAnalyzerPath } from "./serial-analyzer-route";
import type { SerialAnalyzerRequest } from "./serial-analyzer-route";
import { placeAndTrackChildWindow, serialAnalyzerWindowGeometryKey } from "./window-geometry";

export async function openSerialAnalyzerWindow(request: SerialAnalyzerRequest, sessionName: string): Promise<void> {
  const path = buildSerialAnalyzerPath(request);
  if (!isBackendAvailable()) {
    const popup = window.open(path, request.windowId, "popup,width=1180,height=760,resizable=yes");
    if (!popup) throw new Error("浏览器阻止了串口分析窗口，请允许 PortMate 打开弹出窗口。");
    popup.focus();
    return;
  }
  await new Promise<void>((resolve, reject) => {
    let settled = false;
    const finish = (error?: Error) => {
      if (settled) return;
      settled = true;
      window.clearTimeout(timeout);
      if (error) reject(error);
      else resolve();
    };
    const timeout = window.setTimeout(() => finish(new Error("创建串口分析窗口超时")), 8_000);
    const child = new WebviewWindow(request.windowId, {
      url: path,
      title: `${sessionName} - PortMate 串口分析器`,
      center: true,
      visible: false,
      width: 1180,
      height: 760,
      minWidth: 720,
      minHeight: 480,
      preventOverflow: true,
    });
    void child.once("tauri://created", () => {
      void placeAndTrackChildWindow(child, {
        storageKey: serialAnalyzerWindowGeometryKey(request.sessionId),
        width: 1180,
        height: 760,
        minWidth: 720,
        minHeight: 480,
      }).then(() => finish(), (error) => finish(new Error(formatWindowError(error))));
    });
    void child.once<unknown>("tauri://error", (event) => finish(new Error(formatWindowError(event.payload))));
  });
}

function formatWindowError(error: unknown) {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  try {
    return JSON.stringify(error);
  } catch {
    return String(error);
  }
}
