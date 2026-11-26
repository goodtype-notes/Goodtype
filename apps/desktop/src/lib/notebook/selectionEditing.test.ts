import { afterEach, describe, expect, it, vi } from "vitest";
import { createSelection, type SelectionEnvironment } from "./selectionEditing.svelte";
import { DEFAULT_SETTINGS } from "../settings";
import type { PageObject, Stroke } from "../model";
import { projectSnapshot } from "./snapshot";
import type { ClipboardBundle } from "./clipboard";
import type { ShapeView } from "./pageView";

const store = vi.hoisted(() => vi.fn());
vi.mock("../ipc/notebook", () => ({ storePastedImage: store }));
afterEach(() => { vi.resetAllMocks(); vi.unstubAllGlobals(); });

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>(done => { resolve = done; });
  return { promise, resolve };
}
function workspace() {
  const env = {
    activeInkLayerId: "ink", activeInkLayerPath: "ink/page.ink", activePageId: "page-001",
    activeSnapshot: null, changeStrokes: vi.fn(), closeToolCard: vi.fn(), images: [], message: String,
    navigation: {}, nextReadingOrder: () => 0, nextVisualZIndex: () => 0, pageInputLocked: false,
    queueCommit: vi.fn(), root: "notes", scheduleInkCommit: vi.fn(), selectedInk: [], selectedShapes: [],
    settings: DEFAULT_SETTINGS, shapes: [], status: "", strokes: [{ id: "original", points: [] } as unknown as Stroke],
    tauriAvailable: true, tool: "select", typstBlocks: [], zoom: 1,
    editContext() {
      const root = env.root, pageId = env.activePageId;
      return { root, pageId, current: () => root === env.root && pageId === env.activePageId };
    },
  } as unknown as { -readonly [K in keyof SelectionEnvironment]: SelectionEnvironment[K] };
  const selection = createSelection(env);
  selection.selectedStrokeIds = ["original"];
  return { env, selection };
}

describe("selection asynchronous edits", () => {
  it("waits for clipboard confirmation before cutting and keeps originals on failure", async () => {
    const { env, selection } = workspace();
    const confirmed = deferred<void>(), started = deferred<void>();
    const writeText = vi.fn(() => { started.resolve(); return confirmed.promise; });
    vi.stubGlobal("navigator", { clipboard: { writeText } });
    const cut = selection.copySelection(true); await started.promise;
    expect(env.strokes).toHaveLength(1); expect(env.queueCommit).not.toHaveBeenCalled();
    confirmed.resolve(); await cut;
    expect(env.strokes).toHaveLength(0); expect(env.queueCommit).toHaveBeenCalledOnce();

    const next = workspace();
    writeText.mockRejectedValueOnce(new Error("permission denied"));
    await next.selection.copySelection(true);
    expect(next.env.strokes).toHaveLength(1); expect(next.env.queueCommit).not.toHaveBeenCalled();
  });

  it.each(["page", "content"])("does not cut when %s changes during clipboard confirmation", async change => {
    const { env, selection } = workspace();
    const confirmed = deferred<void>(), started = deferred<void>();
    vi.stubGlobal("navigator", { clipboard: { writeText: () => { started.resolve(); return confirmed.promise; } } });
    const cut = selection.copySelection(true); await started.promise;
    if (change === "page") env.activePageId = "page-002";
    else env.strokes = env.strokes.map(stroke => ({ ...stroke, color: "#ff0000" }));
    confirmed.resolve(); await cut;
    expect(env.strokes).toHaveLength(1); expect(env.queueCommit).not.toHaveBeenCalled();
  });

  it.each(["root", "activePageId"] as const)("ignores a delayed image paste after %s changes", async field => {
    const { env, selection } = workspace();
    const saved = deferred<string>(), started = deferred<void>();
    store.mockImplementation(() => { started.resolve(); return saved.promise; });
    const bundle: ClipboardBundle = { token: "copy", pageId: "page-001", strokes: [], blocks: [], shapes: [],
      images: [{ bytes: new Uint8Array([1]), mime: "image/png", alt: "Image", x: 0, y: 0,
        widthPt: 100, heightPt: 80, scale: 1 }] };
    const paste = selection.pasteBundle(bundle); await started.promise;
    env[field] = "other";
    saved.resolve("assets/pasted.png"); await paste;
    expect(env.images).toEqual([]); expect(env.queueCommit).not.toHaveBeenCalled();
  });
});

