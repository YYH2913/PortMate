import React, { lazy, Suspense } from "react";
import ReactDOM from "react-dom/client";
import "./styles.css";
import App from "./App";
import { parseDetachedPaneRequest } from "./detached-pane-state";

const DetachedPaneApp = lazy(() => import("./DetachedPaneApp"));
const detachedPaneRequest = parseDetachedPaneRequest(window.location.search);
if (detachedPaneRequest) document.body.classList.add("detached-window");

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    {detachedPaneRequest ? (
      <Suspense fallback={<div className="detached-pane-loading">正在加载终端...</div>}>
        <DetachedPaneApp request={detachedPaneRequest} />
      </Suspense>
    ) : <App />}
  </React.StrictMode>,
);
