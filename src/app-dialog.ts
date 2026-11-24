export type AppConfirmOptions = {
  title?: string;
  message: string;
  confirmLabel?: string;
  cancelLabel?: string;
  danger?: boolean;
};

export type AppPromptOptions = {
  title?: string;
  message?: string;
  label?: string;
  defaultValue?: string;
  placeholder?: string;
  confirmLabel?: string;
  cancelLabel?: string;
};

export type AppDialogRequest =
  | {
    kind: "confirm";
    id: number;
    options: AppConfirmOptions;
    resolve: (value: boolean) => void;
  }
  | {
    kind: "prompt";
    id: number;
    options: AppPromptOptions;
    resolve: (value: string | null) => void;
  };

const listeners = new Set<() => void>();
let queue: AppDialogRequest[] = [];
let nextId = 1;
let mountedHostCount = 0;

function notify() {
  for (const listener of listeners) listener();
}

export function subscribeAppDialogs(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export function getActiveAppDialog(): AppDialogRequest | null {
  return queue[0] ?? null;
}

export function registerAppDialogHost(): () => void {
  mountedHostCount += 1;
  let registered = true;
  return () => {
    if (!registered) return;
    registered = false;
    if (--mountedHostCount === 0) cancelPendingAppDialogs();
  };
}

export function requestAppConfirm(input: string | AppConfirmOptions): Promise<boolean> {
  const options: AppConfirmOptions = typeof input === "string" ? { message: input } : input;
  if (mountedHostCount === 0) {
    return Promise.resolve(typeof window !== "undefined" && window.confirm?.(options.message) === true);
  }
  return new Promise<boolean>((resolve) => {
    queue = [...queue, { kind: "confirm", id: nextId++, options, resolve }];
    notify();
  });
}

export function requestAppPrompt(options: AppPromptOptions): Promise<string | null> {
  if (mountedHostCount === 0) {
    return Promise.resolve(typeof window === "undefined" ? null
      : window.prompt?.(options.label ?? options.message ?? options.title ?? "", options.defaultValue ?? "") ?? null);
  }
  return new Promise<string | null>((resolve) => {
    queue = [...queue, { kind: "prompt", id: nextId++, options, resolve }];
    notify();
  });
}

export function resolveActiveAppDialog(id: number, result: boolean | string | null) {
  const current = queue[0];
  if (current?.id !== id) return;
  queue = queue.slice(1);
  notify();
  if (current.kind === "confirm") {
    current.resolve(result === true);
  } else {
    current.resolve(typeof result === "string" ? result : null);
  }
}

function cancelPendingAppDialogs() {
  if (!queue.length) return;
  const pending = queue;
  queue = [];
  notify();
  for (const item of pending) {
    if (item.kind === "confirm") item.resolve(false);
    else item.resolve(null);
  }
}
