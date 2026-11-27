import { describe, expect, it, vi } from "vitest";
import { previewChannel, readMarks } from "./wasmPreview";

describe("W1 preview fallback", () => {
  it("terminates a stuck call after 16 ms and keeps late results out", () => {
    vi.useFakeTimers();
    const worker = { terminate: vi.fn(), postMessage: vi.fn() } as unknown as Worker;
    const paint = vi.fn(), fallback = vi.fn();
    const channel = previewChannel(worker, paint, fallback);
    worker.onmessage!({ data: { kind: "ready" } } as MessageEvent);
    channel.submit({ gestureId: "one", revision: 1 });
    vi.advanceTimersByTime(16);
    expect(worker.terminate).toHaveBeenCalledOnce();
    expect(fallback).toHaveBeenCalledOnce();
    worker.onmessage!({ data: { kind: "result", seq: 1 } } as MessageEvent);
    expect(paint).not.toHaveBeenCalled();
    vi.useRealTimers();
  });
  it("rejects malformed output and an over-budget buffer", () => {
    expect(() => readMarks(new TextEncoder().encode('{"marks":[{}]}').buffer)).toThrow();
    expect(() => readMarks(new ArrayBuffer(32769))).toThrow();
  });
  it.each(["failure", "malformed"])("falls back on %s and terminates once", kind => {
    const worker = { terminate: vi.fn(), postMessage: vi.fn() } as unknown as Worker;
    const paint = vi.fn(), fallback = vi.fn();
    const channel = previewChannel(worker, paint, fallback);
    worker.onmessage!({ data: { kind: "ready" } } as MessageEvent);
    channel.submit({ gestureId: "one", revision: 1 });
    worker.onmessage!({ data: kind === "failure" ? { kind } : { kind: "result", seq: 1, output: new ArrayBuffer(0) } } as MessageEvent);
    expect(fallback).toHaveBeenCalledOnce();
    expect(worker.terminate).toHaveBeenCalledOnce();
    channel.stop();
    expect(fallback).toHaveBeenCalledOnce();
    expect(paint).not.toHaveBeenCalled();
  });
});


describe("host mark admission", () => {
  const read = (geometry: unknown, style?: unknown) => readMarks(new TextEncoder().encode(JSON.stringify({
    marks: [{ x: 0, y: 0, rotation: 0, scale: 1, geometry, style }],
  })).buffer);
  it("admits all primitives and clamps widths", () => {
    const points = [{ x: 0, y: 0 }, { x: 1, y: 2 }];
    for (const geometry of [{ kind: "polyline", points }, { kind: "bezier", points: [...points, ...points] },
      { kind: "label", text: "<script>plain text</script>" }, { kind: "ring", radius: 10 }]) {
      expect(read(geometry, { color: "#aabbcc", width: 999 })[0].style.width).toBe(8);
    }
  });
  it("rejects unsafe styles, oversized labels and geometry", () => {
    expect(() => read({ kind: "ring", radius: 1 }, { color: "url(https://example.org)", width: 1 })).toThrow();
    expect(() => read({ kind: "label", text: "x".repeat(257) })).toThrow();
    expect(() => read({ kind: "polyline", points: Array(513).fill({ x: 0, y: 0 }) })).toThrow();
    expect(() => read({ kind: "bezier", points: [{ x: 0, y: 0 }] })).toThrow();
    expect(() => read({ kind: "ring", radius: -1 })).toThrow();
    const marks = Array(129).fill({ x: 0, y: 0, rotation: 0, scale: 1, geometry: { kind: "ring", radius: 1 } });
    expect(() => readMarks(new TextEncoder().encode(JSON.stringify({ marks })).buffer)).toThrow();
  });
});
