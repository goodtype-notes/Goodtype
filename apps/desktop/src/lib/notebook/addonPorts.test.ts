import { describe, expect, it } from "vitest";
import fixtures from "../../../../../fixtures/addon-data/port-positions.json";
import type { PageObject } from "../model";
import { pagePorts } from "./addonPorts";

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
