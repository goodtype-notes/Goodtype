import { describe, expect, it } from "vitest";
import type { Stroke } from "../model";
import {
  bundleCenter,
  clipboardMarker,
  shiftTo,
  isEmptyBundle,
  PASTE_STEP_PT,
  pasteOffset,
  pastedAddonData,
  pastedShapes,
  pastedStrokes,
  type ClipboardBundle,
} from "./clipboard";
import type { ShapeView } from "./pageView";

const stroke: Stroke = {
  id: "ink-1",
  zIndex: 4,
  tool: "pen",
  color: "#111111",
  widthPt: 2,
  pressure: false,
  taper: 0,
  opacity: 1,
  groupId: "group-a",
  points: [{ x: 1, y: 2, pressure: 0.5, timeMs: 0, tiltX: 0, tiltY: 0 }],
  transform: { translateX: 10, translateY: 20, scaleX: 1, scaleY: 1, rotation: 0 },
};

const shape: ShapeView = {
  id: "shape-1",
  x: 50,
  y: 60,
  rotation: 0,
  scale: 1,
  zIndex: 5,
  readingOrder: 2,
  geometry: { kind: "line", start: { x: 0, y: 0 }, end: { x: 10, y: 0 } },
  style: { strokeColor: "#000000", strokeWidthPt: 1, fillColor: null, opacity: 1 },
};

function bundle(changes: Partial<ClipboardBundle> = {}): ClipboardBundle {
  return { token: "t1", pageId: "page-1", strokes: [], shapes: [], blocks: [], images: [], ...changes };
}

describe("clipboard bundles", () => {
  it("fans repeated pastes out on the original page and keeps position elsewhere", () => {
    expect(pasteOffset(bundle(), "page-1", 1)).toEqual({ dx: PASTE_STEP_PT, dy: PASTE_STEP_PT });
    expect(pasteOffset(bundle(), "page-1", 3).dx).toBe(PASTE_STEP_PT * 3);
    expect(pasteOffset(bundle(), "page-2", 1)).toEqual({ dx: 0, dy: 0 });
  });

  it("gives pasted ink fresh ids, no group, its own stacking, and moves it", () => {
    let counter = 0;
    const [copy] = pastedStrokes([stroke], { dx: 12, dy: 12 }, 40, () => `new-${(counter += 1)}`);
    expect(copy.id).toBe("new-1");
    expect(copy.groupId).toBeNull();
    expect(copy.zIndex).toBe(40);
    expect(copy.transform).toMatchObject({ translateX: 22, translateY: 32 });
    expect(copy.points).toEqual(stroke.points);
    expect(stroke.id).toBe("ink-1");
  });

  it("gives pasted shapes fresh ids and moves them", () => {
    const capped = { ...shape, style: { ...shape.style, startCap: "bar" as const, endCap: "arrow" as const } };
    const [copy] = pastedShapes([capped], { dx: 12, dy: 12 }, 9, 7, () => "shape-new");
    expect(copy).toMatchObject({ id: "shape-new", x: 62, y: 72, zIndex: 9, readingOrder: 7 });
    expect(copy.geometry).toEqual(shape.geometry);
    expect(copy.style).toEqual(capped.style);
  });

  it("remaps copied refs and retains external refs only when the original is on the destination", () => {
    const data = { connectors: { refs: ["old", "kept", "missing"], data: { routing: "straight" } } };
    expect(pastedAddonData(data, new Map([["old", "new"]]), new Set(["kept"]))).toEqual({
      connectors: { refs: ["new", "kept"], data: data.connectors.data },
    });
    expect(data.connectors.refs).toEqual(["old", "kept", "missing"]);
  });

  it("centres a paste on the point that was clicked", () => {
    // The line runs 0..10 along x at (50, 60), so its middle is (55, 60).
    const withShape = bundle({ shapes: [shape] });
    expect(bundleCenter(withShape)).toEqual({ x: 55, y: 60 });
    expect(shiftTo(withShape, { x: 100, y: 100 })).toEqual({ dx: 45, dy: 40 });
    expect(bundleCenter(bundle())).toBeNull();
    // A Typst block centres on its measured height, not just its top edge.
    const block = { source: "", x: 0, y: 0, layoutWidthPt: 100, heightPt: 40, scale: 2 };
    expect(bundleCenter(bundle({ blocks: [block] }))).toEqual({ x: 100, y: 40 });
  });

  it("centres rotated copies on their visible bounds", () => {
    const image = { bytes: new Uint8Array(), mime: "image/png", alt: "rotated", x: 50, y: 60,
      widthPt: 30, heightPt: 10, scale: 1, rotation: 90 };
    const center = bundleCenter(bundle({ images: [image] }))!;
    expect(center.x).toBeCloseTo(45);
    expect(center.y).toBeCloseTo(75);
  });

  it("knows an empty copy and marks a copy for the system clipboard", () => {
    expect(isEmptyBundle(bundle())).toBe(true);
    expect(isEmptyBundle(bundle({ shapes: [shape] }))).toBe(false);
    expect(clipboardMarker(bundle())).toBe("goodtype-clipboard:t1");
  });
});
