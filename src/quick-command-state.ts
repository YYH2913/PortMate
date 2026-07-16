export const QUICK_COMMAND_STORAGE_KEY = "portmate.quickCommands.v1";
export const QUICK_BAR_VISIBLE_STORAGE_KEY = "portmate.quickBarVisible.v1";
export const MAX_QUICK_COMMANDS = 64;
export const MAX_QUICK_COMMAND_LABEL_CHARACTERS = 64;
export const MAX_QUICK_COMMAND_TEXT_CHARACTERS = 8_192;

export type QuickCommand = {
  id: string;
  label: string;
  command: string;
  appendEnter: boolean;
};

export type QuickCommandLibrary = {
  version: 1;
  items: QuickCommand[];
};

export type QuickCommandDraft = Omit<QuickCommand, "id">;

export function normalizeQuickCommandLibrary(
  value: unknown,
  createId: () => string = createQuickCommandId,
): QuickCommandLibrary {
  const sourceItems = Array.isArray(value)
    ? value
    : value && typeof value === "object" && !Array.isArray(value) && (value as Record<string, unknown>).version === 1
      ? (value as Record<string, unknown>).items
      : [];
  if (!Array.isArray(sourceItems)) return { version: 1, items: [] };

  const usedIds = new Set<string>();
  const items: QuickCommand[] = [];
  for (const [index, raw] of sourceItems.entries()) {
    if (items.length >= MAX_QUICK_COMMANDS) break;
    if (!raw || typeof raw !== "object" || Array.isArray(raw)) continue;
    const source = raw as Record<string, unknown>;
    const labelSource = typeof source.label === "string" ? source.label : typeof source.name === "string" ? source.name : "";
    const commandSource = typeof source.command === "string" ? source.command : typeof source.text === "string" ? source.text : "";
    const label = normalizeQuickCommandLabel(labelSource);
    const command = normalizeQuickCommandText(commandSource);
    if (!label || !command) continue;
    const preferredId = typeof source.id === "string" && /^[A-Za-z0-9:_-]{1,128}$/.test(source.id) ? source.id : "";
    const id = allocateQuickCommandId(preferredId || createId(), usedIds, index);
    usedIds.add(id);
    items.push({
      id,
      label,
      command,
      appendEnter: typeof source.appendEnter === "boolean" ? source.appendEnter : true,
    });
  }
  return { version: 1, items };
}

export function normalizeQuickCommandLabel(value: string): string {
  return truncateUnicode(value.replace(/\u0000/g, "").trim(), MAX_QUICK_COMMAND_LABEL_CHARACTERS);
}

export function limitQuickCommandLabelInput(value: string): string {
  return truncateUnicode(value.replace(/\u0000/g, ""), MAX_QUICK_COMMAND_LABEL_CHARACTERS);
}

export function normalizeQuickCommandText(value: string): string {
  return truncateUnicode(value.replace(/\u0000/g, ""), MAX_QUICK_COMMAND_TEXT_CHARACTERS);
}

export function createQuickCommand(draft: QuickCommandDraft, id = createQuickCommandId()): QuickCommand | null {
  return normalizeQuickCommandLibrary({ version: 1, items: [{ ...draft, id }] }, () => id).items[0] ?? null;
}

export function quickCommandPayload(command: QuickCommand): string {
  return `${command.command}${command.appendEnter ? "\r" : ""}`;
}

export function quickCommandDispatch(command: QuickCommand): {
  text: string;
  origin: "atomic" | "command";
} {
  return command.appendEnter
    ? { text: command.command, origin: "command" }
    : { text: quickCommandPayload(command), origin: "atomic" };
}

export function moveQuickCommand(items: QuickCommand[], id: string, offset: -1 | 1): QuickCommand[] {
  const index = items.findIndex((item) => item.id === id);
  const target = index + offset;
  if (index < 0 || target < 0 || target >= items.length) return items;
  const next = [...items];
  [next[index], next[target]] = [next[target], next[index]];
  return next;
}

export function createQuickCommandId(): string {
  if (typeof crypto !== "undefined" && "randomUUID" in crypto) return `quick:${crypto.randomUUID()}`;
  return `quick:${Date.now()}:${Math.random().toString(16).slice(2)}`;
}

function allocateQuickCommandId(preferred: string, used: Set<string>, index: number): string {
  const base = (/^[A-Za-z0-9:_-]{1,128}$/.test(preferred) ? preferred : `quick:${index + 1}`).slice(0, 120);
  if (!used.has(base)) return base;
  let suffix = 2;
  while (used.has(`${base}-${suffix}`)) suffix += 1;
  return `${base}-${suffix}`;
}

function truncateUnicode(value: string, maxCharacters: number): string {
  let result = "";
  let count = 0;
  for (const character of value) {
    if (count >= maxCharacters) break;
    result += character;
    count += 1;
  }
  return result;
}
