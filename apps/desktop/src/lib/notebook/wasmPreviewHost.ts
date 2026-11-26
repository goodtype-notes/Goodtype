import { createPlugin, type PluginOutput } from "@extism/extism";
import { IO_LIMIT } from "./wasmPreview";

// SDK 1.0.3 merges functions over kernel imports. Its foreground alloc delegates to
// CallContext.alloc; call/reset do not copy outputs. Keep this pinned-SDK assumption tested.
export const HOST_BUDGET = 2 * IO_LIMIT + 32768;
export function copyPreviewOutput(result: PluginOutput | null): ArrayBuffer {
  if (!result || result.arrayBuffer().byteLength > IO_LIMIT) throw Error("output ceiling");
  return result.bytes().slice().buffer;
}
export async function createBoundedPreview(bytes: ArrayBuffer) {
  let remaining = 0, allocations = 0;
  const plugin = await createPlugin(bytes, {
    runInWorker: false, useWasi: false, allowedHosts: [], allowedPaths: {},
    functions: { "extism:host/env": {
      alloc(context, size: bigint) {
        // Charge freed and zero-length blocks too; no guest-controlled unbounded block list.
        const charge = size > 0n ? size : 1n;
        if (size < 0n || charge > BigInt(remaining) || ++allocations > 1024) throw Error("host allocation ceiling");
        remaining -= Number(charge);
        return context.alloc(size);
      },
    } },
  });
  return async (input: Uint8Array): Promise<ArrayBuffer> => {
    if (input.byteLength > IO_LIMIT) throw Error("input ceiling");
    // Include SDK input storage and the one-byte sentinel in the total budget.
    remaining = HOST_BUDGET - input.byteLength - 1;
    allocations = 0;
    try { return copyPreviewOutput(await plugin.call("preview", input)); }
    finally { await plugin.reset(); }
  };
}
