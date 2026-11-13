/** Candidate membership, not visibility, is cached. Drain pending mutations on
 * reads so a dialog inserted in this very task blocks the next input event. */
export class InteractionLayerIndex {
  private readonly candidates = new Set<HTMLElement>();
  private readonly listeners = new Set<() => void>();
  private readonly observer: MutationObserver;

  constructor(private readonly root: Document, private readonly selector: string) {
    this.observer = new MutationObserver(records => this.update(records));
    this.observer.observe(root, {
      childList: true, subtree: true, attributes: true,
      attributeFilter: ["class", "hidden", "style", "aria-hidden"],
    });
    for (const element of root.querySelectorAll<HTMLElement>(selector)) this.candidates.add(element);
  }

  elements(): readonly HTMLElement[] {
    this.update(this.observer.takeRecords());
    return [...this.candidates].sort((left, right) => left === right ? 0
      : left.compareDocumentPosition(right) & Node.DOCUMENT_POSITION_FOLLOWING ? -1 : 1);
  }

  subscribe(listener: () => void): () => void {
    this.listeners.add(listener);
    return () => { this.listeners.delete(listener); };
  }

  private update(records: MutationRecord[]): void {
    if (!records.length) return;
    const hadLayers = this.candidates.size > 0;
    for (const record of records) {
      if (record.type === "attributes") {
        const target = record.target as HTMLElement;
        if (record.attributeName === "class") {
          if (target.matches(this.selector) && this.root.contains(target)) this.candidates.add(target);
          else this.candidates.delete(target);
        }
        continue;
      }
      for (const node of record.removedNodes) {
        for (const layer of this.candidates) if (node.contains(layer)) this.candidates.delete(layer);
      }
      for (const node of record.addedNodes) {
        if (!(node instanceof HTMLElement) || !this.root.contains(node)) continue;
        if (node.matches(this.selector)) this.candidates.add(node);
        // Echoed text and leaf spans cannot contain dialogs. Never rescan the
        // whole workspace for ordinary xterm/cursor/timestamp DOM changes.
        if (node.childElementCount) {
          for (const layer of node.querySelectorAll<HTMLElement>(this.selector)) this.candidates.add(layer);
        }
      }
    }
    // With a modal open, structural changes can add non-inert siblings or
    // focusable controls. Preserve isolation/focus resynchronization then.
    if (hadLayers || this.candidates.size) for (const listener of this.listeners) listener();
  }
}
