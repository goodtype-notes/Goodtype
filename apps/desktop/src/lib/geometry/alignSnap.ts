import type { Bounds } from "../ink/selection";
import { viewportToPage, type Point } from "./coordinates";

/**
 * Alignment snapping for moved objects.
 *
 * A drag builds an index of everything else on the page once, at pointer-down, and every pointer
 * sample is a lookup in it. Guides are the ephemeral lines the lookup reports: they live in the
 * frontend, in page points, and are never written to the notebook.
 */

/** How close, in screen pixels, a moved object has to come to another before it meets it. */
export const ALIGN_SNAP_SCREEN_PX = 6;

/** Another object, or the group of objects that moves with it: its page-space box and its ports. */
export type SnapFrame = {
  /** Every canonical id the frame stands for, so a drag can leave itself out. */
  ids: readonly string[];
  /** Axis-aligned bounds of the rotated object, in page points. */
  box: Bounds;
  /** Connection points on the page, in page points. */
  ports: readonly Point[];
};

export type SnapAxis = "x" | "y";

/** A line the moved box now shares with another object, along `axis` at `at`, from `from` to `to`. */
export type SnapGuide = { axis: SnapAxis; at: number; from: number; to: number };

type Feature = { value: number; lo: number; hi: number };

type Index = Record<SnapAxis, Feature[]>;

export type SnapSession = {
  /** Other objects' features, sorted by value, per axis. */
  readonly targets: Index;
  /** What is being moved, at the grab. */
  readonly moving: { readonly box: Bounds; readonly ports: readonly Point[] };
};

/** The snap distance on the page for a zoom, through the same conversion the pointer uses. */
export function snapThresholdPt(zoom: number): number {
  return viewportToPage({ x: ALIGN_SNAP_SCREEN_PX, y: 0 }, { pageOriginX: 0, pageOriginY: 0, zoom }).x;
}

/** Left, centre and right in x, top, middle and bottom in y, then each port, as one-axis features. */
function features(box: Bounds, ports: readonly Point[]): Index {
  const centre = { x: (box.left + box.right) / 2, y: (box.top + box.bottom) / 2 };
  return {
    x: [
      ...[box.left, centre.x, box.right].map((value) => ({ value, lo: box.top, hi: box.bottom })),
      ...ports.map((port) => ({ value: port.x, lo: port.y, hi: port.y })),
    ],
    y: [
      ...[box.top, centre.y, box.bottom].map((value) => ({ value, lo: box.left, hi: box.right })),
      ...ports.map((port) => ({ value: port.y, lo: port.x, hi: port.x })),
    ],
  };
}

/**
 * The index for one drag: every frame that is not being moved, as sorted candidates, plus the
 * moving box and ports at the grab. A frame that shares an id with the dragged set is dropped, so
 * an object never snaps to itself or to the group it moves with.
 */
export function beginSnap(
  frames: readonly SnapFrame[],
  moving: { ids: ReadonlySet<string>; box: Bounds; ports?: readonly Point[] },
): SnapSession {
  const targets: Index = { x: [], y: [] };
  for (const frame of frames) {
    if (frame.ids.some((id) => moving.ids.has(id))) continue;
    const found = features(frame.box, frame.ports);
    targets.x.push(...found.x);
    targets.y.push(...found.y);
  }
  targets.x.sort((a, b) => a.value - b.value);
  targets.y.sort((a, b) => a.value - b.value);
  return { targets, moving: { box: moving.box, ports: moving.ports ?? [] } };
}

/** The feature nearest `value` in a sorted list, by binary search; null for an empty list. */
function nearest(sorted: readonly Feature[], value: number): Feature | null {
  if (sorted.length === 0) return null;
  let low = 0, high = sorted.length - 1;
  while (low < high) {
    const middle = (low + high) >> 1;
    if (sorted[middle].value < value) low = middle + 1;
    else high = middle;
  }
  const after = sorted[low], before = sorted[low - 1];
  return before && Math.abs(before.value - value) <= Math.abs(after.value - value) ? before : after;
}

/** The smallest correction, within `threshold`, that puts a moving feature on a target. */
function bestCorrection(moving: readonly Feature[], targets: readonly Feature[], threshold: number): number | null {
  let correction: number | null = null;
  for (const feature of moving) {
    const target = nearest(targets, feature.value);
    if (!target) continue;
    const gap = target.value - feature.value;
    if (Math.abs(gap) <= threshold && (correction === null || Math.abs(gap) < Math.abs(correction))) correction = gap;
  }
  return correction;
}

const SAME = 1e-6;

function guidesFor(axis: SnapAxis, moving: readonly Feature[], targets: readonly Feature[]): SnapGuide[] {
  const found = new Map<number, SnapGuide>();
  for (const feature of moving) {
    for (const target of targets) {
      if (Math.abs(target.value - feature.value) > SAME) continue;
      const lines = found.get(target.value);
      const from = Math.min(feature.lo, target.lo), to = Math.max(feature.hi, target.hi);
      found.set(target.value, lines
        ? { ...lines, from: Math.min(lines.from, from), to: Math.max(lines.to, to) }
        : { axis, at: target.value, from, to });
    }
  }
  return [...found.values()];
}

