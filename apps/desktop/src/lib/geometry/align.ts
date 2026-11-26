import type { Bounds } from "../ink/selection";
import type { Point } from "./coordinates";

/**
 * Align and Distribute for a selection.
 *
 * Each unit is one thing that moves as a whole (a loose object, a whole group, the selected ink)
 * with its page-space bounds. Those bounds are the axis-aligned box around the rotated outline, so
 * a rotated object lines up by its visible extent rather than by its unrotated frame. Both
 * functions only compute how far each unit moves; applying the deltas is one batch elsewhere.
 */
export type AlignUnit = { id: string; box: Bounds };

export type AlignMode = "left" | "centre" | "right" | "top" | "middle" | "bottom";
export type DistributeAxis = "horizontal" | "vertical";

const NONE: Point = { x: 0, y: 0 };

/** The smallest box around all of `boxes`; the boxes must not be empty. */
export function unionBounds(boxes: readonly Bounds[]): Bounds {
  return {
    left: Math.min(...boxes.map((box) => box.left)),
    top: Math.min(...boxes.map((box) => box.top)),
    right: Math.max(...boxes.map((box) => box.right)),
    bottom: Math.max(...boxes.map((box) => box.bottom)),
  };
}

/**
 * Moves every unit to the left, right, top or bottom edge of the selection, or to its horizontal
 * or vertical centre line. Needs two units; fewer return no movement.
 */
export function alignDeltas(units: readonly AlignUnit[], mode: AlignMode): Map<string, Point> {
  const deltas = new Map<string, Point>();
  if (units.length < 2) return deltas;
  const all = unionBounds(units.map(({ box }) => box));
  for (const { id, box } of units) {
    deltas.set(id, {
      left: { x: all.left - box.left, y: 0 },
      right: { x: all.right - box.right, y: 0 },
      centre: { x: (all.left + all.right) / 2 - (box.left + box.right) / 2, y: 0 },
      top: { x: 0, y: all.top - box.top },
      bottom: { x: 0, y: all.bottom - box.bottom },
      middle: { x: 0, y: (all.top + all.bottom) / 2 - (box.top + box.bottom) / 2 },
    }[mode]);
  }
  return deltas;
}

/**
 * Spaces three or more units so the gaps between neighbours are equal. The two outermost units
 * stay where they are, and the rest are placed between them in order of their centres. When the
 * units are wider than the span the equal gaps are negative, which spreads the overlap evenly.
 */
export function distributeDeltas(units: readonly AlignUnit[], axis: DistributeAxis): Map<string, Point> {
  const deltas = new Map<string, Point>(units.map(({ id }) => [id, NONE]));
  if (units.length < 3) return deltas;
  const [near, far, size] = axis === "horizontal"
    ? ["left", "right", ({ left, right }: Bounds) => right - left] as const
    : ["top", "bottom", ({ top, bottom }: Bounds) => bottom - top] as const;
  const ordered = [...units].sort((a, b) => a.box[near] + a.box[far] - b.box[near] - b.box[far]);
  const first = ordered[0].box, last = ordered[ordered.length - 1].box;
  const free = last[far] - first[near] - ordered.reduce((sum, { box }) => sum + size(box), 0);
  const gap = free / (ordered.length - 1);
  let cursor = first[near];
  for (const { id, box } of ordered) {
    const shift = cursor - box[near];
    deltas.set(id, axis === "horizontal" ? { x: shift, y: 0 } : { x: 0, y: shift });
    cursor += size(box) + gap;
  }
  return deltas;
}
