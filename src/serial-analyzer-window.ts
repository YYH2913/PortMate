import { t } from "./i18n";
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
    if (!popup) throw new Error(t("the-browser-blocked-the-serial-analyzer-window-allow-portmate"));
    popup.focus();
    if (!isCurrent()) {
      popup.close();
      throw new Error(t("the-serial-analyzer-window-request-is-stale"));
    }
    return { close: async () => popup.close() };
  }
  const child = new WebviewWindow(request.windowId, {
    url: path,
    title: t("portmate-serial-analyzer", [sessionName]),
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
  }, t("serial-analyzer-window-creation-timed-out"));
  return { close: () => child.destroy() };
}

function requireCurrentLaunch(isCurrent: () => boolean) {
  if (!isCurrent()) throw new Error(t("the-serial-analyzer-window-request-is-stale"));
}
