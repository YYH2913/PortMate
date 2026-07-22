import { describe, expect, it } from "vitest";
import type { TriggerAction } from "./types";
import {
  canAddTrigger,
  canAddTriggerAction,
  defaultTriggerAction,
  MAX_TRIGGER_ACTIONS,
  MAX_TRIGGERS_PER_PROFILE,
  patchTriggerAction,
  triggerActionValue,
} from "./trigger-state";

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

  it("stops adding rules and actions at the shared profile bounds", () => {
    expect(canAddTrigger(MAX_TRIGGERS_PER_PROFILE - 1)).toBe(true);
    expect(canAddTrigger(MAX_TRIGGERS_PER_PROFILE)).toBe(false);
    expect(canAddTriggerAction(MAX_TRIGGER_ACTIONS - 1)).toBe(true);
    expect(canAddTriggerAction(MAX_TRIGGER_ACTIONS)).toBe(false);
    expect(canAddTrigger(-1)).toBe(false);
    expect(canAddTriggerAction(1.5)).toBe(false);
  });
});
