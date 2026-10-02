import { test } from 'node:test';
import assert from 'node:assert/strict';
import { resolve, exec, bashFor } from './rg.mjs';
import { readFileSync } from 'node:fs';

test('prefers Git Bash on win32 and never the WSL shim', () => {
  const env = { ProgramFiles: 'C:\\Program Files' };
  const gitBash = bashFor('win32', env, (p) => p.includes('Git'));
  assert.match(gitBash, /Git[\\/]bin[\\/]bash\.exe$/);
  assert.equal(
    bashFor('win32', {}, () => false),
    'bash.exe',
  );
  assert.equal(bashFor('win32', { RG_BASH: 'X:\\bash.exe' }), 'X:\\bash.exe');
});

test('selects bash elsewhere and passes script args through', () => {
  const [exe, args] = resolve('cargo', ['test', '--workspace'], 'linux');
  assert.equal(exe, 'bash');
  assert.deepEqual(args, ['engine/scripts/cargo.sh', 'test', '--workspace']);
});

test('maps compose stacks to files and rejects unknown stacks', () => {
  assert.deepEqual(resolve('compose', ['test', 'up']), [
    'docker',
    ['compose', '-f', 'infra/compose/docker-compose.test.yml', 'up'],
  ]);
  assert.throws(() => resolve('compose', ['prod', 'up']), /unknown compose stack/);
});

test('propagates child exit code', async () => {
  assert.equal(await exec(process.execPath, ['-e', 'process.exit(7)']), 7);
});

test('does not use a shell', () => {
  const src = readFileSync(new URL('./rg.mjs', import.meta.url), 'utf8');
  assert.match(src, /shell: false/);
  assert.doesNotMatch(src, /shell: true/);
});
