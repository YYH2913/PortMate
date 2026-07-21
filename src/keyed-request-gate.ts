export class KeyedRequestGate<Key> {
  private readonly active = new Map<Key, number>();
  private nextToken = 0;

  begin(key: Key): number | null {
    if (this.active.has(key)) return null;
    const token = ++this.nextToken;
    this.active.set(key, token);
    return token;
  }

  replace(key: Key): number {
    const token = ++this.nextToken;
    this.active.set(key, token);
    return token;
  }

  isCurrent(key: Key, token: number): boolean {
    return this.active.get(key) === token;
  }

  finish(key: Key, token: number): boolean {
    if (!this.isCurrent(key, token)) return false;
    this.active.delete(key);
    return true;
  }

  invalidate(key: Key) {
    this.active.delete(key);
  }

  invalidateAll() {
    this.active.clear();
  }
}
