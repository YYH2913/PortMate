import React, { lazy, Suspense } from "react";
import ReactDOM from "react-dom/client";
import "./styles.css";
import App from "./App";
import { parseDetachedPaneRequest } from "./detached-pane-state";
import { parseSerialAnalyzerRequest } from "./serial-analyzer-route";
import { parseWorkspaceWindowRequest } from "./workspace-window-route";
import { useModalInteractionBoundary } from "./modal-interaction-boundary";
import { isBackendAvailable } from "./api";
import { listenTerminalByteEvents } from "./terminal-byte-events";

const DetachedPaneApp = lazy(() => import("./DetachedPaneApp"));
const SerialAnalyzerApp = lazy(() => import("./SerialAnalyzerApp"));
const detachedPaneRequest = parseDetachedPaneRequest(window.location.search);
const workspaceWindowRequest = detachedPaneRequest ? null : parseWorkspaceWindowRequest(window.location.search);
const serialAnalyzerRequest = detachedPaneRequest || workspaceWindowRequest ? null : parseSerialAnalyzerRequest(window.location.search);
if (detachedPaneRequest) document.body.classList.add("detached-window");
if (serialAnalyzerRequest) document.body.classList.add("serial-analyzer-window");

if (isBackendAvailable()) {
  void listenTerminalByteEvents()
    .then((unlisten) => window.addEventListener("unload", unlisten, { once: true }))
    .catch(() => {});
}

void loadBundledTerminalFont().finally(() => {
  ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
    <React.StrictMode>
      <PortMateRoute />
    </React.StrictMode>,
  );
});

function PortMateRoute() {
  useModalInteractionBoundary();
  if (detachedPaneRequest) {
    return (
      <Suspense fallback={<div className="detached-pane-loading">正在加载终端...</div>}>
        <DetachedPaneApp request={detachedPaneRequest} />
      </Suspense>
    );
  }
  if (serialAnalyzerRequest) {
    return (
      <Suspense fallback={<div className="detached-pane-loading">正在加载串口分析器...</div>}>
        <SerialAnalyzerApp request={serialAnalyzerRequest} />
      </Suspense>
    );
  }
  return <App workspaceWindowId={workspaceWindowRequest?.windowId} />;
}

async function loadBundledTerminalFont() {
  try {
    const faces = await document.fonts.load(
      '400 13px "JetBrains Mono"',
      "PortMate 0O1l []{} ─│┌┐└┘",
    );
    document.documentElement.dataset.bundledTerminalFont = faces.length > 0 ? "loaded" : "fallback";
  } catch {
    document.documentElement.dataset.bundledTerminalFont = "fallback";
  }
}
