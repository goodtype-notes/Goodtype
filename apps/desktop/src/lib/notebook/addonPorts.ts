import { transformPoint, type Point, type Similarity } from "../geometry/coordinates";
import type { PageObject, ShapeGeometry } from "../model";

/** What carries ports: a canonical shape or group, or a live shape view given `type: "shape"`. */
export type PortSource = Similarity & { type: string; ports?: Point[]; geometry?: ShapeGeometry };

function localPorts(object: PortSource): Point[] {
  if (object.type !== "shape" && object.type !== "group") return [];
  if (object.ports !== undefined) return object.ports;
  if (object.type === "group") return [];

  const geometry = object.geometry;
  if (!geometry) return [];
  switch (geometry.kind) {
    case "line":
      return [geometry.start, geometry.end];
    case "spline": {
      const first = geometry.nodes[0], last = geometry.nodes.at(-1);
      return first && last ? [first.point, last.point] : [];
    }
    case "ellipse":
    case "rectangle": {
      const { widthPt: w, heightPt: h } = geometry;
      const edges = [{ x: w / 2, y: 0 }, { x: w, y: h / 2 },
        { x: w / 2, y: h }, { x: 0, y: h / 2 }];
      if (geometry.kind === "rectangle") {
        edges.push({ x: 0, y: 0 }, { x: w, y: 0 }, { x: w, y: h }, { x: 0, y: h });
      }
      return edges;
    }
  }
}

export function pagePorts(object: PortSource): Point[] {
  return localPorts(object).map((port) => transformPoint(object, port));
}

export type Port = Point & { attach: { objectId: string; port: number } };

export function addonPorts(objects: readonly PageObject[], source?: string): Port[] {
  const index = new Map(objects.map((object) => [object.id, object]));
  const ancestors = (id: string) => {
    const parents: PageObject[] = [];
    let parent = index.get(id)?.groupId;
    while (parent) {
      const group = index.get(parent);
      if (!group) break;
      parents.push(group);
      parent = group.groupId;
    }
    return parents;
  };
  const sourceParents = source ? new Set(ancestors(source).map(({ id }) => id)) : new Set();
  return objects.flatMap((object) => {
    if (object.id === source || sourceParents.has(object.id)
      || (object.type === "shape" && (object.geometry.kind === "line" || object.geometry.kind === "spline" && !object.geometry.closed)
        && Object.keys(object.addonData ?? {}).length)
      || ancestors(object.id).some((group) => group.id === source || group.ports != null)) return [];
    return pagePorts(object).map((point, port) => ({ ...point, attach: { objectId: object.id, port } }));
  });
}

export function nearbyPorts(point: Point, ports: readonly Port[], zoom: number) {
  const nearby: Port[] = [];
  let snapped: Port | undefined;
  let limit = 12;
  for (const port of ports) {
    const distance = Math.hypot(port.x - point.x, port.y - point.y) * zoom;
    if (distance <= 32) nearby.push(port);
    // The canonical page order and each object's declared port order break ties.
    if (distance <= limit && (!snapped || distance < limit)) {
      snapped = port;
      limit = distance;
    }
  }
  return { nearby, snapped };
}
