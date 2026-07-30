import React, { lazy, Suspense } from "react";
import ReactDOM from "react-dom/client";
import "./styles.css";
import App from "./App";
import { parseDetachedPaneRequest } from "./detached-pane-state";
import { parseSerialAnalyzerRequest } from "./serial-analyzer-route";
import { parseWorkspaceWindowRequest } from "./workspace-window-route";

const DetachedPaneApp = lazy(() => import("./DetachedPaneApp"));
const SerialAnalyzerApp = lazy(() => import("./SerialAnalyzerApp"));
const detachedPaneRequest = parseDetachedPaneRequest(window.location.search);
const workspaceWindowRequest = detachedPaneRequest ? null : parseWorkspaceWindowRequest(window.location.search);
const serialAnalyzerRequest = detachedPaneRequest || workspaceWindowRequest ? null : parseSerialAnalyzerRequest(window.location.search);
if (detachedPaneRequest) document.body.classList.add("detached-window");
if (serialAnalyzerRequest) document.body.classList.add("serial-analyzer-window");

void loadBundledTerminalFont().finally(() => {
  ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
    <React.StrictMode>
      {detachedPaneRequest ? (
        <Suspense fallback={<div className="detached-pane-loading">正在加载终端...</div>}>
          <DetachedPaneApp request={detachedPaneRequest} />
        </Suspense>
      ) : serialAnalyzerRequest ? (
        <Suspense fallback={<div className="detached-pane-loading">正在加载串口分析器...</div>}>
          <SerialAnalyzerApp request={serialAnalyzerRequest} />
        </Suspense>
      ) : <App workspaceWindowId={workspaceWindowRequest?.windowId} />}
    </React.StrictMode>,
  );
});

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
