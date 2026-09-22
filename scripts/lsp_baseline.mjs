// Measure the actual stdio server with its normal allocator. Run serially:
// node scripts/lsp_baseline.mjs <splitls> <fixture.split> [samples=50]
// Use the same source file and CPU affinity for both revisions being compared.
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import { readFile } from 'node:fs/promises';
import { basename, resolve } from 'node:path';
import { performance } from 'node:perf_hooks';

const [binary, fixturePath, sampleArgument = '50', ...extra] = process.argv.slice(2);
const samples = Number(sampleArgument);
assert(binary && fixturePath && extra.length === 0,
  'usage: node scripts/lsp_baseline.mjs <splitls> <fixture.split> [samples=50]');
assert(Number.isSafeInteger(samples) && samples > 0, 'samples must be a positive integer');
const warmups = 20;
const header = name => `struct Position {
    x: u32,
    y: u32,
}

state "${name}.exe" {
    position: Position at 0x100;
}

`;
const small = `${header('small')}whileAttached {
    let point = current.position
    print(point.x)
}
`;
let large = header('large');
for (let i = 0; i < 500; i++) {
  large += `fn helper${i}(value: u32) -> u32 {\n    return value + ${i}\n}\n\n`;
}
large += `whileAttached {
    let point = current.position
    let selected = helper499(point.x)
    print(selected)
}
`;
const fixtures = [
  ['small', small],
  [basename(fixturePath, '.split'), await readFile(fixturePath, 'utf8')],
  ['generated_large', large],
];
const server = spawn(resolve(binary), [], { windowsHide: true, stdio: ['pipe', 'pipe', 'inherit'] });
const pending = new Map();
let nextId = 0;
let buffer = Buffer.alloc(0);
let failure;
let shuttingDown = false;
function fail(error) {
  failure ??= error;
  for (const waiter of pending.values()) waiter.reject(failure);
  pending.clear();
}
server.on('error', fail);
server.on('exit', (code, signal) => {
  if (!shuttingDown) fail(new Error(`splitls exited early: code=${code}, signal=${signal}`));
});
server.stdin.on('error', fail);
server.stdout.on('data', chunk => {
  try {
    buffer = Buffer.concat([buffer, chunk]);
    for (;;) {
      const end = buffer.indexOf('\r\n\r\n');
      if (end < 0) break;
      const match = /^Content-Length:\s*(\d+)$/im.exec(buffer.subarray(0, end).toString());
      assert(match, 'missing Content-Length');
      const length = Number(match[1]);
      if (buffer.length < end + 4 + length) break;
      const message = JSON.parse(buffer.subarray(end + 4, end + 4 + length).toString());
      buffer = buffer.subarray(end + 4 + length);
      const key = message.method === 'textDocument/publishDiagnostics'
        ? `${message.params.uri}#${message.params.version}` : message.id;
      const waiter = pending.get(key);
      if (!waiter) continue;
      pending.delete(key);
      if (message.error) waiter.reject(new Error(JSON.stringify(message.error)));
      else waiter.resolve(message.params ?? message.result);
    }
  } catch (error) {
    fail(error);
  }
});
function waitFor(key) {
  if (failure) return Promise.reject(failure);
  assert(!pending.has(key), 'duplicate pending response');
  return new Promise((resolve, reject) => pending.set(key, { resolve, reject }));
}
function send(message) {
  const body = Buffer.from(JSON.stringify({ jsonrpc: '2.0', ...message }));
  server.stdin.write(`Content-Length: ${body.length}\r\n\r\n`);
  server.stdin.write(body);
}
function notify(method, params) { send({ method, params }); }
function request(method, params) {
  const id = nextId++;
  const response = waitFor(id);
  send({ id, method, params });
  return response;
}
function checkDiagnostics(result) {
  assert.equal(result.diagnostics.filter(item => item.severity === 1).length, 0,
    JSON.stringify(result.diagnostics));
}
const watchdog = setTimeout(() => {
  fail(new Error('LSP baseline timed out after 180 seconds'));
  server.kill();
}, 180_000);
try {
  await request('initialize', {
    processId: process.pid, rootUri: null,
    capabilities: { textDocument: { publishDiagnostics: { versionSupport: true } } },
  });
  notify('initialized', {});
  console.log(JSON.stringify({ binary: resolve(binary), node: process.version, warmups, samples }));
  for (const [name, source] of fixtures) {
    const uri = `file:///baseline/${encodeURIComponent(name)}.split`;
    const opened = waitFor(`${uri}#1`);
    notify('textDocument/didOpen', { textDocument: { uri, languageId: 'splitscript', version: 1, text: source } });
    checkDiagnostics(await opened);
    const timings = [];
    for (let i = 0; i < warmups + samples; i++) {
      const version = i + 2;
      const diagnostics = waitFor(`${uri}#${version}`);
      const start = performance.now();
      notify('textDocument/didChange', {
        textDocument: { uri, version }, contentChanges: [{ text: source + (i % 2 ? '' : '\n') }],
      });
      const result = await diagnostics;
      const elapsed = performance.now() - start;
      checkDiagnostics(result);
      if (i >= warmups) timings.push(elapsed);
    }
    timings.sort((a, b) => a - b);
    console.log(JSON.stringify({
      fixture: name, sourceBytes: Buffer.byteLength(source),
      sourceSha256: createHash('sha256').update(source).digest('hex'),
      medianMs: timings[Math.floor(samples / 2)], p95Ms: timings[Math.ceil(samples * .95) - 1],
    }));
    notify('textDocument/didClose', { textDocument: { uri } });
  }
  await request('shutdown', null);
  shuttingDown = true;
  notify('exit');
} finally {
  clearTimeout(watchdog);
  server.kill();
}
