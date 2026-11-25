import { describe, expect, it } from "vitest";
import fixtures from "../../../../../fixtures/shape-end-caps.json";
import type { ShapeGeometry, ShapeStyle } from "../model";
import type { ShapeView } from "../notebook/pageView";
import { shapeBounds, shapeCaps, shapePath } from "./geometry";
import { shapeTouchedAt } from "./hit";

describe("shape end caps", () => {
  it("matches the shared export geometry, including tangents and the trimmed shaft", () => {
    for (const { geometry, style, caps, shaft } of fixtures) {
      const marks = shapeCaps(geometry as ShapeGeometry, style as ShapeStyle);
      expect(marks.caps).toHaveLength(caps.length);
      for (const [index, cap] of marks.caps.entries()) {
        const expected = caps[index];
        expect(cap.kind).toBe(expected.kind);
        expect(cap.radius).toBeCloseTo(expected.radius);
        expect(cap.points).toHaveLength(expected.points.length);
        for (const [i, point] of cap.points.entries()) {
          expect(point.x).toBeCloseTo(expected.points[i].x);
          expect(point.y).toBeCloseTo(expected.points[i].y);
        }
      }
      expect(marks.shaft && shapePath(marks.shaft)).toBe(shapePath(shaft as ShapeGeometry));
    }
  });

  it("includes the cap in local bounds and hits it after page rotation and scaling", () => {
    const geometry = fixtures[0].geometry as ShapeGeometry;
    const style = fixtures[0].style as ShapeStyle;
    expect(shapeBounds(geometry, style)).toEqual({ left: 0, top: -4, right: 40, bottom: 4 });
    const shape: ShapeView = { id: "cap", x: 10, y: 20, scale: 2, rotation: 90, zIndex: 0, readingOrder: 0, geometry, style };
    expect(shapeTouchedAt(shape, { x: 4, y: 36 }, 0.01)).toBe(true);
    expect(shapeTouchedAt(shape, { x: 0, y: 36 }, 0.01)).toBe(false);
  });
});
