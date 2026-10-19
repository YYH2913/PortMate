import { describe, expect, it } from "vitest";
import { nativeRustTestArguments } from "../tooling/native-rust-tests.mjs";
import { nativeTestDiagnosticBuildArguments, nativeTestExecutables } from "../tooling/windows-loader-diagnostics.mjs";

describe("Windows native loader diagnostics", () => {
  it("selects only executable test artifacts and deduplicates cache messages", () => {
    const artifact = { reason: "compiler-artifact", profile: { test: true }, executable: "C:\\build path\\test.exe" };
    const messages = [
      "not json", JSON.stringify(artifact), JSON.stringify(artifact),
      JSON.stringify({ ...artifact, profile: { test: false } }),
      JSON.stringify({ ...artifact, executable: "C:\\build\\library.dll" }),
      JSON.stringify({ ...artifact, reason: "build-script-executed" }),
      JSON.stringify({ ...artifact, executable: null }),
    ].join("\r\n");
    expect(nativeTestExecutables(messages)).toEqual([artifact.executable]);
  });

  it.each(["release-upgrade", "portable-vault", "workspace"])("keeps %s build settings but removes harness-only arguments", (suite) => {
    const original = nativeRustTestArguments(suite, "win32");
    const args = nativeTestDiagnosticBuildArguments(original);
    expect(args).toEqual([
      ...original.slice(0, original.indexOf("--")),
      "--no-run", "--message-format=json-render-diagnostics",
    ]);
    expect(args).toContain("--release");
    expect(args).toContain("--locked");
    expect(args).not.toContain("--test-threads=1");
  });
});
