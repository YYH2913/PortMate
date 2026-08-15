export class AsyncOperationQueue {
  private tail: Promise<void> = Promise.resolve();

  enqueue<Result>(operation: () => Promise<Result>): Promise<Result> {
    const task = this.tail.then(operation);
    this.tail = task.then(() => undefined, () => undefined);
    return task;
  }
}
