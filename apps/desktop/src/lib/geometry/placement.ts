// Keeping objects reachable on the page.
//
// An object dragged fully past an edge has nothing left to hit, so it cannot be selected, moved
// back, or deleted — it is lost while still being in the file and still being exported. Partial
// overhang is useful and stays allowed; total escape does not.

/** How much of an object must stay on the page. Roughly a fingertip, so it is always grabbable. */
export const MIN_VISIBLE_PT = 24;
import { rotatedVector } from "./coordinates";

export type PageBox = { widthPt: number; heightPt: number };
export type ViewRect = {
  left: number;
  top: number;
  right: number;
  bottom: number;
  width: number;
  height: number;
};

/**
 * Clamp a top-left position so at least `MIN_VISIBLE_PT` of the object stays over the page on
 * each axis.
 *
 * Objects smaller than the minimum are held entirely on the page rather than being allowed to
 * hang off by their own size, which is what the naive bound would do.
 */
export function keepOnPage(
  position: { x: number; y: number },
  size: { widthPt: number; heightPt: number },
  page: PageBox,
  minimumVisiblePt = MIN_VISIBLE_PT,
  rotation = 0,
): { x: number; y: number } {
  if (rotation) {
    const corners = [{ x: 0, y: 0 }, { x: size.widthPt, y: 0 },
      { x: size.widthPt, y: size.heightPt }, { x: 0, y: size.heightPt }].map(point => rotatedVector(point, rotation));
    const left = Math.min(...corners.map(point => point.x)), top = Math.min(...corners.map(point => point.y));
    const width = Math.max(...corners.map(point => point.x)) - left;
    const height = Math.max(...corners.map(point => point.y)) - top;
    return { x: clampAxis(position.x + left, width, page.widthPt, minimumVisiblePt) - left,
      y: clampAxis(position.y + top, height, page.heightPt, minimumVisiblePt) - top };
  }
  return {
    x: clampAxis(position.x, size.widthPt, page.widthPt, minimumVisiblePt),
    y: clampAxis(position.y, size.heightPt, page.heightPt, minimumVisiblePt),
  };
}

function clampAxis(value: number, extent: number, pageExtent: number, minimum: number): number {
  if (!Number.isFinite(value)) return 0;
  const visible = Math.min(minimum, Math.max(extent, 0));
  const lowest = visible - Math.max(extent, 0);
  const highest = pageExtent - visible;
  // A page narrower than the object leaves the bounds crossed; pinning to the low end keeps the
  // object's leading edge on the sheet rather than snapping it somewhere arbitrary.
  if (highest < lowest) return lowest;
  return Math.min(Math.max(value, lowest), highest);
}

type Box = Pick<ViewRect, "left" | "top" | "right" | "bottom">;

function overlaps(a: Box, b: Box): boolean {
  return a.left < b.right && b.left < a.right && a.top < b.bottom && b.top < a.bottom;
}

/**
 * Place a floating control above its selected object, or below when the top edge is too close.
 * Coordinates are relative to `boundary`, ready for an absolutely positioned child.
 *
 * `avoid` lists other floating chrome in window coordinates (the tool palette, the undo card).
 * When the preferred side would land on one of them and the other side would not, the control
 * takes the other side, so two floating surfaces never stack on each other.
 */
export function placeFloatingToolbar(
  anchor: ViewRect,
  toolbar: Pick<ViewRect, "width" | "height">,
  boundary: ViewRect,
  gap = 10,
  margin = 12,
  avoid: Box[] = [],
): { left: number; top: number; side: "above" | "below" } {
  const left = Math.min(
    Math.max(anchor.left + (anchor.width - toolbar.width) / 2, boundary.left + margin),
    boundary.right - toolbar.width - margin,
  );
  const topFor = (side: "above" | "below") => Math.min(
    Math.max(side === "above" ? anchor.top - toolbar.height - gap : anchor.bottom + gap, boundary.top + margin),
    boundary.bottom - toolbar.height - margin,
  );
  const hit = (top: number) =>
    avoid.find(box => overlaps({ left, top, right: left + toolbar.width, bottom: top + toolbar.height }, box));
  const inside = (top: number) => top >= boundary.top + margin && top + toolbar.height <= boundary.bottom - margin;
  let side: "above" | "below" = anchor.top - toolbar.height - gap >= boundary.top + margin ? "above" : "below";
  const other = side === "above" ? "below" : "above";
  if (hit(topFor(side)) && !hit(topFor(other))) side = other;
  let top = topFor(side);
  // Blocked on both sides (a selection tucked under the palette): step past what is in the way,
  // downward first, then upward, as long as the bar stays inside the boundary.
  for (const direction of ["down", "up"] as const) {
    let candidate = top;
    for (let box = hit(candidate), steps = 0; box && steps < avoid.length; box = hit(candidate), steps++) {
      candidate = direction === "down" ? box.bottom + gap : box.top - gap - toolbar.height;
    }
    if (!hit(candidate) && inside(candidate)) { top = candidate; break; }
  }
  return {
    left: left - boundary.left,
    top: top - boundary.top,
    side: top >= anchor.top ? "below" : "above",
  };
}