function symbol() {
  const { env, selection } = workspace();
  selection.selectedStrokeIds = [];
  const fields = { y: 0, scale: 1, rotation: 0, zIndex: 0, readingOrder: 0, createdAt: "now", modifiedAt: "now" };
  const group = {
    ...fields, id: "symbol", type: "group", groupId: null, x: 0, childIds: ["left", "right"],
    ports: [{ x: 2, y: 0 }],
  } satisfies PageObject;
  const shapes = group.childIds.map((id, index) => ({
    ...fields, id, type: "shape" as const, x: index * 20, groupId: group.id,
    geometry: { kind: "line" as const, start: { x: 0, y: 0 }, end: { x: 10, y: 0 } },
    style: { strokeColor: "#000000", strokeWidthPt: 1, fillColor: null, opacity: 1 },
  }));
  env.shapes = shapes;
  env.activeSnapshot = { page: {
    objects: [group, ...shapes], readingOrder: [group.id], inkLayers: [], schemaVersion: 5,
  }, blocks: [], inkLayers: [], assets: [] } as unknown as typeof env.activeSnapshot;
  function projectedObjects() {
    return projectSnapshot({
      base: env.activeSnapshot, pageId: env.activePageId, revision: 0, strokes: [],
      manifest: { schemaVersion: 5, pages: [] } as never,
      geometry: { widthPt: 595, heightPt: 842 }, background: { kind: "plain", color: "#ffffff" },
      inkLayerId: env.activeInkLayerId, inkLayerPath: env.activeInkLayerPath,
      typst: env.typstBlocks.map(block => ({ ...block, ...block.transform, measuredWidthPt: 16, measuredHeightPt: 10 })),
      pageTypst: null, images: [], shapes: env.shapes, groupEdits: selection.groupEdits,
      mixedGroup: null, groupedStrokeIds: [], sharedStyle: null, now: "later",
    }).page.objects;
  }
  return { env, selection, group, projectedObjects };
}

describe("pack shape groups", () => {
  it("expands shape selection and a partial lasso hit to the whole symbol", () => {
    const { selection, group } = symbol();
    expect(selection.shapeGroupFor("right")?.id).toBe(group.id);
    selection.selectShape("right");
    expect(selection.selectedShapeId).toBeNull();
    expect(selection.selectedGroup?.id).toBe(group.id);
    selection.lassoSelection([{ x: 1, y: -2 }, { x: 4, y: -2 }, { x: 4, y: 2 }, { x: 1, y: 2 }], []);
    expect(selection.lassoObjectIds).toEqual(group.childIds);
    expect(selection.mixedSelectionBounds?.right).toBeGreaterThanOrEqual(30);
  });

  it("commits one transform for children and the group port frame and restores a cancelled preview", () => {
    const { env, selection, projectedObjects } = symbol();
    selection.selectShape("left");
    selection.beginMixedTransform();
    selection.previewMixedTransform({ x: 5, y: 10 }, 2, { x: 0, y: 0 }, 90);
    expect(selection.previewShapes[1].y).toBeCloseTo(50);
    expect(projectedObjects()).toContainEqual(expect.objectContaining({ id: "right", y: 0 }));
    expect(env.queueCommit).not.toHaveBeenCalled();
    selection.endMixedTransform(true);
    expect(env.shapes[1].y).toBe(0);
    selection.beginMixedTransform();
    selection.previewMixedTransform({ x: 5, y: 10 }, 2, { x: 0, y: 0 }, 90);
    selection.endMixedTransform(false);
    expect(projectedObjects()).toEqual(expect.arrayContaining([
      expect.objectContaining({ id: "symbol", x: 5, y: 10, scale: 2, rotation: 90, ports: [{ x: 2, y: 0 }] }),
      expect.objectContaining({ id: "right", y: 50, scale: 2, rotation: 90 }),
    ]));
    expect(env.queueCommit).toHaveBeenCalledOnce();
  });

  it("selects one editable child inside the group and recomputes bounds after exit", () => {
    const { env, selection, projectedObjects } = symbol();
    selection.selectShape("left");
    expect(selection.enterGroup("left")).toBe(true);
    selection.selectShape("right");
    expect(selection.selectedShapeId).toBe("right");
    expect(selection.mixedSelectionBounds).toBeNull();
    env.shapes = env.shapes.map(shape => shape.id === "right" ? { ...shape, x: 50 } : shape);
    expect(selection.exitGroup(true)).toBe(true);
    expect(selection.mixedSelectionBounds?.right).toBeGreaterThanOrEqual(60);
    expect(projectedObjects()).toContainEqual(expect.objectContaining({ id: "right", groupId: "symbol" }));
    expect(projectedObjects()).toContainEqual(expect.objectContaining({ id: "symbol", x: 0, childIds: ["left", "right"] }));
  });

  it("transforms a Typst label with the shapes and can select it as an individual part", () => {
    const { env, selection, projectedObjects } = symbol();
    const label = {
      ...env.activeSnapshot!.page.objects[2], type: "typst" as const, sourcePath: "blocks/right.typ",
      layoutWidthPt: 16, measuredWidthPt: 16, measuredHeightPt: 10,
    };
    env.activeSnapshot!.page.objects[2] = label;
    env.shapes = env.shapes.filter(shape => shape.id !== label.id);
    const transform = { x: label.x, y: label.y, layoutWidthPt: label.layoutWidthPt, scale: 1, rotation: 0 };
    env.typstBlocks = [{ id: label.id, path: label.sourcePath, source: "Label", transform, zIndex: 0, readingOrder: 0, result: null }];
    selection.selectShape("left");
    selection.beginMixedTransform();
    selection.previewMixedTransform({ x: 0, y: 0 }, 1, { x: 0, y: 0 }, 90);
    selection.endMixedTransform(false);
    expect(projectedObjects()).toContainEqual(expect.objectContaining({ id: "right", type: "typst", y: 20, rotation: 90 }));
    expect(selection.enterGroup("right")).toBe(true);
    expect(selection.selectedTypstId).toBe("right");
    expect(selection.exitGroup()).toBe(true);
    expect(selection.selectedTypstId).toBeNull();
    expect(selection.selectedGroup).toBeUndefined();
  });

  it.each(["ungroup", "delete"])("%ss the selected group in one commit", action => {
    const { env, selection, projectedObjects } = symbol();
    selection.selectShape("left");
    if (action === "ungroup") selection.ungroupShapes();
    else expect(selection.deleteSelection()).toBe(true);
    expect(projectedObjects().map(object => [object.id, object.groupId])).toEqual(
      action === "ungroup" ? [["left", null], ["right", null]] : [],
    );
    expect(env.queueCommit).toHaveBeenCalledOnce();
  });
});

