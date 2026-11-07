import { memo, useSyncExternalStore } from "react";
import type { CSSProperties } from "react";
import { t, useLocale } from "./i18n";
import { formatTerminalTimestampClock } from "./terminal-timestamp-state";
import type { TerminalTimestampViewportStore } from "./terminal-timestamp-viewport";

function TerminalTimestampGutter({ store, transform }: { store: TerminalTimestampViewportStore; transform: string | undefined }) {
  useLocale();
  const viewport = useSyncExternalStore(store.subscribe, store.getSnapshot, store.getSnapshot);
  return <div className="terminal-timestamp-gutter" role="list" aria-label={t("terminal-line-timestamps")}
    data-buffer-type={viewport.bufferType} data-timestamp-count={viewport.entries.length}
    style={{ "--terminal-timestamp-cell-height": `${viewport.cellHeight}px`, transform } as CSSProperties}>
    {viewport.entries.map(entry => <time key={entry.line} role="listitem" dateTime={entry.ts}
      title={timestampTitle(entry.ts)} style={{ "--terminal-timestamp-offset": `${viewport.screenTop + entry.row * viewport.cellHeight}px` } as CSSProperties}>
      {formatTerminalTimestampClock(entry.ts)}
    </time>)}
  </div>;
}

const titleCache = new Map<string, string>();
function timestampTitle(value: string): string {
  const cached = titleCache.get(value);
  if (cached) return cached;
  const date = new Date(value);
  const title = Number.isNaN(date.getTime()) ? value : `${date.toLocaleString()} · ${value}`;
  if (titleCache.size >= 512) titleCache.delete(titleCache.keys().next().value!);
  titleCache.set(value, title);
  return title;
}

export default memo(TerminalTimestampGutter);
