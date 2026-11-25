import { translate, type Translator } from "../i18n";
import type { Point } from "../geometry/coordinates";
import { rotatedVector } from "../geometry/coordinates";
import type { Bounds } from "../ink/selection";
import { polylineInLasso } from "../ink/selection";
import { shapeCaps, splinePolyline } from "../shape/geometry";
import { toPagePoint } from "../shape/hit";
import type { ShapeView } from "./pageView";

export type LassoTargets = { handwriting: boolean; highlighter: boolean; shapes: boolean; images: boolean; text: boolean };
export const DEFAULT_LASSO_TARGETS: LassoTargets = { handwriting: true, highlighter: true, shapes: true, images: true, text: true };
export function lassoOptions(t: Translator = translate) { return [
  { key: "handwriting", label: t("workspace.handwriting") },
  { key: "highlighter", label: t("workspace.highlighter") },
  { key: "shapes", label: t("workspace.shapes") },
  { key: "images", label: t("workspace.images") },
  { key: "text", label: t("workspace.textBlocks") },
] as const; }
export const LASSO_OPTIONS = lassoOptions();
type LassoOutline = { outline: Point[]; closed: boolean };
export type LassoItem = LassoOutline & {
  id: string;
  kind: "shapes" | "images" | "text";
  caps?: LassoOutline[];
};

export function boxOutline(x: number, y: number, width: number, height: number, rotation = 0): Point[] {
  return [{ x: 0, y: 0 }, { x: width, y: 0 }, { x: width, y: height }, { x: 0, y: height }]
    .map(point => { const local = rotatedVector(point, rotation); return { x: x + local.x, y: y + local.y }; });
}

export function shapeLassoItem(shape: ShapeView): LassoItem {
  const { shaft: g, caps } = shapeCaps(shape.geometry, shape.style);
  const local = !g ? [] : g.kind === "line" ? [g.start, g.end]
    : g.kind === "spline" ? splinePolyline(g.nodes, g.closed, 16)
    : g.kind === "ellipse" ? Array.from({ length: 64 }, (_, i) => ({
      x: g.widthPt * (1 + Math.cos(i * Math.PI / 32)) / 2,
      y: g.heightPt * (1 + Math.sin(i * Math.PI / 32)) / 2,
    })) : boxOutline(0, 0, g.widthPt, g.heightPt);
  const pagePoint = (point: Point) => toPagePoint(shape, point);
  const capOutlines = caps.flatMap((cap): LassoOutline[] => {
    const points = cap.points.map(pagePoint);
    const radius = cap.radius * Math.abs(shape.scale);
    if (cap.kind === "arrow") return [{ outline: points, closed: true }];
    const ends = points.map(({ x, y }) => ({
      closed: true,
      outline: Array.from({ length: 16 }, (_, i) => ({
        x: x + radius * Math.cos(i * Math.PI / 8),
        y: y + radius * Math.sin(i * Math.PI / 8),
      })),
    }));
    const segments = points.slice(1).map((end, i) => {
      const start = points[i];
      const length = Math.hypot(end.x - start.x, end.y - start.y) || 1;
      const dx = (start.y - end.y) * radius / length;
      const dy = (end.x - start.x) * radius / length;
      return {
        closed: true,
        outline: [
          { x: start.x + dx, y: start.y + dy }, { x: end.x + dx, y: end.y + dy },
          { x: end.x - dx, y: end.y - dy }, { x: start.x - dx, y: start.y - dy },
        ],
      };
    });
    return [...ends, ...segments];
  });
  return {
    id: shape.id, kind: "shapes", outline: local.map(pagePoint),
    closed: !!g && g.kind !== "line" && (g.kind !== "spline" || g.closed),
    ...(capOutlines.length ? { caps: capOutlines } : {}),
  };
}

export function selectLassoObjects(items: LassoItem[], polygon: Point[], targets: LassoTargets): string[] {
  return items.filter((item) => targets[item.kind] && [item, ...(item.caps ?? [])].some(
    (part) => polylineInLasso(part.outline, polygon, part.closed),
  )).map((item) => item.id);
}

export function combinedBounds(items: LassoItem[], ink: Bounds | null): Bounds | null {
  const points = items.flatMap((item) => [item, ...(item.caps ?? [])].flatMap((part) => part.outline));
  if (ink) points.push({ x: ink.left, y: ink.top }, { x: ink.right, y: ink.bottom });
  return points.length ? { left: Math.min(...points.map(p => p.x)), top: Math.min(...points.map(p => p.y)),
    right: Math.max(...points.map(p => p.x)), bottom: Math.max(...points.map(p => p.y)) } : null;
}

export function transformSelectionPoint(point: Point, delta: Point, scale: number, anchor: Point): Point {
  return { x: anchor.x + (point.x - anchor.x) * scale + delta.x, y: anchor.y + (point.y - anchor.y) * scale + delta.y };
}
