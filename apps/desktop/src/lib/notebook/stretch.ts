import { inverseTransformPoint, rotatedVector, transformPoint, type Point, type Similarity } from "../geometry/coordinates";
import type { PageObject, StretchRule } from "../model";
import type { ShapeView } from "./pageView";

export type StretchGroup = Extract<PageObject, { type: "group" }>;
export type StretchPreview = { group: StretchGroup; shapes: ShapeView[] };
const EPS = 1e-6;
const close = (a: Point, b: Point) => Math.abs(a.x - b.x) <= EPS && Math.abs(a.y - b.y) <= EPS;

function minimum(limits: StretchRule["limits"]): number {
  return limits.mode === "span" ? limits.minimumLength : limits.socketSeparation + limits.leadFloors[0] + limits.leadFloors[1];
}

function sockets(limits: StretchRule["limits"], length: number): [number, number] {
  if (limits.mode !== "fixedBody") throw new Error("Invalid fixed-body rule");
  const start = limits.leadFloors[0] + Math.max(0, length - minimum(limits)) * limits.startShare;
  return [start, start + limits.socketSeparation];
}

export function stretchFrame(group: Similarity, a: Point, b: Point, startMoved: boolean, endpoint: Point, minimumLength: number) {
  const fixed = transformPoint(group, startMoved ? b : a);
  const direction = startMoved ? -1 : 1;
  const dx = (endpoint.x - fixed.x) * direction, dy = (endpoint.y - fixed.y) * direction;
  const distance = Math.hypot(dx, dy);
  const length = Math.max(distance / group.scale, minimumLength);
  const rotation = distance === 0 ? group.rotation : Math.atan2(dy, dx) * 180 / Math.PI;
  const end = { x: a.x + length, y: a.y };
  const anchor = transformPoint({ x: 0, y: 0, rotation, scale: group.scale }, startMoved ? end : a);
  return { frame: { x: fixed.x - anchor.x, y: fixed.y - anchor.y, rotation, scale: group.scale }, length, end };
}

/** Always derive from the gesture-start shapes. The saved result is computed independently in Rust. */
export function previewStretch(group: StretchGroup, shapes: readonly ShapeView[], port: number, endpoint: Point): StretchPreview | null {
  const { stretch, ports } = group;
  if (!stretch || !ports || !stretch.axis.includes(port)) return null;
  const [startIndex, endIndex] = stretch.axis;
  const a = ports[startIndex], b = ports[endIndex];
  const { limits } = stretch;
  const { frame, length, end } = stretchFrame(group, a, b, port === startIndex, endpoint, minimum(limits));
  if (close(transformPoint(frame, a), transformPoint(group, a))
      && close(transformPoint(frame, end), transformPoint(group, b))) return null;
  const oldLength = b.x - a.x;
  const ratio = length / oldLength;
  const parts: ShapeView[] = [];
  for (const part of stretch.parts) {
    const child = shapes.find(shape => shape.id === part.childId);
    if (!child) return null;
    let { geometry } = child;
    const origin = inverseTransformPoint(group, child);
    const relative = child.rotation - group.rotation;
    if (part.role === "body") {
      origin.x += sockets(limits, length)[0] - sockets(limits, oldLength)[0];
    } else if (part.role === "span") {
      origin.x = a.x + (origin.x - a.x) * ratio;
      const axial = (point: Point) => {
        const projected = rotatedVector(point, relative);
        return rotatedVector({ x: projected.x * ratio, y: projected.y }, -relative);
      };
      switch (geometry.kind) {
        case "line":
          geometry = { ...geometry, start: axial(geometry.start), end: axial(geometry.end) };
          break;
        case "rectangle":
          geometry = { ...geometry, widthPt: geometry.widthPt * ratio };
          break;
        case "spline":
          geometry = { ...geometry, nodes: geometry.nodes.map(node => ({
            point: axial(node.point),
            handleIn: node.handleIn ? axial(node.handleIn) : node.handleIn,
            handleOut: node.handleOut ? axial(node.handleOut) : node.handleOut,
          })) };
          break;
        default:
          return null;
      }
    }
    const next = { ...child, ...transformPoint(frame, origin), rotation: frame.rotation + relative };
    if (part.role === "startLead" || part.role === "endLead" || part.role === "throughLine") {
      if (geometry.kind !== "line") return null;
      const side = part.role === "endLead" ? 1 : 0;
      const port = [a, end][side];
      const inner = part.role === "throughLine" ? end : { x: a.x + sockets(limits, length)[side], y: a.y };
      const startWasPort = close(inverseTransformPoint(group, transformPoint(child, geometry.start)), [a, b][side]);
      const [p, q] = [port, inner].map(point => inverseTransformPoint(next, transformPoint(frame, point)));
      geometry = { ...geometry, start: startWasPort ? p : q, end: startWasPort ? q : p };
    }
    parts.push({ ...next, geometry });
  }
  return { group: { ...group, ...frame, ports: ports.map((point, index) => index === endIndex ? end : point) }, shapes: parts };
}
