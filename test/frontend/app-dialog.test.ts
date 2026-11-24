import { describe, expect, it } from "vitest";
import {
  getActiveAppDialog,
  registerAppDialogHost,
  requestAppConfirm,
  requestAppPrompt,
  resolveActiveAppDialog,
} from "../../src/app-dialog";

describe("in-app confirm and prompt dialog queue", () => {
  it("queues confirm and prompt requests FIFO when a host is mounted", async () => {
    const unregister = registerAppDialogHost();
    try {
      const first = requestAppConfirm({ title: "Delete", message: "Delete item?", danger: true });
      const second = requestAppPrompt({ title: "Rename", label: "Name", defaultValue: "edge" });

      expect(getActiveAppDialog()?.kind).toBe("confirm");
      expect(getActiveAppDialog()?.options.message).toBe("Delete item?");
      const firstId = getActiveAppDialog()!.id;
      resolveActiveAppDialog(firstId, true);
      expect(await first).toBe(true);

      const next = getActiveAppDialog();
      expect(next?.kind).toBe("prompt");
      expect(next?.kind === "prompt" ? next.options.defaultValue : "").toBe("edge");
      resolveActiveAppDialog(firstId, false); // stale submit/cancel from the previous dialog
      expect(getActiveAppDialog()).toBe(next);
      resolveActiveAppDialog(next!.id, "edge-renamed");
      expect(await second).toBe("edge-renamed");
      expect(getActiveAppDialog()).toBeNull();
    } finally {
      unregister();
    }
  });

  it("resolves pending dialogs cleanly as cancelled when the last host unmounts", async () => {
    const unregister = registerAppDialogHost();
    try {
      const confirmPromise = requestAppConfirm("Proceed?");
      const promptPromise = requestAppPrompt({ label: "Folder" });
      const other = registerAppDialogHost();
      other();
      other();
      expect(getActiveAppDialog()).not.toBeNull();
      unregister();
      expect(await confirmPromise).toBe(false);
      expect(await promptPromise).toBeNull();
      expect(getActiveAppDialog()).toBeNull();
    } finally {
      unregister();
    }
  });
});
