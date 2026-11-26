// Node harness for the actual Worker entry; this is not packaged WebView2 evidence.
import { parentPort } from 'node:worker_threads';
globalThis.self = globalThis;
delete globalThis.SharedArrayBuffer;
globalThis.postMessage = (message, options) => parentPort.postMessage(message, options?.transfer ?? []);
await import('../src/lib/notebook/wasmPreview.worker.ts');
parentPort.on('message', data => globalThis.onmessage({ data }));
