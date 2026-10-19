import { describe, expect, it, vi } from "vitest";
import {
  cleanupMacOSKeychainProbe,
  nativeKeyringProbeArguments,
  runDeniedWindowsCredentialManagerProbe,
} from "../tooling/native-keyring-check.mjs";

describe("native keyring probe build profile", () => {
  it.each(["linux", "darwin", "win32"])("reuses headless native artifacts on %s and retains exact phases", platform => {
    const phase = ["--phase", "verify-denied"];
    expect(nativeKeyringProbeArguments(phase, platform)).toEqual([
      "run", "--locked", "-p", "portmate", "--no-default-features",
      ...(platform === "win32" ? ["--release"] : []),
      "--bin", "native-keyring-probe", "--", ...phase,
    ]);
    expect(phase).toEqual(["--phase", "verify-denied"]);
    expect(nativeKeyringProbeArguments([], platform).at(-1)).toBe("--");
  });
});

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

describe("macOS native keychain cleanup", () => {
  it("continues cleanup after restoring the default keychain fails", () => {
    const calls = [];
    const runCommand = vi.fn((command, args) => {
      calls.push([command, args[0]]);
      if (args[0] === "default-keychain") throw new Error("default keychain is unavailable");
    });
    const runPhase = vi.fn((phase) => calls.push(["phase", phase]));
    const removeDirectory = vi.fn((path) => calls.push(["remove", path]));

    const failures = cleanupMacOSKeychainProbe({
      keychainCreated: true,
      defaultReplaced: true,
      keychain: "/tmp/PortMateProbe.keychain-db",
      originalDefault: "/Users/test/Library/Keychains/login.keychain-db",
      password: "probe-password",
      environment: { probe: "environment" },
      root: "/tmp/portmate-native-keychain",
      runCommand,
      runPhase,
      removeDirectory,
    });

    expect(calls).toEqual([
      ["security", "unlock-keychain"],
      ["phase", "cleanup"],
      ["security", "default-keychain"],
      ["security", "delete-keychain"],
      ["remove", "/tmp/portmate-native-keychain"],
    ]);
    expect(failures).toEqual([
      "restore default keychain: default keychain is unavailable",
    ]);
  });

  it("removes the temporary directory even when no keychain was created", () => {
    const removeDirectory = vi.fn();
    expect(cleanupMacOSKeychainProbe({
      keychainCreated: false,
      defaultReplaced: false,
      keychain: "/tmp/unused.keychain-db",
      originalDefault: "/tmp/original.keychain-db",
      password: "probe-password",
      environment: {},
      root: "/tmp/portmate-native-keychain",
      removeDirectory,
    })).toEqual([]);
    expect(removeDirectory).toHaveBeenCalledWith("/tmp/portmate-native-keychain");
  });
});
