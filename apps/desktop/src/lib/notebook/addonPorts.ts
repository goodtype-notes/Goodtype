import { rotatedVector, type Point } from "../geometry/coordinates";
import type { PageObject } from "../model";

function localPorts(object: PageObject): Point[] {
  if (object.type !== "shape" && object.type !== "group") return [];
  if (object.ports !== undefined) return object.ports;
  if (object.type === "group") return [];

  const geometry = object.geometry;
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

export function pagePorts(object: PageObject): Point[] {
  return localPorts(object).map((port) => {
    const offset = rotatedVector({ x: port.x * object.scale, y: port.y * object.scale }, object.rotation);
    return { x: object.x + offset.x, y: object.y + offset.y };
  });
}
