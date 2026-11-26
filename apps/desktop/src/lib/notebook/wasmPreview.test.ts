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
