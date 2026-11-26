import type { ShapeGeometry } from "../model";

export type Mark = { x: number; y: number; rotation: number; scale: number; geometry: Extract<ShapeGeometry, { kind: "spline" }> };
export const IO_LIMIT = 32768;
export function readMarks(buffer: ArrayBuffer): Mark[] {
  if (buffer.byteLength > IO_LIMIT) throw Error("output ceiling");
  const value = JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(buffer));
  if (!value || Object.keys(value).join() !== "marks" || !Array.isArray(value.marks)) throw Error("marks");
  let count = 0;
  const finite = (n: unknown) => typeof n === "number" && Number.isFinite(n) && Math.abs(n) <= 100000;
  const point = (p: { x?: unknown; y?: unknown }) => p && finite(p.x) && finite(p.y);
  for (const mark of value.marks) {
    if (!finite(mark.x) || !finite(mark.y) || !finite(mark.rotation) || !finite(mark.scale) || mark.scale <= 0 || mark.geometry?.kind !== "spline"
      || mark.geometry.closed !== false || !Array.isArray(mark.geometry.nodes)) throw Error("geometry");
    for (const node of mark.geometry.nodes) {
      count += 1 + Number(node.handleIn != null) + Number(node.handleOut != null);
      if (count > 512 || !point(node.point) || (node.handleIn != null && !point(node.handleIn))
        || (node.handleOut != null && !point(node.handleOut))) throw Error("vertices");
    }
  }
  return value.marks;
}

/** Main-thread deadline owns recovery, including synchronous Wasm that never returns. */
export function previewChannel(worker: Worker, paint: (marks: Mark[]) => void, fallback: () => void) {
  let ready = false, stopped = false, seq = 0;
  let pending: Record<string, unknown> | null = null;
  let inFlight: number | null = null;
  let timer: ReturnType<typeof setTimeout> | undefined;
  const stop = () => {
    if (stopped) return;
    stopped = true; clearTimeout(timer); worker.terminate(); fallback();
  };
  function send() {
    if (!ready || stopped || inFlight !== null || !pending) return;
    const request = pending; pending = null;
    const input = new TextEncoder().encode(JSON.stringify({ ...request, seq: ++seq }));
    if (input.byteLength > IO_LIMIT) { stop(); return; }
    inFlight = seq;
    timer = setTimeout(stop, 16);
    worker.postMessage({ kind: "preview", seq, input: input.buffer }, [input.buffer]);
  }
  worker.onmessage = ({ data }) => {
    if (stopped) return;
    if (data.kind === "ready") { ready = true; clearTimeout(timer); send(); return; }
    if (data.kind !== "result" || data.seq !== inFlight) { stop(); return; }
    clearTimeout(timer); inFlight = null;
    try { const marks = readMarks(data.output); if (!pending) paint(marks); }
    catch { stop(); return; }
    send();
  };
  worker.onerror = stop;
  timer = setTimeout(stop, 5000);
  return { submit(request: Record<string, unknown>) { pending = request; send(); }, stop };
}
