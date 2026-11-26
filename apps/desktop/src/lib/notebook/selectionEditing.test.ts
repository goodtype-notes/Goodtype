import { afterEach, describe, expect, it, vi } from "vitest";
import { createSelection, type SelectionEnvironment } from "./selectionEditing.svelte";
import { DEFAULT_SETTINGS } from "../settings";
import type { PageObject, Stroke } from "../model";
import { projectSnapshot } from "./snapshot";
import type { ClipboardBundle } from "./clipboard";

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
