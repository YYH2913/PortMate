type MessagePart = string | { index: number; placeholder: string };
const compiledMessages = new Map<string, readonly MessagePart[]>();
const MAX_COMPILED_MESSAGES = 512;

/** Cache only application-owned templates; values stay verbatim and uncached. */
export function formatTranslatedMessage(template: string, values: readonly unknown[], cacheTemplate = true): string {
  if (!template.includes("{")) return template;
  let parts = cacheTemplate ? compiledMessages.get(template) : undefined;
  if (!parts) {
    const next: MessagePart[] = [];
    let end = 0;
    for (const match of template.matchAll(/\{(\d+)\}/g)) {
      next.push(template.slice(end, match.index));
      next.push({ index: Number(match[1]), placeholder: match[0] });
      end = match.index + match[0].length;
    }
    next.push(template.slice(end));
    parts = next;
    if (cacheTemplate && template.length <= 4096) {
      if (compiledMessages.size >= MAX_COMPILED_MESSAGES) compiledMessages.delete(compiledMessages.keys().next().value!);
      compiledMessages.set(template, parts);
    }
  }
  let result = "";
  for (const part of parts) {
    result += typeof part === "string" ? part
      : part.index < values.length ? String(values[part.index] ?? "") : part.placeholder;
  }
  return result;
}
