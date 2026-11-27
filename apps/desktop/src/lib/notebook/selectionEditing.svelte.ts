import { materialItems } from "../helper/crop";
import { invoke } from "@tauri-apps/api/core";
import { previewChannel, type Mark } from "./wasmPreview";
import type { createNavigation } from "./viewportNavigation.svelte";

import { translate, notice, type UiText } from "../i18n";

import { storePastedImage } from "../ipc/notebook";

import { finishInkPreview, inkPreviewBounds, type InkPreview } from "../ink/preview";
import type { NotebookSnapshot } from "../ipc/types";

import { pageToScreen, rotatedVector, type Point } from "../geometry/coordinates";
import { alignDeltas, distributeDeltas, unionBounds, type AlignMode, type AlignUnit, type DistributeAxis } from "../geometry/align";
import { beginSnap, createMoveSnapper, type ObjectMoveSnap, type SnapFrame, type SnapGuide, type SnapSession } from "../geometry/alignSnap";
import { pagePorts } from "./addonPorts";
import type { Bounds } from "../ink/selection";
import { placeFloatingToolbar, type ViewRect } from "../geometry/placement";

import { type PageObject, type Stroke } from "../model";

import { pointerRole, type InkTool, type PointerMapping } from "../ink/pipeline";
import { keepsSelection, moveSelected, restyleStrokes, selectionBounds, toolAfterSelection, type StylePatch } from "../ink/selection";

import { inkLayersWithStrokes, type TypstBlockState, type ImageView, type ShapeView, type TypstTransform } from "./pageView";

import { DEFAULT_LASSO_TARGETS, boxOutline, shapeLassoItem, selectLassoObjects, combinedBounds, transformSelectionPoint, type LassoItem } from "./lasso";
import { nativeContextMenuAllowed } from "./contextMenu";

import { type ManagedMixedGroup } from "./snapshot";

import { moveReadingObject, moveVisualItems, type VisualMove } from "./objectOrder";

import { clipboardMarker, completeSelectedGroups, isEmptyBundle, pasteOffset, pastedAddonData, pastedShapes, pastedStrokes, shiftTo, type ClipboardBundle } from "./clipboard";

import { type AppSettings } from "../settings";

type ShapeGroup = Extract<PageObject, { type: "group" }>;
type GroupTransform = Pick<ShapeGroup, "x" | "y" | "scale" | "rotation">;

export type SelectionEnvironment = {
  activeInkLayerId: string;
  activeInkLayerPath: string;
  readonly regionBounds?: import("../ink/selection").Bounds | null;
  readonly activePageId: string;
  activeSnapshot: NotebookSnapshot | null;
  readonly changeStrokes: (next: Stroke[], label: UiText, replaced?: ReadonlyMap<string, string[]>) => void;
  readonly closeToolCard: () => void;
  readonly editContext: () => { root: string; pageId: string; current: () => boolean };
  images: ImageView[];
  addCopiedGroups?: (groups: NonNullable<ClipboardBundle["groups"]>) => void;
  readonly message: (error: unknown) => string;
  readonly navigation: Pick<ReturnType<typeof createNavigation>, "fingerDraws" | "pagePointAt">;
  readonly nextReadingOrder: () => number;
  readonly nextVisualZIndex: () => number;
  readonly pageFrame: HTMLElement | undefined;
  readonly pageInputLocked: boolean;
  readonly pointerMapping: PointerMapping;
  readonly queueCommit: (label: UiText) => void;
  readonly root: string;
  readonly scheduleInkCommit: (label: UiText) => void;
  readonly selectedInk: Stroke[];
  readonly selectedShapes: ShapeView[];
  readonly settings: AppSettings;
  shapes: ShapeView[];
  status: UiText;
  strokes: Stroke[];
  readonly tauriAvailable: boolean;
  tool: InkTool;
  typstBlocks: TypstBlockState[];
  workspace: HTMLElement | undefined;
  readonly zoom: number;
  readonly zooming?: boolean;
};

