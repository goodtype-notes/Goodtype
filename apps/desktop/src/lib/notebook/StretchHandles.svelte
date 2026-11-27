<script lang="ts">
  import { onDestroy } from "svelte";
  import { viewportToPage, type Point } from "../geometry/coordinates";
  import { snapAround } from "../shape/edit";
  import { t } from "../i18n";
  import type { StretchRequest } from "../ipc/notebook";
  import { pagePorts } from "./addonPorts";
  import type { ShapeView } from "./pageView";
  import { previewStretch, type StretchGroup, type StretchPreview } from "./stretch";

  let { group, shapes, pageId, revision, zoom, disabled, onPreview, onCommit }: {
    group: StretchGroup;
    shapes: readonly ShapeView[];
    pageId: string;
    revision: number;
    zoom: number;
    disabled: boolean;
    onPreview: (value: StretchPreview | null) => void;
    onCommit: (request: StretchRequest) => Promise<void>;
  } = $props();

  let preview = $state.raw<StretchPreview | null>(null);
  const ports = $derived(pagePorts(preview?.group ?? group));
  let drag: { id: number; screen: Point; start: Point; fixed: Point; group: StretchGroup;
    shapes: readonly ShapeView[]; request: StretchRequest; zoom: number } | null = null;
  let frame = 0;
  let committing = false;
  let destroyed = false;

  function show(value: StretchPreview | null) {
    preview = value;
    onPreview(value);
  }

  function request(port: number, endpoint: Point): StretchRequest {
    return { pageId, revision, groupId: group.id, port, endpoint, modifiedAt: new Date().toISOString() };
  }

  function begin(event: PointerEvent, port: number) {
    if (disabled || drag || committing || event.button !== 0 || event.buttons !== 1) return;
    event.preventDefault();
    event.stopPropagation();
    const target = event.currentTarget as SVGElement;
    target.focus({ preventScroll: true });
    target.setPointerCapture(event.pointerId);
    const current = pagePorts(group);
    drag = { id: event.pointerId, screen: { x: event.clientX, y: event.clientY },
      start: current[port], fixed: current[1 - port], group, shapes, request: request(port, current[port]), zoom };
  }

  function update(event: PointerEvent) {
    if (!drag) return;
    const offset = viewportToPage({ x: event.clientX - drag.screen.x, y: event.clientY - drag.screen.y },
      { pageOriginX: 0, pageOriginY: 0, zoom: drag.zoom });
    const point = { x: drag.start.x + offset.x, y: drag.start.y + offset.y };
    drag.request.endpoint = event.shiftKey && (offset.x !== 0 || offset.y !== 0) ? snapAround(point, drag.fixed, 0) : point;
    show(previewStretch(drag.group, drag.shapes, drag.request.port, drag.request.endpoint));
  }

  function move(event: PointerEvent) {
    if (drag?.id !== event.pointerId) return;
    event.stopPropagation();
    cancelAnimationFrame(frame);
    frame = requestAnimationFrame(() => update(event));
  }

  async function commit(change: StretchRequest) {
    committing = true;
    try {
      await onCommit(change);
    } finally {
      committing = false;
      if (!destroyed) show(null);
    }
  }

  function end(event: PointerEvent) {
    if (drag?.id !== event.pointerId) return;
    event.stopPropagation();
    cancelAnimationFrame(frame);
    if (event.type === "pointerup") update(event);
    const change = { ...drag.request, modifiedAt: new Date().toISOString() };
    drag = null;
    if (event.type === "pointerup" && preview) void commit(change);
    else show(null);
  }

  function keydown(event: KeyboardEvent, port: number) {
    if (disabled || drag || committing) return;
    const endpoints = pagePorts(group);
    const step = (event.shiftKey ? 10 : 1) / zoom;
    const moves: Record<string, Point> = {
      ArrowLeft: { x: -step, y: 0 }, ArrowRight: { x: step, y: 0 },
      ArrowUp: { x: 0, y: -step }, ArrowDown: { x: 0, y: step },
    };
    const move = moves[event.key];
    if (!move) return;
    event.preventDefault();
    event.stopPropagation();
    const endpoint = { x: endpoints[port].x + move.x, y: endpoints[port].y + move.y };
    show(previewStretch(group, shapes, port, endpoint));
    if (preview) void commit(request(port, endpoint));
  }

  onDestroy(() => {
    destroyed = true;
    cancelAnimationFrame(frame);
    onPreview(null);
  });
</script>

<svg class="stretch-handles" style:--zoom={zoom}>
  {#each group.stretch?.axis ?? [] as port, index (port)}
    <circle class="hit" class:disabled role="button" tabindex={disabled ? -1 : 0}
      aria-disabled={disabled}
      aria-label={$t(index === 0 ? "workspace.stretchStartPort" : "workspace.stretchEndPort")}
      cx={ports[port].x} cy={ports[port].y} r={14 / zoom}
      onkeydown={event => keydown(event, port)} onpointerdown={event => begin(event, port)}
      onpointermove={move} onpointerup={end} onpointercancel={end} onlostpointercapture={end} />
    <circle class="dot" cx={ports[port].x} cy={ports[port].y} r={4 / zoom} />
  {/each}
</svg>

<style>
  svg { position: absolute; inset: 0; width: 100%; height: 100%; overflow: visible; z-index: 2147483647; pointer-events: none; }
  .hit { fill: transparent; pointer-events: all; touch-action: none; cursor: grab; outline: none; }
  .hit.disabled { pointer-events: none; }
  /* A CSS outline would scale with the page zoom; a zoom-divided stroke keeps its screen size. */
  .hit:focus-visible { stroke: var(--paper-select); stroke-width: calc(2px / var(--zoom)); }
  .dot { pointer-events: none; fill: var(--paper); stroke: var(--paper-select); stroke-width: calc(1.5px / var(--zoom)); }
</style>
