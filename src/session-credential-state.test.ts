import { describe, expect, it, vi } from "vitest";
import {
  runtimeCredentialsForStaging,
  stageConnectionCredentials,
} from "./session-credential-state";

describe("session credential staging", () => {
  it("does not send fields that were persisted before opening", () => {
    expect(runtimeCredentialsForStaging({
      password: "saved-password",
      passphrase: "runtime-passphrase",
      savePassword: true,
      savePassphrase: false,
    })).toEqual({ password: null, passphrase: "runtime-passphrase" });
  });

  it("returns null without invoking the backend when no runtime secret remains", async () => {
    const invoke = vi.fn();
    await expect(stageConnectionCredentials(invoke, "ssh-1", {
      password: "saved-password",
      passphrase: null,
      savePassword: true,
      savePassphrase: false,
    })).resolves.toBeNull();
    expect(invoke).not.toHaveBeenCalled();
  });

  it("exchanges runtime secrets for an opaque handle exactly once", async () => {
    const invoke = vi.fn().mockResolvedValue({
      credentialHandle: "session-credential:opaque",
      expiresInMs: 30_000,
    });
    await expect(stageConnectionCredentials(invoke, "ssh-1", {
      password: "runtime-password",
      passphrase: null,
      savePassword: false,
      savePassphrase: false,
    })).resolves.toBe("session-credential:opaque");
    expect(invoke).toHaveBeenCalledTimes(1);
    expect(invoke).toHaveBeenCalledWith("stage_session_credentials", {
      request: {
        sessionId: "ssh-1",
        password: "runtime-password",
        passphrase: null,
      },
    });
  });
});
