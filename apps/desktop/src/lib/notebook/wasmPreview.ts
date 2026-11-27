import type { ShapeGeometry } from "../model";

type Point = { x: number; y: number };
type Geometry = Extract<ShapeGeometry, { kind: "spline" }>
  | { kind: "polyline"; points: Point[] }
  | { kind: "bezier"; points: [Point, Point, Point, Point] }
  | { kind: "label"; text: string }
  | { kind: "ring"; radius: number };
export type Mark = { x: number; y: number; rotation: number; scale: number; geometry: Geometry;
  style: { color: string; width: number } };
export const IO_LIMIT = 32768;
export function readMarks(buffer: ArrayBuffer): Mark[] {
  if (buffer.byteLength > IO_LIMIT) throw Error("output ceiling");
  const value = JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(buffer));
  if (!value || Object.keys(value).join() !== "marks" || !Array.isArray(value.marks) || value.marks.length > 128) throw Error("marks");
  let count = 0;
  const finite = (n: unknown): n is number => typeof n === "number" && Number.isFinite(n) && Math.abs(n) <= 100000;
  const point = (p: Point) => p && finite(p.x) && finite(p.y) && ++count <= 512;
  return value.marks.map((mark: Mark) => {
    if (!mark || !finite(mark.x) || !finite(mark.y) || !finite(mark.rotation) || !finite(mark.scale) || mark.scale <= 0) throw Error("transform");
    const geometry = mark.geometry;
    if (!geometry) throw Error("geometry");
    switch (geometry.kind) {
      case "spline":
        if (geometry.closed !== false || !Array.isArray(geometry.nodes) || geometry.nodes.length < 2) throw Error("spline");
        for (const node of geometry.nodes) {
          if (!node || !point(node.point) || (node.handleIn != null && !point(node.handleIn))
            || (node.handleOut != null && !point(node.handleOut))) throw Error("vertices");
        }
        break;
      case "polyline": case "bezier":
        if (!Array.isArray(geometry.points) || geometry.points.length < 2
          || (geometry.kind === "bezier" && geometry.points.length !== 4) || !geometry.points.every(point)) throw Error("vertices");
        break;
      case "label":
        if (typeof geometry.text !== "string" || geometry.text.length > 256 || /[\u0000-\u001f\u007f]/.test(geometry.text)) throw Error("label");
        break;
      case "ring":
        if (!finite(geometry.radius) || geometry.radius <= 0 || geometry.radius > 4096) throw Error("radius");
        break;
      default: throw Error("geometry");
    }
    const color = mark.style?.color ?? "#2563eb";
    const width = mark.style?.width ?? 1.5;
    if (typeof color !== "string" || !/^(#[0-9a-fA-F]{6}|black|white|red|blue|green|orange)$/.test(color) || !finite(width)) throw Error("style");
    return { x: mark.x, y: mark.y, rotation: mark.rotation, scale: mark.scale, geometry,
      style: { color, width: Math.max(0.25, Math.min(8, width)) } };
  });
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
