import type { TriggerAction } from "./types";

export function defaultTriggerAction(type: TriggerAction["type"]): TriggerAction {
  switch (type) {
    case "notification":
      return { type, message: "触发器命中" };
    case "highlight":
      return { type, color: "#f4b860" };
    case "send-text":
      return { type, text: "" };
    case "local-command":
      return { type, command: "" };
    case "custom-link":
      return { type, url_template: "https://www.google.com/search?q={text}" };
    case "sound":
      return { type, name: "bell" };
    case "timeline-mark":
      return { type, label: "mark" };
  }
}

export function patchTriggerAction(type: TriggerAction["type"], value: string): TriggerAction {
  switch (type) {
    case "notification":
      return { type, message: value };
    case "highlight":
      return { type, color: value };
    case "send-text":
      return { type, text: value };
    case "local-command":
      return { type, command: value };
    case "custom-link":
      return { type, url_template: value };
    case "sound":
      return { type, name: value };
    case "timeline-mark":
      return { type, label: value };
  }
}

export function triggerActionValue(action: TriggerAction): string {
  switch (action.type) {
    case "notification":
      return action.message;
    case "highlight":
      return action.color;
    case "send-text":
      return action.text;
    case "local-command":
      return action.command;
    case "custom-link":
      return action.url_template;
    case "sound":
      return action.name;
    case "timeline-mark":
      return action.label;
  }
}
