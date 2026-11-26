import { expect, it, vi } from "vitest";
import type { PluginOutput } from "@extism/extism";
import { copyPreviewOutput, createBoundedPreview, HOST_BUDGET } from "./wasmPreviewHost";
import { previewChannel } from "./wasmPreview";

// Otherwise-valid goodtype-wasm-v0 fixtures using the actual pinned SDK and kernel imports.
const leb = (value: number): number[] => {
  const bytes = [];
  do { const byte = value & 127; value >>>= 7; bytes.push(byte | (value ? 128 : 0)); } while (value);
  return bytes;
};
const name = (value: string) => [value.length, ...new TextEncoder().encode(value)];
function guest(sizes: number[], output = false) {
  const section = (id: number, data: number[]) => [id, ...leb(data.length), ...data];
  // Positive signed LEB: trailing zero avoids interpreting bit 6 as the sign.
  const constant = (value: number) => [0x42, ...leb(value | 0).slice(0, -1), leb(value).at(-1)! | 128, 0];
  const code = sizes.flatMap(size => [...constant(size), 0x10, 0, ...(output ? [...constant(size), 0x10, 1] : [0x1a])]);
  const body = [0, ...code, 0x41, 0, 0x0b];
  return new Uint8Array([0,97,115,109,1,0,0,0,
    ...section(1, [3, 0x60,1,0x7e,1,0x7e, 0x60,2,0x7e,0x7e,0, 0x60,0,1,0x7f]),
    ...section(2, [2, ...name("extism:host/env"), ...name("alloc"),0,0, ...name("extism:host/env"), ...name("output_set"),0,1]),
    ...section(3, [1,2]), ...section(5, [1,1,1,128,1]),
    ...section(7, [3, ...name("memory"),2,0, ...name("preview"),0,2, ...name("commit"),0,2]),
    ...section(10, [1,...leb(body.length),...body]),
  ]).buffer;
}
it.each([[[HOST_BUDGET + 1]], [[50000, 50000]]])("rejects kernel allocations %j and falls back", async (sizes) => {
  const run = await createBoundedPreview(guest(sizes));
  const fallback = vi.fn(), paint = vi.fn();
  const worker = { terminate: vi.fn(), postMessage: vi.fn() } as unknown as Worker;
  const channel = previewChannel(worker, paint, fallback);
  worker.onmessage!({ data: { kind: "ready" } } as MessageEvent);
  channel.submit({ revision: 1 });
  await expect(run(new Uint8Array([123,125]))).rejects.toThrow("host allocation ceiling");
  worker.onmessage!({ data: { kind: "failure" } } as MessageEvent);
  expect(fallback).toHaveBeenCalledOnce();
  expect(worker.terminate).toHaveBeenCalledOnce();
  expect(paint).not.toHaveBeenCalled();
});
it("rejects an oversized guest output", async () => {
  const run = await createBoundedPreview(guest([32769], true));
  await expect(run(new Uint8Array())).rejects.toThrow("output ceiling");
});
it("checks output length before reading or copying bytes", () => {
  const bytes = vi.fn(() => { throw Error("must not read"); });
  expect(() => copyPreviewOutput({ arrayBuffer: () => ({ byteLength: 32769 }), bytes } as unknown as PluginOutput)).toThrow("output ceiling");
  expect(bytes).not.toHaveBeenCalled();
});
it("resets host storage between calls at the output ceiling", async () => {
  // SDK 1.0.3 returns the entire allocation, regardless of output_set length.
  const run = await createBoundedPreview(guest([32768], true));
  expect((await run(new Uint8Array())).byteLength).toBe(32768);
  expect((await run(new Uint8Array())).byteLength).toBe(32768); // reset between calls
});

it("routes an actual Worker allocation failure to the ghost", async () => {
  const fallback = vi.fn(), paint = vi.fn();
  const worker = { terminate: vi.fn(), postMessage: vi.fn() } as unknown as Worker;
  const channel = previewChannel(worker, paint, fallback);
  const scope = { onmessage: undefined as unknown as (event: { data: unknown }) => Promise<void>,
    postMessage: (data: unknown) => worker.onmessage!({ data } as MessageEvent) };
  vi.stubGlobal("self", scope);
  try {
    await import("./wasmPreview.worker");
    const bytes = guest([HOST_BUDGET + 1]);
    const digest = Array.from(new Uint8Array(await crypto.subtle.digest("SHA-256", bytes)))
      .map(byte => byte.toString(16).padStart(2, "0")).join("");
    await scope.onmessage({ data: { kind: "init", bytes, digest } });
    channel.submit({ revision: 1 });
    await scope.onmessage({ data: { kind: "preview", seq: 1, input: new ArrayBuffer(0) } });
    expect(fallback).toHaveBeenCalledOnce();
    expect(worker.terminate).toHaveBeenCalledOnce();
    expect(paint).not.toHaveBeenCalled();
  } finally { channel.stop(); vi.unstubAllGlobals(); }
});
