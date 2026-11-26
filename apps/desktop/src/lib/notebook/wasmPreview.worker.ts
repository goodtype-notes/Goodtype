// No Tauri bridge, network loader, or user code beyond verified Wasm in this Worker.
import { createBoundedPreview } from "./wasmPreviewHost";
let plugin: Awaited<ReturnType<typeof createBoundedPreview>> | undefined;
self.onmessage = async ({ data }) => {
  try {
    if (data.kind === "init") {
      const digest = Array.from(new Uint8Array(await crypto.subtle.digest("SHA-256", data.bytes)))
        .map(byte => byte.toString(16).padStart(2, "0")).join("");
      if (digest !== data.digest) throw Error("digest");
      plugin = await createBoundedPreview(data.bytes);
      self.postMessage({ kind: "ready" });
    } else if (data.kind === "preview" && plugin) {
      const output = await plugin(new Uint8Array(data.input));
      self.postMessage({ kind: "result", seq: data.seq, output }, { transfer: [output] });
    } else throw Error("protocol");
  } catch { self.postMessage({ kind: "failure" }); }
};