function rectangle(id: string, x: number, y: number, widthPt: number, heightPt: number, rotation = 0): ShapeView {
  return {
    id, x, y, rotation, scale: 1, zIndex: 0, readingOrder: 0,
    geometry: { kind: "rectangle" as const, widthPt, heightPt, cornerRadiusPt: 0 },
    style: { strokeColor: "#000000", strokeWidthPt: 1, fillColor: null, opacity: 1 },
  };
}

function board(...shapes: ShapeView[]) {
  const { env, selection } = workspace();
  selection.selectedStrokeIds = [];
  env.shapes = shapes;
  return { env, selection };
}

describe("snapping a dragged selection to other objects", () => {
  const scene = () => board(rectangle("a", 100, 100, 50, 30), rectangle("b", 300, 200, 80, 40));

  it("snaps edges while dragging and commits once, at the end", () => {
    const { env, selection } = scene();
    selection.lassoObjectIds = ["b"];
    selection.beginMixedTransform();
    selection.previewMixedTransform({ x: -197, y: -96 }, 1, { x: 0, y: 0 }, 0, { altKey: false });
    expect(selection.previewShapes[1]).toMatchObject({ x: 100, y: 100 });
    expect(selection.snapGuides.map(guide => guide.axis).sort()).toEqual(["x", "y"]);
    selection.previewMixedTransform({ x: -198, y: -97 }, 1, { x: 0, y: 0 }, 0, { altKey: false });
    expect(env.queueCommit).not.toHaveBeenCalled();
    selection.endMixedTransform(false);
    expect(env.queueCommit).toHaveBeenCalledOnce();
    expect(env.shapes[1]).toMatchObject({ x: 100, y: 100 });
    expect(selection.snapGuides).toEqual([]);
  });

  it("leaves a drag alone while Alt is held, with the setting off, or without a pointer", () => {
    for (const [pointer, snap] of [[{ altKey: true }, true], [{ altKey: false }, false], [undefined, true]] as const) {
      const { env, selection } = scene();
      env.settings = { ...DEFAULT_SETTINGS, snapToObjects: snap };
      selection.lassoObjectIds = ["b"];
      selection.beginMixedTransform();
      selection.previewMixedTransform({ x: -197, y: -96 }, 1, { x: 0, y: 0 }, 0, pointer);
      expect(selection.previewShapes[1]).toMatchObject({ x: 103, y: 104 });
      expect(selection.snapGuides).toEqual([]);
      selection.endMixedTransform(true);
    }
  });

  it("never snaps a scale or a rotation, and never to the dragged objects themselves", () => {
    const { selection } = scene();
    selection.lassoObjectIds = ["b"];
    selection.beginMixedTransform();
    selection.previewMixedTransform({ x: 0, y: 0 }, 1, { x: 300, y: 200 }, 90, { altKey: false });
    expect(selection.snapGuides).toEqual([]);
    selection.endMixedTransform(true);
    // Dragging "a" near its own old edge does nothing: only "b", far away, is a candidate.
    selection.lassoObjectIds = ["a"];
    selection.beginMixedTransform();
    selection.previewMixedTransform({ x: 3, y: 3 }, 1, { x: 0, y: 0 }, 0, { altKey: false });
    expect(selection.previewShapes[0]).toMatchObject({ x: 103, y: 103 });
    selection.endMixedTransform(true);
  });

  it("snaps a dragged part's port to another part's port", () => {
    const { selection } = board(
      { ...rectangle("a", 100, 100, 40, 200), ports: [{ x: 0, y: 25 }] },
      { ...rectangle("b", 300, 300, 40, 100), ports: [{ x: 0, y: 10 }] },
    );
    selection.lassoObjectIds = ["b"];
    selection.beginMixedTransform();
    // Only the ports can meet: b's own edges and middle are all further than six points away.
    selection.previewMixedTransform({ x: 0, y: -188 }, 1, { x: 0, y: 0 }, 0, { altKey: false });
    expect(selection.previewShapes[1].y).toBe(115);
    expect(selection.snapGuides).toContainEqual(expect.objectContaining({ axis: "y", at: 125 }));
    selection.endMixedTransform(true);
  });

  describe("one child of a group", () => {
    /** Group "g" holds "c1" and "c2"; "t" is a loose rectangle. The group has its own port at (200, 0). */
    function grouped() {
      const fields = { y: 0, scale: 1, rotation: 0, zIndex: 0, readingOrder: 0, createdAt: "now", modifiedAt: "now" };
      const group = { ...fields, id: "g", type: "group", groupId: null, x: 0, childIds: ["c1", "c2"],
        ports: [{ x: 200, y: 0 }] } satisfies PageObject;
      const children = [rectangle("c1", 100, 100, 50, 30), rectangle("c2", 300, 100, 50, 30)];
      const { env, selection } = board(...children, rectangle("t", 190, 500, 40, 40));
      env.activeSnapshot = { page: {
        objects: [group, ...children.map(child => ({ ...fields, ...child, type: "shape", groupId: "g" })),
          { ...fields, ...rectangle("t", 190, 500, 40, 40), type: "shape", groupId: null }],
        readingOrder: ["g", "t"], inkLayers: [], schemaVersion: 5,
      }, blocks: [], inkLayers: [], assets: [] } as unknown as typeof env.activeSnapshot;
      return { env, selection };
    }

    it("snaps to its siblings, which stay targets", () => {
      const { selection } = grouped();
      selection.lassoObjectIds = ["c2"];
      selection.beginMixedTransform();
      // c2's left edge (300 - 148 = 152) is within six points of c1's right edge (150).
      selection.previewMixedTransform({ x: -148, y: 0 }, 1, { x: 0, y: 0 }, 0, { altKey: false });
      expect(selection.previewShapes[1].x).toBe(150);
      expect(selection.snapGuides).toContainEqual(expect.objectContaining({ axis: "x", at: 150 }));
      selection.endMixedTransform(true);
    });

    it("moves with its own ports, not the group's", () => {
      const { selection } = grouped();
      selection.lassoObjectIds = ["c2"];
      selection.beginMixedTransform();
      // The group's port at x = 200 would reach t's left edge (190) after -7; c2 itself reaches nothing.
      selection.previewMixedTransform({ x: -7, y: 0 }, 1, { x: 0, y: 0 }, 0, { altKey: false });
      expect(selection.previewShapes[1].x).toBe(293);
      expect(selection.snapGuides.filter(guide => guide.axis === "x")).toEqual([]);
      selection.endMixedTransform(true);
    });

    it("still moves a whole group as one frame", () => {
      const { selection } = grouped();
      selection.lassoObjectIds = ["c1", "c2"];
      selection.beginMixedTransform();
      // The group's port (x = 200) is pulled onto t's left edge (190): a delta of -10.
      selection.previewMixedTransform({ x: -7, y: 0 }, 1, { x: 0, y: 0 }, 0, { altKey: false });
      expect(selection.previewShapes[0].x).toBe(90);
      selection.endMixedTransform(true);
    });
  });

  it("snaps a rotated object by its visible box", () => {
    // 30 x 40 turned a quarter: it covers x from 200 - 40 = 160 to 200 and y from 100 to 130.
    const { selection } = board(rectangle("a", 100, 100, 50, 30), rectangle("turned", 200, 300, 30, 40, 90));
    selection.lassoObjectIds = ["turned"];
    selection.beginMixedTransform();
    // Its box left (160) lands on a's right edge (150) after -10 -> within 6 of -8.
    selection.previewMixedTransform({ x: -7, y: 0 }, 1, { x: 0, y: 0 }, 0, { altKey: false });
    expect(selection.previewShapes[1].x).toBeCloseTo(190);
    selection.endMixedTransform(true);
  });
});