/** Instance-owned selection state and operations. Callers supply live values at the boundary. */
export function createSelection(env: SelectionEnvironment) {

  let selectedStrokeIds = $state<string[]>([]);

  let groupedStrokeIds = $state<string[]>([]);

  let selectedImageId = $state<string | null>(null);

  let selectedShapeId = $state<string | null>(null);

  let editingShapeId = $state<string | null>(null);

  let mixedGroup = $state<ManagedMixedGroup | null>(null);

  let enteredGroupId = $state<string | null>(null);
  let groupEdits = $state.raw<ReadonlyMap<string, GroupTransform | null>>(new Map());

  const groupCandidates = $derived.by(() => {
    const objects = new Map(env.activeSnapshot?.page.objects.map(object => [object.id, object]));
    return [...objects.values()].flatMap(object => {
      if (object.type !== "group" || object.id === mixedGroup?.groupId) return [];
      const edit = groupEdits.get(object.id);
      if (edit === null || !object.childIds.every(id => {
        const child = objects.get(id);
        return child?.groupId === object.id && (child.type === "shape" || child.type === "typst");
      })) return [];
      return [{ ...object, ...edit }];
    });
  });
  const shapeGroups = $derived.by(() => {
    const live = new Set([...env.shapes, ...env.typstBlocks].map(object => object.id));
    return groupCandidates.flatMap(group => {
      const childIds = group.childIds.filter(id => live.has(id));
      return childIds.length ? [{ ...group, childIds }] : [];
    });
  });
  const selectedGroup = $derived(shapeGroups.find(group =>
    group.childIds.length === lassoObjectIds.length && !selectedStrokeIds.length &&
    group.childIds.every(id => lassoObjectIds.includes(id)),
  ));
  const enteredGroup = $derived(shapeGroups.find(group => group.id === enteredGroupId));

  function shapeGroupFor(id: string | null) {
    return shapeGroups.find(group => id !== null && group.childIds.includes(id));
  }

  function selectGroup(group: ShapeGroup) {
    enteredGroupId = null;
    selectedTypstId = selectedImageId = selectedShapeId = editingShapeId = null;
    lassoObjectIds = [...group.childIds];
    updateInkSelection([]);
  }

  function selectShape(id: string) {
    const group = shapeGroupFor(id);
    if (group && group.id !== enteredGroupId) {
      selectGroup(group);
      return;
    }
    lassoObjectIds = [];
    if (selectedShapeId !== id) editingShapeId = null;
    selectedShapeId = id;
    selectedTypstId = selectedImageId = null;
    updateInkSelection([]);
  }

  function enterGroup(id?: string) {
    const group = selectedGroup;
    if (!group) return false;
    enteredGroupId = group.id;
    lassoObjectIds = [];
    if (id && group.childIds.includes(id)) {
      if (env.typstBlocks.some(block => block.id === id)) selectedTypstId = id;
      else selectedShapeId = id;
    }
    updateInkSelection([]);
    env.status = notice("workspace.editingShapeGroup");
    return true;
  }

  function exitGroup(reselect = false) {
    if (!enteredGroupId) return false;
    if (reselect && enteredGroup) selectGroup(enteredGroup);
    else enteredGroupId = null;
    selectedShapeId = editingShapeId = selectedTypstId = null;
    updateInkSelection([]);
    return true;
  }

  function ungroupShapes() {
    if (!selectedGroup) return;
    const { id, childIds } = selectedGroup;
    groupEdits = new Map(groupEdits).set(id, null);
    env.shapes = env.shapes.map(shape => childIds.includes(shape.id) ? { ...shape, groupId: null } : shape);
    env.typstBlocks = env.typstBlocks.map(block => childIds.includes(block.id) ? { ...block, groupId: null } : block);
    env.queueCommit(notice("workspace.ungroupedShapes"));
  }

  let selectedTypstId = $state<string | null>(null);

  let lassoObjectIds = $state<string[]>([]);



  function lassoItems(): LassoItem[] {
    const measured = new Map((env.activeSnapshot?.page.objects ?? []).flatMap(object =>
      object.type === "typst" ? [[object.id, object] as const] : []));
    return [
      ...materialItems(env.activeSnapshot?.page.objects ?? []),
      ...previewShapes.map(shapeLassoItem),
      ...previewImages.map(image => ({ id: image.id, kind: "images" as const, closed: true,
        outline: boxOutline(image.x, image.y, image.widthPt * image.scale, image.heightPt * image.scale, image.rotation) })),
      ...previewBlocks.map(block => ({ id: block.id, kind: "text" as const, closed: true,
        outline: boxOutline(block.transform.x, block.transform.y,
          (block.result?.widthPt ?? measured.get(block.id)?.measuredWidthPt ?? block.transform.layoutWidthPt) * block.transform.scale,
          (block.result?.heightPt ?? measured.get(block.id)?.measuredHeightPt ?? 64) * block.transform.scale, block.transform.rotation) })),
    ];
  }

  const mixedSelectionBounds = $derived.by(() => {
    if (!lassoObjectIds.length) return null;
    return combinedBounds(lassoItems().filter(item => lassoObjectIds.includes(item.id)),
      inkPreviewBounds(env.strokes, selectedStrokeIds, mixedStrokePreview));
  });

  function lassoSelection(polygon: Point[], inkIds: string[]) {
    exitGroup();
    selectedTypstId = null; selectedImageId = null; selectedShapeId = null; editingShapeId = null;
    lassoObjectIds = selectLassoObjects(lassoItems(), polygon, env.settings.lassoTargets ?? DEFAULT_LASSO_TARGETS);
    const hits = new Set(lassoObjectIds);
    for (const group of shapeGroups) {
      if (group.childIds.some(id => hits.has(id))) group.childIds.forEach(id => hits.add(id));
    }
    lassoObjectIds = [...hits];
    updateInkSelection(inkIds);
    env.status = notice("workspace.selectedItems", { "value1": inkIds.length + lassoObjectIds.length });
  }

  /** Page bounds, rotation included, of every shape, image and text block on the page, by id. */
  function objectBoxes(): Map<string, Bounds> {
    const live = new Set([...env.shapes, ...env.images, ...env.typstBlocks].map(object => object.id));
    return new Map(lassoItems().flatMap(item => {
      const box = live.has(item.id) ? combinedBounds([item], null) : null;
      return box ? [[item.id, box] as const] : [];
    }));
  }

  /**
   * Each loose object, and each whole group as one, with its box and the ports it exposes. A group
   * only some of whose children are being moved has no frame of its own: its children stand
   * alone, so the dragged child moves with its own box and ports and its siblings stay targets.
   */
  function snapFrames(boxes: ReadonlyMap<string, Bounds>, moving: ReadonlySet<string>): SnapFrame[] {
    const grouped = new Set<string>();
    const frames: SnapFrame[] = [];
    for (const group of shapeGroups) {
      const dragged = group.childIds.filter(id => moving.has(id)).length;
      if (dragged > 0 && dragged < group.childIds.length) continue;
      const parts = group.childIds.flatMap(id => boxes.get(id) ?? []);
      if (!parts.length) continue;
      group.childIds.forEach(id => grouped.add(id));
      frames.push({ ids: [group.id, ...group.childIds], box: unionBounds(parts), ports: pagePorts(group) });
    }
    const ports = new Map(env.shapes.map(shape => [shape.id, pagePorts({ type: "shape", ...shape })]));
    for (const [id, box] of boxes) if (!grouped.has(id)) frames.push({ ids: [id], box, ports: ports.get(id) ?? [] });
    return frames;
  }

  /** The index for one drag of these objects and ink strokes, built once at its first sample. */
  function dragSession(ids: readonly string[], strokeIds: readonly string[]): SnapSession | null {
    const boxes = objectBoxes();
    const ink = strokeIds.length ? selectionBounds(env.strokes, [...strokeIds]) : null;
    const parts = [...ids.flatMap(id => boxes.get(id) ?? []), ...(ink ? [ink] : [])];
    if (!parts.length) return null;
    const moving = new Set(ids);
    const frames = snapFrames(boxes, moving);
    return beginSnap(frames, { ids: moving, box: unionBounds(parts),
      ports: frames.filter(frame => frame.ids.some(id => moving.has(id))).flatMap(frame => frame.ports) });
  }

  /** The alignment guides of the drag in progress: ephemeral, in page points, never saved. */
  let snapGuides = $state.raw<SnapGuide[]>([]);
  const snapper = createMoveSnapper({
    enabled: () => env.settings.snapToObjects !== false,
    zoom: () => env.zoom,
    onGuides: guides => { snapGuides = guides; },
  });
  /** What a single object's own drag (shape, image, text block, ink) calls to snap its moves. */
  const moveSnap: ObjectMoveSnap = {
    begin: (ids, strokeIds = []) => snapper.begin(() => dragSession(ids, strokeIds)),
    adjust: snapper.adjust,
    end: snapper.end,
  };
  let snapPending = false;

  let mixedGesture: { current: () => boolean; objects: Set<string>; strokes: string[] } | null = null;
  let wasmMarks = $state.raw<Mark[]>([]);
  let wasmChannel: ReturnType<typeof previewChannel> | null = null;
  let wasmGesture = 0;
  let wasmCurrent: (() => boolean) | null = null;
  let wasmPending: ShapeView[] = [];
  $effect(() => {
    void [env.root, env.activePageId, env.activeSnapshot?.page.revision];
    return () => endWasmPreview();
  });
  let transformPreview = $state.raw<{
    objects: ReadonlySet<string>; ink: InkPreview; rotation: number;
  } | null>(null);
  const mixedStrokePreview = $derived(transformPreview?.ink ?? null);

  function transformed<T extends Point & { scale: number; rotation?: number }>(
    object: T, { ink: { delta, scale, anchor }, rotation }: NonNullable<typeof transformPreview>,
  ): T {
    const offset = rotatedVector({ x: object.x - anchor.x, y: object.y - anchor.y }, rotation);
    const point = transformSelectionPoint({ x: anchor.x + offset.x, y: anchor.y + offset.y }, delta, scale, anchor);
    return { ...object, ...point, scale: object.scale * scale, rotation: (object.rotation ?? 0) + rotation };
  }

  const previewGroup = $derived(selectedGroup && transformPreview
    ? transformed(selectedGroup, transformPreview) : selectedGroup);

  const previewShapes = $derived(env.shapes.map(shape =>
    transformPreview?.objects.has(shape.id) ? transformed(shape, transformPreview) : shape,
  ));
  const previewImages = $derived(env.images.map(image =>
    transformPreview?.objects.has(image.id) ? transformed(image, transformPreview) : image,
  ));
  const previewBlocks = $derived(env.typstBlocks.map(block =>
    transformPreview?.objects.has(block.id) ? { ...block, transform: transformed(block.transform, transformPreview) } : block,
  ));

  function beginMixedTransform() {
    const { current } = env.editContext();
    mixedGesture = { current, objects: new Set(lassoObjectIds), strokes: [...selectedStrokeIds] };
    transformPreview = null;
    snapPending = true;
    startWasmPreview(current);
  }

  function startWasmPreview(current: () => boolean) {
    wasmChannel?.stop(); wasmChannel = null; wasmMarks = [];
    wasmCurrent = current; wasmPending = [];
    const gesture = ++wasmGesture;
    const digest = import.meta.env.VITE_WASM_SPIKE_SHA256;
    if (import.meta.env.DEV && typeof digest === "string" && /^[a-f0-9]{64}$/.test(digest)) {
      void Promise.all([invoke<number[]>("addon_wasm_bytes", { addonId: "connectors", digest }), import("./wasmPreviewWorker")]).then(([bytes, { createPreviewWorker }]) => {
        if (wasmGesture !== gesture || !wasmCurrent?.()) return;
        const worker = createPreviewWorker();
        wasmChannel = previewChannel(worker, marks => { if (wasmCurrent?.()) wasmMarks = marks; }, () => { wasmMarks = []; });
        const buffer = new Uint8Array(bytes).buffer;
        worker.postMessage({ kind: "init", bytes: buffer, digest }, [buffer]);
        submitWasmPreview(wasmPending);
      }).catch(() => { wasmMarks = []; });
    }
  }

  /**
   * `pointer` marks a pointer drag, the only kind that snaps: a keyboard nudge or a +/- scale
   * lands exactly where it was asked to. Only a plain move (no scale, no rotation) snaps.
   */
  function previewMixedTransform(delta: Point, scale: number, anchor: Point, rotation = 0, pointer?: { altKey: boolean }) {
    if (!mixedGesture?.current()) return;
    if (pointer && scale === 1 && rotation === 0) {
      if (snapPending) {
        snapPending = false;
        const { objects, strokes } = mixedGesture;
        snapper.begin(() => dragSession([...objects], strokes));
      }
      delta = snapper.adjust(delta, pointer.altKey);
    }
    transformPreview = { objects: mixedGesture.objects, ink: { ids: mixedGesture.strokes, delta, scale, anchor }, rotation };
    submitWasmPreview(previewShapes.filter(shape => mixedGesture!.objects.has(shape.id)));
  }

  function submitWasmPreview(shapes: ShapeView[]) {
    wasmPending = shapes;
    const page = env.activeSnapshot?.page;
    if (wasmChannel && page && wasmCurrent?.()) {
      const changed = shapes.flatMap(shape => {
        const bounds = combinedBounds([shapeLassoItem(shape)], null);
        if (!bounds) return [];
        const canonical = page.objects.find(object => object.id === shape.id);
        if (!canonical || canonical.type !== "shape") return [];
        return [{ id: shape.id, bounds: { x: bounds.left, y: bounds.top, width: bounds.right - bounds.left, height: bounds.bottom - bounds.top },
          ports: pagePorts({ ...canonical, ...shape }) }];
      });
      if (previewGroup && mixedGesture && previewGroup.childIds.every(id => mixedGesture!.objects.has(id))) {
        const bounds = combinedBounds(shapes.map(shapeLassoItem), null);
        if (bounds) changed.push({ id: previewGroup.id, bounds: { x: bounds.left, y: bounds.top, width: bounds.right - bounds.left, height: bounds.bottom - bounds.top }, ports: pagePorts(previewGroup) });
      }
      const ids = new Set(changed.map(object => object.id));
      const own = page.objects.filter(object => object.addonData?.connectors?.refs.some(id => ids.has(id)));
      if (own.length) wasmChannel.submit({ gestureId: String(wasmGesture), revision: page.revision, kind: "changed", ports: [], settings: {},
        local: { page: { widthPt: page.geometry.widthPt, heightPt: page.geometry.heightPt }, changed, own, deleted: [] } });
    }
  }

  function endMixedTransform(cancelled: boolean, debounce = false) {
    endWasmPreview();
    snapPending = false;
    snapper.end();
    const gesture = mixedGesture;
    const preview = transformPreview;
    const nextShapes = previewShapes, nextImages = previewImages, nextBlocks = previewBlocks;
    mixedGesture = null;
    transformPreview = null;
    if (cancelled || !gesture?.current() || !preview) return;
    const { delta, scale } = preview.ink;
    if (delta.x === 0 && delta.y === 0 && scale === 1 && preview.rotation === 0) return;

    for (const group of shapeGroups) {
      if (!group.childIds.every(id => gesture.objects.has(id))) continue;
      const { x, y, scale, rotation } = transformed(group, preview);
      groupEdits = new Map(groupEdits).set(group.id, { x, y, scale, rotation });
    }
    env.shapes = nextShapes;
    env.images = nextImages;
    env.typstBlocks = nextBlocks;
    if (preview.ink.ids.length) env.strokes = finishInkPreview(env.strokes, preview.ink);
    if (debounce) env.scheduleInkCommit(notice("workspace.movedSelection"));
    else env.queueCommit(notice("workspace.transformedSelection"));
  }

  function endWasmPreview() {
    ++wasmGesture; wasmChannel?.stop(); wasmChannel = null; wasmMarks = []; wasmCurrent = null; wasmPending = [];
  }

  function previewShapeGhost(phase: "start" | "move" | "end", shape: ShapeView) {
    if (phase === "start") startWasmPreview(env.editContext().current);
    if (phase === "end") endWasmPreview();
    else submitWasmPreview([shape]);
  }

  let directObjectInput = $state(false);

  /** True while the selection tool is only active because the lasso handed over to it. */
  /** The tool a selection hand-over gives back once the selection empties. */
  let selectionReturnTo = $state<InkTool | null>(null);

  let selectionToolbarElement = $state<HTMLElement>();

  let selectionToolbarFrame: number | undefined;

  let selectionToolbarPosition = $state({ left: 0, top: 0, ready: false });

  function groupSelectedInk() {
    if (selectedStrokeIds.length === 0) return;
    const layers = inkLayersWithStrokes(env.activeSnapshot?.inkLayers ?? [], env.strokes, env.activeInkLayerId)
      .filter(layer => layer.strokes.some(stroke => selectedStrokeIds.includes(stroke.id)));
    if (layers.length > 1) {
      env.status = notice("workspace.selectInkFromOneLayerBeforeCreating");
      return;
    }
    const drawingLayer = env.activeSnapshot?.page.inkLayers.find(layer => layer.id === layers[0]?.id);
    if (drawingLayer) { env.activeInkLayerId = drawingLayer.id; env.activeInkLayerPath = drawingLayer.path; }
    const typstId = selectedTypstId;
    if (!typstId) {
      env.status = notice("workspace.selectATypstBlockBeforeGroupingIt");
      return;
    }
    const typstObject = env.activeSnapshot?.page.objects.find((object) => object.id === typstId);
    if (
      (typstObject?.groupId && typstObject.groupId !== mixedGroup?.groupId) ||
      (mixedGroup?.active && mixedGroup.typstId !== typstId) ||
      env.strokes.some(
        (stroke) =>
          selectedStrokeIds.includes(stroke.id) &&
          stroke.groupId &&
          stroke.groupId !== mixedGroup?.inkGroupId,
      )
    ) {
      env.status = notice("workspace.ungroupTheExistingSelectionBeforeCreatingAnother");
      return;
    }
    const occupied = new Set(env.activeSnapshot?.page.objects.map((object) => object.id) ?? []);
    const freshId = (prefix: string) => {
      let number = 1;
      while (occupied.has(`${prefix}-${number}`)) number += 1;
      const id = `${prefix}-${number}`;
      occupied.add(id);
      return id;
    };
    mixedGroup ??= {
      inkGroupId: freshId("ink-group"),
      groupId: freshId("group"),
      typstId,
      active: true,
    };
    mixedGroup = { ...mixedGroup, typstId, active: true };
    const selected = new Set(selectedStrokeIds);
    groupedStrokeIds = [...selectedStrokeIds];
    env.strokes = env.strokes.map((stroke) => ({
      ...stroke,
      groupId: selected.has(stroke.id) ? mixedGroup!.inkGroupId : stroke.groupId,
    }));
    env.status = notice("counts.groupedStrokes", { count: groupedStrokeIds.length });
    env.queueCommit(notice("workspace.groupedInkWithTypst"));
  }

  /**
   * Every change to the ink selection goes through here, because the lasso's hand-over to the
   * selection tool has to be undone when the selection empties — and a delete that assigned
   * `selectedStrokeIds` directly would skip that and strand the writer on a tool they never
   * picked.
   */
  function updateInkSelection(ids: string[]) {
    selectedStrokeIds = ids;
    const next = toolAfterSelection(env.tool, ids.length + lassoObjectIds.length + Number(enteredGroupId !== null), selectionReturnTo);
    env.tool = next.tool;
    selectionReturnTo = next.returnTo;
  }

  function restyleSelection(patch: StylePatch) {
    const count = env.selectedInk.length + env.selectedShapes.length;
    env.strokes = restyleStrokes(env.strokes, selectedStrokeIds, patch);
    const ids = new Set(env.selectedShapes.map(shape => shape.id));
    env.shapes = env.shapes.map(shape => {
      if (!ids.has(shape.id)) return shape;
      const style = { ...shape.style };
      if (patch.color !== undefined) {
        style.strokeColor = patch.color;
        if (style.fillColor) style.fillColor = `${patch.color}24`;
      }
      if (patch.widthPt !== undefined) style.strokeWidthPt = patch.widthPt;
      if (patch.pattern !== undefined) style.pattern = patch.pattern;
      if (patch.patternSpacing !== undefined) style.patternSpacing = patch.patternSpacing;
      return { ...shape, style };
    });
    env.queueCommit(notice("workspace.restyledSelection"));
    env.status = notice("counts.restyled", { count: count });
  }

  function routeObjectPointer(event: PointerEvent) {
    // Explicit handles own contact, even with a brush or the Shape tool in hand.
    if (event.target instanceof Element && event.target.closest(".shape-controls .control-hit, .helper-crop, .helper-workspace, .mixed-selection, .typst-block .handle")) {
      directObjectInput = true;
      const role = pointerRole(event, env.tool, env.pointerMapping);
      if (event.type === "pointerdown" && (role === "erase" || role === "lasso" || role === "ignore" && event.pointerType === "pen")) {
        // The eraser end and barrel mappings still take precedence over a handle.
        const ink = event.target.closest(".page")?.querySelector(".ink-surface");
        directObjectInput = false;
        event.preventDefault();
        event.stopPropagation();
        ink?.dispatchEvent(new PointerEvent(event.type, event));
      }
      return;
    }
    // A stylus is for writing, so ink takes it and you can write straight over a block. That was
    // unconditional, which meant a block could never be picked up with the pen that wrote it —
    // there was no tool that gave the stylus the object layer. A selection tool now does.
    //
    // Mouse and touch keep reaching objects under any tool: clicking a block to grab it is how
    // this has always worked, and a brush is no reason to take it away.
    // The bar and its editors restyle whatever is selected, so pressing them must not re-route
    // the press to the page and deselect the very shape about to be restyled.
    if (
      event.target instanceof Element &&
      event.target.closest(".selection-actions, .instrument-palette, .palette-editor, .editor-dock, .shape-panel")
    ) {
      return;
    }
    // The shape tool builds on the ink surface, and the eraser has to reach it to rub anything
    // out, so neither hands the pointer to an object under the cursor.
    if (env.tool === "shape" || env.tool === "eraser" || env.tool === "lasso" || pointerRole(event, env.tool, env.pointerMapping) === "lasso") {
      directObjectInput = false;
      return;
    }
    if ((event.pointerType === "pen" || env.navigation.fingerDraws(event)) && !keepsSelection(env.tool)) {
      directObjectInput = false;
      return;
    }
    const object = document
      .elementsFromPoint(event.clientX, event.clientY)
      .map((element) => element.closest<HTMLElement>(".typst-block, .image-object, .shape-object"))
      .find((element) => element && env.workspace?.contains(element));
    directObjectInput = Boolean(object);
    if (event.type !== "pointerdown") return;
    const objectId =
      object?.dataset.objectId ??
      object?.querySelector<HTMLElement>("[data-object-id]")?.dataset.objectId ??
      null;
    if (enteredGroupId && shapeGroupFor(objectId)?.id !== enteredGroupId) exitGroup();
    lassoObjectIds = [];
    selectedTypstId = object?.classList.contains("typst-block")
      ? objectId
      : null;
    selectedImageId = object?.classList.contains("image-object")
      ? objectId
      : null;
    selectedShapeId = object?.classList.contains("shape-object")
      ? objectId
      : null;
    if (selectedImageId || selectedShapeId) updateInkSelection([]);
    if (selectedShapeId !== editingShapeId) editingShapeId = null;
    if (object && event.target instanceof Element && event.target.closest(".ink-surface")) {
      event.preventDefault();
      event.stopPropagation();
      env.status = selectedTypstId
        ? notice("workspace.typstBlockSelected")
        : selectedImageId
          ? notice("workspace.imageSelected")
          : notice("workspace.shapeSelected");
    }
  }

  function closeObjectSelection(event: PointerEvent) {
    // The card lives inside the bar, so a press anywhere outside it dismisses the card.
    if (!(event.target instanceof Element) || !event.target.closest(".instrument-palette")) {
      env.closeToolCard();
    }
    // A stylus press mid-stroke is writing, not "deselect" — unless a selection tool is active,
    // where a press on empty page means exactly that.
    if (
      ((event.pointerType === "pen" || env.navigation.fingerDraws(event)) && !keepsSelection(env.tool)) ||
      (event.target instanceof Element &&
        event.target.closest(
          ".helper-crop, .helper-workspace, .mixed-selection, .typst-block, .image-object, .shape-object, .selection-actions, .overflow-menu, .instrument-palette, .palette-editor, [data-preserve-selection]",
        ))
    ) {
      return;
    }
    exitGroup();
    if (lassoObjectIds.length) { lassoObjectIds = []; updateInkSelection([]); }
    selectedTypstId = null;
    selectedImageId = null;
    selectedShapeId = null;
    editingShapeId = null;
  }

  /// Delete the selected object or ink as one undoable committed action. Original asset
  /// files are never removed; deleting an image only drops the page's reference.
  function selectedItems() {
    return { objects: [...new Set([...lassoObjectIds, selectedTypstId, selectedImageId, selectedShapeId]
      .filter((id): id is string => id !== null))], strokes: [...selectedStrokeIds] };
  }

  function deleteSelection(selection = selectedItems()): boolean {
    const ids = new Set(selection.objects);
    const inkIds = new Set(selection.strokes);
    if (!ids.size && !inkIds.size) return false;
    const wholeGroups = shapeGroups.filter(group => group.childIds.every(id => ids.has(id)));
    const allowed = new Set([...wholeGroups.map(group => group.id), enteredGroupId, mixedGroup?.groupId]);
    if (env.activeSnapshot?.page.objects.some(object => ids.has(object.id) && object.groupId && groupEdits.get(object.groupId) !== null && !allowed.has(object.groupId)) ||
      env.strokes.some(stroke => inkIds.has(stroke.id) && stroke.groupId && stroke.groupId !== mixedGroup?.inkGroupId)) {
      env.status = notice("workspace.ungroupThisSelectionBeforeDeletingIt");
      return false;
    }
    if (mixedGroup?.active && ids.has(mixedGroup.typstId)) {
      mixedGroup = { ...mixedGroup, active: false };
      env.strokes = env.strokes.map(stroke => groupedStrokeIds.includes(stroke.id) ? { ...stroke, groupId: null } : stroke);
      groupedStrokeIds = [];
    }
    if (wholeGroups.length) {
      const edits = new Map(groupEdits);
      for (const group of wholeGroups) edits.set(group.id, null);
      groupEdits = edits;
    }
    env.typstBlocks = env.typstBlocks.filter(block => !ids.has(block.id));
    for (const image of env.images) if (ids.has(image.id) && image.url.startsWith("blob:")) URL.revokeObjectURL(image.url);
    env.images = env.images.filter(image => !ids.has(image.id));
    env.shapes = env.shapes.filter(shape => !ids.has(shape.id));
    env.strokes = env.strokes.filter(stroke => !inkIds.has(stroke.id));
    groupedStrokeIds = groupedStrokeIds.filter(id => !inkIds.has(id));
    lassoObjectIds = lassoObjectIds.filter(id => !ids.has(id));
    if (selectedTypstId && ids.has(selectedTypstId)) selectedTypstId = null;
    if (selectedImageId && ids.has(selectedImageId)) selectedImageId = null;
    if (selectedShapeId && ids.has(selectedShapeId)) { selectedShapeId = null; editingShapeId = null; }
    updateInkSelection(selectedStrokeIds.filter(id => !inkIds.has(id)));
    env.queueCommit(notice("workspace.deletedSelection"));
    env.status = notice("workspace.deletedSelectedItemsOriginalFilesAreKept");
    return true;
  }

  /// Arrow-key movement for the current selection; batched through the ink debounce so
  /// holding a key produces one committed action.
  function nudgeSelection(dx: number, dy: number): boolean {
    if (mixedGesture) return true;
    if (lassoObjectIds.length) {
      beginMixedTransform(); previewMixedTransform({ x: dx, y: dy }, 1, { x: 0, y: 0 });
      endMixedTransform(false, true);
      return true;
    }
    if (selectedTypstId) {
      env.typstBlocks = env.typstBlocks.map((block) =>
        block.id === selectedTypstId
          ? {
              ...block,
              transform: {
                ...block.transform,
                x: block.transform.x + dx,
                y: block.transform.y + dy,
              },
            }
          : block,
      );
      env.scheduleInkCommit(notice("workspace.movedTypstBlock"));
      return true;
    }
    if (selectedImageId) {
      env.images = env.images.map((image) =>
        image.id === selectedImageId
          ? { ...image, x: image.x + dx, y: image.y + dy }
          : image,
      );
      env.scheduleInkCommit(notice("workspace.movedImage"));
      return true;
    }
    if (selectedShapeId) {
      env.shapes = env.shapes.map((shape) =>
        shape.id === selectedShapeId
          ? { ...shape, x: shape.x + dx, y: shape.y + dy }
          : shape,
      );
      env.scheduleInkCommit(notice("workspace.movedShape"));
      return true;
    }
    if (selectedStrokeIds.length > 0) {
      env.strokes = moveSelected(env.strokes, selectedStrokeIds, { x: dx, y: dy });
      env.scheduleInkCommit(notice("workspace.movedInkSelection"));
      return true;
    }
    return false;
  }

  function ungroupInk() {
    env.strokes = env.strokes.map((stroke) =>
      groupedStrokeIds.includes(stroke.id) &&
      (!mixedGroup || stroke.groupId === mixedGroup.inkGroupId)
        ? { ...stroke, groupId: null }
        : stroke,
    );
    groupedStrokeIds = [];
    if (mixedGroup) mixedGroup = { ...mixedGroup, active: false };
    env.status = notice("workspace.removedTheTypstAndInkGroup");
    env.queueCommit(notice("workspace.ungroupedInkAndTypst"));
  }

  /**
   * Copy, cut, paste and duplicate (specs/qol-wave.md section 3). The bundle lives in the app so it
   * survives page and notebook switches; the system clipboard only carries a marker, so a paste
   * after copying something in another program does not bring back a stale Goodtype copy.
   */
  // Raw: a bundle is replaced whole, never edited in place, and may carry image bytes.
  let clipboard = $state.raw<ClipboardBundle | null>(null);

  let pasteCount = 0;

  function hasCopyableSelection(): boolean {
    return (
      lassoObjectIds.length > 0 ||
      selectedStrokeIds.length > 0 ||
      selectedShapeId !== null ||
      selectedTypstId !== null ||
      selectedImageId !== null
    );
  }

  async function imageBytes(url: string): Promise<{ bytes: Uint8Array; mime: string } | null> {
    try {
      const response = await fetch(url);
      const blob = await response.blob();
      return { bytes: new Uint8Array(await blob.arrayBuffer()), mime: blob.type || "image/png" };
    } catch {
      return null;
    }
  }

  async function bundleSelection(selection = selectedItems()): Promise<ClipboardBundle | null> {
    const context = env.editContext();
    const ids = new Set(selection.strokes);
    const objectIds = new Set(selection.objects);
    const selectedImages = env.images.filter(image => objectIds.has(image.id));
    const canonicalObjects = (env.activeSnapshot?.page.objects ?? []).filter(object => groupEdits.get(object.id) !== null)
      .map(object => object.type === "group" ? { ...object, ...groupEdits.get(object.id) } : object);
    // Capture every value before fetching bytes; selection and page state may change meanwhile.
    const bundle: ClipboardBundle = {
      token: crypto.randomUUID(),
      pageId: context.pageId,
      root: context.root,
      strokes: env.strokes.filter(stroke => ids.has(stroke.id)),
      shapes: env.shapes.filter(shape => objectIds.has(shape.id)),
      blocks: env.typstBlocks.filter(block => objectIds.has(block.id)).map(block => ({
        id: block.id, addonData: block.addonData, source: block.source, ...block.transform,
        heightPt: env.workspace?.querySelector<HTMLElement>(`.active-page [data-object-id="${CSS.escape(block.id)}"]`)?.offsetHeight ?? 0,
      })),
      images: [],
    };
    const imageData = await Promise.all(selectedImages.map(image => imageBytes(image.url)));
    if (!context.current()) return null;
    if (imageData.some(data => !data)) {
      env.status = notice("workspace.anImageCouldNotBeCopiedThe");
      return null;
    }
    bundle.images = selectedImages.map((image, index) => ({
      ...imageData[index]!, id: image.id, addonData: image.addonData, alt: image.alt, x: image.x, y: image.y,
      widthPt: image.widthPt, heightPt: image.heightPt, scale: image.scale, rotation: image.rotation,
    }));
    const copied = new Set([...bundle.shapes, ...bundle.blocks, ...bundle.images]
      .flatMap(({ id }) => id ? [id] : []));
    bundle.groups = completeSelectedGroups(canonicalObjects, copied);
    return isEmptyBundle(bundle) ? null : bundle;
  }

  async function copySelection(cut = false) {
    const context = env.editContext();
    const selection = selectedItems();
    const selectedIds = new Set([...selection.objects, ...selection.strokes]);
    const captured = [...env.typstBlocks, ...env.images, ...env.shapes, ...env.strokes].filter(item => selectedIds.has(item.id));
    const bundle = await bundleSelection(selection);
    if (!context.current()) return;
    if (!bundle) return;
    clipboard = bundle;
    pasteCount = 0;
    try {
      if (!navigator.clipboard) throw new Error("Clipboard unavailable");
      await navigator.clipboard.writeText(clipboardMarker(bundle));
    } catch {
      if (context.current()) env.status = notice("workspace.clipboardWriteFailed");
      return;
    }
    if (!context.current()) return;
    if (cut) {
      const current = new Set([...env.typstBlocks, ...env.images, ...env.shapes, ...env.strokes]);
      if (captured.some(item => !current.has(item))) {
        env.status = notice("workspace.copiedCutWasCanceledBecauseTheCopied");
        return;
      }
      if (deleteSelection(selection)) env.status = notice("workspace.cutToTheClipboard");
    } else env.status = notice("workspace.copied");
  }

  async function duplicateSelection(at: Point | null = null) {
    const context = env.editContext();
    const bundle = await bundleSelection();
    if (!bundle || !context.current()) return;
    clipboard = bundle;
    pasteCount = 0;
    await pasteBundle(bundle, at);
  }

  function takenObjectIds(): Set<string> {
    return new Set([
      ...env.typstBlocks.map((entry) => entry.id),
      ...env.images.map((entry) => entry.id),
      ...env.shapes.map((entry) => entry.id),
      ...(env.activeSnapshot?.page.objects.map((object) => object.id) ?? []),
    ]);
  }

  function freshObjectId(prefix: string, taken: Set<string>): string {
    let number = 1;
    while (taken.has(`${prefix}-${String(number).padStart(3, "0")}`)) number += 1;
    const id = `${prefix}-${String(number).padStart(3, "0")}`;
    taken.add(id);
    return id;
  }

  /// `at` is a page point to centre the paste on (from the context menu); without it a paste
  /// steps away from its original on the same page and keeps its place on any other.
  async function pasteBundle(bundle: ClipboardBundle, at: Point | null = null) {
    const context = env.editContext();
    const paths: string[] = [];
    try {
      for (const image of bundle.images) {
        const filename = `pasted-${crypto.randomUUID()}.${extensionForMime(image.mime)}`;
        paths.push(env.tauriAvailable && context.root
          ? await storePastedImage(context.root, filename, image.bytes)
          : `assets/${filename}`);
        if (!context.current()) return;
      }
    } catch (error) {
      if (context.current()) env.status = notice("workspace.theSelectionCouldNotBePasted", { "value1": env.message(error) });
      return;
    }
    if (!context.current()) return;
    const oldObjects = new Set([...env.typstBlocks, ...env.images, ...env.shapes].map(object => object.id));
    pasteCount += 1;
    const shift = at ? shiftTo(bundle, at) : pasteOffset(bundle, env.activePageId, pasteCount);
    let zIndex = env.nextVisualZIndex();
    let readingOrder = env.nextReadingOrder();

    const newStrokes = pastedStrokes(bundle.strokes, shift, zIndex, () => crypto.randomUUID());
    zIndex += newStrokes.length;
    const newShapes = pastedShapes(bundle.shapes, shift, zIndex, readingOrder, () => crypto.randomUUID());
    const taken = takenObjectIds();
    const destination = bundle.root === context.root && bundle.pageId === context.pageId
      ? new Set(taken) : new Set<string>();
    const blockIds = bundle.blocks.map(() => freshObjectId("typst", taken));
    const imageIds = bundle.images.map(() => freshObjectId("image", taken));
    const copies = new Map<string, string>();
    function recordCopies(originals: { id?: string }[], newIds: string[]) {
      originals.forEach(({ id }, index) => { if (id) copies.set(id, newIds[index]); });
    }
    recordCopies(bundle.shapes, newShapes.map(({ id }) => id));
    recordCopies(bundle.blocks, blockIds);
    recordCopies(bundle.images, imageIds);
    const groups = bundle.groups ?? [];
    recordCopies(groups, groups.map(() => crypto.randomUUID()));
    const parents = new Map(groups.flatMap((group) => group.childIds.map((id) => [id, copies.get(group.id)!] as const)));
    for (const [index, shape] of newShapes.entries()) {
      shape.groupId = parents.get(bundle.shapes[index].id) ?? null;
      if (shape.addonData) shape.addonData = pastedAddonData(shape.addonData, copies, destination);
    }
    zIndex += newShapes.length;
    readingOrder += newShapes.length;

    let lastObject: { kind: "typst" | "image" | "shape"; id: string } | null =
      newShapes.length > 0 ? { kind: "shape", id: newShapes.at(-1)!.id } : null;
    if (newShapes.length > 0) env.shapes = [...env.shapes, ...newShapes];

    for (const [index, block] of bundle.blocks.entries()) {
      const id = blockIds[index];
      env.typstBlocks = [
        ...env.typstBlocks,
        {
          id,
          path: `blocks/${env.activePageId}-${id}.typ`,
          source: block.source,
          addonData: pastedAddonData(block.addonData, copies, destination),
          groupId: block.id ? parents.get(block.id) : undefined,
          transform: {
            x: block.x + shift.dx,
            y: block.y + shift.dy,
            layoutWidthPt: block.layoutWidthPt,
            rotation: block.rotation ?? 0,
            scale: block.scale,
          },
          result: null,
          zIndex: zIndex++,
          readingOrder: readingOrder++,
        },
      ];
      lastObject = { kind: "typst", id };
    }

    for (const [index, image] of bundle.images.entries()) {
      const path = paths[index];
      const id = imageIds[index];
      env.images = [
        ...env.images,
        {
          id,
          path,
          url: URL.createObjectURL(new Blob([image.bytes.slice()], { type: image.mime })),
          addonData: pastedAddonData(image.addonData, copies, destination),
          groupId: image.id ? parents.get(image.id) : undefined,
          alt: image.alt,
          x: image.x + shift.dx,
          y: image.y + shift.dy,
          widthPt: image.widthPt,
          heightPt: image.heightPt,
          rotation: image.rotation ?? 0,
          scale: image.scale,
          zIndex: zIndex++,
          readingOrder: readingOrder++,
        },
      ];
      lastObject = { kind: "image", id };
    }

    const now = new Date().toISOString();
    if (groups.length) env.addCopiedGroups?.(groups.map((group) => ({
      ...group, id: copies.get(group.id)!, childIds: group.childIds.map((id) => copies.get(id)!),
      ...(group.stretch ? { stretch: { ...group.stretch, parts: group.stretch.parts.map(part => ({
        ...part, childId: copies.get(part.childId)!,
      })) } } : {}),
      x: group.x + shift.dx, y: group.y + shift.dy, zIndex: zIndex++, readingOrder: readingOrder++,
      groupId: parents.get(group.id) ?? null, createdAt: now, modifiedAt: now,
      addonData: pastedAddonData(group.addonData, copies, destination),
    })));

    if (newStrokes.length > 0) env.changeStrokes([...env.strokes, ...newStrokes], notice("workspace.pastedInk"));
    if (lastObject) env.queueCommit(notice("workspace.pasted"));

    // The paste stays selected, ready to be dragged into place.
    lassoObjectIds = [...env.typstBlocks, ...env.images, ...env.shapes].filter(object => !oldObjects.has(object.id)).map(object => object.id);
    selectedTypstId = null;
    selectedImageId = null;
    selectedShapeId = null;
    editingShapeId = null;
    updateInkSelection(newStrokes.map((stroke) => stroke.id));
    const count =
      newStrokes.length + newShapes.length + bundle.blocks.length + bundle.images.length;
    env.status = notice("counts.pasted", { count: count });
  }

  /**
   * The canvas context menu: right-click, or a long press with a finger. Paste lands where it was
   * opened; with something selected it also offers the selection's own actions.
   */
  let contextMenu = $state<{ x: number; y: number; at: Point | null } | null>(null);

  function openContextMenu(clientX: number, clientY: number) {
    env.closeToolCard();
    contextMenu = { x: clientX, y: clientY, at: env.navigation.pagePointAt(clientX, clientY) };
  }

  let lastCanvasPointer: { type: string; time: number } | undefined;

  function noteCanvasPointer(event: PointerEvent) {
    lastCanvasPointer = { type: event.pointerType, time: event.timeStamp };
  }

  function canvasContextMenu(event: MouseEvent & { pointerType?: string }) {
    // A pen's barrel press is a tool, handled on the ink surface, which cancels the menu there.
    if (event.defaultPrevented) return;
    const target = event.target instanceof Element ? event.target : null;
    if (!target?.closest(".page-scroll-content")) return;
    if (target.closest(".cm-editor, input, textarea, [contenteditable=true]")) return;
    event.preventDefault();
    // Windows/browser touch right-taps can arrive even after the long-press timer was cancelled.
    if (!nativeContextMenuAllowed(event)) return;
    // A Windows pen hold can also arrive as a compatibility mouse event.
    if (lastCanvasPointer && (lastCanvasPointer.type === "pen" || lastCanvasPointer.type === "touch") &&
      event.timeStamp - lastCanvasPointer.time < 1500) return;
    openContextMenu(event.clientX, event.clientY);
  }

  let contextBarElement = $state<HTMLElement>();

  let contextBarPosition = $state({ left: 0, top: 0, ready: false });

  /** Closes the bar and runs a context-bar action, handing it the page point the bar opened at. */
  function fromContext(action: (at: Point | null) => unknown) {
    return () => {
      const at = contextMenu?.at ?? null;
      contextMenu = null;
      void action(at);
    };
  }

  async function handlePaste(event: ClipboardEvent) {
    if (env.pageInputLocked) return;
    // Text fields and editors paste their own way.
    if (
      event.target instanceof Element &&
      event.target.closest(".cm-editor, input, textarea, [contenteditable=true]")
    ) {
      return;
    }
    const hasImageFile = Array.from(event.clipboardData?.files ?? []).some((item) =>
      item.type.startsWith("image/"),
    );
    if (hasImageFile) {
      await pasteImage(event);
      return;
    }
    const text = event.clipboardData?.getData("text/plain") ?? "";
    if (clipboard && (text === clipboardMarker(clipboard) || text === "")) {
      event.preventDefault();
      await pasteBundle(clipboard);
    }
  }

  async function pasteImage(event: ClipboardEvent) {
    const file = Array.from(event.clipboardData?.files ?? []).find((item) =>
      item.type.startsWith("image/"),
    );
    if (!file) return;
    event.preventDefault();
    if (file.size > 20 * 1024 * 1024) {
      env.status = notice("workspace.thatImageIsLargerThanTheMiB");
      return;
    }
    const context = env.editContext();
    const url = URL.createObjectURL(file);
    const dimensions = await imageDimensions(url);
    const filename = `pasted-${crypto.randomUUID()}.${extensionForMime(file.type)}`;

    // Store the original once, here. Commits then reference the asset by path instead of
    // carrying its bytes, so ordinary pen strokes never marshal an image across IPC.
    let path = `assets/${filename}`;
    if (env.tauriAvailable && env.root) {
      try {
        path = await storePastedImage(context.root, filename, new Uint8Array(await file.arrayBuffer()));
      } catch (error) {
        URL.revokeObjectURL(url);
        env.status = notice("workspace.theImageCouldNotBeStored", { "value1": env.message(error) });
        return;
      }
    }

    if (!context.current()) { URL.revokeObjectURL(url); return; }

    const fit = Math.min(1, 220 / dimensions.width, 160 / dimensions.height);
    const existingIds = new Set([
      ...env.typstBlocks.map((block) => block.id),
      ...env.images.map((image) => image.id),
      ...env.shapes.map((shape) => shape.id),
      ...(env.activeSnapshot?.page.objects.map((object) => object.id) ?? []),
    ]);
    let number = env.images.length + 1;
    while (existingIds.has(`image-${String(number).padStart(3, "0")}`)) number += 1;
    const id = `image-${String(number).padStart(3, "0")}`;
    env.images = [...env.images, {
      id,
      path,
      url,
      alt: file.name || translate("workspace.pastedImage"),
      x: 300,
      y: 380,
      widthPt: Math.max(1, dimensions.width * fit),
      heightPt: Math.max(1, dimensions.height * fit),
      scale: 1,
      zIndex: env.nextVisualZIndex(),
      readingOrder: env.nextReadingOrder(),
    }];
    selectedImageId = id;
    selectedTypstId = null;
    selectedShapeId = null;
    editingShapeId = null;
    env.status = notice("workspace.pastedOneOriginalImage");
    env.queueCommit(notice("workspace.pastedImage"));
  }

  /** How many things Align and Distribute would move: whole groups, loose objects, the ink. */
  const alignCount = $derived.by(() => {
    const live = new Set([...env.shapes, ...env.images, ...env.typstBlocks].map(object => object.id));
    const chosen = new Set(lassoObjectIds);
    const taken = new Set<string>();
    let count = selectedStrokeIds.length ? 1 : 0;
    for (const group of shapeGroups) {
      if (!group.childIds.every(id => chosen.has(id))) continue;
      group.childIds.forEach(id => taken.add(id));
      count++;
    }
    return count + lassoObjectIds.filter(id => live.has(id) && !taken.has(id)).length;
  });

  const INK_UNIT = "\u0000ink";

  /** The selection as things that move whole, with the rotated page box of each. */
  function alignUnits() {
    const boxes = objectBoxes();
    const chosen = new Set(lassoObjectIds);
    const taken = new Set<string>();
    const units: (AlignUnit & { members: string[]; group?: (typeof shapeGroups)[number] })[] = [];
    for (const group of shapeGroups) {
      if (!group.childIds.every(id => chosen.has(id))) continue;
      const parts = group.childIds.flatMap(id => boxes.get(id) ?? []);
      if (!parts.length) continue;
      group.childIds.forEach(id => taken.add(id));
      units.push({ id: group.id, box: unionBounds(parts), members: [...group.childIds], group });
    }
    for (const id of lassoObjectIds) {
      const box = boxes.get(id);
      if (box && !taken.has(id)) units.push({ id, box, members: [id] });
    }
    const ink = selectedStrokeIds.length ? selectionBounds(env.strokes, selectedStrokeIds) : null;
    if (ink) units.push({ id: INK_UNIT, box: ink, members: [] });
    return units;
  }

  /** Applies one delta per unit as a single change, so one undo step puts the selection back. */
  function applyUnitDeltas(deltas: Map<string, Point>, label: UiText) {
    const shifts = new Map<string, Point>();
    const edits = new Map(groupEdits);
    let ink: Point | null = null;
    for (const unit of alignUnits()) {
      const delta = deltas.get(unit.id);
      if (!delta || (Math.abs(delta.x) < 1e-9 && Math.abs(delta.y) < 1e-9)) continue;
      unit.members.forEach(id => shifts.set(id, delta));
      if (unit.group) edits.set(unit.group.id, { x: unit.group.x + delta.x, y: unit.group.y + delta.y,
        scale: unit.group.scale, rotation: unit.group.rotation });
      if (unit.id === INK_UNIT) ink = delta;
    }
    if (!shifts.size && !ink) return;
    groupEdits = edits;
    env.shapes = env.shapes.map(shape => {
      const delta = shifts.get(shape.id);
      return delta ? { ...shape, x: shape.x + delta.x, y: shape.y + delta.y } : shape;
    });
    env.images = env.images.map(image => {
      const delta = shifts.get(image.id);
      return delta ? { ...image, x: image.x + delta.x, y: image.y + delta.y } : image;
    });
    env.typstBlocks = env.typstBlocks.map(block => {
      const delta = shifts.get(block.id);
      return delta ? { ...block, transform: { ...block.transform, x: block.transform.x + delta.x, y: block.transform.y + delta.y } } : block;
    });
    if (ink) env.strokes = moveSelected(env.strokes, selectedStrokeIds, ink);
    env.queueCommit(label);
    env.status = label;
  }

  function alignSelection(mode: AlignMode) {
    if (alignCount < 2) return;
    applyUnitDeltas(alignDeltas(alignUnits(), mode), notice("selectionMenu.alignedSelection"));
  }

  function distributeSelection(axis: DistributeAxis) {
    if (alignCount < 3) return;
    applyUnitDeltas(distributeDeltas(alignUnits(), axis), notice("selectionMenu.distributedSelection"));
  }

  function selectedObjectId(): string | null {
    return selectedTypstId ?? selectedImageId ?? selectedShapeId;
  }

  function canMoveVisual(direction: -1 | 1): boolean {
    const id = selectedObjectId();
    if (!id || !env.activeSnapshot) return false;
    return moveVisualItems(
      env.activeSnapshot.page,
      env.strokes,
      [id],
      [],
      direction < 0 ? "backward" : "forward",
    ) !== null;
  }

  function canMoveReading(direction: -1 | 1): boolean {
    const id = selectedObjectId();
    if (!id || !env.activeSnapshot) return false;
    const index = env.activeSnapshot.page.readingOrder.indexOf(id);
    return direction < 0
      ? index > 0
      : index >= 0 && index < env.activeSnapshot.page.readingOrder.length - 1;
  }

  function inkActionIds(): string[] {
    return selectedStrokeIds.length > 0 ? selectedStrokeIds : groupedStrokeIds;
  }

  function canMoveInkVisual(direction: -1 | 1): boolean {
    return Boolean(
      env.activeSnapshot &&
        moveVisualItems(
          env.activeSnapshot.page,
          env.strokes,
          lassoObjectIds,
          inkActionIds(),
          direction < 0 ? "backward" : "forward",
        ),
    );
  }

  function changeInkVisualOrder(direction: -1 | 1) {
    if (!env.activeSnapshot) return;
    const next = moveVisualItems(
      env.activeSnapshot.page,
      env.strokes,
      lassoObjectIds,
      inkActionIds(),
      direction < 0 ? "backward" : "forward",
    );
    if (!next) return;
    env.strokes = next.strokes;
    applyObjectOrder(next.page, notice("workspace.changedVisualOrder"));
    env.status = direction < 0 ? notice("workspace.movedSelectedInkBack") : notice("workspace.movedSelectedInkForward");
  }

  function scheduleSelectionToolbar(attempt = 0) {
    if (selectionToolbarFrame) cancelAnimationFrame(selectionToolbarFrame);
    if (env.zooming) return;
    selectionToolbarFrame = requestAnimationFrame(() => {
      selectionToolbarFrame = undefined;
      // A block just created or pasted is selected before it is on the page: its element appears
      // a few frames later. Keep looking briefly instead of leaving the toolbar hidden for good.
      if (env.zooming) return;
      if (!positionSelectionToolbar() && attempt < 30) scheduleSelectionToolbar(attempt + 1);
    });
  }

  /** Places the toolbar; false when the selection's element is not on the page yet. */
  function positionSelectionToolbar(): boolean {
    if (!env.workspace || !selectionToolbarElement) return false;
    let anchor: ViewRect | null = null;
    const objectId = selectedObjectId();
    if (objectId) {
      // A selected shape draws its handles in a second layer under the same id; the rotate handle
      // sits above the outline, so placing the bar against the outline alone covered it.
      const rects = [...env.workspace
        .querySelectorAll<Element>(`.active-page [data-object-id="${CSS.escape(objectId)}"]`)]
        .map((element) => element.getBoundingClientRect());
      if (rects.length) {
        const left = Math.min(...rects.map((rect) => rect.left));
        const top = Math.min(...rects.map((rect) => rect.top));
        const right = Math.max(...rects.map((rect) => rect.right));
        const bottom = Math.max(...rects.map((rect) => rect.bottom));
        anchor = { left, top, right, bottom, width: right - left, height: bottom - top };
      }
    } else {
      const bounds = env.regionBounds ?? mixedSelectionBounds ?? selectionBounds(env.strokes, inkActionIds());
      const frame = env.pageFrame?.getBoundingClientRect();
      if (bounds && frame) {
        const view = { pageOriginX: 0, pageOriginY: 0, zoom: env.zoom };
        const topLeft = pageToScreen({ x: bounds.left, y: bounds.top }, frame, view);
        const bottomRight = pageToScreen({ x: bounds.right, y: bounds.bottom }, frame, view);
        anchor = {
          left: topLeft.x, top: topLeft.y - (selectedGroup ? 48 : 0), right: bottomRight.x, bottom: bottomRight.y,
          width: bottomRight.x - topLeft.x, height: bottomRight.y - topLeft.y + (selectedGroup ? 48 : 0),
        };
      }
    }
    if (!anchor) return false;
    const boundary = env.workspace.getBoundingClientRect();
    if (
      anchor.right < boundary.left ||
      anchor.left > boundary.right ||
      anchor.bottom < boundary.top ||
      anchor.top > boundary.bottom
    ) {
      selectionToolbarPosition = { left: 0, top: 0, ready: false };
      return true;
    }
    // The palette and the undo card float over the page too; the bar must not land on them.
    const chrome = [...env.workspace.querySelectorAll<HTMLElement>(".instrument-palette, .history-pill")]
      .map((element) => element.getBoundingClientRect());
    const placed = placeFloatingToolbar(
      anchor,
      selectionToolbarElement.getBoundingClientRect(),
      boundary,
      10,
      12,
      chrome,
    );
    selectionToolbarPosition = { left: placed.left, top: placed.top, ready: true };
    return true;
  }

  function applyObjectOrder(page: NonNullable<typeof env.activeSnapshot>["page"], label: UiText) {
    if (!env.activeSnapshot) return;
    env.activeSnapshot = { ...env.activeSnapshot, page };
    const fields = new Map(page.objects.map((object) => [object.id, object]));
    env.typstBlocks = env.typstBlocks.map((block) => {
      const object = fields.get(block.id);
      return object
        ? { ...block, zIndex: object.zIndex, readingOrder: object.readingOrder }
        : block;
    });
    env.images = env.images.map((image) => {
      const object = fields.get(image.id);
      return object
        ? { ...image, zIndex: object.zIndex, readingOrder: object.readingOrder }
        : image;
    });
    env.shapes = env.shapes.map((shape) => {
      const object = fields.get(shape.id);
      return object
        ? { ...shape, zIndex: object.zIndex, readingOrder: object.readingOrder }
        : shape;
    });
    env.queueCommit(label);
  }

  function changeVisualOrder(move: VisualMove) {
    const id = selectedObjectId();
    if (!id || !env.activeSnapshot) return;
    const next = moveVisualItems(env.activeSnapshot.page, env.strokes, [id], [], move);
    if (!next) return;
    env.strokes = next.strokes;
    applyObjectOrder(next.page, notice("workspace.changedVisualOrder"));
  }

  function changeReadingOrder(direction: -1 | 1) {
    const id = selectedObjectId();
    if (!id || !env.activeSnapshot) return;
    const next = moveReadingObject(env.activeSnapshot.page, id, direction);
    if (next === env.activeSnapshot.page) {
      env.status = notice("workspace.thatObjectCannotMoveFurtherInReading");
      return;
    }
    applyObjectOrder(next, notice("workspace.changedReadingOrder"));
  }

  function imageDimensions(url: string): Promise<{ width: number; height: number }> {
    return new Promise((resolve, reject) => {
      const candidate = new Image();
      candidate.onload = () =>
        resolve({ width: candidate.naturalWidth, height: candidate.naturalHeight });
      candidate.onerror = () => reject(new Error(translate("workspace.thePastedImageCouldNotBeDecoded")));
      candidate.src = url;
    });
  }

  function extensionForMime(mime: string) {
    if (mime === "image/jpeg") return "jpg";
    if (mime === "image/svg+xml") return "svg";
    if (mime === "image/webp") return "webp";
    return "png";
  }
  function dispose() {
    endWasmPreview();
    if (selectionToolbarFrame) cancelAnimationFrame(selectionToolbarFrame);
    selectionToolbarFrame = undefined;
  }
  return {
    dispose,
    selectGroup,
    selectShape,
    enterGroup,
    exitGroup,
    ungroupShapes,
    shapeGroupFor,
    get selectedGroup() { return selectedGroup; },
    get previewGroup() { return previewGroup; },
    get enteredGroup() { return enteredGroup; },
    get groupEdits() { return groupEdits; },
    set groupEdits(value: typeof groupEdits) { groupEdits = value; },
    lassoSelection,
    moveSnap,
    placementSnapFrames: () => snapFrames(objectBoxes(), new Set()),
    get snapGuides() { return snapGuides; },
    get alignCount() { return alignCount; },
    alignSelection,
    distributeSelection,
    beginMixedTransform,
    previewMixedTransform,
    endMixedTransform,
    groupSelectedInk,
    updateInkSelection,
    restyleSelection,
    routeObjectPointer,
    closeObjectSelection,
    deleteSelection,
    nudgeSelection,
    ungroupInk,
    hasCopyableSelection,
    copySelection,
    duplicateSelection,
    pasteBundle,
    openContextMenu,
    canvasContextMenu,
    noteCanvasPointer,
    fromContext,
    handlePaste,
    selectedObjectId,
    canMoveVisual,
    canMoveReading,
    canMoveInkVisual,
    changeInkVisualOrder,
    scheduleSelectionToolbar,
    changeVisualOrder,
    changeReadingOrder,
    get selectedStrokeIds() { return selectedStrokeIds; },
    set selectedStrokeIds(value: typeof selectedStrokeIds) { selectedStrokeIds = value; },
    get groupedStrokeIds() { return groupedStrokeIds; },
    set groupedStrokeIds(value: typeof groupedStrokeIds) { groupedStrokeIds = value; },
    get selectedImageId() { return selectedImageId; },
    set selectedImageId(value: typeof selectedImageId) { selectedImageId = value; },
    get selectedShapeId() { return selectedShapeId; },
    set selectedShapeId(value: typeof selectedShapeId) { selectedShapeId = value; },
    get editingShapeId() { return editingShapeId; },
    set editingShapeId(value: typeof editingShapeId) { editingShapeId = value; },
    get mixedGroup() { return mixedGroup; },
    set mixedGroup(value: typeof mixedGroup) { mixedGroup = value; },
    get selectedTypstId() { return selectedTypstId; },
    set selectedTypstId(value: typeof selectedTypstId) { selectedTypstId = value; },
    get lassoObjectIds() { return lassoObjectIds; },
    set lassoObjectIds(value: typeof lassoObjectIds) { lassoObjectIds = value; },
    get mixedStrokePreview() { return mixedStrokePreview; },
    get previewShapes() { return previewShapes; },
    get wasmMarks() { return wasmMarks; },
    previewShapeGhost,
    get previewImages() { return previewImages; },
    get previewBlocks() { return previewBlocks; },
    get selectionReturnTo() { return selectionReturnTo; },
    set selectionReturnTo(value: typeof selectionReturnTo) { selectionReturnTo = value; },
    get directObjectInput() { return directObjectInput; },
    set directObjectInput(value: typeof directObjectInput) { directObjectInput = value; },
    get mixedSelectionBounds() { return mixedSelectionBounds; },
    get clipboard() { return clipboard; },
    get contextMenu() { return contextMenu; },
    set contextMenu(value: typeof contextMenu) { contextMenu = value; },
    get contextBarElement() { return contextBarElement; },
    set contextBarElement(value: typeof contextBarElement) { contextBarElement = value; },
    get contextBarPosition() { return contextBarPosition; },
    set contextBarPosition(value: typeof contextBarPosition) { contextBarPosition = value; },
    get selectionToolbarElement() { return selectionToolbarElement; },
    set selectionToolbarElement(value: typeof selectionToolbarElement) { selectionToolbarElement = value; },
    get selectionToolbarPosition() { return selectionToolbarPosition; },
    set selectionToolbarPosition(value: typeof selectionToolbarPosition) { selectionToolbarPosition = value; },
  };
}
