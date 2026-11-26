import { describe, expect, it } from "vitest";
import { selectStrokesInLasso } from "../ink/selection";
import type { Stroke } from "../model";
import { rotationHandlePoint, ROTATE_HANDLE_SCREEN_PX } from "../shape/edit";
import { shapeBounds } from "../shape/geometry";
import { toPagePoint } from "../shape/hit";
import { DEFAULT_LASSO_TARGETS, boxOutline, combinedBounds, selectLassoObjects, selectionDragTransform, selectionHandles, shapeLassoItem, transformSelectionPoint } from "./lasso";
import type { ShapeView } from "./pageView";

const polygon = boxOutline(10, 10, 20, 20);
const stroke = (id: string, tool: "pen" | "highlighter", points = [{ x: 0, y: 20 }, { x: 40, y: 20 }]): Stroke => ({
  id, tool, points, transform: { translateX: 0, translateY: 0, scaleX: 1, scaleY: 1, rotation: 0 },
} as Stroke);

describe("lasso categories and geometry", () => {
  it("catches sparse handwriting and highlight segments crossing the loop, with independent filters", () => {
    const strokes = [stroke("pen", "pen"), stroke("highlight", "highlighter"), stroke("outside", "pen", [{ x: 40, y: 40 }])];
    expect(selectStrokesInLasso(strokes, polygon)).toEqual(["pen", "highlight"]);
    expect(selectStrokesInLasso(strokes, polygon, stroke => stroke.tool === "pen")).toEqual(["pen"]);
    expect(selectStrokesInLasso(strokes, polygon, stroke => stroke.tool === "highlighter")).toEqual(["highlight"]);
    expect(selectStrokesInLasso(strokes, [])).toEqual([]);
  });

  it("tests transformed ink instead of its original samples", () => {
    const ink = stroke("moved", "pen", [{ x: 0, y: 0 }]);
    ink.transform.translateX = 20; ink.transform.translateY = 20;
    expect(selectStrokesInLasso([ink], polygon)).toEqual(["moved"]);
  });

  it("selects all enabled object kinds and ignores disabled ones, including a lasso within an image", () => {
    const items = (["shapes", "images", "text"] as const).map(kind => ({ id: kind, kind, closed: true, outline: boxOutline(0, 0, 50, 50) }));
    expect(selectLassoObjects(items, polygon, DEFAULT_LASSO_TARGETS)).toEqual(["shapes", "images", "text"]);
    expect(selectLassoObjects(items, polygon, { ...DEFAULT_LASSO_TARGETS, images: false })).toEqual(["shapes", "text"]);
    expect(selectLassoObjects(items, boxOutline(60, 60, 10, 10), DEFAULT_LASSO_TARGETS)).toEqual([]);
  });

  it("uses rotated/scaled shape outlines and does not select the empty corner of an ellipse's box", () => {
    const shape = { id: "shape", x: 20, y: 20, scale: 2, rotation: 90,
      geometry: { kind: "line", start: { x: 0, y: 0 }, end: { x: 30, y: 0 } },
      style: { strokeColor: "#000000", strokeWidthPt: 2, opacity: 1, fillColor: null } } as ShapeView;
    expect(selectLassoObjects([shapeLassoItem(shape)], boxOutline(15, 40, 10, 10), DEFAULT_LASSO_TARGETS)).toEqual(["shape"]);
    shape.geometry = { kind: "ellipse", widthPt: 100, heightPt: 100 };
    shape.x = 0; shape.y = 0; shape.rotation = 0; shape.scale = 1;
    expect(selectLassoObjects([shapeLassoItem(shape)], boxOutline(0, 0, 3, 3), DEFAULT_LASSO_TARGETS)).toEqual([]);
  });

  it("lassos the cap away from the shaft and includes it in selection bounds", () => {
    const shape: ShapeView = { id: "shape", x: 0, y: 0, scale: 1, rotation: 0, zIndex: 0, readingOrder: 0,
      geometry: { kind: "line", start: { x: 0, y: 0 }, end: { x: 30, y: 0 } },
      style: { strokeColor: "#000000", strokeWidthPt: 2, opacity: 1, fillColor: null, endCap: "arrow" } };
    const item = shapeLassoItem(shape);
    expect(selectLassoObjects([item], boxOutline(22, 2, 2, 1), DEFAULT_LASSO_TARGETS)).toEqual(["shape"]);
    expect(combinedBounds([item], null)).toEqual({ left: 0, top: -4, right: 30, bottom: 4 });
  });

  it("combines ink/object bounds and applies one affine move/scale to every material", () => {
    const item = { id: "image", kind: "images" as const, closed: true, outline: boxOutline(40, 50, 20, 30) };
    expect(combinedBounds([item], { left: 10, top: 15, right: 20, bottom: 25 }))
      .toEqual({ left: 10, top: 15, right: 60, bottom: 80 });
    expect(combinedBounds([], null)).toBeNull();
    expect(transformSelectionPoint({ x: 30, y: 40 }, { x: 5, y: -2 }, 2, { x: 10, y: 10 }))
      .toEqual({ x: 55, y: 68 });
  });
});