function shifted(box: Bounds, ports: readonly Point[], delta: Point): Index {
  return features(
    { left: box.left + delta.x, right: box.right + delta.x, top: box.top + delta.y, bottom: box.bottom + delta.y },
    ports.map((port) => ({ x: port.x + delta.x, y: port.y + delta.y })),
  );
}

/**
 * The delta a drag asks for, pulled onto the nearest edge, centre or port of another object within
 * `thresholdPt`, x and y independently, and the guides that result. With no match an axis keeps its
 * own delta. Pure lookup: it allocates no index and never touches the notebook.
 *
 * `constrain` is whatever limit the object applies after snapping (the page clamp). The guides are
 * measured at the delta it returns, so a clamp that moves the snapped position leaves no guide
 * marking a spot the object did not reach.
 */
export function snapMoveDelta(
  session: SnapSession,
  delta: Point,
  thresholdPt: number,
  constrain?: (delta: Point) => Point,
): { delta: Point; guides: SnapGuide[] } {
  const moving = shifted(session.moving.box, session.moving.ports, delta);
  const dx = bestCorrection(moving.x, session.targets.x, thresholdPt);
  const dy = bestCorrection(moving.y, session.targets.y, thresholdPt);
  const snapped = { x: delta.x + (dx ?? 0), y: delta.y + (dy ?? 0) };
  const final = constrain ? constrain(snapped) : snapped;
  if (dx === null && dy === null) return { delta: final, guides: [] };
  const after = shifted(session.moving.box, session.moving.ports, final);
  return {
    delta: final,
    guides: [
      ...(dx === null ? [] : guidesFor("x", after.x, session.targets.x)),
      ...(dy === null ? [] : guidesFor("y", after.y, session.targets.y)),
    ],
  };
}

/** Whether two guide lists draw the same lines, so an unchanged sample causes no redraw. */
export function sameGuides(a: readonly SnapGuide[], b: readonly SnapGuide[]): boolean {
  return a.length === b.length && a.every((guide, i) =>
    guide.axis === b[i].axis && guide.at === b[i].at && guide.from === b[i].from && guide.to === b[i].to);
}

/**
 * A drag's snapping, held for its duration. `begin` builds the index, `adjust` snaps one pointer
 * delta (Alt or the setting off passes it through unsnapped and clears the guides), `end` drops it.
 * A `constrain` callback is applied to whatever delta results, snapped or not.
 * Guides are reported through `onGuides` only when they change.
 */
export function createMoveSnapper(host: {
  enabled: () => boolean;
  zoom: () => number;
  onGuides: (guides: SnapGuide[]) => void;
}) {
  let session: SnapSession | null = null;
  let shown: SnapGuide[] = [];
  const show = (guides: SnapGuide[]) => {
    if (sameGuides(shown, guides)) return;
    shown = guides;
    host.onGuides(guides);
  };
  return {
    /** Builds the index only when snapping is on, so a drag with it off pays nothing. */
    begin(build: () => SnapSession | null) {
      session = host.enabled() ? build() : null;
    },
    adjust(delta: Point, alt: boolean, constrain?: (delta: Point) => Point): Point {
      if (!session || alt || !host.enabled()) {
        show([]);
        return constrain ? constrain(delta) : delta;
      }
      const result = snapMoveDelta(session, delta, snapThresholdPt(host.zoom()), constrain);
      show(result.guides);
      return result.delta;
    },
    end() {
      session = null;
      show([]);
    },
    get active() { return session !== null; },
  };
}

export type MoveSnapper = ReturnType<typeof createMoveSnapper>;

/** What the object components get: begin a drag of these ids, adjust each delta, end it. */
export type ObjectMoveSnap = {
  begin: (ids: readonly string[], strokeIds?: readonly string[]) => void;
  adjust: (delta: Point, alt: boolean, constrain?: (delta: Point) => Point) => Point;
  end: () => void;
};

/** A palette item starts at centre zero; reuse this index while its pointer moves. */
export function beginPlacementSnap(frames: readonly SnapFrame[], item: {
  widthPt: number; heightPt: number; ports?: readonly Point[];
}): SnapSession {
  const left = -item.widthPt / 2, top = -item.heightPt / 2;
  return beginSnap(frames, { ids: new Set(),
    box: { left, top, right: -left, bottom: -top },
    ports: item.ports?.map(port => ({ x: left + port.x, y: top + port.y })),
  });
}

/** Returns the insertion centre and guides, using the same lookup as an object move. */
export function snapPlacement(session: SnapSession, centre: Point, zoom: number, enabled: boolean, alt: boolean) {
  if (!enabled || alt) return { point: centre, guides: [] as SnapGuide[] };
  const result = snapMoveDelta(session, centre, snapThresholdPt(zoom));
  return { point: result.delta, guides: result.guides };
}
