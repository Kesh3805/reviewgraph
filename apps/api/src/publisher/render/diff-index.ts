export interface DiffLine {
  kind: 'add' | 'del' | 'ctx';
  oldLine?: number;
  newLine?: number;
}

export interface Hunk {
  oldStart: number;
  oldLines: number;
  newStart: number;
  newLines: number;
  lines: DiffLine[];
}

const HUNK_HEADER = /^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@/;

/** Parses the unified-diff `patch` of one file into hunks with per-line old/new numbers. */
export function parsePatch(patch: string): Hunk[] {
  const hunks: Hunk[] = [];
  let current: Hunk | undefined;
  let oldLine = 0;
  let newLine = 0;
  for (const raw of patch.split('\n')) {
    const header = HUNK_HEADER.exec(raw);
    if (header) {
      current = {
        oldStart: Number(header[1]),
        oldLines: header[2] === undefined ? 1 : Number(header[2]),
        newStart: Number(header[3]),
        newLines: header[4] === undefined ? 1 : Number(header[4]),
        lines: [],
      };
      oldLine = current.oldStart;
      newLine = current.newStart;
      hunks.push(current);
      continue;
    }
    if (!current) continue;
    const marker = raw.charAt(0);
    if (marker === '+') current.lines.push({ kind: 'add', newLine: newLine++ });
    else if (marker === '-') current.lines.push({ kind: 'del', oldLine: oldLine++ });
    else if (marker === ' ')
      current.lines.push({ kind: 'ctx', oldLine: oldLine++, newLine: newLine++ });
    // `\ No newline at end of file` and blank trailer lines carry no line number.
  }
  return hunks;
}

export interface DiffFileInput {
  path: string;
  patch?: string;
}

/** Commentable lines per file, built from the provider's changed files (or stored hunks). */
export class DiffIndex {
  private constructor(private readonly files: Map<string, Hunk[]>) {}

  static build(files: readonly DiffFileInput[]): DiffIndex {
    const map = new Map<string, Hunk[]>();
    for (const f of files) if (f.patch) map.set(f.path, parsePatch(f.patch));
    return new DiffIndex(map);
  }

  static fromHunks(files: Record<string, Hunk[]>): DiffIndex {
    return new DiffIndex(new Map(Object.entries(files)));
  }

  hunks(path: string): Hunk[] {
    return this.files.get(path) ?? [];
  }
}
