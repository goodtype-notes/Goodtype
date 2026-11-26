<script lang="ts">
  import { t } from "../i18n";
  import type { Bounds } from "../ink/selection";
  import type { Point } from "../geometry/coordinates";
  import { viewportToPage } from "../geometry/coordinates";
  import { selectionDragTransform, selectionHandles } from "./lasso";
  import { onDestroy } from "svelte";
  let { bounds, zoom, onStart, onPreview, onEnd, onEnter }: {
    bounds: Bounds; zoom: number; onStart: () => void;
    onPreview: (delta: Point, scale: number, anchor: Point, rotation?: number) => void;
    onEnter?: (event?: PointerEvent) => void;
    onEnd: (cancelled: boolean) => void;
  } = $props();
  const handles = $derived(selectionHandles(bounds, zoom));
  let drag: { id: number; x: number; y: number; bounds: Bounds; kind: "move" | "scale" | "rotate"; moved: boolean } | null = null;
  let previewFrame = 0;
  let pending: { delta: Point; scale: number; anchor: Point; rotation: number } | null = null;
  function flushPreview() {
    if (previewFrame) cancelAnimationFrame(previewFrame);
    previewFrame = 0;
    const preview = pending;
    pending = null;
    if (preview) onPreview(preview.delta, preview.scale, preview.anchor, preview.rotation);
  }
  onDestroy(() => {
    if (previewFrame) cancelAnimationFrame(previewFrame);
    if (drag) onEnd(true);
  });
  function begin(event: PointerEvent, kind: "move" | "scale" | "rotate" = "move") {
    if (event.button !== 0 || event.buttons !== 1) return;
    event.preventDefault(); event.stopPropagation();
    const target = event.currentTarget as SVGElement;
    target.focus({ preventScroll: true });
    target.setPointerCapture(event.pointerId);
    drag = { id: event.pointerId, x: event.clientX, y: event.clientY, bounds: { ...bounds }, kind, moved: false };
    onStart();
  }
  function move(event: PointerEvent) {
    if (!drag || event.pointerId !== drag.id) return;
    event.preventDefault(); event.stopPropagation();
    if (Math.hypot(event.clientX - drag.x, event.clientY - drag.y) >= 4) drag.moved = true;
    if (onEnter && !drag.moved) return;
    const { x: dx, y: dy } = viewportToPage({ x: event.clientX - drag.x, y: event.clientY - drag.y },
      { pageOriginX: 0, pageOriginY: 0, zoom });
    pending = selectionDragTransform(drag.kind, drag.bounds, { x: dx, y: dy }, zoom);

    if (!previewFrame) previewFrame = requestAnimationFrame(flushPreview);
  }
  function end(event: PointerEvent) {
    if (!drag || event.pointerId !== drag.id) return;
    event.preventDefault(); event.stopPropagation();
    const cancelled = event.type !== "pointerup";
    if (!cancelled) { move(event); flushPreview(); }
    else { pending = null; flushPreview(); }
    const enter = !cancelled && drag.kind === "move" && !drag.moved;
    drag = null;
    onEnd(cancelled);
    if (enter) onEnter?.(event);
  }
  function keydown(event: KeyboardEvent) {
    if (event.key === "Enter" && onEnter) {
      event.preventDefault(); event.stopPropagation();
      onEnter();
      return;
    }
    if (event.key !== "+" && event.key !== "=" && event.key !== "-") return;
    event.preventDefault(); event.stopPropagation();
    onStart();
    onPreview({ x: 0, y: 0 }, event.key === "-" ? 0.9 : 1.1, { x: bounds.left, y: bounds.top });
    onEnd(false);
  }
</script>

<svg class="mixed-selection" role="group" aria-label={$t(onEnter ? "workspace.shapeGroupSelected" : "workspace.mixedCanvasSelection")} style:--zoom={zoom}>
  <rect role="button" tabindex="0" aria-label={$t(onEnter ? "workspace.shapeGroupSelected" : "workspace.moveSelectedItems")} use:focusOnMount
    x={bounds.left} y={bounds.top} width={Math.max(1, bounds.right - bounds.left)} height={Math.max(1, bounds.bottom - bounds.top)}
    onkeydown={keydown} onpointerdown={event => begin(event)} onpointermove={move} onpointerup={end} onpointercancel={end} onlostpointercapture={end} />
  <circle role="button" tabindex="0" aria-label={$t("workspace.scaleSelectedItems")} cx={handles.scale.x} cy={handles.scale.y} r={22 / zoom}
    onkeydown={keydown} onpointerdown={event => begin(event, "scale")} onpointermove={move} onpointerup={end} onpointercancel={end} onlostpointercapture={end} />
  <circle class="visible-handle" cx={handles.scale.x} cy={handles.scale.y} r={6 / zoom} />
  {#if onEnter}
    <circle class="rotate" role="button" tabindex="0" aria-label={$t("workspace.rotateShapeGroup")}
      cx={handles.rotate.x} cy={handles.rotate.y} r={22 / zoom}
      onpointerdown={event => begin(event, "rotate")} onpointermove={move} onpointerup={end} onpointercancel={end} onlostpointercapture={end} />
    <circle class="visible-handle" cx={handles.rotate.x} cy={handles.rotate.y} r={6 / zoom} />
  {/if}
</svg>

<script lang="ts" module>
  function focusOnMount(node: SVGElement) { node.focus({ preventScroll: true }); }
</script>

<style>
  .mixed-selection { position: absolute; inset: 0; width: 100%; height: 100%; overflow: visible; z-index: 2147483647; pointer-events: none; }
  rect { fill: transparent; stroke: var(--paper-select); stroke-width: calc(1.5px / var(--zoom)); stroke-dasharray: calc(4px / var(--zoom)) calc(3px / var(--zoom)); pointer-events: all; cursor: move; touch-action: none; }
  circle { fill: transparent; pointer-events: all; cursor: nwse-resize; touch-action: none; }
  .rotate { cursor: grab; }
  .visible-handle { fill: var(--paper); stroke: var(--paper-select); stroke-width: calc(1.5px / var(--zoom)); pointer-events: none; }
  [role=button]:focus-visible { outline: 2px solid var(--paper-select); outline-offset: 2px; }
</style>
