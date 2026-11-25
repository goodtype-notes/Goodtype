import { rotatedVector, type Point } from "../geometry/coordinates";
import type { ShapePoint } from "../model";
import type { ShapeView } from "../notebook/pageView";
import { containsPoint, distanceToOutline, distanceToShapeCap, shapeCaps } from "./geometry";

/**
 * Whether an eraser — or anything else with a reach — touches a shape.
 *
 * The test is against the mark, not the bounding box: reaching into the middle of a large empty
 * rectangle should no more erase it than reaching past its edge does, because there is nothing
 * drawn there. A fill changes that, since a tint is part of what is on the page.
 *
 * Half the stroke width counts toward the reach, so a thick outline is as easy to catch as it
 * looks, and so is a hairline one — the eraser's own radius carries that case.
 */
export function shapeTouchedAt(shape: ShapeView, page: Point, reachPt: number): boolean {
  const local = toLocalPoint(shape, page);
  const scale = Math.max(Math.abs(shape.scale), 1e-6);
  // The point has been brought back through the shape's scale, so the reach must be too.
  const reach = (reachPt + shape.style.strokeWidthPt * scale * 0.5) / scale;
  const { shaft, caps } = shapeCaps(shape.geometry, shape.style);
  if (shaft && distanceToOutline(shaft, local) <= reach) return true;
  if (caps.some((cap) => distanceToShapeCap(cap, local) <= reachPt / scale)) return true;
  return shape.style.fillColor !== null && containsPoint(shape.geometry, local);
}

/**
 * The topmost shape under a point, or null.
 *
 * Ties go to the one added later, matching what is drawn over what on the page.
 */
export function topmostShapeAt(
  shapes: readonly ShapeView[],
  page: Point,
  reachPt: number,
): ShapeView | null {
  let found: ShapeView | null = null;
  for (const shape of shapes) {
    if (!shapeTouchedAt(shape, page, reachPt)) continue;
    if (!found || shape.zIndex >= found.zIndex) found = shape;
  }
  return found;
}

/// A point in the shape's own coordinates on the page: the transform every renderer applies,
/// `translate(x y) rotate(rotation) scale(scale)`. [`toLocalPoint`] is its inverse.
export function toPagePoint(shape: ShapeView, local: ShapePoint): Point {
  const { x, y } = rotatedVector({ x: local.x * shape.scale, y: local.y * shape.scale }, shape.rotation);
  return { x: x + shape.x, y: y + shape.y };
}

/// A page point in the shape's own coordinates: the inverse of the transform every renderer
/// applies, `translate(x y) rotate(rotation) scale(scale)`.
export function toLocalPoint(shape: ShapeView, page: Point): ShapePoint {
  const { x, y } = rotatedVector({ x: page.x - shape.x, y: page.y - shape.y }, -shape.rotation);
  const scale = Math.max(Math.abs(shape.scale), 1e-6);
  return { x: x / scale, y: y / scale };
}
