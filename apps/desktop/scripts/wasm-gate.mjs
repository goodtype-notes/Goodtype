// Run after building the sideload module and the unchanged native Connectors executable.
import fs from 'node:fs';
import crypto from 'node:crypto';
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { Worker } from 'node:worker_threads';
import { once } from 'node:events';
import { createPlugin } from '@extism/extism';
import { readMarks } from '../src/lib/notebook/wasmPreview.ts';

const [modulePath, nativePath, outputPath] = process.argv.slice(2);
const bytes = fs.readFileSync(modulePath);
const digest = crypto.createHash('sha256').update(bytes).digest('hex');
const initialMemory = process.memoryUsage();
const coldStart = performance.now();
const plugin = await createPlugin(bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength), {
  runInWorker: false, useWasi: false, allowedHosts: [], allowedPaths: {},
});
const coldMs = performance.now() - coldStart;
const part = (id, x, width = 48) => ({ id, type: 'shape', bounds: { x, y: 100, width, height: 16 }, ports: [{ x, y: 108 }, { x: x + width, y: 108 }] });
const page = { widthPt: 595.28, heightPt: 841.89 };
const settings = { routing: 'orthogonal', arrow: 'none' };
const create = { tool: 'connect', settings, page, style: { strokeColor: '#182c39', strokeWidthPt: 1.5, fillColor: null, opacity: 1 }, points: [
  { x: 148, y: 108, attach: { objectId: 'a', port: 1 } },
  { x: 300, y: 108, attach: { objectId: 'b', port: 0 } },
], objects: [part('a', 100), part('b', 300)] };
let nativeReply;
function native(method, params) {
  const initialize = { jsonrpc: '2.0', id: '1', method: 'initialize', params: {
    apiVersion: 'goodtype-api 0.1', capabilities: ['page-batch-v0', 'path-edit-v0'], addon: { id: 'connectors', version: '0.2.0' },
  } };
  const request = { jsonrpc: '2.0', id: '2', method, params };
  const lines = execFileSync(nativePath, ['--session'], { input: `${JSON.stringify(initialize)}\n${JSON.stringify(request)}\n`, encoding: 'utf8' }).trim().split('\n');
  nativeReply = lines[1];
  const rows = lines.map(JSON.parse);
  assert.ok(rows[1].result, JSON.stringify(rows[1]));
  return rows[1].result;
}
const created = native('tool/apply', create);
const { op, ...shape } = created.batch.operations[0];
const own = { ...shape, id: 'wire', type: 'shape', rotation: 0, scale: 1 };
const fixture = (name, kind, local, revision) => {
  const expected = native(kind === 'tool' ? 'tool/apply' : 'page/changed', local);
  return { name, nativeReply, commit: { revision, gesture: kind, scope: local, settings },
    preview: { gestureId: name, seq: 1, revision, kind, local, ports: [], settings }, expected };
};
const fixtures = [fixture('snapping', 'tool', create, 7),
  fixture('drag', 'changed', { page, own: [own], changed: [part('a', 140)], deleted: [] }, 8),
  fixture('reroute', 'changed', { page, own: [own], changed: [part('a', 200), part('b', 160)], deleted: [] }, 9),
  fixture('stretch', 'changed', { page, own: [own], changed: [part('a', 100, 96)], deleted: [] }, 10)];
for (const fixture of fixtures) {
  fixture.commitRaw = (await plugin.call('commit', JSON.stringify(fixture.commit))).text();
  const output = JSON.parse(fixture.commitRaw);
  assert.deepEqual(output, fixture.expected);
  fixture.previewRaw = (await plugin.call('preview', JSON.stringify(fixture.preview))).text();
  fixture.previewExpected = JSON.parse(fixture.previewRaw);
}
const afterMemory = process.memoryUsage();
const guestMemoryBytes = (await plugin.getInstance()).exports.memory.buffer.byteLength;
await plugin.close();
const worker = new Worker(new URL('./wasm-worker-node.mjs', import.meta.url));
const roundtrip = data => { const reply = once(worker, 'message'); worker.postMessage(data); return reply.then(([data]) => data); };
const workerStart = performance.now();
assert.equal((await roundtrip({ kind: 'init', bytes: bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength), digest })).kind, 'ready');
const workerColdMs = performance.now() - workerStart;
const times = [];
for (let index = 0; index < 620; index++) {
  const fixture = fixtures[index % fixtures.length];
  const start = performance.now();
  const input = new TextEncoder().encode(JSON.stringify(fixture.preview));
  const message = once(worker, 'message');
  worker.postMessage({ kind: 'preview', seq: index, input: input.buffer }, [input.buffer]);
  const [reply] = await message;
  assert.equal(reply.kind, 'result');
  assert.deepEqual({ marks: readMarks(reply.output) }, fixture.previewExpected);
  if (index >= 20) times.push(performance.now() - start);
  await new Promise(resolve => setTimeout(resolve, Math.max(0, 1000 / 60 - (performance.now() - start))));
}
await worker.terminate();
times.sort((a, b) => a - b);
const percentile = p => times[Math.ceil(p * times.length) - 1];
const result = { digest, moduleBytes: bytes.length, fixtures, jsColdMs: coldMs, workerColdMs,
  guestMemoryBytes, jsProcessRssDeltaBytes: afterMemory.rss - initialMemory.rss,
  worker: { samples: times.length, hz: 60, p50: percentile(.5), p95: percentile(.95), p99: percentile(.99) } };
fs.writeFileSync(outputPath, JSON.stringify(result, null, 2));
console.log(JSON.stringify({ ...result, fixtures: fixtures.map(f => f.name) }, null, 2));
