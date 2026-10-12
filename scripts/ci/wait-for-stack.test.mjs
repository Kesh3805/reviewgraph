import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'node:net';
import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const SCRIPT = fileURLToPath(new URL('./wait-for-stack.sh', import.meta.url));

function run(args) {
  return new Promise((resolve) => {
    const child = spawn('bash', [SCRIPT, ...args], { stdio: ['ignore', 'pipe', 'pipe'] });
    let out = '';
    child.stdout.on('data', (d) => (out += d));
    child.stderr.on('data', (d) => (out += d));
    child.on('exit', (code) => resolve({ code, out }));
  });
}

test('wait_for_stack_succeeds_when_listener_is_up', async () => {
  const server = createServer().listen(0, '127.0.0.1');
  await new Promise((r) => server.once('listening', r));
  const { port } = server.address();
  const res = await run(['--timeout', '10', `stub=127.0.0.1:${port}`]);
  server.close();
  assert.equal(res.code, 0, res.out);
  assert.match(res.out, /ready: stub/);
});

test('wait_for_stack_times_out_with_service_name', async () => {
  const server = createServer().listen(0, '127.0.0.1');
  await new Promise((r) => server.once('listening', r));
  const { port } = server.address();
  server.close();
  await new Promise((r) => server.once('close', r));
  const res = await run(['--timeout', '2', `deadservice=127.0.0.1:${port}`]);
  assert.equal(res.code, 1);
  assert.match(res.out, /TIMEOUT: service 'deadservice'/);
});
