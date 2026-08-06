import { describe, expect, it, vi } from "vitest";
import { runDeniedWindowsCredentialManagerProbe } from "./native-keyring-check.mjs";

describe("Windows native keyring denial probe", () => {
  it("writes a real credential before anonymous access and cleans it afterward", () => {
    const environment = { probe: "environment" };
    const phases = [];

    runDeniedWindowsCredentialManagerProbe(environment, (phase, actualEnvironment) => {
      phases.push(phase);
      expect(actualEnvironment).toBe(environment);
    });

    expect(phases).toEqual(["write", "verify-denied", "cleanup"]);
  });

  it("cleans the credential when the denial assertion fails", () => {
    const denialFailure = new Error("anonymous token unexpectedly read a credential");
    const runPhase = vi.fn((phase) => {
      if (phase === "verify-denied") throw denialFailure;
    });

    expect(() => runDeniedWindowsCredentialManagerProbe({}, runPhase)).toThrow(denialFailure);
    expect(runPhase.mock.calls.map(([phase]) => phase)).toEqual([
      "write",
      "verify-denied",
      "cleanup",
    ]);
  });

  it("reports both the probe and cleanup failures without hiding either", () => {
    const runPhase = vi.fn((phase) => {
      if (phase === "verify-denied") throw new Error("denial assertion failed");
      if (phase === "cleanup") throw new Error("cleanup failed");
    });

    expect(() => runDeniedWindowsCredentialManagerProbe({}, runPhase)).toThrow(
      "denial assertion failed\nWindows credential cleanup also failed: cleanup failed",
    );
  });
});
