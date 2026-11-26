import { describe, expect, it } from "vitest";
import { boxOutline, combinedBounds } from "../notebook/lasso";
import { alignDeltas, distributeDeltas, type AlignUnit } from "./align";

const unit = (id: string, left: number, top: number, right: number, bottom: number): AlignUnit =>
  ({ id, box: { left, top, right, bottom } });
const moved = (u: AlignUnit, d: { x: number; y: number }) => ({
  left: u.box.left + d.x, right: u.box.right + d.x, top: u.box.top + d.y, bottom: u.box.bottom + d.y,
});
const turned = (id: string, x: number, y: number, width: number, height: number, rotation: number): AlignUnit =>
  ({ id, box: combinedBounds([{ id, kind: "shapes", closed: true, outline: boxOutline(x, y, width, height, rotation) }], null)! });

const units = [unit("a", 10, 40, 50, 60), unit("b", 100, 10, 120, 90), unit("c", 60, 70, 80, 130)];

describe("align", () => {
  it("lines every unit up with the selection's edge", () => {
    for (const [mode, key, expected] of [
      ["left", "left", 10], ["right", "right", 120], ["top", "top", 10], ["bottom", "bottom", 130],
    ] as const) {
      const deltas = alignDeltas(units, mode);
      for (const u of units) expect(moved(u, deltas.get(u.id)!)[key]).toBe(expected);
    }
  });

  it("centres every unit on the selection's centre line", () => {
    const horizontal = alignDeltas(units, "centre");
    for (const u of units) {
      const box = moved(u, horizontal.get(u.id)!);
      expect((box.left + box.right) / 2).toBe(65);
    }
    const vertical = alignDeltas(units, "middle");
    for (const u of units) {
      const box = moved(u, vertical.get(u.id)!);
      expect((box.top + box.bottom) / 2).toBe(70);
    }
  });

  it("moves along one axis only", () => {
    for (const delta of alignDeltas(units, "left").values()) expect(delta.y).toBe(0);
    for (const delta of alignDeltas(units, "top").values()) expect(delta.x).toBe(0);
  });

  it("needs two units", () => {
    expect(alignDeltas([units[0]], "left").size).toBe(0);
  });

  it("aligns a rotated object by its visible extent", () => {
    // A 100 x 20 bar turned 90 degrees about its origin stands 20 wide, its left edge at x - 20.
    const bar = turned("bar", 200, 50, 100, 20, 90);
    expect(bar.box.left).toBeCloseTo(180);
    expect(bar.box.right).toBeCloseTo(200);
    const flat = unit("flat", 20, 0, 70, 30);
    const deltas = alignDeltas([bar, flat], "left");
    expect(moved(bar, deltas.get("bar")!).left).toBeCloseTo(20);
    expect(deltas.get("flat")).toEqual({ x: 0, y: 0 });
  });

  it("centres a quarter-turned object on its rotated box, not its frame", () => {
    const diamond = turned("d", 100, 0, 100, 100, 45);
    const other = unit("s", 300, 0, 320, 20);
    const deltas = alignDeltas([diamond, other], "centre");
    const middle = (diamond.box.left + 320) / 2;
    const box = moved(diamond, deltas.get("d")!);
    expect((box.left + box.right) / 2).toBeCloseTo(middle);
    const small = moved(other, deltas.get("s")!);
    expect((small.left + small.right) / 2).toBeCloseTo(middle);
  });
});

describe("distribute", () => {
  it("leaves equal gaps horizontally and holds the outer units", () => {
    const row = [unit("a", 0, 0, 10, 10), unit("b", 12, 0, 42, 10), unit("c", 50, 0, 60, 10), unit("d", 190, 0, 200, 10)];
    const deltas = distributeDeltas(row, "horizontal");
    const placed = row.map((u) => moved(u, deltas.get(u.id)!));
    expect(placed[0].left).toBe(0);
    expect(placed[3].right).toBeCloseTo(200);
    const gaps = placed.slice(1).map((p, i) => p.left - placed[i].right);
    expect(gaps[0]).toBeCloseTo(gaps[1]);
    expect(gaps[1]).toBeCloseTo(gaps[2]);
    expect(gaps[0]).toBeCloseTo((200 - 10 - 30 - 10 - 10) / 3);
    for (const delta of deltas.values()) expect(delta.y).toBe(0);
  });

  it("spaces vertically in order of the centres whatever the selection order", () => {
    const column = [unit("c", 0, 80, 10, 90), unit("a", 0, 0, 10, 10), unit("b", 0, 20, 10, 60)];
    const deltas = distributeDeltas(column, "vertical");
    const byId = new Map(column.map((u) => [u.id, moved(u, deltas.get(u.id)!)]));
    expect(byId.get("a")!.top).toBe(0);
    expect(byId.get("c")!.bottom).toBe(90);
    expect(byId.get("b")!.top - byId.get("a")!.bottom).toBeCloseTo(byId.get("c")!.top - byId.get("b")!.bottom);
    for (const delta of deltas.values()) expect(delta.x).toBe(0);
  });

  it("needs three units", () => {
    const two = distributeDeltas([unit("a", 0, 0, 10, 10), unit("b", 50, 0, 60, 10)], "horizontal");
    expect([...two.values()]).toEqual([{ x: 0, y: 0 }, { x: 0, y: 0 }]);
  });

  it("spreads overlap evenly when the units are wider than the span", () => {
    const row = [unit("a", 0, 0, 40, 10), unit("b", 10, 0, 50, 10), unit("c", 20, 0, 60, 10)];
    const deltas = distributeDeltas(row, "horizontal");
    const placed = row.map((u) => moved(u, deltas.get(u.id)!));
    expect(placed[1].left - placed[0].right).toBeCloseTo(placed[2].left - placed[1].right);
    expect(placed[0].left).toBe(0);
    expect(placed[2].right).toBe(60);
  });

  it("distributes rotated objects by their visible extent", () => {
    const row = [unit("a", 0, 0, 10, 10), turned("t", 150, 0, 80, 20, 90), unit("c", 300, 0, 310, 10)];
    const deltas = distributeDeltas(row, "horizontal");
    const placed = row.map((u) => moved(u, deltas.get(u.id)!));
    expect(placed[1].left - placed[0].right).toBeCloseTo(placed[2].left - placed[1].right);
    expect(placed[1].right - placed[1].left).toBeCloseTo(20);
  });
});
