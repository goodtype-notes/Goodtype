// Node harness for the actual Worker entry; this is not packaged WebView2 evidence.
import { parentPort } from 'node:worker_threads';
import { registerHooks } from 'node:module';
// Resolve the app's bundler-style TypeScript imports in this Node-only gate harness.
registerHooks({ resolve(specifier, context, nextResolve) {
  if (specifier.startsWith('./') && !/\.[a-z]+$/.test(specifier)) {
    return nextResolve(specifier + '.ts', context);
  }
  return nextResolve(specifier, context);
} });
globalThis.self = globalThis;
delete globalThis.SharedArrayBuffer;
globalThis.postMessage = (message, options) => parentPort.postMessage(message, options?.transfer ?? []);
await import('../src/lib/notebook/wasmPreview.worker.ts');
parentPort.on('message', data => globalThis.onmessage({ data }));