describe("selection handles and drags", () => {
  const shape = (id: string, x: number, y: number, rotation = 0, scale = 1): ShapeView => ({
    id, x, y, rotation, scale, zIndex: 1, readingOrder: 0,
    geometry: { kind: "rectangle", widthPt: 80, heightPt: 40, cornerRadiusPt: 0 },
    style: { strokeColor: "#000", strokeWidthPt: 1.6, fillColor: null, opacity: 1 },
  });
  const boundsOf = (shapes: ShapeView[]) => combinedBounds(shapes.map(shapeLassoItem), null)!;
  const applyDrag = (shapes: ShapeView[], drag: ReturnType<typeof selectionDragTransform>) => shapes.map(item => ({
    ...item, ...transformSelectionPoint(item, drag.delta, drag.scale, drag.anchor),
    scale: item.scale * drag.scale,
  }));

  it("puts the handles on the box corner and a fixed screen distance above it at any zoom", () => {
    for (const zoom of [0.25, 1, 3.5]) {
      const bounds = boundsOf([shape("a", 30, 50, 25, 1.5), shape("b", 140, 90, 70)]);
      const { scale, rotate } = selectionHandles(bounds, zoom);
      expect(scale).toEqual({ x: bounds.right, y: bounds.bottom });
      expect(rotate.x).toBeCloseTo((bounds.left + bounds.right) / 2, 9);
      expect((bounds.top - rotate.y) * zoom).toBeCloseTo(ROTATE_HANDLE_SCREEN_PX, 9);
    }
  });

  it("matches where an ordinary rectangle puts its own corner and rotate handles", () => {
    for (const zoom of [0.5, 1, 2]) {
      for (const scale of [1, 2.5]) {
        const rectangle = shape("r", 40, 60, 0, scale);
        const local = shapeBounds(rectangle.geometry, rectangle.style);
        const own = {
          scale: toPagePoint(rectangle, { x: local.right, y: local.bottom }),
          rotate: toPagePoint(rectangle, rotationHandlePoint(rectangle.geometry, ROTATE_HANDLE_SCREEN_PX / (zoom * scale), rectangle.style)),
        };
        const grouped = selectionHandles(boundsOf([rectangle]), zoom);
        expect(grouped.scale.x).toBeCloseTo(own.scale.x, 9);
        expect(grouped.scale.y).toBeCloseTo(own.scale.y, 9);
        expect(grouped.rotate.x).toBeCloseTo(own.rotate.x, 9);
        expect(grouped.rotate.y).toBeCloseTo(own.rotate.y, 9);
      }
    }
  });

  it("moves the box exactly by the pointer delta", () => {
    const shapes = [shape("a", 30, 50, 25, 1.5), shape("b", 140, 90, 70)];
    const before = boundsOf(shapes);
    const moved = boundsOf(applyDrag(shapes, selectionDragTransform("move", before, { x: 17.5, y: -9 }, 2)));
    expect(moved.left).toBeCloseTo(before.left + 17.5, 9);
    expect(moved.top).toBeCloseTo(before.top - 9, 9);
    expect(moved.right).toBeCloseTo(before.right + 17.5, 9);
    expect(moved.bottom).toBeCloseTo(before.bottom - 9, 9);
  });

  it("keeps the top-left corner fixed and the corner handle under a diagonal drag", () => {
    for (const zoom of [0.5, 1, 4]) {
      const shapes = [shape("a", 30, 50, 25, 1.5), shape("b", 140, 90, 70)];
      const before = boundsOf(shapes);
      const width = before.right - before.left, height = before.bottom - before.top;
      const pull = { x: width * 0.3, y: height * 0.3 };
      const after = boundsOf(applyDrag(shapes, selectionDragTransform("scale", before, pull, zoom)));
      expect(after.left).toBeCloseTo(before.left, 9);
      expect(after.top).toBeCloseTo(before.top, 9);
      expect(after.right).toBeCloseTo(before.right + pull.x, 9);
      expect(after.bottom).toBeCloseTo(before.bottom + pull.y, 9);
    }
  });

  it("scales by the pointer's projection on the diagonal and never collapses", () => {
    const bounds = { left: 0, top: 0, right: 100, bottom: 50 };
    expect(selectionDragTransform("scale", bounds, { x: -50, y: -25 }, 1).scale).toBeCloseTo(0.5, 9);
    // A pull at right angles to the diagonal changes nothing.
    expect(selectionDragTransform("scale", bounds, { x: -50, y: 100 }, 1).scale).toBeCloseTo(1, 9);
    expect(selectionDragTransform("scale", bounds, { x: -500, y: -500 }, 1).scale).toBe(0.05);
  });

  it("turns about the centre to the pointer's angle from the rotate handle", () => {
    const bounds = { left: 0, top: 0, right: 100, bottom: 60 };
    for (const zoom of [0.5, 2]) {
      const handle = selectionHandles(bounds, zoom).rotate;
      const centre = { x: 50, y: 30 };
      const drag = selectionDragTransform("rotate", bounds, { x: 10, y: 0 }, zoom);
      const expected = Math.atan2(handle.x + 10 - centre.x, centre.y - handle.y) * 180 / Math.PI;
      expect(drag.rotation).toBeCloseTo(expected, 9);
      expect(drag.anchor).toEqual(centre);
      expect(selectionDragTransform("rotate", bounds, { x: 0, y: 0 }, zoom).rotation).toBeCloseTo(0, 9);
    }
  });
});
