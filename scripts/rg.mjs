#!/usr/bin/env node
// Cross-platform task runner used by the root package.json scripts.
//   node scripts/rg.mjs cargo test --workspace
//   node scripts/rg.mjs run sqlx migrate info --source migrations
//   node scripts/rg.mjs compose dev up -d --wait
//   node scripts/rg.mjs integration          (test stack up → integration tests → down -v)
// Children are spawned without a shell, so arguments are passed through verbatim.
import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import { existsSync } from 'node:fs';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');

export const COMPOSE_FILES = {
  dev: 'infra/compose/docker-compose.yml',
  test: 'infra/compose/docker-compose.test.yml',
};

/**
 * Bash to run the engine scripts with. On Windows a bare `bash.exe` may resolve to WSL's
 * System32 shim, which cannot see Docker Desktop's pipe the same way, so prefer Git Bash.
 */
export function bashFor(platform = process.platform, env = process.env, exists = existsSync) {
  if (env.RG_BASH) return env.RG_BASH;
  if (platform !== 'win32') return 'bash';
  const candidates = [
    path.join(env.ProgramFiles ?? 'C:\\Program Files', 'Git', 'bin', 'bash.exe'),
    path.join(env.LOCALAPPDATA ?? '', 'Programs', 'Git', 'bin', 'bash.exe'),
  ];
  return candidates.find((c) => exists(c)) ?? 'bash.exe';
}

/** Resolve a logical command to [executable, args]. `platform` is injectable for tests. */
export function resolve(cmd, args, platform = process.platform) {
  switch (cmd) {
    case 'cargo':
    case 'run': {
      const script = cmd === 'cargo' ? 'engine/scripts/cargo.sh' : 'engine/scripts/run.sh';
      return [bashFor(platform), [script, ...args]];
    }
    case 'compose': {
      const [stack, ...rest] = args;
      const file = COMPOSE_FILES[stack];
      if (!file) throw new Error(`unknown compose stack "${stack}" (expected dev|test)`);
      return ['docker', ['compose', '-f', file, ...rest]];
    }
    default:
      throw new Error(`unknown command "${cmd}"`);
  }
}

export function exec(exe, args, env = {}) {
  return new Promise((resolvePromise) => {
    const child = spawn(exe, args, {
      cwd: ROOT,
      stdio: 'inherit',
      shell: false,
      env: { ...process.env, ...env },
    });
    child.on('error', (err) => {
      console.error(`failed to start ${exe}: ${err.message}`);
      resolvePromise(127);
    });
    child.on('exit', (code, signal) => resolvePromise(code ?? (signal ? 128 : 1)));
  });
}

async function integration(extra) {
  const up = await exec(...resolve('compose', ['test', 'up', '-d', '--wait']));
  if (up !== 0) return up;
  try {
    return await exec(
      ...resolve('cargo', ['test', '--workspace', '--features', 'integration', ...extra]),
      {
        TEST_DATABASE_URL: 'postgres://reviewgraph:reviewgraph-test@host.docker.internal:35432/reviewgraph',
        QDRANT_URL: 'http://host.docker.internal:36333',
        REDIS_URL: 'redis://host.docker.internal:36379',
      },
    );
  } finally {
    await exec(...resolve('compose', ['test', 'down', '-v']));
  }
}

async function main() {
  const [cmd, ...args] = process.argv.slice(2);
  if (!cmd) {
    console.error('usage: rg.mjs <cargo|run|compose|integration> [...args]');
    process.exit(2);
  }
  const code = cmd === 'integration' ? await integration(args) : await exec(...resolve(cmd, args));
  process.exit(code);
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main();
}
