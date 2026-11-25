import { describe, expect, it } from "vitest";
import fixtures from "../../../../../fixtures/addon-data/port-positions.json";
import type { PageObject } from "../model";
import { addonPorts, nearbyPorts, pagePorts } from "./addonPorts";

describe("addon port positions shared with Rust", () => {
  for (const { name, object, ports } of fixtures) {
    it(name, () => {
      const actual = pagePorts(object as PageObject);
      expect(actual).toHaveLength(ports.length);
      for (const [index, port] of ports.entries()) {
        expect(actual[index].x).toBeCloseTo(port.x, 9);
        expect(actual[index].y).toBeCloseTo(port.y, 9);
      }
    });
  }
});

it("uses outer group ports, skips source relatives and owned paths, and preserves explicit empty ports", () => {
  const group = fixtures.find(({ name }) => name === "group with stored ports")!.object as PageObject;
  const child = fixtures[0].object as PageObject;
  const source = { ...child, id: "source" };
  const parts = [source, group, { ...child, id: "group-child", groupId: group.id },
    { ...child, id: "empty", ports: [] }, { ...child, id: "owned", addonData: { addon: { refs: [], data: null } } }];
  expect(addonPorts(parts, source.id).map(({ attach }) => attach.objectId)).toEqual(pagePorts(group).map(() => group.id));
  expect(addonPorts(parts, "group-child")).toEqual(addonPorts([source]));
});

it("uses a 12-screen-pixel snap limit and canonical port order for ties", () => {
  const ports = [{ x: -6, y: 0, attach: { objectId: "a", port: 0 } },
    { x: 6, y: 0, attach: { objectId: "b", port: 0 } }];
  const point = { x: 0, y: 0 };
  expect(nearbyPorts(point, ports, 2)).toEqual({ nearby: ports, snapped: ports[0] });
  expect(nearbyPorts(point, ports, 2.01)).toEqual({ nearby: ports, snapped: undefined });
});
