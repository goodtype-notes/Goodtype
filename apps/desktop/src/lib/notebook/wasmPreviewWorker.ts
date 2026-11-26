export function createPreviewWorker() {
  return new Worker(new URL("./wasmPreview.worker.ts", import.meta.url), { type: "module" });
}