describe("align and distribute", () => {
  it("aligns the selection in one commit and keeps the rest of the page", () => {
    const { env, selection } = board(rectangle("a", 10, 40, 40, 20), rectangle("b", 100, 10, 20, 80), rectangle("c", 60, 70, 20, 60));
    selection.lassoObjectIds = ["a", "b", "c"];
    selection.alignSelection("left");
    expect(env.shapes.map(shape => shape.x)).toEqual([10, 10, 10]);
    expect(env.shapes.map(shape => shape.y)).toEqual([40, 10, 70]);
    expect(env.queueCommit).toHaveBeenCalledOnce();
  });

  it("aligns a rotated shape by its visible box", () => {
    // Turned a quarter, a 30 x 40 box at (200, 300) covers x 160..200.
    const { env, selection } = board(rectangle("a", 100, 100, 50, 30), rectangle("turned", 200, 300, 30, 40, 90));
    selection.lassoObjectIds = ["a", "turned"];
    selection.alignSelection("left");
    expect(env.shapes[0].x).toBe(100);
    expect(env.shapes[1].x).toBeCloseTo(140);
    expect(env.queueCommit).toHaveBeenCalledOnce();
  });

  it("distributes three or more with equal gaps in one commit", () => {
    const { env, selection } = board(rectangle("a", 0, 0, 10, 10), rectangle("b", 12, 0, 30, 10), rectangle("c", 190, 0, 10, 10));
    selection.lassoObjectIds = ["a", "b", "c"];
    selection.distributeSelection("horizontal");
    expect(env.shapes[1].x).toBeCloseTo(85);
    expect(env.queueCommit).toHaveBeenCalledOnce();
  });

  it("offers neither with fewer objects than the action needs", () => {
    const { env, selection } = board(rectangle("a", 0, 0, 10, 10), rectangle("b", 50, 20, 10, 10));
    selection.lassoObjectIds = ["a"];
    expect(selection.alignCount).toBe(1);
    selection.alignSelection("left");
    selection.lassoObjectIds = ["a", "b"];
    expect(selection.alignCount).toBe(2);
    selection.distributeSelection("horizontal");
    expect(env.queueCommit).not.toHaveBeenCalled();
  });

  it("moves selected ink with the objects", () => {
    const { env, selection } = board(rectangle("a", 100, 0, 10, 10));
    env.strokes = [{ id: "ink", points: [{ x: 0, y: 50, pressure: 1, time: 0 }, { x: 20, y: 60, pressure: 1, time: 1 }],
      transform: { translateX: 0, translateY: 0, scaleX: 1, scaleY: 1, rotation: 0 } } as unknown as Stroke];
    selection.lassoObjectIds = ["a"];
    selection.selectedStrokeIds = ["ink"];
    selection.alignSelection("left");
    expect(env.shapes[0].x).toBe(0);
    expect((env.strokes[0] as unknown as { transform: { translateX: number } }).transform.translateX).toBe(0);
    expect(env.queueCommit).toHaveBeenCalledOnce();
  });
});
