import { WebviewWindow } from "@tauri-apps/api/webviewWindow";
import { isBackendAvailable } from "./api";
import { waitForChildWindowReady } from "./child-window-launch";
import { buildSerialAnalyzerPath } from "./serial-analyzer-route";
import type { SerialAnalyzerRequest } from "./serial-analyzer-route";
import { placeAndTrackChildWindow, serialAnalyzerWindowGeometryKey } from "./window-geometry";

export interface SerialAnalyzerWindowController {
  close: () => Promise<void>;
}

export async function openSerialAnalyzerWindow(
  request: SerialAnalyzerRequest,
  sessionName: string,
  isCurrent: () => boolean = () => true,
): Promise<SerialAnalyzerWindowController> {
  const path = buildSerialAnalyzerPath(request);
  if (!isBackendAvailable()) {
    requireCurrentLaunch(isCurrent);
    const popup = window.open(path, request.windowId, "popup,width=1180,height=760,resizable=yes");
    if (!popup) throw new Error("浏览器阻止了串口分析窗口，请允许 PortMate 打开弹出窗口。");
    popup.focus();
    if (!isCurrent()) {
      popup.close();
      throw new Error("串口分析窗口请求已失效。");
    }
    return { close: async () => popup.close() };
  }
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
  await waitForChildWindowReady(child, async () => {
    requireCurrentLaunch(isCurrent);
    await placeAndTrackChildWindow(child, {
      storageKey: serialAnalyzerWindowGeometryKey(request.sessionId),
      width: 1180,
      height: 760,
      minWidth: 720,
      minHeight: 480,
      beforeShow: () => requireCurrentLaunch(isCurrent),
    });
    requireCurrentLaunch(isCurrent);
  }, "创建串口分析窗口超时");
  return { close: () => child.destroy() };
}

function requireCurrentLaunch(isCurrent: () => boolean) {
  if (!isCurrent()) throw new Error("串口分析窗口请求已失效。");
}
