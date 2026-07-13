import { describe, expect, it } from "vitest";
import type { TriggerAction } from "./types";
import { defaultTriggerAction, patchTriggerAction, triggerActionValue } from "./trigger-state";

describe("trigger action state", () => {
  const types: TriggerAction["type"][] = [
    "timeline-mark",
    "notification",
    "highlight",
    "send-text",
    "local-command",
    "custom-link",
    "sound",
  ];

  it("creates a valid default for every action type", () => {
    expect(types.map((type) => defaultTriggerAction(type).type)).toEqual(types);
    expect(defaultTriggerAction("custom-link")).toEqual({
      type: "custom-link",
      url_template: "https://www.google.com/search?q={text}",
    });
  });

  it("round-trips each action parameter without changing its type", () => {
    for (const type of types) {
      const action = patchTriggerAction(type, `value-${type}`);
      expect(action.type).toBe(type);
      expect(triggerActionValue(action)).toBe(`value-${type}`);
    }
  });
});
