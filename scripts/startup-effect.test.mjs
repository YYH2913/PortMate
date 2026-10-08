import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { stripTypeScriptTypes } from "node:module";
import { parse } from "@babel/parser";
import vm from "node:vm";
import { startupCredentialsReady } from "../src/startup-connection-state";
import { createSshConnection } from "../src/session-profile-helpers";

describe("current App startup effect", () => {
  it("keeps a locked vault target pending and starts it once after unlock", async () => {
    const source = readFileSync(new URL("../src/App.tsx", import.meta.url), "utf8");
    const ast = parse(source, { sourceType: "module", plugins: ["typescript", "jsx"] });
    const matches = [];
    function walk(node) {
      if (!node || typeof node !== "object") return;
      if (node.type === "CallExpression" && node.callee?.name === "useEffect") {
        const callback = node.arguments[0];
        if (callback?.type === "ArrowFunctionExpression") {
          const text = source.slice(callback.start, callback.end);
          if (text.includes("startupAppliedRef.current") && text.includes("resolveStartupSessionIds")) matches.push(callback);
        }
      }
      for (const value of Object.values(node)) {
        if (Array.isArray(value)) value.forEach(walk);
        else if (value?.type) walk(value);
      }
    }
    walk(ast);
    expect(matches).toHaveLength(1);
    const callback = source.slice(matches[0].start, matches[0].end);
    const connection = createSshConnection();
    connection.passwordSecretRef = "stronghold:password";
    const attempts = [];
    const context = vm.createContext({
      workspaceWindowId: "", startupAppliedRef: { current: false }, pendingStartupRef: { current: null },
      portableVaultStatusReady: true, portableVaultStatus: { exists: true, unlocked: false },
      sessions: [{ profile: { id: "ssh", kind: "ssh", connection }, runtime: { status: "disconnected" } }],
      terminalPrefs: { startupMode: "last", startupSessions: [] }, workspaceRoot: {}, activePaneId: "pane", activeId: "ssh", tabColors: {},
      reconcileWorkspaceSnapshot: value => value, resolveStartupSessionIds: () => ["ssh"],
      sessionConnectionAction: () => "connect", startupCredentialsReady,
      connectSession: async id => { attempts.push(id); },
    });
    const effect = vm.runInContext(stripTypeScriptTypes(`(${callback})`, { mode: "strip" }), context);
    effect();
    await new Promise(resolve => setImmediate(resolve));
    expect(attempts).toEqual([]);
    expect(context.startupAppliedRef.current).toBe(false);
    context.portableVaultStatus.unlocked = true;
    effect();
    await new Promise(resolve => setImmediate(resolve));
    effect();
    expect(attempts).toEqual(["ssh"]);
    expect(context.startupAppliedRef.current).toBe(true);
  });
});
