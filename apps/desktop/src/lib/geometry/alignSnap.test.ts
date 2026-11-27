import { describe, expect, it, vi } from "vitest";
import {
  beginPlacementSnap, snapPlacement, beginSnap, createMoveSnapper, snapMoveDelta, snapThresholdPt, ALIGN_SNAP_SCREEN_PX,
  type SnapFrame, type SnapGuide,
} from "./alignSnap";

type Box = { left: number; top: number; right: number; bottom: number };
const box = (left: number, top: number, right: number, bottom: number): Box => ({ left, top, right, bottom });
const frame = (id: string, b: Box, ports: { x: number; y: number }[] = []): SnapFrame =>
  ({ ids: [id], box: b, ports });
const moving = (b: Box, ports: { x: number; y: number }[] = [], ids = ["drag"]) =>
  ({ ids: new Set(ids), box: b, ports });

describe("alignment snapping", () => {
  it("converts the screen threshold to page points through the zoom", () => {
    expect(snapThresholdPt(1)).toBe(ALIGN_SNAP_SCREEN_PX);
    expect(snapThresholdPt(2)).toBe(3);
    expect(snapThresholdPt(0.5)).toBe(12);
    expect(snapThresholdPt(8)).toBeCloseTo(0.75);
  });

  it("snaps a left edge to another object's left edge", () => {
    const session = beginSnap([frame("a", box(100, 0, 200, 50))], moving(box(0, 100, 80, 150)));
    const { delta, guides } = snapMoveDelta(session, { x: 103, y: 0 }, 6);
    expect(delta).toEqual({ x: 100, y: 0 });
    expect(guides).toEqual([{ axis: "x", at: 100, from: 0, to: 150 }]);
  });

  it("snaps centres and right edges", () => {
    const session = beginSnap([frame("a", box(100, 0, 200, 50))], moving(box(0, 100, 40, 150)));
    // The moving box's centre (20) reaches 150 at delta 130.
    expect(snapMoveDelta(session, { x: 128, y: 0 }, 6).delta.x).toBe(130);
    // Its right edge (40) reaches 200 at delta 160.
    expect(snapMoveDelta(session, { x: 157, y: 0 }, 6).delta.x).toBe(160);
  });

  it("snaps the top, middle and bottom the same way", () => {
    const session = beginSnap([frame("a", box(0, 100, 50, 200))], moving(box(300, 0, 340, 40)));
    expect(snapMoveDelta(session, { x: 0, y: 98 }, 6).delta.y).toBe(100);
    expect(snapMoveDelta(session, { x: 0, y: 131 }, 6).delta.y).toBe(130);
    expect(snapMoveDelta(session, { x: 0, y: 162 }, 6).delta.y).toBe(160);
  });

  it("snaps a port to another port's coordinate so a wire comes out straight", () => {
    const session = beginSnap(
      [frame("a", box(0, 100, 40, 180), [{ x: 40, y: 122 }])],
      moving(box(300, 300, 340, 340), [{ x: 300, y: 327 }]),
    );
    // The boxes' own edges (100, 140, 180) are all out of reach; only the ports can meet.
    const { delta, guides } = snapMoveDelta(session, { x: 0, y: -207 }, 6);
    expect(delta.y).toBe(-205);
    expect(guides).toContainEqual({ axis: "y", at: 122, from: 40, to: 300 });
  });

  it("does not snap beyond the threshold", () => {
    const session = beginSnap([frame("a", box(100, 0, 230, 50))], moving(box(0, 100, 80, 150)));
    const { delta, guides } = snapMoveDelta(session, { x: 107, y: 0 }, 6);
    expect(delta).toEqual({ x: 107, y: 0 });
    expect(guides).toEqual([]);
  });

  it("measures the threshold in screen pixels at every zoom", () => {
    const session = beginSnap([frame("a", box(100, 0, 230, 50))], moving(box(0, 100, 100, 150)));
    for (const zoom of [0.5, 1, 2, 4, 8]) {
      // The moving left edge sits at 0 and meets 100 at delta 100.
      const inside = 100 - (ALIGN_SNAP_SCREEN_PX - 0.5) / zoom;
      const outside = 100 - (ALIGN_SNAP_SCREEN_PX + 0.5) / zoom;
      expect(snapMoveDelta(session, { x: inside, y: 0 }, snapThresholdPt(zoom)).delta.x).toBeCloseTo(100, 5);
      expect(snapMoveDelta(session, { x: outside, y: 0 }, snapThresholdPt(zoom)).delta.x).toBeCloseTo(outside, 5);
    }
  });

  it("snaps x and y independently", () => {
    const session = beginSnap([frame("a", box(100, 100, 200, 200))], moving(box(0, 0, 80, 80)));
    const x = snapMoveDelta(session, { x: 102, y: 30 }, 6);
    expect(x.delta).toEqual({ x: 100, y: 30 });
    expect(x.guides.map((guide) => guide.axis)).toEqual(["x"]);
    const y = snapMoveDelta(session, { x: 30, y: 98 }, 6);
    expect(y.delta).toEqual({ x: 30, y: 100 });
    expect(y.guides.map((guide) => guide.axis)).toEqual(["y"]);
    const both = snapMoveDelta(session, { x: 102, y: 98 }, 6);
    expect(both.delta).toEqual({ x: 100, y: 100 });
    expect(both.guides.map((guide) => guide.axis).sort()).toEqual(["x", "y"]);
  });

  it("takes the nearest of several candidates", () => {
    const session = beginSnap(
      [frame("far", box(104, 300, 150, 340)), frame("near", box(101, 400, 150, 440))],
      moving(box(0, 0, 40, 40)),
    );
    expect(snapMoveDelta(session, { x: 100, y: 0 }, 6).delta.x).toBe(101);
  });

  it("leaves dragged objects out of the candidates", () => {
    const frames = [frame("self", box(100, 0, 200, 50)), frame("other", box(400, 0, 500, 50))];
    const session = beginSnap(frames, moving(box(0, 100, 80, 150), [], ["self"]));
    expect(snapMoveDelta(session, { x: 102, y: 0 }, 6).delta).toEqual({ x: 102, y: 0 });
    // A group frame stands for its parts: dragging any one part leaves the whole group out.
    const group: SnapFrame = { ids: ["group", "part-1", "part-2"], box: box(100, 0, 200, 50), ports: [] };
    const grouped = beginSnap([group], moving(box(0, 100, 80, 150), [], ["part-2"]));
    expect(snapMoveDelta(grouped, { x: 102, y: 0 }, 6).delta).toEqual({ x: 102, y: 0 });
  });

  it("does nothing when the page has nothing else on it", () => {
    const session = beginSnap([], moving(box(0, 0, 10, 10)));
    expect(snapMoveDelta(session, { x: 5, y: 5 }, 6)).toEqual({ delta: { x: 5, y: 5 }, guides: [] });
  });

  it("spans a guide across every aligned object", () => {
    const session = beginSnap(
      [frame("a", box(100, 0, 150, 40)), frame("b", box(100, 300, 150, 340))],
      moving(box(0, 100, 60, 160)),
    );
    const { guides } = snapMoveDelta(session, { x: 99, y: 0 }, 6);
    expect(guides).toEqual([{ axis: "x", at: 100, from: 0, to: 340 }]);
  });
});

