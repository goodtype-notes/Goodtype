import type { Point } from "../../geometry/coordinates";
import type { PackShape, ShapeChoice, ShapeLibraryModel } from "./content";

export type ShapeRow = ShapeLibraryModel[number]["items"][number];
export type ShapeDrag = { phase: "move" | "drop" | "cancel"; row: ShapeRow; x: number; y: number; alt?: boolean };

export const RECENT_LIMIT = 8;
const RECENT_KEY = "goodtype.shapePanel.recent";

export const shapeKey = (choice: ShapeChoice) => [choice.addonId, choice.libraryId, choice.itemId].join("/");
export const sameShape = (a: ShapeChoice | null | undefined, b: ShapeChoice | null | undefined) =>
  Boolean(a && b && shapeKey(a) === shapeKey(b));

/** The most recent placement first, without duplicates, at most `limit` entries. */
export function rememberShape(recent: readonly string[], choice: ShapeChoice, limit = RECENT_LIMIT): string[] {
  const key = shapeKey(choice);
  return [key, ...recent.filter(entry => entry !== key)].slice(0, limit);
}

/** Recent keys resolved against what is installed now; removed packs simply drop out. */
export function recentRows(libraries: ShapeLibraryModel, recent: readonly string[]) {
  const rows = new Map(libraries.flatMap(library => library.items.map(row => [shapeKey(row.choice), row] as const)));
  return recent.flatMap(key => rows.get(key) ?? []);
}

/** Recently used is a per-device convenience, so storage that is missing or full just forgets. */
export function loadRecent(storage: Pick<Storage, "getItem"> | undefined = globalThis.localStorage): string[] {
  try {
    const value: unknown = JSON.parse(storage?.getItem(RECENT_KEY) ?? "[]");
    return Array.isArray(value) ? value.filter((entry): entry is string => typeof entry === "string").slice(0, RECENT_LIMIT) : [];
  } catch {
    return [];
  }
}

export function saveRecent(recent: readonly string[], storage: Pick<Storage, "setItem"> | undefined = globalThis.localStorage) {
  try {
    storage?.setItem(RECENT_KEY, JSON.stringify(recent));
  } catch {
    // Private windows and full storage keep the list for this session only.
  }
}

export type StampEvent =
  | { type: "tile"; choice: ShapeChoice }
  | { type: "escape" | "tool" | "close" };

/** Tapping a tile starts stamping it; tapping it again, Esc, another tool or closing the panel stops. */
export function nextStamp(current: ShapeChoice | null, event: StampEvent): ShapeChoice | null {
  if (event.type !== "tile") return null;
  return sameShape(current, event.choice) ? null : event.choice;
}

export const TAP_SLOP_PX = 8;
export const TAP_MAX_MS = 450;

/**
 * A press on the page in stamp mode places one copy when it lifts close to where it went down.
 * A finger that travels is a pan, a finger that stays is a long press, and a second finger is a
 * pinch or a history tap; none of those place anything.
 */
export function createTapTracker() {
  let press: { pointerId: number; x: number; y: number; at: number; pageId: string } | null = null;
  return {
    down(pointerId: number, x: number, y: number, at: number, pageId: string) {
      press = press ? null : { pointerId, x, y, at, pageId };
    },
    move(pointerId: number, x: number, y: number) {
      if (press?.pointerId === pointerId && Math.hypot(x - press.x, y - press.y) > TAP_SLOP_PX) press = null;
    },
    /** The press position for a tap, or null when this lift is not one. */
    up(pointerId: number, x: number, y: number, at: number, cancelled = false) {
      const done = press;
      if (done?.pointerId !== pointerId) return null;
      press = null;
      if (cancelled || at - done.at > TAP_MAX_MS || Math.hypot(x - done.x, y - done.y) > TAP_SLOP_PX) return null;
      return { x: done.x, y: done.y, pageId: done.pageId };
    },
    owns: (pointerId: number) => press?.pointerId === pointerId,
    cancel() { press = null; },
  };
}

export const DRAG_SLOP_PX = 6;
export const TOUCH_HOLD_MS = 350;

/**
 * Whether a press on a tile has become a drag. Mouse and pen drag after a few pixels in any
 * direction. A finger scrolls the list vertically (the tile allows `pan-y`), so it drags when it
 * moves sideways, towards the page, or after it was held still.
 */
export function dragStarts(pointerType: string, dx: number, dy: number, heldMs: number): boolean {
  const distance = Math.hypot(dx, dy);
  if (pointerType !== "touch") return distance > DRAG_SLOP_PX;
  if (heldMs >= TOUCH_HOLD_MS) return true;
  return distance > DRAG_SLOP_PX && Math.abs(dx) > Math.abs(dy) * 1.5;
}

type Rect = Pick<DOMRect, "left" | "top" | "right" | "bottom">;

/**
 * The page point under a drop or stamp, through the one coordinate authority. A pointer outside
 * the page frame places nothing rather than being pulled onto the edge.
 */
export function dropPoint(
  navigation: { pagePointAt(clientX: number, clientY: number): Point | null },
  frame: Rect | undefined,
  clientX: number,
  clientY: number,
): Point | null {
  if (!frame || clientX < frame.left || clientX > frame.right || clientY < frame.top || clientY > frame.bottom) return null;
  return navigation.pagePointAt(clientX, clientY);
}

/** The margin `PackShapePreview` draws around an item, so a ghost can be sized to match the page. */
export const previewMargin = (item: Pick<PackShape, "widthPt" | "heightPt">, widthPt: number) =>
  Math.max(item.widthPt, item.heightPt) * 0.12 + widthPt;
