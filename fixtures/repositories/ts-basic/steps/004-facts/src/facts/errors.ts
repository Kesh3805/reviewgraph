export class ImportFailedError extends Error {}

export class Importer {
  private opened = false;

  run(path: string): number {
    try {
      return this.load(path);
    } catch (error) {
      throw new ImportFailedError(path);
    } finally {
      this.close();
    }
  }

  tryQuietly(path: string): void {
    try {
      this.load(path);
    } catch {
      // ignored on purpose
    }
  }

  fail(reason: string): never {
    throw reason;
  }

  load(path: string): number {
    this.opened = true;
    return path.length;
  }

  close(): void {
    this.opened = false;
  }
}