describe("snapping under a clamp", () => {
  const session = () => beginSnap([frame("a", box(100, 0, 200, 50))], moving(box(0, 100, 80, 150)));

  it("draws guides at the clamped spot, so a clamp that moves the snap leaves none", () => {
    // Snapped to x = 100, then the page holds it back to 90: nothing is aligned there.
    const held = snapMoveDelta(session(), { x: 103, y: 0 }, 6, delta => ({ x: Math.min(delta.x, 90), y: delta.y }));
    expect(held.delta).toEqual({ x: 90, y: 0 });
    expect(held.guides).toEqual([]);
  });

  it("keeps the guide of an axis the clamp left alone", () => {
    const { delta, guides } = snapMoveDelta(session(), { x: 103, y: 0 }, 6, d => ({ x: d.x, y: Math.min(d.y, -5) }));
    expect(delta).toEqual({ x: 100, y: -5 });
    expect(guides).toEqual([expect.objectContaining({ axis: "x", at: 100 })]);
  });

  it("applies the clamp with Alt held or snapping off, and without a drag", () => {
    const clamp = (delta: { x: number; y: number }) => ({ x: Math.min(delta.x, 90), y: delta.y });
    const snapper = createMoveSnapper({ enabled: () => true, zoom: () => 1, onGuides: () => {} });
    snapper.begin(() => session());
    expect(snapper.adjust({ x: 103, y: 0 }, true, clamp)).toEqual({ x: 90, y: 0 });
    snapper.end();
    expect(snapper.adjust({ x: 103, y: 0 }, false, clamp)).toEqual({ x: 90, y: 0 });
  });
});

