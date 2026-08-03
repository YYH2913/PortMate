import { describe, expect, it } from "vitest";
import { findNativeKeyringDependencyViolations } from "./native-keyring-dependency-check.mjs";

const providers = [
  {
    name: "dbus-secret-service-keyring-store",
    target: 'cfg(any(target_os = "linux", target_os = "freebsd"))',
  },
  { name: "apple-native-keyring-store", target: 'cfg(target_os = "macos")' },
  { name: "windows-native-keyring-store", target: "cfg(windows)" },
];

function cleanMetadata() {
  return {
    workspace_members: ["portmate 0.1.0", "portmate-keyring 0.1.0", "portmate-mcp 0.1.0"],
    packages: [
      {
        id: "portmate 0.1.0",
        name: "portmate",
        dependencies: [{ name: "keyring-core" }, { name: "portmate-keyring" }],
      },
      {
        id: "portmate-keyring 0.1.0",
        name: "portmate-keyring",
        dependencies: [{ name: "keyring-core" }, ...providers],
      },
      {
        id: "portmate-mcp 0.1.0",
        name: "portmate-mcp",
        dependencies: [{ name: "keyring-core" }, { name: "portmate-keyring" }],
      },
    ],
  };
}

describe("native keyring dependency boundary", () => {
  it("accepts the shared platform-specific provider boundary", () => {
    expect(findNativeKeyringDependencyViolations(cleanMetadata())).toEqual([]);
  });

  it("rejects the upstream sample aggregate and its storage stack", () => {
    const metadata = cleanMetadata();
    metadata.packages.push(
      { id: "keyring 4.0.1", name: "keyring", dependencies: [] },
      { id: "turso 0.6.0", name: "turso", dependencies: [] },
    );
    expect(findNativeKeyringDependencyViolations(metadata)).toEqual([
      "forbidden package is present in the resolved graph: keyring",
      "forbidden package is present in the resolved graph: turso",
    ]);
  });

  it("rejects callers that bypass the shared provider selection", () => {
    const metadata = cleanMetadata();
    const desktop = metadata.packages.find((entry) => entry.name === "portmate");
    desktop.dependencies = [{ name: "dbus-secret-service-keyring-store" }];
    expect(findNativeKeyringDependencyViolations(metadata)).toEqual([
      "portmate must depend on portmate-keyring",
      "portmate must not depend directly on dbus-secret-service-keyring-store",
    ]);
  });

  it("rejects providers without their exact platform target", () => {
    const metadata = cleanMetadata();
    const shared = metadata.packages.find((entry) => entry.name === "portmate-keyring");
    shared.dependencies.find(
      (dependency) => dependency.name === "windows-native-keyring-store",
    ).target = undefined;
    expect(findNativeKeyringDependencyViolations(metadata)).toEqual([
      "windows-native-keyring-store target changed: expected cfg(windows), got all targets",
    ]);
  });
});
