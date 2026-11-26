// No Tauri bridge, network loader, or user code beyond verified Wasm in this Worker.
import { createPlugin, type Plugin } from "@extism/extism";
let plugin: Plugin | undefined;
self.onmessage = async ({ data }) => {
  try {
    if (data.kind === "init") {
      const digest = Array.from(new Uint8Array(await crypto.subtle.digest("SHA-256", data.bytes)))
        .map(byte => byte.toString(16).padStart(2, "0")).join("");
      if (digest !== data.digest) throw Error("digest");
      plugin = await createPlugin(data.bytes, { runInWorker: false, useWasi: false, allowedHosts: [], allowedPaths: {} });
      self.postMessage({ kind: "ready" });
    } else if (data.kind === "preview" && plugin) {
      const result = await plugin.call("preview", new Uint8Array(data.input));
      if (!result) throw Error("missing output");
      const output = new Uint8Array(result.bytes()).slice().buffer;
      self.postMessage({ kind: "result", seq: data.seq, output }, { transfer: [output] });
    } else throw Error("protocol");
  } catch { self.postMessage({ kind: "failure" }); }
};
