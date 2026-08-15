import type { WebviewWindow } from "@tauri-apps/api/webviewWindow";

type PendingChildWindow = Pick<WebviewWindow, "destroy" | "once">;

export function waitForChildWindowReady(
  child: PendingChildWindow,
  initialize: () => Promise<void>,
  timeoutMessage: string,
  timeoutMs = 8_000,
): Promise<void> {
  return new Promise((resolve, reject) => {
    let settled = false;
    let created = false;
    let destroyRequested = false;
    let listenersShouldBeRemoved = false;
    const unlisteners = new Set<() => void>();
    const timeout = globalThis.setTimeout(() => {
      if (settled) return;
      settled = true;
      if (created) {
        removeListeners();
        destroyCreatedChild();
      }
      reject(new Error(timeoutMessage));
    }, timeoutMs);

    const register = (listener: Promise<() => void>) => {
      void listener.then((unlisten) => {
        if (listenersShouldBeRemoved) unlisten();
        else unlisteners.add(unlisten);
      }, (error) => fail(error));
    };
    const removeListeners = () => {
      listenersShouldBeRemoved = true;
      for (const unlisten of unlisteners) unlisten();
      unlisteners.clear();
    };
    const destroyCreatedChild = () => {
      if (!created || destroyRequested) return;
      destroyRequested = true;
      void child.destroy().catch(() => {});
    };
    const fail = (error: unknown) => {
      if (settled) {
        destroyCreatedChild();
        return;
      }
      settled = true;
      globalThis.clearTimeout(timeout);
      removeListeners();
      destroyCreatedChild();
      reject(childWindowError(error));
    };

    register(child.once("tauri://created", () => {
      created = true;
      removeListeners();
      if (settled) {
        destroyCreatedChild();
        return;
      }
      void initialize().then(() => {
        if (settled) {
          destroyCreatedChild();
          return;
        }
        settled = true;
        globalThis.clearTimeout(timeout);
        resolve();
      }, fail);
    }));
    register(child.once<unknown>("tauri://error", (event) => {
      removeListeners();
      fail(event.payload);
    }));
  });
}

function childWindowError(error: unknown): Error {
  if (error instanceof Error) return error;
  if (typeof error === "string") return new Error(error);
  try {
    return new Error(JSON.stringify(error));
  } catch {
    return new Error(String(error));
  }
}
