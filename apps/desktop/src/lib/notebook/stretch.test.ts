import { describe, expect, it } from "vitest";
import cases from "../../../../../fixtures/stretch-endpoints.json";
import { transformPoint, viewportToPage } from "../geometry/coordinates";
import geometryCases from "../../../../../fixtures/stretch-geometry.json";
import { snapAround } from "../shape/edit";
import type { StretchRule } from "../model";
import type { ShapeView } from "./pageView";
import { previewStretch, stretchFrame, type StretchGroup } from "./stretch";

describe("stretch endpoints", () => {
  for (const test of cases) {
    it(test.name, () => {
      const { frame, end } = stretchFrame(test.group, test.start, test.end, test.startMoved, test.endpoint, test.minimum);
      [test.start, end].map(point => transformPoint(frame, point)).forEach((actual, index) => {
        expect(actual.x).toBeCloseTo(test.expected[index].x, 7);
        expect(actual.y).toBeCloseTo(test.expected[index].y, 7);
      });
      expect(frame.scale).toBe(test.group.scale);
    });
  }

  it.each([0.25, 1, 4])("Shift snapping uses page direction at zoom %s", zoom => {
    const delta = viewportToPage({ x: 10 * zoom, y: 8 * zoom }, { pageOriginX: 0, pageOriginY: 0, zoom });
    const snapped = snapAround(delta, { x: 0, y: 0 }, 0);
    expect(snapped.x).toBeCloseTo(snapped.y);
    expect(Math.hypot(snapped.x, snapped.y)).toBeCloseTo(Math.hypot(10, 8));
  });
});

describe("stretch geometry", () => {
  for (const test of geometryCases) {
    it(test.name, () => {
      const shapes = test.parts.map((part, index): ShapeView => ({
        id: String(index), x: part.x, y: part.y, scale: 1, rotation: 0, zIndex: index,
        readingOrder: 0, geometry: part.geometry as ShapeView["geometry"],
        style: { strokeColor: "#000000", strokeWidthPt: 2, opacity: 1, fillColor: null },
      }));
      const group: StretchGroup = {
        id: "group", type: "group", x: 0, y: 0, scale: 1, rotation: 0, groupId: null,
        createdAt: "", modifiedAt: "", zIndex: 0, readingOrder: 0,
        childIds: shapes.map(shape => shape.id), ports: [{ x: 0, y: 0 }, { x: test.length, y: 0 }],
        stretch: {
          axis: [0, 1], limits: test.limits as StretchRule["limits"],
          parts: test.parts.map(({ role }, index) => ({ childId: String(index), role: role as StretchRule["parts"][number]["role"] })),
        },
      };
      const next = previewStretch(group, shapes, 1, { x: test.requested, y: 0 })!;
      for (const { index, x, y, geometry } of test.checks) {
        const after = next.shapes[index];
        expect(after.x).toBeCloseTo(x, 7);
        expect(after.y).toBeCloseTo(y, 7);
        expectGeometry(after.geometry, geometry as ShapeView["geometry"]);
        expect(after.style).toEqual(shapes[index].style);
      }
      const restored = previewStretch(next.group, next.shapes, 1, { x: test.length, y: 0 })!;
      shapes.forEach((shape, index) => {
        const actual = restored.shapes[index];
        expect(actual.x).toBeCloseTo(shape.x, 7);
        expect(actual.y).toBeCloseTo(shape.y, 7);
        expectGeometry(actual.geometry, shape.geometry);
      });
    });
  }
});

function expectGeometry(actual: ShapeView["geometry"], expected: ShapeView["geometry"]) {
  const numbers = (value: unknown): number[] =>
    typeof value === "number" ? [value] : value && typeof value === "object"
      ? Object.entries(value).sort(([a], [b]) => a.localeCompare(b)).flatMap(([, child]) => numbers(child)) : [];
  expect(actual.kind).toBe(expected.kind);
  const a = numbers(actual), b = numbers(expected);
  expect(a).toHaveLength(b.length);
  a.forEach((value, index) => expect(value).toBeCloseTo(b[index], 7));
}
