export type DiagnosticTemplate = {
  id: string;
  source: string;
  causeIndices?: readonly number[];
};

type Pattern = DiagnosticTemplate & { parts: string[]; indices: number[] };
type MessageTranslator = (id: string, values?: readonly unknown[]) => string;

/**
 * A presentation-only adapter for application-owned native diagnostics.
 * It never edits error state or translates unrecognized OS/device output.
 * Literal matching is bounded, without backtracking regular expressions.
 */
export function createDiagnosticLocalizer(templates: readonly DiagnosticTemplate[], translate: MessageTranslator) {
  const exact = new Map<string, string>();
  const buckets = new Map<string, Pattern[]>();
  for (const template of templates) {
    const matches = [...template.source.matchAll(/\{(\d+)\}/g)];
    if (!matches.length) { exact.set(template.source, template.id); continue; }
    const parts = template.source.split(/\{\d+\}/g);
    // Adjacent values cannot be recovered unambiguously from a formatted error.
    if (parts.slice(1, -1).some(part => !part)) continue;
    const prefix = parts[0].slice(0, 2);
    const pattern = { ...template, parts, indices: matches.map(match => Number(match[1])) };
    const items = buckets.get(prefix) ?? [];
    items.push(pattern);
    buckets.set(prefix, items);
  }
  for (const items of buckets.values()) items.sort((a, b) => b.parts[0].length - a.parts[0].length);

  function localize(message: string, depth = 0): string {
    if (message.length > 8_192 || depth > 3) return message;
    const id = exact.get(message);
    if (id) return translate(id);
    const patterns = [
      ...(buckets.get(message.slice(0, 2)) ?? []),
      ...(buckets.get(message.slice(0, 1)) ?? []),
      ...(buckets.get("") ?? []),
    ];
    for (const pattern of patterns) {
      const captures = matchLiteralTemplate(message, pattern.parts);
      if (!captures) continue;
      const values: string[] = [];
      for (let index = 0; index < captures.length; index++) {
        const parameter = pattern.indices[index];
        const raw = captures[index];
        values[parameter] = pattern.causeIndices?.includes(parameter) ? localize(raw, depth + 1) : raw;
      }
      return translate(pattern.id, values);
    }
    // Multi-line diagnostic bundles may contain independently recognized errors.
    if (message.includes("\n")) return message.split("\n").map(line => localize(line, depth + 1)).join("\n");
    return message;
  }
  return (message: string): string => localize(message);
}

function matchLiteralTemplate(message: string, parts: readonly string[]): string[] | null {
  if (!message.startsWith(parts[0]) || !message.endsWith(parts.at(-1)!)) return null;
  let cursor = parts[0].length;
  const values: string[] = [];
  for (let index = 1; index < parts.length; index++) {
    const delimiter = parts[index];
    if (index === parts.length - 1) {
      const end = message.length - delimiter.length;
      if (end < cursor) return null;
      values.push(message.slice(cursor, end));
      cursor = message.length;
      continue;
    }
    const end = message.indexOf(delimiter, cursor);
    if (end < 0 || message.indexOf(delimiter, end + delimiter.length) !== -1) return null;
    values.push(message.slice(cursor, end));
    cursor = end + delimiter.length;
  }
  return values;
}