describe("move snapper", () => {
  const frames = [frame("a", box(100, 0, 200, 50))];
  const host = (enabled = true, zoom = 1) => {
    const seen: SnapGuide[][] = [];
    return { seen, enabled: () => enabled, zoom: () => zoom, onGuides: (guides: SnapGuide[]) => { seen.push(guides); } };
  };

  it("snaps while a drag is active and clears its guides at the end", () => {
    const h = host();
    const snapper = createMoveSnapper(h);
    snapper.begin(() => beginSnap(frames, moving(box(0, 100, 80, 150))));
    expect(snapper.adjust({ x: 103, y: 0 }, false)).toEqual({ x: 100, y: 0 });
    expect(h.seen.at(-1)).toHaveLength(1);
    snapper.end();
    expect(h.seen.at(-1)).toEqual([]);
    expect(snapper.adjust({ x: 103, y: 0 }, false)).toEqual({ x: 103, y: 0 });
  });

  it("passes the delta through while Alt is held", () => {
    const h = host();
    const snapper = createMoveSnapper(h);
    snapper.begin(() => beginSnap(frames, moving(box(0, 100, 80, 150))));
    expect(snapper.adjust({ x: 103, y: 0 }, true)).toEqual({ x: 103, y: 0 });
    expect(h.seen.flat()).toEqual([]);
  });

  it("does not snap when the setting is off", () => {
    const h = host(false);
    const snapper = createMoveSnapper(h);
    snapper.begin(() => beginSnap(frames, moving(box(0, 100, 80, 150))));
    expect(snapper.active).toBe(false);
    expect(snapper.adjust({ x: 103, y: 0 }, false)).toEqual({ x: 103, y: 0 });
  });

  it("reports guides only when they change", () => {
    const h = host();
    const onGuides = vi.spyOn(h, "onGuides");
    const snapper = createMoveSnapper(h);
    snapper.begin(() => beginSnap(frames, moving(box(0, 100, 80, 150))));
    snapper.adjust({ x: 101, y: 0 }, false);
    snapper.adjust({ x: 102, y: 0 }, false);
    snapper.adjust({ x: 103, y: 0 }, false);
    expect(onGuides).toHaveBeenCalledTimes(1);
  });
});


describe("palette placement snapping", () => {
  const item = { widthPt: 20, heightPt: 30, ports: [{ x: 3, y: 7 }] };
  it("snaps the centred candidate box and returns its insertion centre", () => {
    const session = beginPlacementSnap([frame("target", box(100, 100, 140, 140))], item);
    expect(session.moving.box).toEqual(box(-10, -15, 10, 15));
    const result = snapPlacement(session, { x: 111, y: 117 }, 1, true, false);
    expect(result.point).toEqual({ x: 110, y: 115 });
    expect(result.guides.length).toBeGreaterThan(0);
  });
  it("translates item ports from the item origin and snaps them to page ports", () => {
    const session = beginPlacementSnap([frame("target", box(300, 300, 340, 340), [{ x: 80, y: 70 }])], item);
    expect(session.moving.ports).toEqual([{ x: -7, y: -8 }]);
    expect(snapPlacement(session, { x: 88, y: 79 }, 2, true, false).point).toEqual({ x: 87, y: 78 });
  });
  it.each([[false, false], [true, true]])("leaves the centre unchanged when enabled=%s and Alt=%s", (enabled, alt) => {
    const session = beginPlacementSnap([frame("target", box(100, 100, 140, 140))], item);
    expect(snapPlacement(session, { x: 111, y: 117 }, 1, enabled, alt))
      .toEqual({ point: { x: 111, y: 117 }, guides: [] });
  });
});
